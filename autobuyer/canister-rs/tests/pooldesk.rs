//! W7: the pool desk against the fake chain. DLMM's own effects are LiteSVM's to check (`solana/tests/dlmm_fixtures.rs`,
//! `tests/solix2_dlmm.rs`); here the pair, the position and the bins are planted from the landed fixtures, and what is checked is what
//! the desk decides from them, what it signs, how it books it (the mint and burn links through the books), and what it records.
mod desk_common;
mod fakerpc;

use std::collections::BTreeMap;

use base64::Engine;
use desk_common::*;
use etch_autobuyer::books::ONE;
use etch_autobuyer::desk::state;
use etch_autobuyer::pooldesk::state as pool;
use etch_autobuyer::pooldesk::{plan, step::PoolStep};
use etch_autobuyer::solix2::dlmm::{self, Lb};
use etch_autobuyer::solix2::{self, native_mint};
use etch_autobuyer::solkey::KeyPath;
use etch_autobuyer::soltx::{self, Pubkey};
use serde_json::Value;

const SOL: u64 = 1_000_000_000;
/// The fixtures' pool: bin step 50, opened at bin 586 (≈ 18.61 SOL a wX), the position over [567, 605].
const ACTIVE: i32 = 586;

fn flow() -> Value {
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/solix2/dlmm.json");
    serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
}

fn fixture(name: &str) -> Vec<u8> {
    let f = flow();
    let a = f["accounts"].as_array().unwrap().iter().find(|a| a["name"] == name).unwrap_or_else(|| panic!("no account {name}"));
    base64::engine::general_purpose::STANDARD.decode(a["data"].as_str().unwrap()).unwrap()
}

fn dlmm_id() -> Pubkey {
    soltx::key(dlmm::DLMM_PROGRAM)
}

fn desk() -> Pubkey {
    pk(KeyPath::Desk(COLL as u32))
}

fn lb() -> Lb {
    Lb::desk(WX)
}

fn bin_price(id: i32) -> u64 {
    plan::bin_floor(dlmm::DESK_BIN_STEP, id)
}

/// A world at the fixtures' price, with the desk listed and the wSOL mint known to the chain.
fn world(floor_bin: i32, desk_sol: u64) -> World {
    let mut w = World::with_floor(bin_price(floor_bin));
    pool::note_fee_income(COLL as u32, 0, 0);
    w.plant(native_mint(), solix2::token_program(), 1_461_600, mint_data([0; 32], 0));
    if desk_sol > 0 {
        w.deposit_sol(&desk(), desk_sol);
    }
    w
}

/// The pair from a fixture, as the desk's own (its wX mint patched in; the active bin as given).
fn plant_pair(w: &mut World, name: &str, active: i32) {
    let mut d = fixture(name);
    d[88..120].copy_from_slice(&WX);
    d[76..80].copy_from_slice(&active.to_le_bytes());
    w.plant(lb().pair, dlmm_id(), 7_182_720, d);
}

fn plant_array(w: &mut World, name: &str) {
    let d = fixture(name);
    let index = i64::from_le_bytes(d[8..16].try_into().unwrap());
    w.plant(dlmm::bin_array_address(&lb().pair, index), dlmm_id(), 71_437_000, d);
}

/// The desk's position from a fixture (`empty`: its liquidity shares zeroed, as after a withdrawal).
fn plant_position(w: &mut World, name: &str, empty: bool) -> Pubkey {
    let mut d = fixture(name);
    if empty {
        d[72..72 + 70 * 16].fill(0);
    }
    let (lower, upper) = (i32::from_le_bytes(d[7912..7916].try_into().unwrap()), i32::from_le_bytes(d[7916..7920].try_into().unwrap()));
    let key = dlmm::position_address(&lb().pair, &desk(), lower, upper - lower + 1);
    w.plant(key, dlmm_id(), 57_406_000, d);
    pool::with_mut(|s| {
        let d = s.desks.entry(COLL as u32).or_default();
        d.position = Some(key);
        d.lo = lower;
        d.hi = upper;
    });
    key
}

fn plant_wsol(w: &mut World, lamports: u64) {
    let ata = soltx::associated_token_address(&desk(), &native_mint());
    w.plant(ata, solix2::token_program(), RENT165 + lamports, token_data(native_mint(), desk(), lamports));
}

/// The steps that landed, in order.
fn landed(_w: &mut World) -> Vec<PoolStep> {
    pool::with(|s| s.history.iter().map(|(_, p)| p.clone()).collect())
}

