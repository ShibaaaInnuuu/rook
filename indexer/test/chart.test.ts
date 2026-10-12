import assert from "node:assert/strict";
import { test } from "node:test";
import { candles, curvePrice, initialRes, poolPrice, RES, resOf, units } from "../src/candles.js";
import { openDb } from "../src/db.js";
import { apply, rebuild, type Ev } from "../src/fold.js";
import { Live } from "../src/live.js";
import { Views } from "../src/views.js";
import { handler } from "../src/api.js";
import { COIN, CURVE, HOOK, POOL_ID, POOLS, story, T0, TX, WHO, ZERO } from "./story.js";

/** The story, applied and also kept as events, so a rebuild has something to rebuild from. */
const filled = () => {
  const db = openDb(":memory:");
  const ins = db.prepare(`INSERT INTO events (block_number, log_index, tx, address, contract, name, args, ts, listing_id, acquisition_id) VALUES (?, ?, ?, ?, ?, ?, ?, ?, NULL, NULL)`);
  for (const e of story()) { apply(db, e); ins.run(e.block, e.logIndex, e.tx, e.address, e.contract, e.name, JSON.stringify(e.args), e.ts); }
  return db;
};
const viewsOf = (db: ReturnType<typeof openDb>, now = T0 + 60) => new Views(db, new Live(null, null), () => now);

test("candle bucketing: a trade lands in the bucket its second belongs to, the boundary second opens the next one, and a bucket nobody traded in is left out", () => {
  const p = (ts: number, price: number, v = 1) => ({ ts, p: price, v });
  // 300-second buckets: 299 is the last second of the first, 300 the first of the second
  const cs = candles([p(0, 10), p(299, 12), p(300, 9), p(905, 8)], 300);
  assert.deepEqual(cs.map((c) => c.t), [0, 300, 900], "the 600 bucket had no trade and is absent, not a flat candle");
  assert.deepEqual(cs.map((c) => [c.o, c.h, c.l, c.c, c.n]), [[10, 12, 10, 12, 2], [12, 12, 9, 9, 1], [9, 9, 8, 8, 1]],
    "each bucket opens at the last close, so the line is continuous where trading is not");
  assert.deepEqual(cs.map((c) => c.v), [2, 1, 1]);
  // the first candle of all has no previous close, so it opens at its own first price
  assert.deepEqual(candles([p(7, 5)], 300)[0], { t: 0, o: 5, h: 5, l: 5, c: 5, v: 1, n: 1 });
  // res "trade": every trade is its own point at its own second, nothing is merged
  assert.deepEqual(candles([p(10, 1), p(10, 2)], 0).map((c) => [c.t, c.c]), [[10, 1], [10, 2]]);
  assert.deepEqual(candles([], 300), [], "no trades, no candles");
  assert.deepEqual([RES.trade.seconds, RES["5m"].seconds, RES["1h"].seconds], [0, 300, 3_600]);
  assert.deepEqual([resOf("trade"), resOf("1h"), resOf("5m"), resOf("nonsense"), resOf(null)], ["trade", "1h", "5m", "5m", "5m"]);
  assert.deepEqual([initialRes(T0, T0 + 60), initialRes(T0, T0 + 7 * 3_600), initialRes(null, T0)], ["trade", "5m", "5m"]);
});

test("the price is the curve's, not the fee's: a buy and the sell after it are continuous, and the gross prices they hide are not", () => {
  // the shape measured on the staging fork: 1% fee and 3% creator tax, out of the quote leg on both sides
  const q = 1_000_000n, tokens = 1_000_000_000n, fee = 10_000n, tax = 30_000n;
  const buy = curvePrice(true, q, tokens, fee, tax), sell = curvePrice(false, q, tokens, fee, tax);
  assert.equal(buy, 960_000 / 1e9, "a buyer's quote reaches the curve after the fee and the tax come out of it");
  assert.equal(sell, 1_040_000 / 1e9, "a seller is paid after both, so the curve released more than the seller received");
  assert.ok(Math.abs(sell - buy) / buy < 0.09, "the whole gap between them is the 8% of fees, and the chart draws neither end of it");
  assert.equal(curvePrice(true, q, 0n, fee, tax), 0, "a trade that moved no coin has no price to report");
  // a pool swap: the signed legs, with no fee adjustment, because the hook charges outside the Swap event
  assert.equal(poolPrice(1_000n, -2_000n), 0.5);
  assert.equal(poolPrice(-1_000n, 2_000n), 0.5, "the sign says who paid whom, not what the price was");
  assert.equal(poolPrice(1_000n, 0n), 0);
  assert.equal(units("1500000000000000000"), 1.5);
});

