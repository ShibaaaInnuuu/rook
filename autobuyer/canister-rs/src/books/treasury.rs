//! The protocol treasury (rook_core `state/treasury.rs` + `instructions/treasury.rs`; ProtocolTreasury.sol without floats and
//! bootstrap loans): the 90/10 shares, `allocate`, the distribution round weighted by seven-day volume, the per-collection
//! reserve cap, the raffle share of the overflow, and the 24 h public wait on every change.
//!
//! The lamports are at the treasury key `K_tre` (docs/v9/canister-only-solana.md §1.1, §9.1); the books here are claims on them:
//! `balance(K_tre) − rent ≥ unallocated + reserves_pot + Σ raffle pots` ([`super::treasury_balance`]). rook_core read "what
//! arrived" as the account's lamports above its pots; a canister-held key can receive unsolicited SOL, which §4.4 keeps out of
//! the books, so income is booked explicitly ([`Treasury::receive`]) and `allocate` splits only booked income.
//!
//! Pagination goes (§4.1): one canister call weighs every collection ([`Treasury::begin_distribution`] takes all the weights at
//! once; rook_core's `weigh_distribution` is not needed). The round rules of the A2-01 fix are kept.

use candid::CandidType;
use serde::{Deserialize, Serialize};

use super::math::{mul_div, Res, BPS, DAY};
use super::BooksError as E;

/// `round_paid` is a u64 bit set.
pub const MAX_COLLECTIONS: usize = 64;
/// A distribution round left unfinished this long may be restarted (audit A2-01).
pub const ROUND_TIMEOUT: i64 = DAY;
/// How long a change of the shares, the ops address, the reserve cap or the raffle share waits in public.
pub const TREASURY_DELAY: i64 = DAY;
/// Each collection's reserve cap, in bps of its accepted floor: two floors.
pub const DEFAULT_RESERVE_CAP_BPS: u32 = 20_000;
pub const MIN_RESERVE_CAP_BPS: u32 = 1_000;
pub const MAX_RESERVE_CAP_BPS: u32 = 200_000;
/// The share of a collection's payout above its reserve cap that goes to its raffle pot.
pub const DEFAULT_RAFFLE_BPS: u16 = 5_000;

fn ensure(ok: bool, e: E) -> Res<()> {
    if ok {
        Ok(())
    } else {
        Err(e)
    }
}

#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Treasury {
    pub reserves_bps: u16,
    pub ops_bps: u16,
    /// The operations address (a Solana wallet: the fueler's hot key, §9.2); unset, its share joins the reserves pot.
    pub ops: Option<[u8; 32]>,
    /// Booked income not yet allocated (rook_core: the account's lamports above `reserves_pot`).
    pub unallocated: u64,
    pub reserves_pot: u64,
    pub total_received: u64,
    pub total_allocated: u64,
    pub total_to_ops: u64,
    pub total_to_reserves: u64,
    pub total_to_raffles: u64,
    /// Registered collections, by index (the canister's collection ids).
    pub collections: Vec<u16>,
    // ---- the distribution round ----
    pub round_pot: u64,
    pub round_total: u64,
    pub round_weights: Vec<u64>,
    pub round_paid: u64,
    pub round_started_at: i64,
    // ---- changes waiting TREASURY_DELAY (0: nothing pending) ----
    pub pending_reserves_bps: u16,
    pub pending_ops_bps: u16,
    pub pending_shares_at: i64,
    pub pending_ops: Option<[u8; 32]>,
    pub pending_ops_at: i64,
    pub reserve_cap_bps: u32,
    pub pending_reserve_cap_bps: u32,
    pub pending_reserve_cap_at: i64,
    pub raffle_bps: u16,
    pub pending_raffle_bps: u16,
    pub pending_raffle_at: i64,
}

impl Default for Treasury {
    /// rook_core `init_treasury`: 90 % reserves, 10 % ops; two floors of reserve cap; half the overflow to the raffle.
    fn default() -> Self {
        Treasury {
            reserves_bps: 9_000,
            ops_bps: 1_000,
            ops: None,
            unallocated: 0,
            reserves_pot: 0,
            total_received: 0,
            total_allocated: 0,
            total_to_ops: 0,
            total_to_reserves: 0,
            total_to_raffles: 0,
            collections: Vec::new(),
            round_pot: 0,
            round_total: 0,
            round_weights: vec![0; MAX_COLLECTIONS],
            round_paid: 0,
            round_started_at: 0,
            pending_reserves_bps: 0,
            pending_ops_bps: 0,
            pending_shares_at: 0,
            pending_ops: None,
            pending_ops_at: 0,
            reserve_cap_bps: DEFAULT_RESERVE_CAP_BPS,
            pending_reserve_cap_bps: 0,
            pending_reserve_cap_at: 0,
            raffle_bps: DEFAULT_RAFFLE_BPS,
            pending_raffle_bps: 0,
            pending_raffle_at: 0,
        }
    }
}

/// What `allocate` decided: `to_ops` lamports leave `K_tre` for the ops address.
#[derive(CandidType, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Allocation {
    pub amount: u64,
    pub to_reserves: u64,
    pub to_ops: u64,
}

