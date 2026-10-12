//! AUDIT D PoC harness: a verbatim copy of tests/redeem.rs's fake world (lines 1-311). The PoC test is at the bottom.
//!
//! The custodian's frog delivery (docs/v3/wrapper.md, "Step 3, the flow") against a fake world: tickets that resolve, refund
//! and run out of time, ord and explorers that lag or disagree with custody, a Bitcoin that loses broadcasts and evicts them, a
//! keeper who may or may not record, and a canister that dies between any two calls.

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::pin::pin;
use std::task::{Context, Poll, Waker};

use etch_autobuyer::btcsend::parse_signed;
use etch_autobuyer::custody::{Contents, Location, Outpoint, Output, Utxo};
use etch_autobuyer::delivery::{Stage, TicketRead};
use etch_autobuyer::evmtx;
use etch_autobuyer::looper::{BtcView, EvmView};
use etch_autobuyer::policy::script_of;
use etch_autobuyer::redeem::{self, Custody, CustodyIo, RPhase, Redemption};
use etch_autobuyer::{custodycap, outbound, outflow};

fn block_on<F: Future>(f: F) -> F::Output {
    let mut f = pin!(f);
    let mut cx = Context::from_waker(Waker::noop());
    loop {
        if let Poll::Ready(v) = f.as_mut().poll(&mut cx) {
            return v;
        }
    }
}

const DEST: &str = "bc1p5cyxnuxmeuwuvkwfem96lqzszd02n6xdcjrs20cac6yqjjwudpxqkedrcr";
const T0: u64 = 1_790_000_000;
const DAY: u64 = 86_400;

fn ours() -> Vec<u8> {
    let mut s = vec![0x51, 0x20];
    s.extend([1u8; 32]);
    s
}
fn op(tag: u8, vout: u32) -> Outpoint {
    Outpoint { txid: [tag; 32], vout }
}
fn insc(n: u8) -> String {
    format!("{}i0", hex::encode([n; 32]))
}
fn display(internal: [u8; 32]) -> [u8; 32] {
    let mut d = internal;
    d.reverse();
    d
}

#[derive(Clone)]
struct T {
    stage: Stage,
    acq: u128,
    dest: String,
    deliver_by: u64,
    deliver_by_block: u64,
}

struct World {
    now: u64,
    tickets: Vec<T>,
    insc: HashMap<u128, String>,
    loc: HashMap<String, Location>,
    contents: HashMap<Outpoint, Contents>,
    outputs: HashMap<Outpoint, (Output, bool)>,
    utxos: Vec<Utxo>,
    rate: u64,
    keeper: bool,
    mempool: HashSet<[u8; 32]>,
    confirmed: HashSet<[u8; 32]>,
    /// a broadcast reaches a node (false: the canister died between save and send, or the network dropped it)
    accept: bool,
    sent: Vec<Vec<u8>>,
    evm_sent: Vec<Vec<u8>>,
    evm_mined: HashMap<[u8; 32], bool>,
    nonce: u64,
    signs: usize,
    /// called on every ticket read with the read's index: the chain moving between reads
    hook: Option<Box<dyn FnMut(usize, &mut Vec<T>)>>,
    reads: usize,
    saved: HashMap<u64, Redemption>,
    cursor: u64,
    /// H2: the caps in force and the book
    caps: custodycap::CapPolicy,
    book: custodycap::Book,
    /// H3: the limiter, when limits are set (None: every act allowed); the asks and records made, by key
    limits: Option<outflow::Limits>,
    outflow: outflow::Outflow,
    asked: Vec<String>,
    recorded: Vec<String>,
    /// what the canister's pause would be after a trip
    paused: bool,
    /// H4/H9: the ticket as read at the finalized block, changed from what the fast read says (a swapped read, a forgery)
    facts_hook: Option<Box<dyn FnMut(&mut outbound::TicketFacts)>>,
    /// H8: every transaction Bitcoin has been handed, by display txid, so that who spent what can be read back
    txs: HashMap<[u8; 32], Vec<u8>>,
    /// spends by transactions that are not ours (display txid, confirmed)
    third_party: HashMap<Outpoint, ([u8; 32], bool)>,
    /// the threshold signer refuses (the subnet fails, or the canister dies mid-signing)
    fail_sign: bool,
}

const WRAPPER: [u8; 20] = [0xcc; 20];

fn frogs_key() -> String {
    custodycap::btc_key(&WRAPPER)
}

