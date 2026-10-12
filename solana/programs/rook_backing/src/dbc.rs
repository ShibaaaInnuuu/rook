//! Raw readers for the two Meteora DBC accounts `rook_backing` trusts: `PoolConfig` and `VirtualPool`.
//!
//! Offsets are those of DBC 0.2.1 (`state/config.rs`, `state/virtual_pool.rs`; both `#[account(zero_copy)]`, so the layout
//! is the `#[repr(C)]` struct after the 8-byte discriminator). The integration tests read the same fields from accounts the
//! real (mainnet-dumped) DBC program wrote, so a layout change in a DBC upgrade fails the tests, not production silently.
//! Every reader checks owner, length and discriminator first.

use anchor_lang::prelude::*;

use crate::{constants::DBC_ID, error::BackingError};

pub const POOL_CONFIG_DISCRIMINATOR: [u8; 8] = [26, 108, 14, 123, 116, 230, 129, 43];
pub const VIRTUAL_POOL_DISCRIMINATOR: [u8; 8] = [213, 224, 5, 209, 98, 69, 119, 92];
/// 8 + `PoolConfig::INIT_SPACE` (1040).
pub const POOL_CONFIG_LEN: usize = 8 + 1040;
/// 8 + `PoolState::INIT_SPACE` (416).
pub const VIRTUAL_POOL_LEN: usize = 8 + 416;

/// DBC `MigrationOption::DammV2`.
pub const MIGRATION_OPTION_DAMM_V2: u8 = 1;
/// DBC `TokenType::SplToken`.
pub const TOKEN_TYPE_SPL: u8 = 0;
/// DBC `CollectFeeMode::QuoteToken`.
pub const COLLECT_FEE_MODE_QUOTE: u8 = 0;
/// DBC `ActivationType::Timestamp` (the pool's `activation_point` is then unix seconds).
pub const ACTIVATION_TYPE_TIMESTAMP: u8 = 1;
/// DBC `MigratedPoolCollectFeeMode` 0: the DAMM v2 pool collects its fees in the quote token only (SOL; no base-token fees, no
/// compounding).
pub const MIGRATED_COLLECT_FEE_MODE_QUOTE: u8 = 0;
/// `PoolConfig.version` as DBC 0.2.1 writes it (`create_config`). The offsets below are 0.2.1's: a config of another version
/// may lay them out differently and is refused (audit A3-05).
pub const DBC_CONFIG_VERSION: u8 = 0;
/// DBC `MigrationProgress::CreatedPool`.
pub const MIGRATION_PROGRESS_CREATED_POOL: u8 = 3;

/// The fields of a DBC `PoolConfig` that Rook checks before trusting a config.
pub struct DbcConfig {
    pub quote_mint: Pubkey,
    pub fee_claimer: Pubkey,
    pub leftover_receiver: Pubkey,
    pub collect_fee_mode: u8,
    pub migration_option: u8,
    pub activation_type: u8,
    pub token_type: u8,
    pub partner_permanent_locked_liquidity_percentage: u8,
    pub partner_liquidity_percentage: u8,
    pub creator_permanent_locked_liquidity_percentage: u8,
    pub creator_liquidity_percentage: u8,
    pub migration_fee_option: u8,
    pub creator_trading_fee_percentage: u8,
    pub migration_fee_percentage: u8,
    pub creator_migration_fee_percentage: u8,
    pub migration_quote_threshold: u64,
    pub version: u8,
    pub fixed_token_supply_flag: u8,
    /// `partner_liquidity_vesting_info.is_initialized` / `creator_liquidity_vesting_info.is_initialized`.
    pub partner_vesting_initialized: u8,
    pub creator_vesting_initialized: u8,
    pub pre_migration_token_supply: u64,
    pub post_migration_token_supply: u64,
    /// The migrated DAMM v2 pool's `collect_fee_mode` and compounding fee.
    pub migrated_collect_fee_mode: u8,
    pub migrated_compounding_fee_bps: u16,
    pub pool_creation_fee: u64,
}

