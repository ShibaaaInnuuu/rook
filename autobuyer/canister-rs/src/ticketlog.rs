//! The wrapper's `Redeemed` and `Resolved` events, read at finality from a stored cursor (audit D, H-1;
//! docs/audit/2026-09-26/fixes-D-canister.md §2).
//!
//! H9 wanted a delivery's facts read where a reorganisation cannot change them. Reading the ticket's STATE at the finalized block
//! cannot work with the Phase-1 providers: Robinhood's RPC keeps ≈ 6,200 blocks of state and finality is 8,096–12,023 behind the
//! head. Logs need no state, so finality comes from the events instead: the burn's `Redeemed(ticketId, owner, round,
//! destination)` and the draw's `Resolved(ticketId, acquisitionId, deliverBy)`, each at or below the finalized block. They are
//! read in pages of at most `evmread::MAX_LOG_SPAN_BLOCKS` (ordofi refuses more than 10,000), from where the last page ended,
//! a few pages a tick, never past finality. What is found is kept per ticket until its redemption finishes.
//!
//! Pure but for [`advance`], which reads through `evmread::Rpc` (the IC's outcalls, or the tests' fakes with the providers' real
//! limits).

use serde_json::Value;

use crate::evmread::{self, LogView, LogsError, Rpc, MAX_LOG_SPAN_BLOCKS};

/// At most this many pages a tick while behind: ≈ 5,940 new blocks a 10-minute tick, so one page keeps up and eight catch up at
/// ≈ 58k blocks a tick (audit R2, N-11; was 4).
pub const PAGES_PER_TICK: usize = 8;
/// At most this many outcall rounds (one query to every provider) a tick, whatever the chain holds (audit R2, N-1).
pub const MAX_TRIES_PER_TICK: usize = 12;
/// A page may shrink to one block (audit R2, N-1; was 125, below which a dense window stalled the scan for ever).
pub const MIN_SPAN_BLOCKS: u64 = 1;
/// The response caps a page is asked with, in order: the normal one (≈ 50 `Redeemed` logs), then larger, up to the IC's limit.
/// A cap is paid for whether it is used or not, so after each page that succeeds the next one asks one step lower.
pub const LOG_CAPS: [u64; 3] = [48_000, 400_000, evmread::MAX_RESPONSE_BYTES];
/// A block too big to read whole even at the largest cap is read by ticket: this many tickets a query ...
pub const TICKETS_PER_QUERY: usize = 64;
/// ... at this cap (≤ 128 logs of ≈ 900 bytes).
pub const TICKET_QUERY_CAP: u64 = 400_000;
/// A one-ticket query that EVERY provider answers too big after this many tries (something the wrapper cannot emit) is set aside,
/// loud, and the scan moves on: the cursor always advances. Only every provider's size error counts (audit R3, R3-C2); one
/// provider's alone stops the scan there instead.
pub const UNREADABLE_AFTER: u32 = 3;
/// A set-aside ticket is asked again once a tick at this cap (audit R3, R3-C2): its two events are ≈ 2 kB.
pub const UNREADABLE_RETRY_CAP: u64 = LOG_CAPS[0];
/// When a scan first runs and it is not idle, and the install named no start: this far below finality (≈ 5.6 h).
pub const FIRST_LOOKBACK_BLOCKS: u64 = 200_000;
/// The idle skip re-reads this many blocks below the idle pin, for a reorganisation below the fast pin.
pub const IDLE_RESCAN_BLOCKS: u64 = 20_000;

/// `Resolved(uint256 indexed ticketId, uint256 indexed acquisitionId, uint64 deliverBy)`.
pub fn resolved_topic() -> [u8; 32] {
    crate::evmtx::keccak(b"Resolved(uint256,uint256,uint64)")
}

