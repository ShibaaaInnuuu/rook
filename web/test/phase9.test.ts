import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { chiselOf, initials, seedOf, ANGLES } from "../src/lib/plate.js";
import { RANGES, seriesFor, stepPath, type Point } from "../src/lib/spark.js";
import { freshKeys, keyOf, newestFirst } from "../src/lib/feed.js";
import { asOf, choices, isRune, matchChoices, rankedFigure, rankingLine, sats, searchCatalogue, runeTicker, vaultOf,
  vaultsByRef, vaultTag, type Catalogue, type CatalogueEntry, type ChoosableVault,
  type Ranking } from "../src/lib/catalogue.js";
import { search, type CoinRow, type Offered } from "../src/lib/coinlist.js";
import { logoSrc } from "../src/lib/plate.js";
import { CANONICAL, KNOWN, NAV, UNDER } from "../src/lib/routes.js";
import { groupFills, rankByLifetime, rankByWeek, shapeCollections, stamp } from "../scripts/catalogue.mjs";
import { backoffAfter, classify, isCacheGhost, MAGICEDEN_GAP_MS, ROUTES, rowsIn, verdictOf,
  type Outcome as MeOutcome, type Result as MeResult } from "../scripts/magiceden.mjs";

// ---- the picture that is missing, slow, broken, or not a picture ----------------------------------------------------

test("a coin with no logo is cut the same way every time, and no two are cut alike", () => {
  const a = "0x97e4A02a1b8c4E8b9F3d2A1c0B9e8D7c6A5b4C3d";
  const b = "0x718d6ca1F2e3D4c5B6a7980716253443526170Ff";
  assert.deepEqual(chiselOf(a), chiselOf(a), "the same address must always cut the same plate");
  assert.notDeepEqual(chiselOf(a), chiselOf(b));
  assert.ok(ANGLES.includes(chiselOf(a).angle), "a mason works to one of four bevels, never a free angle");
  assert.ok(chiselOf(b).gap >= 7 && chiselOf(b).gap <= 13);
  assert.equal(seedOf(""), 0x811c9dc5, "the empty string has to have a defined seed, not NaN");
});

test("the monogram takes the initials a mason would, whatever it is handed", () => {
  assert.equal(initials("Smoke Stone"), "SS");
  assert.equal(initials("SMOKE"), "SM");
  assert.equal(initials("rfrf"), "RF");
  assert.equal(initials("DEMO•RUNE"), "DR", "a rune's bullets are separators, not letters");
  assert.equal(initials("x"), "X");
  assert.equal(initials(""), "··", "an unnamed thing still gets a plate rather than an empty hole");
  assert.equal(initials("•••"), "··");
  assert.equal(initials("🐕 dog"), "DO", "an emoji is not a letter to carve");
});

// ---- the line: the ranges really do change it ------------------------------------------------------------------------

const NOW = 1_800_000_000;
const pts: Point[] = [
  { t: NOW - 2_000, v: 100 },        // inside 24 hours
  { t: NOW - 100_000, v: 50 },       // inside 7 days, outside 24 hours
  { t: NOW - 3_000_000, v: 25 },     // outside both
];

test("each range is computed from the points inside it, so the line is redrawn and not zoomed", () => {
  const day = seriesFor(pts, RANGES[0], NOW);
  const week = seriesFor(pts, RANGES[1], NOW);
  const all = seriesFor(pts, RANGES[2], NOW);

  assert.equal(day.at(-1)!.v, 100, "24 hours counts only what settled in it");
  assert.equal(week.at(-1)!.v, 150);
  assert.equal(all.at(-1)!.v, 175);
  assert.notDeepEqual(day, week, "changing the range must change the series, not just the axis");

  for (const s of [day, week, all]) {
    assert.equal(s[0].v, 0, "every window opens at nothing and accumulates");
    assert.equal(s.at(-1)!.t, NOW, "the line reaches the right-hand edge rather than stopping short");
    const rising = s.every((p, i) => i === 0 || p.v >= s[i - 1].v);
    assert.ok(rising, "a running total never goes down");
  }
  assert.equal(seriesFor(pts, RANGES[2], NOW)[0].t, NOW - 3_000_000, "All starts at the first thing that happened");
});

test("an empty window draws nothing rather than a flat invented line", () => {
  assert.deepEqual(seriesFor([], RANGES[1], NOW), []);
  assert.deepEqual(seriesFor([{ t: NOW - 3_000_000, v: 9 }], RANGES[0], NOW), [], "a point outside the window is not dragged into it");
  assert.deepEqual(seriesFor([{ t: NOW + 5_000, v: 9 }], RANGES[2], NOW), [], "nothing settles in the future");
});

test("a running total steps at each settlement instead of sloping between them", () => {
  const d = stepPath([{ x: 0, y: 100 }, { x: 10, y: 60 }, { x: 20, y: 20 }]);
  assert.equal(d, "M0.00 100.00 L10.00 100.00 L10.00 60.00 L20.00 60.00 L20.00 20.00");
  assert.equal(stepPath([{ x: 0, y: 5 }, { x: 8, y: 5 }]), "M0.00 5.00 L8.00 5.00 L8.00 5.00", "a flat stretch stays flat");
  assert.equal(stepPath([{ x: 3, y: 4 }]), "M3.00 4.00", "one point is a point, not a line");
});

// ---- the feed --------------------------------------------------------------------------------------------------------

const settled = (listingId: number, acquisitionId: number, settledAt: number | null) => ({ listingId, acquisitionId, settledAt });

test("the feed is newest first, and a row with no time sorts last rather than to the top", () => {
  const rows = [settled(0, 1, 500), settled(0, 2, null), settled(1, 3, 900), settled(0, 4, 700)];
  assert.deepEqual(newestFirst(rows, 10).map(keyOf), ["1-3", "0-4", "0-1", "0-2"]);
  assert.deepEqual(newestFirst(rows, 2).map(keyOf), ["1-3", "0-4"]);
  assert.equal(newestFirst(rows, 10).length, 4);
  assert.deepEqual(rows.map(keyOf), ["0-1", "0-2", "1-3", "0-4"], "sorting must not disturb what it was given");
});

test("nothing is cut in on the first pass; only a row that arrived since is", () => {
  assert.deepEqual([...freshKeys(["a", "b"], null)], [], "a first render is not news");
  assert.deepEqual([...freshKeys(["c", "a", "b"], new Set(["a", "b"]))], ["c"]);
  assert.deepEqual([...freshKeys(["a", "b"], new Set(["a", "b"]))], []);
});

// ---- the search, over what is on chain and what is merely available --------------------------------------------------

