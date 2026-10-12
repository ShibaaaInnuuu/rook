import assert from "node:assert/strict";
import { existsSync, readdirSync, readFileSync } from "node:fs";
import { test } from "node:test";
import { catalogFixtures } from "../src/rook/catalogFixtures.js";
import { CHAIN_TABS, countByChain, entryOfNest, floorNote, floorText, launchable, liveCount, searchEntries, sortEntries, tabEntries, usdText, volText, wizardOrder } from "../src/rook/cataloglogic.js";
import type { Catalog, CatalogEntry, CatalogOne } from "../src/rook/catalogTypes.js";

/** docs/v6/collections-catalog.md §4: the catalogue's tabs, sorts, search and words over the fixture, and the copies that must not drift. */
const here = (p: string) => new URL(p, import.meta.url);
const read = async (f: string) => JSON.parse(readFileSync(here(`../public/fixtures/catalog/${f}`), "utf8"));
const src = await catalogFixtures(read);

test("the catalogue's types and fixtures are byte copies of the indexer's (checked whenever the repository copy is beside us)", () => {
  if (!existsSync(here("../../indexer/src/rook/catalogTypes.ts"))) return;
  assert.equal(readFileSync(here("../src/rook/catalogTypes.ts"), "utf8"), readFileSync(here("../../indexer/src/rook/catalogTypes.ts"), "utf8"));
  const theirs = readdirSync(here("../../indexer/fixtures/catalog/")).sort();
  assert.deepEqual(readdirSync(here("../public/fixtures/catalog/")).sort(), theirs);
  for (const f of theirs) assert.ok(readFileSync(here(`../public/fixtures/catalog/${f}`)).equals(readFileSync(here(`../../indexer/fixtures/catalog/${f}`))), f);
});

