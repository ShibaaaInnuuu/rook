# Phase 4: the data layer

Written 2026-09-21, before its code. Phase 5 (the website) is being built at the same time in a separate worktree and owns `web/`;
this phase owns `indexer/`, `keeper/`, this document and the CI jobs for those two packages, and touches nothing under `web/`.

Nothing here is deployed, signs, or sends to a public chain. There is still no Etch deployment on any public chain, so everything
below is exercised against a local anvil fork (chain id 31337) and the `deployments/31337.json` the Phase 2 run writes.

## 1. What is built

**`indexer/`** — one Node 22 process: a follower that turns our contracts' events into a SQLite file (`node:sqlite`), and a
read-only HTTP API over that file. TypeScript, `viem` for ABI coding only; the JSON-RPC client is our own and refuses every method
that is not a read, the same list the keeper's dry run uses plus `eth_getBlockByHash`.

**Keeper hardening** — structured logs, a health endpoint, a crash-safe journal, configuration that refuses contradictions,
backoff on RPC errors, and dry-run as the default mode. It still cannot sign anywhere but chain 31337.

**A demo** — `npm run demo` at the repo root runs the Phase 2 local end-to-end against anvil and leaves the indexer and API
serving that data on localhost; a static copy of the API's responses is committed under `indexer/fixtures/demo/` for the website's
labelled demo mode.

**Amended after Phase 12.** That committed copy is no longer the anvil run described above. A recording of one coin and two
vaults showed the demo as an empty site, so `fixtures/demo/` is now recorded off the **staging fork** (chain 9466301) with
`npm run fixtures -- demo <api url>`: the 24 most recent coins, each with its detail and all five chart ranges. The contracts
and the fee arithmetic there are real; the coins and trades are a simulator's, so the prices are simulated, and every Bitcoin
value is still a fixture. `index.json`'s note says exactly that.

## 2. The follower

### Where it reads from
`deployments/<chainid>.json`. The chain id is asked of the node (`eth_chainId`); if there is no file for it, or the file has no
`registry`, the follower **idles**: it serves `/v1/health` with `deployment: false` and every list empty. It never invents a row.
It starts from the file's `fromBlock`. (`deployedAtBlock` in the local file is the L1 block number the fork reports inside
contracts — Robinhood Chain is an Arbitrum chain, and `block.number` in a contract is not the L2 height. `fromBlock` is the L2
height `eth_getLogs` understands; the follower uses only that.)

### What it follows
Static addresses from the file: `registry`, `launcher`, `distributor`. Dynamic addresses learnt from events: each listing's
**vault** (`Listed`) and each coin's **router** (`Launched`). Each pass over a block range is two reads: the static addresses
first, which may reveal new vaults and routers, then every known vault and router over the same range, so a vault listed and
deposited to in one block is not missed. Logs are applied in `(blockNumber, logIndex)` order.

| Contract | Events kept |
|---|---|
| ListingRegistry | `Listed`, `Paused`, `RolesProposed`, `RolesSet`, `RootProposed`, `RootSet`, `BtcVaultProposed`, `BtcVaultSet`, `TreasuryProposed`, `TreasurySet`, `BondBpsProposed`, `BondBpsSet`, `Wired` |
| Launcher | `Launched` |
| FeeRouter (per coin) | `Harvested`, `TreasuryDeferred`, `TokenForwarded` |
| AssetVault (per listing) | `Deposited`, `CapPosted`, `Declared`, `Cancelled`, `Released`, `Charged`, `Receipted`, `Settled`, `Returned`, `WrittenOff`, `BondPosted`, `BondWithdrawn`, `BondSlashed` |
| Distributor | `Chosen`, `DrawOpened`, `DrawCancelled`, `DrawPinned`, `DrawResolved`, `Won`, `DrawVoided`, `DestinationSet`, `Delivered`, `DeliveryOverdue`, `DistributionOpened`, `DistributionCancelled`, `ShareClaimed`, `RolledOver`, `MinimumPosted`, `PayoutRequested`, `PayoutRecorded`, `PayoutOverdue` |

A cap has no "expired" event: expiry is `CapPosted.expiry` compared with the clock at read time, and the API says which.

### Reorgs, restarts, duplicates
- The **events table is the only source of truth** and is append-only, keyed `(block_number, log_index)`. Every other table is a
  fold over it. Applying an event twice is impossible (the key), so a restart mid-range is harmless.
