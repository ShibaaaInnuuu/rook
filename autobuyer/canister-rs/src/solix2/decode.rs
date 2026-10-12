//! The accounts the canister reads, decoded from raw bytes with an owner, length and discriminator check first; a short or foreign
//! account is an `Err`, never a panic.
//!
//! - SPL `Mint` (82 bytes) and token `Account` (165), `COption` as a u32 tag + the value (`spl_token::state`'s `Pack`).
//! - DBC 0.2.1 `PoolConfig` (1,048) and `VirtualPool` (424): zero-copy `#[repr(C)]` with explicit padding, so the offsets are the
//!   IDL's fields laid end to end after the 8-byte discriminator (the same offsets `rook_backing::dbc` reads).
//! - DAMM v2 0.2.4 `Pool` (1,112) and `Position` (408), the same way.
//!
//! Checked against accounts the mainnet programs wrote in LiteSVM (`tests/fixtures/solix2/flow.json`).
use super::{id, DAMM_PROGRAM, DBC_PROGRAM};
use crate::soltx::{self, Pubkey};

pub const MINT_LEN: usize = 82;
pub const TOKEN_ACCOUNT_LEN: usize = 165;
pub const DBC_CONFIG_LEN: usize = 1_048;
pub const DBC_POOL_LEN: usize = 424;
pub const DAMM_POOL_LEN: usize = 1_112;
pub const DAMM_POSITION_LEN: usize = 408;
/// `account:PoolConfig`, `account:VirtualPool`, `account:Pool`, `account:Position` (checked against `soltx::anchor_disc` in tests).
pub const DBC_CONFIG_DISC: [u8; 8] = [26, 108, 14, 123, 116, 230, 129, 43];
pub const DBC_POOL_DISC: [u8; 8] = [213, 224, 5, 209, 98, 69, 119, 92];
pub const DAMM_POOL_DISC: [u8; 8] = [241, 154, 109, 4, 17, 177, 109, 188];
pub const DAMM_POSITION_DISC: [u8; 8] = [170, 188, 143, 228, 122, 64, 247, 208];
/// DBC keeps up to 20 curve segments (unused ones are zero).
pub const MAX_CURVE_POINTS: usize = 20;

fn k(d: &[u8], at: usize) -> Pubkey {
    d[at..at + 32].try_into().expect("32")
}
fn u16_at(d: &[u8], at: usize) -> u16 {
    u16::from_le_bytes(d[at..at + 2].try_into().expect("2"))
}
fn u32_at(d: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(d[at..at + 4].try_into().expect("4"))
}
fn u64_at(d: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(d[at..at + 8].try_into().expect("8"))
}
fn u128_at(d: &[u8], at: usize) -> u128 {
    u128::from_le_bytes(d[at..at + 16].try_into().expect("16"))
}
fn coption_key(d: &[u8], at: usize) -> Result<Option<Pubkey>, String> {
    match u32_at(d, at) {
        0 => Ok(None),
        1 => Ok(Some(k(d, at + 4))),
        t => Err(format!("a COption tag of {t}")),
    }
}
fn coption_u64(d: &[u8], at: usize) -> Result<Option<u64>, String> {
    match u32_at(d, at) {
        0 => Ok(None),
        1 => Ok(Some(u64_at(d, at + 4))),
        t => Err(format!("a COption tag of {t}")),
    }
}
fn check(owner: &Pubkey, want_owner: &Pubkey, d: &[u8], len: usize, disc: Option<&[u8; 8]>, what: &str) -> Result<(), String> {
    if owner != want_owner {
        return Err(format!("{what}: owned by {}, not {}", soltx::b58_encode(owner), soltx::b58_encode(want_owner)));
    }
    if d.len() != len {
        return Err(format!("{what}: {} bytes, not {len}", d.len()));
    }
    if let Some(disc) = disc {
        if d[..8] != disc[..] {
            return Err(format!("{what}: wrong discriminator"));
        }
    }
    Ok(())
}

// ---- SPL Token --------------------------------------------------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mint {
    pub mint_authority: Option<Pubkey>,
    pub supply: u64,
    pub decimals: u8,
    pub is_initialized: bool,
    pub freeze_authority: Option<Pubkey>,
}

