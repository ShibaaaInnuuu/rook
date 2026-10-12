//! The registry role (src/ListingRegistry.sol, src/floor/PosterSource.sol, src/curve/CollectionDirectory.sol).

use anchor_lang::prelude::*;
use anchor_lang::solana_program::bpf_loader_upgradeable;

use crate::clock::now;
use crate::error::RookError;
use crate::events::*;
use crate::hashing::{is_taproot_on, parse_evm_address};
use crate::seeds::*;
use crate::state::*;

#[derive(Accounts)]
pub struct InitRegistry<'info> {
    /// The program's upgrade authority (its ProgramData's `upgrade_authority_address`): nobody can front-run the deployer.
    pub authority: Signer<'info>,
    #[account(mut)]
    pub payer: Signer<'info>,
    #[account(init, payer = payer, space = 8 + Registry::INIT_SPACE, seeds = [REGISTRY], bump)]
    pub registry: Account<'info, Registry>,
    /// CHECK: this program's ProgramData (PDA `[program_id]` of the upgradeable loader, owned by it), read by hand in
    /// `upgrade_authority` (Anchor's `Account<ProgramData>` pulls in bincode: +53 KB of program).
    #[account(
        seeds = [crate::ID.as_ref()], bump, seeds::program = bpf_loader_upgradeable::ID,
        owner = bpf_loader_upgradeable::ID @ RookError::NotUpgradeAuthority
    )]
    pub program_data: UncheckedAccount<'info>,
    pub system_program: Program<'info, System>,
}

/// The upgrade authority recorded in an upgradeable loader ProgramData account (bincode
/// `UpgradeableLoaderState::ProgramData { slot: u64, upgrade_authority_address: Option<Pubkey> }`: tag u32 = 3 @0, slot @4,
/// option tag @12, key @13). `None` for an immutable program or any other account.
pub fn upgrade_authority(data: &[u8]) -> Option<Pubkey> {
    if data.len() < 45 || data[..4] != 3u32.to_le_bytes() || data[12] != 1 {
        return None;
    }
    Some(Pubkey::new_from_array(data[13..45].try_into().unwrap()))
}

/// Initialize the registry with its owner (the Squads vault in production) and the Bitcoin network its Inscriptions listings
/// live on (`Mainnet` in production; the launch file names it). Once, and only by the program's upgrade authority (an
/// immutable program, with no authority, can never be initialized: initialize before finalizing). Both are fixed for good.
/// `initial_converters` (at most `MAX_CONVERTERS`, distinct, none default; may be empty) join the allow-list at once, with no
/// `CHANGE_DELAY`: safe because only the upgrade authority can initialize, before anything is live (rook_backing's
/// program-wide `["converter"]` PDA goes in here). Every later addition waits `CHANGE_DELAY` (propose → accept); removal
/// stays instant.
pub fn handle_init_registry(
    ctx: Context<InitRegistry>,
    owner: Pubkey,
    bitcoin_network: BitcoinNetwork,
    initial_converters: Vec<Pubkey>,
) -> Result<()> {
    let authority = upgrade_authority(&ctx.accounts.program_data.try_borrow_data()?);
    require!(authority == Some(ctx.accounts.authority.key()), RookError::NotUpgradeAuthority);
    require!(owner != Pubkey::default(), RookError::BadInput);
    require!(initial_converters.len() <= MAX_CONVERTERS, RookError::Full);
    let r = &mut ctx.accounts.registry;
    for (i, c) in initial_converters.iter().enumerate() {
        require!(*c != Pubkey::default() && !initial_converters[..i].contains(c), RookError::BadInput);
        r.converters[i] = *c;
        emit!(ConverterSet { converter: *c, ready_at: 0, allowed: true });
    }
    r.converter_count = initial_converters.len() as u8;
    r.owner = owner;
    r.bitcoin_network = bitcoin_network;
    r.bump = ctx.bumps.registry;
    Ok(())
}

#[derive(Accounts)]
pub struct RegistryAdmin<'info> {
    pub signer: Signer<'info>,
    #[account(mut, seeds = [REGISTRY], bump = registry.bump)]
    pub registry: Account<'info, Registry>,
}

