//! The conversion's steps in the frog tick (docs/v6/relay-conversion.md §3.3): the advance, the judged deposit, the watch for
//! the arrival or the refund, the return. Sequencing only; the rules are `relayconv`'s, the EVM path is `frogflow::send_value`
//! (one transaction canister-wide, its bytes saved before it is sent).

use crate::custodycap;
use crate::floortrigger::{FloorState, WPhase};
use crate::frogbuy::{FPhase, Target};
use crate::frogflow::{addr, depth, read_sink, send_value, FStep, FrogIo, FrogState};
use crate::itemsink::{self, SinkKind};
use crate::outflow::Flow;
use crate::relayconv::{self as rc, CPhase, ConvRec, Watch};

impl FrogState {
    pub fn convs(&self) -> &[ConvRec] {
        self.convs.as_deref().unwrap_or_default()
    }
    pub fn conv(&self, frog: u64) -> Option<&ConvRec> {
        self.convs().iter().rev().find(|c| c.frog == frog)
    }
    pub fn conv_mut(&mut self, frog: u64) -> Option<&mut ConvRec> {
        self.convs.as_mut()?.iter_mut().rev().find(|c| c.frog == frog)
    }
    /// A conversion not finished (`Stuck` included): nothing new starts while one is.
    pub fn conv_open(&self) -> bool {
        self.convs().iter().any(|c| !c.finished())
    }
    /// The module paid for this frog: it is never released for "no longer needed", and its room stays reserved.
    pub fn conv_committed(&self, frog: u64) -> bool {
        self.conv(frog).is_some_and(|c| !c.finished() && c.committed())
    }
    /// The frog's conversion, if the module has advanced for it (settling reports `settleAdvance`, not `itemArrived`).
    pub fn advanced_for(&self, frog: u64) -> Option<&ConvRec> {
        self.conv(frog).filter(|c| c.advanced > 0 && !matches!(c.phase, CPhase::Returned { .. } | CPhase::Returning { .. }))
    }
}

/// A new conversion for a frog just wanted for a Bitcoin floor sink.
pub fn start(st: &mut FrogState, frog: u64, module: [u8; 20]) {
    st.convs.get_or_insert_with(Vec::new).push(ConvRec::new(frog, module));
}

/// A frog released before its advance was sent: its conversion never did anything and is forgotten.
pub fn drop_uncommitted(st: &mut FrogState, frog: u64) {
    if let Some(v) = st.convs.as_mut() {
        v.retain(|c| !(c.frog == frog && !c.committed()));
    }
}

fn phase_text(p: &CPhase, unit: &str) -> String {
    match p {
        CPhase::NeedAdvance => "the advance is next".into(),
        CPhase::Advancing => "the advance is out".into(),
        CPhase::Funded { wei, .. } => format!("{wei} {unit} at our address, waiting for a quote"),
        CPhase::Depositing { .. } => "the deposit is out".into(),
        CPhase::Converting { min_sats, .. } => format!("converting, at least {min_sats} sats expected at the float"),
        CPhase::Arrived { sats, .. } => format!("{sats} sats arrived"),
        CPhase::Returning { wei } => format!("returning {wei} {unit} to the module"),
        CPhase::Returned { wei } => format!("returned {wei} {unit}"),
        CPhase::Settled { cost, spare_sats } => format!("settled at {cost} {unit}, {spare_sats} sats spare"),
        CPhase::Stuck { why } => format!("STUCK: {why}"),
    }
}

/// Why a frog's offer waits, or `None` when its conversion has arrived (or it has none).
pub fn waits(st: &FrogState, frog: u64) -> Option<String> {
    waits_in(st, frog, "wei")
}

/// [`waits`], amounts named in the home's unit (phase B2: lamports on Solana).
pub fn waits_in(st: &FrogState, frog: u64, unit: &str) -> Option<String> {
    let c = st.conv(frog).filter(|c| !c.finished())?;
    match c.phase {
        CPhase::Arrived { .. } => None,
        ref p => Some(format!("waits for its conversion ({})", phase_text(p, unit))),
    }
}

/// `(advanced, arrived sats)` of a frog whose conversion has arrived: what its buy may spend and what it cost.
pub fn funded(st: &FrogState, frog: u64) -> Option<(u128, u64)> {
    let c = st.conv(frog).filter(|c| !c.finished())?;
    c.arrived_sats().map(|s| (c.advanced, s))
}

async fn float_address<I: FrogIo>(io: &mut I) -> Result<String, String> {
    io.float_address().await
}

async fn ask_for<I: FrogIo>(io: &mut I, wei: u128) -> Result<rc::ConvAsk, String> {
    // phase B2: from our Solana address (lamports) on the Solana home
    if crate::frogsol::solana(io) {
        return crate::frogsol::ask(io, wei).await;
    }
    let me = io.our_evm().await?;
    let float = float_address(io).await?;
    Ok(rc::ask(wei, &me, &float, io.chain_id()))
}

fn collection<I: FrogIo>(io: &I) -> String {
    crate::frogsol::collection_key(io)
}

/// What an advance keeps at our home address for its own fees (wei, or lamports on Solana).
fn keep<I: FrogIo>(io: &I) -> u128 {
    crate::frogsol::gas_keep(crate::frogsol::solana(io))
}

// ---- the watch ------------------------------------------------------------------------------------------------------------------

