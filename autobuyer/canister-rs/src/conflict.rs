//! Conflict-out for leaked signed bytes (docs/v5/custody-hardening.md, H8, and its "Resolved before code, 2026-09-25" note;
//! docs/v5/custody-threat-model.md, T8).
//!
//! A delivery (or a refused deposit's return) is signed, saved and sent. If Bitcoin later stops showing it and custody decides
//! not to send it again, the signed bytes are still valid: anyone who kept them can broadcast them after the ticket refunds,
//! and the redeemer has the frog **and** the token. The only way to kill signed bytes is to spend one of their inputs first.
//! This module spends the leaked transaction's **fee input** back to custody, at a fee Bitcoin will accept as a replacement,
//! and never touches the item: no path here can send an item anywhere.
//!
//! One state machine for both records: `redeem::RPhase::Conflicting` and `deposits::DPhase::ReturnConflicting` each carry a
//! [`Conflict`] and step it through [`step`] with a small [`ConflictIo`] adapter over their own world. The pure half (the fee
//! rule and the builder) is tested here; the races, restarts and the pause in `tests/redeem.rs` and `tests/frogs.rs`.

use crate::custody::{self, Contents, Outpoint, Output, Utxo, DUST_P2TR};
use crate::looper::BtcView;
use crate::outbound::{self, BtcPolicy, Prevout, Purpose};
use crate::psbt::{self, Psbt, Tx, TxIn, TxOut};

/// Bitcoin Core's `-incrementalrelayfee` default, sat/vB: what a replacement must add per vbyte of its own (BIP125 rule 4).
/// Newer Core releases lower it; paying the older, higher figure is the safe side.
pub const INCREMENTAL_RELAY_SATS_PER_VB: u64 = 1;
/// A refused deposit's return that Bitcoin has not shown for this long is abandoned: conflicted out, then built again.
pub const RETURN_STALE_SECS: u64 = 72 * 3600;
/// The most a conflict may pay (audit R2, N-2): four times the delivery ceiling. A conflict pays nobody but custody (H4's
/// `ConflictOut`), so its fee is all that is at risk, and it is what keeps an item from leaving against a refunded ticket. It
/// must be able to replace a delivery bumped to its cap PLUS whatever child the redeemer hung on output 0 (BIP125 rule 3).
pub const MAX_CONFLICT_FEE_SATS: u64 = outbound::MAX_FEE_SATS * outbound::CONFLICT_FEE_FACTOR;
/// A conflict not confirmed this long after it was signed is signed again at a higher fee (audit R2, N-2 and M-1 (b)).
pub const CONFLICT_REPRICE_SECS: u64 = 2 * 3600;
/// R4-check R4-C1: how long "the explorers show our conflict in their mempools" is trusted to mean nothing outbids it there, from
/// the first time they showed it. Past this a conflict at today's rate that no block took is itself the evidence that the miners
/// do not hold it (a mempool partition: the explorers hold ours, the miners a pinned delivery), and the ×1.5 escalation resumes.
pub const HELD_TRUST_SECS: u64 = 24 * 3600;
/// Replace-by-fee signalled, no relative lock (as `custody::build`).
const SEQUENCE: u32 = 0xffff_fffd;
/// A P2TR scriptPubKey's length: the conflict's one output pays custody's.
const P2TR_LEN: usize = 34;

/// Our conflict transaction, once signed: saved before it is sent, and the same bytes until Bitcoin shows them.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, candid::CandidType)]
pub struct ConflictTx {
    /// Display order.
    pub txid: [u8; 32],
    pub raw: Vec<u8>,
    /// Every input it spends: the leaked fee input first, then at most one more custody UTXO.
    pub inputs: Vec<Outpoint>,
    pub fee: u64,
    /// When these bytes were signed: the re-pricing clock. `None` in a record saved before re-pricing existed (the clock then
    /// starts at the next look).
    #[serde(default)]
    pub signed_at: Option<u64>,
    /// Earlier, lower-fee versions of our conflict (display txids). Each pays only custody, so any of them confirming is the
    /// conflict confirming.
    #[serde(default)]
    pub earlier: Option<Vec<[u8; 32]>>,
}

impl ConflictTx {
    /// `t` is this conflict or an earlier version of it.
    pub fn is_ours(&self, t: &[u8; 32]) -> bool {
        self.txid == *t || self.earlier.as_ref().is_some_and(|e| e.contains(t))
    }
}

