//! Custody of the wrapper's frogs, and the transaction that delivers one (docs/v3/wrapper.md, "Step 3, the flow").
//!
//! Pure: no network, no keys, no clock. It reads what ord and the explorers said (as the canister's transforms hand them
//! over), decides whether a frog is where it can safely be sent from, picks a plain UTXO to pay the fee, and builds the
//! transaction. Every parser is tested on replies captured from the real services (`tests/fixtures/delivery/`).
//!
//! The one property the builder exists for: output 0 takes **exactly** the frog's output's value, so under ordinal theory's
//! first-in-first-out assignment every sat of input 0 — the inscribed one wherever it sits — lands on output 0, and no sat
//! of any other input does.

use crate::psbt::{Psbt, Tx, TxIn, TxOut};

/// Bitcoin Core's dust limit for a P2TR output: nothing below it is relayed.
pub const DUST_P2TR: u64 = 330;
/// Fee-rate bounds, sat/vB. The explorers' estimate is a report; these keep a wrong one from mattering.
pub const MIN_FEE_RATE: u64 = 1;
pub const MAX_FEE_RATE: u64 = 100;
/// A delivery whose fee would exceed this is refused rather than paid.
pub const MAX_DELIVERY_FEE_SATS: u64 = 25_000;
/// How many of our UTXOs are looked at on ord, per attempt, for a clean one to pay the fee.
pub const FEE_CANDIDATES: usize = 10;
/// A fee input is never smaller than this (audit D, H-4): a stranger's dust sent to the public custody address is never a
/// candidate, so it is never read, chosen or spent.
pub const MIN_FEE_INPUT_SATS: u64 = 5_000;
/// Replace-by-fee signalled, no relative lock.
const SEQUENCE: u32 = 0xffff_fffd;
/// Every item custody sends (a delivery, its bumps, a refused deposit's return) is a TRUC transaction (BIP 431, `nVersion` 3;
/// audit R2, N-2). While it is unconfirmed, TRUC policy lets its outputs be spent only by one TRUC child of at most 1,000 vB, so
/// the redeemer, who owns output 0, cannot hang the ≈ 100 kvB low-feerate child that pinned a version-2 delivery against every
/// bump and the conflict-out: a child that makes replacing it expensive must pay a feerate that gets the delivery mined.
pub const TRUC_VERSION: i32 = 3;

/// An output, by its transaction id in **display** order (as an explorer shows it) and its index.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize, candid::CandidType)]
pub struct Outpoint {
    pub txid: [u8; 32],
    pub vout: u32,
}

impl Outpoint {
    /// `<64 hex>:<vout>`.
    pub fn parse(s: &str) -> Result<Self, String> {
        let (t, v) = s.split_once(':').ok_or_else(|| format!("an outpoint is txid:vout, got {s:?}"))?;
        Ok(Outpoint { txid: txid_of(t)?, vout: v.parse().map_err(|_| format!("bad vout in {s:?}"))? })
    }
    pub fn text(&self) -> String {
        format!("{}:{}", hex::encode(self.txid), self.vout)
    }
    /// The txid as a transaction input carries it: internal byte order.
    pub fn internal_txid(&self) -> [u8; 32] {
        let mut t = self.txid;
        t.reverse();
        t
    }
}

fn txid_of(t: &str) -> Result<[u8; 32], String> {
    let b = hex::decode(t).map_err(|_| format!("a txid is 64 hex characters, got {t:?}"))?;
    b.try_into().map_err(|_| format!("a txid is 32 bytes, got {t:?}"))
}

/// `<64 hex>i<n>`: the id an inscription is known by. Anything else is refused before it reaches a URL.
pub fn check_inscription_id(id: &str) -> Result<(), String> {
    let (t, n) = id.split_once('i').ok_or_else(|| format!("an inscription id is <txid>i<n>, got {id:?}"))?;
    txid_of(t)?;
    if n.is_empty() || n.len() > 10 || !n.bytes().all(|c| c.is_ascii_digit()) {
        return Err(format!("an inscription id is <txid>i<n>, got {id:?}"));
    }
    Ok(())
}

// ---- what ord says -------------------------------------------------------------------------------------------------------

/// Where ord says an inscription is now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Location {
    pub output: Outpoint,
    /// The inscribed sat's offset within that output.
    pub offset: u64,
    /// The output's value, as ord reports it.
    pub value: u64,
}

/// ord's `/r/inscription/<id>`. `Ok(None)`: ord does not have it (yet).
pub fn parse_ord_inscription(status: u16, body: &[u8], id: &str) -> Result<Option<Location>, String> {
    if status == 404 {
        return Ok(None);
    }
    if status != 200 {
        return Err(format!("ord: HTTP {status}"));
    }
    let v: serde_json::Value = serde_json::from_slice(body).map_err(|_| "ord: not JSON")?;
    if v.get("id").and_then(|x| x.as_str()) != Some(id) {
        return Err("ord: the answer is about a different inscription".into());
    }
    let sp = v.get("satpoint").and_then(|x| x.as_str()).ok_or("ord: no satpoint")?;
    let (out, off) = sp.rsplit_once(':').ok_or("ord: a satpoint is txid:vout:offset")?;
    let output = Outpoint::parse(out)?;
    let offset: u64 = off.parse().map_err(|_| "ord: an unreadable offset")?;
    let value = v.get("value").and_then(|x| x.as_u64()).ok_or("ord: no value")?;
    if v.get("output").and_then(|x| x.as_str()) != Some(out) {
        return Err("ord: the satpoint and the output disagree".into());
    }
    if offset >= value {
        return Err("ord: the inscribed sat's offset is past the output's value".into());
    }
    Ok(Some(Location { output, offset, value }))
}