/// Each tick: a mined deposit is watched for its arrival at the float (three deep), a refund at our address, or its deadline; a
/// written-off order (C-03) for its late refund or fill.
pub async fn pass<I: FrogIo>(io: &mut I, st: &mut FrogState) -> Vec<String> {
    let mut lines = Vec::new();
    // C-01: an arrival saved before its clock existed starts it now
    let now = io.now();
    if let Some(v) = st.convs.as_mut() {
        for c in v.iter_mut().filter(|c| matches!(c.phase, CPhase::Arrived { .. }) && c.arrived_at.is_none()) {
            c.arrived_at = Some(now);
        }
    }
    let ids: Vec<(u64, bool)> = st.convs().iter()
        .filter_map(|c| {
            if matches!(c.phase, CPhase::Converting { .. } | CPhase::Stuck { .. }) && c.last.is_some() {
                Some((c.frog, false))
            } else if matches!(c.phase, CPhase::Returned { .. }) && c.late_open() {
                Some((c.frog, true))
            } else {
                None
            }
        }).collect();
    for (frog, late) in ids {
        let r = if late { watch_late(io, st, frog).await } else { watch(io, st, frog).await };
        let line = match r {
            Ok(s) => s,
            Err(e) => format!("waits: {e}"),
        };
        if let Some(c) = st.conv_mut(frog) {
            c.note = line.clone();
        }
        io.save(st);
        lines.push(format!("conversion for frog {frog}: {line}"));
    }
    lines
}

async fn watch<I: FrogIo>(io: &mut I, st: &mut FrogState, frog: u64) -> Result<String, String> {
    let c = st.conv(frog).cloned().ok_or("no conversion")?;
    let w = c.last.clone().ok_or("nothing is being watched")?;
    let stuck = matches!(c.phase, CPhase::Stuck { .. });
    // 1. the arrival: a confirmed float output that was not there before the deposit, at least the order's minimum
    let utxos = io.float_utxos().await?;
    let before = c.float_before.clone().unwrap_or_default();
    if let Some(a) = rc::arrival_among(&utxos, &before, w.min_sats) {
        return match depth(io, a.outpoint.txid).await? {
            Some(d) if d >= rc::ARRIVAL_CONFS => {
                let x = st.conv_mut(frog).expect("read above");
                x.phase = CPhase::Arrived { order_id: w.order_id, sats: a.sats };
                x.arrival = Some(a.clone());
                x.arrived_at = Some(io.now());
                Ok(format!("ARRIVED: {} sats at {} ({d} deep): the frog may be bought with them", a.sats, a.outpoint.text()))
            }
            Some(d) => Ok(format!("{} sats seen at {}, {d} of {} blocks deep", a.sats, a.outpoint.text(), rc::ARRIVAL_CONFS)),
            None => Ok(format!("{} sats seen at {}, not yet in a block", a.sats, a.outpoint.text())),
        };
    }
    // 2. the refund: our balance back up by at least half the deposit
    let bal = io.our_balance().await?;
    if rc::refunded(bal, w.balance_after, w.value) {
        let back = bal.saturating_sub(keep(io)).min(w.value);
        let attempts = c.attempts;
        // audit C2-13: a partial refund under the least deposit Relay fills (0.15 SOL less the keep, on the Solana home) would be re-funded
        // for a deposit the judge always refuses; it goes straight back to its module instead
        let least = if crate::frogsol::solana(io) { crate::frogsol::RELAY_SOL_BTC_MIN_LAMPORTS } else { 1 };
        let next = if attempts < rc::MAX_ATTEMPTS && back >= least {
            CPhase::Funded { wei: back, since: io.now() }
        } else {
            CPhase::Returning { wei: bal.saturating_sub(keep(io)).min(c.advanced) }
        };
        let ask = match next {
            CPhase::Funded { wei, .. } => Some(ask_for(io, wei).await?),
            _ => None,
        };
        // phase B2: on Solana the memo names the order (a label only: the balance rule decided)
        let label = if crate::frogsol::solana(io) { crate::frogsol::refund_label(io, &w.order_id).await } else { String::new() };
        let unit = if crate::frogsol::solana(io) { "lamports" } else { "wei" };
        let x = st.conv_mut(frog).expect("read above");
        x.last = None;
        x.ask = ask;
        let text = match &next {
            CPhase::Funded { wei, .. } => format!("REFUNDED: {wei} {unit} back at our address{label}; attempt {} of {} next", attempts + 1, rc::MAX_ATTEMPTS),
            CPhase::Returning { wei } => format!("REFUNDED {attempts} times: {wei} {unit} goes back to the module{label}"),
            _ => unreachable!(),
        };
        x.phase = next;
        return Ok(text);
    }
    // 3. overdue: past the order's deadline with neither
    let now = io.now();
    // C-05: past the deadline, a fill split across outputs or Relay's bitcoin-side refund (under the minimum): the new confirmed
    // float outputs, summed, at least half the minimum, every one three deep
    if now > w.deadline {
        if let Some((a, outs)) = rc::arrival_summed(&utxos, &before, w.min_sats) {
            let mut shallow = None;
            for t in outs.iter().map(|o| o.txid).collect::<std::collections::BTreeSet<_>>() {
                match depth(io, t).await? {
                    Some(d) if d >= rc::ARRIVAL_CONFS => {}
                    d => {
                        shallow = Some((t, d.unwrap_or(0)));
                        break;
                    }
                }
            }
            if let Some((t, d)) = shallow {
                return Ok(format!("past the deadline: {} sats in {} new float outputs; {} is {d} of {} blocks deep", a.sats, outs.len(),
                    hex::encode(t), rc::ARRIVAL_CONFS));
            }
            let x = st.conv_mut(frog).expect("read above");
            x.phase = CPhase::Arrived { order_id: w.order_id, sats: a.sats };
            x.arrival = Some(a.clone());
            x.arrived_at = Some(now);
            return Ok(format!("ARRIVED past the deadline (C-05): {} sats in {} new float outputs (a split fill or Relay's bitcoin-side refund, \
                at least {} bps of the {} minimum): the frog may be bought with them", a.sats, outs.len(), rc::REFUND_SHARE_BPS, w.min_sats));
        }
    }
    if now > w.deadline.saturating_add(rc::DEADLINE_GRACE_SECS) {
        if !stuck {
            st.conv_mut(frog).expect("read above").phase = CPhase::Stuck {
                why: format!("order 0x{} is past its deadline {} with no arrival of {} sats at the float and no refund at our address",
                    hex::encode(w.order_id), w.deadline, w.min_sats),
            };
        }
        return Ok("STUCK: past the order's deadline; still watching for a late fill or refund".into());
    }
    Ok(match c.phase {
        CPhase::Converting { since, .. } => format!("converting for {} s; no arrival or refund yet", now.saturating_sub(since)),
        _ => "still watched".into(),
    })
}

