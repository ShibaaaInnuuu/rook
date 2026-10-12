//! W9: the treasury, the draws and the raffle prize as cranks on the books (`cranks`), against the fake chain.
mod desk_common;
mod fakerpc;

use std::cell::RefCell;
use std::collections::HashMap;

use desk_common::*;
use etch_autobuyer::books::ledger::Destination;
use etch_autobuyer::books::{self, ONE};
use etch_autobuyer::cranks::{self, Owed, TreasuryStep};
use etch_autobuyer::desk::state;
use etch_autobuyer::drand;
use etch_autobuyer::crankline::CrankIo;
use etch_autobuyer::solhome::{LineId, LineStep};
use etch_autobuyer::solkey::KeyPath;
use etch_autobuyer::soltx::{self, Pubkey};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

const SOL: u64 = 1_000_000_000;

thread_local! {
    static BEACONS: RefCell<HashMap<(String, u64), (u32, Vec<u8>)>> = RefCell::new(HashMap::new());
}

impl CrankIo for World {
    fn now(&self) -> u64 {
        self.now
    }
    fn paused(&self) -> bool {
        false
    }
    fn drand_relays(&self) -> Vec<String> {
        vec!["https://a.drand".into(), "https://b.drand".into()]
    }
    async fn drand_fetch(&mut self, base: &str, round: u64) -> Result<Value, String> {
        let (status, body) = BEACONS.with(|b| b.borrow().get(&(base.to_string(), round)).cloned()).unwrap_or((404, b"{}".to_vec()));
        Ok(drand::normalise(status, &body))
    }
}

/// A real evmnet beacon's signature (`tests/fixtures/solana/drand-evmnet-20.json`): the canister verifies the BLS signature itself.
fn real_sig(round: u64) -> [u8; 64] {
    let v: Value = serde_json::from_str(include_str!("fixtures/solana/drand-evmnet-20.json")).unwrap();
    let b = v["beacons"].as_array().unwrap().iter().find(|b| b["round"] == round).expect("a real round");
    hex::decode(b["signature"].as_str().unwrap()).unwrap().try_into().unwrap()
}

fn serve(round: u64, sig: &[u8; 64]) {
    let body = json!({ "round": round, "signature": hex::encode(sig), "randomness": hex::encode(Sha256::digest(sig)) }).to_string().into_bytes();
    BEACONS.with(|b| {
        for base in ["https://a.drand", "https://b.drand"] {
            b.borrow_mut().insert((base.into(), round), (200, body.clone()));
        }
    });
}

fn tre() -> Pubkey {
    pk(KeyPath::Treasury)
}

fn tick(w: &mut World) -> Vec<String> {
    let now = w.now;
    block_on(cranks::tick(w, now))
}

fn round(w: &mut World) -> Vec<String> {
    let mut l = tick(w);
    w.settle();
    l.extend(tick(w));
    w.settle();
    l.extend(tick(w));
    l
}

fn world() -> World {
    let w = World::new();
    cranks::reset();
    BEACONS.with(|b| b.borrow_mut().clear());
    w
}

fn treasury() -> books::treasury::Treasury {
    state::with(|s| s.books.treasury.clone())
}

#[test]
fn a_pending_treasury_change_is_applied_once_its_day_is_over() {
    let mut w = world();
    let now = w.now as i64;
    state::with_mut(|s| {
        s.books.treasury.set_shares(8_000, 2_000, now).unwrap();
        // the first ops key is set at once; a change of it waits its day
        s.books.treasury.set_ops([0x55; 32], now).unwrap();
        s.books.treasury.set_ops([0x66; 32], now).unwrap();
    });
    tick(&mut w);
    assert_eq!((treasury().reserves_bps, treasury().ops), (9_000, Some([0x55; 32])), "not yet");
    w.advance_time(86_400 + 1);
    cranks::with_mut(|s| s.next.clear());
    let l = tick(&mut w);
    assert!(l.iter().any(|x| x.contains("shares change is applied")) && l.iter().any(|x| x.contains("ops change is applied")), "{l:?}");
    assert_eq!((treasury().reserves_bps, treasury().ops_bps, treasury().ops), (8_000, 2_000, Some([0x66; 32])));
}