test("a coin that has never traded: an empty series, no invented price, and a 404 for a coin that was never launched", () => {
  const db = openDb(":memory:"), v = viewsOf(db);
  apply(db, { block: 100, logIndex: 0, tx: TX, address: "0x" + "22".repeat(20), contract: "launcher", name: "Launched",
    args: { coin: COIN, creator: WHO, listingId: "0", curve: CURVE, router: "0x" + "d0".repeat(20) }, ts: T0 } satisfies Ev);
  const c = v.chart(COIN, { res: "5m" })!;
  assert.deepEqual([c.candles, c.marks, c.trades, c.tradesAllTime, c.graduation, c.pool], [[], [], 0, 0, null, null]);
  assert.deepEqual([c.quote, c.priceIs], ["ETH", "pair asset per whole coin"]);
  assert.deepEqual(v.trades(COIN, 10), { items: [], next: null, total: 0 });
  assert.equal(v.chart("0x" + "ab".repeat(20), { res: "5m" }), null, "a coin this indexer never saw launched has no chart at all");
  assert.equal(v.coinExists("0x" + "ab".repeat(20)), false);
});

test("a single trade: one candle, opening at its own price, and one mark", () => {
  const db = openDb(":memory:");
  const ev = (contract: string, address: string, name: string, args: Ev["args"], block: number, li = 0): Ev =>
    ({ block, logIndex: li, tx: TX, address, contract, name, args, ts: T0 + block });
  apply(db, ev("launcher", "0x" + "22".repeat(20), "Launched", { coin: COIN, creator: WHO, listingId: "0", curve: CURVE, router: "0x" + "d0".repeat(20) }, 1));
  apply(db, ev("curve", CURVE, "CurveBuy", { buyer: WHO, recipient: WHO, quoteIn: "1000", tokensOut: "1000000", fee: "10", tax: "30" }, 2));
  const c = viewsOf(db, T0 + 100).chart(COIN, { res: "5m" })!;
  assert.equal(c.candles.length, 1);
  assert.deepEqual([c.candles[0].o, c.candles[0].h, c.candles[0].l, c.candles[0].c, c.candles[0].n], [0.00096, 0.00096, 0.00096, 0.00096, 1]);
  assert.deepEqual([c.trades, c.marks.length, c.marks[0].venue, c.marks[0].side], [1, 1, "curve", "buy"]);
  assert.equal(c.marks[0].feeWei, "10", "the fee the chain recorded, kept in wei on the trade itself");
});

test("curve to pool: the trades of both venues are one series, graduation is a fact in the data, and the price does not jump at it", () => {
  const db = filled(), v = viewsOf(db, T0 + 600);
  const page = v.trades(COIN, 50);
  assert.deepEqual(page.items.map((t) => `${t.venue}:${t.side}`), ["pool:buy", "curve:sell", "curve:buy"], "newest first, both venues, in chain order");
  const chain = [...page.items].reverse();
  assert.deepEqual([chain[0].quoteWei, chain[0].coinWei, chain[0].feeWei, chain[0].taxWei], ["1000", "1000000", "10", "25"]);
  assert.deepEqual([chain[2].feeWei, chain[2].taxWei], [null, null], "a pool trade's fees are charged by the hook outside the Swap, so they are not invented");
  assert.equal(chain[2].trader, chain[2].trader.toUpperCase().replace("0X", "0x").slice(0, 2) + chain[2].trader.slice(2), "an address, checksummed");
  const lastCurve = chain[1].price, firstPool = chain[2].price;
  assert.ok(Math.abs(firstPool - lastCurve) / lastCurve < 0.05, `the pool opens where the curve stopped: ${lastCurve} then ${firstPool}`);
  const c = v.chart(COIN, { res: "trade" })!;
  assert.equal(c.candles.length, 3, "at res trade every trade is its own point, across the graduation");
  assert.deepEqual(c.candles.map((x) => x.n), [1, 1, 1]);
  assert.ok(c.graduation && c.graduation.block !== null && c.graduation.at !== null, "graduation is in the chart, so a page can mark where the curve ended");
  assert.ok(c.graduation!.at! > c.candles[1].t && c.graduation!.at! < c.candles[2].t, "and it sits between the last curve trade and the first pool trade");
  assert.equal(c.pool, POOL_ID.toLowerCase(), "the pool it graduated into, learnt from the PoolManager's own Initialize");
  const pool = db.prepare(`SELECT * FROM pools`).get() as { coin: string; coin_is_0: number; quote_token: string; hooks: string };
  assert.deepEqual([pool.coin, pool.coin_is_0, pool.quote_token, pool.hooks], [COIN, 0, ZERO, HOOK], "the coin is currency1 here, so the quote is the other leg");
});

