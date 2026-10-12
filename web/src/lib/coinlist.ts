/**
 * The list logic the catalogue pages run in the browser: the shape of a coin row, the sorts and filters the chips offer, and what
 * the search box can answer. It is here, apart from the hooks, because the indexer's list endpoints take limit, offset and a
 * listing id and nothing else (docs/phase7.md), and because logic worth testing should be testable without a browser.
 */
import type * as Api from "../api/types";

export type CoinRow = {
  address: `0x${string}`; name: string; symbol: string; listingId: number; listingName: string | null;
  creator: `0x${string}`; router: `0x${string}`;
  /** null when a chain read answered instead of the indexer: it knows the coin, not its history */
  launchedAt: number | null; lifetimeFeesWei: bigint | null; fees24hWei: bigint | null; harvests: number | null;
  toVaultWei: bigint; creatorTaxBps: number | null;
  /** the logo its creator gave at launch, as the chain has it; the site chooses the gateway (lib/plate.ts) */
  logoUrl?: string | null;
  /** when its fees were last swept into the vault, for the activity feed */
  lastHarvestAt?: number | null;
  /** docs/phase12.md §3: the coin's whole supply in base units, read once at launch. null when the read has not landed. */
  supplyWei?: string | null;
  /**
   * docs/phase14.md §4: the pair asset traded in the last 24 hours, in wei. `null` means the source did not report any —
   * an indexer older than that phase, or a demo recorded before it — which is NOT the same fact as a quiet day, where the
   * indexer says `0`. A caller that meets null must rank and label by something else and say which.
   */
  volume24hWei?: bigint | null;
};

// ---- sorting and filtering, in the browser (docs/phase7.md) ----

/** docs/phase14.md §3. `cap` is supply × the latest curve-side price, which needs a price the list endpoint has not got: it
 *  is passed in, and a row with no cap sorts last rather than as nothing. */
export type CoinSort = "newest" | "cap" | "fees24h" | "fees" | "harvests" | "name";
export const COIN_SORTS: [CoinSort, string][] = [["newest", "Newest"], ["cap", "Market cap"], ["fees24h", "Fees, 24 hours"], ["fees", "Fees routed"], ["harvests", "Most harvested"], ["name", "Name"]];

const cmpBig = (a: bigint | null, b: bigint | null) => (b ?? -1n) > (a ?? -1n) ? 1 : (b ?? -1n) < (a ?? -1n) ? -1 : 0;
/** a number that is not known sorts last, whichever direction the sort runs: -1 is below every real market cap and volume */
const cmpNum = (a: number | null | undefined, b: number | null | undefined) => (b ?? -1) - (a ?? -1);

/** A market cap per coin address, in the PAIR asset, as `lib/caps.ts` measures it. Keys are lower-cased addresses. */
export type CapIndex = ReadonlyMap<string, number | null>;
const capOfRow = (caps: CapIndex | undefined, c: CoinRow) => caps?.get(c.address.toLowerCase()) ?? null;

export function sortCoins(rows: CoinRow[], by: CoinSort, caps?: CapIndex): CoinRow[] {
  const out = [...rows];
  if (by === "name") return out.sort((a, b) => a.name.localeCompare(b.name));
  if (by === "fees") return out.sort((a, b) => cmpBig(a.lifetimeFeesWei, b.lifetimeFeesWei) || a.name.localeCompare(b.name));
  if (by === "fees24h") return out.sort((a, b) => cmpBig(a.fees24hWei, b.fees24hWei) || cmpBig(a.lifetimeFeesWei, b.lifetimeFeesWei));
  // docs/phase14.md §3: ranked on the unconverted number, so a dollar rate that goes stale cannot reorder the list
  if (by === "cap") return out.sort((a, b) => cmpNum(capOfRow(caps, a), capOfRow(caps, b)) || a.name.localeCompare(b.name));
  // how often its fees have actually been swept, which is the nearest thing the list endpoint has to market activity
  if (by === "harvests") return out.sort((a, b) => (b.harvests ?? -1) - (a.harvests ?? -1) || cmpBig(a.lifetimeFeesWei, b.lifetimeFeesWei));
  return out.sort((a, b) => (b.launchedAt ?? 0) - (a.launchedAt ?? 0) || a.name.localeCompare(b.name));
}

