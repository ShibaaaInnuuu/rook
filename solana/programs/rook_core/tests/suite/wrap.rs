//! The wrapper: custodian mints (EthNft and Inscriptions), H11, redeem, resolve, delivery, refunds.
//! Sources: test/ethnft/EthNftWrapper.t.sol, EthNftWrapperSent.t.sol, test/floor/MintRate.t.sol, test/Wrapper.t.sol,
//! test/WrapperDeposit.t.sol.

use super::common::*;
use rook_core::error::RookError;
use rook_core::hashing::{keccak, leaf_of, BTC_BURN_ADDRESS, ETH_DEAD};
use rook_core::instructions::MintItemArgs;
use rook_core::state::*;
use rook_core::units::ONE;
use solana_keypair::Keypair;
use solana_signer::Signer;

const DEST: [u8; 20] = [0x42; 20];

/// An env with `n` NFTs wrapped to a holder's account; returns (env, holder, account).
fn with_items(n: u64) -> (Env, Keypair, anchor_lang::prelude::Pubkey) {
    let mut env = Env::new();
    let h = Keypair::new();
    env.svm.airdrop(&h.pubkey(), 1_000 * SOL).unwrap();
    let acc = env.token_account(&h.pubkey());
    for i in 0..n {
        ok(env.wrap_nft(100 + i, tx_hash(i), &acc));
    }
    (env, h, acc)
}

// EthNftWrapper.t.sol: test_onlyTheCustodianMints_onceperTokenAndSource
#[test]
fn only_the_custodian_mints_once_per_token_and_source() {
    let (mut env, _h, acc) = with_items(1);
    assert_eq!(env.balance(&acc), ONE);
    let u = env.user.insecure_clone();
    expect_err(env.mint_item_as(&u, Env::eth_args(1, tx_hash(9)), &acc), RookError::NotCustodian);
    expect_err(env.wrap_nft(100, tx_hash(9), &acc), RookError::AlreadyHeld);
    expect_err(env.wrap_nft(1, [0u8; 32], &acc), RookError::BadInput);
    // a source that does not commit to (token, tx) is refused
    let mut a = Env::eth_args(2, tx_hash(2));
    a.source = [1u8; 32];
    let c = env.custodian.insecure_clone();
    expect_err(env.mint_item_as(&c, a, &acc), RookError::BadInput);
    env.check_ledger();
}

// EthNftWrapper.t.sol: test_aPausedListingStopsMints_butNotRedemption
#[test]
fn a_paused_listing_stops_mints_but_not_redemption() {
    let (mut env, h, acc) = with_items(1);
    let o = env.owner.insecure_clone();
    ok(env.listing_admin(rook_core::instruction::SetPaused { paused: true }, &o));
    expect_err(env.wrap_nft(7, tx_hash(7), &acc), RookError::ListingPaused);
    ok(env.redeem(&h, &acc, Destination::Eth(DEST)));
}

// EthNftWrapper.t.sol: test_H11_mintRate; MintRate.t.sol: test_theDefaultIsTwentyADay_andTheTwentyFirstWaits, test_theWindowSlides
#[test]
fn h11_twenty_a_day_and_the_window_slides() {
    let (mut env, _h, acc) = with_items(20);
    expect_err(env.wrap_nft(500, tx_hash(500), &acc), RookError::MintRateLimited);
    env.advance_time(86_400);
    ok(env.wrap_nft(500, tx_hash(500), &acc));
}