/// The fields of a DBC `VirtualPool` that Rook reads.
pub struct DbcPool {
    pub config: Pubkey,
    pub creator: Pubkey,
    pub base_mint: Pubkey,
    pub base_vault: Pubkey,
    pub quote_vault: Pubkey,
    pub quote_reserve: u64,
    /// When trading opens, in the config's activation unit. DBC 0.2.1 sets it to the current point when the pool is
    /// created (`initialize_virtual_pool_*` takes no activation argument), so for a timestamp config it is the launch time.
    pub activation_point: u64,
    pub is_migrated: u8,
    pub migration_progress: u8,
    pub migration_fee_withdraw_status: u8,
}

fn pk(d: &[u8], at: usize) -> Pubkey {
    Pubkey::new_from_array(d[at..at + 32].try_into().unwrap())
}
fn u64_at(d: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(d[at..at + 8].try_into().unwrap())
}

pub fn read_config(info: &AccountInfo) -> Result<DbcConfig> {
    parse_config(info.owner, &info.try_borrow_data()?)
}

pub fn read_pool(info: &AccountInfo) -> Result<DbcPool> {
    parse_pool(info.owner, &info.try_borrow_data()?)
}

pub fn parse_config(owner: &Pubkey, d: &[u8]) -> Result<DbcConfig> {
    require_keys_eq!(*owner, DBC_ID, BackingError::NotDbcAccount);
    require!(
        d.len() == POOL_CONFIG_LEN && d[..8] == POOL_CONFIG_DISCRIMINATOR,
        BackingError::NotDbcConfig
    );
    Ok(DbcConfig {
        quote_mint: pk(d, 8),
        fee_claimer: pk(d, 40),
        leftover_receiver: pk(d, 72),
        // 104..184 pool_fees, 184..216 liquidity vesting infos (16 bytes each, `is_initialized` first), 216..232 padding
        partner_vesting_initialized: d[184],
        creator_vesting_initialized: d[200],
        collect_fee_mode: d[232],
        migration_option: d[233],
        activation_type: d[234],
        version: d[236],
        token_type: d[237],
        partner_permanent_locked_liquidity_percentage: d[239],
        partner_liquidity_percentage: d[240],
        creator_permanent_locked_liquidity_percentage: d[241],
        creator_liquidity_percentage: d[242],
        migration_fee_option: d[243],
        fixed_token_supply_flag: d[244],
        creator_trading_fee_percentage: d[245],
        migration_fee_percentage: d[247],
        creator_migration_fee_percentage: d[248],
        migration_quote_threshold: u64_at(d, 264),
        // 272 migration_base_threshold, 280 migration_sqrt_price, 296..344 locked vesting
        pre_migration_token_supply: u64_at(d, 344),
        post_migration_token_supply: u64_at(d, 352),
        migrated_collect_fee_mode: d[360],
        // 361 migrated_dynamic_fee, 362 migrated_pool_fee_bps, 364 base fee mode, 365 first swap with min fee
        migrated_compounding_fee_bps: u16::from_le_bytes([d[366], d[367]]),
        pool_creation_fee: u64_at(d, 368),
    })
}

pub fn parse_pool(owner: &Pubkey, d: &[u8]) -> Result<DbcPool> {
    require_keys_eq!(*owner, DBC_ID, BackingError::NotDbcAccount);
    // A TransferHookPool has another discriminator and is refused: Rook configs are plain SPL.
    require!(
        d.len() == VIRTUAL_POOL_LEN && d[..8] == VIRTUAL_POOL_DISCRIMINATOR,
        BackingError::NotDbcPool
    );
    // 8..72 volatility tracker
    Ok(DbcPool {
        config: pk(d, 72),
        creator: pk(d, 104),
        base_mint: pk(d, 136),
        base_vault: pk(d, 168),
        quote_vault: pk(d, 200),
        quote_reserve: u64_at(d, 240),
        // 248..280 protocol/partner fees, 280..296 sqrt_price
        activation_point: u64_at(d, 296),
        is_migrated: d[305],
        migration_progress: d[308],
        migration_fee_withdraw_status: d[311],
    })
}
