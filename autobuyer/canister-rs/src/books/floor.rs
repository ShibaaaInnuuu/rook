//! The floor module (rook_core `state/floor.rs`, FloorModule.sol), one per collection, in lamports and 9-decimal units. Every
//! function is pure over the module plus what the program read elsewhere: `now` (unix seconds), `e` = the wrapper ledger's
//! SOL-backed units (rook_core `eth_backed`), `kd` = the poster switch (the global keeper pause or this collection's poster pause).
//!
//! Ported line for line. Changes (docs/v9/canister-only-solana.md §4.1, §4.3):
//! - `convert` is dropped (no SOL-quoted graduation: a graduated coin's backing is already wX); `split`, `intake` and
//!   `reserve_units` stay;
//! - `public_cap` honours [`Params::mint_depth_cap`] (§4.3);
//! - the account plumbing (`listing`, `wrapper`, `vault`, `module_wx`, bumps) is gone; the books live in the canister and the
//!   lamports in the collection's module address `M[i]`;
//! - every mutating method is all-or-nothing: on an error the module is left exactly as it was (a Solana transaction reverted
//!   on any `require!`; in the canister the same guarantee is [`Module::atomic`]).

use candid::CandidType;
use serde::{Deserialize, Serialize};

use super::math::{add, elapsed, mul_div, mul_div_ceil, sub, Res, BPS, DAY, HOUR, MINUTE, ONE};
use super::params::Params;
use super::BooksError as E;

/// `FLOOR_TTL`: a post up to 3 h old prices.
pub const FLOOR_TTL: i64 = 3 * HOUR;
pub const STALE_GRACE: i64 = 10 * MINUTE;
pub const SNAPSHOT_TTL: i64 = 2 * HOUR;
pub const FILL_WINDOW: i64 = 24 * HOUR;
pub const FILLS: usize = 8;
pub const DAYS: usize = 7;
pub const MAX_DAY_MOVE_BPS: u64 = 2_000;
pub const DAY_WINDOW: i64 = DAY;
pub const STALE_ESCAPE: i64 = 7 * DAY;
pub const MINT_LOW_BUCKET: i64 = 12 * HOUR;
/// The H11 ring: `max_items_per_day` may be at most this.
pub const MAX_ITEMS_PER_DAY: usize = 32;

fn ensure(ok: bool, e: E) -> Res<()> {
    if ok {
        Ok(())
    } else {
        Err(e)
    }
}

#[derive(CandidType, Serialize, Deserialize, Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct Fill {
    /// Lamports per whole unit: a purchase's cost net of the conversion allowance.
    pub price: u64,
    pub at: i64,
}

#[derive(CandidType, Serialize, Deserialize, Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct MintLow {
    pub price: u64,
    pub bucket: i64,
}

/// One collection's floor module. The lamports are at the collection's module address `M[i]`; the books below are claims on
/// them: `balance(M[i]) − rent ≥ backing + reserve + surplus + revenue` ([`super::books_balance`]).
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Module {
    pub params: Params,
    // ---- the floor ----
    pub accepted: u64,
    pub target: u64,
    pub posted_at: i64,
    pub move_acc: u64,
    pub confidence_bps: u16,
    pub depth_items: u32,
    pub day_anchor: u64,
    pub day_anchor_at: i64,
    pub last_good_at: i64,
    // ---- hourly caps (leaky buckets, units) ----
    pub mint_level: u64,
    pub mint_at: i64,
    pub burn_level: u64,
    pub burn_at: i64,
    // ---- the books ----
    pub backing: u64,
    pub reserve: u64,
    pub surplus: u64,
    pub reserve_units: u64,
    pub revenue: u64,
    // ---- conversions ----
    pub items_arrived: u64,
    pub arrivals: u64,
    pub arrived_at: [i64; MAX_ITEMS_PER_DAY],
    pub snap_cap: u64,
    pub snap_at: i64,
    // ---- the advance ----
    pub transit_units: u64,
    pub advanced: u64,
    pub advance_basis: u64,
    pub advanced_at: i64,
    // ---- fills and volume ----
    pub fills: [Fill; FILLS],
    pub fill_count: u64,
    pub day_volume: [u64; DAYS],
    pub day_of: [u64; DAYS],
    pub mint_low: [MintLow; 2],
}

