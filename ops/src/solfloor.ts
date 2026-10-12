import { readFileSync } from "node:fs";
import { dirname, isAbsolute, resolve } from "node:path";
import { robustFloor } from "../../keeper/src/floor.js";
import { BookDiff, OrdinalsWalletBook } from "../../keeper/src/floorsources.js";
import { mainnetFeed, VerifiedOrd } from "../../keeper/src/ordapi.js";
import type { Log } from "./log.js";

/**
 * docs/v9/solana-migration.md §2.9 D-i: the canister is the registry keeper and posts the Inscriptions floors itself (`post_floor`,
 * `frogsol::post_act`), but the READING stays runner-supplied. The reading is the keeper's production feed, unchanged
 * (keeper/service/feed.ts `OrdinalsWalletFeed`, docs/v5/floor-feed.md): Ordinals Wallet's book, every listed outpoint's contents believed
 * only when two ordinals providers agree (`VerifiedOrd` over `mainnetFeed()`: only UniSat and ordinals.com serve outputs, so
 * UNISAT_API_KEY is required), the book diff for list/delist events, `robustFloor`'s rules (≥ 3 valid listings). It stays in SATS: the
 * canister converts at its own SOL/BTC rate and clamps into the module's bounds.
 *
 * Ordinals multi-collection (docs/v9/ordinals-collections.md §4, the merged canister): every Inscriptions listing in the books is a
 * frog collection of its own, `frog_collections()` lists them (`coll`, its members `root`, `live`), and the reading names its collection:
 * `frog_floor_reading(coll, sats, confidence_bps, depth_items)`. A launch-file listing is the canister's collection whose root is the
 * listing's `membersRoot`. Every LIVE collection is read and handed in (every Inscriptions listing starts live: default-live, 2026-10-12); a dormant one (the
 * operator put it to sleep with `frog_sleep(coll)`; `frog_wake(coll)` lifts it) is read only with `RUNNER_FLOOR_LISTINGS=all` and held, served at GET /floors.
 *
 * The 20 % band gate (§5 step 2, §8: the monitor's D8 band, which would pause the collection otherwise): a reading is handed in only
 * when CoinGecko's floor for the collection (`coingeckoId`) answers and the reading is within 20 % of it. Otherwise it is held, logged,
 * and the module keeps its last post (the canister lets it go stale at 3 h: /healthz then says so).
 */

export type InscListing = { id: number; slug: string; owSlug: string; name: string; wrapping: "now" | "later"; canister: boolean;
  /** the members root (hex, 0x…), matched against `frog_collections().root`; the members file (absolute); the price reference */
  membersRoot: string; membersFile: string; coingeckoId: string | null };
export type Reading = { sats: bigint; confidenceBps: number; depthItems: number; why: string; ref: string };
export type NoReading = { sats: null; why: string; ref: string };
export type ReadFloor = (owSlug: string) => Promise<Reading | NoReading>;
/** CoinGecko's floor for a collection, in sats (null: no answer, or not in BTC). */
export type ReadRef = (coingeckoId: string) => Promise<bigint | null>;
export type Held = { listing: number; coll: number | null; slug: string; at: number; posted: boolean; sats: string | null; confidenceBps: number | null;
  depthItems: number | null; refSats: string | null; note: string };
/** One canister collection as the floor step reads it (`frog_collections()`). */
export type FrogColl = { coll: number; root: string; live: boolean };

/** The band of §5 step 2: a reading more than this far from CoinGecko's floor is held. */
export const FLOOR_BAND_BPS = 2_000;

/** A root as the canister and the launch file write it, compared: lower-case, no 0x. */
export const rootKey = (r: string): string => r.trim().toLowerCase().replace(/^0x/, "");

/**
 * The Inscriptions listings of the launch file with their launch ids (the Ethereum collections first (0..n-1), then `bitcoin`, then
 * `inscriptions[]`). `membersFile` is resolved against the repository root (the launch file's ../../), as the files name it.
 */
