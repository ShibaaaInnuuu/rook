# R4 check: services part (monitor, indexer, keeper, golive-check; the site as a consumer)

Checkout `/Users/jlohayon/Documents/Rook-rev4` at `fixes-2026-09-26` (`cf6ecda`); diff `4a215f4..fixes-2026-09-26`. No existing file
was changed. One exception, undone: `npm run build` in `web/` rewrites `web/src/content/trust-model.md` from `docs/trust-model.md`
(the committed web copy is stale; see I-S5). I restored it with `git checkout`. Network use was read-only: `cast code` on the
public 4663 RPC, and one forge fork of it.

PoCs (untracked, new files only):
- `monitor/test/audit4_monitor.test.mjs`: 11 tests (5 R3 re-runs, 6 new).
- `indexer/test/audit4_outbound.test.ts`: 6 tests (the R3 M-E re-run and 5 hostile server shapes).
- `keeper/test/audit4_services.test.ts`: 3 tests (L-G).
- `test/audit4/Audit4SafeHidden.t.sol`: 1 test. It is opt-in: `AUDIT4_FORK=1 forge test --match-contract Audit4SafeHidden`. Without
  the variable it returns at once, so the offline `forge test` is unaffected. It was developed and run in a scratch project,
  `R4/safepoc/`, with the repo's compiler settings (solc 0.8.28, via_ir).

## 1. Round-3 services items

| ID | Verdict | Evidence |
| --- | --- | --- |
| **M-A** (span ratchets to 1 block) | **Fixed** | R3's PoC re-run (`audit4_monitor` test 1): after 13 `header not found` errors the ceiling stays at 10,000, and 600 new blocks cost **1** call (R3: 600). The span narrows only on `rangeError`. `refresh()` restores the ceiling after 10 min, and `ok()` grows it back. A non-range error costs at most 3 tries, then a cool-down. |
| **M-B** (one dense block stalls the group) | **Fixed for R3's case** (new residuals **N-S1**, **N-S2**) | R3's fake re-run (test 2): the group and the unrelated coin reach the safe head. **But** in that fake the coin's dense-block logs are *skipped*, not read: N-S1. A dense block whose receipts call fails with a **retryable** error still stalls the whole group: N-S2. |
| **M-E** (`HTTP 101` hangs the fetch) | **Fixed** | R3's PoC re-run through `guardedFetch`: rejects in 26 ms (`HTTP 101 (a protocol switch)`). A 101 held open on the raw transport with no signal also rejects. Headers never sent, a body trickling 1 byte/s, and endless `102 Processing`: each rejects at the 3 s deadline. The server sees **0 open sockets** afterwards in every case, and the process lives. The `inflight` bound (60 s) and the catalogue's 15-min refresh bound are right. Builds are not amplified: at most one new build per collection per 60 s. |
| **L-C** (floor walk evaded by no-move posts) | **Fixed** (residual I-S1) | R3's interleaved walk now pages the edge run and triggers `pause_on_floor_walk`. R3's −0.439 %/h walk pages SEV1 on the 72 h rule. |
| **L-F** (the index never self-repairs) | **Fixed** | R3's lossy index, driven through `evidenceOf`: the dissent triggers `repair`, the rebuilt index reproduces the honest root, and the dissent is dropped. Reconciliation (`totalSupply` plus the top 16 `balanceOf`) and the `resetAt`/`brokenAt` expiry work as designed (the fix's own tests plus a read). Outsiders cannot trigger rebuilds: every trigger needs a lossy or lying provider, or a carried hold. There is a latency residual: **N-S4**. |
| **L-D** (resolver caches one mismatched rebuild for good) | **Fixed** | `rebuilt` is `{entry, retryAt, tries}` with a 10 min to 6 h backoff, and `LogCache.forget` drops the cached logs before the retry. The audit3 regression now really mismatches. |
| **L-G** (one-pass alerts never paged) | **Fixed at the default interval** (residual **L-S2**) | At `KEEPER_INTERVAL_S` 60 a one-pass alert is 503 for 18 min, so it is paged. |
| **L-H** (golive-check "real Safe" check) | **Not fully fixed**: **L-S1** | `--safe-owners` is now required under `--phase1`, and the fallback handler and the SafeSetup initializer are read. But a Safe whose storage was planted by its **creation code** passes every read (fork PoC below). |
| I-1 | Fixed | The deadline and the guardian now run before the dispute-limit throw. |
| I-2, I-3 | Open, as documented | — |

