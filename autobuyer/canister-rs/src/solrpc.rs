//! Reading and writing Solana through ≥ 2 providers that must agree (docs/v9/solana-migration.md §2.4 "Reads", §4), the shape of
//! `canister.rs`'s `rpc_agreed` and of spike Q3's `solrpc.rs`: JSON-RPC over HTTPS outcalls, one transform ([`normalise`], exported
//! as `sol_tf`) whose context byte names what is kept, and agreement: every provider answers and the transformed answers are
//! equal, or the read is REFUSED ([`Refused::Disagree`]) and retried by the caller next tick, never guessed.
//!
//! Written one provider at a time ([`SolRpc`]) so the same logic runs on the IC ([`IcSol`]: replicated outcalls through `sol_tf`)
//! and in the tests against fakes (`tests/fakerpc/sol.rs`), which answer as real providers do and run the same transform.
//!
//! Why each transform keeps what it keeps (the replicas of one outcall must produce identical bytes, and so must the providers):
//! - `context.slot`, `context.apiVersion`, `id`, `jsonrpc`, `confirmations`, `rentEpoch`, `space`, simulation logs and units:
//!   dropped (they move between requests or differ between provider versions);
//! - slots and block heights: rounded DOWN to a bucket (the `head_bucket` trick of the Ethereum pin);
//! - send answers: a class (`accepted`, `already_processed`, `blockhash_not_found`, ...), never the provider's prose; and the
//!   send is NOT replicated (spike Q3 §4.8: the replicas' answers differ and consensus would fail although the bytes were sent).
//!   Nothing is ever decided on a send's answer (Q3 §4.2): only `getSignatureStatuses` says what landed.
//!
//! Every read is at `finalized` (Solana RPC has no historical account reads; §2.4), and every outcall's `max_response_bytes` is
//! bounded by what the call can return ([`account_cap`] and the constants below).
use base64::Engine;
use serde_json::{json, Value};

use crate::soltx::{self, Hash, Pubkey};

/// The response cap of a call whose answer is a number or a short object, headers included (a cap is paid for, used or not).
pub const SMALL_BYTES: u64 = 2_000;
/// `getBlock` with `transactionDetails: none`.
pub const BLOCK_BYTES: u64 = 4_000;
/// `sendTransaction`: an error answer can carry simulation logs (dropped by the transform, but counted against the cap).
pub const SEND_BYTES: u64 = 8_000;
/// `simulateTransaction`: its logs (dropped by the transform) count against the cap; an Anchor instruction's run logs a few kB.
pub const SIM_BYTES: u64 = 24_000;
/// At most this many keys per `getMultipleAccounts` (the RPC's own limit is 100).
pub const MAX_KEYS: usize = 100;
/// A sliced `getProgramAccounts` ([`program_slices`]) is first asked for at most this many entries (a ≈ 16 kB cap instead of the
/// full listing's), and again at its full bound only when that read fails.
pub const SLICES_FIRST: usize = 64;
/// At most this many signatures per `getSignatureStatuses` (the RPC's own limit is 256).
pub const MAX_SIGS: usize = 64;
/// Slots and heights read for ordering are rounded down to this many (spike Q3 §5: two providers asked moments apart agree).
pub const SLOT_BUCKET: u64 = 20;
/// The headers and the JSON-RPC envelope of an account answer.
const ACCOUNT_ENVELOPE: u64 = 2_000;

/// The response cap for an account of at most `max_data` bytes: base64 grows data by 4/3.
pub fn account_cap(max_data: usize) -> u64 {
    ACCOUNT_ENVELOPE + (max_data as u64).div_ceil(3) * 4 + 200
}

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
    Accounts = 7,
    /// `getProgramAccounts`, sorted by address (providers may answer in any order).
    ProgramAccounts = 8,
    /// `getSignaturesForAddress`: `(signature, slot, err, memo)` per entry, newest first as the RPC orders them.
    Signatures = 9,
    /// `getTransaction` (`json`), projected to what `relaysol`'s readers use.
    Transaction = 10,
    /// `simulateTransaction` (phase B2: simulated BEFORE it is signed): its error only (logs and units move between providers).
    Simulate = 11,
    /// A plain string result (`getGenesisHash`: the cluster check).
    Text = 12,
    /// `getProgramAccounts` with a `dataSlice`: `(pubkey, the slice)` per account, sorted (phase B2: listing past 64 vaults).
    ProgramSlices = 13,
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
            7 => Kind::Accounts,
            8 => Kind::ProgramAccounts,
            9 => Kind::Signatures,
            10 => Kind::Transaction,
            11 => Kind::Simulate,
            12 => Kind::Text,
            13 => Kind::ProgramSlices,
            _ => return None,
        })
    }
}

/// The transform's context: the kind, then the bucket (u64 LE).
pub fn context(kind: Kind, bucket: u64) -> Vec<u8> {
    let mut c = vec![kind as u8];
    c.extend(bucket.to_le_bytes());
    c
}