/// C-03: a written-off order (given back while `Stuck`), watched after the return: its fill at the float (the sats stay there,
/// recorded), or its refund at our address (paid into the module with `fundReserve`). Until one, or the operator's release, it holds
/// the Relay leg.
async fn watch_late<I: FrogIo>(io: &mut I, st: &mut FrogState, frog: u64) -> Result<String, String> {
    let c = st.conv(frog).cloned().ok_or("no conversion")?;
    let l = c.late.clone().ok_or("no written-off order")?;
    let order = hex::encode(l.watch.order_id);
    let unit = if crate::frogsol::solana(io) { "lamports" } else { "wei" }; // audit A5-I3
    if let Some(wei) = l.refund_wei {
        return Ok(format!("the written-off order 0x{order} was refunded: {wei} {unit} go into the module (fundReserve) next"));
    }
    let utxos = io.float_utxos().await?;
    let before = c.float_before.clone().unwrap_or_default();
    if let Some(a) = rc::arrival_among(&utxos, &before, l.watch.min_sats) {
        return match depth(io, a.outpoint.txid).await? {
            Some(d) if d >= rc::ARRIVAL_CONFS => {
                let x = st.conv_mut(frog).expect("read above");
                x.abandoned_sats = Some(a.sats);
                if let Some(y) = x.late.as_mut() {
                    y.filled = Some(a.clone());
                }
                Ok(format!("LATE FILL of the written-off order 0x{order}: {} sats at {} after the give-back; they stay in the float, owed \
                    to module 0x{} in the books (it took the return as its loss); the Relay leg is free", a.sats, a.outpoint.text(),
                    hex::encode(c.module)))
            }
            d => Ok(format!("the written-off order's fill seen at {}, {} of {} blocks deep", a.outpoint.text(), d.unwrap_or(0), rc::ARRIVAL_CONFS)),
        };
    }
    let bal = io.our_balance().await?;
    if rc::refunded(bal, l.balance_after, l.watch.value) {
        let wei = bal.saturating_sub(keep(io)).min(l.watch.value);
        let x = st.conv_mut(frog).expect("read above");
        let y = x.late.as_mut().expect("read above");
        if wei == 0 {
            y.credited = Some(0);
            return Ok(format!("the written-off order 0x{order} was refunded, but nothing above the gas keep is here: closed"));
        }
        y.refund_wei = Some(wei);
        return Ok(format!("LATE REFUND of the written-off order 0x{order}: {wei} {unit} go into module 0x{} (fundReserve) next",
            hex::encode(c.module)));
    }
    Ok(format!("the written-off order 0x{order} is still watched: it holds the Relay leg until its refund (paid to the module), its fill, \
        or relay_late_release"))
}

// ---- B-L-6: one Relay leg canister-wide ------------------------------------------------------------------------------------------

/// B-L-6 (docs/audit/2026-09-27-nft/B-canister-judges.md): both sides read a Relay refund as a rise of the same 4663 balance, so only one
/// side may have money moving through that address at a time. A conversion holds the leg from its advance (ETH arriving from the
/// module) until it has arrived as sats or gone back: `Advancing`, `Funded`, `Depositing`, `Converting`, `Returning`. Conversion review
/// C-02: a `Stuck` one still watched (its late fill or refund is read against the same balance) holds it too; C-03: so does a
/// written-off order still watched after its give-back.
pub fn holds_relay(st: &FrogState) -> bool {
    st.convs().iter().any(|c| matches!(c.phase, CPhase::Advancing | CPhase::Funded { .. } | CPhase::Depositing { .. }
        | CPhase::Converting { .. } | CPhase::Returning { .. })
        || (matches!(c.phase, CPhase::Stuck { .. }) && c.last.is_some()) || c.late_open())
}

/// Conversion review C-06: a refund watch reads our 4663 balance (a mined deposit, a stuck one still watched, a written-off order):
/// no reimbursement of ours may land there meanwhile.
pub fn watching(st: &FrogState) -> bool {
    st.convs().iter().any(|c| matches!(c.phase, CPhase::Depositing { .. } | CPhase::Converting { .. })
        || (matches!(c.phase, CPhase::Stuck { .. }) && c.last.is_some()) || c.late_open())
}

