//! The wrapper ledger in the canister's books: item mints (EthNft and Inscriptions), H11, redeem, resolve, delivery, refunds.
//! Ported from solana/programs/rook_core/tests/suite/wrap.rs and flow.rs (names kept; the comment above each test names the
//! rook_core test). Signer checks (`NotCustodian`, `NotOwner`) are gone: the canister is the only caller. Not ported: the
//! membership-root change (registry governance, §4.1 `governance.rs`) and the wX Metaplex metadata (the ceremony's, §2).

mod books_common;

use books_common::*;
use etch_autobuyer::books::ledger::*;
use etch_autobuyer::books::math::{keccak, leaf_of, round_at, BTC_BURN_ADDRESS};
use etch_autobuyer::books::*;

const DEST: [u8; 20] = [0x42; 20];

/// An env with `n` NFTs wrapped to HOLDER.
fn with_items(n: u64) -> Env {
    let mut env = Env::new();
    for i in 0..n {
        ok(env.wrap_nft(100 + i, tx_hash(i), MintTo::Holder(HOLDER)));
    }
    env
}

// rook_core wrap.rs: only_the_custodian_mints_once_per_token_and_source (EthNftWrapper.t.sol: test_onlyTheCustodianMints_onceperTokenAndSource)
#[test]
fn only_the_custodian_mints_once_per_token_and_source() {
    let mut env = with_items(1);
    assert_eq!(env.balance(&HOLDER), ONE);
    expect_err(env.wrap_nft(100, tx_hash(9), MintTo::Holder(HOLDER)), BooksError::AlreadyHeld);
    expect_err(env.wrap_nft(1, [0u8; 32], MintTo::Holder(HOLDER)), BooksError::BadInput);
    let mut a = eth_args(2, tx_hash(2));
    a.source = [1u8; 32];
    expect_err(env.mint_item(a, MintTo::Holder(HOLDER)), BooksError::BadInput);
    env.check_ledger();
}

// rook_core wrap.rs: a_paused_listing_stops_mints_but_not_redemption (EthNftWrapper.t.sol: test_aPausedListingStopsMints_butNotRedemption)
#[test]
fn a_paused_listing_stops_mints_but_not_redemption() {
    let mut env = with_items(1);
    ok(env.b.set_paused(ID, true));
    expect_err(env.wrap_nft(7, tx_hash(7), MintTo::Holder(HOLDER)), BooksError::ListingPaused);
    ok(env.redeem(HOLDER, Destination::Eth(DEST)));
}

// rook_core wrap.rs: h11_twenty_a_day_and_the_window_slides (MintRate.t.sol: test_theDefaultIsTwentyADay_andTheTwentyFirstWaits, test_theWindowSlides)
#[test]
fn h11_twenty_a_day_and_the_window_slides() {
    let mut env = with_items(20);
    expect_err(env.wrap_nft(500, tx_hash(500), MintTo::Holder(HOLDER)), BooksError::MintRateLimited);
    env.advance_time(86_400);
    ok(env.wrap_nft(500, tx_hash(500), MintTo::Holder(HOLDER)));
}

// rook_core wrap.rs: the_mint_limit_is_set_once_lowered_at_once_raised_after_seven_days (MintRate.t.sol:
// test_onlyTheDeployerSetsIt_andInitOnlyBeforeAnyMint, test_loweringIsImmediate_raisingWaitsSevenDaysOnBothClocks)
#[test]
fn the_mint_limit_is_set_once_lowered_at_once_raised_after_seven_days() {
    let mut env = Env::new();
    expect_err(env.b.init_mint_limit(ID, 0, 0), BooksError::BadInput);
    expect_err(env.b.init_mint_limit(ID, 65, 0), BooksError::BadInput);
    ok(env.b.init_mint_limit(ID, 3, 0));
    for i in 0..3 {
        ok(env.wrap_nft(i, tx_hash(i), MintTo::Holder(USER)));
    }
    expect_err(env.wrap_nft(9, tx_hash(9), MintTo::Holder(USER)), BooksError::MintRateLimited);
    let s = env.supply();
    expect_err(env.b.init_mint_limit(ID, 5, s), BooksError::NotFresh);
    ok(env.b.propose_mint_limit(ID, 5, env.now));
    assert_eq!(env.ledger().max_mints_per_day, 3);
    expect_err(env.b.accept_mint_limit(ID, env.now), BooksError::TooEarly);
    env.advance_time(7 * 86_400);
    ok(env.b.accept_mint_limit(ID, env.now));
    assert_eq!(env.ledger().max_mints_per_day, 5);
    ok(env.b.propose_mint_limit(ID, 2, env.now));
    assert_eq!(env.ledger().max_mints_per_day, 2);
    expect_err(env.b.accept_mint_limit(ID, env.now), BooksError::WrongStage);
}

