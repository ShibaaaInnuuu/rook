//! Audit B1 (docs/v9/audit-2026-10/b1-desk-links.md), the Lows fixed after the report: B1-11 (a stuck link resolved by the operator,
//! the chain asked first), B1-12 (the memo scan pages back to its cursor, bounded, and says when it could not), B1-08 (a planted
//! system account at the recipient's token address does not block the mint) and B1-10 (no sweep while the money line has a step out
//! or an outcome not yet booked).
mod desk_common;
mod fakerpc;

use std::collections::BTreeMap;

use desk_common::*;
use etch_autobuyer::chainlink::{driver, Phase};
use etch_autobuyer::desk::intent::State;
use etch_autobuyer::desk::{state, OpenKind};
use etch_autobuyer::solhome::{LineId, SolLine};
use etch_autobuyer::solkey::KeyPath;
use etch_autobuyer::solrpc::SigInfo;
use etch_autobuyer::soltx::{self, Pubkey};

fn open_link_id() -> u64 {
    state::with(|s| s.link.open_of(COLL, None).next().map(|l| l.id)).expect("an open link")
}

/// FIXED B1-11: a signed link whose line holds nothing and whose signature the chain does not know is `Stuck`; the operator's
/// `desk_resolve_link` asks the chain first (a finalized verdict wins), takes "undone" on their word when the chain knows nothing,
/// and never takes "landed" without the chain.
#[test]
fn fixed_b1_11_a_stuck_link_is_resolved_by_the_operator_with_the_chain_asked_first() {
    let mut w = World::new();
    let books_before = w.books().get(COLL).unwrap().module.clone();
    w.open(1, OpenKind::Mint { coll: COLL, recipient: soltx::b58_encode(&wallet(6)), min_out: None }).unwrap();
    w.deposit_sol(&intent_key(0), SOL);
    w.tick(); // found and signed: the link is out, the chain has not finalized it
    let id = open_link_id();
    let sig = state::with(|s| s.link.get(id).unwrap().signature.clone());
    assert!(!sig.is_empty());
    let now = w.now;
    state::with_mut(|s| s.link.stuck(id, "test: absent from its line", now));
    assert_eq!(state::with(|s| s.link.get(id).unwrap().phase), Phase::Stuck);
    assert!(state::with(|s| s.link.incidents.iter().any(|i| i.link == Some(id))), "stuck is an incident");
    // the chain does not know it: "landed" is refused, "undone" is taken and the books restored
    let e = block_on(driver::resolve_stuck(&mut w, id, true, now)).unwrap_err();
    assert!(e.contains("cannot be taken as landed"), "{e}");
    assert_eq!(state::with(|s| s.link.get(id).unwrap().phase), Phase::Stuck);
    let msg = block_on(driver::resolve_stuck(&mut w, id, false, now)).unwrap();
    assert!(msg.contains("undone on the operator's word"), "{msg}");
    assert_eq!(state::with(|s| s.link.get(id).unwrap().phase), Phase::Undone);
    assert_eq!(w.books().get(COLL).unwrap().module.backing, books_before.backing, "the mint's booking is gone");
    assert!(!etch_autobuyer::desk::state::collection_busy(COLL));
    assert!(block_on(driver::resolve_stuck(&mut w, id, false, now)).unwrap_err().contains("not stuck"), "decided once");
    // a second stuck link the chain DOES know, finalized ok: landed whatever the operator said
    let mut w = World::new();
    w.open(1, OpenKind::Mint { coll: COLL, recipient: soltx::b58_encode(&wallet(6)), min_out: None }).unwrap();
    w.deposit_sol(&intent_key(0), SOL);
    w.tick();
    let id = open_link_id();
    w.settle(); // the chain finalizes it; the desk has not settled
    let now = w.now;
    state::with_mut(|s| s.link.stuck(id, "test", now));
    let msg = block_on(driver::resolve_stuck(&mut w, id, false, now)).unwrap();
    assert!(msg.contains("finalized ok on chain") && msg.contains("landed"), "{msg}");
    assert_eq!(state::with(|s| s.link.get(id).unwrap().phase), Phase::Landed);
    let _ = (KeyPath::Main, State::Open, LineId::Money);
}

/// `K_main`'s signatures only: `n` of them newer than the cursor `c`, newest first (`s<k>`), no memos; both pages of the RPC.
struct Sigs(usize);
impl etch_autobuyer::solhome::SolIo for Sigs {
    async fn sol_signatures(&mut self, until: &str, limit: usize) -> Result<Vec<SigInfo>, String> {
        assert_eq!(until, "c");
        Ok((0..self.0.min(limit)).map(|i| SigInfo { signature: format!("s{i}"), slot: 1_000 - i as u64, failed: false, memo: None }).collect())
    }
    async fn sol_signatures_before(&mut self, until: &str, before: &str, limit: usize) -> Result<Vec<SigInfo>, String> {
        assert_eq!(until, "c");
        let from: usize = before.trim_start_matches('s').parse::<usize>().unwrap() + 1;
        Ok((from..self.0.min(from + limit)).map(|i| SigInfo { signature: format!("s{i}"), slot: 1_000 - i as u64, failed: false, memo: None }).collect())
    }
    fn sol_line(&mut self) -> SolLine {
        SolLine::default()
    }
    fn sol_line_save(&mut self, _l: &SolLine) {}
    fn sol_line_of(&mut self, _id: LineId) -> SolLine {
        SolLine::default()
    }
    fn sol_line_save_of(&mut self, _id: LineId, _l: &SolLine) {}
}
impl driver::DeskIo for Sigs {
    async fn key_of(&mut self, _p: &KeyPath) -> Result<Pubkey, String> {
        Err("no keys".into())
    }
}

