/**
 * Demo market history: trades, candles, holders and the activity feed for the demo coins (src/rook/sol/demo.ts), in the indexer's
 * shapes (docs/v9/indexer-solana.md §2), generated so they agree with each coin's curve:
 *
 *  - every curve trade moves the pool along Rook's real curve segment (constant liquidity from the start price to graduation), so a
 *    buy of Δ quote (net of fee) gives Δ / (√Pa·√Pb) coins and lifts the price, a sell does the reverse. The quote is the
 *    collection's wX (ROOK's: SOL); the indexer shapes' `sol` / `solLamports` fields carry it, as src/rook/sol/indexerTypes.ts says;
 *  - the quote that stayed in the curve (buys net of fee, less sells before fee) is exactly the coin's raised amount, so the last
 *    curve trade's price is the coin's price on the board, and a full curve ends at the coin's own threshold (its collection's whole
 *    wX, on the curve scaled to it; ROOK's 85 SOL);
 *  - the fee is the anti-sniper schedule (50 % at launch, straight down to 1 % at ten minutes, then 1 %);
 *  - a graduated coin's pool trades walk its price from the graduation price to the board's price (a Brownian bridge);
 *  - holders are what the trades left in each wallet; the curve (or the pool) holds the rest of the 1B supply.
 *
 * Deterministic per coin (a PRNG seeded by its ticker) for a given `now`. Nothing here is real: traders and signatures are made up,
 * and the pages say "demo" and link none of them.
 */
import { collectionBySlug } from "./collections";
import { DEMO_COINS, DEMO_SOL_USD, demoAddress, demoCurveOf, demoQuoteSol, type DemoCoin } from "./demo";
import { quoteOf } from "./graduation";
import { candlesFromTrades, sparkOf, type MarketCoin } from "./market";
import { RESES, type IxActivity, type IxActivityItem, type IxChart, type IxCoin, type IxHolder, type IxHolderPage, type IxRes, type IxTrade, type IxTradePage } from "./indexerTypes";

const SUPPLY = 1_000_000_000, DEC = 6, SUPPLY_RAW = "1000000000000000";
const Q64 = 2 ** 64;
// √P in quote-per-coin units: σ(q) = (S0 + q·1e9·2^64/L) · √1e-3; P = σ². LS = dq/dσ (quote per unit σ), constant on the segment.
const ROOT = Math.sqrt(1e-3);
/** A collection's curve segment (the shape scaled to its threshold): σ at q whole quote raised, and LS. */
function segmentOf(slug: string) {
  const k = demoCurveOf(slug), S0 = Number(k.START) / Q64, L = Number(k.L1);
  return { sigmaAt: (qSol: number) => (S0 + (qSol * 1e9 * Q64) / L) * ROOT, LS: L / (1e9 * Q64 * ROOT) };
}
/** The curve price (quote per coin) after q whole quote went in, on `slug`'s curve (ROOK's: the 85 SOL curve). */
export const demoCurvePrice = (qSol: number, slug = "rook") => segmentOf(slug).sigmaAt(qSol) ** 2;
/** The dollar price of one coin: its quote price × the demo's wX/SOL rate (the day-1 floor; 1 for ROOK) × SOL/USD. */
const usdPerQuote = (slug: string) => demoQuoteSol(slug) * DEMO_SOL_USD.price;

/** The anti-sniper fee at `dt` seconds after launch: 50 % falling 0.8167 % every 10 s to 1 % at 600 s. */
export const demoFeeRate = (dt: number) => Math.max(0.01, 0.5 - Math.floor(Math.max(0, dt) / 10) * 0.008166666);

function rngOf(seed: string) {
  let h = 2166136261 >>> 0;
  for (let i = 0; i < seed.length; i++) { h ^= seed.charCodeAt(i); h = Math.imul(h, 16777619) >>> 0; }
  return () => { h = (h + 0x6d2b79f5) >>> 0; let t = h; t = Math.imul(t ^ (t >>> 15), t | 1); t ^= t + Math.imul(t ^ (t >>> 7), t | 61); return ((t ^ (t >>> 14)) >>> 0) / 4294967296; };
}
const B58 = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
const fakeSig = (r: () => number) => { let s = ""; for (let i = 0; i < 87; i++) s += B58[Math.floor(r() * 58)]; return s; };
const logn = (r: () => number, sd: number) => Math.exp(sd * Math.sqrt(-2 * Math.log(r() || 1e-9)) * Math.cos(2 * Math.PI * r()));

