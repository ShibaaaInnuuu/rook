//! The sequencer's pieces for protocol sales and relist tickets (docs/v6/canister-floor-followups.md §3–§5), over `FrogIo`:
//! read each module's sale side at the tick's pin and decide a sale; walk the wrapper's tickets for a vault's relists; on
//! Bitcoin sign a listing, watch it for a fill, move a delisted item; and, as the sixth group of `frogflow::evm_act`, send
//! `redeemToRelist`, `recordDelivery`, `itemSold{value}` / `recordSale{value}`, `wrapDeposit` and `reclaimListing`, one
//! transaction at a time. No network of its own; the decisions are `sale.rs`'s. Tested against a fake world in
//! `tests/sales.rs`.

use crate::custody::{self, Frog, NotSendable, Outpoint};
use crate::custodycap;
use crate::delivery::Stage;
use crate::floorsink::{self, FloorExtra, ONE};
use crate::floortrigger::FloorState;
use crate::frogbuy::CONFIRMATIONS;
use crate::frogflow::{self, FStep, FrogIo, FrogState};
use crate::itemsink::{SinkKind, SinkView};
use crate::outflow::Flow;
use crate::psbt::{self, Psbt};
use crate::sale::{self, SPhase, SaleRec, SaleState};
use crate::btcnet::BtcSeen;

fn h(a: &[u8; 20]) -> String {
    format!("0x{}", hex::encode(a))
}

// ---- reads at the pin -------------------------------------------------------------------------------------------------------------

/// A module's ten reads at the current pin, as the trigger and the sale side see them.
async fn read_module<I: FrogIo>(io: &mut I, m: [u8; 20], limit: Option<u128>) -> Result<(SinkView, FloorExtra), String> {
    let mut r = Vec::with_capacity(10);
    for c in floorsink::reads() {
        r.push(frogflow::word(io, m, &c).await?);
    }
    let clock = io.clock().await?;
    floorsink::view_full(&r, clock, limit)
}

async fn balance_of<I: FrogIo>(io: &mut I, token: [u8; 20], who: [u8; 20]) -> Result<u128, String> {
    crate::abi::uint_at(&frogflow::word(io, token, &crate::curvebuy::call_addr(crate::curvebuy::BALANCE_OF, &who)).await?, 0, 0, 128)
}

async fn ticket_count<I: FrogIo>(io: &mut I, w: [u8; 20]) -> Result<u64, String> {
    Ok(crate::abi::uint_at(&frogflow::word(io, w, &crate::redeem::TICKET_COUNT).await?, 0, 0, 64)? as u64)
}

async fn ticket<I: FrogIo>(io: &mut I, w: [u8; 20], id: u64) -> Result<(crate::redeem::TicketFields, [u8; 20]), String> {
    let ret = frogflow::word(io, w, &crate::redeem::call_uint(crate::redeem::TICKET, id as u128)).await?;
    Ok((crate::redeem::decode_ticket(&ret)?, sale::ticket_owner(&ret)?))
}

/// The inscription a ticket's acquisition id names: a deposit's by its leaf (one this canister minted), a purchase's through
/// the registry's vault (as the custodian's `inscription_of`).
async fn inscription_of<I: FrogIo>(io: &mut I, st: &FrogState, acquisition_id: u128) -> Result<String, String> {
    let w = io.wrapper();
    if acquisition_id & crate::deposits::DEPOSIT_BIT != 0 {
        let call = crate::deposits::call_word(crate::deposits::LEAF_OF_FROG, crate::deposits::frog_id_word(acquisition_id));
        let leaf: [u8; 32] = crate::abi::word_at(&frogflow::word(io, w, &call).await?, 0, 0)?.try_into().map_err(|_| "a leaf is one word")?;
        return st.inscription_for_leaf(&leaf)
            .ok_or_else(|| format!("deposit frog {} is none this canister minted", acquisition_id & !crate::deposits::DEPOSIT_BIT));
    }
    let registry = frogflow::addr(io, w, crate::redeem::REGISTRY).await?;
    let listing = crate::abi::uint_at(&frogflow::word(io, w, &crate::redeem::LISTING_ID).await?, 0, 0, 128)?;
    let vault = crate::abi::addr_at(&frogflow::word(io, registry, &crate::redeem::call_uint(crate::redeem::VAULT_OF, listing)).await?, 0, 0)?;
    let (amount, r) = crate::redeem::decode_acquisition_ref(&frogflow::word(io, vault, &crate::redeem::call_uint(crate::redeem::ACQUISITION, acquisition_id)).await?)?;
    if amount != 1 {
        return Err(format!("acquisition {acquisition_id} is {amount} units, not one inscription"));
    }
    custody::check_inscription_id(&r)?;
    Ok(r)
}