/**
 * docs/phase14.md §3. The one supply every coin on the page shares, or null when they do not share one.
 *
 * Pons mints a coin's whole supply at launch and mints the same amount every time, which is why market cap, fully diluted
 * value and price all order a list identically and why there is one sort pill rather than three. That claim is checked here
 * against the rows on screen on every render rather than taken on trust from the document: a coin that ever launches with a
 * different supply makes this null, and the note on the page says the orderings come apart.
 */
export function sharedSupply(rows: CoinRow[]): string | null {
  let one: string | null = null;
  for (const c of rows) {
    if (!c.supplyWei) continue;
    if (one === null) one = c.supplyWei;
    else if (one !== c.supplyWei) return null;
  }
  return one;
}

/** Whole coins from a supply in base units, for the sentence that prints it. Null when it is not a number. */
export function wholeCoins(supplyWei: string | null): number | null {
  if (!supplyWei) return null;
  const n = Number(supplyWei) / 1e18;
  return Number.isFinite(n) && n > 0 ? n : null;
}

// ---- docs/phase14.md §4: what "trending" is allowed to mean here --------------------------------------------------------

/** Which number drew the shortlist. `volume24h` is exact; the others are named on screen so nothing is mislabelled. */
export type TrendKey = "volume24h" | "fees24h" | "fees" | "newest";

/**
 * The shortlist the Trending strip orders, and the key that drew it.
 *
 * The strip prints 24-hour volume and orders by it. Volume for EVERY coin is one field on the list response
 * (`volume24hWei`), and where an indexer serves it the shortlist is already the answer. Where it does not — an indexer older
 * than this phase, or a recorded demo — the strip falls back to measuring volume from each shortlisted coin's own candles,
 * which is one request per coin, so the shortlist has to be short and has to be drawn by the best key the list response
 * actually carries. That key is returned so the page can say which one it was.
 */
export function trendShortlist(rows: CoinRow[], take = 16): { key: TrendKey; rows: CoinRow[] } {
  const has = (pick: (c: CoinRow) => boolean) => rows.some(pick);
  const key: TrendKey = has((c) => (c.volume24hWei ?? null) !== null) ? "volume24h"
    : has((c) => (c.fees24hWei ?? 0n) > 0n) ? "fees24h"
    : has((c) => (c.lifetimeFeesWei ?? 0n) > 0n) ? "fees"
    : "newest";
  const by: Record<TrendKey, CoinSort> = { volume24h: "newest", fees24h: "fees24h", fees: "fees", newest: "newest" };
  const ranked = key === "volume24h"
    ? [...rows].sort((a, b) => cmpBig(a.volume24hWei ?? null, b.volume24hWei ?? null) || a.name.localeCompare(b.name))
    : sortCoins(rows, by[key]);
  return { key, rows: ranked.slice(0, take) };
}

/** 24-hour volume in whole pair-asset units: the indexer's own field where it has one, else what the candles measured. */
export const volume24hOf = (c: CoinRow, measured?: ReadonlyMap<string, number | null>): number | null =>
  (c.volume24hWei ?? null) !== null ? Number(c.volume24hWei) / 1e18 : measured?.get(c.address.toLowerCase()) ?? null;

/** The strip's own order: the number it prints, descending, with a coin whose volume is unknown last. */
export const byVolume24h = (rows: CoinRow[], measured?: ReadonlyMap<string, number | null>): CoinRow[] =>
  [...rows].sort((a, b) => cmpNum(volume24hOf(a, measured), volume24hOf(b, measured)) || a.name.localeCompare(b.name));

// ---- docs/phase14.md §1: the vault control, which must not grow with the number of vaults --------------------------------

/** What the compact vault control shows on the row, and what it keeps behind the panel. */
export type VaultPicks = { shown: Api.ListingSummary[]; rest: Api.ListingSummary[] };

/**
 * The vaults a filter should offer first, and the rest.
 *
 * Ranked by whether a coin actually pays into the vault (a filter offering vaults nothing is paired with offers empty lists),
 * then by how many coins do, then by the ETH that has reached it, then by id so the order never wobbles. `selected` is always
 * among `shown`, however far down the ranking it sits: a grid filtered by something the reader cannot see is worse than a
 * long row.
 */