fn account_view(a: &Value) -> Value {
    if a.is_null() {
        return Value::Null;
    }
    // `data` is `[base64, "base64"]` when asked so; anything else is kept whole, and so disagrees or fails to decode later
    let data = if a["data"][1] == "base64" { a["data"][0].clone() } else { json!({ "unexpected": a["data"] }) };
    json!({ "lamports": a["lamports"], "owner": a["owner"], "executable": a["executable"], "data": data })
}

fn ix_view(ix: &Value) -> Value {
    json!({ "programIdIndex": ix["programIdIndex"], "accounts": ix["accounts"], "data": ix["data"] })
}

/// A finalized transaction, `json` encoded, reduced to the fields `relaysol::{tx_instructions, program_data, lamport_delta,
/// refund_of, deposit_in}` read: everything else (block time, compute units, version-dependent extras) moves between providers.
fn transaction_view(t: &Value) -> Value {
    if t.is_null() {
        return Value::Null;
    }
    let m = &t["meta"];
    let inner: Vec<Value> = m["innerInstructions"].as_array().into_iter().flatten().map(|g| json!({ "index": g["index"],
        "instructions": g["instructions"].as_array().into_iter().flatten().map(ix_view).collect::<Vec<_>>() })).collect();
    json!({
        "transaction": { "message": { "accountKeys": t["transaction"]["message"]["accountKeys"],
            "instructions": t["transaction"]["message"]["instructions"].as_array().into_iter().flatten().map(ix_view).collect::<Vec<_>>() } },
        "meta": { "err": m["err"], "preBalances": m["preBalances"], "postBalances": m["postBalances"], "innerInstructions": inner,
            "loadedAddresses": m["loadedAddresses"], "logMessages": m["logMessages"] },
    })
}

fn status_view(s: &Value) -> Value {
    if s.is_null() {
        Value::Null
    } else {
        json!({ "status": s["confirmationStatus"], "err": s["err"] })
    }
}