/// Everything ord says is on one output.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Contents {
    pub inscriptions: Vec<String>,
    /// Rune names only: any rune on the output is enough to keep it away from a fee.
    pub runes: Vec<String>,
    pub value: u64,
}

/// ord's `/r/utxo/<outpoint>`. `Ok(None)`: not found (spent before ord read it, or never seen).
pub fn parse_ord_contents(status: u16, body: &[u8]) -> Result<Option<Contents>, String> {
    if status == 404 {
        return Ok(None);
    }
    if status != 200 {
        return Err(format!("ord: HTTP {status}"));
    }
    let v: serde_json::Value = serde_json::from_slice(body).map_err(|_| "ord: not JSON")?;
    let mut inscriptions: Vec<String> = v.get("inscriptions").and_then(|x| x.as_array()).ok_or("ord: no inscriptions field")?
        .iter().map(|x| x.as_str().map(str::to_string).ok_or("ord: an inscription id that is not text"))
        .collect::<Result<_, _>>()?;
    inscriptions.sort();
    let mut runes: Vec<String> = v.get("runes").and_then(|x| x.as_object()).ok_or("ord: no runes field")?.keys().cloned().collect();
    runes.sort();
    let value = v.get("value").and_then(|x| x.as_u64()).ok_or("ord: no value")?;
    Ok(Some(Contents { inscriptions, runes, value }))
}

// ---- what the explorers say -----------------------------------------------------------------------------------------------

/// One output as an esplora explorer serves it, with its transaction's confirmation.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Output {
    pub script: Vec<u8>,
    pub value: u64,
    pub confirmed: bool,
}

/// esplora's `/tx/<txid>`, output `vout`.
pub fn parse_esplora_output(status: u16, body: &[u8], vout: u32) -> Result<Output, String> {
    if status != 200 {
        return Err(format!("explorer: HTTP {status}"));
    }
    let v: serde_json::Value = serde_json::from_slice(body).map_err(|_| "explorer: not JSON")?;
    let o = v.get("vout").and_then(|x| x.as_array()).and_then(|a| a.get(vout as usize)).ok_or("explorer: no such output")?;
    let script = hex::decode(o.get("scriptpubkey").and_then(|x| x.as_str()).ok_or("explorer: no script")?)
        .map_err(|_| "explorer: a script that is not hex")?;
    let value = o.get("value").and_then(|x| x.as_u64()).ok_or("explorer: no value")?;
    let confirmed = v.get("status").and_then(|s| s.get("confirmed")).and_then(|c| c.as_bool()).ok_or("explorer: no status")?;
    Ok(Output { script, value, confirmed })
}

/// esplora's `/tx/<txid>/outspend/<vout>`: whether the output is spent (in a block or a mempool).
pub fn parse_outspend(status: u16, body: &[u8]) -> Result<bool, String> {
    if status != 200 {
        return Err(format!("explorer: HTTP {status}"));
    }
    let v: serde_json::Value = serde_json::from_slice(body).map_err(|_| "explorer: not JSON")?;
    v.get("spent").and_then(|x| x.as_bool()).ok_or_else(|| "explorer: no spent field".into())
}

/// One UTXO from esplora's `/address/<a>/utxo` or `/scripthash/<h>/utxo`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Utxo {
    pub outpoint: Outpoint,
    pub value: u64,
    pub confirmed: bool,
}

/// Sorted by outpoint, so that two explorers listing the same set in different orders agree.
pub fn parse_esplora_utxos(status: u16, body: &[u8]) -> Result<Vec<Utxo>, String> {
    if status != 200 {
        return Err(format!("explorer: HTTP {status}"));
    }
    let v: serde_json::Value = serde_json::from_slice(body).map_err(|_| "explorer: not JSON")?;
    let mut out: Vec<Utxo> = v.as_array().ok_or("explorer: not a list")?.iter().map(|u| {
        Ok(Utxo {
            outpoint: Outpoint {
                txid: txid_of(u.get("txid").and_then(|x| x.as_str()).ok_or("explorer: a utxo with no txid")?)?,
                vout: u.get("vout").and_then(|x| x.as_u64()).ok_or("explorer: a utxo with no vout")? as u32,
            },
            value: u.get("value").and_then(|x| x.as_u64()).ok_or("explorer: a utxo with no value")?,
            confirmed: u.get("status").and_then(|s| s.get("confirmed")).and_then(|c| c.as_bool()).unwrap_or(false),
        })
    }).collect::<Result<_, String>>()?;
    out.sort_by(|a, b| a.outpoint.cmp(&b.outpoint));
    Ok(out)
}

/// esplora's `/fee-estimates`: the rate for confirmation within 6 blocks (else 3, else 1), rounded up, bounded.
pub fn parse_fee_estimates(status: u16, body: &[u8]) -> Result<u64, String> {
    if status != 200 {
        return Err(format!("explorer: HTTP {status}"));
    }
    let v: serde_json::Value = serde_json::from_slice(body).map_err(|_| "explorer: not JSON")?;
    let r = ["6", "3", "1"].iter().find_map(|k| v.get(*k).and_then(|x| x.as_f64())).ok_or("explorer: no fee estimate")?;
    if !r.is_finite() || r < 0.0 {
        return Err("explorer: an unreadable fee estimate".into());
    }
    Ok(bound_rate(r.ceil() as u64))
}

pub fn bound_rate(r: u64) -> u64 {
    r.clamp(MIN_FEE_RATE, MAX_FEE_RATE)
}

/// Several explorers' fee estimates as one rate (audit R2, N-3, replacing L-7's `min(max, 2·min)`, under which one explorer
/// reporting low set every custody rate): the **upper median** (`sorted[n / 2]`), then bounded. With two explorers that is the
/// higher one, so a low report cannot underprice a delivery or a conflict-out, and a high one can at most reach the 100 sat/vB
/// clamp (bounded overpayment); with three or more, one explorer either way cannot move it outside the honest ones.
pub fn combine_rates(rates: &[u64]) -> Result<u64, String> {
    Ok(choose_rate(rates)?.0)
}

