import { useQuery } from "@tanstack/react-query";
import { items, reader, statsOf, type StripStats } from "../api/adapt";
import type * as Api from "../api/types";
import type { Source } from "../api/source";
import { useEnv } from "./env";
import type { CoinRow } from "./coinlist";

export type { CapIndex, CoinRow, CoinSort, Hit, TrendKey, VaultPicks } from "./coinlist";
export { byVolume24h, COIN_SORTS, filterCoins, matchCoins, matchVaults, search, sharedSupply, sortCoins, trendShortlist, volume24hOf, vaultPicks, wholeCoins } from "./coinlist";

/**
 * The lists the catalogue pages render: coins, acquisitions, draws and the platform's totals.
 *
 * docs/phase7.md: the indexer's list endpoints take limit, offset and a listing id, and nothing else. There is no sort and no
 * filter, so both happen here, in the browser, over one page of at most `PAGE` rows. Every page that sorts says which slice it
 * sorted. A field the source cannot give is `null` and is printed as unknown, never as zero.
 */
export const PAGE = 200;


const big = (s: string | null | undefined) => BigInt(s ?? "0");
/** A wei field the source may not have at all, kept apart from `big`, which reads a missing figure as zero. */
const maybeBig = (s: string | null | undefined) => (s === null || s === undefined ? null : BigInt(s));
const rowOf = (c: Api.CoinSummary): CoinRow => ({ address: c.address, name: c.name ?? "unnamed", symbol: c.symbol ?? "?", listingId: c.listingId, listingName: c.listingName,
  creator: c.creator, router: c.router, launchedAt: c.launchedAt, lifetimeFeesWei: big(c.lifetimeFeesWei), fees24hWei: big(c.fees24hWei), harvests: c.harvests,
  toVaultWei: big(c.toVaultWei), creatorTaxBps: c.creatorTaxBps, logoUrl: c.logoUrl ?? null, lastHarvestAt: c.lastHarvestAt ?? null,
  supplyWei: c.supplyWei ?? null,
  // docs/phase14.md §4: absent on an indexer older than that phase and on a demo recorded before it, which is "not reported"
  // and not "nothing traded". Only a field actually present becomes a number here.
  volume24hWei: maybeBig((c as { volume24hWei?: Api.Wei | null }).volume24hWei) });

/** The source a page can read, when there is one: the hosted indexer, a recorded demo of it, or nothing. */
export function useSource(): Source | null {
  const env = useEnv();
  if (!env || env.mode === "none") return null;
  if (env.mode === "sample" || env.mode === "api") return env.source;
  return ("api" in env ? env.api : undefined) ?? null;
}

/** True when there is a chain to read but no indexer: the catalogue pages need one and say so. */
export function useNeedsIndexer(): boolean {
  const env = useEnv();
  return !!env && (env.mode === "live" || env.mode === "demo") && !("api" in env && env.api);
}

export function useStats() {
  const src = useSource();
  return useQuery({ queryKey: ["stats", src?.label], enabled: !!src, retry: 0, refetchInterval: 60_000,
    queryFn: async (): Promise<StripStats | null> => {
      try { const s = await reader(src!).stats(); return typeof s.coins === "number" && typeof s.acquisitions?.inscriptions === "number" ? statsOf(s) : null; } catch { return null; }
    } });
}

/** The whole Stats object, for the page that reports every one of its numbers. */
export function useRawStats() {
  const src = useSource();
  return useQuery({ queryKey: ["stats-raw", src?.label], enabled: !!src, retry: 0, refetchInterval: 60_000, queryFn: () => reader(src!).stats() });
}

export function useCoinRows() {
  const src = useSource();
  return useQuery({ queryKey: ["coin-rows", src?.label], enabled: !!src, queryFn: async () => {
    const page = await src!.get<Api.Page<Api.CoinSummary> | Api.CoinSummary[]>(`/v1/coins?limit=${PAGE}`);
    const total = Array.isArray(page) ? page.length : page.total;
    return { rows: items(page).map(rowOf), total };
  } });
}

export function useAcquisitionRows() {
  const src = useSource();
  return useQuery({ queryKey: ["acquisitions", src?.label], enabled: !!src, queryFn: async () => {
    const page = await src!.get<Api.Page<Api.Acquisition> | Api.Acquisition[]>(`/v1/acquisitions?limit=${PAGE}`);
    return { rows: items(page), total: Array.isArray(page) ? page.length : page.total };
  } });
}

export function useDrawRows() {
  const src = useSource();
  return useQuery({ queryKey: ["draws", src?.label], enabled: !!src, queryFn: async () => {
    const page = await src!.get<Api.Page<Api.DrawSummary> | Api.DrawSummary[]>(`/v1/draws?limit=${PAGE}`);
    return { rows: items(page), total: Array.isArray(page) ? page.length : page.total };
  } });
}

export function useListingRows() {
  const src = useSource();
  return useQuery({ queryKey: ["listing-rows", src?.label], enabled: !!src, queryFn: async () =>
    items(await src!.get<Api.Page<Api.ListingSummary> | Api.ListingSummary[]>("/v1/listings")).sort((a, b) => a.id - b.id) });
}
