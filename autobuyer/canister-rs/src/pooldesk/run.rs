//! The pool desk's tick (W7): settle what the pool line reported, then for each open collection ONE batched read of its desk (the key's
//! SOL, its wSOL and wX accounts, the pair, the position, the bin arrays around the floor) and at most ONE step signed, in this order:
//! the arrays and the position's range, a re-centre (§5.4, S4 R1/R2), an arb (§5.4), a burn of spare inventory, a top-up toward the
//! target depth (§5.3), a fee claim, a sweep to the treasury; before any of that, the opening of the pair once the desk can fund it.
//!
//! Never queued: a step signs only on a free pool line, and a refusal by simulation is the step's to wait out, not the line's error.
//! Every step that mints or burns is a [`chainlink`](crate::chainlink) link: booked first, gated by `pre_sign`, memo'd.
use std::collections::BTreeMap;

use crate::books::ONE;
use crate::chainlink::driver::{DeskIo, SignError};
use crate::chainlink::ledger::{Effects, OpKind, Phase, Scope, Spec};
use crate::coins::link;
use crate::desk::state as desk;
use crate::solhome::{LineId, LineOwner, LineStep};
use crate::solix2::decode::TokenAccount;
use crate::solix2::dlmm::{self, Bin, BinArray, Lb, LbPair, PositionV2, RebalanceAdd};
use crate::solkey::KeyPath;
use crate::soltx::Pubkey;

use super::build::{self, Addrs};
use super::plan::{self, Arb, ArbRoom, Prices, Recenter};
use super::state::*;
use super::step::{Planned, PoolStep};

/// Bin arrays read a pass per collection: the position's and the floor band's always (at most two each, audit B3-05), then the ones
/// nearest the floor within ±2 bands up to this many.
pub const ARRAYS_READ: usize = 4;
/// `go_to_a_bin` is refused while any liquidity lies between the pool's price and the target (anyone may add to the pair): after this
/// many refusals the desk stops trying the move and arbs the pool toward the floor instead (audit B3-04). Reset when a withdrawal lands.
pub const MOVE_TRIES: u32 = 3;
/// The least to add in one top-up (lamports, both sides together): a transaction's cost against the slippage it saves.
pub const ADD_MIN_LAMPORTS: u64 = 200_000_000;
/// The desk's wX above this share of its position's value is burned (§5.4 "rebalancing beyond arbitrage"), in bps.
pub const INVENTORY_BURN_BPS: u64 = 6_000;
/// The quote's drift allowed between the plan and the books' mint (bps).
pub const MINT_DRIFT_BPS: u64 = 10;

/// What the chain shows of one collection's desk.
#[derive(Clone, Debug, Default)]
pub struct Reading {
    pub native: u64,
    pub wsol: u64,
    pub wx: u64,
    pub pair: Option<LbPair>,
    pub position: Option<PositionV2>,
    pub bins: BTreeMap<i32, Bin>,
    /// The bin arrays that exist, by index.
    pub arrays: Vec<i64>,
}

/// One pass of the pool desk. `now` in unix seconds.
pub async fn tick<I: DeskIo>(io: &mut I, now: u64) -> Vec<String> {
    let mut log = vec![];
    if with(|s| s.off) {
        return log;
    }
    let mut colls: Vec<u32> = crate::coins::state::with(|s| {
        s.cols.values().filter(|c| c.stage == crate::coins::state::ColStage::Open).map(|c| c.index).collect()
    });
    colls.extend(with(|s| s.desks.keys().copied().collect::<Vec<_>>()));
    colls.sort_unstable();
    colls.dedup();
    if colls.is_empty() {
        return log;
    }
    with_mut(|s| s.roll_day(now));
    settle(io, now, &mut log);
    if io.sol_home().is_none() {
        return log;
    }
    if let Err(e) = crate::crankline::line_free(&io.sol_line_of(LineId::Pool)) {
        if with(|s| s.desks.values().any(|d| d.pair.is_some() || d.native > 0)) {
            log.push(format!("pool: waits: {e}"));
        }
        return log;
    }
    if with(|s| s.signed_today >= STEPS_PER_DAY) {
        log.push(format!("pool: waits: {STEPS_PER_DAY} steps today already (the day's budget)"));
        return log;
    }
    for coll in colls {
        if with(|s| s.desks.get(&coll).is_some_and(|d| d.off)) {
            continue;
        }
        let a = match addrs(io, coll).await {
            Ok(a) => a,
            Err(e) => {
                log.push(format!("pool {coll}: waits: {e}"));
                continue;
            }
        };
        let r = match read(io, &a, coll).await {
            Ok(r) => r,
            Err(e) => {
                log.push(format!("pool {coll}: waits: {e}"));
                continue;
            }
        };
        record(coll, &a, &r, now);
        match decide(coll, &a, &r, now) {
            Ok(Some(p)) => {
                log.push(sign(io, p, now).await);
                return log;
            }
            Ok(None) => {}
            Err(e) => {
                // quiet while the desk has nothing: a collection without fees yet is the normal case
                if r.pair.is_some() || r.native > plan::KEY_RENT {
                    log.push(format!("pool {coll}: waits: {e}"));
                }
            }
        }
    }
    log
}

