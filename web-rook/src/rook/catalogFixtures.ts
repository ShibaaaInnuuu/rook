/** The pure half of the catalogue's source (docs/v6/collections-catalog.md §4): the fixture set. No React, no Vite, so a test can hold it. */
import type { CatalogEntry } from "./catalogTypes";
/** `image(entry, item)`: where the page loads a collection's picture (item null) or the i-th sample item's from; null means draw a mark. */
export type CatalogSrc = { kind: "live" | "fixture"; note: string; get: <T>(path: string) => Promise<T>; image: (e: CatalogEntry, item?: number | null) => string | null };
type Index = { kind: string; note: string; generatedAt: number; files: Record<string, string> };
export const imagePath = (slug: string, item: number | null | undefined) => `/v1/rook/catalog/${slug}${item === null || item === undefined ? "" : `/items/${item}`}/image`;
/** The URL the catalogue holds for a picture: the collection's marketplace logo, or a sample item's picture at its source. */
export const imageUrlOf = (e: CatalogEntry, item: number | null | undefined): string | null => (item === null || item === undefined ? e.image : e.sample[item]?.image ?? null);

/** In the labelled fixture there is no indexer to fetch through, so the page loads each picture straight from the public URL the seed recorded (the CSP allows https images), with the mark as the fallback. */
export async function catalogFixtures(read: (f: string) => Promise<unknown> = async (f) => (await fetch(`/fixtures/catalog/${f}`)).json()): Promise<CatalogSrc> {
  const index = (await read("index.json")) as Index;
  const lower = new Map(Object.entries(index.files).map(([k, v]) => [k.toLowerCase(), v]));
  return { kind: "fixture", note: index.note, get: async <T,>(path: string) => {
    const file = lower.get(path.toLowerCase());
    if (!file) throw new Error(`the fixture has no answer for ${path}`);
    return (await read(file)) as T;
  }, image: imageUrlOf };
}
