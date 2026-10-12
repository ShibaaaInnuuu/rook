//! The wrap role (src/nft/EthNftWrapper.sol; src/Wrapper.sol's inscription keying and taproot destinations).

use anchor_lang::prelude::*;
use anchor_spl::token::{Mint, Token, TokenAccount};

use crate::clock::{now, round_at};
use crate::error::RookError;
use crate::events::*;
use crate::hashing::{is_taproot_on, keccak, leaf_of, taproot_hrp, verify_proof};
use crate::nft::{check_custody, check_delivered};
use crate::metadata::{create_wx_metadata, update_wx_metadata, WxMetadata, METADATA_SEED, MPL_TOKEN_METADATA_ID};
use crate::seeds::*;
use crate::state::*;
use crate::units::{ONE, WX_DECIMALS};

#[derive(Accounts)]
pub struct InitWrapper<'info> {
    pub owner: Signer<'info>,
    #[account(mut)]
    pub payer: Signer<'info>,
    #[account(seeds = [REGISTRY], bump = registry.bump)]
    pub registry: Account<'info, Registry>,
    pub listing: Account<'info, Listing>,
    #[account(init, payer = payer, space = 8 + Wrapper::INIT_SPACE, seeds = [WRAPPER, listing.key().as_ref()], bump)]
    pub wrapper: Box<Account<'info, Wrapper>>,
    #[account(
        init, payer = payer, seeds = [WX_MINT, wrapper.key().as_ref()], bump,
        mint::decimals = WX_DECIMALS, mint::authority = wrapper,
    )]
    pub wx_mint: Account<'info, Mint>,
    #[account(init, payer = payer, space = 8 + ItemSet::INIT_SPACE, seeds = [ITEMS, wrapper.key().as_ref()], bump)]
    pub item_set: Box<Account<'info, ItemSet>>,
    /// CHECK: the wX mint's Metaplex metadata PDA `["metadata", MPL, wx_mint]` (of Metaplex), created by the CPI.
    #[account(
        mut, seeds = [METADATA_SEED, MPL_TOKEN_METADATA_ID.as_ref(), wx_mint.key().as_ref()], bump,
        seeds::program = MPL_TOKEN_METADATA_ID
    )]
    pub metadata: UncheckedAccount<'info>,
    /// CHECK: Metaplex Token Metadata.
    #[account(address = MPL_TOKEN_METADATA_ID)]
    pub token_metadata_program: UncheckedAccount<'info>,
    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

/// The owner: a collection's wrapper and its `wX` mint (classic SPL Token, 9 decimals, mint authority = the wrapper PDA, no
/// freeze authority), with its Metaplex token metadata (name, symbol, uri; update authority = the wrapper PDA, mutable
/// through the owner-only `set_wx_metadata`; see `metadata.rs`). The EVM's `wireFloorModule` is implicit: the one module is
/// the listing's `["module", listing]` PDA.
pub fn handle_init_wrapper(ctx: Context<InitWrapper>, meta: WxMetadata) -> Result<()> {
    ctx.accounts.registry.require_owner(ctx.accounts.owner.key)?;
    let listing = ctx.accounts.listing.key();
    let bump = ctx.bumps.wrapper;
    let w = &mut ctx.accounts.wrapper;
    w.listing = listing;
    w.wx_mint = ctx.accounts.wx_mint.key();
    w.bump = bump;
    w.mint_bump = ctx.bumps.wx_mint;
    w.max_mints_per_day = DEFAULT_MINTS_PER_DAY;
    ctx.accounts.item_set.wrapper = w.key();
    let a = &ctx.accounts;
    create_wx_metadata(
        &a.token_metadata_program.to_account_info(),
        &a.metadata.to_account_info(),
        &a.wx_mint.to_account_info(),
        &a.wrapper.to_account_info(),
        &a.payer.to_account_info(),
        &a.system_program.to_account_info(),
        &[WRAPPER, listing.as_ref(), &[bump]],
        &meta,
    )
}

