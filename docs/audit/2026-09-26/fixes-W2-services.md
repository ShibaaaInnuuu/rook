# Wave 2, services: the keeper, the resolver, the snapshot tool, and the canister's dead keeper path

Branch `w2-services` (from `fixes-2026-09-26` at `d41e28b`, which carries every contract fix). Written before the code, as the
design; "As built" at the end is filled in when the code lands. Nothing here is deployed; no key, wallet, Railway service or live
chain is touched (read-only public RPC and the public drand relays only).

Scope: `fixes-B-C-keeper-draws.md` §5 items 1–4, 6 and 11, and from `E-services-workflow.md` E-01, E-04, E-09, E-10, E-11, E-14 and
the off-chain halves of E-02 (the keeper repairs a confidence-0 post) and E-03 (something resolves). Directories: `keeper/`,
`tools/snapshot/`, `scripts/keeper-fork-test.sh` and `scripts/golive-dryrun.sh` where they drive the keeper, `docs/v6/p1-services.md`
and `docs/deploy/services-v1.md` (the keeper sections), and `autobuyer/canister-rs/src/custodynet.rs`. Not touched: `monitor/`,
`ops/`, `script/golive-check.sh`, the runbook (wave 2, monitor), `web-rook/`, `indexer/` (wave 2, site), and `src/` (contracts).

## 1. Draws: announce, open with the seed round, sentinel leaf, published off the keeper's volume (E-11)

The contract flow is `announceDraw(coin)` → the pinned `seedRound` exists → `openDraw(id, root, totalTickets, uri, sentinelProof,
seedSignature)` within `OPEN_TTL` (6 h, both clocks). The keeper:

1. **Announces at the slot.** Slots stay `T_k = k × KEEPER_DRAW_EVERY_S + KEEPER_DRAW_OFFSET_S` on the chain's clock (daily 15:00
   UTC). Within `KEEPER_DRAW_WINDOW_S` of the slot, for each RAFFLE coin with `units ≥ 1e18`, `drawInFlight(coin) == 0` and no
   draw of this coin announced at or after `T`: `announceDraw(coin)`. A due coin that is not announced by the end of the window is
   an **alert** (`draw-slot-missed`), never a silent skip. Announcing is refused (and alerted) when the snapshot cannot be
   published off the keeper's volume (step 5) or the coin has no ticket at the head (nothing could be drawn).