// MintRate.t.sol: test_onlyTheDeployerSetsIt_andInitOnlyBeforeAnyMint, test_loweringIsImmediate_raisingWaitsSevenDaysOnBothClocks
#[test]
fn the_mint_limit_is_set_once_lowered_at_once_raised_after_seven_days() {
    let mut env = Env::new();
    let u = env.user.insecure_clone();
    let o = env.owner.insecure_clone();
    expect_err(env.wrapper_admin(rook_core::instruction::InitMintLimit { limit: 3 }, &u), RookError::NotOwner);
    expect_err(env.wrapper_admin(rook_core::instruction::InitMintLimit { limit: 0 }, &o), RookError::BadInput);
    ok(env.wrapper_admin(rook_core::instruction::InitMintLimit { limit: 3 }, &o));
    let acc = env.token_account(&u.pubkey());
    for i in 0..3 {
        ok(env.wrap_nft(i, tx_hash(i), &acc));
    }
    expect_err(env.wrap_nft(9, tx_hash(9), &acc), RookError::MintRateLimited);
    expect_err(env.wrapper_admin(rook_core::instruction::InitMintLimit { limit: 5 }, &o), RookError::NotFresh);
    ok(env.wrapper_admin(rook_core::instruction::ProposeMintLimit { limit: 5 }, &o));
    assert_eq!(env.wrapper().max_mints_per_day, 3);
    expect_err(env.wrapper_admin(rook_core::instruction::AcceptMintLimit {}, &u), RookError::TooEarly);
    env.advance_time(7 * 86_400);
    ok(env.wrapper_admin(rook_core::instruction::AcceptMintLimit {}, &u));
    assert_eq!(env.wrapper().max_mints_per_day, 5);
    ok(env.wrapper_admin(rook_core::instruction::ProposeMintLimit { limit: 2 }, &o));
    assert_eq!(env.wrapper().max_mints_per_day, 2);
    expect_err(env.wrapper_admin(rook_core::instruction::AcceptMintLimit {}, &u), RookError::WrongStage);
}

// EthNftWrapper.t.sol: test_redeemRefusals; FloorModule.t.sol: test_ETHBackedUnitsCannotRedeemAnItemThatIsNotThere
#[test]
fn redeem_refusals() {
    let (mut env, h, acc) = with_items(1);
    expect_err(env.redeem(&h, &acc, Destination::Eth([0u8; 20])), RookError::BadDestination);
    let custody = rook_core::hashing::parse_evm_address("0x1111111111111111111111111111111111111111").unwrap();
    expect_err(env.redeem(&h, &acc, Destination::Eth(custody)), RookError::BadDestination);
    expect_err(env.redeem(&h, &acc, Destination::Taproot(BTC_BURN_ADDRESS.into())), RookError::BadDestination);
    ok(env.redeem(&h, &acc, Destination::Eth(DEST)));
    // ETH-backed units cannot redeem an item that is not there
    ok(env.post(FLOOR));
    let u = env.user.insecure_clone();
    let uacc = env.user_with_units(400 * SOL);
    expect_err(env.redeem(&u, &uacc, Destination::Eth(DEST)), RookError::NoItem);
    env.check_ledger();
}

// EthNftWrapper.t.sol: test_resolveInOrder_drawsOnlyItemsFreeAtTheBurn, test_noRedrawExists_C02; Wrapper.t.sol:
// test_aBurnOpensATicketPinnedToARoundThatDoesNotExistYet, test_ticketsResolveStrictlyInOrder, test_aTicketResolvesOnce
#[test]
fn resolve_in_order_draws_only_items_free_at_the_burn() {
    let (mut env, h, acc) = with_items(2);
    ok(env.redeem(&h, &acc, Destination::Eth(DEST)));
    let t0 = env.ticket(0);
    assert_eq!(t0.stage, Stage::Pinned);
    assert_eq!(t0.round, rook_core::clock::round_at(env.now) + 20);
    ok(env.redeem(&h, &acc, Destination::Burn));
    // an item wrapped after ticket 1 was burned is not drawable by tickets 0 or 1
    let c = env.custodian.insecure_clone();
    let cacc = env.token_account(&c.pubkey());
    ok(env.wrap_nft(999, tx_hash(999), &cacc));
    // out of order
    let t1 = env.ticket(1);
    env.reach_round(t1.round); // prove_round refuses a round that is not live yet (audit A2-02)
    ok(env.prove(t1.round, [3u8; 32]));
    let i = env.resolve_ix(1, true);
    let u = env.user.insecure_clone();
    expect_err(env.send(&[i], &[&u]), RookError::OutOfOrder);
    // not proven
    let i = env.resolve_ix(0, false);
    expect_err(env.send(&[i], &[&u]), RookError::RoundNotProven);
    ok(env.resolve(0));
    ok(env.resolve(1));
    let (a, b) = (env.ticket(0).item_id, env.ticket(1).item_id);
    assert!(a != b && a < 2 && b < 2, "only the two items free at the burns");
    assert_eq!(env.items().free, vec![] as Vec<u32>);
    assert_eq!(env.items().pending.len(), 1);
    // once
    let i = env.resolve_ix(0, true);
    expect_err(env.send(&[i], &[&u]), RookError::OutOfOrder);
    assert!(env.ticket(0).deliver_by == env.now + DELIVERY_DEADLINE);
    // only the keeper proves rounds (until Q4)
    let i = anchor_lang::solana_program::instruction::Instruction::new_with_bytes(
        rook_core::id(),
        &anchor_lang::InstructionData::data(&rook_core::instruction::ProveRound { round: 5, randomness: [1u8; 32] }),
        anchor_lang::ToAccountMetas::to_account_metas(
            &rook_core::accounts::ProveRound {
                keeper: u.pubkey(),
                payer: u.pubkey(),
                registry: env.registry,
                drand_round: pda(&[rook_core::seeds::DRAND, &5u64.to_le_bytes()]),
                system_program: anchor_lang::solana_program::system_program::ID,
            },
            None,
        ),
    );
    expect_err(env.send(&[i], &[&u]), RookError::NotKeeper);
    env.check_ledger();
}