/// The estimates disagree strongly: the highest more than this many times the lowest ...
pub const RATE_DISAGREE_FACTOR: u64 = 4;
/// ... and more than this many sat/vB apart.
pub const RATE_DISAGREE_GAP: u64 = 20;

/// [`combine_rates`], and a line for the watcher when the estimates disagree strongly. A disagreement is not a wait: that would
/// hand a lying-high explorer a lever over every delivery and conflict-out. The upper median is used either way.
pub fn choose_rate(rates: &[u64]) -> Result<(u64, Option<String>), String> {
    let mut v = rates.to_vec();
    v.sort_unstable();
    let (lo, hi) = (*v.first().ok_or("no fee estimates")?, *v.last().ok_or("no fee estimates")?);
    let rate = bound_rate(v[v.len() / 2]);
    let alarm = (hi > lo.max(1).saturating_mul(RATE_DISAGREE_FACTOR) && hi - lo > RATE_DISAGREE_GAP)
        .then(|| format!("FEE ESTIMATES DISAGREE: {v:?} sat/vB; paying the upper median, {rate}"));
    Ok((rate, alarm))
}

// ---- the frog, and the fee ------------------------------------------------------------------------------------------------

/// A frog the canister can send: its output, and that output's value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frog {
    pub output: Outpoint,
    pub value: u64,
}

/// Why a frog cannot be sent from where it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotSendable {
    /// Something may still change by itself: ord has not indexed it yet, or its output is not yet confirmed. Read again.
    Wait(String),
    /// It is not where the canister can safely send it from, and waiting will not change that. Loud.
    Stuck(String),
}

/// Judge ord's location and contents against what every explorer said about that output. `ours` is our taproot script.
pub fn frog_at(id: &str, loc: &Location, contents: Option<&Contents>, out: &Output, spent: bool, ours: &[u8])
    -> Result<Frog, NotSendable> {
    let Some(c) = contents else {
        return Err(NotSendable::Wait(format!("ord has not indexed {} yet", loc.output.text())));
    };
    if spent {
        return Err(NotSendable::Wait(format!("{} is spent on the explorers but ord still shows it there: ord is behind", loc.output.text())));
    }
    if !out.confirmed {
        return Err(NotSendable::Wait(format!("{} is not confirmed yet", loc.output.text())));
    }
    if out.script != ours {
        return Err(NotSendable::Stuck(format!("{id} is at {}, which is not ours: it is not in custody", loc.output.text())));
    }
    if out.value != loc.value || c.value != loc.value {
        return Err(NotSendable::Stuck(format!("{} is worth {} on the explorers and {}/{} on ord: nothing is believed",
            loc.output.text(), out.value, loc.value, c.value)));
    }
    if c.inscriptions != [id.to_string()] {
        return Err(NotSendable::Stuck(format!("{} carries {:?}, not exactly {id}: sending it would send more than one frog",
            loc.output.text(), c.inscriptions)));
    }
    if !c.runes.is_empty() {
        return Err(NotSendable::Stuck(format!("{} also carries runes {:?}: sending it would send them too", loc.output.text(), c.runes)));
    }
    Ok(Frog { output: loc.output, value: loc.value })
}

/// Whether one of our UTXOs may pay a fee: confirmed, clean on ord, not the frog, not reserved by another delivery in flight.
pub fn fee_eligible(u: &Utxo, c: Option<&Contents>, frog: &Outpoint, reserved: &[Outpoint]) -> bool {
    u.confirmed
        && u.outpoint != *frog
        && !reserved.contains(&u.outpoint)
        && c.is_some_and(|c| c.inscriptions.is_empty() && c.runes.is_empty() && c.value == u.value)
}

/// The candidates worth asking ord about, smallest sufficient first: confirmed, not the frog, not reserved, able to pay a
/// fee at `rate` on their own, and not dust ([`MIN_FEE_INPUT_SATS`]).
pub fn fee_candidates(utxos: &[Utxo], frog: &Outpoint, reserved: &[Outpoint], rate: u64) -> Vec<Utxo> {
    let need = (vsize(2, &[34]) * rate).max(MIN_FEE_INPUT_SATS);
    let mut c: Vec<Utxo> = utxos.iter().filter(|u| u.confirmed && u.outpoint != *frog && !reserved.contains(&u.outpoint)
        && u.value >= need).cloned().collect();
    c.sort_by_key(|u| (u.value, u.outpoint));
    c.truncate(FEE_CANDIDATES);
    c
}

/// Custody's fee inputs as the canister knows them (audit D, H-4): its own change and conflict outputs, added when the
/// transaction that made them confirms, and plain-sat top-ups the operator registers. The address is public, so a list of its
/// UTXOs is a stranger's to break (more than 500 and mempool.space refuses the listing; ≈ 1,600 and blockstream's answer is
/// past the cap): the pool is what fee inputs are chosen from, and every entry is verified again at use.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize, candid::CandidType)]
pub struct FeePool {
    pub entries: Vec<FeeEntry>,
    /// Entries that failed to read [`QUARANTINE_AFTER`] times while the explorers answered for others (audit R2, N-4): no longer
    /// candidates, kept for the operator to see, remove, or restore by adding again. `None` in a state saved before it existed.
    #[serde(default)]
    pub quarantined: Option<Vec<FeeEntry>>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, candid::CandidType)]
