//! The floor module in the canister's books: post, mint, burn, advance / settle / return, item_arrived, snapshot,
//! fund_reserve, skim. Ported from solana/programs/rook_core/tests/suite/floor.rs (each test keeps its name; the comment above
//! it names the rook_core test and the Solidity test that test ported).
//!
//! What changed in the port: the account plumbing and the signer checks are gone (the canister is the only caller); rook_core's
//! `convert` (dropped, §4.1) seeded SOL-backed units at the grad price, so the tests that used it seed with public mints
//! ([`Env::seed_units`]) and, where the old test relied on the grad price's 3 % margin over the floor, add a surplus with
//! `fund_reserve`; an item bought with an advance is wrapped to the module in the books ([`MintTo::Module`]).

mod books_common;

use books_common::*;
use etch_autobuyer::books::floor::{FLOOR_TTL, STALE_ESCAPE};
use etch_autobuyer::books::*;

fn unit_price(base: u64, spread: u64) -> u64 {
    ((base as u128 * (BPS + spread) as u128).div_ceil(BPS as u128)) as u64
}

// rook_core floor.rs: the_first_post_is_the_anchor (FloorModule.t.sol: test_theFirstPostIsTheAnchor)
#[test]
fn the_first_post_is_the_anchor() {
    let mut env = Env::new();
    ok(env.post(FLOOR));
    let m = env.module();
    assert_eq!(m.accepted, FLOOR);
    assert_eq!(m.target, FLOOR);
    assert_eq!(m.day_anchor, FLOOR);
    assert_eq!(m.posted_at, env.now);
    assert!(m.fresh(env.now));
}

// rook_core floor.rs: only_the_poster_posts_and_never_zero (FloorModule.t.sol: test_onlyTheKeeperPosts_andNeverZero). The
// canister is the only poster: what remains is the switch and the input checks.
#[test]
fn only_the_poster_posts_and_never_zero() {
    let mut env = Env::new();
    env.pause_poster();
    expect_err(env.post(FLOOR), BooksError::NotKeeper);
    env.unpause_poster();
    env.b.set_keeper_paused(true);
    expect_err(env.post(FLOOR), BooksError::NotKeeper);
    env.b.set_keeper_paused(false);
    expect_err(env.post(0), BooksError::BadInput);
    expect_err(env.post_with(FLOOR, 10_001, 20), BooksError::BadInput);
    expect_err(env.b.post_floor(9, FLOOR, 9_000, 20, env.now), BooksError::UnknownCollection);
    assert_eq!(env.module().accepted, 0, "a refused post changes nothing");
}

// rook_core floor.rs: the_floor_moves_at_most_five_percent_an_hour (FloorModule.t.sol: test_theFloorMovesAtMostFivePercentAnHour,
// test_aHalfHourMovesHalfAsFar, test_aSilentKeeperStillMovesOnlyOneHoursWorth)
#[test]
fn the_floor_moves_at_most_five_percent_an_hour() {
    let mut env = Env::new();
    ok(env.post(FLOOR));
    env.advance_time(3_600);
    ok(env.post(2 * FLOOR));
    assert_eq!(env.module().accepted, FLOOR * 105 / 100);
    assert_eq!(env.module().target, 2 * FLOOR);
    let a = env.module().accepted;
    env.advance_time(1_800);
    ok(env.post(a / 2));
    assert_eq!(env.module().accepted, a - a * 500 * 1_800 / (BPS * 3_600));
    let a = env.module().accepted;
    env.advance_time(5 * 3_600);
    ok(env.post(3 * a));
    assert_eq!(env.module().accepted, a + a * 500 / BPS);
}

