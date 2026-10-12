import assert from "node:assert/strict";
import { test } from "node:test";
import * as Demo from "../src/rook/sol/demo";
import * as DM from "../src/rook/sol/demoMarket";
import { coinFromIndexer, indexerBase, INDEXER, IX, nextHolderOffset, supplyOf } from "../src/rook/sol/indexer";
import { RESES, type IxCoin, type IxTrade, type IxTradePage } from "../src/rook/sol/indexerTypes";
import { candlesFromTrades, coinFromRow, columnsOf, filterCoins, marketStats, priceShort, priceText, progressOf, SOON, sparkOf, trendingOf } from "../src/rook/sol/market";
import { isAddress } from "../src/rook/sol/keys";
import { collectionBySlug } from "../src/rook/sol/collections";
import { thresholdOf } from "../src/rook/sol/graduation";

// ---- the indexer client: docs/v9/indexer-solana.md §2 ------------------------------------------------------------------------------

test("the indexer's base URL: https (or local http) only, no trailing slash; none in a test build", () => {
  assert.equal(INDEXER, "");
  assert.equal(indexerBase("https://rook-indexer-sol.up.railway.app/"), "https://rook-indexer-sol.up.railway.app");
  assert.equal(indexerBase(" http://localhost:8788 "), "http://localhost:8788");
  for (const bad of ["", "  ", "http://evil.example", "ftp://x", "javascript:alert(1)", undefined, null]) assert.equal(indexerBase(bad as string), "", String(bad));
});

test("the paths are the contract's, mints encoded, empty parameters left out", () => {
  const m = "DrKrXBpDqF39WSHGdAQmhbuBpixv4Ws5jkcSddJfn2aK";
  assert.equal(IX.board(), "/v1/rook/board");
  assert.equal(IX.coins(), "/v1/rook/coins");
  assert.equal(IX.coins({ sort: "graduating", collection: "milady", q: "a b", limit: 50 }), "/v1/rook/coins?sort=graduating&collection=milady&q=a%20b&limit=50");
  assert.equal(IX.coin(m), `/v1/rook/coins/${m}`);
  assert.equal(IX.chart(m, "15m"), `/v1/rook/coins/${m}/chart?res=15m`);
  assert.equal(IX.trades(m), `/v1/rook/coins/${m}/trades?limit=50`);
  assert.equal(IX.trades(m, 25, "123:456"), `/v1/rook/coins/${m}/trades?limit=25&before=123%3A456`);
  assert.equal(IX.holders(m), `/v1/rook/coins/${m}/holders?limit=20`);
  assert.equal(IX.holders(m, 20, 40), `/v1/rook/coins/${m}/holders?limit=20&offset=40`);
  assert.equal(IX.activity(30), "/v1/rook/activity?limit=30");
  assert.deepEqual(RESES.map((r) => r[0]), ["1m", "5m", "15m", "1h", "1d"]);
});

const ixRow = (): IxCoin => DM.demoIxCoin(Demo.DEMO_COINS()[0], DM.historyOf(Demo.DEMO_COINS()[0].mint)!, DM.demoNow());
test("an indexer row passes through as the market row; one missing the 24 h fields reads as not known, not zero", () => {
  const r = ixRow();
  const m = coinFromIndexer(r);
  assert.equal(m.mint, r.mint); assert.equal(m.volume24hSol, r.volume24hSol); assert.equal(m.collection.slug, r.collection.slug);
  const broken = { ...r, mint: "", collection: undefined, volume24hSol: undefined, trades24h: undefined, links: undefined } as unknown as IxCoin;
  const b = coinFromIndexer(broken);
  assert.equal(b.mint, r.address); assert.equal(b.collection.slug, r.nest.slug); assert.equal(b.volume24hSol, null); assert.equal(b.trades24h, null);
  assert.deepEqual(b.links, { website: null, twitter: null, telegram: null });
});

