//! How many units of which rune a confirmed transaction delivered (`docs/v2/rates-and-indexer.md` §2). Pure: no network.
//!
//! Two keyless indexers are read for what each can establish: ord's `/r/utxo` reports runes **per output** by spaced name, for
//! unspent outputs; Ordinals Wallet reports an **address's** balance per rune, with its id and name. Neither is believed about
//! the rune's name: that is decoded here from the etching transaction itself, by the same audited runestone parser that reads
//! edicts. The answer is ord's per-output count, corroborated by the address balance, or no answer at all.

use std::collections::BTreeMap;

use crate::runestone::{payload_of, varint, Payload, RuneId};

/// ord calls a rune with no name in its etching by this number plus `(block << 32) | tx`.
const RESERVED: u128 = 6402364363415443603228541259936211926;
const TAG_BODY: u128 = 0;
const TAG_FLAGS: u128 = 2;
const TAG_RUNE: u128 = 4;
const TAG_SPACERS: u128 = 3;
const FLAG_ETCHING: u128 = 1;

/// The rune protocol's name for a rune number: bijective base 26, `A` = 0, `Z` = 25, `AA` = 26.
pub fn letters(rune: u128) -> String {
    if rune == u128::MAX {
        return "BCGDENLQRQWDSLRUGSNLBTMFIJAV".into();
    }
    let mut n = rune + 1;
    let mut out = Vec::new();
    while n > 0 {
        out.push(b'A' + ((n - 1) % 26) as u8);
        n = (n - 1) / 26;
    }
    out.reverse();
    String::from_utf8(out).unwrap_or_default()
}

/// The name as ord prints it: a `•` after each letter whose bit is set in `spacers`, never after the last.
pub fn spaced(rune: u128, spacers: u32) -> String {
    let l = letters(rune);
    let n = l.len();
    let mut s = String::new();
    for (i, c) in l.chars().enumerate() {
        s.push(c);
        if i + 1 < n && spacers & (1 << i) != 0 {
            s.push('•');
        }
    }
    s
}

/// The spaced name of rune `id`, from the scripts of the transaction that etched it (transaction `id.tx` of block `id.block`).
/// `1:0` is the protocol's genesis rune, which has no etching transaction.
pub fn etched_name(id: RuneId, output_scripts: &[Vec<u8>]) -> Result<String, String> {
    if id.block == 1 && id.tx == 0 {
        return Ok("UNCOMMON•GOODS".into());
    }
    let payload = output_scripts.iter().find_map(|s| payload_of(s)).ok_or("no runestone in the etching transaction")?;
    let bytes = match payload {
        Payload::Valid(b) => b,
        Payload::Invalid => return Err("the etching transaction's runestone is a cenotaph".into()),
    };
    let mut ints = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let (v, n) = varint(&bytes[i..]).ok_or("a truncated integer in the etching's runestone")?;
        ints.push(v);
        i += n;
    }
    // fields are (tag, value) pairs up to the Body tag; the first value of each tag is the one that counts
    let mut fields: BTreeMap<u128, u128> = BTreeMap::new();
    let mut k = 0;
    while k < ints.len() {
        let tag = ints[k];
        if tag == TAG_BODY {
            break;
        }
        let value = *ints.get(k + 1).ok_or("a tag with no value in the etching's runestone")?;
        fields.entry(tag).or_insert(value);
        k += 2;
    }
    if fields.get(&TAG_FLAGS).copied().unwrap_or(0) & FLAG_ETCHING == 0 {
        return Err(format!("transaction {}:{} does not etch a rune", id.block, id.tx));
    }
    let rune = match fields.get(&TAG_RUNE) {
        Some(r) => *r,
        None => RESERVED + ((id.block as u128) << 32 | id.tx as u128),
    };
    let spacers = u32::try_from(fields.get(&TAG_SPACERS).copied().unwrap_or(0)).map_err(|_| "spacers out of range")?;
    Ok(spaced(rune, spacers))
}

/// One output as ord reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OrdOutput {
    /// Unspent and indexed: its runes by spaced name.
    Found(BTreeMap<String, u128>),
    /// ord does not have it: spent before it was read, or never seen.
    NotFound,
}