impl Mint {
    /// A classic SPL mint (owner = the Token program, exactly 82 bytes; a Token-2022 mint is refused).
    pub fn decode(owner: &Pubkey, d: &[u8]) -> Result<Mint, String> {
        check(owner, &super::token_program(), d, MINT_LEN, None, "SPL mint")?;
        let is_initialized = match d[45] {
            0 => false,
            1 => true,
            b => return Err(format!("SPL mint: is_initialized {b}")),
        };
        Ok(Mint { mint_authority: coption_key(d, 0)?, supply: u64_at(d, 36), decimals: d[44], is_initialized, freeze_authority: coption_key(d, 46)? })
    }
}

/// `AccountState`: 0 uninitialized, 1 initialized, 2 frozen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TokenAccount {
    pub mint: Pubkey,
    pub owner: Pubkey,
    pub amount: u64,
    pub delegate: Option<Pubkey>,
    pub state: u8,
    /// Some(rent-exempt reserve) for a wSOL account.
    pub is_native: Option<u64>,
    pub delegated_amount: u64,
    pub close_authority: Option<Pubkey>,
}

impl TokenAccount {
    pub fn decode(owner: &Pubkey, d: &[u8]) -> Result<TokenAccount, String> {
        check(owner, &super::token_program(), d, TOKEN_ACCOUNT_LEN, None, "SPL token account")?;
        if d[108] > 2 {
            return Err(format!("SPL token account: state {}", d[108]));
        }
        Ok(TokenAccount { mint: k(d, 0), owner: k(d, 32), amount: u64_at(d, 64), delegate: coption_key(d, 72)?, state: d[108],
            is_native: coption_u64(d, 109)?, delegated_amount: u64_at(d, 121), close_authority: coption_key(d, 129)? })
    }
}

// ---- DBC ----------------------------------------------------------------------------------------------------------------------------

/// DBC 0.2.1 `PoolConfig`: every field Rook checks (§3.1, `guard::check_config`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DbcConfig {
    pub quote_mint: Pubkey,
    pub fee_claimer: Pubkey,
    pub leftover_receiver: Pubkey,
    pub cliff_fee_numerator: u64,
    /// `second_factor`, `third_factor`, `first_factor`, `base_fee_mode` as stored.
    pub fee_second_factor: u64,
    pub fee_third_factor: u64,
    pub fee_first_factor: u16,
    pub base_fee_mode: u8,
    pub dynamic_fee_initialized: u8,
    pub partner_vesting_initialized: u8,
    pub creator_vesting_initialized: u8,
    pub collect_fee_mode: u8,
    pub migration_option: u8,
    pub activation_type: u8,
    pub token_decimal: u8,
    pub version: u8,
    pub token_type: u8,
    pub quote_token_flag: u8,
    pub partner_permanent_locked_liquidity_percentage: u8,
    pub partner_liquidity_percentage: u8,
    pub creator_permanent_locked_liquidity_percentage: u8,
    pub creator_liquidity_percentage: u8,
    pub migration_fee_option: u8,
    pub fixed_token_supply_flag: u8,
    pub creator_trading_fee_percentage: u8,
    pub token_update_authority: u8,
    pub migration_fee_percentage: u8,
    pub creator_migration_fee_percentage: u8,
    pub swap_base_amount: u64,
    pub migration_quote_threshold: u64,
    pub migration_base_threshold: u64,
    pub migration_sqrt_price: u128,
    pub locked_vesting_amount_per_period: u64,
    pub locked_vesting_cliff_unlock_amount: u64,
    pub pre_migration_token_supply: u64,
    pub post_migration_token_supply: u64,
    pub migrated_collect_fee_mode: u8,
    pub migrated_dynamic_fee: u8,
    pub migrated_pool_fee_bps: u16,
    pub migrated_pool_base_fee_mode: u8,
    pub enable_first_swap_with_min_fee: u8,
    pub migrated_compounding_fee_bps: u16,
    pub pool_creation_fee: u64,
    pub sqrt_start_price: u128,
    /// The used segments `(sqrt_price, liquidity)`, up to the first zero one.
    pub curve: Vec<(u128, u128)>,
}

