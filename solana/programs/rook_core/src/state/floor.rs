//! The floor module (src/floor/FloorModule.sol), one per collection, in lamports and 9-decimal units. Every function here is
//! pure over the account plus the inputs the EVM read from elsewhere: `now` (unix time; §2.4 has no parent-chain clock),
//! `e` = the wrapper's `eth_backed`, and `kd` = `_keeperDown()` (the registry's or this collection's poster switch).
//! Line refs are FloorModule.sol at tag robinhood-stop-2026-10-09.
//!
//! Not ported (see docs/v9/rook-core-as-built.md §5): rentals, floats and `itemSold`, the bootstrap loan, the graduation queue
//! (`enqueue`/`fill`/`release`/`cancel`/`hasRoomFor`/`holdsRefunds`: `convert` converts at once or refuses), `fundLiquidity`,
//! `reclaimStray`.

use anchor_lang::prelude::*;

use crate::clock::{elapsed, DAY, HOUR, MINUTE};
use crate::error::RookError;
use crate::math::{add, mul_div, mul_div_ceil, sub};
use crate::params::Params;
use crate::units::{BPS, ONE};

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
/// The H11 ring: `maxItemsPerDay` may be at most this (the EVM used an unbounded mapping).
pub const MAX_ITEMS_PER_DAY: usize = 32;

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, Default, Debug, PartialEq, Eq, InitSpace)]
pub struct Fill {
    /// Lamports per whole unit: a purchase's cost net of the conversion allowance.
    pub price: u64,
    pub at: i64,
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, Default, Debug, PartialEq, Eq, InitSpace)]
pub struct MintLow {
    pub price: u64,
    pub bucket: i64,
}

/// PDA `["module", listing]`. The lamports are in `ModuleVault` (`["module_vault", module]`); the books below are claims on
/// them: `vault.lamports − rent ≥ backing + reserve + surplus + revenue`.
#[account]
#[derive(InitSpace)]
pub struct Module {
    pub listing: Pubkey,
    pub wrapper: Pubkey,
    /// `["module_vault", module]`: holds the lamports.
    pub vault: Pubkey,
    /// `["module_wx", module]`: the module's own wX token account (an item bought with an advance is minted here, then
    /// burned by `settle_advance` / `item_arrived`).
    pub module_wx: Pubkey,
    pub bump: u8,
    pub vault_bump: u8,
    pub wx_bump: u8,
    pub params: Params,
    // ---- the floor ----
    /// Lamports per whole unit, after the move limit.
    pub accepted: u64,
    /// The poster's last posted value.
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
    /// The convertible pot.
    pub backing: u64,
    /// The standing reserve's basis: `reserve_bps` of every intake, never converted.
    pub reserve: u64,
    /// No unit claims it; tops up conversions; skimmed once escaped.
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
    // ---- totals (for the reconciler; no rule reads them) ----
    pub converted_lamports: u64,
    pub converted_units: u64,
}

/// The lamport-holding account of a module. PDA `["module_vault", module]`, owned by this program, no fields.
#[account]
#[derive(InitSpace)]
pub struct ModuleVault {}

pub fn validate_params(p: &Params) -> Result<()> {
    let ok = p.reserve_bps <= 5_000
        && p.fill_band_bps <= 1_000
        && p.max_spread_bps <= 5_000
        && p.base_spread_bps <= p.max_spread_bps
        && p.max_move_bps != 0
        && p.max_move_bps <= 2_000
        && p.conv_bps <= 2_000
        && p.buffer_bps <= 2_000
        && p.stale_spread_bps <= p.max_spread_bps
        && p.stress_spread_bps <= p.max_spread_bps
        && (p.vol_weight_bps as u64) <= 5 * BPS
        && p.mint_per_hour != 0
        && p.burn_per_hour != 0
        && p.max_public_eth_backed != 0
        && p.max_public_eth_backed <= p.max_eth_backed
        && p.low_confidence_spread_bps <= p.max_spread_bps
        && p.full_confidence_bps != 0
        && (p.full_confidence_bps as u64) <= BPS
        && p.min_confidence_bps <= p.full_confidence_bps
        && p.max_items_per_day != 0
        && (p.max_items_per_day as usize) <= MAX_ITEMS_PER_DAY;
    require!(ok, RookError::BadParams);
    Ok(())
}

impl Module {
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