#[test]
fn income_is_allocated_distributed_and_the_surplus_and_ops_shares_are_paid_from_the_treasury_key() {
    let mut w = world();
    let now = w.now as i64;
    // income at K_tre (booked and on chain), an ops key named and applied, some volume so the collection has a weight
    w.deposit_sol(&tre(), 20 * SOL);
    state::with_mut(|s| {
        s.books.receive(10 * SOL).unwrap();
        s.books.treasury.ops = Some([0x55; 32]);
        let m = s.books.mint(COLL, SOL, 0, now).unwrap();
        s.books.collections.get_mut(&COLL).unwrap().ledger.sol_backed += m.units;
    });
    w.inject_supply(state::with(|s| s.books.get(COLL).unwrap().ledger.sol_backed) as i64);
    w.deposit_sol(&module(), SOL);
    let l = tick(&mut w);
    assert!(l.iter().any(|x| x.contains("allocated (9000000000 reserves, 1000000000 ops)")), "{l:?}");
    assert!(l.iter().any(|x| x.contains("collection 0 distributed")), "{l:?}");
    let t = treasury();
    assert_eq!((t.unallocated, t.total_to_ops), (0, SOL));
    let owed = cranks::with(|s| s.owed.clone());
    let surplus = owed.iter().find_map(|o| match o { Owed::Surplus { coll: 0, lamports } => Some(*lamports), _ => None }).expect("surplus owed");
    assert!(owed.iter().any(|o| matches!(o, Owed::Ops { lamports } if *lamports >= SOL)), "{owed:?}");
    assert!(surplus > 0 && surplus <= 9 * SOL, "{surplus}");
    // review F2: the surplus is a chain link: booked at its begin (the seam's record-before-sign), gated by pre_sign, undone if the
    // transfer never lands; never a bare book entry the chain may not match
    assert_eq!(state::with(|s| s.books.get(COLL).unwrap().module.surplus), surplus, "booked by the open link");
    assert!(state::with(|s| s.link.open_of(COLL, None).next().is_some()), "the Surplus link is open until it lands");
    // the surplus transfer goes out first, from K_tre to the module, signed by the treasury key; then the ops share
    let module_before = w.lamports(&module());
    let mut l = round(&mut w);
    l.extend(round(&mut w));
    assert!(l.iter().any(|x| x.contains("of surplus to module 0") && x.contains("landed")), "{l:?}");
    assert_eq!(w.lamports(&module()), module_before + surplus);
    assert_eq!(state::with(|s| s.books.get(COLL).unwrap().module.surplus), surplus, "still booked once the link landed");
    assert!(state::with(|s| s.link.open_of(COLL, None).next().is_none()), "the link is decided");
    assert!(l.iter().any(|x| x.contains("of ops to") && x.contains("landed")), "{l:?}");
    let ops_paid = w.lamports(&[0x55; 32]);
    assert_eq!(ops_paid, treasury().total_to_ops, "the ops key got the whole ops share (the allocation's and the distribution's)");
    assert!(ops_paid >= SOL);
    assert!(cranks::with(|s| s.owed.is_empty()), "{:?}", cranks::with(|s| s.owed.clone()));
    assert_eq!(w.lamports(&tre()), 20 * SOL - surplus - ops_paid);
}