/// Conversion review C-06: a reimbursement of ours is owed soon (a frog minted or held for its target and not advanced for, whose
/// report or fulfill pays our address; an EVM item arrived, whose claim does): no new deposit is watched until it is paid.
pub fn reimbursement_owed(st: &FrogState, fl: &FloorState) -> Option<String> {
    if let Some(f) = st.frogs.iter().find(|f| f.target.is_some() && matches!(f.phase, FPhase::Minted { .. } | FPhase::Held { .. })
        && st.advanced_for(f.id).is_none()) {
        return Some(format!("frog {}'s reimbursement is owed first", f.id));
    }
    fl.wants.iter().find(|w| matches!(w.phase, WPhase::Arrived { .. })).map(|w| format!("nft want {}'s claim is owed first", w.id))
}

/// Conversion review C-08: the module's last advance came back less than `RETURN_COOLDOWN_SECS` ago: when it may advance again.
pub fn cooling(st: &FrogState, module: &[u8; 20], now: u64) -> Option<u64> {
    st.convs().iter().filter(|c| c.module == *module && matches!(c.phase, CPhase::Returned { .. })).filter_map(|c| c.returned_at).max()
        .map(|t| t.saturating_add(rc::RETURN_COOLDOWN_SECS)).filter(|t| now < *t)
}

/// B-L-6, fairness: a conversion waiting for the leg (`NeedAdvance`). The Ethereum side starts no new buy while one waits, so a steady
/// stream of Ethereum buys cannot keep the Bitcoin side off the leg for ever (its way back still goes first: that is money returning).
pub fn wants_relay(st: &FrogState) -> bool {
    st.convs().iter().any(|c| matches!(c.phase, CPhase::NeedAdvance))
}

// ---- the EVM acts -----------------------------------------------------------------------------------------------------------------

/// The conversion's EVM act, if one is due: the advance, a judged deposit, or the return. One at most.
pub async fn evm_act<I: FrogIo>(io: &mut I, st: &mut FrogState, fl: &FloorState, errs: &mut Vec<String>) -> Option<String> {
    // C-03: a written-off order's late refund goes into its module first: that is money returning
    if let Some(c) = st.convs().iter().find(|c| c.late.as_ref().is_some_and(|l| l.open() && l.refund_wei.is_some())).cloned() {
        match late_credit(io, st, &c).await {
            Ok(s) => return s,
            Err(e) => {
                if let Some(x) = st.conv_mut(c.frog) {
                    x.note = e.clone();
                }
                errs.push(format!("conversion for frog {}: {e}", c.frog));
                return None;
            }
        }
    }
    let c = st.convs().iter().find(|c| matches!(c.phase, CPhase::NeedAdvance | CPhase::Funded { .. } | CPhase::Returning { .. }))?.clone();
    // B-L-6: a new advance waits while the Ethereum side holds the Relay leg (its refund watch would read the advance as its refund),
    // and (C-02, C-03) while a conversion of this side still watches the balance (a stuck order, a written-off one)
    let wait = if matches!(c.phase, CPhase::NeedAdvance) && io.eth_relay_busy() {
        Some("waits: the Ethereum side holds the one Relay leg (B-L-6); the advance follows when it is free".to_string())
    } else if matches!(c.phase, CPhase::NeedAdvance) && holds_relay(st) {
        Some("waits: an earlier conversion of ours still holds the Relay leg (a stuck or written-off order, C-02/C-03)".to_string())
    } else if matches!(c.phase, CPhase::NeedAdvance | CPhase::Funded { .. }) {
        // C-06: no reimbursement of ours may land while the deposit is watched (it would read as its refund)
        reimbursement_owed(st, fl).map(|w| format!("waits (C-06): {w}; a deposit is watched only once nothing else is to land at our address"))
    } else {
        None
    };
    if let Some(w) = wait {
        if let Some(x) = st.conv_mut(c.frog) {
            x.note = w;
        }
        return None;
    }
    let r = match c.phase {
        CPhase::NeedAdvance => advance(io, st, fl, &c).await,
        CPhase::Funded { wei, .. } => deposit(io, st, &c, wei).await,
        CPhase::Returning { wei } => give_back(io, st, &c, wei).await,
        _ => Ok(None),
    };
    match r {
        Ok(s) => s,
        Err(e) => {
            if let Some(x) = st.conv_mut(c.frog) {
                x.note = e.clone();
            }
            errs.push(format!("conversion for frog {}: {e}", c.frog));
            None
        }
    }
}

