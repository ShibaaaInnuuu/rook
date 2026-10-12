//! The holder raffle's phases and rules (rook_core `state/raffle.rs` + `instructions/raffle.rs`; docs/v9/raffle-design.md):
//! one per collection, a pot of lamports fed by the treasury's overflow, turned into one whole wX by the module's public mint
//! (`fund_prize`, at [`super::Books::fund_prize`]), drawn weekly among the holders of the collection's graduated coins from the
//! Merkle snapshot the canister publishes (`crate::raffle`) and commits here.
//!
//! Canister-only (docs/v9/canister-only-solana.md §9.3): the pot is a book over SOL at `K_tre`; the prize is one wX in a `K_tre`
//! token account; the draw uses the canister's VERIFIED drand randomness; `claim` checks the canister's own tree and the chain
//! executes a `TransferChecked` of ONE to the winner. The keeper role is gone: committing is canister work.

use candid::CandidType;
use serde::{Deserialize, Serialize};

use super::math::{keccak, round_at, verify_proof, Res, DAY, ONE};
use super::BooksError as E;

/// A new round may be committed this long after the previous commit.
pub const RAFFLE_PERIOD: i64 = 7 * DAY;
/// The drand round pinned at commit is this many rounds after the one live then.
pub const RAFFLE_PIN_AHEAD: u64 = 20;
/// A committed round not drawn within this long may be cancelled.
pub const RAFFLE_DRAW_TIMEOUT: i64 = 7 * DAY;
/// A drawn prize unclaimed for this long goes back for the next round.
pub const RAFFLE_CLAIM_WINDOW: i64 = 30 * DAY;
/// The snapshot must be at most this many slots old at commit (≈ 2 days).
pub const RAFFLE_SNAPSHOT_MAX_AGE: u64 = 432_000;
pub const MAX_RAFFLE_PROOF: usize = 32;

pub const RAFFLE_LEAF_TAG: &[u8] = b"rook-raffle-leaf";
pub const RAFFLE_DRAW_TAG: &[u8] = b"rook-raffle-draw";

fn ensure(ok: bool, e: E) -> Res<()> {
    if ok {
        Ok(())
    } else {
        Err(e)
    }
}

#[derive(CandidType, Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RafflePhase {
    #[default]
    Idle,
    Committed,
    Drawn,
}

#[derive(CandidType, Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct Raffle {
    /// The 32 bytes the draw is keyed by (rook_core: the raffle PDA; here fixed at init, e.g. the prize token account).
    pub key: [u8; 32],
    pub phase: RafflePhase,
    /// Lamports earmarked for prizes (held at `K_tre`).
    pub pot: u64,
    /// wX base units held as the prize (a `K_tre` token account).
    pub prize: u64,
    pub round: u64,
    pub root: [u8; 32],
    pub total_weight: u64,
    pub entries: u32,
    pub snapshot_slot: u64,
    pub committed_at: i64,
    pub drand_round: u64,
    pub ticket: u64,
    pub drawn_at: i64,
    pub winner: [u8; 32],
    pub claimed_at: i64,
    pub total_in: u64,
    pub total_spent: u64,
    pub prizes_funded: u64,
    pub prizes_claimed: u64,
    pub rounds_expired: u64,
    pub rounds_cancelled: u64,
}

/// A snapshot leaf: `keccak(keccak("rook-raffle-leaf" ‖ owner ‖ cum_start le ‖ weight le))`.
pub fn raffle_leaf(owner: &[u8; 32], cum_start: u64, weight: u64) -> [u8; 32] {
    let inner = keccak(&[RAFFLE_LEAF_TAG, owner, &cum_start.to_le_bytes(), &weight.to_le_bytes()]);
    keccak(&[&inner])
}

