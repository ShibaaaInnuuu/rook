//! The go-live rehearsal's probe canister (docs/v7/phase1-nft.md §5.1, "the canister's Ethereum pair from a real IC outcall"; review
//! round 7, the hard precondition). Installed in a THROWAWAY canister on the launch subnet, it makes the buyer's Ethereum reads the way
//! the buyer makes them (replicated HTTPS outcalls, the same response caps, the same transform names and bodies: `bodies.rs`) on each
//! provider of the pair, and says whether each came back through consensus, agrees with the other provider and holds the pinned facts
//! (`check.rs`). It holds no key, signs nothing, sends no transaction and keeps no state: `probe` is a controller-only update because
//! each call spends cycles on outcalls.
pub mod bodies;
pub mod check;

use candid::{CandidType, Deserialize};
use ic_cdk::management_canister::{http_request, HttpHeader, HttpMethod, HttpRequestArgs, HttpRequestResult, TransformArgs, TransformContext, TransformFunc};

use check::{Answers, Check};

/// What `probe` returns: the verdict, every check, and each provider's transformed answers (as text) for the record.
#[derive(CandidType, Deserialize, Clone, Debug)]
pub struct Report {
    pub pass: bool,
    pub pin: Option<u64>,
    pub checks: Vec<Check>,
    pub providers: Vec<Trace>,
    /// the canister's cycles before and after the call, and so what the outcalls cost on this subnet
    pub cycles_before: u128,
    pub cycles_after: u128,
}

#[derive(CandidType, Deserialize, Clone, Debug)]
pub struct Trace {
    pub url: String,
    pub head: String,
    pub pin_block: String,
    pub block: String,
    pub logs: String,
    pub receipt: String,
}

fn text(a: &Option<Result<Vec<u8>, String>>) -> String {
    match a {
        None => "(not asked)".into(),
        Some(Err(e)) => format!("OUTCALL FAILED: {e}"),
        Some(Ok(b)) => {
            let s = String::from_utf8_lossy(b);
            if s.len() > 600 { format!("{}… ({} bytes)", &s[..600], b.len()) } else { s.into_owned() }
        }
    }
}

// ---- the transforms: the buyer's names, the buyer's bodies ----------------------------------------------------------------------

/// `canister::head_bucket`.
#[ic_cdk::query]
fn head_bucket(args: TransformArgs) -> HttpRequestResult {
    let b = bodies::u64_of_context(&args.context).unwrap_or(0);
    HttpRequestResult { status: args.response.status, body: bodies::head_bucket_body(&args.response.body, b), headers: vec![] }
}

/// `nftnet::nft_block_at`.
#[ic_cdk::query]
fn nft_block_at(args: TransformArgs) -> HttpRequestResult {
    HttpRequestResult { status: args.response.status, body: bodies::block_body_ts(&args.response.body), headers: vec![] }
}

fn status_u32(s: &candid::Nat) -> u32 {
    u32::try_from(s.0.clone()).unwrap_or(0)
}

/// `ethnet::eth_transfer_logs`.
#[ic_cdk::query]
fn eth_transfer_logs(args: TransformArgs) -> HttpRequestResult {
    let body = bodies::eth_transfer_logs(status_u32(&args.response.status), &args.response.body);
    HttpRequestResult { status: candid::Nat::from(200u32), body, headers: vec![] }
}

/// `ethnet::eth_receipt_logs`.
#[ic_cdk::query]
fn eth_receipt_logs(args: TransformArgs) -> HttpRequestResult {
    let pin = bodies::u64_of_context(&args.context).unwrap_or(0);
    let body = bodies::eth_receipt_logs(status_u32(&args.response.status), &args.response.body, pin);
    HttpRequestResult { status: args.response.status, body, headers: vec![] }
}

// ---- the reads ---------------------------------------------------------------------------------------------------------------

