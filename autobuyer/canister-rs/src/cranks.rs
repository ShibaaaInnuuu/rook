//! W9 (docs/v9/canister-only-solana.md §9, §13.1): the treasury, the redemption draws and the raffles as the canister's own cranks on the
//! books (what the registry keeper cranked on rook_core / rook_backing before decision D-l retired the programs). On the crank line,
//! beside the coins; never queued; at most ONE transaction a pass (`crankline::line_free`).
//!
//! Books-only, no transaction: a pending treasury change past its 24 h (`apply_*`), `allocate`, `distribute_all` (daily), `resolve` of
//! the next pinned ticket of each collection with the drand round two relays agree on and the canister verifies, a raffle's `expire`.
//!
//! SOL moves, as owed transfers the books already count, retried until they land: `K_tre → M[i]` for a collection's distributed
//! surplus (`Surplus`), `K_tre → ops` for the ops share (`Ops`, the fueler's hot key, §9.2). The prize of a raffle is a link (the pot buys
//! one whole wX at the module's mint: `K_tre → M[i]`, `MintTo` the treasury's wX account) through the chain links of W4.
//!
//! The weekly snapshot, commit, draw and claim of a raffle plug in here once `raffle_books` lands (the snapshot of the books' coins).
use std::cell::RefCell;
use std::collections::BTreeMap;

use candid::CandidType;
use serde::{Deserialize, Serialize};

use crate::books::{self, ONE};
use crate::chainlink::driver::{DeskIo, SignError};
use crate::chainlink::ledger::{Effects, OpKind, OwedWrite, Phase, Scope, Spec};
use crate::coins::link;
use crate::desk::state as desk;
use crate::crankline::CrankIo;
use crate::solhome::{LineId, LineOwner, LineStep, SolIo};
use crate::solix2::spl;
use crate::solkey::KeyPath;
use crate::soltx::{self, associated_token_address, b58_encode, Pubkey};

/// A surplus transfer waits until a collection is owed this much (§9.1: 0.01 SOL); the ops share goes out at 0.1 SOL (§9.2).
pub const SURPLUS_MIN_LAMPORTS: u64 = 10_000_000;
pub const OPS_MIN_LAMPORTS: u64 = 100_000_000;
/// `allocate` once the unallocated income reaches this (the keeper's `MIN_FEE_LAMPORTS`, 0.05 SOL).
pub const ALLOCATE_MIN_LAMPORTS: u64 = 50_000_000;
/// SOL at the claimer is swept to the treasury once this much sits above the key's rent.
pub const CLAIMER_SWEEP_MIN_LAMPORTS: u64 = 50_000_000;
pub const KEY_RENT_LAMPORTS: u64 = 890_880;
/// How often each scan runs when it found nothing to do (seconds).
pub const APPLY_EVERY: u64 = 3_600;
pub const ALLOCATE_EVERY: u64 = 3_600;
pub const DISTRIBUTE_EVERY: u64 = 6 * 3_600;
pub const RESOLVE_EVERY: u64 = 900;
pub const RAFFLE_EVERY: u64 = 3_600;
/// Review F11: how often the modules are scanned for revenue above target (`skim`) and the tickets for a refund due (`remint`).
pub const SKIM_EVERY: u64 = 3_600;
pub const REMINT_EVERY: u64 = 3_600;
/// A module's revenue above target is skimmed once it reaches this (0.01 SOL, as a surplus).
pub const SKIM_MIN_LAMPORTS: u64 = 10_000_000;
/// Steps signed a day, at most.
pub const STEPS_PER_DAY: u32 = 96;
pub const WAIT_RETRY_SECS: u64 = 3_600;
pub const WAITS_KEPT: usize = 64;

/// A step of the treasury on the crank line.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum TreasuryStep {
    /// `K_tre → M[i]`: a collection's distributed surplus, owed by the treasury; a link: the module's surplus is credited when it lands.
    Surplus { coll: u32, lamports: u64, link: u64 },
    /// `K_tre → ops`: the ops share owed, to the fueler's hot key.
    Ops { to: Pubkey, lamports: u64 },
    /// A raffle's prize: the pot buys one whole wX at the module's mint (`K_tre → M[i]`, `MintTo` the treasury's wX account; a link).
    Prize { coll: u32, price: u64, link: u64 },
    /// A raffle's claim: one whole wX from the treasury's wX account to the winner's.
    Claim { coll: u32, winner: Pubkey, round: u64 },
    /// `K_fc → K_tre`: SOL that arrived at the claimer (ROOK's fees: a SOL-paired coin's claims land there as SOL), booked as income
    /// when it lands (§9.1, §16).
    ClaimerSweep { lamports: u64 },
    /// Review F11: `M[i] → K_tre`, a module's revenue above its target (`Books::skim`, booked as treasury income at begin; a link).
    Skim { coll: u32, lamports: u64, link: u64 },
    /// Review F11: a ticket's refund: ONE wX re-minted to its owner at the module's mint (`Books::refund`; a link).
    Remint { coll: u32, ticket: u64, owner: Pubkey, link: u64 },
}

impl TreasuryStep {
    pub fn kind(&self) -> &'static str {
        match self {
            TreasuryStep::Surplus { .. } => "surplus",
            TreasuryStep::Ops { .. } => "ops",
            TreasuryStep::Prize { .. } => "prize",
            TreasuryStep::Claim { .. } => "claim",
            TreasuryStep::ClaimerSweep { .. } => "claimer_sweep",
            TreasuryStep::Skim { .. } => "skim",
            TreasuryStep::Remint { .. } => "remint",
        }
    }
    pub fn key(&self) -> String {
        match self {
            TreasuryStep::Surplus { coll, .. } | TreasuryStep::Prize { coll, .. } | TreasuryStep::Claim { coll, .. } | TreasuryStep::Skim { coll, .. } => format!("{}:{coll}", self.kind()),
            TreasuryStep::Remint { coll, ticket, .. } => format!("remint:{coll}:{ticket}"),
            TreasuryStep::Ops { .. } => "ops".into(),
            TreasuryStep::ClaimerSweep { .. } => "claimer_sweep".into(),
        }
    }
    pub fn link(&self) -> Option<u64> {
        match self {
            TreasuryStep::Prize { link, .. } | TreasuryStep::Surplus { link, .. } | TreasuryStep::Skim { link, .. } | TreasuryStep::Remint { link, .. } => Some(*link),
            _ => None,
        }
    }
    pub fn label(&self) -> String {
        match self {
            TreasuryStep::Surplus { coll, lamports, link } => format!("treasury: {lamports} lamports of surplus to module {coll} (link {link})"),
            TreasuryStep::Ops { to, lamports } => format!("treasury: {lamports} lamports of ops to {}", b58_encode(to)),
            TreasuryStep::Prize { coll, price, link } => format!("treasury: raffle {coll}'s prize, one wX at {price} lamports (link {link})"),
            TreasuryStep::Claim { coll, winner, round } => format!("treasury: raffle {coll} round {round}'s prize to {}", b58_encode(winner)),
            TreasuryStep::ClaimerSweep { lamports } => format!("treasury: {lamports} lamports of SOL fees from the claimer to the treasury"),
            TreasuryStep::Skim { coll, lamports, link } => format!("treasury: {lamports} lamports of revenue above target skimmed from module {coll} (link {link})"),
            TreasuryStep::Remint { coll, ticket, owner, link } => format!("treasury: ticket {ticket} of collection {coll} refunded, one wX re-minted to {} (link {link})", b58_encode(owner)),
        }
    }
}