// rook_core floor.rs: the_day_band_holds_twenty_percent (FloorModule.t.sol: the ±20 % day band)
#[test]
fn the_day_band_holds_twenty_percent() {
    let mut env = Env::new();
    ok(env.post(FLOOR));
    for _ in 0..10 {
        env.advance_time(3_600);
        ok(env.post(10 * FLOOR));
        let m = env.module();
        let (lo, hi) = m.floor_bounds(env.now).unwrap();
        assert!(m.accepted <= FLOOR * 12 / 10, "day band");
        assert!(lo <= m.accepted && m.accepted <= hi.max(m.accepted));
    }
    assert_eq!(env.module().accepted, FLOOR * 12 / 10);
}

// rook_core floor.rs: a_post_goes_stale_after_three_hours (FloorModule.t.sol: test_aPostGoesStaleAfterThreeHoursOnEitherClock)
#[test]
fn a_post_goes_stale_after_three_hours() {
    let mut env = Env::new();
    ok(env.post(FLOOR));
    env.user_with_units(10 * SOL);
    env.advance_time(FLOOR_TTL);
    expect_err(env.mint_units(USER, SOL, 0), BooksError::Stale);
    expect_err(env.burn_units(USER, USER, 1_000, 0), BooksError::Stale);
}

// rook_core floor.rs: a_low_confidence_post_leaves_the_module_stale
#[test]
fn a_low_confidence_post_leaves_the_module_stale() {
    let mut env = Env::new();
    ok(env.post_with(FLOOR, 2_999, 20));
    expect_err(env.mint_units(USER, SOL, 0), BooksError::Stale);
}

// rook_core floor.rs: a_mint_is_instant_eth_backed_and_splits_into_the_two_pots (FloorModule.t.sol: test_mintIsInstantAndETHBacked;
// FloorReserve.t.sol: test_aMintSplitsIntoTheTwoPots)
#[test]
fn a_mint_is_instant_eth_backed_and_splits_into_the_two_pots() {
    let mut env = Env::new();
    ok(env.post(FLOOR));
    let pay = 100 * SOL;
    let units = ok(env.mint_units(USER, pay, 0));
    assert_eq!(units, (pay as u128 * ONE as u128 / unit_price(FLOOR, 15) as u128) as u64);
    assert_eq!(env.balance(&USER), units);
    assert_eq!(env.e(), units);
    let m = env.module();
    assert_eq!(m.reserve, pay / 4);
    assert_eq!(m.reserve_units, units / 4);
    assert_eq!(m.backing, pay - pay / 4);
    assert_eq!(env.vault_free(), pay);
    env.check_books();
    env.check_ledger();
}

// rook_core floor.rs: burns_pay_at_most_the_backing_reserve_first (FloorModule.t.sol: test_burnPaysTheFloorLessTheSpread_andKeepsTheDifference,
// test_theBurnPriceNeverExceedsTheBackingPerUnit; FloorReserve.t.sol: test_burnsDrawTheReserveFirst_andSpanTheTiers)
#[test]
fn burns_pay_at_most_the_backing_reserve_first() {
    let mut env = Env::new();
    ok(env.post(FLOOR));
    env.user_with_units(400 * SOL);
    let m0 = env.module();
    let held = env.balance(&USER);
    let r_basis = m0.reserve as u128 * ONE as u128 / m0.reserve_units as u128;
    let units = m0.reserve_units + ONE / 2; // spans both tiers
    let paid = ok(env.burn_units(USER, USER, units, 1));
    let m1 = env.module();
    assert_eq!(m1.reserve_units, 0);
    assert_eq!(m1.reserve, 0);
    assert!(m1.surplus > 0, "the reserve tier's spread goes to the surplus");
    assert!(m1.revenue > 0, "the convertible tier's spread is revenue");
    assert!((paid as u128) <= r_basis * m0.reserve_units as u128 / ONE as u128 + m0.backing as u128);
    assert_eq!(env.balance(&USER), held - units);
    env.check_books();
    env.check_ledger();
    let m = env.module();
    let (_, pc) = m.tier_prices(env.now, env.e(), false).unwrap();
    assert!(pc <= FLOOR * (BPS - 15) / BPS);
}