let traders: string[] | null = null;
/** 160 made-up wallets the demo's trades come from (traders trade several coins, as they do). */
export const demoTraders = (): string[] => (traders ??= Array.from({ length: 160 }, (_, i) => demoAddress(`trader:${i}`)));
const pickTrader = (r: () => number) => { const t = demoTraders(); return t[Math.min(t.length - 1, Math.floor(t.length * r() ** 2.2))]; };

const BASE_SLOT = 370_000_000, BASE_AT = 1_790_000_000;
const slotAt = (at: number) => Math.max(1, Math.floor(BASE_SLOT + (at - BASE_AT) * 2.5));
const lam = (sol: number) => String(Math.round(sol * 1e9));
const raw = (tokens: number) => String(Math.round(tokens * 10 ** DEC));

export type DemoHistory = { trades: IxTrade[]; /** oldest first */ balances: Map<string, number>; graduatedAt: number | null; dammPool: string | null };

/** The coin's whole history up to `now`, oldest first. */
export function demoHistory(c: DemoCoin, now: number): DemoHistory {
  const r = rngOf(`hist:${c.meta.symbol}`);
  const { sigmaAt, LS } = segmentOf(c.slug);
  const out: IxTrade[] = [];
  const bal = new Map<string, number>();
  const add = (who: string, d: number) => bal.set(who, (bal.get(who) ?? 0) + d);
  const graduated = c.stats.phase === "graduated";
  const target = Number(c.stats.raisedLamports) / 1e9;
  const curveEnd = graduated ? Number(c.pool_.finishCurveTimestamp) : now - 3;
  const life = Math.max(4, curveEnd - c.launchedAt);
  const n = Math.max(2, Math.min(Math.round(8 + (graduated ? 1 : c.stats.progress) * 260), Math.floor(life / 3)));
  // times: sorted, a little heavier toward the end of the window (activity picks up as a curve fills)
  const times = Array.from({ length: n }, (_, i) => (i === 0 ? c.launchedAt + 1 + Math.floor(r() * 2) : c.launchedAt + 3 + Math.floor((life - 3) * r() ** 0.85))).sort((a, b) => a - b);
  let q = 0, seq = 0;
  const push = (side: "buy" | "sell", trader: string, solNet: number, tokens: number, fee: number, at: number, venue: "curve" | "pool", price: number) => {
    const sol = side === "buy" ? solNet + fee : solNet - fee;
    const sig = fakeSig(r);
    out.push({ id: `${sig}:0`, side, venue, solLamports: lam(sol), sol, tokenRaw: raw(tokens), tokens, feeLamports: lam(fee), price, priceUsd: price * usdPerQuote(c.slug), trader, signature: sig, tx: sig, slot: slotAt(at) + (seq++ % 3), at });
  };
  const buy = (at: number, net: number, who: string) => {
    const rate = demoFeeRate(at - c.launchedAt);
    const sa = sigmaAt(q), sb = sigmaAt(q + net);
    const tokens = LS * (1 / sa - 1 / sb);
    q += net; add(who, tokens);
    push("buy", who, net, tokens, (net * rate) / (1 - rate), at, "curve", net / tokens);
  };
  for (let k = 0; k < n; k++) {
    const at = times[k], rate = demoFeeRate(at - c.launchedAt);
    const remaining = target - q, late = k >= n - 3;
    const holders = [...bal].filter(([w, b]) => b > 1 && w !== c.creator);
    if (!late && k > 1 && q > 0.05 && holders.length && r() < 0.32) {
      const [who, b] = holders[Math.floor(r() * holders.length)];
      let tokens = b * (0.25 + 0.75 * r());
      const sb = sigmaAt(q), smin = sigmaAt(0);
      let sa = 1 / (1 / sb + tokens / LS);
      if (sa < smin) { sa = smin; tokens = LS * (1 / sa - 1 / sb); }
      const gross = (sb - sa) * LS;               // quote out of the curve, before the fee
      if (gross < 0.002) continue;               // dust sells are left out
      q -= gross; add(who, -tokens);
      push("sell", who, gross, tokens, gross * rate, at, "curve", gross / tokens);
      continue;
    }
    if (remaining <= 1e-9) continue;
    const left = n - k;
    let net = Math.min(remaining * 0.6, Math.max(0.004, (remaining / left) * 1.25 * logn(r, 0.9)));
    if (k === 0) net = Math.min(remaining * (n > 1 ? 0.3 : 1), 0.3 + r() * 1.6);
    buy(at, net, k === 0 ? c.creator : pickTrader(r));
  }
  // top the curve up to its raise with closing buys no bigger than 0.4 quote, so the last trade's price is the board's
  let t = times[n - 1];
  while (target - q > 1e-9) {
    const rem = target - q;
    t = Math.min(curveEnd, t + 1 + Math.floor(r() * 20));
    buy(t, rem > 0.4 ? Math.max(0.4, rem * 0.5) : rem, pickTrader(r));
  }
  let graduatedAt: number | null = null, dammPool: string | null = null;
  if (graduated) {
    graduatedAt = curveEnd + 40 + Math.floor(r() * 80);
    dammPool = demoAddress(`damm:${c.meta.symbol}`);
    const p0 = sigmaAt(target) ** 2, p1 = c.stats.priceSol ?? p0;
    const span = Math.max(60, now - 5 - (graduatedAt + 30));
    const m = Math.max(12, Math.min(220, Math.round(60 + (span / 86_400) * 30)));
    const ts = Array.from({ length: m }, () => graduatedAt! + 30 + Math.floor(span * r())).sort((a, b) => a - b);
    const walk: number[] = [0];
    for (let k = 1; k <= m; k++) walk.push(walk[k - 1] + 0.05 * (r() * 2 - 1) * Math.sqrt(3));
    let prevP = p0;
    for (let k = 1; k <= m; k++) {
      const f = k / m, lp = Math.log(p0) + (Math.log(p1) - Math.log(p0)) * f + walk[k] - walk[m] * f;
      const price = k === m ? p1 : Math.exp(lp);
      const up = price >= prevP;
      prevP = price;
      const holders = [...bal].filter(([, b]) => b > 1);
      const held = holders.reduce((s, [, b]) => s + b, 0);
      if (up && held < SUPPLY * 0.95) {
        const net = Math.max(0.01, 0.6 * logn(r, 1));
        const tokens = net / price, who = pickTrader(r);
        add(who, tokens);
        push("buy", who, net, tokens, (net * 0.01) / 0.99, ts[k - 1], "pool", price);
      } else if (holders.length) {
        const [who, b] = holders[Math.floor(r() * holders.length)];
        const tokens = b * (0.2 + 0.8 * r()), gross = tokens * price;
        add(who, -tokens);
        push("sell", who, gross, tokens, gross * 0.01, ts[k - 1], "pool", price);
      }
    }
  }
  return { trades: out, balances: bal, graduatedAt, dammPool };
}

