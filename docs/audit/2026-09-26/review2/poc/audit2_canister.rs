//! Second review (R2) of the area-D fixes: PoCs for what the fixes introduced or left open. Each test PASSES when the
//! weakness it names is present (it asserts the bad outcome), so a fix turns it red.

mod fakerpc;

use std::cell::RefCell;
use std::future::Future;
use std::pin::pin;
use std::rc::Rc;
use std::task::{Context, Poll, Waker};

use etch_autobuyer::custody::{self, Contents, Location, Outpoint, Output, Utxo};
use etch_autobuyer::delivery::{Stage, TicketRead};
use etch_autobuyer::evmread::{LogView, Rpc};
use etch_autobuyer::looper::{BtcView, EvmView};
use etch_autobuyer::pin::Pin;
use etch_autobuyer::redeem::{self, Custody, CustodyIo, RPhase, Redemption};
use etch_autobuyer::ticketlog::{self, TicketLogs, MIN_SPAN_BLOCKS};
use etch_autobuyer::{conflict, custodycap, frogbuy, outbound, outflow};
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
const HEAD: u64 = 72_856_687; // measured 2026-09-26

// ---- 1. a burst of wrapper events larger than one page's response cap stalls the scanner for ever ----------------------------

/// A provider as the canister reaches it: the IC's `max_response_bytes` for `eth_getLogs` is 48,000 (`canister.rs` `IcRpc::logs`),
/// headers included. A live Robinhood Chain log is ≈ 556 bytes with 2 topics and 32 bytes of data (sampled 2026-09-26); a
/// `Redeemed` log has 3 topics and 160 bytes of data, ≈ 880–900 bytes; ordofi's headers are ≈ 984 bytes. So a page holding more
/// than ≈ 50 `Redeemed` logs is refused by the replica whatever the span.
struct Capped {
    inner: FakeRpc,
    max_bytes: usize,
}

fn raw_log_bytes(l: &LogView) -> usize {
    // the fields a Nitro node returns for each log (address, blockHash, blockNumber, blockTimestamp, data, logIndex, removed,
    // topics, transactionHash, transactionIndex), minified
    let v = serde_json::json!({ "address": l.address, "topics": l.topics, "data": l.data, "blockNumber": format!("0x{:x}", l.block),
        "blockHash": format!("0x{}", "ab".repeat(32)), "blockTimestamp": "0x68d6a2c1", "transactionHash": format!("0x{}", "cd".repeat(32)),
        "transactionIndex": "0x1", "logIndex": "0x1f", "removed": false });
    serde_json::to_string(&v).unwrap().len() + 1
}

impl Rpc for Capped {
    fn name(&self) -> String { self.inner.name() }
    async fn head(&mut self, b: u64) -> Result<u64, String> { self.inner.head(b).await }
    async fn finalized(&mut self, b: u64) -> Result<u64, String> { self.inner.finalized(b).await }
    async fn block(&mut self, n: u64) -> Result<serde_json::Value, String> { self.inner.block(n).await }
    async fn call(&mut self, to: &[u8; 20], d: &[u8], at: &Pin) -> Result<Vec<u8>, String> { self.inner.call(to, d, at).await }
    async fn logs(&mut self, a: &[u8; 20], t: &serde_json::Value, from: u64, to: u64) -> Result<Vec<LogView>, String> {
        let logs = self.inner.logs(a, t, from, to).await?;
        let body: usize = 1_000 + 40 + logs.iter().map(raw_log_bytes).sum::<usize>();
        if body > self.max_bytes {
            return Err(format!("Http body exceeds size limit of {} bytes", self.max_bytes));
        }
        Ok(logs)
    }
}

