//! Third review (R3 check) of the ticket-log scanner (`src/ticketlog.rs`). Each test ASSERTS THE CURRENT BEHAVIOUR: a pass means
//! the weakness is present.

mod fakerpc;

use std::cell::RefCell;
use std::future::Future;
use std::pin::pin;
use std::rc::Rc;
use std::task::{Context, Poll, Waker};

use etch_autobuyer::ticketlog::{self, TicketLogs};
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

/// Ticks until the honest ticket's events are read, with a burst of `n` Redeemed logs in the first block of the first page and
/// the honest ticket `gap` blocks after it.
fn ticks_to_read_honest(n: u64, gap: u64) -> (u32, usize, TicketLogs) {
    let c = Rc::new(RefCell::new(Chain { head: HEAD, logs: vec![] }));
    let mut ps = fakerpc::pair(&c);
    let start = HEAD - 60_000; // the scanner starts at start (cursor = start - 1): the first page is [start, start + 7,999]
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
        if ticks > 5_000 {
            break;
        }
        block_on(ticketlog::advance(&mut ps, &mut idx, &W, Some(start), Some(&wanted))).unwrap();
    }
    (ticks, get_logs(&ps), idx)
}

/// R3-C1 (new): after a range too big for 400 kB is bisected down to one block, a success INSIDE the remembered `hot` range keeps
/// the span (`ticketlog.rs:251-254`), so once the dense block itself is read the scanner walks the rest of `hot` ONE BLOCK A PAGE,
/// 8 pages a tick. A burst of ~500 Redeemed events in the first block of a page delays every later ticket's events by ~1,000
/// ticks (≈ 7 days at the 10-minute tick) — past every delivery's margin, so they all refund, the outcome R2 N-1 described.
#[test]
fn audit3_a_dense_block_at_the_start_of_a_page_makes_the_scanner_crawl_one_block_a_page_through_the_hot_range() {
    // control: the R2 burst (60 events, fits at 400 kB) is read in one tick and the honest ticket 4,000 blocks later too
    let (t60, _, _) = ticks_to_read_honest(60, 4_000);
    assert_eq!(t60, 1, "R2's 60-event burst: read in the first tick");
    // 500 events (≈ 450 kB: past 400 kB, under 2 MB) in the first block of the page, the honest ticket 4,000 blocks later
    let (t500, calls, idx) = ticks_to_read_honest(500, 4_000);
    println!("R3-C1: 500-event block, honest ticket 4,000 blocks on: read after {t500} ticks, {calls} eth_getLogs a provider; span now {}, hot {:?}",
        idx.span, idx.hot);
    assert!(t500 > 400, "the honest ticket's events wait hundreds of ticks: {t500}");
    // and at the far end of the hot range (7,990 blocks on) it is about twice that
    let (t_far, _, _) = ticks_to_read_honest(500, 7_990);
    println!("R3-C1: honest ticket 7,990 blocks on: {t_far} ticks (≈ {:.1} days at 10 min a tick)", t_far as f64 / 144.0);
    assert!(t_far > 900, "{t_far}");
    // a dense block past 2 MB (read by ticket) crawls the same way afterwards
    let (t3000, _, idx) = ticks_to_read_honest(2_400, 4_000);
    println!("R3-C1: 2,400-event block (by ticket), honest ticket 4,000 blocks on: {t3000} ticks; dense blocks {:?}", idx.dense_blocks);
    assert!(t3000 > 400, "{t3000}");
}

/// A provider wrapper that answers any `eth_getLogs` touching `block` with the IC's size error (a provider padding its answer
/// past every cap, or one that echoes that text in a JSON-RPC error: `too_big` matches substrings of the whole error string).
struct Censor {
    inner: FakeRpc,
    block: u64,
    lies: bool,
}

impl etch_autobuyer::evmread::Rpc for Censor {
    fn name(&self) -> String { format!("censor-{}", self.inner.name()) }
    async fn head(&mut self, b: u64) -> Result<u64, String> { self.inner.head(b).await }
    async fn finalized(&mut self, b: u64) -> Result<u64, String> { self.inner.finalized(b).await }
    async fn block(&mut self, n: u64) -> Result<serde_json::Value, String> { self.inner.block(n).await }
    async fn call(&mut self, to: &[u8; 20], d: &[u8], at: &etch_autobuyer::pin::Pin) -> Result<Vec<u8>, String> { self.inner.call(to, d, at).await }
    async fn logs(&mut self, a: &[u8; 20], t: &serde_json::Value, from: u64, to: u64, m: u64)
        -> Result<Vec<etch_autobuyer::evmread::LogView>, String> {
        if self.lies && (from..=to).contains(&self.block) {
            return Err(format!("no result: {{\"code\":-32000,\"message\":\"response size limit of {m} bytes exceeded\"}}"));
        }
        self.inner.logs(a, t, from, to, m).await
    }
}

/// R3-C2 (new): ONE provider (of the two that must agree) can make the scanner give up on a chosen ticket's events for good: a
/// size-looking error on the block holding them sends the scanner down the dense path, the one-ticket query "fails" 3 times, the
/// ticket is set aside as `unreadable`, and the cursor moves past the block. Before R3 the same provider could only stall the
/// scan, loudly, for everyone; now it censors one ticket (its delivery waits until the margin and is refunded) while the other
/// provider saw the events. Loud (`UNREADABLE`), no loss of backing.
#[test]
fn audit3_one_provider_can_make_the_scanner_skip_a_chosen_tickets_events_for_good() {
    let c = Rc::new(RefCell::new(Chain { head: HEAD, logs: vec![] }));
    let target = HEAD - 40_000;
    c.borrow_mut().logs.push(fakerpc::redeemed(&W, 5, DEST, target));
    c.borrow_mut().logs.push(fakerpc::redeemed(&W, 6, DEST, target + 3));
    c.borrow_mut().logs.push(fakerpc::resolved(&W, 6, 9, target + 4));
    // the honest Robinhood provider first, then ordofi lying about one block
    let mut ps = fakerpc::pair(&c).into_iter().enumerate().map(|(i, p)| Censor { inner: p, block: target, lies: i == 1 }).collect::<Vec<_>>();
    let mut idx = TicketLogs::default();
    let mut ticks = 0;
    while idx.cursor.is_none_or(|x| x <= target) {
        ticks += 1;
        assert!(ticks <= 200, "stuck at {:?}", idx.cursor);
        block_on(ticketlog::advance(&mut ps, &mut idx, &W, Some(target - 5_000), Some(&[5, 6]))).unwrap();
    }
    println!("R3-C2: cursor past the censored block after {ticks} ticks; unreadable {:?}; ticket 6 read: {}", idx.unreadable, idx.has_both(6));
    assert!(idx.unreadable.as_ref().is_some_and(|u| u.contains(&(target, 5))), "ticket 5's Redeemed event is given up for good");
    assert!(idx.redeemed_of(5).is_empty());
    assert!(idx.has_both(6), "the rest of the chain is read normally");
}
