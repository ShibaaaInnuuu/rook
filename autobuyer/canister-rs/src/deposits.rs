//! Holder deposits (docs/v3/sweep.md, 1a; docs/v3/curve.md, "Canister: …", "Holder deposits").
//!
//! Pure. A frog owner sends an inscription to the custody address in a transaction whose one `OP_RETURN` names this chain,
//! this wrapper and an EVM recipient; after six confirmations and the checks below, the canister calls
//! `Wrapper.wrapDeposit` once for that outpoint. Anything that fails a check is refused and recorded, and — when the reason
//! is the depositor's and the sender's address is taproot — returned whole with custody's own delivery builder.
//!
//! Which custody outputs are deposits at all: **only those whose transaction carries an `OP_RETURN` beginning `EWD1`**.
//! Custody also receives frogs bought for the vault, our own frog buys, delivery change and plain-sat top-ups; none of those
//! carries the marker, and none of them is ever minted or returned by this module.

use crate::abi::{self, Arg};
use crate::custody::{Contents, Location, Outpoint, Output};

/// `"EWD1"`.
pub const MAGIC: &[u8; 4] = b"EWD1";
/// `"EWD1" ‖ chainId (4) ‖ wrapper (20) ‖ recipient (20)`.
pub const PAYLOAD_LEN: usize = 48;
/// The rate limit: at most this many deposits minted in any 24 hours (docs/v3/sweep.md: "e.g. 50 a day").
pub const DEPOSITS_PER_DAY: usize = 50;
pub const DAY_SECS: u64 = 24 * 60 * 60;
/// Returns of refused deposits in any 24 hours: each pays a fee from custody's float, so a stream of marked junk cannot drain
/// it. Over the limit a refused frog waits (it is not lost); a person can see the queue in `holder_deposits`.
pub const RETURNS_PER_DAY: usize = 10;

// ---- what one tick may spend on deposits, however many a stranger makes (audit D, H-3) -------------------------------------------

/// At most this many unfinished deposits are stepped a tick: the ones due earliest.
pub const MAX_DEPOSITS_STEPPED_PER_TICK: usize = 4;
/// At most this many unfinished deposits are tracked; classification adds no more past it.
pub const MAX_OPEN_DEPOSITS: usize = 32;
/// A deposit that waits is looked at again after 10 min × 2ⁿ, at most a day.
pub const BACKOFF_BASE_SECS: u64 = 600;
pub const BACKOFF_MAX_SECS: u64 = DAY_SECS;
/// A deposit that has waited this long for a membership proof is parked: finished, and read no more (the operator may still
/// refuse it, which returns it, or hand in its proof, which wakes it).
pub const PARK_AFTER_SECS: u64 = 14 * DAY_SECS;
/// A custody output whose transaction cannot be read this many times is set aside, not retried (a transaction padded past the
/// response cap would otherwise be read every tick for ever).
pub const MAX_READ_FAILURES: u32 = 3;

/// When a deposit that has waited `tries` times is next looked at.
pub fn next_check(now: u64, tries: u32) -> u64 {
    now + BACKOFF_BASE_SECS.saturating_mul(1u64 << tries.min(20)).min(BACKOFF_MAX_SECS)
}

/// `wrapDeposit(string,bytes32[],bytes32,uint32,address)`, `depositMinted(bytes32)`, `inscriptionHeld(bytes32)`,
/// `leafOfFrog(uint256)`, `custodian()`, `registry()`, `listingId()`, `custodyAddress()`; the registry's `listing(uint256)`.
pub const WRAP_DEPOSIT: [u8; 4] = [0x50, 0x98, 0xf0, 0x01];
pub const DEPOSIT_MINTED: [u8; 4] = [0x13, 0xf7, 0xfb, 0x0e];
pub const INSCRIPTION_HELD: [u8; 4] = [0x26, 0x3e, 0x60, 0x2c];
pub const LEAF_OF_FROG: [u8; 4] = [0x56, 0x4c, 0x3a, 0x4a];
pub const CUSTODIAN: [u8; 4] = [0x37, 0x5b, 0x74, 0xc3];
pub const REGISTRY: [u8; 4] = [0x7b, 0x10, 0x39, 0x99];
pub const LISTING_ID: [u8; 4] = [0x94, 0x05, 0xdf, 0xdd];
pub const LISTING: [u8; 4] = [0x84, 0xa1, 0x84, 0xde];

