import { useInfiniteQuery, useQuery } from "@tanstack/react-query";
import { useSource } from "./catalog";
import type { HolderPage, TradePage } from "../api/types";
import type { CandleSet, Range } from "./chart";

/**
 * A coin's candles, computed by the indexer, polled faster the shorter the range.
 *
 * docs/phase12.md §4: the request names a RANGE and the indexer derives the bucket from it, so nothing a page can ask for
 * lands on one candle per trade.
 */
export function useCandles(address: string | undefined, range: Range) {
  const src = useSource();
  return useQuery({
    queryKey: ["candles", src?.label, address, range],
    enabled: !!src && !!address,
    retry: 0,
    refetchInterval: range === "5m" ? 5_000 : range === "1h" ? 8_000 : 30_000,
    queryFn: () => src!.get<CandleSet>(`/v1/coins/${address}/candles?range=${range}`),
  });
}

export type { HolderPage, TradePage };

/**
 * A coin's trades, newest first, paged by the indexer's "block:logIndex" cursor.
 *
 * The parameter is **`before`** (indexer/src/api.ts). It was sent as `cursor` until docs/phase13.md §1, and an unknown query
 * parameter is ignored rather than refused — so every press of "Older trades" quietly re-read page one, appended the same
 * twenty-five trades again, and offered itself for ever.
 */
export function useTrades(address: string | undefined, limit = 25) {
  const src = useSource();
  return useInfiniteQuery({
    queryKey: ["trades", src?.label, address],
    enabled: !!src && !!address,
    retry: 0,
    refetchInterval: 8_000,
    initialPageParam: null as string | null,
    getNextPageParam: (last: TradePage) => last.next,
    queryFn: ({ pageParam }) =>
      src!.get<TradePage>(`/v1/coins/${address}/trades?limit=${limit}${pageParam ? `&before=${encodeURIComponent(pageParam)}` : ""}`),
  });
}

/**
 * Who holds a coin, largest first, paged by offset. docs/phase13.md §5.
 *
 * Asked for only once a reader opens that tab: it is a second request, and most visitors want the tape. There is no page
 * after the last one — `offset + rows >= total` is the end — so a recorded demo runs out of pages exactly where the
 * recording does, rather than reaching for one nobody recorded.
 */
export function useHolders(address: string | undefined, enabled: boolean, limit = 25) {
  const src = useSource();
  return useInfiniteQuery({
    queryKey: ["holders", src?.label, address],
    enabled: !!src && !!address && enabled,
    retry: 0,
    refetchInterval: 30_000,
    initialPageParam: 0,
    getNextPageParam: (last: HolderPage) => (last.offset + last.items.length >= last.total ? null : last.offset + last.limit),
    queryFn: ({ pageParam }) =>
      src!.get<HolderPage>(`/v1/coins/${address}/holders?limit=${limit}${pageParam ? `&offset=${pageParam}` : ""}`),
  });
}