impl World {
    /// One wrapped frog (acquisition 7, inscription 0x07…i0) on our own output, one clean fee UTXO, and ticket 0 resolved to it.
    fn new() -> Self {
        let frog = op(0xf0, 0);
        let fee = op(0xfe, 1);
        let mut w = World {
            now: T0,
            tickets: vec![T { stage: Stage::Resolved, acq: 7, dest: DEST.into(), deliver_by: T0 + 7 * DAY, deliver_by_block: (T0 + 7 * DAY) / 12 }],
            insc: HashMap::from([(7, insc(7))]),
            loc: HashMap::from([(insc(7), Location { output: frog, offset: 0, value: 546 })]),
            contents: HashMap::new(),
            outputs: HashMap::new(),
            utxos: vec![Utxo { outpoint: frog, value: 546, confirmed: true }, Utxo { outpoint: fee, value: 20_000, confirmed: true }],
            rate: 3,
            keeper: false,
            mempool: HashSet::new(),
            confirmed: HashSet::new(),
            accept: true,
            sent: vec![],
            evm_sent: vec![],
            evm_mined: HashMap::new(),
            nonce: 4,
            signs: 0,
            hook: None,
            reads: 0,
            saved: HashMap::new(),
            cursor: 0,
            caps: custodycap::CapPolicy { collections: vec![custodycap::CollectionCap { collection: frogs_key(), max_items: 1_000,
                max_value_wei: u128::MAX / 4, max_item_wei: u128::MAX / 4 }], global_max_items: 1_000, global_max_value_wei: u128::MAX / 4 },
            book: custodycap::Book::default(),
            limits: None,
            outflow: outflow::Outflow::default(),
            asked: vec![],
            recorded: vec![],
            paused: false,
            facts_hook: None,
            txs: HashMap::new(),
            third_party: HashMap::new(),
            fail_sign: false,
        };
        w.contents.insert(frog, Contents { inscriptions: vec![insc(7)], runes: vec![], value: 546 });
        w.outputs.insert(frog, (Output { script: ours(), value: 546, confirmed: true }, false));
        w.contents.insert(fee, Contents { inscriptions: vec![], runes: vec![], value: 20_000 });
        w
    }
    fn mine(&mut self) {
        let m: Vec<_> = self.mempool.drain().collect();
        self.confirmed.extend(m);
    }
    fn inputs_of(raw: &[u8]) -> Vec<Outpoint> {
        parse_signed(raw).unwrap().inputs.iter().map(|i| Outpoint { txid: display(i.txid), vout: i.vout }).collect()
    }
    /// What an output is worth: ours as listed, or an output of a transaction Bitcoin was handed.
    fn value_of(&self, o: &Outpoint) -> u64 {
        if let Some(u) = self.utxos.iter().find(|u| u.outpoint == *o) {
            return u.value;
        }
        if let Some((out, _)) = self.outputs.get(o) {
            return out.value;
        }
        self.txs.get(&o.txid).map(|raw| parse_signed(raw).unwrap().outputs[o.vout as usize].0).unwrap_or(0)
    }
    fn fee_of(&self, raw: &[u8]) -> u64 {
        let ins: u64 = Self::inputs_of(raw).iter().map(|o| self.value_of(o)).sum();
        ins - parse_signed(raw).unwrap().outputs.iter().map(|o| o.0).sum::<u64>()
    }
    /// Who spent `o`: a third party's recorded spend, else any transaction in a block or a mempool that spends it.
    fn spender(&self, o: &Outpoint) -> Option<([u8; 32], bool)> {
        if let Some(x) = self.third_party.get(o) {
            return Some(*x);
        }
        self.confirmed.iter().map(|t| (*t, true)).chain(self.mempool.iter().map(|t| (*t, false)))
            .find(|(t, _)| self.txs.get(t).is_some_and(|raw| Self::inputs_of(raw).contains(o)))
    }
    /// A node's mempool policy, simplified: a transaction whose input a block (or a third party, confirmed) already spent is
    /// refused; one that spends what a mempool transaction spends replaces it only by paying a higher fee (BIP125 rule 3).
    fn broadcast(&mut self, raw: &[u8]) {
        self.sent.push(raw.to_vec());
        if !self.accept {
            return;
        }
        let id = display(parse_signed(raw).unwrap().txid);
        if self.confirmed.contains(&id) || self.mempool.contains(&id) {
            return;
        }
        self.txs.insert(id, raw.to_vec());
        let ins = Self::inputs_of(raw);
        if ins.iter().any(|o| matches!(self.spender(o), Some((t, true)) if t != id)) {
            return;
        }
        let clash: Vec<[u8; 32]> = self.mempool.iter().copied()
            .filter(|m| Self::inputs_of(&self.txs[m]).iter().any(|o| ins.contains(o))).collect();
        if clash.iter().any(|m| self.fee_of(&self.txs[m]) >= self.fee_of(raw)) {
            return;
        }
        for m in clash {
            self.mempool.remove(&m);
        }
        self.mempool.insert(id);
    }
    fn tick(&mut self, c: &mut Custody, purchases_evm: bool) -> Vec<String> {
        block_on(redeem::tick(self, c, purchases_evm))
    }
    fn phase(&self, c: &Custody, ticket: u64) -> RPhase {
        c.redemptions.iter().find(|r| r.ticket == ticket).expect("a redemption").phase.clone()
    }
}

