//! The raffle snapshot on the books (docs/v9/canister-only-solana.md §9.3, `src/raffle_books.rs`) against the fake chain of the desk's
//! world: the coins come from `coins::state` (`Backed`, with their `K_coin[c]` vault), `wx(c)` is the vault's live wX balance, holders are
//! weighted by the backing they carry, the canister's own (on-curve) keys and the pool's (off-curve) vaults are excluded with their
//! reasons, the root recomputes from the leaves, and `winner` picks the leaf the books' ticket falls in (and the books' `claim` takes it).
mod desk_common;
mod fakerpc;

use std::collections::BTreeSet;

use desk_common::*;
use ed25519_dalek::SigningKey;
use etch_autobuyer::books::raffle::{raffle_leaf, raffle_ticket, Raffle, RafflePhase};
use etch_autobuyer::books::ONE;
use etch_autobuyer::coins::state::{self as coins, CoinRec, CoinStage, Vault};
use etch_autobuyer::desk::state;
use etch_autobuyer::raffle::{self, Snapshot};
use etch_autobuyer::raffle_books;
use etch_autobuyer::solix2::DBC_POOL_AUTHORITY;
use etch_autobuyer::solhome::{SolHomeCfg, SolIo};
use etch_autobuyer::solkey::KeyPath;
use etch_autobuyer::solrpc::{self, Account};
use etch_autobuyer::soltx::{self, Pubkey};

/// The books' raffle key of collection 0 (the prize token account's address in the real thing; any 32 bytes here).
const RKEY: Pubkey = [0xAA; 32];
/// The coin under test and a second one still on its curve.
const COIN: Pubkey = [0xC1; 32];
const CURVE_COIN: Pubkey = [0xC2; 32];
const RENT_MINT: u64 = 1_461_600;

/// A holder's wallet: a REAL Ed25519 key (on the curve, as `raffle::exclusion` requires of a holder).
fn holder(n: u8) -> Pubkey {
    SigningKey::from_bytes(&[0x30 + n; 32]).verifying_key().to_bytes()
}

fn token_program() -> Pubkey {
    soltx::key(soltx::TOKEN_PROGRAM)
}

/// The desk's world with the reads the snapshot makes (`desk_common::World` answers the desk's; the clock, the slices and the key
/// map are added here, on the same fake providers).
struct Rw(World);

impl SolIo for Rw {
    fn sol_home(&self) -> Option<SolHomeCfg> {
        self.0.sol_home()
    }
    async fn sol_me(&mut self) -> Result<Pubkey, String> {
        Ok(me())
    }
    async fn sol_accounts(&mut self, keys: &[Pubkey], max_data: usize) -> Result<Vec<Option<Account>>, String> {
        Ok(solrpc::accounts(&mut self.0.ps, keys, max_data).await?)
    }
    async fn sol_accounts_slice(&mut self, keys: &[Pubkey], offset: usize, len: usize) -> Result<Vec<Option<Account>>, String> {
        Ok(solrpc::accounts_slice(&mut self.0.ps, keys, offset, len).await?)
    }
    async fn sol_program_slices(&mut self, program: &Pubkey, size: usize, memcmps: &[(usize, Vec<u8>)], slice: (usize, usize), max: usize)
        -> Result<Vec<(Pubkey, Pubkey, Vec<u8>)>, String> {
        Ok(solrpc::program_slices(&mut self.0.ps, program, size, memcmps, slice, max).await?)
    }
    async fn sol_clock(&mut self) -> Result<(u64, u64), String> {
        Ok(solrpc::finalized_clock(&mut self.0.ps).await?)
    }
    async fn sol_key_of(&mut self, p: &KeyPath) -> Result<Pubkey, String> {
        Ok(pk(*p))
    }
}

/// `K_coin[c]`'s wX token account: the coin's vault.
fn vault_of(coin: &Pubkey) -> Pubkey {
    soltx::associated_token_address(&pk(KeyPath::Coin(*coin)), &WX)
}

/// The DBC pool's base vault of the coin (owned by the pool authority, a program address: off the curve).
fn pool_vault(coin: &Pubkey) -> Pubkey {
    soltx::associated_token_address(&soltx::key(DBC_POOL_AUTHORITY), coin)
}