/// Ownable2Step's first step (owner). Refused onto the guardian (ListingRegistry.transferOwnership).
pub fn handle_propose_owner(ctx: Context<RegistryAdmin>, new_owner: Pubkey) -> Result<()> {
    let r = &mut ctx.accounts.registry;
    r.require_owner(ctx.accounts.signer.key)?;
    require!(new_owner == Pubkey::default() || new_owner != r.guardian, RookError::SameKey);
    r.pending_owner = new_owner;
    Ok(())
}

/// The proposed owner accepts.
pub fn handle_accept_owner(ctx: Context<RegistryAdmin>) -> Result<()> {
    let r = &mut ctx.accounts.registry;
    require!(r.pending_owner != Pubkey::default(), RookError::NothingPending);
    require_keys_eq!(*ctx.accounts.signer.key, r.pending_owner, RookError::NotOwner);
    r.owner = r.pending_owner;
    r.pending_owner = Pubkey::default();
    Ok(())
}

/// The owner, at once: the guardian may only stop things (default: none). Never the keeper, pending keeper or owner (N-8).
pub fn handle_set_guardian(ctx: Context<RegistryAdmin>, guardian: Pubkey) -> Result<()> {
    let r = &mut ctx.accounts.registry;
    r.require_owner(ctx.accounts.signer.key)?;
    if guardian != Pubkey::default() {
        require!(
            guardian != r.keeper && guardian != r.pending_keeper && guardian != r.owner && guardian != r.pending_owner,
            RookError::SameKey
        );
    }
    r.guardian = guardian;
    Ok(())
}

/// `proposeRoles`: the first keeper applies at once; every later change waits `CHANGE_DELAY` in public.
pub fn handle_propose_roles(ctx: Context<RegistryAdmin>, keeper: Pubkey) -> Result<()> {
    let r = &mut ctx.accounts.registry;
    r.require_owner(ctx.accounts.signer.key)?;
    require!(keeper != Pubkey::default(), RookError::BadInput);
    require!(keeper != r.guardian && keeper != r.owner, RookError::SameKey);
    if r.keeper == Pubkey::default() {
        r.keeper = keeper;
        emit!(RolesSet { keeper, ready_at: 0 });
        return Ok(());
    }
    let ready_at = now()? + CHANGE_DELAY;
    r.pending_keeper = keeper;
    r.pending_keeper_at = ready_at;
    emit!(RolesSet { keeper, ready_at });
    Ok(())
}

pub fn handle_accept_roles(ctx: Context<RegistryAdmin>) -> Result<()> {
    let r = &mut ctx.accounts.registry;
    r.require_owner(ctx.accounts.signer.key)?;
    require!(r.pending_keeper_at != 0, RookError::NothingPending);
    require!(now()? >= r.pending_keeper_at, RookError::TooEarly);
    r.keeper = r.pending_keeper;
    r.pending_keeper = Pubkey::default();
    r.pending_keeper_at = 0;
    emit!(RolesSet { keeper: r.keeper, ready_at: 0 });
    Ok(())
}

/// The owner or the guardian, at once: no keeper power anywhere until the owner resumes it.
pub fn handle_pause_keeper(ctx: Context<RegistryAdmin>) -> Result<()> {
    let r = &mut ctx.accounts.registry;
    r.require_owner_or_guardian(ctx.accounts.signer.key)?;
    r.keeper_paused = true;
    emit!(KeeperPausedSet { by: *ctx.accounts.signer.key, paused: true });
    Ok(())
}

/// The owner only; a real resume is stamped.
pub fn handle_unpause_keeper(ctx: Context<RegistryAdmin>) -> Result<()> {
    let r = &mut ctx.accounts.registry;
    r.require_owner(ctx.accounts.signer.key)?;
    if r.keeper_paused {
        r.keeper_resumed_at = now()?;
    }
    r.keeper_paused = false;
    emit!(KeeperPausedSet { by: *ctx.accounts.signer.key, paused: false });
    Ok(())
}

/// The owner proposes a program-derived signer (rook_backing's claimer PDA) as a `convert` caller. A converter mints wX at the
/// grad price without the public spread, hourly buckets or public cap, so like the keeper it waits `CHANGE_DELAY` in public
/// (one pending at a time; a new proposal replaces it). Until accepted it converts nothing.
pub fn handle_propose_converter(ctx: Context<RegistryAdmin>, converter: Pubkey) -> Result<()> {
    let r = &mut ctx.accounts.registry;
    r.require_owner(ctx.accounts.signer.key)?;
    require!(converter != Pubkey::default(), RookError::BadInput);
    require!(!r.is_converter(&converter), RookError::BadInput);
    require!((r.converter_count as usize) < MAX_CONVERTERS, RookError::Full);
    let ready_at = now()? + CHANGE_DELAY;
    r.pending_converter = converter;
    r.pending_converter_at = ready_at;
    emit!(ConverterSet { converter, ready_at, allowed: false });
    Ok(())
}

