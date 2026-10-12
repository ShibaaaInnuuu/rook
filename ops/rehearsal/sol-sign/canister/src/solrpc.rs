//! The outcall layer (the shape of §4's `solrpc.rs`): JSON-RPC over replicated HTTPS outcalls, one transform (`sol_tf`) whose context
//! byte names what is kept, and agreement across ≥ 2 providers: every provider answers and the transformed answers are equal, or the
//! read is REFUSED (`Disagree`) and retried by the caller next tick, never guessed.
//!
//! Why each transform keeps what it keeps (the replicas of one outcall must produce identical bytes, and so must the two providers):
//! - `context.slot`, `id`, `jsonrpc`, `confirmations`, simulation logs and units: dropped (they move between requests);
//! - slots and block heights: rounded DOWN to a bucket (the `head_bucket` trick of the Ethereum pin);
//! - send answers: a class (`accepted`, `already_processed`, `blockhash_not_found`, ...), never the provider's prose.
use candid::{CandidType, Deserialize};
use ic_cdk::management_canister::{http_request, HttpHeader, HttpMethod, HttpRequestArgs, HttpRequestResult, TransformArgs, TransformContext, TransformFunc};
use serde_json::{json, Value};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Kind {
    SlotBucket = 0,
    Account = 1,
    SigStatus = 2,
    Send = 3,
    Block = 4,
    Number = 5,
    HeightBucket = 6,
}

impl Kind {
    fn of(b: u8) -> Option<Kind> {
        Some(match b {
            0 => Kind::SlotBucket,
            1 => Kind::Account,
            2 => Kind::SigStatus,
            3 => Kind::Send,
            4 => Kind::Block,
            5 => Kind::Number,
            6 => Kind::HeightBucket,
            _ => return None,
        })
    }
}

pub fn context(kind: Kind, bucket: u64) -> Vec<u8> {
    let mut c = vec![kind as u8];
    c.extend(bucket.to_le_bytes());
    c
}

/// The transform body (pure; tested natively on real devnet answers).
pub fn normalise(status: u32, body: &[u8], ctx: &[u8]) -> Value {
    if status != 200 {
        return json!({ "http": status });
    }
    let Some(kind) = ctx.first().and_then(|b| Kind::of(*b)) else { return json!({ "bad_context": true }) };
    let bucket = ctx.get(1..9).map(|b| u64::from_le_bytes(b.try_into().unwrap())).unwrap_or(1).max(1);
    let Ok(v) = serde_json::from_slice::<Value>(body) else { return json!({ "unparsable": true }) };
    if let Some(e) = v.get("error") {
        return error_class(kind, e);
    }
    let r = &v["result"];
    let val = if r.get("context").is_some() { &r["value"] } else { r };
    match kind {
        Kind::SlotBucket | Kind::HeightBucket => match val.as_u64() {
            Some(n) => json!({ "n": n - n % bucket }),
            None => json!({ "unexpected": true }),
        },
        Kind::Number => match val.as_u64() {
            Some(n) => json!({ "n": n }),
            None => json!({ "unexpected": true }),
        },
        Kind::Account => {
            if val.is_null() {
                json!({ "account": null })
            } else {
                json!({ "account": { "lamports": val["lamports"], "owner": val["owner"], "data": val["data"][0] } })
            }
        }
        Kind::SigStatus => {
            let s = &val[0];
            if s.is_null() {
                json!({ "status": null })
            } else {
                json!({ "status": s["confirmationStatus"], "err": s["err"] })
            }
        }
        Kind::Send => match r.as_str() {
            Some(_) => json!({ "class": "accepted" }),
            None => json!({ "class": "unexpected" }),
        },
        Kind::Block => json!({ "blockhash": val["blockhash"], "blockHeight": val["blockHeight"], "parentSlot": val["parentSlot"] }),
    }
}

fn error_class(kind: Kind, e: &Value) -> Value {
    let code = e["code"].as_i64().unwrap_or(0);
    let msg = e["message"].as_str().unwrap_or("");
    let err = &e["data"]["err"];
    match kind {
        Kind::Send => {
            let errs = err.to_string();
            let class = if errs.contains("AlreadyProcessed") || msg.contains("already been processed") {
                "already_processed"
            } else if errs.contains("BlockhashNotFound") || msg.contains("Blockhash not found") {
                "blockhash_not_found"
            } else if errs.contains("InsufficientFunds") || msg.contains("insufficient") {
                "insufficient_funds"
            } else if errs.contains("AccountNotFound") || msg.contains("no record of a prior credit") {
                // the RPC verifies signatures BEFORE it simulates: reaching this class proves the signature valid
                "account_not_found"
            } else if code == -32003 || msg.contains("signature verification") {
                "signature_failure"
            } else {
                "error"
            };
            // the simulation's `err` is stable across providers; its logs and units are not
            json!({ "class": class, "code": code, "err": if err.is_null() { Value::Null } else { err.clone() } })
        }
        // -32007 slot skipped, -32009 missing in long-term storage, -32004 block not available
        Kind::Block if matches!(code, -32007 | -32009 | -32004) => json!({ "skipped": code }),
        _ => json!({ "error": code }),
    }
}

