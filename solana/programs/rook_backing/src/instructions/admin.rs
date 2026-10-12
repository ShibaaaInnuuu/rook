use anchor_lang::prelude::*;

use crate::{
    constants::*,
    dbc::{
        self, ACTIVATION_TYPE_TIMESTAMP, COLLECT_FEE_MODE_QUOTE, DBC_CONFIG_VERSION, MIGRATED_COLLECT_FEE_MODE_QUOTE,
        MIGRATION_OPTION_DAMM_V2, TOKEN_TYPE_SPL,
    },
    error::BackingError,
    program::RookBacking,
    state::{Partner, Settings},
};

#[derive(Accounts)]
pub struct InitBacking<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,
    #[account(init, payer = authority, space = 8 + Settings::INIT_SPACE, seeds = [SETTINGS_SEED], bump)]
    pub settings: Account<'info, Settings>,
    #[account(constraint = program.programdata_address()? == Some(program_data.key()))]
    pub program: Program<'info, RookBacking>,
    #[account(constraint = program_data.upgrade_authority_address == Some(authority.key()) @ BackingError::Unauthorized)]
    pub program_data: Account<'info, ProgramData>,
    pub system_program: Program<'info, System>,
}

/// Once, by the program's upgrade authority: pins rook_core's registry (whose owner is the admin) and treasury.
pub fn handle_init_backing(ctx: Context<InitBacking>) -> Result<()> {
    let s = &mut ctx.accounts.settings;
    s.registry = Pubkey::find_program_address(&[rook_layouts::registry::SEED], &ROOK_CORE_ID).0;
    s.treasury = Pubkey::find_program_address(&[ROOK_CORE_TREASURY_SEED], &ROOK_CORE_ID).0;
    s.bump = ctx.bumps.settings;
    (s.converter, s.converter_bump) = Pubkey::find_program_address(&[CONVERTER_SEED], &crate::ID);
    Ok(())
}

/// The admin is rook_core's `Registry.owner` (the Squads vault in production): `registry` must be rook_core's registry
/// account (address pinned in `Settings`, owner program rook_core, Anchor discriminator) and `admin` its current owner.
pub fn require_registry_owner(registry: &AccountInfo, admin: &Pubkey) -> Result<()> {
    let data = registry.try_borrow_data()?;
    let owner = rook_layouts::registry::owner(&data).ok_or(BackingError::NotRegistry)?;
    require_keys_eq!(*admin, Pubkey::new_from_array(owner), BackingError::Unauthorized);
    Ok(())
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone)]
pub struct PartnerParams {
    pub listing: Pubkey,
    pub registry: Pubkey,
    pub module: Pubkey,
    pub module_vault: Pubkey,
    pub wrapper: Pubkey,
    pub wx_mint: Pubkey,
}

#[derive(Accounts)]
#[instruction(params: PartnerParams)]
pub struct RegisterPartner<'info> {
    #[account(mut)]
    pub admin: Signer<'info>,
    #[account(seeds = [SETTINGS_SEED], bump = settings.bump)]
    pub settings: Account<'info, Settings>,
    /// CHECK: rook_core's `Registry`: address (`settings.registry`) and owner program checked here, the discriminator and
    /// the `owner` field in `require_registry_owner`.
    #[account(address = settings.registry @ BackingError::NotRegistry, owner = ROOK_CORE_ID @ BackingError::NotRegistry)]
    pub registry: UncheckedAccount<'info>,
    #[account(
        init,
        payer = admin,
        space = 8 + Partner::INIT_SPACE,
        seeds = [PARTNER_SEED, params.listing.as_ref()],
        bump
    )]
    pub partner: Account<'info, Partner>,
    /// CHECK: rook_core's `Listing` (= `params.listing`): owner and discriminator checked in the handler.
    #[account(address = params.listing @ BackingError::WrongCoreAccount, owner = ROOK_CORE_ID @ BackingError::WrongCoreAccount)]
    pub listing: UncheckedAccount<'info>,
    /// CHECK: rook_core's floor `Module` of the listing (= `params.module`): owner, layout and links checked in the handler.
    #[account(address = params.module @ BackingError::WrongCoreAccount, owner = ROOK_CORE_ID @ BackingError::WrongCoreAccount)]
    pub module: UncheckedAccount<'info>,
    /// CHECK: rook_core's `ModuleVault` (= `params.module_vault`): owner, discriminator and `module.vault` checked.
    #[account(
        address = params.module_vault @ BackingError::WrongCoreAccount,
        owner = ROOK_CORE_ID @ BackingError::WrongCoreAccount
    )]
    pub module_vault: UncheckedAccount<'info>,
    /// CHECK: rook_core's `Wrapper` of the listing (= `params.wrapper`): owner, layout and links checked in the handler.
    #[account(address = params.wrapper @ BackingError::WrongCoreAccount, owner = ROOK_CORE_ID @ BackingError::WrongCoreAccount)]
    pub wrapper: UncheckedAccount<'info>,
    pub system_program: Program<'info, System>,
}

