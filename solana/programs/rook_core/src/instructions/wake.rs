//! The wake role (src/floor/WakeDesk.sol). The bond lives in each `Request` account; a coin graduated by the requester is a
//! rook_backing `Vault` (see `BackingVaultView` for the layout this reads).

use anchor_lang::prelude::*;

use crate::clock::now;
use crate::error::RookError;
use crate::events::*;
use crate::seeds::*;
use crate::state::*;

#[derive(Accounts)]
pub struct InitDesk<'info> {
    pub owner: Signer<'info>,
    #[account(mut)]
    pub payer: Signer<'info>,
    #[account(seeds = [REGISTRY], bump = registry.bump)]
    pub registry: Box<Account<'info, Registry>>,
    #[account(init, payer = payer, space = 8 + Desk::INIT_SPACE, seeds = [DESK], bump)]
    pub desk: Account<'info, Desk>,
    pub system_program: Program<'info, System>,
}

/// The owner, once (the WakeDesk constructor): the bond, its bounds and the window, fixed. The constructor's
/// `MIN_BOND ≤ bond ≤ MAX_BOND` holds with the bounds as parameters (the launch file's, D-c); a zero bound is refused.
pub fn handle_init_desk(ctx: Context<InitDesk>, p: DeskParams) -> Result<()> {
    ctx.accounts.registry.require_owner(ctx.accounts.owner.key)?;
    require!(p.min_bond != 0 && p.min_bond <= p.max_bond, RookError::BadInput);
    require!((p.min_bond..=p.max_bond).contains(&p.bond), RookError::BadInput);
    require!((MIN_WINDOW..=MAX_WINDOW).contains(&p.window), RookError::BadInput);
    let d = &mut ctx.accounts.desk;
    d.bond = p.bond;
    d.window = p.window;
    d.min_bond = p.min_bond;
    d.max_bond = p.max_bond;
    d.bump = ctx.bumps.desk;
    Ok(())
}

#[derive(Accounts)]
pub struct RequestWake<'info> {
    #[account(mut)]
    pub requester: Signer<'info>,
    #[account(mut, seeds = [DESK], bump = desk.bump)]
    pub desk: Account<'info, Desk>,
    pub listing: Box<Account<'info, Listing>>,
    #[account(constraint = module.listing == listing.key() @ RookError::WrongCollection)]
    pub module: Box<Account<'info, Module>>,
    #[account(
        init_if_needed, payer = requester, space = 8 + WakeSlot::INIT_SPACE,
        seeds = [WAKE_SLOT, listing.key().as_ref()], bump
    )]
    pub wake_slot: Account<'info, WakeSlot>,
    /// The collection's last request, required when there was one.
    pub last_request: Option<Account<'info, Request>>,
    #[account(
        init, payer = requester, space = 8 + Request::INIT_SPACE,
        seeds = [REQUEST, &desk.request_count.to_le_bytes()], bump
    )]
    pub request: Account<'info, Request>,
    pub system_program: Program<'info, System>,
}

/// `request(wrapper)` (anyone, with exactly the bond): refused while another request for the collection is open, and while
/// its floor is fresh (DI-M-2: it is awake).
pub fn handle_request(ctx: Context<RequestWake>, bond: u64) -> Result<()> {
    let d = &ctx.accounts.desk;
    require!(bond == d.bond, RookError::WrongBond);
    let t = now()?;
    let last = ctx.accounts.wake_slot.last_plus_one;
    if last != 0 {
        let lr = ctx.accounts.last_request.as_ref().ok_or(RookError::BadInput)?;
        require!(lr.id == last - 1, RookError::BadInput);
        require!(!lr.open(t, d.window), RookError::AlreadyRequested);
    }
    require!(!ctx.accounts.module.fresh(t), RookError::Awake);
    let id = d.request_count;
    let r = &mut ctx.accounts.request;
    r.id = id;
    r.listing = ctx.accounts.listing.key();
    r.requester = ctx.accounts.requester.key();
    r.at = t;
    r.bond = bond;
    ctx.accounts.wake_slot.last_plus_one = id + 1;
    ctx.accounts.desk.request_count = id + 1;
    pay_in(&ctx.accounts.requester.to_account_info(), &ctx.accounts.request.to_account_info(), bond)?;
    emit!(WakeRequested { id, listing: ctx.accounts.listing.key(), requester: ctx.accounts.requester.key(), bond });
    Ok(())
}

fn served(r: &Request, m: &Module, window: i64) -> bool {
    r.served_at != 0 || (m.posted_at >= r.at && m.posted_at <= r.at + window)
}

#[derive(Accounts)]
pub struct MarkServed<'info> {
    #[account(seeds = [DESK], bump = desk.bump)]
    pub desk: Account<'info, Desk>,
    #[account(mut)]
    pub request: Account<'info, Request>,
    #[account(constraint = module.listing == request.listing @ RookError::WrongCollection)]
    pub module: Box<Account<'info, Module>>,
}

/// `markServed(id)` (anyone; the canister after its post): the module's latest post lies inside `[at, at + window]`.
pub fn handle_mark_served(ctx: Context<MarkServed>) -> Result<()> {
    let r = &mut ctx.accounts.request;
    require!(!r.closed, RookError::Closed);
    if r.served_at != 0 {
        return Ok(());
    }
    let p = ctx.accounts.module.posted_at;
    require!(p != 0 && p >= r.at && p <= r.at + ctx.accounts.desk.window, RookError::NotServed);
    r.served_at = p;
    emit!(WakeServed { id: r.id, posted_at: p });
    Ok(())
}

