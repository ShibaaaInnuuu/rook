# Rook Phase 1, second review (R2): off-chain services, site and go-live scripts

**Checkout:** `/Users/jlohayon/Documents/Rook-rev2-services`, detached at `fixes-2026-09-26` = `46a4f1e`. The fixes were compared
against the first review's commit `e04f49b` (`E-services-workflow.md`).

**What was touched:**
- No existing source or doc was modified. Nothing was committed, pushed, deployed or signed.
- No key, wallet or Railway service was touched, and no `.env` was read.
- Network use was read-only: public 4663 RPC calls, and the public drand relays through the fork test.
- Foundry libs were copied into `lib/` from `~/Documents/Rook/lib`, with each copied `.git` renamed to `.git.bak-copied`.

**Tests added (all untracked, and all passing):** each one asserts the current behaviour, so a pass means the gap is present.
- `keeper/test/audit2_services.test.ts` (8);
- `monitor/test/audit2_monitor.test.mjs` (5);
- `ops/test/audit2_ops.test.ts` (2);
- `tools/snapshot/test/audit2_monitor_tree.test.ts` (1);
- `indexer/test/audit2_outbound.test.ts` (3);
- `web-rook/test/audit2_e07.test.ts` (2);
- `scripts/audit2_golive_check.test.sh` (6 cases).

**Supporting part reports, all in this folder:**
- `part-site-indexer.md`;
- `part-monitor-ops.md`;
- `part-golive.md`;
- logs `dryrun-stack.log` and `keeper-fork.log`.

## 0. Summary

**Original area-E findings (16).**

| Verdict | Count | Findings |
| --- | --- | --- |
| Fixed | 9 | E-01, E-03, E-04, E-06, E-07, E-09, E-10, E-12, E-13 |
| Fixed, with a new gap in its own path | 3 | E-08, E-14, E-15 |
| Partially fixed | 4 | E-02 (off-chain), E-05, E-11, E-16 |

- E-03's resolver and E-10's re-pricing have new weaknesses of their own, listed in §2.
- E-14: three of its four items are fixed; the fourth, a rehearsal-price test, was not addressed.
- E-15: items 1–4, 6 and 7 are fixed; item 5 is not.

**Off-chain parts of other areas.**

| Verdict | Findings |
| --- | --- |
| Fixed | C-07; B-05, but only when the stack file is passed (§2, G-N1) |
| Partially fixed | C-03 (the monitor half), C-08 |

**New findings.**

| Severity | Count |
| --- | --- |
| High | 1 |
| Medium | 5 |
| Low | 14 |
| Info | 9 |

- **The one High (R2-M1):** the independent draw-root recompute can't finish on the configured public RPC pair, so the guardian's
  auto-cancel doesn't work in practice.
- **The five Mediums:**
  - a queue of draws that can't be verified starves every later draw of its check;
  - a floor walked exactly along the edge of `floorBounds()` is never flagged;
  - the indexer crashes on an HTTP status of 600–999;
  - a published file with forged entries blocks the resolver's claim forever;
  - the keeper's own alerts are never paged.

**Runs.** Every package's suite, typecheck and build is green:

| Package | Tests | Typecheck |
| --- | --- | --- |
| keeper | 123 (115 + 8 new) | clean |
| tools/snapshot | 18 | clean |
| monitor | 84 | — |
| ops | 30 | clean |
| indexer | 111 + 3 | clean |
| web-rook | 57 + 2 | clean; both builds clean |
| staging | 29 | clean |
| `scripts/timelock-watch.test.mjs` | 5 | — |

- `scripts/golive-dryrun.sh stack`: **ALL PASS**.
- `scripts/keeper-fork-test.sh`: **ALL PASS**, 43 ok, with real drand.

## 1. Original findings