#[test]
fn the_next_pinned_ticket_is_resolved_with_the_drand_round_two_relays_agree_on_and_the_canister_verifies() {
    let mut w = world();
    let now = w.now as i64;
    // two items held: a redemption pins a ticket at a round 20 ahead; the test pins it at a real fixture round instead
    state::with_mut(|s| {
        for k in [1u8, 2] {
            let args = books::ledger::MintItemArgs { key: [k; 32], source: books::math::keccak(&[&[k; 32], &[3; 32]]), tx: [3; 32], vout: 0, deposit: false,
                inscription_id: String::new(), proof: vec![] };
            s.books.mint_item(COLL, args, books::MintTo::Holder(wallet(1)), now).unwrap();
        }
        s.books.redeem(COLL, wallet(1), Destination::Eth([0xe1; 20]), now).unwrap();
    });
    let real = 20_025_637u64;
    state::with_mut(|s| s.books.collections.get_mut(&COLL).unwrap().ledger.tickets.get_mut(&0).unwrap().round = real);
    let needed = drand::GENESIS + (real - 1) * drand::PERIOD + 1;
    if w.now < needed {
        w.advance_time(needed - w.now);
    }
    // nobody serves the round yet: it waits
    let l = tick(&mut w);
    assert!(l.iter().any(|x| x.contains("waits for drand round 20025637")), "{l:?}");
    // one relay serves a wrong signature: a disagreement, nothing is resolved
    let bad = [7u8; 64];
    BEACONS.with(|b| b.borrow_mut().insert(("https://a.drand".into(), real), (200, json!({ "round": real, "signature": hex::encode(bad), "randomness": hex::encode(Sha256::digest(bad)) }).to_string().into_bytes())));
    serve(real, &real_sig(real));
    BEACONS.with(|b| b.borrow_mut().insert(("https://a.drand".into(), real), (200, json!({ "round": real, "signature": hex::encode(bad), "randomness": hex::encode(Sha256::digest(bad)) }).to_string().into_bytes())));
    cranks::with_mut(|s| s.next.clear());
    let l = tick(&mut w);
    assert!(l.iter().any(|x| x.contains("waits for drand round")), "{l:?}");
    assert!(cranks::with(|s| s.drand_disagreement.as_ref().is_some_and(|(r, ..)| *r == real)));
    // both relays serve the real beacon: verified, resolved
    serve(real, &real_sig(real));
    cranks::with_mut(|s| s.next.clear());
    let l = tick(&mut w);
    assert!(l.iter().any(|x| x.contains("ticket 0 of collection 0 resolved to item")), "{l:?}");
    let t = state::with(|s| s.books.get(COLL).unwrap().ledger.ticket(0).cloned().unwrap());
    assert_eq!(t.stage, books::ledger::Stage::Resolved);
    assert!(t.deliver_by > now);
    assert!(cranks::with(|s| s.drand_disagreement.is_none()), "cleared once proven");
}

#[test]
fn every_collection_gets_a_raffle_and_a_funded_pot_buys_the_prize_at_the_module_through_a_link() {
    let mut w = world();
    let now = w.now as i64;
    let l = tick(&mut w);
    assert!(l.iter().any(|x| x.contains("collection 0's raffle is opened")), "{l:?}");
    let price = state::with(|s| s.books.get(COLL).unwrap().module.mint_price(now, 0).unwrap());
    assert!(price > FLOOR && price < FLOOR * 102 / 100, "{price}");
    // the pot covers a prize; the treasury key holds the SOL
    w.deposit_sol(&tre(), price + SOL);
    state::with_mut(|s| s.books.fund_raffle(COLL, price).unwrap());
    let supply_before = w.supply();
    let l = round(&mut w);
    assert!(l.iter().any(|x| x.contains("raffle 0's prize, one wX at") && x.contains("landed")), "{l:?}");
    assert_eq!(w.supply(), supply_before + ONE, "one whole wX minted");
    assert_eq!(w.token(&tre()), ONE, "held at the treasury's wX account");
    assert_eq!(w.lamports(&module()), RENT0 + price, "the module got the mint's SOL");
    let r = state::with(|s| s.books.get(COLL).unwrap().raffle.clone().unwrap());
    assert_eq!((r.prize, r.pot, r.prizes_funded), (ONE, 0, 1));
    // the link landed: the books and the chain agree
    assert_eq!(state::with(|s| s.books.get(COLL).unwrap().ledger.sol_backed), ONE);
}

#[test]
fn the_state_round_trips_and_owed_shares_add_up_and_pay_down() {
    let mut s = cranks::CranksState::default();
    s.owe(Owed::Surplus { coll: 1, lamports: 5 });
    s.owe(Owed::Surplus { coll: 1, lamports: 7 });
    s.owe(Owed::Ops { lamports: 3 });
    assert_eq!(s.owed, vec![Owed::Surplus { coll: 1, lamports: 12 }, Owed::Ops { lamports: 3 }]);
    s.paid(&TreasuryStep::Surplus { coll: 1, lamports: 12, link: 0 });
    s.paid(&TreasuryStep::Ops { to: [0; 32], lamports: 1 });
    assert_eq!(s.owed, vec![Owed::Ops { lamports: 2 }]);
    s.wait("x".into(), "why".into(), 1);
    assert_eq!(cranks::decode_state(&cranks::encode_state(&s)).unwrap(), s);
    assert!(cranks::decode_state(b"nope").is_err());
    let _ = LineStep::Treasury(TreasuryStep::Ops { to: [0; 32], lamports: 1 }).label();
    let _ = (LineId::Crank, soltx::SYSTEM_PROGRAM);
}