let cache: { now: number; by: Map<string, DemoHistory> } | null = null;
/** The page's demo clock: fixed at the first read, so every figure agrees with every other one for the page's life. */
export const demoNow = (): number => (cache ??= { now: Math.floor(Date.now() / 1000), by: new Map() }).now;
export function historyOf(mint: string): DemoHistory | null {
  const now = demoNow();
  const c = DEMO_COINS().find((x) => x.mint === mint);
  if (!c) return null;
  let h = cache!.by.get(mint);
  if (!h) { h = demoHistory(c, now); cache!.by.set(mint, h); }
  return h;
}

/** A demo coin as the indexer's row (§2.2): its 24 h figures, change, holders and a sparkline from its own history. */
export function demoIxCoin(c: DemoCoin, h: DemoHistory, now: number): MarketCoin & IxCoin & { spark: number[] } {
  const day = h.trades.filter((t) => (t.at ?? 0) > now - 86_400);
  const before = h.trades.filter((t) => (t.at ?? 0) <= now - 86_400).pop() ?? h.trades[0];
  const last = h.trades[h.trades.length - 1];
  const price = c.stats.priceSol;
  const vol = day.reduce((s, t) => s + t.sol, 0);
  const grad = c.stats.phase === "graduated";
  const collection = { slug: c.slug, listingId: collectionBySlug(c.slug)?.listingId ?? null };
  const holders = [...h.balances.values()].filter((b) => b > 0.5).length;
  const usdPer = usdPerQuote(c.slug);
  return {
    kind: "dbc", address: c.mint, mint: c.mint, pool: c.pool, dammPool: h.dammPool, config: c.config, collection, nest: { ...collection, name: null, symbol: null }, quote: quoteOf(c.slug),
    name: c.meta.name, symbol: c.meta.symbol, logoUrl: c.off.image, description: c.off.description, links: { website: null, twitter: null, telegram: null },
    metadataUri: c.meta.uri, creator: c.creator, decimals: DEC, supplyRaw: SUPPLY_RAW,
    phase: grad ? "graduated" : c.stats.phase === "graduating" ? "graduating" : "trading", graduated: grad,
    launchedAt: c.launchedAt, graduatedAt: h.graduatedAt, lastTradeAt: last?.at ?? null,
    curve: grad ? null : { raisedLamports: c.stats.raisedLamports.toString(), thresholdLamports: c.stats.thresholdLamports.toString(), progress: c.stats.progress },
    raise: grad ? null : { raisedLamports: c.stats.raisedLamports.toString(), raiseLamports: c.stats.thresholdLamports.toString() },
    price, priceUsd: price !== null ? price * usdPer : null,
    change24h: before && price ? price / before.price - 1 : null,
    marketCapSol: c.stats.marketCapSol, marketCapUsd: c.stats.marketCapSol !== null ? c.stats.marketCapSol * usdPer : null,
    volume24hLamports: lam(vol), volume24hSol: vol, volume24hUsd: vol * usdPer,
    trades24h: day.length, buys24h: day.filter((t) => t.side === "buy").length, sells24h: day.filter((t) => t.side === "sell").length,
    holders, holdersAt: now - 60,
    spark: sparkOf(h.trades, Math.max(c.launchedAt, now - 86_400), now, 24),
  };
}

