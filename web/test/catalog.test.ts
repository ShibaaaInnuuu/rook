import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { byVolume24h, COIN_SORTS, filterCoins, matchVaults, search, sharedSupply, sortCoins, trendShortlist, vaultPicks, volume24hOf, wholeCoins, type CoinRow } from "../src/lib/coinlist.js";
import type * as Api from "../src/api/types.js";
import { KNOWN, LEGAL, NAV, PAGES, UNDER, sectionOf } from "../src/lib/routes.js";

const coin = (over: Partial<CoinRow> & { name: string }): CoinRow => ({
  address: `0x${over.name.toLowerCase().padEnd(40, "0")}`, symbol: over.name.slice(0, 4).toUpperCase(), listingId: 0,
  listingName: "Demo Stones", creator: "0x", router: "0x", launchedAt: 1_000, lifetimeFeesWei: 0n, fees24hWei: 0n,
  harvests: 0, toVaultWei: 0n, creatorTaxBps: 200, ...over,
});

test("sorting coins: every sort the chips offer, and what happens to a figure the source could not give", () => {
  const rows = [
    coin({ name: "Beta", launchedAt: 300, lifetimeFeesWei: 5n, fees24hWei: 1n }),
    coin({ name: "Alpha", launchedAt: 100, lifetimeFeesWei: 9n, fees24hWei: null }),
    coin({ name: "Gamma", launchedAt: 200, lifetimeFeesWei: null, fees24hWei: 4n }),
  ];
  const names = (by: Parameters<typeof sortCoins>[1]) => sortCoins(rows, by).map((c) => c.name);

  assert.deepEqual(names("newest"), ["Beta", "Gamma", "Alpha"]);
  assert.deepEqual(names("fees"), ["Alpha", "Beta", "Gamma"], "a coin whose fees are not known sorts last, not as zero");
  assert.deepEqual(names("fees24h"), ["Gamma", "Beta", "Alpha"], "unknown 24h fees sort last even when lifetime fees are high");
  assert.deepEqual(names("name"), ["Alpha", "Beta", "Gamma"]);
  assert.deepEqual(rows.map((c) => c.name), ["Beta", "Alpha", "Gamma"], "sorting does not reorder the caller's array");
  const swept = [coin({ name: "Once", harvests: 1 }), coin({ name: "Often", harvests: 9 }), coin({ name: "Never", harvests: null })];
  assert.deepEqual(sortCoins(swept, "harvests").map((c) => c.name), ["Often", "Once", "Never"],
    "most harvested first, and a coin whose harvests are not known sorts last rather than as none");
  assert.deepEqual(COIN_SORTS.map(([k]) => k).sort(), ["cap", "fees", "fees24h", "harvests", "name", "newest"], "every chip has a sort behind it");
});

test("sorting is stable enough to be read: ties fall back to the name", () => {
  const rows = [coin({ name: "Zed", launchedAt: 5 }), coin({ name: "Ada", launchedAt: 5 })];
  assert.deepEqual(sortCoins(rows, "newest").map((c) => c.name), ["Ada", "Zed"]);
  assert.deepEqual(sortCoins(rows, "fees").map((c) => c.name), ["Ada", "Zed"]);
});

test("filtering coins: by listing, and by whether any fee has actually reached a vault", () => {
  const rows = [
    coin({ name: "One", listingId: 0, lifetimeFeesWei: 0n }),
    coin({ name: "Two", listingId: 1, lifetimeFeesWei: 7n }),
    coin({ name: "Three", listingId: 1, lifetimeFeesWei: null }),
  ];
  assert.deepEqual(filterCoins(rows, null, false).map((c) => c.name), ["One", "Two", "Three"]);
  assert.deepEqual(filterCoins(rows, 1, false).map((c) => c.name), ["Two", "Three"]);
  assert.deepEqual(filterCoins(rows, null, true).map((c) => c.name), ["Two"], "unknown fees are not counted as earning");
  assert.deepEqual(filterCoins(rows, 0, true), [], "the two filters are an and, not an or");
  assert.deepEqual(filterCoins(rows, 9, false), [], "a listing nothing is paired with gives an empty list, not everything");
});

test("search: what it can answer without an endpoint, and what it refuses to guess at", () => {
  const coins = [coin({ name: "Local Stone", symbol: "STONE", address: "0xabc123def4567890abc123def4567890abc12345" })];
  const listings = [{ id: 0, name: "Demo Stones", kind: "inscriptions", ref: "stones-collection", paused: false } as never];
  const receipts = [{ listingId: 0, acquisitionId: 2, btcTxid: "42b21327f2d0aa11", assetRef: "0000i0" } as never];

  assert.deepEqual(search("s", coins, listings, receipts), [], "one character is not a search");
  assert.equal(search("stone", coins, listings, receipts).length, 2, "the coin and the listing both match");
  assert.equal(search("0xABC123", coins, listings, receipts)[0]?.to, "coin/0xabc123def4567890abc123def4567890abc12345", "an address matches whatever its case");
  assert.equal(search("42b21327", coins, listings, receipts)[0]?.kind, "receipt", "a Bitcoin transaction id finds its purchase");
  assert.deepEqual(search("nothing here", coins, listings, receipts), []);
  const many = Array.from({ length: 20 }, (_, i) => coin({ name: `Stone ${i}` }));
  assert.equal(search("stone", many, [], []).length, 8, "the list is capped so the box cannot cover the page");
});

