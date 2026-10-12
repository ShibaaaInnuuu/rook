# Phase 1 services: the keeper sender (K3), the operator runner (K4) and the reserves reconciler (K7)

Written 2026-09-25 on branch `p1-keeper` (from `56da1a0`), **before the code**. Nothing here is deployed. No transaction is sent
to any public chain, no Railway setting is changed and no real key exists anywhere in the repository: every test runs against
fakes, a local anvil fork, or (K4) a local replica. §7 is filled in after the code with what changed and what was measured.

Scope rows: `docs/v6/phase1-scope.md` §3.1 K3, K4, K7. The deploy side (variables, Dockerfiles, Railway) is
`docs/deploy/services-v1.md`.

## 0. What each service may do, in one table

| service | holds | may sign or call | may never |
| --- | --- | --- | --- |
| **keeper** (`keeper/service/`) | the keeper's EVM key, `KEEPER_PRIVATE_KEY`, read in one file | `FloorModule.postFloor`, `SweepVault.postCap` on the RAFFLE and BURN vaults, `SweepVault.openDraw` on the RAFFLE vault. Nothing else: the call list is closed in code | post a floor the feed did not agree on; post a cap or a floor without that agreed floor; touch the YOYO vault; sign for a chain id it was not told; sign with a key that is not `registry.keeper()`; exceed its gas caps; log the key |
| **runner** (`ops/`, `runner.ts`) | the operator's hot IC identity, `OPERATOR_IDENTITY_PEM`, read in one file | the canister's `tick` on a schedule; `frog_offer` from the offer tool **only** with `--send` | export the identity; hand in an offer its own copy of the canister's checks refuses |
| **reconciler** (`ops/`, `reconcile.ts`) | no key; a Telegram bot token for alerts | reads only: 4663 (`eth_call`, `eth_getLogs`), ord and two esploras, the canister's `custody_caps()` query | send anything, anywhere, except an alert message |

Three services, not one, for three reasons:

1. **Secrets never share a process.** The keeper's EVM key and the operator's IC identity are different powers. One process
   holding both makes one bug or one leaked environment worth both.
2. **The alarm must outlive what it watches.** The reconciler reports red when the others fail, for example a stale floor. If it
   shared a process with the keeper, the crash it should report would also silence it. G9 asks for a watcher on a host that is not
   the keeper's. The reconciler is not that watcher (K6 is), but the same logic applies.
3. **Failure domains.** A 4663 RPC outage stops the keeper. An IC outage stops the runner. Neither should stop the other.

The runner and the reconciler share one package (`ops/`), because both speak Candid to the same canister. They build as two images
with two entrypoints. The runner's image is the only one given `OPERATOR_IDENTITY_PEM`, and the reconciler's code never reads it.
A test checks both.

## 1. K3: the keeper sender (`keeper/service/`)

### 1.1 Where it lives, and why not `src/` or `live/`

`keeper/src/` is the dry run, and it cannot send. `test/cannot-send.test.ts` enforces that, and this design keeps it true:
`src/` is imported, never edited to send. `keeper/live/` is tier 2: one command, one transaction, for a deployment labelled
`test`, armed by five facts. That is the wrong shape for a long-running service. The production sender is a new directory,
`keeper/service/`, and `test/send-guards.test.ts` gains its rules:

- `service/signer.ts` is the only file in the package that reads `KEEPER_PRIVATE_KEY`. It prints nothing. The key never leaves a
  closure: the file exports an address and a `sign(tx)` function.
- No other file in `service/` names a key, constructs an account or a wallet client, or calls `eth_sendTransaction`, which would
  make a node sign.
- `service/rpc.ts` is the only file that sends raw bytes (`eth_sendRawTransaction`). Its method list is closed.

### 1.2 Refusals at start, and before every send

The service **refuses to start**, listing every problem at once as `src/config.ts` does, when:

- `KEEPER_CHAIN_ID` is unset or is not a positive integer. There is no default. The operator writes `4663` on Railway, and the
  fork test writes the staging id.
- The stack file (`KEEPER_STACK`) says another `chainId`, or its label is not `stack`.
- No configured RPC reports `KEEPER_CHAIN_ID`. An RPC that reports another id is **dropped** and logged by host only (URLs can
  carry keys). The service runs on the ones that match. The chain id is asked of the endpoint again **before every send**, and a
  changed answer refuses that send.
- In send mode:
  - the key's address is not `registry.keeper()`, or not every module's `keeper()`. The module's keeper is immutable, and the
    sweep vaults read the registry's;
  - the key's address has code. EIP-7702 delegations to sweepers are on the well-known dev accounts
    (`docs/trust-model.md`), and a delegated keeper would lose its gas money;
  - the floor source is the fixture while `KEEPER_CHAIN_ID` is 4663.
