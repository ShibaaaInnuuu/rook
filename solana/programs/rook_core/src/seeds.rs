//! PDA seeds (every account of rook_core is a PDA of these) and the lamport/token helpers shared by the instructions.

use anchor_lang::prelude::*;
use anchor_spl::token::{self, Burn, MintTo};

use crate::error::RookError;

pub const REGISTRY: &[u8] = b"registry";
pub const LISTING: &[u8] = b"listing";
pub const MODULE: &[u8] = b"module";
pub const MODULE_VAULT: &[u8] = b"module_vault";
pub const MODULE_WX: &[u8] = b"module_wx";
pub const WRAPPER: &[u8] = b"wrapper";
pub const WX_MINT: &[u8] = b"wx_mint";
pub const ITEMS: &[u8] = b"items";
pub const ITEM: &[u8] = b"item";
pub const HELD: &[u8] = b"held";
pub const SOURCE: &[u8] = b"source";
pub const TICKET: &[u8] = b"ticket";
pub const DRAND: &[u8] = b"drand";
pub const TREASURY: &[u8] = b"treasury";
pub const DESK: &[u8] = b"desk";
pub const REQUEST: &[u8] = b"request";
pub const WAKE_SLOT: &[u8] = b"wake_slot";
pub const SPENT: &[u8] = b"spent";
pub const RAFFLE: &[u8] = b"raffle";
pub const RAFFLE_WX: &[u8] = b"raffle_wx";

/// Move lamports out of an account this program owns (a module vault, the treasury, a request) to any account.
/// The source never goes below its rent-exempt minimum.
pub fn pay_out<'info>(from: &AccountInfo<'info>, to: &AccountInfo<'info>, amount: u64) -> Result<()> {
    if amount == 0 {
        return Ok(());
    }
    let rent = Rent::get()?.minimum_balance(from.data_len());
    let have = from.lamports();
    require!(have >= rent && have - rent >= amount, RookError::MathOverflow);
    **from.try_borrow_mut_lamports()? -= amount;
    **to.try_borrow_mut_lamports()? = to.lamports().checked_add(amount).ok_or(RookError::MathOverflow)?;
    Ok(())
}

/// Lamports above the rent-exempt minimum.
pub fn spendable(acc: &AccountInfo) -> Result<u64> {
    let rent = Rent::get()?.minimum_balance(acc.data_len());
    Ok(acc.lamports().saturating_sub(rent))
}

/// A system transfer from a signer (`payer`) into any account.
pub fn pay_in<'info>(payer: &AccountInfo<'info>, to: &AccountInfo<'info>, amount: u64) -> Result<()> {
    anchor_lang::system_program::transfer(
        CpiContext::new(
            anchor_lang::system_program::ID,
            anchor_lang::system_program::Transfer { from: payer.clone(), to: to.clone() },
        ),
        amount,
    )
}

/// Mint `amount` wX to `to`, signed by the wrapper PDA (the mint authority).
pub fn wx_mint_to<'info>(
    mint: &AccountInfo<'info>,
    to: &AccountInfo<'info>,
    wrapper: &AccountInfo<'info>,
    listing: &Pubkey,
    wrapper_bump: u8,
    amount: u64,
) -> Result<()> {
    let seeds: &[&[u8]] = &[WRAPPER, listing.as_ref(), &[wrapper_bump]];
    token::mint_to(
        CpiContext::new_with_signer(
            token::ID,
            MintTo { mint: mint.clone(), to: to.clone(), authority: wrapper.clone() },
            &[seeds],
        ),
        amount,
    )
}

/// Burn `amount` wX from `from`, its owner (`authority`) having signed the transaction.
pub fn wx_burn<'info>(
    mint: &AccountInfo<'info>,
    from: &AccountInfo<'info>,
    authority: &AccountInfo<'info>,
    amount: u64,
) -> Result<()> {
    token::burn(CpiContext::new(token::ID, Burn { mint: mint.clone(), from: from.clone(), authority: authority.clone() }), amount)
}

/// Burn `amount` wX from the module's own token account, signed by the module PDA.
pub fn wx_burn_module<'info>(
    mint: &AccountInfo<'info>,
    from: &AccountInfo<'info>,
    module: &AccountInfo<'info>,
    listing: &Pubkey,
    module_bump: u8,
    amount: u64,
) -> Result<()> {
    let seeds: &[&[u8]] = &[MODULE, listing.as_ref(), &[module_bump]];
    token::burn(
        CpiContext::new_with_signer(
            token::ID,
            Burn { mint: mint.clone(), from: from.clone(), authority: module.clone() },
            &[seeds],
        ),
        amount,
    )
}