test("holder pages end at the last row; supply from raw and decimals", () => {
  assert.equal(nextHolderOffset({ offset: 0, items: new Array(20), total: 45, limit: 20 }), 20);
  assert.equal(nextHolderOffset({ offset: 40, items: new Array(5), total: 45, limit: 20 }), null);
  assert.equal(nextHolderOffset({ offset: 0, items: [], total: 10, limit: 20 }), null);
  assert.equal(supplyOf("1000000000000000", 6), 1e9);
  assert.equal(supplyOf(null, 6), null);
});

// ---- the market view -------------------------------------------------------------------------------------------------------------

// canister-only Rook: a collection coin's figures are in its collection's wX (the row's quote); the board reads no wX/SOL pair, so a
// wX-quoted coin has no USD figure on the board (its page prices it through the pair). ROOK's are in SOL, so SOL/USD applies.
test("an RPC row in the market shape: the curve's figures in the coin's quote, USD on SOL-quoted coins only, and nothing historical", () => {
  for (const r of Demo.demoCoins(2_000_000_000)) {
    const m = coinFromRow(r, 170);
    assert.equal(m.mint, r.mint); assert.equal(m.collection.slug, r.slug);
    assert.equal(m.phase, r.stats.phase === "curve" ? "trading" : r.stats.phase);
    assert.equal(progressOf(m), r.stats.phase === "curve" ? r.stats.progress : 1);
    assert.equal(m.curve === null, r.stats.phase === "graduated");
    assert.equal(m.volume24hSol, null); assert.equal(m.change24h, null); assert.equal(m.holders, null); assert.equal(m.trades24h, null);
    assert.deepEqual(m.quote, { symbol: r.slug === "rook" ? "SOL" : collectionBySlug(r.slug)!.wxSymbol, decimals: 9 });
    if (r.slug === "rook") assert.ok(Math.abs((m.marketCapUsd ?? 0) - (r.stats.marketCapSol ?? 0) * 170) < 1e-6);
    else { assert.equal(m.marketCapUsd, null, `${r.meta.symbol}: no USD without the pair`); assert.equal(m.priceUsd, null); }
    assert.equal(m.marketCapSol, r.stats.marketCapSol, "the market cap in the quote's units");
    assert.equal(m.supplyRaw, "1000000000000000");
  }
  const ix = coinFromIndexer(ixRow());
  assert.deepEqual(ix.quote, { symbol: collectionBySlug(ix.collection.slug)?.wxSymbol ?? "SOL", decimals: 9 }, "an indexer row carries its quote too");
});

test("the terminal's columns: every coin in exactly one, each in its order", () => {
  const coins = DM.demoMarket();
  const { fresh, soon, done } = columnsOf(coins);
  assert.equal(fresh.length + soon.length + done.length, coins.length);
  assert.equal(new Set([...fresh, ...soon, ...done].map((c) => c.mint)).size, coins.length);
  assert.ok(fresh.every((c) => c.phase === "trading" && progressOf(c) < SOON));
  assert.ok(soon.every((c) => c.phase === "graduating" || (c.phase === "trading" && progressOf(c) >= SOON)));
  assert.ok(done.every((c) => c.phase === "graduated"));
  for (let i = 1; i < fresh.length; i++) assert.ok((fresh[i - 1].launchedAt ?? 0) >= (fresh[i].launchedAt ?? 0));
  for (let i = 1; i < soon.length; i++) assert.ok(progressOf(soon[i - 1]) >= progressOf(soon[i]));
  for (let i = 1; i < done.length; i++) assert.ok((done[i - 1].graduatedAt ?? 0) >= (done[i].graduatedAt ?? 0));
  assert.ok(fresh.length >= 5 && soon.length >= 5 && done.length >= 3, "the demo fills all three columns");
});

