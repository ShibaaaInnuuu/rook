import assert from "node:assert/strict";
import { test } from "node:test";
import { ALL_FALLBACK_SPAN, ALL_MIN_SPAN, POINTS_MAX, POINTS_TARGET, RANGE_KEYS, initialRange, rangeOf, resOf, windowOf } from "../src/candles.js";
import { handler } from "../src/api.js";
import { openDb } from "../src/db.js";
import { Follower, SUPPLY_RETRY_S } from "../src/follow.js";
import { Live } from "../src/live.js";
import { Views } from "../src/views.js";
import { RATE_EVERY_S, RATE_MAX_AGE_S, USD_POOL, rateOf, refreshRate, usdPerEthFrom } from "../src/usd.js";
import { clockOf, loadTape, replay } from "./helpers.js";

/**
 * docs/phase12.md: the three numbers a dollar market cap needs. This file holds the two the indexer had to learn to read —
 * the coin's whole supply, and dollars per ETH from one Uniswap v3 pool — and the ranges that replaced one-candle-per-trade.
 */

/** `totalSupply()`. Measured on a live staging coin: 1e27 base units, which at 18 decimals is a billion whole coins. */
const SUPPLY_SELECTOR = "0x18160ddd";
const SUPPLY = 10n ** 27n;
const word = (v: bigint) => `0x${v.toString(16).padStart(64, "0")}`;

const passed = async (answer: (data: string) => string | undefined) => {
  const t = loadTape(), db = openDb(":memory:");
  const rpc = replay(t, (method, params) => {
    if (method !== "eth_call") return undefined;
    return answer(String((params[0] as { data: string }).data));
  });
  const f = new Follower({ db, rpc, deployment: t.deployment, confirmations: 0 });
  await f.pass();
  return { db, f, views: new Views(db, new Live(null, null), () => clockOf(t)) };
};

test("a coin's whole supply is read from the chain at launch and served on its summary", async () => {
  const { views } = await passed((data) => (data.startsWith(SUPPLY_SELECTOR) ? word(SUPPLY) : undefined));
  const coins = views.coins(50, 0).items;
  assert.ok(coins.length > 0, "the recording holds at least one coin");
  assert.ok(coins.every((c) => c.supplyWei === String(SUPPLY)), `every coin carries its supply: ${JSON.stringify(coins.map((c) => c.supplyWei))}`);
});

test("a supply that would not read is null, never a guessed billion, and the follower asks again within the hour", async () => {
  const t = loadTape(), db = openDb(":memory:");
  let reads = 0, answer = false, clock = 1_800_000_000;
  const rpc = replay(t, (method, params) => {
    if (method !== "eth_call" || !String((params[0] as { data: string }).data).startsWith(SUPPLY_SELECTOR)) return undefined;
    reads++;
    return answer ? word(SUPPLY) : undefined;                 // undefined falls through to the recording, which holds no such call
  });
  const f = new Follower({ db, rpc, deployment: t.deployment, confirmations: 0, now: () => clock });
  await f.pass();
  const views = new Views(db, new Live(null, null), () => clockOf(t));
  assert.ok(views.coins(50, 0).items.every((c) => c.supplyWei === null), "a read that failed leaves the field empty");

  const asked = reads;
  await f.pass();
  assert.equal(reads, asked, "a coin already asked in this hour is not asked again: a node that will never answer costs one read, not one a pass");

  answer = true;
  clock += SUPPLY_RETRY_S + 1;
  await f.pass();
  assert.ok(reads > asked, "but the hour passes and it asks again, so a node that merely blinked does not cost the coin its market cap forever");
  assert.ok(views.coins(50, 0).items.every((c) => c.supplyWei === String(SUPPLY)), "and it is filled in when the node finally answers");
});

// ---- docs/phase12.md §3: dollars per ETH, from one pool, and every way it is allowed to be null -------------------------

/** The sqrt price that squares to the rate measured against the staging RPC on 2026-09-21. */
const SQRT_2785 = 4_181_220_769_977_932_959_449_088n;
const SLOT0_SELECTOR = "0x3850c7bd";
const slot0Word = (sqrt: bigint) => `0x${sqrt.toString(16).padStart(64, "0")}${"0".repeat(64 * 6)}`;

test("dollars per ETH is one pool's sqrt price squared, and the measured staging reading round-trips", () => {
  assert.equal(usdPerEthFrom(SQRT_2785)!.toFixed(2), "2785.14", "the number docs/phase12.md §3 recorded, from the same arithmetic");
  // the worked example in the doc: a billion coins at 3.133e-9 ETH each, in dollars
  assert.equal(Math.trunc(1e9 * 3.133e-9 * usdPerEthFrom(SQRT_2785)!), 8_725, "CHAL2SIM's $8.7k, which was never a wrong number, only an unreadable one");
  assert.equal(usdPerEthFrom(0n), null, "a pool that was never initialised has no price, and none is invented");
  assert.equal(usdPerEthFrom(null), null);
  assert.equal(usdPerEthFrom(-1n), null);
});

