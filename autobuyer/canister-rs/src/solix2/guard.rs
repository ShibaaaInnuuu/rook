//! What Meteora does not check and Rook must (spikes S1–S3 §2.2, §5 item 9). DBC 0.2.1 accepts a quote mint with a freeze
//! authority, never re-reads the quote's mint authority, and accepts a threshold that does not match the curve. So the ceremony
//! (`rook-ceremony` reads back) and the canister (at install, and before trusting any pool under a config) enforce:
//!
//! - **the wX mint** ([`check_wx_mint`]): initialized, 9 decimals, mint authority = `K_coll[i]`, **freeze authority none**;
//! - **the config** ([`check_config`]): version 0, quote = wX[i], `fee_claimer` = `leftover_receiver` = `K_fc`, Rook's pinned
//!   fee schedule, LP permanently locked 50/50, no creation fee, no creator shares, fixed 1B supply, and **the threshold bound to
//!   the curve**: the curve pinned for `T_wX` and a raise over it from `sqrt_start_price` to the migration price equal to
//!   `T_wX` wX;
//! - **instructions the canister never signs** ([`refuse_forbidden`]): `SetAuthority` (the wX mint authority is never changed or
//!   handed on: a new authority would mint unbacked wX outside the books), `Approve(Checked)` (no delegate over a canister
//!   account), and any Metaplex instruction other than the create and the rename-only update.
use super::decode::{DbcConfig, Mint};
use super::{dbc::pins, id, mpl, spl, METADATA_PROGRAM, TOKEN_2022_PROGRAM};
use crate::soltx::{b58_encode, Instruction, Pubkey};

/// Refuses a transaction carrying an instruction the canister never signs. Called on every message before any signature.
pub fn refuse_forbidden(ixs: &[Instruction]) -> Result<(), String> {
    let (tok, tok22, mpl_p) = (super::token_program(), id(TOKEN_2022_PROGRAM), id(METADATA_PROGRAM));
    for (n, i) in ixs.iter().enumerate() {
        if i.program == tok || i.program == tok22 {
            match i.data.first() {
                Some(&spl::tag::SET_AUTHORITY) => return Err(format!("instruction {n}: SPL SetAuthority is never signed (the wX mint authority stays K_coll)")),
                Some(&spl::tag::APPROVE) | Some(&spl::tag::APPROVE_CHECKED) => return Err(format!("instruction {n}: SPL Approve is never signed")),
                _ => {}
            }
        }
        if i.program == mpl_p {
            match i.data.first() {
                Some(&mpl::CREATE_METADATA_ACCOUNT_V3) => {}
                // rename only (audit B2-02: parsed, not the tail): no update authority, no primary sale, no mutability change
                Some(&mpl::UPDATE_METADATA_ACCOUNT_V2) if mpl::is_rename_only(&i.data) => {}
                _ => return Err(format!("instruction {n}: a Metaplex instruction other than create or rename")),
            }
        }
    }
    Ok(())
}

/// The wX mint as the ceremony must leave it (§2), read back at finalized.
pub fn check_wx_mint(m: &Mint, k_coll: &Pubkey) -> Result<(), String> {
    let mut bad = vec![];
    if !m.is_initialized {
        bad.push("not initialized".to_string());
    }
    if m.decimals != 9 {
        bad.push(format!("decimals {} (want 9)", m.decimals));
    }
    if m.mint_authority != Some(*k_coll) {
        bad.push(format!("mint authority {:?} (want K_coll {})", m.mint_authority.map(|k| b58_encode(&k)), b58_encode(k_coll)));
    }
    if let Some(f) = m.freeze_authority {
        bad.push(format!("freeze authority {} (want none)", b58_encode(&f)));
    }
    if bad.is_empty() { Ok(()) } else { Err(format!("wX mint refused: {}", bad.join("; "))) }
}

/// What one collection's config is pinned to (the launch file).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigPin {
    pub wx_mint: Pubkey,
    pub k_fc: Pubkey,
    pub t_wx: u64,
    pub sqrt_start_price: u128,
    pub curve: Vec<(u128, u128)>,
}

/// The high 128 bits of a 128 × 128-bit product.
fn mul_hi(a: u128, b: u128) -> u128 {
    let (a0, a1) = (a as u64 as u128, a >> 64);
    let (b0, b1) = (b as u64 as u128, b >> 64);
    let (p00, p01, p10, p11) = (a0 * b0, a0 * b1, a1 * b0, a1 * b1);
    let mid = (p00 >> 64) + (p01 as u64 as u128) + (p10 as u64 as u128);
    p11 + (p01 >> 64) + (p10 >> 64) + (mid >> 64)
}

/// Quote raised by buying along the curve from `sqrt_start` to `sqrt_end`: Σ L_i · (√p_hi − √p_lo) / 2^128 over the segments
/// (segment i runs from the previous point, or the start, up to `curve[i].0` with liquidity `curve[i].1`), DBC's
/// `get_delta_amount_quote` rounded down. In quote base units.
pub fn quote_raised(sqrt_start: u128, curve: &[(u128, u128)], sqrt_end: u128) -> u128 {
    let mut lo = sqrt_start;
    let mut total = 0u128;
    for (hi, l) in curve {
        if lo >= sqrt_end {
            break;
        }
        let top = (*hi).min(sqrt_end);
        if top > lo {
            total = total.saturating_add(mul_hi(*l, top - lo));
        }
        lo = *hi;
    }
    total
}