impl Module {
    /// A new module (rook_core `init_module`): the parameters are validated and fixed for good.
    pub fn new(params: Params) -> Res<Module> {
        super::params::validate_params(&params)?;
        Ok(Module {
            params,
            accepted: 0,
            target: 0,
            posted_at: 0,
            move_acc: 0,
            confidence_bps: 0,
            depth_items: 0,
            day_anchor: 0,
            day_anchor_at: 0,
            last_good_at: 0,
            mint_level: 0,
            mint_at: 0,
            burn_level: 0,
            burn_at: 0,
            backing: 0,
            reserve: 0,
            surplus: 0,
            reserve_units: 0,
            revenue: 0,
            items_arrived: 0,
            arrivals: 0,
            arrived_at: [0; MAX_ITEMS_PER_DAY],
            snap_cap: 0,
            snap_at: 0,
            transit_units: 0,
            advanced: 0,
            advance_basis: 0,
            advanced_at: 0,
            fills: [Fill::default(); FILLS],
            fill_count: 0,
            day_volume: [0; DAYS],
            day_of: [0; DAYS],
            mint_low: [MintLow::default(); 2],
        })
    }

    /// Runs `f` on a copy and keeps it only on success: a refused operation changes nothing.
    pub fn atomic<T>(&mut self, f: impl FnOnce(&mut Module) -> Res<T>) -> Res<T> {
        let mut m = self.clone();
        let r = f(&mut m)?;
        *self = m;
        Ok(r)
    }

    pub fn books(&self) -> u128 {
        self.backing as u128 + self.reserve as u128 + self.surplus as u128 + self.revenue as u128
    }

    // ---- freshness -----------------------------------------------------------------------------------------------------

    /// `fresh()`: a floor, a good confidence, and under `FLOOR_TTL` old.
    pub fn fresh(&self, now: i64) -> bool {
        self.accepted != 0 && self.confidence_bps >= self.params.min_confidence_bps && now < self.posted_at + FLOOR_TTL
    }

    /// `escaped()` (B-04): stale, and no good post for `FLOOR_TTL + STALE_ESCAPE`.
    pub fn escaped(&self, now: i64) -> bool {
        if self.fresh(now) || self.last_good_at == 0 {
            return false;
        }
        now >= self.last_good_at + FLOOR_TTL + STALE_ESCAPE
    }

    pub fn burns_open(&self, now: i64, kd: bool) -> bool {
        self.fresh(now) || kd || self.escaped(now)
    }

    // ---- the poster ----------------------------------------------------------------------------------------------------

    pub fn day_anchor_now(&self, now: i64) -> (u64, bool) {
        if self.day_anchor == 0 || now >= self.day_anchor_at + DAY_WINDOW {
            (self.accepted, true)
        } else {
            (self.day_anchor, false)
        }
    }

    /// `floorBounds()`.
    pub fn floor_bounds(&self, now: i64) -> Res<(u64, u64)> {
        let prev = self.accepted;
        if prev == 0 {
            return Ok((0, u64::MAX));
        }
        let el = elapsed(now, self.posted_at).min(HOUR) as u64;
        let step = mul_div(prev, self.params.max_move_bps as u64 * el, BPS * HOUR as u64)?;
        let (anchor, _) = self.day_anchor_now(now);
        let d_lo = mul_div_ceil(anchor, BPS - MAX_DAY_MOVE_BPS, BPS)?;
        let d_hi = mul_div(anchor, BPS + MAX_DAY_MOVE_BPS, BPS)?;
        let lo = (prev - step).max(d_lo.min(prev));
        let hi = prev.saturating_add(step).min(d_hi.max(prev));
        Ok((lo, hi))
    }

    /// `volBps()`: the trailing hour's movement, decaying linearly.
    pub fn vol_bps(&self, now: i64) -> Res<u64> {
        let el = elapsed(now, self.posted_at).min(HOUR) as u64;
        mul_div_ceil(self.move_acc, HOUR as u64 - el, HOUR as u64)
    }

