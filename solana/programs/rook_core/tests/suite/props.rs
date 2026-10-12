//! Property tests over random sequences (a deterministic xorshift RNG, several seeds), ported from
//! FloorModule.invariant.t.sol, FloorReserve.t.sol testFuzz_wXStaysFullyBacked, FloorAttacks.t.sol testFuzz_aLoneTraderNeverProfits,
//! Advance.t.sol testFuzz_advanceThenSettleOrReturn and EthNft.invariant.t.sol. After every step:
//! - the module vault holds at least the sum of the books (backing + reserve + surplus + revenue);
//! - wX supply = (held − open tickets)·10⁹ + eth_backed;
//! - a burn never pays more than the backing per unit of the tiers it draws;
//! - a conversion at a stale floor (or with the poster down) refuses;
//! - a lone mint-then-burn never profits.

use super::common::*;
use anchor_lang::prelude::Pubkey;
use rook_core::error::RookError;
use rook_core::state::*;
use rook_core::units::ONE;
use solana_keypair::Keypair;
use solana_signer::Signer;

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
}

struct World {
    env: Env,
    conv: Keypair,
    conv_wx: Pubkey,
    user_wx: Pubkey,
    sink: Pubkey,
    nft: u64,
    poster_paused: bool,
}

fn world() -> World {
    let mut env = Env::new();
    let conv = Keypair::new();
    env.svm.airdrop(&conv.pubkey(), 90_000 * SOL).unwrap();
    env.allow_converter(&conv.pubkey());
    let conv_wx = env.token_account(&conv.pubkey());
    let u = env.user.pubkey();
    let user_wx = env.token_account(&u);
    let sink = Keypair::new().pubkey();
    env.svm.airdrop(&sink, SOL).unwrap();
    ok(env.post(FLOOR));
    World { env, conv, conv_wx, user_wx, sink, nft: 1, poster_paused: false }
}

/// The most a burn of `units` may pay: each tier at its own backing per unit (reserve units first).
fn burn_bound(m: &Module, e: u64, units: u64) -> u128 {
    let live = m.live_units(e);
    let r = m.reserve_units;
    let r_u = units.min(r);
    let c_u = units - r_u;
    let mut b: u128 = 0;
    if r_u != 0 {
        b += r_u as u128 * m.reserve as u128 / r as u128;
    }
    if c_u != 0 {
        b += c_u as u128 * m.backing as u128 / (live - r) as u128;
    }
    b
}

fn step(w: &mut World, rng: &mut Rng) {
    let env = &mut w.env;
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
            let c = env.custodian.insecure_clone();
            let _ = env.post_as(&c, price, conf, 1 + rng.below(30) as u32);
        }
        3 => {
            let u = env.user.insecure_clone();
            let _ = env.mint_units(&u, &w.user_wx.clone(), 1 + rng.below(400 * SOL), 0);
        }
        4 => {
            let bal = env.balance(&w.user_wx);
            if bal == 0 {
                return;
            }
            let units = 1 + rng.below(bal);
            let m = env.module();
            let e = env.wrapper().eth_backed;
            let bound = burn_bound(&m, e, units.min(m.live_units(e)));
            let before = env.lamports(&w.sink);
            let u = env.user.insecure_clone();
            if env.burn_units(&u, &w.user_wx.clone(), &w.sink.clone(), units, 0).is_ok() {
                let paid = env.lamports(&w.sink) - before;
                assert!(paid as u128 <= bound, "a burn paid {} above the backing bound {}", paid, bound);
            }
        }
        5 => {
            let m = env.module();
            let fresh = m.fresh(env.now) && !w.poster_paused;
            let conv = w.conv.insecure_clone();
            let r = env.convert(&conv, &w.conv_wx.clone(), 1 + rng.below(300 * SOL), 0);
            if !m.fresh(env.now) {
                expect_err(r, RookError::Stale);
            } else if w.poster_paused {
                expect_err(r, RookError::Paused);
            } else {
                let _ = (r, fresh);
            }
        }
        6 => {
            let m = env.module();
            let e = env.wrapper().eth_backed;
            if m.transit_units != 0 {
                // settle (an item bought) or return (it could not be)
                if rng.below(2) == 0 {
                    let mwx = env.module_wx;
                    let n = w.nft;
                    w.nft += 1;
                    if env.wrap_nft(n, tx_hash(n), &mwx).is_ok() {
                        let cost = 1 + rng.below(m.advanced);
                        ok(env.settle(cost));
                        return;
                    }
                }
                let back = rng.below(m.advanced + m.advanced / 5);
                let c = env.custodian.insecure_clone();
                ok(env.return_advance_as(&c, back));
                return;
            }
            let max = m.max_item_cost(env.now, e, w.poster_paused).unwrap_or(0);
            if max > 0 {
                ok(env.advance(1 + rng.below(max)));
            }
        }
        7 => {
            let _ = env.fund_reserve(1 + rng.below(5 * SOL));
        }
        8 => {
            let _ = env.skim();
        }
        9 => {
            if w.poster_paused {
                let o = env.owner.insecure_clone();
                ok(env.listing_admin(rook_core::instruction::UnpausePoster {}, &o));
            } else {
                let g = env.guardian.insecure_clone();
                ok(env.listing_admin(rook_core::instruction::PausePoster {}, &g));
            }
            w.poster_paused = !w.poster_paused;
        }
        _ => {
            // a lone trader: mint, then burn the same units at once (FloorAttacks testFuzz_aLoneTraderNeverProfits)
            let t = Keypair::new();
            env.svm.airdrop(&t.pubkey(), 1_000 * SOL).unwrap();
            let acc = env.token_account(&t.pubkey());
            let pay = 1 + rng.below(300 * SOL);
            if env.mint_units(&t, &acc, pay, 0).is_err() {
                return;
            }
            let units = env.balance(&acc);
            let before = env.lamports(&w.sink);
            if env.burn_units(&t, &acc, &w.sink.clone(), units, 0).is_ok() {
                let got = env.lamports(&w.sink) - before;
                assert!(got <= pay, "a lone trader profited: paid {} got {}", pay, got);
            }
        }
    }
}

