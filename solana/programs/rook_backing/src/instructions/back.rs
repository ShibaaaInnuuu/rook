use anchor_lang::{
    prelude::*,
    solana_program::{instruction::Instruction, program::invoke_signed},
    system_program,
};
use anchor_spl::{
    token::{self, CloseAccount, Token, TokenAccount},
    token_interface::{Mint, TokenAccount as IfaceTokenAccount, TokenInterface},
};

use crate::{
    constants::*,
    dbc,
    dynamic_bonding_curve::{self, program::DynamicBondingCurve},
    error::BackingError,
    state::{Partner, Settings, Vault},
};

#[derive(Accounts)]
pub struct Back<'info> {
    #[account(mut)]
    pub cranker: Signer<'info>,
    /// The one `Settings` (owner and discriminator checked by `Account`; only `init_backing` creates one, at ["settings"], so
    /// no seeds re-derivation: saves a `create_program_address`).
    pub settings: Box<Account<'info, Settings>>,
    #[account(mut, seeds = [PARTNER_SEED, partner.listing.as_ref()], bump = partner.bump)]
    pub partner: Box<Account<'info, Partner>>,
    #[account(
        mut,
        seeds = [VAULT_SEED, vault.coin_mint.as_ref()],
        bump = vault.bump,
        has_one = partner,
        constraint = !vault.graduated @ BackingError::AlreadyBacked,
    )]
    pub vault: Box<Account<'info, Vault>>,

    // ---- Meteora DBC: withdraw_migration_fee(flag = 0, partner) signed by the claimer PDA
    /// CHECK: the coin's DBC pool (DBC re-checks it against config and vault)
    #[account(mut, address = vault.dbc_pool)]
    pub dbc_pool: UncheckedAccount<'info>,
    /// CHECK: the pool's DBC config
    #[account(address = vault.dbc_config)]
    pub dbc_config: UncheckedAccount<'info>,
    /// CHECK: the pool's quote (wSOL) vault; DBC checks pool.quote_vault == this
    #[account(mut)]
    pub dbc_quote_vault: UncheckedAccount<'info>,
    /// CHECK: DBC's pool authority
    #[account(address = DBC_POOL_AUTHORITY)]
    pub dbc_pool_authority: UncheckedAccount<'info>,
    #[account(address = NATIVE_MINT)]
    pub quote_mint: Box<InterfaceAccount<'info, Mint>>,
    /// The DBC fee_claimer: system-owned, no data; signs DBC with ["claimer", listing, bump] and pays rook_core `convert`.
    #[account(mut, seeds = [CLAIMER_SEED, partner.listing.as_ref()], bump = partner.claimer_bump)]
    pub claimer: SystemAccount<'info>,
    /// Transient wSOL account that receives the fee; closed into the claimer before the end of the instruction.
    #[account(
        init,
        payer = cranker,
        seeds = [CLAIMER_WSOL_SEED, partner.listing.as_ref()],
        bump,
        token::mint = quote_mint,
        token::authority = claimer,
        token::token_program = token_program,
    )]
    pub claimer_wsol: Box<Account<'info, TokenAccount>>,
    /// CHECK: DBC's event authority (DBC checks it)
    pub dbc_event_authority: UncheckedAccount<'info>,
    pub dbc_program: Program<'info, DynamicBondingCurve>,

    // ---- rook_core: convert(lamports, min_units), converter = the program-wide ["converter"] PDA, payer = the claimer PDA
    /// CHECK: data-less PDA ["converter"] (address pinned in `Settings` at init); rook_core checks it is an allowed converter.
    #[account(address = settings.converter)]
    pub converter: UncheckedAccount<'info>,
    /// CHECK: rook_core program
    #[account(address = ROOK_CORE_ID, executable)]
    pub rook_core: UncheckedAccount<'info>,
    /// CHECK: pinned in Partner; rook_core validates
    #[account(address = partner.registry)]
    pub registry: UncheckedAccount<'info>,
    /// CHECK: pinned in Partner; rook_core validates
    #[account(address = partner.listing)]
    pub listing: UncheckedAccount<'info>,
    /// CHECK: pinned in Partner; rook_core validates; `back` reads its accepted floor (owner and layout checked in
    /// `min_units_floor`).
    #[account(mut, address = partner.module)]
    pub module: UncheckedAccount<'info>,
    /// CHECK: pinned in Partner; rook_core validates
    #[account(mut, address = partner.module_vault)]
    pub module_vault: UncheckedAccount<'info>,
    /// CHECK: pinned in Partner; rook_core validates
    #[account(mut, address = partner.wrapper)]
    pub wrapper: UncheckedAccount<'info>,
    #[account(mut, address = partner.wx_mint)]
    pub wx_mint: Box<InterfaceAccount<'info, Mint>>,
    #[account(mut, address = vault.wx_account)]
    pub vault_wx: Box<InterfaceAccount<'info, IfaceTokenAccount>>,
    #[account(address = vault.coin_mint)]
    pub coin_mint: Box<InterfaceAccount<'info, Mint>>,

    /// SPL Token (wSOL)
    pub token_program: Program<'info, Token>,
    /// wX's token program
    pub wx_token_program: Interface<'info, TokenInterface>,
    pub system_program: Program<'info, System>,
}