#[derive(Accounts)]
pub struct SetWxMetadata<'info> {
    pub owner: Signer<'info>,
    #[account(seeds = [REGISTRY], bump = registry.bump)]
    pub registry: Account<'info, Registry>,
    pub wrapper: Box<Account<'info, Wrapper>>,
    /// CHECK: the wX mint's Metaplex metadata PDA (Metaplex checks its update authority is the wrapper).
    #[account(
        mut, seeds = [METADATA_SEED, MPL_TOKEN_METADATA_ID.as_ref(), wrapper.wx_mint.as_ref()], bump,
        seeds::program = MPL_TOKEN_METADATA_ID
    )]
    pub metadata: UncheckedAccount<'info>,
    /// CHECK: Metaplex Token Metadata.
    #[account(address = MPL_TOKEN_METADATA_ID)]
    pub token_metadata_program: UncheckedAccount<'info>,
}

/// The owner: a wX's name, symbol and uri (the image), signed by the wrapper PDA as the metadata's update authority.
pub fn handle_set_wx_metadata(ctx: Context<SetWxMetadata>, meta: WxMetadata) -> Result<()> {
    ctx.accounts.registry.require_owner(ctx.accounts.owner.key)?;
    let w = &ctx.accounts.wrapper;
    update_wx_metadata(
        &ctx.accounts.token_metadata_program.to_account_info(),
        &ctx.accounts.metadata.to_account_info(),
        &w.to_account_info(),
        &[WRAPPER, w.listing.as_ref(), &[w.bump]],
        &meta,
    )?;
    emit!(WxMetadataSet { wrapper: w.key(), name: meta.name, symbol: meta.symbol, uri: meta.uri });
    Ok(())
}

#[derive(Accounts)]
pub struct WrapperAdmin<'info> {
    pub signer: Signer<'info>,
    #[account(seeds = [REGISTRY], bump = registry.bump)]
    pub registry: Account<'info, Registry>,
    #[account(mut)]
    pub wrapper: Box<Account<'info, Wrapper>>,
    #[account(address = wrapper.wx_mint)]
    pub wx_mint: Account<'info, Mint>,
}

/// `initMintLimit`: the owner, once, before anything was ever minted.
pub fn handle_init_mint_limit(ctx: Context<WrapperAdmin>, limit: u32) -> Result<()> {
    ctx.accounts.registry.require_owner(ctx.accounts.signer.key)?;
    require!(limit != 0 && (limit as usize) <= MAX_MINTS_PER_DAY, RookError::BadInput);
    let w = &mut ctx.accounts.wrapper;
    require!(w.item_mints == 0 && ctx.accounts.wx_mint.supply == 0 && w.held == 0, RookError::NotFresh);
    w.max_mints_per_day = limit;
    emit!(MintLimitSet { wrapper: w.key(), limit, ready_at: 0 });
    Ok(())
}

/// `proposeMintLimit`: the owner lowers at once, or proposes a raise anyone may accept after `LIMIT_DELAY`.
pub fn handle_propose_mint_limit(ctx: Context<WrapperAdmin>, limit: u32) -> Result<()> {
    ctx.accounts.registry.require_owner(ctx.accounts.signer.key)?;
    require!(limit != 0 && (limit as usize) <= MAX_MINTS_PER_DAY, RookError::BadInput);
    let w = &mut ctx.accounts.wrapper;
    if limit <= w.max_mints_per_day {
        w.max_mints_per_day = limit;
        w.pending_mint_limit = 0;
        w.pending_limit_at = 0;
        emit!(MintLimitSet { wrapper: w.key(), limit, ready_at: 0 });
        return Ok(());
    }
    let ready_at = now()? + LIMIT_DELAY;
    w.pending_mint_limit = limit;
    w.pending_limit_at = ready_at;
    emit!(MintLimitSet { wrapper: w.key(), limit, ready_at });
    Ok(())
}