/// A round's snapshot taken and published, waiting for a later finalized slot to be committed as the books require.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct PendingCommit {
    pub round: u64,
    pub root: [u8; 32],
    pub total_weight: u64,
    pub snapshot_slot: u64,
    pub entries: u32,
    pub taken_at: u64,
}

/// A drawn round's winner: the leaf the books accept and the proof, paid by [`TreasuryStep::Claim`] and booked when it lands.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct PendingClaim {
    pub round: u64,
    pub owner: Pubkey,
    pub cum_start: u64,
    pub weight: u64,
    pub proof: Vec<[u8; 32]>,
}

/// SOL the books already moved and the chain still owes.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum Owed {
    Surplus { coll: u32, lamports: u64 },
    Ops { lamports: u64 },
}

#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Wait {
    pub key: String,
    pub since: u64,
    pub why: String,
    pub retry_at: u64,
    pub tries: u32,
}

#[derive(CandidType, Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct CranksState {
    /// When each scan is next due.
    pub next: BTreeMap<String, u64>,
    pub owed: Vec<Owed>,
    pub waits: Vec<Wait>,
    /// A counter for the idempotence keys of the links this module begins.
    pub ops: u64,
    pub day: u64,
    pub signed_today: u32,
    /// Each step kind's last landing, and the last books-only actions.
    pub last: Vec<(String, u64, String)>,
    /// The last drand disagreement: `(round, when, what the relays served)`; cleared once that round is proven.
    pub drand_disagreement: Option<(u64, u64, String)>,
    /// Per collection: a snapshot published and not committed yet, and a drawn round's winner not paid yet.
    pub commits: BTreeMap<u32, PendingCommit>,
    pub claims: BTreeMap<u32, PendingClaim>,
    /// §10.4: resumes scheduled (a pause lifts only [`RESUME_DELAY_SECS`] after it was asked): `(what, collection, when)`.
    pub resumes: Vec<(Resume, u32, u64)>,
    /// The operator's switch.
    pub off: bool,
}

/// What a guardian's pause stopped, and a resume lifts after its delay.
#[derive(CandidType, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Resume {
    Collection,
    Desk,
    AllMints,
}

/// §10.4: unpausing takes this long (canister-enforced).
pub const RESUME_DELAY_SECS: u64 = 24 * 3_600;

impl CranksState {
    pub fn due(&self, k: &str, now: u64) -> bool {
        self.next.get(k).is_none_or(|t| now >= *t)
    }
    pub fn set_next(&mut self, k: &str, at: u64) {
        self.next.insert(k.to_string(), at);
    }
    pub fn waiting(&self, key: &str, now: u64) -> bool {
        self.waits.iter().any(|w| w.key == key && now < w.retry_at)
    }
    pub fn wait(&mut self, key: String, why: String, now: u64) {
        match self.waits.iter_mut().find(|w| w.key == key) {
            Some(w) => {
                w.why = why;
                w.retry_at = now + WAIT_RETRY_SECS;
                w.tries += 1;
            }
            None => {
                self.waits.push(Wait { key, since: now, why, retry_at: now + WAIT_RETRY_SECS, tries: 1 });
                if self.waits.len() > WAITS_KEPT {
                    self.waits.remove(0);
                }
            }
        }
    }
    pub fn clear_wait(&mut self, key: &str) {
        self.waits.retain(|w| w.key != key);
    }
    pub fn roll_day(&mut self, now: u64) {
        let d = now / 86_400;
        if d != self.day {
            self.day = d;
            self.signed_today = 0;
        }
    }
    pub fn note(&mut self, kind: &str, now: u64, what: String) {
        self.last.retain(|(k, ..)| k != kind);
        self.last.push((kind.to_string(), now, what));
    }
    /// Adds to what a collection (or ops) is owed.
    pub fn owe(&mut self, o: Owed) {
        match o {
            Owed::Surplus { coll, lamports } => match self.owed.iter_mut().find(|x| matches!(x, Owed::Surplus { coll: c, .. } if *c == coll)) {
                Some(Owed::Surplus { lamports: l, .. }) => *l = l.saturating_add(lamports),
                _ => self.owed.push(Owed::Surplus { coll, lamports }),
            },
            Owed::Ops { lamports } => match self.owed.iter_mut().find(|x| matches!(x, Owed::Ops { .. })) {
                Some(Owed::Ops { lamports: l }) => *l = l.saturating_add(lamports),
                _ => self.owed.push(Owed::Ops { lamports }),
            },
        }
    }
    /// A landed transfer: what it paid is no longer owed.
    pub fn paid(&mut self, step: &TreasuryStep) {
        match step {
            TreasuryStep::Surplus { coll, lamports, .. } => {
                for o in self.owed.iter_mut() {
                    if let Owed::Surplus { coll: c, lamports: l } = o {
                        if c == coll {
                            *l = l.saturating_sub(*lamports);
                        }
                    }
                }
            }
            TreasuryStep::Ops { lamports, .. } => {
                for o in self.owed.iter_mut() {
                    if let Owed::Ops { lamports: l } = o {
                        *l = l.saturating_sub(*lamports);
                    }
                }
            }
            _ => {}
        }
        self.owed.retain(|o| !matches!(o, Owed::Surplus { lamports: 0, .. } | Owed::Ops { lamports: 0 }));
    }
}

#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum StableCranks {
    V1(CranksState),
}

pub fn encode_state(s: &CranksState) -> Vec<u8> {
    candid::encode_one(StableCranks::V1(s.clone())).expect("the cranks state encodes")
}

pub fn decode_state(bytes: &[u8]) -> Result<CranksState, String> {
    match candid::decode_one::<StableCranks>(bytes).map_err(|e| format!("the cranks state does not decode: {e}"))? {
        StableCranks::V1(s) => Ok(s),
    }
}

thread_local! {
    static STATE: RefCell<CranksState> = RefCell::new(CranksState::default());
}

pub fn with<R>(f: impl FnOnce(&CranksState) -> R) -> R {
    STATE.with(|s| f(&s.borrow()))
}

pub fn with_mut<R>(f: impl FnOnce(&mut CranksState) -> R) -> R {
    STATE.with(|s| f(&mut s.borrow_mut()))
}

pub fn reset() {
    STATE.with(|c| *c.borrow_mut() = CranksState::default());
}

pub fn before_upgrade() {
    let blob = with(|s| (*s != CranksState::default()).then(|| encode_state(s)));
    crate::solhome::LINES.with(|l| l.borrow_mut().cranks_state = blob);
}

