//! The pool desk's decisions (pure): what to do with one collection's wX/SOL DLMM pool given what the chain and the books show now. No
//! reads, no keys, no clock but `now`; every number is an argument, so each rule is tested on its own (§5.3, §5.4, §17; spike S4).
//!
//! Prices are lamports per WHOLE wX (the books' unit) unless named `*_q64` (DLMM's Q64.64 lamports per wX base unit).
use std::collections::BTreeMap;

use crate::books::ONE;
use crate::solix2::dlmm::{self, Bin, LbPair, SwapQuote};

/// The band the desk's position spans: ±10 % in price around the floor (§17, S4: 39 bins at bin step 50).
pub const BAND_BPS: u32 = 1_000;
/// Re-centre when the floor has drifted this far from the position's centre (§5.4: 70 % of the way to an edge ≈ 7 %).
pub const RECENTER_BPS: u32 = 700;
/// The pool is "at the floor" (the arb has done its part, a re-centre may go) within this many bins.
pub const AT_FLOOR_BINS: i32 = 1;
/// Target depth a side (§5.3: ≈ 10 SOL a side passes both Jupiter tests), and the least a side a pool opens with (≈ 0.5 SOL, O-1).
pub const TARGET_SIDE_LAMPORTS: u64 = 10_000_000_000;
pub const OPEN_MIN_SIDE_LAMPORTS: u64 = 500_000_000;
/// The rent a pool's opening sinks (S4 §5.1 item 5): the pair (0.0346), one or two bin arrays (0.0714 each), the position (0.0574,
/// refundable). Kept above both sides before the desk opens.
pub const OPEN_RENT_LAMPORTS: u64 = 34_647_000 + 2 * 71_437_000 + 57_406_000;
/// Arb bounds (§5.4 [A]): the profit floor (0.0005 SOL + the fee + ≈ 3 signatures of cycles), the share of the pool's scarcer side one arb
/// may take, arbs an hour, the desk's wX inventory as a share of the collection's backing, the slippage on `min_out`.
pub const ARB_MIN_PROFIT: u64 = 1_200_000;
pub const ARB_SIDE_BPS: u64 = 2_500;
pub const ARBS_PER_HOUR: usize = 12;
pub const INVENTORY_BACKING_BPS: u64 = 1_000;
pub const SLIPPAGE_BPS: u64 = 30;
/// Fees claimed when at least this much SOL is claimable; the desk's SOL above its target capital goes to the treasury (§5.3).
pub const CLAIM_MIN_LAMPORTS: u64 = 20_000_000;
/// The desk keeps this much native SOL for the module payments of its mint legs (the rest is wrapped into the position).
pub const NATIVE_FLOAT_LAMPORTS: u64 = 1_000_000_000;
/// The rent-exempt minimum of a system account the desk key never goes under.
pub const KEY_RENT: u64 = 890_880;

/// What the books say of the collection now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Prices {
    /// The accepted floor.
    pub floor: u64,
    /// A floor post under its TTL with the poster up (the up-arb and every re-centre need it).
    pub fresh: bool,
    /// The public mint's price of one whole wX now (`None`: mint closed).
    pub mint: Option<u64>,
    /// What the public burn pays for one whole wX now (`None`: burns closed).
    pub burn: Option<u64>,
    /// The module's backing (the inventory bound).
    pub backing: u64,
}

/// The floor's bin.
pub fn floor_bin(bin_step: u16, floor: u64) -> Option<i32> {
    dlmm::bin_for_price(bin_step, dlmm::price_of_floor(floor))
}

/// The band's half width in bins, and the re-centre trigger.
pub fn half(bin_step: u16) -> i32 {
    dlmm::half_width(bin_step, BAND_BPS).unwrap_or(19)
}
pub fn trigger(bin_step: u16) -> i32 {
    dlmm::half_width(bin_step, RECENTER_BPS).unwrap_or(14)
}

/// The price of bin `id` in lamports per whole wX.
pub fn bin_floor(bin_step: u16, id: i32) -> u64 {
    dlmm::price_q64(bin_step, id).and_then(dlmm::floor_of_price).unwrap_or(0)
}

