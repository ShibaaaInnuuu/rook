//! Decoding a runestone: what a Bitcoin transaction actually moves.
//!
//! A rune transfer is an `OP_RETURN OP_13 <pushes>` output whose payload is a sequence of LEB128 varints. The buyer must be
//! able to say, from the raw transaction alone, "this sends at least N units of rune R to the output holding the winner's
//! script" — otherwise it is taking a venue's word for the one fact that matters. That check is arithmetic, so we do it here
//! rather than believe anybody.
//!
//! This decodes the parts the buyer needs and no more: the edicts, and the pointer that says where unallocated runes go.
//! Etching, minting and the protocol's other tags are parsed only far enough to be skipped correctly.
//!
//! Reference: the Runes protocol's own encoding — tag/value pairs of LEB128 varints, tag `0` (Body) introducing edicts of
//! four varints each (block, tx, amount, output), each edict's rune id delta-encoded against the one before it.

/// A rune's id: the block it was etched in and the index of the etching transaction in that block.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize, candid::CandidType)]
pub struct RuneId {
    pub block: u64,
    pub tx: u32,
}

impl RuneId {
    /// The `block:tx` form every explorer and API prints.
    pub fn parse(s: &str) -> Option<Self> {
        let (b, t) = s.split_once(':')?;
        Some(Self { block: b.parse().ok()?, tx: t.parse().ok()? })
    }
}

impl core::fmt::Display for RuneId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}:{}", self.block, self.tx)
    }
}