// rook_core floor.rs: mints_and_burns_are_capped_per_hour (FloorModule.t.sol: test_mintsAreCappedPerHour_onTheSlowerClock,
// test_burnsAreCappedPerHour)
#[test]
fn mints_and_burns_are_capped_per_hour() {
    let mut env = Env::new();
    ok(env.post(FLOOR));
    ok(env.mint_units(USER, 900 * SOL, 0));
    expect_err(env.mint_units(USER, 200 * SOL, 0), BooksError::OverCap);
    env.advance_time(3_600);
    ok(env.post(FLOOR));
    ok(env.mint_units(USER, 200 * SOL, 0));
    assert!(env.balance(&USER) > 5 * ONE);
    expect_err(env.burn_units(USER, USER, 5 * ONE + 1, 0), BooksError::OverCap);
    ok(env.burn_units(USER, USER, 5 * ONE, 0));
    expect_err(env.burn_units(USER, USER, ONE / 10, 0), BooksError::OverCap);
}

// rook_core floor.rs: the_books_depth_bounds_public_minting (FloorModule.t.sol: test_theBooksDepthBoundsPublicMinting,
// test_publicMintsStopAtTheETHBackedCap). With rook_core's parameters (`mint_depth_cap`).
#[test]
fn the_books_depth_bounds_public_minting() {
    let mut env = Env::new();
    ok(env.post_with(FLOOR, 9_000, 1)); // public cap: one convertible unit
    expect_err(env.mint_units(USER, 300 * SOL, 0), BooksError::OverBacked);
    ok(env.mint_units(USER, 200 * SOL, 0)); // 0.99 unit, 0.75 of it convertible
    expect_err(env.mint_units(USER, 80 * SOL, 0), BooksError::OverBacked);
}

// New (§4.3, decision O-4): with value-based parameters the poster's depth no longer bounds minting; the public cap does.
#[test]
fn value_params_free_minting_from_the_depth_but_keep_the_public_cap() {
    let p = value_params(FLOOR, 5, 1, REVENUE_TARGET).unwrap(); // public 10, hourly 5 (⌈250/200⌉ = 2 < 5)
    let mut env = Env::with_params(p);
    ok(env.post_with(FLOOR, 9_000, 1));
    ok(env.mint_units(USER, 300 * SOL, 0)); // over the one-unit depth: taken
    assert_eq!(env.module().public_cap(), 10 * ONE);
    assert_eq!(env.module().depth_cap(), ONE, "the depth stays readable for the buy pipeline");
    env.check_books();
    env.check_ledger();
}

// rook_core floor.rs: slippage_and_bad_inputs (FloorModule.t.sol: test_slippageAndBadInputs)
#[test]
fn slippage_and_bad_inputs() {
    let mut env = Env::new();
    ok(env.post(FLOOR));
    expect_err(env.mint_units(USER, 0, 0), BooksError::BadInput);
    expect_err(env.mint_units(USER, 10 * SOL, ONE), BooksError::Slippage);
    expect_err(env.mint_units(USER, 1, 0), BooksError::Slippage);
    ok(env.mint_units(USER, 10 * SOL, 0));
    expect_err(env.burn_units(USER, USER, 0, 0), BooksError::BadInput);
    expect_err(env.burn_units(USER, USER, 1_000, 10 * SOL), BooksError::Slippage);
}

