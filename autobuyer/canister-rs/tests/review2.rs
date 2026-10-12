//! Second review (R2) of the area-D fixes, round 3 (docs/audit/2026-09-26/review2/fixes-R3-canister.md). R2's PoCs
//! (`docs/audit/2026-09-26/review2/poc/audit2_canister.rs`, kept there as the review's record) asserted each weakness; these are
//! the same scenarios as regression tests that pass only because the attacks now fail. Where a weakness is bounded rather than
//! closed (N-2 in an extreme fee spike), the test says so and measures the bound.

mod fakerpc;

use std::cell::RefCell;
use std::future::Future;
use std::pin::pin;
use std::rc::Rc;
use std::task::{Context, Poll, Waker};

use etch_autobuyer::custody::{self, Contents, Location, Outpoint, Output, Utxo};
use etch_autobuyer::delivery::{Stage, TicketRead};
use etch_autobuyer::evmread::{self, MAX_RESPONSE_BYTES};
use etch_autobuyer::looper::{BtcView, EvmView};
use etch_autobuyer::redeem::{self, Custody, CustodyIo, RPhase, Redemption};
use etch_autobuyer::ticketlog::{self, TicketLogs, MAX_TRIES_PER_TICK};
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

fn get_logs(ps: &[FakeRpc]) -> usize {
    ps[0].asked.borrow().iter().filter(|(m, ..)| m == "eth_getLogs").count()
}

// ---- N-1: a burst of wrapper events larger than one page's response cap no longer stalls the scanner --------------------------

/// R2's scenario: 60 `Redeemed` logs in one block (≈ 54 kB, past the 48,000-byte cap), then an honest ticket 50 blocks later. The
/// fakes now fail an answer over the cap as the IC does (`tests/fakerpc`). The page is retried at a larger cap in the same tick.
#[test]
fn audit2_a_burst_of_redemptions_in_one_block_no_longer_stalls_the_ticket_log_scanner() {
    let c = Rc::new(RefCell::new(Chain { head: HEAD, logs: vec![] }));
    let mut ps = fakerpc::pair(&c);
    let one = fakerpc::raw_log_bytes(&fakerpc::redeemed(&W, 0, DEST, 1));
    assert!((45..=60).contains(&((48_000 - fakerpc::HEADER_BYTES) / one)), "a 48 kB page fits ≈ 50 Redeemed logs of {one} bytes");
    let burst = HEAD - 40_000;
    for t in 0..60 {
        c.borrow_mut().logs.push(fakerpc::redeemed(&W, t, DEST, burst));
    }
    c.borrow_mut().logs.push(fakerpc::redeemed(&W, 60, DEST, burst + 50));
    c.borrow_mut().logs.push(fakerpc::resolved(&W, 60, 7, burst + 60));
    let mut idx = TicketLogs::default();
    let line = block_on(ticketlog::advance(&mut ps, &mut idx, &W, Some(burst - 10_000), Some(&(0..=60).collect::<Vec<_>>()))).unwrap();
    assert!(idx.cursor.unwrap() > burst, "the cursor passes the burst in the first tick: {line}");
    assert!(idx.has_both(60), "the honest ticket's events are read: {line}");
    assert_eq!((0..60).filter(|t| !idx.redeemed_of(*t).is_empty()).count(), 60, "and the burst's own");
    assert_eq!((idx.failures, idx.last_error.clone()), (0, None));
    assert!(line.contains("too big and retried"), "{line}");
    assert!(ps[0].caps.borrow().contains(&400_000), "the retry asked for the larger cap");
    assert!(ps[0].caps.borrow().iter().all(|c| *c <= MAX_RESPONSE_BYTES));
    // the larger cap is paid for: the next page asks one step lower
    assert_eq!(idx.cap, None, "back to the normal cap after the next page");
}