/// What a record in conflict keeps: the leaked transaction and, once built, ours.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, candid::CandidType)]
pub struct Conflict {
    /// The leaked transaction's txid, display order.
    pub leaked_txid: [u8; 32],
    /// The leaked transaction's signed bytes: the fee it paid is computed from them.
    pub raw: Vec<u8>,
    /// Its input 1: the one this conflict spends.
    pub fee_input: Outpoint,
    /// Its input 0, the item: never an input of the conflict.
    pub item: Outpoint,
    pub tx: Option<ConflictTx>,
    /// Earlier versions of the leaked transaction (display txids): the same inputs at a lower fee (audit D, M-1's re-pricing).
    /// Any of them confirming is the leaked transaction confirming. `None` in a record saved before they existed.
    #[serde(default)]
    pub earlier: Option<Vec<[u8; 32]>>,
    /// What the leaked transaction paid the miner, when its bytes alone cannot say (a float buy's, whose other inputs are the
    /// seller's: audit D, L-5). `None`: computed from the bytes and the fee input's value, as for custody's own two-input spends.
    #[serde(default)]
    pub leaked_fee: Option<u64>,
    /// R4-check R4-C1: when the explorers first showed a version of our conflict in their mempools. Never reset: the "held" rule
    /// (today's rate, no ×1.5) applies only for [`HELD_TRUST_SECS`] from it. `None` in a record saved before it existed, or while
    /// no version has been seen.
    #[serde(default)]
    pub held_since: Option<u64>,
}

fn display_outpoint(i: &outbound::TxIn) -> Outpoint {
    let mut t = i.txid;
    t.reverse();
    Outpoint { txid: t, vout: i.vout }
}

impl Conflict {
    /// The record for saved bytes `raw` whose txid is `leaked_txid` (display order) and whose fee input is `fee_input`. Refuses
    /// bytes that are not a two-input custody spend with that fee input as input 1: our own saved bytes always are.
    pub fn of(leaked_txid: [u8; 32], raw: &[u8], fee_input: Outpoint) -> Result<Conflict, String> {
        let t = outbound::parse_tx(raw)?;
        let computed = crate::btcsend::parse_signed(raw)?.txid;
        let mut shown = computed;
        shown.reverse();
        if shown != leaked_txid {
            return Err("the saved bytes are not the transaction the record names".into());
        }
        if t.inputs.len() != 2 || display_outpoint(&t.inputs[1]) != fee_input {
            return Err("the saved bytes do not spend the record's fee input as their input 1".into());
        }
        Ok(Conflict { leaked_txid, raw: raw.to_vec(), fee_input, item: display_outpoint(&t.inputs[0]), tx: None, earlier: None,
            leaked_fee: None, held_since: None })
    }

    /// The record for a float buy's saved bytes (audit D, L-5): `ours` is one of OUR inputs of it (the one spent back to the
    /// float), `item` its seller's input (never an input of the conflict, and not ours to spend anyway), and `fee` what the buy
    /// pays the miner (the judge's figure, saved with the buy).
    pub fn of_input(leaked_txid: [u8; 32], raw: &[u8], ours: Outpoint, item: Outpoint, fee: u64) -> Result<Conflict, String> {
        let t = outbound::parse_tx(raw)?;
        let mut shown = crate::btcsend::parse_signed(raw)?.txid;
        shown.reverse();
        if shown != leaked_txid {
            return Err("the saved bytes are not the transaction the record names".into());
        }
        let ins: Vec<Outpoint> = t.inputs.iter().map(display_outpoint).collect();
        if !ins.contains(&ours) || !ins.contains(&item) || ours == item {
            return Err("the saved bytes do not spend both the input named ours and the seller's".into());
        }
        Ok(Conflict { leaked_txid, raw: raw.to_vec(), fee_input: ours, item, tx: None, earlier: None, leaked_fee: Some(fee),
            held_since: None })
    }

    /// The record for signed bytes the canister never holds whole (a protected Ordinals Wallet sale, docs/v8/ow-protected.md
    /// §3.3: Ordinals Wallet adds its co-signature and broadcasts). `ours` is the float input named to be spent back to the float
    /// (the caller has checked it is the float's), `item` the input that is never a conflict's (the escrow), `fee` what the
    /// transactions this conflict must replace pay the miner together. `earlier`: other transactions spending `ours` whose
    /// confirmation the caller decides about (a protected sale's setup), which the conflict may replace while they are unconfirmed.
    /// `raw` is empty: the leaked fee is always `fee`.
    pub fn of_named(leaked_txid: [u8; 32], ours: Outpoint, item: Outpoint, fee: u64, earlier: Option<Vec<[u8; 32]>>)
        -> Result<Conflict, String> {
        if ours == item {
            return Err("the input named ours is the item".into());
        }
        Ok(Conflict { leaked_txid, raw: vec![], fee_input: ours, item, tx: None, earlier, leaked_fee: Some(fee), held_since: None })
    }

    /// `t` is an earlier version of the leaked transaction.
    pub fn is_earlier(&self, t: &[u8; 32]) -> bool {
        self.earlier.as_ref().is_some_and(|e| e.contains(t))
    }