    /// `postFloor`, after the poster check. Returns (accepted, moved bps).
    pub fn post_floor(&mut self, price: u64, confidence_bps: u16, depth_items: u32, now: i64) -> Res<(u64, u64)> {
        self.atomic(|m| m.post_floor_raw(price, confidence_bps, depth_items, now))
    }

    fn post_floor_raw(&mut self, price: u64, confidence_bps: u16, depth_items: u32, now: i64) -> Res<(u64, u64)> {
        ensure(price != 0 && (confidence_bps as u64) <= BPS, E::BadInput)?;
        let prev = self.accepted;
        let mut next = price;
        let mut moved = 0;
        if prev != 0 {
            let (lo, hi) = self.floor_bounds(now)?;
            next = next.clamp(lo, hi);
            let diff = if next > prev { next - prev } else { prev - next };
            moved = mul_div_ceil(diff, BPS, prev)?;
            self.move_acc = add(self.vol_bps(now)?, moved)?;
            let (_, reset) = self.day_anchor_now(now);
            if reset {
                self.day_anchor = prev;
                self.day_anchor_at = now;
            }
        } else {
            self.day_anchor = next;
            self.day_anchor_at = now;
        }
        if confidence_bps >= self.params.min_confidence_bps {
            self.last_good_at = now;
        }
        self.accepted = next;
        self.target = price;
        self.confidence_bps = confidence_bps;
        self.depth_items = depth_items;
        self.posted_at = now;
        Ok((next, moved))
    }

    // ---- prices --------------------------------------------------------------------------------------------------------

    /// `price()`: the TWAP of the module's own fills over 24 h, within `fill_band_bps` of the accepted floor.
    pub fn price(&self, now: i64) -> Res<u64> {
        let acc = self.accepted;
        let n = self.fill_count as usize;
        if acc == 0 || n == 0 {
            return Ok(acc);
        }
        let start = if now > FILL_WINDOW { now - FILL_WINDOW } else { 0 };
        let kept = n.min(FILLS);
        let newest = self.fills[(n - 1) % FILLS];
        if newest.at < start {
            return Ok(acc);
        }
        let mut weighted: u128 = 0;
        let mut span: u128 = 0;
        let mut prev_price = acc as u128;
        let mut prev_at = start;
        for i in 0..kept {
            let f = self.fills[(n - kept + i) % FILLS];
            if f.at <= start {
                continue;
            }
            // `elapsed` (floored at 0): a fill stamped before the previous one cannot panic (A1-04)
            let dt = elapsed(f.at, prev_at) as u128;
            weighted += prev_price * dt;
            span += dt;
            prev_price = f.price as u128;
            prev_at = f.at;
        }
        weighted += prev_price * elapsed(now, prev_at) as u128;
        span += elapsed(now, prev_at) as u128;
        let twap = if span == 0 { prev_price } else { weighted / span } as u64;
        let lo = mul_div(acc, BPS - self.params.fill_band_bps as u64, BPS)?;
        let hi = mul_div_ceil(acc, BPS + self.params.fill_band_bps as u64, BPS)?;
        Ok(twap.clamp(lo, hi))
    }

    /// `_upTarget()` (A-10).
    pub fn up_target(&self, now: i64) -> Res<u64> {
        let (anchor, _) = self.day_anchor_now(now);
        Ok(self.target.min(mul_div(anchor, BPS + MAX_DAY_MOVE_BPS, BPS)?.max(self.accepted)))
    }

    /// `_burnTarget()` (N-1a).
    pub fn burn_target(&self, now: i64) -> Res<u64> {
        let a = self.accepted;
        let (anchor, _) = self.day_anchor_now(now);
        let day_lo = mul_div_ceil(anchor, BPS - MAX_DAY_MOVE_BPS, BPS)?;
        let lo = mul_div_ceil(a, BPS - self.params.max_move_bps as u64, BPS)?.max(day_lo.min(a));
        Ok(self.target.max(lo))
    }

    pub fn mint_base(&self, now: i64) -> Res<u64> {
        Ok(self.price(now)?.max(self.up_target(now)?))
    }

