//! Pinned reads (`docs/v2/pinned-reads.md`): every EVM state read names one block that every IC node and every provider names
//! identically, because at ten blocks a second `"latest"` is a different question on every node and every provider.
//!
//! This module is pure: the rounding a transform applies, the choice of the pin from every provider's rounded head, the agreement
//! on the pinned block, and the receipt-at-the-pin projection. The outcalls that feed it live in `canister.rs`.

use crate::evmsend::receipt_view;
use serde_json::Value;

/// Heads are rounded down to a multiple of this many blocks before consensus: ≈ 103 s at Robinhood Chain's ≈ 0.1 s blocks.
/// Larger means fewer boundary disagreements between nodes and a deeper pin; Robinhood's own RPC keeps state ≈ 6,200 blocks back
/// (bisected 2026-09-26: 6,201 answers, 6,250 does not), which bounds it at about 2,000 (the pin is up to 2 × BUCKET + lag deep).
pub const BUCKET: u64 = 1_024;

/// `head` rounded down to a multiple of `b`.
pub fn round_head(head: u64, b: u64) -> u64 {
    head / b * b
}

/// The context bytes a transform is handed: one u64, big-endian.
pub fn context_u64(n: u64) -> Vec<u8> {
    n.to_be_bytes().to_vec()
}

pub fn u64_of_context(c: &[u8]) -> Option<u64> {
    Some(u64::from_be_bytes(c.try_into().ok()?))
}

/// What a transform returns for an answer it cannot use: a JSON-RPC error, identical on every node, so the read fails rather
/// than one node's garbage winning.
fn unusable() -> Vec<u8> {
    br#"{"error":"unusable reply"}"#.to_vec()
}

fn result_of(body: &[u8]) -> Option<Value> {
    serde_json::from_slice::<Value>(body).ok()?.get("result").cloned()
}

fn quantity(v: &Value) -> Option<u64> {
    u64::from_str_radix(v.as_str()?.strip_prefix("0x")?, 16).ok()
}

/// The transform for `eth_blockNumber`: the head rounded down to a multiple of `b`, as `{"result":"0x…"}`.
pub fn head_bucket_body(body: &[u8], b: u64) -> Vec<u8> {
    match result_of(body).as_ref().and_then(quantity) {
        Some(h) if b > 0 => serde_json::json!({ "result": format!("0x{:x}", round_head(h, b)) }).to_string().into_bytes(),
        _ => unusable(),
    }
}

/// The pin, from every provider's rounded head. All must have answered (the caller fails on any error); the rounded heads may
/// differ by at most one bucket — a provider further behind is stale, one further ahead is wrong, and either way nothing is
/// believed. The pin is one bucket below the lowest, so it exists on every provider and is ≥ `b` blocks behind every head.
pub fn choose_pin(rounded: &[u64], b: u64) -> Result<u64, String> {
    let lo = *rounded.iter().min().ok_or("no providers")?;
    let hi = *rounded.iter().max().ok_or("no providers")?;
    if hi - lo > b {
        return Err(format!("providers' heads are {} blocks apart (more than one bucket of {b}): one is stale or wrong", hi - lo));
    }
    lo.checked_sub(b).ok_or_else(|| "chain too short to pin".into())
}

/// Review round 4 (C4-H-1): what one chain's pins have shown so far, so no read of it goes back. `choose_pin` accepts heads one
/// bucket apart, so a provider whose head falls back across a bucket boundary moves the pin back a whole bucket between two ticks;
/// at that lower pin our own mined transactions are unseen (the nonce is one we already used, the balance the one before our
/// spend). Every read of the chain is therefore taken only at a pin at or above the highest pin already read (`pin`), and a nonce
/// read is believed only at or above the highest nonce already read (`nonce`): lower, the read waits a tick; it is never a fact.
/// Both only grow. Kept in stable memory (`ethflow::EthState::pin_floors`).
#[derive(candid::CandidType, serde::Serialize, serde::Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PinFloor {
    pub pin: u64,
    pub nonce: u64,
}

