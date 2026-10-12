//! The treasury role (src/floor/ProtocolTreasury.sol) without floats and bootstrap loans.

use anchor_lang::prelude::*;

use crate::clock::now;
use crate::error::RookError;
use crate::events::*;
use crate::math::mul_div;
use crate::seeds::*;
use crate::state::*;
use crate::units::BPS;

#[derive(Accounts)]
pub struct InitTreasury<'info> {
    pub owner: Signer<'info>,
    #[account(mut)]
    pub payer: Signer<'info>,
    #[account(seeds = [REGISTRY], bump = registry.bump)]
    pub registry: Box<Account<'info, Registry>>,
    #[account(init, payer = payer, space = 8 + Treasury::INIT_SPACE, seeds = [TREASURY], bump)]
    pub treasury: Box<Account<'info, Treasury>>,
    pub system_program: Program<'info, System>,
}

/// The owner, once. Phase 1's split from creation (C-07): 90 % reserves, 10 % operations; each collection's reserve capped
/// at two floor prices (`DEFAULT_RESERVE_CAP_BPS`); half of what a collection is paid above its cap to its raffle pot
/// (`DEFAULT_RAFFLE_BPS`).
pub fn handle_init_treasury(ctx: Context<InitTreasury>) -> Result<()> {
    ctx.accounts.registry.require_owner(ctx.accounts.owner.key)?;
    let t = &mut ctx.accounts.treasury;
    t.reserves_bps = 9_000;
    t.ops_bps = 1_000;
    t.reserve_cap_bps = DEFAULT_RESERVE_CAP_BPS;
    t.raffle_bps = DEFAULT_RAFFLE_BPS;
    t.bump = ctx.bumps.treasury;
    Ok(())
}

#[derive(Accounts)]
pub struct TreasuryAdmin<'info> {
    pub owner: Signer<'info>,
    #[account(seeds = [REGISTRY], bump = registry.bump)]
    pub registry: Box<Account<'info, Registry>>,
    #[account(mut, seeds = [TREASURY], bump = treasury.bump)]
    pub treasury: Box<Account<'info, Treasury>>,
}

/// `set_shares` (owner): proposes reserves + ops = 100 %. Nothing changes for `TREASURY_DELAY`; then anyone applies it with
/// `apply_shares`. A new proposal replaces the pending one and restarts the wait.
pub fn handle_set_shares(ctx: Context<TreasuryAdmin>, reserves_bps: u16, ops_bps: u16) -> Result<()> {
    ctx.accounts.registry.require_owner(ctx.accounts.owner.key)?;
    require!(reserves_bps as u64 + ops_bps as u64 == BPS, RookError::BadShares);
    let ready_at = now()? + TREASURY_DELAY;
    let t = &mut ctx.accounts.treasury;
    t.pending_reserves_bps = reserves_bps;
    t.pending_ops_bps = ops_bps;
    t.pending_shares_at = ready_at;
    emit!(TreasurySharesProposed { reserves_bps, ops_bps, ready_at });
    Ok(())
}

#[derive(Accounts)]
pub struct TreasuryApply<'info> {
    #[account(mut, seeds = [TREASURY], bump = treasury.bump)]
    pub treasury: Box<Account<'info, Treasury>>,
}

/// `apply_shares` (anyone): the pending shares, once their wait is over.
pub fn handle_apply_shares(ctx: Context<TreasuryApply>) -> Result<()> {
    let t = &mut ctx.accounts.treasury;
    require!(t.pending_shares_at != 0, RookError::NothingPending);
    require!(now()? >= t.pending_shares_at, RookError::TooEarly);
    t.reserves_bps = t.pending_reserves_bps;
    t.ops_bps = t.pending_ops_bps;
    t.pending_reserves_bps = 0;
    t.pending_ops_bps = 0;
    t.pending_shares_at = 0;
    emit!(TreasurySharesSet { reserves_bps: t.reserves_bps, ops_bps: t.ops_bps });
    Ok(())
}