// rook_core floor.rs: the_spreads (FloorModule.t.sol: test_movementWidensBothSidesAndDecaysOverAnHour, test_stalenessWidensFromTenMinutesToThreeHours,
// test_stressWidensMintingOnly, test_theSpreadNeverExceedsTheMaximum, test_lowConfidenceWidensBothSides)
#[test]
fn the_spreads() {
    let mut env = Env::new();
    ok(env.post(FLOOR));
    let e = 0;
    let m = env.module();
    assert_eq!(m.spreads(env.now, e).unwrap(), (15, 15));
    env.advance_time(3_600);
    ok(env.post(2 * FLOOR));
    let m = env.module();
    assert_eq!(m.spreads(env.now, e).unwrap(), (515, 515));
    let (s, _) = m.spreads(env.now + 1_800, e).unwrap();
    assert!(s > 15 + 100 && s < 515, "decays (and staleness starts at 10 min)");
    let (s, b) = m.spreads(env.now + FLOOR_TTL, e).unwrap();
    assert_eq!((s, b), (115, 115));
    env.advance_time(3_600);
    let a = env.module().accepted;
    ok(env.post_with(a, 4_000, 20));
    let m = env.module();
    assert_eq!(m.spreads(env.now, e).unwrap().1, 15 + 50);
    let (sm, sb) = m.spreads(env.now, 10 * ONE).unwrap();
    assert_eq!(sm - sb, 300);
    let mut p = test_params();
    p.max_spread_bps = 100;
    p.stale_spread_bps = 100;
    p.stress_spread_bps = 100;
    p.low_confidence_spread_bps = 100;
    let mut env = Env::with_params(p);
    ok(env.post(FLOOR));
    env.advance_time(3_600);
    ok(env.post(2 * FLOOR));
    assert_eq!(env.module().spreads(env.now, 10 * ONE).unwrap(), (100, 100));
}

// rook_core floor.rs: the_lag_is_priced_against_the_trader (FloorModule.t.sol: test_theLagIsAlwaysPricedAgainstTheTrader)
#[test]
fn the_lag_is_priced_against_the_trader() {
    let mut env = Env::new();
    ok(env.post(FLOOR));
    env.advance_time(3_600);
    ok(env.post(FLOOR * 3 / 2));
    let m = env.module();
    assert_eq!(m.mint_base(env.now).unwrap(), FLOOR * 12 / 10);
    env.advance_time(3_600);
    ok(env.post(FLOOR / 2));
    let m = env.module();
    assert!(m.burn_target(env.now).unwrap() >= m.accepted * (BPS - 500) / BPS);
}

// rook_core floor.rs: the_pauses_stop_every_inflow_priced_by_the_poster_but_not_burns (EthNftFlow.t.sol: test_thePosterIsTheCanister_andTheRegistrySwitchZeroesIt,
// test_D_M_1_thePosterHasItsOwnSwitch…, test_thePauseStopsEveryInflowPricedByThePoster_butNotBurns). Who may flip the switches
// is governance (§10.4), not the books.
#[test]
fn the_pauses_stop_every_inflow_priced_by_the_poster_but_not_burns() {
    let mut env = Env::new();
    ok(env.post(FLOOR));
    env.user_with_units(400 * SOL);
    env.pause_poster();
    expect_err(env.post(FLOOR), BooksError::NotKeeper);
    expect_err(env.mint_units(USER, SOL, 0), BooksError::Paused);
    expect_err(env.advance(SOL), BooksError::OverItemCap);
    assert!(ok(env.burn_units(USER, USER, ONE / 10, 1)) > 0);
    env.advance_time(FLOOR_TTL + 60);
    ok(env.burn_units(USER, USER, ONE / 10, 1));
    env.unpause_poster();
    ok(env.post(FLOOR));
    ok(env.mint_units(USER, SOL, 0));
    env.b.set_keeper_paused(true);
    expect_err(env.post(FLOOR), BooksError::NotKeeper);
    expect_err(env.mint_units(USER, SOL, 0), BooksError::Paused);
    ok(env.burn_units(USER, USER, ONE / 10, 1));
    env.b.set_keeper_paused(false);
    ok(env.post(FLOOR));
    env.check_books();
    env.check_ledger();
}

// New (§10.4 `pause_collection`): a paused collection closes the public mint and item mints; burns and redemptions go on.
#[test]
fn a_paused_collection_closes_minting_but_not_burns_or_redemptions() {
    let mut env = Env::new();
    ok(env.post(FLOOR));
    env.user_with_units(400 * SOL);
    ok(env.wrap_nft(1, tx_hash(1), MintTo::Holder(HOLDER)));
    ok(env.b.set_paused(ID, true));
    expect_err(env.mint_units(USER, SOL, 0), BooksError::ListingPaused);
    expect_err(env.wrap_nft(2, tx_hash(2), MintTo::Holder(HOLDER)), BooksError::ListingPaused);
    ok(env.burn_units(USER, USER, ONE / 10, 1));
    ok(env.redeem(HOLDER, Destination::Eth([0x42; 20])));
    env.check_books();
    env.check_ledger();
}