/// What a re-centre must do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Recenter {
    /// The position's centre is within the trigger of the floor.
    No,
    /// The floor moved inside the band and the pool followed it (the arb's part): rebalance in place around the active bin (S4 R1).
    Inside { active: i32 },
    /// The floor left the band: withdraw, move the empty pool's price to the floor bin, add around it (S4 R2, as two transactions).
    Past { target: i32 },
    /// The floor moved inside the band but the pool has not followed yet: the arb goes first.
    WaitArb,
}

/// §5.4 / §17: when and how to re-centre the position `[lo, hi]`.
pub fn recenter(bin_step: u16, lo: i32, hi: i32, active: i32, floor: i32) -> Recenter {
    let centre = lo + (hi - lo) / 2;
    if (floor - centre).abs() < trigger(bin_step) {
        return Recenter::No;
    }
    if floor <= lo || floor >= hi {
        return Recenter::Past { target: floor };
    }
    if (active - floor).abs() <= AT_FLOOR_BINS {
        Recenter::Inside { active }
    } else {
        Recenter::WaitArb
    }
}

/// The spot amounts of a deposit around active bin `a` with `h` bins a side (S4 §2.4): `y0` lamports in each of the `h + 1` bid bins (the
/// active one holds SOL), `x0` lamports' worth of wX in each of the `h` ask bins. Returns `(x0, y0, x_needed)`, `x_needed` the wX the ask
/// side takes (1 % over, as the spike minted).
pub fn spot_amounts(bin_step: u16, a: i32, h: i32, value_x_lamports: u64, y_lamports: u64, x_units: u64, mint_room: u64) -> (u64, u64, u64) {
    // the most per bin each side can fund: the SOL over h + 1 bins; the wX (held plus what may be minted) over the ask bins' prices
    let inv: u128 = (1..=h).map(|i| dlmm::price_q64(bin_step, a + i).map(|p| (1u128 << 64) * (1u128 << 32) / p).unwrap_or(0)).sum();
    let per_y = y_lamports as u128 / (h as u128 + 1);
    let total = (value_x_lamports as u128 + y_lamports as u128) * 98 / 100 / (2 * h as u128 + 1);
    let x_cap = if inv == 0 { 0 } else { ((x_units as u128 + mint_room as u128) << 32) * 100 / 101 / inv };
    let per = total.min(per_y).min(x_cap);
    let x_needed = (per * inv >> 32) * 101 / 100;
    (per as u64, per as u64, u64::try_from(x_needed).unwrap_or(u64::MAX))
}

/// The two sides of a pool, read from its bins (both in lamports: wX valued at `price` per whole).
pub fn sides(bins: &BTreeMap<i32, Bin>, price: u64) -> (u64, u64) {
    let (x, y): (u128, u128) = bins.values().fold((0, 0), |(x, y), b| (x + b.amount_x as u128, y + b.amount_y as u128));
    ((x * price as u128 / ONE as u128) as u64, y as u64)
}

/// One arb.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Arb {
    /// The pool is above the band: mint `units` at the module for `lamports` and sell them for at least `min_out`.
    Up { units: u64, lamports: u64, min_out: u64, quote: SwapQuote, profit: u64 },
    /// The pool is below the band: buy wX with `lamports` (at least `min_out`), to burn at the module after.
    Down { lamports: u64, min_out: u64, quote: SwapQuote, profit: u64 },
}

/// What one arb may use.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ArbRoom {
    /// The desk's native SOL for a mint leg, and its wSOL for a buy.
    pub native: u64,
    pub wsol: u64,
    /// wX the module may still mint now (its hourly room and caps), base units.
    pub mint_units: u64,
    /// wX the desk may hold more of (the inventory bound), base units.
    pub inventory_units: u64,
    /// Arbs already made in the last hour.
    pub arbs_last_hour: usize,
}