impl CustodyIo for World {
    fn now(&self) -> u64 { self.now }
    fn chain_id(&self) -> u64 { 4663 }
    async fn ticket_count(&mut self) -> Result<u64, String> { Ok(self.tickets.len() as u64) }
    async fn read_ticket(&mut self, id: u64, _fresh: bool) -> Result<TicketRead, String> {
        let n = self.reads;
        self.reads += 1;
        if let Some(h) = self.hook.as_mut() {
            h(n, &mut self.tickets);
        }
        let t = self.tickets.get(id as usize).cloned().ok_or("no such ticket")?;
        Ok(TicketRead { stage: t.stage, acquisition_id: t.acq, destination: t.dest, deliver_by: t.deliver_by,
            deliver_by_block: t.deliver_by_block, chain_time: self.now, parent_block: self.now / 12, read_at: self.now })
    }
    async fn inscription_of(&mut self, a: u128) -> Result<String, String> { self.insc.get(&a).cloned().ok_or("no acquisition".into()) }
    async fn locate(&mut self, i: &str) -> Result<Option<Location>, String> { Ok(self.loc.get(i).cloned()) }
    async fn contents(&mut self, o: &Outpoint) -> Result<Option<Contents>, String> { Ok(self.contents.get(o).cloned()) }
    async fn output(&mut self, o: &Outpoint) -> Result<(Output, bool), String> {
        let spent = self.spender(o).is_some();
        if let Some((out, s)) = self.outputs.get(o).cloned() {
            return Ok((out, s || spent));
        }
        let u = self.utxos.iter().find(|u| u.outpoint == *o).cloned().ok_or("unknown output")?;
        Ok((Output { script: ours(), value: u.value, confirmed: u.confirmed }, spent))
    }
    async fn our_utxos(&mut self) -> Result<Vec<Utxo>, String> {
        Ok(self.utxos.iter().filter(|u| self.spender(&u.outpoint).is_none()).cloned().collect())
    }
    async fn fee_rate(&mut self) -> Result<u64, String> { Ok(self.rate) }
    async fn our_script(&mut self) -> Result<Vec<u8>, String> { Ok(ours()) }
    fn btc_hrp(&self) -> Result<&'static str, String> { Ok("bc") }
    async fn sign_taproot(&mut self, _h: [u8; 32]) -> Result<Vec<u8>, String> {
        if self.fail_sign {
            return Err("the subnet did not answer".into());
        }
        self.signs += 1;
        Ok(vec![0x5a; 64])
    }
    async fn btc_send(&mut self, raw: &[u8]) -> Result<(), String> {
        parse_signed(raw)?;
        self.broadcast(raw);
        Ok(())
    }
    async fn btc_seen(&mut self, txid: [u8; 32]) -> BtcView {
        if self.confirmed.contains(&txid) { BtcView::Confirmed } else if self.mempool.contains(&txid) { BtcView::Mempool } else { BtcView::NotYet }
    }
    async fn may_record(&mut self) -> Result<bool, String> { Ok(self.keeper) }
    fn wrapper(&self) -> [u8; 20] { WRAPPER }
    async fn evm_nonce(&mut self) -> Result<u64, String> { Ok(self.nonce) }
    async fn base_fee(&mut self) -> Result<u128, String> { Ok(10_000_000) }
    async fn estimate_gas(&mut self, _to: [u8; 20], _v: u128, _d: &[u8]) -> Result<u64, String> { Ok(50_000) }
    async fn sign_evm(&mut self, _h: [u8; 32]) -> Result<(u8, [u8; 32], [u8; 32]), String> { Ok((0, [1; 32], [2; 32])) }
    async fn send_evm(&mut self, raw: &[u8]) -> Result<(), String> {
        self.evm_sent.push(raw.to_vec());
        Ok(())
    }
    async fn evm_seen(&mut self, hash: [u8; 32]) -> EvmView {
        match self.evm_mined.get(&hash) { Some(s) => EvmView::Mined { success: *s }, None => EvmView::NotYet }
    }
    fn save(&mut self, r: &Redemption) { self.saved.insert(r.ticket, r.clone()); }
    fn save_cursor(&mut self, c: u64) { self.cursor = c; }
    fn with_caps<R>(&mut self, f: impl FnOnce(&custodycap::CapPolicy, &mut custodycap::Book) -> R) -> R { f(&self.caps, &mut self.book) }
    fn outflow_check(&mut self, key: &str, collection: &str, flow: outflow::Flow, amount: u128) -> Result<(), String> {
        self.asked.push(key.to_string());
        let Some(l) = self.limits.clone() else { return Ok(()) };
        match self.outflow.check(&l, key, collection, flow, amount, self.now) {
            outflow::Verdict::Allow => Ok(()),
            outflow::Verdict::Wait { why, .. } => Err(format!("waits (outflow limit): {why}")),
            outflow::Verdict::Frozen(c) => {
                self.paused = true;
                Err(format!("refused: custody is frozen: {c:?}"))
            }
        }
    }
    fn outflow_record(&mut self, key: &str, collection: &str, flow: outflow::Flow, amount: u128) {
        self.recorded.push(key.to_string());
        self.outflow.record(key, collection, flow, amount, self.now);
    }
    async fn ticket_facts(&mut self, id: u64, acquisition_id: u128) -> Result<outbound::TicketFacts, String> {
        let t = self.tickets.get(id as usize).cloned().ok_or("no such ticket")?;
        let d = script_of(&t.dest, "bc")?;
        let mut f = outbound::TicketFacts { ticket: id, resolved: t.stage == Stage::Resolved, finalized: true, destination_script: d.clone(),
            event_destination_script: Some(d), names_this_item: t.acq == acquisition_id };
        if let Some(h) = self.facts_hook.as_mut() {
            h(&mut f);
        }
        Ok(f)
    }
    fn outbound_policy(&self, custody_script: &[u8]) -> outbound::BtcPolicy {
        outbound::BtcPolicy { custody_script: custody_script.to_vec(), max_fee_sats: outbound::MAX_FEE_SATS, require_finalized: true,
            require_event: true }
    }
    fn paused(&self) -> bool { self.paused }
    async fn outspend(&mut self, o: &Outpoint) -> Result<Option<([u8; 32], bool)>, String> { Ok(self.spender(o)) }
}

