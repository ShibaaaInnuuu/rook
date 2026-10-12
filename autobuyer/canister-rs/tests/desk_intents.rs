//! W5 (docs/v9/canister-only-solana.md §6): the intent desk against the fake chain with its mini runtime (`desk_common`): a deposit
//! address per intent, the canister detecting finalized deposits in ONE batched read, the books operation and the transaction it signs
//! (sweep and `MintTo`, burn and pay), refunds on refusal and expiry, and the intent's states.
mod desk_common;
mod fakerpc;

use desk_common::*;
use etch_autobuyer::books::{self, ONE};
use etch_autobuyer::desk::intent::*;
use etch_autobuyer::desk::{state, OpenKind};
use etch_autobuyer::soltx;

fn mint_args(recipient: u8, min_out: Option<u64>) -> OpenKind {
    OpenKind::Mint { coll: COLL, recipient: soltx::b58_encode(&wallet(recipient)), min_out }
}

fn burn_args(recipient: u8) -> OpenKind {
    OpenKind::Burn { coll: COLL, recipient: soltx::b58_encode(&wallet(recipient)), refund_to: None, min_out: None }
}

/// The books and the chain agree (after what landed): the invariants the reconciliation reads.
fn assert_agree(w: &World) {
    let b = w.books();
    let c = b.get(COLL).unwrap();
    assert_eq!(w.supply(), c.ledger.expected_supply().unwrap(), "the wX supply is the ledger's");
    let free = w.lamports(&module()) - RENT0;
    assert!(free as u128 >= c.module.books(), "M[0] holds {free}, the books need {}", c.module.books());
    assert!(state::with(|s| !s.link.any_open()), "nothing undecided");
}

#[test]
fn a_mint_happy_path_sweeps_the_deposit_and_mints_to_the_recipient_with_the_memo() {
    let mut w = World::new();
    let o = w.open(1, mint_args(5, None)).unwrap();
    assert_eq!((o.id, o.fee, o.min_deposit), (0, FEE_LAMPORTS, MIN_MINT_LAMPORTS));
    assert_eq!(o.address, soltx::b58_encode(&intent_key(0)));
    assert!(o.token_account.is_none() && o.quote.is_some());
    assert_eq!(w.intent(0).state, State::Open);
    // nothing is signed before a deposit
    w.rounds(2);
    assert_eq!(w.signer.signed(), 0);
    // 2 SOL arrives; the next rounds detect it, mint, land it
    w.deposit_sol(&intent_key(0), 2 * SOL);
    let want = state::with(|s| etch_autobuyer::desk::quote::quote_mint(&s.books, COLL, 2 * SOL, RENT165, w.now as i64).unwrap());
    w.rounds(3);
    let i = w.intent(0);
    assert_eq!(i.state, State::Done, "{}", i.note);
    assert_eq!(i.minted_units, want.units);
    // the chain: the recipient holds the units, the supply is the units, A is drained, the fee is K_main's, M[0] got the rest
    assert_eq!(w.token(&wallet(5)), want.units);
    assert_eq!(w.supply(), want.units);
    assert_eq!(w.lamports(&intent_key(0)), 0, "the deposit address is emptied");
    assert_eq!(w.lamports(&module()), RENT0 + 2 * SOL - FEE_LAMPORTS - RENT165);
    assert_eq!(w.lamports(&me()), 10 * SOL + FEE_LAMPORTS - 5_000, "K_main: the fee in, the transaction fee out");
    assert_agree(&w);
    // its memo is on chain and in the ledger
    let l = state::with(|s| s.link.get(0).cloned().unwrap());
    assert_eq!((l.memo.as_str(), l.phase), ("rook:v1:0:mint:0:i0", etch_autobuyer::chainlink::Phase::Landed));
    let sigs = block_on(etch_autobuyer::solhome::SolIo::sol_signatures(&mut w, "none", 10)).unwrap();
    assert!(sigs.iter().any(|s| s.memo.as_deref().is_some_and(|m| m.contains(&l.memo))));
    // three signatures (K_main, K_int[0], K_coll[0]) on the one transaction
    let calls = w.signer.calls.borrow().clone();
    let mut paths: Vec<_> = calls.iter().map(|(p, _)| *p).collect();
    paths.sort();
    assert_eq!(paths, vec![etch_autobuyer::solkey::KeyPath::Main, etch_autobuyer::solkey::KeyPath::Coll(0), etch_autobuyer::solkey::KeyPath::Intent(0)]);
}