#[test]
fn audit2_a_burst_of_redemptions_in_one_block_stalls_the_ticket_log_scanner_for_ever() {
    let c = Rc::new(RefCell::new(Chain { head: HEAD, logs: vec![] }));
    let mut ps: Vec<Capped> = fakerpc::pair(&c).into_iter().map(|inner| Capped { inner, max_bytes: 48_000 }).collect();
    let one = raw_log_bytes(&fakerpc::redeemed(&W, 0, DEST, 1));
    let fit = (48_000 - 1_040) / one;
    assert!((45..=60).contains(&fit), "a page fits {fit} Redeemed logs of {one} bytes");
    // one holder redeems 60 units in one transaction (a contract looping `redeem`), well below today's finality
    let burst = HEAD - 40_000;
    for t in 0..60 {
        c.borrow_mut().logs.push(fakerpc::redeemed(&W, t, DEST, burst));
    }
    // an honest redeemer's ticket, just after the burst
    c.borrow_mut().logs.push(fakerpc::redeemed(&W, 60, DEST, burst + 50));
    c.borrow_mut().logs.push(fakerpc::resolved(&W, 60, 7, burst + 60));
    let mut idx = TicketLogs::default();
    // a week of ticks (7 × 144), the chain moving ≈ 5,940 blocks a tick
    for _ in 0..(7 * 144) {
        let _ = block_on(ticketlog::advance(&mut ps, &mut idx, &W, Some(burst - 10_000)));
        c.borrow_mut().head += 5_940;
    }
    assert!(idx.cursor.unwrap() < burst, "the cursor never passes the burst: {:?} < {burst}", idx.cursor);
    assert_eq!(idx.span, MIN_SPAN_BLOCKS, "halved to the floor, and the floor still fails");
    assert!(idx.failures >= 1_000, "{} failures in a row", idx.failures);
    assert!(!idx.has_both(60), "the honest ticket's events are never read: its delivery waits until it refunds");
    println!("AUDIT2: cursor stuck at {:?} (burst at {burst}), span {}, {} failures: {:?}", idx.cursor, idx.span, idx.failures,
        idx.last_error);
}

// ---- 2. one fee-pool entry the explorers cannot read blocks every delivery -------------------------------------------------

fn ours() -> Vec<u8> {
    [vec![0x51, 0x20], vec![0x11; 32]].concat()
}
fn op(tag: u8, vout: u32) -> Outpoint {
    Outpoint { txid: [tag; 32], vout }
}
fn insc(n: u8) -> String {
    format!("{}i0", format!("{n:02x}").repeat(32))
}

/// The smallest custody world a delivery needs: ticket 0 resolved to frog 7 (on `FROG`), its events already final, and the
/// candidates `our_utxos` returns (the fee pool, as `CustodianIo::our_utxos` returns it when it is not empty).
struct PoolWorld {
    now: u64,
    pool: Vec<Utxo>,
    /// outputs the explorers cannot read (a typo'd or unknown outpoint registered with `add_custody_fee_input`)
    unreadable: Vec<Outpoint>,
    signs: usize,
    sent: usize,
    caps: custodycap::CapPolicy,
    book: custodycap::Book,
}

const FROG: Outpoint = Outpoint { txid: [0xf0; 32], vout: 0 };
const T0: u64 = 1_790_000_000;

