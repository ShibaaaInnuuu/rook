/**
 * The catalogue of real Bitcoin assets a coin's fees can be pointed at.
 *
 * It is a file, not an API. `scripts/catalogue.mjs` fetches it by hand and commits it, so a visitor's browser never calls a
 * marketplace: no key ships, no rate limit applies, and nothing here breaks when somebody else's service does. Every price
 * carries the date it was fetched, and every screen that prints one prints that date beside it.
 *
 * **What its order means.** Fifty of each kind, on two bases the file states separately and this site never merges: the runes
 * by the volume traded in the seven days before the fetch, the collections by lifetime volume ACROSS ALL MARKETPLACES.
 * Not by their week: Ordinals Wallet's index does publish one, and it is non-zero on 29 of the 1,076 collections that pass
 * the filters (2.7%, measured 2026-09-22), which is not a top fifty (docs/phase9.md §11.1, §11.2, §11.3). Which field each list was sorted by is in
 * `ranking.<kind>.field`, so `rankingLine` and every card read the fetch's own record rather than assuming a window. A file
 * fetched before that basis was written down carries none, and `rankingLine` says so instead of implying one.
 *
 * docs/phase9.md records which endpoints were called, at what rate, how each list is ranked, and where permission stands.
 */
import { useQuery } from "@tanstack/react-query";

export type CatalogueItem = { id: string; num: number | null; name: string | null };

export type CatalogueCollection = {
  id: string; slug: string; name: string; kind: "inscriptions";
  description: string | null;
  supply: number | null;
  /** the cheapest one listed, in sats, as of `fetchedAt`; never live. Zero where nothing is listed at all. */
  /** sats, or null where nothing is listed: the site prints that as "not known", never as free */
  floorSats: number | null;
  /** the sats traded in the seven days before `fetchedAt`: the figure this list is ranked by. Absent on a file fetched before the basis was recorded. */
  volume7dSats?: number | null;
  /** how many sales that figure is the sum of, when it was summed from a marketplace's fills rather than stated by it */
  trades7d?: number | null;
  /** a lifetime figure, where the source publishes one. NOTHING is sorted by it: it ranks the whole history, not this week. */
  volumeAllTimeSats?: number | null;
  listed: number | null;
  verified: boolean;
  /** the inscription numbers the collection spans, which is how a listing describes what it will buy */
  first: number | null; last: number | null;
  sampleInscription: string | null;
  image: string | null; imageFrom: string | null;
  /**
   * True when this collection is in the file because it was NAMED, not because it placed in the ranking.
   *
   * Ordinals Wallet's paginated index only lists what has traded on Ordinals Wallet, so the best-known Bitcoin
   * collections are absent from all 6,000 rows of it. They are fetched one at a time instead, and they carry no
   * `volumeAllTimeSats`: the per-collection endpoint publishes that venue's own volume, which is not the market-wide
   * figure the ranking uses, and sorting one against the other would rank Bitcoin Puppets below collections a hundredth
   * its size. So they sit after the ranked fifty and say why.
   */
  byName?: boolean;
  /** Ordinals Wallet's own volume, kept so the gap against the market-wide figure is visible rather than argued. */
  volumeOnOrdinalsWalletSats?: number | null;
  items: CatalogueItem[];
  source: string; sourceUrl: string;
};

export type CatalogueRune = {
  id: string; slug: string; name: string; kind: "rune";
  runeId: string; symbol: string | null; divisibility: number;
  supply: string | null; etchedBlock: number; mintable: boolean;
  /** null unless the operator ran the fetch with a marketplace key: a floor needs a marketplace */
  floorSats: number | null; floorSource?: string; holders: number | null;
  /** the sats traded in the seven days before `fetchedAt`. Null when the runes here are a curated list rather than a ranking. */
  volume7dSats?: number | null;
  source: string; sourceUrl: string;
};

export type CatalogueEntry = CatalogueCollection | CatalogueRune;

/** What the order of one of the two lists means, and which field it was sorted by. `field: null` is not a ranking at all. */
export type RankingOf = {
  said: string;
  field: "volume7dSats" | "volumeAllTimeSats" | null;
  source: string;
  /** the window this list actually got, which is not always the one asked for: `day7`, or `5d` when a feed ran out first */
  window: string | null;
  want: number; kept: number;
  /** how many collections are in the file by NAME rather than by rank; `kept + named` is what the file holds */
  named?: number;
  /** a summed figure says the span it covered and how many fills it is made of; a stated one carries none of these */
  from?: string; to?: string; trades?: number;
  /** why this basis and not a better one, when the fetch had to fall back to it. The notes carry the same in full. */
  why?: string;
};

export type Ranking = {
  /** the window ASKED FOR, e.g. `day7`. Each side's own `window` is the one it got. */
  window: string;
  unit: string;
  collections: RankingOf;
  runes: RankingOf;
};

