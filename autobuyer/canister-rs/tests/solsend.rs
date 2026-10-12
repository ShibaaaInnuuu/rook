//! The Solana send line end to end against the fake provider pair (docs/v9/solana-migration.md §2.4, spike Q3 §4 and §7): a
//! durable-nonce transaction lands; the SAME bytes are resent and exactly one lands; a blockhash transaction expires, is re-signed
//! under the same intent and lands once; a nonce moved under a pending transaction is an incident with no re-sign; the ordering
//! race of "can never land" is closed; disagreement and failed outcalls change nothing.
mod fakerpc;

use std::future::Future;
use std::pin::pin;
use std::task::{Context, Poll, Waker};

use etch_autobuyer::solrpc::{Level, Refused};
use etch_autobuyer::solsend::{self, Line, LifetimeKind, Step};
use etch_autobuyer::soltx::{self, Instruction, Pubkey};
use fakerpc::sol::{pair, FakeSigner, SolChain};

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

fn intent(lamports: u64) -> Vec<Instruction> {
    vec![soltx::transfer(ME, TO, lamports)]
}

/// A chain with the line's nonce account (authority = us), the pair, a signer and a durable-nonce line.
fn setup() -> (std::rc::Rc<std::cell::RefCell<SolChain>>, Vec<fakerpc::sol::FakeSol>, FakeSigner, Line, Pubkey) {
    let chain = SolChain::new();
    // the fee payer exists (phase B2: every transaction is simulated before it is signed, and a payer with nothing is refused)
    chain.borrow_mut().set_account(ME, soltx::SYSTEM_PROGRAM, 10_000_000_000, vec![]);
    let nonce = solsend::nonce_account_of(&ME, SEED).unwrap();
    chain.borrow_mut().set_nonce(nonce, ME);
    let ps = pair(&chain);
    let line = Line { nonce_account: Some(nonce), ..Default::default() };
    (chain, ps, FakeSigner::new(ME), line, nonce)
}

#[test]
fn a_durable_nonce_transaction_is_saved_before_its_first_send_and_lands() {
    let (chain, mut ps, mut signer, mut line, nonce) = setup();
    let n0 = chain.borrow().nonce_of(&nonce).durable_nonce;
    block_on(solsend::prepare(&mut line, LifetimeKind::DurableNonce, 7, "transfer 1", intent(1_000_000), &mut ps, &mut signer)).unwrap();
    // saved, signed once, NOT sent
    let s = line.saved.clone().unwrap();
    assert_eq!((s.sends, s.history.len(), s.recent_blockhash, s.intent), (0, 1, n0, 7));
    assert!(chain.borrow().sent.is_empty(), "prepare never sends");
    assert_eq!(signer.signed.borrow().len(), 1);
    // the bytes: AdvanceNonceAccount first, the nonce as the blockhash, one signer (us), under 1,232
    let (sigs, msg) = soltx::split_tx(&s.tx).unwrap();
    let p = soltx::Parts::parse(msg).unwrap();
    assert_eq!((sigs.len(), p.signers(), p.blockhash), (1, vec![ME], n0));
    let first = &p.decompile(&[], &[]).unwrap()[0];
    let adv = soltx::advance_nonce(nonce, ME);
    assert_eq!((first.program, &first.data), (adv.program, &adv.data));
    assert_eq!(first.accounts.iter().map(|a| a.key).collect::<Vec<_>>(), adv.accounts.iter().map(|a| a.key).collect::<Vec<_>>());
    assert_eq!(soltx::b58_encode(&sigs[0]), s.signature);
    // tick 1: the first send
    assert_eq!(block_on(solsend::tick(&mut line, &mut ps, &mut signer)).unwrap(), Step::Pending { intent: 7, sends: 1 });
    assert_eq!(chain.borrow().sent.len(), 2, "both providers are sent the bytes");
    // a second intent is refused while this one is in flight (one per nonce account)
    let e = block_on(solsend::prepare(&mut line, LifetimeKind::DurableNonce, 8, "transfer 2", intent(1), &mut ps, &mut signer)).unwrap_err();
    assert!(e.contains("one transaction per line"), "{e}");
    // it lands (confirmed first: not settled yet)
    assert_eq!(chain.borrow_mut().process(Level::Confirmed).len(), 1);
    assert!(matches!(block_on(solsend::tick(&mut line, &mut ps, &mut signer)).unwrap(), Step::Pending { .. }));
    chain.borrow_mut().finalize_all();
    let st = block_on(solsend::tick(&mut line, &mut ps, &mut signer)).unwrap();
    assert_eq!(st, Step::Landed(solsend::Done { intent: 7, what: "transfer 1".into(), signature: s.signature.clone(),
        history: vec![s.signature.clone()], err: None, abandoned: None }));
    assert!(line.saved.is_none() && line.done.len() == 1);
    assert_ne!(chain.borrow().nonce_of(&nonce).durable_nonce, n0, "the transaction advanced its nonce");
    // the line is free: the next intent signs on the NEW nonce
    block_on(solsend::prepare(&mut line, LifetimeKind::DurableNonce, 8, "transfer 2", intent(2), &mut ps, &mut signer)).unwrap();
    assert_eq!(line.saved.as_ref().unwrap().recent_blockhash, chain.borrow().nonce_of(&nonce).durable_nonce);
    assert_eq!(solsend::sol_seen(&[&line]).len(), 2);
}

