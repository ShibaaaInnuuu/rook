//! The Solana send line (docs/v9/solana-migration.md §2.4, spike Q3 §7): "sign once, save the bytes, rebroadcast until they land,
//! at most one lands" — the invariant `frogflow::send` and `ethflow::HomeTx` keep on the EVM home — with the transaction's
//! LIFETIME behind one trait, so the mechanism can change when durable nonces go:
//!
//! - [`DurableNonce`] (the default): `AdvanceNonceAccount` first, the nonce account's stored value as `recent_blockhash`. The
//!   saved bytes stay valid until they land or the nonce is advanced by something else; they never expire with time. One nonce
//!   account per line, the canister its only authority, so a nonce that moves under a pending transaction is a bug or a
//!   compromised key: an [`Step::Incident`], never an automatic re-sign.
//! - [`RecentBlockhash`] (the fallback, selectable by `Home::Solana.blockhash_fallback`): a finalized slot rounded down to 20 →
//!   `getBlock` → its blockhash, valid until block height `blockHeight + 150`. Once expiry is PROVEN (the finalized height past it,
//!   then every signature of the intent still unknown) the SAME intent is re-signed on a fresh blockhash; the old bytes can no
//!   longer land, so at most one does.
//!
//! The rules, each from the spike:
//! - the [`Saved`] record is written BEFORE the first send ([`prepare`] signs and saves and never sends; [`tick`] sends);
//! - one transaction in flight per line (= per nonce account): [`prepare`] refuses until the last one is finalized (Q3 §4.5);
//! - what landed is read ONLY from `getSignatureStatuses` (agreed, with history), never from a send's answer (Q3 §4.2);
//! - "can never land" reads the lifetime fact FIRST (the nonce at finalized / the finalized height) and the signatures' status
//!   SECOND (Q3 §4.3): the other order races a landing between the two reads into a wrong "expired" and a second valid signature;
//! - any [`Refused`] (a disagreement, a failed outcall) changes nothing: the next tick retries.
use candid::CandidType;
use serde::Deserialize;

use crate::solkey::{sign_message, KeyPath, SolSigner};
use crate::solrpc::{self, Level, Refused, SolRpc};
use crate::soltx::{self, Hash, Instruction, Pubkey};

/// `MAX_PROCESSING_AGE`: a blockhash is valid for 150 blocks after its own.
pub const BLOCKHASH_VALID_BLOCKS: u64 = 150;
/// Finished intents kept on a line for `sol_seen` and the operator's view.
pub const DONE_KEPT: usize = 32;

#[derive(CandidType, Deserialize, serde::Serialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum LifetimeKind {
    DurableNonce,
    RecentBlockhash,
}

/// One line's saved transaction: everything needed to resend it, to prove it can never land, and (fallback only) to re-sign the
/// same intent.
#[derive(CandidType, Deserialize, serde::Serialize, Clone, Debug, PartialEq, Eq)]
pub struct Saved {
    /// The caller's id for what this transaction does (a re-sign keeps it).
    pub intent: u64,
    /// A label for the log and the operator (`"post_floor listing 3"`).
    pub what: String,
    pub lifetime: LifetimeKind,
    /// The intent's instructions WITHOUT the lifetime's prefix (the nonce advance), for a fallback re-sign.
    pub instructions: Vec<Instruction>,
    pub recent_blockhash: Hash,
    /// `RecentBlockhash` only: the last block height the bytes can land at.
    pub last_valid_height: Option<u64>,
    /// The signed wire transaction.
    pub tx: Vec<u8>,
    /// Its signature (base58), the transaction's id.
    pub signature: String,
    /// Every signature this intent was ever signed under (more than one only after a proven expiry under the fallback).
    pub history: Vec<String>,
    /// Sends so far (0: saved, never sent).
    pub sends: u32,
    /// Audit A4-01: the intent is being GIVEN UP (its first send's preflight was refused by every provider for its instruction). Never
    /// resent as itself again; under the durable nonce its nonce is burned ([`Saved::burn`]), under the fallback it waits for its proven
    /// expiry. Only then is it reported abandoned; if a signature of it lands meanwhile, that landing is its outcome.
    #[serde(default)]
    pub giving_up: Option<String>,
    /// Audit A4-01, durable nonce: the signature of the `AdvanceNonceAccount`-only transaction on the SAME nonce that gives the intent up
    /// (in `history` too; `tx` holds its bytes). At most one of it and the intent's own bytes can land.
    #[serde(default)]
    pub burn: Option<String>,
    /// Audit A4-03, fallback: the intent's signatures proven expired and unknown (they can never land): kept for `sol_seen` and `Done`,
    /// no longer asked for.
    #[serde(default)]
    pub expired: Option<Vec<String>>,
    /// W3 (canister-only design §1.1): the canister keys besides `K_main` the intent's message is signed by (`K_coll`, `K_int`, …), for a
    /// fallback re-sign of the same intent. `None`: `K_main` alone (every intent saved before multi-key signing).
    #[serde(default)]
    pub keys: Option<Vec<KeyPath>>,
    /// The address lookup tables the intent's v0 message loads keys from, as the canister read them (2026-10-12: a Magic Eden M2 pNFT
    /// purchase, `solnftflow::table`), for a fallback re-sign of the same intent. `None`: a legacy message.
    #[serde(default)]
    pub tables: Option<Vec<soltx::LookupTable>>,
}

