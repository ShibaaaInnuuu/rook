//! The floor module's parameters (rook_core `params.rs` + `validate_params`), and the value-based caps of the canister-only design
//! (docs/v9/canister-only-solana.md §4.3, decision O-4).
//!
//! Change from rook_core: [`Params::mint_depth_cap`]. rook_core bounds public minting by `min(max_public_eth_backed, depth_items)`;
//! §4.3 keeps the poster's depth as a brake on the BUY PIPELINE only, so wX-quoted DBC curves can fill. `default_params` keeps
//! rook_core's behaviour (`true`); [`value_params`] sets `false`.

use candid::CandidType;
use serde::{Deserialize, Serialize};

use super::floor::MAX_ITEMS_PER_DAY;
use super::math::{Res, BPS, LAMPORTS_PER_SOL, ONE};
use super::BooksError;

#[derive(CandidType, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Params {
    pub base_spread_bps: u16,
    pub vol_weight_bps: u16,
    pub stale_spread_bps: u16,
    pub stress_spread_bps: u16,
    pub max_spread_bps: u16,
    pub max_move_bps: u16,
    pub conv_bps: u16,
    pub buffer_bps: u16,
    pub low_confidence_spread_bps: u16,
    pub full_confidence_bps: u16,
    pub min_confidence_bps: u16,
    pub max_items_per_day: u16,
    pub reserve_bps: u16,
    pub fill_band_bps: u16,
    pub mint_per_hour: u64,
    pub burn_per_hour: u64,
    /// rook_core `max_public_eth_backed`: SOL-backed units the public may hold beyond the reserve tier.
    pub max_public_eth_backed: u64,
    /// rook_core `max_eth_backed`: all SOL-backed units.
    pub max_eth_backed: u64,
    /// `revenueTarget`, lamports.
    pub revenue_target: u64,
    /// `true`: the public cap is `min(max_public_eth_backed, depth_items · ONE)` (rook_core). `false`: `max_public_eth_backed`
    /// alone (§4.3: the depth brakes the buy pipeline, not minting).
    pub mint_depth_cap: bool,
}

/// rook_core `default_params` (StackChecks stage 0) in 9-decimal units.
pub const fn default_params(revenue_target: u64) -> Params {
    Params {
        base_spread_bps: 15,
        vol_weight_bps: 10_000,
        stale_spread_bps: 100,
        stress_spread_bps: 300,
        max_spread_bps: 1_000,
        max_move_bps: 500,
        conv_bps: 300,
        buffer_bps: 500,
        low_confidence_spread_bps: 100,
        full_confidence_bps: 8_000,
        min_confidence_bps: 3_000,
        max_items_per_day: 3,
        reserve_bps: 2_500,
        fill_band_bps: 300,
        mint_per_hour: 5 * ONE,
        burn_per_hour: 5 * ONE,
        max_public_eth_backed: 10 * ONE,
        max_eth_backed: 40 * ONE,
        revenue_target,
        mint_depth_cap: true,
    }
}

/// SOL of hourly flow the buckets allow at the listing floor (§4.3: `⌈250 SOL / floor⌉`).
pub const VALUE_HOURLY_SOL: u64 = 250;
/// SOL of SOL-backed float `max_eth_backed` allows (§4.3: `⌈2,000 SOL / floor⌉`, ≈ 20 graduations' float).
pub const VALUE_FLOAT_SOL: u64 = 2_000;

/// `⌈x_sol · 10⁹ / floor⌉` whole units, as base units; at least `min_whole` whole units.
fn whole_units_for(x_sol: u64, floor: u64, min_whole: u64) -> Res<u64> {
    let n = (x_sol as u128 * LAMPORTS_PER_SOL as u128).div_ceil(floor as u128);
    let n = u64::try_from(n).map_err(|_| BooksError::MathOverflow)?.max(min_whole);
    n.checked_mul(ONE).ok_or(BooksError::MathOverflow)
}