/** Every demo coin in the market shape, newest first. */
export function demoMarket(): MarketCoin[] {
  const now = demoNow();
  return DEMO_COINS().map((c) => demoIxCoin(c, historyOf(c.mint)!, now));
}

const SRC = { kind: "fixture" as const, note: "demo: invented coins, not on chain" };

export function demoChart(mint: string, res: IxRes): IxChart | null {
  const h = historyOf(mint), now = demoNow();
  if (!h) return null;
  const [, , seconds, buckets] = RESES.find((x) => x[0] === res)!;
  const c = DEMO_COINS().find((x) => x.mint === mint)!;
  const from = res === "1d" ? -Infinity : now - seconds * buckets;
  const candles = candlesFromTrades(h.trades, seconds, from, now);
  const usd = usdPerQuote(c.slug);
  return {
    source: SRC, kind: "dbc", coin: mint, res, seconds, span: res === "1d" ? Math.max(seconds, now - c.launchedAt) : seconds * buckets, quote: quoteOf(c.slug).symbol,
    candles, usd: candles.map((k) => ({ ...k, o: k.o * usd, h: k.h * usd, l: k.l * usd, c: k.c * usd, v: k.v * usd })),
    trades: candles.reduce((s, k) => s + k.n, 0), tradesAllTime: h.trades.length, launchedAt: c.launchedAt,
    graduation: h.graduatedAt ? { at: h.graduatedAt, slot: slotAt(h.graduatedAt), tx: fakeSig(rngOf(`grad:${mint}`)), pool: h.dammPool } : null,
    supplyRaw: SUPPLY_RAW, decimals: DEC, usdPerSol: usd,
  };
}

/** The tape, newest first; `before` is the previous page's `next` ("slot:index" into the newest-first list). */
export function demoTrades(mint: string, before: string | null = null, limit = 50): IxTradePage | null {
  const h = historyOf(mint);
  if (!h) return null;
  const all = h.trades.slice().reverse();
  const start = before ? Number(before.split(":")[1] ?? 0) : 0;
  const items = all.slice(start, start + limit);
  const end = start + items.length;
  return { source: SRC, kind: "dbc", coin: mint, items, next: end < all.length ? `${items[items.length - 1].slot}:${end}` : null, total: all.length };
}