/// ord's `/r/utxo/<outpoint>` reply. An amount beyond `u64` is refused rather than read through a float.
pub fn parse_ord_utxo(status: u16, body: &[u8]) -> Result<OrdOutput, String> {
    if status == 404 || (status == 200 && body.starts_with(b"output ") && body.ends_with(b"not found")) {
        return Ok(OrdOutput::NotFound);
    }
    if status != 200 {
        return Err(format!("ord: HTTP {status}"));
    }
    let v: serde_json::Value = serde_json::from_slice(body).map_err(|_| "ord: not JSON")?;
    let runes = v.get("runes").and_then(|r| r.as_object()).ok_or("ord: no runes field")?;
    let mut out = BTreeMap::new();
    for (name, r) in runes {
        let a = r.get("amount").ok_or("ord: a rune with no amount")?;
        let n = a.as_u64().ok_or_else(|| format!("ord: {name}'s amount {a} is beyond what is read exactly"))?;
        out.insert(name.clone(), n as u128);
    }
    Ok(OrdOutput::Found(out))
}

/// One row of Ordinals Wallet's `/wallet/<address>/rune-balance`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Holding {
    pub id: RuneId,
    pub name: String,
    pub amount: u128,
}

pub fn parse_rune_id(s: &str) -> Option<RuneId> {
    let (b, t) = s.split_once(':')?;
    Some(RuneId { block: b.parse().ok()?, tx: t.parse().ok()? })
}

/// Ordinals Wallet's address balance. Amounts are decimal strings, read exactly.
pub fn parse_ow_balance(status: u16, body: &[u8]) -> Result<Vec<Holding>, String> {
    if status != 200 {
        return Err(format!("ordinals wallet: HTTP {status}"));
    }
    let v: serde_json::Value = serde_json::from_slice(body).map_err(|_| "ordinals wallet: not JSON")?;
    let rows = v.as_array().ok_or("ordinals wallet: not a list")?;
    rows.iter().map(|r| {
        let id = r.get("rune_id").and_then(|x| x.as_str()).and_then(parse_rune_id).ok_or("ordinals wallet: a row with no rune id")?;
        let name = r.get("name").and_then(|x| x.as_str()).ok_or("ordinals wallet: a row with no name")?.to_string();
        let amount = match r.get("amount") {
            Some(serde_json::Value::String(s)) => s.parse::<u128>().ok(),
            Some(serde_json::Value::Number(n)) => n.as_u64().map(|x| x as u128),
            _ => None,
        }.ok_or("ordinals wallet: an unreadable amount")?;
        Ok(Holding { id, name, amount })
    }).collect()
}

/// A transaction's outputs and confirmation, as an esplora explorer serves `/tx/<txid>`, reduced to what is compared.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TxOutputs {
    pub height: Option<u64>,
    /// (vout, scriptpubkey hex, address if the explorer names one)
    pub outputs: Vec<(u32, String, Option<String>)>,
}

pub fn parse_esplora_tx(status: u16, body: &[u8]) -> Result<TxOutputs, String> {
    if status != 200 {
        return Err(format!("explorer: HTTP {status}"));
    }
    let v: serde_json::Value = serde_json::from_slice(body).map_err(|_| "explorer: not JSON")?;
    let st = v.get("status").ok_or("explorer: no status")?;
    let height = if st.get("confirmed").and_then(|c| c.as_bool()) == Some(true) {
        Some(st.get("block_height").and_then(|h| h.as_u64()).ok_or("explorer: confirmed with no height")?)
    } else {
        None
    };
    let outs = v.get("vout").and_then(|o| o.as_array()).ok_or("explorer: no outputs")?;
    let outputs = outs.iter().enumerate().map(|(i, o)| {
        let spk = o.get("scriptpubkey").and_then(|s| s.as_str()).ok_or("explorer: an output with no script")?.to_ascii_lowercase();
        let addr = o.get("scriptpubkey_address").and_then(|a| a.as_str()).map(str::to_string);
        Ok((i as u32, spk, addr))
    }).collect::<Result<Vec<_>, String>>()?;
    Ok(TxOutputs { height, outputs })
}

