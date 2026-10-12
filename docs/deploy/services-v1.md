# Phase 1 services: keeper, runner, reconciler — how to run them (v1)

Written 2026-09-25 on branch `p1-keeper`. **Nothing here has been deployed and no Railway setting has been changed.** This is the
operator's page: what each service is, every variable it reads, what it refuses, and the order to start them in. The design and
the reasons are in `docs/v6/p1-services.md`; the tests that are the spec are listed at the end.

## 1. Three services

| service | code | image | holds | Railway project | volume |
| --- | --- | --- | --- | --- | --- |
| **keeper** (K3) | `keeper/service/` | `keeper/Dockerfile`, config `keeper/railway.json` | the keeper's EVM key | the operator's | `/data`: the transaction in flight, the day's gas, the coin index and log cache, the published snapshots (also mirrored: the R2 bucket, `KEEPER_SNAPSHOT_S3_*`, or `KEEPER_SNAPSHOT_UPLOAD_URL`) |
| **runner** (K4) | `ops/src/runner.ts` | `ops/Dockerfile.runner`, config `ops/railway.runner.json` | the operator's hot IC identity | the operator's | none |
| **reconciler** (K7) | `ops/src/reconcile.ts` | `ops/Dockerfile.reconciler`, config `ops/railway.reconciler.json` | a Telegram bot token only | **the monitor's** (K6) | `/data`: the last report and the alert state |

**Why three, not one.**

- The keeper's EVM key and the operator's IC identity are different powers. One process holding both makes a single bug, or a
  single leaked environment, worth both.
- The reconciler reports what the other two break. Suppose it shared a process or a host with them. Then the crash, or the
  compromise, it should report would also silence it. The monitor (K6) is placed in a different Railway project for the same
  reason (`docs/v6/k6-watcher-canary.md` §7), so the reconciler goes with the monitor.
- Their failures are independent. A 4663 RPC outage stops the keeper, and an IC outage stops the runner; neither should stop the
  other.

Each image carries only what it runs. The runner's image holds four files, and has neither the offer tool nor the reconciler. The
reconciler's image has neither the runner nor the identity loader. The Dockerfiles' `.dockerignore` files keep the build context
to those paths.

In Railway, each service is added from this repository with root directory `/`, and its config file path is set to the
`railway.json` in the table. Each Dockerfile builds with the repository root as its context. Secrets are **service** variables,
set in Railway's UI. They never go in a project-shared variable, a command line or a file.

## 2. The keeper (K3)

It posts four things:

- `FloorModule.postFloor` from the agreed floor feed;
- `SweepVault.postCap` on the RAFFLE and BURN vaults, never the YOYO vault;
- `SweepVault.announceDraw` at the documented slots, and `openDraw` once the announcement's drand seed round exists, with a
  snapshot at the draw's `snapshotAt` (sentinel leaf included) published and read back first.

After its own work, each pass runs the **keyless resolver** (`requestDraw`, `resolve`, `claim`, `voidDraw` on the RAFFLE vault;
`Wrapper.resolve` for every pinned redemption, in order). Those calls need no role. The same resolver also runs as a **separate
service** with its own funded key, so no redemption or raffle depends on the keeper's key (§2.1).

It is a **dry run unless `KEEPER_MODE=send`**. The rules are `docs/v6/p1-services.md` §1 and
`docs/audit/2026-09-26/fixes-W2-services.md`.

