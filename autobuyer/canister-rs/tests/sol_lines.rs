//! W3 (docs/v9/canister-only-solana.md §1.1, §13.1; spike S3): multi-key signing and the four send lines, against the fake provider pair
//! (`tests/fakerpc/sol.rs`, the canister's own transform) and a signer holding REAL Ed25519 keys, one per `KeyPath`, so every signature of
//! a multi-signer transaction is verified here with `ed25519-dalek` (`verify_strict`), in the compiled message's signer order.
//!
//! - one transaction signed by `K_main` + `K_int[7]` + `K_coll[3]`: the signer order is the compiler's (fee payer first, the rest by key
//!   bytes), each signature verifies, it lands once; a fallback re-sign of it signs with the same three keys;
//! - refusals BEFORE any signature (and before the simulation): a foreign signer, the Relay key with another, a fee payer other than
//!   `K_main`, a transaction over 1,232 bytes; a simulation refusal signs nothing; a full signing queue saves nothing (retry next tick);
//! - four lines, each its own durable nonce (opened by the line itself), never waiting for each other: land, resend, expiry under the
//!   fallback, an incident on one line only, outcomes kept per line; the fair pass and its cursor; the busy flag the timer reads;
//! - the upgrade: a state saved before W3 (the 14-element tuple, the one line) restores into four lines; a line saved with the registry
//!   keeper's state (`crank`, retired with rook_core: D-l) still decodes, the state dropped.
mod fakerpc;

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::future::Future;
use std::pin::pin;
use std::rc::Rc;
use std::task::{Context, Poll, Waker};

use ed25519_dalek::{Signer, SigningKey, Verifier};
use etch_autobuyer::cranks::TreasuryStep;
use etch_autobuyer::solhome::{self, CustodyStep, LineId, LineOp, LineOwner, LineStep, SolHomeCfg, SolIo, SolLine, SolLines};
use etch_autobuyer::solkey::{self, KeyPath, SolSigner};
use etch_autobuyer::solrpc::{self, Account, Level};
use etch_autobuyer::solsend::{self, LifetimeKind, Line, Step};
use etch_autobuyer::soltx::{self, AccountMeta, Instruction, Pubkey};
use fakerpc::sol::{pair, FakeSol, SolChain};
use sha2::{Digest, Sha256};

fn block_on<F: Future>(f: F) -> F::Output {
    let mut f = pin!(f);
    let mut cx = Context::from_waker(Waker::noop());
    loop {
        if let Poll::Ready(v) = f.as_mut().poll(&mut cx) {
            return v;
        }
    }
}

const SEED: &str = "rook-home-0";
const TO: Pubkey = [0x22; 32];

/// A signer with a real Ed25519 key per path (seeded from the path's derivation bytes: a stand-in for the IC's derivation, which only
/// the subnet can do with private keys). `full`: every request is refused as the IC refuses one over its per-key queue (S3 row 9).
#[derive(Clone)]
struct RealSigner {
    calls: Rc<RefCell<Vec<(KeyPath, Vec<u8>)>>>,
    full: Rc<RefCell<bool>>,
}

fn sk(p: &KeyPath) -> SigningKey {
    let seed: [u8; 32] = Sha256::digest([b"rook-w3-test".to_vec(), p.derivation().concat()].concat()).into();
    SigningKey::from_bytes(&seed)
}

fn pk(p: KeyPath) -> Pubkey {
    sk(&p).verifying_key().to_bytes()
}

impl RealSigner {
    fn new() -> RealSigner {
        RealSigner { calls: Rc::new(RefCell::new(vec![])), full: Rc::new(RefCell::new(false)) }
    }
    fn sign(&mut self, p: &KeyPath, m: &[u8]) -> Result<[u8; 64], String> {
        if *self.full.borrow() {
            return Err("sign_with_schnorr(ed25519): CallRejected { reject_code: 4, message: \"request queue for key schnorr:Ed25519:key_1 is full\" }".into());
        }
        self.calls.borrow_mut().push((*p, m.to_vec()));
        Ok(sk(p).sign(m).to_bytes())
    }
    fn signed(&self) -> usize {
        self.calls.borrow().len()
    }
}

impl SolSigner for RealSigner {
    fn me(&self) -> Pubkey {
        pk(KeyPath::Main)
    }
    async fn sign_bytes(&mut self, message: &[u8]) -> Result<[u8; 64], String> {
        self.sign(&KeyPath::Main, message)
    }
    fn relay(&self) -> Option<Pubkey> {
        Some(pk(KeyPath::Relay))
    }
    async fn sign_relay_bytes(&mut self, message: &[u8]) -> Result<[u8; 64], String> {
        self.sign(&KeyPath::Relay, message)
    }
    async fn key(&mut self, path: &KeyPath) -> Result<Pubkey, String> {
        Ok(pk(*path))
    }
    async fn sign_paths(&mut self, paths: &[KeyPath], message: &[u8]) -> Vec<Result<[u8; 64], String>> {
        paths.iter().map(|p| self.sign(p, message)).collect()
    }
}

fn me() -> Pubkey {
    pk(KeyPath::Main)
}