/** Holders, largest first, the curve's (or the pool's) vault among them with its label; `holders` counts wallets only. */
export function demoHolders(mint: string, offset = 0, limit = 20): IxHolderPage | null {
  const h = historyOf(mint);
  const c = DEMO_COINS().find((x) => x.mint === mint);
  if (!h || !c) return null;
  const wallets = [...h.balances].filter(([, b]) => b > 0.5);
  const held = wallets.reduce((s, [, b]) => s + b, 0);
  const vault: [string, number, IxHolder["label"]] = h.dammPool ? [h.dammPool, SUPPLY - held, "pool"] : [c.pool_.baseVault, SUPPLY - held, "curve"];
  const rows: [string, number, IxHolder["label"]][] = [vault, ...wallets.map(([w, b]): [string, number, IxHolder["label"]] => [w, b, w === c.creator ? "creator" : null])].sort((a, b) => b[1] - a[1]);
  const items = rows.slice(offset, offset + limit).map(([address, amount, label], i): IxHolder => ({
    rank: offset + i + 1, address, tokenAccount: demoAddress(`ata:${mint}:${address}`), amountRaw: raw(amount), amount, share: amount / SUPPLY, label,
  }));
  return { source: SRC, kind: "dbc", coin: mint, holders: wallets.length, supplyRaw: SUPPLY_RAW, decimals: DEC, complete: true, refreshedAt: demoNow() - 60, items, total: rows.length, limit, offset };
}

const shortAddr = (a: string) => `${a.slice(0, 4)}…${a.slice(-4)}`;
/** "8Xk3…f3Qa bought 0.120 wMILADY of MOOD": `quote` is the coin's quote symbol (its collection's wX; SOL for ROOK). */
export const activityText = (kind: IxActivityItem["kind"], symbol: string, sol: number | null, trader: string | null, quote = "SOL"): string =>
  kind === "launch" ? `${symbol} launched` : kind === "graduated" ? `${symbol} graduated` : `${trader ? shortAddr(trader) : "someone"} ${kind === "buy" ? "bought" : "sold"} ${sol !== null ? `${sol < 1 ? sol.toFixed(3) : sol.toFixed(2)} ${quote} of` : ""} ${symbol}`;

/** The cross-coin feed (§2.9): trades, launches and graduations, newest first. */
export function demoActivity(limit = 50): IxActivity {
  const now = demoNow();
  const items: IxActivityItem[] = [];
  for (const c of DEMO_COINS()) {
    const h = historyOf(c.mint)!;
    const coin = { address: c.mint, symbol: c.meta.symbol, name: c.meta.name, logoUrl: c.off.image, collection: c.slug };
    const quote = quoteOf(c.slug).symbol;
    items.push({ at: c.launchedAt, kind: "launch", coin, text: activityText("launch", c.meta.symbol, null, null, quote), solLamports: null, trader: c.creator, tx: null });
    if (h.graduatedAt) items.push({ at: h.graduatedAt, kind: "graduated", coin, text: activityText("graduated", c.meta.symbol, null, null, quote), solLamports: null, trader: null, tx: null });
    for (const t of h.trades.slice(-40)) items.push({ at: t.at, kind: t.side, coin, text: activityText(t.side, c.meta.symbol, t.sol, t.trader, quote), solLamports: t.solLamports, trader: t.trader, tx: t.tx });
  }
  items.sort((a, b) => (b.at ?? 0) - (a.at ?? 0));
  return { source: SRC, at: now, items: items.slice(0, limit) };
}

/** One more invented trade for the demo's live stream (client side, on a timer, demo only): a coin picked by its 24 h volume,
 *  a size around its typical trade (sized to its curve: a 5-wX curve sees trades 17× smaller than the 85 SOL one), in its quote,
 *  at its current price. Display only: it does not move the coin's curve. */
export function demoStreamItem(coins: MarketCoin[], r: () => number = Math.random, at = Math.floor(Date.now() / 1000)): IxActivityItem | null {
  const live = coins.filter((c) => c.phase !== "graduating");
  if (!live.length) return null;
  const w = live.map((c) => 1 + (c.volume24hSol ?? 0));
  let x = r() * w.reduce((s, v) => s + v, 0), i = 0;
  while (i < live.length - 1 && (x -= w[i]) > 0) i++;
  const c = live[i];
  const side: "buy" | "sell" = r() < 0.62 ? "buy" : "sell";
  const scale = Number(demoCurveOf(c.collection.slug).THRESHOLD) / 85e9;
  const sol = Math.max(0.01 * scale, Math.min(25 * scale, 0.35 * scale * logn(r, 1.1)));
  const trader = pickTrader(r);
  return { at, kind: side, coin: { address: c.mint, symbol: c.symbol, name: c.name, logoUrl: c.logoUrl, collection: c.collection.slug }, text: activityText(side, c.symbol, sol, trader, c.quote.symbol), solLamports: lam(sol), trader, tx: null };
}