/// The desk's addresses: the keys, and the pair of the collection's wX.
pub async fn addrs<I: DeskIo>(io: &mut I, coll: u32) -> Result<Addrs, String> {
    let wx = desk::with(|s| s.books.get(coll as u16).map(|c| c.wx_mint)).map_err(|e| format!("{e}"))?;
    Ok(Addrs {
        main: io.key_of(&KeyPath::Main).await?,
        desk: io.key_of(&KeyPath::Desk(coll)).await?,
        module: io.key_of(&KeyPath::Coll(coll)).await?,
        treasury: io.key_of(&KeyPath::Treasury).await?,
        lb: Lb::desk(wx),
    })
}

fn floor_of(coll: u32) -> u64 {
    desk::with(|s| s.books.get(coll as u16).map(|c| c.module.accepted).unwrap_or(0))
}

/// The bin arrays of the floor's band `[fb − h, fb + h]` (one or two): the ones the next add touches.
fn band_arrays(bin_step: u16, fb: i32) -> std::ops::RangeInclusive<i64> {
    let h = plan::half(bin_step);
    dlmm::array_index(fb - h)..=dlmm::array_index(fb + h)
}

/// The arrays step B may create: the position's own and the floor band's. The read always asks for every one of them, so one the read
/// did not return is one the chain does not have (audit B3-05).
fn arrays_needed(bin_step: u16, fb: i32, pos: Option<(i32, i32)>) -> Vec<i64> {
    let mut idx: Vec<i64> = band_arrays(bin_step, fb).collect();
    if let Some((lo, hi)) = pos {
        idx.extend(dlmm::array_index(lo)..=dlmm::array_index(hi));
    }
    idx.sort_unstable();
    idx.dedup();
    idx
}

/// The bin arrays worth reading: the position's and the floor band's always, then the floor's ±2 bands, nearest first.
fn arrays_wanted(pair: &Pubkey, bin_step: u16, floor: u64, pos: Option<(i32, i32)>) -> Vec<(i64, Pubkey)> {
    // audit B3-05: a cap over all of them dropped the position's arrays once the floor was two or three arrays away; the position then
    // read as empty and step B re-created an array that exists. The position's and the band's are never dropped
    let fb = plan::floor_bin(bin_step, floor);
    let mut idx: Vec<i64> = match fb {
        Some(fb) => arrays_needed(bin_step, fb, pos),
        None => pos.map(|(lo, hi)| (dlmm::array_index(lo)..=dlmm::array_index(hi)).collect()).unwrap_or_default(),
    };
    if let Some(fb) = fb {
        let h = plan::half(bin_step);
        let centre = dlmm::array_index(fb);
        let mut more: Vec<i64> = (dlmm::array_index(fb - 2 * h)..=dlmm::array_index(fb + 2 * h)).filter(|i| !idx.contains(i)).collect();
        more.sort_by_key(|i| (i - centre).abs());
        let room = ARRAYS_READ.saturating_sub(idx.len());
        idx.extend(more.into_iter().take(room));
    }
    idx.sort_unstable();
    idx.into_iter().map(|i| (i, dlmm::bin_array_address(pair, i))).collect()
}

