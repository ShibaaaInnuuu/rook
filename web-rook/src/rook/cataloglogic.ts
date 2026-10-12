import type { Catalog, CatalogChain, CatalogEntry, CatalogStatusKind } from "./catalogTypes";

/** The catalogue's tabs, sorts and search, in the browser over one answer (docs/v6/collections-catalog.md §4). Pure, so a test can hold it. */
export type ChainTab = "all" | CatalogChain;
export const CHAIN_TABS: [ChainTab, string][] = [["all", "All"], ["bitcoin", "Bitcoin"], ["ethereum", "Ethereum"], ["base", "Base"], ["other", "Other"]];
export type CatalogSort = "floor" | "volume" | "items" | "owners" | "status" | "name";
export const CATALOG_SORTS: [CatalogSort, string][] = [["status", "Rook status"], ["floor", "Floor"], ["volume", "7d volume"], ["items", "NFTs"], ["owners", "Holders"], ["name", "Name"]];
export const CHAIN_WORD: Record<CatalogChain, string> = { bitcoin: "Bitcoin", ethereum: "Ethereum", base: "Base", robinhood: "Robinhood Chain", other: "Other" };
export const STANDARD_WORD: Record<CatalogEntry["standard"], string> = { inscriptions: "inscriptions", erc721: "ERC-721", cryptopunks: "CryptoPunks", erc1155: "ERC-1155", other: "other" };
export const STATUS_WORD: Record<CatalogStatusKind, string> = { live: "Live on Rook", listable: "Listable", later: "Later", unsupported: "Unsupported" };
export const STATUS_TONE: Record<CatalogStatusKind, "live" | "good" | "warn" | "bad"> = { live: "live", listable: "good", later: "warn", unsupported: "bad" };
const STATUS_RANK: Record<CatalogStatusKind, number> = { live: 0, listable: 1, later: 2, unsupported: 3 };

export const tabEntries = (rows: CatalogEntry[], tab: ChainTab): CatalogEntry[] => (tab === "all" ? rows : rows.filter((e) => e.chain === tab));
export const countByChain = (rows: CatalogEntry[]): Record<ChainTab, number> => {
  const out: Record<ChainTab, number> = { all: rows.length, bitcoin: 0, ethereum: 0, base: 0, robinhood: 0, other: 0 };
  for (const e of rows) out[e.chain]++;
  return out;
};

export function searchEntries(rows: CatalogEntry[], q: string): CatalogEntry[] {
  const s = q.trim().toLowerCase();
  if (!s) return rows;
  return rows.filter((e) => e.name.toLowerCase().includes(s) || e.slug.toLowerCase().includes(s) || e.id.toLowerCase().includes(s) || e.chainName.toLowerCase().includes(s)
    || STATUS_WORD[e.rook.kind].toLowerCase().includes(s) || (e.rook.kind === "live" && e.rook.slug.toLowerCase().includes(s)));
}

const n = (x: number | null | undefined) => (x === null || x === undefined || !Number.isFinite(x) ? -1 : x);
/** Highest first for a figure; an unknown figure (null) sorts last, never as zero. Status sorts live → listable → later → unsupported, then by floor. */
export function sortEntries(rows: CatalogEntry[], by: CatalogSort): CatalogEntry[] {
  const out = [...rows];
  const desc = (f: (e: CatalogEntry) => number) => (a: CatalogEntry, b: CatalogEntry) => f(b) - f(a) || a.name.localeCompare(b.name);
  if (by === "floor") return out.sort(desc((e) => n(e.floor?.eth)));
  if (by === "volume") return out.sort(desc((e) => n(e.volume7dEth)));
  if (by === "items") return out.sort(desc((e) => n(e.items)));
  if (by === "owners") return out.sort(desc((e) => n(e.owners)));
  if (by === "name") return out.sort((a, b) => a.name.localeCompare(b.name));
  return out.sort((a, b) => STATUS_RANK[a.rook.kind] - STATUS_RANK[b.rook.kind] || n(b.floor?.eth) - n(a.floor?.eth) || a.name.localeCompare(b.name));
}