/// The fewest wX units `fee` lamports may convert into (audit A3-06): `(1 − BACK_PRICE_TOLERANCE_BPS)` of `fee` at the
/// module's accepted floor plus its conversion allowance, read from rook_core's `Module` (owner rook_core, exact layout,
/// `module.listing == listing`).
pub fn min_units_floor(module: &AccountInfo, listing: &Pubkey, fee: u64) -> Result<u64> {
    require_keys_eq!(*module.owner, ROOK_CORE_ID, BackingError::WrongCoreAccount);
    let m = rook_layouts::module::parse(&module.try_borrow_data()?).ok_or(BackingError::WrongCoreAccount)?;
    require!(m.listing == listing.to_bytes(), BackingError::WrongCoreAccount);
    require!(m.accepted != 0, BackingError::NoFloor);
    Ok(units_floor(fee, m.accepted, m.conv_bps))
}

/// `⌊⌊fee · ONE / ⌈accepted · (1 + conv)⌉⌋ · (1 − TOL)⌋` (u128; the reference price rounds up, as rook_core's `grad_price`).
pub fn units_floor(fee: u64, accepted: u64, conv_bps: u16) -> u64 {
    let num = accepted as u128 * (BPS + conv_bps as u64) as u128;
    let price = num.div_ceil(BPS as u128);
    let units = fee as u128 * WX_ONE as u128 / price;
    (units * (BPS - BACK_PRICE_TOLERANCE_BPS) as u128 / BPS as u128) as u64
}

