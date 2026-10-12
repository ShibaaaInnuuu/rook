import { useQuery } from "@tanstack/react-query";
import { API_BASE, httpSource } from "../api/source";
import { useEnv } from "../lib/env";
import type { Catalog, CatalogOne } from "./catalogTypes";

/**
 * Where the catalogue reads from, and never a third place (docs/v6/collections-catalog.md §4): the hosted indexer's
 * /v1/rook/catalog, or the `catalog` fixture set (web-rook/public/fixtures/catalog, a byte copy of indexer/fixtures/catalog)
 * under ?demo=sample. The fixture's times are not shifted: every figure in it is a dated record and is shown with its date.
 *
 * Pictures: live, the indexer's image route (it fetches, checks and caches them; the browser never talks to a marketplace); in the
 * labelled fixture, the public URL the seed recorded, loaded by the page itself. `image()` is null when there is no picture to
 * offer, and the page draws a mark; a picture that fails to load falls back to the mark too.
 */
export { catalogFixtures, imageUrlOf, type CatalogSrc } from "./catalogFixtures";
import { catalogFixtures, imagePath, imageUrlOf, type CatalogSrc } from "./catalogFixtures";
async function liveCatalog(): Promise<CatalogSrc | null> {
  if (!API_BASE) return null;
  try { const src = httpSource(API_BASE); const c = await src.get<Catalog>("/v1/rook/catalog"); return Array.isArray(c.entries) ? { kind: "live", note: c.source.note, get: src.get, image: (e, item) => (imageUrlOf(e, item) ? `${API_BASE}${imagePath(e.slug, item)}` : null) } : null; } catch { return null; }
}
export function useCatalogSource() {
  const env = useEnv();
  return useQuery({ queryKey: ["catalog-source", env?.mode], enabled: env !== null, staleTime: Infinity, refetchInterval: false,
    queryFn: async (): Promise<CatalogSrc | null> => (env?.mode === "sample" ? catalogFixtures() : liveCatalog()) });
}
export function useCatalog() {
  const s = useCatalogSource();
  const q = useQuery({ queryKey: ["catalog", s.data?.kind], enabled: !!s.data, retry: 0, refetchInterval: s.data?.kind === "live" ? 60_000 : false, queryFn: () => s.data!.get<Catalog>("/v1/rook/catalog") });
  return { source: s.data ?? null, sourceLoading: s.isLoading, ...q };
}
export function useCatalogEntry(slug?: string) {
  const s = useCatalogSource();
  const q = useQuery({ queryKey: ["catalog-one", s.data?.kind, slug], enabled: !!s.data && !!slug, retry: 0, refetchInterval: s.data?.kind === "live" ? 60_000 : false, queryFn: () => s.data!.get<CatalogOne>(`/v1/rook/catalog/${slug}`) });
  return { source: s.data ?? null, sourceLoading: s.isLoading, ...q };
}