export function vaultPicks(vaults: Api.ListingSummary[], selected: number | null, limit = 8): VaultPicks {
  const wei = (v: Api.ListingSummary) => { try { return BigInt(v.depositedWei); } catch { return 0n; } };
  const ranked = [...vaults].sort((a, b) =>
    (b.coins > 0 ? 1 : 0) - (a.coins > 0 ? 1 : 0) || b.coins - a.coins || (wei(b) > wei(a) ? 1 : wei(b) < wei(a) ? -1 : 0) || a.id - b.id);
  const shown = ranked.slice(0, limit);
  const rest = ranked.slice(limit);
  const at = rest.findIndex((v) => v.id === selected);
  if (at >= 0) shown.push(rest.splice(at, 1)[0]);
  return { shown, rest };
}

/** The panel's own search, over every vault: a name, a slug, or a rune id. An empty query is the whole list, not none of it. */
export function matchVaults(q: string, vaults: Api.ListingSummary[]): Api.ListingSummary[] {
  const s = q.trim().toLowerCase();
  if (!s) return vaults;
  return vaults.filter((v) => v.name.toLowerCase().includes(s) || v.name.replace(/•/g, "").toLowerCase().includes(s) || v.ref.toLowerCase().includes(s));
}

export const filterCoins = (rows: CoinRow[], listing: number | null, earning: boolean): CoinRow[] =>
  rows.filter((c) => (listing === null || c.listingId === listing) && (!earning || (c.lifetimeFeesWei ?? 0n) > 0n));

/** A name, a ticker or an address, matched over the page already fetched. */
export function matchCoins(rows: CoinRow[], q: string): CoinRow[] {
  const s = q.trim().toLowerCase();
  if (!s) return rows;
  return rows.filter((c) => c.name.toLowerCase().includes(s) || c.symbol.toLowerCase().includes(s) || c.address.toLowerCase().includes(s));
}

/** What a search box can answer without an endpoint: a name, a symbol, an address, a Bitcoin transaction id, or a rune id. */
export type Hit = {
  kind: "coin" | "listing" | "receipt" | "offered";
  label: string; sub: string; to: string;
  /** printed as a chip, so a vault and a collection with no vault can never be mistaken for one another in a dropdown */
  state?: "vault" | "no vault";
};

/** The catalogue's shape, as the search needs it. Kept structural so the search stays testable without loading the file. */
export type Offered = { id: string; slug: string; name: string; kind: "inscriptions" | "rune"; runeId?: string };

export function search(
  q: string,
  coins: CoinRow[],
  listings: Api.ListingSummary[],
  receipts: Api.Acquisition[],
  offered: Offered[] = [],
): Hit[] {
  const s = q.trim().toLowerCase();
  if (s.length < 2) return [];
  const hits: Hit[] = [];
  for (const c of coins) {
    if (c.name.toLowerCase().includes(s) || c.symbol.toLowerCase().includes(s) || c.address.toLowerCase().includes(s))
      hits.push({ kind: "coin", label: `${c.name} ${c.symbol}`, sub: c.listingName ?? `listing ${c.listingId}`, to: `coin/${c.address}` });
  }
  for (const l of listings) {
    if (l.name.toLowerCase().includes(s) || l.ref.toLowerCase().includes(s))
      hits.push({ kind: "listing", label: l.name, sub: l.kind === "rune" ? "rune" : "inscriptions", state: "vault", to: `listing/${l.id}` });
  }
  for (const a of receipts) {
    if (a.btcTxid.toLowerCase().includes(s) || a.assetRef.toLowerCase().includes(s))
      hits.push({ kind: "receipt", label: `purchase ${a.listingId}-${a.acquisitionId}`, sub: a.btcTxid.slice(0, 16), to: `listing/${a.listingId}` });
  }
  // What is listed on the chain always outranks what is merely available to list, so the two are never confused.
  const listedRefs = new Set(listings.map((l) => l.ref.toLowerCase()));
  for (const o of offered) {
    if (hits.length >= 12) break;
    const flat = o.name.replace(/•/g, "").toLowerCase();
    if (!(o.name.toLowerCase().includes(s) || flat.includes(s) || o.slug.includes(s) || o.runeId === s)) continue;
    if (listedRefs.has(o.slug.toLowerCase()) || listedRefs.has(o.runeId?.toLowerCase() ?? "\u0000")) continue;
    hits.push({
      kind: "offered",
      label: o.name,
      sub: o.kind === "rune" ? "rune" : "collection",
      state: "no vault",
      to: `vaults/${o.slug}`,
    });
  }
  return hits.slice(0, 8);
}
