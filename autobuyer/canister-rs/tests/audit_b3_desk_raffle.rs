//! Audit B3 (docs/v9/audit-2026-10/b3-desk-raffle-surface.md): proofs of concept over the pool desk's decisions (`pooldesk::plan`,
//! `pooldesk::run::decide`) against the landed DLMM fixtures, and over the guardian surface. Every `poc_b3_NN_…` PASSES by exhibiting
//! the finding it is named for (the comment above each says what a fix must refuse), so the suite stays green while the findings stand.
//! No source file is changed by this audit.
mod desk_common;
mod fakerpc;

use std::collections::BTreeMap;

use base64::Engine;
use desk_common::*;
use etch_autobuyer::books::ONE;
use etch_autobuyer::cranks;
use etch_autobuyer::desk::state;
use etch_autobuyer::pooldesk::build::Addrs;
use etch_autobuyer::pooldesk::state as pool;
use etch_autobuyer::pooldesk::step::PoolStep;
use etch_autobuyer::pooldesk::{plan, run};
use etch_autobuyer::solix2::dlmm::{self, Bin, BinArray, Lb, LbPair, PositionV2};
use etch_autobuyer::solix2::{self, native_mint};
use etch_autobuyer::solkey::KeyPath;
use etch_autobuyer::solrpc::Account;
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

fn addrs() -> Addrs {
    Addrs { main: me(), desk: desk(), module: module(), treasury: pk(KeyPath::Treasury), lb: lb() }
}

/// The pair of a fixture as the desk's own (its mints patched, the active bin as given), decoded.
fn pair_of(name: &str, active: i32) -> LbPair {
    let mut p = LbPair::decode(&dlmm_id(), &fixture(name)).unwrap();
    p.token_x_mint = WX;
    p.token_y_mint = native_mint();
    p.active_id = active;
    p
}

fn position_of(name: &str) -> PositionV2 {
    PositionV2::decode(&dlmm_id(), &fixture(name)).unwrap()
}

/// The bins of a bin-array fixture, their amounts scaled by `k` (the fee accumulators zeroed: the position's fees are not the point).
fn bins_of(name: &str, k: u64) -> BTreeMap<i32, Bin> {
    let a = BinArray::decode(&dlmm_id(), &fixture(name)).unwrap();
    dlmm::bins_of(&[a])
        .into_iter()
        .map(|(id, mut b)| {
            b.amount_x *= k;
            b.amount_y *= k;
            b.fee_x_per_token_stored = 0;
            b.fee_y_per_token_stored = 0;
            (id, b)
        })
        .collect()
}

/// The prices the desk's `decide` computes from the books (its private `prices`, rebuilt here for the pure calls).
fn prices(now: u64) -> plan::Prices {
    state::with(|s| {
        let c = s.books.get(COLL).unwrap();
        let accepted = c.module.accepted;
        let now_i = now as i64;
        let mint = s.books.quote_mint(COLL, accepted, now_i).ok().filter(|u| *u > 0).map(|u| ((accepted as u128 * ONE as u128).div_ceil(u as u128)) as u64);
        let burn = s.books.quote_burn(COLL, ONE, now_i).ok();
        plan::Prices { floor: accepted, fresh: c.module.fresh(now_i), mint, burn, backing: c.module.backing }
    })
}

fn src(rel: &str) -> String {
    std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src").join(rel)).unwrap()
}

// ---- B3-01 ------------------------------------------------------------------------------------------------------------------------