/// `set_ops` (owner): the first operations address applies at once (the launch ceremony names it); every later change waits
/// `TREASURY_DELAY` in public, then anyone applies it with `apply_ops`.
pub fn handle_set_ops(ctx: Context<TreasuryAdmin>, ops: Pubkey) -> Result<()> {
    ctx.accounts.registry.require_owner(ctx.accounts.owner.key)?;
    let t = &mut ctx.accounts.treasury;
    if t.ops == Pubkey::default() {
        t.ops = ops;
        emit!(TreasuryOpsSet { ops });
        return Ok(());
    }
    let ready_at = now()? + TREASURY_DELAY;
    t.pending_ops = ops;
    t.pending_ops_at = ready_at;
    emit!(TreasuryOpsProposed { ops, ready_at });
    Ok(())
}

/// `apply_ops` (anyone): the pending operations address, once its wait is over.
pub fn handle_apply_ops(ctx: Context<TreasuryApply>) -> Result<()> {
    let t = &mut ctx.accounts.treasury;
    require!(t.pending_ops_at != 0, RookError::NothingPending);
    require!(now()? >= t.pending_ops_at, RookError::TooEarly);
    t.ops = t.pending_ops;
    t.pending_ops = Pubkey::default();
    t.pending_ops_at = 0;
    emit!(TreasuryOpsSet { ops: t.ops });
    Ok(())
}

/// `set_reserve_cap` (owner): proposes each collection's reserve cap, in bps of its accepted floor price
/// (`MIN_RESERVE_CAP_BPS..=MAX_RESERVE_CAP_BPS`; no "no cap"). Nothing changes for `TREASURY_DELAY`; then anyone applies it with
/// `apply_reserve_cap`. A new proposal replaces the pending one and restarts the wait.
pub fn handle_set_reserve_cap(ctx: Context<TreasuryAdmin>, bps: u32) -> Result<()> {
    ctx.accounts.registry.require_owner(ctx.accounts.owner.key)?;
    require!((MIN_RESERVE_CAP_BPS..=MAX_RESERVE_CAP_BPS).contains(&bps), RookError::BadReserveCap);
    let ready_at = now()? + TREASURY_DELAY;
    let t = &mut ctx.accounts.treasury;
    t.pending_reserve_cap_bps = bps;
    t.pending_reserve_cap_at = ready_at;
    emit!(ReserveCapProposed { bps, ready_at });
    Ok(())
}

/// `apply_reserve_cap` (anyone): the pending reserve cap, once its wait is over.
pub fn handle_apply_reserve_cap(ctx: Context<TreasuryApply>) -> Result<()> {
    let t = &mut ctx.accounts.treasury;
    require!(t.pending_reserve_cap_at != 0, RookError::NothingPending);
    require!(now()? >= t.pending_reserve_cap_at, RookError::TooEarly);
    t.reserve_cap_bps = t.pending_reserve_cap_bps;
    t.pending_reserve_cap_bps = 0;
    t.pending_reserve_cap_at = 0;
    emit!(ReserveCapSet { bps: t.reserve_cap_bps });
    Ok(())
}

/// `set_raffle_bps` (owner): proposes the share (`0..=10_000` bps) of a collection's payout above its reserve cap that
/// `distribute` sends to its raffle pot (the rest goes to operations). Nothing changes for `TREASURY_DELAY`; then anyone applies
/// it with `apply_raffle_bps`. A new proposal replaces the pending one and restarts the wait.
pub fn handle_set_raffle_bps(ctx: Context<TreasuryAdmin>, bps: u16) -> Result<()> {
    ctx.accounts.registry.require_owner(ctx.accounts.owner.key)?;
    require!(bps as u64 <= BPS, RookError::BadRaffleBps);
    let ready_at = now()? + TREASURY_DELAY;
    let t = &mut ctx.accounts.treasury;
    t.pending_raffle_bps = bps;
    t.pending_raffle_at = ready_at;
    emit!(RaffleBpsProposed { bps, ready_at });
    Ok(())
}

/// `apply_raffle_bps` (anyone): the pending raffle share, once its wait is over.
pub fn handle_apply_raffle_bps(ctx: Context<TreasuryApply>) -> Result<()> {
    let t = &mut ctx.accounts.treasury;
    require!(t.pending_raffle_at != 0, RookError::NothingPending);
    require!(now()? >= t.pending_raffle_at, RookError::TooEarly);
    t.raffle_bps = t.pending_raffle_bps;
    t.pending_raffle_bps = 0;
    t.pending_raffle_at = 0;
    emit!(RaffleBpsSet { bps: t.raffle_bps });
    Ok(())
}

