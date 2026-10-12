//! Audit A4 (docs/v9/audit-2026-10/a4-canister-sol-primitives.md): proof-of-concept tests for the canister's Solana primitives.
//!
//! Each `poc_*` test asserts the property the code is meant to hold and FAILS on the baseline (f2c24e0); it passes once the finding
//! is fixed. The world is the fake provider pair of `tests/fakerpc/sol.rs` (running the canister's own transform), wrapped where a
//! finding needs one provider answer to be forged (a non-replicated send answered by one replica) or a status read to be served from
//! a lagging backend.
mod fakerpc;

use std::cell::Cell;
use std::future::Future;
use std::pin::pin;
use std::rc::Rc;
use std::task::{Context, Poll, Waker};

use etch_autobuyer::canister::{restore_home, Home};
use etch_autobuyer::ethflow::Step as EStep;
use etch_autobuyer::solhome::{self, LineOwner, LineStep, SolHomeCfg, SolIo, SolLine};
use etch_autobuyer::solrpc::{self, normalise, Account, Level, Req, SolRpc};
use etch_autobuyer::solsend::{self, LifetimeKind, Line, Step};
use etch_autobuyer::soltx::{self, Instruction, Pubkey};
use fakerpc::sol::{pair, FakeSigner, FakeSol, SolChain};
use serde_json::{json, Value};

fn block_on<F: Future>(f: F) -> F::Output {
    let mut f = pin!(f);
    let mut cx = Context::from_waker(Waker::noop());
    loop {
        if let Poll::Ready(v) = f.as_mut().poll(&mut cx) {
            return v;
        }
    }
}

const ME: Pubkey = [0x11; 32];
const TO: Pubkey = [0x22; 32];
const SEED: &str = "rook-home-0";

/// One fake provider, optionally lying the way a single replica of a NON-replicated outcall can (its answer is not agreed), or
/// serving `getSignatureStatuses` from a backend that has not seen the latest finalized blocks (a load-balanced provider).
struct Wrap {
    inner: FakeSol,
    /// Sends still to be answered with a forged preflight refusal (the bytes ARE forwarded first).
    forge_refusals: Rc<Cell<u32>>,
    /// Status reads answered "unknown" for every signature (a lagging backend).
    stale_status: Rc<Cell<bool>>,
}

impl SolRpc for Wrap {
    fn name(&self) -> String {
        self.inner.name()
    }
    async fn ask(&mut self, r: &Req) -> Result<Value, String> {
        if r.method == "sendTransaction" && self.forge_refusals.get() > 0 {
            self.forge_refusals.set(self.forge_refusals.get() - 1);
            let _really_forwarded = self.inner.ask(r).await;
            // what one replica reports back for the provider: a preflight refusal of the instruction (the transform's class)
            let body = json!({ "jsonrpc": "2.0", "id": 1, "error": { "code": -32002,
                "message": "Transaction simulation failed: Error processing Instruction 1: custom program error: 0x177c",
                "data": { "err": { "InstructionError": [1, { "Custom": 6012 }] }, "logs": [] } } });
            return Ok(normalise(200, body.to_string().as_bytes(), &solrpc::context(r.kind, r.bucket)));
        }
        if r.method == "getSignatureStatuses" && self.stale_status.get() {
            let n = r.params[0].as_array().map(|a| a.len()).unwrap_or(0);
            let body = json!({ "jsonrpc": "2.0", "id": 1, "result": { "context": { "slot": 1 }, "value": vec![Value::Null; n] } });
            return Ok(normalise(200, body.to_string().as_bytes(), &solrpc::context(r.kind, r.bucket)));
        }
        self.inner.ask(r).await
    }
}

/// A Solana home world for `solhome` (the IC's `SolNet` with the fakes): one line, one signer, the wrapped pair.
struct World {
    cfg: SolHomeCfg,
    ps: Vec<Wrap>,
    signer: FakeSigner,
    line: SolLine,
}