test("routes: the nav, the known pages and the page dispatch all name the same routes", () => {
  assert.equal(new Set(PAGES).size, PAGES.length, "no route is listed twice");
  for (const [to] of NAV) assert.ok(KNOWN.has(to), `the nav offers ${to}, so it must be a known page`);
  for (const [at, section] of Object.entries(UNDER)) {
    assert.ok(KNOWN.has(at) && KNOWN.has(section), `${at} sits under ${section}, so both must exist`);
    assert.ok(NAV.some(([to]) => to === section) || section === "listings", "a section must be somewhere a reader can get back to");
  }
  for (const l of LEGAL) assert.ok(KNOWN.has(l), `${l} must be a known page`);
  assert.equal(sectionOf("coin"), "explore", "a coin's page keeps Explore lit");
  assert.equal(sectionOf("receipts"), "receipts");
  assert.equal(sectionOf("nonsense"), "nonsense", "an unknown route is its own section, and the shell shows Lost");
});

test("routes: every page that is not the landing page or a legal page is dispatched by AppPages", () => {
  const src = readFileSync(new URL("../src/AppPages.tsx", import.meta.url), "utf8");
  const dispatched = new Set([...src.matchAll(/at === "([a-z]+)"/g)].map((m) => m[1]));
  for (const p of PAGES) {
    if (p === "" || LEGAL.has(p)) continue;
    // trust is the fall-through, so it is handled without being named
    if (p === "trust") { assert.ok(src.includes("<Trust />"), "trust is the fall-through"); continue; }
    assert.ok(dispatched.has(p), `#/${p} is a known route, so AppPages must have a branch for it`);
  }
  for (const d of dispatched) assert.ok(KNOWN.has(d), `AppPages dispatches ${d}, so it must be a known page`);
});

// ---- docs/phase14.md: the vault control, the trending shortlist, and the one sort for a supply that never varies -------

const vault = (over: Partial<Api.ListingSummary> & { id: number }): Api.ListingSummary => ({
  kind: "inscriptions", name: `Vault ${over.id}`, ref: `vault-${over.id}`, divisibility: null, vault: "0x", btcVault: null, membershipRoot: null,
  paused: false, coins: 0, depositedWei: "0", acquisitions: 0, capWei: null, capExpiry: null, capExpired: null, openDeclarations: 0, ...over,
} as Api.ListingSummary);

test("the vault control offers the vaults coins actually pay into, and never more chips than it promised", () => {
  const vaults = [
    vault({ id: 1, coins: 0, depositedWei: "900" }),
    vault({ id: 2, coins: 3, depositedWei: "10" }),
    vault({ id: 3, coins: 3, depositedWei: "20" }),
    vault({ id: 4, coins: 1, depositedWei: "5" }),
  ];
  const picked = vaultPicks(vaults, null, 2);
  assert.deepEqual(picked.shown.map((v) => v.id), [3, 2], "a coin paying in outranks a full vault nothing is paired with, and ETH breaks the tie");
  assert.deepEqual(picked.rest.map((v) => v.id), [4, 1], "and the rest keep the same ranking behind the panel");

  // sixty vaults is what staging carries, and the row must not grow with them
  const many = Array.from({ length: 60 }, (_, i) => vault({ id: i + 1, coins: i % 3 === 0 ? 2 : 0 }));
  const wide = vaultPicks(many, null, 8);
  assert.equal(wide.shown.length, 8, "eight, whatever the vault count");
  assert.equal(wide.rest.length, 52, "and every one of the others is still reachable");
  assert.ok(wide.shown.every((v) => v.coins > 0), "the eight are the ones a filter can usefully filter to");

  // a vault chosen out of the panel is put on the row, so a filtered grid never hides what filtered it
  const deep = vaultPicks(many, 59, 8);
  assert.equal(deep.shown.length, 9);
  assert.equal(deep.shown[8].id, 59, "the chosen vault is on the row even when it is not one of the eight");
  assert.ok(!deep.rest.some((v) => v.id === 59), "and it is not offered twice");
  assert.equal(vaultPicks([], null, 8).shown.length, 0, "no vaults, no chips");
});

