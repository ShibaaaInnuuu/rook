# Review round 3, part B: the canister's floor and sale sample (after fix round 2)

Reviewer: independent AI review, 2026-09-27. Worktree `/Users/jlohayon/Documents/Rook-nft`, branch `nft-side` at HEAD `4949da0`
(round-2 fix delta `79455e1..HEAD`). **Nothing in tracked code or tests was modified. Nothing was deployed, pushed or signed.** All
PoCs ran in a private copy of the canister crate under my scratch dir, never in the worktree (`cargo test`, toolchain 1.98.1 from
`~/.cargo/bin`). Line numbers at HEAD `4949da0`.

Scope: `ethnft.rs` (`counted_sales`, `round_trips`, `floor_from`, `sales_in`, `sales_window`, `quantile`/`median`, the held/flag
logic, `transfer_logs_body`/`transfer_txs_of`), `ethflow.rs` (`sale_scan`, `SaleScan`, `sale_cache`, `floor_pass`, `reading_of`,
`post_reading`, `deposit_floor_ok`, `due_readings`), `ethnet.rs` (`eth_transfer_txs`, the `eth_transfer_logs` transform, chunk
halving, size classes, `SALE_FINAL_DEPTH_BLOCKS`), `nftnet.rs` (`ask`/`one`/`agreed` consensus), the `.did`, and the fixer's test
adaptations (`tests/audit_nft_r2.rs`, `tests/ethnft.rs`, `tests/audit_nft.rs`, `fixes-round2-poc/`).

## Verdict (part B)

Round 2's floor/sample rewrite is sound in its arithmetic (the washer's median is closed, the deposit band is the canister's own
`S25`, oversized receipts are cached once, the veto-that-froze-posts is replaced by a held post). Every round-2 PoC re-runs on this
code and **fails on its attack assertion, not on a control** — the fixer's `fixes-round2-poc/` claim is honest.

**But the round-2 fix's central invariant is false.** `fixes-round2.md` §0/§1.2 and the `counted_sales` comment (`ethnft.rs:601-603`)
claim: "the sample is the whole window, so no volume of washes crowds a real sale out", and §1.4 claims that "many real transfers
lengthen the scan's queue [but] neither stops a post nor opens the deposit band". Both are wrong. The `SaleScan` queue is capped at
512 (`SALE_TODO_MAX`), built newest-first, and **entries past 512 are silently dropped and their block range is never re-scanned**
(`ethflow.rs:879-889`). A gas-only flood of plain ERC-721 transfers (not even sales) permanently starves every real sale out of the
queue: the floor is held forever (never confirms, never rises), deposits (D5 on) never mint, and — before a collection's first post —
the floor post is blocked entirely, so the module is stale from genesis. This reopens the harm B2-M-1 / A-M-1 were meant to close,
by a different route (**B3-M-1**, Medium). Also one Low (provider disagreement stalls the sample) and two Infos.

| Area | Critical | High | Medium | Low | Info |
| --- | ---: | ---: | ---: | ---: | ---: |
| New in round 3 (B3-) | 0 | 0 | 1 | 1 | 2 |
| Round 2, not fully closed | 0 | 0 | 0 | 0 | 0 |
| **Open, part B** | **0** | **0** | **1** | **1** | **2** |

The one Medium: **B3-M-1** — `ethflow.rs:879-889` (`sale_scan`, `SALE_TODO_MAX` truncation) + `ethnft.rs:604` (`counted_sales`) +
`ethflow.rs:2101-2123` (`deposit_floor_ok`).

## 1. Round-2 items in scope

