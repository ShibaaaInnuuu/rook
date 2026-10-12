// The follower's writes (no RPC: decoded fixtures fed to SolIndexer.write), the candle math, holder labelling, the API's shapes,
// config loading and the RPC failover.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { decodeTx, type ParsedTx } from "../src/sol/decode.js";
import { openSolDb } from "../src/sol/db.js";
import { SolIndexer } from "../src/sol/indexer.js";
import { SolService, sortRows, phaseOf, progressOf, type SolCoin } from "../src/sol/service.js";
import { route } from "../src/sol/api.js";
import { label, DBC_POOL_AUTHORITY } from "../src/sol/holders.js";
import { clean, loadConfigs, parseDay1Plan, parseEnvConfigs, parseSiteConfig } from "../src/sol/configs.js";
import { solRpc, SolRpcError } from "../src/sol/rpc.js";
import { fetchable, gatewayUrl, parseOffChain } from "../src/sol/offchain.js";
import { storeRate } from "../src/sol/usd.js";
import { candles } from "../src/candles.js";

const fx = (name: string) => JSON.parse(readFileSync(new URL(`./fixtures/sol/${name}.json`, import.meta.url), "utf8")) as ParsedTx;
const CONFIG = "9q4yEJsVqUQ4SQp24o8moARNrBFFCm1vZDKLo3KPkqTe", MINT = "8gTju7eND1D8pm52664toFE6gHZKMSGA8jnM9PvQoJkp";
const DAMM = "F9xaKn1zZCaa1NTiC96VKTfmjnd6Vx2e1FrJqbedVm92", POOL = "GYJrkcixUR1o4wG96JYUo2vQdjskohkqgZewM1arBDNL";
const noRpc = { call: async () => { throw new Error("no RPC in this test"); }, state: () => [] };

function world() {
  const db = openSolDb(":memory:");
  db.prepare(`INSERT INTO configs (config, slug, listing_id, quote_mint, fee_mode, decimals, threshold) VALUES (?, 'milady', 0, 'So11111111111111111111111111111111111111112', 1, 6, '7536904')`).run(CONFIG);
  const ix = new SolIndexer({ db, rpc: noRpc, configs: [{ slug: "milady", listingId: 0, config: CONFIG }], http: false });
  const create = decodeTx(fx("dbc-swap-5LAUFFX9zK")), migrate = decodeTx(fx("migrate-damm-v2-FQgKq1qWYL")), damm = decodeTx(fx("damm-swap-23RE4dZAqF"));
  ix.write({ address: CONFIG, kind: "config", mint: null }, [{ d: create, txIndex: 5 }, { d: migrate, txIndex: 9 }]);
  db.prepare(`UPDATE coins SET coin_is_a = 1, supply = '1000000000000000' WHERE mint = ?`).run(MINT);
  ix.write({ address: DAMM, kind: "damm", mint: MINT }, [{ d: damm, txIndex: 2 }]);
  return { db, ix, create, migrate, damm };
}

test("write: creation, first buy, completion, migration and the pool's first swap make one graduated coin with two trades", () => {
  const { db, create, damm } = world();
  const c = db.prepare(`SELECT * FROM coins WHERE mint = ?`).get(MINT) as Record<string, unknown>;
  assert.equal(c.pool, POOL); assert.equal(c.name, "Goon Index 6900"); assert.equal(c.symbol, "GI6900");
  assert.equal(c.creator, "8HMn6b9Vt9mp2kRGhxgvJMDTaZLPMzVdpKqsbfYLaFPM"); assert.equal(c.created_at, create.blockTime);
  assert.equal(c.damm_pool, DAMM); assert.equal(c.is_migrated, 1); assert.ok(c.completed_at);
  assert.equal(c.trades_n, 2); assert.equal(c.last_trade_at, damm.blockTime);
  const s = db.prepare(`SELECT * FROM streams ORDER BY kind`).all() as { address: string; kind: string; mint: string | null }[];
  assert.deepEqual(s.map((x) => [x.kind, x.address]), [["damm", DAMM]], "the migration opened a stream for the DAMM pool");
  const t = db.prepare(`SELECT venue, side FROM trades ORDER BY ord`).all() as { venue: string; side: string }[];
  assert.deepEqual(t.map((x) => `${x.venue}:${x.side}`), ["curve:buy", "pool:buy"]);
});

test("write is idempotent: the same page twice adds nothing", () => {
  const { db, ix, create } = world();
  const n = ix.write({ address: CONFIG, kind: "config", mint: null }, [{ d: create, txIndex: 5 }]);
  assert.equal(n, 0);
  assert.equal((db.prepare(`SELECT count(*) AS n FROM trades`).get() as { n: number }).n, 2);
  assert.equal((db.prepare(`SELECT trades_n FROM coins`).get() as { trades_n: number }).trades_n, 2);
});

