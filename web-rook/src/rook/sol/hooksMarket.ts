/**
 * The market hooks: the coin list, the cross-coin activity feed, and a coin's chart, tape, holders and indexer row. One source per
 * build, in this order: `?demo` (fixtures from src/rook/sol/demoMarket.ts, with a client-side stream of invented trades), the
 * indexer (`VITE_ROOK_INDEXER`, docs/v9/indexer-solana.md), else the RPC reads the board already makes (no history: the chart, the
 * tape, the holders and the feed say they need the indexer). If the indexer fails, the list falls back to the RPC.
 */
import { useEffect, useMemo, useRef, useState } from "react";
import { useInfiniteQuery, useQuery } from "@tanstack/react-query";
import { DEMO, SITE, useRows, useSolUsd } from "./hooks";
import { CANISTER_ON } from "./hooksCanister";
import { demoActivity, demoChart, demoHolders, demoMarket, demoStreamItem, demoTrades } from "./demoMarket";
import { coinFromIndexer, fetchActivity, fetchBoard, fetchChart, fetchCoin, fetchHolders, fetchTrades, INDEXER, nextHolderOffset } from "./indexer";
import type { IxActivityItem, IxHolderPage, IxRes, IxTradePage } from "./indexerTypes";
import { coinFromRow, type MarketCoin, type MarketSource } from "./market";

export const HAS_HISTORY = DEMO || !!INDEXER;
/** The RPC board reads the site config's configs once it is complete, or the canister's Open collections' (opened on demand) once it names one. */
const RPC_BOARD = SITE.ready || CANISTER_ON;
export const MARKET_SOURCE: MarketSource = DEMO ? "demo" : INDEXER ? "indexer" : RPC_BOARD ? "rpc" : "none";

export type Market = { coins: MarketCoin[]; source: MarketSource; loading: boolean; error: unknown; usdPerSol: number | null; total: number | null; more: (() => void) | null; loadingMore: boolean };

export function useMarket(): Market {
  const px = useSolUsd();
  const demo = useQuery({ queryKey: ["demo", "market"], enabled: DEMO, staleTime: Infinity, refetchInterval: false, queryFn: async () => demoMarket() });
  const ix = useQuery({ queryKey: ["ix-board"], enabled: !DEMO && !!INDEXER, refetchInterval: 10_000, staleTime: 5_000, retry: 1, queryFn: async () => fetchBoard(INDEXER) });
  const viaRpc = !DEMO && (!INDEXER || !!ix.error) && RPC_BOARD;
  const { list, rows } = useRows(viaRpc);
  const usd = ix.data?.usdPerSol ?? px.data?.price ?? null;
  const rpcCoins = useMemo(() => (rows.data?.pages.flat() ?? []).map((r) => coinFromRow(r, usd)), [rows.data, usd]);
  const ixCoins = useMemo(() => ix.data?.rows.map(coinFromIndexer) ?? [], [ix.data]);
  if (DEMO) return { coins: demo.data ?? [], source: "demo", loading: demo.isLoading, error: demo.error, usdPerSol: px.data?.price ?? null, total: demo.data?.length ?? null, more: null, loadingMore: false };
  if (INDEXER && !ix.error) return { coins: ixCoins, source: "indexer", loading: ix.isLoading, error: null, usdPerSol: usd, total: ix.data?.rows.length ?? null, more: null, loadingMore: false };
  if (!RPC_BOARD) return { coins: [], source: "none", loading: false, error: null, usdPerSol: usd, total: 0, more: null, loadingMore: false };
  return {
    coins: rpcCoins, source: "rpc", loading: list.isLoading || rows.isLoading, error: list.error ?? rows.error ?? null, usdPerSol: usd, total: list.data?.length ?? null,
    more: rows.hasNextPage ? () => void rows.fetchNextPage() : null, loadingMore: rows.isFetchingNextPage,
  };
}

/** A feed row, keyed for React; `live` marks one that arrived after the page opened (it flashes in). */
export type FeedItem = IxActivityItem & { key: string; live: boolean };
const keyOf = (a: IxActivityItem, i: number) => `${a.at ?? 0}:${a.kind}:${a.tx ?? a.coin.address}:${a.trader ?? ""}:${i}`;

