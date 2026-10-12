/**
 * The tracked-collection list (#/tracked) on the Solana site: the collections catalogue (docs/v6/collections-catalog.md), read from
 * the hosted indexer when this build names one (VITE_ETCH_API) and otherwise from the catalogue's committed, dated seed records
 * (public/fixtures/catalog, a byte copy of indexer/fixtures/catalog). No wallet code, no wagmi: the Robinhood design's list page
 * (src/pages/Collection.tsx) needed both and an indexer, so on a build without one it said "Nothing is deployed yet".
 *
 * What changes from that page: Robinhood Chain's own entries are dropped (that design stopped), and an entry's standing is
 * whether Rook launches coins on it on Solana (one of Rook's collections, matched by contract or by slug), not the Robinhood vault rule.
 * Pure (no React), so test/tracked.test.ts can hold it.
 */
import type { Catalog, CatalogChain, CatalogEntry } from "../catalogTypes";
import { COLLECTIONS, type SolCollection } from "./collections";

export type TrackedTab = "all" | Exclude<CatalogChain, "robinhood">;
export const TRACKED_TABS: [TrackedTab, string][] = [["all", "All"], ["bitcoin", "Bitcoin"], ["ethereum", "Ethereum"], ["base", "Base"], ["other", "Other"]];
export type TrackedSort = "rook" | "floor" | "items" | "owners" | "name";
export const TRACKED_SORTS: [TrackedSort, string][] = [["rook", "Rook first"], ["floor", "Floor"], ["owners", "Holders"], ["items", "NFTs"], ["name", "Name"]];

export type TrackedRow = {
  e: CatalogEntry;
  /** the Rook collection this entry is, when it is one of Rook's collections */
  rook: SolCollection | null;
  /** where the row goes: the Rook collection's page, else the collection at its marketplace (or explorer, or website) */
  link: string | null;
  external: boolean;
};

/** Which of Rook's collections an entry is: an Ethereum contract match, or the same slug (the Ordinals collections have no contract). */
export function rookOf(e: CatalogEntry, cols: readonly SolCollection[] = COLLECTIONS): SolCollection | null {
  const id = e.id.toLowerCase();
  return cols.find((c) => (c.contract.startsWith("0x") && c.contract.toLowerCase() === id && e.chain === "ethereum") || c.slug === e.slug) ?? null;
}

/** The catalogue's entries as the Solana site lists them: Robinhood Chain's dropped, each with its link. `hrefOf` makes an in-site link. */
export function trackedRows(entries: CatalogEntry[], hrefOf: (slug: string) => string, cols: readonly SolCollection[] = COLLECTIONS): TrackedRow[] {
  return entries.filter((e) => e.chain !== "robinhood").map((e) => {
    const rook = rookOf(e, cols);
    if (rook) return { e, rook, link: hrefOf(rook.slug), external: false };
    const out = e.links.marketplace ?? e.links.explorer ?? e.links.website ?? null;
    return { e, rook: null, link: out && /^https:\/\//.test(out) ? out : null, external: true };
  });
}

export const countTabs = (rows: TrackedRow[]): Record<TrackedTab, number> => {
  const out: Record<TrackedTab, number> = { all: rows.length, bitcoin: 0, ethereum: 0, base: 0, other: 0 };
  for (const r of rows) if (r.e.chain !== "robinhood") out[r.e.chain]++;
  return out;
};

const n = (x: number | null | undefined) => (x === null || x === undefined || !Number.isFinite(x) ? -1 : x);
export function viewRows(rows: TrackedRow[], tab: TrackedTab, q: string, sort: TrackedSort): TrackedRow[] {
  const s = q.trim().toLowerCase();
  const out = rows.filter((r) => (tab === "all" || r.e.chain === tab)
    && (!s || r.e.name.toLowerCase().includes(s) || r.e.slug.toLowerCase().includes(s) || r.e.id.toLowerCase().includes(s) || r.e.chainName.toLowerCase().includes(s) || (!!r.rook && "rook".includes(s))));
  const desc = (f: (r: TrackedRow) => number) => (a: TrackedRow, b: TrackedRow) => f(b) - f(a) || a.e.name.localeCompare(b.e.name);
  if (sort === "floor") return out.sort(desc((r) => n(r.e.floor?.eth)));
  if (sort === "items") return out.sort(desc((r) => n(r.e.items)));
  if (sort === "owners") return out.sort(desc((r) => n(r.e.owners)));
  if (sort === "name") return out.sort((a, b) => a.e.name.localeCompare(b.e.name));
  return out.sort((a, b) => (a.rook ? a.rook.listingId : 1e6) - (b.rook ? b.rook.listingId : 1e6) || n(b.e.floor?.eth) - n(a.e.floor?.eth) || a.e.name.localeCompare(b.e.name));
}

/** The newest date any figure in the answer carries (a seed record's date when nothing fresher was read). */
export const newestRecord = (c: Pick<Catalog, "entries" | "refreshedAt">): number | null =>
  c.refreshedAt ?? c.entries.reduce<number | null>((m, e) => (e.floor && (m === null || e.floor.at > m) ? e.floor.at : m), null);