#[derive(Accounts)]
pub struct Reclaim<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,
    #[account(seeds = [DESK], bump = desk.bump)]
    pub desk: Account<'info, Desk>,
    #[account(mut)]
    pub request: Account<'info, Request>,
    /// CHECK: must be `request.requester`; receives the bond.
    #[account(mut, address = request.requester @ RookError::BadInput)]
    pub requester: UncheckedAccount<'info>,
    /// CHECK: a rook_backing `Vault`: owner and layout checked in the handler (`BackingVaultView`).
    #[account(owner = ROOK_BACKING_ID @ RookError::NotALaunch)]
    pub backing_vault: UncheckedAccount<'info>,
    #[account(
        init_if_needed, payer = payer, space = 8 + Spent::INIT_SPACE,
        seeds = [SPENT, backing_vault.key().as_ref()], bump
    )]
    pub spent: Account<'info, Spent>,
    pub system_program: Program<'info, System>,
}

/// `reclaim(id, coin)` (anyone): the requester's own coin on this collection (a rook_backing vault whose partner is this
/// listing's and whose `creator == requester`: the holder of DBC's transferable creator role when the coin was backed, not
/// necessarily its launcher; audit A3-01), launched at or after the request (`launched_at >= at`: the DBC pool's own
/// creation time, so a coin launched before the request never qualifies, whenever it graduates) and graduated
/// (`graduated == true`), which paid back no bond before: the bond goes back. Allowed until `window + RECLAIM_GRACE`.
pub fn handle_reclaim(ctx: Context<Reclaim>) -> Result<()> {
    let window = ctx.accounts.desk.window;
    let r = &mut ctx.accounts.request;
    require!(!r.closed, RookError::Closed);
    require!(now()? < r.at + window + RECLAIM_GRACE, RookError::TooLate);
    let data = ctx.accounts.backing_vault.try_borrow_data()?;
    let v = BackingVaultView::parse(&data).ok_or(RookError::NotALaunch)?;
    drop(data);
    require!(v.creator == r.requester, RookError::NotALaunch);
    require!(v.partner == BackingVaultView::partner_of(&r.listing), RookError::NotALaunch);
    require!(v.graduated, RookError::NotGraduated);
    require!(v.launched_at >= r.at, RookError::NotALaunch);
    require!(!ctx.accounts.spent.used, RookError::AlreadySpent);
    ctx.accounts.spent.used = true;
    r.closed = true;
    let b = r.bond;
    emit!(WakeClosed { id: r.id, outcome: 0, bond: b });
    pay_out(&ctx.accounts.request.to_account_info(), &ctx.accounts.requester.to_account_info(), b)
}

#[derive(Accounts)]
pub struct Forfeit<'info> {
    #[account(seeds = [DESK], bump = desk.bump)]
    pub desk: Account<'info, Desk>,
    #[account(mut)]
    pub request: Account<'info, Request>,
    #[account(constraint = module.listing == request.listing @ RookError::WrongCollection)]
    pub module: Box<Account<'info, Module>>,
    /// CHECK: must be `request.requester`; receives the bond if the wake was not served.
    #[account(mut, address = request.requester @ RookError::BadInput)]
    pub requester: UncheckedAccount<'info>,
    #[account(mut, seeds = [TREASURY], bump = treasury.bump)]
    pub treasury: Box<Account<'info, Treasury>>,
}

/// `forfeit(id)` (anyone, after `window + RECLAIM_GRACE`): the bond goes to the treasury if the wake was served inside the
/// window, back to the requester otherwise (DR-H-3, DI-M-1).
pub fn handle_forfeit(ctx: Context<Forfeit>) -> Result<()> {
    let window = ctx.accounts.desk.window;
    let r = &mut ctx.accounts.request;
    require!(!r.closed, RookError::Closed);
    require!(now()? >= r.at + window + RECLAIM_GRACE, RookError::TooEarly);
    r.closed = true;
    let b = r.bond;
    let was_served = served(r, &ctx.accounts.module, window);
    let id = r.id;
    let from = ctx.accounts.request.to_account_info();
    if was_served {
        emit!(WakeClosed { id, outcome: 1, bond: b });
        pay_out(&from, &ctx.accounts.treasury.to_account_info(), b)?;
        let t = &mut ctx.accounts.treasury;
        t.total_received = t.total_received.saturating_add(b);
        Ok(())
    } else {
        emit!(WakeClosed { id, outcome: 2, bond: b });
        pay_out(&from, &ctx.accounts.requester.to_account_info(), b)
    }
}

#[derive(Accounts)]
pub struct Withdraw<'info> {
    #[account(mut)]
    pub requester: Signer<'info>,
    #[account(mut, close = requester, constraint = request.requester == requester.key() @ RookError::BadInput)]
    pub request: Account<'info, Request>,
    #[account(mut, seeds = [WAKE_SLOT, request.listing.as_ref()], bump)]
    pub wake_slot: Account<'info, WakeSlot>,
}

/// `withdraw` (the requester): on Solana a bond is paid out by crediting the account, which cannot refuse it, so the EVM's
/// DI-L-2 credit never arises; what is left to withdraw is the closed request's rent. Closes the request account.
pub fn handle_withdraw(ctx: Context<Withdraw>) -> Result<()> {
    require!(ctx.accounts.request.closed, RookError::NotClosed);
    if ctx.accounts.wake_slot.last_plus_one == ctx.accounts.request.id + 1 {
        ctx.accounts.wake_slot.last_plus_one = 0;
    }
    Ok(())
}