    /// Every custody output this conflict has claimed: no other delivery, return or move may choose them.
    pub fn reserved(&self) -> Vec<Outpoint> {
        let mut v = vec![self.fee_input];
        for i in self.tx.iter().flat_map(|t| t.inputs.iter()) {
            if !v.contains(i) {
                v.push(*i);
            }
        }
        v
    }
}

// ---- the pure half ------------------------------------------------------------------------------------------------------------

/// The fee the leaked transaction pays: its fee input's value less everything but output 0. Output 0 carries the item's whole
/// value (`outbound::verify_btc` checked that before the bytes were saved), so the item's value cancels out.
pub fn leaked_fee(raw: &[u8], fee_input_value: u64) -> Result<u64, String> {
    let t = outbound::parse_tx(raw)?;
    let rest = t.outputs.iter().skip(1).try_fold(0u64, |s, o| s.checked_add(o.value)).ok_or("the leaked outputs overflow")?;
    fee_input_value.checked_sub(rest).ok_or_else(|| format!("the explorers value the fee input at {fee_input_value} sats, less than the \
        {rest} the leaked bytes pay back to custody: nothing is believed"))
}

/// The conflict's fee with `inputs` inputs and one P2TR output: at least the leaked fee plus its own vsize at the incremental
/// relay rate (BIP125 rules 3 and 4), and at least its vsize at the current rate.
pub fn fee_for(inputs: usize, leaked_fee: u64, rate: u64) -> u64 {
    let vs = custody::vsize(inputs, &[P2TR_LEN]);
    leaked_fee.saturating_add(vs * INCREMENTAL_RELAY_SATS_PER_VB).max(vs * custody::bound_rate(rate))
}

/// The value of the one output when `value_in` pays `fee` and leaves at least dust; `None` otherwise.
pub fn output_value(value_in: u64, fee: u64) -> Option<u64> {
    value_in.checked_sub(fee).filter(|v| *v >= DUST_P2TR)
}

/// The conflict: every input to one output paying custody, `fee` to the miner.
pub fn build(inputs: &[(Outpoint, u64)], custody_script: &[u8], fee: u64) -> Result<Psbt, String> {
    if custody_script.len() != P2TR_LEN || custody_script[0] != 0x51 || custody_script[1] != 0x20 {
        return Err("custody's script is not a taproot script".into());
    }
    if inputs.is_empty() || inputs.len() > 2 {
        return Err("a conflict spends the fee input and at most one more".into());
    }
    if fee > MAX_CONFLICT_FEE_SATS {
        return Err(format!("a conflict fee of {fee} sats is over the {MAX_CONFLICT_FEE_SATS}-sat ceiling: refused rather than paid"));
    }
    let total = inputs.iter().try_fold(0u64, |s, (_, v)| s.checked_add(*v)).ok_or("the inputs overflow")?;
    let value = output_value(total, fee).ok_or_else(|| format!("{total} sats cannot pay {fee} and keep a {DUST_P2TR}-sat output"))?;
    Ok(Psbt {
        tx: Tx {
            version: 2,
            inputs: inputs.iter().map(|(o, _)| TxIn { txid: o.internal_txid(), vout: o.vout, sequence: SEQUENCE }).collect(),
            outputs: vec![TxOut { value, script: custody_script.to_vec() }],
            locktime: 0,
        },
        witness_utxos: inputs.iter().map(|(_, v)| Some(TxOut { value: *v, script: custody_script.to_vec() })).collect(),
    })
}

// ---- the world, and one step ----------------------------------------------------------------------------------------------------

/// What the conflict needs of the world. `redeem` and `frogflow` each adapt their own `Io` to it; `save` writes the conflict
/// into its record and must be durable before the next `await` returns.
#[allow(async_fn_in_trait)]
pub trait ConflictIo {
    /// Whether the conflict must sign and send nothing now. For a stale buy's or a return's conflict, the buyer's pause (H1). A
    /// delivery's conflict runs while paused (audit R2, M-1 (c)): it pays only custody and can only stop an item leaving.
    fn paused(&self) -> bool;
    /// Seconds, for the re-pricing clock.
    fn now(&self) -> u64;
    /// `txid` in display order.
    async fn seen(&mut self, txid: [u8; 32]) -> BtcView;
    /// The output as every explorer serves it, agreeing, and whether it is spent.
    async fn output(&mut self, o: &Outpoint) -> Result<(Output, bool), String>;
    /// Who spent an output (display txid) and whether that is confirmed; `None` unspent.
    async fn outspend(&mut self, o: &Outpoint) -> Result<Option<([u8; 32], bool)>, String>;
    /// Custody's confirmed UTXOs.
    async fn utxos(&mut self) -> Result<Vec<Utxo>, String>;
    async fn contents(&mut self, o: &Outpoint) -> Result<Option<Contents>, String>;
    async fn fee_rate(&mut self) -> Result<u64, String>;
    async fn custody_script(&mut self) -> Result<Vec<u8>, String>;
    /// A signature with the CUSTODY key, never the buyer's.
    async fn sign_custody(&mut self, sighash: [u8; 32]) -> Result<Vec<u8>, String>;
    /// Send; the answer is not believed.
    async fn send(&mut self, raw: &[u8]);
    fn policy(&self, custody_script: &[u8]) -> BtcPolicy;
    fn save(&mut self, c: &Conflict);
}

