//! Audit A3 (October 2026) proofs of concept for rook_backing, against the mainnet DBC 0.2.1 / DAMM v2 0.2.4 bytes.
//! Report: docs/v9/audit-2026-10/a3-backing.md. Each test is named after its finding (A3-NN) or the "checked and sound"
//! property it pins. They PASSED when the behaviour described in the report was present (they demonstrated, not regressed).
//! Fixes of 2026-10-10: A3-01, A3-02 (+A3-05), A3-03 and A3-06 are `fixed_*` regressions that expect the new behaviour.
//!
//! Run: cd solana && cargo test -p rook_backing --test audit_a3_backing -- --nocapture
//! (needs the SBPF v2 rook_backing build in target/deploy-sbpfv2 and tests/fixtures/mock_rook_core.so, as tests/backing.rs).

#[path = "../../../tests/harness/mod.rs"]
mod harness;

use anchor_lang::{
    prelude::*,
    solana_program::{instruction::Instruction, system_instruction},
    InstructionData, ToAccountMetas,
};
use anchor_spl::{associated_token::get_associated_token_address, token::spl_token};
use harness::*;
use rook_backing::{
    constants::{DBC_ID, NATIVE_MINT},
    dynamic_bonding_curve as dbc, Vault,
};
use solana_keypair::Keypair;
use solana_signer::Signer;

fn err_has(logs: &str, needle: &str) {
    assert!(logs.contains(needle), "expected `{needle}` in:\n{logs}");
}

fn ix_transfer_pool_creator(coin: &Coin, new_creator: &Pubkey) -> Instruction {
    Instruction {
        program_id: DBC_ID,
        accounts: dbc::client::accounts::TransferPoolCreator {
            virtual_pool: coin.pool,
            config: coin.config,
            creator: coin.creator.pubkey(),
            new_creator: *new_creator,
            event_authority: event_authority(&DBC_ID),
            program: DBC_ID,
        }
        .to_account_metas(None),
        data: dbc::client::args::TransferPoolCreator {}.data(),
    }
}

// ------------------------------------------------------------------------------------------------- A3-01
/// A3-01: `Vault.creator` was DBC's `pool.creator` at the moment anyone called `open_vault`, never refreshed, although DBC lets
/// the creator hand the role to any key (`transfer_pool_creator`). Fixed 2026-10-10 by deciding the semantics: the wake desk
/// credits whoever holds the creator role when the coin is BACKED (its graduation for Rook): `back` re-reads `pool.creator`
/// and stores it. `open_vault` still writes the holder at opening (overwritten at `back`; nothing reads it before
/// `graduated`). The role stays transferable on DBC until then (documented; the loss is bounded by one wake bond per coin).
#[test]
fn fixed_a3_01_vault_creator_is_the_role_holder_at_backing() {
    let mut env = Env::new();
    let col = env.setup_collection();
    let cfg = env.create_config(&col.claimer, 50);
    let ix = env.ix_register_config(&col, &col.admin.pubkey(), &cfg);
    env.send_ok("register", &[ix], &[&col.admin]);

    // the vault is opened first (holder at opening: the launcher), then the launcher hands the role to `heir`
    let coin = env.launch(&cfg);
    let ix = env.ix_open_vault(&col, &coin.pool, &coin.mint);
    env.send_ok("open_vault", &[ix], &[]);
    let v: Vault = env.anchor(&vault_pda(&coin.mint));
    assert_eq!(v.creator, coin.creator.pubkey(), "holder at opening");
    let heir = Keypair::new().pubkey();
    let ix = ix_transfer_pool_creator(&coin, &heir);
    env.send_ok("dbc.transfer_pool_creator (pre-bonding)", &[ix], &[&coin.creator]);
    // graduation and backing
    env.warp(700);
    let whale = env.funded(200);
    env.buy(&coin, &whale, 150 * LAMPORTS, 1).map_err(|e| e.meta.logs.join("\n")).unwrap();
    let anyone = env.funded(5);
    env.migrate(&coin, &anyone);
    let ix = env.ix_back(&col, &coin, &env.payer.pubkey(), 0);
    env.send_ok("back", &[ix], &[]);
    let v: Vault = env.anchor(&vault_pda(&coin.mint));
    assert!(v.graduated);
    assert_eq!(v.creator, heir, "Vault.creator = the creator role holder at backing");
    let view = rook_layouts::backing_vault::parse(&env.raw(&vault_pda(&coin.mint))).unwrap();
    assert_eq!(view.creator, heir.to_bytes(), "rook_core's wake reclaim reads the holder at backing");
}

