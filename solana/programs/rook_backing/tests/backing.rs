//! rook_backing against the mainnet DBC / DAMM v2 bytes in LiteSVM (harness: solana/tests/harness).
//! Needs `anchor build` and the SBPF v2 test binary (see harness::rook_backing_so) and tests/fixtures/mock_rook_core.so.

#[path = "../../../tests/harness/mod.rs"]
mod harness;

use anchor_lang::prelude::*;
use anchor_spl::associated_token::get_associated_token_address;
use harness::*;
use rook_backing::{
    constants::{DBC_ID, ROOK_CORE_ID},
    instructions::redeem::payout,
    Partner, Vault,
};
use solana_keypair::Keypair;
use solana_signer::Signer;

fn err_has(logs: &str, needle: &str) {
    assert!(logs.contains(needle), "expected `{needle}` in:\n{logs}");
}

// ------------------------------------------------------------------------------------------------- admin

#[test]
fn init_backing_only_by_upgrade_authority() {
    let mut env = Env::new();
    let upgrade = env.funded(1);
    env.set_upgrade_authority(&upgrade.pubkey());
    let stranger = env.funded(1);
    let ix = env.ix_init_backing(&stranger.pubkey());
    err_has(&env.send_err("init by stranger", &[ix], &[&stranger]), "Unauthorized");
    let ix = env.ix_init_backing(&upgrade.pubkey());
    env.send_ok("init", &[ix], &[&upgrade]);
    let s: rook_backing::Settings = env.anchor(&settings_pda());
    assert_eq!(s.registry, registry_pda(), "pins rook_core's registry PDA");
    assert_eq!(s.treasury, treasury_pda());
    let ix = env.ix_init_backing(&upgrade.pubkey());
    env.send_err("init twice", &[ix], &[&upgrade]);
}

// The admin is rook_core's Registry.owner, read from the registry account on every call: the owner succeeds, anyone else is
// refused, an ownership change carries over at once, and a look-alike registry is refused.
#[test]
fn the_admin_is_the_registry_owner() {
    let mut env = Env::new();
    let col = env.setup_collection(); // registered by the registry owner
    let good = env.create_config(&col.claimer, 50);
    let stranger = env.funded(1);
    let ix = env.ix_register_config(&col, &stranger.pubkey(), &good);
    err_has(&env.send_err("stranger", &[ix], &[&stranger]), "Unauthorized");
    // a fresh collection's rook_core accounts (register_partner checks them, audit A3-03)
    let params = |env: &mut Env| {
        let (listing, module, module_vault, wrapper, wx_mint) = env.mock_core_accounts(|e, w| e.create_mint(w, 9));
        rook_backing::PartnerParams { listing, registry: registry_pda(), module, module_vault, wrapper, wx_mint }
    };
    let pp = params(&mut env);
    let ix = env.ix_register_partner(&stranger.pubkey(), &registry_pda(), pp);
    err_has(&env.send_err("stranger registers a collection", &[ix], &[&stranger]), "Unauthorized");

    // ownership moves in rook_core (here: the mock registry's owner field rewritten)
    let new_owner = env.funded(5);
    env.mock_registry_set_owner(&new_owner.pubkey());
    let ix = env.ix_register_config(&col, &col.admin.pubkey(), &good);
    err_has(&env.send_err("old owner", &[ix], &[&col.admin]), "Unauthorized");
    let ix = env.ix_register_config(&col, &new_owner.pubkey(), &good);
    env.send_ok("new owner", &[ix], &[&new_owner]);
    let pp = params(&mut env);
    let ix = env.ix_register_partner(&new_owner.pubkey(), &registry_pda(), pp);
    env.send_ok("new owner registers a collection", &[ix], &[&new_owner]);

    // a partner pinned to another registry is refused
    let mut p = params(&mut env);
    p.registry = Keypair::new().pubkey();
    let ix = env.ix_register_partner(&new_owner.pubkey(), &registry_pda(), p);
    err_has(&env.send_err("partner with a foreign registry", &[ix], &[&new_owner]), "WrongRegistry");

    // a look-alike registry: another address naming the caller as owner
    let fake = Keypair::new().pubkey();
    let mut data = env.raw(&registry_pda());
    data[8..40].copy_from_slice(stranger.pubkey().as_ref());
    let lamports = env.lamports(&registry_pda());
    env.svm
        .set_account(fake, solana_account::Account { lamports, data: data.clone(), owner: ROOK_CORE_ID, executable: false, rent_epoch: 0 })
        .unwrap();
    let pp = params(&mut env);
    let ix = env.ix_register_partner(&stranger.pubkey(), &fake, pp);
    err_has(&env.send_err("look-alike address", &[ix], &[&stranger]), "NotRegistry");
    // the right address, but not rook_core's account (owner program) or not a Registry (discriminator)
    let real = env.svm.get_account(&registry_pda()).unwrap();
    let mut forged = real.clone();
    forged.owner = Keypair::new().pubkey();
    forged.data[8..40].copy_from_slice(stranger.pubkey().as_ref());
    env.svm.set_account(registry_pda(), forged).unwrap();
    let pp = params(&mut env);
    let ix = env.ix_register_partner(&stranger.pubkey(), &registry_pda(), pp);
    err_has(&env.send_err("foreign-owned registry", &[ix], &[&stranger]), "NotRegistry");
    let mut forged = real.clone();
    forged.data[0] ^= 1;
    env.svm.set_account(registry_pda(), forged).unwrap();
    let pp = params(&mut env);
    let ix = env.ix_register_partner(&new_owner.pubkey(), &registry_pda(), pp);
    err_has(&env.send_err("not a Registry", &[ix], &[&new_owner]), "NotRegistry");
    env.svm.set_account(registry_pda(), real).unwrap();
}