test("the vault panel's search: a name, a slug, a rune id, and an empty query is every vault", () => {
  const vaults = [
    vault({ id: 1, name: "Bitcoin Frogs", ref: "bitcoin-frogs" }),
    vault({ id: 2, name: "DOG•GO•TO•THE•MOON", ref: "840000:3", kind: "rune" }),
  ];
  assert.equal(matchVaults("", vaults).length, 2, "an empty query is not a filter");
  assert.deepEqual(matchVaults("frog", vaults).map((v) => v.id), [1]);
  assert.deepEqual(matchVaults("bitcoin-fro", vaults).map((v) => v.id), [1], "the slug matches too");
  assert.deepEqual(matchVaults("840000:3", vaults).map((v) => v.id), [2], "so does a rune id");
  assert.deepEqual(matchVaults("doggo", vaults).map((v) => v.id), [2], "a rune's bulleted name matches without its bullets");
  assert.deepEqual(matchVaults("nothing", vaults), []);
});

test("sorting by market cap: the caps come from outside, and a coin without one is last rather than zero", () => {
  const rows = [coin({ name: "Small" }), coin({ name: "Big" }), coin({ name: "Unread" })];
  const caps = new Map<string, number | null>([
    [rows[0].address.toLowerCase(), 5],
    [rows[1].address.toLowerCase(), 50],
    [rows[2].address.toLowerCase(), null],
  ]);
  assert.deepEqual(sortCoins(rows, "cap", caps).map((c) => c.name), ["Big", "Small", "Unread"]);
  assert.deepEqual(sortCoins(rows, "cap", new Map()).map((c) => c.name), ["Big", "Small", "Unread"],
    "with no caps at all nothing is known, so the order is the name and not a fabricated ranking");
  assert.ok(COIN_SORTS.some(([k]) => k === "cap"), "the chip exists");
  assert.equal(COIN_SORTS.filter(([, label]) => /cap|dilut|price/i.test(label)).length, 1,
    "one pill, not three: market cap, fully diluted value and price order the list identically");
});

test("the supply every coin shares is checked against the rows, not asserted", () => {
  const same = [coin({ name: "A", supplyWei: "1000000000000000000000000000" }), coin({ name: "B", supplyWei: "1000000000000000000000000000" })];
  assert.equal(sharedSupply(same), "1000000000000000000000000000");
  assert.equal(wholeCoins(sharedSupply(same)), 1e9, "a billion whole coins, which is what Pons mints");
  const mixed = [...same, coin({ name: "C", supplyWei: "5" })];
  assert.equal(sharedSupply(mixed), null, "one coin with a different supply and the orderings come apart");
  assert.equal(sharedSupply([coin({ name: "D", supplyWei: null })]), null, "an unread supply is not a shared one");
  assert.equal(wholeCoins(null), null);
  assert.equal(sharedSupply([]), null);
});

test("trending: the shortlist names the key that drew it, and the order is the number that is printed", () => {
  const rows = [
    coin({ name: "Quiet", volume24hWei: 0n, fees24hWei: 9n }),
    coin({ name: "Busy", volume24hWei: 100n, fees24hWei: 0n }),
    coin({ name: "Middling", volume24hWei: 50n, fees24hWei: 1n }),
  ];
  const served = trendShortlist(rows);
  assert.equal(served.key, "volume24h", "where the indexer reports volume, volume is the key");
  assert.deepEqual(served.rows.map((c) => c.name), ["Busy", "Middling", "Quiet"]);
  assert.deepEqual(byVolume24h(served.rows).map((c) => c.name), ["Busy", "Middling", "Quiet"], "and the printed order is the same order");
  assert.equal(volume24hOf(rows[1]), 100 / 1e18, "the figure printed is the field, in whole pair-asset units");

  // an indexer older than this phase reports none, so the fallback is named and the volumes come from the candles
  const older = rows.map((c) => ({ ...c, volume24hWei: null }));
  const fell = trendShortlist(older);
  assert.equal(fell.key, "fees24h", "the key is the best the list response carries, and the page says which");
  assert.deepEqual(fell.rows.map((c) => c.name), ["Quiet", "Middling", "Busy"], "fees are not volume, which is why the shortlist is only a shortlist");
  const measured = new Map<string, number | null>(older.map((c, i) => [c.address.toLowerCase(), [0, 100, 50][i]]));
  assert.deepEqual(byVolume24h(fell.rows, measured).map((c) => c.name), ["Busy", "Middling", "Quiet"],
    "and once measured, the strip is ordered by the very number it prints");
  assert.equal(volume24hOf(older[0], measured), 0, "a day with no trade is zero, and prints as zero");
  assert.equal(volume24hOf(older[0], new Map()), null, "a coin nothing could be measured for is not known, which is not zero");

  const fresh = older.map((c) => ({ ...c, fees24hWei: 0n, lifetimeFeesWei: 0n }));
  assert.equal(trendShortlist(fresh).key, "newest", "a chain where nothing has been harvested falls all the way back, and says so");
  assert.equal(trendShortlist(rows, 2).rows.length, 2, "the shortlist is bounded: it costs one request per coin on it");
});