/// B3-01 (High): the guardian's `pause_collection(i)` (§10.4: stop-only, lifted only by `resume`, 24 h later) and the operator's
/// `desk_pause_collection(i, paused)` write the SAME books flag. The operator lifts a guardian's pause (or an incident halt) at once, with
/// no resume scheduled. A fix must make `desk_pause_collection(i, false)` schedule a delayed resume (or refuse), so that the only path
/// from paused to open is `cranks::apply` after `RESUME_DELAY_SECS`.
/// FIXED B3-01: the operator's `desk_pause_collection(i, false)` and the guardian's resume share `cranks::collection_pause`: a lift is a
/// resume scheduled `RESUME_DELAY_SECS` ahead, never at once; a pause clears any scheduled resume.
#[test]
fn fixed_b3_01_the_operator_cannot_lift_a_collection_pause_at_once() {
    let w = World::new();
    cranks::reset();
    let now = w.now as i64;
    cranks::collection_pause(COLL, true, w.now).unwrap();
    assert!(state::with(|s| s.books.quote_mint(COLL, SOL, now)).is_err(), "the public mint is closed by the pause");
    let msg = cranks::collection_pause(COLL, false, w.now).unwrap();
    assert!(msg.contains(&format!("resumes at {}", w.now + cranks::RESUME_DELAY_SECS)), "{msg}");
    assert!(state::with(|s| s.books.quote_mint(COLL, SOL, now)).is_err(), "still closed: the lift is a day away");
    assert_eq!(cranks::with(|s| s.resumes.clone()), vec![(cranks::Resume::Collection, COLL as u32, w.now + cranks::RESUME_DELAY_SECS)]);
    // paused again before the day is over: the scheduled resume is gone
    cranks::collection_pause(COLL, true, w.now + 10).unwrap();
    assert!(cranks::with(|s| s.resumes.is_empty()));
    // an unknown collection is refused
    assert!(cranks::collection_pause(77, false, w.now).is_err());
    // the operator's method is the shared path
    let api = src("desk/api.rs");
    let body: String = api.split("pub fn desk_pause_collection").nth(1).expect("the method exists").split("\n}\n").next().unwrap().into();
    assert!(body.contains("cranks::collection_pause(coll, paused") && !body.contains("set_paused"), "{body}");
}

// ---- B3-02 ------------------------------------------------------------------------------------------------------------------------

/// B3-02 (Medium): the up-arb's `min_out` is the quote less 0.3 %, with no floor at the mint leg's cost. Whenever 0.3 % of the proceeds
/// exceeds the expected profit (any arb over ≈ 0.4 SOL of proceeds at a thin margin), a sandwich that moves the pool by under 0.3 %
/// lands the swap at a LOSS: the desk mints at `lamports` and may sell for `min_out < lamports`. A fix must refuse any
/// `min_out < lamports + ARB_MIN_PROFIT` (clamp `min_out` up, or shrink the size until the clamp holds).
/// FIXED (B3-02): `plan::up_min_out` clamps the up-arb's `min_out` to at least `lamports + ARB_MIN_PROFIT` (the quote clears it, the
/// profit floor being met), and `run::decide` keeps the clamp after scaling `min_out` to the units actually minted.
#[test]
fn fixed_b3_02_the_up_arbs_min_out_is_never_under_its_mint_cost() {
    let pair = pair_of("lb_pair_after_add", ACTIVE);
    // the pool's bids twenty times the fixture's (≈ 10 SOL in the active bin): a size where 0.3 % is more than the profit floor
    let bins = bins_of("bin_array_8", 20);
    let now: i64 = 10_000; // the fixture's references are at 0: the variable fee has decayed, the rate is the base 0.4 %
    let price = bin_price(ACTIVE);
    let f = pair.current_fee_rate(now);
    assert_eq!(f, 4_000_000, "the base fee");
    // the mint price 0.1 % under the up-arb's threshold price × (1 − f): a thin but real margin
    let threshold = (price as u128 * (dlmm::FEE_PRECISION - f) / dlmm::FEE_PRECISION) as u64;
    let m = (threshold as u128 * 9_990 / 10_000) as u64;
    let p = plan::Prices { floor: price, fresh: true, mint: Some(m), burn: None, backing: 1_000 * SOL };
    let room = plan::ArbRoom { native: 100 * SOL, wsol: 0, mint_units: 100 * ONE, inventory_units: 0, arbs_last_hour: 0 };
    let arb = plan::arb(&pair, &bins, &p, &room, now, &|_| None).expect("an up-arb is planned");
    let plan::Arb::Up { units, lamports, min_out, quote, profit } = arb else { panic!("{arb:?}") };
    assert!(profit >= plan::ARB_MIN_PROFIT, "{profit}");
    let slipped = quote.amount_out - quote.amount_out * plan::SLIPPAGE_BPS / 10_000;
    assert!(slipped < lamports, "the scenario still is the finding's: 0.3 % of {} is over the margin ({units} units)", quote.amount_out);
    // the fix: the least the swap may return covers the mint leg and the profit floor, and is within the quote
    assert_eq!(min_out, lamports + plan::ARB_MIN_PROFIT, "the clamp holds (units {units}, profit {profit})");
    assert!(min_out <= quote.amount_out, "{min_out} is within the quote {}", quote.amount_out);
    // the rule alone: the slippage bound when it is the higher, the cost floor otherwise
    assert_eq!(plan::up_min_out(10 * SOL, SOL), 10 * SOL - 10 * SOL * plan::SLIPPAGE_BPS / 10_000);
    assert_eq!(plan::up_min_out(10 * SOL, 10 * SOL - 1), 10 * SOL - 1 + plan::ARB_MIN_PROFIT);
    // and decide keeps the clamp after scaling to the minted units
    let run = src("pooldesk/run.rs");
    assert!(run.contains("/ units as u128) as u64).max(lamports.saturating_add(plan::ARB_MIN_PROFIT))"), "the scaled min_out is clamped");
}

