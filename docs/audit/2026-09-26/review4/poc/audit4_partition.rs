//! R4 check: R3-C7 under a mempool partition. The harness is copied from tests/review3.rs (World), extended with a pinned
//! delivery held at the MINERS while the explorers (what the canister reads) hold our conflict. A pass shows the behaviour described.
#![allow(unused)]

mod fakerpc;

use std::cell::RefCell;
use std::future::Future;
use std::pin::pin;
use std::rc::Rc;
use std::task::{Context, Poll, Waker};

use etch_autobuyer::custody::{self, Contents, Location, Outpoint, Output, Utxo};
use etch_autobuyer::delivery::{Stage, TicketRead};
use etch_autobuyer::evmread::{self, LogsError, Rpc, SIZE_REJECT};
use etch_autobuyer::looper::{BtcView, EvmView};
use etch_autobuyer::redeem::{self, Attest, Custody, CustodyIo, RPhase, Redemption};
use etch_autobuyer::ticketlog::{self, TicketLogs, MAX_TRIES_PER_TICK};
use etch_autobuyer::{conflict, custodycap, outbound, outflow};
use fakerpc::{Chain, FakeRpc};

fn block_on<F: Future>(f: F) -> F::Output {
    let mut f = pin!(f);
    let mut cx = Context::from_waker(Waker::noop());
    loop {
        if let Poll::Ready(v) = f.as_mut().poll(&mut cx) {
            return v;
        }
    }
}

const W: [u8; 20] = [0xcc; 20];
const DEST: &str = "bc1p5cyxnuxmeuwuvkwfem96lqzszd02n6xdcjrs20cac6yqjjwudpxqkedrcr";
const HEAD: u64 = 72_856_687;
const T0: u64 = 1_790_000_000;
const DAY: u64 = 86_400;
const DELIVER_BY: u64 = T0 + 7 * DAY;

fn ours() -> Vec<u8> {
    [vec![0x51, 0x20], vec![0x11; 32]].concat()
}
fn insc(n: u8) -> String {
    format!("{}i0", format!("{n:02x}").repeat(32))
}
fn display_txid(raw: &[u8]) -> [u8; 32] {
    let mut t = etch_autobuyer::btcsend::parse_signed(raw).unwrap().txid;
    t.reverse();
    t
}

const FROG: Outpoint = Outpoint { txid: [0xf0; 32], vout: 0 };

#[derive(Clone, Debug)]
struct Mem {
    txid: [u8; 32],
    delivery: bool,
    confirmed: bool,
}

/// One fee input, one frog. `pin`: R2's mempool — the first delivery is pinned, and NO replacement of it is ever accepted (the
/// strongest pin: a fee spike above anything the conflict can pay). `blocks_take_conflict`: a block takes our conflict when it is
/// in a mempool (R3-C7's test turns that off). `rate`: the explorers' fee estimate, sat/vB.
struct World {
    blocks_take_conflict: bool,
    /// R4: the absolute fee of the pinned delivery package the MINERS hold (0: no partition). A block takes our conflict only
    /// when it pays more than this; the explorers meanwhile show our conflict in their mempools.
    pin_abs: u64,
    delivery_txid: Option<[u8; 32]>,
    /// control: the explorers do NOT show our unconfirmed conflict (round 3's behaviour for every conflict: not `held`)
    hide_conflict: bool,
    conflict_fee: u64,
    max_conflict_fee: u64,
    rate: u64,
    now: u64,
    fee: Utxo,
    spender: Option<Mem>,
    pin: bool,
    paused: bool,
    /// the gated wrapper's state
    stage: Stage,
    undeliverable: bool,
    attests_sent: usize,
    records_sent: usize,
    signs: usize,
    rejected: usize,
    lines: Vec<String>,
    caps: custodycap::CapPolicy,
    book: custodycap::Book,
}

