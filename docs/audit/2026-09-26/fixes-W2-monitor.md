# Wave 2: the monitor, the reconciler, the go-live check and the runbooks

Branch `w2-monitor` (from `fixes-2026-09-26`, which carries every contract and canister fix). Written before the code, as the
design; the "As built" section at the end is filled in when the code lands. Nothing here is deployed; no key, wallet, Railway
setting or live chain is touched (read-only public RPCs only, for measurements and the dry run's fork).

Scope, from the fix docs' hand-over lists:

- `fixes-B-C-keeper-draws.md` §5 items 7–10 (monitor and reconciler), 15 (go-live check), 16–17 (runbooks), and the §3 powers table;
- `fixes-D-canister.md` §6 "Off-chain services" (the monitor's decoder, `TICKET LOGS NOT READ`, the canary's txids, the timelock's
  `NotStopped`/`NotStarted`, the operator's fee-input funding);
- `fixes-A-graduation.md` "Left open" (the new graduation views, printed by the go-live check);
- `E-services-workflow.md` E-05, E-06, E-08, E-13, E-15, E-16 (the monitor's and the ops images);
- `C-treasury-roles-deploy.md` C-07, C-08; `B-floor.md` B-05.

Out of scope here (other wave-2 branches): `keeper/`, `tools/snapshot/`, the canister's dead `may_record` path (w2-services);
`web-rook/`, `indexer/`, the Trust page (w2-site). This branch reads their interfaces and changes none of their files. The one file
under `autobuyer/` it touches is the monitor's fixture generator, `autobuyer/canister-rs/examples/watch_fixtures.rs`.

## 1. The monitor (K6): what it watches after the fixes

### 1.1 The 4663 events (§5 item 7)

The EVM canary already reads every log of its addresses from **two** RPCs and judges only what both return. Its address list
grows from "the wrappers and the module" to the whole stack: the registry, every module, every wrapper and every sweep vault
(RAFFLE, BURN, YOYO). The configuration may name them one by one (as today) or give the stack file (`evm.stack`, inline, or
`evm.stack_file`), from which the monitor derives them. New matchers:

| Event (emitter) | Alert |
| --- | --- |
| `FloorPosted(target, accepted, moveBps, confidenceBps, depthItems)` (module) | confidence below the module's `minConfidenceBps`: SEV2 (E-02a: a thief can freeze the module with confidence 0; the stale fallback ends it after 7 days). **`target` differs from `accepted` by more than 50 bps**: SEV1 "a post outside the module's bounds" — an honest keeper plans inside `floorBounds()` (§5 item 3), so a clamped post is a post the keeper's plan could not have made. Two clamped posts in the same direction within 6 hours are a **runaway floor** (the guardian's trigger, §1.4) |
| `CapPosted(cap, at)` (vault) | on the YOYO vault: SEV1 (the keeper never posts it in Phase 1). On RAFFLE/BURN: SEV2 when the cap is more than 125% of the module's accepted floor (the keeper posts floor × 1.1) |
| `DrawAnnounced(id, coin, seedRound)` | INFO, once; the draw is tracked from here |
| `DrawOpened(id, coin, root, totalTickets, uri, challengeEnds)` + `SnapshotFixed(id, snapshotAt)` | **the root is recomputed** (§1.3); a mismatch is SEV1 and the guardian's trigger |
| `KeeperPaused(by, paused)` (registry) | paused: SEV1 (persistent while `keeperPaused()`); resumed: SEV2 |
| `GuardianSet(guardian)` | SEV2 (who can pause changed) |
| `RolesProposed(keeper, purchaser, attestor, readyAt)` | SEV1: a 7-day role rotation has started; expected only in a keeper-key incident (runbook row, §4) |
| `RolesSet` | INFO |

### 1.2 Polled state (§5 item 9, E-08)

Every poll reads, at the agreed safe block of RPC A, and **confirms on RPC B before a SEV1 page** (a disagreement is a
`provider-disagreement` alert, never a guess):

| Condition | Alert |
| --- | --- |
| `registry.keeperPaused()` | SEV1, persistent (repeats every 6 h) |
| `module.escaped()` | SEV1, persistent: the stale fallback is live (7 days without a good post) |
| `module.fresh() == false` for more than 15 min | SEV2, persistent ("the floor is stale since …; mint, fills and sweeps are refused") |
| no good post (`lastGoodAt`) for more than 45 min | SEV2, persistent |
| RAFFLE or BURN `capFresh() == false` for more than 15 min while the module is fresh | SEV2, persistent |
| a `Wrapper` ticket **Pinned** and its drand round more than 1 h in the past | SEV2, persistent: the resolver (keyless, §5 item 2) is not running. Refundable after 7 days (E-03) |
| a draw **Announced** more than 1 h ago, or **Pinned** with its round more than 1 h in the past | SEV2, persistent: the keeper did not open it / nobody resolved it |

Round → time uses drand evmnet (genesis 1727521075, period 3 s), as `src/rand/DrandEvmnet.sol`. At most 200 tickets and 200 draws
are read a poll (from `nextToResolve` and from the lowest draw not yet final), so a flood of draws cannot make a poll unbounded.

### 1.3 Recomputing a draw's root (item 7, "on a second RPC")

The root is recomputed **independently of the keeper and of its published file**, from public data, exactly as the keeper's rule
(`keeper/service/draws.ts`, `docs/snapshot-v1.md` plus v2 of `fixes-B-C-keeper-draws.md` §6):

1. the snapshot block is the last block whose timestamp is ≤ `snapshotAt` (binary search on block headers);
2. the coin's `Transfer` logs from the stack's deploy block to the snapshot block, paged adaptively (ordofi refuses more than
   10,000 blocks a query, Robinhood more than 10,000 logs: a refused page is retried at half the span);
3. balances; the exclusion set built by the monitor from the stack (0x0, 0xdead, the coin, its curve, the factory, lock, router,
   pool manager, the three vaults, the module, the desk, the treasury, the registry) plus the listed exclusions
   (`tools/snapshot/exclusions.json`, copied into the config as `draws.exclusions`);
4. weighted tickets (one per whole coin, 18 decimals), sorted leaves `keccak256(keccak256(abi.encode(holder, first, end)))`,
   **plus the sentinel leaf** `(address(0), totalTickets, snapshotAt)`, OpenZeppelin's commutative pair hash.

The merkle code is a port of `tools/snapshot/src/core.ts` (the monitor has no TypeScript toolchain in its image); a test holds
the port to the committed fixtures `test/fixtures/snapshot-weighted.json` byte for byte, so the two cannot drift silently.

It is done **on both RPCs**. Outcomes: both give the on-chain root → INFO "root verified"; both give the same **other** root →
**confirmed bad root** (SEV1; the guardian's trigger); the two recomputations differ from each other → SEV2 provider
disagreement, no automatic action; a recomputation fails → retried every poll, and SEV2 if the draw is still unverified 10
minutes before its challenge window ends. Also SEV1 without recomputing: `totalTickets` on chain ≠ the recomputed count.

### 1.4 The guardian (item 8), optional and off unless configured

**What it is.** `ListingRegistry.guardian()` can do two things: `pauseKeeper()` and `SweepVault.cancelDraw(id)` for a draw not yet
pinned. Both only stop things (§3 of the keeper doc: "denial of service only"). The Safe unpauses and can replace the guardian at
once.

**Where the key lives.** In the monitor's own Railway project (never the keeper's, never the operator runner's), as the service
variable `MONITOR_GUARDIAN_KEY`, read in one file (`monitor/src/guardian.mjs`), scrubbed from every error, never logged, never in
`/status` or the state file. The config must also name the guardian's address (`evm.guardian.address`); a key whose address is
not that one, a guardian address with code (EIP-7702), or a key with no config (or the reverse) **refuses to start**.

**When it acts.** Only on a confirmed trigger, and each action at most once per trigger:

| Trigger (confirmed as in §1.1–1.3) | Action |
| --- | --- |
| confirmed bad root on a draw that is Open or Announced | `cancelDraw(id)`, then `pauseKeeper()` |
| runaway floor: two clamped `FloorPosted` in the same direction within 6 h | `pauseKeeper()` |

Before sending, it re-reads `registry.guardian()` (must be its address), `keeperPaused()` (skip if already paused) and the draw's
stage (skip if not Open/Announced), simulates with `eth_call`, signs an EIP-1559 transaction for chain 4663 (the chain id is read
from both RPCs and must be the configured one), under a gas and fee cap, and sends it to both RPCs. Every action pages SEV1 with
its transaction hash; a failed action pages SEV1 "act by hand" with the exact `cast send` for the Safe. `evm.guardian.auto_pause`
and `auto_cancel` default to true once the key is configured; either may be turned off to keep the key for manual use only.

**The decision left to the operator** (recorded in `phase1-scope.md` §3.3 as O11): run with a guardian (recommended: a keeper
theft is then answered in seconds) or without one (then only the Safe, on two devices, can pause, and a bad root is answered by
hand within the 15-minute window). Without one, `golive-check.sh --phase1 --no-guardian` turns "guardian unset" from a FAIL into a
WARN, and the operator writes the choice in the ceremony log. The key needs a little ETH on 4663 for gas (two transactions).

### 1.5 Service liveness (E-08)

Railway checks `/healthz` only at deploy. The monitor now polls each service's health URL (`liveness: [{ name, url }]` in the
config: keeper, the standalone resolver, runner, reconciler, indexer) every poll; a service that has not answered 200 for 10 minutes is SEV2, persistent
(repeats every 6 h), and its recovery is INFO. The keeper's and runner's `/healthz` already turn 503 on refusals or failed
ticks, so this also pages "running but refusing".

### 1.6 E-06: a disagreement never freezes the canary

- A disputed chunk no longer stops the scan. Its range is kept in `disputed` (at most 32, else the checker errors), the cursor
  moves past it, and every later chunk is judged as before. A disputed range is re-read every poll; the logs **both** RPCs return
  in it are judged at once (an unbacked mint both show is caught in the disputed range too), and the whole range is judged when
  they agree.
- The alert key is the range (not its counts), persistent: repeated every 6 h; after 1 h also "STILL disagreeing".
- A disputed range older than 1 h marks the checker **degraded**: `/status` `ok: false`, `/healthz` 503.
- A cursor that has not moved for 30 min while both heads moved is a `watcher-error` and degrades the checker.

### 1.7 E-08: alerts fail closed

- `Telegram` refuses to be built without both `ALERT_TELEGRAM_BOT_TOKEN` and `ALERT_TELEGRAM_CHAT_ID` unless the caller asked for
  a dry run explicitly (`--dry-run`). The monitor and the reconciler exit 2 at start ("refusing to start: …"). A missing token is
  never "sent" again.
- The reconciler gains `--test-alert`: one RED, one stale and one recovered message through its own channel, so its variables
  are tested, not the monitor's. The runbook's test step names both commands (and the monitor's new kinds).

### 1.8 The canister side (fixes-D §6)

- **Decoder.** `Init` learns `holder_deposits : opt bool` and `wrapper_from_block : opt nat64`. The diff: `holder_deposits` from
  off to on is CRITICAL LOOSENS (the deposits pass, returns and their limiter flow come alive); on to off TIGHTENS;
  `wrapper_from_block` changed is WARN (where the ticket-log scanner starts). An argument encoded before 2026-09-26 still decodes
  (the fields are `opt`), and a baseline without them means "null".
- **Limiter.** `Flow` gains `Return`; `Outflow` gains `acked : opt vec Act`. The freeze alert names the acknowledged keys.
- **Records.** `RPhase` gains `Conflicting` and `Signed`/`Sent` carry `bump`; `FPhase` gains `Abandoning`; `DPhase` gains
  `ReturnConflicting`, `AwaitingProof`, `Parked`. Every variant tag is listed (an unknown tag is a decode error, which would stop
  the canary).
- **The custody canary vouches for every txid the canister saved:** a delivery's current txid **and every `bump.earlier`**; a
  conflict-out's `conflict.tx.txid` (the replacement, spending the fee input back to custody) and its `leaked_txid` and `earlier`;
  a float abandon's conflict (`FPhase::Abandoning.conflict`), and a return's conflict. A delivery version that confirmed is
  checked against the destination like the current one.
- **The scanner.** `ticket_logs()`: `failures > 0` continuously for more than 1 h is SEV2, persistent; `log()` lines with
  `TICKET LOGS NOT READ` are SEV2 (keyed by the line). `holder_deposits()` in `Parked` is INFO (the operator may hand in the
  proof or refuse it). `ord_providers()` without UniSat is SEV2, persistent (E-08: the key is lost at every upgrade; deliveries
  and buys then refuse as "unverified").
- **The timelock.** `NotStopped` (the buyer did not stop; nothing installed; the proposal is pending again) is SEV2;
  `NotStarted` (installed, but the buyer did not start again: nothing delivers) is SEV1. Both in watcher v1's words too
  (`scripts/timelock-watch.mjs`). The buyer is briefly **Stopped** during every executed upgrade: its queries fail for that
  moment, and the checkers' 30-minute silence rule already absorbs it.
- **Fixtures.** `examples/watch_fixtures.rs` gains an argument with the two new fields, a redemption with a bump and a conflict,
  a frog buy abandoning, a parked deposit and an outflow with `acked`, encoded by the canister's own types; `monitor/fixtures/
  canister.json` is regenerated with `cargo run --example watch_fixtures`.

## 2. The reconciler (K7): E-05, E-08a, E-13

- **The module's real ETH (E-05).** Per collection, at one pinned block: `eth_getBalance(module)` must be ≥ `backing + reserve +
  surplus + revenue + gradEth + rentalEscrow + renterPool + owedTotal` (the module's own books, as the invariant test
  `_books()` states them), else RED. `reserveUnits ≤ ethBacked`, and `backing == 0` when `ethBacked == 0`, else RED. Escrow
  (`backing + reserve`) per ETH-backed unit below 50% of the accepted floor is RED (`RECONCILE_MIN_BACKING_BPS`, default 5000).
  The page and JSON show `surplus` and the balance beside the books.
- **A stale floor is RED (E-05).** `fresh() == false` for more than an hour past the TTL, or `escaped()`, is RED with the time
  of the last good post. (The monitor pages it after 15 minutes; the reserves page turns red after an hour.)
- **Two providers for the location (E-13).** The indexer's check (unchanged) asks ordinals.com where an inscription is. The
  reconciler hands it a fetch that answers that request only when **Ordinals Wallet** (`turbo.ordinalswallet.com/inscription/<id>`,
  keyless) names the same address and the same satpoint; otherwise the request fails and the item is "unverified". So one lying
  ord can no longer keep a missing frog green, nor make the page red on demand.
- **Unverified is paged (E-08a).** "Unverified" for more than 6 hours (`RECONCILE_UNVERIFIED_ALERT_S`) alerts SEV2, repeating
  like red; a run that could read **no** collection alerts at once ("cannot read").
- **The watchdog is its own timer (E-08a).** It no longer shares the loop with `runOnce`, and `runOnce` is bounded
  (`RECONCILE_RUN_TIMEOUT_S`, default 900): a hung run is abandoned, logged, and the report goes stale and red on the watchdog.

## 3. The go-live check (C-08, B-05, item 15)

`script/golive-check.sh` gains `--phase1` ("step 7 is done: this is the go/no-go read") and `--no-guardian` (the operator's
written choice, §1.4). **Always FAIL** (any mode): `module.registry() ≠ registry`; `module.wrapped() ≠ wrapper`;
`registry.keeperPaused()`; the named keeper (`namedKeeper()`, so a pause does not hide it) ≠ the file's; at stage 0 treasury
shares ≠ 9000/0/0/1000 or `bootstrapLoan ≠ 0`; a pending `transferOwnership` (`pendingOwner()`), pending roles, treasury, bond,
listing root or BTC vault; a pending cap-ceiling raise or mint-limit raise; a vault ceiling other than the stack file's.
**FAIL with `--phase1` or on 4663, a note before step 7 otherwise:** a wrapper not wired (`floorModule() == 0`); a vault with
`capCeiling == 0`; a collection not in the treasury or directory; unset ops; the registry owner not a Safe with
`getThreshold() == 2` and two owners (an EOA, or an account with EIP-7702 code, is not a Safe); a role address (named keeper,
purchaser, attestor, custodian, ops, guardian) with code, including a 7702 delegation; the guardian unset (WARN with
`--no-guardian`), or equal to another role. **Printed:** `factory.launchesOpen()`, and for every launched coin its curve's
`canClose()`, `canGraduate()` and `refundsOpenAt()` (A-03), with `module.hasRoomFor(itemBudget)`.

`scripts/golive-dryrun.sh stack` then shows both directions on the fork: after the rehearsal's own owner steps (plus the guardian)
and with the fork's dev-account delegations cleared from the role addresses and a stand-in 2-of-2 Safe planted at the owner
(`script/local/StandInSafe.sol`, anvil only), `golive-check.sh --phase1` passes; each of these then makes it FAIL, one at a time
and restored after: the keeper paused, the guardian unset, the Safe at 1-of-2, a pending ownership transfer, a role with code, and
a stack whose owner steps were skipped (a second stack deployed without them).

## 4. Runbooks and docs (item 10, items 16–17, E-15)

- `docs/v5/launch-gates.md` §4 and `docs/deploy/key-ceremony.md`: the **keeper key stolen** row becomes: the guardian or the Safe
  `pauseKeeper()` (instant) → `cancelDraw(id)` for every draw not pinned → `proposeRoles(new keeper, …)` → 7 days →
  `acceptRoles()` → `unpauseKeeper()`. Burns continue throughout (at most the backing per unit), and from day 7 without any floor
  (the stale fallback). The old row ("does not recover the FloorModule") is gone because the module now reads the registry.
- `docs/v6/phase1-scope.md` §3: go-live step 7 drops `setShares`/`setBootstrapLoan` (constructor defaults) and the ceiling
  acceptance (the first ceiling applies at once), and gains `setGuardian` (or the written no-guardian choice); step 8 runs
  `golive-check.sh --phase1`; O6 adds the guardian EOA; O7 says the custody fee pool is funded (a plain ≥ 20,000-sat output, then
  `add_custody_fee_input`) instead of "custody: nothing"; K6/K7 rows record this wave.
- `docs/deploy/services-v1.md`: the reconciler's new variables, Telegram required, the test alert for **both** services
  (E-15 item 2), `/healthz` described as deploy-time only with the monitor's liveness polling as the continuous check (E-15 item 1).
- `docs/v6/k6-watcher-canary.md`: the new checkers, the guardian, the healthz wording, the new alert kinds.
- `docs/v6/g10-incident-drill.md`: a keeper step (the guardian or the Safe pauses the keeper on a rehearsal stack, the monitor's
  SEV1 arrives, the Safe unpauses), and the test-alert count updated.

## 5. E-16: images as non-root

The monitor and both ops images run the service as the image's `node` user (uid 1000). Railway mounts volumes owned by root, so
the images whose service has a volume (monitor, reconciler) start a tiny entrypoint that, as root, makes `/data` writable by
`node` and then drops to it with `su-exec` before `node` starts; nothing else runs as root. The runner (no volume) is `USER node`
outright. The reconciler's image stops installing the indexer's dev dependencies (it runs under `tsx`, which the ops package
already carries).

## 6. Tests

`cd monitor && npm test`, `cd ops && npm test` (and the existing `scripts/timelock-watch.test.mjs`), plus
`scripts/golive-dryrun.sh stack` (fork of 4663 from the public archive RPC, all local, `DRYRUN_NO_DRAND=1` if the drand wait is
not wanted). New tests: every row of §1.1–1.8 (including E-06's audit PoC, which must now fail: the unbacked mint past a
disputed chunk is alerted), the guardian's refusals and its one-shot actions against a fake RPC, the root recomputation against
the committed fixture and a tampered root, the reconciler's balance and stale rules, the two-provider location, the unverified
page, the hung run, and Telegram failing closed in both services.

## As built

Commits on `w2-monitor`: `0b770d4` (this design), `53ea373` (monitor), `b187baa` (reconciler and the ops images), `77845bc`
(go-live check and its dry run), `f9eadf5` (runbooks and docs), `d1ab789` (the guardian-mismatch page), and this section.

**As designed**, with these differences and precisions:

- **Guardian start-up.** A preflight failure at start (an RPC down, the registry naming another guardian) is a warning, not a refusal:
  a monitor must not crash-loop over its optional key. Instead the chain checker pages every poll (SEV2, `guardian`) while the
  registry's guardian is not the configured one, and every action runs the preflight first (a failure pages "ACT BY HAND NOW"
  with the Safe's exact call). A half-configured guardian (a key without config, config without a key, a key for another
  address, no registry) still refuses to start.
- **Draw roots.** The recomputation does not fetch the keeper's published file at all: it builds the tree from the chain
  (the coin's `Transfer` logs up to the last block at or before `snapshotAt`) and the monitor's own exclusion set, which is the
  keeper's rule (`keeper/service/draws.ts` `exclusionsFor`) over the stack plus `draws.exclusions`. A keeper that excluded a real
  holder, truncated the count or moved the time produces a root neither RPC reproduces. At most two draws are recomputed a poll;
  a recomputation that fails is retried every poll until 6 h after the window, and said 10 minutes before the window ends.
- **The canister fixtures** in `monitor/fixtures/canister.json` had not been regenerated since the audit-D types changed; they are
  now (`cargo run --example watch_fixtures`, with a bumped delivery, a conflict-out, a float abandon, a parked deposit, a frozen
  outflow with `acked`, a failing `ticket_logs` and an argument with the two new fields), and the audited build's encodings are kept
  in `canister-pre-audit-d.json`, which the decoder must still read.
- **`Delivered` on 4663** is matched against every version of the delivery (the recorded txid may be an earlier, re-priced one), and
  its alert now says `recordDelivery` is the custodian's alone (B-03).
- **E-16.** The reconciler's image already installed the indexer with `--omit=dev`; nothing to change there. The shared entrypoint is
  `ops/docker/drop-to-node.sh` (su-exec). Built and run locally: the monitor and reconciler run as uid 1000 and write to a
  root-owned volume; the runner is `USER node`.
- **The reconciler's stale rule** skips a module that has never posted and backs nothing (a stack before its launch), so the
  keeper fork test's pre-launch reconciliation stays green. `scripts/keeper-fork-test.sh` runs the reconciler with `--dry-run`
  (it refuses to start without Telegram now).
- **The go-live check.** The file's guardian is compared with the registry's only when one is set (none is the no-guardian rule's);
  `treasury.ops` unset while the stack names one is a step-7 FAIL under `--phase1`, a note before. A missing file argument is a FAIL.
  The rehearsal knob for a skipped step 7 is `ETCH_REHEARSE_UNMADE_OWNER_STEPS=1` in `script/DeployStack.s.sol` (our chains only:
  the owner's steps are printed, as on 4663, instead of made).

**Tests.**

- `cd monitor && npm test`: **79 pass, 0 fail** (48 before). New: `test/stack.test.mjs` (the cast-encoded stack events, the
  runaway floor, caps, draws, pause, guardian and roles; the tree against `tools/snapshot`'s committed fixture and the sentinel
  against `cast`; the recomputation under a 10,000-block log limit; the verdicts; the guardian acting once on a confirmed bad root,
  doing nothing on a disagreement, "act by hand" without one), `test/guardian.test.mjs` (every start-up refusal, the chain id, 7702
  code and registry checks, the signed transaction decoded and its signer recovered, the gas and fee caps, the key in no error or
  JSON), `test/health.test.mjs` (the chain checker's pages confirmed on the second RPC, liveness, the canister's health, the new
  Init fields and records, the timelock's new events), and in `test/evm.test.mjs` / `test/alerts.test.mjs` the **E-06 and E-08b
  PoCs inverted** (the unbacked mint past a disputed chunk is alerted, the dispute repeats and degrades `/healthz`; a missing token
  refuses to start).
- `cd ops && npm test`: **28 pass, 0 fail** (20 before); `tsc --noEmit` clean. New `test/w2.test.ts`: E-05 (books, units, floor
  fraction, stale, escaped, one-block reads), E-13 (the indexer's own `checkInscription` through the two-provider fetch), the
  **E-08a PoC inverted** (168 hourly runs reading nothing: 28 pages, at once then every 6 h), a partial unverified paged after 6 h,
  E-08b, the hung run and the watchdog, the reconciler's own `--test-alert`.
- `node --test scripts/timelock-watch.test.mjs`: 5 pass.
- `scripts/golive-dryrun.sh all` (bare, fork, refusals; port 8674): **ALL PASS**.
- `scripts/golive-dryrun.sh stack` (fork of 4663 from `rpc.ordofi.network`, all local): **ALL PASS**, including the new step:
  `golive-check.sh --phase1` passes on the correct deploy (guardian set, role keys code-less, a stand-in 2-of-2 Safe) and is
  refused, one at a time and put back, with the keeper paused (by the guardian), no guardian (and passes as a WARN with
  `--no-guardian`), a 1-of-2 Safe, a pending ownership transfer, the keeper's key 7702-delegated, and a cap-ceiling raise pending;
  and on a second stack whose step 7 was skipped it notes before step 7 and FAILS under `--phase1` (wrapper not wired, no cap
  ceiling, not in the treasury). Run twice: with `DRYRUN_NO_DRAND=1`, and in full (the draw opened with the seed round's real drand signature, pinned, resolved, one whole wX claimed).
- Docker: `monitor/Dockerfile`, `ops/Dockerfile.reconciler`, `ops/Dockerfile.runner` built; each runs as `node`; both alerting
  images refuse to start without the Telegram variables and print their test alerts with `--dry-run`.

**After merging `fixes-2026-09-26`** (w2-services, w2-site; one conflict, `p1-services.md` §1.6, resolved to both texts): the
monitor, ops and watcher-v1 suites re-run green (79, 28, 5; `tsc` clean); the standalone resolver joins the liveness list;
`phase1-scope.md` §8.3 now describes the site's per-argument `requestRefusal` (E-07) instead of the old target-only check.
`scripts/keeper-fork-test.sh` (after merging, on a fork of 4663): **KEEPER FORK TEST: ALL PASS**, 43 ok, including its K7 step: the reconciler (now with the module's real ETH, the two-provider location fetch and `--dry-run`) reads the graduation's ETH-backed units and reconciles the collection green, unverified as a whole without a canister, and sends nothing. Its step 9 (a local replica) was skipped: no dfx here.

**Not run:** `ops/scripts/runner-replica-test.sh` (the runner is unchanged but for its image's user) and `script/deploy.test.sh`
(DeployStack gained only the rehearsal knob; `forge build` is clean).

## Left open, and the operator's decisions

1. **The guardian (O11 in `phase1-scope.md` §3.3).** Recommended: yes, a fresh code-less EOA held only by the monitor's Railway
   project, named by `setGuardian` at go-live step 7, with a little ETH. Without it, `golive-check.sh --phase1 --no-guardian` and a
   line in the ceremony log; a bad root or runaway floor is then answered by hand (the monitor pages the Safe's exact calls). The
   thresholds that trigger it are constants in `monitor/src/evm.mjs` (`CLAMP_BPS` 50, `RUNAWAY_MS` 6 h): change them before deploy
   if wanted.
2. **The runaway rule reads the keeper's intent from the clamp.** It depends on the keeper service planning inside
   `floorBounds()` (keeper doc §5 item 3, the w2-services branch). If the service ever posts raw targets, every out-of-band move
   would page SEV1, and two would pause: merge the keeper's planning fix with this.
3. **The draw recomputation follows the v2 snapshot rules** (`docs/snapshot-v1.md` "Version 2"), which w2-services implemented in
   `tools/snapshot` and the keeper. After the merge of `fixes-2026-09-26` the monitor's port is held to the tool's committed v2
   fixture (`test/fixtures/snapshot-weighted-v2.json`: root, count and sentinel leaf, byte for byte), and its exclusion set is the
   same function as the tool's `stackExclusions` (the same fifteen addresses plus the listed ones). The alerts give the by-hand
   check, `npm run snapshot -- --vault … --draw-id … --stack … --slug … --exclusions exclusions.json --rpc …` (exit 3 on a
   mismatch). Still worth one rehearsal draw through the real keeper and the monitor together before go-live: the stack dry run's
   `phase open` uses a fixture root, not a published snapshot.
4. **E-15 item 4** (the rate direction in `p1-services.md` §1.4) is the keeper's (w2-services); items 5 and 6 are the site's.