/// `wrapDeposit(inscriptionId, proof, txid, vout, to)`. `txid` in display order, as an explorer shows it.
pub fn wrap_deposit(inscription: &str, proof: &[[u8; 32]], txid: [u8; 32], vout: u32, to: &[u8; 20]) -> Vec<u8> {
    let mut out = WRAP_DEPOSIT.to_vec();
    let head = 5 * 32;
    let s = inscription.as_bytes();
    let s_len = 32 + s.len().div_ceil(32) * 32;
    out.extend(abi::uint_word(head as u128));
    out.extend(abi::uint_word((head + s_len) as u128));
    out.extend(txid);
    out.extend(abi::uint_word(vout as u128));
    out.extend(abi::addr_word(to));
    out.extend(abi::uint_word(s.len() as u128));
    out.extend_from_slice(s);
    out.resize(out.len() + (32 - s.len() % 32) % 32, 0);
    out.extend(abi::uint_word(proof.len() as u128));
    for p in proof {
        out.extend(p);
    }
    out
}

/// The Wrapper's key for an outpoint: `keccak256(abi.encode(txid, vout))`.
pub fn outpoint_key(txid: [u8; 32], vout: u32) -> [u8; 32] {
    crate::evmtx::keccak(&[txid, abi::uint_word(vout as u128)].concat())
}

pub fn call_word(sel: [u8; 4], w: [u8; 32]) -> Vec<u8> {
    abi::encode_call(sel, &[Arg::Word(w)])
}

/// A frog id as the Wrapper numbers deposits (`DEPOSIT_FLAG | n`, bit 255) as the custodian carries it internally: bit 127.
pub const DEPOSIT_BIT: u128 = 1 << 127;

/// The 256-bit word for an internal frog id: bit 127 set means the Wrapper's bit 255.
pub fn frog_id_word(id: u128) -> [u8; 32] {
    let mut w = abi::uint_word(id & !DEPOSIT_BIT);
    if id & DEPOSIT_BIT != 0 {
        w[0] |= 0x80;
    }
    w
}

/// The inverse: a Wrapper frog id word to the internal id. Refuses any other bit above 127.
pub fn frog_id_of_word(w: &[u8]) -> Result<u128, String> {
    if w.len() != 32 {
        return Err("a frog id is one word".into());
    }
    let flag = w[0] & 0x80 != 0;
    if w[0] & 0x7f != 0 || w[1..16].iter().any(|x| *x != 0) || (w[16] & 0x80 != 0) {
        return Err("a frog id wider than an acquisition id or a deposit number".into());
    }
    let low = u128::from_be_bytes(w[16..].try_into().expect("16 bytes"));
    Ok(if flag { DEPOSIT_BIT | low } else { low })
}

/// The registry's `listing(id)` membership root (word 5 of the returned struct).
pub fn ret_membership_root(b: &[u8]) -> Result<[u8; 32], String> {
    let base = abi::uint_at(b, 0, 0, 32)? as usize;
    abi::word_at(b, base, 5)?.try_into().map_err(|_| "a root is one word".to_string())
}

// ---- the transaction, as the explorers serve it --------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct InView {
    pub prev: Outpoint,
    pub prev_script: Vec<u8>,
    pub script_sig: Vec<u8>,
    pub witness: Vec<Vec<u8>>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct OutView {
    pub value: u64,
    pub script: Vec<u8>,
}

/// A transaction as esplora's `/tx/<txid>` describes it, reduced to what the checks read. Two explorers must agree on it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TxView {
    pub txid: [u8; 32],
    pub inputs: Vec<InView>,
    pub outputs: Vec<OutView>,
    pub confirmed: bool,
    pub height: Option<u64>,
    pub block_hash: Option<String>,
}

fn hexv(v: Option<&serde_json::Value>, what: &str) -> Result<Vec<u8>, String> {
    hex::decode(v.and_then(|x| x.as_str()).unwrap_or("")).map_err(|_| format!("explorer: {what} is not hex"))
}

pub fn parse_esplora_tx(status: u16, body: &[u8]) -> Result<TxView, String> {
    if status != 200 {
        return Err(format!("explorer: HTTP {status}"));
    }
    let v: serde_json::Value = serde_json::from_slice(body).map_err(|_| "explorer: not JSON")?;
    let id = |x: Option<&serde_json::Value>| -> Result<[u8; 32], String> {
        hex::decode(x.and_then(|t| t.as_str()).ok_or("explorer: no txid")?).ok().and_then(|b| b.try_into().ok())
            .ok_or_else(|| "explorer: an unreadable txid".to_string())
    };
    let txid = id(v.get("txid"))?;
    let mut inputs = Vec::new();
    for i in v.get("vin").and_then(|x| x.as_array()).ok_or("explorer: no vin")? {
        if i.get("is_coinbase").and_then(|x| x.as_bool()) == Some(true) {
            return Err("explorer: a coinbase is not a deposit".into());
        }
        let witness = match i.get("witness").and_then(|x| x.as_array()) {
            Some(w) => w.iter().map(|x| hexv(Some(x), "a witness item")).collect::<Result<_, _>>()?,
            None => vec![],
        };
        inputs.push(InView {
            prev: Outpoint { txid: id(i.get("txid"))?, vout: i.get("vout").and_then(|x| x.as_u64()).ok_or("explorer: no vout")? as u32 },
            prev_script: hexv(i.get("prevout").and_then(|p| p.get("scriptpubkey")), "a prevout script")?,
            script_sig: hexv(i.get("scriptsig"), "a scriptsig")?,
            witness,
        });
    }
    let mut outputs = Vec::new();
    for o in v.get("vout").and_then(|x| x.as_array()).ok_or("explorer: no vout list")? {
        outputs.push(OutView {
            value: o.get("value").and_then(|x| x.as_u64()).ok_or("explorer: an output with no value")?,
            script: hexv(o.get("scriptpubkey"), "an output script")?,
        });
    }
    let st = v.get("status").ok_or("explorer: no status")?;
    let confirmed = st.get("confirmed").and_then(|x| x.as_bool()).ok_or("explorer: no confirmed")?;
    let (height, block_hash) = if confirmed {
        (Some(st.get("block_height").and_then(|x| x.as_u64()).ok_or("explorer: confirmed, no height")?),
         Some(st.get("block_hash").and_then(|x| x.as_str()).ok_or("explorer: confirmed, no block hash")?.to_ascii_lowercase()))
    } else {
        (None, None)
    };
    Ok(TxView { txid, inputs, outputs, confirmed, height, block_hash })
}

