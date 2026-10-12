# Round 3, services: what the second review (R2) found in the monitor, keeper, reconciler, indexer, site and go-live scripts

Branch `r3-services` (from `fixes-2026-09-26` at `2bb3ee7`). Written before the code, as the design; "As built" at the end is
filled in when the code lands. Nothing here is deployed; no key, wallet or Railway service is touched. Network use is read-only
(public 4663 RPCs, the public drand relays through the fork test).

Scope: every High and Medium of `R2-services.md` §2, its partials (E-05, E-11, E-14, E-15, E-16, C-08) and the cheap Lows.
Directories: `monitor/`, `keeper/`, `tools/snapshot/`, `ops/`, `indexer/`, `web-rook/`, `script/golive-check.sh`, `scripts/`, the
Dockerfiles and the deploy docs. Not touched: `src/` (contracts: the parallel `r3-contracts` round) and `autobuyer/` (the canister:
`r3-canister`). Where their changes need a service, site or monitor change, it is listed in §9 for a follow-up.

The R2 PoCs (`review2/poc/audit2_*`) asserted the gaps. Each becomes a regression test in its package that asserts the attack
now **fails**; the file keeps its `audit2_` name so the trail is one grep.

## 0. What was measured before the design (2026-09-26, from this machine, read-only)

| Provider | `eth_getLogs` for one coin's `Transfer` | Notes |
| --- | --- | --- |
| `rpc.mainnet.chain.robinhood.com` | 1k blocks 0.11 s, 10k 0.17 s, 50k 0.31 s | refuses a query matching more than 10,000 logs (`-32000 … exceeds limit of 10000`) |
| `rpc.ordofi.network` | **every** query, 10 to 200k blocks: `-32005 the network is busy` after ~0.6–1.1 s | still fine for `eth_call` and headers |
| `robinhood.drpc.org` | 100-block spans ~0.07 s; wider spans `ranges over 10000 blocks are not supported on free plan` | no state at explicit blocks since 2026-09-24, but logs over short spans work |
| `robinhood-rpc.publicnode.com` | `Archive requests require a personal token` for any `eth_getLogs` | headers only |

4663 makes about 858k blocks a day (~9.9 blocks/s). So: one provider serves wide log ranges fast, one serves only narrow ones, and
the second configured one serves none. A design that needs "both RPCs" to read the whole history on every poll cannot work
(R2-M1); a design that keeps an index and reads only new blocks can, even on narrow spans (≈600 new blocks a minute = 6 calls of
100 blocks).

## 1. R2-M1 (High) and R2-M2: the draw-root recompute runs on an incremental holder index, a queue and a deadline

### 1.1 A holder index per provider, with a saved cursor (`monitor/src/holders.mjs`, new)

- **Which coins.** Every coin the stack's factories launch as RAFFLE (`CurveFactory.Launched(coin, curve, creator, policy, …)`,
  policy 0), read from the stack's L2 start block, plus any coin a `DrawAnnounced` names (a coin the index has never seen is
  added and back-filled from the stack's start). Each coin keeps its launch block and its curve.
- **Per provider, independently.** Each configured provider keeps its own index: a cursor per coin (the last block read), the
  balances at a base block, and the Transfer logs after it. A root is computed from one provider's index alone, so the
  providers stay independent of each other and of the keeper and its file.
- **Incremental.** A poll reads only `[cursor + 1, safe head]`, for all caught-up coins at once (`address: [coins]`, topic
  `Transfer`), paged. A coin that is behind (new, or a provider that was down) is back-filled on its own, oldest first, within a
  per-poll time budget, so the canary's own reads are never starved.
- **Bounded storage.** Logs older than the retention (`SNAPSHOT_WINDOW` 24 h + `OPEN_TTL` 6 h + 6 h margin = 36 h of blocks) are
  folded into the base balances. A snapshot block below the base is refused (never guessed). The index lives in its own file
  next to the state (`<state dir>/holders-<provider host>.json`, written atomically), not in the per-poll state.