/// A world with the raffle on the books and coin [`COIN`] backed: its vault holds `wx` wX, its token accounts are `holders`
/// `(owner, amount)`, and its mint's supply is their sum (`supply_delta` away from it, for the straddle test).
fn world(wx: u64, holders: &[(Pubkey, u64)], supply_delta: i64) -> Rw {
    let mut w = World::new();
    state::with_mut(|s| s.books.init_raffle(COLL, RKEY).unwrap());
    let supply = (holders.iter().map(|h| h.1 as i64).sum::<i64>() + supply_delta) as u64;
    let kcoin = pk(KeyPath::Coin(COIN));
    w.plant(COIN, token_program(), RENT_MINT, mint_data(soltx::key(DBC_POOL_AUTHORITY), supply));
    w.plant(vault_of(&COIN), token_program(), RENT165, token_data(WX, kcoin, wx));
    w.inject_supply(wx as i64);
    for (owner, amount) in holders {
        w.plant(soltx::associated_token_address(owner, &COIN), token_program(), RENT165, token_data(COIN, *owner, *amount));
    }
    let rec = |coin: Pubkey, stage: CoinStage, vault: Option<Vault>| CoinRec {
        coll: COLL as u32, coin, pool: [0x55; 32], creator: holder(9), launched_at: 1, seq: 0, stage, vault, damm_pool: None, partner_nft: None,
        surplus_tries: 0,
    };
    coins::with_mut(|s| {
        s.coins.insert(COIN, rec(COIN, CoinStage::Backed, Some(Vault { address: vault_of(&COIN), owner: kcoin, wx_in: wx, ..Default::default() })));
        s.coins.insert(CURVE_COIN, rec(CURVE_COIN, CoinStage::Curve, None));
    });
    // the curve coin exists on chain with holders of its own: not backed, so not in the snapshot
    w.plant(CURVE_COIN, token_program(), RENT_MINT, mint_data(soltx::key(DBC_POOL_AUTHORITY), 500));
    w.plant(soltx::associated_token_address(&holder(1), &CURVE_COIN), token_program(), RENT165, token_data(CURVE_COIN, holder(1), 500));
    Rw(w)
}

fn take(w: &mut Rw, round: u64) -> Result<Snapshot, raffle::SnapError> {
    block_on(raffle_books::take(w, COLL, round))
}

/// The standard holding: two wallets, the pool desk's key and the DBC pool vault, 9,000 wX of backing.
fn standard() -> Rw {
    world(9_000, &[(holder(1), 600), (holder(2), 300), (pk(KeyPath::Desk(0)), 100), (soltx::key(DBC_POOL_AUTHORITY), 1_000)], 0)
}

#[test]
fn the_vault_balance_is_the_backing_and_holders_are_weighted_by_it() {
    let mut w = standard();
    let s = take(&mut w, 1).unwrap();
    assert_eq!(s.listing, WX, "the listing is the collection's wX mint");
    assert_eq!(s.raffle, RKEY, "the raffle is the books' raffle key");
    assert_eq!(s.round, 1);
    assert!(s.slot_lo <= s.slot, "read between {} and {}", s.slot_lo, s.slot);
    assert_eq!(s.coins.len(), 1, "the curve coin is not snapshotted");
    let c = &s.coins[0];
    assert_eq!((c.mint, c.vault_wx), (COIN, vault_of(&COIN)));
    assert_eq!(c.wx, 9_000, "wx(c) is K_coin[c]'s wX balance");
    assert_eq!(c.supply, 2_000);
    assert_eq!(c.circ, 900, "circ: the supply less the desk key's and the pool vault's balances");
    assert_eq!(c.holders, 2);
    // weight(w) = ⌊balance × wx / circ⌋: 600 × 9000 / 900 and 300 × 9000 / 900
    let mut expect = vec![(holder(1), 6_000u64), (holder(2), 3_000)];
    expect.sort_unstable();
    let mut cum = 0;
    for (l, (owner, weight)) in s.leaves.iter().zip(&expect) {
        assert_eq!((l.owner, l.weight), (*owner, *weight));
        assert_eq!(l.cum_start, cum, "cum_start is the prefix sum");
        cum += weight;
    }
    assert_eq!(s.leaves.len(), 2);
    assert_eq!(s.total_weight, 9_000, "the total is Σ wx(c) when nothing is floored");
}

