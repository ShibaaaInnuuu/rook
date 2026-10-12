# Round 7, part B (+P): the sale scan and the canister's Ethereum pair after fix round 6

Reviewer: independent AI re-review, 2026-09-28. Worktree `/Users/jlohayon/Documents/Rook-nft`, branch `nft-side` at `a78a542`
(delta `bb713fe..HEAD`, canister commit `2d747ba`; go-live `b4c3f55`, `2841f90`; monitor `7b9e8ba`).

**Scope:**
- `pin.rs`: `error_class` (`:199`), `busy_body` (`:219`), `json_rpc_norm_body` (the class, `:254`);
- `ethnet.rs`: `rate_limited` (`:49`), the three scan transforms (`:36-70`), `eth_sale_moves_whole` (`:442-468`); `nftnet.rs` `ask` /
  `one` (`:52-69`), which decide whose error a read returns;
- `ethflow.rs` `sale_scan` (`:1176-1598`): the lag rule (`:1233`), the forward walk's `Busy` / `Deep` (`:1253-1263`), the hole walk
  (`:1270-1294`), the walk back's `lost` and its trim (`:1305-1320`), the skipped-block retry (`:1415-1439`), the receipts' `rfail`
  (`:1474-1551`), `behind` (`:1578`); `scan_error` (`:1648`), `reanchor` (`:1671`), `skip_again`, `receipt_again`, `defer` (`:1715`),
  `block_sales` (`:1735`), `scan_read` (`:1783`);
