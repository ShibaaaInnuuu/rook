//! The floor module: post, mint, burn, advance / settle / return, item_arrived, snapshot, fund_reserve, skim.
//! Each test names the Solidity test it ports (test/floor/*.t.sol, test/ethnft/EthNftFlow.t.sol).

use super::common::*;
use rook_core::error::RookError;
use rook_core::state::{FLOOR_TTL, STALE_ESCAPE};
use rook_core::units::{BPS, ONE};
use solana_keypair::Keypair;
use solana_signer::Signer;

fn unit_price(base: u64, spread: u64) -> u64 {
    ((base as u128 * (BPS + spread) as u128).div_ceil(BPS as u128)) as u64
}

// FloorModule.t.sol: test_theFirstPostIsTheAnchor
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

// FloorModule.t.sol: test_onlyTheKeeperPosts_andNeverZero; EthNftFlow.t.sol: test_theKeeperEoaHasNoFloorRoleHere
#[test]
fn only_the_poster_posts_and_never_zero() {
    let mut env = Env::new();
    let u = env.user.insecure_clone();
    expect_err(env.post_as(&u, FLOOR, 9_000, 20), RookError::NotKeeper);
    let k = env.keeper.insecure_clone();
    expect_err(env.post_as(&k, FLOOR, 9_000, 20), RookError::NotKeeper);
    expect_err(env.post(0), RookError::BadInput);
    let c = env.custodian.insecure_clone();
    expect_err(env.post_as(&c, FLOOR, 10_001, 20), RookError::BadInput);
}

// FloorModule.t.sol: test_theFloorMovesAtMostFivePercentAnHour, test_aHalfHourMovesHalfAsFar,
// test_aSilentKeeperStillMovesOnlyOneHoursWorth
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
    // a silent poster: five hours later still only one hour's worth
    let a = env.module().accepted;
    env.advance_time(5 * 3_600);
    ok(env.post(3 * a));
    assert_eq!(env.module().accepted, a + a * 500 / BPS);
}

// FloorModule.t.sol: test_floorBoundsIsExactlyWhatAPostCanBecome / the ±20 % day band (MAX_DAY_MOVE_BPS)
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

// FloorModule.t.sol: test_aPostGoesStaleAfterThreeHoursOnEitherClock; FloorAttacks: test_stale_nothingTradesAfterThreeHours
#[test]
fn a_post_goes_stale_after_three_hours() {
    let mut env = Env::new();
    ok(env.post(FLOOR));
    let acc = env.user_with_units(10 * SOL);
    env.advance_time(FLOOR_TTL);
    let u = env.user.insecure_clone();
    expect_err(env.mint_units(&u, &acc, SOL, 0), RookError::Stale);
    expect_err(env.burn_units(&u, &acc, &u.pubkey(), 1_000, 0), RookError::Stale);
}

// FloorModule.t.sol: test_aPostUnderTheMinimumConfidenceLeavesTheModuleStale
#[test]
fn a_low_confidence_post_leaves_the_module_stale() {
    let mut env = Env::new();
    let c = env.custodian.insecure_clone();
    ok(env.post_as(&c, FLOOR, 2_999, 20));
    let u = env.user.insecure_clone();
    let acc = env.token_account(&u.pubkey());
    expect_err(env.mint_units(&u, &acc, SOL, 0), RookError::Stale);
}

// FloorModule.t.sol: test_mintIsInstantAndETHBacked; FloorReserve.t.sol: test_aMintSplitsIntoTheTwoPots
#[test]
fn a_mint_is_instant_eth_backed_and_splits_into_the_two_pots() {
    let mut env = Env::new();
    ok(env.post(FLOOR));
    let u = env.user.insecure_clone();
    let acc = env.token_account(&u.pubkey());
    let pay = 100 * SOL;
    let r = ok(env.mint_units(&u, &acc, pay, 0));
    let units = (pay as u128 * ONE as u128 / unit_price(FLOOR, 15) as u128) as u64;
    assert_eq!(env.balance(&acc), units);
    assert_eq!(env.wrapper().eth_backed, units);
    let m = env.module();
    assert_eq!(m.reserve, pay / 4);
    assert_eq!(m.reserve_units, units / 4);
    assert_eq!(m.backing, pay - pay / 4);
    assert_eq!(env.vault_free(), pay);
    env.check_books();
    env.check_ledger();
    assert!(r.compute_units_consumed < 200_000);
}