| ID | Verdict | Evidence and why |
| --- | --- | --- |
| **E-01** wash sales drag the floor down | **Fixed**, but see R2-S4 | `S-item-repeat` counts one sale per item, and `maxSalePullBps` = 5% caps how far sales from one marketplace can pull (`keeper/src/floor.ts:257-280`). The audit's attack (11 pairs at ⌈L/4⌉+1) now moves the floor at most 5%; one churned frog counts once (`w2-hardening.test.ts`). The other direction got worse: see R2-S4. Listing freshness on Ordinals Wallet (`verifiedAt: fetchedAt`) and the in-memory `BookDiff` are unchanged. |
| **E-02** hot keeper key (off-chain halves) | **Partially fixed** | **Keeper: fixed.** `needsRepair` re-posts when `!fresh()` or confidence is under the minimum (`floorpost.ts:34-42`); the audit's PoC is inverted (0 of 180 minutes stale). The pause is read every pass and before every signature. **Monitoring: partial.** FloorPosted, DrawOpened, KeeperPaused, GuardianSet and RolesProposed are watched, and the root is recomputed. But R2-M1, R2-M2 and R2-M3 undo this in practice. **Not done:** a KMS signer (recorded as open). |
| **E-03** nothing resolves | **Fixed**, with R2-S1 and R2-S2 | `service/resolver.ts` walks tickets in order, and draws through request, resolve, claim and void. There is a standalone `resolver-main.ts` with its own key, which refuses the named keeper. The site has Resolve, Refund and Claim buttons. The fork test resolves and claims with real drand. New weaknesses: R2-S1 (a forged file blocks the claim and the void), R2-S2 (one lying relay stops everything). |
| **E-04** unbounded pass, no RPC failover | **Fixed** (keeper) | `CoinIndex` indexes at most 200 coins a pass. `DrawBook` walks from the lowest draw that isn't final. Floors and caps run first; draws and the resolver are time-boxed (`KEEPER_PASS_BUDGET_S`). `ChainRpc` fails over on 429, -32005 and -32016, then backs off for two more rounds; `sendRaw` does too. The monitor's `rpc()` and the reconciler's `pinnedRpc` still don't fail over on these codes (part-monitor-ops I-1 and I-3; related to R2-M1). |
| **E-05** reserves green while unbacked or stale | **Partially fixed** | `ops/src/backing.ts` checks the module's `eth_getBalance` against its full books, units, and backing per unit ≥ 50% of the floor; the original PoC is now RED. But the stale rule keys on `postedAt`, not `lastGoodAt` (`backing.ts:83-84`), so a module frozen by confidence-0 posts stays GREEN (R2-L4). The monitor does page this case (`no-good-post`). |
| **E-06** canary silenced by one disagreement | **Fixed** | A disputed range is kept (at most 32), the agreed logs in it are judged, the cursor moves on, the alert repeats every 6 h with "STILL" after 1 h, and `/healthz` degrades after 1 h. The audit's PoC is inverted. Caveat: an RPC that *errors* (-32005) still stops the whole 4663 checker. It is paged as `checker-down` after 30 minutes (see R2-M1). |
| **E-07** send guard checks only the target | **Fixed**, residual N-2 (Low) | `requestRefusal` (`web-rook/src/rook/phase.ts:219-229`): an allow-list of functions per kind of target, and every address argument (tuples and arrays included) is denied unless a rule allows it. The router, wrapper, factory and raffle vault come from the bundled stack file. All 13 write sites pass; none is wrongly refused. |
| **E-08** monitoring gaps, fail-open alerting | **Fixed**, with R2-S8 and R2-L5 | Now paged: service liveness, `fresh`, `capFresh`, `lastGoodAt`, `escaped`, `keeperPaused`, UniSat missing, late tickets and draws, and "unverified" for more than 6 h. Telegram refuses to start without its variables (monitor and reconciler), and the reconciler has its own `--test-alert`. Gaps: the keeper's own pass alerts are never paged (R2-S8), and "cannot read anything" is not paged at once (R2-L5). |
| **E-09** rate direction | **Fixed** | `AgreedRate` returns the higher sats/ETH (`feed.ts:53`), and the doc (`p1-services.md:108`) is corrected. |
| **E-10** no fee bump | **Fixed**, with a new bug R2-S3 | The pending record keeps every attempt, and any attempt's receipt counts. It re-prices at `max(old × 1.125 + 1, 2 × base + tip)`, or cancels with a self-transfer, within the caps and never while paused. `tx-stuck` and `tx-replaced` alerts, but see R2-S8. New bug: a replacement after a cancel reuses 21,000 gas (R2-S3). |
| **E-11** fragile snapshots | **Partially fixed** | Fixed: adaptive splitting (on "exceeds limit" etc.); failures throw and raise an alert; an incremental log cache capped at confirmed blocks; the file is read back byte for byte before `openDraw`; the resolver can rebuild from chain. Partial: the draw's URI still defaults to the **keeper's host** even with a mirror configured, and the read-back only checks that URI (R2-S5). The "alert on a skipped slot" is a pass-report alert that nobody pages (R2-S8). |
| **E-12** SSRF guard hostname-only | **Fixed**, new crash N-1 | `indexer/src/outbound.ts`: v6 is an allow-list (2000::/3 minus special ranges); IPv4-mapped, compatible, NAT64 and 6to4 forms, and decimal, octal and hex literals are all refused. DNS is pinned in the socket's `lookup`, redirects are manual and re-checked, and bodies are capped. The new transport crashes the process on a status outside 200–599 (N-1). |
| **E-13** single-provider location | **Fixed** | `ops/src/location.ts`: ordinals.com is believed only when Ordinals Wallet names the same address and satpoint. The indexer's own reserves check is still single-provider; that path is not what the go/no-go reads. |
| **E-14** small keeper hardening | **Fixed** (3 of 4) | Fixed: `readExclusions` fails closed; the public URLs, mirror, base URL and relays must be https; `/status` is a public summary; `/status/full` needs a bearer token of at least 32 characters, compared in constant time; the token and upload credential are scrubbed from logs. Not addressed: a rehearsal floor above `MAX_PURCHASE_WEI` (0.5 ETH) has no test (Info). |
| **E-15** runbook and code mismatches | **Fixed**, except item 5 | Items 1–4, 6 and 7 are corrected (`services-v1.md` §5, `launch-gates.md:410`, `p1-services.md` §1.4, the Redeem copy). **Item 5 is not fixed:** `web-rook/Dockerfile:1` still says `railway up ./web`, and a Docker build ships whatever is committed in `src/content`. The committed `web-rook/src/content/launch-gates.md` still has the old "Keeper key stolen" row (N-3). |
| **E-16** dependencies and images | **Partially fixed** | Fixed: web-rook `npm audit` is 21 moderate and 0 high (was 25, with 2 high: vite 7.3.6 and a `ws` override); the shipped dependencies are pinned; the monitor, reconciler and runner images run as `node` (checked in Docker, symlink-chown tested). **Not fixed: the keeper image still runs as root and installs dev dependencies** (`keeper/Dockerfile`, no `USER`; R2-S7). The same image runs the standalone resolver. The 21 moderates need wagmi 3 (left open). |
| **C-03** (off-chain half) | **Partially fixed** | Fixed: the indexer rebuilds the list with the sentinel and never trusts the file's proofs, and the site's `claimRefusal` re-reads `draw(id)`. The monitor's tree port equals `tools/snapshot` on 178 comparable random adversarial holder sets (R2-T1). But the recompute-and-cancel within the window is not achievable on the configured RPC pair (R2-M1), and draws starve each other (R2-M2). |
| **C-07** (off-chain half) | **Fixed** (given G-N1) | golive-check compares the split 9000/0/0/1000 and the loan 0 at stage 0 (`golive-check.sh:246-248`). The failing path is never exercised by the dry run (G-N5). |
| **C-08** golive-check gaps | **Partially fixed** | Fixed: under `--phase1` (forced on 4663 from `cast chain-id`), each step-7 note is a FAIL; `pendingOwner`, `pendingRoles`, `pendingTreasury`, `pendingBondAt`, `pendingRoot`, `pendingBtcVault`, `pendingCeiling` and `pendingMintLimit` all fail closed; the Safe's threshold and owner count; code, including 7702, at every role; the ceiling vs the stack file. **Still not read:** `FloorModule.payee()` / `MintDesk.payee()`; `maxEthBacked`, `maxPublicEthBacked`, `mintPerHour` and `burnPerHour` (they scale with the unverified `ETCH_STACK_ITEM_UNITS`, and the Trust page's "≈1.3 ETH/day" depends on `maxPublicEthBacked`); the listing's `paused` flag. New: G-N1, G-N2, G-N3. |
| **B-05** (off-chain half) | **Fixed** (given G-N1) | `module.wrapped()` and `module.registry()` are checked every run (`:298-299`); an unwired wrapper FAILs under `--phase1`. |
| **`ETCH_REHEARSE_UNMADE_OWNER_STEPS`** | **Safe** | `ownerSteps = local && !knob` with `local = isOurs(chainid)`, which is only 31337 or 9466301 (`DeployStack.s.sol:106`, `DeployChecks.sol:11,15,67`). On 4663 the knob can only turn "make" into "print", never the reverse, and golive-check forces `--phase1` from the chain id, not from the file. |