// ------------------------------------------------------------------------------------------------- A3-02
/// A3-02 / A3-05: `register_partner_config` accepted configs that break the invariants the design relies on (unlocked or
/// vesting LP, a migrated pool collecting base-token or compounding fees, a pool-creation fee, a foreign leftover receiver),
/// and did not check the config version whose layout it reads. Fixed 2026-10-10: each is refused with its own error; the
/// launch's config (the harness's `rook_config_params(50)`, = ops/solana/launch-params.mjs) registers.
#[test]
fn fixed_a3_02_register_partner_config_enforces_the_rook_invariants() {
    let mut env = Env::new();
    let col = env.setup_collection();
    let stranger = Keypair::new().pubkey();
    let try_cfg = |env: &mut Env, p: rook_backing::dynamic_bonding_curve::types::ConfigParameters, leftover: &Pubkey| {
        let cfg = Keypair::new();
        let ix = ix_create_config(&env.payer.pubkey(), &cfg.pubkey(), &col.claimer, leftover, p);
        env.send_ok("dbc.create_config", &[ix], &[&cfg]);
        cfg.pubkey()
    };
    let refused = |env: &mut Env, cfg: &Pubkey, err: &str| {
        let ix = env.ix_register_config(&col, &col.admin.pubkey(), cfg);
        err_has(&env.send_err(err, &[ix], &[&col.admin]), err);
    };

    let mut p = rook_config_params(50);
    p.partner_liquidity_percentage = 50; // unlocked, claimable LP for the partner
    p.partner_permanent_locked_liquidity_percentage = 0;
    let c = try_cfg(&mut env, p, &col.claimer);
    refused(&mut env, &c, "UnlockedLiquidity");
    let mut p = rook_config_params(50);
    p.creator_liquidity_percentage = 50; // the creator could pull liquidity
    p.creator_permanent_locked_liquidity_percentage = 0;
    let c = try_cfg(&mut env, p, &col.claimer);
    refused(&mut env, &c, "UnlockedLiquidity");
    let mut p = rook_config_params(50);
    p.migrated_pool_fee.collect_fee_mode = 1; // DAMM v2 BothToken
    let c = try_cfg(&mut env, p, &col.claimer);
    refused(&mut env, &c, "WrongMigratedFeeMode");
    let mut p = rook_config_params(50);
    p.pool_creation_fee = 100_000_000; // 0.1 SOL per launch, claimable only by the claimer
    let c = try_cfg(&mut env, p, &col.claimer);
    refused(&mut env, &c, "PoolCreationFee");
    let c = try_cfg(&mut env, rook_config_params(50), &stranger);
    refused(&mut env, &c, "WrongLeftoverReceiver");

    // the launch's config registers; DBC 0.2.1 writes version 0 at byte 236
    let good = try_cfg(&mut env, rook_config_params(50), &col.claimer);
    assert_eq!(env.raw(&good)[236], rook_backing::dbc::DBC_CONFIG_VERSION);
    // the same bytes with a vesting flag set, or another version byte, are refused (the program reads DBC's own account,
    // so these are written over a copy owned by DBC at another address)
    for (at, v, err) in [(184usize, 1u8, "UnlockedLiquidity"), (200, 1, "UnlockedLiquidity"), (236, 1, "WrongConfigVersion")] {
        let mut acc = env.svm.get_account(&good).unwrap();
        acc.data[at] = v;
        let k = Keypair::new().pubkey();
        env.svm.set_account(k, acc).unwrap();
        refused(&mut env, &k, err);
    }
    let ix = env.ix_register_config(&col, &col.admin.pubkey(), &good);
    env.send_ok("register_partner_config (the launch's shape)", &[ix], &[&col.admin]);
    let partner: rook_backing::Partner = env.anchor(&col.partner);
    assert_eq!(partner.configs, vec![good]);
}