| variable | required | secret | default | meaning |
| --- | --- | --- | --- | --- |
| `KEEPER_CHAIN_ID` | yes | | — (no default) | the chain it signs for: `4663`. Every RPC must report it; a stack file for another chain is refused |
| `KEEPER_STACK` | yes | | — | the stack file, `/app/deployments/stack-4663.json` in the image |
| `KEEPER_MODE` | | | `dry-run` | `send` to sign and send. `--dry-run` on the command line always wins |
| `KEEPER_PRIVATE_KEY` | send mode | **yes** | — | the keeper's key: 32-byte hex. Read in `keeper/service/signer.ts` only, and only in send mode. Its address must be `registry.keeper()` and every module's `keeper()`, and must have no code |
| `KEEPER_RPC_URLS` | | | `https://rpc.mainnet.chain.robinhood.com,https://rpc.ordofi.network` | tried in order. Must be https. `robinhood.drpc.org` is dead for this (phase1-scope §3.2) |
| `UNISAT_API_KEY` | for a floor | **yes** | — | read by `keeper/src/ordapi.ts` at call time. Without it only one ordinals provider can read an output's contents, so **no floor is posted**: the module goes stale, and sweeps stop |
| `KEEPER_OW_SLUGS` | | | the stack's slug | `<stack slug>=<Ordinals Wallet slug>`, for example `frogs=bitcoin-frogs` |
| `KEEPER_PUBLIC_URL` | for draws | | — | this service's public URL (https; plain http only for 127.0.0.1). The default snapshot base is `<it>/snapshots` |
| `KEEPER_SNAPSHOT_S3_ENDPOINT` | draws on 4663 (this or `KEEPER_SNAPSHOT_UPLOAD_URL`) | | — | the S3 API origin of the snapshot bucket, e.g. `https://<account id>.r2.cloudflarestorage.com` for Cloudflare R2 (https, origin only). The snapshot is `PUT` there path-style (`<endpoint>/<bucket>/<prefix><file>`), signed with AWS Signature V4 (region `auto`), `Content-Type: application/json`, no ACL. In send mode on 4663 **no draw is announced without a mirror** (this, or the plain PUT below; not both) |
| `KEEPER_SNAPSHOT_S3_BUCKET` | with the endpoint | | — | the bucket's name |
| `KEEPER_SNAPSHOT_S3_PREFIX` | | | `rook-snapshots/` | the key prefix (a trailing `/` is added; `""` for the bucket's root) |
| `KEEPER_SNAPSHOT_S3_ACCESS_KEY_ID` | with the endpoint | | — | the R2 API token's access key id |
| `KEEPER_SNAPSHOT_S3_SECRET_ACCESS_KEY` | with the endpoint | **yes** | — | the R2 API token's secret access key: used only as the signing key, never sent, never in an error; scrubbed from every log line |
| `KEEPER_SNAPSHOT_S3_REGION` | | | `auto` | the SigV4 region: `auto` for R2; another S3 service's region otherwise |
| `KEEPER_SNAPSHOT_UPLOAD_URL` | draws on 4663 (this or `KEEPER_SNAPSHOT_S3_ENDPOINT`) | | — | a plain-PUT mirror (not R2/S3): https base the snapshot is `PUT` to (`<it>/<file>`), off the keeper's volume |
| `KEEPER_SNAPSHOT_UPLOAD_AUTH` | | **yes** | — | the `Authorization` header value for the plain PUT (e.g. `Bearer …`); scrubbed from every log line |
| `KEEPER_SNAPSHOT_BASE_URL` | with a mirror (**required on 4663**) | | `<KEEPER_PUBLIC_URL>/snapshots` | the public https base a draw's URI names (`<it>/<file>`): on 4663 with a mirror it must be the mirror's public address, on another host than `KEEPER_PUBLIC_URL` (audit R2-S5: the URI on chain never names the hot key's host; the keeper refuses to start otherwise), and never the S3 API's host (it serves signed requests only). The keeper reads the file back from it, and the mirror's own object (the bucket's, with a signed GET; or `<upload url>/<name>` with the upload credential), byte for byte, before `openDraw` |
| `KEEPER_SNAPSHOT_UPLOAD_READBACK` | | | `on` | `off` only for a mirror whose object cannot be read back at its API (the public URI is still read back) |
| `KEEPER_ALERT_PASSES` | | | `3` | `/healthz` answers 503 once alerts have been in force this many passes in a row (R2-S8) |
| `KEEPER_ALERT_LATCH_S` | | | `1200` | an alert stays in force this long after the last pass that raised it, so a one-pass alert (`draw-slot-missed`, `draw-not-opened`) or one that flaps still reaches `KEEPER_ALERT_PASSES` and keeps `/healthz` at 503 past the monitor's 10 minutes (R3-check L-G); a value under `(KEEPER_ALERT_PASSES + 1) × interval + 720` s is raised to that (R4-check L-S2), for the keeper and the resolver (`RESOLVER_INTERVAL_S`) |
| `KEEPER_DRAND_URLS` | | | `https://api.drand.sh,https://drand.cloudflare.com,https://api2.drand.sh` | drand relays (evmnet); two must serve the same signature, which the beacon's `verify` then checks. A relay that disagrees is a refusal |
| `KEEPER_STATUS_TOKEN` | | **yes** | — | ≥ 32 characters: `/status/full` answers only `Authorization: Bearer <it>`. Unset: the full view is off; `/status` is a public summary |
| `KEEPER_RESOLVER` | | | `on` | the in-process resolver. `off` only when the standalone resolver runs |
| `KEEPER_RESOLVER_MAX_TX` | | | `10` | resolver transactions per pass |
| `KEEPER_RESOLVER_MAX_GAS_PER_TX` | | | `900000` | the resolver's own gas cap (a drand proof is ~180k gas) |
| `KEEPER_STUCK_AFTER_S` | | | `180` | a transaction without a receipt this long (or refused as underpriced) is **re-priced at the same nonce**: the same call while it simulates, else a zero-value self-transfer. Within every cap, and alerted |
| `KEEPER_PASS_BUDGET_S` | | | `240` | draws and the resolver stop when a pass has run this long; the floor and caps always run first |
| `KEEPER_COINS_PER_PASS` | | | `200` | launched coins indexed per pass (the index is kept on the volume) |
| `KEEPER_STATE_DIR` | | | `/data` in the image | the volume |
| `PORT` / `KEEPER_STATUS_PORT` | | | `8080` in the image | serves `/healthz` (counts only), `/status` (a public summary), `/status/full` (bearer token) and `/snapshots/<file>` |
| `KEEPER_INTERVAL_S` | | | `60` | seconds between passes (15–1,800) |
| `KEEPER_MAX_GAS_PER_TX` | | | `400000` | gas cap per transaction (the estimate plus 25%) |
| `KEEPER_MAX_FEE_PER_GAS_WEI` | | | `1000000000` (1 gwei) | a higher fee is **refused, never lowered**. 4663's base fee is about 0.01 gwei |
| `KEEPER_MAX_WEI_PER_TX` | | | `500000000000000` (0.0005 ETH) | the worst case (gas × max fee) per transaction |
| `KEEPER_MAX_WEI_PER_DAY` | | | `10000000000000000` (0.01 ETH) | per UTC day. The worst case is reserved at send, then replaced by the actual cost at the receipt |
| `KEEPER_RECEIPT_WAIT_S` | | | `90` | how long it waits for a receipt before leaving the transaction to the next pass |
| `KEEPER_CAP_MARGIN_BPS` | | | `1000` | cap = floor × (1 + margin): the all-in price a sweep may pay per whole wX |
| `KEEPER_RATE_MAX_DISAGREE_BPS` | | | `100` | how far Coinbase and Kraken may differ on sats per ETH. The higher sats/ETH (the lower wei floor) is used. Past this there is no rate, and so no floor |
| `KEEPER_DRAW_EVERY_S`, `KEEPER_DRAW_OFFSET_S` | | | `86400`, `54000` | draw slots `T = k × every + offset`: daily at 15:00 UTC |
| `KEEPER_DRAW_WINDOW_S` | | | `3000` | a slot's draws are announced within 50 minutes of it; a due coin not announced by then is an **alert** (`draw-slot-missed`). The snapshot time is the seed round's, not the slot's |
| `KEEPER_FLOOR_SOURCE`, `KEEPER_FLOOR_FIXTURE` | | | `ordinalswallet` | `fixture` is for tests and local forks. It is refused for a send on 4663 |

**It refuses to start** (exit 2, every problem listed) when:

- the chain id is missing, or not a number;
- the stack file is for another chain, or is not labelled `stack`;
- no RPC reports the chain id. An RPC that reports another id is dropped and named by host only;
- it is in send mode with no key, or with a malformed key. The value is never repeated;
- it is asked to send a fixture floor to 4663;
- a snapshot mirror is half-configured: a `KEEPER_SNAPSHOT_S3_*` variable without the endpoint, the endpoint without the bucket
  or either key, both an S3 endpoint and `KEEPER_SNAPSHOT_UPLOAD_URL`, or (on 4663) a mirror without `KEEPER_SNAPSHOT_BASE_URL`,
  a base on the keeper's own host, or a base on the S3 API's host.

**It refuses to send** (every pass: `/status/full` lists the refusals and `/healthz` is 503) when:

- the registry has paused the keeper (`keeperPaused()`); the pause is read again immediately before every signature;
- the key is not `registry.keeper()`, or not a module's keeper;
- the key has code (EIP-7702);
- a vault names another registry;
- the stack's RAFFLE or BURN vault is not of that policy;
- an endpoint changes its chain id before a send.

**It never:**

- posts a floor the feed did not agree on: fewer than 3 valid listings, confidence under the higher of 3,000 bps and the module's
  own minimum, two ordinals providers not agreeing on a listed output, or no agreed rate;
- posts a cap without a fresh module floor, or to a vault with no `capCeiling()` (alerted), or more than 20% above the last cap;
- keeps a posted floor that is stale or whose confidence is under the module's minimum (it re-posts: E-02a);
- touches the YOYO vault;
- has two transactions in flight;
- signs a transaction for a new nonce while one of its own is open. After a restart it re-broadcasts the same bytes, and a stuck
  one is replaced at the same nonce with a bumped fee (or cleared by a self-transfer), never a second nonce;
- opens a draw whose seed signature fewer than two relays served or the beacon refused (a relay serving other bytes is outvoted by
  the beacon and alerted as `drand-liar`, audit R2-S2), or whose snapshot is not served, byte for byte, at its URI and at the
  mirror's object.

**Alerts.** Each pass report carries `alerts` (on `/status/full`; their count on `/status` and `/healthz`): a missed slot, a draw
not opened in time, a snapshot or publish failure, drand relays that disagree or are late, a ticket or draw pinned past its round,
a vault with no ceiling, a stuck or replaced transaction, a pass that ran out of its time budget, a draw Resolved for over an
hour with no provable claim (`claim-stuck`), a drand relay whose bytes the beacon refused (`drand-liar`). **How they page (audit
R2-S8):** once alerts have been in force for `KEEPER_ALERT_PASSES` passes in a row (default 3, so about 3 minutes), `/healthz`
answers **503**, and its JSON body names the alert kinds (`alertKinds`: the word before the colon, never the text). The
monitor's liveness poll pages a non-200 after 10 minutes and puts those kinds in the page; the texts are on `/status/full`. An
alert stays in force `KEEPER_ALERT_LATCH_S` (20 minutes) after the last pass that raised it (R3-check L-G), and never less than
`(KEEPER_ALERT_PASSES + 1) × interval + 12 minutes` (R4-check L-S2: at `KEEPER_INTERVAL_S` 300, 32 minutes; at 1,800, 2 h 12 min),
so a one-pass or flapping alert pages at every allowed interval too; a pass with nothing in force resets the count. The standalone resolver's `/healthz` follows the same rule with its own alerts.

### 2.1 The standalone resolver

`npm run resolver` (`keeper/service/resolver-main.ts`), the same image with the start command
`node --import tsx service/resolver-main.ts`. It makes only calls anyone may make, from its **own** funded key, and keeps working
while the keeper is paused or down. Put it in a Railway project other than the keeper's (the monitor's is a good home).

