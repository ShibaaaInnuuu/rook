/**
 * The market view the terminal, the coins board and the coin page share: the indexer's coin row (docs/v9/indexer-solana.md §2.2)
 * with the 24 h fields allowed to be unknown, because without an indexer the site reads the curve straight from the RPC and has no
 * history (no 24 h change, no volume, no trades, no holders). Pure: test/market.test.ts is its spec.
 */
import type { CoinRow } from "./board";
import { quoteOf, thresholdOf } from "./graduation";
import type { IxCandle, IxCoin, IxTrade } from "./indexerTypes";

/** What a coin is quoted in (canister-only Rook: its collection's wX; ROOK: SOL). Every `*Sol` / `*Lamports` figure of a MarketCoin
 *  is in this quote's units: a wX-quoted board reads as wX. USD and SOL views need the collection's wX/SOL pair, which the board does
 *  not read: `marketCapUsd` and `priceUsd` are null for a wX-quoted coin unless the source (the indexer, the demo) supplied them. */
export type Quote = { symbol: string; decimals: number };
/** IxCoin, with what only an indexer knows allowed to be null; `spark` is a few closing prices over the last day (demo only). */
export type MarketCoin = Omit<IxCoin, "volume24hLamports" | "volume24hSol" | "trades24h" | "buys24h" | "sells24h"> & {
  quote: Quote;
  volume24hLamports: string | null; volume24hSol: number | null; trades24h: number | null; buys24h: number | null; sells24h: number | null;
  spark?: number[] | null;
};
export type MarketSource = "demo" | "indexer" | "rpc" | "none";

const lamText = (l: bigint) => l.toString();

/** A coin read straight from the RPC (src/rook/sol/board.ts's row) in the market shape; everything historical is null. */
export function coinFromRow(r: CoinRow, usdPerSol: number | null): MarketCoin {
  const s = r.stats, grad = s.phase === "graduated";
  const fin = Number(r.pool_.finishCurveTimestamp);
  const price = s.priceSol;
  const collection = { slug: r.slug, listingId: null };
  const quote = quoteOf(r.slug), inSol = quote.symbol === "SOL";
  return {
    kind: "dbc", address: r.mint, mint: r.mint, pool: r.pool, dammPool: null, config: r.config, quote,
    collection, nest: { ...collection, name: null, symbol: null },
    name: r.meta?.name || "Unnamed", symbol: r.meta?.symbol || "?", logoUrl: null, description: null,
    links: { website: null, twitter: null, telegram: null }, metadataUri: r.meta?.uri || null, creator: r.creator,
    decimals: 6, supplyRaw: s.supply ? String(BigInt(Math.round(s.supply)) * 1_000_000n) : null,
    phase: grad ? "graduated" : s.phase === "graduating" ? "graduating" : "trading", graduated: grad,
    launchedAt: r.launchedAt || null, graduatedAt: grad && fin ? fin : null, lastTradeAt: null,
    curve: grad ? null : { raisedLamports: lamText(s.raisedLamports), thresholdLamports: lamText(s.thresholdLamports), progress: s.progress },
    raise: grad ? null : { raisedLamports: lamText(s.raisedLamports), raiseLamports: lamText(s.thresholdLamports) },
    price, priceUsd: inSol && price !== null && usdPerSol ? price * usdPerSol : null, change24h: null,
    marketCapSol: s.marketCapSol, marketCapUsd: inSol && s.marketCapSol !== null && usdPerSol ? s.marketCapSol * usdPerSol : null,
    volume24hLamports: null, volume24hSol: null, volume24hUsd: null, trades24h: null, buys24h: null, sells24h: null,
    holders: null, holdersAt: null, spark: null,
  };
}

export const progressOf = (c: Pick<MarketCoin, "phase" | "curve">): number => (c.phase === "graduated" ? 1 : c.phase === "graduating" ? 1 : Math.max(0, Math.min(1, c.curve?.progress ?? 0)));
export const raisedSol = (c: Pick<MarketCoin, "curve">): number | null => (c.curve ? Number(c.curve.raisedLamports) / 1e9 : null);

/** "About to graduate" starts here (share of the coin's own threshold raised: its collection's, in whole wX). */
export const SOON = 0.6;
export type Columns = { fresh: MarketCoin[]; soon: MarketCoin[]; done: MarketCoin[] };
/** The terminal's three columns: New (on the curve, below SOON, newest first), About to graduate (SOON and up, and the curves
 *  that are full and migrating, furthest along first), Graduated (newest graduation first). Every coin is in exactly one. */