// ------------------------------------------------------------------------------------------------- A3-03
/// A3-03: `register_partner` stored the rook_core accounts unchecked. Fixed 2026-10-10: each must be rook_core's (owner and
/// discriminator) and linked as rook_core wrote them (`module.listing`, `module.vault`, `module.wrapper`, `wrapper.listing`,
/// `wrapper.wx_mint`).
#[test]
fn fixed_a3_03_register_partner_checks_the_rook_core_accounts() {
    let mut env = Env::new();
    let _ = env.setup_collection(); // settings, the registry and its converter
    let admin = env.rook_owner.insecure_clone();
    let (l1, m1, mv1, w1, x1) = env.mock_core_accounts(|e, w| e.create_mint(w, 9));
    let (l2, m2, mv2, w2, x2) = env.mock_core_accounts(|e, w| e.create_mint(w, 9));
    let base = rook_backing::PartnerParams { listing: l1, registry: registry_pda(), module: m1, module_vault: mv1, wrapper: w1, wx_mint: x1 };
    let mut bad: Vec<(rook_backing::PartnerParams, &str)> = Vec::new();
    for (i, other) in [m2, mv2, w2, x2].into_iter().enumerate() {
        let mut p = base.clone();
        match i {
            0 => p.module = other,
            1 => p.module_vault = other,
            2 => p.wrapper = other,
            _ => p.wx_mint = other,
        }
        bad.push((p, "WrongCoreAccount"));
    }
    let mut p = base.clone();
    p.listing = l2; // another listing's PDA with this listing's module
    bad.push((p, "WrongCoreAccount"));
    let mut p = base.clone();
    p.listing = Keypair::new().pubkey(); // not a rook_core account at all
    bad.push((p, "WrongCoreAccount"));
    for (p, err) in bad {
        let ix = env.ix_register_partner(&admin.pubkey(), &registry_pda(), p);
        err_has(&env.send_err(err, &[ix], &[&admin]), err);
    }
    let ix = env.ix_register_partner(&admin.pubkey(), &registry_pda(), base);
    let r = env.send_ok("register_partner (checked)", &[ix], &[&admin]);
    println!("register_partner with the A3-03 checks: {} CU", r.compute_units_consumed);
    let _ = (l2, w2);
}

// ------------------------------------------------------------------------------------------------- A3-06
/// A3-06 / A1-08: the cranker of `back` passed any `min_units`, so it could pick the moment the keeper's target ran ahead of
/// the accepted floor. Fixed 2026-10-10: `back` reads the module's accepted floor and requires at least 90 % of the units the
/// fee buys at `accepted × (1 + conv_bps)`, whatever the caller passes. Here the mock converts at 2 SOL per unit: with the
/// module's accepted floor at 1.7 SOL the conversion is more than 11 % dearer than the reference and is refused even with
/// `min_units = 0`; at 1.85 SOL it is within the tolerance and goes through.
#[test]
fn fixed_a3_06_back_bounds_the_price_by_the_accepted_floor() {
    let mut env = Env::new();
    let col = env.setup_collection();
    let (coin, _m) = env.graduated_coin(&col, 50);
    env.mock_set_accepted(&col.module, 1_700_000_000);
    let ix = env.ix_back(&col, &coin, &env.payer.pubkey(), 0);
    err_has(&env.send_err("back above the bound", &[ix], &[]), "below min units");
    assert!(!env.anchor::<Vault>(&vault_pda(&coin.mint)).graduated, "the fee stays in DBC");
    env.mock_set_accepted(&col.module, 0);
    let ix = env.ix_back(&col, &coin, &env.payer.pubkey(), 0);
    err_has(&env.send_err("back with no floor", &[ix], &[]), "NoFloor");
    env.mock_set_accepted(&col.module, 1_850_000_000);
    let ix = env.ix_back(&col, &coin, &env.payer.pubkey(), 0);
    env.send_ok("back within the bound", &[ix], &[]);
    let v: Vault = env.anchor(&vault_pda(&coin.mint));
    assert_eq!(v.wx_units, v.lamports_backed / 2);
    assert!(v.wx_units >= rook_backing::units_floor(v.lamports_backed, 1_850_000_000, 0));
}

