//! W8 (docs/v9/canister-only-solana.md §8): the frog side's home: the canister keeps the books itself (decision D-l). The surface
//! `frogsol` dispatches to, read from the books' one Inscriptions listing; every former rook_core instruction is a book update, final
//! at once and posted as a landed outcome on the line, except the three that move SOL: the advance (`M → K_main`, signed by the
//! collection's key, booked before the signature and taken back if it never lands), the way back and a late credit (`K_main → M`,
//! booked when they land). The Relay deposit from `K_main` is `frogsol`'s, unchanged.
use super::*;
use crate::books::ledger::{Destination as BDest, MintTo, Stage as BStage};
use crate::books::{self, ListingKind};
use crate::chainlink::ledger::OwedWrite;
use crate::desk::state as desk;
use crate::solkey::KeyPath;

/// The Inscriptions listing a world serves (the Ordinals multi-collection change, docs/v9/ordinals-collections.md §4): the one whose wX
/// mint is `w` (each collection's `ItemIo` / `CustodianIo` names its listing's mint as its `sol_wrapper`); a world that names none of
/// them, while the books list exactly one Inscriptions listing, is that one's (the single-collection world of before).
pub fn coll_for(w: Option<&Pubkey>) -> Result<(u16, books::Collection), String> {
    desk::with(|s| {
        let ins: Vec<(u16, &books::Collection)> = s.books.collections.iter()
            .filter(|(_, c)| matches!(c.kind, ListingKind::Inscriptions { .. })).map(|(id, c)| (*id, c)).collect();
        if let Some((id, c)) = w.and_then(|w| ins.iter().find(|(_, c)| c.wx_mint == *w)) {
            return Ok((*id, (*c).clone()));
        }
        match ins.as_slice() {
            [] => Err("no Inscriptions listing in the books".to_string()),
            [(id, c)] => Ok((*id, (*c).clone())),
            _ => Err(format!("this world names none of the books' {} Inscriptions listings by its wX mint", ins.len())),
        }
    })
}

/// [`coll_for`] of a frog-side world.
pub fn coll<I: FrogIo>(io: &I) -> Result<(u16, books::Collection), String> {
    coll_for(io.sol_wrapper().as_ref())
}

fn root_of(c: &books::Collection) -> Result<[u8; 32], String> {
    match c.kind {
        ListingKind::Inscriptions { membership_root, .. } => Ok(membership_root),
        _ => Err("the frog listing is not an Inscriptions listing".into()),
    }
}

fn custody_of(c: &books::Collection) -> Result<String, String> {
    match &c.kind {
        ListingKind::Inscriptions { custody, .. } => Ok(custody.clone()),
        _ => Err("the frog listing is not an Inscriptions listing".into()),
    }
}

/// The frog listing must live on the Bitcoin network this canister was installed for: its custody address carries the prefix.
pub fn network_ok(hrp: &str, custody: &str) -> Result<(), String> {
    if !custody.starts_with(&format!("{hrp}1p")) {
        return Err(format!("the frog listing's custody address {custody} is not a {hrp}1p… taproot address of this canister's network: nothing is \
            bought, minted or delivered"));
    }
    Ok(())
}

/// The frog module's address on this home: the collection key's.
pub async fn module<I: FrogIo>(io: &mut I) -> Result<(u16, Pubkey), String> {
    let (id, _) = coll(io)?;
    Ok((id, io.sol_key_of(&KeyPath::Coll(id as u32)).await?))
}

pub async fn module_key<I: FrogIo>(io: &mut I) -> Result<[u8; 20], String> {
    Ok(super::module_key(&module(io).await?.1))
}

/// The record key of the frog collection (custody caps): `sol:<the books' wX mint>`.
pub fn collection_key(w: Option<&Pubkey>) -> Option<String> {
    coll_for(w).ok().map(|(_, c)| crate::custodycap::sol_key(&c.wx_mint))
}

pub async fn custodian_ok<I: FrogIo>(io: &mut I) -> Result<[u8; 20], String> {
    coll(io)?;
    let me = io.sol_me().await?;
    Ok(me[..20].try_into().expect("20"))
}

pub async fn root<I: FrogIo>(io: &mut I) -> Result<[u8; 32], String> {
    root_of(&coll(io)?.1)
}

pub async fn identity<I: FrogIo>(io: &mut I, key: [u8; 20]) -> Result<Option<String>, String> {
    let m = module_key(io).await?;
    if m != key {
        return Ok(Some("it is not the frog collection's module (the collection key's address): on the books home a collection has one module".into()));
    }
    Ok(None)
}