// EthNftWrapper.t.sol: test_deliveryAndBurn; Wrapper.t.sol: test_onlyTheCustodianRecordsTheDelivery_andTheFrogLeavesTheBooks,
// test_nothingIsDeliveredBeforeItIsResolved, test_aLateDeliveryIsStillRecorded_untilSomebodyRefunds
#[test]
fn delivery_and_burn() {
    let (mut env, h, acc) = with_items(2);
    ok(env.redeem(&h, &acc, Destination::Eth(DEST)));
    ok(env.redeem(&h, &acc, Destination::Burn));
    let u = env.user.insecure_clone();
    // nothing is delivered before it is resolved
    let c = env.custodian.insecure_clone();
    let i = anchor_lang::solana_program::instruction::Instruction::new_with_bytes(
        rook_core::id(),
        &anchor_lang::InstructionData::data(&rook_core::instruction::NoteSent { tx: tx_hash(1) }),
        anchor_lang::ToAccountMetas::to_account_metas(
            &rook_core::accounts::CustodianTicket {
                custodian: c.pubkey(),
                listing: env.listing,
                wrapper: env.wrapper,
                ticket: env.ticket_pda(0),
            },
            None,
        ),
    );
    expect_err(env.send(&[i], &[&c]), RookError::WrongStage);
    ok(env.resolve(0));
    ok(env.resolve(1));
    expect_err(env.record_delivery_as(&u, 0, tx_hash(1)), RookError::NotCustodian);
    expect_err(env.record_delivery(0, [0u8; 32]), RookError::BadInput);
    env.advance_time(10 * 86_400); // late, still recorded
    ok(env.record_delivery(0, tx_hash(1)));
    ok(env.record_delivery(1, tx_hash(2)));
    // a second record: the held marker no longer points to the item (audit A2-09's check fires before the stage's)
    expect_err(env.record_delivery(1, tx_hash(2)), RookError::WrongHeldMarker);
    let w = env.wrapper();
    assert_eq!((w.held, w.open_tickets, w.burned), (0, 0, 1));
    assert_eq!(env.ticket(1).destination, Destination::Burn);
    let _ = ETH_DEAD;
    env.check_ledger();
    // a delivered NFT may come back into custody (its held marker is cleared), but never by the same transaction
    expect_err(env.wrap_nft(100, tx_hash(0), &acc), RookError::AlreadyMinted);
    ok(env.wrap_nft(100, tx_hash(77), &acc));
}

