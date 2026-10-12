# Round 4, services: what the targeted check of round 3 (R3-check) found in the monitor, indexer, keeper and go-live check

Branch `r4-services` (from `fixes-2026-09-26` at `4a215f4`). Written before the code, as the design; "As built" at the end is
filled in when the code lands. Nothing here is deployed; no key, wallet or Railway service is touched. Network use is read-only
(the public 4663 RPCs, the public drand relays through the fork test).

Inputs: `review3/R3-check.md` (§2), `review3/parts/part-services.md`, the PoCs `review3/poc/audit3_monitor.test.mjs` and
`review3/poc/audit3_outbound.test.ts`, `review2/fixes-R3-services.md` and `review2/integration-R3.md`.

Scope: services M-A, M-B, M-E, the false-pause lever that M-A/M-B/L-F formed with contracts A3-M1, and the Lows L-C, L-F, L-D,
L-G, L-H. Directories: `monitor/`, `indexer/`, `keeper/`, `ops/`, `tools/snapshot/`, `web-rook/`, `script/golive-check.sh`,
`scripts/`, the deploy docs. Not touched: `src/` (the parallel `r4-contracts` round: A3-M1 paused-burn pricing, A3-L1 grace after
an unpause) and `autobuyer/` (`r4-canister`). §8 lists what those rounds' changes need here.

Each audit3 PoC becomes a regression test in its package that asserts the attack now **fails**; the file keeps its `audit3_`
name so the trail is one grep.

## 0. What was measured before the design (2026-09-26, read-only)

| Question | `rpc.mainnet.chain.robinhood.com` | `robinhood.drpc.org` |
| --- | --- | --- |
| `eth_getBlockReceipts(block)` | served | served |
| state at an explicit block (`eth_getBalance`) | 50 and 5,000 blocks back: served; 500,000 back: `historical state … is not available` | never (`Unknown state. First available state is 1`) |

So a block's logs can always be read without `eth_getLogs` (the receipts), and recent state (minutes old) can be read on
Robinhood to check an index against the chain, but a snapshot a day old cannot be checked against state.

## 1. M-A: a provider's page span recovers

`monitor/src/holders.mjs` `Provider` and `pagedLogs` (the holder indexes and the canary's `bothLogs` share them).

- **Which errors lower the ceiling.** Only an answer about the query's size: `exceeds`, `too many` (not "too many requests"),
  `range`, `limit` (not "rate limit"), `more than N results`, `response too large`, `block range`, `log response size`, HTTP 413.
  A throttle (`retryable()`: 429, 5xx, `-32005`, busy, timeouts, resets) is checked first and cools the provider down, as now.
  `rpcOnce` puts the first 200 bytes of a non-2xx body in its error (a provider that says "range too large" with HTTP 400 is then
  recognised); a body is provider text, never a secret of ours.
- **Any other error** (`header not found`, HTTP 400/403 with an HTML page, a parse error) does not touch the span or the
  ceiling: the same page is asked again, at most `PAGE_TRIES` = 3 times with backoff, then the read ends for this poll with the
  error (the cursor stays before it). Three pages failing that way in a row cool the provider down for `COOL_MS` (2 min), so a
  provider that is broken for good costs a few calls a poll, not a stream of them.
- **Recovery.** A success doubles the span up to the ceiling (as now). The ceiling itself grows back: doubled (up to the
  configured `start`) after every `GROW_AFTER` = 2 consecutive successful pages at the ceiling, and reset to `start` when the
  range refusal that set it is older than `CEILING_TTL_MS` = 10 minutes ("a fresh window"). So a burst of dense blocks costs a few
  narrow pages, then the provider is back at its full span within a few dozen calls or ten minutes, whichever comes first, and a
  restart still starts from `start`.
- **Never stuck at one block.** The span never ends a poll at 1 because of anything but a range answer at span 1, and that case
  is M-B's (§2), which reads the block another way and moves on.

Regression: the PoC's 13 `header not found` errors leave the ceiling at the configured span, and 600 new blocks cost one call.
New: after range refusals the ceiling grows back after successes and resets after its TTL; a provider answering `HTTP 403` for good
costs at most 3 calls, then cools down.

## 2. M-B: a dense block never stalls the index, and only consistent state is saved

`pagedLogs` gains a dense-block path. When a page of **one block** is refused for its size (`rangeError`), the block `X` is read
another way, filtered to the query's addresses and first topics (the index needs only RAFFLE coins' `Transfer` and the factories'
`Launched`):

