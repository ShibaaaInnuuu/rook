use anchor_lang::{prelude::*, system_program};
use anchor_spl::{
    token::{self, CloseAccount, Token, TokenAccount},
    token_interface::{Mint, TokenAccount as IfaceTokenAccount, TokenInterface},
};

use crate::{
    constants::*,
    cp_amm::{self, program::CpAmm},
    dynamic_bonding_curve::{self, program::DynamicBondingCurve},
    error::BackingError,
    state::{Partner, Settings, Vault},
};

/// Unwraps the transient wSOL account into the claimer, pays its rent back to the cranker and the SOL to the treasury.
#[allow(clippy::too_many_arguments)]
fn unwrap_to_treasury<'info>(
    claimer_wsol: &mut Account<'info, TokenAccount>,
    claimer: &AccountInfo<'info>,
    cranker: &AccountInfo<'info>,
    treasury: &AccountInfo<'info>,
    signer: &[&[&[u8]]],
) -> Result<u64> {
    claimer_wsol.reload()?;
    let amount = claimer_wsol.amount;
    let rent_back = claimer_wsol.to_account_info().lamports() - amount;
    token::close_account(CpiContext::new_with_signer(
        token::ID,
        CloseAccount {
            account: claimer_wsol.to_account_info(),
            destination: claimer.clone(),
            authority: claimer.clone(),
        },
        signer,
    ))?;
    system_program::transfer(
        CpiContext::new_with_signer(
            system_program::ID,
            system_program::Transfer { from: claimer.clone(), to: cranker.clone() },
            signer,
        ),
        rent_back,
    )?;
    if amount > 0 {
        system_program::transfer(
            CpiContext::new_with_signer(
                system_program::ID,
                system_program::Transfer { from: claimer.clone(), to: treasury.clone() },
                signer,
            ),
            amount,
        )?;
    }
    Ok(amount)
}

#[derive(Accounts)]
pub struct SweepPartnerFees<'info> {
    #[account(mut)]
    pub cranker: Signer<'info>,
    #[account(seeds = [SETTINGS_SEED], bump = settings.bump, has_one = treasury @ BackingError::WrongTreasury)]
    pub settings: Box<Account<'info, Settings>>,
    #[account(mut, seeds = [PARTNER_SEED, partner.listing.as_ref()], bump = partner.bump)]
    pub partner: Box<Account<'info, Partner>>,
    #[account(seeds = [VAULT_SEED, vault.coin_mint.as_ref()], bump = vault.bump, has_one = partner)]
    pub vault: Box<Account<'info, Vault>>,
    /// CHECK: rook_core treasury PDA (== settings.treasury)
    #[account(mut)]
    pub treasury: UncheckedAccount<'info>,

    /// CHECK: DBC pool of the coin
    #[account(mut, address = vault.dbc_pool)]
    pub dbc_pool: UncheckedAccount<'info>,
    /// CHECK: DBC config of the pool
    #[account(address = vault.dbc_config)]
    pub dbc_config: UncheckedAccount<'info>,
    /// CHECK: DBC checks pool.base_vault == this
    #[account(mut)]
    pub dbc_base_vault: UncheckedAccount<'info>,
    /// CHECK: DBC checks pool.quote_vault == this
    #[account(mut)]
    pub dbc_quote_vault: UncheckedAccount<'info>,
    /// CHECK: DBC pool authority
    #[account(address = DBC_POOL_AUTHORITY)]
    pub dbc_pool_authority: UncheckedAccount<'info>,
    #[account(address = vault.coin_mint)]
    pub coin_mint: Box<InterfaceAccount<'info, Mint>>,
    #[account(address = NATIVE_MINT)]
    pub quote_mint: Box<InterfaceAccount<'info, Mint>>,
    #[account(mut, seeds = [CLAIMER_SEED, partner.listing.as_ref()], bump = partner.claimer_bump)]
    pub claimer: SystemAccount<'info>,
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
    /// The claimer's account of the coin. Rook configs collect fees in SOL only, so DBC moves nothing into it
    /// (max_amount_a = 0); it is required by DBC's account list.
    #[account(mut, token::mint = coin_mint, token::authority = claimer)]
    pub claimer_coin: Box<InterfaceAccount<'info, IfaceTokenAccount>>,
    /// CHECK: DBC event authority
    pub dbc_event_authority: UncheckedAccount<'info>,
    pub dbc_program: Program<'info, DynamicBondingCurve>,
    pub coin_token_program: Interface<'info, TokenInterface>,
    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