// EthNftWrapper.t.sol: test_aResolvedTicketRefundsOnlyAfterTheGraceWithTheAttestation_orTheLateWindow,
// test_withoutTheAttestationTheLateWindowIsThirtyDays_onBothClocks; EthNftWrapperSent.t.sol (D3-H-1)
#[test]
fn resolved_tickets_refund_only_after_the_windows() {
    let (mut env, h, acc) = with_items(3);
    for _ in 0..3 {
        ok(env.redeem(&h, &acc, Destination::Eth(DEST)));
    }
    for i in 0..3 {
        ok(env.resolve(i));
    }
    let c = env.custodian.insecure_clone();
    // ticket 0: marked undeliverable, refunds at deliver_by + 72 h
    ok(env.custodian_ticket(rook_core::instruction::MarkUndeliverable {}, 0, &c));
    // ticket 1: noted sent, never refunded by the late window
    ok(env.custodian_ticket(rook_core::instruction::NoteSent { tx: tx_hash(5) }, 1, &c));
    let u = env.user.insecure_clone();
    expect_err(env.custodian_ticket(rook_core::instruction::NoteSent { tx: tx_hash(5) }, 1, &u), RookError::NotCustodian);
    expect_err(env.custodian_ticket(rook_core::instruction::NoteSent { tx: [0u8; 32] }, 1, &c), RookError::BadInput);
    let deliver_by = env.ticket(0).deliver_by;
    env.set_time(deliver_by + REFUND_GRACE - 1);
    expect_err(env.refund(0, &acc), RookError::TooEarly);
    env.set_time(deliver_by + REFUND_GRACE);
    ok(env.refund(0, &acc));
    expect_err(env.refund(1, &acc), RookError::DeliverySentNotRecorded);
    expect_err(env.refund(2, &acc), RookError::NotUndeliverable);
    env.set_time(deliver_by + LATE_DELIVERY_WINDOW);
    expect_err(env.refund(1, &acc), RookError::DeliverySentNotRecorded);
    ok(env.refund(2, &acc));
    // D3-H-1: marked after noted, it refunds at the grace
    ok(env.custodian_ticket(rook_core::instruction::MarkUndeliverable {}, 1, &c));
    ok(env.refund(1, &acc));
    expect_err(env.refund(1, &acc), RookError::WrongStage);
    assert_eq!(env.balance(&acc), 3 * ONE);
    assert_eq!(env.items().pending.len(), 3, "the items go back to the set");
    env.check_ledger();
}

// EthNftWrapper.t.sol: test_aPinnedTicketRefundsAfterThePinTimeout_andResolutionSkipsIt; MintRate.t.sol: test_aRefundIsNotAMint
#[test]
fn a_pinned_ticket_refunds_after_the_pin_timeout_and_resolution_skips_it() {
    let (mut env, h, acc) = with_items(2);
    ok(env.redeem(&h, &acc, Destination::Eth(DEST)));
    ok(env.redeem(&h, &acc, Destination::Eth(DEST)));
    env.advance_time(PIN_TIMEOUT - 1);
    expect_err(env.refund(1, &acc), RookError::TooEarly);
    env.advance_time(1);
    let mints = env.wrapper().item_mints;
    ok(env.refund(1, &acc));
    assert_eq!(env.wrapper().item_mints, mints, "a refund is not a mint");
    ok(env.resolve(0));
    // the cursor now sits on the refunded ticket 1: resolving it steps over
    let i = env.resolve_ix(1, false);
    let u = env.user.insecure_clone();
    ok(env.send(&[i], &[&u]));
    assert_eq!(env.wrapper().next_to_resolve, 2);
    // a pinned ticket refunded at the cursor moves it at once
    ok(env.redeem(&h, &acc, Destination::Eth(DEST)));
    env.advance_time(PIN_TIMEOUT);
    ok(env.refund(2, &acc));
    assert_eq!(env.wrapper().next_to_resolve, 3);
    env.check_ledger();
}

/// A two-leaf membership tree.
fn tree(a: &str, b: &str) -> ([u8; 32], Vec<[u8; 32]>, Vec<[u8; 32]>) {
    let (la, lb) = (leaf_of(a), leaf_of(b));
    let root = if la <= lb { keccak(&[&la, &lb]) } else { keccak(&[&lb, &la]) };
    (root, vec![lb], vec![la])
}

fn frog_args(id: &str, proof: Vec<[u8; 32]>, txid: [u8; 32], vout: u32) -> MintItemArgs {
    MintItemArgs {
        key: leaf_of(id),
        source: keccak(&[&txid, &vout.to_be_bytes()]),
        tx: txid,
        vout,
        deposit: true,
        inscription_id: id.into(),
        proof,
    }
}

