# Round 4, part B: the canister's own sale sample (B3-M-1, the 500-block scan, B3-L-1)

Reviewer: independent AI re-review, 2026-09-27. Worktree `/Users/jlohayon/Documents/Rook-nft`, branch `nft-side` at `1eb5788`. Scope:
`ethflow.rs` `sale_scan`, `sale_order`, `SaleScan` (`pend`, `rescan`, `judged`), the sale cache; `ethnft.rs` `transfer_logs_body`,
`transfer_moves_of`, `Move`, `sales_in`, `counted_sales`; `ethnet.rs` `eth_transfer_moves` and `LOGS_BYTES`; `nftnet::ask`; the
operator's `logs_max_blocks` (a37f54b) and fix round 3's B3-M-1 (fb54018); the fixer's tests `r3_b3_m1_*`. Toolchain 1.98.1.

**Nothing in the worktree was modified except this report and `poc/B/`; nothing committed, pushed, deployed or signed.** The PoC ran
in a private copy of `autobuyer/canister-rs` (scratch dir, checked identical to the tree with `diff -rq` apart from the PoC file), on
the fake world of `tests/audit_nft_r3.rs` (its harness copied whole). The harness has two additions (`poc/B/b4_poc.harness.diff`):
a log reader that refuses, for size, any range holding one given block, and a pin that can advance.

Severity scale: the README's.

## Verdict

**B3-M-1's route is closed.** A flood of plain transfers can no longer starve real sales or the first post. The order holds up
under attack: a real sale waits only for moves of distinct tokens queued before it, and flooding past that costs one NFT held per
receipt of delay. The `judged` list cannot overflow from reads. Sales are never evicted from the cache by non-sales.

The sale sample still has a cheap stall, and it is worse than B3-M-1:

- **B4-H-1 (High):** a single Ethereum transaction whose `Transfer` logs exceed the largest response class (2 MB) inside 25 blocks
  stalls the scan at that range for as long as it is in the 7-day window.
  - **How:** the chunk halves down to 25 blocks and is then asked again, every pass, until the pass's whole log budget is spent.
  - **Effect on the floor:** either no newer sale is read for seven days (the floor is held and deposits stop), or the older half of
    the window is never read.
  - **Effect on cycles:** every pass pays 52 failed `eth_getLogs` of about 2.46 MB each. The operator's 500-block chunks
    quadrupled that from 13. The burn runs until the balance reaches `EVIDENCE_CYCLES_FLOOR`. Below that, no collection's floor is
    posted, so every module goes stale and mints and burns revert for everyone, unless the operator keeps paying for the burn.
  - **Cost to the attacker:** one transaction that re-transfers one NFT to itself about 3,300 times, roughly 0.002–0.03 ETH, once a
    week per collection.
  - **Origin:** the retry shape dates from round 2 (B-M-7); rounds 2 and 3 did not report it.

## Counts (this part: new, plus earlier items not actually fixed)

| Critical | High | Medium | Low | Info |
| --- | --- | --- | --- | --- |
| 0 | 1 | 0 | 2 | 3 |

(B3-L-1 carried: Low, re-rated below, counted once in the README as carried.)

## Earlier items in scope