async fn advance<I: FrogIo>(io: &mut I, st: &mut FrogState, fl: &FloorState, c: &ConvRec) -> Result<Option<String>, String> {
    let f = st.frogs.iter().find(|f| f.id == c.frog).cloned().ok_or("its frog is gone")?;
    let FPhase::Wanted { cap_wei } = f.phase else { return Err(format!("its frog is {:?}, not wanted", f.phase)) };
    let limit = fl.limit_for(&c.module);
    let v = read_sink(io, SinkKind::Floor, c.module, limit).await?;
    // the module still wants it, now: the frog on its way counts itself
    itemsink::may_start(&v, itemsink::assigned(&st.frogs, &c.module).0.saturating_sub(1))?;
    let amount = cap_wei.min(v.item_cap.unwrap_or(cap_wei));
    let sol = crate::frogsol::solana(io);
    crate::frogsol::check_limits_min(amount, limit, fl.min_for(&c.module), sol)?;
    // phase B2: Relay's SOL to BTC capacity (spike Q5: ≈ 250 SOL an order): a larger advance could never be converted in one order
    if sol && amount > crate::frogsol::RELAY_SOL_BTC_MAX_LAMPORTS {
        return Err(format!("{amount} lamports is over Relay's SOL to BTC capacity of {} an order: no advance", crate::frogsol::RELAY_SOL_BTC_MAX_LAMPORTS));
    }
    if crate::frogsol::convertible(amount, sol).is_none() {
        return Err(format!("{amount} {} would leave nothing to convert after the gas keep", if sol { "lamports" } else { "wei" }));
    }
    // H3 and H2, before the module pays: the day's value limit, then the item's room in custody, reserved at the advance (in wei:
    // lamports converted at this tick's SOL/ETH rate on Solana)
    let (value_key, flight, col) = (format!("buy:frog:{}", c.frog), format!("frog:{}", c.frog), collection(io));
    let value = crate::frogsol::value_wei(io, amount).await?;
    io.outflow_check(&value_key, &col, Flow::Value, value)?;
    io.with_caps(|p, b| custodycap::reserve(p, b, &flight, &col, value)).map_err(|e| format!("custody cap: {e:?}"))?;
    let sent = if sol {
        crate::frogsol::send(io, st, FStep::Advance { frog: c.frog }, crate::frogsol::Call::Advance(amount)).await
    } else {
        send_value(io, st, FStep::Advance { frog: c.frog }, c.module, 0, rc::advance_call(amount)).await
    };
    match sent {
        Ok(s) => {
            if sol {
                st.put_value_pub(c.frog, value);
            }
            io.outflow_record(&value_key, &col, Flow::Value, value);
            let x = st.conv_mut(c.frog).expect("found above");
            x.advanced = amount;
            x.phase = CPhase::Advancing;
            x.note = String::new();
            io.save(st);
            Ok(Some(format!("frog {}: advance of {amount} {}: {s}", c.frog, if sol { "lamports" } else { "wei" })))
        }
        Err(e) => {
            let _ = io.with_caps(|_, b| custodycap::settle(b, &flight, None));
            Err(e)
        }
    }
}

async fn deposit<I: FrogIo>(io: &mut I, st: &mut FrogState, c: &ConvRec, wei: u128) -> Result<Option<String>, String> {
    let Some((body, at)) = io.take_quote(c.frog) else {
        return Ok(None); // the runner has not quoted yet: the ask stands
    };
    if crate::frogsol::solana(io) {
        // audit A5-07: the line's turn first, before the quote is consumed or any read is paid for (a busy line keeps the quote)
        let waits = if st.sol_step.is_some() { Some("the frog side's step is still out on the Solana line".to_string()) }
            else { crate::solhome::turn(io, crate::solhome::LineOwner::Frog, io.now()).err() };
        if let Some(e) = waits {
            io.return_quote(c.frog, body, at);
            return Err(format!("frog {}: the quote waits (kept): {e}", c.frog));
        }
        return deposit_sol(io, st, c, wei, &body, at).await;
    }
    let rate = io.rate().await?;
    let allocator = addr(io, rc::RELAY_DEPOSITORY_4663, rc::ALLOCATOR).await?;
    let me = io.our_evm().await?;
    let float = float_address(io).await?;
    let mut used = rc::used_orders(st.convs());
    used.extend(io.used_elsewhere());
    let funding = io.funding().ok_or("frog funding is not Relay here")?;
    let want = rc::Want { chain_id: io.chain_id(), our_evm: me, float_address: &float, amount_wei: wei, sats_per_eth: rate,
        max_fee_bps: funding.max_fee_bps, allocator, now: io.now(), handed_in_at: at, used: &used };
    let d = rc::judge(&body, &want).map_err(|e| format!("quote refused, nothing signed: {e}"))?;
    let bal = io.our_balance().await?;
    if bal < d.value {
        return Err(format!("our balance {bal} wei is under the deposit's {} wei", d.value));
    }
    let before: Vec<crate::custody::Outpoint> = io.float_utxos().await?.iter().map(|u| u.outpoint).collect();
    {
        // the order is used from here on, sent or not (rule 12): recorded before the bytes exist
        let x = st.conv_mut(c.frog).expect("found above");
        x.orders.push(d.order_id);
        x.float_before = Some(before);
        x.balance_before = Some(bal);
        x.phase = CPhase::Depositing { order_id: d.order_id, value: d.value, min_sats: d.min_sats, deadline: d.deadline };
        x.ask = None;
    }
    io.save(st);
    match send_value(io, st, FStep::Convert { frog: c.frog }, d.to, d.value, d.data.clone()).await {
        Ok(s) => {
            let hash = st.pending.as_ref().map(|p| p.hash).unwrap_or_default();
            let x = st.conv_mut(c.frog).expect("found above");
            x.deposits.push(hash);
            x.attempts += 1;
            x.note = format!("deposit of order 0x{} for at least {} sats", hex::encode(d.order_id), d.min_sats);
            Ok(Some(format!("frog {}: Relay deposit of {} wei (order 0x{}, at least {} sats to the float): {s}", c.frog, d.value,
                hex::encode(d.order_id), d.min_sats)))
        }
        Err(e) => {
            let ask = ask_for(io, wei).await.ok();
            let x = st.conv_mut(c.frog).expect("found above");
            x.phase = CPhase::Funded { wei, since: io.now() };
            x.ask = ask;
            Err(e)
        }
    }
}

