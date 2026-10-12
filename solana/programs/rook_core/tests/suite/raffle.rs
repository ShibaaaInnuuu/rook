//! The holder raffle (docs/v9/raffle-design.md): the treasury's overflow split, the prize bought at the floor, and the weekly
//! commit → prove → draw → claim / expire cycle against a real sorted-pair Merkle tree.

use super::common::*;
use super::treasury_wake::*;
use anchor_lang::prelude::{Clock, Pubkey};
use anchor_lang::Space;
use anchor_lang::solana_program::system_program;
use anchor_spl::associated_token::get_associated_token_address;
use anchor_spl::token::spl_token;
use rook_core::clock::round_at;
use rook_core::error::RookError;
use rook_core::hashing::keccak;
use rook_core::seeds::*;
use rook_core::state::*;
use rook_core::units::ONE;
use solana_keypair::Keypair;
use solana_signer::Signer;

const DAY: i64 = 86_400;

// ---- builders ---------------------------------------------------------------------------------------------------------

fn raffle_pda(env: &Env) -> Pubkey {
    pda(&[RAFFLE, env.listing.as_ref()])
}

fn raffle_wx(env: &Env) -> Pubkey {
    pda(&[RAFFLE_WX, raffle_pda(env).as_ref()])
}

fn raffle(env: &Env) -> Raffle {
    env.get(&raffle_pda(env))
}

fn init_raffle(env: &mut Env) -> Res {
    let u = env.user.insecure_clone();
    let i = ix(
        rook_core::instruction::InitRaffle {},
        rook_core::accounts::InitRaffle {
            payer: u.pubkey(),
            listing: env.listing,
            module: env.module,
            wrapper: env.wrapper,
            wx_mint: env.wx_mint,
            raffle: raffle_pda(env),
            raffle_wx: raffle_wx(env),
            token_program: spl_token::ID,
            system_program: system_program::ID,
        },
    );
    env.send(&[i], &[&u])
}

fn fund_prize(env: &mut Env) -> Res {
    let u = env.user.insecure_clone();
    let i = ix(
        rook_core::instruction::FundPrize {},
        rook_core::accounts::FundPrize {
            registry: env.registry,
            listing: env.listing,
            module: env.module,
            module_vault: env.vault,
            wrapper: env.wrapper,
            wx_mint: env.wx_mint,
            raffle: raffle_pda(env),
            raffle_wx: raffle_wx(env),
            token_program: spl_token::ID,
        },
    );
    env.send(&[i], &[&u])
}

fn commit_as(env: &mut Env, who: &Keypair, root: [u8; 32], total: u64, snapshot_slot: u64, entries: u32) -> Res {
    let i = ix(
        rook_core::instruction::CommitRaffle { root, total_weight: total, snapshot_slot, entries },
        rook_core::accounts::CommitRaffle {
            keeper: who.pubkey(),
            registry: env.registry,
            raffle: raffle_pda(env),
            raffle_wx: raffle_wx(env),
        },
    );
    let w = who.insecure_clone();
    env.send(&[i], &[&w])
}

fn slot(env: &Env) -> u64 {
    env.svm.get_sysvar::<Clock>().slot
}

/// Moves the clock's slot forward (the harness moves it one slot per `set_time`).
fn bump_slot(env: &mut Env, n: u64) {
    let mut c: Clock = env.svm.get_sysvar();
    c.slot += n;
    env.svm.set_sysvar(&c);
}

/// The keeper commits `tree` with a snapshot one slot old.
fn commit(env: &mut Env, tree: &Tree) -> Res {
    let k = env.keeper.insecure_clone();
    bump_slot(env, 100);
    let s = slot(env) - 1;
    commit_as(env, &k, tree.root(), tree.total, s, tree.leaves.len() as u32)
}

fn draw(env: &mut Env) -> Res {
    let r = raffle(env);
    let u = env.user.insecure_clone();
    let i = ix(
        rook_core::instruction::DrawRaffle {},
        rook_core::accounts::DrawRaffle { raffle: raffle_pda(env), drand_round: pda(&[DRAND, &r.drand_round.to_le_bytes()]) },
    );
    env.send(&[i], &[&u])
}

