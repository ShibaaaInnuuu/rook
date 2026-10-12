//! Meteora Dynamic Bonding Curve 0.2.1 (`dbcij3…`), by hand from the official IDL: account lists in IDL order with its
//! writable/signer flags, an absent optional account as the program id (Anchor's convention), data = `sha256("global:<name>")[..8]`
//! ‖ Borsh of the arguments.
use super::{dbc_pool_address, dbc_vault_address, event_authority, id, ix, metadata_address, ro, rw, signer, DBC_POOL_AUTHORITY, DBC_PROGRAM,
    DAMM_CONFIG_CUSTOMIZABLE, DAMM_POOL_AUTHORITY, DAMM_PROGRAM, METADATA_PROGRAM, TOKEN_2022_PROGRAM};
use crate::soltx::{AccountMeta, Borsh, Instruction, Pubkey, SYSTEM_PROGRAM};

pub fn program() -> Pubkey {
    id(DBC_PROGRAM)
}
fn u128le(b: Borsh, v: u128) -> Borsh {
    b.bytes(&v.to_le_bytes())
}

// ---- create_config ------------------------------------------------------------------------------------------------------------

/// `BaseFeeParameters` (the curve's fee scheduler).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BaseFee {
    pub cliff_fee_numerator: u64,
    pub first_factor: u16,
    pub second_factor: u64,
    pub third_factor: u64,
    pub base_fee_mode: u8,
}
/// `DynamicFeeParameters`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DynamicFee {
    pub bin_step: u16,
    pub bin_step_u128: u128,
    pub filter_period: u16,
    pub decay_period: u16,
    pub reduction_factor: u16,
    pub max_volatility_accumulator: u32,
    pub variable_fee_control: u32,
}
/// `LockedVestingParams`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LockedVesting {
    pub amount_per_period: u64,
    pub cliff_duration_from_migration_time: u64,
    pub frequency: u64,
    pub number_of_period: u64,
    pub cliff_unlock_amount: u64,
}
/// `LiquidityVestingInfoParams`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LiquidityVesting {
    pub vesting_percentage: u8,
    pub bps_per_period: u16,
    pub number_of_periods: u16,
    pub cliff_duration_from_migration_time: u32,
    pub frequency: u32,
}
/// `MigratedPoolMarketCapFeeSchedulerParams`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MarketCapFeeScheduler {
    pub number_of_period: u16,
    pub sqrt_price_step_bps: u16,
    pub scheduler_expiration_duration: u32,
    pub reduction_factor: u64,
}

/// DBC 0.2.1 `ConfigParameters`, field for field.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigParameters {
    pub base_fee: BaseFee,
    pub dynamic_fee: Option<DynamicFee>,
    pub collect_fee_mode: u8,
    pub migration_option: u8,
    pub activation_type: u8,
    pub token_type: u8,
    pub token_decimal: u8,
    pub partner_liquidity_percentage: u8,
    pub partner_permanent_locked_liquidity_percentage: u8,
    pub creator_liquidity_percentage: u8,
    pub creator_permanent_locked_liquidity_percentage: u8,
    pub migration_quote_threshold: u64,
    pub sqrt_start_price: u128,
    pub locked_vesting: LockedVesting,
    pub migration_fee_option: u8,
    /// (pre-migration, post-migration) supply.
    pub token_supply: Option<(u64, u64)>,
    pub creator_trading_fee_percentage: u8,
    pub token_update_authority: u8,
    /// `MigrationFee { fee_percentage, creator_fee_percentage }`.
    pub migration_fee_percentage: u8,
    pub creator_migration_fee_percentage: u8,
    /// `MigratedPoolFee { collect_fee_mode, dynamic_fee, pool_fee_bps }`.
    pub migrated_collect_fee_mode: u8,
    pub migrated_dynamic_fee: u8,
    pub migrated_pool_fee_bps: u16,
    pub pool_creation_fee: u64,
    pub partner_liquidity_vesting: LiquidityVesting,
    pub creator_liquidity_vesting: LiquidityVesting,
    pub migrated_pool_base_fee_mode: u8,
    pub migrated_pool_market_cap_fee_scheduler: MarketCapFeeScheduler,
    pub enable_first_swap_with_min_fee: bool,
    pub compounding_fee_bps: u16,
    /// `(sqrt_price, liquidity)` per segment, at most 20.
    pub curve: Vec<(u128, u128)>,
}