pub struct FeeEntry {
    pub outpoint: Outpoint,
    pub value: u64,
    /// `change` (ours, from a confirmed custody spend) or `operator` (a registered top-up).
    pub source: String,
    /// Reads in a row that failed while another candidate's read succeeded (audit R2, N-4).
    #[serde(default)]
    pub fails: Option<u32>,
    /// When the last counted failure was (seconds): failures closer together than [`QUARANTINE_SPACING_SECS`] count once
    /// (audit R3, R3-C4). `None`: none counted, or a state saved before it existed.
    #[serde(default)]
    pub last_failed_at: Option<u64>,
}

/// A pool entry that fails to read this many times, each time while the explorers answered for another candidate, is
/// quarantined (audit R2, N-4). An explorer outage fails every read and so counts nothing.
pub const QUARANTINE_AFTER: u32 = 3;
/// A pool entry's failed reads count at most once in this long (audit R3, R3-C4): quarantine takes three failing reads in three
/// separate hours, never three fee choices in one tick with a transient per-output explorer error.
pub const QUARANTINE_SPACING_SECS: u64 = 3_600;

/// At most this many fee inputs are kept: the oldest go first.
pub const MAX_FEE_POOL: usize = 32;

impl FeePool {
    /// Add one; under [`MIN_FEE_INPUT_SATS`] is not worth keeping (it could never be chosen).
    pub fn add(&mut self, outpoint: Outpoint, value: u64, source: &str) -> Result<(), String> {
        if value < MIN_FEE_INPUT_SATS {
            return Err(format!("{} holds {value} sats, under the {MIN_FEE_INPUT_SATS}-sat least a fee input may hold", outpoint.text()));
        }
        if self.entries.iter().any(|e| e.outpoint == outpoint) {
            return Ok(());
        }
        // adding a quarantined entry again is the operator restoring it
        if let Some(q) = self.quarantined.as_mut() {
            q.retain(|e| e.outpoint != outpoint);
        }
        self.entries.push(FeeEntry { outpoint, value, source: source.into(), fails: None, last_failed_at: None });
        if self.entries.len() > MAX_FEE_POOL {
            self.entries.remove(0);
        }
        Ok(())
    }
    pub fn drop_entry(&mut self, o: &Outpoint) {
        self.entries.retain(|e| e.outpoint != *o);
    }
    /// The operator removes an entry, a candidate or a quarantined one. Whether there was one.
    pub fn remove(&mut self, o: &Outpoint) -> bool {
        let before = self.entries.len() + self.quarantined.as_ref().map_or(0, Vec::len);
        self.entries.retain(|e| e.outpoint != *o);
        if let Some(q) = self.quarantined.as_mut() {
            q.retain(|e| e.outpoint != *o);
        }
        before != self.entries.len() + self.quarantined.as_ref().map_or(0, Vec::len)
    }
    /// A candidate's read failed while another's worked (audit R2, N-4), at `now` (seconds). At [`QUARANTINE_AFTER`] counted
    /// failures it leaves the candidates; the line says so, for the watcher. A failure within [`QUARANTINE_SPACING_SECS`] of the
    /// last counted one is not counted again (audit R3, R3-C4).
    pub fn failed(&mut self, o: &Outpoint, why: &str, now: u64) -> Option<String> {
        let e = self.entries.iter_mut().find(|e| e.outpoint == *o)?;
        if e.last_failed_at.is_some_and(|t| now.saturating_sub(t) < QUARANTINE_SPACING_SECS) {
            return None;
        }
        let n = e.fails.unwrap_or(0) + 1;
        e.fails = Some(n);
        e.last_failed_at = Some(now);
        if n < QUARANTINE_AFTER {
            return None;
        }
        let i = self.entries.iter().position(|e| e.outpoint == *o)?;
        let mut gone = self.entries.remove(i);
        gone.fails = Some(n);
        let q = self.quarantined.get_or_insert_with(Vec::new);
        q.push(gone);
        if q.len() > MAX_FEE_POOL {
            q.remove(0);
        }
        Some(format!("FEE POOL: {} quarantined after {n} failed reads, in separate hours, while other outputs read fine ({why}); \
            the operator removes it, or adds it again to restore it", o.text()))
    }
    /// A candidate read fine: its failures are forgotten.
    pub fn read_ok(&mut self, o: &Outpoint) {
        if let Some(e) = self.entries.iter_mut().find(|e| e.outpoint == *o) {
            e.fails = None;
            e.last_failed_at = None;
        }
    }
    /// The entries as fee candidates. `confirmed` is the claim the chooser verifies on every explorer before use.
    pub fn utxos(&self) -> Vec<Utxo> {
        self.entries.iter().map(|e| Utxo { outpoint: e.outpoint, value: e.value, confirmed: true }).collect()
    }
}

/// Belt and braces for the BUYER (docs/v3/wrapper.md, "Custody on its own key"): before it signs an input as its own, ord's
/// account of that output. Anything carrying an inscription or a rune is refused; an output ord does not know is refused too
/// (fail closed: ord a block behind is a wait, not a pass).
pub fn buyer_may_sign(o: &Outpoint, c: Option<&Contents>) -> Result<(), String> {
    match c {
        None => Err(format!("refused: ord does not know {}, so what it carries cannot be checked before signing", o.text())),
        Some(c) if !c.inscriptions.is_empty() => Err(format!("refused: {} carries inscription(s) {:?}: a purchase never spends one",
            o.text(), c.inscriptions)),
        Some(c) if !c.runes.is_empty() => Err(format!("refused: {} carries runes {:?}: a purchase never spends them", o.text(), c.runes)),
        Some(_) => Ok(()),
    }
}

/// The input's outpoint as an explorer names it (display order).
pub fn outpoint_of(i: &TxIn) -> Outpoint {
    let mut t = i.txid;
    t.reverse();
    Outpoint { txid: t, vout: i.vout }
}

// ---- the transaction ------------------------------------------------------------------------------------------------------