impl PinFloor {
    /// A fresh pin `n` of this chain: refused below the highest pin already read, else it raises the floor.
    pub fn admit_pin(&mut self, what: &str, n: u64) -> Result<(), String> {
        if n < self.pin {
            return Err(format!("{what}: the pin {n} is below block {} (a pin already read: a provider's head fell back): nothing is \
                read there this tick (C4-H-1)", self.pin));
        }
        self.pin = n;
        Ok(())
    }
    /// A nonce read of our address: refused below the highest already read (a pin behind our own mined transactions), else it
    /// raises the floor. Nothing is ever signed below it.
    pub fn admit_nonce(&mut self, what: &str, n: u64) -> Result<u64, String> {
        if n < self.nonce {
            return Err(format!("{what}: our nonce at this pin is {n}, below {} (already read: the pin is behind our own mined \
                transactions): nothing is signed there (C4-H-1)", self.nonce));
        }
        self.nonce = n;
        Ok(n)
    }
}

/// Both chains' floors: the home chain's (4663: every home read of every part of the canister, `canister::pin`) and the
/// Ethereum NFT side's (`ethnet::EthNet::pin`).
#[derive(candid::CandidType, serde::Serialize, serde::Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PinFloors {
    pub home: PinFloor,
    pub eth: PinFloor,
}

/// A block, projected to what the pin needs: `{number, hash, baseFeePerGas}`, hex lower-cased; `null` stays `null`.
pub fn block_view(v: &Value) -> Value {
    if v.is_null() {
        return Value::Null;
    }
    let s = |k: &str| v.get(k).and_then(|x| x.as_str()).map(|x| Value::String(x.to_ascii_lowercase())).unwrap_or(Value::Null);
    serde_json::json!({ "number": s("number"), "hash": s("hash"), "baseFeePerGas": s("baseFeePerGas") })
}

/// The transform for `eth_getBlockByNumber(pin)`: the projection, so the consensus body is small and provider-neutral.
pub fn block_body(body: &[u8]) -> Vec<u8> {
    match result_of(body) {
        Some(b) => serde_json::json!({ "result": block_view(&b) }).to_string().into_bytes(),
        None => unusable(),
    }
}

/// The agreed pin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pin {
    pub number: u64,
    /// `0x…`, lower-case: what every state read names (EIP-1898 `{"blockHash": …}`).
    pub hash: String,
    /// The pinned block's base fee, in wei: the fee the loop caps at `FEE_MULTIPLE` times.
    pub base_fee: u128,
}

/// Every provider's view of the pinned block must be the same block, at the number asked for. A different hash at the same
/// number is a reorg in progress or a lie; either way nothing is believed.
pub fn agree_block(views: &[Value], want: u64) -> Result<Pin, String> {
    let first = views.first().ok_or("no providers")?;
    if views.iter().any(|v| v != first) {
        return Err(format!("providers disagree on block {want}: a reorg or a wrong provider; nothing is believed"));
    }
    if first.is_null() {
        return Err(format!("no provider has block {want}"));
    }
    let number = first.get("number").and_then(quantity).ok_or("the pinned block has no readable number")?;
    if number != want {
        return Err(format!("asked for block {want}, given {number}"));
    }
    let hash = first.get("hash").and_then(|h| h.as_str()).filter(|h| h.len() == 66 && h.starts_with("0x"))
        .ok_or("the pinned block has no readable hash")?.to_string();
    let base_fee = first.get("baseFeePerGas").and_then(|x| x.as_str()).and_then(|x| x.strip_prefix("0x"))
        .and_then(|x| u128::from_str_radix(x, 16).ok()).ok_or("the pinned block has no base fee")?;
    Ok(Pin { number, hash, base_fee })
}