/// The transform body (pure; tested natively on real devnet answers): what every replica and every provider must agree on.
pub fn normalise(status: u32, body: &[u8], ctx: &[u8]) -> Value {
    if status != 200 {
        return json!({ "http": status });
    }
    let Some(kind) = ctx.first().and_then(|b| Kind::of(*b)) else { return json!({ "bad_context": true }) };
    let bucket = ctx.get(1..9).map(|b| u64::from_le_bytes(b.try_into().expect("8"))).unwrap_or(1).max(1);
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
        Kind::Account => json!({ "account": account_view(val) }),
        Kind::Accounts => match val.as_array() {
            Some(a) => json!({ "accounts": a.iter().map(account_view).collect::<Vec<_>>() }),
            None => json!({ "unexpected": true }),
        },
        // audit A4-04: a status read bound to a slot (the bucket, > 1) is refused as `stale` when the provider's own context is older
        // than it (a load-balanced backend behind the one that answered the lifetime read); the slot itself is then dropped as before
        Kind::SigStatus if bucket > 1 && r["context"]["slot"].as_u64().is_none_or(|s| s < bucket) => json!({ "stale": true }),
        Kind::SigStatus => match val.as_array() {
            Some(a) => json!({ "statuses": a.iter().map(status_view).collect::<Vec<_>>() }),
            None => json!({ "unexpected": true }),
        },
        Kind::Send => match r.as_str() {
            Some(_) => json!({ "class": "accepted" }),
            None => json!({ "class": "unexpected" }),
        },
        Kind::Block => json!({ "blockhash": val["blockhash"], "blockHeight": val["blockHeight"], "parentSlot": val["parentSlot"] }),
        Kind::ProgramAccounts => match val.as_array() {
            Some(a) => {
                let mut v: Vec<Value> = a.iter().map(|x| json!({ "pubkey": x["pubkey"], "account": account_view(&x["account"]) })).collect();
                v.sort_by(|x, y| x["pubkey"].as_str().cmp(&y["pubkey"].as_str()));
                json!({ "program_accounts": v })
            }
            None => json!({ "unexpected": true }),
        },
        Kind::Signatures => match val.as_array() {
            Some(a) => json!({ "signatures": a.iter().map(|x| json!({ "signature": x["signature"], "slot": x["slot"], "err": x["err"],
                "memo": x["memo"] })).collect::<Vec<_>>() }),
            None => json!({ "unexpected": true }),
        },
        Kind::Transaction => json!({ "transaction": transaction_view(val) }),
        Kind::Simulate => match val.as_object() {
            Some(o) => json!({ "sim_err": o.get("err").cloned().unwrap_or(Value::Null) }),
            None => json!({ "unexpected": true }),
        },
        Kind::Text => match val.as_str() {
            Some(t) => json!({ "text": t }),
            None => json!({ "unexpected": true }),
        },
        Kind::ProgramSlices => match val.as_array() {
            Some(a) => {
                let mut v: Vec<Value> = a.iter().map(|x| json!({ "pubkey": x["pubkey"], "data": x["account"]["data"][0],
                    "owner": x["account"]["owner"] })).collect();
                v.sort_by(|x, y| x["pubkey"].as_str().cmp(&y["pubkey"].as_str()));
                json!({ "slices": v })
            }
            None => json!({ "unexpected": true }),
        },
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
                // the RPC verifies signatures BEFORE it simulates: reaching this class proves the signature valid (Q3 §3.4)
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

/// The one transform every Solana outcall names.
#[ic_cdk::query]
fn sol_tf(args: ic_cdk::management_canister::TransformArgs) -> ic_cdk::management_canister::HttpRequestResult {
    let status = u32::try_from(args.response.status.0.clone()).unwrap_or(0);
    let body = normalise(status, &args.response.body, &args.context).to_string().into_bytes();
    ic_cdk::management_canister::HttpRequestResult { status: candid::Nat::from(200u32), body, headers: vec![] }
}

/// One JSON-RPC request.
#[derive(Clone, Debug)]
pub struct Req {
    pub method: &'static str,
    pub params: Value,
    pub kind: Kind,
    pub bucket: u64,
    pub max_bytes: u64,
    /// `false` only for `sendTransaction` (Q3 §4.8).
    pub replicated: bool,
}

impl Req {
    pub fn read(method: &'static str, params: Value, kind: Kind, bucket: u64, max_bytes: u64) -> Req {
        Req { method, params, kind, bucket, max_bytes, replicated: true }
    }
    pub fn body(&self) -> String {
        json!({ "jsonrpc": "2.0", "id": 1, "method": self.method, "params": self.params }).to_string()
    }
}

/// One provider. On the IC, one outcall through `sol_tf`; in the tests, a fake that answers as a real provider and runs
/// [`normalise`] itself. The answer is the TRANSFORMED one. An `Err` is the outcall's own failure (consensus, network, cap).
#[allow(async_fn_in_trait)]
pub trait SolRpc {
    fn name(&self) -> String;
    async fn ask(&mut self, r: &Req) -> Result<Value, String>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refused {
    /// an outcall failed (consensus, network, cap) or an answer was malformed: retry next tick
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

impl From<Refused> for String {
    fn from(r: Refused) -> String {
        r.to_string()
    }
}

fn failed(e: impl std::fmt::Display) -> Refused {
    Refused::Failed(e.to_string())
}

/// Every provider's answer, concurrently (two providers asked moments apart disagree more often: Q3 §5).
pub async fn each<P: SolRpc>(ps: &mut [P], r: &Req) -> Vec<Result<Value, String>> {
    let names: Vec<String> = ps.iter().map(|p| p.name()).collect();
    let out = futures::future::join_all(ps.iter_mut().map(|p| p.ask(r))).await;
    out.into_iter().zip(names).map(|(a, n)| a.map_err(|e| format!("{n}: {e}"))).collect()
}

/// `rpc_agreed`: at least two providers, all answer, and identically after the transform.
pub async fn agreed<P: SolRpc>(ps: &mut [P], r: &Req) -> Result<Value, Refused> {
    if ps.len() < 2 {
        return Err(failed(format!("{} provider(s): two at least, one is not a check", ps.len())));
    }
    agree(each(ps, r).await)
}

/// The agreement rule, pure: an error answer of the RPC (`{"error": code}`, `{"http": n}`, ...) is agreed like any other, and the
/// typed readers below refuse it.
pub fn agree(answers: Vec<Result<Value, String>>) -> Result<Value, Refused> {
    let mut vals = Vec::new();
    for a in answers {
        vals.push(a.map_err(Refused::Failed)?);
    }
    match vals.first() {
        None => Err(failed("no providers")),
        Some(v0) if vals.iter().all(|v| v == v0) => Ok(v0.clone()),
        _ => Err(Refused::Disagree(vals.iter().map(|v| v.to_string()).collect())),
    }
}

// ---- the typed reads ------------------------------------------------------------------------------------------------------------

fn num(v: &Value) -> Result<u64, Refused> {
    v["n"].as_u64().ok_or_else(|| failed(format!("no number in {v}")))
}

/// An account at `finalized`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Account {
    pub lamports: u64,
    pub owner: Pubkey,
    pub executable: bool,
    pub data: Vec<u8>,
}

fn account_of(a: &Value) -> Result<Option<Account>, Refused> {
    if a.is_null() {
        return Ok(None);
    }
    let data = base64::engine::general_purpose::STANDARD.decode(a["data"].as_str().ok_or_else(|| failed(format!("account data in {a}")))?)
        .map_err(failed)?;
    Ok(Some(Account {
        lamports: a["lamports"].as_u64().ok_or_else(|| failed("account lamports"))?,
        owner: soltx::pubkey(a["owner"].as_str().ok_or_else(|| failed("account owner"))?).map_err(failed)?,
        executable: a["executable"].as_bool().unwrap_or(false),
        data,
    }))
}

/// The finalized slot, rounded down to `bucket`.
pub async fn finalized_slot<P: SolRpc>(ps: &mut [P], bucket: u64) -> Result<u64, Refused> {
    num(&agreed(ps, &Req::read("getSlot", json!([{ "commitment": "finalized" }]), Kind::SlotBucket, bucket, SMALL_BYTES)).await?)
}

/// The finalized block height, rounded DOWN to [`SLOT_BUCKET`] (conservative for "has the blockhash expired").
pub async fn finalized_height<P: SolRpc>(ps: &mut [P]) -> Result<u64, Refused> {
    num(&agreed(ps, &Req::read("getBlockHeight", json!([{ "commitment": "finalized" }]), Kind::HeightBucket, SLOT_BUCKET, SMALL_BYTES)).await?)
}

/// A finalized block's hash and height; `None` for a skipped slot.
pub async fn block<P: SolRpc>(ps: &mut [P], slot: u64) -> Result<Option<(Hash, u64)>, Refused> {
    let params = json!([slot, { "commitment": "finalized", "encoding": "base64", "transactionDetails": "none", "rewards": false,
        "maxSupportedTransactionVersion": 0 }]);
    let v = agreed(ps, &Req::read("getBlock", params, Kind::Block, 1, BLOCK_BYTES)).await?;
    if v.get("skipped").is_some() {
        return Ok(None);
    }
    let hash = v["blockhash"].as_str().ok_or_else(|| failed(format!("no blockhash in {v}")))?;
    let height = v["blockHeight"].as_u64().ok_or_else(|| failed(format!("no blockHeight in {v}")))?;
    Ok(Some((soltx::pubkey(hash).map_err(failed)?, height)))
}

/// The fallback anchor (Q3 §2): a finalized slot rounded down to [`SLOT_BUCKET`], the first non-skipped block at or below it.
pub async fn rounded_blockhash<P: SolRpc>(ps: &mut [P]) -> Result<(Hash, u64), Refused> {
    let top = finalized_slot(ps, SLOT_BUCKET).await?;
    for slot in (top.saturating_sub(SLOT_BUCKET - 1)..=top).rev() {
        if let Some(b) = block(ps, slot).await? {
            return Ok(b);
        }
    }
    Err(failed(format!("{SLOT_BUCKET} skipped slots below {top}")))
}

/// One account at `finalized` (`None`: it does not exist), of at most `max_data` bytes.
pub async fn account<P: SolRpc>(ps: &mut [P], key: &Pubkey, max_data: usize) -> Result<Option<Account>, Refused> {
    let params = json!([soltx::b58_encode(key), { "commitment": "finalized", "encoding": "base64" }]);
    let v = agreed(ps, &Req::read("getAccountInfo", params, Kind::Account, 1, account_cap(max_data))).await?;
    if v.get("account").is_none() {
        return Err(failed(format!("getAccountInfo: {v}")));
    }
    account_of(&v["account"])
}

/// `len` bytes of one account from `offset` (`dataSlice`): a header of an account too big to read whole (a program's `ProgramData`,
/// rule S3 of the Relay judge reads 45 bytes of it).
pub async fn account_slice<P: SolRpc>(ps: &mut [P], key: &Pubkey, offset: usize, len: usize) -> Result<Option<Account>, Refused> {
    let params = json!([soltx::b58_encode(key), { "commitment": "finalized", "encoding": "base64", "dataSlice": { "offset": offset, "length": len } }]);
    let v = agreed(ps, &Req::read("getAccountInfo", params, Kind::Account, 1, account_cap(len))).await?;
    if v.get("account").is_none() {
        return Err(failed(format!("getAccountInfo: {v}")));
    }
    account_of(&v["account"])
}

/// Several accounts at one `finalized` slot, in order, each of at most `max_data` bytes.
pub async fn accounts<P: SolRpc>(ps: &mut [P], keys: &[Pubkey], max_data: usize) -> Result<Vec<Option<Account>>, Refused> {
    if keys.is_empty() || keys.len() > MAX_KEYS {
        return Err(failed(format!("{} keys per getMultipleAccounts (1..={MAX_KEYS})", keys.len())));
    }
    let list: Vec<String> = keys.iter().map(|k| soltx::b58_encode(k)).collect();
    let params = json!([list, { "commitment": "finalized", "encoding": "base64" }]);
    let cap = ACCOUNT_ENVELOPE + keys.len() as u64 * (account_cap(max_data) - ACCOUNT_ENVELOPE + 200);
    let v = agreed(ps, &Req::read("getMultipleAccounts", params, Kind::Accounts, 1, cap)).await?;
    let a = v["accounts"].as_array().ok_or_else(|| failed(format!("getMultipleAccounts: {v}")))?;
    if a.len() != keys.len() {
        return Err(failed(format!("{} accounts for {} keys", a.len(), keys.len())));
    }
    a.iter().map(account_of).collect()
}

/// The nonce account's state at `finalized` (it must exist and be initialized).
pub async fn nonce<P: SolRpc>(ps: &mut [P], key: &Pubkey) -> Result<soltx::NonceState, Refused> {
    match account(ps, key, soltx::NONCE_ACCOUNT_SPACE as usize).await? {
        None => Err(failed("the nonce account does not exist (at finalized)")),
        Some(a) if a.owner != soltx::SYSTEM_PROGRAM => Err(failed("the nonce account is not owned by the System Program")),
        Some(a) => soltx::decode_nonce(&a.data).map_err(failed),
    }
}

/// One program's accounts at `finalized` whose `data_size` is exact and whose bytes at `offset` are `bytes` (a `memcmp`), sorted by
/// address, at most `max_accounts` of at most `max_data` bytes each (the response cap is paid for whatever comes back).
pub async fn program_accounts<P: SolRpc>(ps: &mut [P], program: &Pubkey, data_size: usize, offset: usize, bytes: &[u8],
    max_accounts: usize, max_data: usize) -> Result<Vec<(Pubkey, Account)>, Refused> {
    let filters = json!([{ "dataSize": data_size }, { "memcmp": { "offset": offset, "bytes": soltx::b58_encode(bytes) } }]);
    let params = json!([soltx::b58_encode(program), { "commitment": "finalized", "encoding": "base64", "filters": filters }]);
    let cap = ACCOUNT_ENVELOPE + max_accounts as u64 * (account_cap(max_data) - ACCOUNT_ENVELOPE + 300);
    let v = agreed(ps, &Req::read("getProgramAccounts", params, Kind::ProgramAccounts, 1, cap)).await?;
    let a = v["program_accounts"].as_array().ok_or_else(|| failed(format!("getProgramAccounts: {v}")))?;
    if a.len() > max_accounts {
        return Err(failed(format!("{} accounts, over the {max_accounts} read", a.len())));
    }
    a.iter().map(|x| {
        let k = soltx::pubkey(x["pubkey"].as_str().ok_or_else(|| failed("no pubkey"))?).map_err(failed)?;
        Ok((k, account_of(&x["account"])?.ok_or_else(|| failed("a null account"))?))
    }).collect()
}

/// The finalized clock: a finalized slot rounded down to [`SLOT_BUCKET`] and its block's unix time (`getBlockTime`; a skipped slot
/// steps down, as [`rounded_blockhash`] does). `(unix seconds, slot)`: deadlines are unix time, the slot only orders (§2.4 "Clocks").
pub async fn finalized_clock<P: SolRpc>(ps: &mut [P]) -> Result<(u64, u64), Refused> {
    let top = finalized_slot(ps, SLOT_BUCKET).await?;
    for slot in (top.saturating_sub(SLOT_BUCKET - 1)..=top).rev() {
        let v = agreed(ps, &Req::read("getBlockTime", json!([slot]), Kind::Number, 1, SMALL_BYTES)).await?;
        if let Some(t) = v["n"].as_u64() {
            return Ok((t, slot));
        }
    }
    Err(failed(format!("no block time for the {SLOT_BUCKET} slots under {top}")))
}

/// One finalized signature of `getSignaturesForAddress`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SigInfo {
    pub signature: String,
    pub slot: u64,
    pub failed: bool,
    pub memo: Option<String>,
}

/// The finalized signatures touching `addr` newer than `until` (exclusive; the RPC's own cursor), newest first, at most `limit`.
pub async fn signatures_since<P: SolRpc>(ps: &mut [P], addr: &Pubkey, until: &str, limit: usize) -> Result<Vec<SigInfo>, Refused> {
    signatures_page(ps, addr, json!({ "commitment": "finalized", "until": until, "limit": limit }), limit).await
}

/// Audit B1-12: the page older than `before` (exclusive) and newer than `until`, newest first, at most `limit`.
pub async fn signatures_between<P: SolRpc>(ps: &mut [P], addr: &Pubkey, until: &str, before: &str, limit: usize) -> Result<Vec<SigInfo>, Refused> {
    signatures_page(ps, addr, json!({ "commitment": "finalized", "until": until, "before": before, "limit": limit }), limit).await
}

async fn signatures_page<P: SolRpc>(ps: &mut [P], addr: &Pubkey, cfg: serde_json::Value, limit: usize) -> Result<Vec<SigInfo>, Refused> {
    let params = json!([soltx::b58_encode(addr), cfg]);
    let v = agreed(ps, &Req::read("getSignaturesForAddress", params, Kind::Signatures, 1, SMALL_BYTES + 400 * limit as u64)).await?;
    let a = v["signatures"].as_array().ok_or_else(|| failed(format!("getSignaturesForAddress: {v}")))?;
    a.iter().map(|x| Ok(SigInfo {
        signature: x["signature"].as_str().ok_or_else(|| failed("no signature"))?.to_string(),
        slot: x["slot"].as_u64().unwrap_or(0),
        failed: !x["err"].is_null(),
        memo: x["memo"].as_str().map(String::from),
    })).collect()
}

/// A finalized transaction (`json`), projected as [`normalise`] keeps it; `None` when no provider has it at `finalized`.
pub async fn transaction<P: SolRpc>(ps: &mut [P], sig: &str, max_bytes: u64) -> Result<Option<Value>, Refused> {
    let params = json!([sig, { "commitment": "finalized", "encoding": "json", "maxSupportedTransactionVersion": 0 }]);
    let v = agreed(ps, &Req::read("getTransaction", params, Kind::Transaction, 1, max_bytes)).await?;
    match v.get("transaction") {
        Some(Value::Null) => Ok(None),
        Some(t) => Ok(Some(t.clone())),
        None => Err(failed(format!("getTransaction: {v}"))),
    }
}

pub async fn balance<P: SolRpc>(ps: &mut [P], key: &Pubkey) -> Result<u64, Refused> {
    num(&agreed(ps, &Req::read("getBalance", json!([soltx::b58_encode(key), { "commitment": "finalized" }]), Kind::Number, 1, SMALL_BYTES)).await?)
}

pub async fn rent_exempt<P: SolRpc>(ps: &mut [P], space: u64) -> Result<u64, Refused> {
    num(&agreed(ps, &Req::read("getMinimumBalanceForRentExemption", json!([space, { "commitment": "finalized" }]), Kind::Number, 1, SMALL_BYTES)).await?)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Processed,
    Confirmed,
    Finalized,
}

/// A signature's status as `getSignatureStatuses` (with history) reports it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SigStatus {
    pub level: Level,
    /// The transaction's error, as JSON text (`None`: it succeeded).
    pub err: Option<String>,
}