test("filters, trending and the hero's figures", () => {
  const coins = DM.demoMarket();
  assert.ok(filterCoins(coins, { collection: "milady" }).every((c) => c.collection.slug === "milady"));
  assert.deepEqual(filterCoins(coins, { q: "mood" }).map((c) => c.symbol), ["MOOD"]);
  assert.equal(filterCoins(coins, { q: coins[3].mint.toLowerCase() }).length, 1);
  const t = trendingOf(coins, 5);
  for (let i = 1; i < t.length; i++) assert.ok((t[i - 1].volume24hSol ?? 0) >= (t[i].volume24hSol ?? 0));
  const st = marketStats(coins);
  assert.equal(st.coins, coins.length);
  assert.equal(st.graduated, coins.filter((c) => c.phase === "graduated").length);
  // the backing is the graduated collection coins' half-threshold in wX, and its value at the day-1 floors (ROOK has none)
  const grads = coins.filter((c) => c.phase === "graduated" && c.collection.slug !== "rook").map((c) => thresholdOf(c.collection.slug)!);
  assert.equal(st.backingWx, grads.reduce((s, t) => s + t.wx / 2, 0));
  assert.ok(Math.abs(st.backingSolAtFloors - grads.reduce((s, t) => s + (t.wx / 2) * t.floorSol, 0)) < 1e-9);
  assert.ok(st.backingWx > 0 && st.backingSolAtFloors > st.backingWx, "the demo has graduated collection coins, each wX worth more than a SOL");
  assert.ok(st.volume24hSol! > 0 && Math.abs(st.volume24hSol! - coins.reduce((s, c) => s + (c.volume24hSol ?? 0), 0)) < 1e-9);
  assert.equal(st.next.length, 3); assert.ok(st.next.every((c) => c.phase !== "graduated"));
  const noHist = marketStats(coins.map((c) => ({ ...c, volume24hSol: null, trades24h: null })));
  assert.equal(noHist.volume24hSol, null); assert.equal(noHist.trades24h, null);
});

test("candles from trades: buckets, OHLC, volume, empty buckets left out and the next candle opening at the last close", () => {
  const T = (at: number, price: number, sol = 1) => ({ at, price, sol });
  const k = candlesFromTrades([T(130, 3), T(100, 2), T(110, 5), T(350, 4, 2)], 60);
  assert.deepEqual(k.map((c) => c.t), [60, 120, 300]);
  assert.deepEqual(k[0], { t: 60, o: 2, h: 5, l: 2, c: 5, v: 2, n: 2 });
  assert.deepEqual(k[1], { t: 120, o: 5, h: 5, l: 3, c: 3, v: 1, n: 1 });
  assert.deepEqual(k[2], { t: 300, o: 3, h: 4, l: 3, c: 4, v: 2, n: 1 });
  const w = candlesFromTrades([T(100, 2), T(200, 7)], 60, 150);
  assert.equal(w.length, 1); assert.equal(w[0].o, 2, "a window's first candle opens at the price before it");
  assert.deepEqual(sparkOf([T(0, 1), T(50, 2), T(100, 3)], 0, 100, 3), [1, 2, 3]);
});

test("prices print without scientific notation; tiny ones with a subscript zero count", () => {
  assert.equal(priceText(1.429e-7), "0.0000001429");
  assert.equal(priceShort(1.429e-7), "0.0₆1429");
  assert.equal(priceShort(5.525e-7), "0.0₆5525");
  assert.equal(priceShort(0.0123), "0.01230");
  assert.equal(priceShort(null), "—");
});

// ---- the demo market: consistent with each coin's curve ---------------------------------------------------------------------------

const now = DM.demoNow();
const lam = (t: IxTrade) => Number(t.feeLamports ?? 0) / 1e9;

