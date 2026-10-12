//! Sending a signed EVM transaction, and why the send is never the source of truth.
//!
//! Audit finding B4. A replicated HTTPS outcall is made by every node of the subnet and their replies must agree. For a read
//! that is free integrity; for `eth_sendRawTransaction` it cannot work: the provider accepts the first copy and answers every
//! later copy "already known", so the replies differ, consensus fails, and the canister is told the call failed about a
//! transaction that was in fact sent.
//!
//! So the send is made by **one** node (`is_replicated: false`), and nothing it says is believed. It is one replica's report of
//! one provider's reply. What is believed is the read-back: `eth_getTransactionByHash` for the hash we computed ourselves, made
//! replicated through [`crate::canister`]'s `rpc`, where every node and every provider must agree. The hash is ours because it is
//! the keccak of the signed bytes, so a provider cannot talk us into watching the wrong transaction.
//!
//! Resending the same signed bytes is safe: they carry one nonce, so at most one of them can ever be mined. That is the EVM half
//! of the rule the Bitcoin side already follows — save the bytes before sending, and after a restart send the SAME bytes.
//!
//! This module is pure: no network, no replica. It classifies a provider's reply and reads the chain id out of a signed
//! transaction, and both are tested here.

/// What one provider said to one send. A report, not a fact: see the module docs.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, candid::CandidType)]
pub enum SendOutcome {
    /// It returned our hash.
    Accepted,
    /// It already had it — from an earlier send of the same bytes, or another provider's gossip. As good as accepted.
    AlreadyKnown,
    /// Its nonce is used. Either this transaction was already mined, or ANOTHER transaction with the same nonce was; only the
    /// read-back can say which, so this is never read as success.
    NonceTooLow,
    /// The sender cannot pay for the gas.
    NoFunds,
    /// It returned a hash that is not the keccak of the bytes we sent. A provider that does this is not to be believed about
    /// anything.
    WrongHash(String),
    /// Any other refusal, with the provider's message (shortened).
    Refused(String),
    /// Not a JSON-RPC reply at all: an HTTP error page, a timeout body, garbage.
    Unreadable(String),
}

impl SendOutcome {
    /// Whether this reply says the provider has the transaction. Still only a report.
    pub fn has_it(&self) -> bool {
        matches!(self, SendOutcome::Accepted | SendOutcome::AlreadyKnown)
    }
}

/// Classify one provider's reply to `eth_sendRawTransaction`. `our_hash` is the keccak of the bytes sent, `0x`-prefixed hex.
pub fn classify(status: u16, body: &[u8], our_hash: &str) -> SendOutcome {
    let short = |s: &str| s.chars().take(200).collect::<String>();
    let v: serde_json::Value = match serde_json::from_slice(body) {
        Ok(v) => v,
        Err(_) => return SendOutcome::Unreadable(format!("HTTP {status}: {}", short(&String::from_utf8_lossy(body)))),
    };
    if let Some(r) = v.get("result").and_then(|r| r.as_str()) {
        return if r.eq_ignore_ascii_case(our_hash) { SendOutcome::Accepted } else { SendOutcome::WrongHash(short(r)) };
    }
    let Some(msg) = v.get("error").and_then(|e| e.get("message")).and_then(|m| m.as_str()) else {
        return SendOutcome::Unreadable(format!("HTTP {status}: no result and no error: {}", short(&v.to_string())));
    };
    let m = msg.to_ascii_lowercase();
    // geth "already known"; older geth "known transaction"; nethermind "AlreadyKnown"; erigon "already imported"
    if m.contains("already known") || m.contains("known transaction") || m.contains("alreadyknown") || m.contains("already imported") {
        SendOutcome::AlreadyKnown
    } else if m.contains("nonce too low") {
        SendOutcome::NonceTooLow
    } else if m.contains("insufficient funds") {
        SendOutcome::NoFunds
    } else {
        SendOutcome::Refused(short(msg))
    }
}