impl SolIo for World {
    fn sol_home(&self) -> Option<SolHomeCfg> {
        Some(self.cfg.clone())
    }
    async fn sol_me(&mut self) -> Result<Pubkey, String> {
        Ok(ME)
    }
    async fn sol_accounts(&mut self, keys: &[Pubkey], max_data: usize) -> Result<Vec<Option<Account>>, String> {
        Ok(solrpc::accounts(&mut self.ps, keys, max_data).await?)
    }
    async fn sol_rent(&mut self, space: u64) -> Result<u64, String> {
        Ok(solrpc::rent_exempt(&mut self.ps, space).await?)
    }
    async fn sol_relay(&mut self) -> Result<Pubkey, String> {
        Ok(self.signer.relay)
    }
    async fn sol_balance_of(&mut self, key: &Pubkey) -> Result<u128, String> {
        Ok(solrpc::balance(&mut self.ps, key).await? as u128)
    }
    async fn sol_prepare(&mut self, line: &mut Line, kind: LifetimeKind, intent: u64, what: &str, ixs: Vec<Instruction>) -> Result<(), String> {
        solsend::prepare(line, kind, intent, what, ixs, &mut self.ps, &mut self.signer).await
    }
    async fn sol_tick(&mut self, line: &mut Line) -> Result<Step, String> {
        Ok(solsend::tick(line, &mut self.ps, &mut self.signer).await?)
    }
    fn sol_line(&mut self) -> SolLine {
        self.line.clone()
    }
    fn sol_line_save(&mut self, l: &SolLine) {
        self.line = l.clone();
    }
}

struct Knobs {
    forge: Rc<Cell<u32>>,
    stale: Rc<Cell<bool>>,
}

fn world(chain: &std::rc::Rc<std::cell::RefCell<SolChain>>, lifetime: LifetimeKind) -> (World, Knobs) {
    let forge = Rc::new(Cell::new(0));
    let stale = Rc::new(Cell::new(false));
    let ps = pair(chain).into_iter().map(|inner| Wrap { inner, forge_refusals: forge.clone(), stale_status: stale.clone() }).collect();
    let cfg = SolHomeCfg { lifetime, nonce_seed: SEED.into() };
    (World { cfg, ps, signer: FakeSigner::new(ME), line: SolLine::default() }, Knobs { forge, stale })
}

fn funded_chain() -> std::rc::Rc<std::cell::RefCell<SolChain>> {
    let chain = SolChain::new();
    chain.borrow_mut().set_account(ME, soltx::SYSTEM_PROGRAM, 10_000_000_000, vec![]);
    chain.borrow_mut().advance(25, &[]);
    chain
}

/// The step's instructions: a plain transfer (a Relay `deposit_native` or an `advance` moves lamports the same way: not idempotent).
fn pay() -> Vec<Instruction> {
    vec![soltx::transfer(ME, TO, 1_000_000)]
}

/// How many landed transactions carried the step (any signature of it).
fn landed_payments(chain: &std::rc::Rc<std::cell::RefCell<SolChain>>, sigs: &[String]) -> usize {
    let c = chain.borrow();
    sigs.iter().filter(|s| c.landed.get(*s).is_some_and(|l| l.err.is_none())).count()
}

/// A flow's step through the line as `ethflow::home_sol` drives it: claim, sign (save, first send), then one `tick_line` pass and the
/// flow's outcome. Returns the signature and the outcome's `ok`.
fn run_step(w: &mut World, chain: &std::rc::Rc<std::cell::RefCell<SolChain>>, buy: u64) -> (String, Option<bool>) {
    block_on(solhome::ready(w, LineOwner::Eth, 1_000, "advance")).expect("the line is free");
    let (_text, p) = block_on(solhome::sign(w, LineStep::Eth(EStep::Advance { buy }), 1_000, pay())).expect("signed and saved");
    let sig = w.line.line.saved.as_ref().map(|s| s.signature.clone()).unwrap_or_default();
    // the chain moves on; what was forwarded lands and finalizes
    chain.borrow_mut().advance(40, &[]);
    chain.borrow_mut().process(Level::Finalized);
    let mut ok = None;
    for _ in 0..4 {
        let _ = block_on(solhome::tick_line(w, 1_000));
        let mut l = w.sol_line();
        let mine = l.take(LineOwner::Eth);
        w.sol_line_save(&l);
        if let Some(o) = mine.into_iter().find(|o| o.intent == p.intent) {
            ok = o.ok;
            break;
        }
    }
    (sig, ok)
}