/// One block holding more wrapper events than the IC's largest response (3,000 `Redeemed` logs, ≈ 2.7 MB): at the largest cap
/// the span shrinks to that one block, which is then read by ticket, over several ticks, never more than
/// `MAX_TRIES_PER_TICK` rounds a tick, and the cursor moves past it.
#[test]
fn audit2_a_block_past_the_largest_cap_is_read_by_ticket_and_the_cursor_always_advances() {
    let c = Rc::new(RefCell::new(Chain { head: HEAD, logs: vec![] }));
    let mut ps = fakerpc::pair(&c);
    let dense = HEAD - 30_000;
    for t in 0..3_000 {
        c.borrow_mut().logs.push(fakerpc::redeemed(&W, t, DEST, dense));
    }
    c.borrow_mut().logs.push(fakerpc::redeemed(&W, 3_000, DEST, dense + 5));
    c.borrow_mut().logs.push(fakerpc::resolved(&W, 3_000, 9, dense + 9));
    let body = fakerpc::HEADER_BYTES + 3_000 * fakerpc::raw_log_bytes(&fakerpc::redeemed(&W, 0, DEST, dense));
    assert!(body as u64 > MAX_RESPONSE_BYTES, "{body} bytes: past the IC's limit even alone");
    let wanted: Vec<u64> = (0..=3_000).collect();
    let mut idx = TicketLogs::default();
    let mut ticks = 0;
    // a tick in which the tickets were not walked cannot choose them: it waits, and keeps what was done
    let mut unwalked = false;
    while !idx.has_both(3_000) {
        ticks += 1;
        assert!(ticks <= 12, "stuck: {:?}", idx.dense);
        let before = get_logs(&ps);
        let w = if idx.dense.as_ref().is_some_and(|d| d.after.is_some()) && !unwalked { None } else { Some(wanted.as_slice()) };
        let kept = idx.dense.clone();
        let r = block_on(ticketlog::advance(&mut ps, &mut idx, &W, Some(dense - 5_000), w));
        if w.is_none() {
            unwalked = true;
            assert!(r.unwrap_err().contains("read by ticket"));
            assert_eq!(idx.dense, kept, "the progress is kept");
        } else {
            r.unwrap();
        }
        assert!(get_logs(&ps) - before <= MAX_TRIES_PER_TICK, "a tick's outcalls are bounded");
    }
    assert!(unwalked, "the unwalked tick was exercised");
    assert!(idx.cursor.unwrap() > dense, "past the dense block");
    assert_eq!(idx.dense_blocks, Some(1));
    assert!(idx.dense.is_none() && idx.unreadable.is_none());
    assert_eq!((0..3_000).filter(|t| idx.redeemed_of(*t) == vec![DEST.to_string()]).count(), 3_000, "every ticket's event, once");
    assert!(ps[0].caps.borrow().iter().all(|c| *c <= MAX_RESPONSE_BYTES));
    let at_max = ps[0].caps.borrow().iter().filter(|c| **c == MAX_RESPONSE_BYTES).count();
    assert!(at_max <= 2, "the 2 MB cap is paid only for the one block itself: {at_max}");
    println!("REVIEW2: a 2.7 MB block read by ticket in {ticks} ticks; {} eth_getLogs a provider, {at_max} at 2 MB", get_logs(&ps));
}

/// Even what the wrapper cannot emit (one ticket whose events alone are past 2 MB) does not stop the cursor: after
/// `UNREADABLE_AFTER` tries that ticket is set aside, loud, and the block is finished.
#[test]
fn audit2_even_one_ticket_too_big_to_read_alone_does_not_stop_the_cursor() {
    let c = Rc::new(RefCell::new(Chain { head: HEAD, logs: vec![] }));
    let mut ps = fakerpc::pair(&c);
    let dense = HEAD - 30_000;
    for _ in 0..2_400 {
        c.borrow_mut().logs.push(fakerpc::redeemed(&W, 5, DEST, dense)); // 2,400 copies of ticket 5's event: impossible, and too big
    }
    c.borrow_mut().logs.push(fakerpc::redeemed(&W, 6, DEST, dense));
    let mut idx = TicketLogs::default();
    let mut ticks = 0;
    while idx.cursor.is_none_or(|c| c <= dense) {
        ticks += 1;
        assert!(ticks <= 6, "stuck: {:?}", idx.dense);
        block_on(ticketlog::advance(&mut ps, &mut idx, &W, Some(dense - 5_000), Some(&[5, 6]))).unwrap();
    }
    assert_eq!(idx.unreadable, Some(vec![(dense, 5)]), "set aside, loud");
    assert_eq!(idx.redeemed_of(6), vec![DEST.to_string()], "the other ticket in that block is read");
    assert!(idx.dense_status().unwrap().contains("UNREADABLE"));
}

