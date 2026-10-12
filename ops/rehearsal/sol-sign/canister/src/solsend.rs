//! The saved-bytes send line (§2.4): "sign once, save the bytes, rebroadcast until mined, at most one mines", with the transaction's
//! LIFETIME behind one trait so the mechanism can be swapped when durable nonces go:
//!
//! - `DurableNonce` (default): `AdvanceNonceAccount` first, the nonce account's stored value as `recent_blockhash`. The saved bytes
//!   stay valid until they land or the nonce is advanced by something else; they never expire with time.
//! - `RecentBlockhash` (the fallback): `getSlot(finalized)` rounded down to 20 slots → `getBlock` (walking back over skipped slots)
//!   → its blockhash, valid until block height `blockHeight + 150`. Once expiry is PROVEN (finalized height past it, then the
//!   signature still unknown) the same intent is re-signed on a fresh blockhash; the old bytes can no longer land, so at most one mines.
//!
//! Ordering rule for "can never land" (found while writing this): read the lifetime fact FIRST (nonce value / finalized height), the
//! signature status SECOND. The other order races a landing between the two reads into a wrong "expired" and a second send.
use crate::solrpc::{self, agreed, Call, Kind, Refused};
use crate::soltx::{self, Hash, Instruction, Pubkey};
use base64::Engine;
use serde_json::{json, Value};

pub const SLOT_BUCKET: u64 = 20;
/// `MAX_PROCESSING_AGE`: a blockhash is valid for 150 blocks after its own.
pub const BLOCKHASH_VALID_BLOCKS: u64 = 150;

pub struct Ctx<'a> {
    pub urls: &'a [String],
    pub me: Pubkey,
    pub replicated_send: bool,
}

/// What a lifetime puts in a new transaction.
#[derive(Clone, Debug)]
pub struct Anchor {
    pub recent_blockhash: Hash,
    pub prefix: Vec<Instruction>,
    pub last_valid_height: Option<u64>,
}

/// The one trait the real canister's `solsend.rs` would hide the mechanism behind.
#[allow(async_fn_in_trait)]
pub trait Lifetime {
    fn name(&self) -> &'static str;
    /// the `recent_blockhash` field and the prefix instructions for a transaction signed now
    async fn anchor(&self, ctx: &Ctx<'_>) -> Result<Anchor, Refused>;
    /// proven that the saved bytes can never land (so a re-sign under the same intent cannot double-spend); `Ok(false)` otherwise
    async fn can_never_land(&self, saved: &Saved, ctx: &Ctx<'_>) -> Result<bool, Refused>;
}

pub struct DurableNonce {
    pub account: Pubkey,
}

pub struct RecentBlockhash;

impl Lifetime for DurableNonce {
    fn name(&self) -> &'static str {
        "durable-nonce"
    }
    async fn anchor(&self, ctx: &Ctx<'_>) -> Result<Anchor, Refused> {
        let n = nonce_state(ctx, &self.account).await?;
        if n.authority != ctx.me {
            return Err(Refused::Failed("the nonce account's authority is not this canister".into()));
        }
        Ok(Anchor { recent_blockhash: n.durable_nonce, prefix: vec![soltx::advance_nonce(self.account, ctx.me)], last_valid_height: None })
    }
    async fn can_never_land(&self, saved: &Saved, ctx: &Ctx<'_>) -> Result<bool, Refused> {
        let n = nonce_state(ctx, &self.account).await?; // FIRST
        if n.durable_nonce == saved.recent_blockhash {
            return Ok(false); // the nonce has not moved: the saved bytes are still good
        }
        Ok(sig_status(ctx, &saved.signature).await?["status"].is_null()) // SECOND
    }
}

impl Lifetime for RecentBlockhash {
    fn name(&self) -> &'static str {
        "recent-blockhash"
    }
    async fn anchor(&self, ctx: &Ctx<'_>) -> Result<Anchor, Refused> {
        let (hash, height) = rounded_blockhash(ctx).await?;
        Ok(Anchor { recent_blockhash: hash, prefix: vec![], last_valid_height: Some(height + BLOCKHASH_VALID_BLOCKS) })
    }
    async fn can_never_land(&self, saved: &Saved, ctx: &Ctx<'_>) -> Result<bool, Refused> {
        let Some(last) = saved.last_valid_height else { return Ok(false) };
        let h = finalized_height(ctx).await?; // FIRST (rounded DOWN: conservative)
        if h <= last {
            return Ok(false);
        }
        Ok(sig_status(ctx, &saved.signature).await?["status"].is_null()) // SECOND
    }
}

/// One line's saved transaction.
#[derive(Clone, Debug)]
pub struct Saved {
    pub intent: u64,
    pub lifetime: &'static str,
    pub to: Pubkey,
    pub lamports: u64,
    pub recent_blockhash: Hash,
    pub last_valid_height: Option<u64>,
    pub tx: Vec<u8>,
    pub signature: String,
    /// every signature this intent was ever signed under (more than one only after a proven expiry)
    pub history: Vec<String>,
    pub sends: u32,
}

// ---- the reads ------------------------------------------------------------------------------------------------------------------

fn call(method: &str, params: Value, kind: Kind, bucket: u64, max_bytes: u64) -> Call<'_> {
    Call { method, params, kind, bucket, max_bytes, replicated: true }
}