#[test]
fn the_canister_keys_and_the_pool_vaults_are_excluded_with_their_reasons() {
    let mut w = standard();
    let s = take(&mut w, 1).unwrap();
    let reasons: BTreeSet<(Pubkey, Pubkey, u64, String)> = s.excluded.iter().map(|e| (e.account, e.owner, e.amount, e.reason.clone())).collect();
    let desk = pk(KeyPath::Desk(0));
    assert!(soltx::is_on_curve(&desk), "the desk key is on the curve: only its name excludes it");
    assert!(reasons.contains(&(soltx::associated_token_address(&desk, &COIN), desk, 100, "canister key K_desk[0]".into())), "{reasons:?}");
    assert!(reasons.contains(&(pool_vault(&COIN), soltx::key(DBC_POOL_AUTHORITY), 1_000, "DBC pool vault".into())), "{reasons:?}");
    assert_eq!(s.excluded.len(), 2);
    assert!(s.leaves.iter().all(|l| l.owner != desk), "an excluded owner has no leaf");
    assert_eq!(s.excluded.iter().map(|e| e.amount).sum::<u64>() + s.coins[0].circ, s.coins[0].supply, "excluded + circ = supply");
}

#[test]
fn every_canister_key_is_excluded_not_just_the_desk() {
    // K_fc holds coins (the partner's fee share, say), K_main too, and an open intent's deposit address: none of them holds a leaf
    let kfc = pk(KeyPath::Claimer);
    let kmain = me();
    let mut w = world(9_000, &[(holder(1), 100), (kfc, 100), (kmain, 100), (pk(KeyPath::Treasury), 100), (pk(KeyPath::Coll(0)), 100)], 0);
    let s = take(&mut w, 1).unwrap();
    assert_eq!(s.leaves.len(), 1);
    assert_eq!(s.leaves[0].owner, holder(1));
    assert_eq!(s.leaves[0].weight, 9_000, "the only holder carries the whole backing");
    let why: BTreeSet<String> = s.excluded.iter().map(|e| e.reason.clone()).collect();
    for name in ["K_fc", "K_main", "K_tre", "K_coll[0]"] {
        assert!(why.contains(&format!("canister key {name}")), "{why:?}");
    }
    assert_eq!(s.coins[0].circ, 100);
}

#[test]
fn the_root_recomputes_from_the_leaves_with_the_books_leaf_hash() {
    let mut w = world(9_000, &[(holder(1), 600), (holder(2), 300), (holder(3), 50), (holder(4), 49), (holder(5), 1)], 0);
    let s = take(&mut w, 3).unwrap();
    assert_eq!(s.leaves.len(), 5);
    let hashes: Vec<[u8; 32]> = s.leaves.iter().map(|l| raffle_leaf(&l.owner, l.cum_start, l.weight)).collect();
    assert_eq!(raffle::root_of(&hashes), s.root, "the books' raffle_leaf and the snapshot's tree agree");
    assert_ne!(s.root, [0; 32]);
    let lv = raffle::levels(&hashes);
    for (i, h) in hashes.iter().enumerate() {
        assert!(raffle::verify(&raffle::proof_of(&lv, i), &s.root, *h), "leaf {i} verifies");
    }
    // floors: 50 × 9000 / 1000 = 450, 49 × 9000 / 1000 = 441, 1 × 9000 / 1000 = 9: the total is under Σ wx(c)
    assert_eq!(s.total_weight, 5_400 + 2_700 + 450 + 441 + 9);
    assert_eq!(s.leaves.last().map(|l| l.cum_start + l.weight), Some(s.total_weight));
}

