//! `back` against the REAL rook_core `convert` (opt-in, needs rook_core to compile and its SBPF v2 test binary):
//!   cargo build-sbf --manifest-path programs/rook_core/Cargo.toml --arch v2 --sbf-out-dir target/deploy-sbpfv2
//!   cargo test -p rook_backing --features real-rook-core --test real_rook_core -- --nocapture
//! rook_core's own LiteSVM suite (programs/rook_core/tests/suite/common.rs) builds the registry, the Milady listing, its
//! wrapper, floor module and treasury, posts a floor, and passes rook_backing's program-wide ["converter"] PDA to
//! `init_registry` as an initial converter (allowed at once: no `CHANGE_DELAY`, no clock jump); this test then adds Meteora
//! DBC / DAMM v2 and rook_backing to the same SVM and runs a coin to graduation and `back`.

#[path = "../../../tests/harness/mod.rs"]
mod harness;
#[path = "../../rook_core/tests/suite/common.rs"]
mod core;

use harness::*;
use rook_backing::Vault;
use rook_core::state::Module;
use solana_signer::Signer;

#[test]
fn back_converts_through_the_real_rook_core() {
    let mut c = core::Env::with_converters(vec![converter_pda()]);
    let t0 = c.now;
    core::ok(c.post(core::FLOOR));
    // allowed from init: nothing proposed, the clock never moved past CHANGE_DELAY
    let reg = c.registry();
    assert!(reg.is_converter(&converter_pda()) && reg.converter_count == 1);
    assert_eq!((reg.pending_converter, reg.pending_converter_at), (anchor_lang::prelude::Pubkey::default(), 0));
    let (listing, registry, module, module_vault, wrapper, wx_mint, treasury) =
        (c.listing, c.registry, c.module, c.vault, c.wrapper, c.wx_mint, c.treasury);
    assert_eq!(treasury, treasury_pda(), "rook_core's treasury is the PDA rook_backing pins");
    let module_before: Module = c.get(&module);
    let owner = c.owner.insecure_clone();
    assert_eq!(registry, registry_pda());

    let mut env = Env::from_svm(c.svm);
    // rook_backing's admin is the real registry's owner: a stranger is refused, the owner registers
    let stranger = env.funded(5);
    let params = rook_backing::PartnerParams { listing, registry, module, module_vault, wrapper, wx_mint };
    let ix = env.ix_register_partner(&stranger.pubkey(), &registry, params);
    // (settings are created by register_collection below; create them first so the refusal is about the admin)
    let upgrade = env.funded(1);
    env.set_upgrade_authority(&upgrade.pubkey());
    let init = env.ix_init_backing(&upgrade.pubkey());
    env.send_ok("init_backing", &[init], &[&upgrade]);
    let logs = env.send_err("stranger registers", &[ix], &[&stranger]);
    assert!(logs.contains("Unauthorized"), "{logs}");
    let col = env.register_collection(&owner, listing, registry, module, module_vault, wrapper, wx_mint);
    let (coin, m) = env.graduated_coin(&col, 50);
    let v: Vault = env.anchor(&vault_pda(&coin.mint));
    assert!(v.launched_at > 0 && v.launched_at < env.now(), "the pool's creation time");

    let vault_lamports = env.lamports(&module_vault);
    let ix = env.ix_back(&col, &coin, &env.payer.pubkey(), 1);
    let r = env.send_ok("rook_backing.back (real rook_core)", &[ix], &[]);
    let v: Vault = env.anchor(&vault_pda(&coin.mint));
    let module_after: Module = env.anchor(&module);
    println!(
        "real convert: {} lamports → {} wX base units ({} CU for back); module converted {} → {}",
        v.lamports_backed,
        v.wx_units,
        r.compute_units_consumed - 150,
        module_before.converted_lamports,
        module_after.converted_lamports
    );
    assert!(v.graduated);
    assert!(env.now() - t0 < rook_core::state::CHANGE_DELAY, "back converted with no clock jump past CHANGE_DELAY");
    assert_eq!(v.lamports_backed, THRESHOLD / 2);
    assert!(v.wx_units > 0);
    assert_eq!(env.token_amount(&vault_wx_pda(&coin.mint)), v.wx_units);
    assert_eq!(env.lamports(&module_vault) - vault_lamports, THRESHOLD / 2);
    assert_eq!(module_after.converted_lamports - module_before.converted_lamports, THRESHOLD / 2);
    assert_eq!(module_after.converted_units - module_before.converted_units, v.wx_units);

    // the partner fees land in the real treasury account
    env.ata(&col.claimer, &coin.mint);
    let t0 = env.lamports(&treasury);
    let ix = env.ix_sweep_partner_fees(&col, &coin, &env.payer.pubkey(), &treasury);
    env.send_ok("sweep_partner_fees (real treasury)", &[ix], &[]);
    assert!(env.lamports(&treasury) > t0);

    // and a holder redeems against real wX
    let trader = env.funded(20);
    env.damm_buy(&coin, &m, &trader, 5 * LAMPORTS);
    let tc = anchor_spl::associated_token::get_associated_token_address(&trader.pubkey(), &coin.mint);
    let tw = env.ata(&trader.pubkey(), &wx_mint);
    let held = env.token_amount(&tc);
    let ix = env.ix_redeem(&coin.mint, &wx_mint, &trader.pubkey(), &tc, &tw, held);
    env.send_ok("redeem (real wX)", &[ix], &[&trader]);
    assert!(env.token_amount(&tw) > 0);
}