/// W9 + `raffle_books`: a whole raffle round on the books: the snapshot of the collection's one backed coin published, the round committed at
/// a later finalized slot, drawn with the pinned drand round once two relays agree, and the winner paid one wX from the treasury's wX account.
#[test]
fn a_raffle_round_is_snapshotted_committed_drawn_and_its_winner_paid_from_the_treasury() {
    use ed25519_dalek::SigningKey;
    use etch_autobuyer::coins::state::{self as coins, CoinRec, CoinStage, Vault};
    let holder = |n: u8| -> Pubkey { SigningKey::from_bytes(&[0x30 + n; 32]).verifying_key().to_bytes() };
    let tok = soltx::key(soltx::TOKEN_PROGRAM);
    let coin: Pubkey = [0xC1; 32];
    let mut w = world();
    let now = w.now as i64;
    // the raffle opened by the first pass; its prize held already (a funded prize: one wX at the treasury's account, booked SOL-backed)
    tick(&mut w);
    state::with_mut(|s| {
        let c = s.books.collections.get_mut(&COLL).unwrap();
        c.raffle.as_mut().unwrap().prize = ONE;
        c.ledger.sol_backed += ONE;
    });
    w.plant(soltx::associated_token_address(&tre(), &WX), tok, RENT165, token_data(WX, tre(), ONE));
    w.inject_supply(ONE as i64);
    // one backed coin with 9,000 wX of backing: two holders, 600 and 300 of a supply of 900
    let kcoin = pk(KeyPath::Coin(coin));
    let vault = soltx::associated_token_address(&kcoin, &WX);
    w.plant(coin, tok, 1_461_600, mint_data([0x99; 32], 900));
    w.plant(vault, tok, RENT165, token_data(WX, kcoin, 9_000));
    w.inject_supply(9_000);
    state::with_mut(|s| s.books.collections.get_mut(&COLL).unwrap().ledger.sol_backed += 9_000);
    for (o, n) in [(holder(1), 600u64), (holder(2), 300)] {
        w.plant(soltx::associated_token_address(&o, &coin), tok, RENT165, token_data(coin, o, n));
    }
    coins::with_mut(|s| {
        s.coins.insert(coin, CoinRec { coll: COLL as u32, coin, pool: [0x55; 32], creator: holder(9), launched_at: 1, seq: 0, stage: CoinStage::Backed,
            vault: Some(Vault { address: vault, owner: kcoin, wx_in: 9_000, ..Default::default() }), damm_pool: None, partner_nft: None, surplus_tries: 0 });
    });
    // 1. the snapshot, published before any commit
    cranks::with_mut(|s| s.next.clear());
    let l = tick(&mut w);
    assert!(l.iter().any(|x| x.contains("snapshot of 2 holders") && x.contains("published")), "{l:?}");
    let pc = cranks::with(|s| s.commits.get(&0).cloned()).expect("a pending commit");
    assert_eq!((pc.round, pc.entries, pc.total_weight), (1, 2, 9_000));
    assert!(etch_autobuyer::raffle::published(&WX, 1).is_some(), "served by raffle_snapshot");
    // 2. committed once the finalized slot has moved past the snapshot's
    w.settle();
    cranks::with_mut(|s| s.next.clear());
    let l = tick(&mut w);
    assert!(l.iter().any(|x| x.contains("round 1 committed: drawn with drand round")), "{l:?}");
    let r = state::with(|s| s.books.get(COLL).unwrap().raffle.clone().unwrap());
    assert_eq!((r.round, r.phase, r.root, r.total_weight), (1, books::raffle::RafflePhase::Committed, pc.root, 9_000));
    // 3. drawn with the pinned round (pinned here at a real fixture round the relays serve)
    let real = 20_125_640u64;
    state::with_mut(|s| s.books.collections.get_mut(&COLL).unwrap().raffle.as_mut().unwrap().drand_round = real);
    let needed = drand::GENESIS + (real - 1) * drand::PERIOD + 1;
    if w.now < needed {
        w.advance_time(needed - w.now);
    }
    cranks::with_mut(|s| s.next.clear());
    let l = tick(&mut w);
    assert!(l.iter().any(|x| x.contains("waits for drand round")), "{l:?}");
    serve(real, &real_sig(real));
    cranks::with_mut(|s| s.next.clear());
    let l = tick(&mut w);
    assert!(l.iter().any(|x| x.contains("round 1 drawn: ticket") && x.contains("the winner is")), "{l:?}");
    let claim = cranks::with(|s| s.claims.get(&0).cloned()).expect("a winner");
    assert!(claim.owner == holder(1) || claim.owner == holder(2));
    assert!(claim.weight == 6_000 || claim.weight == 3_000);
    // 4. the prize: one wX from the treasury's wX account to the winner's, then the books' claim
    let l = round(&mut w);
    assert!(l.iter().any(|x| x.contains("'s prize to") && x.contains("landed")), "{l:?}");
    assert_eq!(w.token(&claim.owner), ONE);
    assert_eq!(w.token(&tre()), 0);
    let r = state::with(|s| s.books.get(COLL).unwrap().raffle.clone().unwrap());
    assert_eq!((r.phase, r.prize, r.winner, r.prizes_claimed), (books::raffle::RafflePhase::Idle, 0, claim.owner, 1));
    assert!(cranks::with(|s| s.claims.is_empty()));
    let _ = now;
}