#[test]
fn winner_is_the_leaf_whose_range_holds_the_ticket_and_the_books_claim_takes_it() {
    let mut w = world(9_000, &[(holder(1), 600), (holder(2), 300), (holder(3), 100)], 0);
    let round = 2;
    let s = take(&mut w, round).unwrap();
    let mut seen = BTreeSet::new();
    for seed in 0..40u8 {
        let randomness = [seed; 32];
        let ticket = raffle_ticket(&randomness, &RKEY, round, s.total_weight);
        let (leaf, proof) = raffle_books::winner(&s, &randomness, &RKEY, round).expect("a winner");
        assert!(leaf.cum_start <= ticket && ticket < leaf.cum_start + leaf.weight, "ticket {ticket} in {leaf:?}");
        assert!(s.leaves.contains(&leaf));
        assert!(raffle::verify(&proof, &s.root, raffle_leaf(&leaf.owner, leaf.cum_start, leaf.weight)));
        // the books: a drawn raffle with this root and ticket pays the leaf and nobody else
        let mut r = Raffle::new(RKEY);
        r.phase = RafflePhase::Drawn;
        r.root = s.root;
        r.total_weight = s.total_weight;
        r.round = round;
        r.ticket = ticket;
        r.prize = ONE;
        r.drawn_at = 1_000;
        let other = s.leaves.iter().find(|l| **l != leaf).unwrap();
        assert!(r.clone().claim(other.owner, other.cum_start, other.weight, &proof, 1_001).is_err(), "another leaf is refused");
        assert_eq!(r.claim(leaf.owner, leaf.cum_start, leaf.weight, &proof, 1_001), Ok(ONE));
        assert_eq!(r.winner, leaf.owner);
        seen.insert(leaf.owner);
    }
    assert!(seen.len() >= 2, "different randomness picks different holders: {seen:?}");
    // the ticket is keyed by the books' key and round: another key or round moves it
    let r0 = raffle_books::winner(&s, &[7; 32], &RKEY, round).unwrap().0;
    let tickets: BTreeSet<u64> = [(RKEY, round), ([0xAB; 32], round), (RKEY, round + 1)].iter()
        .map(|(k, r)| raffle_ticket(&[7; 32], k, *r, s.total_weight)).collect();
    assert!(tickets.len() > 1, "{tickets:?}");
    assert!(s.leaves.contains(&r0));
}

#[test]
fn no_backed_coin_is_a_soft_refusal_and_a_straddled_read_is_hard() {
    let mut w = standard();
    coins::with_mut(|s| s.coins.get_mut(&COIN).unwrap().stage = CoinStage::Migrated);
    let e = take(&mut w, 1).unwrap_err();
    assert!(e.soft, "{e:?}");
    assert!(e.why.contains("no backed coin"), "{e:?}");

    let mut w = world(9_000, &[(holder(1), 600)], 1);
    let e = take(&mut w, 1).unwrap_err();
    assert!(!e.soft, "{e:?}");
    assert!(e.why.contains("sum to 600") && e.why.contains("supply is 601"), "{e:?}");

    // a backed coin whose vault is empty is skipped, and with nothing else the refusal is soft too
    let mut w = world(0, &[(holder(1), 600)], 0);
    let e = take(&mut w, 1).unwrap_err();
    assert!(e.soft && e.why.contains("no graduated coin with backing"), "{e:?}");
}

#[test]
fn the_snapshot_publishes_and_commits_like_the_on_chain_one() {
    let mut w = standard();
    let s = take(&mut w, 1).unwrap();
    raffle::publish(s.clone());
    assert_eq!(raffle::published(&WX, 1).as_ref(), Some(&s));
    let page = raffle::raffle_snapshot(soltx::b58_encode(&WX), 1, 0, 10).unwrap();
    assert_eq!(page.entries, 2);
    assert_eq!(page.excluded.len(), 2);
    assert_eq!(page.root, hex::encode(s.root));
    // the books commit it: the snapshot's slot is before the commit's finalized slot
    let now = w.0.now as i64;
    let committed = state::with_mut(|st| {
        let r = st.books.collections.get_mut(&COLL).unwrap().raffle.as_mut().unwrap();
        r.prize = ONE;
        r.commit(s.root, s.total_weight, s.slot, s.leaves.len() as u32, s.slot + 1, now)
    });
    assert!(committed.is_ok(), "{committed:?}");
    let r = state::with(|st| st.books.get(COLL).unwrap().raffle.clone().unwrap());
    assert_eq!((r.phase, r.round, r.root, r.total_weight, r.entries, r.snapshot_slot), (RafflePhase::Committed, 1, s.root, 9_000, 2, s.slot));
}
