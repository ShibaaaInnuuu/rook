//! The pending-ledger (W4): the one place the books and Solana are tied together.
//!
//! **Decision (§4.4 rule 1): book first, with an undo journal; not "apply on finalize".** An operation changes the books at once (so
//! the price, the hourly buckets and the caps it consumed are consumed, and a second operation sees them), and a [`Link`] records it
//! BEFORE its transaction is signed: its memo, its chain effects ([`Effects`]: wX to mint or burn, lamports into or out of `M[i]`)
//! and an [`Undo`] journal (a snapshot of what the operation changed). The link is final only when the line reports the transaction
//! finalized; if it can never land (the send line's `can_never_land`: the nonce moved and no signature is known) or lands failed, the
//! journal is replayed and the books are as they were.
//!
//! Why not apply-on-finalize: the transaction's amounts (units minted, lamports paid) are fixed when it is signed, from the books'
//! state of that moment; re-running the books operation at finalization would price it at another moment (a floor post, another
//! mint) and could disagree with the bytes already on chain. Why the journal is sound: ONE unfinished books-changing link per
//! collection at a time ([`LinkError::Busy`]), so between `begin` and the outcome nothing else of that collection's money changes;
//! the only concurrent writers are the poster (floor fields) and the guardian's pause flags, which an undo leaves alone
//! ([`restore`] keeps them). Every book-changing flow (the desk, W6's coins, W8's NFT side, W9's treasury) must go through here.
//! A non-link writer waits while the collection has an open link (review F1, `desk::state::collection_busy`); a writer that cannot
//! wait, a LINE OUTCOME (lamports that landed at `M[i]`, a prize paid), hands its write to [`Ledger::write`], which owes it
//! ([`OwedWrite`], kept here across an upgrade) until the collection has no open link and then applies it (audit B1-06).
//!
//! **Idempotence.** Every link has a caller key (`intent:7:mint:0`); `begin` with a key already recorded returns that link and
//! changes nothing, so a retry after a crash, a resend or a repeated tick never books twice.
//!
//! Units: lamports, wX base units (10⁹ per unit), unix seconds.
use std::collections::BTreeMap;

use candid::CandidType;
use serde::{Deserialize, Serialize};

use super::memo;
use crate::books::{Books, BooksError, Collection, Module};

/// What a link does, as the memo names it.
#[derive(CandidType, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum OpKind {
    /// A public mint (SOL in, wX out).
    Mint,
    /// A public burn (wX in, SOL out).
    Burn,
    /// A deposit returned to its sender (no books change).
    Refund,
    /// A redemption's burn of one wX (a ticket).
    Redeem,
    /// A ticket's refund: ONE wX re-minted to the redeemer.
    Remint,
    /// The raffle's prize: ONE wX minted to the prize account.
    Prize,
    /// Lamports between the treasury and a module: a distributed surplus in (`fund_reserve`), revenue above target out (`skim`).
    Sweep,
    /// A coin's redemption for its backing.
    Coin,
    Other,
    /// Audit B1-04: an intent's emptied token account closed (no books change, no collection).
    Close,
    /// W8c: a Solana NFT purchase's advance (`M[i] → K_main`, `Books::advance`) in the same transaction as the purchase.
    Advance,
}

impl OpKind {
    /// The memo's kind token (lowercase ASCII).
    pub fn tag(&self) -> &'static str {
        match self {
            OpKind::Mint => "mint",
            OpKind::Burn => "burn",
            OpKind::Refund => "refund",
            OpKind::Redeem => "redeem",
            OpKind::Remint => "remint",
            OpKind::Prize => "prize",
            OpKind::Sweep => "sweep",
            OpKind::Coin => "coin",
            OpKind::Other => "other",
            OpKind::Close => "close",
            OpKind::Advance => "advance",
        }
    }
    /// A kind whose transaction mints wX (an unexplained one on chain is a SEV1).
    pub fn mints(&self) -> bool {
        matches!(self, OpKind::Mint | OpKind::Remint | OpKind::Prize)
    }
}