/// A config under which the canister treats pools as Rook's (§3.1), else the list of mismatches. `quote_raised` over the decoded
/// curve must equal the threshold to within 0.1 % (DBC derives the migration price from the threshold; a threshold that is not
/// the curve's own puts the migration price somewhere else on it).
pub fn check_config(c: &DbcConfig, pin: &ConfigPin) -> Result<(), String> {
    let mut bad: Vec<String> = vec![];
    let mut want = |ok: bool, what: String| {
        if !ok {
            bad.push(what);
        }
    };
    // the config's layout version: 0 as DBC 0.2.1 wrote it (the spikes), 1 as DBC writes it since (read on devnet 2026-10-12, the W15
    // rehearsal: the same 1,048-byte layout, every field Rook pins identical; only this byte moved). Anything else is a layout this
    // decoder has not been checked against.
    want(c.version <= 1, format!("version {} (0 or 1: the layouts checked)", c.version));
    want(c.quote_mint == pin.wx_mint, format!("quote {} is not wX", b58_encode(&c.quote_mint)));
    want(c.fee_claimer == pin.k_fc, format!("fee_claimer {} is not K_fc", b58_encode(&c.fee_claimer)));
    want(c.leftover_receiver == pin.k_fc, format!("leftover_receiver {} is not K_fc", b58_encode(&c.leftover_receiver)));
    want(c.activation_type == pins::ACTIVATION_TYPE_TIMESTAMP, format!("activation_type {}", c.activation_type));
    want(c.collect_fee_mode == pins::COLLECT_FEE_MODE_QUOTE, format!("collect_fee_mode {}", c.collect_fee_mode));
    want(c.migration_option == pins::MIGRATION_OPTION_DAMM_V2, format!("migration_option {}", c.migration_option));
    want(c.token_type == pins::TOKEN_TYPE_SPL, format!("token_type {}", c.token_type));
    want(c.token_decimal == pins::COIN_DECIMALS, format!("token_decimal {}", c.token_decimal));
    want(c.migration_fee_option == pins::MIGRATION_FEE_OPTION_CUSTOMIZABLE, format!("migration_fee_option {}", c.migration_fee_option));
    want(c.partner_permanent_locked_liquidity_percentage == 50 && c.creator_permanent_locked_liquidity_percentage == 50
        && c.partner_liquidity_percentage == 0 && c.creator_liquidity_percentage == 0, "LP is not permanently locked 50/50".into());
    want(c.partner_vesting_initialized == 0 && c.creator_vesting_initialized == 0 && c.locked_vesting_amount_per_period == 0
        && c.locked_vesting_cliff_unlock_amount == 0, "vesting configured".into());
    want(c.pool_creation_fee == 0, format!("pool_creation_fee {}", c.pool_creation_fee));
    want(c.creator_trading_fee_percentage == 0, format!("creator_trading_fee_percentage {}", c.creator_trading_fee_percentage));
    want(c.migration_fee_percentage == pins::MIGRATION_FEE_PERCENTAGE, format!("migration_fee_percentage {}", c.migration_fee_percentage));
    want(c.creator_migration_fee_percentage == 0, format!("creator_migration_fee_percentage {}", c.creator_migration_fee_percentage));
    want(c.fixed_token_supply_flag == 1 && c.pre_migration_token_supply == pins::TOTAL_SUPPLY && c.post_migration_token_supply == pins::TOTAL_SUPPLY,
        "supply is not the fixed 1B".into());
    want(c.token_update_authority == pins::TOKEN_UPDATE_AUTHORITY_IMMUTABLE, format!("token_update_authority {}", c.token_update_authority));
    want(c.cliff_fee_numerator == pins::FEE_CLIFF && c.fee_first_factor == pins::FEE_PERIODS && c.fee_second_factor == pins::FEE_PERIOD_SECS
        && c.fee_third_factor == pins::FEE_REDUCTION && c.base_fee_mode == 0 && c.dynamic_fee_initialized == 0, "fee schedule is not Rook's".into());
    want(c.migrated_collect_fee_mode == 0 && c.migrated_pool_fee_bps == pins::MIGRATED_POOL_FEE_BPS && c.migrated_compounding_fee_bps == 0
        && c.migrated_dynamic_fee == 0 && c.migrated_pool_base_fee_mode == 0, "migrated pool fee is not Rook's".into());
    // the curve's first swap: on (the creator's launch buy pays the minimum fee, `pins::FIRST_SWAP_WITH_MIN_FEE`); a config without it
    // is not the one `rook_wx_config` creates
    want(c.enable_first_swap_with_min_fee == pins::FIRST_SWAP_WITH_MIN_FEE as u8,
        format!("enable_first_swap_with_min_fee {} (want {})", c.enable_first_swap_with_min_fee, pins::FIRST_SWAP_WITH_MIN_FEE as u8));
    // the threshold, bound to the curve
    let threshold = pin.t_wx.saturating_mul(pins::WX_ONE);
    want(c.migration_quote_threshold == threshold, format!("threshold {} (want T_wX {} = {threshold})", c.migration_quote_threshold, pin.t_wx));
    want(c.sqrt_start_price == pin.sqrt_start_price && c.curve == pin.curve, "curve is not the one pinned for T_wX".into());
    let raised = quote_raised(c.sqrt_start_price, &c.curve, c.migration_sqrt_price);
    let t = c.migration_quote_threshold as u128;
    want(raised.abs_diff(t) <= t / 1_000, format!("the curve raises {raised} by its migration price, not the threshold {t}"));
    want(c.curve.first().map(|p| p.0) == Some(c.migration_sqrt_price), "the migration price is not the curve's first point".into());
    if bad.is_empty() { Ok(()) } else { Err(format!("DBC config refused: {}", bad.join("; "))) }
}