// rook_core floor.rs: the_stale_escape_pays_the_backing_and_skims_the_surplus (FloorModule B-04, `escaped()`)
#[test]
fn the_stale_escape_pays_the_backing_and_skims_the_surplus() {
    let mut env = Env::new();
    ok(env.post(FLOOR));
    env.user_with_units(400 * SOL);
    ok(env.fund_reserve(5 * SOL));
    env.advance_time(FLOOR_TTL + 10);
    expect_err(env.burn_units(USER, USER, ONE / 10, 0), BooksError::Stale);
    env.advance_time(STALE_ESCAPE);
    let m = env.module();
    assert!(m.escaped(env.now));
    let b = m.beta(env.e()).unwrap();
    let got = ok(env.burn_units(USER, USER, ONE / 10, 0));
    let pr = b as u128 * (BPS - 15) as u128 / BPS as u128;
    assert_eq!(got as u128, (ONE / 10) as u128 * pr / ONE as u128);
    let t0 = env.tre;
    ok(env.skim());
    assert!(env.tre - t0 >= 5 * SOL);
    assert_eq!(env.b.treasury.unallocated, env.tre, "the skim is booked as treasury income");
    let m = env.module();
    assert_eq!((m.surplus, m.revenue), (0, 0));
    env.check_books();
}

// rook_core floor.rs: fund_reserve_joins_the_surplus_and_skim_takes_revenue_beyond_the_target (FloorReserve.t.sol:
// test_fundReserveJoinsTheSurplus_notTheUnitsBacking)
#[test]
fn fund_reserve_joins_the_surplus_and_skim_takes_revenue_beyond_the_target() {
    let mut env = Env::new();
    ok(env.post(FLOOR));
    expect_err(env.fund_reserve(0), BooksError::BadInput);
    ok(env.fund_reserve(3 * SOL));
    let m = env.module();
    assert_eq!((m.surplus, m.backing, m.reserve), (3 * SOL, 0, 0));
    expect_err(env.skim(), BooksError::NothingToSettle);
    env.check_books();
}

// rook_core floor.rs: advance_pays_the_custodian_and_puts_one_unit_in_transit (Advance.t.sol: test_advancePaysThePayeeNowAndPutsOneUnitInTransit,
// test_advanceRefusals, test_noAdvanceWhenStale_orWhileTheKeeperIsPaused_orWithoutAWholeUnit)
#[test]
fn advance_pays_the_custodian_and_puts_one_unit_in_transit() {
    let mut env = Env::new();
    ok(env.post(FLOOR));
    expect_err(env.advance(FLOOR), BooksError::OverItemCap); // no whole convertible unit
    env.seed_units(HOLDER, 1_000 * SOL); // ≈ 4.9 units, 3.7 convertible
    ok(env.fund_reserve(10 * SOL)); // rook_core seeded at the grad price (floor + 3 %): the surplus stands in for that margin
    let e = env.e();
    let max = env.module().max_item_cost(env.now, e, false).unwrap();
    assert_eq!(max, FLOOR * (BPS + 300) / BPS);
    expect_err(env.advance(0), BooksError::BadInput);
    expect_err(env.advance(max + 1), BooksError::OverItemCap);
    let c0 = env.lamports(&CUSTODIAN);
    ok(env.advance(max));
    assert_eq!(env.lamports(&CUSTODIAN) - c0, max as i128);
    let m = env.module();
    assert_eq!((m.transit_units, m.advanced), (ONE, max));
    expect_err(env.advance(1), BooksError::AdvanceOpen);
    env.check_books();
    ok(env.return_advance(max));
    env.advance_time(FLOOR_TTL);
    expect_err(env.advance(SOL), BooksError::OverItemCap);
}