/// Anyone: claim the partner's share of the curve's trading fees (DBC `claim_trading_fee`, SOL) and send it to rook_core's
/// treasury.
pub fn handle_sweep_partner_fees(mut ctx: Context<SweepPartnerFees>) -> Result<()> {
    let listing = ctx.accounts.partner.listing;
    let claimer_seeds: &[&[u8]] = &[CLAIMER_SEED, listing.as_ref(), &[ctx.accounts.partner.claimer_bump]];
    let signer = &[claimer_seeds];
    let a = &ctx.accounts;
    dynamic_bonding_curve::cpi::claim_trading_fee(
        CpiContext::new_with_signer(
            DBC_ID,
            dynamic_bonding_curve::cpi::accounts::ClaimTradingFee {
                pool_authority: a.dbc_pool_authority.to_account_info(),
                config: a.dbc_config.to_account_info(),
                pool: a.dbc_pool.to_account_info(),
                token_a_account: a.claimer_coin.to_account_info(),
                token_b_account: a.claimer_wsol.to_account_info(),
                base_vault: a.dbc_base_vault.to_account_info(),
                quote_vault: a.dbc_quote_vault.to_account_info(),
                base_mint: a.coin_mint.to_account_info(),
                quote_mint: a.quote_mint.to_account_info(),
                fee_claimer: a.claimer.to_account_info(),
                token_base_program: a.coin_token_program.to_account_info(),
                token_quote_program: a.token_program.to_account_info(),
                event_authority: a.dbc_event_authority.to_account_info(),
                program: a.dbc_program.to_account_info(),
            },
            signer,
        ),
        0,
        u64::MAX,
    )?;
    let a = &mut ctx.accounts;
    let amount = unwrap_to_treasury(
        &mut a.claimer_wsol,
        &a.claimer.to_account_info(),
        &a.cranker.to_account_info(),
        &a.treasury.to_account_info(),
        signer,
    )?;
    a.partner.lamports_swept = a.partner.lamports_swept.saturating_add(amount);
    emit!(FeesSwept { coin_mint: a.vault.coin_mint, source: 0, lamports: amount });
    Ok(())
}

