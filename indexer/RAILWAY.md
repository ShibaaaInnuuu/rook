> The **Solana** indexer is a separate service: `indexer/Dockerfile.sol`, `indexer/railway.sol.json`, setup in docs/v9/indexer-solana.md §6.
> This file is the Robinhood indexer's (stopped since 2026-10-09).

# Deploying the indexer + API to Railway (later — nothing here has been run)

One service: the follower and the read-only API in one process, with a volume for the SQLite file. **Deploy it only once
`deployments/4663.json` exists** (a public deployment, which is the owner's action). Before that the service would start, find
no deployment for the chain, idle, and answer every list with nothing — correct, and pointless to pay for.

No variable below is a secret. There is no key anywhere in this package: it cannot sign.

```
cd ~/Documents/Etch && git status --short && git pull          # `railway up` ships the folder as it is: it must be clean
railway link                                                   # the existing `etch` project (created for the website), or: railway init --name etch
railway add --service indexer
railway volume add --service indexer --mount-path /data
railway variables --service indexer --set "RPC_URL=https://rpc.mainnet.chain.robinhood.com" \
  --set "DB_PATH=/data/etch.db" --set "DEPLOYMENTS_DIR=/app/deployments" --set "CONFIRMATIONS=20" \
  --set "TRUST_PROXY=1" --set "RAILWAY_DOCKERFILE_PATH=indexer/Dockerfile"
railway up --service indexer --ci                              # build context is the repo root: the Dockerfile copies deployments/
railway domain --service indexer
curl -s https://<the domain>/v1/health                         # deployment: true, lagBlocks small, lastError: null
```

- `RAILWAY_DOCKERFILE_PATH` tells Railway which Dockerfile to build from the root context. The container runs as `node`; if the
  volume is not writable by it, set `RAILWAY_RUN_UID=0`.
- `TRUST_PROXY=1` because Railway's edge sets `X-Forwarded-For`; without it every visitor shares one rate-limit bucket.
- A dedicated RPC is kinder to the public endpoint. If its URL carries a key, it is a secret: set it in Railway's UI, never in a
  command that lands in shell history, and note that `/v1/health` never echoes the URL.
- To re-index from scratch: delete the volume's `etch.db*` files and restart. Every derived table is rebuilt from the chain.
- The website reads the API's base URL at build time (see the website's own notes); CORS is open by design.