// FloorModule.t.sol: test_burnPaysTheFloorLessTheSpread_andKeepsTheDifference, test_theBurnPriceNeverExceedsTheBackingPerUnit
// FloorReserve.t.sol: test_burnsDrawTheReserveFirst_andSpanTheTiers, test_theReserveKeepsItsBasisPerUnit_andTheSpreadGoesToTheSurplus
#[test]
fn burns_pay_at_most_the_backing_reserve_first() {
    let mut env = Env::new();
    ok(env.post(FLOOR));
    let acc = env.user_with_units(400 * SOL);
    let u = env.user.insecure_clone();
    let m0 = env.module();
    let held = env.balance(&acc);
    let r_basis = m0.reserve as u128 * ONE as u128 / m0.reserve_units as u128;
    let before = env.lamports(&u.pubkey());
    let units = m0.reserve_units + ONE / 2; // spans both tiers
    ok(env.burn_units(&u, &acc, &u.pubkey(), units, 1));
    let paid = env.lamports(&u.pubkey()) - before + 5_000; // the fee
    let m1 = env.module();
    assert_eq!(m1.reserve_units, 0);
    assert_eq!(m1.reserve, 0);
    assert!(m1.surplus > 0, "the reserve tier's spread goes to the surplus");
    assert!(m1.revenue > 0, "the convertible tier's spread is revenue");
    assert!((paid as u128) <= r_basis * m0.reserve_units as u128 / ONE as u128 + m0.backing as u128);
    assert_eq!(env.balance(&acc), held - units);
    env.check_books();
    env.check_ledger();
    // the floor less the burn spread is what a calm burn pays per unit when the backing allows
    let m = env.module();
    let (_, pc) = m.tier_prices(env.now, env.wrapper().eth_backed, false).unwrap();
    assert!(pc <= FLOOR * (BPS - 15) / BPS);
}

// FloorModule.t.sol: test_mintsAreCappedPerHour_onTheSlowerClock, test_burnsAreCappedPerHour
#[test]
fn mints_and_burns_are_capped_per_hour() {
    let mut env = Env::new();
    ok(env.post(FLOOR));
    let u = env.user.insecure_clone();
    let acc = env.token_account(&u.pubkey());
    // 5 units an hour: about 1000 SOL at a 200 SOL floor
    ok(env.mint_units(&u, &acc, 900 * SOL, 0));
    expect_err(env.mint_units(&u, &acc, 200 * SOL, 0), RookError::OverCap);
    env.advance_time(3_600);
    ok(env.post(FLOOR));
    ok(env.mint_units(&u, &acc, 200 * SOL, 0));
    let bal = env.balance(&acc);
    assert!(bal > 5 * ONE);
    expect_err(env.burn_units(&u, &acc, &u.pubkey(), 5 * ONE + 1, 0), RookError::OverCap);
    ok(env.burn_units(&u, &acc, &u.pubkey(), 5 * ONE, 0));
    expect_err(env.burn_units(&u, &acc, &u.pubkey(), ONE / 10, 0), RookError::OverCap);
}

// FloorModule.t.sol: test_theBooksDepthBoundsPublicMinting, test_publicMintsStopAtTheETHBackedCap
#[test]
fn the_books_depth_bounds_public_minting() {
    let mut env = Env::new();
    let c = env.custodian.insecure_clone();
    ok(env.post_as(&c, FLOOR, 9_000, 1)); // public cap: one convertible unit
    let u = env.user.insecure_clone();
    let acc = env.token_account(&u.pubkey());
    expect_err(env.mint_units(&u, &acc, 300 * SOL, 0), RookError::OverBacked);
    ok(env.mint_units(&u, &acc, 200 * SOL, 0)); // 0.99 unit, 0.75 of it convertible
    expect_err(env.mint_units(&u, &acc, 80 * SOL, 0), RookError::OverBacked);
}

// FloorModule.t.sol: test_slippageAndBadInputs
#[test]
fn slippage_and_bad_inputs() {
    let mut env = Env::new();
    ok(env.post(FLOOR));
    let u = env.user.insecure_clone();
    let acc = env.token_account(&u.pubkey());
    expect_err(env.mint_units(&u, &acc, 0, 0), RookError::BadInput);
    expect_err(env.mint_units(&u, &acc, 10 * SOL, ONE), RookError::Slippage);
    expect_err(env.mint_units(&u, &acc, 1, 0), RookError::Slippage);
    ok(env.mint_units(&u, &acc, 10 * SOL, 0));
    expect_err(env.burn_units(&u, &acc, &u.pubkey(), 0, 0), RookError::BadInput);
    expect_err(env.burn_units(&u, &acc, &u.pubkey(), 1_000, 10 * SOL), RookError::Slippage);
}

