//! The holder raffle (operator decision 2026-10-10). The contract a canister builds against, account by account, is
//! docs/v9/raffle-design.md; the pot is fed by `distribute` (instructions/treasury.rs).
//!
//! Cycle, per collection: `fund_prize` (anyone: one whole wX bought from the pot by the module's public mint) →
//! `commit_raffle` (keeper, weekly: the snapshot's Merkle root, a future drand round pinned) → `prove_round` (keeper, as for
//! redemptions) → `draw_raffle` (anyone) → `claim_raffle` (anyone, with the winner's leaf and proof) or, after 30 days,
//! `expire_raffle` (the prize stays for the next round).

use anchor_lang::prelude::*;
use anchor_spl::associated_token::AssociatedToken;
use anchor_spl::token::{self, Mint, Token, TokenAccount, Transfer};

use crate::clock::{now, round_at};
use crate::error::RookError;
use crate::events::*;
use crate::hashing::verify_proof;
use crate::instructions::floor::module_mint;
use crate::seeds::*;
use crate::state::*;
use crate::units::ONE;

#[derive(Accounts)]
pub struct InitRaffle<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,
    pub listing: Box<Account<'info, Listing>>,
    #[account(constraint = module.listing == listing.key() @ RookError::WrongCollection)]
    pub module: Box<Account<'info, Module>>,
    #[account(address = module.wrapper)]
    pub wrapper: Box<Account<'info, Wrapper>>,
    #[account(address = wrapper.wx_mint)]
    pub wx_mint: Box<Account<'info, Mint>>,
    #[account(init, payer = payer, space = 8 + Raffle::INIT_SPACE, seeds = [RAFFLE, listing.key().as_ref()], bump)]
    pub raffle: Box<Account<'info, Raffle>>,
    #[account(
        init, payer = payer, seeds = [RAFFLE_WX, raffle.key().as_ref()], bump,
        token::mint = wx_mint, token::authority = raffle,
    )]
    pub raffle_wx: Box<Account<'info, TokenAccount>>,
    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

/// `init_raffle` (anyone, paying the rent): a collection's raffle and its wX token account. Until it exists, `distribute`
/// keeps the collection's raffle part in the reserves pot.
pub fn handle_init_raffle(ctx: Context<InitRaffle>) -> Result<()> {
    let r = &mut ctx.accounts.raffle;
    r.listing = ctx.accounts.listing.key();
    r.module = ctx.accounts.module.key();
    r.wx_mint = ctx.accounts.wx_mint.key();
    r.wx_account = ctx.accounts.raffle_wx.key();
    r.bump = ctx.bumps.raffle;
    r.wx_bump = ctx.bumps.raffle_wx;
    emit!(RaffleInitialized { raffle: r.key(), listing: r.listing, wx_account: r.wx_account });
    Ok(())
}

#[derive(Accounts)]
pub struct FundPrize<'info> {
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
    pub wx_mint: Box<Account<'info, Mint>>,
    #[account(mut, seeds = [RAFFLE, listing.key().as_ref()], bump = raffle.bump)]
    pub raffle: Box<Account<'info, Raffle>>,
    #[account(mut, address = raffle.wx_account)]
    pub raffle_wx: Box<Account<'info, TokenAccount>>,
    pub token_program: Program<'info, Token>,
}

/// `fund_prize` (anyone): the pot buys exactly one whole wX into the raffle's token account through the floor module's public
/// mint (`Module::mint` at `mint_price`: spreads, the hourly bucket, `public_cap`, `max_eth_backed`, a fresh floor and the
/// poster up all apply), paying the module vault like any buyer. Refused while a whole wX is already held (`PrizeHeld`) or
/// when the pot cannot pay (`PotTooSmall`). The pot is first re-synced to the account's lamports above rent (a plain transfer
/// to the raffle joins it).
pub fn handle_fund_prize(ctx: Context<FundPrize>) -> Result<()> {
    require!(ctx.accounts.raffle_wx.amount < ONE, RookError::PrizeHeld);
    let kd = ctx.accounts.listing.keeper_down(&ctx.accounts.registry);
    let e = ctx.accounts.wrapper.eth_backed;
    let t = now()?;
    let info = ctx.accounts.raffle.to_account_info();
    let pot = spendable(&info)?;
    let m = &mut ctx.accounts.module;
    require!(m.fresh(t), RookError::Stale);
    require!(!kd, RookError::Paused);
    // `mint` buys ⌊lamports · ONE / unit_price⌋ with unit_price = mint_price: paying exactly mint_price buys exactly ONE
    let price = m.mint_price(t, e)?;
    require!(pot >= price, RookError::PotTooSmall);
    let units = m.mint(price, ONE, t, e, kd)?;
    require!(units == ONE, RookError::MathOverflow);
    pay_out(&info, &ctx.accounts.module_vault.to_account_info(), price)?;
    module_mint(&mut ctx.accounts.wrapper, &ctx.accounts.wx_mint, &ctx.accounts.raffle_wx.to_account_info(), units)?;
    let r = &mut ctx.accounts.raffle;
    r.pot = pot - price;
    r.total_spent = r.total_spent.saturating_add(price);
    r.prizes_funded += 1;
    emit!(Minted { module: ctx.accounts.module.key(), payer: r.key(), lamports: price, units });
    emit!(RafflePrizeFunded { raffle: r.key(), round: r.round, lamports: price, units, pot: r.pot });
    Ok(())
}