// ---- the marker -----------------------------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, candid::CandidType)]
pub struct Marker {
    pub chain_id: u32,
    pub wrapper: [u8; 20],
    pub recipient: [u8; 20],
}

/// What a transaction's `OP_RETURN`s say about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Marked {
    /// No `OP_RETURN` begins `EWD1`: not a deposit (never minted, never returned by this code).
    No,
    /// A deposit: the marker, or why it is malformed.
    Yes(Result<Marker, String>),
}

/// The data of an `OP_RETURN` that is a single push: `6a <n> data` (n ≤ 75) or `6a 4c <n> data`. Anything else: `None`.
fn single_push(script: &[u8]) -> Option<&[u8]> {
    match script {
        [0x6a, n, rest @ ..] if (1..=75).contains(n) && rest.len() == *n as usize => Some(rest),
        [0x6a, 0x4c, n, rest @ ..] if rest.len() == *n as usize => Some(rest),
        _ => None,
    }
}

/// Find the marker. A transaction is a deposit if any `OP_RETURN` carries the `EWD1` prefix anywhere a push could put it;
/// it is well-formed only if it has exactly one `OP_RETURN`, a single push of exactly 48 bytes.
pub fn marker(outputs: &[OutView]) -> Marked {
    let returns: Vec<&OutView> = outputs.iter().filter(|o| o.script.first() == Some(&0x6a)).collect();
    let tagged = returns.iter().any(|o| o.script.windows(4).any(|w| w == MAGIC));
    if !tagged {
        return Marked::No;
    }
    if returns.len() != 1 {
        return Marked::Yes(Err(format!("{} OP_RETURN outputs; a deposit has exactly one", returns.len())));
    }
    let Some(d) = single_push(&returns[0].script) else {
        return Marked::Yes(Err("the OP_RETURN is not a single push".into()));
    };
    if d.len() != PAYLOAD_LEN || &d[..4] != MAGIC {
        return Marked::Yes(Err(format!("the payload is {} bytes, not EWD1 and 44 more", d.len())));
    }
    Marked::Yes(Ok(Marker {
        chain_id: u32::from_be_bytes(d[4..8].try_into().expect("4")),
        wrapper: d[8..28].try_into().expect("20"),
        recipient: d[28..48].try_into().expect("20"),
    }))
}

// ---- the checks -------------------------------------------------------------------------------------------------------------

/// Why a deposit is refused. `returnable` says whether the frog goes back to its sender.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, candid::CandidType)]
pub enum Reason {
    Malformed(String),
    WrongChain(u32),
    WrongWrapper,
    ZeroRecipient,
    /// An input of the deposit pays custody: custody's own coin. Never returned.
    SpendsCustody(u32),
    /// An input not signed `SIGHASH_ALL`/`DEFAULT`, or one whose signature cannot be read that way.
    Sighash { input: u32, why: String },
    /// Not exactly one inscription on the output.
    Inscriptions(Vec<String>),
    Runes(Vec<String>),
    /// ord places the inscription on another output than this one.
    Elsewhere,
    /// The operator refused it as not in the collection (no proof exists).
    NotInCollection(String),
    /// Custody has no room for it under the caps (docs/v5/custody-hardening.md, H2): returned to its depositor.
    OverCustodyCap(String),
}

impl Reason {
    pub fn returnable(&self) -> bool {
        !matches!(self, Reason::SpendsCustody(_))
    }
}

