//! The holder raffle in the canister's books: the treasury's overflow split, the prize bought at the floor, and the weekly
//! commit → prove → draw → claim / expire cycle against a real sorted-pair Merkle tree. Ported from
//! solana/programs/rook_core/tests/suite/raffle.rs (names kept). The keeper role is gone (committing is canister work); the
//! draw takes the canister's verified drand randomness; the claim moves ONE wX from the prize account at `K_tre`.

mod books_common;

use books_common::*;
use etch_autobuyer::books::math::{keccak, round_at};
use etch_autobuyer::books::raffle::*;
use etch_autobuyer::books::treasury::*;
use etch_autobuyer::books::*;

const DAY: i64 = 86_400;
const OPS: [u8; 32] = [0x99; 32];
const RAFFLE_KEY: [u8; 32] = [0x5e; 32];

fn one_collection() -> Env {
    let mut env = Env::new();
    ok(env.post(FLOOR));
    ok(env.mint_units(USER, 100 * SOL, 0));
    env
}

fn init_raffle(env: &mut Env) -> Res<()> {
    env.b.init_raffle(ID, RAFFLE_KEY)
}

#[track_caller]
fn invariants(env: &Env) {
    env.check_books();
    env.check_ledger();
    assert_eq!(env.balance(&PRIZE), env.raffle().prize, "the prize account holds the book's prize");
}

fn bump_slot(env: &mut Env, n: u64) {
    env.slot += n;
}

/// Commits `tree` with a snapshot one slot old.
fn commit(env: &mut Env, tree: &Tree) -> Res<u64> {
    bump_slot(env, 100);
    let s = env.slot - 1;
    env.commit(tree.root(), tree.total, s, tree.leaves.len() as u32)
}

fn draw(env: &mut Env) -> Res<u64> {
    let r = env.raffle();
    let rnd = env.drand.get(&r.drand_round).copied().ok_or(BooksError::RoundNotProven)?;
    env.b.draw_raffle(ID, r.drand_round, &rnd, env.now)
}

fn claim_leaf(env: &mut Env, tree: &Tree, i: usize) -> Res<u64> {
    let (o, cs, w) = tree.leaves[i];
    env.claim(o, cs, w, &tree.proof(i))
}

fn expire(env: &mut Env) -> Res<bool> {
    env.b.expire_raffle(ID, env.now)
}

// ---- the snapshot tree (the canister's convention, crate::raffle) ------------------------------------------------------------