test("the cursor pages a coin's trades without repeating or skipping one, whatever lands between pages", () => {
  const db = openDb(":memory:");
  const ev = (name: string, args: Ev["args"], block: number, li: number): Ev => ({ block, logIndex: li, tx: TX, address: CURVE, contract: "curve", name, args, ts: T0 + block });
  apply(db, { block: 1, logIndex: 0, tx: TX, address: "0x" + "22".repeat(20), contract: "launcher", name: "Launched",
    args: { coin: COIN, creator: WHO, listingId: "0", curve: CURVE, router: "0x" + "d0".repeat(20) }, ts: T0 });
  // two trades in the same block, so the cursor has to carry the log index as well as the height
  for (let i = 0; i < 9; i++) apply(db, ev("CurveBuy", { buyer: WHO, recipient: WHO, quoteIn: String(1000 + i), tokensOut: "1000000", fee: "10", tax: "30" }, 2 + Math.floor(i / 2), i % 2));
  const v = viewsOf(db, T0 + 100);
  const seen: string[] = [];
  let next: string | null = null, pages = 0;
  do {
    const page: ReturnType<Views["trades"]> = v.trades(COIN, 2, next);
    assert.ok(page.items.length <= 2);
    seen.push(...page.items.map((t) => t.id));
    next = page.next;
    assert.ok(++pages < 20, "the cursor terminates");
  } while (next);
  assert.equal(seen.length, 9, "every trade once");
  assert.equal(new Set(seen).size, 9, "and none twice");
  assert.deepEqual(seen, [...seen].sort((a, b) => {
    const [ab, al] = a.split(":").map(Number), [bb, bl] = b.split(":").map(Number);
    return bb - ab || bl - al;
  }), "newest first throughout");
  assert.equal(v.trades(COIN, 50).total, 9);
});

test("the API serves both, and the shapes are the documented ones", async () => {
  const db = filled(), v = viewsOf(db, T0 + 600);
  const call = async (path: string) => {
    const res: { status?: number; body?: string; headers?: Record<string, string> } = {};
    const fake = { writeHead(s: number, h: Record<string, string>) { res.status = s; res.headers = h; }, end(b?: string) { res.body = b; } };
    await handler({ views: v, follower: () => ({ lastPassAt: null, lastError: null, head: null, reorgs: 0, idle: false }), confirmations: 0, perMinute: 10_000, burst: 10_000 })(
      { method: "GET", url: path, headers: {}, socket: { remoteAddress: "1.2.3.4" } } as never, fake as never);
    return { status: res.status!, body: res.body ? (JSON.parse(res.body) as Record<string, unknown>) : null };
  };
  const t = await call(`/v1/coins/${COIN}/trades?limit=2`);
  assert.equal(t.status, 200);
  assert.deepEqual(Object.keys(t.body!), ["items", "next", "total"]);
  assert.equal((t.body!.items as unknown[]).length, 2);
  assert.match(t.body!.next as string, /^\d+:\d+$/);
  const c = await call(`/v1/coins/${COIN}/candles?res=1h`);
  assert.equal(c.status, 200);
  assert.equal(c.body!.res, "1h");
  assert.deepEqual(Object.keys((c.body!.candles as Record<string, unknown>[])[0]), ["t", "o", "h", "l", "c", "v", "n"]);
  assert.equal((await call(`/v1/coins/${COIN}/trades?before=nonsense`)).status, 400);
  assert.equal((await call(`/v1/coins/0x${"ab".repeat(20)}/trades`)).status, 404);
  assert.equal((await call(`/v1/coins/0x${"ab".repeat(20)}/candles`)).status, 404);
});