    /// `_dayAnchorNow()`.
    pub fn day_anchor_now(&self, now: i64) -> (u64, bool) {
        if self.day_anchor == 0 || now >= self.day_anchor_at + DAY_WINDOW {
            (self.accepted, true)
        } else {
            (self.day_anchor, false)
        }
    }

    /// `floorBounds()` (:422-432).
    pub fn floor_bounds(&self, now: i64) -> Result<(u64, u64)> {
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
    pub fn vol_bps(&self, now: i64) -> Result<u64> {
        let el = elapsed(now, self.posted_at).min(HOUR) as u64;
        mul_div_ceil(self.move_acc, HOUR as u64 - el, HOUR as u64)
    }

    /// `postFloor` (:389-417), after the signer check. Returns (accepted, moved bps).
    pub fn post_floor(&mut self, price: u64, confidence_bps: u16, depth_items: u32, now: i64) -> Result<(u64, u64)> {
        require!(price != 0 && (confidence_bps as u64) <= BPS, RookError::BadInput);
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

    /// `price()` (:1211-1238): the TWAP of the module's own fills over 24 h, within `fill_band_bps` of the accepted floor;
    /// the accepted floor alone with no fill in the window.
    pub fn price(&self, now: i64) -> Result<u64> {
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
            // `elapsed` (floored at 0), as the final term: a fill stamped before the previous one cannot panic (A1-04)
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

    /// `_upTarget()`: the target as mints and conversions count it, no higher than the day band's top (A-10).
    pub fn up_target(&self, now: i64) -> Result<u64> {
        let (anchor, _) = self.day_anchor_now(now);
        Ok(self.target.min(mul_div(anchor, BPS + MAX_DAY_MOVE_BPS, BPS)?.max(self.accepted)))
    }

    /// `_burnTarget()` (N-1a): the target as burns count it, never below the next hour's legal floor nor the day band.
    pub fn burn_target(&self, now: i64) -> Result<u64> {
        let a = self.accepted;
        let (anchor, _) = self.day_anchor_now(now);
        let day_lo = mul_div_ceil(anchor, BPS - MAX_DAY_MOVE_BPS, BPS)?;
        let lo = mul_div_ceil(a, BPS - self.params.max_move_bps as u64, BPS)?.max(day_lo.min(a));
        Ok(self.target.max(lo))
    }

    pub fn mint_base(&self, now: i64) -> Result<u64> {
        Ok(self.price(now)?.max(self.up_target(now)?))
    }

    /// `gradPrice()` (:1259-1261): what a conversion pays per whole unit, `max(price, upTarget) × (1 + convBps)`, rounded up.
    pub fn grad_price(&self, now: i64) -> Result<u64> {
        mul_div_ceil(self.mint_base(now)?, BPS + self.params.conv_bps as u64, BPS)
    }

    /// `_pauseFloor()` (A3-M1, A4-L1).
    pub fn pause_floor(&self, now: i64) -> Result<u64> {
        let mut f = self.burn_target(now)?.min(mul_div(self.accepted, BPS - self.params.fill_band_bps as u64, BPS)?);
        let b = (self.posted_at + FLOOR_TTL) / MINT_LOW_BUCKET;
        for v in self.mint_low.iter() {
            if v.price != 0 && (v.bucket == b || v.bucket + 1 == b) {
                f = f.min(v.price);
            }
        }
        Ok(f)
    }

    /// `_noteMintBase()`.
    fn note_mint_base(&mut self, base: u64, now: i64) {
        let b = now / MINT_LOW_BUCKET;
        let i = (b % 2) as usize;
        let v = self.mint_low[i];
        if v.price == 0 || v.bucket != b || base < v.price {
            self.mint_low[i] = MintLow { price: base, bucket: b };
        }
    }

    // ---- units ---------------------------------------------------------------------------------------------------------

    /// `_liveUnits()`: ETH-backed units less the one in transit.
    pub fn live_units(&self, e: u64) -> u64 {
        e.saturating_sub(self.transit_units)
    }

    /// `netUnits()`: the canister's buy signal, `eth_backed − transit − reserve_units`.
    pub fn net_units(&self, e: u64) -> u64 {
        self.live_units(e).saturating_sub(self.reserve_units)
    }

    pub fn public_cap(&self) -> u64 {
        self.params.max_public_eth_backed.min((self.depth_items as u64).saturating_mul(ONE))
    }

    /// `spreads()` (:1192-1205): (mint, burn) bps.
    pub fn spreads(&self, now: i64, e: u64) -> Result<(u64, u64)> {
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

    /// `_split()`: the reserve's share of an intake; a share too small to carry both a lamport and a unit is all convertible.
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

    /// `_intake()`: `reserve_bps` of the lamports and units into the standing reserve, the rest convertible.
    fn intake(&mut self, lamports: u64, units: u64) -> Result<()> {
        let (r_l, r_u) = self.split(lamports, units);
        self.reserve = add(self.reserve, r_l)?;
        self.reserve_units = add(self.reserve_units, r_u)?;
        self.backing = add(self.backing, lamports - r_l)?;
        Ok(())
    }

    /// `_gradRoom()`: units still under `max_eth_backed` (no queued graduations exist here).
    pub fn grad_room(&self, e: u64) -> u64 {
        self.params.max_eth_backed.saturating_sub(e)
    }

    /// `_level()`: a leaky bucket that drains `cap` per hour.
    fn level(level: u64, at: i64, cap: u64, now: i64) -> Result<u64> {
        let leak = mul_div(cap, elapsed(now, at).min(HOUR) as u64, HOUR as u64)?;
        Ok(level.saturating_sub(leak))
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

    fn priced(&self, now: i64, kd: bool) -> Result<()> {
        require!(self.fresh(now), RookError::Stale);
        require!(!kd, RookError::Paused);
        Ok(())
    }

    // ---- mint and burn -------------------------------------------------------------------------------------------------

    /// `mint()` (:457-478), books only: returns the units the caller mints to the buyer. `e` is the wrapper's `eth_backed`
    /// before the mint.
    pub fn mint(&mut self, lamports: u64, min_units: u64, now: i64, e: u64, kd: bool) -> Result<u64> {
        self.priced(now, kd)?;
        require!(lamports != 0, RookError::BadInput);
        let (s_mint, _) = self.spreads(now, e)?;
        let base = self.mint_base(now)?;
        let unit_price = mul_div_ceil(base, BPS + s_mint, BPS)?;
        let units = mul_div(lamports, ONE, unit_price)?;
        require!(units != 0 && units >= min_units, RookError::Slippage);
        self.note_mint_base(base, now);
        let (_, r_units) = self.split(lamports, units);
        require!(self.net_units(e) + units - r_units <= self.public_cap(), RookError::OverBacked);
        require!(units <= self.grad_room(e), RookError::OverBacked);
        let lvl = Self::level(self.mint_level, self.mint_at, self.params.mint_per_hour, now)?;
        require!(lvl + units <= self.params.mint_per_hour, RookError::OverCap);
        self.mint_level = lvl + units;
        self.mint_at = now;
        self.intake(lamports, units)?;
        self.volume(lamports, now);
        Ok(units)
    }

    /// `_tierPrices()` (:528-542): (reserve tier, convertible tier) lamports per whole unit.
    pub fn tier_prices(&self, now: i64, e: u64, kd: bool) -> Result<(u64, u64)> {
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
        require!(self.fresh(now), RookError::Stale);
        let (_, s_burn) = self.spreads(now, e)?;
        let p = mul_div(self.price(now)?.min(self.burn_target(now)?), BPS - s_burn, BPS)?;
        Ok((p.min(b_r), p.min(b_c)))
    }

    /// `burn()` (:504-522), books only: returns (lamports paid, lamports released from the pots).
    pub fn burn(&mut self, units: u64, min_lamports: u64, now: i64, e: u64, kd: bool) -> Result<(u64, u64)> {
        require!(units != 0, RookError::BadInput);
        let (pr, pc) = self.tier_prices(now, e, kd)?;
        let live = self.live_units(e);
        require!(units <= live, RookError::NotEnoughBacked);
        let lvl = Self::level(self.burn_level, self.burn_at, self.params.burn_per_hour, now)?;
        require!(lvl + units <= self.params.burn_per_hour, RookError::OverCap);
        self.burn_level = lvl + units;
        self.burn_at = now;
        let (paid, released) = self.release(units, live, pr, pc)?;
        require!(paid != 0 && paid >= min_lamports, RookError::Slippage);
        self.volume(paid, now);
        Ok((paid, released))
    }

    /// `_release()` (:603-627): reserve units first, each tier releases exactly its backing per unit and pays at most that.
    fn release(&mut self, units: u64, live: u64, pr: u64, pc: u64) -> Result<(u64, u64)> {
        let r = self.reserve_units;
        let r_u = units.min(r);
        let mut paid_total = 0;
        let mut released = 0;
        if r_u != 0 {
            let rel = if r_u == r { self.reserve } else { mul_div(self.reserve, r_u, r)? };
            let paid = mul_div(r_u, pr, ONE)?;
            require!(paid <= rel, RookError::NotEnoughBacked);
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
            require!(paid <= rel, RookError::NotEnoughBacked);
            self.backing -= rel;
            self.revenue = add(self.revenue, rel - paid)?;
            paid_total += paid;
            released += rel;
        }
        Ok((paid_total, released))
    }

    // ---- convert (the EVM graduation lane, `enqueue` → `_tryFill`, :826-858) -------------------------------------------

    /// Books only: returns the units the caller mints, ETH-backed, to the converter's destination.
    pub fn convert(&mut self, lamports: u64, min_units: u64, now: i64, e: u64, kd: bool) -> Result<u64> {
        require!(lamports != 0, RookError::BadInput);
        self.priced(now, kd)?;
        let unit_price = self.grad_price(now)?;
        let units = mul_div(lamports, ONE, unit_price)?;
        require!(units != 0, RookError::Slippage);
        require!(e.saturating_add(units) <= self.params.max_eth_backed, RookError::OverBacked);
        require!(units >= min_units, RookError::Slippage);
        self.intake(lamports, units)?;
        self.converted_lamports = self.converted_lamports.saturating_add(lamports);
        self.converted_units = self.converted_units.saturating_add(units);
        Ok(units)
    }

    // ---- the canister --------------------------------------------------------------------------------------------------

    /// `itemCap()`: the accepted floor plus the conversion allowance while fresh and the poster is up; 0 otherwise (N-7).
    pub fn item_cap(&self, now: i64, kd: bool) -> Result<u64> {
        if self.fresh(now) && !kd {
            mul_div(self.accepted, BPS + self.params.conv_bps as u64, BPS)
        } else {
            Ok(0)
        }
    }

    fn cap_or_snapshot(&self, now: i64, kd: bool) -> Result<u64> {
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

    /// `_countArrival()` (H11 on the module's side).
    fn count_arrival(&mut self, now: i64) -> Result<()> {
        require!(self.arrival_room(now), RookError::ArrivalRateLimited);
        let n = self.arrivals;
        self.arrived_at[(n as usize) % MAX_ITEMS_PER_DAY] = now;
        self.arrivals = n + 1;
        Ok(())
    }

    /// `maxItemCost()` (:1344-1348).
    pub fn max_item_cost(&self, now: i64, e: u64, kd: bool) -> Result<u64> {
        let c = self.net_units(e);
        if c < ONE || !self.arrival_room(now) {
            return Ok(0);
        }
        let per = mul_div(self.backing, ONE, c)?;
        Ok(self.cap_or_snapshot(now, kd)?.min(per.saturating_add(self.revenue).saturating_add(self.surplus)))
    }

    /// `snapshotCap()`.
    pub fn snapshot_cap(&mut self, now: i64, kd: bool) -> Result<u64> {
        let cap = self.item_cap(now, kd)?;
        require!(cap != 0, RookError::CapStale);
        self.snap_cap = cap;
        self.snap_at = now;
        Ok(cap)
    }

    fn fill(&mut self, price: u64, now: i64) {
        self.fills[(self.fill_count as usize) % FILLS] = Fill { price, at: now };
        self.fill_count += 1;
    }

    /// One convertible unit releases its backing per unit; `cost` beyond it comes from revenue, then surplus (never the
    /// reserve). Shared by `itemArrived` and `advance`. Returns the released backing.
    fn take_one(&mut self, cost: u64, e: u64) -> Result<u64> {
        let c = self.net_units(e);
        require!(c >= ONE, RookError::NotEnoughBacked);
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
            require!(need <= self.revenue.saturating_add(self.surplus), RookError::OverBasis);
            let from_revenue = need.min(self.revenue);
            self.revenue -= from_revenue;
            self.surplus -= need - from_revenue;
        }
        Ok(released)
    }

    /// `itemArrived(cost)` (:634-661), books only. `module_wx` = the module's wX balance (the wrapped item's unit must be
    /// there). The caller burns ONE (eth_backed −= ONE) and pays `cost` to the custodian.
    pub fn item_arrived(&mut self, cost: u64, now: i64, e: u64, kd: bool, module_wx: u64) -> Result<u64> {
        require!(cost != 0, RookError::BadInput);
        let max = self.cap_or_snapshot(now, kd)?;
        require!(max != 0, RookError::CapStale);
        require!(cost <= max, RookError::OverItemCap);
        require!(module_wx >= ONE, RookError::NoWrapped);
        let released = self.take_one(cost, e)?;
        self.count_arrival(now)?;
        let conv = self.params.conv_bps as u64;
        self.fill(mul_div(cost, BPS, BPS + conv)?, now);
        self.items_arrived += 1;
        Ok(released)
    }

    /// `advance(amount)` (:687-713), books only; the caller pays `amount` to the custodian.
    pub fn advance(&mut self, amount: u64, now: i64, e: u64, kd: bool) -> Result<u64> {
        require!(self.transit_units == 0, RookError::AdvanceOpen);
        require!(amount != 0, RookError::BadInput);
        let max = self.max_item_cost(now, e, kd)?;
        require!(amount <= max, RookError::OverItemCap);
        let released = self.take_one(amount, e)?;
        self.count_arrival(now)?;
        self.transit_units = ONE;
        self.advanced = amount;
        self.advance_basis = released;
        self.advanced_at = now;
        Ok(released)
    }

    /// `settleAdvance(cost)` (:718-732), books only; the caller burns the module's ONE (eth_backed −= ONE).
    pub fn settle_advance(&mut self, cost: u64, now: i64, kd: bool, module_wx: u64) -> Result<u64> {
        require!(self.transit_units != 0, RookError::NoAdvance);
        let adv = self.advanced;
        require!(cost != 0 && cost <= adv, RookError::BadInput);
        require!(module_wx >= ONE, RookError::NoWrapped);
        self.transit_units = 0;
        self.advanced = 0;
        self.advance_basis = 0;
        self.advanced_at = 0;
        if !kd {
            let conv = self.params.conv_bps as u64;
            self.fill(mul_div(cost, BPS, BPS + conv)?, now);
        }
        self.items_arrived += 1;
        Ok(adv)
    }

    /// `returnAdvance()` (:738-761) with `v` lamports returned. Returns (basis, loss).
    pub fn return_advance(&mut self, v: u64) -> Result<(u64, u64)> {
        require!(self.transit_units != 0, RookError::NoAdvance);
        let basis = self.advance_basis;
        self.transit_units = 0;
        self.advanced = 0;
        self.advance_basis = 0;
        self.advanced_at = 0;
        let mut loss = 0;
        if v >= basis {
            self.backing = add(self.backing, basis)?;
            self.revenue = add(self.revenue, v - basis)?;
        } else {
            let need = basis - v;
            let from_revenue = need.min(self.revenue);
            self.revenue -= from_revenue;
            let from_surplus = (need - from_revenue).min(self.surplus);
            self.surplus -= from_surplus;
            loss = need - from_revenue - from_surplus;
            self.backing = add(self.backing, basis - loss)?;
        }
        Ok((basis, loss))
    }

    /// `skim()` (:1035-1049): revenue above `revenue_target` (all of it, and the surplus, once escaped).
    pub fn skim(&mut self, now: i64) -> Result<u64> {
        let esc = self.escaped(now);
        let keep = if esc { 0 } else { self.params.revenue_target };
        let mut amount = 0;
        if self.revenue > keep {
            amount = self.revenue - keep;
            self.revenue = keep;
        }
        if self.surplus != 0 && esc {
            amount += self.surplus;
            self.surplus = 0;
        }
        require!(amount != 0, RookError::NothingToSettle);
        Ok(amount)
    }

    pub fn fund_reserve(&mut self, lamports: u64) -> Result<()> {
        require!(lamports != 0, RookError::BadInput);
        self.surplus = add(self.surplus, lamports)?;
        Ok(())
    }

    /// `mintPrice()`, for quotes and tests.
    pub fn mint_price(&self, now: i64, e: u64) -> Result<u64> {
        let (s, _) = self.spreads(now, e)?;
        mul_div_ceil(self.mint_base(now)?, BPS + s, BPS)
    }

    /// The backing per whole unit of the tier a burn draws first (`beta()`).
    pub fn beta(&self, e: u64) -> Result<u64> {
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

    /// For tests: the vault must hold at least this many lamports above its rent.
    pub fn sub_books(&self, held: u64) -> Result<u64> {
        sub(held, self.books() as u64)
    }
}
