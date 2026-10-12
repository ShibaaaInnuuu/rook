//! The treasury in the canister's books: shares and their 24 h wait, allocate, the volume-weighted distribution, the reserve
//! cap, skims. Ported from solana/programs/rook_core/tests/suite/treasury_wake.rs (names kept). The wake desk is not ported
//! (decision O-11: dropped for launch). Owner checks are governance's (§10.4); the "ops account passed / missing" refusals are
//! account plumbing and are gone (the canister pays `treasury.ops` itself).

mod books_common;

use books_common::*;
use etch_autobuyer::books::treasury::*;
use etch_autobuyer::books::*;

const OPS: [u8; 32] = [0x99; 32];

/// One registered collection with seven-day volume (100 SOL of mints at FLOOR).
fn one_collection() -> Env {
    let mut env = Env::new();
    ok(env.post(FLOOR));
    ok(env.mint_units(USER, 100 * SOL, 0));
    env
}

#[track_caller]
fn treasury_covers_pot(env: &Env) {
    ok(treasury_balance(&env.b, env.tre));
}

// rook_core treasury_wake.rs: shares_allocate_and_ops (Treasury.t.sol: test_theSharesSumToOneHundredPercent_andOnlyTheTimelockChangesThem,
// test_allocateSplitsIncomeByTheShares_andPaysOperations, test_theBooksBalanceAtEveryStep)
#[test]
fn shares_allocate_and_ops() {
    let mut env = Env::new();
    expect_err(env.b.treasury.set_shares(8_000, 1_000, env.now), BooksError::BadShares);
    let t = &env.b.treasury;
    assert_eq!((t.reserves_bps, t.ops_bps), (9_000, 1_000));
    expect_err(env.allocate(), BooksError::NothingToDo);
    expect_err(env.receive(0), BooksError::BadInput);
    ok(env.receive(10 * SOL));
    ok(env.allocate());
    assert_eq!(env.b.treasury.reserves_pot, 10 * SOL, "no ops address: its share joins the reserves");
    ok(env.b.treasury.set_ops(OPS, env.now));
    ok(env.receive(10 * SOL));
    ok(env.allocate());
    assert_eq!(env.lamports(&OPS), SOL as i128);
    assert_eq!(env.b.treasury.reserves_pot, 19 * SOL);
    treasury_covers_pot(&env);
}

// rook_core treasury_wake.rs: treasury_changes_wait_a_day (operator decision 2026-10-10)
#[test]
fn treasury_changes_wait_a_day() {
    let mut env = Env::new();
    let t0 = env.now;
    expect_err(env.b.treasury.apply_shares(env.now), BooksError::NothingPending);
    ok(env.b.treasury.set_shares(8_000, 2_000, env.now));
    assert_eq!((env.b.treasury.reserves_bps, env.b.treasury.ops_bps), (9_000, 1_000), "a proposal changes nothing yet");
    expect_err(env.b.treasury.apply_shares(t0 + TREASURY_DELAY - 1), BooksError::TooEarly);
    ok(env.b.treasury.apply_shares(t0 + TREASURY_DELAY));
    let t = &env.b.treasury;
    assert_eq!((t.reserves_bps, t.ops_bps, t.pending_shares_at), (8_000, 2_000, 0));
    expect_err(env.b.treasury.apply_shares(t0 + TREASURY_DELAY), BooksError::NothingPending);
    let first = [1u8; 32];
    ok(env.b.treasury.set_ops(first, env.now));
    assert_eq!(env.b.treasury.ops, Some(first));
    let second = [2u8; 32];
    let t1 = env.now;
    ok(env.b.treasury.set_ops(second, t1));
    assert_eq!(env.b.treasury.ops, Some(first), "a change waits");
    expect_err(env.b.treasury.apply_ops(t1), BooksError::TooEarly);
    ok(env.b.treasury.apply_ops(t1 + TREASURY_DELAY));
    assert_eq!((env.b.treasury.ops, env.b.treasury.pending_ops_at), (Some(second), 0));
}

// rook_core treasury_wake.rs: distribute_is_weighted_by_seven_day_volume_one_collection_per_call (Treasury.t.sol:
// test_distributeIsWeightedBySevenDayVolume, test_nothingLeavesButToRegisteredModulesAndOperations)
#[test]
fn distribute_is_weighted_by_seven_day_volume_one_collection_per_call() {
    let mut env = Env::new();
    env.add_collection(1, test_params());
    expect_err(env.b.treasury.add_collection(0), BooksError::AlreadyCollection);
    ok(env.post(FLOOR));
    ok(env.mint_units(USER, 300 * SOL, 0));
    env.use_listing(0);
    ok(env.post(FLOOR));
    ok(env.mint_units(USER, 100 * SOL, 0));
    ok(env.receive(40 * SOL));
    ok(env.allocate());
    ok(env.begin());
    expect_err(env.distribute(2), BooksError::NotCollection);
    ok(env.distribute(0));
    expect_err(env.distribute(0), BooksError::NothingToDo);
    ok(env.distribute(1));
    assert_eq!(env.module_of(0).surplus, 40 * SOL / 4, "no ops address: the whole 40 SOL is the reserves pot");
    assert_eq!(env.module_of(1).surplus, 40 * SOL * 3 / 4);
    assert_eq!(env.b.treasury.reserves_pot, 0);
    env.check_books();
}

