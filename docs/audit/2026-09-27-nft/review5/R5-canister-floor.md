# Round 5, part B: the canister's own sale sample after fix round 4 (B4-H-1, B4-L-1, B4-L-2, B4-I-3, B3-L-1)

Reviewer: independent AI re-review, 2026-09-28. Worktree `/Users/jlohayon/Documents/Rook-nft`, branch `nft-side` at `1dc2a61`. Scope:
fix round 4's B4-H-1 (`git diff 2735dac..1dc2a61`):

- `ethflow.rs`: `sale_scan`, `scan_read`, `ScanPass`, `floor_pass`, `reading_of`, `SaleScan.bad` and `.skipped`;
- `ethnet.rs`: `eth_sale_moves`, `logs_classes`, `eth_sale_logs`, `cycles_ok`;
- `ethnft.rs`: `sale_logs_body` and the scan constants;
- the fixer's tests `r4_b4_*` in `tests/audit_nft_r4.rs`.

Toolchain 1.98.1.

**Nothing in the worktree was modified except this report and `review5/poc/B/`; nothing committed, pushed, deployed or signed.**
The PoCs ran in a private copy of `autobuyer/canister-rs` at `/tmp/rook-r5/B/`, removed afterwards. Another reviewer deleted that
directory mid-review, so it was re-copied, checked identical to the worktree with `diff -rq` (`src/` identical; `tests/` differs only
by the added PoC file), and every PoC was re-run on the fresh copy. The harness is the fixer's own `tests/audit_nft_r4.rs`, copied
whole, with two switches added (`poc/B/r5_b.harness.diff`):

- `big_sale_blocks`: a block whose marketplace logs are also over the largest class;
- `dense_blocks`: a provider's result-count cap, answered as a JSON-RPC error.

The fake counts reserved bytes the way `EthNet::logs_classes` does: every class tried, times two providers.

Severity scale and threat model: the README's.

## Verdict

**B4-H-1 is closed as a High.** Its route no longer works.

- One oversized block now costs 7–9 failed reads once, and then nothing:
  - the control run: 7 failed reads in all, and 60 of 60 newer sales read by pass 4;
  - the fixer's `r4_b4_h1_*` pass.