test("each demo coin's history agrees with its curve: the SOL left in the curve is its raise, the last price its board price", () => {
  for (const c of Demo.DEMO_COINS()) {
    const h = DM.historyOf(c.mint)!, sym = c.meta.symbol;
    assert.ok(h.trades.length >= 2, `${sym} has trades`);
    const curve = h.trades.filter((t) => t.venue === "curve");
    // buys: what the buyer paid less the fee went in; sells: what the seller got plus the fee came out
    const left = curve.reduce((s, t) => s + (t.side === "buy" ? t.sol - lam(t) : -(t.sol + lam(t))), 0);
    assert.ok(Math.abs(left - Number(c.stats.raisedLamports) / 1e9) < 1e-6, `${sym}: ${left} in the curve vs raised ${c.stats.raisedLamports}`);
    const last = h.trades[h.trades.length - 1];
    assert.ok(Math.abs(last.price / c.stats.priceSol! - 1) < 0.03, `${sym}: last trade ${last.price} vs board ${c.stats.priceSol}`);
    if (c.stats.phase !== "graduated") assert.equal(last.venue, "curve");
    for (let i = 0; i < h.trades.length; i++) {
      const t = h.trades[i];
      assert.ok(t.at! >= c.launchedAt && t.at! <= now, `${sym} trade inside its life`);
      if (i) assert.ok(t.at! >= h.trades[i - 1].at!, `${sym} trades in time order`);
      assert.ok(t.sol > 0 && t.tokens > 0 && t.price > 0);
      assert.equal(t.solLamports, String(Math.round(t.sol * 1e9))); assert.equal(t.tokenRaw, String(Math.round(t.tokens * 1e6)));
      assert.equal(t.tx, t.signature); assert.equal(t.id, `${t.signature}:0`);
      assert.ok(isAddress(t.trader));
      const rate = lam(t) / (t.side === "buy" ? t.sol : t.sol + lam(t));
      if (t.venue === "curve") assert.ok(Math.abs(rate - DM.demoFeeRate(t.at! - c.launchedAt)) < 1e-6 + 1 / Number(t.solLamports), `${sym}: the anti-sniper fee`);
    }
    assert.equal(h.trades[0].side, "buy"); assert.equal(h.trades[0].trader, c.creator, "the creator buys first");
    if (c.stats.phase === "graduated") {
      assert.ok(h.graduatedAt! > Number(c.pool_.finishCurveTimestamp) && h.dammPool && isAddress(h.dammPool));
      assert.ok(h.trades.filter((t) => t.venue === "pool").every((t) => t.at! > h.graduatedAt!), `${sym}: pool trades after graduation`);
    }
    assert.ok([...h.balances.values()].every((b) => b > -1e-6), `${sym}: nobody sold more than they held`);
  }
});

test("demo coin rows: 24 h figures from the trades, holders from the balances", () => {
  for (const c of DM.demoMarket()) {
    const h = DM.historyOf(c.mint)!;
    const day = h.trades.filter((t) => t.at! > now - 86_400);
    assert.equal(c.trades24h, day.length); assert.ok(Math.abs(c.volume24hSol! - day.reduce((s, t) => s + t.sol, 0)) < 1e-9);
    assert.equal(c.buys24h! + c.sells24h!, c.trades24h);
    assert.equal(c.lastTradeAt, h.trades[h.trades.length - 1].at);
    assert.equal(c.spark?.length, 24);
    const hp = DM.demoHolders(c.mint, 0, 500)!;
    assert.equal(c.holders, hp.holders);
  }
});

test("demo holders: the whole supply, largest first, the curve or the pool labelled, the creator labelled, pages that end", () => {
  for (const c of Demo.DEMO_COINS()) {
    const all = DM.demoHolders(c.mint, 0, 1000)!;
    const sum = all.items.reduce((s, h) => s + h.amount, 0);
    assert.ok(Math.abs(sum - 1e9) < 1, `${c.meta.symbol}: holders hold the supply (${sum})`);
    for (let i = 1; i < all.items.length; i++) assert.ok(all.items[i - 1].amount >= all.items[i].amount);
    assert.deepEqual(all.items.map((h) => h.rank), all.items.map((_, i) => i + 1));
    const vault = all.items.filter((h) => h.label === "curve" || h.label === "pool");
    assert.equal(vault.length, 1); assert.equal(vault[0].label, c.stats.phase === "graduated" ? "pool" : "curve");
    assert.equal(all.holders, all.items.length - 1, "the count leaves the vault out");
    assert.ok(all.items.filter((h) => h.label === "creator").every((h) => h.address === c.creator));
    let off = 0, seen = 0;
    for (;;) { const p = DM.demoHolders(c.mint, off, 20)!; seen += p.items.length; const n = nextHolderOffset(p); if (n === null) break; off = n; }
    assert.equal(seen, all.total);
  }
});