#[test]
fn the_same_bytes_are_resent_and_exactly_one_lands() {
    let (chain, mut ps, mut signer, mut line, _) = setup();
    block_on(solsend::prepare(&mut line, LifetimeKind::DurableNonce, 1, "transfer", intent(1_000_000), &mut ps, &mut signer)).unwrap();
    for i in 1..=4 {
        assert_eq!(block_on(solsend::tick(&mut line, &mut ps, &mut signer)).unwrap(), Step::Pending { intent: 1, sends: i });
    }
    {
        let c = chain.borrow();
        assert_eq!(c.sent.len(), 8, "4 sends × 2 providers");
        assert_eq!(c.distinct_sent(), 1, "every send is the SAME bytes");
    }
    assert_eq!(signer.signed.borrow().len(), 1, "signed once");
    // every copy reaches the chain; one lands, the rest are dead (the nonce moved)
    assert_eq!(chain.borrow_mut().process(Level::Finalized).len(), 1);
    assert_eq!(chain.borrow_mut().process(Level::Finalized).len(), 0);
    assert!(matches!(block_on(solsend::tick(&mut line, &mut ps, &mut signer)).unwrap(), Step::Landed(d) if d.err.is_none()));
    assert_eq!(chain.borrow().landed.len(), 1);
}

#[test]
fn a_transaction_that_lands_and_fails_settles_with_its_error() {
    let (chain, mut ps, mut signer, mut line, _) = setup();
    block_on(solsend::prepare(&mut line, LifetimeKind::DurableNonce, 3, "transfer", intent(1), &mut ps, &mut signer)).unwrap();
    block_on(solsend::tick(&mut line, &mut ps, &mut signer)).unwrap();
    chain.borrow_mut().fail_next = Some(serde_json::json!({ "InstructionError": [1, { "Custom": 6003 }] }));
    chain.borrow_mut().process(Level::Finalized);
    let Step::Landed(d) = block_on(solsend::tick(&mut line, &mut ps, &mut signer)).unwrap() else { panic!() };
    assert!(d.err.unwrap().contains("6003"));
    assert!(line.saved.is_none(), "settled: the intent did not happen, the caller retries under a new intent");
}

#[test]
fn a_nonce_moved_under_a_pending_transaction_is_an_incident_never_a_re_sign() {
    let (chain, mut ps, mut signer, mut line, nonce) = setup();
    block_on(solsend::prepare(&mut line, LifetimeKind::DurableNonce, 5, "post_floor", intent(1), &mut ps, &mut signer)).unwrap();
    block_on(solsend::tick(&mut line, &mut ps, &mut signer)).unwrap();
    chain.borrow_mut().move_nonce(&nonce); // not our transaction
    assert_eq!(chain.borrow_mut().process(Level::Finalized).len(), 0, "the saved bytes can never land now");
    let st = block_on(solsend::tick(&mut line, &mut ps, &mut signer)).unwrap();
    let Step::Incident(why) = st else { panic!("{st:?}") };
    assert!(why.contains("no automatic re-sign"), "{why}");
    let sends = chain.borrow().sent.len();
    // the incident holds: no re-sign, no send, no new intent
    for _ in 0..3 {
        assert!(matches!(block_on(solsend::tick(&mut line, &mut ps, &mut signer)).unwrap(), Step::Incident(_)));
    }
    assert_eq!((signer.signed.borrow().len(), chain.borrow().sent.len()), (1, sends));
    assert!(block_on(solsend::prepare(&mut line, LifetimeKind::DurableNonce, 6, "x", intent(1), &mut ps, &mut signer)).unwrap_err().contains("incident"));
    // an operator clears it: the intent comes back to be prepared again under a fresh read of the nonce
    let abandoned = solsend::clear_incident(&mut line).unwrap();
    assert_eq!(abandoned.intent, 5);
    block_on(solsend::prepare(&mut line, LifetimeKind::DurableNonce, abandoned.intent, &abandoned.what, abandoned.instructions, &mut ps, &mut signer)).unwrap();
    block_on(solsend::tick(&mut line, &mut ps, &mut signer)).unwrap();
    assert_eq!(chain.borrow_mut().process(Level::Finalized).len(), 1);
    assert!(matches!(block_on(solsend::tick(&mut line, &mut ps, &mut signer)).unwrap(), Step::Landed(_)));
}