fn claim_as(env: &mut Env, claimer: &Keypair, owner: &Pubkey, cum_start: u64, weight: u64, proof: Vec<[u8; 32]>) -> Res {
    let i = ix(
        rook_core::instruction::ClaimRaffle { cum_start, weight, proof },
        rook_core::accounts::ClaimRaffle {
            claimer: claimer.pubkey(),
            owner: *owner,
            raffle: raffle_pda(env),
            raffle_wx: raffle_wx(env),
            wx_mint: env.wx_mint,
            owner_wx: get_associated_token_address(owner, &env.wx_mint),
            token_program: spl_token::ID,
            associated_token_program: anchor_spl::associated_token::ID,
            system_program: system_program::ID,
        },
    );
    let c = claimer.insecure_clone();
    env.send(&[i], &[&c])
}

/// The canister (here `user`) claims for leaf `i` of `tree`.
fn claim_leaf(env: &mut Env, tree: &Tree, i: usize) -> Res {
    let (o, cs, w) = tree.leaves[i];
    let u = env.user.insecure_clone();
    claim_as(env, &u, &o, cs, w, tree.proof(i))
}

fn expire(env: &mut Env) -> Res {
    let u = env.user.insecure_clone();
    let i = ix(rook_core::instruction::ExpireRaffle {}, rook_core::accounts::ExpireRaffle { raffle: raffle_pda(env) });
    env.send(&[i], &[&u])
}

/// Lamports straight into the raffle account (`fund_prize` re-syncs the pot to them).
fn top_up(env: &mut Env, lamports: u64) {
    let k = raffle_pda(env);
    env.svm.expire_blockhash();
    env.svm.airdrop(&k, lamports).unwrap();
}

/// The raffle's pot never exceeds what it holds above rent.
#[track_caller]
fn raffle_covers_pot(env: &Env) {
    let k = raffle_pda(env);
    let a = env.svm.get_account(&k).unwrap();
    let rent = env.svm.minimum_balance_for_rent_exemption(a.data.len());
    assert!(a.lamports - rent >= raffle(env).pot, "raffle lamports − rent ≥ pot");
}

#[track_caller]
fn invariants(env: &Env) {
    env.check_books();
    env.check_ledger();
    raffle_covers_pot(env);
    treasury_covers_pot(env);
}

// ---- the snapshot tree (the canister's convention, docs/v9/raffle-design.md §3) ----------------------------------------

struct Tree {
    /// (owner, cum_start, weight), sorted by owner.
    leaves: Vec<(Pubkey, u64, u64)>,
    levels: Vec<Vec<[u8; 32]>>,
    total: u64,
}

fn pair(a: [u8; 32], b: [u8; 32]) -> [u8; 32] {
    if a <= b {
        keccak(&[&a, &b])
    } else {
        keccak(&[&b, &a])
    }
}

impl Tree {
    fn new(holders: &[(Pubkey, u64)]) -> Tree {
        let mut h = holders.to_vec();
        h.sort_by_key(|(o, _)| o.to_bytes());
        let mut leaves = Vec::new();
        let mut cum = 0;
        for (o, w) in h {
            leaves.push((o, cum, w));
            cum += w;
        }
        let mut levels = vec![leaves.iter().map(|(o, c, w)| raffle_leaf(o, *c, *w)).collect::<Vec<_>>()];
        while levels.last().unwrap().len() > 1 {
            let l = levels.last().unwrap();
            let next = l.chunks(2).map(|c| if c.len() == 2 { pair(c[0], c[1]) } else { c[0] }).collect();
            levels.push(next);
        }
        Tree { leaves, levels, total: cum }
    }

    fn root(&self) -> [u8; 32] {
        self.levels.last().unwrap()[0]
    }

    fn proof(&self, mut i: usize) -> Vec<[u8; 32]> {
        let mut p = Vec::new();
        for l in &self.levels[..self.levels.len() - 1] {
            if (i ^ 1) < l.len() {
                p.push(l[i ^ 1]);
            }
            i /= 2;
        }
        p
    }

    /// The leaf holding ticket `t`.
    fn winner(&self, t: u64) -> usize {
        self.leaves.iter().position(|(_, c, w)| *c <= t && t < c + w).unwrap()
    }
}

fn holders(weights: &[u64]) -> (Vec<Keypair>, Tree) {
    let ks: Vec<Keypair> = weights.iter().map(|_| Keypair::new()).collect();
    let t = Tree::new(&ks.iter().zip(weights).map(|(k, w)| (k.pubkey(), *w)).collect::<Vec<_>>());
    (ks, t)
}

