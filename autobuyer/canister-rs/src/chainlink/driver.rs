//! The async side of the links, generic over [`DeskIo`] (the IC's world in the canister, a fake chain in the tests): read the chain at
//! finalized, pass the gate, sign on the desk line, settle what the line reports, and reconcile.
//!
//! State is read and written only in synchronous sections of `desk::state` (copy out, await, write back): nothing is held across an
//! await, and the record of an operation is written before the await that signs it.
use super::gate::{self, ChainView};
use super::ledger::{Link, Phase};
use super::memo;
use super::recon::{self, Report};
use crate::books::BooksError;
use crate::desk::state::{with, with_mut};
use crate::solhome::{self, LineId, LineOp, LineOwner, LineStep, SolIo};
use crate::solix2::decode::Mint;
use crate::solkey::KeyPath;
use crate::soltx::{Instruction, Pubkey};

/// The desk's world: the Solana reads and the send lines ([`SolIo`]), and the public key of any of the canister's paths.
#[allow(async_fn_in_trait)]
pub trait DeskIo: SolIo {
    async fn key_of(&mut self, p: &KeyPath) -> Result<Pubkey, String>;
}

/// A signed link older than this with no word from its line or the chain is [`Phase::Stuck`] (seconds).
pub const ORPHAN_SECS: u64 = 900;
/// Signatures read by one memo scan, at most.
pub const SCAN_LIMIT: usize = 100;

/// Why an operation was not signed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SignError {
    /// The gate refused (an incident: the collection is stopped, the link undone).
    Refused(BooksError),
    /// Not signed for now (the line is busy or opening, the chain cannot be read, the simulation refused, the signing queue is full):
    /// the link is undone and the caller starts afresh next tick.
    Deferred(String),
}

/// A system account's rent-exempt minimum for `space` bytes, read once and kept.
pub async fn rent_of<I: DeskIo>(io: &mut I, space: u64) -> Result<u64, String> {
    if let Some(r) = with(|s| s.rent.get(&space).copied()) {
        return Ok(r);
    }
    let r = io.sol_rent(space).await?;
    with_mut(|s| s.rent.insert(space, r));
    Ok(r)
}

/// `M[i]` and the wX mint of collection `coll`.
pub async fn module_address<I: DeskIo>(io: &mut I, coll: u16) -> Result<Pubkey, String> {
    io.key_of(&KeyPath::Coll(coll as u32)).await
}

/// Collection `coll` at finalized: `M[i]`'s lamports above its rent, and the wX supply.
pub async fn read_view<I: DeskIo>(io: &mut I, coll: u16) -> Result<ChainView, String> {
    let mint = with(|s| s.books.get(coll).map(|c| c.wx_mint)).map_err(|e| format!("{e}"))?;
    let m = module_address(io, coll).await?;
    let rent0 = rent_of(io, 0).await?;
    let a = io.sol_accounts(&[m, mint], 82).await?;
    let free = match &a[0] {
        Some(acc) if acc.owner == crate::soltx::SYSTEM_PROGRAM => acc.lamports.saturating_sub(rent0),
        Some(_) => return Err(format!("M[{coll}] is not a system account")),
        None => 0,
    };
    let acc = a[1].as_ref().ok_or_else(|| format!("the wX mint of collection {coll} does not exist"))?;
    let supply = Mint::decode(&acc.owner, &acc.data)?.supply;
    Ok(ChainView { coll, free_lamports: free, supply })
}