/// The sale side at this tick's pin: each Bitcoin module with a policy (§3.1), the wrapper's tickets (§4), our own tickets
/// waiting to be drawn.
pub async fn read<I: FrogIo>(io: &mut I, st: &FrogState, fl: &FloorState, ss: &mut SaleState, me: [u8; 20]) -> Vec<String> {
    let mut lines = Vec::new();
    if ss.policies.is_empty() && ss.sales.is_empty() {
        return lines;
    }
    let w = io.wrapper();
    let modules: Vec<[u8; 20]> = st.sinks.iter().filter(|s| s.kind == SinkKind::Floor && !s.done() && !fl.is_evm(&s.sink)).map(|s| s.sink).collect();
    for m in modules {
        let Some(p) = ss.policy(&m).cloned() else { continue };
        let r = async {
            let (v, x) = read_module(io, m, fl.limit_for(&m)).await?;
            let seed = balance_of(io, w, me).await?;
            Ok::<_, String>((v, x, seed))
        }.await;
        match r {
            Ok((v, x, seed)) => {
                ss.set_extra(m, x.clone());
                match sale::sell_now(&p, &v, &x, seed, ss.open_for(&m)) {
                    Ok(()) => {
                        let id = ss.plan(m, w);
                        lines.push(format!("sale {id}: planned for Floor {}: the reserve holds {} wei under the {} floor and the module holds \
                            inventory; redeemToRelist is next", h(&m), x.reserve, p.reserve_floor_wei));
                    }
                    Err(e) if !e.contains("not under") => lines.push(format!("Floor {}: no sale: {e}", h(&m))),
                    Err(_) => {}
                }
            }
            Err(e) => lines.push(format!("Floor {}: sale side not read: {e}", h(&m))),
        }
    }
    match read_tickets(io, st, ss, me).await {
        Ok(ls) => lines.extend(ls),
        Err(e) => lines.push(format!("sales: tickets not read: {e}")),
    }
    // our own tickets, waiting to be drawn
    for i in 0..ss.sales.len() {
        let s = ss.sales[i].clone();
        let (SPhase::Pinned, Some(t), Some(m)) = (&s.phase, s.ticket, s.module) else { continue };
        let r = async {
            let (f, _) = ticket(io, w, t).await?;
            match f.stage {
                Stage::Pinned => Ok(None),
                Stage::Resolved => {
                    let inscription = inscription_of(io, st, f.acquisition_id).await?;
                    let price = ss.extra(&m).map(|x| x.price).ok_or("the module's price is not read")?;
                    let markup = ss.policy(&m).map_or(sale::DEFAULT_MARKUP_BPS, |p| p.markup_bps);
                    Ok(Some((inscription, sale::ask_wei(price, markup))))
                }
                s => Err(format!("ticket {t} reads {s:?} before it was drawn: the contract cannot produce that")),
            }
        }.await;
        match r {
            Ok(Some((inscription, ask_wei))) => {
                ss.sales[i].phase = SPhase::Listing { inscription: inscription.clone(), ask_wei };
                lines.push(format!("sale {}: ticket {t} drew {inscription}; to be listed at {ask_wei} wei", s.id));
            }
            Ok(None) => {}
            Err(e) => lines.push(format!("sale {}: waits: {e}", s.id)),
        }
    }
    lines
}

