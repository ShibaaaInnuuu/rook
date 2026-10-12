//! The rehearsal's verdict, pure (no network): what each provider answered THROUGH the buyer's transforms, judged against the pinned
//! facts measured on both providers (docs/v7/phase1-nft.md §5.1; review round 7, D7-L-1), and against each other.
use candid::{CandidType, Deserialize};
use serde_json::Value;

/// The canister's Ethereum pair (install argument, evm_chains[1]; scripts/golive-dryrun.sh).
pub const PAIR: [&str; 2] = ["https://rpc.mevblocker.io", "https://gateway.tenderly.co/public/mainnet"];
/// The IPv4-only host (no AAAA record): the IC's outcalls reach it only through NAT64.
pub const IPV4_ONLY_HOST: &str = "gateway.tenderly.co";
/// Ethereum's bucket in the install argument (`EvmChain.bucket`).
pub const BUCKET: u64 = 8;
/// The pinned facts: Milady's `Transfer` logs over this range are exactly 10 (measured 2026-09-28 on both providers), and one of them is
/// the pinned transaction's, mined in PIN_TX_BLOCK, whose hash is PIN_BLOCK_HASH.
pub const MILADY: &str = "0x5af0d9827e0c53e4799bb226655a1de152a425a5";
pub const PIN_FROM: u64 = 26_012_410;
pub const PIN_TO: u64 = 26_012_909;
pub const PIN_LOGS: usize = 10;
pub const PIN_TX: &str = "0x17133e8bbc4d8f1fbdf25e1698229bbe9f732d085069b82dd03cd75cdd55c093";
pub const PIN_TX_BLOCK: u64 = 26_012_500;
pub const PIN_BLOCK_HASH: &str = "0xda4c2b0a15d889c5b6dbcb29d21ffd6b99255cea2cdf426da26e8cbe9843b909";
/// What the buyer's `eth_transfer_logs` makes of the 10 logs: one entry per TRANSACTION that moved a token (the 10 logs are 7
/// transactions; none is a mint). Measured from both providers' answers (tests/fixtures), 2026-09-28. A provider that drops any of
/// the 10 logs' transactions gives fewer.
pub const PIN_MOVES: usize = 7;

/// The response caps the buyer reserves for each read (ethnet.rs / nftnet.rs), so the rehearsal pays and is limited as the buyer is.
pub const HEAD_BYTES: u64 = 8_000;
/// `nftnet::pin_on`'s cap for `eth_getBlockByNumber(pin, false)` on Ethereum (`nftnet::ETH_BLOCK_VIEW_BYTES`): the buyer's own pin read,
/// asked with exactly this cap. It was 16,000 until go-live prep (2026-09-28), which today's blocks exceed (this rehearsal found it);
/// tests.rs holds it to the buyer's constant.
pub const PIN_BLOCK_BYTES: u64 = 128_000;
/// The fixed block's read is for its FACTS (hash, agreement), so it is asked with room (its raw answer is 20,844 bytes).
pub const BLOCK_BYTES: u64 = 64_000;
pub const LOGS_BYTES: u64 = 64_000;
pub const RECEIPT_BYTES: u64 = 32_000;

/// One provider's four answers, each the TRANSFORMED body (what consensus agreed on) or the outcall's failure.
#[derive(CandidType, Deserialize, Clone, Debug, Default)]
pub struct Answers {
    pub url: String,
    pub head: Option<Result<Vec<u8>, String>>,
    /// the buyer's pin read: `eth_getBlockByNumber(pin, false)` via `nft_block_at` at PIN_BLOCK_BYTES
    pub pin_block: Option<Result<Vec<u8>, String>>,
    pub block: Option<Result<Vec<u8>, String>>,
    pub logs: Option<Result<Vec<u8>, String>>,
    pub receipt: Option<Result<Vec<u8>, String>>,
}

#[derive(CandidType, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Check {
    pub name: String,
    pub pass: bool,
    pub detail: String,
}

fn result(b: &[u8]) -> Result<Value, String> {
    let v: Value = serde_json::from_slice(b).map_err(|e| format!("unreadable transformed body: {e}"))?;
    match v.get("result") {
        Some(r) => Ok(r.clone()),
        None => Err(format!("no result: {}", v.get("error").map(|e| e.to_string()).unwrap_or_else(|| "?".into()))),
    }
}

fn got(a: &Option<Result<Vec<u8>, String>>) -> Result<Value, String> {
    match a {
        None => Err("not asked (an earlier read failed)".into()),
        Some(Err(e)) => Err(format!("outcall failed: {e}")),
        Some(Ok(b)) => result(b),
    }
}

fn hexnum(v: &Value) -> Option<u64> {
    u64::from_str_radix(v.as_str()?.strip_prefix("0x")?, 16).ok()
}

fn host(url: &str) -> &str {
    url.split("://").nth(1).unwrap_or(url).split(['/', ':']).next().unwrap_or("")
}