/// Q3 §4.3: the transaction lands between the status read and the nonce read. Read in that order alone, "status unknown + nonce
/// moved" would be an incident (or, under the fallback, a second valid signature). The nonce is read FIRST inside
/// `can_never_land`, and the status re-read SECOND sees the landing.
#[test]
fn a_landing_between_the_reads_is_not_mistaken_for_a_moved_nonce() {
    let (_chain, mut ps, mut signer, mut line, nonce) = setup();
    block_on(solsend::prepare(&mut line, LifetimeKind::DurableNonce, 9, "transfer", intent(1), &mut ps, &mut signer)).unwrap();
    block_on(solsend::tick(&mut line, &mut ps, &mut signer)).unwrap();
    let key = format!("getAccountInfo:{}", soltx::b58_encode(&nonce));
    let fired = std::rc::Rc::new(std::cell::Cell::new(false));
    let f = fired.clone();
    let nonce_b58 = soltx::b58_encode(&nonce);
    *ps[0].hook.borrow_mut() = Some(Box::new(move |r, c| {
        if !f.get() && r.method == "getAccountInfo" && r.params[0] == nonce_b58.as_str() {
            c.process(Level::Finalized); // the landing, just before the nonce read
            f.set(true);
        }
    }));
    ps[0].asked.borrow_mut().clear();
    let st = block_on(solsend::tick(&mut line, &mut ps, &mut signer)).unwrap();
    assert!(fired.get());
    assert!(matches!(st, Step::Pending { .. }), "not an incident: {st:?}");
    assert!(line.incident.is_none());
    // the order of the reads: statuses, then the nonce (FIRST of can_never_land), then the statuses again (SECOND)
    let asked = ps[0].asked.borrow().clone();
    let at = |m: &str, from: usize| asked.iter().skip(from).position(|x| x.starts_with(m)).map(|i| i + from).unwrap();
    let s1 = at("getSignatureStatuses", 0);
    let n = asked.iter().position(|x| *x == key).unwrap();
    let s2 = at("getSignatureStatuses", n);
    assert!(s1 < n && n < s2, "{asked:?}");
    assert!(matches!(block_on(solsend::tick(&mut line, &mut ps, &mut signer)).unwrap(), Step::Landed(_)));
    assert_eq!(signer.signed.borrow().len(), 1);
}