// rook_core floor.rs: settle_turns_the_transit_unit_into_the_item (Advance.t.sol: test_settleTurnsTheTransitUnitIntoTheItem_noEthMoves,
// test_settleRefusals, test_aSecondAdvanceAfterSettlement; EthNftFlow.t.sol: test_settleNeedsTheUnitAtTheModule)
#[test]
fn settle_turns_the_transit_unit_into_the_item() {
    let mut env = Env::new();
    ok(env.post(FLOOR));
    env.seed_units(HOLDER, 1_000 * SOL);
    expect_err(env.settle(SOL), BooksError::NoAdvance);
    let max = env.module().max_item_cost(env.now, env.e(), false).unwrap();
    ok(env.advance(max));
    expect_err(env.settle(SOL), BooksError::NoWrapped);
    ok(env.wrap_nft(42, tx_hash(1), MintTo::Module));
    assert_eq!(env.ledger().module_units, ONE);
    expect_err(env.settle(0), BooksError::BadInput);
    expect_err(env.settle(max + 1), BooksError::BadInput);
    let e0 = env.e();
    let v0 = env.vault_free();
    let s0 = env.supply();
    ok(env.settle(max - SOL));
    assert_eq!(env.e(), e0 - ONE);
    assert_eq!(env.ledger().module_units, 0);
    assert_eq!(env.vault_free(), v0, "no lamports move");
    assert_eq!(env.supply(), s0, "and no wX moves (§8)");
    let m = env.module();
    assert_eq!((m.transit_units, m.items_arrived, m.fill_count), (0, 1, 1));
    env.check_books();
    env.check_ledger();
    let max = env.module().max_item_cost(env.now, env.e(), false).unwrap();
    ok(env.advance(max.min(FLOOR)));
}

// rook_core floor.rs: return_advance_restores_the_basis (Advance.t.sol: test_returnAtOrAboveTheBasisRestoresTheUnit_theRestIsRevenue,
// test_aShortReturnIsCoveredByRevenueThenSurplus, test_aReturnTheyCannotCoverIsALossSharedByTheConvertibleUnits, test_returnRefusals)
#[test]
fn return_advance_restores_the_basis() {
    let mut env = Env::new();
    ok(env.post(FLOOR));
    env.seed_units(HOLDER, 1_000 * SOL);
    expect_err(env.return_advance(SOL), BooksError::NoAdvance);
    let backing0 = env.module().backing;
    ok(env.advance(FLOOR));
    let basis = env.module().advance_basis;
    ok(env.return_advance(FLOOR + SOL));
    let m = env.module();
    assert_eq!(m.backing, backing0);
    assert_eq!(m.transit_units, 0);
    assert!(basis >= FLOOR);
    assert_eq!(m.revenue, SOL, "what came back above the basis, plus what the basis kept above the advance");
    ok(env.fund_reserve(SOL));
    ok(env.advance(FLOOR));
    let m0 = env.module();
    ok(env.return_advance(0));
    let m1 = env.module();
    let need = m0.advance_basis;
    let from_rev = need.min(m0.revenue);
    let from_sur = (need - from_rev).min(m0.surplus);
    assert_eq!(m1.revenue, m0.revenue - from_rev);
    assert_eq!(m1.surplus, m0.surplus - from_sur);
    assert_eq!(m1.backing, m0.backing + need - (need - from_rev - from_sur));
    env.check_books();
}

// rook_core floor.rs: item_arrivals_are_limited_per_day (Advance.t.sol: test_theDailyCountIsTakenAtTheAdvance_andNotAgainAtSettlement;
// FloorModule.t.sol: test_itemArrivalsAreLimitedPerDay_onBothClocks)
#[test]
fn item_arrivals_are_limited_per_day() {
    let mut env = Env::new(); // max_items_per_day 3
    ok(env.post(FLOOR));
    env.seed_units(HOLDER, 2_000 * SOL);
    ok(env.post(FLOOR));
    for _ in 0..3 {
        ok(env.advance(FLOOR));
        ok(env.return_advance(FLOOR));
    }
    expect_err(env.advance(FLOOR), BooksError::OverItemCap);
    env.advance_time(86_400);
    ok(env.post(FLOOR));
    ok(env.advance(FLOOR));
}