/** The cross-coin activity feed, newest first. Demo: the fixtures' recent history, then an invented trade every 1–3.5 s. */
export function useActivity(coins: MarketCoin[], limit = 60): { items: FeedItem[]; source: MarketSource } {
  const ix = useQuery({ queryKey: ["ix-activity", limit], enabled: !DEMO && !!INDEXER, refetchInterval: 5_000, staleTime: 3_000, retry: 1, queryFn: async () => fetchActivity(INDEXER, limit) });
  const [stream, setStream] = useState<FeedItem[]>(() => (DEMO ? demoActivity(limit).items.map((a, i) => ({ ...a, key: keyOf(a, i), live: false })) : []));
  const coinsRef = useRef(coins);
  coinsRef.current = coins;
  useEffect(() => {
    if (!DEMO) return;
    let n = 0, t: ReturnType<typeof setTimeout>;
    const tick = () => {
      const a = demoStreamItem(coinsRef.current);
      if (a) setStream((s) => [{ ...a, key: `live:${n++}`, live: true }, ...s].slice(0, limit));
      t = setTimeout(tick, 1_000 + Math.random() * 2_500);
    };
    t = setTimeout(tick, 900);
    return () => clearTimeout(t);
  }, [limit]);
  const prev = useRef<Set<string>>(new Set());
  const ixItems = useMemo(() => {
    const seen = new Set<string>();
    const items = (ix.data?.items ?? []).map((a, i) => {
      let k = keyOf(a, 0);
      if (seen.has(k)) k = keyOf(a, i);
      seen.add(k);
      return { ...a, key: k, live: prev.current.size > 0 && !prev.current.has(k) };
    });
    prev.current = new Set(items.map((x) => x.key));
    return items;
  }, [ix.data]);
  if (DEMO) return { items: stream, source: "demo" };
  if (INDEXER) return { items: ixItems, source: "indexer" };
  return { items: [], source: MARKET_SOURCE };
}

/** The coin's indexer row (24 h figures, holders, trades all time); null without history. */
export function useMarketCoin(mint: string | undefined) {
  return useQuery({
    queryKey: [DEMO ? "demo" : "ix", "market-coin", mint], enabled: !!mint && HAS_HISTORY, refetchInterval: DEMO ? false : 10_000, staleTime: DEMO ? Infinity : 5_000, retry: 1,
    queryFn: async (): Promise<(MarketCoin & { tradesAllTime: number | null }) | null> => {
      if (DEMO) { const c = demoMarket().find((x) => x.mint === mint); return c ? { ...c, tradesAllTime: demoTrades(mint!)?.total ?? null } : null; }
      const d = await fetchCoin(INDEXER, mint!);
      return { ...coinFromIndexer(d), tradesAllTime: d.tradesAllTime };
    },
  });
}
export function useCoinChart(mint: string | undefined, res: IxRes) {
  return useQuery({
    queryKey: [DEMO ? "demo" : "ix", "chart", mint, res], enabled: !!mint && HAS_HISTORY, retry: 1, staleTime: DEMO ? Infinity : 4_000,
    refetchInterval: DEMO ? false : res === "1m" ? 5_000 : 15_000,
    queryFn: async () => (DEMO ? demoChart(mint!, res) : fetchChart(INDEXER, mint!, res)),
  });
}
export function useCoinTrades(mint: string | undefined, limit = 50) {
  return useInfiniteQuery({
    queryKey: [DEMO ? "demo" : "ix", "trades", mint], enabled: !!mint && HAS_HISTORY, retry: 1, refetchInterval: DEMO ? false : 8_000,
    initialPageParam: null as string | null, getNextPageParam: (last: IxTradePage | null) => last?.next ?? null,
    queryFn: async ({ pageParam }) => (DEMO ? demoTrades(mint!, pageParam, limit) : fetchTrades(INDEXER, mint!, pageParam, limit)),
  });
}
export function useCoinHolders(mint: string | undefined, enabled: boolean, limit = 20) {
  return useInfiniteQuery({
    queryKey: [DEMO ? "demo" : "ix", "holders", mint], enabled: !!mint && enabled && HAS_HISTORY, retry: 1, refetchInterval: DEMO ? false : 30_000,
    initialPageParam: 0, getNextPageParam: (last: IxHolderPage | null) => (last ? nextHolderOffset(last) : null),
    queryFn: async ({ pageParam }) => (DEMO ? demoHolders(mint!, pageParam, limit) : fetchHolders(INDEXER, mint!, pageParam, limit)),
  });
}