/// Drives one step: ticks until `signed` is signed (it may already be, in `acc`), plants what the chain would then show, and ticks on until
/// the line reports it landed. The landing tick may already carry the next step signed: `acc` keeps it for the next call.
fn step(w: &mut World, acc: &mut Vec<String>, signed: &str, plant: impl FnOnce(&mut World)) {
    let mut plant = Some(plant);
    let mut planted = false;
    if acc.iter().any(|l| l.contains(signed) && l.contains("signed")) {
        plant.take().unwrap()(w);
        planted = true;
    }
    for _ in 0..8 {
        let log = w.pool_tick();
        acc.extend(log.clone());
        if !planted && log.iter().any(|l| l.contains(signed) && l.contains("signed")) {
            plant.take().unwrap()(w);
            planted = true;
        }
        if log.iter().any(|l| l.contains(signed) && l.contains("landed")) {
            assert!(planted, "{signed} landed without being seen signed: {acc:?}");
            acc.retain(|l| !(l.contains(signed)));
            return;
        }
        w.settle();
    }
    panic!("{signed} never landed: {acc:?}");
}

/// A bin array that exists but holds nothing (its bins zeroed), at `index`.
fn plant_empty_array(w: &mut World, index: i64) {
    let mut d = fixture("bin_array_8");
    d[8..16].copy_from_slice(&index.to_le_bytes());
    for b in d[56..].iter_mut() {
        *b = 0;
    }
    w.plant(dlmm::bin_array_address(&lb().pair, index), dlmm_id(), 71_437_000, d);
}

fn book() -> pool::DeskBook {
    pool::with(|s| s.desks.get(&(COLL as u32)).cloned().unwrap_or_default())
}

#[test]
fn nothing_is_signed_before_the_desk_can_fund_an_opening() {
    let mut w = world(ACTIVE, SOL / 2);
    let log = w.pool_tick();
    assert_eq!(log, ["pool 0: waits: under the opening float (500000000 native, 0 wSOL, 0 wX)"]);
    assert!(book().pair.is_none());
    // the float short by a hair: still nothing; at the float: the pair opens
    let mut w = world(ACTIVE, 2 * plan::OPEN_MIN_SIDE_LAMPORTS + plan::OPEN_RENT_LAMPORTS + plan::NATIVE_FLOAT_LAMPORTS + plan::KEY_RENT - 1);
    assert!(!w.pool_tick().iter().any(|l| l.contains("signed")));
    let mut w = world(ACTIVE, 2 * plan::OPEN_MIN_SIDE_LAMPORTS + plan::OPEN_RENT_LAMPORTS + plan::NATIVE_FLOAT_LAMPORTS + plan::KEY_RENT);
    let log = w.pool_tick();
    assert!(log.iter().any(|l| l.contains("open the pair at bin 586") && l.contains("signed")), "{log:?}");
}

#[test]
fn the_opening_sequence_pair_arrays_position_mint_leg_add() {
    let mut w = world(ACTIVE, 3 * SOL);
    let mut acc = vec![];
    // 1. the pair, at the floor's bin
    step(&mut w, &mut acc, "open the pair at bin 586", |w| plant_pair(w, "lb_pair_open", ACTIVE));
    assert_eq!(book().pair, Some(lb().pair));
    // 2. the bin array of the band [567, 605]
    step(&mut w, &mut acc, "bin arrays [8]", |w| plant_array(w, "bin_array_8"));
    // 3. the position
    step(&mut w, &mut acc, "the position [567, 605]", |w| {
        plant_position(w, "position_after_add", true);
    });
    let b = book();
    assert_eq!((b.lo, b.hi), (567, 605));
    assert_eq!(b.position, Some(dlmm::position_address(&lb().pair, &desk(), 567, 39)));
    // 4. the wX side: a public mint, a link through the books, paid from the desk's SOL into the module
    let module_before = w.lamports(&module());
    step(&mut w, &mut acc, "mint", |_| {});
    let minted = w.supply();
    assert!(minted > 0);
    assert_eq!(w.token(&desk()), minted, "the wX are the desk's");
    assert!(w.lamports(&module()) > module_before, "the module holds the mint's SOL");
    let backed = state::with(|s| s.books.get(COLL).unwrap().ledger.sol_backed);
    assert_eq!(backed, minted, "booked like any holder's mint");
    // 5. the add: the wX held and the SOL wrapped, over the band inside the position
    step(&mut w, &mut acc, "add", |_| {});
    let add = landed(&mut w).into_iter().find_map(|s| match s {
        PoolStep::Add { ax, ay, lo, hi, .. } => Some((ax, ay, lo, hi)),
        _ => None,
    });
    let (ax, ay, lo, hi) = add.unwrap_or_else(|| panic!("{acc:?}"));
    assert!(ax > 0 && ax <= minted, "{ax} of {minted}");
    assert!(ay > 0 && ay < 3 * SOL / 2);
    assert_eq!((lo, hi), (567, 605));
    assert_eq!(w.token(&desk()), minted, "the fake keeps the wX where they are: the add is DLMM's");
    assert!(w.lamports(&desk()) >= plan::NATIVE_FLOAT_LAMPORTS, "the native float stays");
}