/// A fresh floor, a raffle and enough lamports in its pot for one prize.
fn ready() -> Env {
    let mut env = Env::new();
    ok(env.post(FLOOR));
    ok(init_raffle(&mut env));
    top_up(&mut env, 2 * FLOOR);
    bump_slot(&mut env, 1_000);
    env
}

/// The randomness for drand round `round` that makes this raffle's round `r` draw ticket `want` out of `total`.
fn randomness_for(env: &Env, r: u64, total: u64, want: u64) -> [u8; 32] {
    let k = raffle_pda(env);
    for n in 1u64.. {
        let mut x = [0x5au8; 32];
        x[..8].copy_from_slice(&n.to_le_bytes());
        if raffle_ticket(&x, &k, r, total) == want {
            return x;
        }
    }
    unreachable!()
}

/// After a commit: reach the pinned round, prove it with `randomness`, draw.
fn prove_and_draw(env: &mut Env, randomness: [u8; 32]) -> Res {
    let round = raffle(env).drand_round;
    env.reach_round(round);
    ok(env.prove(round, randomness));
    draw(env)
}

// ---- the treasury's overflow split --------------------------------------------------------------------------------------

#[test]
fn overflow_above_the_cap_is_split_between_the_raffle_pot_and_ops() {
    let mut env = one_collection();
    let ops = Keypair::new().pubkey();
    let o = env.owner.insecure_clone();
    ok(admin(&mut env, rook_core::instruction::SetOps { ops }, &o));
    env.svm.airdrop(&ops, SOL).unwrap();
    assert_eq!(treasury(&env).raffle_bps, DEFAULT_RAFFLE_BPS);
    assert_eq!(DEFAULT_RAFFLE_BPS, 5_000);
    let meta = ok(init_raffle(&mut env));
    println!("init_raffle: {} CU", meta.compute_units_consumed);
    let r0 = raffle(&env);
    assert_eq!((r0.listing, r0.module, r0.wx_mint, r0.wx_account, r0.phase), (env.listing, env.module, env.wx_mint, raffle_wx(&env), RafflePhase::Idle));
    expect_fail(init_raffle(&mut env)); // once
    let cap = cap_of(&env);
    fund_round(&mut env, (cap + 101 * SOL) * 10 / 9);
    let pot = treasury(&env).reserves_pot;
    let room = cap - env.module().surplus;
    let over = pot - room;
    let ops0 = env.lamports(&ops);
    let raffle0 = env.lamports(&raffle_pda(&env));
    // another account in the raffle's place is refused: an existing raffle cannot be skipped
    let u = env.user.insecure_clone();
    let bad = ix(
        rook_core::instruction::Distribute { index: 0 },
        rook_core::accounts::Distribute {
            treasury: env.treasury,
            module: env.module,
            module_vault: env.vault,
            ops: Some(ops),
            raffle: Keypair::new().pubkey(),
        },
    );
    expect_err(env.send(&[bad], &[&u]), RookError::WrongCollection);
    let meta = ok(dist_one(&mut env, Some(ops)));
    println!("distribute (partial: surplus + raffle + ops): {} CU", meta.compute_units_consumed);
    let to_raffle = over / 2;
    let t = treasury(&env);
    let r = raffle(&env);
    assert_eq!(env.module().surplus, cap);
    assert_eq!((r.pot, r.total_in), (to_raffle, to_raffle));
    assert_eq!(env.lamports(&raffle_pda(&env)) - raffle0, to_raffle);
    assert_eq!(env.lamports(&ops) - ops0, over - to_raffle);
    assert_eq!((t.reserves_pot, t.total_to_raffles), (0, to_raffle));
    invariants(&env);
    // no ops address: the ops part stays in the reserves pot, the raffle part is paid
    let mut env = one_collection();
    ok(init_raffle(&mut env));
    let cap = cap_of(&env);
    fund_round(&mut env, cap + 100 * SOL + 1);
    let pot = treasury(&env).reserves_pot;
    let over = pot - (cap - env.module().surplus);
    ok(dist_one(&mut env, None));
    let to_raffle = over / 2;
    assert_eq!((raffle(&env).pot, treasury(&env).reserves_pot), (to_raffle, over - to_raffle));
    invariants(&env);
    // under the cap: nothing reaches the raffle
    let mut env = one_collection();
    ok(init_raffle(&mut env));
    fund_round(&mut env, 10 * SOL);
    ok(dist_one(&mut env, None));
    assert_eq!(raffle(&env).pot, 0);
    invariants(&env);
}