#[test]
fn a_slippage_bound_the_books_cannot_meet_returns_the_deposit_less_the_fee() {
    let mut w = World::new();
    w.open(1, mint_args(5, Some(ONE))).unwrap(); // one whole wX for ≈ 2 SOL: never
    w.deposit_sol(&intent_key(0), 2 * SOL);
    let before = w.books();
    w.rounds(3);
    let i = w.intent(0);
    assert_eq!(i.state, State::Refunded, "{}", i.note);
    assert_eq!(w.lamports(&wallet(5)), 2 * SOL - FEE_LAMPORTS);
    assert_eq!(w.lamports(&intent_key(0)), 0);
    assert_eq!((w.supply(), w.token(&wallet(5))), (0, 0), "nothing minted");
    assert_eq!(w.books(), before, "the books never moved");
    // the refund is a link with a memo, and no collection effect
    let log = etch_autobuyer::desk::api::log(0, 10);
    assert_eq!(log.len(), 1);
    assert_eq!((log[0].kind.as_str(), log[0].coll, log[0].memo.as_str()), ("refund", None, "rook:v1:-:refund:0:i0"));
}

#[test]
fn a_burn_pays_sol_from_the_module_burns_the_wx_and_closes_the_deposit_account() {
    let mut w = World::new();
    // a holder minted 5 SOL worth first
    w.open(1, mint_args(1, None)).unwrap();
    w.deposit_sol(&intent_key(0), 5 * SOL);
    w.rounds(3);
    let held = w.token(&wallet(1));
    assert!(held > 0);
    // the burn intent: the holder's wX are sent to A's token account (the site creates it)
    let o = w.open(2, burn_args(7)).unwrap();
    assert_eq!(o.token_account, Some(soltx::b58_encode(&soltx::associated_token_address(&intent_key(1), &WX))));
    w.deposit_wx(&intent_key(1), held);
    {
        // those wX come out of the holder's account: the chain's supply stays
        let ata = soltx::associated_token_address(&wallet(1), &WX);
        let mut c = w.chain.borrow_mut();
        c.accounts.get_mut(&ata).unwrap().data[64..72].copy_from_slice(&0u64.to_le_bytes());
    }
    let q = state::with(|s| etch_autobuyer::desk::quote::quote_burn(&s.books, COLL, held, w.now as i64).unwrap());
    let sol_before = w.lamports(&wallet(7));
    let me_before = w.lamports(&me());
    w.rounds(3);
    let i = w.intent(1);
    assert_eq!(i.state, State::Done, "{}", i.note);
    assert_eq!(i.paid_lamports, q.paid);
    // the recipient got the net; the wX are burned
    assert_eq!(w.lamports(&wallet(7)) - sol_before, q.paid - FEE_LAMPORTS);
    assert_eq!(w.supply(), 0);
    // audit B1-04: the close is a transaction of its own after the burn landed; the account's rent pays for it (to K_main)
    assert!(!w.chain.borrow().accounts.contains_key(&soltx::associated_token_address(&intent_key(1), &WX)), "A's token account is closed");
    assert!(!w.intent(1).close_due);
    assert_eq!(w.lamports(&me()) + 2 * 5_000, me_before + FEE_LAMPORTS + RENT165, "K_main: the fee and the rent in, two transaction fees out");
    assert_agree(&w);
}

