use anchor_lang::prelude::*;
use anchor_spl::token_interface::{self, Burn, Mint, TokenAccount, TokenInterface, TransferChecked};

use crate::{constants::*, error::BackingError, state::Vault};

#[derive(Accounts)]
pub struct RedeemBacking<'info> {
    pub holder: Signer<'info>,
    #[account(
        mut,
        seeds = [VAULT_SEED, vault.coin_mint.as_ref()],
        bump = vault.bump,
        constraint = vault.graduated @ BackingError::NotBacked,
    )]
    pub vault: Box<Account<'info, Vault>>,
    #[account(mut, address = vault.coin_mint @ BackingError::WrongCoinMint)]
    pub coin_mint: Box<InterfaceAccount<'info, Mint>>,
    #[account(mut, token::mint = coin_mint, token::authority = holder, token::token_program = coin_token_program)]
    pub holder_coin: Box<InterfaceAccount<'info, TokenAccount>>,
    #[account(mut, address = vault.wx_account)]
    pub vault_wx: Box<InterfaceAccount<'info, TokenAccount>>,
    #[account(address = vault_wx.mint)]
    pub wx_mint: Box<InterfaceAccount<'info, Mint>>,
    /// Any wX token account the holder chooses.
    #[account(mut, token::mint = wx_mint, token::token_program = wx_token_program)]
    pub holder_wx: Box<InterfaceAccount<'info, TokenAccount>>,
    pub coin_token_program: Interface<'info, TokenInterface>,
    pub wx_token_program: Interface<'info, TokenInterface>,
}

/// payout = floor(vault_wx_balance × amount / live_coin_supply).
///
/// Rounding is always down, so the vault never pays more than the pro-rata share; the remainder stays in the vault and
/// accrues to the holders who remain (each redemption leaves vault/supply ≥ what it was). Burning the whole remaining
/// supply pays the whole balance (amount == supply ⇒ payout == balance), so the last redeemer takes the dust.
pub fn payout(vault_balance: u64, amount: u64, supply: u64) -> Result<u64> {
    require!(amount > 0, BackingError::ZeroAmount);
    require!(supply >= amount, BackingError::MathOverflow);
    let p = (vault_balance as u128)
        .checked_mul(amount as u128)
        .ok_or(BackingError::MathOverflow)?
        / supply as u128;
    Ok(p as u64)
}

pub fn handle_redeem_backing(ctx: Context<RedeemBacking>, amount: u64) -> Result<()> {
    require!(amount > 0, BackingError::ZeroAmount);
    let supply = ctx.accounts.coin_mint.supply;
    let wx = payout(ctx.accounts.vault_wx.amount, amount, supply)?;
    require!(wx > 0, BackingError::ZeroPayout);

    token_interface::burn(
        CpiContext::new(
            ctx.accounts.coin_token_program.key(),
            Burn {
                mint: ctx.accounts.coin_mint.to_account_info(),
                from: ctx.accounts.holder_coin.to_account_info(),
                authority: ctx.accounts.holder.to_account_info(),
            },
        ),
        amount,
    )?;

    let coin_mint = ctx.accounts.vault.coin_mint;
    let seeds: &[&[u8]] = &[VAULT_SEED, coin_mint.as_ref(), &[ctx.accounts.vault.bump]];
    token_interface::transfer_checked(
        CpiContext::new_with_signer(
            ctx.accounts.wx_token_program.key(),
            TransferChecked {
                from: ctx.accounts.vault_wx.to_account_info(),
                mint: ctx.accounts.wx_mint.to_account_info(),
                to: ctx.accounts.holder_wx.to_account_info(),
                authority: ctx.accounts.vault.to_account_info(),
            },
            &[seeds],
        ),
        wx,
        ctx.accounts.wx_mint.decimals,
    )?;

    let v = &mut ctx.accounts.vault;
    v.coins_burned = v.coins_burned.saturating_add(amount);
    v.wx_paid = v.wx_paid.saturating_add(wx);
    emit!(BackingRedeemed { coin_mint, holder: ctx.accounts.holder.key(), coins: amount, wx_units: wx });
    Ok(())
}

#[event]
pub struct BackingRedeemed {
    pub coin_mint: Pubkey,
    pub holder: Pubkey,
    pub coins: u64,
    pub wx_units: u64,
}
