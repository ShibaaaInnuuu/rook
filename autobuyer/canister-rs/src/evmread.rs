//! Reading chain 4663 through several providers that must agree (docs/v2/pinned-reads.md), written one provider at a time
//! ([`Rpc`]) so that the same logic runs on the IC (`canister.rs`, replicated outcalls and transforms) and in the tests against
//! fakes that carry each real provider's limits (`tests/fakerpc`). The limits are measured, not assumed
//! (docs/audit/2026-09-26/fixes-D-canister.md §0, 2026-09-26):
//!
//! - `rpc.mainnet.chain.robinhood.com` serves state about **6,200** blocks behind its head, and `eth_getLogs` over any span;
//! - `rpc.ordofi.network` serves state at any depth, and refuses `eth_getLogs` over more than **10,000** blocks;
//! - both report `finalized` **8,096–12,023** blocks behind the head, moving in steps of ≈ 3,700–3,900 blocks every ≈ 6 minutes,
//!   and for ≈ 20 s after a step one provider can be a step ahead of the other.
//!
//! So no read may name the finalized block's STATE (Robinhood no longer has it), and no log query may span more than
//! [`MAX_LOG_SPAN_BLOCKS`]. Finality is read as a block NUMBER only, bounded ([`choose_finality`]), and is used to bound log
//! queries (`ticketlog.rs`), which need no state.

use serde_json::Value;

use crate::pin::{self, Pin, BUCKET};

/// The most blocks one `eth_getLogs` asks for, inclusive: ordofi's measured 10,000 less a fifth.
pub const MAX_LOG_SPAN_BLOCKS: u64 = 8_000;
/// A provider whose finalized block is further than this behind its own head is refused: 4× the largest lag measured (12,023).
pub const MAX_FINALITY_LAG: u64 = 50_000;
/// Providers' finalized blocks may differ by at most this much: two finality steps (≈ 3,900 each) rounded up.
pub const MAX_FINALIZED_SPREAD: u64 = 8_192;

/// One log, projected to what every provider agrees on, with its block number.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct LogView {
    /// `0x…`, lower-case.
    pub address: String,
    /// `0x…`, lower-case.
    pub topics: Vec<String>,
    /// `0x…`, lower-case.
    pub data: String,
    pub block: u64,
    pub removed: bool,
}

/// One provider. On the IC each method is one replicated outcall through a transform; in the tests, a fake with that provider's
/// measured limits. An `Err` names what the provider said.
#[allow(async_fn_in_trait)]
pub trait Rpc {
    fn name(&self) -> String;
    /// `eth_blockNumber`, rounded down to a multiple of `bucket`.
    async fn head(&mut self, bucket: u64) -> Result<u64, String>;
    /// `eth_getBlockByNumber("finalized")`'s number, rounded down to a multiple of `bucket`.
    async fn finalized(&mut self, bucket: u64) -> Result<u64, String>;
    /// `eth_getBlockByNumber(n)`, projected by `pin::block_view` (`null` when the provider has no such block).
    async fn block(&mut self, n: u64) -> Result<Value, String>;
    /// `eth_call` at the pin, named by hash (EIP-1898).
    async fn call(&mut self, to: &[u8; 20], data: &[u8], at: &Pin) -> Result<Vec<u8>, String>;
    /// `eth_getLogs` for `address` and `topics` over `[from, to]`, projected. `max_bytes` is the response cap the outcall asks
    /// for (headers included, paid for whether used or not); an answer over it fails with an error that [`too_big`] recognises.
    async fn logs(&mut self, address: &[u8; 20], topics: &Value, from: u64, to: u64, max_bytes: u64) -> Result<Vec<LogView>, String>;
}

fn named<R: Rpc>(p: &R, e: String) -> String {
    format!("{}: {e}", p.name())
}

/// The fast pin (docs/v2/pinned-reads.md): every provider's head rounded to [`BUCKET`], one bucket below the lowest, and that
/// block agreed by hash on every provider. 1,024–2,048 blocks (plus lag) behind the head: inside Robinhood's ≈ 6,200.
pub async fn fast_pin<R: Rpc>(ps: &mut [R]) -> Result<Pin, String> {
    if ps.is_empty() {
        return Err("no providers".into());
    }
    let mut rounded = Vec::new();
    for p in ps.iter_mut() {
        let h = p.head(BUCKET).await.map_err(|e| named(p, e))?;
        rounded.push(h);
    }
    let n = pin::choose_pin(&rounded, BUCKET)?;
    let mut views = Vec::new();
    for p in ps.iter_mut() {
        let v = p.block(n).await.map_err(|e| named(p, e))?;
        views.push(v);
    }
    pin::agree_block(&views, n)
}