/// How a conflict ended. In every case the leaked bytes can no longer be mined, or they were.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Settled {
    /// The leaked transaction confirmed: it won the race. The record goes where a confirmed send goes.
    LeakedConfirmed,
    /// An earlier, lower-fee version of the leaked transaction confirmed (display txid): the same outcome as `LeakedConfirmed`.
    EarlierConfirmed([u8; 32]),
    /// Our conflict confirmed (display txid).
    ConflictConfirmed([u8; 32]),
    /// The fee input was spent by a third transaction, confirmed (display txid): the leaked bytes are dead all the same.
    SpentElsewhere([u8; 32]),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stepped {
    /// Still in conflict; the line says why.
    Waits(String),
    Settled(Settled),
    /// Something that cannot be: the verifier refused our conflict, or the explorers contradict our own saved bytes. Loud.
    Stuck(String),
}

/// One pass over a conflict. `Err` is a wait with its reason; `reserved` are custody outputs other records have claimed.
pub async fn step<I: ConflictIo>(io: &mut I, c: &mut Conflict, reserved: &[Outpoint]) -> Result<Stepped, String> {
    if io.paused() {
        return Err("the buyer is paused: the conflict-out signs and sends nothing until k guardians resume it".into());
    }
    // (a) the leaked transaction won
    match io.seen(c.leaked_txid).await {
        BtcView::Confirmed => return Ok(Stepped::Settled(Settled::LeakedConfirmed)),
        BtcView::Unsure(e) => return Err(e),
        BtcView::Mempool | BtcView::NotYet => {}
    }
    if let Some(mut tx) = c.tx.clone() {
        // a record saved before re-pricing existed: its clock starts now
        if tx.signed_at.is_none() {
            tx.signed_at = Some(io.now());
            c.tx = Some(tx.clone());
            io.save(c);
        }
        let due = tx.signed_at.is_some_and(|t| io.now().saturating_sub(t) >= CONFLICT_REPRICE_SECS);
        // (b) ours won
        let seen = io.seen(tx.txid).await;
        if matches!(seen, BtcView::Mempool) && c.held_since.is_none() {
            // R4-C1: the first time the explorers show our conflict held: the clock of the "held" rule starts
            c.held_since = Some(io.now());
            io.save(c);
        }
        let held = match seen {
            BtcView::Confirmed => return Ok(Stepped::Settled(Settled::ConflictConfirmed(tx.txid))),
            BtcView::Mempool if !due => {
                return Ok(Stepped::Waits(format!("conflict {} is in a mempool; waiting for a block", hex::encode(tx.txid))));
            }
            BtcView::Unsure(e) => return Err(e),
            // R4-C1: trusted only for HELD_TRUST_SECS from the first sighting; after that, escalated as if not seen
            BtcView::Mempool => c.held_since.is_none_or(|t| io.now().saturating_sub(t) < HELD_TRUST_SECS),
            BtcView::NotYet => false,
        };
        // (c) the fee input gone in a block: to a version of ours, an earlier version of the leaked bytes, or a third transaction;
        // or our extra input gone, which makes ours unminable: build again
        if let Some((t, true)) = io.outspend(&c.fee_input).await? {
            if c.is_earlier(&t) {
                return Ok(Stepped::Settled(Settled::EarlierConfirmed(t)));
            }
            if tx.is_ours(&t) {
                return Ok(Stepped::Settled(Settled::ConflictConfirmed(t)));
            }
            if t != c.leaked_txid {
                return Ok(Stepped::Settled(Settled::SpentElsewhere(t)));
            }
        }
        for extra in tx.inputs.iter().filter(|i| **i != c.fee_input) {
            if let Some((t, true)) = io.outspend(extra).await? {
                if !tx.is_ours(&t) {
                    c.tx = None;
                    io.save(c);
                    return Err(format!("our conflict's second input {} was spent by {}: it is built again", extra.text(), hex::encode(t)));
                }
            }
        }
        // (d) not confirmed after CONFLICT_REPRICE_SECS: signed again at a higher fee (audit R2, N-2: a pinned or under-priced
        // delivery is replaced only by paying more), up to MAX_CONFLICT_FEE_SATS
        if due {
            return reprice(io, c, reserved, tx, held).await;
        }
        // (e) the same saved bytes again
        io.send(&tx.raw).await;
        return Ok(Stepped::Waits(format!("conflict {} sent again; Bitcoin does not show it yet", hex::encode(tx.txid))));
    }
    build_sign_save_send(io, c, reserved).await
}