/// `acceptMintLimit`: anyone, after the delay.
pub fn handle_accept_mint_limit(ctx: Context<WrapperAdmin>) -> Result<()> {
    let w = &mut ctx.accounts.wrapper;
    let limit = w.pending_mint_limit;
    require!(limit != 0, RookError::WrongStage);
    require!(now()? >= w.pending_limit_at, RookError::TooEarly);
    w.pending_mint_limit = 0;
    w.pending_limit_at = 0;
    w.max_mints_per_day = limit;
    emit!(MintLimitSet { wrapper: w.key(), limit, ready_at: 0 });
    Ok(())
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct MintItemArgs {
    /// EthNft: the token id (32 bytes big-endian). Inscriptions: `leaf_of(inscription_id)`.
    pub key: [u8; 32],
    /// EthNft: `keccak(key ‖ tx)`. Inscriptions: `keccak(tx ‖ vout be)` (the outpoint).
    pub source: [u8; 32],
    /// The transaction that brought the item into custody (purchase or deposit).
    pub tx: [u8; 32],
    pub vout: u32,
    pub deposit: bool,
    /// Inscriptions only.
    pub inscription_id: String,
    /// Inscriptions only: the membership proof.
    pub proof: Vec<[u8; 32]>,
}

#[derive(Accounts)]
#[instruction(args: MintItemArgs)]
pub struct MintItem<'info> {
    #[account(mut)]
    pub custodian: Signer<'info>,
    #[account(constraint = listing.custodian == custodian.key() @ RookError::NotCustodian)]
    pub listing: Box<Account<'info, Listing>>,
    #[account(mut, constraint = wrapper.listing == listing.key() @ RookError::WrongCollection)]
    pub wrapper: Box<Account<'info, Wrapper>>,
    #[account(mut, address = wrapper.wx_mint)]
    pub wx_mint: Account<'info, Mint>,
    #[account(mut, constraint = item_set.wrapper == wrapper.key() @ RookError::WrongCollection)]
    pub item_set: Box<Account<'info, ItemSet>>,
    #[account(
        init, payer = custodian, space = 8 + Item::INIT_SPACE,
        seeds = [ITEM, wrapper.key().as_ref(), &wrapper.item_count.to_le_bytes()], bump
    )]
    pub item: Box<Account<'info, Item>>,
    #[account(
        init_if_needed, payer = custodian, space = 8 + HeldMarker::INIT_SPACE,
        seeds = [HELD, wrapper.key().as_ref(), &args.key], bump
    )]
    pub held: Account<'info, HeldMarker>,
    #[account(
        init_if_needed, payer = custodian, space = 8 + SourceMarker::INIT_SPACE,
        seeds = [SOURCE, wrapper.key().as_ref(), &args.source], bump
    )]
    pub source: Account<'info, SourceMarker>,
    #[account(mut, token::mint = wx_mint)]
    pub dest: Account<'info, TokenAccount>,
    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

/// `mint` / `mintDeposit` (EthNft) and `wrapDeposit` (Inscriptions), the custodian only: one `ONE` to `dest` (the module's
/// token account for a purchase its advance paid). Refused while the listing is paused, once per item held, once per
/// (item, transaction) ever, and under H11's mints per day.
pub fn handle_mint_item<'info>(ctx: Context<'info, MintItem<'info>>, args: MintItemArgs) -> Result<()> {
    let listing = &ctx.accounts.listing;
    require!(!listing.paused, RookError::ListingPaused);
    require!(args.tx != [0u8; 32], RookError::BadInput);
    match listing.kind {
        ListingKind::EthNft { .. } => {
            require!(args.source == keccak(&[&args.key, &args.tx]), RookError::BadInput);
            require!(args.inscription_id.is_empty(), RookError::BadInput);
        }
        ListingKind::Inscriptions { membership_root } => {
            require!(
                !args.inscription_id.is_empty() && args.inscription_id.len() <= MAX_INSCRIPTION_ID,
                RookError::BadInput
            );
            require!(args.key == leaf_of(&args.inscription_id), RookError::BadInput);
            require!(verify_proof(&args.proof, &membership_root, args.key), RookError::NotInCollection);
            require!(args.source == keccak(&[&args.tx, &args.vout.to_be_bytes()]), RookError::BadInput);
        }
        // the NFT must be a member, in the custodian's wallet now (docs/v9/solana-nfts-design.md §4)
        ListingKind::SolNft { collection, standard } => {
            require!(args.source == keccak(&[&args.key, &args.tx]), RookError::BadInput);
            require!(args.inscription_id.is_empty(), RookError::BadInput);
            check_custody(ctx.remaining_accounts, standard, &collection, &args.key, &listing.custodian)?;
        }
    }
    require!(ctx.accounts.held.item_plus_one == 0, RookError::AlreadyHeld);
    require!(!ctx.accounts.source.used, RookError::AlreadyMinted);
    let t = now()?;
    let w = &mut ctx.accounts.wrapper;
    require!((w.held as usize) < MAX_HELD, RookError::CustodyFull);
    w.count_mint(t)?;
    ctx.accounts.source.used = true;
    let item_id = w.item_count;
    let item = &mut ctx.accounts.item;
    item.wrapper = w.key();
    item.item_id = item_id;
    item.key = args.key;
    item.source_tx = args.tx;
    item.vout = args.vout;
    item.deposit = args.deposit;
    item.inscription_id = args.inscription_id;
    ctx.accounts.held.item_plus_one = item_id + 1;
    w.item_count = item_id.checked_add(1).ok_or(RookError::MathOverflow)?;
    w.held += 1;
    ctx.accounts.item_set.pending.push(Pending { item_id, stamp: w.ticket_count });
    emit!(Wrapped {
        wrapper: w.key(),
        item_id,
        key: args.key,
        source_tx: args.tx,
        to: ctx.accounts.dest.key(),
        deposit: args.deposit
    });
    wx_mint_to(
        &ctx.accounts.wx_mint.to_account_info(),
        &ctx.accounts.dest.to_account_info(),
        &w.to_account_info(),
        &w.listing,
        w.bump,
        ONE,
    )
}

