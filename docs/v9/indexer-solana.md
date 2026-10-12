# Rook indexer on Solana: API contract, setup and costs

Status (2026-10-10): built and tested; a read-only dry run against mainnet passed (§9). Not deployed. Code: `indexer/src/sol/`. The contract (§1–§2) was
written first so the site could be built against it in parallel.
Entry point: `indexer/src/sol/main.ts` (`npm run sol` in `indexer/`). Railway: `indexer/Dockerfile.sol`, `indexer/railway.sol.json`.

The Solana indexer is a separate process from the old Robinhood indexer (`indexer/src/main.ts`, which is kept unchanged and is not
deployed for Solana). It reads Solana mainnet through read-only JSON-RPC (it holds no key and sends no transaction), writes a SQLite
file, and serves a read-only JSON API. The shapes follow the old `/v1/rook/*` API (`indexer/src/rook/types.ts`) wherever the meaning
is the same, so the old board/chart/tape/holders components can be reused; amounts that were wei are lamports or token base units
here, and are named so.

## 1. Conventions

- Base URL: the service's Railway domain (the site reads it from `VITE_ROOK_INDEXER`, e.g. `https://rook-indexer-sol.up.railway.app`).
- Every endpoint is `GET` (and `HEAD`/`OPTIONS`), JSON, CORS `*`, no auth, no cookies. Rate limit per client IP: 240/min, burst 60;
  over it: `429 {error, status}` with `retry-after`.
