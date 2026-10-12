// Builds the catalogue of real Bitcoin assets a coin's fees can be pointed at.
//
// Run by hand, not by the build, and never by the site:
//
//     UNISAT_API_KEY=... node scripts/catalogue.mjs      # the only run that may write catalogue.json
//     node scripts/catalogue.mjs                         # refuses, loudly; see "why a key is required"
//
// It writes public/catalogue/catalogue.json and public/catalogue/img/*, both committed.
// A visitor's browser therefore never calls a marketplace: no key ships, no rate limit
// applies, and no page breaks when somebody else's API does. Every price carries the
// date it was fetched, so the site can say "as of" and never imply live.
//
// WHAT THE ORDER MEANS, and it is the order that is printed. Fifty of each kind, on TWO BASES, which are printed apart
// and never as one (docs/phase9.md §11.3):
//
//   runes        the volume traded in the SEVEN DAYS before the fetch. `runes_types` states it inside UniSat's own
//                `day7` window and is simply read. `volume7dSats`, `window: "day7"`.
//   collections  LIFETIME volume, from Ordinals Wallet's keyless index. `volumeAllTimeSats`, `window: null`, and the
//                file says in plain words that it is the whole history of a market and NOT this week.
//
// THE COLLECTIONS ARE ON LIFETIME BECAUSE NO SOURCE WILL STATE THEIR WEEK. UniSat's `collection_statistic_list` is the
// one endpoint that takes a window and it answers an empty list for every window; `collection_statistic` takes a
// collection id and no window, so its figure is lifetime; the order book carries no volume at all. Summing the market's
// own `Sold` fills -- which this script still does, first, every run -- found FOUR collections over SEVENTY fills in a
// week (§11.2): their inscription market is dead for ranking. Magic Eden's whole Bitcoin API answers nothing live. So
// the week is asked for and taken only when it can actually FILL THE FIFTY; otherwise the run falls to lifetime, says
// so in the file's notes in the fetch's own words, and never prints one window as the other.
//
// WHY A KEY IS REQUIRED. The runes' week cannot be had without one, and a file with no ranked list in it is not this
// catalogue: keyless, the collections would be a lifetime order and the runes a curated list in no order at all. So a
// keyless run refuses to touch catalogue.json or the pictures beside it. CATALOGUE_ALLOW_UNRANKED=1 lets one write the
// clearly marked catalogue.unranked.json, with its own img-unranked/, which the site does not read and must never be
// renamed.
//
// Reads only. Nothing here places, changes or cancels an order.
// See docs/phase9.md for the provenance record and the terms position.