#[test]
fn register_partner_config_checks_admin_and_config() {
    let mut env = Env::new();
    let col = env.setup_collection();
    // a config whose fee_claimer is not the claimer PDA
    let other = Keypair::new().pubkey();
    let foreign = env.create_config(&other, 50);
    let ix = env.ix_register_config(&col, &col.admin.pubkey(), &foreign);
    err_has(&env.send_err("foreign claimer", &[ix], &[&col.admin]), "WrongFeeClaimer");
    // a config with no partner migration fee
    let nofee = env.create_config(&col.claimer, 0);
    let ix = env.ix_register_config(&col, &col.admin.pubkey(), &nofee);
    err_has(&env.send_err("no migration fee", &[ix], &[&col.admin]), "NoMigrationFee");
    // slot activation: a pool's activation_point would be a slot, not its launch time
    let mut p = rook_config_params(50);
    p.activation_type = 0;
    let slots = env.create_config_with(&col.claimer, p);
    let ix = env.ix_register_config(&col, &col.admin.pubkey(), &slots);
    err_has(&env.send_err("slot activation", &[ix], &[&col.admin]), "WrongActivationType");
    // not a DBC account
    let ix = env.ix_register_config(&col, &col.admin.pubkey(), &col.partner);
    err_has(&env.send_err("not dbc", &[ix], &[&col.admin]), "NotDbcAccount");
    // a good config, by a stranger, then by the admin, then again
    let good = env.create_config(&col.claimer, 50);
    let stranger = env.funded(1);
    let ix = env.ix_register_config(&col, &stranger.pubkey(), &good);
    err_has(&env.send_err("stranger", &[ix], &[&stranger]), "Unauthorized");
    let ix = env.ix_register_config(&col, &col.admin.pubkey(), &good);
    env.send_ok("register", &[ix], &[&col.admin]);
    let ix = env.ix_register_config(&col, &col.admin.pubkey(), &good);
    err_has(&env.send_err("twice", &[ix], &[&col.admin]), "ConfigAlreadyRegistered");
    let p: Partner = env.anchor(&col.partner);
    assert_eq!(p.configs, vec![good]);
}

// ------------------------------------------------------------------------------------------------- open_vault