impl DbcConfig {
    pub fn decode(owner: &Pubkey, d: &[u8]) -> Result<DbcConfig, String> {
        check(owner, &id(DBC_PROGRAM), d, DBC_CONFIG_LEN, Some(&DBC_CONFIG_DISC), "DBC PoolConfig")?;
        let curve = (0..MAX_CURVE_POINTS).map(|i| (u128_at(d, 408 + 32 * i), u128_at(d, 424 + 32 * i))).take_while(|(s, _)| *s != 0).collect();
        Ok(DbcConfig {
            quote_mint: k(d, 8),
            fee_claimer: k(d, 40),
            leftover_receiver: k(d, 72),
            cliff_fee_numerator: u64_at(d, 104),
            fee_second_factor: u64_at(d, 112),
            fee_third_factor: u64_at(d, 120),
            fee_first_factor: u16_at(d, 128),
            base_fee_mode: d[130],
            dynamic_fee_initialized: d[136],
            partner_vesting_initialized: d[184],
            creator_vesting_initialized: d[200],
            collect_fee_mode: d[232],
            migration_option: d[233],
            activation_type: d[234],
            token_decimal: d[235],
            version: d[236],
            token_type: d[237],
            quote_token_flag: d[238],
            partner_permanent_locked_liquidity_percentage: d[239],
            partner_liquidity_percentage: d[240],
            creator_permanent_locked_liquidity_percentage: d[241],
            creator_liquidity_percentage: d[242],
            migration_fee_option: d[243],
            fixed_token_supply_flag: d[244],
            creator_trading_fee_percentage: d[245],
            token_update_authority: d[246],
            migration_fee_percentage: d[247],
            creator_migration_fee_percentage: d[248],
            swap_base_amount: u64_at(d, 256),
            migration_quote_threshold: u64_at(d, 264),
            migration_base_threshold: u64_at(d, 272),
            migration_sqrt_price: u128_at(d, 280),
            locked_vesting_amount_per_period: u64_at(d, 296),
            locked_vesting_cliff_unlock_amount: u64_at(d, 328),
            pre_migration_token_supply: u64_at(d, 344),
            post_migration_token_supply: u64_at(d, 352),
            migrated_collect_fee_mode: d[360],
            migrated_dynamic_fee: d[361],
            migrated_pool_fee_bps: u16_at(d, 362),
            migrated_pool_base_fee_mode: d[364],
            enable_first_swap_with_min_fee: d[365],
            migrated_compounding_fee_bps: u16_at(d, 366),
            pool_creation_fee: u64_at(d, 368),
            sqrt_start_price: u128_at(d, 392),
            curve,
        })
    }
}

/// DBC 0.2.1 `VirtualPool`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DbcPool {
    pub config: Pubkey,
    pub creator: Pubkey,
    pub base_mint: Pubkey,
    pub base_vault: Pubkey,
    pub quote_vault: Pubkey,
    pub base_reserve: u64,
    pub quote_reserve: u64,
    pub protocol_base_fee: u64,
    pub protocol_quote_fee: u64,
    /// The partner's (K_fc's) unclaimed curve fees: base, and quote (wX).
    pub partner_base_fee: u64,
    pub partner_quote_fee: u64,
    pub sqrt_price: u128,
    pub activation_point: u64,
    pub pool_type: u8,
    pub is_migrated: bool,
    pub is_partner_withdraw_surplus: bool,
    pub migration_progress: u8,
    pub is_withdraw_leftover: bool,
    /// Bit 0 creator's, bit 1 partner's migration fee withdrawn.
    pub migration_fee_withdraw_status: u8,
    pub finish_curve_timestamp: u64,
}

impl DbcPool {
    pub fn decode(owner: &Pubkey, d: &[u8]) -> Result<DbcPool, String> {
        check(owner, &id(DBC_PROGRAM), d, DBC_POOL_LEN, Some(&DBC_POOL_DISC), "DBC VirtualPool")?;
        Ok(DbcPool {
            config: k(d, 72),
            creator: k(d, 104),
            base_mint: k(d, 136),
            base_vault: k(d, 168),
            quote_vault: k(d, 200),
            base_reserve: u64_at(d, 232),
            quote_reserve: u64_at(d, 240),
            protocol_base_fee: u64_at(d, 248),
            protocol_quote_fee: u64_at(d, 256),
            partner_base_fee: u64_at(d, 264),
            partner_quote_fee: u64_at(d, 272),
            sqrt_price: u128_at(d, 280),
            activation_point: u64_at(d, 296),
            pool_type: d[304],
            is_migrated: d[305] == 1,
            is_partner_withdraw_surplus: d[306] == 1,
            migration_progress: d[308],
            is_withdraw_leftover: d[309] == 1,
            migration_fee_withdraw_status: d[311],
            finish_curve_timestamp: u64_at(d, 344),
        })
    }
}

// ---- DAMM v2 ------------------------------------------------------------------------------------------------------------------------

