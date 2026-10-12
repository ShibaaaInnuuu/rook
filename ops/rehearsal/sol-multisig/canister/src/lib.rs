//! Spike S3 (docs/v9/canister-only-solana.md §1.1 keys, §6.1 `open_mint`, §15 S3): a THROWAWAY canister that builds ONE legacy
//! Solana transaction needing three signatures from three derivation paths of the same threshold Ed25519 key, signs the message
//! with each path (sequentially or concurrently), and assembles the transaction in header order. No outcalls: the driver passes the
//! blockhash (or the durable nonce value) in, and submits the bytes itself. Local replica only.
//!
//! Keys (design §1.1):
//!   K_main    = [b"sol"]                                  fee payer, nonce authority (the existing solkey.rs path)
//!   K_int[n]  = Multi: [b"sol", b"intent", n as u64 LE]   Flat: [b"sol/intent/<n>"]
//!   K_coll[i] = Multi: [b"sol", b"coll",   i as u32 LE]   Flat: [b"sol/coll/<i>"]
//!
//! Every update is controller-only (each spends cycles). State is heap only.
pub mod soltx;

use base64::Engine;
use candid::{CandidType, Deserialize};
use ic_cdk::management_canister::{
    cost_sign_with_schnorr, schnorr_public_key, sign_with_schnorr, SchnorrAlgorithm, SchnorrKeyId, SchnorrPublicKeyArgs, SignWithSchnorrArgs,
};
use soltx::{AccountMeta, Instruction, Pubkey};
use std::cell::RefCell;

/// `TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA`
pub const TOKEN_PROGRAM: &str = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA";
/// SPL Memo v2 (`MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr`)
pub const MEMO_PROGRAM: &str = "MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr";

#[derive(CandidType, Deserialize, Clone, Debug)]
pub struct Init {
    /// `key_1` on dfx 0.32's local replica
    pub key_name: String,
}

#[derive(Default)]
struct State {
    init: Option<Init>,
    keys: Vec<(Vec<Vec<u8>>, Pubkey)>,
}

thread_local! {
    static S: RefCell<State> = RefCell::new(State::default());
}

#[ic_cdk::init]
fn init(i: Init) {
    S.with(|s| s.borrow_mut().init = Some(i));
}

#[ic_cdk::post_upgrade]
fn post_upgrade(i: Init) {
    init(i)
}

fn guard() {
    if !ic_cdk::api::is_controller(&ic_cdk::api::msg_caller()) {
        ic_cdk::trap("controllers only: every call spends cycles");
    }
}

fn key_id() -> SchnorrKeyId {
    let name = S.with(|s| s.borrow().init.clone()).expect("initialised").key_name;
    SchnorrKeyId { algorithm: SchnorrAlgorithm::Ed25519, name }
}

// ---- paths ------------------------------------------------------------------------------------------------------------------------

#[derive(CandidType, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathStyle {
    /// [b"sol", b"intent", n LE]: one element per level (BIP32-like: each element is one derivation step)
    Multi,
    /// [b"sol/intent/<n>"]: one element holding the whole string (one derivation step)
    Flat,
}

pub fn path_main() -> Vec<Vec<u8>> {
    vec![b"sol".to_vec()]
}

pub fn path_intent(style: PathStyle, n: u64) -> Vec<Vec<u8>> {
    match style {
        PathStyle::Multi => vec![b"sol".to_vec(), b"intent".to_vec(), n.to_le_bytes().to_vec()],
        PathStyle::Flat => vec![format!("sol/intent/{n}").into_bytes()],
    }
}

pub fn path_coll(style: PathStyle, i: u32) -> Vec<Vec<u8>> {
    match style {
        PathStyle::Multi => vec![b"sol".to_vec(), b"coll".to_vec(), i.to_le_bytes().to_vec()],
        PathStyle::Flat => vec![format!("sol/coll/{i}").into_bytes()],
    }
}

// ---- public keys ------------------------------------------------------------------------------------------------------------------

#[derive(CandidType, Deserialize, Clone, Debug)]
pub struct PubkeyReport {
    pub path: Vec<Vec<u8>>,
    pub address: String,
    pub public_key: Vec<u8>,
    pub chain_code: Vec<u8>,
    pub latency_ms: u64,
    /// canister balance before − after (schnorr_public_key attaches nothing; this is the call's own execution)
    pub balance_delta: u128,
    pub canister_id: String,
}

