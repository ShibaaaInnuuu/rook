//! The holder raffle's snapshot (docs/v9/raffle-design.md §3–§4, operator decision 2026-10-10): who holds a collection's graduated coins,
//! weighted by the backing they carry, as the sorted-pair Merkle tree `commit_raffle` fixes on chain; and its PUBLICATION, kept here and
//! served by the `raffle_snapshot` query so anyone can recompute the root from chain data and check the winner after the draw.
//!
//! The weight rule (computed here, never on chain): for each graduated coin `c` of the collection, `wx(c)` = the live balance of its
//! vault's wX account `["vault_wx", coin_mint]`, `circ(c)` = its supply less the excluded token accounts (owners off the ed25519 curve:
//! pool vaults, lockers, program PDAs; the incinerator; the system program); a wallet's weight is `Σ_c ⌊balance_c × wx(c) / circ(c)⌋`
//! (u128, floored per coin), wallets of weight 0 dropped. Leaves in ascending owner bytes, `cum_start` the prefix sum.
//!
//! What is read is kept honest by the read itself: every token account of a coin comes in ONE `getProgramAccounts` answer two providers
//! agree on byte for byte, the sum of all of them must equal the mint's supply (a read that straddles a mint or a burn is refused), and
//! `circ` is the sum of the included balances of that same answer. Solana RPC has no historical reads: the snapshot states the finalized
//! slots it was read between (`slot_lo..=slot`) and commits `slot`.
//!
//! Storage: the published snapshots live in [`BOOK`], the last [`KEEP_ROUNDS`] rounds of each collection, and ride the upgrade in the
//! line's `raffle_book` slot (`solhome::SolLine`, the upgrade tuple's fourteenth element); a snapshot holds at most [`MAX_LEAVES`]
//! leaves (a 16-node proof: `claim_raffle` fits one transaction).
use std::cell::RefCell;
use std::collections::BTreeMap;

use candid::CandidType;
use serde::{Deserialize, Serialize};
use sha3::{Digest, Keccak256};

use crate::soltx::{self, b58_encode, Pubkey};

pub const LEAF_TAG: &[u8] = b"rook-raffle-leaf";
pub const DRAW_TAG: &[u8] = b"rook-raffle-draw";
/// Leaves in one snapshot, at most (2^16: a proof of at most 16 nodes, well inside `claim_raffle`'s transaction).
pub const MAX_LEAVES: usize = 65_536;
/// Excluded token accounts listed in one snapshot, at most (pool vaults and PDAs: a handful per coin).
pub const MAX_EXCLUDED: usize = 4_096;
/// Rounds of each collection kept published.
pub const KEEP_ROUNDS: usize = 4;
/// Leaves served by one `raffle_snapshot` page, at most.
pub const PAGE_LEAVES: u32 = 2_000;

pub const INCINERATOR: &str = "1nc1nerator11111111111111111111111111111111";

pub fn keccak(parts: &[&[u8]]) -> [u8; 32] {
    let mut h = Keccak256::new();
    for p in parts {
        h.update(p);
    }
    h.finalize().into()
}

/// rook_core `raffle_leaf`: `keccak(keccak("rook-raffle-leaf" ‖ owner ‖ cum_start le ‖ weight le))`.
pub fn leaf_hash(owner: &Pubkey, cum_start: u64, weight: u64) -> [u8; 32] {
    let inner = keccak(&[LEAF_TAG, owner, &cum_start.to_le_bytes(), &weight.to_le_bytes()]);
    keccak(&[&inner])
}

/// An inner node: the two children, the smaller first (OpenZeppelin sorted pairs, rook_core `hashing::verify_proof`).
pub fn node(a: &[u8; 32], b: &[u8; 32]) -> [u8; 32] {
    if a <= b { keccak(&[a, b]) } else { keccak(&[b, a]) }
}

/// rook_core `raffle_ticket`: `u128(keccak("rook-raffle-draw" ‖ randomness ‖ raffle ‖ round le)[..16], big-endian) mod total`.
pub fn ticket(randomness: &[u8; 32], raffle: &Pubkey, round: u64, total_weight: u64) -> u64 {
    let h = keccak(&[DRAW_TAG, randomness, raffle, &round.to_le_bytes()]);
    (u128::from_be_bytes(h[..16].try_into().expect("16")) % total_weight.max(1) as u128) as u64
}

/// The tree's levels, leaves first: neighbours paired left to right, an unpaired last node moved up unchanged.
pub fn levels(leaves: &[[u8; 32]]) -> Vec<Vec<[u8; 32]>> {
    let mut lv = vec![leaves.to_vec()];
    while lv.last().is_some_and(|l| l.len() > 1) {
        let l = lv.last().expect("a level");
        let next = l.chunks(2).map(|c| if c.len() == 2 { node(&c[0], &c[1]) } else { c[0] }).collect();
        lv.push(next);
    }
    lv
}