/// What the scanner keeps (inside `redeem::Custody`, so it survives upgrades).
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize, candid::CandidType)]
pub struct TicketLogs {
    /// The last block read, inclusive. `None`: not started.
    pub cursor: Option<u64>,
    /// Blocks per page now; `0` means [`MAX_LOG_SPAN_BLOCKS`].
    pub span: u64,
    /// A fast pin at which every ticket was accounted for: nothing relevant lies below it (less [`IDLE_RESCAN_BLOCKS`]).
    pub idle_through: Option<u64>,
    /// `Redeemed`, per ticket: the destination. More than one distinct per ticket is kept, and refused at use.
    pub redeemed: Vec<(u64, String)>,
    /// `Resolved`, per ticket: the acquisition id (a deposit's carried as bit 127).
    pub resolved: Vec<(u64, u128)>,
    /// The finalized block at the last read.
    pub finalized: Option<u64>,
    /// Scans in a row that failed, and the last reason: the watcher's alarm.
    pub failures: u32,
    pub last_error: Option<String>,
    /// The response cap the next page asks with; `None` is the first of [`LOG_CAPS`] (audit R2, N-1).
    #[serde(default)]
    pub cap: Option<u64>,
    /// The end of a range known to be too big for a page: until the cursor passes it, or its dense block is read, pages inside it
    /// ask at the cheap cap and a failure halves rather than retrying at a larger cap (audit R2, N-1; audit R3, R3-C1: the span
    /// still doubles after each success, so the scan grows back to full pages).
    #[serde(default)]
    pub hot: Option<u64>,
    /// A block too big to read whole, being read by ticket.
    #[serde(default)]
    pub dense: Option<Dense>,
    /// How many blocks were read by ticket so far (for the watcher: a dense block is somebody's burst).
    #[serde(default)]
    pub dense_blocks: Option<u64>,
    /// `(block, ticket)`: a ticket whose events in that block every provider answered too big even alone. Loud; asked again once a
    /// tick (audit R3, R3-C2), dropped once read or no longer wanted.
    #[serde(default)]
    pub unreadable: Option<Vec<(u64, u64)>>,
}

/// The block after the cursor, too big to read whole, read by ticket (audit R2, N-1). Saved, so a block that needs more queries
/// than one tick allows is finished over the next ticks.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize, candid::CandidType)]
pub struct Dense {
    pub block: u64,
    /// Every wanted ticket up to this one is read.
    pub after: Option<u64>,
    /// Times the chunk was halved: a query asks for `TICKETS_PER_QUERY >> shrink` tickets.
    pub shrink: u32,
    /// Tries in a row that a one-ticket query was too big.
    pub fails: u32,
}

impl TicketLogs {
    fn span_now(&self) -> u64 {
        if self.span == 0 { MAX_LOG_SPAN_BLOCKS } else { self.span.clamp(MIN_SPAN_BLOCKS, MAX_LOG_SPAN_BLOCKS) }
    }
    fn cap_now(&self) -> u64 {
        self.cap.filter(|c| LOG_CAPS.contains(c)).unwrap_or(LOG_CAPS[0])
    }
    /// The finalized `Redeemed` destinations for `ticket` (distinct).
    pub fn redeemed_of(&self, ticket: u64) -> Vec<String> {
        self.redeemed.iter().filter(|(t, _)| *t == ticket).map(|(_, d)| d.clone()).collect()
    }
    /// The finalized `Resolved` acquisition ids for `ticket` (distinct).
    pub fn resolved_of(&self, ticket: u64) -> Vec<u128> {
        self.resolved.iter().filter(|(t, _)| *t == ticket).map(|(_, a)| *a).collect()
    }
    /// Both of `ticket`'s events are in.
    pub fn has_both(&self, ticket: u64) -> bool {
        !self.redeemed_of(ticket).is_empty() && !self.resolved_of(ticket).is_empty()
    }
    /// Forget what a finished redemption needed: the state stays bounded by the unfinished ones.
    pub fn forget(&mut self, ticket: u64) {
        self.redeemed.retain(|(t, _)| *t != ticket);
        self.resolved.retain(|(t, _)| *t != ticket);
    }
    /// For a waiting redemption's line: what the scanner is doing.
    pub fn status(&self) -> String {
        match (&self.last_error, self.cursor, self.finalized) {
            (Some(e), _, _) => format!("the ticket logs are not being read ({} tries): {e}", self.failures),
            (None, Some(c), Some(f)) if c < f => format!("the ticket logs are read to block {c}, {} behind finality", f - c),
            (None, Some(c), _) => format!("the ticket logs are read to block {c} (finality)"),
            (None, None, _) => "the ticket logs are not read yet".into(),
        }
    }
    /// For the log: what the dense-block fallback has done.
    pub fn dense_status(&self) -> Option<String> {
        let d = self.dense.as_ref().map(|d| format!("block {} is read by ticket (through ticket {:?})", d.block, d.after));
        let u = self.unreadable.as_ref().filter(|u| !u.is_empty()).map(|u| format!("UNREADABLE ticket events (block, ticket): {u:?}"));
        match (d, u) {
            (None, None) => None,
            (a, b) => Some([a, b].into_iter().flatten().collect::<Vec<_>>().join("; ")),
        }
    }
    /// Take in one page's logs (already bounded to the range and the wrapper by `evmread::logs`).
    pub fn ingest(&mut self, logs: &[LogView]) -> Result<usize, String> {
        let redeemed = format!("0x{}", hex::encode(crate::redeem::REDEEMED_TOPIC));
        let resolved = format!("0x{}", hex::encode(resolved_topic()));
        let mut n = 0;
        for l in logs {
            let word = |i: usize| -> Result<Vec<u8>, String> {
                hex::decode(l.topics.get(i).ok_or("a log with too few topics")?.trim_start_matches("0x")).map_err(|_| "a topic that is not hex".to_string())
            };
            let Some(t0) = l.topics.first() else { continue };
            if *t0 == redeemed {
                let ticket = crate::abi::uint_at(&word(1)?, 0, 0, 64)? as u64;
                let data = hex::decode(l.data.trim_start_matches("0x")).map_err(|_| "a log whose data is not hex")?;
                let dest = String::from_utf8(crate::abi::bytes_at(&data, 0, 1)?).map_err(|_| "a Redeemed destination that is not text")?;
                if !self.redeemed.contains(&(ticket, dest.clone())) {
                    self.redeemed.push((ticket, dest));
                }
                n += 1;
            } else if *t0 == resolved {
                let ticket = crate::abi::uint_at(&word(1)?, 0, 0, 64)? as u64;
                let acq = crate::deposits::frog_id_of_word(&word(2)?)?;
                if !self.resolved.contains(&(ticket, acq)) {
                    self.resolved.push((ticket, acq));
                }
                n += 1;
            }
        }
        Ok(n)
    }
}