/// A failure that is not about size (the providers disagree) is what it was: the span halves and the tick stops, loud.
#[test]
fn audit2_a_failure_that_is_not_about_size_still_halves_and_stops() {
    let c = Rc::new(RefCell::new(Chain { head: HEAD, logs: vec![] }));
    let mut ps = fakerpc::pair(&c);
    let start = HEAD - 60_000;
    ps[1].extra_logs.push(fakerpc::redeemed(&W, 0, "bc1pforged", start + 20));
    let mut idx = TicketLogs::default();
    let e = block_on(ticketlog::advance(&mut ps, &mut idx, &W, Some(start), None)).unwrap_err();
    assert!(e.contains("disagree"), "{e}");
    assert_eq!((get_logs(&ps), idx.failures, idx.span), (1, 1, evmread::MAX_LOG_SPAN_BLOCKS / 2));
}

// ---- a custody world for the flow's regressions ---------------------------------------------------------------------------------

fn ours() -> Vec<u8> {
    [vec![0x51, 0x20], vec![0x11; 32]].concat()
}
fn op(tag: u8, vout: u32) -> Outpoint {
    Outpoint { txid: [tag; 32], vout }
}
fn insc(n: u8) -> String {
    format!("{}i0", format!("{n:02x}").repeat(32))
}

const FROG: Outpoint = Outpoint { txid: [0xf0; 32], vout: 0 };
const T0: u64 = 1_790_000_000;

/// Ticket 0 resolved to frog 7 (on `FROG`); the candidates `our_utxos` returns are the fee pool; `listed` is the address's
/// listing (the fallback).
struct PoolWorld {
    now: u64,
    pool: Vec<Utxo>,
    listed: Vec<Utxo>,
    /// outputs the explorers cannot read (a typo'd or unknown outpoint registered with `add_custody_fee_input`)
    unreadable: Vec<Outpoint>,
    failed_reports: Vec<Outpoint>,
    signs: usize,
    sent: usize,
    asked: usize,
    scan_fails: bool,
    caps: custodycap::CapPolicy,
    book: custodycap::Book,
}

/// Phase B2: this world's home is the EVM chain (no Solana).
impl etch_autobuyer::solhome::SolIo for PoolWorld {}
/// Phase B2: this world's home is the EVM chain (no Solana).
impl etch_autobuyer::solhome::SolIo for PinWorld {}

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
        let v = self.pool.iter().chain(self.listed.iter()).find(|u| u.outpoint == *o).map_or(0, |u| u.value);
        Ok(Some(if *o == FROG { Contents { inscriptions: vec![insc(7)], runes: vec![], value: 546 } }
            else { Contents { inscriptions: vec![], runes: vec![], value: v } }))
    }
    async fn output(&mut self, o: &Outpoint) -> Result<(Output, bool), String> {
        if self.unreadable.contains(o) {
            return Err("https://mempool.space/api: HTTP 404 Transaction not found".into());
        }
        if *o == FROG {
            return Ok((Output { script: ours(), value: 546, confirmed: true }, false));
        }
        let u = self.pool.iter().chain(self.listed.iter()).find(|u| u.outpoint == *o).ok_or("unknown")?;
        Ok((Output { script: ours(), value: u.value, confirmed: true }, false))
    }
    async fn our_utxos(&mut self) -> Result<Vec<Utxo>, String> { Ok(self.pool.clone()) }
    async fn listed_utxos(&mut self) -> Result<Vec<Utxo>, String> { Ok(self.listed.clone()) }
    fn fee_pool_failed(&mut self, o: &Outpoint, _why: &str) { self.failed_reports.push(*o); }
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
    fn outflow_check(&mut self, _k: &str, _c: &str, _f: outflow::Flow, _a: u128) -> Result<(), String> {
        self.asked += 1;
        Ok(())
    }
    fn outflow_record(&mut self, _k: &str, _c: &str, _f: outflow::Flow, _a: u128) {}
    async fn scan_ticket_logs(&mut self, l: &mut TicketLogs, _w: Option<&[u64]>) -> Result<String, String> {
        if self.scan_fails {
            l.failures += 1;
            l.last_error = Some("blocks 1..=8000: Http body exceeds size limit".into());
            return Err("the scan fails".into());
        }
        Ok("nothing new".into())
    }
    fn outbound_policy(&self, s: &[u8]) -> outbound::BtcPolicy {
        outbound::BtcPolicy { custody_script: s.to_vec(), max_fee_sats: outbound::MAX_FEE_SATS, require_finalized: true, require_event: true }
    }
    fn paused(&self) -> bool { false }
    async fn outspend(&mut self, _o: &Outpoint) -> Result<Option<([u8; 32], bool)>, String> { Ok(None) }
}

