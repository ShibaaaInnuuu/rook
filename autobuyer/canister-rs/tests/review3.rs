//! The R3 check's canister findings (docs/audit/2026-09-26/review3/R3-check.md, part-canister.md) as regression tests
//! (docs/audit/2026-09-26/review3/fixes-R4-canister.md). The R3 PoCs (`review3/poc/audit3_scanner.rs`, `audit3_pin.rs`, kept
//! there as the review's record) asserted each weakness; these are the same scenarios, passing only because the weaknesses are
//! gone. N-2 is bounded rather than closed: its test pins the bound.

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

// ---- R3-C1: after a dense block the scanner grows back to full pages ---------------------------------------------------------

/// A provider (any `Rpc`) with its `eth_getLogs` counted, so every tick's outcalls can be bounded.
fn get_logs(ps: &[FakeRpc]) -> usize {
    ps[0].asked.borrow().iter().filter(|(m, ..)| m == "eth_getLogs").count()
}

/// Ticks until the honest ticket's events are read, with a burst of `n` Redeemed logs in the first block of the first page and
/// the honest ticket `gap` blocks after it. Every tick is checked against the outcall bound. `(ticks, eth_getLogs a provider)`.
fn ticks_to_read_honest(n: u64, gap: u64) -> (u32, usize) {
    let c = Rc::new(RefCell::new(Chain { head: HEAD, logs: vec![] }));
    let mut ps = fakerpc::pair(&c);
    let start = HEAD - 60_000;
    let burst = start;
    for t in 0..n {
        c.borrow_mut().logs.push(fakerpc::redeemed(&W, t, DEST, burst));
    }
    c.borrow_mut().logs.push(fakerpc::redeemed(&W, n, DEST, burst + gap));
    c.borrow_mut().logs.push(fakerpc::resolved(&W, n, 7, burst + gap + 1));
    let wanted: Vec<u64> = (0..=n).collect();
    let mut idx = TicketLogs::default();
    let mut ticks = 0;
    while !idx.has_both(n) {
        ticks += 1;
        assert!(ticks <= 50, "the honest ticket is still unread after {ticks} ticks: cursor {:?}, span {}", idx.cursor, idx.span);
        let before = get_logs(&ps);
        block_on(ticketlog::advance(&mut ps, &mut idx, &W, Some(start), Some(&wanted))).unwrap();
        assert!(get_logs(&ps) - before <= MAX_TRIES_PER_TICK, "a tick's outcalls are bounded");
    }
    assert!(idx.hot.is_none(), "the dense range is behind the scan: {:?}", idx.hot);
    assert!((0..n).all(|t| !idx.redeemed_of(t).is_empty()), "the burst's own events are read too");
    (ticks, get_logs(&ps))
}

/// R3-C1 (R3's PoC `audit3_a_dense_block_at_the_start_of_a_page_makes_the_scanner_crawl_…`: 502, 1,000 and 505 ticks). The span
/// now doubles after every success, inside the bisected range too, and `hot` is cleared once the dense block is read: the pages
/// are back to 8,000 blocks within a couple of ticks.
#[test]
fn audit3_after_a_dense_block_the_scanner_grows_back_to_full_pages_within_a_few_ticks() {
    let (t60, _) = ticks_to_read_honest(60, 4_000);
    assert_eq!(t60, 1, "R2's 60-event burst: read in the first tick");
    let (t500, calls) = ticks_to_read_honest(500, 4_000);
    println!("REVIEW3 R3-C1: 500-event block, honest ticket 4,000 blocks on: {t500} ticks, {calls} eth_getLogs a provider (was 502 ticks)");
    assert!(t500 <= 4, "{t500}");
    let (t_far, calls) = ticks_to_read_honest(500, 7_990);
    println!("REVIEW3 R3-C1: honest ticket 7,990 blocks on: {t_far} ticks, {calls} eth_getLogs (was 1,000 ticks, ≈ 6.9 days)");
    assert!(t_far <= 4, "{t_far}");
    let (t2400, calls) = ticks_to_read_honest(2_400, 4_000);
    println!("REVIEW3 R3-C1: 2,400-event block read by ticket, honest ticket 4,000 blocks on: {t2400} ticks, {calls} eth_getLogs (was 505)");
    assert!(t2400 <= 12, "{t2400}");
}

