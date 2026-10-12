//! The floor role (src/floor/FloorModule.sol): the handlers check signers and accounts, call the pure book-keeping on
//! `Module` (state/floor.rs), and move lamports and wX.

use anchor_lang::prelude::*;
use anchor_spl::token::{Mint, Token, TokenAccount};

use crate::clock::now;
use crate::error::RookError;
use crate::events::*;
use crate::seeds::*;
use crate::state::*;
use crate::params::Params;
use crate::units::ONE;

#[derive(Accounts)]
pub struct InitModule<'info> {
    pub owner: Signer<'info>,
    #[account(mut)]
    pub payer: Signer<'info>,
    #[account(seeds = [REGISTRY], bump = registry.bump)]
    pub registry: Box<Account<'info, Registry>>,
    pub listing: Box<Account<'info, Listing>>,
    #[account(constraint = wrapper.listing == listing.key() @ RookError::WrongCollection)]
    pub wrapper: Box<Account<'info, Wrapper>>,
    #[account(address = wrapper.wx_mint)]
    pub wx_mint: Box<Account<'info, Mint>>,
    #[account(init, payer = payer, space = 8 + Module::INIT_SPACE, seeds = [MODULE, listing.key().as_ref()], bump)]
    pub module: Box<Account<'info, Module>>,
    #[account(init, payer = payer, space = 8 + ModuleVault::INIT_SPACE, seeds = [MODULE_VAULT, module.key().as_ref()], bump)]
    pub module_vault: Account<'info, ModuleVault>,
    #[account(
        init, payer = payer, seeds = [MODULE_WX, module.key().as_ref()], bump,
        token::mint = wx_mint, token::authority = module,
    )]
    pub module_wx: Box<Account<'info, TokenAccount>>,
    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

/// The owner: a collection's floor module with its parameters, every one fixed for good (FloorModule has no setter).
pub fn handle_init_module(ctx: Context<InitModule>, params: Params) -> Result<()> {
    ctx.accounts.registry.require_owner(ctx.accounts.owner.key)?;
    validate_params(&params)?;
    let m = &mut ctx.accounts.module;
    m.listing = ctx.accounts.listing.key();
    m.wrapper = ctx.accounts.wrapper.key();
    m.vault = ctx.accounts.module_vault.key();
    m.module_wx = ctx.accounts.module_wx.key();
    m.bump = ctx.bumps.module;
    m.vault_bump = ctx.bumps.module_vault;
    m.wx_bump = ctx.bumps.module_wx;
    m.params = params;
    Ok(())
}

#[derive(Accounts)]
pub struct PostFloor<'info> {
    pub poster: Signer<'info>,
    #[account(seeds = [REGISTRY], bump = registry.bump)]
    pub registry: Box<Account<'info, Registry>>,
    pub listing: Box<Account<'info, Listing>>,
    #[account(mut, constraint = module.listing == listing.key() @ RookError::WrongCollection)]
    pub module: Box<Account<'info, Module>>,
}

/// `postFloor(price, confidence, depth)`: the listing's poster (EthNft: the canister) or, for an Inscriptions listing, the
/// registry keeper; nobody while paused. Lamports per whole unit. Clamped to ±`max_move_bps` per hour since the last post (one
/// hour's worth at most) and ±20 % of the day's anchor; a confidence under the minimum leaves the module stale.
pub fn handle_post_floor(ctx: Context<PostFloor>, price: u64, confidence_bps: u16, depth_items: u32) -> Result<()> {
    let poster = ctx.accounts.listing.live_poster(&ctx.accounts.registry).ok_or(RookError::NotKeeper)?;
    require_keys_eq!(poster, ctx.accounts.poster.key(), RookError::NotKeeper);
    let m = &mut ctx.accounts.module;
    let (accepted, moved_bps) = m.post_floor(price, confidence_bps, depth_items, now()?)?;
    emit!(FloorPosted { module: m.key(), target: price, accepted, moved_bps, confidence_bps, depth_items });
    Ok(())
}

