//! The registry: listings, roles, pauses, converters, ownership. Source: test/ListingRegistry.t.sol and
//! test/ethnft/EthNftFlow.t.sol (constructor refusals).

use super::common::*;
use rook_core::error::RookError;
use rook_core::state::*;
use solana_keypair::Keypair;
use solana_signer::Signer;
use anchor_lang::prelude::Pubkey;

// ListingRegistry.t.sol: test_onlyTheOwnerLists, test_aListingNeedsItsFacts; EthNftWrapper.t.sol: test_constructorRefusals
#[test]
fn only_the_owner_lists_and_a_listing_needs_its_facts() {
    let mut env = Env::bare();
    let c = env.custodian.pubkey();
    let eth = ListingKind::EthNft { chain_id: 1, collection: [1u8; 20] };
    let custody = "0x1111111111111111111111111111111111111111";
    // not the owner
    let u = env.user.insecure_clone();
    let listing = pda(&[rook_core::seeds::LISTING, &0u32.to_le_bytes()]);
    let i = ix(
        rook_core::instruction::ListCollection {
            args: rook_core::instructions::ListArgs { kind: eth, name: "x".into(), custody: custody.into(), custodian: c, poster: c },
        },
        rook_core::accounts::ListCollection {
            owner: u.pubkey(),
            payer: u.pubkey(),
            registry: env.registry,
            listing,
            system_program: anchor_lang::solana_program::system_program::ID,
        },
    );
    expect_err(env.send(&[i], &[&u]), RookError::NotOwner);
    expect_err(env.list(eth, "", custody, c, c), RookError::Empty);
    expect_err(env.list(eth, "Milady", "", c, c), RookError::Empty);
    expect_err(env.list(eth, "Milady", custody, anchor_lang::prelude::Pubkey::default(), c), RookError::BadInput);
    expect_err(env.list(eth, "Milady", custody, c, anchor_lang::prelude::Pubkey::default()), RookError::BadInput);
    expect_err(env.list(ListingKind::EthNft { chain_id: 0, collection: [1u8; 20] }, "Milady", custody, c, c), RookError::BadInput);
    expect_err(env.list(ListingKind::EthNft { chain_id: 1, collection: [0u8; 20] }, "Milady", custody, c, c), RookError::BadInput);
    expect_err(env.list(eth, "Milady", "bc1notanevmaddress", c, c), RookError::BadInput);
    ok(env.list(eth, "Milady", custody, c, c));
    ok(env.list(eth, "Azuki", custody, c, c));
    assert_eq!(env.listing_count(), 2);
    let l: Listing = env.get(&env.listing);
    assert_eq!((l.id, l.name.as_str(), l.custodian), (1, "Azuki", c));
}

// ListingRegistry.t.sol: test_rolesChangeOnlyAfterSevenPublicDays, test_whoeverSpendsNeverVouches (N-8: the guardian is no role)
#[test]
fn roles_change_only_after_seven_public_days() {
    let mut env = Env::new();
    let k2 = Keypair::new();
    let g = env.guardian.insecure_clone();
    expect_err(env.registry_admin(rook_core::instruction::ProposeRoles { keeper: k2.pubkey() }, Some(&g)), RookError::NotOwner);
    expect_err(env.registry_admin(rook_core::instruction::ProposeRoles { keeper: g.pubkey() }, None), RookError::SameKey);
    expect_err(env.registry_admin(rook_core::instruction::AcceptRoles {}, None), RookError::NothingPending);
    ok(env.registry_admin(rook_core::instruction::ProposeRoles { keeper: k2.pubkey() }, None));
    assert_eq!(env.registry().keeper, env.keeper.pubkey(), "the first keeper stays until the delay");
    env.advance_time(7 * 86_400 - 1);
    expect_err(env.registry_admin(rook_core::instruction::AcceptRoles {}, None), RookError::TooEarly);
    env.advance_time(1);
    ok(env.registry_admin(rook_core::instruction::AcceptRoles {}, None));
    assert_eq!(env.registry().keeper, k2.pubkey());
    // the guardian may be no role
    expect_err(env.registry_admin(rook_core::instruction::SetGuardian { guardian: k2.pubkey() }, None), RookError::SameKey);
    let o = env.owner.pubkey();
    expect_err(env.registry_admin(rook_core::instruction::SetGuardian { guardian: o }, None), RookError::SameKey);
}