// rook_core wrap.rs: redeem_refusals (EthNftWrapper.t.sol: test_redeemRefusals; FloorModule.t.sol:
// test_ETHBackedUnitsCannotRedeemAnItemThatIsNotThere)
#[test]
fn redeem_refusals() {
    let mut env = with_items(1);
    expect_err(env.redeem(HOLDER, Destination::Eth([0u8; 20])), BooksError::BadDestination);
    expect_err(env.redeem(HOLDER, Destination::Eth(CUSTODY_EVM)), BooksError::BadDestination);
    expect_err(env.redeem(HOLDER, Destination::Taproot(BTC_BURN_ADDRESS.into())), BooksError::BadDestination);
    expect_err(env.redeem(HOLDER, Destination::Sol([5; 32])), BooksError::BadDestination);
    ok(env.redeem(HOLDER, Destination::Eth(DEST)));
    ok(env.post(FLOOR));
    env.user_with_units(400 * SOL);
    expect_err(env.redeem(USER, Destination::Eth(DEST)), BooksError::NoItem);
    env.check_ledger();
}

// rook_core wrap.rs: resolve_in_order_draws_only_items_free_at_the_burn (EthNftWrapper.t.sol: test_resolveInOrder_drawsOnlyItemsFreeAtTheBurn,
// test_noRedrawExists_C02; Wrapper.t.sol: test_aBurnOpensATicketPinnedToARoundThatDoesNotExistYet, test_ticketsResolveStrictlyInOrder,
// test_aTicketResolvesOnce)
#[test]
fn resolve_in_order_draws_only_items_free_at_the_burn() {
    let mut env = with_items(2);
    ok(env.redeem(HOLDER, Destination::Eth(DEST)));
    let t0 = env.ticket(0);
    assert_eq!(t0.stage, Stage::Pinned);
    assert_eq!(t0.round, round_at(env.now) + 20);
    ok(env.redeem(HOLDER, Destination::Burn));
    // an item wrapped after ticket 1 was burned is not drawable by tickets 0 or 1
    ok(env.wrap_nft(999, tx_hash(999), MintTo::Holder(CUSTODIAN)));
    let t1 = env.ticket(1);
    env.reach_round(t1.round);
    ok(env.prove(t1.round, [3u8; 32]));
    expect_err(env.resolve_as_is(1), BooksError::OutOfOrder);
    // no verified randomness given (rook_core: the drand account omitted)
    expect_err(env.b.resolve(ID, 0, None, env.now), BooksError::RoundNotProven);
    ok(env.resolve(0));
    ok(env.resolve(1));
    let (a, b) = (env.ticket(0).item_id, env.ticket(1).item_id);
    assert!(a != b && a < 2 && b < 2, "only the two items free at the burns");
    assert!(env.ledger().free.is_empty());
    assert_eq!(env.ledger().pending.len(), 1);
    expect_err(env.resolve_as_is(0), BooksError::OutOfOrder);
    assert_eq!(env.ticket(0).deliver_by, env.now + DELIVERY_DEADLINE);
    env.check_ledger();
}