/// ONE batched read of the desk.
pub async fn read<I: DeskIo>(io: &mut I, a: &Addrs, coll: u32) -> Result<Reading, String> {
    let (pos, bin_step) = with(|s| s.desks.get(&coll).map(|d| (d.position.map(|p| (p, d.lo, d.hi)), dlmm::DESK_BIN_STEP)).unwrap_or((None, dlmm::DESK_BIN_STEP)));
    let floor = floor_of(coll);
    let mut keys = vec![a.desk, a.desk_y(), a.desk_x(), a.lb.pair];
    if let Some((p, ..)) = pos {
        keys.push(p);
    }
    let arrays = arrays_wanted(&a.lb.pair, bin_step, floor, pos.map(|(_, lo, hi)| (lo, hi)));
    keys.extend(arrays.iter().map(|(_, k)| *k));
    let max = dlmm::LB_PAIR_LEN + dlmm::POSITION_V2_LEN + arrays.len() * dlmm::BIN_ARRAY_LEN + 4 * 165;
    let accs = io.sol_accounts(&keys, max).await?;
    let amount = |i: usize| accs.get(i).and_then(|x| x.as_ref()).and_then(|x| TokenAccount::decode(&x.owner, &x.data).ok()).map(|t| t.amount).unwrap_or(0);
    let mut r = Reading { native: accs[0].as_ref().map(|x| x.lamports).unwrap_or(0), wsol: amount(1), wx: amount(2), ..Default::default() };
    r.pair = accs[3].as_ref().and_then(|x| LbPair::decode(&x.owner, &x.data).ok());
    let mut n = 4;
    if pos.is_some() {
        r.position = accs[4].as_ref().and_then(|x| PositionV2::decode(&x.owner, &x.data).ok());
        n = 5;
    }
    let mut decoded = vec![];
    for ((i, _), acc) in arrays.iter().zip(&accs[n..]) {
        if let Some(b) = acc.as_ref().and_then(|x| BinArray::decode(&x.owner, &x.data).ok()) {
            r.arrays.push(*i);
            decoded.push(b);
        }
    }
    r.bins = dlmm::bins_of(&decoded);
    Ok(r)
}

/// What the reading says, into the desk's book.
pub fn record(coll: u32, a: &Addrs, r: &Reading, now: u64) {
    with_mut(|s| {
        let d = s.desks.entry(coll).or_default();
        d.native = r.native;
        d.wsol = r.wsol;
        d.wx = r.wx;
        d.read_at = now;
        if let Some(p) = &r.pair {
            d.pair = Some(a.lb.pair);
            d.active = p.active_id;
        }
        if let Some(p) = &r.position {
            d.lo = p.lower;
            d.hi = p.upper;
            if let Ok((x, y)) = dlmm::position_amounts(p, &r.bins) {
                d.pos_x = x;
                d.pos_y = y;
            }
        }
    });
}

/// The books' prices for the arb and the adds: the mint's and the burn's price of one whole wX, and the room to mint (units).
fn prices(coll: u32, now: u64) -> (Prices, u64) {
    desk::with(|s| {
        let now_i = now as i64;
        let Ok(c) = s.books.get(coll as u16) else { return (Prices { floor: 0, fresh: false, mint: None, burn: None, backing: 0 }, 0) };
        let accepted = c.module.accepted;
        let mint = s.books.quote_mint(coll as u16, accepted, now_i).ok().filter(|u| *u > 0).map(|u| ((accepted as u128 * ONE as u128).div_ceil(u as u128)) as u64);
        let burn = s.books.quote_burn(coll as u16, ONE, now_i).ok();
        let room = c.module.mint_room(now_i).unwrap_or(0);
        (Prices { floor: accepted, fresh: c.module.fresh(now_i), mint, burn, backing: c.module.backing }, room)
    })
}

fn pending_key(coll: u32, now: u64, step: &PoolStep) -> bool {
    with(|s| s.waiting(&step.key(), now) || s.receipts.iter().any(|r| r.step.coll() == coll))
}

/// Begins a link of this desk (idempotent by a fresh key), returning its id and memo.
fn begin(coll: u32, kind: OpKind, now: u64, f: impl FnOnce(&mut crate::books::Books) -> Result<(Effects, u64), crate::books::BooksError>) -> Result<(u64, String, u64), String> {
    let op = with_mut(|s| {
        s.ops += 1;
        s.ops
    });
    let spec = Spec { key: format!("pool:{}:{coll}:{op}", kind.tag()), kind, coll: Some(coll as u16), intent: None, scope: Scope::Module };
    let (id, _) = desk::with_mut(|s| s.link.begin(&mut s.books, spec, now, f)).map_err(|e| match e {
        crate::chainlink::ledger::LinkError::Books(b) => format!("the books refused: {b}"),
        crate::chainlink::ledger::LinkError::Busy(c) => format!("collection {c} has an unfinished operation"),
        crate::chainlink::ledger::LinkError::Halted(c) => format!("collection {c} is stopped by an incident"),
    })?;
    let (memo, aux) = desk::with(|s| s.link.get(id).map(|l| (l.memo.clone(), l.aux)).unwrap_or_default());
    Ok((id, memo, aux))
}

