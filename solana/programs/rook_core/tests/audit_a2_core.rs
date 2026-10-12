//! Audit 2026-10, area A2 (docs/v9/audit-2026-10/a2-core-registry-wrap.md): proofs of concept.
//!
//! `poc_*` tests assert the property the code should have; they FAIL on the audited commit (f2c24e0) while the finding stands.
//! `demo_*` tests asserted that an attack works (they passed while the finding stood).
//! Fixes of 2026-10-10: both `poc_a2_01_*` pass; the A2-02, A2-03 and A2-04 demos are inverted into `fixed_*` regressions that
//! expect the refusals.
//!
//! Build the SBPF v2 binary first:
//!   cargo build-sbf --manifest-path programs/rook_core/Cargo.toml --arch v2 --sbf-out-dir target/deploy-sbpfv2
//! then: cargo test -p rook_core --test audit_a2_core

#[path = "suite/common.rs"]
mod common;

use anchor_lang::prelude::Pubkey;
use anchor_lang::solana_program::instruction::AccountMeta;
use common::*;
use rook_core::clock::round_at;
use rook_core::error::RookError;
use rook_core::hashing::keccak;
use rook_core::seeds::*;
use rook_core::state::*;
use solana_keypair::Keypair;
use solana_signer::Signer;

fn add_collection(env: &mut Env, module: Pubkey) -> Res {
    let o = env.owner.insecure_clone();
    let i = ix(
        rook_core::instruction::AddCollection {},
        rook_core::accounts::AddCollection { owner: o.pubkey(), registry: env.registry, treasury: env.treasury, module },
    );
    env.send(&[i], &[&o])
}

fn receive(env: &mut Env, who: &Keypair, lamports: u64) -> Res {
    let i = ix(
        rook_core::instruction::Receive { lamports },
        rook_core::accounts::Receive {
            payer: who.pubkey(),
            treasury: env.treasury,
            system_program: anchor_lang::solana_program::system_program::ID,
        },
    );
    let w = who.insecure_clone();
    env.send(&[i], &[&w])
}

fn allocate(env: &mut Env) -> Res {
    let u = env.user.insecure_clone();
    let i = ix(rook_core::instruction::Allocate {}, rook_core::accounts::Allocate { treasury: env.treasury, ops: None });
    env.send(&[i], &[&u])
}

fn begin(env: &mut Env, who: &Keypair, modules: &[Pubkey]) -> Res {
    let mut i = ix(rook_core::instruction::BeginDistribution {}, rook_core::accounts::BeginDistribution { treasury: env.treasury });
    for m in modules {
        i.accounts.push(AccountMeta::new_readonly(*m, false));
    }
    let w = who.insecure_clone();
    env.send(&[i], &[&w])
}

fn distribute(env: &mut Env, who: &Keypair, index: u8, module: Pubkey) -> Res {
    let vault = pda(&[MODULE_VAULT, module.as_ref()]);
    let i = ix(
        rook_core::instruction::Distribute { index },
        rook_core::accounts::Distribute {
            treasury: env.treasury,
            module,
            module_vault: vault,
            ops: None,
            raffle: pda(&[RAFFLE, env.get::<rook_core::state::Module>(&module).listing.as_ref()]),
        },
    );
    let w = who.insecure_clone();
    env.send(&[i], &[&w])
}

