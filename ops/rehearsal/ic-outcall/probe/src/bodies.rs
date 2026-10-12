//! The buyer's transform BODIES, copied verbatim from `autobuyer/canister-rs/src` (only the paths of their helpers changed), because
//! the buyer's crate cannot be linked into another canister: every buyer endpoint, `canister_init` included, would be exported with
//! it. `tests::*_is_the_buyers_own` hold each copy byte for byte to the buyer's function on the real providers' answers
//! (`tests/fixtures/`), so the rehearsal exercises the transforms that will run, not a look-alike.
//!
//! | here | the buyer |
//! | --- | --- |
//! | `head_bucket_body` | `pin::head_bucket_body` (transform `head_bucket`) |
//! | `block_body_ts` | `chains::block_body_ts` (transform `nft_block_at`) |
//! | `transfer_logs_body` | `ethnft::transfer_logs_body` (transform `eth_transfer_logs`, the sale scan's log read) |
//! | `receipt_projection` | `ethnft::receipt_projection` (transform `eth_receipt_logs`, the sale scan's receipt read) |
//! | `json_rpc_norm_body`, `error_class`, `busy_body` | `pin::*` |
use serde_json::Value;

/// `ethnft::TRANSFER`: `Transfer(address,address,uint256)`.
pub const TRANSFER: [u8; 32] = [0xdd, 0xf2, 0x52, 0xad, 0x1b, 0xe2, 0xc8, 0x9b, 0x69, 0xc2, 0xb0, 0x68, 0xfc, 0x37, 0x8d, 0xaa, 0x95, 0x2b,
    0xa7, 0xf1, 0x63, 0xc4, 0xa1, 0x16, 0x28, 0xf5, 0x5a, 0x4d, 0xf5, 0x23, 0xb3, 0xef];

// ---- pin.rs --------------------------------------------------------------------------------------------------------------------

pub fn round_head(head: u64, b: u64) -> u64 {
    head / b * b
}

pub fn context_u64(n: u64) -> Vec<u8> {
    n.to_be_bytes().to_vec()
}

pub fn u64_of_context(c: &[u8]) -> Option<u64> {
    Some(u64::from_be_bytes(c.try_into().ok()?))
}

fn unusable() -> Vec<u8> {
    br#"{"error":"unusable reply"}"#.to_vec()
}

fn result_of(body: &[u8]) -> Option<Value> {
    serde_json::from_slice::<Value>(body).ok()?.get("result").cloned()
}

fn quantity(v: &Value) -> Option<u64> {
    u64::from_str_radix(v.as_str()?.strip_prefix("0x")?, 16).ok()
}

pub fn head_bucket_body(body: &[u8], b: u64) -> Vec<u8> {
    match result_of(body).as_ref().and_then(quantity) {
        Some(h) if b > 0 => serde_json::json!({ "result": format!("0x{:x}", round_head(h, b)) }).to_string().into_bytes(),
        _ => unusable(),
    }
}

pub fn choose_pin(rounded: &[u64], b: u64) -> Result<u64, String> {
    let lo = *rounded.iter().min().ok_or("no providers")?;
    let hi = *rounded.iter().max().ok_or("no providers")?;
    if hi - lo > b {
        return Err(format!("providers' heads are {} blocks apart (more than one bucket of {b}): one is stale or wrong", hi - lo));
    }
    lo.checked_sub(b).ok_or_else(|| "chain too short to pin".into())
}

pub fn block_view(v: &Value) -> Value {
    if v.is_null() {
        return Value::Null;
    }
    let s = |k: &str| v.get(k).and_then(|x| x.as_str()).map(|x| Value::String(x.to_ascii_lowercase())).unwrap_or(Value::Null);
    serde_json::json!({ "number": s("number"), "hash": s("hash"), "baseFeePerGas": s("baseFeePerGas") })
}

pub fn error_class(words: &str) -> Option<&'static str> {
    let l = words.to_ascii_lowercase();
    let any = |ws: &[&str]| ws.iter().any(|w| l.contains(w));
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

pub fn busy_body() -> Vec<u8> {
    serde_json::json!({ "error": { "class": "busy", "code": 429, "reverted": false } }).to_string().into_bytes()
}

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

// ---- chains.rs -------------------------------------------------------------------------------------------------------------------

pub fn block_view_ts(v: &Value) -> Value {
    let mut out = block_view(v);
    if let Some(o) = out.as_object_mut() {
        let ts = v.get("timestamp").and_then(|x| x.as_str()).map(|x| Value::String(x.to_ascii_lowercase()));
        o.insert("timestamp".into(), ts.unwrap_or(Value::Null));
    }
    out
}

