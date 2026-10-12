//! Spike Q3 (docs/v9/solana-migration.md §2.4, §4; docs/v9/spikes/q3-ic-ed25519-durable-nonce.md): a THROWAWAY canister that holds a
//! Solana identity under threshold Ed25519 (`sign_with_schnorr`, algorithm ed25519), creates a durable-nonce account, signs a transfer
//! that uses it, sends it through HTTPS outcalls to Solana DEVNET, and resends the SAME saved bytes. The fallback lifetime (rounded
//! slot → getBlock → blockhash, re-sign on proven expiry) sits behind the same trait (`solsend::Lifetime`).
//!
//! Every update is controller-only (each spends cycles). State is in heap memory only: the canister is deleted after the run.
pub mod solrpc;
pub mod solsend;
pub mod soltx;

use candid::{CandidType, Deserialize};
use ic_cdk::management_canister::{HttpRequestResult, TransformArgs, schnorr_public_key, sign_with_schnorr, SchnorrAlgorithm, SchnorrKeyId, SchnorrPublicKeyArgs, SignWithSchnorrArgs};
use sha2::{Digest, Sha256};
use solrpc::{CallSample, Refused};
use solsend::{Ctx, DurableNonce, Lifetime, RecentBlockhash, Saved};
use soltx::Pubkey;
use std::cell::RefCell;

#[derive(CandidType, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Nonce,
    Blockhash,
}

#[derive(CandidType, Deserialize, Clone, Debug)]
pub struct Init {
    /// `key_1` (dfx 0.32 local replica: its only Ed25519 key), `test_key_1` (IC mainnet, test key), `key_1` (production)
    pub key_name: String,
    /// ≥ 2 keyless devnet providers
    pub rpc_urls: Vec<String>,
    pub mode: Mode,
    /// sendTransaction as a replicated outcall (every replica sends) or a non-replicated one (one replica sends; cheaper)
    pub replicated_send: bool,
}

#[derive(Default)]
struct State {
    init: Option<Init>,
    me: Option<Pubkey>,
    nonce_account: Option<Pubkey>,
    line: Option<Saved>,
    next_intent: u64,
    signs: Vec<SignSample>,
}

thread_local! {
    static S: RefCell<State> = RefCell::new(State::default());
}

pub const NONCE_SEED: &str = "rook-q3-nonce-0";

#[ic_cdk::init]
fn init(i: Init) {
    assert!(!i.rpc_urls.is_empty(), "at least one provider");
    S.with(|s| s.borrow_mut().init = Some(i));
}

fn guard() {
    if !ic_cdk::api::is_controller(&ic_cdk::api::msg_caller()) {
        ic_cdk::trap("controllers only: every call spends cycles");
    }
}

fn cfg() -> Init {
    S.with(|s| s.borrow().init.clone()).expect("initialised")
}

fn key_id(cfg: &Init) -> SchnorrKeyId {
    SchnorrKeyId { algorithm: SchnorrAlgorithm::Ed25519, name: cfg.key_name.clone() }
}

// ---- the identity -------------------------------------------------------------------------------------------------------------

#[derive(CandidType, Deserialize, Clone, Debug)]
pub struct Identity {
    pub address: String,
    pub key_name: String,
    pub latency_ms: u64,
}

async fn me() -> Result<Pubkey, String> {
    if let Some(k) = S.with(|s| s.borrow().me) {
        return Ok(k);
    }
    let c = cfg();
    let r = schnorr_public_key(&SchnorrPublicKeyArgs { canister_id: None, derivation_path: vec![], key_id: key_id(&c) })
        .await
        .map_err(|e| format!("schnorr_public_key: {e:?}"))?;
    let k: Pubkey = r.public_key.as_slice().try_into().map_err(|_| format!("an ed25519 key is 32 bytes, got {}", r.public_key.len()))?;
    S.with(|s| s.borrow_mut().me = Some(k));
    Ok(k)
}

/// The canister's Solana address: the threshold Ed25519 public key at the empty derivation path, base58.
#[ic_cdk::update]
async fn sol_address() -> Result<Identity, String> {
    guard();
    let t0 = ic_cdk::api::time();
    let k = me().await?;
    Ok(Identity { address: soltx::b58_encode(&k), key_name: cfg().key_name, latency_ms: (ic_cdk::api::time() - t0) / 1_000_000 })
}

// ---- signing --------------------------------------------------------------------------------------------------------------------