## 2. New findings

### High

#### R2-M1: the independent draw-root check, and so the guardian's auto-cancel, cannot work on the configured public RPC pair

**Where:** `monitor/src/draws.mjs:84-107`; `monitor/src/evm.mjs:236-241,262-275,320-327`; `monitor/src/abi.mjs:58-64`.

**What goes wrong:**
- Each recompute re-reads the coin's Transfer logs from the stack's first block, on **both** RPCs, every poll. There is no cache.
- It reads in 10k-block pages, with a budget of 5,000 pages.
- `rpc()` has no retry on 429 or -32005. `bothLogs` uses `Promise.all`, so one error from either RPC throws the whole 4663 checker.

**Measured today:**
- 4663 makes about 858k blocks a day.
- `rpc.ordofi.network` takes 7–11 s per 10k-block `eth_getLogs` for one client.
- For me, from this machine, it answered **every** `eth_getLogs` (spans of 10 to 200k blocks) with `-32005 "the network is busy"`.
- The Robinhood RPC serves 200k-block spans in about 0.1 s.

**Consequences:**
- Within one to two days of stack life, the recompute outlasts the 15-minute challenge window. After about 58 days it can never
  finish.
- With ordofi throttling, DrawOpened is never even read: no recompute, and no guardian action.
- The only page is `checker-down` after 30 minutes. By then a thief holding the keeper key has called `requestDraw`, and a pinned draw
  cannot be cancelled.
