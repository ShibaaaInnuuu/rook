# Railway layout for the launch

## 0. The Solana launch (canister-only, 2026-10-12): what runs, and when

The Robinhood-era services below (keeper, resolver, the 4663 indexer) stay stopped. The Solana launch runs four services, all made
**at the ceremony**, after the canister exists (each needs its id or its addresses; made earlier they only fail their healthcheck):

| service | config-as-code | template | needs from the ceremony | secrets (sealed, set in Railway's UI by the operator) |
|---|---|---|---|---|
| `runner` | `/ops/railway.runner.json` | `runner.env.example` (`ROOK_HOME=solana`) | `CANISTER_ID` | `OPERATOR_IDENTITY_PEM`, `OPENSEA_API_KEY` (the rotated one), `UNISAT_API_KEY`, `SOL_RPC_URLS` (holds the Helius key) |
| `indexer-sol` | `/indexer/railway.sol.json`, volume `/data` | `indexer.env.example` (top section) | `ROOK_CANISTER_ID`, `ROOK_DESK_KEYS` | `SOLANA_RPC_URL`, `HELIUS_API_KEY` |
| `monitor` | `/monitor/railway.json`, volume `/data` | `monitor.env.example` | the canister id in its config | `ALERT_TELEGRAM_BOT_TOKEN` |
| `fueler` | `/ops/railway.fueler.json`, volume `/data` | `fueler.env.example` (`FUEL_HOME=books`, starts `dry`) | the canister id; its hot key set as `treasury_set_ops` | `FUELER_SOL_KEYPAIR`, `FUELER_IC_IDENTITY_PEM`, `FUELER_EVM_KEY`, `RELAY_API_KEY` |

Keys: one Helius key per service (the canister's own key is a third, in its install argument), each on the free plan until the month
runs short (decision O-13). Never reuse the keys pasted into a chat. Steps, from the repository root with a clean tree:

```
railway link                                              # the Rook project (rook-site lives there)
railway add --service runner
railway add --service indexer-sol && railway volume add --service indexer-sol --mount-path /data
# per service, in the UI: Settings → Source = this repo, root /, config-as-code path as in the table; Variables = the template's
# non-secret values with the ceremony's ids filled in; the secrets sealed
node deployments/launch/railway/check-env.mjs             # every template still names every variable its code reads
```

Then `GET /healthz` (runner) and `/v1/health` (indexer-sol) answer 200, and the runner's first tick shows in `coins_log`.

## The earlier layout (2026-09-28): the Ethereum NFT collections and Bitcoin Frogs (templates only)

Written 2026-09-28 on `golive-prep`. **No Railway service, project or variable was created or changed.** This folder is the
launch-specific layer on top of `docs/deploy/services-v1.md` (every keeper, runner and reconciler variable explained) and
`docs/v6/k6-watcher-canary.md` §7 (the monitor). It holds:

| file | what |
| --- | --- |
| `runner.env.example`, `keeper.env.example`, `resolver.env.example`, `monitor.env.example`, `reconciler.env.example`, `indexer.env.example`, `fueler.env.example` | every variable each service's code reads: default, launch value, **SECRET** marker (set by the operator in Railway), `<deploy-time>` placeholders |
| `check-env.mjs` | `node deployments/launch/railway/check-env.mjs`: holds each template to the variables the code reads (exit 1 on a missing one). Read-only |
| `inline-monitor-config.mjs` | builds the monitor's `MONITOR_CONFIG_JSON` from the filled `monitor-nft.json` (the three referenced files inlined) and checks it with the monitor's own loader |

Secrets never go in these files, a shell command, git, or a project-shared variable: each is a **sealed service variable** set in
Railway's UI on the one service that needs it.

## 1. The services

| service | image (build context: repo root) | Railway config file | start command | healthcheck | volume | replicas | Railway project |
| --- | --- | --- | --- | --- | --- | --- | --- |
| **runner** (K4) | `ops/Dockerfile.runner` | `/ops/railway.runner.json` | image CMD `node --import tsx src/runner.ts` | `/healthz` | none | **1** | the operator's (holds the IC identity) |
| **keeper** (K3) | `keeper/Dockerfile` | `/keeper/railway.json` | image CMD `node --import tsx service/main.ts` | `/healthz` | `/data` | **1** | the keeper's (holds the keeper key); not the runner's service, ideally not its project |
| **resolver** (keyless) | `keeper/Dockerfile` | `/keeper/railway.json` | **override**: `node --import tsx service/resolver-main.ts` | `/healthz` (needs `RESOLVER_STATUS_PORT`) | `/data` (needs `RESOLVER_STATE_DIR=/data`) | **1** | the monitor's (docs/deploy/services-v1.md §2.1) |
| **monitor** (K6) | `monitor/Dockerfile` | `/monitor/railway.json` | image CMD `node monitor/src/main.mjs` | `/healthz` | `/data` (`monitor-state.json`, holder indexes) | **1** | its own, another account if possible (k6 §7) |
| **reconciler** (K7) | `ops/Dockerfile.reconciler` | `/ops/railway.reconciler.json` | image CMD `node --import tsx src/reconcile.ts` | `/healthz` | `/data` (last report, alert state) | **1** | the monitor's |
| **indexer** + API | `indexer/Dockerfile` (`RAILWAY_DOCKERFILE_PATH`, no railway.json) | — | image CMD | `/v1/health` | `/data` (`etch.db`) | 1 | the site's (`indexer/RAILWAY.md`) |
| **fueler** (cycles from fees, docs/v7/cycles-from-fees.md §4.2) | `ops/Dockerfile.fueler` | `/ops/railway.fueler.json` | image CMD `node --import tsx src/fueler.ts` | `/healthz` | `/data` (`fueler.json`, the journal: a lost volume cannot make it pay twice, but a run in flight then needs a human) | **1** | the operator's, its own service (its keys are its own) |

Each service: "add from repo", root directory `/`, config file path as above. The images that write to `/data` start as root only
to `chown` the volume, then drop to `node` (`ops/docker/drop-to-node.sh`, `indexer/entrypoint.sh`).

**Why one replica each.** The runner is the canister's only `tick` caller: a second one doubles the floor-evidence and quote
hand-ins and the cycles bill, and the canister refuses the overlap anyway. The keeper and the resolver each keep one transaction in
flight per key on their volume (a second replica races the nonce). Two monitors double every page and, with the guardian key, race
its pauses. `ops/railway.runner.json`, `railway.reconciler.json` and `keeper/railway.json` pin `numReplicas: 1`;
`monitor/railway.json` does not (Railway's default is 1): keep it at 1. The fueler keeps one transaction in flight per chain on its
volume, like the keeper: `ops/railway.fueler.json` pins `numReplicas: 1`.

**Who holds what.**

| secret | service | read in |
| --- | --- | --- |
| `OPERATOR_IDENTITY_PEM` (the operator's hot IC identity; its principal is the install argument's `operator`) | runner | `ops/src/identity.ts:23` |
| `OPENSEA_API_KEY` (listings, fulfillment data) | runner (optionally keeper, indexer) | `ops/src/runner.ts:213,220`; scrubbed `ops/src/log.ts:21` |
| `ETH_LOGS_RPC` when it is the keyed Dwellir URL | runner | `ops/src/runner.ts:67` |
| `KEEPER_PRIVATE_KEY`, `KEEPER_SNAPSHOT_S3_SECRET_ACCESS_KEY`, `KEEPER_STATUS_TOKEN` | keeper | `keeper/service/signer.ts`, `config.ts` |
| `RESOLVER_PRIVATE_KEY` (any funded 4663 account with no role) | resolver | `keeper/service/signer.ts` |
| `ALERT_TELEGRAM_BOT_TOKEN` | monitor, reconciler | `monitor/src/alerts.mjs:52` |
| `MONITOR_GUARDIAN_KEY` (the guardian EOA = `registry.guardian()`) | monitor only | `monitor/src/guardian.mjs:25` |
| `DWELLIR_API_KEY` (expanded into `eth_nft.eth_rpc` at load, redacted from every alert) | monitor | `monitor/src/config.mjs:73` |
| `MONITOR_COINGECKO_KEY` (optional) | monitor | `monitor/src/ethnft.mjs:489` |
| `FUELER_EVM_KEY` (a fresh EOA, used for nothing else: `treasury.ops()`, the same address on 4663 and Ethereum) | fueler only | `ops/src/fuel/keys.ts` |
| `FUELER_SOL_KEYPAIR` (the Solana hot key = the canister's `treasury.ops`, a fresh `solana-keygen` JSON; or `FUELER_SOL_KEYPAIR_FILE`; docs/v9/fueler-solana.md, docs/v9/canister-only-solana.md §9.2) | fueler only | `ops/src/fuel/keys.ts` |
| `FUELER_IC_IDENTITY_PEM` (a fresh IC identity, never the runner's) | fueler only | `ops/src/fuel/keys.ts` |
| `RELAY_API_KEY` (Relay's `/quote/v2` needs one from 2026-10-02) | fueler | `ops/src/fueler.ts` |

## 2. Launch values that matter

- **Runner:** `RUNNER_PENDING_EVERY_S=120` (docs/v7/phase1-nft.md §5.1; the runner refuses more than 300 and anything but digits,
  `ops/src/runner.ts:52-55`); `RUNNER_ETH_NFT=on`; `RUNNER_IDLE_TICK_EVERY_S=900` (docs/v7/dormant-collections.md §7: the tick while every collection sleeps); `ETH_NFT_SLUGS` = the eleven allowlisted collections (Identity MD since 2026-10-09) of `nft-pilot.json` as
  `0x<collection>=<openseaSlug>` in its order (checked equal to the file); `ETH_LOGS_MAX_BLOCKS=500`; `ETH_LOGS_RPC` = the
  operator's Dwellir URL (the key is in the path: set the whole URL sealed, or as a Railway reference to a sealed
  `DWELLIR_API_KEY`); keyless fallback `https://xrpc.cl/eth`. Without `ETH_LOGS_RPC` no holder deposit is ever discovered
  (deposits are on at launch, D5; the monitor pages each after 2 h). This URL is the runner's only: never in the canister's install
  argument (published with every proposal).
- **Monitor:** the filled `deployments/launch/monitor-nft.json`. The image has no `deployments/` tree and the file references
  `../canister-ic.json`, `nft-pilot.json` and `../stack-4663.json` relative to itself, which `MONITOR_CONFIG_JSON` cannot resolve
  (the loader uses the cwd then). Use `node deployments/launch/railway/inline-monitor-config.mjs deployments/launch/monitor-nft.json`
  and paste its stdout as `MONITOR_CONFIG_JSON` (it prints the loader's verdict on stderr; `${DWELLIR_API_KEY}` stays a
  placeholder, expanded by the monitor at start). Also run `script/golive-check.sh … --monitor-config <the filled file>`.
- **Resolver:** `RESOLVER_STATE_DIR=/data` and `RESOLVER_STATUS_PORT=8080` are required in practice: the defaults are
  `./resolver-state` (off the volume) and no status server at all (the resolver never reads `PORT`), so the shared
  `keeper/railway.json` healthcheck would fail the deploy.
- **Keeper / reconciler / indexer:** `KEEPER_STACK` / `RECONCILE_STACK` / `WRAPPED_CONFIG` = `/app/deployments/stack-4663.json`,
  baked into their images at build: commit the deploy's `deployments/stack-4663.json` before building them. For the NFT collections
  the keeper posts no floor (the canister does, `keeper/service/keeper.ts:68`).
- **Ordinals at launch (the operator, 2026-10-09; `nft-pilot.json` `bitcoin`).** Bitcoin Frogs is in the same stack file (the mixed
  deploy). **Keeper:** `KEEPER_OW_SLUGS=bitcoin-frogs=bitcoin-frogs` (the stack slug, then the Ordinals Wallet slug): it posts the
  Bitcoin module's floor from the OW book, so it runs in `send` with `KEEPER_PRIVATE_KEY` (docs/v8/ordinals-wave.md §3.4).
  **Runner:** `RUNNER_RELAY=on` (each frog advance converts through Relay, `frog_funding` 400 bps, in the same tick as the Ethereum
  step); `RUNNER_INSCRIPTIONS=bitcoin-frogs=bitcoin-frogs:/app/deployments/launch/btc/bitcoin-frogs-members.json` (the old
  `RUNNER_OW_SLUG` / `RUNNER_OW_MEMBERS` still read as Bitcoin Frogs' entry) only once the protected-buy build is reviewed and installed
  and a funded rehearsal has passed (until then Bitcoin buys are manual, `npm run offer -- --coll <its coll>`). Every Inscriptions
  collection is per collection since the multi-collection canister: the floor reading and the holder proofs follow each LIVE one
  (`frog_wake(coll)`), and a further collection's buying is one more `RUNNER_INSCRIPTIONS` entry.
  **Reconciler:** nothing new: it reads the Bitcoin module and its custody from the same stack file and `custody_caps()`.
  **Monitor:** the `bitcoin` canary block of `monitor-nft.json` (custody, float and buyer from the canister after the install).

## 3. Order at go-live (after the canister install and the 4663 stack deploy)

Follows `docs/deploy/services-v1.md` §5 with the NFT side added:

1. **Monitor** (its own project, volume `/data`): `MONITOR_CONFIG_JSON`, the Telegram pair, `DWELLIR_API_KEY`,
   `MONITOR_GUARDIAN_KEY`. `node monitor/src/main.mjs --test-alert all` on the service (`railway ssh`): every kind reaches the phone.
2. **Reconciler** (monitor's project, volume `/data`): `CANISTER_ID`, the Telegram pair. `--test-alert` through its own channel;
   `/` and `/v1/reserves` show the collections and `custody_caps()`; `/v1/eth-records` answers.
3. **Indexer** (volume `/data`), with `CANISTER_RECORDS_URL` = the reconciler's `/v1/eth-records` if the v8 routes are on.
4. **Resolver** in `dry-run`, then `send` with its own funded key (monitor's project).
5. **Keeper** in `dry-run` (read `/status`: caps, draws, no refusals), then `KEEPER_PRIVATE_KEY` + `KEEPER_MODE=send`.
6. **Runner** last: `CANISTER_ID`, `OPERATOR_IDENTITY_PEM`, `OPENSEA_API_KEY`, `ETH_LOGS_RPC`, `ETH_NFT_SLUGS`,
   `RUNNER_PENDING_EVERY_S=120`. Its first log line is the principal (must be the install argument's `operator`); then read the
   `eth-nft` lines (quotes, listings, evidence accepted per collection within 40 min, else `/healthz` 503) and measure the cycle
   length (review round 7, D7-I-3: over ≈ 250 s leaves no margin for the 4663 reach).
7. Put every service's public `/healthz` into the monitor configuration's `liveness` list (runner, reconciler, indexer, **keeper,
   resolver**) and redeploy the monitor.

## 4. Found while writing this (fixed on this branch, `golive-prep`)

- **The runner image could not start.** `ops/Dockerfile.runner` (and its `.dockerignore`) copied only `runner.ts`, `ic.ts`,
  `identity.ts`, `log.ts`, but `runner.ts:5-6` imports `./relay.js` and `./ethnft.js` (`ERR_MODULE_NOT_FOUND … src/relay.js`).
  **Fixed:** both files are copied; the image built and started to its config refusal (`CANISTER_ID is not set`).
- **The reconciler image could not start.** `ops/src/reconcile.ts` loads `./ethRecords.js`, and through the indexer and the monitor
  also `indexer/src/outbound.ts`, `indexer/src/rook/catalogSources.ts` and `monitor/src/config.mjs`, none of which were copied.
  **Fixed:** all four are copied; the image built and started to its config refusal (the Telegram pair). `ops/test/guards.test.ts`
  now holds each image's COPY lines and `.dockerignore` to everything its entry point loads, transitively.
- **Fixed:** `monitor-nft.json`'s `liveness` now names the keeper and the resolver too.
- **Fixed:** `ETH_LOGS_RPC` is scrubbed from every runner log line (`ops/src/log.ts` `urlSecrets`: the URL and its key segment).