#[derive(Accounts)]
pub struct AddCollection<'info> {
    pub owner: Signer<'info>,
    #[account(seeds = [REGISTRY], bump = registry.bump)]
    pub registry: Box<Account<'info, Registry>>,
    #[account(mut, seeds = [TREASURY], bump = treasury.bump)]
    pub treasury: Box<Account<'info, Treasury>>,
    pub module: Box<Account<'info, Module>>,
}

/// `addCollection(module)` (owner): a floor module (an account of this program, so it names this treasury by construction).
pub fn handle_add_collection(ctx: Context<AddCollection>) -> Result<()> {
    ctx.accounts.registry.require_owner(ctx.accounts.owner.key)?;
    let key = ctx.accounts.module.key();
    let t = &mut ctx.accounts.treasury;
    require!(!t.collections.contains(&key), RookError::AlreadyCollection);
    require!(t.collections.len() < MAX_COLLECTIONS, RookError::Full);
    t.collections.push(key);
    Ok(())
}

#[derive(Accounts)]
pub struct Receive<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,
    #[account(mut, seeds = [TREASURY], bump = treasury.bump)]
    pub treasury: Box<Account<'info, Treasury>>,
    pub system_program: Program<'info, System>,
}

/// `receive()` (anyone, with lamports). Lamports sent to the treasury account by a plain transfer are income too.
pub fn handle_receive(ctx: Context<Receive>, lamports: u64) -> Result<()> {
    require!(lamports != 0, RookError::BadInput);
    pay_in(&ctx.accounts.payer.to_account_info(), &ctx.accounts.treasury.to_account_info(), lamports)?;
    let t = &mut ctx.accounts.treasury;
    t.total_received = t.total_received.saturating_add(lamports);
    Ok(())
}

#[derive(Accounts)]
pub struct Allocate<'info> {
    #[account(mut, seeds = [TREASURY], bump = treasury.bump)]
    pub treasury: Box<Account<'info, Treasury>>,
    /// CHECK: `treasury.ops`; required when an operations address is set (omitted otherwise).
    #[account(mut)]
    pub ops: Option<UncheckedAccount<'info>>,
}

/// `allocate()` (anyone): what arrived since the last call goes into the reserves pot and to operations by the shares.
pub fn handle_allocate(ctx: Context<Allocate>) -> Result<()> {
    let info = ctx.accounts.treasury.to_account_info();
    let t = &mut ctx.accounts.treasury;
    let amount = spendable(&info)?.saturating_sub(t.reserves_pot);
    require!(amount != 0, RookError::NothingToDo);
    let mut to_reserves = mul_div(amount, t.reserves_bps as u64, BPS)?;
    let mut to_ops = amount - to_reserves;
    if t.ops == Pubkey::default() {
        to_reserves += to_ops;
        to_ops = 0;
    }
    t.reserves_pot += to_reserves;
    t.total_allocated = t.total_allocated.saturating_add(amount);
    t.total_to_ops = t.total_to_ops.saturating_add(to_ops);
    emit!(Allocated { amount, to_reserves, to_ops });
    if to_ops != 0 {
        let ops = ctx.accounts.ops.as_ref().ok_or(RookError::BadInput)?;
        require_keys_eq!(ops.key(), t.ops, RookError::BadInput);
        pay_out(&info, &ops.to_account_info(), to_ops)?;
    }
    Ok(())
}

#[derive(Accounts)]
pub struct BeginDistribution<'info> {
    #[account(mut, seeds = [TREASURY], bump = treasury.bump)]
    pub treasury: Box<Account<'info, Treasury>>,
}

