//! Audit D, H-1: the reads a delivery rests on, against fakes that carry the two Phase-1 providers' measured limits
//! (`tests/fakerpc`). The old reads (the ticket's state at the finalized block; one `eth_getLogs` over 8,000,000 blocks) are
//! refused by them, as the real providers refused them on 2026-09-25/26; the new ones (state at the fast pin, events paged to
//! finality from a stored cursor) are answered.

mod fakerpc;

use std::cell::RefCell;
use std::future::Future;
use std::pin::pin;
use std::rc::Rc;
use std::task::{Context, Poll, Waker};

use etch_autobuyer::evmread::{self, MAX_LOG_SPAN_BLOCKS};
use etch_autobuyer::pin::{Pin, BUCKET};
use etch_autobuyer::ticketlog::{self, TicketLogs, PAGES_PER_TICK};
use fakerpc::{Chain, FakeRpc, FIN_STEP, ORDOFI_LOG_SPAN, RH_STATE_BLOCKS};

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
/// The head measured at 03:08:28 UTC on 2026-09-26.
const HEAD: u64 = 72_751_086;

fn chain(head: u64) -> Rc<RefCell<Chain>> {
    Rc::new(RefCell::new(Chain { head, logs: vec![] }))
}

/// AUDIT-D H-1 regression: what the canister used to read, the providers refuse. The ticket's state at the finalized block is
/// past Robinhood's window, and one `eth_getLogs` over the 8M-block lookback is past ordofi's span.
#[test]
fn audit_d_the_old_finalized_reads_are_refused_by_providers_with_the_measured_limits() {
    let c = chain(HEAD);
    let mut ps = fakerpc::pair(&c);
    // the old pin: every provider's finalized block rounded down to the bucket, the lowest
    let fin = block_on(evmread::finality(&mut ps)).unwrap() / BUCKET * BUCKET;
    let lag = HEAD - fin;
    assert!(lag > RH_STATE_BLOCKS, "finality is {lag} behind, past Robinhood's {RH_STATE_BLOCKS}");
    let at = Pin { number: fin, hash: fakerpc::hash_of(fin), base_fee: 1 };
    let old_ticket = block_on(evmread::call(&mut ps, &W, &[0xc1, 0x29, 0xff, 0x32], &at)).unwrap_err();
    assert!(old_ticket.contains("Robinhood") && old_ticket.contains("historical state"), "{old_ticket}");
    // the old log query: [fin − 8,000,000, fin] in one request
    let topics = ticketlog::topics();
    let refused = block_on(ps[1].logs_direct(&W, &topics, fin - 8_000_000, fin)).unwrap_err();
    assert!(refused.contains("-32005"), "{refused}");
    // and evmread will not even ask for a span past its page
    let e = block_on(evmread::logs(&mut ps, &W, &topics, fin - 8_000_000, fin, 48_000)).unwrap_err();
    assert!(e.contains("refused to ask"), "{e}");
}

trait Direct {
    async fn logs_direct(&mut self, a: &[u8; 20], t: &serde_json::Value, from: u64, to: u64) -> Result<Vec<evmread::LogView>, String>;
}
impl Direct for FakeRpc {
    async fn logs_direct(&mut self, a: &[u8; 20], t: &serde_json::Value, from: u64, to: u64) -> Result<Vec<evmread::LogView>, String> {
        evmread::Rpc::logs(self, a, t, from, to, 48_000).await
    }
}

#[test]
fn the_fast_pin_is_inside_robinhoods_window_and_both_providers_serve_state_there() {
    let c = chain(HEAD);
    let mut ps = fakerpc::pair(&c);
    ps[1].lag = 340; // ordofi a few hundred behind, as seen once
    let p = block_on(evmread::fast_pin(&mut ps)).unwrap();
    assert!(HEAD - p.number <= 2 * BUCKET + 340 && HEAD - p.number < RH_STATE_BLOCKS, "{}", HEAD - p.number);
    assert_eq!(block_on(evmread::call(&mut ps, &W, &[1], &p)).unwrap(), vec![0u8; 32]);
    // the fake does enforce the window: the same read a bucket past it is refused
    let old = HEAD - RH_STATE_BLOCKS - BUCKET;
    let far = Pin { number: old, hash: fakerpc::hash_of(old), base_fee: 1 };
    assert!(block_on(evmread::call(&mut ps, &W, &[1], &far)).is_err());
}

