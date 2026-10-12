//! Spike Q1 — a Rook partner config on Meteora DBC (mainnet program bytes, LiteSVM): launch, buy to graduation, migrate to
//! DAMM v2, `rook_backing::back` CPIs `withdraw_migration_fee` signed by the claimer PDA, partner fee sweeps.
//! Run: `cargo test -p rook_backing --test q1_meteora_dbc -- --nocapture` (after `anchor build`).
//! Write-up: docs/v9/spikes/q1-meteora-dbc.md.

#[path = "harness/mod.rs"]
mod harness;

use anchor_lang::prelude::*;
use anchor_spl::associated_token::get_associated_token_address;
use harness::*;
use rook_backing::{dbc as dbcread, Vault};
use solana_signer::Signer;

#[test]
fn q1_full_flow() {
    let mut env = Env::new();
    let rook_core_is_mock = rook_core_so().1;
    let col = env.setup_collection();

    // a. the partner config: fee_claimer = rook_backing PDA ["claimer", listing] (off-curve; DBC takes it as an
    //    UncheckedAccount), leftover receiver = the same PDA.
    assert!(!col.claimer.is_on_curve(), "the claimer is a PDA");
    let config = env.create_config(&col.claimer, 50);
    let cfg = config_of(&env, &config);
    assert_eq!(cfg.fee_claimer, col.claimer);
    assert_eq!(cfg.migration_fee_percentage, 50);
    assert_eq!(cfg.migration_quote_threshold, THRESHOLD);
    let ix = env.ix_register_config(&col, &col.admin.pubkey(), &config);
    env.send_ok("register_partner_config", &[ix], &[&col.admin]);

    // b. launch, anti-sniper fee at t0, buy to the threshold, migrate (permissionless).
    let coin = env.launch(&config);
    let ix = env.ix_open_vault(&col, &coin.pool, &coin.mint);
    env.send_ok("open_vault", &[ix], &[]);

    let sniper = env.funded(10);
    let r = env.buy(&coin, &sniper, LAMPORTS, 0).expect("snipe");
    env.cu.insert("dbc.swap2 (first buy)".into(), r.compute_units_consumed - 150);
    let fee_t0 = pool_fees_quote(&env, &coin.pool);
    println!("anti-sniper: a 1 SOL buy at launch paid {} lamports of fees ({:.2} %)", fee_t0, fee_t0 as f64 / 1e7);

    let back_early = env.ix_back(&col, &coin, &env.payer.pubkey(), 0);
    let logs = env.send_err("back before migration", &[back_early], &[]);
    assert!(logs.contains("NotMigrated"), "{logs}");

    env.warp(700); // past the 10-minute fee schedule
    let fees_before = pool_fees_quote(&env, &coin.pool);
    let whale = env.funded(200);
    let r = env.buy(&coin, &whale, 150 * LAMPORTS, 1).map_err(|e| e.meta.logs.join("\n")).unwrap();
    env.cu.insert("dbc.swap2 (partial fill to threshold)".into(), r.compute_units_consumed - 150);
    let p = pool_of(&env, &coin.pool);
    println!(
        "after the fill: quote_reserve {} (threshold {}), migration_progress {}, fees on the 2nd buy {} lamports",
        p.quote_reserve,
        THRESHOLD,
        p.migration_progress,
        pool_fees_quote(&env, &coin.pool) - fees_before
    );
    assert!(p.quote_reserve >= THRESHOLD);
    assert_eq!(p.migration_progress, 2, "LockedVesting: ready to migrate (no locker)");

    let anyone = env.funded(5);
    let m = env.migrate(&coin, &anyone);
    let p = pool_of(&env, &coin.pool);
    assert_eq!(p.is_migrated, 1);
    let supply_after = env.mint_supply(&coin.mint);
    println!("migrated: DAMM v2 pool {}, coin supply {}", m.damm_pool, supply_after);
    // LP: both positions permanently locked; owners = claimer (partner) and creator
    let o1 = env.token_owner(&damm_position_nft_account(&m.nft1));
    let o2 = env.token_owner(&damm_position_nft_account(&m.nft2));
    println!("position NFT owners: first {o1}, second {o2} (claimer {}, creator {})", col.claimer, coin.creator.pubkey());
    let (partner_nft, _creator_nft) = if o1 == col.claimer { (m.nft1, m.nft2) } else { (m.nft2, m.nft1) };
    assert!(o1 == col.claimer || o2 == col.claimer);
    let creator_nft = if partner_nft == m.nft1 { m.nft2 } else { m.nft1 };
    for (who, nft) in [("partner (claimer)", partner_nft), ("creator", creator_nft)] {
        let (u, v, p) = position_liquidity(&env, &damm_position(&nft));
        println!("DAMM v2 position of the {who}: unlocked {u}, vested {v}, permanently locked {p}");
        assert!(u <= 1 && v == 0, "all LP is permanently locked (1 unit of rounding may stay unlocked)");
    }
    // the creator cannot pull liquidity out of its locked position
    let creator_wsol = env.wsol(&coin.creator, 1_000_000);
    let creator_coin = env.ata(&coin.creator.pubkey(), &coin.mint);
    let ix = damm_ix(&coin, &m, &creator_nft, &coin.creator.pubkey(), creator_coin, creator_wsol, true);
    let logs = env.send_err("creator remove_all_liquidity", &[ix], &[&coin.creator]);
    println!("creator remove_all_liquidity refused: {}", logs.lines().find(|l| l.contains("Error")).unwrap_or("?"));

    // c. back: the claimer PDA signs withdraw_migration_fee inside rook_backing, then rook_core convert.
    let claimer_before = env.lamports(&col.claimer);
    let module_before = env.lamports(&col.module_vault);
    let cranker = env.funded(1);
    let cranker_before = env.lamports(&cranker.pubkey());
    let ix = env.ix_back(&col, &coin, &cranker.pubkey(), 0);
    let r = env.send_ok("rook_backing.back", &[ix], &[&cranker]);
    let fee = env.lamports(&col.module_vault) - module_before;
    let vault: Vault = env.anchor(&vault_pda(&coin.mint));
    println!(
        "back: {} lamports migration fee → module vault; vault wX units {}; claimer {}→{}; cranker paid {} (tx fee {})",
        fee,
        vault.wx_units,
        claimer_before,
        env.lamports(&col.claimer),
        cranker_before - env.lamports(&cranker.pubkey()),
        r.fee
    );
    assert_eq!(fee, THRESHOLD / 2, "50 % of the 85 SOL threshold");
    assert_eq!(vault.lamports_backed, fee);
    assert!(vault.graduated);
    if rook_core_is_mock {
        assert_eq!(vault.wx_units, fee / 2);
    }
    assert_eq!(env.token_amount(&vault_wx_pda(&coin.mint)), vault.wx_units);
    assert_eq!(env.lamports(&cranker.pubkey()), cranker_before, "the cranker gets the transient wSOL rent back (tx fee is paid by the env payer)");
    for l in r.logs.iter().filter(|l| l.contains("consumed")) {
        println!("  {l}");
    }

    // second back refuses
    let ix = env.ix_back(&col, &coin, &cranker.pubkey(), 0);
    let logs = env.send_err("back twice", &[ix], &[&cranker]);
    assert!(logs.contains("AlreadyBacked"), "{logs}");

    // partner trading fees on the curve → treasury
    env.ata(&col.claimer, &coin.mint);
    let t_before = env.lamports(&treasury_pda());
    let ix = env.ix_sweep_partner_fees(&col, &coin, &cranker.pubkey(), &treasury_pda());
    env.send_ok("rook_backing.sweep_partner_fees", &[ix], &[&cranker]);
    let swept = env.lamports(&treasury_pda()) - t_before;
    println!("sweep_partner_fees: {} lamports → treasury", swept);
    assert!(swept > 0);

    // post-graduation: trade on DAMM v2, sweep the partner position's fees
    let trader = env.funded(20);
    env.damm_buy(&coin, &m, &trader, 10 * LAMPORTS);
    let t_before = env.lamports(&treasury_pda());
    let ix = env.ix_sweep_lp_fees(&col, &coin, &m, &partner_nft, &cranker.pubkey(), &treasury_pda());
    env.send_ok("rook_backing.sweep_lp_fees", &[ix], &[&cranker]);
    let lp_swept = env.lamports(&treasury_pda()) - t_before;
    println!("sweep_lp_fees: a 10 SOL buy on DAMM v2 paid the partner position {} lamports → treasury", lp_swept);
    assert!(lp_swept > 0);
    let before = env.token_amount(&creator_wsol);
    let ix = damm_ix(&coin, &m, &creator_nft, &coin.creator.pubkey(), creator_coin, creator_wsol, false);
    env.send_ok("damm_v2.claim_position_fee (creator)", &[ix], &[&coin.creator]);
    println!("creator claim_position_fee: {} lamports (wSOL)", env.token_amount(&creator_wsol) - before);

    // redeem: a holder burns half their coins
    let holder_coin = get_associated_token_address(&whale.pubkey(), &coin.mint);
    let held = env.token_amount(&holder_coin);
    let holder_wx = env.ata(&whale.pubkey(), &col.wx_mint);
    let ix = env.ix_redeem(&coin.mint, &col.wx_mint, &whale.pubkey(), &holder_coin, &holder_wx, held / 2);
    env.send_ok("rook_backing.redeem_backing", &[ix], &[&whale]);
    println!("redeem: burned {} coins for {} wX base units", held / 2, env.token_amount(&holder_wx));

    println!("\nCU (excluding the compute-budget instruction):");
    let mut v: Vec<_> = env.cu.iter().collect();
    v.sort();
    for (k, c) in v {
        println!("  {k:48} {c:>9}");
    }
    println!("rook_core: {}", if rook_core_is_mock { "MOCK (tests/mock_rook_core)" } else { "real" });
}