fn run(seed: u64, steps: usize) {
    let mut w = world();
    let mut rng = Rng(seed);
    for _ in 0..steps {
        step(&mut w, &mut rng);
        w.env.check_books();
        w.env.check_ledger();
        let m = w.env.module();
        assert!(m.reserve_units <= w.env.wrapper().eth_backed);
        assert!(m.transit_units <= ONE);
    }
}

#[test]
fn books_and_ledger_hold_seed_1() {
    run(0x9E37_79B9_7F4A_7C15, 250);
}

#[test]
fn books_and_ledger_hold_seed_2() {
    run(0xD1B5_4A32_D192_ED03, 250);
}

#[test]
fn books_and_ledger_hold_seed_3() {
    run(0x2545_F491_4F6C_DD1D, 250);
}

#[test]
fn books_and_ledger_hold_seed_4() {
    run(0xA076_1D64_78BD_642F, 250);
}

/// EthNft.invariant.t.sol / testFuzz_theLedgerHoldsThroughAnyMixOfDeliveriesAndRefunds: random wraps, redeems, resolutions,
/// deliveries, undeliverable marks and refunds; the ledger holds after each.
#[test]
fn the_wx_ledger_holds_through_any_mix_of_deliveries_and_refunds() {
    for seed in [7u64, 11, 13] {
        let mut env = Env::new();
        let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15));
        let o = env.owner.insecure_clone();
        ok(env.wrapper_admin(rook_core::instruction::InitMintLimit { limit: 64 }, &o));
        let h = Keypair::new();
        env.svm.airdrop(&h.pubkey(), 10_000 * SOL).unwrap();
        let acc = env.token_account(&h.pubkey());
        let c = env.custodian.insecure_clone();
        let mut nft = 0u64;
        for _ in 0..200 {
            let w = env.wrapper();
            match rng.below(7) {
                0 | 1 => {
                    nft += 1;
                    let _ = env.wrap_nft(nft, tx_hash(nft), &acc);
                }
                2 => {
                    if env.balance(&acc) >= ONE && w.held > w.open_tickets {
                        ok(env.redeem(&h, &acc, if rng.below(2) == 0 { Destination::Burn } else { Destination::Eth([1; 20]) }));
                    }
                }
                3 => {
                    if w.next_to_resolve < w.ticket_count {
                        let id = w.next_to_resolve;
                        if env.ticket(id).stage == Stage::Refunded {
                            let i = env.resolve_ix(id, false);
                            let u = env.user.insecure_clone();
                            ok(env.send(&[i], &[&u]));
                        } else {
                            ok(env.resolve(id));
                        }
                    }
                }
                4 => {
                    if w.ticket_count > 0 {
                        let id = rng.below(w.ticket_count);
                        if env.ticket(id).stage == Stage::Resolved {
                            if rng.below(2) == 0 {
                                ok(env.record_delivery(id, tx_hash(1_000 + id)));
                            } else {
                                ok(env.custodian_ticket(rook_core::instruction::MarkUndeliverable {}, id, &c));
                            }
                        }
                    }
                }
                5 => {
                    if w.ticket_count > 0 {
                        let id = rng.below(w.ticket_count);
                        let _ = env.refund(id, &acc);
                    }
                }
                _ => env.advance_time(rng.below(4 * 86_400) as i64),
            }
            env.check_ledger();
            let w = env.wrapper();
            let s = env.items();
            assert!(w.held >= w.open_tickets);
            assert!(s.free.len() as u64 + s.pending.len() as u64 <= w.held);
        }
    }
}