- A pass's log reads are bounded.
- Low cycles no longer stop the post (`floor_pass` posts from listings and the sales already read).
- The marketplace route can add no fake sale. What it finds is judged by the same receipt rules as every other sale (`sales_in`,
  then `counted_sales`' self-trade, mint, wash and band filters).

The same effect on the sale sample (newer sales unread for days, or no first read at all) is still reachable, at a higher price and
without the cycles burn:

- **B5-M-1 (Medium):** the bisection costs ~log2(chunk) failed reads for each oversized block, and each walk may fail only twice a
  pass. So the attacker's cost per block is fixed, while the scan's cost per block grows with the chunk.
  - **(a) A burst of 126 oversized blocks, one block apart** (about 50 minutes of chain, paid once): no newer sale is read for
    504 passes, a week.
  - **(b) One oversized block a pass:** the forward walk falls ~60 blocks further behind every pass, 30,406 blocks after a week.
  - **(c) While the scan has no anchor** (at launch, and after a window reset): one oversized block every 50 blocks keeps the first
    read from ever landing. No sale is read and there is no first post.
- **B5-M-2 (Medium, conditional):** a range that a provider refuses with an error of its own (a result-count cap) is not a size
  reject. It still takes round 2's halve-and-retry path, with no memory. Two dense blocks (about 60 M gas) stall the forward walk
  for the window, exactly as B4-H-1 (b) did, at almost no cycles cost to the canister. Review 4 flagged this sibling under B3-L-1;
  the fix did not cover it.

The fix's cycles claim (worst pass 12.16 MB, "≈ 126 B cycles") is not a bound; see B5-L-1 and the recomputation below.

## Counts (this part: new, plus earlier items not actually fixed)

| Critical | High | Medium | Low | Info |
| --- | --- | --- | --- | --- |
| 0 | 0 | 2 | 2 | 3 |

B4-L-2 (partly fixed, stays Low) and B3-L-1 (Low) are carried and counted once in the README as carried.

## Round-4 items in scope

| ID | Sev (R4) | Verdict | How it was re-attacked |
| --- | --- | --- | --- |
| **B4-H-1**: one oversized range stalls the scan and burns the log budget every pass | High | **Closed as rated.** Residual Medium: B5-M-1, B5-M-2. Cycles claim: B5-L-1 | See **B4-H-1 in detail** below the table. |
| **B4-L-1**: the re-read waits for a queue a flood keeps full | Low | **Fixed** (residual: B5-I-2) | `share` (`ethflow.rs:1139-1141`) gives the re-read one chunk a pass whatever the queue. It still sits under `p.can`, so a pass whose earlier walks spent the byte budget skips it. `r4_b4_l1_…` passes. |
| **B4-L-2**: the rescan hull spans disjoint drops | Low | **Partly fixed; stays Low** | An oversized block inside the hull no longer stalls it. The hull still merges `(min, max)` (`:1212`). **New:** a single oversized block that the walk already resolved is not remembered (only an unreadable one is, in `skipped`). A hull over it pays its whole bisection again: 9 failed reads, 46 MB reserved over 30 passes (`r5_b4_l2_…`). A window reset does the same. Cost only, under the per-pass caps. |
| **B4-I-3**: a pin moving back resets the scan | Info | **Fixed** | The reset now needs `up < bottom` (`:1064`). Moves above a lowered `top` are kept in `pend` (`later`, `:1199-1203`); the walks do not run while `up >= top`. `r4_b4_i3_…` passes. |
| **B4-I-1**: the evidence floor is above the cycles page | Info | **Partly fixed** (residual: B5-I-3) | The post no longer stops. The scan's pause between 8 T and 5 T is still only a note in the reading. |
| **B3-L-1**: `eth_getLogs` must be byte-identical from both providers | Low | **Open; stays Low** | Unchanged (`nftnet::one`). **New edge:** `eth_sale_moves` also goes through `nftnet::one`. For an oversized block, a provider disagreement now counts as the fallback failing, so the block is skipped, retried 3 times (6 h, then 12 h, then 18 h later) and then given up: its sales are lost for good (before, the range was retried every pass). The sibling of B3-L-1 that review 4 named is B5-M-2. |
| Upgrade safety of `bad` / `skipped` | — | **Holds** | Both are `Option` with `#[serde(default)]` in a candid-encoded `Saved` (`stable_save`, `canister.rs:413`). An old record decodes them as `None`. `r4_states_saved_by_the_old_builds_…` restores 4 old builds' states (1eb5788's with a non-empty `SaleScan`) with nothing lost and both fields `None`. `eth-state-round4.hex` round-trips them set. A rollback to an older build is refused by `lost_opts`, by design. |

### B4-H-1 in detail

**What holds:**

- **Bisection.** `SaleScan.bad` is bisected correctly in both walk directions (`scan_read`, `ethflow.rs:1302-1384`):
  - the trimming around ranges ahead of the walk is correct;
  - a success narrows the known range;
  - a failed half replaces its parent.
- **The fallback.** A single block goes to `eth_sale_moves`. Its transform keeps only this collection's Seaport 1.5/1.6 and Blur v2
  sales, and each transaction found is then judged by its receipt.
- **Skipped blocks.** A block with no readable route goes to `skipped`: at most one retry a pass, 3 tries, then `u64::MAX`.
- **Per-walk cap.** At most 2 `Big` / `Failed` reads a walk. At most 3 failed reads a pass were observed, including the one-block
  read.
- **Cycles check.** `cycles_ok` is `balance >= 8 T` (`ethnet.rs:224-226`), checked once per collection before the scan. That is
  the right direction and harmless granularity.
- **Posting under the cycles floor.** `floor_pass` still posts under the floor (`r4_b4_h1_d`). The window is filtered at the current
  pin, and the sales it uses are only the canister's own judged ones. Stale sales age out; nothing can be injected. With fewer than
  `min_sales`, the post is HELD (it cannot rise), which is the conservative side.

**What does not hold:** B5-M-1, B5-M-2, B5-L-1.

## Findings

### B5-M-1 (Medium): a steady rate of oversized blocks outpaces the bisection, so newer sales wait for days, or there is no first read at all

**Code.**

- `scan_read` (`ethflow.rs:1302-1384`): an oversized range is isolated by halving. Each failing half is one `Got::Big` read, 2.464 MB
  reserved on each of two providers (4.928 MB counted).