Consumers of the R4 contract views: `web-rook` `Books.tsx` (`keeperDown = burnsOpen && !fresh && !escaped`), the indexer's
`burnsOpen`/`pauseFloorWei`/`refundsHeldUntil` (read tolerantly from a pre-R4 module), and the monitor's keeper-paused page
(quotes `pauseFloor()`). All three match the contract semantics in fixes-R4-contracts §6. No issue found.

## 2. New findings

### Low

**L-S1: golive-check's real-Safe check is still defeatable by whoever created the Safe, through its creation code instead of
`setup()`.**
- **Where:** `script/golive-check.sh:125-165` (`safecheck`).
- **Mechanism:**
  - The check accepts any account whose runtime hash is a canonical SafeProxy's.
  - It trusts the SafeSetup event's `initializer == 0` as proof that nothing wrote hidden state.
  - But creation code can SSTORE anything (slot 0, `modules[x]`, `owners[x]`) and then RETURN the canonical proxy runtime. A later
    ordinary `setup(to = 0)` emits a clean SafeSetup, because setup's checks (GS100 on the sentinel, GS200 on the threshold, GS204 on
    the listed owners) never look at other mapping keys.
- **PoC:** `test/audit4/Audit4SafeHidden.t.sol` (read-only fork of 4663, canonical SafeL2 1.4.1, canonical 1.4.1 handler):
  **PASS, the attack works.** Every read `safecheck` makes passes:
  - the runtime hash is in `SAFE_PROXY_HASHES`;
  - slot 0 is the canonical singleton;
  - `getModulesPaginated` is `[]`;
  - there is no guard;
  - the fallback handler is canonical;
  - there is exactly one SafeSetup, with `initializer` zero;
  - the Safe is 2-of-2 with code-less owners equal to `--safe-owners`.

  Yet `isModuleEnabled(thief)` is true, and the thief's `execTransactionFromModule` drains the Safe's 10 ETH with no signature
  (it could equally call the registry as owner).
- **Severity:** Low, the same actor class as R3's L-H (whoever deploys the Safe, or a compromised deploy tool). It is still a
  go-live gate that says "real Safe" and is wrong.