fn kd(c: &books::Collection) -> bool {
    desk::with(|s| c.kd(s.books.keeper_paused))
}

pub async fn read_sink<I: FrogIo>(io: &mut I, kind: SinkKind, key: [u8; 20], limit: Option<u128>) -> Result<SinkView, String> {
    if kind != SinkKind::Floor {
        return Err(format!("a {kind:?} sink does not exist on the books home (DBC owns the curve; the coins module backs its raise)"));
    }
    if module_key(io).await? != key {
        return Err("not the frog collection's module".into());
    }
    let (_, c) = coll(io)?;
    let limit = limit.unwrap_or(MAX_PURCHASE_LAMPORTS).min(RELAY_SOL_BTC_MAX_LAMPORTS);
    let (now, slot) = io.sol_clock().await?;
    Ok(sh::books_sink_view(&c, kd(&c), now as i64, slot, limit))
}

pub async fn transit_open<I: FrogIo>(io: &mut I) -> Result<bool, String> {
    Ok(coll(io)?.1.module.transit_units != 0)
}

/// Whether the ledger holds this frog, minted from this very outpoint.
pub async fn minted<I: FrogIo>(io: &mut I, buy: &Bought) -> Result<bool, String> {
    let (_, c) = coll(io)?;
    let leaf = crate::frogbuy::leaf_of(&buy.inscription);
    let Some(id) = c.ledger.held_markers.get(&leaf) else { return Ok(false) };
    Ok(c.ledger.items.get(id).is_some_and(|x| x.key == leaf && x.source_tx == buy.outpoint.txid && x.vout == buy.outpoint.vout))
}

fn books_err(e: books::BooksError) -> String {
    format!("the books refused: {e}")
}

/// A frog step on this home. A step that moves SOL is signed on the money line and resolves through the line's outcome ([`landed`]);
/// every other step is a book update, final at once, posted as a landed outcome the next `resolve` reports to the flow.
pub async fn send<I: FrogIo>(io: &mut I, st: &mut FrogState, step: FStep, call: Call) -> Result<String, String> {
    if io.paused() {
        return Err("paused: nothing new is signed".into());
    }
    let (id, module) = module(io).await?;
    if desk::collection_busy(id) && !matches!(call, Call::Deposit(_)) {
        return Err(format!("collection {id} has an unfinished chain link: the step waits for it (review F1)"));
    }
    let now = io.now() as i64;
    let book = |f: &dyn Fn(&mut books::Books) -> Result<String, books::BooksError>| -> Result<String, String> {
        desk::with_mut(|s| f(&mut s.books)).map_err(books_err)
    };
    let what = LineStep::Frog(step.clone()).label();
    let text = match call {
        Call::Deposit(_) => return Err(format!("{step:?}: not a book step of this home")),
        Call::Advance(a) => {
            let lamports = u64_of(a, "the advance")?;
            if st.sol_step.is_some() {
                return Err("the frog side's step is still out on the Solana line (one at a time)".into());
            }
            let me = sh::ready(io, LineOwner::Frog, io.now(), &what).await?;
            desk::with_mut(|s| s.books.advance(id, lamports, now)).map_err(books_err)?;
            match sh::sign_keys(io, LineStep::Frog(step), io.now(), vec![soltx::transfer(module, me, lamports)], vec![KeyPath::Coll(id as u32)]).await {
                Ok((text, p)) => {
                    st.sol_step = Some(p);
                    io.save(st);
                    return Ok(text);
                }
                Err(e) => {
                    desk::with_mut(|s| s.books.return_advance(id, lamports)).ok();
                    return Err(e);
                }
            }
        }
        Call::Return(l) | Call::Reserve(l) => {
            let lamports = u64_of(l, "the return")?;
            if st.sol_step.is_some() {
                return Err("the frog side's step is still out on the Solana line (one at a time)".into());
            }
            let me = sh::ready(io, LineOwner::Frog, io.now(), &what).await?;
            let (text, p) = sh::sign(io, LineStep::Frog(step), io.now(), vec![soltx::transfer(me, module, lamports)]).await?;
            st.sol_step = Some(p);
            io.save(st);
            return Ok(text);
        }
        Call::Settle(cost) => {
            let cost = u64_of(cost, "the cost")?;
            book(&|b| b.settle_advance(id, cost, now).map(|_| format!("settled at {cost} lamports in the books")))?
        }
        Call::Arrived(cost) => {
            let cost = u64_of(cost, "the cost")?;
            book(&|b| b.item_arrived(id, cost, now).map(|_| format!("item_arrived at {cost} lamports in the books")))?
        }
        Call::Mint { inscription, proof, outpoint } => {
            let args = books::ledger::MintItemArgs { key: crate::frogbuy::leaf_of(&inscription), source: source_of(outpoint.txid, outpoint.vout),
                tx: outpoint.txid, vout: outpoint.vout, deposit: false, inscription_id: inscription.clone(), proof: proof.clone() };
            book(&|b| b.mint_item(id, args.clone(), MintTo::Module, now).map(|i| format!("item {i} ({inscription}) minted to the module in the books")))?
        }
        Call::Post { price, confidence, depth } => {
            book(&|b| b.post_floor(id, price, confidence, depth, now).map(|(a, t)| format!("floor posted in the books: accepted {a}, target {t}")))?
        }
    };
    let mut l = io.sol_line();
    // the outcome names its collection (`books:<id>`): each Inscriptions collection's resolve takes its own (`frogsol::owns`). Audit
    // C2-03: on the book outcomes' own count, so many live collections' book outcomes never trim a landed advance's
    l.book_outcome(sh::LineOutcome { step: LineStep::Frog(step), intent: 0, ok: Some(true), signature: format!("books:{id}"), at: io.now() });
    io.sol_line_save(&l);
    Ok(format!("{text} (final; the flow reads it next tick)"))
}