fn config_of(env: &Env, key: &Pubkey) -> dbcread::DbcConfig {
    let a = env.svm.get_account(key).unwrap();
    dbcread::parse_config(&a.owner, &a.data).unwrap()
}
fn pool_of(env: &Env, key: &Pubkey) -> dbcread::DbcPool {
    let a = env.svm.get_account(key).unwrap();
    dbcread::parse_pool(&a.owner, &a.data).unwrap()
}

/// (unlocked, vested, permanently locked) liquidity of a DAMM v2 position (offsets of cp_amm 0.2.4 `Position`).
fn position_liquidity(env: &Env, position: &Pubkey) -> (u128, u128, u128) {
    let d = env.raw(position);
    let r = |o: usize| u128::from_le_bytes(d[o..o + 16].try_into().unwrap());
    (r(152), r(168), r(184))
}

/// DAMM v2 claim_position_fee or remove_all_liquidity, signed by the position owner.
fn damm_ix(coin: &Coin, m: &Migrated, nft: &Pubkey, owner: &Pubkey, a_acc: Pubkey, b_acc: Pubkey, remove: bool) -> anchor_lang::solana_program::instruction::Instruction {
    use anchor_lang::{InstructionData, ToAccountMetas};
    use rook_backing::{constants::*, cp_amm};
    let spl = anchor_spl::token::ID;
    let accounts = cp_amm::client::accounts::ClaimPositionFee {
        pool_authority: DAMM_V2_POOL_AUTHORITY,
        pool: m.damm_pool,
        position: damm_position(nft),
        token_a_account: a_acc,
        token_b_account: b_acc,
        token_a_vault: damm_vault(&coin.mint, &m.damm_pool),
        token_b_vault: damm_vault(&NATIVE_MINT, &m.damm_pool),
        token_a_mint: coin.mint,
        token_b_mint: NATIVE_MINT,
        position_nft_account: damm_position_nft_account(nft),
        signer: *owner,
        token_a_program: spl,
        token_b_program: spl,
        event_authority: event_authority(&DAMM_V2_ID),
        program: DAMM_V2_ID,
    };
    let mut metas = accounts.to_account_metas(None);
    let data = if remove {
        // same account list as claim_position_fee, but pool is writable
        metas[1].is_writable = true;
        cp_amm::client::args::RemoveAllLiquidity { token_a_amount_threshold: 0, token_b_amount_threshold: 0 }.data()
    } else {
        cp_amm::client::args::ClaimPositionFee {}.data()
    };
    anchor_lang::solana_program::instruction::Instruction { program_id: DAMM_V2_ID, accounts: metas, data }
}

/// The protocol + partner quote fees the pool has accrued (DBC VirtualPool: protocol_quote_fee @256, partner_quote_fee
/// @272).
fn pool_fees_quote(env: &Env, pool: &Pubkey) -> u64 {
    let d = env.raw(pool);
    let r = |o: usize| u64::from_le_bytes(d[o..o + 8].try_into().unwrap());
    r(256) + r(272)
}

