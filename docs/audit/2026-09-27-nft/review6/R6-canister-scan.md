# Round 6, part B: the sale scan after B5-M-1 / B5-M-2 / B5-L-1

Reviewer: independent AI re-review, 2026-09-28. Worktree `/Users/jlohayon/Documents/Rook-nft`, branch `nft-side` at `e04fa16`
(delta `8ef82d6..HEAD`, canister commit `e811911`).

**Scope:**
- `ethflow.rs`: `sale_scan` (`:1135-1430`), `ScanPass`, `Got`, `ScanErr` / `scan_error` (`:1460-1485`), `remember_big`, `defer`
  (`:1497`), `block_sales` (`:1511`), `scan_read` (`:1547`); `SaleScan.bad` / `skipped` / `done_big` / `stuck` / `spent`;
- `ethnet.rs`: `logs_classes`, `eth_transfer_moves`, `eth_sale_moves_range` (`:425-450`), `logs_read_worst`, `receipt_bytes`, the
  transforms `eth_transfer_logs` / `eth_sale_logs` / `eth_receipt_logs` (`:37-58`);
- `ethnft.rs` `SALE_*` (`:111-156`), `transfer_logs_body`, `sale_logs_body`; `pin.rs` `json_rpc_norm_body` (`:196`);
- `canister.rs` `rpc_once_with` (`:770-788`).

Toolchain 1.98.1.

**Nothing in the worktree was modified except this report and `review6/poc/B/`. Nothing was committed, pushed, deployed or
signed.**

- **Where the PoCs ran:** a private copy of `autobuyer/canister-rs` at `/tmp/rook-r6-B`. `diff -rq` showed it identical to the
  worktree before anything was added, and it was removed afterwards.
- **The harness:** `tests/audit_nft_r5.rs`'s fake world, copied whole up to its first test, plus five switches
  (`poc/B/r6_b.harness.diff`):
  - `log_depth` / `depth_raw`: a provider's log history depth, and whether its refusal is seen raw or through the canister's
    transform;
  - `receipt_disagree`: one provider answers null for a receipt;
  - `busy_pct`: the -32005 rate limit, drawn per marketplace query;
  - `seaport_range_cap`: Seaport's global `OrderFulfilled` volume, applied before the IC cap.
- **Read-only probes:** the network use was read-only `eth_getLogs` probes of `ethereum-rpc.publicnode.com` and `xrpc.cl/eth`
  (`poc/B/providers.probe.txt`).

Severity scale and threat model: `../README.md`'s. A passing PoC shows the weakness is present.

## Verdict

- **B5-M-1 is closed as reported.** No walk is held by an oversized block or a burst of them. The first read anchors even when it
  is too big. The fixer's scenarios hold, and I could not find a new starvation of the forward walk or the first post by an
  attacker.
- **B5-M-2 is closed in effect, but not by the mechanism the fix describes.** The canister's `eth_getLogs` transforms drop the
  provider's `message` and keep only its `code`. So:
  - `scan_error`'s word lists never see a provider's words. Only `-32005` and the IC's own reject classify.
  - `Deep` can never fire in production.
  - The fixer's shape test feeds raw messages the canister never receives (B6-I-1).
  - A result cap or range refusal under any other code is still deferred, but only by the "6 in a row at one start" catch-all:
    about an hour a range.