/// Every signature of a wire transaction verified against the key at its index (strict Ed25519); returns the signers in order.
fn verify_all(tx: &[u8]) -> Vec<Pubkey> {
    let (sigs, msg) = soltx::split_tx(tx).unwrap();
    let p = soltx::Parts::parse(msg).unwrap();
    let signers = p.signers();
    assert_eq!(sigs.len(), signers.len());
    for (s, k) in sigs.iter().zip(&signers) {
        let vk = ed25519_dalek::VerifyingKey::from_bytes(k).unwrap();
        vk.verify_strict(msg, &ed25519_dalek::Signature::from_bytes(s)).expect("each signature verifies against its signer");
        assert!(vk.verify(msg, &ed25519_dalek::Signature::from_bytes(s)).is_ok());
    }
    signers
}

fn chain() -> Rc<RefCell<SolChain>> {
    let c = SolChain::new();
    c.borrow_mut().set_account(me(), soltx::SYSTEM_PROGRAM, 10_000_000_000, vec![]);
    c.borrow_mut().advance(25, &[]);
    c
}

/// The intent's money moves: K_int[7] pays into K_coll[3], and K_coll[3] pays out (each a writable signer, like S3's transfer + MintTo).
fn deposit_and_mint() -> Vec<Instruction> {
    vec![soltx::transfer(pk(KeyPath::Intent(7)), pk(KeyPath::Coll(3)), 2_000_000), soltx::transfer(pk(KeyPath::Coll(3)), TO, 1_000_000)]
}

fn nonce_line(c: &Rc<RefCell<SolChain>>, id: LineId) -> Line {
    let n = solsend::nonce_account_of(&me(), &solhome::line_seed(SEED, id)).unwrap();
    c.borrow_mut().set_nonce(n, me());
    Line { nonce_account: Some(n), ..Default::default() }
}

// ---- multi-key signing --------------------------------------------------------------------------------------------------------------

#[test]
fn one_transaction_signed_by_three_canister_keys_verifies_in_the_compiled_signer_order_and_lands_once() {
    let c = chain();
    let mut ps = pair(&c);
    let mut s = RealSigner::new();
    let mut line = nonce_line(&c, LineId::Desk);
    let keys = vec![KeyPath::Intent(7), KeyPath::Coll(3)];
    block_on(solsend::prepare_keys(&mut line, LifetimeKind::DurableNonce, 1, "mint 1 wX for intent 7", deposit_and_mint(), keys.clone(), &mut ps,
        &mut s)).unwrap();
    let saved = line.saved.clone().unwrap();
    assert_eq!(saved.keys, Some(keys.clone()), "kept for a re-sign of the same intent");
    let signers = verify_all(&saved.tx);
    // the fee payer (and nonce authority) first, then the other signers as the compiler sorts them: by key bytes, not by role (S3)
    let mut rest = vec![pk(KeyPath::Intent(7)), pk(KeyPath::Coll(3))];
    rest.sort();
    assert_eq!(signers, [vec![me()], rest].concat());
    // each path signed exactly once, the same message, and the transaction id is K_main's signature
    let calls = s.calls.borrow().clone();
    assert_eq!(calls.len(), 3);
    let mut paths: Vec<KeyPath> = calls.iter().map(|(p, _)| *p).collect();
    paths.sort();
    assert_eq!(paths, vec![KeyPath::Main, KeyPath::Coll(3), KeyPath::Intent(7)]);
    let (sigs, msg) = soltx::split_tx(&saved.tx).unwrap();
    assert!(calls.iter().all(|(_, m)| m == msg));
    assert_eq!(soltx::b58_encode(&sigs[0]), saved.signature);
    assert!(saved.tx.len() <= soltx::PACKET_DATA_SIZE);
    // it lands once, on its nonce
    assert_eq!(block_on(solsend::tick(&mut line, &mut ps, &mut s)).unwrap(), Step::Pending { intent: 1, sends: 1 });
    c.borrow_mut().process(Level::Finalized);
    assert!(matches!(block_on(solsend::tick(&mut line, &mut ps, &mut s)).unwrap(), Step::Landed(d) if d.err.is_none() && d.signature == saved.signature));
    assert_eq!(c.borrow().landed.len(), 1);
}

#[test]
fn a_fallback_re_sign_of_a_multi_key_intent_signs_with_the_same_keys() {
    let c = chain();
    let mut ps = pair(&c);
    let mut s = RealSigner::new();
    let mut line = Line::default();
    let keys = vec![KeyPath::Intent(7), KeyPath::Coll(3)];
    block_on(solsend::prepare_keys(&mut line, LifetimeKind::RecentBlockhash, 4, "mint", deposit_and_mint(), keys, &mut ps, &mut s)).unwrap();
    let first = line.saved.clone().unwrap();
    c.borrow_mut().sent.clear();
    assert!(matches!(block_on(solsend::tick(&mut line, &mut ps, &mut s)).unwrap(), Step::Pending { .. }));
    c.borrow_mut().sent.clear(); // it never reached a leader
    c.borrow_mut().advance(200, &[]);
    let st = block_on(solsend::tick(&mut line, &mut ps, &mut s)).unwrap();
    assert!(matches!(st, Step::Resigned { intent: 4, .. }), "{st:?}");
    let again = line.saved.clone().unwrap();
    assert_ne!(again.signature, first.signature);
    assert_eq!(verify_all(&again.tx).len(), 3, "re-signed by K_main, K_int[7] and K_coll[3]");
    assert_eq!(s.signed(), 6);
}