/// The line's outcome of a step that moved SOL: the books follow. Before the flow's own landing logic. Audit B1-06: through
/// `desk::book_outcome`, never under a desk link's snapshot.
pub fn landed(w: Option<&Pubkey>, st: &FrogState, step: &FStep, ok: bool) {
    let Ok((id, _)) = coll_for(w) else { return };
    match step {
        FStep::Advance { frog } if !ok => {
            if let Some(l) = st.conv(*frog).and_then(|c| u64::try_from(c.advanced).ok()) {
                let _ = desk::book_outcome(id, OwedWrite::ReturnAdvance { lamports: l });
            }
        }
        FStep::GiveBack { frog } if ok => {
            if let Some(crate::relayconv::CPhase::Returning { wei }) = st.conv(*frog).map(|c| c.phase.clone()) {
                if let Ok(l) = u64::try_from(wei) {
                    let _ = desk::book_outcome(id, OwedWrite::ReturnAdvance { lamports: l });
                }
            }
        }
        FStep::LateCredit { frog } if ok => {
            if let Some(l) = st.conv(*frog).and_then(|c| c.late.as_ref()).and_then(|l| l.refund_wei).and_then(|w| u64::try_from(w).ok()) {
                let _ = desk::book_outcome(id, OwedWrite::FundReserve { lamports: l });
            }
        }
        _ => {}
    }
}

/// The keeper's floor post on this home: the canister is the poster; a reading is posted inside the module's bounds.
pub async fn post_act<I: FrogIo>(io: &mut I, st: &mut FrogState) -> Result<Option<String>, String> {
    let Some(r) = st.floor_reading.clone() else { return Ok(None) };
    let now = io.now();
    if now.saturating_sub(r.at) > READING_TTL_SECS {
        st.floor_reading = None;
        return Ok(Some(format!("frog floor: the reading of {} sats is {} s old: dropped, not posted", r.sats, now.saturating_sub(r.at))));
    }
    let (_, c) = coll(io)?;
    if kd(&c) {
        st.floor_reading = None;
        return Ok(Some("frog floor: the poster is paused: the reading is dropped".into()));
    }
    let rate = io.sol_btc_rate().await?;
    let bounds = c.module.floor_bounds(now as i64).unwrap_or((0, u64::MAX));
    let price = post_price(r.sats, rate, bounds);
    if price == 0 {
        st.floor_reading = None;
        return Err(format!("frog floor: {} sats is 0 lamports at {rate} sats per SOL: nothing is posted", r.sats));
    }
    let raw = crate::rates::sats_to_lamports(r.sats as u128, rate);
    // audit C2-12 (default-live): a listing's FIRST floor has no accepted floor to be clamped to; it is refused unless it is within the
    // band of the floor its launch pin was sized from, so 15 first floors at once are never the runner's band alone
    if c.module.accepted == 0 {
        if let Err(e) = launch::first_floor_ok(&root_of(&c)?, raw) {
            st.floor_reading = None;
            return Err(format!("frog floor: REFUSED: {e}"));
        }
    }
    let s = send(io, st, FStep::Post, Call::Post { price, confidence: r.confidence_bps, depth: r.depth_items }).await?;
    Ok(Some(format!("frog floor: post_floor({price} lamports per unit, {}, {}) from {} sats at {rate} sats per SOL{}: {s}", r.confidence_bps,
        r.depth_items, r.sats, if raw != price as u128 { format!(" (clamped from {raw} into the module's bounds [{}, {}])", bounds.0, bounds.1) }
        else { String::new() })))
}

