//! Audit 2026-10 regressions (docs/v9/audit-2026-10, areas A1 and A2) carried into the canister's books. Ported from
//! solana/programs/rook_core/tests/audit_a1_core.rs and audit_a2_core.rs (names kept). Every fix of 2026-10-10 must hold here:
//! A1-01 (the poster switch on every listing kind), A1-04 (the TWAP's out-of-order fill, in `Module::price`), A2-01 (a
//! distribution round cannot be restarted to pay one collection twice), A2-02 (no randomness of a round that is not live),
//! A2-03 (no redemption to the custody address), A2-04 (no refund of a pinned ticket whose round is proven), A2-09 (the held
//! marker must point to the delivered item). A1-05 is FIXED by construction here (`demo_` → `fixed_`).

mod books_common;

use books_common::*;
use etch_autobuyer::books::ledger::MintItemArgs;
use etch_autobuyer::books::math::{keccak, leaf_of, round_at, BTC_BURN_ADDRESS};
use etch_autobuyer::books::treasury::ROUND_TIMEOUT;
use etch_autobuyer::books::*;

// ---- A1 ---------------------------------------------------------------------------------------------------------------------

// rook_core audit_a1_core.rs: poc_a1_01_pausing_an_inscriptions_poster_stops_its_floor
#[test]
fn poc_a1_01_pausing_an_inscriptions_poster_stops_its_floor() {
    let mut env = Env::bare();
    ok(env.b.add_collection(ID, ListingKind::Inscriptions { membership_root: [7u8; 32], custody: BTC_BURN_ADDRESS.into() }, [1; 32], test_params()));
    ok(env.post(FLOOR));
    env.pause_poster();
    assert!(env.b.get(ID).unwrap().poster_paused, "the switch is recorded");
    env.advance_time(600);
    expect_err(env.post(FLOOR * 104 / 100), BooksError::NotKeeper);
    expect_err(env.mint_units(USER, 2 * FLOOR, 0), BooksError::Paused);
}

// rook_core audit_a1_core.rs: demo_a1_01_control_the_switch_works_on_an_ethnft_listing
#[test]
fn demo_a1_01_control_the_switch_works_on_an_ethnft_listing() {
    let mut env = Env::new();
    ok(env.post(FLOOR));
    env.pause_poster();
    expect_err(env.post(FLOOR), BooksError::NotKeeper);
    expect_err(env.mint_units(USER, 2 * FLOOR, 0), BooksError::Paused);
}

// rook_core audit_a1_core.rs: demo_a1_02_anyone_lowers_the_snapshot_cap (Low, inherited). Canister-only nobody else can call
// `snapshot_cap`; the rule itself (a later snapshot replaces an earlier one) is unchanged and shown here.
#[test]
fn demo_a1_02_anyone_lowers_the_snapshot_cap() {
    let mut env = Env::new();
    ok(env.post(FLOOR));
    ok(env.mint_units(USER, 4 * FLOOR, 0));
    ok(env.fund_reserve(10 * SOL));
    ok(env.wrap_nft(1, tx_hash(1), MintTo::Module));
    ok(env.snapshot());
    let cap = env.module().snap_cap;
    env.advance_time(3_600);
    ok(env.post(FLOOR / 2));
    ok(env.snapshot());
    let lowered = env.module().snap_cap;
    assert!(lowered < cap);
    expect_err(env.item_arrived(cap), BooksError::OverItemCap);
    ok(env.item_arrived(lowered));
}

// rook_core audit_a1_core.rs: demo_a1_05_a_stray_public_unit_satisfies_no_wrapped (Informational). FIXED by construction: the
// module's item units live in the ledger (`module_units`), credited only by `mint_item(.., MintTo::Module)`; a public mint
// (or any wX sent anywhere) can never satisfy `NoWrapped`.
#[test]
fn fixed_a1_05_a_stray_public_unit_cannot_satisfy_no_wrapped() {
    let mut env = Env::new();
    ok(env.post(FLOOR));
    ok(env.mint_units(USER, 2 * FLOOR, 0));
    ok(env.fund_reserve(10 * SOL));
    let held = env.ledger().held;
    expect_err(env.item_arrived(FLOOR), BooksError::NoWrapped);
    assert_eq!(env.ledger().held, held, "no item was wrapped");
    env.check_books();
    env.check_ledger();
}