/// The one step the desk makes now for `coll`, or none. `Err`: why nothing can be done (quiet unless the desk is live).
pub fn decide(coll: u32, a: &Addrs, r: &Reading, now: u64) -> Result<Option<Planned>, String> {
    let (p, mint_room) = prices(coll, now);
    let floor = p.floor;
    if floor == 0 {
        return Err("no accepted floor".into());
    }
    let bin_step = r.pair.as_ref().map(|x| x.bin_step).unwrap_or(dlmm::DESK_BIN_STEP);
    let fb = plan::floor_bin(bin_step, floor).ok_or("the floor is outside the pair's bins")?;
    let h = plan::half(bin_step);
    let ready = |step: PoolStep| -> bool { !pending_key(coll, now, &step) };
    // A. no pair: open it once the desk can fund the opening
    let Some(pair) = &r.pair else {
        if !plan::can_open(r.native, r.wsol, r.wx, floor) {
            return Err(format!("under the opening float ({} native, {} wSOL, {} wX)", r.native, r.wsol, r.wx));
        }
        let step = PoolStep::OpenPair { coll, active: fb, pair: a.lb.pair };
        return Ok(ready(step.clone()).then(|| Planned { ixs: build::open_pair_tx(a, fb), keys: build::main_only(), link: None, step }));
    };
    if pair.token_x_mint != a.lb.x || pair.token_y_mint != a.lb.y {
        return Err("the pair at the desk's address is not wX/wSOL".into());
    }
    let active = pair.active_id;
    // B. the bin arrays of the position's range and the floor's band (rent, before any add touches them). Audit B3-05: only those, which
    // the read always asks for, never the stretch between (the read may skip it: an array missing there is unread, not absent)
    let want: Vec<i64> =
        arrays_needed(bin_step, fb, r.position.as_ref().map(|p| (p.lower, p.upper))).into_iter().filter(|i| !r.arrays.contains(i)).take(2).collect();
    if !want.is_empty() {
        let step = PoolStep::OpenArrays { coll, indexes: want.clone() };
        return Ok(ready(step.clone()).then(|| Planned { ixs: build::open_arrays_tx(a, &want), keys: build::main_only(), link: None, step }));
    }
    // C. the position
    let Some(pos) = &r.position else {
        let (lower, width) = (fb - h, 2 * h + 1);
        let step = PoolStep::OpenPosition { coll, lower, width, position: a.position(lower, width) };
        return Ok(ready(step.clone()).then(|| Planned { ixs: build::open_position_tx(a, lower, width), keys: build::desk_keys(coll), link: None, step }));
    };
    let position = a.position(pos.lower, pos.width());
    // audit B3-05: a position whose bins the read lacks is not empty: nothing is decided on it (the next pass reads again)
    let (pos_x, pos_y) = dlmm::position_amounts(pos, &r.bins).map_err(|e| format!("the position cannot be valued, nothing decided: {e}"))?;
    let empty = pos_x == 0 && pos_y == 0;
    let budget = (r.native + r.wsol).saturating_sub(plan::KEY_RENT + plan::NATIVE_FLOAT_LAMPORTS);
    // audit B3-03: the wSOL back to native (all of it: `close_account`), when a mint leg or the sweep needs native the desk holds as wSOL
    let unwrap = || -> Option<Planned> {
        let step = PoolStep::Unwrap { coll, lamports: r.wsol };
        (r.wsol >= plan::CLAIM_MIN_LAMPORTS && ready(step.clone())).then(|| Planned { ixs: build::unwrap_tx(a), keys: build::desk_keys(coll), link: None, step })
    };
    if empty {
        // D. an empty position: the pool's price to the floor first (after a withdrawal: S4 R2), then the deposit around it (H). Audit
        // B3-04: DLMM refuses the move while anyone's liquidity lies between; while it waits, and for good after MOVE_TRIES refusals,
        // the desk goes on to the arb (which trades through that liquidity toward the floor) and deposits once the pool is near it
        if (active - fb).abs() > plan::AT_FLOOR_BINS {
            let arr = |b: i32| r.arrays.contains(&dlmm::array_index(b)).then(|| dlmm::bin_array_address(&a.lb.pair, dlmm::array_index(b)));
            let step = PoolStep::Move { coll, target: fb };
            let tries = with(|s| s.waits.iter().find(|w| w.key == step.key()).map(|w| w.tries).unwrap_or(0));
            if tries < MOVE_TRIES && ready(step.clone()) {
                return Ok(Some(Planned { ixs: build::move_tx(a, arr(active), arr(fb), fb), keys: build::main_only(), link: None, step }));
            }
        }
    } else {
        // E. a live position: re-centre
        match plan::recenter(bin_step, pos.lower, pos.upper, active, fb) {
            Recenter::Past { .. } => {
                let step = PoolStep::Withdraw { coll, lo: pos.lower, hi: pos.upper };
                return Ok(ready(step.clone())
                    .then(|| Planned { ixs: build::withdraw_tx(a, position, pos.lower, pos.upper), keys: build::desk_keys(coll), link: None, step }));
            }
            Recenter::Inside { active } => {
                let price = plan::bin_floor(bin_step, active);
                let x_value = (pos_x as u128 * price as u128 / ONE as u128) as u64;
                let (x0, y0, _) = plan::spot_amounts(bin_step, active, h, x_value, pos_y, pos_x, 0);
                let adds = [RebalanceAdd { min_delta: -h, max_delta: h, x0, y0 }];
                let step = PoolStep::Recenter { coll, active, lo: pos.lower, hi: pos.upper };
                // the wSOL account the withdrawn SOL side passes through (audit B3-03: an unwrap may have closed it)
                let mut ixs = vec![crate::solix2::spl::create_ata_idempotent(a.main, a.desk, a.lb.y)];
                ixs.extend(build::recenter_tx(a, position, active, Some((pos.lower, pos.upper)), &adds, active - h, active + h));
                return Ok(ready(step.clone()).then(|| Planned { ixs, keys: build::desk_keys(coll), link: None, step }));
            }
            Recenter::WaitArb | Recenter::No => {}
        }
    }
    // F. the arb
    let arbs_last_hour = with(|s| s.desks.get(&coll).map(|d| d.arbs.iter().filter(|t| **t + 3_600 > now).count()).unwrap_or(0));
    // the inventory bound (§5.4): wX worth at most 10 % of the collection's backing, at the floor, less what the desk holds
    let inventory_cap = (p.backing as u128 * plan::INVENTORY_BACKING_BPS as u128 / 10_000 * ONE as u128 / floor as u128) as u64;
    let room = ArbRoom { native: r.native.saturating_sub(plan::KEY_RENT), wsol: r.wsol, mint_units: mint_room, inventory_units: inventory_cap.saturating_sub(r.wx), arbs_last_hour };
    let burn_of = |units: u64| desk::with(|s| s.books.quote_burn(coll as u16, units, now as i64).ok());
    if let Some(arb) = plan::arb(pair, &r.bins, &p, &room, now as i64, &burn_of) {
        // audit B3-03: the up-arb's mint leg is paid natively and its proceeds arrive as wSOL; with the native float spent, the wSOL
        // comes back first
        if matches!(arb, Arb::Up { .. }) && r.native < plan::KEY_RENT + plan::NATIVE_FLOAT_LAMPORTS {
            if let Some(u) = unwrap() {
                return Ok(Some(u));
            }
        }
        match arb {
            Arb::Up { units, lamports, min_out, quote, profit } => {
                let step0 = PoolStep::ArbUp { coll, units, lamports, min_out, link: 0, profit };
                if ready(step0.clone()) {
                    let min_units = units - units * MINT_DRIFT_BPS / 10_000;
                    match begin(coll, OpKind::Mint, now, |b| {
                        let m = b.mint(coll as u16, lamports, min_units, now as i64)?;
                        Ok((Effects { mint_units: m.units, lamports_in: lamports, ..Default::default() }, m.units))
                    }) {
                        Ok((link, memo, minted)) => {
                            // audit B3-02: scaled to the units minted, and still never under what the mint leg paid plus the profit floor
                            let min_out = ((min_out as u128 * minted as u128 / units as u128) as u64).max(lamports.saturating_add(plan::ARB_MIN_PROFIT));
                            let arrays = (dlmm::array_index(active) - dlmm::array_index(quote.end_id) + 1).max(1) as usize;
                            let step = PoolStep::ArbUp { coll, units: minted, lamports, min_out, link, profit };
                            with_mut(|s| s.receipts.push(Receipt { link, step: step.clone(), at: now }));
                            return Ok(Some(Planned { ixs: build::arb_up_tx(a, minted, lamports, min_out, active, arrays, &memo), keys: build::module_keys(coll), link: Some(link), step }));
                        }
                        // review F8: a books refusal is this step's to wait out; the later steps still run
                        Err(e) => with_mut(|s| s.wait(step0.key(), e, now)),
                    }
                }
            }
            Arb::Down { lamports, min_out, quote, profit } => {
                let step = PoolStep::ArbDown { coll, lamports, min_out, profit };
                if ready(step.clone()) {
                    let arrays = (dlmm::array_index(quote.end_id) - dlmm::array_index(active) + 1).max(1) as usize;
                    return Ok(Some(Planned { ixs: build::arb_down_tx(a, lamports, min_out, active, arrays), keys: build::desk_keys(coll), link: None, step }));
                }
            }
        }
    }
    // G. spare inventory burned at the module (a down-arb's second step; wX above its share of the position)
    let under = plan::under_target(pos_x, pos_y, floor);
    let value = plan::value(r.native, r.wsol, r.wx, pos_x, pos_y, floor);
    let wx_value = r.wx as u128 * floor as u128 / ONE as u128;
    // audit B3-09: the excess over the inventory share is burned (all of it once the target is met), within the hourly burn bucket's room
    let allowed = if under { (value * INVENTORY_BURN_BPS as u128 / 10_000 * ONE as u128 / floor.max(1) as u128).min(u64::MAX as u128) as u64 } else { 0 };
    let room = desk::with(|s| s.books.get(coll as u16).ok().and_then(|c| c.module.burn_room(now as i64).ok())).unwrap_or(0);
    let excess = r.wx.saturating_sub(allowed).min(room);
    if excess >= crate::desk::intent::MIN_BURN_UNITS && (!under || wx_value * 10_000 > value * INVENTORY_BURN_BPS as u128) && p.burn.is_some() {
        let step0 = PoolStep::Burn { coll, units: excess, paid: 0, link: 0 };
        if ready(step0.clone()) {
            let units = excess;
            match begin(coll, OpKind::Burn, now, |b| {
                let x = b.burn(coll as u16, units, 0, now as i64)?;
                Ok((Effects { burn_units: x.units, lamports_out: x.paid, ..Default::default() }, x.paid))
            }) {
                Ok((link, memo, paid)) => {
                    let step = PoolStep::Burn { coll, units, paid, link };
                    with_mut(|s| s.receipts.push(Receipt { link, step: step.clone(), at: now }));
                    return Ok(Some(Planned { ixs: build::burn_tx(a, units, paid, &memo), keys: build::module_keys(coll), link: Some(link), step }));
                }
                Err(e) => with_mut(|s| s.wait(step0.key(), e, now)),
            }
        }
    }
    // H. an empty position's deposit around the active bin once the pool is within the re-centre trigger of the floor (a re-centre would
    // not withdraw it again: audit B3-04), or a top-up toward the target depth around the active bin inside the position
    if empty && (active - fb).abs() < plan::trigger(bin_step) {
        if budget < ADD_MIN_LAMPORTS {
            return Err(format!("too little to deposit ({budget} lamports free)"));
        }
        return plan_add(coll, a, r, position, active, pos, budget, mint_room, &p, now);
    }
    if !empty && under && budget >= ADD_MIN_LAMPORTS && active > pos.lower && active < pos.upper {
        if let Some(x) = plan_add(coll, a, r, position, active, pos, budget, mint_room, &p, now)? {
            return Ok(Some(x));
        }
    }
    // I. the fees
    if let Ok((_, fee_y)) = dlmm::claimable_fees(pos, &r.bins) {
        if fee_y >= plan::CLAIM_MIN_LAMPORTS {
            let step = PoolStep::Claim { coll, lo: pos.lower, hi: pos.upper };
            if ready(step.clone()) {
                return Ok(Some(Planned { ixs: build::claim_tx(a, position, pos.lower, pos.upper), keys: build::desk_keys(coll), link: None, step }));
            }
        }
    }
    // J. the treasury's share
    // review F3: the treasury owes native SOL; the sweep pays native. Audit B3-03: the excess counts the wSOL (fees, arb proceeds and
    // withdrawals arrive so), which is unwrapped first when the native above the float does not cover it
    let excess_all = plan::excess_for_treasury(r.native, r.wsol, pos_x, pos_y, floor);
    let native_free = r.native.saturating_sub(plan::KEY_RENT + plan::NATIVE_FLOAT_LAMPORTS);
    if excess_all >= plan::CLAIM_MIN_LAMPORTS && native_free < excess_all {
        if let Some(u) = unwrap() {
            return Ok(Some(u));
        }
    }
    let excess = excess_all.min(native_free);
    if excess >= plan::CLAIM_MIN_LAMPORTS {
        let step = PoolStep::Sweep { coll, lamports: excess };
        if ready(step.clone()) {
            return Ok(Some(Planned { ixs: build::sweep_tx(a, excess), keys: build::desk_keys(coll), link: None, step }));
        }
    }
    Ok(None)
}