/// Phase B2: a funded conversion's quote judged with a Solana origin and the float as its Bitcoin recipient (`frogsol::judge`, rules
/// S1–S12), then our own `deposit_native` on the line. The order is used from the judgement on, sent or not (rule 12).
async fn deposit_sol<I: FrogIo>(io: &mut I, st: &mut FrogState, c: &ConvRec, lamports: u128, body: &[u8], at: u64) -> Result<Option<String>, String> {
    let mut used = rc::used_orders(st.convs());
    used.extend(io.used_elsewhere());
    let funding = io.funding().ok_or("frog funding is not Relay here")?;
    let d = crate::frogsol::judge(io, body, at, lamports, funding.max_fee_bps, &used).await.map_err(|e| format!("quote refused, nothing signed: {e}"))?;
    let bal = io.our_balance().await?;
    // the deposit, the Relay key's fee for it, our fee for its funding (audit A5-03)
    let need = d.lamports as u128 + 2 * crate::solsend::SIGNATURE_FEE_LAMPORTS as u128;
    if bal < need {
        return Err(format!("our Solana balance {bal} lamports is under the deposit's {} and its fees ({need})", d.lamports));
    }
    let before: Vec<crate::custody::Outpoint> = io.float_utxos().await?.iter().map(|u| u.outpoint).collect();
    let min_sats = u64::try_from(d.min_out).map_err(|_| "the order's minimum is over a u64 of sats")?;
    {
        let x = st.conv_mut(c.frog).expect("found above");
        x.orders.push(d.order_id);
        x.float_before = Some(before);
        x.balance_before = Some(bal);
        x.phase = CPhase::Depositing { order_id: d.order_id, value: d.lamports as u128, min_sats, deadline: d.deadline };
        x.ask = None;
    }
    io.save(st);
    match crate::frogsol::send(io, st, FStep::Convert { frog: c.frog }, crate::frogsol::Call::Deposit(d.clone())).await {
        Ok(s) => {
            let x = st.conv_mut(c.frog).expect("found above");
            x.attempts += 1;
            x.note = format!("deposit of order 0x{} for at least {min_sats} sats", hex::encode(d.order_id));
            Ok(Some(format!("frog {}: Relay deposit of {} lamports from Solana (order 0x{}, at least {min_sats} sats to the float): {s}", c.frog,
                d.lamports, hex::encode(d.order_id))))
        }
        Err(e) => {
            let ask = ask_for(io, lamports).await.ok();
            let x = st.conv_mut(c.frog).expect("found above");
            x.phase = CPhase::Funded { wei: lamports, since: io.now() };
            x.ask = ask;
            Err(e)
        }
    }
}

async fn give_back<I: FrogIo>(io: &mut I, st: &mut FrogState, c: &ConvRec, wei: u128) -> Result<Option<String>, String> {
    let bal = io.our_balance().await?;
    if bal < wei {
        return Err(format!("our balance {bal} is under the {wei} to return"));
    }
    if crate::frogsol::solana(io) {
        let s = crate::frogsol::send(io, st, FStep::GiveBack { frog: c.frog }, crate::frogsol::Call::Return(wei)).await?;
        return Ok(Some(format!("frog {}: return_advance of {wei} lamports: {s}", c.frog)));
    }
    let s = send_value(io, st, FStep::GiveBack { frog: c.frog }, c.module, wei, rc::return_call()).await?;
    Ok(Some(format!("frog {}: returnAdvance of {wei} wei: {s}", c.frog)))
}

/// C-03: a written-off order's late refund into its module: `fundReserve{value}` (its advance is closed: `returnAdvance` would
/// revert). Nowhere else.
async fn late_credit<I: FrogIo>(io: &mut I, st: &mut FrogState, c: &ConvRec) -> Result<Option<String>, String> {
    let wei = c.late.as_ref().and_then(|l| l.refund_wei).ok_or("no late refund")?;
    let bal = io.our_balance().await?;
    if bal < wei {
        return Err(format!("our balance {bal} is under the {wei} of the late refund"));
    }
    if crate::frogsol::solana(io) {
        let s = crate::frogsol::send(io, st, FStep::LateCredit { frog: c.frog }, crate::frogsol::Call::Reserve(wei)).await?;
        return Ok(Some(format!("frog {}: the written-off order's late refund, {wei} lamports, into the module (fund_reserve): {s}", c.frog)));
    }
    let s = send_value(io, st, FStep::LateCredit { frog: c.frog }, c.module, wei, crate::ethflow::fund_reserve_call()).await?;
    Ok(Some(format!("frog {}: the written-off order's late refund, {wei} wei, into the module (fundReserve): {s}", c.frog)))
}

// ---- mined -------------------------------------------------------------------------------------------------------------------------