#[test]
fn without_a_raffle_the_whole_overflow_goes_to_ops_as_before() {
    let mut env = one_collection();
    let ops = Keypair::new().pubkey();
    let o = env.owner.insecure_clone();
    ok(admin(&mut env, rook_core::instruction::SetOps { ops }, &o));
    env.svm.airdrop(&ops, SOL).unwrap();
    let cap = cap_of(&env);
    fund_round(&mut env, (cap + 100 * SOL) * 10 / 9);
    let pot = treasury(&env).reserves_pot;
    let over = pot - (cap - env.module().surplus);
    let ops0 = env.lamports(&ops);
    let meta = ok(dist_one(&mut env, Some(ops)));
    println!("distribute (partial, no raffle yet): {} CU", meta.compute_units_consumed);
    assert_eq!(env.lamports(&ops) - ops0, over);
    assert_eq!(treasury(&env).total_to_raffles, 0);
    assert!(!env.exists(&raffle_pda(&env)));
    treasury_covers_pot(&env);
    env.check_books();
}

#[test]
fn the_raffle_share_changes_after_a_day_within_bounds() {
    let mut env = one_collection();
    let o = env.owner.insecure_clone();
    let u = env.user.insecure_clone();
    expect_err(apply(&mut env, rook_core::instruction::ApplyRaffleBps {}), RookError::NothingPending);
    expect_err(admin(&mut env, rook_core::instruction::SetRaffleBps { bps: 10_000 }, &u), RookError::NotOwner);
    expect_err(admin(&mut env, rook_core::instruction::SetRaffleBps { bps: 10_001 }, &o), RookError::BadRaffleBps);
    ok(admin(&mut env, rook_core::instruction::SetRaffleBps { bps: 0 }, &o));
    let t0 = env.now;
    ok(admin(&mut env, rook_core::instruction::SetRaffleBps { bps: 10_000 }, &o)); // replaces, restarts the wait
    let t = treasury(&env);
    assert_eq!((t.raffle_bps, t.pending_raffle_bps, t.pending_raffle_at), (DEFAULT_RAFFLE_BPS, 10_000, t0 + TREASURY_DELAY));
    env.set_time(t0 + TREASURY_DELAY - 1);
    expect_err(apply(&mut env, rook_core::instruction::ApplyRaffleBps {}), RookError::TooEarly);
    env.set_time(t0 + TREASURY_DELAY);
    ok(apply(&mut env, rook_core::instruction::ApplyRaffleBps {}));
    let t = treasury(&env);
    assert_eq!((t.raffle_bps, t.pending_raffle_bps, t.pending_raffle_at), (10_000, 0, 0));
    // at 10_000 the whole overflow reaches the raffle, nothing goes to ops
    let ops = Keypair::new().pubkey();
    ok(admin(&mut env, rook_core::instruction::SetOps { ops }, &o));
    ok(env.post(FLOOR)); // fresh again (the mints' volume is still within seven days)
    ok(init_raffle(&mut env));
    let cap = cap_of(&env);
    fund_round(&mut env, (cap + 100 * SOL) * 10 / 9);
    let pot = treasury(&env).reserves_pot;
    let over = pot - (cap - env.module().surplus);
    ok(dist_one(&mut env, None)); // no ops account needed: nothing is paid to it
    assert_eq!(raffle(&env).pot, over);
    invariants(&env);
}

// ---- the prize ----------------------------------------------------------------------------------------------------------

#[test]
fn fund_prize_buys_one_whole_wx_at_the_mint_price() {
    let mut env = ready();
    let m0 = env.module();
    let w0 = env.wrapper();
    let price = m0.mint_price(env.now, w0.eth_backed).unwrap();
    let vault0 = env.vault_free();
    let pot0 = env.lamports(&raffle_pda(&env)) - env.svm.minimum_balance_for_rent_exemption(8 + Raffle::INIT_SPACE);
    let meta = ok(fund_prize(&mut env));
    println!("fund_prize: {} CU", meta.compute_units_consumed);
    let r = raffle(&env);
    assert_eq!(env.balance(&raffle_wx(&env)), ONE);
    assert_eq!(env.vault_free() - vault0, price, "the pot pays the mint price into the module vault");
    assert_eq!((r.pot, r.total_spent, r.prizes_funded), (pot0 - price, price, 1));
    assert_eq!(env.wrapper().eth_backed, w0.eth_backed + ONE, "an ETH-backed unit, as any public mint");
    let m = env.module();
    assert!(m.backing + m.reserve > m0.backing + m0.reserve, "the intake joins the books");
    assert!(price > FLOOR, "at or above the floor plus the mint spread");
    invariants(&env);
    // a prize is held: no second one
    expect_err(fund_prize(&mut env), RookError::PrizeHeld);
}