export function inscriptionListings(pilot: { collections?: unknown[]; bitcoin?: Record<string, unknown> | null; inscriptions?: Record<string, unknown>[] },
  root = "."): InscListing[] {
  const base = (pilot.collections ?? []).length;
  const rows = [...(pilot.bitcoin ? [pilot.bitcoin] : []), ...(pilot.inscriptions ?? [])];
  return rows.map((r, i) => {
    const slug = String(r.slug ?? ""), ow = String(r.owSlug ?? slug);
    if (!/^[a-z0-9_-]{1,64}$/.test(ow)) throw new Error(`${slug}: owSlug "${ow}" is not an Ordinals Wallet slug`);
    const mf = String(r.membersFile ?? "");
    return { id: base + i, slug, owSlug: ow, name: String(r.listName ?? r.name ?? slug), wrapping: r.wrapping === "later" ? "later" : "now", canister: i === 0 && !!pilot.bitcoin,
      membersRoot: String(r.membersRoot ?? ""), membersFile: mf && !isAbsolute(mf) ? resolve(root, mf) : mf,
      coingeckoId: typeof r.coingeckoId === "string" && r.coingeckoId ? r.coingeckoId : null };
  });
}
export const loadInscriptionListings = (path: string) => inscriptionListings(JSON.parse(readFileSync(path, "utf8")), resolve(dirname(path), "..", ".."));

/** The launch listing of each canister collection, by root (a collection no listing names, or a listing the canister lacks, has no pair). */
export function pairListings(listings: readonly InscListing[], colls: readonly FrogColl[]): Map<number, FrogColl> {
  const byRoot = new Map(colls.map((c) => [rootKey(c.root), c]));
  const out = new Map<number, FrogColl>();
  for (const l of listings) { const c = l.membersRoot ? byRoot.get(rootKey(l.membersRoot)) : undefined; if (c) out.set(l.id, c); }
  return out;
}

/** The production reading (keeper/service/feed.ts `OrdinalsWalletFeed.floor`, in sats): one book reader, one verified ord, one diff, for the process. */
export function productionReader(o: { fetch?: typeof fetch; log?: Log; nowS?: () => number } = {}): ReadFloor {
  const fetchImpl = o.fetch ?? fetch;
  const book = new OrdinalsWalletBook(fetchImpl);
  const ord = new VerifiedOrd(mainnetFeed({ fetchImpl, log: o.log }));
  const diff = new BookDiff();
  return async (ow) => {
    const ref = `ordinalswallet:${ow}`;
    let b;
    try { b = await book.book(ow, ord); } catch (err) { return { sats: null, ref, why: `the book could not be read: ${String((err as Error).message).slice(0, 200)}` }; }
    const events = diff.observe(ow, b);
    const f = robustFloor({ listings: b.listings, sales: b.sales, events, snapshotAt: o.nowS ? o.nowS() : b.snapshotAt });
    if (f.price === null) {
      const unverified = f.excluded.filter((x) => x.rule === "L-unverified").length;
      return { sats: null, ref, why: `no floor: fewer than 3 valid listings (${f.excluded.length} excluded, ${unverified} of them because two ordinals providers did not agree on the listed output)` };
    }
    return { sats: f.price, confidenceBps: Math.min(10_000, Math.max(0, f.confidenceBps)), depthItems: f.depthItems, ref,
      why: `floor ${f.price} sats, confidence ${f.confidenceBps} bps, depth ${f.depthItems}` };
  };
}

export const COINGECKO_API_DEFAULT = "https://api.coingecko.com/api/v3";
/**
 * CoinGecko's NFT floor (`/nfts/{id}`, `floor_price.native_currency`, BTC for an Ordinals collection) in sats, as the monitor reads it
 * (monitor/src/ethnft.mjs). The optional demo key (RUNNER_COINGECKO_KEY) is sent only to CoinGecko. A non-BTC or missing answer is null.
 */