#[derive(Accounts)]
pub struct MintUnits<'info> {
    #[account(mut)]
    pub buyer: Signer<'info>,
    #[account(seeds = [REGISTRY], bump = registry.bump)]
    pub registry: Box<Account<'info, Registry>>,
    pub listing: Box<Account<'info, Listing>>,
    #[account(mut, constraint = module.listing == listing.key() @ RookError::WrongCollection)]
    pub module: Box<Account<'info, Module>>,
    #[account(mut, address = module.vault)]
    pub module_vault: Account<'info, ModuleVault>,
    #[account(mut, address = module.wrapper)]
    pub wrapper: Box<Account<'info, Wrapper>>,
    #[account(mut, address = wrapper.wx_mint)]
    pub wx_mint: Account<'info, Mint>,
    #[account(mut, token::mint = wx_mint)]
    pub dest: Account<'info, TokenAccount>,
    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

/// `mint(to, minUnits)` (anyone): pay `lamports`, receive wX at `max(price, target) × (1 + spread)`, ETH-backed. Needs a
/// fresh floor and the poster up; bounded by the hourly bucket, `public_cap` and `max_eth_backed`.
pub fn handle_mint(ctx: Context<MintUnits>, lamports: u64, min_units: u64) -> Result<()> {
    let kd = ctx.accounts.listing.keeper_down(&ctx.accounts.registry);
    let e = ctx.accounts.wrapper.eth_backed;
    let units = ctx.accounts.module.mint(lamports, min_units, now()?, e, kd)?;
    pay_in(&ctx.accounts.buyer.to_account_info(), &ctx.accounts.module_vault.to_account_info(), lamports)?;
    module_mint(&mut ctx.accounts.wrapper, &ctx.accounts.wx_mint, &ctx.accounts.dest.to_account_info(), units)?;
    emit!(Minted { module: ctx.accounts.module.key(), payer: ctx.accounts.buyer.key(), lamports, units });
    Ok(())
}

/// `wrapper.moduleMint`: ETH-backed units.
pub(crate) fn module_mint<'info>(
    wrapper: &mut Account<'info, Wrapper>,
    wx_mint: &Account<'info, Mint>,
    to: &AccountInfo<'info>,
    units: u64,
) -> Result<()> {
    require!(units != 0, RookError::BadInput);
    wrapper.eth_backed = wrapper.eth_backed.checked_add(units).ok_or(RookError::MathOverflow)?;
    wx_mint_to(&wx_mint.to_account_info(), to, &wrapper.to_account_info(), &wrapper.listing, wrapper.bump, units)
}

#[derive(Accounts)]
pub struct BurnUnits<'info> {
    pub owner: Signer<'info>,
    #[account(seeds = [REGISTRY], bump = registry.bump)]
    pub registry: Box<Account<'info, Registry>>,
    pub listing: Box<Account<'info, Listing>>,
    #[account(mut, constraint = module.listing == listing.key() @ RookError::WrongCollection)]
    pub module: Box<Account<'info, Module>>,
    #[account(mut, address = module.vault)]
    pub module_vault: Account<'info, ModuleVault>,
    #[account(mut, address = module.wrapper)]
    pub wrapper: Box<Account<'info, Wrapper>>,
    #[account(mut, address = wrapper.wx_mint)]
    pub wx_mint: Account<'info, Mint>,
    #[account(mut, token::mint = wx_mint, token::authority = owner)]
    pub owner_wx: Account<'info, TokenAccount>,
    /// CHECK: any account; receives the lamports.
    #[account(mut)]
    pub to: UncheckedAccount<'info>,
    pub token_program: Program<'info, Token>,
}

/// `burn(units, minEth, to)` (anyone): return wX for lamports. The reserve tier goes first; each tier pays the lower of the
/// floor less the burn spread and its own backing per unit. While the poster is paused: the lower of the backing and the
/// pause floor, less the base spread; after `STALE_ESCAPE` without a good post: the backing, less the base spread.
pub fn handle_burn(ctx: Context<BurnUnits>, units: u64, min_lamports: u64) -> Result<()> {
    let kd = ctx.accounts.listing.keeper_down(&ctx.accounts.registry);
    let e = ctx.accounts.wrapper.eth_backed;
    let (paid, released) = ctx.accounts.module.burn(units, min_lamports, now()?, e, kd)?;
    wx_burn(
        &ctx.accounts.wx_mint.to_account_info(),
        &ctx.accounts.owner_wx.to_account_info(),
        &ctx.accounts.owner.to_account_info(),
        units,
    )?;
    let w = &mut ctx.accounts.wrapper;
    w.eth_backed = w.eth_backed.checked_sub(units).ok_or(RookError::BadInput)?;
    pay_out(&ctx.accounts.module_vault.to_account_info(), &ctx.accounts.to.to_account_info(), paid)?;
    emit!(Burned { module: ctx.accounts.module.key(), owner: ctx.accounts.owner.key(), units, lamports: paid, released });
    Ok(())
}