/// The transform for `eth_getTransactionReceipt` at pin `pin`: a receipt counts only if its block is at or below the pin, so a
/// missing receipt and one above the pin are the same `null` — every node answers the same thing, and "mined" becomes true at
/// the same pin as the nonce the loop reads. The rest is projected to `receipt_view`'s fields.
pub fn receipt_at_pin_body(body: &[u8], pin: u64) -> Vec<u8> {
    let Some(r) = result_of(body) else { return unusable() };
    let counted = !r.is_null() && r.get("blockNumber").and_then(quantity).is_some_and(|n| n <= pin);
    let result = if counted { receipt_view(&r) } else { Value::Null };
    serde_json::json!({ "result": result }).to_string().into_bytes()
}

/// A block parameter naming the pin by hash (EIP-1898): a provider that has reorganised away from it fails the read instead of
/// answering from another fork.
pub fn at(pin: &Pin) -> Value {
    serde_json::json!({ "blockHash": pin.hash })
}

// ---- the finalized pin (docs/v5/custody-hardening.md, H9) ------------------------------------------------------------------

/// The transform for `eth_getBlockByNumber("finalized")`: the finalized block's number rounded down to a multiple of `b`, as
/// `{"result":"0x…"}`, so nodes asking moments apart agree (Nitro reports the L1-finalized L2 block; Ethereum and Base their own).
pub fn finalized_bucket_body(body: &[u8], b: u64) -> Vec<u8> {
    match result_of(body).as_ref().and_then(|r| r.get("number")).and_then(quantity) {
        Some(n) if b > 0 => serde_json::json!({ "result": format!("0x{:x}", round_head(n, b)) }).to_string().into_bytes(),
        _ => unusable(),
    }
}

/// The transform for `eth_getBlockByNumber("finalized")` read for its CLOCK (audit 2026-09-27-nft, B-L-2 / C-L-6: the H9 rule on the
/// Ethereum NFT side): the finalized block's timestamp rounded down to a multiple of `s` seconds, as `{"result":"0x…"}`. Rounding
/// down is conservative: a ticket resolved at a time strictly before it is in a final block.
pub fn finalized_time_body(body: &[u8], s: u64) -> Vec<u8> {
    match result_of(body).as_ref().and_then(|r| r.get("timestamp")).and_then(quantity) {
        Some(t) if s > 0 => serde_json::json!({ "result": format!("0x{:x}", t - t % s) }).to_string().into_bytes(),
        _ => unusable(),
    }
}

/// Review round 6 (B6-M-1 / B6-L-2 / B6-I-1): the CLASS of a provider's JSON-RPC error, read from its words (the message and a string
/// `data`, where Tenderly puts them: `"invalid params"` with `"Query returned more than 20000 results…"` in `data`), so the scan can
/// tell a rate limit from a range too big to read from a history the provider does not keep, although the words themselves are
/// dropped. The same words give the same class on every replica, so a class never breaks consensus. `None`: the words name none.
/// Measured 2026-09-28 (docs/audit/2026-09-27-nft/fixes-round6.md §1): `busy` — xrpc.cl's `-32005 "XRPC anonymous capacity temporarily
/// busy"` (HTTP 429), Tenderly's `-32005 "rate limit exceeded"` (HTTP 429); `deep` — publicnode's `-32602 "Archive requests require a
/// personal token"`, Robinhood's `"historical state … is not available"`, dRPC's `"Unknown state"`; `limit` — MEV Blocker's
/// `-32005 "query returned more than 10000 results"`, Tenderly's 20,000-result cap, "block range is too wide", "Log response size
/// exceeded", "query timeout exceeded".
pub fn error_class(words: &str) -> Option<&'static str> {
    let l = words.to_ascii_lowercase();
    let any = |ws: &[&str]| ws.iter().any(|w| l.contains(w));
    // review round 7 (B7-I-4): a backend lagging behind the pin ("block range extends beyond current head block", Tenderly) is a
    // transient: read again, never deferred as a range too big
    if any(&["rate limit", "rate-limit", "ratelimit", "too many requests", "request rate", "capacity", "busy", "throttl", "quota",
        "try again later", "error code: 1015", "beyond current head"]) {
        return Some("busy");
    }
    if any(&["archive", "pruned", "missing trie node", "historical state", "unknown state", "first available state", "personal token",
        "state is not available", "state not available"]) {
        return Some("deep");
    }
    if any(&["more than", "too many", "too large", "too wide", "limit exceeded", "exceeds", "exceeded", "response size", "block range",
        "range is too", "range too", "timeout", "timed out", "results"]) {
        return Some("limit");
    }
    None
}