#[test]
fn ticket_events_are_read_in_pages_under_ordofis_limit_from_a_stored_cursor_and_only_once_final() {
    let c = chain(HEAD);
    let mut ps = fakerpc::pair(&c);
    let start = HEAD - 100_000;
    // ticket 0: redeemed and resolved well before finality; ticket 1: redeemed and resolved just now (not final yet)
    c.borrow_mut().logs.extend([fakerpc::redeemed(&W, 0, DEST, start + 1_000), fakerpc::resolved(&W, 0, 7, start + 70_000),
        fakerpc::redeemed(&W, 1, DEST, HEAD - 100), fakerpc::resolved(&W, 1, 8, HEAD - 50)]);
    let mut idx = TicketLogs::default();
    // tick 1: eight pages of at most 8,000 blocks from `start`
    block_on(ticketlog::advance(&mut ps, &mut idx, &W, Some(start), None)).unwrap();
    assert_eq!(idx.cursor, Some(start - 1 + PAGES_PER_TICK as u64 * MAX_LOG_SPAN_BLOCKS));
    assert_eq!(idx.redeemed_of(0), vec![DEST.to_string()], "found in the first page");
    assert!(idx.resolved_of(0).is_empty(), "block start + 70,000 is in the next tick's pages");
    // tick 2: to finality, never past it
    block_on(ticketlog::advance(&mut ps, &mut idx, &W, Some(start), None)).unwrap();
    let fin = idx.finalized.unwrap();
    assert_eq!(idx.cursor, Some(fin));
    assert!(idx.has_both(0) && !idx.has_both(1), "ticket 1's events are above finality: not believed yet");
    for p in &ps {
        for (m, from, to) in p.asked.borrow().iter() {
            if m == "eth_getLogs" {
                assert!(to - from < ORDOFI_LOG_SPAN && *to <= fin, "{from}..={to}");
            }
        }
    }
    // the chain moves on ≈ 20 minutes: ticket 1's events become final and are read in one page
    c.borrow_mut().head += 12_000;
    block_on(ticketlog::advance(&mut ps, &mut idx, &W, Some(start), None)).unwrap();
    assert!(idx.has_both(1), "{:?}", idx);
    assert_eq!(idx.resolved_of(1), vec![8]);
}

#[test]
fn a_provider_a_finality_step_behind_is_fine_one_far_behind_is_refused_by_name_and_the_scan_says_so() {
    let c = chain(HEAD);
    let mut ps = fakerpc::pair(&c);
    ps[0].fin_steps_behind = 1; // for ~20 s after a step, as measured
    let fin = block_on(evmread::finality(&mut ps)).unwrap();
    assert_eq!(fin, (Chain::finalized_at(HEAD) - FIN_STEP) / BUCKET * BUCKET, "the lower one");
    ps[0].fin_steps_behind = 0;
    ps[1].fin_override = Some(HEAD - 900_000); // a broken node, or a liar, 900k blocks deep
    let mut idx = TicketLogs::default();
    let e = block_on(ticketlog::advance(&mut ps, &mut idx, &W, Some(HEAD - 60_000), None)).unwrap_err();
    assert!(e.contains("Ordofi") && e.contains("refused"), "{e}");
    assert_eq!((idx.failures, idx.last_error.clone()), (1, Some(e)));
    assert!(idx.status().contains("not being read"), "{}", idx.status());
}

#[test]
fn providers_that_disagree_fail_the_page_loudly_it_retries_smaller_and_recovers() {
    let c = chain(HEAD);
    let mut ps = fakerpc::pair(&c);
    let start = HEAD - 60_000;
    c.borrow_mut().logs.push(fakerpc::redeemed(&W, 0, DEST, start + 10));
    ps[1].extra_logs.push(fakerpc::redeemed(&W, 0, "bc1pforged", start + 20)); // ordofi alone says more
    let mut idx = TicketLogs::default();
    let e = block_on(ticketlog::advance(&mut ps, &mut idx, &W, Some(start), None)).unwrap_err();
    assert!(e.contains("disagree"), "{e}");
    assert!(idx.redeemed.is_empty(), "nothing is believed");
    assert_eq!(idx.cursor, Some(start - 1), "the cursor stays where the failed page began");
    assert_eq!(idx.span, MAX_LOG_SPAN_BLOCKS / 2, "the next try asks for half");
    assert_eq!(idx.failures, 1);
    ps[1].extra_logs.clear();
    block_on(ticketlog::advance(&mut ps, &mut idx, &W, Some(start), None)).unwrap();
    assert_eq!((idx.failures, idx.last_error.clone()), (0, None));
    assert_eq!(idx.redeemed_of(0), vec![DEST.to_string()]);
}

#[test]
fn after_an_idle_stretch_the_scan_starts_near_the_idle_pin_not_at_a_blind_lookback() {
    let c = chain(HEAD);
    let mut ps = fakerpc::pair(&c);
    let mut idx = TicketLogs { idle_through: Some(HEAD - 1_500), ..Default::default() };
    // a day later a ticket is redeemed; the scan starts at the idle pin less the re-scan margin, not at the first lookback
    c.borrow_mut().head += 864_000;
    let fin = Chain::finalized_at(HEAD + 864_000) / BUCKET * BUCKET;
    block_on(ticketlog::advance(&mut ps, &mut idx, &W, None, None)).unwrap();
    let first_from = ps[0].asked.borrow().iter().find(|(m, ..)| m == "eth_getLogs").map(|(_, f, _)| *f).unwrap();
    assert_eq!(first_from, HEAD - 1_500 - ticketlog::IDLE_RESCAN_BLOCKS + 1);
    assert!(idx.cursor.unwrap() < fin, "catching up, four pages a tick");
}