    /// `gradPrice()`, kept as a quote (no conversion lane uses it any more).
    pub fn grad_price(&self, now: i64) -> Res<u64> {
        mul_div_ceil(self.mint_base(now)?, BPS + self.params.conv_bps as u64, BPS)
    }

    /// `_pauseFloor()` (A3-M1, A4-L1).
    pub fn pause_floor(&self, now: i64) -> Res<u64> {
        let mut f = self.burn_target(now)?.min(mul_div(self.accepted, BPS - self.params.fill_band_bps as u64, BPS)?);
        let b = (self.posted_at + FLOOR_TTL) / MINT_LOW_BUCKET;
        for v in self.mint_low.iter() {
            if v.price != 0 && (v.bucket == b || v.bucket + 1 == b) {
                f = f.min(v.price);
            }
        }
        Ok(f)
    }

    fn note_mint_base(&mut self, base: u64, now: i64) {
        let b = now / MINT_LOW_BUCKET;
        let i = (b % 2) as usize;
        let v = self.mint_low[i];
        if v.price == 0 || v.bucket != b || base < v.price {
            self.mint_low[i] = MintLow { price: base, bucket: b };
        }
    }

    // ---- units ---------------------------------------------------------------------------------------------------------

    /// `_liveUnits()`: SOL-backed units less the one in transit.
    pub fn live_units(&self, e: u64) -> u64 {
        e.saturating_sub(self.transit_units)
    }

    /// `netUnits()`: the canister's buy signal, `sol_backed − transit − reserve_units`.
    pub fn net_units(&self, e: u64) -> u64 {
        self.live_units(e).saturating_sub(self.reserve_units)
    }

    /// The poster's depth in units (§4.3: the buy pipeline's brake).
    pub fn depth_cap(&self) -> u64 {
        (self.depth_items as u64).saturating_mul(ONE)
    }

    pub fn public_cap(&self) -> u64 {
        if self.params.mint_depth_cap {
            self.params.max_public_eth_backed.min(self.depth_cap())
        } else {
            self.params.max_public_eth_backed
        }
    }

    /// `spreads()`: (mint, burn) bps.
    pub fn spreads(&self, now: i64, e: u64) -> Res<(u64, u64)> {
        let p = &self.params;
        let mut s = p.base_spread_bps as u64 + self.vol_bps(now)? * p.vol_weight_bps as u64 / BPS;
        if self.confidence_bps < p.full_confidence_bps {
            s += mul_div_ceil(
                p.low_confidence_spread_bps as u64,
                (p.full_confidence_bps - self.confidence_bps) as u64,
                p.full_confidence_bps as u64,
            )?;
        }
        let age = elapsed(now, self.posted_at);
        if age > STALE_GRACE {
            let x = (age - STALE_GRACE).min(FLOOR_TTL - STALE_GRACE) as u64;
            s += mul_div_ceil(p.stale_spread_bps as u64, x, (FLOOR_TTL - STALE_GRACE) as u64)?;
        }
        let pc = self.public_cap();
        let stress = if pc == 0 {
            p.stress_spread_bps as u64
        } else {
            mul_div_ceil(p.stress_spread_bps as u64, self.net_units(e).min(pc), pc)?
        };
        let max = p.max_spread_bps as u64;
        Ok(((s + stress).min(max), s.min(max)))
    }

    /// `_split()`: the reserve's share of an intake.
    pub fn split(&self, lamports: u64, units: u64) -> (u64, u64) {
        let r = self.params.reserve_bps as u64;
        let r_l = (lamports as u128 * r as u128 / BPS as u128) as u64;
        let r_u = (units as u128 * r as u128 / BPS as u128) as u64;
        if r_l == 0 || r_u == 0 {
            (0, 0)
        } else {
            (r_l, r_u)
        }
    }

    /// `_intake()`.
    fn intake(&mut self, lamports: u64, units: u64) -> Res<()> {
        let (r_l, r_u) = self.split(lamports, units);
        self.reserve = add(self.reserve, r_l)?;
        self.reserve_units = add(self.reserve_units, r_u)?;
        self.backing = add(self.backing, lamports - r_l)?;
        Ok(())
    }