test("trades and pools are derived: rebuilding from the events table gives exactly the same rows", () => {
  const db = filled();
  const before = JSON.stringify([db.prepare(`SELECT * FROM trades ORDER BY block, log_index`).all(), db.prepare(`SELECT * FROM pools`).all(),
    db.prepare(`SELECT graduated_block, graduated_ts, graduated_quote FROM coins ORDER BY coin`).all()]);
  rebuild(db);
  assert.equal(JSON.stringify([db.prepare(`SELECT * FROM trades ORDER BY block, log_index`).all(), db.prepare(`SELECT * FROM pools`).all(),
    db.prepare(`SELECT graduated_block, graduated_ts, graduated_quote FROM coins ORDER BY coin`).all()]), before);
});

test("a curve or a pool that is not ours changes nothing: an unknown curve's trade, and a pool of somebody else's token", () => {
  const db = filled(), before = JSON.stringify(db.prepare(`SELECT * FROM trades ORDER BY block, log_index`).all());
  apply(db, { block: 900, logIndex: 0, tx: TX, address: "0x" + "ee".repeat(20), contract: "curve", name: "CurveBuy",
    args: { buyer: WHO, recipient: WHO, quoteIn: "1", tokensOut: "1", fee: "0", tax: "0" }, ts: T0 });
  apply(db, { block: 901, logIndex: 0, tx: TX, address: POOLS, contract: "pool", name: "Initialize",
    args: { id: "0x" + "ff".repeat(32), currency0: ZERO, currency1: "0x" + "ab".repeat(20), fee: 3000, tickSpacing: 200, hooks: HOOK, sqrtPriceX96: "1", tick: 0 }, ts: T0 });
  apply(db, { block: 902, logIndex: 0, tx: TX, address: POOLS, contract: "pool", name: "Swap",
    args: { id: "0x" + "ff".repeat(32), sender: WHO, amount0: "1", amount1: "-1", sqrtPriceX96: "1", liquidity: "1", tick: 0, fee: 3000 }, ts: T0 });
  assert.equal(JSON.stringify(db.prepare(`SELECT * FROM trades ORDER BY block, log_index`).all()), before);
  assert.equal((db.prepare(`SELECT count(*) AS n FROM pools`).get() as { n: number }).n, 1, "only the pool of a coin we launched is kept");
});

/**
 * docs/phase14.md §4. The market page ranks by volume, so the list endpoint has to carry volume rather than the fee figure
 * that was standing in for it. The invariant worth holding is that the two places it is now reported agree.
 */
test("24-hour volume on a coin summary: the quote leg of every trade in the window, the same trades the chart counts", () => {
  const db = filled(), views = viewsOf(db);
  const coins = views.coins(50, 0).items;
  assert.ok(coins.length >= 2, "the story launches more than one coin");

  for (const c of coins) {
    const tape = views.trades(c.address, 500).items;
    const byHand = tape.reduce((t, x) => t + BigInt(x.quoteWei), 0n);
    assert.equal(c.volume24hWei, String(byHand), `${c.symbol}: the summary's volume is the sum of its own tape's quote legs`);
    // the chart's own volume is the same number in whole units, which is the claim API.md makes about it
    const drawn = views.chart(c.address, { range: "1d" })!.candles.reduce((t, k) => t + k.v, 0);
    assert.ok(Math.abs(drawn - Number(byHand) / 1e18) < 1e-30, `${c.symbol}: sum(candle.v) over a day is volume24hWei / 1e18`);
  }
  // a coin that traded is not reported as quiet, and the fee figure is not the same number: the whole reason for the field
  const traded = coins.find((c) => BigInt(c.volume24hWei) > 0n);
  assert.ok(traded, "at least one coin in the story has traded");
  assert.notEqual(traded!.volume24hWei, traded!.fees24hWei, "volume is the trade leg; fees are what a harvest swept");
});

test("a day with no trade in it has volume 0, which is a fact, and never null", () => {
  const db = filled();
  const later = new Views(db, new Live(null, null), () => T0 + 10 * 86_400);
  for (const c of later.coins(50, 0).items) {
    assert.equal(c.volume24hWei, "0", `${c.symbol}: nothing traded in the last day, so the volume is zero`);
    assert.equal(typeof c.volume24hWei, "string", "never null: a quiet coin is not an unknown one");
  }
  assert.ok(viewsOf(db).coins(50, 0).items.some((c) => c.volume24hWei !== "0"), "and the same coins do report volume inside the window");
});