/// An ownership change in the real rook_core registry (Ownable2Step) carries over to rook_backing's admin at once.
#[test]
fn the_backing_admin_follows_the_real_registry_owner() {
    let mut c = core::Env::with_converters(vec![converter_pda()]);
    let (listing, registry, module, module_vault, wrapper, wx_mint) = (c.listing, c.registry, c.module, c.vault, c.wrapper, c.wx_mint);
    let old = c.owner.insecure_clone();
    let new = solana_keypair::Keypair::new();
    c.svm.airdrop(&new.pubkey(), 100 * LAMPORTS).unwrap();
    let mut env = Env::from_svm(c.svm);
    let col = env.register_collection(&old, listing, registry, module, module_vault, wrapper, wx_mint);
    let cfg = env.create_config(&col.claimer, 50);
    let cfg2 = env.create_config(&col.claimer, 50);
    let ix = env.ix_register_config(&col, &old.pubkey(), &cfg);
    env.send_ok("old owner registers", &[ix], &[&old]);

    let admin_ix = |signer: &solana_keypair::Keypair, data: Vec<u8>| anchor_lang::solana_program::instruction::Instruction {
        program_id: rook_core::ID,
        accounts: anchor_lang::ToAccountMetas::to_account_metas(
            &rook_core::accounts::RegistryAdmin { signer: signer.pubkey(), registry },
            None,
        ),
        data,
    };
    use anchor_lang::InstructionData;
    let ix = admin_ix(&old, rook_core::instruction::ProposeOwner { new_owner: new.pubkey() }.data());
    env.send_ok("propose_owner", &[ix], &[&old]);
    // proposed is not yet the owner: the old one still administers rook_backing
    let ix = env.ix_register_config(&col, &new.pubkey(), &cfg2);
    assert!(env.send_err("pending owner", &[ix], &[&new]).contains("Unauthorized"));
    let ix = admin_ix(&new, rook_core::instruction::AcceptOwner {}.data());
    env.send_ok("accept_owner", &[ix], &[&new]);
    let ix = env.ix_register_config(&col, &old.pubkey(), &cfg2);
    assert!(env.send_err("old owner after the change", &[ix], &[&old]).contains("Unauthorized"));
    let ix = env.ix_register_config(&col, &new.pubkey(), &cfg2);
    env.send_ok("new owner registers", &[ix], &[&new]);
    let p: rook_backing::Partner = env.anchor(&col.partner);
    assert_eq!(p.configs, vec![cfg, cfg2]);
}