/// Anyone: open a distribution round. `remaining_accounts` = every registered module, in `collections` order (read-only);
/// each one's weight is its trailing seven-day volume. Refused while the current round is open (some index with a weight is
/// still owed a non-zero share: `distribute` it first), unless it began `ROUND_TIMEOUT` ago, when a stale round may be
/// restarted with the pot as it then is (audit A2-01: a restart used to clear `round_paid`, so anyone could pay one
/// collection again and again).
pub fn handle_begin_distribution<'info>(ctx: Context<'info, BeginDistribution<'info>>) -> Result<()> {
    let t = &mut ctx.accounts.treasury;
    let n = t.collections.len();
    require!(ctx.remaining_accounts.len() == n, RookError::BadInput);
    let today = now()?;
    require!(!t.round_open()? || today >= t.round_started_at + ROUND_TIMEOUT, RookError::RoundOpen);
    // a paged weighing in progress is cancelled only once it is `ROUND_TIMEOUT` old (docs/v9/solana-nfts-design.md §6)
    require!(!t.weighing() || today >= t.round_started_at + ROUND_TIMEOUT, RookError::RoundOpen);
    t.weigh_next = 0;
    t.weigh_total = 0;
    let mut total: u64 = 0;
    let mut weights = [0u64; MAX_COLLECTIONS];
    for (i, acc) in ctx.remaining_accounts.iter().enumerate() {
        require_keys_eq!(acc.key(), t.collections[i], RookError::NotCollection);
        let m: Account<Module> = Account::try_from(acc)?;
        weights[i] = m.volume_7d(today);
        total = total.saturating_add(weights[i]);
    }
    require!(total != 0, RookError::NothingToDo);
    t.round_pot = t.reserves_pot;
    t.round_total = total;
    t.round_weights = weights;
    t.round_paid = 0;
    t.round_started_at = today;
    Ok(())
}

/// `weigh_distribution(start)` (anyone): `begin_distribution` in pages, for more modules than one transaction can lock (a
/// transaction locks at most 64 accounts: treasury + payer + program + 61 modules). `remaining_accounts` = modules
/// `collections[start .. start + k]` in order (read-only). `start == 0` begins a weighing under `begin_distribution`'s rule
/// (no open round, unless it is `ROUND_TIMEOUT` old) and not over a weighing younger than `ROUND_TIMEOUT`; it clears the
/// round (nothing is owed while weighing). A later page continues exactly where the previous stopped. When the last module
/// is weighed the round opens as `begin_distribution` opens it.
pub fn handle_weigh_distribution<'info>(ctx: Context<'info, BeginDistribution<'info>>, start: u8) -> Result<()> {
    let t = &mut ctx.accounts.treasury;
    let n = t.collections.len();
    let s = start as usize;
    let k = ctx.remaining_accounts.len();
    require!(k != 0 && s + k <= n, RookError::BadInput);
    let today = now()?;
    if s == 0 {
        require!(!t.round_open()? || today >= t.round_started_at + ROUND_TIMEOUT, RookError::RoundOpen);
        require!(!t.weighing() || today >= t.round_started_at + ROUND_TIMEOUT, RookError::RoundOpen);
        t.round_pot = 0;
        t.round_total = 0;
        t.round_weights = [0u64; MAX_COLLECTIONS];
        t.round_paid = 0;
        t.round_started_at = today;
        t.weigh_total = 0;
    } else {
        require!(t.weighing(), RookError::NothingPending);
        require!(s == t.weigh_next as usize, RookError::OutOfOrder);
    }
    for (j, acc) in ctx.remaining_accounts.iter().enumerate() {
        let i = s + j;
        require_keys_eq!(acc.key(), t.collections[i], RookError::NotCollection);
        let m: Account<Module> = Account::try_from(acc)?;
        let w = m.volume_7d(today);
        t.round_weights[i] = w;
        t.weigh_total = t.weigh_total.saturating_add(w);
    }
    if s + k < n {
        t.weigh_next = (s + k) as u8;
        return Ok(());
    }
    require!(t.weigh_total != 0, RookError::NothingToDo);
    t.weigh_next = 0;
    t.round_pot = t.reserves_pot;
    t.round_total = t.weigh_total;
    t.round_paid = 0;
    t.round_started_at = today;
    Ok(())
}