#[test]
fn an_up_arb_mints_at_the_module_and_sells_into_the_pool_in_one_transaction() {
    // the floor 6 bins (≈ 3 %) under the pool's price: the pool pays more than the mint costs
    let mut w = world(ACTIVE - 6, 30 * SOL);
    plant_pair(&mut w, "lb_pair_after_add", ACTIVE);
    plant_array(&mut w, "bin_array_8");
    plant_position(&mut w, "position_after_add", false);
    let supply_before = w.supply();
    let mut acc = vec![];
    step(&mut w, &mut acc, "up-arb", |_| {});
    let arb = landed(&mut w).into_iter().find_map(|s| match s {
        PoolStep::ArbUp { units, lamports, min_out, profit, .. } => Some((units, lamports, min_out, profit)),
        _ => None,
    });
    let (units, lamports, min_out, profit) = arb.unwrap_or_else(|| panic!("{acc:?}"));
    assert!(profit >= plan::ARB_MIN_PROFIT, "{profit}");
    assert!(min_out > lamports, "the sale brings back more than the mint cost ({min_out} > {lamports})");
    assert_eq!(w.supply() - supply_before, units, "the mint leg landed");
    assert_eq!(w.lamports(&module()), RENT0 + lamports, "the module holds the mint's SOL");
    let b = book();
    assert_eq!(b.arbs.len(), 1);
    assert_eq!(b.arb_profit, profit);
    // the size bounds: at most a quarter of the pool's SOL side, and never more than the hourly mint room
    assert!(units <= 5 * ONE);
    let (_, y_side) = plan::sides(&bins_of(&w), bin_price(ACTIVE));
    assert!(lamports <= y_side / 4 * 101 / 100, "{lamports} vs a quarter of {y_side}");
    // a second pass: the pool is unchanged in the fake, so the arb would repeat; the hourly count and the mint room bound it
    assert!(book().arbs.len() == 1);
}

fn bins_of(w: &World) -> BTreeMap<i32, dlmm::Bin> {
    let a = w.chain.borrow().accounts[&dlmm::bin_array_address(&lb().pair, 8)].clone();
    dlmm::bins_of(&[dlmm::BinArray::decode(&a.owner, &a.data).unwrap()])
}

#[test]
fn a_down_arb_buys_with_wsol_and_spare_inventory_is_burned_at_the_module() {
    // the floor 12 bins over the pool's price: the burn pays more than the pool asks
    let mut w = world(ACTIVE + 6, 2 * SOL);
    plant_pair(&mut w, "lb_pair_after_add", ACTIVE);
    plant_array(&mut w, "bin_array_8");
    plant_position(&mut w, "position_after_add", false);
    plant_wsol(&mut w, SOL);
    // the module has backing (a holder's mint of 50 SOL), so the inventory bound (10 % of it) and the burn have room
    let now = w.now as i64;
    let units = state::with_mut(|s| s.books.mint(COLL, 50 * SOL, 0, now).unwrap().units);
    w.inject_supply(units as i64);
    w.deposit_sol(&module(), 50 * SOL);
    let mut acc = vec![];
    step(&mut w, &mut acc, "down-arb", |_| {});
    let arb = landed(&mut w).into_iter().find_map(|s| match s {
        PoolStep::ArbDown { lamports, min_out, profit, .. } => Some((lamports, min_out, profit)),
        _ => None,
    });
    let (lamports, min_out, profit) = arb.unwrap_or_else(|| panic!("{acc:?}"));
    assert!(profit >= plan::ARB_MIN_PROFIT);
    assert!(lamports <= SOL, "{lamports}: at most the desk's wSOL");
    assert!(min_out > 0);
    assert_eq!(book().arbs.len(), 1);
    // the wX bought arrive (the fake does not run DLMM: planted out of the holder's supply), and the desk burns them at the module
    w.deposit_wx(&desk(), 2 * ONE);
    let supply_before = w.supply();
    let desk_before = w.lamports(&desk());
    acc.clear();
    step(&mut w, &mut acc, "burn", |_| {});
    let burn = landed(&mut w).into_iter().find_map(|s| match s {
        PoolStep::Burn { units, paid, .. } => Some((units, paid)),
        _ => None,
    });
    let (burned, paid) = burn.unwrap_or_else(|| panic!("{acc:?}"));
    assert_eq!(burned, 2 * ONE);
    assert_eq!(supply_before - w.supply(), 2 * ONE);
    assert_eq!(w.lamports(&desk()), desk_before + paid, "the module paid the desk");
    assert!(paid > 35 * SOL && paid < 40 * SOL, "≈ 2 × 19.2 SOL less the spread: {paid}");
    assert_eq!(w.token(&desk()), 0);
}