impl ConfigParameters {
    pub fn encode(&self, mut b: Borsh) -> Borsh {
        let f = &self.base_fee;
        b = b.u64(f.cliff_fee_numerator).u16(f.first_factor).u64(f.second_factor).u64(f.third_factor).u8(f.base_fee_mode);
        b = match &self.dynamic_fee {
            None => b.u8(0),
            Some(d) => u128le(b.u8(1).u16(d.bin_step), d.bin_step_u128).u16(d.filter_period).u16(d.decay_period).u16(d.reduction_factor)
                .u32(d.max_volatility_accumulator).u32(d.variable_fee_control),
        };
        b = b.u8(self.collect_fee_mode).u8(self.migration_option).u8(self.activation_type).u8(self.token_type).u8(self.token_decimal)
            .u8(self.partner_liquidity_percentage).u8(self.partner_permanent_locked_liquidity_percentage).u8(self.creator_liquidity_percentage)
            .u8(self.creator_permanent_locked_liquidity_percentage).u64(self.migration_quote_threshold);
        b = u128le(b, self.sqrt_start_price);
        let v = &self.locked_vesting;
        b = b.u64(v.amount_per_period).u64(v.cliff_duration_from_migration_time).u64(v.frequency).u64(v.number_of_period).u64(v.cliff_unlock_amount)
            .u8(self.migration_fee_option);
        b = match self.token_supply {
            None => b.u8(0),
            Some((pre, post)) => b.u8(1).u64(pre).u64(post),
        };
        b = b.u8(self.creator_trading_fee_percentage).u8(self.token_update_authority).u8(self.migration_fee_percentage)
            .u8(self.creator_migration_fee_percentage).u8(self.migrated_collect_fee_mode).u8(self.migrated_dynamic_fee).u16(self.migrated_pool_fee_bps)
            .u64(self.pool_creation_fee);
        for l in [&self.partner_liquidity_vesting, &self.creator_liquidity_vesting] {
            b = b.u8(l.vesting_percentage).u16(l.bps_per_period).u16(l.number_of_periods).u32(l.cliff_duration_from_migration_time).u32(l.frequency);
        }
        let m = &self.migrated_pool_market_cap_fee_scheduler;
        b = b.u8(self.migrated_pool_base_fee_mode).u16(m.number_of_period).u16(m.sqrt_price_step_bps).u32(m.scheduler_expiration_duration)
            .u64(m.reduction_factor).bool(self.enable_first_swap_with_min_fee).u16(self.compounding_fee_bps).bytes(&[0, 0]).u32(self.curve.len() as u32);
        for (s, l) in &self.curve {
            b = u128le(u128le(b, *s), *l);
        }
        b
    }
}

/// Rook's pinned config constants (spike Q1, `solana/tests/harness` `rook_config_params(50)`; unchanged by canister-only, §3.1).
pub mod pins {
    /// Anti-sniper: linear scheduler from 50 %, −0.8166666 points every 10 s for 60 periods (≈ 1 % after 10 minutes).
    pub const FEE_CLIFF: u64 = 500_000_000;
    pub const FEE_PERIODS: u16 = 60;
    pub const FEE_PERIOD_SECS: u64 = 10;
    pub const FEE_REDUCTION: u64 = 8_166_666;
    /// The creator's launch buy (operator decision 2026-10-11, pump.fun's "dev buy"): DBC's `enable_first_swap_with_min_fee`. The
    /// pool's FIRST swap, made at the top level of the same transaction as the `initialize_virtual_pool_*` that created that pool, with
    /// the Instructions sysvar as its one remaining account and no referral, pays the scheduler's minimum fee (its end fee,
    /// `FEE_CLIFF − FEE_PERIODS·FEE_REDUCTION` = 10,000,040 / 1e9 ≈ 1 %) instead of the anti-sniper fee; every other swap pays the
    /// schedule (DBC 0.2.1 `process_swap.rs` `validate_contain_initialize_pool_ix_and_no_cpi`, `VirtualPool::is_first_swap`).
    pub const FIRST_SWAP_WITH_MIN_FEE: bool = true;
    /// That minimum fee, the scheduler's value after its last period (numerator over 1e9).
    pub const FEE_MIN: u64 = FEE_CLIFF - FEE_PERIODS as u64 * FEE_REDUCTION;
    /// 1B coins of 6 decimals, fixed before and after migration.
    pub const TOTAL_SUPPLY: u64 = 1_000_000_000_000_000;
    pub const COIN_DECIMALS: u8 = 6;
    /// 50 % of the raise to the partner (K_fc), in wX.
    pub const MIGRATION_FEE_PERCENTAGE: u8 = 50;
    /// The migrated pool's fee, 1 %.
    pub const MIGRATED_POOL_FEE_BPS: u16 = 100;
    /// DBC enums.
    pub const COLLECT_FEE_MODE_QUOTE: u8 = 0;
    pub const MIGRATION_OPTION_DAMM_V2: u8 = 1;
    pub const ACTIVATION_TYPE_TIMESTAMP: u8 = 1;
    pub const TOKEN_TYPE_SPL: u8 = 0;
    pub const MIGRATION_FEE_OPTION_CUSTOMIZABLE: u8 = 6;
    pub const TOKEN_UPDATE_AUTHORITY_IMMUTABLE: u8 = 1;
    /// wX has 9 decimals, so a whole wX is 10⁹ base units, as a SOL is.
    pub const WX_ONE: u64 = 1_000_000_000;
}