/// The chain id a signed transaction commits to: EIP-1559 (`0x02`) and EIP-2930 (`0x01`) carry it as the first item of their
/// RLP list. Legacy transactions are refused: their chain id is folded into `v`, and the buyer never builds one.
pub fn chain_id_of(raw: &[u8]) -> Result<u64, String> {
    let (&kind, rest) = raw.split_first().ok_or("an empty transaction")?;
    if kind != 0x01 && kind != 0x02 {
        return Err(format!("not a typed transaction (first byte 0x{kind:02x}); only EIP-2930 and EIP-1559 are sent"));
    }
    let (list, _) = rlp_item(rest, true)?;
    let (first, _) = rlp_item(list, false)?;
    if first.len() > 8 {
        return Err("chain id wider than 64 bits".into());
    }
    if first.first() == Some(&0) {
        return Err("chain id with a leading zero is not canonical RLP".into());
    }
    Ok(first.iter().fold(0u64, |a, &b| (a << 8) | b as u64))
}

/// One RLP item at the front of `b`: its payload, and what follows it. `want_list` says which kind is expected.
fn rlp_item(b: &[u8], want_list: bool) -> Result<(&[u8], &[u8]), String> {
    let (&p, rest) = b.split_first().ok_or("RLP ended early")?;
    let (is_list, off, len) = match p {
        0x00..=0x7f => (false, 0usize, 1usize),
        0x80..=0xb7 => (false, 1, (p - 0x80) as usize),
        0xb8..=0xbf => { let n = (p - 0xb7) as usize; (false, 1 + n, be_len(rest, n)?) }
        0xc0..=0xf7 => (true, 1, (p - 0xc0) as usize),
        0xf8..=0xff => { let n = (p - 0xf7) as usize; (true, 1 + n, be_len(rest, n)?) }
    };
    if is_list != want_list {
        return Err(format!("RLP: expected a {}", if want_list { "list" } else { "string" }));
    }
    let start = if p <= 0x7f { 0 } else { off };
    let end = start.checked_add(len).ok_or("RLP length overflows")?;
    if end > b.len() {
        return Err("RLP item runs past the end".into());
    }
    Ok((&b[start..end], &b[end..]))
}

fn be_len(b: &[u8], n: usize) -> Result<usize, String> {
    if n > 4 || b.len() < n {
        return Err("RLP length prefix too long or truncated".into());
    }
    Ok(b[..n].iter().fold(0usize, |a, &x| (a << 8) | x as usize))
}

/// The fields of a receipt the buyer acts on, and nothing else. Providers agree on these or they disagree about the chain;
/// anything else is presentation. Found by the B4 rehearsal: Robinhood Chain's own node adds Arbitrum's `gasUsedForL1` and
/// `l1BlockNumber` to every receipt and its explorer's RPC does not, so comparing whole bodies could never agree on one.
pub fn receipt_view(r: &serde_json::Value) -> serde_json::Value {
    project(r, &["transactionHash", "blockHash", "blockNumber", "from", "to", "status"])
}

/// The same for a transaction still waiting to be mined: which one it is, and whose.
pub fn tx_view(t: &serde_json::Value) -> serde_json::Value {
    project(t, &["hash", "from", "nonce"])
}

/// Review round 4 (C4-M-1): a transaction as the payout judge reads it: who sent it where, the value and the input.
pub fn payout_view(t: &serde_json::Value) -> serde_json::Value {
    project(t, &["hash", "from", "to", "value", "input", "blockNumber"])
}