/// The winning ticket: `u128(keccak("rook-raffle-draw" ‖ randomness ‖ key ‖ round le)[0..16], big-endian) mod total`.
pub fn raffle_ticket(randomness: &[u8; 32], key: &[u8; 32], round: u64, total_weight: u64) -> u64 {
    let h = keccak(&[RAFFLE_DRAW_TAG, randomness, key, &round.to_le_bytes()]);
    let mut b = [0u8; 16];
    b.copy_from_slice(&h[..16]);
    (u128::from_be_bytes(b) % total_weight.max(1) as u128) as u64
}

impl Raffle {
    pub fn new(key: [u8; 32]) -> Raffle {
        Raffle { key, ..Default::default() }
    }

    /// The pot's share of a distribution (rook_core `distribute`).
    pub fn credit(&mut self, lamports: u64) {
        self.pot = self.pot.saturating_add(lamports);
        self.total_in = self.total_in.saturating_add(lamports);
    }

    /// `fund_prize`'s raffle side, before the module mints: refused while a whole wX is held, or if the pot cannot pay `price`.
    pub fn check_fund(&self, price: u64) -> Res<()> {
        ensure(self.prize < ONE, E::PrizeHeld)?;
        ensure(self.pot >= price, E::PotTooSmall)
    }

    /// `fund_prize`'s raffle side, after the module minted ONE for `price`.
    pub fn funded(&mut self, price: u64, units: u64) {
        self.pot -= price;
        self.prize += units;
        self.total_spent = self.total_spent.saturating_add(price);
        self.prizes_funded += 1;
    }

    /// `commit_raffle`: fixes the round's snapshot and pins the drand round `round_at(now) + RAFFLE_PIN_AHEAD`. Once
    /// `RAFFLE_PERIOD` after the previous commit, with a whole wX prize held. `slot` = the finalized slot now.
    pub fn commit(&mut self, root: [u8; 32], total_weight: u64, snapshot_slot: u64, entries: u32, slot: u64, now: i64) -> Res<u64> {
        ensure(self.phase == RafflePhase::Idle, E::RafflePhase)?;
        ensure(self.committed_at == 0 || now >= self.committed_at + RAFFLE_PERIOD, E::RaffleTooEarly)?;
        ensure(self.prize >= ONE, E::NoPrize)?;
        ensure(root != [0u8; 32] && total_weight != 0 && entries != 0, E::BadInput)?;
        ensure(
            snapshot_slot < slot && slot - snapshot_slot <= RAFFLE_SNAPSHOT_MAX_AGE && snapshot_slot > self.snapshot_slot,
            E::BadSnapshot,
        )?;
        self.round += 1;
        self.phase = RafflePhase::Committed;
        self.root = root;
        self.total_weight = total_weight;
        self.entries = entries;
        self.snapshot_slot = snapshot_slot;
        self.committed_at = now;
        self.drand_round = round_at(now) + RAFFLE_PIN_AHEAD;
        self.ticket = 0;
        self.drawn_at = 0;
        Ok(self.drand_round)
    }

    /// `draw_raffle`: `randomness` = the canister's verified randomness of the pinned round `round`.
    pub fn draw(&mut self, round: u64, randomness: &[u8; 32], now: i64) -> Res<u64> {
        ensure(self.phase == RafflePhase::Committed, E::RafflePhase)?;
        ensure(round == self.drand_round, E::RoundNotProven)?;
        ensure(round <= round_at(now), E::RoundInFuture)?;
        self.ticket = raffle_ticket(randomness, &self.key, self.round, self.total_weight);
        self.phase = RafflePhase::Drawn;
        self.drawn_at = now;
        Ok(self.ticket)
    }