pub fn after_upgrade() -> Result<(), String> {
    match crate::solhome::LINES.with(|l| l.borrow_mut().cranks_state.take()) {
        Some(b) => {
            let s = decode_state(&b)?;
            STATE.with(|c| *c.borrow_mut() = s);
            Ok(())
        }
        None => {
            reset();
            Ok(())
        }
    }
}

// ---- the pass -----------------------------------------------------------------------------------------------------------------------

/// A step ready to sign.
pub struct Planned {
    pub step: TreasuryStep,
    pub ixs: Vec<soltx::Instruction>,
    pub keys: Vec<KeyPath>,
    pub link: Option<u64>,
}

fn books_err(e: books::BooksError) -> String {
    format!("{e}")
}

/// One pass. `now` in unix seconds.
pub async fn tick<I: CrankIo + DeskIo>(io: &mut I, now: u64) -> Vec<String> {
    let mut log = vec![];
    if with(|s| s.off) || io.sol_home().is_none() {
        return log;
    }
    if desk::with(|s| s.books.collections.is_empty()) {
        return log;
    }
    with_mut(|s| s.roll_day(now));
    settle(io, now, &mut log);
    // the books-only work, every pass it is due
    log.extend(apply(now));
    log.extend(allocate(now));
    log.extend(distribute(now));
    log.extend(resolve(io, now).await);
    log.extend(raffles(io, now).await);
    if io.paused() {
        return log;
    }
    // then ONE transaction on the crank line, only while it is free
    if let Err(e) = crate::crankline::line_free(&io.sol_line_of(LineId::Crank)) {
        if with(|s| !s.owed.is_empty()) {
            log.push(format!("treasury: waits: {e}"));
        }
        return log;
    }
    if with(|s| s.signed_today >= STEPS_PER_DAY) {
        log.push(format!("treasury: waits: {STEPS_PER_DAY} steps today already (the day's budget)"));
        return log;
    }
    match plan(io, now).await {
        Ok(Some(p)) => log.push(sign(io, p, now).await),
        Ok(None) => {}
        Err(e) => log.push(format!("treasury: waits: {e}")),
    }
    log
}

/// A pending treasury change past its delay is applied; a resume past its delay lifts its pause.
fn apply(now: u64) -> Vec<String> {
    let due: Vec<(Resume, u32)> = with_mut(|s| {
        let (due, later): (Vec<_>, Vec<_>) = s.resumes.drain(..).partition(|(_, _, at)| now >= *at);
        s.resumes = later;
        due.into_iter().map(|(r, c, _)| (r, c)).collect()
    });
    let mut out = vec![];
    for (r, c) in due {
        let done = desk::with_mut(|s| match r {
            Resume::Collection => s.books.set_paused(c as u16, false).is_ok(),
            Resume::AllMints => {
                s.books.set_keeper_paused(false);
                true
            }
            Resume::Desk => true,
        });
        if r == Resume::Desk {
            crate::pooldesk::state::with_mut(|s| {
                if let Some(d) = s.desks.get_mut(&c) {
                    d.off = false;
                }
            });
        }
        if done {
            out.push(format!("guardian: the pause of {} is lifted (its 24 h are over)", match r { Resume::Collection => format!("collection {c}"), Resume::Desk => format!("desk {c}"), Resume::AllMints => "every mint".into() }));
        }
    }
    if !with(|s| s.due("apply", now)) {
        return out;
    }
    let now_i = now as i64;
    desk::with_mut(|s| {
        let t = &mut s.books.treasury;
        for (what, r) in [("shares", t.apply_shares(now_i)), ("ops", t.apply_ops(now_i)), ("reserve cap", t.apply_reserve_cap(now_i)),
            ("raffle bps", t.apply_raffle_bps(now_i))] {
            if r.is_ok() {
                out.push(format!("treasury: the pending {what} change is applied"));
            }
        }
    });
    with_mut(|s| {
        s.set_next("apply", now + APPLY_EVERY);
        for o in &out {
            s.note("apply", now, o.clone());
        }
    });
    out
}

/// Unallocated income is split 90/10 (the shares) once it reaches the bound; the ops share is owed to the ops key.
fn allocate(now: u64) -> Vec<String> {
    if !with(|s| s.due("allocate", now)) {
        return vec![];
    }
    let unallocated = desk::with(|s| s.books.treasury.unallocated);
    with_mut(|s| s.set_next("allocate", now + ALLOCATE_EVERY));
    if unallocated < ALLOCATE_MIN_LAMPORTS {
        return vec![];
    }
    match desk::with_mut(|s| s.books.allocate()) {
        Ok(a) => {
            with_mut(|s| {
                if a.to_ops > 0 {
                    s.owe(Owed::Ops { lamports: a.to_ops });
                }
                s.note("allocate", now, format!("{} allocated: {} to the reserves pot, {} to ops", a.amount, a.to_reserves, a.to_ops));
            });
            vec![format!("treasury: {} lamports allocated ({} reserves, {} ops)", a.amount, a.to_reserves, a.to_ops)]
        }
        Err(e) => vec![format!("treasury: allocate waits: {}", books_err(e))],
    }
}

/// The daily distribution of the reserves pot to the collections by their seven-day volume: booked (surplus, raffle pot) and owed on chain.
fn distribute(now: u64) -> Vec<String> {
    if !with(|s| s.due("distribute", now)) {
        return vec![];
    }
    with_mut(|s| s.set_next("distribute", now + DISTRIBUTE_EVERY));
    if desk::with(|s| s.books.treasury.reserves_pot) == 0 {
        return vec![];
    }
    // review F1: a distribution writes every collection's raffle pot; none may have an open link
    let busy: Vec<u16> = desk::with(|s| s.books.collections.keys().copied().filter(|c| s.link.open_of(*c, None).next().is_some()).collect());
    if !busy.is_empty() {
        with_mut(|s| s.set_next("distribute", now + 300));
        return vec![format!("treasury: the distribution waits: collections {busy:?} have an unfinished chain link")];
    }
    // review F6: all or nothing: on a copy of the books, kept only when every index distributes
    let tried = desk::with_mut(|s| {
        let mut b = s.books.clone();
        let r = b.distribute_all_owed(now as i64);
        if r.is_ok() {
            s.books = b;
        }
        r
    });
    match tried {
        Ok(ds) => {
            let mut out = vec![];
            with_mut(|s| {
                for d in &ds {
                    if d.to_surplus > 0 {
                        s.owe(Owed::Surplus { coll: d.collection as u32, lamports: d.to_surplus });
                    }
                    if d.to_ops > 0 {
                        s.owe(Owed::Ops { lamports: d.to_ops });
                    }
                    out.push(format!("treasury: collection {} distributed {}: {} to its surplus, {} to its raffle, {} to ops, {} kept", d.collection, d.owed,
                        d.to_surplus, d.to_raffle, d.to_ops, d.kept));
                }
                s.note("distribute", now, format!("{} collections", ds.len()));
            });
            out
        }
        Err(books::BooksError::NothingToDo) | Err(books::BooksError::TooEarly) | Err(books::BooksError::RoundOpen) => vec![],
        Err(e) => vec![format!("treasury: distribute waits: {}", books_err(e))],
    }
}