    /// `_gradRoom()`: units still under `max_eth_backed`.
    pub fn grad_room(&self, e: u64) -> u64 {
        self.params.max_eth_backed.saturating_sub(e)
    }

    fn level(level: u64, at: i64, cap: u64, now: i64) -> Res<u64> {
        let leak = mul_div(cap, elapsed(now, at).min(HOUR) as u64, HOUR as u64)?;
        Ok(level.saturating_sub(leak))
    }

    /// The units the hourly mint bucket still takes now.
    pub fn mint_room(&self, now: i64) -> Res<u64> {
        Ok(self.params.mint_per_hour.saturating_sub(Self::level(self.mint_level, self.mint_at, self.params.mint_per_hour, now)?))
    }

    /// The units the hourly burn bucket still takes now.
    pub fn burn_room(&self, now: i64) -> Res<u64> {
        Ok(self.params.burn_per_hour.saturating_sub(Self::level(self.burn_level, self.burn_at, self.params.burn_per_hour, now)?))
    }

    fn volume(&mut self, lamports: u64, now: i64) {
        let day = (now / DAY) as u64;
        let i = (day % DAYS as u64) as usize;
        if self.day_of[i] != day {
            self.day_of[i] = day;
            self.day_volume[i] = 0;
        }
        self.day_volume[i] = self.day_volume[i].saturating_add(lamports);
    }

    /// `volume7d()`: mint + burn lamports over the trailing seven days (the treasury's weight).
    pub fn volume_7d(&self, now: i64) -> u64 {
        let today = (now / DAY) as u64;
        let mut v: u64 = 0;
        for i in 0..DAYS {
            if self.day_of[i] + DAYS as u64 > today && self.day_of[i] <= today {
                v = v.saturating_add(self.day_volume[i]);
            }
        }
        v
    }

    fn priced(&self, now: i64, kd: bool) -> Res<()> {
        ensure(self.fresh(now), E::Stale)?;
        ensure(!kd, E::Paused)
    }

    // ---- mint and burn -------------------------------------------------------------------------------------------------

    /// `mint()`, books only: the units the canister mints to the buyer. `e` = SOL-backed units before the mint.
    pub fn mint(&mut self, lamports: u64, min_units: u64, now: i64, e: u64, kd: bool) -> Res<u64> {
        self.atomic(|m| m.mint_raw(lamports, min_units, now, e, kd))
    }

    fn mint_raw(&mut self, lamports: u64, min_units: u64, now: i64, e: u64, kd: bool) -> Res<u64> {
        self.priced(now, kd)?;
        ensure(lamports != 0, E::BadInput)?;
        let (s_mint, _) = self.spreads(now, e)?;
        let base = self.mint_base(now)?;
        let unit_price = mul_div_ceil(base, BPS + s_mint, BPS)?;
        let units = mul_div(lamports, ONE, unit_price)?;
        ensure(units != 0 && units >= min_units, E::Slippage)?;
        self.note_mint_base(base, now);
        let (_, r_units) = self.split(lamports, units);
        ensure(self.net_units(e) as u128 + units as u128 - r_units as u128 <= self.public_cap() as u128, E::OverBacked)?;
        ensure(units <= self.grad_room(e), E::OverBacked)?;
        let lvl = Self::level(self.mint_level, self.mint_at, self.params.mint_per_hour, now)?;
        ensure(lvl as u128 + units as u128 <= self.params.mint_per_hour as u128, E::OverCap)?;
        self.mint_level = lvl + units;
        self.mint_at = now;
        self.intake(lamports, units)?;
        self.volume(lamports, now);
        Ok(units)
    }

