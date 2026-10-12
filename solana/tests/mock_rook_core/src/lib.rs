//! Mock of rook_core's `convert(lamports, min_units)` — the interface rook_backing CPIs into, nothing more.
//! Converts at a fixed floor of 2 SOL per wX unit (1 unit = 10^9 base units, so base units = lamports / 2), takes the
//! lamports from `payer` into `module_vault`, and mints to `dest` with the wrapper PDA ["wrapper", listing] (rook_core's
//! seeds) as mint authority. The test harness writes rook_core-shaped `Listing`/`Module`/`ModuleVault`/`Wrapper` accounts at
//! those PDAs (rook_backing's `register_partner` and `back` read them; audit A3-03/A3-06).
//! Like the real one, it requires the converter to be a signer and registered: here, the registry account's data must
//! hold the converter's key (the test writes it).
use anchor_lang::{prelude::*, system_program};
use anchor_spl::token_interface::{self, Mint, MintTo, TokenAccount, TokenInterface};

declare_id!("GepazuDZfS6zjwQ3U2zHCgk93xpRkp34AkmEFZKrAd7T");

#[program]
pub mod mock_rook_core {
    use super::*;

    pub fn convert(ctx: Context<Convert>, lamports: u64, min_units: u64) -> Result<()> {
        let reg = ctx.accounts.registry.try_borrow_data()?;
        let conv = ctx.accounts.converter.key();
        require!(reg.chunks(32).any(|c| c == conv.as_ref()), MockError::NotConverter);
        drop(reg);
        let units = lamports / 2;
        require!(units >= min_units, MockError::Slippage);
        system_program::transfer(
            CpiContext::new(
                system_program::ID,
                system_program::Transfer {
                    from: ctx.accounts.payer.to_account_info(),
                    to: ctx.accounts.module_vault.to_account_info(),
                },
            ),
            lamports,
        )?;
        let listing = ctx.accounts.listing.key();
        let bump = ctx.bumps.wrapper;
        token_interface::mint_to(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.key(),
                MintTo {
                    mint: ctx.accounts.wx_mint.to_account_info(),
                    to: ctx.accounts.dest.to_account_info(),
                    authority: ctx.accounts.wrapper.to_account_info(),
                },
                &[&[b"wrapper", listing.as_ref(), &[bump]]],
            ),
            units,
        )?;
        msg!("mock convert: {} lamports -> {} units", lamports, units);
        Ok(())
    }
}

#[derive(Accounts)]
pub struct Convert<'info> {
    pub converter: Signer<'info>,
    #[account(mut)]
    pub payer: Signer<'info>,
    /// CHECK: mock registry (raw list of converter keys)
    pub registry: UncheckedAccount<'info>,
    /// CHECK: the wrapper's seed
    pub listing: UncheckedAccount<'info>,
    /// CHECK: unused by the mock
    #[account(mut)]
    pub module: UncheckedAccount<'info>,
    /// CHECK: receives the lamports
    #[account(mut)]
    pub module_vault: UncheckedAccount<'info>,
    /// CHECK: mint authority PDA ["wrapper", listing]
    #[account(mut, seeds = [b"wrapper", listing.key().as_ref()], bump)]
    pub wrapper: UncheckedAccount<'info>,
    #[account(mut)]
    pub wx_mint: InterfaceAccount<'info, Mint>,
    #[account(mut, token::mint = wx_mint)]
    pub dest: InterfaceAccount<'info, TokenAccount>,
    pub token_program: Interface<'info, TokenInterface>,
    pub system_program: Program<'info, System>,
}

#[error_code]
pub enum MockError {
    #[msg("converter not registered")]
    NotConverter,
    #[msg("below min units")]
    Slippage,
}