/// §4: the wrapper's tickets from the cursor, bounded per tick, stopping at the first pinned one (tickets resolve in order).
/// A resolved ticket to custody owned by someone else is a vault's relist (adopted once its vault checks out); one owned by
/// us belongs to the sale that sent `redeemToRelist` (adopted here if `mined` missed it).
async fn read_tickets<I: FrogIo>(io: &mut I, st: &FrogState, ss: &mut SaleState, me: [u8; 20]) -> Result<Vec<String>, String> {
    let mut lines = Vec::new();
    let w = io.wrapper();
    let count = ticket_count(io, w).await?;
    let end = count.min(ss.ticket_cursor + sale::MAX_TICKETS_READ_PER_TICK);
    let cust = io.custody_script().await?;
    let hrp = io.btc_hrp()?;
    while ss.ticket_cursor < end {
        let c = ss.ticket_cursor;
        let (t, owner) = ticket(io, w, c).await?;
        if t.stage == Stage::Pinned {
            break;
        }
        let to_custody = crate::policy::script_of(&t.destination, hrp).ok().as_deref() == Some(cust.as_slice());
        if t.stage == Stage::Resolved && to_custody && !ss.has_ticket(c) {
            if owner == me {
                if let Some(s) = ss.sales.iter_mut().find(|s| matches!(s.phase, SPhase::Redeeming { count_before } if count_before <= c) && s.ticket.is_none()) {
                    s.ticket = Some(c);
                    s.phase = SPhase::Pinned;
                    lines.push(format!("sale {}: ticket {c} is ours", s.id));
                }
            } else {
                match adopt_relist(io, st, ss, c, owner, &t).await {
                    Ok(id) => lines.push(format!("sale {id}: ticket {c} is a relist by vault {}", h(&owner))),
                    Err(e) => lines.push(format!("ticket {c} to custody by {}: not listed: {e}", h(&owner))),
                }
            }
        }
        ss.ticket_cursor = c + 1;
    }
    Ok(lines)
}

async fn adopt_relist<I: FrogIo>(io: &mut I, st: &FrogState, ss: &mut SaleState, ticket: u64, vault: [u8; 20], t: &crate::redeem::TicketFields)
    -> Result<u64, String> {
    let w = io.wrapper();
    if frogflow::addr(io, vault, sale::VAULT_WRAPPER).await? != w {
        return Err("its owner's wrapper is not ours: not a vault of this collection".into());
    }
    let l = sale::decode_listing(&frogflow::word(io, vault, &sale::listing_call(ticket)).await?)?;
    if l.stage != sale::LISTING_LISTED {
        return Err(format!("the vault's listing for it is in stage {}, not Listed", l.stage));
    }
    let inscription = inscription_of(io, st, t.acquisition_id).await?;
    ss.adopt_relist(w, ticket, vault, inscription, l.ask_wei).ok_or("adopted already".into())
}

// ---- Bitcoin ------------------------------------------------------------------------------------------------------------------------

/// Every unfinished sale's Bitcoin step: the listing to sign, the outpoint to watch, the move to confirm.
pub async fn btc_pass<I: FrogIo>(io: &mut I, ss: &mut SaleState, reserved: &[Outpoint]) -> Vec<String> {
    let mut lines = Vec::new();
    for i in 0..ss.sales.len() {
        if ss.sales[i].finished() {
            continue;
        }
        let mut s = ss.sales[i].clone();
        let others: Vec<Outpoint> = reserved.iter().copied().chain(ss.sales.iter().enumerate().filter(|(j, _)| *j != i).filter_map(|(_, x)| x.reserved())).collect();
        let line = match step_btc(io, &mut s, &others).await {
            Ok(Some(l)) => Some(format!("sale {}: {l}", s.id)),
            Ok(None) => None,
            Err(e) => Some(format!("sale {}: waits: {e}", s.id)),
        };
        ss.sales[i] = s;
        io.save_sales(ss);
        lines.extend(line);
    }
    lines
}

async fn step_btc<I: FrogIo>(io: &mut I, s: &mut SaleRec, reserved: &[Outpoint]) -> Result<Option<String>, String> {
    match s.phase.clone() {
        SPhase::Listing { inscription, ask_wei } => list(io, s, inscription, ask_wei).await.map(Some),
        SPhase::Listed { .. } | SPhase::Recorded { .. } => watch(io, s).await,
        SPhase::Delisting { inscription, outpoint } => move_out(io, s, inscription, outpoint, reserved).await.map(Some),
        SPhase::Moving { inscription, outpoint, txid, raw, .. } => match io.tx_status(txid).await {
            BtcSeen::Confirmed { .. } => match frogflow::depth(io, txid).await? {
                Some(d) if d >= CONFIRMATIONS => {
                    s.phase = SPhase::Redeposit { inscription, outpoint };
                    Ok(Some(format!("moved to {} and {d} deep: wrapDeposit is next", outpoint.text())))
                }
                Some(d) => Ok(Some(format!("the move is {d} of {CONFIRMATIONS} deep"))),
                None => Ok(Some("the move left the blocks: waiting".into())),
            },
            BtcSeen::Mempool => Ok(None),
            BtcSeen::NotYet => {
                let _ = io.btc_send(&raw).await;
                Ok(Some("the same saved move sent again".into()))
            }
            BtcSeen::Unsure(e) => Err(e),
        },
        _ => Ok(None),
    }
}