const coin = (name: string): CoinRow => ({
  address: `0x${name.toLowerCase().padEnd(40, "0")}` as `0x${string}`, name, symbol: name.slice(0, 3).toUpperCase(),
  listingId: 0, listingName: "Demo Stones", creator: "0x0" as `0x${string}`, router: "0x0" as `0x${string}`,
  launchedAt: 1, lifetimeFeesWei: 0n, fees24hWei: 0n, harvests: 0, toVaultWei: 0n, creatorTaxBps: 200,
});
const offered: Offered[] = [
  { id: "ow:bitcoin-frogs", slug: "bitcoin-frogs", name: "Bitcoin Frogs", kind: "inscriptions" },
  { id: "rune:840000:3", slug: "doggotothemoon", name: "DOG•GO•TO•THE•MOON", kind: "rune", runeId: "840000:3" },
];

test("the search answers over the catalogue too, so the box still works with nothing deployed", () => {
  const hits = search("frog", [], [], [], offered);
  assert.equal(hits.length, 1);
  assert.equal(hits[0].kind, "offered");
  assert.equal(hits[0].sub, "collection");
  assert.equal(hits[0].state, "no vault", "a dropdown must never let a vault and a collection without one look alike");
  assert.equal(hits[0].to, "vaults/bitcoin-frogs", "a catalogue hit opens the page that separates the two");

  const bullets = search("doggotothemoon", [], [], [], offered);
  assert.equal(bullets.length, 1, "a rune is findable by its ticker, without typing the bullets");
  assert.equal(search("840000:3", [], [], [], offered).length, 1, "and by its rune id");
  assert.equal(search("f", [], [], [], offered).length, 0, "one character is not a search");
});

test("what is listed on chain outranks what is only available, and the two are never both offered", () => {
  const listing = { id: 0, kind: "inscriptions" as const, name: "Bitcoin Frogs", ref: "bitcoin-frogs", divisibility: 0,
    vault: "0x0" as `0x${string}`, btcVault: null, membershipRoot: null, paused: false, coins: 1, depositedWei: "0",
    acquisitions: 0, capWei: null, capExpiry: null, capExpired: null, capPostedAt: null, openDeclarations: 0 , autonomous: false, buyer: null, maxInFlightWei: null };
  const hits = search("frog", [], [listing], [], offered);
  assert.equal(hits.length, 1, "the same collection must not appear twice");
  assert.equal(hits[0].kind, "listing", "the one with a vault wins");
  assert.equal(hits[0].state, "vault");
});

test("a coin still outranks the catalogue it is paired with", () => {
  const hits = search("frog", [coin("Frogcoin")], [], [], offered);
  assert.equal(hits[0].kind, "coin");
  assert.equal(hits[1].kind, "offered");
});

// ---- the catalogue itself ---------------------------------------------------------------------------------------------

test("a price is said with its units, and an absent one is not said as zero", () => {
  assert.equal(sats(10_000), "10,000 sats");
  assert.equal(sats(411_099), "411,099 sats");
  assert.equal(sats(250_000_000), "2.50 BTC");
  assert.equal(sats(null), "not known");
  assert.equal(sats(0), "not known", "a floor of nothing is a floor nobody posted");
  assert.equal(sats(undefined), "not known");
  assert.equal(sats(Number.NaN), "not known");
});

test("a fetched-on date is printed, and a missing one is admitted", () => {
  assert.equal(asOf("2026-09-21T20:03:31.389Z"), "21 September 2026");
  assert.equal(asOf(undefined), "an earlier fetch");
  assert.equal(asOf("not a date"), "an earlier fetch");
});

// ---- docs/phase9.md §11: what the order of the catalogue means, in the file's own words ------------------------------

const rank = (over: Partial<Ranking> = {}): Ranking => ({
  window: "day7", unit: "sats",
  collections: { said: "UniSat's volume over the seven days before this file was fetched", field: "volume7dSats", source: "UniSat", window: "day7", want: 50, kept: 50 },
  runes: { said: "UniSat's volume over the seven days before this file was fetched", field: "volume7dSats", source: "UniSat", window: "day7", want: 50, kept: 47 },
  ...over,
});
const withRanking = (r: Ranking | undefined) => ({ fetchedAt: "2026-09-21T20:03:31.389Z", ranking: r } as unknown as Catalogue);

test("a ranked list says what it is ranked by, and the figure and the window are both in the sentence", () => {
  const line = rankingLine(withRanking(rank()));
  assert.match(line, /^The top 50 collections by/);
  assert.match(line, /seven days/, "a window nobody can read is a window nobody can check");
  assert.match(line, /the top 47 runes by/, "and it says how many of each, not a round number it was asked for");
});

test("a curated list is never printed as a ranking, and neither is a lifetime one", () => {
  const curated = rankingLine(withRanking(rank({
    runes: { said: "a curated list this script holds, in no meaningful order", field: null, source: "ord", window: null, want: 50, kept: 30 },
  })));
  assert.match(curated, /30 runes, which are a curated list/, "plain words: these were chosen, not ranked");
  assert.ok(!/top 30 runes/.test(curated), "and nothing in the sentence calls them a top thirty");

  const lifetime = rankingLine(withRanking(rank({
    collections: { said: "lifetime volume, which is NOT this week", field: "volumeAllTimeSats", source: "Ordinals Wallet", window: null, want: 50, kept: 50 },
  })));
  assert.match(lifetime, /NOT this week/, "a lifetime ranking must say so where the ranking is stated");
});

test("a catalogue that records no basis says it records none, rather than letting its order be read as one", () => {
  assert.match(rankingLine(withRanking(undefined)), /records no ranking basis/);
  assert.equal(rankingLine(undefined), "", "and with no catalogue at all there is nothing to say");
});

test("runes keep their bullets on screen and lose them in a ticker", () => {
  assert.equal(runeTicker("DOG•GO•TO•THE•MOON"), "DOGGOTOTHEMOON");
  assert.equal(runeTicker("RUNESTONE"), "RUNESTONE");
});

test("the browser's own search ranks a prefix above a mention, and offers everything with an empty query", () => {
  const entries = [
    { id: "a", slug: "pixel-pepes", name: "Pixel Pepes", kind: "inscriptions", description: null } as unknown as CatalogueEntry,
    { id: "b", slug: "clay-pepes", name: "Clay Pepes", kind: "inscriptions", description: "pixel art" } as unknown as CatalogueEntry,
  ];
  assert.deepEqual(searchCatalogue("pixel", entries).map((e) => e.name), ["Pixel Pepes", "Clay Pepes"]);
  assert.equal(searchCatalogue("", entries).length, 2);
  assert.equal(searchCatalogue("nothing here", entries).length, 0);
  assert.equal(searchCatalogue("pepes", entries, 1).length, 1, "the limit is honoured");
});

// ---- the committed file ------------------------------------------------------------------------------------------------

const SHIPPED = JSON.parse(readFileSync(new URL("../public/catalogue/catalogue.json", import.meta.url), "utf8")) as Catalogue;