async fn build_sign_save_send<I: ConflictIo>(io: &mut I, c: &mut Conflict, reserved: &[Outpoint]) -> Result<Stepped, String> {
    let custody_script = io.custody_script().await?;
    let (out, spent) = io.output(&c.fee_input).await?;
    if out.script != custody_script {
        return Ok(Stepped::Stuck(format!("the explorers say fee input {} is not custody's", c.fee_input.text())));
    }
    if spent {
        match io.outspend(&c.fee_input).await? {
            // the leaked bytes are in a mempool (not confirmed: (a) said so): ours replaces them
            Some((t, _)) if t == c.leaked_txid => {}
            Some((t, true)) if c.is_earlier(&t) => return Ok(Stepped::Settled(Settled::EarlierConfirmed(t))),
            // an earlier version in a mempool: ours pays more than the latest, so it replaces that too
            Some((t, false)) if c.is_earlier(&t) => {}
            Some((t, true)) => return Ok(Stepped::Settled(Settled::SpentElsewhere(t))),
            Some((t, false)) => return Err(format!("fee input {} is spent by {}, unconfirmed, which is neither the leaked bytes nor ours",
                c.fee_input.text(), hex::encode(t))),
            None => {} // the explorers moved between two reads; Bitcoin decides
        }
    }
    let leaked = match c.leaked_fee.map_or_else(|| leaked_fee(&c.raw, out.value), Ok) {
        Ok(f) => f,
        Err(e) => return Ok(Stepped::Stuck(e)),
    };
    let rate = io.fee_rate().await?;
    let mut inputs = vec![(c.fee_input, out.value)];
    let one = fee_for(1, leaked, rate);
    let fee = if output_value(out.value, one).is_some() {
        one
    } else {
        let two = fee_for(2, leaked, rate);
        let need = (two + DUST_P2TR).saturating_sub(out.value).max(1);
        let extra = pick_extra(io, c, reserved, need).await?
            .ok_or("no clean confirmed custody UTXO can fund the conflict-out with the fee input (the operator tops custody up)")?;
        inputs.push((extra.outpoint, extra.value));
        two
    };
    sign_save_send(io, c, &inputs, fee, &custody_script, None, format!("the leaked {} paid {leaked}", hex::encode(c.leaked_txid))).await
}

/// Audit R2, N-2 (M-1 (b)): our conflict, not confirmed after [`CONFLICT_REPRICE_SECS`], signed again over the same inputs at
/// `max(1.5 × old, old + its size at the incremental rate + 1, today's rate × its size)`, at most [`MAX_CONFLICT_FEE_SATS`] and
/// at most what its inputs can pay (one more clean custody UTXO is added when the fee input alone cannot). A version that cannot
/// pay more is sent again as it is.
///
/// `held`: our conflict is in a mempool, so it already holds the fee input there and nothing is outbidding it (audit R3, R3-C7).
/// It then only follows the market: re-signed when today's rate × its size is above its fee, to that (at least its fee plus the
/// incremental relay fee), never ×1.5. The escalation is for a conflict that is not seen (the pinned delivery, or a package that
/// outbid ours, holds the mempool), and, since R4-check R4-C1, for one seen held for [`HELD_TRUST_SECS`] or longer: the explorers
/// are not the miners, and a conflict at today's rate that no block takes for a day is not held where it matters.
async fn reprice<I: ConflictIo>(io: &mut I, c: &mut Conflict, reserved: &[Outpoint], prior: ConflictTx, held: bool)
    -> Result<Stepped, String> {
    let custody_script = io.custody_script().await?;
    let mut inputs = Vec::new();
    for o in &prior.inputs {
        let (out, _) = io.output(o).await?;
        if out.script != custody_script || !out.confirmed {
            return Ok(Stepped::Stuck(format!("the explorers say our conflict's input {} is not a confirmed custody output", o.text())));
        }
        inputs.push((*o, out.value));
    }
    let rate = custody::bound_rate(io.fee_rate().await?);
    if held && custody::vsize(inputs.len(), &[P2TR_LEN]) * rate <= prior.fee {
        // in a mempool and paying today's rate: nothing to outbid, so not re-priced; looked at again in CONFLICT_REPRICE_SECS
        if let Some(t) = c.tx.as_mut() {
            t.signed_at = Some(io.now());
        }
        io.save(c);
        io.send(&prior.raw).await;
        return Ok(Stepped::Waits(format!("conflict {} is in a mempool at today's rate ({} sats): not re-priced, waiting for a block",
            hex::encode(prior.txid), prior.fee)));
    }
    let want = |n: usize| {
        let vs = custody::vsize(n, &[P2TR_LEN]);
        let bip125 = prior.fee + vs * INCREMENTAL_RELAY_SATS_PER_VB + 1;
        let escalate = if held { 0 } else { prior.fee * 3 / 2 };
        escalate.max(bip125).max(vs * rate).min(MAX_CONFLICT_FEE_SATS)
    };
    let mut fee = want(inputs.len());
    let total = |i: &[(Outpoint, u64)]| i.iter().map(|(_, v)| *v).sum::<u64>();
    if output_value(total(&inputs), fee).is_none() && inputs.len() == 1 {
        let two = want(2);
        let need = (two + DUST_P2TR).saturating_sub(total(&inputs)).max(1);
        if let Some(extra) = pick_extra(io, c, reserved, need).await? {
            inputs.push((extra.outpoint, extra.value));
            fee = two;
        }
    }
    // what the inputs can pay, keeping the output above dust
    fee = fee.min(total(&inputs).saturating_sub(DUST_P2TR));
    let floor = prior.fee + custody::vsize(inputs.len(), &[P2TR_LEN]) * INCREMENTAL_RELAY_SATS_PER_VB;
    if fee <= floor {
        // looked at again in another CONFLICT_REPRICE_SECS, not every tick
        if let Some(t) = c.tx.as_mut() {
            t.signed_at = Some(io.now());
        }
        io.save(c);
        io.send(&prior.raw).await;
        return Ok(Stepped::Waits(format!("conflict {} not confirmed; it cannot pay more ({} sats; ceiling {MAX_CONFLICT_FEE_SATS}, \
            inputs {} sats): the same bytes sent again", hex::encode(prior.txid), prior.fee, total(&inputs))));
    }
    let mut earlier = prior.earlier.clone().unwrap_or_default();
    earlier.push(prior.txid);
    sign_save_send(io, c, &inputs, fee, &custody_script, Some(earlier), format!("re-priced from {} sats", prior.fee)).await
}