test("the fixture catalogue is labelled, holds every chain, has two live nests, dated stale floors, and pictures that are the public URLs the seed recorded", async () => {
  assert.equal(src.kind, "fixture"); assert.match(src.note, /^FIXTURE/);
  const c = await src.get<Catalog>("/v1/rook/catalog");
  assert.equal(c.source.kind, "fixture"); assert.equal(c.stale, true); assert.equal(c.refreshedAt, null); assert.equal(c.rule.stage, 0);
  assert.ok(c.entries.length >= 100);
  const counts = countByChain(c.entries);
  assert.equal(counts.bitcoin, 50); assert.equal(counts.robinhood, 12); assert.ok(counts.ethereum >= 30 && counts.base >= 4 && counts.other >= 15); assert.equal(counts.all, c.entries.length);
  for (const [tab] of CHAIN_TABS) assert.equal(tabEntries(c.entries, tab).length, counts[tab]);
  assert.equal(liveCount(c), 2);
  const frogs = entryOfNest(c.entries, "frogs")!, apes = entryOfNest(c.entries, "apes")!;
  assert.equal(frogs.slug, "bitcoin-frogs"); assert.equal(apes.slug, "boredapeyachtclub");
  assert.ok(frogs.heldByRook !== null && frogs.heldByRook > 0); assert.equal(frogs.floor?.stale, true); assert.equal(frogs.floor?.unit, "sats");
  assert.match(src.image(frogs)!, /^https:\/\/ordinals\.com\/content\/[0-9a-f]{64}i\d+$/, "a Bitcoin nest's picture is its icon inscription at the content gateway");
  assert.match(src.image(apes)!, /^https:\/\/[a-z0-9.]*seadn\.io\//, "an EVM collection's picture is its public marketplace logo");
  assert.match(src.image(frogs, 0)!, /^https:\/\/ordinals\.com\/content\//, "an inscription's picture is the content gateway");
  const stonk = c.entries.find((e) => e.slug === "stonkbrokers-434284142")!;
  assert.ok(stonk.sample.length >= 6 && src.image(stonk, 0)!.startsWith("data:image/svg+xml;base64,"), "a Robinhood Chain collection's items come from its tokenURI: StonkBrokers' are on-chain SVGs");
  const mech = c.entries.find((e) => e.slug === "theroyalmechanica")!;
  assert.ok(mech.sample.length >= 6 && src.image(mech, 0)!.startsWith("https://ipfs.filebase.io/ipfs/"), "an ipfs:// picture is named on a gateway that serves it");
  assert.equal(src.image(apes, 99), null, "no such item: a mark");
  const withPicture = c.entries.filter((e) => src.image(e));
  assert.ok(withPicture.length >= 110, `${withPicture.length} of ${c.entries.length} have a picture`);
  assert.ok(c.entries.filter((e) => e.chain !== "other" && e.sample.some((i) => i.image)).length >= 80, "most collections that a vault could hold have item pictures");
  for (const slug of ["boredapeyachtclub", "azuki", "pudgypenguins", "nouns", "moonbirds", "chain-mancers", "stonkbrokers-434284142"]) assert.ok(c.entries.find((e) => e.slug === slug)!.sample.filter((i) => i.image).length >= 6, `${slug} has item pictures`);
  assert.ok(c.entries.every((e) => !/^https:\/\/(?:ipfs\.io|dweb\.link|cdn\.discordapp\.com|turbo\.ordinalswallet\.com\/inscription\/preview)\//.test(e.image ?? "")), "no picture on a host that no longer serves it");
  assert.ok(c.entries.every((e) => (e.image === null || /^https:\/\//.test(e.image)) && e.sample.every((i) => i.image === null || /^https:\/\//.test(i.image) || /^data:image\/[a-z0-9.+-]+;base64,/.test(i.image))), "every picture URL is https, or an on-chain picture as a data: URI");
  assert.ok(c.entries.every((e) => e.floor === null || e.floor.stale), "every fixture floor is dated: nothing is the price now");
  const one = await src.get<CatalogOne>("/v1/rook/catalog/apes");
  assert.equal(one.entry.slug, "boredapeyachtclub"); assert.equal(one.entry.rook.kind, "live");
  await assert.rejects(src.get("/v1/rook/catalog/nothing"), /no answer/);
});

test("tabs, search and sorts over the fixture: status ranks live → listable → later → unsupported, an unknown figure sorts last, search finds by name, chain, status, contract and Rook slug", async () => {
  const c = await src.get<Catalog>("/v1/rook/catalog");
  const byStatus = sortEntries(c.entries, "status");
  const rank = { live: 0, listable: 1, later: 2, unsupported: 3 };
  for (let i = 1; i < byStatus.length; i++) assert.ok(rank[byStatus[i - 1].rook.kind] <= rank[byStatus[i].rook.kind]);
  assert.equal(byStatus[0].rook.kind, "live");
  const byFloor = sortEntries(c.entries, "floor");
  assert.ok((byFloor[0].floor?.eth ?? 0) >= (byFloor[1].floor?.eth ?? 0)); assert.equal(byFloor.at(-1)!.floor, null, "no floor sorts last");
  const byItems = sortEntries(c.entries, "items");
  assert.ok((byItems[0].items ?? 0) >= (byItems[1].items ?? 0));
  const byName = sortEntries(c.entries, "name");
  assert.ok(byName[0].name.localeCompare(byName[1].name) <= 0);
  assert.ok(searchEntries(c.entries, "punks").some((e) => e.slug === "cryptopunks"));
  assert.ok(searchEntries(c.entries, "solana").every((e) => e.chainName === "Solana") && searchEntries(c.entries, "solana").length >= 10);
  assert.ok(searchEntries(c.entries, "live on rook").every((e) => e.rook.kind === "live") && searchEntries(c.entries, "live on rook").length === 2);
  assert.equal(searchEntries(c.entries, "0xbc4ca0eda7647a8ab7c2061c2e118a18a936f13d")[0].slug, "boredapeyachtclub");
  assert.ok(searchEntries(c.entries, "apes").some((e) => e.rook.kind === "live" && e.rook.slug === "apes"), "a nest's Rook slug is searchable");
  assert.equal(searchEntries(c.entries, "").length, c.entries.length);
  assert.equal(searchEntries(c.entries, "zzzz-nothing").length, 0);
});

test("the wizard's order and what can be launched: live and listable can, later and unsupported cannot; the board's nest list is the same order", async () => {
  const c = await src.get<Catalog>("/v1/rook/catalog");
  const w = wizardOrder(c.entries);
  assert.equal(w[0].rook.kind, "live"); assert.equal(w[1].rook.kind, "live"); assert.equal(w[2].rook.kind, "listable");
  assert.ok(w.every((e) => launchable(e) === (e.rook.kind === "live" || e.rook.kind === "listable")));
  assert.ok(w.filter(launchable).length >= 20 && w.filter((e) => !launchable(e)).length >= 20);
  const stonk = c.entries.find((e) => e.slug === "stonkbrokers-434284142")!;
  assert.equal(stonk.chain, "robinhood"); assert.equal(stonk.rook.kind, "listable"); assert.ok(launchable(stonk));
  assert.equal(c.entries.find((e) => e.slug === "mad-lads")!.rook.kind, "unsupported");
  assert.equal(c.entries.find((e) => e.slug === "cryptopunks")!.rook.kind, "later");
});

test("the words: a floor in its unit and in ETH, dollars only with a rate, a volume of nothing is 0 and of nobody is —, a stale floor says so with its date", () => {
  const e = (floor: CatalogEntry["floor"], extra: Partial<CatalogEntry> = {}): CatalogEntry => ({ slug: "x", name: "X", chain: "bitcoin", chainName: "Bitcoin", chainId: null, standard: "inscriptions", id: "x", description: null, image: null,
    links: { website: null, x: null, discord: null, marketplace: null, explorer: null }, floor, items: null, owners: null, listed: null, volume24hEth: null, volume7dEth: null, heldByRook: null, sample: [], sources: [], rook: { kind: "later", stage: null, why: "" }, curated: true, ...extra });
  assert.equal(floorText(e({ native: "433069", unit: "sats", eth: 0.1367, source: "ordinalswallet", ref: "r", at: 1, stale: false })), "0.137 ETH · 433,069 sats");
  assert.equal(floorText(e({ native: "150000000", unit: "sats", eth: null, source: "seed", ref: "r", at: 1, stale: true })), "1.5 BTC", "no rate: the native unit alone");
  assert.equal(floorText(e({ native: "6847000000000000000", unit: "wei", eth: 6.847, source: "opensea", ref: "r", at: 1, stale: false })), "6.85 ETH");
  assert.equal(floorText(e({ native: "18979000000000", unit: "wei", eth: 0.000018979, source: "opensea", ref: "r", at: 1, stale: false })), "0.000019 ETH");
  assert.equal(floorText(e(null)), "—");
  assert.equal(usdText(6.847, 4000), "$27.4k"); assert.equal(usdText(0.01, 4000), "$40.00"); assert.equal(usdText(6.847, null), null); assert.equal(usdText(null, 4000), null);
  assert.equal(volText(null), "—"); assert.equal(volText(0), "0"); assert.equal(volText(524.6), "525 ETH");
  const now = 1_790_000_000;
  assert.equal(floorNote(e({ native: "1", unit: "wei", eth: 1, source: "opensea", ref: "r", at: now - 120, stale: false }), now), "From OpenSea, 2 min ago.");
  assert.match(floorNote(e({ native: "1", unit: "wei", eth: 1, source: "seed", ref: "r", at: now - 3 * 86_400, stale: true }), now), /^From the seed's record, 3 d ago: not a fresh read, and not the price now\.$/);
  assert.equal(floorNote(e(null), now), "No source reported a floor.");
});
