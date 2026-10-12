//! The full path with a fake custodian keypair (EthNftFlow.t.sol: test_theWholeLoop_buyRedeemDeliver,
//! test_aGraduationsBudgetBuysAnNft_andTheUnitBecomesItemBacked): a graduated coin's SOL is converted (as rook_backing will),
//! the canister sees the buy signal, takes an advance, buys the NFT on Ethereum (off-chain), wraps it into the module and
//! settles; a holder redeems a unit, the round resolves it, the canister delivers. Compute units are measured on the way.

use super::common::*;
use rook_core::state::*;
use rook_core::units::{BPS, ONE};
use solana_keypair::Keypair;
use solana_signer::Signer;

#[test]
fn ethereum_to_wx_to_redeem_to_deliver() {
    let mut env = Env::new();
    let mut cu: Vec<(&str, u64)> = vec![];

    // the canister posts the floor (10 ETH ≈ 200 SOL)
    let r = ok(env.post(FLOOR));
    cu.push(("post_floor", r.compute_units_consumed));

    // a graduated coin's backing share arrives through convert (rook_backing's role, here a keypair)
    let backing = Keypair::new();
    env.svm.airdrop(&backing.pubkey(), 10_000 * SOL).unwrap();
    env.allow_converter(&backing.pubkey());
    let vault_wx = env.token_account(&backing.pubkey());
    let r = ok(env.convert(&backing, &vault_wx, 824 * SOL / 10 * 5, 0)); // 412 SOL: ≈2 units
    cu.push(("convert", r.compute_units_consumed));
    let units = env.balance(&vault_wx);
    assert!(units > 19 * ONE / 10);
    env.check_books();
    env.check_ledger();

    // the canister's buy signal: netUnits ≥ 1 + buffer
    let m = env.module();
    let e = env.wrapper().eth_backed;
    assert!(m.net_units(e) >= ONE + ONE * m.params.buffer_bps as u64 / BPS);

    // advance → (buy on Ethereum) → wrap into the module → settle
    let max = m.max_item_cost(env.now, e, false).unwrap();
    let r = ok(env.advance(max));
    cu.push(("advance", r.compute_units_consumed));
    env.check_books();
    let mwx = env.module_wx;
    let r = ok(env.wrap_nft(8_888, tx_hash(8_888), &mwx));
    cu.push(("mint_item", r.compute_units_consumed));
    let r = ok(env.settle(max - SOL / 2));
    cu.push(("settle_advance", r.compute_units_consumed));
    let w = env.wrapper();
    assert_eq!(w.held, 1);
    assert_eq!(w.eth_backed, units - ONE + 0);
    env.check_books();
    env.check_ledger();

    // the vault's units are a claim on one NFT and on SOL; anyone with a whole unit redeems the NFT
    let holder = Keypair::new();
    env.svm.airdrop(&holder.pubkey(), 100 * SOL).unwrap();
    let hacc = env.token_account(&holder.pubkey());
    let t = anchor_spl::token::spl_token::instruction::transfer(
        &anchor_spl::token::ID,
        &vault_wx,
        &hacc,
        &backing.pubkey(),
        &[],
        ONE,
    )
    .unwrap();
    ok(env.send(&[t], &[&backing]));

    // a public mint and a burn, measured
    let u = env.user.insecure_clone();
    let uacc = env.token_account(&u.pubkey());
    let r = ok(env.mint_units(&u, &uacc, 50 * SOL, 0));
    cu.push(("mint", r.compute_units_consumed));
    let r = ok(env.burn_units(&u, &uacc, &u.pubkey(), env.balance(&uacc) / 2, 1));
    cu.push(("burn", r.compute_units_consumed));
    env.check_books();

    let r = ok(env.redeem(&holder, &hacc, Destination::Eth([0x42; 20])));
    cu.push(("redeem", r.compute_units_consumed));
    assert_eq!(env.wrapper().open_tickets, 1);
    env.check_ledger();
    let round = env.ticket(0).round;
    env.reach_round(round); // prove_round refuses a round that is not live yet (audit A2-02)
    ok(env.prove(round, [9u8; 32]));
    env.advance_time(60);
    let r = ok(env.resolve(0));
    cu.push(("resolve", r.compute_units_consumed));
    assert_eq!(env.ticket(0).stage, Stage::Resolved);
    assert_eq!(env.ticket(0).item_id, 0);

    // the canister sends the NFT on Ethereum, notes it mined, then records it final
    let c = env.custodian.insecure_clone();
    ok(env.custodian_ticket(rook_core::instruction::NoteSent { tx: tx_hash(1) }, 0, &c));
    let r = ok(env.record_delivery(0, tx_hash(1)));
    cu.push(("record_delivery", r.compute_units_consumed));
    let w = env.wrapper();
    assert_eq!((w.held, w.open_tickets), (0, 0));
    env.check_ledger();
    env.check_books();

    eprintln!("\nCompute units (LiteSVM, SBPF v2 build):");
    for (n, c) in &cu {
        eprintln!("  {:<16} {:>7}", n, c);
        assert!(*c < 200_000, "{} over the default per-instruction budget", n);
    }
}