#[test]
fn a_stale_floor_waits_and_mints_once_posted_and_goes_back_after_six_hours() {
    let mut w = World::new();
    w.open(1, mint_args(5, None)).unwrap();
    w.advance_time(4 * 3_600); // the 3 h TTL has passed: the floor is stale
    w.deposit_sol(&intent_key(0), 2 * SOL);
    w.rounds(2);
    let i = w.intent(0);
    assert_eq!(i.state, State::Funded, "{}", i.note);
    assert!(i.note.contains("Stale"), "{}", i.note);
    assert_eq!(w.signer.signed(), 0, "nothing signed while the books refuse");
    // the poster posts: the next tick mints
    w.post_floor();
    w.rounds(2);
    assert_eq!(w.intent(0).state, State::Done);
    // a second intent whose floor never comes back: after 6 h the deposit goes back
    w.advance_time(4 * 3_600);
    w.open(2, mint_args(6, None)).unwrap();
    w.deposit_sol(&intent_key(1), 2 * SOL);
    w.rounds(2);
    assert_eq!(w.intent(1).state, State::Funded);
    w.advance_time(6 * 3_600 + 10);
    w.rounds(3);
    assert_eq!(w.intent(1).state, State::Refunded, "{}", w.intent(1).note);
    assert_eq!(w.lamports(&wallet(6)), 2 * SOL - FEE_LAMPORTS);
}

#[test]
fn a_paused_collection_refuses_the_mint_at_open_and_returns_a_deposit_made_after_it_paused() {
    let mut w = World::new();
    w.open(1, mint_args(5, None)).unwrap();
    state::with_mut(|s| s.books.set_paused(COLL, true).unwrap());
    // a new mint intent is refused at the door; the open one's deposit goes back
    assert!(w.open(2, mint_args(6, None)).unwrap_err().contains("paused"));
    w.deposit_sol(&intent_key(0), 2 * SOL);
    w.rounds(3);
    assert_eq!(w.intent(0).state, State::Refunded, "{}", w.intent(0).note);
    assert_eq!(w.lamports(&wallet(5)), 2 * SOL - FEE_LAMPORTS);
    assert_eq!(w.supply(), 0);
    // burns go on while paused
    state::with_mut(|s| s.books.set_paused(COLL, false).unwrap());
}

#[test]
fn two_deposits_are_two_operations_and_the_same_deposit_is_never_minted_twice() {
    let mut w = World::new();
    w.open(1, mint_args(5, None)).unwrap();
    w.deposit_sol(&intent_key(0), SOL);
    w.rounds(3);
    let first = w.intent(0).minted_units;
    assert_eq!(w.intent(0).state, State::Done);
    // many more ticks with nothing new: nothing happens
    w.rounds(4);
    assert_eq!((w.intent(0).minted_units, w.intent(0).done_ops), (first, 1));
    assert_eq!(state::with(|s| s.link.links.len()), 1, "one link, one transaction");
    // a second deposit (the same address, repeatable) is the next operation
    w.deposit_sol(&intent_key(0), SOL);
    w.rounds(3);
    let i = w.intent(0);
    assert_eq!((i.state, i.done_ops), (State::Done, 2), "{}", i.note);
    assert!(i.minted_units > first && w.token(&wallet(5)) == i.minted_units);
    assert_eq!(state::with(|s| s.link.links.len()), 2);
    assert_agree(&w);
    // a deposit under the minimum waits, and is returned at expiry
    w.deposit_sol(&intent_key(0), 30_000_000);
    w.rounds(2);
    assert_eq!(w.intent(0).state, State::Done);
    assert!(w.intent(0).note.contains("below the minimum"));
    assert_eq!(w.lamports(&intent_key(0)), 30_000_000);
    w.advance_time(TTL_SECS + 10);
    w.rounds(3);
    assert_eq!(w.lamports(&intent_key(0)), 0, "returned: {}", w.intent(0).note);
    assert_eq!(w.intent(0).state, State::Refunded);
}

