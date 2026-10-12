//! `convert`: the interface rook_backing depends on (the EVM graduation conversion, `enqueue` → `_tryFill`).
//! Sources: test/floor/FloorGraduation.t.sol, test/floor/FloorAttacks.t.sol, test/floor/FloorReserve.t.sol.

use super::common::*;
use rook_core::error::RookError;
use rook_core::state::FLOOR_TTL;
use rook_core::units::{BPS, ONE};
use solana_keypair::Keypair;
use solana_signer::Signer;

fn setup() -> (Env, Keypair, anchor_lang::prelude::Pubkey) {
    let mut env = Env::new();
    ok(env.post(FLOOR));
    let conv = Keypair::new();
    env.svm.airdrop(&conv.pubkey(), 50_000 * SOL).unwrap();
    env.allow_converter(&conv.pubkey());
    let dest = env.token_account(&conv.pubkey());
    (env, conv, dest)
}

// FloorGraduation.t.sol: test_aGraduationConvertsItsBudgetAtOnce; FloorReserve.t.sol: test_aMintSplitsIntoTheTwoPots
#[test]
fn a_conversion_converts_at_once_at_the_grad_price() {
    let (mut env, conv, dest) = setup();
    let lamports = 824 * SOL / 10; // a 4.12 ETH item budget at 20 SOL/ETH
    let r = ok(env.convert(&conv, &dest, lamports, 0));
    let grad = (FLOOR as u128 * (BPS + 300) as u128).div_ceil(BPS as u128);
    let units = (lamports as u128 * ONE as u128 / grad) as u64;
    assert_eq!(env.balance(&dest), units);
    let m = env.module();
    assert_eq!(m.reserve, lamports / 4, "reserveBps of the lamports");
    assert_eq!(m.reserve_units, units / 4, "and of the units");
    assert_eq!(m.backing, lamports - lamports / 4);
    assert_eq!((m.converted_lamports, m.converted_units), (lamports, units));
    assert_eq!(env.wrapper().eth_backed, units);
    assert_eq!(env.vault_free(), lamports);
    assert_eq!(m.net_units(units), units - units / 4, "the reserve tier is never advanced");
    env.check_books();
    env.check_ledger();
    assert!(r.compute_units_consumed < 200_000);
}

#[test]
fn the_payer_may_differ_from_the_converter() {
    let (mut env, conv, dest) = setup();
    let payer = Keypair::new();
    env.svm.airdrop(&payer.pubkey(), 1_000 * SOL).unwrap();
    let i = env.convert_ix(&conv.pubkey(), &payer.pubkey(), &dest, 100 * SOL, 0);
    ok(env.send(&[i], &[&payer, &conv]));
    assert!(env.lamports(&payer.pubkey()) < 900 * SOL + 1);
    assert!(env.balance(&dest) > 0);
}

// FloorGraduation.t.sol: test_onlyACurveUsesTheGraduationLane
#[test]
fn only_an_allowed_converter_converts() {
    let (mut env, conv, dest) = setup();
    let u = env.user.insecure_clone();
    let udest = env.token_account(&u.pubkey());
    expect_err(env.convert(&u, &udest, 100 * SOL, 0), RookError::NotConverter);
    ok(env.registry_admin(rook_core::instruction::RemoveConverter { converter: conv.pubkey() }, None));
    expect_err(env.convert(&conv, &dest, 100 * SOL, 0), RookError::NotConverter);
    // only the owner manages the allow-list
    let g = env.guardian.insecure_clone();
    expect_err(
        env.registry_admin(rook_core::instruction::ProposeConverter { converter: conv.pubkey() }, Some(&g)),
        RookError::NotOwner,
    );
}

// A proposed converter converts nothing until accepted (CHANGE_DELAY later), and a cancelled one never.
#[test]
fn a_proposed_converter_cannot_convert() {
    let mut env = Env::new();
    let conv = Keypair::new();
    env.svm.airdrop(&conv.pubkey(), 1_000 * SOL).unwrap();
    let dest = env.token_account(&conv.pubkey());
    ok(env.registry_admin(rook_core::instruction::ProposeConverter { converter: conv.pubkey() }, None));
    ok(env.post(FLOOR));
    expect_err(env.convert(&conv, &dest, 100 * SOL, 0), RookError::NotConverter);
    env.advance_time(rook_core::state::CHANGE_DELAY);
    ok(env.post(FLOOR));
    expect_err(env.convert(&conv, &dest, 100 * SOL, 0), RookError::NotConverter); // ready, not yet accepted
    ok(env.registry_admin(rook_core::instruction::AcceptConverter {}, None));
    ok(env.convert(&conv, &dest, 100 * SOL, 0));
    // the guardian stops it at once
    let g = env.guardian.insecure_clone();
    ok(env.registry_admin(rook_core::instruction::RemoveConverter { converter: conv.pubkey() }, Some(&g)));
    expect_err(env.convert(&conv, &dest, 100 * SOL, 0), RookError::NotConverter);
}