#[derive(Accounts)]
pub struct Redeem<'info> {
    #[account(mut)]
    pub owner: Signer<'info>,
    pub listing: Box<Account<'info, Listing>>,
    #[account(mut, constraint = wrapper.listing == listing.key() @ RookError::WrongCollection)]
    pub wrapper: Box<Account<'info, Wrapper>>,
    #[account(mut, address = wrapper.wx_mint)]
    pub wx_mint: Account<'info, Mint>,
    #[account(mut, token::mint = wx_mint, token::authority = owner)]
    pub owner_wx: Account<'info, TokenAccount>,
    #[account(
        init, payer = owner, space = 8 + Ticket::INIT_SPACE,
        seeds = [TICKET, wrapper.key().as_ref(), &wrapper.ticket_count.to_le_bytes()], bump
    )]
    pub ticket: Box<Account<'info, Ticket>>,
    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

/// `redeem` / `redeemToBurn` (anyone): burn one whole unit for one random item, delivered to `destination` (an address on
/// the NFT's chain, a taproot address on the listing's Bitcoin network for inscriptions, or the burn address). The round that decides is pinned now
/// (`round_at(now) + 20`) and does not exist yet; the destination is fixed. Pausing the listing stops no redemption.
pub fn handle_redeem(ctx: Context<Redeem>, destination: Destination) -> Result<()> {
    let listing = &ctx.accounts.listing;
    match (&listing.kind, &destination) {
        (ListingKind::EthNft { .. }, Destination::Eth(a)) => {
            require!(*a != [0u8; 20] && *a != listing.custody_evm, RookError::BadDestination);
        }
        (ListingKind::Inscriptions { .. }, Destination::Taproot(s)) => {
            // the listing's network: its custody address was checked against the registry's at `list_collection`, so its
            // prefix is the registry's (`bc` on mainnet, `tb` on signet/testnet4) and needs no registry account here
            let hrp = taproot_hrp(&listing.custody).ok_or(RookError::BadDestination)?;
            require!(is_taproot_on(s, hrp), RookError::BadDestination);
            // never the custody address itself (audit A2-03, as EthNft's A-M-2 fix): the canister would treat the ticket as
            // a relist and never deliver it, and the refund after the late window made it a free, repeatable lock on items
            require!(*s != listing.custody, RookError::CustodyDestination);
        }
        (ListingKind::SolNft { .. }, Destination::Sol(a)) => {
            require!(*a != Pubkey::default(), RookError::BadDestination);
            // never the custody wallet (as A-M-2 / A2-03)
            require!(*a != listing.custodian, RookError::CustodyDestination);
        }
        // no burn address on Solana: a burn would need the custodian to burn through Token Metadata / Core
        (ListingKind::SolNft { .. }, _) => return err!(RookError::BadDestination),
        (_, Destination::Burn) => {}
        _ => return err!(RookError::BadDestination),
    }
    let w = &mut ctx.accounts.wrapper;
    require!(w.held >= w.open_tickets + 1, RookError::NoItem);
    wx_burn(
        &ctx.accounts.wx_mint.to_account_info(),
        &ctx.accounts.owner_wx.to_account_info(),
        &ctx.accounts.owner.to_account_info(),
        ONE,
    )?;
    let t = now()?;
    let id = w.ticket_count;
    let tk = &mut ctx.accounts.ticket;
    tk.wrapper = w.key();
    tk.id = id;
    tk.stage = Stage::Pinned;
    tk.owner = ctx.accounts.owner.key();
    tk.round = round_at(t) + PIN_AHEAD;
    tk.pinned_at = t;
    tk.destination = destination;
    w.ticket_count = id + 1;
    w.open_tickets += 1;
    emit!(Redeemed { wrapper: w.key(), ticket: id, owner: tk.owner, round: tk.round, destination: tk.destination.clone() });
    Ok(())
}