// ---- A4-01: "given up unforwarded" is believed from answers that prove nothing ---------------------------------------------------------

/// Durable nonce (the default). The first send's answers are forged into preflight refusals by the replica that ran the
/// non-replicated outcall (or the bytes are kept by a provider / read from the public `sol_line` query and submitted later). The
/// line abandons the step as "never landed"; the flow takes it back and signs it again; BOTH payments land.
#[test]
fn poc_a4_01_abandoned_step_lands_and_its_replacement_lands_too_durable_nonce() {
    let chain = funded_chain();
    let (mut w, k) = world(&chain, LifetimeKind::DurableNonce);
    let nonce = solsend::nonce_account_of(&ME, SEED).unwrap();
    chain.borrow_mut().set_nonce(nonce, ME);
    w.line.line.nonce_account = Some(nonce);

    // the first send of step 1: forwarded by both providers, reported back as "every preflight refused"
    k.forge.set(4);
    let (first, ok) = run_step(&mut w, &chain, 1);
    assert!(chain.borrow().landed.contains_key(&first), "it landed");
    // FIXED (A4-01): the forged refusals decide nothing; the line reports what the agreed statuses say (the PoC asserted the old
    // "abandoned, never landed" here)
    assert_eq!(ok, Some(true), "the line reports the landing, never 'abandoned' from send answers");

    // the flow signs the step again only if it was told it never landed
    let mut sigs = vec![first];
    if ok != Some(true) {
        let (second, ok2) = run_step(&mut w, &chain, 1);
        assert_eq!(ok2, Some(true));
        sigs.push(second);
    }
    let n = landed_payments(&chain, &sigs);
    assert!(n <= 1, "A4-01: one intent, {n} successful landings (the line said the first never landed)");
}

/// The blockhash fallback: the same, and the abandoned bytes stay valid for up to 150 blocks while the replacement is signed on a
/// fresh blockhash, so both land without even an incident.
#[test]
fn poc_a4_01_abandoned_step_lands_and_its_replacement_lands_too_blockhash_fallback() {
    let chain = funded_chain();
    let (mut w, k) = world(&chain, LifetimeKind::RecentBlockhash);
    k.forge.set(4);
    block_on(solhome::ready(&mut w, LineOwner::Eth, 1_000, "advance")).unwrap();
    let (_t, p) = block_on(solhome::sign(&mut w, LineStep::Eth(EStep::Advance { buy: 7 }), 1_000, pay())).unwrap();
    let first = w.line.line.saved.as_ref().unwrap().clone();
    // the next pass: preflight "refused" by every provider. FIXED (A4-01): nothing is decided on it, no outcome is reported (the PoC
    // asserted the old outcome "never landed" here), the intent is given up and waits for its fate on chain
    let _ = block_on(solhome::tick_line(&mut w, 1_000));
    let mut l = w.sol_line();
    let o = l.take(LineOwner::Eth);
    w.sol_line_save(&l);
    assert!(o.is_empty(), "no outcome from send answers: {o:?}");
    assert!(w.line.line.saved.as_ref().is_some_and(|s| s.giving_up.is_some()), "given up, still on the line");
    // the bytes were forwarded (or are submitted by whoever kept them) inside their 150 blocks: their landing is the step's outcome
    chain.borrow_mut().advance(25, &[]);
    chain.borrow_mut().process(Level::Finalized);
    let mut sigs = vec![first.signature.clone()];
    let mut ok = None;
    for _ in 0..4 {
        let _ = block_on(solhome::tick_line(&mut w, 1_000));
        let mut l = w.sol_line();
        let mine = l.take(LineOwner::Eth);
        w.sol_line_save(&l);
        if let Some(o) = mine.into_iter().find(|o| o.intent == p.intent) {
            ok = Some(o.ok);
            break;
        }
    }
    assert_eq!(ok, Some(Some(true)), "the landing is reported");
    // the flow signs the step again only if it was told it never landed
    if ok != Some(Some(true)) {
        let (second, _) = run_step(&mut w, &chain, 7);
        sigs.push(second);
    }
    chain.borrow_mut().process(Level::Finalized);
    let n = landed_payments(&chain, &sigs);
    assert!(n <= 1, "A4-01: one intent, {n} successful landings under the blockhash fallback");
}