// rook_core floor.rs: item_arrived_turns_one_unit_into_an_item (FloorModule.t.sol: test_itemArrivedTurnsOneUnitIntoAnItem,
// test_itemArrivedRefusals, test_aSnapshotCoversACapDropBetweenPurchaseAndRecord)
#[test]
fn item_arrived_turns_one_unit_into_an_item() {
    let mut env = Env::new();
    ok(env.post(FLOOR));
    env.seed_units(HOLDER, 1_000 * SOL);
    ok(env.fund_reserve(10 * SOL));
    expect_err(env.item_arrived(FLOOR), BooksError::NoWrapped);
    ok(env.wrap_nft(1, tx_hash(1), MintTo::Module));
    expect_err(env.item_arrived(0), BooksError::BadInput);
    let cap = FLOOR * (BPS + 300) / BPS;
    expect_err(env.item_arrived(cap + 1), BooksError::OverItemCap);
    ok(env.snapshot());
    env.advance_time(3_600);
    ok(env.post(FLOOR / 2));
    ok(env.item_arrived(cap));
    assert_eq!(env.module().items_arrived, 1);
    env.check_books();
    env.check_ledger();
    ok(env.wrap_nft(2, tx_hash(2), MintTo::Module));
    env.advance_time(FLOOR_TTL);
    expect_err(env.item_arrived(FLOOR / 2), BooksError::CapStale);
}

// rook_core floor.rs: fills_move_the_price_within_the_band (FloorReserve.t.sol: test_fillsMoveThePriceWithinTheBand_timeWeighted,
// test_beforeAnyFillTheKeeperFloorAlone, test_theBandBoundsWhatFillsCanDo_andTheCapStaysOnTheKeeperFloor)
#[test]
fn fills_move_the_price_within_the_band() {
    let mut env = Env::new();
    ok(env.post(FLOOR));
    assert_eq!(env.module().price(env.now).unwrap(), FLOOR);
    env.seed_units(HOLDER, 1_000 * SOL);
    ok(env.advance(FLOOR));
    ok(env.wrap_nft(1, tx_hash(1), MintTo::Module));
    ok(env.settle(FLOOR / 2));
    env.advance_time(2 * 3_600);
    let m = env.module();
    assert_eq!(m.price(env.now).unwrap(), FLOOR * (BPS - 300) / BPS);
    assert_eq!(m.item_cap(env.now, false).unwrap(), FLOOR * (BPS + 300) / BPS, "the cap stays on the poster's floor");
}

// New: every refused floor operation leaves the books exactly as they were (a Solana transaction reverted; here `atomic`).
#[test]
fn a_refused_operation_changes_nothing() {
    let mut env = Env::new();
    ok(env.post(FLOOR));
    ok(env.mint_units(USER, 900 * SOL, 0));
    let before = env.b.clone();
    expect_err(env.mint_units(USER, 200 * SOL, 0), BooksError::OverCap); // after note_mint_base in rook_core's order
    expect_err(env.burn_units(USER, USER, 4 * ONE, 1_000 * FLOOR), BooksError::Slippage); // after the bucket and release
    expect_err(env.advance(FLOOR * 2), BooksError::OverItemCap);
    expect_err(env.settle(1), BooksError::NoAdvance);
    assert_eq!(env.b, before);
}