/// The round's randomness when two relays agree on it exactly and the canister verifies the signature: `None` not yet, or refused.
async fn drand_round<I: CrankIo>(io: &mut I, round: u64, now: u64) -> Option<([u8; 32], String)> {
    let relays = io.drand_relays();
    let mut answers = Vec::new();
    for r in &relays {
        let a = io.drand_fetch(r, round).await;
        answers.push((crate::drand::host_of(r), a));
    }
    match crate::drand::judge(round, &answers, crate::drand::NEED) {
        crate::drand::Verdict::Proven { beacon, relays } => {
            with_mut(|s| {
                if s.drand_disagreement.as_ref().is_some_and(|(r, ..)| *r == round) {
                    s.drand_disagreement = None;
                }
            });
            Some((beacon.randomness, format!("drand round {round}: signature {}, randomness {}, agreed by {}", hex::encode(&beacon.signature),
                hex::encode(beacon.randomness), relays.join(" + "))))
        }
        crate::drand::Verdict::NotYet { .. } => None,
        crate::drand::Verdict::Disagree(why) => {
            with_mut(|s| s.drand_disagreement = Some((round, now, why)));
            None
        }
    }
}

/// The next redemption ticket of each collection, resolved with its drand round (one per collection a pass).
async fn resolve<I: CrankIo>(io: &mut I, now: u64) -> Vec<String> {
    if !with(|s| s.due("resolve", now)) {
        return vec![];
    }
    with_mut(|s| s.set_next("resolve", now + RESOLVE_EVERY));
    let live = books::math::round_at(now as i64);
    // (collection, ticket, the round to prove: None for a refunded ticket, stepped over)
    let due: Vec<(u16, u64, Option<u64>)> = desk::with(|s| {
        s.books.collections.iter().filter_map(|(id, c)| {
            let l = &c.ledger;
            let n = l.next_to_resolve;
            let t = l.tickets.get(&n)?;
            if s.link.open_of(*id, None).next().is_some() {
                return None; // review F1: a resolution writes the ledger; not under an open link
            }
            match t.stage {
                books::ledger::Stage::Refunded => Some((*id, n, None)),
                books::ledger::Stage::Pinned if t.round <= live => Some((*id, n, Some(t.round))),
                _ => None,
            }
        }).collect()
    });
    let mut out = vec![];
    for (id, n, round) in due {
        let (randomness, note) = match round {
            None => (None, String::new()),
            Some(r) => match drand_round(io, r, now).await {
                Some((x, note)) => (Some(x), note),
                None => {
                    out.push(format!("treasury: ticket {n} of collection {id} waits for drand round {r}"));
                    continue;
                }
            },
        };
        match desk::with_mut(|s| s.books.resolve(id, n, randomness, now as i64)) {
            Ok(books::ledger::Resolution::Skipped) => out.push(format!("treasury: ticket {n} of collection {id} was refunded: stepped over")),
            Ok(books::ledger::Resolution::Drawn { item_id, deliver_by }) => {
                with_mut(|s| s.note("resolve", now, format!("collection {id} ticket {n}: item {item_id}")));
                out.push(format!("treasury: ticket {n} of collection {id} resolved to item {item_id}, deliver by {deliver_by} ({note})"));
                // the next ticket of the same collection waits for the next pass (one drand fetch each)
                with_mut(|s| s.set_next("resolve", now));
            }
            Err(e) => out.push(format!("treasury: ticket {n} of collection {id} is not resolved: {}", books_err(e))),
        }
    }
    out
}

/// The raffles (§9.3): every listed collection gets one; a round is taken as a snapshot of the books' coins (published first, then
/// committed at a later finalized slot), drawn with its pinned drand round once two relays agree and the signature verifies, and its
/// winner paid by [`TreasuryStep::Claim`]; an unclaimed or undrawn round expires. Nothing here signs.
async fn raffles<I: CrankIo + SolIo>(io: &mut I, now: u64) -> Vec<String> {
    if !with(|s| s.due("raffle", now)) {
        return vec![];
    }
    with_mut(|s| s.set_next("raffle", now + RAFFLE_EVERY));
    let mut out = vec![];
    let now_i = now as i64;
    let ids: Vec<(u16, [u8; 32])> = desk::with(|s| s.books.collections.iter().map(|(id, c)| (*id, c.wx_mint)).collect());
    let claim_in_flight = match &io.sol_line_of(LineId::Crank).pending {
        Some(p) => match &p.step {
            LineStep::Treasury(TreasuryStep::Claim { coll, .. }) => Some(*coll),
            _ => None,
        },
        None => None,
    };
    for (id, key) in ids {
        if desk::collection_busy(id) {
            continue; // review F1: the raffle's books are in the collection's snapshot
        }
        let r = desk::with(|s| s.books.get(id).ok().and_then(|c| c.raffle.clone()));
        let Some(r) = r else {
            if desk::with_mut(|s| s.books.init_raffle(id, key)).is_ok() {
                out.push(format!("treasury: collection {id}'s raffle is opened (its key is its wX mint)"));
            }
            continue;
        };
        // review F5: a prize whose claim is out on the line is not expired under it
        if claim_in_flight == Some(id as u32) {
            continue;
        }
        if let Ok(drawn) = desk::with_mut(|s| s.books.expire_raffle(id, now_i)) {
            with_mut(|s| {
                s.claims.remove(&(id as u32));
            });
            out.push(if drawn { format!("treasury: collection {id}'s raffle round {} expired unclaimed: the prize stays for the next round", r.round) }
                else { format!("treasury: collection {id}'s raffle round {} was not drawn in time: cancelled", r.round) });
            continue;
        }
        match r.phase {
            books::raffle::RafflePhase::Idle => {
                if r.prize < ONE || !(r.committed_at == 0 || now_i >= r.committed_at + books::raffle::RAFFLE_PERIOD) {
                    continue;
                }
                let pending = with(|s| s.commits.get(&(id as u32)).cloned());
                match pending {
                    None => {
                        // the snapshot of the books' coins, published BEFORE anything is committed (what keeps the draw honest)
                        match crate::raffle_books::take(io, id, r.round + 1).await {
                            Ok(snap) => {
                                let pc = PendingCommit { round: snap.round, root: snap.root, total_weight: snap.total_weight, snapshot_slot: snap.slot,
                                    entries: snap.leaves.len() as u32, taken_at: now };
                                io.raffle_publish(snap);
                                with_mut(|s| {
                                    s.commits.insert(id as u32, pc.clone());
                                    s.set_next("raffle", now);
                                });
                                out.push(format!("treasury: collection {id}'s raffle round {}: snapshot of {} holders at slot {} published; committed at the next \
                                    finalized slot", pc.round, pc.entries, pc.snapshot_slot));
                            }
                            Err(e) if e.soft => {}
                            Err(e) => out.push(format!("treasury: collection {id}'s raffle snapshot is not taken: {}", e.why)),
                        }
                    }
                    Some(pc) => {
                        let (_, slot) = match io.sol_clock().await {
                            Ok(c) => c,
                            Err(e) => {
                                out.push(format!("treasury: collection {id}'s raffle commit waits: {e}"));
                                continue;
                            }
                        };
                        if slot <= pc.snapshot_slot {
                            with_mut(|s| s.set_next("raffle", now + 30));
                            continue;
                        }
                        match desk::with_mut(|s| s.books.commit_raffle(id, pc.root, pc.total_weight, pc.snapshot_slot, pc.entries, slot, now_i)) {
                            Ok(drand) => {
                                with_mut(|s| {
                                    s.commits.remove(&(id as u32));
                                    s.note("raffle", now, format!("collection {id} round {} committed, drand round {drand}", pc.round));
                                });
                                out.push(format!("treasury: collection {id}'s raffle round {} committed: drawn with drand round {drand}", pc.round));
                            }
                            Err(e) => {
                                with_mut(|s| {
                                    s.commits.remove(&(id as u32));
                                });
                                out.push(format!("treasury: collection {id}'s raffle commit refused ({}): the snapshot is taken again", books_err(e)));
                            }
                        }
                    }
                }
            }
            books::raffle::RafflePhase::Committed => {
                if r.drand_round > books::math::round_at(now_i) {
                    continue;
                }
                let Some((randomness, note)) = drand_round(io, r.drand_round, now).await else {
                    out.push(format!("treasury: collection {id}'s raffle waits for drand round {}", r.drand_round));
                    continue;
                };
                match desk::with_mut(|s| s.books.draw_raffle(id, r.drand_round, &randomness, now_i)) {
                    Ok(ticket) => {
                        let snap = io.raffle_published(&key, r.round);
                        let winner = snap.as_ref().and_then(|s| crate::raffle_books::winner(s, &randomness, &r.key, r.round));
                        match winner {
                            Some((leaf, proof)) => {
                                with_mut(|s| {
                                    s.claims.insert(id as u32, PendingClaim { round: r.round, owner: leaf.owner, cum_start: leaf.cum_start, weight: leaf.weight, proof });
                                    s.note("draw", now, format!("collection {id} round {}: ticket {ticket}, winner {}", r.round, b58_encode(&leaf.owner)));
                                });
                                out.push(format!("treasury: collection {id}'s raffle round {} drawn: ticket {ticket}, the winner is {} ({note})", r.round,
                                    b58_encode(&leaf.owner)));
                            }
                            None => out.push(format!("treasury: collection {id}'s raffle round {} drawn (ticket {ticket}) but its published snapshot is \
                                missing or names no leaf for it: the prize expires with the round", r.round)),
                        }
                    }
                    Err(e) => out.push(format!("treasury: collection {id}'s raffle draw refused: {}", books_err(e))),
                }
            }
            books::raffle::RafflePhase::Drawn => {}
        }
    }
    out
}