/// §4.3, per collection, from its floor at listing (`floor`, lamports per whole wX), its graduation threshold `t_wx` (whole wX)
/// and the open curves allowed at once:
/// - `mint_per_hour = burn_per_hour = max(5, ⌈250 SOL / floor⌉)` whole wX;
/// - `max_public_eth_backed = max(10, ⌈1.5 × T_wX × open_curves⌉)` whole wX, the depth no longer bounding minting;
/// - `max_eth_backed = max(40, ⌈2,000 SOL / floor⌉)` whole wX, and never below the public cap;
/// - everything else as `default_params` (spreads, `conv_bps`, `fill_band_bps`, `reserve_bps` 2,500 per O-5, `max_items_per_day` 3).
pub fn value_params(floor: u64, t_wx: u64, open_curves: u64, revenue_target: u64) -> Res<Params> {
    if floor == 0 || t_wx == 0 || open_curves == 0 {
        return Err(BooksError::BadParams);
    }
    let mut p = default_params(revenue_target);
    let hourly = whole_units_for(VALUE_HOURLY_SOL, floor, 5)?;
    p.mint_per_hour = hourly;
    p.burn_per_hour = hourly;
    let public = (t_wx as u128 * open_curves as u128 * 3).div_ceil(2);
    let public = u64::try_from(public).map_err(|_| BooksError::MathOverflow)?.max(10);
    p.max_public_eth_backed = public.checked_mul(ONE).ok_or(BooksError::MathOverflow)?;
    p.max_eth_backed = whole_units_for(VALUE_FLOAT_SOL, floor, 40)?.max(p.max_public_eth_backed);
    p.mint_depth_cap = false;
    validate_params(&p)?;
    Ok(p)
}

/// rook_core `validate_params`.
pub fn validate_params(p: &Params) -> Res<()> {
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
    if ok {
        Ok(())
    } else {
        Err(BooksError::BadParams)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SOL: u64 = LAMPORTS_PER_SOL;

    #[test]
    fn value_caps_follow_the_design_table() {
        // Milady (§3.2): floor 18.61 SOL, T_wX 5
        let p = value_params(18_610_000_000, 5, 4, 20 * SOL).unwrap();
        assert_eq!(p.mint_per_hour, 14 * ONE); // ⌈250 / 18.61⌉ = 14
        assert_eq!(p.max_public_eth_backed, 30 * ONE); // ⌈1.5 × 5 × 4⌉
        assert_eq!(p.max_eth_backed, 108 * ONE); // ⌈2000 / 18.61⌉ = 108
        assert!(!p.mint_depth_cap);
        // BAYC: an expensive floor keeps rook_core's minimums
        let p = value_params(136_840_000_000, 4, 1, 20 * SOL).unwrap();
        assert_eq!((p.mint_per_hour, p.max_public_eth_backed, p.max_eth_backed), (5 * ONE, 10 * ONE, 40 * ONE));
        // Bitcoin Weirdos: one 174-wX curve fits
        let p = value_params(490_000_000, 174, 2, 20 * SOL).unwrap();
        assert_eq!(p.max_public_eth_backed, 522 * ONE);
        assert_eq!(p.max_eth_backed, 4_082 * ONE); // ⌈2000 / 0.49⌉
        assert_eq!(p.mint_per_hour, 511 * ONE);
        // the public cap never exceeds the total
        let p = value_params(100 * SOL, 500, 10, 0).unwrap();
        assert_eq!(p.max_eth_backed, p.max_public_eth_backed);
        assert!(value_params(0, 5, 1, 0).is_err());
    }

    #[test]
    fn the_defaults_are_rook_cores_and_valid() {
        let p = default_params(20 * SOL);
        validate_params(&p).unwrap();
        assert!(p.mint_depth_cap);
        let mut q = p;
        q.max_items_per_day = 33;
        assert_eq!(validate_params(&q), Err(BooksError::BadParams));
    }
}