#[test]
fn a_foreign_signer_a_shared_relay_key_a_foreign_payer_or_an_oversize_transaction_is_refused_before_any_signature() {
    let c = chain();
    let mut ps = pair(&c);
    let mut s = RealSigner::new();
    let mut line = nonce_line(&c, LineId::Desk);
    // K_coll[3] signs, but only K_int[7] was named
    let e = block_on(solsend::prepare_keys(&mut line, LifetimeKind::DurableNonce, 1, "x", deposit_and_mint(), vec![KeyPath::Intent(7)], &mut ps,
        &mut s)).unwrap_err();
    assert!(e.contains("is not a key of this canister"), "{e}");
    // a key nobody here holds
    let stranger = vec![soltx::transfer([0x77; 32], TO, 1)];
    let e = block_on(solsend::prepare_keys(&mut line, LifetimeKind::DurableNonce, 1, "x", stranger, vec![KeyPath::Coll(3)], &mut ps, &mut s))
        .unwrap_err();
    assert!(e.contains("is not a key of this canister"), "{e}");
    // the Relay key never signs with another (A5-03)
    let shared = vec![soltx::transfer(pk(KeyPath::Relay), TO, 1), soltx::transfer(pk(KeyPath::Coll(3)), TO, 1)];
    let e = block_on(solsend::prepare_keys(&mut line, LifetimeKind::DurableNonce, 1, "x", shared, vec![KeyPath::Relay, KeyPath::Coll(3)], &mut ps,
        &mut s)).unwrap_err();
    assert!(e.contains("Relay deposit key signs alone"), "{e}");
    // over 1,232 bytes: refused before the simulation and before any signature
    let mut big = deposit_and_mint();
    big.push(Instruction { program: soltx::key(etch_autobuyer::relaysol::MEMO_V2), accounts: vec![], data: vec![b'x'; 900] });
    let e = block_on(solsend::prepare_keys(&mut line, LifetimeKind::DurableNonce, 1, "x", big, vec![KeyPath::Intent(7), KeyPath::Coll(3)], &mut ps,
        &mut s)).unwrap_err();
    assert!(e.contains("1232"), "{e}");
    assert_eq!((s.signed(), c.borrow().simulated.len()), (0, 0), "nothing simulated, nothing signed");
    assert!(line.saved.is_none() && line.error.is_none());
    // the pure guard: a message whose fee payer is a backing key (K_coll) is refused, whatever else holds
    let msg = soltx::compile_parts(&deposit_and_mint(), pk(KeyPath::Coll(3)), [9; 32]).unwrap().message();
    let held = vec![(KeyPath::Main, me()), (KeyPath::Coll(3), pk(KeyPath::Coll(3))), (KeyPath::Intent(7), pk(KeyPath::Intent(7)))];
    assert!(solkey::signer_paths(&msg, &held).unwrap_err().contains("paid by K_main"));
    let ok = soltx::compile_parts(&deposit_and_mint(), me(), [9; 32]).unwrap().message();
    let paths = solkey::signer_paths(&ok, &held).unwrap();
    assert_eq!(paths[0], KeyPath::Main);
    assert_eq!(paths.len(), 3);
}

#[test]
fn a_simulation_refusal_signs_nothing_and_a_full_signing_queue_saves_nothing_until_the_next_tick() {
    let c = chain();
    let mut ps = pair(&c);
    let mut s = RealSigner::new();
    let mut line = nonce_line(&c, LineId::Pool);
    let keys = vec![KeyPath::Intent(7), KeyPath::Coll(3)];
    c.borrow_mut().sim_err = Some(serde_json::json!({ "InstructionError": [1, { "Custom": 1 }] }));
    let e = block_on(solsend::prepare_keys(&mut line, LifetimeKind::DurableNonce, 1, "x", deposit_and_mint(), keys.clone(), &mut ps, &mut s))
        .unwrap_err();
    assert!(e.starts_with(solsend::SIMULATION_REFUSED), "{e}");
    assert_eq!(s.signed(), 0);
    c.borrow_mut().sim_err = None;
    line.error = None;
    // the IC refuses at once over its per-key queue (S3 row 9): nothing is saved, the line is as it was, no line error (no alert)
    *s.full.borrow_mut() = true;
    let before = line.clone();
    let e = block_on(solsend::prepare_keys(&mut line, LifetimeKind::DurableNonce, 1, "x", deposit_and_mint(), keys.clone(), &mut ps, &mut s))
        .unwrap_err();
    assert!(e.starts_with(solkey::QUEUE_FULL) && solkey::is_queue_full(&e), "{e}");
    assert_eq!(line, before);
    assert!(c.borrow().sent.is_empty());
    // the next tick: the queue has room, the same intent is signed
    *s.full.borrow_mut() = false;
    block_on(solsend::prepare_keys(&mut line, LifetimeKind::DurableNonce, 1, "x", deposit_and_mint(), keys, &mut ps, &mut s)).unwrap();
    assert_eq!(verify_all(&line.saved.unwrap().tx).len(), 3);
}