/// Two collections with equal seven-day volume and 40 SOL in the reserves pot.
fn two_equal_collections() -> (Env, Pubkey, Pubkey) {
    let mut env = Env::new();
    let m0 = env.module;
    let c = env.custodian.pubkey();
    ok(env.list(ListingKind::EthNft { chain_id: 1, collection: [2u8; 20] }, "Azuki", "0x1111111111111111111111111111111111111111", c, c));
    env.init_wrapper();
    env.init_module(default_params());
    let m1 = env.module;
    ok(add_collection(&mut env, m0));
    ok(add_collection(&mut env, m1));
    let u = env.user.insecure_clone();
    ok(env.post(FLOOR));
    let acc1 = env.token_account(&u.pubkey());
    ok(env.mint_units(&u, &acc1, 100 * SOL, 0));
    env.use_listing(0);
    ok(env.post(FLOOR));
    let acc0 = env.token_account(&u.pubkey());
    ok(env.mint_units(&u, &acc0, 100 * SOL, 0));
    let w0 = env.get::<Module>(&m0).volume_7d(env.now);
    let w1 = env.get::<Module>(&m1).volume_7d(env.now);
    assert_eq!(w0, w1, "equal weights");
    ok(receive(&mut env, &u, 40 * SOL));
    ok(allocate(&mut env));
    assert_eq!(env.get::<Treasury>(&env.treasury).reserves_pot, 40 * SOL);
    (env, m0, m1)
}

/// A2-01 (Medium after verification; fixed 2026-10-10): `begin_distribution` restarted an unfinished round for anyone,
/// clearing `round_paid`, so a collection already paid was paid again from what was left. Repeating `begin_distribution` + `distribute(i)` drains the whole reserves pot into
/// collection `i`, whatever the weights. Expected: with equal weights, each collection receives half of the 40 SOL.
#[test]
fn poc_a2_01_restarting_a_distribution_round_drains_the_pot_into_one_collection() {
    let (mut env, m0, m1) = two_equal_collections();
    let attacker = Keypair::new();
    env.svm.airdrop(&attacker.pubkey(), 10 * SOL).unwrap();
    let mods = [m0, m1];
    // anyone, permissionless: pay collection 0, restart, pay collection 0 again, ... (30 rounds)
    for _ in 0..30 {
        if begin(&mut env, &attacker, &mods).is_err() {
            break;
        }
        if distribute(&mut env, &attacker, 0, m0).is_err() {
            break;
        }
    }
    // the honest crank finally pays collection 1
    let _ = begin(&mut env, &attacker, &mods);
    let _ = distribute(&mut env, &attacker, 1, m1);
    let s0 = env.get::<Module>(&m0).surplus;
    let s1 = env.get::<Module>(&m1).surplus;
    println!("collection 0 received {} lamports, collection 1 received {} lamports", s0, s1);
    assert_eq!(s0, 20 * SOL, "A2-01: collection 0 took {} of the 40 SOL pot (equal weights: 20 SOL each)", s0);
    assert_eq!(s1, 20 * SOL, "A2-01: collection 1 got {} instead of its 20 SOL", s1);
}

/// A2-01, the accidental variant: two honest cranks racing (one begins a round while the other is mid-way) paid the first
/// collections twice. Expected: a collection is paid at most once per pot. Fixed 2026-10-10: a second `begin_distribution`
/// is refused while the round is open (`RoundOpen`), a finished round may be followed at once, and a stale one (`ROUND_TIMEOUT`,
/// one day) may be restarted.
#[test]
fn poc_a2_01_a_racing_crank_pays_a_collection_twice() {
    let (mut env, m0, m1) = two_equal_collections();
    let u = env.user.insecure_clone();
    ok(begin(&mut env, &u, &[m0, m1]));
    ok(distribute(&mut env, &u, 0, m0));
    // a second crank starts its own round: refused while collection 1 is still owed its share
    expect_err(begin(&mut env, &u, &[m0, m1]), RookError::RoundOpen);
    let second = distribute(&mut env, &u, 0, m0);
    assert!(second.is_err(), "A2-01: collection 0 was paid a second time in the same distribution");
    ok(distribute(&mut env, &u, 1, m1));
    assert_eq!(env.get::<Module>(&m0).surplus, 20 * SOL);
    assert_eq!(env.get::<Module>(&m1).surplus, 20 * SOL);
    // the round is finished: the next one may begin at once
    ok(begin(&mut env, &u, &[m0, m1]));
}