/// The topics filter: `topic0 ∈ {Redeemed, Resolved}`.
pub fn topics() -> Value {
    serde_json::json!([[format!("0x{}", hex::encode(crate::redeem::REDEEMED_TOPIC)), format!("0x{}", hex::encode(resolved_topic()))]])
}

/// The topics filter for one block read by ticket: `topic0 ∈ {Redeemed, Resolved}` and `topic1` (the ticket id, indexed first in
/// both events) one of `tickets`.
pub fn ticket_topics(tickets: &[u64]) -> Value {
    let t0 = topics();
    let ids: Vec<String> = tickets.iter().map(|t| format!("0x{}", hex::encode(crate::abi::uint_word(*t as u128)))).collect();
    serde_json::json!([t0[0].clone(), ids])
}

/// Read the wrapper's events forward from the cursor to finality: at most [`PAGES_PER_TICK`] pages and [`MAX_TRIES_PER_TICK`]
/// outcall rounds. `start`: where to begin when the scanner has never run (the install's `wrapper_from_block`). `wanted`: the
/// tickets the custodian can still need (redemptions lacking events, and every ticket not yet walked up to `ticketCount` at the
/// tick's fast pin), used only to read a block too big to read whole; `None` when the walk failed this tick. The outcome is
/// recorded in `idx` either way; an `Err` is also the line for the log.
///
/// A page too big for its response cap (audit R2, N-1) is retried in the same tick: once at the next cap (a burst of a few hundred
/// events), then halved towards the dense range at the cheap cap down to one block, that block at the larger caps up to the IC's
/// limit, and finally that one block by ticket. So no burst of events stops the cursor. After every success the span doubles, so
/// the pages grow back once the dense block is read (audit R3, R3-C1).
///
/// A size verdict is believed only when every provider gives it (audit R3, R3-C2): one provider's size error alone may make the
/// scan work harder (a larger cap, a smaller page, a block by ticket) but never skip anything; for a single ticket it stops the scan
/// there, loud, until the providers agree.
pub async fn advance<R: Rpc>(ps: &mut [R], idx: &mut TicketLogs, wrapper: &[u8; 20], start: Option<u64>, wanted: Option<&[u64]>)
    -> Result<String, String> {
    let r = advance_inner(ps, idx, wrapper, start, wanted).await;
    match &r {
        Ok(_) => {
            idx.failures = 0;
            idx.last_error = None;
        }
        Err(e) => {
            idx.failures = idx.failures.saturating_add(1);
            idx.last_error = Some(e.clone());
        }
    }
    r
}