#[test]
fn open_vault_refuses_a_non_rook_pool() {
    let mut env = Env::new();
    let col = env.setup_collection();
    // somebody else's DBC config and coin (another partner, same program)
    let other_claimer = Keypair::new().pubkey();
    let foreign_cfg = env.create_config(&other_claimer, 50);
    let foreign = env.launch(&foreign_cfg);
    let ix = env.ix_open_vault(&col, &foreign.pool, &foreign.mint);
    err_has(&env.send_err("foreign pool", &[ix], &[]), "NotRookPool");

    // a Rook pool, but the wrong coin mint
    let cfg = env.create_config(&col.claimer, 50);
    let ix = env.ix_register_config(&col, &col.admin.pubkey(), &cfg);
    env.send_ok("register", &[ix], &[&col.admin]);
    let coin = env.launch(&cfg);
    let launched = env.now();
    env.warp(3_600); // the vault may be opened any time later: it records the pool's creation, not its own
    let ix = env.ix_open_vault(&col, &coin.pool, &foreign.mint);
    err_has(&env.send_err("wrong mint", &[ix], &[]), "WrongCoinMint");
    // a non-pool account
    let ix = env.ix_open_vault(&col, &cfg, &coin.mint);
    err_has(&env.send_err("config as pool", &[ix], &[]), "NotDbcPool");
    // the right one
    let ix = env.ix_open_vault(&col, &coin.pool, &coin.mint);
    env.send_ok("open", &[ix], &[]);
    let v: Vault = env.anchor(&vault_pda(&coin.mint));
    assert_eq!(v.dbc_pool, coin.pool);
    assert_eq!(v.creator, coin.creator.pubkey());
    assert!(!v.graduated);
    let pool = rook_backing::dbc::parse_pool(&DBC_ID, &env.raw(&coin.pool)).unwrap();
    assert_eq!(pool.activation_point, launched as u64, "DBC sets the activation point to the creation time");
    assert_eq!(v.launched_at, launched);
    // and rook_core reads it through the shared layout
    let view = rook_layouts::backing_vault::parse(&env.raw(&vault_pda(&coin.mint))).unwrap();
    assert_eq!((view.launched_at, view.creator, view.graduated), (launched, coin.creator.pubkey().to_bytes(), false));
    // once per coin
    let ix = env.ix_open_vault(&col, &coin.pool, &coin.mint);
    env.send_err("open twice", &[ix], &[]);
}

// ------------------------------------------------------------------------------------------------- back

#[test]
fn back_refuses_before_migration_then_backs_once() {
    let mut env = Env::new();
    let col = env.setup_collection();
    let cfg = env.create_config(&col.claimer, 50);
    let ix = env.ix_register_config(&col, &col.admin.pubkey(), &cfg);
    env.send_ok("register", &[ix], &[&col.admin]);
    let coin = env.launch(&cfg);
    let ix = env.ix_open_vault(&col, &coin.pool, &coin.mint);
    env.send_ok("open", &[ix], &[]);

    let ix = env.ix_back(&col, &coin, &env.payer.pubkey(), 0);
    err_has(&env.send_err("back at launch", &[ix], &[]), "NotMigrated");

    // curve complete but not migrated: DBC would pay the fee, rook_backing still refuses
    env.warp(700);
    let whale = env.funded(200);
    env.buy(&coin, &whale, 150 * LAMPORTS, 1).map_err(|e| e.meta.logs.join("\n")).unwrap();
    let ix = env.ix_back(&col, &coin, &env.payer.pubkey(), 0);
    err_has(&env.send_err("back before migration", &[ix], &[]), "NotMigrated");

    let anyone = env.funded(5);
    env.migrate(&coin, &anyone);

    // a substituted rook_core account is refused (pinned in Partner)
    let mut ix = env.ix_back(&col, &coin, &env.payer.pubkey(), 0);
    let module_idx = ix.accounts.iter().position(|a| a.pubkey == col.module).unwrap();
    ix.accounts[module_idx].pubkey = Keypair::new().pubkey();
    err_has(&env.send_err("fake module", &[ix], &[]), "ConstraintAddress");
    // a substituted converter (not the ["converter"] PDA pinned in Settings) is refused; so is the claimer in its place
    for fake in [Keypair::new().pubkey(), col.claimer] {
        let mut ix = env.ix_back(&col, &coin, &env.payer.pubkey(), 0);
        let i = ix.accounts.iter().position(|a| a.pubkey == converter_pda()).unwrap();
        ix.accounts[i].pubkey = fake;
        err_has(&env.send_err("fake converter", &[ix], &[]), "ConstraintAddress");
    }
    let s: rook_backing::Settings = env.anchor(&settings_pda());
    assert_eq!(s.converter, converter_pda());

    // slippage: more units than convert gives
    let ix = env.ix_back(&col, &coin, &env.payer.pubkey(), THRESHOLD);
    env.send_err("min_units too high", &[ix], &[]);

    let ix = env.ix_back(&col, &coin, &env.payer.pubkey(), THRESHOLD / 4);
    env.send_ok("back", &[ix], &[]);
    println!("back (mock convert): {} CU", env.cu["back"]);
    let v: Vault = env.anchor(&vault_pda(&coin.mint));
    assert!(v.graduated);
    assert_eq!(v.lamports_backed, THRESHOLD / 2);
    assert_eq!(v.wx_units, THRESHOLD / 4);
    assert_eq!(v.coin_supply_at_backing, TOTAL_SUPPLY);
    assert_eq!(env.lamports(&col.claimer), 0, "nothing stays in the claimer");
    assert!(!env.exists(&claimer_wsol_pda(&col.listing)), "transient wSOL account closed");

    // idempotent: the second call refuses cleanly, nothing moves
    let before = env.token_amount(&vault_wx_pda(&coin.mint));
    let ix = env.ix_back(&col, &coin, &env.payer.pubkey(), 0);
    err_has(&env.send_err("back twice", &[ix], &[]), "AlreadyBacked");
    assert_eq!(env.token_amount(&vault_wx_pda(&coin.mint)), before);
    let p: Partner = env.anchor(&col.partner);
    assert_eq!(p.lamports_backed, THRESHOLD / 2);
}

