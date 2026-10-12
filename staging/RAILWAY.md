# Hosting staging on Railway (the parent runs this; nothing here was run)

Two new services in the existing project `etch`, beside `web`. **No variable below is a secret**, and staging uses no API key:
the UniSat key on the project is not read by anything in staging, so do not share it with these services.
The Dockerfile has **not been built** (Docker was down on the machine that wrote it): build it locally first if you can:
`docker build -f staging/chain/Dockerfile -t etched-staging-chain .`

## 1. `staging-chain`: node + bootstrap + proxy + operator, one container, one volume

```sh
cd ~/Documents/Etch && railway link            # project etch
railway add --service staging-chain
railway service staging-chain
railway volume add --mount-path /data
railway variables --set RAILWAY_DOCKERFILE_PATH=staging/chain/Dockerfile \
                  --set STAGING_WAIT_S=120
railway domain                                  # note the URL: https://<chain-domain>
railway variables --set STAGING_PUBLIC_URL=https://<chain-domain>
railway up --service staging-chain --detach
```

Check: `curl https://<chain-domain>/healthz` says `"chainId":9466301` and `"deployed":true` (the first boot compiles nothing, deploys in
under a minute); `curl https://<chain-domain>/info` lists the addresses; an admin call is refused:
`curl -X POST -H 'content-type: application/json' --data '{"jsonrpc":"2.0","id":1,"method":"anvil_setBalance","params":[]}' https://<chain-domain>/`

Optional variables: `STAGING_FAUCET_WEI` (default 10 play ETH a day per address; the smoke test was run with the default), `STAGING_UPSTREAM` (default the public Robinhood RPC), `STAGING_FAUCET_PER_IP` (default 5; the smoke test funds four wallets from one address), `STAGING_TICK_S`,
`STAGING_CHOOSE_AFTER_S` (300), `STAGING_SETTLE_AFTER_S` (45), `STAGING_MANUAL_SETTLE=1` (you settle by hand with `npm run attest`),
`STAGING_LISTING_WINDOW` (12: how many otherwise-quiet listings the operator reads per tick).

**The inventory (docs/phase11.md).** On first boot the chain lists the committed catalogue as 58 vaults — 39 collections and 19
runes — beside the proxy rather than inside the boot path, in batches of eight, and `/healthz` reports `"42 of 58 vaults
created"` while it fills. It is idempotent: a restart finishes an unfinished inventory and lists nothing twice. `STAGING_LISTINGS=0`
turns it off and leaves the two demo listings; `STAGING_LISTINGS_BATCH` (8) changes the batch size. Measured on a fork of
Robinhood Chain, a cold first boot reaches a full inventory in 16.6 s against the container's 180 s healthcheck start period.
Every membership root there is a **placeholder** and every Bitcoin address is a marker string nobody can be paid at; the manifest
at `${STAGING_DATA}/listings.json` says so, listing by listing.
Give it 1 GB of memory or more: a fork caches what it reads.

The **simulators** (docs/phase8.md) start in this same container, so that a visitor meets a product in use rather than an empty
grid. Every switch below is optional, none is a secret, and all default on; `STAGING_SIM=0` turns the lot off and leaves the
chain exactly as it was before phase 8.

| Variable | Default | What it does |
|---|---|---|
| `STAGING_SIM` | `1` | all of the below |
| `STAGING_SIM_TRADE` | `1` | launching coins and trading their curves |
| `STAGING_SIM_HARVEST` | `1` | a stranger calling `harvest()` on a schedule |
| `STAGING_SIM_SPREAD` | `1` | holding every purchase and draw state occupied at once |
| `STAGING_SIM_COINS` | `64` | how many simulated coins at most: one for every vault of the catalogue, and a few over |
| `STAGING_SIM_WALLETS` | `30` | how many throwaway wallets (no key of theirs is written or logged) |
| `STAGING_SIM_EVERY_MS` | `4000` | the base gap between trades |
| `STAGING_SIM_HARVEST_MS` | `45000` | how often a harvest is attempted |
| `STAGING_SIM_SPREAD_MS` | `20000` | how often the states are re-checked |
| `STAGING_SIM_TOPUP` | `1` | topping a vault up to its cap as the treasury, so purchases keep flowing (docs/phase11.md §7) |
| `STAGING_SIM_TOPUP_MS` | `12000` | how often one vault is topped up |
| `STAGING_SIM_SPREAD_WINDOW` | `12` | how many vaults' counts a spread pass re-reads |
| `STAGING_SIM_LISTINGS_MS` | `30000` | how often the listing set is re-read while the inventory fills |

The simulators fund their own wallets with the node's own method and never the faucet, so a visitor's daily faucet allowance is
never spent by them. `STAGING_CHOOSE_AFTER_S` should stay well above the spread interval: the simulator lets each coin's creator
choose what happens to what its fees bought, across every mode, and the operator's fallback is only there for what it misses.

## 2. `staging-api`: the indexer, pointed at the proxy

```sh
railway add --service staging-api
railway service staging-api
railway volume add --mount-path /data
railway variables --set RAILWAY_DOCKERFILE_PATH=indexer/Dockerfile \
                  --set RPC_URL=https://<chain-domain> \
                  --set DEPLOYMENT_FROM_RPC=1 \
                  --set DB_PATH=/data/etch.db --set TRUST_PROXY=1
railway domain                                  # https://<api-domain>
railway up --service staging-api --detach
```

Check: `curl https://<api-domain>/v1/health`, then `/v1/coins` after the first launch.

## 3. The website in staging mode

Either a third service `web-staging` built from `web/Dockerfile`, or (simpler) the existing `web` service. They are **build-time**
variables; without them the site is exactly what it is today.

```sh
railway service web            # or web-staging
railway variables --set VITE_ETCH_STAGING_RPC=https://<chain-domain> --set VITE_ETCH_STAGING_API=https://<api-domain>
railway up --service web --detach
```

On the main site staging appears only at `#/staging` (the app routes on the hash: `https://etched.meme/?net=staging#/staging`) and under
`?net=staging`, behind a STAGING ribbon; the default view is unchanged.

## 4. Proving it

```sh
cd staging && npm ci && npm run smoke -- https://<chain-domain> https://<api-domain>
```

Twelve checks, about five minutes. It launches one coin with a throwaway key it makes for chain 9466301 and throws away.

## Resetting

Stop `staging-chain`, empty its volume (`railway ssh` then `rm -rf /data/*`, or delete and re-add the volume), start it: a new
fork block, a new deployment. Then empty `staging-api`'s volume too, or it will hold coins that no longer exist.

## What to watch

`/healthz` has `upstreamFallbacks`: the public RPC forgets state after about fifteen minutes, so reads of state the fork has not
cached are answered from the present head (docs/staging.md, "A fork that outlives its block"). That number growing is normal. If
you have an archive RPC for Robinhood Chain, set `STAGING_UPSTREAM` to it (that URL may then be a secret; nothing else here is).
The chain id is 9466301 (`staging/chain-id.json`): the service refuses to run as 4663 or as 46630, Robinhood's public testnet.
