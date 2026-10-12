//! Audit 2026-10, area A1 (docs/v9/audit-2026-10/a1-core-floor.md): rook_core's floor module and money core.
//!
//! `poc_*` tests assert the property the code should have; they FAIL on the audited commit (f2c24e0) while the finding stands.
//! `demo_*` tests show a behaviour that holds today (they pass while the finding stands).
//! `measure_*` tests quantify a property (an accepted risk, the books, compute units) and pass.
//!
//! Build the SBPF v2 binary first:
//!   cargo build-sbf --manifest-path programs/rook_core/Cargo.toml --arch v2 --sbf-out-dir target/deploy-sbpfv2
//! then: cargo test -p rook_core --test audit_a1_core -- --nocapture

#[path = "suite/common.rs"]
mod common;

use anchor_lang::prelude::Pubkey;
use common::*;
use rook_core::error::RookError;
use rook_core::state::*;
use rook_core::units::{BPS, ONE};
use solana_keypair::Keypair;
use solana_signer::Signer;

use rook_core::hashing::BTC_BURN_ADDRESS;

fn funded(env: &mut Env, sol: u64) -> (Keypair, Pubkey) {
    let k = Keypair::new();
    env.svm.airdrop(&k.pubkey(), sol * SOL).unwrap();
    let acc = env.token_account(&k.pubkey());
    (k, acc)
}

/// Mint with `lamports`, return the units received.
fn mint(env: &mut Env, who: &Keypair, acc: &Pubkey, lamports: u64) -> u64 {
    let before = env.balance(acc);
    ok(env.mint_units(who, acc, lamports, 0));
    env.balance(acc) - before
}

/// Burn `units` to the burner's own wallet, return the lamports received.
fn burn(env: &mut Env, who: &Keypair, acc: &Pubkey, units: u64) -> u64 {
    let before = env.lamports(&who.pubkey());
    let to = who.pubkey();
    // the burner pays the fee (5000 lamports per signature in LiteSVM); add it back
    ok(env.burn_units(who, acc, &to, units, 0));
    env.lamports(&who.pubkey()) + 5_000 - before
}

// ---- A1-01: pause_poster is a silent no-op on an Inscriptions listing ---------------------------------------------------

/// A1-01. The guardian's per-collection switch (`pause_poster`, D-M-1) succeeds on an Inscriptions listing and emits
/// `PosterPausedSet { paused: true }`, but `Listing::keeper_down` ignores `poster_paused` for that kind: the keeper keeps
/// posting and the public keeps minting at its word. The property asserted (pausing stops the collection's floor, as it does
/// for an EthNft listing) FAILED on f2c24e0; it passes since the fix of 2026-10-10 (`keeper_down`: `poster_paused ||
/// live_keeper().is_none()` for Inscriptions).
#[test]
fn poc_a1_01_pausing_an_inscriptions_poster_stops_its_floor() {
    let mut env = Env::bare();
    let c = env.custodian.pubkey();
    ok(env.list(ListingKind::Inscriptions { membership_root: [7u8; 32] }, "Frogs", BTC_BURN_ADDRESS, c, c));
    env.init_wrapper();
    env.init_module(default_params());
    let k = env.keeper.insecure_clone();
    ok(env.post_as(&k, FLOOR, 9_000, 20));
    // the guardian pauses this collection's floor: the call succeeds
    let g = env.guardian.insecure_clone();
    ok(env.listing_admin(rook_core::instruction::PausePoster {}, &g));
    let l: Listing = env.get(&env.listing);
    assert!(l.poster_paused, "the switch is recorded");
    // ...but nothing stops: the keeper still posts and the public still mints
    env.advance_time(600);
    let post = env.post_as(&k, FLOOR * 104 / 100, 9_000, 20);
    let (u, acc) = funded(&mut env, 1_000);
    let mint = env.mint_units(&u, &acc, 2 * FLOOR, 0);
    let posted = post.is_ok();
    let minted = mint.is_ok();
    println!("A1-01: after pause_poster on an Inscriptions listing: post accepted = {posted}, public mint accepted = {minted}");
    assert!(!posted, "A1-01: the keeper still posts a paused Inscriptions collection's floor");
    assert!(!minted, "A1-01: the public still mints on a paused Inscriptions collection's floor");
}

/// Control for A1-01: the same switch on an EthNft listing stops posts and mints (Paused / NotKeeper).
#[test]
fn demo_a1_01_control_the_switch_works_on_an_ethnft_listing() {
    let mut env = Env::new();
    ok(env.post(FLOOR));
    let g = env.guardian.insecure_clone();
    ok(env.listing_admin(rook_core::instruction::PausePoster {}, &g));
    expect_err(env.post(FLOOR), RookError::NotKeeper);
    let (u, acc) = funded(&mut env, 1_000);
    expect_err(env.mint_units(&u, &acc, 2 * FLOOR, 0), RookError::Paused);
}

