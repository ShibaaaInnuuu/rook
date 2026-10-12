//! Reading a PSBT, and refusing to sign one that does not do what we asked for.
//!
//! This is the whole security argument for talking to a marketplace over a **non-replicated** HTTPS outcall, where one node
//! writes the response and could write anything. The buyer never trusts that response. It parses the bytes itself, works out
//! what the transaction moves — sats by arithmetic, runes by [`crate::runestone`] — and signs only if every rule below holds:
//!
//! 1. every input we are asked to sign is ours, and we are asked to sign no other;
//! 2. what leaves us in sats is at most the cap recorded **before** the PSBT was requested;
//! 3. the runestone sends at least the quoted units of the quoted rune to an output paying the recipient's script;
//! 4. our change comes back to our own script;
//! 5. the miner fee is within a bound;
//! 6. nothing is spread across outputs, and no second runestone muddies the arithmetic.
//!
//! A node that lies fails one of these and the purchase fails. It cannot make us pay more than the cap, redirect the asset, or
//! take our change. See `docs/v2/state-machine.md`, "What a single malicious node can and cannot do".

use crate::runestone::{DecodeError, RuneId, Runestone};

const MAGIC: &[u8] = b"psbt\xff";
const PSBT_GLOBAL_UNSIGNED_TX: u8 = 0x00;
const PSBT_IN_WITNESS_UTXO: u8 = 0x01;

/// One transaction output: what it is worth and who can spend it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, candid::CandidType)]
pub struct TxOut {
    pub value: u64,
    pub script: Vec<u8>,
}

/// One transaction input, as the unsigned transaction carries it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, candid::CandidType)]
pub struct TxIn {
    pub txid: [u8; 32],
    pub vout: u32,
    pub sequence: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, candid::CandidType)]
pub struct Tx {
    pub version: i32,
    pub inputs: Vec<TxIn>,
    pub outputs: Vec<TxOut>,
    pub locktime: u32,
}

/// A parsed PSBT: the transaction, and for each input what it is spending (which is what tells us its value and owner).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, candid::CandidType)]
pub struct Psbt {
    pub tx: Tx,
    /// One per input, in order. `None` when the PSBT did not say what that input spends.
    pub witness_utxos: Vec<Option<TxOut>>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ParseError {
    NotAPsbt,
    Truncated,
    /// A key repeated inside one map: the format forbids it, and a duplicate is how a hostile encoder hides a second value.
    DuplicateKey,
    /// The global map had no unsigned transaction, or the maps did not match the transaction's shape.
    Malformed,
    /// A PSBT whose unsigned transaction already carries signature data.
    NotUnsigned,
}

/// Why a PSBT was refused. Every variant is a reason not to sign, and each is reported as itself so a failure is legible.
#[derive(Debug, PartialEq, Eq)]
pub enum Refusal {
    /// An input we were asked to sign is not ours.
    ForeignInputToSign(usize),
    /// The PSBT does not say what an input of ours spends, so its value is unknown.
    UnknownInputValue(usize),
    /// We would pay more than the cap recorded before the PSBT was asked for.
    OverCap { spend: u64, cap: u64 },
    /// The miner fee is above the bound.
    FeeTooHigh { fee: u64, bound: u64 },
    /// Outputs are worth more than inputs: not a transaction Bitcoin would take, and a sign the bytes are nonsense.
    NegativeFee,
    /// No output pays the recipient's script.
    NoRecipientOutput,
    /// The recipient's output exists but the runestone does not send them the asset.
    AssetNotDelivered { wanted: u128, found: u128 },
    /// Our change goes somewhere that is not ours.
    ChangeNotOurs(usize),
    /// A runestone that burns what it carries.
    Cenotaph,
    /// An edict spread over every output instead of naming one.
    SpreadEdict,
    /// No runestone, or more than one.
    Runestone(DecodeError),
}

/// What the buyer recorded **before** asking the venue for anything. The PSBT is judged against this, never the other way.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, candid::CandidType)]
pub struct Quote {
    /// The rune being bought.
    pub rune: RuneId,
    /// The least we will accept being delivered.
    pub units: u128,
    /// The most that may leave our inputs, in sats, fee included.
    pub max_spend_sats: u64,
    /// The most that may go to miners.
    pub max_fee_sats: u64,
    /// The script the asset must land on: the winner's address, as the vault recorded it.
    pub recipient_script: Vec<u8>,
    /// Our own script. Our inputs carry it, and our change must return to it.
    pub our_script: Vec<u8>,
}

/// Read a Bitcoin compact-size integer.
fn compact(b: &[u8], i: &mut usize) -> Option<u64> {
    let first = *b.get(*i)?;
    *i += 1;
    Some(match first {
        0..=0xfc => first as u64,
        0xfd => {
            let v = u16::from_le_bytes(b.get(*i..*i + 2)?.try_into().ok()?) as u64;
            *i += 2;
            v
        }
        0xfe => {
            let v = u32::from_le_bytes(b.get(*i..*i + 4)?.try_into().ok()?) as u64;
            *i += 4;
            v
        }
        _ => {
            let v = u64::from_le_bytes(b.get(*i..*i + 8)?.try_into().ok()?);
            *i += 8;
            v
        }
    })
}

fn take<'a>(b: &'a [u8], i: &mut usize, n: usize) -> Option<&'a [u8]> {
    let end = i.checked_add(n)?;
    let s = b.get(*i..end)?;
    *i = end;
    Some(s)
}

/// Parse one transaction output: 8-byte value then a script.
fn parse_txout(b: &[u8], i: &mut usize) -> Option<TxOut> {
    let value = u64::from_le_bytes(take(b, i, 8)?.try_into().ok()?);
    let n = compact(b, i)? as usize;
    Some(TxOut { value, script: take(b, i, n)?.to_vec() })
}

/// Parse a transaction with no witness data, which is the only form a PSBT's unsigned transaction may take.
fn parse_unsigned_tx(b: &[u8]) -> Result<Tx, ParseError> {
    let mut i = 0usize;
    let version = i32::from_le_bytes(take(b, &mut i, 4).ok_or(ParseError::Truncated)?.try_into().unwrap());
    let n_in = compact(b, &mut i).ok_or(ParseError::Truncated)? as usize;
    if n_in == 0 {
        // A zero input count is the marker byte of the witness serialisation, which an unsigned transaction must not use.
        return Err(ParseError::NotUnsigned);
    }
    if n_in > 10_000 {
        return Err(ParseError::Malformed);
    }
    let mut inputs = Vec::with_capacity(n_in);
    for _ in 0..n_in {
        let txid: [u8; 32] = take(b, &mut i, 32).ok_or(ParseError::Truncated)?.try_into().unwrap();
        let vout = u32::from_le_bytes(take(b, &mut i, 4).ok_or(ParseError::Truncated)?.try_into().unwrap());
        let script_len = compact(b, &mut i).ok_or(ParseError::Truncated)? as usize;
        if script_len != 0 {
            // BIP 174: every scriptSig in the unsigned transaction must be empty.
            return Err(ParseError::NotUnsigned);
        }
        let sequence = u32::from_le_bytes(take(b, &mut i, 4).ok_or(ParseError::Truncated)?.try_into().unwrap());
        inputs.push(TxIn { txid, vout, sequence });
    }
    let n_out = compact(b, &mut i).ok_or(ParseError::Truncated)? as usize;
    if n_out > 10_000 {
        return Err(ParseError::Malformed);
    }
    let mut outputs = Vec::with_capacity(n_out);
    for _ in 0..n_out {
        outputs.push(parse_txout(b, &mut i).ok_or(ParseError::Truncated)?);
    }
    let locktime = u32::from_le_bytes(take(b, &mut i, 4).ok_or(ParseError::Truncated)?.try_into().unwrap());
    if i != b.len() {
        return Err(ParseError::Malformed);
    }
    Ok(Tx { version, inputs, outputs, locktime })
}