/// After the dense block the span keeps doubling to the full page, and a second dense block further on is bisected again (the
/// growth does not skip anything: every event of both bursts and of the tickets between is read).
#[test]
fn audit3_growth_after_a_dense_block_still_reads_a_second_dense_block_and_everything_between() {
    let c = Rc::new(RefCell::new(Chain { head: HEAD, logs: vec![] }));
    let mut ps = fakerpc::pair(&c);
    let start = HEAD - 60_000;
    let mut t = 0;
    for (block, n) in [(start + 10, 500), (start + 20_000, 500)] {
        for _ in 0..n {
            c.borrow_mut().logs.push(fakerpc::redeemed(&W, t, DEST, block));
            t += 1;
        }
    }
    for k in 0..40u64 {
        c.borrow_mut().logs.push(fakerpc::redeemed(&W, t + k, DEST, start + 11 + k * 997));
    }
    let total = t + 40;
    let mut idx = TicketLogs::default();
    let mut ticks = 0;
    while idx.finalized.is_none() || idx.cursor < idx.finalized {
        ticks += 1;
        assert!(ticks <= 30, "stuck at {:?}", idx.cursor);
        block_on(ticketlog::advance(&mut ps, &mut idx, &W, Some(start), Some(&(0..total).collect::<Vec<_>>()))).unwrap();
    }
    assert_eq!((0..total).filter(|t| idx.redeemed_of(*t) == vec![DEST.to_string()]).count() as u64, total, "every event, once");
    assert_eq!(idx.span, evmread::MAX_LOG_SPAN_BLOCKS, "back to full pages");
    assert!(idx.hot.is_none() && idx.cap.is_none());
    println!("REVIEW3 R3-C1: two 500-event blocks 20,000 blocks apart and 40 tickets between: to finality in {ticks} ticks");
}

// ---- R3-C2: one provider can no longer make the scanner skip a ticket's events -----------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Lie {
    /// R3's PoC: a JSON-RPC error whose message mentions a size limit (a provider's own words).
    Text,
    /// A provider that pads its answer for that block past every cap: a real IC reject, from this provider only.
    Pad,
    Honest,
}

/// A provider wrapper that answers any `eth_getLogs` touching `block` with a size-looking failure, per `lie`.
struct Censor {
    inner: FakeRpc,
    block: u64,
    lie: Lie,
}

impl Rpc for Censor {
    fn name(&self) -> String { format!("censor-{}", self.inner.name()) }
    async fn head(&mut self, b: u64) -> Result<u64, String> { self.inner.head(b).await }
    async fn finalized(&mut self, b: u64) -> Result<u64, String> { self.inner.finalized(b).await }
    async fn block(&mut self, n: u64) -> Result<serde_json::Value, String> { self.inner.block(n).await }
    async fn call(&mut self, to: &[u8; 20], d: &[u8], at: &etch_autobuyer::pin::Pin) -> Result<Vec<u8>, String> { self.inner.call(to, d, at).await }
    async fn logs(&mut self, a: &[u8; 20], t: &serde_json::Value, from: u64, to: u64, m: u64)
        -> Result<Vec<evmread::LogView>, String> {
        if (from..=to).contains(&self.block) {
            match self.lie {
                Lie::Text => return Err(format!("no result: {{\"code\":-32000,\"message\":\"response size limit of {m} bytes exceeded\"}}")),
                Lie::Pad => return Err(format!("{SIZE_REJECT}Http body exceeds size limit of {m} bytes.")),
                Lie::Honest => {}
            }
        }
        self.inner.logs(a, t, from, to, m).await
    }
}

/// The honest Robinhood provider first, then ordofi lying about `target` (R3's PoC layout); tickets 5 (at `target`) and 6.
fn censored(lie: Lie) -> (Vec<Censor>, u64) {
    let c = Rc::new(RefCell::new(Chain { head: HEAD, logs: vec![] }));
    let target = HEAD - 40_000;
    c.borrow_mut().logs.push(fakerpc::redeemed(&W, 5, DEST, target));
    c.borrow_mut().logs.push(fakerpc::redeemed(&W, 6, DEST, target + 3));
    c.borrow_mut().logs.push(fakerpc::resolved(&W, 6, 9, target + 4));
    let ps = fakerpc::pair(&c).into_iter().enumerate()
        .map(|(i, p)| Censor { inner: p, block: target, lie: if i == 1 { lie } else { Lie::Honest } }).collect();
    (ps, target)
}