/// Review round 5 (F5-M-1): a block's transactions (`eth_getBlockByNumber(n, true)`) projected to those sent by `us`, as
/// `{"result": [[hash, nonce], …]}` lower-cased: how the canister finds which transaction of ours used a nonce without the
/// transaction index (which is what failed when no provider shows a mined transaction of ours).
pub fn our_txs_body(body: &[u8], us: &[u8; 20]) -> Vec<u8> {
    let Some(r) = serde_json::from_slice::<serde_json::Value>(body).ok().and_then(|v| v.get("result").cloned()) else {
        return br#"{"error":"unusable reply"}"#.to_vec();
    };
    if r.is_null() {
        return br#"{"result":null}"#.to_vec();
    }
    let me = format!("0x{}", hex::encode(us));
    let txs: Vec<serde_json::Value> = r.get("transactions").and_then(|t| t.as_array()).map(|t| t.iter().filter(|x| {
        x.get("from").and_then(|f| f.as_str()).is_some_and(|f| f.eq_ignore_ascii_case(&me))
    }).map(|x| serde_json::json!([x.get("hash").and_then(|h| h.as_str()).unwrap_or("").to_ascii_lowercase(),
        x.get("nonce").and_then(|n| n.as_str()).unwrap_or("").to_ascii_lowercase()])).collect()).unwrap_or_default();
    serde_json::json!({ "result": txs }).to_string().into_bytes()
}

/// `null` stays `null` ("not found" is an answer providers must also agree on); otherwise only `keys`, hex lower-cased so a
/// provider's capitalisation is not a disagreement. A missing key is kept as `null`, so a provider that leaves out a field we
/// act on disagrees rather than slipping through.
fn project(v: &serde_json::Value, keys: &[&str]) -> serde_json::Value {
    if v.is_null() {
        return serde_json::Value::Null;
    }
    let mut m = serde_json::Map::new();
    for k in keys {
        let x = v.get(*k).cloned().unwrap_or(serde_json::Value::Null);
        let x = match x {
            serde_json::Value::String(s) => serde_json::Value::String(s.to_ascii_lowercase()),
            other => other,
        };
        m.insert((*k).to_string(), x);
    }
    serde_json::Value::Object(m)
}

#[cfg(test)]
mod tests {
    use super::*;

    const H: &str = "0xb0de664d35e3df8523466d20f2e5c02121bd762b69bcbfcb417c2b6dbd8f95ff";