import { mkdir, writeFile, rm } from "node:fs/promises";
import { dirname, join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const OUT = join(HERE, "..", "public", "catalogue");

const UA = "etched-catalogue/1 (+https://github.com/jlo110/etch)";

/** The top fifty of each kind, which is what a creator choosing a vault was promised. */
export const WANT_COLLECTIONS = Number(process.env.CATALOGUE_COLLECTIONS ?? 50);
export const WANT_RUNES = Number(process.env.CATALOGUE_RUNES ?? 50);

/**
 * UniSat's own name for the seven-day window. It is the only window anything in this file is ranked by and it is sent
 * on every market call. `day1` and `day30` are the other two the API offers; neither of them is "this week".
 */
export const WINDOW = "day7";
/** The one field a WEEKLY list may be sorted by. Named for its window so it cannot be misread as a lifetime figure. */
export const RANK_FIELD = "volume7dSats";
/**
 * The one field a LIFETIME list may be sorted by, which today is the collections' (docs/phase9.md §11.3). It is never
 * the runes' and it is never copied into `volume7dSats`: a list ranked by it says so in its own `ranking` block.
 */
export const LIFETIME_FIELD = "volumeAllTimeSats";
/**
 * How far past the fiftieth rune the ranked pool reaches. ord drops a ticker it cannot resolve, and a ticker it resolves
 * to a different rune id than UniSat named, so a pool of exactly fifty yields fewer than fifty. The pool is walked in
 * ranked order and stopped at fifty described: nothing below a rune that was kept is ever promoted above it.
 */
const RUNE_HEADROOM = Number(process.env.CATALOGUE_RUNE_HEADROOM ?? 30);
const RUNE_PAGE = 50;

/**
 * The seven days, in seconds, and how the collection market's own fills are paged.
 *
 * `collection/auction/actions` caps `limit` below 100, so the window is walked backwards a page at a time until a fill
 * older than the cutoff proves the whole of it has been seen. The page cap is a stop, not a window: if it is reached
 * first, the figure covers fewer than seven days and is LABELLED with the days it actually covers.
 */
const WEEK_S = 7 * 24 * 60 * 60;
const ACTION_PAGE = 99;
const ACTION_PAGES = Number(process.env.CATALOGUE_ACTION_PAGES ?? 60);
/**
 * A seven-day volume for a top-fifty market, in sats. Below this the figure is far likelier to be denominated in BTC
 * than in sats, and a BTC figure written into a field called `...Sats` is a lie the site would print on every screen.
 * 10,000 sats is 0.0001 BTC: no market that ranks in anybody's top fifty traded that little in a week.
 */
const SATS_SANITY = 10_000;

// The index is about 6,000 collections deep and the endpoint takes no sort parameter, so a ranking is only as good as the
// share of it that was read. At 6 pages this read 600 of them and called the result a top fifty. `collectionsKeyless` stops
// early on a short page, so walking further costs nothing once the index ends.
const PAGES = Number(process.env.CATALOGUE_PAGES ?? 60);
const GAP_MS = Number(process.env.CATALOGUE_GAP_MS ?? 260); // well under UniSat's 5/s
const MAX_IMAGE_BYTES = 600_000;

/** Where this run may write. A run that cannot rank never touches the committed catalogue or its pictures. */
let IMG = join(OUT, "img");
let IMG_URL = "/catalogue/img";
let OUT_FILE = join(OUT, "catalogue.json");

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const log = (...a) => console.log("[catalogue]", ...a);

const REFUSAL = `[catalogue] refused: there is no UNISAT_API_KEY (or UNISAT) in the environment.

  This file is the top ${WANT_COLLECTIONS} collections by lifetime volume across every marketplace. Not by the week:
  Ordinals Wallet's index does carry a weekly figure, but it is non-zero on 29 of the 1,076 collections that
  pass the filters (2.7%, measured 2026-09-22), and twenty-nine is not fifty -- and the top ${WANT_RUNES} runes by the volume traded in the
  seven days before the fetch. UniSat states that week for a rune and nothing keyless does: their market endpoints
  answer code -2004 unauthenticated, and ord lists runes in etching order, which is not a ranking by anything. So a
  keyless run has no ranked list in it at all, and a file with no ranked list in it is not this catalogue.

  Run it with the key:        UNISAT_API_KEY=... node scripts/catalogue.mjs
  Or write the lesser file:   CATALOGUE_ALLOW_UNRANKED=1 node scripts/catalogue.mjs
                              -> public/catalogue/catalogue.unranked.json, which the site does not read,
                                 which says in its own notes that its runes are a curated list and not
                                 ranked by this week, and which must never be renamed to catalogue.json.

  Nothing was written and no picture was touched.`;

/** Every outbound call goes through here, so the delay is never forgotten. */
let lastCall = 0;
async function call(url, init = {}) {
  const wait = GAP_MS - (Date.now() - lastCall);
  if (wait > 0) await sleep(wait);
  lastCall = Date.now();
  const res = await fetch(url, {
    ...init,
    headers: { "user-agent": UA, ...(init.headers ?? {}) },
    signal: AbortSignal.timeout(30_000),
  });
  return res;
}

/**
 * Every UniSat call: one place for the key, one place for their envelope. The key goes into a header and nowhere else.
 * It is never logged, never written into the file, and never part of a message this throws.
 */
async function unisat(path, key, body) {
  const res = await call(`https://open-api.unisat.io/v3/market/${path}`, {
    method: "POST",
    headers: { authorization: `Bearer ${key}`, "content-type": "application/json" },
    body: JSON.stringify(body),
  });
  if (!res.ok) throw new Error(`${path}: http ${res.status}`);
  const envelope = await res.json();
  if (envelope?.code !== 0) throw new Error(`${path}: code ${envelope?.code} ${envelope?.msg ?? ""}`.trim());
  return envelope?.data ?? {};
}

// ------------------------------------------------------------------ the ranking

/**
 * One figure, descending, and the top `want` of them. This is the whole point of the file, so it refuses rather than
 * return an order that merely looks like a ranking:
 *
 *  - no rows at all, or a list where nothing carries the figure, would be sorted arbitrarily and then printed as "the
 *    top by volume", which is the defect this replaced;
 *  - a figure too small to be sats is a figure in BTC, and one written into a field whose name ends `Sats` would
 *    mislabel the unit everywhere the site prints it.
 *
 * Either way the run stops and names the field to look at, because a wrong ranking committed is worse than no fetch.
 * Both bases go through here: until docs/phase9.md §11.3 the lifetime path had no guard at all, because it was only
 * ever allowed to write the lesser file. It writes the committed one now, so it gets all three.
 */
function rankBy(rows, field, want, what, asked) {
  const seen = new Set();
  const unique = rows.filter((r) => (seen.has(r.id) ? false : (seen.add(r.id), true)));
  const ranked = unique.slice().sort((a, b) => (b[field] ?? 0) - (a[field] ?? 0)).slice(0, want);
  const figures = ranked.map((r) => r[field] ?? 0).filter((v) => v > 0);
  if (!figures.length) {
    throw new Error(`${asked} for ${what} and got ${unique.length} rows, not one of which `
      + `carried a positive ${field}. Ranking them would be arbitrary, and an arbitrary order printed as "the top `
      + `by volume" is the bug this script exists to prevent. Check which field the volume arrives in.`);
  }
  if (figures[0] < SATS_SANITY) {
    throw new Error(`the largest ${what} volume returned is ${figures[0]}, which is too small to be a `
      + `figure in sats (${SATS_SANITY} sats is 0.0001 BTC). It is very likely denominated in BTC, and a BTC figure in `
      + `a field called ${field} would be printed as sats on every screen. Fix the unit before trusting this run.`);
  }
  log(`${what}: ${ranked.length} kept of ${unique.length} offered, ${figures.length} with a figure`);
  for (const r of ranked.slice(0, 5)) log(`  ${r.name ?? r.slug ?? r.tick}: ${r[field]} sats`);
  return ranked;
}

/** The seven-day figure, descending. The runes' basis, and the collections' on the day their market comes back. */
export function rankByWeek(rows, want, what) {
  return rankBy(rows, RANK_FIELD, want, what, `asked UniSat over timeType=${WINDOW}`);
}

/**
 * The lifetime figure, descending: the collections' basis while no source will state their week (docs/phase9.md §11.3).
 * It is a different field from the week's and it is never copied into it, so a list ranked here can only ever be
 * described as what it is.
 */
export function rankByLifetime(rows, want, what) {
  return rankBy(rows, LIFETIME_FIELD, want, what,
    "asked Ordinals Wallet's keyless index for lifetime volume ACROSS ALL MARKETPLACES (`global_volume_all_time`), " +
    "not the volume on Ordinals Wallet alone");
}

/**
 * Collections to fetch by NAME and fold into the ranking.
 *
 * `/collections` pages an index of 426,444 in a fixed order that no parameter changes, so a walk of the first few thousand
 * misses collections that outrank everything in it. Ordinals Wallet's own "Global Vol 7d" board shows these; the paginated
 * list never reaches them.
 *
 * `/search/{name}` does, and it answers in the SAME row shape the list uses — `global_volume_all_time` included. So these
 * are not a separate class that has to be appended unranked: they are rows the walk missed, fetched another way, and they
 * take their real place in the order. NodeMonkes at 466bn outranks everything the walk found.
 *
 * An earlier version used `/collection/{slug}` and `/collection/{slug}/stats`, which carry no market-wide volume at all —
 * hence the conclusion, wrong, that these could not be ranked. The endpoint was the limit, not the data.
 */
export const BY_NAME = [
  "Bitcoin Puppets", "NodeMonkes", "Runestone", "O.P.I.U.M.", "Ordinal Penguins", "Taproot Wizards",
  "Quantum Cats", "The Wizards of Ord", "Bitcoin Wizards", "CENTS", "VANGUARDS OF BTC", "RSIC METAPROTOCOL",
  "Bitcoin Planets", "Natcats",
];

/** One collection by name, in the list's own row shape. Null when the search returns nothing under that exact name. */
export async function collectionByName(name) {
  const res = await call(`https://turbo.ordinalswallet.com/search/${encodeURIComponent(name)}`);
  if (!res.ok) return null;
  const hits = (await res.json())?.collections ?? [];
  // the exact name, never merely the first hit: "Planets" would otherwise match a dozen things that are not it
  return hits.find((h) => (h?.name ?? "").toLowerCase() === name.toLowerCase()) ?? null;
}

/** What the order of a list means, recorded in the file so that a future reader cannot be misled again. */
const RANKED = {
  said: `UniSat's volume over the seven days before this file was fetched (their own ${WINDOW})`,
  field: RANK_FIELD, source: "UniSat", window: WINDOW,
};
/**
 * The collections' basis, which is the one that has to state its own span: it is summed from fills, and a feed that ran
 * out before the cutoff covers fewer days than were asked for. Whatever it covered is what it says.
 */
const summed = (cover) => ({
  said: cover.full
    ? "the sats paid in UniSat's own collection-market fills over the seven days before this file was fetched"
    : `the sats paid in UniSat's own collection-market fills over the ${cover.days} days its feed reached back, which is `
      + "less than the seven that were asked for",
  field: RANK_FIELD,
  source: "UniSat",
  window: cover.full ? WINDOW : `${cover.days}d`,
  from: cover.from, to: cover.to, trades: cover.trades,
});
const CURATED = {
  said: "a curated list this script holds, in no meaningful order -- nothing about it is a ranking by volume",
  field: null, source: "ord (ordinals.com)", window: null,
};
/**
 * The collections' basis while no source will state their week (docs/phase9.md §11.3). `said` is read straight out of
 * the file by `rankingLine()` and printed after the words "the top fifty collections by", so it has to carry the whole
 * correction itself: what the figure covers, that it is not this week, and why there is no week to have instead.
 */
const LIFETIME = {
  said: "lifetime volume from Ordinals Wallet's keyless index -- the whole history of a market, NOT this week, because "
    + "the index carries a weekly figure but it is non-zero on 29 of 1,076 collections (2.7%), so there is no week to "
    + "rank fifty by. Lifetime here is ACROSS ALL MARKETPLACES, not Ordinals Wallet's own book",
  field: LIFETIME_FIELD, source: "Ordinals Wallet", window: null,
};
/** What this run in particular found when it asked for the week anyway. It goes in the notes, not on every card. */
const lifetimeBasis = (why) => ({ ...LIFETIME, why });

// ---------------------------------------------------------------- collections

/**
 * UniSat is the source the operator asked for: it is the marketplace most Bitcoin collectors actually price against. It
 * needs a key, which lives in the operator's environment and never in this repository. Commercial use needs written
 * permission under their §3.1; see docs/phase9.md.
 *
 * COLLECTIONS ARE SUMMED FROM THE FILLS, because no UniSat endpoint will state a collection's weekly volume.
 * `collection_statistic_list` is the one that takes a window (`filter.timeType`, required, and nothing else is allowed
 * beside it) and it answers `{"list":[],"total":0}` for every window; `collection_statistic` takes a `collectionId` and
 * no window at all, so its `btcValue` is a lifetime figure; `collection/auction/list` is the order book and carries no
 * volume field of any kind. What does answer is `collection/auction/actions`, the market's own record of listings,
 * cancellations and sales: every `Sold` row carries `collectionId`, `price` and `timestamp`.
 *
 * So the week is computed rather than asked for — the sats paid, per collection, in the fills inside the window. That is
 * a better provenance than an aggregate anyway, because the file can say exactly what was summed and over what span, and
 * `trades7d` says how many fills each figure is made of. What it cannot do is invent a span it did not see: if the feed
 * runs out before the cutoff, the coverage it DID reach is what gets labelled.
 */
async function collectionFillsUnisat(key) {
  const now = Math.floor(Date.now() / 1000);
  const cutoff = now - WEEK_S;
  const sold = [];
  let reached = false, pages = 0, seen = 0;
  for (let start = 0; pages < ACTION_PAGES; start += ACTION_PAGE, pages++) {
    const data = await unisat("collection/auction/actions", key,
      { filter: { nftType: "collection", event: "Sold" }, start, limit: ACTION_PAGE });
    const batch = data?.list ?? [];
    seen += batch.length;
    for (const r of batch) {
      const at = stamp(r.timestamp);
      if (at && at < cutoff) { reached = true; continue; }
      if (!r.collectionId || !(Number(r.price) > 0)) continue;
      sold.push({
        collectionId: String(r.collectionId),
        priceSats: Number(r.price),
        at: at ?? now,
        inscriptionId: r.inscriptionId ?? null,
        itemName: r.collectionItemName ?? null,
        contentType: r.contentType ?? null,
      });
    }
    log(`unisat collection fills: page ${pages + 1}, ${batch.length} rows, ${sold.length} inside the window`);
    if (batch.length < ACTION_PAGE) { reached = true; break; }   // the end of the feed: the window is fully covered
    if (reached) break;
  }
  if (!sold.length) {
    throw new Error(`UniSat's collection market reported no sale at all in the last seven days (${seen} action rows read `
      + `over ${pages} pages). There is nothing to rank collections by, and an order over nothing is not a ranking.`);
  }
  const earliest = Math.min(...sold.map((f) => f.at));
  const days = Math.max(1, Math.round((now - earliest) / 86_400));
  return {
    sold,
    cover: {
      full: reached,
      days: reached ? 7 : days,
      from: new Date((reached ? cutoff : earliest) * 1000).toISOString(),
      to: new Date(now * 1000).toISOString(),
      trades: sold.length,
      pages,
    },
  };
}

/** A marketplace timestamp, in seconds, whichever of the two units it arrives in. */
export const stamp = (t) => {
  const n = Number(t);
  if (!Number.isFinite(n) || n <= 0) return null;
  return n > 1e12 ? Math.floor(n / 1000) : Math.floor(n);
};

/** The fills, per collection: what was paid, how many times, and the real inscription ids that changed hands. */
export function groupFills(sold) {
  const by = new Map();
  for (const f of sold) {
    let g = by.get(f.collectionId);
    if (!g) {
      g = { id: `unisat:${f.collectionId}`, slug: f.collectionId, name: f.collectionId, volume7dSats: 0, trades7d: 0, items: [] };
      by.set(f.collectionId, g);
    }
    g.volume7dSats += f.priceSats;
    g.trades7d++;
    const wanted = /^[0-9a-f]{64}i\d+$/.test(f.inscriptionId ?? "")
      && (!f.contentType || /^image\//.test(f.contentType))
      && g.items.length < 8
      && !g.items.some((i) => i.id === f.inscriptionId);
    if (wanted) g.items.push({ id: f.inscriptionId, num: null, name: f.itemName ?? null });
  }
  for (const g of by.values()) g.volume7dSats = Math.round(g.volume7dSats);
  return [...by.values()];
}

/**
 * The facts a card needs, per ranked collection: `collection_statistic`, which takes a collection id and no window, so
 * its `btcValue` is the LIFETIME figure and is stored as one. The ranking is never touched here — a collection's place
 * in the list was decided by the fills, and nothing this call returns can change it.
 */
async function describeCollections(groups, key) {
  const out = [];
  for (const g of groups) {
    let s = {};
    try {
      s = await unisat("collection/auction/collection_statistic", key, { collectionId: g.slug });
    } catch (err) {
      log(`unisat statistic ${g.slug}: ${err.message}`);
    }
    out.push({
      id: g.id,
      slug: g.slug,
      name: s?.name ?? g.name,
      kind: "inscriptions",
      description: (s?.desc ?? "").trim().slice(0, 240) || null,
      supply: Number(s?.total ?? s?.supply ?? 0) || null,
      floorSats: Math.round(Number(s?.floorPrice ?? 0)) || 0,
      volume7dSats: g.volume7dSats,
      trades7d: g.trades7d,
      volumeAllTimeSats: Math.round(Number(s?.btcValue ?? 0)) || null,
      listed: Number(s?.listed ?? 0) || null,
      verified: Boolean(s?.verification),
      first: null,
      last: null,
      sampleInscription: g.items[0]?.id ?? null,
      soldItems: g.items,                       // the ids the fills gave; `itemsOf` prefers them to another call
      iconUrl: s?.icon ?? null,
      source: "UniSat",
      sourceUrl: `https://unisat.io/market/collection?collectionId=${g.slug}`,
    });
  }
  return out;
}

/**
 * Ordinals Wallet publishes its collection index without a key. It carries the name, the hosted thumbnail, the floor
 * in sats, the supply and the inscription number range, which is what a listing needs to describe what it buys -- but
 * its only volume figures are lifetime and a `volume_day` that is null on every row. It can therefore order a list; it
 * cannot order one by this week.
 *
 * That is the collections' basis now, not the lesser file's (docs/phase9.md §11.3): no source states a collection's
 * week, so the choice is a lifetime order that says out loud what it is, or no collections at all. The rows come back
 * unordered and `rankByLifetime` does the sorting, the cutting and the refusing.
 */
/**
 * The ranked fifty, then the named ones the index cannot rank, with any duplicate dropped.
 *
 * Appended rather than merged: they have no `volumeAllTimeSats` by design, so they cannot take a place in an order built
 * from it, and pretending otherwise would be the same mistake as ranking a market-wide figure against a one-venue one.
 */
async function collectionsWithNamed(want) {
  const rows = await collectionsKeyless();
  const have = new Set(rows.map((c) => c.slug.toLowerCase()));
  const raw = [];
  const missing = [];
  for (const name of BY_NAME) {
    const c = await collectionByName(name).catch(() => null);
    if (!c?.slug) { missing.push(name); continue; }
    if (have.has(c.slug.toLowerCase())) continue;     // the walk already has it; one row, not two
    raw.push(c);
  }
  const extra = shapeCollections(raw);
  if (missing.length) log(`by name, no exact match: ${missing.join(", ")}`);
  log(`${rows.length} from the walk, ${extra.length} more by name (${extra.map((c) => c.name).join(", ") || "none"})`);
  // one pool, one order: these were rows the walk missed, not a second kind of row
  return rankByLifetime([...rows, ...extra], want, "collections");
}

async function collectionsKeyless() {
  const rows = [];
  for (let page = 0; page < PAGES; page++) {
    const res = await call(
      `https://turbo.ordinalswallet.com/collections?offset=${page * 100}&limit=100`,
    );
    if (!res.ok) throw new Error(`ordinals wallet ${res.status} on page ${page}`);
    const body = await res.json();
    const batch = body?.collections ?? [];
    rows.push(...batch);
    log(`ordinals wallet page ${page + 1}/${PAGES}: ${batch.length} rows`);
    if (batch.length < 100) break;
  }
  return shapeCollections(rows);
}

/**
 * The index's own rows, filtered and shaped. Shared by the walk and by the name lookup, because a row fetched one way and
 * a row fetched the other must be the same row — if the two shapes drifted, a ranking over both would be comparing
 * different things and nothing would say so.
 *
 * The floor filter admits a collection with a real market-wide volume and nothing listed: having no floor this minute is
 * not the same as being no market. Taproot Wizards is the case in point — 27.1bn traded, not one item listed today.
 */
/** A collection's icon inscription id. The index gives it as an id string for some rows and as an object ({ id,
 * content_type }) for others -- Taproot Wizards' only picture arrives the second way, which is how it was dropped. */
const iconInscriptionOf = (c) => (typeof c?.icon_inscription === "string" ? c.icon_inscription : c?.icon_inscription?.id) || null;

export function shapeCollections(rows) {
  return rows
    .filter((c) => c?.name && c?.slug && c?.active !== false)
    .filter((c) => Number(c.total_supply) > 0 && Number(c.total_supply) <= 250_000)
    .filter((c) => Number(c.floor_price) > 0 || Number(c.global_volume_all_time) > 0)
    .filter((c) => c.icon || c.gallery_inscription_id || iconInscriptionOf(c))
    .filter((c) => !/^[A-Z]{12,}$/.test(c.name)) // machine-named noise
    .map((c) => ({
      id: `ow:${c.slug}`,
      slug: c.slug,
      name: c.name,
      kind: "inscriptions",
      description: (c.description ?? "").trim().slice(0, 240) || null,
      supply: Number(c.total_supply),
      floorSats: Number(c.floor_price) || null,   // null is "nothing listed", which the site prints as not known
      volume7dSats: null,                                      // this index publishes no seven-day figure
      // `global_volume_all_time` is every marketplace; `volume_all_time` is Ordinals Wallet's own order book alone, and the
      // two are not close. Bitcoin Frogs: 151.2bn against 7.0bn. Ordinal Maxi Biz: 277.7bn against NULL, because OMB does
      // not trade here — which is how the largest collection in this index by real volume was missing from a list of fifty.
      volumeAllTimeSats: Number(c.global_volume_all_time ?? 0) || null,
      /** kept so the two can be compared, and so the gap above is visible rather than argued */
      volumeOnOrdinalsWalletSats: Number(c.volume_all_time ?? 0) || null,
      listed: Number(c.listed ?? 0) || null,
      verified: Boolean(c.verified),
      first: Number(c.lowest_inscription_num) || null,
      last: Number(c.highest_inscription_num) || null,
      sampleInscription: c.gallery_inscription_id ?? iconInscriptionOf(c),
      iconUrl: c.icon || null,
      source: "Ordinals Wallet",
      sourceUrl: `https://ordinalswallet.com/collection/${c.slug}`,
    }));
}

// ---------------------------------------------------------------------- runes

/**
 * The runes market's own statistic list. `runes_types` is the sibling of the collection call above -- same `timeType`,
 * same envelope -- and it is the reason runes can be ranked at all: its rows carry `tick` and `btcVolume`, the volume
 * traded inside that window.
 *
 * It answers no divisibility, symbol or supply, so ord still describes every rune it names. That division is deliberate:
 * a vault commits to a rune ID, and the reference indexer is the only place an exact one comes from. It does answer a
 * `runeId` of its own, which is kept only to be checked against ord's — two sources disagreeing about which rune a
 * ticker means is the one thing that must never pass quietly into a vault.
 */
async function runeTicksUnisat(key) {
  // a pool wider than fifty, paged, because ord drops what it cannot resolve and a pool of exactly fifty yields fewer
  const want = WANT_RUNES + RUNE_HEADROOM;
  const rows = [];
  for (let start = 0; rows.length < want; start += RUNE_PAGE) {
    const data = await unisat("runes/auction/runes_types", key,
      { timeType: WINDOW, start, limit: RUNE_PAGE });
    const batch = data?.list ?? [];
    rows.push(...batch);
    log(`unisat runes: ${batch.length} rows from ${start}, ${rows.length} of ${want} wanted`);
    if (batch.length < RUNE_PAGE) break;                       // the end of what they will hand over
  }
  const mapped = rows
    .map((r) => {
      const tick = String(r.tick ?? "").replace(/[•\s]/g, "").toUpperCase();
      return {
        id: `tick:${tick}`,
        tick,
        volume7dSats: Math.round(Number(r.btcVolume ?? 0)) || null,
        holders: Number(r.holders ?? 0) || null,
        marketRuneId: /^\d+:\d+$/.test(r.runeId ?? "") ? r.runeId : null,
      };
    })
    .filter((r) => /^[A-Z]+$/.test(r.tick));
  return rankByWeek(mapped, want, "runes");
}

const dd = (html, label) => {
  const m = html.match(
    new RegExp(`<dt>${label}</dt>\\s*<dd[^>]*>([\\s\\S]*?)</dd>`, "i"),
  );
  if (!m) return null;
  return m[1].replace(/<[^>]+>/g, "").trim() || null;
};

/**
 * Runes are named, not pictured. The reference ord server publishes every rune's id, divisibility, symbol, supply and
 * etching block on a public page with no key, which is everything a listing needs and everything the site draws.
 *
 * This is the only thing ord is asked for: given the ticks UniSat ranked, it turns each one into something a vault
 * could commit to. A tick ord cannot resolve is dropped rather than guessed. A floor needs a marketplace and stays
 * null unless UniSat supplies one.
 *
 * It walks the pool in RANKED ORDER and stops at `want` described, so a drop is made up from the next most-traded rune
 * rather than leaving the list short. Nothing below a rune that was kept is ever promoted above it.
 */
async function describeRunes(ticks, disagreed = [], want = Infinity) {
  const out = [];
  for (const row of ticks) {
    if (out.length >= want) break;                     // fifty described, in ranked order: the rest of the pool is spare
    let res;
    try {
      res = await call(`https://ordinals.com/rune/${row.tick}`, { headers: { accept: "text/html" } });
    } catch (err) {
      log(`rune ${row.tick}: ${err.message}`);
      continue;
    }
    if (!res.ok) { log(`rune ${row.tick}: ${res.status}`); continue; }
    const html = await res.text();
    const main = html.slice(html.indexOf("<main>"), html.indexOf("</main>") + 7);
    const name = (main.match(/<h1>([^<]+)<\/h1>/) ?? [])[1]?.trim();
    const id = dd(main, "id");
    if (!name || !id || !/^\d+:\d+$/.test(id)) { log(`rune ${row.tick}: not etched`); continue; }
    const symbol = dd(main, "symbol");
    const divisibility = Number(dd(main, "divisibility") ?? 0);
    const supplyRaw = (dd(main, "supply") ?? "").replace(/[^\d.]/g, "");
    // two sources disagreeing about which rune a ticker means is dropped, not resolved in favour of either
    if (row.marketRuneId && row.marketRuneId !== id) {
      log(`rune ${row.tick}: UniSat says ${row.marketRuneId}, ord says ${id} -- dropped`);
      disagreed.push(`${row.tick} (UniSat ${row.marketRuneId}, ord ${id})`);
      continue;
    }
    out.push({
      id: `rune:${id}`,
      slug: row.tick.toLowerCase(),
      name,                       // already spaced, e.g. DOG•GO•TO•THE•MOON
      kind: "rune",
      runeId: id,
      symbol: symbol && symbol.length <= 4 ? symbol : null,
      divisibility: Number.isFinite(divisibility) ? divisibility : 0,
      supply: supplyRaw || null,
      etchedBlock: Number(id.split(":")[0]),
      mintable: dd(main, "mintable") === "true",
      floorSats: null,            // needs a marketplace; see docs/phase9.md
      volume7dSats: row.volume7dSats ?? null,
      holders: row.holders ?? null,
      source: "ord (ordinals.com)",
      sourceUrl: `https://ordinals.com/rune/${row.tick}`,
    });
    log(`rune ${name} ${id} ${symbol ?? ""}`);
  }
  return out;
}

/**
 * The fallback, and only the fallback: well-known runes, held here, in no order that means anything. It is used when
 * UniSat's runes statistic list cannot be read, and the file and the site then say in plain words that the runes are a
 * curated list rather than a ranking. Enumerating ord instead is not an alternative, because ord lists runes in
 * etching order, which is mostly machine-named noise.
 */
const RUNE_SEEDS = [
  "DOGGOTOTHEMOON", "ZZZZZFEHUZZZZZ", "SATOSHINAKAMOTO", "RSICGENESISRUNE",
  "PUPSWORLDPEACE", "BILLIONDOLLARCAT", "MEMEECONOMICS", "EPICEPICEPICEPIC",
  "LOBOTHEWOLF", "UNCOMMONGOODS", "RUNESXBITCOIN", "THEBLACKCAT",
  "DECENTRALIZEDDOG", "BITCOINISSTOLENGOLD", "WANKOMANEKOSAN", "ANARCHOCATBUS",
  "RUNESTONE", "LIQUIDIUMTOKEN", "GIZMOIMAGINARYKITTEN", "ORDINALSISBITCOIN",
  "CYPHERPUNKSWRITECODE", "MAKEBITCOINGREATAGAIN", "DOGECOINISGOODASGOLD",
  "ALPHAALPHAALPHAALPHA", "FEHUFEHUFEHUFEHU", "PIZZAPIZZAPIZZAPIZZA",
  "BITCOINFROGSRUNE", "NIKOLATESLAGENIUS", "THEPEPEOFWALLSTREET", "MEMERUNESRUNES",
];
const seedRows = () => RUNE_SEEDS.map((tick) => ({ id: `tick:${tick}`, tick, volume7dSats: null, holders: null }));

/** With a key, UniSat can price the runes ord described. Only the ones still without a floor are asked for. Reads only. */
async function runeFloorsUnisat(runes, key) {
  let priced = 0, asked = 0;
  for (const rune of runes) {
    if (rune.floorSats) continue;
    asked++;
    let row;
    try {
      const data = await unisat("runes/auction/runes_types_specified", key,
        { tick: rune.name.replace(/•/g, ""), timeType: WINDOW });
      row = data?.detail?.[0] ?? data;
    } catch { continue; }
    const floor = Number(row?.floorPrice ?? 0);
    if (floor > 0) {
      rune.floorSats = floor;
      rune.floorSource = "UniSat";
      priced++;
    }
    if (!rune.holders && row?.holders) rune.holders = Number(row.holders);
  }
  log(`unisat priced ${priced}/${asked} runes`);
}

// ----------------------------------------------------------------- thumbnails

const MAGIC = [
  [[0x89, 0x50, 0x4e, 0x47], "png"],
  [[0xff, 0xd8, 0xff], "jpg"],
  [[0x47, 0x49, 0x46, 0x38], "gif"],
  [[0x52, 0x49, 0x46, 0x46], "webp"], // RIFF....WEBP
];

function imageKind(bytes) {
  for (const [sig, ext] of MAGIC) {
    if (sig.every((b, i) => bytes[i] === b)) {
      if (ext === "webp" && String.fromCharCode(...bytes.slice(8, 12)) !== "WEBP") continue;
      return ext;
    }
  }
  const head = new TextDecoder().decode(bytes.slice(0, 400)).trimStart().toLowerCase();
  if (head.startsWith("<svg") || head.startsWith("<?xml")) return "svg";
  return null;
}

/**
 * The real inscription ids a collection contains. They are what the cards draw from and what a membership root is
 * built over (docs/phase11.md), so a catalogue that dropped them would quietly empty the vault list.
 *
 * Ordinals Wallet publishes a collection's own inscriptions with no key. UniSat's equivalent is
 * `collection/auction/list` -- the market's own order book for one collection, whose rows carry `inscriptionId`. Both
 * are reads: nothing here places, changes or cancels an order.
 */
async function itemsOf(entry, key) {
  if (entry.source === "UniSat") {
    if (!key) return [];
    // the fills that ranked this collection already named real ids; another call would ask for what we have
    if (entry.soldItems?.length) return entry.soldItems;
    try {
      const data = await unisat("collection/auction/list", key, {
        filter: { nftType: "collection", collectionId: entry.slug },
        sort: { unitPrice: 1 }, start: 0, limit: 8,
      });
      return (data?.list ?? [])
        .filter((r) => /^[0-9a-f]{64}i\d+$/.test(r?.inscriptionId ?? ""))
        .filter((r) => !r.contentType || /^image\//.test(r.contentType))
        .map((r) => ({ id: r.inscriptionId, num: Number(r.inscriptionNumber) || null, name: r.collectionItemName ?? null }));
    } catch (err) {
      log(`unisat items ${entry.slug}: ${err.message}`);
      return [];
    }
  }
  if (entry.source === "Ordinals Wallet") {
    try {
      const res = await call(
        `https://turbo.ordinalswallet.com/collection/${entry.slug}/inscriptions?offset=0&limit=8`,
      );
      if (res.ok) {
        const rows = await res.json();
        return (Array.isArray(rows) ? rows : [])
          .filter((r) => r?.id && /^image\//.test(r.content_type ?? ""))
          .map((r) => ({
            id: r.id,
            num: Number(r.num) || null,
            name: r.meta?.name ?? null,
          }));
      }
    } catch { /* fall through */ }
  }
  return [];
}

/** Largest download worth shrinking; past it the picture is not worth the fetch. */
const SHRINK_FROM_BYTES = 8_000_000;
/** A 320-pixel-wide webp of `buf` through cwebp, or null when cwebp is missing or cannot read it (it does not read GIF). */
async function shrink(buf, ext) {
  const { execFile } = await import("node:child_process");
  const { readFile, unlink } = await import("node:fs/promises");
  const { tmpdir } = await import("node:os");
  const base = join(tmpdir(), `catalogue-${process.pid}-${Math.random().toString(36).slice(2)}`);
  const src = `${base}.${ext}`, dst = `${base}.webp`;
  try {
    await writeFile(src, buf);
    await new Promise((ok, fail) => execFile("cwebp", ["-quiet", "-resize", "320", "0", "-q", "82", src, "-o", dst], (e) => (e ? fail(e) : ok())));
    return new Uint8Array(await readFile(dst));
  } catch {
    return null;
  } finally {
    await unlink(src).catch(() => {});
    await unlink(dst).catch(() => {});
  }
}

async function grab(url, slug, tag, cap) {
  try {
    const res = await call(url);
    if (!res.ok) return null;
    let buf = new Uint8Array(await res.arrayBuffer());
    if (buf.byteLength === 0 || buf.byteLength > SHRINK_FROM_BYTES) return null;
    let ext = imageKind(buf);
    if (!ext) return null;
    // Over the cap: shrink it to a small webp rather than drop it. Quantum Cats' only picture is a 788 KB PNG, and dropping
    // it is how a top-ten collection ended up drawn as a monogram. Still too big, or no cwebp on this machine: dropped as before.
    if (buf.byteLength > cap) {
      const small = await shrink(buf, ext);
      if (!small || small.byteLength > cap) return null;
      buf = small;
      ext = "webp";
    }
    const file = `${slug.replace(/[^a-z0-9-]/gi, "-").toLowerCase()}${tag}.${ext}`;
    await writeFile(join(IMG, file), buf);
    return { path: `${IMG_URL}/${file}`, bytes: buf.byteLength };
  } catch {
    return null;
  }
}

/**
 * Real artwork, in order of how real it is.
 *
 * Best is an item the collection actually contains, fetched from the reference ord content gateway: that is the thing a
 * vault would hold, served by the same software that decides what an inscription is. Next is the marketplace's own
 * thumbnail. Last is nothing, and the site draws its carved monogram rather than a broken picture.
 */
async function artworkFor(entry, key) {
  const items = await itemsOf(entry, key);
  const previewId = (entry.iconUrl ?? "").match(/\/inscription\/preview\/([a-f0-9]+i\d+)/i)?.[1];
  const ids = [
    ...items.map((i) => i.id),
    entry.sampleInscription,
    previewId,
  ].filter(Boolean);

  const kept = [];
  const from = [];
  for (const id of ids) {
    if (kept.length >= 1) break;
    const got = await grab(
      `https://ordinals.com/content/${id}`,
      entry.slug,
      `-${kept.length}`,
      MAX_IMAGE_BYTES,
    );
    if (got) { kept.push(got.path); from.push("ord content gateway"); }
  }

  if (!kept.length && entry.iconUrl && /^https?:\/\//.test(entry.iconUrl) && !entry.iconUrl.includes("/preview/")) {
    const got = await grab(entry.iconUrl, entry.slug, "-0", MAX_IMAGE_BYTES);
    if (got) { kept.push(got.path); from.push(entry.source); }
  }

  return {
    image: kept[0] ?? null,
    imageFrom: from[0] ?? null,
    // Real item names, so a card can say what it is showing rather than guessing.
    items: items.slice(0, 3).map((i) => ({ id: i.id, num: i.num, name: i.name })),
  };
}

// ------------------------------------------------------------------------ run

async function main() {
  // `UNISAT` too, because that is what the name is on the Railway service and a run that refuses over a variable's spelling
  // is a failure of this script rather than of the operator. The refusal above still names UNISAT_API_KEY first.
  const key = (process.env.UNISAT_API_KEY ?? process.env.UNISAT)?.trim();
  const allowUnranked = process.env.CATALOGUE_ALLOW_UNRANKED === "1";
  const notes = [];

  if (!key) {
    if (!allowUnranked) { console.error(REFUSAL); process.exit(1); }
    IMG = join(OUT, "img-unranked");
    IMG_URL = "/catalogue/img-unranked";
    OUT_FILE = join(OUT, "catalogue.unranked.json");
    log(`no key: writing ${OUT_FILE}, which the site does not read. It is NOT ranked by this week. Do not rename it.`);
  }

  let collections, collectionBasis, collectionSource;
  let runes, runeBasis, runeSource;

  if (key) {
    // THE WEEK IS ASKED FOR FIRST, every run, because the day UniSat's collection market comes back is the day the
    // collections should be on a week again and nobody should have to notice (docs/phase9.md §11.3). It is TAKEN only
    // when it can fill the fifty: a window that names four collections is not a top fifty of anything, and half a list
    // of this week beside half a list of all time would be the §11 defect with extra steps.
    const dontAsk = process.env.CATALOGUE_COLLECTIONS_LIFETIME === "1";
    let weekly = null, fellBack = null;
    if (dontAsk) {
      fellBack = "the fetch was told not to ask (CATALOGUE_COLLECTIONS_LIFETIME=1)";
    } else {
      try {
        const fills = await collectionFillsUnisat(key);
        log(`collections: ${fills.cover.trades} fills over ${fills.cover.days} days (${fills.cover.pages} pages), `
          + `${fills.cover.full ? "the whole window" : "LESS than the seven asked for"}`);
        const ranked = rankByWeek(groupFills(fills.sold), WANT_COLLECTIONS, "collections");
        if (ranked.length >= WANT_COLLECTIONS) weekly = { ranked, cover: fills.cover };
        else fellBack = `UniSat's own fills -- the only weekly figure obtainable anywhere -- named ${ranked.length} `
          + `collections over ${fills.cover.trades} fills in ${fills.cover.days} days, which is not a top `
          + `${WANT_COLLECTIONS}`;
      } catch (err) {
        fellBack = `UniSat's fills could not rank them by the week (${err.message})`;
      }
    }

    if (weekly) {
      collections = await describeCollections(weekly.ranked, key);
      collectionBasis = summed(weekly.cover);
      collectionSource = "UniSat";
      if (!weekly.cover.full) {
        notes.push(`The collections' figure covers ${weekly.cover.days} days, not seven: UniSat's action feed ran out `
          + `after ${weekly.cover.pages} pages before reaching the cutoff. The window it does cover is what is stated, `
          + "and CATALOGUE_ACTION_PAGES raises the stop.");
      }
    } else {
      collections = await collectionsWithNamed(WANT_COLLECTIONS);
      collectionBasis = lifetimeBasis(fellBack);
      collectionSource = "Ordinals Wallet";
      notes.push("COLLECTIONS ARE ORDERED BY LIFETIME VOLUME, NOT BY THIS WEEK. No marketplace states a Bitcoin "
        + "collection's weekly volume -- UniSat's windowed endpoint answers an empty list for every window, its "
        + "per-collection one takes no window, and Magic Eden's whole Bitcoin API answers nothing live (docs/phase9.md "
        + `§11.1, §11.2). The week was asked for on this run anyway, and ${fellBack}. So the collections are ordered by `
        + "the whole history of a market and say so here, in `rankedBy`, in `ranking.collections` and on every screen. "
        + "The runes below ARE this week's. Two lists, two windows, each said out loud: nothing here prints one as the "
        + "other.");
      if (collections.length < WANT_COLLECTIONS) {
        notes.push(`${collections.length} collections carry a lifetime volume in Ordinals Wallet's index, not `
          + `${WANT_COLLECTIONS}. This is every one that does, rather than padded out from somewhere else.`);
      }
    }

    const disagreed = [];
    try {
      const ticks = await runeTicksUnisat(key);
      runes = await describeRunes(ticks, disagreed, WANT_RUNES);
      runeBasis = RANKED;
      runeSource = "UniSat (the ranking) and ord (ordinals.com) (every rune fact)";
      if (disagreed.length) {
        const named = disagreed.slice(0, 10).join("; ");
        notes.push(`${disagreed.length} runes were dropped because UniSat and ord disagree about which rune the ticker `
          + `names: ${named}${disagreed.length > 10 ? `, and ${disagreed.length - 10} more` : ""}. A vault commits to a `
          + "rune id, so a disputed one is not offered.");
      }
      if (runes.length >= WANT_RUNES && ticks.length > WANT_RUNES) {
        notes.push(`The runes are the ${runes.length} most-traded of the week that ord could resolve. UniSat ranked a `
          + `pool of ${ticks.length} tickers and the list was filled from the top of it in order, so a ticker ord could `
          + "not resolve -- or resolved to a different rune id than UniSat named -- was made up from the next "
          + "most-traded rune rather than leaving the list short. Nothing below a rune that was kept was promoted "
          + "above it.");
      }
      if (runes.length < WANT_RUNES) {
        notes.push(`${runes.length} runes, not ${WANT_RUNES}: UniSat ranked ${ticks.length} tickers and `
          + `${ticks.length - runes.length} of them were dropped rather than guessed -- ${disagreed.length} because `
          + `UniSat and ord disagree about which rune the ticker names, and ${ticks.length - runes.length - disagreed.length} `
          + "because ord could not resolve the ticker at all.");
      }
    } catch (err) {
      notes.push("RUNES ARE A CURATED LIST, NOT A RANKING. UniSat's runes statistic list could not be read "
        + `(${err.message}), so the runes here are the well-known ones this script holds, described by ord. Their `
        + "order means nothing and no volume figure is claimed for any of them.");
      runes = await describeRunes(seedRows());
      runeBasis = CURATED;
      runeSource = "ord (ordinals.com)";
    }
  } else {
    collections = await collectionsWithNamed(WANT_COLLECTIONS);
    collectionBasis = lifetimeBasis("there was no UNISAT_API_KEY on this run, so nothing here was asked for a week at all");
    collectionSource = "Ordinals Wallet";
    runes = await describeRunes(seedRows(), [], WANT_RUNES);
    runeBasis = CURATED;
    runeSource = "ord (ordinals.com)";
    notes.push("THIS FILE IS NOT RANKED BY THIS WEEK'S VOLUME and the site does not read it. There was no "
      + "UNISAT_API_KEY, so the collections are ordered by lifetime volume from the keyless index -- which ranks the "
      + "whole history of a market rather than the last seven days, and which is the committed file's basis too "
      + "(docs/phase9.md §11.3) -- and the RUNES are a curated list in no order at all, which is what makes this file "
      + "the lesser one.");
    notes.push("Runes carry no floor: a floor needs a marketplace, and every keyless one refuses unauthenticated "
      + "reads. Their id, symbol, divisibility and supply are the reference indexer's own and are exact.");
  }

  if (key) {
    try { await runeFloorsUnisat(runes, key); }
    catch (err) { notes.push(`UniSat rune pricing failed (${err.message}); those runes carry no floor.`); }
    if (!runes.some((r) => r.floorSats)) {
      notes.push("No rune carries a floor: UniSat's runes calls answered none, so the site prints no price for one "
        + "rather than inventing a number no marketplace gave.");
    }
  }

  await rm(IMG, { recursive: true, force: true });
  await mkdir(IMG, { recursive: true });

  let withArt = 0;
  for (const entry of collections) {
    const art = await artworkFor(entry, key);
    Object.assign(entry, art);
    delete entry.iconUrl;
    delete entry.soldItems;             // a working list, not a fact: `items` is what the cards read
    if (art.image) withArt++;
  }
  log(`artwork: ${withArt}/${collections.length}`);
  if (withArt < collections.length) {
    notes.push(`${collections.length - withArt} collections had no usable image and draw their carved monogram instead.`);
  }

  // one sentence only when both lists really are on the same window; two clauses whenever they are not
  const both = collectionBasis.window === WINDOW && runeBasis.window === WINDOW;
  const catalogue = {
    fetchedAt: new Date().toISOString(),
    // The ranking, in the data, so the site can state it and nothing downstream has to infer a window.
    rankedBy: both
      ? `UniSat ${WINDOW} volume, for collections and runes alike`
      : `collections: ${collectionBasis.said}. Runes: ${runeBasis.said}`,
    ranking: {
      // the window asked for. Each side's own `window` is the one it actually got, which is not always this.
      window: WINDOW,
      unit: "sats",
      // `kept` is what the RANKING kept, not what the file holds: the by-name rows were never in the order, and counting
      // them here would have the file claim it ranked fifty-five when it ranked fifty. `named` says how many came the
      // other way, so the two numbers add up in public.
      collections: { ...collectionBasis, want: WANT_COLLECTIONS,
        kept: collections.filter((c) => !c.byName).length,
        named: collections.filter((c) => c.byName).length },
      runes: { ...runeBasis, want: WANT_RUNES, kept: runes.length },
    },
    collectionSource,
    runeSource,
    attribution: [
      { name: "UniSat", url: "https://unisat.io", used: !key ? "not used on this fetch"
        : [
            collectionSource === "UniSat" ? "the collection ranking, summed from its own market's fills" : null,
            collectionSource === "UniSat" ? "each collection's facts, floor and lifetime volume" : null,
            collectionSource === "UniSat" ? "the inscription ids behind the artwork" : null,
            "the runes' seven-day ranking",
            runes.some((r) => r.floorSource === "UniSat") ? "the rune floors it gave one for" : null,
          ].filter(Boolean).join(", ") },
      { name: "Ordinals Wallet", url: "https://ordinalswallet.com", used: collectionSource === "Ordinals Wallet"
        ? "the collection list and its ranking by lifetime volume, its floors, its thumbnails, and the inscription ids "
          + "behind the artwork"
        : "not used on this fetch" },
      { name: "ord", url: "https://ordinals.com", used: "rune facts and inscription artwork" },
    ],
    notes,
    collections,
    runes,
  };

  await writeFile(OUT_FILE, JSON.stringify(catalogue, null, 2) + "\n");
  log(`wrote ${collections.length} collections and ${runes.length} runes to ${OUT_FILE}, fetched ${catalogue.fetchedAt}`);
  log(`ranked by: ${catalogue.rankedBy}`);
  for (const note of notes) log(`note: ${note}`);
}

// Only when run, so the ranking above can be imported and read by a test without a network, a key or a write.
if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  main().catch((err) => {
    console.error("[catalogue] failed:", err);
    process.exit(1);
  });
}
