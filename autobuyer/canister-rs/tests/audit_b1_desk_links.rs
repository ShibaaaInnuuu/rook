//! Audit B1 (docs/v9/audit-2026-10/b1-desk-links.md): proofs of concept against the intent desk (`src/desk`) and the chain links
//! (`src/chainlink`) at `e4d4d34`. Each `poc_` test PASSES by exhibiting its finding; a `fixed_` test is a PoC inverted after its fix on the current code; the comment above each says what a
//! fix must make it refuse (the test is then inverted and kept as the regression).
mod desk_common;
mod fakerpc;

use desk_common::*;
use etch_autobuyer::books::{self, ListingKind};
use etch_autobuyer::chainlink::ledger::OwedWrite;
use etch_autobuyer::chainlink::recon::FindingKind;
use etch_autobuyer::chainlink::{driver, Phase};
use etch_autobuyer::desk::intent::*;
use etch_autobuyer::desk::{state, OpenKind};
use etch_autobuyer::solkey::KeyPath;
use etch_autobuyer::solrpc::Account;
use etch_autobuyer::soltx::{self, Pubkey, SYSTEM_PROGRAM};
use serde_json::json;

fn mint_args(coll: u16, recipient: Pubkey, min_out: Option<u64>) -> OpenKind {
    OpenKind::Mint { coll, recipient: soltx::b58_encode(&recipient), min_out }
}

fn burn_args(recipient: Pubkey, refund_to: Option<Pubkey>, min_out: Option<u64>) -> OpenKind {
    OpenKind::Burn { coll: COLL, recipient: soltx::b58_encode(&recipient), refund_to: refund_to.map(|k| soltx::b58_encode(&k)), min_out }
}

/// The amount of `owner`'s wX token account on the fake chain, set as a holder's own transaction would leave it.
fn set_token_amount(w: &mut World, owner: &Pubkey, amount: u64) {
    let ata = soltx::associated_token_address(owner, &WX);
    let mut c = w.chain.borrow_mut();
    c.accounts.get_mut(&ata).expect("a token account").data[64..72].copy_from_slice(&amount.to_le_bytes());
}

/// `owner` closes their (emptied) wX token account: its rent is theirs.
fn close_token_account(w: &mut World, owner: &Pubkey) {
    let ata = soltx::associated_token_address(owner, &WX);
    let mut c = w.chain.borrow_mut();
    let a = c.accounts.remove(&ata).expect("a token account");
    c.accounts.entry(*owner).or_insert(Account { lamports: 0, owner: SYSTEM_PROGRAM, executable: false, data: vec![] }).lamports += a.lamports;
}

fn halted() -> bool {
    state::with(|s| s.link.halted.contains_key(&COLL))
}

/// A mint of 2 SOL to `recipient` by caller `caller`, landed: the intent's id.
fn minted(w: &mut World, caller: u8, recipient: Pubkey, lamports: u64) -> u64 {
    let id = w.open(caller, mint_args(COLL, recipient, None)).unwrap().id;
    w.deposit_sol(&intent_key(id), lamports);
    w.rounds(3);
    assert_eq!(w.intent(id).state, State::Done, "{}", w.intent(id).note);
    id
}

// ---- B1-01 (High) ---------------------------------------------------------------------------------------------------------------