fn best_by<F: Fn(u64) -> Option<(i128, SwapQuote)>>(cap: u64, f: F) -> Option<(u64, i128, SwapQuote)> {
    if cap == 0 {
        return None;
    }
    // a coarse grid, then a refinement around the best point: profit is concave in size (a walk over bins at falling prices against a
    // linear cost), so the best grid point brackets the optimum
    let mut best: Option<(u64, i128, SwapQuote)> = None;
    let grid = 32u64;
    for k in 1..=grid {
        let x = (cap as u128 * k as u128 / grid as u128) as u64;
        if let Some((p, q)) = f(x) {
            if best.as_ref().is_none_or(|b| p > b.1) {
                best = Some((x, p, q));
            }
        }
    }
    let (x0, ..) = best?;
    let step = (cap / grid).max(1);
    let (mut lo, mut hi) = (x0.saturating_sub(step).max(1), x0.saturating_add(step).min(cap));
    for _ in 0..40 {
        if hi <= lo + 1 {
            break;
        }
        let m1 = lo + (hi - lo) / 3;
        let m2 = hi - (hi - lo) / 3;
        let (p1, p2) = (f(m1).map(|r| r.0).unwrap_or(i128::MIN), f(m2).map(|r| r.0).unwrap_or(i128::MIN));
        if p1 < p2 {
            lo = m1 + 1;
        } else {
            hi = m2;
        }
    }
    for x in [lo, hi] {
        if let Some((p, q)) = f(x) {
            if best.as_ref().is_none_or(|b| p > b.1) {
                best = Some((x, p, q));
            }
        }
    }
    best
}

/// §5.4: the arb that brings the pool back into the band `[burn/(1+f), mint/(1−f)]`, sized for the most profit within the bounds, or
/// none. `burn_of(units)` is what the public burn pays for `units` now (the books' `quote_burn`).
pub fn arb(pair: &LbPair, bins: &BTreeMap<i32, Bin>, p: &Prices, room: &ArbRoom, now: i64, burn_of: &dyn Fn(u64) -> Option<u64>) -> Option<Arb> {
    if room.arbs_last_hour >= ARBS_PER_HOUR {
        return None;
    }
    let price = bin_floor(pair.bin_step, pair.active_id);
    let f = pair.current_fee_rate(now) as u128;
    let (x_side, y_side) = sides(bins, price.max(1));
    // up: the pool sells above the mint (it pays the seller more than the mint costs)
    if let (true, Some(m)) = (p.fresh, p.mint) {
        if price as u128 * (dlmm::FEE_PRECISION - f) > m as u128 * dlmm::FEE_PRECISION {
            let by_native = (room.native.saturating_sub(KEY_RENT) as u128 * ONE as u128 / m as u128) as u64;
            let by_side = (y_side as u128 * ARB_SIDE_BPS as u128 / 10_000 * ONE as u128 / price.max(1) as u128) as u64;
            // the minted units are sold in the same transaction: the inventory bound is the down-arb's, not this one's
            let cap = room.mint_units.min(by_native).min(by_side);
            let profit_of = |x: u64| -> Option<(i128, SwapQuote)> {
                let q = dlmm::quote_swap(pair, bins, true, x, now).ok()?;
                let cost = (x as u128 * m as u128).div_ceil(ONE as u128) as i128;
                Some((q.amount_out as i128 - cost, q))
            };
            if let Some((units, profit, q)) = best_by(cap, profit_of) {
                if profit >= ARB_MIN_PROFIT as i128 && q.complete {
                    let lamports = (units as u128 * m as u128).div_ceil(ONE as u128) as u64;
                    // audit B3-02: the 0.3 % allowance alone lets a sandwich land the sale under the mint leg's cost whenever 0.3 % of the
                    // proceeds exceeds the profit; the least the swap may return is never under the cost plus the profit floor. The quote
                    // clears that bound (profit ≥ ARB_MIN_PROFIT and the cost is `lamports`), so the clamp never exceeds it
                    let min_out = up_min_out(q.amount_out, lamports);
                    return Some(Arb::Up { units, lamports, min_out, quote: q, profit: profit as u64 });
                }
            }
        }
    }
    // down: the pool sells below what the burn pays
    if let Some(b) = p.burn {
        if price as u128 * (dlmm::FEE_PRECISION + f) < b as u128 * dlmm::FEE_PRECISION {
            let by_side = (x_side as u128 * ARB_SIDE_BPS as u128 / 10_000) as u64;
            let by_inv = (room.inventory_units as u128 * price as u128 / ONE as u128) as u64;
            let cap = room.wsol.min(by_side).min(by_inv);
            let profit_of = |y: u64| -> Option<(i128, SwapQuote)> {
                let q = dlmm::quote_swap(pair, bins, false, y, now).ok()?;
                let back = burn_of(q.amount_out)?;
                Some((back as i128 - y as i128, q))
            };
            if let Some((lamports, profit, q)) = best_by(cap, profit_of) {
                if profit >= ARB_MIN_PROFIT as i128 && q.complete {
                    return Some(Arb::Down { lamports, min_out: q.amount_out - q.amount_out * SLIPPAGE_BPS / 10_000, quote: q, profit: profit as u64 });
                }
            }
        }
    }
    None
}