/// Virtual size of a transaction spending `inputs` taproot key-path inputs to outputs with the given script lengths.
pub fn vsize(inputs: usize, output_script_lens: &[usize]) -> u64 {
    let compact = |n: usize| if n < 0xfd { 1 } else { 3 };
    let base = 4 + compact(inputs) + inputs * (32 + 4 + 1 + 4) + compact(output_script_lens.len())
        + output_script_lens.iter().map(|l| 8 + compact(*l) + l).sum::<usize>() + 4;
    let witness = 2 + inputs * (1 + 1 + 64);
    (base * 4 + witness).div_ceil(4) as u64
}

fn is_p2tr(s: &[u8]) -> bool {
    s.len() == 34 && s[0] == 0x51 && s[1] == 0x20
}

/// The checks every delivery builder makes before it chooses a fee.
fn build_checks(frog: &Frog, fee: &Utxo, dest: &[u8], ours: &[u8]) -> Result<(), String> {
    if !is_p2tr(dest) {
        return Err("the destination is not a taproot script".into());
    }
    if !is_p2tr(ours) {
        return Err("our own script is not a taproot script".into());
    }
    if dest == ours {
        return Err("refused: the destination is the custodian itself".into());
    }
    if frog.value < DUST_P2TR {
        return Err(format!("the frog's output is {} sats, under the {DUST_P2TR}-sat dust limit for its destination", frog.value));
    }
    if fee.outpoint == frog.output {
        return Err("the fee input is the frog".into());
    }
    Ok(())
}

/// The delivery: the frog's whole output to `dest`, the fee from `fee`, change (if not dust) back to `ours`.
pub fn build(frog: &Frog, fee: &Utxo, dest: &[u8], ours: &[u8], rate: u64) -> Result<Psbt, String> {
    build_checks(frog, fee, dest, ours)?;
    let rate = bound_rate(rate);
    let with_change = vsize(2, &[dest.len(), ours.len()]) * rate;
    let (fee_sats, change) = if fee.value >= with_change + DUST_P2TR {
        (with_change, Some(fee.value - with_change))
    } else {
        let alone = vsize(2, &[dest.len()]) * rate;
        if fee.value < alone {
            return Err(format!("the fee input holds {} sats; {alone} are needed at {rate} sat/vB", fee.value));
        }
        (fee.value, None) // what is under dust goes to the miner rather than making an unrelayable output
    };
    assemble(frog, fee, dest, ours, fee_sats, change)
}

/// The same delivery paying exactly `fee_sats` (audit D, M-1's re-pricing: the fee is chosen by the bump rule, not a rate),
/// with change back to `ours`. Refused when the fee input cannot pay it and keep a change output above dust: a bump never
/// turns the change into fee silently.
pub fn build_with_fee(frog: &Frog, fee: &Utxo, dest: &[u8], ours: &[u8], fee_sats: u64) -> Result<Psbt, String> {
    build_checks(frog, fee, dest, ours)?;
    let change = fee.value.checked_sub(fee_sats).filter(|c| *c >= DUST_P2TR)
        .ok_or_else(|| format!("the fee input holds {} sats: it cannot pay {fee_sats} and keep change", fee.value))?;
    assemble(frog, fee, dest, ours, fee_sats, Some(change))
}

fn assemble(frog: &Frog, fee: &Utxo, dest: &[u8], ours: &[u8], fee_sats: u64, change: Option<u64>) -> Result<Psbt, String> {
    if fee_sats > MAX_DELIVERY_FEE_SATS {
        return Err(format!("a fee of {fee_sats} sats is over the {MAX_DELIVERY_FEE_SATS}-sat ceiling: refused rather than paid"));
    }
    let mut outputs = vec![TxOut { value: frog.value, script: dest.to_vec() }];
    if let Some(c) = change {
        outputs.push(TxOut { value: c, script: ours.to_vec() });
    }
    let tx = Tx {
        version: TRUC_VERSION,
        inputs: vec![
            TxIn { txid: frog.output.internal_txid(), vout: frog.output.vout, sequence: SEQUENCE },
            TxIn { txid: fee.outpoint.internal_txid(), vout: fee.outpoint.vout, sequence: SEQUENCE },
        ],
        outputs,
        locktime: 0,
    };
    let psbt = Psbt { tx, witness_utxos: vec![Some(TxOut { value: frog.value, script: ours.to_vec() }),
        Some(TxOut { value: fee.value, script: ours.to_vec() })] };
    check(&psbt, frog, dest)?;
    Ok(psbt)
}