/// B1-01: any holder may `Burn` their own wX on chain (SPL `Burn` needs only the token account owner's signature). The chain's supply
/// is then one base unit UNDER the ledger's. `books::supply_matches` is exact, so the next `pre_sign` of the collection refuses with
/// `SupplyMismatch` and HALTS it (`sign_recorded`), and the reconciliation strikes it; no operator method books a holder's burn, so the
/// operator's `clear_halt` only lasts until the next operation. Mint, burn and redemption at the desk, and the treasury's links, are
/// stopped for good: a permanent, free denial of service by anyone holding dust of the collection's wX.
///
/// A fix must make this test refuse: after a holder's burn the second mint must land (`State::Done`), nothing is halted, and the
/// reconciliation reports the shortfall as a soft finding (or books it), never as a hard one.
/// FIXED (`supply_matches` accepts a chain UNDER the ledger; the reconciliation books the difference as `holder_burned`): after a
/// holder's own burn the next mint lands, nothing is halted, the reconciliation files one soft finding and books it, then agrees.
#[test]
fn fixed_b1_01_a_holders_own_burn_on_chain_is_booked_and_stops_nothing() {
    let mut w = World::new();
    minted(&mut w, 1, wallet(5), 5 * SOL);
    let held = w.token(&wallet(5));
    assert!(held > 1);
    // the holder burns ONE base unit (1e-9 wX) of their own wX: owner-signed, no canister key involved
    set_token_amount(&mut w, &wallet(5), held - 1);
    w.inject_supply(-1);
    assert_eq!(w.supply() + 1, w.books().get(COLL).unwrap().ledger.expected_supply().unwrap(), "the chain is one under the ledger");
    // an unrelated user's mint lands: the gate tolerates a supply under the ledger's
    w.open(2, mint_args(COLL, wallet(6), None)).unwrap();
    w.deposit_sol(&intent_key(1), SOL);
    w.rounds(3);
    assert!(!halted(), "{:?}", state::with(|s| s.link.halted.clone()));
    assert_eq!(w.intent(1).state, State::Done, "{}", w.intent(1).note);
    assert!(w.token(&wallet(6)) > 0);
    // the reconciliation: one SOFT finding, booked at once as the ledger's holder_burned; the next pass agrees
    let now = w.now;
    let r = block_on(driver::reconcile(&mut w, now));
    assert!(r.findings.iter().any(|f| f.coll == COLL && f.kind == FindingKind::HolderBurn && f.books - f.chain == 1), "{:?}", r.findings);
    assert!(!r.findings.iter().any(|f| f.hard()), "{:?}", r.findings);
    assert!(r.halted.is_empty());
    assert_eq!(w.books().get(COLL).unwrap().ledger.holder_burned, 1);
    assert_eq!(w.books().get(COLL).unwrap().ledger.expected_supply().unwrap(), w.supply(), "the ledger's supply is the chain's again");
    let r = block_on(driver::reconcile(&mut w, now + 1));
    assert!(r.findings.is_empty(), "{:?}", r.findings);
    // a supply OVER the ledger's (a mint the books never made) is still the hard finding it was
    w.inject_supply(2);
    let r = block_on(driver::reconcile(&mut w, now + 2));
    assert!(r.findings.iter().any(|f| f.coll == COLL && f.kind == FindingKind::SupplyMismatch && f.hard()), "{:?}", r.findings);
    w.inject_supply(-2);
    // the holders' exit stays open: a burn of the rest lands
    w.open(4, burn_args(wallet(5), None, None)).unwrap();
    w.deposit_wx(&intent_key(2), held - 1);
    set_token_amount(&mut w, &wallet(5), 0);
    w.rounds(4);
    assert!(!halted());
    assert_eq!(w.intent(2).state, State::Done, "{}", w.intent(2).note);
}

// ---- B1-02 (High) ---------------------------------------------------------------------------------------------------------------