- A vault in the stack names another registry, or the stack names a YOYO vault as a cap target. The YOYO vault is OFF in Phase 1:
  `phase1-scope.md` §1 says the keeper posts it no cap. The code has no path that posts one, and a test checks this.

The identity checks are repeated every pass. `proposeRoles` can rotate `registry.keeper()` after seven public days, and a keeper
rotated out must stop, not keep posting until its calls revert.

### 1.3 Modes

- **dry-run**, the default. Every planned call is **simulated** with `eth_call` from the keeper's address, at `latest`. The
  simulation reports whether the call would succeed and, if not, the revert decoded against the contract's errors. Nothing is
  signed. The key is not needed: the address is `registry.keeper()`.
- **send** runs only when both `KEEPER_MODE=send` is set and `--dry-run` is absent. A `--dry-run` flag always wins. Every send is
  simulated first, and a failed simulation sends nothing.

### 1.4 What it posts, and when

The service reads everything each pass (`KEEPER_INTERVAL_S`, default 60 s) from the chain at `latest`, and uses the **chain's
clock** (the latest block's timestamp) for every decision the contracts will judge. A restart therefore re-derives the same plan
from the same state.

**Floor, per collection (`FloorModule.postFloor`).** The target comes from the existing feed and nowhere else:

- the book comes from Ordinals Wallet's escrows;
- each listed outpoint's contents come through `VerifiedOrd(mainnetFeed())`, where two providers must agree at the same height
  (`docs/v6/ordinals-apis.md` §3). An unverified outpoint is `L-unverified`;
- `BookDiff` supplies listing events;
- `robustFloor` applies every exclusion rule;
- the result is converted to wei at the agreed rate (below).

The feed has no floor when there are fewer than 3 valid listings, when confidence is under the higher of
`POST_POLICY.minConfidenceBps` and the module's own `minConfidenceBps()`, or when the rate sources disagree. With no floor the
keeper posts nothing and says why. The module then goes stale, which is the safe failure. **"At least hourly" holds while the feed
agrees, and on purpose not otherwise.**

`planPost` (`src/floorpost.ts`) decides skip, keep or post, and steps inside `floorBounds()` (since the fixes: ±5 %/h and ±20 % of
the day's anchor; a module that does not answer it gets no plan). A module that is not fresh, or whose posted confidence is under
its minimum, is **re-posted** even at the same price (audit E-02a: a stolen key's confidence-0 post is overwritten next pass). The production policy
differs from the dry run's defaults in two numbers, both taken from the module's own document (`floor-module.md` §5, "posts at
least every 30 minutes, and on every move of more than 0.5%"):

- `refreshBeforeExpiryS` = 1,800;
- `repostMovedBps` = 50.

The TTL stays the chain's (`floor()` answers 3,600).

**The rate.** Coinbase and Kraken are both keyless. They must agree within `KEEPER_RATE_MAX_DISAGREE_BPS` (default 100). The
higher number of sats per ETH is used, which gives the lower floor in wei: mints dearer at the margin, and burns capped at β
anyway. (Until wave 2 the code took the lower sats/ETH, the opposite of this sentence: audit E-09, fixed.) If they disagree or one
is down, there is no rate, so no floor and no cap. This closes `floor-feed.md` §8.6 (a single rate reference) as far as the
keeper is concerned.

**Caps, RAFFLE and BURN (`SweepVault.postCap`).**

- `cap = floor × (1 + KEEPER_CAP_MARGIN_BPS)`, default 1,000 bps. That is the rehearsal's 0.66 on a 0.60 floor: the all-in price
  a sweep may pay per whole wX. The floor is **the module's accepted floor**, read after this pass's post and only while
  `fresh()`. A cap therefore never rests on a floor the chain has not accepted, and never on a stale one.
- The step rules of H12 are applied before planning, so the chain never refuses for a reason the keeper could have known:
  - while the posted cap is fresh, the new cap is clamped into ±20% of it;
  - no post within 10 minutes of the last, on the chain's clock;
  - never above `capCeiling()`; a vault with **no** ceiling takes no cap (`NoCeiling`), so nothing is planned and an alert says so;
  - at most 20% above the last cap on every post, fresh or stale (a stale re-post may fall freely).
- Re-post when under 15 minutes of the one-hour TTL are left, or when the new cap differs by more than 2%. These are
  `plan.ts`'s `repostIfUnderS` and `repostIfMovedBps`.
- With no fresh floor, no cap is posted. The caps then age out and `sweep` refuses. That is safe, and it is the same failure as
  the floor's.

**Draws (RAFFLE only), since the 2026-09-26 fixes.** The contracts changed the flow (`fixes-B-C-keeper-draws.md` C-03, C-04,
E-02) and the keeper follows it (`docs/audit/2026-09-26/fixes-W2-services.md` §1):

- **The slots.** `T_k = k × KEEPER_DRAW_EVERY_S + KEEPER_DRAW_OFFSET_S`, on the chain's clock (daily at 15:00 UTC by default).
  Within `KEEPER_DRAW_WINDOW_S` of a slot, each RAFFLE coin with `units ≥ 1e18`, no draw in flight and a holder of a whole coin
  now is **announced** (`announceDraw`). A due coin not announced in the window is an alert, never a quiet skip.
- **The snapshot time is drand's.** The announcement pins a future seed round. Once two relays serve its signature and the beacon's
  `verify` accepts it, `snapshotTimeOf(id, randomness)` gives `snapshotAt`, a moment of the day before the announcement that nobody
  knew when it was announced. The snapshot block is the last block stamped at or before `snapshotAt` (`docs/snapshot-v1.md "Version 2"`).
- **The snapshot** comes from `tools/snapshot` (the code the public verifier runs): weighted, the stack's exclusions
  (`tools/snapshot/src/stack.ts`, with `exclusions.json`, which must be readable), and the sentinel leaf
  `leaf(address(0), totalTickets, snapshotAt)`. Transfer logs are read incrementally (a per-coin cache on the volume), in ranges
  that are split when a provider refuses them as too large.
- **Publishing.** The file is written to the volume, uploaded to the mirror (the operator's Cloudflare R2 bucket through its S3
  API with a SigV4-signed `PUT`, `KEEPER_SNAPSHOT_S3_*`; or a plain `PUT` to `KEEPER_SNAPSHOT_UPLOAD_URL`; docs/deploy/services-v1.md
  §2.2), and read back from the URI the draw will carry (`KEEPER_SNAPSHOT_BASE_URL/<name>`) before `openDraw(id, root, total, uri, sentinelProof, seedSignature)`.
  On 4663 in send mode no draw is announced without a mirror. The keeper opens only under `OPEN_TTL − 15 min`; past it, the draw
  is voided by anyone after 6 h (the resolver does it).
- **An empty snapshot** (no eligible holder had a whole coin at `snapshotAt`; `docs/audit/2026-09-26/review2/fixes-R3-carry.md`):
  the file is the v2 tree of nobody, the sentinel alone, and the draw is opened with `totalTickets = 0`, root = the sentinel leaf and
  no proof. After the challenge window the resolver's `requestDraw` closes it with no winner (`Empty`, `DrawEmpty`): the unit goes
  back to the coin and a carried seed is consumed. A coin whose last draw was voided or cancelled before its pin (a carried seed) is
  announced even when the carried snapshot is empty, for that reason; a new seed is still announced only when the whole window has
  tickets.
- **The resolver** (§1.7) takes the draw from there: `requestDraw`, `resolve`, `claim`.

### 1.5 The transaction pipeline (`service/sender.ts`)

For each planned call, in this order, **one transaction in flight at a time**:

1. **Simulate** the call with `eth_call` from the keeper. On a revert, nothing is sent, and the error is logged and shown.
2. **Estimate gas.** `gasLimit = estimate × 1.25`. If that is above `KEEPER_MAX_GAS_PER_TX` (default 400,000), the call is
   refused.
3. **Fees.** `maxFeePerGas = 2 × baseFee + priority`, where priority is `eth_maxPriorityFeePerGas` (or 0) capped at the max fee.
   If that is above `KEEPER_MAX_FEE_PER_GAS_WEI` (default 1 gwei), the call is refused and the fee is never quietly lowered: a
   transaction priced under the market would sit.
4. **Money caps.** The worst case, `gasLimit × maxFeePerGas`, must be at or under `KEEPER_MAX_WEI_PER_TX` (default 0.0005 ETH).
   It must also fit in the day's budget `KEEPER_MAX_WEI_PER_DAY` (default 0.01 ETH, UTC days) after what is spent and what is
   reserved. The worst case is reserved at send, and replaced by `gasUsed × effectiveGasPrice` at the receipt.
5. **Nonce.** The nonce is read from the chain (`pending`) before each send. A local record that still waits for a receipt
   blocks the next send, which is why there is one transaction in flight at a time.
6. **Journal, then sign, then send.** The intent, nonce, hash and raw bytes are written and fsync'd **before** the send
   (`service/state.ts`, atomic rename). The raw transaction is public once sent, so writing it down reveals nothing. The service
   then broadcasts through the first matching RPC and falls back to the next on a network error. The same bytes can be sent
   again: they have the same hash.
7. **Receipt.** The service waits up to `KEEPER_RECEIPT_WAIT_S` (default 90). `status = 1` records success. A revert records the
   failure, and the call is not retried until a later pass plans it again from the chain.

**Across restarts.** Each journal entry still pending is resolved first:

- the receipt is there: record it;
- no receipt, and the chain's `latest` nonce is past the entry's: record it as dropped or replaced;
- neither: re-broadcast **the same bytes**; and once they have waited `KEEPER_STUCK_AFTER_S` (180 s) or a node refuses them as
  underpriced, sign a **replacement for the same nonce** at a bumped fee (+12.5% on both fee fields, at least `2 × base + tip`),
  the same call while it still simulates, else a zero-value self-transfer that clears the nonce; within the fee and money caps,
  never while the keeper is paused, and alerted. One that cannot be re-priced within the caps is an alert (`tx-stuck`) (E-10).

A transaction for a NEW nonce is never signed while one of ours is open. The registry's `keeperPaused()` is read again immediately
before every signature. Every plan is re-derived from the chain, so a
restart never posts twice.

### 1.6 Status and logs

`GET /healthz` returns 200 when the last pass ended under 3 intervals ago with no refusal in force and no alert in force for
`KEEPER_ALERT_PASSES` passes in a row (default 3; audit R2-S8), and 503 otherwise, with counts and the alert kinds only (never the
texts). Railway reads it only when it deploys the service (audit E-15 item 1); the continuous check is the
monitor's liveness poll, which pages after 10 minutes of anything but 200 (`docs/audit/2026-09-26/fixes-W2-monitor.md` §1.5). The
standalone resolver's `/healthz` is polled the same way.

`GET /status` is a **public summary** (audit E-14): the mode, chain id, keeper address, passes, the last pass, whether it is healthy,
and how many refusals and alerts are in force. `GET /status/full` returns the whole state (per module, per vault, the draws, the
resolver's actions, gas against the budget, the pending transaction, the last error, the refusal and alert texts), only with
`Authorization: Bearer <KEEPER_STATUS_TOKEN>`, and with no wildcard CORS. Without a token it is off.

`GET /snapshots/<name>` serves the published files.

Logs are one JSON object per line, through `src/log.ts`'s `logger`. The service passes its own secrets function, which adds the
private key's value (with and without `0x`), the status token and the upload credential to the scrubber. The scrubber is belt
and braces: no code path hands the key to a logger. A test runs a full pass with a known key and asserts that its hex appears in
no log line, no status body and no error.

### 1.7 The keyless resolver

`keeper/service/resolver.ts`, run after each keeper pass and standalone (`npm run resolver`, its own key, `RESOLVER_MODE=send`):
only calls anyone may make. The Wrapper's pinned tickets are resolved strictly in order as soon as two drand relays serve their
round; RAFFLE draws are requested after the challenge window (not while the keeper is paused, which the vault refuses), resolved
when their pinned round exists, and claimed for the winner with the proof from the published file (used only if its root is the
draw's) or from a tree rebuilt from the chain at `snapshotAt`; a draw announced and never opened (6 h), pinned and never resolved
(7 days), or resolved and unclaimable (7 days) is voided. The standalone refuses the registry's keeper as its key and keeps
running while the keeper is paused. Details: `docs/audit/2026-09-26/fixes-W2-services.md` §2.

## 2. K4: the operator runner and the offer tool (`ops/`)

### 2.1 The runner

`tick` is operator-only and has no timer (`canister.rs:1129`). The runner:

- loads the identity from `OPERATOR_IDENTITY_PEM` in `ops/src/identity.ts` and nowhere else. A secp256k1 `EC PRIVATE KEY` is
  what `dfx identity export` prints; an Ed25519 PKCS#8 key also loads;
- logs only the principal;
- calls `tick()` every `RUNNER_TICK_EVERY_S` (default 600; 60 minimum). Calls never overlap: the next waits for the previous
  one, which the canister's own tick guard also enforces.

The cadence is a cycles question (`canister-mainnet.md`: an idle loop that reads once a minute costs about 2 T a day on 13
nodes). Ten minutes is well inside every deadline the flows have: an hour of cap margin, and six Bitcoin confirmations.

Each answer is logged line by line: `Ok(lines)` at info, `Err` at warn. A rejected call, for example a trap on the operator
check, is an error, and three in a row turn `/healthz` red. The runner never retries a tick faster than its schedule. On start
the runner asks `paused()` and reports it; a paused canister is still ticked, and it answers "paused".

### 2.2 The offer tool (`npm run offer --`, dry run by default)

The input is **a venue's listing**: the seller-signed PSBT, plus which inputs the venue asks the buyer to sign. Two shapes are
accepted:

- `--psbt <hex|base64|file>` with `--to-sign 1,2`, which is any venue's PSBT;
- `--unisat-bid <file.json>`, UniSat's `create_bid` answer (`psbtBid` / `toSignInputs`). UniSat is the one marketplace the repo
  already speaks to (`keeper/src/ordapi.ts`). The shape comes from its swagger and is **UNVERIFIED**, like every UniSat shape
  here. Anything unexpected is refused.

The tool does **not** call UniSat's bid endpoints: buying there needs UniSat's written permission (`docs/phase1.md` F3), which
the operator has asked for and not received. It takes the file the operator obtained.

It then runs **every check `frogflow::accept` and `frogbuy::judge` run**, in the same order, against the same kinds of sources:

1. The inscription id is well formed, and the frog id is `Wanted` in `frog_buys()` (a query). The cap in wei comes from there.
2. **Membership.** `leafOf = keccak256(keccak256(id))`, and the proof is verified against the listing's `membershipRoot`, read
   on 4663 from the registry. The proof is taken from `--proof`, or computed from `--members` (the published id list).
3. **Location.** Two ordinals providers agree on the inscription's outpoint and offset, and on that output's contents: exactly
   this inscription and no runes. The esploras agree the output exists, is confirmed, is unspent, and holds ord's value.
4. **The PSBT.** The TypeScript port of `judge` checks:
   - every input's value is known;
   - `to_sign` is only float inputs;
   - no input spends custody;
   - exactly one input is the seller's, at ord's value;
   - there is no `OP_RETURN`;
   - the fee is not negative;
   - spend ≤ cap (in sats, at the rate) and fee ≤ `fee_bound(spend)`;
   - exactly one custody output, above dust;
   - the inscribed sat lands in the custody output, and every sat of that output is the seller's or ours;
   - every foreign witness is a key-path signature or P2WPKH.
5. **Limits.** `cost_wei = wei_for_sats(spend, rate)` is within the policy's 0.05–0.5 ETH, or `--limit-wei`, and within the cap.
6. **The print.** It prints exactly what would be signed:
   - each float input to be signed, with its outpoint, value and script;
   - every output, with its value, script and owner (custody, float, seller, other);
   - the spend, fee and cost;
   - the BIP-341 key-path sighash of each input the canister will sign;
   - the Candid arguments of `frog_offer`.

The rate is the canister's to choose at accept time (the oracle median). The tool uses the agreed Coinbase/Kraken rate, or
`--sats-per-eth`, and says so. When the cost is within 1% of the cap it warns that the canister's own rate may refuse the offer.

Only `--send`, with `OPERATOR_IDENTITY_PEM` set, calls `frog_offer`, and only when every local check passed. The tool cannot
override a failed check. The canister judges again anyway: the tool exists to fail early and to show the operator what they are
handing in.

## 3. K7: the reserves reconciliation (`ops/`, `reconcile.ts`)

- **Hourly run.** Every `RECONCILE_EVERY_S` (default 3,600) the reconciler runs the indexer's `WrappedService` (imported from
  `indexer/src/wrapped/`, unchanged). The configuration is the stack file, which is a valid `WRAPPED_CONFIG` by design
  (`deploy-stack.md` §3). It reads 4663 through a read-only RPC with the primary and fallback URLs and a pinned chain id, and
  Bitcoin through ord and the two esploras that `verify.ts` already uses.
- **Custody caps.** It calls the canister's `custody_caps()` anonymously, since it is a query, and gets the `CapPolicy` and the
  headroom. From the caps it derives the **stage** using `launch-gates.md` G7's table:
  - stage `n` when every collection cap and both global caps equal stage `n`'s numbers;
  - "custom, at most stage n" when they fit under stage `n` without equalling it;
  - "above stage 3" otherwise.

  It compares that with the stack file's `stage`. If the canister's caps are **above** the stack's stage, the result is red: G7
  says they must match. If they are below, it is a note. If the caps cannot be read, the result is unverified.
- **Status.** The status is the worst of the collections' statuses and the caps. **Stale is red.** A report older than
  `RECONCILE_STALE_S` (default 10,800, which is 3 hours) is served as red with the reason "stale: last computed …". The age is
  judged when the report is served, so a hung job turns red without running. A watchdog timer (every 5 minutes) alerts on it. The
  last report is persisted to `RECONCILE_STATE_DIR`, so a restart keeps the real age instead of forgetting it.
- **Alerts.** The `Alerter` interface is pluggable. `TelegramAlerter` uses `ALERT_TELEGRAM_BOT_TOKEN` and
  `ALERT_TELEGRAM_CHAT_ID`, read at the moment of the call. The token is only ever in the URL of one POST to `api.telegram.org`,
  is scrubbed from any error text, and is never logged. `LogAlerter` always runs. The service alerts:
  - on the transition to red;
  - on a change in the red reasons;
  - every `RECONCILE_REALERT_S` (default 6 h) while still red;
  - once on recovery.
- **Served.**
  - `GET /v1/reserves` returns the reports with `caps`, `stage`, `stale` and `ageS` added;
  - `GET /` is a small server-rendered page with a red, amber or green banner, the age, each collection's reasons, and the caps
    and stage;
  - `GET /healthz` is read by Railway at deploy and by the monitor's liveness poll. It stays 200 while the process runs, because a
    red reserve is news, not a crash.

  The site's own reserves page (`web-rook/`, K5's files) is not edited here. It can point at this API, and §6 lists that as a
  follow-up for K5.

## 4. Tests (the spec)

**Keeper (`keeper/test/service-*.test.ts`)**

- Configuration refusals. Each of these is refused, and all problems are listed at once:
  - no `KEEPER_CHAIN_ID`;
  - a non-numeric `KEEPER_CHAIN_ID`;
  - a stack for another chain;
  - no matching RPC;
  - a key that is not the registry's keeper, or not the module's;
  - a key with code;
  - the fixture source on 4663;
  - a YOYO target.
- RPCs. An RPC that changes its chain id between start and send has that send refused.
- Simulation. A failing simulation means no send. A dry run never signs, and never calls `eth_sendRawTransaction`.
- Caps. The per-transaction gas, fee and money caps each refuse. The per-day budget refuses, and it resets at the UTC day.
- Nonces. There is one transaction in flight at a time. On restart, a mined, dropped or pending journal entry is each
  resolved correctly, and the same bytes are re-broadcast.
- Receipts. A reverted receipt is recorded, not retried.
- Plans.
  - The floor is posted only from an agreed feed. It is refused on rate disagreement, on one ord provider, and on low
    confidence.
  - The refresh happens at 30 minutes.
  - Caps are clamped to ±20%, respect the 10-minute gap, respect the ceiling, and are never posted without a fresh module
    floor.
  - YOYO is never touched.
  - The draw slot rule, the snapshot block chosen by timestamp, the snapshot root equal to `tools/snapshot`'s, the published
    URI, one draw per coin per slot, and a missed window skipped.
- Secrets. The key appears in no log line, status body or error. The source scan finds that only `service/signer.ts` reads
  it.

**Ops (`ops/test/*.test.ts`)**

- Runner. The identity loads (secp256k1 and Ed25519 PEMs) and only its principal is printed. Ticks never overlap. `Err` and
  rejects are counted and turn health red. The source scan finds the PEM variable read in one file only.
- Offer tool.
  - The TypeScript judge on constructed PSBTs covers every refusal of `frogbuy::judge`:
    - an unknown input value;
    - a foreign input to sign;
    - spending custody;
    - the seller input missing or duplicated;
    - the seller value mismatched;
    - `OP_RETURN`;
    - a negative fee;
    - over the cap;
    - the fee over its bound;
    - zero or two custody outputs;
    - custody dust;
    - the sat not landing in custody;
    - foreign sats in custody;
    - a script-path witness.
  - The rest:
    - `leafOf` and the proof verification match the canister's (and `staging/members.ts`'s pinned root);
    - `wei_for_sats` and `sats_for_wei` match the Rust arithmetic, including the one-step-down;
    - a dry run prints and does not call;
    - `--send` without the identity is refused;
    - a failed check cannot be sent.
- Reconciler.
  - Stages are derived from the G7 table, and a caps/stack mismatch is red.
  - A report older than 3 h is served red, and it persists across a restart.
  - Alert deduplication, the re-alert, and recovery.
  - The Telegram token appears in no log and no error, including when a failing fetch's error contains the URL.

**Fork (`scripts/keeper-fork-test.sh`)**, local only:

1. Fork. Anvil forks `rpc.ordofi.network` at head − 2,000 under chain id 9466301, using anvil's `--chain-id` flag, as
   `golive-dryrun.sh stack` does. Nothing signed on the fork is valid on 4663 (EIP-155).
2. Deploy. `deploy.sh`, `list-auto.sh` and `deploy-stack.sh` run with anvil dev account 3 as the keeper. Its EIP-7702 code
   on the fork is cleared with `anvil_setCode`, as the no-code refusal requires.
3. Dry run. The keeper runs once in dry-run: every call is simulated, and nothing moves.
4. Send, then restart.
   - The keeper runs once in send mode:
     - `postFloor` is mined, and the module is `fresh()`;
     - `postCap` is mined on RAFFLE and BURN, and both are `capFresh()`;
     - YOYO gets no cap.
   - A second pass keeps: nothing is re-posted.
   - A restart keeps: nothing is re-posted.
5. The draw.
   - A coin is launched and graduated through the module, and a sweep buys at least one whole wX, the same steps as the
     rehearsal.
   - The keeper opens a draw at the next slot with a snapshot it published.
   - That snapshot's root is recomputed with `tools/snapshot`'s CLI and must match.
6. The refusals, each with the nonce unchanged:
   - the wrong `KEEPER_CHAIN_ID`;
   - a key that is not the keeper;
   - an RPC reporting another id;
   - the fixture source on a node that reports 4663 (`anvil_setChainId`). The keeper in dry-run against that node simulates
     only.
7. The real feed. The Ordinals Wallet book with keyless providers only is run in dry-run: with no UniSat key the contents are
   unverified, so there is no floor and the keeper posts nothing.

**Replica (`ops/scripts/runner-replica-test.sh`)**, local only:

- A local dfx replica installs the built canister with `operator` set to a freshly generated test identity.
- All `rpc_urls` and `btc_api_urls` point at unreachable local `https://127.0.0.1:1`, so no outcall can reach any chain.
- The runner ticks with that identity and gets an answer. A different identity is refused by the operator check.

## 5. What this does not do

- It does not deploy anything or change Railway. The operator does both, from `docs/deploy/services-v1.md`.
- It does not make the floor feed richer. Magic Eden's book, a third ordinals provider and UniSat's listings stay open
  (`ordinals-apis.md` §6). Without `UNISAT_API_KEY`, no frog floor is ever posted: two providers must agree on an output's
  contents, and keyless there is one.
- It does not move a cap's source off the keeper's word (`trust-model.md` residual 2). It makes the word follow a published
  rule from an agreed floor.
- It does not make the snapshot provable (`snapshot-v1.md`, "What this does not prove"). It makes the block rule public in
  advance.

## 6. Open problems (filled in as found)

1. The site's reserves page (`web-rook/`) does not show the reconciler's `caps`, `stage` or staleness. It needs a K5 follow-up,
   so the two branches do not edit the same files. Until then the reconciler's own page, `/`, is the G5 page.
2. **No frog floor without `UNISAT_API_KEY`.** This is measured, not assumed. On the fork run the real keyless feed excluded 40
   of 40 checked listings as `L-unverified`, so the keeper posted nothing. It is the rule working (`ordinals-apis.md` §6.2), and
   it is also a go-live dependency: O2.
3. **UniSat's `create_bid` shape is unverified** (swagger only, like every UniSat shape here). The offer tool refuses anything
   unexpected. It does not call the bid endpoints, because of F3.
4. **The rate references (Coinbase, Kraken) are keyless, and neither publishes terms.** Two of them are better than one. The
   rate still moves the posted floor, and the posted floor moves only at the module's pace (`floor-module.md` §5).
5. ~~**Unverified is not alerted.**~~ **Fixed 2026-09-26 (audit E-08a, E-15 item 7):** it was permanent silence, not a provider's
   bad hour. A reconciler that reads nothing now pages at once, and unverified for 6 hours (`RECONCILE_UNVERIFIED_ALERT_S`)
   pages, both repeating every 6 hours (`docs/audit/2026-09-26/fixes-W2-monitor.md` §2).
6. **The keeper and the runner share the operator's Railway project** (separate services, separate variables), as
   `k6-watcher-canary.md` §7 assumes. A compromise of that project is a compromise of both. The monitor and the reconciler
   are in another project.
7. **On the fork the first buy once reverted silently** (the numbers show it: the curve took one buy, not two). `cast send`
   without `--json` hid the status. The script now prints each buy's status, buys again while the curve is still trading, and
   fails if the raise is not reached. The cause of that single revert was not found. The keeper was not involved: its posts
   were mined before it.

## 7. Results (2026-09-25, after the code)

**Built.**

- `keeper/service/` (13 files): the production sender. `keeper/src/` is unchanged, and `test/cannot-send.test.ts` still passes
  over it.
- `ops/` (a new package, `@icp-sdk/core` 6.1.0 as `monitor/` uses): the runner, the offer tool with its TypeScript `judge`,
  and the reconciler.
- Three Dockerfiles with `.dockerignore` whitelists, and three `railway.json` files.
- `scripts/keeper-fork-test.sh` and `ops/scripts/runner-replica-test.sh`.
- `docs/deploy/services-v1.md`.

**What the code changed in this note.**

1. **`block.number` on 4663 is the parent chain's** (`src/lib/Wait.sol`). The keeper therefore judges both clocks as the
   contracts do:
   - the "EVM block" of any block is its `l1BlockNumber`, falling back to its own number on a node that has none, such as
     anvil;
   - `openDraw`'s `snapshotBlock` is that parent-chain number of the snapshot's block;
   - the snapshot itself is built at the L2 block, whose number is what `tools/snapshot` takes and what the file names;
   - the cap's ten-minute gap is checked in parent blocks as well as in seconds.
2. **"A YOYO target" is checked on chain.** The stack's RAFFLE and BURN vaults must report those policies, and each vault's
   `registry()` must be the stack's. `keeper.ts` never names the YOYO vault, and a test reads the file to check that.
3. **The chain id is asked every pass as well as before every send.** A send refused before the bytes leave clears the
   in-flight record. Nothing left, so nothing is pending.
4. **A dry run's caps are simulated against the floor it would have posted.** Without that, a fresh deployment's dry run
   could never show a cap.
5. **The alerter is the monitor's**, as the coordinator suggested. The reconciler's `Channel` is `monitor/src/alerts.mjs`'s
   `Telegram` and its `format`, used unchanged. Its own dedupe (§3) is a small pure function, because `due()` knows only
   the monitor's alert kinds.
6. **Starting fails with exit 2, and a refused pass exits 1** under `--once`.
7. **The fee cap on a fork.** Anvil suggests a 1 gwei tip where Nitro suggests 0, so the fork run sets
   `KEEPER_MAX_FEE_PER_GAS_WEI` to 5 gwei. The default stays 1 gwei, which is 50× 4663's base fee.

**Tests.**

- `keeper`: **96 pass** (67 before, 29 new in five `service-*.test.ts` files), and `tsc --noEmit` is clean.
- `ops`: **20 pass**, and `tsc` is clean.
- Unchanged and still green after the merge of main:
  - indexer 99;
  - monitor 48;
  - `tools/snapshot` 12.

**The fork run** (`scripts/keeper-fork-test.sh`). It forks `rpc.ordofi.network` at head − 2,000 under chain id 9466301, and
the result is **KEEPER FORK TEST: ALL PASS**. Measured on the fourth and fifth runs:

- **Dry run.** Every call was simulated from `registry.keeper()`. The keeper's nonce stayed at 657, and the module was not
  fresh.
- **Send.** `postFloor` (0.6 ETH, a fixture floor on the fork) and `postCap` on RAFFLE and BURN (0.66 ETH each) were mined.
  The nonce went from 657 to 660, and the YOYO vault's cap stayed 0.
- **Keep and restart.** The second pass sent nothing, and a restart sent nothing.
- **Launch.** A coin was launched. Its graduation was filled by the module at the keeper's floor, giving 10.909 ETH-backed
  units. A sweep bought 1.30 wX under the keeper's cap.
- **Draw.** `openDraw` was sent at the slot. The snapshot had 800,000,000 tickets (one per whole coin, the curve excluded). It
  was published under `state/snapshots/` and its root matched the one on chain. `tools/snapshot`'s CLI, run by hand from the
  file's inputs, gave **the same root and the same bytes**. The same slot was not reopened.
- **Refusals**, each with the keeper's nonce unchanged:
  - `KEEPER_CHAIN_ID=4663` with a fixture, in send mode;
  - `KEEPER_CHAIN_ID=4663` against the 9466301 stack;
  - a key that is not the keeper's;
  - the keeper carrying its real EIP-7702 code;
  - an RPC reporting 31337, which was dropped while the keeper ran on the good one;
  - only a wrong-chain RPC.
- **The node switched to report 4663.** A dry run with the real keyless feed gave "no floor: 40 of them because two ordinals
  providers did not agree". Nothing was signed.
- **K7 against the fork.**
  - Without a canister, the collection was green (its supply was the module's 12.21 ETH-backed units, with no items), and the whole
    was unverified.
  - With this worktree's canister on a local replica (every outcall pointed nowhere), `custody_caps()` said stage 0, equal to
    the stack's stage, and the reconciliation was **GREEN**.

**The replica run** (`ops/scripts/runner-replica-test.sh`) gave **RUNNER REPLICA TEST: ALL PASS**:

- this worktree's canister was installed with the runner's fresh identity as its operator;
- the runner logged its principal;
- it read `paused() = false`;
- `tick()` answered the operator. It was an `Err`, because every outcall pointed at an unreachable local address;
- a stranger's tick was rejected with "neither the operator nor a controller";
- with no identity, the runner refused to start;
- the PEM appeared in no line.

**The images.** All three were built locally (`docker build -f … .` from the repository root), and each was run once with no
configuration. Each refused to start, with exit 2, naming its missing variables. So the images resolve their imports: the
keeper's `tools/snapshot`, and the reconciler's `indexer/` and `monitor/` files. The reconciler's image holds only
`reconcile.ts`, `stage.ts`, `ic.ts` and `log.ts` from `ops/src/`. The images were removed afterwards and pushed nowhere.

Nothing was deployed. No transaction was sent to any public chain, no Railway setting was touched, and no real key exists. The
only keys used were anvil's public dev keys and freshly generated test identities, on local nodes.