export function coingeckoReader(f: typeof fetch, key: () => string | undefined = () => undefined, api = COINGECKO_API_DEFAULT): ReadRef {
  return async (id) => {
    try {
      const k = key();
      const r = await f(`${api}/nfts/${encodeURIComponent(id)}`, { signal: AbortSignal.timeout(15_000), ...(k ? { headers: { "x-cg-demo-api-key": k } } : {}) });
      if (!r.ok) return null;
      const j = await r.json() as { id?: unknown; native_currency_symbol?: unknown; floor_price?: { native_currency?: number } };
      // audit C4-08: the answer must name its currency, BTC, and be the asked collection's; anything else is no reading (fail closed)
      if (typeof j.native_currency_symbol !== "string" || j.native_currency_symbol.toLowerCase() !== "btc" || j.id !== id) return null;
      const v = Number(j.floor_price?.native_currency);
      if (!(v > 0) || !Number.isFinite(v)) return null;
      return BigInt(Math.round(v * 1e8));
    } catch { return null; }
  };
}

/** §5 step 2: is the reading within FLOOR_BAND_BPS of the reference? (No reference: no.) */
export function inBand(sats: bigint, ref: bigint | null, bandBps = FLOOR_BAND_BPS): { ok: boolean; why: string } {
  if (ref === null || ref <= 0n) return { ok: false, why: "no CoinGecko reference answered: the 20 % band cannot be checked" };
  const diff = sats > ref ? sats - ref : ref - sats;
  const bps = Number((diff * 10_000n) / ref);
  return diff * 10_000n <= ref * BigInt(bandBps)
    ? { ok: true, why: `within ${(bps / 100).toFixed(1)} % of CoinGecko's ${ref} sats` }
    : { ok: false, why: `${(bps / 100).toFixed(1)} % from CoinGecko's ${ref} sats, outside the ${bandBps / 100} % band` };
}

export type FloorDeps = {
  buyer: { frog_floor_reading(coll: number, sats: bigint, confidenceBps: number, depthItems: number): Promise<{ Ok: string } | { Err: string }>;
    frog_collections(): Promise<FrogColl[]> };
  log: Log;
  listings: InscListing[];
  read: ReadFloor;
  /** CoinGecko's floor (the band gate); without it nothing passes the gate */
  ref?: ReadRef;
  /** which listings are read: the canister's LIVE collections (default), or also every dormant one whose wrapping is not "later" (held) */
  scope?: "canister" | "all";
  everyS?: number;
  nowS?: () => number;
  /** per listing, when it was last read (seconds) */
  last?: Map<number, number>;
  /** per canister collection, when the canister last accepted a reading (seconds), for /healthz */
  accepted?: Map<number, number>;
  /** the canister's live collections as of the last pass, for /healthz */
  live?: Set<number>;
  /** what each listing read last, for GET /floors */
  held?: Map<number, Held>;
};

/** Default: every 30 min (the canister drops a reading not posted within the hour; rook_core's FLOOR_TTL is 3 h). */
export const FLOOR_EVERY_DEFAULT_S = 1_800;