// ListingRegistry.t.sol: test_pausingIsTheOwners; the keeper switch (owner or guardian pause, owner resume)
#[test]
fn pausing_is_the_owners_and_the_guardian_only_stops() {
    let mut env = Env::new();
    let g = env.guardian.insecure_clone();
    let u = env.user.insecure_clone();
    expect_err(env.listing_admin(rook_core::instruction::SetPaused { paused: true }, &g), RookError::NotOwner);
    expect_err(env.registry_admin(rook_core::instruction::PauseKeeper {}, Some(&u)), RookError::NotGuardian);
    expect_err(env.listing_admin(rook_core::instruction::PausePoster {}, &u), RookError::NotGuardian);
    ok(env.registry_admin(rook_core::instruction::PauseKeeper {}, Some(&g)));
    assert!(env.registry().keeper_paused);
    expect_err(env.registry_admin(rook_core::instruction::UnpauseKeeper {}, Some(&g)), RookError::NotOwner);
    env.advance_time(60);
    ok(env.registry_admin(rook_core::instruction::UnpauseKeeper {}, None));
    let r = env.registry();
    assert!(!r.keeper_paused);
    assert_eq!(r.keeper_resumed_at, env.now);
}

#[test]
fn converters_are_the_owners_allow_list() {
    let mut env = Env::new();
    let a = Keypair::new().pubkey();
    env.allow_converter(&a);
    expect_err(env.registry_admin(rook_core::instruction::ProposeConverter { converter: a }, None), RookError::BadInput);
    for _ in 0..7 {
        env.allow_converter(&Keypair::new().pubkey());
    }
    expect_err(
        env.registry_admin(rook_core::instruction::ProposeConverter { converter: Keypair::new().pubkey() }, None),
        RookError::Full,
    );
    ok(env.registry_admin(rook_core::instruction::RemoveConverter { converter: a }, None));
    assert!(!env.registry().is_converter(&a));
    expect_err(env.registry_admin(rook_core::instruction::RemoveConverter { converter: a }, None), RookError::BadInput);
}

// Open question 2 of the as-built doc: a converter waits CHANGE_DELAY in public, like the keeper.
#[test]
fn a_converter_waits_seven_public_days() {
    let mut env = Env::new();
    let a = Keypair::new().pubkey();
    let g = env.guardian.insecure_clone();
    let u = env.user.insecure_clone();
    expect_err(env.registry_admin(rook_core::instruction::AcceptConverter {}, None), RookError::NothingPending);
    expect_err(env.registry_admin(rook_core::instruction::ProposeConverter { converter: a }, Some(&g)), RookError::NotOwner);
    ok(env.registry_admin(rook_core::instruction::ProposeConverter { converter: a }, None));
    let r = env.registry();
    assert_eq!((r.pending_converter, r.pending_converter_at), (a, env.now + CHANGE_DELAY));
    assert!(!r.is_converter(&a), "proposed is not allowed");
    env.advance_time(CHANGE_DELAY - 1);
    expect_err(env.registry_admin(rook_core::instruction::AcceptConverter {}, None), RookError::TooEarly);
    env.advance_time(1);
    expect_err(env.registry_admin(rook_core::instruction::AcceptConverter {}, Some(&u)), RookError::NotOwner);
    ok(env.registry_admin(rook_core::instruction::AcceptConverter {}, None));
    let r = env.registry();
    assert!(r.is_converter(&a));
    assert_eq!((r.pending_converter, r.pending_converter_at, r.converter_count), (Pubkey::default(), 0, 1));
    expect_err(env.registry_admin(rook_core::instruction::AcceptConverter {}, None), RookError::NothingPending);
    // a new proposal restarts the clock and replaces the pending one
    let b = Keypair::new().pubkey();
    let c = Keypair::new().pubkey();
    ok(env.registry_admin(rook_core::instruction::ProposeConverter { converter: b }, None));
    env.advance_time(CHANGE_DELAY - 10);
    ok(env.registry_admin(rook_core::instruction::ProposeConverter { converter: c }, None));
    env.advance_time(10);
    expect_err(env.registry_admin(rook_core::instruction::AcceptConverter {}, None), RookError::TooEarly);
    env.advance_time(CHANGE_DELAY);
    ok(env.registry_admin(rook_core::instruction::AcceptConverter {}, None));
    assert!(env.registry().is_converter(&c) && !env.registry().is_converter(&b));
}