// WrapperDeposit.t.sol: test_aDepositMintsOneToTheNamedAddress, test_aFrogOutsideTheCollectionIsRefused,
// test_anOutpointMintsOnceEver, test_aFrogAlreadyHeldCannotMintAgain, test_aDeliveredFrogMayBeDepositedAgain;
// Wrapper.t.sol: test_aBurnNeedsATaprootAddress
#[test]
fn inscriptions_membership_outpoints_and_taproot_destinations() {
    let mut env = Env::bare();
    let frog_a = "6fb976ab49dcec017f1e201e84395983204ae1a7c2abf7ced0a85d692e442799i0";
    let frog_b = "a1b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f90i1";
    let (root, proof_a, _proof_b) = tree(frog_a, frog_b);
    let c = env.custodian.pubkey();
    // the listing needs a taproot custody address and a root
    expect_err(
        env.list(ListingKind::Inscriptions { membership_root: root }, "Bitcoin Frogs", "0x1111111111111111111111111111111111111111", c, c),
        RookError::BadInput,
    );
    expect_err(
        env.list(ListingKind::Inscriptions { membership_root: [0u8; 32] }, "Bitcoin Frogs", BTC_BURN_ADDRESS, c, c),
        RookError::Empty,
    );
    ok(env.list(ListingKind::Inscriptions { membership_root: root }, "Bitcoin Frogs", FROG_CUSTODY, c, c));
    env.init_wrapper();
    env.init_module(default_params());
    let h = Keypair::new();
    env.svm.airdrop(&h.pubkey(), 100 * SOL).unwrap();
    let acc = env.token_account(&h.pubkey());
    let cu = env.custodian.insecure_clone();
    // outside the collection
    expect_err(
        env.mint_item_as(&cu, frog_args("deadbeefi0", proof_a.clone(), tx_hash(1), 0), &acc),
        RookError::NotInCollection,
    );
    ok(env.mint_item_as(&cu, frog_args(frog_a, proof_a.clone(), tx_hash(1), 0), &acc));
    assert_eq!(env.balance(&acc), ONE);
    expect_err(env.mint_item_as(&cu, frog_args(frog_a, proof_a.clone(), tx_hash(2), 0), &acc), RookError::AlreadyHeld);
    // the keeper posts an inscriptions floor; the canister may not
    let k = env.keeper.insecure_clone();
    expect_err(env.post(FLOOR), RookError::NotKeeper);
    ok(env.post_as(&k, FLOOR, 9_000, 10));
    // taproot destinations only
    expect_err(env.redeem(&h, &acc, Destination::Eth(DEST)), RookError::BadDestination);
    let mut bad = BTC_BURN_ADDRESS.to_string();
    bad.replace_range(20..21, "x");
    expect_err(env.redeem(&h, &acc, Destination::Taproot(bad)), RookError::BadDestination);
    // never to the custody address itself (audit A2-03)
    expect_err(env.redeem(&h, &acc, Destination::Taproot(FROG_CUSTODY.into())), RookError::CustodyDestination);
    ok(env.redeem(&h, &acc, Destination::Taproot(BTC_BURN_ADDRESS.into())));
    ok(env.resolve(0));
    ok(env.record_delivery(0, tx_hash(3)));
    // delivered, it may be deposited again, but never from the same outpoint
    expect_err(env.mint_item_as(&cu, frog_args(frog_a, proof_a.clone(), tx_hash(1), 0), &acc), RookError::AlreadyMinted);
    ok(env.mint_item_as(&cu, frog_args(frog_a, proof_a, tx_hash(1), 1), &acc));
    env.check_ledger();
}

// ListingRegistry.t.sol: test_aMembershipRootChangesOnlyAfterSevenPublicDays, test_aRuneHasNoRoot (an NFT listing has none)
#[test]
fn a_membership_root_changes_only_after_seven_public_days() {
    let mut env = Env::new();
    let o = env.owner.insecure_clone();
    expect_err(env.listing_admin(rook_core::instruction::ProposeRoot { root: [1u8; 32] }, &o), RookError::WrongKind);
    let c = env.custodian.pubkey();
    ok(env.list(ListingKind::Inscriptions { membership_root: [9u8; 32] }, "Frogs", BTC_BURN_ADDRESS, c, c));
    let g = env.guardian.insecure_clone();
    expect_err(env.listing_admin(rook_core::instruction::ProposeRoot { root: [1u8; 32] }, &g), RookError::NotOwner);
    expect_err(env.listing_admin(rook_core::instruction::ProposeRoot { root: [0u8; 32] }, &o), RookError::Empty);
    expect_err(env.listing_admin(rook_core::instruction::AcceptRoot {}, &o), RookError::NothingPending);
    ok(env.listing_admin(rook_core::instruction::ProposeRoot { root: [1u8; 32] }, &o));
    env.advance_time(7 * 86_400 - 1);
    expect_err(env.listing_admin(rook_core::instruction::AcceptRoot {}, &o), RookError::TooEarly);
    env.advance_time(1);
    ok(env.listing_admin(rook_core::instruction::AcceptRoot {}, &o));
    let l: Listing = env.get(&env.listing);
    assert_eq!(l.kind, ListingKind::Inscriptions { membership_root: [1u8; 32] });
}