/// A deposit of `budget` lamports of value around `active`: the wX side minted first when the desk holds too few (a link); then the add,
/// inside the position when its range covers the band, else a `rebalance_liquidity` that moves the position's range to the band (S4 R2).
#[allow(clippy::too_many_arguments)]
fn plan_add(coll: u32, a: &Addrs, r: &Reading, position: Pubkey, active: i32, pos: &PositionV2, budget: u64, mint_room: u64, p: &Prices, now: u64)
    -> Result<Option<Planned>, String> {
    let bin_step = r.pair.as_ref().map(|x| x.bin_step).unwrap_or(dlmm::DESK_BIN_STEP);
    let h = plan::half(bin_step);
    let room = if p.fresh && p.mint.is_some() { mint_room } else { 0 };
    let (x0, y0, x_needed) = plan::spot_amounts(bin_step, active, h, budget / 2, budget / 2, r.wx, room);
    if y0 == 0 {
        return Err("nothing to deposit".into());
    }
    // a shortfall under 1 % is not worth a mint: the add takes what the desk holds
    if x_needed > r.wx && (x_needed - r.wx) * 100 > x_needed {
        let units = x_needed - r.wx;
        let Some(m) = p.mint else { return Err("the mint is closed: the wX side cannot be funded".into()) };
        let cost = (units as u128 * m as u128).div_ceil(ONE as u128) as u64;
        // audit B3-03: a mint leg short of native takes the desk's wSOL back first (the add re-wraps what its SOL side needs)
        if cost > r.native.saturating_sub(plan::KEY_RENT) && r.wsol >= plan::CLAIM_MIN_LAMPORTS {
            let step = PoolStep::Unwrap { coll, lamports: r.wsol };
            if pending_key(coll, now, &step) {
                return Ok(None);
            }
            return Ok(Some(Planned { ixs: build::unwrap_tx(a), keys: build::desk_keys(coll), link: None, step }));
        }
        let lamports = cost.min(r.native.saturating_sub(plan::KEY_RENT));
        if lamports == 0 {
            return Err("no native SOL for the mint leg".into());
        }
        let step0 = PoolStep::Mint { coll, units, lamports, link: 0 };
        if !pending_key(coll, now, &step0) {
            let min_units = (lamports as u128 * ONE as u128 / m as u128) as u64;
            let min_units = min_units - min_units * MINT_DRIFT_BPS / 10_000;
            match begin(coll, OpKind::Mint, now, |b| {
                let x = b.mint(coll as u16, lamports, min_units, now as i64)?;
                Ok((Effects { mint_units: x.units, lamports_in: lamports, ..Default::default() }, x.units))
            }) {
                Ok((link, memo, minted)) => {
                    let step = PoolStep::Mint { coll, units: minted, lamports, link };
                    with_mut(|s| s.receipts.push(Receipt { link, step: step.clone(), at: now }));
                    return Ok(Some(Planned { ixs: build::mint_tx(a, minted, lamports, &memo), keys: build::module_keys(coll), link: Some(link), step }));
                }
                Err(e) => with_mut(|s| s.wait(step0.key(), e, now)),
            }
        }
        return Ok(None);
    }
    let ax = x_needed.min(r.wx);
    let ay = y0 * (h as u64 + 1);
    let wrap = ay.saturating_sub(r.wsol);
    let (lo, hi) = (active - h, active + h);
    if pos.lower <= lo && pos.upper >= hi {
        let step = PoolStep::Add { coll, ax, ay, lo, hi };
        if pending_key(coll, now, &step) {
            return Ok(None);
        }
        return Ok(Some(Planned { ixs: build::add_tx(a, position, ax, ay, wrap, active, lo, hi), keys: build::desk_keys(coll), link: None, step }));
    }
    // the position's range is elsewhere (the floor left it and the position was withdrawn): the rebalance moves it, with the wSOL wrapped first
    let adds = [RebalanceAdd { min_delta: -h, max_delta: h, x0, y0 }];
    let step = PoolStep::Recenter { coll, active, lo: pos.lower, hi: pos.upper };
    if pending_key(coll, now, &step) {
        return Ok(None);
    }
    let mut ixs = vec![crate::solix2::spl::create_ata_idempotent(a.main, a.desk, a.lb.y)];
    if wrap > 0 {
        ixs.push(crate::soltx::transfer(a.desk, a.desk_y(), wrap));
        ixs.push(crate::solix2::spl::sync_native(a.desk_y()));
    }
    ixs.extend(build::recenter_tx(a, position, active, None, &adds, lo, hi));
    Ok(Some(Planned { ixs, keys: build::desk_keys(coll), link: None, step }))
}