- **New Medium, B6-M-1 (conditional on the canister's provider pair):** after any gap in the scan longer than the shallow
  provider's log depth, the forward walk freezes until the window resets.
  - The trigger is publicnode's ~10,000 blocks, about 33 h: the runner down, the cycles under the evidence floor, and the like.
  - No newer sale is read for about 5.3 days (PoC: first newer sale at pass 385–395 of 504 after a 12,000-block gap; the control
    reads it at pass 1).
  - The same pair's receipt depth (publicnode answers `null` 20–30 k blocks back) adds a head-of-line block (B6-L-1).
- **The transient question is live (B6-L-2).** xrpc.cl's rate limit is `-32005` (HTTP 429). It is classified `TooBig`. At a 50 %
  busy rate, 20 of 432 due honest sales in a week (4.6 %) were lost for good and 36 more were still deferred. `SaleScan.bad` grew to
  529 ranges: `SALE_BAD_MAX` is not a bound.
- **B5-L-1 is closed.** Every log read is started only when its worst case fits the pass, receipts have their own bound, and the day
  has one. The ~10.5 T cycles a collection a day (~52 T for five, about $70 a day) is an attacker-imposed ceiling, not the honest
  cost. It is acceptable (see below).
- **B5-L-2 stays partly fixed.** The partial-Ok marketplace read also turns a transient Seaport failure into a permanent miss
  (in B6-L-2).
- **B4-L-2's edge is closed** (`done_big`).

## Counts (part B, open)

| Critical | High | Medium | Low | Info |
| ---: | ---: | ---: | ---: | ---: |
| 0 | 0 | 1 (B6-M-1, provider-conditional) | 2 (B6-L-1, B6-L-2) + B5-L-2 partial, B3-L-1 carried | 3 |

## B6-M-1 (Medium, conditional on the canister's pair): a scan gap past a provider's log depth freezes the forward walk for ~5 days

**Code:**
- the forward walk `ethflow.rs:1186-1206`: its start is always `sc.up + 1`, and a failure only shrinks the chunk or ends the pass;
- the only way out is the window reset at `:1145` (`sc.up < bottom`, 50,400 blocks after `sc.up`);
- `scan_error` `:1471-1485`; the transforms `ethnet.rs:49-58` → `ethnft::transfer_logs_body` → `pin::json_rpc_norm_body`
  (`pin.rs:214-219`: `{"error":{"code":…,"reverted":…}}`, message dropped); `canister.rs:786`.

**What happens:**
- **The trigger.** `ethereum-rpc.publicnode.com` refuses `eth_getLogs` older than about 10,000 blocks
  (`-32602 "Archive requests require a personal token"`; the fixer's and the lead's probes agree). It is the second provider of the
  pair the dry run installs (`scripts/golive-dryrun.sh:266`) and that `fixes-round5.md` §3 calls "the canister's keyless pair".
- **The freeze.**
  - After any stretch without a scan pass longer than that depth (about 33 h), `sc.up + 1` is deeper than publicnode serves.
  - Every forward read fails. As the canister sees it (`no result: {"code":-32602,"reverted":false}`), that is `Other`, not
    `Deep`.
  - The walk fails twice a pass. Every sixth failure at one start defers 25–50 blocks. Meanwhile the head moves 100 blocks a pass.
  - The walk never catches up. It waits for the window reset, 50,400 blocks after its last read block.
- **With the words kept.** If the words were kept (`Deep`), the result is the same: a Deep failure in the forward walk only
  counts and retries.
- **Such a gap needs no attacker:**
  - the runner (whose floor hand-ins are the passes) down for a day and a half;
  - the canister under `EVIDENCE_CYCLES_FLOOR` (8 T) until topped up;
  - a pause of the evidence path.

**Effect:**
- **The sales stop.** For about 5.3 days after such a gap, the canister counts no new sale for that collection.
- **The posts.** Posts continue from the listings, but a rise is HELD (not confirmed by counted sales). As the old sales leave the
  7-day window the counted sample shrinks. A collection with no first post yet gets none.
- **The deposit gate.** The gate's p25 is computed from stale sales. It is still capped by the live listing floor.
- **No loss of funds.** Nothing is signed from it. The integrity of what is counted holds. This is the price feed's availability for
  most of a week, which the design treats as a Medium-class liveness failure (as B5-M-1 was).

**Related, same root: the receipt depth.** The lead's probe shows publicnode answering `result: null` for receipts 20–30 k blocks
back (tx-by-hash still served). A null from one provider and a receipt from the other is a disagreement, so `again` (`:1385`,
`:1391-1393`) holds the move forever. See B6-L-1 for what that does to the queue. A pair that is deep for logs but prunes receipts
(the natural "fix" of swapping only the log side) makes it strike at launch: the window read finds moves 20–50 k blocks back whose
receipts can never be read.

**PoC:** `poc/B/r6_b.rs.snippet` `r6_b_m1_a_…`, output in `r6_b.output.txt`.

| Scenario (12,000-block gap, then a week of passes 100 blocks apart, a sale every 100 blocks) | First newer sale read | After a day | Behind the head after a day | After a week |
| --- | --- | --- | --- | --- |
| Deep providers (control) | pass 1 | 72 / 72 | 0 | all |
| publicnode depth, the refusal as the canister sees it (`Other`) | pass **395** | **0** | 19,200 | 207 / 504 |
| publicnode depth, raw words (`Deep`) | pass **385** | **0** | 19,200 | 214 / 504 |

`r6_b_m1_b_…` runs the measured error bodies through the real transforms: `Archive…` → `Other`; the result cap and the rate limit
(both `-32005`) → `TooBig`; "Log response size exceeded" (-32602), "block range is too wide" and "query timeout exceeded"
(-32000), and -32701 → `Other`.

**Fix:**
1. **Configuration (launch blocker, with part-3's provider recommendation).** The canister's two Ethereum providers must each serve
   `eth_getLogs` AND `eth_getTransactionReceipt` (a receipt, not `null`) over the whole 7-day window plus a margin: at least 60,000
   blocks. Verify it with a read-only probe before the install argument is built. golive-check should refuse a pair that fails the
   probe, or at least print it.
2. **Code.** When the forward walk's start is refused as too deep (or fails at one start for a bound), move the anchor. Set
   `sc.up` to the newest block the providers serve (or restart from `top` like a window reset, keeping `sale_cache`) and record the
   skipped span in the reading's note. The monitor then pages "scan lag > N blocks" (`fixes-round5.md` §10 already leaves the lag
   unpaged).
3. **Classification.** Keep a class of the provider's error in the transform, not just its code: for example, a small enum
   computed in the transform from the lowercased message, which is the same text on every replica for one provider. Then `Deep` and
   the provider size refusals the fix lists are real.

## B6-L-1 (Low): a receipt that can never be read stays first in the queue for the window: head-of-line block of every newer sale

**Code:** `ethflow.rs:1357-1397`.
- `sale_order` ranks each move by its repeats, oldest block first.
- The loop reads at most `SALE_RECEIPTS_PER_PASS` (16) in that order.
- A failed or disagreeing receipt goes to `again`, back into `pend` with its old block and rank. Nothing bounds its retries, and
  nothing moves it behind newer moves.

**Effect:**
- **The block.** With 16 or more such moves at rank 1 (distinct tokens and senders, the normal case for real transfers), the 16
  reads of every pass are spent on them. No newer receipt is ever judged until they leave the window, up to 7 days.
- **The trigger:**
  - the receipt depth of B6-M-1's provider (moves more than 20–30 k blocks old after an outage of about 3 days, or at launch with a
    pair that is deep for logs only);
  - a provider persistently disagreeing on old receipts.