impl CustodyIo for PoolWorld {
    fn now(&self) -> u64 { self.now }
    fn chain_id(&self) -> u64 { 4663 }
    async fn ticket_count(&mut self) -> Result<(u64, u64), String> { Ok((1, HEAD - 1_500)) }
    async fn read_ticket(&mut self, id: u64, _f: bool) -> Result<TicketRead, String> {
        if id != 0 {
            return Err("no such ticket".into());
        }
        Ok(TicketRead { stage: Stage::Resolved, acquisition_id: 7, destination: DEST.into(), deliver_by: T0 + 7 * 86_400,
            deliver_by_block: 1_000_000, chain_time: self.now, parent_block: 1_000, read_at: self.now })
    }
    async fn inscription_of(&mut self, _a: u128) -> Result<String, String> { Ok(insc(7)) }
    async fn locate(&mut self, _i: &str) -> Result<Option<Location>, String> { Ok(Some(Location { output: FROG, offset: 0, value: 546 })) }
    async fn contents(&mut self, o: &Outpoint) -> Result<Option<Contents>, String> {
        Ok(Some(if *o == FROG { Contents { inscriptions: vec![insc(7)], runes: vec![], value: 546 } }
            else { Contents { inscriptions: vec![], runes: vec![], value: self.pool.iter().find(|u| u.outpoint == *o).map_or(0, |u| u.value) } }))
    }
    async fn output(&mut self, o: &Outpoint) -> Result<(Output, bool), String> {
        if self.unreadable.contains(o) {
            return Err("https://mempool.space/api: HTTP 404 Transaction not found".into());
        }
        if *o == FROG {
            return Ok((Output { script: ours(), value: 546, confirmed: true }, false));
        }
        let u = self.pool.iter().find(|u| u.outpoint == *o).ok_or("unknown")?;
        Ok((Output { script: ours(), value: u.value, confirmed: true }, false))
    }
    async fn our_utxos(&mut self) -> Result<Vec<Utxo>, String> { Ok(self.pool.clone()) }
    async fn fee_rate(&mut self) -> Result<u64, String> { Ok(3) }
    async fn our_script(&mut self) -> Result<Vec<u8>, String> { Ok(ours()) }
    fn btc_hrp(&self) -> Result<&'static str, String> { Ok("bc") }
    async fn sign_taproot(&mut self, _h: [u8; 32]) -> Result<Vec<u8>, String> {
        self.signs += 1;
        Ok(vec![0x5a; 64])
    }
    async fn btc_send(&mut self, _raw: &[u8]) -> Result<(), String> {
        self.sent += 1;
        Ok(())
    }
    async fn btc_seen(&mut self, _t: [u8; 32]) -> BtcView { BtcView::NotYet }
    async fn may_record(&mut self) -> Result<bool, String> { Ok(true) }
    fn wrapper(&self) -> [u8; 20] { W }
    async fn evm_nonce(&mut self) -> Result<u64, String> { Ok(0) }
    async fn base_fee(&mut self) -> Result<u128, String> { Ok(1) }
    async fn estimate_gas(&mut self, _t: [u8; 20], _v: u128, _d: &[u8]) -> Result<u64, String> { Ok(1) }
    async fn sign_evm(&mut self, _h: [u8; 32]) -> Result<(u8, [u8; 32], [u8; 32]), String> { Ok((0, [1; 32], [2; 32])) }
    async fn send_evm(&mut self, _r: &[u8]) -> Result<(), String> { Ok(()) }
    async fn evm_seen(&mut self, _h: [u8; 32]) -> EvmView { EvmView::NotYet }
    fn save(&mut self, _r: &Redemption) {}
    fn save_cursor(&mut self, _c: u64) {}
    fn with_caps<R>(&mut self, f: impl FnOnce(&custodycap::CapPolicy, &mut custodycap::Book) -> R) -> R { f(&self.caps, &mut self.book) }
    fn outflow_check(&mut self, _k: &str, _c: &str, _f: outflow::Flow, _a: u128) -> Result<(), String> { Ok(()) }
    fn outflow_record(&mut self, _k: &str, _c: &str, _f: outflow::Flow, _a: u128) {}
    async fn scan_ticket_logs(&mut self, _l: &mut TicketLogs) -> Result<String, String> { Ok("nothing new".into()) }
    fn outbound_policy(&self, s: &[u8]) -> outbound::BtcPolicy {
        outbound::BtcPolicy { custody_script: s.to_vec(), max_fee_sats: outbound::MAX_FEE_SATS, require_finalized: true, require_event: true }
    }
    fn paused(&self) -> bool { false }
    async fn outspend(&mut self, _o: &Outpoint) -> Result<Option<([u8; 32], bool)>, String> { Ok(None) }
}