/// BIP341's NUMS key-path output on signet/testnet4 (the burn address's witness program with the `tb` prefix).
const TB_BURN: &str = "tb1p2zffkaxp5py4fdutfdsrt6t6tcrc5ks09rkfd428hlhf4n5q8tqqeygjde";
const TB_DEST: &str = "tb1p5cyxnuxmeuwuvkwfem96lqzszd02n6xdcjrs20cac6yqjjwudpxqp3mvzv";

// 2026-10-09 (devnet rehearsal): the registry's Bitcoin network decides the taproot prefix of an Inscriptions listing's custody
// and of its redemptions; mainnet stays bc1p-only.
#[test]
fn the_registry_bitcoin_network_decides_the_taproot_prefix() {
    let frog_a = "6fb976ab49dcec017f1e201e84395983204ae1a7c2abf7ced0a85d692e442799i0";
    let frog_b = "a1b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f90i1";
    let (root, proof_a, proof_b) = tree(frog_a, frog_b);

    // mainnet (the default every other test runs on): a signet custody address is refused
    let mut env = Env::bare();
    assert_eq!(env.registry().bitcoin_network, BitcoinNetwork::Mainnet);
    let c = env.custodian.pubkey();
    expect_err(env.list(ListingKind::Inscriptions { membership_root: root }, "Frogs", TB_BURN, c, c), RookError::BadInput);

    // signet: a mainnet custody address is refused, a signet one listed
    let mut env = Env::bare_on(BitcoinNetwork::Signet);
    assert_eq!(env.registry().bitcoin_network, BitcoinNetwork::Signet);
    let c = env.custodian.pubkey();
    expect_err(env.list(ListingKind::Inscriptions { membership_root: root }, "Frogs", BTC_BURN_ADDRESS, c, c), RookError::BadInput);
    ok(env.list(ListingKind::Inscriptions { membership_root: root }, "Frogs", TB_BURN, c, c));
    env.init_wrapper();
    env.init_module(default_params());
    let h = Keypair::new();
    env.svm.airdrop(&h.pubkey(), 100 * SOL).unwrap();
    let acc = env.token_account(&h.pubkey());
    let cu = env.custodian.insecure_clone();
    ok(env.mint_item_as(&cu, frog_args(frog_a, proof_a, tx_hash(1), 0), &acc));
    ok(env.mint_item_as(&cu, frog_args(frog_b, proof_b, tx_hash(2), 0), &acc));
    // a mainnet destination on a signet listing is refused; signet ones and the burn are taken
    expect_err(env.redeem(&h, &acc, Destination::Taproot(BTC_BURN_ADDRESS.into())), RookError::BadDestination);
    let mut bad = TB_DEST.to_string();
    bad.replace_range(20..21, "x");
    expect_err(env.redeem(&h, &acc, Destination::Taproot(bad)), RookError::BadDestination);
    ok(env.redeem(&h, &acc, Destination::Taproot(TB_DEST.into())));
    ok(env.redeem(&h, &acc, Destination::Burn));
    assert_eq!(env.ticket(0).destination, Destination::Taproot(TB_DEST.into()));
    ok(env.resolve(0));
    ok(env.record_delivery(0, tx_hash(3)));
    env.check_ledger();

    // testnet4 shares signet's prefix
    let mut env = Env::bare_on(BitcoinNetwork::Testnet4);
    let c = env.custodian.pubkey();
    ok(env.list(ListingKind::Inscriptions { membership_root: root }, "Frogs", TB_DEST, c, c));
}

/// A mainnet taproot address for the frog listing's custody (any valid one; distinct from the burn address the test redeems to).
const FROG_CUSTODY: &str = "bc1p5cyxnuxmeuwuvkwfem96lqzszd02n6xdcjrs20cac6yqjjwudpxqkedrcr";