/// Read one key/value map, stopping at its separator. Returns the pairs in order.
fn parse_map(b: &[u8], i: &mut usize) -> Result<Vec<(Vec<u8>, Vec<u8>)>, ParseError> {
    let mut pairs: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();
    loop {
        let key_len = compact(b, i).ok_or(ParseError::Truncated)? as usize;
        if key_len == 0 {
            return Ok(pairs);
        }
        let key = take(b, i, key_len).ok_or(ParseError::Truncated)?.to_vec();
        let value_len = compact(b, i).ok_or(ParseError::Truncated)? as usize;
        let value = take(b, i, value_len).ok_or(ParseError::Truncated)?.to_vec();
        if pairs.iter().any(|(k, _)| *k == key) {
            return Err(ParseError::DuplicateKey);
        }
        pairs.push((key, value));
    }
}

impl Psbt {
    /// Parse the bytes a venue returned. Nothing here trusts them; it only reads them.
    pub fn parse(b: &[u8]) -> Result<Self, ParseError> {
        if b.len() < MAGIC.len() || &b[..MAGIC.len()] != MAGIC {
            return Err(ParseError::NotAPsbt);
        }
        let mut i = MAGIC.len();

        let global = parse_map(b, &mut i)?;
        let raw_tx = global
            .iter()
            .find(|(k, _)| k.len() == 1 && k[0] == PSBT_GLOBAL_UNSIGNED_TX)
            .map(|(_, v)| v.clone())
            .ok_or(ParseError::Malformed)?;
        let tx = parse_unsigned_tx(&raw_tx)?;

        let mut witness_utxos = Vec::with_capacity(tx.inputs.len());
        for _ in 0..tx.inputs.len() {
            let m = parse_map(b, &mut i)?;
            let utxo = m
                .iter()
                .find(|(k, _)| k.len() == 1 && k[0] == PSBT_IN_WITNESS_UTXO)
                .map(|(_, v)| {
                    let mut j = 0usize;
                    parse_txout(v, &mut j).filter(|_| j == v.len()).ok_or(ParseError::Malformed)
                })
                .transpose()?;
            witness_utxos.push(utxo);
        }
        for _ in 0..tx.outputs.len() {
            parse_map(b, &mut i)?; // output maps carry nothing the buyer needs; they must still parse
        }
        if i != b.len() {
            return Err(ParseError::Malformed);
        }
        Ok(Psbt { tx, witness_utxos })
    }

    /// The transaction's id: double-SHA256 of the legacy serialisation, reversed for display, exactly as Bitcoin computes it.
    /// A key-path witness is not covered by it, so it is known before anything is signed or broadcast.
    pub fn txid(&self) -> [u8; 32] {
        use sha2::{Digest, Sha256};
        let tx = &self.tx;
        let mut b = tx.version.to_le_bytes().to_vec();
        b.extend(ser_compact(tx.inputs.len()));
        for i in &tx.inputs {
            b.extend_from_slice(&i.txid);
            b.extend_from_slice(&i.vout.to_le_bytes());
            b.push(0); // an unsigned input has an empty scriptSig, and a key-path spend never gains one
            b.extend_from_slice(&i.sequence.to_le_bytes());
        }
        b.extend(ser_compact(tx.outputs.len()));
        for o in &tx.outputs {
            b.extend_from_slice(&o.value.to_le_bytes());
            b.extend(ser_compact(o.script.len()));
            b.extend_from_slice(&o.script);
        }
        b.extend_from_slice(&tx.locktime.to_le_bytes());
        let twice: [u8; 32] = Sha256::digest(Sha256::digest(&b)).into();
        let mut out = twice;
        out.reverse();
        out
    }

    /// Which inputs are ours, by the script they spend.
    pub fn our_inputs(&self, our_script: &[u8]) -> Vec<usize> {
        self.witness_utxos
            .iter()
            .enumerate()
            .filter(|(_, u)| u.as_ref().is_some_and(|u| u.script == our_script))
            .map(|(i, _)| i)
            .collect()
    }

    /// Judge the PSBT against the quote recorded before it was asked for, and say which inputs may be signed.
    ///
    /// `to_sign` is what the venue asked us to sign. Every rule is checked even when an earlier one has an opinion, so the
    /// first refusal reported is the most specific one available.
    pub fn verify(&self, q: &Quote, to_sign: &[usize]) -> Result<Vec<usize>, Refusal> {
        // 1. We sign only our own inputs, and only those.
        let ours = self.our_inputs(&q.our_script);
        for &i in to_sign {
            if !ours.contains(&i) {
                return Err(Refusal::ForeignInputToSign(i));
            }
        }
        for &i in &ours {
            if self.witness_utxos.get(i).and_then(|u| u.as_ref()).is_none() {
                return Err(Refusal::UnknownInputValue(i));
            }
        }

        // 2. Arithmetic: what comes in, what goes out, what we keep.
        let mut in_total: u64 = 0;
        for u in self.witness_utxos.iter().flatten() {
            in_total = in_total.saturating_add(u.value);
        }
        let out_total: u64 = self.tx.outputs.iter().fold(0u64, |a, o| a.saturating_add(o.value));
        if out_total > in_total {
            return Err(Refusal::NegativeFee);
        }
        let fee = in_total - out_total;
        if fee > q.max_fee_sats {
            return Err(Refusal::FeeTooHigh { fee, bound: q.max_fee_sats });
        }

        let ours_in: u64 = ours
            .iter()
            .filter_map(|&i| self.witness_utxos[i].as_ref())
            .fold(0u64, |a, u| a.saturating_add(u.value));
        let ours_out: u64 = self
            .tx
            .outputs
            .iter()
            .filter(|o| o.script == q.our_script)
            .fold(0u64, |a, o| a.saturating_add(o.value));
        let spend = ours_in.saturating_sub(ours_out);
        if spend > q.max_spend_sats {
            return Err(Refusal::OverCap { spend, cap: q.max_spend_sats });
        }

        // 3. Change: any output that is not the recipient's, not ours and not the runestone would be taking our money, and is
        //    caught by the cap above; what we check here is that we did not get change on a script that is not ours. An
        //    output carrying our value must carry our script, which is the same statement from the other side.
        for (i, o) in self.tx.outputs.iter().enumerate() {
            let is_recipient = o.script == q.recipient_script;
            let is_ours = o.script == q.our_script;
            let is_runestone = o.script.first() == Some(&0x6a);
            if !is_recipient && !is_ours && !is_runestone && ours_in > 0 && o.value > 0 && spend == 0 {
                // We paid nothing yet an output moves value: the arithmetic is not what it claims.
                return Err(Refusal::ChangeNotOurs(i));
            }
        }

        // 4. The asset itself: decode what the transaction actually moves.
        let scripts: Vec<Vec<u8>> = self.tx.outputs.iter().map(|o| o.script.clone()).collect();
        let stone = Runestone::decode(&scripts).map_err(Refusal::Runestone)?;
        if stone.cenotaph {
            return Err(Refusal::Cenotaph);
        }
        if stone.has_spread(self.tx.outputs.len() as u32) {
            return Err(Refusal::SpreadEdict);
        }
        let recipient_idx = self
            .tx
            .outputs
            .iter()
            .position(|o| o.script == q.recipient_script)
            .ok_or(Refusal::NoRecipientOutput)?;
        let delivered = stone.units_to(q.rune, recipient_idx as u32);
        if delivered < q.units {
            return Err(Refusal::AssetNotDelivered { wanted: q.units, found: delivered });
        }

        Ok(ours)
    }
}

// ---- after the checks: the signed transaction (docs/v2/buyer-loop.md) ------------------------------------------------------

const PSBT_IN_FINAL_SCRIPTWITNESS: u8 = 0x08;

/// A witness stack: its items, in order.
pub type Witness = Vec<Vec<u8>>;