| variable | required | secret | default | meaning |
| --- | --- | --- | --- | --- |
| `KEEPER_CHAIN_ID`, `KEEPER_STACK`, `KEEPER_RPC_URLS`, `KEEPER_DRAND_URLS`, the gas caps | as for the keeper | | | read the same way |
| `RESOLVER_MODE` | | | `dry-run` | `send` to sign and send |
| `RESOLVER_PRIVATE_KEY` | send mode | **yes** | — | any funded account with no role. The registry's keeper is **refused** (the two would race for one nonce) |
| `RESOLVER_STATE_DIR` | | | `./resolver-state` | its own volume: the transaction in flight, the coin index, the log cache |
| `RESOLVER_INTERVAL_S`, `RESOLVER_STATUS_PORT` | | | `60`, none | as the keeper's |

Run it with `KEEPER_RESOLVER=off` on the keeper, or leave both on: the resolver's calls are idempotent on chain (a second
`resolve` of the same ticket reverts in simulation and sends nothing).

**Gas.** A floor post every 30 minutes and each cap about every 45 minutes come to about 110 transactions a day, plus two per
RAFFLE coin a day for its draw (announce, open) and four resolver calls (request, resolve, claim, and a ticket's resolve). At about 60,000 gas and 0.01 gwei that is well under 0.001 ETH a day. Fund the keeper with a little ETH, and
watch `balanceWei` on `/status`.

### 2.2 The snapshot mirror: Cloudflare R2

The operator's choice is its existing R2 bucket, public dev URL `https://pub-736187aaa62744868846472f72e8d77b.r2.dev`, snapshots
under `rook-snapshots/`. R2 does not take the plain `PUT <url>/<file>` of `KEEPER_SNAPSHOT_UPLOAD_URL`, so the keeper writes to it
through R2's S3-compatible API with AWS Signature V4 (`keeper/service/s3.ts`, no SDK; the signer is tested against AWS's
published examples and a stub bucket that checks every signature, `keeper/test/snapshot-s3.test.ts`):