/// `canister::rpc_once_with`, returning the transformed body itself (what the replicas agreed on) rather than its `result`.
async fn outcall(url: &str, method: &str, params: serde_json::Value, transform: &str, context: Vec<u8>, max: u64) -> Result<Vec<u8>, String> {
    let body = serde_json::json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params }).to_string();
    let req = HttpRequestArgs {
        url: url.to_string(),
        method: HttpMethod::POST,
        body: Some(body.into_bytes()),
        max_response_bytes: Some(max),
        transform: Some(TransformContext {
            function: TransformFunc(candid::Func { principal: ic_cdk::api::canister_self(), method: transform.into() }),
            context,
        }),
        headers: vec![HttpHeader { name: "content-type".into(), value: "application/json".into() }],
        is_replicated: Some(true),
    };
    http_request(&req).await.map(|r| r.body).map_err(|e| format!("{e:?}"))
}

fn head_of(a: &Option<Result<Vec<u8>, String>>) -> Option<u64> {
    let v: serde_json::Value = serde_json::from_slice(a.as_ref()?.as_ref().ok()?).ok()?;
    u64::from_str_radix(v.get("result")?.as_str()?.strip_prefix("0x")?, 16).ok()
}

/// The rehearsal: every read on every provider (default: the canister's pair), then the verdict. Controller only.
#[ic_cdk::update]
async fn probe(urls: Option<Vec<String>>) -> Report {
    if !ic_cdk::api::is_controller(&ic_cdk::api::msg_caller()) {
        ic_cdk::trap("the probe spends cycles on outcalls: controllers only");
    }
    let cycles_before = ic_cdk::api::canister_cycle_balance();
    let urls = urls.filter(|u| !u.is_empty()).unwrap_or_else(|| check::PAIR.iter().map(|s| s.to_string()).collect());
    let mut ps: Vec<Answers> = urls.iter().map(|u| Answers { url: u.clone(), ..Default::default() }).collect();
    for p in ps.iter_mut() {
        p.head = Some(outcall(&p.url, "eth_blockNumber", serde_json::json!([]), "head_bucket", bodies::context_u64(check::BUCKET), check::HEAD_BYTES).await);
    }
    let heads: Vec<u64> = ps.iter().filter_map(|p| head_of(&p.head)).collect();
    let pin = (heads.len() == ps.len()).then(|| bodies::choose_pin(&heads, check::BUCKET).ok()).flatten();
    // a receipt is counted at or below the pin, as the buyer's scan reads it
    let receipt_pin = pin.or_else(|| heads.iter().min().copied()).unwrap_or(u64::MAX);
    for p in ps.iter_mut() {
        if let Some(n) = pin {
            p.pin_block = Some(outcall(&p.url, "eth_getBlockByNumber", serde_json::json!([format!("0x{n:x}"), false]), "nft_block_at", vec![],
                check::PIN_BLOCK_BYTES).await);
        }
        p.block = Some(outcall(&p.url, "eth_getBlockByNumber", serde_json::json!([format!("0x{:x}", check::PIN_TX_BLOCK), false]), "nft_block_at",
            vec![], check::BLOCK_BYTES).await);
        let q = serde_json::json!([{ "address": check::MILADY, "topics": [format!("0x{}", hex::encode(bodies::TRANSFER))],
            "fromBlock": format!("0x{:x}", check::PIN_FROM), "toBlock": format!("0x{:x}", check::PIN_TO) }]);
        p.logs = Some(outcall(&p.url, "eth_getLogs", q, "eth_transfer_logs", vec![], check::LOGS_BYTES).await);
        p.receipt = Some(outcall(&p.url, "eth_getTransactionReceipt", serde_json::json!([check::PIN_TX]), "eth_receipt_logs",
            bodies::context_u64(receipt_pin), check::RECEIPT_BYTES).await);
    }
    let checks = check::judge(&ps);
    Report {
        pass: checks.iter().all(|c| c.pass),
        pin,
        checks,
        providers: ps.iter().map(|p| Trace { url: p.url.clone(), head: text(&p.head), pin_block: text(&p.pin_block), block: text(&p.block), logs: text(&p.logs), receipt: text(&p.receipt) }).collect(),
        cycles_before,
        cycles_after: ic_cdk::api::canister_cycle_balance(),
    }
}

ic_cdk::export_candid!();

#[cfg(test)]
mod tests;