#[derive(CandidType, Deserialize, Clone, Debug)]
pub struct SignSample {
    pub message_bytes: u32,
    /// what `ic0.cost_sign_with_schnorr` said and ic-cdk attached
    pub cost_api: u128,
    /// the canister's balance before − after the call (attached − refunded, plus the call's own execution)
    pub balance_delta: u128,
    pub latency_ms: u64,
}

async fn sign(message: &[u8]) -> Result<[u8; 64], String> {
    let c = cfg();
    let arg = SignWithSchnorrArgs { message: message.to_vec(), derivation_path: vec![], key_id: key_id(&c), aux: None };
    let cost = ic_cdk::management_canister::cost_sign_with_schnorr(&arg).map_err(|e| format!("cost: {e:?}"))?;
    let (t0, b0) = (ic_cdk::api::time(), ic_cdk::api::canister_cycle_balance());
    let r = sign_with_schnorr(&arg).await.map_err(|e| format!("sign_with_schnorr: {e:?}"))?;
    let sample = SignSample {
        message_bytes: message.len() as u32,
        cost_api: cost,
        balance_delta: b0.saturating_sub(ic_cdk::api::canister_cycle_balance()),
        latency_ms: (ic_cdk::api::time() - t0) / 1_000_000,
    };
    S.with(|s| s.borrow_mut().signs.push(sample));
    r.signature.as_slice().try_into().map_err(|_| format!("an ed25519 signature is 64 bytes, got {}", r.signature.len()))
}

/// Compile, check the size BEFORE signing, sign once, assemble (and check again).
async fn build(ixs: &[soltx::Instruction], payer: Pubkey, blockhash: soltx::Hash) -> Result<(Vec<u8>, String), String> {
    let msg = soltx::compile(ixs, payer, blockhash);
    soltx::check_size(soltx::wire_size(&msg))?;
    if msg.signers != vec![payer] {
        return Err("the spike signs with one key only".into());
    }
    let sig = sign(&msg.bytes).await?;
    let tx = soltx::assemble(&msg, &[sig])?;
    Ok((tx, soltx::b58_encode(&sig)))
}

// ---- reports ----------------------------------------------------------------------------------------------------------------------

#[derive(CandidType, Deserialize, Clone, Debug)]
pub struct SendReport {
    pub intent: u64,
    pub lifetime: String,
    pub signature: String,
    pub history: Vec<String>,
    pub tx_bytes: u32,
    /// sha256 of the bytes sent: equal across `send_transfer` and every `resend` unless `resigned`
    pub tx_sha256: String,
    pub recent_blockhash: String,
    pub last_valid_height: Option<u64>,
    pub resigned: bool,
    pub sends: u32,
    /// each provider's answer to sendTransaction, classified by the transform
    pub providers: Vec<String>,
    pub latency_ms: u64,
}