- Addresses (mints, pools, wallets, signatures) are base58 strings. A coin is addressed by its **mint**.
- Times are unix **seconds** (`blockTime`). `null` always means "not known", never zero.
- SOL amounts: exact lamports as decimal strings in `*Lamports` fields, plus a JSON number in SOL where a chart or list needs one.
- Token amounts: exact base units as decimal strings (`*Raw`), plus `decimals`. Rook coins have 6 decimals (DBC's default); read it
  from the response, never assume.
- `price` is **SOL per one whole coin** (a small number like `2.8e-8`), fee-excluded (the curve-side price: SOL that reached the
  pool divided by coins that left it, see §5). `priceUsd = price × solUsd` at the trade's time when a rate is known.
- `marketCapSol = price × totalSupply` (whole coins). Rook coins have a fixed 1,000,000,000 supply.
- Errors: `{ "error": string, "status": number }` with 400/404/429/500/503.
- `source`: `{ kind: "live" | "fixture", note: string }` as before.

## 2. Endpoints

| path | cache | what |
|---|---|---|
| `/v1/health` | no-store | liveness and lag |
| `/v1/rook/coins?sort=&collection=&limit=&offset=&q=` | 5 s | the coin list (sorted views) |
| `/v1/rook/board` | 10 s | the old board shape: every coin, `rows: SolCoin[]` |
| `/v1/rook/coins/:mint` | 5 s | one coin, with its curve and links |
| `/v1/rook/coins/:mint/chart?res=1m\|5m\|15m\|1h\|1d&quote=wx\|sol\|usd` | 5 s | OHLCV candles in the coin's wX, in SOL and in USD (W13: `wx`, `sol`, `usd` series; `quote` picks which one `candles` carries) |
| `/v1/rook/coins/:mint/trades?limit=&before=` | 5 s | the tape, newest first, cursor-paged |
| `/v1/rook/coins/:mint/holders?limit=&offset=` | 30 s | top holders, labelled, and the holder count |
| `/v1/rook/activity?limit=` | 5 s | the ticker: trades, launches, graduations across all coins |
| `/v1/rook/collections` | 15 s | per collection config: coins, 24 h volume, top coin, its wX and wX/SOL rate, module and pair (W13) |
| `/v1/rook/wallets/:address` | 15 s | what a wallet launched and traded |
| `/v1/rook/desk/events?coll=&kind=&since=&limit=&before=` | 5 s | W13: the canister's memo-labelled transactions, newest first, cursor-paged (`indexer/README.md`) |
| `/v1/rook/desk/totals?coll=` | 15 s | W13: per collection, the counts by kind and the sums of what those transactions' balances moved |

W13 (canister-only design §3, §5): a Rook config's quote is its collection's **wX**, not SOL. Every `price`, `marketCapSol`,
`volume24hSol`, `sol` field keeps meaning SOL, now through the wX/SOL rate of the trade's own time (the desk's DLMM pair, else the
canister's accepted floor), and is null when no rate is near; the exact figures in the coin's own quote are `priceWx`, `marketCapWx`,
`volume24hWx`, `volume24hQuoteRaw`, `quoteRaw`, with `quote: { kind: "wx" | "sol", mint, decimals, symbol }` and `wxSol`
on every coin row. `indexer/README.md` has the field-by-field list and the new environment.

### 2.1 `GET /v1/health`
```ts
type Health = { ok: boolean; version: "sol-1"; cluster: "mainnet-beta" | "devnet"; configs: number; configsSource: string;
  coins: number; trades: number; lastPollAt: number | null; lagSeconds: number | null; backfilling: boolean; historyTruncated: boolean;
  solUsd: number | null; solUsdAt: number | null; rpc: { name: "primary" | "secondary"; ok: boolean; lastError: string | null; calls: number }[];
  lastError: string | null; note: string | null };
```
`ok` is false when no poll succeeded in the last 120 s. RPC URLs are never echoed.

### 2.2 The coin row (`SolCoin`) — used by `/coins`, `/board`, `/coins/:mint`
```ts
type SolPhase = "trading" | "graduating" | "graduated";
type SolCoin = {
  kind: "dbc";
  address: string;            // the mint (old: the coin address)
  mint: string;               // same as address
  pool: string;               // the DBC virtual pool
  dammPool: string | null;    // the DAMM v2 pool after graduation
  config: string;             // the Rook DBC config it launched under
  collection: { slug: string; listingId: number | null } ;   // "rook" (listingId null) for the genesis ROOK coin's config
  nest: { slug: string; listingId: number | null; name: null; symbol: null };   // old-shape alias of `collection`
  name: string; symbol: string;
  logoUrl: string | null;     // the metadata JSON's `image`, ipfs:// and ar:// rewritten to https gateways; null until fetched
  description: string | null; links: { website: string | null; twitter: string | null; telegram: string | null };
  metadataUri: string | null;
  creator: string;
  decimals: number;
  supplyRaw: string | null;   // mint supply, base units
  phase: SolPhase;            // graduating = curve complete (85 SOL reached) but not yet migrated
  graduated: boolean;
  launchedAt: number | null;  // the pool's creation blockTime (falls back to its activation point)
  graduatedAt: number | null; // the migration transaction's blockTime
  lastTradeAt: number | null;
  curve: { raisedLamports: string; thresholdLamports: string; progress: number } | null;  // progress 0..1 toward the config's
                              // migration threshold (85 SOL on Rook's configs); null once graduated
  raise: { raisedLamports: string; raiseLamports: string } | null;   // old-shape alias of `curve`
  price: number | null;       // SOL per whole coin, last trade (or the pool's sqrt price when no trade is indexed)
  priceUsd: number | null;
  change24h: number | null;   // fraction (0.12 = +12%) vs the last price before 24 h ago (or the first price in the window)
  marketCapSol: number | null; marketCapUsd: number | null;
  volume24hLamports: string;  volume24hSol: number; volume24hUsd: number | null;
  trades24h: number; buys24h: number; sells24h: number;
  holders: number | null;     // holder count (owners with a positive balance, pool vaults excluded); null until first counted
  holdersAt: number | null;
};
```

### 2.3 `GET /v1/rook/coins`
Query: `sort` = `new` (launchedAt desc, the default) | `graduating` ("about to graduate": not graduated, progress desc) |
`graduated` (graduated only, graduatedAt desc) | `trending` (24 h volume desc, then trades24h) | `mcap` (marketCapSol desc) |
`active` (lastTradeAt desc); `collection` = a slug (or `rook`); `q` = case-insensitive substring of name/symbol, or an exact mint;
`limit` 1–200 (50), `offset`.
```ts
type CoinList = { source: Source; at: number; solUsd: number | null; sort: string; collection: string | null; total: number; limit: number; offset: number; items: SolCoin[] };
```

### 2.4 `GET /v1/rook/board`
The old `Board` shape: `{ source, at, usdPerSol: number | null, rows: SolCoin[] }` — every indexed coin (at most 500, newest first).

### 2.5 `GET /v1/rook/coins/:mint`
`SolCoin & { tradesAllTime: number; graduation: { at: number | null; slot: number; tx: string; pool: string | null } | null; solUsd: number | null }`. 404 if the mint is not a Rook coin.

### 2.6 `GET /v1/rook/coins/:mint/chart?res=5m`
`res` ∈ `1m` (240 buckets = 4 h), `5m` (20 h), `15m` (2.5 d), `1h` (10 d), `1d` (the coin's whole life, up to 365 d). Default `5m`.
Candle shape unchanged from the old API. Empty buckets are left out (the next candle opens at the previous close).
```ts
type Candle = { t: number; o: number; h: number; l: number; c: number; v: number; n: number };   // t = bucket start (s); o/h/l/c SOL per coin; v = SOL volume; n = trades
type SolChart = {
  source: Source; kind: "dbc"; coin: string; res: "1m" | "5m" | "15m" | "1h" | "1d"; seconds: number; span: number;
  quote: "SOL";
  candles: Candle[];
  usd: Candle[] | null;       // the same buckets in USD per coin (and v in USD), from the SOL/USD rate of each bucket; null if no rate
  trades: number; tradesAllTime: number;
  launchedAt: number | null;
  graduation: { at: number | null; slot: number; tx: string; pool: string | null } | null;   // a chart marks it; trades after it are venue "pool"
  supplyRaw: string | null; decimals: number;   // market cap candle = price × supplyRaw / 10^decimals
  usdPerSol: number | null;   // the current rate (old name usdPerEth)
};
```

### 2.7 `GET /v1/rook/coins/:mint/trades?limit=50&before=<cursor>`
```ts
type SolTrade = {
  id: string;                 // `${signature}:${n}` (n = the swap's index within the transaction)
  side: "buy" | "sell";       // buy = SOL in, coin out
  venue: "curve" | "pool";    // DBC curve, or the DAMM v2 pool after graduation
  solLamports: string; sol: number;        // SOL the trader paid (buy) or received (sell), fee included as the trader saw it
  tokenRaw: string; tokens: number;        // coins received (buy) or sold (sell)
  feeLamports: string | null;              // trading + protocol + referral fee in SOL when the event says so
  price: number;              // SOL per whole coin, fee-excluded
  priceUsd: number | null;
  trader: string;             // the fee payer / signer that owns the swap's user account
  signature: string; tx: string;           // tx = signature (old field name)
  slot: number; at: number | null;
};
type SolTradePage = { source: Source; kind: "dbc"; coin: string; items: SolTrade[]; next: string | null; total: number };
```
`before` is the `next` of the previous page (an opaque `slot:seq` string). Idempotent: one row per (signature, n).

### 2.8 `GET /v1/rook/coins/:mint/holders?limit=20&offset=0`
```ts
type SolHolder = { rank: number; address: string /* owner wallet */; tokenAccount: string; amountRaw: string; amount: number;
  share: number | null /* of supply, 0..1 */; label: "creator" | "curve" | "pool" | null };
type SolHolderPage = { source: Source; kind: "dbc"; coin: string; holders: number | null; supplyRaw: string | null; decimals: number;
  complete: boolean;          // true when `items` is every holder (Helius / full scan), false when it is the top 20 only
  refreshedAt: number | null; items: SolHolder[]; total: number; limit: number; offset: number };
```
`curve` = the DBC pool's base vault (the unsold curve supply), `pool` = the DAMM v2 pool's vault, `creator` = the pool's creator.
`holders` (the count) excludes the `curve` and `pool` vaults. Refreshed every 3 minutes per active coin (every 30 min for a coin
with no trade in a day).

### 2.9 `GET /v1/rook/activity?limit=50`
```ts
type ActivityItem = { at: number | null; kind: "buy" | "sell" | "launch" | "graduated";
  coin: { address: string; symbol: string; name: string; logoUrl: string | null; collection: string };
  text: string; solLamports: string | null; trader: string | null; tx: string | null };
type Activity = { source: Source; at: number; items: ActivityItem[] };
```

### 2.10 `GET /v1/rook/collections`
```ts
type CollectionStat = { slug: string; listingId: number | null; config: string; coins: number; graduated: number;
  volume24hLamports: string; volume24hSol: number; marketCapSol: number; topCoin: { mint: string; symbol: string; marketCapSol: number | null } | null };
type Collections = { source: Source; at: number; solUsd: number | null; items: CollectionStat[] };
```

### 2.11 `GET /v1/rook/wallets/:address`
`{ source, address, at, launches: SolCoin[], trades: (SolTrade & { coin: { address, symbol, name, logoUrl } })[] /* newest 50 */ }`.

## 3. How it reads the chain (summary; details in the code)

- **Configs**: Rook's DBC configs come from `ROOK_CONFIGS` (env, overrides everything: `slug=Config,slug=Config` or a JSON array of
  `{slug, config, listingId?}`), else the site's `web-rook/src/config/solana.ts` (`SITE_CONFIG_PATH`), else the ceremony's
  `day1-plan.json` (`DAY1_PLAN_PATH`). Before day 1 the site file has empty configs: the service then idles and says so in `/v1/health`.
- **Discovery**: every DBC `VirtualPool` whose `config` is a Rook config (`getProgramAccounts` on DBC, size 424, memcmp at 72), on
  start and every 10 minutes, plus `EvtInitializePool` events seen in the config's transaction stream.
- **Trades**: `getSignaturesForAddress(config)` (DBC's `swap`/`swap2` both take `config` as account 1, so every swap of every pool
  under the config is in that address's history), then `getTransaction(jsonParsed, maxSupportedTransactionVersion 1)` (mainnet carries version-1 transactions since 2026; an RPC that
  refuses `1` makes the indexer fall back to `0`), decoding
  DBC's self-CPI Anchor events `EvtSwap` / `EvtSwap2` (exact-in, partial-fill and exact-out). After graduation, the DAMM v2 pool
  (account 4 of `migration_damm_v2`) is polled the same way and its `EvtSwap2` events are decoded. Backfill on first run, then a
  cursor per address every few seconds. History is applied 100 transactions at a time (`TX_CHUNK`), the newest first, so a
  first run shows today's trades within seconds and fills older ones behind them. 429/5xx: exponential backoff, failover to the second RPC URL.
- **Holders**: `getTokenLargestAccounts(mint)` + owners via `getMultipleAccounts(jsonParsed)`; the count via Helius DAS
  `getTokenAccounts` (when `HELIUS_API_KEY` is set) or `getProgramAccounts(Token program, memcmp mint, dataSlice amount)`.
- **Metadata**: Metaplex metadata PDA (name, symbol, uri) and the uri's JSON (`image`, `description`, socials), https only,
  64 KB cap, 5 s timeout, cached.
- **USD**: SOL/USD from Pyth Hermes every 60 s (CoinGecko as fallback), stored per minute; historical hours for the backfill from
  Pyth Benchmarks.

## 4. Environment

| var | default | |
|---|---|---|
| `SOLANA_RPC_URL` | — (required) | primary RPC (Helius recommended). A secret if it carries a key: set it in Railway's UI. |
| `SOLANA_RPC_URL_2` | `https://api.mainnet-beta.solana.com` | failover RPC |
| `HELIUS_API_KEY` | unset | enables DAS `getTokenAccounts` for exact holder counts |
| `SOL_CLUSTER` | `mainnet-beta` | `devnet` for a rehearsal |
| `ROOK_CONFIGS` | unset | test/devnet override of the config list (see §3) |
| `SITE_CONFIG_PATH` | `../web-rook/src/config/solana.ts` (`/app/web-rook/src/config/solana.ts` in the image) | |
| `DAY1_PLAN_PATH` | unset | |
| `SOL_DB_PATH` | `./data/rook-sol.db` (`/data/rook-sol.db` in the image) | SQLite file on a Railway volume |
| `PORT` | `8788` | |
| `POLL_MS` | `4000` | between polls of an active signature stream (one with a new transaction in the last 10 min); an idle stream is polled every 30 s, a DAMM pool quiet for a day every 2 min |
| `BACKFILL_MAX_TX` | `20000` | per address on first run (older history is left out and `/v1/health` says so) |
| `HOLDERS_EVERY_S` | `180` | holder refresh for a coin traded in the last day (others: 30 min) |
| `TX_CONCURRENCY` | `6` | parallel `getTransaction` calls |
| `TX_CHUNK` | `100` | transactions written per step (backfill and catch-up) |
| `RPC_MIN_GAP_MS` | `0` | spacing between RPC calls (set ~250 on a free endpoint) |
| `TRUST_PROXY` | `1` on Railway | take the client IP from `X-Forwarded-For` for the rate limit |
| `ROOK_DESK_KEYS` | unset | W13: the canister's Solana keys, `K_main` first, comma-separated; their memo-labelled history becomes `/v1/rook/desk/*` |
| `ROOK_CANISTER_ID` | unset | W13: the canister whose `coins_collections` / `books_view` are queried (collection index ↔ wX; the floor the wX/SOL rate falls back to) |
| `IC_HOST` | `https://icp-api.io` | W13: the IC boundary node for those queries |

## 5. Price and amounts (exact rules)

- `solLamports`: what the trader paid on a buy (the input including fee: `EvtSwap2.swap_result.included_fee_input_amount`, or
  `EvtSwap.actual_input_amount`) or received on a sell (`output_amount`, after fee). `tokenRaw`: coins out (buy) or in (sell).
- Where the fee was taken is read from the event: if the included-fee and excluded-fee input amounts differ, the fee came off the
  input (`price = excluded input / output` on a buy); otherwise it came off the output (`price = (output + fees) / input` on a sell,
  `input / (output + fees)` on a buy whose fee is in coins). A v1-only `EvtSwap` falls back to the config's collect_fee_mode
  (Rook's configs: 0, fees in SOL). DBC 0.2.1 emits `EvtSwap` and `EvtSwap2` for one swap; they are one trade.
- DAMM v2: the same from its `EvtSwap2` (fees = claiming + protocol + compounding + referral); `trade_direction` 0 is A→B, and
  whether the coin is token A is read from the pool account (DBC migrates it as A).
- `feeLamports` is set when the fee was in SOL, else null. Every price is (lamports / 1e9) ÷ (raw / 10^decimals).

## 6. Railway

A new service in the existing Railway project, separate from any Robinhood service (which stays stopped). Build context = repo root.

```
cd ~/Documents/Rook && git status --short          # `railway up` ships the folder as it is
railway link                                        # the Rook project
railway add --service indexer-sol
railway volume add --service indexer-sol --mount-path /data
# in the service's Settings → Config-as-code, set the file path to /indexer/railway.sol.json (Dockerfile indexer/Dockerfile.sol,
# healthcheck /v1/health, restart on failure). Or: railway variables --set "RAILWAY_DOCKERFILE_PATH=indexer/Dockerfile.sol"
railway variables --service indexer-sol --set "TRUST_PROXY=1" --set "SOL_CLUSTER=mainnet-beta" --set "SOL_DB_PATH=/data/rook-sol.db"
# SECRETS, in the Railway UI only (never on a command line): SOLANA_RPC_URL (Helius URL with key), HELIUS_API_KEY,
# optionally SOLANA_RPC_URL_2 (a second provider; default is the public api.mainnet-beta endpoint)
railway up --service indexer-sol --ci
railway domain --service indexer-sol
curl -s https://<domain>/v1/health                  # configs: 18, ok: true, lagSeconds small, lastError null
```

- The image copies `web-rook/src/config/solana.ts`, so **after day 1** (when `ops/solana/site-config.mjs` fills the configs) the indexer
  must be redeployed with the same commit as the site; until then it idles (`/v1/health`: `configs: 0`, a note says why). The
  watch patterns in `railway.sol.json` include that file, so a push that changes it rebuilds the service.
- One replica only (SQLite on a volume). If the volume is not writable by `node`, set `RAILWAY_RUN_UID=0`.
- To re-index from scratch: delete `/data/rook-sol.db*` and restart; the backfill reads every config's history again.
- The site: set `VITE_ROOK_INDEXER=https://<domain>` at build time. CORS is open.
- Railway cost: the process idles at ~60–90 MB RAM and well under 0.1 vCPU; Hobby plan usage ≈ $3–6/month plus the volume (<1 GB).

## 7. Costs (RPC)

Calls per day, for C configs (18), A of them active at a time, G graduated pools (g active), N coins on a curve, H coins traded in a day, T trades/day:

| what | calls/day |
|---|---|
| `getSignaturesForAddress`, active stream every 4 s | 21,600 × (A + g) |
| `getSignaturesForAddress`, idle stream every 30 s (DAMM quiet > 1 day: 2 min) | 2,880 × (C − A) + 2,880 or 720 × (G − g) |
| `getTransaction` | ≈ T (+ creations, migrations, fee claims) |
| `getMultipleAccounts` (pools, every 15 s, 100 per call) | 5,760 × ⌈N / 100⌉ |
| holders: `getProgramAccounts` (token program, by mint) or Helius `getTokenAccounts` | 480 × H (every 3 min) + 48 × (others) |
| mints, metadata, `getBlockTime` | < 2,000 |

A launch-week estimate (C = 18, A = 6, G = 20 with g = 5, N = 200, H = 50, T = 20,000): signatures ≈ 238k (active) + 35k + ~20k (idle),
transactions ≈ 25k, pools ≈ 12k, holders ≈ 31k, other ≈ 2k: **≈ 360k calls/day ≈ 11M/month**. In Helius credits (check the current
price page; at the time of writing standard calls are 1 credit, `getProgramAccounts` and DAS calls 10): ≈ 330k + 31k × 10 ≈ 640k/day
≈ **19M credits/month**.

Recommendation: **Helius Business** (100M credits/month) for launch. On **Developer** (10M) set `POLL_MS=10000` and
`HOLDERS_EVERY_S=1800` (≈ 95k + 55k signature calls and ≈ 10k holder scans a day: ≈ 8.5M credits/month, close to the cap), at the price of a chart that is
up to ~10 s behind. The free tier (1M) runs a quiet pre-launch or a devnet rehearsal only. A second
provider for `SOLANA_RPC_URL_2` (QuickNode / Triton free tier) is worth having for failover; the public endpoint is the default fallback
and is heavily rate-limited (≈10 `getTransaction` per 10 s, measured). Later, Helius webhooks or LaserStream on the configs would
replace the polling and cut the first row to zero.

## 8. Operator checklist

1. Create a Helius account and an API key (Business, or Developer with the tuning above). Keep it server-side: this key is the
   indexer's, not the site's (the site's browser key is a separate, domain-restricted key — `web-rook/src/config/solana.ts`).
2. Railway: the service, volume, config path and variables of §6; `SOLANA_RPC_URL=https://mainnet.helius-rpc.com/?api-key=…` and
   `HELIUS_API_KEY=…` as secrets in the UI.
3. Before day 1: deploy and check `/v1/health` says `configs: 0` and a note (idling is correct). Optional rehearsal: set
   `ROOK_CONFIGS=test=<any busy DBC config>` for an hour, look at `/v1/rook/coins?sort=trending`, then remove it and delete the volume's db.
4. After day 1's `site-config.mjs`: commit, redeploy (site and indexer from the same commit), check `configs: 18`, `backfilling` goes
   false within minutes, and `/v1/rook/coins/<ROOK mint>` answers.
5. Set `VITE_ROOK_INDEXER` for the site build.

## 9. Tests and the dry run

- `npm run test:sol` (in `indexer/`): decoding against recorded mainnet transactions in `test/fixtures/sol/` (a DBC sell that
  emits both `EvtSwap` and `EvtSwap2`, buys and sells on output-fee and quote-fee configs, a creation with a first buy that completes
  the curve, a DAMM v2 migration, a DAMM v2 swap, plus the cases `scripts/sol-record.ts` found: exact-out `swap2`, a swap routed through
  another program by CPI), the follower's writes (idempotency, foreign configs ignored, graduation opening a DAMM stream), candle math,
  phase/progress, sorted views, holder labelling, every endpoint's shape, config loading (site file, day-1 plan, env), RPC failover and
  the read-only refusal, and the metadata fetch fence.
- `npm run sol:dry-run -- --config <any DBC config> --max-tx 40`: read-only, in memory, prints what the API would answer.
  `scripts/sol-explore.ts <address>` prints the decoding of an address's recent transactions; `scripts/sol-record.ts` records fixtures.

### Dry-run results (2026-10-10, public RPCs, read-only)

- Config `5qvf3QJ5LDo4z2TaYCHmcptSpHtxu4hJ348Np6oCDPUJ` (a SOL-quoted launchpad config, output-token fee mode), 40 transactions:
  37 curve trades decoded, the pool's migration to DAMM v2 caught in the stream (`9gzkHd…` → pool `J2hnHU…`), the DAMM stream
  opened and 39 pool trades decoded; holders 11 (complete via `getProgramAccounts`), the DAMM vault labelled `pool`
  (owner `HLnpSz9h…`, DAMM v2's pool authority); 1m candles and USD candles from Pyth.
- Config `CYAhfxmZs9dXhcCtyAeRBdTQupZyEWG4ski6wYbtqqF6` (the busiest DBC config in a 300-transaction sample; Token-2022 coins),
  80 transactions: 50 trades, curve progress 0.86, holders 206 (complete), name/symbol/uri from the Token-2022 metadata extension,
  image from the uri's JSON (through a second IPFS gateway when ipfs.io answered 429). Running the full service on it for 90 s:
  1,687 pools discovered by `getProgramAccounts`, the forward stream and the backfill interleaving, every endpoint answering.
- Prices were checked against the pools' own state: every recorded DBC trade's price is within 0.1–3 % of the price implied by
  the event's `next_sqrt_price`, and DAMM v2 trades match the reserves the event reports. (Both test launchpads' DAMM pools had been
  emptied of SOL right after migration, so their post-graduation prices are ~10⁻¹² SOL; that is the pools' real state.)
- `getSignaturesForAddress(config)` returned a swap that another program made through CPI in a v0 transaction (fixture
  `cpi-swap-5V3AS5sLKH`), as the IDL implies. Not yet seen in a fixture: a swap whose config account was loaded from an address
  lookup table (the RPC indexes loaded addresses too, but no such transaction turned up in the sample), and an exact-out `swap2`
  (covered by a synthetic test on the recorded `EvtSwap2` layout).