/// Review round 6 (B6-L-2): the error body a transform gives an HTTP 429 answer that carries no JSON-RPC result (MEV Blocker's
/// Cloudflare page is `error code: 1015`): a rate limit, whatever the page says.
pub fn busy_body() -> Vec<u8> {
    serde_json::json!({ "error": { "class": "busy", "code": 429, "reverted": false } }).to_string().into_bytes()
}

/// The transform for a JSON-RPC state read (audit 2026-09-27-nft, B-M-6): the body re-emitted in ONE canonical shape whatever the
/// provider sent, so replicas agree on what the chain said, not on how the provider wrote it. `{"result": …}` with its keys sorted
/// and every string lower-cased (hex); a JSON-RPC error as `{"error":{"code":c,"reverted":b}}` (the message dropped: providers word
/// it differently; `reverted` is whether it says the call reverted: code 3, or "revert" in the message), with review round 6's
/// `"class"` when its words name one (`error_class`); anything else (a rate limit page, HTML, a `result` carrying an error object)
/// as the one `{"error":{"code":"unusable","reverted":false}}`. The status is fixed at 200: a provider's 429 on some replicas is an
/// error either way, and the same error on every replica agrees.
pub fn json_rpc_norm_body(body: &[u8]) -> Vec<u8> {
    fn lower(v: &Value) -> Value {
        match v {
            Value::String(s) => Value::String(s.to_ascii_lowercase()),
            Value::Array(a) => Value::Array(a.iter().map(lower).collect()),
            Value::Object(o) => Value::Object(o.iter().map(|(k, v)| (k.clone(), lower(v))).collect()),
            x => x.clone(),
        }
    }
    let bad = || serde_json::json!({ "error": { "code": "unusable", "reverted": false } }).to_string().into_bytes();
    let Ok(v) = serde_json::from_slice::<Value>(body) else { return bad() };
    if let Some(r) = v.get("result") {
        // a `result` that is an object with an error code in it is a provider's error dressed as data (measured on 1rpc.io)
        if r.get("code").is_some() && (r.get("message").is_some() || r.get("error").is_some()) {
            return bad();
        }
        return serde_json::json!({ "result": lower(r) }).to_string().into_bytes();
    }
    if let Some(e) = v.get("error") {
        let code = e.get("code").and_then(|c| c.as_i64());
        let msg = e.get("message").and_then(|m| m.as_str()).unwrap_or("").to_ascii_lowercase();
        let reverted = code == Some(3) || msg.contains("revert");
        let data = e.get("data").and_then(|d| d.as_str()).unwrap_or("");
        let mut out = serde_json::json!({ "error": { "code": code.map_or(Value::from("unusable"), Value::from), "reverted": reverted } });
        if let Some(c) = error_class(&format!("{msg} {data}")).filter(|_| !reverted) {
            out["error"]["class"] = Value::from(c);
        }
        return out.to_string().into_bytes();
    }
    bad()
}

/// Whether an error from a read through [`json_rpc_norm_body`] is the provider saying the call REVERTED (as opposed to a transport
/// failure, a rate limit or a disagreement). `rpc_once_with` words a JSON-RPC error as `no result: {error}`.
pub fn said_reverted(e: &str) -> bool {
    e.contains("no result:") && e.contains("\"reverted\":true")
}