pub fn root_of(leaves: &[[u8; 32]]) -> [u8; 32] {
    levels(leaves).last().and_then(|l| l.first().copied()).unwrap_or([0; 32])
}

/// Leaf `i`'s proof (bottom up).
pub fn proof_of(lv: &[Vec<[u8; 32]>], mut i: usize) -> Vec<[u8; 32]> {
    let mut p = Vec::new();
    for l in &lv[..lv.len().saturating_sub(1)] {
        if (i ^ 1) < l.len() {
            p.push(l[i ^ 1]);
        }
        i /= 2;
    }
    p
}

/// rook_core `verify_proof`.
pub fn verify(proof: &[[u8; 32]], root: &[u8; 32], leaf: [u8; 32]) -> bool {
    proof.iter().fold(leaf, |h, p| node(&h, p)) == *root
}

#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Leaf {
    pub owner: Pubkey,
    pub cum_start: u64,
    pub weight: u64,
}

#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct SnapCoin {
    pub mint: Pubkey,
    pub vault_wx: Pubkey,
    /// The vault's live wX balance (base units).
    pub wx: u64,
    pub supply: u64,
    /// `supply` less the excluded accounts' balances.
    pub circ: u64,
    /// Wallets holding it that count (on-curve owners with a balance).
    pub holders: u32,
}

#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Excluded {
    pub coin: Pubkey,
    pub account: Pubkey,
    pub owner: Pubkey,
    pub amount: u64,
    pub reason: String,
}

/// One published snapshot: everything needed to recompute `root` from chain data.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub listing: Pubkey,
    pub raffle: Pubkey,
    /// The round it is committed as (`Raffle.round` after the commit).
    pub round: u64,
    /// Finalized slots the balances were read between; `slot` is committed (`snapshot_slot`).
    pub slot_lo: u64,
    pub slot: u64,
    pub taken_at: u64,
    pub coins: Vec<SnapCoin>,
    pub excluded: Vec<Excluded>,
    pub leaves: Vec<Leaf>,
    pub root: [u8; 32],
    pub total_weight: u64,
}

impl Snapshot {
    pub fn leaf_hashes(&self) -> Vec<[u8; 32]> {
        self.leaves.iter().map(|l| leaf_hash(&l.owner, l.cum_start, l.weight)).collect()
    }
    /// The leaf whose range holds `ticket`, its index and its proof.
    pub fn winner(&self, ticket: u64) -> Option<(usize, &Leaf, Vec<[u8; 32]>)> {
        let i = self.leaves.partition_point(|l| l.cum_start.saturating_add(l.weight) <= ticket);
        let l = self.leaves.get(i).filter(|l| l.cum_start <= ticket && ticket < l.cum_start + l.weight)?;
        Some((i, l, proof_of(&levels(&self.leaf_hashes()), i)))
    }
}

/// One graduated coin as read: its mint, the vault's wX account and balance, the supply, and every token account of the mint
/// `(account, owner, amount)` from one agreed `getProgramAccounts`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CoinRead {
    pub mint: Pubkey,
    pub vault_wx: Pubkey,
    pub wx: u64,
    pub supply: u64,
    pub accounts: Vec<(Pubkey, Pubkey, u64)>,
}

/// Why a token account receives no weight (`None`: it counts). docs/v9/raffle-design.md §4.3.
pub fn exclusion(owner: &Pubkey) -> Option<&'static str> {
    if *owner == soltx::key(INCINERATOR) {
        return Some("incinerator");
    }
    if *owner == soltx::SYSTEM_PROGRAM {
        return Some("system program");
    }
    if soltx::is_on_curve(owner) {
        return None;
    }
    Some(if *owner == soltx::key(crate::solix2::DBC_POOL_AUTHORITY) {
        "DBC pool vault"
    } else if *owner == soltx::key(crate::solix2::DAMM_POOL_AUTHORITY) {
        "DAMM v2 pool vault"
    } else {
        "off-curve owner (a program address)"
    })
}

/// Why no snapshot: `soft` when there is simply nobody to draw among yet (no graduated coin, no holder): waited out, never alerted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SnapError {
    pub soft: bool,
    pub why: String,
}

impl SnapError {
    fn hard(why: String) -> SnapError {
        SnapError { soft: false, why }
    }
    fn soft(why: &str) -> SnapError {
        SnapError { soft: true, why: why.into() }
    }
}