#[test]
fn a_floor_drift_inside_the_band_recentres_in_place_and_past_it_withdraws_then_moves() {
    // inside: the floor and the pool at 600 while the position is centred on 586 (14 bins: the 70 % trigger)
    let mut w = world(ACTIVE + 14, 2 * SOL);
    plant_pair(&mut w, "lb_pair_after_add", ACTIVE + 14);
    plant_array(&mut w, "bin_array_8");
    plant_position(&mut w, "position_after_add", false);
    let mut acc = vec![];
    step(&mut w, &mut acc, "re-centre [567, 605] around bin 600", |_| {});
    // past: the floor and the pool at 614, beyond the position's upper bin: the new band's array first, then the withdrawal
    let mut w = world(ACTIVE + 28, 3 * SOL);
    plant_pair(&mut w, "lb_pair_after_add", ACTIVE + 28);
    plant_array(&mut w, "bin_array_8");
    plant_position(&mut w, "position_after_add", false);
    let mut acc = vec![];
    step(&mut w, &mut acc, "bin arrays [9]", |w| plant_empty_array(w, 9));
    step(&mut w, &mut acc, "withdraw [567, 605]", |w| {
        plant_position(w, "position_after_add", true);
    });
    // withdrawn: the position is empty, the pool's price is at the floor already (no move), the wX side is minted for the new range
    step(&mut w, &mut acc, "mint", |_| {});
    assert!(w.token(&desk()) > 0);
    // then the rebalance moves the position's range to the band around 614
    step(&mut w, &mut acc, "re-centre [567, 605] around bin 614", |_| {});
    // the pool's price away from the floor on an empty position: moved first
    let mut w = world(ACTIVE + 28, 3 * SOL);
    plant_pair(&mut w, "lb_pair_after_add", ACTIVE);
    plant_array(&mut w, "bin_array_8");
    plant_empty_array(&mut w, 9);
    plant_position(&mut w, "position_after_add", true);
    let mut acc = vec![];
    step(&mut w, &mut acc, "move the empty pool to bin 614", |w| plant_pair(w, "lb_pair_after_add", ACTIVE + 28));
}

#[test]
fn fees_are_claimed_once_worth_a_transaction_and_a_failed_step_waits() {
    let mut w = world(ACTIVE, SOL + SOL / 10);
    plant_pair(&mut w, "lb_pair_after_add", ACTIVE);
    plant_array(&mut w, "bin_array_before_claim_8");
    let position = plant_position(&mut w, "position_before_claim", false);
    let a = w.chain.borrow().accounts[&position].clone();
    let pos = dlmm::PositionV2::decode(&a.owner, &a.data).unwrap();
    let (_, fee_y) = dlmm::claimable_fees(&pos, &bins_of_name(&w, 8)).unwrap();
    if fee_y >= plan::CLAIM_MIN_LAMPORTS {
        step(&mut w, &mut vec![], "claim the fees of [567, 605]", |_| {});
    } else {
        let log = w.pool_tick();
        assert!(!log.iter().any(|l| l.contains("claim")), "{fee_y} lamports of fees are under the bound: {log:?}");
    }
    // a step that fails on chain is filed as a wait and not retried until its time
    let mut w = world(ACTIVE, 3 * SOL);
    w.chain.borrow_mut().exec = Some(Box::new(|_, _, _, _| Err(serde_json::json!({ "InstructionError": [2, "Custom"] }))));
    let mut log = vec![];
    for _ in 0..4 {
        log.extend(w.pool_tick());
        w.settle();
    }
    assert!(log.iter().any(|l| l.contains("FAILED on chain")), "{log:?}");
    assert!(pool::with(|s| s.waits.iter().any(|x| x.key == "open_pair:0")));
    assert!(w.pool_tick().is_empty(), "nothing new while the wait holds");
    w.advance_time(pool::WAIT_RETRY_SECS + 1);
    let log = w.pool_tick();
    assert!(log.iter().any(|l| l.contains("open the pair") && l.contains("signed")), "{log:?}");
}