/// The deposit's transaction-level checks: the marker names us, nothing spends custody, every input signs everything.
pub fn check_tx(tx: &TxView, m: &Marker, chain_id: u64, wrapper: &[u8; 20], custody: &[u8]) -> Result<(), Reason> {
    if m.chain_id as u64 != chain_id {
        return Err(Reason::WrongChain(m.chain_id));
    }
    if m.wrapper != *wrapper {
        return Err(Reason::WrongWrapper);
    }
    if m.recipient == [0u8; 20] {
        return Err(Reason::ZeroRecipient);
    }
    if let Some(i) = tx.inputs.iter().position(|i| i.prev_script == custody) {
        return Err(Reason::SpendsCustody(i as u32));
    }
    for (i, inp) in tx.inputs.iter().enumerate() {
        sighash_all(inp).map_err(|why| Reason::Sighash { input: i as u32, why })?;
    }
    Ok(())
}

fn is_p2tr(s: &[u8]) -> bool {
    s.len() == 34 && s[0] == 0x51 && s[1] == 0x20
}
fn is_p2wpkh(s: &[u8]) -> bool {
    s.len() == 22 && s[0] == 0x00 && s[1] == 0x14
}
fn is_p2sh(s: &[u8]) -> bool {
    s.len() == 23 && s[0] == 0xa9 && s[1] == 0x14 && s[22] == 0x87
}

/// One input signs every input and output: taproot key path (64 bytes, or 65 ending `0x01`), P2WPKH or P2SH-P2WPKH (the DER
/// signature's last byte `0x01`). Anything the canister cannot read that way is refused.
pub fn sighash_all(i: &InView) -> Result<(), String> {
    let s = &i.prev_script;
    if is_p2tr(s) {
        if !i.script_sig.is_empty() {
            return Err("a taproot input with a scriptSig".into());
        }
        return match i.witness.as_slice() {
            [sig] if sig.len() == 64 => Ok(()),
            [sig] if sig.len() == 65 && sig[64] == 0x01 => Ok(()),
            [sig] if sig.len() == 65 => Err(format!("sighash type 0x{:02x}", sig[64])),
            [.., last] if last.first() == Some(&0x50) => Err("an annex".into()),
            _ => Err("a script-path spend: its signatures cannot be checked here".into()),
        };
    }
    let wpkh_sig = |w: &[Vec<u8>]| -> Result<(), String> {
        match w {
            [sig, key] if key.len() == 33 && sig.len() >= 9 && sig[0] == 0x30 => match sig.last() {
                Some(0x01) => Ok(()),
                Some(t) => Err(format!("sighash type 0x{t:02x}")),
                None => Err("an empty signature".into()),
            },
            _ => Err("not a P2WPKH witness".into()),
        }
    };
    if is_p2wpkh(s) {
        if !i.script_sig.is_empty() {
            return Err("a P2WPKH input with a scriptSig".into());
        }
        return wpkh_sig(&i.witness);
    }
    if is_p2sh(s) {
        // P2SH-P2WPKH: the scriptSig pushes exactly the 22-byte witness program, and the witness is a P2WPKH pair
        if i.script_sig.len() == 23 && i.script_sig[0] == 22 && is_p2wpkh(&i.script_sig[1..]) {
            return wpkh_sig(&i.witness);
        }
        return Err("a P2SH input that is not P2SH-P2WPKH".into());
    }
    Err("a legacy or unknown input type: its sighash cannot be checked here".into())
}

/// What ord and the explorers say about the output, judged: `Ok(Some(id))` exactly one inscription and no runes, here, at
/// custody; `Ok(None)` a plain output (not a frog); `Err(Ok(r))` refused; `Err(Err(s))` wait (ord behind, not confirmed).
pub fn check_output(o: &Outpoint, contents: Option<&Contents>, located: Option<&Location>, out: &Output, spent: bool,
    custody: &[u8]) -> Result<Option<String>, Result<Reason, String>> {
    let Some(c) = contents else {
        return Err(Err(format!("ord has not indexed {} yet", o.text())));
    };
    if c.inscriptions.is_empty() && c.runes.is_empty() {
        return Ok(None);
    }
    if !c.runes.is_empty() {
        return Err(Ok(Reason::Runes(c.runes.clone())));
    }
    if c.inscriptions.len() != 1 {
        return Err(Ok(Reason::Inscriptions(c.inscriptions.clone())));
    }
    let id = c.inscriptions[0].clone();
    match located {
        None => return Err(Err(format!("ord does not locate {id} yet"))),
        Some(l) if l.output != *o => return Err(Ok(Reason::Elsewhere)),
        Some(l) if l.value != c.value => return Err(Err("ord's location and contents disagree on the value".into())),
        Some(_) => {}
    }
    if spent {
        return Err(Err(format!("{} is spent on the explorers: ord is behind", o.text())));
    }
    if !out.confirmed || out.script != custody || out.value != c.value {
        return Err(Err(format!("the explorers do not show {} confirmed at custody for {} sats", o.text(), c.value)));
    }
    Ok(Some(id))
}