/// What the transaction does on chain to collection `coll`: the numbers the guards and the reconciliation reason about.
#[derive(CandidType, Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Effects {
    /// wX the chain will mint (`MintTo`).
    pub mint_units: u64,
    /// wX the chain will burn (`Burn`).
    pub burn_units: u64,
    /// Lamports that arrive at `M[i]`.
    pub lamports_in: u64,
    /// Lamports that leave `M[i]`.
    pub lamports_out: u64,
}

impl Effects {
    pub fn is_none(&self) -> bool {
        *self == Effects::default()
    }
    fn add(&mut self, o: &Effects) {
        self.mint_units = self.mint_units.saturating_add(o.mint_units);
        self.burn_units = self.burn_units.saturating_add(o.burn_units);
        self.lamports_in = self.lamports_in.saturating_add(o.lamports_in);
        self.lamports_out = self.lamports_out.saturating_add(o.lamports_out);
    }
}

/// How much of the books an operation snapshots for its undo.
#[derive(CandidType, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    /// The operation changes no books (a refund of a deposit).
    None,
    /// The collection's module and its SOL-backed units (a public mint or burn, a prize).
    Module,
    /// The whole collection (tickets, items: a redemption, a ticket refund).
    Collection,
}

/// The undo journal of one link.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum Undo {
    None,
    Module { module: Box<Module>, sol_backed: u64 },
    Collection(Box<Collection>),
}

/// Where a link is.
#[derive(CandidType, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// Booked and journalled; not signed (or not known to be).
    Recorded,
    /// Signed and saved on a send line; not finalized.
    Signed,
    /// Finalized, succeeded: final.
    Landed,
    /// Finalized, failed on chain (it paid its fee and changed nothing): undone.
    Failed,
    /// Proven never to land, or never signed: undone.
    Undone,
    /// Signed, and neither its line nor the chain says what became of it: the books stay reserved, an operator looks.
    Stuck,
}

impl Phase {
    /// Books reserved, the chain not yet decided.
    pub fn open(&self) -> bool {
        matches!(self, Phase::Recorded | Phase::Signed | Phase::Stuck)
    }
}

#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Link {
    pub id: u64,
    /// The caller's idempotence key.
    pub key: String,
    pub kind: OpKind,
    pub coll: Option<u16>,
    /// The desk intent it serves.
    pub intent: Option<u64>,
    pub memo: String,
    pub effects: Effects,
    /// A number the booking returned (a redemption's ticket id).
    pub aux: u64,
    pub phase: Phase,
    pub undo: Undo,
    /// The signature it was signed under (a re-sign under the recent-blockhash fallback changes it: the line's outcome carries the last).
    pub signature: String,
    pub reason: String,
    pub created_at: u64,
    pub updated_at: u64,
}

/// A reason the collection (or the link) stopped: the monitor pages on these.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Incident {
    pub coll: Option<u16>,
    pub link: Option<u64>,
    pub what: String,
    pub at: u64,
}

/// Audit B1-06: a book write made at a LINE OUTCOME, which cannot wait on `collection_busy` the way a writer at a decision does
/// (review F1): the chain has moved (lamports landed at `M[i]`, a prize left the treasury) and the books must follow, but not under
/// an open link, whose undo would restore the collection's snapshot and wipe the write. [`Ledger::write`] applies it at once when the
/// collection has no open link, else owes it here (`Ledger::owed`, saved with the desk state across an upgrade) and applies it the
/// moment the last link of the collection is decided, before any new link's snapshot. Chosen over an inverse-delta undo: three
/// writers and three write kinds against every booking closure of every flow, and the snapshot undo stays as it is.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum OwedWrite {
    /// `Books::return_advance`: an advance's lamports came back to `M[i]` (the way back landed, or the advance never left).
    ReturnAdvance { lamports: u64 },
    /// `Books::fund_reserve`: lamports arrived at `M[i]` and join the surplus (a sweep's way back, a late credit).
    FundReserve { lamports: u64 },
    /// `Books::claim_raffle`: the prize was PAID on chain at `now`; the books' claim is as of then.
    ClaimRaffle { owner: [u8; 32], cum_start: u64, weight: u64, proof: Vec<[u8; 32]>, now: i64 },
    /// W8c: a Solana NFT purchase landed and the item is in `K_main`'s custody at finalized: `mint_item` to the module with the
    /// custody proof (`key` = the mint, `tx` = the landed signature's first 32 bytes), then `settle_advance` at `cost` (the advance
    /// was exactly the ask's all-in cost). All or nothing.
    SolNftArrived { key: [u8; 32], tx: [u8; 32], cost: u64, now: i64 },
    /// W8c: a delivery landed and the destination holds the item: `record_delivery`.
    RecordDelivery { ticket: u64, tx: [u8; 32] },
    /// W8c: a delivery landed but the destination's custody could not be confirmed: `note_sent` (never a late refund of it).
    NoteSent { ticket: u64, tx: [u8; 32] },
    /// W8c: a delivery the chain refused (a rule set veto, B2-09): `mark_undeliverable`.
    MarkUndeliverable { ticket: u64 },
}