| ID | Sev (R3) | Verdict | How it was re-attacked |
| --- | --- | --- | --- |
| **B3-M-1**: a flood of transfers starves real sales / the first post | Med | **Closed for its route** (a new route of the same effect, cheaper, is B4-H-1) | `sale_order` (`ethflow.rs:1099-1113`): rank = max(the older pending moves of the token, of the sender) + 1, then block, then hash. **A rank-1 real sale** is preceded only by rank-1 entries OLDER than it. Each of those needs a token with no older pending move, so the delay is bounded by the distinct NFTs moved before the sale (16 receipts a pass: 16 NFTs held per 20 minutes of delay). New flood entries are newer and sort after it. It is never truncated unless 512 entries precede it, which again needs 512 distinct tokens. **Rank-2+ real sales** (a seller's second sale, a token resold while its first sale is still pending) become rank 1 once the older sibling is read, and that sibling is older than a flood that follows. **The transform** (`ethnft.rs:461-493`) keeps the least (sender, token) per transaction. A flood transaction cannot claim a fresh least pair without a fresh token (rank = max of both counts), and mints (`from` 0) and self-transfers (`from == to`) are dropped. **`judged`**: at most 16 reads a pass × 504 passes in the window = 8,064 < 8,192, so it cannot overflow from reads at the runner's 1,200 s cadence (B4-I-2). An 8-byte prefix collision with a pending real sale's hash needs about 2^64 work. **The cache** sorts sales first (`:1073-1080`). The fixer's four flood shapes and first-post test are real (read in the harness). Residuals: B4-L-1, B4-I-1. |
| **B3-L-1**: `eth_getLogs` must be byte-identical from both providers | Low | **Open, unchanged, stays Low** | `nftnet::one` (`nftnet.rs:63-69`) on the transform's projection. The projection sorts, lowercases and drops `removed`, so formatting differences between providers do not matter; a provider missing a final block's logs does. Note the sibling in B4-H-1: a provider that refuses a dense range for a reason other than size (a result-count cap) takes the same halve-then-retry path. |
| B3-I-2: receipts per flood transaction | Info | Closed (each transaction is read once and kept in `judged`) | — |

## B4-H-1 (High): one oversized log range stalls the sale scan for the window and burns the log budget every pass

**Code.**

- `ethnet.rs:370-386` (`eth_transfer_moves`): the three classes `LOGS_BYTES = [64_000, 400_000, 2_000_000]` (`:102`). A size
  reject at 2 MB returns `SIZE_REJECT`. `nftnet::ask` (`nftnet.rs:52-61`) stops at the first provider that fails.
- `ethflow.rs:950-986` (`sale_scan`'s forward and backward walks): on any error the chunk halves, `shrink` = `(x/2).max(25)`
  (`:948`). The loop then asks the **same** start again with the next unit of `budget`.
- The budget is `13 × 2,000 / logs_max` = **52** at the operator's 500 blocks (`:945`, from a37f54b; 13 before).
- There is no per-range failure memory. The only exits are the window reset (`:934-940`, when `up` falls under `bottom`, seven days
  later) and `down.max(bottom)`.

**Trigger.** The IC rejects an HTTPS outcall whose raw response exceeds `max_response_bytes`, and the transform runs only after
that limit. A geth-style `Transfer` log is about 600 bytes of JSON, so about 3,300 of them in 25 blocks exceed 2 MB. Anyone
holding one NFT of a collection can produce that in **one transaction**:

- a contract calls `transferFrom(self, self, id)` in a loop, or ping-pongs the token between two of its own contracts;
- a warm self-transfer is on the order of 5–10 k gas, so 3,300 of them are 15–35 M gas, one block;
- at the base fees measured in `phase1-nft.md` (median 0.068 gwei, max 1.5 gwei) that is about **0.002–0.05 ETH**;
- the NFT is not spent;
- the canister's own transform drops self-transfers, but only after the size check, so they still count toward the response size.

A provider that caps a log query's result count, or its CPU, fails the same way at a lower threshold, with no attacker.

**Effect: the sample.** `poc/B/b4_h1.output.txt`:

- **(a) The oversized block inside the read window** (26,060,000, between the fixture's real sales). The walk back stops at it.
  After 72 passes (a day), 3 of the 5 real sales are read. The 2 older ones never are, while the block is in the window (seven
  days). The chunk sits at 25.
- **(b) The oversized block just past what was read** (the attacker's transaction after the window was read). The forward walk
  retries it with the whole budget: after a day, **0 of 40 newer final sales** are read, with `up` stuck under the block. This
  lasts until the block falls out of the window seven days later. Meanwhile:
  - the floor cannot confirm or rise past the old sales, which age out;
  - a collection before its first post can post only from the sales already read;
  - deposits wait on `deposit_floor_ok`;
  - the attacker renews it weekly.

**Effect: cycles.**

- **Per pass:** 51–52 failed reads (both PoCs: `[33, 52, 52] … [52, 52, 52]` and `[51, 52, 52] … [52, 52, 52]`), each paying the
  three classes, 2.464 MB of `max_response_bytes` plus 3 bases.
- **Per day, at the IC's published HTTPS-outcall price** (a fee charged on `max_response_bytes`, not on the answer):
  - 13-node subnet: 10,400 cycles a byte, about 25.8 B cycles a failed read, **1.34 T a pass, about 97 T a day** for one
    collection;
  - 34-node subnet: about 250 T a day.

  That is ×4 what the same stall cost before a37f54b, and about 50 threshold-ECDSA signatures' worth every pass.
- **The cycles floor:** the burn continues until `EVIDENCE_CYCLES_FLOOR` (8 T, `ethflow.rs:67`). There `floor_pass` refuses
  evidence (`:891-893`), no reading exists, and no collection's floor is posted. Every module goes stale within its 1 h TTL, and
  mints, burns and graduation fills revert for all five collections.
- **What the operator can do:**
  - **pay:** about 97–250 T, $130–$340 a day per griefed collection, for up to a week per transaction;
  - **stop the runner's evidence:** this also stops the floor;
  - **upgrade:** timelocked 72 h.

  Note B4-I-1: the evidence stops at 8 T before the cycles page fires at 5 T.

**Why High.** By the scale, a stranger can cheaply stop the floor, and with it burns, for everyone, or make the operator pay
continuously to keep it running. No holder value is taken from the modules. The attack renews weekly for a few cents to a few
dollars of gas.

**Not new, but not reported.** The halve-and-retry shape and the reset are round 2's (B-M-7, `55d8d32:ethflow.rs:838-873`, 13 reads
a pass). The operator's 500-block chunks (a37f54b) raised the per-pass bill to 52.

**PoC.** `poc/B/b4_h1.rs.snippet`, appended to the round-3 harness with `poc/B/b4_poc.harness.diff`. Two tests, both pass on
`1eb5788`: each asserts the weakness, and each turns red once it is fixed. They include a control with no oversized block, where all
5 real sales are read.

**Recommended fix.**

1. **Bound the failures.** Count failed chunk reads per pass separately from the budget, at most 2. At the minimum chunk, a size
   reject (at 2 MB) must not be asked again in the same pass.
2. **Bisect to one block, then mark it.** A single block still over 2 MB is either made of a flood (no real sale can be priced
   from it) or needs another route. Record it in `SaleScan` as read-as-unreadable, the logs' analogue of B2-L-2 for receipts, and
   move past it. What that gives up is the real sales in that one block. The canister does not count them as sales (the same rule
   as B2-L-2), the monitor pages it, and the operator can see it.
3. **Put a cycles bound on the evidence pass**, and a page when a pass spends more than N.
4. Optionally, ask `eth_getLogs` for the `Transfer` topic with `topics[1] != topics[2]`. JSON-RPC cannot express that, so bisecting
   is the practical route.

## B4-L-1 (Low): the re-read of dropped ranges waits for a queue that a cheap flood keeps full

`ethflow.rs:992` reads `rescan` only while `pend.len() + found.len() < 256`.

- **Cost of holding it:** one token ping-ponged at 16+ transactions a pass keeps the queue at 496–512 for about 0.01–0.1 ETH a day
  (its entries rank 2, 3, …, so they are the ones truncated, and they never displace a rank-1 sale).
- **What is lost:** the ranges an honest overflow already dropped are not read while the flood lasts. For example, the initial
  window read of a busy collection with more than 512 transfer transactions in a week (Pudgy-sized volume). What was dropped is the
  newest or highest-rank tail: repeat sellers, resold tokens.
- **What still works:** new rank-1 sales are still read first.
- **Effect:** some real sales of the busy week are never counted while the flood lasts. That narrows the sample, and it can hold a
  rise. It is bounded and heals when the flood stops, so Low.
- **Fix:** give `rescan` a fixed share of the budget each pass, say 1 chunk, rather than a queue-room condition; or keep dropped
  entries as `(hash, block)` (40 bytes) rather than a block hull.

## B4-L-2 (Low): the rescan's block hull grows across disjoint drops

`:1033` merges each drop into `(min, max)`. Two drops far apart re-read everything between them, one chunk at a time within the
budget. Each move already judged or cached is filtered, so this costs log reads only (bounded per pass), not receipts. Combined
with B4-H-1: an oversized block inside the hull stalls the re-read loop too (`:992-1010`: halve, retry, same start), taking the
budget left after the two walks. Fix: keep a short list of ranges; the B4-H-1 fix covers the stall.

## Infos

- **B4-I-1: the evidence stops before the cycles page.** `EVIDENCE_CYCLES_FLOOR` (8 T, `ethflow.rs:67`) and
  `DEPOSITS_CYCLES_FLOOR` (8 T, `frognet.rs:21`) are above `LOW_CYCLES` (5 T, `canister.rs:1649`), the threshold the `cycles()`
  query reports. A drain like B4-H-1's stops floors and deposits at 8 T, while the balance can sit above 5 T with no cycles page.
  The monitor's no-good-post pause and stale-module pages fire instead, which point elsewhere. Page at the evidence floor.
- **B4-I-2: `judged`'s bound depends on the runner's cadence.** One pass per evidence hand-in (`floor_pass`, `:882-889`); 16 reads
  a pass. At the runner's 1,200 s that is 8,064 in the 7-day window, under the 8,192 kept. A runner handing evidence more often (the
  operator's identity, `HANDIN_TTL_SECS` permitting) can exceed it, and the oldest judgments are forgotten and paid for again if
  re-found. Cost only. Derive the cap from the window and a minimum pass interval, or rate-limit `put_evidence` per collection.
- **B4-I-3: a pin that moves back one bucket resets the scan.** `sc.up > top` (`:934-940`) clears `pend` and `rescan` and re-reads
  the window: 2 passes of 52 chunks at 500 blocks. Receipts are not re-paid (`judged` and the cache are kept). This is round 2's
  rule; the cost grew with the 500-block chunks. Tolerate `up` up to one bucket over `top` instead of resetting.