/// B1-02: a refund of a TOKEN deposit (`build::refund_token_tx`) charges no desk fee and has `K_main` pay for the destination's token
/// account (`create_ata_idempotent(main, to, mint)`: 2,039,280 lamports when it is absent), the transaction fee and two threshold
/// signatures. A burn intent whose `min_out` can never be met is refused at once (`Slippage`, not transient) and refunded to
/// `refund_to`, a wallet of the attacker's choice. The attacker then closes the token account they were given (its rent is theirs),
/// sends the wX back to the same intent, and the same intent refunds again: ≈ 0.002 SOL of `K_main` plus the signatures per cycle,
/// no rate limit (one intent, no new opening), bounded only by the desk line's pace.
///
/// A fix must make this test refuse: `K_main`'s balance must not fall by the destination's rent (refund only into an existing token
/// account, or charge the rent and the fee to the deposit), and a second refund to the same wallet must not cost `K_main` again.
/// FIXED (`refund_token_tx` closes `A`'s token account to `K_main`: its rent is the token refund's fee and covers the destination
/// account `K_main` creates): a refund costs `K_main` the transaction fee only, every cycle costs the depositor `A`'s rent.
/// Since B1-04 the close is a transaction of its own after the refund landed (still to `K_main`): `K_main` pays two transaction fees.
#[test]
fn fixed_b1_02_a_token_refund_is_paid_by_the_deposits_own_rent() {
    let mut w = World::new();
    minted(&mut w, 1, wallet(1), 5 * SOL);
    let held = w.token(&wallet(1));
    assert!(held >= MIN_BURN_UNITS);
    // a burn whose bound can never be met; its refund goes to a fresh wallet with no token account
    let fresh = wallet(9);
    let fresh_ata = soltx::associated_token_address(&fresh, &WX);
    w.open(2, burn_args(wallet(1), Some(fresh), Some(u64::MAX))).unwrap();
    let a = intent_key(1);
    w.deposit_wx(&a, MIN_BURN_UNITS);
    set_token_amount(&mut w, &wallet(1), held - MIN_BURN_UNITS);
    let me_before = w.lamports(&me());
    w.rounds(3);
    assert_eq!(w.intent(1).state, State::Refunded, "{}", w.intent(1).note);
    // the wX are at `fresh`, in a token account K_main created; A's rent came to K_main, not to the refund's destination
    assert_eq!(w.token(&fresh), MIN_BURN_UNITS);
    assert_eq!(w.lamports(&fresh_ata), RENT165);
    assert_eq!(w.lamports(&fresh), 0, "A's token account rent is the refund's fee, K_main's");
    assert_eq!(me_before - w.lamports(&me()), 2 * 5_000, "K_main paid the refund's and the close's fees only: the destination's rent was A's");
    // the cycle: close the token account, send the wX back to the SAME intent (a new A, the depositor's rent): refunded again at the
    // same cost to K_main, and A's rent again to it
    close_token_account(&mut w, &fresh);
    assert_eq!(w.lamports(&fresh), RENT165);
    w.deposit_wx(&a, MIN_BURN_UNITS);
    let me_mid = w.lamports(&me());
    w.rounds(3);
    assert_eq!(w.intent(1).state, State::Refunded, "{}", w.intent(1).note);
    assert_eq!(me_mid - w.lamports(&me()), 2 * 5_000, "K_main pays the fees only, every cycle; the depositor pays A's rent every cycle");
    assert_eq!(w.lamports(&fresh_ata), RENT165);
    assert_eq!(w.lamports(&fresh), RENT165, "the destination account's rent was theirs once; A's rent never");
}

// ---- B1-03 (Medium) -------------------------------------------------------------------------------------------------------------

const WX1: Pubkey = [0xC1; 32];

/// A second collection (id 1, wX mint [`WX1`], module `K_coll[1]`) on the chain and in the books, with a fresh floor.
fn add_collection_1(w: &mut World) {
    let m1 = pk(KeyPath::Coll(1));
    w.plant(m1, SYSTEM_PROGRAM, RENT0, vec![]);
    w.plant(WX1, soltx::key(soltx::TOKEN_PROGRAM), 1_461_600, mint_data(m1, 0));
    let now = w.now as i64;
    state::with_mut(|s| {
        s.books.add_collection(1, ListingKind::EthNft { chain_id: 1, collection: [0x5b; 20], custody: [0x12; 20] }, WX1, books::default_params(20 * SOL)).unwrap();
        s.books.post_floor(1, FLOOR, 9_000, 20, now).unwrap();
    });
}

/// B1-03: `execute_next` tries the `TRIES_PER_TICK` (5) oldest funded intents, in `(funded_at, id)` order, and gives up. Five intents
/// waiting on a transient refusal (one collection's floor stale, its poster paused, its hourly bucket, its cap) are always the five
/// oldest, so every other collection's funded intents are never reached, for the 6 h until the five are refunded; then the next five.
/// No attacker is needed: five users of one stale collection stop the desk for the rest.
///
/// A fix must make this test refuse: collection 1's intent must mint (`State::Done`) while collection 0's five wait.
/// FIXED (the pass tries the never-skipped intents first, then the least recently skipped: `Intent::skipped_at`; still at most five
/// tries a tick): the five are tried and skipped once, collection 1's intent mints at the next tick while they wait, and they mint in
/// turn once their floor is posted.
#[test]
fn fixed_b1_03_five_waiting_intents_of_one_collection_no_longer_hold_another_collections_intent() {
    let mut w = World::new();
    add_collection_1(&mut w);
    // collection 0's floor goes stale (no post in 3 h); collection 1's is fresh
    w.advance_time(4 * 3_600);
    let now = w.now as i64;
    state::with_mut(|s| s.books.post_floor(1, FLOOR, 9_000, 20, now).unwrap());
    for n in 0..5u8 {
        w.open(n + 1, mint_args(COLL, wallet(10 + n), None)).unwrap();
        w.deposit_sol(&intent_key(n as u64), 2 * SOL);
    }
    w.open(7, mint_args(1, wallet(20), None)).unwrap();
    w.deposit_sol(&intent_key(5), 2 * SOL);
    w.rounds(4);
    for n in 0..5u64 {
        let i = w.intent(n);
        assert_eq!(i.state, State::Funded, "{}", i.note);
        assert!(i.note.contains("Stale"), "{}", i.note);
        assert!(i.skipped_at.is_some(), "tried, and sent to the back");
    }
    let i = w.intent(5);
    assert_eq!(i.state, State::Done, "collection 1's intent is reached while collection 0's five wait: {}", i.note);
    assert!(i.minted_units > 0);
    assert_eq!(w.supply(), 0, "collection 0's supply is untouched");
    // the five keep their turn: once the floor is posted they mint, one a round, oldest first
    w.post_floor();
    w.rounds(6);
    for n in 0..5u64 {
        assert_eq!(w.intent(n).state, State::Done, "{}", w.intent(n).note);
    }
}

