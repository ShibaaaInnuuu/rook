//! Reconciliation (§4.4, §10.2): the books against the chain at finalized, per collection. Pure comparison here; the reads and the
//! reaction (`Ledger::note_findings`, the memo scan) are in `driver`.
use candid::CandidType;
use serde::{Deserialize, Serialize};

use super::gate::{eventual, ChainView};
use super::ledger::{Incident, Ledger};
use crate::books::{self, Books};

/// Consecutive reconciliations that must find a collection diverging before it is stopped (a read between two landings can see a
/// half-settled pair; two in a row cannot).
pub const HARD_STRIKES: u8 = 2;
/// Lamports above the books at `M[i]` that count as unbooked income worth reporting (and sweeping into the surplus).
pub const UNBOOKED_MIN: u64 = 10_000_000;

#[derive(CandidType, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum FindingKind {
    /// The wX supply on chain is not the ledger's (after what is still to land).
    SupplyMismatch,
    /// `M[i]` holds less than the books say it must.
    BooksUnbacked,
    /// Another invariant of `check_collection` fails.
    Invariant,
    /// `M[i]` holds more than the books (a gift, an unsolicited transfer): reported, swept into the surplus by the operator.
    Unbooked,
    /// Audit B1-01: the wX supply on chain is UNDER the ledger's: holders burned their own wX (owner-signed `Burn`); booked as the
    /// ledger's `holder_burned` once no link of the collection is open. Soft.
    HolderBurn,
}

#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Finding {
    pub coll: u16,
    pub kind: FindingKind,
    /// What the chain shows (adjusted for what is still to land).
    pub chain: u64,
    /// What the books say.
    pub books: u64,
    pub note: String,
}

impl Finding {
    pub fn hard(&self) -> bool {
        !matches!(self.kind, FindingKind::Unbooked | FindingKind::HolderBurn)
    }
}

#[derive(CandidType, Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct Report {
    pub at: u64,
    pub checked: u32,
    pub findings: Vec<Finding>,
    /// Collections this pass stopped.
    pub halted: Vec<u16>,
    /// Rook memos read from the chain, and the ones no link explains (a mint among them is SEV1).
    pub memos_seen: u32,
    pub unexplained: Vec<String>,
}

/// The findings for one collection (empty: books and chain agree).
pub fn compare(books: &Books, ledger: &Ledger, view: &ChainView, now: i64) -> Vec<Finding> {
    let Ok(c) = books.get(view.coll) else {
        return vec![Finding { coll: view.coll, kind: FindingKind::Invariant, chain: 0, books: 0, note: "unknown collection".into() }];
    };
    let mut out = vec![];
    let Some((free, supply)) = eventual(ledger, view) else {
        return vec![Finding { coll: view.coll, kind: FindingKind::Invariant, chain: view.supply, books: 0, note: "the undecided effects do not add up".into() }];
    };
    let want = c.ledger.expected_supply().unwrap_or(u64::MAX);
    if supply > want {
        out.push(Finding { coll: view.coll, kind: FindingKind::SupplyMismatch, chain: supply, books: want,
            note: format!("wX supply {supply} on chain after what is still to land, the ledger's is {want}") });
    } else if supply < want {
        out.push(Finding { coll: view.coll, kind: FindingKind::HolderBurn, chain: supply, books: want,
            note: format!("wX supply {supply} on chain after what is still to land, the ledger's is {want}: {} base units burned by their holders", want - supply) });
    }
    let owed = c.module.books();
    if (free as u128) < owed {
        out.push(Finding { coll: view.coll, kind: FindingKind::BooksUnbacked, chain: free, books: u64::try_from(owed).unwrap_or(u64::MAX),
            note: format!("M[{}] holds {free} lamports above rent, the books need {owed}", view.coll) });
    } else if (free as u128) - owed >= UNBOOKED_MIN as u128 {
        out.push(Finding { coll: view.coll, kind: FindingKind::Unbooked, chain: free, books: owed as u64,
            note: format!("M[{}] holds {} lamports above the books", view.coll, (free as u128) - owed) });
    }
    // (audit B1-I6: the invariants are checked only when the two findings above are absent, so a refusal here is another one)
    if out.is_empty() {
        if let Err(e) = books::check_collection(books, view.coll, free, supply, now) {
            out.push(Finding { coll: view.coll, kind: FindingKind::Invariant, chain: free, books: want, note: format!("{e}") });
        }
    }
    out
}