struct Tree {
    leaves: Vec<([u8; 32], u64, u64)>,
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
    fn new(holders: &[([u8; 32], u64)]) -> Tree {
        let mut h = holders.to_vec();
        h.sort_by_key(|(o, _)| *o);
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

    fn winner(&self, t: u64) -> usize {
        self.leaves.iter().position(|(_, c, w)| *c <= t && t < c + w).unwrap()
    }
}

fn holders(weights: &[u64]) -> Tree {
    holders_from(1_000, weights)
}

/// Holders `wallet(base + i)` (fresh wallets per tree, as rook_core's `Keypair::new()`).
fn holders_from(base: u64, weights: &[u64]) -> Tree {
    Tree::new(&weights.iter().enumerate().map(|(i, w)| (wallet(base + i as u64), *w)).collect::<Vec<_>>())
}

/// A fresh floor, a raffle and enough lamports in its pot for one prize.
fn ready() -> Env {
    let mut env = Env::new();
    ok(env.post(FLOOR));
    ok(init_raffle(&mut env));
    env.top_up(2 * FLOOR);
    bump_slot(&mut env, 1_000);
    env
}

/// The randomness that makes round `r` draw ticket `want` out of `total`.
fn randomness_for(r: u64, total: u64, want: u64) -> [u8; 32] {
    for n in 1u64.. {
        let mut x = [0x5au8; 32];
        x[..8].copy_from_slice(&n.to_le_bytes());
        if raffle_ticket(&x, &RAFFLE_KEY, r, total) == want {
            return x;
        }
    }
    unreachable!()
}

/// After a commit: reach the pinned round, prove it with `randomness`, draw.
fn prove_and_draw(env: &mut Env, randomness: [u8; 32]) -> Res<u64> {
    let round = env.raffle().drand_round;
    env.reach_round(round);
    ok(env.prove(round, randomness));
    draw(env)
}

// ---- the treasury's overflow split ---------------------------------------------------------------------------------------

// rook_core raffle.rs: overflow_above_the_cap_is_split_between_the_raffle_pot_and_ops
#[test]
fn overflow_above_the_cap_is_split_between_the_raffle_pot_and_ops() {
    let mut env = one_collection();
    ok(env.b.treasury.set_ops(OPS, env.now));
    assert_eq!(env.b.treasury.raffle_bps, DEFAULT_RAFFLE_BPS);
    assert_eq!(DEFAULT_RAFFLE_BPS, 5_000);
    ok(init_raffle(&mut env));
    let r0 = env.raffle();
    assert_eq!((r0.key, r0.phase, r0.pot, r0.prize), (RAFFLE_KEY, RafflePhase::Idle, 0, 0));
    assert!(init_raffle(&mut env).is_err(), "once");
    let cap = env.cap_of();
    env.fund_round((cap + 101 * SOL) * 10 / 9);
    let pot = env.b.treasury.reserves_pot;
    let room = cap - env.module().surplus;
    let over = pot - room;
    let ops0 = env.lamports(&OPS);
    let tre0 = env.tre;
    ok(env.distribute(0));
    let to_raffle = over / 2;
    let t = env.b.treasury.clone();
    let r = env.raffle();
    assert_eq!(env.module().surplus, cap);
    assert_eq!((r.pot, r.total_in), (to_raffle, to_raffle));
    assert_eq!(tre0 - env.tre, room + (over - to_raffle), "the raffle's part stays at K_tre");
    assert_eq!(env.lamports(&OPS) - ops0, (over - to_raffle) as i128);
    assert_eq!((t.reserves_pot, t.total_to_raffles), (0, to_raffle));
    invariants(&env);
    // no ops address: the ops part stays in the reserves pot, the raffle part is paid
    let mut env = one_collection();
    ok(init_raffle(&mut env));
    let cap = env.cap_of();
    env.fund_round(cap + 100 * SOL + 1);
    let pot = env.b.treasury.reserves_pot;
    let over = pot - (cap - env.module().surplus);
    ok(env.distribute(0));
    let to_raffle = over / 2;
    assert_eq!((env.raffle().pot, env.b.treasury.reserves_pot), (to_raffle, over - to_raffle));
    invariants(&env);
    // under the cap: nothing reaches the raffle
    let mut env = one_collection();
    ok(init_raffle(&mut env));
    env.fund_round(10 * SOL);
    ok(env.distribute(0));
    assert_eq!(env.raffle().pot, 0);
    invariants(&env);
}

// rook_core raffle.rs: without_a_raffle_the_whole_overflow_goes_to_ops_as_before
#[test]
fn without_a_raffle_the_whole_overflow_goes_to_ops_as_before() {
    let mut env = one_collection();
    ok(env.b.treasury.set_ops(OPS, env.now));
    let cap = env.cap_of();
    env.fund_round((cap + 100 * SOL) * 10 / 9);
    let pot = env.b.treasury.reserves_pot;
    let over = pot - (cap - env.module().surplus);
    let ops0 = env.lamports(&OPS);
    ok(env.distribute(0));
    assert_eq!(env.lamports(&OPS) - ops0, over as i128);
    assert_eq!(env.b.treasury.total_to_raffles, 0);
    assert!(env.b.get(ID).unwrap().raffle.is_none());
    env.check_books();
}

// rook_core raffle.rs: the_raffle_share_changes_after_a_day_within_bounds
#[test]
fn the_raffle_share_changes_after_a_day_within_bounds() {
    let mut env = one_collection();
    expect_err(env.b.treasury.apply_raffle_bps(env.now), BooksError::NothingPending);
    expect_err(env.b.treasury.set_raffle_bps(10_001, env.now), BooksError::BadRaffleBps);
    ok(env.b.treasury.set_raffle_bps(0, env.now));
    let t0 = env.now;
    ok(env.b.treasury.set_raffle_bps(10_000, t0));
    let t = &env.b.treasury;
    assert_eq!((t.raffle_bps, t.pending_raffle_bps, t.pending_raffle_at), (DEFAULT_RAFFLE_BPS, 10_000, t0 + TREASURY_DELAY));
    env.set_time(t0 + TREASURY_DELAY - 1);
    expect_err(env.b.treasury.apply_raffle_bps(env.now), BooksError::TooEarly);
    env.set_time(t0 + TREASURY_DELAY);
    ok(env.b.treasury.apply_raffle_bps(env.now));
    let t = &env.b.treasury;
    assert_eq!((t.raffle_bps, t.pending_raffle_bps, t.pending_raffle_at), (10_000, 0, 0));
    ok(env.b.treasury.set_ops(OPS, env.now));
    ok(env.post(FLOOR));
    ok(init_raffle(&mut env));
    let cap = env.cap_of();
    env.fund_round((cap + 100 * SOL) * 10 / 9);
    let pot = env.b.treasury.reserves_pot;
    let over = pot - (cap - env.module().surplus);
    ok(env.distribute(0));
    assert_eq!(env.raffle().pot, over);
    invariants(&env);
}

// ---- the prize -----------------------------------------------------------------------------------------------------------

// rook_core raffle.rs: fund_prize_buys_one_whole_wx_at_the_mint_price
#[test]
fn fund_prize_buys_one_whole_wx_at_the_mint_price() {
    let mut env = ready();
    let m0 = env.module();
    let e0 = env.e();
    let price = m0.mint_price(env.now, e0).unwrap();
    let vault0 = env.vault_free();
    let pot0 = env.raffle().pot;
    ok(env.fund_prize());
    let r = env.raffle();
    assert_eq!(env.balance(&PRIZE), ONE);
    assert_eq!(env.vault_free() - vault0, price, "the pot pays the mint price into the module address");
    assert_eq!((r.pot, r.total_spent, r.prizes_funded), (pot0 - price, price, 1));
    assert_eq!(env.e(), e0 + ONE, "a SOL-backed unit, as any public mint");
    let m = env.module();
    assert!(m.backing + m.reserve > m0.backing + m0.reserve, "the intake joins the books");
    assert!(price > FLOOR, "at or above the floor plus the mint spread");
    invariants(&env);
    expect_err(env.fund_prize(), BooksError::PrizeHeld);
}

// rook_core raffle.rs: fund_prize_refusals
#[test]
fn fund_prize_refusals() {
    let mut env = Env::new();
    ok(env.post(FLOOR));
    expect_err(env.fund_prize(), BooksError::NoRaffle);
    ok(init_raffle(&mut env));
    env.top_up(FLOOR / 2);
    expect_err(env.fund_prize(), BooksError::PotTooSmall);
    env.top_up(2 * FLOOR);
    env.advance_time(3 * 3_600);
    expect_err(env.fund_prize(), BooksError::Stale);
    ok(env.post(FLOOR));
    env.pause_poster();
    expect_err(env.fund_prize(), BooksError::Paused);
    env.unpause_poster();
    ok(env.post(FLOOR));
    ok(env.fund_prize());
    // the hourly mint bucket applies: a full bucket refuses the prize too
    let mut env = ready();
    let p = test_params();
    let price = env.module().mint_price(env.now, 0).unwrap();
    ok(env.mint_units(USER, price * (p.mint_per_hour / ONE) - price / 2, 0));
    let before = env.b.clone();
    expect_err(env.fund_prize(), BooksError::OverCap);
    assert_eq!(env.b, before, "a refused prize changes nothing");
    invariants(&env);
}

// ---- the draw cycle --------------------------------------------------------------------------------------------------------

// rook_core raffle.rs: full_cycle_commit_prove_draw_claim_with_a_real_tree
#[test]
fn full_cycle_commit_prove_draw_claim_with_a_real_tree() {
    let mut env = ready();
    env.top_up(2 * FLOOR);
    ok(env.fund_prize());
    let tree = holders(&[30 * ONE, 5 * ONE, 12 * ONE + 7, ONE, 200_000_000]);
    let t_commit = env.now;
    ok(commit(&mut env, &tree));
    let r = env.raffle();
    assert_eq!((r.phase, r.round, r.root, r.total_weight, r.entries), (RafflePhase::Committed, 1, tree.root(), tree.total, 5));
    assert_eq!((r.committed_at, r.drand_round), (t_commit, round_at(t_commit) + RAFFLE_PIN_AHEAD));
    assert!(round_at(env.now) < r.drand_round, "the pinned round does not exist at commit");
    expect_err(env.prove(r.drand_round, [1; 32]), BooksError::RoundInFuture);
    expect_err(env.b.draw_raffle(ID, r.drand_round, &[1; 32], env.now), BooksError::RoundInFuture);
    expect_err(draw(&mut env), BooksError::RoundNotProven);
    expect_err(claim_leaf(&mut env, &tree, 0), BooksError::RafflePhase);
    let rnd = [0x42u8; 32];
    ok(prove_and_draw(&mut env, rnd));
    let r = env.raffle();
    let ticket = raffle_ticket(&rnd, &RAFFLE_KEY, 1, tree.total);
    assert_eq!((r.phase, r.ticket), (RafflePhase::Drawn, ticket));
    expect_err(draw(&mut env), BooksError::RafflePhase);
    let w = tree.winner(ticket);
    let other = (w + 1) % tree.leaves.len();
    expect_err(claim_leaf(&mut env, &tree, other), BooksError::NotTheWinner);
    let (wo, cs, wt) = tree.leaves[w];
    let thief = wallet(9_999);
    expect_err(env.claim(thief, cs, wt, &tree.proof(w)), BooksError::BadProof);
    expect_err(env.claim(tree.leaves[other].0, cs, wt, &tree.proof(w)), BooksError::BadProof);
    expect_err(env.claim(wo, 0, tree.total, &tree.proof(w)), BooksError::BadProof);
    let mut bad = tree.proof(w);
    bad[0][0] ^= 1;
    expect_err(env.claim(wo, cs, wt, &bad), BooksError::BadProof);
    ok(claim_leaf(&mut env, &tree, w));
    assert_eq!(env.balance(&wo), ONE);
    assert_eq!(env.balance(&PRIZE), 0);
    let r = env.raffle();
    assert_eq!((r.phase, r.winner, r.prizes_claimed), (RafflePhase::Idle, wo, 1));
    expect_err(claim_leaf(&mut env, &tree, w), BooksError::RafflePhase);
    invariants(&env);
    ok(env.post(FLOOR));
    ok(env.fund_prize());
    expect_err(commit(&mut env, &tree), BooksError::RaffleTooEarly);
    env.set_time(t_commit + RAFFLE_PERIOD - 1);
    expect_err(commit(&mut env, &tree), BooksError::RaffleTooEarly);
    env.set_time(t_commit + RAFFLE_PERIOD);
    ok(commit(&mut env, &tree));
    assert_eq!(env.raffle().round, 2);
    invariants(&env);
}

// rook_core raffle.rs: the_winner_at_every_boundary_and_a_single_holder
#[test]
fn the_winner_at_every_boundary_and_a_single_holder() {
    let mut env = ready();
    env.top_up(10 * FLOOR);
    let tree = holders(&[3, 2, 4]);
    let total = tree.total;
    assert_eq!(total, 9);
    let (_, mcs, mw) = tree.leaves[1];
    let cases = [0, total - 1, mcs, mcs + mw - 1];
    for (k, want) in cases.into_iter().enumerate() {
        ok(env.post(FLOOR));
        ok(env.fund_prize());
        ok(commit(&mut env, &tree));
        let r = env.raffle().round;
        ok(prove_and_draw(&mut env, randomness_for(r, total, want)));
        assert_eq!(env.raffle().ticket, want);
        let w = tree.winner(want);
        for j in 0..tree.leaves.len() {
            if j != w {
                expect_err(claim_leaf(&mut env, &tree, j), BooksError::NotTheWinner);
            }
        }
        ok(claim_leaf(&mut env, &tree, w));
        assert_eq!(env.raffle().prizes_claimed, k as u64 + 1);
        invariants(&env);
        let at = env.raffle().committed_at + RAFFLE_PERIOD;
        env.set_time(at);
    }
    let one = holders_from(5_000, &[ONE]);
    assert!(one.proof(0).is_empty());
    ok(env.post(FLOOR));
    ok(env.fund_prize());
    ok(commit(&mut env, &one));
    ok(prove_and_draw(&mut env, [0x77; 32]));
    ok(claim_leaf(&mut env, &one, 0));
    assert_eq!(env.balance(&one.leaves[0].0), ONE);
    invariants(&env);
}

// rook_core raffle.rs: commit_rules (the keeper's pause no longer applies: committing is canister work)
#[test]
fn commit_rules() {
    let mut env = ready();
    env.top_up(2 * FLOOR);
    let tree = holders(&[5, 6]);
    expect_err(commit(&mut env, &tree), BooksError::NoPrize);
    ok(env.fund_prize());
    let s = env.slot;
    expect_err(env.commit([0; 32], tree.total, s - 1, 2), BooksError::BadInput);
    expect_err(env.commit(tree.root(), 0, s - 1, 2), BooksError::BadInput);
    expect_err(env.commit(tree.root(), tree.total, s - 1, 0), BooksError::BadInput);
    expect_err(env.commit(tree.root(), tree.total, s, 2), BooksError::BadSnapshot); // not in the past
    bump_slot(&mut env, RAFFLE_SNAPSHOT_MAX_AGE + 10);
    let s = env.slot;
    expect_err(env.commit(tree.root(), tree.total, s - RAFFLE_SNAPSHOT_MAX_AGE - 1, 2), BooksError::BadSnapshot);
    ok(env.commit(tree.root(), tree.total, s - RAFFLE_SNAPSHOT_MAX_AGE, 2));
    expect_err(env.commit(tree.root(), tree.total, s - 1, 2), BooksError::RafflePhase);
    expect_err(env.fund_prize(), BooksError::PrizeHeld);
    let snap = env.raffle().snapshot_slot;
    ok(prove_and_draw(&mut env, [3; 32]));
    let w = tree.winner(env.raffle().ticket);
    ok(claim_leaf(&mut env, &tree, w));
    let at = env.raffle().committed_at + RAFFLE_PERIOD;
    env.set_time(at);
    ok(env.post(FLOOR));
    ok(env.fund_prize());
    expect_err(env.commit(tree.root(), tree.total, snap, 2), BooksError::BadSnapshot);
}

// rook_core raffle.rs: an_unclaimed_prize_expires_after_30_days_and_an_undrawn_round_after_7
#[test]
fn an_unclaimed_prize_expires_after_30_days_and_an_undrawn_round_after_7() {
    let mut env = ready();
    ok(env.fund_prize());
    let tree = holders(&[1, 1, 1]);
    ok(commit(&mut env, &tree));
    expect_err(expire(&mut env), BooksError::RaffleNotExpired);
    let c = env.raffle().committed_at;
    env.set_time(c + RAFFLE_DRAW_TIMEOUT - 1);
    expect_err(expire(&mut env), BooksError::RaffleNotExpired);
    env.set_time(c + RAFFLE_DRAW_TIMEOUT);
    ok(expire(&mut env));
    let r = env.raffle();
    assert_eq!((r.phase, r.rounds_cancelled, r.rounds_expired), (RafflePhase::Idle, 1, 0));
    assert_eq!(env.balance(&PRIZE), ONE, "the prize stays");
    ok(commit(&mut env, &tree));
    ok(prove_and_draw(&mut env, [9; 32]));
    let d = env.raffle().drawn_at;
    expect_err(expire(&mut env), BooksError::RaffleNotExpired);
    env.set_time(d + 30 * DAY - 1);
    expect_err(expire(&mut env), BooksError::RaffleNotExpired);
    env.set_time(d + RAFFLE_CLAIM_WINDOW);
    let w = tree.winner(env.raffle().ticket);
    expect_err(claim_leaf(&mut env, &tree, w), BooksError::ClaimWindowOver);
    ok(expire(&mut env));
    let r = env.raffle();
    assert_eq!((r.phase, r.rounds_expired, r.prizes_claimed), (RafflePhase::Idle, 1, 0));
    assert_eq!(env.balance(&PRIZE), ONE);
    ok(env.post(FLOOR));
    expect_err(env.fund_prize(), BooksError::PrizeHeld);
    ok(commit(&mut env, &tree));
    ok(prove_and_draw(&mut env, [10; 32]));
    let w = tree.winner(env.raffle().ticket);
    ok(claim_leaf(&mut env, &tree, w));
    assert_eq!(env.raffle().prizes_funded, 1, "one purchase served two rounds");
    invariants(&env);
}

// rook_core raffle.rs: from_treasury_overflow_to_a_winner
#[test]
fn from_treasury_overflow_to_a_winner() {
    let mut env = one_collection();
    ok(env.b.treasury.set_ops(OPS, env.now));
    ok(init_raffle(&mut env));
    let cap = env.cap_of();
    env.fund_round((cap + 3 * FLOOR) * 10 / 9);
    invariants(&env);
    ok(env.distribute(0));
    invariants(&env);
    assert!(env.raffle().pot > FLOOR);
    ok(env.post(FLOOR));
    let supply0 = env.supply();
    ok(env.fund_prize());
    assert_eq!(env.supply(), supply0 + ONE);
    invariants(&env);
    let tree = holders(&[ONE, 2 * ONE, 3 * ONE, 4 * ONE]);
    ok(commit(&mut env, &tree));
    ok(prove_and_draw(&mut env, [0xab; 32]));
    let w = tree.winner(env.raffle().ticket);
    ok(claim_leaf(&mut env, &tree, w));
    invariants(&env);
    assert_eq!(env.supply(), supply0 + ONE, "a transfer: the supply is unchanged by the claim");
}

/// rook_core raffle.rs `VECTOR_HOLDERS` / `VECTOR_ROOT` (the canister's cross-check vector).
const VECTOR_HOLDERS: [([u8; 32], u64); 5] = [([4; 32], 10), ([1; 32], 5), ([3; 32], 1), ([5; 32], 3), ([2; 32], 7)];
const VECTOR_ROOT: &str = "de15ed1483f99479cda54f46d94501fd58c66abbec3065de58c8fec113de9692";

// rook_core raffle.rs: the_canister_tree_vector_commits_and_every_leaf_claims
#[test]
fn the_canister_tree_vector_commits_and_every_leaf_claims() {
    let tree = Tree::new(&VECTOR_HOLDERS);
    let hx: String = tree.root().iter().map(|x| format!("{x:02x}")).collect();
    assert_eq!(hx, VECTOR_ROOT, "the canister's cross-check vector");
    assert_eq!(tree.leaves.iter().map(|l| (l.1, l.2)).collect::<Vec<_>>(), vec![(0, 5), (5, 7), (12, 1), (13, 10), (23, 3)]);
    for (i, (o, c, w)) in tree.leaves.iter().enumerate() {
        assert!(etch_autobuyer::books::math::verify_proof(&tree.proof(i), &tree.root(), raffle_leaf(o, *c, *w)), "leaf {i}");
    }
    let mut env = ready();
    env.top_up(2 * FLOOR);
    ok(env.fund_prize());
    ok(commit(&mut env, &tree));
    ok(prove_and_draw(&mut env, randomness_for(1, tree.total, 17)));
    assert_eq!(env.raffle().ticket, 17);
    ok(claim_leaf(&mut env, &tree, 3));
    assert_eq!(env.balance(&[4; 32]), ONE);
}