/// The snapshot from what was read (pure): exclusions, circulating supplies, weights, leaves and root.
#[allow(clippy::too_many_arguments)]
pub fn build(listing: Pubkey, raffle: Pubkey, round: u64, slot_lo: u64, slot: u64, taken_at: u64, coins: &[CoinRead])
    -> Result<Snapshot, SnapError> {
    build_with(listing, raffle, round, slot_lo, slot, taken_at, coins, &[])
}

/// [`build`] with OWNERS excluded on top of [`exclusion`]'s rule, each with its reason (canister-only, docs/v9/canister-only-solana.md
/// §9.3: the canister's own derived keys are on the curve and would otherwise count as holders). Their balances are listed in
/// `excluded`, kept in `supply` (the "accounts sum to the supply" check stays whole) and left out of `circ`, like a pool vault's.
#[allow(clippy::too_many_arguments)]
pub fn build_with(listing: Pubkey, raffle: Pubkey, round: u64, slot_lo: u64, slot: u64, taken_at: u64, coins: &[CoinRead],
    extra_excluded: &[(Pubkey, String)]) -> Result<Snapshot, SnapError> {
    let mut weights: BTreeMap<Pubkey, u128> = BTreeMap::new();
    let mut out_coins = Vec::new();
    let mut excluded = Vec::new();
    let extra: BTreeMap<&Pubkey, &str> = extra_excluded.iter().map(|(k, why)| (k, why.as_str())).collect();
    for c in coins {
        let all: u128 = c.accounts.iter().map(|a| a.2 as u128).sum();
        if all != c.supply as u128 {
            return Err(SnapError::hard(format!("coin {}: its token accounts sum to {all}, its supply is {}: the reads straddle a mint or a \
                burn (or a listing is incomplete): taken again later", b58_encode(&c.mint), c.supply)));
        }
        let mut held: BTreeMap<Pubkey, u128> = BTreeMap::new();
        for (acct, owner, amount) in &c.accounts {
            if *amount == 0 {
                continue;
            }
            match exclusion(owner).or_else(|| extra.get(owner).copied()) {
                Some(why) => excluded.push(Excluded { coin: c.mint, account: *acct, owner: *owner, amount: *amount, reason: why.into() }),
                None => *held.entry(*owner).or_default() += *amount as u128,
            }
        }
        let circ: u128 = held.values().sum();
        out_coins.push(SnapCoin { mint: c.mint, vault_wx: c.vault_wx, wx: c.wx, supply: c.supply, circ: circ as u64, holders: held.len() as u32 });
        if c.wx == 0 || circ == 0 {
            continue;
        }
        for (o, b) in held {
            let w = b * c.wx as u128 / circ;
            if w != 0 {
                *weights.entry(o).or_default() += w;
            }
        }
    }
    if excluded.len() > MAX_EXCLUDED {
        return Err(SnapError::hard(format!("{} excluded token accounts, over the {MAX_EXCLUDED} a snapshot lists", excluded.len())));
    }
    if out_coins.is_empty() {
        return Err(SnapError::soft("no graduated coin with backing yet"));
    }
    if weights.is_empty() {
        return Err(SnapError::soft("no holder carries any weight yet"));
    }
    if weights.len() > MAX_LEAVES {
        return Err(SnapError::hard(format!("{} holders, over the {MAX_LEAVES} a snapshot holds (a proof of 16 nodes)", weights.len())));
    }
    let mut leaves = Vec::with_capacity(weights.len());
    let mut cum: u64 = 0;
    for (owner, w) in weights {
        let w = u64::try_from(w).map_err(|_| SnapError::hard("a weight past u64".into()))?;
        leaves.push(Leaf { owner, cum_start: cum, weight: w });
        cum = cum.checked_add(w).ok_or_else(|| SnapError::hard("the total weight is past u64".into()))?;
    }
    let mut s = Snapshot { listing, raffle, round, slot_lo, slot, taken_at, coins: out_coins, excluded, leaves, root: [0; 32], total_weight: cum };
    s.root = root_of(&s.leaf_hashes());
    Ok(s)
}

// ---- the published book --------------------------------------------------------------------------------------------------------------

/// Every published snapshot, the last [`KEEP_ROUNDS`] of each collection.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct RaffleBook {
    pub snapshots: Vec<Snapshot>,
}