export const liveEntries = (rows: CatalogEntry[]): CatalogEntry[] => rows.filter((e) => e.rook.kind === "live");
export const liveCount = (c: Catalog | undefined | null): number | null => (c ? liveEntries(c.entries).length : null);
/** The entry a nest's Rook slug names, for the pages that start from a nest. */
export const entryOfNest = (rows: CatalogEntry[], slug: string): CatalogEntry | null => rows.find((e) => e.rook.kind === "live" && e.rook.slug.toLowerCase() === slug.toLowerCase()) ?? null;
/** The wizard can start a launch on a nest that is live (its factory exists) or listable (the wizard walks, the transaction waits for the listing). */
export const launchable = (e: CatalogEntry): boolean => e.rook.kind === "live" || e.rook.kind === "listable";
/** The nests the wizard offers: live first, then listable, then the rest greyed with their status, each group by floor. */
export const wizardOrder = (rows: CatalogEntry[]): CatalogEntry[] => sortEntries(rows, "status");

/** "0.137 ETH (433,069 sats)", "6.85 ETH", "—": a floor in the unit at hand, with the native unit beside a converted one. */
export function floorText(e: CatalogEntry): string {
  const f = e.floor;
  if (!f) return "—";
  if (f.unit === "wei") return `${ethText(f.eth ?? Number(BigInt(f.native) / 10n ** 12n) / 1e6)} ETH`;
  const sats = Number(f.native);
  const native = sats >= 1e8 ? `${(sats / 1e8).toLocaleString("en-US", { maximumFractionDigits: 3 })} BTC` : `${sats.toLocaleString("en-US")} sats`;
  return f.eth === null ? native : `${ethText(f.eth)} ETH · ${native}`;
}
export function ethText(v: number): string {
  if (!Number.isFinite(v)) return "—";
  if (v >= 100) return v.toLocaleString("en-US", { maximumFractionDigits: 0 });
  if (v >= 1) return v.toLocaleString("en-US", { maximumFractionDigits: 2 });
  if (v >= 0.001) return v.toLocaleString("en-US", { maximumFractionDigits: 3 });
  return v.toPrecision(2);
}
export function usdText(eth: number | null | undefined, usdPerEth: number | null): string | null {
  if (eth === null || eth === undefined || usdPerEth === null) return null;
  const v = eth * usdPerEth;
  return v >= 1e6 ? `$${(v / 1e6).toFixed(2)}M` : v >= 1e3 ? `$${(v / 1e3).toFixed(1)}k` : `$${v.toFixed(v >= 100 ? 0 : 2)}`;
}
export const volText = (v: number | null): string => (v === null ? "—" : v === 0 ? "0" : `${ethText(v)} ETH`);
export const countText = (v: number | null): string => (v === null ? "—" : v.toLocaleString("en-US"));

/** When a floor was true, said plainly beside it: a fresh read says its source; a stale one says its date and that it is not the price now. */
export function floorNote(e: CatalogEntry, now: number): string {
  const f = e.floor;
  if (!f) return "No source reported a floor.";
  const src = f.source === "seed" ? "the seed's record" : f.source === "rook" ? "the keeper's cap" : f.source === "ordinalswallet" ? "Ordinals Wallet" : f.source === "magiceden" ? "Magic Eden" : f.source === "opensea" ? "OpenSea" : f.source === "nosh" ? "Nosh (OpenSea's figure)" : f.source;
  const age = Math.max(0, now - f.at);
  const when = age < 3_600 ? `${Math.max(1, Math.floor(age / 60))} min ago` : age < 86_400 ? `${Math.floor(age / 3_600)} h ago` : `${Math.floor(age / 86_400)} d ago`;
  return f.stale ? `From ${src}, ${when}: not a fresh read, and not the price now.` : `From ${src}, ${when}.`;
}

/**
 * The home page's "In the vault now" strip: one picture per NFT a live collection's vault holds, from that collection's own
 * catalogue pictures (the same ones its page shows), as many as there are pictures for; the rest are counted, not drawn. The
 * pictures stand for the collection: which NFTs exactly are held, with their ids and checks, is on the reserves page.
 */
export type VaultTile = { slug: string; name: string; entry: CatalogEntry; item: number };
export function vaultTiles(held: { slug: string; name: string; itemsHeld: number | null }[], entries: CatalogEntry[], max = 12): { tiles: VaultTile[]; held: number; more: number } {
  const tiles: VaultTile[] = [];
  let total = 0;
  for (const c of held) {
    const n = c.itemsHeld ?? 0;
    total += n;
    const e = entryOfNest(entries, c.slug);
    if (!e) continue;
    const pics = e.sample.map((s, i) => (s.image ? i : -1)).filter((i) => i >= 0);
    for (let k = 0; k < Math.min(n, pics.length); k++) tiles.push({ slug: c.slug, name: c.name, entry: e, item: pics[k] });
  }
  const shown = tiles.slice(0, max);
  return { tiles: shown, held: total, more: Math.max(0, total - shown.length) };
}