/// Custody holds exactly `inscription`: ord's location and contents, the explorers' output. A `Stuck` is written into the
/// record; a wait is an error.
async fn held<I: FrogIo>(io: &mut I, s: &mut SaleRec, inscription: &str, at: Option<&Outpoint>) -> Result<Option<Frog>, String> {
    let cust = io.custody_script().await?;
    let loc = io.locate(inscription).await?.ok_or_else(|| format!("ord does not know {inscription} yet"))?;
    if let Some(o) = at {
        if loc.output != *o {
            return Err(format!("ord places {inscription} at {}, not {} yet", loc.output.text(), o.text()));
        }
    }
    let c = io.contents(&loc.output).await?;
    let (out, spent) = io.output(&loc.output).await?;
    match custody::frog_at(inscription, &loc, c.as_ref(), &out, spent, &cust) {
        Ok(f) => Ok(Some(f)),
        Err(NotSendable::Wait(e)) => Err(e),
        Err(NotSendable::Stuck(why)) => {
            s.phase = SPhase::Stuck { why: why.clone() };
            Ok(None)
        }
    }
}

/// §3.3: the listing, signed by the custody key under `0x83`, checked again from its own bytes, saved; one item out (H3).
async fn list<I: FrogIo>(io: &mut I, s: &mut SaleRec, inscription: String, ask_wei: u128) -> Result<String, String> {
    let ticket = s.ticket.ok_or("a listing with no ticket")?;
    let Some(frog) = held(io, s, &inscription, None).await? else { return Ok(format!("STUCK: {:?}", s.phase)) };
    let cust = io.custody_script().await?;
    let rate = io.rate().await?;
    let ask_sats = sale::ask_sats(ask_wei, rate)?;
    if ask_sats < frog.value {
        s.phase = SPhase::Stuck { why: format!("an ask of {ask_sats} sats is under the item's own {} sats", frog.value) };
        return Ok(format!("STUCK: {:?}", s.phase));
    }
    let collection = custodycap::btc_key(&io.wrapper());
    let key = format!("listing:{ticket}");
    io.outflow_check(&key, &collection, Flow::Item, 1)?;
    let p = sale::listing_psbt(&frog, &cust, ask_sats)?;
    let digest = crate::sighash::taproot_key_path_typed(&p, 0, sale::SIGHASH_LISTING).map_err(|e| format!("digest: {e:?}"))?;
    let sig = io.sign_custody(digest).await?;
    let bytes = sale::listing_bytes(&p, &sig)?;
    // the bytes, read back and judged independently of the builder: a refusal is loud and final, nothing saved
    let back = Psbt::parse(&bytes).map_err(|e| format!("the listing does not parse: {e:?}"))?;
    if let Err(e) = sale::check_listing(&back, &frog, &cust, ask_sats) {
        s.phase = SPhase::Stuck { why: format!("the listing check refused the signed listing: {e}; nothing was saved") };
        return Ok(format!("STUCK: {:?}", s.phase));
    }
    let hash = sale::listing_hash(&bytes);
    s.phase = SPhase::Listed { inscription: inscription.clone(), outpoint: frog.output, ask_sats, ask_wei, rate, listing: bytes, hash };
    io.outflow_record(&key, &collection, Flow::Item, 1);
    Ok(format!("{inscription} listed at {ask_sats} sats ({ask_wei} wei), signed and saved (hash {}); recordDelivery is next; \
        sale_listings() has the PSBT to post", hex::encode(hash)))
}