#[derive(Accounts)]
pub struct CommitRaffle<'info> {
    pub keeper: Signer<'info>,
    #[account(seeds = [REGISTRY], bump = registry.bump)]
    pub registry: Box<Account<'info, Registry>>,
    #[account(mut)]
    pub raffle: Box<Account<'info, Raffle>>,
    #[account(address = raffle.wx_account)]
    pub raffle_wx: Box<Account<'info, TokenAccount>>,
}

/// `commit_raffle(root, total_weight, snapshot_slot, entries)` (keeper): fixes the round's snapshot (the Merkle root of the
/// leaves `(owner, cum_start, weight)`, published off chain) and pins the drand round `round_at(now) + RAFFLE_PIN_AHEAD`, which
/// does not exist yet. Once `RAFFLE_PERIOD` after the previous commit, with a whole wX prize held.
pub fn handle_commit_raffle(
    ctx: Context<CommitRaffle>,
    root: [u8; 32],
    total_weight: u64,
    snapshot_slot: u64,
    entries: u32,
) -> Result<()> {
    let k = ctx.accounts.registry.live_keeper().ok_or(RookError::NotKeeper)?;
    require_keys_eq!(k, ctx.accounts.keeper.key(), RookError::NotKeeper);
    let r = &mut ctx.accounts.raffle;
    require!(r.phase == RafflePhase::Idle, RookError::RafflePhase);
    let t = now()?;
    require!(r.committed_at == 0 || t >= r.committed_at + RAFFLE_PERIOD, RookError::RaffleTooEarly);
    require!(ctx.accounts.raffle_wx.amount >= ONE, RookError::NoPrize);
    require!(root != [0u8; 32] && total_weight != 0 && entries != 0, RookError::BadInput);
    let slot = Clock::get()?.slot;
    require!(
        snapshot_slot < slot && slot - snapshot_slot <= RAFFLE_SNAPSHOT_MAX_AGE && snapshot_slot > r.snapshot_slot,
        RookError::BadSnapshot
    );
    r.round += 1;
    r.phase = RafflePhase::Committed;
    r.root = root;
    r.total_weight = total_weight;
    r.entries = entries;
    r.snapshot_slot = snapshot_slot;
    r.committed_at = t;
    r.drand_round = round_at(t) + RAFFLE_PIN_AHEAD;
    r.ticket = 0;
    r.drawn_at = 0;
    emit!(RaffleCommitted {
        raffle: r.key(),
        round: r.round,
        root,
        total_weight,
        entries,
        snapshot_slot,
        drand_round: r.drand_round
    });
    Ok(())
}

#[derive(Accounts)]
pub struct DrawRaffle<'info> {
    #[account(mut)]
    pub raffle: Box<Account<'info, Raffle>>,
    /// The pinned round's randomness (`["drand", round]`, written by `prove_round`; a `DrandRound` exists only at its PDA).
    #[account(constraint = drand_round.round == raffle.drand_round @ RookError::RoundNotProven)]
    pub drand_round: Account<'info, DrandRound>,
}

/// `draw_raffle` (anyone): the winning ticket from the pinned round's randomness, keyed by the raffle and its round.
pub fn handle_draw_raffle(ctx: Context<DrawRaffle>) -> Result<()> {
    let key = ctx.accounts.raffle.key();
    let r = &mut ctx.accounts.raffle;
    require!(r.phase == RafflePhase::Committed, RookError::RafflePhase);
    r.ticket = raffle_ticket(&ctx.accounts.drand_round.randomness, &key, r.round, r.total_weight);
    r.phase = RafflePhase::Drawn;
    r.drawn_at = now()?;
    emit!(RaffleDrawn { raffle: key, round: r.round, drand_round: r.drand_round, ticket: r.ticket });
    Ok(())
}