/// The up-arb's `minimum_amount_out` (§5.4, audit B3-02): the quote less [`SLIPPAGE_BPS`], but never under what the mint leg paid plus
/// [`ARB_MIN_PROFIT`], so that no fill the swap accepts is a loss.
pub fn up_min_out(quoted: u64, lamports: u64) -> u64 {
    (quoted - quoted * SLIPPAGE_BPS / 10_000).max(lamports.saturating_add(ARB_MIN_PROFIT))
}

/// Is the desk funded enough to open its pool: both sides of [`OPEN_MIN_SIDE_LAMPORTS`] plus the opening's rent, the native float the
/// deposit keeps ([`NATIVE_FLOAT_LAMPORTS`]) and the key's rent, counting its wX at the floor only as what they save on the mint leg
/// (audit B3-08: the pair and its arrays were opened with too little left to make the first deposit at the designed size).
pub fn can_open(native: u64, wsol: u64, wx_units: u64, floor: u64) -> bool {
    let sol = native as u128 + wsol as u128;
    let wx_value = wx_units as u128 * floor as u128 / ONE as u128;
    let fixed = OPEN_RENT_LAMPORTS as u128 + NATIVE_FLOAT_LAMPORTS as u128 + KEY_RENT as u128;
    sol + wx_value >= 2 * OPEN_MIN_SIDE_LAMPORTS as u128 + fixed && sol >= OPEN_MIN_SIDE_LAMPORTS as u128 + fixed
}

/// The desk's total value at `floor`: SOL (native and wrapped), wX held, and the position.
pub fn value(native: u64, wsol: u64, wx_units: u64, pos_x: u64, pos_y: u64, floor: u64) -> u128 {
    native as u128 + wsol as u128 + pos_y as u128 + (wx_units as u128 + pos_x as u128) * floor as u128 / ONE as u128
}

/// Whether a pool is under its target depth: its SOL side (and its wX side at the floor) under [`TARGET_SIDE_LAMPORTS`].
pub fn under_target(pos_x: u64, pos_y: u64, floor: u64) -> bool {
    let x_value = pos_x as u128 * floor as u128 / ONE as u128;
    (pos_y as u128) < TARGET_SIDE_LAMPORTS as u128 || x_value < TARGET_SIDE_LAMPORTS as u128
}

/// The desk's SOL above what its pool needs at target (both sides, plus the native float): the treasury's (§5.3).
pub fn excess_for_treasury(native: u64, wsol: u64, pos_x: u64, pos_y: u64, floor: u64) -> u64 {
    if under_target(pos_x, pos_y, floor) {
        return 0;
    }
    let keep = NATIVE_FLOAT_LAMPORTS as u128 + KEY_RENT as u128;
    let spare = (native as u128 + wsol as u128).saturating_sub(keep);
    u64::try_from(spare).unwrap_or(u64::MAX)
}