// rook_core flow.rs: ethereum_to_wx_to_redeem_to_deliver (EthNftFlow.t.sol: test_theWholeLoop_buyRedeemDeliver,
// test_aGraduationsBudgetBuysAnNft_andTheUnitBecomesItemBacked). The graduated coin's backing is wX bought at the module (no
// convert); the canister sees the buy signal, advances, buys, wraps to the module, settles; a holder redeems; it resolves and
// is delivered.
#[test]
fn ethereum_to_wx_to_redeem_to_deliver() {
    let mut env = Env::new();
    ok(env.post(FLOOR));
    let vault = wallet(1);
    env.seed_units(vault, 412 * SOL);
    let units = env.balance(&vault);
    assert!(units > 19 * ONE / 10);
    env.check_books();
    env.check_ledger();
    let m = env.module();
    let e = env.e();
    assert!(m.net_units(e) >= ONE + ONE * m.params.buffer_bps as u64 / BPS);
    let (net, max) = ok(env.b.buy_signal(ID, env.now));
    assert_eq!((net, max), (m.net_units(e), m.max_item_cost(env.now, e, false).unwrap()));
    ok(env.advance(max));
    env.check_books();
    ok(env.wrap_nft(8_888, tx_hash(8_888), MintTo::Module));
    ok(env.settle(max - SOL / 2));
    let l = env.ledger();
    assert_eq!(l.held, 1);
    assert_eq!(l.sol_backed, units - ONE);
    env.check_books();
    env.check_ledger();
    env.transfer_wx(vault, HOLDER, ONE);
    ok(env.mint_units(USER, 50 * SOL, 0));
    let half = env.balance(&USER) / 2;
    ok(env.burn_units(USER, USER, half, 1));
    env.check_books();
    ok(env.redeem(HOLDER, Destination::Eth([0x42; 20])));
    assert_eq!(env.ledger().open_tickets, 1);
    env.check_ledger();
    let round = env.ticket(0).round;
    env.reach_round(round);
    ok(env.prove(round, [9u8; 32]));
    env.advance_time(60);
    ok(env.resolve(0));
    assert_eq!(env.ticket(0).stage, Stage::Resolved);
    assert_eq!(env.ticket(0).item_id, 0);
    ok(env.note_sent(0, tx_hash(1)));
    ok(env.record_delivery(0, tx_hash(1)));
    let l = env.ledger();
    assert_eq!((l.held, l.open_tickets), (0, 0));
    env.check_ledger();
    env.check_books();
}

// New (§4.4 rules 2 and 3): the guards the flows call before signing. Book first, then sign: a `MintTo` whose amount the books
// already count passes; one more unit, or a mint the books never decided, is refused. A payout the books released passes while
// what stays covers the books; a second, unbooked payout of the same lamports is refused.
#[test]
fn the_pre_signature_guards_catch_an_over_mint_and_an_over_payout() {
    let mut env = Env::new();
    ok(env.post(FLOOR));
    let finalized = env.supply();
    let m = ok(env.b.mint(ID, 100 * SOL, 0, env.now)); // booked; the chain has not minted yet
    let l = env.ledger();
    ok(supply_guard(&l, finalized, 0, m.units));
    expect_err(supply_guard(&l, finalized, 0, m.units + 1), BooksError::SupplyMismatch);
    expect_err(supply_guard(&l, finalized, 1, m.units), BooksError::SupplyMismatch);
    // audit B1-01: a chain UNDER the ledger is holders' own burns (booked by the reconciliation), never a refusal; OVER it is
    ok(supply_matches(&l, finalized));
    expect_err(supply_matches(&l, finalized + m.units + 1), BooksError::SupplyMismatch);
    ok(supply_matches(&l, finalized + m.units));
    let free = 100 * SOL; // the deposit landed at M[i]
    let b = ok(env.b.burn(ID, m.units / 2, 1, env.now)); // booked; the payout not yet signed
    let module = env.module();
    ok(payout_guard(&module, free, 0, b.paid));
    expect_err(payout_guard(&module, free, b.paid, b.paid), BooksError::BooksUnbacked);
    expect_err(payout_guard(&module, free - b.paid, 0, b.paid), BooksError::BooksUnbacked);
    expect_err(books_balance(&module, (module.books() - 1) as u64), BooksError::BooksUnbacked);
}