// ---- A1-02: anyone overwrites the item-cap snapshot downward --------------------------------------------------------------

/// A1-02 (Low, inherited from FloorModule.snapshotCap). The canister snapshots the item cap before a purchase; once the floor
/// falls, anyone snapshots again and the lower cap replaces the higher one, so `item_arrived` at the snapshotted cost is refused.
#[test]
fn demo_a1_02_anyone_lowers_the_snapshot_cap() {
    let mut env = Env::new();
    ok(env.post(FLOOR));
    let (u, acc) = funded(&mut env, 10_000);
    let _ = mint(&mut env, &u, &acc, 4 * FLOOR);
    let mwx = env.module_wx;
    ok(env.wrap_nft(1, tx_hash(1), &mwx));
    // the canister snapshots at the fresh cap, then buys
    ok(env.snapshot());
    let cap = env.module().snap_cap;
    // an hour later the floor is 5 % lower; a griefer snapshots again
    env.advance_time(3_600);
    ok(env.post(FLOOR / 2));
    ok(env.snapshot());
    let lowered = env.module().snap_cap;
    assert!(lowered < cap);
    println!("A1-02: snapshot {cap} overwritten by anyone with {lowered} ({} bps lower)", (cap - lowered) * BPS / cap);
    expect_err(env.item_arrived(cap), RookError::OverItemCap);
    ok(env.item_arrived(lowered));
}

// ---- A1-05: a stray unit in module_wx satisfies NoWrapped ----------------------------------------------------------------

/// A1-05 (Informational). Anyone can mint (or transfer) wX into the module's token account; such a unit can never leave
/// (`reclaimStray` was not ported) and it satisfies `item_arrived`'s / `settle_advance`'s `NoWrapped` check with no item wrapped.
#[test]
fn demo_a1_05_a_stray_public_unit_satisfies_no_wrapped() {
    let mut env = Env::new();
    ok(env.post(FLOOR));
    let (u, acc) = funded(&mut env, 10_000);
    let _ = mint(&mut env, &u, &acc, 2 * FLOOR);
    let mwx = env.module_wx;
    // a public mint straight into module_wx
    ok(env.mint_units(&u, &mwx, 2 * FLOOR, 0));
    assert!(env.balance(&mwx) >= ONE);
    let held = env.wrapper().held;
    ok(env.item_arrived(FLOOR));
    assert_eq!(env.wrapper().held, held, "no item was wrapped");
    env.check_books();
    env.check_ledger();
}

// ---- Q15 (accepted risk): quantified --------------------------------------------------------------------------------------