impl OwedWrite {
    fn apply(self, books: &mut Books, coll: u16) -> Result<(), BooksError> {
        match self {
            OwedWrite::ReturnAdvance { lamports } => books.return_advance(coll, lamports).map(|_| ()),
            OwedWrite::FundReserve { lamports } => books.fund_reserve(coll, lamports),
            OwedWrite::ClaimRaffle { owner, cum_start, weight, proof, now } => books.claim_raffle(coll, owner, cum_start, weight, &proof, now).map(|_| ()),
            OwedWrite::SolNftArrived { key, tx, cost, now } => {
                // on a copy: the mint and the settlement land together or not at all
                let mut b = books.clone();
                let args = crate::books::MintItemArgs { key, source: crate::books::math::keccak(&[&key, &tx]), tx, vout: 0, deposit: false,
                    inscription_id: String::new(), proof: vec![] };
                b.mint_item(coll, args, crate::books::MintTo::Module, now)?;
                b.settle_advance(coll, cost, now)?;
                *books = b;
                Ok(())
            }
            OwedWrite::RecordDelivery { ticket, tx } => books.record_delivery(coll, ticket, tx),
            OwedWrite::NoteSent { ticket, tx } => books.note_sent(coll, ticket, tx),
            OwedWrite::MarkUndeliverable { ticket } => books.mark_undeliverable(coll, ticket),
        }
    }

    pub fn label(&self) -> String {
        match self {
            OwedWrite::ReturnAdvance { lamports } => format!("return_advance({lamports})"),
            OwedWrite::FundReserve { lamports } => format!("fund_reserve({lamports})"),
            OwedWrite::ClaimRaffle { owner, .. } => format!("claim_raffle({})", crate::soltx::b58_encode(owner)),
            OwedWrite::SolNftArrived { key, cost, .. } => format!("mint_item({}) + settle_advance({cost})", crate::soltx::b58_encode(key)),
            OwedWrite::RecordDelivery { ticket, .. } => format!("record_delivery({ticket})"),
            OwedWrite::NoteSent { ticket, .. } => format!("note_sent({ticket})"),
            OwedWrite::MarkUndeliverable { ticket } => format!("mark_undeliverable({ticket})"),
        }
    }
}

/// What `begin` is asked.
#[derive(Clone, Debug)]
pub struct Spec {
    pub key: String,
    pub kind: OpKind,
    pub coll: Option<u16>,
    pub intent: Option<u64>,
    pub scope: Scope,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkError {
    /// The books refused the operation; they are unchanged.
    Books(BooksError),
    /// Another books-changing link of this collection is unfinished: try again when it is decided.
    Busy(u16),
    /// The collection is stopped by an incident (`halt`): nothing is signed for it until the operator clears it.
    Halted(u16),
}

impl std::fmt::Display for LinkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LinkError::Books(e) => write!(f, "{e}"),
            LinkError::Busy(c) => write!(f, "collection {c} has an unfinished operation"),
            LinkError::Halted(c) => write!(f, "collection {c} is halted by an incident"),
        }
    }
}