| ID | Sev (R2) | Verdict | Basis |
| --- | --- | --- | --- |
| **A-M-1** (deposit → burn at a high floor) | Med | **Closed as a bypass; the gate is now DoS-able (B3-M-1)** | The deposit band compares `accepted` with the LOWER of `L` and the canister's OWN `S25` (`deposit_floor_ok`, `ethflow.rs:2101-2123`), and `S25` is the 25th percentile of `counted_sales` read by the canister itself. Washes no longer set it (`r2_a_m1_…` passes; the round-2 PoC `audit2_a_m1_…` now fails: the median is the real 0.91, not the washer's 1.20). The burn side is the post, which never exceeds a live listing and rises only as far as `S25×1.10`. **But** the gate requires a quorum of counted sales, and B3-M-1 shows an attacker can permanently prevent that quorum (deposit never mints). The bypass is closed; the availability of the gate is not. |
| **B2-M-1 / D2-M-2** (sales veto stops the post) | Med | **Partly closed** | For a collection that has already posted, `floor_from` returns a HELD post (`min(L, max(accepted, S25×1.10))`, confidence 5000) instead of an error, so washes can no longer make the module stale (`r2_b2_m1_…` passes; `audit2_b2_m1_…` now fails on its attack line). The reviewer's veto-freeze is gone. **But** B3-M-1 shows the freeze of the harmful part — the floor never rising and deposits never minting — is still reachable cheaply, and at the FIRST post (`accepted == 0`) the post is blocked entirely (module stale from genesis). The claim "no volume of washes crowds a real sale out" is refuted. |
| **B2-L-1** (upward veto wash-disarmable / swept book) | Low | **Closed (residual acknowledged)** | The band is over `S25` (25th pct), not the median (`floor_from`, `ethnft.rs:695`), and lifting it needs wash votes outnumbering ¾ of the window's real distinct-wallet sales, each its own token + two fresh wallets (`counted_sales` one-per-token/one-per-wallet + `round_trips`). `audit2_b2_l1_…` now fails: the swept book is HELD at `S25+10%`, not posted at the attacker's 1.12. The market-power residual (sweep + outnumber ¾ of sellers) is stated in §10; genuinely expensive now. |
| **B2-L-2** (oversized receipt re-reserved every pass) | Low | **Closed** | A receipt the IC refuses at the largest class is cached as no-sale and never re-read (`ethflow.rs:901-905`). `audit2_b2_l2_…` now fails ([5,0,0,0] reads, not [12,12,12,12]); regression `r2_b2_l2_…` present. |
| **B2-I-1** (a veto leaves the stale reading in place) | Info | **Closed** | On a `floor_from` error, `floor_pass` drops the reading (`ethflow.rs:812-814`), so no stale post/buy/deposit uses it. |
| **B2-I-2** (self-contradicting `post_reading` comment) | Info | **Closed** | Comment corrected (`ethflow.rs:2043-2046`). |
| **B2-I-3** (deep-crash "no post" understated) | Info | **Closed** | The crash case is now a flagged post at the listings (`floor_from` `l < lo` branch, `ethnft.rs:706-712`); docs updated. |

Also re-checked unchanged and correct: `sales_in` Seaport/Blur parsing (lowest price per token, requires the collection's own
`Transfer`, `ethnft.rs:539-566`); `round_trips` (`ethnft.rs:578-595`, correctly flags A→B→A and rings); `quantile`/`median`
(`ethnft.rs:645-653`, no off-by-one or panic — called only on a non-empty sorted slice, `(n-1)*num/den` stays in range);
`floor_from`'s per-offerer/one-token dedup and rank selection.

## 2. New findings

### Medium

#### B3-M-1: a gas-only plain-transfer flood permanently starves real sales out of the 512-entry scan queue — the floor is held forever, deposits never mint, and the first post is blocked entirely

- **Where:** `ethflow.rs:826-932` (`sale_scan`), specifically `:879-889` (the queue is built newest-first from `newer ++
  sc.todo ++ older`, deduped against the cache, then `todo.truncate(SALE_TODO_MAX)` — `SALE_TODO_MAX = 512`, `ethnft.rs:104`).
  Only up to `SALE_RECEIPTS_PER_PASS = 16` receipts are read per pass (`:891-896`); the rest that survive truncation are kept in
  `sc.todo`, but **entries beyond 512 are dropped and are never re-fetched** because their block range is already inside the scanned
  interval `[sc.down, sc.up]` (`:842-877`) and is not scanned again. Downstream: `ethnft.rs:604` (`counted_sales`),
  `ethflow.rs:2101-2123` (`deposit_floor_ok` requires a quorum), `ethnft.rs:719-720` (`floor_from` errs at `accepted == 0`).
- **Mechanism.** The canister's own sale sample is the collection's `Transfer` logs, and every log becomes a queue entry
  (`transfer_logs_body`, one per transaction, `from != to`, `from != 0` — `ethnft.rs:431-456`). A plain `transferFrom` (no
  marketplace log) is such a log but not a sale (`sales_in` returns nothing for it). An attacker moves a token they own between
  wallets — even ping-ponging one NFT A→B→A→… (each tx a distinct queue entry) — to emit an unbounded number of `Transfer` logs at
  the newest final blocks, for gas only (~30 kgas each; no marketplace, no wash-pair rules apply because these are not counted
  sales). Once more than 512 of these land newer (higher block) than the real sales in the same window, the newest 512 fill the
  queue, the real sales (older, appended after `newer`) are truncated out, and their block range — already scanned — is never
  re-read. The real sales are gone from the sample permanently, even after the flood stops (until they age out of the 7-day window,
  by which point they are moot). To hold the freeze indefinitely the attacker re-floods as new real sales appear; sustained cost is
  roughly one flood tx per real sale plus a margin (order of 0.1–0.5 ETH/day per attacked collection at Ethereum gas; a one-time
  burst suffices to wipe the current sample). Low-volume collections (BAYC/Pudgy later) are the cheapest to keep frozen.