/// What one collection's distribution decided. `to_surplus` moves `K_tre → M[i]`; `to_raffle` stays at `K_tre` as the raffle
/// pot (a book move only); `to_ops` moves `K_tre → ops`; `kept` stays in the reserves pot (no ops address).
#[derive(CandidType, Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Distribution {
    pub index: u8,
    pub collection: u16,
    pub owed: u64,
    pub to_surplus: u64,
    pub to_raffle: u64,
    pub to_ops: u64,
    pub kept: u64,
}

impl Treasury {
    /// The lamports these books claim at `K_tre` (raffle pots are claimed by the raffles).
    pub fn books(&self) -> u128 {
        self.unallocated as u128 + self.reserves_pot as u128
    }

    /// A collection's reserve cap in lamports; `None` while it has no accepted floor.
    pub fn reserve_cap(&self, accepted: u64) -> Option<u64> {
        if accepted == 0 {
            return None;
        }
        let c = accepted as u128 * self.reserve_cap_bps as u128 / BPS as u128;
        Some(u64::try_from(c).unwrap_or(u64::MAX))
    }

    /// What index `i` of the current round is still owed.
    pub fn round_owed(&self, i: usize) -> Res<u64> {
        if i >= MAX_COLLECTIONS || self.round_total == 0 || self.round_weights[i] == 0 || self.round_paid & (1u64 << i) != 0 {
            return Ok(0);
        }
        Ok(mul_div(self.round_pot, self.round_weights[i], self.round_total)?.min(self.reserves_pot))
    }

    /// A round is open while some index is still owed a non-zero amount.
    pub fn round_open(&self) -> Res<bool> {
        for i in 0..MAX_COLLECTIONS {
            if self.round_owed(i)? != 0 {
                return Ok(true);
            }
        }
        Ok(false)
    }

    pub fn index_of(&self, collection: u16) -> Option<usize> {
        self.collections.iter().position(|c| *c == collection)
    }

    /// `addCollection`.
    pub fn add_collection(&mut self, collection: u16) -> Res<()> {
        ensure(!self.collections.contains(&collection), E::AlreadyCollection)?;
        ensure(self.collections.len() < MAX_COLLECTIONS, E::Full)?;
        self.collections.push(collection);
        Ok(())
    }

    // ---- the 24 h changes ----------------------------------------------------------------------------------------------

    /// `set_shares`: proposes reserves + ops = 100 %; anyone applies it after `TREASURY_DELAY`.
    pub fn set_shares(&mut self, reserves_bps: u16, ops_bps: u16, now: i64) -> Res<i64> {
        ensure(reserves_bps as u64 + ops_bps as u64 == BPS, E::BadShares)?;
        self.pending_reserves_bps = reserves_bps;
        self.pending_ops_bps = ops_bps;
        self.pending_shares_at = now + TREASURY_DELAY;
        Ok(self.pending_shares_at)
    }

    pub fn apply_shares(&mut self, now: i64) -> Res<()> {
        ensure(self.pending_shares_at != 0, E::NothingPending)?;
        ensure(now >= self.pending_shares_at, E::TooEarly)?;
        self.reserves_bps = self.pending_reserves_bps;
        self.ops_bps = self.pending_ops_bps;
        self.pending_reserves_bps = 0;
        self.pending_ops_bps = 0;
        self.pending_shares_at = 0;
        Ok(())
    }

    /// `set_ops`: the first ops address applies at once; every later change waits `TREASURY_DELAY`. Returns when it applies.
    pub fn set_ops(&mut self, ops: [u8; 32], now: i64) -> Res<i64> {
        if self.ops.is_none() {
            self.ops = Some(ops);
            return Ok(0);
        }
        self.pending_ops = Some(ops);
        self.pending_ops_at = now + TREASURY_DELAY;
        Ok(self.pending_ops_at)
    }

    pub fn apply_ops(&mut self, now: i64) -> Res<()> {
        ensure(self.pending_ops_at != 0, E::NothingPending)?;
        ensure(now >= self.pending_ops_at, E::TooEarly)?;
        self.ops = self.pending_ops;
        self.pending_ops = None;
        self.pending_ops_at = 0;
        Ok(())
    }

    /// `set_reserve_cap`: bps of the accepted floor, `MIN..=MAX`; applies after `TREASURY_DELAY`.
    pub fn set_reserve_cap(&mut self, bps: u32, now: i64) -> Res<i64> {
        ensure((MIN_RESERVE_CAP_BPS..=MAX_RESERVE_CAP_BPS).contains(&bps), E::BadReserveCap)?;
        self.pending_reserve_cap_bps = bps;
        self.pending_reserve_cap_at = now + TREASURY_DELAY;
        Ok(self.pending_reserve_cap_at)
    }

    pub fn apply_reserve_cap(&mut self, now: i64) -> Res<()> {
        ensure(self.pending_reserve_cap_at != 0, E::NothingPending)?;
        ensure(now >= self.pending_reserve_cap_at, E::TooEarly)?;
        self.reserve_cap_bps = self.pending_reserve_cap_bps;
        self.pending_reserve_cap_bps = 0;
        self.pending_reserve_cap_at = 0;
        Ok(())
    }