// =====================================================================================================================================
// AUDIT D PoC
// =====================================================================================================================================

/// AUDIT-D M-1 PoC. A delivery that reaches a mempool is only ever watched: `Sent` + `Mempool` is "waiting for a block", the rule
/// is not re-run, nothing re-prices it, and H8's conflict-out starts only if every explorer stops showing it. If it is still
/// unconfirmed at `deliverBy` (a fee spike above the 6-block estimate, or above the 100 sat/vB clamp), the redeemer calls the
/// Wrapper's permissionless `refund` (one wX back) and the delivery confirms afterwards: the frog AND the token, one unbacked
/// unit forever.
#[test]
fn audit_d_a_delivery_pending_past_its_deadline_is_never_bumped_or_conflicted_and_ends_a_lost_race() {
    let mut w = World::new();
    let mut c = Custody::default();
    w.tick(&mut c, false);
    let RPhase::Signed { txid, raw, .. } = w.phase(&c, 0) else { panic!("{:?}", w.phase(&c, 0)) };
    w.tick(&mut c, false);
    assert!(matches!(w.phase(&c, 0), RPhase::Sent { .. }), "in a mempool");
    let (signs, sends) = (w.signs, w.sent.len());
    // fees spike: the delivery sits in the mempool for the rest of the week. Tick every 10 minutes up to the deadline.
    while w.now < T0 + 7 * DAY {
        w.now += 600;
        w.tick(&mut c, false);
    }
    // the deadline passes; the redeemer refunds (Wrapper.refund is anyone's once deliverBy has passed on both clocks)
    w.now += 600;
    w.tickets[0].stage = Stage::Refunded;
    for _ in 0..6 {
        w.now += 600;
        w.tick(&mut c, false);
    }
    assert!(matches!(w.phase(&c, 0), RPhase::Sent { .. }), "still just watching: {:?}", w.phase(&c, 0));
    assert_eq!(w.signs, signs, "nothing was ever re-signed: no fee bump, no conflict-out");
    assert!(w.sent[sends..].is_empty() || w.sent[sends..].iter().all(|r| *r == raw), "no new transaction was ever sent");
    // fees fall; the old bytes confirm
    w.mine();
    w.tick(&mut c, false);
    assert_eq!(w.phase(&c, 0), RPhase::Confirmed { txid });
    w.tick(&mut c, false);
    assert_eq!(w.phase(&c, 0), RPhase::LostRace { txid }, "the redeemer has the frog and the refunded token");
    println!("AUDIT-D: {} ticks watched a pending delivery through its deadline; outcome {:?}", 7 * 144 + 6, w.phase(&c, 0));
}