#[test]
fn fund_prize_refusals() {
    // the pot cannot cover one whole wX
    let mut env = Env::new();
    ok(env.post(FLOOR));
    ok(init_raffle(&mut env));
    top_up(&mut env, FLOOR / 2);
    expect_err(fund_prize(&mut env), RookError::PotTooSmall);
    // a stale floor
    top_up(&mut env, 2 * FLOOR);
    env.advance_time(3 * 3_600);
    expect_err(fund_prize(&mut env), RookError::Stale);
    // the poster paused
    ok(env.post(FLOOR));
    let g = env.guardian.insecure_clone();
    ok(env.listing_admin(rook_core::instruction::PausePoster {}, &g));
    expect_err(fund_prize(&mut env), RookError::Paused);
    let o = env.owner.insecure_clone();
    ok(env.listing_admin(rook_core::instruction::UnpausePoster {}, &o));
    ok(env.post(FLOOR));
    ok(fund_prize(&mut env));
    // the hourly mint bucket applies: a full bucket refuses the prize too
    let mut env = ready();
    let u = env.user.insecure_clone();
    let acc = env.token_account(&u.pubkey());
    let p = default_params();
    let price = env.module().mint_price(env.now, 0).unwrap();
    ok(env.mint_units(&u, &acc, price * (p.mint_per_hour / ONE) - price / 2, 0));
    expect_err(fund_prize(&mut env), RookError::OverCap);
    invariants(&env);
}

// ---- the draw cycle -----------------------------------------------------------------------------------------------------

#[test]
fn full_cycle_commit_prove_draw_claim_with_a_real_tree() {
    let mut env = ready();
    top_up(&mut env, 2 * FLOOR);
    ok(fund_prize(&mut env));
    let (_ks, tree) = holders(&[30 * ONE, 5 * ONE, 12 * ONE + 7, ONE, 200_000_000]);
    // only the keeper commits
    let u = env.user.insecure_clone();
    let s = slot(&env) - 1;
    expect_err(commit_as(&mut env, &u, tree.root(), tree.total, s, 5), RookError::NotKeeper);
    let t_commit = env.now;
    let meta = ok(commit(&mut env, &tree));
    println!("commit_raffle: {} CU", meta.compute_units_consumed);
    let r = raffle(&env);
    assert_eq!((r.phase, r.round, r.root, r.total_weight, r.entries), (RafflePhase::Committed, 1, tree.root(), tree.total, 5));
    assert_eq!((r.committed_at, r.drand_round), (t_commit, round_at(t_commit) + RAFFLE_PIN_AHEAD));
    assert!(round_at(env.now) < r.drand_round, "the pinned round does not exist at commit");
    // the round cannot be proven yet, nor drawn without it
    expect_err(env.prove(r.drand_round, [1; 32]), RookError::RoundInFuture);
    expect_fail(draw(&mut env));
    // nothing to claim before the draw
    expect_err(claim_leaf(&mut env, &tree, 0), RookError::RafflePhase);
    let rnd = [0x42u8; 32];
    let meta = ok(prove_and_draw(&mut env, rnd));
    println!("draw_raffle: {} CU", meta.compute_units_consumed);
    let r = raffle(&env);
    let ticket = raffle_ticket(&rnd, &raffle_pda(&env), 1, tree.total);
    assert_eq!((r.phase, r.ticket), (RafflePhase::Drawn, ticket));
    expect_err(draw(&mut env), RookError::RafflePhase);
    let w = tree.winner(ticket);
    // another holder's genuine leaf does not hold the ticket
    let other = (w + 1) % tree.leaves.len();
    expect_err(claim_leaf(&mut env, &tree, other), RookError::NotTheWinner);
    // the winner's range with someone else's wallet, a stretched weight, or a wrong proof
    let (wo, cs, wt) = tree.leaves[w];
    let thief = Keypair::new().pubkey();
    expect_err(claim_as(&mut env, &u, &thief, cs, wt, tree.proof(w)), RookError::BadProof);
    expect_err(claim_as(&mut env, &u, &tree.leaves[other].0, cs, wt, tree.proof(w)), RookError::BadProof);
    expect_err(claim_as(&mut env, &u, &wo, 0, tree.total, tree.proof(w)), RookError::BadProof);
    let mut bad = tree.proof(w);
    bad[0][0] ^= 1;
    expect_err(claim_as(&mut env, &u, &wo, cs, wt, bad), RookError::BadProof);
    // the winner's leaf: the prize lands in the winner's associated token account (created, paid by the claimer)
    let meta = ok(claim_leaf(&mut env, &tree, w));
    println!("claim_raffle ({} proof nodes, ATA created): {} CU", tree.proof(w).len(), meta.compute_units_consumed);
    assert_eq!(env.balance(&get_associated_token_address(&wo, &env.wx_mint)), ONE);
    assert_eq!(env.balance(&raffle_wx(&env)), 0);
    let r = raffle(&env);
    assert_eq!((r.phase, r.winner, r.prizes_claimed), (RafflePhase::Idle, wo, 1));
    expect_err(claim_leaf(&mut env, &tree, w), RookError::RafflePhase);
    invariants(&env);
    // the next round: a new prize, but no commit for seven days
    ok(env.post(FLOOR));
    ok(fund_prize(&mut env));
    expect_err(commit(&mut env, &tree), RookError::RaffleTooEarly);
    env.set_time(t_commit + RAFFLE_PERIOD - 1);
    expect_err(commit(&mut env, &tree), RookError::RaffleTooEarly);
    env.set_time(t_commit + RAFFLE_PERIOD);
    ok(commit(&mut env, &tree));
    assert_eq!(raffle(&env).round, 2);
    invariants(&env);
}