- **Impact.**
  - For an already-posting collection: `counted_sales` sees 0, `floor_from` returns a HELD post, so the floor **can never confirm
    or rise** and `deposit_floor_ok` **never admits a mint** — with D5 ON, every holder deposit sits in `XPhase::Mint` forever
    (never refused, never returned), and the pool's buy cap is pinned at the held level. This is exactly the freeze A-M-1/B2-M-1
    were meant to remove, reached by a cheaper and more reliable route than the acknowledged wash-vote residual.
  - Before a collection's first post (`accepted == 0`, e.g. at launch): `floor_from` returns `Err` (no held fallback,
    `ethnft.rs:719-720`), so the module **never receives any floor** and mints, burns and graduation fills revert `Stale` from
    genesis until the flood stops and a clean sample is read.
  - It does not make an already-posted module stale (the held post keeps it fresh, so burns/mints still work), which bounds it below
    the original B-H-1/B2-M-1 High. No value is lost and it self-heals when the flood stops. Consistent with the reviewer's own
    grade of the same freeze class as **Medium**, and it directly refutes `fixes-round2.md` §0/§1.2/§1.4.
- **PoC** (`review3/poc/B/b3_nft_flood.rs.snippet`, run in the repo's `audit_nft_r2.rs` fake harness; output
  `b3_m1.output.txt`), all passing:
  - `b3_m1_control_no_flood_…`: 5 real sales → `sales counted = 5`, `held = None` (confirmed), the deposit mints (`items = [950]`).
  - `b3_m1_a_plain_transfer_flood_permanently_starves_real_sales_…`: 520 gas-only plain transfers newer than the real sales →
    `cached real sales = 0`, `sales counted = 0`, held forever, `deposit not minted`, and `todo left = 0` (the queue fully drained,
    so the starvation is permanent, not a transient backlog).
  - `b3_m1b_at_first_post_accepted_zero_…`: same flood with `accepted = 0` → `module posts = 0`; the module never gets a floor.
- **Recommended fix.** Do not drop queue entries on truncation. Either (a) key the scan on the block interval read (persist which
  final blocks have been fully judged) so a truncated transaction is re-discovered when its range is re-scanned, rather than lost;
  or (b) cap the receipt reads per collection but keep the FULL set of pending transaction hashes (they are only 32 bytes each; 7
  days of Milady transfers is far under any real memory limit) so no real sale is ever silently discarded; or (c) as the reviewer
  originally recommended, sample sales by a wallet-diverse, time-based method that a flood of raw transfers cannot dominate. In all
  cases, prefer the OLDEST-unread transactions when the read budget is spent so the sample cannot be starved by newer volume; and
  when `accepted == 0`, do not let a transient empty sample block the first post indefinitely (page instead).

### Low

#### B3-L-1: with two providers and strict all-agree consensus on `eth_getLogs`, one disagreeing or lagging provider stalls the sale sample indefinitely (held floor, no deposit mints)

- **Where:** `ethnet.rs:367-383` (`eth_transfer_txs` → `nn::one`), `nftnet.rs:52-68` (`ask` propagates the FIRST provider error;
  `one` requires byte-identical answers from every provider), the launch template's two-provider pair (`fixes-round2.md` §6).
- **Mechanism.** Unlike `fees()` (`ethnet.rs:138-159`), which tolerates provider disagreement as "unread" (a wait), the sale scan's
  log read requires every provider to return the exact same set (after the `eth_transfer_logs` transform). `eth_getLogs` over
  2,000-block ranges is far more prone to cross-provider divergence than single-value reads (result caps, indexing lag, `removed`
  handling). With exactly two configured providers, any persistent divergence — one provider truncating a large chunk, or lagging
  on a final-block range, or a transport error — makes `eth_transfer_txs` error on every chunk. `sale_scan` then never advances
  `sc.up`/`sc.down`, the window stays empty, and the floor is held (never rising/confirming) with deposits never minting, exactly as
  in B3-M-1 but from provider flakiness rather than an attacker (and it does not need an attacker).