// rook_core treasury_wake.rs: the_reserve_cap_starts_at_two_floors
#[test]
fn the_reserve_cap_starts_at_two_floors() {
    let env = Env::new();
    let t = &env.b.treasury;
    assert_eq!((t.reserve_cap_bps, t.pending_reserve_cap_bps, t.pending_reserve_cap_at), (DEFAULT_RESERVE_CAP_BPS, 0, 0));
    assert_eq!(DEFAULT_RESERVE_CAP_BPS, 20_000);
}

// rook_core treasury_wake.rs: distribute_under_the_cap_pays_the_full_share
#[test]
fn distribute_under_the_cap_pays_the_full_share() {
    let mut env = one_collection();
    ok(env.b.treasury.set_ops(OPS, env.now));
    env.fund_round(40 * SOL); // 36 SOL to the pot, 4 SOL to ops
    let s0 = env.module().surplus;
    assert!(s0 + 36 * SOL < env.cap_of(), "well under the cap");
    let ops0 = env.lamports(&OPS);
    let t0 = env.b.treasury.clone();
    ok(env.distribute(0));
    let t = &env.b.treasury;
    assert_eq!(env.module().surplus - s0, 36 * SOL);
    assert_eq!(env.lamports(&OPS), ops0, "nothing to ops below the cap");
    assert_eq!((t.reserves_pot, t.total_to_reserves - t0.total_to_reserves, t.total_to_ops - t0.total_to_ops), (0, 36 * SOL, 0));
    env.fund_round(40 * SOL);
    ok(env.distribute(0));
    treasury_covers_pot(&env);
    env.check_books();
}

// rook_core treasury_wake.rs: distribute_above_the_cap_sends_the_excess_to_ops
#[test]
fn distribute_above_the_cap_sends_the_excess_to_ops() {
    let mut env = one_collection();
    ok(env.b.treasury.set_ops(OPS, env.now));
    let cap = env.cap_of();
    assert_eq!(cap, 2 * env.module().accepted, "two floors");
    let income = (cap + 100 * SOL) * 10 / 9 + 10;
    env.fund_round(income);
    let pot = env.b.treasury.reserves_pot;
    let s0 = env.module().surplus;
    let room = cap - s0;
    assert!(pot > room);
    let ops0 = env.lamports(&OPS);
    let t0 = env.b.treasury.clone();
    ok(env.distribute(0));
    let t = env.b.treasury.clone();
    assert_eq!(env.module().surplus, cap, "topped up to the cap");
    assert_eq!(env.lamports(&OPS) - ops0, (pot - room) as i128);
    assert_eq!(t.total_to_reserves - t0.total_to_reserves, room);
    assert_eq!(t.total_to_ops - t0.total_to_ops, pot - room);
    assert_eq!(t.reserves_pot, 0);
    treasury_covers_pot(&env);
    env.check_books();
    env.fund_round(50 * SOL);
    let pot = env.b.treasury.reserves_pot;
    let ops0 = env.lamports(&OPS);
    ok(env.distribute(0));
    assert_eq!(env.module().surplus, cap);
    assert_eq!(env.lamports(&OPS) - ops0, pot as i128);
    assert_eq!(env.b.treasury.reserves_pot, 0);
    // over the cap (a lower cap applied later): nothing more reaches the surplus, nothing is taken back
    ok(env.b.treasury.set_reserve_cap(MIN_RESERVE_CAP_BPS, env.now));
    let at = env.now + TREASURY_DELAY;
    env.set_time(at);
    ok(env.b.treasury.apply_reserve_cap(env.now));
    assert!(env.module().surplus > env.cap_of());
    env.fund_round(10 * SOL);
    let pot = env.b.treasury.reserves_pot;
    let ops0 = env.lamports(&OPS);
    ok(env.distribute(0));
    assert_eq!((env.module().surplus, env.lamports(&OPS) - ops0), (cap, pot as i128));
    treasury_covers_pot(&env);
    env.check_books();
}

// rook_core treasury_wake.rs: distribute_without_a_floor_pays_the_full_share
#[test]
fn distribute_without_a_floor_pays_the_full_share() {
    let mut env = one_collection();
    ok(env.b.treasury.set_ops(OPS, env.now));
    env.b.collections.get_mut(&ID).unwrap().module.accepted = 0; // a test-only shortcut: the books do not depend on it
    env.fund_round(1_000 * SOL);
    let pot = env.b.treasury.reserves_pot;
    assert!(pot > 2 * FLOOR);
    let s0 = env.module().surplus;
    ok(env.distribute(0));
    assert_eq!(env.module().surplus - s0, pot);
    assert_eq!(env.b.treasury.reserves_pot, 0);
    treasury_covers_pot(&env);
    env.check_books();
}