test("the rate is cached, re-read only every few minutes, and never read per request", async () => {
  const t = loadTape(), db = openDb(":memory:");
  let reads = 0, clock = 1_800_000_000;
  const rpc = replay(t, (method, params) => {
    if (method !== "eth_call" || !String((params[0] as { data: string }).data).startsWith(SLOT0_SELECTOR)) return undefined;
    reads++;
    return slot0Word(SQRT_2785);
  });
  assert.equal((await refreshRate(rpc, db, USD_POOL, clock)).usdPerEth!.toFixed(2), "2785.14");
  assert.equal(reads, 1);
  for (let i = 0; i < 5; i++) await refreshRate(rpc, db, USD_POOL, (clock += 30));
  assert.equal(reads, 1, "two and a half minutes of requests cost no second read");
  await refreshRate(rpc, db, USD_POOL, (clock += RATE_EVERY_S));
  assert.equal(reads, 2, "and past the interval it is read once more, not once per caller");
  assert.equal(rateOf(db, clock).pool, USD_POOL, "the pool it came from is served with it, so nobody has to take the number on trust");
});

test("no pool, a pool that will not answer, and a rate gone stale are all null rather than a guess", async () => {
  const t = loadTape(), db = openDb(":memory:");
  const dead = replay(t, (method) => (method === "eth_call" ? undefined : undefined));   // the recording holds no slot0: the read throws and tryRead swallows it
  assert.deepEqual(await refreshRate(dead, db, null, 1_000), { usdPerEth: null, at: null, pool: null }, "a chain with no such pool has no dollar rate at all");
  assert.equal((await refreshRate(dead, db, USD_POOL, 1_000)).usdPerEth, null, "a pool that will not read is null, not zero and not a default");

  const live = openDb(":memory:");
  const rpc = replay(t, (method, params) => (method === "eth_call" && String((params[0] as { data: string }).data).startsWith(SLOT0_SELECTOR) ? slot0Word(SQRT_2785) : undefined));
  await refreshRate(rpc, live, USD_POOL, 1_000);
  assert.ok(rateOf(live, 1_000 + RATE_MAX_AGE_S).usdPerEth, "inside its life it is served");
  const stale = rateOf(live, 1_000 + RATE_MAX_AGE_S + 1);
  assert.equal(stale.usdPerEth, null, "past it an indexer that lost its node stops quoting an old price");
  assert.equal(stale.at, 1_000, "but it still says when the last reading was, so the staleness is visible rather than silent");
});

test("the chart carries the supply and the rate, and says which pool the rate came from", async () => {
  const t = loadTape(), db = openDb(":memory:");
  const rpc = replay(t, (method, params) => {
    if (method !== "eth_call") return undefined;
    const data = String((params[0] as { data: string }).data);
    if (data.startsWith(SUPPLY_SELECTOR)) return word(SUPPLY);
    if (data.startsWith(SLOT0_SELECTOR)) return slot0Word(SQRT_2785);
    return undefined;
  });
  const f = new Follower({ db, rpc, deployment: t.deployment, confirmations: 0, now: () => clockOf(t) });
  await f.pass();
  const views = new Views(db, new Live(null, null), () => clockOf(t));
  const coin = views.coins(50, 0).items[0];
  const chart = views.chart(coin.address, { res: "5m" })!;
  assert.equal(chart.supplyWei, String(SUPPLY));
  assert.equal(chart.usdPerEth!.toFixed(2), "2785.14");
  assert.equal(chart.usdPerEthAt, clockOf(t));
  assert.equal(chart.usdPool!.toLowerCase(), USD_POOL);
});

// ---- docs/phase12.md §4: ranges, and the one-candle-per-trade view they replace ------------------------------------------

test("every range divides its window into time, and none of them can produce one candle per trade", () => {
  const now = 2_000_000;
  for (const key of RANGE_KEYS) {
    const w = windowOf(key, now - 30 * 86_400, now);
    assert.ok(w.seconds > 0, `${key} buckets by time, never by trade`);
    assert.ok(w.span / w.seconds <= POINTS_MAX, `${key} stays under the point ceiling: ${w.span / w.seconds}`);
    assert.ok(w.span / w.seconds >= 24, `${key} still draws enough points to be a line: ${w.span / w.seconds}`);
  }
  assert.deepEqual(windowOf("5m", now, now), { seconds: 5, span: 300 });
  assert.deepEqual(windowOf("1h", now, now), { seconds: 60, span: 3_600 });
  assert.deepEqual(windowOf("6h", now, now), { seconds: 300, span: 21_600 });
  assert.deepEqual(windowOf("1d", now, now), { seconds: 900, span: 86_400 });
});