```
KEEPER_SNAPSHOT_S3_ENDPOINT=https://<account id>.r2.cloudflarestorage.com
KEEPER_SNAPSHOT_S3_BUCKET=<the bucket>
KEEPER_SNAPSHOT_S3_PREFIX=rook-snapshots/                      # the default
KEEPER_SNAPSHOT_S3_ACCESS_KEY_ID=<R2 API token: access key id>
KEEPER_SNAPSHOT_S3_SECRET_ACCESS_KEY=<R2 API token: secret>    # secret (Railway's secret store)
KEEPER_SNAPSHOT_BASE_URL=https://pub-736187aaa62744868846472f72e8d77b.r2.dev/rook-snapshots
```

- **The token.** An R2 API token with *Object Read & Write* on **this bucket only**. The keeper writes with no ACL header and
  nothing in it can make the bucket publicly writable: the public side is R2's read-only public URL.
- **Publishing.** Each draw's file is written to `/data`, `PUT` to `<bucket>/rook-snapshots/<file>` (`Content-Type:
  application/json`), then read back twice, byte for byte, before `openDraw`: from the URI the draw will carry
  (`KEEPER_SNAPSHOT_BASE_URL/<file>`, the r2.dev URL) and from the bucket's own object (a signed GET). Any difference, or a
  file not served yet, stops the draw with an alert. The URI on chain names the r2.dev host, never the keeper's (R2-S5).
- **Who reads it.** The indexer fetches the list from the URI in the `DrawOpened` event through its outbound guard (https, a
  public name, every address public: `*.r2.dev` resolves to Cloudflare's public addresses and passes, `indexer/test/w2.test.ts`)
  and shows whether it rebuilds to the on-chain root; the site links that URI and shows the indexer's verdict; the keeper's
  resolver reads it only as data checked against the root. The monitor does not need the file: it recomputes the root from the
  chain's Transfer logs.
- **r2.dev is for now.** Cloudflare rate-limits `r2.dev` URLs and advises a custom domain for production. The plan is
  `snapshots.rook.fun` once `rook.fun` is registered: connect it to the bucket in R2 and change **only**
  `KEEPER_SNAPSHOT_BASE_URL` (to `https://snapshots.rook.fun/rook-snapshots`, with the prefix as the path). Draws already opened
  keep their r2.dev URI on chain, so leave the r2.dev URL enabled until those draws are final.