// FloorModule.t.sol: test_movementWidensBothSidesAndDecaysOverAnHour, test_stalenessWidensFromTenMinutesToThreeHours,
// test_stressWidensMintingOnly, test_theSpreadNeverExceedsTheMaximum, test_lowConfidenceWidensBothSides
#[test]
fn the_spreads() {
    let mut env = Env::new();
    ok(env.post(FLOOR));
    let e = 0;
    let m = env.module();
    assert_eq!(m.spreads(env.now, e).unwrap(), (15, 15));
    // movement: a 5 % move adds 500 bps × volWeight 1×, decaying over an hour
    env.advance_time(3_600);
    ok(env.post(2 * FLOOR));
    let m = env.module();
    assert_eq!(m.spreads(env.now, e).unwrap(), (515, 515));
    let (s, _) = m.spreads(env.now + 1_800, e).unwrap();
    assert!(s > 15 + 100 && s < 515, "decays (and staleness starts at 10 min)");
    // staleness: +100 at three hours
    let (s, b) = m.spreads(env.now + FLOOR_TTL, e).unwrap();
    assert_eq!((s, b), (115, 115));
    // low confidence: +100 at 0, nothing at 8000
    let c = env.custodian.insecure_clone();
    env.advance_time(3_600);
    ok(env.post_as(&c, env.module().accepted, 4_000, 20));
    let m = env.module();
    assert_eq!(m.spreads(env.now, e).unwrap().1, 15 + 50);
    // stress widens minting only, and nothing exceeds the maximum
    let (sm, sb) = m.spreads(env.now, 10 * ONE).unwrap();
    assert_eq!(sm - sb, 300);
    let mut p = default_params();
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

// FloorModule.t.sol: test_theLagIsAlwaysPricedAgainstTheTrader
#[test]
fn the_lag_is_priced_against_the_trader() {
    let mut env = Env::new();
    ok(env.post(FLOOR));
    env.advance_time(3_600);
    ok(env.post(FLOOR * 3 / 2)); // accepted +5 %, target +50 %: mints pay the target (up to the day band)
    let m = env.module();
    assert_eq!(m.mint_base(env.now).unwrap(), FLOOR * 12 / 10);
    env.advance_time(3_600);
    ok(env.post(FLOOR / 2)); // target far below: burns count no lower than the next hour's legal floor
    let m = env.module();
    assert!(m.burn_target(env.now).unwrap() >= m.accepted * (BPS - 500) / BPS);
}

// The poster's switches. EthNftFlow.t.sol: test_thePosterIsTheCanister_andTheRegistrySwitchZeroesIt,
// test_D_M_1_thePosterHasItsOwnSwitch_theGuardianPausesOneCollection_theSafeResumesIt,
// test_thePauseStopsEveryInflowPricedByThePoster_butNotBurns
#[test]
fn the_pauses_stop_every_inflow_priced_by_the_poster_but_not_burns() {
    let mut env = Env::new();
    ok(env.post(FLOOR));
    let acc = env.user_with_units(400 * SOL);
    let u = env.user.insecure_clone();
    let g = env.guardian.insecure_clone();
    // the guardian pauses this collection's poster
    ok(env.listing_admin(rook_core::instruction::PausePoster {}, &g));
    expect_err(env.post(FLOOR), RookError::NotKeeper);
    expect_err(env.mint_units(&u, &acc, SOL, 0), RookError::Paused);
    expect_err(env.advance(SOL), RookError::OverItemCap);
    let before = env.lamports(&u.pubkey());
    ok(env.burn_units(&u, &acc, &u.pubkey(), ONE / 10, 1));
    assert!(env.lamports(&u.pubkey()) > before - 5_000);
    // even three hours on (stale), burns go on at the pause floor
    env.advance_time(FLOOR_TTL + 60);
    ok(env.burn_units(&u, &acc, &u.pubkey(), ONE / 10, 1));
    // only the owner resumes
    expect_err(env.listing_admin(rook_core::instruction::UnpausePoster {}, &g), RookError::NotOwner);
    let o = env.owner.insecure_clone();
    ok(env.listing_admin(rook_core::instruction::UnpausePoster {}, &o));
    ok(env.post(FLOOR));
    ok(env.mint_units(&u, &acc, SOL, 0));
    // the registry's keeper switch stops the poster too
    ok(env.registry_admin(rook_core::instruction::PauseKeeper {}, Some(&g)));
    expect_err(env.post(FLOOR), RookError::NotKeeper);
    expect_err(env.mint_units(&u, &acc, SOL, 0), RookError::Paused);
    ok(env.burn_units(&u, &acc, &u.pubkey(), ONE / 10, 1));
    expect_err(env.registry_admin(rook_core::instruction::UnpauseKeeper {}, Some(&g)), RookError::NotOwner);
    ok(env.registry_admin(rook_core::instruction::UnpauseKeeper {}, None));
    ok(env.post(FLOOR));
    env.check_books();
    env.check_ledger();
}

// FloorModule (B-04, `escaped()`): a week with no good post, burns exit at the backing per unit; skim takes the surplus.
#[test]
fn the_stale_escape_pays_the_backing_and_skims_the_surplus() {
    let mut env = Env::new();
    ok(env.post(FLOOR));
    let acc = env.user_with_units(400 * SOL);
    ok(env.fund_reserve(5 * SOL));
    let u = env.user.insecure_clone();
    env.advance_time(FLOOR_TTL + 10);
    expect_err(env.burn_units(&u, &acc, &u.pubkey(), ONE / 10, 0), RookError::Stale);
    env.advance_time(STALE_ESCAPE);
    let m = env.module();
    assert!(m.escaped(env.now));
    let b = m.beta(env.wrapper().eth_backed).unwrap();
    let before = env.lamports(&u.pubkey());
    ok(env.burn_units(&u, &acc, &u.pubkey(), ONE / 10, 0));
    let got = env.lamports(&u.pubkey()) + 5_000 - before;
    let pr = b as u128 * (BPS - 15) as u128 / BPS as u128;
    assert_eq!(got as u128, (ONE / 10) as u128 * pr / ONE as u128);
    let t0 = env.lamports(&env.treasury);
    ok(env.skim());
    assert!(env.lamports(&env.treasury) - t0 >= 5 * SOL);
    let m = env.module();
    assert_eq!((m.surplus, m.revenue), (0, 0));
    env.check_books();
}

// FloorReserve.t.sol: test_fundReserveJoinsTheSurplus_notTheUnitsBacking; FloorModule (skim beyond revenueTarget)
#[test]
fn fund_reserve_joins_the_surplus_and_skim_takes_revenue_beyond_the_target() {
    let mut env = Env::new();
    ok(env.post(FLOOR));
    expect_err(env.fund_reserve(0), RookError::BadInput);
    ok(env.fund_reserve(3 * SOL));
    let m = env.module();
    assert_eq!((m.surplus, m.backing, m.reserve), (3 * SOL, 0, 0));
    expect_err(env.skim(), RookError::NothingToSettle);
    env.check_books();
}

// Advance.t.sol: test_advancePaysThePayeeNowAndPutsOneUnitInTransit, test_advanceRefusals,
// test_noAdvanceWhenStale_orWhileTheKeeperIsPaused_orWithoutAWholeUnit
#[test]
fn advance_pays_the_custodian_and_puts_one_unit_in_transit() {
    let mut env = Env::new();
    ok(env.post(FLOOR));
    expect_err(env.advance(FLOOR), RookError::OverItemCap); // no whole convertible unit
    let conv = Keypair::new();
    env.svm.airdrop(&conv.pubkey(), 10_000 * SOL).unwrap();
    env.allow_converter(&conv.pubkey());
    let dest = env.token_account(&conv.pubkey());
    ok(env.convert(&conv, &dest, 1_000 * SOL, 0)); // ≈4.85 units, 3.6 convertible
    let e = env.wrapper().eth_backed;
    let m = env.module();
    let max = m.max_item_cost(env.now, e, false).unwrap();
    assert_eq!(max, FLOOR * (BPS + 300) / BPS);
    expect_err(env.advance(0), RookError::BadInput);
    expect_err(env.advance(max + 1), RookError::OverItemCap);
    let u = env.user.insecure_clone();
    let i = env.advance_ix(&u.pubkey(), max);
    expect_err(env.send(&[i], &[&u]), RookError::NotCustodian);
    let c0 = env.lamports(&env.custodian.pubkey());
    ok(env.advance(max));
    assert_eq!(env.lamports(&env.custodian.pubkey()) - c0 + 5_000, max);
    let m = env.module();
    assert_eq!((m.transit_units, m.advanced), (ONE, max));
    expect_err(env.advance(1), RookError::AdvanceOpen);
    env.check_books();
    // stale: no advance
    ok(env.return_advance_as(&env.custodian.insecure_clone(), max));
    env.advance_time(FLOOR_TTL);
    expect_err(env.advance(SOL), RookError::OverItemCap);
}

// Advance.t.sol: test_settleTurnsTheTransitUnitIntoTheItem_noEthMoves, test_settleRefusals, test_aSecondAdvanceAfterSettlement;
// EthNftFlow.t.sol: test_settleNeedsTheUnitAtTheModule, test_aGraduationsBudgetBuysAnNft_andTheUnitBecomesItemBacked
#[test]
fn settle_turns_the_transit_unit_into_the_item() {
    let mut env = Env::new();
    ok(env.post(FLOOR));
    let conv = Keypair::new();
    env.svm.airdrop(&conv.pubkey(), 10_000 * SOL).unwrap();
    env.allow_converter(&conv.pubkey());
    let dest = env.token_account(&conv.pubkey());
    ok(env.convert(&conv, &dest, 1_000 * SOL, 0));
    expect_err(env.settle(SOL), RookError::NoAdvance);
    let max = env.module().max_item_cost(env.now, env.wrapper().eth_backed, false).unwrap();
    ok(env.advance(max));
    expect_err(env.settle(SOL), RookError::NoWrapped);
    let mwx = env.module_wx;
    ok(env.wrap_nft(42, tx_hash(1), &mwx));
    expect_err(env.settle(0), RookError::BadInput);
    expect_err(env.settle(max + 1), RookError::BadInput);
    let e0 = env.wrapper().eth_backed;
    let v0 = env.vault_free();
    ok(env.settle(max - SOL));
    assert_eq!(env.wrapper().eth_backed, e0 - ONE);
    assert_eq!(env.balance(&mwx), 0);
    assert_eq!(env.vault_free(), v0, "no lamports move");
    let m = env.module();
    assert_eq!((m.transit_units, m.items_arrived, m.fill_count), (0, 1, 1));
    env.check_books();
    env.check_ledger();
    // a second advance after settlement
    let max = env.module().max_item_cost(env.now, env.wrapper().eth_backed, false).unwrap();
    ok(env.advance(max.min(FLOOR)));
}

// Advance.t.sol: test_returnAtOrAboveTheBasisRestoresTheUnit_theRestIsRevenue, test_aShortReturnIsCoveredByRevenueThenSurplus,
// test_aReturnTheyCannotCoverIsALossSharedByTheConvertibleUnits, test_returnRefusals
#[test]
fn return_advance_restores_the_basis() {
    let mut env = Env::new();
    ok(env.post(FLOOR));
    let conv = Keypair::new();
    env.svm.airdrop(&conv.pubkey(), 10_000 * SOL).unwrap();
    env.allow_converter(&conv.pubkey());
    let dest = env.token_account(&conv.pubkey());
    ok(env.convert(&conv, &dest, 1_000 * SOL, 0));
    let c = env.custodian.insecure_clone();
    expect_err(env.return_advance_as(&c, SOL), RookError::NoAdvance);
    let backing0 = env.module().backing;
    ok(env.advance(FLOOR));
    let basis = env.module().advance_basis;
    let u = env.user.insecure_clone();
    expect_err(env.return_advance_as(&u, FLOOR), RookError::NotCustodian);
    ok(env.return_advance_as(&c, FLOOR + SOL));
    let m = env.module();
    assert_eq!(m.backing, backing0);
    assert_eq!(m.transit_units, 0);
    assert!(basis >= FLOOR);
    assert_eq!(m.revenue, SOL, "what came back above the basis, plus what the basis kept above the advance");
    // a short return: revenue, then surplus, then a loss
    ok(env.fund_reserve(SOL));
    ok(env.advance(FLOOR));
    let m0 = env.module();
    ok(env.return_advance_as(&c, 0));
    let m1 = env.module();
    let need = m0.advance_basis;
    let from_rev = need.min(m0.revenue);
    let from_sur = (need - from_rev).min(m0.surplus);
    assert_eq!(m1.revenue, m0.revenue - from_rev);
    assert_eq!(m1.surplus, m0.surplus - from_sur);
    assert_eq!(m1.backing, m0.backing + need - (need - from_rev - from_sur));
    env.check_books();
}

// Advance.t.sol: test_theDailyCountIsTakenAtTheAdvance_andNotAgainAtSettlement; FloorModule.t.sol:
// test_itemArrivalsAreLimitedPerDay_onBothClocks
#[test]
fn item_arrivals_are_limited_per_day() {
    let mut env = Env::new(); // maxItemsPerDay 3
    ok(env.post(FLOOR));
    let conv = Keypair::new();
    env.svm.airdrop(&conv.pubkey(), 100_000 * SOL).unwrap();
    env.allow_converter(&conv.pubkey());
    let dest = env.token_account(&conv.pubkey());
    ok(env.convert(&conv, &dest, 2_000 * SOL, 0));
    let c = env.custodian.insecure_clone();
    for _ in 0..3 {
        ok(env.advance(FLOOR));
        ok(env.return_advance_as(&c, FLOOR));
    }
    // the daily count is not given back by the returns
    expect_err(env.advance(FLOOR), RookError::OverItemCap);
    env.advance_time(86_400);
    ok(env.post(FLOOR));
    ok(env.advance(FLOOR));
}

// FloorModule.t.sol: test_itemArrivedTurnsOneUnitIntoAnItem, test_itemArrivedRefusals,
// test_aConversionAboveTheBackingIsPaidByTheReserve_orWaits, test_aSnapshotCoversACapDropBetweenPurchaseAndRecord
#[test]
fn item_arrived_turns_one_unit_into_an_item() {
    let mut env = Env::new();
    ok(env.post(FLOOR));
    let conv = Keypair::new();
    env.svm.airdrop(&conv.pubkey(), 10_000 * SOL).unwrap();
    env.allow_converter(&conv.pubkey());
    let dest = env.token_account(&conv.pubkey());
    ok(env.convert(&conv, &dest, 1_000 * SOL, 0));
    let mwx = env.module_wx;
    expect_err(env.item_arrived(FLOOR), RookError::NoWrapped);
    ok(env.wrap_nft(1, tx_hash(1), &mwx));
    expect_err(env.item_arrived(0), RookError::BadInput);
    let cap = FLOOR * (BPS + 300) / BPS;
    expect_err(env.item_arrived(cap + 1), RookError::OverItemCap);
    // a snapshot covers a cap drop between purchase and record
    ok(env.snapshot());
    env.advance_time(3_600);
    ok(env.post(FLOOR / 2));
    ok(env.item_arrived(cap));
    let m = env.module();
    assert_eq!(m.items_arrived, 1);
    env.check_books();
    env.check_ledger();
    // stale: CapStale (the snapshot has expired too)
    ok(env.wrap_nft(2, tx_hash(2), &mwx));
    env.advance_time(FLOOR_TTL);
    expect_err(env.item_arrived(FLOOR / 2), RookError::CapStale);
}

// FloorReserve.t.sol: test_fillsMoveThePriceWithinTheBand_timeWeighted, test_beforeAnyFillTheKeeperFloorAlone,
// test_theBandBoundsWhatFillsCanDo_andTheCapStaysOnTheKeeperFloor
#[test]
fn fills_move_the_price_within_the_band() {
    let mut env = Env::new();
    ok(env.post(FLOOR));
    assert_eq!(env.module().price(env.now).unwrap(), FLOOR);
    let conv = Keypair::new();
    env.svm.airdrop(&conv.pubkey(), 10_000 * SOL).unwrap();
    env.allow_converter(&conv.pubkey());
    let dest = env.token_account(&conv.pubkey());
    ok(env.convert(&conv, &dest, 1_000 * SOL, 0));
    ok(env.advance(FLOOR));
    let mwx = env.module_wx;
    ok(env.wrap_nft(1, tx_hash(1), &mwx));
    ok(env.settle(FLOOR / 2)); // a fill far below: the price stays within 3 % of the floor
    env.advance_time(2 * 3_600);
    let m = env.module();
    let p = m.price(env.now).unwrap();
    assert_eq!(p, FLOOR * (BPS - 300) / BPS);
    assert_eq!(m.item_cap(env.now, false).unwrap(), FLOOR * (BPS + 300) / BPS, "the cap stays on the poster's floor");
}