/// `getSignatureStatuses` with `searchTransactionHistory`, agreed: one entry per signature (`None`: unknown to both providers).
/// A provider that is merely behind (`confirmed` against `finalized`) is a disagreement too, and resolves itself (Q3 §5).
pub async fn statuses<P: SolRpc>(ps: &mut [P], sigs: &[String]) -> Result<Vec<Option<SigStatus>>, Refused> {
    statuses_at(ps, sigs, 0).await?.ok_or_else(|| failed("a status read with no slot bound answered stale"))
}

/// [`statuses`] bound to `min_slot` (audit A4-04): `Ok(None)` when a provider answered from a context older than `min_slot` (its view
/// may predate the lifetime fact read before it: nothing "unknown" in it is believed). `min_slot` ≤ 1: unbound. Asked in chunks of
/// [`MAX_SIGS`] (audit A4-03: a long history is never refused for its length).
pub async fn statuses_at<P: SolRpc>(ps: &mut [P], sigs: &[String], min_slot: u64) -> Result<Option<Vec<Option<SigStatus>>>, Refused> {
    if sigs.is_empty() {
        return Err(failed("no signatures for getSignatureStatuses"));
    }
    let mut out = Vec::with_capacity(sigs.len());
    for chunk in sigs.chunks(MAX_SIGS) {
        match statuses_chunk(ps, chunk, min_slot).await? {
            Some(v) => out.extend(v),
            None => return Ok(None),
        }
    }
    Ok(Some(out))
}

