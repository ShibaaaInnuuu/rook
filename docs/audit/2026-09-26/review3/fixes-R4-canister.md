# Fix round 4, area D: the buyer canister and the timelock, after the R3 check

Written 2026-09-26 on branch `r4-canister` (from `fixes-2026-09-26` at `4a215f4`), **before the code**. It answers the canister
items of `review3/R3-check.md` and `review3/parts/part-canister.md`: R3-C1 and R3-C2 (Low, recommended before merge), the N-2
"fixed, bounded" note, and the Infos R3-C3 to R3-C7. §8 ("As built") is added after the code, with the test counts and the
reproducible-build hashes. Nothing here is deployed, installed or signed.

The rules of `fixes-D-canister.md` §1 and `review2/fixes-R3-canister.md` still hold: nothing is weakened to buy availability (H1
pause, H2 caps, H3 limiter, H4 outbound verifier, H8 conflict-out, "save, then send"); stable state stays decodable (new fields
are `opt`, the golden fixture `saved-e04f49b.hex` must still decode); every outcall a stranger can cause is bounded per tick.

## 1. R3-C1 (Low): after a dense block the scanner grows back to full pages

**The failure (R3):** a page too big even at 400 kB is halved towards the dense range at the cheap cap, and `TicketLogs.hot`
remembers the end of that range. A success *inside* `hot` kept the span, so once the dense block itself was read (one block, at
a larger cap or by ticket) the rest of `hot` was walked **one block a page**, 8 pages a tick: a 500-event block at a page's start
delayed a ticket 7,990 blocks later by ≈ 1,000 ticks (≈ 6.9 days), and every delivery behind it refunded.

**Design.**

- **Multiplicative increase everywhere.** After every page that succeeds, the span doubles (up to `MAX_LOG_SPAN_BLOCKS`), inside
  `hot` too. Inside `hot` the next page still asks at the cheap cap (48 kB), so growing back into the dense range costs one
  cheap failed round (≈ 0.5 B cycles a provider), after which it halves again (additive-increase is not needed: the failure
  path already halves). From one block, 8,000 blocks are reached again in 13 pages, i.e. within two ticks.
- **`hot` is cleared once the dense block is read**: when a one-block page that needed a cap above the cheap one succeeds, and
  when a block read by ticket is finished. The cap then returns to the cheap one. A later dense block starts a new bisection
  (with its one 400 kB retry of the whole page, as now).
- **Cycles:** unchanged bounds. At most `MAX_TRIES_PER_TICK` = 12 outcall rounds and `PAGES_PER_TICK` = 8 pages a tick; the
  worst tick (every round at 2 MB) stays ≈ 0.5 T cycles on 13 nodes. Growing back adds at most one cheap failed round per
  doubling that overshoots, inside the same 12.

**Test:** R3's `audit3_scanner.rs` PoC becomes a regression: a 500-event block at a page's start and an honest ticket 4,000 and
7,990 blocks later, and a 2,400-event block (read by ticket) with the honest ticket 4,000 blocks later, each read within a few
ticks (was 502 / 1,000 / 505), every tick within the outcall bound.

## 2. R3-C2 (Low): one provider can no longer make the scanner skip a ticket's events

**The failure (R3):** `evmread::too_big` matched size-looking substrings anywhere in an error string, including a provider's own
JSON-RPC message (`"no result: {…response size limit of … exceeded}"`), and `evmread::logs` stopped at the first provider's
error. So one provider, by lying about one block, sent the scanner down the dense path; the one-ticket query "failed" 3 times;
the ticket went to `unreadable` and the cursor passed the block, although the other provider served the events. This bypassed
"providers must agree" for a chosen ticket.

**Design.** The size fallback may make the scanner *work harder*; only the providers' agreement may make it *skip*.