// ---- B3-03 ------------------------------------------------------------------------------------------------------------------------

/// B3-03 (Medium): the desk has no wSOL → native path. Pool fees (`claim_fee2` pays Y = wSOL), up-arb proceeds and withdrawn liquidity
/// arrive as wSOL; the mint legs and the review-F3 sweep spend native only. At target depth the desk's wSOL is counted as the
/// treasury's excess and yet nothing is swept, no fee income is taken (`wants_fees` is false), and the native float is all the desk
/// has for its next mint leg. A fix must add an unwrap step (close and recreate the wSOL account, or `transfer` + `close_account`
/// of a temporary account) before the sweep and the mint legs, or sweep wSOL to a treasury wSOL account that the treasury unwraps.
/// FIXED (B3-03): `PoolStep::Unwrap` (`build::unwrap_tx`: `close_account` of the desk's wSOL account to `K_desk[i]`, all of it native),
/// planned before a sweep the native does not cover and before a mint leg (an add's or an up-arb's) short of native; the builders that
/// pay into the wSOL account (claim, withdraw, re-centre) re-create it first. The mint-leg case is shown in `fixed_b3_04_…`.
#[test]
fn fixed_b3_03_the_desks_wsol_is_unwrapped_and_reaches_the_treasury() {
    let w = World::with_floor(bin_price(ACTIVE));
    let now = w.now;
    let position = position_of("position_after_add");
    let bins = bins_of("bin_array_8", 20);
    let (pos_x, pos_y) = dlmm::position_amounts(&position, &bins).unwrap();
    let floor = bin_price(ACTIVE);
    assert!(!plan::under_target(pos_x, pos_y, floor), "the position is at target: {pos_x} wX, {pos_y} lamports");
    let a = addrs();
    pool::with_mut(|s| {
        let d = s.desks.entry(COLL as u32).or_default();
        d.pair = Some(a.lb.pair);
        d.position = Some(a.position(position.lower, position.width()));
        d.lo = position.lower;
        d.hi = position.upper;
        d.pos_x = pos_x;
        d.pos_y = pos_y;
    });
    let native = plan::KEY_RENT + plan::NATIVE_FLOAT_LAMPORTS + 1_000;
    let wsol = 30 * SOL;
    let r = run::Reading { native, wsol, wx: 0, pair: Some(pair_of("lb_pair_after_add", ACTIVE)), position: Some(position.clone()), bins: bins.clone(), arrays: vec![8] };
    let excess = plan::excess_for_treasury(native, wsol, pos_x, pos_y, floor);
    assert!(excess >= wsol, "the plan counts the wSOL as the treasury's: {excess}");
    // 1. the treasury's share is wSOL: the desk unwraps it first
    let p = run::decide(COLL as u32, &a, &r, now).unwrap().expect("a step");
    assert_eq!(p.step, PoolStep::Unwrap { coll: COLL as u32, lamports: wsol });
    assert_eq!(p.ixs, etch_autobuyer::pooldesk::build::unwrap_tx(&a));
    let ix = &p.ixs[0];
    assert_eq!(ix.program, solix2::token_program());
    assert_eq!(ix.data, vec![9], "SPL CloseAccount");
    assert_eq!(ix.accounts.iter().map(|m| (m.key, m.signer)).collect::<Vec<_>>(), vec![(a.desk_y(), false), (desk(), false), (desk(), true)]);
    // on the chain: the wSOL account closes into the desk's native SOL, balance and rent
    let mut acc = BTreeMap::new();
    acc.insert(desk(), Account { lamports: native, owner: soltx::SYSTEM_PROGRAM, executable: false, data: vec![] });
    acc.insert(a.desk_y(), Account { lamports: RENT165 + wsol, owner: solix2::token_program(), executable: false, data: token_data(native_mint(), desk(), wsol) });
    run_ixs(&p.ixs, &mut acc).unwrap();
    assert!(!acc.contains_key(&a.desk_y()), "the wSOL account is closed");
    let native2 = acc[&desk()].lamports;
    assert_eq!(native2, native + wsol + RENT165);
    // 2. the next pass sweeps it, natively, to the treasury
    let r2 = run::Reading { native: native2, wsol: 0, ..r.clone() };
    let p2 = run::decide(COLL as u32, &a, &r2, now).unwrap().expect("a step");
    let want = native2 - plan::KEY_RENT - plan::NATIVE_FLOAT_LAMPORTS;
    assert_eq!(p2.step, PoolStep::Sweep { coll: COLL as u32, lamports: want });
    assert!(want > wsol);
    // the builders that pay into the wSOL account re-create it first (fixture-independent)
    use etch_autobuyer::pooldesk::build;
    let ata = solix2::spl::create_ata_idempotent(a.main, desk(), native_mint());
    let pk = a.position(position.lower, position.width());
    assert_eq!(build::claim_tx(&a, pk, position.lower, position.upper)[0], ata);
    assert!(build::withdraw_tx(&a, pk, position.lower, position.upper).contains(&ata));
    assert!(build::arb_up_tx(&a, ONE, SOL, SOL, ACTIVE, 1, "m").contains(&ata));
    assert!(build::add_tx(&a, pk, 0, SOL, SOL, ACTIVE, 567, 605).contains(&ata));
    // nothing unwraps under the claim bound (a dust account is not worth a transaction)
    let r3 = run::Reading { wsol: plan::CLAIM_MIN_LAMPORTS - 1, ..r.clone() };
    let p3 = run::decide(COLL as u32, &a, &r3, now).unwrap();
    assert!(!p3.as_ref().is_some_and(|p| matches!(p.step, PoolStep::Unwrap { .. })), "{:?}", p3.map(|p| p.step));
}