// ---- B1-04 (Medium) -------------------------------------------------------------------------------------------------------------

/// B1-04: a burn, a redemption's leftover refund and a token refund end with `CloseAccount` of the intent's token account, which Solana
/// refuses unless the account is empty. The transaction burns or moves the amount READ at finalized before it was signed; any wX sent to
/// `A`'s token account by a third party between that read and the landing (one base unit is enough; the address is public through
/// `intent_status`) makes the whole transaction land FAILED. Three failures put the deposit on the refund path; the refund is the same
/// transaction shape, fails the same way, and five failures leave the intent to the operator. Each failed landing costs `K_main` a fee.
///
/// A fix must make this test refuse: the burn must land (`State::Done`) with the dust left behind at `A`, or the close must be a
/// separate, later step.
/// FIXED (`burn_tx` and `refund_token_tx` no longer close `A`'s token account; `close_tx` does, as a transaction of its own after the
/// operation landed, only when finalized shows the account empty, given up after `MAX_CLOSE_TRIES`): the dusted burn lands at once, the
/// dust stays at `A` with the account's rent, and a dusted token refund lands too.
#[test]
fn fixed_b1_04_a_dust_transfer_to_the_intents_token_account_no_longer_fails_the_burn_or_its_refund() {
    let mut w = World::new();
    minted(&mut w, 1, wallet(1), 5 * SOL); // the victim
    minted(&mut w, 2, wallet(2), 3 * SOL); // the stranger: a little wX of their own to dust with
    let held = w.token(&wallet(1));
    let strangers = std::cell::Cell::new(w.token(&wallet(2)));
    w.open(3, burn_args(wallet(7), None, None)).unwrap();
    let a = intent_key(2);
    w.deposit_wx(&a, held);
    set_token_amount(&mut w, &wallet(1), 0);
    let supply = w.supply();
    let me_before = w.lamports(&me());
    let sol_before = w.lamports(&wallet(7));
    // one base unit of the stranger's wX to an intent's token account
    let dust = |w: &mut World, to: &Pubkey| {
        strangers.set(strangers.get() - 1);
        set_token_amount(w, &wallet(2), strangers.get());
        w.deposit_wx(to, 1);
    };
    w.tick(); // the deposit is found and the burn signed (simulated against an exact balance)
    let i = w.intent(2);
    assert_eq!((i.state, i.op.map(|o| o.purpose)), (State::Executing, Some(Purpose::Execute)), "{}", i.note);
    dust(&mut w, &a); // the stranger's base unit lands first
    w.settle();
    w.tick(); // the burn LANDED: nothing in it needs the account empty
    let i = w.intent(2);
    assert_eq!(i.state, State::Done, "{}", i.note);
    assert_eq!(i.attempts, 0);
    assert_eq!(w.supply(), supply - held, "the deposit is burned");
    assert_eq!(w.lamports(&wallet(7)) - sol_before, i.paid_lamports - FEE_LAMPORTS, "the recipient is paid");
    // the close finds the dust, twice, and leaves the account open: the dust and the rent stay at A, no transaction is spent on it
    w.rounds(2);
    let i = w.intent(2);
    assert!(!i.close_due && i.close_tries == MAX_CLOSE_TRIES, "{}", i.note);
    assert_eq!(w.token(&a), 1);
    assert_eq!(w.lamports(&soltx::associated_token_address(&a, &WX)), RENT165, "the rent stays at A");
    assert_eq!(w.lamports(&me()), me_before + FEE_LAMPORTS - 5_000, "one landing, one transaction fee paid by K_main");
    // a token refund (a burn whose bound can never be met) dusted the same way lands too
    w.open(4, burn_args(wallet(8), Some(wallet(1)), Some(u64::MAX))).unwrap();
    let a3 = intent_key(3);
    strangers.set(strangers.get() - MIN_BURN_UNITS);
    set_token_amount(&mut w, &wallet(2), strangers.get());
    w.deposit_wx(&a3, MIN_BURN_UNITS);
    w.tick(); // the burn is refused (Slippage): the deposit goes back
    w.tick(); // the refund is signed
    let i = w.intent(3);
    assert_eq!((i.state, i.op.map(|o| o.purpose)), (State::Executing, Some(Purpose::Refund)), "{}", i.note);
    dust(&mut w, &a3);
    w.settle();
    w.tick();
    let i = w.intent(3);
    assert_eq!((i.state, i.refund_attempts), (State::Refunded, 0), "{}", i.note);
    assert_eq!(w.token(&wallet(1)), MIN_BURN_UNITS, "the deposit went back");
    assert_eq!(w.token(&a3), 1, "the dust stayed");
}