/// An intent that reached `finalized`.
#[derive(CandidType, Deserialize, serde::Serialize, Clone, Debug, PartialEq, Eq)]
pub struct Done {
    pub intent: u64,
    pub what: String,
    pub signature: String,
    /// Every signature the intent was signed under (the landed one among them): all of them stay in `sol_seen`.
    pub history: Vec<String>,
    /// The transaction's error (`None`: it succeeded). A failed transaction still paid its fee and, nonced, advanced its nonce;
    /// the intent did NOT happen and is the caller's to retry under a new intent.
    pub err: Option<String>,
    /// Audit A4-01: `Some(true)` when the intent is PROVEN never to land (its nonce burn finalized, its expiry proven while given up, or
    /// the operator's clear after an incident): `signature` is the burn's (or empty) and the intent did not happen.
    #[serde(default)]
    pub abandoned: Option<bool>,
}

/// One send line: at most one transaction in flight.
#[derive(CandidType, Deserialize, serde::Serialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct Line {
    /// The line's durable-nonce account (seed-derived from the canister's key; [`nonce_account_of`]).
    pub nonce_account: Option<Pubkey>,
    pub saved: Option<Saved>,
    /// Set when the nonce moved under a pending transaction that is not ours on chain: no re-sign until an operator clears it.
    pub incident: Option<String>,
    pub done: Vec<Done>,
    /// The first send's preflight was refused by EVERY provider (insufficient funds, a simulation that fails). The answers are not
    /// agreed (a non-replicated outcall) and a refusing provider may keep the bytes, so this decides nothing about landing (audit A4-01):
    /// it is a line error the operator sees. Insufficient funds: the bytes are resent with preflight each tick (cleared once a provider
    /// takes them); any other refusal: the intent is given up ([`Saved::giving_up`]).
    #[serde(default)]
    pub error: Option<String>,
}

/// What a lifetime puts in a new transaction.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Anchor {
    pub recent_blockhash: Hash,
    pub prefix: Vec<Instruction>,
    pub last_valid_height: Option<u64>,
}

/// The one trait the mechanism hides behind (Q3 §7).
#[allow(async_fn_in_trait)]
pub trait Lifetime {
    fn kind(&self) -> LifetimeKind;
    /// The `recent_blockhash` field and the prefix instructions for a transaction signed now by `me`.
    async fn anchor<P: SolRpc>(&self, me: &Pubkey, ps: &mut [P]) -> Result<Anchor, Refused>;
    /// PROVEN that the saved bytes can never land: the lifetime fact first, every signature's status second. `Ok(false)` otherwise.
    async fn can_never_land<P: SolRpc>(&self, saved: &Saved, ps: &mut [P]) -> Result<bool, Refused>;
}

pub struct DurableNonce {
    pub account: Pubkey,
}

pub struct RecentBlockhash;

/// Whether no signature of the intent is known to the providers (agreed), read SECOND, bound to a finalized slot read after the lifetime
/// fact (audit A4-04: a provider answering from a context older than that slot is not believed; "not proven" then, never "unknown").
async fn none_known<P: SolRpc>(saved: &Saved, ps: &mut [P]) -> Result<bool, Refused> {
    let min = solrpc::finalized_slot(ps, solrpc::SLOT_BUCKET).await?;
    Ok(match solrpc::statuses_at(ps, &saved.history, min).await? {
        Some(st) => st.iter().all(|s| s.is_none()),
        None => false,
    })
}