/// Our conversion transaction is in a block.
pub async fn mined<I: FrogIo>(io: &mut I, st: &mut FrogState, step: &FStep, success: bool) -> String {
    match *step {
        FStep::Advance { frog } => {
            let Some(c) = st.conv(frog).cloned() else { return "an unknown conversion".into() };
            if !success {
                let _ = io.with_caps(|_, b| custodycap::settle(b, &format!("frog:{frog}"), None));
                let x = st.conv_mut(frog).expect("found");
                x.phase = CPhase::NeedAdvance;
                x.advanced = 0;
                return "the advance reverted: nothing was paid; tried again while the module wants the frog".into();
            }
            let next = match crate::frogsol::convertible(c.advanced, crate::frogsol::solana(io)) {
                Some(w) => CPhase::Funded { wei: w, since: io.now() },
                None => CPhase::Returning { wei: c.advanced },
            };
            let ask = match next {
                CPhase::Funded { wei, .. } => ask_for(io, wei).await.ok(),
                _ => None,
            };
            let x = st.conv_mut(frog).expect("found");
            x.phase = next;
            x.ask = ask;
            format!("the module advanced {} {}: the runner is asked for a Relay quote", c.advanced,
                if crate::frogsol::solana(io) { "lamports" } else { "wei" })
        }
        FStep::Convert { frog } => {
            let Some(c) = st.conv(frog).cloned() else { return "an unknown conversion".into() };
            let CPhase::Depositing { order_id, value, min_sats, deadline } = c.phase else { return "not depositing".into() };
            if !success {
                let ask = ask_for(io, value).await.ok();
                let x = st.conv_mut(frog).expect("found");
                x.phase = CPhase::Funded { wei: value, since: io.now() };
                x.ask = ask;
                return "the deposit reverted: the ETH is still ours; a new quote is asked".into();
            }
            let after = c.balance_before.unwrap_or(0).saturating_sub(value);
            let x = st.conv_mut(frog).expect("found");
            x.last = Some(Watch { order_id, value, min_sats, deadline, balance_after: after });
            x.phase = CPhase::Converting { order_id, value, min_sats, deadline, since: io.now(), balance_after: after };
            format!("deposited with Relay: order 0x{}, at least {min_sats} sats to the float by {deadline}", hex::encode(order_id))
        }
        FStep::GiveBack { frog } => {
            let Some(c) = st.conv(frog).cloned() else { return "an unknown conversion".into() };
            let CPhase::Returning { wei } = c.phase else { return "not returning".into() };
            if !success {
                return "returnAdvance reverted: sent again next tick".into();
            }
            let _ = io.with_caps(|_, b| custodycap::settle(b, &format!("frog:{frog}"), None));
            let now = io.now();
            let x = st.conv_mut(frog).expect("found");
            x.phase = CPhase::Returned { wei };
            x.returned_at = Some(now); // C-08
            // audit A5-I3: the home's unit (lamports on the Solana home)
            let unit = if crate::frogsol::solana(io) { "lamports" } else { "wei" };
            if let Some(f) = st.frogs.iter_mut().find(|f| f.id == frog) {
                if matches!(f.phase, FPhase::Wanted { .. }) {
                    f.phase = FPhase::Failed { why: format!("its conversion was returned to the module ({wei} {unit}): nothing bought") };
                }
            }
            format!("{wei} {unit} returned to the module: the unit is ETH-backed again; the frog is not bought")
        }
        FStep::LateCredit { frog } => {
            let unit = if crate::frogsol::solana(io) { "lamports" } else { "wei" }; // audit A5-I3
            let Some(x) = st.conv_mut(frog) else { return "an unknown conversion".into() };
            let Some(l) = x.late.as_mut() else { return "no written-off order".into() };
            if !success {
                return "fundReserve reverted: sent again next tick (an estimate guards it)".into();
            }
            l.credited = l.refund_wei;
            format!("the written-off order's late refund ({} {unit}) is in the module's reserve: the Relay leg is free", l.refund_wei.unwrap_or(0))
        }
        _ => "not a conversion step".into(),
    }
}

/// The report of a frog the module advanced for: `settleAdvance(cost)`.
pub fn report_call(st: &FrogState, frog: u64, cost: u128) -> Option<Vec<u8>> {
    st.advanced_for(frog).map(|_| rc::settle_call(cost))
}

/// The report mined: the conversion is settled, with the sats the frog did not use.
pub fn settled(st: &mut FrogState, frog: u64, spend_sats: u64, cost: u128) {
    if let Some(x) = st.conv_mut(frog) {
        let spare = x.arrival.as_ref().map_or(0, |a| a.sats.saturating_sub(spend_sats));
        x.phase = CPhase::Settled { cost, spare_sats: spare };
    }
}

/// A frog of a Bitcoin floor sink that has a conversion.
pub fn is_floor_btc(fl: &FloorState, t: &Option<Target>) -> Option<[u8; 20]> {
    match t {
        Some(Target::Sink(s)) if !fl.is_evm(s) => Some(*s),
        _ => None,
    }
}

// ---- endpoints' pure parts -------------------------------------------------------------------------------------------------------------

/// `relay_give_back`: a funded (or stuck) conversion's ETH goes back to the module. `balance`: ours now, for a stuck one. Conversion
/// review C-03: a stuck one's live order is not forgotten: it is kept as written off (`late`), still watched after the return and
/// still holding the Relay leg, so its late refund is paid into the module and read by no one else, and its late fill is recorded.
pub fn give_back_now(st: &mut FrogState, frog: u64, balance: u128, now: u64) -> Result<String, String> {
    give_back_now_keeping(st, frog, balance, now, rc::GAS_KEEP_WEI)
}

