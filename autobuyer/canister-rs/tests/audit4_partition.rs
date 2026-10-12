//! R4-check R4-C1, now a regression (docs/audit/2026-09-26/review4/fixes-golive-lows.md §2). The reviewer's world (the harness of
//! tests/review3.rs, extended with a pinned delivery held at the MINERS while the explorers, what the canister reads, hold our
//! conflict): a mempool partition used to keep our conflict at today's rate for 30 days, so a 20k-sat pin won the refund race. The
//! "held" rule now lasts HELD_TRUST_SECS (24 h) from the first sighting; after that the conflict escalates and out-pays the pin.
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
         Custody { cursor: 0, redemptions: vec![], logs: Some(logs), colls: None })
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

/// Phase B2: this world's home is the EVM chain (no Solana).
impl etch_autobuyer::solhome::SolIo for World {}

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

/// R4-C1 under the reviewer's partition: the explorers hold our conflict (so it is `held`), the miners a pinned delivery package of
/// 20,000 sats. Our conflict follows today's rate for the first HELD_TRUST_SECS after the explorers first showed it, then escalates
/// ×1.5 every CONFLICT_REPRICE_SECS, out-pays the pin within hours, and confirms: no LostRace (was: 2,231 sats for 40 days, the
/// refund at deliverBy + 30 d, then LostRace).
#[test]
fn audit4_r4c1_a_partition_no_longer_keeps_the_conflict_at_todays_rate_it_escalates_after_a_day_and_beats_the_pin() {
    let (mut w, mut c) = World::new(false);
    w.rate = 10;
    w.pin_abs = 20_000;
    let mut refunded_at = None;
    let (mut first_fee, mut first_at, mut escalated_at, mut confirmed_at) = (0, None, None, None);
    for i in 0..(40 * 144) {
        w.now = T0 + i * 600;
        // paused between the first send and the margin (as review3's R3-C7 test), so the delivery is not bumped first
        w.paused = i > 0 && conflict_of(&c).is_none() && w.max_conflict_fee == 0;
        if refunded_at.is_none() && w.refundable() {
            w.stage = Stage::Refunded;
            refunded_at = Some(w.now);
            // the miners' pinned delivery confirms, unless our conflict already spent its fee input
            if !w.spender.as_ref().is_some_and(|m| m.confirmed && !m.delivery) {
                let d = w.delivery_txid.expect("a delivery was sent");
                w.spender = Some(Mem { txid: d, delivery: true, confirmed: true });
            }
        }
        w.tick(&mut c);
        if first_at.is_none() && w.max_conflict_fee > 0 {
            first_at = Some(w.now);
            first_fee = w.max_conflict_fee;
        }
        if escalated_at.is_none() && first_fee > 0 && w.max_conflict_fee > first_fee * 14 / 10 {
            escalated_at = Some(w.now);
        }
        if confirmed_at.is_none() && w.spender.as_ref().is_some_and(|m| m.confirmed && !m.delivery) {
            confirmed_at = Some(w.now);
        }
    }
    let (first_at, escalated_at) = (first_at.expect("conflicted out"), escalated_at.expect("escalated"));
    println!("R4-C1 regression: conflict at {first_fee} sats from {} s; escalated at +{} s; max {} sats (pin {}); confirmed at +{:?} s; \
        refunded {:?}; phase {:?}", first_at - T0, escalated_at - first_at, w.max_conflict_fee, w.pin_abs,
        confirmed_at.map(|t| t - first_at), refunded_at, c.redemptions[0].phase);
    assert!(escalated_at - first_at >= conflict::HELD_TRUST_SECS, "the first day in a mempool only follows the market (R3-C7 kept)");
    assert!(escalated_at - first_at <= conflict::HELD_TRUST_SECS + conflict::CONFLICT_REPRICE_SECS + 1_200, "then it escalates");
    assert!(w.max_conflict_fee > w.pin_abs, "the conflict out-pays the 20k-sat pin at the miners");
    let confirmed_at = confirmed_at.expect("our conflict confirmed");
    assert!(confirmed_at < DELIVER_BY + 30 * DAY, "before the refund race ends");
    assert!(!matches!(c.redemptions[0].phase, RPhase::LostRace { .. }), "{:?}", c.redemptions[0].phase);
}

/// R4-C1: inside HELD_TRUST_SECS the R3-C7 rule is unchanged: a conflict held at today's rate is sent again, not re-priced.
#[test]
fn audit4_r4c1_the_first_day_held_is_still_not_escalated() {
    let (mut w, mut c) = World::new(false);
    w.rate = 10;
    w.pin_abs = 20_000;
    let mut first: Option<(u64, u64)> = None;
    for i in 0..(10 * 144) {
        w.now = T0 + i * 600;
        w.paused = i > 0 && conflict_of(&c).is_none() && w.max_conflict_fee == 0;
        w.tick(&mut c);
        if first.is_none() && w.max_conflict_fee > 0 {
            first = Some((w.now, w.max_conflict_fee));
        }
        if let Some((at, fee)) = first {
            if w.now < at + conflict::HELD_TRUST_SECS {
                assert_eq!(w.max_conflict_fee, fee, "held for under a day: not re-priced (at +{} s)", w.now - at);
            }
        }
    }
    assert!(first.is_some());
    assert!(w.lines.iter().any(|l| l.contains("not re-priced")));
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