    /// `set_raffle_bps`: `0..=10_000`; applies after `TREASURY_DELAY`.
    pub fn set_raffle_bps(&mut self, bps: u16, now: i64) -> Res<i64> {
        ensure(bps as u64 <= BPS, E::BadRaffleBps)?;
        self.pending_raffle_bps = bps;
        self.pending_raffle_at = now + TREASURY_DELAY;
        Ok(self.pending_raffle_at)
    }

    pub fn apply_raffle_bps(&mut self, now: i64) -> Res<()> {
        ensure(self.pending_raffle_at != 0, E::NothingPending)?;
        ensure(now >= self.pending_raffle_at, E::TooEarly)?;
        self.raffle_bps = self.pending_raffle_bps;
        self.pending_raffle_bps = 0;
        self.pending_raffle_at = 0;
        Ok(())
    }

    // ---- income --------------------------------------------------------------------------------------------------------

    /// `receive()`: booked income (a module's skim, cashed fee wX, a deposit at `K_tre` the canister attributes).
    pub fn receive(&mut self, lamports: u64) -> Res<()> {
        ensure(lamports != 0, E::BadInput)?;
        self.unallocated = self.unallocated.checked_add(lamports).ok_or(E::MathOverflow)?;
        self.total_received = self.total_received.saturating_add(lamports);
        Ok(())
    }

    /// `allocate()`: what arrived since the last call into the reserves pot and to operations by the shares.
    pub fn allocate(&mut self) -> Res<Allocation> {
        let amount = self.unallocated;
        ensure(amount != 0, E::NothingToDo)?;
        let mut to_reserves = mul_div(amount, self.reserves_bps as u64, BPS)?;
        let mut to_ops = amount - to_reserves;
        if self.ops.is_none() {
            to_reserves += to_ops;
            to_ops = 0;
        }
        self.reserves_pot = self.reserves_pot.checked_add(to_reserves).ok_or(E::MathOverflow)?;
        self.unallocated = 0;
        self.total_allocated = self.total_allocated.saturating_add(amount);
        self.total_to_ops = self.total_to_ops.saturating_add(to_ops);
        Ok(Allocation { amount, to_reserves, to_ops })
    }

    // ---- distribution --------------------------------------------------------------------------------------------------

    /// `begin_distribution`: opens a round; `weights[i]` = collection `collections[i]`'s seven-day volume. Refused while the
    /// current round is open, unless it began `ROUND_TIMEOUT` ago (A2-01).
    pub fn begin_distribution(&mut self, weights: &[u64], now: i64) -> Res<()> {
        let n = self.collections.len();
        ensure(weights.len() == n, E::BadInput)?;
        ensure(!self.round_open()? || now >= self.round_started_at + ROUND_TIMEOUT, E::RoundOpen)?;
        let mut total: u64 = 0;
        let mut w = vec![0u64; MAX_COLLECTIONS];
        for (i, x) in weights.iter().enumerate() {
            w[i] = *x;
            total = total.saturating_add(*x);
        }
        ensure(total != 0, E::NothingToDo)?;
        self.round_pot = self.reserves_pot;
        self.round_total = total;
        self.round_weights = w;
        self.round_paid = 0;
        self.round_started_at = now;
        Ok(())
    }

    /// `distribute(index)` for one collection: its share of the round pot. The collection's surplus is topped up to its reserve
    /// cap (`accepted` = its accepted floor; the whole share with none); of the rest `raffle_bps` to its raffle pot (when it has
    /// a raffle) and the remainder to ops, or kept in the reserves pot with no ops address. The index is paid for the round
    /// either way. The caller books `to_surplus` into the module and `to_raffle` into the raffle.
    pub fn distribute(&mut self, index: u8, accepted: u64, surplus: u64, has_raffle: bool) -> Res<Distribution> {
        let i = index as usize;
        ensure(i < self.collections.len(), E::NotCollection)?;
        ensure(self.round_total != 0 && self.round_paid & (1u64 << i) == 0, E::NothingToDo)?;
        let r = self.round_owed(i)?;
        ensure(r != 0, E::NothingToDo)?;
        self.round_paid |= 1u64 << i;
        let to_surplus = match self.reserve_cap(accepted) {
            None => r,
            Some(cap) => r.min(cap.saturating_sub(surplus)),
        };
        let over = r - to_surplus;
        let to_raffle = if has_raffle { mul_div(over, self.raffle_bps as u64, BPS)? } else { 0 };
        let rest = over - to_raffle;
        let (to_ops, kept) = if self.ops.is_none() { (0, rest) } else { (rest, 0) };
        self.reserves_pot -= to_surplus + to_raffle + to_ops;
        self.total_to_reserves = self.total_to_reserves.saturating_add(to_surplus);
        self.total_to_ops = self.total_to_ops.saturating_add(to_ops);
        self.total_to_raffles = self.total_to_raffles.saturating_add(to_raffle);
        Ok(Distribution { index, collection: self.collections[i], owed: r, to_surplus, to_raffle, to_ops, kept })
    }
}