- **Pages.** Each provider has its own page shape: a starting span (`draws.providers[i].span`, default 10,000 blocks; drpc 100),
  halved on a refusal ("exceeds limit", "too many", "range", `-32005`, HTTP 429/5xx, timeouts) down to one block, grown back after
  success. Transient errors are retried with backoff (0.5 s, 1 s, 2 s; at most 3 tries a page), then the provider is marked
  cooling down for the rest of the poll (the others go on). Nothing throws out of the poll: an unreadable range leaves the cursor
  where it was.

### 1.2 Providers: at least two, with failover

`evm.rpcs` (exactly two, which must agree for the canary) stays. The index reads from `draws.providers` (default: the two
`evm.rpcs`); the example config names three: Robinhood (span 10,000), drpc (span 100), ordofi (span 10,000, cooled down while it
answers "busy"). The `rpc()` helper gains retry and backoff on 429, `-32005`, `-32016`, 5xx and timeouts (R2 I-1), and the
canary's `bothLogs` uses `Promise.allSettled` so one throttled provider fails a range, not the poll.

### 1.3 The verdict, and the queue

- A pending draw's root is recomputed from each provider's index whose cursor has passed the snapshot block (a cheap local
  replay: no RPC but the snapshot-block binary search, which uses headers any provider serves).
  - **verified**: at least `draws.verify_quorum` providers (default 2) reproduce the on-chain root and count, and none computes
    another;
  - **bad**: at least two providers compute the same other root (or count), and none reproduces the chain's; a confirmed bad root:
    SEV1, the guardian **pauses first, then cancels** (R2-L3);
  - **disagree**: providers compute different roots: SEV2 at once, no automatic action until the deadline (below);
  - **pending**: fewer providers than the quorum have caught up.
- **Queue (R2-M2).** Every pending draw is considered every poll, ordered by `challengeEnds` (earliest first). Recomputing is
  local and cheap, so there is no two-draws-a-poll slot to hold; the index work a draw needs (its coin's back-fill) is ordered the
  same way. A draw that cannot be decided never blocks another: its failure is recorded on it and the loop goes on.
- **Deadline.** For every Open draw not verified:
  - `challengeEnds − 10 min`: SEV2 "not verified yet" (as today, but for every draw, not only those in a slot);
  - `challengeEnds − draws.deadline_s` (default 6 min): **SEV1 "draw N not verified before its pin at T"**, with the by-hand
    `cancelDraw` and `pauseKeeper` commands, and, when the guardian is configured with `auto_pause` and
    `draws.pause_unverified` (default true), **`pauseKeeper()`**. `requestDraw` reverts while the keeper is paused
    (`SweepVault.requestDraw`: `if (registry.keeperPaused()) revert KeeperPaused()`), so the draw cannot be pinned; the monitor
    keeps recomputing, cancels it if it turns out bad, and says "verified: the Safe may unpause" if it verifies. A draw verified
    by exactly one provider at the deadline (the other behind, none disagreeing) is SEV2, not a pause.
  - A draw past `challengeEnds` still Open and unverified stays paged every poll's 6-hour repeat until it is final.
- **Guardian (R2-L3).** An action is recorded as done only after its receipt shows success (waited up to 45 s; a pending
  transaction is re-checked next poll before anything is re-sent); pause triggers run before cancel triggers.

### 1.4 The start block and the exclusions come from the stack and the published list (R2-L1, R2-L2)

- `stackOf` reads the stack file's L2 `fromBlock` first, then `deployedAtBlock` (the keeper's order). A configured
  `evm.from_block` later than the stack's is refused at start (it would make honest draws recompute "bad").
- The listed exclusions are read from `tools/snapshot/exclusions.json`, shipped in the monitor image, with its sha256 printed at
  start and in `/status`. `draws.exclusions` in the config, if present, must equal that file (refused otherwise): no hand copy
  can drift into a self-inflicted cancel.

### 1.5 Measured

A script (`monitor/scripts/measure-index.mjs`) builds the index for real coins against the real providers, read-only, and prints
the back-fill and per-poll times per provider. Its numbers go in "As built".