/// Sign the conflict over `inputs` paying `fee`, check it (H4), and only then save and send it.
async fn sign_save_send<I: ConflictIo>(io: &mut I, c: &mut Conflict, inputs: &[(Outpoint, u64)], fee: u64, custody_script: &[u8],
    earlier: Option<Vec<[u8; 32]>>, note: String) -> Result<Stepped, String> {
    let p = match build(inputs, custody_script, fee) {
        Ok(p) => p,
        Err(e) => return Ok(Stepped::Stuck(format!("the conflict cannot be built: {e}"))),
    };
    let mut sigs = Vec::new();
    for i in 0..p.tx.inputs.len() {
        let h = crate::sighash::taproot_key_path(&p, i).map_err(|e| format!("digest: {e:?}"))?;
        sigs.push((i, io.sign_custody(h).await?));
    }
    let raw = psbt::finalize(&p, &vec![None; p.tx.inputs.len()], &sigs)?;
    // H4: the signed bytes spend the leaked fee input and pay nobody but custody, checked apart from the builder
    let pre: Vec<Prevout> = inputs.iter().map(|(o, v)| Prevout { txid: o.internal_txid(), vout: o.vout, value: *v,
        script: custody_script.to_vec() }).collect();
    if let Err(e) = outbound::verify_btc(&raw, &pre, &Purpose::ConflictOut { spend: pre[0].clone() }, &io.policy(custody_script)) {
        return Ok(Stepped::Stuck(format!("the outbound check refused the signed conflict: {e:?}; nothing was saved or sent")));
    }
    // H1: signing awaited the subnet, and a guardian may have paused meanwhile: nothing is saved or sent then
    if io.paused() {
        return Err("paused while the conflict was being signed: nothing saved or sent".into());
    }
    let tx = ConflictTx { txid: p.txid(), raw, inputs: inputs.iter().map(|(o, _)| *o).collect(), fee, signed_at: Some(io.now()),
        earlier };
    c.tx = Some(tx.clone());
    io.save(c);
    io.send(&tx.raw).await;
    Ok(Stepped::Waits(format!("conflict {} signed, saved and sent: fee input {} back to custody at {fee} sats ({note})",
        hex::encode(tx.txid), c.fee_input.text())))
}