/// Sign link `id` on the desk line: the gate (the chain read at finalized, then [`gate::pre_sign`]), the line's turn, the signature;
/// the link becomes Signed. Anything short of a signature undoes the link (the books are as before).
pub async fn sign_recorded<I: DeskIo>(io: &mut I, id: u64, ixs: Vec<Instruction>, keys: Vec<KeyPath>, now: u64) -> Result<String, SignError> {
    let (coll, label, memo_text, has_effect) = with(|s| {
        s.link.get(id).map(|l| (l.coll, l.memo.clone(), l.memo.clone(), !l.effects.is_none()))
    }).ok_or_else(|| SignError::Deferred(format!("no link {id}")))?;
    let undo = |why: String, phase: Phase| with_mut(|s| {
        let DeskStateRef { books, link } = DeskStateRef::of(s);
        link.undo(books, id, phase, &why, now);
    });
    // 1. the chain at finalized (only an operation that moves money at a collection needs it)
    let view = match coll {
        Some(c) if has_effect => match read_view(io, c).await {
            Ok(v) => Some(v),
            Err(e) => {
                undo(format!("the chain was not readable: {e}"), Phase::Undone);
                return Err(SignError::Deferred(format!("{memo_text}: the chain is not readable: {e}")));
            }
        },
        _ => None,
    };
    // 2. the gate: book first, then sign, and only if every guard holds against the chain
    if let Some(v) = view {
        let gate = with(|s| gate::pre_sign(&s.books, &s.link, id, &v, now as i64));
        if let Err(e) = gate {
            let why = format!("pre_sign refused {memo_text}: {e} (chain: free {} lamports, supply {})", v.free_lamports, v.supply);
            with_mut(|s| {
                let DeskStateRef { books, link } = DeskStateRef::of(s);
                link.undo(books, id, Phase::Undone, &why, now);
                if let Some(c) = coll {
                    link.halt(books, c, &why, now);
                }
            });
            return Err(SignError::Refused(e));
        }
    } else if let Err(e) = with(|s| gate::pre_sign(&s.books, &s.link, id, &ChainView { coll: coll.unwrap_or(0), free_lamports: 0, supply: 0 }, now as i64)) {
        undo(format!("pre_sign refused {memo_text}: {e}"), Phase::Undone);
        return Err(SignError::Refused(e));
    }
    // 3. the line's turn (an incident, another flow's turn, the nonce account's opening) and the signature
    if let Err(e) = solhome::ready(io, LineOwner::Desk, now, &label).await {
        undo(format!("the desk line was not ready: {e}"), Phase::Undone);
        return Err(SignError::Deferred(e));
    }
    let step = LineStep::Op(LineOp { owner: LineOwner::Desk, what: label, id });
    match solhome::sign_keys(io, step, now, ixs, keys).await {
        Ok(_) => {
            let sig = io.sol_line_of(LineId::Desk).line.saved.as_ref().map(|s| s.signature.clone()).unwrap_or_default();
            with_mut(|s| s.link.signed(id, &sig, now));
            Ok(sig)
        }
        Err(e) => {
            undo(format!("not signed: {e}"), Phase::Undone);
            Err(SignError::Deferred(e))
        }
    }
}

/// Split borrows of the two books-side parts of the state.
pub struct DeskStateRef<'a> {
    pub books: &'a mut crate::books::Books,
    pub link: &'a mut super::ledger::Ledger,
}