- The walks (`:1093-1113` forward, `:1116-1132` back, `:1140-1162` re-read) stop for the pass at `SALE_SCAN_FAILS_PER_WALK = 2`
  (`ethnft.rs:124`, `ScanPass::can` `ethflow.rs:1281-1283`).
- When a half fails, its parent is removed (`:1350-1352`, then `bad.push`), so the far half is forgotten and the next oversized
  block after the resolved one costs a full bisection again. `sc.chunk` is not shrunk on `Big`, so that bisection starts from 500
  blocks: 9 failed reads.
- With no anchor (`up == 0`, `:1094-1095`), only the first read runs, down from `top`, and the walk back needs `up != 0` (`:1116`).

**Trigger.** An oversized block is one transaction re-transferring one held NFT of the collection to itself ~3,300 times (review 4:
15–35 M gas, about 0.001–0.05 ETH at 0.068–1.5 gwei; the NFT is not spent). Each shape below was measured in the fake world, at the
runner's cadence (the pin moves 100 blocks a pass):

- **(a) A burst, one block apart** (`r5_b5_m1_a_…`).
  - Every other block holds a flood. After each resolved block, the next costs 9 failed reads, 4 passes at 2 a pass.
  - 126 flood blocks (~250 blocks, ~50 minutes): **no newer sale is read for 504 passes (7 days)**. The walk is 7,165 blocks
    behind after a day, and the first newer sale is read at pass 504.
  - Meanwhile the old sales age out: the last 66 passes (~22 h) post HELD, so the floor cannot rise.
  - Cost: 126 × 15–35 M gas, **≈ 0.13–6 ETH once per week per collection** (≈ 0.17 ETH at the median fee).
  - Control, the fix's own case (one block): 60 of 60 newer sales read by pass 4.
- **(b) Sustained, one oversized block a pass** (`r5_b5_m1_b_…`, every 100 blocks).
  - The forward walk resolves one block in about 2–3 passes, so it is 4,499 blocks behind after a day and 30,406 after a week.
  - 200 of 504 newer sales were read in the week, the newest days never.
  - After ~12 days the lag passes the window's start and the scan resets (`:1064-1069`), which puts it in case (c).
  - Cost: **≈ 0.07–3.6 ETH a day** (the fixer's own §3 estimate, 0.15–3.6).
- **(c) No anchor** (`r5_b5_m1_c_…`).
  - At a collection's first pass, or after a reset, the first read (the pass's newest 100 blocks) fails if a flood block is in its
    upper half; the half nearest `top` fails again, and the pass is spent. The next pass asks the next 100 new blocks the same way.
  - **One flood block every 50 blocks, at any offset:** 0 of 5 real sales read in two days, `up` stays 0, and there is no reading
    with ≥ 3 sales. The floor's first post (`floor_from`: "no first post until the listings are confirmed") never comes.
  - `SaleScan.bad` reaches its cap of 8 and evicts (B5-I-1).
  - Cost: 144 blocks a day, **≈ 0.15–7 ETH a day**.
  - Control, one flood block every 100 blocks in the lower half of each pass's new blocks: the first post comes at pass 63 (~21 h).
    Even that is a long launch delay.

**Impact.**

- The canister's sale sample stalls or lags by days:
  - the floor cannot confirm a rise (HELD once the old sales age out);
  - deposits wait on `deposit_floor_ok`;
  - with (c), a newly launched collection gets no floor at all while the flood lasts (D1 launches three at once).
- The floor is still posted from listings, burns keep working, and there is no cycles outage. Per pass the canister spends
  ≈ 15.0–15.4 MB counted (≈ 156–160 B cycles, ≈ 11.5 T a day per collection).
- **Why Medium, not High.** It stops the floor's rise and a new collection's first post, not the floor itself. The attacker pays
  continuously, 1–3 orders of magnitude more than B4-H-1's route. It is bounded to the flood's duration.

**PoC.** `poc/B/r5_b.rs.snippet`: `r5_b5_m1_a_…`, `r5_b5_m1_b_…`, `r5_b5_m1_c_…`. They pass on `1dc2a61`; each asserts the weakness,
and each has a control.

**Fix.** Decouple the walk's progress from the bisection:

1. **Defer, don't block.** On `Big`, record the range in `bad` and move the walk's front past it at once (forward: `up = b`; back:
   `down = a`). Newer and older blocks keep being read, and the bisection of deferred ranges runs on whatever budget is left, newest
   first. An oversized range then delays only its own sales.
2. **Anchor even on failure.** When `up == 0`, set `up = top` and `down = top + 1` on the first `Big`, deferring the range.
   Case (c) disappears.
3. **Remember resolved oversized blocks** (a short list of block numbers, pruned to the window), so a hull or a reset never
   re-bisects them. This also closes B4-L-2's new edge.
4. Optionally, start a re-read of a known-dense neighbourhood at a smaller chunk.

### B5-M-2 (Medium, conditional on a provider): a provider's own refusal of a dense range still stalls the walk for the window

**Code.** `scan_read` treats only the IC's size reject as `Big`: `Err(e) if e.contains(SIZE_REJECT)` (`ethflow.rs:1349`). Everything
else is `Got::Failed` (`:1381`). The walks then `shrink` the chunk to at least 25 blocks and ask the **same start** again
(`:1107-1111`, `:1126-1130`, `:1156-1160`). Nothing is remembered: no `bad`, no bisection to one block, no marketplace route, no
`skipped`.

A provider's JSON-RPC error arrives as `no result: {…}` (`canister.rs:749-751`), deliberately never a size reject (R3-C2).

**Trigger.** The canister's keyless Ethereum pair (`phase1-nft.md` D6) is public endpoints. Public endpoints commonly refuse a log
query over a result count with an error rather than the logs:

- "query returned more than 10000 results";
- reth's default `max-logs-per-response` is 20,000.

Against a 10,000 cap:

- two adjacent blocks of ~5,001 self-transfers each (≈ 30 M gas each, ~60 M in all, ≈ 0.004–0.09 ETH);
- each alone is over the IC's 2 MB (so it would go `Big`), but any range holding both is refused by the provider;
- at the 25-block minimum chunk, the range from the walk's front always holds both.

A 20,000 cap needs about twice the gas. A provider that times out on a dense range (no attacker) takes the same path.

**Impact.** Exactly B4-H-1 (b)'s effect on the sample:

- no sale newer than the pair is read until the pair leaves the window, seven days later, and then the reset leads to B5-M-1 (c);
- measured over one day (`r5_b5_m2_…`): 0 of 40 newer sales read, `up` stuck 4 blocks before the pair, chunk 25, `bad` and
  `skipped` empty, 2 failed reads every pass.

The cycles cost is small: the failures are small error bodies. So the only limit on this stall is the attacker's ~60 M gas a week.

Medium because it needs a provider behaviour that has not been verified for the configured pair. It is High-shaped if the pair
does refuse dense ranges.

**PoC.** `r5_b5_m2_a_provider_result_cap_error_still_stalls_the_forward_walk_for_the_window_weakness`.

**Fix.** Treat persistence, not the message:

- a range that fails at the minimum chunk (any error) on N consecutive passes (say 3) is recorded in `bad` like a size reject and
  bisected down to one block;
- a single block failing that way goes through `eth_sale_moves`, then `skipped`.

Transient errors heal before N. Also measure the configured pair's `eth_getLogs` limits (result count, time) and write them into
`phase1-nft.md` §7.

### B5-L-1 (Low): the per-pass byte budget is not a bound: the fallback is unchecked, receipts are outside it, and the stated worst is low

**Code.**

- `ScanPass::can` (`ethflow.rs:1281-1283`) is checked before each `scan_read` only.
- Inside one `scan_read`, a single known-oversized block reserves its `Transfer` logs (4.928 MB counted) and then its marketplace
  logs through `eth_sale_moves` (`:1353-1372`, another up to 4.928 MB when they reach the 2 MB class) with no check between.
- The receipt reads (`:1217-1245`: 16 a pass, 32 kB + 200 kB classes, two providers, `ethnet.rs:392`) are not counted at all.
- The budget is per collection per pass (`bytes_from` at each `sale_scan`), and passes are set by the runner's hand-ins
  (B4-I-2, unchanged), so nothing bounds it per unit of time.

**Measured.**

- `r5_b5_l1_…` (b): one pass reserved **19.71 MB** (205 B cycles). That is over the fixer's own test bound (12 MB + 2 × 2.464 MB =
  16.93 MB) and its stated worst (12.16 MB). The setup: the forward walk's two failures, then the walk back's front on a
  known one-block range (the rest of a two-block bisection) whose marketplace logs are also oversized.
