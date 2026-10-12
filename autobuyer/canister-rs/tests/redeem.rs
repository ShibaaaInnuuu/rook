//! The custodian's frog delivery (docs/v3/wrapper.md, "Step 3, the flow") against a fake world: tickets that resolve, refund
//! and run out of time, ord and explorers that lag or disagree with custody, a Bitcoin that loses broadcasts and evicts them, a
//! keeper who may or may not record, and a canister that dies between any two calls. The ticket's `Redeemed` and `Resolved`
//! events are served by fakes with the Phase-1 providers' measured limits (`tests/fakerpc`: Robinhood's ≈ 6,200 blocks of state,
//! ordofi's 10,000-block log span, finality 8,096–11,996 behind), so the delivery cannot pass against a provider that answers
//! everything again (audit D, H-1).

mod fakerpc;

use std::cell::RefCell;
use std::rc::Rc;

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
use etch_autobuyer::{custodycap, outbound, outflow, ticketlog};
use fakerpc::{Chain, FakeRpc, BLOCKS_PER_SEC};

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
/// Chain 4663's head at T0 (the one measured on 2026-09-26).
const H0: u64 = 72_751_086;
/// Where the scanner starts (the install's `wrapper_from_block`): the wrapper's first block, before any ticket.
const WRAPPER_FROM: u64 = H0 - 30_000;

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
    /// H4/H9 (audit D, H-1): chain 4663 as two fake providers with the measured limits serve its logs; the events of each
    /// resolved ticket are put on it when first needed: before the scanner starts, deep (final); after, at the head (final ~20
    /// minutes later, as on the real chain)
    chain: Rc<RefCell<Chain>>,
    rpcs: Vec<FakeRpc>,
    /// Tickets whose events are on the chain.
    on_chain: HashSet<u64>,
    /// A forged or odd event, in place of the true one: the Redeemed destination, and the Resolved acquisition id, by ticket.
    event_dest: HashMap<u64, String>,
    event_acq: HashMap<u64, u128>,
    /// New events go on at the head even before the scanner starts (they then wait for finality).
    events_at_head: bool,
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
            chain: Rc::new(RefCell::new(Chain { head: H0, logs: vec![] })),
            rpcs: vec![],
            on_chain: HashSet::new(),
            event_dest: HashMap::new(),
            event_acq: HashMap::new(),
            events_at_head: false,
            txs: HashMap::new(),
            third_party: HashMap::new(),
            fail_sign: false,
        };
        w.contents.insert(frog, Contents { inscriptions: vec![insc(7)], runes: vec![], value: 546 });
        w.outputs.insert(frog, (Output { script: ours(), value: 546, confirmed: true }, false));
        w.contents.insert(fee, Contents { inscriptions: vec![], runes: vec![], value: 20_000 });
        w.rpcs = fakerpc::pair(&w.chain);
        w
    }
    /// Chain 4663's head now: ≈ 10 blocks a second since T0.
    fn head(&self) -> u64 {
        H0 + self.now.saturating_sub(T0) * BLOCKS_PER_SEC
    }
    /// Every resolved (or later) ticket's events on the chain, once.
    fn put_events(&mut self, scanner_started: bool) {
        let head = self.head();
        for (i, t) in self.tickets.clone().iter().enumerate() {
            let i = i as u64;
            if t.stage == Stage::Pinned || t.stage == Stage::None || self.on_chain.contains(&i) {
                continue;
            }
            let at = if scanner_started || self.events_at_head { head } else { H0 - 25_000 + i };
            let dest = self.event_dest.get(&i).cloned().unwrap_or(t.dest.clone());
            let acq = self.event_acq.get(&i).copied().unwrap_or(t.acq);
            let mut c = self.chain.borrow_mut();
            c.logs.push(fakerpc::redeemed(&WRAPPER, i, &dest, at));
            c.logs.push(fakerpc::resolved(&WRAPPER, i, acq, at + 1));
            drop(c);
            self.on_chain.insert(i);
        }
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

/// Phase B2: this world's home is the EVM chain (no Solana).
impl etch_autobuyer::solhome::SolIo for World {}

impl CustodyIo for World {
    fn now(&self) -> u64 { self.now }
    fn chain_id(&self) -> u64 { 4663 }
    async fn ticket_count(&mut self) -> Result<(u64, u64), String> { Ok((self.tickets.len() as u64, self.head() - 1_500)) }
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
    async fn scan_ticket_logs(&mut self, logs: &mut ticketlog::TicketLogs, wanted: Option<&[u64]>) -> Result<String, String> {
        let head = self.head();
        self.chain.borrow_mut().head = head;
        self.put_events(logs.cursor.is_some());
        ticketlog::advance(&mut self.rpcs, logs, &WRAPPER, Some(WRAPPER_FROM), wanted).await
    }
    fn outbound_policy(&self, custody_script: &[u8]) -> outbound::BtcPolicy {
        outbound::BtcPolicy { custody_script: custody_script.to_vec(), max_fee_sats: outbound::MAX_FEE_SATS, require_finalized: true,
            require_event: true }
    }
    fn paused(&self) -> bool { self.paused }
    async fn outspend(&mut self, o: &Outpoint) -> Result<Option<([u8; 32], bool)>, String> { Ok(self.spender(o)) }
}

#[test]
fn a_resolved_ticket_is_delivered_confirmed_and_then_recorded_by_the_keeper() {
    let mut w = World::new();
    let mut c = Custody::default();
    w.tick(&mut c, false);
    // found, built, signed (both inputs), saved, sent — in that order
    let RPhase::Signed { txid, raw, fee_input, .. } = w.phase(&c, 0) else { panic!("{:?}", w.phase(&c, 0)) };
    assert_eq!((w.signs, w.sent.len(), fee_input), (2, 1, op(0xfe, 1)));
    assert_eq!(w.saved[&0].phase, w.phase(&c, 0), "saved before the tick ended");
    assert_eq!(w.sent[0], raw, "what was sent is what was saved");
    // the transaction: input 0 is the frog, output 0 pays the redeemer exactly the frog's value, change comes back to us
    let tx = parse_signed(&raw).unwrap();
    assert_eq!(display(tx.txid), txid);
    assert_eq!(tx.inputs.len(), 2);
    assert_eq!(tx.outputs[0], (546, script_of(DEST, "bc").unwrap()));
    assert_eq!(tx.outputs[1].1, ours());
    assert_eq!(tx.outputs[1].0, 20_000 - 3 * 212);
    // Bitcoin has it, then confirms it; not the keeper, so it waits for the record
    w.tick(&mut c, false);
    assert!(matches!(w.phase(&c, 0), RPhase::Sent { .. }));
    w.mine();
    w.tick(&mut c, false);
    assert_eq!(w.phase(&c, 0), RPhase::Confirmed { txid });
    w.tick(&mut c, false);
    assert_eq!(w.phase(&c, 0), RPhase::AwaitingRecord { txid });
    assert_eq!(c.redemptions[0].record_owed(), Some(txid), "listed for the keeper");
    assert!(w.evm_sent.is_empty(), "not ours to record: nothing sent to 4663");
    // the keeper records it
    w.tickets[0].stage = Stage::Delivered;
    w.tick(&mut c, false);
    assert_eq!(w.phase(&c, 0), RPhase::Done { txid });
    let n = w.sent.len();
    w.tick(&mut c, false);
    assert_eq!(w.sent.len(), n, "a finished delivery sends nothing more");
}

#[test]
fn inside_the_margin_nothing_is_built_signed_or_sent_and_the_refusal_is_final() {
    let mut w = World::new();
    w.now = T0 + 6 * DAY; // exactly one day left: inside
    let mut c = Custody::default();
    w.tick(&mut c, false);
    assert!(matches!(w.phase(&c, 0), RPhase::Refused { ref reason } if reason.contains("InsideMargin")));
    assert_eq!((w.signs, w.sent.len()), (0, 0));
    w.now = T0; // even if the clock were to read earlier again, a refusal stays
    w.tick(&mut c, false);
    assert_eq!((w.signs, w.sent.len()), (0, 0));
}

#[test]
fn a_ticket_refunded_between_the_first_read_and_the_send_is_refused_after_signing_and_nothing_is_saved_or_sent() {
    let mut w = World::new();
    // discovery read (0), the rule before building (1), the rule after signing (2): refunded at the third
    w.hook = Some(Box::new(|n, t| if n == 2 { t[0].stage = Stage::Refunded }));
    let mut c = Custody::default();
    w.tick(&mut c, false);
    assert!(matches!(w.phase(&c, 0), RPhase::Refused { ref reason } if reason.contains("Refunded")));
    assert_eq!(w.signs, 2, "it had signed");
    assert!(w.sent.is_empty(), "and sent nothing");
    assert!(!matches!(w.saved[&0].phase, RPhase::Signed { .. }), "and never saved the bytes as Signed");
}

#[test]
fn a_canister_that_dies_between_save_and_send_wakes_and_sends_the_same_bytes() {
    let mut w = World::new();
    w.accept = false; // the send never reached a node
    let mut c = Custody::default();
    w.tick(&mut c, false);
    let RPhase::Signed { raw, .. } = w.phase(&c, 0) else { panic!() };
    // the restart: the saved record is all there is
    let mut c = Custody { cursor: w.cursor, redemptions: w.saved.values().cloned().collect(), logs: c.logs.clone(), colls: None };
    w.accept = true;
    w.tick(&mut c, false);
    assert_eq!(w.sent.last().unwrap(), &raw, "the SAME bytes, not a new transaction");
    assert_eq!(w.signs, 2, "nothing signed again");
    w.tick(&mut c, false);
    assert!(matches!(w.phase(&c, 0), RPhase::Sent { .. }));
}

/// H8, resolved 2026-09-25 (point 1): saved bytes were handed to `btc_send` even when no explorer ever showed them, so a
/// refusal of them is a conflict-out, not a bare `Refused`. The delivery bytes are never sent again; the one new transaction
/// is the conflict, which spends only the fee input and pays only custody.
#[test]
fn saved_bytes_never_seen_are_not_resent_once_the_ticket_is_inside_the_margin() {
    let mut w = World::new();
    w.accept = false;
    let mut c = Custody::default();
    w.tick(&mut c, false);
    let RPhase::Signed { raw: leaked, .. } = w.phase(&c, 0) else { panic!() };
    let sends = w.sent.len();
    w.now = T0 + 6 * DAY + 60; // a day later, still unseen: now inside the margin
    w.accept = true;
    w.tick(&mut c, false);
    assert!(matches!(w.phase(&c, 0), RPhase::Conflicting { ref reason, .. } if reason.contains("not sent again")), "{:?}", w.phase(&c, 0));
    assert_eq!(w.sent.len(), sends + 1, "one new transaction: the conflict");
    assert!(!w.sent[sends..].contains(&leaked), "the delivery bytes were not sent again");
    assert_eq!(World::inputs_of(&w.sent[sends]), vec![op(0xfe, 1)], "the conflict spends the fee input and nothing else");
}

#[test]
fn a_lost_reply_is_covered_by_reading_bitcoin_and_an_evicted_transaction_goes_back_through_the_rule() {
    let mut w = World::new();
    let mut c = Custody::default();
    w.tick(&mut c, false); // sent, and the node has it (the reply is ignored either way)
    w.tick(&mut c, false);
    let RPhase::Sent { txid, raw, .. } = w.phase(&c, 0) else { panic!() };
    // evicted from every mempool
    w.mempool.clear();
    w.tick(&mut c, false);
    assert!(matches!(w.phase(&c, 0), RPhase::Signed { .. }), "back to Signed: the rule runs before re-sending");
    w.tick(&mut c, false);
    assert_eq!(w.sent.last().unwrap(), &raw);
    w.mine();
    w.tick(&mut c, false);
    assert_eq!(w.phase(&c, 0), RPhase::Confirmed { txid }, "already confirmed when next read: straight to Confirmed");
}

#[test]
fn a_delivery_whose_ticket_was_refunded_anyway_is_recorded_as_a_lost_race_and_never_retried() {
    let mut w = World::new();
    let mut c = Custody::default();
    w.tick(&mut c, false);
    w.mine();
    w.tick(&mut c, false);
    let RPhase::Confirmed { txid } = w.phase(&c, 0) else { panic!("{:?}", w.phase(&c, 0)) };
    w.tickets[0].stage = Stage::Refunded;
    w.tick(&mut c, false);
    assert_eq!(w.phase(&c, 0), RPhase::LostRace { txid });
    let n = w.sent.len();
    w.tick(&mut c, false);
    assert_eq!(w.sent.len(), n);
}

#[test]
fn as_the_keeper_it_records_its_own_delivery_saving_the_bytes_first_one_evm_transaction_at_a_time() {
    let mut w = World::new();
    w.keeper = true;
    let mut c = Custody::default();
    w.tick(&mut c, false);
    w.mine();
    w.tick(&mut c, false);
    let RPhase::Confirmed { txid } = w.phase(&c, 0) else { panic!() };
    // a purchase's EVM transaction is out: the record waits
    w.tick(&mut c, true);
    assert!(w.evm_sent.is_empty());
    w.tick(&mut c, false);
    let RPhase::Recording { raw, hash, nonce, .. } = w.phase(&c, 0) else { panic!("{:?}", w.phase(&c, 0)) };
    assert_eq!((nonce, w.evm_sent.last().unwrap()), (4, &raw), "saved, then sent");
    assert!(raw.windows(4).any(|x| x == redeem::RECORD_DELIVERY), "a recordDelivery call");
    assert!(raw.windows(32).any(|x| x == txid), "naming the Bitcoin txid in display order");
    assert_eq!(evmtx::keccak(&raw), hash);
    assert!(c.redemptions[0].evm_pending());
    // mined; the ticket then reads Delivered
    w.evm_mined.insert(hash, true);
    w.tickets[0].stage = Stage::Delivered;
    w.tick(&mut c, false);
    w.tick(&mut c, false);
    assert_eq!(w.phase(&c, 0), RPhase::Done { txid });
}

#[test]
fn a_frog_not_safely_in_custody_is_stuck_loudly_and_ord_lagging_is_a_wait() {
    // ord does not know it yet: wait, nothing signed
    let mut w = World::new();
    w.loc.clear();
    let mut c = Custody::default();
    let lines = w.tick(&mut c, false);
    assert_eq!(w.phase(&c, 0), RPhase::Owed);
    assert!(lines.iter().any(|l| l.contains("ord does not know")), "{lines:?}");
    // not our output
    let mut w = World::new();
    let frog = op(0xf0, 0);
    w.outputs.get_mut(&frog).unwrap().0.script = vec![0x51, 0x20, 9, 9, 9];
    let mut c = Custody::default();
    w.tick(&mut c, false);
    assert!(matches!(w.phase(&c, 0), RPhase::Stuck { ref why } if why.contains("not in custody")));
    // the frog's output also carries runes
    let mut w = World::new();
    w.contents.get_mut(&frog).unwrap().runes = vec!["DOG•GO•TO•THE•MOON".into()];
    let mut c = Custody::default();
    w.tick(&mut c, false);
    assert!(matches!(w.phase(&c, 0), RPhase::Stuck { ref why } if why.contains("runes")));
    assert_eq!((w.signs, w.sent.len()), (0, 0));
}

#[test]
fn the_fee_never_comes_from_a_runes_utxo_an_inscribed_one_or_another_deliverys_input() {
    let mut w = World::new();
    // a smaller UTXO carrying runes sorts first as a candidate: it must be skipped for the clean one
    let runes = op(0xaa, 0);
    w.utxos.push(Utxo { outpoint: runes, value: 5_000, confirmed: true });
    w.contents.insert(runes, Contents { inscriptions: vec![], runes: vec!["BITBLOCK".into()], value: 5_000 });
    // a second resolved ticket for a second frog
    let frog2 = op(0xf2, 0);
    w.tickets.push(T { stage: Stage::Resolved, acq: 8, dest: DEST.into(), deliver_by: T0 + 7 * DAY, deliver_by_block: (T0 + 7 * DAY) / 12 });
    w.insc.insert(8, insc(8));
    w.loc.insert(insc(8), Location { output: frog2, offset: 0, value: 546 });
    w.contents.insert(frog2, Contents { inscriptions: vec![insc(8)], runes: vec![], value: 546 });
    w.outputs.insert(frog2, (Output { script: ours(), value: 546, confirmed: true }, false));
    w.utxos.push(Utxo { outpoint: frog2, value: 546, confirmed: true });
    let mut c = Custody::default();
    w.tick(&mut c, false);
    let RPhase::Signed { fee_input: f0, .. } = w.phase(&c, 0) else { panic!("{:?}", w.phase(&c, 0)) };
    assert_eq!(f0, op(0xfe, 1));
    // the only clean fee UTXO is reserved by ticket 0: ticket 1 waits rather than share it
    assert_eq!(w.phase(&c, 1), RPhase::Owed);
    for raw in &w.sent {
        let tx = parse_signed(raw).unwrap();
        let ins: Vec<_> = tx.inputs.iter().map(|i| format!("{i:?}")).collect();
        assert!(!ins.iter().any(|i| i.contains(&hex::encode(display([0xaa; 32])))) , "the runes UTXO was spent");
    }
}

#[test]
fn discovery_walks_in_order_stops_at_the_first_pinned_ticket_and_never_duplicates() {
    let mut w = World::new();
    let t = |stage| T { stage, acq: 0, dest: DEST.into(), deliver_by: T0 + 7 * DAY, deliver_by_block: (T0 + 7 * DAY) / 12 };
    // ticket 0 resolved (ours), 1 already delivered by someone, 2 refunded, 3 pinned, 4 pinned
    w.tickets.extend([t(Stage::Delivered), t(Stage::Refunded), t(Stage::Pinned), t(Stage::Pinned)]);
    let mut c = Custody::default();
    w.tick(&mut c, false);
    assert_eq!(c.redemptions.iter().map(|r| r.ticket).collect::<Vec<_>>(), vec![0]);
    assert_eq!((c.cursor, w.cursor), (3, 3), "stopped at the first pinned ticket, and saved where");
    // ticket 3 resolves; the walk resumes from 3 and adds it once
    w.tickets[3].stage = Stage::Resolved;
    w.tickets[3].acq = 7;
    w.tick(&mut c, false);
    w.tick(&mut c, false);
    assert_eq!(c.redemptions.iter().map(|r| r.ticket).collect::<Vec<_>>(), vec![0, 3]);
    assert_eq!(c.cursor, 4);
}

/// docs/v6/canister-floor-followups.md §4: a resolved ticket whose destination is custody itself (a YOYO relist, or a
/// protocol sale's own ticket) is not a delivery: nothing is built or signed for it, the limiter is never asked, and it is
/// final here (the sale side lists the item).
#[test]
fn a_relist_ticket_to_custody_itself_is_never_delivered_nor_asked_of_the_limiter() {
    let mut w = World::new();
    let ours_addr = etch_autobuyer::policy::taproot_address(&[1u8; 32], "bc");
    w.tickets[0].dest = ours_addr.clone();
    w.tickets.push(T { stage: Stage::Resolved, acq: 8, dest: DEST.into(), deliver_by: T0 + 7 * DAY, deliver_by_block: (T0 + 7 * DAY) / 12 });
    w.insc.insert(8, insc(8));
    let frog2 = op(0xf2, 0);
    w.loc.insert(insc(8), Location { output: frog2, offset: 0, value: 546 });
    w.contents.insert(frog2, Contents { inscriptions: vec![insc(8)], runes: vec![], value: 546 });
    w.outputs.insert(frog2, (Output { script: ours(), value: 546, confirmed: true }, false));
    let mut c = Custody::default();
    let lines = w.tick(&mut c, false);
    assert_eq!(w.phase(&c, 0), RPhase::Relist);
    assert!(c.redemptions[0].finished());
    assert!(lines.iter().any(|l| l.contains("a relist")), "{lines:?}");
    // the ordinary ticket beside it is delivered as ever; the relist asked nothing of the limiter
    assert!(matches!(w.phase(&c, 1), RPhase::Signed { .. }));
    assert_eq!(w.asked, vec!["ticket:1".to_string()]);
    for _ in 0..3 {
        w.tick(&mut c, false);
    }
    assert_eq!(w.phase(&c, 0), RPhase::Relist);
    assert_eq!(w.asked, vec!["ticket:1".to_string()], "still never asked");
    assert_eq!(w.sent.len(), 1, "only the holder's delivery went out");
}

// ---- the custody hardening, wired (docs/v5/custody-hardening.md, H2–H4 and H9) ----------------------------------------------

const ATTACKER: &str = "bc1p0xlxvlhemja6c4dqv22uapctqupfhlxm9h8z3k2e72q4k9hcz7vqzk5jj0";

#[test]
fn a_delivery_asks_the_limiter_counts_once_saved_and_frees_its_room_under_the_caps_when_done() {
    let mut w = World::new();
    w.keeper = false;
    // the frog is held under the caps (it was bought or deposited)
    let caps = w.caps.clone();
    custodycap::admit(&caps, &mut w.book, &insc(7), &frogs_key(), 5).unwrap();
    let mut c = Custody::default();
    w.tick(&mut c, false);
    let RPhase::Signed { txid, .. } = w.phase(&c, 0) else { panic!("{:?}", w.phase(&c, 0)) };
    assert_eq!((w.asked.clone(), w.recorded.clone()), (vec!["ticket:0".to_string()], vec!["ticket:0".to_string()]));
    assert_eq!(c.redemptions[0].inscription, Some(insc(7)), "the record knows what it delivers");
    assert!(w.book.contains(&insc(7)), "still held until the delivery is done");
    w.tick(&mut c, false);
    w.mine();
    w.tick(&mut c, false);
    w.tickets[0].stage = Stage::Delivered;
    w.tick(&mut c, false);
    w.tick(&mut c, false);
    assert_eq!(w.phase(&c, 0), RPhase::Done { txid });
    assert!(!w.book.contains(&insc(7)), "delivered: its room under the caps is free again");
    assert_eq!(w.recorded.len(), 1, "one act out, counted once");
}

/// The ticket at the fast pin says DEST, and so does everything the builder saw; the FINAL `Redeemed` event names someone else
/// (a forged or swapped read). The verifier refuses the signed delivery against the event, before anything is saved or sent.
#[test]
fn a_delivery_to_a_destination_the_final_redeemed_event_does_not_name_is_refused_before_anything_is_saved_or_sent() {
    let mut w = World::new();
    w.event_dest.insert(0, ATTACKER.into());
    let mut c = Custody::default();
    w.tick(&mut c, false);
    assert!(matches!(w.phase(&c, 0), RPhase::Stuck { ref why } if why.contains("EventDisagrees")), "{:?}", w.phase(&c, 0));
    assert!(w.sent.is_empty(), "nothing sent");
    assert!(!matches!(w.saved[&0].phase, RPhase::Signed { .. }), "and the bytes were never saved");
    assert!(w.recorded.is_empty(), "nothing counted as gone");
    // final: later ticks send nothing
    w.tick(&mut c, false);
    assert!(w.sent.is_empty());
}

/// The final `Resolved` event names another item than the ticket's state at the fast pin: two reads that must agree (audit D,
/// I-1), and a signature is refused when they do not.
#[test]
fn a_final_resolved_event_naming_another_item_is_refused_too() {
    let mut w = World::new();
    w.event_acq.insert(0, 99);
    let mut c = Custody::default();
    w.tick(&mut c, false);
    assert!(matches!(w.phase(&c, 0), RPhase::Stuck { ref why } if why.contains("WrongItem")), "{:?}", w.phase(&c, 0));
    assert!(w.sent.is_empty());
}

/// Events not yet final (the ticket resolved moments ago): the delivery waits, says why, and signs nothing; about twenty
/// minutes of chain later (two finality steps) the scanner has them and the delivery goes.
#[test]
fn a_ticket_whose_events_are_not_final_yet_waits_and_nothing_is_signed_until_they_are() {
    let mut w = World::new();
    w.events_at_head = true;
    let mut c = Custody::default();
    let lines = w.tick(&mut c, false);
    assert_eq!(w.phase(&c, 0), RPhase::Owed, "{lines:?}");
    assert_eq!((w.signs, w.sent.len()), (0, 0));
    assert!(lines.iter().any(|l| l.contains("not final yet")), "{lines:?}");
    w.now += 20 * 60;
    w.tick(&mut c, false);
    assert!(matches!(w.phase(&c, 0), RPhase::Signed { .. }), "{:?}", w.phase(&c, 0));
    // every log query the flow made stayed inside ordofi's span and at or below finality
    for p in &w.rpcs {
        for (m, from, to) in p.asked.borrow().iter() {
            if m == "eth_getLogs" {
                assert!(to - from < fakerpc::ORDOFI_LOG_SPAN, "{from}..={to}");
            }
        }
    }
}

/// T10 against the wired flow: colluding providers forge 40 resolved tickets, each for a frog custody really holds, each to the
/// attacker. Every one passes the bytes check (the forgery is in the reads). The limiter lets six out, the tripwire freezes
/// custody at the 18th distinct ask, and nothing more leaves: not later that tick, not days later, until a resume.
#[test]
fn forged_tickets_drain_six_frogs_then_the_tripwire_freezes_custody_and_nothing_else_leaves() {
    let mut w = World::new();
    w.tickets.clear();
    w.utxos.clear();
    for i in 1..=40u8 {
        let (frog, fee) = (op(i, 0), op(100 + i, 1));
        w.tickets.push(T { stage: Stage::Resolved, acq: i as u128, dest: ATTACKER.into(), deliver_by: T0 + 7 * DAY, deliver_by_block: (T0 + 7 * DAY) / 12 });
        w.insc.insert(i as u128, insc(i));
        w.loc.insert(insc(i), Location { output: frog, offset: 0, value: 546 });
        w.contents.insert(frog, Contents { inscriptions: vec![insc(i)], runes: vec![], value: 546 });
        w.outputs.insert(frog, (Output { script: ours(), value: 546, confirmed: true }, false));
        w.utxos.push(Utxo { outpoint: fee, value: 20_000, confirmed: true });
        w.contents.insert(fee, Contents { inscriptions: vec![], runes: vec![], value: 20_000 });
    }
    // stage 1's limits (docs/v5/launch-gates.md, G7)
    w.limits = Some(outflow::Limits { per_collection_items: 6, global_items: 12, per_collection_mints: 6, global_mints: 12,
        global_value_wei: 20 * 1_000_000_000_000_000_000, trip_factor: 3 });
    let mut c = Custody::default();
    let lines = w.tick(&mut c, false);
    let signed = c.redemptions.iter().filter(|r| matches!(r.phase, RPhase::Signed { .. })).count();
    assert_eq!((signed, w.sent.len()), (6, 6), "six frogs, not the vault: {lines:?}");
    assert!(w.paused, "the tripwire paused the canister");
    assert!(matches!(w.outflow.freeze, outflow::Freeze::Frozen { cause: outflow::Cause::Tripwire { .. }, .. }));
    assert_eq!(w.asked.len(), 40, "every forged ticket asked once");
    assert_eq!(w.asked.iter().position(|k| k == "ticket:17"), Some(17), "the 18th distinct ask is the one that froze it");
    // days later, the window long past: still frozen, nothing leaves until k guardians resume
    for _ in 0..3 {
        w.now += DAY;
        w.tick(&mut c, false);
    }
    assert_eq!(w.sent.iter().filter(|r| !w.sent[..6].contains(r)).count(), 0, "no new delivery was sent");
    assert_eq!(c.redemptions.iter().filter(|r| matches!(r.phase, RPhase::Signed { .. } | RPhase::Sent { .. })).count(), 6);
}

// ---- H8: conflict-out for leaked signed bytes (docs/v5/custody-hardening.md, H8, resolved 2026-09-25) ---------------------------

/// Deliver ticket 0, let a mempool take it, evict it, and move the clock inside the margin, then tick until the rule has refused
/// it. Returns the leaked delivery's txid and bytes.
fn evicted_then_refused(w: &mut World, c: &mut Custody) -> ([u8; 32], Vec<u8>) {
    w.tick(c, false); // signed, saved, sent
    w.tick(c, false); // a mempool has it: Sent
    let RPhase::Sent { txid, raw, .. } = w.phase(c, 0) else { panic!("{:?}", w.phase(c, 0)) };
    w.mempool.remove(&txid); // evicted from every mempool (a fee spike)
    w.tick(c, false); // Bitcoin no longer shows it: back to Signed
    assert!(matches!(w.phase(c, 0), RPhase::Signed { .. }));
    w.now = T0 + 6 * DAY + 60; // it stays gone, and the ticket is now inside the margin
    w.tick(c, false); // the rule refuses the saved bytes
    (txid, raw)
}

fn conflict_of(c: &Custody, ticket: u64) -> etch_autobuyer::conflict::Conflict {
    match &c.redemptions.iter().find(|r| r.ticket == ticket).unwrap().phase {
        RPhase::Conflicting { conflict, .. } => conflict.clone(),
        p => panic!("not conflicting: {p:?}"),
    }
}

/// The launch gate's own test (docs/v5/launch-gates.md, G4 C5): a delivery that was broadcast, evicted and then refused must end
/// with its fee input spent to custody, and the leaked bytes must be unminable ever after.
#[test]
fn a_delivery_evicted_and_then_refused_ends_with_its_fee_input_spent_to_custody() {
    let mut w = World::new();
    let caps = w.caps.clone();
    custodycap::admit(&caps, &mut w.book, &insc(7), &frogs_key(), 5).unwrap();
    let mut c = Custody::default();
    let (leaked_txid, leaked) = evicted_then_refused(&mut w, &mut c);
    // Conflicting: the delivery is not sent again, and the conflict is signed, saved and sent in the same pass
    let k = conflict_of(&c, 0);
    assert_eq!((k.leaked_txid, k.fee_input, k.item), (leaked_txid, op(0xfe, 1), op(0xf0, 0)));
    let tx = k.tx.clone().expect("built at once");
    assert_eq!(w.saved[&0].phase, w.phase(&c, 0), "saved before it was sent");
    assert_eq!(w.sent.iter().filter(|r| **r == leaked).count(), 1, "the refused delivery went out once, and never again");
    assert_eq!(w.sent.last().unwrap(), &tx.raw);
    assert!(w.mempool.contains(&tx.txid));
    // what it is: the fee input, whole, back to custody; the frog untouched
    let p = parse_signed(&tx.raw).unwrap();
    assert_eq!(World::inputs_of(&tx.raw), vec![op(0xfe, 1)]);
    assert_eq!(p.outputs, vec![(20_000 - tx.fee, ours())]);
    // the fee: the leaked one (3 sat/vB × 212 vB) plus its own 111 vB at 1 sat/vB, above the current 3 × 111
    assert_eq!(tx.fee, 3 * 212 + 111);
    assert_eq!(c.redemptions[0].reserved(), vec![op(0xfe, 1)], "the fee input stays claimed until it settles");
    // someone who kept the leaked bytes re-broadcasts them: they cannot displace a conflict that pays more
    w.broadcast(&leaked);
    assert!(!w.mempool.contains(&leaked_txid) && w.mempool.contains(&tx.txid));
    // a block: the conflict confirms, the record says so, and nothing is signed again
    w.mine();
    let signs = w.signs;
    w.tick(&mut c, false);
    assert!(matches!(w.phase(&c, 0), RPhase::Refused { ref reason }
        if reason.contains(&format!("conflicted out in {}", hex::encode(tx.txid)))), "{:?}", w.phase(&c, 0));
    assert!(c.redemptions[0].finished() && c.redemptions[0].reserved().is_empty());
    // the leaked bytes can never be mined now: its fee input is spent in a block
    w.broadcast(&leaked);
    w.mine();
    assert!(!w.confirmed.contains(&leaked_txid), "the leaked delivery is dead");
    assert_eq!(w.spender(&op(0xfe, 1)), Some((tx.txid, true)), "the fee input is spent to custody");
    assert_eq!(w.spender(&op(0xf0, 0)), None, "the frog never moved");
    assert!(w.book.contains(&insc(7)), "and it is still counted as held (H2)");
    for _ in 0..3 {
        w.tick(&mut c, false);
    }
    assert_eq!(w.signs, signs, "a settled conflict signs nothing more");
}

#[test]
fn the_leaked_delivery_confirming_first_takes_the_record_path_and_is_a_lost_race_when_the_ticket_refunds() {
    let mut w = World::new();
    let caps = w.caps.clone();
    custodycap::admit(&caps, &mut w.book, &insc(7), &frogs_key(), 5).unwrap();
    let mut c = Custody::default();
    let (leaked_txid, _) = evicted_then_refused(&mut w, &mut c);
    let conflict = conflict_of(&c, 0).tx.unwrap().txid;
    // a miner who kept the leaked delivery mines it instead of our conflict
    w.mempool.remove(&conflict);
    w.confirmed.insert(leaked_txid);
    w.tick(&mut c, false);
    assert_eq!(w.phase(&c, 0), RPhase::Confirmed { txid: leaked_txid }, "a confirmed delivery like any other");
    // the ticket was refunded (it was inside its margin): the redeemer has both, recorded loudly, never retried
    w.tickets[0].stage = Stage::Refunded;
    w.tick(&mut c, false);
    assert_eq!(w.phase(&c, 0), RPhase::LostRace { txid: leaked_txid });
    assert!(!w.book.contains(&insc(7)), "the frog left custody: released under the caps");
}

#[test]
fn a_leaked_delivery_with_no_change_is_conflicted_out_with_one_more_clean_custody_utxo() {
    let mut w = World::new();
    // a fee input too small for change at a high rate: the leaked delivery gave all of it to the miner (fee inputs under
    // 5,000 sats are never chosen at all since audit D, H-4)
    w.rate = 25;
    w.utxos[1].value = 5_000;
    w.contents.get_mut(&op(0xfe, 1)).unwrap().value = 5_000;
    let mut c = Custody::default();
    let (leaked_txid, leaked) = evicted_then_refused(&mut w, &mut c);
    assert_eq!(parse_signed(&leaked).unwrap().outputs.len(), 1, "no change output");
    // nothing else in custody: the conflict waits, signs nothing, and says why
    let lines = w.tick(&mut c, false);
    assert!(conflict_of(&c, 0).tx.is_none());
    assert!(lines.iter().any(|l| l.contains("tops custody up")), "{lines:?}");
    // custody holds a runes UTXO (smaller, so it sorts first), an inscribed one and a clean one: only the clean one is taken
    for (tag, value, runes, insc_) in [(0xab, 3_000, true, false), (0xac, 4_000, false, true), (0xaa, 5_000, false, false)] {
        w.utxos.push(Utxo { outpoint: op(tag, 0), value, confirmed: true });
        w.contents.insert(op(tag, 0), Contents { inscriptions: if insc_ { vec![insc(tag)] } else { vec![] },
            runes: if runes { vec!["BITBLOCK".into()] } else { vec![] }, value });
    }
    w.tick(&mut c, false);
    let tx = conflict_of(&c, 0).tx.expect("built");
    assert_eq!(World::inputs_of(&tx.raw), vec![op(0xfe, 1), op(0xaa, 0)]);
    assert_eq!(tx.fee, 5_000 + 169, "the leaked 5,000 plus its own 169 vB");
    assert_eq!(parse_signed(&tx.raw).unwrap().outputs, vec![(5_000 + 5_000 - 5_169, ours())]);
    assert_eq!(c.redemptions[0].reserved(), vec![op(0xfe, 1), op(0xaa, 0)], "both inputs claimed");
    w.mine();
    w.tick(&mut c, false);
    assert!(matches!(w.phase(&c, 0), RPhase::Refused { .. }));
    w.broadcast(&leaked);
    w.mine();
    assert!(!w.confirmed.contains(&leaked_txid));
}

/// Two leaked deliveries at once: each conflict takes its own inputs, the second never the first's, and a conflict already
/// built is sent again as the same bytes, never signed twice.
#[test]
fn two_conflicts_at_once_never_share_an_input_and_neither_is_signed_twice() {
    let mut w = World::new();
    w.rate = 25;
    w.utxos[1].value = 5_000;
    w.contents.get_mut(&op(0xfe, 1)).unwrap().value = 5_000;
    let frog2 = op(0xf2, 0);
    w.tickets.push(T { stage: Stage::Resolved, acq: 8, dest: DEST.into(), deliver_by: T0 + 7 * DAY, deliver_by_block: (T0 + 7 * DAY) / 12 });
    w.insc.insert(8, insc(8));
    w.loc.insert(insc(8), Location { output: frog2, offset: 0, value: 546 });
    w.contents.insert(frog2, Contents { inscriptions: vec![insc(8)], runes: vec![], value: 546 });
    w.outputs.insert(frog2, (Output { script: ours(), value: 546, confirmed: true }, false));
    w.utxos.push(Utxo { outpoint: frog2, value: 546, confirmed: true });
    w.utxos.push(Utxo { outpoint: op(0xfd, 1), value: 5_000, confirmed: true });
    w.contents.insert(op(0xfd, 1), Contents { inscriptions: vec![], runes: vec![], value: 5_000 });
    let mut c = Custody::default();
    w.tick(&mut c, false);
    w.tick(&mut c, false);
    let ids: Vec<[u8; 32]> = [0, 1].iter().map(|t| match w.phase(&c, *t) { RPhase::Sent { txid, .. } => txid, p => panic!("{p:?}") }).collect();
    w.mempool.clear();
    w.tick(&mut c, false);
    // one extra clean UTXO: the first conflict takes it, the second waits rather than share it
    w.utxos.push(Utxo { outpoint: op(0xaa, 0), value: 5_000, confirmed: true });
    w.contents.insert(op(0xaa, 0), Contents { inscriptions: vec![], runes: vec![], value: 5_000 });
    w.now = T0 + 6 * DAY + 60;
    w.tick(&mut c, false);
    let (a, b) = (conflict_of(&c, 0), conflict_of(&c, 1));
    assert_ne!(a.fee_input, b.fee_input);
    assert_eq!(World::inputs_of(&a.tx.clone().unwrap().raw), vec![a.fee_input, op(0xaa, 0)]);
    assert!(b.tx.is_none(), "the only extra UTXO is claimed by the first conflict");
    // a second clean UTXO: the second conflict takes that one
    w.utxos.push(Utxo { outpoint: op(0xa9, 0), value: 6_000, confirmed: true });
    w.contents.insert(op(0xa9, 0), Contents { inscriptions: vec![], runes: vec![], value: 6_000 });
    w.tick(&mut c, false);
    let b_fee = b.fee_input;
    let b = conflict_of(&c, 1).tx.expect("built");
    assert_eq!(World::inputs_of(&b.raw), vec![b_fee, op(0xa9, 0)]);
    // evicted again and again: each pass sends the same bytes, and no new signature is made
    let signs = w.signs;
    for _ in 0..3 {
        w.mempool.clear();
        w.tick(&mut c, false);
    }
    assert_eq!(w.signs, signs);
    assert_eq!(w.sent.iter().filter(|r| **r == b.raw).count(), 4, "sent once, then again on each of three passes");
    w.mine();
    w.tick(&mut c, false);
    for (t, id) in [0u64, 1].iter().zip(&ids) {
        assert!(matches!(w.phase(&c, *t), RPhase::Refused { .. }), "{:?}", w.phase(&c, *t));
        assert!(!w.confirmed.contains(id));
    }
}

#[test]
fn a_fee_input_already_spent_by_a_confirmed_third_transaction_settles_without_a_conflict() {
    let mut w = World::new();
    let mut c = Custody::default();
    w.tick(&mut c, false);
    w.tick(&mut c, false);
    let RPhase::Sent { txid, .. } = w.phase(&c, 0) else { panic!() };
    w.mempool.remove(&txid);
    w.tick(&mut c, false);
    w.now = T0 + 6 * DAY + 60;
    w.fail_sign = true; // the refusal lands while the subnet does not sign: Conflicting, nothing built
    w.tick(&mut c, false);
    assert!(conflict_of(&c, 0).tx.is_none());
    // meanwhile the fee input is spent by a transaction that is not ours to know about, first unconfirmed: wait
    w.fail_sign = false;
    w.third_party.insert(op(0xfe, 1), ([0x77; 32], false));
    let signs = w.signs;
    let lines = w.tick(&mut c, false);
    assert!(conflict_of(&c, 0).tx.is_none() && w.signs == signs, "nothing signed against an unknown spend: {lines:?}");
    // then confirmed: the leaked bytes can never be mined, so the record settles, naming the spend
    w.third_party.insert(op(0xfe, 1), ([0x77; 32], true));
    w.tick(&mut c, false);
    assert!(matches!(w.phase(&c, 0), RPhase::Refused { ref reason } if reason.contains(&hex::encode([0x77; 32]))), "{:?}", w.phase(&c, 0));
    assert_eq!(w.signs, signs, "no conflict was needed");
}

#[test]
fn a_leaked_delivery_back_in_a_mempool_is_replaced_by_the_conflict() {
    let mut w = World::new();
    let mut c = Custody::default();
    w.tick(&mut c, false);
    w.tick(&mut c, false);
    let RPhase::Sent { txid, raw, .. } = w.phase(&c, 0) else { panic!() };
    w.mempool.remove(&txid);
    w.tick(&mut c, false);
    w.now = T0 + 6 * DAY + 60;
    w.fail_sign = true;
    w.tick(&mut c, false);
    assert!(matches!(w.phase(&c, 0), RPhase::Conflicting { .. }));
    // someone re-broadcasts the leaked bytes: the fee input now reads spent, by the leaked delivery itself
    w.broadcast(&raw);
    assert!(w.mempool.contains(&txid));
    w.fail_sign = false;
    w.tick(&mut c, false);
    let tx = conflict_of(&c, 0).tx.expect("built all the same: it is the replacement");
    assert!(w.mempool.contains(&tx.txid) && !w.mempool.contains(&txid), "the conflict pays more and replaces it");
    w.mine();
    w.tick(&mut c, false);
    assert!(matches!(w.phase(&c, 0), RPhase::Refused { .. }));
}

/// Audit R2, M-1 (c), replacing "the conflict waits for the pause": while the buyer is paused (a guardian, or an H3 trip)
/// nothing new leaves custody (no delivery signed, bumped or re-sent), but a delivery that already left and that the rule now
/// refuses is conflicted out, because the conflict pays only custody. It never asks or records the limiter (a limiter at its
/// limit does not hold it back), and the item stays counted under the caps (H2).
#[test]
fn the_conflict_runs_while_paused_never_touches_the_limiter_and_keeps_the_item_counted() {
    let mut w = World::new();
    w.limits = Some(outflow::Limits::stage0());
    let caps = w.caps.clone();
    custodycap::admit(&caps, &mut w.book, &insc(7), &frogs_key(), 5).unwrap();
    let mut c = Custody::default();
    w.tick(&mut c, false);
    w.tick(&mut c, false);
    let RPhase::Sent { txid, raw, .. } = w.phase(&c, 0) else { panic!() };
    w.mempool.remove(&txid);
    w.tick(&mut c, false);
    // the day's limit is used up by other deliveries asking: an outflow wait must not hold the conflict back
    for k in ["ticket:90", "ticket:91", "ticket:92"] {
        w.outflow.record(k, &frogs_key(), outflow::Flow::Item, 1, w.now);
    }
    let (asked, recorded) = (w.asked.clone(), w.recorded.clone());
    // paused (a guardian, or a trip) before the delivery comes back into view: its saved bytes are not re-sent
    w.paused = true;
    let sends = w.sent.len();
    for _ in 0..3 {
        let lines = w.tick(&mut c, false);
        assert!(lines.iter().any(|l| l.contains("paused")), "{lines:?}");
    }
    assert_eq!(w.sent.len(), sends, "paused: the delivery is not re-sent");
    // inside the margin, still paused: the refusal, and the conflict-out signed and sent
    w.now = T0 + 6 * DAY + 60;
    let signs = w.signs;
    let lines = w.tick(&mut c, false);
    let tx = conflict_of(&c, 0).tx.unwrap_or_else(|| panic!("the conflict is built while paused: {lines:?}"));
    assert_eq!(w.signs, signs + 1, "one signature: the conflict's");
    assert!(w.sent.iter().all(|r| *r != raw || w.sent.iter().filter(|x| **x == raw).count() == 1), "the delivery never again");
    assert!(w.mempool.contains(&tx.txid));
    w.mine();
    w.tick(&mut c, false);
    assert!(matches!(w.phase(&c, 0), RPhase::Refused { .. }), "{:?}", w.phase(&c, 0));
    assert_eq!((w.asked.clone(), w.recorded.clone()), (asked, recorded), "the conflict never asked or recorded the limiter");
    assert_eq!(w.recorded, vec!["ticket:0".to_string()], "the delivery counted once when it was saved; nothing more");
    assert!(w.book.contains(&insc(7)), "the frog never left: still held under the caps");
}

/// Canister review2 N-2, the wrapper side (r3-contracts): a resolved ticket refunds early only once the custodian attests that no
/// delivery can confirm. A refusal with nothing ever broadcast is attested at once; a conflicted-out one once its conflict is deep
/// (`attest` in `review2.rs`); a canister that is not the custodian sends nothing.
#[test]
fn a_refused_redemption_is_attested_undeliverable_to_the_wrapper_and_only_by_the_custodian() {
    let mut w = World::new();
    w.keeper = true;
    w.now = T0 + 6 * DAY + 60; // inside the margin before anything was signed
    let mut c = Custody::default();
    w.tick(&mut c, false);
    assert!(matches!(w.phase(&c, 0), RPhase::Refused { .. }));
    assert_eq!(c.redemptions[0].attest, Some(redeem::Attest::Due { deep: None }));
    w.tick(&mut c, false);
    let Some(redeem::Attest::Sent { hash, .. }) = c.redemptions[0].attest.clone() else { panic!("{:?}", c.redemptions[0].attest) };
    assert_eq!(w.evm_sent.len(), 1);
    let call = redeem::mark_undeliverable(0);
    assert!(w.evm_sent[0].windows(call.len()).any(|x| x == call.as_slice()), "markUndeliverable(0)");
    assert_eq!(hex::encode(&call[..4]), hex::encode(&etch_autobuyer::evmtx::keccak(b"markUndeliverable(uint256)")[..4]));
    assert!(c.redemptions[0].evm_pending(), "one EVM transaction of ours at a time");
    w.evm_mined.insert(hash, true);
    w.tick(&mut c, false);
    assert!(matches!(c.redemptions[0].attest, Some(redeem::Attest::Done { ref note }) if note.contains("mined")));
    w.tick(&mut c, false);
    assert_eq!(w.evm_sent.len(), 2, "sent once more while waiting for the receipt, then nothing");
    // not the custodian: nothing is sent, and the ticket refunds after the wrapper's late-delivery window
    let mut w = World::new();
    w.now = T0 + 6 * DAY + 60;
    let mut c = Custody::default();
    w.tick(&mut c, false);
    w.tick(&mut c, false);
    assert!(matches!(c.redemptions[0].attest, Some(redeem::Attest::Done { ref note }) if note.contains("not the wrapper's custodian")));
    assert!(w.evm_sent.is_empty());
}

/// Audit R2, M-1 (c): a delivery Bitcoin confirms while the buyer is paused is still recorded (`recordDelivery` moves nothing, and
/// it is what stops a refund of a delivered ticket), and nothing new is signed on Bitcoin meanwhile.
#[test]
fn a_delivery_confirmed_while_paused_is_still_recorded_and_nothing_new_leaves() {
    let mut w = World::new();
    w.keeper = true;
    let mut c = Custody::default();
    w.tick(&mut c, false);
    w.tick(&mut c, false);
    assert!(matches!(w.phase(&c, 0), RPhase::Sent { .. }));
    w.paused = true;
    w.mine();
    let (signs, sends) = (w.signs, w.sent.len());
    w.tick(&mut c, false);
    assert!(matches!(w.phase(&c, 0), RPhase::Confirmed { .. }), "{:?}", w.phase(&c, 0));
    w.tick(&mut c, false);
    assert!(matches!(w.phase(&c, 0), RPhase::Recording { .. }), "{:?}", w.phase(&c, 0));
    assert_eq!(w.evm_sent.len(), 1, "recordDelivery sent while paused");
    assert_eq!((w.signs, w.sent.len()), (signs, sends), "no Bitcoin signature or send while paused");
}

/// A restart or an upgrade in the middle of a conflict: the saved record is all there is. Signed-and-saved bytes are sent
/// again, never re-signed; a signature that never got saved is simply made again; the record survives the stable-memory
/// encoding an upgrade uses.
#[test]
fn a_restart_or_an_upgrade_mid_conflict_resends_the_saved_conflict_and_never_signs_twice() {
    let mut w = World::new();
    w.fail_sign = false;
    let mut c = Custody::default();
    w.tick(&mut c, false);
    w.tick(&mut c, false);
    let RPhase::Sent { txid, .. } = w.phase(&c, 0) else { panic!() };
    w.mempool.remove(&txid);
    w.tick(&mut c, false);
    w.now = T0 + 6 * DAY + 60;
    // the subnet fails mid-signing the conflict: nothing is saved or sent
    w.fail_sign = true;
    let sends = w.sent.len();
    w.tick(&mut c, false);
    assert!(conflict_of(&c, 0).tx.is_none() && w.sent.len() == sends, "no signature, no save, no send");
    // it answers again, but the canister dies between the save and the send (the send never reaches a node)
    w.fail_sign = false;
    w.accept = false;
    w.tick(&mut c, false);
    let tx = conflict_of(&c, 0).tx.expect("saved");
    assert!(!w.mempool.contains(&tx.txid));
    let signs = w.signs;
    // an upgrade: the custody state through the candid encoding pre_upgrade/post_upgrade use, then a restart from it
    let saved = Custody { cursor: w.cursor, redemptions: w.saved.values().cloned().collect(), logs: c.logs.clone(), colls: None };
    let back: Custody = candid::decode_one(&candid::encode_one(&saved).unwrap()).unwrap();
    assert_eq!(back, saved, "the Conflicting record survives the upgrade encoding");
    let mut c = back;
    w.accept = true;
    w.tick(&mut c, false);
    assert_eq!(w.signs, signs, "nothing signed again");
    assert_eq!(w.sent.last().unwrap(), &tx.raw, "the SAME conflict bytes");
    assert!(w.mempool.contains(&tx.txid));
    w.mine();
    w.tick(&mut c, false);
    assert!(matches!(w.phase(&c, 0), RPhase::Refused { .. }));
}

// ---- audit D, M-1: a delivery stuck in a mempool is re-priced, then conflicted out before its deadline --------------------------

/// AUDIT-D M-1 regression (the PoC `audit_d_a_delivery_pending_past_its_deadline_is_never_bumped_or_conflicted_and_ends_a_lost_race`,
/// now failing as it should). Fees spike and the delivery sits in a mempool for the rest of the week. Every six hours it is
/// re-priced (same frog, same fee input, same destination, more fee from the change) up to what the change allows; a day before
/// the deadline the rule refuses it and the conflict-out replaces it in the mempool. The redeemer refunds after `deliverBy`, a
/// block comes, and what confirms is the conflict: the frog never left, and no LOST RACE.
#[test]
fn audit_d_a_delivery_stuck_in_a_mempool_is_re_priced_then_conflicted_out_before_its_deadline_and_never_lost() {
    let mut w = World::new();
    let caps = w.caps.clone();
    custodycap::admit(&caps, &mut w.book, &insc(7), &frogs_key(), 5).unwrap();
    let mut c = Custody::default();
    w.tick(&mut c, false);
    let RPhase::Signed { txid: first, .. } = w.phase(&c, 0) else { panic!("{:?}", w.phase(&c, 0)) };
    w.tick(&mut c, false);
    assert!(matches!(w.phase(&c, 0), RPhase::Sent { .. }), "in a mempool");
    // tick every 10 minutes, no block, up to the deadline
    let mut versions: Vec<[u8; 32]> = vec![first];
    while w.now < T0 + 7 * DAY {
        w.now += 600;
        w.tick(&mut c, false);
        if let RPhase::Signed { txid, .. } | RPhase::Sent { txid, .. } = w.phase(&c, 0) {
            if !versions.contains(&txid) {
                versions.push(txid);
            }
        }
    }
    assert!(versions.len() >= 5, "re-priced every six hours while the change allowed: {} versions", versions.len());
    let fees: Vec<u64> = versions.iter().map(|t| w.fee_of(&w.txs[t])).collect();
    assert!(fees.windows(2).all(|f| f[1] > f[0]), "each version pays more than the last: {fees:?}");
    assert!(*fees.last().unwrap() <= redeem::MAX_BUMP_FEE_SATS, "{fees:?}");
    for t in &versions {
        let raw = &w.txs[t];
        assert_eq!(World::inputs_of(raw), vec![op(0xf0, 0), op(0xfe, 1)], "the same frog and fee input");
        assert_eq!(parse_signed(raw).unwrap().outputs[0], (546, script_of(DEST, "bc").unwrap()), "the same frog to the same redeemer");
    }
    // a day before the deadline the rule refused it: the conflict replaced the delivery in the mempool
    let k = conflict_of(&c, 0);
    let conflict = k.tx.clone().expect("built");
    assert_eq!(k.earlier.clone().unwrap_or_default().len(), versions.len() - 1, "every earlier version is known to the conflict");
    assert!(w.mempool.contains(&conflict.txid) && versions.iter().all(|t| !w.mempool.contains(t)), "no version of the delivery is left");
    // the redeemer refunds; someone re-broadcasts every version they kept; a block
    w.now += 600;
    w.tickets[0].stage = Stage::Refunded;
    for t in &versions {
        let raw = w.txs[t].clone();
        w.broadcast(&raw);
    }
    w.mine();
    w.tick(&mut c, false);
    assert!(matches!(w.phase(&c, 0), RPhase::Refused { ref reason } if reason.contains("conflicted out")), "{:?}", w.phase(&c, 0));
    assert!(versions.iter().all(|t| !w.confirmed.contains(t)), "no version of the delivery ever confirmed");
    assert_eq!(w.spender(&op(0xf0, 0)), None, "the frog never moved");
    assert!(w.book.contains(&insc(7)), "and it is still held (H2)");
}

/// M-1: an EARLIER, lower-fee version confirming is the delivery: it is recorded with its own txid (what `recordDelivery`
/// must name), whichever version the record held.
#[test]
fn an_earlier_version_of_a_re_priced_delivery_confirming_is_recorded_with_its_own_txid() {
    let mut w = World::new();
    let mut c = Custody::default();
    w.tick(&mut c, false);
    let RPhase::Signed { txid: first, raw: first_raw, .. } = w.phase(&c, 0) else { panic!() };
    w.tick(&mut c, false);
    w.now += redeem::BUMP_AFTER_SECS;
    w.tick(&mut c, false);
    let RPhase::Signed { txid: second, bump: Some(b), .. } = w.phase(&c, 0) else { panic!("{:?}", w.phase(&c, 0)) };
    assert_ne!(second, first);
    assert_eq!(b.earlier, vec![first]);
    // a miner who kept the first version mines it
    w.mempool.clear();
    w.txs.insert(first, first_raw);
    w.confirmed.insert(first);
    w.tick(&mut c, false);
    assert_eq!(w.phase(&c, 0), RPhase::Confirmed { txid: first }, "the version that confirmed, not the one the record held");
    assert_eq!(c.redemptions[0].record_owed(), Some(first));
}