- The control that the E-02 and C-03 fixes and the Trust page rely on ("forged lists … cancellable until the pin") is not
  effective as deployed.

**Tests:** `monitor/test/audit2_monitor.test.mjs` R2-M1a (B throws "busy", so no recompute and no action) and R2-M1b (a 60-day
history makes 5,000 sequential calls, then throws). Both pass.

**Fix:**
- Cache logs incrementally, as `keeper/service/logs.ts` does.
- Retry and back off on throttle codes; record a chunk that can't be read instead of throwing.
- Use a second RPC that reliably serves `eth_getLogs`.
- **Fail closed:** `pauseKeeper()` when a draw is still unverified N minutes before `challengeEnds`, and pause first on a bad root.

### Medium

#### R2-M2: two draws that can't be recomputed starve every later draw of verification

**Where:** `monitor/src/evm.mjs:309`, which takes `slice(0, 2)` in insertion order; the "NOT verified" SEV2 at `:328-331` is raised
only inside that loop.

**What goes wrong:** a stuck draw holds its slot for 6 hours past its window. A forged draw queued behind two stuck ones gets no
recompute, no unverified page and no cancel.

**Test:** R2-M2 passes.

**Fix:**
- Order the queue by `challengeEnds`.
- Page every pending draw near its window.

#### R2-M3: a stolen key posting exactly at `floorBounds()` walks the floor with no alert

**Where:** `monitor/src/evm.mjs:132-145`. The only floor-walk rule is "target ≠ accepted by more than 50 bps".

**What goes wrong:**
- `floorBounds()` is public, so a thief posts `target = lo` and nothing is clamped. There is no SEV1 and no runaway trigger, the
  confidence is fine and the floor stays fresh.
- That allows −20% a day, which is the Trust page's "≈1.3 ETH/day" worst case, repeated daily, and nothing pages.

**Test:** R2-M3: 24 hourly edge posts move the floor −20% with 0 alerts. Passes.

**Fix:**
- Page on the size of moves over 24 h, or on K consecutive posts at the band's edge.
- Compare the posted floor with an independent floor reference read by the monitor.

#### N-1: an HTTP status outside 200–599 from any fetched URL crashes the indexer

**Where:** `indexer/src/outbound.ts:209-210`. `new Response(stream, { status })` is built inside the `https.request` response
listener. Node accepts statuses 600–999, and undici's `Response` throws a `RangeError` on them. The throw is outside the
promise, so it is an uncaught exception and the process exits.

**Who can trigger it:** anyone who controls a URL the indexer fetches:
- a tokenURI or a picture host;
- **a draw's on-chain `uri`** (the keeper's word, or the snapshot mirror's host).

**Impact:** the verdict cache is in memory, so after a restart the same list is fetched again. The indexer can crash-loop, and the
site loses its data source.

**Test:** `indexer/test/audit2_outbound.test.ts` test 3: a local TLS server answering `HTTP/1.1 999` kills the child process.
Passes.

**Fix:** validate the status, and wrap the listener body in try/catch that rejects the fetch.

#### R2-S1: a published file with forged entries blocks the resolver's claim for good, and its void too

**Where:** `keeper/service/resolver.ts:117-123,130-133`.

**What goes wrong:**
- `entryIn` accepts the file at the draw's URI when its top-level `root` and `totalTickets` **fields** equal the draw's. It never
  checks the entry's proof against the root.
- The resolver then sends `claim` with the file's holder and proof. The simulation reverts (`BadProof`) and the `case` ends with
  `break`.
- So the chain rebuild (the "claims never depend on the file" fallback) is never tried. The `voidDraw` after 7 days is also
  unreachable.
- The failure is only a warn log line. The monitor does not page Resolved-but-unclaimed draws: it pages Announced and Pinned only
  (`monitor/src/chain.mjs:82-88`).

**Who can trigger it:** anyone who can rewrite the file after `openDraw`:
- the mirror or CDN;
- whoever holds the upload credential;
- the keeper host.