test("demo charts: every resolution, candles that are candles, the last close the last trade, graduation marked", () => {
  for (const c of Demo.DEMO_COINS().slice(0, 12)) {
    const h = DM.historyOf(c.mint)!;
    for (const [res, , seconds, buckets] of RESES) {
      const ch = DM.demoChart(c.mint, res)!;
      // the candles are priced in the coin's quote: its collection's wX, SOL for ROOK (canister-only Rook, §3)
      assert.equal(ch.seconds, seconds); assert.equal(ch.quote, c.slug === "rook" ? "SOL" : collectionBySlug(c.slug)!.wxSymbol); assert.equal(ch.decimals, 6); assert.equal(ch.supplyRaw, "1000000000000000");
      assert.ok(ch.candles.length <= (res === "1d" ? 365 : buckets + 1));
      for (const k of ch.candles) { assert.ok(k.h >= Math.max(k.o, k.c) && k.l <= Math.min(k.o, k.c) && k.n > 0 && k.t % seconds === 0); }
      assert.equal(ch.usd!.length, ch.candles.length);
      assert.equal(ch.tradesAllTime, h.trades.length);
      if (ch.candles.length) assert.equal(ch.candles[ch.candles.length - 1].c, h.trades[h.trades.length - 1].price);
      assert.equal(ch.graduation !== null, c.stats.phase === "graduated");
    }
    assert.equal(DM.demoChart(c.mint, "1d")!.trades, h.trades.length, "the 1d chart covers the whole life");
  }
});

test("demo trade pages walk the whole tape once, newest first", () => {
  const c = Demo.DEMO_COINS().find((x) => x.meta.symbol === "MOOD")!;
  const ids: string[] = [];
  let before: string | null = null;
  for (;;) { const p: IxTradePage = DM.demoTrades(c.mint, before, 50)!; ids.push(...p.items.map((t) => t.id)); if (!p.next) break; before = p.next; }
  const h = DM.historyOf(c.mint)!;
  assert.equal(ids.length, h.trades.length); assert.equal(new Set(ids).size, ids.length);
  assert.deepEqual(ids, h.trades.map((t) => t.id).reverse());
});

test("the demo feed: trades, launches and graduations, newest first; the stream invents trades on listed coins only", () => {
  const a = DM.demoActivity(80);
  assert.equal(a.items.length, 80);
  for (let i = 1; i < a.items.length; i++) assert.ok((a.items[i - 1].at ?? 0) >= (a.items[i].at ?? 0));
  assert.ok(a.items.some((x) => x.kind === "buy") && a.items.some((x) => x.kind === "sell"));
  assert.equal(a.source.kind, "fixture");
  const coins = DM.demoMarket();
  let s = 7;
  const r = () => ((s = (s * 16807) % 2147483647) / 2147483647);
  for (let i = 0; i < 50; i++) {
    const x = DM.demoStreamItem(coins, r, 123)!;
    assert.ok(coins.some((c) => c.mint === x.coin.address && c.phase !== "graduating"));
    assert.ok(x.kind === "buy" || x.kind === "sell"); assert.equal(x.at, 123); assert.equal(x.tx, null, "an invented trade links nowhere");
    assert.ok(Number(x.solLamports) > 0);
  }
  assert.equal(DM.demoStreamItem([], r), null);
});

test("top market cap ranks by USD market cap, then SOL", async () => {
  const { topByMcapOf } = await import("../src/rook/sol/market");
  const c = (mint: string, usd: number | null, sol: number | null) => ({ mint, marketCapUsd: usd, marketCapSol: sol }) as never;
  assert.deepEqual(topByMcapOf([c("a", 10, 1), c("b", 30, 3), c("c", null, 9), c("d", 20, 2)], 3).map((x: { mint: string }) => x.mint), ["b", "d", "a"]);
});