impl Lifetime for DurableNonce {
    fn kind(&self) -> LifetimeKind {
        LifetimeKind::DurableNonce
    }
    async fn anchor<P: SolRpc>(&self, me: &Pubkey, ps: &mut [P]) -> Result<Anchor, Refused> {
        let n = solrpc::nonce(ps, &self.account).await?;
        if n.authority != *me {
            return Err(Refused::Failed("the nonce account's authority is not this canister".into()));
        }
        Ok(Anchor { recent_blockhash: n.durable_nonce, prefix: vec![soltx::advance_nonce(self.account, *me)], last_valid_height: None })
    }
    async fn can_never_land<P: SolRpc>(&self, saved: &Saved, ps: &mut [P]) -> Result<bool, Refused> {
        let n = solrpc::nonce(ps, &self.account).await?; // FIRST
        if n.durable_nonce == saved.recent_blockhash {
            return Ok(false); // the nonce has not moved: the saved bytes are still good
        }
        none_known(saved, ps).await // SECOND
    }
}

impl Lifetime for RecentBlockhash {
    fn kind(&self) -> LifetimeKind {
        LifetimeKind::RecentBlockhash
    }
    async fn anchor<P: SolRpc>(&self, _me: &Pubkey, ps: &mut [P]) -> Result<Anchor, Refused> {
        let (hash, height) = solrpc::rounded_blockhash(ps).await?;
        Ok(Anchor { recent_blockhash: hash, prefix: vec![], last_valid_height: Some(height + BLOCKHASH_VALID_BLOCKS) })
    }
    async fn can_never_land<P: SolRpc>(&self, saved: &Saved, ps: &mut [P]) -> Result<bool, Refused> {
        let Some(last) = saved.last_valid_height else { return Ok(false) };
        let h = solrpc::finalized_height(ps).await?; // FIRST (rounded DOWN: conservative)
        if h <= last {
            return Ok(false);
        }
        none_known(saved, ps).await // SECOND
    }
}

/// Either lifetime, by kind (async trait methods are not object-safe).
pub enum AnyLifetime {
    Nonce(DurableNonce),
    Blockhash(RecentBlockhash),
}

impl AnyLifetime {
    pub fn of(kind: LifetimeKind, nonce_account: Option<Pubkey>) -> Result<AnyLifetime, String> {
        match kind {
            LifetimeKind::DurableNonce => {
                Ok(AnyLifetime::Nonce(DurableNonce { account: nonce_account.ok_or("a durable-nonce line without a nonce account")? }))
            }
            LifetimeKind::RecentBlockhash => Ok(AnyLifetime::Blockhash(RecentBlockhash)),
        }
    }
}

impl Lifetime for AnyLifetime {
    fn kind(&self) -> LifetimeKind {
        match self {
            AnyLifetime::Nonce(l) => l.kind(),
            AnyLifetime::Blockhash(l) => l.kind(),
        }
    }
    async fn anchor<P: SolRpc>(&self, me: &Pubkey, ps: &mut [P]) -> Result<Anchor, Refused> {
        match self {
            AnyLifetime::Nonce(l) => l.anchor(me, ps).await,
            AnyLifetime::Blockhash(l) => l.anchor(me, ps).await,
        }
    }
    async fn can_never_land<P: SolRpc>(&self, saved: &Saved, ps: &mut [P]) -> Result<bool, Refused> {
        match self {
            AnyLifetime::Nonce(l) => l.can_never_land(saved, ps).await,
            AnyLifetime::Blockhash(l) => l.can_never_land(saved, ps).await,
        }
    }
}

/// The line's durable-nonce account: `create_with_seed(me, seed, System Program)` (Q3: created by the canister alone, one signature).
pub fn nonce_account_of(me: &Pubkey, seed: &str) -> Result<Pubkey, String> {
    soltx::address_with_seed(me, seed, &soltx::SYSTEM_PROGRAM)
}