/// Signs a planned step on the pool line: a linked one through the gate, a plain one by its instructions alone.
async fn sign<I: DeskIo>(io: &mut I, p: Planned, now: u64) -> String {
    let label = p.step.label();
    let key = p.step.key();
    let before = io.sol_line_of(LineId::Pool).line.error.clone();
    let step = LineStep::Pool(p.step.clone());
    let r = match p.link {
        Some(id) => link::sign_linked(io, LineOwner::Pool, step, id, p.ixs, p.keys, now).await.map_err(|e| match e {
            SignError::Refused(b) => format!("REFUSED by pre_sign: {b}"),
            SignError::Deferred(s) => s,
        }),
        None => link::sign_plain(io, LineOwner::Pool, step, p.ixs, p.keys, now).await,
    };
    match r {
        Ok(sig) => {
            with_mut(|s| {
                s.signed_today += 1;
                s.clear_wait(&key);
            });
            format!("pool: {label} signed and sent ({sig})")
        }
        Err(e) => {
            if let Some(id) = p.link {
                with_mut(|s| s.receipts.retain(|r| r.link != id));
            }
            if e.contains(crate::solsend::SIMULATION_REFUSED) && !e.contains("insufficient_funds") {
                let mut l = io.sol_line_of(LineId::Pool);
                l.line.error = before;
                io.sol_line_save_of(LineId::Pool, &l);
                with_mut(|s| s.wait(key, e.clone(), now));
                format!("pool: {label} is refused by simulation, nothing signed: {e}")
            } else {
                with_mut(|s| s.wait(key, e.clone(), now));
                format!("pool: {label} waits: {e}")
            }
        }
    }
}