// rook_core wrap.rs: delivery_and_burn (EthNftWrapper.t.sol: test_deliveryAndBurn; Wrapper.t.sol: test_onlyTheCustodianRecordsTheDelivery_andTheFrogLeavesTheBooks,
// test_nothingIsDeliveredBeforeItIsResolved, test_aLateDeliveryIsStillRecorded_untilSomebodyRefunds); audit A2-09's held-marker check
#[test]
fn delivery_and_burn() {
    let mut env = with_items(2);
    ok(env.redeem(HOLDER, Destination::Eth(DEST)));
    ok(env.redeem(HOLDER, Destination::Burn));
    expect_err(env.note_sent(0, tx_hash(1)), BooksError::WrongStage);
    ok(env.resolve(0));
    ok(env.resolve(1));
    expect_err(env.record_delivery(0, [0u8; 32]), BooksError::BadInput);
    env.advance_time(10 * 86_400);
    ok(env.record_delivery(0, tx_hash(1)));
    ok(env.record_delivery(1, tx_hash(2)));
    expect_err(env.record_delivery(1, tx_hash(2)), BooksError::WrongHeldMarker);
    let l = env.ledger();
    assert_eq!((l.held, l.open_tickets, l.burned), (0, 0, 1));
    assert_eq!(env.ticket(1).destination, Destination::Burn);
    env.check_ledger();
    expect_err(env.wrap_nft(100, tx_hash(0), MintTo::Holder(HOLDER)), BooksError::AlreadyMinted);
    ok(env.wrap_nft(100, tx_hash(77), MintTo::Holder(HOLDER)));
}

// rook_core wrap.rs: resolved_tickets_refund_only_after_the_windows (EthNftWrapper.t.sol:
// test_aResolvedTicketRefundsOnlyAfterTheGraceWithTheAttestation_orTheLateWindow, …; EthNftWrapperSent.t.sol (D3-H-1))
#[test]
fn resolved_tickets_refund_only_after_the_windows() {
    let mut env = with_items(3);
    for _ in 0..3 {
        ok(env.redeem(HOLDER, Destination::Eth(DEST)));
    }
    for i in 0..3 {
        ok(env.resolve(i));
    }
    ok(env.mark_undeliverable(0));
    ok(env.note_sent(1, tx_hash(5)));
    expect_err(env.note_sent(1, [0u8; 32]), BooksError::BadInput);
    let deliver_by = env.ticket(0).deliver_by;
    env.set_time(deliver_by + REFUND_GRACE - 1);
    expect_err(env.refund(0), BooksError::TooEarly);
    env.set_time(deliver_by + REFUND_GRACE);
    ok(env.refund(0));
    expect_err(env.refund(1), BooksError::DeliverySentNotRecorded);
    expect_err(env.refund(2), BooksError::NotUndeliverable);
    env.set_time(deliver_by + LATE_DELIVERY_WINDOW);
    expect_err(env.refund(1), BooksError::DeliverySentNotRecorded);
    ok(env.refund(2));
    ok(env.mark_undeliverable(1));
    ok(env.refund(1));
    expect_err(env.refund(1), BooksError::WrongStage);
    assert_eq!(env.balance(&HOLDER), 3 * ONE);
    assert_eq!(env.ledger().pending.len(), 3, "the items go back to the set");
    env.check_ledger();
}

// rook_core wrap.rs: a_pinned_ticket_refunds_after_the_pin_timeout_and_resolution_skips_it (EthNftWrapper.t.sol:
// test_aPinnedTicketRefundsAfterThePinTimeout_andResolutionSkipsIt; MintRate.t.sol: test_aRefundIsNotAMint)
#[test]
fn a_pinned_ticket_refunds_after_the_pin_timeout_and_resolution_skips_it() {
    let mut env = with_items(2);
    ok(env.redeem(HOLDER, Destination::Eth(DEST)));
    ok(env.redeem(HOLDER, Destination::Eth(DEST)));
    env.advance_time(PIN_TIMEOUT - 1);
    expect_err(env.refund(1), BooksError::TooEarly);
    env.advance_time(1);
    let mints = env.ledger().item_mints;
    ok(env.refund(1));
    assert_eq!(env.ledger().item_mints, mints, "a refund is not a mint");
    ok(env.resolve(0));
    assert_eq!(ok(env.resolve_as_is(1)), Resolution::Skipped);
    assert_eq!(env.ledger().next_to_resolve, 2);
    ok(env.redeem(HOLDER, Destination::Eth(DEST)));
    env.advance_time(PIN_TIMEOUT);
    ok(env.refund(2));
    assert_eq!(env.ledger().next_to_resolve, 3);
    env.check_ledger();
}

/// A two-leaf membership tree: (root, proof of a, proof of b).
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

