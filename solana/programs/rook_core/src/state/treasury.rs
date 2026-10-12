//! The protocol treasury (src/floor/ProtocolTreasury.sol) minus floats and bootstrap loans. The lamports are held by the
//! `Treasury` account itself (program-owned); `unallocated = lamports − rent − reserves_pot`.

use anchor_lang::prelude::*;

/// Raised from 32 to 64 on 2026-10-10 (Solana NFT collections, docs/v9/solana-nfts-design.md §6): `round_paid` is a u64 bit
/// set; above 61 modules `begin_distribution` cannot lock them all in one transaction and `weigh_distribution` pages it.
pub const MAX_COLLECTIONS: usize = 64;
/// A distribution round left unfinished this long may be restarted (audit A2-01): until then `begin_distribution` refuses
/// while any index with a non-zero share is unpaid, so no collection is paid twice from one pot.
pub const ROUND_TIMEOUT: i64 = crate::clock::DAY;
/// How long a change of the shares, or of a named operations address, waits in public before anyone may apply it.
pub const TREASURY_DELAY: i64 = crate::clock::DAY;
/// Each collection's reserve cap, in basis points of its module's accepted floor price per whole unit: `distribute` tops a
/// module's `surplus` up to `reserve_cap_bps × accepted / 10_000` and sends the rest of its share to operations (operator
/// decision 2026-10-10). The surplus only pays for purchases above backing per unit and for shortfalls; it is never paid to
/// holders, so beyond a cushion it sits idle. The default, two floor prices, covers the collection's next ten purchases
/// each landing about 20 % above backing (10 × 0.2 floor = 2 floors), a sharp rise for a floor that moves at most a few
/// percent a day.
pub const DEFAULT_RESERVE_CAP_BPS: u32 = 20_000;
/// Bounds of `set_reserve_cap`: a tenth of a floor to twenty floors. Zero ("no cap") is refused.
pub const MIN_RESERVE_CAP_BPS: u32 = 1_000;
pub const MAX_RESERVE_CAP_BPS: u32 = 200_000;

/// PDA `["treasury"]`.
#[account]
#[derive(InitSpace)]
pub struct Treasury {
    /// The shares (sum 10_000). Floats and bootstrap shares are dropped with their routes.
    pub reserves_bps: u16,
    pub ops_bps: u16,
    /// Operations, paid its share at every allocation; unset (default), that share joins the reserves pot.
    pub ops: Pubkey,
    pub reserves_pot: u64,
    pub total_received: u64,
    pub total_allocated: u64,
    pub total_to_ops: u64,
    pub total_to_reserves: u64,
    /// Registered floor modules.
    #[max_len(MAX_COLLECTIONS)]
    pub collections: Vec<Pubkey>,
    // ---- the paginated distribution round ----
    /// The reserves pot when the round began, the sum of weights, each collection's weight (index = `collections`), and which
    /// were paid.
    pub round_pot: u64,
    pub round_total: u64,
    pub round_weights: [u64; MAX_COLLECTIONS],
    /// One bit per index of `collections` (u32 until 2026-10-10, when `MAX_COLLECTIONS` became 64).
    pub round_paid: u64,
    pub bump: u8,
    /// When the current round began (`begin_distribution`); appended after `bump` so earlier offsets are unchanged.
    pub round_started_at: i64,
    // ---- changes waiting `TREASURY_DELAY` (appended; earlier offsets unchanged) ----
    /// Proposed shares and when they may be applied (0: nothing pending).
    pub pending_reserves_bps: u16,
    pub pending_ops_bps: u16,
    pub pending_shares_at: i64,
    /// A proposed new operations address and when it may be applied (0: nothing pending).
    pub pending_ops: Pubkey,
    pub pending_ops_at: i64,
    // ---- the per-collection reserve cap (appended 2026-10-10; earlier offsets unchanged) ----
    /// See `DEFAULT_RESERVE_CAP_BPS`; set at `init_treasury`, changed only through `set_reserve_cap` + `apply_reserve_cap`.
    pub reserve_cap_bps: u32,
    /// A proposed cap and when it may be applied (0: nothing pending).
    pub pending_reserve_cap_bps: u32,
    pub pending_reserve_cap_at: i64,
    // ---- the holder raffle (appended 2026-10-10; earlier offsets unchanged; docs/v9/raffle-design.md) ----
    /// The share, in bps, of a collection's payout above its reserve cap that `distribute` sends to that collection's raffle
    /// pot (`DEFAULT_RAFFLE_BPS` at `init_treasury`; changed through `set_raffle_bps` + `apply_raffle_bps`).
    pub raffle_bps: u16,
    /// A proposed raffle share and when it may be applied (0: nothing pending).
    pub pending_raffle_bps: u16,
    pub pending_raffle_at: i64,
    /// Lamports `distribute` sent to raffle pots, ever.
    pub total_to_raffles: u64,
    // ---- the paged weighing (appended 2026-10-10; docs/v9/solana-nfts-design.md §6) ----
    /// The next index `weigh_distribution` expects; 0 when no weighing is in progress.
    pub weigh_next: u8,
    /// The sum of the weights weighed so far.
    pub weigh_total: u64,
}

impl Treasury {
    /// A module's reserve cap in lamports: `reserve_cap_bps × accepted / 10_000`, saturating at `u64::MAX`. `None` while the
    /// module has no accepted floor (its share is then paid in full, as before the cap).
    pub fn reserve_cap(&self, accepted: u64) -> Option<u64> {
        if accepted == 0 {
            return None;
        }
        let c = accepted as u128 * self.reserve_cap_bps as u128 / crate::units::BPS as u128;
        Some(u64::try_from(c).unwrap_or(u64::MAX))
    }

    /// What index `i` of the current round is still owed: its share of the round pot (bounded by the pot), 0 once paid or
    /// with no weight.
    pub fn round_owed(&self, i: usize) -> Result<u64> {
        if self.round_total == 0 || self.round_weights[i] == 0 || self.round_paid & (1u64 << i) != 0 {
            return Ok(0);
        }
        Ok(crate::math::mul_div(self.round_pot, self.round_weights[i], self.round_total)?.min(self.reserves_pot))
    }

    /// A paged weighing has started and not finished (its round is not open yet).
    pub fn weighing(&self) -> bool {
        self.weigh_next != 0
    }

    /// A round is open while some index is still owed a non-zero amount.
    pub fn round_open(&self) -> Result<bool> {
        for i in 0..MAX_COLLECTIONS {
            if self.round_owed(i)? != 0 {
                return Ok(true);
            }
        }
        Ok(false)
    }
}