/// Checks that the accounts a `Partner` pins are this listing's rook_core accounts (audit A3-03): each is owned by rook_core
/// and carries its Anchor discriminator (so rook_core created it, at its PDA), and the links rook_core itself wrote hold:
/// `module.listing == listing`, `module.vault == module_vault`, `module.wrapper == wrapper`, `wrapper.listing == listing`,
/// `wrapper.wx_mint == wx_mint`. Returns the listing id (for the event the ceremony checks the listing order against).
fn check_core_accounts(a: &RegisterPartner, params: &PartnerParams) -> Result<u32> {
    let listing_id = rook_layouts::listing::id(&a.listing.try_borrow_data()?).ok_or(BackingError::WrongCoreAccount)?;
    let m = rook_layouts::module::parse(&a.module.try_borrow_data()?).ok_or(BackingError::WrongCoreAccount)?;
    require!(rook_layouts::module_vault::is(&a.module_vault.try_borrow_data()?), BackingError::WrongCoreAccount);
    let w = rook_layouts::wrapper::parse(&a.wrapper.try_borrow_data()?).ok_or(BackingError::WrongCoreAccount)?;
    let listing = params.listing.to_bytes();
    require!(
        m.listing == listing
            && m.vault == params.module_vault.to_bytes()
            && m.wrapper == params.wrapper.to_bytes()
            && w.listing == listing
            && w.wx_mint == params.wx_mint.to_bytes(),
        BackingError::WrongCoreAccount
    );
    Ok(listing_id)
}

pub fn handle_register_partner(ctx: Context<RegisterPartner>, params: PartnerParams) -> Result<()> {
    require_registry_owner(&ctx.accounts.registry, ctx.accounts.admin.key)?;
    require_keys_eq!(params.registry, ctx.accounts.settings.registry, BackingError::WrongRegistry);
    let listing_id = check_core_accounts(ctx.accounts, &params)?;
    let (claimer, claimer_bump) =
        Pubkey::find_program_address(&[CLAIMER_SEED, params.listing.as_ref()], &crate::ID);
    let p = &mut ctx.accounts.partner;
    p.listing = params.listing;
    p.registry = params.registry;
    p.module = params.module;
    p.module_vault = params.module_vault;
    p.wrapper = params.wrapper;
    p.wx_mint = params.wx_mint;
    p.claimer = claimer;
    p.claimer_bump = claimer_bump;
    p.bump = ctx.bumps.partner;
    p.configs = Vec::new();
    emit!(PartnerRegistered { listing: p.listing, listing_id, module: p.module, wrapper: p.wrapper, wx_mint: p.wx_mint, claimer });
    Ok(())
}

/// A collection registered: the listing id lets the ceremony check that each claimer (and so each day-1 config) belongs to
/// the collection it was made for (audit A3-03, A6-02).
#[event]
pub struct PartnerRegistered {
    pub listing: Pubkey,
    pub listing_id: u32,
    pub module: Pubkey,
    pub wrapper: Pubkey,
    pub wx_mint: Pubkey,
    pub claimer: Pubkey,
}