/// FIXED B1-12: the scan pages back (`before`) until the cursor is met, at most `SCAN_PAGES` pages; the cursor moves to the newest;
/// a history longer than the budget is an incident (the reconciliation is the check), not a silent gap.
#[test]
fn fixed_b1_12_the_memo_scan_pages_back_to_its_cursor_and_says_when_it_cannot() {
    let budget = driver::SCAN_PAGES * driver::SCAN_LIMIT;
    for (n, gap) in [(7, false), (driver::SCAN_LIMIT, false), (driver::SCAN_LIMIT * 3 + 5, false), (budget, true), (budget + 300, true)] {
        state::reset();
        state::with_mut(|s| s.link.memo_cursor = Some("c".into()));
        let (seen, unexplained) = block_on(driver::scan_memos(&mut Sigs(n)));
        assert_eq!((seen, unexplained.len()), (0, 0));
        assert_eq!(state::with(|s| s.link.memo_cursor.clone()).as_deref(), Some("s0"), "{n}: the cursor is the newest signature");
        let incidents: Vec<String> = state::with(|s| s.link.incidents.iter().map(|i| i.what.clone()).collect());
        assert_eq!(incidents.iter().any(|w| w.starts_with("memo scan: over")), gap, "{n} signatures: {incidents:?}");
    }
    let _ = BTreeMap::<u8, u8>::new();
}

/// FIXED B1-08: anyone can put a system account at the address of (recipient, wX mint); the mint still creates the token account over
/// it (the program allocates a pre-funded address) and lands.
#[test]
fn fixed_b1_08_a_planted_system_account_at_the_recipients_token_address_does_not_block_the_mint() {
    let mut w = World::new();
    let to = wallet(6);
    let ata = soltx::associated_token_address(&to, &WX);
    w.deposit_sol(&ata, 890_880);
    w.open(1, OpenKind::Mint { coll: COLL, recipient: soltx::b58_encode(&to), min_out: None }).unwrap();
    w.deposit_sol(&intent_key(0), SOL);
    w.rounds(3);
    assert_eq!(w.intent(0).state, State::Done, "{}", w.intent(0).note);
    assert!(w.token(&to) > 0, "the recipient holds the wX");
}

/// FIXED B1-10: `sweep_unbooked` waits while the money line holds a step or an outcome its flow has not booked (a way back landed and
/// not yet in the books would be swept, then booked again); once the line is idle the unbooked lamports are swept.
#[test]
fn fixed_b1_10_no_sweep_while_the_money_line_has_a_step_out_or_an_outcome_unbooked() {
    let mut w = World::new();
    w.deposit_sol(&module(), SOL);
    let now = w.now;
    let mut l = w.line(LineId::Money);
    l.outcomes.push(etch_autobuyer::solhome::LineOutcome { step: etch_autobuyer::solhome::LineStep::Op(etch_autobuyer::solhome::LineOp {
        owner: etch_autobuyer::solhome::LineOwner::Eth, what: "a way back".into(), id: 1 }), intent: 0, ok: Some(true), signature: "s".into(), at: now });
    w.lines.insert(LineId::Money, l);
    let e = block_on(driver::sweep_unbooked(&mut w, COLL, now)).unwrap_err();
    assert!(e.contains("money line"), "{e}");
    w.lines.insert(LineId::Money, SolLine::default());
    let swept = block_on(driver::sweep_unbooked(&mut w, COLL, now)).unwrap();
    assert_eq!(swept, SOL);
}

/// FIXED B1-07 (§6.1): a deposit whose wX would overflow the hourly bucket mints what fits now; the rest stays at A for the next hour.
#[test]
fn fixed_b1_07_a_deposit_over_the_hourly_bucket_mints_what_fits_and_keeps_the_rest() {
    let mut w = World::new();
    let now = w.now as i64;
    let (room, per_one) = state::with(|s| (s.books.get(COLL).unwrap().module.mint_room(now).unwrap(), s.books.quote_mint(COLL, SOL, now).unwrap()));
    // lamports for 1.5 × the room's wX (the price is near flat over the bucket)
    let over = (room as u128 * 3 / 2 * SOL as u128 / per_one as u128) as u64;
    let to = wallet(6);
    w.open(1, OpenKind::Mint { coll: COLL, recipient: soltx::b58_encode(&to), min_out: None }).unwrap();
    w.deposit_sol(&intent_key(0), over + 10_000_000);
    w.rounds(3);
    let minted = w.token(&to);
    assert!(minted > room * 9 / 10 && minted <= room, "most of the room, never over it: {minted} of {room}");
    let left = w.lamports(&intent_key(0));
    assert!(left > over / 4 && left < over / 2, "the rest ({left} of {over}) stays at A");
    // the rest waits for the bucket (a sliver of room is not chased), as §6.1 says: the next hour's bucket, or the refund after the wait
    let it = w.intent(0);
    assert!(it.state == State::Funded && it.note.contains("OverCap"), "{:?} {}", it.state, it.note);
    assert_eq!(it.done_ops, 1, "one operation, not a trickle");
}