test("the catalogue that ships is real, dated, credited, and every picture it names exists", () => {
  const c = SHIPPED;

  assert.ok(c.collections.length >= 20, "a chooser with a handful of collections is the problem, not the fix");
  assert.ok(!Number.isNaN(Date.parse(c.fetchedAt)), "a price without a date is a price pretending to be live");
  assert.ok(c.attribution.length >= 2 && c.attribution.every((a) => a.name && a.url.startsWith("https://")));

  // a collection is here for its volume, so one with nothing listed today is still here and carries no floor; the site
  // prints that as "not known" rather than as free
  for (const col of c.collections) {
    // null IS "none at all", which is what the comment above has always said and what the site prints as "not known". A
    // collection with nothing listed has no floor, and 0 would say it was free.
    assert.ok(col.floorSats === null || (Number.isFinite(col.floorSats) && col.floorSats > 0),
      `${col.name}: a floor is a positive number of sats or null, never 0 and never NaN`);
    if (!col.floorSats) assert.equal(sats(col.floorSats), "not known");
    assert.ok(col.sourceUrl.startsWith("https://"), `${col.name} says where it came from`);
  }
  const pictured = c.collections.filter((col) => col.image);
  assert.ok(pictured.length >= Math.ceil(c.collections.length * 0.8), "most of the catalogue carries real artwork, or the cards are a wall of monograms");
  for (const col of pictured) {
    assert.ok(col.image!.startsWith("/catalogue/img/"), "artwork is committed here, not fetched from a marketplace at view time");
    readFileSync(new URL(`../public${col.image}`, import.meta.url)); // throws if the picture named is not in the build
  }
  if (pictured.length < c.collections.length) {
    assert.ok(c.notes.some((n) => /no usable image/.test(n)), "and what has none is written down rather than left to be noticed");
  }

  for (const r of c.runes) {
    assert.ok(isRune(r));
    assert.match(r.runeId, /^\d+:\d+$/, `${r.name} carries a real rune id`);
    assert.ok(r.name.length > 0 && r.divisibility >= 0);
    assert.ok(r.floorSats === null || (r.floorSats > 0 && !!r.floorSource),
      "a rune floor needs a marketplace: either there is none, or the marketplace that gave it is named");
  }
  assert.ok(c.notes.length > 0, "whatever could not be fetched is written down");
});

test("the catalogue is in the order it claims, and a file that claims none is not read as a ranking", () => {
  const c = SHIPPED;

  // docs/phase9.md §11. This branch is the committed file as it stands: fetched before the basis was recorded, so it
  // must carry no seven-day figure either, and the site must say its order is not a ranking.
  if (!c.ranking) {
    assert.match(rankingLine(c), /records no ranking basis/);
    assert.ok(c.collections.every((col) => col.volume7dSats == null),
      "a file with no basis must not carry the field a basis would be read from");
    return;
  }

  assert.equal(c.ranking.window, "day7", "this week means seven days, and the window is the marketplace's own name for it");
  assert.equal(c.ranking.unit, "sats");
  assert.match(c.rankedBy ?? "", /day7|seven/, "the one-sentence basis says the window too");
  for (const [side, all] of [[c.ranking.collections, c.collections], [c.ranking.runes, c.runes]] as const) {
    // A collection may also be in the file BY NAME — Ordinals Wallet's paginated index omits the best-known ones, and the
    // per-collection endpoint that does answer for them publishes only that venue's own volume, which is not the
    // market-wide figure the ranking uses. They carry no `volumeAllTimeSats` and cannot be ordered against rows that do,
    // so the rule is not "never more than fifty" but: fifty ranked, every named one after them, and no named row
    // smuggling a figure that would let it look ranked.
    const named = all.filter((r) => (r as { byName?: boolean }).byName);
    const rows = all.filter((r) => !(r as { byName?: boolean }).byName);
    assert.ok(side.kept <= 50 && side.want === 50, "the top fifty of each kind, and never more");
    assert.equal(side.kept, rows.length, "and the count it claims is the count it RANKED");
    assert.deepEqual(all.slice(0, rows.length), rows, "every named row comes after every ranked one");
    for (const n of named) {
      assert.equal((n as { volumeAllTimeSats?: number | null }).volumeAllTimeSats ?? null, null,
        `${n.name} is here by name and must carry no figure that would let it pass for ranked`);
    }
    if (!side.field) continue;                       // a curated list has no order to check

    // docs/phase9.md §11.3: two bases, and the ONE rule is that each list is sorted by the field it says it was sorted
    // by, and that a lifetime order says lifetime in the same sentence a reader sees the order in.
    assert.ok(side.field === "volume7dSats" || side.field === "volumeAllTimeSats", `an unknown basis: ${side.field}`);
    if (side.field === "volume7dSats") {
      assert.equal(side.window, "day7", "a weekly figure names its window");
    } else {
      assert.equal(side.window, null, "a lifetime figure has no window, and must not borrow the week's");
      assert.match(side.said, /NOT this week/, "a lifetime order says so where the order is stated");
      assert.match(rankingLine(c), /NOT this week/, "and says it in the sentence the site actually prints");
    }
    const v = rows.map((r) => (r as Record<string, unknown>)[side.field!] as number ?? 0);
    assert.deepEqual(v, [...v].sort((a, b) => b - a), "the list is in the order it says it is in");
    assert.ok(v[0] > 0, "and the figure it is ranked by is a figure, not a column of zeros");
  }
  assert.ok(c.ranking.runes.field !== "volumeAllTimeSats", "the runes have a real week, so nothing may put them on all time");
});

test("the runes' week and the collections' lifetime are never printed as one figure", () => {
  // The whole risk of two bases is a screen that merges them. `rankedFigure` reads the file's own record of which field
  // ranked which list, so a card can only ever print the figure its own list was ordered by. docs/phase9.md §11.3.
  const week = { window: "day7", unit: "sats",
    collections: { said: "lifetime volume ... NOT this week", field: "volumeAllTimeSats" as const, source: "Ordinals Wallet", window: null, want: 50, kept: 50 },
    runes: { said: "UniSat's volume over the seven days before this file was fetched", field: "volume7dSats" as const, source: "UniSat", window: "day7", want: 50, kept: 50 } };
  const col = { kind: "inscriptions", volumeAllTimeSats: 250_000_000, volume7dSats: null } as unknown as CatalogueEntry;
  const rune = { kind: "rune", volume7dSats: 97_665_811, volumeAllTimeSats: null } as unknown as CatalogueEntry;

  assert.equal(rankedFigure(col, week), "2.50 BTC traded in all, lifetime", "a lifetime figure says lifetime on the card");
  assert.match(rankedFigure(rune, week) ?? "", /traded that week/, "and a weekly one says the week");
  assert.equal(rankedFigure(col, undefined), null, "a file that records no basis prints no figure it cannot explain");
  assert.equal(rankedFigure({ kind: "rune", volume7dSats: 0 } as unknown as CatalogueEntry, week), null,
    "and an entry with nothing in the field its list was sorted by prints nothing, not a zero");
  // the field a list was NOT sorted by is never reached for, even when the entry carries it
  const bothFigures = { kind: "inscriptions", volumeAllTimeSats: 250_000_000, volume7dSats: 1_000_000 } as unknown as CatalogueEntry;
  assert.match(rankedFigure(bothFigures, week) ?? "", /lifetime/, "the collections' card prints the collections' basis");
});