fn pool_world(pool: Vec<Utxo>, unreadable: Vec<Outpoint>) -> (PoolWorld, Custody) {
    let mut logs = TicketLogs::default();
    logs.ingest(&[fakerpc::redeemed(&W, 0, DEST, 10), fakerpc::resolved(&W, 0, 7, 11)]).unwrap();
    let caps = custodycap::CapPolicy { collections: vec![custodycap::CollectionCap { collection: custodycap::btc_key(&W), max_items: 1_000,
        max_value_wei: u128::MAX / 4, max_item_wei: u128::MAX / 4 }], global_max_items: 1_000, global_max_value_wei: u128::MAX / 4 };
    (PoolWorld { now: T0, pool, unreadable, signs: 0, sent: 0, caps, book: Default::default() },
        Custody { cursor: 0, redemptions: vec![], logs: Some(logs) })
}

#[test]
fn audit2_one_unreadable_fee_pool_entry_blocks_every_delivery_and_is_never_dropped() {
    let good = Utxo { outpoint: op(0xfe, 1), value: 20_000, confirmed: true };
    // control: a pool of one good entry delivers
    let (mut w, mut c) = pool_world(vec![good.clone()], vec![]);
    let l0 = block_on(redeem::tick(&mut w, &mut c, false));
    assert!(matches!(c.redemptions[0].phase, RPhase::Signed { .. }), "{:?} {l0:?}", c.redemptions[0].phase);
    // the operator registers one more top-up, mistyped (vout 9 of a real txid, or a txid that does not exist): 5,000 sats,
    // the smallest a pool entry may hold, so it sorts first among the candidates
    let bogus = Utxo { outpoint: op(0xab, 9), value: 5_000, confirmed: true };
    let (mut w, mut c) = pool_world(vec![good, bogus.clone()], vec![bogus.outpoint]);
    let mut lines = vec![];
    for i in 0..144 {
        w.now = T0 + i * 600;
        lines = block_on(redeem::tick(&mut w, &mut c, false));
    }
    assert_eq!(c.redemptions[0].phase, RPhase::Owed, "a day of ticks: still owed");
    assert_eq!((w.signs, w.sent), (0, 0), "nothing signed, although a good fee input is in the pool");
    assert!(lines.iter().any(|l| l.contains("waits") && l.contains("404")), "{lines:?}");
    // `choose_fee` drops a pool entry only when the explorers read it as SPENT; an unreadable one stays, and the pool is not
    // empty, so the listing fallback never runs either. The only way out is 32 more `add_custody_fee_input`s (FIFO eviction).
    println!("AUDIT2: {lines:?}");
}

// ---- 3. a stale buy that paid the judge's fee bound cannot be abandoned ------------------------------------------------------

#[test]
fn audit2_a_stale_buy_at_the_judges_fee_bound_cannot_be_conflicted_out_under_the_ceiling() {
    // the judge lets a buy pay up to fee_bound(spend), which reaches the 25,000-sat custody ceiling from ~833k sats of spend
    let bound = frogbuy::fee_bound(1_000_000);
    assert_eq!(bound, custody::MAX_DELIVERY_FEE_SATS);
    // L-5's conflict must pay the buy's fee plus its own size (BIP125 rules 3 and 4), one input or two
    let one = conflict::fee_for(1, bound, 1);
    let two = conflict::fee_for(2, bound, 1);
    assert!(one > custody::MAX_DELIVERY_FEE_SATS && two > custody::MAX_DELIVERY_FEE_SATS, "{one} / {two}");
    // and `conflict::build` refuses any fee over the ceiling, so `abandoning` ends `Stuck` -> FPhase::Stranded, the stale buy's
    // bytes stay valid, and its `frog:{id}` reservation is never settled (sweep_stale releases only Wanted/Failed)
    let float = [vec![0x51, 0x20], vec![0x22; 32]].concat();
    let e = conflict::build(&[(op(0x0f, 0), 1_000_000)], &float, one).unwrap_err();
    assert!(e.contains("ceiling"), "{e}");
    // the largest buy fee that can still be abandoned
    let max_ok = custody::MAX_DELIVERY_FEE_SATS - custody::vsize(1, &[34]) * conflict::INCREMENTAL_RELAY_SATS_PER_VB;
    println!("AUDIT2: buys paying {}..={} sats of fee cannot be abandoned", max_ok + 1, bound);
}