// The finalized block's number is chosen, bounded, in `evmread::choose_finality`; the logs' projection is `evmread::logs_body`.
// Nothing reads STATE at the finalized block: Robinhood's RPC keeps ≈ 6,200 blocks of it and finality is 8,096–12,023 behind the
// head (measured 2026-09-26; audit D, H-1).

#[cfg(test)]
mod tests {
    use super::*;

    const H: &str = "0xdf1a51bb4113aa00000000000000000000000000000000000000000000000001";

    #[test]
    fn b_m_6_json_rpc_reads_have_one_shape_whatever_the_provider_wrote() {
        // measured 2026-09-27 on 1rpc.io/eth: one eth_call answered in two field orders (and upper-case hex elsewhere)
        let a = br#"{"jsonrpc":"2.0","id":1,"result":"0x000000000000000000000000ABCdef"}"#;
        let b = br#"{"jsonrpc":"2.0","result":"0x000000000000000000000000abcdef","id":1}"#;
        assert_eq!(json_rpc_norm_body(a), json_rpc_norm_body(b));
        assert_eq!(json_rpc_norm_body(a), br#"{"result":"0x000000000000000000000000abcdef"}"#.to_vec());
        // every unusable body is the same error on every replica: a rate-limit page, HTML, "discontinued", an error as a result
        let u = json_rpc_norm_body(b"<html>301 Moved Permanently</html>");
        for x in [&b"Too Many Requests"[..], br#"{"message":"This endpoint has been discontinued"}"#,
            br#"{"jsonrpc":"2.0","id":1,"result":{"code":503,"message":"backend down"}}"#] {
            assert_eq!(json_rpc_norm_body(x), u);
        }
        // a revert is told apart from a failure, whatever the wording
        let r1 = json_rpc_norm_body(br#"{"jsonrpc":"2.0","id":1,"error":{"code":3,"message":"execution reverted: blocked","data":"0x08c379a0"}}"#);
        let r2 = json_rpc_norm_body(br#"{"id":1,"error":{"message":"Execution Reverted","code":-32000},"jsonrpc":"2.0"}"#);
        assert!(String::from_utf8(r1.clone()).unwrap().contains(r#""reverted":true"#));
        assert!(String::from_utf8(r2).unwrap().contains(r#""reverted":true"#));
        let t = json_rpc_norm_body(br#"{"jsonrpc":"2.0","id":1,"error":{"code":-32005,"message":"rate limited"}}"#);
        assert!(String::from_utf8(t).unwrap().contains(r#""reverted":false"#));
        assert!(said_reverted(&format!("chain 1: eth_call: no result: {}", r#"{"code":3,"reverted":true}"#)));
        assert!(!said_reverted("chain 1: eth_call: CallRejected(SysTransient: HTTP 429)"));
        assert!(!said_reverted(r#"no result: {"code":"unusable","reverted":false}"#));
    }

    #[test]
    fn the_finalized_clock_is_rounded_down() {
        let b = br#"{"jsonrpc":"2.0","id":1,"result":{"number":"0x10","timestamp":"0x6000003b"}}"#;
        assert_eq!(finalized_time_body(b, 60), format!(r#"{{"result":"0x{:x}"}}"#, 0x6000003bu64 - 0x6000003bu64 % 60).into_bytes());
        assert_eq!(finalized_time_body(b"{}", 60), unusable());
    }

    fn head(n: u64) -> Vec<u8> {
        format!(r#"{{"jsonrpc":"2.0","id":1,"result":"0x{n:x}"}}"#).into_bytes()
    }
    fn result(b: &[u8]) -> Value {
        serde_json::from_slice::<Value>(b).unwrap()["result"].clone()
    }

    #[test]
    fn nodes_asking_moments_apart_agree_on_the_rounded_head_and_disagree_only_at_a_boundary() {
        // measured: ten blocks a second, providers up to 9 apart. Two nodes 1.5 s apart see heads 15 apart.
        let (a, b) = (70_166_000u64, 70_166_015u64);
        assert_eq!(head_bucket_body(&head(a), BUCKET), head_bucket_body(&head(b), BUCKET), "same window: identical bodies");
        let edge = round_head(a, BUCKET) + BUCKET;
        assert_ne!(head_bucket_body(&head(edge - 1), BUCKET), head_bucket_body(&head(edge), BUCKET),
            "straddling a boundary: different bodies, so the read fails and the tick waits — never a wrong number");
        assert_eq!(result(&head_bucket_body(&head(70_166_755), BUCKET)), serde_json::json!(format!("0x{:x}", 70_166_755 / 1024 * 1024)));
        for bad in [&br#"{"error":{"code":-32000}}"#[..], b"<html>", br#"{"result":"latest"}"#] {
            assert_eq!(head_bucket_body(bad, BUCKET), unusable(), "{}", String::from_utf8_lossy(bad));
        }
    }

    #[test]
    fn the_pin_is_one_bucket_below_the_lowest_head_and_a_stale_or_wild_provider_stops_it() {
        let b = BUCKET;
        // lag inside one bucket, or across one boundary: pinned below the lower
        assert_eq!(choose_pin(&[10 * b, 10 * b], b), Ok(9 * b));
        assert_eq!(choose_pin(&[10 * b, 11 * b], b), Ok(9 * b), "one provider a boundary ahead: pinned below the other");
        // a provider two buckets behind is stale; one far ahead is wrong
        assert!(choose_pin(&[10 * b, 12 * b], b).unwrap_err().contains("stale or wrong"));
        assert!(choose_pin(&[10 * b, 900 * b], b).unwrap_err().contains("stale or wrong"));
        assert!(choose_pin(&[], b).is_err());
        assert!(choose_pin(&[0], b).is_err());
        // depth: between b and 2b + lag behind any real head in the bucket
        let pin = choose_pin(&[round_head(70_166_755, b), round_head(70_166_764, b)], b).unwrap();
        assert!((b..=2 * b + 9).contains(&(70_166_755 - pin)), "{}", 70_166_755 - pin);
    }

    #[test]
    fn c4_h1_a_pin_or_a_nonce_below_one_already_read_is_a_wait_never_a_fact() {
        let b = BUCKET;
        let mut f = PinFloor::default();
        // two providers one bucket apart: the pin is one bucket under the lower; next tick the lower one's head fell back a bucket
        let p1 = choose_pin(&[10 * b, 11 * b], b).unwrap();
        f.admit_pin("4663", p1).unwrap();
        f.admit_nonce("4663", 7).unwrap();
        let back = choose_pin(&[9 * b, 10 * b], b).unwrap();
        assert!(back < p1, "choose_pin accepts it: the heads are one bucket apart");
        assert!(f.admit_pin("4663", back).unwrap_err().contains("below block"), "a pin that moved back is refused");
        assert_eq!(f.pin, p1, "and the floor does not move down");
        assert!(f.admit_nonce("4663", 6).unwrap_err().contains("below 7"), "a nonce one of ours already used is never signed at");
        // equal or higher: believed, and the floor rises
        f.admit_pin("4663", p1).unwrap();
        f.admit_pin("4663", p1 + b).unwrap();
        assert_eq!(f.admit_nonce("4663", 7), Ok(7));
        assert_eq!(f.admit_nonce("4663", 9), Ok(9));
        assert_eq!((f.pin, f.nonce), (p1 + b, 9));
    }

    fn block(n: u64, hash: &str, extra: bool) -> Value {
        let mut v = serde_json::json!({ "number": format!("0x{n:x}"), "hash": hash, "baseFeePerGas": "0x3669b00",
            "parentHash": "0x00", "transactions": ["0xaa"] });
        if extra { v["l1BlockNumber"] = serde_json::json!("0x18d4b00"); v["sendCount"] = serde_json::json!("0x1"); }
        v
    }

    #[test]
    fn the_pinned_block_must_be_one_block_on_every_provider() {
        // providers that add Nitro fields and ones that do not agree on what the pin needs
        let views: Vec<Value> = [block(9 * 1024, H, true), block(9 * 1024, &H.to_uppercase().replace("0X", "0x"), false)]
            .iter().map(block_view).collect();
        let pin = agree_block(&views, 9 * 1024).unwrap();
        assert_eq!(pin, Pin { number: 9 * 1024, hash: H.to_string(), base_fee: 57_056_000 });
        assert_eq!(at(&pin), serde_json::json!({ "blockHash": H }));
        // a reorg in progress (a different hash at the same number), or a lie: nothing is believed
        let other = "0xdf1a51bb4113aa00000000000000000000000000000000000000000000000002";
        let split: Vec<Value> = [block(9 * 1024, H, false), block(9 * 1024, other, false)].iter().map(block_view).collect();
        assert!(agree_block(&split, 9 * 1024).unwrap_err().contains("disagree"));
        // a provider that answers a different number than asked, or has no such block
        assert!(agree_block(&[block_view(&block(9 * 1024 + 1, H, false))], 9 * 1024).unwrap_err().contains("asked for"));
        assert!(agree_block(&[Value::Null, Value::Null], 9 * 1024).unwrap_err().contains("no provider has"));
        // the transform keeps only the projection, so the consensus body does not carry the transaction list
        let body = serde_json::json!({ "jsonrpc": "2.0", "id": 1, "result": block(5, H, true) }).to_string();
        assert_eq!(result(&block_body(body.as_bytes())), block_view(&block(5, H, true)));
        assert_eq!(block_body(b"nope"), unusable());
    }

    #[test]
    fn a_receipt_counts_only_at_or_below_the_pin_so_every_node_says_the_same() {
        let receipt = |n: u64| format!(r#"{{"jsonrpc":"2.0","id":1,"result":{{"transactionHash":"0xAB","blockHash":"0xCD","blockNumber":"0x{n:x}",
            "from":"0x01","to":"0x02","status":"0x1","gasUsedForL1":"0xfa9","logs":[]}}}}"#).into_bytes();
        let none = br#"{"jsonrpc":"2.0","id":1,"result":null}"#.to_vec();
        let pin = 1_000;
        // below or at the pin: the projection
        assert_eq!(result(&receipt_at_pin_body(&receipt(1_000), pin))["blockNumber"], serde_json::json!("0x3e8"));
        assert_eq!(result(&receipt_at_pin_body(&receipt(999), pin))["from"], serde_json::json!("0x01"));
        assert!(result(&receipt_at_pin_body(&receipt(999), pin)).get("gasUsedForL1").is_none(), "provider extras dropped");
        // above the pin and missing are the SAME body: a node that sees it mined and one that does not yet agree
        assert_eq!(receipt_at_pin_body(&receipt(1_001), pin), receipt_at_pin_body(&none, pin));
        assert_eq!(result(&receipt_at_pin_body(&none, pin)), Value::Null);
        // an error is an error, not "not yet"
        assert_eq!(receipt_at_pin_body(br#"{"error":{"code":-32000}}"#, pin), unusable());
        assert_eq!(u64_of_context(&context_u64(70_166_016)), Some(70_166_016));
        assert_eq!(u64_of_context(&[1, 2]), None);
    }

    #[test]
    fn the_finalized_block_rounds_down_and_an_unusable_reply_is_not_zero() {
        let body = br#"{"jsonrpc":"2.0","id":1,"result":{"number":"0x1234","hash":"0xab"}}"#;
        assert_eq!(finalized_bucket_body(body, 1024), br#"{"result":"0x1000"}"#.to_vec());
        assert_eq!(finalized_bucket_body(br#"{"result":null}"#, 1024), unusable(), "no finalized block: unusable, not zero");
    }
}