async fn statuses_chunk<P: SolRpc>(ps: &mut [P], sigs: &[String], min_slot: u64) -> Result<Option<Vec<Option<SigStatus>>>, Refused> {
    if sigs.is_empty() || sigs.len() > MAX_SIGS {
        return Err(failed(format!("{} signatures per getSignatureStatuses (1..={MAX_SIGS})", sigs.len())));
    }
    let params = json!([sigs, { "searchTransactionHistory": true }]);
    let v = agreed(ps, &Req::read("getSignatureStatuses", params, Kind::SigStatus, min_slot.max(1), SMALL_BYTES + 256 * sigs.len() as u64)).await?;
    if v.get("stale").is_some() {
        return Ok(None);
    }
    let a = v["statuses"].as_array().ok_or_else(|| failed(format!("getSignatureStatuses: {v}")))?;
    if a.len() != sigs.len() {
        return Err(failed(format!("{} statuses for {} signatures", a.len(), sigs.len())));
    }
    a.iter().map(|s| {
        if s.is_null() {
            return Ok(None);
        }
        let level = match s["status"].as_str() {
            Some("processed") => Level::Processed,
            Some("confirmed") => Level::Confirmed,
            Some("finalized") => Level::Finalized,
            other => return Err(failed(format!("confirmationStatus {other:?}"))),
        };
        Ok(Some(SigStatus { level, err: (!s["err"].is_null()).then(|| s["err"].to_string()) }))
    }).collect::<Result<Vec<_>, _>>().map(Some)
}