1. `eth_getBlockReceipts(X)`: one call returns every log of the block (both public providers serve it, §0);
2. else the query split by address (halves, down to one address), each for block `X` alone;
3. else, for an address still refused alone, the block's transactions' receipts one by one (`eth_getBlockByNumber(X, false)`,
   then `eth_getTransactionReceipt`, at most 2,000 transactions);
4. else that address is **stuck** at `X`: `pagedLogs` returns it in `stuck` (`address → X`) and goes on past `X` for the rest.

Receipts carry `removed: false` logs of the block as mined at the provider's head; the index reads only blocks at least
`confirmations` (20) deep, as now.

`HolderIndex.update` then:
- never lets one coin hold the group: a coin stuck at `X` keeps its cursor at `X − 1` with its logs up to `X − 1` (its logs at
  `X` and later from this read are dropped), is marked `stuck: { block, since, error }`, leaves the group and is caught up on its
  own (`#catchUp`, the same dense path) every poll from `X`; every other coin and the group cursor advance past `X`;
- a factory stuck at `X` (a `Launched` log unreadable) still holds the group, since a coin launched there could be missed; that
  needs a factory block with more logs than a provider's cap that is also refused by receipts and by transaction receipts, which
  no outsider can make cheaply (a launch costs a launch);
- a batch of addresses that a provider refuses for its number of addresses ("too many addresses") is the same range answer at
  one block and is split the same way.

A coin's cursor, base and logs move together in memory and the file is written atomically, so what is saved is always a state
some prefix of the chain's logs gives. `summary()` lists the stuck coins; `checkEvm` pages SEV2 "holder index: coin C stuck at
block X on provider P (why)" (persistent), and SEV1 if a stuck coin is a DECIDING draw's coin.

Regression: the PoC's dense block no longer stalls the group or the unrelated coin, and the dense coin itself is read (by
receipts) and verified. New: receipts refused too → only the dense coin is stuck, the rest advance; a batch refused for its
number of addresses is split; a restart from the saved file resumes from the stuck block.

## 3. False-pause resistance: the deadline pauses on evidence, and escalates otherwise

Today, at `challengeEnds − deadline_s`, any confirmed Open draw that is not verified (and not single-provider verified) pauses the
keeper. After r4-contracts' A3-M1 fix a pause no longer pays burns above what a pre-positioned mint could have paid, and after
A3-L1 an unpause no longer lets a refund race the keeper's first post, so a false pause is no longer a profit lever; it is still
a liveness cost (mints, fills, closes and draws stop until the Safe unpauses) and a false pause that outsiders can cause at will
is a denial of service. So the pause must need evidence where evidence is possible.

### 3.1 What each provider says, per draw

The draw record keeps, per provider, the last outcome: `match` (reproduces root and count), `other` (computes another root or
count from an index caught up past the snapshot block), or `error` with its class:
- **degraded**: the index is behind the snapshot block, the coin is not indexed yet, stuck (§2) or broken, being rebuilt, older
  than the index keeps, or the provider is cooling down / throttled / timing out / unreachable. None of these says anything about
  the root, and several can be caused by outsiders (dense blocks, permissionless launches) or by ordinary flakiness.
- **unexplained**: anything else (for example no block at or before the snapshot time on a provider that answers): the draw cannot
  be recomputed by a healthy index for a reason the monitor does not know.

### 3.2 Self-repair before trusting a dissent (L-F, and the evidence rule)

When a provider computes `other` for a DECIDING draw, its index of that coin is rebuilt from the coin's launch once for that draw
(`HolderIndex.repair(coin, "draw:<vault>:<id>")`, priority in the next update). The dissent is recorded (`dissent[host] =
{ root, tickets, at, repaired: false }`); when the rebuilt index computes again, `repaired: true` if it still dissents, or the
dissent is dropped if it now matches (it was a dropped log). A dissent is **confirmed** once repaired. Two providers computing the
same other root is still `bad` at once (pause, then cancel), unchanged.

### 3.3 The rule at the deadline