/// Where a refused frog goes back to: its transaction's input 0, if that is a taproot script that is not one of ours (custody,
/// the float, the buyer: audit D, I-6).
pub fn return_dest(tx: &TxView, ours: &[&[u8]]) -> Option<Vec<u8>> {
    let s = &tx.inputs.first()?.prev_script;
    if is_p2tr(s) && !ours.contains(&s.as_slice()) { Some(s.clone()) } else { None }
}

/// Deposits minted in the 24 hours before `now` are under the limit.
pub fn under_limit(mint_times: &[u64], now: u64) -> bool {
    mint_times.iter().filter(|t| now.saturating_sub(**t) < DAY_SECS).count() < DEPOSITS_PER_DAY
}

/// Returns sent in the 24 hours before `now` are under the limit.
pub fn returns_under_limit(return_times: &[u64], now: u64) -> bool {
    return_times.iter().filter(|t| now.saturating_sub(**t) < DAY_SECS).count() < RETURNS_PER_DAY
}

// ---- the record ----------------------------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, candid::CandidType)]
pub enum DPhase {
    /// Seen, marked; checks pending (depth, ord, proof, the wrapper's state, the rate limit). `why` is the last wait.
    Pending { why: String },
    /// Every check passed at the last read; the mint is next (re-checked immediately before it).
    Ready { inscription: String, recipient: [u8; 20] },
    Minted { inscription: String, recipient: [u8; 20] },
    /// Refused and to be returned.
    Refused { reason: Reason },
    /// Refused, and the canister will not return it (not returnable, or no taproot sender): recorded for a person.
    Held { reason: Reason, why: String },
    /// The return, signed and saved; the same bytes until Bitcoin shows them. `txid` display order. `unseen_since`: when Bitcoin
    /// was first found not showing them, cleared whenever a mempool does; after `conflict::RETURN_STALE_SECS` of that the
    /// return is abandoned and conflicted out (H8). `None` in a record saved before it existed: the clock starts at the next look.
    Returning {
        reason: Reason,
        txid: [u8; 32],
        raw: Vec<u8>,
        fee_input: Outpoint,
        #[serde(default)]
        unseen_since: Option<u64>,
    },
    /// H8: an evicted return abandoned; its fee input is being spent back to custody so the old bytes can never be mined.
    /// Ends `Refused` (a fresh return is built) or `Returned` (the old return confirmed first).
    ReturnConflicting { reason: Reason, conflict: crate::conflict::Conflict },
    Returned { reason: Reason, txid: [u8; 32] },
    /// Plain sats with a marker: nothing to mint or return.
    NotAFrog,
    /// Every check passed but the membership proof (audit D, H-3): not read again until a proof for `inscription` is handed in
    /// (`frog_proof`), or the operator refuses it (`refuse_deposit`), which returns it.
    AwaitingProof { inscription: String },
    /// Waited `PARK_AFTER_SECS` for a proof: finished and read no more. The operator may still refuse it (it is returned) or hand
    /// in its proof (it wakes).
    Parked { inscription: String },
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, candid::CandidType)]
pub struct DepositRec {
    pub outpoint: Outpoint,
    pub first_seen: u64,
    pub phase: DPhase,
    /// The inscription counted under the custody caps for this deposit (H2), released when a return confirms.
    #[serde(default)]
    pub admitted: Option<String>,
    /// Not looked at again before this time (audit D, H-3's backoff). `None`: due now.
    #[serde(default)]
    pub next_check: Option<u64>,
    /// Looks in a row that changed nothing.
    #[serde(default)]
    pub tries: Option<u32>,
}