// rook_core audit_a1_core.rs: measure_a1_q15_front_running_a_floor_post (accepted risk Q15, quantified)
#[test]
fn measure_a1_q15_front_running_a_floor_post() {
    // (a) an atomic sandwich of an upward post loses
    let mut env = Env::new();
    ok(env.post(FLOOR));
    ok(env.mint_units(USER, 4 * FLOOR, 0));
    env.advance_time(3_600);
    ok(env.post(FLOOR));
    env.advance_time(1_200);
    let a = wallet(1);
    let paid_in = FLOOR;
    let units = ok(env.mint_units(a, paid_in, 0));
    ok(env.post(FLOOR * 110 / 100));
    let out = ok(env.burn_units(a, a, units, 0));
    let pnl_a = out as i128 - paid_in as i128;
    assert!(pnl_a < 0, "an atomic sandwich of one post loses");
    env.check_books();
    // (b) and (c): mint before a post that lifts the target 25 % above a floor that fell 20 %, hold 4 h, burn
    for high_basis in [false, true] {
        let mut env = Env::new();
        ok(env.post(FLOOR));
        if high_basis {
            ok(env.mint_units(USER, 5 * FLOOR, 0));
        }
        for _ in 0..30 {
            env.advance_time(3_600);
            ok(env.post(FLOOR * 8 / 10));
        }
        if !high_basis {
            ok(env.mint_units(USER, 4 * FLOOR, 0));
            env.advance_time(3_600);
            ok(env.post(FLOOR * 8 / 10));
        }
        env.advance_time(1_200);
        let a = wallet(2);
        let paid_in = 4 * FLOOR;
        let units = ok(env.mint_units(a, paid_in, 0));
        for _ in 0..12 {
            ok(env.post(FLOOR));
            env.advance_time(1_200);
        }
        ok(env.post(FLOOR));
        env.advance_time(3_000);
        ok(env.post(FLOOR));
        let out = ok(env.burn_units(a, a, units, 0));
        let pnl = out as i128 - paid_in as i128;
        env.check_books();
        env.check_ledger();
        if high_basis {
            assert!(pnl > 0, "against a pool minted higher the gain is the jump");
        } else {
            assert!(pnl <= paid_in as i128 / 100, "against a low-basis pool the burn is capped at the backing");
        }
    }
}

// rook_core audit_a1_core.rs: measure_a1_books_ledger_and_compute_units (the books and the ledger after every money operation;
// compute units do not apply)
#[test]
fn measure_a1_books_ledger_and_compute_units() {
    let mut p = test_params();
    p.max_public_eth_backed = 20 * ONE; // rook_core's `convert` was not bounded by the public cap
    let mut env = Env::with_params(p);
    ok(env.post(FLOOR));
    ok(env.mint_units(USER, 4 * FLOOR, 0));
    env.check_books();
    env.check_ledger();
    env.seed_units(HOLDER, 2_000 * SOL);
    ok(env.fund_reserve(10 * SOL));
    env.check_books();
    env.check_ledger();
    ok(env.advance(FLOOR));
    env.check_books();
    ok(env.wrap_nft(1, tx_hash(1), MintTo::Module));
    ok(env.settle(FLOOR));
    env.check_books();
    env.check_ledger();
    let b = env.balance(&USER);
    ok(env.burn_units(USER, USER, b / 2, 0));
    env.check_books();
    env.check_ledger();
    for i in 0..8u64 {
        ok(env.advance(FLOOR));
        ok(env.wrap_nft(10 + i, tx_hash(10 + i), MintTo::Module));
        ok(env.settle(FLOOR - i));
        env.advance_time(86_400 / 2 + 1);
        ok(env.post(FLOOR));
    }
    ok(env.mint_units(USER, FLOOR, 0));
    ok(env.post(FLOOR));
    env.check_books();
    env.check_ledger();
    assert!(env.vault_free() as u128 >= env.module().books());
}

// A1-04 (fixed before the port): a fill stamped before the previous one cannot break the TWAP.
#[test]
fn fixed_a1_04_an_out_of_order_fill_does_not_break_the_price() {
    let mut env = Env::new();
    ok(env.post(FLOOR));
    let mut m = env.module();
    m.fills[0] = floor::Fill { price: FLOOR, at: env.now - 100 };
    m.fills[1] = floor::Fill { price: FLOOR / 2, at: env.now - 200 };
    m.fill_count = 2;
    let p = m.price(env.now).unwrap();
    assert!(p >= FLOOR * (BPS - 300) / BPS && p <= FLOOR * (BPS + 300) / BPS);
}