// ---- the four lines -----------------------------------------------------------------------------------------------------------------

struct World {
    cfg: SolHomeCfg,
    ps: Vec<FakeSol>,
    signer: RealSigner,
    lines: BTreeMap<LineId, SolLine>,
    chain: Rc<RefCell<SolChain>>,
    now: u64,
}

impl SolIo for World {
    fn sol_home(&self) -> Option<SolHomeCfg> {
        Some(self.cfg.clone())
    }
    async fn sol_me(&mut self) -> Result<Pubkey, String> {
        Ok(me())
    }
    async fn sol_accounts(&mut self, keys: &[Pubkey], max_data: usize) -> Result<Vec<Option<Account>>, String> {
        Ok(solrpc::accounts(&mut self.ps, keys, max_data).await?)
    }
    async fn sol_rent(&mut self, space: u64) -> Result<u64, String> {
        Ok(solrpc::rent_exempt(&mut self.ps, space).await?)
    }
    async fn sol_relay(&mut self) -> Result<Pubkey, String> {
        Ok(pk(KeyPath::Relay))
    }
    async fn sol_balance_of(&mut self, key: &Pubkey) -> Result<u128, String> {
        Ok(solrpc::balance(&mut self.ps, key).await? as u128)
    }
    async fn sol_prepare(&mut self, line: &mut Line, kind: LifetimeKind, intent: u64, what: &str, ixs: Vec<Instruction>) -> Result<(), String> {
        solsend::prepare(line, kind, intent, what, ixs, &mut self.ps, &mut self.signer).await
    }
    async fn sol_prepare_keys(&mut self, line: &mut Line, kind: LifetimeKind, intent: u64, what: &str, ixs: Vec<Instruction>,
        keys: Vec<KeyPath>) -> Result<(), String> {
        solsend::prepare_keys(line, kind, intent, what, ixs, keys, &mut self.ps, &mut self.signer).await
    }
    async fn sol_tick(&mut self, line: &mut Line) -> Result<Step, String> {
        Ok(solsend::tick(line, &mut self.ps, &mut self.signer).await?)
    }
    fn sol_line(&mut self) -> SolLine {
        self.sol_line_of(LineId::Money)
    }
    fn sol_line_save(&mut self, l: &SolLine) {
        self.sol_line_save_of(LineId::Money, l)
    }
    fn sol_line_of(&mut self, id: LineId) -> SolLine {
        self.lines.get(&id).cloned().unwrap_or_default()
    }
    fn sol_line_save_of(&mut self, id: LineId, l: &SolLine) {
        self.lines.insert(id, l.clone());
    }
}

fn world(lifetime: LifetimeKind) -> World {
    let chain = chain();
    let cfg = SolHomeCfg { lifetime, nonce_seed: SEED.into() };
    let now = chain.borrow().unix;
    World { cfg, ps: pair(&chain), signer: RealSigner::new(), lines: BTreeMap::new(), chain, now }
}

impl World {
    /// The chain lands and finalizes everything sent so far.
    fn settle(&mut self) {
        self.chain.borrow_mut().advance(40, &[]);
        self.chain.borrow_mut().process(Level::Finalized);
    }
    fn pass(&mut self, cursor: Option<u8>) -> (Vec<String>, u8, bool) {
        let now = self.now;
        block_on(solhome::tick_lines(self, cursor, now))
    }
    fn line(&self, id: LineId) -> SolLine {
        self.lines.get(&id).cloned().unwrap_or_default()
    }
    fn open_nonces(&mut self) {
        for id in LineId::ALL {
            let l = nonce_line(&self.chain, id);
            self.lines.insert(id, SolLine { line: l, ..Default::default() });
        }
    }
}

/// A step of each line, with its keys: the money line a custodian's record, the crank line a treasury step, the desks an `Op`.
fn step_of(id: LineId, n: u64) -> (LineStep, Vec<Instruction>, Vec<KeyPath>) {
    match id {
        LineId::Money => (LineStep::Custody(CustodyStep::Mark { ticket: n }), vec![soltx::transfer(me(), TO, 1_000 + n)], vec![]),
        LineId::Crank => (LineStep::Treasury(TreasuryStep::Ops { to: TO, lamports: n }), vec![soltx::transfer(me(), TO, 2_000 + n)], vec![]),
        LineId::Desk => (LineStep::Op(LineOp { owner: LineOwner::Desk, what: "mint for intent".into(), id: n }),
            vec![soltx::transfer(pk(KeyPath::Intent(n)), pk(KeyPath::Coll(1)), 3_000 + n), soltx::transfer(pk(KeyPath::Coll(1)), TO, 1)],
            vec![KeyPath::Intent(n), KeyPath::Coll(1)]),
        LineId::Pool => (LineStep::Op(LineOp { owner: LineOwner::Pool, what: "re-range".into(), id: n }),
            vec![soltx::transfer(pk(KeyPath::Desk(1)), TO, 4_000 + n)], vec![KeyPath::Desk(1)]),
    }
}