- **Fix:**
  - Require the Safe to come from a canonical SafeProxyFactory. Read its `ProxyCreation` event: 1.4.1
    `0x4e1DCf7AD4e460CfD30791CCC4F9c8a4f820ec67`, 1.3.0 `0xa6B71E26…6AB2` and its eip155 variant. The canonical proxy constructor
    only writes slot 0.
  - Require the SafeSetup to be in the **same transaction** as the ProxyCreation (the factory's `initializer`), so nothing runs in
    between.
  - Optionally, spot-check `isModuleEnabled` / `isOwner` for the role keys and the deployer.
- **Doc mismatch (Info):** the fix doc says the SafeSetup owners and threshold are compared with the chain's. The script extracts
  the threshold only to print it and never compares either.

**L-S2: the L-G latch is fixed only for short keeper intervals. At `KEEPER_INTERVAL_S` ≥ 300 (the allowed range is 15–1,800) a
one-pass alert is never paged.**
- **Where:** `keeper/service/status.ts:36-47` (`AlertLatch`); `config.ts:194` (a fixed 1,200 s default).
- **Mechanism:**
  - 503 needs 3 passes in force, so it starts at `t0 + 2I`.
  - The latch drops the alert at the first pass at or after `t0 + 1200 s`.
  - At I = 300 that leaves exactly 10 minutes of 503, and a 1-minute monitor poll sees under 10 minutes, so no page.
  - At I ≥ 600 the pass count never reaches 3.
- **PoC:** `keeper/test/audit4_services.test.ts`, **PASS** (gap present):
  - at 60 s the alert is paged;
  - at 300 s it is not paged;
  - at 600, 900 and 1,800 s it is never 503 at all.
- **Fix:** latch at least `(KEEPER_ALERT_PASSES + 1) × interval + DOWN_MS`, or reject a latch shorter than that at config time.

**L-S3: the monitor's dense-block fallback can skip a block's logs silently (N-S1), and a transient-looking failure there stalls
the whole group again (N-S2).**
- **Where:** `monitor/src/holders.mjs:95-112`.
- **N-S1 (skip):**
  - `perTx` does `txs ??= (…eth_getBlockByNumber…)?.transactions ?? []`. A `null` block (a lagging load-balanced backend) or a
    header without `transactions` is taken as an **empty block**.
  - The dense coin's logs in block X are dropped, its cursor passes X, and nothing is marked `stuck`.
  - This contradicts "Nothing is skipped" (the docblock at `:140`).
  - The trigger chain: `getLogs(X)` refused for size, then `eth_getBlockReceipts` failing non-retryably (unsupported, or refused
    for size), then the one-address query refused, then a null header.
  - The `eth_getBlockReceipts` path has the same shape: an answer of `[]` for a block the provider just said has too many logs is
    accepted as "no logs".
  - **PoC:** `audit4_monitor` tests 6 and 7, **PASS**. R3's own M-B fake (header without `transactions`) with 51 value-bearing
    transfers X→Y. Coin A's cursor reaches the head, no coin is stuck, and the index says X holds 100 and Y 0 (on chain: 49 and 51).
    The same happens with a `null` header.
  - The index wrongly computes "other" for that coin's draws until the L-F repair or reconciliation catches it. Pause evidence
    comes from a wrong index: a dissent against an honest root, and with the other provider degraded that is a pause (§3.3).
- **N-S2 (stall):**
  - `denseBlock` rethrows a *retryable* error from `eth_getBlockReceipts` (`HTTP 5xx`, a timeout on a multi-MB answer).
  - `pagedLogs` then cools the provider down and returns before X, for the group.
  - Every poll repeats it: the page is refused, the block is read as dense, the receipts time out or 504, and the provider cools
    again (2 min doubling to 30 min).
  - **PoC:** `audit4_monitor` test 8, **PASS**: after 20 polls over 10 h the group and the unrelated coin B are still at X − 1, and
    `rootAt(coinB)` says "behind the snapshot block".
  - All of that provider's draws are "degraded", which since R4 means escalate, not pause. If both providers' gateways do this for
    the same block, **every** later draw is only escalated.
  - That weakens the §3.4 claim that a thief cannot cheaply make every provider degraded. Whether an attacker can force it depends
    on the providers' behaviour on a ~5–8 MB receipts answer (> 10,000 logs in one block), which was not measured.
- **Fix:**
  - Treat a null or malformed header as a throw (retry), never as `[]`.
  - Sanity-check receipts: the block hash and number, a non-empty result when `getLogs` said "too many", and optionally the number
    of receipts against the header's transaction count.
  - On a receipts failure of any kind, go on to the per-address split, so the non-dense addresses advance.
  - Keep only the addresses that fail every way behind (stuck, retried).

**L-S4 (N-S4): `reconcile()` ignores the poll's deadline inside a coin, and runs before the deadline pause.**
- **Where:** `monitor/src/holders.mjs:399-429`, called from `evm.mjs:668` before the deadline block `:705-731`.
- **Mechanism:**
  - Per coin: 1 `totalSupply` read (each state URL in turn) plus up to 16 sequential `balanceOf` reads.
  - Each read is `rpc()` with the default 30 s timeout and 3 tries on a timeout. The deadline is checked only between coins.
  - A provider that answers `totalSupply` and then hangs costs 16 × ~90 s ≈ 24 min for one coin. With 2 coins and 2 indexes in
    parallel, one `checkEvm` is held for tens of minutes.
  - The draw-deadline pause and the guardian run only after it, so a bad root can be pinned during the stall. The monitor has no
    per-checker timeout (`main.mjs` `pollOnce`).
- **PoC:** `audit4_monitor` test 11, **PASS**: with a budget of one call (150 ms), `reconcile` makes 17+ sequential reads and takes
  more than 16 × 150 ms.
- **Fix:**
  - Pass `timeoutMs = min(30 s, deadline − now)` and `tries: 1` into these reads.
  - Check the deadline before each `balanceOf`.
  - Better: run reconciliation after the deadline and guardian block, or fire-and-forget with its own budget.

### Info

- **I-S1: a slow walk under every rule.**
  - A walk at −0.399 %/h (72 h ≈ −25 %, day ≈ −9 %) raises **no SEV1 and no pause trigger** for a week: −49 %, SEV2 only.
  - At −0.224 %/h (72 h ≈ −15 %) nothing pages at all: **−31 % in a week** (R3: −50 % with no page).
  - `audit4_monitor` test 9, PASS.
  - Inherent to heuristics; an honest bear market looks the same. The operator should know the numbers.
- **I-S2: a stale dissent pauses.**
  - A provider that dissented once is being rebuilt (so degraded). If the other provider is degraded at the deadline, the call is
    **pause**, though the rebuilt index has not confirmed anything (`deadlineCall` row 4; test 10).
  - Not outsider-controllable (it needs a lossy provider), and deliberate per §3.3 ("repaired or not").
  - With N-S1 it can come from a skipped dense block.
- **I-S3: a repair from launch on a narrow-span provider takes about a day and a half.** On drpc at the documented ~100-block span,
  a coin launched 30 days ago takes about 36 h to rebuild (about 22 min on Robinhood's 10k span). That provider is degraded for
  that coin meanwhile. Consider rebuilding from the last reconciled base rather than from launch.
- **I-S4: timeout-flavoured size answers never narrow the span.**
  - `retryable()` matches any "timeout" or "timed out" text. A provider that answers a heavy `eth_getLogs` range with
    `-32000 query timeout exceeded` (geth-style) is cooled down, and the same span is retried for ever.
  - Consider counting repeated timeouts at the same span as a size answer.
- **I-S5: the committed web copy of the trust model is stale.** `web/src/content/trust-model.md` differs from
  `docs/trust-model.md`, which `web/scripts/sync-content.mjs` copies on every build. A build dirties the tree. Commit the synced
  copy.
- **I-S6: the no-good-post pause is available to an outsider who can drag the keeper's confidence below the minimum.** With
  `pause_on_no_good_post_h`, an outsider who manipulates marketplace data so the keeper's confidence falls below the minimum gets a
  guardian pause after N hours. That is a liveness cost only, it is opt-in, and it is economically harmless after A3-M1.
- Checked, no finding:
  - `evidenceOf` / `deadlineCall`:
    - a thief's bad root is still `bad` when both providers read the coin;
    - a split verdict repairs both providers and pauses at the deadline;
    - dissents clear only on a match.
  - The per-address split and the "too many addresses" batching.
  - Keeping a stuck coin consistent (cursor X − 1, logs ≤ X − 1).
  - The factory stuck holding the group.
  - `retryable`/`rangeError` ordering ("rate limit", "too many requests").
  - The canary (`allowStuck` off).
  - The golive-check SafeSetup decoding (the word layout is right for 1.3.0 and 1.4.1).
  - The SafeToL2Setup code-hash check.
  - The `--safe-owners` requirement.

## 3. Suite results (this machine; with the audit4 files)

| Suite | Result |
| --- | --- |
| monitor `npm test` | **122 pass**, 0 fail (as shipped); 133 with `audit4_monitor` (11) |
| keeper `npm test` / typecheck | **134 pass** / clean (as shipped); 137 with `audit4_services` (3); typecheck clean with it |
| indexer `npm test` / typecheck | **132 pass** / clean (as shipped); 138 with `audit4_outbound` (6) |
| ops `npm test` / typecheck | 31 pass / clean |
| staging `npm test` / typecheck | 29 pass / clean |
| tools/snapshot `npm test` / `tsc` | 19 pass / clean |
| web-rook `npm test` / typecheck / build | 66 pass / clean / ok |
| web `npm test` / typecheck / build | 118 pass / clean / ok (the build rewrote `trust-model.md`, now restored; I-S5) |
| `scripts/timelock-watch.test.mjs` | 4 pass, 1 skipped (no local replica) |
| `scripts/audit2_golive_check.test.sh` | **49 ok**, exit 0 |
| `Audit4SafeHidden.t.sol` (`AUDIT4_FORK=1`, read-only fork of 4663) | 1 pass: the L-S1 attack works |

The dry run was not run here (the parent runs it).

**Cleanup:** I installed every package's `node_modules`. Deleted: `web`, `web-rook` and `indexer` `node_modules`, plus
`web/dist` and `web-rook/dist`. Left for the parent's dry run and fork test, to delete afterwards: `keeper`, `monitor`,
`staging`, `ops` and `tools/snapshot` `node_modules` (~550 MB). Scratch forge outputs deleted.

## 4. Verdict for the services part

**No new Critical, High or Medium.** The services' round-3 Mediums (M-A, M-B, M-E) are fixed. So are L-C, L-F, L-D and I-1. L-G is
fixed at the default interval (L-S2). L-H is partly fixed (L-S1).

New: 4 Low (L-S1 Safe creation-code bypass, L-S2 latch vs interval, L-S3 dense-block skip/stall, L-S4 reconcile blocks the
deadline) and 6 Info. None is blocking by R3's standard, but the following are recommended before go-live:
- **L-S1**: go-live gate correctness;
- **L-S3**: silent skip; a stall that weakens §3.4's "degraded is hard to force" argument;
- **L-S4**: the deadline pause can be delayed by a hanging provider.