The winner is then paid only if they notice and use the site's Claim button (the indexer's own tree). Otherwise, after 7 days,
anyone can void the draw and the prize returns to the coin.

**Test:** `keeper/test/audit2_services.test.ts` R2-S1: still Resolved after 7 days and more; never claimed, never voided. Passes.

**Fix:**
- In `entryIn`, verify `proof(holder, first, end)` against `dv.root` (`tools/snapshot` `verify`), and fall through to the next
  source when it fails.
- When a claim simulation reverts, try the chain rebuild.
- Page a draw that has been Resolved for more than 1 h.

#### R2-S8: the keeper's own alerts are never paged

**Where:** `keeper/service/status.ts:25-26` (`healthy` ignores alerts); `monitor/src/liveness.mjs:16-17` (only the HTTP status is
read).

**What goes wrong:**
- The keeper's pass alerts all stay 200 on `/healthz`. These are:
  - `tx-stuck` and `tx-replaced`;
  - `draw-slot-missed`, `draw-not-announced`, `draw-not-opened` and `draw-empty`;
  - `snapshot`, `publish`, `drand` and `drand-late`;
  - `claim`, `ticket-late`, `no-ceiling`, `pass-budget` and `coin-index`.
- The monitor's liveness poll never reads the body.
- `services-v1.md:115` says "The monitor pages on them", which is false.

