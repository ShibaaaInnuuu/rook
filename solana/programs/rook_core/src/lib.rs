//! rook_core: the registry, floor modules, wrappers, treasury and wake desk of Rook on Solana (docs/v9/solana-migration.md
//! §3), one program with a module per role, so the tightly coupled parts need no CPI between them. The EVM's per-collection
//! contracts are PDAs of the collection's `Listing`. As built: docs/v9/rook-core-as-built.md.
#![allow(unexpected_cfgs)]
#![allow(clippy::result_large_err)]

pub mod clock;
pub mod error;
pub mod events;
pub mod hashing;
pub mod instructions;
pub mod math;
pub mod metadata;
pub mod nft;
pub mod params;
pub mod seeds;
pub mod state;
pub mod units;
/// Vectors of the pure rules the canister ports (`autobuyer/canister-rs/tests/fixtures/solana/rook-core-vectors.json`, phase B2).
#[cfg(test)]
mod vectors;

use anchor_lang::prelude::*;

pub use instructions::*;
pub use state::{BitcoinNetwork, DeskParams, Destination, ListingKind, NftStandard};
pub use metadata::WxMetadata;
pub use params::Params;

declare_id!("GepazuDZfS6zjwQ3U2zHCgk93xpRkp34AkmEFZKrAd7T");

#[program]
pub mod rook_core {
    use super::*;

    // ---- registry ----
    pub fn init_registry(
        ctx: Context<InitRegistry>,
        owner: Pubkey,
        bitcoin_network: BitcoinNetwork,
        initial_converters: Vec<Pubkey>,
    ) -> Result<()> {
        instructions::registry::handle_init_registry(ctx, owner, bitcoin_network, initial_converters)
    }
    pub fn propose_owner(ctx: Context<RegistryAdmin>, new_owner: Pubkey) -> Result<()> {
        instructions::registry::handle_propose_owner(ctx, new_owner)
    }
    pub fn accept_owner(ctx: Context<RegistryAdmin>) -> Result<()> {
        instructions::registry::handle_accept_owner(ctx)
    }
    pub fn set_guardian(ctx: Context<RegistryAdmin>, guardian: Pubkey) -> Result<()> {
        instructions::registry::handle_set_guardian(ctx, guardian)
    }
    pub fn propose_roles(ctx: Context<RegistryAdmin>, keeper: Pubkey) -> Result<()> {
        instructions::registry::handle_propose_roles(ctx, keeper)
    }
    pub fn accept_roles(ctx: Context<RegistryAdmin>) -> Result<()> {
        instructions::registry::handle_accept_roles(ctx)
    }
    pub fn pause_keeper(ctx: Context<RegistryAdmin>) -> Result<()> {
        instructions::registry::handle_pause_keeper(ctx)
    }
    pub fn unpause_keeper(ctx: Context<RegistryAdmin>) -> Result<()> {
        instructions::registry::handle_unpause_keeper(ctx)
    }
    pub fn propose_converter(ctx: Context<RegistryAdmin>, converter: Pubkey) -> Result<()> {
        instructions::registry::handle_propose_converter(ctx, converter)
    }
    pub fn accept_converter(ctx: Context<RegistryAdmin>) -> Result<()> {
        instructions::registry::handle_accept_converter(ctx)
    }
    pub fn remove_converter(ctx: Context<RegistryAdmin>, converter: Pubkey) -> Result<()> {
        instructions::registry::handle_remove_converter(ctx, converter)
    }
    pub fn list_collection(ctx: Context<ListCollection>, args: ListArgs) -> Result<()> {
        instructions::registry::handle_list_collection(ctx, args)
    }
    pub fn set_paused(ctx: Context<ListingAdmin>, paused: bool) -> Result<()> {
        instructions::registry::handle_set_paused(ctx, paused)
    }
    pub fn pause_poster(ctx: Context<ListingAdmin>) -> Result<()> {
        instructions::registry::handle_pause_poster(ctx)
    }
    pub fn unpause_poster(ctx: Context<ListingAdmin>) -> Result<()> {
        instructions::registry::handle_unpause_poster(ctx)
    }
    pub fn propose_root(ctx: Context<ListingAdmin>, root: [u8; 32]) -> Result<()> {
        instructions::registry::handle_propose_root(ctx, root)
    }
    pub fn accept_root(ctx: Context<ListingAdmin>) -> Result<()> {
        instructions::registry::handle_accept_root(ctx)
    }