    /// `claim_raffle`: the leaf `(owner, cum_start, weight)` is in the committed root and its range holds the winning ticket;
    /// the chain transfers ONE wX to the owner's associated token account. Within `RAFFLE_CLAIM_WINDOW` of the draw.
    pub fn claim(&mut self, owner: [u8; 32], cum_start: u64, weight: u64, proof: &[[u8; 32]], now: i64) -> Res<u64> {
        ensure(self.phase == RafflePhase::Drawn, E::RafflePhase)?;
        ensure(now < self.drawn_at + RAFFLE_CLAIM_WINDOW, E::ClaimWindowOver)?;
        ensure(proof.len() <= MAX_RAFFLE_PROOF, E::BadInput)?;
        let end = cum_start.checked_add(weight).ok_or(E::NotTheWinner)?;
        ensure(weight != 0 && cum_start <= self.ticket && self.ticket < end, E::NotTheWinner)?;
        ensure(verify_proof(proof, &self.root, raffle_leaf(&owner, cum_start, weight)), E::BadProof)?;
        ensure(self.prize >= ONE, E::NoPrize)?;
        self.prize -= ONE;
        self.phase = RafflePhase::Idle;
        self.winner = owner;
        self.claimed_at = now;
        self.prizes_claimed += 1;
        Ok(ONE)
    }

    /// `expire_raffle`: a prize unclaimed `RAFFLE_CLAIM_WINDOW` after its draw stays for the next round; a committed round not
    /// drawn within `RAFFLE_DRAW_TIMEOUT` is cancelled. Returns whether the expired round was drawn.
    pub fn expire(&mut self, now: i64) -> Res<bool> {
        let drawn = match self.phase {
            RafflePhase::Drawn if now >= self.drawn_at + RAFFLE_CLAIM_WINDOW => {
                self.rounds_expired += 1;
                true
            }
            RafflePhase::Committed if now >= self.committed_at + RAFFLE_DRAW_TIMEOUT => {
                self.rounds_cancelled += 1;
                false
            }
            _ => return Err(E::RaffleNotExpired),
        };
        self.phase = RafflePhase::Idle;
        Ok(drawn)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(a: [u8; 32], b: [u8; 32]) -> [u8; 32] {
        if a <= b {
            keccak(&[&a, &b])
        } else {
            keccak(&[&b, &a])
        }
    }

    // rook_core state/raffle.rs: leaves_verify_in_a_sorted_pair_tree_and_are_not_nodes
    #[test]
    fn leaves_verify_in_a_sorted_pair_tree_and_are_not_nodes() {
        let o: Vec<[u8; 32]> = (1..=3u8).map(|i| [i; 32]).collect();
        let l = [raffle_leaf(&o[0], 0, 5), raffle_leaf(&o[1], 5, 7), raffle_leaf(&o[2], 12, 1)];
        let n01 = node(l[0], l[1]);
        let root = node(n01, l[2]);
        assert!(verify_proof(&[l[1], l[2]], &root, l[0]));
        assert!(verify_proof(&[l[0], l[2]], &root, l[1]));
        assert!(verify_proof(&[n01], &root, l[2]));
        assert!(!verify_proof(&[n01], &root, raffle_leaf(&o[2], 12, 2)), "another weight");
    }

    // rook_core state/raffle.rs: tickets_are_below_the_total_and_keyed_by_raffle_and_round
    #[test]
    fn tickets_are_below_the_total_and_keyed_by_raffle_and_round() {
        let r = [9u8; 32];
        let a = [1u8; 32];
        let b = [2u8; 32];
        for total in [1u64, 2, 7, 1_000_000_007, u64::MAX] {
            assert!(raffle_ticket(&r, &a, 1, total) < total);
        }
        assert_eq!(raffle_ticket(&r, &a, 1, 1), 0);
        assert_ne!(raffle_ticket(&r, &a, 1, u64::MAX), raffle_ticket(&r, &b, 1, u64::MAX));
        assert_ne!(raffle_ticket(&r, &a, 1, u64::MAX), raffle_ticket(&r, &a, 2, u64::MAX));
    }

    // the snapshot builder (crate::raffle) and the books agree on the leaf and the ticket
    #[test]
    fn the_books_and_the_snapshot_builder_hash_alike() {
        let o = [7u8; 32];
        assert_eq!(raffle_leaf(&o, 3, 9), crate::raffle::leaf_hash(&o, 3, 9));
        assert_eq!(raffle_ticket(&[4; 32], &o, 2, 1_000), crate::raffle::ticket(&[4; 32], &o, 2, 1_000));
    }
}
