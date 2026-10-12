import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { serve } from "./api.js";
import { openDb } from "./db.js";
import { Follower, loadDeployment } from "./follow.js";
import { Live } from "./live.js";
import { BACKOFF_MS, rpcOf } from "./rpc.js";
import { rateOf, USD_POOL } from "./usd.js";
import { Views } from "./views.js";
import { loadConfig } from "./wrapped/config.js";
import { WrappedService } from "./wrapped/service.js";
import { CatalogService } from "./rook/catalog.js";
import { rookRoutes } from "./rook/routes.js";
import { RookService } from "./rook/service.js";
import { v8FromEnv } from "./v8/routes.js";

/**
 * docs/phase4.md. One process: the follower and the read-only API over its SQLite file.
 *   RPC_URL            the chain to follow (default http://127.0.0.1:8545)
 *   DEPLOYMENTS_DIR    where <chainid>.json lives (default ../deployments)
 *   DB_PATH            the SQLite file (default ./data/etch.db)
 *   PORT               the API's port (default 8787)
 *   CONFIRMATIONS      blocks behind the head (default 20; 0 on chain 31337)
 *   POLL_MS            between passes (default 4000)
 *   TRUST_PROXY=1      take the client address from X-Forwarded-For (only behind a proxy that sets it)
 *   DEPLOYMENT_FROM_RPC=1  docs/phase6.md: for a chain whose deployment is made at its first boot (our staging fork), ask the RPC's
 *                      OWN host for /info. Still one host and no other. Waited for; written beside the database as <chainid>.json.
 *   DEPLOYMENT_LABEL   "test" reads test-<chainid>.json (docs/phase6.md, tier 2): a small real test is never read as THE deployment
 *   USD_POOL           docs/phase12.md §3: the Uniswap v3 pool dollars per ETH is read from, every few minutes. Defaults to
 *                      Robinhood Chain's WETH/USDG pool. Set it to "none" on a chain that has no such pool: the chart then
 *                      plots the pair asset instead of dollars, which is the honest fallback, not a degraded one.
 *   V8_CONFIG          docs/v8: the Ethereum collections whose proof of reserves and draws /v1/v8/* serves (src/v8/service.ts);
 *                      CANISTER_RECORDS_URL, the canister's own records by slug, labelled on the page (P4). Unset: not served.
 *   WRAPPED_CONFIG     docs/v5/ux-and-reserves.md: a JSON file naming the wrapped collections to report on (wrappers, custody,
 *                      providers). Unset: /v1/wrapped and /v1/reserves answer `configured: false`.
 *   OPENSEA_API_KEY    docs/v6/collections-catalog.md: read at the moment of each OpenSea call by the collections catalogue,
 *                      never at load and never logged. Unset: OpenSea is asked keyless, which answers with intermittent 401s.
 *   CATALOG_TTL_S      seconds between the catalogue's refreshes of the marketplace sources (default 600; 300–900 is the range)
 * No key is read, because there is no key: this package cannot sign.
 */
/** THE one constant in this package (its image is built without staging/): test/guards.test.ts compares it with staging/chain-id.json. */
export const STAGING_CHAIN_ID = 9466301;
const here = dirname(fileURLToPath(import.meta.url));
const env = process.env;
const log = (o: Record<string, unknown>) => console.log(JSON.stringify({ at: new Date().toISOString(), ...o }));
const url = env.RPC_URL ?? "http://127.0.0.1:8545";
const dbPath = resolve(env.DB_PATH ?? resolve(here, "../data/etch.db"));
mkdirSync(dirname(dbPath), { recursive: true });
const db = openDb(dbPath);
const rpc = rpcOf(url);