// ---- the custodian's side: frog redemptions from the books' tickets ------------------------------------------------------------------

pub async fn ticket_count<I: SolIo>(io: &mut I, w: &Pubkey, hrp: &str) -> Result<(u64, u64), String> {
    let (_, c) = coll_for(Some(w))?;
    network_ok(hrp, &custody_of(&c)?)?;
    let (_, slot) = io.sol_clock().await?;
    Ok((c.ledger.ticket_count, slot))
}

/// One frog ticket (what the custodian reads), from the books; a ticket not created yet reads `Stage::None` (the EVM's empty struct).
pub fn ticket(w: &Pubkey, id: u64) -> Result<books::ledger::Ticket, String> {
    let (_, c) = coll_for(Some(w))?;
    Ok(c.ledger.tickets.get(&id).cloned().unwrap_or(books::ledger::Ticket { id, stage: BStage::None, owner: [0; 32], round: 0, pinned_at: 0,
        deliver_by: 0, item_id: 0, delivery_tx: [0; 32], sent_tx: [0; 32], undeliverable: false, destination: BDest::Burn }))
}

pub async fn ticket_read<I: SolIo>(io: &mut I, w: &Pubkey, id: u64, now: u64, hrp: &str) -> Result<crate::delivery::TicketRead, String> {
    let (_, c) = coll_for(Some(w))?;
    network_ok(hrp, &custody_of(&c)?)?;
    let t = ticket(w, id)?;
    let (time, _) = io.sol_clock().await?;
    let chain_time = time.max(now);
    let step = crate::delivery::PARENT_BLOCK_SECONDS;
    let deliver_by = t.deliver_by.max(0) as u64;
    Ok(crate::delivery::TicketRead {
        stage: match t.stage {
            BStage::None => crate::delivery::Stage::None,
            BStage::Pinned => crate::delivery::Stage::Pinned,
            BStage::Resolved => crate::delivery::Stage::Resolved,
            BStage::Delivered => crate::delivery::Stage::Delivered,
            BStage::Refunded => crate::delivery::Stage::Refunded,
        },
        acquisition_id: t.item_id as u128,
        destination: destination_text(&t.destination, hrp),
        deliver_by,
        deliver_by_block: deliver_by / step,
        chain_time,
        parent_block: chain_time.div_ceil(step),
        read_at: now,
    })
}

pub fn inscription_of(w: &Pubkey, item_id: u128) -> Result<String, String> {
    let id = u32::try_from(item_id).map_err(|_| format!("item {item_id} is over a u32"))?;
    let (_, c) = coll_for(Some(w))?;
    let x = c.ledger.items.get(&id).ok_or_else(|| format!("item {id} does not exist"))?;
    if x.inscription_id.is_empty() || crate::frogbuy::leaf_of(&x.inscription_id) != x.key {
        return Err(format!("item {id} carries no inscription id whose leaf is its key"));
    }
    Ok(x.inscription_id.clone())
}

/// A custodian's step: a book update, final at once (the custodian reads the ticket back, never a landing).
pub fn custody_send(w: &Pubkey, step: sh::CustodyStep, now: u64) -> Result<(String, sh::LinePending), String> {
    let (id, _) = coll_for(Some(w))?;
    let text = desk::with_mut(|s| match &step {
        sh::CustodyStep::Record { ticket, txid } => s.books.record_delivery(id, *ticket, *txid).map(|_| format!("ticket {ticket} recorded delivered in the books")),
        sh::CustodyStep::Mark { ticket } => s.books.mark_undeliverable(id, *ticket).map(|_| format!("ticket {ticket} marked undeliverable in the books")),
        sh::CustodyStep::Sent { ticket, txid } => s.books.note_sent(id, *ticket, *txid).map(|_| format!("ticket {ticket} noted sent in the books")),
    }).map_err(books_err)?;
    Ok((text, sh::LinePending { step: LineStep::Custody(step), intent: 0, at: now }))
}