/// The owner, after `CHANGE_DELAY`: the pending converter joins the allow-list (≤ `MAX_CONVERTERS`).
pub fn handle_accept_converter(ctx: Context<RegistryAdmin>) -> Result<()> {
    let r = &mut ctx.accounts.registry;
    r.require_owner(ctx.accounts.signer.key)?;
    require!(r.pending_converter_at != 0, RookError::NothingPending);
    require!(now()? >= r.pending_converter_at, RookError::TooEarly);
    let n = r.converter_count as usize;
    require!(n < MAX_CONVERTERS, RookError::Full);
    let converter = r.pending_converter;
    r.converters[n] = converter;
    r.converter_count += 1;
    r.pending_converter = Pubkey::default();
    r.pending_converter_at = 0;
    emit!(ConverterSet { converter, ready_at: 0, allowed: true });
    Ok(())
}

/// The owner or the guardian, at once (stop-only): drops an allowed converter, or cancels the pending one.
pub fn handle_remove_converter(ctx: Context<RegistryAdmin>, converter: Pubkey) -> Result<()> {
    let r = &mut ctx.accounts.registry;
    r.require_owner_or_guardian(ctx.accounts.signer.key)?;
    require!(converter != Pubkey::default(), RookError::BadInput);
    if r.pending_converter_at != 0 && r.pending_converter == converter {
        r.pending_converter = Pubkey::default();
        r.pending_converter_at = 0;
    } else {
        let n = r.converter_count as usize;
        let i = r.converters[..n].iter().position(|c| *c == converter).ok_or(RookError::BadInput)?;
        r.converters[i] = r.converters[n - 1];
        r.converters[n - 1] = Pubkey::default();
        r.converter_count -= 1;
    }
    emit!(ConverterRemoved { converter, by: *ctx.accounts.signer.key });
    Ok(())
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct ListArgs {
    pub kind: ListingKind,
    pub name: String,
    pub custody: String,
    pub custodian: Pubkey,
    pub poster: Pubkey,
}

#[derive(Accounts)]
pub struct ListCollection<'info> {
    pub owner: Signer<'info>,
    #[account(mut)]
    pub payer: Signer<'info>,
    #[account(mut, seeds = [REGISTRY], bump = registry.bump)]
    pub registry: Account<'info, Registry>,
    #[account(
        init, payer = payer, space = 8 + Listing::INIT_SPACE,
        seeds = [LISTING, &registry.listing_count.to_le_bytes()], bump
    )]
    pub listing: Account<'info, Listing>,
    pub system_program: Program<'info, System>,
}

/// `listInscriptions` / the NFT listing (owner). The facts are checked as the constructors did: a root for inscriptions and a
/// taproot custody address on the registry's Bitcoin network (Wrapper's constructor; `bc1p…` on mainnet, `tb1p…` otherwise); a chain, a collection and an EVM custody address for an NFT
/// (EthNftWrapper's constructor); a collection, a poster and no custody address for a Solana NFT listing
/// (docs/v9/solana-nfts-design.md §3); a custodian always, a poster for an NFT listing.
pub fn handle_list_collection(ctx: Context<ListCollection>, args: ListArgs) -> Result<()> {
    let reg = &mut ctx.accounts.registry;
    reg.require_owner(ctx.accounts.owner.key)?;
    require!(!args.name.is_empty() && args.name.len() <= MAX_NAME, RookError::Empty);
    // a Solana NFT listing has no custody address off chain (its escrow is the PDA `["escrow", wrapper]`): empty
    let sol = matches!(args.kind, ListingKind::SolNft { .. });
    require!(sol == args.custody.is_empty() && args.custody.len() <= MAX_CUSTODY, RookError::Empty);
    require!(args.custodian != Pubkey::default(), RookError::BadInput);
    let mut custody_evm = [0u8; 20];
    match args.kind {
        ListingKind::EthNft { chain_id, collection } => {
            require!(chain_id != 0 && collection != [0u8; 20], RookError::BadInput);
            custody_evm = parse_evm_address(&args.custody).ok_or(RookError::BadInput)?;
            require!(custody_evm != [0u8; 20], RookError::BadInput);
            require!(args.poster != Pubkey::default(), RookError::BadInput);
        }
        ListingKind::Inscriptions { membership_root } => {
            require!(membership_root != [0u8; 32], RookError::Empty);
            require!(is_taproot_on(&args.custody, reg.bitcoin_network.hrp()), RookError::BadInput);
        }
        ListingKind::SolNft { collection, .. } => {
            require!(collection != Pubkey::default(), RookError::BadInput);
            require!(args.poster != Pubkey::default(), RookError::BadInput);
        }
    }
    let id = reg.listing_count;
    let l = &mut ctx.accounts.listing;
    l.id = id;
    l.kind = args.kind;
    l.custodian = args.custodian;
    l.poster = args.poster;
    l.custody = args.custody;
    l.custody_evm = custody_evm;
    l.name = args.name;
    l.bump = ctx.bumps.listing;
    reg.listing_count = id.checked_add(1).ok_or(RookError::MathOverflow)?;
    emit!(Listed { listing: l.key(), id });
    Ok(())
}