- **Integrity holds.** A null is never judged "no sale" (only `Ok(Some(r))` with no sale, or the IC's size reject, is), so nothing
  false is counted.

**PoC:** `r6_b_l_sixteen_unreadable_old_receipts_hold_every_newer_sale`. 16 honest moves 30,000 blocks back whose receipts one
provider nulls:
- after two days, **0 of 144** newer final sales are read;
- the control, the same moves readable, reads 144 of 144.

**Fix:** count a receipt's failed reads (in `pend`). After a few, move it behind every newer move, or retry it only once a day,
and never let such moves take more than a share of the pass's 16 reads.

## B6-L-2 (Low, provider-conditional): a rate-limit answer (-32005) is taken as "too big": deferred ranges pile up, and real sales are lost for good

**Code:** `scan_error` `:1479` (`"-32005"`, and `"capacity"`, `"timeout"`), `scan_read` `:1581-1616`, the refinement split at
`:1296-1306`, `defer` `:1497-1505`, `block_sales` `:1511-1545`, `ethnet.rs:445`.

**What happens:**
- **The rate limit.** xrpc.cl, the pair's first provider, answers its anonymous rate limit with HTTP 429 and
  `{"code":-32005,"message":"XRPC anonymous capacity temporarily busy"}`. It survives the transform as code -32005, which is
  `TooBig`.
- **Measured** (one host): 9 of 20 sequential calls at one a second were busy, and 7–10 of 13 simultaneous ones. An IC read is
  13 replicas' calls. A mixed answer across replicas is a consensus failure (`Other`), and a mostly-busy one is `TooBig`.
- **Each busy answer, taken as "too big":**
  - on a walk read, it defers the range and makes a marketplace range read, which is busy too, or returns Blur's half alone;
  - in the refinement, it splits the range and pushes a new piece with no cap: `bad.push`, not `defer`;
  - on a single block, it goes through `block_sales`. There, a busy Seaport half with Blur answering returns `Ok` (`ethnet.rs:445`
    errs only when both fail), so the block goes to `done_big` and its Seaport sale is never read. Both busy make it `skipped`: 3
    tries, then never.
  - When `bad` is over 32, every `defer` evicts the oldest range, sales unread and all.
- **The size half of the same route.** A marketplace range read covers every collection's Seaport fills. The collection filter is
  in the transform, after the IC's cap. So a range read of more than about 500 blocks is over 2 MB on mainnet (measured 1.9 MB per
  500 blocks, 9.9 MB per 2,000), and returns Blur's half only.