/// R3-C2 (R3's PoC `audit3_one_provider_can_make_the_scanner_skip_a_chosen_tickets_events_for_good`: skipped in 3 ticks), for both
/// kinds of lie. The lying provider can stall the scan at the block (loud: `failures`, the error names it), as any failing provider
/// always could, but it can no longer make the scanner pass the block without ticket 5's event. Once it answers honestly, the event
/// is read and the scan goes on.
#[test]
fn audit3_one_provider_can_stall_the_scanner_but_never_make_it_skip_a_tickets_events() {
    for lie in [Lie::Text, Lie::Pad] {
        let (mut ps, target) = censored(lie);
        let mut idx = TicketLogs::default();
        let mut last = String::new();
        for _ in 0..200 {
            if let Err(e) = block_on(ticketlog::advance(&mut ps, &mut idx, &W, Some(target - 5_000), Some(&[5, 6]))) {
                last = e;
            }
            assert!(idx.cursor.is_some_and(|x| x < target), "{lie:?}: the cursor never passes the censored block: {:?}", idx.cursor);
        }
        assert_eq!(idx.unreadable, None, "{lie:?}: nothing set aside");
        assert!(idx.redeemed_of(5).is_empty() && idx.failures > 0, "{lie:?}");
        assert!(last.contains("censor-Ordofi"), "{lie:?}: the error names the provider: {last}");
        if lie == Lie::Pad {
            assert!(last.contains("do not agree") && last.contains("nothing is skipped"), "{last}");
        }
        println!("REVIEW3 R3-C2 ({lie:?}): stalled before block {target}, {} failures: {last}", idx.failures);
        // the provider answers honestly again: the block is read, ticket 5's event with it
        ps[1].lie = Lie::Honest;
        let mut ticks = 0;
        while !(idx.has_both(6) && idx.cursor.is_some_and(|x| x > target)) {
            ticks += 1;
            assert!(ticks <= 20, "{lie:?}: not recovered: {:?}", idx.cursor);
            block_on(ticketlog::advance(&mut ps, &mut idx, &W, Some(target - 5_000), Some(&[5, 6]))).unwrap();
        }
        assert_eq!(idx.redeemed_of(5), vec![DEST.to_string()], "{lie:?}: ticket 5's event is read");
        assert_eq!((idx.failures, idx.unreadable.clone()), (0, None));
    }
}

/// `logs_sized` compares the size verdict across providers: every provider over the cap is `TooBig`, some is `Split` (named),
/// and a provider's text about a size limit is an ordinary failure.
#[test]
fn audit3_the_size_verdict_is_compared_across_providers() {
    let topics = ticketlog::topics();
    let (mut ps, target) = censored(Lie::Pad);
    match block_on(evmread::logs_sized(&mut ps, &W, &topics, target, target, 48_000)) {
        Err(LogsError::Split(s)) => assert!(s.contains("Robinhood answered") && s.contains("censor-Ordofi said too big"), "{s}"),
        other => panic!("{other:?}"),
    }
    let (mut ps, target) = censored(Lie::Text);
    assert!(matches!(block_on(evmread::logs_sized(&mut ps, &W, &topics, target, target, 48_000)), Err(LogsError::Failed(e)) if e.contains("censor-Ordofi")));
    // every provider over the cap
    let c = Rc::new(RefCell::new(Chain { head: HEAD, logs: vec![] }));
    for t in 0..100 {
        c.borrow_mut().logs.push(fakerpc::redeemed(&W, t, DEST, HEAD - 30_000));
    }
    let mut ps = fakerpc::pair(&c);
    assert_eq!(block_on(evmread::logs_sized(&mut ps, &W, &topics, HEAD - 30_000, HEAD - 30_000, 48_000)), Err(LogsError::TooBig));
    assert_eq!(get_logs(&ps[1..]), 1, "the second provider is asked too, although the first was over the cap");
    assert_eq!(block_on(evmread::logs_sized(&mut ps, &W, &topics, HEAD - 30_000, HEAD - 30_000, 400_000)).unwrap().len(), 100);
}