// Removal is stop-only and instant: the owner or the guardian, of an allowed or a pending converter; never a stranger.
#[test]
fn the_guardian_removes_a_converter_at_once() {
    let mut env = Env::new();
    let a = Keypair::new().pubkey();
    env.allow_converter(&a);
    let g = env.guardian.insecure_clone();
    let u = env.user.insecure_clone();
    expect_err(env.registry_admin(rook_core::instruction::RemoveConverter { converter: a }, Some(&u)), RookError::NotGuardian);
    ok(env.registry_admin(rook_core::instruction::RemoveConverter { converter: a }, Some(&g)));
    assert!(!env.registry().is_converter(&a));
    // a pending one is cancelled the same way, and cannot then be accepted
    let b = Keypair::new().pubkey();
    ok(env.registry_admin(rook_core::instruction::ProposeConverter { converter: b }, None));
    ok(env.registry_admin(rook_core::instruction::RemoveConverter { converter: b }, Some(&g)));
    env.advance_time(CHANGE_DELAY);
    expect_err(env.registry_admin(rook_core::instruction::AcceptConverter {}, None), RookError::NothingPending);
    assert!(!env.registry().is_converter(&b));
    // the guardian can neither propose nor accept
    expect_err(env.registry_admin(rook_core::instruction::ProposeConverter { converter: b }, Some(&g)), RookError::NotOwner);
    // nothing to remove
    expect_err(env.registry_admin(rook_core::instruction::RemoveConverter { converter: b }, Some(&g)), RookError::BadInput);
}

// Open question 1 of the as-built doc: init_registry is the upgrade authority's, not the first caller's.
#[test]
fn only_the_upgrade_authority_initializes_the_registry() {
    let mut env = Env::bare(); // the harness initialized it as the authority (the owner)
    assert_eq!(env.registry().owner, env.owner.pubkey());
    // a fresh SVM: nobody initialized yet
    let mut svm = litesvm::LiteSVM::new();
    svm.add_program(rook_core::id(), &rook_core_so()).unwrap();
    std::mem::swap(&mut env.svm, &mut svm);
    let deployer = Keypair::new();
    let stranger = env.user.insecure_clone();
    for k in [&deployer, &stranger] {
        env.svm.airdrop(&k.pubkey(), 10 * SOL).unwrap();
    }
    let s = stranger.pubkey();
    let d = deployer.pubkey();
    // no upgrade authority (LiteSVM's default; an immutable program): nobody
    let i = env.init_registry_ix(&d, &d, &d);
    expect_err(env.send(&[i], &[&deployer]), RookError::NotUpgradeAuthority);
    env.set_upgrade_authority(Some(&d));
    // a stranger, alone or with the deployer only paying
    let i = env.init_registry_ix(&s, &s, &s);
    expect_err(env.send(&[i], &[&stranger]), RookError::NotUpgradeAuthority);
    let i = env.init_registry_ix(&s, &d, &s);
    expect_err(env.send(&[i], &[&deployer, &stranger]), RookError::NotUpgradeAuthority);
    // a substituted ProgramData (another program's, naming the stranger) is refused
    let mut i = env.init_registry_ix(&s, &s, &s);
    let fake_pd = Keypair::new().pubkey();
    let mut acc = env.svm.get_account(&Env::program_data()).unwrap();
    acc.data[12] = 1;
    acc.data[13..45].copy_from_slice(s.as_ref());
    env.svm.set_account(fake_pd, acc).unwrap();
    let idx = i.accounts.iter().position(|m| m.pubkey == Env::program_data()).unwrap();
    i.accounts[idx].pubkey = fake_pd;
    match env.send(&[i], &[&stranger]) {
        Err((e, _)) => assert!(e.contains("Custom(2006)"), "ConstraintSeeds, got {e}"),
        Ok(_) => panic!("a substituted ProgramData was accepted"),
    }
    // the authority (a third party pays), naming the owner
    let i = env.init_registry_ix(&d, &s, &s);
    ok(env.send(&[i], &[&stranger, &deployer]));
    assert_eq!(env.registry().owner, s);
    // once
    let i = env.init_registry_ix(&d, &d, &d);
    expect_fail(env.send(&[i], &[&deployer]));
}