fn sign_on(w: &mut World, id: LineId, n: u64) -> Result<(String, solhome::LinePending), String> {
    let (step, ixs, keys) = step_of(id, n);
    let now = w.now;
    block_on(solhome::sign_keys(w, step, now, ixs, keys))
}

#[test]
fn each_line_opens_its_own_durable_nonce_and_none_waits_for_another() {
    let mut w = world(LifetimeKind::DurableNonce);
    // the desk asks first: its line opens its OWN nonce account (one signature, K_main alone, on a recent blockhash) and the step waits
    let now = w.now;
    let e = block_on(solhome::ready(&mut w, LineOwner::Desk, now, "mint")).unwrap_err();
    assert!(e.contains("desk line is opening its durable-nonce account"), "{e}");
    let e = block_on(solhome::ready(&mut w, LineOwner::Pool, now, "re-range")).unwrap_err();
    assert!(e.contains("pool line is opening"), "{e}");
    assert!(w.line(LineId::Money).line.saved.is_none(), "the money line is untouched");
    w.settle();
    let (out, _, _) = w.pass(None);
    assert_eq!(out.iter().filter(|t| t.contains("durable-nonce account") && t.contains("is open")).count(), 2, "{out:?}");
    let (d, p) = (w.line(LineId::Desk).line.nonce_account.unwrap(), w.line(LineId::Pool).line.nonce_account.unwrap());
    assert_eq!(d, solsend::nonce_account_of(&me(), &format!("desk:{SEED}")).unwrap());
    assert_eq!(p, solsend::nonce_account_of(&me(), &format!("pool:{SEED}")).unwrap());
    assert_ne!(d, solsend::nonce_account_of(&me(), SEED).unwrap(), "the money line keeps the install's seed");
    // K_main alone signed both openings (the fake chain does not run the System Program: the account itself is not built here)
    assert!(w.signer.calls.borrow().iter().all(|(p, _)| *p == KeyPath::Main));
    // the seeds: the money line's is the install's; the others are cut to create_with_seed's 32 bytes
    assert_eq!(solhome::line_seed(SEED, LineId::Money), SEED);
    let long = "x".repeat(32);
    assert_eq!(solhome::line_seed(&long, LineId::Crank).len(), 32);
    assert_ne!(solhome::line_seed(&long, LineId::Crank), solhome::line_seed(&long, LineId::Pool));
}

#[test]
fn four_steps_out_at_once_land_each_on_its_own_line_and_nonce_with_outcomes_kept_per_line() {
    let mut w = world(LifetimeKind::DurableNonce);
    w.open_nonces();
    for id in LineId::ALL {
        let (text, p) = sign_on(&mut w, id, 7).unwrap();
        assert!(text.contains(&format!("on the Solana {} line", id.tag())), "{text}");
        assert_eq!(LineId::of(p.step.owner()), id);
    }
    // all four in flight at once; a second step on a busy line is refused, on that line only
    assert!(LineId::ALL.iter().all(|id| w.line(*id).busy()));
    assert!(sign_on(&mut w, LineId::Desk, 8).unwrap_err().contains("desk line is busy"));
    let nonces: Vec<_> = LineId::ALL.iter().map(|id| w.line(*id).line.nonce_account.unwrap()).collect();
    let mut uniq = nonces.clone();
    uniq.sort();
    uniq.dedup();
    assert_eq!(uniq.len(), 4, "four nonce accounts");
    // each signed transaction carries its own line's nonce and its own keys' signatures
    for id in LineId::ALL {
        let s = w.line(id).line.saved.unwrap();
        let signers = verify_all(&s.tx);
        assert_eq!(signers[0], me());
        assert_eq!(signers.len(), 1 + step_of(id, 7).2.len());
    }
    w.settle();
    let (out, _, busy) = w.pass(None);
    assert_eq!(out.len(), 4, "{out:?}");
    assert!(!busy, "nothing out any more");
    for id in LineId::ALL {
        let mut l = w.line(id);
        assert!(!l.busy());
        let owner = step_of(id, 7).0.owner();
        let got = l.take(owner);
        assert_eq!(got.len(), 1, "{id:?}: its outcome, on its own line");
        assert_eq!(got[0].ok, Some(true));
        assert!(l.outcomes.is_empty(), "no other line's outcome here");
    }
    assert_eq!(w.chain.borrow().landed.len(), 4);
}