fn report(saved: &Saved, resigned: bool, answers: Vec<Result<serde_json::Value, String>>, t0: u64) -> SendReport {
    SendReport {
        intent: saved.intent,
        lifetime: saved.lifetime.into(),
        signature: saved.signature.clone(),
        history: saved.history.clone(),
        tx_bytes: saved.tx.len() as u32,
        tx_sha256: hex(&Sha256::digest(&saved.tx)),
        recent_blockhash: soltx::b58_encode(&saved.recent_blockhash),
        last_valid_height: saved.last_valid_height,
        resigned,
        sends: saved.sends,
        providers: answers.into_iter().map(|a| match a { Ok(v) => v.to_string(), Err(e) => format!("OUTCALL FAILED: {e}") }).collect(),
        latency_ms: (ic_cdk::api::time() - t0) / 1_000_000,
    }
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn err(r: Refused) -> String {
    r.to_string()
}

// ---- the nonce account ----------------------------------------------------------------------------------------------------------

#[derive(CandidType, Deserialize, Clone, Debug)]
pub struct NonceReport {
    pub nonce_account: String,
    pub created_now: bool,
    pub signature: Option<String>,
    pub rent_lamports: u64,
    pub tx_bytes: u32,
    pub providers: Vec<String>,
    /// the current durable nonce, when the account is initialised at finalized
    pub durable_nonce: Option<String>,
}

/// The nonce account at `create_with_seed(me, NONCE_SEED, System)`, authority = this canister. One signature (the seed account needs
/// no key of its own). The creating transaction itself uses the FALLBACK lifetime (there is no nonce yet).
#[ic_cdk::update]
async fn create_nonce_account() -> Result<NonceReport, String> {
    guard();
    let c = cfg();
    let me = me().await?;
    let nonce = soltx::address_with_seed(&me, NONCE_SEED, &soltx::SYSTEM_PROGRAM)?;
    S.with(|s| s.borrow_mut().nonce_account = Some(nonce));
    let ctx = Ctx { urls: &c.rpc_urls, me, replicated_send: c.replicated_send };
    let rent = solsend::rent_exempt(&ctx, soltx::NONCE_ACCOUNT_SPACE).await.map_err(err)?;
    if let Some((_, data)) = solsend::account(&ctx, &nonce).await.map_err(err)? {
        let n = soltx::decode_nonce(&data)?;
        return Ok(NonceReport { nonce_account: soltx::b58_encode(&nonce), created_now: false, signature: None, rent_lamports: rent, tx_bytes: 0,
            providers: vec![], durable_nonce: Some(soltx::b58_encode(&n.durable_nonce)) });
    }
    let anchor = Lifetime::anchor(&RecentBlockhash, &ctx).await.map_err(err)?;
    let ixs = [
        soltx::create_with_seed(me, nonce, me, NONCE_SEED, rent, soltx::NONCE_ACCOUNT_SPACE, soltx::SYSTEM_PROGRAM),
        soltx::initialize_nonce(nonce, me),
    ];
    let (tx, sig) = build(&ixs, me, anchor.recent_blockhash).await?;
    let answers = solsend::send_bytes(&ctx, &tx, true).await;
    Ok(NonceReport {
        nonce_account: soltx::b58_encode(&nonce),
        created_now: true,
        signature: Some(sig),
        rent_lamports: rent,
        tx_bytes: tx.len() as u32,
        providers: answers.into_iter().map(|a| match a { Ok(v) => v.to_string(), Err(e) => format!("OUTCALL FAILED: {e}") }).collect(),
        durable_nonce: None,
    })
}

// ---- the send line ----------------------------------------------------------------------------------------------------------------

fn lifetime(mode: Mode) -> Result<Box<dyn LifetimeAny>, String> {
    Ok(match mode {
        Mode::Nonce => Box::new(DurableNonce { account: S.with(|s| s.borrow().nonce_account).ok_or("no nonce account: call create_nonce_account first")? }),
        Mode::Blockhash => Box::new(RecentBlockhash),
    })
}

/// Object-safe shim over `Lifetime` (its async fns are not dyn-compatible) for the two endpoints below.
pub trait LifetimeAny {
    fn name(&self) -> &'static str;
    fn anchor<'a>(&'a self, ctx: &'a Ctx<'a>) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<solsend::Anchor, Refused>> + 'a>>;
    fn can_never_land<'a>(&'a self, s: &'a Saved, ctx: &'a Ctx<'a>) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<bool, Refused>> + 'a>>;
}

impl<L: Lifetime> LifetimeAny for L {
    fn name(&self) -> &'static str {
        Lifetime::name(self)
    }
    fn anchor<'a>(&'a self, ctx: &'a Ctx<'a>) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<solsend::Anchor, Refused>> + 'a>> {
        Box::pin(Lifetime::anchor(self, ctx))
    }
    fn can_never_land<'a>(&'a self, s: &'a Saved, ctx: &'a Ctx<'a>) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<bool, Refused>> + 'a>> {
        Box::pin(Lifetime::can_never_land(self, s, ctx))
    }
}

async fn sign_intent(l: &dyn LifetimeAny, ctx: &Ctx<'_>, intent: u64, to: Pubkey, lamports: u64, history: Vec<String>) -> Result<Saved, String> {
    let a = l.anchor(ctx).await.map_err(err)?;
    let mut ixs = a.prefix.clone();
    ixs.push(soltx::transfer(ctx.me, to, lamports));
    let (tx, sig) = build(&ixs, ctx.me, a.recent_blockhash).await?;
    let mut history = history;
    history.push(sig.clone());
    Ok(Saved { intent, lifetime: l.name(), to, lamports, recent_blockhash: a.recent_blockhash, last_valid_height: a.last_valid_height, tx,
        signature: sig, history, sends: 0 })
}

