import assert from "node:assert/strict";
import type { AddressInfo } from "node:net";
import { test } from "node:test";
import { RateLimit, serve } from "../src/api.js";
import { openDb } from "../src/db.js";
import { Follower } from "../src/follow.js";
import { Live } from "../src/live.js";
import { Views } from "../src/views.js";
import type * as T from "../types.js";
import { clockOf, loadTape, replay } from "./helpers.js";

async function up(opts: { perMinute?: number; burst?: number; trustProxy?: boolean } = {}) {
  const t = loadTape(), db = openDb(":memory:"), rpc = replay(t);
  const f = new Follower({ db, rpc, deployment: t.deployment, confirmations: 0 });
  await f.pass();
  const views = new Views(db, new Live(rpc, t.deployment.ponsFactory), () => clockOf(t));
  const server = serve({ views, follower: () => f.state, confirmations: 0, ...opts }, 0, "127.0.0.1");   // the production path, so its limiter is the one under test
  await new Promise((r) => server.once("listening", r));
  const base = `http://127.0.0.1:${(server.address() as AddressInfo).port}`;
  return { base, close: () => server.close(), get: async <X>(p: string, init?: RequestInit) => { const r = await fetch(base + p, init); return { r, j: (await r.json().catch(() => null)) as X }; } };
}

test("every endpoint answers in its documented shape, from the recorded run", async () => {
  const s = await up();
  try {
    const health = (await s.get<T.Health>("/v1/health")).j;
    assert.deepEqual([health.ok, health.deployment, health.chainId, health.demo, health.lagBlocks, health.version], [true, true, 31337, true, 0, "1"]);
    const coins = (await s.get<T.Page<T.CoinSummary>>("/v1/coins")).j;
    assert.deepEqual([coins.total, coins.items.length, coins.limit, coins.offset], [1, 1, 50, 0]);
    const c = coins.items[0];
    for (const k of ["address", "router", "curve", "creator"] as const) assert.match(c[k], /^0x[0-9a-fA-F]{40}$/);
    for (const k of ["lifetimeFeesWei", "toVaultWei", "toTreasuryWei", "fees24hWei", "fees7dWei"] as const) assert.match(c[k], /^\d+$/, `${k} is wei as a decimal string`);
    const coin = (await s.get<T.CoinDetail>(`/v1/coins/${c.address.toLowerCase()}`)).j;
    assert.equal(coin.address, c.address, "an address is accepted in any case and returned checksummed");
    assert.ok(coin.volumeUntilNextBuy.assumptions.length >= 4 && coin.volumeUntilNextBuy.inputs.vaultShareBps === 8000);
    const listings = (await s.get<T.Page<T.ListingSummary>>("/v1/listings")).j;
    assert.deepEqual(listings.items.map((l) => l.kind), ["inscriptions", "rune"]);
    const l = (await s.get<T.ListingDetail>("/v1/listings/0")).j;
    assert.ok(l.live && l.declarations.length === 1 && l.acquisitionsRecent.length === 1 && l.coinsList.length === 1);
    const acq = (await s.get<T.Page<T.Acquisition>>("/v1/acquisitions?listing=0")).j.items[0];
    assert.match(acq.btcTxid, /^[0-9a-f]{64}$/);
    assert.ok(acq.verify.links.every((x) => /^https:\/\/(mempool\.space|ordinals\.com)\//.test(x.url)), "links for the reader — to public explorers, never to a venue's API");
    assert.equal((await s.get<T.Page<T.Acquisition>>("/v1/acquisitions?listing=1")).j.total, 0);
    const draws = (await s.get<T.Page<T.DrawSummary>>("/v1/draws")).j;
    assert.deepEqual(draws.items.map((d) => [d.id, d.type, d.state]), [["0-0", "draw", "delivered"]]);
    assert.equal((await s.get<T.DrawDetail>("/v1/draws/0-0")).j.timeline.length, 8);
    assert.equal((await s.get<T.Stats>("/v1/stats")).j.coins, 1);
    assert.ok(((await s.get<{ endpoints: string[] }>("/")).j.endpoints).includes("/v1/health"));
  } finally { s.close(); }
});

test("read-only, and says so: 404, 405, CORS for anyone, short caching on data and none on health, no cookie ever", async () => {
  const s = await up();
  try {
    for (const p of ["/v1/nope", "/v1/coins/0x1234", `/v1/coins/0x${"00".repeat(20)}`, "/v1/listings/99", "/v1/draws/9-9", "/v1/draws/abc"]) assert.equal((await s.get(p)).r.status, 404, p);
    for (const method of ["POST", "PUT", "DELETE", "PATCH"]) {
      const { r, j } = await s.get<T.ApiError>("/v1/coins", { method, body: method === "DELETE" ? undefined : "{}" });
      assert.deepEqual([r.status, r.headers.get("allow"), j.status], [405, "GET, HEAD, OPTIONS", 405]);
    }
    const ok = (await s.get("/v1/coins")).r;
    assert.deepEqual([ok.headers.get("access-control-allow-origin"), ok.headers.get("cache-control"), ok.headers.get("content-type"), ok.headers.get("set-cookie")],
      ["*", "public, max-age=5", "application/json; charset=utf-8", null]);
    assert.equal((await s.get("/v1/health")).r.headers.get("cache-control"), "no-store");
    assert.equal((await s.get("/v1/nope")).r.headers.get("cache-control"), "no-store");
    const pre = await fetch(s.base + "/v1/coins", { method: "OPTIONS" });
    assert.deepEqual([pre.status, pre.headers.get("access-control-allow-methods")], [204, "GET, HEAD, OPTIONS"]);
    const head = await fetch(s.base + "/v1/stats", { method: "HEAD" });
    assert.deepEqual([head.status, (await head.text()).length], [200, 0]);
    assert.equal((await s.get("/v1/coins?limit=100000&offset=-5")).j && ((await s.get<T.Page<T.CoinSummary>>("/v1/coins?limit=100000&offset=-5")).j.limit), 200, "limits are clamped, not trusted");
  } finally { s.close(); }
});

test("rate limited per client: a burst, then 429 with Retry-After; X-Forwarded-For is believed only when told a proxy sets it", async () => {
  const s = await up({ perMinute: 60, burst: 3 });
  try {
    const codes: number[] = [];
    for (let i = 0; i < 5; i++) codes.push((await s.get("/v1/stats", { headers: { "x-forwarded-for": `10.0.0.${i}` } })).r.status);
    assert.deepEqual(codes, [200, 200, 200, 429, 429], "a spoofed header buys nothing");
    assert.ok(Number((await s.get("/v1/stats")).r.headers.get("retry-after")) >= 1);
  } finally { s.close(); }
  const p = await up({ perMinute: 60, burst: 1, trustProxy: true });
  try {
    assert.deepEqual([(await p.get("/v1/stats", { headers: { "x-forwarded-for": "1.1.1.1, 9.9.9.9" } })).r.status, (await p.get("/v1/stats", { headers: { "x-forwarded-for": "2.2.2.2" } })).r.status,
      (await p.get("/v1/stats", { headers: { "x-forwarded-for": "1.1.1.1" } })).r.status], [200, 200, 429]);
  } finally { p.close(); }
  let t = 0; const rl = new RateLimit(60, 2, () => t);
  assert.deepEqual([rl.take("a"), rl.take("a"), rl.take("a") > 0], [0, 0, true]);
  t += 1_000; assert.equal(rl.take("a"), 0, "a token a second at 60 a minute");
});
