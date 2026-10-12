import assert from "node:assert/strict";
import type { AddressInfo } from "node:net";
import { test } from "node:test";
import type { Abi } from "viem";
import { serve } from "../src/api.js";
import { openDb } from "../src/db.js";
import { Follower } from "../src/follow.js";
import { Live } from "../src/live.js";
import { Views } from "../src/views.js";
import { parseConfig } from "../src/wrapped/config.js";
import { WrappedService } from "../src/wrapped/service.js";
import { CURVE_TRADE_EVENTS, ERC20_EVENTS } from "../src/rook/abis.js";
import { chartOf, marketOf, spanOf } from "../src/rook/board.js";
import { CurveReader } from "../src/rook/curve.js";
import { rookRoutes } from "../src/rook/routes.js";
import { RookService } from "../src/rook/service.js";
import type { Activity, Board, RookChart, RookHolderPage, RookTradePage, WalletProfile } from "../src/rook/types.js";
import { clockOf, loadTape, replay } from "./helpers.js";
import { buildWorld, FROGS, NOW, worldConfig } from "./wrappedWorld.js";

/** docs/v6/site-design.md §6: the board and the coin page's data for both kinds of coin, from the recorded run and from the worked world. */
const E = 10n ** 18n;
const buyer = "0x4000000000000000000000000000000000000001", other = "0x4000000000000000000000000000000000000002";

function worldWithTrades() {
  const w = buildWorld();
  // three buys and a sell on BETA's curve, at the worked prices, and the coin's transfers with them
  const steps: [string, "Bought" | "Sold", bigint, bigint, bigint, number][] = [
    [buyer, "Bought", 300n * 10n ** 15n, 3n * 10n ** 15n, 120_000_000n * E, 320], [other, "Bought", 250n * 10n ** 15n, 25n * 10n ** 14n, 90_000_000n * E, 330],
    [buyer, "Bought", 400n * 10n ** 15n, 4n * 10n ** 15n, 130_000_000n * E, 340], [other, "Sold", 60n * 10n ** 15n, 6n * 10n ** 14n, 20_000_000n * E, 350]];
  for (const [who, ev, eth, fee, tokens, block] of steps) {
    if (ev === "Bought") { w.event(FROGS.betaCurve, CURVE_TRADE_EVENTS as Abi, "Bought", { buyer: who, ethIn: eth, fee, tokensOut: tokens }, block); w.event(FROGS.beta, ERC20_EVENTS as Abi, "Transfer", { from: FROGS.betaCurve, to: who, value: tokens }, block); }
    else { w.event(FROGS.betaCurve, CURVE_TRADE_EVENTS as Abi, "Sold", { seller: who, tokensIn: tokens, ethOut: eth, fee }, block); w.event(FROGS.beta, ERC20_EVENTS as Abi, "Transfer", { from: who, to: FROGS.betaCurve, value: tokens }, block); }
  }
  return w;
}

test("the curve reader folds Bought/Sold into curve-side prices and Transfers into balances; a second read is incremental", async () => {
  const w = worldWithTrades();
  const r = new CurveReader(w.rpc, { confirmations: 0, chunk: 250, now: () => NOW, ttlS: 0 });
  const c = { coin: FROGS.beta as `0x${string}`, curve: FROGS.betaCurve as `0x${string}`, fromBlock: 1 };
  const a = await r.read(c);
  assert.equal(a.trades.length, 4); assert.deepEqual(a.trades.map((t) => t.side), ["buy", "buy", "buy", "sell"]);
  const first = a.trades[0];
  assert.equal(first.price, Number(300n * 10n ** 15n - 3n * 10n ** 15n) / Number(120_000_000n * E), "the fee comes off the ETH in before the price");
  assert.equal(a.balances.get(buyer), 250_000_000n * E); assert.equal(a.balances.get(other), 70_000_000n * E);
  assert.equal(a.graduation, null);
  w.head = 1_001; w.event(FROGS.betaCurve, CURVE_TRADE_EVENTS as Abi, "Bought", { buyer, ethIn: 10n ** 17n, fee: 10n ** 15n, tokensOut: 30_000_000n * E }, 1_001);
  const b = await r.read(c);
  assert.equal(b.trades.length, 5, "the new trade is read from where the last read stopped");
});

test("market and chart arithmetic: change against the price a day earlier, the day's volume, candles at the asked resolution", () => {
  const t = (at: number, price: number, quote = "1000000000000000000") => ({ id: `${at}:0`, side: "buy" as const, venue: "curve" as const, quoteWei: quote, coinWei: "1", feeWei: null, taxWei: null, price, trader: buyer as `0x${string}`, tx: "0x1" as `0x${string}`, block: at, logIndex: 0, at });
  const now = 1_000_000;
  const m = marketOf([t(now - 90_000, 2), t(now - 3_000, 3), t(now - 100, 4)], now);
  assert.equal(m.price, 4); assert.equal(m.change24h, 1, "against the last price before the window"); assert.equal(m.volume24hWei, String(2n * E)); assert.equal(m.trades24h, 2);
  assert.equal(marketOf([t(now - 100, 4)], now).change24h, null, "one price is no change");
  assert.equal(marketOf([t(now - 200, 2), t(now - 100, 3)], now).change24h, 0.5, "with nothing before the window, the first in it");
  assert.equal(marketOf([], now).price, null);
  const c = chartOf([t(now - 130, 1), t(now - 125, 2), t(now - 40, 3)], "1m", now);
  assert.equal(c.seconds, 60); assert.equal(c.span, spanOf("1m")); assert.equal(c.candles.length, 2, "two trades in one minute share a candle"); assert.equal(c.candles[0].n, 2); assert.equal(c.candles[1].n, 1); assert.equal(c.candles[0].c, 2);
});