/// Reads (update authority, mint, name, symbol, uri, is_mutable) of a Metaplex `Metadata` account: key u8, update_authority,
/// mint, then `Data { name, symbol, uri }` as Borsh strings (Metaplex pads them with NULs), seller fee u16, creators Option,
/// primary_sale_happened bool, is_mutable bool.
fn read_metadata(d: &[u8]) -> (Pubkey, Pubkey, String, String, String, bool) {
    let key = |o: usize| Pubkey::new_from_array(d[o..o + 32].try_into().unwrap());
    let mut o = 65;
    let mut strings = Vec::new();
    for _ in 0..3 {
        let n = u32::from_le_bytes(d[o..o + 4].try_into().unwrap()) as usize;
        strings.push(String::from_utf8(d[o + 4..o + 4 + n].to_vec()).unwrap().trim_end_matches('\0').to_string());
        o += 4 + n;
    }
    o += 2; // seller_fee_basis_points
    assert_eq!(d[o], 0, "no creators");
    let is_mutable = d[o + 2] == 1; // after primary_sale_happened
    let uri = strings.pop().unwrap();
    let symbol = strings.pop().unwrap();
    let name = strings.pop().unwrap();
    (key(1), key(33), name, symbol, uri, is_mutable)
}

use anchor_lang::prelude::Pubkey;

// The wX mint's Metaplex metadata (2026-10-10): created at `init_wrapper` (name, symbol, uri; update authority = the wrapper
// PDA; mutable), changed only through the owner's `set_wx_metadata`. Measures CU and rent.
#[test]
fn wx_metadata_at_init_and_the_owner_updates_it() {
    let mut env = Env::bare();
    let c = env.custodian.pubkey();
    ok(env.list(ListingKind::EthNft { chain_id: 1, collection: [0x5a; 20] }, "Milady", "0x1111111111111111111111111111111111111111", c, c));
    // Metaplex limits: symbol at most 10, name at most 32
    let bad = rook_core::WxMetadata { name: "Wrapped Milady".into(), symbol: "wMILADYMAKER".into(), uri: String::new() };
    expect_err(env.init_wrapper_with(bad), RookError::BadMetadata);
    let bad = rook_core::WxMetadata { name: "W".repeat(33), symbol: "wMILADY".into(), uri: String::new() };
    expect_err(env.init_wrapper_with(bad), RookError::BadMetadata);
    let o = env.owner.pubkey();
    let before = env.lamports(&o);
    let meta = rook_core::WxMetadata { name: "Wrapped Milady".into(), symbol: "wMILADY".into(), uri: String::new() };
    let r = ok(env.init_wrapper_with(meta));
    let paid = before - env.lamports(&o);
    let md = metadata_pda(&env.wx_mint);
    let acc = env.svm.get_account(&md).expect("metadata account");
    assert_eq!(acc.owner, rook_core::metadata::MPL_TOKEN_METADATA_ID);
    let (ua, mint, name, symbol, uri, mutable) = read_metadata(&acc.data);
    assert_eq!((ua, mint), (env.wrapper, env.wx_mint), "update authority = the wrapper PDA, for this mint");
    assert_eq!((name.as_str(), symbol.as_str(), uri.as_str(), mutable), ("Wrapped Milady", "wMILADY", "", true));
    let rent = env.svm.minimum_balance_for_rent_exemption(acc.data.len());
    println!(
        "init_wrapper with metadata: {} CU; metadata account {} bytes, {} lamports (rent-exempt minimum {}); the owner paid {} lamports for the whole init",
        r.compute_units_consumed,
        acc.data.len(),
        acc.lamports,
        rent,
        paid
    );
    // the owner sets the image later; anyone else is refused
    let img = rook_core::WxMetadata { name: "Wrapped Milady".into(), symbol: "wMILADY".into(), uri: "https://rook.example/wMILADY.json".into() };
    let u = env.user.insecure_clone();
    expect_err(env.set_wx_metadata_as(&u, img.clone()), RookError::NotOwner);
    let ow = env.owner.insecure_clone();
    let r = ok(env.set_wx_metadata_as(&ow, img));
    println!("set_wx_metadata: {} CU", r.compute_units_consumed);
    let (ua, _, name, symbol, uri, mutable) = read_metadata(&env.svm.get_account(&md).unwrap().data);
    assert_eq!(
        (ua, name.as_str(), symbol.as_str(), uri.as_str(), mutable),
        (env.wrapper, "Wrapped Milady", "wMILADY", "https://rook.example/wMILADY.json", true)
    );
    // the wrapper still mints (the mint authority is unchanged)
    env.init_module(default_params());
    let acc = env.token_account(&u.pubkey());
    ok(env.wrap_nft(1, tx_hash(1), &acc));
    assert_eq!(env.balance(&acc), ONE);
}