/// Finished links kept for the idempotence keys and `ledger_log`, at most.
pub const KEEP_FINISHED: usize = 2_000;
/// Incidents kept, at most.
pub const INCIDENTS_KEPT: usize = 64;

#[derive(CandidType, Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct Ledger {
    pub next_id: u64,
    pub links: BTreeMap<u64, Link>,
    pub keys: BTreeMap<String, u64>,
    /// Collections stopped by an incident, with why.
    pub halted: BTreeMap<u16, Incident>,
    pub incidents: Vec<Incident>,
    /// Consecutive reconciliations that found the collection diverging.
    pub strikes: BTreeMap<u16, u8>,
    /// The last signature the memo scan has read (newer ones are read next).
    pub memo_cursor: Option<String>,
    pub last_report: Option<super::recon::Report>,
    /// Audit B1-06: line outcomes' book writes owed to a collection while it had an open link, in order (`opt`: candid decodes a
    /// field absent from older bytes only as an option; `#[serde(default)]` is not enough for it).
    pub owed: Option<BTreeMap<u16, Vec<OwedWrite>>>,
}

/// The floor module's own inputs (written by the poster, never by a mint or a burn): an undo leaves them as they are.
#[derive(Clone, Copy)]
struct FloorFields {
    accepted: u64,
    target: u64,
    posted_at: i64,
    move_acc: u64,
    confidence_bps: u16,
    depth_items: u32,
    day_anchor: u64,
    day_anchor_at: i64,
    last_good_at: i64,
}

impl FloorFields {
    fn of(m: &Module) -> FloorFields {
        FloorFields { accepted: m.accepted, target: m.target, posted_at: m.posted_at, move_acc: m.move_acc, confidence_bps: m.confidence_bps,
            depth_items: m.depth_items, day_anchor: m.day_anchor, day_anchor_at: m.day_anchor_at, last_good_at: m.last_good_at }
    }
    fn put(self, m: &mut Module) {
        m.accepted = self.accepted;
        m.target = self.target;
        m.posted_at = self.posted_at;
        m.move_acc = self.move_acc;
        m.confidence_bps = self.confidence_bps;
        m.depth_items = self.depth_items;
        m.day_anchor = self.day_anchor;
        m.day_anchor_at = self.day_anchor_at;
        m.last_good_at = self.last_good_at;
    }
}

/// Replays an undo journal onto `coll` (the floor fields and the pause flags stay as they are now).
pub fn restore(books: &mut Books, coll: u16, undo: Undo) {
    let Some(c) = books.collections.get_mut(&coll) else { return };
    match undo {
        Undo::None => {}
        Undo::Module { module, sol_backed } => {
            let keep = FloorFields::of(&c.module);
            c.module = *module;
            keep.put(&mut c.module);
            c.ledger.sol_backed = sol_backed;
        }
        Undo::Collection(snap) => {
            let keep = FloorFields::of(&c.module);
            let (paused, poster_paused) = (c.paused, c.poster_paused);
            *c = *snap;
            keep.put(&mut c.module);
            c.paused = paused;
            c.poster_paused = poster_paused;
        }
    }
}