**PoC:** `r6_b_m2_a_rate_limit_answers_are_taken_as_too_big`. A week, honest sales across the window and one every 100 blocks
after, Seaport's range half over the cap above 520 blocks:

| Busy rate | Due sales counted | Lost for good (no longer in any range, queue or skip) | Still deferred | `SaleScan.bad` max |
| --- | --- | --- | --- | --- |
| 0 % | 432 / 432 | 0 | 0 | 0 |
| 30 % | 431 / 432 | 1 | 0 | 43 |
| 50 % | **376 / 432** | **20** | 36 | **529** |

**Effect:** with no attacker, a keyless provider's rate limit steadily thins the canister's sale sample and permanently drops some
real sales. That makes more HELD posts and a weaker first-post path. No fake sale is produced. Low, because the sample only shrinks
(integrity holds) and it depends on the pair: a provider without an anonymous rate limit does not trigger it.

**Fix:**
- classify -32005 by the transform's message class (B6-M-1 fix 3). Treat a rate limit as `Other`, a retry that neither defers nor
  splits;
- `eth_sale_moves_range` / `block_sales`: record a block as `done_big` only when **both** marketplace queries answered. With one
  answered, keep it `skipped` for the other;
- cap `bad` in the refinement too (`defer`, not `push`), and evict a range only once its marketplace read succeeded;
- read a range's marketplace logs in sub-ranges of at most ~250 blocks: Seaport's global volume, measured.

## Round-5 items: verdicts