/** One pass. True when the canister took a reading (the runner then ticks again at once, so it is posted now). */
export async function frogFloorOnce(d: FloorDeps): Promise<boolean> {
  const now = d.nowS ?? (() => Math.floor(Date.now() / 1000));
  const every = d.everyS ?? FLOOR_EVERY_DEFAULT_S, last = d.last ?? new Map<number, number>();
  let colls: FrogColl[];
  try { colls = await d.buyer.frog_collections(); } catch (err) { d.log("warn", "frog-collections-unread", { error: String((err as Error).message).slice(0, 200) }); return false; }
  const pairs = pairListings(d.listings, colls);
  if (d.live) { d.live.clear(); for (const c of colls) if (c.live) d.live.add(c.coll); }
  for (const c of colls.filter((x) => x.live && ![...pairs.values()].includes(x)))
    d.log("error", "frog-floor-no-listing", { coll: c.coll, root: c.root, why: "a live collection whose root no launch-file listing names: no floor is read for it" });
  let took = false;
  for (const l of d.listings) {
    const c = pairs.get(l.id) ?? null, live = !!c?.live;
    if (!live && (d.scope !== "all" || l.wrapping === "later")) continue;
    if ((last.get(l.id) ?? -Infinity) + every > now()) continue;
    last.set(l.id, now());
    const r = await d.read(l.owSlug).catch((err): NoReading => ({ sats: null, ref: `ordinalswallet:${l.owSlug}`, why: String((err as Error).message).slice(0, 200) }));
    const row: Held = { listing: l.id, coll: c?.coll ?? null, slug: l.slug, at: now(), posted: false, sats: r.sats === null ? null : String(r.sats),
      confidenceBps: r.sats === null ? null : r.confidenceBps, depthItems: r.sats === null ? null : r.depthItems, refSats: null, note: r.why };
    d.held?.set(l.id, row);
    if (r.sats === null) { d.log("warn", "frog-floor-none", { listing: l.id, coll: row.coll, slug: l.slug, ref: r.ref, why: r.why }); continue; }
    if (!c || !live) {
      row.note = `held, not posted: ${c ? `collection ${c.coll} is asleep (the operator's frog_sleep; frog_wake(${c.coll}) lifts it)` : "no canister collection has this listing's root"} (${r.why})`;
      d.log("info", "frog-floor-held", { listing: l.id, coll: row.coll, slug: l.slug, sats: r.sats, confidenceBps: r.confidenceBps, depthItems: r.depthItems });
      continue;
    }
    // §5 step 2: the 20 % band against CoinGecko before anything is handed in
    const ref = l.coingeckoId && d.ref ? await d.ref(l.coingeckoId).catch(() => null) : null;
    row.refSats = ref === null ? null : String(ref);
    const gate = inBand(r.sats, ref);
    if (!gate.ok) {
      row.note = `held, not posted: ${l.coingeckoId ? gate.why : "the listing names no coingeckoId: the 20 % band cannot be checked"} (${r.why})`;
      d.log("warn", "frog-floor-out-of-band", { listing: l.id, coll: c.coll, slug: l.slug, sats: r.sats, refSats: ref, why: row.note });
      continue;
    }
    try {
      const res = await d.buyer.frog_floor_reading(c.coll, r.sats, r.confidenceBps, r.depthItems);
      if ("Ok" in res) {
        took = true; row.posted = true; row.note = `${r.why}; ${gate.why}`;
        d.accepted?.set(c.coll, now());
        d.log("info", "frog-floor-handed-in", { listing: l.id, coll: c.coll, slug: l.slug, sats: r.sats, confidenceBps: r.confidenceBps, depthItems: r.depthItems, refSats: ref, answer: res.Ok });
      } else { row.note = `refused: ${res.Err}`; d.log("warn", "frog-floor-not-taken", { listing: l.id, coll: c.coll, slug: l.slug, error: res.Err }); }
    } catch (err) { row.note = `call failed: ${String((err as Error).message).slice(0, 200)}`; d.log("warn", "frog-floor-call-failed", { listing: l.id, coll: c.coll, error: row.note }); }
  }
  return took;
}

/** /healthz: the live collections with no reading accepted for `staleS` (once the runner has run that long). */
export function staleFloors(live: Iterable<number>, accepted: Map<number, number>, nowS: number, startedS: number, staleS: number): string[] {
  if (nowS - startedS < staleS) return [];
  return [...live].filter((c) => nowS - (accepted.get(c) ?? startedS) > staleS)
    .map((c) => `no frog floor reading accepted by the canister for collection ${c} in ${Math.round((nowS - (accepted.get(c) ?? startedS)) / 60)} min (the module goes stale at 3 h)`);
}