test("write ignores swaps on configs that are not Rook's", () => {
  const db = openSolDb(":memory:");
  const ix = new SolIndexer({ db, rpc: noRpc, configs: [{ slug: "milady", listingId: 0, config: "11111111111111111111111111111111" }], http: false });
  assert.equal(ix.write({ address: CONFIG, kind: "config", mint: null }, [{ d: decodeTx(fx("dbc-swap-45hYJ2Vc5o")), txIndex: 0 }]), 0);
  assert.equal((db.prepare(`SELECT count(*) AS n FROM coins`).get() as { n: number }).n, 0);
});

test("the API: coin list, coin, chart, trades, holders, activity, collections, wallet — the contract's shapes", () => {
  const { db, create, damm } = world();
  storeRate(db, Math.floor(damm.blockTime! / 60) * 60, 200, "pyth");
  storeRate(db, Math.floor(create.blockTime! / 60) * 60, 200, "pyth");
  const svc = new SolService(db, { now: () => damm.blockTime! + 30, ttlS: 0 });
  const health = () => ({ ok: true });
  const get = (p: string) => { const u = new URL(p, "http://x"); return route({ svc, health }, u.pathname, u); };

  const list = get("/v1/rook/coins?sort=graduated");
  assert.equal(list.status, 200);
  const L = list.body as { items: SolCoin[]; total: number; sort: string };
  assert.equal(L.sort, "graduated"); assert.equal(L.total, 1);
  const r = L.items[0];
  for (const k of ["kind", "address", "mint", "pool", "dammPool", "config", "collection", "nest", "name", "symbol", "logoUrl", "description", "links", "metadataUri", "creator", "decimals",
    "supplyRaw", "phase", "graduated", "launchedAt", "graduatedAt", "lastTradeAt", "curve", "raise", "price", "priceUsd", "change24h", "marketCapSol", "marketCapUsd",
    "volume24hLamports", "volume24hSol", "volume24hUsd", "trades24h", "buys24h", "sells24h", "holders", "holdersAt"]) assert.ok(k in r, `SolCoin.${k}`);
  assert.equal(r.phase, "graduated"); assert.equal(r.graduated, true); assert.equal(r.curve, null); assert.equal(r.collection.slug, "milady");
  assert.equal(r.trades24h, 2); assert.equal(r.buys24h, 2);
  assert.equal(r.volume24hLamports, String(7536904 + 100000));
  assert.ok(r.price! > 0); assert.equal(r.marketCapSol, r.price! * 1e9); assert.equal(r.priceUsd, r.price! * 200);
  assert.ok(r.change24h !== null, "two prices in the window: a change");

  assert.equal((get("/v1/rook/coins?sort=graduating").body as { total: number }).total, 0);
  assert.equal((get("/v1/rook/coins?collection=azuki").body as { total: number }).total, 0);
  assert.equal((get("/v1/rook/coins?q=gi69").body as { total: number }).total, 1);
  assert.equal(get("/v1/rook/coins?collection=../x").status, 400);

  const one = get(`/v1/rook/coins/${MINT}`).body as SolCoin & { tradesAllTime: number; graduation: { pool: string; tx: string } };
  assert.equal(one.tradesAllTime, 2); assert.equal(one.graduation.pool, DAMM);
  assert.equal(get("/v1/rook/coins/So11111111111111111111111111111111111111112").status, 404);

  const ch = get(`/v1/rook/coins/${MINT}/chart?res=1h`).body as { candles: { t: number; o: number; c: number; n: number; v: number }[]; usd: unknown[] | null; quote: string; seconds: number; span: number; graduation: unknown };
  assert.equal(ch.quote, "SOL"); assert.equal(ch.seconds, 3600); assert.equal(ch.span, 240 * 3600);
  assert.equal(ch.candles.reduce((a, c) => a + c.n, 0), 2); assert.ok(ch.usd && ch.usd.length === ch.candles.length); assert.ok(ch.graduation);
  assert.equal(get(`/v1/rook/coins/${MINT}/chart?res=2m`).status, 400);

  const tp = get(`/v1/rook/coins/${MINT}/trades?limit=1`).body as { items: { venue: string; signature: string; priceUsd: number | null; id: string }[]; next: string | null; total: number };
  assert.equal(tp.items.length, 1); assert.equal(tp.items[0].venue, "pool", "newest first"); assert.equal(tp.total, 2); assert.ok(tp.next);
  assert.equal(tp.items[0].id, `${tp.items[0].signature}:0`); assert.ok(tp.items[0].priceUsd! > 0);
  const tp2 = get(`/v1/rook/coins/${MINT}/trades?limit=1&before=${tp.next}`).body as { items: { venue: string }[]; next: string | null };
  assert.equal(tp2.items[0].venue, "curve"); assert.equal(tp2.next, null);
  assert.equal(get(`/v1/rook/coins/${MINT}/trades?before=abc`).status, 400);

  const h = get(`/v1/rook/coins/${MINT}/holders`).body as { holders: number | null; items: unknown[]; complete: boolean };
  assert.equal(h.holders, null); assert.deepEqual(h.items, []); assert.equal(h.complete, false);

  const act = get("/v1/rook/activity?limit=10").body as { items: { kind: string }[] };
  assert.deepEqual(act.items.map((x) => x.kind).sort(), ["buy", "buy", "graduated", "launch"]);
  const cols = get("/v1/rook/collections").body as { items: { slug: string; coins: number; graduated: number }[] };
  assert.deepEqual(cols.items.map((c) => [c.slug, c.coins, c.graduated]), [["milady", 1, 1]]);
  const w = get("/v1/rook/wallets/8HMn6b9Vt9mp2kRGhxgvJMDTaZLPMzVdpKqsbfYLaFPM").body as { launches: unknown[]; trades: unknown[] };
  assert.equal(w.launches.length, 1); assert.equal(w.trades.length, 1);
  assert.equal(get("/v1/rook/nothing").status, 404);
  assert.deepEqual(get("/v1/health").body, { ok: true });
});