#[test]
fn an_incident_on_one_line_stops_that_line_only() {
    let mut w = world(LifetimeKind::DurableNonce);
    w.open_nonces();
    for id in [LineId::Desk, LineId::Pool, LineId::Money] {
        sign_on(&mut w, id, 3).unwrap();
    }
    // the desk line's nonce moves under its pending transaction (a key used elsewhere, a bug): only the desk sees an incident
    let n = w.line(LineId::Desk).line.nonce_account.unwrap();
    w.chain.borrow_mut().sent.retain(|tx| {
        let (_, m) = soltx::split_tx(tx).unwrap();
        !soltx::Parts::parse(m).unwrap().keys.contains(&n)
    });
    w.chain.borrow_mut().move_nonce(&n);
    w.settle();
    let (out, _, busy) = w.pass(None);
    assert!(out.iter().any(|t| t.starts_with("sol line [desk]: INCIDENT")), "{out:?}");
    assert!(out.iter().any(|t| t.starts_with("sol line [pool]:") && t.contains("landed (success")), "{out:?}");
    assert!(out.iter().any(|t| t.starts_with("sol line: ") && t.contains("landed (success")), "{out:?}");
    assert!(!busy, "an open incident is the operator's, not the timer's: the idle cadence");
    // the desk refuses new work; the pool and the money line take theirs
    let now = w.now;
    assert!(solhome::turn(&mut w, LineOwner::Desk, now).unwrap_err().contains("desk line has an open INCIDENT"));
    assert!(solhome::turn(&mut w, LineOwner::Pool, now).is_ok());
    sign_on(&mut w, LineId::Pool, 4).unwrap();
    sign_on(&mut w, LineId::Money, 4).unwrap();
    // the operator, having looked (no signature known), clears it: the desk's step comes back to its flow at its next tick
    let mut d = w.line(LineId::Desk);
    solhome::clear_incident(&mut d, false).unwrap();
    w.lines.insert(LineId::Desk, d);
    w.settle();
    let (out, _, _) = w.pass(None);
    assert!(out.iter().any(|t| t.starts_with("sol line [desk]:") && t.contains("abandoned after an incident")), "{out:?}");
    let mut d = w.line(LineId::Desk);
    assert_eq!(d.take(LineOwner::Desk)[0].ok, None, "taken back unlanded");
    assert!(!d.busy());
}

#[test]
fn per_line_resend_and_expiry_under_the_fallback_never_cross_lines() {
    let mut w = world(LifetimeKind::RecentBlockhash);
    sign_on(&mut w, LineId::Desk, 1).unwrap();
    // the desk's bytes never reach a leader; the pool signs later
    w.chain.borrow_mut().sent.clear();
    w.chain.borrow_mut().advance(200, &[]);
    sign_on(&mut w, LineId::Pool, 1).unwrap();
    let pool_sig = w.line(LineId::Pool).line.saved.unwrap().signature;
    let desk_first = w.line(LineId::Desk).line.saved.unwrap().signature;
    let (out, _, busy) = w.pass(None);
    assert!(out.iter().any(|t| t.starts_with("sol line [desk]:") && t.contains("re-signed")), "{out:?}");
    assert!(out.iter().any(|t| t.starts_with("sol line [pool]:") && t.contains("sent")), "{out:?}");
    assert!(busy);
    let desk = w.line(LineId::Desk).line.saved.unwrap();
    assert_ne!(desk.signature, desk_first);
    assert_eq!(desk.keys, Some(vec![KeyPath::Intent(1), KeyPath::Coll(1)]));
    assert_eq!(w.line(LineId::Pool).line.saved.unwrap().signature, pool_sig, "the pool's intent is untouched");
    // the re-signed desk transaction goes out, both land, each settles on its own line
    w.pass(None);
    w.settle();
    let (out, _, busy) = w.pass(None);
    assert_eq!(out.iter().filter(|t| t.contains("landed (success")).count(), 2, "{out:?}");
    assert!(!busy);
    let sent_once = |sig: &str| w.chain.borrow().landed.keys().filter(|k| k.as_str() == sig).count();
    assert_eq!(sent_once(&pool_sig), 1);
    assert_eq!(sent_once(&desk_first), 0, "the expired desk bytes never landed");
}

#[test]
fn the_fair_pass_rotates_and_a_full_signing_queue_defers_the_rest_to_the_front_of_the_next() {
    use LineId::*;
    assert_eq!(solhome::pass_order(None), [Money, Crank, Desk, Pool]);
    assert_eq!(solhome::pass_order(Some(2)), [Desk, Pool, Money, Crank]);
    assert_eq!(solhome::pass_order(Some(6)), [Desk, Pool, Money, Crank], "the cursor wraps");
    // a full pass moves the start one place; a stopped pass starts next at the first line it did not reach
    assert_eq!(solhome::next_cursor(&solhome::pass_order(Some(3)), None), 0);
    assert_eq!(solhome::next_cursor(&solhome::pass_order(Some(1)), None), 2);
    assert_eq!(solhome::next_cursor(&solhome::pass_order(Some(1)), Some(2)), 3, "Crank, Desk served; Pool goes first next");
    assert_eq!(solhome::next_cursor(&solhome::pass_order(Some(0)), Some(4)), 1);
    // the world: four lines whose intents must be re-signed (expired under the fallback) while the signing queue is full
    let mut w = world(LifetimeKind::RecentBlockhash);
    for id in LineId::ALL {
        sign_on(&mut w, id, 2).unwrap();
    }
    w.chain.borrow_mut().sent.clear();
    w.chain.borrow_mut().advance(200, &[]);
    let before: Vec<_> = LineId::ALL.iter().map(|id| w.line(*id)).collect();
    *w.signer.full.borrow_mut() = true;
    let (out, cursor, busy) = w.pass(Some(1));
    assert!(out[0].starts_with("sol line [crank]: waits") && solkey::is_queue_full(&out[0]), "{out:?}");
    assert!(out[1].contains("signing queue is full: desk, pool, money wait"), "{out:?}");
    assert_eq!(out.len(), 2);
    assert_eq!(cursor, 2, "the desk goes first next pass");
    assert!(busy);
    // nothing changed on any line (a full queue is retry-next-tick)
    let after: Vec<_> = LineId::ALL.iter().map(|id| w.line(*id)).collect();
    assert_eq!(before, after);
    // the queue has room: the next pass, from the desk, re-signs all four
    *w.signer.full.borrow_mut() = false;
    let (out, cursor, _) = w.pass(Some(cursor));
    assert_eq!(out.iter().filter(|t| t.contains("re-signed")).count(), 4, "{out:?}");
    assert!(out[0].starts_with("sol line [desk]:"), "{out:?}");
    assert_eq!(cursor, 3);
}