/// What the pool line reported of the desk's steps, and what the links decided.
pub fn settle<I: DeskIo>(io: &mut I, now: u64, log: &mut Vec<String>) {
    let mut l = io.sol_line_of(LineId::Pool);
    let mine = l.take(LineOwner::Pool);
    if !mine.is_empty() {
        io.sol_line_save_of(LineId::Pool, &l);
    }
    for o in mine {
        let LineStep::Pool(ps) = &o.step else { continue };
        match o.ok {
            Some(true) => {
                if let Some(id) = ps.link() {
                    desk::with_mut(|s| s.link.landed(&mut s.books, id, &o.signature, now));
                }
                apply_landed(ps, now);
                with_mut(|s| {
                    s.clear_wait(&ps.key());
                    s.landed(ps, now);
                });
                log.push(format!("pool: {} landed ({})", ps.label(), o.signature));
            }
            Some(false) => {
                if let Some(id) = ps.link() {
                    desk::with_mut(|s| {
                        let crate::chainlink::driver::DeskStateRef { books, link } = crate::chainlink::driver::DeskStateRef::of(s);
                        link.undo(books, id, Phase::Failed, &format!("landed and failed on chain ({})", o.signature), now);
                    });
                }
                with_mut(|s| s.wait(ps.key(), format!("FAILED on chain ({})", o.signature), now));
                log.push(format!("pool: {} FAILED on chain ({}): tried again in {} min", ps.label(), o.signature, WAIT_RETRY_SECS / 60));
            }
            None => {
                if let Some(id) = ps.link() {
                    desk::with_mut(|s| {
                        let crate::chainlink::driver::DeskStateRef { books, link } = crate::chainlink::driver::DeskStateRef::of(s);
                        link.undo(books, id, Phase::Undone, "proven never to land (its nonce burned or its blockhash expired)", now);
                    });
                }
                log.push(format!("pool: {} never landed: decided again at the next pass", ps.label()));
            }
        }
    }
    // receipts of links the ledger decided on its own (the desk's recovery): dropped once decided
    with_mut(|s| {
        s.receipts.retain(|r| matches!(link::phase_of(r.link), Some(Phase::Recorded) | Some(Phase::Signed) | Some(Phase::Stuck)));
    });
}