#[derive(Accounts)]
pub struct Resolve<'info> {
    #[account(mut)]
    pub wrapper: Box<Account<'info, Wrapper>>,
    #[account(mut, constraint = item_set.wrapper == wrapper.key() @ RookError::WrongCollection)]
    pub item_set: Box<Account<'info, ItemSet>>,
    #[account(mut, constraint = ticket.wrapper == wrapper.key() @ RookError::WrongCollection)]
    pub ticket: Box<Account<'info, Ticket>>,
    /// The pinned round's randomness; omitted only to step past a refunded ticket.
    pub drand_round: Option<Account<'info, DrandRound>>,
}

/// `resolve` (anyone, strictly in ticket order): draws the item among those already free when this ticket was burned, with
/// the pinned round's randomness. A ticket refunded while pinned is stepped over here (the EVM's `_skipRefunded`).
pub fn handle_resolve(ctx: Context<Resolve>) -> Result<()> {
    let w = &mut ctx.accounts.wrapper;
    let tk = &mut ctx.accounts.ticket;
    let id = tk.id;
    require!(id == w.next_to_resolve, RookError::OutOfOrder);
    if tk.stage == Stage::Refunded {
        w.next_to_resolve = id + 1;
        return Ok(());
    }
    require!(tk.stage == Stage::Pinned, RookError::WrongStage);
    let round = ctx.accounts.drand_round.as_ref().ok_or(RookError::RoundNotProven)?;
    require!(round.round == tk.round, RookError::RoundNotProven);
    let set = &mut ctx.accounts.item_set;
    let k = set.pending.iter().take_while(|p| p.stamp <= id).count();
    let promoted: Vec<u32> = set.pending.drain(..k).map(|p| p.item_id).collect();
    set.free.extend(promoted);
    let size = set.free.len() as u64;
    require!(size != 0, RookError::NothingToDraw);
    let h = keccak(&[&round.randomness, w.key().as_ref(), &id.to_le_bytes()]);
    let mut i: u64 = 0;
    for b in h {
        i = ((i << 8) | b as u64) % size;
    }
    let item_id = set.free.swap_remove(i as usize);
    let t = now()?;
    tk.item_id = item_id;
    tk.stage = Stage::Resolved;
    tk.deliver_by = t + DELIVERY_DEADLINE;
    w.next_to_resolve = id + 1;
    emit!(Resolved { wrapper: w.key(), ticket: id, item_id, deliver_by: tk.deliver_by });
    Ok(())
}

#[derive(Accounts)]
pub struct CustodianTicket<'info> {
    pub custodian: Signer<'info>,
    #[account(constraint = listing.custodian == custodian.key() @ RookError::NotCustodian)]
    pub listing: Box<Account<'info, Listing>>,
    #[account(constraint = wrapper.listing == listing.key() @ RookError::WrongCollection)]
    pub wrapper: Box<Account<'info, Wrapper>>,
    #[account(mut, constraint = ticket.wrapper == wrapper.key() @ RookError::WrongCollection)]
    pub ticket: Box<Account<'info, Ticket>>,
}

/// `noteSent` (custodian): a transaction delivering this resolved ticket mined; from now on the late window never refunds it.
pub fn handle_note_sent(ctx: Context<CustodianTicket>, tx: [u8; 32]) -> Result<()> {
    let tk = &mut ctx.accounts.ticket;
    require!(tk.stage == Stage::Resolved, RookError::WrongStage);
    require!(tx != [0u8; 32], RookError::BadInput);
    tk.sent_tx = tx;
    emit!(DeliverySent { wrapper: tk.wrapper, ticket: tk.id, tx });
    Ok(())
}