/// A2-01's timeout: a round left unfinished for `ROUND_TIMEOUT` (one day) may be restarted with the pot as it then is.
#[test]
fn fixed_a2_01_a_stale_round_restarts_after_a_day() {
    let (mut env, m0, m1) = two_equal_collections();
    let u = env.user.insecure_clone();
    ok(begin(&mut env, &u, &[m0, m1]));
    ok(distribute(&mut env, &u, 0, m0));
    env.advance_time(rook_core::state::ROUND_TIMEOUT - 10);
    expect_err(begin(&mut env, &u, &[m0, m1]), RookError::RoundOpen);
    env.advance_time(10);
    ok(begin(&mut env, &u, &[m0, m1]));
    let t = env.get::<Treasury>(&env.treasury);
    assert_eq!((t.round_pot, t.round_paid, t.round_started_at), (20 * SOL, 0, env.now));
}

/// A2-02 (Medium, the deferred drand trust, assessed): the keeper wrote any randomness for any round, including rounds that
/// did not exist yet, so a public pre-image of future draws let anyone time a redemption. Fixed 2026-10-10: `prove_round`
/// refuses a round after `round_at(now)` (`RoundInFuture`). Residual (Q4, by design until the BLS check): the keeper still
/// chooses a LIVE round's randomness.
#[test]
fn fixed_a2_02_the_keeper_cannot_prove_a_round_that_is_not_live() {
    let mut env = Env::new();
    let u = env.user.insecure_clone();
    let acc = env.token_account(&u.pubkey());
    for t in 1..=4u64 {
        ok(env.wrap_nft(t, tx_hash(t), &acc));
    }
    ok(env.redeem(&u, &acc, Destination::Eth([0x22; 20])));
    let tk = env.ticket(0);
    // the round is in the future (20 rounds ahead): the keeper may not prove it now, nor any round beyond it
    assert!(tk.round > round_at(env.now));
    expect_err(env.prove(round_at(env.now) + 100_000, [9u8; 32]), RookError::RoundInFuture);
    expect_err(env.prove(tk.round, [9u8; 32]), RookError::RoundInFuture);
    // the live round is fine
    ok(env.prove(round_at(env.now), [9u8; 32]));
    // once the ticket's round is live, it is proven and resolved
    env.reach_round(tk.round);
    ok(env.prove(tk.round, [5u8; 32]));
    let i = env.resolve_ix(0, true);
    ok(env.send(&[i], &[&u]));
    assert_eq!(env.ticket(0).stage, Stage::Resolved);
}

/// A2-04 (Low): while the queue was blocked past `PIN_TIMEOUT` (an unproven round ahead of it), a pinned ticket whose round
/// was already proven could be refunded by its owner after reading the outcome: a free re-roll. Fixed 2026-10-10: `refund`
/// takes the ticket's `["drand", round]` PDA and refuses a pinned ticket whose round exists (`RoundProven`). No trap: the
/// blocking ticket (round never proven, pinned earlier) is refundable by anyone, and then the proven ticket resolves.
#[test]
fn fixed_a2_04_a_proven_pinned_ticket_resolves_instead_of_refunding() {
    let mut env = Env::new();
    let u = env.user.insecure_clone();
    let acc = env.token_account(&u.pubkey());
    for t in 1..=3u64 {
        ok(env.wrap_nft(t, tx_hash(t), &acc));
    }
    ok(env.redeem(&u, &acc, Destination::Eth([0x22; 20]))); // ticket 0: its round is never proven (keeper outage)
    env.advance_time(120);
    ok(env.redeem(&u, &acc, Destination::Eth([0x22; 20]))); // ticket 1: its round is proven
    let r1 = env.ticket(1).round;
    env.reach_round(r1);
    ok(env.prove(r1, [5u8; 32]));
    // ticket 1 cannot resolve: ticket 0 blocks the queue
    let i = env.resolve_ix(1, true);
    expect_err(env.send(&[i], &[&u]), RookError::OutOfOrder);
    env.advance_time(7 * 86_400 + 200);
    // ticket 1's outcome is public: no refund (the free option is gone)
    expect_err(env.refund(1, &acc), RookError::RoundProven);
    // ticket 0 (never proven) is refundable by anyone; the queue steps past it (refund advances `next_to_resolve`) and
    // ticket 1 resolves
    let r = ok(env.refund(0, &acc));
    println!("refund (pinned; reads the drand PDA): {} CU", r.compute_units_consumed);
    let i = env.resolve_ix(1, true);
    ok(env.send(&[i], &[&u]));
    assert_eq!((env.ticket(0).stage, env.ticket(1).stage), (Stage::Refunded, Stage::Resolved));
    env.check_ledger();
}