For a confirmed (on both RPCs) Open draw not verified at `challengeEnds − deadline_s`:

| Evidence | Page | Automatic pause (`auto_pause`, `pause_unverified`) |
| --- | --- | --- |
| `bad` (two providers, same other root) | SEV1, as now | pause, then cancel (as now, not only at the deadline) |
| no provider reproduces the chain's root, and at least one caught-up index computes another (repaired or not) | SEV1 | **pause**: a healthy index's history disagrees with the root; no outsider can cheaply make an index compute another root |
| some provider reproduces it and every dissent is **confirmed** after a rebuild | SEV1 | **pause**: a provider that re-read the coin's whole history still disagrees |
| some provider reproduces it, a dissent not yet repaired | SEV1 "act by hand" | no: one provider reproduced the exact root, the dissent is likely one dropped log (L-F) |
| no provider could compute, and at least one error is **unexplained** | SEV1 | **pause** (fail closed: a healthy index cannot say why) |
| no provider could compute, every error **degraded** (behind, stuck, cooling, throttled) | SEV1 "ESCALATE: act by hand", repeated each poll, and the checker is `degraded` (the monitor's `/healthz` answers 503) | **no** |
| one provider reproduces it, none dissents | SEV2 (as now) | no |

`draws.pause_on_degraded` (default **false**) restores round 3's fail-closed rule (pause on any unverified draw) for an operator
who prefers it. A draw pinned while unverified is SEV1 as now.

### 3.4 The trade-off

- A stolen keeper key that publishes a bad root is still paused (and cancelled) whenever the providers can read the coin: `bad`
  before the deadline, or the one-provider dissent at it. What it no longer gets for free is the pause when the monitor is blind.
  To have a bad root pinned the thief now needs every provider degraded for the draw's coin at the deadline, which it cannot cause
  cheaply once §1 and §2 hold (dense blocks are read by receipts; a coin cannot be stuck on every path; throttling is the
  providers'), or a provider that lies in its favour (I-2, already accepted in round 3). The SEV1 escalation puts a human in front
  of every such draw with the by-hand pause and cancel.
- An outsider who wants a pause (A3-M1's lever, now much smaller) must make the monitor see evidence: a caught-up index
  computing another root, or an unexplained failure. Dense blocks, many launches, and provider flakiness give only "degraded",
  which pages but does not pause.
- A pause should be rare: it now needs a wrong root, a provider serving a different history, or a failure the monitor cannot
  explain. Each is a real problem worth a human's attention before any draw is pinned.

## 4. L-F: the holder index repairs itself

- **On a dissent**: §3.2 (a rebuild per draw per provider).
- **Periodic reconciliation against the chain's state** (`HolderIndex.reconcile`, from `checkEvm` after the update): each poll,
  up to `RECONCILE_PER_POLL` = 2 coins per index, each at most every `RECONCILE_MS` = 30 min, oldest first. At the index's cursor
  `B` for the coin (only when `B` is within `RECONCILE_MAX_AGE` = 3,000 blocks of the head, where Robinhood serves state), it reads
  `totalSupply()` and `balanceOf` of the index's 16 largest holders at `B`, from the index's own provider, else from any other
  configured provider (drpc serves no state at a block, §0). The index's sum of balances must equal `totalSupply`, and each read
  balance the index's. A mismatch rebuilds the coin from its launch (at most once an hour a coin) and is recorded; a read that
  fails is "unreadable" and changes nothing. A lying state provider can at most make an index rebuild once an hour, which is
  "degraded", never a pause.
- **Expiry**: `resets` becomes `resetAt` (a second rebuild on missing logs is allowed 6 h after the first); a `broken` coin is
  rebuilt again after 1 h instead of never.

Regression: the PoC's lossy index is rebuilt on the disagreement and then verifies the honest draw; a dropped mint is found by
reconciliation (`totalSupply`) with no draw at all; a broken coin is retried after its hour.

## 5. M-E: the indexer's transport always settles

`indexer/src/outbound.ts`:
- `pinnedHttpsFetch`: `req.on("upgrade", …)` destroys the socket and rejects (`Refused("… HTTP 101 …")`); `req.on("information")`
  for 1xx is left to Node (a 100 is followed by the real answer), but any final status outside 200–599 is refused as now; `close`
  before a response rejects; a response listener that throws rejects; the request's own `timeout` covers slow headers.
- `guardedFetch`: the whole request, redirects and body included, races the deadline: on abort the transport's promise is
  abandoned and the fetch rejects `Refused("… timed out after N ms")`, and the signal passed to the transport aborts the socket.
- `wrapped/service.ts`: an `inflight` build older than 2 × the guard's timeout is dropped (the next caller builds again); every
  caller waits at most that long.
- The catalogue's background `refresh()` gets the same bound (a hung refresh no longer blocks every later one).

Regression: the PoC's `HTTP 101` rejects within the deadline; new: a server that accepts the connection and never sends headers
rejects at the deadline; a 101 through `guardedFetch` rejects; the process lives in every case.

## 6. L-C: the floor-walk rule cannot be evaded by interleaved posts

`monitor/src/evm.mjs` `floorWalk`:
- A post that moves less than `WALK_IGNORE_BPS` = 10 % of the hourly band (0.5 % at 5 %/h) neither counts as an edge post nor
  resets the edge streak: a no-move post one second after an edge post changes nothing. An opposite move that large costs the
  walker its progress.
- **Net move over a rolling 6 h**: the accepted floor at least 12 % (3 hours at the band's edge) away from its value 6 h ago:
  SEV1 "walked", with the `pause_on_floor_walk` trigger, whatever the post pattern.
- **Day move**: SEV2 at 10 %, SEV1 at 18 % as now; the SEV1 now carries the `pause_on_floor_walk` trigger.
- **Three days**: 15 % over 72 h SEV2, 25 % SEV1 (with the trigger). The history kept per module grows from 26 h to 74 h, capped at
  400 entries (the oldest thinned first).

Regression: the PoC's interleaved walk pages SEV1 on the edge rule and the 6 h rule and triggers the pause when told to; the
−0.44 %/h walk pages SEV1 on the 72 h rule before it reaches −30 %.

## 7. The other Lows

- **L-D (keeper resolver)**: a chain rebuild that does not prove is cached with a retry time (10 min, doubling to 6 h) instead of
  for the life of the process; a rebuild that throws is retried on the same schedule (not every pass). The vacuous audit2 test is
  rewritten so the rebuild really runs and really mismatches, and counts it.
- **L-G (keeper `/healthz`)**: every alert raised is kept "in force" for `KEEPER_ALERT_LATCH_S` (default 1,200 s, longer than the
  monitor's 10-minute `DOWN_MS`) after its last pass, so a one-pass alert (`draw-slot-missed`, `draw-not-opened`) or a flapping one
  reaches `KEEPER_ALERT_PASSES` and turns `/healthz` 503 long enough for the monitor's liveness to page it. `/status` shows the
  latched kinds. Same in the standalone resolver.
- **L-H (golive-check)**: under `--phase1` (not the rehearsal) `--safe-owners` is required; the Safe's fallback handler (slot
  `keccak256("fallback_manager.handler.address")`) must be zero or the canonical 1.3.0 / 1.4.1 `CompatibilityFallbackHandler`;
  and the Safe's `SafeSetup` event is read (from `SAFE_FROM_BLOCK`, default 0): exactly one, with `initializer` zero (no
  delegatecall in `setup()` that could plant owners or modules `getOwners`/`getModulesPaginated` cannot see) and owners and
  threshold equal to what the chain reports. An unreadable `SafeSetup` is a FAIL.
- **I-1** (the deadline skipped when `checkEvm` throws early): the deadline and the guardian now run before the "more than 32
  disputed ranges" throw; "neither RPC answers" still throws (nothing can be read or sent then; `checker-down` pages it).
- **I-2, I-3**: unchanged, listed as open.

## 8. For the other rounds

- **r4-contracts (A3-M1, A3-L1)**: paused burns priced at `min(β, max(acceptedAtPause, dayLow))` and a grace after an unpause.
  Any new view or event (for example `acceptedAtPause`, `unpausedAt`, a grace constant) is added to the monitor's, keeper's,
  indexer's and site's ABIs when their list is final; the monitor's keeper-paused page should then quote the paused burn price,
  and the keeper should post the floor on its first pass after an unpause (the grace ends at its first good post). Listed in
  "As built" with what was done on this branch and what waits for the merge.
- **r4-canister**: no service change expected (R3-C1/R3-C2 are inside the scanner); the monitor's candid fixture is regenerated
  only if its types change.

## 9. Runs this branch must pass

Every package's tests, typecheck and build; the `audit3_*` regressions; `scripts/audit2_golive_check.test.sh`;
`scripts/golive-dryrun.sh stack` and then `scripts/keeper-fork-test.sh` (one after the other, public read-only fork).

## As built

Commits on `r4-services`: the design (`5aac131`), then monitor, indexer, keeper, golive-check, the r4-contracts pickup, the merge of
`fixes-2026-09-26` (r4-contracts and r4-canister merged there) and the service-side changes it needs, and this section. Nothing
deployed; no key, wallet or Railway service touched.

### Monitor (§1–§4, §6)

- `abi.mjs`: `rangeError()` (size answers only; throttles excluded first); `retryable()` matches a status only as `HTTP 5xx/429`
  (a provider's text now follows it); a non-2xx body's first 200 bytes are in the error; `ERC20` (`totalSupply`, `balanceOf`);
  `FloorModule.pauseFloor/burnsOpen/RESUME_GRACE`, `ListingRegistry.keeperResumedAt/…Block`.
- `holders.mjs` **M-A**: `Provider.refused` halves the ceiling on a size answer only; `ok()` doubles the span and grows the ceiling
  back after `growAfter` (8) pages at it, the probe interval doubling each time the regrown ceiling is refused again (so a provider
  with a real 100-block limit is probed rarely); `refresh()` forgets a ceiling 10 minutes old. Other errors: the page retried at
  most 3 times, then the read ends; three such failures in a row cool the provider down. **M-B**: one block refused for its size →
  `denseBlock`: `eth_getBlockReceipts`, else address halving at that block, else per-transaction receipts (≤ 300 transactions),
  else the address is `stuck` there; the holder index keeps a stuck coin at `X − 1` (paged SEV2, SEV1 if a deciding draw needs it)
  and moves everyone else on; the canary (`allowStuck` off) stops before such a block. A filter refused for its number of addresses
  is retried with half as many addresses per filter. **L-F**: `repair(coin, tag)`, `reconcile()` (§4), `resetAt`/`brokenAt`
  expiry; `rootAt` returns the index's `repairTag`.
- `evm.mjs` §3: `evidenceOf` (per-provider outcome, dissents, the repair), `deadlineCall` (the table of §3.3), `errorClass`
  (degraded vs unexplained); `draws.pause_on_degraded`, `draws.reconcile`; the escalation repeats every 2 minutes until the pin
  and marks the checker degraded. **I-1**: the dispute-limit throw comes after the deadline and the guardian. **L-C**: §6 as
  designed. **A3-I1** (decided here): `evm.guardian.pause_on_no_good_post_h` (opt-in, 2–160 h; example value 12): after that long
  with no good floor post the page is SEV1 and the guardian pauses (paused burns pay the frozen floor, so a pause is worthless to an
  outsider); off by default because a pause also stops draws and graduations and a stale floor is not evidence of a stolen key.
- `chain.mjs`: the keeper-paused page quotes each module's `pauseFloor()`; `exitAtBacking()` (now `escaped()` alone) only when
  escaped; `burnsOpen()` false while paused is called out. The unpause page names the 6-hour refund grace.

### Indexer (§5)

`pinnedHttpsFetch` settles exactly once and rejects on `upgrade` (101), on `close` before an answer, and errors a body whose socket
closes early; `guardedFetch` races the whole request (redirects and body) against its deadline and aborts the transport's signal;
`WrappedService.snapshot` waits at most `inflightMs` (60 s) on a build and starts a new one past it; the catalogue starts a new
refresh when one is older than 15 minutes. r4-contracts: the module report adds `burnsOpen`, `pauseFloorWei` and
`refundsHeldUntil` (`keeperResumedAt + RESUME_GRACE` while it runs); `exitAtBacking` is documented as `escaped()` alone.

### Keeper (§7)

L-D: `mem.rebuilt` holds `{ entry, retryAt, tries }`; a rebuild that does not prove (or throws) is retried after 10 min, doubling to
6 h, and its cached logs are dropped first (`LogCache.forget`), so the retry reads the chain. The audit2 test that compared 0 with 0
now removes only the coin's cache, sees one mismatch, none on the next pass, a second after the backoff. L-G: `AlertLatch`
(`KEEPER_ALERT_LATCH_S`, 1,200 s) in the keeper and the resolver; `/healthz`'s alerts are those in force.

### Go-live check (§7)

L-H as designed; the SafeSetup event is read with `cast logs` (`SAFE_FROM_BLOCK`, default 0: Robinhood's RPC serves a Safe's
logs from block 0 in about 2 s, measured); the canonical `SafeToL2Setup` (what Safe{Wallet} passes on an L2) is accepted when its
runtime hash on the chain matches (`0x2f25…274b`, read on 4663). `--safe-owners` missing under `--phase1` is a FAIL (a WARN under
`GOLIVE_REHEARSAL`). `docs/v6/phase1-scope.md` step 8 names the new reads.

### Site and docs

Books: a paused module is `burnsOpen && !fresh && !escaped` (a round-3 module: `exitAtBacking && !escaped`); the text gives the
frozen floor and the refund grace. Trust page, `launch-gates.md` "Keeper key stolen": while paused, holders burn at the frozen floor
(never above the backing per unit) less the base spread, at the backing only after a week; refunds held 6 h after an unpause; the
optional no-good-post pause. `k6-watcher-canary.md` and `services-v1.md` (the latch). Site ABIs `FLOOR_MODULE_R4`,
`LISTING_REGISTRY_R4`, held to `out/` by `abis.test.ts`; the indexer's `r3.test.ts` checks the new views too. `types.ts` synced in
indexer, web-rook and web.

### Runs (after merging `fixes-2026-09-26` at `4c3334d`, with r4-contracts and r4-canister)

| Run | Result |
| --- | --- |
| monitor `npm test` | **122 pass** (106 + `audit3_monitor` 16: M-A 3, M-B 4, L-F 3, the deadline 3, L-C 2, A3-I1 1) |
| keeper `npm test` / typecheck | **134 pass** (131 + `audit3_services` 3; the audit2 rebuild test rewritten) / clean |
| indexer `npm test` / typecheck | **132 pass** (123 + `audit3_outbound` 9: 101, 101 held open, slow headers, stalled body, the four R3 variations, a hung build) / clean |
| web-rook `npm test` / typecheck / both builds | **66 pass** (+ the R4 ABI case against `out/`) / clean / ok / ok |
| ops, tools/snapshot, staging, web | 31, 19, 29, 118 pass; typechecks clean |
| `scripts/timelock-watch.test.mjs` | 4 pass, 1 skipped (no local replica) |
| `scripts/audit2_golive_check.test.sh` | **49 ok** (39 + 10 L-H cases), exit 0 |
| `STACK_PORT=8797 scripts/golive-dryrun.sh stack` (public read-only fork, real drand) | **DRY RUN: ALL PASS**, 44 ok, exit 0: launch → graduation → module mint → sweep → draw announced → opened → pinned → resolved → one whole wX claimed; the `--phase1` read under `GOLIVE_REHEARSAL` passes (no `--safe-owners`: a WARN there) |
| `KEEPER_FORK_PORT=8847 scripts/keeper-fork-test.sh` (run after the dry run) | **KEEPER FORK TEST: ALL PASS**, 43 ok, exit 0 (step 9 skipped: no dfx) |

### Left open

- **I-2** (one lying provider plus one lagging suppresses the pause) and the matching §3.4 trade-off: a dissent that has not been
  re-read by the deadline, against a provider that reproduces the root, escalates instead of pausing. A third log provider (a paid
  or own node) is still the real fix.
- **I-3** (an untracked empty close pages SEV2): unchanged.
- The reconciliation checks at the index's cursor, where Robinhood serves state; a snapshot a day old cannot be checked against
  state on the public providers (drpc serves none), so reconciliation catches a dropped log only once, at the head, not at the
  snapshot block. A dropped log older than the index's last reconciliation is still found by the dissent repair.
- `denseBlock` gives up per-transaction receipts above 300 transactions in the block; such a coin is stuck (paged), never skipped.
- Measuring M-B on the real providers: drpc's per-query log cap was not measured (as in R3).
- Not run: a fork run driving the keeper and the monitor together; the monitor's Docker image.
