//! Third review (R3 check): R2's canister N-2 PoC (a redeemer pins custody's delivery so that NO replacement is accepted) and
//! M-1 (c) (a pause across the margin), re-run against the round-3 canister and the gated wrapper's rules
//! (`refund` of a Resolved ticket at `deliverBy + 72 h` only once `markUndeliverable` landed, else at `deliverBy + 30 d`).

mod fakerpc;

use std::future::Future;
use std::pin::pin;
use std::task::{Context, Poll, Waker};

use etch_autobuyer::custody::{self, Contents, Location, Outpoint, Output, Utxo};
use etch_autobuyer::delivery::{Stage, TicketRead};
use etch_autobuyer::looper::{BtcView, EvmView};
use etch_autobuyer::redeem::{self, Attest, Custody, CustodyIo, RPhase, Redemption};
use etch_autobuyer::ticketlog::TicketLogs;
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

const W: [u8; 20] = [0xcc; 20];
const DEST: &str = "bc1p5cyxnuxmeuwuvkwfem96lqzszd02n6xdcjrs20cac6yqjjwudpxqkedrcr";
const HEAD: u64 = 72_856_687;
const FROG: Outpoint = Outpoint { txid: [0xf0; 32], vout: 0 };
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

#[derive(Clone, Debug)]
struct Mem {
    txid: [u8; 32],
    delivery: bool,
    confirmed: bool,
}

/// One fee input, one frog. `pin`: R2's mempool — the first delivery is pinned, and NO replacement of it is ever accepted (the
/// strongest pin: a fee spike above anything the conflict can pay). `market_ok`: a block takes our conflict when it is in.
struct World {
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
        (World { now: T0, fee: Utxo { outpoint: Outpoint { txid: [0xfe; 32], vout: 1 }, value: 150_000, confirmed: true }, spender: None,
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
        if let Some(m) = self.spender.as_mut().filter(|m| !m.confirmed && !m.delivery) {
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
    async fn fee_rate(&mut self) -> Result<u64, String> { Ok(100) }
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

/// R2 N-2 with R2's own mempool (NO replacement of the pinned delivery is ever accepted): against the gated wrapper the canister
/// never attests (its conflict never confirms), so the refund waits `LATE_DELIVERY_WINDOW`; a redeemer who keeps the pin for 30
/// days then refunds and confirms the delivery: LostRace. So N-2 is closed for any pin shorter than 30 days (and, per
/// fixes-R3-canister §2.1, TRUC makes such a pin need a ~80+ sat/vB market all that time), not closed absolutely.
#[test]
fn audit3_r2_n2_poc_an_unbreakable_pin_now_needs_thirty_days_and_the_canister_never_attests_falsely() {
    let (mut w, mut c) = World::new(true);
    let mut refunded_at = None;
    for i in 0..(40 * 144) {
        w.now = T0 + i * 600;
        // the redeemer refunds the moment the gated wrapper lets them, then confirms their pinned delivery
        if refunded_at.is_none() && w.refundable() {
            w.stage = Stage::Refunded;
            refunded_at = Some(w.now);
            if let Some(m) = w.spender.as_mut().filter(|m| m.delivery) {
                m.confirmed = true;
            }
        }
        w.tick(&mut c);
        // at the old wrapper's refund time (deliverBy + 72 h) nothing has been attested
        if w.now == DELIVER_BY + 72 * 3600 {
            assert_eq!(w.attests_sent, 0, "no attestation while the delivery can still confirm");
            assert!(!w.refundable(), "the gated wrapper refuses the refund at deliverBy + 72 h");
        }
    }
    assert!(w.rejected > 0, "every replacement was refused: {}", w.rejected);
    assert_eq!(w.attests_sent, 0, "never attested: the conflict never confirmed");
    assert_eq!(refunded_at, Some(DELIVER_BY + 30 * DAY), "refunded only at deliverBy + 30 days");
    assert!(matches!(c.redemptions[0].phase, RPhase::LostRace { .. }), "{:?}", c.redemptions[0].phase);
    println!("R3 check N-2: unbreakable pin: {} replacements refused, 0 attestations, refund at deliverBy + 30 d, then LOST RACE", w.rejected);
}

/// The honest case, and M-1 (c): the buyer is paused from day 5 (before the margin) to day 20. Nothing is re-sent or bumped while
/// paused; at the margin the rule refuses and the conflict-out is signed and confirms while paused; the attestation waits for the
/// resume (not sent while paused) and then goes out, so the refund is held past deliverBy + 72 h by the pause only.
#[test]
fn audit3_m1c_a_pause_across_the_margin_conflicts_out_and_attests_only_after_the_resume() {
    let (mut w, mut c) = World::new(true); // the delivery sits in the mempool, never replaced (so it never confirms by itself)
    w.pin = false;
    for i in 0..(25 * 144) {
        w.now = T0 + i * 600;
        w.paused = w.now >= T0 + 5 * DAY && w.now < T0 + 20 * DAY;
        let before = (w.signs, w.attests_sent);
        w.tick(&mut c);
        if w.paused {
            assert_eq!(w.attests_sent, before.1, "no EVM attestation while paused");
        }
        if before.1 == 0 && w.attests_sent > 0 {
            assert!(w.now >= T0 + 20 * DAY, "the first attestation is after the resume");
        }
        if w.now == T0 + 19 * DAY {
            assert!(matches!(c.redemptions[0].phase, RPhase::Refused { .. }), "conflicted out while paused: {:?}", c.redemptions[0].phase);
            assert!(matches!(c.redemptions[0].attest, Some(Attest::Due { .. })), "{:?}", c.redemptions[0].attest);
        }
    }
    assert!(w.attests_sent >= 1, "attested after the resume (the same signed bytes may be re-sent once before the receipt)");
    assert!(w.undeliverable && w.stage == Stage::Resolved);
    let m = w.spender.clone().unwrap();
    assert!(m.confirmed && !m.delivery, "the conflict confirmed, the frog never left");
}