/// [`give_back_now`] with the home's own gas keep (phase B2: lamports on Solana).
pub fn give_back_now_keeping(st: &mut FrogState, frog: u64, balance: u128, now: u64, keep: u128) -> Result<String, String> {
    // audit A5-I3: the home's unit (the keep is the home's own: lamports on the Solana home)
    let unit = if keep == crate::solhome::SOL_GAS_KEEP_LAMPORTS { "lamports" } else { "wei" };
    let c = st.conv_mut(frog).ok_or_else(|| format!("no conversion for frog {frog}"))?;
    let wei = match c.phase {
        // what the conversion would have sent; the gas keep stays to pay for the return itself
        CPhase::Funded { wei, .. } => wei.min(balance.saturating_sub(keep)),
        CPhase::Stuck { .. } => balance.saturating_sub(keep).min(c.advanced),
        ref p => return Err(format!("refused: the conversion is {p:?}; only a funded or stuck one gives its ETH back")),
    };
    if wei == 0 {
        return Err("refused: nothing to give back".into());
    }
    let written_off = matches!(c.phase, CPhase::Stuck { .. }).then(|| c.last.clone()).flatten();
    c.phase = CPhase::Returning { wei };
    c.ask = None;
    c.last = None;
    if let Some(w) = written_off {
        let order = hex::encode(w.order_id);
        c.late = Some(rc::LateConv { watch: w, written_off_at: now, balance_after: balance.saturating_sub(wei), refund_wei: None,
            credited: None, filled: None, released_at: None });
        return Ok(format!("frog {frog}: {wei} {unit} goes back to the module at the next tick; order 0x{order} is written off and still \
            watched (C-03): its refund goes into the module, its fill is recorded; it holds the Relay leg until then or relay_late_release"));
    }
    Ok(format!("frog {frog}: {wei} {unit} goes back to the module at the next tick"))
}

/// Conversion review C-01: a frog whose conversion has arrived, whose buy failed (the listing vanished, a stale buy abandoned):
/// back to wanted, for at most the advance, so the arrived sats fund the next offer. The conversion stays `Arrived` and the room
/// reserved at the advance (H2) stays reserved. `None` when the frog has no arrived conversion (it fails as before).
pub fn rewant(st: &mut FrogState, frog: u64, now: u64, why: &str) -> Option<String> {
    let (advanced, sats) = funded(st, frog)?;
    let f = st.frogs.iter_mut().find(|f| f.id == frog)?;
    f.phase = FPhase::Wanted { cap_wei: advanced };
    let x = st.conv_mut(frog).expect("funded above");
    x.arrived_at = Some(now);
    x.note = format!("its buy failed ({why}): wanted again with the {sats} sats that arrived");
    Some(format!("{why}: its conversion has arrived, so the frog is wanted again for the {sats} sats (C-01); nothing of ours was lost"))
}

/// `relay_arrived_exit` (conversion review C-01): an arrival that has funded no buy for `ARRIVED_EXIT_SECS` is given up. What is at
/// our 4663 address (`balance` less the gas keep, at most the advance; possibly nothing) goes back with `returnAdvance` (the module
/// books any shortfall as its loss), the frog fails at the return, the leg and the room are freed, and the arrived sats stay in the
/// float, recorded on the conversion as `abandoned_sats`. Only while the frog is wanted (no buy out).
pub fn arrived_exit_now(st: &mut FrogState, frog: u64, balance: u128, now: u64) -> Result<String, String> {
    arrived_exit_now_keeping(st, frog, balance, now, rc::GAS_KEEP_WEI)
}

/// [`arrived_exit_now`] with the home's own gas keep (phase B2: lamports on Solana).
pub fn arrived_exit_now_keeping(st: &mut FrogState, frog: u64, balance: u128, now: u64, keep: u128) -> Result<String, String> {
    // audit A5-I3: the home's unit (the keep is the home's own: lamports on the Solana home)
    let unit = if keep == crate::solhome::SOL_GAS_KEEP_LAMPORTS { "lamports" } else { "wei" };
    let wanted = st.frogs.iter().any(|f| f.id == frog && matches!(f.phase, FPhase::Wanted { .. }));
    let c = st.conv_mut(frog).ok_or_else(|| format!("no conversion for frog {frog}"))?;
    let CPhase::Arrived { sats, .. } = c.phase else {
        return Err(format!("refused: the conversion is {:?}; only an arrived one is given up here", c.phase));
    };
    if !wanted {
        return Err(format!("refused: frog {frog} has a buy out (or is not wanted): its arrival is in use"));
    }
    let since = c.arrived_at.unwrap_or(now);
    if now < since.saturating_add(rc::ARRIVED_EXIT_SECS) {
        return Err(format!("refused: the arrival has funded no buy since {since}; it may be given up from {} ({} days)",
            since.saturating_add(rc::ARRIVED_EXIT_SECS), rc::ARRIVED_EXIT_SECS / 86_400));
    }
    let wei = balance.saturating_sub(keep).min(c.advanced);
    c.abandoned_sats = Some(sats);
    c.phase = CPhase::Returning { wei };
    c.note = format!("ARRIVAL GIVEN UP by the operator after {} days with no buy: {wei} {unit} of the {} advanced go back; {sats} sats stay \
        in the float, owed to module 0x{} in the books", now.saturating_sub(since) / 86_400, c.advanced, hex::encode(c.module));
    Ok(format!("frog {frog}: {}", c.note))
}

/// `relay_late_release` (conversion review C-03): the operator, after looking at Relay's own records, releases a written-off order's
/// hold on the Relay leg. A refund already seen is paid into the module first (refused until then).
pub fn late_release_now(st: &mut FrogState, frog: u64, now: u64) -> Result<String, String> {
    let c = st.conv_mut(frog).ok_or_else(|| format!("no conversion for frog {frog}"))?;
    let l = c.late.as_mut().filter(|l| l.open()).ok_or_else(|| format!("refused: frog {frog} has no written-off order still watched"))?;
    if l.refund_wei.is_some() {
        return Err("refused: its refund was seen and goes into the module first".into());
    }
    l.released_at = Some(now);
    let order = hex::encode(l.watch.order_id);
    c.note = format!("written-off order 0x{order} released by the operator at {now}: no longer watched; a refund of it landing later \
        is not attributed");
    Ok(format!("frog {frog}: {}", c.note))
}