test("the collections' week is summed from the fills, per collection, with the ids that changed hands", () => {
  const at = 1_800_000_000;
  const id = (n: number) => `${String(n).repeat(64).slice(0, 64)}i0`;
  const groups = groupFills([
    { collectionId: "a", priceSats: 400_000, at, inscriptionId: id(1), itemName: "One", contentType: "image/png" },
    { collectionId: "a", priceSats: 600_000, at, inscriptionId: id(2), itemName: "Two", contentType: "image/png" },
    { collectionId: "a", priceSats: 100_000, at, inscriptionId: id(2), itemName: "Two again", contentType: "image/png" },
    { collectionId: "b", priceSats: 900_000, at, inscriptionId: id(3), itemName: null, contentType: "text/plain" },
    { collectionId: "b", priceSats: 100_000, at, inscriptionId: "not-an-inscription-id", itemName: null, contentType: null },
  ]);
  const a = groups.find((g) => g.slug === "a")!, b = groups.find((g) => g.slug === "b")!;
  assert.equal(a.volume7dSats, 1_100_000, "the week is the sum of what was actually paid");
  assert.equal(a.trades7d, 3, "and it says how many fills it is made of");
  assert.deepEqual(a.items.map((i) => i.id), [id(1), id(2)], "the same inscription sold twice is one id, not two");
  assert.equal(b.volume7dSats, 1_000_000);
  assert.deepEqual(b.items, [], "a fill that is not an image, or not an inscription id, seeds no artwork");
  assert.equal(a.id, "unisat:a", "and the id is the marketplace's own");
});

test("a fill's timestamp is read in whichever unit it arrives in, and nothing else is read as a time", () => {
  assert.equal(stamp(1_800_000_000), 1_800_000_000, "seconds stay seconds");
  assert.equal(stamp(1_800_000_000_000), 1_800_000_000, "milliseconds become seconds");
  assert.equal(stamp(0), null);
  assert.equal(stamp(null), null);
  assert.equal(stamp("not a time"), null);
});

test("the ranking refuses an order it cannot stand behind: no figure at all, or a figure that is not in sats", () => {
  const row = (id: string, v: number | null) => ({ id, name: id, volume7dSats: v });
  const ranked = rankByWeek([row("a", 1_000_000), row("c", 9_000_000), row("b", 5_000_000)], 2, "collections");
  assert.deepEqual(ranked.map((r) => r.id), ["c", "b"], "biggest first, and only as many as were asked for");

  assert.throws(() => rankByWeek([row("a", 0), row("b", null)], 2, "collections"), /not one of which carried a positive/,
    "an arbitrary order printed as a ranking is the whole defect");
  assert.throws(() => rankByWeek([row("a", 3), row("b", 2)], 2, "runes"), /too small to be a figure in sats/,
    "a figure in BTC written into a field called ...Sats would be printed as sats on every screen");
  assert.deepEqual(rankByWeek([row("a", 5_000_000), row("a", 9_000_000)], 5, "collections").map((r) => r.volume7dSats),
    [5_000_000], "a row seen twice while paging is one row");
});