// ---- B1-05 (Medium) -------------------------------------------------------------------------------------------------------------

/// B1-05: opening an intent is free, anonymous callers are accepted at the ingress (`is_public`), self-authenticating principals cost
/// nothing to make, recipients are any 32 bytes. 300 openings (60 principals × 5 an hour, 3 a recipient; or 2.5 h of anonymous calls)
/// fill `MAX_OPEN`: every other user is refused for the 24 h until they expire, and again after. Expired intents stay WATCHED for
/// `REFUND_GRACE_SECS` (7 days): the batched reads grow by one `getMultipleAccounts` per 100 (per provider, per tick) beyond anything
/// the 300 cap bounds.
///
/// A fix must make this test refuse: a free opening must not take a slot from a funded user (a deposit, a stake, or a far smaller
/// free cap), and the watched set must be bounded (never-funded intents watched briefly after expiry, or rotated through one read).
/// FIXED (intents that never saw a deposit have their own cap, `MAX_UNFUNDED_OPEN`, and never count toward `MAX_OPEN`; an hour after
/// expiry a never-funded intent leaves the tick's batch for a rotation of one `getMultipleAccounts` a tick, `DeskState::rotation`):
/// free openings stop at their cap, a user's intent still funds and mints, and three days of refilled openings cost one read a tick
/// more, not one per hundred; a late deposit at the oldest of them is still found and returned. Funded intents keep the 7-day grace.
#[test]
fn fixed_b1_05_free_openings_have_their_own_cap_and_expired_ones_are_read_in_rotation() {
    let mut w = World::new();
    // a user opens first, as a user does; the attacker's free openings then fill the free cap
    let user = w.open(130, mint_args(COLL, wallet(120), None)).unwrap().id;
    for n in 0..MAX_UNFUNDED_OPEN - 1 {
        w.open((n / 5) as u8, mint_args(COLL, wallet((n / 3) as u8), None)).unwrap();
    }
    assert!(w.open(131, mint_args(COLL, wallet(121), None)).unwrap_err().contains("waiting for a deposit"), "the free cap holds");
    assert_eq!(w.signer.signed(), 0, "the attacker deposited nothing");
    // the user's intent funds and mints: free openings never take the slots of intents with a deposit
    w.deposit_sol(&intent_key(user), 2 * SOL);
    w.rounds(3);
    assert_eq!(w.intent(user).state, State::Done, "{}", w.intent(user).note);
    // three days of refilled free openings
    for _ in 0..3 {
        w.advance_time(TTL_SECS + UNFUNDED_GRACE_SECS + 10);
        w.tick();
        for n in 0..MAX_UNFUNDED_OPEN {
            w.open((n / 5) as u8, mint_args(COLL, wallet((n / 3) as u8), None)).unwrap();
        }
    }
    let now = w.now;
    let (watched, rotated) = state::with(|s| (s.intents.values().filter(|i| i.watched(now)).count(), s.intents.values().filter(|i| i.rotated(now)).count()));
    assert_eq!((watched, rotated), (1 + 4 * MAX_UNFUNDED_OPEN - 1, 3 * MAX_UNFUNDED_OPEN - 1), "every address is still watched, most in rotation");
    let asked = w.ps[0].asked.clone();
    asked.borrow_mut().clear();
    w.tick();
    let reads = asked.borrow().iter().filter(|m| m.starts_with("getMultipleAccounts")).count();
    // the batch: the 200 live free openings and the user's (funded, in its 7-day grace) = 3 chunks; the rotation: 1; both providers
    assert_eq!(reads, 2 * (3 + 1), "not one read per hundred expired openings");
    // a late deposit at the oldest expired opening is found by the rotation and returned
    w.deposit_sol(&intent_key(1), 3 * SOL);
    for _ in 0..rotated.div_ceil(100) {
        w.tick();
    }
    w.rounds(2);
    assert_eq!(w.intent(1).state, State::Refunded, "{}", w.intent(1).note);
    assert_eq!(w.lamports(&wallet(0)), 3 * SOL - FEE_LAMPORTS);
}