/// The instructions that create and initialize a line's nonce account (sent under [`RecentBlockhash`]: the nonce does not exist yet).
pub fn open_nonce_ixs(me: &Pubkey, seed: &str, rent_lamports: u64) -> Result<Vec<Instruction>, String> {
    let n = nonce_account_of(me, seed)?;
    Ok(vec![
        soltx::create_with_seed(*me, n, *me, seed, rent_lamports, soltx::NONCE_ACCOUNT_SPACE, soltx::SYSTEM_PROGRAM),
        soltx::initialize_nonce(n, *me),
    ])
}

/// Audit A4-02: the instructions that open the line's nonce account IN PLACE when its address already holds a System-owned account with
/// no data (anyone can transfer lamports to it; nobody but `me` can allocate it): `AllocateWithSeed(80, System)`, a top-up `Transfer`
/// to the rent-exempt amount when it holds less, then `InitializeNonceAccount`. Signed by `me` alone, as the creation is.
pub fn open_squatted_nonce_ixs(me: &Pubkey, seed: &str, rent_lamports: u64, held: u64) -> Result<Vec<Instruction>, String> {
    let n = nonce_account_of(me, seed)?;
    let mut v = vec![soltx::allocate_with_seed(n, *me, seed, soltx::NONCE_ACCOUNT_SPACE, soltx::SYSTEM_PROGRAM)];
    if held < rent_lamports {
        v.push(soltx::transfer(*me, n, rent_lamports - held));
    }
    v.push(soltx::initialize_nonce(n, *me));
    Ok(v)
}

/// The fee of a one-signature transaction (no priority fee is ever set): what the Relay deposit key is funded with on top of the deposit.
pub const SIGNATURE_FEE_LAMPORTS: u64 = 5_000;

/// Who pays for (and alone signs) `instructions`: the dedicated Relay deposit key when it is their only signer (audit A5-03), else `me`.
fn payer_of<S: SolSigner>(signer: &S, instructions: &[Instruction]) -> Pubkey {
    let me = signer.me();
    let signers: Vec<Pubkey> = instructions.iter().flat_map(|i| i.accounts.iter().filter(|a| a.signer).map(|a| a.key)).collect();
    match signer.relay() {
        Some(r) if r != me && !signers.is_empty() && signers.iter().all(|k| *k == r) => r,
        _ => me,
    }
}

/// A simulation every provider refuses, in the classes the line's error names (`insufficient_funds` for a fee payer that cannot pay:
/// the monitor pages it SEV1; anything else is the instruction failing on the state it was built from).
pub fn simulation_class(err: &str) -> &'static str {
    if err.contains("InsufficientFunds") || err.contains("AccountNotFound") || err.contains("insufficient") {
        "insufficient_funds"
    } else {
        "error"
    }
}

/// The prefix of a refusal by simulation ([`sign_intent`]): nothing was signed.
pub const SIMULATION_REFUSED: &str = "simulation refused by every provider";

/// The message of `all` (the lifetime's prefix and the intent): legacy without `tables`; with them a v0 message loading every key it can
/// from them (`soltx::compile_v0_parts`), refused unless it decompiles, against the tables as read, to exactly `all`'s programs, keys
/// and data (the tables decide nothing the canister did not intend).
pub fn compile_with(all: &[Instruction], me: Pubkey, blockhash: Hash, tables: &[soltx::LookupTable]) -> Result<soltx::Message, String> {
    if tables.is_empty() {
        return Ok(soltx::compile_parts(all, me, blockhash)?.message());
    }
    let p = soltx::compile_v0_parts(all, me, blockhash, tables)?;
    let (w, r) = soltx::loaded_keys(&p, tables)?;
    let back = p.decompile(&w, &r)?;
    let same = back.len() == all.len() && back.iter().zip(all).all(|(x, y)| x.program == y.program && x.data == y.data
        && x.accounts.iter().map(|m| m.key).eq(y.accounts.iter().map(|m| m.key)));
    if !same {
        return Err("refused: the v0 message does not decompile to the intent's instructions".into());
    }
    Ok(p.message())
}