#[test]
fn under_the_fallback_a_proven_expiry_re_signs_the_same_intent_and_one_lands() {
    let chain = SolChain::new();
    // the fee payer exists (phase B2: every transaction is simulated before it is signed, and a payer with nothing is refused)
    chain.borrow_mut().set_account(ME, soltx::SYSTEM_PROGRAM, 10_000_000_000, vec![]);
    chain.borrow_mut().advance(30, &[1_020, 1_019]); // the rounded slot (1,020) is skipped: walk back to 1,018
    let mut ps = pair(&chain);
    let mut signer = FakeSigner::new(ME);
    let mut line = Line::default();
    block_on(solsend::prepare(&mut line, LifetimeKind::RecentBlockhash, 11, "transfer", intent(5), &mut ps, &mut signer)).unwrap();
    let s = line.saved.clone().unwrap();
    let (b, h) = chain.borrow().blocks[&1_018];
    assert_eq!((s.recent_blockhash, s.last_valid_height), (b, Some(h + 150)));
    assert!(ps[0].asked.borrow().iter().any(|a| a == "getBlock:1020"), "the skipped slot was asked and walked over");
    assert_eq!(block_on(solsend::tick(&mut line, &mut ps, &mut signer)).unwrap(), Step::Pending { intent: 11, sends: 1 });
    // not proven yet: resent, never re-signed
    chain.borrow_mut().advance(100, &[]);
    assert!(matches!(block_on(solsend::tick(&mut line, &mut ps, &mut signer)).unwrap(), Step::Pending { sends: 2, .. }));
    assert_eq!(signer.signed.borrow().len(), 1);
    // the blockhash expires unlanded (the leader dropped it): proven by the finalized height, then the status
    chain.borrow_mut().advance(80, &[]);
    assert_eq!(chain.borrow_mut().process(Level::Finalized).len(), 0, "expired");
    ps[0].asked.borrow_mut().clear();
    let st = block_on(solsend::tick(&mut line, &mut ps, &mut signer)).unwrap();
    let Step::Resigned { intent: 11, signature } = st else { panic!("{st:?}") };
    let asked = ps[0].asked.borrow().clone();
    let h_at = asked.iter().position(|x| x == "getBlockHeight").unwrap();
    assert!(asked.iter().skip(h_at).any(|x| x.starts_with("getSignatureStatuses")), "the height FIRST, the status SECOND: {asked:?}");
    let s2 = line.saved.clone().unwrap();
    // audit A4-03: the old signature, proven expired and unknown, leaves the status query (`expired`, still ours)
    assert_eq!((s2.intent, s2.sends, s2.history.clone(), s2.signature.clone()), (11, 0, vec![signature.clone()], signature));
    assert_eq!(s2.expired, Some(vec![s.signature.clone()]));
    assert_eq!(s2.instructions, s.instructions, "the SAME intent");
    assert_ne!(s2.tx, s.tx);
    assert_eq!(signer.signed.borrow().len(), 2);
    // the new bytes go out on the next tick and land; the old ones never can
    assert!(matches!(block_on(solsend::tick(&mut line, &mut ps, &mut signer)).unwrap(), Step::Pending { sends: 1, .. }));
    assert_eq!(chain.borrow_mut().process(Level::Finalized), vec![s2.signature.clone()]);
    let Step::Landed(d) = block_on(solsend::tick(&mut line, &mut ps, &mut signer)).unwrap() else { panic!() };
    assert_eq!(d.signature, s2.signature);
    let mut both = vec![s.signature.clone(), s2.signature.clone()];
    both.sort();
    assert_eq!(solsend::sol_seen(&[&line]), both, "the expired signature stays ours");
}

/// A landing under an OLD signature of the intent (after a re-sign) still settles it: every signature in `history` is asked.
#[test]
fn an_old_signature_landing_after_a_re_sign_still_settles_the_intent() {
    let chain = SolChain::new();
    // the fee payer exists (phase B2: every transaction is simulated before it is signed, and a payer with nothing is refused)
    chain.borrow_mut().set_account(ME, soltx::SYSTEM_PROGRAM, 10_000_000_000, vec![]);
    chain.borrow_mut().advance(25, &[]);
    let mut ps = pair(&chain);
    let mut signer = FakeSigner::new(ME);
    let mut line = Line::default();
    block_on(solsend::prepare(&mut line, LifetimeKind::RecentBlockhash, 12, "t", intent(5), &mut ps, &mut signer)).unwrap();
    let first = line.saved.clone().unwrap();
    // pretend it is in history alongside a newer one, and the old one is what landed
    let mut s = first.clone();
    s.history.push("1111111111111111111111111111111111111111111111111111111111111111".into());
    s.sends = 1;
    line.saved = Some(s);
    chain.borrow_mut().sent.push(first.tx.clone());
    chain.borrow_mut().process(Level::Finalized);
    let Step::Landed(d) = block_on(solsend::tick(&mut line, &mut ps, &mut signer)).unwrap() else { panic!() };
    assert_eq!(d.signature, first.signature);
}