    /// `_tierPrices()`: (reserve tier, convertible tier) lamports per whole unit.
    pub fn tier_prices(&self, now: i64, e: u64, kd: bool) -> Res<(u64, u64)> {
        let live = self.live_units(e);
        let r = self.reserve_units;
        let b_r = if r == 0 { 0 } else { mul_div(self.reserve, ONE, r)? };
        let b_c = if live > r { mul_div(self.backing, ONE, live - r)? } else { 0 };
        let base = BPS - self.params.base_spread_bps as u64;
        if self.escaped(now) {
            return Ok((mul_div(b_r, base, BPS)?, mul_div(b_c, base, BPS)?));
        }
        if kd {
            let f = self.pause_floor(now)?;
            return Ok((mul_div(b_r.min(f), base, BPS)?, mul_div(b_c.min(f), base, BPS)?));
        }
        ensure(self.fresh(now), E::Stale)?;
        let (_, s_burn) = self.spreads(now, e)?;
        let p = mul_div(self.price(now)?.min(self.burn_target(now)?), BPS - s_burn, BPS)?;
        Ok((p.min(b_r), p.min(b_c)))
    }

    /// `burn()`, books only: (lamports paid, lamports released from the pots).
    pub fn burn(&mut self, units: u64, min_lamports: u64, now: i64, e: u64, kd: bool) -> Res<(u64, u64)> {
        self.atomic(|m| m.burn_raw(units, min_lamports, now, e, kd))
    }

    fn burn_raw(&mut self, units: u64, min_lamports: u64, now: i64, e: u64, kd: bool) -> Res<(u64, u64)> {
        ensure(units != 0, E::BadInput)?;
        let (pr, pc) = self.tier_prices(now, e, kd)?;
        let live = self.live_units(e);
        ensure(units <= live, E::NotEnoughBacked)?;
        let lvl = Self::level(self.burn_level, self.burn_at, self.params.burn_per_hour, now)?;
        ensure(lvl as u128 + units as u128 <= self.params.burn_per_hour as u128, E::OverCap)?;
        self.burn_level = lvl + units;
        self.burn_at = now;
        let (paid, released) = self.release(units, live, pr, pc)?;
        ensure(paid != 0 && paid >= min_lamports, E::Slippage)?;
        self.volume(paid, now);
        Ok((paid, released))
    }

    /// `_release()`: reserve units first; each tier releases exactly its backing per unit and pays at most that.
    fn release(&mut self, units: u64, live: u64, pr: u64, pc: u64) -> Res<(u64, u64)> {
        let r = self.reserve_units;
        let r_u = units.min(r);
        let mut paid_total = 0;
        let mut released = 0;
        if r_u != 0 {
            let rel = if r_u == r { self.reserve } else { mul_div(self.reserve, r_u, r)? };
            let paid = mul_div(r_u, pr, ONE)?;
            ensure(paid <= rel, E::NotEnoughBacked)?;
            self.reserve -= rel;
            self.reserve_units = r - r_u;
            self.surplus = add(self.surplus, rel - paid)?;
            paid_total = paid;
            released = rel;
        }
        let c_u = units - r_u;
        if c_u != 0 {
            let c = live - r;
            let rel = if c_u == c { self.backing } else { mul_div(self.backing, c_u, c)? };
            let paid = mul_div(c_u, pc, ONE)?;
            ensure(paid <= rel, E::NotEnoughBacked)?;
            self.backing -= rel;
            self.revenue = add(self.revenue, rel - paid)?;
            paid_total += paid;
            released += rel;
        }
        Ok((paid_total, released))
    }

    // ---- the canister's purchases --------------------------------------------------------------------------------------

    /// `itemCap()` (N-7).
    pub fn item_cap(&self, now: i64, kd: bool) -> Res<u64> {
        if self.fresh(now) && !kd {
            mul_div(self.accepted, BPS + self.params.conv_bps as u64, BPS)
        } else {
            Ok(0)
        }
    }

    fn cap_or_snapshot(&self, now: i64, kd: bool) -> Res<u64> {
        if kd {
            return Ok(0);
        }
        let mut max = self.item_cap(now, kd)?;
        if self.snap_cap > max && now < self.snap_at + SNAPSHOT_TTL {
            max = self.snap_cap;
        }
        Ok(max)
    }

    pub fn arrival_room(&self, now: i64) -> bool {
        let n = self.arrivals;
        let m = self.params.max_items_per_day as u64;
        if n < m {
            return true;
        }
        now >= self.arrived_at[((n - m) as usize) % MAX_ITEMS_PER_DAY] + DAY
    }