impl World {
    fn new(pin: bool) -> (World, Custody) {
        let mut logs = TicketLogs::default();
        logs.ingest(&[fakerpc::redeemed(&W, 0, DEST, 10), fakerpc::resolved(&W, 0, 7, 11)]).unwrap();
        (World { blocks_take_conflict: true, pin_abs: 0, delivery_txid: None, hide_conflict: false, conflict_fee: 0, max_conflict_fee: 0, rate: 100, now: T0, fee: Utxo { outpoint: Outpoint { txid: [0xfe; 32], vout: 1 }, value: 150_000, confirmed: true }, spender: None,
            pin, paused: false, stage: Stage::Resolved, undeliverable: false, attests_sent: 0, records_sent: 0, signs: 0, rejected: 0,
            lines: vec![],
            caps: custodycap::CapPolicy { collections: vec![custodycap::CollectionCap { collection: custodycap::btc_key(&W), max_items: 1_000,
                max_value_wei: u128::MAX / 4, max_item_wei: u128::MAX / 4 }], global_max_items: 1_000, global_max_value_wei: u128::MAX / 4 },
            book: Default::default() },
         Custody { cursor: 0, redemptions: vec![], logs: Some(logs) })
    }
    fn tick(&mut self, c: &mut Custody) {
        let lines = block_on(redeem::tick(self, c, false));
        self.lines.extend(lines);
        // a block: a conflict in the mempool confirms (the market pays it); a pinned delivery does not
        let (pin_abs, fee) = (self.pin_abs, self.conflict_fee);
        if let Some(m) = self.spender.as_mut().filter(|m| !m.confirmed && !m.delivery && self.blocks_take_conflict
            && (pin_abs == 0 || fee > pin_abs)) {
            m.confirmed = true;
        }
    }
    /// The gated wrapper's `refund` rule for a Resolved ticket (r3-contracts `Wrapper.refund`).
    fn refundable(&self) -> bool {
        self.stage == Stage::Resolved
            && ((self.undeliverable && self.now >= DELIVER_BY + 72 * 3600) || self.now >= DELIVER_BY + 30 * DAY)
    }
}