/// Begins a link of the treasury's (idempotent by a fresh key): its id, its memo and the booking's number.
fn begin(coll: u16, kind: OpKind, now: u64, f: impl FnOnce(&mut books::Books) -> Result<(Effects, u64), books::BooksError>) -> Result<(u64, String, u64), String> {
    let op = with_mut(|s| {
        s.ops += 1;
        s.ops
    });
    let spec = Spec { key: format!("treasury:{}:{coll}:{op}", kind.tag()), kind, coll: Some(coll), intent: None, scope: Scope::Module };
    let (id, _) = desk::with_mut(|s| s.link.begin(&mut s.books, spec, now, f)).map_err(|e| match e {
        crate::chainlink::ledger::LinkError::Books(b) => format!("the books refused: {b}"),
        crate::chainlink::ledger::LinkError::Busy(c) => format!("collection {c} has an unfinished operation"),
        crate::chainlink::ledger::LinkError::Halted(c) => format!("collection {c} is stopped by an incident"),
    })?;
    let (memo, aux) = desk::with(|s| s.link.get(id).map(|l| (l.memo.clone(), l.aux)).unwrap_or_default());
    Ok((id, memo, aux))
}

/// The one transaction of this pass: an owed surplus, the owed ops share, or a raffle's prize.
async fn plan<I: CrankIo + DeskIo>(io: &mut I, now: u64) -> Result<Option<Planned>, String> {
    let tre = io.key_of(&KeyPath::Treasury).await?;
    // 0. a raffle's winner waiting for the prize: one whole wX from the treasury's wX account to the winner's
    let claims: Vec<(u32, PendingClaim)> = with(|s| s.claims.iter().map(|(c, p)| (*c, p.clone())).collect());
    for (coll, c) in claims {
        let step = TreasuryStep::Claim { coll, winner: c.owner, round: c.round };
        if with(|s| s.waiting(&step.key(), now)) {
            continue;
        }
        let wx = desk::with(|s| s.books.get(coll as u16).map(|x| x.wx_mint)).map_err(books_err)?;
        let main = io.key_of(&KeyPath::Main).await?;
        let (from, to) = (associated_token_address(&tre, &wx), associated_token_address(&c.owner, &wx));
        let ixs = vec![spl::create_ata_idempotent(main, c.owner, wx), spl::transfer_checked(from, wx, to, tre, ONE, 9)];
        return Ok(Some(Planned { ixs, keys: vec![KeyPath::Treasury], link: None, step }));
    }
    // 1. a collection's surplus owed
    let owed: Vec<Owed> = with(|s| s.owed.clone());
    for o in &owed {
        if let Owed::Surplus { coll, lamports } = o {
            let step0 = TreasuryStep::Surplus { coll: *coll, lamports: *lamports, link: 0 };
            if *lamports >= SURPLUS_MIN_LAMPORTS && !with(|s| s.waiting(&step0.key(), now)) && !desk::collection_busy(*coll as u16) {
                let module = io.key_of(&KeyPath::Coll(*coll)).await?;
                // review F2: a link: the module's surplus is credited by `fund_reserve` only when the transfer lands
                let (id, lamports) = (*coll as u16, *lamports);
                match begin(*coll as u16, OpKind::Sweep, now, |b| {
                    b.fund_reserve(id, lamports)?;
                    Ok((Effects { lamports_in: lamports, ..Default::default() }, lamports))
                }) {
                    Ok((link, memo, _)) => {
                        let step = TreasuryStep::Surplus { coll: *coll, lamports, link };
                        return Ok(Some(Planned { ixs: vec![soltx::transfer(tre, module, lamports), spl::memo(&memo, &[])], keys: vec![KeyPath::Treasury], link: Some(link), step }));
                    }
                    Err(e) => with_mut(|s| s.wait(step0.key(), e, now)),
                }
            }
        }
    }
    // 2. the ops share owed, to the ops key the books name
    if let Some(Owed::Ops { lamports }) = owed.iter().find(|o| matches!(o, Owed::Ops { .. })) {
        if *lamports >= OPS_MIN_LAMPORTS {
            if let Some(to) = desk::with(|s| s.books.treasury.ops) {
                let step = TreasuryStep::Ops { to, lamports: *lamports };
                if !with(|s| s.waiting(&step.key(), now)) {
                    return Ok(Some(Planned { ixs: vec![soltx::transfer(tre, to, *lamports)], keys: vec![KeyPath::Treasury], link: None, step }));
                }
            }
        }
    }
    // 3. SOL fees at the claimer (ROOK's): swept to the treasury, booked when they land
    let fc = io.key_of(&KeyPath::Claimer).await?;
    let step = TreasuryStep::ClaimerSweep { lamports: 0 };
    if !with(|s| s.waiting(&step.key(), now)) {
        let bal = io.sol_accounts(&[fc], 0).await?.first().and_then(|a| a.as_ref()).map(|a| a.lamports).unwrap_or(0);
        let free = bal.saturating_sub(KEY_RENT_LAMPORTS);
        if free >= CLAIMER_SWEEP_MIN_LAMPORTS {
            return Ok(Some(Planned { ixs: vec![soltx::transfer(fc, tre, free)], keys: vec![KeyPath::Claimer], link: None, step: TreasuryStep::ClaimerSweep { lamports: free } }));
        }
    }
    // 4. a raffle whose pot covers a prize and holds none: the pot buys one whole wX at the module's mint (a link)
    let wants: Vec<u16> = desk::with(|s| {
        s.books.collections.iter().filter(|(_, c)| c.raffle.as_ref().is_some_and(|r| r.prize < ONE && r.pot > 0)).map(|(id, _)| *id).collect()
    });
    for id in wants {
        let step0 = TreasuryStep::Prize { coll: id as u32, price: 0, link: 0 };
        if with(|s| s.waiting(&step0.key(), now)) {
            continue;
        }
        let (wx, module) = (desk::with(|s| s.books.get(id).map(|c| c.wx_mint)).map_err(books_err)?, io.key_of(&KeyPath::Coll(id as u32)).await?);
        let main = io.key_of(&KeyPath::Main).await?; // review F7: every await before the link is begun
        let op = with_mut(|s| {
            s.ops += 1;
            s.ops
        });
        let spec = Spec { key: format!("prize:{id}:{op}"), kind: OpKind::Prize, coll: Some(id), intent: None, scope: Scope::Module };
        let begun = desk::with_mut(|s| {
            s.link.begin(&mut s.books, spec, now, |b| {
                let p = b.fund_prize(id, now as i64)?;
                Ok((Effects { mint_units: p.units, lamports_in: p.price, ..Default::default() }, p.price))
            })
        });
        let (link, _) = match begun {
            Ok(x) => x,
            Err(crate::chainlink::ledger::LinkError::Books(books::BooksError::NotEnoughBacked)) | Err(crate::chainlink::ledger::LinkError::Books(books::BooksError::BadInput)) => continue,
            Err(e) => {
                let why = match e {
                    crate::chainlink::ledger::LinkError::Books(b) => format!("{b}"),
                    crate::chainlink::ledger::LinkError::Busy(c) => format!("collection {c} has an unfinished operation"),
                    crate::chainlink::ledger::LinkError::Halted(c) => format!("collection {c} is stopped by an incident"),
                };
                with_mut(|s| s.wait(step0.key(), why, now));
                continue;
            }
        };
        let (price, memo) = desk::with(|s| s.link.get(link).map(|l| (l.aux, l.memo.clone())).unwrap_or_default());
        let ata = associated_token_address(&tre, &wx);
        let ixs = vec![spl::create_ata_idempotent(main, tre, wx), soltx::transfer(tre, module, price), spl::mint_to(wx, ata, module, ONE), spl::memo(&memo, &[])];
        let step = TreasuryStep::Prize { coll: id as u32, price, link };
        return Ok(Some(Planned { ixs, keys: vec![KeyPath::Treasury, KeyPath::Coll(id as u32)], link: Some(link), step }));
    }
    // 5. review F11: a ticket whose refund is due: ONE wX re-minted to its owner (a link; the books' rules decide, A2-04 included)
    if with(|s| s.due("remint", now)) {
        with_mut(|s| s.set_next("remint", now + REMINT_EVERY));
        let live = books::math::round_at(now as i64);
        let due: Vec<(u16, u64, Pubkey, bool)> = desk::with(|s| {
            s.books.collections.iter().flat_map(|(id, c)| {
                let open = s.link.open_of(*id, None).next().is_some();
                c.ledger.tickets.values().filter(move |t| !open && refund_due(t, now as i64, live)).map(|t| (*id, t.id, t.owner, t.round <= live)).collect::<Vec<_>>()
            }).collect()
        });
        for (id, ticket, owner, proven) in due {
            let step0 = TreasuryStep::Remint { coll: id as u32, ticket, owner, link: 0 };
            if with(|s| s.waiting(&step0.key(), now)) {
                continue;
            }
            let (wx, module) = (desk::with(|s| s.books.get(id).map(|c| c.wx_mint)).map_err(books_err)?, io.key_of(&KeyPath::Coll(id as u32)).await?);
            let main = io.key_of(&KeyPath::Main).await?;
            match begin(id, OpKind::Remint, now, |b| {
                let to = b.refund(id, ticket, proven, now as i64)?;
                Ok((Effects { mint_units: ONE, ..Default::default() }, if to == owner { 0 } else { 1 }))
            }) {
                Ok((link, memo, _)) => {
                    with_mut(|s| s.set_next("remint", now)); // the next due ticket next pass
                    let ata = associated_token_address(&owner, &wx);
                    let ixs = vec![spl::create_ata_idempotent(main, owner, wx), spl::mint_to(wx, ata, module, ONE), spl::memo(&memo, &[])];
                    let step = TreasuryStep::Remint { coll: id as u32, ticket, owner, link };
                    return Ok(Some(Planned { ixs, keys: vec![KeyPath::Coll(id as u32)], link: Some(link), step }));
                }
                Err(e) => with_mut(|s| s.wait(step0.key(), e, now)),
            }
        }
    }
    // 6. review F11: a module's revenue above its target (and its surplus once escaped) to the treasury, booked as income (a link)
    if with(|s| s.due("skim", now)) {
        with_mut(|s| s.set_next("skim", now + SKIM_EVERY));
        let due: Vec<(u16, u64)> = desk::with(|s| {
            s.books.collections.iter().filter_map(|(id, c)| {
                let mut m = c.module.clone();
                let amount = m.skim(now as i64).ok()?;
                (amount >= SKIM_MIN_LAMPORTS && s.link.open_of(*id, None).next().is_none()).then_some((*id, amount))
            }).collect()
        });
        for (id, amount) in due {
            let step0 = TreasuryStep::Skim { coll: id as u32, lamports: amount, link: 0 };
            if with(|s| s.waiting(&step0.key(), now)) {
                continue;
            }
            let module = io.key_of(&KeyPath::Coll(id as u32)).await?;
            match begin(id, OpKind::Sweep, now, |b| {
                let lamports = b.skim(id, now as i64)?;
                Ok((Effects { lamports_out: lamports, ..Default::default() }, lamports))
            }) {
                Ok((link, memo, lamports)) => {
                    with_mut(|s| s.set_next("skim", now));
                    let step = TreasuryStep::Skim { coll: id as u32, lamports, link };
                    return Ok(Some(Planned { ixs: vec![soltx::transfer(module, tre, lamports), spl::memo(&memo, &[])], keys: vec![KeyPath::Coll(id as u32)], link: Some(link), step }));
                }
                Err(e) => with_mut(|s| s.wait(step0.key(), e, now)),
            }
        }
    }
    Ok(None)
}