    fn count_arrival(&mut self, now: i64) -> Res<()> {
        ensure(self.arrival_room(now), E::ArrivalRateLimited)?;
        let n = self.arrivals;
        self.arrived_at[(n as usize) % MAX_ITEMS_PER_DAY] = now;
        self.arrivals = n + 1;
        Ok(())
    }

    /// `maxItemCost()`.
    pub fn max_item_cost(&self, now: i64, e: u64, kd: bool) -> Res<u64> {
        let c = self.net_units(e);
        if c < ONE || !self.arrival_room(now) {
            return Ok(0);
        }
        let per = mul_div(self.backing, ONE, c)?;
        Ok(self.cap_or_snapshot(now, kd)?.min(per.saturating_add(self.revenue).saturating_add(self.surplus)))
    }

    /// `snapshotCap()`.
    pub fn snapshot_cap(&mut self, now: i64, kd: bool) -> Res<u64> {
        let cap = self.item_cap(now, kd)?;
        ensure(cap != 0, E::CapStale)?;
        self.snap_cap = cap;
        self.snap_at = now;
        Ok(cap)
    }

    fn fill(&mut self, price: u64, now: i64) {
        self.fills[(self.fill_count as usize) % FILLS] = Fill { price, at: now };
        self.fill_count += 1;
    }

    /// One convertible unit releases its backing per unit; `cost` beyond it comes from revenue, then surplus.
    fn take_one(&mut self, cost: u64, e: u64) -> Res<u64> {
        let c = self.net_units(e);
        ensure(c >= ONE, E::NotEnoughBacked)?;
        let mut released = mul_div(self.backing, ONE, c)?;
        self.backing -= released;
        if c == ONE && self.backing != 0 {
            released += self.backing;
            self.backing = 0;
        }
        if cost <= released {
            self.revenue = add(self.revenue, released - cost)?;
        } else {
            let need = cost - released;
            ensure(need <= self.revenue.saturating_add(self.surplus), E::OverBasis)?;
            let from_revenue = need.min(self.revenue);
            self.revenue -= from_revenue;
            self.surplus -= need - from_revenue;
        }
        Ok(released)
    }

    /// `itemArrived(cost)`, books only. `module_wx` = the item units the ledger credits the module (never on chain); the caller
    /// takes one (`sol_backed −= ONE`) and pays `cost` from `M[i]` to the purchase leg.
    pub fn item_arrived(&mut self, cost: u64, now: i64, e: u64, kd: bool, module_wx: u64) -> Res<u64> {
        self.atomic(|m| {
            ensure(cost != 0, E::BadInput)?;
            let max = m.cap_or_snapshot(now, kd)?;
            ensure(max != 0, E::CapStale)?;
            ensure(cost <= max, E::OverItemCap)?;
            ensure(module_wx >= ONE, E::NoWrapped)?;
            let released = m.take_one(cost, e)?;
            m.count_arrival(now)?;
            let conv = m.params.conv_bps as u64;
            m.fill(mul_div(cost, BPS, BPS + conv)?, now);
            m.items_arrived += 1;
            Ok(released)
        })
    }

    /// `advance(amount)`, books only; the caller pays `amount` from `M[i]` to the purchase leg.
    pub fn advance(&mut self, amount: u64, now: i64, e: u64, kd: bool) -> Res<u64> {
        self.atomic(|m| {
            ensure(m.transit_units == 0, E::AdvanceOpen)?;
            ensure(amount != 0, E::BadInput)?;
            let max = m.max_item_cost(now, e, kd)?;
            ensure(amount <= max, E::OverItemCap)?;
            let released = m.take_one(amount, e)?;
            m.count_arrival(now)?;
            m.transit_units = ONE;
            m.advanced = amount;
            m.advance_basis = released;
            m.advanced_at = now;
            Ok(released)
        })
    }