async fn fetch_pubkey(path: Vec<Vec<u8>>) -> Result<PubkeyReport, String> {
    let (t0, b0) = (ic_cdk::api::time(), ic_cdk::api::canister_cycle_balance());
    let r = schnorr_public_key(&SchnorrPublicKeyArgs { canister_id: None, derivation_path: path.clone(), key_id: key_id() })
        .await
        .map_err(|e| format!("schnorr_public_key: {e:?}"))?;
    let latency_ms = (ic_cdk::api::time() - t0) / 1_000_000;
    let balance_delta = b0.saturating_sub(ic_cdk::api::canister_cycle_balance());
    let k: Pubkey = r.public_key.as_slice().try_into().map_err(|_| format!("ed25519 key is 32 bytes, got {}", r.public_key.len()))?;
    S.with(|s| {
        let mut s = s.borrow_mut();
        if !s.keys.iter().any(|(p, _)| p == &path) {
            s.keys.push((path.clone(), k));
        }
    });
    Ok(PubkeyReport {
        path,
        address: soltx::b58_encode(&k),
        public_key: r.public_key,
        chain_code: r.chain_code,
        latency_ms,
        balance_delta,
        canister_id: ic_cdk::api::canister_self().to_text(),
    })
}

async fn key_for(path: &[Vec<u8>]) -> Result<Pubkey, String> {
    if let Some(k) = S.with(|s| s.borrow().keys.iter().find(|(p, _)| p == path).map(|(_, k)| *k)) {
        return Ok(k);
    }
    let r = fetch_pubkey(path.to_vec()).await?;
    Ok(r.public_key.as_slice().try_into().unwrap())
}

/// The threshold Ed25519 public key (and chain code) at any derivation path; `vec {}` = the canister-level key.
#[ic_cdk::update]
async fn pubkey(path: Vec<Vec<u8>>) -> Result<PubkeyReport, String> {
    guard();
    fetch_pubkey(path).await
}

/// The design's named paths, as a convenience for the driver.
#[ic_cdk::query]
fn paths(style: PathStyle, n: u64, i: u32) -> Vec<(String, Vec<Vec<u8>>)> {
    vec![("K_main".into(), path_main()), ("K_int".into(), path_intent(style, n)), ("K_coll".into(), path_coll(style, i))]
}

// ---- instructions not in soltx ----------------------------------------------------------------------------------------------------

/// SPL Token `MintTo { amount }` (index 7): mint (w), destination token account (w), mint authority (signer).
pub fn mint_to(mint: Pubkey, dest: Pubkey, authority: Pubkey, amount: u64) -> Instruction {
    let mut data = vec![7u8];
    data.extend(amount.to_le_bytes());
    Instruction {
        program: soltx::pubkey(TOKEN_PROGRAM).unwrap(),
        accounts: vec![
            AccountMeta { key: mint, signer: false, writable: true },
            AccountMeta { key: dest, signer: false, writable: true },
            AccountMeta { key: authority, signer: true, writable: false },
        ],
        data,
    }
}

/// SPL Memo v2: UTF-8 data; every listed account must be a signer.
pub fn memo(text: &str, signers: &[Pubkey]) -> Instruction {
    Instruction {
        program: soltx::pubkey(MEMO_PROGRAM).unwrap(),
        accounts: signers.iter().map(|k| AccountMeta { key: *k, signer: true, writable: false }).collect(),
        data: text.as_bytes().to_vec(),
    }
}

// ---- the multi-signer transaction -------------------------------------------------------------------------------------------------

#[derive(CandidType, Deserialize, Clone, Debug)]
pub struct MultiArgs {
    pub style: PathStyle,
    pub intent: u64,
    pub coll: u32,
    /// base58: a recent blockhash, or the durable nonce value when `nonce_account` is set
    pub blockhash: String,
    /// base58: when set, AdvanceNonceAccount(nonce, K_main) is the first instruction
    pub nonce_account: Option<String>,
    /// lamports K_int → K_coll
    pub lamports: u64,
    /// base58 (mint, destination token account): when set, MintTo(mint, dest, K_coll, mint_amount); else a K_coll → K_main transfer
    pub mint: Option<(String, String)>,
    pub mint_amount: u64,
    pub memo: String,
    /// list K_int as a Memo signer (Memo v2 requires every listed account to sign; it adds no signature: K_int signs anyway)
    pub memo_signer: bool,
    pub parallel: bool,
}