**What still pages, and what doesn't:**
- Covered indirectly: a stale floor or cap, and draws or tickets late while Announced or Pinned (the monitor's chain reads).
- Not covered at all:
  - a missed announcement slot;
  - a stuck transaction that doesn't (yet) make the floor stale;
  - claim failures (R2-S1);
  - a mirror that is down, which means no draws are announced.

This re-opens the E-08 pattern for E-10 and E-11's "never silent".

**Test:** R2-S8: `healthy()` is true with three alerts in force, and `liveness.mjs` never reads the body. Passes.

**Fix:**
- Either make `/healthz` 503, or add an `alerts` count the monitor reads with a severity map, while alerts are in force for more
  than N passes.
- Or have the monitor poll `/status` (the public count) and page on a count above 0 for more than 10 minutes.

### Low

**R2-S2: one relay of three answering other bytes stops everything.**
- **Where:** `keeper/service/drand.ts:48-55`. Any distinct second answer throws `RelayDisagreement`, even when two relays agree and
  the beacon's `verify` would pick the right one.
- **What goes wrong:** one lying or broken relay (the three defaults are api.drand.sh, drand.cloudflare.com and api2.drand.sh), or a
  path MITM, stops every draw open (voided after 6 h) and every ticket and draw resolve (refundable after 7 days). It is not paged
  (R2-S8).
- **Test:** R2-S2 passes.
- **Fix:** verify each distinct candidate with `beacon.verify`, use the one the beacon accepts, and alert on the liar.

**R2-S3: a re-price after a cancel attempt signs the original call with 21,000 gas.**
- **Where:** `keeper/service/sender.ts:129-133`. `gas = last.gas` is taken from the newest attempt, which can be the self-transfer.
- **What goes wrong:**
  - The node refuses the replacement (intrinsic gas too low), or it would run out of gas.
  - The refused attempt is still recorded as the newest one, and every later re-price repeats it.
  - Meanwhile the cancel attempt is no longer re-broadcast.
  - If the cancel is underpriced, the nonce stays stuck until the base fee falls.
- **Test:** R2-S3 passes.
- **Fix:**
  - For a non-cancel replacement, re-estimate gas, or take the last *non-cancel* attempt's gas.
  - Don't record an attempt the node refused with an error that isn't about underpricing.

**R2-S4: E-01 in the other direction; the 5% pull cap removes the sales side's damping of a listing pump.**
- **Where:** `keeper/src/floor.ts:273-280`.
- **Measured on the recorded 2026-09-23 book:** buying the single cheapest listing (433k sats; the buyer keeps the frog) lifts the
  planned floor **+7.3%** at 3,901 bps confidence, which gets posted. The pre-fix blend gave +0.4%.

  | Listings bought | Floor as built | Pre-fix blend | Confidence |
  | --- | --- | --- | --- |
  | 1 | +7.3% | +0.4% | 3,901 bps (posted) |
  | 3 | +82% | +55% | 2,725 bps (below 3,000, not posted) |

- **Impact:** bounded by the module (±5%/h, ±20%/day), but caps (floor × 1.1), `maxItemCost` and `gradPrice` rise with it. Sweeps
  and the canister then pay more.
- **Test:** R2-S4 passes; the numbers are printed.
- **Fix:**
  - Cap the upward move per post relative to the recent sales median too.
  - Or require `depthItems ≥ k` for an upward step larger than X.

**R2-S5: E-11 residual.**
- **Where:** `keeper/service/config.ts:133`; `keeper/service/draws.ts:102-105`.
- **What goes wrong:**
  - With a mirror configured on 4663, `KEEPER_SNAPSHOT_BASE_URL` still defaults to `KEEPER_PUBLIC_URL/snapshots`, the hot key's host.
  - `publish` reads back only that URI, never the mirror.
  - A configuration that follows the defaults puts the key's host on chain as the URI, which the audit asked to avoid.
  - Mitigation: the resolver and the indexer can rebuild from chain.
- **Test:** R2-S5 passes.
- **Fix:** on 4663 with a mirror, require `KEEPER_SNAPSHOT_BASE_URL` on a host other than `KEEPER_PUBLIC_URL`'s, and read the mirror
  back too.

**R2-S6: the resolver's fetch of the keeper-chosen URI.**
- **Where:** `keeper/service/draws.ts:71-84`.
- **What goes wrong:**
  - `fetchText` buffers the whole body before checking the 20 MB cap when no content-length is sent. A hostile URI can stream until
    the keeper or resolver runs out of memory.
  - It GETs any https host from the keeper's or resolver's network, with no private-address check.
- **Test:** R2-S6 passes.
- **Fix:** stream with a byte cap and reuse the indexer's `guardedFetch` rules.

**R2-S7 (E-16 residual, listed for completeness): the keeper image runs as root with dev dependencies.**
- **Where:** `keeper/Dockerfile`: no `USER`, and `npm ci` without `--omit=dev`. It holds the hot key and runs the standalone
  resolver.
- **Test:** R2-S7 passes.
- **Fix:** the `drop-to-node.sh` pattern already used for the monitor.

**N-2: the site's send guard binds a trade to "a launched coin or curve", not to the page's coin.**
- **Where:** `web-rook/src/rook/phase.ts:131-147,183-184`; `Ticket.tsx:93-94`.
- **What goes wrong:** a hostile indexer can route a buy to an attacker-launched coin's curve. The loss is bounded by the trade.
- **Test:** `audit2_e07.test.ts` test 1 passes.
- **Fix:** pass the route's coin into `requestRefusal`.

**R2-L1: the monitor takes its start block from the wrong field.**
- **Where:** `monitor/src/stack.mjs:35` uses `deployedAtBlock`, which is the L1 `block.number` (about 26.06M), not the stack's L2
  `fromBlock` (about 72.8M).
- **What goes wrong:** about 47M wasted blocks per scan. If `from_block` is set by hand later than the first launch, honest draws
  recompute as "bad", so the guardian cancels them and pauses the keeper.
- **Test:** R2-M1c passes.
- **Fix:** read `s.fromBlock ?? s.deployedAtBlock`.

**R2-L2: the monitor's exclusions are copied into its config by hand.**
- **What goes wrong:** if the copy drifts from the keeper's, every draw is "bad", which means an auto-cancel and a pause. This is an
  operational self-DoS.
- **Fix:** compare against the published file's `inputs.exclusions`, or ship the same file in the monitor image with a pinned hash.

**R2-L3: the guardian marks an action done without a receipt, and cancels before it pauses.**
- **Where:** `monitor/src/guardian.mjs`.
- **Fix:**
  - Wait for the receipt and check its status.
  - Pause first.

**R2-L4 (E-05 residual): the reserves page stays green on a frozen module.**
- **Where:** `ops/src/backing.ts:83-84`. The stale rule uses `postedAt`, so confidence-0 re-posts keep it GREEN.
- **Test:** `ops/test/audit2_ops.test.ts` R2-O1 passes.
- **Fix:** use `lastGoodAt`.

**R2-L5: "Could read nothing, page at once" never fires.**
- **Where:** `ops/src/reconcile.ts:83,160`.
- **What goes wrong:** `WrappedService` turns total RPC failure into per-collection "unverified", so the page only comes after
  6 hours. At start, the reconciler crash-loops with no page, unless it is on the monitor's liveness list.
- **Test:** R2-O2 passes.
- **Fix:**
  - Treat a run with no read at all as blind.
  - Keep the reconciler on the liveness list.

**G-N1: golive-check `--phase1` prints the go/no-go PASS without any stack file.**
- **Where:** `script/golive-check.sh:376-401`.
- **What goes wrong:** none of the C-07, B-05, ceiling or wiring checks run. It also passes without `--canister` (only a note).
- **Test:** `scripts/audit2_golive_check.test.sh` case 1 exits 0.
- **Fix:** require a stack file under `--phase1`, and require `--canister` unless the operator passes an explicit
  `--before-reinstall`.

**G-N2: the "2-of-2 Safe" check is duck-typed.**
- **Where:** `golive-check.sh:82-88`. It only asks for `getThreshold()` = 2 and two entries from `getOwners()`.
- **What it doesn't check:**
  - enabled modules or a guard;
  - the singleton;
  - who the owners are, so the keeper or guardian could be owners;
  - whether the owners have code.
- **Test:** cases 2 and 3 exit 0: owners set to the keeper and the guardian, and a bare contract answering the two getters.
- **Fix:**
  - Require no modules and no guard, and the canonical singleton.
  - Require owners that are distinct from every role and code-less, and compare them with the ceremony's devices.

**G-N3: some reads fail open.**
- `registry.count()` unreadable: no listing is checked, and the run passes (`:115-116`; case 4).
- `indexOf != "0"` on an empty result prints "is in the directory" (`:359`).
- Two failed reads compare as equal (`:108`).
- A stack file without `capCeiling` accepts any non-zero ceiling (`:341-343`).
- **Fix:** a `call` that FAILs on empty output, and positive comparisons everywhere.

### Info

1. **N-3.** The Docker-built site ships the stale `web-rook/src/content/launch-gates.md` "Keeper key stolen" row, a consequence of
   E-15 item 5. Commit `npm run sync` and add a copies test for `src/content`.
2. **N-4.** An indexer that calls an inscriptions collection "nft" disables Redeem: `redeem(address)` is refused. It fails closed.
3. **E-14 residual.** No test covers a rehearsal floor above `MAX_PURCHASE_WEI` (0.5 ETH).
4. **I-1 and I-3.** The reconciler's `pinnedRpc` and the monitor's chain checker don't fail over on throttle codes, so one throttled
   read fails a poll.
5. **I-2.** The reconciler's "escrow ≥ 50% of floor" rule can false-RED in a fast-rising market, after about 4 days at +20% a day.
6. **I-4 and I-5.** `no-good-post` pages before launch. More than 32 disputed ranges throws every poll (paged, not silent).
7. **G-N4.** The guardian is not compared with ops, the registry treasury, the custodian or the Safe's owners, and `setGuardian` has
   no role-separation check.
8. **G-N5.** The dry run never exercises the FAIL paths for a wrong treasury split, a `wrapped()` mismatch, most pending states, code
   at non-keeper roles, a guardian equal to a role, or a missing stack file.
9. **Keeper draws.**
   - In the first 24 h after a stack is deployed, `snapshotAt` can fall before the stack's first block. The keeper then raises a
     `snapshot` alert instead of `draw-empty`, and the draw is voided after 6 h.
   - A slot missed across a restart after its window is never reported (`draw-slot-missed` depends on in-memory `sc`), and nothing
     else pages it.

## 3. Focus questions, answered

**The draw flow (announce, seed round from the mined tx, two relays, snapshot at time, sentinel, publish and read-back, open within
6 h).**
- It is correct and was proven end to end with real drand on a fork. The seed round is read from `draw(id)` at `latest`; the
  snapshot block is the last one at or before `snapshotAt` (the binary search is correct: `ts(lo) ≤ t < ts(head)`); the sentinel
  proof is carried; the open stops 15 minutes before `OPEN_TTL`.
- **Failure modes:**
  - one bad relay stops every open (R2-S2);
  - a mirror that is down stops announcements;
  - publish or snapshot failures only raise alerts, and those alerts are not paged (R2-S8).

**The keyless resolver.**
- **Ordering:** strict order from `nextToResolve`; the walk stops at the first ticket that isn't mined.
- **Spinning:** a dry run can't spin (the `next === last` guard).
- **Spending:** bounded by `KEEPER_RESOLVER_MAX_TX` (10 a pass), a 900k gas cap per transaction, the per-transaction and per-day
  money caps and the pass budget. Draws are created only by the keeper, one per coin before its pin. Tickets cost a burnt whole wX
  each, so spam is expensive.
- **Shared budget:** the in-process resolver shares the keeper's 0.01 ETH/day cap. At about 0.02 gwei that is several hundred
  resolves, so the floor post can't realistically be starved.
- **Gap found:** a malicious file blocks the claim path (R2-S1).

**Floor posting and wash-sale resistance.**
- The post stays inside `floorBounds()`, and the repair works.
- The downward attack is fixed.
- The upward push is now easier (R2-S4), but bounded on chain and gated by confidence.

**Transaction replacement and nonces.**
- It works in the common case: one nonce, every attempt's receipt counted, the cap and the pause honoured, "dropped" detected.
- One bug: R2-S3.

**RPC failover.** The keeper is fixed. The monitor and reconciler aren't (R2-M1, I-1, I-3).

**The monitor's root recompute and exclusions.**
- The tree matches `tools/snapshot` byte for byte on random adversarial holder sets.
- Its exclusion set equals `stackExclusions`, provided the hand-copied list matches (R2-L2).
- Its start block is wrong (R2-L1).
- **It is not operational on the public RPC pair (R2-M1).**

**The guardian.**
- **Can an attacker cause a false pause cheaply?** Not without the keeper key or a config drift. The runaway rule needs two clamped
  `FloorPosted` events, which only the keeper can emit. A root counts as "confirmed bad" only when both RPCs agree on another root.
  The false triggers that remain are self-inflicted (R2-L1, R2-L2).
- **Can a real attack avoid triggering it?** Yes:
  - edge-walking the floor (R2-M3);
  - opening a forged draw while ordofi is throttling or the history is long (R2-M1);
  - opening it behind two stuck draws (R2-M2).

**Alerts fail closed.** Telegram does; the keeper's own alerts don't (R2-S8).

**The reconciler's new balance checks.**
- A false GREEN is possible on a frozen module (R2-L4).
- A false RED is possible in a fast-rising market (Info I-2).

**golive-check `--phase1`.**
- It still passes with no stack file (G-N1), with a duck-typed Safe (G-N2) and on some failed reads (G-N3).
- The rehearsal knob can't be used on a real chain.

**The site's `requestRefusal`.**
- No bypass of the target, function or argument rules, and nothing that should be sent is refused.
- The guard doesn't tie a trade to the page's coin (N-2).

**The indexer's SSRF guard.** The IPv6 forms, numeric literals, redirects and DNS rebinding are all refused. There is a new crash
(N-1).

**Secrets.** The keeper scrubs its key, the status token and the upload credential. `/status` is only a summary, and the guardian key
never appears in an error or in JSON (the monitor's tests).

**Dockerfiles.** The monitor, reconciler, runner, web and indexer images drop to a non-root user. The keeper image doesn't (R2-S7).

## 4. Test and run results

| Run | Result |
| --- | --- |
| keeper `npm test` / `tsc --noEmit` | 123 pass, 0 fail (115 + 8 audit2) / clean |
| tools/snapshot `npm test` / `tsc` | 18 pass (17 + 1) / clean |
| staging `npm test` / `tsc` | 29 pass / clean |
| monitor `npm test` | 84 pass (79 + 5) |
| ops `npm test` / `tsc` | 30 pass (28 + 2) / clean |
| indexer `npm test` / typecheck / `npm audit` | 111 pass / clean / 0 vulnerabilities; `audit2_outbound` 3 pass |
| web-rook `npm test` / typecheck / both builds / `npm audit` | 57 pass / clean / clean / 21 moderate, 0 high; `audit2_e07` 2 pass |
| `node --test scripts/timelock-watch.test.mjs` | 5 pass |
| `STACK_PORT=8731 DRYRUN_NO_DRAND=1 scripts/golive-dryrun.sh stack` (fork of 4663, local) | **ALL PASS**; golive-check 138 ok; the `--phase1` refusals behave as documented |
| `KEEPER_FORK_PORT=8741 scripts/keeper-fork-test.sh` (fork about 25 h back) | **ALL PASS**, 43 ok; step 5 ran the draw end to end with real drand, and `tools/snapshot --vault/--draw-id` reproduced the same root; step 9 skipped (no dfx) |
| `scripts/audit2_golive_check.test.sh` (fake `cast`) | 6/6 cases as recorded |
| Docker (monitor, reconciler, runner) | run as uid 1000; symlink-chown not exploitable |

## 5. Not covered

- The canister and the contracts beyond what the services call. `forge test` was not re-run; only `forge build` was run for the dry
  run.
- The dry run's draw was taken only to the announcement (`DRYRUN_NO_DRAND=1`). The keeper fork test covered the full drand path.
- The fork test's recompute uses `tools/snapshot`, which shares `stack.ts` with the keeper, so it shows determinism, not
  independence. The monitor's port was tested separately, but **no fork run drove the keeper and the monitor together**.
- The guardian's signed transaction was not sent to a local anvil; only the unit tests cover it.
- `ops/scripts/runner-replica-test.sh` and keeper-fork step 9 (no dfx).
- A browser end-to-end run of the site against a hostile indexer.
- Whether `api3.drand.sh` (used by the site) serves evmnet.
- Railway, DNS and account configuration; KMS for the keeper key (recorded as open).
- Live UniSat and Ordinals Wallet behaviour beyond the recorded fixtures.