#[test]
fn back_with_a_different_backing_share() {
    // Q9 is open: the share is a per-config setting. 30 % here.
    let mut env = Env::new();
    let col = env.setup_collection();
    let (coin, _) = env.graduated_coin(&col, 30);
    let ix = env.ix_back(&col, &coin, &env.payer.pubkey(), 0);
    env.send_ok("back", &[ix], &[]);
    let v: Vault = env.anchor(&vault_pda(&coin.mint));
    assert_eq!(v.lamports_backed, THRESHOLD * 30 / 100);
    assert_eq!(v.coin_supply_at_backing, env.mint_supply(&coin.mint));
}

// ------------------------------------------------------------------------------------------------- redeem

#[test]
fn payout_never_overpays() {
    // exhaustive-ish: sequential redemptions by many holders; the vault never goes negative, every payout is the floor
    // of the pro-rata share, backing per remaining coin never falls, and burning the whole supply pays the whole vault.
    for &(vault, supply) in &[(21_250_000_000u64, TOTAL_SUPPLY), (7, 10), (1_000_000_007, 999_999_937), (u64::MAX / 3, u64::MAX / 2)] {
        let mut v = vault as u128;
        let mut s = supply as u128;
        let mut seed = 0x9e3779b97f4a7c15u64;
        while s > 0 {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            let amount = ((seed as u128) % s.min(1 << 60)).max(1) as u64;
            let p = payout(v as u64, amount, s as u64).unwrap() as u128;
            assert!(p * s <= v * amount as u128, "over-paid");
            assert!((p + 1) * s > v * amount as u128, "not the floor");
            let before = v * (s - amount as u128); // compare v'/s' >= v/s  ⇔  (v-p)*s >= v*(s-amount)
            v -= p;
            if s - (amount as u128) > 0 {
                assert!(v * s >= before, "backing per coin fell");
            }
            s -= amount as u128;
        }
        // the last redemption had amount == remaining supply ⇒ it took everything
        assert_eq!(v, 0, "the last redeemer takes the dust");
    }
    assert_eq!(payout(10, 10, 10).unwrap(), 10);
    assert!(payout(10, 0, 10).is_err());
}

#[test]
fn redeem_backing_rules() {
    let mut env = Env::new();
    let col = env.setup_collection();
    let (coin, m) = env.graduated_coin(&col, 50);

    // before backing: refuses
    let alice = env.funded(20);
    env.damm_buy(&coin, &m, &alice, 5 * LAMPORTS);
    let alice_coin = get_associated_token_address(&alice.pubkey(), &coin.mint);
    let alice_wx = env.ata(&alice.pubkey(), &col.wx_mint);
    let ix = env.ix_redeem(&coin.mint, &col.wx_mint, &alice.pubkey(), &alice_coin, &alice_wx, 1_000_000);
    err_has(&env.send_err("redeem before back", &[ix], &[&alice]), "NotBacked");

    let ix = env.ix_back(&col, &coin, &env.payer.pubkey(), 0);
    env.send_ok("back", &[ix], &[]);

    // zero amount
    let ix = env.ix_redeem(&coin.mint, &col.wx_mint, &alice.pubkey(), &alice_coin, &alice_wx, 0);
    err_has(&env.send_err("zero", &[ix], &[&alice]), "ZeroAmount");
    // too small to pay a single wX base unit: refused rather than burned for nothing
    let ix = env.ix_redeem(&coin.mint, &col.wx_mint, &alice.pubkey(), &alice_coin, &alice_wx, 1);
    err_has(&env.send_err("dust", &[ix], &[&alice]), "ZeroPayout");
    // wrong mint: another coin's mint against this vault
    let (other, _) = env.graduated_coin(&col, 50);
    let ix = env.ix_redeem(&coin.mint, &col.wx_mint, &alice.pubkey(), &alice_coin, &alice_wx, 1_000_000);
    let mut bad = ix.clone();
    bad.accounts[2].pubkey = other.mint;
    err_has(&env.send_err("wrong mint", &[bad], &[&alice]), "WrongCoinMint");
    // somebody else's coins: the holder must own (and sign for) the burned account
    let bob = env.funded(20);
    let bob_wx = env.ata(&bob.pubkey(), &col.wx_mint);
    let ix = env.ix_redeem(&coin.mint, &col.wx_mint, &bob.pubkey(), &alice_coin, &bob_wx, 1_000_000);
    err_has(&env.send_err("not the owner", &[ix], &[&bob]), "ConstraintTokenOwner");

    // the real thing: pro-rata, floor, live supply
    let held = env.token_amount(&alice_coin);
    let supply = env.mint_supply(&coin.mint);
    let vault_bal = env.token_amount(&vault_wx_pda(&coin.mint));
    let ix = env.ix_redeem(&coin.mint, &col.wx_mint, &alice.pubkey(), &alice_coin, &alice_wx, held);
    env.send_ok("redeem", &[ix], &[&alice]);
    let got = env.token_amount(&alice_wx);
    assert_eq!(got as u128, vault_bal as u128 * held as u128 / supply as u128);
    assert_eq!(env.mint_supply(&coin.mint), supply - held, "burned");
    assert_eq!(env.token_amount(&alice_coin), 0);
    let v: Vault = env.anchor(&vault_pda(&coin.mint));
    assert_eq!((v.coins_burned, v.wx_paid), (held, got));
    // per-coin backing did not fall
    let vault_after = env.token_amount(&vault_wx_pda(&coin.mint)) as u128;
    assert!(vault_after * supply as u128 >= vault_bal as u128 * (supply - held) as u128);
}