#[test]
fn a_deposit_after_expiry_is_returned_never_minted() {
    let mut w = World::new();
    w.open(1, mint_args(5, None)).unwrap();
    w.advance_time(TTL_SECS + 5);
    w.rounds(2);
    assert_eq!(w.intent(0).state, State::Expired);
    // the address is still watched for a week: the money coming late goes back
    w.deposit_sol(&intent_key(0), 3 * SOL);
    w.rounds(3);
    let i = w.intent(0);
    assert_eq!(i.state, State::Refunded, "{}", i.note);
    assert_eq!(w.lamports(&wallet(5)), 3 * SOL - FEE_LAMPORTS);
    assert_eq!((w.supply(), w.token(&wallet(5))), (0, 0));
    // past the grace, an address is no longer read
    w.advance_time(REFUND_GRACE_SECS + TTL_SECS);
    w.deposit_sol(&intent_key(0), 3 * SOL);
    w.rounds(2);
    assert_eq!(w.lamports(&intent_key(0)), 3 * SOL, "left alone");
}

#[test]
fn ten_open_intents_are_read_in_one_batched_request() {
    let mut w = World::new();
    for n in 0..9u8 {
        w.open(n, mint_args(n + 10, None)).unwrap();
    }
    w.open(20, burn_args(30)).unwrap();
    // a first tick (the first reconciliation reads the chain too), then the next one is measured
    w.tick();
    let asked = w.ps[0].asked.clone();
    asked.borrow_mut().clear();
    w.tick();
    let reads: Vec<String> = asked.borrow().iter().filter(|m| m.starts_with("getMultipleAccounts")).cloned().collect();
    // both providers answer one request: two entries, not twenty
    assert_eq!(reads.len(), 2, "{reads:?}");
}

#[test]
fn the_limits_hold_per_recipient_per_caller_and_in_all() {
    let mut w = World::new();
    for _ in 0..MAX_PER_RECIPIENT {
        w.open(1, mint_args(5, None)).unwrap();
    }
    assert!(w.open(1, mint_args(5, None)).unwrap_err().contains("already open for this recipient"));
    // audit B1-09: another caller's intents for the same recipient are their own: a stranger cannot lock a wallet out
    w.open(2, mint_args(5, None)).unwrap();
    // five an hour per caller
    let mut w = World::new();
    for n in 0..MAX_PER_CALLER_HOUR as u8 {
        w.open(9, mint_args(n + 40, None)).unwrap();
    }
    assert!(w.open(9, mint_args(99, None)).unwrap_err().contains("an hour per caller"));
    w.advance_time(3_700);
    w.open(9, mint_args(99, None)).unwrap();
    // bad input
    assert!(w.open(1, OpenKind::Mint { coll: 7, recipient: soltx::b58_encode(&wallet(1)), min_out: None }).is_err());
    assert!(w.open(1, OpenKind::Mint { coll: COLL, recipient: "nope".into(), min_out: None }).is_err());
    assert!(w.open(1, OpenKind::RedeemCoin { coin: soltx::b58_encode(&wallet(1)), coll: COLL, recipient: soltx::b58_encode(&wallet(1)), refund_to: None })
        .unwrap_err().contains("not enabled"));
}