const FROG_A: &str = "6fb976ab49dcec017f1e201e84395983204ae1a7c2abf7ced0a85d692e442799i0";
const FROG_B: &str = "a1b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f90i1";
const FROG_CUSTODY: &str = "bc1p5cyxnuxmeuwuvkwfem96lqzszd02n6xdcjrs20cac6yqjjwudpxqkedrcr";
const TB_BURN: &str = "tb1p2zffkaxp5py4fdutfdsrt6t6tcrc5ks09rkfd428hlhf4n5q8tqqeygjde";
const TB_DEST: &str = "tb1p5cyxnuxmeuwuvkwfem96lqzszd02n6xdcjrs20cac6yqjjwudpxqp3mvzv";

fn frogs(root: [u8; 32], custody: &str) -> ListingKind {
    ListingKind::Inscriptions { membership_root: root, custody: custody.into() }
}

// rook_core wrap.rs: inscriptions_membership_outpoints_and_taproot_destinations (WrapperDeposit.t.sol: test_aDepositMintsOneToTheNamedAddress,
// test_aFrogOutsideTheCollectionIsRefused, test_anOutpointMintsOnceEver, test_aFrogAlreadyHeldCannotMintAgain,
// test_aDeliveredFrogMayBeDepositedAgain; Wrapper.t.sol: test_aBurnNeedsATaprootAddress)
#[test]
fn inscriptions_membership_outpoints_and_taproot_destinations() {
    let mut env = Env::bare();
    let (root, proof_a, _) = tree(FROG_A, FROG_B);
    expect_err(env.b.add_collection(ID, frogs(root, "0x1111111111111111111111111111111111111111"), [1; 32], test_params()), BooksError::BadInput);
    expect_err(env.b.add_collection(ID, frogs([0u8; 32], BTC_BURN_ADDRESS), [1; 32], test_params()), BooksError::BadInput);
    ok(env.b.add_collection(ID, frogs(root, FROG_CUSTODY), [1; 32], test_params()));
    expect_err(env.mint_item(frog_args("deadbeefi0", proof_a.clone(), tx_hash(1), 0), MintTo::Holder(HOLDER)), BooksError::NotInCollection);
    ok(env.mint_item(frog_args(FROG_A, proof_a.clone(), tx_hash(1), 0), MintTo::Holder(HOLDER)));
    assert_eq!(env.balance(&HOLDER), ONE);
    expect_err(env.mint_item(frog_args(FROG_A, proof_a.clone(), tx_hash(2), 0), MintTo::Holder(HOLDER)), BooksError::AlreadyHeld);
    ok(env.post(FLOOR));
    expect_err(env.redeem(HOLDER, Destination::Eth(DEST)), BooksError::BadDestination);
    let mut bad = BTC_BURN_ADDRESS.to_string();
    bad.replace_range(20..21, "x");
    expect_err(env.redeem(HOLDER, Destination::Taproot(bad)), BooksError::BadDestination);
    expect_err(env.redeem(HOLDER, Destination::Taproot(FROG_CUSTODY.into())), BooksError::CustodyDestination);
    ok(env.redeem(HOLDER, Destination::Taproot(BTC_BURN_ADDRESS.into())));
    ok(env.resolve(0));
    ok(env.record_delivery(0, tx_hash(3)));
    expect_err(env.mint_item(frog_args(FROG_A, proof_a.clone(), tx_hash(1), 0), MintTo::Holder(HOLDER)), BooksError::AlreadyMinted);
    ok(env.mint_item(frog_args(FROG_A, proof_a, tx_hash(1), 1), MintTo::Holder(HOLDER)));
    env.check_ledger();
}