- The natural runs of B5-M-1 reserve 15.0–15.4 MB a pass.

**Cycles, recomputed** (the IC's HTTPS-outcall price, not in the code: `(3,000,000 + 60,000·n)·n` per call, `400·n` per request byte,
`800·n` per `max_response_bytes`; n = 13 gives **10,400 cycles per reserved byte** and 49.1 M per call):

| Item | Counted bytes | Cycles at 13 nodes |
| --- | --- | --- |
| A failed log read (3 classes, 2 providers counted) | 4.928 MB | ≈ 51 B (charged ≈ 25.8 B: `nftnet::ask` stops at the first provider that errors) |
| The log reads' bound per collection-pass: < 12 MB, plus one unchecked `scan_read` (2 × 4.928 MB) | ≤ 21.86 MB | ≤ 227 B |
| Receipts (16 × (32 + 200) kB × 2) | ≤ 7.42 MB | ≤ 77 B (≈ 39 B charged when the 200 kB class is also refused) |
| Listings (≤ 8 × ~6 small calls × 8 kB × 2, plus base fees) | ≈ 0.77 MB | ≈ 13 B |
| **Worst per collection-pass** | **≈ 30 MB** | **≈ 0.32 T** |

- **Worst cases:** three launch collections at the runner's 72 passes a day, ≈ **69 T a day**; five collections ≈ 115 T; ×2.6 on a
  34-node subnet.
- **What an attacker actually drives** (B5-M-1): ≈ 156–160 B a collection-pass, ≈ 11.5 T a day per collection. The fixer's
  "≈ 126 B, ≈ 9 T a day" understates both the bound and the measured case.
- Everything the scan's logs cost is under the one `ScanPass`: the three walks, the one-block fallback (as counted, but unchecked
  within its read) and the skipped retry. Receipts, listings and pins are not.

**Why Low.** The drain is bounded per pass, and the floor no longer depends on it (`cycles_ok` only pauses the scan). What remains
is a mis-stated budget and a cost that scales with the runner's cadence.

**Fix.**

1. Check `p.can` again before `eth_sale_moves` inside `scan_read`. Over budget, record the block in `skipped` with `at = now` (read
   next pass) instead of reading it.
2. Count receipt reads in `ScanPass` (or give them their own byte cap).
3. Rate-limit passes per collection (B4-I-2).
4. Correct §3 of `fixes-round4.md` to the bound.

### B5-L-2 (Low): the marketplace fallback is not filtered by collection, so a padded block hides its real sales

**Code.**

- `eth_sale_moves` (`ethnet.rs:416-426`) asks one block's logs by address (Seaport 1.6, Seaport 1.5, Blur v2) and `topic0` only.
  JSON-RPC cannot filter by the collection, which sits in the log's data.
- `sale_logs_body` (`ethnft.rs:512`) filters by collection only after the IC's size check.
- The docstrings (`ethnft.rs:505-511`, `ethflow.rs` `eth_sale_moves`) state that "a block cannot hold 2 MB of sale logs (each fill
  pays a marketplace's gas)".

**Trigger.** The docstring's claim does not hold against padding:

- a Seaport self-fill (the offerer is the caller, so no signature) with ~6,000 consideration items of the attacker's own no-op ERC-20;
- ≈ 4–5 k gas an item, plus quadratic memory, is ≈ 28–31 M gas;
- it emits one `OrderFulfilled` of ≈ 2 MB of JSON hex.

Together with the ~20 M-gas `Transfer` flood in the **same block** (both fit a 60 M-gas block), the block is unreadable both ways:
it is `skipped`, retried 3 times, then given up, and its real sales are never counted.

**Targeted use:** backrun a chosen public sale in the same block (a bundle), and that sale is censored from the sample. The cost is
about 50 M gas plus the builder's price, per censored sale.

**Impact.** Selective removal of individual sales from the sample. `counted_sales`' band and quorum, and the listing floor, bound
what it moves, and the note names the block. It costs about a block's gas per sale. Low.

**Fix.**

1. Split the fallback into one query per marketplace address and topic (4–6 small reads), so padding one hides nothing on another.
2. Page the operator when a block is given up.
3. Drop the docstring's claim.