// ---- B3-04 ------------------------------------------------------------------------------------------------------------------------

/// B3-04 (Medium): after a past-the-band withdrawal (S4 R2) the desk's only move is `go_to_a_bin`, which DLMM refuses while any
/// liquidity lies between the pool's price and the floor's bin. With a stranger's liquidity there, the move fails every 30 minutes
/// for ever: on an empty position `decide` returns before the arb, so the desk neither trades the pool back to the floor (the arb
/// that is profitable against that liquidity) nor deposits. A fix must let an empty position's desk arb (and then rebalance around
/// the active bin it reaches) instead of insisting on the move.
/// FIXED (B3-04): on an empty position the move is tried while it is ready and refused fewer than `run::MOVE_TRIES` times; otherwise
/// `decide` goes on to the arb and, once the pool is within the re-centre trigger of the floor, deposits around the active bin (a
/// landed withdrawal resets the count).
#[test]
fn fixed_b3_04_a_withdrawn_desk_arbs_a_stranger_pool_and_gives_up_the_move() {
    // the floor 28 bins over the pool's price (past the band), the desk's position withdrawn (its shares zero), a 50 SOL backing
    let w = World::with_floor(bin_price(ACTIVE + 28));
    let now = w.now;
    state::with_mut(|s| s.books.mint(COLL, 50 * SOL, 0, now as i64).unwrap());
    let mut position = position_of("position_after_add");
    position.liquidity_shares = vec![0; position.liquidity_shares.len()];
    let bins = bins_of("bin_array_8", 1); // the asks 587…605 hold wX: someone else's liquidity between the pool's price and the floor
    assert!(bins.get(&(ACTIVE + 1)).is_some_and(|b| b.amount_x > 0));
    let a = addrs();
    pool::with_mut(|s| {
        let d = s.desks.entry(COLL as u32).or_default();
        d.pair = Some(a.lb.pair);
        d.position = Some(a.position(position.lower, position.width()));
        d.lo = position.lower;
        d.hi = position.upper;
    });
    let pair = pair_of("lb_pair_after_add", ACTIVE);
    let r = run::Reading { native: 2 * SOL, wsol: 5 * SOL, wx: 0, pair: Some(pair.clone()), position: Some(position.clone()), bins: bins.clone(), arrays: vec![8, 9] };
    // 1. the first idea is still the move
    let step = run::decide(COLL as u32, &a, &r, now).unwrap().expect("a step").step;
    assert_eq!(step, PoolStep::Move { coll: COLL as u32, target: ACTIVE + 28 });
    // 2. DLMM refuses it (liquidity between): while it waits, the desk arbs through that liquidity toward the floor
    let refuse = |at: u64| pool::with_mut(|s| s.wait(step.key(), "refused by simulation: liquidity between the active bin and the target".into(), at));
    refuse(now);
    let p = prices(now);
    assert!(p.burn.is_some_and(|b| b > bin_price(ACTIVE + 1)), "the burn pays more than the first ask: {:?} vs {}", p.burn, bin_price(ACTIVE + 1));
    let next = run::decide(COLL as u32, &a, &r, now).unwrap().expect("a step").step;
    assert!(matches!(next, PoolStep::ArbDown { .. }), "{next:?}");
    // 3. refused MOVE_TRIES times: after the wait the move is given up, the arb goes on
    let mut t = now;
    for _ in 1..run::MOVE_TRIES {
        t += pool::WAIT_RETRY_SECS + 1;
        refuse(t);
    }
    let later = t + pool::WAIT_RETRY_SECS + 1;
    let step3 = run::decide(COLL as u32, &a, &r, later).unwrap().expect("a step").step;
    assert!(matches!(step3, PoolStep::ArbDown { .. }), "the move is not tried a {}th time: {step3:?}", run::MOVE_TRIES + 1);
    // 4. the arb brought the pool within the trigger (5 bins under the floor), its hourly count spent: the desk re-opens around the
    //    active bin. Its wX side is a mint leg the native cannot pay: the wSOL comes back first (B3-03), then the mint
    pool::with_mut(|s| s.desks.get_mut(&(COLL as u32)).unwrap().arbs = vec![later; plan::ARBS_PER_HOUR]);
    let near = run::Reading { pair: Some(pair_of("lb_pair_after_add", ACTIVE + 23)), ..r.clone() };
    let s4 = run::decide(COLL as u32, &a, &near, later).unwrap().expect("a step").step;
    assert_eq!(s4, PoolStep::Unwrap { coll: COLL as u32, lamports: 5 * SOL });
    let unwrapped = run::Reading { native: 7 * SOL, wsol: 0, ..near.clone() };
    let s5 = run::decide(COLL as u32, &a, &unwrapped, later).unwrap().expect("a step").step;
    assert!(matches!(s5, PoolStep::Mint { .. }), "the deposit's wX side: {s5:?}");
    // the empty desk never deposits around a pool still past the trigger (a re-centre would withdraw it again)
    pool::with_mut(|s| s.receipts.clear());
    let far = run::Reading { native: 7 * SOL, wsol: 0, ..r.clone() };
    let s6 = run::decide(COLL as u32, &a, &far, later).unwrap();
    assert!(!s6.as_ref().is_some_and(|p| matches!(p.step, PoolStep::Mint { .. } | PoolStep::Add { .. } | PoolStep::Recenter { .. })), "{:?}", s6.map(|p| p.step));
}