    // ---- floor ----
    pub fn init_module(ctx: Context<InitModule>, params: Params) -> Result<()> {
        instructions::floor::handle_init_module(ctx, params)
    }
    pub fn post_floor(ctx: Context<PostFloor>, price: u64, confidence_bps: u16, depth_items: u32) -> Result<()> {
        instructions::floor::handle_post_floor(ctx, price, confidence_bps, depth_items)
    }
    pub fn mint(ctx: Context<MintUnits>, lamports: u64, min_units: u64) -> Result<()> {
        instructions::floor::handle_mint(ctx, lamports, min_units)
    }
    pub fn burn(ctx: Context<BurnUnits>, units: u64, min_lamports: u64) -> Result<()> {
        instructions::floor::handle_burn(ctx, units, min_lamports)
    }
    /// See `instructions::floor::convert` for the full interface (the accounts, in order, and the semantics).
    pub fn convert(ctx: Context<Convert>, lamports: u64, min_units: u64) -> Result<()> {
        instructions::floor::handle_convert(ctx, lamports, min_units)
    }
    pub fn advance(ctx: Context<Advance>, amount: u64) -> Result<()> {
        instructions::floor::handle_advance(ctx, amount)
    }
    pub fn settle_advance(ctx: Context<Settle>, cost: u64) -> Result<()> {
        instructions::floor::handle_settle_advance(ctx, cost)
    }
    pub fn item_arrived(ctx: Context<Settle>, cost: u64) -> Result<()> {
        instructions::floor::handle_item_arrived(ctx, cost)
    }
    pub fn return_advance(ctx: Context<ReturnAdvance>, lamports: u64) -> Result<()> {
        instructions::floor::handle_return_advance(ctx, lamports)
    }
    pub fn snapshot_cap(ctx: Context<SnapshotCap>) -> Result<()> {
        instructions::floor::handle_snapshot_cap(ctx)
    }
    pub fn fund_reserve(ctx: Context<FundReserve>, lamports: u64) -> Result<()> {
        instructions::floor::handle_fund_reserve(ctx, lamports)
    }
    pub fn skim(ctx: Context<Skim>) -> Result<()> {
        instructions::floor::handle_skim(ctx)
    }