/// Rook's config for collection i: the Q1 parameters, the threshold `T_wX` whole wX, and the curve `design_curve.mjs 1e9 20
/// <T_wX> 50 6` produced for that threshold (pinned per collection in the launch file; `guard::check_config` binds the two).
pub fn rook_wx_config(t_wx: u64, sqrt_start_price: u128, curve: Vec<(u128, u128)>) -> ConfigParameters {
    use pins::*;
    ConfigParameters {
        base_fee: BaseFee { cliff_fee_numerator: FEE_CLIFF, first_factor: FEE_PERIODS, second_factor: FEE_PERIOD_SECS, third_factor: FEE_REDUCTION, base_fee_mode: 0 },
        dynamic_fee: None,
        collect_fee_mode: COLLECT_FEE_MODE_QUOTE,
        migration_option: MIGRATION_OPTION_DAMM_V2,
        activation_type: ACTIVATION_TYPE_TIMESTAMP,
        token_type: TOKEN_TYPE_SPL,
        token_decimal: COIN_DECIMALS,
        partner_liquidity_percentage: 0,
        partner_permanent_locked_liquidity_percentage: 50,
        creator_liquidity_percentage: 0,
        creator_permanent_locked_liquidity_percentage: 50,
        migration_quote_threshold: t_wx * WX_ONE,
        sqrt_start_price,
        locked_vesting: LockedVesting::default(),
        migration_fee_option: MIGRATION_FEE_OPTION_CUSTOMIZABLE,
        token_supply: Some((TOTAL_SUPPLY, TOTAL_SUPPLY)),
        creator_trading_fee_percentage: 0,
        token_update_authority: TOKEN_UPDATE_AUTHORITY_IMMUTABLE,
        migration_fee_percentage: MIGRATION_FEE_PERCENTAGE,
        creator_migration_fee_percentage: 0,
        migrated_collect_fee_mode: 0,
        migrated_dynamic_fee: 0,
        migrated_pool_fee_bps: MIGRATED_POOL_FEE_BPS,
        pool_creation_fee: 0,
        partner_liquidity_vesting: LiquidityVesting::default(),
        creator_liquidity_vesting: LiquidityVesting::default(),
        migrated_pool_base_fee_mode: 0,
        migrated_pool_market_cap_fee_scheduler: MarketCapFeeScheduler::default(),
        enable_first_swap_with_min_fee: FIRST_SWAP_WITH_MIN_FEE,
        compounding_fee_bps: 0,
        curve,
    }
}

/// `create_config`: the config keypair and the payer sign (the ceremony's step; the canister never pays for configs).
pub fn create_config(config: Pubkey, fee_claimer: Pubkey, leftover_receiver: Pubkey, quote_mint: Pubkey, payer: Pubkey, params: &ConfigParameters) -> Instruction {
    let p = program();
    ix(p, vec![signer(config, true), ro(fee_claimer), ro(leftover_receiver), ro(quote_mint), signer(payer, true), ro(SYSTEM_PROGRAM),
        ro(event_authority(&p)), ro(p)], params.encode(Borsh::ix("create_config")).done())
}

