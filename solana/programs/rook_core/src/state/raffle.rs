//! The holder raffle (operator decision 2026-10-10; docs/v9/raffle-design.md). One per collection: a pot of lamports fed by
//! the treasury's reserve overflow (`distribute`), turned into one whole wX by the floor module's public mint (`fund_prize`),
//! drawn weekly among the holders of the collection's graduated coins from a Merkle snapshot the keeper commits.

use anchor_lang::prelude::*;

use crate::clock::DAY;
use crate::hashing::keccak;

/// A new round may be committed this long after the previous commit.
pub const RAFFLE_PERIOD: i64 = 7 * DAY;
/// The drand round pinned at commit is this many rounds (3 s each) after the one live then: it does not exist yet.
pub const RAFFLE_PIN_AHEAD: u64 = 20;
/// A committed round whose drand round is not proven within this long may be cancelled (`expire_raffle`).
pub const RAFFLE_DRAW_TIMEOUT: i64 = 7 * DAY;
/// A drawn prize unclaimed for this long goes back to the raffle for the next round (`expire_raffle`).
pub const RAFFLE_CLAIM_WINDOW: i64 = 30 * DAY;
/// The snapshot must be at most this many slots old at commit (≈ 2 days at 400 ms).
pub const RAFFLE_SNAPSHOT_MAX_AGE: u64 = 432_000;
/// Longest Merkle proof accepted (2^32 leaves).
pub const MAX_RAFFLE_PROOF: usize = 32;
/// The treasury's default share of reserve overflow sent to the collection's raffle pot.
pub const DEFAULT_RAFFLE_BPS: u16 = 5_000;

pub const RAFFLE_LEAF_TAG: &[u8] = b"rook-raffle-leaf";
pub const RAFFLE_DRAW_TAG: &[u8] = b"rook-raffle-draw";

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, Debug, Default, PartialEq, Eq, InitSpace)]
pub enum RafflePhase {
    /// No round in progress: the prize may be funded and the next round committed.
    #[default]
    Idle,
    /// A snapshot is committed and a future drand round pinned.
    Committed,
    /// The winning ticket is known; the prize waits for its claim.
    Drawn,
}

/// PDA `["raffle", listing]`. Holds its pot's lamports itself (`lamports − rent ≥ pot`); its wX token account
/// `["raffle_wx", raffle]` (authority = this PDA) holds the prize.
#[account]
#[derive(InitSpace)]
pub struct Raffle {
    pub listing: Pubkey,
    pub module: Pubkey,
    pub wx_mint: Pubkey,
    pub wx_account: Pubkey,
    pub bump: u8,
    pub wx_bump: u8,
    pub phase: RafflePhase,
    /// Lamports earmarked for prizes.
    pub pot: u64,
    /// Rounds committed so far (the current or last round's number, 1-based).
    pub round: u64,
    // ---- the current (or last) round ----
    pub root: [u8; 32],
    pub total_weight: u64,
    pub entries: u32,
    pub snapshot_slot: u64,
    pub committed_at: i64,
    pub drand_round: u64,
    pub ticket: u64,
    pub drawn_at: i64,
    pub winner: Pubkey,
    pub claimed_at: i64,
    // ---- history ----
    pub total_in: u64,
    pub total_spent: u64,
    pub prizes_funded: u64,
    pub prizes_claimed: u64,
    pub rounds_expired: u64,
    pub rounds_cancelled: u64,
}

/// A snapshot leaf: `keccak(keccak("rook-raffle-leaf" ‖ owner ‖ cum_start le ‖ weight le))`, hashed twice so an inner node is
/// never a leaf (as `hashing::leaf_of`).
pub fn raffle_leaf(owner: &Pubkey, cum_start: u64, weight: u64) -> [u8; 32] {
    let inner = keccak(&[RAFFLE_LEAF_TAG, owner.as_ref(), &cum_start.to_le_bytes(), &weight.to_le_bytes()]);
    keccak(&[&inner])
}

/// The winning ticket: `u128(keccak("rook-raffle-draw" ‖ randomness ‖ raffle ‖ round le)[0..16], big-endian) mod total`.
pub fn raffle_ticket(randomness: &[u8; 32], raffle: &Pubkey, round: u64, total_weight: u64) -> u64 {
    let h = keccak(&[RAFFLE_DRAW_TAG, randomness, raffle.as_ref(), &round.to_le_bytes()]);
    let mut b = [0u8; 16];
    b.copy_from_slice(&h[..16]);
    (u128::from_be_bytes(b) % total_weight as u128) as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hashing::verify_proof;

    fn node(a: [u8; 32], b: [u8; 32]) -> [u8; 32] {
        if a <= b {
            keccak(&[&a, &b])
        } else {
            keccak(&[&b, &a])
        }
    }

    #[test]
    fn leaves_verify_in_a_sorted_pair_tree_and_are_not_nodes() {
        let o: Vec<Pubkey> = (1..=3u8).map(|i| Pubkey::new_from_array([i; 32])).collect();
        let l = [raffle_leaf(&o[0], 0, 5), raffle_leaf(&o[1], 5, 7), raffle_leaf(&o[2], 12, 1)];
        let n01 = node(l[0], l[1]);
        let root = node(n01, l[2]);
        assert!(verify_proof(&[l[1], l[2]], &root, l[0]));
        assert!(verify_proof(&[l[0], l[2]], &root, l[1]));
        assert!(verify_proof(&[n01], &root, l[2]));
        assert!(!verify_proof(&[n01], &root, raffle_leaf(&o[2], 12, 2)), "another weight");
    }

    #[test]
    fn sizes() {
        assert_eq!(8 + Raffle::INIT_SPACE, 327);
        assert_eq!(8 + crate::state::Treasury::INIT_SPACE, 2_778); // 64 collections and the paged weighing (2026-10-10)
    }

    #[test]
    fn tickets_are_below_the_total_and_keyed_by_raffle_and_round() {
        let r = [9u8; 32];
        let a = Pubkey::new_from_array([1; 32]);
        let b = Pubkey::new_from_array([2; 32]);
        for total in [1u64, 2, 7, 1_000_000_007, u64::MAX] {
            assert!(raffle_ticket(&r, &a, 1, total) < total);
        }
        assert_eq!(raffle_ticket(&r, &a, 1, 1), 0);
        assert_ne!(raffle_ticket(&r, &a, 1, u64::MAX), raffle_ticket(&r, &b, 1, u64::MAX));
        assert_ne!(raffle_ticket(&r, &a, 1, u64::MAX), raffle_ticket(&r, &a, 2, u64::MAX));
    }
}