/// A landed step, into the desk's book (the holdings and the position are re-read at the next pass).
fn apply_landed(ps: &PoolStep, now: u64) {
    with_mut(|s| {
        s.receipts.retain(|r| r.step.link() != ps.link() || ps.link().is_none());
        let d = s.desks.entry(ps.coll()).or_default();
        match ps {
            PoolStep::OpenPair { pair, active, .. } => {
                d.pair = Some(*pair);
                d.active = *active;
            }
            PoolStep::OpenPosition { lower, width, position, .. } => {
                d.position = Some(*position);
                d.lo = *lower;
                d.hi = lower + width - 1;
            }
            PoolStep::ArbUp { profit, .. } | PoolStep::ArbDown { profit, .. } => {
                d.arbs.retain(|t| *t + 3_600 > now);
                d.arbs.push(now);
                d.arb_profit = d.arb_profit.saturating_add(*profit);
            }
            PoolStep::Sweep { lamports, .. } => {
                d.swept = d.swept.saturating_add(*lamports);
            }
            _ => {}
        }
    });
    // audit B3-04: a landed withdrawal starts a new attempt at the move (its refusals counted afresh)
    if let PoolStep::Withdraw { coll, .. } = ps {
        with_mut(|s| s.clear_wait(&PoolStep::Move { coll: *coll, target: 0 }.key()));
    }
    if let PoolStep::Sweep { lamports, .. } = ps {
        desk::with_mut(|s| {
            let _ = s.books.receive(*lamports);
        });
    }
}