impl Ledger {
    /// Book an operation and record it, BEFORE anything is signed. `f` runs the books operation (all-or-nothing) and returns what the
    /// chain must do and a number to keep. With a key already recorded nothing runs and the existing link is returned (`false`: not new).
    pub fn begin(&mut self, books: &mut Books, spec: Spec, now: u64, f: impl FnOnce(&mut Books) -> Result<(Effects, u64), BooksError>)
        -> Result<(u64, bool), LinkError> {
        if let Some(id) = self.keys.get(&spec.key) {
            return Ok((*id, false));
        }
        if let Some(c) = spec.coll {
            if self.halted.contains_key(&c) {
                return Err(LinkError::Halted(c));
            }
            if spec.scope != Scope::None && self.links.values().any(|l| l.coll == Some(c) && l.phase.open() && l.undo != Undo::None) {
                return Err(LinkError::Busy(c));
            }
            // audit B1-06: what a line outcome owed the books goes in BEFORE this link's snapshot (nothing, unless a link was decided
            // without passing through `landed` or `undo`)
            self.apply_owed(books, c, now);
        }
        let undo = match (spec.scope, spec.coll) {
            (Scope::None, _) => Undo::None,
            (_, None) => return Err(LinkError::Books(BooksError::UnknownCollection)),
            (Scope::Module, Some(c)) => {
                let col = books.get(c).map_err(LinkError::Books)?;
                Undo::Module { module: Box::new(col.module.clone()), sol_backed: col.ledger.sol_backed }
            }
            (Scope::Collection, Some(c)) => Undo::Collection(Box::new(books.get(c).map_err(LinkError::Books)?.clone())),
        };
        let (effects, aux) = f(books).map_err(LinkError::Books)?;
        let id = self.next_id;
        self.next_id += 1;
        let m = memo::memo(spec.coll, spec.kind, id, spec.intent);
        self.keys.insert(spec.key.clone(), id);
        self.links.insert(id, Link { id, key: spec.key, kind: spec.kind, coll: spec.coll, intent: spec.intent, memo: m, effects, aux, phase: Phase::Recorded,
            undo, signature: String::new(), reason: String::new(), created_at: now, updated_at: now });
        Ok((id, true))
    }

    pub fn get(&self, id: u64) -> Option<&Link> {
        self.links.get(&id)
    }

    /// The link whose memo is `text`.
    pub fn by_memo(&self, text: &str) -> Option<&Link> {
        let r = memo::parse(text)?;
        self.links.get(&r.id).filter(|l| l.memo == text)
    }

    /// The signature is saved on a line: the transaction is out (or will be sent from the saved bytes).
    pub fn signed(&mut self, id: u64, signature: &str, now: u64) {
        if let Some(l) = self.links.get_mut(&id) {
            if l.phase == Phase::Recorded {
                l.phase = Phase::Signed;
            }
            if l.phase.open() {
                l.signature = signature.to_string();
                l.updated_at = now;
            }
        }
    }

    /// Finalized and succeeded: final. The journal is dropped; what the collection's outcomes owed the books meanwhile goes in.
    pub fn landed(&mut self, books: &mut Books, id: u64, signature: &str, now: u64) {
        let mut coll = None;
        if let Some(l) = self.links.get_mut(&id) {
            if l.phase.open() {
                l.phase = Phase::Landed;
                l.signature = signature.to_string();
                l.undo = Undo::None;
                l.updated_at = now;
                coll = l.coll;
            }
        }
        if let Some(c) = coll {
            self.apply_owed(books, c, now);
        }
    }

    /// Finalized and failed on chain, or proven never to land, or never signed: the journal is replayed, then what the collection's
    /// outcomes owed the books meanwhile goes in (on top of the restored snapshot, never under it: audit B1-06).
    pub fn undo(&mut self, books: &mut Books, id: u64, phase: Phase, why: &str, now: u64) {
        debug_assert!(matches!(phase, Phase::Failed | Phase::Undone));
        let Some(l) = self.links.get_mut(&id) else { return };
        if !l.phase.open() {
            return;
        }
        l.phase = phase;
        l.reason = why.to_string();
        l.updated_at = now;
        let (coll, undo) = (l.coll, std::mem::replace(&mut l.undo, Undo::None));
        if let Some(c) = coll {
            restore(books, c, undo);
            self.apply_owed(books, c, now);
        }
    }

    /// Audit B1-06: a line outcome's book write on `coll`: applied now when the collection has no open link (the refusal is the
    /// caller's), else owed and applied by [`Ledger::apply_owed`] the moment its last open link is decided (a refusal then is an
    /// incident). `Ok` means applied or owed: either way the books will say it, and no link's undo wipes it.
    pub fn write(&mut self, books: &mut Books, coll: u16, w: OwedWrite) -> Result<(), BooksError> {
        if self.open_of(coll, None).next().is_some() {
            self.owed.get_or_insert_with(BTreeMap::new).entry(coll).or_default().push(w);
            return Ok(());
        }
        w.apply(books, coll)
    }

