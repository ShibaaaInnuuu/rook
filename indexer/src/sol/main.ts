/**
 * Rook's Solana indexer: the follower and the read-only API in one process (docs/v9/indexer-solana.md).
 *   npm run sol                       serve (env below)
 *   npm run sol -- --dry-run --config <DBC config> [--max-tx 200] [--no-http]
 *                                     read-only proof against mainnet: index one page of a config's history into memory, print what
 *                                     the API would answer, exit. Any busy launchpad config works.
 * Env: SOLANA_RPC_URL (required), SOLANA_RPC_URL_2, HELIUS_API_KEY, SOL_CLUSTER, ROOK_CONFIGS, SITE_CONFIG_PATH, DAY1_PLAN_PATH,
 *      SOL_DB_PATH, PORT, POLL_MS, BACKFILL_MAX_TX, HOLDERS_EVERY_S, TX_CONCURRENCY, TX_CHUNK, RPC_MIN_GAP_MS, TRUST_PROXY;
 *      W13: ROOK_DESK_KEYS (the canister's Solana keys, `K_main` first, comma-separated: their memo-labelled history is indexed),
 *      ROOK_CANISTER_ID and IC_HOST (the canister's public queries: the collection index ↔ wX mapping and the accepted floor the
 *      wX/SOL rate falls back to). Without them: no desk events, and a collection with no DLMM pair has no SOL price.
 * No key is read except the RPC's and Helius's, and neither can sign: this process sends nothing (the canister is only queried).
 */
import { mkdirSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { serveSol, SOL_VERSION } from "./api.js";
import { isAddress } from "./bytes.js";
import { icCanister, IC_HOST, type CanisterReader } from "./canister.js";
import { loadConfigs } from "./configs.js";
import { getMeta, openSolDb } from "./db.js";
import { SolIndexer } from "./indexer.js";
import { solRpc } from "./rpc.js";
import { SolService } from "./service.js";
import { latestRate } from "./usd.js";

const here = dirname(fileURLToPath(import.meta.url));
const env = process.env, argv = process.argv.slice(2);
const flag = (k: string) => argv.includes(k);
const opt = (k: string) => { const i = argv.indexOf(k); return i >= 0 ? argv[i + 1] : undefined; };
const log = (o: Record<string, unknown>) => console.log(JSON.stringify({ at: new Date().toISOString(), ...o }));
const dry = flag("--dry-run");

const cluster = env.SOL_CLUSTER ?? "mainnet-beta";
const primary = env.SOLANA_RPC_URL ?? (dry ? "https://api.mainnet-beta.solana.com" : "");
const secondary = env.SOLANA_RPC_URL_2 ?? (cluster === "devnet" ? "https://api.devnet.solana.com" : dry ? "https://solana-rpc.publicnode.com" : "https://api.mainnet-beta.solana.com");
if (!primary) { log({ level: "error", event: "no-rpc", message: "set SOLANA_RPC_URL" }); process.exit(2); }
const rpc = solRpc([primary, secondary !== primary ? secondary : ""].filter(Boolean), { minGapMs: Number(env.RPC_MIN_GAP_MS ?? (dry ? 250 : 0)) });
// W13: the canister's keys to follow for memo-labelled transactions, and the canister to query (both optional; a bad key is dropped, said)
const deskKeys = (env.ROOK_DESK_KEYS ?? "").split(",").map((s) => s.trim()).filter(Boolean);
for (const k of deskKeys.filter((k) => !isAddress(k))) log({ level: "warn", event: "desk-key-invalid", key: k });
const canister: CanisterReader | null = env.ROOK_CANISTER_ID ? await icCanister(env.ROOK_CANISTER_ID, env.IC_HOST ?? IC_HOST) : null;

if (dry) {
  const config = opt("--config") ?? env.ROOK_CONFIGS;
  if (!config) { console.error("--dry-run needs --config <DBC config address>"); process.exit(2); }
  const maxTx = Number(opt("--max-tx") ?? 200);
  const db = openSolDb(":memory:");
  const configs = loadConfigs({ ROOK_CONFIGS: config.includes("=") || config.startsWith("[") ? config : `dryrun=${config}` }, { sitePath: "" });
  const ix = new SolIndexer({ db, rpc, configs: configs.configs, log, heliusKey: env.HELIUS_API_KEY ?? null, cluster, backfillMax: maxTx,
    txConcurrency: Number(env.TX_CONCURRENCY ?? 3), http: !flag("--no-http"), deskKeys: deskKeys.filter(isAddress), canister });
  const t0 = Date.now();
  await ix.init();
  // the head, then the backfill a chunk at a time up to --max-tx
  let trades = 0;
  for (let i = 0; i < 1 + Math.ceil(maxTx / 100); i++) trades += await ix.pollStream(db.prepare(`SELECT * FROM streams WHERE kind = 'config'`).get() as never);
  // graduated coins found in this page: one page of each DAMM v2 pool too
  for (let i = 0; i < 2; i++) for (const d of db.prepare(`SELECT * FROM streams WHERE kind = 'damm' LIMIT 3`).all() as never[]) await ix.pollStream(d).catch((e) => log({ level: "warn", event: "damm-dry", message: String(e) }));
  await ix.refreshPools(); await ix.refreshMeta(); await ix.refreshSupply(); await ix.refreshMeta(); await ix.refreshUsd(); await ix.refreshWx();
  const svc = new SolService(db, { ttlS: 0 });
  const top = svc.list({ sort: "trending", limit: 5, offset: 0 });
  if (top.items[0]) { await ix.refreshHolders(1, top.items[0].mint); }
  const svc2 = new SolService(db, { ttlS: 0 });
  const first = top.items[0]?.mint;
  const out = {
    config, seconds: Math.round((Date.now() - t0) / 100) / 10, rpc: rpc.state().map((e) => ({ name: e.name, calls: e.calls, ok: e.ok, lastError: e.lastError })),
    transactionsRead: (db.prepare(`SELECT seen FROM streams WHERE kind = 'config'`).get() as { seen: number }).seen, tradesDecoded: trades, ...svc2.counts(),
    byKind: db.prepare(`SELECT venue, side, count(*) AS n FROM trades GROUP BY venue, side`).all(),
    graduations: db.prepare(`SELECT mint, damm_pool, graduated_tx FROM coins WHERE damm_pool IS NOT NULL LIMIT 3`).all(),
    trending: svc2.list({ sort: "trending", limit: 5, offset: 0 }).items.map((r) => ({ mint: r.mint, name: r.name, symbol: r.symbol, logoUrl: r.logoUrl, phase: r.phase, progress: r.curve?.progress ?? null,
      price: r.price, marketCapSol: r.marketCapSol, marketCapUsd: r.marketCapUsd, volume24hSol: r.volume24hSol, trades24h: r.trades24h, holders: r.holders, launchedAt: r.launchedAt })),
    chart: first ? (({ candles, usd, ...rest }) => ({ ...rest, candles: candles.slice(-5), usd: usd?.slice(-2) ?? null, candleCount: candles.length }))(svc2.chart(first, "1m")!) : null,
    trades: first ? svc2.trades(first, 3, null)!.items : null,
    holders: first ? (({ items, ...rest }) => ({ ...rest, items: items.slice(0, 6) }))(svc2.holders(first, 20, 0)!) : null,
  };
  console.log(JSON.stringify(out, null, 2));
  process.exit(0);
}

const dbPath = resolve(env.SOL_DB_PATH ?? resolve(here, "../../data/rook-sol.db"));
mkdirSync(dirname(dbPath), { recursive: true });
const db = openSolDb(dbPath);
const cfg = loadConfigs(env, { sitePath: resolve(here, "../../../web-rook/src/config/solana.ts") });
for (const p of cfg.problems) log({ level: "warn", event: "config-problem", message: p });
const ix = new SolIndexer({ db, rpc, configs: cfg.configs, log, heliusKey: env.HELIUS_API_KEY ?? null, cluster,
  backfillMax: Number(env.BACKFILL_MAX_TX ?? 20_000), chunk: Number(env.TX_CHUNK ?? 100), txConcurrency: Number(env.TX_CONCURRENCY ?? 6), holdersEveryS: Number(env.HOLDERS_EVERY_S ?? 180),
  deskKeys: deskKeys.filter(isAddress), canister });
const svc = new SolService(db, { cluster });
const pollMs = Math.max(1_000, Number(env.POLL_MS ?? 4_000));
const health = () => {
  const now = Math.floor(Date.now() / 1000), last = ix.state.lastPollAt ?? (Number(getMeta(db, "last_poll_at") ?? "") || null), rate = latestRate(db, now);
  const truncated = (db.prepare(`SELECT count(*) AS n FROM streams WHERE truncated = 1`).get() as { n: number }).n;
  return { ok: cfg.configs.length > 0 && last !== null && now - last < 120, version: SOL_VERSION, cluster, configs: cfg.configs.length, configsSource: cfg.source, ...svc.counts(),
    lastPollAt: last, lagSeconds: last === null ? null : now - last, backfilling: ix.state.backfilling, historyTruncated: truncated > 0,
    solUsd: rate?.usd ?? null, solUsdAt: rate?.at ?? null, rpc: rpc.state().map((e) => ({ name: e.name, ok: e.ok, lastError: e.lastError, calls: e.calls })), lastError: ix.state.lastError,
    // W13: the desk keys followed and the canister queried (ids only, never a URL with a key), and each wX's latest rate and its source
    deskKeys: deskKeys.filter(isAddress).length, canister: canister?.id ?? null,
    wxSol: svc.collections().items.filter((c) => c.quote.kind === "wx").map((c) => ({ slug: c.slug, sol: c.wxSol?.sol ?? null, at: c.wxSol?.at ?? null, src: c.wxSol?.src ?? null })),
    note: cfg.configs.length ? null : "no Rook config yet (placeholders until day 1): idling" };
};
const port = Number(env.PORT ?? 8788);
serveSol({ svc, health, trustProxy: env.TRUST_PROXY === "1", log }, port);
log({ level: "info", event: "start", port, cluster, configs: cfg.configs.length, configsSource: cfg.source, db: dbPath });

let stop = false, failures = 0;
for (const sgn of ["SIGINT", "SIGTERM"] as const) process.on(sgn, () => { stop = true; db.close(); process.exit(0); });
let ready = false;
while (!stop) {
  if (!cfg.configs.length) { await new Promise((r) => setTimeout(r, 60_000)); continue; }
  try {
    if (!ready) { await ix.init(); ready = true; }
    const n = await ix.pass(pollMs); failures = 0;
    if (n) log({ level: "info", event: "indexed", trades: n });
  } catch (e) { failures++; log({ level: "warn", event: "pass-error", message: String((e as Error).message).slice(0, 200) }); }
  await new Promise((r) => setTimeout(r, failures ? Math.min(60_000, 1_000 * 2 ** failures) : pollMs));
}