#[test]
fn sol_fees_at_the_claimer_are_swept_to_the_treasury_and_booked_as_income() {
    let mut w = world();
    let fc = pk(KeyPath::Claimer);
    w.deposit_sol(&fc, SOL);
    let before = treasury().total_received;
    let mut l = round(&mut w);
    l.extend(round(&mut w));
    assert!(l.iter().any(|x| x.contains("SOL fees from the claimer to the treasury") && x.contains("landed")), "{l:?}");
    assert_eq!(w.lamports(&fc), cranks::KEY_RENT_LAMPORTS, "the key keeps its rent");
    assert_eq!(w.lamports(&tre()), SOL - cranks::KEY_RENT_LAMPORTS);
    assert_eq!(treasury().total_received, before + SOL - cranks::KEY_RENT_LAMPORTS, "booked when it landed");
    // under the bound nothing moves
    w.deposit_sol(&fc, cranks::CLAIMER_SWEEP_MIN_LAMPORTS - 1);
    cranks::with_mut(|s| s.next.clear());
    let l = round(&mut w);
    assert!(!l.iter().any(|x| x.contains("claimer")), "{l:?}");
}

#[test]
fn a_resolved_ticket_marked_undeliverable_is_refunded_past_its_grace_one_wx_re_minted_to_its_owner_through_a_link() {
    let mut w = world();
    let now = w.now as i64;
    // two items held by deposits (two wX minted), one redeemed (one burned) and drawn; the canister cannot deliver it
    state::with_mut(|s| {
        for k in [1u8, 2] {
            let args = books::ledger::MintItemArgs { key: [k; 32], source: books::math::keccak(&[&[k; 32], &[3; 32]]), tx: [3; 32], vout: 0, deposit: false,
                inscription_id: String::new(), proof: vec![] };
            s.books.mint_item(COLL, args, books::MintTo::Holder(wallet(1)), now).unwrap();
        }
        s.books.redeem(COLL, wallet(1), Destination::Eth([0xe1; 20]), now).unwrap();
        // the ticket's round is pinned ahead: drawn as the books draw once it is live (the crank's drand path is tested above)
        let live = drand::GENESIS as i64 + (s.books.get(COLL).unwrap().ledger.ticket(0).unwrap().round as i64) * drand::PERIOD as i64;
        s.books.resolve(COLL, 0, Some([9u8; 32]), live).unwrap();
        s.books.mark_undeliverable(COLL, 0).unwrap();
    });
    w.inject_supply(ONE as i64);
    // not yet: the grace after its delivery deadline has not passed
    let l = round(&mut w);
    assert!(!l.iter().any(|x| x.contains("re-minted")), "{l:?}");
    state::with_mut(|s| s.books.collections.get_mut(&COLL).unwrap().ledger.tickets.get_mut(&0).unwrap().deliver_by = now - books::ledger::REFUND_GRACE - 1);
    cranks::with_mut(|s| s.next.clear());
    let supply_before = w.supply();
    let mut l = round(&mut w);
    l.extend(round(&mut w));
    assert!(l.iter().any(|x| x.contains("ticket 0 of collection 0 refunded, one wX re-minted to") && x.contains("landed")), "{l:?}");
    assert_eq!(w.token(&wallet(1)), ONE, "the owner got the wX back");
    assert_eq!(w.supply(), supply_before + ONE);
    let t = state::with(|s| s.books.get(COLL).unwrap().ledger.ticket(0).cloned().unwrap());
    assert_eq!(t.stage, books::ledger::Stage::Refunded);
    assert_eq!(state::with(|s| s.books.get(COLL).unwrap().ledger.open_tickets), 0);
    assert!(state::with(|s| s.link.open_of(COLL, None).next().is_none()), "the link is decided");
    // once only
    cranks::with_mut(|s| s.next.clear());
    let l = round(&mut w);
    assert!(!l.iter().any(|x| x.contains("re-minted")), "{l:?}");
    assert_eq!(w.token(&wallet(1)), ONE);
}

