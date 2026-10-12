import { useInfiniteQuery, useQuery } from "@tanstack/react-query";
import { API_BASE, httpSource } from "../api/source";
import type { Chart, HolderPage, TradePage } from "../api/types";
import { useEnv } from "../lib/env";
import { useSource } from "../lib/catalog";
import type { Activity, Board, RookChart, RookHolderPage, RookKind, RookRes, RookTradePage } from "./apiTypes";
import { PHASE1, activityFor, boardFor } from "./phase";

/**
 * Where the board reads from, and never a third place: the hosted indexer's /v1/rook endpoints, or the `rook` fixture set
 * (web-rook/public/fixtures/rook, a byte copy of indexer/fixtures/rook) under ?demo=sample. In the fixture a pons coin's
 * chart, tape and holders come from the demo recording's own endpoints, reshaped, because that recording is where they are.
 */
export { rookFixtures, shiftTimes, type RookSrc } from "./fixtures";
import { rookFixtures, shiftTimes, type RookSrc } from "./fixtures";

async function liveRook(): Promise<RookSrc | null> {
  if (!API_BASE) return null;
  try { const src = httpSource(API_BASE); const b = await src.get<Board>("/v1/rook/board"); return b.rows ? { kind: "live", note: b.source.note, shiftS: 0, get: src.get } : null; } catch { return null; }
}
export function useRookSource() {
  const env = useEnv();
  return useQuery({ queryKey: ["rook-source", env?.mode], enabled: env !== null, staleTime: Infinity, refetchInterval: false,
    queryFn: async (): Promise<RookSrc | null> => env?.mode === "sample" ? rookFixtures(Math.floor(Date.now() / 1000)) : liveRook() });
}
function useR<T>(key: unknown[], path: string | null, every = 15_000) {
  const s = useRookSource();
  const q = useQuery({ queryKey: ["rook", s.data?.kind, ...key], enabled: !!s.data && path !== null, refetchInterval: s.data?.kind === "live" ? every : false, retry: 0, queryFn: () => s.data!.get<T>(path!) });
  return { source: s.data ?? null, sourceLoading: s.isLoading, ...q };
}
/** Every coin, pons coins included: the coin page reads this, so a link to a pons coin says what it is instead of "no such coin". */
export const useBoardAll = () => useR<Board>(["board"], "/v1/rook/board");
/** The board as the lists show it: in a Phase-1 build without pons coins (docs/v6/phase1-scope.md §8.3). */
export const useBoard = () => { const q = useBoardAll(); return PHASE1 && q.data ? { ...q, data: boardFor(q.data, true) } : q; };
export const useActivity = () => { const q = useR<Activity>(["activity"], "/v1/rook/activity?limit=50", 8_000); return PHASE1 && q.data ? { ...q, data: activityFor(q.data, true) } : q; };

/** The resolution tabs, and the nearest recorded window when a pons coin is read from the demo recording. */
export const RESES: [RookRes, string][] = [["1m", "1m"], ["5m", "5m"], ["15m", "15m"], ["1h", "1h"], ["1d", "1d"]];
const RANGE_OF: Record<RookRes, "5m" | "1h" | "6h" | "1d" | "all"> = { "1m": "1h", "5m": "6h", "15m": "1d", "1h": "all", "1d": "all" };

export function useRookChart(kind: RookKind | null, address: string | undefined, res: RookRes) {
  const s = useRookSource(), demo = useSource();
  const viaDemo = s.data?.kind === "fixture" && kind === "pons";
  return useQuery({ queryKey: ["rook-chart", s.data?.kind, kind, address, res], enabled: !!s.data && !!address && !!kind, retry: 0, refetchInterval: s.data?.kind === "live" ? (res === "1m" ? 5_000 : 15_000) : false,
    queryFn: async (): Promise<RookChart> => {
      if (viaDemo && demo) {
        const c = shiftTimes(await demo.get<Chart>(`/v1/coins/${address}/candles?range=${RANGE_OF[res]}`), s.data!.shiftS);
        return { source: { kind: "fixture", note: s.data!.note }, kind: "pons", coin: c.coin, res, seconds: c.seconds, span: c.span, quote: "ETH", candles: c.candles, trades: c.trades, tradesAllTime: c.tradesAllTime,
          launchedAt: c.launchedAt, graduation: c.graduation, supplyWei: c.supplyWei, usdPerEth: c.usdPerEth };
      }
      return s.data!.get<RookChart>(`/v1/rook/coins/${address}/chart?res=${res}`);
    } });
}
export function useRookTrades(kind: RookKind | null, address: string | undefined, limit = 25) {
  const s = useRookSource(), demo = useSource();
  const viaDemo = s.data?.kind === "fixture" && kind === "pons";
  return useInfiniteQuery({ queryKey: ["rook-trades", s.data?.kind, kind, address], enabled: !!s.data && !!address && !!kind, retry: 0, refetchInterval: s.data?.kind === "live" ? 8_000 : false,
    initialPageParam: null as string | null, getNextPageParam: (last: RookTradePage) => last.next,
    queryFn: async ({ pageParam }): Promise<RookTradePage> => {
      const q = `limit=${limit}${pageParam ? `&before=${encodeURIComponent(pageParam)}` : ""}`;
      if (viaDemo && demo) {
        const p = shiftTimes(await demo.get<TradePage>(`/v1/coins/${address}/trades?${q}`), s.data!.shiftS);
        return { source: { kind: "fixture", note: s.data!.note }, kind: "pons", items: p.items.map((t) => ({ id: t.id, side: t.side, venue: t.venue, quoteWei: t.quoteWei, coinWei: t.coinWei, feeWei: t.feeWei, taxWei: t.taxWei, price: t.price, trader: t.trader, tx: t.tx, block: t.block, logIndex: t.logIndex, at: t.at })), next: p.next, total: p.total };
      }
      return s.data!.get<RookTradePage>(`/v1/rook/coins/${address}/trades?${q}`);
    } });
}
export function useRookHolders(kind: RookKind | null, address: string | undefined, enabled: boolean, limit = 25) {
  const s = useRookSource(), demo = useSource();
  const viaDemo = s.data?.kind === "fixture" && kind === "pons";
  return useInfiniteQuery({ queryKey: ["rook-holders", s.data?.kind, kind, address], enabled: !!s.data && !!address && !!kind && enabled, retry: 0, refetchInterval: s.data?.kind === "live" ? 30_000 : false,
    initialPageParam: 0, getNextPageParam: (last: RookHolderPage) => (last.offset + last.items.length >= last.total ? null : last.offset + last.limit),
    queryFn: async ({ pageParam }): Promise<RookHolderPage> => {
      const q = `limit=${limit}${pageParam ? `&offset=${pageParam}` : ""}`;
      if (viaDemo && demo) {
        const h = await demo.get<HolderPage>(`/v1/coins/${address}/holders?${q}`);
        return { source: { kind: "fixture", note: s.data!.note }, kind: "pons", coin: h.coin, holders: h.holders, supplyWei: h.supplyWei, complete: h.complete,
          items: h.items.map((x) => ({ rank: x.rank, address: x.address, balanceWei: x.balanceWei, share: x.share, label: x.isCreator ? "creator" : null })), total: h.total, limit: h.limit, offset: h.offset };
      }
      return s.data!.get<RookHolderPage>(`/v1/rook/coins/${address}/holders?${q}`);
    } });
}