/// §3.4: the listed outpoint, spent six deep by a fill paying custody the ask at the item's index: Sold. The item left
/// custody: its room under the caps is free (H2).
async fn watch<I: FrogIo>(io: &mut I, s: &mut SaleRec) -> Result<Option<String>, String> {
    let (inscription, outpoint, ask_sats, _) = s.listing().ok_or("not listed")?;
    let (inscription, outpoint) = (inscription.to_string(), *outpoint);
    let Some((txid, confirmed)) = io.outspend(&outpoint).await? else { return Ok(None) };
    if !confirmed {
        return Ok(Some("a spend of the listed outpoint is in a mempool".into()));
    }
    let d = match frogflow::depth(io, txid).await? {
        Some(d) => d,
        None => return Ok(Some("the spend of the listed outpoint left the blocks: watching".into())),
    };
    if d < CONFIRMATIONS {
        return Ok(Some(format!("the spend of the listed outpoint is {d} of {CONFIRMATIONS} deep")));
    }
    let cust = io.custody_script().await?;
    let tx = io.tx_view(txid).await?;
    let (ask_wei, hash) = match &s.phase {
        SPhase::Listed { ask_wei, hash, .. } | SPhase::Recorded { ask_wei, hash, .. } => (*ask_wei, *hash),
        _ => unreachable!("listed"),
    };
    match sale::fill_pays(&tx, &outpoint, &cust, ask_sats) {
        Ok(sold_sats) => {
            let rate = io.rate().await?;
            let proceeds_wei = sale::proceeds_wei(sold_sats, rate)?;
            s.phase = SPhase::Sold { inscription: inscription.clone(), fill_txid: txid, sold_sats, rate, proceeds_wei, ask_wei, hash };
            let _ = io.with_caps(|_, b| custodycap::release(b, &inscription));
            Ok(Some(format!("SOLD: {} paid custody {sold_sats} sats; {proceeds_wei} wei net at {rate} sats/ETH is the report, once the ETH is on 4663",
                hex::encode(txid))))
        }
        Err(e) => {
            s.phase = SPhase::Stuck { why: format!("the listed outpoint was spent by {} which is not a fill: {e}", hex::encode(txid)) };
            Ok(Some(format!("STUCK: {:?}", s.phase)))
        }
    }
}

/// §3.3, the delist: the item to a fresh custody outpoint, every output custody's, judged as a conflict-out (H4).
async fn move_out<I: FrogIo>(io: &mut I, s: &mut SaleRec, inscription: String, outpoint: Outpoint, reserved: &[Outpoint]) -> Result<String, String> {
    let Some(frog) = held(io, s, &inscription, Some(&outpoint)).await? else { return Ok(format!("STUCK: {:?}", s.phase)) };
    let cust = io.custody_script().await?;
    let rate = io.fee_rate().await?;
    let utxos = io.custody_utxos().await?;
    let mut fee = None;
    for u in custody::fee_candidates(&utxos, &frog.output, reserved, rate) {
        let c = io.contents(&u.outpoint).await?;
        if custody::fee_eligible(&u, c.as_ref(), &frog.output, reserved) {
            fee = Some(u);
            break;
        }
    }
    let fee = fee.ok_or("no clean custody UTXO can pay the move's fee (the operator tops custody up)")?;
    let p = sale::move_psbt(&frog, &fee, &cust, rate)?;
    let mut sigs = Vec::new();
    for i in 0..p.tx.inputs.len() {
        let d = crate::sighash::taproot_key_path(&p, i).map_err(|e| format!("digest: {e:?}"))?;
        sigs.push((i, io.sign_custody(d).await?));
    }
    let raw = psbt::finalize(&p, &vec![None; p.tx.inputs.len()], &sigs)?;
    let pre = vec![
        crate::outbound::Prevout { txid: frog.output.internal_txid(), vout: frog.output.vout, value: frog.value, script: cust.clone() },
        crate::outbound::Prevout { txid: fee.outpoint.internal_txid(), vout: fee.outpoint.vout, value: fee.value, script: cust.clone() },
    ];
    if let Err(e) = crate::outbound::verify_btc(&raw, &pre, &crate::outbound::Purpose::ConflictOut { spend: pre[0].clone() }, &io.outbound_policy(&cust)) {
        s.phase = SPhase::Stuck { why: format!("the outbound check refused the signed move: {e:?}; nothing was saved or sent") };
        return Ok(format!("STUCK: {:?}", s.phase));
    }
    let txid = p.txid();
    s.phase = SPhase::Moving { inscription, outpoint: Outpoint { txid, vout: 0 }, txid, raw: raw.clone(), fee_input: fee.outpoint };
    let _ = io.btc_send(&raw).await;
    Ok(format!("the move is signed, saved and sent in {} (the listing is cancelled once it confirms)", hex::encode(txid)))
}