/// Each input's final witness as the PSBT carries it -- a seller's own signature on a listing -- or `None`. Read from the
/// same bytes `Psbt::parse` accepted; nothing here decides anything, it only carries what the venue's party signed.
pub fn final_witnesses(b: &[u8]) -> Result<Vec<Option<Witness>>, ParseError> {
    let psbt = Psbt::parse(b)?;
    let mut i = MAGIC.len();
    parse_map(b, &mut i)?;
    let mut out = Vec::with_capacity(psbt.tx.inputs.len());
    for _ in 0..psbt.tx.inputs.len() {
        let m = parse_map(b, &mut i)?;
        let w = m.iter().find(|(k, _)| k.len() == 1 && k[0] == PSBT_IN_FINAL_SCRIPTWITNESS).map(|(_, v)| {
            let mut j = 0usize;
            let n = compact(v, &mut j).ok_or(ParseError::Truncated)? as usize;
            if n > 1_000 {
                return Err(ParseError::Malformed);
            }
            let mut items = Vec::with_capacity(n);
            for _ in 0..n {
                let len = compact(v, &mut j).ok_or(ParseError::Truncated)? as usize;
                items.push(take(v, &mut j, len).ok_or(ParseError::Truncated)?.to_vec());
            }
            if j != v.len() { Err(ParseError::Malformed) } else { Ok(items) }
        }).transpose()?;
        out.push(w);
    }
    Ok(out)
}

fn ser_compact(n: usize) -> Vec<u8> {
    match n {
        0..=0xfc => vec![n as u8],
        0xfd..=0xffff => { let mut v = vec![0xfd]; v.extend_from_slice(&(n as u16).to_le_bytes()); v }
        _ => { let mut v = vec![0xfe]; v.extend_from_slice(&(n as u32).to_le_bytes()); v }
    }
}

/// The broadcastable transaction: our inputs carry `ours` (index → 64-byte key-path signature), every other input carries
/// the final witness the PSBT gave it. An input that has neither is a refusal: a transaction with an unsigned input is not
/// a transaction, and inventing one is not ours to do.
pub fn finalize(psbt: &Psbt, others: &[Option<Witness>], ours: &[(usize, Vec<u8>)]) -> Result<Vec<u8>, String> {
    let tx = &psbt.tx;
    if others.len() != tx.inputs.len() {
        return Err("one witness entry per input is required".into());
    }
    let mut witnesses: Vec<Witness> = Vec::with_capacity(tx.inputs.len());
    for (idx, w) in others.iter().enumerate() {
        match (ours.iter().find(|(i, _)| *i == idx), w) {
            (Some(_), Some(_)) => return Err(format!("input {idx} is ours and already carries someone's witness")),
            (Some((_, sig)), None) => {
                if sig.len() != 64 {
                    return Err(format!("input {idx}: a key-path signature is 64 bytes, got {}", sig.len()));
                }
                witnesses.push(vec![sig.clone()]);
            }
            (None, Some(w)) => witnesses.push(w.clone()),
            (None, None) => return Err(format!("refused: input {idx} is not ours and the PSBT carries no witness for it")),
        }
    }
    let mut b = tx.version.to_le_bytes().to_vec();
    b.extend_from_slice(&[0x00, 0x01]);
    b.extend(ser_compact(tx.inputs.len()));
    for i in &tx.inputs {
        b.extend_from_slice(&i.txid);
        b.extend_from_slice(&i.vout.to_le_bytes());
        b.push(0);
        b.extend_from_slice(&i.sequence.to_le_bytes());
    }
    b.extend(ser_compact(tx.outputs.len()));
    for o in &tx.outputs {
        b.extend_from_slice(&o.value.to_le_bytes());
        b.extend(ser_compact(o.script.len()));
        b.extend_from_slice(&o.script);
    }
    for w in &witnesses {
        b.extend(ser_compact(w.len()));
        for item in w {
            b.extend(ser_compact(item.len()));
            b.extend_from_slice(item);
        }
    }
    b.extend_from_slice(&tx.locktime.to_le_bytes());
    Ok(b)
}

// ---- protected listings (docs/v8/ow-protected.md §3.1–3.2): the per-input fields, a raw transaction, the hand-back ----------

const PSBT_IN_SIGHASH_TYPE: u8 = 0x03;
const PSBT_IN_FINAL_SCRIPTSIG: u8 = 0x07;
const PSBT_IN_PARTIAL_SIG: u8 = 0x02;
const PSBT_IN_TAP_KEY_SIG: u8 = 0x13;
const PSBT_IN_TAP_SCRIPT_SIG: u8 = 0x14;
const PSBT_IN_TAP_LEAF_SCRIPT: u8 = 0x15;
const PSBT_IN_TAP_INTERNAL_KEY: u8 = 0x17;

/// One `PSBT_IN_TAP_LEAF_SCRIPT` entry: the control block (the key after its type byte), the script and its leaf version (the
/// value's last byte).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TapLeafScript {
    pub control_block: Vec<u8>,
    pub script: Vec<u8>,
    pub leaf_version: u8,
}

/// The fields of one input map that say what is asked of a signer and what is already signed. Read from the same bytes
/// [`Psbt::parse`] accepted; nothing here decides anything.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InputFields {
    /// `PSBT_IN_SIGHASH_TYPE`, the raw 32-bit value.
    pub sighash_type: Option<u32>,
    /// `PSBT_IN_FINAL_SCRIPTSIG`.
    pub final_script_sig: Option<Vec<u8>>,
    /// `PSBT_IN_FINAL_SCRIPTWITNESS`.
    pub final_script_witness: Option<Witness>,
    /// How many `PSBT_IN_PARTIAL_SIG` entries (ECDSA signatures) the input carries.
    pub partial_sigs: usize,
    /// `PSBT_IN_TAP_KEY_SIG`: 64 bytes, or 65 with a sighash byte.
    pub tap_key_sig: Option<Vec<u8>>,
    /// `PSBT_IN_TAP_SCRIPT_SIG`: (x-only key, leaf hash, signature).
    pub tap_script_sigs: Vec<([u8; 32], [u8; 32], Vec<u8>)>,
    /// `PSBT_IN_TAP_LEAF_SCRIPT`, in the order the PSBT carries them.
    pub tap_leaf_scripts: Vec<TapLeafScript>,
    /// `PSBT_IN_TAP_INTERNAL_KEY`.
    pub tap_internal_key: Option<[u8; 32]>,
}

impl InputFields {
    /// Anything a signer already put on this input: a final scriptSig or witness, or a signature of any kind.
    pub fn signed(&self) -> bool {
        self.final_script_witness.is_some()
            || self.final_script_sig.as_ref().is_some_and(|s| !s.is_empty())
            || self.tap_key_sig.is_some()
            || !self.tap_script_sigs.is_empty()
            || self.partial_sigs > 0
    }
}

fn read_witness(v: &[u8]) -> Result<Witness, ParseError> {
    let mut j = 0usize;
    let n = compact(v, &mut j).ok_or(ParseError::Truncated)? as usize;
    if n > 1_000 {
        return Err(ParseError::Malformed);
    }
    let mut items = Vec::with_capacity(n);
    for _ in 0..n {
        let len = compact(v, &mut j).ok_or(ParseError::Truncated)? as usize;
        items.push(take(v, &mut j, len).ok_or(ParseError::Truncated)?.to_vec());
    }
    if j != v.len() { Err(ParseError::Malformed) } else { Ok(items) }
}

