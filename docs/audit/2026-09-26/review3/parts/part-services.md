# R3 check: off-chain services (monitor, keeper, snapshot tool, ops, indexer, site, golive-check)

Checkout `/Users/jlohayon/Documents/Rook-rev3` (detached at `fixes-2026-09-26`, head `82a2aff`). Diff reviewed:
`git diff 2bb3ee7..fixes-2026-09-26 -- monitor keeper tools ops indexer web-rook web script/golive-check.sh scripts`
(77 files, +3,515 / −380).

Scope and rules:

- Only new untracked PoC files were added: `monitor/test/audit3_monitor.test.mjs` (5 tests) and
  `indexer/test/audit3_outbound.test.ts` (5 tests). A keeper PoC was drafted and then removed (see L-D).
- No existing file was changed. Nothing was committed, pushed, deployed or signed.
- Network use was read-only: `eth_getBlockByNumber`, `eth_gasPrice`, and ArbGasInfo `getGasAccountingParams` / `getMaxBlockGasLimit`
  on `rpc.mainnet.chain.robinhood.com`.
- I did not run forge, anvil, `golive-dryrun.sh`, `keeper-fork-test.sh` or `script/deploy.test.sh` (it needs anvil and forge). Those
  are the parent's.

## 1. The round-2 services Highs and Mediums

| R2 | Verdict | Why (and what I re-ran) |
| --- | --- | --- |
| **R2-M1** (High): the recompute cannot finish on the public RPC pair | **Fixed**, with new residuals M-A, M-B and L-F | Each provider now has an incremental holder index with a saved cursor (`holders.mjs`). Draws are also found from the vault's state (`scanDraws`). A deadline at `challengeEnds − 360 s` pauses the keeper. `requestDraw` refuses while the keeper is paused (checked in `SweepVault.requestDraw`). The original idea (B answers -32005 to every `eth_getLogs`, with 60 days of history) is now: draw found, root replayed locally, bad → pause then cancel (`audit2_monitor` R2-M1a/b, `holders.test` "whole poll from two indexes"). **But** the index has new ways to stall or go wrong for good (M-A, M-B, L-F). In the example config only Robinhood and drpc can serve logs (ordofi refuses all of them), so quorum 2 in practice means "both of those". |
| **R2-M2**: a stuck draw starves later ones | **Fixed** | Every DECIDING draw is recomputed each poll, earliest `challengeEnds` first, each in its own try/catch. The deadline loop runs over every draw, not a slot. Variation with many draws: no slot limit is left. Info: `scanDraws` reads at most 50 draws per vault per poll from the lowest Announced/Open one, so ≥ 50 never-voided Announced draws would hold the state-scan window. The `DrawOpened` log path still finds later draws, and void is permissionless. |
| **R2-M3**: a floor walked along `floorBounds()` pages nothing | **Fixed** for R2's PoC, with residual **L-C** | R2's 24 hourly edge posts now page SEV1 at the 3rd post, and the day move pages too. **Evasion (PoC):** add a no-move post 1 s after every second edge post. That resets the streak, and also resets the module's clock, so it costs nothing. The edge-run rule, and so `pause_on_floor_walk`, never fire. Only the day-move rule catches it (SEV1 at ≥ 18 %, with no trigger). A walk just under ~10 % measured over 23 h pages nothing at all: −50 % in a week, silently (PoC). |
| **R2-S1**: a forged file blocks the claim and the void | **Fixed**, with residual **L-D** | `entryIn` proves `ticketLeaf(holder, first, end)` against the **on-chain** root and refuses holder 0. The leaf is double-hashed, so no internal node can pass. A reverted claim retries with the chain rebuild in the same pass. There is `claim-stuck` after 1 h, and a void after 7 d is reachable. `fetchText` never throws (null on any failure, 20 s abort, streamed 20 MB cap, public-address check), so a hostile URI costs at most 20 s a pass. |
| **R2-S8**: the keeper's alerts are never paged | **Fixed**, with residual **L-G** | `/healthz` answers 503 once alerts have been in force for 3 consecutive passes, or on any refusal. The monitor's liveness puts the sanitised kinds in the page (`audit2_monitor` R2-S8). An outsider cannot realistically force a permanent 503: none of the alert sources are outsider-driven, and Railway's healthcheck applies only at deploy (`restartPolicyType: ALWAYS` restarts on exit, not on 503). |
| **N-1** (indexer): a status of 600–999 crashes the process | **Fixed** for 600–999, 1000, 199, an invalid reason phrase, 100-then-999, over-cap bodies and a reset mid-body (5 audit3 variations; the process lives). **New gap M-E:** an `HTTP/1.1 101` answer leaves the fetch pending for ever | See M-E. |

## 2. New findings

