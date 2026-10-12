import { useQuery } from "@tanstack/react-query";
import { API_BASE, httpSource } from "../../api/source";
import { useEnv } from "../../lib/env";
import type { CallLog, DrawReport, EthReserveReport, MetadataSnapshot } from "./core/types";

/**
 * docs/v8: where the draw page and the reserves page's Ethereum section read from, and never a third place:
 *   - the hosted indexer's /v1/v8/* (VITE_ETCH_API), when it names collections: live;
 *   - the indexer's fixture set `v8` (web-rook/public/fixtures/v8, a byte copy of indexer/fixtures/v8), under ?demo=sample only,
 *     labelled FIXTURE. Reserve reports are moved forward by the set's age (a fixture's green must be fresh to stay green: the
 *     status is re-judged at every render); draw proofs are NOT moved (their times are pinned to real drand rounds).
 * Everything a page concludes (a draw verified, the odds, a status) is recomputed here from the data, by core/ — the indexer's own code.
 */
export { shiftDraws, shiftReserve, v8Fixtures, type V8Src } from "./fixtures";
import { v8Fixtures, type V8Src } from "./fixtures";

async function live(): Promise<V8Src | null> {
  if (!API_BASE) return null;
  try {
    const src = httpSource(API_BASE);
    const l = await src.get<{ collections: string[] }>("/v1/v8");
    return l.collections?.length ? { kind: "live", note: "", shiftS: 0, get: src.get } : null;
  } catch { return null; }
}
export function useV8Source() {
  const env = useEnv();
  return useQuery({ queryKey: ["v8-source", env?.mode], enabled: env !== null, staleTime: Infinity, refetchInterval: false,
    queryFn: async (): Promise<V8Src | null> => (env?.mode === "sample" ? v8Fixtures(Math.floor(Date.now() / 1000)) : live()) });
}
function useV8<T>(key: unknown[], path: string | null, every = 60_000) {
  const s = useV8Source();
  const q = useQuery({ queryKey: ["v8", s.data?.kind, ...key], enabled: !!s.data && path !== null, retry: 0, refetchInterval: s.data?.kind === "live" ? every : false, queryFn: () => s.data!.get<T>(path!) });
  return { source: s.data ?? null, sourceLoading: s.isLoading, ...q };
}
export const useEthReserve = (slug?: string) => useV8<EthReserveReport>(["reserve", slug], slug ? `/v1/v8/reserves/${slug}` : null);
export const useEthReserves = () => {
  const s = useV8Source();
  return useQuery({ queryKey: ["v8", s.data?.kind, "all"], enabled: !!s.data, retry: 0, queryFn: async () => {
    const l = await s.data!.get<{ collections: string[] }>("/v1/v8");
    return Promise.all(l.collections.map((c) => s.data!.get<EthReserveReport>(`/v1/v8/reserves/${c}`)));
  } });
};
/** The collections the redemption draw page covers (`/v1/v8`), so a page links to a draw page only where one exists. */
export const useV8Collections = () => useV8<{ collections: string[] }>(["list"], "/v1/v8");
export const useDraws = (slug?: string) => useV8<DrawReport>(["draws", slug], slug ? `/v1/v8/draws/${slug}` : null);
export const useSnapshot = (slug?: string) => useV8<MetadataSnapshot>(["snapshot", slug], slug ? `/v1/v8/snapshots/${slug}` : null, 3_600_000);
/** Fixture mode only: a recording of the "check it yourself" run over the fixture's chain (the fixture's hosts do not exist). */
export const useRecordedSelfCheck = (slug: string | undefined, enabled: boolean) =>
  useV8<{ report: EthReserveReport; calls: CallLog[]; hosts: { eth: string[]; home: string[] } }>(["selfcheck", slug], enabled && slug ? `selfcheck/${slug}` : null);