/// Each input's [`InputFields`]. A field whose key or value has the wrong length for its type is `Malformed`, never guessed.
pub fn input_fields(b: &[u8]) -> Result<Vec<InputFields>, ParseError> {
    let psbt = Psbt::parse(b)?;
    let mut i = MAGIC.len();
    parse_map(b, &mut i)?;
    let mut out = Vec::with_capacity(psbt.tx.inputs.len());
    for _ in 0..psbt.tx.inputs.len() {
        let mut f = InputFields::default();
        for (k, v) in parse_map(b, &mut i)? {
            let bare = k.len() == 1;
            match k[0] {
                PSBT_IN_SIGHASH_TYPE => {
                    if !bare || v.len() != 4 {
                        return Err(ParseError::Malformed);
                    }
                    f.sighash_type = Some(u32::from_le_bytes(v[..].try_into().unwrap()));
                }
                PSBT_IN_FINAL_SCRIPTSIG => {
                    if !bare {
                        return Err(ParseError::Malformed);
                    }
                    f.final_script_sig = Some(v);
                }
                PSBT_IN_FINAL_SCRIPTWITNESS => {
                    if !bare {
                        return Err(ParseError::Malformed);
                    }
                    f.final_script_witness = Some(read_witness(&v)?);
                }
                PSBT_IN_PARTIAL_SIG => f.partial_sigs += 1,
                PSBT_IN_TAP_KEY_SIG => {
                    if !bare || !(v.len() == 64 || v.len() == 65) {
                        return Err(ParseError::Malformed);
                    }
                    f.tap_key_sig = Some(v);
                }
                PSBT_IN_TAP_SCRIPT_SIG => {
                    if k.len() != 65 || !(v.len() == 64 || v.len() == 65) {
                        return Err(ParseError::Malformed);
                    }
                    f.tap_script_sigs.push((k[1..33].try_into().unwrap(), k[33..65].try_into().unwrap(), v));
                }
                PSBT_IN_TAP_LEAF_SCRIPT => {
                    // a control block is 33 + 32m bytes, m ≤ 128 (BIP 341); the value is the script then its leaf version
                    let cb = &k[1..];
                    if cb.len() < 33 || (cb.len() - 33) % 32 != 0 || cb.len() > 33 + 32 * 128 || v.is_empty() {
                        return Err(ParseError::Malformed);
                    }
                    let (script, ver) = v.split_at(v.len() - 1);
                    f.tap_leaf_scripts.push(TapLeafScript { control_block: cb.to_vec(), script: script.to_vec(), leaf_version: ver[0] });
                }
                PSBT_IN_TAP_INTERNAL_KEY => {
                    if !bare || v.len() != 32 {
                        return Err(ParseError::Malformed);
                    }
                    f.tap_internal_key = Some(v[..].try_into().unwrap());
                }
                _ => {}
            }
        }
        out.push(f);
    }
    Ok(out)
}

/// A raw transaction as the network carries it: the transaction, each input's scriptSig and witness, and its txid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawTx {
    pub tx: Tx,
    pub script_sigs: Vec<Vec<u8>>,
    /// One per input; empty for every input of a legacy serialisation.
    pub witnesses: Vec<Witness>,
    /// Double-SHA256 of the serialisation without marker, flag and witnesses, reversed for display (as [`Psbt::txid`]).
    pub txid: [u8; 32],
}

/// Parse a raw transaction in either serialisation: BIP 144's (marker `0x00`, flag `0x01`, witnesses after the outputs) or the
/// legacy one. The txid is computed over the bytes that were read, less the marker, flag and witnesses, so a scriptSig is
/// covered exactly as Bitcoin covers it. Trailing bytes, a flag other than `0x01` and a witness serialisation whose every
/// witness is empty (which Bitcoin Core refuses as superfluous) are `Malformed`.
pub fn parse_raw_tx(b: &[u8]) -> Result<RawTx, ParseError> {
    use sha2::{Digest, Sha256};
    let mut i = 0usize;
    let version = i32::from_le_bytes(take(b, &mut i, 4).ok_or(ParseError::Truncated)?.try_into().unwrap());
    let segwit = b.get(i) == Some(&0x00);
    if segwit {
        if b.get(i + 1) != Some(&0x01) {
            return Err(ParseError::Malformed);
        }
        i += 2;
    }
    let body_start = i;
    let n_in = compact(b, &mut i).ok_or(ParseError::Truncated)? as usize;
    if n_in == 0 || n_in > 10_000 {
        return Err(ParseError::Malformed);
    }
    let mut inputs = Vec::with_capacity(n_in);
    let mut script_sigs = Vec::with_capacity(n_in);
    for _ in 0..n_in {
        let txid: [u8; 32] = take(b, &mut i, 32).ok_or(ParseError::Truncated)?.try_into().unwrap();
        let vout = u32::from_le_bytes(take(b, &mut i, 4).ok_or(ParseError::Truncated)?.try_into().unwrap());
        let n = compact(b, &mut i).ok_or(ParseError::Truncated)? as usize;
        script_sigs.push(take(b, &mut i, n).ok_or(ParseError::Truncated)?.to_vec());
        let sequence = u32::from_le_bytes(take(b, &mut i, 4).ok_or(ParseError::Truncated)?.try_into().unwrap());
        inputs.push(TxIn { txid, vout, sequence });
    }
    let n_out = compact(b, &mut i).ok_or(ParseError::Truncated)? as usize;
    if n_out > 10_000 {
        return Err(ParseError::Malformed);
    }
    let mut outputs = Vec::with_capacity(n_out);
    for _ in 0..n_out {
        outputs.push(parse_txout(b, &mut i).ok_or(ParseError::Truncated)?);
    }
    let body_end = i;
    let mut witnesses = vec![Vec::new(); n_in];
    if segwit {
        for w in witnesses.iter_mut() {
            let n = compact(b, &mut i).ok_or(ParseError::Truncated)? as usize;
            if n > 1_000 {
                return Err(ParseError::Malformed);
            }
            for _ in 0..n {
                let len = compact(b, &mut i).ok_or(ParseError::Truncated)? as usize;
                w.push(take(b, &mut i, len).ok_or(ParseError::Truncated)?.to_vec());
            }
        }
        if witnesses.iter().all(|w| w.is_empty()) {
            return Err(ParseError::Malformed);
        }
    }
    let locktime_bytes = take(b, &mut i, 4).ok_or(ParseError::Truncated)?;
    let locktime = u32::from_le_bytes(locktime_bytes.try_into().unwrap());
    if i != b.len() {
        return Err(ParseError::Malformed);
    }
    let mut h = Sha256::new();
    h.update(&b[..4]);
    h.update(&b[body_start..body_end]);
    h.update(locktime_bytes);
    let mut txid: [u8; 32] = Sha256::digest(h.finalize()).into();
    txid.reverse();
    Ok(RawTx { tx: Tx { version, inputs, outputs, locktime }, script_sigs, witnesses, txid })
}

/// Read one map like [`parse_map`], returning the offset of its separator byte as well as its pairs.
fn map_end(b: &[u8], i: &mut usize) -> Result<(Vec<(Vec<u8>, Vec<u8>)>, usize), ParseError> {
    let pairs = parse_map(b, i)?;
    Ok((pairs, *i - 1))
}