fn caps() -> custodycap::CapPolicy {
    custodycap::CapPolicy { collections: vec![custodycap::CollectionCap { collection: custodycap::btc_key(&W), max_items: 1_000,
        max_value_wei: u128::MAX / 4, max_item_wei: u128::MAX / 4 }], global_max_items: 1_000, global_max_value_wei: u128::MAX / 4 }
}

fn with_events() -> Custody {
    let mut logs = TicketLogs::default();
    logs.ingest(&[fakerpc::redeemed(&W, 0, DEST, 10), fakerpc::resolved(&W, 0, 7, 11)]).unwrap();
    Custody { cursor: 0, redemptions: vec![], logs: Some(logs), colls: None }
}

fn pool_world(pool: Vec<Utxo>, unreadable: Vec<Outpoint>) -> PoolWorld {
    PoolWorld { now: T0, pool, listed: vec![], unreadable, failed_reports: vec![], signs: 0, sent: 0, asked: 0, scan_fails: false,
        caps: caps(), book: Default::default() }
}

// ---- N-12: a ticket behind a failing scan never asks the limiter ----------------------------------------------------------------

#[test]
fn audit2_a_ticket_waiting_behind_a_failing_scan_never_counts_toward_the_tripwire() {
    let good = Utxo { outpoint: op(0xfe, 1), value: 20_000, confirmed: true };
    let mut w = pool_world(vec![good], vec![]);
    w.scan_fails = true;
    let mut c = Custody { logs: Some(TicketLogs::default()), ..Default::default() };
    let mut lines = vec![];
    for i in 0..144 {
        w.now = T0 + i * 600;
        lines = block_on(redeem::tick(&mut w, &mut c, false));
    }
    assert_eq!(c.redemptions[0].phase, RPhase::Owed);
    assert_eq!((w.asked, w.signs), (0, 0), "a day of ticks behind the scanner: the limiter was never asked: {lines:?}");
    assert!(lines.iter().any(|l| l.contains("not final yet")), "{lines:?}");
    // once the events are in, it asks once and signs
    let mut c = Custody { redemptions: c.redemptions, ..with_events() };
    w.scan_fails = false;
    block_on(redeem::tick(&mut w, &mut c, false));
    assert_eq!(w.asked, 1);
    assert!(matches!(c.redemptions[0].phase, RPhase::Signed { .. }));
}

// ---- N-4: one fee-pool entry the explorers cannot read no longer blocks deliveries ---------------------------------------------

#[test]
fn audit2_one_unreadable_fee_pool_entry_is_skipped_counted_and_the_delivery_signs() {
    let good = Utxo { outpoint: op(0xfe, 1), value: 20_000, confirmed: true };
    // R2's scenario: the good entry, and a mistyped 5,000-sat top-up that sorts first
    let bogus = Utxo { outpoint: op(0xab, 9), value: 5_000, confirmed: true };
    let mut w = pool_world(vec![good.clone(), bogus.clone()], vec![bogus.outpoint]);
    let mut c = with_events();
    let lines = block_on(redeem::tick(&mut w, &mut c, false));
    let RPhase::Signed { fee_input, .. } = c.redemptions[0].phase else { panic!("{:?} {lines:?}", c.redemptions[0].phase) };
    assert_eq!(fee_input, good.outpoint, "the unreadable entry is skipped, the good one pays");
    assert_eq!((w.signs, w.sent), (2, 1));
    assert_eq!(w.failed_reports, vec![bogus.outpoint], "counted against the entry: another read worked, so the explorers were up");
    // the pool itself quarantines it at the third such failure, and the operator can remove or restore it
    let mut pool = custody::FeePool::default();
    pool.add(bogus.outpoint, 5_000, "operator").unwrap();
    pool.add(good.outpoint, 20_000, "operator").unwrap();
    // (audit R3, R3-C4: failures count at most once an hour)
    assert!(pool.failed(&bogus.outpoint, "404", T0).is_none() && pool.failed(&bogus.outpoint, "404", T0 + 3_600).is_none());
    let line = pool.failed(&bogus.outpoint, "404", T0 + 7_200).expect("quarantined at the third");
    assert!(line.contains("quarantined"), "{line}");
    assert_eq!(pool.utxos(), vec![good.clone()], "no longer a candidate");
    assert_eq!(pool.quarantined.as_ref().map(|q| q.len()), Some(1));
    pool.add(bogus.outpoint, 5_000, "operator").unwrap();
    assert_eq!((pool.utxos().len(), pool.quarantined.as_ref().map(|q| q.len())), (2, Some(0)), "adding it again restores it");
    assert!(pool.remove(&bogus.outpoint) && !pool.remove(&bogus.outpoint));
    // an outage (every read fails) counts nothing against anyone
    let mut w = pool_world(vec![good.clone(), bogus.clone()], vec![good.outpoint, bogus.outpoint]);
    let mut c = with_events();
    block_on(redeem::tick(&mut w, &mut c, false));
    assert!(w.failed_reports.is_empty() && c.redemptions[0].phase == RPhase::Owed);
}