// ------------------------------------------------------------------------------------------------- A3-03
/// A3-03: DBC pays the partner's surplus share (`partner_withdraw_surplus`) and the partner pool-creation fee only to a
/// signature of `config.fee_claimer` = the claimer PDA; rook_backing exposes no instruction that signs either, so whatever
/// accrues there is stranded. Shown: after graduation the pool's `is_partner_withdraw_surplus` stays 0 and a non-claimer
/// signer is refused by DBC. Measured: with DBC 0.2.1 swaps cannot cross `migration_sqrt_price`, so the surplus of the Rook
/// curve is 0 (PartialFill fill); the finding is informational unless a config sets a pool-creation fee (A3-02).
#[test]
fn a3_03_partner_surplus_has_no_claim_path() {
    let mut env = Env::new();
    let col = env.setup_collection();
    let (coin, _m) = env.graduated_coin(&col, 50);
    let d = env.raw(&coin.pool);
    let quote_reserve = u64::from_le_bytes(d[240..248].try_into().unwrap());
    println!("A3-03: quote_reserve {quote_reserve} vs threshold {THRESHOLD}: total surplus {}", quote_reserve - THRESHOLD);
    assert_eq!(d[306], 0, "is_partner_withdraw_surplus: never withdrawn");
    let thief = env.funded(1);
    let wsol = env.wsol(&thief, 1_000);
    let ix = Instruction {
        program_id: DBC_ID,
        accounts: dbc::client::accounts::PartnerWithdrawSurplus {
            pool_authority: rook_backing::constants::DBC_POOL_AUTHORITY,
            config: coin.config,
            virtual_pool: coin.pool,
            token_quote_account: wsol,
            quote_vault: coin.quote_vault,
            quote_mint: NATIVE_MINT,
            fee_claimer: thief.pubkey(),
            token_quote_program: spl_token::ID,
            event_authority: event_authority(&DBC_ID),
            program: DBC_ID,
        }
        .to_account_metas(None),
        data: dbc::client::args::PartnerWithdrawSurplus {}.data(),
    };
    env.send_err("surplus by non-claimer", &[ix], &[&thief]);
}

// ------------------------------------------------------------------------------------------------- sound: look-alike config
/// Checked sound: anyone may create a DBC config whose fee_claimer is Rook's claimer PDA (DBC does not restrict it), but a
/// pool under such an UNREGISTERED look-alike config cannot get a vault, so it can never be backed or swept.
#[test]
fn sound_lookalike_config_with_rooks_claimer_is_refused() {
    let mut env = Env::new();
    let col = env.setup_collection();
    let mut p = rook_config_params(50);
    p.migration_fee.creator_fee_percentage = 0;
    let fake = env.create_config_with(&col.claimer, p); // attacker-created, fee_claimer = Rook's claimer
    let coin = env.launch(&fake);
    let ix = env.ix_open_vault(&col, &coin.pool, &coin.mint);
    err_has(&env.send_err("look-alike config", &[ix], &[]), "NotRookPool");
}