// ---- B1-06 (Medium) -------------------------------------------------------------------------------------------------------------

/// B1-06: review F1 made every non-link writer WAIT on `collection_busy` at its decision (`home_books::send`, `frogsol::books::send`,
/// the cranks' distribute / resolve / raffle). The writers at a LINE OUTCOME do not: `ethflow::home_books::landed` (`return_advance`
/// of an advance that never landed, `fund_reserve` / `return_advance` of a way back that landed), `frogsol::books::landed` (the same,
/// and `LateCredit`), `cranks::settle` (`claim_raffle` of a landed prize claim). They run inside `looper::step`, BEFORE the desk's pass,
/// while a desk link signed in the previous tick is still open. A link's undo restores the collection's snapshot taken at `begin`, so
/// such a write is wiped when the desk's transaction fails or never lands: a way back's lamports are at `M[i]` and not in the books, or a
/// unit stays in transit for ever (`AdvanceOpen`: the NFT pipeline of the collection stops). Here the outcome-side write is stood in by
/// `fund_reserve`, the call `home_books::landed` makes for a landed way back.
///
/// A fix must make this test refuse: the write made under the open link survives the link's undo (an inverse-delta undo, or the
/// outcome-side writers deferred while `collection_busy`).
/// FIXED (the outcome-side writers go through `desk::state::book_outcome`: under an open link the write is owed by the link ledger,
/// `Ledger::owed`, saved with the desk state, and applied once the link is decided, on top of its undo): the way back is booked after
/// the failed mint's undo, and the owed write survives an upgrade in between.
#[test]
fn fixed_b1_06_a_book_write_at_a_line_outcome_under_an_open_desk_link_survives_the_links_undo() {
    let mut w = World::new();
    w.open(1, mint_args(COLL, wallet(5), None)).unwrap();
    w.deposit_sol(&intent_key(0), 2 * SOL);
    w.tick(); // found and signed: the mint link is open, its snapshot taken at `begin`
    assert_eq!(w.intent(0).state, State::Executing);
    assert!(state::collection_busy(COLL));
    // the money line reports a way back landed: `home_books::landed` books it through `book_outcome`, owed while the link is open
    let way_back = SOL;
    state::book_outcome(COLL, OwedWrite::FundReserve { lamports: way_back }).unwrap();
    assert_eq!(w.books().get(COLL).unwrap().module.surplus, 0, "not written under the open link");
    assert_eq!(state::with(|s| s.link.owed_of(COLL).to_vec()), vec![OwedWrite::FundReserve { lamports: way_back }]);
    // an upgrade in between: the owed write is in the saved desk state
    let bytes = state::encode();
    state::reset();
    state::restore(&bytes).unwrap();
    assert_eq!(state::with(|s| s.link.owed_of(COLL).len()), 1, "kept across the upgrade");
    // the desk's transaction lands FAILED on chain: its link is undone from the snapshot, then the owed write goes in
    w.chain.borrow_mut().fail_next = Some(json!({ "InstructionError": [1, { "Custom": 1 }] }));
    w.settle();
    w.tick();
    assert_eq!(state::with(|s| s.link.get(0).unwrap().phase), Phase::Failed);
    assert_eq!(w.intent(0).attempts, 1);
    assert_eq!(w.books().get(COLL).unwrap().module.surplus, way_back, "the way back's booking survives the desk link's undo");
    assert!(state::with(|s| s.link.owed_of(COLL).is_empty()));
}