#[derive(Accounts)]
pub struct Distribute<'info> {
    #[account(mut, seeds = [TREASURY], bump = treasury.bump)]
    pub treasury: Box<Account<'info, Treasury>>,
    #[account(mut)]
    pub module: Box<Account<'info, Module>>,
    #[account(mut, address = module.vault)]
    pub module_vault: Account<'info, ModuleVault>,
    /// CHECK: `treasury.ops`; required when an operations address is set and the share exceeds the module's reserve cap
    /// (omitted otherwise: the program id in its place).
    #[account(mut)]
    pub ops: Option<UncheckedAccount<'info>>,
    /// CHECK: the collection's raffle, PDA `["raffle", module.listing]`, initialized or not (checked in the handler: an
    /// initialized `Raffle` of this listing, else the PDA's address). Until it exists the raffle share is 0.
    #[account(mut)]
    pub raffle: UncheckedAccount<'info>,
}

/// `distribute()`, one collection per call (anyone): collection `index` is owed `round_pot × weight / total` of the reserves
/// pot. Its module's surplus (`fundReserve`) is topped up to its reserve cap (`reserve_cap_bps` × accepted floor / 10_000;
/// the whole share while the module has no accepted floor). Of the rest, `raffle_bps` goes to the collection's raffle pot (none
/// while the raffle does not exist) and the remainder to operations, or stays in the reserves pot for a
/// later round while no operations address is set. Either way index `index` is paid for this round, so the round closes.
pub fn handle_distribute(ctx: Context<Distribute>, index: u8) -> Result<()> {
    let info = ctx.accounts.treasury.to_account_info();
    let raffle_info = ctx.accounts.raffle.to_account_info();
    let t = &mut ctx.accounts.treasury;
    let i = index as usize;
    require!(i < t.collections.len(), RookError::NotCollection);
    require_keys_eq!(ctx.accounts.module.key(), t.collections[i], RookError::NotCollection);
    require!(t.round_total != 0 && t.round_paid & (1u64 << i) == 0, RookError::NothingToDo);
    let r = t.round_owed(i)?;
    t.round_paid |= 1u64 << i;
    require!(r != 0, RookError::NothingToDo);
    let m = &mut ctx.accounts.module;
    let mut raffle: Option<Raffle> = None;
    if raffle_info.owner == &crate::ID && raffle_info.data_len() > 0 {
        let a = Raffle::try_deserialize(&mut &raffle_info.try_borrow_data()?[..])?;
        require_keys_eq!(a.listing, m.listing, RookError::WrongCollection);
        raffle = Some(a);
    } else {
        let (k, _) = Pubkey::find_program_address(&[RAFFLE, m.listing.as_ref()], &crate::ID);
        require_keys_eq!(k, raffle_info.key(), RookError::WrongCollection);
    }
    let to_surplus = match t.reserve_cap(m.accepted) {
        None => r,
        Some(cap) => r.min(cap.saturating_sub(m.surplus)),
    };
    let over = r - to_surplus;
    let to_raffle = if raffle.is_some() { mul_div(over, t.raffle_bps as u64, BPS)? } else { 0 };
    let rest = over - to_raffle;
    let (to_ops, kept) = if t.ops == Pubkey::default() { (0, rest) } else { (rest, 0) };
    t.reserves_pot -= to_surplus + to_raffle + to_ops;
    t.total_to_reserves = t.total_to_reserves.saturating_add(to_surplus);
    t.total_to_ops = t.total_to_ops.saturating_add(to_ops);
    t.total_to_raffles = t.total_to_raffles.saturating_add(to_raffle);
    if to_surplus != 0 {
        m.fund_reserve(to_surplus)?;
        pay_out(&info, &ctx.accounts.module_vault.to_account_info(), to_surplus)?;
    }
    if to_ops != 0 {
        let ops = ctx.accounts.ops.as_ref().ok_or(RookError::BadInput)?;
        require_keys_eq!(ops.key(), t.ops, RookError::BadInput);
        pay_out(&info, &ops.to_account_info(), to_ops)?;
    }
    if let (Some(mut a), true) = (raffle, to_raffle != 0) {
        pay_out(&info, &raffle_info, to_raffle)?;
        a.pot = a.pot.saturating_add(to_raffle);
        a.total_in = a.total_in.saturating_add(to_raffle);
        a.try_serialize(&mut &mut raffle_info.try_borrow_mut_data()?[..])?;
        emit!(RafflePotFunded { raffle: raffle_info.key(), listing: a.listing, lamports: to_raffle, pot: a.pot });
    }
    emit!(ReserveDelivered { module: m.key(), amount: to_surplus, to_ops, kept });
    Ok(())
}