#[derive(Accounts)]
pub struct Convert<'info> {
    /// An allowed converter (rook_backing signs with its PDA through `invoke_signed`).
    pub converter: Signer<'info>,
    /// Pays the lamports by a system transfer; may be the converter.
    #[account(mut)]
    pub payer: Signer<'info>,
    #[account(seeds = [REGISTRY], bump = registry.bump)]
    pub registry: Box<Account<'info, Registry>>,
    pub listing: Box<Account<'info, Listing>>,
    #[account(mut, constraint = module.listing == listing.key() @ RookError::WrongCollection)]
    pub module: Box<Account<'info, Module>>,
    #[account(mut, address = module.vault)]
    pub module_vault: Account<'info, ModuleVault>,
    #[account(mut, address = module.wrapper)]
    pub wrapper: Box<Account<'info, Wrapper>>,
    #[account(mut, address = wrapper.wx_mint)]
    pub wx_mint: Account<'info, Mint>,
    #[account(mut, token::mint = wx_mint)]
    pub dest: Account<'info, TokenAccount>,
    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

/// **`convert(lamports, min_units)`** — the interface `rook_backing` depends on: a graduated coin's backing share, in SOL,
/// becomes ETH-backed wX units at the collection's floor. The EVM graduation conversion (`enqueue` → `_tryFill`,
/// FloorModule.sol:826-858), without the queue: it converts at once or refuses.
///
/// Accounts, in this order: `converter` (signer; must be in `Registry.converters`), `payer` (signer, mut; the lamports come
/// from here by a system transfer; may equal `converter`), `registry`, `listing`, `module` (mut), `module_vault` (mut, the
/// lamport-holding PDA `["module_vault", module]`), `wrapper` (mut), `wx_mint` (mut), `dest` (mut, an SPL token account of
/// `wx_mint`), `token_program`, `system_program`.
///
/// Semantics:
/// - refused unless the floor is fresh (posted < 3 h ago with confidence ≥ the minimum: `Stale`), the poster is up
///   (`Paused`) and the listing is not paused (`ListingPaused`);
/// - `gradPrice = ⌈max(price, upTarget) × (1 + convBps)⌉` lamports per whole unit, where `price` is the fills TWAP within the
///   fill band of the accepted floor and `upTarget` the poster's target capped at the top of the day band (A-10);
/// - `units = ⌊lamports × 10⁹ / gradPrice⌋`; refused if 0 (`Slippage`), if `eth_backed + units > maxEthBacked`
///   (`OverBacked`), or if `units < min_units` (`Slippage`);
/// - `reserveBps` of the lamports AND of the units go to the standing reserve tier (never advanced, never converted into an
///   item); the rest to the convertible backing;
/// - `units` are minted to `dest` as ETH-backed units (`wrapper.eth_backed += units`);
/// - returns nothing; emits `Converted { module, converter, dest, lamports, units, unit_price }`.
pub fn handle_convert(ctx: Context<Convert>, lamports: u64, min_units: u64) -> Result<()> {
    require!(ctx.accounts.registry.is_converter(ctx.accounts.converter.key), RookError::NotConverter);
    require!(!ctx.accounts.listing.paused, RookError::ListingPaused);
    let kd = ctx.accounts.listing.keeper_down(&ctx.accounts.registry);
    let e = ctx.accounts.wrapper.eth_backed;
    let t = now()?;
    let m = &mut ctx.accounts.module;
    let unit_price = if m.fresh(t) { m.grad_price(t)? } else { 0 };
    let units = m.convert(lamports, min_units, t, e, kd)?;
    pay_in(&ctx.accounts.payer.to_account_info(), &ctx.accounts.module_vault.to_account_info(), lamports)?;
    module_mint(&mut ctx.accounts.wrapper, &ctx.accounts.wx_mint, &ctx.accounts.dest.to_account_info(), units)?;
    emit!(Converted {
        module: ctx.accounts.module.key(),
        converter: ctx.accounts.converter.key(),
        dest: ctx.accounts.dest.key(),
        lamports,
        units,
        unit_price
    });
    Ok(())
}