- The follower indexes only to `head − CONFIRMATIONS` (default 20 on a public chain, 0 on 31337).
- It keeps the hash of every block it took events from and of its cursor block. Each pass first asks the node for the cursor
  block's hash; on a mismatch it walks back through the stored hashes to the newest one the node still agrees with, deletes every
  event above it, **rebuilds every derived table from the events that remain**, and resumes. At our scale a full rebuild is
  milliseconds; it is chosen over clever undo logic because it cannot be subtly wrong.
- RPC errors back off 1 s → 60 s and never crash the API; `/v1/health` shows the last error's class and age.

### What is read from the chain besides logs
Block timestamps for blocks that carried events (needed for 24 h / 7 d windows and time-to-deliver). Once per coin, at its
`Launched`: `name()`, `symbol()` and Pons's `getLaunchedToken(coin)` for the **creator tax** and the pool key, because our own
event does not carry the tax. **Live reads**, cached 15 s and clearly separated in the responses under `live`: a router's balance
and its Pons escrow balance (what `harvest()` would move), a vault's `attributed`, `waiting`, `reserved`, `bond`, `outstanding`,
`capWei`, `capExpiry`, `isBlocked`, a coin's `unspentOf` and `queued`, the curve's `graduated()`, and the hook's pending buckets
(below). If a live read fails the field is `null` and `live.error` says so; indexed fields are unaffected.

## 3. Derived numbers, each with its formula

All amounts are wei as decimal strings. `take(coin) = 0.0070 + creatorTaxBps / 10000` — the share of trading volume that reaches
a coin's router, **measured** in Phase 1 (fork: 0.699% at 0% tax; eight live graduated coins within ~2% of 0.70% + tax, curve and
pool alike). It is a measurement of Pons's present behaviour, not a constant of ours; the response carries it as an input.

**Per coin**
- `lifetimeFeesWei` = Σ `Harvested.claimed` of its router. `toVaultWei`, `toTreasuryWei` likewise (80/20 by the router's constant;
  the API reports what the events say, not 80% of a total).
- `fees24hWei`, `fees7dWei` = the same sum over harvests whose block timestamp is within the window. Harvests are lumpy — fees sit
  in the curve or the hook until swept — so a quiet window means "not harvested", not "no trading". The response says so.