/// Everything read for one delivery, in one place, so the decision is a pure function of it.
pub struct Evidence<'a> {
    pub rune: RuneId,
    /// decoded from the etching, not taken from an indexer
    pub name: &'a str,
    pub recipient_script: &'a [u8],
    pub tx: &'a TxOutputs,
    pub ord_height: u64,
    /// ord's reply for each output paying the recipient, by vout
    pub ord: &'a [(u32, OrdOutput)],
    /// Ordinals Wallet's balance for the recipient's address
    pub holdings: &'a [Holding],
}

/// The delivery outputs: those paying the quote's recipient script.
pub fn delivery_outputs(tx: &TxOutputs, recipient_script: &[u8]) -> Vec<(u32, Option<String>)> {
    let want = hex::encode(recipient_script);
    tx.outputs.iter().filter(|(_, spk, _)| *spk == want).map(|(n, _, a)| (*n, a.clone())).collect()
}

/// Units of `rune` delivered to the recipient, or why there is no answer yet.
pub fn delivered(e: &Evidence) -> Result<u128, String> {
    let height = e.tx.height.ok_or("the transaction is not confirmed")?;
    if e.ord_height < height {
        return Err(format!("ord has indexed to {} and the transaction is in {height}: not yet", e.ord_height));
    }
    let outs = delivery_outputs(e.tx, e.recipient_script);
    if outs.is_empty() {
        return Ok(0); // nothing paid the recipient at all, so nothing was delivered to it
    }
    let mut units: u128 = 0;
    for (vout, _) in &outs {
        match e.ord.iter().find(|(v, _)| v == vout).map(|(_, o)| o) {
            Some(OrdOutput::Found(runes)) => units = units.checked_add(*runes.get(e.name).unwrap_or(&0)).ok_or("overflow")?,
            Some(OrdOutput::NotFound) => return Err(format!("output {vout} was spent before ord could be read: no answer")),
            None => return Err(format!("output {vout} was not read")),
        }
    }
    // corroboration: the address holds this id, under this name, at least this much
    let held = e.holdings.iter().find(|h| h.id == e.rune);
    match held {
        None if units == 0 => Ok(0),
        None => Err(format!("ord counts {units} of {} but Ordinals Wallet shows none at the address: not yet, or wrong", e.name)),
        Some(h) if h.name != e.name => Err(format!("Ordinals Wallet names {}:{} {:?}, the etching names it {:?}", e.rune.block,
            e.rune.tx, h.name, e.name)),
        Some(h) if h.amount < units => Err(format!("ord counts {units}, the address holds {} per Ordinals Wallet", h.amount)),
        Some(_) => Ok(units),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> Vec<u8> {
        std::fs::read(format!("{}/tests/fixtures/rates-indexer/{name}.json", env!("CARGO_MANIFEST_DIR"))).expect("fixture")
    }
    fn etching(name: &str) -> (RuneId, Vec<Vec<u8>>) {
        let v: serde_json::Value = serde_json::from_slice(&fixture(name)).unwrap();
        (parse_rune_id(v["id"].as_str().unwrap()).unwrap(), vec![hex::decode(v["script"].as_str().unwrap()).unwrap()])
    }

    #[test]
    fn names_are_decoded_from_real_etchings_exactly_as_ord_prints_them() {
        let (id, scripts) = etching("etching-840000-3");
        assert_eq!(etched_name(id, &scripts), Ok("DOG•GO•TO•THE•MOON".into()));
        let (id, scripts) = etching("etching-840000-28");
        assert_eq!(etched_name(id, &scripts), Ok("RSIC•GENESIS•RUNE".into()));
        assert_eq!(etched_name(RuneId { block: 1, tx: 0 }, &[]), Ok("UNCOMMON•GOODS".into()), "the genesis rune");
    }

    #[test]
    fn the_name_alphabet_and_the_spacers_follow_the_protocol() {
        assert_eq!((letters(0), letters(25), letters(26), letters(27)), ("A".into(), "Z".into(), "AA".into(), "AB".into()));
        assert_eq!(spaced(letters_to(b"ABC"), 0b01), "A•BC");
        assert_eq!(spaced(letters_to(b"ABC"), 0b11), "A•B•C");
        assert_eq!(spaced(letters_to(b"ABC"), 0b100), "ABC", "no spacer after the last letter");
        assert_eq!(letters(u128::MAX), "BCGDENLQRQWDSLRUGSNLBTMFIJAV");
        for w in [&b"A"[..], b"Z", b"AA", b"DOGGOTOTHEMOON", b"UNCOMMONGOODS"] {
            assert_eq!(letters(letters_to(w)).as_bytes(), w, "round trip");
        }
    }
    /// The inverse of `letters`: bijective base 26, so "A" = 0 and "AA" = 26.
    fn letters_to(s: &[u8]) -> u128 {
        s.iter().fold(0u128, |n, c| n * 26 + (c - b'A') as u128 + 1) - 1
    }

    #[test]
    fn an_etching_with_no_name_gets_the_reserved_one_and_a_non_etching_is_refused() {
        // OP_RETURN OP_13, push [Flags=2, 1 (etching)]: no Rune tag → reserved name for 900000:7
        let script = vec![0x6a, 0x5d, 0x02, 0x02, 0x01];
        let reserved = etched_name(RuneId { block: 900_000, tx: 7 }, &[script]).unwrap();
        assert_eq!(reserved, letters(RESERVED + ((900_000u128) << 32 | 7)));
        assert!(reserved.starts_with("AAAAAAAAAAAAA"), "{reserved}");
        // flags without the etching bit
        assert!(etched_name(RuneId { block: 5, tx: 1 }, &[vec![0x6a, 0x5d, 0x02, 0x02, 0x02]]).unwrap_err().contains("does not etch"));
        // a script with an opcode where a push must be: a cenotaph
        assert!(etched_name(RuneId { block: 5, tx: 1 }, &[vec![0x6a, 0x5d, 0x51]]).unwrap_err().contains("cenotaph"));
        assert!(etched_name(RuneId { block: 5, tx: 1 }, &[vec![0x51, 0x20]]).unwrap_err().contains("no runestone"));
    }

    #[test]
    fn ord_replies_read_as_runes_empty_or_spent() {
        let found = parse_ord_utxo(200, &fixture("ord-utxo-runes")).unwrap();
        assert_eq!(found, OrdOutput::Found(BTreeMap::from([("UNCOMMON•GOODS".to_string(), 3586)])));
        assert_eq!(parse_ord_utxo(200, &fixture("ord-utxo-empty")).unwrap(), OrdOutput::Found(BTreeMap::new()));
        // ord answers a spent output with 200? it answers 404 with this text; either reads as NotFound
        let spent = fixture("ord-utxo-spent");
        assert_eq!(parse_ord_utxo(404, &spent).unwrap(), OrdOutput::NotFound);
        assert_eq!(parse_ord_utxo(200, &spent).unwrap(), OrdOutput::NotFound);
        assert!(parse_ord_utxo(406, b"JSON API disabled").is_err());
        assert!(parse_ord_utxo(200, br#"{"runes":{"BIG":{"amount":1e30}}}"#).unwrap_err().contains("beyond"), "no float amounts");
    }

    #[test]
    fn ordinals_wallet_rows_carry_id_name_and_an_exact_amount() {
        let h = parse_ow_balance(200, &fixture("ow-rune-balance")).unwrap();
        assert_eq!(h, vec![Holding { id: RuneId { block: 1, tx: 0 }, name: "UNCOMMON•GOODS".into(), amount: 7322 }]);
        assert_eq!(parse_ow_balance(200, &fixture("ow-rune-balance-none")).unwrap(), vec![]);
        assert_eq!(parse_ow_balance(200, br#"[{"rune_id":"840000:3","name":"X","amount":"340282366920938463463374607431768211455"}]"#)
            .unwrap()[0].amount, u128::MAX, "past u64, exactly");
        assert!(parse_ow_balance(400, br#"{"error":true}"#).is_err());
    }

    fn real_delivery() -> (TxOutputs, Vec<u8>) {
        let tx = parse_esplora_tx(200, &fixture("esplora-tx-runes")).unwrap();
        // output 0 of ec2eb840… pays bc1qat3y… and holds 3,586 UNCOMMON•GOODS per ord
        let spk = hex::decode(&tx.outputs[0].1).unwrap();
        (tx, spk)
    }

    #[test]
    fn a_real_delivery_reads_as_ord_counts_it_when_the_address_balance_backs_it() {
        let (tx, spk) = real_delivery();
        assert!(tx.height.is_some());
        let ord = vec![(0u32, parse_ord_utxo(200, &fixture("ord-utxo-runes")).unwrap())];
        let holdings = parse_ow_balance(200, &fixture("ow-rune-balance")).unwrap();
        let e = Evidence { rune: RuneId { block: 1, tx: 0 }, name: "UNCOMMON•GOODS", recipient_script: &spk, tx: &tx,
            ord_height: tx.height.unwrap(), ord: &ord, holdings: &holdings };
        assert_eq!(delivered(&e), Ok(3586), "3,586 in the output; the address holds 7,322 across two outputs");
    }

    #[test]
    fn every_way_the_evidence_can_fail_to_agree_gives_no_answer() {
        let (tx, spk) = real_delivery();
        let h = tx.height.unwrap();
        let ord = vec![(0u32, parse_ord_utxo(200, &fixture("ord-utxo-runes")).unwrap())];
        let holdings = parse_ow_balance(200, &fixture("ow-rune-balance")).unwrap();
        let base = |ord_height, ord: &'_ [(u32, OrdOutput)], holdings: &'_ [Holding], name: &'static str| {
            delivered(&Evidence { rune: RuneId { block: 1, tx: 0 }, name, recipient_script: &spk, tx: &tx, ord_height, ord, holdings })
        };
        assert!(base(h - 1, &ord, &holdings, "UNCOMMON•GOODS").unwrap_err().contains("not yet"), "ord behind the block");
        assert!(base(h, &[(0, OrdOutput::NotFound)], &holdings, "UNCOMMON•GOODS").unwrap_err().contains("spent"));
        assert!(base(h, &[], &holdings, "UNCOMMON•GOODS").unwrap_err().contains("not read"));
        assert!(base(h, &ord, &[], "UNCOMMON•GOODS").unwrap_err().contains("shows none"), "ord counts, the address doesn't back it");
        let renamed = vec![Holding { name: "SOMETHING•ELSE".into(), ..holdings[0].clone() }];
        assert!(base(h, &ord, &renamed, "UNCOMMON•GOODS").unwrap_err().contains("names"), "an indexer naming the id differently");
        let short = vec![Holding { amount: 10, ..holdings[0].clone() }];
        assert!(base(h, &ord, &short, "UNCOMMON•GOODS").unwrap_err().contains("holds 10"));
        // a different rune in the output than the one bought: zero of ours, and the address holds none of ours → 0, consistent
        assert_eq!(base(h, &ord, &[], "DOG•GO•TO•THE•MOON"), Ok(0));
        // a cenotaph: ord shows the output empty, the address shows nothing new → 0
        assert_eq!(base(h, &[(0, OrdOutput::Found(BTreeMap::new()))], &[], "UNCOMMON•GOODS"), Ok(0));
        // unconfirmed
        let pending = TxOutputs { height: None, ..tx.clone() };
        assert!(delivered(&Evidence { rune: RuneId { block: 1, tx: 0 }, name: "UNCOMMON•GOODS", recipient_script: &spk,
            tx: &pending, ord_height: h, ord: &ord, holdings: &holdings }).unwrap_err().contains("not confirmed"));
        // nothing paid the recipient
        assert_eq!(delivered(&Evidence { rune: RuneId { block: 1, tx: 0 }, name: "UNCOMMON•GOODS", recipient_script: b"\x51\x20",
            tx: &tx, ord_height: h, ord: &ord, holdings: &holdings }), Ok(0));
    }
}