test("`all` measures the coin's own life, and a coin twenty minutes old is not one box", () => {
  const now = 2_000_000;
  const young = windowOf("all", now - 1_200, now);
  assert.equal(young.span, ALL_MIN_SPAN, "under an hour old, `all` still shows an hour rather than a window of three seconds");
  assert.ok(young.span / young.seconds >= 24, "and it is a line, not a box");
  const week = windowOf("all", now - 7 * 86_400, now);
  assert.equal(week.span, 7 * 86_400, "older than that, `all` is exactly the coin's age");
  assert.ok(week.span / week.seconds <= POINTS_TARGET);
  const never = windowOf("all", null, now);
  assert.equal(never.span, ALL_FALLBACK_SPAN, "a coin with no launch time gets a month, not a crash and not a guess at its age");
  const ancient = windowOf("all", now - 40 * 365 * 86_400, now);
  assert.ok(ancient.span / ancient.seconds <= POINTS_MAX, "and nothing, however old, can ask for an unbounded series");
});

test("a coin opens on the finest range its own age can fill", () => {
  const now = 2_000_000;
  assert.equal(initialRange(now - 600, now), "1h", "ten minutes old");
  assert.equal(initialRange(now - 10_000, now), "6h");
  assert.equal(initialRange(now - 50_000, now), "1d");
  assert.equal(initialRange(now - 400_000, now), "all");
  assert.equal(initialRange(null, now), "1d", "an unknown age does not crash the chart");
});

test("rangeOf refuses what it does not know instead of quietly serving something else", () => {
  assert.deepEqual(RANGE_KEYS.map((r) => rangeOf(r)), [...RANGE_KEYS]);
  assert.equal(rangeOf("trade"), null, "the per-trade view is not a range and cannot be asked for as one");
  assert.equal(rangeOf("nonsense"), null);
  assert.equal(rangeOf(null), null);
  assert.equal(resOf("nonsense"), "5m", "the old parameter keeps its old forgiving behaviour, so the deployed site is unaffected");
});

test("the candles endpoint serves ranges, still serves res, and says which it drew", async () => {
  const t = loadTape(), db = openDb(":memory:");
  const rpc = replay(t, (method, params) => {
    if (method !== "eth_call") return undefined;
    const data = String((params[0] as { data: string }).data);
    return data.startsWith(SUPPLY_SELECTOR) ? word(SUPPLY) : data.startsWith(SLOT0_SELECTOR) ? slot0Word(SQRT_2785) : undefined;
  });
  const f = new Follower({ db, rpc, deployment: t.deployment, confirmations: 0, now: () => clockOf(t) });
  await f.pass();
  const views = new Views(db, new Live(null, null), () => clockOf(t));
  const coin = views.coins(50, 0).items[0].address.toLowerCase();
  const call = async (path: string) => {
    const res: { status?: number; body?: string } = {};
    const fake = { writeHead(s: number) { res.status = s; }, end(b?: string) { res.body = b; } };
    await handler({ views, follower: () => ({ lastPassAt: null, lastError: null, head: null, reorgs: 0, idle: false }), confirmations: 0, perMinute: 10_000, burst: 10_000 })(
      { method: "GET", url: path, headers: {}, socket: { remoteAddress: "1.2.3.4" } } as never, fake as never);
    return { status: res.status!, body: res.body ? (JSON.parse(res.body) as Record<string, unknown>) : null };
  };
  const six = await call(`/v1/coins/${coin}/candles?range=6h`);
  assert.equal(six.status, 200);
  assert.deepEqual([six.body!.range, six.body!.seconds, six.body!.span], ["6h", 300, 21_600], "it reports the range and the bucket it actually used");
  assert.equal(six.body!.supplyWei, String(SUPPLY));
  assert.equal((six.body!.usdPerEth as number).toFixed(2), "2785.14");

  const legacy = await call(`/v1/coins/${coin}/candles?res=1h`);
  assert.deepEqual([legacy.status, legacy.body!.res, legacy.body!.range], [200, "1h", null], "the deployed site's request still answers in the shape it expects");

  const bad = await call(`/v1/coins/${coin}/candles?range=trade`);
  assert.equal(bad.status, 400, "a range nobody defined is refused rather than silently turned into another one");
});