#[test]
fn an_op_step_belongs_to_a_desk_and_a_relay_deposit_never_shares_its_signature() {
    let mut w = world(LifetimeKind::DurableNonce);
    w.open_nonces();
    let now = w.now;
    let bad = LineStep::Op(LineOp { owner: LineOwner::Eth, what: "x".into(), id: 1 });
    let e = block_on(solhome::sign_keys(&mut w, bad, now, vec![soltx::transfer(me(), TO, 1)], vec![])).unwrap_err();
    assert!(e.contains("an Op step is the desks'"), "{e}");
    // a Relay deposit is the money line's, signed by the Relay key alone (A5-03): never with extra keys
    let relay = pk(KeyPath::Relay);
    let dep = Instruction { program: soltx::key(etch_autobuyer::relaysol::DEPOSITORY_PROGRAM),
        accounts: etch_autobuyer::relaysol::deposit_native_metas(&relay), data: etch_autobuyer::relaysol::encode_deposit_native(5_000_000, &[3; 32]) };
    let step = LineStep::Custody(CustodyStep::Mark { ticket: 1 });
    let e = block_on(solhome::sign_keys(&mut w, step, now, vec![dep], vec![KeyPath::Coll(1)])).unwrap_err();
    assert!(e.contains("Relay key alone"), "{e}");
    assert!(LineId::ALL.iter().all(|id| !w.line(*id).busy()));
}

// ---- the upgrade ----------------------------------------------------------------------------------------------------------------------