/// The property, checked on the built transaction rather than trusted to the builder: input 0 is the frog, output 0 pays the
/// destination exactly the frog's value, and no other output pays the destination.
pub fn check(psbt: &Psbt, frog: &Frog, dest: &[u8]) -> Result<(), String> {
    let tx = &psbt.tx;
    let i0 = tx.inputs.first().ok_or("no inputs")?;
    if i0.txid != frog.output.internal_txid() || i0.vout != frog.output.vout {
        return Err("input 0 is not the frog".into());
    }
    let o0 = tx.outputs.first().ok_or("no outputs")?;
    if o0.script != dest || o0.value != frog.value {
        return Err("output 0 does not pay the destination exactly the frog's value".into());
    }
    if tx.outputs.iter().skip(1).any(|o| o.script == dest) {
        return Err("a second output pays the destination".into());
    }
    let ins: u64 = psbt.witness_utxos.iter().map(|w| w.as_ref().map_or(0, |o| o.value)).sum();
    let outs: u64 = tx.outputs.iter().map(|o| o.value).sum();
    if outs > ins {
        return Err("the outputs are worth more than the inputs".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIX: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/delivery/");
    fn fx(name: &str) -> Vec<u8> {
        std::fs::read(format!("{FIX}{name}")).expect("fixture")
    }
    const ID0: &str = "6fb976ab49dcec017f1e201e84395983204ae1a7c2abf7ced0a85d692e442799i0";
    const OUT0: &str = "717d25914a1191c30f4c1d709e3ec0e1fab79cc7bf92cd12a93bed68db63416a:0";

    fn script(tag: u8) -> Vec<u8> {
        let mut s = vec![0x51, 0x20];
        s.extend([tag; 32]);
        s
    }

    /// Audit R2, N-3 (L-7 again): the upper median, bounded. One explorer reporting low never lowers it; one reporting high
    /// reaches at most the clamp with two explorers, and nothing with three.
    #[test]
    fn the_fee_rate_is_the_upper_median_so_one_low_explorer_cannot_set_it() {
        assert_eq!(combine_rates(&[5, 6]), Ok(6));
        assert_eq!(combine_rates(&[1, 150]), Ok(MAX_FEE_RATE), "R2's case: the honest 150 (clamped), not 2");
        assert_eq!(combine_rates(&[0, 150]), Ok(MAX_FEE_RATE));
        assert_eq!(combine_rates(&[80, 90]), Ok(90));
        assert_eq!(combine_rates(&[300, 400]), Ok(MAX_FEE_RATE));
        assert_eq!(combine_rates(&[0, 0]), Ok(MIN_FEE_RATE));
        assert_eq!(combine_rates(&[7]), Ok(7));
        assert_eq!(combine_rates(&[1, 40, 42]), Ok(40), "three: a low liar is outvoted");
        assert_eq!(combine_rates(&[40, 42, 400]), Ok(42), "three: a high liar is outvoted");
        assert_eq!(combine_rates(&[1, 40, 41, 42]), Ok(41));
        assert!(combine_rates(&[]).is_err());
        // a strong disagreement is said, not waited on
        let (r, alarm) = choose_rate(&[1, 150]).unwrap();
        assert_eq!(r, MAX_FEE_RATE);
        assert!(alarm.unwrap().contains("DISAGREE"));
        assert_eq!(choose_rate(&[5, 12]).unwrap().1, None, "within 4x: quiet");
        assert_eq!(choose_rate(&[1, 15]).unwrap().1, None, "15x but only 14 sat/vB apart: quiet");
    }

    /// Audit D, H-4 and M-1: dust is never a fee candidate; the pool keeps what can be one; a bump pays an exact fee from the
    /// change and never turns the change into fee.
    #[test]
    fn dust_is_never_a_fee_input_the_pool_is_bounded_and_a_bump_pays_exactly_its_fee() {
        let o = |t: u8| Outpoint { txid: [t; 32], vout: 0 };
        let us: Vec<Utxo> = [(1, 330), (2, 4_999), (3, 5_000), (4, 20_000)].iter()
            .map(|(t, v)| Utxo { outpoint: o(*t), value: *v, confirmed: true }).collect();
        let c = fee_candidates(&us, &o(9), &[], 1);
        assert_eq!(c.iter().map(|u| u.value).collect::<Vec<_>>(), vec![5_000, 20_000], "no dust, even at 1 sat/vB");
        let mut pool = FeePool::default();
        assert!(pool.add(o(1), 546, "change").is_err());
        for t in 0..(MAX_FEE_POOL as u8 + 3) {
            pool.add(o(t), 10_000, "change").unwrap();
        }
        pool.add(o(40), 10_000, "change").unwrap();
        pool.add(o(40), 10_000, "change").unwrap();
        assert_eq!(pool.entries.len(), MAX_FEE_POOL);
        pool.drop_entry(&o(40));
        assert!(!pool.utxos().iter().any(|u| u.outpoint == o(40)));
        let frog = Frog { output: o(0xf0), value: 546 };
        let fee = Utxo { outpoint: o(0xfe), value: 20_000, confirmed: true };
        let (dest, ours) = ([vec![0x51, 0x20], vec![0xde; 32]].concat(), [vec![0x51, 0x20], vec![1; 32]].concat());
        let p = build_with_fee(&frog, &fee, &dest, &ours, 7_000).unwrap();
        assert_eq!((p.tx.outputs[0].value, p.tx.outputs[1].value), (546, 13_000));
        assert!(build_with_fee(&frog, &fee, &dest, &ours, 19_700).unwrap_err().contains("keep change"));
        assert!(build_with_fee(&frog, &Utxo { value: 60_000, ..fee }, &dest, &ours, 26_000).unwrap_err().contains("ceiling"));
    }

    #[test]
    fn ord_says_where_inscription_zero_is_and_what_its_output_carries() {
        let loc = parse_ord_inscription(200, &fx("ord-inscription-0.json"), ID0).unwrap().unwrap();
        assert_eq!(loc, Location { output: Outpoint::parse(OUT0).unwrap(), offset: 0, value: 606 });
        assert!(parse_ord_inscription(200, &fx("ord-inscription-0.json"), "00i0").unwrap_err().contains("different inscription"));
        assert_eq!(parse_ord_inscription(404, b"", ID0), Ok(None));
        let c = parse_ord_contents(200, &fx("ord-utxo-inscription-0.json")).unwrap().unwrap();
        assert_eq!((c.inscriptions, c.runes, c.value), (vec![ID0.to_string()], vec![], 606));
        let r = parse_ord_contents(200, &fx("ord-utxo-runes.json")).unwrap().unwrap();
        assert_eq!((r.inscriptions.len(), r.runes, r.value), (0, vec!["BITBLOCK".to_string()], 546));
        let p = parse_ord_contents(200, &fx("ord-utxo-plain.json")).unwrap().unwrap();
        assert_eq!((p.inscriptions.len(), p.runes.len(), p.value), (0, 0, 2500));
        assert_eq!(parse_ord_contents(404, b"output deadbeef:0 not found"), Ok(None));
        assert!(check_inscription_id(ID0).is_ok());
        for bad in ["", "i0", "deadbeefi0", &format!("{}x", ID0), &ID0.replace("i0", "i"), "../../etc/passwd"] {
            assert!(check_inscription_id(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_explorers_say_the_output_exists_unspent_confirmed_and_whose_it_is() {
        let o = parse_esplora_output(200, &fx("esplora-tx-717d.json"), 0).unwrap();
        assert_eq!((o.value, o.confirmed, hex::encode(&o.script[..2])), (606, true, "5120".to_string()));
        assert!(parse_esplora_output(200, &fx("esplora-tx-717d.json"), 9).is_err());
        assert_eq!(parse_outspend(200, &fx("esplora-outspend-717d-0.json")), Ok(false));
        assert_eq!(parse_outspend(200, br#"{"spent":true,"txid":"ab"}"#), Ok(true));
        let us = parse_esplora_utxos(200, &fx("esplora-utxo-bc1pd96x.json")).unwrap();
        assert_eq!(us.len(), 12);
        assert!(us.windows(2).all(|w| w[0].outpoint <= w[1].outpoint), "sorted, so two explorers' orders agree");
        assert_eq!(parse_fee_estimates(200, br#"{"1":2.411,"3":2.005,"6":1.071}"#), Ok(2));
        assert_eq!(parse_fee_estimates(200, br#"{"6":0.2}"#), Ok(MIN_FEE_RATE));
        assert_eq!(parse_fee_estimates(200, br#"{"6":5000}"#), Ok(MAX_FEE_RATE));
    }

    #[test]
    fn a_frog_is_sendable_only_from_our_own_confirmed_unspent_output_carrying_exactly_it() {
        let loc = parse_ord_inscription(200, &fx("ord-inscription-0.json"), ID0).unwrap().unwrap();
        let c = parse_ord_contents(200, &fx("ord-utxo-inscription-0.json")).unwrap().unwrap();
        let mut out = parse_esplora_output(200, &fx("esplora-tx-717d.json"), 0).unwrap();
        // the real inscription 0 is not ours: stuck, loudly
        assert!(matches!(frog_at(ID0, &loc, Some(&c), &out, false, &script(9)), Err(NotSendable::Stuck(s)) if s.contains("not in custody")));
        let ours = out.script.clone();
        assert_eq!(frog_at(ID0, &loc, Some(&c), &out, false, &ours), Ok(Frog { output: loc.output, value: 606 }));
        assert!(matches!(frog_at(ID0, &loc, None, &out, false, &ours), Err(NotSendable::Wait(_))), "ord behind: wait");
        assert!(matches!(frog_at(ID0, &loc, Some(&c), &out, true, &ours), Err(NotSendable::Wait(_))), "spent, ord behind: wait");
        let mut two = c.clone();
        two.inscriptions.push("aa".repeat(32) + "i1");
        two.inscriptions.sort();
        assert!(matches!(frog_at(ID0, &loc, Some(&two), &out, false, &ours), Err(NotSendable::Stuck(s)) if s.contains("not exactly")));
        let mut runes = c.clone();
        runes.runes = vec!["BITBLOCK".into()];
        assert!(matches!(frog_at(ID0, &loc, Some(&runes), &out, false, &ours), Err(NotSendable::Stuck(s)) if s.contains("runes")));
        let mut cheap = c.clone();
        cheap.value = 546;
        assert!(matches!(frog_at(ID0, &loc, Some(&cheap), &out, false, &ours), Err(NotSendable::Stuck(_))), "values disagree");
        out.confirmed = false;
        assert!(matches!(frog_at(ID0, &loc, Some(&c), &out, false, &ours), Err(NotSendable::Wait(_))));
    }

    #[test]
    fn a_fee_is_paid_only_by_a_clean_confirmed_unreserved_utxo_that_is_not_the_frog() {
        let frog = Outpoint::parse(OUT0).unwrap();
        let plain = parse_ord_contents(200, &fx("ord-utxo-plain.json")).unwrap().unwrap();
        let runes = parse_ord_contents(200, &fx("ord-utxo-runes.json")).unwrap().unwrap();
        let u = Utxo { outpoint: Outpoint::parse(&("57dd43538d17048682ca40662df1889d4d579bfcff16e81449a6541181457e9a".to_string() + ":2")).unwrap(), value: 2500, confirmed: true };
        assert!(fee_eligible(&u, Some(&plain), &frog, &[]));
        assert!(!fee_eligible(&u, Some(&runes), &frog, &[]), "carries runes (and a different value)");
        assert!(!fee_eligible(&u, None, &frog, &[]), "ord does not know it");
        assert!(!fee_eligible(&u, Some(&plain), &frog, &[u.outpoint]), "reserved by another delivery");
        assert!(!fee_eligible(&Utxo { confirmed: false, ..u.clone() }, Some(&plain), &frog, &[]));
        assert!(!fee_eligible(&Utxo { outpoint: frog, value: 2500, confirmed: true }, Some(&plain), &frog, &[]), "the frog itself");
        let all = parse_esplora_utxos(200, &fx("esplora-utxo-bc1pd96x.json")).unwrap();
        let c = fee_candidates(&all, &frog, &[], 2);
        assert!(c.iter().all(|x| x.outpoint != frog && x.confirmed && x.value >= vsize(2, &[34]) * 2));
        assert!(c.windows(2).all(|w| w[0].value <= w[1].value), "smallest sufficient first");
    }

    #[test]
    fn the_buyer_signs_only_outputs_ord_shows_clean() {
        let o = Outpoint::parse(OUT0).unwrap();
        let frog = parse_ord_contents(200, &fx("ord-utxo-inscription-0.json")).unwrap().unwrap();
        let runes = parse_ord_contents(200, &fx("ord-utxo-runes.json")).unwrap().unwrap();
        let plain = parse_ord_contents(200, &fx("ord-utxo-plain.json")).unwrap().unwrap();
        assert!(buyer_may_sign(&o, Some(&frog)).unwrap_err().contains("inscription"));
        assert!(buyer_may_sign(&o, Some(&runes)).unwrap_err().contains("runes"));
        assert!(buyer_may_sign(&o, None).unwrap_err().contains("does not know"), "fail closed");
        assert_eq!(buyer_may_sign(&o, Some(&plain)), Ok(()));
        // an input's txid (internal order) names the display-order outpoint
        let i = TxIn { txid: o.internal_txid(), vout: 0, sequence: 0 };
        assert_eq!(outpoint_of(&i), o);
    }

    #[test]
    fn the_delivery_pays_the_whole_frog_output_to_the_destination_and_the_fee_from_the_other_input() {
        let (ours, dest) = (script(1), script(2));
        let frog = Frog { output: Outpoint::parse(OUT0).unwrap(), value: 606 };
        let fee = Utxo { outpoint: Outpoint::parse(&("57dd43538d17048682ca40662df1889d4d579bfcff16e81449a6541181457e9a".to_string() + ":2")).unwrap(), value: 2500, confirmed: true };
        let p = build(&frog, &fee, &dest, &ours, 2).unwrap();
        assert_eq!(p.tx.outputs[0].value, 606);
        assert_eq!(p.tx.outputs[0].script, dest);
        let fee_sats = 606 + 2500 - p.tx.outputs.iter().map(|o| o.value).sum::<u64>();
        assert_eq!(fee_sats, vsize(2, &[34, 34]) * 2);
        assert_eq!(p.tx.outputs[1], TxOut { value: 2500 - fee_sats, script: ours.clone() });
        assert_eq!(p.tx.inputs[0].txid, frog.output.internal_txid());
        // vsize: 2 key-path inputs, 2 P2TR outputs is the textbook 211 vB (212 with the rounding up of the witness discount)
        assert_eq!(vsize(2, &[34, 34]), 212);
        // change under dust goes to the miner rather than making an unrelayable output
        let small = Utxo { value: vsize(2, &[34, 34]) * 2 + 100, ..fee.clone() };
        assert_eq!(build(&frog, &small, &dest, &ours, 2).unwrap().tx.outputs.len(), 1);
        // refusals
        assert!(build(&frog, &Utxo { value: 100, ..fee.clone() }, &dest, &ours, 2).unwrap_err().contains("needed"));
        assert!(build(&frog, &fee, &ours, &ours, 2).unwrap_err().contains("custodian itself"));
        assert!(build(&frog, &fee, &[0x00, 0x14, 1, 2], &ours, 2).unwrap_err().contains("not a taproot"));
        assert!(build(&Frog { value: 294, ..frog.clone() }, &fee, &dest, &ours, 2).unwrap_err().contains("dust"));
        assert!(build(&frog, &Utxo { outpoint: frog.output, ..fee.clone() }, &dest, &ours, 2).unwrap_err().contains("is the frog"));
        let rich = Utxo { value: 10_000_000, ..fee.clone() };
        assert!(build(&frog, &rich, &dest, &ours, 10_000).is_ok(), "a rate is bounded to 100 sat/vB, so it is not refused");
        assert!(vsize(2, &[34, 34]) * MAX_FEE_RATE <= MAX_DELIVERY_FEE_SATS, "the ceiling never binds inside the rate bound");
        // an implementation that is not ours agrees, byte for byte, on the TRUC (version 3) delivery (@scure/btc-signer, through
        // autobuyer/gen-delivery-fixture.mjs with `version: 3`; audit R2, N-2)
        assert_eq!(p.tx.version, TRUC_VERSION, "every delivery is a TRUC transaction");
        let dest86 = hex::decode("5120a60869f0dbcf1dc659c9cecbaf8050135ea9e8cdc487053f1dc6880949dc684c").unwrap();
        let scure = build(&frog, &fee, &dest86, &ours, 2).unwrap();
        assert_eq!(hex::encode(scure.txid()), "3ab1dcf517e2ceef2ebc7a7dd0dccc170bb00d5cf605b1a09c680c58e32aa26c");
        assert_eq!(scure.tx.outputs[1].value, 2076);
        assert_eq!(hex::encode(crate::sighash::taproot_key_path(&scure, 0).unwrap()), "272db9c80da91ffaa9d7405d66fc1575ddc4eeb6c3046bb0e60a291064036585");
        assert_eq!(hex::encode(crate::sighash::taproot_key_path(&scure, 1).unwrap()), "ada45320049b6d921777920545307f93eb3224ff7b9305d68a2cef874c13ac68");
        // the version-2 build scure also agreed with (round 1) differs only in nVersion: the same digest code, another commitment
        let mut v2 = scure.clone();
        v2.tx.version = 2;
        assert_eq!(hex::encode(v2.txid()), "c94a3138906e565b7b4be8af684fb3108dae744826d6cdbee0bb9af20deeab41");
        assert_eq!(hex::encode(crate::sighash::taproot_key_path(&v2, 0).unwrap()), "d96dd925e254c73563b3252ca63f1cf59b2bbf783accc770d2ef91f8b11de728");
        // the check refuses a tampered transaction
        let mut bad = p.clone();
        bad.tx.outputs[0].value = 605;
        assert!(check(&bad, &frog, &dest).is_err());
        let mut swapped = p.clone();
        swapped.tx.inputs.swap(0, 1);
        assert!(check(&swapped, &frog, &dest).is_err());
    }
}