impl CustodyIo for World {
    fn now(&self) -> u64 { self.now }
    fn chain_id(&self) -> u64 { 4663 }
    async fn ticket_count(&mut self) -> Result<(u64, u64), String> { Ok((1, HEAD - 1_500)) }
    async fn read_ticket(&mut self, _id: u64, _f: bool) -> Result<TicketRead, String> {
        Ok(TicketRead { stage: self.stage, acquisition_id: 7, destination: DEST.into(), deliver_by: DELIVER_BY,
            deliver_by_block: DELIVER_BY / 12, chain_time: self.now, parent_block: self.now / 12, read_at: self.now })
    }
    async fn inscription_of(&mut self, _a: u128) -> Result<String, String> { Ok(insc(7)) }
    async fn locate(&mut self, _i: &str) -> Result<Option<Location>, String> { Ok(Some(Location { output: FROG, offset: 0, value: 546 })) }
    async fn contents(&mut self, o: &Outpoint) -> Result<Option<Contents>, String> {
        Ok(Some(if *o == FROG { Contents { inscriptions: vec![insc(7)], runes: vec![], value: 546 } }
            else { Contents { inscriptions: vec![], runes: vec![], value: self.fee.value } }))
    }
    async fn output(&mut self, o: &Outpoint) -> Result<(Output, bool), String> {
        if *o == FROG {
            return Ok((Output { script: ours(), value: 546, confirmed: true }, self.spender.as_ref().is_some_and(|m| m.delivery)));
        }
        if *o == self.fee.outpoint {
            return Ok((Output { script: ours(), value: self.fee.value, confirmed: true }, self.spender.is_some()));
        }
        Err("unknown output".into())
    }
    async fn our_utxos(&mut self) -> Result<Vec<Utxo>, String> {
        Ok(if self.spender.is_none() { vec![self.fee.clone()] } else { vec![] })
    }
    async fn fee_rate(&mut self) -> Result<u64, String> { Ok(self.rate) }
    async fn our_script(&mut self) -> Result<Vec<u8>, String> { Ok(ours()) }
    fn btc_hrp(&self) -> Result<&'static str, String> { Ok("bc") }
    async fn sign_taproot(&mut self, _h: [u8; 32]) -> Result<Vec<u8>, String> {
        self.signs += 1;
        Ok(vec![0x5a; 64])
    }
    async fn btc_send(&mut self, raw: &[u8]) -> Result<(), String> {
        let id = display_txid(raw);
        let t = outbound::parse_tx(raw)?;
        let delivery = t.inputs.iter().any(|i| custody::outpoint_of(&etch_autobuyer::psbt::TxIn { txid: i.txid, vout: i.vout, sequence: 0 }) == FROG);
        if delivery {
            self.delivery_txid.get_or_insert(id);
        } else {
            let out: u64 = t.outputs.iter().map(|o| o.value).sum();
            self.conflict_fee = self.fee.value.saturating_sub(out);
            self.max_conflict_fee = self.max_conflict_fee.max(self.conflict_fee);
        }
        match self.spender.clone() {
            None => self.spender = Some(Mem { txid: id, delivery, confirmed: false }),
            Some(m) if m.txid == id || m.confirmed => {}
            Some(_) if self.pin => self.rejected += 1, // R2: every replacement of the pinned delivery is refused
            Some(_) => self.spender = Some(Mem { txid: id, delivery, confirmed: false }),
        }
        Ok(())
    }
    async fn btc_seen(&mut self, t: [u8; 32]) -> BtcView {
        match &self.spender {
            Some(m) if m.txid == t && !m.confirmed && !m.delivery && self.hide_conflict => BtcView::NotYet,
            Some(m) if m.txid == t => if m.confirmed { BtcView::Confirmed } else { BtcView::Mempool },
            _ => BtcView::NotYet,
        }
    }
    async fn may_record(&mut self) -> Result<bool, String> { Ok(true) }
    fn wrapper(&self) -> [u8; 20] { W }
    async fn evm_nonce(&mut self) -> Result<u64, String> { Ok(0) }
    async fn base_fee(&mut self) -> Result<u128, String> { Ok(1) }
    async fn estimate_gas(&mut self, _t: [u8; 20], _v: u128, _d: &[u8]) -> Result<u64, String> { Ok(1) }
    async fn sign_evm(&mut self, _h: [u8; 32]) -> Result<(u8, [u8; 32], [u8; 32]), String> { Ok((0, [1; 32], [2; 32])) }
    async fn send_evm(&mut self, raw: &[u8]) -> Result<(), String> {
        // the gated wrapper: markUndeliverable (custodian, Resolved) or recordDelivery (custodian, Resolved)
        let mark = redeem::mark_undeliverable(0);
        if raw.windows(4).any(|w| w == &mark[..4]) {
            self.attests_sent += 1;
            if self.stage == Stage::Resolved {
                self.undeliverable = true;
            }
        } else {
            self.records_sent += 1;
            if self.stage == Stage::Resolved {
                self.stage = Stage::Delivered;
            }
        }
        Ok(())
    }
    async fn evm_seen(&mut self, _h: [u8; 32]) -> EvmView { EvmView::Mined { success: true } }
    fn save(&mut self, _r: &Redemption) {}
    fn save_cursor(&mut self, _c: u64) {}
    fn with_caps<R>(&mut self, f: impl FnOnce(&custodycap::CapPolicy, &mut custodycap::Book) -> R) -> R { f(&self.caps, &mut self.book) }
    fn outflow_check(&mut self, _k: &str, _c: &str, _f: outflow::Flow, _a: u128) -> Result<(), String> { Ok(()) }
    fn outflow_record(&mut self, _k: &str, _c: &str, _f: outflow::Flow, _a: u128) {}
    async fn scan_ticket_logs(&mut self, _l: &mut TicketLogs, _w: Option<&[u64]>) -> Result<String, String> { Ok("nothing new".into()) }
    fn outbound_policy(&self, s: &[u8]) -> outbound::BtcPolicy {
        outbound::BtcPolicy { custody_script: s.to_vec(), max_fee_sats: outbound::MAX_FEE_SATS, require_finalized: true, require_event: true }
    }
    fn paused(&self) -> bool { self.paused }
    async fn outspend(&mut self, o: &Outpoint) -> Result<Option<([u8; 32], bool)>, String> {
        let Some(m) = &self.spender else { return Ok(None) };
        if *o == self.fee.outpoint || (*o == FROG && m.delivery) {
            return Ok(Some((m.txid, m.confirmed)));
        }
        Ok(None)
    }
}