/// A state saved before W3: the 14-element tuple, whose one line carried the registry keeper's crank state (`crank`, retired with
/// rook_core: decision D-l) and a desk's turn in its queue. The old shape is encoded here with the field (a record the build no longer
/// has: candid drops it on decode, and `lost_opts` reports nothing for it): old bytes decode into this build's `SolLine`, and
/// `post_upgrade`'s move gives four lines. A line with a keeper's crank step still in flight (`LineStep::Crank`) is the one old encoding
/// that no longer decodes: none exists on the books home, where the keeper never ran.
#[test]
fn a_pre_w3_state_restores_into_four_lines_and_a_saved_keepers_state_is_dropped() {
    use etch_autobuyer::canister::{lost_opts, Saved};
    /// The keeper's state as the pre-W3 build saved it (the fields the monitor read; the rest were vectors, empty here).
    #[derive(candid::CandidType, serde::Deserialize, Clone, Debug, PartialEq)]
    struct OldCrankState { day: u64, signed_today: u32, opened_today: u32, last_pass: u64, drand_disagreement: Option<(u64, u64, String)> }
    #[derive(candid::CandidType, serde::Deserialize, Clone, Debug, PartialEq)]
    struct OldSolLine { line: Line, pending: Option<solhome::LinePending>, next_intent: u64, opening: Option<u64>, deposits: Vec<solhome::SolDepositRec>,
        outcomes: Vec<solhome::LineOutcome>, queue: Vec<(LineOwner, u64)>, relay: Option<solhome::RelayLeg>, crank: Option<OldCrankState> }
    let mut w = world(LifetimeKind::DurableNonce);
    w.open_nonces();
    sign_on(&mut w, LineId::Money, 9).unwrap();
    let money_then = w.line(LineId::Money);
    let saved_line = OldSolLine { line: money_then.line.clone(), pending: money_then.pending.clone(), next_intent: money_then.next_intent,
        opening: money_then.opening, deposits: vec![], outcomes: vec![], queue: vec![(LineOwner::Desk, 5), (LineOwner::Eth, 6)], relay: None,
        crank: Some(OldCrankState { day: 20_735, signed_today: 5, opened_today: 1, last_pass: 1_791_565_840, drand_disagreement: Some((20_025_637, 1_791_565_720, "the relays disagree".into())) }) };
    let bytes = candid::encode_one(Some(saved_line.clone())).unwrap();
    let old: Option<SolLine> = candid::decode_one(&bytes).expect("the pre-W3 bytes decode into this build's SolLine: the keeper's state dropped");
    let old = old.expect("a line");
    assert_eq!((old.line.clone(), old.pending.clone(), old.next_intent, old.queue.clone()),
        (saved_line.line.clone(), saved_line.pending.clone(), saved_line.next_intent, saved_line.queue.clone()));
    // the tuple as the pre-W3 build saved it: fourteen elements
    type SavedB2 = (Option<etch_autobuyer::canister::Init>, Vec<etch_autobuyer::state::Purchase>, bool, Vec<String>,
        Option<etch_autobuyer::redeem::Custody>, Option<etch_autobuyer::chains::NftState>, Option<etch_autobuyer::frogflow::FrogState>,
        Option<etch_autobuyer::floortrigger::FloorState>, Option<(etch_autobuyer::custodycap::Book, etch_autobuyer::outflow::Outflow)>,
        Option<etch_autobuyer::handin::HandinState>, Option<etch_autobuyer::sale::SaleState>, Option<etch_autobuyer::custody::FeePool>,
        Option<etch_autobuyer::ethflow::EthState>, Option<SolLine>);
    let b2: SavedB2 = (None, vec![], false, vec![], None, None, None, None, None, None, None, None, None, Some(old.clone()));
    let stable = candid::encode_args((b2,)).unwrap();
    let (saved,): (Saved,) = candid::decode_args(&stable).unwrap();
    assert!(saved.14.is_none(), "no fifteenth element in a pre-W3 state");
    assert!(lost_opts(&stable, &candid::encode_args((saved.clone(),)).unwrap()).unwrap().is_empty(), "nothing saved restores as nothing");
    // post_upgrade's move
    let (money, lines) = solhome::migrate_lines(saved.13, saved.14).unwrap();
    let money = money.unwrap();
    assert_eq!(money.queue, vec![(LineOwner::Eth, 6)], "another line's turn leaves the money line's queue");
    assert_eq!((money.line.clone(), money.next_intent, money.pending.clone()), (old.line.clone(), old.next_intent, old.pending.clone()),
        "the money line's nonce, bytes, history and step are as saved");
    assert!(lines.crank.is_none() && lines.desk.is_none() && lines.pool.is_none() && lines.next.is_none());
    // once: a state saved by this build moves nothing
    let again = solhome::migrate_lines(Some(money.clone()), Some(lines.clone())).unwrap();
    assert_eq!(again, (Some(money.clone()), lines.clone()));
    // the fifteen-element tuple this build saves round-trips and loses nothing
    let w3: Saved = (None, vec![], false, vec![], None, None, None, None, None, None, None, None, None, Some(money.clone()), Some(lines.clone()));
    let b = candid::encode_args((w3,)).unwrap();
    let (back,): (Saved,) = candid::decode_args(&b).unwrap();
    assert!(lost_opts(&b, &candid::encode_args((back.clone(),)).unwrap()).unwrap().is_empty());
    assert_eq!((back.13, back.14), (Some(money.clone()), Some(lines.clone())));
    assert_eq!(solhome::migrate_lines(None, None).unwrap(), (None, SolLines::default()));
}

/// The `sol_lines()` reply the monitor decodes (`monitor/test/sol_lines.test.mjs`): four lines (an incident on the desk, an
/// `insufficient_funds` error on the pool, a step out on the money line, the crank line never used) and the timer.
#[test]
fn the_sol_lines_reply_is_what_the_monitor_decodes() {
    let mut w = world(LifetimeKind::DurableNonce);
    w.open_nonces();
    for id in [LineId::Money, LineId::Desk, LineId::Pool] {
        sign_on(&mut w, id, 9).unwrap();
    }
    let mut desk = w.line(LineId::Desk);
    desk.line.incident = Some("the nonce account moved under intent 0 (desk: mint for intent (9))".into());
    let mut pool = w.line(LineId::Pool);
    pool.line.error = Some("intent 0 (pool: re-range (9)): every provider's preflight refused it: insufficient_funds".into());
    let crank = SolLine::default();
    let mut money = w.line(LineId::Money);
    if let Some(p) = money.pending.as_mut() {
        p.at = 1_791_560_000;
    }
    let lines: Vec<(LineId, Option<SolLine>)> = vec![(LineId::Money, Some(solhome::public_view(money))), (LineId::Crank, Some(crank)),
        (LineId::Desk, Some(solhome::public_view(desk))), (LineId::Pool, Some(solhome::public_view(pool)))];
    let timer = Some(solhome::TimerSeen { last_run: 1_791_565_800, next_at: 1_791_565_810, runs: 42, skipped: 1, busy: true });
    let bytes = hex::encode(candid::encode_args((lines, timer, false)).unwrap());
    if std::env::var("WRITE_FIXTURES").is_ok() {
        std::fs::write(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/sol-lines.hex"), format!("{bytes}\n")).unwrap();
    }
    assert_eq!(bytes, include_str!("fixtures/sol-lines.hex").trim(), "tests/fixtures/sol-lines.hex is stale (WRITE_FIXTURES=1), then run the \
        monitor's tests");
}

#[allow(dead_code)]
fn _uses(_: AccountMeta) {}