/// One instruction to move runes to one output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Edict {
    pub id: RuneId,
    pub amount: u128,
    /// Index into the transaction's outputs. An index equal to the number of outputs means "spread over every non-OP_RETURN
    /// output", which the buyer refuses: we want one named recipient, not a spread.
    pub output: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Runestone {
    pub edicts: Vec<Edict>,
    /// Where unallocated runes go. Absent means the first non-OP_RETURN output, which is a default we refuse to rely on.
    pub pointer: Option<u32>,
    /// True when a tag we do not understand appeared, or the payload did not decode. The protocol calls such a runestone a
    /// cenotaph and **burns** the runes in it. The buyer must never sign one.
    pub cenotaph: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub enum DecodeError {
    /// No `OP_RETURN OP_13` output in the transaction: nothing moves any rune.
    NoRunestone,
    /// More than one. The protocol takes the first; we refuse the ambiguity outright.
    ManyRunestones,
}

const OP_RETURN: u8 = 0x6a;
const OP_13: u8 = 0x5d;
const TAG_BODY: u128 = 0;
const TAG_POINTER: u128 = 22;
const TAG_MINT: u128 = 20;

/// The even tags this decoder knows. **Every other even tag makes the runestone a cenotaph**, and that is not a guess:
/// it was measured against a live `ord` indexer on signet, with a control.
///
/// Two transactions in signet block 323,262, byte-identical but for three bytes:
///
/// ```text
/// deee13a7…a66c  6a5d08        1484c80b140f1600   -> indexer credited 1 UNCOMMON•GOODS
/// 904e55c3…8f4a  6a5d0b fe0100 1484c80b140f1600   -> indexer credited nothing
/// ```
///
/// `fe 01` is tag 254 and `00` is its value. The mint tag, the rune id and the pointer are identical in both. The first
/// one minted; the second did not. Runes follows the usual forward-compatibility rule — an unrecognised ODD tag is
/// ignored, an unrecognised EVEN tag is a cenotaph and the runes in it are burned.
///
/// Before this, `decode` read past every tag it did not act on. So a venue could add one even tag to an otherwise honest
/// runestone: this code would report "delivers N units to the winner", `Psbt::verify` would sign it, and every indexer in
/// the world would burn the asset. Same shape as the `payload_of` hole, and found the same way — by asking something that
/// does not share our assumptions.
///
/// **This list is deliberately narrower than `ord`'s.** `ord` also recognises the etching tags (Flags, Rune, Premine,
/// Cap, Amount, the height and offset bounds) and would not call those a cenotaph. Those were not measured here, so they
/// are not on this list, and a runestone carrying one is refused rather than accepted. That direction is deliberate: the
/// buyer only ever needs to read a transfer, refusing a valid runestone costs a purchase, and accepting an invalid one
/// costs the asset. Widening this is a change to make against `ord`'s source, not against a memory of it.
const RECOGNISED_EVEN_TAGS: [u128; 3] = [TAG_BODY, TAG_MINT, TAG_POINTER];

/// Read one LEB128 varint. Returns the value and how many bytes it took, or `None` on a truncated or over-long encoding.
pub(crate) fn varint(b: &[u8]) -> Option<(u128, usize)> {
    let mut value: u128 = 0;
    for (i, &byte) in b.iter().enumerate() {
        if i > 18 {
            return None; // longer than any u128 can hold
        }
        let part = (byte & 0x7f) as u128;
        value = value.checked_add(part.checked_shl(7 * i as u32)?)?;
        if byte & 0x80 == 0 {
            return Some((value, i + 1));
        }
    }
    None
}

/// What an `OP_RETURN OP_13 …` output's script carries.
pub(crate) enum Payload {
    /// Every data push after the opcode, concatenated.
    Valid(Vec<u8>),
    /// The script is a runestone output but is **not** a sequence of data pushes. The protocol calls this a cenotaph and
    /// burns the runes; it is not "the pushes we managed to read before the odd byte".
    Invalid,
}

/// Pull the payload out of one `OP_RETURN OP_13 …` script.
///
/// `None` means the script is not a runestone output at all. `Invalid` means it is one and is malformed, which is a
/// **cenotaph** — the runes it names are burned.
///
/// That distinction used to be missing and it was a hole in the buyer's security argument, not a tidiness point. This
/// function used to stop at the first non-push opcode and return the pushes it had already read, with a comment saying the
/// caller would catch it; the caller did not, and could not, because nothing was passed back to catch. A venue could
/// therefore hand us a PSBT whose runestone *this code* reads as "delivers N units to the winner" while `ord` reads it as
/// a cenotaph and burns the lot — and `Psbt::verify` would have signed it.
///
/// Nothing in this module's tests could have found it: every one of them builds its scripts out of pure data pushes,
/// because that is what a correct encoder emits. The assumption "a runestone script contains only pushes" was held by the
/// encoder, the decoder and every test at once, and the only party that disagrees is an `ord` indexer.
pub(crate) fn payload_of(script: &[u8]) -> Option<Payload> {
    if script.len() < 2 || script[0] != OP_RETURN || script[1] != OP_13 {
        return None;
    }
    let mut out = Vec::new();
    let mut i = 2usize;
    while i < script.len() {
        let op = script[i];
        i += 1;
        let len = match op {
            // OP_0 pushes the empty string, and counts as a push.
            0 => 0usize,
            1..=75 => op as usize,
            76 => match script.get(i) {
                // OP_PUSHDATA1
                Some(&n) => {
                    i += 1;
                    n as usize
                }
                None => return Some(Payload::Invalid),
            },
            77 => match script.get(i..i + 2) {
                // OP_PUSHDATA2
                Some(n) => {
                    i += 2;
                    u16::from_le_bytes([n[0], n[1]]) as usize
                }
                None => return Some(Payload::Invalid),
            },
            78 => match script.get(i..i + 4) {
                // OP_PUSHDATA4
                Some(n) => {
                    i += 4;
                    u32::from_le_bytes([n[0], n[1], n[2], n[3]]) as usize
                }
                None => return Some(Payload::Invalid),
            },
            // Any other opcode. Not a push, therefore not a runestone, therefore a cenotaph.
            _ => return Some(Payload::Invalid),
        };
        let Some(end) = i.checked_add(len) else { return Some(Payload::Invalid) };
        match script.get(i..end) {
            Some(bytes) => out.extend_from_slice(bytes),
            // A push that runs off the end of the script: the script does not parse, so it is a cenotaph too.
            None => return Some(Payload::Invalid),
        }
        i = end;
    }
    Some(Payload::Valid(out))
}

impl Runestone {
    /// Decode the runestone in a transaction's outputs.
    ///
    /// `scripts` is every output's scriptPubKey, in order, so that an edict's `output` index means what it means on chain.
    pub fn decode(scripts: &[Vec<u8>]) -> Result<Self, DecodeError> {
        let mut found: Option<Payload> = None;
        for s in scripts {
            if s.len() >= 2 && s[0] == OP_RETURN && s[1] == OP_13 {
                if found.is_some() {
                    return Err(DecodeError::ManyRunestones);
                }
                found = Some(payload_of(s).expect("the guard above is payload_of's own test"));
            }
        }
        let payload = match found.ok_or(DecodeError::NoRunestone)? {
            Payload::Valid(p) => p,
            // A script that is not a sequence of data pushes is a cenotaph. Report it as one rather than as the prefix
            // that happened to parse: the difference is whether the runes are delivered or burned.
            Payload::Invalid => return Ok(Runestone { cenotaph: true, ..Default::default() }),
        };

        let mut out = Runestone::default();
        let mut i = 0usize;
        // Tag/value pairs until tag 0 (Body), after which everything is edicts.
        while i < payload.len() {
            let Some((tag, n)) = varint(&payload[i..]) else {
                out.cenotaph = true;
                return Ok(out);
            };
            i += n;
            if tag == TAG_BODY {
                let mut last = RuneId::default();
                while i < payload.len() {
                    let mut f = [0u128; 4];
                    for slot in f.iter_mut() {
                        let Some((v, n)) = varint(&payload[i..]) else {
                            out.cenotaph = true;
                            return Ok(out);
                        };
                        i += n;
                        *slot = v;
                    }
                    // An edict's id is a delta on the previous one: (block delta, tx delta), tx absolute when block delta is 0.
                    // `as` would truncate here, and a truncated id is a DIFFERENT rune — the decoder would report a
                    // delivery of something nobody sent. The protocol calls an out-of-range id a cenotaph; so do we.
                    let (Ok(block_delta), Ok(tx_field)) = (u64::try_from(f[0]), u32::try_from(f[1])) else {
                        out.cenotaph = true;
                        return Ok(out);
                    };
                    let block = match last.block.checked_add(block_delta) {
                        Some(b) => b,
                        None => {
                            out.cenotaph = true;
                            return Ok(out);
                        }
                    };
                    let tx = if block_delta == 0 {
                        match last.tx.checked_add(tx_field) {
                            Some(t) => t,
                            None => {
                                out.cenotaph = true;
                                return Ok(out);
                            }
                        }
                    } else {
                        tx_field
                    };
                    let id = RuneId { block, tx };
                    last = id;
                    if f[3] > u32::MAX as u128 {
                        out.cenotaph = true;
                        return Ok(out);
                    }
                    out.edicts.push(Edict { id, amount: f[2], output: f[3] as u32 });
                }
                break;
            }
            // An unrecognised EVEN tag is a cenotaph. An unrecognised odd tag is ignored, which is what makes the rule a
            // forward-compatibility rule rather than a freeze.
            if tag % 2 == 0 && !RECOGNISED_EVEN_TAGS.contains(&tag) {
                out.cenotaph = true;
                return Ok(out);
            }
            // Every other tag carries one value; read and keep only the pointer.
            let Some((value, n)) = varint(&payload[i..]) else {
                out.cenotaph = true;
                return Ok(out);
            };
            i += n;
            if tag == TAG_POINTER {
                if value > u32::MAX as u128 {
                    out.cenotaph = true;
                    return Ok(out);
                }
                out.pointer = Some(value as u32);
            }
        }
        Ok(out)
    }

    /// How many units of `id` this runestone sends to output `index`, counting every edict that names it.
    pub fn units_to(&self, id: RuneId, index: u32) -> u128 {
        self.edicts
            .iter()
            .filter(|e| e.id == id && e.output == index)
            .fold(0u128, |a, e| a.saturating_add(e.amount))
    }

    /// True when any edict spreads over every output rather than naming one. The buyer refuses these: a spread's arithmetic
    /// depends on how many outputs there are, which a hostile PSBT controls.
    pub fn has_spread(&self, output_count: u32) -> bool {
        self.edicts.iter().any(|e| e.output >= output_count)
    }
}

// ---- building one, which until now nothing here could do --------------------------------------------------------------------

/// A runestone we intend to *build*, as opposed to one we found on a chain.
///
/// The buyer has always been able to read a runestone and never to write one, which meant the encoding had only ever been
/// exercised against this module's own test helpers — the closed loop that the signet rehearsal's taproot bug came out of.
/// `Plan::script` is the encoder that `docs/v2/2026-09-22-evm-and-runes-rehearsals.md` puts on a real chain.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Plan {
    /// Move runes to named outputs. Sorted by rune id before encoding, because the ids are delta-encoded and a delta is
    /// only meaningful against a sorted sequence.
    pub edicts: Vec<Edict>,
    /// Where unallocated runes go. The buyer always sets it: the default is "the first non-OP_RETURN output", and relying
    /// on a default for where an asset lands is how an asset lands somewhere else.
    pub pointer: Option<u32>,
    /// Mint an open rune. The minted amount is unallocated, so it follows `pointer`.
    pub mint: Option<RuneId>,
}