export function columnsOf(coins: MarketCoin[]): Columns {
  const fresh: MarketCoin[] = [], soon: MarketCoin[] = [], done: MarketCoin[] = [];
  for (const c of coins) (c.phase === "graduated" ? done : c.phase === "graduating" || progressOf(c) >= SOON ? soon : fresh).push(c);
  fresh.sort((a, b) => (b.launchedAt ?? 0) - (a.launchedAt ?? 0) || (a.mint < b.mint ? -1 : 1));
  soon.sort((a, b) => progressOf(b) - progressOf(a) || (a.mint < b.mint ? -1 : 1));
  done.sort((a, b) => (b.graduatedAt ?? b.launchedAt ?? 0) - (a.graduatedAt ?? a.launchedAt ?? 0) || (a.mint < b.mint ? -1 : 1));
  return { fresh, soon, done };
}

/** Trending: 24 h volume, then trades; without history (RPC reads) the market cap stands in. */
export function trendingOf(coins: MarketCoin[], n = 12): MarketCoin[] {
  const hist = coins.some((c) => c.volume24hSol !== null);
  return coins.slice().sort((a, b) => hist
    ? (b.volume24hSol ?? 0) - (a.volume24hSol ?? 0) || (b.trades24h ?? 0) - (a.trades24h ?? 0)
    : (b.marketCapSol ?? 0) - (a.marketCapSol ?? 0)).slice(0, n);
}

/** Top market cap: the biggest coins by market cap (USD when known, else SOL), graduated or not. */
export function topByMcapOf(coins: MarketCoin[], n = 12): MarketCoin[] {
  return coins.slice().sort((a, b) => (b.marketCapUsd ?? 0) - (a.marketCapUsd ?? 0) || (b.marketCapSol ?? 0) - (a.marketCapSol ?? 0)).slice(0, n);
}

/** The terminal's filters: one collection (or "rook"), and a search over name, ticker, collection slug, or an exact mint. */
export function filterCoins(coins: MarketCoin[], f: { collection?: string; q?: string }): MarketCoin[] {
  const q = (f.q ?? "").trim().toLowerCase();
  return coins.filter((c) => (!f.collection || c.collection.slug === f.collection)
    && (!q || c.mint.toLowerCase() === q || c.name.toLowerCase().includes(q) || c.symbol.toLowerCase().includes(q) || c.collection.slug.includes(q)));
}

export type MarketStats = { coins: number; graduated: number; volume24hSol: number | null; trades24h: number | null; backingWx: number; backingSolAtFloors: number; next: MarketCoin[] };
/** The hero's figures. The backing is the graduated collection coins' 50 % share of their own threshold, in whole wX across the
 *  collections (`backingWx`) and valued at the day-1 floors (`backingSolAtFloors`); ROOK has none. `volume24hSol` sums each
 *  coin's volume in its own quote, so it is a count of quote units, not SOL, on a mixed board: the pages say which. */
export function marketStats(coins: MarketCoin[]): MarketStats {
  const hist = coins.some((c) => c.volume24hSol !== null);
  const graduated = coins.filter((c) => c.phase === "graduated");
  const shares = graduated.map((c) => thresholdOf(c.collection.slug)).filter((t): t is NonNullable<typeof t> => t !== null);
  return {
    coins: coins.length, graduated: graduated.length,
    volume24hSol: hist ? coins.reduce((s, c) => s + (c.volume24hSol ?? 0), 0) : null,
    trades24h: hist ? coins.reduce((s, c) => s + (c.trades24h ?? 0), 0) : null,
    backingWx: shares.reduce((s, t) => s + t.backingWx, 0),
    backingSolAtFloors: shares.reduce((s, t) => s + t.backingWx * t.floorSol, 0),
    next: coins.filter((c) => c.phase !== "graduated").sort((a, b) => progressOf(b) - progressOf(a)).slice(0, 3),
  };
}