| ID | Verdict | Basis, residuals |
| --- | --- | --- |
| **B5-M-1** | **Closed** | Walks count only failed reads. An oversized range is deferred, and the walk goes on. The first read sets both ends even when too big (`:1186-1195`). The refinement runs after the walks on its own two failures. The fixer's `r5_b5_m1_a/b/c` reproduce. My attempts to starve the walk or the first post again: (1) oversized ranges whose marketplace read is oversized too: deferred with nothing, but the walk still passes and the refinement recovers them, newest first; (2) filling `bad` to evict real sales: needs more than 32 deferrals faster than 2 refinement failures a pass (about 3–5 passes an attacker range), which costs a sustained oversized-block supply at B5's prices; the eviction only loses sales whose marketplace read also failed (B6-L-2); (3) `done_big` over a real sale: only with B5-L-2's padded Seaport fill in the same block (unchanged) or a transient failure (B6-L-2); (4) the first read's range: anchored, its sales are either read through the marketplace logs or refined first (it is the newest). **Residual:** the "sales read at once" claim holds only for ranges under ~500 blocks (B6-L-2), and a gap past a provider's depth freezes the walk (B6-M-1, a provider issue, not an attacker's) |
| **B5-M-2** | **Closed in effect; the described mechanism is dead code** | The transform keeps only `code`. A -32005 cap is `TooBig`. A range refusal under any other code is `Other`, deferred after 6 failures at one start: about 3 passes a range, and the walk is never held for the window. `Deep` never fires (B6-I-1). The transient side is live: B6-L-2 |
| **B5-L-1** | **Closed** | `can_read` is checked before every log read, the marketplace read and `block_sales` (worst 4.928 MB on two providers, pass cap 10 MB). `can_receipt` is checked before every receipt (464 kB worst, cap 4 MB). The day's bound is 72 × 14 MB = 1,008 MB a collection. The spend is counted after the pass from `EthNet`'s own counters, so a pass cannot exceed 10 MB + 4 MB of reserve. A single `sale_scan` call is the only log reader, and nothing else charges the scan's counters |
| **B5-L-2** | **Partly** (unchanged) | Each marketplace is its own query now. A padded Seaport fill in a flooded block still hides that block's Seaport sales, now for good (`done_big`), because the one-sided `Ok` marks it read. The transient version is in B6-L-2 |
| **B4-L-2** (edge) | **Closed** | `done_big` is stepped over by the walks, the refinement and the rescan. It is capped at 64 (the lowest evicted; an evicted one may be re-read, which is harmless) |
| **B3-L-1** (carried) | Low, unchanged | A disagreement on the fallback read: the block is skipped, 3 tries, then never |

## The cycles bound (B5-L-1): acceptable

- **The price.** An HTTPS outcall's reserve costs 800 × n cycles a byte, which on a 13-node subnet is 10,400 a byte.
- **The worst case.** It is 14 MB a collection-pass: about 146 B cycles. The day's bound makes that at most 1,008 MB a day, about
  **10.5 T a collection, 52 T for five**. At about $1.35 per T that is about $14 and $70 a day, or roughly $2.1 k a month if it were
  sustained.
- **The honest cost.** A quiet collection after the window read makes 1–3 forward reads and a few receipts a pass: under 0.5 MB,
  or under 0.4 T a day.
- **What reaching the ceiling takes.** A pass is driven to its ceiling only by oversized reads, at B5's prices: 0.07–3.6 ETH a day
  a collection, about $170–$9,000. That is 12× to 600× what it costs us.
- **The floor.** Below `EVIDENCE_CYCLES_FLOOR` (8 T) the scan stops and the rest of the canister goes on.
- **Rating:** acceptable, provided the monitor pages the cycles burn rate and not only the balance. Note (B6-I-3): the bound is per
  collection. A top-up policy should assume 52 T a day under attack, and after a top-up from below the floor the scan can hit
  B6-M-1's gap.

## Infos

- **B6-I-1:** `r5_b5_m2_every_too_big_answer_takes_the_same_path_the_measured_shapes` asserts `scan_error` on raw provider
  messages the canister never receives. The transforms keep only the code (`pin.rs:214-219`). The test should build its inputs
  through `transfer_logs_body` / `sale_logs_body`, as `r6_b_m1_b_…` does. It then shows `Deep` and four of the five `TooBig` shapes
  are dead.
- **B6-I-2:** `SALE_BAD_MAX` (32) bounds only `defer`. The refinement's split pushes past it (529 in the PoC). The ranges are
  pruned to the window, so the growth is bounded, at about 16 B a range, but each pass's reading note lists them all.
- **B6-I-3:** `sc.stuck` is one counter shared by the three walks and keyed on a start. Interleaved failures of two walks reset each
  other's count, so the "6 at one start" catch-all can take longer than the 3 passes computed.

## Recommended fixes, in order

1. **B6-M-1.** A provider pair deep enough for the window in both logs and receipts (launch blocker; part 3's probe). Then the code
   path that re-anchors a forward walk refused for depth, and a lag page.
2. **B6-L-2 and B6-I-1 together.** Classify by a transform-kept message class, treat a rate limit as a retry, mark `done_big`
   only on a two-sided read, and cap the refinement's growth.
3. **B6-L-1.** Bounded receipt retries that cannot hold newer moves.