/// `markUndeliverable` (custodian): no version of this delivery can confirm any more.
pub fn handle_mark_undeliverable(ctx: Context<CustodianTicket>) -> Result<()> {
    let tk = &mut ctx.accounts.ticket;
    require!(tk.stage == Stage::Resolved, RookError::WrongStage);
    tk.undeliverable = true;
    emit!(Undeliverable { wrapper: tk.wrapper, ticket: tk.id });
    Ok(())
}

#[derive(Accounts)]
pub struct RecordDelivery<'info> {
    pub custodian: Signer<'info>,
    #[account(constraint = listing.custodian == custodian.key() @ RookError::NotCustodian)]
    pub listing: Box<Account<'info, Listing>>,
    #[account(mut, constraint = wrapper.listing == listing.key() @ RookError::WrongCollection)]
    pub wrapper: Box<Account<'info, Wrapper>>,
    #[account(mut, constraint = ticket.wrapper == wrapper.key() @ RookError::WrongCollection)]
    pub ticket: Box<Account<'info, Ticket>>,
    #[account(
        constraint = item.wrapper == wrapper.key() && item.item_id == ticket.item_id @ RookError::WrongCollection
    )]
    pub item: Box<Account<'info, Item>>,
    #[account(
        mut, seeds = [HELD, wrapper.key().as_ref(), &item.key], bump,
        constraint = held.item_plus_one == item.item_id + 1 @ RookError::WrongHeldMarker
    )]
    pub held: Account<'info, HeldMarker>,
}

/// `recordDelivery` (custodian): the item left custody to the ticket's destination; allowed past the deadline while nobody
/// has refunded.
pub fn handle_record_delivery<'info>(ctx: Context<'info, RecordDelivery<'info>>, tx: [u8; 32]) -> Result<()> {
    let tk = &mut ctx.accounts.ticket;
    require!(tk.stage == Stage::Resolved, RookError::WrongStage);
    require!(tx != [0u8; 32], RookError::BadInput);
    // a Solana NFT: the destination holds it now (the canister sends the transfer and this in one transaction)
    if let (Some((_, standard)), Destination::Sol(to)) = (ctx.accounts.listing.sol_nft(), &tk.destination) {
        check_delivered(ctx.remaining_accounts, standard, &ctx.accounts.item.key, to)?;
    }
    tk.stage = Stage::Delivered;
    tk.delivery_tx = tx;
    ctx.accounts.held.item_plus_one = 0;
    let w = &mut ctx.accounts.wrapper;
    w.held -= 1;
    w.open_tickets -= 1;
    if tk.destination == Destination::Burn {
        w.burned += 1;
    }
    emit!(Delivered { wrapper: w.key(), ticket: tk.id, tx });
    Ok(())
}

#[derive(Accounts)]
pub struct Refund<'info> {
    #[account(constraint = wrapper.listing == listing.key() @ RookError::WrongCollection)]
    pub listing: Box<Account<'info, Listing>>,
    #[account(mut)]
    pub wrapper: Box<Account<'info, Wrapper>>,
    #[account(mut, constraint = item_set.wrapper == wrapper.key() @ RookError::WrongCollection)]
    pub item_set: Box<Account<'info, ItemSet>>,
    #[account(mut, constraint = ticket.wrapper == wrapper.key() @ RookError::WrongCollection)]
    pub ticket: Box<Account<'info, Ticket>>,
    #[account(mut, address = wrapper.wx_mint)]
    pub wx_mint: Account<'info, Mint>,
    #[account(mut, token::mint = wx_mint, token::authority = ticket.owner)]
    pub owner_wx: Account<'info, TokenAccount>,
    /// CHECK: the PDA `["drand", ticket.round]` of the ticket's pinned round, whether or not it exists (audit A2-04): a
    /// pinned ticket whose round is proven is not refunded.
    #[account(seeds = [DRAND, &ticket.round.to_le_bytes()], bump)]
    pub drand_round: UncheckedAccount<'info>,
    pub token_program: Program<'info, Token>,
}