- `shareOfAcquisitions[]` = for each acquisition charged to it, `Charged.amount` of that purchase for this coin ÷ Σ `Charged.amount`
  of that purchase (for an inscription the contract assigns a whole item to one coin; the share is of the purchase's ETH).
- **`volumeUntilNextBuy`** — the headline. The keeper's published policy declares a purchase when the listing's spendable balance
  covers it: `target = max(capWei, 0.05 ETH)` for inscriptions (one item under the cap, and never under the policy's minimum
  declaration), `0.05 ETH` for a rune (any amount over the minimum buys raw units). With
  `spendable = attributed − reserved` (live), `w = unspentOf(coin)` if the coin is not yet queued else 0, and `MIN_ENTRY = 0.01 ETH`:
  `depositNeeded = max(0, target − spendable − w, (queued ? 0 : MIN_ENTRY − w))`
  `volumeNeededWei = depositNeeded / (0.80 × take(coin))`
  Assumptions, returned with the number: only this coin trades (other coins' fees on the same listing shorten it); the cap posted
  now is still the cap then; the take stays what was measured; fees already earned but not yet swept or harvested are **not**
  counted unless they are in escrow or on the router (those are subtracted from `depositNeeded` at 80%, and shown). It is `null`,
  with the reason, when no unexpired cap is posted, the listing is paused or blocked by an overdue receipt, or the tax is unknown.

**Per listing** — vault balances (live), cap and `capExpired`, open declarations with `releasableAt` and seconds remaining, released
purchases with their receipt `deadline`, `bondCoverage = bond ÷ outstanding` (null when nothing is outstanding), acquisitions and
their delivery state, `timeToDeliver` = median and max seconds from `Won`/`PayoutRequested` to `Delivered`/`PayoutRecorded`,
`writtenOffWei`.

**Protocol** — coins, listings, total claimed, to vaults, to treasury, spent (released − returned), acquisitions by kind, draws by
state, deliveries done and overdue.

**Fees pending in the pool, not yet swept.** Readable, so exposed: after graduation the API returns the hook's
`pendingFees(poolId, currency)` and `pendingCreatorTax(poolId, currency)` for both currencies, with
`poolId = keccak256(abi.encode(native, coin, poolFee, tickSpacing, hook))` from Pons's own record. The ETH-side amounts are wei.
The **coin-side amounts are in the coin**, and their ETH value is unknown until Pons's operator sweeps and converts them; the API
does not price them. While any coin-side fee is pending only Pons's operator can sweep (Phase 1), which is why this number exists:
it is income that is real and not yet ours to move. Before graduation fees accrue inside the curve; **no getter for that amount
has been verified**, so the API returns `curvePending: null` with that reason rather than a guess.

## 4. The API

Read-only `GET`, JSON, no auth, no cookies, no PII (addresses are public chain data). `Access-Control-Allow-Origin: *`.
`Cache-Control: public, max-age=5` on lists and details, `no-store` on health. Per-IP token bucket (default 120 requests a minute,
burst 40), `429` with `Retry-After`. Only `GET`/`HEAD`/`OPTIONS` are answered; anything else is `405`. Behind a proxy the client IP
is the first `X-Forwarded-For` entry only when `TRUST_PROXY=1`.

`/v1/health`, `/v1/stats`, `/v1/coins`, `/v1/coins/:address`, `/v1/listings`, `/v1/listings/:id`, `/v1/acquisitions`,
`/v1/draws`, `/v1/draws/:id` (`:id` is `<listingId>-<acquisitionId>`). Shapes and examples: `indexer/API.md`; types:
`indexer/types.ts`, a single file with no imports so the website can copy it.

### Verification hints, and the calls we do not make
Every settled acquisition carries `verify`: the Bitcoin explorer URL for its txid (mempool.space), the inscription's page
(ordinals.com) or, for a rune, the rune's page and **the path on a public indexer that shows the listing's Bitcoin address's
balance of that rune** (UniSat's documented `GET /v1/indexer/address/{address}/runes/{runeid}/balance`), plus a
`check` object a program can act on (`{kind, txid, address, runeId|inscriptionId, expect}`). These are **links and instructions
for the reader**. The indexer itself makes **no request to UniSat or Magic Eden**: both venues' terms require written permission
for commercial use (docs/phase1.md), and we have none yet. The only hosts this package ever contacts are the configured chain RPC.
A test asserts that no venue hostname appears in a `fetch` call in `indexer/src`.

## 5. Keeper hardening
Dry-run stays the default and the only mode outside chain 31337. Added: JSON-lines structured logs (`level`, `at`, `event`, fields;
never a key, there is none); `/healthz` (last pass, last error class, mode, chain id, listings read); a journal written by
append + `fsync` of whole lines, with a torn final line dropped on restart; a config loader that **refuses to start** on
contradictions (a source or quoter that needs an address without one, `--watch` under 15 s, `KEEPER_MODE=local` with a non-local
RPC URL, an unknown flag); exponential backoff on RPC errors with a ceiling and a give-up-this-pass rule. The "cannot send" tests
are extended to the new files. It is not deployed anywhere.

## 6. How it is checked
Unit tests: event folding for every event above; idempotent re-application; rollback + rebuild equals a fresh index of the same
chain; idle with no deployment; each formula on hand-computed cases (including the MIN_ENTRY waiting case and every `null` reason);
API: shapes against `types.ts`, 405/404/429, CORS and cache headers, no `Set-Cookie`; the no-venue-host test. An integration test
runs the follower against the recorded logs of a real local e2e run (a fixture of `eth_getLogs` responses), so CI needs no anvil.
The demo itself is run once here and its result recorded below.

## 7. Railway (for later; nothing is created in this phase)
One service: follower + API in one process, a volume mounted at `/data` for the SQLite file. Exact commands are in
`indexer/RAILWAY.md`. It is pointless to deploy before a public deployment file exists; the service would idle, correctly.

## How this could mislead
- `volumeUntilNextBuy` is an estimate resting on a measured take and a cap that expires hourly. It is a guide to order of
  magnitude, and the response carries its inputs so that anyone can recompute it.
- Harvest-based fee windows under-report trading between harvests.
- A `verify` link proves a transaction exists, not that the keeper bought well.
- Live fields are up to 15 s old; indexed fields are `CONFIRMATIONS` blocks old.

## Findings

### Two definitions changed while building, and why
- **`lifetimeFeesWei` is Σ (`toVault` + `toTreasury`), not Σ `claimed`.** `Harvested.claimed` is only what came out of Pons's escrow in
  that call; the router splits its whole fresh balance, which also holds anything sent to it directly. What was split is the
  honest "fees", and it is what the 80/20 is checked against. `claimedWei` is still reported, in `/v1/stats`. A treasury leg that
  failed is `treasuryDeferredWei` on the coin until a later harvest pays it.
- **Time to deliver runs from the winner's FIRST posted address to `Delivered`**, not from `Won`. That is the contract's own
  delivery clock (Phase 3, L3: a correction inside the hour changes the address, not the clock), and the stretch before an address
  exists is the winner's, not the keeper's. The fold keeps the first `DestinationSet` time and the latest address.

### Verified
- **Against a real local run.** `npm run demo` was run on 2026-09-21: anvil fork of Robinhood Chain (31337), the Phase 2 end-to-end
  (15 steps, a real drand round), then the indexer: 22 events folded, cursor at the head, `/v1/stats` showing one coin, a harvest of
  0.0729 ETH split 0.05832 / 0.01458 (80/20 exactly), one purchase settled, one inscription acquired, one draw delivered. The coin's
  creator tax (200 bps) came from Pons's own `getLaunchedToken` on the fork; `volumeUntilNextBuy` answered 1.79 ETH of volume for a
  0.0387 ETH shortfall at a 2.70% take — `0.0387 / (0.80 × 0.027)`, checked by hand.
- **The fold is a function of the events.** Rebuilding every derived table from the events table gives byte-for-byte what incremental
  application gave, over a story that emits every one of the contracts' 48 events at least once (`test/fold.test.ts`).
- **Reorg.** With the node disagreeing about the cursor block, the follower fell back to the newest block it still agreed with,
  dropped the events above it, rebuilt, re-indexed the new branch, and ended identical to a fresh index of that branch.
- **Idempotent, resumable, confirmations honoured, idle without a deployment** (no RPC call is made at all when idle), a failing node
  is an error in `/v1/health` and changes no table. A deployment file with only `deployedAtBlock` is refused: that is the L1 height.
- **The event ABIs are the contracts'.** Each declared event's selector equals the compiled contract's (CI compiles first so the
  test cannot skip).
- **The API**: shapes, 404/405/429, open CORS, `max-age=5` on data and `no-store` on health and errors, never a cookie, limits
  clamped. **A real bug found by its own test:** the server built a new handler — and so a new rate limiter — per request, so no
  client was ever limited. One handler now lives as long as the server, and the test drives the production `serve()` path.
- **It cannot send and calls no venue.** The RPC function refuses every non-read before any request; no signing or key identifier
  appears in `src/` or `scripts/`; the only thing ever passed to `fetch` is the configured RPC URL; a venue's name appears in one
  file, inside the verification hint handed to the reader.
- **Keeper hardening**, by test and by hand against the fork: `--mode live` and a 3-second watch are refused together with both
  reasons; a dead node is retried at 1, 2, 4, 8 s and the pass given up; the journal survives a torn final line.
- **Fixtures are reproducible**: `npm run fixtures -- synthetic` rewrites `indexer/fixtures/synthetic/` to the same bytes (CI checks).

### Unverified
- **`pool` pending fees on a graduated coin.** The code reads the hook's four buckets with the pool id built as Phase 1's fork test
  builds it, but no graduated Etch coin exists on any chain we can index, so that branch has run only in type-checking. The first
  graduated coin on a fork should be looked at by eye.
- **The Dockerfile was not built** (Docker is not running on this machine), and nothing was created on Railway. `indexer/RAILWAY.md`
  is a plan, not a record.
- **Scale.** Address lists are chunked by 200 per `eth_getLogs`; with thousands of coins a pass becomes many calls. Fine for a
  launch; it wants a topic-filtered read before it wants anything else.
- **The public RPC's limits** (range size, rate) under the follower's 2,000-block ranges: only ever run against anvil.
- **Bitcoin txid byte order** is a convention (hex as explorers print it, in the `bytes32`), enforced nowhere on chain. The keeper
  and attestor must follow it or every `verify` link breaks; it belongs in their runbooks when they exist.

### For the website
Copy `indexer/types.ts`. Read the API at a configurable base URL; when it cannot be reached, the labelled demo mode reads
`indexer/fixtures/demo/index.json` (path → file). `indexer/fixtures/synthetic/` shows every state the demo run does not reach
(a rune distribution with a roll-over, a voided and a cancelled draw, a named winner, a write-off with a slashed bond, an overdue
delivery); it is invented data and its `index.json` says so.

### CI
Green at `906017c` (run 35612503002): contracts, tools (snapshot, keeper), **indexer** (forge build, typecheck, 25 replayed tests, fixtures reproducible), web.