/// Rounds drawn with chosen randomness so the ticket hits each boundary: the very first and very last ticket, the first and
/// last ticket of a middle leaf (its neighbours refused).
#[test]
fn the_winner_at_every_boundary_and_a_single_holder() {
    let mut env = ready();
    top_up(&mut env, 10 * FLOOR);
    let (_ks, tree) = holders(&[3, 2, 4]); // ranges [0,3) [3,5) [5,9) by owner order
    let total = tree.total;
    assert_eq!(total, 9);
    let (_, mcs, mw) = tree.leaves[1];
    let cases = [0, total - 1, mcs, mcs + mw - 1];
    for (k, want) in cases.into_iter().enumerate() {
        ok(env.post(FLOOR));
        ok(fund_prize(&mut env));
        ok(commit(&mut env, &tree));
        let r = raffle(&env).round;
        let rnd = randomness_for(&env, r, total, want);
        ok(prove_and_draw(&mut env, rnd));
        assert_eq!(raffle(&env).ticket, want);
        let w = tree.winner(want);
        for j in 0..tree.leaves.len() {
            if j != w {
                expect_err(claim_leaf(&mut env, &tree, j), RookError::NotTheWinner);
            }
        }
        ok(claim_leaf(&mut env, &tree, w));
        assert_eq!(raffle(&env).prizes_claimed, k as u64 + 1);
        invariants(&env);
        let at = raffle(&env).committed_at + RAFFLE_PERIOD;
        env.set_time(at);
    }
    // the same winner twice: its account already exists (init_if_needed)
    let w = tree.winner(0);
    assert_eq!(env.balance(&get_associated_token_address(&tree.leaves[w].0, &env.wx_mint)), ONE);
    // a single holder: the root is its leaf, the proof empty, and it always wins
    let (_k1, one) = holders(&[ONE]);
    assert!(one.proof(0).is_empty());
    ok(env.post(FLOOR));
    ok(fund_prize(&mut env));
    ok(commit(&mut env, &one));
    ok(prove_and_draw(&mut env, [0x77; 32]));
    let meta = ok(claim_leaf(&mut env, &one, 0));
    println!("claim_raffle (single holder, empty proof): {} CU", meta.compute_units_consumed);
    assert_eq!(env.balance(&get_associated_token_address(&one.leaves[0].0, &env.wx_mint)), ONE);
    invariants(&env);
}