/// Review F11: the ledger's refund rules (`books::ledger::Ledger::refund`), read without writing: a pinned ticket past its timeout whose
/// round is still ahead (a public round is never refunded, audit A2-04), or a resolved one past its grace that was marked undeliverable
/// or never sent within the late-delivery window.
fn refund_due(t: &books::ledger::Ticket, now: i64, live: u64) -> bool {
    use books::ledger::{Stage, LATE_DELIVERY_WINDOW, PIN_TIMEOUT, REFUND_GRACE};
    match t.stage {
        Stage::Pinned => now >= t.pinned_at + PIN_TIMEOUT && t.round > live,
        Stage::Resolved => now >= t.deliver_by + REFUND_GRACE && (t.undeliverable || (t.sent_tx == [0u8; 32] && now >= t.deliver_by + LATE_DELIVERY_WINDOW)),
        _ => false,
    }
}

async fn sign<I: CrankIo + DeskIo>(io: &mut I, p: Planned, now: u64) -> String {
    let label = p.step.label();
    let key = p.step.key();
    let before = io.sol_line_of(LineId::Crank).line.error.clone();
    let step = LineStep::Treasury(p.step.clone());
    let r = match p.link {
        Some(id) => link::sign_linked(io, LineOwner::Treasury, step, id, p.ixs, p.keys, now).await.map_err(|e| match e {
            SignError::Refused(b) => format!("REFUSED by pre_sign: {b}"),
            SignError::Deferred(s) => s,
        }),
        None => link::sign_plain(io, LineOwner::Treasury, step, p.ixs, p.keys, now).await,
    };
    match r {
        Ok(sig) => {
            with_mut(|s| {
                s.signed_today += 1;
                s.clear_wait(&key);
            });
            format!("{label}: signed and sent ({sig})")
        }
        Err(e) => {
            if e.contains(crate::solsend::SIMULATION_REFUSED) && !e.contains("insufficient_funds") {
                let mut l = io.sol_line_of(LineId::Crank);
                l.line.error = before;
                io.sol_line_save_of(LineId::Crank, &l);
            }
            with_mut(|s| s.wait(key, e.clone(), now));
            format!("{label}: not signed: {e}")
        }
    }
}