async fn advance_inner<R: Rpc>(ps: &mut [R], idx: &mut TicketLogs, wrapper: &[u8; 20], start: Option<u64>, wanted: Option<&[u64]>)
    -> Result<String, String> {
    let fin = evmread::finality(ps).await?;
    idx.finalized = Some(fin);
    // an idle pin: every ticket was accounted for there, so nothing relevant lies below it (less the re-scan margin)
    let idle = idx.idle_through.take().map(|i| i.saturating_sub(IDLE_RESCAN_BLOCKS).min(fin));
    let cursor = match (idx.cursor, idle) {
        (Some(c), Some(i)) => c.max(i),
        (Some(c), None) => c,
        (None, Some(i)) => i,
        (None, None) => start.map_or(fin.saturating_sub(FIRST_LOOKBACK_BLOCKS), |s| s.saturating_sub(1)),
    };
    let mut cursor = cursor;
    idx.cursor = Some(cursor);
    let topics = topics();
    let (mut pages, mut tries, mut found, mut resized) = (0, 0, 0, 0);
    // audit R3, R3-C2: a set-aside ticket is asked again (one round, the cheap cap), and forgotten once no longer wanted
    let retried = retry_unreadable(ps, idx, wrapper, wanted, &mut tries).await?;
    while cursor < fin && pages < PAGES_PER_TICK && tries < MAX_TRIES_PER_TICK {
        let from = cursor + 1;
        // a dense marker left behind by a cursor that moved (an idle jump) is stale
        if idx.dense.as_ref().is_some_and(|d| d.block != from) {
            idx.dense = None;
        }
        if idx.dense.is_some() {
            let Some(w) = wanted else {
                return Err(format!("block {from} is too big to read whole and is read by ticket; the tickets were not walked this \
                    tick, so it waits"));
            };
            if !by_ticket(ps, idx, wrapper, from, w, &mut tries).await? {
                break; // this tick's budget is spent; the next goes on where it stopped
            }
            cursor = from;
            idx.cursor = Some(cursor);
            idx.dense = None;
            idx.dense_blocks = Some(idx.dense_blocks.unwrap_or(0) + 1);
            // audit R3, R3-C1: the dense block is read; what follows grows back from one block at the cheap cap
            idx.hot = None;
            idx.cap = None;
            idx.span = (idx.span_now() * 2).min(MAX_LOG_SPAN_BLOCKS);
            pages += 1;
            continue;
        }
        let span = idx.span_now();
        let cap = idx.cap_now();
        let to = fin.min(from + span - 1);
        tries += 1;
        match evmread::logs_sized(ps, wrapper, &topics, from, to, cap).await {
            Ok(logs) => {
                found += idx.ingest(&logs)?;
                cursor = to;
                idx.cursor = Some(cursor);
                // one block that needed more than the cheap cap: the dense block itself is read (audit R3, R3-C1)
                let dense_read = span <= MIN_SPAN_BLOCKS && cap > LOG_CAPS[0];
                let inside_hot = idx.hot.is_some_and(|h| to < h) && !dense_read;
                if inside_hot || dense_read {
                    // near a range known to be too big: the cheap cap (a failure costs ≈ 0.5 B cycles, not 21 B)
                    idx.cap = None;
                } else {
                    // the cap is paid for: one step down after a success
                    idx.cap = LOG_CAPS.iter().rev().copied().find(|c| *c < cap).filter(|c| *c != LOG_CAPS[0]);
                }
                if !inside_hot {
                    idx.hot = None;
                }
                // audit R3, R3-C1: the span doubles after every success, inside `hot` too, so the scan does not crawl one block a
                // page through the rest of a bisected range; overshooting into the dense range costs one cheap round and halves
                idx.span = (span * 2).min(MAX_LOG_SPAN_BLOCKS);
                pages += 1;
            }
            Err(LogsError::TooBig | LogsError::Split(_)) => {
                // a split verdict (one provider over the cap, another not) only makes the scan work harder; it never skips
                // anything: see `by_ticket` (audit R3, R3-C2)
                resized += 1;
                let bisecting = idx.hot.is_some_and(|h| from <= h);
                idx.hot = Some(idx.hot.filter(|_| bisecting).map_or(to, |h| h.max(to)));
                if span <= MIN_SPAN_BLOCKS {
                    // one block: a larger cap, then by ticket
                    match LOG_CAPS.iter().copied().find(|c| *c > cap) {
                        Some(next) => idx.cap = Some(next),
                        None => idx.dense = Some(Dense { block: from, ..Default::default() }),
                    }
                } else if !bisecting && cap == LOG_CAPS[0] {
                    // a burst a larger cap holds (≈ 440 logs at 400 kB): one retry of the same page
                    idx.cap = Some(LOG_CAPS[1]);
                } else {
                    // halve towards it at the cheap cap: an empty half costs ≈ 0.5 B cycles, not 21 B
                    idx.span = (span / 2).max(MIN_SPAN_BLOCKS);
                    idx.cap = None;
                }
            }
            Err(LogsError::Failed(e)) => {
                idx.span = (span / 2).max(MIN_SPAN_BLOCKS);
                return Err(format!("blocks {from}..={to}: {e} (next try: {} blocks a page)", idx.span_now()));
            }
        }
    }
    let dense = idx.dense_status().map(|d| format!("; {d}")).unwrap_or_default();
    let resized = if resized > 0 { format!(", {resized} page(s) too big and retried") } else { String::new() };
    let retried = retried.map(|r| format!("; {r}")).unwrap_or_default();
    Ok(format!("ticket logs read to block {cursor} ({pages} page(s), {found} event(s){resized}); finality {fin}{dense}{retried}"))
}