#[test]
fn a_redemption_burns_one_wx_pins_a_ticket_and_returns_the_rest() {
    let mut w = World::new();
    // two items are held (holder deposits): two whole wX exist on chain for them, booked
    state::with_mut(|s| {
        for k in [1u8, 2] {
            let args = books::ledger::MintItemArgs { key: [k; 32], source: books::math::keccak(&[&[k; 32], &[3; 32]]), tx: [3; 32], vout: 0, deposit: false,
                inscription_id: String::new(), proof: vec![] };
            s.books.mint_item(COLL, args, books::MintTo::Holder(wallet(1)), w.now as i64).unwrap();
        }
    });
    w.inject_supply(2 * ONE as i64);
    let dest = format!("0x{}", "ab".repeat(20));
    w.open(1, OpenKind::RedeemNft { coll: COLL, destination: dest, owner: soltx::b58_encode(&wallet(1)) }).unwrap();
    // 1.5 of them are sent: one is burned for the ticket, the half goes back to the owner
    w.deposit_wx(&intent_key(0), ONE + ONE / 2);
    w.rounds(5);
    let i = w.intent(0);
    assert_eq!(i.ticket, Some(0), "{}", i.note);
    assert_eq!(state::with(|s| s.books.get(COLL).unwrap().ledger.open_tickets), 1);
    assert_eq!(w.supply(), ONE, "one wX burned of two");
    assert_eq!(w.token(&wallet(1)), ONE / 2);
    assert_eq!(i.state, State::Refunded, "{}", i.note);
    assert!(i.done_ops == 1);
}

/// The bytes the monitor decodes (monitor/test/books.test.mjs): `WRITE_FIXTURES=1` rewrites them.
fn view_fixture(file: &str, bytes: &[u8]) {
    let path = format!("{}/tests/fixtures/views/{file}", env!("CARGO_MANIFEST_DIR"));
    let hex = hex::encode(bytes);
    if std::env::var("WRITE_FIXTURES").is_ok() {
        std::fs::write(&path, format!("{hex}\n")).unwrap();
    }
    assert_eq!(std::fs::read_to_string(&path).unwrap().trim(), hex, "{file} is stale: WRITE_FIXTURES=1, then the monitor's tests");
}

#[test]
fn the_desk_status_bytes_are_the_monitors_fixture() {
    let mut w = World::new();
    w.open(1, mint_args(5, None)).unwrap();
    w.tick();
    state::with_mut(|s| {
        s.link.halt(&mut s.books, COLL, "pre_sign refused: SupplyMismatch", 1_700_000_000);
        s.last_reconcile = 1_700_000_000;
    });
    let v = etch_autobuyer::desk::api::status();
    view_fixture("desk-status.hex", &candid::encode_one(&v).unwrap());
}

#[test]
fn the_books_view_bytes_are_the_reconcilers_fixture() {
    let w = World::new();
    let now = w.now as i64;
    state::with_mut(|s| {
        let args = books::ledger::MintItemArgs { key: [1; 32], source: books::math::keccak(&[&[1; 32], &[3; 32]]), tx: [3; 32], vout: 0, deposit: false,
            inscription_id: String::new(), proof: vec![] };
        s.books.mint_item(COLL, args, books::MintTo::Holder(wallet(1)), now).unwrap();
        let args = books::ledger::MintItemArgs { key: [2; 32], source: books::math::keccak(&[&[2; 32], &[3; 32]]), tx: [3; 32], vout: 0, deposit: false,
            inscription_id: String::new(), proof: vec![] };
        s.books.mint_item(COLL, args, books::MintTo::Holder(wallet(1)), now).unwrap();
        s.books.redeem(COLL, wallet(1), books::ledger::Destination::Eth([0xe1; 20]), now).unwrap();
        let m = s.books.mint(COLL, SOL, 0, now).unwrap();
        s.books.collections.get_mut(&COLL).unwrap().ledger.sol_backed += m.units;
    });
    let v = etch_autobuyer::desk::api::books_of(COLL, now).unwrap();
    assert_eq!((v.held, v.open_tickets, v.items.len(), v.tickets_open.len()), (2, 1, 2, 1));
    assert_eq!(v.expected_supply, Some(books::ONE + v.sol_backed));
    assert_eq!(v.owed, v.backing + v.reserve + v.surplus + v.revenue);
    assert!(v.fresh && v.mint_price.is_some());
    view_fixture("books-view.hex", &candid::encode_one(&v).unwrap());
    assert_eq!(etch_autobuyer::desk::api::books_collections(), vec![(COLL, soltx::b58_encode(&WX))]);
}