#[test]
fn audit2_a_pool_with_no_usable_entry_falls_back_to_the_listing() {
    let bogus = Utxo { outpoint: op(0xab, 9), value: 5_000, confirmed: true };
    let listed = Utxo { outpoint: op(0xfd, 0), value: 30_000, confirmed: true };
    let mut w = pool_world(vec![bogus.clone()], vec![bogus.outpoint]);
    w.listed = vec![listed.clone()];
    let mut c = with_events();
    let lines = block_on(redeem::tick(&mut w, &mut c, false));
    let RPhase::Signed { fee_input, .. } = c.redemptions[0].phase else { panic!("{:?} {lines:?}", c.redemptions[0].phase) };
    assert_eq!(fee_input, listed.outpoint, "the listing, when the pool had nothing usable");
}

// ---- N-5: a stale buy at the judge's fee bound can be abandoned -----------------------------------------------------------------

#[test]
fn audit2_a_stale_buy_at_the_judges_fee_bound_can_be_conflicted_out() {
    let float = [vec![0x51, 0x20], vec![0x22; 32]].concat();
    // new buys leave room under the delivery ceiling for their abandon
    let bound = frogbuy::fee_bound(1_000_000);
    assert_eq!(bound, custody::MAX_DELIVERY_FEE_SATS - 500);
    for inputs in [1usize, 2] {
        assert!(conflict::fee_for(inputs, bound, 1) <= custody::MAX_DELIVERY_FEE_SATS, "{inputs} input(s)");
    }
    // and a buy an earlier build signed at the old bound (25,000) can still be conflicted out: the conflict ceiling is above it
    let old = custody::MAX_DELIVERY_FEE_SATS;
    let f = conflict::fee_for(2, old, 1);
    let p = conflict::build(&[(op(0x0f, 0), 1_000_000), (op(0x0e, 0), 50_000)], &float, f).expect("buildable past the delivery ceiling");
    let raw = etch_autobuyer::psbt::finalize(&p, &[None, None], &[(0, vec![0x5a; 64]), (1, vec![0x5a; 64])]).unwrap();
    let pre: Vec<outbound::Prevout> = [(op(0x0f, 0), 1_000_000), (op(0x0e, 0), 50_000)].iter().map(|(o, v)| outbound::Prevout {
        txid: o.internal_txid(), vout: o.vout, value: *v, script: float.clone() }).collect();
    let pol = outbound::BtcPolicy { custody_script: float.clone(), max_fee_sats: outbound::MAX_FEE_SATS, require_finalized: true,
        require_event: true };
    assert!(outbound::verify_btc(&raw, &pre, &outbound::Purpose::ConflictOut { spend: pre[0].clone() }, &pol).is_ok(),
        "H4 allows a conflict up to four times the delivery ceiling");
    // deliveries keep theirs
    assert_eq!(conflict::MAX_CONFLICT_FEE_SATS, 4 * outbound::MAX_FEE_SATS);
}

// ---- positive checks kept from R2 ------------------------------------------------------------------------------------------------

#[test]
fn audit2_a_delivery_bumped_to_its_cap_can_still_be_replaced_by_the_conflict_out() {
    for inputs in [1usize, 2] {
        let f = conflict::fee_for(inputs, redeem::MAX_BUMP_FEE_SATS, custody::MAX_FEE_RATE);
        assert!(f <= custody::MAX_DELIVERY_FEE_SATS, "{inputs} input(s): {f}");
        assert!(f >= redeem::MAX_BUMP_FEE_SATS + custody::vsize(inputs, &[34]), "BIP125 rule 4");
    }
    assert!(custody::vsize(2, &[34, 34]) * custody::MAX_FEE_RATE + 330 < redeem::MAX_BUMP_FEE_SATS);
}

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
    assert_eq!(allowed, 3);
    assert!(o.frozen().is_none());
}

// ---- N-3: one explorer reporting low no longer sets the custody rate ------------------------------------------------------------