/// Audit R3, R3-C2: the first set-aside `(block, ticket)` is asked again, one round at [`UNREADABLE_RETRY_CAP`]. Read, with every
/// provider agreeing: its events are taken in and the entry removed. Otherwise it goes to the back of the list. Entries whose
/// ticket the custodian no longer wants (known only when the tickets were walked this tick) are dropped first. A retry's failure
/// is reported, never the tick's error: the scan goes on.
async fn retry_unreadable<R: Rpc>(ps: &mut [R], idx: &mut TicketLogs, wrapper: &[u8; 20], wanted: Option<&[u64]>, tries: &mut usize)
    -> Result<Option<String>, String> {
    if let (Some(u), Some(w)) = (idx.unreadable.as_mut(), wanted) {
        u.retain(|(_, t)| w.contains(t));
    }
    if idx.unreadable.as_ref().is_some_and(|u| u.is_empty()) {
        idx.unreadable = None;
    }
    let Some(&(block, ticket)) = idx.unreadable.as_ref().and_then(|u| u.first()) else { return Ok(None) };
    *tries += 1;
    let r = evmread::logs_sized(ps, wrapper, &ticket_topics(&[ticket]), block, block, UNREADABLE_RETRY_CAP).await;
    let u = idx.unreadable.get_or_insert_with(Vec::new);
    u.remove(0);
    let line = match r {
        Ok(logs) => {
            idx.ingest(&logs)?;
            format!("set-aside ticket {ticket} in block {block} read again ({} event(s))", logs.len())
        }
        Err(e) => {
            u.push((block, ticket));
            format!("set-aside ticket {ticket} in block {block} still unread: {e}")
        }
    };
    if idx.unreadable.as_ref().is_some_and(|u| u.is_empty()) {
        idx.unreadable = None;
    }
    Ok(Some(line))
}