impl<'a> DeskStateRef<'a> {
    pub fn of(s: &'a mut crate::desk::state::DeskState) -> DeskStateRef<'a> {
        DeskStateRef { books: &mut s.books, link: &mut s.link }
    }
}

fn op_id_of(step: &LineStep) -> Option<u64> {
    match step {
        LineStep::Op(o) if o.owner == LineOwner::Desk && o.what.starts_with(memo::PREFIX) => Some(o.id),
        // review F9: the pool desk's, the coins' and the treasury's linked steps carry their link id
        LineStep::Pool(p) => p.link(),
        LineStep::Coin(c) => c.link(),
        LineStep::Treasury(t) => t.link(),
        // W8c: the Solana NFT flow's advance + purchase carries its link id
        LineStep::SolNft(s) => s.link(),
        _ => None,
    }
}

/// Take the desk line's outcomes that are links of ours and apply them: landed → final, failed on chain or proven never to land →
/// undone. Then every link still Recorded (a crash between the record and the signature) is adopted if the line carries it, else
/// undone; every Signed link the line no longer knows is looked up by its signature. Returns the log.
pub async fn settle<I: DeskIo>(io: &mut I, now: u64) -> Vec<String> {
    let mut log = vec![];
    // 1. the line's verdicts
    let mut l = io.sol_line_of(LineId::Desk);
    let (mine, rest): (Vec<_>, Vec<_>) = l.outcomes.drain(..).partition(|o| op_id_of(&o.step).is_some_and(|id| with(|s| s.link.get(id).is_some())));
    l.outcomes = rest;
    if !mine.is_empty() {
        io.sol_line_save_of(LineId::Desk, &l);
    }
    for o in mine {
        let id = op_id_of(&o.step).expect("filtered");
        with_mut(|s| {
            let DeskStateRef { books, link } = DeskStateRef::of(s);
            match o.ok {
                Some(true) => {
                    link.landed(books, id, &o.signature, now);
                    log.push(format!("link {id}: landed ({})", o.signature));
                }
                Some(false) => {
                    link.undo(books, id, Phase::Failed, &format!("landed and failed on chain ({})", o.signature), now);
                    log.push(format!("link {id}: FAILED on chain ({}): undone", o.signature));
                }
                None => {
                    link.undo(books, id, Phase::Undone, "proven never to land (its nonce burned or its blockhash expired)", now);
                    log.push(format!("link {id}: proven never to land: undone"));
                }
            }
        });
    }
    // 2. crash recovery: what the line says it holds
    // review F9: a link may be out on any line (the desk's, the pool desk's, the crank line's coins/treasury steps)
    let held: Vec<(u64, String)> = LineId::ALL.iter().filter_map(|id| {
        let l = io.sol_line_of(*id);
        l.pending.as_ref().and_then(|p| op_id_of(&p.step)).map(|id| (id, l.line.saved.as_ref().map(|s| s.signature.clone()).unwrap_or_default()))
    }).collect();
    let open: Vec<Link> = with(|s| s.link.links.values().filter(|x| x.phase.open() && x.phase != Phase::Stuck).cloned().collect());
    for k in open {
        let on_line = held.iter().find(|(id, _)| *id == k.id);
        match (k.phase, on_line) {
            (Phase::Recorded, Some((_, sig))) => {
                with_mut(|s| s.link.signed(k.id, sig, now));
                log.push(format!("link {}: recovered as signed on the desk line ({sig})", k.id));
            }
            (Phase::Recorded, None) => {
                with_mut(|s| {
                    let DeskStateRef { books, link } = DeskStateRef::of(s);
                    link.undo(books, k.id, Phase::Undone, "recorded and never signed (a restart between the record and the signature)", now);
                });
                log.push(format!("link {}: recorded, never signed: undone", k.id));
            }
            (Phase::Signed, Some(_)) => {}
            (Phase::Signed, None) => {
                // the line holds nothing of it and gave no verdict: ask the chain by its signature
                match io.sol_transaction(&k.signature).await {
                    // audit B1-11: a transaction answered without its `meta` says nothing of its outcome
                    Ok(Some(tx)) if tx.get("meta").is_none_or(|m| m.is_null()) => log.push(format!("link {}: the chain answered without meta: unread", k.id)),
                    Ok(Some(tx)) => {
                        let failed = !tx["meta"]["err"].is_null();
                        with_mut(|s| {
                            let DeskStateRef { books, link } = DeskStateRef::of(s);
                            if failed {
                                link.undo(books, k.id, Phase::Failed, "finalized and failed on chain (found by its signature)", now);
                            } else {
                                link.landed(books, k.id, &k.signature, now);
                            }
                        });
                        log.push(format!("link {}: found finalized by its signature ({})", k.id, if failed { "failed" } else { "ok" }));
                    }
                    Ok(None) if now.saturating_sub(k.updated_at) > ORPHAN_SECS => {
                        with_mut(|s| s.link.stuck(k.id, "signed, absent from its line and not finalized: the books stay reserved", now));
                        log.push(format!("link {}: STUCK (an operator looks)", k.id));
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }
    with_mut(|s| s.link.prune());
    log
}

/// Audit B1-11: the operator, having looked at the chain, decides a [`Phase::Stuck`] link. The chain is asked first by the link's
/// signature: a finalized answer decides it whatever the operator said; with none, only `landed = false` is taken (the operator
/// asserts the transaction can never land: its nonce burned or its blockhash gone), `landed = true` is refused.
pub async fn resolve_stuck<I: DeskIo>(io: &mut I, id: u64, landed: bool, now: u64) -> Result<String, String> {
    let k = with(|s| s.link.get(id).cloned()).ok_or_else(|| format!("link {id}: unknown"))?;
    if k.phase != Phase::Stuck {
        return Err(format!("link {id} is {:?}, not stuck", k.phase));
    }
    let chain = io.sol_transaction(&k.signature).await?;
    let verdict = match chain {
        Some(tx) if tx.get("meta").is_none_or(|m| m.is_null()) => return Err(format!("link {id}: the chain answered without meta; ask again")),
        Some(tx) => Some(tx["meta"]["err"].is_null()),
        None => None,
    };
    match (verdict, landed) {
        (Some(true), _) => {
            with_mut(|s| {
                let DeskStateRef { books, link } = DeskStateRef::of(s);
                link.landed(books, id, &k.signature, now);
            });
            Ok(format!("link {id}: finalized ok on chain ({}): landed", k.signature))
        }
        (Some(false), _) => {
            with_mut(|s| {
                let DeskStateRef { books, link } = DeskStateRef::of(s);
                link.undo(books, id, Phase::Failed, "finalized and failed on chain (the operator's resolve)", now);
            });
            Ok(format!("link {id}: finalized and failed on chain ({}): undone", k.signature))
        }
        (None, true) => Err(format!("link {id}: the chain does not know {}: it cannot be taken as landed", k.signature)),
        (None, false) => {
            with_mut(|s| {
                let DeskStateRef { books, link } = DeskStateRef::of(s);
                link.undo(books, id, Phase::Undone, "unknown to the chain; the operator says it can never land (desk_resolve_link)", now);
            });
            Ok(format!("link {id}: unknown to the chain, undone on the operator's word"))
        }
    }
}

/// Audit B1-12: pages of [`SCAN_LIMIT`] signatures read per scan, at most (older ones are the reconciliation's; a scan that could not
/// reach its cursor says so).
pub const SCAN_PAGES: usize = 5;

/// The Rook memos on the chain's recent history of `K_main` (every Rook transaction is paid by it) read against the links: how many
/// were seen, and the ones no link explains. Moves the scan's cursor. Audit B1-12: pages back (`before`) until the cursor is met, at
/// most [`SCAN_PAGES`] pages; past that, the gap is an incident (nothing in it is read) and the cursor still moves.
pub async fn scan_memos<I: DeskIo>(io: &mut I) -> (u32, Vec<String>) {
    let cursor = with(|s| s.link.memo_cursor.clone().or_else(|| s.link.links.values().map(|l| l.signature.clone()).find(|x| !x.is_empty())));
    let Some(cursor) = cursor else { return (0, vec![]) };
    let Ok(mut sigs) = io.sol_signatures(&cursor, SCAN_LIMIT).await else { return (0, vec![]) };
    let mut pages = 1;
    while sigs.len() == pages * SCAN_LIMIT && pages < SCAN_PAGES {
        let oldest = sigs.last().map(|s| s.signature.clone()).unwrap_or_default();
        match io.sol_signatures_before(&cursor, &oldest, SCAN_LIMIT).await {
            Ok(more) if !more.is_empty() => sigs.extend(more),
            _ => break,
        }
        pages += 1;
    }
    if sigs.len() == SCAN_PAGES * SCAN_LIMIT {
        let now = sigs.first().map(|s| s.slot).unwrap_or(0);
        with_mut(|st| st.link.incident(crate::chainlink::ledger::Incident { coll: None, link: None, at: now,
            what: format!("memo scan: over {} signatures of K_main since the last scan; the older ones are unread (the reconciliation is the check)", SCAN_PAGES * SCAN_LIMIT) }));
    }
    let mut seen = 0;
    let mut unexplained = vec![];
    for s in sigs.iter().rev() {
        let Some(text) = s.memo.as_deref() else { continue };
        for m in memo::parse_all(text) {
            seen += 1;
            let known = with(|st| st.link.get(m.id).is_some_and(|l| l.coll == m.coll && l.kind.tag() == m.kind && l.intent == m.intent));
            if !known {
                unexplained.push(format!("{} in {}", memo::memo_text(&m), s.signature));
            }
        }
    }
    if let Some(newest) = sigs.first() {
        with_mut(|st| st.link.memo_cursor = Some(newest.signature.clone()));
    }
    (seen, unexplained)
}

/// One reconciliation of every collection against the chain (the caller settled first): compare, strike, stop; the memo scan.
pub async fn reconcile<I: DeskIo>(io: &mut I, now: u64) -> Report {
    let colls: Vec<u16> = with(|s| s.books.collections.keys().copied().collect());
    let mut findings = vec![];
    let mut checked = vec![];
    for c in colls {
        match read_view(io, c).await {
            Ok(v) => {
                checked.push(c);
                findings.extend(with(|s| recon::compare(&s.books, &s.link, &v, now as i64)));
            }
            Err(e) => with_mut(|s| s.link.incident(super::ledger::Incident { coll: Some(c), link: None, what: format!("reconcile could not read: {e}"), at: now })),
        }
    }
    let halted = with_mut(|s| {
        let DeskStateRef { books, link } = DeskStateRef::of(s);
        link.note_findings(books, &checked, &findings, now)
    });
    let (memos_seen, unexplained) = scan_memos(io).await;
    let report = Report { at: now, checked: checked.len() as u32, findings, halted, memos_seen, unexplained };
    with_mut(|s| {
        s.link.last_report = Some(report.clone());
        s.last_reconcile = now;
    });
    report
}

/// Lamports at `M[i]` above the books are booked as surplus (the unsolicited transfer, the gift): refused while anything of the
/// collection is undecided. Returns the amount booked.
pub async fn sweep_unbooked<I: DeskIo>(io: &mut I, coll: u16, now: u64) -> Result<u64, String> {
    if with(|s| s.link.open_of(coll, None).next().is_some()) {
        return Err(format!("collection {coll} has an undecided operation: sweep after it"));
    }
    // audit B1-10: the money line's steps are not links: a way back landed and not yet booked by its flow would be swept, then booked
    // again; the sweep waits for an idle money line with nothing to report, and for the collection's deferred writes
    let money = io.sol_line_of(crate::solhome::LineId::Money);
    if money.busy() || !money.outcomes.is_empty() || with(|s| !s.link.owed_of(coll).is_empty()) {
        return Err(format!("collection {coll}: the money line holds a step or an outcome not yet booked: sweep after the next tick"));
    }
    let v = read_view(io, coll).await?;
    let owed = with(|s| s.books.get(coll).map(|c| c.module.books())).map_err(|e| format!("{e}"))?;
    let extra = (v.free_lamports as u128).saturating_sub(owed);
    if extra < recon::UNBOOKED_MIN as u128 {
        return Err(format!("only {extra} unbooked lamports at M[{coll}] (below {})", recon::UNBOOKED_MIN));
    }
    let extra = u64::try_from(extra).map_err(|_| "overflow".to_string())?;
    with_mut(|s| {
        let DeskStateRef { books, link } = DeskStateRef::of(s);
        let spec = super::ledger::Spec { key: format!("sweep:{coll}:{now}"), kind: super::ledger::OpKind::Sweep, coll: Some(coll), intent: None,
            scope: super::ledger::Scope::Module };
        let (id, fresh) = link.begin(books, spec, now, |b| b.fund_reserve(coll, extra).map(|_| (super::ledger::Effects::default(), extra))).map_err(|e| format!("{e}"))?;
        if fresh {
            link.landed(books, id, "", now);
        }
        Ok::<_, String>(())
    })?;
    Ok(extra)
}