/// Anchor, compile (legacy, or v0 with `tables`: [`compile_with`]), check the signers and the size, SIMULATE (phase B2: the unsigned
/// bytes, `sigVerify` off, at finalized: a step that would fail on chain costs no threshold signature), sign with every key at once (W3:
/// `K_main` and `keys`, each signer one of ours, in the compiled order): the bytes of one intent under one lifetime. No send.
async fn sign_intent<P: SolRpc, S: SolSigner>(l: &impl Lifetime, ps: &mut [P], signer: &mut S, instructions: &[Instruction], keys: &[KeyPath],
    tables: &[soltx::LookupTable]) -> Result<(Anchor, Vec<u8>, String), String> {
    let me = payer_of(signer, instructions);
    if me != signer.me() && l.kind() != LifetimeKind::RecentBlockhash {
        return Err("refused: a transaction of the Relay deposit key is signed on a recent blockhash only (the nonce's authority is not it)".into());
    }
    let a = l.anchor(&me, ps).await?;
    let mut all = a.prefix.clone();
    all.extend_from_slice(instructions);
    let msg = compile_with(&all, me, a.recent_blockhash, tables)?;
    // the 1,232-byte check and the signer check (every signer a key of ours named for it; the Relay key alone), BEFORE anything is
    // simulated or signed
    if keys.is_empty() {
        crate::solkey::signable(&msg, &me)?;
    }
    let held = crate::solkey::held_keys(signer, keys).await?;
    let paths = crate::solkey::signer_paths(&msg, &held)?;
    let unsigned = soltx::assemble(&msg, &vec![[0u8; 64]; msg.signers.len()])?;
    if let Some(e) = solrpc::simulate(ps, &unsigned).await.map_err(|e| format!("not simulated, nothing signed: {e}"))? {
        return Err(format!("{SIMULATION_REFUSED}: {} {e}; nothing was signed", simulation_class(&e)));
    }
    let (tx, sigs) = crate::solkey::sign_paths_checked(signer, &msg, &paths).await?;
    Ok((a, tx, soltx::b58_encode(&sigs[0])))
}

/// Sign an intent and SAVE it on the line (never send: [`tick`] sends). Refused while the line has a transaction in flight or an
/// open incident (one in flight per nonce account: the next intent reads the nonce at finalized, and a nonce read before the last
/// transaction finalized may already be consumed). `K_main` alone signs; [`prepare_keys`] for an intent that moves a derived key's money.
pub async fn prepare<P: SolRpc, S: SolSigner>(line: &mut Line, kind: LifetimeKind, intent: u64, what: &str, instructions: Vec<Instruction>,
    ps: &mut [P], signer: &mut S) -> Result<(), String> {
    prepare_keys(line, kind, intent, what, instructions, vec![], ps, signer).await
}

/// [`prepare`] for an intent signed by `K_main` (fee payer, nonce authority) AND the canister keys in `keys` (W3: the owner of the money
/// it moves: `K_coll[i]` for a mint or a module transfer, `K_int[n]` for an intent's deposit, …). Every signer of the compiled message
/// must be `K_main` or one of `keys`, or nothing is signed. A full signing queue ([`crate::solkey::QUEUE_FULL`]) changes nothing: retry
/// at the next tick.
#[allow(clippy::too_many_arguments)]
pub async fn prepare_keys<P: SolRpc, S: SolSigner>(line: &mut Line, kind: LifetimeKind, intent: u64, what: &str, instructions: Vec<Instruction>,
    keys: Vec<KeyPath>, ps: &mut [P], signer: &mut S) -> Result<(), String> {
    prepare_tables(line, kind, intent, what, instructions, keys, vec![], ps, signer).await
}