/// Read one block's events by ticket, from where the saved progress stopped. `Ok(true)`: every wanted ticket is read; `Ok(false)`:
/// the tick's budget ran out first. A query too big is retried at half the tickets; a one-ticket query that every provider answers
/// too big [`UNREADABLE_AFTER`] times is set aside in `unreadable`, so the block is finished unless a provider disagrees (audit R3,
/// R3-C2), which stops the scan here, loud.
async fn by_ticket<R: Rpc>(ps: &mut [R], idx: &mut TicketLogs, wrapper: &[u8; 20], block: u64, wanted: &[u64], tries: &mut usize)
    -> Result<bool, String> {
    let mut ws = wanted.to_vec();
    ws.sort_unstable();
    ws.dedup();
    loop {
        let d = idx.dense.clone().ok_or("no dense block")?;
        let rest: Vec<u64> = ws.iter().copied().filter(|t| d.after.is_none_or(|a| *t > a)).collect();
        if rest.is_empty() {
            return Ok(true);
        }
        if *tries >= MAX_TRIES_PER_TICK {
            return Ok(false);
        }
        let n = (TICKETS_PER_QUERY >> d.shrink.min(16)).max(1).min(rest.len());
        let chunk = &rest[..n];
        let cap = if n == 1 && d.fails > 0 { evmread::MAX_RESPONSE_BYTES } else { TICKET_QUERY_CAP };
        *tries += 1;
        let r = evmread::logs_sized(ps, wrapper, &ticket_topics(chunk), block, block, cap).await;
        let d = idx.dense.as_mut().ok_or("no dense block")?;
        match r {
            Ok(logs) => {
                let last = *chunk.last().expect("not empty");
                d.after = Some(last);
                d.fails = 0;
                d.shrink = d.shrink.saturating_sub(1);
                idx.ingest(&logs)?;
            }
            Err(LogsError::TooBig | LogsError::Split(_)) if n > 1 => d.shrink += 1,
            // every provider: too big even alone, so set aside after UNREADABLE_AFTER tries (two broken providers cannot stop the
            // cursor for ever; the entry is asked again every tick)
            Err(LogsError::TooBig) => {
                d.fails += 1;
                if d.fails >= UNREADABLE_AFTER {
                    d.after = Some(chunk[0]);
                    d.fails = 0;
                    idx.unreadable.get_or_insert_with(Vec::new).push((block, chunk[0]));
                }
            }
            // audit R3, R3-C2: one provider says too big and another answered. That is a disagreement, not a size: nothing is
            // skipped, the scan stops here (its place kept) and asks again next tick
            Err(LogsError::Split(s)) => {
                return Err(format!("block {block}, read by ticket: the providers do not agree that ticket {}'s events are too big \
                    ({s}): nothing is skipped; the scan waits here", chunk[0]));
            }
            Err(LogsError::Failed(e)) => return Err(format!("block {block}, read by ticket: {e}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn word(v: u128) -> String {
        format!("0x{}", hex::encode(crate::abi::uint_word(v)))
    }

    #[test]
    fn the_resolved_topic_is_the_events_keccak_and_events_are_kept_per_ticket_once() {
        assert_eq!(hex::encode(resolved_topic()), hex::encode(crate::evmtx::keccak(b"Resolved(uint256,uint256,uint64)")));
        // `cast abi-encode "f(uint64,string)" 3 bc1p5cyx…`
        let data = "0x00000000000000000000000000000000000000000000000000000000000000030000000000000000000000000000000000000000000000000000000000000040000000000000000000000000000000000000000000000000000000000000003e62633170356379786e75786d65757775766b7766656d39366c717a737a6430326e367864636a727332306361633679716a6a7775647078716b65647263720000";
        let red = LogView { address: "0xcc".into(), topics: vec![format!("0x{}", hex::encode(crate::redeem::REDEEMED_TOPIC)), word(4), word(9)],
            data: data.into(), block: 10, removed: false };
        let mut dep = crate::abi::uint_word(7);
        dep[0] |= 0x80;
        let res = LogView { address: "0xcc".into(), topics: vec![format!("0x{}", hex::encode(resolved_topic())), word(4),
            format!("0x{}", hex::encode(dep))], data: "0x".into(), block: 11, removed: false };
        let mut idx = TicketLogs::default();
        assert_eq!(idx.ingest(&[red.clone(), res.clone(), red, res]).unwrap(), 4);
        assert_eq!(idx.redeemed_of(4), vec!["bc1p5cyxnuxmeuwuvkwfem96lqzszd02n6xdcjrs20cac6yqjjwudpxqkedrcr".to_string()], "once");
        assert_eq!(idx.resolved_of(4), vec![crate::deposits::DEPOSIT_BIT | 7], "a deposit frog's id carried as bit 127");
        assert!(idx.has_both(4) && !idx.has_both(5));
        idx.forget(4);
        assert!(idx.redeemed.is_empty() && idx.resolved.is_empty());
    }
}