#[derive(Accounts)]
pub struct SweepLpFees<'info> {
    #[account(mut)]
    pub cranker: Signer<'info>,
    #[account(seeds = [SETTINGS_SEED], bump = settings.bump, has_one = treasury @ BackingError::WrongTreasury)]
    pub settings: Box<Account<'info, Settings>>,
    #[account(mut, seeds = [PARTNER_SEED, partner.listing.as_ref()], bump = partner.bump)]
    pub partner: Box<Account<'info, Partner>>,
    #[account(seeds = [VAULT_SEED, vault.coin_mint.as_ref()], bump = vault.bump, has_one = partner)]
    pub vault: Box<Account<'info, Vault>>,
    /// CHECK: rook_core treasury PDA (== settings.treasury)
    #[account(mut)]
    pub treasury: UncheckedAccount<'info>,

    /// CHECK: the graduated DAMM v2 pool (DAMM checks mints and vaults against it)
    pub damm_pool: UncheckedAccount<'info>,
    /// CHECK: the partner's DAMM v2 position (DAMM checks position.pool and the NFT)
    #[account(mut)]
    pub damm_position: UncheckedAccount<'info>,
    /// The position NFT, held by the claimer.
    #[account(constraint = position_nft_account.owner == claimer.key() @ BackingError::WrongPositionOwner)]
    pub position_nft_account: Box<InterfaceAccount<'info, IfaceTokenAccount>>,
    /// CHECK: DAMM checks
    #[account(mut)]
    pub damm_token_a_vault: UncheckedAccount<'info>,
    /// CHECK: DAMM checks
    #[account(mut)]
    pub damm_token_b_vault: UncheckedAccount<'info>,
    /// CHECK: DAMM v2 pool authority
    #[account(address = DAMM_V2_POOL_AUTHORITY)]
    pub damm_pool_authority: UncheckedAccount<'info>,
    #[account(address = vault.coin_mint)]
    pub coin_mint: Box<InterfaceAccount<'info, Mint>>,
    #[account(address = NATIVE_MINT)]
    pub quote_mint: Box<InterfaceAccount<'info, Mint>>,
    #[account(mut, seeds = [CLAIMER_SEED, partner.listing.as_ref()], bump = partner.claimer_bump)]
    pub claimer: SystemAccount<'info>,
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
    /// The claimer's account of the coin; the migrated pool collects fees in SOL only (OnlyB), so nothing lands here.
    #[account(mut, token::mint = coin_mint, token::authority = claimer)]
    pub claimer_coin: Box<InterfaceAccount<'info, IfaceTokenAccount>>,
    /// CHECK: DAMM v2 event authority
    pub damm_event_authority: UncheckedAccount<'info>,
    pub damm_program: Program<'info, CpAmm>,
    pub coin_token_program: Interface<'info, TokenInterface>,
    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

/// Anyone: claim the fees of the partner's (permanently locked) DAMM v2 position and send them to rook_core's treasury.
pub fn handle_sweep_lp_fees(mut ctx: Context<SweepLpFees>) -> Result<()> {
    let listing = ctx.accounts.partner.listing;
    let claimer_seeds: &[&[u8]] = &[CLAIMER_SEED, listing.as_ref(), &[ctx.accounts.partner.claimer_bump]];
    let signer = &[claimer_seeds];
    let a = &ctx.accounts;
    cp_amm::cpi::claim_position_fee(CpiContext::new_with_signer(
        DAMM_V2_ID,
        cp_amm::cpi::accounts::ClaimPositionFee {
            pool_authority: a.damm_pool_authority.to_account_info(),
            pool: a.damm_pool.to_account_info(),
            position: a.damm_position.to_account_info(),
            token_a_account: a.claimer_coin.to_account_info(),
            token_b_account: a.claimer_wsol.to_account_info(),
            token_a_vault: a.damm_token_a_vault.to_account_info(),
            token_b_vault: a.damm_token_b_vault.to_account_info(),
            token_a_mint: a.coin_mint.to_account_info(),
            token_b_mint: a.quote_mint.to_account_info(),
            position_nft_account: a.position_nft_account.to_account_info(),
            signer: a.claimer.to_account_info(),
            token_a_program: a.coin_token_program.to_account_info(),
            token_b_program: a.token_program.to_account_info(),
            event_authority: a.damm_event_authority.to_account_info(),
            program: a.damm_program.to_account_info(),
        },
        signer,
    ))?;
    let a = &mut ctx.accounts;
    let amount = unwrap_to_treasury(
        &mut a.claimer_wsol,
        &a.claimer.to_account_info(),
        &a.cranker.to_account_info(),
        &a.treasury.to_account_info(),
        signer,
    )?;
    a.partner.lamports_swept = a.partner.lamports_swept.saturating_add(amount);
    emit!(FeesSwept { coin_mint: a.vault.coin_mint, source: 1, lamports: amount });
    Ok(())
}

#[event]
pub struct FeesSwept {
    pub coin_mint: Pubkey,
    /// 0 = DBC curve trading fees, 1 = DAMM v2 position fees
    pub source: u8,
    pub lamports: u64,
}