#[derive(Accounts)]
pub struct ClaimRaffle<'info> {
    /// Pays the winner's associated token account when it does not exist.
    #[account(mut)]
    pub claimer: Signer<'info>,
    /// CHECK: the winner's wallet: bound by the Merkle leaf.
    pub owner: UncheckedAccount<'info>,
    #[account(mut)]
    pub raffle: Box<Account<'info, Raffle>>,
    #[account(mut, address = raffle.wx_account)]
    pub raffle_wx: Box<Account<'info, TokenAccount>>,
    #[account(address = raffle.wx_mint)]
    pub wx_mint: Box<Account<'info, Mint>>,
    #[account(
        init_if_needed, payer = claimer,
        associated_token::mint = wx_mint, associated_token::authority = owner, associated_token::token_program = token_program,
    )]
    pub owner_wx: Box<Account<'info, TokenAccount>>,
    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}

/// `claim_raffle(cum_start, weight, proof)` (anyone): the leaf `(owner, cum_start, weight)` is in the committed root and its
/// ticket range `[cum_start, cum_start + weight)` holds the winning ticket: the one whole wX goes to the owner's associated
/// token account. Within `RAFFLE_CLAIM_WINDOW` of the draw.
pub fn handle_claim_raffle(ctx: Context<ClaimRaffle>, cum_start: u64, weight: u64, proof: Vec<[u8; 32]>) -> Result<()> {
    let r = &ctx.accounts.raffle;
    require!(r.phase == RafflePhase::Drawn, RookError::RafflePhase);
    let t = now()?;
    require!(t < r.drawn_at + RAFFLE_CLAIM_WINDOW, RookError::ClaimWindowOver);
    require!(proof.len() <= MAX_RAFFLE_PROOF, RookError::BadInput);
    let end = cum_start.checked_add(weight).ok_or(RookError::NotTheWinner)?;
    require!(weight != 0 && cum_start <= r.ticket && r.ticket < end, RookError::NotTheWinner);
    let owner = ctx.accounts.owner.key();
    require!(verify_proof(&proof, &r.root, raffle_leaf(&owner, cum_start, weight)), RookError::BadProof);
    let seeds: &[&[u8]] = &[RAFFLE, r.listing.as_ref(), &[r.bump]];
    token::transfer(
        CpiContext::new_with_signer(
            token::ID,
            Transfer {
                from: ctx.accounts.raffle_wx.to_account_info(),
                to: ctx.accounts.owner_wx.to_account_info(),
                authority: ctx.accounts.raffle.to_account_info(),
            },
            &[seeds],
        ),
        ONE,
    )?;
    let r = &mut ctx.accounts.raffle;
    r.phase = RafflePhase::Idle;
    r.winner = owner;
    r.claimed_at = t;
    r.prizes_claimed += 1;
    emit!(RaffleClaimed { raffle: r.key(), round: r.round, winner: owner, ticket: r.ticket, cum_start, weight });
    Ok(())
}

#[derive(Accounts)]
pub struct ExpireRaffle<'info> {
    #[account(mut)]
    pub raffle: Box<Account<'info, Raffle>>,
}

/// `expire_raffle` (anyone): a prize unclaimed `RAFFLE_CLAIM_WINDOW` after its draw stays in the raffle as the next round's
/// prize; a committed round not drawn within `RAFFLE_DRAW_TIMEOUT` (its drand round never proven) is cancelled.
pub fn handle_expire_raffle(ctx: Context<ExpireRaffle>) -> Result<()> {
    let r = &mut ctx.accounts.raffle;
    let t = now()?;
    let drawn = match r.phase {
        RafflePhase::Drawn if t >= r.drawn_at + RAFFLE_CLAIM_WINDOW => {
            r.rounds_expired += 1;
            true
        }
        RafflePhase::Committed if t >= r.committed_at + RAFFLE_DRAW_TIMEOUT => {
            r.rounds_cancelled += 1;
            false
        }
        _ => return err!(RookError::RaffleNotExpired),
    };
    r.phase = RafflePhase::Idle;
    emit!(RaffleExpired { raffle: r.key(), round: r.round, drawn });
    Ok(())
}