test("candles: buckets by start time, open at the previous close, empty buckets left out", () => {
  const cs = candles([{ ts: 60, p: 1, v: 1 }, { ts: 100, p: 3, v: 2 }, { ts: 119, p: 2, v: 1 }, { ts: 300, p: 4, v: 1 }], 60);
  assert.deepEqual(cs, [
    { t: 60, o: 1, h: 3, l: 1, c: 2, v: 4, n: 3 },
    { t: 300, o: 2, h: 4, l: 2, c: 4, v: 1, n: 1 },
  ]);
});

test("phase and progress", () => {
  const base = { damm_pool: null, is_migrated: 0, completed_at: null, quote_reserve: "42500000000", threshold: "85000000000" };
  assert.equal(phaseOf(base), "trading"); assert.equal(progressOf(base.quote_reserve, base.threshold), 0.5);
  assert.equal(phaseOf({ ...base, quote_reserve: "85000000001" }), "graduating");
  assert.equal(phaseOf({ ...base, completed_at: 1 }), "graduating");
  assert.equal(phaseOf({ ...base, damm_pool: "x" }), "graduated");
  assert.equal(progressOf("100000000000", "85000000000"), 1); assert.equal(progressOf(null, "1"), 0);
});

test("sorted views", () => {
  const mk = (mint: string, o: Partial<SolCoin>) => ({ mint, launchedAt: 0, graduated: false, graduatedAt: null, curve: { progress: 0 }, volume24hSol: 0, trades24h: 0, marketCapSol: null, lastTradeAt: null, ...o }) as SolCoin;
  const xs = [mk("a", { launchedAt: 3, curve: { progress: 0.2 } as SolCoin["curve"], volume24hSol: 5 }), mk("b", { launchedAt: 2, curve: { progress: 0.9 } as SolCoin["curve"], volume24hSol: 1 }),
    mk("c", { launchedAt: 1, graduated: true, graduatedAt: 10, curve: null, volume24hSol: 9, marketCapSol: 100 })];
  assert.deepEqual(sortRows(xs, "new").map((r) => r.mint), ["a", "b", "c"]);
  assert.deepEqual(sortRows(xs, "graduating").map((r) => r.mint), ["b", "a"]);
  assert.deepEqual(sortRows(xs, "graduated").map((r) => r.mint), ["c"]);
  assert.deepEqual(sortRows(xs, "trending").map((r) => r.mint), ["c", "a", "b"]);
  assert.deepEqual(sortRows(xs, "mcap").map((r) => r.mint)[0], "c");
});