// ---- a coin's pool --------------------------------------------------------------------------------------------------------------

/// The addresses of one coin's curve under a config.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Curve {
    pub config: Pubkey,
    pub pool: Pubkey,
    pub base_mint: Pubkey,
    pub quote_mint: Pubkey,
    pub base_vault: Pubkey,
    pub quote_vault: Pubkey,
}

impl Curve {
    pub fn new(config: Pubkey, base_mint: Pubkey, quote_mint: Pubkey) -> Curve {
        let pool = dbc_pool_address(&config, &base_mint, &quote_mint);
        Curve { config, pool, base_mint, quote_mint, base_vault: dbc_vault_address(&base_mint, &pool), quote_vault: dbc_vault_address(&quote_mint, &pool) }
    }
}

/// `initialize_virtual_pool_with_spl_token`: a creator launches a coin (the coin mint signs).
pub fn initialize_virtual_pool_with_spl_token(c: &Curve, creator: Pubkey, payer: Pubkey, name: &str, symbol: &str, uri: &str) -> Instruction {
    let p = program();
    let tp = super::token_program();
    ix(p, vec![ro(c.config), ro(id(DBC_POOL_AUTHORITY)), signer(creator, false), signer(c.base_mint, true), ro(c.quote_mint), rw(c.pool),
        rw(c.base_vault), rw(c.quote_vault), rw(metadata_address(&c.base_mint)), ro(id(METADATA_PROGRAM)), signer(payer, true), ro(tp), ro(tp),
        ro(SYSTEM_PROGRAM), ro(event_authority(&p)), ro(p)],
        Borsh::ix("initialize_virtual_pool_with_spl_token").string(name).string(symbol).string(uri).done())
}

fn swap_accounts(c: &Curve, payer: Pubkey, input: Pubkey, output: Pubkey, referral: Option<Pubkey>) -> Vec<AccountMeta> {
    let p = program();
    let tp = super::token_program();
    vec![ro(id(DBC_POOL_AUTHORITY)), ro(c.config), rw(c.pool), rw(input), rw(output), rw(c.base_vault), rw(c.quote_vault), ro(c.base_mint),
        ro(c.quote_mint), signer(payer, false), ro(tp), ro(tp), referral.map(rw).unwrap_or(ro(p)), ro(event_authority(&p)), ro(p)]
}

/// `swap { amount_in, minimum_amount_out }` (exact in). Direction follows the accounts: quote → base is a buy.
pub fn swap(c: &Curve, payer: Pubkey, input: Pubkey, output: Pubkey, amount_in: u64, minimum_amount_out: u64) -> Instruction {
    ix(program(), swap_accounts(c, payer, input, output, None), Borsh::ix("swap").u64(amount_in).u64(minimum_amount_out).done())
}

/// `swap2 { amount_0, amount_1, swap_mode }`: mode 0 exact in (`amount_0` in, `amount_1` minimum out), 1 partial fill (stops at the
/// migration threshold), 2 exact out (`amount_0` out, `amount_1` maximum in).
pub fn swap2(c: &Curve, payer: Pubkey, input: Pubkey, output: Pubkey, amount_0: u64, amount_1: u64, swap_mode: u8) -> Instruction {
    ix(program(), swap_accounts(c, payer, input, output, None), Borsh::ix("swap2").u64(amount_0).u64(amount_1).u8(swap_mode).done())
}

// ---- the partner's (K_fc's) instructions (§3.3) ----------------------------------------------------------------------------------

/// `claim_trading_fee`: the partner's curve fees, base to `token_a_account`, quote (wX) to `token_b_account`. `K_fc` signs.
pub fn claim_trading_fee(c: &Curve, fee_claimer: Pubkey, token_a_account: Pubkey, token_b_account: Pubkey, max_amount_a: u64, max_amount_b: u64) -> Instruction {
    let p = program();
    let tp = super::token_program();
    ix(p, vec![ro(id(DBC_POOL_AUTHORITY)), ro(c.config), rw(c.pool), rw(token_a_account), rw(token_b_account), rw(c.base_vault), rw(c.quote_vault),
        ro(c.base_mint), ro(c.quote_mint), signer(fee_claimer, false), ro(tp), ro(tp), ro(event_authority(&p)), ro(p)],
        Borsh::ix("claim_trading_fee").u64(max_amount_a).u64(max_amount_b).done())
}