// ---- the EVM group -------------------------------------------------------------------------------------------------------------------

/// The sixth group of `frogflow::evm_act`. One that cannot go now is reported in `errs` and does not hold up the others.
pub async fn evm_act<I: FrogIo>(io: &mut I, st: &mut FrogState, ss: &mut SaleState, errs: &mut Vec<String>) -> Option<String> {
    if ss.sales.iter().all(|s| s.finished()) {
        return None;
    }
    let me = match io.our_evm().await {
        Ok(m) => m,
        Err(e) => {
            errs.push(format!("sales: {e}"));
            return None;
        }
    };
    for i in 0..ss.sales.len() {
        if ss.sales[i].finished() {
            continue;
        }
        let s = ss.sales[i].clone();
        match one(io, st, ss, &s, me).await {
            Ok(Some(l)) => return Some(format!("sale {}: {l}", s.id)),
            Ok(None) => {}
            Err(e) => errs.push(format!("sale {}: {e}", s.id)),
        }
    }
    None
}

async fn one<I: FrogIo>(io: &mut I, st: &mut FrogState, ss: &mut SaleState, s: &SaleRec, me: [u8; 20]) -> Result<Option<String>, String> {
    let w = s.wrapper;
    match &s.phase {
        SPhase::Planned => {
            if balance_of(io, w, me).await? < ONE {
                return Err("no seed unit to redeem: the canister holds less than one wX".into());
            }
            let count_before = ticket_count(io, w).await?;
            let l = frogflow::send(io, st, FStep::Relist { sale: s.id, count_before }, w, sale::REDEEM_TO_RELIST.to_vec()).await?;
            if let Some(x) = ss.sale_mut(s.id) {
                x.phase = SPhase::Redeeming { count_before };
            }
            Ok(Some(l))
        }
        SPhase::Listed { .. } | SPhase::Recorded { .. } | SPhase::Sold { .. } => {
            let (hash, inscription) = match &s.phase {
                SPhase::Listed { hash, inscription, .. } | SPhase::Recorded { hash, inscription, .. } | SPhase::Sold { hash, inscription, .. } =>
                    (*hash, inscription.clone()),
                _ => unreachable!("matched above"),
            };
            let ticket = s.ticket.ok_or("no ticket")?;
            let (t, _) = crate::saleflow::ticket(io, w, ticket).await?;
            match t.stage {
                Stage::Resolved => frogflow::send(io, st, FStep::SaleRecord { sale: s.id }, w, crate::redeem::record_delivery(ticket, hash)).await.map(Some),
                Stage::Delivered => match &s.phase {
                    SPhase::Listed { .. } => {
                        if let Some(x) = ss.sale_mut(s.id) {
                            x.phase = recorded(&s.phase);
                        }
                        Ok(Some("the ticket reads Delivered: the delivery-by-listing is recorded".into()))
                    }
                    SPhase::Sold { .. } => report(io, st, ss, s, me).await,
                    _ => Ok(None),
                },
                Stage::Refunded => {
                    // the listing is live and the item is back in the set: cancel the listing before anything else
                    let why = format!("ticket {ticket} refunded while {inscription} was listed: the listing is being cancelled");
                    let next = match outpoint_of(&s.phase) {
                        Some(outpoint) => SPhase::Delisting { inscription: inscription.clone(), outpoint },
                        None => SPhase::Stuck { why: format!("ticket {ticket} refunded after the item was sold: a person looks") },
                    };
                    if let Some(x) = ss.sale_mut(s.id) {
                        x.phase = next;
                    }
                    Ok(Some(why))
                }
                other => Err(format!("ticket {ticket} reads {other:?}")),
            }
        }
        SPhase::Redeposit { inscription, outpoint } => {
            let ticket = s.ticket.ok_or("no ticket")?;
            if frogflow::flag(io, w, crate::deposits::DEPOSIT_MINTED, crate::deposits::outpoint_key(outpoint.txid, outpoint.vout)).await? {
                if let Some(x) = ss.sale_mut(s.id) {
                    x.phase = if s.vault.is_some() { SPhase::Reclaiming } else { SPhase::Returned };
                }
                return Ok(Some("the wrapper says the moved outpoint has minted: adopted".into()));
            }
            match frogflow::depth(io, outpoint.txid).await? {
                Some(d) if d >= CONFIRMATIONS => {}
                Some(d) => return Err(format!("the move is only {d} deep now (a reorganisation)")),
                None => return Err("the move is no longer in a block (a reorganisation)".into()),
            }
            let proof = st.proof(inscription).ok_or_else(|| format!("awaiting a membership proof for {inscription} (frog_proof)"))?;
            let to = s.vault.unwrap_or(me);
            let data = crate::deposits::wrap_deposit(inscription, &proof, outpoint.txid, outpoint.vout, &to);
            let (key, collection) = (format!("mint:sale:{}", s.id), custodycap::btc_key(&w));
            io.outflow_check(&key, &collection, Flow::Mint, 1)?;
            let l = frogflow::send(io, st, FStep::SaleDeposit { sale: s.id }, w, data).await?;
            io.outflow_record(&key, &collection, Flow::Mint, 1);
            let _ = ticket;
            Ok(Some(format!("wrapDeposit of {inscription} to {}: {l}", h(&to))))
        }
        SPhase::Reclaiming => {
            let (ticket, vault) = (s.ticket.ok_or("no ticket")?, s.vault.ok_or("no vault")?);
            frogflow::send(io, st, FStep::SaleReclaim { sale: s.id }, vault, sale::reclaim_listing(ticket)).await.map(Some)
        }
        _ => Ok(None),
    }
}

