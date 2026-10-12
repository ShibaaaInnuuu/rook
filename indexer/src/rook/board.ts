import type * as T from "../../types.js";
import { candles, type Candle, type Point } from "../candles.js";
import type { ActivityItem, BoardRow, RookRes, RookTrade } from "./types.js";

/** The pure half of the board: prices from trades, rows from either kind of coin, the ticker's items. No chain, no database, no clock of its own. */
export const DAY = 86_400;
export const RES_SECONDS: Record<RookRes, number> = { "1m": 60, "5m": 300, "15m": 900, "1h": 3_600, "1d": DAY };
/** 240 buckets of the resolution; a day's buckets cover the coin's whole life (one month of them is longer than anything here has lived) */
export const spanOf = (res: RookRes): number => (res === "1d" ? 30 * DAY : RES_SECONDS[res] * 240);
export const resOf = (s: string | null | undefined): RookRes | null => (s === "1m" || s === "5m" || s === "15m" || s === "1h" || s === "1d" ? s : null);

const toN = (wei: string | bigint) => Number(BigInt(wei)) / 1e18;
export const sumWei = (xs: readonly string[]): string => String(xs.reduce((a, x) => a + BigInt(x), 0n));

/** What a series of trades (chain order) says about a coin at `now`: the last price, the price a day earlier, the day's volume and count. */
export function marketOf(trades: readonly RookTrade[], now: number): { price: number | null; change24h: number | null; volume24hWei: string; trades24h: number; buys24h: number } {
  const last = trades.length ? trades[trades.length - 1] : null;
  const since = now - DAY;
  const day = trades.filter((t) => t.at !== null && t.at >= since);
  const before = [...trades].reverse().find((t) => t.at !== null && t.at < since) ?? null;
  // the reference price is the last trade before the window, or the first in it: with only one price there is no change to report
  const ref = before ?? (day.length > 1 ? day[0] : null);
  const change = last && ref && ref.price > 0 && ref !== last ? (last.price - ref.price) / ref.price : null;
  return { price: last ? last.price : null, change24h: change, volume24hWei: sumWei(day.map((t) => t.quoteWei)), trades24h: day.length, buys24h: day.filter((t) => t.side === "buy").length };
}

export const capOf = (supplyWei: string | null, price: number | null): number | null => (supplyWei && price !== null && price > 0 ? toN(supplyWei) * price : null);

export const chartOf = (trades: readonly RookTrade[], res: RookRes, now: number): { candles: Candle[]; trades: number; seconds: number; span: number } => {
  const seconds = RES_SECONDS[res], span = spanOf(res), since = now - span;
  const pts: Point[] = trades.filter((t) => t.at !== null && t.at >= since).map((t) => ({ ts: t.at!, p: t.price, v: toN(t.quoteWei) }));
  return { candles: candles(pts, seconds), trades: pts.length, seconds, span };
};

/** A pons coin's row: the list endpoint's own fields, plus what its trades say. */
export function ponsRow(c: T.CoinSummary, m: { price: number | null; change24h: number | null; trades24h: number | null; buys24h: number | null; newHolders24h: number | null; graduatedAt: number | null }): BoardRow {
  return { kind: "pons", address: c.address, curve: c.curve, name: c.name ?? "unnamed", symbol: c.symbol ?? "?", logoUrl: c.logoUrl, creator: c.creator,
    nest: { slug: null, listingId: c.listingId, name: c.listingName, symbol: null }, policy: null, feeBps: c.creatorTaxBps === null ? null : 70 + c.creatorTaxBps,
    phase: m.graduatedAt !== null ? "graduated" : "trading", launchedAt: c.launchedAt, graduatedAt: m.graduatedAt, raise: null,
    price: m.price, change24h: m.change24h, marketCapEth: capOf(c.supplyWei, m.price), supplyWei: c.supplyWei, volume24hWei: c.volume24hWei ?? null, trades24h: m.trades24h, buys24h: m.buys24h,
    holders: c.holders, newHolders24h: m.newHolders24h, sweptWei: c.toVaultWei };
}

/** A Rook coin's row: the collection's record of it, plus what its curve's logs say. */
export function curveRow(col: Pick<T.WrappedCollection, "slug" | "name" | "symbol">, coin: T.WrappedCoin, r: { trades: readonly RookTrade[]; holders: number | null; newHolders24h: number | null; graduatedAt: number | null; feeBps: number | null; supplyWei: string | null } | null, now: number): BoardRow {
  const m = r ? marketOf(r.trades, now) : null;
  return { kind: "curve", address: coin.address, curve: coin.curve, name: coin.name, symbol: coin.symbol, logoUrl: null, creator: coin.creator,
    nest: { slug: col.slug, listingId: null, name: col.name, symbol: col.symbol }, policy: coin.policy, feeBps: r?.feeBps ?? null,
    phase: coin.phase, launchedAt: coin.launchedAt, graduatedAt: r?.graduatedAt ?? null,
    raise: coin.phase === "trading" || coin.phase === "graduating" ? { raisedWei: coin.raisedWei ?? "0", raiseWei: coin.raiseWei } : null,
    price: m?.price ?? null, change24h: m?.change24h ?? null, marketCapEth: capOf(r?.supplyWei ?? null, m?.price ?? null), supplyWei: r?.supplyWei ?? null,
    volume24hWei: m ? m.volume24hWei : null, trades24h: m?.trades24h ?? null, buys24h: m?.buys24h ?? null, holders: r?.holders ?? null, newHolders24h: r?.newHolders24h ?? null, sweptWei: null };
}

/** The ticker's items from a coin's trades and a nest's activity, newest first. */
export function tradeItems(row: Pick<BoardRow, "address" | "symbol" | "kind" | "nest">, trades: readonly RookTrade[]): ActivityItem[] {
  return trades.map((t) => ({ at: t.at, kind: t.side, coin: { address: row.address, symbol: row.symbol, kind: row.kind }, nest: row.nest.name,
    text: `${t.side === "buy" ? "bought" : "sold"} ${row.symbol} for ${toN(t.quoteWei).toFixed(4)} ETH`, amountWei: t.quoteWei, tx: t.tx }));
}
export function nestItems(col: Pick<T.WrappedCollection, "name" | "activity">): ActivityItem[] {
  const kind = (k: T.WrappedActivity["kind"]): ActivityItem["kind"] | null => (k === "raffle-won" ? "raffle-won" : k === "burned" ? "burned" : k === "arrived" ? "arrived" : null);
  return col.activity.flatMap((a) => { const k = kind(a.kind); return k ? [{ at: a.at, kind: k, coin: null, nest: col.name, text: a.text, amountWei: null, tx: a.tx }] : []; });
}
export const launchItem = (row: BoardRow): ActivityItem | null => row.launchedAt === null ? null
  : { at: row.launchedAt, kind: "launch", coin: { address: row.address, symbol: row.symbol, kind: row.kind }, nest: row.nest.name, text: `${row.symbol} launched on ${row.nest.name ?? "its nest"}`, amountWei: null, tx: null };
export const graduationItem = (row: BoardRow): ActivityItem | null => row.graduatedAt === null ? null
  : { at: row.graduatedAt, kind: "graduated", coin: { address: row.address, symbol: row.symbol, kind: row.kind }, nest: row.nest.name, text: `${row.symbol} graduated to its pool`, amountWei: null, tx: null };
export const newestFirst = (items: ActivityItem[], limit: number): ActivityItem[] => [...items].sort((a, b) => (b.at ?? 0) - (a.at ?? 0)).slice(0, limit);