/// What the crank line reported of the treasury's steps.
pub fn settle<I: DeskIo>(io: &mut I, now: u64, log: &mut Vec<String>) {
    let mut l = io.sol_line_of(LineId::Crank);
    let mine = l.take(LineOwner::Treasury);
    if !mine.is_empty() {
        io.sol_line_save_of(LineId::Crank, &l);
    }
    for o in mine {
        let LineStep::Treasury(ts) = &o.step else { continue };
        match o.ok {
            Some(true) => {
                if let Some(id) = ts.link() {
                    desk::with_mut(|s| s.link.landed(&mut s.books, id, &o.signature, now));
                }
                if let TreasuryStep::ClaimerSweep { lamports } = ts {
                    let _ = desk::with_mut(|s| s.books.receive(*lamports));
                }
                if let TreasuryStep::Claim { coll, round, winner } = ts {
                    // review F4: the prize left the treasury's account: the claim is over whatever the books say; a refusal is an incident
                    let c = with_mut(|s| s.claims.remove(coll));
                    let r = match c.filter(|c| c.round == *round) {
                        // audit B1-06: owed while the collection has an open link (its undo would wipe it), applied once it is decided
                        Some(c) => desk::book_outcome(*coll as u16, OwedWrite::ClaimRaffle { owner: c.owner, cum_start: c.cum_start, weight: c.weight,
                            proof: c.proof, now: now as i64 }),
                        None => Err(books::BooksError::NothingPending),
                    };
                    if let Err(e) = r {
                        let why = format!("the prize of raffle {coll} round {round} was PAID to {} but the books refuse the claim: {} (OPERATOR: the books' prize is unpaid)",
                            b58_encode(winner), books_err(e));
                        with_mut(|s| s.wait(format!("claim_paid:{coll}:{round}"), why.clone(), now));
                        log.push(format!("treasury: {why}"));
                    }
                }
                with_mut(|s| {
                    s.paid(ts);
                    s.clear_wait(&ts.key());
                    s.note(ts.kind(), now, ts.label());
                });
                log.push(format!("{} landed ({})", ts.label(), o.signature));
            }
            Some(false) => {
                if let Some(id) = ts.link() {
                    desk::with_mut(|s| {
                        let crate::chainlink::driver::DeskStateRef { books, link } = crate::chainlink::driver::DeskStateRef::of(s);
                        link.undo(books, id, Phase::Failed, &format!("landed and failed on chain ({})", o.signature), now);
                    });
                }
                with_mut(|s| s.wait(ts.key(), format!("FAILED on chain ({})", o.signature), now));
                log.push(format!("{} FAILED on chain ({}): tried again in {} min", ts.label(), o.signature, WAIT_RETRY_SECS / 60));
            }
            None => {
                if let Some(id) = ts.link() {
                    desk::with_mut(|s| {
                        let crate::chainlink::driver::DeskStateRef { books, link } = crate::chainlink::driver::DeskStateRef::of(s);
                        link.undo(books, id, Phase::Undone, "proven never to land (its nonce burned or its blockhash expired)", now);
                    });
                }
                log.push(format!("{} never landed: owed still, tried again", ts.label()));
            }
        }
    }
}

// ---- the IC glue --------------------------------------------------------------------------------------------------------------------

pub(crate) struct CranksNet {
    sol: Option<crate::solhome::SolNet>,
    relays: Vec<String>,
}

crate::sol_io_via!(CranksNet, sol);

impl DeskIo for CranksNet {
    async fn key_of(&mut self, p: &KeyPath) -> Result<Pubkey, String> {
        crate::solkey::derive_key(p).await
    }
}

impl CrankIo for CranksNet {
    fn now(&self) -> u64 {
        ic_cdk::api::time() / 1_000_000_000
    }
    fn paused(&self) -> bool {
        crate::canister::is_paused()
    }
    fn drand_relays(&self) -> Vec<String> {
        self.relays.clone()
    }
    async fn drand_fetch(&mut self, base: &str, round: u64) -> Result<serde_json::Value, String> {
        crate::drand::fetch_ic(base, round).await
    }
}