// ---- A4-02: anyone can squat the line's durable-nonce address for ~0.0009 SOL --------------------------------------------------------

/// `create_with_seed(me, seed, System)` is deterministic; a plain transfer to it (>= the rent-exempt minimum of 0 bytes, 890,880
/// lamports) makes a system-owned, data-less account there. `CreateAccountWithSeed` then fails (`AccountAlreadyInUse`) and
/// `open_line` refuses the account as "not ours" on every tick. The seed is identity (`restore_home` keeps it), so no upgrade of
/// the seed recovers; only switching the whole home to the weaker blockhash fallback does.
#[test]
fn poc_a4_02_a_transfer_to_the_nonce_address_blocks_the_durable_nonce_line_for_good() {
    let chain = funded_chain();
    let (mut w, _k) = world(&chain, LifetimeKind::DurableNonce);
    let nonce = solsend::nonce_account_of(&ME, SEED).unwrap();
    // the griefer's transfer (System Program owned, no data)
    chain.borrow_mut().set_account(nonce, soltx::SYSTEM_PROGRAM, 890_880, vec![]);

    // the identity cannot be moved off the squatted seed by an upgrade
    let saved = Home::Solana { cluster: "devnet".into(), rpc_urls: vec!["https://a".into(), "https://b".into()],
        rook_core: "books".into(), nonce_seed: SEED.into(), blockhash_fallback: None, rook_backing: None, drand_urls: None };
    let arg = Home::Solana { cluster: "devnet".into(), rpc_urls: vec!["https://a".into(), "https://b".into()],
        rook_core: "books".into(), nonce_seed: "rook-home-1".into(), blockhash_fallback: None, rook_backing: None, drand_urls: None };
    let Some(Home::Solana { nonce_seed, .. }) = restore_home(Some(saved), Some(&arg)) else { panic!() };
    assert_eq!(nonce_seed, SEED, "the seed is identity: an upgrade cannot move it");

    let mut last = Ok(ME);
    for _ in 0..5 {
        last = block_on(solhome::ready(&mut w, LineOwner::Eth, 1_000, "advance"));
        chain.borrow_mut().advance(40, &[]);
        chain.borrow_mut().process(Level::Finalized);
        let _ = block_on(solhome::tick_line(&mut w, 1_000));
    }
    assert!(last.is_ok() && w.line.line.nonce_account.is_some(),
        "A4-02: the durable-nonce line never opens after a 890,880-lamport transfer to its address: {last:?}");
}

// ---- A4-03: the blockhash fallback stalls for good after 64 expiries -------------------------------------------------------------------

/// Every proven expiry pushes one more signature onto `history`; `getSignatureStatuses` is refused above `MAX_SIGS` (64) signatures,
/// so from the 65th signature on every tick of the line is `Refused` before anything else: the line (and every flow behind it) is
/// stuck, with no incident for the operator to clear.
#[test]
fn poc_a4_03_sixty_four_expiries_stall_the_fallback_line() {
    let chain = funded_chain();
    let mut ps = pair(&chain);
    let mut signer = FakeSigner::new(ME);
    let mut line = Line::default();
    block_on(solsend::prepare(&mut line, LifetimeKind::RecentBlockhash, 1, "t", pay(), &mut ps, &mut signer)).unwrap();
    let mut resigns = 0;
    for _ in 0..200 {
        let st = block_on(solsend::tick(&mut line, &mut ps, &mut signer));
        match st {
            Ok(Step::Resigned { .. }) => resigns += 1,
            Ok(_) => {}
            Err(e) => {
                panic!("A4-03: after {resigns} expiries ({} signatures in history) the line is refused every tick: {e}",
                    line.saved.as_ref().unwrap().history.len());
            }
        }
        // congestion: nothing of ours lands; the blockhash expires
        chain.borrow_mut().advance(200, &[]);
        if resigns >= 70 {
            break;
        }
    }
}

// ---- A4-04: "can never land" under the fallback trusts a status read that may be older than the height read ---------------------