/// Every provider too big for one ticket (something the wrapper cannot emit) still sets it aside so the cursor advances (R2's
/// N-1 guarantee), but it is no longer dropped for good: it is asked again each tick, and read once the providers answer.
#[test]
fn audit3_a_set_aside_ticket_is_asked_again_and_read_once_the_providers_answer() {
    let c = Rc::new(RefCell::new(Chain { head: HEAD, logs: vec![] }));
    let mut ps = fakerpc::pair(&c);
    let dense = HEAD - 30_000;
    for _ in 0..2_400 {
        c.borrow_mut().logs.push(fakerpc::redeemed(&W, 5, DEST, dense)); // impossible, and too big even alone
    }
    c.borrow_mut().logs.push(fakerpc::redeemed(&W, 6, DEST, dense));
    let mut idx = TicketLogs::default();
    let mut ticks = 0;
    while idx.cursor.is_none_or(|x| x <= dense) {
        ticks += 1;
        assert!(ticks <= 6, "stuck: {:?}", idx.dense);
        block_on(ticketlog::advance(&mut ps, &mut idx, &W, Some(dense - 5_000), Some(&[5, 6]))).unwrap();
    }
    assert_eq!(idx.unreadable, Some(vec![(dense, 5)]), "set aside, loud");
    // still broken: asked again once a tick at the cheap cap, and kept
    let before = ps[0].caps.borrow().len();
    let line = block_on(ticketlog::advance(&mut ps, &mut idx, &W, Some(dense - 5_000), Some(&[5, 6]))).unwrap();
    assert!(line.contains("set-aside ticket 5") && line.contains("still unread"), "{line}");
    assert_eq!(ps[0].caps.borrow()[before], ticketlog::UNREADABLE_RETRY_CAP);
    assert_eq!(idx.unreadable, Some(vec![(dense, 5)]));
    // the providers answer: the event is read and the entry goes
    c.borrow_mut().logs.retain(|l| !(l.block == dense && l.topics[1] == fakerpc::word(5)));
    c.borrow_mut().logs.push(fakerpc::redeemed(&W, 5, DEST, dense));
    let line = block_on(ticketlog::advance(&mut ps, &mut idx, &W, Some(dense - 5_000), Some(&[5, 6]))).unwrap();
    assert!(line.contains("read again (1 event(s))"), "{line}");
    assert_eq!((idx.unreadable.clone(), idx.redeemed_of(5)), (None, vec![DEST.to_string()]));
    // an entry for a ticket no longer wanted (its redemption finished) is dropped without an outcall
    idx.unreadable = Some(vec![(dense, 9)]);
    let line = block_on(ticketlog::advance(&mut ps, &mut idx, &W, Some(dense - 5_000), Some(&[5, 6]))).unwrap();
    assert_eq!(idx.unreadable, None);
    assert!(!line.contains("set-aside"), "no retry for it: {line}");
    // with the tickets not walked this tick (what is wanted is not known), it is not dropped but asked again: the providers agree
    // that block holds nothing for ticket 9, so the entry is settled by that answer
    idx.unreadable = Some(vec![(dense, 9)]);
    let line = block_on(ticketlog::advance(&mut ps, &mut idx, &W, Some(dense - 5_000), None)).unwrap();
    assert!(line.contains("set-aside ticket 9") && line.contains("read again (0 event(s))"), "{line}");
    assert_eq!(idx.unreadable, None);
}