/// What one provider said about its head and its finality, for [`choose_finality`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finality {
    pub provider: String,
    pub head: u64,
    pub finalized: u64,
}

/// The finalized block every provider has finalized, bounded (audit D, L-7): each provider's finality at or below its head and
/// at most [`MAX_FINALITY_LAG`] behind it, and the providers within [`MAX_FINALIZED_SPREAD`] of each other. The lowest is
/// used. A provider that reports finality far too deep (which would stall every read that waits for finality) or not at all is
/// refused by name, rather than silently believed.
pub fn choose_finality(obs: &[Finality]) -> Result<u64, String> {
    if obs.is_empty() {
        return Err("no providers".into());
    }
    for o in obs {
        if o.finalized == 0 {
            return Err(format!("{} reports no finalized block", o.provider));
        }
        if o.finalized > o.head {
            return Err(format!("{} reports block {} finalized above its head {}: refused", o.provider, o.finalized, o.head));
        }
        if o.head - o.finalized > MAX_FINALITY_LAG {
            return Err(format!("{} reports finality {} blocks behind its head (more than {MAX_FINALITY_LAG}): refused, not waited on",
                o.provider, o.head - o.finalized));
        }
    }
    let lo = obs.iter().map(|o| o.finalized).min().expect("not empty");
    let hi = obs.iter().map(|o| o.finalized).max().expect("not empty");
    if hi - lo > MAX_FINALIZED_SPREAD {
        return Err(format!("the providers' finalized blocks are {} apart (more than {MAX_FINALIZED_SPREAD}): one is stale or wrong",
            hi - lo));
    }
    Ok(lo)
}

/// Every provider's finality, and the one chosen (see [`choose_finality`]).
pub async fn finality<R: Rpc>(ps: &mut [R]) -> Result<u64, String> {
    let mut obs = Vec::new();
    for p in ps.iter_mut() {
        let head = p.head(BUCKET).await.map_err(|e| named(p, e))?;
        let finalized = p.finalized(BUCKET).await.map_err(|e| named(p, e))?;
        obs.push(Finality { provider: p.name(), head, finalized });
    }
    choose_finality(&obs)
}

/// `eth_call` at the pin, every provider agreeing.
pub async fn call<R: Rpc>(ps: &mut [R], to: &[u8; 20], data: &[u8], at: &Pin) -> Result<Vec<u8>, String> {
    let mut answers = Vec::new();
    for p in ps.iter_mut() {
        let a = p.call(to, data, at).await.map_err(|e| named(p, e))?;
        answers.push(a);
    }
    let first = answers.first().cloned().ok_or("no providers")?;
    if answers.iter().any(|a| *a != first) {
        return Err("providers disagree on eth_call at the pin; nothing is believed".into());
    }
    Ok(first)
}

/// Why [`logs_sized`] has no answer (audit R3, R3-C2): the size fallback may make the scanner work harder, but only the providers'
/// agreement may make it skip anything.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LogsError {
    /// **Every** provider's answer was over the response cap (the IC's own reject, from each).
    TooBig,
    /// Some providers' answers were over the cap and the others answered (named). Not proof of size: a reason to try a larger cap
    /// or a smaller query, never to skip.
    Split(String),
    /// Anything else: a provider down or refusing, or the providers disagreeing on the logs.
    Failed(String),
}

impl std::fmt::Display for LogsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LogsError::TooBig => write!(f, "{SIZE_REJECT}every provider's answer is over the response cap"),
            LogsError::Split(s) => write!(f, "{s}"),
            LogsError::Failed(e) => write!(f, "{e}"),
        }
    }
}

/// `eth_getLogs` over `[from, to]`, every provider agreeing on the projected list. Refuses, without asking, a span wider than
/// [`MAX_LOG_SPAN_BLOCKS`]: a provider would refuse it, and the caller pages instead. Logs outside the range, from another
/// address, or marked removed are dropped (a provider's answer is checked, not trusted).
pub async fn logs<R: Rpc>(ps: &mut [R], address: &[u8; 20], topics: &Value, from: u64, to: u64, max_bytes: u64)
    -> Result<Vec<LogView>, String> {
    logs_sized(ps, address, topics, from, to, max_bytes).await.map_err(|e| e.to_string())
}