/// The ordering rule (lifetime fact FIRST, statuses SECOND) assumes each provider's view only moves forward. A load-balanced provider
/// can serve `getBlockHeight` from an up-to-date backend and `getSignatureStatuses` from one a few slots behind (nothing asks for a
/// `minContextSlot`; the transform drops `context.slot`). A transaction that landed in the last blocks of its validity then looks
/// "unknown + expired", the same intent is re-signed, and both land.
#[test]
fn poc_a4_04_a_lagging_status_backend_makes_the_fallback_re_sign_a_landed_intent() {
    let chain = funded_chain();
    let (mut w, k) = world(&chain, LifetimeKind::RecentBlockhash);
    let mut line = Line::default();
    block_on(solsend::prepare(&mut line, LifetimeKind::RecentBlockhash, 1, "t", pay(), &mut w.ps, &mut w.signer)).unwrap();
    let first = line.saved.clone().unwrap();
    assert!(matches!(block_on(solsend::tick(&mut line, &mut w.ps, &mut w.signer)).unwrap(), Step::Pending { .. }));
    // it lands near the end of its validity and finalizes; the chain moves past the last valid height
    chain.borrow_mut().advance(100, &[]);
    assert_eq!(chain.borrow_mut().process(Level::Finalized), vec![first.signature.clone()]);
    chain.borrow_mut().advance(60, &[]);
    // both providers' status reads hit a backend that has not caught up
    k.stale.set(true);
    let st = block_on(solsend::tick(&mut line, &mut w.ps, &mut w.signer)).unwrap();
    k.stale.set(false);
    if let Step::Resigned { signature, .. } = st {
        assert!(matches!(block_on(solsend::tick(&mut line, &mut w.ps, &mut w.signer)).unwrap(), Step::Pending { .. }));
        chain.borrow_mut().process(Level::Finalized);
        let n = landed_payments(&chain, &[first.signature.clone(), signature]);
        assert!(n <= 1, "A4-04: one intent, {n} successful landings after a stale status read");
    }
}

/// No forged answer needed: a real preflight refusal (the state the step was signed for moved between the simulation and the first
/// send) abandons the step, but its signed bytes were public (`sol_line` is a query anyone may call) and are still valid on the
/// nonce. Anyone submits them with `skipPreflight`; they land FAILED, which still advances the nonce, so the replacement the flow
/// signed on that same nonce can never land: an INCIDENT halts every Solana flow until the operator clears it. The claim that a
/// moved nonce means "a bug or a compromised key" does not hold.
#[test]
fn poc_a4_01_anyone_turns_an_abandoned_step_into_an_incident() {
    let chain = funded_chain();
    let (mut w, _k) = world(&chain, LifetimeKind::DurableNonce);
    let nonce = solsend::nonce_account_of(&ME, SEED).unwrap();
    chain.borrow_mut().set_nonce(nonce, ME);
    w.line.line.nonce_account = Some(nonce);
    // the simulation passes; the first send's preflight (moments later) fails on the moved state, at every provider
    let flip: Box<dyn FnMut(&Req, &mut SolChain)> = Box::new(|r: &Req, c: &mut SolChain| {
        if r.method == "sendTransaction" {
            c.preflight_err = Some(json!({ "InstructionError": [1, { "Custom": 6012 }] }));
        }
    });
    *w.ps[0].inner.hook.borrow_mut() = Some(flip);
    block_on(solhome::ready(&mut w, LineOwner::Eth, 1_000, "advance")).unwrap();
    let (_t, p) = block_on(solhome::sign(&mut w, LineStep::Eth(EStep::Advance { buy: 3 }), 1_000, pay())).unwrap();
    // FIXED (A4-01): the public query no longer shows the signed bytes; the griefer is given them anyway (a provider kept them)
    assert!(solhome::public_view(w.line.clone()).line.saved.unwrap().tx.is_empty(), "sol_line hides the bytes in flight");
    let public_bytes = w.line.line.saved.clone().unwrap().tx;
    // the next pass: the intent is given up by burning its nonce (the PoC asserted the old outcome "never landed" here: none is
    // reported before the intent's fate is known)
    let t = block_on(solhome::tick_line(&mut w, 1_000)).unwrap();
    assert!(t.contains("nonce burn"), "{t}");
    let mut l = w.sol_line();
    assert!(l.take(LineOwner::Eth).is_empty(), "no outcome from a preflight refusal");
    w.sol_line_save(&l);
    *w.ps[0].inner.hook.borrow_mut() = None;
    chain.borrow_mut().preflight_err = None;
    // the griefer submits the bytes first, skipping preflight; they land failed and advance the nonce (the burn can no longer land)
    chain.borrow_mut().sent.push(public_bytes);
    chain.borrow_mut().fail_next = Some(json!({ "InstructionError": [1, { "Custom": 6012 }] }));
    chain.borrow_mut().advance(40, &[]);
    chain.borrow_mut().process(Level::Finalized);
    let mut ok = None;
    for _ in 0..4 {
        let _ = block_on(solhome::tick_line(&mut w, 1_000));
        chain.borrow_mut().process(Level::Finalized);
        let mut l = w.sol_line();
        let mine = l.take(LineOwner::Eth);
        w.sol_line_save(&l);
        if let Some(o) = mine.into_iter().find(|o| o.intent == p.intent) {
            ok = Some(o.ok);
            break;
        }
    }
    assert_eq!(ok, Some(Some(false)), "the griefer's failed landing IS the intent's outcome (failed on chain): the flow retries");
    // the flow signs the replacement on the moved nonce, from a fresh read
    block_on(solhome::ready(&mut w, LineOwner::Eth, 1_000, "advance")).unwrap();
    block_on(solhome::sign(&mut w, LineStep::Eth(EStep::Advance { buy: 3 }), 1_000, vec![soltx::transfer(ME, TO, 999_999)])).unwrap();
    chain.borrow_mut().advance(40, &[]);
    chain.borrow_mut().process(Level::Finalized);
    for _ in 0..3 {
        let _ = block_on(solhome::tick_line(&mut w, 1_000));
    }
    assert!(w.line.line.incident.is_none(), "A4-01: a stranger's submission of abandoned public bytes halts the line: {:?}", w.line.line.incident);
}