#[test]
fn commit_rules() {
    let mut env = ready();
    top_up(&mut env, 2 * FLOOR);
    let (_ks, tree) = holders(&[5, 6]);
    let k = env.keeper.insecure_clone();
    // no prize held yet
    expect_err(commit(&mut env, &tree), RookError::NoPrize);
    ok(fund_prize(&mut env));
    let s = slot(&env);
    expect_err(commit_as(&mut env, &k, [0; 32], tree.total, s - 1, 2), RookError::BadInput);
    expect_err(commit_as(&mut env, &k, tree.root(), 0, s - 1, 2), RookError::BadInput);
    expect_err(commit_as(&mut env, &k, tree.root(), tree.total, s - 1, 0), RookError::BadInput);
    expect_err(commit_as(&mut env, &k, tree.root(), tree.total, s, 2), RookError::BadSnapshot); // not in the past
    // too old: push the clock's slot forward
    bump_slot(&mut env, RAFFLE_SNAPSHOT_MAX_AGE + 10);
    let s = slot(&env);
    expect_err(commit_as(&mut env, &k, tree.root(), tree.total, s - RAFFLE_SNAPSHOT_MAX_AGE - 1, 2), RookError::BadSnapshot);
    ok(commit_as(&mut env, &k, tree.root(), tree.total, s - RAFFLE_SNAPSHOT_MAX_AGE, 2));
    // committed: no second commit, no new prize
    expect_err(commit_as(&mut env, &k, tree.root(), tree.total, s - 1, 2), RookError::RafflePhase);
    expect_err(fund_prize(&mut env), RookError::PrizeHeld);
    // a paused keeper commits nothing
    let snap = raffle(&env).snapshot_slot;
    ok(prove_and_draw(&mut env, [3; 32]));
    let w = tree.winner(raffle(&env).ticket);
    ok(claim_leaf(&mut env, &tree, w));
    env.set_time(raffle(&env).committed_at + RAFFLE_PERIOD);
    ok(env.post(FLOOR));
    ok(fund_prize(&mut env));
    // the snapshot must be newer than the previous one
    expect_err(commit_as(&mut env, &k, tree.root(), tree.total, snap, 2), RookError::BadSnapshot);
    let g = env.guardian.insecure_clone();
    ok(env.registry_admin(rook_core::instruction::PauseKeeper {}, Some(&g)));
    expect_err(commit(&mut env, &tree), RookError::NotKeeper);
}

#[test]
fn an_unclaimed_prize_expires_after_30_days_and_an_undrawn_round_after_7() {
    let mut env = ready();
    ok(fund_prize(&mut env));
    let (_ks, tree) = holders(&[1, 1, 1]);
    ok(commit(&mut env, &tree));
    // committed, never drawn: cancelled after RAFFLE_DRAW_TIMEOUT
    expect_err(expire(&mut env), RookError::RaffleNotExpired);
    let c = raffle(&env).committed_at;
    env.set_time(c + RAFFLE_DRAW_TIMEOUT - 1);
    expect_err(expire(&mut env), RookError::RaffleNotExpired);
    env.set_time(c + RAFFLE_DRAW_TIMEOUT);
    ok(expire(&mut env));
    let r = raffle(&env);
    assert_eq!((r.phase, r.rounds_cancelled, r.rounds_expired), (RafflePhase::Idle, 1, 0));
    assert_eq!(env.balance(&raffle_wx(&env)), ONE, "the prize stays");
    // a new round at once (seven days have passed), drawn and left unclaimed
    ok(commit(&mut env, &tree));
    ok(prove_and_draw(&mut env, [9; 32]));
    let d = raffle(&env).drawn_at;
    expect_err(expire(&mut env), RookError::RaffleNotExpired);
    env.set_time(d + 30 * DAY - 1);
    expect_err(expire(&mut env), RookError::RaffleNotExpired);
    env.set_time(d + RAFFLE_CLAIM_WINDOW);
    let w = tree.winner(raffle(&env).ticket);
    expect_err(claim_leaf(&mut env, &tree, w), RookError::ClaimWindowOver);
    let meta = ok(expire(&mut env));
    println!("expire_raffle: {} CU", meta.compute_units_consumed);
    let r = raffle(&env);
    assert_eq!((r.phase, r.rounds_expired, r.prizes_claimed), (RafflePhase::Idle, 1, 0));
    // the prize is the next round's: no new purchase, and the next round may be committed and claimed
    assert_eq!(env.balance(&raffle_wx(&env)), ONE);
    ok(env.post(FLOOR));
    expect_err(fund_prize(&mut env), RookError::PrizeHeld);
    ok(commit(&mut env, &tree));
    ok(prove_and_draw(&mut env, [10; 32]));
    let w = tree.winner(raffle(&env).ticket);
    ok(claim_leaf(&mut env, &tree, w));
    assert_eq!(raffle(&env).prizes_funded, 1, "one purchase served two rounds");
    invariants(&env);
}