/// [`logs`], with the size verdict compared across providers (audit R3, R3-C2): every provider is asked even when one's answer
/// was over the cap, and the answer is [`LogsError::TooBig`] only if **every** provider's was. A failure that is not about size
/// stops at once, as before.
pub async fn logs_sized<R: Rpc>(ps: &mut [R], address: &[u8; 20], topics: &Value, from: u64, to: u64, max_bytes: u64)
    -> Result<Vec<LogView>, LogsError> {
    if to < from {
        return Ok(vec![]);
    }
    if to - from + 1 > MAX_LOG_SPAN_BLOCKS {
        return Err(LogsError::Failed(format!("refused to ask for logs over {} blocks: at most {MAX_LOG_SPAN_BLOCKS} (ordofi refuses \
            more than 10,000)", to - from + 1)));
    }
    if ps.is_empty() {
        return Err(LogsError::Failed("no providers".into()));
    }
    let (mut answers, mut fit, mut big) = (Vec::new(), Vec::new(), Vec::new());
    for p in ps.iter_mut() {
        match p.logs(address, topics, from, to, max_bytes).await {
            Ok(a) => {
                fit.push(p.name());
                answers.push(a);
            }
            Err(e) if too_big(&e) => big.push(p.name()),
            Err(e) => return Err(LogsError::Failed(named(p, e))),
        }
    }
    if !big.is_empty() {
        return Err(if fit.is_empty() {
            LogsError::TooBig
        } else {
            LogsError::Split(format!("blocks {from}..={to} at a {max_bytes}-byte cap: {} answered, {} said too big",
                fit.join(", "), big.join(", ")))
        });
    }
    let first = answers.first().cloned().ok_or(LogsError::Failed("no providers".into()))?;
    if answers.iter().any(|a| *a != first) {
        return Err(LogsError::Failed(format!("providers disagree on the logs of blocks {from}..={to}; nothing is believed")));
    }
    let want = format!("0x{}", hex::encode(address));
    Ok(first.into_iter().filter(|l| !l.removed && l.address == want && (from..=to).contains(&l.block)).collect())
}

/// The IC's largest response for one HTTPS outcall, in bytes (headers included).
pub const MAX_RESPONSE_BYTES: u64 = 2_000_000;

/// The prefix of an error that is the IC's own reject of an answer over the response cap, and nothing else (audit R3, R3-C2).
/// Only the IC path makes it (`canister.rs`, from a `CallRejected` whose message is the IC's wording, see [`ic_size_reject`]); a
/// provider's own words reach the canister only inside `no result: …` or `unreadable: …`, never at the start.
pub const SIZE_REJECT: &str = "IC reject, answer over the response cap: ";

/// Whether an outcall failed because its answer was over the response cap it asked for (audit R2, N-1): the size is the only
/// thing a bigger cap or a smaller query can fix, so it is told apart from every other failure. Only an error that **starts**
/// with [`SIZE_REJECT`] counts (audit R3, R3-C2: substrings of a provider's JSON-RPC message used to count, which let one
/// provider send the scanner down the dense path and make it give up a chosen ticket).
pub fn too_big(e: &str) -> bool {
    e.starts_with(SIZE_REJECT)
}

/// Whether the IC's reject message of an HTTPS outcall is its refusal of an answer over the cap: "Http body exceeds size limit of
/// N bytes" or "Header size exceeds specified response size limit N". The message must **start** with the IC's wording.
pub fn ic_size_reject(message: &str) -> bool {
    let m = message.trim_start().to_ascii_lowercase();
    m.starts_with("http body exceeds size limit") || m.starts_with("header size exceeds specified response size limit")
}

/// The transform's half for `eth_getLogs` on the IC: each log projected to [`LogView`], as JSON (`{"result":[…]}`), so the
/// consensus body is provider-neutral. An error, or a log without a readable block number, is unusable.
pub fn logs_body(body: &[u8]) -> Vec<u8> {
    let unusable = || br#"{"error":"unusable reply"}"#.to_vec();
    let v = serde_json::from_slice::<Value>(body).ok();
    // a provider's own refusal is kept (every node sees the same one), so the log says why: ordofi's -32005 past 10,000 blocks
    if let Some(e) = v.as_ref().and_then(|v| v.get("error")) {
        let code = e.get("code").and_then(|c| c.as_i64()).unwrap_or(0);
        let msg: String = e.get("message").and_then(|m| m.as_str()).unwrap_or("").chars().take(200).collect();
        return serde_json::json!({ "error": { "code": code, "message": msg } }).to_string().into_bytes();
    }
    let Some(Value::Array(logs)) = v.and_then(|v| v.get("result").cloned()) else {
        return unusable();
    };
    let lc = |v: Option<&Value>| v.and_then(|x| x.as_str()).map(str::to_ascii_lowercase).unwrap_or_default();
    let mut out = Vec::new();
    for l in &logs {
        let Some(block) = l.get("blockNumber").and_then(|x| x.as_str()).and_then(|x| x.strip_prefix("0x"))
            .and_then(|x| u64::from_str_radix(x, 16).ok()) else { return unusable() };
        out.push(LogView {
            address: lc(l.get("address")),
            topics: l.get("topics").and_then(|t| t.as_array()).map(|t| t.iter().map(|x| lc(Some(x))).collect()).unwrap_or_default(),
            data: lc(l.get("data")),
            block,
            removed: l.get("removed").and_then(|r| r.as_bool()).unwrap_or(false),
        });
    }
    serde_json::json!({ "result": out }).to_string().into_bytes()
}