// ---- B3-05 ------------------------------------------------------------------------------------------------------------------------

/// B3-05 (Medium): the pass reads at most four bin arrays, the nearest to the floor's; a position two or three arrays away from the
/// floor (a floor that doubled or halved since the last re-centre, say while the desk was paused) is dropped from the read. Then
/// (a) `position_amounts` fails and `decide` treats the position as EMPTY, and (b) step B wants every array from the position to the
/// floor's band that the read did not return, and signs `initialize_bin_array` for one that EXISTS: refused on chain, retried every
/// 30 minutes, ahead of every other step. A fix must always read the position's own arrays, treat an unreadable position as not
/// empty, and create only the arrays the add will touch.
/// FIXED (B3-05): `run::read` always asks for the position's arrays and the floor band's (the ±2 bands fill what is left); step B
/// creates only those of them the read did not return (never the stretch between); a position the read cannot value is refused
/// (`decide` errs, nothing signed) instead of being taken as empty.
#[test]
fn fixed_b3_05_a_far_floor_reads_the_positions_arrays_and_creates_only_the_bands() {
    // the position [587, 633] spans arrays 8 and 9; the floor at bin 805 has its band [786, 824] in array 11
    let far = 805;
    let mut w = World::with_floor(bin_price(far));
    w.plant(native_mint(), solix2::token_program(), 1_461_600, mint_data([0; 32], 0));
    w.deposit_sol(&desk(), 3 * SOL);
    let mut pd = fixture("lb_pair_rebalance_past");
    pd[88..120].copy_from_slice(&WX);
    pd[76..80].copy_from_slice(&614i32.to_le_bytes());
    w.plant(lb().pair, dlmm_id(), 7_182_720, pd);
    // bin arrays 8 and 9 exist on chain (the desk made them for the position)
    w.plant(dlmm::bin_array_address(&lb().pair, 8), dlmm_id(), 71_437_000, fixture("bin_array_8"));
    let empty_array = |index: i64| {
        let mut d = fixture("bin_array_8");
        d[8..16].copy_from_slice(&index.to_le_bytes());
        d[56..].fill(0);
        d
    };
    w.plant(dlmm::bin_array_address(&lb().pair, 9), dlmm_id(), 71_437_000, empty_array(9));
    let pos = position_of("position_rebalance_past");
    assert_eq!((pos.lower, pos.upper), (587, 633));
    assert!(!pos.is_empty(), "the position holds liquidity");
    let key = dlmm::position_address(&lb().pair, &desk(), pos.lower, pos.width());
    w.plant(key, dlmm_id(), 57_406_000, fixture("position_rebalance_past"));
    pool::with_mut(|s| {
        let d = s.desks.entry(COLL as u32).or_default();
        d.position = Some(key);
        d.lo = pos.lower;
        d.hi = pos.upper;
    });
    let a = addrs();
    // (a) the read: the position's arrays are read whatever the floor's distance, so the position is valued
    let r = block_on(run::read(&mut w, &a, COLL as u32)).unwrap();
    assert!(r.arrays.contains(&8) && r.arrays.contains(&9), "{:?}", r.arrays);
    let (x, y) = dlmm::position_amounts(r.position.as_ref().unwrap(), &r.bins).expect("the position's bins are read");
    assert!(x > 0 || y > 0, "not empty");
    // (b) step B creates the band's array only (never 8, which exists, nor the stretch 10 between)
    let step = run::decide(COLL as u32, &a, &r, w.now).unwrap().expect("a step").step;
    assert_eq!(step, PoolStep::OpenArrays { coll: COLL as u32, indexes: vec![11] });
    let log = w.pool_tick();
    assert!(log.iter().any(|l| l.contains("bin arrays [11]") && l.contains("signed")), "{log:?}");
    // (c) the band's array there: the floor move withdraws the live position (it is not taken as empty)
    w.plant(dlmm::bin_array_address(&lb().pair, 11), dlmm_id(), 71_437_000, empty_array(11));
    let r = block_on(run::read(&mut w, &a, COLL as u32)).unwrap();
    let step = run::decide(COLL as u32, &a, &r, w.now).unwrap().expect("a step").step;
    assert_eq!(step, PoolStep::Withdraw { coll: COLL as u32, lo: 587, hi: 633 });
    // (d) a position whose bins are missing from what was read is refused, never taken as empty nor re-created around
    let mut lacking = r.clone();
    lacking.bins.retain(|id, _| dlmm::array_index(*id) != 8);
    let e = run::decide(COLL as u32, &a, &lacking, w.now).err().expect("refused");
    assert!(e.contains("cannot be valued"), "{e}");
}