#[derive(Accounts)]
pub struct Advance<'info> {
    #[account(mut)]
    pub custodian: Signer<'info>,
    #[account(seeds = [REGISTRY], bump = registry.bump)]
    pub registry: Box<Account<'info, Registry>>,
    #[account(constraint = listing.custodian == custodian.key() @ RookError::NotCustodian)]
    pub listing: Box<Account<'info, Listing>>,
    #[account(mut, constraint = module.listing == listing.key() @ RookError::WrongCollection)]
    pub module: Box<Account<'info, Module>>,
    #[account(mut, address = module.vault)]
    pub module_vault: Account<'info, ModuleVault>,
    #[account(address = module.wrapper)]
    pub wrapper: Box<Account<'info, Wrapper>>,
}

/// `advance(amount)` (custodian): one convertible unit goes into transit and `amount` is paid to the custodian now. One at a
/// time, at most `maxItemCost()`; H11 counts here.
pub fn handle_advance(ctx: Context<Advance>, amount: u64) -> Result<()> {
    let kd = ctx.accounts.listing.keeper_down(&ctx.accounts.registry);
    let e = ctx.accounts.wrapper.eth_backed;
    let released = ctx.accounts.module.advance(amount, now()?, e, kd)?;
    pay_out(&ctx.accounts.module_vault.to_account_info(), &ctx.accounts.custodian.to_account_info(), amount)?;
    emit!(Advanced { module: ctx.accounts.module.key(), amount, released });
    Ok(())
}

#[derive(Accounts)]
pub struct Settle<'info> {
    #[account(mut)]
    pub custodian: Signer<'info>,
    #[account(seeds = [REGISTRY], bump = registry.bump)]
    pub registry: Box<Account<'info, Registry>>,
    #[account(constraint = listing.custodian == custodian.key() @ RookError::NotCustodian)]
    pub listing: Box<Account<'info, Listing>>,
    #[account(mut, constraint = module.listing == listing.key() @ RookError::WrongCollection)]
    pub module: Box<Account<'info, Module>>,
    #[account(mut, address = module.vault)]
    pub module_vault: Account<'info, ModuleVault>,
    #[account(mut, address = module.wrapper)]
    pub wrapper: Box<Account<'info, Wrapper>>,
    #[account(mut, address = wrapper.wx_mint)]
    pub wx_mint: Account<'info, Mint>,
    #[account(mut, address = module.module_wx)]
    pub module_wx: Account<'info, TokenAccount>,
    pub token_program: Program<'info, Token>,
}

fn burn_module_unit(ctx: &mut Settle) -> Result<()> {
    let m = &ctx.module;
    wx_burn_module(
        &ctx.wx_mint.to_account_info(),
        &ctx.module_wx.to_account_info(),
        &m.to_account_info(),
        &m.listing,
        m.bump,
        ONE,
    )?;
    let w = &mut ctx.wrapper;
    w.eth_backed = w.eth_backed.checked_sub(ONE).ok_or(RookError::BadInput)?;
    Ok(())
}

/// `settleAdvance(cost)` (custodian): the advanced item was wrapped into the module's token account; the transit unit becomes
/// item-backed (the module's unit is burned). No lamports move; `cost` is recorded as a fill unless the poster is paused.
pub fn handle_settle_advance(mut ctx: Context<Settle>, cost: u64) -> Result<()> {
    let kd = ctx.accounts.listing.keeper_down(&ctx.accounts.registry);
    let bal = ctx.accounts.module_wx.amount;
    let adv = ctx.accounts.module.settle_advance(cost, now()?, kd, bal)?;
    burn_module_unit(&mut ctx.accounts)?;
    emit!(AdvanceSettled { module: ctx.accounts.module.key(), advanced: adv, cost });
    Ok(())
}

/// `itemArrived(cost)` (custodian): the older pay-on-arrival path. One ETH-backed unit becomes item-backed and `cost` is paid
/// to the custodian from that unit's backing, then revenue, then surplus; at most `maxItemCost`, H11 counted.
pub fn handle_item_arrived(mut ctx: Context<Settle>, cost: u64) -> Result<()> {
    let kd = ctx.accounts.listing.keeper_down(&ctx.accounts.registry);
    let bal = ctx.accounts.module_wx.amount;
    let e = ctx.accounts.wrapper.eth_backed;
    let released = ctx.accounts.module.item_arrived(cost, now()?, e, kd, bal)?;
    burn_module_unit(&mut ctx.accounts)?;
    pay_out(&ctx.accounts.module_vault.to_account_info(), &ctx.accounts.custodian.to_account_info(), cost)?;
    emit!(ItemArrived { module: ctx.accounts.module.key(), cost, released });
    Ok(())
}