// ------------------------------------------------------------------------------------------------- sweeps

#[test]
fn sweeps_go_to_the_treasury_only() {
    let mut env = Env::new();
    let col = env.setup_collection();
    let (coin, m) = env.graduated_coin(&col, 50);
    env.ata(&col.claimer, &coin.mint);
    let cranker = env.funded(1);

    // any other destination is refused
    let thief = env.funded(1);
    let ix = env.ix_sweep_partner_fees(&col, &coin, &cranker.pubkey(), &thief.pubkey());
    err_has(&env.send_err("sweep to thief", &[ix], &[&cranker]), "WrongTreasury");

    let t0 = env.lamports(&treasury_pda());
    let c0 = env.lamports(&cranker.pubkey());
    let ix = env.ix_sweep_partner_fees(&col, &coin, &cranker.pubkey(), &treasury_pda());
    env.send_ok("sweep", &[ix], &[&cranker]);
    let swept = env.lamports(&treasury_pda()) - t0;
    assert!(swept > 0);
    assert_eq!(env.lamports(&cranker.pubkey()), c0, "cranker gets the transient rent back");
    assert_eq!(env.lamports(&col.claimer), 0);
    // nothing left: a second sweep moves nothing (and does not fail)
    let ix = env.ix_sweep_partner_fees(&col, &coin, &cranker.pubkey(), &treasury_pda());
    env.send_ok("sweep again", &[ix], &[&cranker]);
    assert_eq!(env.lamports(&treasury_pda()) - t0, swept);
    let p: Partner = env.anchor(&col.partner);
    assert_eq!(p.lamports_swept, swept);

    // LP fees: find the claimer's position
    let nft = if env.token_owner(&damm_position_nft_account(&m.nft1)) == col.claimer { m.nft1 } else { m.nft2 };
    let ix = env.ix_sweep_lp_fees(&col, &coin, &m, &nft, &cranker.pubkey(), &thief.pubkey());
    err_has(&env.send_err("lp sweep to thief", &[ix], &[&cranker]), "WrongTreasury");
    // the creator's position cannot be swept by Rook (the claimer does not hold it)
    let creator_nft = if nft == m.nft1 { m.nft2 } else { m.nft1 };
    let ix = env.ix_sweep_lp_fees(&col, &coin, &m, &creator_nft, &cranker.pubkey(), &treasury_pda());
    err_has(&env.send_err("creator position", &[ix], &[&cranker]), "WrongPositionOwner");

    let trader = env.funded(30);
    env.damm_buy(&coin, &m, &trader, 20 * LAMPORTS);
    let t1 = env.lamports(&treasury_pda());
    let ix = env.ix_sweep_lp_fees(&col, &coin, &m, &nft, &cranker.pubkey(), &treasury_pda());
    env.send_ok("lp sweep", &[ix], &[&cranker]);
    let lp = env.lamports(&treasury_pda()) - t1;
    // 1 % pool fee on 20 SOL, 20 % to Meteora, half of the rest to each locked position
    assert!(lp >= 79_000_000 && lp <= 80_000_000, "lp fees {lp}");
}