/// `sendTransaction` of saved bytes to EVERY provider, NOT replicated (Q3 §4.8). The answers are for the log only: either provider
/// forwarding is enough, and nothing is ever decided on them (Q3 §4.2: a landed durable-nonce transaction resent is answered
/// `BlockhashNotFound`, the same class a nonce moved by someone else gives).
pub async fn send<P: SolRpc>(ps: &mut [P], tx: &[u8], preflight: bool) -> Vec<Result<Value, String>> {
    let b64 = base64::engine::general_purpose::STANDARD.encode(tx);
    let params = json!([b64, { "encoding": "base64", "skipPreflight": !preflight, "preflightCommitment": "finalized", "maxRetries": 0 }]);
    let r = Req { method: "sendTransaction", params, kind: Kind::Send, bucket: 1, max_bytes: SEND_BYTES, replicated: false };
    each(ps, &r).await
}


/// `getProgramAccounts` with `dataSize` and any number of `memcmp` filters, returning only `slice` (offset, length) of each account:
/// ≈ 150 bytes an entry, so one outcall lists thousands (phase B2: a collection's rook_backing vaults past the 64 a whole-account
/// read carries). Sorted by address; refused past `max_accounts` (loud: the caller says how many it can take).
pub async fn program_slices<P: SolRpc>(ps: &mut [P], program: &Pubkey, data_size: usize, memcmps: &[(usize, Vec<u8>)], slice: (usize, usize),
    max_accounts: usize) -> Result<Vec<(Pubkey, Pubkey, Vec<u8>)>, Refused> {
    let mut filters = vec![json!({ "dataSize": data_size })];
    for (o, b) in memcmps {
        filters.push(json!({ "memcmp": { "offset": o, "bytes": soltx::b58_encode(b) } }));
    }
    let params = json!([soltx::b58_encode(program), { "commitment": "finalized", "encoding": "base64", "filters": filters,
        "dataSlice": { "offset": slice.0, "length": slice.1 } }]);
    let per = 180 + (slice.1 as u64).div_ceil(3) * 4;
    let read = |n: usize| Req::read("getProgramAccounts", params.clone(), Kind::ProgramSlices, 1, ACCOUNT_ENVELOPE + n as u64 * per);
    // the cost review of 2026-10-09: a response cap is paid for whether it is used or not (≈ 10,400 cycles a byte on 13 nodes, per
    // provider), and the full listing's cap is ≈ 0.9 MB (≈ 9.6 B cycles a provider). So the listing is asked first at
    // [`SLICES_FIRST`] entries; only when that read FAILS (an answer over its cap is refused by the IC, never truncated) is it asked
    // again at `max_accounts`. A disagreement is not a size problem: it is refused at once, as every read.
    let v = if max_accounts > SLICES_FIRST {
        match agreed(ps, &read(SLICES_FIRST)).await {
            Ok(v) => v,
            Err(Refused::Failed(_)) => agreed(ps, &read(max_accounts)).await?,
            Err(e) => return Err(e),
        }
    } else {
        agreed(ps, &read(max_accounts)).await?
    };
    let a = v["slices"].as_array().ok_or_else(|| failed(format!("getProgramAccounts (sliced): {v}")))?;
    if a.len() > max_accounts {
        return Err(failed(format!("{} accounts, over the {max_accounts} listed", a.len())));
    }
    a.iter().map(|x| {
        let k = soltx::pubkey(x["pubkey"].as_str().ok_or_else(|| failed("no pubkey"))?).map_err(failed)?;
        let owner = soltx::pubkey(x["owner"].as_str().ok_or_else(|| failed("no owner"))?).map_err(failed)?;
        let d = base64::engine::general_purpose::STANDARD.decode(x["data"].as_str().ok_or_else(|| failed("no data"))?).map_err(failed)?;
        Ok((k, owner, d))
    }).collect()
}