let chainId: number | null = null;
try { chainId = Number(BigInt(await rpc<string>("eth_chainId", []))); } catch (err) { log({ level: "error", event: "no-chain", message: String((err as Error).message) }); }
let deploymentsDir = resolve(env.DEPLOYMENTS_DIR ?? resolve(here, "../../deployments"));
if (chainId !== null && env.DEPLOYMENT_FROM_RPC === "1") {
  const infoUrl = new URL("/info", url);      // the same host as the RPC, by construction: test/guards.test.ts holds this line to that
  deploymentsDir = dirname(dbPath);
  for (let i = 0; ; i++) {
    try {
      const j = (await (await fetch(infoUrl)).json()) as { deployment?: Record<string, unknown> | null } & Record<string, unknown>;
      const d = (j.deployment !== undefined ? j.deployment : j) as Record<string, unknown> | null;
      if (d && Number(d.chainId) === chainId) { writeFileSync(resolve(deploymentsDir, `${chainId}.json`), JSON.stringify(d, null, 2) + "\n"); break; }
    } catch { /* not up yet */ }
    if (i % 10 === 0) log({ level: "info", event: "waiting-for-deployment", from: infoUrl.host });
    await new Promise((r) => setTimeout(r, 6_000));
  }
}
if (chainId !== null && env.DEPLOYMENT_LABEL === "test") {
  // loadDeployment reads <chainid>.json: a labelled test is copied beside the database under that name, and only if its label says test
  const src = resolve(deploymentsDir, `test-${chainId}.json`);
  deploymentsDir = dirname(dbPath);
  try { const j = JSON.parse(readFileSync(src, "utf8")) as { label?: string }; if (j.label === "test") writeFileSync(resolve(deploymentsDir, `${chainId}.json`), JSON.stringify(j, null, 2) + "\n"); } catch { /* none: it idles */ }
}
const deployment = chainId === null ? null : loadDeployment(deploymentsDir, chainId);
const confirmations = env.CONFIRMATIONS !== undefined ? Math.max(0, Number(env.CONFIRMATIONS) | 0) : chainId === 31337 || chainId === STAGING_CHAIN_ID ? 0 : 20;
const usdPool = env.USD_POOL === undefined ? USD_POOL : /^0x[0-9a-fA-F]{40}$/.test(env.USD_POOL) ? env.USD_POOL.toLowerCase() : null;
const follower = new Follower({ db, rpc, deployment, confirmations, log, usdPool });
const views = new Views(db, new Live(deployment ? rpc : null, deployment?.ponsFactory ?? null));
const port = Number(env.PORT ?? 8787);
let wrapped: WrappedService | null = null, wrappedConfig = null as ReturnType<typeof loadConfig> | null;
if (env.WRAPPED_CONFIG) {
  try {
    const config = loadConfig(resolve(env.WRAPPED_CONFIG));
    if (config.chainId !== chainId) throw new Error(`the wrapped config is for chain ${config.chainId} and the node is on ${chainId}`);
    wrapped = new WrappedService({ config, rpc, fetch, log }); wrappedConfig = config;
  }
  catch (err) { log({ level: "error", event: "wrapped-config", message: String((err as Error).message) }); }
}
// docs/v6/site-design.md: the Rook board over the same database and the wrapped collections (src/rook/, another agent's files are untouched)
const catalogTtl = Math.min(900, Math.max(300, Number(env.CATALOG_TTL_S ?? 600) || 600));
const catalog = new CatalogService({ wrapped, wrappedConfig, ttlS: catalogTtl, log, usdPerEth: () => rateOf(db, Math.floor(Date.now() / 1000)).usdPerEth });
const board = rookRoutes(new RookService({ db, views, wrapped, wrappedConfig, rpc: chainId === null ? null : rpc, log }), catalog);
// docs/v8: the Ethereum side's proof of reserves and the redemption draws (src/v8/routes.ts), composed with the board's routes
let v8: ReturnType<typeof v8FromEnv> = null;
try { v8 = v8FromEnv(env, log); } catch (err) { log({ level: "error", event: "v8-config", message: String((err as Error).message) }); }
const rook: typeof board = v8 ? async (p, url) => (await v8!.routes(p, url)) ?? board(p, url) : board;
serve({ views, wrapped, rook, follower: () => follower.state, confirmations, trustProxy: env.TRUST_PROXY === "1", log }, port);
log({ level: "info", event: "start", port, chainId, deployment: deployment !== null, confirmations, usdPool,
  note: deployment ? `following from block ${deployment.fromBlock}` : "no deployment file for this chain: idling, and every list is empty" });

let failures = 0, stop = false;
for (const s of ["SIGINT", "SIGTERM"] as const) process.on(s, () => { stop = true; db.close(); process.exit(0); });
while (!stop) {
  try { const n = await follower.pass(); failures = 0; if (n) log({ level: "info", event: "indexed", events: n, cursor: follower.cursor() }); }
  catch { failures++; }
  await new Promise((r) => setTimeout(r, failures ? BACKOFF_MS[Math.min(failures - 1, BACKOFF_MS.length - 1)] : Number(env.POLL_MS ?? 4_000)));
}