// Q14 (2026-10-09): init_registry allows its initial converters at once (only the upgrade authority initializes, before
// anything is live); at most MAX_CONVERTERS, distinct, none default. Later additions still wait CHANGE_DELAY.
#[test]
fn init_registry_allows_its_initial_converters_at_once() {
    let mut env = Env::bare();
    let mut svm = litesvm::LiteSVM::new();
    svm.add_program(rook_core::id(), &rook_core_so()).unwrap();
    std::mem::swap(&mut env.svm, &mut svm);
    let d = Keypair::new();
    env.svm.airdrop(&d.pubkey(), 10 * SOL).unwrap();
    env.set_upgrade_authority(Some(&d.pubkey()));
    let k = d.pubkey();
    let net = BitcoinNetwork::Mainnet;
    let keys: Vec<Pubkey> = (0..=MAX_CONVERTERS).map(|_| Keypair::new().pubkey()).collect();
    let i = env.init_registry_with_ix(&k, &k, &k, net, keys.clone());
    expect_err(env.send(&[i], &[&d]), RookError::Full);
    let i = env.init_registry_with_ix(&k, &k, &k, net, vec![keys[0], Pubkey::default()]);
    expect_err(env.send(&[i], &[&d]), RookError::BadInput);
    let i = env.init_registry_with_ix(&k, &k, &k, net, vec![keys[0], keys[1], keys[0]]);
    expect_err(env.send(&[i], &[&d]), RookError::BadInput);
    let i = env.init_registry_with_ix(&k, &k, &k, net, vec![keys[0], keys[1]]);
    ok(env.send(&[i], &[&d]));
    let r = env.registry();
    assert!(r.is_converter(&keys[0]) && r.is_converter(&keys[1]) && !r.is_converter(&keys[2]));
    assert_eq!((r.converter_count, r.pending_converter, r.pending_converter_at), (2, Pubkey::default(), 0));
    // a third still waits seven public days; removal is instant
    ok(env.registry_admin(rook_core::instruction::ProposeConverter { converter: keys[2] }, Some(&d)));
    expect_err(env.registry_admin(rook_core::instruction::AcceptConverter {}, Some(&d)), RookError::TooEarly);
    ok(env.registry_admin(rook_core::instruction::RemoveConverter { converter: keys[0] }, Some(&d)));
    assert!(!env.registry().is_converter(&keys[0]) && env.registry().is_converter(&keys[1]));
    // a full initial list fills the allow-list
    let mut svm = litesvm::LiteSVM::new();
    svm.add_program(rook_core::id(), &rook_core_so()).unwrap();
    std::mem::swap(&mut env.svm, &mut svm);
    env.svm.airdrop(&k, 10 * SOL).unwrap();
    env.set_upgrade_authority(Some(&k));
    let i = env.init_registry_with_ix(&k, &k, &k, net, keys[..MAX_CONVERTERS].to_vec());
    ok(env.send(&[i], &[&d]));
    assert_eq!(env.registry().converter_count as usize, MAX_CONVERTERS);
    expect_err(
        env.registry_admin(rook_core::instruction::ProposeConverter { converter: keys[MAX_CONVERTERS] }, Some(&d)),
        RookError::Full,
    );
}

// Ownable2Step: the new owner accepts; ownership is never renounced (there is no instruction for it).
#[test]
fn ownership_moves_in_two_steps() {
    let mut env = Env::new();
    let n = Keypair::new();
    env.svm.airdrop(&n.pubkey(), 10 * SOL).unwrap();
    let g = env.guardian.pubkey();
    expect_err(env.registry_admin(rook_core::instruction::ProposeOwner { new_owner: g }, None), RookError::SameKey);
    ok(env.registry_admin(rook_core::instruction::ProposeOwner { new_owner: n.pubkey() }, None));
    let u = env.user.insecure_clone();
    expect_err(env.registry_admin(rook_core::instruction::AcceptOwner {}, Some(&u)), RookError::NotOwner);
    ok(env.registry_admin(rook_core::instruction::AcceptOwner {}, Some(&n)));
    assert_eq!(env.registry().owner, n.pubkey());
    expect_err(env.registry_admin(rook_core::instruction::PauseKeeper {}, None), RookError::NotGuardian);
}

#[test]
fn module_parameters_are_checked_and_fixed() {
    let mut env = Env::bare();
    let c = env.custodian.pubkey();
    ok(env.list(ListingKind::EthNft { chain_id: 1, collection: [1u8; 20] }, "Milady", "0x1111111111111111111111111111111111111111", c, c));
    env.init_wrapper();
    let mut p = default_params();
    p.reserve_bps = 5_001;
    let i = env.init_module_ix(p);
    let o = env.owner.insecure_clone();
    expect_err(env.send(&[i], &[&o]), RookError::BadParams);
    let mut p = default_params();
    p.max_items_per_day = 33;
    let i = env.init_module_ix(p);
    expect_err(env.send(&[i], &[&o]), RookError::BadParams);
    let mut p = default_params();
    p.max_public_eth_backed = p.max_eth_backed + 1;
    let i = env.init_module_ix(p);
    expect_err(env.send(&[i], &[&o]), RookError::BadParams);
    env.init_module(default_params());
    assert_eq!(env.module().params, default_params());
    // once
    let i = env.init_module_ix(default_params());
    expect_fail(env.send(&[i], &[&o]));
}