/// The cluster's genesis hash (base58), agreed: the Solana form of the EVM home's `eth_chainId` guard (phase B2).
pub async fn genesis_hash<P: SolRpc>(ps: &mut [P]) -> Result<String, Refused> {
    let v = agreed(ps, &Req::read("getGenesisHash", json!([]), Kind::Text, 1, SMALL_BYTES)).await?;
    v["text"].as_str().map(String::from).ok_or_else(|| failed(format!("getGenesisHash: {v}")))
}

/// The genesis hash of each cluster an install may name (`canister::SOLANA_CLUSTERS`).
pub fn genesis_of(cluster: &str) -> Option<&'static str> {
    match cluster {
        "mainnet-beta" => Some("5eykt4UsFv8P8NJdTREpY1vzqKqZKvdpKuc147dw2N9d"),
        "devnet" => Some("EtWTRABZaYq6iMfeYKouRu166VU2xqa1wcaWoxPkrZBG"),
        "testnet" => Some("4uhcVJyU9pJkvQyS88uRDiswHXSDkY9xV8AYqv2UwvNH"),
        _ => None,
    }
}

/// What every provider's simulation of an UNSIGNED transaction says (phase B2: simulated before it is signed, so a step that would
/// fail costs no threshold signature): `Ok(None)` it succeeds, `Ok(Some(err))` every provider agrees it fails with `err` (JSON
/// text), `Err` the providers could not be read or disagree (nothing is signed this tick either). `sigVerify` off, the blockhash
/// (or durable nonce) as written, at `finalized`, as every read is.
pub async fn simulate<P: SolRpc>(ps: &mut [P], unsigned_tx: &[u8]) -> Result<Option<String>, Refused> {
    let b64 = base64::engine::general_purpose::STANDARD.encode(unsigned_tx);
    let params = json!([b64, { "encoding": "base64", "sigVerify": false, "replaceRecentBlockhash": false, "commitment": "finalized" }]);
    let v = agreed(ps, &Req::read("simulateTransaction", params, Kind::Simulate, 1, SIM_BYTES)).await?;
    match v.get("sim_err") {
        Some(Value::Null) => Ok(None),
        Some(e) => Ok(Some(e.to_string())),
        None => Err(failed(format!("simulateTransaction: {v}"))),
    }
}