/// `withdraw_migration_fee(flag)`: flag 0 = the partner's share (`sender` = `K_fc`), into any wX account (the coin's vault ATA).
pub fn withdraw_migration_fee(c: &Curve, sender: Pubkey, token_quote_account: Pubkey, flag: u8) -> Instruction {
    let p = program();
    ix(p, vec![ro(id(DBC_POOL_AUTHORITY)), ro(c.config), rw(c.pool), rw(token_quote_account), rw(c.quote_vault), ro(c.quote_mint), signer(sender, false),
        ro(super::token_program()), ro(event_authority(&p)), ro(p)], Borsh::ix("withdraw_migration_fee").u8(flag).done())
}

/// `partner_withdraw_surplus`: quote above the threshold left by the last buy (0 after a partial fill). `K_fc` signs.
pub fn partner_withdraw_surplus(c: &Curve, fee_claimer: Pubkey, token_quote_account: Pubkey) -> Instruction {
    let p = program();
    ix(p, vec![ro(id(DBC_POOL_AUTHORITY)), ro(c.config), rw(c.pool), rw(token_quote_account), rw(c.quote_vault), ro(c.quote_mint), signer(fee_claimer, false),
        ro(super::token_program()), ro(event_authority(&p)), ro(p)], Borsh::ix("partner_withdraw_surplus").done())
}

/// `withdraw_leftover`: permissionless; the unsold curve coins go to the leftover receiver's (`K_fc`'s) coin ATA.
pub fn withdraw_leftover(c: &Curve, leftover_receiver: Pubkey) -> Instruction {
    let p = program();
    let tp = super::token_program();
    let dest = crate::soltx::associated_token_address(&leftover_receiver, &c.base_mint);
    ix(p, vec![ro(id(DBC_POOL_AUTHORITY)), ro(c.config), rw(c.pool), rw(dest), rw(c.base_vault), ro(c.base_mint), ro(leftover_receiver), ro(tp),
        ro(event_authority(&p)), ro(p)], Borsh::ix("withdraw_leftover").done())
}

/// `migration_damm_v2`: permissionless; a graduated curve becomes a DAMM v2 COIN/wX pool on the Customizable config, two LP
/// positions (the partner's and the creator's) whose NFT mints sign. `DAMM_CONFIG_CUSTOMIZABLE` follows as a remaining account.
pub fn migration_damm_v2(c: &Curve, payer: Pubkey, first_nft_mint: Pubkey, second_nft_mint: Pubkey) -> Instruction {
    use super::{damm_config_pool_address, damm_position_address, damm_position_nft_account, damm_vault_address};
    let damm_config = id(DAMM_CONFIG_CUSTOMIZABLE);
    let damm = id(DAMM_PROGRAM);
    let pool = damm_config_pool_address(&damm_config, &c.base_mint, &c.quote_mint);
    let tp = super::token_program();
    ix(program(), vec![
        rw(c.pool),
        ro(SYSTEM_PROGRAM), // migration_metadata: unused by the DAMM v2 path (as the spike sent it)
        ro(c.config),
        rw(id(DBC_POOL_AUTHORITY)),
        rw(pool),
        signer(first_nft_mint, true),
        rw(damm_position_nft_account(&first_nft_mint)),
        rw(damm_position_address(&first_nft_mint)),
        signer(second_nft_mint, true),
        rw(damm_position_nft_account(&second_nft_mint)),
        rw(damm_position_address(&second_nft_mint)),
        ro(id(DAMM_POOL_AUTHORITY)),
        ro(damm),
        rw(c.base_mint),
        rw(c.quote_mint),
        rw(damm_vault_address(&c.base_mint, &pool)),
        rw(damm_vault_address(&c.quote_mint, &pool)),
        rw(c.base_vault),
        rw(c.quote_vault),
        signer(payer, true),
        ro(tp),
        ro(tp),
        ro(id(TOKEN_2022_PROGRAM)),
        ro(event_authority(&damm)),
        ro(SYSTEM_PROGRAM),
        ro(damm_config),
    ], Borsh::ix("migration_damm_v2").done())
}