// ---- 4. positive check: a bump at its cap can still be conflicted out ---------------------------------------------------------

#[test]
fn audit2_a_delivery_bumped_to_its_cap_can_still_be_replaced_by_the_conflict_out() {
    for inputs in [1usize, 2] {
        let f = conflict::fee_for(inputs, redeem::MAX_BUMP_FEE_SATS, custody::MAX_FEE_RATE);
        assert!(f <= custody::MAX_DELIVERY_FEE_SATS, "{inputs} input(s): {f}");
        assert!(f >= redeem::MAX_BUMP_FEE_SATS + custody::vsize(inputs, &[34]), "BIP125 rule 4");
    }
    // the first send never pays more than the bump cap either (the no-change case is bounded by the 100 sat/vB clamp)
    assert!(custody::vsize(2, &[34, 34]) * custody::MAX_FEE_RATE + 330 < redeem::MAX_BUMP_FEE_SATS);
}

// ---- 5. the limiter: returns never trip, however many --------------------------------------------------------------------

#[test]
fn audit2_returns_are_rate_limited_but_never_trip_whatever_their_number() {
    let l = outflow::Limits::stage0();
    let mut o = outflow::Outflow::default();
    let c = "btc:0xcc";
    let mut allowed = 0;
    for i in 0..1_000 {
        match o.check(&l, &format!("return:{i}"), c, outflow::Flow::Return, 1, T0 + i) {
            outflow::Verdict::Allow => {
                o.record(&format!("return:{i}"), c, outflow::Flow::Return, 1, T0 + i);
                allowed += 1;
            }
            outflow::Verdict::Frozen(cause) => panic!("a return tripped: {cause:?}"),
            outflow::Verdict::Wait { .. } => {}
        }
    }
    assert_eq!(allowed, 3, "the item limit, counted apart from deliveries");
    assert!(o.frozen().is_none(), "1,000 distinct return asks in a day: no freeze");
    // deliveries still have their own full budget the same day: 3 + 3 items can leave custody
    let mut d = 0;
    for i in 0..3 {
        if o.check(&l, &format!("ticket:{i}"), c, outflow::Flow::Item, 1, T0 + 2_000) == outflow::Verdict::Allow {
            o.record(&format!("ticket:{i}"), c, outflow::Flow::Item, 1, T0 + 2_000);
            d += 1;
        }
    }
    assert_eq!(d, 3);
}

// ---- 6. M-1 residual: the redeemer pins the delivery through output 0 (theirs), then refunds and confirms it -----------------

/// A mempool in which the first delivery is PINNED: the redeemer, who owns output 0, has hung a large low-feerate child on it,
/// so BIP125 rule 3 asks any replacement for more absolute fee than the delivery and the child together (≈ 100k sats), which
/// neither a bump (≤ 24,500) nor the conflict-out (≤ 25,000) can pay. Every replacement is refused; the pinned package waits,
/// under the market, until the redeemer chooses to confirm it (CPFP-ing their own child).
struct PinWorld {
    now: u64,
    fee: Utxo,
    original: Option<([u8; 32], Vec<u8>)>,
    confirmed: Option<[u8; 32]>,
    refunded: bool,
    rejected: usize,
    signs: usize,
    caps: custodycap::CapPolicy,
    book: custodycap::Book,
}

fn display_txid(raw: &[u8]) -> [u8; 32] {
    let mut t = etch_autobuyer::btcsend::parse_signed(raw).unwrap().txid;
    t.reverse();
    t
}