/// [`prepare_keys`] for an intent sent as a v0 message loading keys from `tables` (each as the canister read it at finalized, owned by
/// `K_main`: [`compile_with`]); no tables: a legacy message, as [`prepare_keys`].
#[allow(clippy::too_many_arguments)]
pub async fn prepare_tables<P: SolRpc, S: SolSigner>(line: &mut Line, kind: LifetimeKind, intent: u64, what: &str, instructions: Vec<Instruction>,
    keys: Vec<KeyPath>, tables: Vec<soltx::LookupTable>, ps: &mut [P], signer: &mut S) -> Result<(), String> {
    if let Some(i) = &line.incident {
        return Err(format!("refused: the line has an open incident ({i}); an operator must clear it"));
    }
    if let Some(s) = &line.saved {
        return Err(format!("refused: intent {} ({}) is still in flight as {}; one transaction per line", s.intent, s.what, s.signature));
    }
    // audit A5-03: the dedicated Relay deposit key pays for and signs its own transactions, on a recent blockhash (bounded life)
    let kind = if payer_of(signer, &instructions) != signer.me() { LifetimeKind::RecentBlockhash } else { kind };
    let l = AnyLifetime::of(kind, line.nonce_account)?;
    let (a, tx, signature) = match sign_intent(&l, ps, signer, &instructions, &keys, &tables).await {
        Ok(x) => x,
        Err(e) => {
            // a simulation every provider refused is the line's error (the operator sees it; the monitor pages a fee payer that
            // cannot pay), and nothing was signed; any other failure changes nothing
            if e.starts_with(SIMULATION_REFUSED) {
                line.error = Some(format!("intent {intent} ({what}): {e}"));
            }
            return Err(e);
        }
    };
    line.error = None;
    line.saved = Some(Saved { intent, what: what.to_string(), lifetime: kind, instructions, recent_blockhash: a.recent_blockhash,
        last_valid_height: a.last_valid_height, tx, signature: signature.clone(), history: vec![signature], sends: 0, giving_up: None,
        burn: None, expired: None, keys: (!keys.is_empty()).then_some(keys), tables: (!tables.is_empty()).then_some(tables) });
    Ok(())
}

/// What one tick of a line did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Step {
    /// Nothing in flight.
    Idle,
    /// A signature of the intent is `finalized` (with its error, if it failed on chain): the intent is settled, the line free.
    Landed(Done),
    /// The saved bytes were (re)sent; nothing proven yet.
    Pending { intent: u64, sends: u32 },
    /// Fallback only: expiry proven, the same intent re-signed and SAVED (sent on the next tick).
    Resigned { intent: u64, signature: String },
    /// Durable nonce moved under a pending transaction that is not ours on chain: no re-sign, an operator must look.
    Incident(String),
    /// The first send's preflight was refused by every provider (the class and the error). Nothing is decided on it (audit A4-01): the
    /// bytes count as sent, `Line.error` names the refusal, and an intent refused for its instruction is given up from the next tick.
    Preflight(String),
    /// Audit A4-01, durable nonce: the given-up intent's nonce burn (`AdvanceNonceAccount` alone, the same nonce) is signed and saved.
    Burning { intent: u64, signature: String },
    /// Audit A4-01: the given-up intent is PROVEN never to land (its burn finalized, or its expiry proven under the fallback): the line is
    /// free and the intent did not happen (`Done.abandoned`).
    Abandoned(Done),
}

/// The send answers' classes a preflight REFUSES with (the transaction was not forwarded): what `solrpc::normalise` names them.
pub const PREFLIGHT_REFUSALS: [&str; 4] = ["insufficient_funds", "account_not_found", "signature_failure", "error"];

/// Every provider answered, and every answer is a preflight refusal: `Some(why)`. One provider accepting (or failing to answer) is
/// not a refusal. Even `Some` proves nothing (audit A4-01): the answers are not agreed and a refusing provider may keep the bytes; it
/// is an error for the operator and the reason to give an intent up, never "it did not happen".
pub fn preflight_refused(answers: &[Result<serde_json::Value, String>]) -> Option<String> {
    if answers.is_empty() {
        return None;
    }
    let mut why = Vec::new();
    for a in answers {
        let v = a.as_ref().ok()?;
        let class = v["class"].as_str()?;
        if !PREFLIGHT_REFUSALS.contains(&class) {
            return None;
        }
        why.push(if v["err"].is_null() { class.to_string() } else { format!("{class} {}", v["err"]) });
    }
    why.dedup();
    Some(why.join(" / "))
}

/// Every signature the intent was ever signed under, the expired ones included.
fn all_signatures(s: &Saved) -> Vec<String> {
    s.expired.iter().flatten().chain(s.history.iter()).cloned().collect()
}

fn settle(line: &mut Line, s: &Saved, signature: &str, err: Option<String>, abandoned: bool) -> Done {
    let d = Done { intent: s.intent, what: s.what.clone(), signature: signature.to_string(), history: all_signatures(s), err,
        abandoned: abandoned.then_some(true) };
    line.saved = None;
    line.incident = None;
    line.error = None;
    line.done.push(d.clone());
    if line.done.len() > DONE_KEPT {
        let extra = line.done.len() - DONE_KEPT;
        line.done.drain(..extra);
    }
    d
}