impl Ledger {
    /// Record what a reconciliation found: a collection with a hard finding gains a strike, one without loses them; at
    /// [`HARD_STRIKES`] it is stopped. Returns the collections stopped now.
    pub fn note_findings(&mut self, books: &mut Books, checked: &[u16], findings: &[Finding], now: u64) -> Vec<u16> {
        let mut stopped = vec![];
        for c in checked {
            let hard: Vec<&Finding> = findings.iter().filter(|f| f.coll == *c && f.hard()).collect();
            if hard.is_empty() {
                self.strikes.remove(c);
                continue;
            }
            let s = self.strikes.entry(*c).or_insert(0);
            *s = s.saturating_add(1);
            if *s >= HARD_STRIKES && !self.halted.contains_key(c) {
                let why = format!("reconciliation: {} (twice in a row)", hard.iter().map(|f| f.note.clone()).collect::<Vec<_>>().join("; "));
                self.halt(books, *c, &why, now);
                stopped.push(*c);
            }
        }
        for f in findings.iter().filter(|f| !f.hard()) {
            // B1-01: a holders' burn is booked (never under an open link, whose undo would wipe it) and is then not an incident
            if f.kind == FindingKind::HolderBurn && self.open_of(f.coll, None).next().is_none() {
                if books.book_holder_burn(f.coll, f.books.saturating_sub(f.chain)).is_ok() {
                    continue;
                }
            }
            // audit B1-I5: the same soft finding, pass after pass (a stranger's 0.01 SOL until the operator sweeps), is one incident
            let what = f.note.clone();
            if self.incidents.iter().rev().any(|i| i.coll == Some(f.coll) && i.link.is_none() && i.what == what) {
                continue;
            }
            self.incident(Incident { coll: Some(f.coll), link: None, what, at: now });
        }
        stopped
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn books() -> Books {
        let mut b = Books::new();
        b.add_collection(0, books::ListingKind::EthNft { chain_id: 1, collection: [1; 20], custody: [2; 20] }, [9; 32], books::default_params(20 * books::LAMPORTS_PER_SOL))
            .unwrap();
        b
    }

    /// Audit B1-I5: the same soft finding pass after pass is one incident; a different one is another; a hard one is strikes, not incidents.
    #[test]
    fn a_repeated_soft_finding_is_one_incident() {
        let mut b = books();
        let mut l = Ledger::default();
        let f = Finding { coll: 0, kind: FindingKind::Unbooked, chain: 10_000_000, books: 0, note: "M[0] holds 10000000 lamports above the books".into() };
        for i in 0..5 {
            assert!(l.note_findings(&mut b, &[0], &[f.clone()], 600 * i).is_empty());
        }
        assert_eq!(l.incidents.len(), 1, "{:?}", l.incidents);
        let g = Finding { note: "M[0] holds 20000000 lamports above the books".into(), ..f.clone() };
        l.note_findings(&mut b, &[0], &[g], 3_600);
        assert_eq!(l.incidents.len(), 2);
        assert!(l.strikes.is_empty(), "soft findings never strike");
    }

    /// Audit B1-01: a supply under the ledger's is booked as the holders' burns, once, and strikes nothing; over it is a hard finding.
    #[test]
    fn a_holder_burn_is_booked_not_an_incident() {
        let mut b = books();
        b.collections.get_mut(&0).unwrap().ledger.sol_backed = 7;
        let mut l = Ledger::default();
        let f = Finding { coll: 0, kind: FindingKind::HolderBurn, chain: 5, books: 7, note: "two burned".into() };
        assert!(l.note_findings(&mut b, &[0], &[f], 1).is_empty());
        assert_eq!(b.get(0).unwrap().ledger.holder_burned, 2);
        assert!(l.incidents.is_empty());
        let h = Finding { coll: 0, kind: FindingKind::SupplyMismatch, chain: 9, books: 7, note: "over".into() };
        assert!(h.hard());
        l.note_findings(&mut b, &[0], &[h.clone()], 2);
        assert_eq!(l.strikes.get(&0), Some(&1));
    }
}