#[test]
fn disagreement_or_a_failed_outcall_changes_nothing() {
    let (chain, mut ps, mut signer, mut line, nonce) = setup();
    // one provider is behind on the nonce account: prepare refuses, nothing signed, nothing saved
    let mut stale = chain.borrow().accounts[&nonce].clone();
    stale.data[40] ^= 1;
    ps[1].lie.insert(nonce, Some(stale));
    let e = block_on(solsend::prepare(&mut line, LifetimeKind::DurableNonce, 1, "t", intent(1), &mut ps, &mut signer)).unwrap_err();
    assert!(e.contains("disagree"), "{e}");
    assert!(line.saved.is_none() && signer.signed.borrow().is_empty());
    ps[1].lie.clear();
    block_on(solsend::prepare(&mut line, LifetimeKind::DurableNonce, 1, "t", intent(1), &mut ps, &mut signer)).unwrap();
    block_on(solsend::tick(&mut line, &mut ps, &mut signer)).unwrap();
    chain.borrow_mut().process(Level::Finalized);
    // a provider a slot behind (confirmed against finalized) is a disagreement: refused, the line unchanged
    ps[1].status_behind = true;
    let before = line.clone();
    assert!(matches!(block_on(solsend::tick(&mut line, &mut ps, &mut signer)), Err(Refused::Disagree(_))));
    assert_eq!(line, before);
    ps[1].status_behind = false;
    // a provider down: refused, unchanged
    ps[0].down = true;
    assert!(matches!(block_on(solsend::tick(&mut line, &mut ps, &mut signer)), Err(Refused::Failed(_))));
    assert_eq!(line, before);
    ps[0].down = false;
    assert!(matches!(block_on(solsend::tick(&mut line, &mut ps, &mut signer)).unwrap(), Step::Landed(_)));
}

#[test]
fn a_single_provider_is_refused_and_an_authority_not_ours_is_refused() {
    let (chain, mut ps, mut signer, mut line, nonce) = setup();
    let e = block_on(solsend::prepare(&mut line, LifetimeKind::DurableNonce, 1, "t", intent(1), &mut ps[..1], &mut signer)).unwrap_err();
    assert!(e.contains("two at least"), "{e}");
    chain.borrow_mut().set_nonce(nonce, [9; 32]);
    let e = block_on(solsend::prepare(&mut line, LifetimeKind::DurableNonce, 1, "t", intent(1), &mut ps, &mut signer)).unwrap_err();
    assert!(e.contains("authority"), "{e}");
    assert!(signer.signed.borrow().is_empty());
}

#[test]
fn an_oversized_intent_is_refused_before_any_signature() {
    let (_chain, mut ps, mut signer, mut line, _) = setup();
    let big: Vec<Instruction> = (0..40u8).map(|i| soltx::transfer(ME, [i + 1; 32], 1)).collect();
    let e = block_on(solsend::prepare(&mut line, LifetimeKind::DurableNonce, 1, "big", big, &mut ps, &mut signer)).unwrap_err();
    assert!(e.contains("1232"), "{e}");
    assert!(signer.signed.borrow().is_empty() && line.saved.is_none());
    // and an intent needing another signer is refused the same way
    let other = vec![soltx::transfer([7; 32], TO, 1)];
    let e = block_on(solsend::prepare(&mut line, LifetimeKind::DurableNonce, 1, "x", other, &mut ps, &mut signer)).unwrap_err();
    assert!(e.contains("signs alone"), "{e}");
}

#[test]
fn the_nonce_line_is_opened_by_the_canister_alone() {
    let chain = SolChain::new();
    // the fee payer exists (phase B2: every transaction is simulated before it is signed, and a payer with nothing is refused)
    chain.borrow_mut().set_account(ME, soltx::SYSTEM_PROGRAM, 10_000_000_000, vec![]);
    chain.borrow_mut().advance(25, &[]);
    let mut ps = pair(&chain);
    let mut signer = FakeSigner::new(ME);
    let rent = block_on(etch_autobuyer::solrpc::rent_exempt(&mut ps, soltx::NONCE_ACCOUNT_SPACE)).unwrap();
    assert_eq!(rent, 1_447_680);
    let ixs = solsend::open_nonce_ixs(&ME, SEED, rent).unwrap();
    let mut line = Line::default();
    block_on(solsend::prepare(&mut line, LifetimeKind::RecentBlockhash, 0, "open nonce", ixs, &mut ps, &mut signer)).unwrap();
    let s = line.saved.unwrap();
    let p = soltx::Parts::parse(soltx::split_tx(&s.tx).unwrap().1).unwrap();
    assert_eq!(p.signers(), vec![ME], "one signature: the seed-derived account needs no key of its own");
    assert!(p.keys.contains(&solsend::nonce_account_of(&ME, SEED).unwrap()));
}