pub fn block_body_ts(body: &[u8]) -> Vec<u8> {
    match serde_json::from_slice::<Value>(body).ok().and_then(|v| v.get("result").cloned()) {
        Some(b) => serde_json::json!({ "result": block_view_ts(&b) }).to_string().into_bytes(),
        None => br#"{"error":"unusable reply"}"#.to_vec(),
    }
}

// ---- ethnft.rs -------------------------------------------------------------------------------------------------------------------

pub fn receipt_projection(result: &Value, pin: u64) -> Value {
    let Some(block) = result.get("blockNumber").and_then(|x| x.as_str()).and_then(|x| u64::from_str_radix(x.trim_start_matches("0x"), 16).ok())
    else { return Value::Null };
    if block > pin {
        return Value::Null;
    }
    let logs: Vec<Value> = result.get("logs").and_then(|x| x.as_array()).map(|ls| ls.iter().map(|l| serde_json::json!({
        "address": l.get("address").and_then(|x| x.as_str()).unwrap_or("").to_ascii_lowercase(),
        "topics": l.get("topics").cloned().unwrap_or(Value::Null),
        "data": l.get("data").cloned().unwrap_or(Value::Null),
    })).collect()).unwrap_or_default();
    serde_json::json!({ "blockNumber": format!("0x{block:x}"), "status": result.get("status").cloned().unwrap_or(Value::Null),
        "logs": logs })
}

pub fn transfer_logs_body(body: &[u8]) -> Vec<u8> {
    let Ok(v) = serde_json::from_slice::<Value>(body) else { return json_rpc_norm_body(body) };
    let Some(arr) = v.get("result").and_then(|r| r.as_array()) else { return json_rpc_norm_body(body) };
    let mut out: Vec<(u64, String, String, String)> = Vec::new();
    for l in arr {
        if l.get("removed").and_then(|x| x.as_bool()) == Some(true) {
            continue;
        }
        let Some(t) = l.get("topics").and_then(|x| x.as_array()) else { continue };
        let tx = l.get("transactionHash").and_then(|x| x.as_str()).unwrap_or("").to_ascii_lowercase();
        let block = l.get("blockNumber").and_then(|x| x.as_str()).and_then(|x| u64::from_str_radix(x.trim_start_matches("0x"), 16).ok());
        let topic = |i: usize| t.get(i).and_then(|x| x.as_str()).unwrap_or("").to_ascii_lowercase();
        let (from, to) = (topic(1), topic(2));
        let zero = from.trim_start_matches("0x").chars().all(|c| c == '0');
        if t.len() != 4 || topic(0) != format!("0x{}", hex::encode(TRANSFER)) || zero || from == to || tx.len() != 66 {
            continue;
        }
        let Some(b) = block else { continue };
        match out.iter_mut().find(|(_, x, ..)| *x == tx) {
            Some(e) => {
                if (from.clone(), topic(3)) < (e.3.clone(), e.2.clone()) {
                    e.2 = topic(3);
                    e.3 = from;
                }
            }
            None => out.push((b, tx, topic(3), from)),
        }
    }
    out.sort();
    let res: Vec<Value> = out.into_iter().map(|(b, tx, token, from)| serde_json::json!([tx, format!("0x{b:x}"), token, from])).collect();
    serde_json::json!({ "result": res }).to_string().into_bytes()
}

// ---- ethnet.rs's wrappers (the transform functions themselves; the status and the rate-limit rule as the buyer's) -------------------

/// `ethnet::rate_limited`.
pub fn rate_limited(status: u32, body: &[u8]) -> bool {
    let no_result = serde_json::from_slice::<Value>(body).ok().and_then(|v| v.get("result").cloned()).is_none();
    no_result && (status == 429 || String::from_utf8(json_rpc_norm_body(body)).unwrap_or_default().contains(r#""class":"busy""#))
}

/// `ethnet::eth_transfer_logs`'s body.
pub fn eth_transfer_logs(status: u32, body: &[u8]) -> Vec<u8> {
    if rate_limited(status, body) { busy_body() } else { transfer_logs_body(body) }
}

/// `ethnet::eth_receipt_logs`'s body.
pub fn eth_receipt_logs(status: u32, body: &[u8], pin: u64) -> Vec<u8> {
    match serde_json::from_slice::<Value>(body).ok().and_then(|v| v.get("result").cloned()) {
        Some(r) => serde_json::json!({ "result": receipt_projection(&r, pin) }).to_string().into_bytes(),
        None if rate_limited(status, body) => busy_body(),
        None => br#"{"error":"unusable reply"}"#.to_vec(),
    }
}
