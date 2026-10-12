//! The floor module's parameters (FloorModule's immutables), fixed at `init_module`.

use anchor_lang::prelude::*;

use crate::units::ONE;

/// The floor module's parameters: every one fixed at `init_module` (FloorModule's immutables; no setter exists).
/// Rentals (`rentTerm`, `premiumBps`, `rentShareBps`, `maxRentals`) are dropped with rentals.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, Debug, PartialEq, Eq, InitSpace)]
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
    pub max_public_eth_backed: u64,
    pub max_eth_backed: u64,
    /// `revenueTarget`, lamports: the 1 ETH of the EVM, converted by the launch file at the live SOL/ETH rate (D-c).
    pub revenue_target: u64,
}

/// `StackChecks.defaultParams(1e18, stage 0)` (script/StackChecks.sol:171-195) in 9-decimal units, with `revenueTarget` (1 ETH
/// in the EVM) given in lamports by the caller: the launch file's value (`ops/solana/launch-params.mjs`), never a rate fixed
/// here (D-c). Stage 0's `maxItemsPerDay` is 3 (StackChecks:53-58). The unit amounts (`5 · ONE`…) are counts of items, not money.
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
    }
}
