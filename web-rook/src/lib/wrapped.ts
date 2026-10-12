import { useQuery } from "@tanstack/react-query";
import type { HandinReport, ReserveReport, Ticket, WrappedCollection, WrappedList } from "../api/types";
import { API_BASE, httpSource } from "../api/source";
import { useEnv } from "./env";
import { wrappedFixtures, type WSource } from "./wrappedSource";
export { marketVenues, shiftTimes, wrappedFixtures, type WSource } from "./wrappedSource";

/**
 * docs/v5/ux-and-reserves.md. Where the wrapped-collection pages read from, and never a fourth place:
 *   - the hosted indexer (VITE_ETCH_API), when it says it has wrapped collections configured: live;
 *   - the indexer's fixture set `wrapped` (web/public/fixtures/wrapped, a byte-for-byte copy), only under ?demo=sample, and every
 *     page then shows a FIXTURE banner with the set's own note;
 *   - nowhere: the pages say nothing is deployed and offer the fixture.
 */
async function liveSource(): Promise<WSource | null> {
  if (!API_BASE) return null;
  try {
    const src = httpSource(API_BASE);
    const list = await src.get<WrappedList>("/v1/wrapped");
    return list.configured ? { kind: "live", note: list.source?.note ?? "", shiftS: 0, get: src.get } : null;
  } catch { return null; }
}

export function useWrappedSource() {
  const env = useEnv();
  return useQuery({ queryKey: ["wrapped-source", env?.mode], enabled: env !== null, staleTime: Infinity, refetchInterval: false,
    queryFn: async (): Promise<WSource | null> => env?.mode === "sample" ? wrappedFixtures(Math.floor(Date.now() / 1000)) : liveSource() });
}
function useW<T>(key: unknown[], path: string | null) {
  const s = useWrappedSource();
  const q = useQuery({ queryKey: ["wrapped", s.data?.kind, ...key], enabled: !!s.data && path !== null, refetchInterval: 30_000, queryFn: () => s.data!.get<T>(path!) });
  return { source: s.data ?? null, sourceLoading: s.isLoading, ...q };
}
export const useWrappedList = () => useW<WrappedList>(["list"], "/v1/wrapped");
export const useWrappedCollection = (slug?: string) => useW<WrappedCollection>(["collection", slug], slug ? `/v1/wrapped/${slug}` : null);
export const useReserves = () => useW<{ configured: boolean; reports: ReserveReport[] }>(["reserves"], "/v1/reserves");
export const useReserve = (slug?: string) => useW<ReserveReport>(["reserve", slug], slug ? `/v1/reserves/${slug}` : null);
export const useHandins = (slug?: string) => useW<HandinReport>(["handins", slug], slug ? `/v1/handins/${slug}` : null);
export const useTicket = (slug?: string, id?: number) => useW<Ticket>(["ticket", slug, id], slug && id !== undefined && Number.isInteger(id) ? `/v1/wrapped/${slug}/tickets/${id}` : null);

/** The collection a v3 coin belongs to, or null when no configured collection lists it (then it is not a wrapped-collection coin). */
export function useWrappedCoin(address?: string) {
  const s = useWrappedSource(), list = useWrappedList();
  return useQuery({ queryKey: ["wrapped-coin", s.data?.kind, address, list.data?.collections.length], enabled: !!s.data && !!list.data && !!address,
    queryFn: async () => {
      for (const c of list.data!.collections) {
        const d = await s.data!.get<WrappedCollection>(`/v1/wrapped/${c.slug}`);
        const coin = d.coinsList.find((x) => x.address.toLowerCase() === address!.toLowerCase());
        if (coin) return { collection: d, coin };
      }
      return null;
    } });
}