fn outpoint_of(p: &SPhase) -> Option<Outpoint> {
    match p {
        SPhase::Listed { outpoint, .. } | SPhase::Recorded { outpoint, .. } => Some(*outpoint),
        _ => None,
    }
}

fn recorded(p: &SPhase) -> SPhase {
    match p.clone() {
        SPhase::Listed { inscription, outpoint, ask_sats, ask_wei, rate, listing, hash } =>
            SPhase::Recorded { inscription, outpoint, ask_sats, ask_wei, rate, listing, hash },
        other => other,
    }
}

/// §3.4 / §4: the report, with the ETH: `itemSold{value}` on the module (the net proceeds at least its minimum now, the
/// module still holding an inventory unit, our 4663 balance holding the value and the gas), or `recordSale{value}` on the
/// vault (the net at least the ask less the allowance, else stuck). `Value` out, against the day's limit (H3).
async fn report<I: FrogIo>(io: &mut I, st: &mut FrogState, ss: &mut SaleState, s: &SaleRec, _me: [u8; 20]) -> Result<Option<String>, String> {
    let SPhase::Sold { proceeds_wei, ask_wei, .. } = &s.phase else { return Ok(None) };
    let (proceeds, ask_wei) = (*proceeds_wei, *ask_wei);
    let ticket = s.ticket.ok_or("no ticket")?;
    let (to, data, what) = match (s.module, s.vault) {
        (Some(m), _) => {
            let (_, x) = read_module(io, m, None).await?;
            if proceeds < x.min_sale_proceeds {
                return Err(format!("the net proceeds {proceeds} wei are under the module's minimum {} now (the floor rose): waits", x.min_sale_proceeds));
            }
            if x.float_units < ONE {
                return Err("the module holds no inventory unit to hand back: waits".into());
            }
            (m, sale::item_sold(proceeds), "itemSold")
        }
        (None, Some(v)) => {
            let min = sale::relist_min_proceeds(ask_wei);
            if proceeds < min {
                if let Some(x) = ss.sale_mut(s.id) {
                    x.phase = SPhase::Stuck { why: format!("the realised net {proceeds} wei is under the vault's minimum {min} (the rate moved): a person looks") };
                }
                return Ok(Some("STUCK: the realised proceeds are under the vault's allowance".into()));
            }
            (v, sale::record_sale(ticket), "recordSale")
        }
        _ => return Err("a sale with neither a module nor a vault".into()),
    };
    // conversion review C-04: the 4663 balance is shared with the Relay leg; while either side has money moving through it (an
    // advance or a refund being watched, a conversion's ETH waiting for its quote) the sale's ETH is not told apart from it: wait
    if crate::convflow::holds_relay(st) || io.eth_relay_busy() {
        return Err(format!("{what} waits (C-04): the Relay leg is in use at our 4663 address; the sale's ETH is told apart only once it is free"));
    }
    let balance = io.our_balance().await?;
    let need = proceeds.saturating_add(sale::REPORT_GAS_MARGIN_WEI);
    if balance < need {
        return Err(format!("the 4663 balance holds {balance} wei; {need} are needed for {what}: the sale's sats wait to be bridged"));
    }
    let (key, collection) = (format!("sale:{}", s.id), custodycap::btc_key(&io.wrapper()));
    io.outflow_check(&key, &collection, Flow::Value, proceeds)?;
    let l = frogflow::send_value(io, st, FStep::SaleReport { sale: s.id }, to, proceeds, data).await?;
    io.outflow_record(&key, &collection, Flow::Value, proceeds);
    Ok(Some(format!("{what} with {proceeds} wei: {l}")))
}