export type Catalogue = {
  fetchedAt: string;
  /** the ranking in one sentence, written by the fetch. Absent on a file fetched before it was recorded. */
  rankedBy?: string;
  ranking?: Ranking;
  collectionSource: string; runeSource: string;
  attribution: { name: string; url: string; used: string }[];
  notes: string[];
  collections: CatalogueCollection[];
  runes: CatalogueRune[];
};

export const isRune = (e: CatalogueEntry): e is CatalogueRune => e.kind === "rune";

/**
 * A listing's catalogue entry, found by the `ref` the chain carries — a collection's slug, or a rune's "block:tx" id.
 *
 * The catalogue is a file the site ships (`web/public/catalogue/`, docs/phase9.md), so a vault's picture cannot fail on
 * somebody else's outage, and a listing the catalogue does not know simply has no entry rather than a broken image.
 */
export function entriesByRef(catalogue: Catalogue | undefined): Map<string, CatalogueEntry> {
  const out = new Map<string, CatalogueEntry>();
  if (!catalogue) return out;
  for (const e of [...catalogue.collections, ...catalogue.runes]) {
    out.set(e.slug.toLowerCase(), e);
    if (isRune(e)) out.set(e.runeId.toLowerCase(), e);
  }
  return out;
}

/** Loaded once and kept: it is a committed file, so it cannot change while the page is open. */
export function useCatalogue() {
  return useQuery({
    queryKey: ["catalogue"],
    staleTime: Infinity,
    gcTime: Infinity,
    retry: 1,
    queryFn: async (): Promise<Catalogue> => {
      const res = await fetch(`${import.meta.env.BASE_URL}catalogue/catalogue.json`);
      if (!res.ok) throw new Error(`catalogue ${res.status}`);
      return res.json();
    },
  });
}

/** Everything in one list, collections first, for the choosers that offer both. */
export const allOf = (c: Catalogue | undefined): CatalogueEntry[] =>
  c ? [...c.collections, ...c.runes] : [];

/**
 * Sats to a readable figure and its unit, kept apart so a narrow card can set them on separate lines.
 * A floor of 10,000 sats is 0.0001 BTC, which nobody reads, so under a million it stays in sats.
 */
export function satsParts(n: number | null | undefined): [string, string] {
  if (n === null || n === undefined || !Number.isFinite(n) || n <= 0) return ["not known", ""];
  if (n < 1_000_000) return [Math.round(n).toLocaleString("en-US"), "sats"];
  return [(n / 100_000_000).toLocaleString("en-US", { maximumFractionDigits: 4, minimumFractionDigits: 2 }), "BTC"];
}

/** The same figure, said in one breath, for a line of prose. */
export function sats(n: number | null | undefined): string {
  const [v, unit] = satsParts(n);
  return unit ? `${v} ${unit}` : v;
}

/**
 * What the order of this catalogue means, in the file's own words, for the screens that show it.
 *
 * It reads the fetch's record rather than restating what the site hopes is true, so a list ranked by this week says so, a
 * list that is a curated selection says THAT, and a file that records no basis at all says it records none. The one thing
 * it must never do is let an order be read as a ranking it is not. docs/phase9.md §11.
 */
export function rankingLine(c: Catalogue | undefined): string {
  if (!c) return "";
  if (!c.ranking) {
    return "This catalogue records no ranking basis, so its order is not one: it was fetched before the basis was "
      + "written into the file.";
  }
  const part = (r: RankingOf, noun: string) =>
    r.field ? `the top ${r.kept} ${noun} by ${r.said}` : `${r.kept} ${noun}, which are ${r.said}`;
  const line = `${part(c.ranking.collections, "collections")}, and ${part(c.ranking.runes, "runes")}.`;
  return line.charAt(0).toUpperCase() + line.slice(1);
}

/**
 * The figure a card prints, which is the figure its list was ranked by and nothing else.
 *
 * The two lists are on two bases, so a card cannot assume a window: it reads `ranking.<kind>.field` and prints THAT
 * field, with the span it covers in the same breath. A list with no basis recorded, or an entry with no figure in the
 * field its list was sorted by, prints nothing at all rather than a number whose meaning nobody can state.
 */
export function rankedFigure(e: CatalogueEntry, ranking: Ranking | undefined): string | null {
  const of = isRune(e) ? ranking?.runes : ranking?.collections;
  if (!of?.field) return null;
  const v = (e as unknown as Record<string, number | null | undefined>)[of.field];
  if (!v || !Number.isFinite(v) || v <= 0) return null;
  if (of.field === "volumeAllTimeSats") return `${sats(v)} traded in all, lifetime`;
  const fills = !isRune(e) && of.trades ? ` over ${(e as CatalogueCollection).trades7d ?? 0} sales` : "";
  return `${sats(v)} traded that week${fills}`;
}

/** What a vault looks like to the catalogue: the registry's ref is a collection's slug, or a rune's id. */
export type VaultRef = { id: number; ref: string; paused: boolean };

