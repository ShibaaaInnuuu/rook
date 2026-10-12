use anchor_lang::prelude::*;
use anchor_spl::token_interface::{Mint, TokenAccount, TokenInterface};

use crate::{
    constants::*,
    dbc,
    error::BackingError,
    state::{Partner, Vault},
};

#[derive(Accounts)]
pub struct OpenVault<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,
    #[account(mut, seeds = [PARTNER_SEED, partner.listing.as_ref()], bump = partner.bump)]
    pub partner: Box<Account<'info, Partner>>,
    /// CHECK: the coin's DBC VirtualPool; owner, discriminator, config and base mint checked in the handler.
    pub dbc_pool: UncheckedAccount<'info>,
    pub coin_mint: Box<InterfaceAccount<'info, Mint>>,
    #[account(address = partner.wx_mint)]
    pub wx_mint: Box<InterfaceAccount<'info, Mint>>,
    #[account(
        init,
        payer = payer,
        space = 8 + Vault::INIT_SPACE,
        seeds = [VAULT_SEED, coin_mint.key().as_ref()],
        bump
    )]
    pub vault: Box<Account<'info, Vault>>,
    #[account(
        init,
        payer = payer,
        seeds = [VAULT_WX_SEED, coin_mint.key().as_ref()],
        bump,
        token::mint = wx_mint,
        token::authority = vault,
        token::token_program = wx_token_program,
    )]
    pub vault_wx: Box<InterfaceAccount<'info, TokenAccount>>,
    pub wx_token_program: Interface<'info, TokenInterface>,
    pub system_program: Program<'info, System>,
}

/// Anyone opens the vault of a coin launched under one of Rook's configs. The pool account is DBC's own (owner checked),
/// so its `config` field is authoritative: a pool under any other partner's config is refused.
pub fn handle_open_vault(ctx: Context<OpenVault>) -> Result<()> {
    let pool = dbc::read_pool(&ctx.accounts.dbc_pool.to_account_info())?;
    let partner = &mut ctx.accounts.partner;
    require!(partner.configs.contains(&pool.config), BackingError::NotRookPool);
    require_keys_eq!(pool.base_mint, ctx.accounts.coin_mint.key(), BackingError::WrongCoinMint);
    // The pool's own creation time (registered configs all use timestamp activation), never this instruction's clock: a
    // vault opened late still records when the coin launched.
    let launched_at = i64::try_from(pool.activation_point).map_err(|_| BackingError::NoLaunchTime)?;
    require!(launched_at > 0, BackingError::NoLaunchTime);

    let v = &mut ctx.accounts.vault;
    v.partner = partner.key();
    v.coin_mint = pool.base_mint;
    v.dbc_pool = ctx.accounts.dbc_pool.key();
    v.dbc_config = pool.config;
    // the role holder now; `back` overwrites it with the holder at backing, which the wake desk credits (audit A3-01)
    v.creator = pool.creator;
    v.wx_account = ctx.accounts.vault_wx.key();
    v.graduated = false;
    v.launched_at = launched_at;
    v.bump = ctx.bumps.vault;
    v.wx_bump = ctx.bumps.vault_wx;
    partner.vaults = partner.vaults.saturating_add(1);
    Ok(())
}