/// Expired signatures kept per intent, at most (they can never land; the oldest go first).
pub const EXPIRED_KEPT: usize = 64;

/// One tick of one line (no state is held across an await that a later read does not re-check):
///   never sent → send the saved bytes (preflight on), counted as sent WHATEVER the answers (audit A4-01: they are not agreed and prove
///                nothing); every provider's preflight refusing them → Preflight (`Line.error`); refused for the instruction itself
///                (not for want of funds) → the intent is given up (`Saved.giving_up`)
///   a signature of the intent `finalized` → Landed (ok or failed), or Abandoned when it is the given-up intent's nonce burn
///   else `can_never_land` (lifetime fact FIRST, statuses SECOND, bound to a slot read after it) → DurableNonce: Incident
///                                                               → RecentBlockhash: given up → Abandoned; else re-sign the SAME intent
///   else given up under the durable nonce, no burn yet → sign and save the burn (`AdvanceNonceAccount` alone, the SAME nonce), Burning
///   else given up → wait (never resent as itself); else resend the saved bytes, Pending
///   any `Refused` → nothing changes; retry next tick
pub async fn tick<P: SolRpc, S: SolSigner>(line: &mut Line, ps: &mut [P], signer: &mut S) -> Result<Step, Refused> {
    let Some(s) = line.saved.clone() else { return Ok(Step::Idle) };
    if s.sends == 0 && line.incident.is_none() {
        let answers = solrpc::send(ps, &s.tx, true).await;
        if let Some(x) = line.saved.as_mut() {
            x.sends = x.sends.saturating_add(1);
        }
        if let Some(why) = preflight_refused(&answers) {
            let funds = why.contains("insufficient_funds");
            let giving_up = !funds && s.giving_up.is_none() && s.burn.is_none();
            let e = format!("intent {} ({}): every provider's preflight refused it: {why}; {}", s.intent, s.what,
                if funds { "it is resent with preflight each tick until a provider takes it (the fee payer cannot pay)" }
                else if s.burn.is_some() { "this is its nonce burn: resent until it or the intent lands" }
                else { "its signatures' fate is read from the chain; the intent is given up (never reported unlanded before that is proven)" });
            line.error = Some(e.clone());
            if let (true, Some(x)) = (giving_up, line.saved.as_mut()) {
                x.giving_up = Some(why);
            }
            return Ok(Step::Preflight(e));
        }
        if s.giving_up.is_none() {
            line.error = None;
        }
        return Ok(Step::Pending { intent: s.intent, sends: s.sends + 1 });
    }
    let st = solrpc::statuses(ps, &s.history).await?;
    if let Some((sig, status)) = s.history.iter().zip(&st).find_map(|(g, x)| x.as_ref().filter(|x| x.level == Level::Finalized).map(|x| (g, x))) {
        if s.burn.as_deref() == Some(sig.as_str()) {
            return Ok(Step::Abandoned(settle(line, &s, sig, status.err.clone(), true)));
        }
        return Ok(Step::Landed(settle(line, &s, sig, status.err.clone(), false)));
    }
    if let Some(i) = &line.incident {
        return Ok(Step::Incident(i.clone()));
    }
    let l = AnyLifetime::of(s.lifetime, line.nonce_account).map_err(Refused::Failed)?;
    if l.can_never_land(&s, ps).await? {
        match s.lifetime {
            LifetimeKind::DurableNonce => {
                let why = format!("the nonce account moved under intent {} ({}), and none of {} is known: no automatic re-sign",
                    s.intent, s.what, s.history.join(", "));
                line.incident = Some(why.clone());
                return Ok(Step::Incident(why));
            }
            LifetimeKind::RecentBlockhash if s.giving_up.is_some() => {
                // PROVEN: every signature of the given-up intent is past its last valid height and unknown at a fresh read
                return Ok(Step::Abandoned(settle(line, &s, "", None, true)));
            }
            LifetimeKind::RecentBlockhash => {
                let keys = s.keys.clone().unwrap_or_default();
                let tables = s.tables.clone().unwrap_or_default();
                let (a, tx, signature) = sign_intent(&l, ps, signer, &s.instructions, &keys, &tables).await.map_err(Refused::Failed)?;
                // re-read: the line is the caller's; only the saved record of THIS intent is replaced. The old signatures are proven
                // expired and unknown: they leave the status query (audit A4-03) and stay in `expired` for `sol_seen`
                let mut expired: Vec<String> = s.expired.clone().unwrap_or_default();
                expired.extend(s.history.iter().cloned());
                if expired.len() > EXPIRED_KEPT {
                    let extra = expired.len() - EXPIRED_KEPT;
                    expired.drain(..extra);
                }
                line.saved = Some(Saved { recent_blockhash: a.recent_blockhash, last_valid_height: a.last_valid_height, tx,
                    signature: signature.clone(), history: vec![signature.clone()], sends: 0, expired: Some(expired), ..s.clone() });
                return Ok(Step::Resigned { intent: s.intent, signature });
            }
        }
    }
    if s.giving_up.is_some() {
        if s.lifetime == LifetimeKind::DurableNonce && s.burn.is_none() {
            // audit A4-01: give the intent up by burning its nonce: an `AdvanceNonceAccount`-only transaction on the SAME nonce. At most
            // one of the two lands; the statuses say which (the intent's own landing is its outcome, the burn's is "abandoned")
            let me = signer.me();
            let a = l.anchor(&me, ps).await?;
            if a.recent_blockhash != s.recent_blockhash {
                return Err(Refused::Failed(format!("intent {}'s nonce moved before its burn was signed: its statuses decide next tick", s.intent)));
            }
            // not simulated: the burn is the nonce advance alone (it fails only if the nonce moved, which was just read not to be so),
            // and a simulation refusing it for any other reason must not keep the intent alive
            let msg = soltx::compile_parts(&a.prefix, me, a.recent_blockhash).map_err(Refused::Failed)?.message();
            let (tx, sig) = sign_message(signer, &msg).await.map_err(Refused::Failed)?;
            let signature = soltx::b58_encode(&sig);
            let mut history = s.history.clone();
            history.push(signature.clone());
            line.saved = Some(Saved { tx, burn: Some(signature.clone()), history, sends: 0, ..s.clone() });
            return Ok(Step::Burning { intent: s.intent, signature });
        }
        if s.burn.is_none() {
            // fallback: never resent as itself; its expiry is waited for
            return Ok(Step::Pending { intent: s.intent, sends: s.sends });
        }
    }
    send_saved(line, ps).await;
    Ok(Step::Pending { intent: s.intent, sends: s.sends + 1 })
}