async function up() {
  const t = loadTape(), db = openDb(":memory:"), rpc = replay(t);
  const f = new Follower({ db, rpc, deployment: t.deployment, confirmations: 0 });
  await f.pass();
  const now = () => clockOf(t);
  const views = new Views(db, new Live(rpc, t.deployment.ponsFactory), now);
  const w = worldWithTrades();
  const wrappedConfig = parseConfig(worldConfig());
  const wrapped = new WrappedService({ config: wrappedConfig, rpc: w.rpc, fetch: w.fetch, now: () => NOW });
  const rook = rookRoutes(new RookService({ db, views, wrapped, wrappedConfig, rpc: w.rpc, now: () => NOW, ttlS: 0 }));
  const server = serve({ views, wrapped, rook, follower: () => f.state, confirmations: 0, perMinute: 10_000, burst: 10_000 }, 0, "127.0.0.1");
  await new Promise((r) => server.once("listening", r));
  const base = `http://127.0.0.1:${(server.address() as AddressInfo).port}`;
  return { close: () => server.close(), get: async <X>(p: string) => { const r = await fetch(base + p); return { r, j: (await r.json()) as X }; } };
}

test("the board lists both kinds of coin, and each coin answers chart, trades and holders in one shape", async () => {
  const s = await up();
  try {
    const { j: board } = await s.get<Board>("/v1/rook/board");
    assert.equal(board.source.kind, "live");
    const kinds = new Set(board.rows.map((r) => r.kind));
    assert.ok(kinds.has("pons") && kinds.has("curve"), `both kinds: ${[...kinds]}`);
    const beta = board.rows.find((r) => r.symbol === "BETA")!;
    assert.equal(beta.kind, "curve"); assert.equal(beta.phase, "trading"); assert.equal(beta.policy, "burn"); assert.equal(beta.nest.slug, "frogs");
    assert.ok(beta.price! > 0 && beta.trades24h === 4 && beta.holders === 2, JSON.stringify(beta));
    assert.equal(beta.buys24h, 3); assert.equal(beta.newHolders24h, 2, "both holders first received the coin inside the day");
    assert.deepEqual(beta.raise, { raisedWei: "1200000000000000000", raiseWei: "5000000000000000000" });
    const pons = board.rows.find((r) => r.kind === "pons")!;
    assert.match(pons.address, /^0x/); assert.equal(pons.policy, null); assert.ok(pons.nest.listingId !== null);
    const { j: chart } = await s.get<RookChart>(`/v1/rook/coins/${FROGS.beta}/chart?res=5m`);
    assert.equal(chart.kind, "curve"); assert.equal(chart.seconds, 300); assert.equal(chart.tradesAllTime, 4); assert.ok(chart.candles.length >= 1);
    assert.equal((await s.get(`/v1/rook/coins/${FROGS.beta}/chart?res=2m`)).r.status, 400);
    const { j: trades } = await s.get<RookTradePage>(`/v1/rook/coins/${FROGS.beta}/trades?limit=3`);
    assert.equal(trades.total, 4); assert.equal(trades.items.length, 3); assert.equal(trades.items[0].side, "sell", "newest first"); assert.ok(trades.next);
    const { j: page2 } = await s.get<RookTradePage>(`/v1/rook/coins/${FROGS.beta}/trades?limit=3&before=${trades.next}`);
    assert.equal(page2.items.length, 1); assert.equal(page2.next, null);
    const { j: holders } = await s.get<RookHolderPage>(`/v1/rook/coins/${FROGS.beta}/holders`);
    assert.equal(holders.holders, 2); assert.equal(holders.items[0].address.toLowerCase(), buyer); assert.equal(holders.items[0].rank, 1);
    const { j: pchart } = await s.get<RookChart>(`/v1/rook/coins/${pons.address}/chart?res=1h`);
    assert.equal(pchart.kind, "pons"); assert.equal(pchart.seconds, 3_600);
    const { j: ptrades } = await s.get<RookTradePage>(`/v1/rook/coins/${pons.address}/trades`);
    assert.equal(ptrades.kind, "pons");
    assert.equal((await s.get(`/v1/rook/coins/0x${"9".repeat(40)}/chart`)).r.status, 404);
    const { j: act } = await s.get<Activity>("/v1/rook/activity?limit=20");
    assert.ok(act.items.some((x) => x.kind === "buy" && x.coin?.symbol === "BETA") && act.items.some((x) => x.kind === "launch"), "trades and launches in the ticker");
    for (let i = 1; i < act.items.length; i++) assert.ok((act.items[i - 1].at ?? 0) >= (act.items[i].at ?? 0), "newest first");
    const { j: w } = await s.get<WalletProfile>(`/v1/rook/wallets/${buyer}`);
    assert.equal(w.address.toLowerCase(), buyer); assert.deepEqual(w.holdings.map((h) => h.coin.symbol), ["BETA"]); assert.equal(w.holdings[0].balanceWei, String(250_000_000n * E));
    assert.equal(w.trades.length, 2); assert.equal(w.trades[0].coin.symbol, "BETA"); assert.deepEqual(w.launches, []); assert.deepEqual(w.wins, []);
    const { j: root } = await s.get<{ endpoints: string[] }>("/");
    assert.ok(root.endpoints.includes("/v1/rook/board"));
  } finally { s.close(); }
});