impl RaffleBook {
    /// Publish `s`, replacing a snapshot of the same collection and round (a commit that never landed is taken again).
    pub fn publish(&mut self, s: Snapshot) {
        self.snapshots.retain(|x| !(x.listing == s.listing && x.round == s.round));
        let listing = s.listing;
        self.snapshots.push(s);
        let mut rounds: Vec<u64> = self.snapshots.iter().filter(|x| x.listing == listing).map(|x| x.round).collect();
        rounds.sort_unstable();
        if rounds.len() > KEEP_ROUNDS {
            let keep_from = rounds[rounds.len() - KEEP_ROUNDS];
            self.snapshots.retain(|x| x.listing != listing || x.round >= keep_from);
        }
    }
    pub fn get(&self, listing: &Pubkey, round: u64) -> Option<&Snapshot> {
        self.snapshots.iter().find(|x| x.listing == *listing && x.round == round)
    }
    pub fn rounds(&self, listing: &Pubkey) -> Vec<u64> {
        let mut r: Vec<u64> = self.snapshots.iter().filter(|x| x.listing == *listing).map(|x| x.round).collect();
        r.sort_unstable();
        r
    }
}

thread_local! {
    /// The published snapshots. Rides the upgrade in the line's `raffle_book` slot.
    pub(crate) static BOOK: RefCell<RaffleBook> = RefCell::new(RaffleBook::default());
}

pub fn publish(s: Snapshot) {
    BOOK.with(|b| b.borrow_mut().publish(s));
}

pub fn published(listing: &Pubkey, round: u64) -> Option<Snapshot> {
    BOOK.with(|b| b.borrow().get(listing, round).cloned())
}

pub fn take_book() -> RaffleBook {
    BOOK.with(|b| b.borrow().clone())
}

pub fn restore_book(b: RaffleBook) {
    BOOK.with(|x| *x.borrow_mut() = b);
}

// ---- the queries ------------------------------------------------------------------------------------------------------------------------

#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct PageCoin {
    pub mint: String,
    pub vault_wx: String,
    pub wx: u64,
    pub supply: u64,
    pub circ: u64,
    pub holders: u32,
}

#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct PageExcluded {
    pub coin: String,
    pub account: String,
    pub owner: String,
    pub amount: u64,
    pub reason: String,
}

#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct PageLeaf {
    pub owner: String,
    pub cum_start: u64,
    pub weight: u64,
}

/// One page of a published snapshot, addresses in base58 and the root in hex: the coins and exclusions whole, `leaves[offset..]`.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct SnapshotPage {
    pub listing: String,
    pub raffle: String,
    pub round: u64,
    pub slot: u64,
    pub slot_lo: u64,
    pub taken_at: u64,
    pub root: String,
    pub total_weight: u64,
    pub entries: u32,
    pub coins: Vec<PageCoin>,
    pub excluded: Vec<PageExcluded>,
    pub offset: u32,
    pub leaves: Vec<PageLeaf>,
}

pub fn page(s: &Snapshot, offset: u32, limit: u32) -> SnapshotPage {
    let from = (offset as usize).min(s.leaves.len());
    let to = from.saturating_add(limit.clamp(1, PAGE_LEAVES) as usize).min(s.leaves.len());
    SnapshotPage {
        listing: b58_encode(&s.listing),
        raffle: b58_encode(&s.raffle),
        round: s.round,
        slot: s.slot,
        slot_lo: s.slot_lo,
        taken_at: s.taken_at,
        root: hex::encode(s.root),
        total_weight: s.total_weight,
        entries: s.leaves.len() as u32,
        coins: s.coins.iter().map(|c| PageCoin { mint: b58_encode(&c.mint), vault_wx: b58_encode(&c.vault_wx), wx: c.wx, supply: c.supply,
            circ: c.circ, holders: c.holders }).collect(),
        excluded: s.excluded.iter().map(|e| PageExcluded { coin: b58_encode(&e.coin), account: b58_encode(&e.account), owner: b58_encode(&e.owner),
            amount: e.amount, reason: e.reason.clone() }).collect(),
        offset: from as u32,
        leaves: s.leaves[from..to].iter().map(|l| PageLeaf { owner: b58_encode(&l.owner), cum_start: l.cum_start, weight: l.weight }).collect(),
    }
}

/// A published raffle snapshot (docs/v9/raffle-design.md §4.6), `limit` leaves from `offset` (at most [`PAGE_LEAVES`]); `None` when the
/// collection (base58 listing address) has no snapshot of that round kept.
#[ic_cdk::query]
pub fn raffle_snapshot(listing: String, round: u64, offset: u32, limit: u32) -> Option<SnapshotPage> {
    let l = soltx::pubkey(&listing).ok()?;
    BOOK.with(|b| b.borrow().get(&l, round).map(|s| page(s, offset, limit)))
}

/// The rounds of a collection whose snapshot is kept published.
#[ic_cdk::query]
pub fn raffle_rounds(listing: String) -> Vec<u64> {
    let Ok(l) = soltx::pubkey(&listing) else { return vec![] };
    BOOK.with(|b| b.borrow().rounds(&l))
}