/// The cranks' pass for the canister's timer and the runner's tick, on the books home only.
pub(crate) async fn tick_ic() -> Vec<String> {
    let Some(home) = crate::canister::cfg().home else { return vec![] };
    let crate::canister::Home::Solana { drand_urls, .. } = &home;
    let relays = drand_urls.clone().unwrap_or_else(|| crate::drand::DEFAULT_RELAYS.iter().map(|s| s.to_string()).collect());
    match crate::solhome::SolNet::of_install() {
        Ok(Some(sol)) => {
            let now = ic_cdk::api::time() / 1_000_000_000;
            tick(&mut CranksNet { sol: Some(sol), relays }, now).await
        }
        Ok(None) => vec![],
        Err(e) => vec![format!("treasury: waits: {e}")],
    }
}

// ---- the surface ----------------------------------------------------------------------------------------------------------------

/// The treasury and the cranks as the operator and the site see them (§10.1 `treasury()`).
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct TreasuryStatus {
    pub treasury: books::treasury::Treasury,
    pub owed: Vec<Owed>,
    pub waits: Vec<Wait>,
    pub last: Vec<(String, u64, String)>,
    pub commits: Vec<(u32, PendingCommit)>,
    pub claims: Vec<(u32, PendingClaim)>,
    pub resumes: Vec<(Resume, u32, u64)>,
    pub drand_disagreement: Option<(u64, u64, String)>,
    pub off: bool,
}

#[ic_cdk::query]
pub fn treasury_status() -> crate::cranks::TreasuryStatus {
    let treasury = desk::with(|s| s.books.treasury.clone());
    with(|s| TreasuryStatus { treasury, owed: s.owed.clone(), waits: s.waits.clone(), last: s.last.clone(),
        commits: s.commits.iter().map(|(c, p)| (*c, p.clone())).collect(), claims: s.claims.iter().map(|(c, p)| (*c, p.clone())).collect(),
        resumes: s.resumes.clone(), drand_disagreement: s.drand_disagreement.clone(), off: s.off })
}

/// A collection's raffle as the books hold it (§10.1 `raffle(coll)`).
#[ic_cdk::query]
pub fn raffle_status(coll: u16) -> Option<crate::books::raffle::Raffle> {
    desk::with(|s| s.books.get(coll).ok().and_then(|c| c.raffle.clone()))
}

fn now_secs() -> u64 {
    ic_cdk::api::time() / 1_000_000_000
}

/// The operator proposes the treasury's shares (reserves, ops, in bps): applied by the cranks after the books' 24 h.
#[ic_cdk::update]
pub fn treasury_set_shares(reserves_bps: u16, ops_bps: u16) -> Result<i64, String> {
    crate::canister::only_operator();
    crate::canister::not_paused()?;
    desk::with_mut(|s| s.books.treasury.set_shares(reserves_bps, ops_bps, now_secs() as i64)).map_err(books_err)
}

/// The operator names the ops key (the fueler's hot key, base58): the first at once, a change after the books' 24 h.
#[ic_cdk::update]
pub fn treasury_set_ops(ops: String) -> Result<i64, String> {
    crate::canister::only_operator();
    crate::canister::not_paused()?;
    let k = soltx::pubkey(&ops)?;
    desk::with_mut(|s| s.books.treasury.set_ops(k, now_secs() as i64)).map_err(books_err)
}

#[ic_cdk::update]
pub fn treasury_set_reserve_cap(bps: u32) -> Result<i64, String> {
    crate::canister::only_operator();
    crate::canister::not_paused()?;
    desk::with_mut(|s| s.books.treasury.set_reserve_cap(bps, now_secs() as i64)).map_err(books_err)
}

#[ic_cdk::update]
pub fn treasury_set_raffle_bps(bps: u16) -> Result<i64, String> {
    crate::canister::only_operator();
    crate::canister::not_paused()?;
    desk::with_mut(|s| s.books.treasury.set_raffle_bps(bps, now_secs() as i64)).map_err(books_err)
}

/// The operator switches the cranks off (nothing signs, nothing is booked) or on again.
#[ic_cdk::update]
pub fn cranks_set_off(off: bool) -> Result<String, String> {
    crate::canister::only_operator();
    with_mut(|s| s.off = off);
    Ok(format!("the cranks are {}", if off { "off" } else { "on" }))
}

/// §10.4, a guardian's method (stop-only, effective at once): collection `coll`'s public mint and intents close; burns stay open by the
/// module's own rules; redemptions keep running. Anyone trusted may stop it.
#[ic_cdk::update]
pub fn pause_collection(coll: u16) -> Result<String, String> {
    crate::canister::only_operator();
    collection_pause(coll, true, now_secs())
}

/// §10.4: collection `coll`'s pool desk stops (its position stays; nothing signs for it).
#[ic_cdk::update]
pub fn pause_desk(coll: u32) -> Result<String, String> {
    crate::canister::only_operator();
    crate::pooldesk::state::with_mut(|s| s.desks.entry(coll).or_default().off = true);
    with_mut(|s| s.resumes.retain(|(r, c, _)| !(*r == Resume::Desk && *c == coll)));
    Ok(format!("desk {coll} is paused"))
}

/// §10.4: every collection's mint closes (the keeper switch: burns stay open).
#[ic_cdk::update]
pub fn pause_all_mints() -> Result<String, String> {
    crate::canister::only_operator();
    desk::with_mut(|s| s.books.set_keeper_paused(true));
    with_mut(|s| s.resumes.retain(|(r, ..)| *r != Resume::AllMints));
    Ok("every mint is paused".into())
}

/// §10.4: a pause lifts 24 h after the resume is asked (the cranks apply it); asked again, the clock restarts.
#[ic_cdk::update]
pub fn resume(what: crate::cranks::Resume, coll: u32) -> Result<String, String> {
    crate::canister::only_operator();
    crate::canister::not_paused()?;
    Ok(schedule_resume(what, coll, now_secs()))
}

/// The one way a pause lifts (audit B3-01): scheduled [`RESUME_DELAY_SECS`] ahead, applied by the cranks; asked again, the clock restarts.
pub fn schedule_resume(what: Resume, coll: u32, now: u64) -> String {
    let at = now + RESUME_DELAY_SECS;
    with_mut(|s| {
        s.resumes.retain(|(r, c, _)| !(*r == what && *c == coll));
        s.resumes.push((what, coll, at));
    });
    format!("{what:?} {coll} resumes at {at} (unix), unless paused again before")
}

/// A collection's public mint paused at once, or its resume scheduled a day ahead (audit B3-01: the operator's `desk_pause_collection`
/// and the guardian's `pause_collection` share this; no method lifts a pause at once).
pub fn collection_pause(coll: u16, paused: bool, now: u64) -> Result<String, String> {
    if paused {
        desk::with_mut(|s| s.books.set_paused(coll, true)).map_err(books_err)?;
        with_mut(|s| s.resumes.retain(|(r, c, _)| !(*r == Resume::Collection && *c == coll as u32)));
        Ok(format!("collection {coll}: its mint is paused"))
    } else {
        desk::with(|s| s.books.get(coll).map(|_| ())).map_err(books_err)?;
        Ok(schedule_resume(Resume::Collection, coll as u32, now))
    }
}