/// A new intent: sign ONCE under the current lifetime, SAVE the bytes, send them. Refused while the line's previous transaction is not
/// finalized (one transaction in flight per nonce account / line).
#[ic_cdk::update]
async fn send_transfer(to: String, lamports: u64) -> Result<SendReport, String> {
    guard();
    new_intent(to, lamports, true).await
}

/// The expiry drill (fallback lifetime): sign and SAVE a transfer but do not send it, so that it expires unsent; a later `resend()`
/// must prove expiry, re-sign the same intent, and land exactly one of the two signatures.
#[ic_cdk::update]
async fn hold_transfer(to: String, lamports: u64) -> Result<SendReport, String> {
    guard();
    new_intent(to, lamports, false).await
}

async fn new_intent(to: String, lamports: u64, send: bool) -> Result<SendReport, String> {
    let t0 = ic_cdk::api::time();
    let c = cfg();
    let me = me().await?;
    let to = soltx::pubkey(&to)?;
    let ctx = Ctx { urls: &c.rpc_urls, me, replicated_send: c.replicated_send };
    if let Some(prev) = S.with(|s| s.borrow().line.clone()) {
        let st = solsend::sig_status(&ctx, &prev.signature).await.map_err(err)?;
        if st["status"] != "finalized" {
            return Err(format!("intent {} ({}) is not finalized yet ({st}): resend() it, or wait", prev.intent, prev.signature));
        }
    }
    let intent = S.with(|s| { let mut s = s.borrow_mut(); s.next_intent += 1; s.next_intent });
    let mut saved = sign_intent(lifetime(c.mode)?.as_ref(), &ctx, intent, to, lamports, vec![]).await?;
    S.with(|s| s.borrow_mut().line = Some(saved.clone())); // saved BEFORE the first send
    if !send {
        return Ok(report(&saved, false, vec![], t0));
    }
    let answers = solsend::send_bytes(&ctx, &saved.tx, true).await;
    saved.sends += 1;
    S.with(|s| s.borrow_mut().line = Some(saved.clone()));
    Ok(report(&saved, false, answers, t0))
}

/// Rebroadcast the SAVED bytes (unchanged; `tx_sha256` proves it). Under the fallback lifetime, first ask whether the bytes can never
/// land (expiry proven); only then re-sign the SAME intent on a fresh blockhash and save the new bytes.
#[ic_cdk::update]
async fn resend(preflight: Option<bool>) -> Result<SendReport, String> {
    guard();
    let t0 = ic_cdk::api::time();
    let c = cfg();
    let me = me().await?;
    let ctx = Ctx { urls: &c.rpc_urls, me, replicated_send: c.replicated_send };
    let mut saved = S.with(|s| s.borrow().line.clone()).ok_or("nothing saved: send_transfer first")?;
    let mode = if saved.lifetime == "durable-nonce" { Mode::Nonce } else { Mode::Blockhash };
    let l = lifetime(mode)?;
    let dead = l.can_never_land(&saved, &ctx).await.map_err(err)?;
    let mut resigned = false;
    if dead {
        // only the fallback re-signs on its own; a durable-nonce line whose nonce moved under it is an incident, not a retry
        if mode == Mode::Nonce {
            return Err("the nonce moved and this signature is unknown: the saved bytes can never land; NOT re-signing (investigate)".into());
        }
        let (to, lamports, intent, history) = (saved.to, saved.lamports, saved.intent, saved.history.clone());
        saved = sign_intent(l.as_ref(), &ctx, intent, to, lamports, history).await?;
        resigned = true;
        S.with(|s| s.borrow_mut().line = Some(saved.clone()));
    }
    let answers = solsend::send_bytes(&ctx, &saved.tx, preflight.unwrap_or(true)).await;
    saved.sends += 1;
    S.with(|s| s.borrow_mut().line = Some(saved.clone()));
    Ok(report(&saved, resigned, answers, t0))
}

#[derive(CandidType, Deserialize, Clone, Debug)]
pub struct StatusReport {
    pub signature: String,
    pub intent: u64,
    /// each provider's `getSignatureStatuses` (transformed), and the agreed verdict
    pub providers: Vec<String>,
    pub agreed: Option<String>,
    pub finalized: bool,
    pub failed: Option<String>,
    pub durable_nonce_now: Option<String>,
    pub balance_lamports: Option<u64>,
}