/// R3-C7 under a partition: the explorers (the canister's only view) hold our conflict in their mempools, so `held` is true
/// and the conflict only follows today's rate; the miners hold the redeemer's pinned delivery package (absolute fee `pin_abs`),
/// which our conflict must out-pay in absolute fee to replace there. Before R4 the conflict escalated x1.5 every 2 h towards
/// 100,000 sats and out-paid any pin under that within hours; now it stays at today's rate for 30 days, the gated wrapper
/// refunds at deliverBy + 30 d, and the delivery confirms: LostRace against a pin of only 20,000 sats (R3's bound: >= ~75k sats
/// of child fee at risk). The hard part for the attacker is the partition itself (every miner holding the pin, none the
/// conflict, the explorers the opposite, across two 14-day expiries): see the report.
#[test]
fn audit4_r3c7_a_partition_keeps_the_conflict_at_todays_rate_and_a_cheap_pin_wins_after_thirty_days() {
    let (mut w, mut c) = World::new(false);
    w.rate = 10;
    w.pin_abs = 20_000;
    let mut refunded_at = None;
    for i in 0..(40 * 144) {
        w.now = T0 + i * 600;
        // paused between the first send and the margin (as review3's R3-C7 test), so the delivery is not bumped first
        w.paused = i > 0 && conflict_of(&c).is_none() && w.max_conflict_fee == 0;
        if refunded_at.is_none() && w.refundable() {
            w.stage = Stage::Refunded;
            refunded_at = Some(w.now);
            // the miners' pinned delivery confirms
            let d = w.delivery_txid.expect("a delivery was sent");
            w.spender = Some(Mem { txid: d, delivery: true, confirmed: true });
        }
        w.tick(&mut c);
    }
    println!("R4 R3-C7 partition: max conflict fee over 40 days {} sats (pin at miners {} sats); refunded at deliverBy + {:?} s; phase {:?}",
        w.max_conflict_fee, w.pin_abs, refunded_at.map(|r| r - DELIVER_BY), c.redemptions[0].phase);
    assert!(w.max_conflict_fee > 0 && w.max_conflict_fee < w.pin_abs, "the conflict never out-pays a 20k-sat pin");
    assert_eq!(refunded_at, Some(DELIVER_BY + 30 * DAY));
    assert!(matches!(c.redemptions[0].phase, RPhase::LostRace { .. }), "{:?}", c.redemptions[0].phase);
}
fn conflict_of(c: &Custody) -> Option<conflict::ConflictTx> {
    match &c.redemptions[0].phase {
        RPhase::Conflicting { conflict, .. } => conflict.tx.clone(),
        _ => None,
    }
}


/// Control for the test above: the same pin at the miners, but the conflict is not seen in the explorers' mempools (so it is not
/// `held`, which is how round 3 treated every conflict): the x1.5 escalation out-pays the 20k-sat pin and our conflict confirms.
#[test]
fn audit4_r3c7_control_an_unseen_conflict_escalates_and_beats_the_same_pin() {
    let (mut w, mut c) = World::new(false);
    w.rate = 10;
    w.pin_abs = 20_000;
    w.hide_conflict = true;
    for i in 0..(40 * 144) {
        w.now = T0 + i * 600;
        w.paused = i > 0 && conflict_of(&c).is_none() && w.max_conflict_fee == 0;
        if w.refundable() && w.stage == Stage::Resolved && !w.undeliverable {
            w.stage = Stage::Refunded;
            let d = w.delivery_txid.expect("a delivery was sent");
            w.spender = Some(Mem { txid: d, delivery: true, confirmed: true });
        }
        w.tick(&mut c);
    }
    println!("R4 R3-C7 control: max conflict fee {} sats; phase {:?}; attests {}", w.max_conflict_fee, c.redemptions[0].phase, w.attests_sent);
    assert!(w.max_conflict_fee > w.pin_abs, "escalated past the pin");
    assert!(!matches!(c.redemptions[0].phase, RPhase::LostRace { .. }));
}