1. **A size error is the IC's own reject, and only that.** `IcRpc::logs` (canister.rs) recognises the IC's reject of an answer
   over the cap (`CallRejected` whose message *starts with* the IC's wording, "Http body exceeds size limit of" / "Header size
   exceeds specified response size limit") and returns it with a fixed prefix (`evmread::SIZE_REJECT`). `too_big` accepts
   only an error that *starts with* that prefix. A provider's own words reach the canister only inside `no result: …` or
   `unreadable: …`, never at the start, so no provider text can pass for a size error. (A provider that *pads* its answer past
   the cap does cause a real IC reject: point 2 handles that.)
2. **Every provider is asked, and the verdict is compared.** New `evmread::logs_sized` asks every provider even when one fails
   for size, and returns:
   - `Ok(logs)`: every provider answered, and the answers agree (as now);
   - `TooBig`: **every** provider's answer was over the cap;
   - `Split`: some answered and some were over the cap (named), which is not proof of size, only a reason to try a larger cap
     or a smaller query;
   - `Failed`: any other error (a provider down, the providers disagree): as now.
   `evmread::logs` keeps its signature (the `Split`/`TooBig` text as the error) for its other callers.
3. **The scanner.** `TooBig` and `Split` both let a page grow its cap or shrink its span, and let one block be read by ticket
   (that only costs outcalls, within the per-tick bound). But in the by-ticket read, a **one-ticket** query:
   - `TooBig` (every provider, 3 tries, the last at 2 MB): set aside in `unreadable`, as now, so two broken providers cannot
     stop the cursor for ever;
   - `Split`: **the scan stops there, loud** ("the providers do not agree that ticket T's events in block B are too big: nothing
     is skipped"), keeping its place (`TicketLogs.dense`), and tries again next tick. `failures`/`last_error` say so; the
     watcher's existing scanner alarm pages it. One lying provider can stall the scan (it always could, with any error), but
     can no longer make it skip anything.
4. **`unreadable` is retried, not dropped for good.** Each tick with an `unreadable` entry, before the pages, one entry is
   asked again (one round at the cheap cap: a single ticket's two events are ≈ 2 kB), first in the list, rotating on failure,
   within the tick's 12 rounds. Read (every provider agreeing): its events are taken in and the entry removed. An entry whose
   ticket the custodian no longer wants (its redemption finished) is dropped. Cost while an entry exists: ≈ 1.1 B cycles a tick
   on 13 nodes; nothing when there is none.

**Tests:** R3's PoC (`Censor`, a provider-text size error) becomes a regression: the cursor stops before the censored block,
nothing is `unreadable`, the error names the provider, and once the provider answers honestly the ticket's events are read. A
second censor that pads (a real IC-format reject from one provider only) stalls the same way. Every provider too big for one
ticket still sets it aside (the R2 regression), and a later honest answer brings it back through the retry. Unit tests for
`too_big` (prefix only) and `logs_sized`'s verdicts.

## 3. N-2 "fixed, bounded" (accepted residual)

Nothing changes: R3 re-ran R2's strongest mempool (no replacement of the pinned delivery ever accepted) and found 0 false
attestations, the refund refused at `deliverBy + 72 h`, and a LostRace only for a pin held **30 days** (`LATE_DELIVERY_WINDOW`),
which under TRUC needs a ≥ ~82 sat/vB market for 30 days and ≥ 75k sats of child fee at risk (Core's 14-day mempool expiry works
against it too). R3's `audit3_pin.rs` becomes a regression that pins that bound (and M-1 (c): a pause across the margin conflicts
out and attests only after the resume). The residual stays listed in §7.

## 4. The Infos

| ID | Decision |
| --- | --- |
| **R3-C3** the paused pass skips `assert_chain` | **Fixed.** `tick`'s paused branch runs `assert_chain()` (the RPC is chain 4663) before the restricted custodian pass; a failure skips that pass with a line, as the unpaused tick does. The Bitcoin checkpoint check (`ensure_btc_network`) already runs inside `custodynet::tick`. |
| **R3-C4** quarantine counted per choice, not per tick | **Fixed.** A pool entry's failed read counts at most once per `QUARANTINE_SPACING_SECS` = 3,600 s (`FeeEntry.last_failed_at`, `opt`), so quarantine needs three failing reads in three separate hours (≥ 2 h), never three choices in one tick. A good read still clears the count. Auto-restore of a quarantined entry is **not** added (listed): it stays the operator's, with the loud line. |
| **R3-C5** (a) an orphaned `running` entry blocks `start_target` for ever | **Fixed.** `Running.since_ns` (`opt`). An entry older than `RUNNING_ORPHAN_NS` = 24 h (an execute that trapped after an await; a real one takes minutes) is an orphan: `start_target` (a guardian) first clears orphans, each recorded as `Failed` ("orphaned: its execute never finished"), then checks as before. A state whose running entry predates `since_ns` counts it as an orphan. |
| **R3-C5** (b) a veto during `install_code` is recorded but not honoured | **Fixed.** `Running.committed` (`opt`): set when `execute` has passed its last check (after the stop, for an upgrade; at once for a `Controllers` change, whose single await has no later check). A veto or withdrawal of a committed proposal is **refused** ("too late: it is being installed"), with no `Vetoed`/`Cancelled` event; a roles change no longer marks it cancelled. So the record never says "vetoed" for something that then executes. |
| **R3-C6** `markUndeliverable` never re-priced; blocks `recordDelivery` | **Listed, not changed.** The same pattern as `recordDelivery` (signed once at 4 × the base fee, one EVM transaction at a time). The fix is a same-nonce replacement for both, with its own tests; left for a later round. The watcher alert on `attest` staying `Sent` for more than a day stands (fixes-R3-canister §9). |
| **R3-C7** conflicts escalate to 100k sats even when not racing | **Fixed (cheap part).** When our conflict is **in a mempool** at re-price time (it holds the fee input there; the gated wrapper waits for its confirmation before any refund), it is re-signed only to follow the market: when today's rate × its size exceeds its fee, to that (at least its fee plus the incremental relay fee). The ×1.5 escalation stays for a conflict that is **not** seen (something else holds the mempool: the pinned delivery, or a package that outbid ours). |

## 5. What the monitor and fixtures need

- `FeeEntry.last_failed_at` (`opt nat64`) in `custody_fee_pool()`; `etch_autobuyer.did` updated. An older decoder still reads.
- `TicketLogs` gains no field. `unreadable` entries can now disappear (re-read); a new stall text ("do not agree that … is too
  big") arrives through `failures`/`last_error`, which the scanner alarm already pages.
- Timelock: `Running` gains `since_ns` and `committed` inside the saved state only; `Status` and the `.did` are unchanged. An
  orphan cleared by `start_target` appears as a `Failed` event.
- `examples/watch_fixtures.rs` must compile with the new field; `monitor/fixtures/canister.json` regeneration is outside this
  change (as in round 3).

## 6. Order of work

1. This document, committed alone.
2. Code and tests: `evmread` (`SIZE_REJECT`, `logs_sized`), `ticketlog` (growth, `hot`, split handling, retry), `canister.rs`
   (size reject, paused `assert_chain`), `custody.rs` / `custodynet.rs` (quarantine spacing), `conflict.rs` (R3-C7), the
   timelock's `rules.rs` / `canister.rs` (R3-C5); the fakes' size error in the new format; R3's PoCs as regressions
   (`tests/review3.rs`).
3. `cargo test` for both crates (and the timelock `--features rehearsal`), the reproducible Docker build twice, §8.

## 7. Left open

- R3-C6 (same-nonce re-pricing of `recordDelivery` and `markUndeliverable`).
- Auto-restore of quarantined fee-pool entries (R3-C4's second half).
- N-2 bounded at 30 days (§3); the gated wrapper and this canister ship together.
- From round 3, still open: the replica / mainnet-fork run of the scanner against the real providers; TRUC relay on both
  explorers and a signet delivery; the monitor learning round 3's and this round's fields; `gen-delivery-fixture.mjs` saying
  `version: 3`; N-8, N-10, `MAX_FINALITY_LAG`, L-6.

## 8. As built (2026-09-26)

Code and tests: commit `cbcec1f` on `r4-canister`. Built as designed above; the differences, found while testing:

- **R3-C2, the retry of a set-aside ticket** runs even in a tick whose tickets were not walked (`wanted` unknown): nothing is
  dropped for being unwanted then, but the entry is still asked again; an answer every provider agrees on (even an empty one)
  settles it. A retry's failure is a line in the tick's report, never the tick's error.
- **R3-C2, `Split` in paging:** a page (or one block at a larger cap) that one provider answers and another says is too big
  reshapes the page exactly as `TooBig` does, down to by-ticket; the stall happens only at a one-ticket query. So one provider can
  make the scan pay the same bounded extra rounds as a real burst, and then stop it, loud; it can never make it pass a block.
- **R3-C7** was tested by pausing the buyer between the first send and the margin, so the delivery is not bumped towards its
  25,000-sat cap first (otherwise the conflict's feerate is already above the 100 sat/vB clamp and never follows the market).
- **R3-C5:** the commit of an upgrade is taken in the same state access as its last `still_wanted` check (no await between).
  `Status` and the timelock `.did` are unchanged.

**Measured (R3's PoC scenarios as regressions, `tests/review3.rs`):**

| Scenario | R3 | Now |
| --- | ---: | ---: |
| 500-event block at a page's start, honest ticket 4,000 blocks on | 502 ticks | **3 ticks** (31 `eth_getLogs` a provider) |
| the same, 7,990 blocks on | 1,000 ticks (≈ 6.9 days) | **3 ticks** |
| 2,400-event block (read by ticket), honest ticket 4,000 blocks on | 505 ticks | **6 ticks** |
| two 500-event blocks 20,000 blocks apart, 40 tickets between | — | to finality in 8 ticks, every event once, back to 8,000-block pages |
| one provider's text size error on ticket 5's block | skipped for good in 3 ticks | stalls before the block (200 ticks, `failures` 200), nothing set aside; read at once when the provider answers |
| one provider padding past every cap (a real IC reject) | (same class) | stalls at the one-ticket query, "the providers do not agree … nothing is skipped"; read when it answers |
| every provider too big for one ticket | set aside for good | set aside (cursor advances), asked again each tick at 48 kB, read once the providers answer |
| N-2 unbreakable pin (bound) | 30 days | unchanged: 4,500 replacements refused, 0 attestations, refund at deliverBy + 30 d, then LostRace |
| conflict in a mempool at today's rate, 10 h | ×1.5 every 2 h | unchanged bytes (2,231 sats); rate ×3 → one re-sign to 6,660 sats (today's rate) |

**Tests.** `cargo test` (toolchain 1.98.1): canister **489 pass, 0 fail** (R3's baseline 478; +9 `review3.rs`, +1 `evmread`
unit, +1 `canister` source test; `review2.rs`'s quarantine lines take times now, and the fakes' size error is in the IC format
behind `SIZE_REJECT`). The golden saved state (`saved-e04f49b.hex`) still decodes. Timelock **15 pass, 0 fail** (baseline 12;
+2 R3-C5 rule tests, +1 "a state saved by round 3 decodes"), the same with `--features rehearsal`. `examples/watch_fixtures.rs`
compiles with `FeeEntry.last_failed_at`. Not run: `cargo clippy` (the clippy component is not installed for 1.98.1),
`cargo audit`; the lockfiles did not change.

**Size:** the buyer's wasm is 5,202,104 bytes, **1,017,615 gzipped** (`gzip -9 -n`; the proposal limit is 2,000,000).

**Reproducible build** (`Dockerfile.build`, linux/amd64, the script's steps on a `git archive` export of `cbcec1f`), run twice,
the second with `docker build --no-cache`, **identical both times**:

```
dead93fa93c8a52f84090d7b44fbd6a83b845bbb5494c573fdad76d60f44136b  buyer.wasm
b23a190365d4c5b2bfc41bd9e9caf147bf2eba50ba3b3d477e2be372840edc41  timelock.wasm
2145099ea00214e8b521d456f6fc48145c7139f1ecd0c5643393c09c688e62f4  timelock-rehearsal.wasm
```

All three are new (round 3 recorded `36ae6e2…` / `92248ca…` / `742bf47…`): the buyer changed in the scanner, the outcall errors,
the paused tick, the fee pool and the conflict; the timelock in `Running`, `may_start` and `execute`. Nothing was installed.

**Still open:** §7, unchanged.