#[test]
fn audit2_one_explorer_reporting_a_low_estimate_no_longer_sets_the_custody_rate() {
    assert_eq!(custody::combine_rates(&[1, 150]), Ok(custody::MAX_FEE_RATE), "the honest estimate (clamped), not 2");
    assert_eq!(custody::combine_rates(&[0, 150]), Ok(custody::MAX_FEE_RATE));
    let (_, alarm) = custody::choose_rate(&[1, 150]).unwrap();
    assert!(alarm.is_some_and(|a| a.contains("DISAGREE")), "said, for the watcher");
    // the conflict priced from it is at the market
    let leaked = custody::vsize(2, &[34, 34]) * 100;
    assert!(conflict::fee_for(1, leaked, 100) >= custody::vsize(1, &[34]) * 100);
}

// ---- N-2: pinning through output 0, against TRUC deliveries and a re-priced conflict-out ---------------------------------------

/// What spends the fee input now, in the mempool or a block.
#[derive(Clone, Debug)]
struct Mem {
    txid: [u8; 32],
    fee: u64,
    vs: u64,
    delivery: bool,
    /// The redeemer's child on output 0: (fee, vsize).
    child: Option<(u64, u64)>,
    confirmed: bool,
}

/// A mempool with the replacement rules that matter here (BIP125 rules 3 and 4, which cluster-mempool RBF keeps) and TRUC's
/// child limit: the redeemer hangs the largest child that keeps the delivery's package under the market. Under TRUC
/// (`nVersion` 3) that child is at most 1,000 vB; on a version-2 delivery it would be R2's 100 kvB pin at 1 sat/vB. Each tick
/// a block takes whatever pays the market.
struct PinWorld {
    now: u64,
    market: u64,
    fee: Utxo,
    spender: Option<Mem>,
    versions: Vec<u32>,
    refunded: bool,
    /// The wrapper of r3-contracts: `refund` at `deliverBy + REFUND_GRACE` needs the custodian's `markUndeliverable`.
    gated: bool,
    rejected: usize,
    signs: usize,
    lines: Vec<String>,
    caps: custodycap::CapPolicy,
    book: custodycap::Book,
}

const CHILD: [u8; 32] = [0xc1; 32];

fn display_txid(raw: &[u8]) -> [u8; 32] {
    let mut t = etch_autobuyer::btcsend::parse_signed(raw).unwrap().txid;
    t.reverse();
    t
}