// rook_core wrap.rs: the_registry_bitcoin_network_decides_the_taproot_prefix (2026-10-09). Canister-only, the network is the
// custody address's: a `tb1p…` custody listing takes `tb1p…` destinations and the burn, never `bc1p…`.
#[test]
fn the_registry_bitcoin_network_decides_the_taproot_prefix() {
    let (root, proof_a, proof_b) = tree(FROG_A, FROG_B);
    let mut env = Env::bare();
    ok(env.b.add_collection(ID, frogs(root, TB_BURN), [1; 32], test_params()));
    ok(env.mint_item(frog_args(FROG_A, proof_a, tx_hash(1), 0), MintTo::Holder(HOLDER)));
    ok(env.mint_item(frog_args(FROG_B, proof_b, tx_hash(2), 0), MintTo::Holder(HOLDER)));
    expect_err(env.redeem(HOLDER, Destination::Taproot(BTC_BURN_ADDRESS.into())), BooksError::BadDestination);
    let mut bad = TB_DEST.to_string();
    bad.replace_range(20..21, "x");
    expect_err(env.redeem(HOLDER, Destination::Taproot(bad)), BooksError::BadDestination);
    ok(env.redeem(HOLDER, Destination::Taproot(TB_DEST.into())));
    ok(env.redeem(HOLDER, Destination::Burn));
    assert_eq!(env.ticket(0).destination, Destination::Taproot(TB_DEST.into()));
    ok(env.resolve(0));
    ok(env.record_delivery(0, tx_hash(3)));
    env.check_ledger();
    let mut env = Env::bare();
    ok(env.b.add_collection(ID, frogs(root, TB_DEST), [1; 32], test_params()));
}

// rook_core flow.rs: resolve_cost_with_many_items (64 items, all promoted at once)
#[test]
fn resolve_cost_with_many_items() {
    let mut env = Env::new();
    ok(env.b.init_mint_limit(ID, 64, 0));
    for i in 0..64 {
        ok(env.wrap_nft(i, tx_hash(i), MintTo::Holder(HOLDER)));
    }
    ok(env.redeem(HOLDER, Destination::Burn));
    ok(env.resolve(0));
    assert_eq!((env.ledger().free.len(), env.ledger().pending.len()), (63, 0));
    env.check_ledger();
}

// New: an item bought with an advance is credited to the module in the books only; the supply on chain never sees it, and the
// module never holds more item units than SOL-backed units.
#[test]
fn module_units_never_reach_the_chain() {
    let mut env = Env::new();
    expect_err(env.wrap_nft(1, tx_hash(1), MintTo::Module), BooksError::NotEnoughBacked);
    ok(env.post(FLOOR));
    env.user_with_units(300 * SOL); // ≈ 1.5 units
    ok(env.wrap_nft(1, tx_hash(1), MintTo::Module));
    expect_err(env.wrap_nft(2, tx_hash(2), MintTo::Module), BooksError::NotEnoughBacked);
    let l = env.ledger();
    assert_eq!((l.held, l.module_units), (1, ONE));
    assert_eq!(l.expected_supply(), Some(env.supply()));
    env.check_ledger();
}

// rook_core props.rs: the_wx_ledger_holds_through_any_mix_of_deliveries_and_refunds (EthNft.invariant.t.sol /
// testFuzz_theLedgerHoldsThroughAnyMixOfDeliveriesAndRefunds)
#[test]
fn the_wx_ledger_holds_through_any_mix_of_deliveries_and_refunds() {
    for seed in [7u64, 11, 13] {
        let mut env = Env::new();
        let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15));
        ok(env.b.init_mint_limit(ID, 64, 0));
        let mut nft = 0u64;
        for _ in 0..200 {
            let w = env.ledger();
            match rng.below(7) {
                0 | 1 => {
                    nft += 1;
                    let _ = env.wrap_nft(nft, tx_hash(nft), MintTo::Holder(HOLDER));
                }
                2 => {
                    if env.balance(&HOLDER) >= ONE && w.held > w.open_tickets {
                        let d = if rng.below(2) == 0 { Destination::Burn } else { Destination::Eth([1; 20]) };
                        ok(env.redeem(HOLDER, d));
                    }
                }
                3 => {
                    if w.next_to_resolve < w.ticket_count {
                        let id = w.next_to_resolve;
                        if env.ticket(id).stage == Stage::Refunded {
                            ok(env.resolve_as_is(id));
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
                                ok(env.mark_undeliverable(id));
                            }
                        }
                    }
                }
                5 => {
                    if w.ticket_count > 0 {
                        let id = rng.below(w.ticket_count);
                        let _ = env.refund(id);
                    }
                }
                _ => env.advance_time(rng.below(4 * 86_400) as i64),
            }
            env.check_ledger();
            let w = env.ledger();
            assert!(w.held >= w.open_tickets);
            assert!(w.free.len() as u64 + w.pending.len() as u64 <= w.held);
        }
    }
}