/// The inverse, on the canister's side of consensus.
pub fn logs_of(result: &Value) -> Result<Vec<LogView>, String> {
    serde_json::from_value(result.clone()).map_err(|e| format!("unreadable logs: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(p: &str, head: u64, fin: u64) -> Finality {
        Finality { provider: p.into(), head, finalized: fin }
    }

    #[test]
    fn finality_is_the_lowest_and_bounded_as_measured() {
        // measured 2026-09-26: 8,096..12,023 behind, one provider a step (3,939) ahead for ~20 s after a step
        assert_eq!(choose_finality(&[f("rh", 72_751_714, 72_739_691), f("of", 72_751_726, 72_743_630)]), Ok(72_739_691));
        assert_eq!(choose_finality(&[f("rh", 100_000, 90_000)]), Ok(90_000));
        let deep = choose_finality(&[f("rh", 72_751_714, 72_739_691), f("liar", 72_751_726, 1_000)]).unwrap_err();
        assert!(deep.contains("liar") && deep.contains("refused"), "{deep}");
        let wide = choose_finality(&[f("a", 200_000, 190_000), f("b", 200_000, 170_000)]).unwrap_err();
        assert!(wide.contains("apart"), "{wide}");
        assert!(choose_finality(&[f("a", 10, 20)]).unwrap_err().contains("above its head"));
        assert!(choose_finality(&[f("a", 10, 0)]).unwrap_err().contains("no finalized"));
        assert!(choose_finality(&[]).is_err());
    }

    #[test]
    fn the_log_transform_keeps_the_block_and_refuses_a_log_without_one() {
        let body = br#"{"result":[{"address":"0xAB","topics":["0xT0","0xT1"],"data":"0xDD","blockNumber":"0x5","logIndex":"0x1","removed":false}]}"#;
        let v: Value = serde_json::from_slice(&logs_body(body)).unwrap();
        let got = logs_of(&v["result"]).unwrap();
        assert_eq!(got, vec![LogView { address: "0xab".into(), topics: vec!["0xt0".into(), "0xt1".into()], data: "0xdd".into(), block: 5,
            removed: false }]);
        let refused = String::from_utf8(logs_body(br#"{"jsonrpc":"2.0","id":1,"error":{"code":-32005,"message":"the network is busy"}}"#)).unwrap();
        assert!(refused.contains("-32005") && refused.contains("busy"), "{refused}");
        assert!(logs_body(br#"{"result":[{"address":"0xab","topics":[],"data":"0x"}]}"#).starts_with(br#"{"error""#));
    }

    /// Audit R3, R3-C2: only the IC's own reject is a size error. A provider's JSON-RPC message that merely mentions a size limit
    /// (the R3 PoC's lie) is an ordinary failure.
    #[test]
    fn only_the_ics_own_reject_is_a_size_error() {
        assert!(ic_size_reject("Http body exceeds size limit of 48000 bytes."));
        assert!(ic_size_reject("Header size exceeds specified response size limit 48000"));
        assert!(!ic_size_reject("Connecting to rpc.example failed: Http body exceeds size limit of 1 bytes"));
        assert!(too_big(&format!("{SIZE_REJECT}Http body exceeds size limit of 48000 bytes.")));
        for lie in [
            "no result: {\"code\":-32000,\"message\":\"response size limit of 48000 bytes exceeded\"}",
            "no result: {\"code\":-32000,\"message\":\"IC reject, answer over the response cap: \"}",
            "rpc.ordofi.network: IC reject, answer over the response cap: x",
            "Http body exceeds size limit of 48000 bytes",
        ] {
            assert!(!too_big(lie), "{lie}");
        }
        assert!(LogsError::TooBig.to_string().starts_with(SIZE_REJECT));
    }
}