- **Impact.** A liveness/robustness degradation of the floor's upward mobility and of deposit mints, self-healing when providers
  agree; no value loss. **Low.** (Chunk halving down to 25 blocks mitigates size-cap divergence but not lag or a persistent liar.)
- **Recommended fix.** Treat `eth_getLogs` like `fees()`: reconcile providers (e.g. the union of final-block logs, or a majority
  once the block range is well below the head) rather than demanding byte-equality; and require ≥ 2 providers that actually agree
  before a chunk is considered read, but do not stall forever on one that never does.

### Info

- **B3-I-1: the `MAX_SALES = 12` interface leftover.** `eth_floor_evidence` still accepts up to 12 sale hashes and ignores them
  (`ethnet.rs:450-461`, `ethnft.rs:70-73`). Harmless, but a runner or monitor author may believe the sale hand-in still matters.
  Consider dropping the parameter at the next `.did` break.
- **B3-I-2: the flood in B3-M-1 also inflates cycle spend.** Every flood transaction costs one receipt read (16/pass) plus the log
  chunks; a sustained flood pays the receipt-read budget continuously (bounded by 16/pass/collection, so not a drain on its own, but
  it compounds the B2-L-2 class of paid reads). The monitor's low-cycles alert covers it operationally; noted for completeness.

## 3. The fixer's re-runs, checked

- `review2/poc/B/audit2_nft_b.rs` does **not** compile unmodified at HEAD (the `EthIo` trait gained `eth_transfer_txs` and
  `eth_plan_call`; `floor_from` gained the `accepted` argument). The fixer's `fixes-round2-poc/audit2_nft_b.compile-adaptations.diff`
  supplies exactly those adaptations and nothing else (a fake `eth_transfer_txs` derived from the sale receipts, a fake
  `eth_plan_call`, and the `accepted` argument threaded through). I applied it verbatim and ran all five tests
  (`review3/poc/B/audit2_nft_b.rerun-on-HEAD.output.txt`): **0 of 5 pass**, and each fails on its ATTACK assertion, not a control:
  - `audit2_a_m1_…`: fails because the band's median is now the real 0.91, not the washer's 1.20.
  - `audit2_b2_m1_ten_dust_ping_pong_…`: fails because the module IS posted (0.93) every pass.
  - `audit2_b2_m1_seven_in_band_washes_…` and `audit2_b2_l1_…`: fail because the swept/washed book is now a HELD `Ok` post, not the
    `Err`/attacker-price the PoCs asserted.
  - `audit2_b2_l2_…`: fails because the oversized receipt is read once (5,0,0,0), not every pass.
  The fixer's `canister-pocs-rerun.output.txt` claim ("canister B 0 of 5 pass") is honest.
- The fixer's changes to `tests/ethnft.rs` and `tests/audit_nft.rs` are faithful, not weakened: `b_h_1` was **strengthened** (it now
  asserts the post equals the rank-2 listing 0.93, not merely "no post"); the `FLOOR0 = 0.47 ETH` change is required by the new
  advance gate (A2-M-1), not a floor weakening; the sale-block moves to `ETH_PIN-500` are because the canister now reads only final
  blocks (≤ pin−128); `b_m_1` gained a real receipt so the own-scan finds the bad hash. None of the round-1/round-2 regressions is
  made vacuous.

## 4. Carried-forward residuals (unchanged verdicts)

- The module-side burn cap (cap a fresh burn at the trailing-24 h low of `accepted`) was **not built** (`fixes-round2.md` §1.3, §10);
  the deposit-then-burn-after-a-real-rise residual remains, bounded by the day band, β, H11 and stage-0 custody. B3-M-1 makes an
  upward floor move *harder* (a flood holds it down), so it does not worsen this residual. Open, Low/residual as graded.
- The runner can still stop posts by handing in fewer than three listings (A-I-6). Unchanged.

## 5. Commands

```
# private copy under $S; toolchain 1.98.1 from ~/.cargo/bin
cargo test --test audit2_nft_b          # round-2 PoCs + fixer's compile-adaptations: 0 pass, 5 fail on the attack assertion
cargo test --test audit_nft_r2 b3_m1    # B3-M-1 control + attack + first-post variant: 3 pass (each shows the weakness)
```

PoC sources and outputs: `review3/poc/B/` (`b3_nft_flood.rs.snippet`, `b3_m1.output.txt`,
`audit2_nft_b.rerun-on-HEAD.output.txt`). Nothing was added to the worktree's test tree.