// rook_core treasury_wake.rs: distribute_without_ops_keeps_the_excess_and_the_round_closes
#[test]
fn distribute_without_ops_keeps_the_excess_and_the_round_closes() {
    let mut env = one_collection();
    let cap = env.cap_of();
    env.fund_round(cap + 100 * SOL);
    let pot = env.b.treasury.reserves_pot;
    let room = cap - env.module().surplus;
    let t0 = env.b.treasury.clone();
    ok(env.distribute(0));
    let t = env.b.treasury.clone();
    assert_eq!(env.module().surplus, cap);
    assert_eq!(t.reserves_pot, pot - room, "the excess stays in the pot");
    assert_eq!((t.total_to_reserves - t0.total_to_reserves, t.total_to_ops - t0.total_to_ops), (room, 0));
    assert!(!t.round_open().unwrap(), "index 0 is paid for this round");
    expect_err(env.distribute(0), BooksError::NothingToDo);
    treasury_covers_pot(&env);
    ok(env.begin());
    let kept = env.b.treasury.reserves_pot;
    ok(env.distribute(0));
    let t = env.b.treasury.clone();
    assert_eq!((t.reserves_pot, env.module().surplus), (kept, cap));
    assert!(!t.round_open().unwrap());
    ok(env.begin());
    treasury_covers_pot(&env);
    env.check_books();
}

// rook_core treasury_wake.rs: the_reserve_cap_changes_after_a_day_within_bounds
#[test]
fn the_reserve_cap_changes_after_a_day_within_bounds() {
    let mut env = Env::new();
    let t = &mut env.b.treasury;
    expect_err(t.apply_reserve_cap(T0), BooksError::NothingPending);
    for bad in [0, MIN_RESERVE_CAP_BPS - 1, MAX_RESERVE_CAP_BPS + 1, u32::MAX] {
        expect_err(t.set_reserve_cap(bad, T0), BooksError::BadReserveCap);
    }
    ok(t.set_reserve_cap(MAX_RESERVE_CAP_BPS, T0));
    ok(t.set_reserve_cap(MIN_RESERVE_CAP_BPS, T0));
    ok(t.set_reserve_cap(30_000, T0));
    assert_eq!(t.reserve_cap_bps, DEFAULT_RESERVE_CAP_BPS, "a proposal changes nothing yet");
    expect_err(t.apply_reserve_cap(T0 + TREASURY_DELAY - 1), BooksError::TooEarly);
    ok(t.apply_reserve_cap(T0 + TREASURY_DELAY));
    assert_eq!((t.reserve_cap_bps, t.pending_reserve_cap_bps, t.pending_reserve_cap_at), (30_000, 0, 0));
    expect_err(t.apply_reserve_cap(T0 + TREASURY_DELAY), BooksError::NothingPending);
}

// rook_core treasury_wake.rs: skims_reach_the_treasury (Treasury.t.sol: test_theGraduationFeeAndTheSkimsReachTheTreasury)
#[test]
fn skims_reach_the_treasury() {
    let mut p = test_params();
    p.revenue_target = 0;
    let mut env = Env::with_params(p);
    ok(env.post(FLOOR));
    env.user_with_units(400 * SOL);
    ok(env.burn_units(USER, USER, ONE, 0));
    let rev = env.module().revenue;
    assert!(rev > 0);
    let t0 = env.tre;
    ok(env.skim());
    assert_eq!(env.tre - t0, rev);
    ok(env.allocate());
    env.check_books();
}

// New (§9.1, pagination goes): one call weighs every collection and pays each its share; a second call in the same day is refused
// only while something is still owed (A2-01).
#[test]
fn distribute_all_pays_every_collection_in_one_call() {
    let mut env = Env::new();
    env.add_collection(1, test_params());
    env.add_collection(2, test_params());
    for (id, pay) in [(0u16, 100 * SOL), (1, 300 * SOL)] {
        env.use_listing(id);
        ok(env.post(FLOOR));
        ok(env.mint_units(USER, pay, 0));
    }
    ok(env.receive(40 * SOL));
    ok(env.allocate());
    let ds = ok(env.b.distribute_all(env.now));
    for d in &ds {
        env.apply_distribution(d);
    }
    assert_eq!(ds.len(), 2, "collection 2 has no volume");
    assert_eq!((env.module_of(0).surplus, env.module_of(1).surplus, env.module_of(2).surplus), (10 * SOL, 30 * SOL, 0));
    assert!(!env.b.treasury.round_open().unwrap());
    assert!(ok(env.b.distribute_all(env.now)).is_empty(), "an empty pot pays nothing");
    env.check_books();
}