#[derive(CandidType, Deserialize, Clone, Debug)]
pub struct SignerOut {
    pub role: String,
    pub path: Vec<Vec<u8>>,
    pub address: String,
    pub signature: Vec<u8>,
    /// ic0.cost_sign_with_schnorr: what ic-cdk attached
    pub cost_api: u128,
    /// call start → reply (sequential: this signature alone; parallel: all started in the same message, each one's own reply)
    pub latency_ms: u64,
    /// sequential only: the balance delta around this one call
    pub balance_delta: Option<u128>,
}

#[derive(CandidType, Deserialize, Clone, Debug)]
pub struct MultiReport {
    pub tx_base64: String,
    pub message_base64: String,
    pub tx_bytes: u32,
    pub header: (u8, u8, u8),
    pub signers: Vec<SignerOut>,
    pub parallel: bool,
    /// first sign call → last reply
    pub sign_total_ms: u64,
    /// balance before the first sign − after the last (attached cycles + execution, all three)
    pub sign_balance_delta: u128,
    /// instructions spent compiling the message (performance counter)
    pub compile_instructions: u64,
}

fn sign_args(path: &[Vec<u8>], message: &[u8]) -> SignWithSchnorrArgs {
    SignWithSchnorrArgs { message: message.to_vec(), derivation_path: path.to_vec(), key_id: key_id(), aux: None }
}

/// One timed signature: (signature, latency ms, start time).
async fn timed_sign(arg: SignWithSchnorrArgs) -> Result<([u8; 64], u64), String> {
    let t0 = ic_cdk::api::time();
    let r = sign_with_schnorr(&arg).await.map_err(|e| format!("sign_with_schnorr: {e:?}"))?;
    let ms = (ic_cdk::api::time() - t0) / 1_000_000;
    let sig: [u8; 64] = r.signature.as_slice().try_into().map_err(|_| format!("ed25519 signature is 64 bytes, got {}", r.signature.len()))?;
    Ok((sig, ms))
}