| ID | Sev | Title | Where | PoC | Result |
| --- | --- | --- | --- | --- | --- |
| M-A | **Medium** | A provider's page-span ceiling only ever shrinks. One burst of transient non-range errors (header not found, 400/403, a proxy's HTML) ratchets it to 1 block a call for the life of the process. The index and the canary then fall behind the chain for good. | `monitor/src/holders.mjs:69` (`p.ceiling = min(ceiling, b − a)`, never raised); `:76`; shared by `evm.mjs` `canaryProviders`/`bothLogs` | `audit3_monitor` test 1 | PASS (gap present): after 13 errors, ceiling = 1; 600 new blocks cost 600 calls |
| M-B | **Medium** (Low if every provider's per-query cap exceeds what 32 M gas can emit) | One block whose Transfer logs exceed a provider's per-query cap stalls that provider's **group** cursor, so every coin's index stops for good. Every draw after it is unverified, and at each deadline `pauseKeeper()` fires. That is a repeatable false pause, and a pause now means burns at the backing and no mints, fills or closes. | `holders.mjs:185-193` (the group read; a 1-block page that fails returns an error, the group cursor never passes it, no per-coin or per-topic split), `:69-71` | `audit3_monitor` test 2 | PASS: the group and an unrelated coin stop before the dense block; `rootAt` throws "behind the snapshot block" |
| M-E | **Medium** (the R2 N-1 class) | `pinnedHttpsFetch` never settles on `HTTP 101 Switching Protocols`, and `guardedFetch`'s deadline does not rescue it. The indexer's `snapshot()` shares one `inflight` promise per collection, so one hostile draw URI hangs **every** API request for that collection for good. A hostile tokenURI or marketplace host freezes the catalogue's background refresh. | `indexer/src/outbound.ts:194-221` (no `upgrade`/`close` handler); `guardedFetch` awaits the transport with no race; `wrapped/service.ts:74-83` | `audit3_outbound` test 5 | PASS (gap present): still pending after 10 s with a 3 s guard timeout |
| L-C | Low | The R2-M3 edge-run rule is evaded by interleaved no-move posts (`pause_on_floor_walk` never triggers). A walk under ~10 %/23 h pages nothing. | `monitor/src/evm.mjs:156-161, 175-181` | `audit3_monitor` tests 3, 4 | PASS: 0 edge pages or triggers, day-move SEV1 only; −50 % in 7 days with 0 alerts |
| L-F | Low | The holder index never repairs itself. A page that silently lacks a log (with no balance going negative) makes the index wrong for good. An honest draw is then "disagree", and the deadline treats disagree as pause. `resets` and `broken` are persisted in the index file and never cleared, so one earlier reset means the next inconsistency, however much later, breaks the coin for good. | `holders.mjs:208-229` (only negative balances are detected; `resets`/`broken` persisted); `evm.mjs` deadline (`matches ≥ 1 && status !== "disagree"` is the only no-pause path) | `audit3_monitor` test 5 | PASS |
| L-D | Low | A chain rebuild that does not match once (the resolver's own log provider short at that moment) is cached as "no entry" for the life of the process. The resolver never rebuilds or claims that draw again, and voids it after 7 d unless the winner claims through the site. Also, the audit2 regression "one rebuild per draw" is vacuous: in that test the rebuild throws ENOENT every pass (the test deletes the logs dir), so it compares 0 with 0. | `keeper/service/resolver.ts:186, 192-193`; `keeper/test/audit2_services.test.ts:84,88` | reasoned (end-to-end PoC not completed: the in-process log cache kept serving whole logs) | — |
| L-G | Low | R2-S8 residual: one-pass alerts never reach 3 passes, so they never 503 and never page. `draw-slot-missed` is raised once (`sc.reported = true`); `draw-not-opened` lasts about one pass before the void. Any clean pass resets the count, so flapping alerts never page either. | `keeper/service/keeper.ts:286-292`; `main.ts` `alertPasses = alerts.length ? +1 : 0` | reasoned | — |
| L-H | Low | golive-check's "real Safe" (G-N2) can be defeated by whoever ran the Safe's `setup()`. Its `to`/`data` delegatecall can write an unlinked `owners[x]` (which `checkSignatures` accepts) or `modules[x]` (which `execTransactionFromModule` accepts), and both are invisible to `getOwners`/`getModulesPaginated`. The fallback handler is not read. `--safe-owners` is optional, so under `--phase1` without it any two code-less owners pass. | `script/golive-check.sh:119-147` | reasoned (needs a real Safe on anvil) | — |
| I-1 | Info | The fail-closed deadline only runs if `checkEvm` reaches it. `disputed > 32` and "neither RPC answers" throw first, and the `minConfidenceBps` read is uncaught. The checkers run in sequence with 30 s × 3 RPC timeouts, so a slow poll can miss the 6-minute deadline window. `checker-down` pages after 30 min, which is longer than the 15-minute window. | `evm.mjs:452, 492, 479-481`; `main.mjs` sequential checkers | reasoned | — |
| I-2 | Info | With `verify_quorum` 2 and ordofi refusing every `eth_getLogs`, any drpc lag gives Robinhood-only verification, which is SEV2 with no pause (by design). One lying provider plus one lagging provider can suppress the pause (`matches ≥ 1`). | `evm.mjs` deadline | reasoned | — |
| I-3 | Info | `drawEmptyAlert` treats "the monitor never tracked this draw" (`status === undefined`) as trusted (SEV2). An empty close the monitor missed pages SEV2, not SEV1. | `evm.mjs:293` | reasoned | — |

### M-A: the span ceiling ratchet

In `pagedLogs`, any error that `retryable()` does not match halves the span and sets `ceiling = min(ceiling, b − a)`. After a
success the span doubles, but only up to that ceiling, and nothing ever raises the ceiling again. So errors unrelated to the range
lower it permanently:

- `-32000 header not found` from a load-balanced backend that lags;
- an HTTP 400 or 403 from a CDN challenge;
- a proxy's HTML error page.

Thirteen consecutive errors of that kind take it to 1 block. The same `Provider` class serves the canary (`canaryProviders`). The
span lives in memory only, so a restart clears it.

What that means at 1 block a call:

- Robinhood covers ~125–250 blocks in the 25 s index budget, against ~600 new blocks a minute. The index never catches up.
- Every draw's recompute sits in "pending".
- If drpc lags too, every honest draw reaches its deadline, which means `pauseKeeper()` (economic effect: `exitAtBacking()`, and
  mints, fills and closes refused).
- Otherwise the draw is verified by one provider only (SEV2, no pause).

**Fix:**

- Lower the ceiling only on errors that are really about the range ("exceeds", "too many", "range", "limit").
- Treat any other error as a failure with backoff, and do not halve.
- Let the ceiling grow back after N successes, or decay back to `start` over time.
- Persist the learned span with the index.

### M-B: one dense block stalls the whole group

The group read puts the factories and every caught-up coin in one filter. When a page fails for a non-retryable reason, the span is
halved down to 1 block; if that still fails, `pagedLogs` returns an error and `update` breaks. The group cursor, which is persisted,
never passes that block, and every coin's cursor waits with it. Nothing splits the query by address or by coin.

Robinhood refuses a query matching more than 10,000 logs. 4663 allows 32 M gas a block (read-only ArbGasInfo:
`getMaxBlockGasLimit` = 32,000,000) at a base fee of about 0.03 gwei. A zero-value `transfer` loop costs about 3–3.5k gas a
Transfer log, so reaching 10,000 in one block is borderline (about 0.001 ETH per block). A provider with a lower per-query cap
(drpc's free plan is unmeasured) makes it cheap.

The result is a permanent, repeatable false pause at every later draw's deadline, until the index file is edited. A keeper pause
now pays burns at `β × (1 − base)`: whoever causes it can burn at the backing when the floor is below it. That is the parent
contracts review's question. A related variant: a filter with more addresses than a provider accepts (ADDRESS_BATCH is 200, and
launches are permissionless) fails the same way.

**Fix:**

- On a 1-block failure, split the group by address (down to one coin), then by `topics[1]`/`topics[2]`.
- Advance the other coins and mark only the dense coin behind.
- Never let one coin hold the group.
- Page SEV2 with the reason.

### M-E: an HTTP 101 answer hangs the indexer

Node does not emit `response` for a 101 with no `upgrade` listener. It closes the socket and emits neither `error` nor anything
the promise listens to. The `AbortSignal` is then a no-op. `guardedFetch` awaits the transport directly, so its deadline does not
help either.

In the wrapped service, `snapshot(c)` stores the build promise in `inflight`, and `build → withPublished → checkPublished` awaits
the fetch. Once the 60 s TTL has passed, every `list`, `ticket`, `draws` and `handins` call for that collection awaits the hung
promise for ever: the site loses its data for the collection (tickets, refund dates, draws).

Who can trigger it: whoever serves a draw URI (the keeper's word, the mirror, or its DNS). In the catalogue, a tokenURI or
marketplace host freezes the background `refresh()` for good.

**Fix:**

- `req.on("upgrade", (res, sock) => { sock.destroy(); no(new Refused("101")) })` and `req.on("close", () => no(...))` when not
  settled.
- In `guardedFetch`, race the transport against `deadline` (reject on abort).
- Never cache an `inflight` promise without a timeout.

### L-C: evading the floor-walk rule

The module computes the allowed move pro rata from `postedAt` (capped at 1 h), and so does the monitor's rule. A post that does not
move the floor resets both the module's clock and the monitor's streak (`dir === null` → `streaks[m] = null`). Edge, edge, no-move
(1 s later), edge, edge, and so on reaches the module's full 20 % a day with a longest run of 2.

The day-move rule measures against the most recent entry that is at least 23 h old. So:

- about 10 %/23 h pages nothing;
- 10–18 % is SEV2;
- only ≥ 18 % is SEV1, and neither makes a trigger.

**Fix:**

- Count edge posts in a sliding window (e.g. ≥ 3 of the last 4 in one direction, ignoring posts that move < 10 % of the band).
- Or sum the one-way movement over a rolling 6 h and 24 h.
- Give the day-move SEV1 the `pause_on_floor_walk` trigger.
- Add a multi-day cumulative rule (e.g. > 25 % over 72 h).

### L-F: no self-repair in the holder index

`compact()` detects missing logs only when a balance would go negative. A dropped transfer to an address that never spends leaves a
silently wrong base. The cursor has moved past it, so it is never re-read. Every later draw of that coin is "disagree" with the
honest provider, and the deadline pauses the keeper.

`resets: 1` and `broken` are saved in the index file. Nothing clears them except deleting the file, so the one reset budget lasts
for the whole life of the stack.

**Fix:**

- When a provider disagrees with the quorum on a draw, rebuild that coin from its launch on that provider before the deadline,
  instead of counting it as "disagree".
- Periodically cross-check each coin's `totalSupply()` against the index's sum of balances.
- Let `broken` and `resets` expire.

## 3. Other checks (no finding)

- **Resolver proofs:** `entryIn` refuses holder 0, malformed hex and entries outside `[first, end)`, and verifies against
  `dv.root`. `fetchText` returns null on any failure. The rebuild is cached per draw (see L-D for the negative case).
- **Keeper announce guard:** `firstEmptyBlock` covers `[now − 24 h, confirmed]`, and the unconfirmed tail is negligible. A carried
  seed reads `snapshotTimeOf(origin, randomness)` on chain, and an empty carried time is announced, opened empty and closed. A
  holder cannot steer a new seed: the window is in the past at announcement.
- **Empty draws in the monitor:** the sentinel-only tree equals `weightedTree(∅, …, sentinel)`. A false empty gives another root
  with tickets, which is bad, which is pause then cancel. An unverified empty close is SEV1.
- **Indexer:**
  - `judgeList` accepts an empty list only when `totalTickets == "0"`, and then still rebuilds to the sentinel alone (a list with
    holders gives another root).
  - `refundableAt` uses `deliverBy + 72 h` only after `Undeliverable`, else `+ 30 d`.
  - `foldDraws`: `seedPassedOn` is set only for cancels and voids of Announced draws; `DrawEmpty` is final.
- **Site `requestRefusal`:** coin and curve targets are bound to the page coin. The curve's `token()` is read on chain, and a
  missing page coin refuses. I found no bypass.
- **golive-check:**
  - The Safe proxy runtime hash and the slot-0 singleton are checked, along with the singleton's code hash.
  - `getModulesPaginated(SENTINEL, 10)` must return `[]`, and the guard slot must be zero (the same slot in 1.3.0 and 1.4.1).
  - Owners must be code-less (so 7702 delegation fails) and distinct.
  - `GOLIVE_REHEARSAL` is refused unless `cast chain-id` is 31337 or 9466301, and 4663 forces `--phase1`.
  - The only gap is L-H.
- **Keeper `/healthz`:** refusals are still an immediate 503, so nothing is hidden. The public body names only alert kinds
  (sanitised), and an outsider cannot hold it at 503.

## 4. Suites (this checkout, `npm ci` then the commands below)

| Suite | Result |
| --- | --- |
| keeper `npm test` / `tsc --noEmit` | **131 pass**, 0 fail / clean |
| monitor `npm test` | **106 pass** before the audit3 file; **111 pass** with `audit3_monitor.test.mjs` (5 PoCs, asserting the gaps) |
| ops `npm test` / typecheck | **31 pass** / clean |
| indexer `npm test` / typecheck | **123 pass** / clean; `audit3_outbound.test.ts` **5 pass** (4 variations fixed, 1 gap: M-E) |
| web-rook `npm test` / typecheck / `npm run build` / `VITE_ROOK_PHASE=1 npm run build` | **65 pass** / clean / ok / ok |
| tools/snapshot `npm test` / `tsc` | **19 pass** / clean |
| staging `npm test` / typecheck | **29 pass** / clean |
| web `npm test` / typecheck | **118 pass** / clean |
| `node --test scripts/timelock-watch.test.mjs` | **4 pass, 1 skipped** (no local replica), 0 fail |
| `bash scripts/audit2_golive_check.test.sh` | **39 ok**, 0 unexpected, exit 0 |
| `script/deploy.test.sh` | not run: it starts anvil and runs forge scripts (the parent's) |