/// Append one LEB128 varint.
fn push_varint(out: &mut Vec<u8>, mut v: u128) {
    loop {
        let b = (v & 0x7f) as u8;
        v >>= 7;
        if v == 0 {
            out.push(b);
            return;
        }
        out.push(b | 0x80);
    }
}

/// Append a minimal data push of `data`. Minimal matters: a non-minimal push is still a push, but an encoder that emits
/// one is an encoder whose output differs from every other implementation's for the same input, and that difference is
/// the sort of thing that is only noticed once it is on a chain.
fn push_data(out: &mut Vec<u8>, data: &[u8]) {
    match data.len() {
        0 => out.push(0),
        n if n <= 75 => out.push(n as u8),
        n if n <= 255 => {
            out.push(76);
            out.push(n as u8);
        }
        n => {
            out.push(77);
            out.extend_from_slice(&(n as u16).to_le_bytes());
        }
    }
    out.extend_from_slice(data);
}

/// The largest data push a standard script may carry.
const MAX_PUSH: usize = 520;

impl Plan {
    /// The runestone's payload: tag/value pairs, then the Body tag, then the edicts.
    ///
    /// Ordering is not cosmetic. Every non-Body tag must come **before** `Body`, because `Body` means "everything after
    /// this is edicts" and there is no way back out of it. An encoder that emitted the pointer after the body would write
    /// a pointer that every decoder reads as part of an edict.
    pub fn payload(&self) -> Vec<u8> {
        let mut p = Vec::new();
        if let Some(id) = self.mint {
            // The Mint tag carries a rune id, which is two values, so the tag itself appears twice.
            push_varint(&mut p, TAG_MINT);
            push_varint(&mut p, id.block as u128);
            push_varint(&mut p, TAG_MINT);
            push_varint(&mut p, id.tx as u128);
        }
        if let Some(ptr) = self.pointer {
            push_varint(&mut p, TAG_POINTER);
            push_varint(&mut p, ptr as u128);
        }
        if !self.edicts.is_empty() {
            push_varint(&mut p, TAG_BODY);
            let mut sorted = self.edicts.clone();
            sorted.sort_by_key(|e| (e.id.block, e.id.tx));
            let mut last = RuneId::default();
            for e in &sorted {
                let block_delta = e.id.block - last.block;
                // The tx field is a delta only within one block; across blocks it is absolute. This is the rule the
                // decoder already implements, written from the other side.
                let tx_field = if block_delta == 0 { e.id.tx - last.tx } else { e.id.tx };
                push_varint(&mut p, block_delta as u128);
                push_varint(&mut p, tx_field as u128);
                push_varint(&mut p, e.amount);
                push_varint(&mut p, e.output as u128);
                last = e.id;
            }
        }
        p
    }