// FloorGraduation.t.sol: test_aStaleFloorAtTheClose_theGraduationWaits (here: refused); the poster's and the listing's pauses
#[test]
fn a_conversion_needs_a_fresh_floor_the_poster_up_and_the_listing_unpaused() {
    let (mut env, conv, dest) = setup();
    env.advance_time(FLOOR_TTL);
    expect_err(env.convert(&conv, &dest, 100 * SOL, 0), RookError::Stale);
    ok(env.post(FLOOR));
    let g = env.guardian.insecure_clone();
    ok(env.listing_admin(rook_core::instruction::PausePoster {}, &g));
    expect_err(env.convert(&conv, &dest, 100 * SOL, 0), RookError::Paused);
    let o = env.owner.insecure_clone();
    ok(env.listing_admin(rook_core::instruction::UnpausePoster {}, &o));
    ok(env.registry_admin(rook_core::instruction::PauseKeeper {}, Some(&g)));
    expect_err(env.convert(&conv, &dest, 100 * SOL, 0), RookError::Paused);
    ok(env.registry_admin(rook_core::instruction::UnpauseKeeper {}, None));
    ok(env.listing_admin(rook_core::instruction::SetPaused { paused: true }, &o));
    expect_err(env.convert(&conv, &dest, 100 * SOL, 0), RookError::ListingPaused);
    ok(env.listing_admin(rook_core::instruction::SetPaused { paused: false }, &o));
    ok(env.convert(&conv, &dest, 100 * SOL, 0));
    // a low-confidence post is stale too
    let c = env.custodian.insecure_clone();
    env.advance_time(3_600);
    ok(env.post_as(&c, FLOOR, 1_000, 20));
    expect_err(env.convert(&conv, &dest, 100 * SOL, 0), RookError::Stale);
}

// FloorGraduation.t.sol: test_theHardCapHoldsALaterCloseInTrading_untilConversionsMakeRoom
#[test]
fn the_hard_cap_holds() {
    let mut p = default_params();
    p.max_public_eth_backed = 2 * ONE;
    p.max_eth_backed = 2 * ONE;
    let mut env = Env::with_params(p);
    ok(env.post(FLOOR));
    let conv = Keypair::new();
    env.svm.airdrop(&conv.pubkey(), 50_000 * SOL).unwrap();
    env.allow_converter(&conv.pubkey());
    let dest = env.token_account(&conv.pubkey());
    expect_err(env.convert(&conv, &dest, 500 * SOL, 0), RookError::OverBacked);
    ok(env.convert(&conv, &dest, 400 * SOL, 0));
    expect_err(env.convert(&conv, &dest, 20 * SOL, 0), RookError::OverBacked);
}

#[test]
fn slippage_and_bad_inputs() {
    let (mut env, conv, dest) = setup();
    expect_err(env.convert(&conv, &dest, 0, 0), RookError::BadInput);
    expect_err(env.convert(&conv, &dest, 1, 0), RookError::Slippage);
    expect_err(env.convert(&conv, &dest, 100 * SOL, ONE), RookError::Slippage);
    // a destination of another mint is refused by the account constraint
    let other = Keypair::new();
    let i = env.convert_ix(&conv.pubkey(), &conv.pubkey(), &other.pubkey(), 100 * SOL, 0);
    expect_fail(env.send(&[i], &[&conv]));
}

// FloorAttacks.t.sol: test_graduationsCannotBeMadeCheapByALowTarget (and A-10: a far-off high target is capped at the day band)
#[test]
fn the_grad_price_follows_the_target_only_within_the_day_band() {
    let (mut env, conv, dest) = setup();
    env.advance_time(3_600);
    ok(env.post(10 * FLOOR)); // accepted +5 %, target ×10: counted only up to anchor × 1.2
    let m = env.module();
    let grad = m.grad_price(env.now).unwrap();
    assert_eq!(grad, (FLOOR as u128 * 12 / 10 * (BPS + 300) as u128).div_ceil(BPS as u128) as u64);
    env.advance_time(3_600);
    ok(env.post(FLOOR / 10)); // a low target: the grad price never drops under the accepted floor's price
    let m = env.module();
    assert!(m.grad_price(env.now).unwrap() >= m.price(env.now).unwrap() * (BPS + 300) / BPS);
    ok(env.convert(&conv, &dest, 100 * SOL, 0));
}