const TB_CUSTODY: &str = "tb1p5cyxnuxmeuwuvkwfem96lqzszd02n6xdcjrs20cac6yqjjwudpxqp3mvzv";

fn frog(id: &str, proof: Vec<[u8; 32]>, txid: [u8; 32]) -> rook_core::instructions::MintItemArgs {
    rook_core::instructions::MintItemArgs {
        key: rook_core::hashing::leaf_of(id),
        source: keccak(&[&txid, &0u32.to_be_bytes()]),
        tx: txid,
        vout: 0,
        deposit: true,
        inscription_id: id.into(),
        proof,
    }
}

/// A2-03 (Medium): an Inscriptions redemption could name the listing's own custody address; the canister classes such a
/// ticket as a relist (never delivered) and rook_core refunded it after the late window: a free, repeatable lock on held
/// items. Fixed 2026-10-10: `redeem` refuses `destination == listing.custody` (`CustodyDestination`), as the EthNft branch.
#[test]
fn fixed_a2_03_an_inscription_redemption_to_custody_is_refused() {
    let mut env = Env::bare_on(BitcoinNetwork::Signet);
    let a = "6fb976ab49dcec017f1e201e84395983204ae1a7c2abf7ced0a85d692e442799i0";
    let b = "a1b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f90i1";
    let (la, lb) = (rook_core::hashing::leaf_of(a), rook_core::hashing::leaf_of(b));
    let root = if la <= lb { keccak(&[&la, &lb]) } else { keccak(&[&lb, &la]) };
    let c = env.custodian.pubkey();
    ok(env.list(ListingKind::Inscriptions { membership_root: root }, "Frogs", TB_CUSTODY, c, c));
    env.init_wrapper();
    env.init_module(default_params());
    let attacker = Keypair::new();
    env.svm.airdrop(&attacker.pubkey(), 100 * SOL).unwrap();
    let acc = env.token_account(&attacker.pubkey());
    let cu = env.custodian.insecure_clone();
    ok(env.mint_item_as(&cu, frog(a, vec![lb], tx_hash(1)), &acc));
    ok(env.mint_item_as(&cu, frog(b, vec![la], tx_hash(2)), &acc));
    expect_err(env.redeem(&attacker, &acc, Destination::Taproot(TB_CUSTODY.into())), RookError::CustodyDestination);
    let w = env.wrapper();
    assert_eq!((w.held, w.open_tickets), (2, 0));
    assert_eq!(env.balance(&acc), 2 * rook_core::units::ONE, "nothing burned");
    env.check_ledger();
}

/// A2-I (informational): `begin_distribution` deserializes every registered module; the cost per module, to size the
/// `MAX_COLLECTIONS = 32` case (which also needs an address lookup table: 32 module keys do not fit a legacy transaction).
#[test]
fn measure_begin_distribution_cu() {
    let (mut env, m0, m1) = two_equal_collections();
    let u = env.user.insecure_clone();
    let one = {
        let mut i = ix(rook_core::instruction::BeginDistribution {}, rook_core::accounts::BeginDistribution { treasury: env.treasury });
        i.accounts.push(AccountMeta::new_readonly(m0, false));
        i.accounts.push(AccountMeta::new_readonly(m1, false));
        ok(env.send(&[i], &[&u])).compute_units_consumed
    };
    let size = env.svm.get_account(&m0).unwrap().data.len();
    println!("begin_distribution with 2 modules: {} CU; a Module account is {} bytes", one, size);
}