/// Q15 measured. (a) An atomic sandwich of an upward post (mint before, burn right after) LOSES: the post sets `vol_bps` to the
/// move it made and `vol_weight_bps` = 100 % charges all of it to the burn. (b) Minting before an upward post and holding while
/// the accepted floor climbs captures the target's jump, but every burn is capped at the tier's backing per unit, so the gain is
/// taken only from holders whose basis is above the new floor (dilution); against a pool minted at the attacker's own price it
/// is ~0. (c) With a pool minted higher (floor fell, then rebounds) the gain is the jump, bounded by 5 units/h and the day band.
#[test]
fn measure_a1_q15_front_running_a_floor_post() {
    // (a) atomic sandwich across a +1.67 % post (20 min after the last)
    let mut env = Env::new();
    ok(env.post(FLOOR));
    let (u, uacc) = funded(&mut env, 10_000);
    let _ = mint(&mut env, &u, &uacc, 4 * FLOOR);
    env.advance_time(3_600);
    ok(env.post(FLOOR));
    env.advance_time(1_200);
    let (a, aacc) = funded(&mut env, 10_000);
    let paid_in = FLOOR;
    let units = mint(&mut env, &a, &aacc, paid_in);
    ok(env.post(FLOOR * 110 / 100));
    let moved = env.module().move_acc;
    let out = burn(&mut env, &a, &aacc, units);
    let pnl_a = out as i128 - paid_in as i128;
    println!("Q15 (a) atomic: post moved {moved} bps; in {paid_in}, out {out}, pnl {pnl_a} ({} bps)", pnl_a * 10_000 / paid_in as i128);
    assert!(pnl_a < 0, "an atomic sandwich of one post loses");
    env.check_books();

    // (b) and (c): mint before a post that lifts the target 25 % above a floor that fell 20 %, hold 4 h, burn
    for high_basis in [false, true] {
        let mut env = Env::new();
        ok(env.post(FLOOR));
        let (u, uacc) = funded(&mut env, 10_000);
        if high_basis {
            // holders minted at FLOOR; then the floor falls to 0.8 FLOOR
            let _ = mint(&mut env, &u, &uacc, 5 * FLOOR);
        }
        for _ in 0..30 {
            env.advance_time(3_600);
            ok(env.post(FLOOR * 8 / 10));
        }
        if !high_basis {
            // holders minted at the low floor
            let _ = mint(&mut env, &u, &uacc, 4 * FLOOR);
            env.advance_time(3_600);
            ok(env.post(FLOOR * 8 / 10));
        }
        env.advance_time(1_200);
        let (a, aacc) = funded(&mut env, 10_000);
        let paid_in = 4 * FLOOR;
        let units = mint(&mut env, &a, &aacc, paid_in);
        // the poster's word: back to FLOOR; posts every 20 min for 4 h, a 50 min gap (the volatility spread decays), then the exit
        for _ in 0..12 {
            ok(env.post(FLOOR));
            env.advance_time(1_200);
        }
        ok(env.post(FLOOR));
        env.advance_time(3_000);
        ok(env.post(FLOOR));
        let m = env.module();
        let out = burn(&mut env, &a, &aacc, units);
        let pnl = out as i128 - paid_in as i128;
        println!(
            "Q15 ({}) hold 4 h: accepted {} -> {:.3} FLOOR, in {paid_in}, out {out}, pnl {pnl} ({} bps)",
            if high_basis { "c, pool minted at FLOOR" } else { "b, pool minted at 0.8 FLOOR" },
            FLOOR * 8 / 10,
            m.accepted as f64 / FLOOR as f64,
            pnl * 10_000 / paid_in as i128
        );
        env.check_books();
        env.check_ledger();
        if high_basis {
            assert!(pnl > 0);
        } else {
            assert!(pnl <= paid_in as i128 / 100, "against a low-basis pool the burn is capped at the backing");
        }
    }
}

// ---- books and compute ----------------------------------------------------------------------------------------------------

/// Every money instruction keeps `vault − rent ≥ books` and `supply = (held − open)·ONE + eth_backed`, and its compute.
#[test]
fn measure_a1_books_ledger_and_compute_units() {
    let mut env = Env::new();
    ok(env.post(FLOOR));
    let (u, uacc) = funded(&mut env, 100_000);
    let cu = |m: &litesvm::types::TransactionMetadata| m.compute_units_consumed;
    let r = ok(env.mint_units(&u, &uacc, 4 * FLOOR, 0));
    println!("CU mint {}", cu(&r));
    env.check_books();
    env.check_ledger();
    let conv = Keypair::new();
    env.svm.airdrop(&conv.pubkey(), 100_000 * SOL).unwrap();
    env.allow_converter(&conv.pubkey());
    let dest = env.token_account(&conv.pubkey());
    let r = ok(env.convert(&conv, &dest, 2_000 * SOL, 0));
    println!("CU convert {}", cu(&r));
    env.check_books();
    env.check_ledger();
    let r = ok(env.advance(FLOOR));
    println!("CU advance {}", cu(&r));
    env.check_books();
    let mwx = env.module_wx;
    ok(env.wrap_nft(1, tx_hash(1), &mwx));
    let r = ok(env.settle(FLOOR));
    println!("CU settle_advance {}", cu(&r));
    env.check_books();
    env.check_ledger();
    let b = env.balance(&uacc);
    let to = u.pubkey();
    let r = ok(env.burn_units(&u, &uacc, &to, b / 2, 0));
    println!("CU burn {}", cu(&r));
    env.check_books();
    env.check_ledger();
    // fill the price ring (8 fills) and measure the worst-case price() walk in a mint
    for i in 0..8u64 {
        ok(env.advance(FLOOR));
        ok(env.wrap_nft(10 + i, tx_hash(10 + i), &mwx));
        ok(env.settle(FLOOR - i));
        env.advance_time(86_400 / 2 + 1);
        ok(env.post(FLOOR));
    }
    let r = ok(env.mint_units(&u, &uacc, FLOOR, 0));
    println!("CU mint with 8 fills in the window {}", cu(&r));
    let r = ok(env.post(FLOOR));
    println!("CU post_floor {}", cu(&r));
    env.check_books();
    env.check_ledger();
    let m = env.module();
    assert!(env.vault_free() as u128 >= m.books());
}