## 2. R2-M3: a floor walked along the band's edge pages

`judgeStackLogs` keeps each module's accepted floors for 26 h (in the state). Two rules, both on `accepted`:

- **Edge run.** A post whose move from the previous accepted floor is at least 80% of what `floorBounds()` allowed (the module's
  `maxMoveBps` per hour, pro rata to the time since the previous post, read once like `minConfidenceBps`) is an edge post. `K`
  (default 3) consecutive edge posts in one direction: SEV1 "the floor is being walked at the band's edge".
- **Day move.** The accepted floor more than 10% from its value 24 h ago: SEV2; more than 18%: SEV1.

Both carry the by-hand pause. `evm.guardian.pause_on_floor_walk` (default **false**: an honest keeper following a real crash
also posts at the edge) lets the guardian pause on the SEV1s. An independent floor reference in the monitor is left open (§10).

## 3. R2-S1: the resolver never trusts a file's proofs, and never loops

`keeper/service/resolver.ts`:
- `entryIn` verifies the entry's proof against the **draw's on-chain root** (`tools/snapshot` `verify`), not the file's `root`
  field; a file whose entry does not prove falls through to the next source (the keeper's copy, then the chain rebuild).
- If the claim's simulation reverts, the same pass tries the chain rebuild once.
- A draw Resolved for more than 1 h without a claim raises `claim-stuck` (paged via §4); after `VOID_AFTER` the resolver voids it
  (the branch is now reachable: an entry that does not prove is "no entry").
- Each draw gets at most one claim attempt and one rebuild a pass; the rebuild is cached per draw, so a pass is bounded.
- The monitor pages a draw Resolved for more than 1 h (SEV2, `chain.mjs`).

## 4. R2-S8: the keeper's alerts page

- `/healthz` answers **503** when the pass's alerts have been in force for `KEEPER_ALERT_PASSES` consecutive passes (default 3)
  or a refusal is in force; its JSON body names the alert kinds (the prefix before `:`, never the text). Railway's deploy
  check is unaffected (a fresh process has no alerts). The same rule holds for the standalone resolver.
- The monitor's liveness poll reads the body when it is JSON and puts `alerts: [kinds]` in the page.
- `docs/deploy/services-v1.md` says exactly this (the false "the monitor pages on them" goes).

## 5. N-1: the indexer survives any status

`pinnedHttpsFetch` validates the status (200–599 accepted; anything else rejects the fetch with an error) and wraps the response
listener's body in try/catch that rejects. Test: the R2 PoC's local TLS server answering `HTTP/1.1 999` (and 600, 1000) now
rejects cleanly and the process lives.

## 6. Reserves, snapshots, images, runbook (E-05, E-11, E-14, E-15, E-16)

- **E-05 / R2-L4.** `ops/src/backing.ts`: the stale rule keys on the module's real freshness: RED when `fresh()` is false and
  the last **good** post (`lastGoodAt`, confidence ≥ minimum) is older than `floorTtl + staleFloorRedS`; a module frozen by
  confidence-0 re-posts is RED. **R2-L5:** a run with no successful read at all is "blind" and pages at once.
- **E-11 / R2-S5.** On 4663 with `KEEPER_SNAPSHOT_UPLOAD_URL` set, `KEEPER_SNAPSHOT_BASE_URL` is required and must not be on
  `KEEPER_PUBLIC_URL`'s host; the draw's URI is the mirror's. `publish` reads back the on-chain URI **and** the upload URL's
  object (when it is a different URL) and requires the same bytes from both.
- **E-16 / R2-S7.** The keeper image installs production dependencies only (`tsx` becomes a runtime dependency) and runs as
  `node` through `ops/docker/drop-to-node.sh`, as the monitor does. The resolver shares the image.
- **E-15 item 5 / N-3.** `web-rook/Dockerfile`'s header names the real build context (`web-rook/`); `web-rook/src/content` is
  re-synced from the docs (the "Keeper key stolen" row), and `test/copies.test.ts` covers `src/content` so it cannot drift again.
- **E-14.** A test: a rehearsal floor above `MAX_PURCHASE_WEI` (0.5 ETH) is refused / clamped as the code documents.

## 7. The go-live check (C-08, G-N1, G-N2, G-N3)

`script/golive-check.sh`:
- **G-N1.** Under `--phase1` a stack file is required (FAIL without it), and `--canister` is required unless
  `--before-reinstall` is passed.
- **C-08.** Reads and checks, per collection: `FloorModule.payee()` and `MintDesk.payee()` (the treasury), `maxEthBacked`,
  `maxPublicEthBacked`, `mintPerHour`, `burnPerHour` (against the stack file's values when it has them, and non-zero and bounded
  otherwise), and each listing's `paused` flag (Phase 1: unpaused unless the stack file says otherwise).
- **G-N2.** The owner is a real Safe: its proxy's code hash is one of the canonical Safe proxies, its `masterCopy`
  (storage slot 0) is a canonical Safe singleton (1.3.0 / 1.4.1, L1 and L2), `getModulesPaginated` is empty, the guard slot is
  zero, threshold 2 of 2 owners, each owner code-less and distinct from every role, the guardian and the treasury.
- **G-N3.** Every `call` FAILs on empty output (a read error is a FAIL, never a skip); `count()` unreadable FAILs; two failed
  reads never compare equal; `indexOf` is compared positively; a stack file without `capCeiling` FAILs under `--phase1`.
- The R2 shell PoC becomes `scripts/audit2_golive_check.test.sh` with the cases inverted (cases 1–4 now exit 1) plus a real-Safe
  case that passes.

## 8. The rest of the Lows

- **R2-S4.** An upward step of the planned floor that comes from the listings alone (sales not supporting it) is bounded:
  above the recent sale median the floor may rise at most `maxListingLiftBps` per post (default 200 bps) unless the depth
  (items within 5% of the new listing floor) is at least `k`. Measured on the recorded 2026-09-23 book (§As built).
- **N-2.** The site passes the page's coin into `requestRefusal`; a trade (curve buy/sell, router buy/sell) whose coin or curve
  is not the page's is refused.
- **R2-S2.** A relay answering other bytes no longer stops the draw when two others agree and the beacon verifies their
  signature: every distinct candidate is checked with `beacon.verify`; the accepted one is used and the liar is an alert.
- **R2-S3.** A non-cancel re-price takes the gas of the last non-cancel attempt; an attempt the node refused with an error that
  is not about price is not recorded.
- **R2-S6.** `fetchText` streams with a byte cap and refuses private addresses (the indexer's rules).
- **I-2, I-4, I-5, G-N4, G-N5, the keeper draw notes:** listed with their status in "As built".

## 9. For the other rounds (follow-ups, not done here)

- If `r3-contracts` changes `requestDraw`'s pause check, the monitor's fail-closed pause (§1.3) no longer holds the pin; the
  deadline must then cancel instead.
- Burn-side clamp and paused-burn pricing, the close gate, anti re-roll of draws and the Wrapper's late-delivery reconciliation
  may add events or views: the monitor's ABI (`abi.mjs`), the keeper's (`service/abis.ts`), the indexer's and the site's.

## 10. Runs this branch must pass

Every package's tests, typecheck and build; `scripts/golive-dryrun.sh stack`; `scripts/keeper-fork-test.sh` (public read-only
fork); the `audit2_*` regressions.

## As built

Commits on `r3-services`: the design (`0b46a84`), then keeper, monitor, golive-check, indexer + reconciler, site, and this
section. Nothing deployed; no key, wallet or Railway service touched.

### Monitor (§1, §2)

- `monitor/src/holders.mjs` (new): `Provider` (learned span, ceiling after a range refusal, cool-down 2 → 30 min after a
  throttle), `pagedLogs` (never skips; a call's timeout is the poll's remaining budget), `HolderIndex` (discovery from
  `Launched`, policy 0 only; the factories and every caught-up coin read together in one filter, batches of 200 addresses; a
  coin behind is back-filled alone, pending draws' coins first; compaction past 36 h of blocks; a coin whose logs go negative is
  rebuilt once, then marked broken and its draws stay unverified), `indexesFrom` (`draws.providers`, files
  `<state dir>/holders-<host>.json`).
- `abi.mjs` `rpc()`: retries 429 / 5xx / `-32005` / `-32016` / timeouts / resets with backoff (0.5 s, 1 s), then throws.
- `evm.mjs`: the canary pages each provider with its own span (`draws.providers` span for the same URL) and stops its cursor at
  the first range either cannot read (stall paged after 30 min as before) instead of throwing the checker. Draws are also read
  from each RAFFLE vault's state (`drawCount`, `draw(i)`, from the lowest not final, 50 a poll) and confirmed on RPC B by
  `eth_call`, so `eth_getLogs` is not needed to find one. Every undecided draw is recomputed each poll, earliest window first;
  `verdictN` with `draws.verify_quorum` (2). Deadline at `challengeEnds − draws.deadline_s` (360 s): SEV1 "DRAW N NOT VERIFIED
  BEFORE ITS PIN AT T" and, with the guardian's `auto_pause` and `draws.pause_unverified` (default true) and a draw confirmed on
  both RPCs, `pauseKeeper()`. Verified by one provider only at the deadline: SEV2, no pause. Pinned while unverified: SEV1.
  Triggers: pause before cancel; an unfinished one (not mined, or failed) is retried each poll for 24 h.
- Floor walk (§2): `EDGE_RUN` 3, `EDGE_FRAC_BPS` 80 %, `maxMoveBps` read from the module (default 500); day move 10 % SEV2,
  18 % SEV1; `evm.guardian.pause_on_floor_walk` (default false).
- `guardian.mjs`: waits up to 45 s for the receipt; success → done; reverted → throws (SEV1 act by hand); not mined → pending.
- `stack.mjs` `startBlock`: `fromBlock` first, config only earlier; `config.mjs` refuses a later `evm.from_block`, reads
  `tools/snapshot/exclusions.json` (shipped in the image; sha256 in the start line and `/status`) and refuses a drifted
  `draws.exclusions`; validates `draws.providers` (≥ 2), `verify_quorum`, `deadline_s` (120–899), `pause_unverified`.
- `chain.mjs`: a draw Resolved for more than 1 h is SEV2 (R2-S1). `liveness.mjs`: a non-200 page carries the body's
  `alertKinds` (sanitised tokens only) (R2-S8).
- `config.example.json`: `evm.rpcs` Robinhood + drpc (ordofi refuses every `eth_getLogs`); `draws.providers` Robinhood
  (span 10,000), drpc (span 100), ordofi (10,000, cools down while busy).

### Measured on the real providers (2026-09-26, `monitor/scripts/measure-index.mjs`, read-only)

A live ERC-20 with ~2–3k transfers a day stands in for a coin (no Rook stack on 4663 yet).

| Provider (span) | Back-fill | One poll after (new blocks) | Root (headers, replay) |
| --- | --- | --- | --- |
| Robinhood (10,000), 2 h of history | 0.56 s, 8 `eth_getLogs` | 0.16 s, 1 call (597 blocks) | 1.1 s, 18 headers |
| Robinhood, 7 days (6.0M blocks, 1,232 transfers) | 43.6 s, 609 calls | 0.18 s, 1 call (734 blocks) | — |
| Robinhood, 24 h of a busier coin (347 transfers) | 5.8 s, 86 calls | 0.18 s, 1 call (348 blocks) | 1.5 s, 22 headers |
| drpc (100), 2 h | 153 s, 713 calls (~0.2 s each) | 8.9 s, 21 calls (2,094 blocks; a 60 s poll is ~600 blocks, ~6 calls) | 1.3 s, 18 headers |
| ordofi (10,000), 2 h | read 7 pages in 61 s, then `-32005` and cooled down | 0.4 s, 0 calls (cooling) | 4.3 s, 18 headers |

So: after the one-time back-fill (Robinhood ~6 min for 60 days; drpc ~30 min per day of history, which is why the index runs
from the stack's first block, when the history is short), a poll costs one call on Robinhood and a handful on drpc, and a root
is a second or two of headers plus a local replay: well inside the 15-minute window, with the deadline at 6 minutes before the
pin. Wave 2's design needed ~5 pages a day of history per provider per poll and could not finish on ordofi at all.

### Keeper (§3, §4, §6, §8)

- R2-S1: `entryIn` proves each entry (`ticketLeaf` + `verify`) against the draw's on-chain root; file → keeper copy → chain
  rebuild (cached per draw); a reverted claim simulation tries the rebuild once in the same pass; `claim-stuck` after 1 h;
  `voidDraw` after `VOID_AFTER` is reached.
- R2-S8: `/healthz` 503 after `KEEPER_ALERT_PASSES` (3) passes with alerts, or a refusal; body `alertKinds`, `alertPasses`,
  `alertPassesLimit`; the same in `resolver-main.ts`. `services-v1.md` §2 and `p1-services.md` §1.6 say so.
- R2-S5: on 4663 with an upload URL, `KEEPER_SNAPSHOT_BASE_URL` is required and must not be on `KEEPER_PUBLIC_URL`'s host;
  `publish` reads back the on-chain URI and the mirror's object and requires identical bytes (`KEEPER_SNAPSHOT_UPLOAD_READBACK=off`
  for a write-only mirror).
- R2-S7 / E-16: `npm ci --omit=dev` (tsx is a runtime dependency), runs as `node` via `drop-to-node.sh`; built with Docker,
  uid 1000, `/data` owned by node, keeper and resolver both start.
- R2-S4: `liftCeiling` in `planPost` (`liftBpsPerHour` 200, `liftDepth` 5): a rise with no sales support and < 5 listings
  deep follows at most 2 %/h. On the recorded 2026-09-23 book, buying the cheapest listing: one hour's post +4.99 % → +1.99 %
  (target +7.27 %, confidence 3,901); 3 listings: target +81.8 % at 2,725 bps, not posted either way.
- R2-S2: each distinct relay signature is checked with `beacon.verify`; the accepted one (served by ≥ 2 relays) is used; the
  others raise `drand-liar`. R2-S3: a non-cancel re-price uses the last non-cancel attempt's gas; a non-price refusal is rolled
  back and alerted. R2-S6: `fetchText` streams under the 20 MB cap and refuses private addresses (`service/outbound.ts`, the
  indexer's rules). E-14: `ops/test/offer.test.ts` holds a ~0.6 ETH frog under a 0.66 ETH cap refused by `MAX_PURCHASE_WEI`.

### golive-check (§7)

G-N1 (stack file and `--canister` required under `--phase1`; `--before-reinstall` makes the canister a WARN), G-N2 (proxy code
hash 1.3.0 or 1.4.1, singleton in slot 0 among the canonical 1.3.0 / 1.4.1 L1 and L2 with matching code, no module, no guard,
2 of 2, code-less distinct owners that are no role, guardian, treasury, ops or custodian; optional `--safe-owners A,B`),
G-N3 (empty reads FAIL, `count()` unreadable FAILs, failed reads never compare equal, `indexOf` ≥ 1, `capCeiling` required),
C-08 (`payee()` of module and desk, `mintPerHour` / `burnPerHour` / `maxPublicEthBacked` / `maxEthBacked` against the stack or
the Phase-1 defaults, the listing's pause). Every canonical Safe deployment has code on 4663 (read on the Robinhood RPC); 1.5.0
is not accepted (a module guard the check does not read). The dry run's stand-in Safe passes only under `GOLIVE_REHEARSAL=1`,
which golive-check refuses on any chain but 31337 / 9466301.

### Indexer, reconciler, site

N-1: statuses outside 200–599 reject the fetch; the listener is wrapped (tested with 999, 600, 1000, 199 against a local TLS
server; the process lives). R2-L4 / R2-L5 as designed. N-2: `requestRefusal` takes the page's coin (the coin, the curve's
`token()`, or the router call's coin must be it; no page coin refuses every trade); `Coin.tsx` → `Ticket` → every `TxButton`.
E-15 item 5 / N-3: `web-rook/Dockerfile` names `railway up ./web-rook --path-as-root`; `src/content` re-synced; `copies.test.ts`
fails on any difference or extra file.

### Runs

| Run | Result |
| --- | --- |
| keeper `npm test` / typecheck | 125 pass (115 + 10 audit2) / clean |
| tools/snapshot `npm test` / `tsc` | 18 pass (17 + audit2 tree equality) / clean |
| monitor `npm test` | 93 pass (79 + 8 audit2 + 5 holder index + 1 guardian receipt) |
| ops `npm test` / typecheck | 31 pass / clean |
| indexer `npm test` / typecheck | 117 pass / clean |
| web-rook `npm test` / typecheck / both builds | 60 pass / clean / clean |
| staging `npm test` / typecheck | 29 pass / clean |
| `scripts/timelock-watch.test.mjs` | 5 pass |
| `scripts/audit2_golive_check.test.sh` | 26 ok: R2's cases 1–4 now FAIL with their reasons, controls unchanged, a real Safe + complete stack passes, each new check FAILs alone, `GOLIVE_REHEARSAL` refused on 4663 |
| `script/deploy.test.sh` | 37 ok |
| `STACK_PORT=8731 DRYRUN_NO_DRAND=1 scripts/golive-dryrun.sh stack` | **ALL PASS**, 40 ok (the `--phase1` read under `GOLIVE_REHEARSAL=1`: the stand-in Safe and the missing `--canister` are WARNs on chain 9466301 only). A first run in parallel with the keeper fork test clobbered the shared `deployments/*-9466301.json` files: run them one after the other |
| `KEEPER_FORK_PORT=8741 scripts/keeper-fork-test.sh` | **ALL PASS**, 43 ok (real drand; step 9 skipped: no dfx) |

### Left open

- **The contracts round:** the fail-closed deadline relies on `requestDraw` reverting while the keeper is paused. If
  `r3-contracts` changes that ("close gate vs pause"), the deadline must cancel instead. Its new events or views (burn-side
  clamp, paused-burn pricing, anti re-roll, late delivery) need the monitor's, keeper's, indexer's and site's ABIs.
- `script/DeployStack.s.sol` should write `payee`, `itemUnits` and `listingPaused` into the stack file; golive-check assumes
  payee = custodian and a 1e18 unit until then. The dry run could plant a real Safe proxy instead of `GOLIVE_REHEARSAL`.
- An independent floor reference in the monitor (R2-M3's second suggestion); R2-S4 slows an unsupported pump to 2 %/h, it
  does not stop it (the module's ±20 %/day is the bound).
- The keeper's `outbound.ts` checks DNS before the fetch but does not pin the socket (one-TTL rebinding window; impact: a
  failed GET, since every entry must prove against the chain). The public `/healthz` now names alert kinds.
- drpc is the only second provider that serves logs today, on 100-block spans; ordofi throttles every `eth_getLogs`. A paid or
  own node as a third provider would make the quorum sturdier. The monitor's back-fill must start at deploy.
- The monitor image was built (Docker): `/app/tools/snapshot/exclusions.json` is shipped and is what `config.mjs` reads. Not run here: a fork run driving the
  keeper and the monitor together, the canister (`r3-canister`).
- Info items unchanged: I-2 (reconciler false RED in a fast rise), I-4 (`no-good-post` before launch), I-5 (> 32 disputed
  ranges throws, paged), G-N4 (`setGuardian` role separation), G-N5 (dry run FAIL paths), N-4 (fails closed), the keeper's
  first-24 h snapshot note and a missed slot across a restart, the indexer's own single-provider reserves check.