// ---- the IC provider ------------------------------------------------------------------------------------------------------------

/// One configured Solana provider, over HTTPS outcalls through `sol_tf`.
pub struct IcSol {
    pub url: String,
}

/// Every configured provider (`Home::Solana.rpc_urls`).
pub fn ic_providers(urls: &[String]) -> Vec<IcSol> {
    urls.iter().map(|u| IcSol { url: u.clone() }).collect()
}

impl SolRpc for IcSol {
    fn name(&self) -> String {
        // the host only: a keyed URL's path or query must never reach a log
        self.url.split("://").nth(1).unwrap_or(&self.url).split(['/', '?']).next().unwrap_or("").to_string()
    }
    async fn ask(&mut self, r: &Req) -> Result<Value, String> {
        use ic_cdk::management_canister::{http_request, HttpHeader, HttpMethod, HttpRequestArgs, TransformContext, TransformFunc};
        let req = HttpRequestArgs {
            url: self.url.clone(),
            method: HttpMethod::POST,
            body: Some(r.body().into_bytes()),
            max_response_bytes: Some(r.max_bytes),
            transform: Some(TransformContext {
                function: TransformFunc(candid::Func { principal: ic_cdk::api::canister_self(), method: "sol_tf".into() }),
                context: context(r.kind, r.bucket),
            }),
            headers: vec![HttpHeader { name: "content-type".into(), value: "application/json".into() }],
            is_replicated: Some(r.replicated),
        };
        let res = http_request(&req).await.map_err(crate::canister::outcall_error)?;
        serde_json::from_slice(&res.body).map_err(|e| format!("transform output: {e}"))
    }
}

/// `len` bytes from `offset` of several accounts at one `finalized` slot (`getMultipleAccounts` with a `dataSlice`), in order: the
/// registry keeper reads a field of many pools (DBC's `is_migrated`, a partner's unclaimed fee) without paying for whole accounts.
pub async fn accounts_slice<P: SolRpc>(ps: &mut [P], keys: &[Pubkey], offset: usize, len: usize) -> Result<Vec<Option<Account>>, Refused> {
    if keys.is_empty() || keys.len() > MAX_KEYS {
        return Err(failed(format!("{} keys per getMultipleAccounts (1..={MAX_KEYS})", keys.len())));
    }
    let list: Vec<String> = keys.iter().map(|k| soltx::b58_encode(k)).collect();
    let params = json!([list, { "commitment": "finalized", "encoding": "base64", "dataSlice": { "offset": offset, "length": len } }]);
    let cap = ACCOUNT_ENVELOPE + keys.len() as u64 * (account_cap(len) - ACCOUNT_ENVELOPE + 200);
    let v = agreed(ps, &Req::read("getMultipleAccounts", params, Kind::Accounts, 1, cap)).await?;
    let a = v["accounts"].as_array().ok_or_else(|| failed(format!("getMultipleAccounts: {v}")))?;
    if a.len() != keys.len() {
        return Err(failed(format!("{} accounts for {} keys", a.len(), keys.len())));
    }
    a.iter().map(account_of).collect()
}

/// `owner`'s token accounts of `token_program` at `finalized` (`getTokenAccountsByOwner`, `slice` of each), sorted by address, at most
/// `max_accounts` (the registry keeper's discovery of the DAMM v2 position NFTs Rook's claimer PDA holds).
pub async fn token_accounts<P: SolRpc>(ps: &mut [P], owner: &Pubkey, token_program: &Pubkey, slice: (usize, usize), max_accounts: usize)
    -> Result<Vec<(Pubkey, Account)>, Refused> {
    let params = json!([soltx::b58_encode(owner), { "programId": soltx::b58_encode(token_program) },
        { "commitment": "finalized", "encoding": "base64", "dataSlice": { "offset": slice.0, "length": slice.1 } }]);
    let read = |n: usize| Req::read("getTokenAccountsByOwner", params.clone(), Kind::ProgramAccounts, 1,
        ACCOUNT_ENVELOPE + n as u64 * (account_cap(slice.1) - ACCOUNT_ENVELOPE + 300));
    // as `program_slices`: a response cap is paid for whether used or not, so the small cap first and the full one only when it fails
    let v = if max_accounts > SLICES_FIRST {
        match agreed(ps, &read(SLICES_FIRST)).await {
            Ok(v) => v,
            Err(Refused::Failed(_)) => agreed(ps, &read(max_accounts)).await?,
            Err(e) => return Err(e),
        }
    } else {
        agreed(ps, &read(max_accounts)).await?
    };
    let a = v["program_accounts"].as_array().ok_or_else(|| failed(format!("getTokenAccountsByOwner: {v}")))?;
    if a.len() > max_accounts {
        return Err(failed(format!("{} token accounts, over the {max_accounts} read", a.len())));
    }
    a.iter().map(|x| {
        let k = soltx::pubkey(x["pubkey"].as_str().ok_or_else(|| failed("no pubkey"))?).map_err(failed)?;
        Ok((k, account_of(&x["account"])?.ok_or_else(|| failed("a null account"))?))
    }).collect()
}