    /// The full scriptPubKey: `OP_RETURN OP_13` and the payload, pushed in standard-sized chunks.
    pub fn script(&self) -> Vec<u8> {
        let mut s = vec![OP_RETURN, OP_13];
        let payload = self.payload();
        if payload.is_empty() {
            return s;
        }
        for chunk in payload.chunks(MAX_PUSH) {
            push_data(&mut s, chunk);
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build an `OP_RETURN OP_13` script from a payload, pushing it the way a small payload is pushed.
    fn runestone_script(payload: &[u8]) -> Vec<u8> {
        let mut s = vec![OP_RETURN, OP_13];
        assert!(payload.len() <= 75, "test helper only pushes small payloads");
        s.push(payload.len() as u8);
        s.extend_from_slice(payload);
        s
    }

    /// A runestone script from a hex string, so a test can name the exact bytes a chain held.
    fn hex_script(h: &str) -> Vec<u8> {
        (0..h.len()).step_by(2).map(|i| u8::from_str_radix(&h[i..i + 2], 16).unwrap()).collect()
    }

    /// `runestone_script` but without the 75-byte assertion, for payloads a test builds by hand.
    fn runestone_script_long(payload: &[u8]) -> Vec<u8> {
        let mut s = vec![OP_RETURN, OP_13];
        if payload.len() <= 75 {
            s.push(payload.len() as u8);
        } else {
            s.push(76);
            s.push(payload.len() as u8);
        }
        s.extend_from_slice(payload);
        s
    }

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

    fn body(edicts: &[(u128, u128, u128, u128)]) -> Vec<u8> {
        let mut p = leb(TAG_BODY);
        for (blk, tx, amt, out) in edicts {
            p.extend(leb(*blk));
            p.extend(leb(*tx));
            p.extend(leb(*amt));
            p.extend(leb(*out));
        }
        p
    }

    #[test]
    fn varint_round_trips_and_refuses_a_truncated_one() {
        for v in [0u128, 1, 127, 128, 300, u64::MAX as u128] {
            let e = leb(v);
            assert_eq!(varint(&e), Some((v, e.len())), "{v}");
        }
        assert_eq!(varint(&[0x80]), None, "a continuation byte with nothing after it");
        assert_eq!(varint(&[]), None);
    }

    #[test]
    fn one_edict_is_read_back_with_its_id_amount_and_output() {
        let p = body(&[(840_000, 3, 5_000, 1)]);
        let r = Runestone::decode(&[vec![0x51], runestone_script(&p), vec![0x52]]).unwrap();
        assert!(!r.cenotaph);
        assert_eq!(r.edicts, vec![Edict { id: RuneId { block: 840_000, tx: 3 }, amount: 5_000, output: 1 }]);
        assert_eq!(r.units_to(RuneId { block: 840_000, tx: 3 }, 1), 5_000);
        assert_eq!(r.units_to(RuneId { block: 840_000, tx: 3 }, 0), 0, "a different output is not ours");
        assert_eq!(r.units_to(RuneId { block: 1, tx: 1 }, 1), 0, "a different rune is not ours");
    }

    #[test]
    fn ids_are_delta_encoded_against_the_edict_before() {
        // (840000,3) then a delta of (0,2) meaning the same block, tx 3+2; then (1,7) meaning block 840001, tx 7 absolute.
        let p = body(&[(840_000, 3, 10, 0), (0, 2, 20, 1), (1, 7, 30, 2)]);
        let r = Runestone::decode(&[runestone_script(&p)]).unwrap();
        assert_eq!(
            r.edicts.iter().map(|e| (e.id.block, e.id.tx, e.amount)).collect::<Vec<_>>(),
            vec![(840_000, 3, 10), (840_000, 5, 20), (840_001, 7, 30)]
        );
    }

    #[test]
    fn two_edicts_to_the_same_output_add_up() {
        let p = body(&[(840_000, 3, 400, 1), (0, 0, 600, 1)]);
        let r = Runestone::decode(&[runestone_script(&p)]).unwrap();
        assert_eq!(r.units_to(RuneId { block: 840_000, tx: 3 }, 1), 1_000);
    }

    #[test]
    fn the_pointer_is_read() {
        let mut p = leb(TAG_POINTER);
        p.extend(leb(2));
        p.extend(body(&[(840_000, 3, 1, 1)]));
        let r = Runestone::decode(&[runestone_script(&p)]).unwrap();
        assert_eq!(r.pointer, Some(2));
    }

    #[test]
    fn a_truncated_edict_is_a_cenotaph_not_a_transfer() {
        let mut p = leb(TAG_BODY);
        p.extend(leb(840_000));
        p.extend(leb(3)); // and then it stops: two of the four fields
        let r = Runestone::decode(&[runestone_script(&p)]).unwrap();
        assert!(r.cenotaph, "runes in a cenotaph are burned; the buyer must never sign one");
        assert!(r.edicts.is_empty());
    }

    #[test]
    fn a_spread_edict_is_visible_so_it_can_be_refused() {
        // output index == the number of outputs means "spread", whose arithmetic depends on the output count.
        let p = body(&[(840_000, 3, 100, 2)]);
        let scripts = vec![runestone_script(&p), vec![0x51]];
        let r = Runestone::decode(&scripts).unwrap();
        assert!(r.has_spread(scripts.len() as u32));
        assert!(!Runestone::decode(&[runestone_script(&body(&[(840_000, 3, 100, 1)])), vec![0x51]])
            .unwrap()
            .has_spread(2));
    }

    #[test]
    fn no_runestone_and_two_runestones_are_both_refused() {
        assert_eq!(Runestone::decode(&[vec![0x51]]), Err(DecodeError::NoRunestone));
        let s = runestone_script(&body(&[(1, 1, 1, 0)]));
        assert_eq!(Runestone::decode(&[s.clone(), s]), Err(DecodeError::ManyRunestones));
    }

    #[test]
    fn a_large_payload_pushed_with_pushdata1_still_decodes() {
        let p = body(&(0..30).map(|i| (if i == 0 { 840_000 } else { 0 }, 1u128, 7u128, 1u128)).collect::<Vec<_>>());
        assert!(p.len() > 75, "the point of this test is the longer push form");
        let mut s = vec![OP_RETURN, OP_13, 76, p.len() as u8];
        s.extend_from_slice(&p);
        let r = Runestone::decode(&[s]).unwrap();
        assert!(!r.cenotaph);
        assert_eq!(r.edicts.len(), 30);
        assert_eq!(r.units_to(RuneId { block: 840_000, tx: 1 }, 1), 7, "only the first edict has that id");
    }

    // ---- the encoder, and the round trip through it -------------------------------------------------------------------------

    #[test]
    fn what_the_encoder_writes_the_decoder_reads_back_unchanged() {
        let plan = Plan {
            edicts: vec![
                Edict { id: RuneId { block: 189_444, tx: 15 }, amount: 1, output: 0 },
                Edict { id: RuneId { block: 840_000, tx: 3 }, amount: 5_000, output: 1 },
            ],
            pointer: Some(2),
            mint: None,
        };
        let script = plan.script();
        assert_eq!(&script[..2], &[OP_RETURN, OP_13], "a runestone output is OP_RETURN OP_13 and then pushes");
        let r = Runestone::decode(&[vec![0x51], script, vec![0x52], vec![0x53]]).unwrap();
        assert!(!r.cenotaph);
        assert_eq!(r.pointer, Some(2));
        assert_eq!(r.edicts, plan.edicts, "same ids, same amounts, same outputs");
        assert_eq!(r.units_to(RuneId { block: 189_444, tx: 15 }, 0), 1);
        assert_eq!(r.units_to(RuneId { block: 840_000, tx: 3 }, 1), 5_000);
    }

    #[test]
    fn the_encoder_sorts_edicts_because_the_ids_are_deltas() {
        // Given out of order, the encoded form must still name the same runes. A delta encoder that trusted the caller's
        // order would write ids that decode to different runes entirely -- silently, and only on a chain.
        let unsorted = vec![
            Edict { id: RuneId { block: 840_001, tx: 7 }, amount: 30, output: 2 },
            Edict { id: RuneId { block: 189_444, tx: 15 }, amount: 1, output: 0 },
            Edict { id: RuneId { block: 840_000, tx: 5 }, amount: 20, output: 1 },
            Edict { id: RuneId { block: 840_000, tx: 3 }, amount: 10, output: 1 },
        ];
        let r = Runestone::decode(&[Plan { edicts: unsorted.clone(), ..Default::default() }.script()]).unwrap();
        assert!(!r.cenotaph);
        let mut expected = unsorted;
        expected.sort_by_key(|e| (e.id.block, e.id.tx));
        assert_eq!(r.edicts, expected);
        // and the same-block pair really is delta encoded, not repeated absolutely
        assert_eq!(r.units_to(RuneId { block: 840_000, tx: 5 }, 1), 20);
        assert_eq!(r.units_to(RuneId { block: 840_000, tx: 3 }, 1), 10);
    }

    #[test]
    fn a_mint_encodes_the_tag_twice_because_a_rune_id_is_two_values() {
        let plan = Plan { mint: Some(RuneId { block: 189_444, tx: 15 }), pointer: Some(0), ..Default::default() };
        let payload = plan.payload();
        let mut want = leb(TAG_MINT);
        want.extend(leb(189_444));
        want.extend(leb(TAG_MINT));
        want.extend(leb(15));
        want.extend(leb(TAG_POINTER));
        want.extend(leb(0));
        assert_eq!(payload, want);

        // The decoder skips tags it does not act on, and must not mistake either half of the mint for something else.
        let r = Runestone::decode(&[plan.script()]).unwrap();
        assert!(!r.cenotaph, "a mint is not a cenotaph");
        assert_eq!(r.pointer, Some(0));
        assert!(r.edicts.is_empty(), "a mint moves nothing by edict");
    }

    #[test]
    fn the_body_tag_comes_last_or_the_pointer_is_swallowed_by_an_edict() {
        let plan = Plan {
            edicts: vec![Edict { id: RuneId { block: 1, tx: 1 }, amount: 1, output: 0 }],
            pointer: Some(3),
            mint: Some(RuneId { block: 2, tx: 2 }),
        };
        let p = plan.payload();
        let body_at = p.iter().position(|&b| b == 0).expect("the Body tag is a zero byte");
        let pointer_at = p.windows(2).position(|w| w == [TAG_POINTER as u8, 3]).expect("the pointer pair");
        assert!(pointer_at < body_at, "every non-Body tag must precede Body: there is no way back out of the body");
        let r = Runestone::decode(&[plan.script()]).unwrap();
        assert_eq!(r.pointer, Some(3));
        assert_eq!(r.edicts.len(), 1);
    }

    #[test]
    fn a_payload_too_long_for_one_push_round_trips_through_several() {
        let edicts: Vec<Edict> = (0..80)
            .map(|i| Edict { id: RuneId { block: 800_000 + i, tx: 1 }, amount: 1_000_000_000_000u128 + i as u128, output: 1 })
            .collect();
        let plan = Plan { edicts: edicts.clone(), pointer: Some(2), ..Default::default() };
        assert!(plan.payload().len() > 255, "long enough to need more than a one-byte length");
        let r = Runestone::decode(&[plan.script(), vec![0x51], vec![0x52]]).unwrap();
        assert!(!r.cenotaph);
        assert_eq!(r.edicts, edicts);
        assert_eq!(r.pointer, Some(2));
    }

    // ---- the cenotaph hole, which no encoder would ever produce and a hostile venue certainly would ------------------------

    #[test]
    fn a_runestone_script_containing_a_non_push_opcode_is_a_cenotaph_not_a_partial_transfer() {
        // The attack this closes: a venue hands us a PSBT whose runestone reads, to a decoder that stops at the odd byte,
        // as "delivers 5000 units to output 1". `ord` reads the same script as a cenotaph and BURNS the runes. The buyer
        // would have signed it and the winner would have got nothing.
        let good = Plan {
            edicts: vec![Edict { id: RuneId { block: 840_000, tx: 3 }, amount: 5_000, output: 1 }],
            ..Default::default()
        }
        .script();
        let clean = Runestone::decode(&[good.clone(), vec![0x51]]).unwrap();
        assert_eq!(clean.units_to(RuneId { block: 840_000, tx: 3 }, 1), 5_000, "the honest form delivers");

        for trailing in [
            0x51u8, // OP_1
            0x75,   // OP_DROP
            0x6a,   // a second OP_RETURN inside the script
            0xac,   // OP_CHECKSIG
            0xff,   // not an opcode at all
        ] {
            let mut hostile = good.clone();
            hostile.push(trailing);
            let r = Runestone::decode(&[hostile, vec![0x51]]).unwrap();
            assert!(r.cenotaph, "opcode {trailing:#04x} after the pushes must be a cenotaph");
            assert_eq!(r.units_to(RuneId { block: 840_000, tx: 3 }, 1), 0, "and it must deliver nothing");
            assert!(r.edicts.is_empty());
        }
    }

    #[test]
    fn a_push_that_runs_off_the_end_of_the_script_is_a_cenotaph_too() {
        for script in [
            vec![OP_RETURN, OP_13, 10, 1, 2, 3],       // claims 10 bytes, supplies 3
            vec![OP_RETURN, OP_13, 76],                // OP_PUSHDATA1 with no length byte
            vec![OP_RETURN, OP_13, 76, 200, 1, 2],     // OP_PUSHDATA1 claiming 200
            vec![OP_RETURN, OP_13, 77, 0],             // OP_PUSHDATA2 with half a length
            vec![OP_RETURN, OP_13, 78, 0, 0, 0],       // OP_PUSHDATA4 with three quarters of one
            vec![OP_RETURN, OP_13, 78, 255, 255, 255, 255], // a length no script could hold
        ] {
            let r = Runestone::decode(&[script.clone(), vec![0x51]]).unwrap();
            assert!(r.cenotaph, "{script:?} must decode as a cenotaph");
        }
        // OP_0 is a push of nothing and is perfectly legal; it must NOT be a cenotaph.
        let empty = Runestone::decode(&[vec![OP_RETURN, OP_13, 0], vec![0x51]]).unwrap();
        assert!(!empty.cenotaph, "OP_0 pushes the empty string, which is still a push");
        assert!(empty.edicts.is_empty());
    }

    #[test]
    fn a_rune_id_too_large_for_its_type_is_a_cenotaph_rather_than_a_truncation() {
        // `as u64` / `as u32` would wrap here, and a wrapped id names a DIFFERENT rune. The decoder would then report a
        // delivery of a rune nobody sent -- which `Psbt::verify` would accept.
        let mut p = leb(TAG_BODY);
        p.extend(leb((u64::MAX as u128) + 1)); // block delta
        p.extend(leb(1));
        p.extend(leb(1));
        p.extend(leb(0));
        let r = Runestone::decode(&[runestone_script(&p)]).unwrap();
        assert!(r.cenotaph, "a block delta wider than u64 is out of range, not a small number");

        let mut q = leb(TAG_BODY);
        q.extend(leb(0)); // same block
        q.extend(leb((u32::MAX as u128) + 1)); // tx delta
        q.extend(leb(1));
        q.extend(leb(0));
        let r2 = Runestone::decode(&[runestone_script(&q)]).unwrap();
        assert!(r2.cenotaph, "a tx field wider than u32 is out of range too");
    }

    #[test]
    fn an_unrecognised_even_tag_is_a_cenotaph_and_a_real_indexer_says_so() {
        // Not a guess and not a reading of a specification. These are the exact bytes of two transactions in signet block
        // 323,262, and the difference between them is three bytes. See `docs/v2/2026-09-22-evm-and-runes-rehearsals.md`.
        let minted = hex_script("6a5d081484c80b140f1600"); // deee13a7…a66c -> indexer credited 1 UNCOMMON*GOODS
        let burned = hex_script("6a5d0bfe01001484c80b140f1600"); // 904e55c3…8f4a -> indexer credited nothing

        let ok = Runestone::decode(&[vec![0x51], minted, vec![0x52]]).unwrap();
        assert!(!ok.cenotaph, "the control minted, so it is not a cenotaph");
        assert_eq!(ok.pointer, Some(0));

        let bad = Runestone::decode(&[vec![0x51], burned, vec![0x52]]).unwrap();
        assert!(bad.cenotaph, "tag 254 is even and unrecognised: the indexer minted nothing for it");
        assert!(bad.edicts.is_empty(), "and a cenotaph delivers nothing at all");
    }

    #[test]
    fn an_even_tag_smuggled_alongside_an_honest_edict_is_caught() {
        // The attack. Without the even-tag rule this reads as "delivers 5,000 units to output 1" and `Psbt::verify`
        // signs it, while every indexer burns the lot.
        let honest = Plan {
            edicts: vec![Edict { id: RuneId { block: 840_000, tx: 3 }, amount: 5_000, output: 1 }],
            pointer: Some(2),
            mint: None,
        };
        let clean = Runestone::decode(&[honest.script(), vec![0x51], vec![0x52]]).unwrap();
        assert_eq!(clean.units_to(RuneId { block: 840_000, tx: 3 }, 1), 5_000);

        for tag in [2u128, 4, 6, 8, 26, 100, 254, 1000] {
            let mut p = leb(tag);
            p.extend(leb(0));
            p.extend(honest.payload());
            let r = Runestone::decode(&[runestone_script_long(&p), vec![0x51], vec![0x52]]).unwrap();
            assert!(r.cenotaph, "even tag {tag} must be a cenotaph");
            assert_eq!(r.units_to(RuneId { block: 840_000, tx: 3 }, 1), 0, "and deliver nothing");
        }
    }

    #[test]
    fn an_unrecognised_odd_tag_is_ignored_rather_than_fatal() {
        // The other half of the rule. Odd tags are the protocol's room to grow, so refusing them would make the buyer
        // break the first time anything new shipped.
        let honest = Plan {
            edicts: vec![Edict { id: RuneId { block: 840_000, tx: 3 }, amount: 5_000, output: 1 }],
            pointer: Some(2),
            mint: None,
        };
        for tag in [1u128, 3, 5, 7, 101, 255] {
            let mut p = leb(tag);
            p.extend(leb(12_345));
            p.extend(honest.payload());
            let r = Runestone::decode(&[runestone_script_long(&p), vec![0x51], vec![0x52]]).unwrap();
            assert!(!r.cenotaph, "odd tag {tag} must be ignored, not fatal");
            assert_eq!(r.units_to(RuneId { block: 840_000, tx: 3 }, 1), 5_000, "and delivery still reads");
            assert_eq!(r.pointer, Some(2));
        }
    }

    #[test]
    fn rune_id_parses_the_form_every_explorer_prints() {
        assert_eq!(RuneId::parse("840000:3"), Some(RuneId { block: 840_000, tx: 3 }));
        assert_eq!(RuneId::parse("840000"), None);
        assert_eq!(RuneId::parse("a:b"), None);
        assert_eq!(RuneId { block: 840_000, tx: 3 }.to_string(), "840000:3");
    }
}
