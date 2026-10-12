/**
 * The client for Rook's Solana indexer (docs/v9/indexer-solana.md). Its base URL is the build's `VITE_ROOK_INDEXER`; empty, the
 * site has no indexer and reads the curve straight from the RPC (the board), with the chart, the tape and the holders saying they
 * need the indexer. Read-only JSON over GET; nothing here holds a key.
 */
import { quoteOf } from "./graduation";
import type { IxActivity, IxBoard, IxChart, IxCoin, IxCoinDetail, IxCoinList, IxHolderPage, IxRes, IxSort, IxTradePage } from "./indexerTypes";
import type { MarketCoin } from "./market";

const viteEnv = (import.meta as unknown as { env?: Record<string, string | undefined> }).env ?? {};
/** The indexer's base URL, without a trailing slash; "" when the build names none. Only https (or a local http) is accepted. */
export function indexerBase(raw: string | undefined | null): string {
  const v = (raw ?? "").trim().replace(/\/+$/, "");
  return /^https:\/\/[^\s/]+|^http:\/\/(127\.0\.0\.1|localhost)(:\d+)?/.test(v) ? v : "";
}
export const INDEXER: string = indexerBase(viteEnv.VITE_ROOK_INDEXER);

export class IndexerError extends Error { constructor(public status: number, message: string) { super(message); } }

export async function ixGet<T>(base: string, path: string, timeoutMs = 10_000): Promise<T> {
  if (!base) throw new IndexerError(0, "no indexer in this build");
  const ctl = new AbortController(), t = setTimeout(() => ctl.abort(), timeoutMs);
  try {
    const r = await fetch(`${base}${path}`, { signal: ctl.signal, headers: { accept: "application/json" }, referrerPolicy: "no-referrer" });
    const body = (await r.json().catch(() => null)) as (T & { error?: string }) | null;
    if (!r.ok || body === null) throw new IndexerError(r.status, body && typeof body.error === "string" ? body.error : `the indexer answered ${r.status}`);
    return body;
  } finally { clearTimeout(t); }
}

const q = (o: Record<string, string | number | null | undefined>) => {
  const s = Object.entries(o).filter(([, v]) => v !== null && v !== undefined && v !== "").map(([k, v]) => `${k}=${encodeURIComponent(String(v))}`).join("&");
  return s ? `?${s}` : "";
};
/** The paths, as §2 names them. */
export const IX = {
  board: () => "/v1/rook/board",
  coins: (o: { sort?: IxSort; collection?: string; q?: string; limit?: number; offset?: number } = {}) => `/v1/rook/coins${q(o)}`,
  coin: (mint: string) => `/v1/rook/coins/${encodeURIComponent(mint)}`,
  chart: (mint: string, res: IxRes) => `/v1/rook/coins/${encodeURIComponent(mint)}/chart${q({ res })}`,
  trades: (mint: string, limit = 50, before?: string | null) => `/v1/rook/coins/${encodeURIComponent(mint)}/trades${q({ limit, before })}`,
  holders: (mint: string, limit = 20, offset = 0) => `/v1/rook/coins/${encodeURIComponent(mint)}/holders${q({ limit, offset: offset || null })}`,
  activity: (limit = 50) => `/v1/rook/activity${q({ limit })}`,
};

export const fetchBoard = (base: string) => ixGet<IxBoard>(base, IX.board());
export const fetchCoins = (base: string, o: Parameters<typeof IX.coins>[0]) => ixGet<IxCoinList>(base, IX.coins(o));
export const fetchCoin = (base: string, mint: string) => ixGet<IxCoinDetail>(base, IX.coin(mint));
export const fetchChart = (base: string, mint: string, res: IxRes) => ixGet<IxChart>(base, IX.chart(mint, res));
export const fetchTrades = (base: string, mint: string, before?: string | null, limit = 50) => ixGet<IxTradePage>(base, IX.trades(mint, limit, before));
export const fetchHolders = (base: string, mint: string, offset = 0, limit = 20) => ixGet<IxHolderPage>(base, IX.holders(mint, limit, offset));
export const fetchActivity = (base: string, limit = 50) => ixGet<IxActivity>(base, IX.activity(limit));

/** An indexer row as the market view holds it (the same object; the type only widens what may be null). Rows the contract
 *  says are always there but a misbehaving service left out are mended to "not known" rather than crashing a page. */
export function coinFromIndexer(c: IxCoin): MarketCoin {
  const slug = c.collection?.slug ?? c.nest?.slug ?? "";
  const collection = { slug, listingId: c.collection?.listingId ?? c.nest?.listingId ?? null };
  return {
    ...c, mint: c.mint || c.address, address: c.address || c.mint, collection, nest: { ...collection, name: null, symbol: null }, quote: quoteOf(slug),
    links: c.links ?? { website: null, twitter: null, telegram: null },
    volume24hLamports: c.volume24hLamports ?? null, volume24hSol: typeof c.volume24hSol === "number" ? c.volume24hSol : null,
    trades24h: typeof c.trades24h === "number" ? c.trades24h : null, buys24h: c.buys24h ?? null, sells24h: c.sells24h ?? null, spark: null,
  };
}

/** The holder page's "more": none past the last row (§2.8). */
export const nextHolderOffset = (p: Pick<IxHolderPage, "offset" | "items" | "total" | "limit">): number | null => (p.offset + p.items.length >= p.total || !p.items.length ? null : p.offset + p.items.length);
/** The market cap of a candle's price: price × supplyRaw / 10^decimals (§2.6). */
export const supplyOf = (supplyRaw: string | null, decimals: number): number | null => (supplyRaw ? Number(BigInt(supplyRaw)) / 10 ** decimals : null);