/// Pot → prize → winner, end to end from the treasury: income above the cap funds the pot, the pot buys the prize, and
/// every invariant holds at each step (the treasury's, the module's books, the wX ledger, the raffle's pot).
#[test]
fn from_treasury_overflow_to_a_winner() {
    let mut env = one_collection();
    let o = env.owner.insecure_clone();
    let ops = Keypair::new().pubkey();
    ok(admin(&mut env, rook_core::instruction::SetOps { ops }, &o));
    env.svm.airdrop(&ops, SOL).unwrap();
    ok(init_raffle(&mut env));
    let cap = cap_of(&env);
    fund_round(&mut env, (cap + 3 * FLOOR) * 10 / 9);
    invariants(&env);
    ok(dist_one(&mut env, Some(ops)));
    invariants(&env);
    assert!(raffle(&env).pot > FLOOR);
    ok(env.post(FLOOR));
    let supply0 = env.supply();
    ok(fund_prize(&mut env));
    assert_eq!(env.supply(), supply0 + ONE);
    invariants(&env);
    let (_ks, tree) = holders(&[ONE, 2 * ONE, 3 * ONE, 4 * ONE]);
    ok(commit(&mut env, &tree));
    ok(prove_and_draw(&mut env, [0xab; 32]));
    let w = tree.winner(raffle(&env).ticket);
    ok(claim_leaf(&mut env, &tree, w));
    invariants(&env);
    assert_eq!(env.supply(), supply0 + ONE, "a transfer: the supply is unchanged by the claim");
}

/// The cross-check vector the canister's snapshot builder is held to (autobuyer/canister-rs `tests/cranks_sol.rs`,
/// `the_snapshot_tree_matches_rook_cores_vector`): five fixed owners, the canister's tree convention, committed and claimed on chain
/// with every leaf's proof accepted by the program's own `verify_proof`.
pub const VECTOR_HOLDERS: [([u8; 32], u64); 5] = [([4; 32], 10), ([1; 32], 5), ([3; 32], 1), ([5; 32], 3), ([2; 32], 7)];
pub const VECTOR_ROOT: &str = "de15ed1483f99479cda54f46d94501fd58c66abbec3065de58c8fec113de9692";

#[test]
fn the_canister_tree_vector_commits_and_every_leaf_claims() {
    let tree = Tree::new(&VECTOR_HOLDERS.iter().map(|(o, w)| (Pubkey::new_from_array(*o), *w)).collect::<Vec<_>>());
    let hx: String = tree.root().iter().map(|x| format!("{x:02x}")).collect();
    println!("VECTOR_ROOT = {hx}");
    assert_eq!(hx, VECTOR_ROOT, "the canister's cross-check vector");
    assert_eq!(tree.leaves.iter().map(|l| (l.1, l.2)).collect::<Vec<_>>(), vec![(0, 5), (5, 7), (12, 1), (13, 10), (23, 3)]);
    for (i, (o, c, w)) in tree.leaves.iter().enumerate() {
        assert!(rook_core::hashing::verify_proof(&tree.proof(i), &tree.root(), raffle_leaf(o, *c, *w)), "leaf {i}");
    }
    let mut env = ready();
    top_up(&mut env, 2 * FLOOR);
    ok(fund_prize(&mut env));
    ok(commit(&mut env, &tree));
    // the ticket falls in leaf 3 (owner [4; 32], range 13..23): the program accepts its proof
    let rnd = randomness_for(&env, 1, tree.total, 17);
    ok(prove_and_draw(&mut env, rnd));
    assert_eq!(raffle(&env).ticket, 17);
    ok(claim_leaf(&mut env, &tree, 3));
    assert_eq!(env.balance(&get_associated_token_address(&Pubkey::new_from_array([4; 32]), &env.wx_mint)), ONE);
}