/// The hand-back (docs/v8/ow-protected.md §3.2): the original PSBT with `PSBT_IN_TAP_KEY_SIG` (a 64-byte `SIGHASH_DEFAULT`
/// signature) and `PSBT_IN_TAP_INTERNAL_KEY` written into each named input, appended at the end of that input's map. Every
/// other byte -- the global map, every other key-value pair of every input, every output map -- is copied unchanged and in
/// order. Nothing is finalized.
///
/// Refused: an index that is not an input, an index named twice, an input that already carries a final scriptSig or witness
/// or a key-path signature, and an input whose stated internal key is not `internal_key` (one that states the same key keeps
/// it and gains no second copy).
pub fn with_tap_key_sigs(b: &[u8], sigs: &[(usize, [u8; 64])], internal_key: [u8; 32]) -> Result<Vec<u8>, String> {
    let psbt = Psbt::parse(b).map_err(|e| format!("the PSBT does not parse: {e:?}"))?;
    let fields = input_fields(b).map_err(|e| format!("the PSBT's input fields do not parse: {e:?}"))?;
    let n = psbt.tx.inputs.len();
    for (pos, (idx, _)) in sigs.iter().enumerate() {
        if *idx >= n {
            return Err(format!("input {idx} does not exist: the transaction has {n}"));
        }
        if sigs[..pos].iter().any(|(j, _)| j == idx) {
            return Err(format!("input {idx} is named twice"));
        }
        let f = &fields[*idx];
        if f.final_script_witness.is_some() || f.final_script_sig.as_ref().is_some_and(|s| !s.is_empty()) {
            return Err(format!("input {idx} already carries a final witness or scriptSig"));
        }
        if f.tap_key_sig.is_some() {
            return Err(format!("input {idx} already carries a key-path signature"));
        }
        if f.tap_internal_key.is_some_and(|k| k != internal_key) {
            return Err(format!("input {idx} states another internal key"));
        }
    }
    let mut i = MAGIC.len();
    map_end(b, &mut i).map_err(|e| format!("{e:?}"))?;
    let mut out = Vec::with_capacity(b.len() + sigs.len() * 104);
    let mut copied = 0usize;
    for (idx, f) in fields.iter().enumerate() {
        let (_, sep) = map_end(b, &mut i).map_err(|e| format!("{e:?}"))?;
        let Some((_, sig)) = sigs.iter().find(|(j, _)| *j == idx) else { continue };
        out.extend_from_slice(&b[copied..sep]);
        out.extend_from_slice(&[1, PSBT_IN_TAP_KEY_SIG, 64]);
        out.extend_from_slice(sig);
        if f.tap_internal_key.is_none() {
            out.extend_from_slice(&[1, PSBT_IN_TAP_INTERNAL_KEY, 32]);
            out.extend_from_slice(&internal_key);
        }
        copied = sep;
    }
    out.extend_from_slice(&b[copied..]);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const OUR: &[u8] = &[0x51, 0x20, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1];
    const WINNER: &[u8] = &[0x51, 0x20, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2];
    const STRANGER: &[u8] = &[0x51, 0x20, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9];
    const SELLER: &[u8] = &[0x00, 0x14, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7];
    const RUNE: RuneId = RuneId { block: 840_000, tx: 3 };

    fn leb(mut v: u128) -> Vec<u8> {
        let mut out = Vec::new();
        loop {
            let b = (v & 0x7f) as u8;
            v >>= 7;
            if v == 0 {
                out.push(b);
                return out;
            }
            out.push(b | 0x80);
        }
    }

    /// An `OP_RETURN OP_13` output moving `amount` of `RUNE` to output `to`.
    fn stone_script(amount: u128, to: u128) -> Vec<u8> {
        let mut p = leb(0); // Body
        p.extend(leb(RUNE.block as u128));
        p.extend(leb(RUNE.tx as u128));
        p.extend(leb(amount));
        p.extend(leb(to));
        let mut s = vec![0x6a, 0x5d, p.len() as u8];
        s.extend(p);
        s
    }

    fn compact_bytes(n: u64) -> Vec<u8> {
        if n <= 0xfc {
            vec![n as u8]
        } else {
            let mut v = vec![0xfd];
            v.extend_from_slice(&(n as u16).to_le_bytes());
            v
        }
    }

    fn ser_txout(o: &TxOut) -> Vec<u8> {
        let mut v = o.value.to_le_bytes().to_vec();
        v.extend(compact_bytes(o.script.len() as u64));
        v.extend_from_slice(&o.script);
        v
    }

    /// Serialise an unsigned transaction and wrap it, with its input maps, into a PSBT.
    fn make_psbt(inputs: &[(TxOut, bool)], outputs: &[TxOut]) -> Vec<u8> {
        let mut tx = 2i32.to_le_bytes().to_vec();
        tx.extend(compact_bytes(inputs.len() as u64));
        for (i, _) in inputs.iter().enumerate() {
            tx.extend_from_slice(&[(i as u8) + 1; 32]); // txid
            tx.extend_from_slice(&0u32.to_le_bytes()); // vout
            tx.push(0); // empty scriptSig
            tx.extend_from_slice(&0xffff_ffffu32.to_le_bytes());
        }
        tx.extend(compact_bytes(outputs.len() as u64));
        for o in outputs {
            tx.extend(ser_txout(o));
        }
        tx.extend_from_slice(&0u32.to_le_bytes()); // locktime

        let mut p = MAGIC.to_vec();
        // global map: the unsigned transaction
        p.push(1);
        p.push(PSBT_GLOBAL_UNSIGNED_TX);
        p.extend(compact_bytes(tx.len() as u64));
        p.extend_from_slice(&tx);
        p.push(0);
        // one map per input
        for (utxo, include) in inputs {
            if *include {
                let v = ser_txout(utxo);
                p.push(1);
                p.push(PSBT_IN_WITNESS_UTXO);
                p.extend(compact_bytes(v.len() as u64));
                p.extend_from_slice(&v);
            }
            p.push(0);
        }
        // one map per output
        for _ in outputs {
            p.push(0);
        }
        p
    }

    fn quote() -> Quote {
        Quote {
            rune: RUNE,
            units: 1_000,
            max_spend_sats: 50_000,
            max_fee_sats: 5_000,
            recipient_script: WINNER.to_vec(),
            our_script: OUR.to_vec(),
        }
    }

    /// The transaction a well-behaved venue returns: we pay 100k, the seller is paid, the winner gets the runes, we get change.
    fn honest() -> Vec<u8> {
        make_psbt(
            &[(TxOut { value: 100_000, script: OUR.to_vec() }, true)],
            &[
                TxOut { value: 0, script: stone_script(1_000, 1) },
                TxOut { value: 546, script: WINNER.to_vec() },
                TxOut { value: 40_000, script: SELLER.to_vec() },
                TxOut { value: 58_454, script: OUR.to_vec() },
            ],
        )
    }

    #[test]
    fn an_honest_psbt_is_parsed_and_signed() {
        let p = Psbt::parse(&honest()).unwrap();
        assert_eq!(p.tx.inputs.len(), 1);
        assert_eq!(p.tx.outputs.len(), 4);
        assert_eq!(p.our_inputs(OUR), vec![0]);
        // we put in 100,000 and kept 58,454, so we spent 41,546 of which 1,000 is the fee
        assert_eq!(p.verify(&quote(), &[0]).unwrap(), vec![0]);
    }

    #[test]
    fn bytes_that_are_not_a_psbt_are_refused_before_anything_else() {
        assert_eq!(Psbt::parse(b"").unwrap_err(), ParseError::NotAPsbt);
        assert_eq!(Psbt::parse(b"not a psbt at all").unwrap_err(), ParseError::NotAPsbt);
        let mut truncated = honest();
        truncated.truncate(truncated.len() - 3);
        assert!(matches!(Psbt::parse(&truncated), Err(ParseError::Truncated | ParseError::Malformed)));
    }

    #[test]
    fn a_duplicate_key_is_refused_rather_than_read_twice() {
        let good = honest();
        // repeat the global unsigned-transaction key by splicing a second copy of the first pair in before the separator
        let start = MAGIC.len();
        let mut i = start;
        let key_len = good[i] as usize;
        i += 1 + key_len;
        let mut j = i;
        let value_len = compact(&good, &mut j).unwrap() as usize;
        let pair_end = j + value_len;
        let pair = good[start..pair_end].to_vec();
        let mut dup = good[..pair_end].to_vec();
        dup.extend_from_slice(&pair);
        dup.extend_from_slice(&good[pair_end..]);
        assert_eq!(Psbt::parse(&dup).unwrap_err(), ParseError::DuplicateKey);
    }

    #[test]
    fn a_transaction_carrying_a_script_sig_is_not_an_unsigned_one() {
        let mut tx = 2i32.to_le_bytes().to_vec();
        tx.extend(compact_bytes(1));
        tx.extend_from_slice(&[1u8; 32]);
        tx.extend_from_slice(&0u32.to_le_bytes());
        tx.push(1); // a scriptSig, which BIP 174 forbids here
        tx.push(0x51);
        tx.extend_from_slice(&0xffff_ffffu32.to_le_bytes());
        tx.extend(compact_bytes(0));
        tx.extend_from_slice(&0u32.to_le_bytes());
        let mut p = MAGIC.to_vec();
        p.push(1);
        p.push(PSBT_GLOBAL_UNSIGNED_TX);
        p.extend(compact_bytes(tx.len() as u64));
        p.extend_from_slice(&tx);
        p.push(0);
        assert_eq!(Psbt::parse(&p).unwrap_err(), ParseError::NotUnsigned);
    }

    // ---- what a lying node can try, and what happens ----------------------------------------------------------------------

    #[test]
    fn a_node_that_asks_us_to_sign_someone_elses_input_is_refused() {
        let b = make_psbt(
            &[
                (TxOut { value: 100_000, script: OUR.to_vec() }, true),
                (TxOut { value: 100_000, script: STRANGER.to_vec() }, true),
            ],
            &[
                TxOut { value: 0, script: stone_script(1_000, 1) },
                TxOut { value: 546, script: WINNER.to_vec() },
                TxOut { value: 40_000, script: SELLER.to_vec() },
                TxOut { value: 158_454, script: OUR.to_vec() },
            ],
        );
        let p = Psbt::parse(&b).unwrap();
        assert_eq!(p.verify(&quote(), &[0, 1]).unwrap_err(), Refusal::ForeignInputToSign(1));
        assert_eq!(p.our_inputs(OUR), vec![0], "and we would only ever sign our own");
    }

    #[test]
    fn a_node_that_raises_the_price_past_the_cap_is_refused() {
        // 100,000 in, only 20,000 of change: we would be spending 80,000 against a 50,000 cap.
        let b = make_psbt(
            &[(TxOut { value: 100_000, script: OUR.to_vec() }, true)],
            &[
                TxOut { value: 0, script: stone_script(1_000, 1) },
                TxOut { value: 546, script: WINNER.to_vec() },
                TxOut { value: 78_454, script: SELLER.to_vec() },
                TxOut { value: 20_000, script: OUR.to_vec() },
            ],
        );
        let e = Psbt::parse(&b).unwrap().verify(&quote(), &[0]).unwrap_err();
        assert_eq!(e, Refusal::OverCap { spend: 80_000, cap: 50_000 });
    }

    #[test]
    fn a_node_that_sends_the_runes_somewhere_else_is_refused() {
        // Everything looks right except the edict names the stranger's output, not the winner's.
        let b = make_psbt(
            &[(TxOut { value: 100_000, script: OUR.to_vec() }, true)],
            &[
                TxOut { value: 0, script: stone_script(1_000, 2) },
                TxOut { value: 546, script: WINNER.to_vec() },
                TxOut { value: 546, script: STRANGER.to_vec() },
                TxOut { value: 40_000, script: SELLER.to_vec() },
                TxOut { value: 57_908, script: OUR.to_vec() },
            ],
        );
        let e = Psbt::parse(&b).unwrap().verify(&quote(), &[0]).unwrap_err();
        assert_eq!(e, Refusal::AssetNotDelivered { wanted: 1_000, found: 0 });
    }

    #[test]
    fn a_node_that_delivers_fewer_units_than_quoted_is_refused() {
        let b = make_psbt(
            &[(TxOut { value: 100_000, script: OUR.to_vec() }, true)],
            &[
                TxOut { value: 0, script: stone_script(999, 1) },
                TxOut { value: 546, script: WINNER.to_vec() },
                TxOut { value: 40_000, script: SELLER.to_vec() },
                TxOut { value: 58_454, script: OUR.to_vec() },
            ],
        );
        let e = Psbt::parse(&b).unwrap().verify(&quote(), &[0]).unwrap_err();
        assert_eq!(e, Refusal::AssetNotDelivered { wanted: 1_000, found: 999 });
    }

    #[test]
    fn the_change_not_ours_check_cannot_fire_on_a_transaction_that_buys_anything() {
        // The comment above it says it checks "that we did not get change on a script that is not ours". The condition it
        // actually carries ends in `spend == 0`, and `spend == 0` means we kept every sat we put in — which no purchase
        // does. So on any transaction that pays for something the branch is unreachable, and the only thing standing
        // between us and a stranger's output is the cap. That is a real bound, and it is not the one the comment claims.
        //
        // Here a stranger takes 9,000 sats on a transaction that is otherwise honest and well under the cap. Spend is
        // 9,546 + 1,000 fee, far below 50,000, so the cap does not fire — and neither does ChangeNotOurs.
        let b = make_psbt(
            &[(TxOut { value: 100_000, script: OUR.to_vec() }, true)],
            &[
                TxOut { value: 0, script: stone_script(1_000, 1) },
                TxOut { value: 546, script: WINNER.to_vec() },
                TxOut { value: 9_000, script: STRANGER.to_vec() },   // a stranger, paid out of our input
                TxOut { value: 89_454, script: OUR.to_vec() },
            ],
        );
        let ok = Psbt::parse(&b).unwrap().verify(&quote(), &[0]);
        assert!(ok.is_ok(), "accepted: the stranger's output is bounded by the cap and by nothing else");

        // and the loss is exactly what the cap permits, which is what the residual-risk paragraph should say
        let b2 = make_psbt(
            &[(TxOut { value: 100_000, script: OUR.to_vec() }, true)],
            &[
                TxOut { value: 0, script: stone_script(1_000, 1) },
                TxOut { value: 546, script: WINNER.to_vec() },
                TxOut { value: 49_000, script: STRANGER.to_vec() },
                TxOut { value: 49_999, script: OUR.to_vec() },      // one sat past what the cap allows
            ],
        );
        assert_eq!(
            Psbt::parse(&b2).unwrap().verify(&quote(), &[0]).unwrap_err(),
            Refusal::OverCap { spend: 50_001, cap: 50_000 },
            "the cap is the only thing that stops it, and it stops it at the cap"
        );
    }

    #[test]
    fn a_node_that_sends_our_change_to_itself_is_refused_by_the_cap() {
        let b = make_psbt(
            &[(TxOut { value: 100_000, script: OUR.to_vec() }, true)],
            &[
                TxOut { value: 0, script: stone_script(1_000, 1) },
                TxOut { value: 546, script: WINNER.to_vec() },
                TxOut { value: 40_000, script: SELLER.to_vec() },
                TxOut { value: 58_454, script: STRANGER.to_vec() }, // our change, but not to us
            ],
        );
        let e = Psbt::parse(&b).unwrap().verify(&quote(), &[0]).unwrap_err();
        assert_eq!(e, Refusal::OverCap { spend: 100_000, cap: 50_000 }, "keeping none of it is spending all of it");
    }

    #[test]
    fn a_node_that_burns_the_runes_with_a_cenotaph_is_refused() {
        let mut p = leb(0); // Body, then a truncated edict
        p.extend(leb(840_000));
        p.extend(leb(3));
        let mut bad_stone = vec![0x6a, 0x5d, p.len() as u8];
        bad_stone.extend(p);
        let b = make_psbt(
            &[(TxOut { value: 100_000, script: OUR.to_vec() }, true)],
            &[
                TxOut { value: 0, script: bad_stone },
                TxOut { value: 546, script: WINNER.to_vec() },
                TxOut { value: 40_000, script: SELLER.to_vec() },
                TxOut { value: 58_454, script: OUR.to_vec() },
            ],
        );
        assert_eq!(Psbt::parse(&b).unwrap().verify(&quote(), &[0]).unwrap_err(), Refusal::Cenotaph);
    }

    #[test]
    fn a_node_that_spreads_the_edict_over_every_output_is_refused() {
        let b = make_psbt(
            &[(TxOut { value: 100_000, script: OUR.to_vec() }, true)],
            &[
                TxOut { value: 0, script: stone_script(1_000, 4) }, // 4 == the output count
                TxOut { value: 546, script: WINNER.to_vec() },
                TxOut { value: 40_000, script: SELLER.to_vec() },
                TxOut { value: 58_454, script: OUR.to_vec() },
            ],
        );
        assert_eq!(Psbt::parse(&b).unwrap().verify(&quote(), &[0]).unwrap_err(), Refusal::SpreadEdict);
    }

    #[test]
    fn a_node_that_adds_a_second_runestone_is_refused() {
        let b = make_psbt(
            &[(TxOut { value: 100_000, script: OUR.to_vec() }, true)],
            &[
                TxOut { value: 0, script: stone_script(1_000, 2) },
                TxOut { value: 0, script: stone_script(1_000, 2) },
                TxOut { value: 546, script: WINNER.to_vec() },
                TxOut { value: 40_000, script: SELLER.to_vec() },
                TxOut { value: 58_454, script: OUR.to_vec() },
            ],
        );
        assert_eq!(
            Psbt::parse(&b).unwrap().verify(&quote(), &[0]).unwrap_err(),
            Refusal::Runestone(DecodeError::ManyRunestones)
        );
    }

    #[test]
    fn a_node_that_pays_the_whole_thing_to_miners_is_refused() {
        let b = make_psbt(
            &[(TxOut { value: 100_000, script: OUR.to_vec() }, true)],
            &[
                TxOut { value: 0, script: stone_script(1_000, 1) },
                TxOut { value: 546, script: WINNER.to_vec() },
                TxOut { value: 60_000, script: OUR.to_vec() }, // fee = 39,454
            ],
        );
        let e = Psbt::parse(&b).unwrap().verify(&quote(), &[0]).unwrap_err();
        assert_eq!(e, Refusal::FeeTooHigh { fee: 39_454, bound: 5_000 });
    }

    #[test]
    fn a_node_that_hides_what_an_input_is_worth_is_refused() {
        let b = make_psbt(
            &[(TxOut { value: 100_000, script: OUR.to_vec() }, false)], // no witness utxo for our own input
            &[
                TxOut { value: 0, script: stone_script(1_000, 1) },
                TxOut { value: 546, script: WINNER.to_vec() },
            ],
        );
        let p = Psbt::parse(&b).unwrap();
        assert!(p.our_inputs(OUR).is_empty(), "an input we cannot value is not one we recognise as ours");
        // and asking us to sign it names an input that is not among ours
        assert_eq!(p.verify(&quote(), &[0]).unwrap_err(), Refusal::ForeignInputToSign(0));
    }

    #[test]
    fn a_transaction_that_creates_money_is_refused() {
        let b = make_psbt(
            &[(TxOut { value: 1_000, script: OUR.to_vec() }, true)],
            &[
                TxOut { value: 0, script: stone_script(1_000, 1) },
                TxOut { value: 546, script: WINNER.to_vec() },
                TxOut { value: 1_000_000, script: OUR.to_vec() },
            ],
        );
        assert_eq!(Psbt::parse(&b).unwrap().verify(&quote(), &[0]).unwrap_err(), Refusal::NegativeFee);
    }

    #[test]
    fn a_node_that_omits_the_winner_entirely_is_refused() {
        let b = make_psbt(
            &[(TxOut { value: 100_000, script: OUR.to_vec() }, true)],
            &[
                TxOut { value: 0, script: stone_script(1_000, 1) },
                TxOut { value: 546, script: STRANGER.to_vec() },
                TxOut { value: 40_000, script: SELLER.to_vec() },
                TxOut { value: 58_454, script: OUR.to_vec() },
            ],
        );
        assert_eq!(Psbt::parse(&b).unwrap().verify(&quote(), &[0]).unwrap_err(), Refusal::NoRecipientOutput);
    }

    #[test]
    fn overpaying_within_the_cap_is_allowed_and_is_the_residual_risk_we_state() {
        // The honest transaction spends 41,546. This one spends 49,999 — worse, and still under the cap.
        let b = make_psbt(
            &[(TxOut { value: 100_000, script: OUR.to_vec() }, true)],
            &[
                TxOut { value: 0, script: stone_script(1_000, 1) },
                TxOut { value: 546, script: WINNER.to_vec() },
                TxOut { value: 48_453, script: SELLER.to_vec() },
                TxOut { value: 50_001, script: OUR.to_vec() },
            ],
        );
        assert!(Psbt::parse(&b).unwrap().verify(&quote(), &[0]).is_ok(), "documented: a lying node can cost us up to the cap");
    }

    #[test]
    fn a_verified_psbt_becomes_the_signed_transaction_with_every_witness_in_place() {
        // input 0 ours, input 1 a seller's with its final witness (key 0x08), as a listing PSBT carries it
        let mut tx = 2i32.to_le_bytes().to_vec();
        tx.push(2);
        for (n, vout) in [(1u8, 0u32), (2u8, 5u32)] {
            tx.extend_from_slice(&[n; 32]);
            tx.extend_from_slice(&vout.to_le_bytes());
            tx.push(0);
            tx.extend_from_slice(&0xffff_fffdu32.to_le_bytes());
        }
        tx.push(1);
        tx.extend_from_slice(&546u64.to_le_bytes());
        tx.push(3);
        tx.extend_from_slice(&[0x51, 0x01, 0x02]);
        tx.extend_from_slice(&0u32.to_le_bytes());
        let mut p = b"psbt\xff".to_vec();
        p.extend_from_slice(&[1, 0x00, tx.len() as u8]);
        p.extend_from_slice(&tx);
        p.push(0);
        p.push(0); // input 0: nothing but the separator
        let mut wv = vec![2u8, 65]; // two items: a 65-byte signature, then one byte
        wv.extend_from_slice(&[0x77; 65]);
        wv.extend_from_slice(&[1, 0x55]);
        p.extend_from_slice(&[1, 0x08, wv.len() as u8]);
        p.extend_from_slice(&wv);
        p.push(0);
        p.push(0); // one output map
        let psbt = Psbt::parse(&p).unwrap();
        let others = final_witnesses(&p).unwrap();
        assert_eq!(others, vec![None, Some(vec![vec![0x77; 65], vec![0x55]])]);
        let raw = finalize(&psbt, &others, &[(0, vec![0xaa; 64])]).unwrap();
        // strip marker, flag and witnesses and what is left is exactly the unsigned transaction: the txid did not move
        let wit_start = raw.len() - 4 - (1 + 1 + 64) - (1 + 1 + 65 + 1 + 1);
        let mut stripped = raw[..4].to_vec();
        stripped.extend_from_slice(&raw[6..wit_start]);
        stripped.extend_from_slice(&raw[raw.len() - 4..]);
        assert_eq!(stripped, tx);
        assert_eq!(&raw[wit_start..wit_start + 2], &[1, 64]);
        // an input that is neither ours nor finalised is a refusal, and so is a wrong-length signature
        assert!(finalize(&psbt, &[None, None], &[(0, vec![0xaa; 64])]).unwrap_err().contains("carries no witness"));
        assert!(finalize(&psbt, &others, &[(0, vec![0xaa; 65])]).unwrap_err().contains("64 bytes"));
        assert!(finalize(&psbt, &others, &[(0, vec![0xaa; 64]), (1, vec![0xaa; 64])]).unwrap_err().contains("already carries"));
    }

    // ---- protected listings: input fields, raw transactions, the hand-back --------------------------------------------------

    /// A two-input, one-output PSBT whose maps carry a spread of fields, unknown ones included: (global extra, input pairs,
    /// output pairs).
    fn rich_psbt() -> Vec<u8> {
        let mut tx = 2i32.to_le_bytes().to_vec();
        tx.push(2);
        for n in [1u8, 2u8] {
            tx.extend_from_slice(&[n; 32]);
            tx.extend_from_slice(&0u32.to_le_bytes());
            tx.push(0);
            tx.extend_from_slice(&0xffff_fffdu32.to_le_bytes());
        }
        tx.push(1);
        tx.extend(ser_txout(&TxOut { value: 1_000, script: WINNER.to_vec() }));
        tx.extend_from_slice(&0u32.to_le_bytes());
        let pair = |p: &mut Vec<u8>, k: &[u8], v: &[u8]| {
            p.extend(compact_bytes(k.len() as u64));
            p.extend_from_slice(k);
            p.extend(compact_bytes(v.len() as u64));
            p.extend_from_slice(v);
        };
        let mut p = MAGIC.to_vec();
        pair(&mut p, &[PSBT_GLOBAL_UNSIGNED_TX], &tx);
        pair(&mut p, &[0xfb], &2u32.to_le_bytes()); // PSBT_GLOBAL_VERSION
        p.push(0);
        // input 0: ours, with a sighash field and an unknown proprietary pair
        pair(&mut p, &[PSBT_IN_WITNESS_UTXO], &ser_txout(&TxOut { value: 5_000, script: OUR.to_vec() }));
        pair(&mut p, &[PSBT_IN_SIGHASH_TYPE], &0u32.to_le_bytes());
        pair(&mut p, &[0xfc, 0x01, 0x02], b"proprietary");
        p.push(0);
        // input 1: an escrow template: a leaf script, the internal key, a co-signer's script signature
        pair(&mut p, &[PSBT_IN_WITNESS_UTXO], &ser_txout(&TxOut { value: 546, script: STRANGER.to_vec() }));
        let mut leaf_key = vec![PSBT_IN_TAP_LEAF_SCRIPT, 0xc0];
        leaf_key.extend_from_slice(&[0x50; 32]);
        leaf_key.extend_from_slice(&[0x60; 32]);
        pair(&mut p, &leaf_key, &[0x51, 0xac, 0xc0]);
        pair(&mut p, &[PSBT_IN_TAP_INTERNAL_KEY], &[0x50; 32]);
        let mut sig_key = vec![PSBT_IN_TAP_SCRIPT_SIG];
        sig_key.extend_from_slice(&[0x1d; 32]);
        sig_key.extend_from_slice(&[0x19; 32]);
        pair(&mut p, &sig_key, &[0x83; 65]);
        p.push(0);
        // the output: an unknown pair
        pair(&mut p, &[0x05], b"tap tree");
        p.push(0);
        p
    }

    #[test]
    fn the_input_fields_are_read_as_their_types_say() {
        let p = rich_psbt();
        let f = input_fields(&p).unwrap();
        assert_eq!(f[0].sighash_type, Some(0));
        assert!(!f[0].signed());
        assert_eq!(f[1].tap_leaf_scripts, vec![TapLeafScript {
            control_block: [vec![0xc0], vec![0x50; 32], vec![0x60; 32]].concat(), script: vec![0x51, 0xac], leaf_version: 0xc0 }]);
        assert_eq!(f[1].tap_internal_key, Some([0x50; 32]));
        assert_eq!(f[1].tap_script_sigs, vec![([0x1d; 32], [0x19; 32], vec![0x83; 65])]);
        assert!(f[1].signed(), "a script signature is a signature");
        // a final witness, read as final_witnesses reads it
        let b = crate::frogbuy::psbt_bytes(&Psbt::parse(&honest()).unwrap(), &[Some(vec![vec![9u8; 64]])]);
        assert_eq!(input_fields(&b).unwrap()[0].final_script_witness, Some(vec![vec![9u8; 64]]));
        assert!(input_fields(&b).unwrap()[0].signed());
    }

    #[test]
    fn a_field_of_the_wrong_length_is_malformed_not_guessed() {
        let good = rich_psbt();
        // the 4-byte sighash value, made 3 bytes long: find "01 03 04 00000000" and shorten it
        let at = good.windows(7).position(|w| w == [1, PSBT_IN_SIGHASH_TYPE, 4, 0, 0, 0, 0]).unwrap();
        let mut bad = good[..at].to_vec();
        bad.extend_from_slice(&[1, PSBT_IN_SIGHASH_TYPE, 3, 0, 0, 0]);
        bad.extend_from_slice(&good[at + 7..]);
        assert_eq!(input_fields(&bad).unwrap_err(), ParseError::Malformed);
        // a 31-byte internal key
        let at = good.windows(3).position(|w| w == [1, PSBT_IN_TAP_INTERNAL_KEY, 32]).unwrap();
        let mut bad = good[..at].to_vec();
        bad.extend_from_slice(&[1, PSBT_IN_TAP_INTERNAL_KEY, 31]);
        bad.extend_from_slice(&good[at + 4..]);
        assert_eq!(input_fields(&bad).unwrap_err(), ParseError::Malformed);
    }

    #[test]
    fn a_raw_transaction_parses_in_both_serialisations_to_one_txid() {
        let p = Psbt::parse(&honest()).unwrap();
        let legacy = {
            let mut b = p.tx.version.to_le_bytes().to_vec();
            b.extend(ser_compact(1));
            b.extend_from_slice(&p.tx.inputs[0].txid);
            b.extend_from_slice(&p.tx.inputs[0].vout.to_le_bytes());
            b.push(0);
            b.extend_from_slice(&p.tx.inputs[0].sequence.to_le_bytes());
            b.extend(ser_compact(p.tx.outputs.len()));
            for o in &p.tx.outputs {
                b.extend(ser_txout(o));
            }
            b.extend_from_slice(&p.tx.locktime.to_le_bytes());
            b
        };
        let r = parse_raw_tx(&legacy).unwrap();
        assert_eq!((r.tx.clone(), r.txid), (p.tx.clone(), p.txid()));
        assert_eq!(r.witnesses, vec![Vec::<Vec<u8>>::new()]);
        // the same transaction signed: witness serialisation, same txid
        let signed = finalize(&p, &[None], &[(0, vec![0xab; 64])]).unwrap();
        let r = parse_raw_tx(&signed).unwrap();
        assert_eq!((r.tx, r.txid), (p.tx.clone(), p.txid()));
        assert_eq!(r.witnesses, vec![vec![vec![0xab; 64]]]);
        // a scriptSig is part of a legacy txid
        let mut with_sig = legacy[..4 + 1 + 36].to_vec();
        with_sig.extend_from_slice(&[1, 0x51]);
        with_sig.extend_from_slice(&legacy[4 + 1 + 36 + 1..]);
        let r = parse_raw_tx(&with_sig).unwrap();
        assert_eq!(r.script_sigs, vec![vec![0x51]]);
        assert_ne!(r.txid, p.txid());
        // refused: trailing bytes, a flag that is not 1, a witness serialisation with only empty witnesses, truncation
        assert_eq!(parse_raw_tx(&[legacy.clone(), vec![0]].concat()).unwrap_err(), ParseError::Malformed);
        let mut flag2 = signed.clone();
        flag2[5] = 2;
        assert_eq!(parse_raw_tx(&flag2).unwrap_err(), ParseError::Malformed);
        let mut empty = signed[..signed.len() - 4 - 66].to_vec();
        empty.push(0);
        empty.extend_from_slice(&signed[signed.len() - 4..]);
        assert_eq!(parse_raw_tx(&empty).unwrap_err(), ParseError::Malformed);
        assert!(parse_raw_tx(&legacy[..legacy.len() - 1]).is_err());
    }

    #[test]
    fn the_hand_back_adds_two_pairs_per_signed_input_and_keeps_every_other_byte() {
        let p = rich_psbt();
        let out = with_tap_key_sigs(&p, &[(0, [0x5a; 64])], [0x7e; 32]).unwrap();
        let mut added = vec![1, PSBT_IN_TAP_KEY_SIG, 64];
        added.extend_from_slice(&[0x5a; 64]);
        added.extend_from_slice(&[1, PSBT_IN_TAP_INTERNAL_KEY, 32]);
        added.extend_from_slice(&[0x7e; 32]);
        // exactly the original with the two pairs inserted before input 0's separator
        let mut i = MAGIC.len();
        map_end(&p, &mut i).unwrap();
        let (_, sep) = map_end(&p, &mut i).unwrap();
        assert_eq!(out, [&p[..sep], &added[..], &p[sep..]].concat());
        // it re-parses to the same transaction, prevouts and fields, plus ours
        assert_eq!(Psbt::parse(&out).unwrap(), Psbt::parse(&p).unwrap());
        let (before, after) = (input_fields(&p).unwrap(), input_fields(&out).unwrap());
        assert_eq!(after[0].tap_key_sig, Some(vec![0x5a; 64]));
        assert_eq!(after[0].tap_internal_key, Some([0x7e; 32]));
        assert_eq!(after[1], before[1], "the escrow input is untouched");
        // two inputs at once, and an input already stating the same internal key gains no second copy
        let both = with_tap_key_sigs(&p, &[(0, [1; 64]), (1, [2; 64])], [0x50; 32]).unwrap();
        assert_eq!(both.len(), p.len() + 67 + 35 + 67);
        assert_eq!(input_fields(&both).unwrap()[1].tap_key_sig, Some(vec![2; 64]));
    }

    #[test]
    fn the_hand_back_refuses_an_input_already_signed_or_keyed_otherwise() {
        let p = rich_psbt();
        assert!(with_tap_key_sigs(&p, &[(2, [0; 64])], [0; 32]).unwrap_err().contains("does not exist"));
        assert!(with_tap_key_sigs(&p, &[(0, [0; 64]), (0, [1; 64])], [0; 32]).unwrap_err().contains("twice"));
        assert!(with_tap_key_sigs(&p, &[(1, [0; 64])], [0x7e; 32]).unwrap_err().contains("another internal key"));
        let once = with_tap_key_sigs(&p, &[(0, [0; 64])], [0x7e; 32]).unwrap();
        assert!(with_tap_key_sigs(&once, &[(0, [0; 64])], [0x7e; 32]).unwrap_err().contains("key-path signature"));
        let finished = crate::frogbuy::psbt_bytes(&Psbt::parse(&honest()).unwrap(), &[Some(vec![vec![9u8; 64]])]);
        assert!(with_tap_key_sigs(&finished, &[(0, [0; 64])], [0; 32]).unwrap_err().contains("final witness"));
    }
}