/// `refund` (anyone; the unit goes back to the ticket's owner). A pinned ticket after `PIN_TIMEOUT` whose round was never
/// proven; a resolved one after `deliver_by + REFUND_GRACE` with `mark_undeliverable`, or after `deliver_by +
/// LATE_DELIVERY_WINDOW` without it and without `note_sent` (D3-H-1). The item goes back to the set.
///
/// A pinned ticket whose round IS proven is refused (audit A2-04): its outcome is public, so a refund would be a free
/// re-roll. This traps nobody: anyone may `resolve` it once every earlier ticket is resolved or refunded, and every earlier
/// ticket is either resolvable (its round proven) or refundable no later than this one (pinned earlier, round unproven;
/// anyone may call `refund`, the unit goes to that ticket's owner).
pub fn handle_refund(ctx: Context<Refund>) -> Result<()> {
    let t = now()?;
    let w = &mut ctx.accounts.wrapper;
    let tk = &mut ctx.accounts.ticket;
    let item = match tk.stage {
        Stage::Resolved => Some(tk.item_id),
        _ => None,
    };
    match tk.stage {
        Stage::Pinned => {
            require!(t >= tk.pinned_at + PIN_TIMEOUT, RookError::TooEarly);
            let r = &ctx.accounts.drand_round;
            require!(!(r.owner == &crate::ID && r.data_len() > 0), RookError::RoundProven);
            tk.stage = Stage::Refunded;
            w.open_tickets -= 1;
            if tk.id == w.next_to_resolve {
                w.next_to_resolve += 1;
            }
        }
        Stage::Resolved => {
            require!(t >= tk.deliver_by + REFUND_GRACE, RookError::TooEarly);
            if !tk.undeliverable {
                require!(tk.sent_tx == [0u8; 32], RookError::DeliverySentNotRecorded);
                require!(t >= tk.deliver_by + LATE_DELIVERY_WINDOW, RookError::NotUndeliverable);
            }
            tk.stage = Stage::Refunded;
            w.open_tickets -= 1;
            ctx.accounts.item_set.pending.push(Pending { item_id: tk.item_id, stamp: w.ticket_count });
        }
        _ => return err!(RookError::WrongStage),
    }
    emit!(Refunded { wrapper: w.key(), ticket: tk.id, owner: tk.owner, item, destination: tk.destination.clone() });
    wx_mint_to(
        &ctx.accounts.wx_mint.to_account_info(),
        &ctx.accounts.owner_wx.to_account_info(),
        &w.to_account_info(),
        &w.listing,
        w.bump,
        ONE,
    )
}

#[derive(Accounts)]
#[instruction(round: u64)]
pub struct ProveRound<'info> {
    pub keeper: Signer<'info>,
    #[account(mut)]
    pub payer: Signer<'info>,
    #[account(seeds = [REGISTRY], bump = registry.bump)]
    pub registry: Account<'info, Registry>,
    #[account(init, payer = payer, space = 8 + DrandRound::INIT_SPACE, seeds = [DRAND, &round.to_le_bytes()], bump)]
    pub drand_round: Account<'info, DrandRound>,
    pub system_program: Program<'info, System>,
}

/// TODO(drand on alt_bn128, Q4): until DrandEvmnet's BLS verification is ported to the alt_bn128 syscalls, a round's
/// randomness is written by the registry keeper alone (trusted). Then this takes `(round, signature)` from anyone, verifies
/// the pairing, and stores `keccak(signature)` as DrandEvmnet did.
pub fn handle_prove_round(ctx: Context<ProveRound>, round: u64, randomness: [u8; 32]) -> Result<()> {
    let k = ctx.accounts.registry.live_keeper().ok_or(RookError::NotKeeper)?;
    require_keys_eq!(k, ctx.accounts.keeper.key(), RookError::NotKeeper);
    require!(round != 0 && randomness != [0u8; 32], RookError::BadInput);
    // never a round that does not exist yet (audit A2-02): pre-published randomness would let any redeemer time a redemption
    // to choose its item
    require!(round <= round_at(now()?), RookError::RoundInFuture);
    ctx.accounts.drand_round.round = round;
    ctx.accounts.drand_round.randomness = randomness;
    Ok(())
}