    #[test]
    fn f5_m1_a_blocks_transactions_are_projected_to_ours_hash_and_nonce() {
        let us = [0x76u8; 20];
        let me = format!("0x{}", hex::encode(us));
        let body = serde_json::json!({ "jsonrpc": "2.0", "id": 1, "result": { "number": "0x10", "transactions": [
            { "hash": "0xAA", "from": me.to_uppercase().replace("0X", "0x"), "nonce": "0x7", "input": "0x1234" },
            { "hash": "0xbb", "from": "0x0000000000000000000000000000000000000001", "nonce": "0x7" },
            { "hash": "0xcc", "from": me, "nonce": "0x8" } ] } }).to_string();
        let v: serde_json::Value = serde_json::from_slice(&our_txs_body(body.as_bytes(), &us)).unwrap();
        assert_eq!(v["result"], serde_json::json!([["0xaa", "0x7"], ["0xcc", "0x8"]]));
        assert_eq!(our_txs_body(br#"{"result":null}"#, &us), br#"{"result":null}"#.to_vec());
        assert!(String::from_utf8(our_txs_body(b"nope", &us)).unwrap().contains("error"));
    }

    #[test]
    fn the_replies_that_broke_consensus_now_mean_the_same_thing() {
        // B4 itself: the first copy gets the hash, every later copy "already known". Both say "the provider has it".
        let first = classify(200, format!(r#"{{"jsonrpc":"2.0","id":1,"result":"{H}"}}"#).as_bytes(), H);
        let later = classify(200, br#"{"jsonrpc":"2.0","id":1,"error":{"code":-32000,"message":"already known"}}"#, H);
        assert_eq!((first.clone(), later.clone()), (SendOutcome::Accepted, SendOutcome::AlreadyKnown));
        assert!(first.has_it() && later.has_it());
        // the other clients' spellings of it
        for m in ["known transaction: 0xabc", "AlreadyKnown", "transaction already imported"] {
            let b = format!(r#"{{"error":{{"code":-32010,"message":"{m}"}}}}"#);
            assert_eq!(classify(200, b.as_bytes(), H), SendOutcome::AlreadyKnown, "{m}");
        }
        // case-insensitive on our own hash
        assert_eq!(classify(200, format!(r#"{{"result":"{}"}}"#, H.to_uppercase().replace("0X", "0x")).as_bytes(), H), SendOutcome::Accepted);
    }

    #[test]
    fn nothing_ambiguous_is_read_as_success() {
        let e = |m: &str| classify(200, format!(r#"{{"error":{{"code":-32000,"message":"{m}"}}}}"#).as_bytes(), H);
        assert_eq!(e("nonce too low: next nonce 5, tx nonce 4"), SendOutcome::NonceTooLow);
        assert!(!SendOutcome::NonceTooLow.has_it(), "a used nonce may be someone else's transaction: only the read-back can tell");
        // the exact refusal the script rehearsal met before it was funded
        assert_eq!(e("insufficient funds for gas * price + value: balance 0, tx cost 21000"), SendOutcome::NoFunds);
        assert_eq!(e("max fee per gas less than block base fee"), SendOutcome::Refused("max fee per gas less than block base fee".into()));
        assert_eq!(classify(200, br#"{"result":"0x1111111111111111111111111111111111111111111111111111111111111111"}"#, H),
            SendOutcome::WrongHash("0x1111111111111111111111111111111111111111111111111111111111111111".into()));
        assert!(matches!(classify(502, b"<html>Bad Gateway</html>", H), SendOutcome::Unreadable(s) if s.starts_with("HTTP 502")));
        assert!(matches!(classify(200, br#"{"jsonrpc":"2.0","id":1}"#, H), SendOutcome::Unreadable(_)));
        assert!(matches!(classify(200, br#"{"result":42}"#, H), SendOutcome::Unreadable(_)));
    }

    #[test]
    fn a_signed_transaction_says_which_chain_it_is_for() {
        // EIP-1559 for chain 46630 (0xb626): 0x02 ‖ rlp([chainId, nonce, ...]); only the front matters here
        let tx = |chain: &[u8]| {
            let mut items = vec![0x80 + chain.len() as u8];
            items.extend_from_slice(chain);
            items.extend_from_slice(&[0x80, 0x01, 0x02]); // nonce 0, then anything
            let mut t = vec![0x02, 0xc0 + items.len() as u8];
            t.extend(items);
            t
        };
        assert_eq!(chain_id_of(&tx(&[0xb6, 0x26])), Ok(46630));
        assert_eq!(chain_id_of(&tx(&[0x12, 0x37])), Ok(4663), "mainnet is read as mainnet, so the caller can refuse it");
        assert_eq!(chain_id_of(&[0x01, 0xc2, 0x01, 0x80]), Ok(1), "EIP-2930 too; a single byte below 0x80 is itself");
        assert!(chain_id_of(&[0xf8, 0x6b]).unwrap_err().contains("not a typed transaction"), "legacy is refused");
        assert!(chain_id_of(&[]).is_err());
        assert!(chain_id_of(&[0x02, 0xc5, 0x84, 0xb6]).unwrap_err().contains("past the end"), "truncated");
        assert!(chain_id_of(&[0x02, 0xc3, 0x82, 0x00, 0x01]).unwrap_err().contains("leading zero"));
        assert!(chain_id_of(&[0x02, 0x82, 0xb6, 0x26]).unwrap_err().contains("expected a list"));
    }

    #[test]
    fn two_real_receipts_that_differ_only_in_arbitrum_extras_agree_on_what_the_buyer_acts_on() {
        // Both providers' receipts for the canister's own broadcast (chain 46630, 23 Sep), trimmed of logsBloom.
        let node = serde_json::json!({ "blockHash": "0x88a078c4f41d0011b1048b7e2d36f43a95a79e29890b75a729e9234110416e46",
            "blockNumber": "0x7545b99", "contractAddress": null, "cumulativeGasUsed": "0x61b1", "effectiveGasPrice": "0x989680",
            "from": "0x5b0b0649f0306d08b4c0ad37406a7082fc25ebc2", "gasUsed": "0x61b1", "gasUsedForL1": "0xfa9", "l1BlockNumber": "0xb37748",
            "logs": [], "status": "0x1", "to": "0x5b0b0649f0306d08b4c0ad37406a7082fc25ebc2",
            "transactionHash": "0x2eebfa6ef9ba9cf1cb65e3769d751c6257fc0d9a289e1297829c14b2642a6b00", "transactionIndex": "0x1", "type": "0x2" });
        let mut explorer = node.clone();
        let o = explorer.as_object_mut().unwrap();
        o.remove("gasUsedForL1");
        o.remove("l1BlockNumber");
        assert_ne!(node, explorer, "whole bodies differ: this is why the read-back could never agree");
        assert_eq!(receipt_view(&node), receipt_view(&explorer));
        // ...but a real disagreement still is one: another status, another block, another sender, or a field left out
        for (k, v) in [("status", "0x0"), ("blockNumber", "0x7545b9a"), ("from", "0x0000000000000000000000000000000000000001")] {
            let mut other = explorer.clone();
            other[k] = serde_json::json!(v);
            assert_ne!(receipt_view(&node), receipt_view(&other), "{k}");
        }
        let mut partial = explorer.clone();
        partial.as_object_mut().unwrap().remove("status");
        assert_ne!(receipt_view(&node), receipt_view(&partial), "a provider that omits status does not agree by omission");
        // capitalisation is not a disagreement; "not found" from both is agreement on not found
        let mut shouty = explorer.clone();
        shouty["from"] = serde_json::json!("0x5B0B0649F0306D08B4C0AD37406A7082FC25EBC2");
        assert_eq!(receipt_view(&node), receipt_view(&shouty));
        assert_eq!(receipt_view(&serde_json::Value::Null), serde_json::Value::Null);
        assert_eq!(tx_view(&serde_json::json!({"hash": "0xAB", "from": "0xCD", "nonce": "0x2", "gasPrice": "0x1"})),
            serde_json::json!({"hash": "0xab", "from": "0xcd", "nonce": "0x2"}));
    }

    /// The script rehearsal's own signed bytes, as mined on chain 46630 in block 122882989 (autobuyer/.evm-rehearsal.json).
    const MINED: &str = "02f86882b62601808402625a0082a1b0945b0b0649f0306d08b4c0ad37406a7082fc25ebc28080c080a0ed058429f5854747ef2599377fa0a8c13d72df31bbc556113f315b556286d774a036676166f86adb87da22ff1a6c1ff288ba3de6752966b75fbf4b46463be90e86";
    const MINED_HASH: &str = "0xb0de664d35e3df8523466d20f2e5c02121bd762b69bcbfcb417c2b6dbd8f95ff";

    #[test]
    fn the_rehearsal_transaction_that_was_mined_reads_as_testnet_and_hashes_to_its_mined_hash() {
        use sha3::{Digest, Keccak256};
        let raw = hex::decode(MINED).unwrap();
        assert_eq!(chain_id_of(&raw), Ok(46630));
        // the hash we watch is ours: the keccak of the bytes, and the chain agreed with it when it mined them
        let ours = format!("0x{}", hex::encode(Keccak256::digest(&raw)));
        assert_eq!(ours, MINED_HASH);
        assert_eq!(classify(200, format!(r#"{{"result":"{MINED_HASH}"}}"#).as_bytes(), &ours), SendOutcome::Accepted);
    }
}