/** OHLCV buckets from trades (oldest first): empty buckets left out, each candle opening at the previous close (§2.6). */
export function candlesFromTrades(trades: Pick<IxTrade, "price" | "sol" | "at">[], seconds: number, from = -Infinity, to = Infinity): IxCandle[] {
  const out: IxCandle[] = [];
  let prev: number | null = null;
  const sorted = trades.filter((t) => t.at !== null).slice().sort((a, b) => a.at! - b.at!);
  for (const t of sorted) {
    const at = t.at!;
    if (at < from || at > to) { prev = at < from ? t.price : prev; continue; }
    const b = Math.floor(at / seconds) * seconds;
    let k = out[out.length - 1];
    if (!k || k.t !== b) { const o: number = prev ?? t.price; k = { t: b, o, h: Math.max(o, t.price), l: Math.min(o, t.price), c: t.price, v: 0, n: 0 }; out.push(k); }
    k.h = Math.max(k.h, t.price); k.l = Math.min(k.l, t.price); k.c = t.price; k.v += t.sol; k.n += 1;
    prev = t.price;
  }
  return out;
}

/** n closing prices evenly over [from, to], carried forward from the last trade before each point (a sparkline). */
export function sparkOf(trades: Pick<IxTrade, "price" | "at">[], from: number, to: number, n = 24): number[] {
  const sorted = trades.filter((t) => t.at !== null).slice().sort((a, b) => a.at! - b.at!);
  const out: number[] = [];
  let i = 0, last = sorted[0]?.price ?? 0;
  for (let k = 0; k < n; k++) {
    const at = from + ((to - from) * k) / (n - 1);
    while (i < sorted.length && sorted[i].at! <= at) last = sorted[i++].price;
    out.push(last);
  }
  return out;
}

/** "8Xk3…f3Qa bought 2.1 wMILADY of $MOOD": an activity line's amount, short. */
export const solShort = (sol: number): string => (sol >= 100 ? sol.toFixed(0) : sol >= 10 ? sol.toFixed(1) : sol >= 1 ? sol.toFixed(2) : sol >= 0.01 ? sol.toFixed(3) : sol.toPrecision(2));
/** A market cap or volume, compact: $12.4k / 312 SOL / 2.5 wMILADY. `unit` is "$" or the quote's symbol. */
export function compact(v: number | null | undefined, unit: string): string {
  if (v === null || v === undefined || !Number.isFinite(v)) return "—";
  const a = Math.abs(v), s = a >= 1e9 ? `${(v / 1e9).toFixed(2)}B` : a >= 1e6 ? `${(v / 1e6).toFixed(2)}M` : a >= 1e4 ? `${(v / 1e3).toFixed(1)}k` : a >= 1e3 ? `${(v / 1e3).toFixed(2)}k` : a >= 10 ? v.toFixed(0) : a >= 1 ? v.toFixed(2) : v.toPrecision(2);
  return unit === "$" ? `$${s}` : `${s} ${unit}`;
}
export const changeText = (c: number | null | undefined): string => (c === null || c === undefined || !Number.isFinite(c) ? "—" : `${c >= 0 ? "+" : "−"}${Math.abs(c * 100).toFixed(Math.abs(c) < 0.1 ? 1 : 0)}%`);
/** A coin's price, quote per coin (2.8e-8 → "0.0000000280"): significant digits, never scientific. */
export function priceText(p: number | null | undefined): string {
  if (p === null || p === undefined || !Number.isFinite(p) || p <= 0) return "—";
  const dp = Math.max(2, Math.min(14, 3 - Math.floor(Math.log10(p))));
  return p.toFixed(dp);
}
const SUB = "₀₁₂₃₄₅₆₇₈₉";
/** A tiny price the way DEX screens print it: 0.0000001429 → "0.0₆1429" (the subscript counts the zeros after the point). */
export function priceShort(p: number | null | undefined): string {
  if (p === null || p === undefined || !Number.isFinite(p) || p <= 0) return "—";
  if (p >= 0.001) return priceText(p);
  const zeros = -Math.floor(Math.log10(p)) - 1;
  const digits = Math.round(p * 10 ** (zeros + 4)).toString().slice(0, 4);
  return `0.0${String(zeros).split("").map((d) => SUB[+d]).join("")}${digits}`;
}
/** Whole coins, compact: 12.5M. */
export const tokensText = (n: number): string => (n >= 1e9 ? `${(n / 1e9).toFixed(2)}B` : n >= 1e6 ? `${(n / 1e6).toFixed(2)}M` : n >= 1e3 ? `${(n / 1e3).toFixed(1)}k` : n.toFixed(n < 10 ? 2 : 0));