#[test]
fn revenue_above_a_modules_target_is_skimmed_to_the_treasury_through_a_link_and_booked_as_income() {
    let mut w = world();
    let now = w.now as i64;
    let target = state::with(|s| s.books.get(COLL).unwrap().module.params.revenue_target);
    // the module holds revenue above its target, on chain and in the books; under the bound nothing moves
    state::with_mut(|s| s.books.collections.get_mut(&COLL).unwrap().module.revenue = target + cranks::SKIM_MIN_LAMPORTS - 1);
    w.deposit_sol(&module(), target + 2 * SOL);
    let l = round(&mut w);
    assert!(!l.iter().any(|x| x.contains("skimmed")), "{l:?}");
    state::with_mut(|s| s.books.collections.get_mut(&COLL).unwrap().module.revenue = target + 2 * SOL);
    cranks::with_mut(|s| s.next.clear());
    let (tre_before, received_before) = (w.lamports(&tre()), treasury().total_received);
    let mut l = round(&mut w);
    l.extend(round(&mut w));
    assert!(l.iter().any(|x| x.contains("2000000000 lamports of revenue above target skimmed from module 0") && x.contains("landed")), "{l:?}");
    assert_eq!(w.lamports(&tre()), tre_before + 2 * SOL);
    assert_eq!(w.lamports(&module()), RENT0 + target);
    let m = state::with(|s| s.books.get(COLL).unwrap().module.clone());
    assert_eq!(m.revenue, target, "the target stays at the module");
    assert_eq!(treasury().total_received, received_before + 2 * SOL, "booked as income by the link's begin");
    assert!(state::with(|s| s.link.open_of(COLL, None).next().is_none()), "the link is decided");
    let _ = now;
}

/// The bytes the monitor decodes (monitor/test/books.test.mjs): `WRITE_FIXTURES=1` rewrites them.
fn view_fixture(file: &str, bytes: &[u8]) {
    let path = format!("{}/tests/fixtures/views/{file}", env!("CARGO_MANIFEST_DIR"));
    let hex = hex::encode(bytes);
    if std::env::var("WRITE_FIXTURES").is_ok() {
        std::fs::write(&path, format!("{hex}\n")).unwrap();
    }
    assert_eq!(std::fs::read_to_string(&path).unwrap().trim(), hex, "{file} is stale: WRITE_FIXTURES=1, then the monitor's tests");
}

#[test]
fn the_treasury_status_bytes_are_the_monitors_fixture() {
    let mut w = world();
    let now = w.now as i64;
    w.deposit_sol(&tre(), 20 * SOL);
    state::with_mut(|s| {
        s.books.receive(10 * SOL).unwrap();
        s.books.treasury.ops = Some([0x55; 32]);
        let m = s.books.mint(COLL, SOL, 0, now).unwrap();
        s.books.collections.get_mut(&COLL).unwrap().ledger.sol_backed += m.units;
    });
    w.inject_supply(state::with(|s| s.books.get(COLL).unwrap().ledger.sol_backed) as i64);
    tick(&mut w);
    // a wait of 7 hours, a drand disagreement, a resume scheduled, a commit pending: everything the monitor pages on, at once
    cranks::with_mut(|s| {
        s.wait("ops".into(), "the ops key refused the transfer".into(), 1_700_000_000);
        s.drand_disagreement = Some((20_025_637, 1_700_000_000, "a.drand served other bytes".into()));
        s.resumes.push((cranks::Resume::Collection, 0, 1_700_100_000));
        s.commits.insert(0, cranks::PendingCommit { round: 1, root: [1; 32], total_weight: 9_000, snapshot_slot: 100, entries: 2, taken_at: 1_700_000_000 });
    });
    let v = cranks::treasury_status();
    view_fixture("treasury-status.hex", &candid::encode_one(&v).unwrap());
}