async fn status_of(ctx: &Ctx<'_>, sig: &str, intent: u64) -> StatusReport {
    let views = solsend::sig_views(ctx, sig).await;
    let providers = views.iter().map(|a| match a { Ok(v) => v.to_string(), Err(e) => format!("OUTCALL FAILED: {e}") }).collect();
    let ag = solrpc::agree(views);
    let (agreed, finalized, failed) = match &ag {
        Ok(v) => (Some(v.to_string()), v["status"] == "finalized" && v["err"].is_null(), (!v["err"].is_null() && v.get("err").is_some()).then(|| v["err"].to_string())),
        Err(e) => (Some(e.to_string()), false, None),
    };
    let nonce = S.with(|s| s.borrow().nonce_account);
    let durable_nonce_now = match nonce {
        Some(n) => solsend::nonce_state(ctx, &n).await.ok().map(|s| soltx::b58_encode(&s.durable_nonce)),
        None => None,
    };
    let balance_lamports = solsend::balance(ctx, &ctx.me).await.ok();
    StatusReport { signature: sig.into(), intent, providers, agreed, finalized, failed, durable_nonce_now, balance_lamports }
}

/// The line's transaction at `finalized`, through both providers.
#[ic_cdk::update]
async fn status() -> Result<StatusReport, String> {
    guard();
    let c = cfg();
    let me = me().await?;
    let ctx = Ctx { urls: &c.rpc_urls, me, replicated_send: c.replicated_send };
    let saved = S.with(|s| s.borrow().line.clone()).ok_or("nothing saved")?;
    Ok(status_of(&ctx, &saved.signature, saved.intent).await)
}

/// Any signature (the nonce account's creation).
#[ic_cdk::update]
async fn sig_status(sig: String) -> Result<StatusReport, String> {
    guard();
    let c = cfg();
    let me = me().await?;
    let ctx = Ctx { urls: &c.rpc_urls, me, replicated_send: c.replicated_send };
    Ok(status_of(&ctx, &sig, 0).await)
}

#[ic_cdk::update]
fn set_mode(m: Mode) {
    guard();
    S.with(|s| s.borrow_mut().init.as_mut().expect("initialised").mode = m);
}

/// Forget the saved line (the spike's own reset; the real canister never does this).
#[ic_cdk::update]
fn drop_line() {
    guard();
    S.with(|s| s.borrow_mut().line = None);
}

#[derive(CandidType, Deserialize, Clone, Debug)]
pub struct SlotProbe {
    pub bucket: u64,
    pub providers: Vec<String>,
    pub agreed: Option<u64>,
    pub refused: Option<String>,
}

/// The provider-agreement probe: `getSlot(finalized)` at a bucket (1 = raw, 20 = the fallback's rounding).
#[ic_cdk::update]
async fn slot_agreement(bucket: u64) -> SlotProbe {
    guard();
    let c = cfg();
    let call = solrpc::Call { method: "getSlot", params: serde_json::json!([{ "commitment": "finalized" }]), kind: solrpc::Kind::SlotBucket, bucket,
        max_bytes: 1_000, replicated: true };
    let views = solrpc::each(&c.rpc_urls, &call).await;
    let providers = views.iter().map(|a| match a { Ok(v) => v.to_string(), Err(e) => format!("OUTCALL FAILED: {e}") }).collect();
    match solrpc::agree(views) {
        Ok(v) => SlotProbe { bucket, providers, agreed: v["n"].as_u64(), refused: None },
        Err(e) => SlotProbe { bucket, providers, agreed: None, refused: Some(e.to_string()) },
    }
}

#[derive(CandidType, Deserialize, Clone, Debug)]
pub struct Metrics {
    pub cycles: u128,
    pub signs: Vec<SignSample>,
    pub calls: Vec<CallSample>,
}

#[ic_cdk::query]
fn metrics() -> Metrics {
    Metrics {
        cycles: ic_cdk::api::canister_cycle_balance(),
        signs: S.with(|s| s.borrow().signs.clone()),
        calls: solrpc::CALLS.with(|c| c.borrow().clone()),
    }
}

/// The saved bytes, base64 (the run script checks them against the CLI's decoder; nothing secret: a signed public transaction).
#[ic_cdk::query]
fn saved_tx() -> Option<String> {
    use base64::Engine;
    S.with(|s| s.borrow().line.as_ref().map(|l| base64::engine::general_purpose::STANDARD.encode(&l.tx)))
}

ic_cdk::export_candid!();

#[cfg(test)]
mod tests;