    // ---- wrap ----
    /// The owner: the wrapper, its wX mint and the mint's Metaplex metadata (`meta`: name, symbol, uri).
    pub fn init_wrapper(ctx: Context<InitWrapper>, meta: WxMetadata) -> Result<()> {
        instructions::wrap::handle_init_wrapper(ctx, meta)
    }
    /// The owner: a wX's Metaplex name, symbol and uri (the wrapper PDA is the update authority).
    pub fn set_wx_metadata(ctx: Context<SetWxMetadata>, meta: WxMetadata) -> Result<()> {
        instructions::wrap::handle_set_wx_metadata(ctx, meta)
    }
    pub fn init_mint_limit(ctx: Context<WrapperAdmin>, limit: u32) -> Result<()> {
        instructions::wrap::handle_init_mint_limit(ctx, limit)
    }
    pub fn propose_mint_limit(ctx: Context<WrapperAdmin>, limit: u32) -> Result<()> {
        instructions::wrap::handle_propose_mint_limit(ctx, limit)
    }
    pub fn accept_mint_limit(ctx: Context<WrapperAdmin>) -> Result<()> {
        instructions::wrap::handle_accept_mint_limit(ctx)
    }
    pub fn mint_item<'info>(ctx: Context<'info, MintItem<'info>>, args: MintItemArgs) -> Result<()> {
        instructions::wrap::handle_mint_item(ctx, args)
    }
    pub fn redeem(ctx: Context<Redeem>, destination: Destination) -> Result<()> {
        instructions::wrap::handle_redeem(ctx, destination)
    }
    pub fn resolve(ctx: Context<Resolve>) -> Result<()> {
        instructions::wrap::handle_resolve(ctx)
    }
    pub fn note_sent(ctx: Context<CustodianTicket>, tx: [u8; 32]) -> Result<()> {
        instructions::wrap::handle_note_sent(ctx, tx)
    }
    pub fn mark_undeliverable(ctx: Context<CustodianTicket>) -> Result<()> {
        instructions::wrap::handle_mark_undeliverable(ctx)
    }
    pub fn record_delivery<'info>(ctx: Context<'info, RecordDelivery<'info>>, tx: [u8; 32]) -> Result<()> {
        instructions::wrap::handle_record_delivery(ctx, tx)
    }
    pub fn refund(ctx: Context<Refund>) -> Result<()> {
        instructions::wrap::handle_refund(ctx)
    }
    pub fn prove_round(ctx: Context<ProveRound>, round: u64, randomness: [u8; 32]) -> Result<()> {
        instructions::wrap::handle_prove_round(ctx, round, randomness)
    }

    // ---- treasury ----
    pub fn init_treasury(ctx: Context<InitTreasury>) -> Result<()> {
        instructions::treasury::handle_init_treasury(ctx)
    }
    pub fn set_shares(ctx: Context<TreasuryAdmin>, reserves_bps: u16, ops_bps: u16) -> Result<()> {
        instructions::treasury::handle_set_shares(ctx, reserves_bps, ops_bps)
    }
    pub fn set_ops(ctx: Context<TreasuryAdmin>, ops: Pubkey) -> Result<()> {
        instructions::treasury::handle_set_ops(ctx, ops)
    }
    pub fn apply_shares(ctx: Context<TreasuryApply>) -> Result<()> {
        instructions::treasury::handle_apply_shares(ctx)
    }
    pub fn apply_ops(ctx: Context<TreasuryApply>) -> Result<()> {
        instructions::treasury::handle_apply_ops(ctx)
    }
    pub fn set_reserve_cap(ctx: Context<TreasuryAdmin>, bps: u32) -> Result<()> {
        instructions::treasury::handle_set_reserve_cap(ctx, bps)
    }
    pub fn apply_reserve_cap(ctx: Context<TreasuryApply>) -> Result<()> {
        instructions::treasury::handle_apply_reserve_cap(ctx)
    }
    pub fn set_raffle_bps(ctx: Context<TreasuryAdmin>, bps: u16) -> Result<()> {
        instructions::treasury::handle_set_raffle_bps(ctx, bps)
    }
    pub fn apply_raffle_bps(ctx: Context<TreasuryApply>) -> Result<()> {
        instructions::treasury::handle_apply_raffle_bps(ctx)
    }
    pub fn add_collection(ctx: Context<AddCollection>) -> Result<()> {
        instructions::treasury::handle_add_collection(ctx)
    }
    pub fn receive(ctx: Context<Receive>, lamports: u64) -> Result<()> {
        instructions::treasury::handle_receive(ctx, lamports)
    }
    pub fn allocate(ctx: Context<Allocate>) -> Result<()> {
        instructions::treasury::handle_allocate(ctx)
    }
    pub fn begin_distribution<'info>(ctx: Context<'info, BeginDistribution<'info>>) -> Result<()> {
        instructions::treasury::handle_begin_distribution(ctx)
    }
    /// Anyone: `begin_distribution` in pages, for more modules than one transaction can lock (docs/v9/solana-nfts-design.md §6).
    pub fn weigh_distribution<'info>(ctx: Context<'info, BeginDistribution<'info>>, start: u8) -> Result<()> {
        instructions::treasury::handle_weigh_distribution(ctx, start)
    }
    pub fn distribute(ctx: Context<Distribute>, index: u8) -> Result<()> {
        instructions::treasury::handle_distribute(ctx, index)
    }

    // ---- raffle (docs/v9/raffle-design.md) ----
    pub fn init_raffle(ctx: Context<InitRaffle>) -> Result<()> {
        instructions::raffle::handle_init_raffle(ctx)
    }
    pub fn fund_prize(ctx: Context<FundPrize>) -> Result<()> {
        instructions::raffle::handle_fund_prize(ctx)
    }
    pub fn commit_raffle(
        ctx: Context<CommitRaffle>,
        root: [u8; 32],
        total_weight: u64,
        snapshot_slot: u64,
        entries: u32,
    ) -> Result<()> {
        instructions::raffle::handle_commit_raffle(ctx, root, total_weight, snapshot_slot, entries)
    }
    pub fn draw_raffle(ctx: Context<DrawRaffle>) -> Result<()> {
        instructions::raffle::handle_draw_raffle(ctx)
    }
    pub fn claim_raffle(ctx: Context<ClaimRaffle>, cum_start: u64, weight: u64, proof: Vec<[u8; 32]>) -> Result<()> {
        instructions::raffle::handle_claim_raffle(ctx, cum_start, weight, proof)
    }
    pub fn expire_raffle(ctx: Context<ExpireRaffle>) -> Result<()> {
        instructions::raffle::handle_expire_raffle(ctx)
    }

    // ---- wake ----
    pub fn init_desk(ctx: Context<InitDesk>, params: DeskParams) -> Result<()> {
        instructions::wake::handle_init_desk(ctx, params)
    }
    pub fn request(ctx: Context<RequestWake>, bond: u64) -> Result<()> {
        instructions::wake::handle_request(ctx, bond)
    }
    pub fn mark_served(ctx: Context<MarkServed>) -> Result<()> {
        instructions::wake::handle_mark_served(ctx)
    }
    pub fn reclaim(ctx: Context<Reclaim>) -> Result<()> {
        instructions::wake::handle_reclaim(ctx)
    }
    pub fn forfeit(ctx: Context<Forfeit>) -> Result<()> {
        instructions::wake::handle_forfeit(ctx)
    }
    pub fn withdraw(ctx: Context<Withdraw>) -> Result<()> {
        instructions::wake::handle_withdraw(ctx)
    }
}