fn bins_of_name(w: &World, index: i64) -> BTreeMap<i32, dlmm::Bin> {
    let a = w.chain.borrow().accounts[&dlmm::bin_array_address(&lb().pair, index)].clone();
    dlmm::bins_of(&[dlmm::BinArray::decode(&a.owner, &a.data).unwrap()])
}

#[test]
fn the_pure_rules_of_the_desk() {
    let s = dlmm::DESK_BIN_STEP;
    let h = plan::half(s);
    assert_eq!(h, 19, "±10 % at bin step 50 is 19 bins a side (S4)");
    assert_eq!(plan::trigger(s), 14);
    // re-centre: nothing within the trigger; past the band withdraws; inside waits for the arb unless the pool is at the floor
    assert_eq!(plan::recenter(s, 567, 605, 586, 590), plan::Recenter::No);
    assert_eq!(plan::recenter(s, 567, 605, 600, 600), plan::Recenter::Inside { active: 600 });
    assert_eq!(plan::recenter(s, 567, 605, 586, 600), plan::Recenter::WaitArb);
    assert_eq!(plan::recenter(s, 567, 605, 614, 614), plan::Recenter::Past { target: 614 });
    assert_eq!(plan::recenter(s, 567, 605, 560, 560), plan::Recenter::Past { target: 560 });
    // the opening float and the target
    let floor = bin_price(ACTIVE);
    // audit B3-08: the opening's rent, the native float and the key's rent come first; then 0.5 SOL a side
    let fixed = plan::OPEN_RENT_LAMPORTS + plan::NATIVE_FLOAT_LAMPORTS + plan::KEY_RENT;
    assert!(!plan::can_open(2 * SOL, 0, 0, floor));
    assert!(!plan::can_open(fixed + SOL - 1, 0, 0, floor));
    assert!(plan::can_open(fixed + SOL, 0, 0, floor));
    assert!(plan::can_open(fixed + SOL / 2, 0, ONE, floor), "wX at the floor count toward the wX side");
    assert!(!plan::can_open(fixed + SOL / 2 - 1, 0, 10 * ONE, floor), "but the SOL side, the rent and the float need SOL");
    assert!(plan::under_target(0, 0, floor));
    assert!(!plan::under_target(ONE, 10 * SOL, floor), "≈ 18.6 SOL of wX and 10 SOL");
    assert!(plan::under_target(ONE / 2, 10 * SOL, floor));
    assert_eq!(plan::excess_for_treasury(5 * SOL, 2 * SOL, ONE, 10 * SOL, floor), 6 * SOL - plan::KEY_RENT);
    assert_eq!(plan::excess_for_treasury(5 * SOL, 2 * SOL, 0, 10 * SOL, floor), 0, "under target: nothing leaves");
    // a spot deposit funds every bin the same, the ask side's wX at 1 % over
    let (x0, y0, x_needed) = plan::spot_amounts(s, ACTIVE, h, SOL, SOL, 0, 10 * ONE);
    assert_eq!(x0, y0);
    assert!(y0 * (h as u64 + 1) <= SOL);
    assert!(x_needed > 0 && x_needed < ONE / 10, "{x_needed}: ≈ 1 SOL of wX at 18.6");
    let (_, _, capped) = plan::spot_amounts(s, ACTIVE, h, SOL, SOL, 0, ONE / 100);
    assert!(capped <= ONE / 100 * 101 / 100 + 1, "no more wX than the desk holds plus the mint's room");
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
fn the_pool_desks_bytes_are_the_monitors_fixture() {
    let mut w = world(ACTIVE, 3 * SOL);
    plant_pair(&mut w, "lb_pair_after_add", ACTIVE);
    plant_array(&mut w, "bin_array_8");
    plant_position(&mut w, "position_after_add", false);
    w.pool_tick();
    pool::with_mut(|s| {
        let d = s.desks.get_mut(&0).unwrap();
        d.read_at = 1_700_000_000;
        d.arbs = vec![1_700_000_000];
        d.arb_profit = 1_234_567;
        d.off = false;
    });
    let v = etch_autobuyer::pooldesk::api::desks(1_700_000_600);
    view_fixture("pool-desks.hex", &candid::encode_one(&v).unwrap());
}