/// Resend the saved bytes to every provider (not replicated) and count the send. The answers are not consulted, but for a line whose
/// fee payer could not pay (`Line.error` names `insufficient_funds`): those bytes are resent WITH preflight, and the error is cleared
/// once no provider refuses them any more.
async fn send_saved<P: SolRpc>(line: &mut Line, ps: &mut [P]) {
    let Some(tx) = line.saved.as_ref().map(|s| s.tx.clone()) else { return };
    let funds = line.error.as_deref().is_some_and(|e| e.contains("insufficient_funds"));
    let answers = solrpc::send(ps, &tx, funds).await;
    if funds && preflight_refused(&answers).is_none() && answers.iter().any(|a| a.is_ok()) {
        line.error = None;
    }
    if let Some(s) = line.saved.as_mut() {
        s.sends = s.sends.saturating_add(1);
    }
}

/// The operator's act after an incident (`solhome::sol_clear_incident`, which first re-reads every signature's status on chain and
/// refuses if any is known: NEW-A4-01): the pending intent is abandoned and the line is free. Its signatures move to `done`
/// (`abandoned`), so `sol_seen` keeps them. Returns the abandoned record. Never automatic.
pub fn clear_incident(line: &mut Line) -> Option<Saved> {
    line.incident.as_ref()?;
    let s = line.saved.clone();
    match &s {
        Some(x) => {
            settle(line, x, "", None, true);
        }
        None => line.incident = None,
    }
    s
}

/// `sol_seen`: every signature of every line, pending or finished (a landed transaction from an old signature is still ours).
pub fn sol_seen(lines: &[&Line]) -> Vec<String> {
    let mut v: Vec<String> = lines.iter().flat_map(|l| {
        l.saved.iter().flat_map(all_signatures).chain(l.done.iter().flat_map(|d| d.history.iter().cloned()))
    }).collect();
    v.sort();
    v.dedup();
    v
}