// ---- A2 ---------------------------------------------------------------------------------------------------------------------

/// Two collections with equal seven-day volume and 40 SOL in the reserves pot.
fn two_equal_collections() -> Env {
    let mut env = Env::new();
    env.add_collection(1, test_params());
    ok(env.post(FLOOR));
    ok(env.mint_units(USER, 100 * SOL, 0));
    env.use_listing(0);
    ok(env.post(FLOOR));
    ok(env.mint_units(USER, 100 * SOL, 0));
    assert_eq!(env.module_of(0).volume_7d(env.now), env.module_of(1).volume_7d(env.now), "equal weights");
    ok(env.receive(40 * SOL));
    ok(env.allocate());
    assert_eq!(env.b.treasury.reserves_pot, 40 * SOL);
    env
}

// rook_core audit_a2_core.rs: poc_a2_01_restarting_a_distribution_round_drains_the_pot_into_one_collection
#[test]
fn poc_a2_01_restarting_a_distribution_round_drains_the_pot_into_one_collection() {
    let mut env = two_equal_collections();
    for _ in 0..30 {
        if env.begin().is_err() {
            break;
        }
        if env.distribute(0).is_err() {
            break;
        }
    }
    let _ = env.begin();
    let _ = env.distribute(1);
    assert_eq!(env.module_of(0).surplus, 20 * SOL, "A2-01: collection 0 took more than its half");
    assert_eq!(env.module_of(1).surplus, 20 * SOL, "A2-01: collection 1 got less than its half");
    env.check_books();
}

// rook_core audit_a2_core.rs: poc_a2_01_a_racing_crank_pays_a_collection_twice
#[test]
fn poc_a2_01_a_racing_crank_pays_a_collection_twice() {
    let mut env = two_equal_collections();
    ok(env.begin());
    ok(env.distribute(0));
    expect_err(env.begin(), BooksError::RoundOpen);
    assert!(env.distribute(0).is_err(), "A2-01: collection 0 was paid a second time in the same distribution");
    ok(env.distribute(1));
    assert_eq!(env.module_of(0).surplus, 20 * SOL);
    assert_eq!(env.module_of(1).surplus, 20 * SOL);
    ok(env.begin());
}

// rook_core audit_a2_core.rs: fixed_a2_01_a_stale_round_restarts_after_a_day
#[test]
fn fixed_a2_01_a_stale_round_restarts_after_a_day() {
    let mut env = two_equal_collections();
    ok(env.begin());
    ok(env.distribute(0));
    env.advance_time(ROUND_TIMEOUT - 10);
    expect_err(env.begin(), BooksError::RoundOpen);
    env.advance_time(10);
    ok(env.begin());
    let t = &env.b.treasury;
    assert_eq!((t.round_pot, t.round_paid, t.round_started_at), (20 * SOL, 0, env.now));
}

// New: `distribute_all` under A2-01's rule: a second call while a round is open and young is refused.
#[test]
fn fixed_a2_01_distribute_all_cannot_restart_an_open_round() {
    let mut env = two_equal_collections();
    ok(env.begin());
    ok(env.distribute(0));
    expect_err(env.b.distribute_all(env.now).map(|_| ()), BooksError::RoundOpen);
    env.advance_time(ROUND_TIMEOUT);
    let ds = ok(env.b.distribute_all(env.now));
    for d in &ds {
        env.apply_distribution(d);
    }
    // the restart re-weighs what is left (20 SOL): each half of it; collection 0 got its first 20 SOL earlier
    assert_eq!((env.module_of(0).surplus, env.module_of(1).surplus), (30 * SOL, 10 * SOL));
    env.check_books();
}

// rook_core audit_a2_core.rs: fixed_a2_02_the_keeper_cannot_prove_a_round_that_is_not_live. The canister verifies BLS itself
// (D-i), so no randomness of a future round exists; the books refuse one anyway.
#[test]
fn fixed_a2_02_the_keeper_cannot_prove_a_round_that_is_not_live() {
    let mut env = Env::new();
    for t in 1..=4u64 {
        ok(env.wrap_nft(t, tx_hash(t), MintTo::Holder(USER)));
    }
    ok(env.redeem(USER, Destination::Eth([0x22; 20])));
    let tk = env.ticket(0);
    assert!(tk.round > round_at(env.now));
    expect_err(env.prove(round_at(env.now) + 100_000, [9u8; 32]), BooksError::RoundInFuture);
    expect_err(env.b.resolve(ID, 0, Some([9u8; 32]), env.now), BooksError::RoundInFuture);
    ok(env.prove(round_at(env.now), [9u8; 32]));
    env.reach_round(tk.round);
    ok(env.prove(tk.round, [5u8; 32]));
    ok(env.resolve_as_is(0));
    assert_eq!(env.ticket(0).stage, Stage::Resolved);
}