// ------------------------------------------------------------------------------------------------- sound: pre-funded PDAs
/// Checked sound (A3-08, informational): lamports sent ahead of time to the transient ["claimer_wsol", listing] PDA or to the
/// claimer do not block `back`; the donation is wrapped and converted into this coin's backing, and the claimer keeps a
/// pre-existing balance. The cranker is refunded the account rent it did not pay.
#[test]
fn sound_prefunded_claimer_pdas_do_not_block_back() {
    let mut env = Env::new();
    let col = env.setup_collection();
    let (coin, _m) = env.graduated_coin(&col, 50);
    let donor = env.funded(10);
    let wsol_pda = claimer_wsol_pda(&col.listing);
    let ixs = [
        system_instruction::transfer(&donor.pubkey(), &wsol_pda, 3 * LAMPORTS),
        system_instruction::transfer(&donor.pubkey(), &col.claimer, LAMPORTS),
    ];
    env.send_ok("donate", &ixs, &[&donor]);
    let cranker = env.funded(1);
    let c0 = env.lamports(&cranker.pubkey());
    let ix = env.ix_back(&col, &coin, &cranker.pubkey(), 0);
    env.send_ok("back with pre-funded PDAs", &[ix], &[&cranker]);
    let v: Vault = env.anchor(&vault_pda(&coin.mint));
    assert!(v.graduated);
    let rent = env.svm.minimum_balance_for_rent_exemption(165);
    assert_eq!(v.lamports_backed, THRESHOLD / 2 + 3 * LAMPORTS - rent, "the donation (less rent) is backed too");
    assert_eq!(env.lamports(&col.claimer), LAMPORTS, "pre-existing claimer balance untouched (and unrecoverable)");
    assert_eq!(env.lamports(&cranker.pubkey()), c0 + rent, "cranker receives rent it never paid");
    assert!(!env.exists(&wsol_pda));
}

// ------------------------------------------------------------------------------------------------- sound: redeem
/// Checked sound: a holder cannot redeem from another coin's vault, cannot redeem someone else's coins, and full-supply
/// redemption math holds against the real mint (live supply decreases by exactly the burn).
#[test]
fn sound_redeem_binds_vault_mint_and_holder() {
    let mut env = Env::new();
    let col = env.setup_collection();
    let (coin, m) = env.graduated_coin(&col, 50);
    let ix = env.ix_back(&col, &coin, &env.payer.pubkey(), 0);
    env.send_ok("back", &[ix], &[]);
    let holder = env.funded(30);
    env.damm_buy(&coin, &m, &holder, 5 * LAMPORTS);
    let holder_coin = get_associated_token_address(&holder.pubkey(), &coin.mint);
    let holder_wx = env.ata(&holder.pubkey(), &col.wx_mint);
    let bal = env.token_amount(&holder_coin);
    assert!(bal > 0);

    // someone else's coins: refused (token::authority = holder)
    let thief = env.funded(1);
    let thief_wx = env.ata(&thief.pubkey(), &col.wx_mint);
    let ix = env.ix_redeem(&coin.mint, &col.wx_mint, &thief.pubkey(), &holder_coin, &thief_wx, bal);
    env.send_err("redeem another holder's coins", &[ix], &[&thief]);

    // a second graduated coin: redeeming coin-1 tokens against coin-2's vault is refused (mint pinned by the vault)
    let (coin2, _m2) = env.graduated_coin(&col, 50);
    let ix = env.ix_back(&col, &coin2, &env.payer.pubkey(), 0);
    env.send_ok("back 2", &[ix], &[]);
    let mut ix = env.ix_redeem(&coin.mint, &col.wx_mint, &holder.pubkey(), &holder_coin, &holder_wx, bal);
    ix.accounts[1].pubkey = vault_pda(&coin2.mint); // vault of coin 2
    ix.accounts[4].pubkey = vault_wx_pda(&coin2.mint);
    env.send_err("cross-vault redeem", &[ix], &[&holder]);

    let s0 = env.mint_supply(&coin.mint);
    let w0 = env.token_amount(&vault_wx_pda(&coin.mint));
    let ix = env.ix_redeem(&coin.mint, &col.wx_mint, &holder.pubkey(), &holder_coin, &holder_wx, bal);
    env.send_ok("redeem", &[ix], &[&holder]);
    assert_eq!(env.mint_supply(&coin.mint), s0 - bal);
    let paid = env.token_amount(&holder_wx);
    assert_eq!(paid as u128, (w0 as u128 * bal as u128) / s0 as u128);
    // the mint authority was revoked by DBC at launch: nobody can inflate the supply against the vault
    let mint = env.raw(&coin.mint);
    assert_eq!(&mint[0..4], &[0, 0, 0, 0], "mint_authority: None (COption tag 0)");
    assert_eq!(&mint[46..50], &[0, 0, 0, 0], "freeze_authority: None");
}