/// `resolve` with a full item set: the draw sets are bounded by MAX_HELD; measure the worst case we allow in a test
/// (64 items, all promoted at once).
#[test]
fn resolve_cost_with_many_items() {
    let mut env = Env::new();
    let o = env.owner.insecure_clone();
    ok(env.wrapper_admin(rook_core::instruction::InitMintLimit { limit: 64 }, &o));
    let h = Keypair::new();
    env.svm.airdrop(&h.pubkey(), 1_000 * SOL).unwrap();
    let acc = env.token_account(&h.pubkey());
    for i in 0..64 {
        ok(env.wrap_nft(i, tx_hash(i), &acc));
    }
    ok(env.redeem(&h, &acc, Destination::Burn));
    let r = ok(env.resolve(0));
    eprintln!("  resolve (64 pending promoted): {}", r.compute_units_consumed);
    assert!(r.compute_units_consumed < 200_000);
    env.check_ledger();
}

/// Account sizes and rent, for docs/v9/rook-core-as-built.md.
#[test]
fn account_sizes_and_rent() {
    use anchor_lang::Space;
    let env = Env::new();
    let rows: [(&str, usize); 16] = [
        ("Registry", Registry::INIT_SPACE),
        ("Listing", Listing::INIT_SPACE),
        ("Module", Module::INIT_SPACE),
        ("ModuleVault", ModuleVault::INIT_SPACE),
        ("Wrapper", Wrapper::INIT_SPACE),
        ("ItemSet", ItemSet::INIT_SPACE),
        ("Item", Item::INIT_SPACE),
        ("HeldMarker", HeldMarker::INIT_SPACE),
        ("SourceMarker", SourceMarker::INIT_SPACE),
        ("Ticket", Ticket::INIT_SPACE),
        ("DrandRound", DrandRound::INIT_SPACE),
        ("Treasury", Treasury::INIT_SPACE),
        ("Desk", Desk::INIT_SPACE),
        ("Request", Request::INIT_SPACE),
        ("WakeSlot", WakeSlot::INIT_SPACE),
        ("Spent", Spent::INIT_SPACE),
    ];
    for (n, s) in rows {
        let total = 8 + s;
        let rent = env.svm.minimum_balance_for_rent_exemption(total);
        eprintln!("  | `{}` | {} | {:.6} |", n, total, rent as f64 / 1e9);
        assert!(total <= 10_240);
    }
    eprintln!("  | SPL mint | 82 | {:.6} |", env.svm.minimum_balance_for_rent_exemption(82) as f64 / 1e9);
    eprintln!("  | SPL token account | 165 | {:.6} |", env.svm.minimum_balance_for_rent_exemption(165) as f64 / 1e9);
}