/// After graduation, anyone cranks `back`: the claimer PDA withdraws the partner migration fee from DBC, unwraps it, and
/// converts the lamports into wX through rook_core's `convert` (signed by the program-wide ["converter"] PDA, paid by the
/// claimer), minted into this coin's vault. Runs once per coin.
///
/// `min_units` is the cranker's slippage bound; the bound actually applied is the larger of it and `min_units_floor` (audit
/// A3-06), so a cranker cannot pick the moment the keeper's target runs ahead of the accepted floor.
///
/// It also records the DBC creator role holder NOW, at backing (audit A3-01): `Vault.creator` is whoever holds DBC's
/// transferable `pool.creator` when the coin is backed (its graduation for Rook), which is what the wake desk's `reclaim`
/// credits. DBC lets the creator hand the role to anyone until then; `open_vault`'s earlier copy is overwritten.
pub fn handle_back(ctx: Context<Back>, min_units: u64) -> Result<()> {
    let pool = dbc::read_pool(&ctx.accounts.dbc_pool.to_account_info())?;
    require!(pool.is_migrated == 1, BackingError::NotMigrated);

    let listing = ctx.accounts.partner.listing;
    let claimer_seeds: &[&[u8]] = &[CLAIMER_SEED, listing.as_ref(), &[ctx.accounts.partner.claimer_bump]];
    let signer = &[claimer_seeds];

    // 1. DBC withdraw_migration_fee (flag 0 = partner): wSOL -> claimer_wsol. DBC requires sender == config.fee_claimer
    //    and a Signer, so the PDA signs through invoke_signed.
    dynamic_bonding_curve::cpi::withdraw_migration_fee(
        CpiContext::new_with_signer(
            DBC_ID,
            dynamic_bonding_curve::cpi::accounts::WithdrawMigrationFee {
                pool_authority: ctx.accounts.dbc_pool_authority.to_account_info(),
                config: ctx.accounts.dbc_config.to_account_info(),
                virtual_pool: ctx.accounts.dbc_pool.to_account_info(),
                token_quote_account: ctx.accounts.claimer_wsol.to_account_info(),
                quote_vault: ctx.accounts.dbc_quote_vault.to_account_info(),
                quote_mint: ctx.accounts.quote_mint.to_account_info(),
                sender: ctx.accounts.claimer.to_account_info(),
                token_quote_program: ctx.accounts.token_program.to_account_info(),
                event_authority: ctx.accounts.dbc_event_authority.to_account_info(),
                program: ctx.accounts.dbc_program.to_account_info(),
            },
            signer,
        ),
        0,
    )?;
    ctx.accounts.claimer_wsol.reload()?;
    let fee = ctx.accounts.claimer_wsol.amount;

    // 2. Unwrap: close the wSOL account into the claimer (fee + rent), give the cranker its rent back.
    let rent_back = ctx.accounts.claimer_wsol.to_account_info().lamports() - fee;
    token::close_account(CpiContext::new_with_signer(
        token::ID,
        CloseAccount {
            account: ctx.accounts.claimer_wsol.to_account_info(),
            destination: ctx.accounts.claimer.to_account_info(),
            authority: ctx.accounts.claimer.to_account_info(),
        },
        signer,
    ))?;
    system_program::transfer(
        CpiContext::new_with_signer(
            system_program::ID,
            system_program::Transfer {
                from: ctx.accounts.claimer.to_account_info(),
                to: ctx.accounts.cranker.to_account_info(),
            },
            signer,
        ),
        rent_back,
    )?;

    // 3. rook_core convert(fee, min_units): the ["converter"] PDA signs (the one key in Registry.converters), the claimer
    //    pays (system transfer into the floor module), wX into the vault.
    let before = ctx.accounts.vault_wx.amount;
    let min_units = if fee > 0 {
        min_units.max(min_units_floor(&ctx.accounts.module.to_account_info(), &listing, fee)?)
    } else {
        min_units
    };
    if fee > 0 {
        let mut data = Vec::with_capacity(24);
        data.extend_from_slice(&CONVERT_DISCRIMINATOR);
        data.extend_from_slice(&fee.to_le_bytes());
        data.extend_from_slice(&min_units.to_le_bytes());
        let a = &ctx.accounts;
        let converter_bump = [a.settings.converter_bump];
        let converter_seeds: &[&[u8]] = &[CONVERTER_SEED, &converter_bump];
        let ix = Instruction {
            program_id: ROOK_CORE_ID,
            accounts: vec![
                AccountMeta::new_readonly(a.converter.key(), true), // converter
                AccountMeta::new(a.claimer.key(), true),          // payer
                AccountMeta::new_readonly(a.registry.key(), false),
                AccountMeta::new_readonly(a.listing.key(), false),
                AccountMeta::new(a.module.key(), false),
                AccountMeta::new(a.module_vault.key(), false),
                AccountMeta::new(a.wrapper.key(), false),
                AccountMeta::new(a.wx_mint.key(), false),
                AccountMeta::new(a.vault_wx.key(), false), // dest
                AccountMeta::new_readonly(a.wx_token_program.key(), false),
                AccountMeta::new_readonly(a.system_program.key(), false),
            ],
            data,
        };
        invoke_signed(
            &ix,
            &[
                a.converter.to_account_info(),
                a.claimer.to_account_info(),
                a.registry.to_account_info(),
                a.listing.to_account_info(),
                a.module.to_account_info(),
                a.module_vault.to_account_info(),
                a.wrapper.to_account_info(),
                a.wx_mint.to_account_info(),
                a.vault_wx.to_account_info(),
                a.wx_token_program.to_account_info(),
                a.system_program.to_account_info(),
                a.rook_core.to_account_info(),
            ],
            &[claimer_seeds, converter_seeds],
        )?;
        ctx.accounts.vault_wx.reload()?;
    }
    let units = ctx
        .accounts
        .vault_wx
        .amount
        .checked_sub(before)
        .ok_or(BackingError::MathOverflow)?;
    require!(units >= min_units, BackingError::BelowMinUnits);

    let now = Clock::get()?.unix_timestamp;
    let supply = ctx.accounts.coin_mint.supply;
    let v = &mut ctx.accounts.vault;
    v.graduated = true;
    v.creator = pool.creator;
    v.lamports_backed = fee;
    v.wx_units = units;
    v.coin_supply_at_backing = supply;
    v.backed_at = now;
    let p = &mut ctx.accounts.partner;
    p.lamports_backed = p.lamports_backed.saturating_add(fee);

    emit!(Backed {
        coin_mint: v.coin_mint,
        lamports: fee,
        wx_units: units,
        coin_supply: supply,
    });
    Ok(())
}

#[event]
pub struct Backed {
    pub coin_mint: Pubkey,
    pub lamports: u64,
    pub wx_units: u64,
    pub coin_supply: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_price_bound_is_the_accepted_floor_plus_allowance_less_the_tolerance() {
        // 42.5 SOL at a 200 SOL floor and 3 %: 0.20631 units at 206 SOL; 90 % of it
        let f = units_floor(42_500_000_000, 200_000_000_000, 300);
        assert_eq!(f, (42_500_000_000u128 * 1_000_000_000 / 206_000_000_000 * 9 / 10) as u64);
        // the reference rounds the price up (never more units than rook_core's own rounding)
        // (1 lamport × 1.03 → 2 lamports per unit: 3 lamports buy 1.5 units, 90 % = 1.35)
        assert_eq!(units_floor(3, 1, 300), 1_350_000_000);
        // a conversion at the top of a 10 % fill band (the widest validate_params allows) still clears the bound
        let accepted = 1_000_000_000u64;
        let at_band_top = 42_500_000_000u128 * 1_000_000_000 / (accepted as u128 * 11_000 / 10_000 * 10_300 / 10_000);
        assert!(at_band_top as u64 >= units_floor(42_500_000_000, accepted, 300));
        // a conversion at the day band's top (+20 %) does not
        let at_day_top = 42_500_000_000u128 * 1_000_000_000 / (accepted as u128 * 12_000 / 10_000 * 10_300 / 10_000);
        assert!((at_day_top as u64) < units_floor(42_500_000_000, accepted, 300));
        assert_eq!(units_floor(0, accepted, 300), 0);
    }
}