/// Every check, in order. `pass` of the whole is all of them.
pub fn judge(ps: &[Answers]) -> Vec<Check> {
    let mut out = Vec::new();
    let mut push = |name: String, r: Result<String, String>| {
        let (pass, detail) = match r { Ok(d) => (true, d), Err(d) => (false, d) };
        out.push(Check { name, pass, detail });
    };
    for p in ps {
        let h = host(&p.url).to_string();
        push(format!("{h}: head (eth_blockNumber via head_bucket): replicated outcall reached consensus"),
            got(&p.head).and_then(|v| hexnum(&v).map(|n| format!("rounded head {n}")).ok_or_else(|| format!("not a quantity: {v}"))));
        push(format!("{h}: the buyer's pin read (eth_getBlockByNumber(pin, false) via nft_block_at, cap {PIN_BLOCK_BYTES} bytes as nftnet::pin_on)"),
            got(&p.pin_block).and_then(|v| match v.get("hash").and_then(|x| x.as_str()) {
                Some(hash) if hash.len() == 66 => Ok(format!("block {:?} hash {hash}", hexnum(&v["number"]))),
                _ => Err(format!("no block view: {v}")),
            }));
        push(format!("{h}: block {PIN_TX_BLOCK} (eth_getBlockByNumber via nft_block_at): consensus, and the measured hash"),
            got(&p.block).and_then(|v| {
                let hash = v.get("hash").and_then(|x| x.as_str()).unwrap_or("");
                if hash == PIN_BLOCK_HASH && hexnum(&v["number"]) == Some(PIN_TX_BLOCK) { Ok(format!("hash {hash}")) }
                else { Err(format!("block view {v} is not block {PIN_TX_BLOCK} with hash {PIN_BLOCK_HASH}")) }
            }));
        push(format!("{h}: Milady Transfer logs {PIN_FROM}..{PIN_TO} (eth_getLogs via eth_transfer_logs): consensus, and all {PIN_LOGS} logs' {PIN_MOVES} moves"),
            got(&p.logs).and_then(|v| {
                let a = v.as_array().ok_or_else(|| format!("not a list: {v}"))?;
                let has_tx = a.iter().any(|e| e.get(0).and_then(|x| x.as_str()) == Some(PIN_TX));
                if a.len() == PIN_MOVES && has_tx { Ok(format!("{} moves, {PIN_TX} among them", a.len())) }
                else { Err(format!("{} moves (want {PIN_MOVES}; an empty or short answer is a provider that does not keep logs that deep, D7-L-1){}",
                    a.len(), if has_tx { "" } else { "; the pinned transaction is missing" })) }
            }));
        push(format!("{h}: receipt {PIN_TX} (eth_getTransactionReceipt via eth_receipt_logs): consensus, block {PIN_TX_BLOCK}, status 1, a Milady Transfer"),
            got(&p.receipt).and_then(|v| {
                if v.is_null() { return Err("null (no receipt, or one above the pin)".into()) }
                let transfer = format!("0x{}", hex::encode(crate::bodies::TRANSFER));
                let n = v["logs"].as_array().map(|ls| ls.iter().filter(|l| l["address"] == MILADY && l["topics"].get(0).and_then(|x| x.as_str()) == Some(transfer.as_str())).count()).unwrap_or(0);
                if hexnum(&v["blockNumber"]) == Some(PIN_TX_BLOCK) && v["status"] == "0x1" && n >= 1 { Ok(format!("block {PIN_TX_BLOCK}, status 0x1, {n} Milady Transfer log(s)")) }
                else { Err(format!("receipt view is not the measured one: block {:?}, status {}, {n} Milady Transfer log(s)", hexnum(&v["blockNumber"]), v["status"])) }
            }));
    }
    // the pair agrees: the transformed bodies (what the buyer compares, nftnet::one) are byte for byte the same
    let same = |pick: fn(&Answers) -> &Option<Result<Vec<u8>, String>>, what: &str| -> Result<String, String> {
        let bodies: Vec<&Vec<u8>> = ps.iter().filter_map(|p| match pick(p) { Some(Ok(b)) => Some(b), _ => None }).collect();
        if bodies.len() != ps.len() || ps.len() < 2 { return Err(format!("{what}: not every provider answered")) }
        if bodies.iter().all(|b| *b == bodies[0]) { Ok(format!("{what}: {} identical bytes on {} providers", bodies[0].len(), ps.len())) }
        else { Err(format!("{what}: the providers' transformed answers DIFFER (the buyer would believe nothing)")) }
    };
    push("the pair agrees on the pinned block (chains::agree_block_ts)".into(), same(|p| &p.pin_block, "pin block"));
    push("the pair agrees on the block (nftnet::one)".into(), same(|p| &p.block, "block"));
    push("the pair agrees on the logs (nftnet::one)".into(), same(|p| &p.logs, "logs"));
    push("the pair agrees on the receipt (nftnet::one)".into(), same(|p| &p.receipt, "receipt"));
    let heads: Vec<u64> = ps.iter().filter_map(|p| got(&p.head).ok().and_then(|v| hexnum(&v))).collect();
    push("the pair's heads pin (pin::choose_pin, within one bucket)".into(), if heads.len() == ps.len() {
        crate::bodies::choose_pin(&heads, BUCKET).map(|n| format!("pin {n} from rounded heads {heads:?}"))
    } else { Err("not every provider gave a head".into()) });
    // Tenderly's host publishes no AAAA record: it answered at all only if the IC's NAT64 path reaches it
    match ps.iter().find(|p| host(&p.url) == IPV4_ONLY_HOST) {
        Some(p) => push(format!("{IPV4_ONLY_HOST} (IPv4 only) is reachable from this subnet"), match &p.head {
            Some(Ok(_)) => Ok("its outcall came back (whatever the answer): the IC reaches it".into()),
            Some(Err(e)) => Err(format!("its outcall failed: {e}")),
            None => Err("not asked".into()),
        }),
        None => push(format!("{IPV4_ONLY_HOST} (IPv4 only) is reachable from this subnet"), Ok("not in this run's providers (skipped)".into())),
    }
    out
}