/// A mined sale step: on success the next phase; on a revert the chain's own word where it has one, else read again.
pub async fn mined<I: FrogIo>(io: &mut I, ss: &mut SaleState, step: &FStep, success: bool) -> String {
    let me = io.our_evm().await.ok();
    match *step {
        FStep::Relist { sale, count_before } => {
            let Some(s) = ss.sales.iter().find(|s| s.id == sale).cloned() else { return "an unknown sale".into() };
            if !success {
                if let Some(x) = ss.sale_mut(sale) {
                    x.phase = SPhase::Planned;
                }
                return "redeemToRelist reverted: planned again (an estimate guards the next attempt)".into();
            }
            // our ticket: the first at or after the count read before sending whose owner is us
            let w = s.wrapper;
            let found = async {
                let count = ticket_count(io, w).await?;
                for c in count_before..count {
                    let (_, owner) = ticket(io, w, c).await?;
                    if Some(owner) == me && !ss.has_ticket(c) {
                        return Ok::<_, String>(Some(c));
                    }
                }
                Ok(None)
            }.await;
            match found {
                Ok(Some(c)) => {
                    if let Some(x) = ss.sale_mut(sale) {
                        x.ticket = Some(c);
                        x.phase = SPhase::Pinned;
                    }
                    format!("ticket {c} is ours: waiting for its round")
                }
                Ok(None) => "mined, but no ticket of ours is visible yet: the ticket walk adopts it".into(),
                Err(e) => format!("mined; the tickets are not read ({e}): next tick"),
            }
        }
        FStep::SaleRecord { sale } => {
            let Some(x) = ss.sale_mut(sale) else { return "an unknown sale".into() };
            if success {
                x.phase = recorded(&x.phase);
                "recordDelivery mined: the delivery-by-listing is recorded".into()
            } else {
                "recordDelivery reverted: the ticket is read again next tick".into()
            }
        }
        FStep::SaleReport { sale } => {
            let Some(x) = ss.sale_mut(sale) else { return "an unknown sale".into() };
            match (&x.phase, success) {
                (SPhase::Sold { proceeds_wei, .. }, true) => {
                    x.phase = SPhase::Reported { proceeds_wei: *proceeds_wei };
                    "reported with the ETH: the sale is done".into()
                }
                _ => "the report reverted: read again next tick (an estimate guards the next attempt)".into(),
            }
        }
        FStep::SaleDeposit { sale } => {
            let Some(s) = ss.sales.iter().find(|s| s.id == sale).cloned() else { return "an unknown sale".into() };
            let SPhase::Redeposit { outpoint, .. } = s.phase else { return "not a re-deposit".into() };
            let minted = success || frogflow::flag(io, s.wrapper, crate::deposits::DEPOSIT_MINTED, crate::deposits::outpoint_key(outpoint.txid, outpoint.vout))
                .await.unwrap_or(false);
            let Some(x) = ss.sale_mut(sale) else { return "an unknown sale".into() };
            if minted {
                x.phase = if s.vault.is_some() { SPhase::Reclaiming } else { SPhase::Returned };
                if s.vault.is_some() { "re-deposited to the vault: reclaimListing is next".into() } else { "re-deposited to the canister: the seed is back".into() }
            } else {
                "wrapDeposit reverted: checked again next tick".into()
            }
        }
        FStep::SaleReclaim { sale } => {
            let Some(x) = ss.sale_mut(sale) else { return "an unknown sale".into() };
            if success {
                x.phase = SPhase::Returned;
                "reclaimed: the unit is the coin's again".into()
            } else {
                "reclaimListing reverted: sent again next tick (an estimate guards it)".into()
            }
        }
        _ => "not a sale step".into(),
    }
}