#[derive(Accounts)]
pub struct RegisterPartnerConfig<'info> {
    pub admin: Signer<'info>,
    #[account(seeds = [SETTINGS_SEED], bump = settings.bump)]
    pub settings: Account<'info, Settings>,
    /// CHECK: rook_core's `Registry`: address (`settings.registry`) and owner program checked here, the discriminator and
    /// the `owner` field in `require_registry_owner`.
    #[account(address = settings.registry @ BackingError::NotRegistry, owner = ROOK_CORE_ID @ BackingError::NotRegistry)]
    pub registry: UncheckedAccount<'info>,
    #[account(mut, seeds = [PARTNER_SEED, partner.listing.as_ref()], bump = partner.bump)]
    pub partner: Account<'info, Partner>,
    /// CHECK: a DBC PoolConfig; owner, length, discriminator and fields checked in the handler.
    pub dbc_config: UncheckedAccount<'info>,
}

/// Registers a DBC config Rook created (off-chain, with `create_config`) for this collection. The config is only accepted
/// if DBC will pay this collection's claimer PDA: fee_claimer = claimer, quote = wSOL, migration to DAMM v2, plain SPL
/// coins, fees collected in SOL, a non-zero partner migration fee with no creator share, and timestamp activation (so every
/// pool's `activation_point` is its launch time in unix seconds, which `open_vault` records for the wake desk).
///
/// And the design's invariants (audit A3-02/A3-05), each of which would otherwise strand value no instruction can move:
/// the DBC 0.2.1 config version (whose layout these offsets are); partner and creator LP permanently locked (no unlocked
/// share, no vesting: the claimer can only ever claim fees, and the creator cannot pull liquidity); the migrated DAMM v2
/// pool collecting its fees in SOL only, without compounding (`sweep_lp_fees` moves SOL only); no pool creation fee (no
/// instruction claims it); and leftover coins to the claimer (the launch's choice, `fee_claimer = leftover_receiver`).
pub fn handle_register_partner_config(ctx: Context<RegisterPartnerConfig>) -> Result<()> {
    require_registry_owner(&ctx.accounts.registry, ctx.accounts.admin.key)?;
    let p = &mut ctx.accounts.partner;
    let c = dbc::read_config(&ctx.accounts.dbc_config.to_account_info())?;
    require!(c.version == DBC_CONFIG_VERSION, BackingError::WrongConfigVersion);
    require_keys_eq!(c.fee_claimer, p.claimer, BackingError::WrongFeeClaimer);
    require_keys_eq!(c.leftover_receiver, p.claimer, BackingError::WrongLeftoverReceiver);
    require!(
        c.partner_liquidity_percentage == 0
            && c.creator_liquidity_percentage == 0
            && c.partner_vesting_initialized == 0
            && c.creator_vesting_initialized == 0,
        BackingError::UnlockedLiquidity
    );
    require!(
        c.migrated_collect_fee_mode == MIGRATED_COLLECT_FEE_MODE_QUOTE && c.migrated_compounding_fee_bps == 0,
        BackingError::WrongMigratedFeeMode
    );
    require!(c.pool_creation_fee == 0, BackingError::PoolCreationFee);
    require_keys_eq!(c.quote_mint, NATIVE_MINT, BackingError::WrongQuoteMint);
    require!(c.migration_option == MIGRATION_OPTION_DAMM_V2, BackingError::WrongMigrationOption);
    require!(c.token_type == TOKEN_TYPE_SPL, BackingError::WrongTokenType);
    require!(c.collect_fee_mode == COLLECT_FEE_MODE_QUOTE, BackingError::WrongCollectFeeMode);
    require!(c.activation_type == ACTIVATION_TYPE_TIMESTAMP, BackingError::WrongActivationType);
    require!(c.migration_fee_percentage > 0, BackingError::NoMigrationFee);
    require!(c.creator_migration_fee_percentage == 0, BackingError::CreatorMigrationFee);
    let key = ctx.accounts.dbc_config.key();
    require!(!p.configs.contains(&key), BackingError::ConfigAlreadyRegistered);
    require!(p.configs.len() < MAX_CONFIGS, BackingError::TooManyConfigs);
    p.configs.push(key);
    Ok(())
}