## Infos

- **B5-I-1: `SaleScan.bad` holds 8, oldest evicted** (`ethflow.rs:1373-1377`).
  - Reached only by B5-M-1 (c) (8 entries: the upper halves of past passes' first reads). Under the walks, at most one range per
    front is live (1–2 observed).
  - An evicted range costs at most one extra failed read if asked again, and is then re-added. No stall and no re-asking of whole
    ranges every pass (the per-walk cap holds).
  - The ninth range simply evicts the first.
- **B5-I-2: B4-L-1's re-read chunk is still under the byte budget.** `share` bypasses the queue-room condition, not `p.can`. A pass
  whose forward walk spent the budget (B5-M-1's 15 MB passes) re-reads nothing. This is subsumed by B5-M-1's fix.
- **B5-I-3 (B4-I-1's residual): the scan's pause below `EVIDENCE_CYCLES_FLOOR` is silent until 5 T.**
  - Between 8 T and `LOW_CYCLES` (5 T), the only signal is the reading's note "did not run".
  - The sample ages. After about 7 days with fewer than `min_sales`, posts are HELD (they cannot rise), and deposits wait.
  - Page at the evidence floor, or have the monitor match the note.

## What holds

- **One oversized block** (the fix's case) is isolated once: 7 failed reads, then 0, with every newer and older sale read (the
  control runs; `r4_b4_h1_a/b/c`).
- **Direction handling in `scan_read`:** the "inside" test, the trimming against a range ahead, and "the half nearest the walk"
  were checked by hand and by the runs. They are consistent for the forward walk, the walk back, the re-read and the anchorless
  first read.
- **No fake sales from the fallback.** Its transactions are judged exactly as the normal path's:
  - `eth_sale_receipt`, then `sales_in` (`ethnft.rs:668-695`: a marketplace log AND the collection's `Transfer` of that token);
  - then `counted_sales`' exclusions (`:739`: `seller == buyer`, from 0, round trips, out of band);
  - an attacker's flood transaction in the same block has a receipt over 200 kB, so it is judged as no sale (B2-L-2);
  - discovery differs (the fallback finds transactions by marketplace log, including self-fills with no two-wallet move), but
    counting does not.
- **The first post under a flood of the newest blocks that spares the lower half of each pass's new blocks** is made (control of
  `r5_b5_m1_c`: pass 63). An anchored scan's walk back runs on whatever the forward walk leaves of the budget, so older sales keep
  being read under B5-M-1 (b).
- **The floor post no longer depends on cycles**, and posting from stale sales is not manipulable (only the canister's own judged
  sales, filtered to the current window).
- **The per-walk failure cap and the per-pass budget apply to all three walks and the skipped retry.** Before this round's fix, 52
  failed reads a pass; now ≤ 3.
- **Upgrade:** old states restore with `bad` and `skipped` as `None`; new ones round-trip.

## PoCs

`poc/B/`:

- `r5_b.harness.diff`: the two switches added to a copy of `tests/audit_nft_r4.rs`.
- `r5_b.rs.snippet`: the tests, appended to it.
- `r5_b.output.txt`: the run on `1dc2a61`: 6 passed.

| Test | Shows |
| --- | --- |
| `r5_b5_m1_a_burst_…_weakness` | B5-M-1 (a); control = the fix's case |
| `r5_b5_m1_b_one_oversized_block_a_pass_…_weakness` | B5-M-1 (b) |
| `r5_b5_m1_c_no_anchor_…_weakness` | B5-M-1 (c); control = the first post made |
| `r5_b5_m2_a_provider_result_cap_error_…_weakness` | B5-M-2 |
| `r5_b5_l1_cycles_a_pass_reserves_more_than_the_12_mb_budget_weakness` | B5-L-1 (19.71 MB in one pass) |
| `r5_b4_l2_a_rescan_hull_over_a_resolved_oversized_block_…` | B4-L-2's new edge (9 failed reads again, 46 MB) |

**How to reproduce:**

1. Copy `autobuyer/canister-rs` somewhere else.
2. Copy `tests/audit_nft_r4.rs` to `tests/r5_b.rs` and apply `r5_b.harness.diff`.
3. Append `r5_b.rs.snippet`.
4. Run `cargo test --test r5_b r5_ -- --nocapture --test-threads 1`.
