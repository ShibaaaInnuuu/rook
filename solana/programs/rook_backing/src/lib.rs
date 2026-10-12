//! rook_backing — Rook's Meteora DBC partner integration and the per-coin backing vaults (docs/v9/solana-migration.md §2.1,
//! docs/v9/rook-backing-as-built.md).
#![allow(unexpected_cfgs)]

pub mod constants;
pub mod dbc;
pub mod error;
pub mod instructions;
pub mod state;

use anchor_lang::prelude::*;

pub use constants::*;
pub use instructions::*;
pub use state::*;

declare_id!("8VqSJrKwQjFAXY5RXFxb69wYXmcCzTTuct4chu7FxbGf");

// Meteora's programs, from the IDLs of the deployed releases (DBC 0.2.1, DAMM v2 0.2.4) in `idls/`.
declare_program!(dynamic_bonding_curve);
declare_program!(cp_amm);

#[program]
pub mod rook_backing {
    use super::*;

    /// Once, by the program's upgrade authority: pins rook_core's registry and treasury. There is no stored admin: the admin
    /// instructions require rook_core's `Registry.owner` to sign (read from the registry account each time).
    pub fn init_backing(ctx: Context<InitBacking>) -> Result<()> {
        instructions::admin::handle_init_backing(ctx)
    }

    /// Admin (the registry owner): a collection (its rook_core listing and the accounts `convert` needs).
    pub fn register_partner(ctx: Context<RegisterPartner>, params: PartnerParams) -> Result<()> {
        instructions::admin::handle_register_partner(ctx, params)
    }

    /// Admin (the registry owner): a DBC config Rook created for the collection (fee_claimer must be the collection's claimer PDA).
    pub fn register_partner_config(ctx: Context<RegisterPartnerConfig>) -> Result<()> {
        instructions::admin::handle_register_partner_config(ctx)
    }

    /// Anyone: the backing vault of a coin launched under one of the collection's configs.
    pub fn open_vault(ctx: Context<OpenVault>) -> Result<()> {
        instructions::open_vault::handle_open_vault(ctx)
    }

    /// Anyone, after migration: DBC migration fee → SOL → rook_core `convert` → wX in the vault. Once per coin.
    pub fn back(ctx: Context<Back>, min_units: u64) -> Result<()> {
        instructions::back::handle_back(ctx, min_units)
    }

    /// Any holder: burn `amount` coins for floor(vault_wx × amount / supply) wX.
    pub fn redeem_backing(ctx: Context<RedeemBacking>, amount: u64) -> Result<()> {
        instructions::redeem::handle_redeem_backing(ctx, amount)
    }

    /// Anyone: DBC partner trading fees → rook_core treasury.
    pub fn sweep_partner_fees(ctx: Context<SweepPartnerFees>) -> Result<()> {
        instructions::sweep::handle_sweep_partner_fees(ctx)
    }

    /// Anyone: the partner DAMM v2 position's fees → rook_core treasury.
    pub fn sweep_lp_fees(ctx: Context<SweepLpFees>) -> Result<()> {
        instructions::sweep::handle_sweep_lp_fees(ctx)
    }
}