fn num(v: &Value) -> Result<u64, Refused> {
    v["n"].as_u64().ok_or_else(|| Refused::Failed(format!("no number in {v}")))
}

pub async fn finalized_slot(ctx: &Ctx<'_>, bucket: u64) -> Result<u64, Refused> {
    num(&agreed(ctx.urls, &call("getSlot", json!([{ "commitment": "finalized" }]), Kind::SlotBucket, bucket, 1_000)).await?)
}

pub async fn finalized_height(ctx: &Ctx<'_>) -> Result<u64, Refused> {
    num(&agreed(ctx.urls, &call("getBlockHeight", json!([{ "commitment": "finalized" }]), Kind::HeightBucket, SLOT_BUCKET, 1_000)).await?)
}

/// The fallback anchor: a finalized slot rounded down to 20, the first non-skipped block at or below it (≤ 20 tries).
pub async fn rounded_blockhash(ctx: &Ctx<'_>) -> Result<(Hash, u64), Refused> {
    let top = finalized_slot(ctx, SLOT_BUCKET).await?;
    for slot in (top.saturating_sub(SLOT_BUCKET - 1)..=top).rev() {
        let params = json!([slot, { "commitment": "finalized", "encoding": "base64", "transactionDetails": "none", "rewards": false,
            "maxSupportedTransactionVersion": 0 }]);
        let v = agreed(ctx.urls, &call("getBlock", params, Kind::Block, 1, 4_000)).await?;
        if v.get("skipped").is_some() {
            continue;
        }
        let hash = v["blockhash"].as_str().ok_or_else(|| Refused::Failed(format!("no blockhash in {v}")))?;
        let hash: Hash = soltx::pubkey(hash).map_err(Refused::Failed)?;
        let height = v["blockHeight"].as_u64().ok_or_else(|| Refused::Failed(format!("no blockHeight in {v}")))?;
        return Ok((hash, height));
    }
    Err(Refused::Failed(format!("{SLOT_BUCKET} skipped slots below {top}")))
}

pub async fn account(ctx: &Ctx<'_>, key: &Pubkey) -> Result<Option<(u64, Vec<u8>)>, Refused> {
    let params = json!([soltx::b58_encode(key), { "commitment": "finalized", "encoding": "base64" }]);
    let v = agreed(ctx.urls, &call("getAccountInfo", params, Kind::Account, 1, 4_000)).await?;
    let a = &v["account"];
    if a.is_null() {
        return Ok(None);
    }
    let data = base64::engine::general_purpose::STANDARD.decode(a["data"].as_str().unwrap_or("")).map_err(|e| Refused::Failed(e.to_string()))?;
    Ok(Some((a["lamports"].as_u64().unwrap_or(0), data)))
}

pub async fn nonce_state(ctx: &Ctx<'_>, key: &Pubkey) -> Result<soltx::NonceState, Refused> {
    match account(ctx, key).await? {
        None => Err(Refused::Failed("the nonce account does not exist (at finalized)".into())),
        Some((_, data)) => soltx::decode_nonce(&data).map_err(Refused::Failed),
    }
}

pub async fn rent_exempt(ctx: &Ctx<'_>, space: u64) -> Result<u64, Refused> {
    num(&agreed(ctx.urls, &call("getMinimumBalanceForRentExemption", json!([space, { "commitment": "finalized" }]), Kind::Number, 1, 1_000)).await?)
}

pub async fn balance(ctx: &Ctx<'_>, key: &Pubkey) -> Result<u64, Refused> {
    num(&agreed(ctx.urls, &call("getBalance", json!([soltx::b58_encode(key), { "commitment": "finalized" }]), Kind::Number, 1, 1_000)).await?)
}

/// `getSignatureStatuses` with history: `{status: null | "processed" | "confirmed" | "finalized", err}`, agreed.
pub async fn sig_status(ctx: &Ctx<'_>, sig: &str) -> Result<Value, Refused> {
    let params = json!([[sig], { "searchTransactionHistory": true }]);
    agreed(ctx.urls, &call("getSignatureStatuses", params, Kind::SigStatus, 1, 2_000)).await
}

/// Each provider's view of a signature, without agreement (for the report).
pub async fn sig_views(ctx: &Ctx<'_>, sig: &str) -> Vec<Result<Value, String>> {
    let params = json!([[sig], { "searchTransactionHistory": true }]);
    solrpc::each(ctx.urls, &call("getSignatureStatuses", params, Kind::SigStatus, 1, 2_000)).await
}

/// `sendTransaction` of the given bytes to EVERY provider (broadcast, no agreement needed: the signature is known locally).
pub async fn send_bytes(ctx: &Ctx<'_>, tx: &[u8], preflight: bool) -> Vec<Result<Value, String>> {
    let b64 = base64::engine::general_purpose::STANDARD.encode(tx);
    let params = json!([b64, { "encoding": "base64", "skipPreflight": !preflight, "preflightCommitment": "confirmed" }]);
    let c = Call { method: "sendTransaction", params, kind: Kind::Send, bucket: 1, max_bytes: 4_000, replicated: ctx.replicated_send };
    solrpc::each(ctx.urls, &c).await
}