test("holders: vaults labelled and left out of the count, a wallet's accounts summed, the creator labelled", () => {
  const s = label([
    { tokenAccount: "VAULT", owner: DBC_POOL_AUTHORITY, amount: 800n },
    { tokenAccount: "DV", owner: "SomePoolAuth", amount: 50n },
    { tokenAccount: "t1", owner: "alice", amount: 30n }, { tokenAccount: "t2", owner: "alice", amount: 15n },
    { tokenAccount: "t3", owner: "creator", amount: 40n }, { tokenAccount: "t4", owner: "bob", amount: 0n },
  ], { creator: "creator", curveVaults: ["VAULT"], poolVaults: ["DV"] }, true);
  assert.equal(s.count, 2);
  assert.deepEqual(s.items.map((x) => [x.address, String(x.amount), x.label]), [[DBC_POOL_AUTHORITY, "800", "curve"], ["SomePoolAuth", "50", "pool"], ["alice", "45", null], ["creator", "40", "creator"]]);
  assert.equal(label([{ tokenAccount: "t1", owner: "a", amount: 1n }], { creator: null, curveVaults: [], poolVaults: [] }, false).count, null, "top 20 only: no count");
});

test("configs: the site file (placeholders = none), the day-1 plan (17 collections + ROOK), the env override", () => {
  const site = readFileSync(new URL("../../web-rook/src/config/solana.ts", import.meta.url), "utf8");
  const raw = parseSiteConfig(site);
  assert.ok(raw.length >= 13, "every collection and ROOK are read");
  assert.equal(raw.at(-1)!.slug, "rook");
  const plan = parseDay1Plan(readFileSync(new URL("../../web-rook/test/fixtures/sol/day1-plan.rehearsal.json", import.meta.url), "utf8"));
  const c = clean(plan);
  assert.equal(c.problems.length, 0); assert.equal(c.configs.at(-1)!.slug, "rook"); assert.equal(c.configs[0].slug, "milady"); assert.equal(c.configs[0].listingId, 0);
  assert.deepEqual(parseEnvConfigs("a=9q4yEJsVqUQ4SQp24o8moARNrBFFCm1vZDKLo3KPkqTe, b=5qvf3QJ5LDo4z2TaYCHmcptSpHtxu4hJ348Np6oCDPUJ").map((x) => x.slug), ["a", "b"]);
  assert.equal(parseEnvConfigs(`[{"slug":"x","config":"${CONFIG}","listingId":3}]`)[0].listingId, 3);
  assert.deepEqual(clean([{ slug: "a", listingId: null, config: "nope" }, { slug: "b", listingId: null, config: "" }]).problems.length, 1);
  const l = loadConfigs({ ROOK_CONFIGS: `t=${CONFIG}` }, { sitePath: "/nonexistent" });
  assert.equal(l.configs.length, 1); assert.equal(l.source, "env ROOK_CONFIGS");
});

test("rpc: refuses anything that is not a read; fails over on 429 to the second URL", async () => {
  const hits: string[] = [];
  const f = (async (url: string) => {
    hits.push(url);
    if (url === "https://a") return new Response("", { status: 429 });
    return new Response(JSON.stringify({ jsonrpc: "2.0", id: 1, result: 42 }), { status: 200 });
  }) as typeof fetch;
  const rpc = solRpc(["https://a", "https://b"], { fetch: f, sleep: async () => {} });
  assert.equal(await rpc.call<number>("getSlot", []), 42);
  assert.deepEqual(hits, ["https://a", "https://b"]);
  assert.equal(rpc.state()[0].ok, false); assert.match(rpc.state()[0].lastError!, /429/);
  await assert.rejects(rpc.call("sendTransaction" as never, []), (e: unknown) => e instanceof SolRpcError && e.kind === "refused");
  // a plain bad request is not retried
  const g = (async () => new Response(JSON.stringify({ jsonrpc: "2.0", id: 1, error: { code: -32602, message: "Invalid param" } }))) as unknown as typeof fetch;
  await assert.rejects(solRpc(["https://c"], { fetch: g }).call("getSlot", []), /Invalid param/);
});

test("off-chain metadata: https / ipfs / ar only, no private hosts, fields trimmed", () => {
  assert.equal(gatewayUrl("ipfs://bafyabc/1.json"), "https://ipfs.io/ipfs/bafyabc/1.json");
  assert.equal(gatewayUrl("ar://xyz"), "https://arweave.net/xyz");
  assert.equal(gatewayUrl("http://x.com/a"), null);
  assert.equal(fetchable("https://metadata.j7tracker.io/m.json"), true);
  for (const bad of ["https://127.0.0.1/x", "https://localhost/x", "https://db.railway.internal/x", "https://[::1]/x", "http://example.com", "https://user:pw@example.com/"]) assert.equal(fetchable(bad), false, bad);
  const o = parseOffChain({ image: "ipfs://Qm1", description: " hi ", extensions: { twitter: "https://x.com/a" }, website: "javascript:alert(1)" });
  assert.deepEqual(o, { image: "https://ipfs.io/ipfs/Qm1", description: "hi", website: null, twitter: "https://x.com/a", telegram: null });
});