test("the fetch asks for the collections' week first, every run, and can only fall to lifetime by saying so", () => {
  const src = readFileSync(new URL("../scripts/catalogue.mjs", import.meta.url), "utf8");

  assert.match(src, /WANT_COLLECTIONS = Number\(process\.env\.CATALOGUE_COLLECTIONS \?\? 50\)/, "the top fifty collections");
  assert.match(src, /WANT_RUNES = Number\(process\.env\.CATALOGUE_RUNES \?\? 50\)/, "and the top fifty runes");
  assert.match(src, /WINDOW = "day7"/);
  assert.match(src, /RANK_FIELD = "volume7dSats"/);
  assert.match(src, /LIFETIME_FIELD = "volumeAllTimeSats"/, "the other basis has its own named field, never the week's");

  // the runes' window is stated by the marketplace and is sent flat, as their schema takes it
  assert.match(src, /timeType: WINDOW, start,/);
  // the collections' week is still SUMMED from the market's own Sold rows, and still asked for before anything else
  assert.match(src, /"collection\/auction\/actions"/);
  assert.match(src, /event: "Sold"/);
  assert.match(src, /WEEK_S = 7 \* 24 \* 60 \* 60/, "seven days, and the cutoff is computed from it rather than typed twice");
  assert.ok(!/unisat\("collection\/auction\/collection_statistic_list"/.test(src),
    "the endpoint that answers an empty list for every window is named in the comments and called nowhere");

  // docs/phase9.md §11.3: the week is TAKEN only when it fills the fifty, and the fall to lifetime is labelled
  assert.match(src, /ranked\.length >= WANT_COLLECTIONS/, "half a week beside half a lifetime is not a ranking");
  assert.match(src, /collectionBasis = lifetimeBasis\(fellBack\)/, "and the reason it fell is what the basis is built from");
  assert.match(src, /COLLECTIONS ARE ORDERED BY LIFETIME VOLUME, NOT BY THIS WEEK/,
    "the file's own notes say it in the words a reader would use");

  // every ranking goes through a guard: no rows, no positive figure, or a figure too small to be sats
  assert.ok(!/\.sort\(/.test(src.slice(src.indexOf("async function collectionsKeyless"), src.indexOf("// ---------------------------------------------------------------------- runes"))),
    "the keyless index no longer sorts itself: rankByLifetime sorts it, and refuses an order it cannot stand behind");
  for (const m of src.matchAll(/\.sort\(\(a, b\) => \(?b\[?(\w+)/g)) {
    assert.ok(m[1] === "field", `a ranking sorted by something other than the field it declares: ${m[1]}`);
  }
  assert.ok(!/volume7dSats: .*volumeAllTimeSats/.test(src), "a lifetime figure is never copied into the week's field");

  // and a keyless run refuses rather than writing a worse catalogue over a better one
  assert.match(src, /if \(!allowUnranked\) \{ console\.error\(REFUSAL\); process\.exit\(1\); \}/);
  assert.match(src, /catalogue\.unranked\.json/, "the lesser file is marked in its own name");
  assert.match(src, /img-unranked/, "and writes its pictures somewhere the committed ones are not");
});

test("the lifetime ranking is guarded exactly as the week's is: no rows, no figure, or a figure that is not in sats", () => {
  const row = (id: string, v: number | null) => ({ id, name: id, volumeAllTimeSats: v });
  const ranked = rankByLifetime([row("a", 1_000_000), row("c", 9_000_000), row("b", 5_000_000)], 2, "collections");
  assert.deepEqual(ranked.map((r) => r.id), ["c", "b"], "biggest first, and only as many as were asked for");

  assert.throws(() => rankByLifetime([], 50, "collections"), /not one of which carried a positive/,
    "no rows at all is still a refusal: an order over nothing is not a ranking");
  assert.throws(() => rankByLifetime([row("a", 0), row("b", null)], 2, "collections"), /not one of which carried a positive/,
    "an all-zero column is the same defect wearing a different field");
  assert.throws(() => rankByLifetime([row("a", 3), row("b", 2)], 2, "collections"), /too small to be a figure in sats/,
    "a figure in BTC written into a field called ...Sats would be printed as sats on every screen");
  assert.ok(!(ranked[0] as { volume7dSats?: number }).volume7dSats,
    "and ranking by lifetime never invents a seven-day figure for the row it ranked");
});

// ---- Magic Eden: asked for the collections' week, and answering nothing live (docs/phase9.md §11.2) -------------------

const seen = (over: Partial<MeResult> & { windowed: boolean; outcome: MeOutcome }): MeResult =>
  ({ name: "a route", url: "https://example.invalid/x", rows: 0, ...over });

test("a 200 Cloudflare invented from a stale copy is not an answer, whatever its body says", () => {
  // the whole finding of §11.2: /stat?collectionSymbol=bitcoin-frogs answers 200 with a plausible body, forever, from
  // one frozen edge copy of unknown age. Ranking a catalogue by it would be the §11 defect wearing a 200.
  assert.equal(isCacheGhost(200, "STALE"), true);
  assert.equal(isCacheGhost(200, "stale"), true, "the header's case is nobody's guarantee");
  assert.equal(isCacheGhost(200, " EXPIRED "), true);
  assert.equal(isCacheGhost(200, "HIT"), false, "a cache hit still came from something the origin served");
  assert.equal(isCacheGhost(200, "DYNAMIC"), false);
  assert.equal(isCacheGhost(200, "BYPASS"), false);
  assert.equal(isCacheGhost(200, null), false, "a header nobody sent is not evidence of staleness");
  assert.equal(isCacheGhost(503, "STALE"), false, "a 503 is refused for being a 503, not for being stale");
});

test("each answer is read for what it is: a body, a ghost, an outage, or a live origin declining", () => {
  assert.equal(classify(200, "MISS"), "live");
  assert.equal(classify(200, "STALE"), "ghost", "a ghost is never counted as live");
  assert.equal(classify(503, "BYPASS"), "busy", "no healthy upstream is an outage, never a missing route");
  assert.equal(classify(429, null), "busy", "and so is being asked to slow down");
  assert.equal(classify(0, null), "busy", "a host with no DNS record is written down, not thrown");
  assert.equal(classify(400, "DYNAMIC"), "refused", '"Only solana is supported" is a decision, not an outage');
  assert.equal(classify(404, null), "refused");
});

test("the verdict cannot be talked into yes by a ghost, and tells an outage apart from a refusal", () => {
  assert.equal(verdictOf([seen({ windowed: true, outcome: "live", rows: 5 })]), "yes");
  assert.equal(verdictOf([seen({ windowed: true, outcome: "live", rows: 0 })]), "not today",
    "a live body with no rows ranks nothing, so it is not a yes");
  assert.equal(verdictOf([seen({ windowed: true, outcome: "ghost", rows: 9 })]), "not today",
    "a cache ghost must never be able to say a week is obtainable");
  assert.equal(verdictOf([seen({ windowed: false, outcome: "live", rows: 9 })]), "not today",
    "a lifetime route answering says nothing about the week, which is the only question asked");
  assert.equal(verdictOf([seen({ windowed: true, outcome: "refused" })]), "no",
    "a live origin declining will not become data by waiting");
  assert.equal(verdictOf([seen({ windowed: true, outcome: "busy" })]), "not today");
  assert.equal(
    verdictOf([seen({ windowed: true, outcome: "busy" }), seen({ windowed: true, outcome: "live", rows: 3 })]),
    "yes", "one route answering the week is enough, whatever the others did");
});

test("a windowed body is counted whichever shape it arrives in, and an unparsed one counts as nothing", () => {
  assert.equal(rowsIn([1, 2, 3]), 3);
  assert.equal(rowsIn({ collections: [1, 2] }), 2);
  assert.equal(rowsIn({ runes: [1] }), 1);
  assert.equal(rowsIn({ total: 9, results: [] }), 0, "a total nobody returned rows for is not rows");
  assert.equal(rowsIn(null), 0);
  assert.equal(rowsIn("no healthy upstream"), 0);
});

test("the prober paces itself at its own rate, backs off growing, and only ever reads", () => {
  const src = readFileSync(new URL("../scripts/magiceden.mjs", import.meta.url), "utf8");
  const cat = readFileSync(new URL("../scripts/catalogue.mjs", import.meta.url), "utf8");

  // one marketplace's budget must never be spent at another's rate: their gateway sulks for minutes, UniSat's does not
  assert.ok(MAGICEDEN_GAP_MS >= 2_000, "Magic Eden's gap is its own, and far larger than UniSat's");
  assert.match(cat, /CATALOGUE_GAP_MS \?\? 260/, "and UniSat's 260 ms is untouched by any of this");
  assert.match(src, /MAGICEDEN_GAP_MS = Number\(process\.env\.MAGICEDEN_GAP_MS \?\? 2_000\)/,
    "the prober declares its own gap rather than borrowing one");
  assert.ok(!/process\.env\.CATALOGUE_GAP_MS/.test(src),
    "and never reads UniSat's, so slowing one marketplace down cannot speed the other up");

  assert.equal(backoffAfter(0), 20_000, "a 503 backs off from twenty seconds, not from the gap");
  assert.equal(backoffAfter(20_000), 40_000, "and doubles");
  assert.equal(backoffAfter(240_000), 300_000, "up to a ceiling, so an outage costs a bounded wait");
  assert.equal(backoffAfter(300_000), 300_000);

  // reads only: nothing here places, changes or cancels an order or a listing
  for (const route of ROUTES) {
    assert.ok(route.url.startsWith("https://"), `${route.name} is fetched over https`);
    assert.ok(!/psbt|sweep|swap|cancel|create|sell|buy/i.test(route.url),
      `${route.name} must be a collection or market read, never an order route`);
  }
  assert.ok(ROUTES.some((r) => r.windowed), "at least one route could actually answer the week");
  assert.ok(ROUTES.some((r) => r.windowed && /window=7d/.test(r.url)), "and it asks for seven days by name");
});

test("no Magic Eden figure has leaked into the catalogue, because none was ever read", () => {
  // §11.2: their whole Bitcoin API answered nothing live, so an adapter would have taken its field names from a schema
  // rather than a response. The catalogue must show no trace of one until the prober says yes.
  const cat = readFileSync(new URL("../scripts/catalogue.mjs", import.meta.url), "utf8");
  assert.ok(!/magiceden/i.test(cat), "the fetch names no marketplace it never got an answer from");

  const c = SHIPPED;
  assert.ok(!/magic\s*eden/i.test(JSON.stringify(c.attribution)),
    "and nothing is credited for data it did not supply");
  for (const col of c.collections) {
    assert.ok(!/magiceden/i.test(col.source + col.sourceUrl), `${col.name} came from the source it names`);
  }
});

// ---- the logo the chain now hands over --------------------------------------------------------------------------------

test("a coin's logo is fetchable, and a string that is not a picture never reaches an img", () => {
  assert.equal(logoSrc("ipfs://bafkreicuspukrnaisnzred73svouxh3favyf4frtdbjlha36juefmx2ygq"),
    "https://ipfs.io/ipfs/bafkreicuspukrnaisnzred73svouxh3favyf4frtdbjlha36juefmx2ygq",
    "the indexer names no gateway, so the site picks one");
  assert.equal(logoSrc("ipfs://ipfs/bafyabc"), "https://ipfs.io/ipfs/bafyabc", "a doubled ipfs prefix is not a second directory");
  assert.equal(logoSrc("https://example.com/a.png"), "https://example.com/a.png");
  assert.ok(logoSrc("data:image/png;base64,iVBORw0KGgo=")?.startsWith("data:image/"));
  assert.equal(logoSrc(null), null);
  assert.equal(logoSrc(""), null);
  assert.equal(logoSrc("   "), null);
  assert.equal(logoSrc("ipfs://"), null);
  assert.equal(logoSrc("http://example.com/a.png"), null, "plain http is refused rather than mixed into a secure page");
  assert.equal(logoSrc("javascript:alert(1)"), null);
  assert.equal(logoSrc("<img onerror=alert(1)>"), null);
  assert.equal(logoSrc("data:text/html,<script>"), null, "a data URL that is not an image is not a logo");
  assert.equal(logoSrc("https://example.com/" + "a".repeat(1100)), null, "a stranger's string has a length");
});

// ---- docs/phase9.md §11.3: a vault is a fact on a card, not a section the card lives in -------------------------------

const catalogueOf = (collections: unknown[], runes: unknown[]) =>
  ({ fetchedAt: "2026-09-21T20:03:31.389Z", collections, runes } as unknown as Catalogue);
const col = (slug: string, name = slug) => ({ id: `ow:${slug}`, slug, name, kind: "inscriptions" });
const rune = (runeId: string, name: string) => ({ id: `rune:${runeId}`, slug: name.toLowerCase(), name, kind: "rune", runeId });
const vault = (id: number, ref: string, name = ref, over: Record<string, unknown> = {}) =>
  ({ id, ref, name, kind: ref.includes(":") ? "rune" : "inscriptions", paused: false, ...over }) as ChoosableVault;

test("the catalogue and the vaults are one list, joined on the registry's own ref, in the catalogue's ranked order", () => {
  const cat = catalogueOf([col("bitcoin-frogs"), col("runestone")], [rune("840000:3", "DOG•GO•TO•THE•MOON")]);
  const rows = choices(cat, [vault(7, "840000:3", "DOG•GO•TO•THE•MOON"), vault(2, "bitcoin-frogs"), vault(0, "demo-stones")]);

  assert.deepEqual(rows.map((r) => r.ref), ["bitcoin-frogs", "runestone", "840000:3", "demo-stones"],
    "the catalogue's own order first, then whatever the registry holds that the catalogue does not name");
  assert.equal(rows[0].vault?.id, 2, "a collection joins on its slug");
  assert.equal(rows[2].vault?.id, 7, "and a rune on its rune id, which is the ref a rune is listed under");
  assert.equal(rows[1].vault, undefined, "an entry with no vault is in the SAME list, not fenced off into a bucket");
  assert.equal(rows[3].entry, undefined, "and a vault the catalogue does not name is still choosable");
  assert.equal(choices(undefined, [vault(0, "demo-stones")]).length, 1, "no catalogue is not no vaults");
  assert.equal(choices(cat, []).filter((r) => r.vault).length, 0, "and no vaults is not no catalogue");
});

test("a card says which of the three it is, because 'paused' and 'not created' are different facts", () => {
  assert.equal(vaultTag(undefined), "no vault yet");
  assert.equal(vaultTag({ id: 3, ref: "bitcoin-frogs", paused: false }), "vault");
  assert.equal(vaultTag({ id: 3, ref: "bitcoin-frogs", paused: true }), "paused",
    "a paused vault exists and cannot be chosen; an entry with none does not exist at all");

  const by = vaultsByRef([{ id: 2, ref: "Bitcoin-Frogs", paused: false }, { id: 7, ref: "840000:3", paused: true }]);
  const frogs = col("bitcoin-frogs") as unknown as CatalogueEntry;
  const dog = rune("840000:3", "DOG•GO•TO•THE•MOON") as unknown as CatalogueEntry;
  assert.equal(vaultOf(frogs, by)?.id, 2, "the ref is matched whatever case the registry stored it in");
  assert.equal(vaultOf(dog, by)?.id, 7);
  assert.equal(vaultOf(col("nodemonkes") as unknown as CatalogueEntry, by), undefined);
  assert.equal(vaultTag(vaultOf(dog, by)), "paused");
});

test("the chooser's search is over the one list, and finds a rune by its ticker or its id", () => {
  const rows = choices(catalogueOf([col("bitcoin-frogs", "Bitcoin Frogs")], [rune("840000:3", "DOG•GO•TO•THE•MOON")]),
    [vault(2, "bitcoin-frogs", "Bitcoin Frogs")]);
  assert.equal(matchChoices("", rows).length, 2, "an empty query is not a filter");
  assert.deepEqual(matchChoices("frogs", rows).map((r) => r.ref), ["bitcoin-frogs"]);
  assert.deepEqual(matchChoices("doggo", rows).map((r) => r.ref), ["840000:3"], "the bullets are not something to type");
  assert.deepEqual(matchChoices("840000", rows).map((r) => r.ref), ["840000:3"]);
  assert.equal(matchChoices("nothing here", rows).length, 0);
});

// ---- vaults: the word the reader sees, against the word the contracts use ---------------------------------------------

test("#/vaults is what people see, #/listings still answers, and neither can 404", () => {
  assert.ok(KNOWN.has("vaults"), "the new name is a page");
  assert.ok(KNOWN.has("listings"), "the old name must keep answering: the deployed site and the docs link to it");
  assert.equal(CANONICAL.listings, "vaults", "and it settles on the new one");
  assert.equal(CANONICAL.vaults, undefined, "the canonical name does not redirect to itself");
  assert.equal(UNDER.listing, "vaults", "a single vault lights the Vaults nav item");
  assert.equal(UNDER.listings, "vaults");
  assert.ok(NAV.some(([to, label]) => to === "vaults" && label === "Vaults"), "and it is in the nav, under the word the user chose");
});

// ---- the market: candles, and what a chart must never invent ----------------------------------------------------------

import { bounds, cap, capOf, change, clock, coins as coinAmount, graduationAt, hasUsd, initialRange, pct as pctOf, price as priceOf, RANGES as CHART_RANGES, wholeSupply, type Candle } from "../src/lib/chart.js";
import { KINDS, mergeActivity, ofKind } from "../src/lib/activity.js";
import { matchCoins } from "../src/lib/coinlist.js";

const k = (t: number, o: number, h: number, l: number, c: number, v = 1, n = 1): Candle => ({ t, o, h, l, c, v, n });

test("a coin opens on the range its own age can fill, and no range is per-trade", () => {
  const now = 2_000_000;
  assert.equal(initialRange(now - 600, now), "1h", "ten minutes old: a day would be one bar of nothing");
  assert.equal(initialRange(now - 10_000, now), "6h");
  assert.equal(initialRange(now - 50_000, now), "1d");
  assert.equal(initialRange(now - 400_000, now), "all");
  assert.equal(initialRange(null, now), "1d", "an unknown age does not crash the chart");
  // docs/phase12.md §1: the per-trade view is gone from the page, which is what removes the +82.3% between two trades
  assert.deepEqual(CHART_RANGES.map((r) => r.key), ["5m", "1h", "6h", "1d", "all"]);
  assert.ok(!CHART_RANGES.some((r) => String(r.key) === "trade"), "there is no chip a reader can press to get one candle per trade");
});

test("the price window fits the candles, with air, and never drops below zero", () => {
  const b = bounds([k(1, 10, 12, 8, 11), k(2, 11, 20, 9, 19)]);
  assert.ok(b && b.lo >= 0 && b.lo < 8 && b.hi > 20, "the extremes are inside, with room");
  assert.equal(b!.t0, 1);
  assert.equal(b!.t1, 2);
  const flat = bounds([k(1, 5, 5, 5, 5)]);
  assert.ok(flat && flat.hi > flat.lo, "a single flat candle still has a window to be drawn in");
  assert.equal(bounds([]), null, "no candles is not a window");
});

test("change is over the whole range shown, is not computed from one candle, and a zero open does not produce infinity", () => {
  assert.equal(change([k(1, 10, 10, 10, 10)]), null);
  assert.equal(change([]), null);
  assert.equal(change([k(1, 0, 1, 0, 1)]), null);
  assert.equal(change([k(1, 10, 10, 10, 10), k(2, 10, 12, 10, 12)]), 0.2);
  assert.equal(pctOf(0.2), "+20.0%");
  assert.equal(pctOf(-0.005), "-0.50%");
  assert.equal(pctOf(null), "not known");
  // docs/phase12.md §1: the figure is the first bucket's OPEN against the last bucket's CLOSE, over however long the range
  // is — never the step from one trade to the next, which is where a routine +82.3% came from. Six buckets of a bonding
  // curve climbing hard say what the window did, not what the last buy cost.
  const climbing = [k(0, 100, 110, 100, 110), k(300, 110, 140, 108, 138), k(600, 138, 182, 138, 182)];
  assert.equal(change(climbing), 0.82, "the whole window, not the last step");
  assert.equal(change([climbing[1], climbing[2]]), (182 - 110) / 110, "and a narrower window is a narrower claim, not a different kind of number");
});

// ---- docs/phase12.md §3: market cap, and every way it is allowed to be unknown ----------------------------------------

const withRate = { supplyWei: String(10n ** 27n), usdPerEth: 2_785.14 };

test("market cap is supply times price times the rate, and the doc's worked example comes out", () => {
  assert.equal(wholeSupply(String(10n ** 27n)), 1e9, "1e27 base units at 18 decimals is a billion whole coins");
  assert.equal(wholeSupply(null), null);
  assert.equal(wholeSupply("0"), null, "a supply of nothing is not a supply");
  // docs/phase12.md §3: CHAL2SIM at 3.133e-9 is an $8.7k coin, and always was
  assert.equal(Math.trunc(capOf(withRate, 3.133e-9, "usd")!), 8_725);
  assert.equal(capOf(withRate, 3.133e-9, "pair"), 1e9 * 3.133e-9, "the same cap in the pair asset needs no rate at all");
});

test("a missing supply, a missing rate or a missing price is null, and the dollar option is then not offered", () => {
  assert.equal(capOf({ supplyWei: null, usdPerEth: 2_785.14 }, 3e-9, "usd"), null, "no supply, no market cap: it is not guessed at a round billion");
  assert.equal(capOf({ supplyWei: String(10n ** 27n), usdPerEth: null }, 3e-9, "usd"), null, "no rate, no dollars");
  assert.equal(capOf({ supplyWei: String(10n ** 27n), usdPerEth: null }, 3e-9, "pair"), 1e9 * 3e-9, "but the pair asset still works, which is the honest fallback");
  assert.equal(capOf(withRate, null, "usd"), null);
  assert.equal(capOf(withRate, 0, "usd"), null, "a coin that has never traded has no price and so no cap");
  assert.equal(hasUsd(withRate), true);
  assert.equal(hasUsd({ supplyWei: String(10n ** 27n), usdPerEth: null }), false, "a chain with no readable pool never shows a USD chip");
  assert.equal(hasUsd({ supplyWei: null, usdPerEth: 2_785.14 }), false);
  assert.equal(hasUsd(undefined), false);
  // a recorded demo taken before this phase has neither field; a missing number must read as an unknown one, not as NaN
  const older = {} as unknown as { supplyWei: string | null; usdPerEth: number | null };
  assert.equal(hasUsd(older), false);
  assert.equal(capOf(older, 3e-9, "usd"), null);
  assert.equal(capOf({ supplyWei: String(10n ** 27n), usdPerEth: undefined as unknown as number }, 3e-9, "usd"), null);
});

test("a market cap is written the way one is read, and an unknown one says so in words", () => {
  assert.equal(cap(8_725.84, "usd"), "$8,726");
  assert.equal(cap(3_260_000, "usd"), "$3.26M");
  assert.equal(cap(1_240_000_000, "usd"), "$1.24B");
  assert.equal(cap(12.5, "usd"), "$12.50");
  assert.equal(cap(3.133, "pair", "ETH"), "3.13 ETH");
  assert.equal(cap(2_450_000, "pair", "ETH"), "2.45M ETH");
  assert.equal(cap(null, "usd"), "not known");
  assert.equal(cap(Number.NaN, "usd"), "not known");
});

test("the crosshair's clock shows the second only when the bucket is small enough for it to mean anything", () => {
  const t = Date.UTC(2026, 8, 21, 14, 3, 7) / 1000;
  assert.match(clock(t, 5), /14:03:07 UTC$/, "five-second buckets: the second is the point");
  assert.match(clock(t, 60), /14:03 UTC$/);
  assert.ok(clock(t, 900).includes("21 Sep"), "at fifteen minutes and wider the day matters more than the second");
});

test("a curve price is a very small number and is printed as one, not as a row of zeros", () => {
  assert.equal(priceOf(6.349706642413743e-9), "6.350e-9");
  assert.equal(priceOf(1234.5), "1,234.5");
  assert.equal(priceOf(0.5), "0.5000");
  assert.equal(priceOf(0), "not known");
  assert.equal(priceOf(null), "not known");
  assert.equal(priceOf(Number.NaN), "not known");
});

test("graduation is placed by the indexer's own timestamp, and only when it falls inside the window drawn", () => {
  const candles = [k(100, 1, 1, 1, 1), k(200, 1, 1, 1, 1)];
  assert.equal(graduationAt({ graduation: { at: 150, block: 1, tx: "0x" }, candles }), 150);
  assert.equal(graduationAt({ graduation: { at: 50, block: 1, tx: "0x" }, candles }), null, "before the window: not drawn at its edge");
  assert.equal(graduationAt({ graduation: null, candles }), null, "still on the curve");
  assert.equal(graduationAt({ graduation: { at: 150, block: 1, tx: "0x" }, candles: [] }), null);
});

test("a size in raw units is said in whole coins", () => {
  assert.equal(coinAmount("20745197746075189049944116"), "20.75M");
  assert.equal(coinAmount("1500000000000000000000"), "1.50k");
  assert.equal(coinAmount("2500000000000000000"), "2.50");
});

// ---- the site-wide feed ------------------------------------------------------------------------------------------------

const row = (name: string, over: Record<string, unknown> = {}) => ({
  address: `0x${name.toLowerCase().padEnd(40, "0")}` as `0x${string}`, name, symbol: name.slice(0, 3).toUpperCase(),
  listingId: 0, listingName: "Demo", creator: "0x0" as `0x${string}`, router: "0x0" as `0x${string}`,
  launchedAt: 100, lifetimeFeesWei: 0n, fees24hWei: 0n, harvests: 0, toVaultWei: 0n, creatorTaxBps: 200, ...over,
});
const acq = (id: number, settledAt: number | null, delivered = false) => ({
  listingId: 0, acquisitionId: id, purchaseId: id, kind: "inscriptions" as const, coin: "0x0" as `0x${string}`,
  coinSymbol: "AAA", assetRef: "abc", amount: "1", btcTxid: "f".repeat(64), satsPaid: "100", settledAt,
  tx: "0x0" as `0x${string}`, purchaseSpentWei: "0", purchaseAcquisitions: 1,
  delivery: { mode: "hold" as const, state: "held" as const, winner: null, destination: null, deliveredTxid: delivered ? "e".repeat(64) : null, overdue: false, secondsToDeliver: null },
  verify: {
    links: [],
    check: { kind: "inscription" as const, txid: "f".repeat(64), inscriptionId: "abc", address: null, expect: "the inscription's current owner is `address`, until it is delivered" as const },
    note: "",
  },
});

test("everything that happened, in one list, newest first, and nothing without a time", () => {
  const events = mergeActivity(
    [row("Alpha", { launchedAt: 300, harvests: 2, lastHarvestAt: 500 }), row("Beta", { launchedAt: null })],
    [acq(1, 400), acq(2, null), acq(3, 600, true)],
    [{ id: "d1", listingId: 0, acquisitionId: 1, type: "draw", state: "draw-open", mode: "draw-weighted", assetRef: "abc", openedAt: 450, challengeEnds: null, challengeEndsIn: null, root: null, uri: null, total: null }],
  );
  const at = events.map((e) => e.at);
  assert.deepEqual(at, [...at].sort((a, b) => b - a), "newest first");
  assert.ok(!events.some((e) => e.kind === "launch" && e.coin.name === "Beta"), "a launch with no time is left out, not floated to the top");
  assert.ok(!events.some((e) => e.kind === "purchase" && e.a.acquisitionId === 2), "and so is a purchase with no time");
  assert.equal(events.filter((e) => e.kind === "harvest").length, 0, "a harvest needs its time passed in");

  const withHarvest = mergeActivity([row("Alpha", { launchedAt: 300, harvests: 2, lastHarvestAt: 500 })], [], [], (c) => c.lastHarvestAt ?? null);
  assert.equal(withHarvest.filter((e) => e.kind === "harvest").length, 1);
  assert.equal(withHarvest[0].kind, "harvest", "the sweep is newer than the launch");

  assert.equal(ofKind(events, "draw").length, 1);
  assert.equal(ofKind(events, "all").length, events.length);
  assert.ok(KINDS.some((x) => x.key === "all"), "the feed can always be shown whole");
});

test("the coin search matches a name, a ticker or an address, and an empty query is not a filter", () => {
  const rows = [row("Gneiss"), row("Quartz")];
  assert.equal(matchCoins(rows, "gne").length, 1);
  assert.equal(matchCoins(rows, "QUA").length, 1, "a ticker is matched whatever the case");
  assert.equal(matchCoins(rows, rows[0].address.slice(0, 8)).length, 1);
  assert.equal(matchCoins(rows, "   ").length, 2, "whitespace is not a search");
  assert.equal(matchCoins(rows, "zzz").length, 0);
});

test("a collection whose only picture is an icon inscription given as an object is kept, and its picture is that inscription", () => {
  // Taproot Wizards as the index serves it, 2026-09-23: no icon URL, no gallery inscription, the icon inscription as an object
  const row = { name: "Taproot Wizards", slug: "taproot-wizards", active: true, total_supply: 2106, floor_price: null,
    global_volume_all_time: 27178337358, icon: "", gallery_inscription_id: null,
    icon_inscription: { id: "0301e0480b374b32851a9462db29dc19fe830a7f7d7a88b81612b9d42099c0aei0", content_type: "image/jpeg" } };
  const [c] = shapeCollections([row]);
  assert.ok(c, "kept: a 27bn-sat collection is not dropped for how its picture is described");
  assert.equal(c.sampleInscription, "0301e0480b374b32851a9462db29dc19fe830a7f7d7a88b81612b9d42099c0aei0");
  assert.equal(c.iconUrl, null, "an empty icon string is no icon");
  // the id-string form still works, and a row with no picture of any kind is still dropped
  assert.equal(shapeCollections([{ ...row, icon_inscription: "abc123i0" }])[0].sampleInscription, "abc123i0");
  assert.deepEqual(shapeCollections([{ ...row, icon_inscription: null }]), []);
});