    /// The writes owed to `coll`, applied in order once it has no open link (nothing while one is open). Called at every decision
    /// of a link ([`Ledger::landed`], [`Ledger::undo`]) and before a new link's snapshot ([`Ledger::begin`]).
    pub fn apply_owed(&mut self, books: &mut Books, coll: u16, now: u64) {
        if self.open_of(coll, None).next().is_some() {
            return;
        }
        let Some(ws) = self.owed.as_mut().and_then(|o| o.remove(&coll)) else { return };
        for w in ws {
            let label = w.label();
            if let Err(e) = w.apply(books, coll) {
                let what = format!("{label}, owed to the books by a line outcome while a link was open, is refused: {e} (OPERATOR: the chain moved, the books did not)");
                self.incident(Incident { coll: Some(coll), link: None, what, at: now });
            }
        }
    }

    /// The writes owed to `coll` and not yet applied.
    pub fn owed_of(&self, coll: u16) -> &[OwedWrite] {
        self.owed.as_ref().and_then(|o| o.get(&coll)).map_or(&[], |v| v.as_slice())
    }

    /// Neither the line nor the chain says what became of it: reserved stays reserved, and an incident is raised.
    pub fn stuck(&mut self, id: u64, why: &str, now: u64) {
        let Some(l) = self.links.get_mut(&id) else { return };
        if l.phase.open() && l.phase != Phase::Stuck {
            l.phase = Phase::Stuck;
            l.reason = why.to_string();
            l.updated_at = now;
            let inc = Incident { coll: l.coll, link: Some(id), what: format!("link {id} ({}) is stuck: {why}", l.memo), at: now };
            self.incident(inc);
        }
    }

    pub fn incident(&mut self, i: Incident) {
        self.incidents.push(i);
        let extra = self.incidents.len().saturating_sub(INCIDENTS_KEPT);
        self.incidents.drain(..extra);
    }

    /// Stop collection `coll`: no operation is begun for it until [`Ledger::clear_halt`]; its public mint is paused too (burns and
    /// redemptions are the holders' exit and stay, refusing only if their own guards do).
    pub fn halt(&mut self, books: &mut Books, coll: u16, why: &str, now: u64) {
        let inc = Incident { coll: Some(coll), link: None, what: why.to_string(), at: now };
        self.halted.entry(coll).or_insert_with(|| inc.clone());
        self.incident(inc);
        let _ = books.set_paused(coll, true);
    }

    /// The operator, having looked: the collection runs again (its public mint stays paused until the operator lifts that too).
    pub fn clear_halt(&mut self, coll: u16) -> bool {
        self.strikes.remove(&coll);
        self.halted.remove(&coll).is_some()
    }

    /// Links of `coll` whose chain effect is not yet decided (the optional `except` left out).
    pub fn open_of(&self, coll: u16, except: Option<u64>) -> impl Iterator<Item = &Link> {
        self.links.values().filter(move |l| l.coll == Some(coll) && l.phase.open() && Some(l.id) != except)
    }

    /// The sum of the undecided effects on `coll` (`except` left out).
    pub fn totals(&self, coll: u16, except: Option<u64>) -> Effects {
        let mut t = Effects::default();
        for l in self.open_of(coll, except) {
            t.add(&l.effects);
        }
        t
    }

    pub fn any_open(&self) -> bool {
        self.links.values().any(|l| l.phase.open())
    }

    /// Finished links beyond [`KEEP_FINISHED`] go, oldest first (their keys with them).
    pub fn prune(&mut self) {
        let finished: Vec<u64> = self.links.values().filter(|l| !l.phase.open()).map(|l| l.id).collect();
        let extra = finished.len().saturating_sub(KEEP_FINISHED);
        for id in finished.into_iter().take(extra) {
            if let Some(l) = self.links.remove(&id) {
                self.keys.remove(&l.key);
            }
        }
    }
}