impl CustodyIo for PinWorld {
    fn now(&self) -> u64 { self.now }
    fn chain_id(&self) -> u64 { 4663 }
    async fn ticket_count(&mut self) -> Result<(u64, u64), String> { Ok((1, HEAD - 1_500)) }
    async fn read_ticket(&mut self, _id: u64, _f: bool) -> Result<TicketRead, String> {
        let deliver_by = T0 + 7 * 86_400;
        Ok(TicketRead { stage: if self.refunded { Stage::Refunded } else { Stage::Resolved }, acquisition_id: 7, destination: DEST.into(),
            deliver_by, deliver_by_block: deliver_by / 12, chain_time: self.now, parent_block: self.now / 12, read_at: self.now })
    }
    async fn inscription_of(&mut self, _a: u128) -> Result<String, String> { Ok(insc(7)) }
    async fn locate(&mut self, _i: &str) -> Result<Option<Location>, String> { Ok(Some(Location { output: FROG, offset: 0, value: 546 })) }
    async fn contents(&mut self, o: &Outpoint) -> Result<Option<Contents>, String> {
        Ok(Some(if *o == FROG { Contents { inscriptions: vec![insc(7)], runes: vec![], value: 546 } }
            else { Contents { inscriptions: vec![], runes: vec![], value: self.fee.value } }))
    }
    async fn output(&mut self, o: &Outpoint) -> Result<(Output, bool), String> {
        let spent = self.original.is_some();
        if *o == FROG {
            return Ok((Output { script: ours(), value: 546, confirmed: true }, spent));
        }
        if *o == self.fee.outpoint {
            return Ok((Output { script: ours(), value: self.fee.value, confirmed: true }, spent));
        }
        Err("unknown output".into())
    }
    async fn our_utxos(&mut self) -> Result<Vec<Utxo>, String> {
        Ok(if self.original.is_none() { vec![self.fee.clone()] } else { vec![] })
    }
    async fn fee_rate(&mut self) -> Result<u64, String> { Ok(3) }
    async fn our_script(&mut self) -> Result<Vec<u8>, String> { Ok(ours()) }
    fn btc_hrp(&self) -> Result<&'static str, String> { Ok("bc") }
    async fn sign_taproot(&mut self, _h: [u8; 32]) -> Result<Vec<u8>, String> {
        self.signs += 1;
        Ok(vec![0x5a; 64])
    }
    async fn btc_send(&mut self, raw: &[u8]) -> Result<(), String> {
        let id = display_txid(raw);
        match &self.original {
            None => self.original = Some((id, raw.to_vec())),
            Some((o, _)) if *o == id => {}
            Some(_) => self.rejected += 1, // pinned: "insufficient fee, rejecting replacement"
        }
        Ok(())
    }
    async fn btc_seen(&mut self, t: [u8; 32]) -> BtcView {
        if self.confirmed == Some(t) {
            BtcView::Confirmed
        } else if self.confirmed.is_none() && self.original.as_ref().is_some_and(|(o, _)| *o == t) {
            BtcView::Mempool
        } else {
            BtcView::NotYet
        }
    }
    async fn may_record(&mut self) -> Result<bool, String> { Ok(true) }
    fn wrapper(&self) -> [u8; 20] { W }
    async fn evm_nonce(&mut self) -> Result<u64, String> { Ok(0) }
    async fn base_fee(&mut self) -> Result<u128, String> { Ok(1) }
    async fn estimate_gas(&mut self, _t: [u8; 20], _v: u128, _d: &[u8]) -> Result<u64, String> { Ok(1) }
    async fn sign_evm(&mut self, _h: [u8; 32]) -> Result<(u8, [u8; 32], [u8; 32]), String> { Ok((0, [1; 32], [2; 32])) }
    async fn send_evm(&mut self, _r: &[u8]) -> Result<(), String> { Ok(()) }
    async fn evm_seen(&mut self, _h: [u8; 32]) -> EvmView { EvmView::NotYet }
    fn save(&mut self, _r: &Redemption) {}
    fn save_cursor(&mut self, _c: u64) {}
    fn with_caps<R>(&mut self, f: impl FnOnce(&custodycap::CapPolicy, &mut custodycap::Book) -> R) -> R { f(&self.caps, &mut self.book) }
    fn outflow_check(&mut self, _k: &str, _c: &str, _f: outflow::Flow, _a: u128) -> Result<(), String> { Ok(()) }
    fn outflow_record(&mut self, _k: &str, _c: &str, _f: outflow::Flow, _a: u128) {}
    async fn scan_ticket_logs(&mut self, _l: &mut TicketLogs) -> Result<String, String> { Ok("nothing new".into()) }
    fn outbound_policy(&self, s: &[u8]) -> outbound::BtcPolicy {
        outbound::BtcPolicy { custody_script: s.to_vec(), max_fee_sats: outbound::MAX_FEE_SATS, require_finalized: true, require_event: true }
    }
    fn paused(&self) -> bool { false }
    async fn outspend(&mut self, o: &Outpoint) -> Result<Option<([u8; 32], bool)>, String> {
        if *o == self.fee.outpoint || *o == FROG {
            return Ok(self.original.as_ref().map(|(t, _)| (*t, self.confirmed == Some(*t))));
        }
        Ok(None)
    }
}

