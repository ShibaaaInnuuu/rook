//! The desk's state: the books, the pending-ledger and the intents, in one thread-local that is written to stable memory at upgrade.
//!
//! **Persistence.** The upgrade tuple is fixed-width, so the desk rides in the fifteenth element's `SolLines.desk_state` as one
//! versioned candid blob ([`StableDesk`], like `books::StableBooks`), written by `pre_upgrade` ([`before_upgrade`]) and taken out at
//! `post_upgrade` ([`after_upgrade`]); the live `SolLines` never carries it. A later layout is a new variant; old bytes keep decoding. A
//! blob that does not decode refuses the upgrade (it is never a fresh start: that would forget every open intent and every reserved
//! operation).
use std::cell::RefCell;
use std::collections::BTreeMap;

use candid::CandidType;
use serde::{Deserialize, Serialize};

use super::intent::Intent;
use crate::books::Books;
use crate::chainlink::Ledger;

/// One opening of an intent (the rate limits).
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct OpenLog {
    pub caller: Vec<u8>,
    pub at: u64,
}

#[derive(CandidType, Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct DeskState {
    pub books: Books,
    pub link: Ledger,
    pub intents: BTreeMap<u64, Intent>,
    pub next_intent: u64,
    pub opens: Vec<OpenLog>,
    /// Rent-exempt minimums read from the chain, by account size.
    pub rent: BTreeMap<u64, u64>,
    pub last_reconcile: u64,
    /// The operator's switch for the whole desk (stop-only: an open intent still finishes or is returned).
    pub desk_paused: bool,
    /// Coin redemption intents are accepted (set once W6's executor is wired).
    pub coin_redeem: bool,
    /// Audit B1-05: the last intent read by the rotation of never-funded expired intents (the next pass starts after it).
    #[serde(default)]
    pub rotation: u64,
}

/// The stable layout. A new layout is a new variant.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum StableDesk {
    V1(DeskState),
}

pub fn encode_state(s: &DeskState) -> Vec<u8> {
    candid::encode_one(StableDesk::V1(s.clone())).expect("the desk state encodes")
}

pub fn decode_state(bytes: &[u8]) -> Result<DeskState, String> {
    match candid::decode_one::<StableDesk>(bytes).map_err(|e| format!("the desk state does not decode: {e}"))? {
        StableDesk::V1(s) => Ok(s),
    }
}

thread_local! {
    static STATE: RefCell<DeskState> = RefCell::new(DeskState::default());
}

/// Read the state. Never across an await.
/// Review F1: whether collection `coll` has an undecided chain link. A link's undo restores the collection's books from a snapshot
/// taken at `begin`, so no other writer may change those books while one is open: every non-link writer waits on this.
pub fn collection_busy(coll: u16) -> bool {
    with(|s| s.link.open_of(coll, None).next().is_some())
}

/// Audit B1-06: a LINE OUTCOME's book write (the chain already moved: it cannot wait like a decision): applied at once, or owed by
/// the link ledger while `coll` has an open link and applied when the link is decided, so no link's undo wipes it.
pub fn book_outcome(coll: u16, w: crate::chainlink::ledger::OwedWrite) -> Result<(), crate::books::BooksError> {
    with_mut(|s| s.link.write(&mut s.books, coll, w))
}

pub fn with<R>(f: impl FnOnce(&DeskState) -> R) -> R {
    STATE.with(|s| f(&s.borrow()))
}

/// Write the state. Never across an await.
pub fn with_mut<R>(f: impl FnOnce(&mut DeskState) -> R) -> R {
    STATE.with(|s| f(&mut s.borrow_mut()))
}

/// The state as stable bytes.
pub fn encode() -> Vec<u8> {
    with(encode_state)
}

/// Replace the state with what `bytes` hold (an upgrade, or a test's restart).
pub fn restore(bytes: &[u8]) -> Result<(), String> {
    let s = decode_state(bytes)?;
    STATE.with(|c| *c.borrow_mut() = s);
    Ok(())
}

/// A fresh, empty state (tests).
pub fn reset() {
    STATE.with(|c| *c.borrow_mut() = DeskState::default());
}

/// `pre_upgrade`: the desk goes into the saved lines' blob. Skipped for a desk that never held anything (the blob stays absent).
pub fn before_upgrade() {
    let blob = with(|s| (*s != DeskState::default()).then(|| encode_state(s)));
    crate::solhome::LINES.with(|l| l.borrow_mut().desk_state = blob);
}

/// `post_upgrade`, after the lines are restored: the desk comes out of the blob (and the blob is cleared from the live lines).
pub fn after_upgrade() -> Result<(), String> {
    let blob = crate::solhome::LINES.with(|l| l.borrow_mut().desk_state.take());
    match blob {
        Some(b) => restore(&b),
        None => {
            reset();
            Ok(())
        }
    }
}

/// Something of the desk needs the fast cadence: a funded intent to act on, a transaction out, or an operation not yet decided.
pub fn busy() -> bool {
    with(|s| s.link.any_open() || s.intents.values().any(|i| matches!(i.state, super::intent::State::Funded | super::intent::State::Executing)
        || (i.close_due && !i.attention)))
}