2. **Reads the seed round from the chain.** Every pass, each coin's in-flight draw is read with `draw(drawInFlight(coin) − 1)` at
   `latest`: the mined announcement, never a simulation (the rehearsal's `InvalidSignature` lesson). A draw in `Announced` is
   opened as soon as `seedRound` exists.
3. **Fetches the seed signature from two drand relays that must agree.** `KEEPER_DRAND_URLS` (default `api.drand.sh`,
   `drand.cloudflare.com`, `api2.drand.sh`; all serve evmnet, measured). At least two answers, byte-equal; any relay that answers
   differently is a refusal (BLS signatures are unique, so one of them is lying). The signature is then checked with
   `beacon.verify(round, sig)` by `eth_call`, which returns the randomness; `snapshotAt = vault.snapshotTimeOf(id, randomness)`
   (the contract's own view, so the keeper never re-implements the hash).
4. **Snapshots at the last L2 block with timestamp ≤ `snapshotAt`** (binary search on 4663's own numbering: the first block with
   timestamp ≥ `snapshotAt + 1`, minus one). The tree is `tools/snapshot`'s weighted tree plus the sentinel leaf
   `leaf(address(0), totalTickets, snapshotAt)`; the file is v2 (below) and carries the sentinel and its proof.
5. **Publishes, then opens.** The file is written to `KEEPER_STATE_DIR/snapshots/` (served at `/snapshots/<name>` as before) **and**
   uploaded to a mirror: `KEEPER_SNAPSHOT_UPLOAD_URL` (https; `PUT <base>/<name>`, optional `Authorization` from
   `KEEPER_SNAPSHOT_UPLOAD_AUTH`, a secret). The draw's `uri` is `KEEPER_SNAPSHOT_BASE_URL/<name>` (default
   `KEEPER_PUBLIC_URL/snapshots`). Before `openDraw` the keeper GETs the `uri` and requires the exact bytes (sha256): the URI on
   chain is live before the draw is. In send mode on 4663 a draw is **not announced** without an upload URL (floors and caps are
   unaffected). Claims never depend on the file alone: the resolver can rebuild it from chain (item 2).
6. **E-11, the scan.** `transferLogs` splits a range adaptively (halving on "exceeds limit" / "too many" / -32005 answers, down to
   one block) and never skips a failure: it throws, the draw stays Announced, the pass reports an alert, and the next pass retries.
   Scans are incremental: each coin's Transfer logs are cached in `KEEPER_STATE_DIR/logs/<coin>.json` up to a cursor that stays
   ten minutes behind the head, so a draw reads only the blocks since the last one, not from genesis.
7. **Opening late.** The keeper opens only while the announcement is under `OPEN_TTL − 15 min` old on both clocks; past that the
   draw is left for `voidDraw` (anyone, after 6 h; the resolver does it), and an alert says so.

`openDraw`'s ABI, `announceDraw`, `drawInFlight`, `snapshotTimeOf`, `draw(id)` (18 fields), `DrawAnnounced` and `SnapshotFixed` are
added to `service/abis.ts`; the old `openDraw(coin, root, total, uri, snapshotAt, snapshotBlock)` goes.

## 2. A keyless resolver (E-03)

`service/resolver.ts`, `resolverPass(deps)`: only permissionless calls, no role needed.

- **Wrapper tickets, in order.** From `nextToResolve()`: a Pinned ticket whose round's signature two relays agree on → `resolve(id,
  sig)`; the next one is read after the receipt. The first ticket whose round does not exist yet stops the walk (order is the
  contract's). Per pass at most `KEEPER_RESOLVER_MAX_TX` sends (default 10).
- **RAFFLE draws.** From a cursor (the lowest draw id not Claimed/Cancelled/Voided), per stage:
  `Open` past the challenge window on both clocks and the keeper not paused → `requestDraw`; `Pinned` whose round exists →
  `resolve`; `Resolved` → `claim` for the winner, with the entry taken from the published file (fetched from `uri`, https, size
  cap, and accepted only if its root is the on-chain root) or, failing that, **rebuilt from chain** with `tools/snapshot` at the
  draw's `snapshotAt` and accepted only if the root matches; `Announced` past `OPEN_TTL`, `Pinned` past `PIN_TIMEOUT`, or `Resolved`
  past `VOID_AFTER` with no provable claim → `voidDraw`.
- **Where it runs.** Inside the keeper process after the keeper's own work (`KEEPER_RESOLVER=on`, the default), using the keeper's
  sender; and standalone, `npm run resolver` (`service/resolver-main.ts`), with its own funded key `RESOLVER_PRIVATE_KEY` (read in
  `service/signer.ts` only), `RESOLVER_MODE=send` to send, its own state directory. The standalone refuses a key that is the
  registry's named keeper (it would fight the keeper for nonces) and needs no floor feed and no public URL. It keeps running while
  the keeper is paused; the keeper process does not (item 4). So no redemption or raffle depends on the keeper key.
- Gas: `resolve` proves a BN254 signature (~180k gas for `prove`, measured by forge); the resolver's calls use
  `KEEPER_RESOLVER_MAX_GAS_PER_TX` (default 900,000) instead of the keeper's 400,000, within the same fee and money caps.

## 3. Posting the floor (E-02a, the ±20 %/day band, E-01, E-09)

- **Repair.** `planPost` gains the module's `fresh()`, posted `confidenceBps()` and `minConfidenceBps()`. When the module is not
  fresh, or its posted confidence is under the minimum, a feed floor that passes the policy is **posted even at the same price**
  ("repair"), never "kept". A thief's confidence-0 post is overwritten on the next pass.
- **Bands.** `floorBounds()` is now required (the module's intersection of ±5 %/h and ±20 % of the day's anchor); the service
  refuses to plan a floor for a module that does not answer it. The plan is clamped into the intersection of the chain's bounds and
  the keeper's own hourly pro-rata reading, as before.
- **E-01, wash sales.** `robustFloor` gets two rules, both pure and both tested on the recorded 2026-09-23 Ordinals Wallet book:
  1. `S-item-repeat`: one sale per item in the window (the most recent). Moving one frog around wallet pairs counts once.
  2. **Sales can pull the floor at most `maxSalePullBps` (500 = 5 %) below the listing floor while sales come from one
     marketplace** (`saleSources < 2`; Ordinals Wallet only today). With two or more independent sale sources the old blend
     (0.7·L + 0.3·S) applies. The listing side is the part an attacker must pay to move (each fake-low listing can be sniped); the
     sales side costs only fees, so its influence is capped at one hour of the module's movement. The sales still lower confidence
     through the agreement term, so a real crash that listings have not caught up with shows as low confidence (no post, the
     module goes stale: the safe failure).
  Consequence on the recorded book: the honest floor moves from 503,683 to at least 0.95 × 565,262 sats. A higher floor makes mints
  dearer and burns are β-capped, so it errs to the protocol's side; caps and `maxItemCost` rise with it by the same ≤ 5 %.
- **E-09.** `AgreedRate` returns the **higher** sats/ETH of the two references, which is the lower floor in wei, as §1.4 says.

## 4. Caps and the pause (C-01, the incident switch)

- `planCap`: `capCeiling() == 0` → "cannot post" (the vault reverts `NoCeiling`); never above the ceiling; up by at most 20 % of
  the last cap on every post, fresh **or stale** (a stale re-post may fall freely).
- `registry.keeperPaused()` is read every pass; while it is true the pass is refused (nothing sent, nothing simulated as the
  keeper). The sender also re-reads it immediately before signing, so a pause that lands mid-pass stops the next signature.
  `keeperPaused`, `namedKeeper`, `guardian` join the registry ABI.

## 5. Bounded passes and rate limits (E-04)

- **Coins incrementally.** The factory's coins are indexed in `KEEPER_STATE_DIR/coins-<factory>.json` (`coin`, `policy`, `curve`
  are immutable): each pass reads only coins launched since, at most 200 per pass, with a concurrency cap. RAFFLE coins' `units` are
  read only within the draw window (once per slot, then only for the due ones); in-flight draws are found by `drawInFlight`.
- **Order and time box.** Every collection's floor and caps run first; draws, then the resolver, run after, and stop when the pass
  has used `KEEPER_PASS_BUDGET_S` (default 240): the floor is never behind a draw.
- **Rate limits.** `ChainRpc` treats JSON-RPC answers that are really outages — HTTP 200 with code 429, -32005, -32016, or a
  message like "too many requests" / "network is busy" / "rate limit" — as an endpoint failure: the next endpoint is tried, and after
  all of them a short backoff and at most two more rounds. A revert is still an answer. "exceeds limit" on `eth_getLogs` goes back
  to the caller, which splits the range (item 1.6). `sendRaw` fails over the same way (same bytes).

## 6. A stuck transaction is re-priced (E-10)

The pending record keeps every attempt (hash, bytes, fees). `resolvePending` looks for a receipt of **any** attempt. When the newest
attempt has waited `KEEPER_STUCK_AFTER_S` (default 180 s), or its re-broadcast is refused as underpriced / below the base fee, the
sender signs a **replacement for the same nonce**: the same call if it still simulates, otherwise a zero-value self-transfer that
clears the nonce; fees `max(old × 1.125 + 1, 2 × base + tip)` for both fee fields (the 10 % replacement rule), within the fee cap and
the per-transaction and per-day money caps (the reservation is re-priced). A transaction that is stuck and cannot be re-priced
within the caps is an **alert** (`tx-stuck`), shown in the pass report and `/healthz`, and still re-broadcast each pass. The state
file becomes version 2; version 1 is read and upgraded.

## 7. Small hardening (E-14)

- `exclusions.json` unreadable or malformed → the snapshot fails (and the draw with it, alerted); never `[]`.
- `KEEPER_PUBLIC_URL`, `KEEPER_SNAPSHOT_BASE_URL`, `KEEPER_SNAPSHOT_UPLOAD_URL` and the drand relays must be https (plain http only
  for this machine, as for RPCs, so the fork test runs locally).
- `/status` answers a public summary only (mode, chain, keeper address, passes, last pass, counts of refusals and alerts, health).
  The full state (pending transaction, history, refusal texts, errors) is `/status/full`, served only with
  `Authorization: Bearer <KEEPER_STATUS_TOKEN>` (≥ 32 characters, compared in constant time; unset: the full view is off). No
  wildcard CORS on it. The token and the upload credential are scrubbed from every log line like the keys.

## 8. The canister's dead keeper path

`Wrapper.recordDelivery` is custodian-only on chain, so `custodynet.rs`'s `may_record` fallback ("the keeper too, if it ever is
us") can only produce a transaction that reverts. It goes: `may_record` is true only when the wrapper's `custodian()` is the
canister's address. The now-unused `redeem::KEEPER` selector goes with its selector test entry. The keeper service has no
`recordDelivery` (a guard test pins that). The canister's wasm hash changes: a new reproducible build is needed before any reinstall.

## 9. The snapshot tool (§5 item 11)

- `core.snapshot(…, { snapshotAt })` (weighted only) adds the sentinel leaf and outputs `version: 2`, `sentinel: { leaf, proof }`,
  `inputs.snapshotAt`. Without it the output is v1, byte for byte (the committed Solidity fixtures do not change).
- CLI: `--time T` resolves the block as the last block with timestamp ≤ T and adds the sentinel; `--stack <file> --slug <slug>`
  derives the exclusions of a coin on our own curve exactly as the keeper does (shared code, `src/stack.ts`); `--vault V --draw-id N`
  reads the draw (coin, `snapshotAt`, root, total) from chain, rebuilds, and exits 3 when the root differs — the monitor's
  recompute. Log reads split adaptively and retry rate limits; an unreadable exclusions file is an error.
- The rules stay in `docs/snapshot-v1.md`, whose "Version 2" section (written with the contract fixes) gains the tool's interface.

## 10. Tests and runs

Keeper: the fake chain learns the new draw flow, `keeperPaused`, the ceiling, the beacon's `verify`, `snapshotTimeOf`, the Wrapper's
tickets and the resolver's calls; new tests per item (announce → open with the sentinel and a verified two-relay signature; relays
that disagree; a slot missed is an alert; a truncated root is refused by the fake as by the chain; the resolver requests, resolves,
claims, voids, walks tickets in order; a confidence-0 post is repaired (the E-02 PoC inverted); wash sales on the recorded book move
the floor ≤ 5 % (E-01 inverted); the rate is the higher sats/ETH (E-09 inverted); a stuck, underpriced transaction is replaced at the
same nonce and mined (E-10 inverted); passes read O(new coins) (E-04 inverted); rate-limited answers fail over; the log scan splits
at the cap; exclusions fail closed; `/status` is a summary without the token). Snapshot tool: sentinel, `--time` block rule, v1
unchanged. `forge test` if anything Foundry reads changes; canister `cargo test`; `scripts/golive-dryrun.sh stack` on a local fork.

## As built

Commits on `w2-services`: the design (this file, alone), then the code, tests and docs. As designed, with these precisions:

- **Item 1.** `service/draws.ts` (snapshot, publish, `DrawBook`), `service/drand.ts`, `service/logs.ts`, `service/coins.ts`, and the
  draw half of `service/keeper.ts`. The seed round is read from `draw(id)` at `latest` on every pass (the mined announcement); the
  announced draws are found by `DrawBook`'s walk of the vault's draws (from the lowest not final), not coin by coin.
  Before announcing, the coin's tickets at the head are counted (from the log cache) and a coin with none is not announced. The
  snapshot file keeps v1's name (`snapshot-<coin>-<block>-weighted.json`); the URI is `KEEPER_SNAPSHOT_BASE_URL/<name>`.
  `openDraw` gets the resolver's gas cap (it proves the seed round, a BN254 pairing). Alerts: `draw-slot-missed`,
  `draw-not-announced`, `draw-not-opened`, `draw-empty`, `snapshot`, `publish`, `drand`, `drand-late`.
- **Item 2.** `service/resolver.ts` and `service/resolver-main.ts` (`npm run resolver`). A draw's URI is untrusted input for the
  resolver: https only on 4663 (plain http to this machine only on other chains, for the fork test), no redirects, 20 MB cap, and
  used only when its root and ticket count are the draw's; otherwise the tree is rebuilt from the chain. Resolving tickets while
  the keeper process also resolves is safe (the second simulation reverts, nothing is sent); running both costs at most a reverted
  transaction on a race.
- **Item 3.** `src/floorpost.ts` `needsRepair`; `src/floor.ts` `S-item-repeat` and `maxSalePullBps` (500) with `saleSources`;
  `service/feed.ts` `AgreedRate` returns the higher sats/ETH. On the recorded book: floor 536,999 sats at 5,204 bps (was 503,683 at
  4,900); the audit's attack (11 wash sales at ⌈L/4⌉+1 on fresh pairs) now moves it ≤ 5 % (was −13 %); one frog churned 11 times
  counts once. Two real flips in the recorded week also count once now (the sale floor moves 360,000 → 382,330).
- **Item 4.** `service/plan.ts` `planCap`; the pause in `identityProblems` (first, alone) and the sender's `guard`, read again
  immediately before every signature, including a replacement's (none is signed while paused).
- **Item 5.** `ChainRpc` fails over on 429 / -32005 / -32016 and throttle messages in HTTP 200, then backs off (two more rounds);
  `sendRaw` too. The pass is ordered (floors and caps for every collection, then draws, then the resolver) with
  `KEEPER_PASS_BUDGET_S`. Draws are walked by `DrawBook` from the lowest id not final; coins by `CoinIndex` (200 a pass).
- **Item 6.** `service/sender.ts` `resolvePending` / `replace`; `service/state.ts` version 2 (`attempts`, `firstSentAt`; version 1
  upgraded on read). Alerts `tx-replaced`, `tx-stuck`.
- **Item 7.** `service/config.ts` (https for the public URL, the mirror, the base URL and the relays; the token's length),
  `service/status.ts` (`/status` summary, `/status/full` bearer, `/healthz` counts), `tools/snapshot/src/stack.ts`
  `readExclusions` (fail closed). `KEEPER_STATUS_TOKEN` and `KEEPER_SNAPSHOT_UPLOAD_AUTH` join the scrubbed secrets.
- **Item 8.** `custodynet.rs` `may_record` is custodian-only; `redeem::KEEPER` and its selector check are gone; the
  `pending_delivery_records` query stays (monitoring) with its prose corrected. **The canister's wasm changes**: a new
  reproducible build and its hashes are needed before any reinstall (`fixes-D-canister.md`'s hashes are for `4e8c78f`).
- **Item 9 (the tool).** `tools/snapshot/src/{core,rpc,stack,cli}.ts`, the v2 fixture `test/fixtures/snapshot-weighted-v2.json`
  and `test/SnapshotToolV2.t.sol` (the vault's `leaf` and OpenZeppelin's `MerkleProof` accept the tool's sentinel and ranges).
- **Scripts.** `scripts/keeper-fork-test.sh` step 5 follows the new flow end to end (announce, open with the real drand signature,
  recompute with `--vault/--draw-id`, then the in-process resolver requests, resolves and claims); the fork is now taken ~25 h back
  (`KEEPER_FORK_BACK`, default 900,000 blocks) so the holders predate the snapshot window.

**Existing tests changed, and why.**

| File | Change | Why |
| --- | --- | --- |
| `keeper/test/floor.test.ts` | the recorded frog book's floor, sale floor, confidence and depth; BAYC's repeat counts | E-01 rules (item-repeat, the pull cap) |
| `keeper/test/service-feed.test.ts` | the rate is the higher sats/ETH; the recorded wei floor; caps: a ceiling in every state, stale +20 %, no ceiling → skip | E-09, E-01, C-01 |
| `keeper/test/service-guards.test.ts` | the key readers include `RESOLVER_PRIVATE_KEY`; the write lists per role (keeper: postFloor, postCap, announceDraw, openDraw; resolver: permissionless only); no `recordDelivery` anywhere | items 2, 8 |
| `keeper/test/service-keeper.test.ts` | the module-keeper refusal's wording; the two old draw tests move to `service-draws.test.ts` in the new flow | the registry-read keeper; C-03/C-04 |
| `keeper/test/service-fake.ts` | rewritten for the new contracts (registry pause, beacon, draw flow, tickets, relays, mirror) | all |
| `tools/snapshot/test/core.test.ts` | the fixture list gains v2; the read-only check covers `stack.ts`, `giveaway.ts` | item 9 |

**Runs** (on the final tree):

- `keeper`: `npm test` **115 passed, 0 failed** (96 before; new: `test/service-draws.test.ts` 12, `test/w2-hardening.test.ts` 9);
  `tsc --noEmit` clean.
- `tools/snapshot`: `npm test` **17 passed, 0 failed** (12 before; new: `test/v2.test.ts` 5); `tsc --noEmit` clean.
- `forge test`: **996 passed, 0 failed** (108 suites; the new `test/SnapshotToolV2.t.sol`; fork suites return early without a URL).
- `autobuyer/canister-rs`: `cargo test` **459 passed, 0 failed**.
- `STACK_PORT=8697 scripts/golive-dryrun.sh stack` (fork of 4663 from the public archive RPC, all local): **DRY RUN: ALL PASS**
  (golive-check 120 checks ok; announce → open with the seed round's real drand signature → pin → resolve with the pinned round's
  real signature → claim).
- `scripts/keeper-fork-test.sh` (fork ~25 h back, all local): **KEEPER FORK TEST: ALL PASS**, 43 ok; step 5 is the keeper's own
  draw end to end with real drand relays: announced, opened (the beacon verified the relays' signature; snapshot at the last block
  ≤ `snapshotAt`, v2 file read back from its URI), recomputed independently by `tools/snapshot --vault/--draw-id` (same root, same
  bytes), then requested, resolved and claimed by the in-process resolver. Step 9 (a local replica) was skipped: no `dfx` here.

**Left open (user decisions or later work).**

1. **The snapshot mirror.** Draws on 4663 need `KEEPER_SNAPSHOT_UPLOAD_URL` (+ auth) and `KEEPER_SNAPSHOT_BASE_URL`: pick the
   store (e.g. an R2 bucket behind a small PUT-authenticated worker, or any WebDAV/HTTP PUT host). Until then the keeper announces
   no draw there (it says so every slot).
2. **The standalone resolver's key and home.** A small funded account (no role) in a Railway project other than the keeper's.
3. **`KEEPER_STATUS_TOKEN`** if anyone (the monitor) should read `/status/full`; otherwise only the public summary is served.
4. **E-01's pull cap (5 %)** is a policy number: with a second independent marketplace feeding sales (`saleSources ≥ 2`) the old
   blend applies. The honest recorded floor rises from 503,683 to 536,999 sats with it.
5. **The canister's wasm hash** changes with item 8: rebuild reproducibly and record the new hashes before any reinstall.
6. Not done here: a KMS signer for the keeper (E-02's custody half), the monitor's recompute/paging (wave 2, monitor), the site's
   Resolve/Claim buttons (wave 2, site).