/// The one transform every outcall names.
#[ic_cdk::query]
fn sol_tf(args: TransformArgs) -> HttpRequestResult {
    let status = u32::try_from(args.response.status.0.clone()).unwrap_or(0);
    let body = normalise(status, &args.response.body, &args.context).to_string().into_bytes();
    HttpRequestResult { status: candid::Nat::from(200u32), body, headers: vec![] }
}

pub struct Call<'a> {
    pub method: &'a str,
    pub params: Value,
    pub kind: Kind,
    pub bucket: u64,
    pub max_bytes: u64,
    pub replicated: bool,
}

#[derive(CandidType, Deserialize, Clone, Debug)]
pub struct CallSample {
    pub method: String,
    pub providers: u32,
    pub latency_ms: u64,
    pub cycles: u128,
    pub outcome: String,
}

thread_local! {
    pub static CALLS: std::cell::RefCell<Vec<CallSample>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// One outcall to one provider: the transformed answer, or the IC's error.
pub async fn once(url: &str, c: &Call<'_>) -> Result<Value, String> {
    let body = json!({ "jsonrpc": "2.0", "id": 1, "method": c.method, "params": c.params }).to_string();
    let req = HttpRequestArgs {
        url: url.to_string(),
        method: HttpMethod::POST,
        body: Some(body.into_bytes()),
        max_response_bytes: Some(c.max_bytes),
        transform: Some(TransformContext {
            function: TransformFunc(candid::Func { principal: ic_cdk::api::canister_self(), method: "sol_tf".into() }),
            context: context(c.kind, c.bucket),
        }),
        headers: vec![HttpHeader { name: "content-type".into(), value: "application/json".into() }],
        is_replicated: Some(c.replicated),
    };
    let r = http_request(&req).await.map_err(|e| format!("{e:?}"))?;
    serde_json::from_slice(&r.body).map_err(|e| format!("transform output: {e}"))
}

/// Every provider's answer, concurrently, with the call's latency and cycles recorded.
pub async fn each(urls: &[String], c: &Call<'_>) -> Vec<Result<Value, String>> {
    let t0 = ic_cdk::api::time();
    let b0 = ic_cdk::api::canister_cycle_balance();
    let out = futures::future::join_all(urls.iter().map(|u| once(u, c))).await;
    let sample = CallSample {
        method: c.method.into(),
        providers: urls.len() as u32,
        latency_ms: (ic_cdk::api::time() - t0) / 1_000_000,
        cycles: b0.saturating_sub(ic_cdk::api::canister_cycle_balance()),
        outcome: out.iter().map(|r| match r { Ok(v) => v.to_string(), Err(e) => format!("ERR {e}") }).collect::<Vec<_>>().join(" | ").chars().take(300).collect(),
    };
    CALLS.with(|s| s.borrow_mut().push(sample));
    out
}

#[derive(Debug, Clone)]
pub enum Refused {
    /// an outcall failed (consensus, network, cap): retry next tick
    Failed(String),
    /// the providers answered differently: refused, retried next tick, never guessed
    Disagree(Vec<String>),
}

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Refused::Failed(e) => write!(f, "refused (an outcall failed): {e}"),
            Refused::Disagree(v) => write!(f, "refused (providers disagree): {}", v.join(" ≠ ")),
        }
    }
}

/// `rpc_agreed`: all providers answer, and identically after the transform.
pub async fn agreed(urls: &[String], c: &Call<'_>) -> Result<Value, Refused> {
    let answers = each(urls, c).await;
    agree(answers)
}

pub fn agree(answers: Vec<Result<Value, String>>) -> Result<Value, Refused> {
    let mut vals = Vec::new();
    for a in answers {
        vals.push(a.map_err(Refused::Failed)?);
    }
    match vals.first() {
        None => Err(Refused::Failed("no providers".into())),
        Some(v0) if vals.iter().all(|v| v == v0) => Ok(v0.clone()),
        _ => Err(Refused::Disagree(vals.iter().map(|v| v.to_string()).collect())),
    }
}