/// DAMM v2 0.2.4 `Pool`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DammPool {
    /// The base fee's info bytes (for a fixed fee: the numerator in the first 8).
    pub base_fee_info: [u8; 32],
    pub protocol_fee_percent: u8,
    pub referral_fee_percent: u8,
    pub compounding_fee_bps: u16,
    pub dynamic_fee_initialized: u8,
    pub token_a_mint: Pubkey,
    pub token_b_mint: Pubkey,
    pub token_a_vault: Pubkey,
    pub token_b_vault: Pubkey,
    pub liquidity: u128,
    pub sqrt_min_price: u128,
    pub sqrt_max_price: u128,
    pub sqrt_price: u128,
    pub activation_point: u64,
    pub activation_type: u8,
    pub pool_status: u8,
    pub collect_fee_mode: u8,
    pub pool_type: u8,
    /// U256 little-endian, fee per unit of liquidity scaled by 2^128.
    pub fee_a_per_liquidity: [u8; 32],
    pub fee_b_per_liquidity: [u8; 32],
    pub permanent_lock_liquidity: u128,
    pub creator: Pubkey,
    pub token_a_amount: u64,
    pub token_b_amount: u64,
}

impl DammPool {
    pub fn decode(owner: &Pubkey, d: &[u8]) -> Result<DammPool, String> {
        check(owner, &id(DAMM_PROGRAM), d, DAMM_POOL_LEN, Some(&DAMM_POOL_DISC), "DAMM v2 Pool")?;
        Ok(DammPool {
            base_fee_info: d[8..40].try_into().expect("32"),
            protocol_fee_percent: d[48],
            referral_fee_percent: d[50],
            compounding_fee_bps: u16_at(d, 54),
            dynamic_fee_initialized: d[56],
            token_a_mint: k(d, 168),
            token_b_mint: k(d, 200),
            token_a_vault: k(d, 232),
            token_b_vault: k(d, 264),
            liquidity: u128_at(d, 360),
            sqrt_min_price: u128_at(d, 424),
            sqrt_max_price: u128_at(d, 440),
            sqrt_price: u128_at(d, 456),
            activation_point: u64_at(d, 472),
            activation_type: d[480],
            pool_status: d[481],
            collect_fee_mode: d[484],
            pool_type: d[485],
            fee_a_per_liquidity: d[488..520].try_into().expect("32"),
            fee_b_per_liquidity: d[520..552].try_into().expect("32"),
            permanent_lock_liquidity: u128_at(d, 552),
            creator: k(d, 648),
            token_a_amount: u64_at(d, 680),
            token_b_amount: u64_at(d, 688),
        })
    }
    /// The price of token a in token b's base units per a base unit: `(sqrt_price / 2^64)²`, as f64 (display and sizing only;
    /// never in a money rule).
    pub fn price_f64(&self) -> f64 {
        let r = self.sqrt_price as f64 / 18_446_744_073_709_551_616f64;
        r * r
    }
}

/// DAMM v2 0.2.4 `Position`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DammPosition {
    pub pool: Pubkey,
    pub nft_mint: Pubkey,
    pub fee_a_per_token_checkpoint: [u8; 32],
    pub fee_b_per_token_checkpoint: [u8; 32],
    pub fee_a_pending: u64,
    pub fee_b_pending: u64,
    pub unlocked_liquidity: u128,
    pub vested_liquidity: u128,
    pub permanent_locked_liquidity: u128,
    pub total_claimed_a_fee: u64,
    pub total_claimed_b_fee: u64,
}

impl DammPosition {
    pub fn decode(owner: &Pubkey, d: &[u8]) -> Result<DammPosition, String> {
        check(owner, &id(DAMM_PROGRAM), d, DAMM_POSITION_LEN, Some(&DAMM_POSITION_DISC), "DAMM v2 Position")?;
        Ok(DammPosition {
            pool: k(d, 8),
            nft_mint: k(d, 40),
            fee_a_per_token_checkpoint: d[72..104].try_into().expect("32"),
            fee_b_per_token_checkpoint: d[104..136].try_into().expect("32"),
            fee_a_pending: u64_at(d, 136),
            fee_b_pending: u64_at(d, 144),
            unlocked_liquidity: u128_at(d, 152),
            vested_liquidity: u128_at(d, 168),
            permanent_locked_liquidity: u128_at(d, 184),
            total_claimed_a_fee: u64_at(d, 200),
            total_claimed_b_fee: u64_at(d, 208),
        })
    }
    pub fn liquidity(&self) -> u128 {
        self.unlocked_liquidity.saturating_add(self.vested_liquidity).saturating_add(self.permanent_locked_liquidity)
    }
}