#[test]
fn audit2_a_redeemer_who_pins_the_delivery_defeats_the_bump_and_the_conflict_and_ends_with_frog_and_refund() {
    let (pw, mut c) = pool_world(vec![], vec![]);
    let mut w = PinWorld { now: T0, fee: Utxo { outpoint: op(0xfe, 1), value: 20_000, confirmed: true }, original: None, confirmed: None,
        refunded: false, rejected: 0, signs: 0, caps: pw.caps, book: pw.book };
    let mut phases = vec![];
    // ten days of ticks: the delivery is signed and sent, sits (pinned, under a fee spike), is bumped, then conflicted out at
    // the margin; nothing replaces it
    for i in 0..(10 * 144) {
        w.now = T0 + i * 600;
        block_on(redeem::tick(&mut w, &mut c, false));
        let p = format!("{:?}", c.redemptions[0].phase).split([' ', '{', '(']).next().unwrap().to_string();
        if phases.last() != Some(&p) {
            phases.push(p);
        }
    }
    let (orig, _) = w.original.clone().unwrap();
    assert!(matches!(c.redemptions[0].phase, RPhase::Conflicting { .. }), "{:?}", c.redemptions[0].phase);
    assert!(w.rejected > 0, "every bump and the conflict were refused by the pinned mempool");
    // deliverBy + 72 h: the redeemer refunds (wX back), then confirms the pinned delivery by CPFP-ing their own child
    w.refunded = true;
    w.now = T0 + 10 * 86_400 + 600;
    block_on(redeem::tick(&mut w, &mut c, false));
    w.confirmed = Some(orig);
    for k in 0..3 {
        w.now += 600 * (k + 1);
        block_on(redeem::tick(&mut w, &mut c, false));
    }
    assert_eq!(c.redemptions[0].phase, RPhase::LostRace { txid: orig }, "the redeemer has the frog AND the refunded token");
    println!("AUDIT2: phases {phases:?}; {} replacement(s) refused; {} signatures; ends {:?}", w.rejected, w.signs,
        c.redemptions[0].phase);
}

// ---- 7. L-7's rule moved the single-provider lever from "every send at the clamp" to "every send at twice the lowest" --------

#[test]
fn audit2_one_explorer_reporting_a_low_estimate_sets_every_custody_rate() {
    // the honest explorer sees a 150 sat/vB market; a stale or lying one reports 1
    assert_eq!(custody::combine_rates(&[1, 150]), Ok(2), "custody signs at 2 sat/vB");
    assert_eq!(custody::combine_rates(&[0, 150]), Ok(custody::MIN_FEE_RATE));
    // and the conflict-out it may need later is priced from the same number, and is never re-priced once sent
    let leaked = custody::vsize(2, &[34, 34]) * 2;
    assert!(conflict::fee_for(1, leaked, 2) < custody::vsize(1, &[34]) * 150, "the conflict is under the market too");
}