/**
 * The vaults, by the ref a catalogue entry would be listed under. There is no join key on the wire other than this one:
 * the registry's `ref` is the slug the fetch wrote for a collection and the rune id ord gave for a rune.
 */
export const vaultsByRef = (vaults: readonly VaultRef[] | undefined): Map<string, VaultRef> =>
  new Map((vaults ?? []).map((v) => [v.ref.toLowerCase(), v]));

/**
 * What a card says about its vault, which is a fact about that ONE entry and is printed on it rather than asserted over a
 * section. "paused" and "no vault yet" are different facts and a reader must not have to guess which one they are looking
 * at. docs/phase9.md §11.3.
 */
export const vaultTag = (vault: VaultRef | undefined): string => (!vault ? "no vault yet" : vault.paused ? "paused" : "vault");

/** The vault this entry has, if it has one. Nothing is inferred from a name: only the ref it would be listed under. */
export const vaultOf = (e: CatalogueEntry, by: Map<string, VaultRef>): VaultRef | undefined =>
  by.get((isRune(e) ? e.runeId : e.slug).toLowerCase());

/** A vault as the chooser needs it: the registry's own fields, which is what `/v1/listings` hands over. */
export type ChoosableVault = { id: number; kind: "inscriptions" | "rune"; paused: boolean; name: string; ref: string };

/** One row of the chooser: a catalogue entry, the vault it has, or both. Everything with a vault can be picked. */
export type Choice = {
  key: string; name: string; kind: "inscriptions" | "rune"; ref: string;
  entry?: CatalogueEntry; vault?: ChoosableVault;
};

/**
 * The catalogue and the vaults as ONE list, joined on the registry's own ref and read in the catalogue's ranked order,
 * with whatever the registry holds that the catalogue does not name — the demo listings, and anything that has dropped
 * out of a later fetch's fifty — added after it. Nothing is fenced off into a section, because a vault is a fact about
 * an entry and not a place it lives. docs/phase9.md §11.3.
 */
export function choices(catalogue: Catalogue | undefined, vaults: readonly ChoosableVault[]): Choice[] {
  const by = new Map(vaults.map((v) => [v.ref.toLowerCase(), v]));
  const taken = new Set<string>();
  const out: Choice[] = [];
  for (const e of [...(catalogue?.collections ?? []), ...(catalogue?.runes ?? [])]) {
    const ref = isRune(e) ? e.runeId : e.slug;
    const vault = by.get(ref.toLowerCase());
    if (vault) taken.add(ref.toLowerCase());
    out.push({ key: e.id, name: e.name, kind: e.kind, ref, entry: e, vault });
  }
  for (const v of vaults) {
    if (taken.has(v.ref.toLowerCase())) continue;
    out.push({ key: `vault-${v.id}`, name: v.name, kind: v.kind, ref: v.ref, vault: v });
  }
  return out;
}

/** Name, ticker or ref, over the one list. */
export const matchChoices = (q: string, rows: readonly Choice[]): Choice[] => {
  const s = q.trim().toLowerCase();
  if (!s) return [...rows];
  return rows.filter((r) => r.name.toLowerCase().includes(s)
    || r.name.replace(/•/g, "").toLowerCase().includes(s)
    || r.ref.toLowerCase().includes(s));
};

/** The date a price was true, said plainly. Never "live". */
export const asOf = (iso: string | undefined): string => {
  if (!iso) return "an earlier fetch";
  const d = new Date(iso);
  return Number.isNaN(d.valueOf())
    ? "an earlier fetch"
    : d.toLocaleDateString("en-GB", { day: "numeric", month: "long", year: "numeric" });
};

/** A rune's name is spaced with bullets by the protocol. Its ticker is that name without them. */
export const runeTicker = (name: string): string => name.replace(/•/g, "");

/** What the chooser's own search can answer, over the committed file: a name, a ticker, or a rune id. */
export function searchCatalogue(q: string, entries: CatalogueEntry[], limit = 24): CatalogueEntry[] {
  const s = q.trim().toLowerCase();
  if (!s) return entries.slice(0, limit);
  const scored: [number, CatalogueEntry][] = [];
  for (const e of entries) {
    const name = e.name.toLowerCase();
    const flat = isRune(e) ? runeTicker(e.name).toLowerCase() : e.slug.toLowerCase();
    const runeId = isRune(e) ? e.runeId : "";
    let score = -1;
    if (name.startsWith(s) || flat.startsWith(s)) score = 0;
    else if (name.includes(s) || flat.includes(s)) score = 1;
    else if (runeId === s) score = 0;
    else if (!isRune(e) && (e.description ?? "").toLowerCase().includes(s)) score = 2;
    if (score >= 0) scored.push([score, e]);
  }
  return scored
    .sort((a, b) => a[0] - b[0] || a[1].name.localeCompare(b[1].name))
    .slice(0, limit)
    .map(([, e]) => e);
}