    /// `settleAdvance(cost)`, books only; the caller takes the module's item unit (`sol_backed −= ONE`).
    pub fn settle_advance(&mut self, cost: u64, now: i64, kd: bool, module_wx: u64) -> Res<u64> {
        self.atomic(|m| {
            ensure(m.transit_units != 0, E::NoAdvance)?;
            let adv = m.advanced;
            ensure(cost != 0 && cost <= adv, E::BadInput)?;
            ensure(module_wx >= ONE, E::NoWrapped)?;
            m.transit_units = 0;
            m.advanced = 0;
            m.advance_basis = 0;
            m.advanced_at = 0;
            if !kd {
                let conv = m.params.conv_bps as u64;
                m.fill(mul_div(cost, BPS, BPS + conv)?, now);
            }
            m.items_arrived += 1;
            Ok(adv)
        })
    }

    /// `returnAdvance()` with `v` lamports back at `M[i]`. Returns (basis, loss).
    pub fn return_advance(&mut self, v: u64) -> Res<(u64, u64)> {
        self.atomic(|m| {
            ensure(m.transit_units != 0, E::NoAdvance)?;
            let basis = m.advance_basis;
            m.transit_units = 0;
            m.advanced = 0;
            m.advance_basis = 0;
            m.advanced_at = 0;
            let mut loss = 0;
            if v >= basis {
                m.backing = add(m.backing, basis)?;
                m.revenue = add(m.revenue, v - basis)?;
            } else {
                let need = basis - v;
                let from_revenue = need.min(m.revenue);
                m.revenue -= from_revenue;
                let from_surplus = (need - from_revenue).min(m.surplus);
                m.surplus -= from_surplus;
                loss = need - from_revenue - from_surplus;
                m.backing = add(m.backing, basis - loss)?;
            }
            Ok((basis, loss))
        })
    }

    /// `skim()`: revenue above `revenue_target` (all of it, and the surplus, once escaped).
    pub fn skim(&mut self, now: i64) -> Res<u64> {
        self.atomic(|m| {
            let esc = m.escaped(now);
            let keep = if esc { 0 } else { m.params.revenue_target };
            let mut amount = 0;
            if m.revenue > keep {
                amount = m.revenue - keep;
                m.revenue = keep;
            }
            if m.surplus != 0 && esc {
                amount += m.surplus;
                m.surplus = 0;
            }
            ensure(amount != 0, E::NothingToSettle)?;
            Ok(amount)
        })
    }

    pub fn fund_reserve(&mut self, lamports: u64) -> Res<()> {
        ensure(lamports != 0, E::BadInput)?;
        self.surplus = add(self.surplus, lamports)?;
        Ok(())
    }

    /// `mintPrice()`: lamports per whole unit now.
    pub fn mint_price(&self, now: i64, e: u64) -> Res<u64> {
        let (s, _) = self.spreads(now, e)?;
        mul_div_ceil(self.mint_base(now)?, BPS + s, BPS)
    }

    /// The backing per whole unit of the tier a burn draws first (`beta()`).
    pub fn beta(&self, e: u64) -> Res<u64> {
        let live = self.live_units(e);
        if live == 0 {
            return Ok(0);
        }
        if self.reserve_units != 0 {
            mul_div(self.reserve, ONE, self.reserve_units)
        } else {
            mul_div(self.backing, ONE, live)
        }
    }

    /// The most a burn of `units` may pay: each tier at its own backing per unit, reserve units first (rook_core props.rs
    /// `burn_bound`).
    pub fn burn_bound(&self, e: u64, units: u64) -> u128 {
        let live = self.live_units(e);
        let units = units.min(live);
        let r = self.reserve_units;
        let r_u = units.min(r);
        let c_u = units - r_u;
        let mut b: u128 = 0;
        if r_u != 0 {
            b += r_u as u128 * self.reserve as u128 / r as u128;
        }
        if c_u != 0 && live > r {
            b += c_u as u128 * self.backing as u128 / (live - r) as u128;
        }
        b
    }

    /// The lamports `M[i]` must hold above its rent beyond the books.
    pub fn sub_books(&self, held: u64) -> Res<u64> {
        sub(held, u64::try_from(self.books()).map_err(|_| E::MathOverflow)?)
    }
}