#[derive(Accounts)]
pub struct ReturnAdvance<'info> {
    #[account(mut)]
    pub custodian: Signer<'info>,
    #[account(constraint = listing.custodian == custodian.key() @ RookError::NotCustodian)]
    pub listing: Box<Account<'info, Listing>>,
    #[account(mut, constraint = module.listing == listing.key() @ RookError::WrongCollection)]
    pub module: Box<Account<'info, Module>>,
    #[account(mut, address = module.vault)]
    pub module_vault: Account<'info, ModuleVault>,
    pub system_program: Program<'info, System>,
}

/// `returnAdvance()` (custodian, with `lamports`): the advance could not become an item; the transit unit is ETH-backed again.
/// A shortfall comes from revenue, then surplus; the rest is a loss shared by the convertible units.
pub fn handle_return_advance(ctx: Context<ReturnAdvance>, lamports: u64) -> Result<()> {
    let (basis, loss) = ctx.accounts.module.return_advance(lamports)?;
    pay_in(&ctx.accounts.custodian.to_account_info(), &ctx.accounts.module_vault.to_account_info(), lamports)?;
    emit!(AdvanceReturned { module: ctx.accounts.module.key(), returned: lamports, basis, loss });
    Ok(())
}

#[derive(Accounts)]
pub struct SnapshotCap<'info> {
    #[account(seeds = [REGISTRY], bump = registry.bump)]
    pub registry: Box<Account<'info, Registry>>,
    pub listing: Box<Account<'info, Listing>>,
    #[account(mut, constraint = module.listing == listing.key() @ RookError::WrongCollection)]
    pub module: Box<Account<'info, Module>>,
}

/// `snapshotCap()` (anyone): remember the fresh item cap for `SNAPSHOT_TTL`.
pub fn handle_snapshot_cap(ctx: Context<SnapshotCap>) -> Result<()> {
    let kd = ctx.accounts.listing.keeper_down(&ctx.accounts.registry);
    ctx.accounts.module.snapshot_cap(now()?, kd)?;
    Ok(())
}

#[derive(Accounts)]
pub struct FundReserve<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,
    #[account(mut)]
    pub module: Box<Account<'info, Module>>,
    #[account(mut, address = module.vault)]
    pub module_vault: Account<'info, ModuleVault>,
    pub system_program: Program<'info, System>,
}

/// `fundReserve()` (anyone, with lamports): joins the reserve surplus (tops up conversions; no burn is paid from it, B-02).
pub fn handle_fund_reserve(ctx: Context<FundReserve>, lamports: u64) -> Result<()> {
    ctx.accounts.module.fund_reserve(lamports)?;
    pay_in(&ctx.accounts.payer.to_account_info(), &ctx.accounts.module_vault.to_account_info(), lamports)?;
    emit!(ReserveFunded { module: ctx.accounts.module.key(), from: ctx.accounts.payer.key(), lamports });
    Ok(())
}

#[derive(Accounts)]
pub struct Skim<'info> {
    #[account(mut)]
    pub module: Box<Account<'info, Module>>,
    #[account(mut, address = module.vault)]
    pub module_vault: Account<'info, ModuleVault>,
    #[account(mut, seeds = [TREASURY], bump = treasury.bump)]
    pub treasury: Box<Account<'info, Treasury>>,
}

/// `skim()` (anyone): revenue beyond `revenueTarget` to the treasury; once escaped, all revenue and the surplus.
pub fn handle_skim(ctx: Context<Skim>) -> Result<()> {
    let amount = ctx.accounts.module.skim(now()?)?;
    pay_out(&ctx.accounts.module_vault.to_account_info(), &ctx.accounts.treasury.to_account_info(), amount)?;
    ctx.accounts.treasury.total_received = ctx.accounts.treasury.total_received.saturating_add(amount);
    emit!(Skimmed { module: ctx.accounts.module.key(), lamports: amount });
    Ok(())
}