- `ethnft.rs` `SALE_REANCHOR_LAG_BLOCKS`, `SALE_RECEIPT_*`, `SALE_SKIP_*`;
- where the pair is configured: `script/golive-check.sh` (`DEEP`, `depth_probe`, `:885-1011`), `scripts/golive-dryrun.sh:268`,
  `docs/v7/phase1-nft.md` §4.1 / §5.1, `deployments/launch/monitor-nft.json`; the monitor's scan pages `monitor/src/ethnft.mjs:17-18,
  146-159`.

Toolchain 1.98.1.

**Nothing in the worktree was modified except this report and `review7/poc/B/`, `review7/poc/P/`. Nothing was committed, pushed,
deployed or signed.**

- **Where the PoCs ran:** a private copy of `autobuyer/canister-rs` at `/tmp/rook-r7-B` (with `autobuyer/rust-toolchain.toml`).
  `diff -rq` showed it identical to the worktree before anything was added, and it was removed afterwards.
- **The harness:** `tests/audit_nft_r6.rs`'s fake world copied whole to `tests/r7_b.rs`, plus three switches
  (`poc/B/r7_b.vs_audit_nft_r6.diff`):
  - `deep_once`: log reads over a span are refused, N times, by one provider with words of the `deep` class, through the real
    transform;
  - `receipt_busy`: chosen receipts answered as a rate limit (the transform's own `busy_body`);
  - `diverge_pct`: a share of log reads rejected by the IC for divergent replica answers (some replicas 429, some a result).
- **Provider probe (P), read-only:** `poc/P/probe7.py`, output `probe7.output.txt`. No `eth_sendRawTransaction` of any kind was sent.

Severity scale and threat model: `../README.md`'s. A passing PoC shows the weakness is present.

## Verdict

- **B6-M-1 is closed.** The committed pair (`rpc.mevblocker.io` + `gateway.tenderly.co/public/mainnet`) serves logs and non-null
  receipts past the window. Re-probed: logs at 30 k / 60 k / 80 k and receipts at 30 k / 60 k on both. The forward walk no longer
  freezes: a day's lag or a `Deep` refusal re-anchors it, and the newest sales are read at once. The fixer's scenarios reproduce.
  **No new Medium.** Three residuals are Lows. All three are provider-conditional, and none produces a false sale.
  - **B7-L-1:** a single `deep`-worded answer from either provider gives up the rest of a re-anchor's hole for good. It is never
    retried, even once the provider serves it again.
  - **B7-L-2:** the `busy` class has no bound anywhere. One provider's rate-limit or quota words for chosen receipts hold every newer
    sale, with nothing the monitor pages on (B6-L-1 by another door).
  - **B7-L-3:** the new `eth-scan-behind` page practically never fires. The re-anchor keeps `behind` at or below 7,200 at every
    pass's end, and the page needs at least 7,200.
- **Classification, steered by a provider's words.** This is the direct question of scope item 1.
  - The class is computed per provider, before any cross-provider agreement. `nftnet::ask` returns the first provider's error as the
    read's error (`?` on each provider, `nftnet.rs:57-58`), so a class needs only ONE of the two providers.
  - The 13-replica consensus binds the words to what the provider actually sent. It does not stop the provider from choosing them.
  - An outsider cannot inject words. The request is built by the canister, and no probed error echoes attacker-chosen text: the
    Tenderly and MEV Blocker caps echo only a block range.
  - A malicious or compromised provider (or a backend that honestly says `archive` / `pruned`) can therefore:
    - send `deep` to have a span given up (B7-L-1);
    - send `busy` to hold receipts or walks indefinitely with no page (B7-L-2, B7-L-3);
    - send `limit` to have a range deferred. This is harmless: the range is refined later, as before.
  - It cannot make a sale be counted that did not happen, or count one twice. Before round 6, the same lie was `Other` and was
    retried or deferred and later read once the provider behaved.
  - This regresses the audit-R3 rule (R3-C2, `evmread.rs:224-229`: "substrings of a provider's JSON-RPC message used to count, which
    let one provider … give up a chosen ticket") for the sale scan. Low, because the scan's output is only the floor's evidence
    sample, which only shrinks.
- **Double counting: none found.** The hole walk, the forward walk, the walk back, the rescan and the refinement can overlap after a
  second re-anchor (B7-I-3). Every move is de-duplicated by its whole hash against the cache and within the pass
  (`ethflow.rs:1453-1456`), and a judged non-sale by its 8-byte prefix. A re-read costs cycles, never a second count.
- **The first post: not starved.** After a re-anchor, the forward walk reads the newest final chunk first. A persistent `busy` stalls
  it, but that is a provider outage (B7-L-3 is the missing page).
- **Word-list false positives (honest pair, measured):**
  - Both result caps classify `limit`.
  - Tenderly's "block range extends beyond current head block" (a lagging backend) and "invalid block range params" also classify
    `limit`, a deferral instead of a retry (B7-I-4).
  - MEV Blocker's answer for a 1,000,000-block range is `-32603 "service temporarily unavailable"`, no class: the 6-in-a-row
    catch-all handles it. It answers `[]` for blocks past its head.
  - The IC's own divergence reject contains `timeout:` and classifies `limit` (B7-I-1). No honest answer of the pair classifies
    `deep`.
- **B6-L-1, B6-L-2, B5-L-2: fixed** for what they reported (verdicts below).

## Counts (part B, open)

| Critical | High | Medium | Low | Info |
| ---: | ---: | ---: | ---: | ---: |
| 0 | 0 | 0 | 3 (B7-L-1, B7-L-2, B7-L-3) + B3-L-1 carried | 6 |

## B7-L-1 (Low, provider-conditional): one `deep` answer in the hole walk gives up the rest of the hole for good

**Code:**
- the hole walk `ethflow.rs:1284-1289`: on `ScanErr::Deep` it sets `sc.lost = (x, b)`, `sc.hole = None` and breaks;
- nothing ever reads `lost` again. The only trim is in the walk back (`:1305-1308`), which reads below `sc.down`, while a hole lies
  above it (between the old `sc.up` and the re-anchor point);
- the class comes from one provider (`nftnet.rs:57`, `pin.rs:254`).

**What happens:**
- After any gap over 7,200 blocks (the runner down a day, the cycles under the evidence floor, a pause, the day's scan budget spent),
  the passed-over blocks are a hole, read newest first.
- If any read there gets ONE `deep`-class answer from ONE provider, everything below that read's top is recorded `lost` and dropped.
  The trigger can be a transient backend without the history (`archive`, `pruned`, geth's `4444 "pruned history unavailable"`), or a
  provider that lies.
- The fix's claim "a later successful read trims `lost` (a provider back to depth)" does not hold for a hole's span: it is never read
  again while in the window.
- A lying provider can also choose the span:
  - near the head, a `Deep` is only a failed read (`:1261`), not counted toward `stuck`;
  - so refusing the target range stalls the forward walk until it is 2,000 blocks behind (`:1258`, about 7 h, no new sale counted);
  - that re-anchors, the target joins the hole, and one more `deep` answer marks it lost.

**PoC:** `r7_b_l1_a_one_transient_deep_answer_in_the_hole_gives_up_the_rest_of_the_hole_for_good`. A 12,000-block gap on a deep pair,
119 honest sales inside the gap:

| Scenario | Gap sales counted after two days | `lost` |
| --- | --- | --- |
| control | 119 / 119 | none |
| one provider answers `4444 "pruned history unavailable"` ONCE on one hole read; every later read of both is served | **88 / 119** | `(26068801, 26071900)`, never re-read |

**Effect:** the floor's 7-day sample loses up to the rest of a hole (a day or more of sales). The page `eth-scan-lost` says "replace
the shallow provider" when the providers are deep. With a lying provider the sample can be biased, since chosen sales are dropped.
Nothing false is counted, and nothing is signed from it.

**Fix:**
- Treat a hole walk's `Deep` like the walk back's: record it and break for the pass, keep the hole, and retry next pass.
- Give up a span only after repeated `Deep` over a time bound (for example a day), then trim `lost` on any later successful read of
  it.
- Alternatively, require both providers to answer `deep` (ask the second provider when the first refuses) before a span is given up.

## B7-L-2 (Low, provider-conditional): `busy` has no bound: rate-limit or quota words hold receipts (and walks) indefinitely, unpaged

**Code:**
- receipts: `ethflow.rs:1518-1522`. A `Busy` receipt goes back to the queue with `failed = false`: no `rfail` entry, no
  `SALE_RECEIPT_RETRY_READS` share, no back-off;
- the walks: `:1255`, `:1283`, `:1314`, `:1353` break on `Busy` with no count;
- the skipped retry: `:1433-1434` keeps a busy block due at `now`, first in line;
- the class comes from the words (`busy`: "rate limit", "quota", "capacity", "busy", "try again later", …) or an HTTP 429
  (`ethnet.rs:49-53`), from one provider.

**What happens:**
- `sale_order` puts the oldest moves first, and a busy one keeps its place.
- With 16 or more moves whose receipt one provider answers with rate-limit words, or a 429, every pass's 16 reads go to them and no
  newer receipt is judged. That is B6-L-1's head-of-line block, now through `busy` instead of `null`.
- The honest variant needs no lie. A keyless tier with a per-method or daily quota on `eth_getTransactionReceipt` ("quota exceeded"
  → `busy`) stops every receipt, while the log reads, and so `behind`, `hole` and `lost`, look healthy.
- No scan page fires: `rfail` is empty, `behind` is 0, and there is no hole or lost span.

**PoC:** `r7_b_l1_b_a_busy_answer_is_unbounded_16_receipts_hold_every_newer_sale_with_no_page`. 16 old moves whose receipts are
answered busy: after two days, **0 of 144** final newer sales are read; `rfail` None, `behind` 0, `lost` / `hole` None. The fixer's
control with `null` answers reads 144 / 144.

**Effect:** no new sale is counted (rises held; no first post for a new collection) with no scan page, while one provider, or one
provider's quota, says `busy`. Nothing false is counted.

**Fix:**
- Count busy answers per receipt, and per walk start, over time. Past a bound (for example 3 passes), treat them like failures: the
  `rfail` share and back-off, and the walks' `stuck`.
- Page a scan whose counted sales or judged receipts have not advanced for N hours whatever the reason (the monitor has the state:
  `sale_cache`, `pend`).

## B7-L-3 (Low): the scan-lag page practically never fires

**Code:**
- `ethflow.rs:1233`: at a pass's start, `top > sc.up + 7_200` re-anchors, so `top - sc.up <= 7_200` afterwards;
- `:1578`: `behind = top - sc.up` at the pass's end;
- `monitor/src/ethnft.mjs:18,150`: `eth-scan-behind` needs `behind >= SCAN_BEHIND_BLOCKS` (7,200).

**What happens:**
- While passes run, `behind` is at most 7,200 at every pass's end. It is exactly 7,200 only when the pin's step happens to land on it.
- While passes do not run (the runner down, the cycles under the evidence floor: the page's own listed causes), `behind` is not
  rewritten at all and keeps its last small value.
- So the SEV2 "a day behind … no new sale is counted" page, which the fixer's judgment call 1 names as the watch on the pair, fires
  only by chance.
- A total `busy` or `Other` outage of either provider pages only the non-persistent SEV3 `eth-scan-hole`.

**PoC:** `r7_b_l2_the_scan_lag_page_cannot_fire_the_reanchor_keeps_behind_under_a_day`. Three days with every log read busy and the
pin stepping 97–103 blocks a pass: 0 of 216 newer sales counted; `behind` at most **7,200**; the page's condition held on **1 of 216
passes** (one 20-minute window in three days); `hole` set (SEV3 only).

**Fix:**
- Page on the scan's own progress, not on the post-re-anchor lag. Options:
  - save the time of the forward walk's last successful read (or a re-anchor count) and page when it is over N hours;
  - page `hole` persistently when it has not shrunk for a day;
  - compute the lag in the monitor from the Ethereum head and `sc.up`.
- Lower `SCAN_BEHIND_BLOCKS` well under `SALE_REANCHOR_LAG_BLOCKS` (for example 2,400, 8 h).

## Round-6 items: verdicts

| ID | Verdict | Basis, residuals |
| --- | --- | --- |
| **B6-M-1** | **Closed** | (1) **The pair.** It is configured in the dry run's install argument (`golive-dryrun.sh:268`), golive-check's `DEEP` list (`:891-892`), `phase1-nft.md` §4.1 / §5.1 / §11.13, and nowhere else. The committed monitor configuration holds a placeholder argument, and `evm_chains` has no default Ethereum provider. It was re-probed (below). (2) **The transforms.** They keep a class, and `r6_b6_m1_b` holds it through the real transforms. (3) **The re-anchor.** It removes the freeze: `r6_b6_m1_a` reproduces (first newer sale at pass ≤ 3, a day's sales read, lag at the head). (4) **golive-check.** It refuses an unlisted chain-1 provider under `--phase1`; `--probe-depth` fails a provider that errors or returns a null receipt 60,000 blocks back. Residuals: B7-L-1, B7-L-2, B7-L-3; B7-I-2 (a hole's `lost` erased by the walk back's trim); B7-I-5 (the probe accepts an empty log list). The IC-side facts are still the operator's rehearsal check (below) |
| **B6-L-1** | **Closed** for failures (`null`, disagreement) | `rfail` is keyed by the whole hash. There are 3 fast tries, then 6–24 h back-off, and at most 4 of 16 reads are retries. `r6_b6_l1` reproduces (144 / 144, the 16 kept and read once served). **Residual:** `busy` bypasses it (B7-L-2) |
| **B6-L-2** | **Closed** as reported | A per-query `busy` never defers, splits or gives up. `done_big` is only marked on a two-sided answer (`block_sales` `:1746`, the retry `:1420`). The refinement's split goes through `defer`, and `raw` ranges are evicted last. `r6_b6_l2_a` reproduces (432 / 432 at 0 / 30 / 50 %). **Residual (B7-I-1):** a rate limit that splits a replica set is the IC's divergence reject, which says `timeout:` and so is `TooBig`. The fix's "a rate limit no longer defers anything" is not true of it, but measured (`r7_b_i1_a`) nothing is lost: 432 / 432 at 30 % and 50 %, `bad` ≤ 32 |
| **B5-L-2** | **Closed** | A skipped block is never given up in the window (`skip_again`: 6 / 12 / 18 h, then every 18 h). One skipped retry a pass, only when due, inside the pass's and the day's byte bounds. A one-sided answer counts what it read and stays skipped. `r6_b5_l2` reproduces. The padded-fill block is still unreadable for Seaport there but retried (the fixer's call 8, accepted below). Note B7-I-6 |
| **B3-L-1** (carried) | Low, unchanged | A disagreement on the fallback read: the block is skipped (now retried for the window, not given up) |

## The fixer's judgment calls (scan and pair)

| Call | Assessment |
| --- | --- |
| **1. MEV Blocker + Tenderly as the canister's pair** | **Accept, with the rehearsal check as a hard precondition.** Both are deep (probe below). Every Ethereum read of the canister (floor, buys, deliveries, deposits, the scan) needs BOTH (`nftnet::one`). Tenderly is IPv4-only, so the IC reaches it only through DFINITY's SOCKS fallback, which the repo's own `docs/v4/eth-nft-canister.md:24-35` called "a second path to trust" when it rejected Tenderly as `(IPv4)`. Consequences: (a) if the fallback fails on the subnet, every Ethereum read fails: the §5.1 IC-outcall rehearsal is not optional; (b) through a few proxy addresses Tenderly's per-IP limit (12 / 260 429s in the fixer's one-IP burst) is likelier to split replica sets, which is B7-I-1's harmless-but-TooBig path. The watch the call names is not there: `eth-scan-behind` is B7-L-3. Sends: MEV Blocker's private orderflow is part F's |
| **2. Re-anchor at a day's lag, gap read newest first** | **Accept.** It is the right shape: the newest sales count first, and the old ones are read as the budget allows. Fix B7-L-1 (the `Deep` give-up) and B7-L-3 (the page) with it |
| **8. A skipped oversized block retried for the window (at most every 18 h)** | **Accept.** At most one skipped retry a pass, only when due, after every walk, inside the day's byte bound (`SALE_SCAN_BYTES_PER_DAY`). It cannot starve a walk, and its cycles are inside B5-L-1's accepted ceiling. The list itself has no cap (B7-I-6) |

## Providers (part P): the committed pair, re-probed

Read-only, 2026-09-28 13:21 UTC (`poc/P/probe7.py`, `probe7.output.txt`). Every error is classified with a copy of
`pin::error_class`.

| Probe | `rpc.mevblocker.io` | `gateway.tenderly.co/public/mainnet` |
| --- | --- | --- |
| DNS | A + AAAA | **A only** |
| Milady `Transfer` `eth_getLogs`, 500 blocks at 30 k / 60 k / 80 k deep | 2 / 4 / 0 logs | the same |
| a receipt at 30 k / 60 k | served | served |
| USDT 1,000 blocks (result cap) | `-32005 "query returned more than 10000 results …"` → `limit` | `-32602 "invalid params"` + data "more than 20000 results" → `limit` |
| Milady 1,000,000-block range | `-32603 "service temporarily unavailable"` → none (`Other`: the 6-in-a-row catch-all) | answered (4,777 logs, 3 MB: over the IC's 2 MB class → the IC's size reject, `TooBig`) |
| `fromBlock > toBlock`; a range past the head | `[]`; `[]` | `-32602 "invalid block range params"`; `"block range extends beyond current head block"` → `limit` |
| malformed topic / hash; unknown method | `-32602` / `-32601`, no class | the same |
| an unknown receipt | `null` | `null` |

**Reading:**
- Depth holds with margin on both.
- No honest answer of the pair classifies `deep`, so B7-L-1 needs a backend change or a lie.
- A Tenderly backend lagging behind the canister's pin says `limit` (a deferral, harmless).
- MEV Blocker answers `[]` for blocks past its head. The canister's 2-of-2 agreement turns a lagging MEV Blocker into a disagreement,
  never a silent empty read.
- No `eth_sendRawTransaction` was sent.

## Infos

- **B7-I-1:** the IC's reject for divergent replica answers ("No consensus could be reached. Replicas had different responses.
  Details: request_id: …, timeout: …, hashes: […]") and its "Timeout expired" both classify `limit` through `error_class`'s `timeout`,
  so a replica-split rate limit is `TooBig`, not `Busy`. This was so in round 5 too. `r7_b_i1_a` measures no loss (432 / 432 at 30 %
  and 50 %, `bad` ≤ 32). Fix: classify the IC's divergence text as `busy` (it is a transient disagreement), before the word lists.
- **B7-I-2:** the walk back's trim (`ethflow.rs:1305-1308`) resets `lost` to `(p0, a-1)` or `None` on any successful read below
  `sc.down`. A hole's lost span lies above `sc.down`, so the trim erases its record (and its page) while its sales stay uncounted.
  This happens when a walk back runs at the same time: at launch, after a window reset, or while a walk back is itself `Deep`. The two
  spans are also merged into one min/max range. Keep a list of lost spans, and trim only what was read.
- **B7-I-3:** a second re-anchor merges holes by min/max (`reanchor` `:1675`), so blocks the forward walk read between the two are read
  again by the hole walk. The cost is cycles only (de-duplicated by hash), and it is rare: a 7,200-block hole is read in about one
  pass.
- **B7-I-4:** Tenderly's "block range extends beyond current head block" and "invalid block range params" contain `block range` →
  `limit`: a lagging backend defers a range instead of retrying it. It is harmless (refined later). The word list could exclude
  "beyond current head".
- **B7-I-5:** `golive-check --probe-depth` accepts an empty `eth_getLogs` list 60,000 blocks back (`golive-check.sh:915`; at 80 k our
  probe range held 0 logs). A provider that silently answers `[]` for a pruned range passes if its receipts are served. Compare the two
  providers' answers over a range known to hold logs (for example 10,000 Milady blocks: 44 logs measured by the fixer), and fail on
  any difference or an empty list. Also, the allowlist matches host and path but not scheme or port (`http://…` or `:8443` pass): the
  canister fails closed on them, but the check should refuse them.
- **B7-I-6:** `SaleScan.skipped` has no size cap (`bad` and `done_big` have one), and every pass's note lists all of them
  (`:1440-1443`). Each entry needs an oversized block of the collection (B5's price), so growth is expensive. Cap the list or the note.

## Recommended fixes, in order

1. **B7-L-3 and B7-L-2 together:** a scan-progress page that does not depend on `behind` (time since the forward walk's last read,
   counted sales not advancing), and a bound on `busy` so it becomes a counted failure after a few passes.
2. **B7-L-1:** never give up a hole span on one answer; retry it, and trim `lost` when read (B7-I-2).
3. B7-I-1 (IC divergence as `busy`), B7-I-5 (probe compares providers), B7-I-4, B7-I-6.
4. **Operator precondition (unchanged from round 6):** the §5.1 rehearsal from a real IC outcall on the launch subnet, both providers
   through the transforms, including Tenderly over the IPv4 fallback. If Tenderly is unreachable there, every Ethereum read fails.