#[ic_cdk::update]
async fn multi_sign(a: MultiArgs) -> Result<MultiReport, String> {
    guard();
    let p_main = path_main();
    let p_int = path_intent(a.style, a.intent);
    let p_coll = path_coll(a.style, a.coll);
    let k_main = key_for(&p_main).await?;
    let k_int = key_for(&p_int).await?;
    let k_coll = key_for(&p_coll).await?;
    let blockhash = soltx::pubkey(&a.blockhash)?;

    let pc0 = ic_cdk::api::performance_counter(0);
    let mut ixs = Vec::new();
    if let Some(n) = &a.nonce_account {
        ixs.push(soltx::advance_nonce(soltx::pubkey(n)?, k_main));
    }
    ixs.push(soltx::transfer(k_int, k_coll, a.lamports));
    match &a.mint {
        Some((mint, dest)) => ixs.push(mint_to(soltx::pubkey(mint)?, soltx::pubkey(dest)?, k_coll, a.mint_amount)),
        None => ixs.push(soltx::transfer(k_coll, k_main, a.mint_amount)),
    }
    ixs.push(memo(&a.memo, if a.memo_signer { std::slice::from_ref(&k_int) } else { &[] }));
    let msg = soltx::compile(&ixs, k_main, blockhash);
    soltx::check_size(soltx::wire_size(&msg))?;
    let compile_instructions = ic_cdk::api::performance_counter(0) - pc0;

    // the rule that replaces solkey.rs's "only signer": every signer of the message is one of THIS canister's keys, by path
    let roles = [("K_main", &p_main, k_main), ("K_int", &p_int, k_int), ("K_coll", &p_coll, k_coll)];
    let mut ordered = Vec::new();
    for s in &msg.signers {
        let r = roles.iter().find(|r| &r.2 == s).ok_or_else(|| format!("signer {} is not a canister key", soltx::b58_encode(s)))?;
        ordered.push(*r);
    }
    if ordered.len() != 3 || ordered[0].0 != "K_main" {
        return Err(format!("expected 3 signers with K_main first, got {:?}", ordered.iter().map(|r| r.0).collect::<Vec<_>>()));
    }

    let args: Vec<SignWithSchnorrArgs> = ordered.iter().map(|r| sign_args(r.1, &msg.bytes)).collect();
    let costs: Vec<u128> = args.iter().map(|x| cost_sign_with_schnorr(x).map_err(|e| format!("cost: {e:?}"))).collect::<Result<_, _>>()?;
    let (t0, b0) = (ic_cdk::api::time(), ic_cdk::api::canister_cycle_balance());
    let mut results: Vec<([u8; 64], u64, Option<u128>)> = Vec::new();
    if a.parallel {
        let rs = futures::future::join_all(args.into_iter().map(timed_sign)).await;
        for r in rs {
            let (s, ms) = r?;
            results.push((s, ms, None));
        }
    } else {
        for x in args {
            let b = ic_cdk::api::canister_cycle_balance();
            let (s, ms) = timed_sign(x).await?;
            results.push((s, ms, Some(b.saturating_sub(ic_cdk::api::canister_cycle_balance()))));
        }
    }
    let sign_total_ms = (ic_cdk::api::time() - t0) / 1_000_000;
    let sign_balance_delta = b0.saturating_sub(ic_cdk::api::canister_cycle_balance());

    let sigs: Vec<[u8; 64]> = results.iter().map(|r| r.0).collect();
    let tx = soltx::assemble(&msg, &sigs)?;
    let b64 = base64::engine::general_purpose::STANDARD;
    Ok(MultiReport {
        tx_base64: b64.encode(&tx),
        message_base64: b64.encode(&msg.bytes),
        tx_bytes: tx.len() as u32,
        header: (msg.bytes[0], msg.bytes[1], msg.bytes[2]),
        signers: ordered
            .iter()
            .zip(results.iter())
            .zip(costs.iter())
            .map(|((r, res), c)| SignerOut {
                role: r.0.into(),
                path: r.1.clone(),
                address: soltx::b58_encode(&r.2),
                signature: res.0.to_vec(),
                cost_api: *c,
                latency_ms: res.1,
                balance_delta: res.2,
            })
            .collect(),
        parallel: a.parallel,
        sign_total_ms,
        sign_balance_delta,
        compile_instructions,
    })
}

/// N concurrent signatures of one message on N intent paths (the concurrency probe; §5 of the task).
#[ic_cdk::update]
async fn burst(n: u32, style: PathStyle) -> Result<(u32, u32, u64, Vec<String>), String> {
    guard();
    let t0 = ic_cdk::api::time();
    let args: Vec<_> = (0..n).map(|k| sign_args(&path_intent(style, 10_000 + k as u64), b"rook s3 burst")).collect();
    let rs = futures::future::join_all(args.into_iter().map(timed_sign)).await;
    let ok = rs.iter().filter(|r| r.is_ok()).count() as u32;
    let mut errs: Vec<String> = rs.into_iter().filter_map(|r| r.err()).collect();
    errs.dedup();
    errs.truncate(5);
    Ok((ok, n - ok, (ic_cdk::api::time() - t0) / 1_000_000, errs))
}

#[ic_cdk::query]
fn balance() -> u128 {
    ic_cdk::api::canister_cycle_balance()
}

ic_cdk::export_candid!();

#[cfg(test)]
mod tests {
    /// `cargo test` writes `sol_multisig.did` from the code.
    #[test]
    fn did() {
        std::fs::write(concat!(env!("CARGO_MANIFEST_DIR"), "/sol_multisig.did"), crate::__export_service()).unwrap();
    }

    /// The compiler orders the three signers payer first, then the writable signers by key; header (3, 0, n).
    #[test]
    fn three_signers() {
        let (main, int, coll) = ([9u8; 32], [1u8; 32], [5u8; 32]);
        let ixs = vec![
            crate::soltx::advance_nonce([7u8; 32], main),
            crate::soltx::transfer(int, coll, 1),
            crate::mint_to([3u8; 32], [4u8; 32], coll, 1),
            crate::memo("x", &[int]),
        ];
        let m = crate::soltx::compile(&ixs, main, [0u8; 32]);
        assert_eq!(m.signers, vec![main, int, coll]);
        assert_eq!(m.bytes[0], 3);
        assert_eq!(m.bytes[1], 0);
    }
}