// ---- N-2 (accepted residual, bounded) and M-1 (c): R3's pin re-run ----------------------------------------------------------

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
        (World { blocks_take_conflict: true, rate: 100, now: T0, fee: Utxo { outpoint: Outpoint { txid: [0xfe; 32], vout: 1 }, value: 150_000, confirmed: true }, spender: None,
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
        if let Some(m) = self.spender.as_mut().filter(|m| !m.confirmed && !m.delivery && self.blocks_take_conflict) {
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

/// N-2's accepted residual, pinned (R3 check: "fixed, bounded"; fixes-R4-canister §3). R2's own mempool (NO replacement of the
/// pinned delivery is ever accepted): against the gated wrapper the canister never attests (its conflict never confirms), so the
/// refund waits `LATE_DELIVERY_WINDOW`; a redeemer who keeps the pin for 30 days then refunds and confirms the delivery:
/// LostRace. So N-2 is closed for any pin shorter than 30 days (and, per fixes-R3-canister §2.1, TRUC makes such a pin need a
/// ~80+ sat/vB market all that time), not closed absolutely. If this test starts failing, the bound moved: say so in the docs.
#[test]
fn audit3_n2_bound_an_unbreakable_pin_needs_thirty_days_and_the_canister_never_attests_falsely() {
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
    println!("REVIEW3 N-2 bound: unbreakable pin: {} replacements refused, 0 attestations, refund at deliverBy + 30 d, then LOST RACE", w.rejected);
}

/// The honest case, and M-1 (c): the buyer is paused from day 5 (before the margin) to day 20. Nothing is re-sent or bumped while
/// paused; at the margin the rule refuses and the conflict-out is signed and confirms while paused; the attestation waits for the
/// resume (not sent while paused) and then goes out, so the refund is held past deliverBy + 72 h by the pause only.
#[test]
fn audit3_m1c_a_pause_across_the_margin_still_conflicts_out_and_attests_only_after_the_resume() {
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

// ---- R3-C7: a conflict that holds the mempool only follows the market ---------------------------------------------------------

fn conflict_of(c: &Custody) -> Option<conflict::ConflictTx> {
    match &c.redemptions[0].phase {
        RPhase::Conflicting { conflict, .. } => conflict.tx.clone(),
        _ => None,
    }
}

/// R3-C7 (Info): with the gated wrapper nothing is refunded before our conflict is 6 deep, so a conflict that is in a mempool (it
/// replaced the delivery there) was needlessly escalated ×1.5 every two hours towards 100,000 sats. Now it is re-signed only when
/// today's rate is above what it pays, and then to that rate; the ×1.5 escalation is kept for a conflict that is not seen (R2's
/// pin, `audit3_n2_bound_…` and review2's re-priced conflict tests).
#[test]
fn audit3_a_conflict_in_a_mempool_is_not_escalated_it_only_follows_the_market() {
    let (mut w, mut c) = World::new(false);
    w.blocks_take_conflict = false; // our conflict sits in the mempool; no block takes it
    w.rate = 10;
    let mut first = None;
    for i in 0..(12 * 144) {
        w.now = T0 + i * 600;
        // paused after the first send, so the delivery is not bumped towards its cap before the margin (the conflict-out still
        // runs while paused, M-1 (c)); resumed once the conflict is out
        w.paused = i > 0 && first.is_none();
        w.tick(&mut c);
        if first.is_none() {
            first = conflict_of(&c).map(|t| (w.now, t));
        }
        if first.as_ref().is_some_and(|(at, _)| w.now >= at + 10 * 3600) {
            break;
        }
    }
    let (at, tx) = first.expect("conflicted out at the margin");
    assert!(!w.paused);
    let now = conflict_of(&c).unwrap();
    assert_eq!((now.txid, now.fee, now.earlier.clone()), (tx.txid, tx.fee, None),
        "ten hours in a mempool at today's rate: the same bytes, not re-priced (was ×1.5 every 2 h)");
    assert!(w.lines.iter().any(|l| l.contains("not re-priced")), "{:?}", w.lines.iter().rev().take(3).collect::<Vec<_>>());
    // today's rate rises above what it pays: re-signed once, to that rate, not ×1.5
    let vsize = custody::vsize(1, &[34]);
    w.rate = tx.fee / vsize * 3;
    let until = w.now + 2 * 3600 + 600;
    while w.now < until {
        w.now += 600;
        w.tick(&mut c);
    }
    let now = conflict_of(&c).unwrap();
    assert_eq!(now.earlier, Some(vec![tx.txid]), "one re-price");
    assert_eq!(now.fee, vsize * w.rate, "to today's rate");
    println!("REVIEW3 R3-C7: conflict at {} sats held the mempool 10 h unchanged (since {}); rate x3 -> re-signed at {} sats",
        tx.fee, at - T0, now.fee);
}

// ---- R3-C4: quarantine counts separate hours, not fee choices ---------------------------------------------------------------

/// R3-C4 (Info): three fee choices in one tick (three owed tickets) with a transient per-output explorer failure used to
/// quarantine a good entry. A failure now counts at most once in `QUARANTINE_SPACING_SECS`; quarantine takes three failing reads
/// in three separate hours, and a good read in between clears the count.
#[test]
fn audit3_a_fee_pool_entry_is_quarantined_only_after_failures_in_separate_hours() {
    let (a, b) = (Outpoint { txid: [0xa1; 32], vout: 0 }, Outpoint { txid: [0xb2; 32], vout: 1 });
    let mut pool = custody::FeePool::default();
    pool.add(a, 20_000, "change").unwrap();
    pool.add(b, 30_000, "operator").unwrap();
    // one tick, three choices, three failures: counted once
    for k in 0..3 {
        assert_eq!(pool.failed(&a, "429", T0 + k), None);
    }
    assert_eq!(pool.entries[0].fails, Some(1));
    // later in the same hour: still once
    assert_eq!(pool.failed(&a, "429", T0 + custody::QUARANTINE_SPACING_SECS - 1), None);
    assert_eq!(pool.entries[0].fails, Some(1));
    // a good read clears it
    pool.read_ok(&a);
    assert_eq!((pool.entries[0].fails, pool.entries[0].last_failed_at), (None, None));
    // three separate hours: quarantined at the third
    let h = custody::QUARANTINE_SPACING_SECS;
    assert_eq!(pool.failed(&a, "404", T0), None);
    assert_eq!(pool.failed(&a, "404", T0 + h), None);
    let line = pool.failed(&a, "404", T0 + 2 * h).expect("quarantined");
    assert!(line.contains("quarantined") && line.contains("separate hours"), "{line}");
    assert_eq!(pool.utxos().iter().map(|u| u.outpoint).collect::<Vec<_>>(), vec![b]);
}