#[derive(Accounts)]
pub struct ListingAdmin<'info> {
    pub signer: Signer<'info>,
    #[account(seeds = [REGISTRY], bump = registry.bump)]
    pub registry: Account<'info, Registry>,
    #[account(mut, seeds = [LISTING, &listing.id.to_le_bytes()], bump = listing.bump)]
    pub listing: Account<'info, Listing>,
}

/// `setPaused` (owner, instant).
pub fn handle_set_paused(ctx: Context<ListingAdmin>, paused: bool) -> Result<()> {
    ctx.accounts.registry.require_owner(ctx.accounts.signer.key)?;
    ctx.accounts.listing.paused = paused;
    emit!(ListingPausedSet { listing: ctx.accounts.listing.key(), paused });
    Ok(())
}

/// `PosterSource.pause`: the owner or the guardian stops this collection's floor alone (D-M-1).
pub fn handle_pause_poster(ctx: Context<ListingAdmin>) -> Result<()> {
    ctx.accounts.registry.require_owner_or_guardian(ctx.accounts.signer.key)?;
    ctx.accounts.listing.poster_paused = true;
    emit!(PosterPausedSet { listing: ctx.accounts.listing.key(), by: *ctx.accounts.signer.key, paused: true });
    Ok(())
}

/// `PosterSource.unpause`: the owner only.
pub fn handle_unpause_poster(ctx: Context<ListingAdmin>) -> Result<()> {
    ctx.accounts.registry.require_owner(ctx.accounts.signer.key)?;
    let l = &mut ctx.accounts.listing;
    if l.poster_paused {
        l.poster_resumed_at = now()?;
    }
    l.poster_paused = false;
    emit!(PosterPausedSet { listing: l.key(), by: *ctx.accounts.signer.key, paused: false });
    Ok(())
}

/// `proposeRoot` (owner; inscriptions only; seven public days).
pub fn handle_propose_root(ctx: Context<ListingAdmin>, root: [u8; 32]) -> Result<()> {
    ctx.accounts.registry.require_owner(ctx.accounts.signer.key)?;
    let l = &mut ctx.accounts.listing;
    require!(matches!(l.kind, ListingKind::Inscriptions { .. }), RookError::WrongKind);
    require!(root != [0u8; 32], RookError::Empty);
    let ready_at = now()? + CHANGE_DELAY;
    l.pending_root = root;
    l.pending_root_at = ready_at;
    emit!(RootSet { listing: l.key(), root, ready_at });
    Ok(())
}

pub fn handle_accept_root(ctx: Context<ListingAdmin>) -> Result<()> {
    ctx.accounts.registry.require_owner(ctx.accounts.signer.key)?;
    let l = &mut ctx.accounts.listing;
    require!(l.pending_root_at != 0, RookError::NothingPending);
    require!(now()? >= l.pending_root_at, RookError::TooEarly);
    l.kind = ListingKind::Inscriptions { membership_root: l.pending_root };
    let root = l.pending_root;
    l.pending_root = [0u8; 32];
    l.pending_root_at = 0;
    emit!(RootSet { listing: l.key(), root, ready_at: 0 });
    Ok(())
}