impl DepositRec {
    pub fn finished(&self) -> bool {
        matches!(self.phase, DPhase::Minted { .. } | DPhase::Held { .. } | DPhase::Returned { .. } | DPhase::NotAFrog | DPhase::Parked { .. })
    }
    /// The custody outputs a return in flight, or its conflict-out, has claimed: no other custody send may choose them.
    pub fn reserved(&self) -> Vec<Outpoint> {
        match &self.phase {
            DPhase::Returning { fee_input, .. } => vec![*fee_input],
            DPhase::ReturnConflicting { conflict, .. } => conflict.reserved(),
            _ => vec![],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: [u8; 20] = [0xaa; 20];
    const R: [u8; 20] = [0xbb; 20];

    fn custody() -> Vec<u8> {
        let mut s = vec![0x51, 0x20];
        s.extend([2u8; 32]);
        s
    }
    fn p2tr(tag: u8) -> Vec<u8> {
        let mut s = vec![0x51, 0x20];
        s.extend([tag; 32]);
        s
    }
    fn payload(chain: u32, w: [u8; 20], r: [u8; 20]) -> Vec<u8> {
        [MAGIC.to_vec(), chain.to_be_bytes().to_vec(), w.to_vec(), r.to_vec()].concat()
    }
    fn op_return(d: &[u8]) -> OutView {
        OutView { value: 0, script: [vec![0x6a, d.len() as u8], d.to_vec()].concat() }
    }
    fn key_path_in(tag: u8, sig_len: usize, last: u8) -> InView {
        let mut sig = vec![7u8; sig_len];
        if sig_len == 65 {
            sig[64] = last;
        }
        InView { prev: Outpoint { txid: [tag; 32], vout: 0 }, prev_script: p2tr(tag), script_sig: vec![], witness: vec![sig] }
    }
    fn tx(outputs: Vec<OutView>) -> TxView {
        TxView { txid: [9; 32], inputs: vec![key_path_in(5, 64, 0)], outputs, confirmed: true, height: Some(100),
            block_hash: Some("00".repeat(32)) }
    }

    #[test]
    fn the_calls_match_foundrys_encoding() {
        for (sel, s) in [(WRAP_DEPOSIT, "wrapDeposit(string,bytes32[],bytes32,uint32,address)"), (DEPOSIT_MINTED, "depositMinted(bytes32)"),
            (INSCRIPTION_HELD, "inscriptionHeld(bytes32)"), (LEAF_OF_FROG, "leafOfFrog(uint256)"), (CUSTODIAN, "custodian()"),
            (REGISTRY, "registry()"), (LISTING_ID, "listingId()"), (LISTING, "listing(uint256)")] {
            assert_eq!(crate::evmtx::keccak(s.as_bytes())[..4], sel, "{s}");
        }
        // `cast calldata "wrapDeposit(string,bytes32[],bytes32,uint32,address)" "<id>" "[0x11…,0x22…]" 0x717d…416a 3 0x…aa`
        let want = "5098f00100000000000000000000000000000000000000000000000000000000000000a00000000000000000000000000000000000000000000000000000000000000120717d25914a1191c30f4c1d709e3ec0e1fab79cc7bf92cd12a93bed68db63416a000000000000000000000000000000000000000000000000000000000000000300000000000000000000000000000000000000000000000000000000000000aa0000000000000000000000000000000000000000000000000000000000000042366662393736616234396463656330313766316532303165383433393539383332303461653161376332616266376365643061383564363932653434323739396930000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000211111111111111111111111111111111111111111111111111111111111111112222222222222222222222222222222222222222222222222222222222222222";
        let txid: [u8; 32] = hex::decode("717d25914a1191c30f4c1d709e3ec0e1fab79cc7bf92cd12a93bed68db63416a").unwrap().try_into().unwrap();
        let mut to = [0u8; 20];
        to[19] = 0xaa;
        let got = wrap_deposit("6fb976ab49dcec017f1e201e84395983204ae1a7c2abf7ced0a85d692e442799i0", &[[0x11; 32], [0x22; 32]], txid, 3, &to);
        assert_eq!(hex::encode(got), want);
        // an empty proof and a short id: `cast calldata … "aai0" "[]" 0x…01 0 0x…aa`
        let mut one = [0u8; 32];
        one[31] = 1;
        assert_eq!(hex::encode(wrap_deposit("aai0", &[], one, 0, &to)), "5098f00100000000000000000000000000000000000000000000000000000000000000a000000000000000000000000000000000000000000000000000000000000000e00000000000000000000000000000000000000000000000000000000000000001000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000aa000000000000000000000000000000000000000000000000000000000000000461616930000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000");
        // `cast keccak $(cast abi-encode "f(bytes32,uint32)" 0x717d…416a 3)`
        assert_eq!(hex::encode(outpoint_key(txid, 3)), "fba625b2943e1686550931dfbd10a3b662c9c06b9122fba4e5d9b2bc1637bf64");
    }

    #[test]
    fn deposit_ids_carry_bit_255_through_as_bit_127_and_nothing_else_wider() {
        let mut w = [0u8; 32];
        w[0] = 0x80;
        w[31] = 7;
        assert_eq!(frog_id_of_word(&w), Ok(DEPOSIT_BIT | 7));
        assert_eq!(frog_id_word(DEPOSIT_BIT | 7), w);
        assert_eq!(frog_id_of_word(&abi::uint_word(42)), Ok(42), "an acquisition id is unchanged");
        let mut bad = w;
        bad[1] = 1;
        assert!(frog_id_of_word(&bad).is_err());
        let mut bad = [0u8; 32];
        bad[16] = 0x80;
        assert!(frog_id_of_word(&bad).is_err(), "bit 127 itself is not an id the wrapper makes");
    }

    #[test]
    fn a_well_formed_marker_names_chain_wrapper_and_recipient() {
        let t = tx(vec![OutView { value: 546, script: custody() }, op_return(&payload(4663, W, R))]);
        let Marked::Yes(Ok(m)) = marker(&t.outputs) else { panic!() };
        assert_eq!(m, Marker { chain_id: 4663, wrapper: W, recipient: R });
        assert_eq!(check_tx(&t, &m, 4663, &W, &custody()), Ok(()));
        assert_eq!(check_tx(&t, &m, 46630, &W, &custody()), Err(Reason::WrongChain(4663)));
        assert_eq!(check_tx(&t, &m, 4663, &[1; 20], &custody()), Err(Reason::WrongWrapper));
        let z = Marker { recipient: [0; 20], ..m.clone() };
        assert_eq!(check_tx(&t, &z, 4663, &W, &custody()), Err(Reason::ZeroRecipient));
        // OP_PUSHDATA1 is the same single push
        let pd1 = OutView { value: 0, script: [vec![0x6a, 0x4c, 48], payload(4663, W, R)].concat() };
        assert!(matches!(marker(&[pd1]), Marked::Yes(Ok(_))));
    }

    #[test]
    fn a_missing_marker_is_not_a_deposit_and_a_malformed_one_is_refused() {
        // no OP_RETURN, or one that is not ours: not a deposit at all (a vault-bought frog, our own buy, change)
        assert_eq!(marker(&[OutView { value: 546, script: custody() }]), Marked::No);
        assert_eq!(marker(&[op_return(b"hello")]), Marked::No);
        // the prefix with the wrong length, two OP_RETURNs, two pushes
        assert!(matches!(marker(&[op_return(&payload(4663, W, R)[..47])]), Marked::Yes(Err(s)) if s.contains("47 bytes")));
        let mut long = payload(4663, W, R);
        long.push(0);
        assert!(matches!(marker(&[op_return(&long)]), Marked::Yes(Err(_))));
        assert!(matches!(marker(&[op_return(&payload(4663, W, R)), op_return(b"x")]), Marked::Yes(Err(s)) if s.contains("2 OP_RETURN")));
        let two_pushes = OutView { value: 0, script: [vec![0x6a, 4], MAGIC.to_vec(), vec![44], vec![0; 44]].concat() };
        assert!(matches!(marker(&[two_pushes]), Marked::Yes(Err(s)) if s.contains("single push")));
    }

    #[test]
    fn every_input_must_sign_everything() {
        assert_eq!(sighash_all(&key_path_in(1, 64, 0)), Ok(()));
        assert_eq!(sighash_all(&key_path_in(1, 65, 0x01)), Ok(()));
        assert!(sighash_all(&key_path_in(1, 65, 0x83)).unwrap_err().contains("0x83"), "SINGLE|ANYONECANPAY");
        let mut sp = key_path_in(1, 64, 0);
        sp.witness = vec![vec![1; 64], vec![0x20; 34], vec![0xc0; 33]];
        assert!(sighash_all(&sp).unwrap_err().contains("script-path"));
        let mut annex = key_path_in(1, 64, 0);
        annex.witness.push(vec![0x50, 1]);
        assert!(sighash_all(&annex).unwrap_err().contains("annex"));
        let wpkh = |last: u8| InView { prev: Outpoint { txid: [1; 32], vout: 0 }, prev_script: [vec![0, 0x14], vec![3; 20]].concat(),
            script_sig: vec![], witness: vec![[vec![0x30; 70], vec![last]].concat(), vec![2; 33]] };
        assert_eq!(sighash_all(&wpkh(0x01)), Ok(()));
        assert!(sighash_all(&wpkh(0x02)).is_err(), "SIGHASH_NONE");
        let mut nested = wpkh(0x01);
        nested.prev_script = [vec![0xa9, 0x14], vec![4; 20], vec![0x87]].concat();
        nested.script_sig = [vec![22, 0, 0x14], vec![3; 20]].concat();
        assert_eq!(sighash_all(&nested), Ok(()));
        let legacy = InView { prev_script: [vec![0x76, 0xa9, 0x14], vec![3; 20], vec![0x88, 0xac]].concat(), ..wpkh(0x01) };
        assert!(sighash_all(&legacy).is_err());
        // and the transaction check reports which input
        let mut t = tx(vec![op_return(&payload(4663, W, R))]);
        t.inputs.push(key_path_in(6, 65, 0x82));
        let m = Marker { chain_id: 4663, wrapper: W, recipient: R };
        assert!(matches!(check_tx(&t, &m, 4663, &W, &custody()), Err(Reason::Sighash { input: 1, .. })));
        t.inputs[1].prev_script = custody();
        assert_eq!(check_tx(&t, &m, 4663, &W, &custody()), Err(Reason::SpendsCustody(1)), "custody's own coin");
        assert!(!Reason::SpendsCustody(1).returnable());
    }

    #[test]
    fn the_output_carries_exactly_the_inscription_and_no_runes_per_ord_and_the_explorers() {
        let o = Outpoint { txid: [9; 32], vout: 0 };
        let id = format!("{}i0", "ab".repeat(32));
        let c = Contents { inscriptions: vec![id.clone()], runes: vec![], value: 546 };
        let loc = Location { output: o, offset: 0, value: 546 };
        let out = Output { script: custody(), value: 546, confirmed: true };
        assert_eq!(check_output(&o, Some(&c), Some(&loc), &out, false, &custody()), Ok(Some(id.clone())));
        assert!(matches!(check_output(&o, None, None, &out, false, &custody()), Err(Err(_))), "ord behind: wait");
        let runes = Contents { runes: vec!["BITBLOCK".into()], ..c.clone() };
        assert_eq!(check_output(&o, Some(&runes), Some(&loc), &out, false, &custody()), Err(Ok(Reason::Runes(vec!["BITBLOCK".into()]))));
        let two = Contents { inscriptions: vec![id.clone(), format!("{}i1", "ab".repeat(32))], ..c.clone() };
        assert!(matches!(check_output(&o, Some(&two), Some(&loc), &out, false, &custody()), Err(Ok(Reason::Inscriptions(v))) if v.len() == 2));
        let plain = Contents { inscriptions: vec![], runes: vec![], value: 10_000 };
        assert_eq!(check_output(&o, Some(&plain), None, &out, false, &custody()), Ok(None), "plain sats: not a frog");
        let elsewhere = Location { output: Outpoint { txid: [8; 32], vout: 1 }, ..loc.clone() };
        assert_eq!(check_output(&o, Some(&c), Some(&elsewhere), &out, false, &custody()), Err(Ok(Reason::Elsewhere)));
        assert!(matches!(check_output(&o, Some(&c), Some(&loc), &out, true, &custody()), Err(Err(_))));
        let unconfirmed = Output { confirmed: false, ..out.clone() };
        assert!(matches!(check_output(&o, Some(&c), Some(&loc), &unconfirmed, false, &custody()), Err(Err(_))));
    }

    #[test]
    fn a_refused_frog_goes_back_to_a_taproot_sender_only() {
        let t = tx(vec![]);
        assert_eq!(return_dest(&t, &[&custody()]), Some(p2tr(5)));
        assert_eq!(return_dest(&t, &[&p2tr(5)]), None, "never to ourselves");
        let mut w = t.clone();
        w.inputs[0].prev_script = [vec![0, 0x14], vec![3; 20]].concat();
        assert_eq!(return_dest(&w, &[]), None, "not taproot: held for a person");
    }

    #[test]
    fn a_waiting_deposit_backs_off_exponentially_to_a_day() {
        let now = 1_000_000;
        assert_eq!(next_check(now, 0), now + 600);
        assert_eq!(next_check(now, 1), now + 1_200);
        assert_eq!(next_check(now, 5), now + 600 * 32);
        assert_eq!(next_check(now, 9), now + DAY_SECS, "capped at a day");
        assert_eq!(next_check(now, 60), now + DAY_SECS, "no overflow");
    }

    #[test]
    fn fifty_a_day() {
        let now = 1_000_000;
        let full: Vec<u64> = (0..50).map(|i| now - i * 60).collect();
        assert!(!under_limit(&full, now));
        assert!(under_limit(&full[1..], now));
        let old: Vec<u64> = (0..50).map(|i| now - DAY_SECS - i).collect();
        assert!(under_limit(&old, now), "a day later they no longer count");
        assert!(!returns_under_limit(&vec![now; RETURNS_PER_DAY], now));
        assert!(returns_under_limit(&vec![now; RETURNS_PER_DAY - 1], now));
    }

    #[test]
    fn an_esplora_transaction_reads_back_with_its_witnesses_and_status() {
        let body = serde_json::json!({
            "txid": "09".repeat(32),
            "vin": [{ "txid": "05".repeat(32), "vout": 1, "prevout": { "scriptpubkey": hex::encode(p2tr(5)), "value": 600 },
                "scriptsig": "", "witness": ["07".repeat(64)], "is_coinbase": false }],
            "vout": [{ "scriptpubkey": hex::encode(custody()), "value": 546 },
                { "scriptpubkey": hex::encode(op_return(&payload(4663, W, R)).script), "value": 0 }],
            "status": { "confirmed": true, "block_height": 800_000, "block_hash": "AB".repeat(32) }
        }).to_string();
        let t = parse_esplora_tx(200, body.as_bytes()).unwrap();
        assert_eq!((t.txid, t.inputs[0].prev.vout, t.inputs[0].witness[0].len(), t.height), ([9; 32], 1, 64, Some(800_000)));
        assert_eq!(t.block_hash.as_deref(), Some("ab".repeat(32).as_str()), "lower-cased, so explorers agree");
        assert!(matches!(marker(&t.outputs), Marked::Yes(Ok(_))));
        assert!(parse_esplora_tx(404, b"").is_err());
    }
}