/// One more clean custody UTXO worth at least `need`: confirmed, not the item, not the fee input, not reserved, and ord shows
/// it carrying nothing (the same test a delivery's fee input passes).
async fn pick_extra<I: ConflictIo>(io: &mut I, c: &Conflict, reserved: &[Outpoint], need: u64) -> Result<Option<Utxo>, String> {
    let mut cands: Vec<Utxo> = io.utxos().await?.into_iter().filter(|u| u.confirmed && u.outpoint != c.item
        && u.outpoint != c.fee_input && !reserved.contains(&u.outpoint) && u.value >= need).collect();
    cands.sort_by_key(|u| (u.value, u.outpoint));
    cands.truncate(custody::FEE_CANDIDATES);
    let script = io.custody_script().await?;
    for u in cands {
        // a candidate is a claim (the canister's fee pool, or a listing): every explorer must show it ours, confirmed, unspent.
        // One that cannot be read is skipped, never the end of the choice (audit R2, N-4)
        let Ok((out, spent)) = io.output(&u.outpoint).await else { continue };
        if spent || !out.confirmed || out.script != script || out.value != u.value {
            continue;
        }
        let Ok(ct) = io.contents(&u.outpoint).await else { continue };
        if custody::fee_eligible(&u, ct.as_ref(), &c.item, reserved) {
            return Ok(Some(u));
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// R4-check R4-C1: a conflict saved before `held_since` existed decodes with `None` (candid and serde), so its "held" clock
    /// starts at the next sighting; the new field round-trips.
    #[test]
    fn a_conflict_saved_before_held_since_decodes_and_the_field_round_trips() {
        #[derive(candid::CandidType, serde::Serialize)]
        struct Old { leaked_txid: [u8; 32], raw: Vec<u8>, fee_input: Outpoint, item: Outpoint, tx: Option<ConflictTx>,
            earlier: Option<Vec<[u8; 32]>>, leaked_fee: Option<u64> }
        let o = Outpoint { txid: [3; 32], vout: 1 };
        let old = Old { leaked_txid: [1; 32], raw: vec![2], fee_input: o, item: o, tx: None, earlier: None, leaked_fee: Some(9) };
        let back: Conflict = candid::decode_one(&candid::encode_one(&old).unwrap()).expect("candid: an older record decodes");
        assert_eq!((back.held_since, back.leaked_fee), (None, Some(9)));
        let back: Conflict = serde_json::from_str(&serde_json::to_string(&old).unwrap()).expect("serde: an older record decodes");
        assert_eq!(back.held_since, None);
        let now = Conflict { held_since: Some(1_790_000_000), ..back };
        let again: Conflict = candid::decode_one(&candid::encode_one(&now).unwrap()).unwrap();
        assert_eq!(again, now);
    }

    fn p2tr(b: u8) -> Vec<u8> {
        [vec![0x51, 0x20], vec![b; 32]].concat()
    }
    fn op(tag: u8, vout: u32) -> Outpoint {
        Outpoint { txid: [tag; 32], vout }
    }

    /// A delivery as `custody::build` makes it, signed with a dummy signature.
    fn leaked(fee_value: u64, rate: u64) -> (Vec<u8>, [u8; 32]) {
        let frog = custody::Frog { output: op(0xf0, 0), value: 546 };
        let fee = Utxo { outpoint: op(0xfe, 1), value: fee_value, confirmed: true };
        let p = custody::build(&frog, &fee, &p2tr(0xde), &p2tr(1), rate).unwrap();
        let raw = psbt::finalize(&p, &[None, None], &[(0, vec![0x5a; 64]), (1, vec![0x5a; 64])]).unwrap();
        (raw, p.txid())
    }

    #[test]
    fn the_leaked_fee_is_read_from_the_bytes_and_the_fee_inputs_value() {
        let (raw, _) = leaked(20_000, 3);
        assert_eq!(leaked_fee(&raw, 20_000), Ok(3 * 212), "20,000 in, change back: the fee is what custody::build charged");
        // no change: the whole fee input was the fee
        let (raw, _) = leaked(700, 3);
        assert_eq!(parse(&raw).outputs.len(), 1);
        assert_eq!(leaked_fee(&raw, 700), Ok(700));
        // explorers valuing the input below the change the bytes pay: not believed
        let (raw, _) = leaked(20_000, 3);
        assert!(leaked_fee(&raw, 100).unwrap_err().contains("nothing is believed"));
    }

    fn parse(raw: &[u8]) -> outbound::Tx {
        outbound::parse_tx(raw).unwrap()
    }

    #[test]
    fn the_record_is_made_only_from_our_own_two_input_bytes() {
        let (raw, txid) = leaked(20_000, 3);
        let c = Conflict::of(txid, &raw, op(0xfe, 1)).unwrap();
        assert_eq!((c.item, c.fee_input, c.tx.clone()), (op(0xf0, 0), op(0xfe, 1), None));
        assert_eq!(c.reserved(), vec![op(0xfe, 1)]);
        assert!(Conflict::of(txid, &raw, op(0xfe, 2)).is_err(), "another fee input");
        assert!(Conflict::of([0; 32], &raw, op(0xfe, 1)).is_err(), "another txid");
        let with_tx = Conflict { tx: Some(ConflictTx { txid: [9; 32], raw: vec![], inputs: vec![op(0xfe, 1), op(0xaa, 0)], fee: 1,
            signed_at: None, earlier: None }), ..c };
        assert_eq!(with_tx.reserved(), vec![op(0xfe, 1), op(0xaa, 0)]);
    }

    #[test]
    fn the_fee_replaces_the_leaked_one_under_bip125_and_pays_the_current_rate() {
        // one input, one P2TR output: 111 vB
        assert_eq!(custody::vsize(1, &[34]), 111);
        // the leaked fee plus 111 × 1 sat/vB when the rate is low
        assert_eq!(fee_for(1, 636, 3), 636 + 111);
        // the current rate when it is higher
        assert_eq!(fee_for(1, 636, 50), 111 * 50);
        assert!(fee_for(1, 636, 50) > 636 + 111);
        // a rate is bounded as everywhere else
        assert_eq!(fee_for(1, 0, 100_000), 111 * custody::MAX_FEE_RATE);
        // the conflict's feerate is always above the leaked delivery's (212 vB at the leaked rate)
        for rate in [1, 3, 20, 100] {
            let leaked = 212 * rate;
            let f = fee_for(1, leaked, rate);
            assert!(f * 212 > leaked * 111, "feerate {f}/111 must beat {leaked}/212");
            assert!(f >= leaked + 111, "BIP125 rule 4");
        }
        // within the rate bound the ceiling never binds, with two inputs and no change on the leaked side either
        let worst_leaked = 212 * custody::MAX_FEE_RATE + DUST_P2TR;
        assert!(fee_for(2, worst_leaked, custody::MAX_FEE_RATE) <= custody::MAX_DELIVERY_FEE_SATS);
        // the conflict ceiling is four times the delivery's (audit R2, N-2)
        assert_eq!(MAX_CONFLICT_FEE_SATS, 4 * custody::MAX_DELIVERY_FEE_SATS);
    }

    #[test]
    fn the_conflict_pays_only_custody_and_passes_the_outbound_check() {
        let custody = p2tr(1);
        let (raw_leaked, _) = leaked(20_000, 3);
        let fee = fee_for(1, leaked_fee(&raw_leaked, 20_000).unwrap(), 3);
        let p = build(&[(op(0xfe, 1), 20_000)], &custody, fee).unwrap();
        assert_eq!((p.tx.inputs.len(), p.tx.outputs.len()), (1, 1));
        assert_eq!(p.tx.outputs[0], TxOut { value: 20_000 - fee, script: custody.clone() });
        assert_eq!(p.tx.inputs[0].sequence, 0xffff_fffd, "replaceable, like every custody spend");
        let raw = psbt::finalize(&p, &[None], &[(0, vec![0x5a; 64])]).unwrap();
        let pre = vec![Prevout { txid: op(0xfe, 1).internal_txid(), vout: 1, value: 20_000, script: custody.clone() }];
        let pol = BtcPolicy { custody_script: custody.clone(), max_fee_sats: outbound::MAX_FEE_SATS, require_finalized: true, require_event: true };
        let checked = outbound::verify_btc(&raw, &pre, &Purpose::ConflictOut { spend: pre[0].clone() }, &pol).unwrap();
        assert_eq!(checked.fee, fee);
        // it spends the leaked delivery's fee input: the two cannot both be mined
        let leaked_ins = parse(&raw_leaked).inputs;
        assert_eq!((leaked_ins[1].txid, leaked_ins[1].vout), (parse(&raw).inputs[0].txid, parse(&raw).inputs[0].vout));
        // two inputs
        let p2 = build(&[(op(0xfe, 1), 700), (op(0xaa, 0), 5_000)], &custody, 900).unwrap();
        assert_eq!(p2.tx.outputs[0].value, 4_800);
        // refusals
        assert!(build(&[(op(0xfe, 1), 700)], &custody, 500).unwrap_err().contains("keep a 330-sat output"));
        assert!(build(&[(op(0xfe, 1), 200_000)], &custody, 25_001).is_ok(), "a conflict may pay past the delivery ceiling");
        assert!(build(&[(op(0xfe, 1), 200_000)], &custody, MAX_CONFLICT_FEE_SATS + 1).unwrap_err().contains("ceiling"));
        assert!(build(&[], &custody, 1).is_err());
        assert!(build(&[(op(1, 0), 9_000), (op(2, 0), 9_000), (op(3, 0), 9_000)], &custody, 1_000).is_err());
        assert!(build(&[(op(0xfe, 1), 20_000)], &[0x00, 0x14, 1], 1_000).is_err());
        assert_eq!(output_value(1_000, 670), Some(330));
        assert_eq!(output_value(1_000, 671), None);
    }
}