`KEEPER_SNAPSHOT_UPLOAD_URL` (+ `KEEPER_SNAPSHOT_UPLOAD_AUTH`) remains for any other mirror that accepts a plain `PUT`. One mirror
at a time.

## 3. The runner and the offer tool (K4)

**The runner** calls the canister's `tick` as the operator, one call at a time.

| variable | required | secret | default | meaning |
| --- | --- | --- | --- | --- |
| `CANISTER_ID` | yes | | — | the buyer canister |
| `OPERATOR_IDENTITY_PEM` | yes | **yes** | — | `dfx identity export <the runner's identity>`: secp256k1 `EC PRIVATE KEY`, or Ed25519 PKCS#8. Escaped `\n` newlines are accepted. Read in `ops/src/identity.ts` only. The install argument's `operator` must be this principal |
| `IC_HOST` | | | `https://icp-api.io` | plain http only for a local replica |
| `RUNNER_TICK_EVERY_S` | | | `600` | 60 to 86,400. Each tick costs cycles (`canister-mainnet.md`). Ten minutes is well inside every flow's deadlines |
| `PORT` | | | `8080` | `/healthz` (503 after three failed ticks in a row, or no tick for three intervals) and `/status` |
| `RUNNER_RELAY` | | | `on` | `on` or `off`: the relay step below (`docs/v6/relay-conversion.md` §1 step 4). Off, a conversion waits in `Funded` and the monitor pages after 2 h |
| `RELAY_API_URL` | | | `https://api.relay.link` | https only. Relay's public quote API; no key |
| `RUNNER_ETH_NFT` | | | `on` | `on` or `off`: the Ethereum NFT step (`docs/v7/phase1-nft.md` §6). It does nothing while the canister's `eth_nft_config()` is null |
| `ETH_NFT_SLUGS` | with eth_nft | | — | `0x<collection>=<opensea slug>,…`, e.g. `0x5af0d9827e0c53e4799bb226655a1de152a425a5=milady` |
| `OPENSEA_API_KEY` | with eth_nft | **yes** | — | OpenSea's key: listings and fulfillment data are keyed. Read at each call, never logged, never sent to the canister |
| `ETH_LOGS_RPC` | with eth_nft | | — | an https Ethereum RPC that serves `eth_getLogs` over ~7,200 blocks: recent sales for the floor evidence, and (holder deposits on, D5) every NFT of a configured collection sent to the canister's Ethereum address in the last day, in 2,400-block chunks, handed in with `eth_deposit`. Without it no sales are handed in (the canister posts no floor) and no deposit is claimed (the monitor pages each one after 2 h) |