// ---- the fixes (docs/v9/audit-2026-10/a4-canister-sol-primitives.md "## Fixes") -------------------------------------------------------

fn refuse_preflight(w: &mut World) {
    let flip: Box<dyn FnMut(&Req, &mut SolChain)> = Box::new(|r: &Req, c: &mut SolChain| {
        if r.method == "sendTransaction" {
            c.preflight_err = Some(json!({ "InstructionError": [1, { "Custom": 6012 }] }));
        }
    });
    *w.ps[0].inner.hook.borrow_mut() = Some(flip);
}

fn outcome(w: &mut World, chain: &std::rc::Rc<std::cell::RefCell<SolChain>>, intent: u64, passes: usize) -> Option<(Option<bool>, String)> {
    for _ in 0..passes {
        let _ = block_on(solhome::tick_line(w, 1_000));
        chain.borrow_mut().process(Level::Finalized);
        let mut l = w.sol_line();
        let mine = l.take(LineOwner::Eth);
        w.sol_line_save(&l);
        if let Some(o) = mine.into_iter().find(|o| o.intent == intent) {
            return Some((o.ok, o.signature));
        }
    }
    None
}

/// A4-01, durable nonce: a step whose preflight every provider refuses is given up by burning its nonce (an `AdvanceNonceAccount`-only
/// transaction on the same nonce); it is reported unlanded only once that burn finalizes, and its own bytes can never land after.
#[test]
fn fix_a4_01_a_given_up_step_is_reported_unlanded_only_when_its_nonce_burn_lands() {
    let chain = funded_chain();
    let (mut w, _k) = world(&chain, LifetimeKind::DurableNonce);
    let nonce = solsend::nonce_account_of(&ME, SEED).unwrap();
    chain.borrow_mut().set_nonce(nonce, ME);
    w.line.line.nonce_account = Some(nonce);
    refuse_preflight(&mut w);
    block_on(solhome::ready(&mut w, LineOwner::Eth, 1_000, "advance")).unwrap();
    let (_t, p) = block_on(solhome::sign(&mut w, LineStep::Eth(EStep::Advance { buy: 5 }), 1_000, pay())).unwrap();
    let original = w.line.line.saved.clone().unwrap();
    assert!(original.giving_up.is_some() && w.line.line.error.is_some(), "a line error, and the intent given up");
    *w.ps[0].inner.hook.borrow_mut() = None;
    chain.borrow_mut().preflight_err = None;
    let (ok, sig) = outcome(&mut w, &chain, p.intent, 6).expect("an outcome once the burn lands");
    let burn = w.line.line.done.last().unwrap().clone();
    assert_eq!((ok, burn.abandoned), (None, Some(true)), "abandoned, proven by the burn");
    assert_eq!(sig, burn.signature);
    assert!(burn.history.contains(&original.signature) && burn.history.contains(&sig), "every signature kept: {:?}", burn.history);
    assert!(solsend::sol_seen(&[&w.line.line]).contains(&original.signature));
    // whoever kept the original bytes submits them now: the nonce moved, they cannot land
    chain.borrow_mut().sent.push(original.tx.clone());
    chain.borrow_mut().process(Level::Finalized);
    assert_eq!(landed_payments(&chain, &[original.signature]), 0);
}