impl PinWorld {
    fn mine(&mut self) {
        let market = self.market;
        if let Some(m) = self.spender.as_mut().filter(|m| !m.confirmed) {
            let (cf, cv) = m.child.unwrap_or((0, 0));
            if (m.fee + cf) >= market * (m.vs + cv) {
                m.confirmed = true;
            }
        }
    }
    fn tick(&mut self, c: &mut Custody) {
        let lines = block_on(redeem::tick(self, c, false));
        self.lines.extend(lines);
        self.mine();
    }
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
        let spent = self.spender.is_some();
        if *o == FROG {
            return Ok((Output { script: ours(), value: 546, confirmed: true }, self.spender.as_ref().is_some_and(|m| m.delivery)));
        }
        if *o == self.fee.outpoint {
            return Ok((Output { script: ours(), value: self.fee.value, confirmed: true }, spent));
        }
        Err("unknown output".into())
    }
    async fn our_utxos(&mut self) -> Result<Vec<Utxo>, String> {
        Ok(if self.spender.is_none() { vec![self.fee.clone()] } else { vec![] })
    }
    async fn fee_rate(&mut self) -> Result<u64, String> { Ok(self.market) }
    async fn our_script(&mut self) -> Result<Vec<u8>, String> { Ok(ours()) }
    fn btc_hrp(&self) -> Result<&'static str, String> { Ok("bc") }
    async fn sign_taproot(&mut self, _h: [u8; 32]) -> Result<Vec<u8>, String> {
        self.signs += 1;
        Ok(vec![0x5a; 64])
    }
    async fn btc_send(&mut self, raw: &[u8]) -> Result<(), String> {
        let id = display_txid(raw);
        let t = outbound::parse_tx(raw)?;
        let spends = |o: &Outpoint| t.inputs.iter().any(|i| custody::outpoint_of(&etch_autobuyer::psbt::TxIn { txid: i.txid, vout: i.vout,
            sequence: 0 }) == *o);
        let delivery = spends(&FROG);
        let value_in = if delivery { 546 } else { 0 } + if spends(&self.fee.outpoint) { self.fee.value } else { 0 };
        let fee = value_in - t.outputs.iter().map(|o| o.value).sum::<u64>();
        let vs = custody::vsize(t.inputs.len(), &t.outputs.iter().map(|o| o.script.len()).collect::<Vec<_>>());
        if delivery {
            self.versions.push(t.version);
        }
        // the redeemer's child: under TRUC at most 1,000 vB, carrying the most fee that keeps the package under the market
        let child = |fee: u64, vs: u64| -> Option<(u64, u64)> {
            if t.version == 3 {
                Some(((self.market * (vs + 1_000)).saturating_sub(fee + 1), 1_000))
            } else {
                Some((100_000, 100_000)) // R2's pin: 100 kvB at 1 sat/vB
            }
        };
        match self.spender.clone() {
            None => self.spender = Some(Mem { txid: id, fee, vs, delivery, child: if delivery { child(fee, vs) } else { None }, confirmed: false }),
            Some(m) if m.txid == id || m.confirmed => {}
            Some(m) => {
                let (cf, _) = m.child.unwrap_or((0, 0));
                if fee >= m.fee + cf + vs {
                    self.spender = Some(Mem { txid: id, fee, vs, delivery, child: if delivery { child(fee, vs) } else { None }, confirmed: false });
                } else {
                    self.rejected += 1; // "insufficient fee, rejecting replacement"
                }
            }
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
    async fn send_evm(&mut self, _r: &[u8]) -> Result<(), String> { Ok(()) }
    async fn evm_seen(&mut self, _h: [u8; 32]) -> EvmView { EvmView::NotYet }
    fn save(&mut self, _r: &Redemption) {}
    fn save_cursor(&mut self, _c: u64) {}
    fn with_caps<R>(&mut self, f: impl FnOnce(&custodycap::CapPolicy, &mut custodycap::Book) -> R) -> R { f(&self.caps, &mut self.book) }
    fn outflow_check(&mut self, _k: &str, _c: &str, _f: outflow::Flow, _a: u128) -> Result<(), String> { Ok(()) }
    fn outflow_record(&mut self, _k: &str, _c: &str, _f: outflow::Flow, _a: u128) {}
    async fn scan_ticket_logs(&mut self, _l: &mut TicketLogs, _w: Option<&[u64]>) -> Result<String, String> { Ok("nothing new".into()) }
    fn outbound_policy(&self, s: &[u8]) -> outbound::BtcPolicy {
        outbound::BtcPolicy { custody_script: s.to_vec(), max_fee_sats: outbound::MAX_FEE_SATS, require_finalized: true, require_event: true }
    }
    fn paused(&self) -> bool { false }
    async fn outspend(&mut self, o: &Outpoint) -> Result<Option<([u8; 32], bool)>, String> {
        let Some(m) = &self.spender else { return Ok(None) };
        if *o == self.fee.outpoint || (*o == FROG && m.delivery) {
            return Ok(Some((m.txid, m.confirmed)));
        }
        if *o == (Outpoint { txid: m.txid, vout: 0 }) && m.delivery && m.child.is_some() {
            return Ok(Some((CHILD, m.confirmed)));
        }
        Ok(None)
    }
}

fn pin_world(market: u64) -> (PinWorld, Custody) {
    (PinWorld { now: T0, market, fee: Utxo { outpoint: op(0xfe, 1), value: 150_000, confirmed: true }, spender: None, versions: vec![],
        refunded: false, gated: false, rejected: 0, signs: 0, lines: vec![], caps: caps(), book: Default::default() }, with_events())
}

/// Ten days: the delivery is sent into a sustained spike and pinned; at `deliverBy + 72 h` the redeemer refunds and confirms
/// whatever of theirs is still in the mempool.
fn run_pinned(w: &mut PinWorld, c: &mut Custody) {
    for i in 0..(11 * 144) {
        w.now = T0 + i * 600;
        if w.now >= T0 + 10 * 86_400 && !w.refunded {
            // deliverBy + 72 h: the redeemer refunds (refused by a gated wrapper: nothing attested the delivery undeliverable)
            w.refunded = !w.gated;
            if let Some(m) = w.spender.as_mut().filter(|m| m.delivery && !m.confirmed) {
                m.confirmed = true; // the redeemer CPFPs their own child
            }
        }
        w.tick(c);
    }
}

/// R2's N-2 scenario in a 60 sat/vB spike that lasts all ten days. Every delivery is TRUC, so the redeemer's child is at most
/// 1,000 vB and the most it can carry while the package stays unmined is ≈ 60k sats. Every bump is refused, as in R2; but the
/// conflict-out is re-priced every two hours until it outbids the delivery and the child, and the frog never leaves.
#[test]
fn audit2_a_redeemer_who_pins_a_truc_delivery_in_a_spike_is_outbid_by_the_re_priced_conflict_and_gets_no_frog() {
    let (mut w, mut c) = pin_world(60);
    run_pinned(&mut w, &mut c);
    assert!(!w.versions.is_empty() && w.versions.iter().all(|v| *v == 3), "every delivery version is TRUC: {:?}", w.versions);
    assert!(w.rejected > 0, "the pin refused the bumps, as in R2");
    assert!(w.lines.iter().any(|l| l.contains("PIN WATCH")), "and the watcher was told");
    let RPhase::Refused { reason } = &c.redemptions[0].phase else { panic!("{:?}", c.redemptions[0].phase) };
    assert!(reason.contains("conflicted out"), "{reason}");
    let m = w.spender.clone().unwrap();
    assert!(m.confirmed && !m.delivery && m.fee <= conflict::MAX_CONFLICT_FEE_SATS, "our conflict confirmed at {} sats", m.fee);
    // and the wrapper is told the delivery can no longer confirm, once the conflict is deep: the refund is not held a month
    assert!(matches!(c.redemptions[0].attest, Some(redeem::Attest::Sent { .. })), "{:?}", c.redemptions[0].attest);
    println!("REVIEW2: pinned TRUC delivery at 60 sat/vB: {} refused replacements, conflict confirmed at {} sats", w.rejected, m.fee);
}

/// The bound, measured (fixes-R3 §2.1): in a spike above ≈ 82 sat/vB held for four days, a redeemer who commits ≈ 100k sats of
/// child fee (≤ 1,000 vB) can still out-last a conflict at its 100,000-sat ceiling. Against the old wrapper (refund at
/// `deliverBy + 72 h`) that ends with the frog and the refund; the redeemer pays the child's fee. Against r3-contracts' wrapper
/// the refund needs the custodian's `markUndeliverable`, which the canister sends only once its conflict is deep, so the
/// pinned delivery that confirms is simply recorded.
#[test]
fn audit2_residual_a_pin_in_a_spike_past_the_conflict_ceiling_costs_the_redeemer_its_child_fee() {
    let (mut w, mut c) = pin_world(100);
    run_pinned(&mut w, &mut c);
    assert!(matches!(c.redemptions[0].phase, RPhase::LostRace { .. }), "{:?}", c.redemptions[0].phase);
    let m = w.spender.clone().unwrap();
    let (child_fee, child_vs) = m.child.unwrap();
    assert_eq!(child_vs, 1_000, "TRUC: the child is at most 1,000 vB");
    assert!(child_fee >= 75_000, "the pin cost the redeemer {child_fee} sats of fee");
    assert!(w.lines.iter().any(|l| l.contains("cannot pay more")), "the conflict reached its ceiling and says so");
    // with r3-contracts' wrapper the refund waits for the custodian's attestation, which never came (the conflict never confirmed):
    // the pinned delivery that confirms is recorded, and the unit stays backed
    let (mut w, mut c) = pin_world(100);
    w.gated = true;
    run_pinned(&mut w, &mut c);
    assert!(matches!(c.redemptions[0].phase, RPhase::Recording { .. } | RPhase::Confirmed { .. }), "{:?}", c.redemptions[0].phase);
    assert!(c.redemptions[0].attest.is_none(), "nothing was attested: the delivery could still confirm");
}

// ---- I-3 / N-9: a failed checkpoint check gates the frog pass too ---------------------------------------------------------------

#[test]
fn audit2_the_bitcoin_checkpoint_gates_the_frog_pass_as_well_as_the_custodian() {
    for (name, src, pass) in [("frognet.rs", include_str!("../src/frognet.rs"), "tick_all("),
        ("custodynet.rs", include_str!("../src/custodynet.rs"), "redeem::tick(")] {
        let tick = src.split("pub(crate) async fn tick(").nth(1).unwrap_or_else(|| panic!("{name}: no tick"));
        let gate = tick.find("ensure_btc_network().await").unwrap_or_else(|| panic!("{name}: the frog/custody pass is not gated"));
        let run = tick.find(pass).unwrap_or_else(|| panic!("{name}: no pass"));
        assert!(gate < run, "{name}: the checkpoint is checked before the pass reads Bitcoin");
    }
}