It refuses to start without a canister id or an identity. It logs the principal and every line `tick` answers. A rejected call,
such as a trap on the operator check, is logged by its reject text.

**The relay step** (`ops/src/relay.ts`). After each tick the runner reads `frog_conversions()`. For a conversion that asks for a
quote (`Funded`, its `ask` set) it calls `POST $RELAY_API_URL/quote` with exactly the ask (the canister's EVM address as `user`
and `refundTo`, the float as `recipient`, the amount, 4663 → Bitcoin, native to native, `EXACT_INPUT`), checks the answer with
the canister's rules 1–9 and 11 (`relay-conversion.md` §3.1: one deposit to the pinned depository, our value and depositor, the
order id recomputed from `orderData`, our recipient and refunds, no calls or fees, the deadline band), and hands the raw JSON in
with `relay_quote(frog, json)`. It then ticks again at once, so the canister judges the quote inside its 300 s window, on its own
reads (the rate bound, rule 10, is the canister's alone). A refused quote is logged (`relay-quote-rejected`) and nothing is
handed in. The runner can waste a quote; it cannot redirect one. `relay_give_back(frog)` (the ETH back to the module instead of
a new quote) is the operator's, by hand: `dfx canister --network ic call <buyer> relay_give_back '(<frog>)'`.

**The offer tool** is `cd ops && npm run offer -- …`, run by the operator on their own machine. It is a **dry run by default**.

- It takes a venue's seller-signed PSBT: `--psbt <hex|base64|@file>` with `--to-sign`, or UniSat's `create_bid` answer saved to
  a file with `--unisat-bid`. UniSat's shape comes from its swagger and is unverified.
- It needs the frog id, the inscription, and a membership proof: `--proof <file>`, or `--members <the published id list>`.
- It needs the listing root: `--root`, or `--stack` to read it on 4663.
- It needs the float address, `--float`. The custody address comes from the stack or `--custody`.
- The cap is `--cap-wei`, or read from `frog_buys()` with `--canister`. The rate is `--sats-per-eth`, or the agreed Coinbase and
  Kraken rate.
- It reads the location and the contents from two ordinals providers (`UNISAT_API_KEY` for the second), and from two
  explorers.
- It runs every check the canister's `frogflow::accept` and `frogbuy::judge` make, in their order.
- It prints every input and output with its owner, the spend, fee and cost, **each input the canister would sign with its
  BIP-341 key-path sighash**, and the `frog_offer` Candid arguments.
- Only `--send`, with `OPERATOR_IDENTITY_PEM` and every check passed, calls `frog_offer`. The canister judges it again.

## 4. The reconciler (K7, gate G5)

It runs hourly:

- the indexer's reserves code over the stack file, with every inscription's location confirmed by **two** ordinals providers
  (ordinals.com and Ordinals Wallet agreeing on the address and the satpoint, or the item is unverified: audit E-13);
- each FloorModule's **real ETH** (`eth_getBalance`) against its own books (`backing + reserve + surplus + revenue + gradEth +
  rentalEscrow + renterPool + owedTotal`), its reserve units against the wrapper's ETH-backed units, and its escrow per unit
  against the floor; a floor stale for an hour past its TTL, or the stale fallback, is red (audit E-05);
- the canister's `custody_caps()` and the stage it implies;
- red when the report is older than 3 hours;
- Telegram on red, on "cannot read", and on unverified for 6 hours (audit E-08a).

| variable | required | secret | default | meaning |
| --- | --- | --- | --- | --- |
| `RECONCILE_CHAIN_ID` | yes | | — | `4663`. The stack file must say the same |
| `RECONCILE_STACK` | yes | | — | `/app/deployments/stack-4663.json` in the image (a valid `WRAPPED_CONFIG`) |
| `RECONCILE_RPC_URLS` | | | as the keeper's | read-only, with the chain id pinned |
| `CANISTER_ID` | for green | | — | `custody_caps()` is a query, read anonymously. Without it the caps are unverified, and the whole is **never green** |
| `IC_HOST` | | | `https://icp-api.io` | |
| `ALERT_TELEGRAM_BOT_TOKEN` | **yes** | **yes** | — | the monitor's channel (`monitor/src/alerts.mjs`). Used only in the request path, and scrubbed from every line and error. **Without it (or the chat id) the reconciler refuses to start** (audit E-08b: it used to swallow reds into the log); `--dry-run` on the command line is the only way to run without it |
| `ALERT_TELEGRAM_CHAT_ID` | **yes** | | — | as above |
| `RECONCILE_EVERY_S` | | | `3600` | 60 to 3,600 |
| `RECONCILE_STALE_S` | | | `10800` | a report older than this is served **red** (G5: 3 hours) |
| `RECONCILE_REALERT_S` | | | `21600` | how often a red that stays red is said again |
| `RECONCILE_STATE_DIR` | | | `/data` in the image | the last report and the alert state survive a restart |
| `RECONCILE_PUBLIC_URL` | | | — | named in the alerts |
| `RECONCILE_UNVERIFIED_ALERT_S` | | | `21600` | unverified this long is paged (a run that reads nothing is paged at once) |
| `RECONCILE_RUN_TIMEOUT_S` | | | `900` | a run not finished by then is abandoned (logged); the watchdog, its own timer, turns the aging report red |
| `RECONCILE_MIN_BACKING_BPS` | | | `5000` | a module whose escrow per ETH-backed unit is under this share of its floor is red |
| `RECONCILE_STALE_FLOOR_RED_S` | | | `3600` | a floor not fresh this long past its TTL is red (the monitor pages it after 15 minutes) |
| `PORT` | | | `8080` | `/` (the page: banner, age, reasons, caps and stage), `/v1/reserves` (JSON), `/healthz` |

**Status.** The status is the worst of the collections' reserve reports (green, red or unverified: `ux-and-reserves.md` §2), the
caps and the modules' ETH:

- caps above the stack's stage are **red**. G7 says they must match;
- caps below it add a note;
- caps that cannot be read are unverified;
- a module short of its books, backing units it cannot, under the floor fraction, stale past the hour or escaped is **red**.

The age is judged when the report is served, and a watchdog runs every 5 minutes **on its own timer**, so a hung job turns red,
and is alerted, without running; a run is abandoned after `RECONCILE_RUN_TIMEOUT_S`.

**Alerts** go out:

- when the status turns red;
- when the reasons change;
- every 6 hours while it stays red;
- once when it recovers.

Severity is SEV1 when an item is not in custody or the equation fails, and SEV2 when the report is stale or the caps mismatch.
Unverified is SEV2: at once when nothing could be read ("cannot read"), after 6 hours otherwise, then every 6 hours while it lasts.

**Its own test alert.** `node --import tsx src/reconcile.ts --test-alert` (in `ops/`, or on the service with `railway ssh`) sends a
red, an unverified and a recovery, each marked `🧪 TEST`, through **this service's** variables. The monitor's `--test-alert` does
not test them (E-15).

**`/healthz` is checked by Railway only when it deploys** (it does not restart a running service whose health check fails). The
continuous check of every service's `/healthz` (keeper, standalone resolver, runner, reconciler, indexer) is the monitor's `liveness` list
(`k6-watcher-canary.md`, wave 2), which pages one that has not answered 200 for 10 minutes.

## 5. Order of starting (go-live checklist step 11)

1. **Reconciler** first. It holds no key. Check `/` and `/v1/reserves` show the collection and `custody_caps()`, and send a test
   alert through **its own** channel (`node --import tsx src/reconcile.ts --test-alert`: 3 messages), then the monitor's
   (`node monitor/src/main.mjs --test-alert all`: 16 messages). Each service must post to the phone; a missing variable is a
   refusal to start, not a silent log line.
2. **Keeper in a dry run.** Set every variable but `KEEPER_MODE` and `KEEPER_PRIVATE_KEY`, then read `/status`:
   - the floor plan and why;
   - the caps it would post;
   - the draw rule;
   - that there are no refusals.

   With `UNISAT_API_KEY` unset it says "no floor … two ordinals providers did not agree", which is the rule working.
3. **Keeper sending.** Add `KEEPER_PRIVATE_KEY` and `KEEPER_MODE=send`. Watch the first `tx-mined` lines, `module.fresh()`
   and both vaults' `capFresh()`.
4. **Runner.** Add `OPERATOR_IDENTITY_PEM` and `CANISTER_ID`. The first line is the principal, which must be the install
   argument's `operator`.

**The custody fee pool's minimum** (go-live step 11, `docs/v6/phase1-scope.md` §3.3) is one value, **20,000 sats** by default (the
operator's choice, 2026-09-26, instead of holding 100,000 sats ≈ $84 for fee spikes): `script/golive-check.sh --canister <buyer>
--min-fee-pool-sats <sats>` FAILs without one usable `custody_fee_pool()` entry that large, and the monitor (K6) pages SEV2 below
its `min_fee_pool_sats` (same default). The canister's conflict-out ceiling stays 100,000 sats; an entry under it passes with a
note (golive-check) and an INFO (monitor). In a large fee spike a stuck delivery is cancelled more slowly; the gated Wrapper
(refunds only after markUndeliverable or 30 days) keeps that from becoming a loss.

To stop a service, remove it or scale it to zero. A stopped keeper leaves the module stale after an hour, so mint and burn are
refused. That is the safe failure, and it is the one `phase1-scope.md` §1 describes.

## 6. Tested, locally only

- `cd keeper && npm test`: the service's configuration, sender, plans, feed, draws and guards, against a fake node.
- `cd ops && npm test`: the runner, the identity, the offer tool's judge (every refusal of `frogbuy::judge`), and the
  reconciler's stage, staleness, alerts and token handling.
- `scripts/keeper-fork-test.sh`: the whole stack on an anvil fork of 4663 (`rpc.ordofi.network`, head − 2,000), under the staging
  chain id, with anvil's dev account 3 as the keeper. It covers:
  - a dry run, a send, a restart, and a launch-graduate-sweep followed by a draw with a published snapshot, which
    `tools/snapshot`'s CLI recomputes;
  - every refusal;
  - a dry run on the node switched to report 4663;
  - the reconciler against the same fork, with and without a canister (a local replica) for `custody_caps()`.
- `ops/scripts/runner-replica-test.sh`: the runner against this worktree's canister on a local replica. The operator's
  tick is answered, a stranger's is refused, and no identity refuses at start.