/// A4-01, fallback: a given-up step is never resent as itself and is reported unlanded only once its expiry is PROVEN.
#[test]
fn fix_a4_01_under_the_fallback_a_given_up_step_is_reported_unlanded_only_after_its_proven_expiry() {
    let chain = funded_chain();
    let (mut w, _k) = world(&chain, LifetimeKind::RecentBlockhash);
    refuse_preflight(&mut w);
    block_on(solhome::ready(&mut w, LineOwner::Eth, 1_000, "advance")).unwrap();
    let (_t, p) = block_on(solhome::sign(&mut w, LineStep::Eth(EStep::Advance { buy: 6 }), 1_000, pay())).unwrap();
    *w.ps[0].inner.hook.borrow_mut() = None;
    chain.borrow_mut().preflight_err = None;
    let sent = chain.borrow().sent.len();
    assert!(outcome(&mut w, &chain, p.intent, 2).is_none(), "not proven yet: no outcome");
    assert_eq!(chain.borrow().sent.len(), sent, "never resent as itself");
    chain.borrow_mut().advance(200, &[]);
    let (ok, _) = outcome(&mut w, &chain, p.intent, 2).expect("expiry proven");
    assert_eq!(ok, None);
    assert_eq!(w.line.line.done.last().unwrap().abandoned, Some(true));
}

/// A4-04: a status read bound to a slot is refused as stale when the provider's context is older (the transform's bucket carries the
/// slot, so every replica agrees); unbound reads are as before.
#[test]
fn fix_a4_04_a_status_answer_older_than_the_bound_slot_is_stale() {
    use etch_autobuyer::solrpc::{context, Kind};
    let body = |slot: u64| json!({ "jsonrpc": "2.0", "id": 1, "result": { "context": { "slot": slot }, "value": [null] } }).to_string();
    assert_eq!(normalise(200, body(5).as_bytes(), &context(Kind::SigStatus, 100)), json!({ "stale": true }));
    assert_eq!(normalise(200, body(120).as_bytes(), &context(Kind::SigStatus, 100)), json!({ "statuses": [null] }));
    assert_eq!(normalise(200, body(5).as_bytes(), &context(Kind::SigStatus, 1)), json!({ "statuses": [null] }));
}

/// A4-02: the in-place opening's instructions: AllocateWithSeed (9) of the seed address, a top-up only when it holds under the rent,
/// InitializeNonceAccount, one signer (us).
#[test]
fn fix_a4_02_the_in_place_opening_is_allocate_top_up_initialize() {
    let n = solsend::nonce_account_of(&ME, SEED).unwrap();
    let v = solsend::open_squatted_nonce_ixs(&ME, SEED, 1_447_680, 890_880).unwrap();
    assert_eq!(v.len(), 3);
    assert_eq!(&v[0].data[..4], &9u32.to_le_bytes());
    assert_eq!((v[0].accounts[0].key, v[0].accounts[1].key, v[0].accounts[1].signer), (n, ME, true));
    assert_eq!(&v[1].data[..4], &2u32.to_le_bytes());
    assert_eq!(u64::from_le_bytes(v[1].data[4..12].try_into().unwrap()), 1_447_680 - 890_880);
    assert_eq!(&v[2].data[..4], &6u32.to_le_bytes());
    assert_eq!(solsend::open_squatted_nonce_ixs(&ME, SEED, 1_447_680, 2_000_000).unwrap().len(), 2, "no top-up needed");
}

