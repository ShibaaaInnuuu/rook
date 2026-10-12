//! R4 check (independent review of fix round 4) of the ticket-log scanner's R3-C1 / R3-C2 fixes. These tests probe the NEW code
//! paths (growth inside `hot`, `hot` clearing, the `Split` verdict, the unreadable retry) for liveness regressions and skips.
//! A pass means the property named in the test holds.

mod fakerpc;

use std::cell::RefCell;
use std::future::Future;
use std::pin::pin;
use std::rc::Rc;
use std::task::{Context, Poll, Waker};

use etch_autobuyer::evmread::{self, Rpc, SIZE_REJECT};
use etch_autobuyer::ticketlog::{self, TicketLogs, MAX_TRIES_PER_TICK};
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

fn get_logs(ps: &[FakeRpc]) -> usize {
    ps[0].asked.borrow().iter().filter(|(m, ..)| m == "eth_getLogs").count()
}

/// A provider that pads EVERY eth_getLogs answer past the cap (a real IC reject from this provider only), whatever the query.
struct Padder {
    inner: FakeRpc,
    on: bool,
}

impl Rpc for Padder {
    fn name(&self) -> String { format!("padder-{}", self.inner.name()) }
    async fn head(&mut self, b: u64) -> Result<u64, String> { self.inner.head(b).await }
    async fn finalized(&mut self, b: u64) -> Result<u64, String> { self.inner.finalized(b).await }
    async fn block(&mut self, n: u64) -> Result<serde_json::Value, String> { self.inner.block(n).await }
    async fn call(&mut self, to: &[u8; 20], d: &[u8], at: &etch_autobuyer::pin::Pin) -> Result<Vec<u8>, String> { self.inner.call(to, d, at).await }
    async fn logs(&mut self, a: &[u8; 20], t: &serde_json::Value, from: u64, to: u64, m: u64)
        -> Result<Vec<evmread::LogView>, String> {
        if self.on {
            // counted in the inner provider's log, for the per-tick bound
            self.inner.asked.borrow_mut().push(("eth_getLogs".into(), from, to));
            return Err(format!("{SIZE_REJECT}Http body exceeds size limit of {m} bytes."));
        }
        self.inner.logs(a, t, from, to, m).await
    }
}

/// One provider that pads every answer can stall the scan (as a provider returning any error always could), but: nothing is
/// skipped or set aside, a tick never exceeds the outcall bound, and in the steady state the stall costs ~1 round a tick (the
/// `dense` state keeps its one-ticket shrink), not a fresh bisection every tick.
#[test]
fn audit4_a_provider_padding_every_answer_stalls_cheaply_and_skips_nothing() {
    let c = Rc::new(RefCell::new(Chain { head: HEAD, logs: vec![] }));
    let start = HEAD - 60_000;
    c.borrow_mut().logs.push(fakerpc::redeemed(&W, 5, DEST, start + 100));
    c.borrow_mut().logs.push(fakerpc::resolved(&W, 5, 3, start + 101));
    let mut ps: Vec<Padder> = fakerpc::pair(&c).into_iter().enumerate().map(|(i, p)| Padder { inner: p, on: i == 1 }).collect();
    let mut idx = TicketLogs::default();
    let mut per_tick = vec![];
    let mut last = String::new();
    for _ in 0..40 {
        let before = ps[0].inner.asked.borrow().iter().filter(|(m, ..)| m == "eth_getLogs").count();
        if let Err(e) = block_on(ticketlog::advance(&mut ps, &mut idx, &W, Some(start), Some(&[5]))) {
            last = e;
        }
        let after = ps[0].inner.asked.borrow().iter().filter(|(m, ..)| m == "eth_getLogs").count();
        per_tick.push(after - before);
        assert!(after - before <= MAX_TRIES_PER_TICK, "a tick's outcalls are bounded: {}", after - before);
    }
    println!("R4 padder: eth_getLogs per tick (provider 0): {per_tick:?}; cursor {:?} (start {start}); last error: {last}", idx.cursor);
    assert!(idx.cursor.is_some_and(|x| x < start + 100), "stalled before the ticket's block");
    assert_eq!(idx.unreadable, None, "nothing set aside");
    assert!(idx.redeemed_of(5).is_empty());
    assert!(per_tick[10..].iter().all(|n| *n <= 2), "steady state is cheap: {per_tick:?}");
    // the padder stops: the scan resumes and reads the ticket
    ps[1].on = false;
    for _ in 0..10 {
        let _ = block_on(ticketlog::advance(&mut ps, &mut idx, &W, Some(start), Some(&[5])));
        if idx.has_both(5) {
            break;
        }
    }
    assert!(idx.has_both(5), "recovered once the provider answers: {:?}", idx.cursor);
}

/// Dense (500-event, 400 kB < x < 2 MB) blocks at a sweep of offsets in the first page, plus a run of 20 consecutive dense
/// blocks: every event read, no skip, the honest ticket past them read within a handful of ticks, every tick bounded.
#[test]
fn audit4_dense_blocks_at_any_offset_and_in_runs_are_read_without_crawling() {
    for (offsets, label) in [
        (vec![0u64], "offset 0"),
        (vec![1], "offset 1"),
        (vec![3_999], "offset 3,999"),
        (vec![4_000], "offset 4,000"),
        (vec![7_999], "offset 7,999 (page end)"),
        (vec![8_000], "offset 8,000 (next page start)"),
        ((0..20).collect(), "20 consecutive"),
        ((0..20).map(|i| i * 2).collect(), "20 alternate"),
        ((0..10).map(|i| 100 + i * 700).collect(), "10 spread 700 apart"),
    ] {
        let c = Rc::new(RefCell::new(Chain { head: HEAD, logs: vec![] }));
        let mut ps = fakerpc::pair(&c);
        let start = HEAD - 60_000;
        let mut t = 0u64;
        for o in &offsets {
            for _ in 0..500 {
                c.borrow_mut().logs.push(fakerpc::redeemed(&W, t, DEST, start + o));
                t += 1;
            }
        }
        let honest = t;
        c.borrow_mut().logs.push(fakerpc::redeemed(&W, honest, DEST, start + 7_990 + offsets.last().unwrap()));
        c.borrow_mut().logs.push(fakerpc::resolved(&W, honest, 7, start + 7_991 + offsets.last().unwrap()));
        let wanted: Vec<u64> = (0..=honest).collect();
        let mut idx = TicketLogs::default();
        let mut ticks = 0;
        while !idx.has_both(honest) {
            ticks += 1;
            assert!(ticks <= 60, "{label}: honest ticket unread after {ticks} ticks, cursor {:?} span {} hot {:?}", idx.cursor, idx.span, idx.hot);
            let before = get_logs(&ps);
            block_on(ticketlog::advance(&mut ps, &mut idx, &W, Some(start), Some(&wanted))).unwrap();
            assert!(get_logs(&ps) - before <= MAX_TRIES_PER_TICK);
        }
        assert!((0..honest).all(|t| !idx.redeemed_of(t).is_empty()), "{label}: every burst event read");
        assert_eq!(idx.unreadable, None);
        println!("R4 dense sweep: {label:>28}: honest ticket read in {ticks} ticks ({} eth_getLogs a provider), span now {}",
            get_logs(&ps), idx.span);
    }
}
