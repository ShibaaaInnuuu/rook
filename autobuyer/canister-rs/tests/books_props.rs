//! Property tests over random sequences (deterministic xorshift, several seeds), ported from
//! solana/programs/rook_core/tests/suite/props.rs (FloorModule.invariant.t.sol, FloorReserve.t.sol testFuzz_wXStaysFullyBacked,
//! FloorAttacks.t.sol testFuzz_aLoneTraderNeverProfits, Advance.t.sol testFuzz_advanceThenSettleOrReturn). After every step:
//! - each module address holds at least its books ([`books_balance`]);
//! - wX supply = (held − open tickets)·10⁹ + sol_backed − module units ([`supply_matches`]);
//! - no unit can be minted and burned at once for a profit ([`no_free_profit`]);
//! - a burn never pays more than the backing per unit of the tiers it draws;
//! - a mint at a stale floor (or with the poster down) refuses (rook_core: a conversion; `convert` is dropped);
//! - a lone mint-then-burn never profits;
//! - (new) a refused operation leaves the books exactly as they were.

mod books_common;

use books_common::*;
use etch_autobuyer::books::*;

struct World {
    env: Env,
    nft: u64,
    poster_paused: bool,
    /// Operations that went through: (mints, burns, advances, settles).
    done: [u32; 4],
}

const SINK: [u8; 32] = [0x55; 32];

fn world() -> World {
    let mut env = Env::new();
    ok(env.post(FLOOR));
    World { env, nft: 1, poster_paused: false, done: [0; 4] }
}

fn step(w: &mut World, rng: &mut Rng) {
    let env = &mut w.env;
    let before = env.b.clone();
    match rng.below(11) {
        0 => {
            let dt = match rng.below(10) {
                0 => 86_400,
                1 => 8 * 86_400,
                _ => 60 + rng.below(3 * 3_600) as i64,
            };
            env.advance_time(dt);
        }
        1 | 2 => {
            let a = env.module().accepted.max(FLOOR / 4);
            let price = a / 10 * (8 + rng.below(5)) + 1;
            let conf = 2_500 + rng.below(7_501) as u16;
            if env.post_with(price, conf, 1 + rng.below(30) as u32).is_err() {
                assert_eq!(env.b, before, "a refused post");
            }
        }
        3 => {
            match env.mint_units(USER, 1 + rng.below(400 * SOL), 0) {
                Ok(_) => w.done[0] += 1,
                Err(_) => assert_eq!(env.b, before, "a refused mint"),
            }
        }
        4 => {
            let bal = env.balance(&USER);
            if bal == 0 {
                return;
            }
            let units = 1 + rng.below(bal);
            let m = env.module();
            let bound = m.burn_bound(env.e(), units);
            match env.burn_units(USER, SINK, units, 0) {
                Ok(paid) => {
                    w.done[1] += 1;
                    assert!(paid as u128 <= bound, "a burn paid {paid} above the backing bound {bound}")
                }
                Err(_) => assert_eq!(env.b, before, "a refused burn"),
            }
        }
        5 => {
            // rook_core: a conversion at a stale floor or with the poster down refuses; here the public mint
            let m = env.module();
            let r = env.mint_units(HOLDER, 1 + rng.below(300 * SOL), 0);
            if !m.fresh(env.now) {
                expect_err(r, BooksError::Stale);
            } else if w.poster_paused {
                expect_err(r, BooksError::Paused);
            }
        }
        6 => {
            let m = env.module();
            let e = env.e();
            if m.transit_units != 0 {
                if rng.below(2) == 0 {
                    let n = w.nft;
                    w.nft += 1;
                    if env.wrap_nft(n, tx_hash(n), MintTo::Module).is_ok() {
                        let cost = 1 + rng.below(m.advanced);
                        ok(env.settle(cost));
                        w.done[3] += 1;
                        return;
                    }
                }
                let back = rng.below(m.advanced + m.advanced / 5);
                ok(env.return_advance(back));
                return;
            }
            let max = m.max_item_cost(env.now, e, w.poster_paused).unwrap_or(0);
            if max > 0 {
                ok(env.advance(1 + rng.below(max)));
                w.done[2] += 1;
            }
        }
        7 => {
            let _ = env.fund_reserve(1 + rng.below(5 * SOL));
        }
        8 => {
            if env.skim().is_err() {
                assert_eq!(env.b, before, "a refused skim");
            }
        }
        9 => {
            if w.poster_paused {
                env.unpause_poster();
            } else {
                env.pause_poster();
            }
            w.poster_paused = !w.poster_paused;
        }
        _ => {
            // a lone trader: mint, then burn the same units at once (FloorAttacks testFuzz_aLoneTraderNeverProfits)
            let t = wallet(rng.next());
            let pay = 1 + rng.below(300 * SOL);
            let Ok(units) = env.mint_units(t, pay, 0) else { return };
            if let Ok(got) = env.burn_units(t, SINK, units, 0) {
                assert!(got <= pay, "a lone trader profited: paid {pay} got {got}");
            }
        }
    }
}

fn run(seed: u64, steps: usize) -> [u32; 4] {
    let mut w = world();
    let mut rng = Rng(seed);
    for _ in 0..steps {
        step(&mut w, &mut rng);
        w.env.check_books();
        w.env.check_ledger();
        ok(check_collection(&w.env.b, ID, w.env.vault_free(), w.env.supply(), w.env.now));
        let m = w.env.module();
        assert!(m.reserve_units <= w.env.e());
        assert!(m.transit_units <= ONE);
    }
    assert!(w.done[..3].iter().all(|n| *n > 0), "the walk reached mints, burns and advances: {:?}", w.done);
    w.done
}

// rook_core props.rs: books_and_ledger_hold_seed_1
#[test]
fn books_and_ledger_hold_seed_1() {
    run(0x9E37_79B9_7F4A_7C15, 250);
}

// rook_core props.rs: books_and_ledger_hold_seed_2
#[test]
fn books_and_ledger_hold_seed_2() {
    run(0xD1B5_4A32_D192_ED03, 250);
}

// rook_core props.rs: books_and_ledger_hold_seed_3
#[test]
fn books_and_ledger_hold_seed_3() {
    run(0x2545_F491_4F6C_DD1D, 250);
}

// rook_core props.rs: books_and_ledger_hold_seed_4
#[test]
fn books_and_ledger_hold_seed_4() {
    run(0xA076_1D64_78BD_642F, 250);
}

// New: the same walk, longer, over more seeds, and with the value-based parameters of §4.3 (a cheap collection's caps).
#[test]
fn books_and_ledger_hold_many_seeds_and_value_params() {
    let mut settles = 0;
    for seed in 1..=24u64 {
        settles += run(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1, 400)[3];
    }
    assert!(settles > 10, "the walks settle advances: {settles}");
    let p = value_params(FLOOR / 100, 174, 2, REVENUE_TARGET).unwrap();
    for seed in 1..=8u64 {
        let mut w = World { env: Env::with_params(p), nft: 1, poster_paused: false, done: [0; 4] };
        ok(w.env.post(FLOOR));
        let mut rng = Rng(seed.wrapping_mul(0xD1B5_4A32_D192_ED03) | 1);
        for _ in 0..400 {
            step(&mut w, &mut rng);
            w.env.check_books();
            w.env.check_ledger();
        }
    }
}