// rook_core audit_a2_core.rs: fixed_a2_04_a_proven_pinned_ticket_resolves_instead_of_refunding
#[test]
fn fixed_a2_04_a_proven_pinned_ticket_resolves_instead_of_refunding() {
    let mut env = Env::new();
    for t in 1..=3u64 {
        ok(env.wrap_nft(t, tx_hash(t), MintTo::Holder(USER)));
    }
    ok(env.redeem(USER, Destination::Eth([0x22; 20])));
    env.advance_time(120);
    ok(env.redeem(USER, Destination::Eth([0x22; 20])));
    let r0 = env.ticket(0).round;
    let r1 = env.ticket(1).round;
    assert!(r1 > r0);
    env.reach_round(r1);
    ok(env.prove(r1, [5u8; 32]));
    expect_err(env.resolve_as_is(1), BooksError::OutOfOrder);
    env.advance_time(7 * 86_400 + 200);
    expect_err(env.refund(1), BooksError::RoundProven);
    ok(env.refund(0));
    ok(env.resolve_as_is(1));
    assert_eq!((env.ticket(0).stage, env.ticket(1).stage), (Stage::Refunded, Stage::Resolved));
    env.check_ledger();
}

const TB_CUSTODY: &str = "tb1p5cyxnuxmeuwuvkwfem96lqzszd02n6xdcjrs20cac6yqjjwudpxqp3mvzv";

fn frog(id: &str, proof: Vec<[u8; 32]>, txid: [u8; 32]) -> MintItemArgs {
    MintItemArgs {
        key: leaf_of(id),
        source: keccak(&[&txid, &0u32.to_be_bytes()]),
        tx: txid,
        vout: 0,
        deposit: true,
        inscription_id: id.into(),
        proof,
    }
}

// rook_core audit_a2_core.rs: fixed_a2_03_an_inscription_redemption_to_custody_is_refused
#[test]
fn fixed_a2_03_an_inscription_redemption_to_custody_is_refused() {
    let mut env = Env::bare();
    let a = "6fb976ab49dcec017f1e201e84395983204ae1a7c2abf7ced0a85d692e442799i0";
    let b = "a1b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f90i1";
    let (la, lb) = (leaf_of(a), leaf_of(b));
    let root = if la <= lb { keccak(&[&la, &lb]) } else { keccak(&[&lb, &la]) };
    ok(env.b.add_collection(ID, ListingKind::Inscriptions { membership_root: root, custody: TB_CUSTODY.into() }, [1; 32], test_params()));
    let attacker = wallet(7);
    ok(env.mint_item(frog(a, vec![lb], tx_hash(1)), MintTo::Holder(attacker)));
    ok(env.mint_item(frog(b, vec![la], tx_hash(2)), MintTo::Holder(attacker)));
    expect_err(env.redeem(attacker, Destination::Taproot(TB_CUSTODY.into())), BooksError::CustodyDestination);
    let l = env.ledger();
    assert_eq!((l.held, l.open_tickets), (2, 0));
    assert_eq!(env.balance(&attacker), 2 * ONE, "nothing burned");
    env.check_ledger();
}

// A2-09 (fixed before the port): a delivery is recorded only while the item's held marker points to it; a second record of the
// same ticket is refused before its stage is even read.
#[test]
fn fixed_a2_09_the_held_marker_must_point_to_the_delivered_item() {
    let mut env = Env::new();
    ok(env.wrap_nft(1, tx_hash(1), MintTo::Holder(USER)));
    ok(env.redeem(USER, Destination::Eth([0x22; 20])));
    ok(env.resolve(0));
    ok(env.record_delivery(0, tx_hash(9)));
    // the same NFT comes back into custody as a new item; the old ticket cannot record it
    ok(env.wrap_nft(1, tx_hash(2), MintTo::Holder(USER)));
    expect_err(env.record_delivery(0, tx_hash(9)), BooksError::WrongHeldMarker);
    env.check_ledger();
}
