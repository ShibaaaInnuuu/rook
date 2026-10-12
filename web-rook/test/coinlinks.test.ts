import assert from "node:assert/strict";
import { test } from "node:test";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { COLLECTIONS, ROBINHOOD_COLLECTIONS } from "../src/rook/sol/collections";
import { COLLECTION_LINKS, coinLinksOf } from "../src/rook/sol/collectionLinks";
import { EMPTY_DRAFT, launchCosts, normalizeLink, prefillLinks, withCollection, type Draft } from "../src/rook/sol/launchWizard";
import { CollectionLinksRow } from "../src/ui/CollectionLinksRow";
import { LaunchCosts } from "../src/ui/LaunchCosts";
import { COLLECTION_SLUGS, checkFields, coinMetadata } from "../server.mjs";

/** A coin's Website and X start as its collection's official links (collectionLinks.ts), and the creator can change or clear them. */

test("every collection in collections.ts has links to prefill", () => {
  for (const c of [...COLLECTIONS, ...ROBINHOOD_COLLECTIONS]) {
    assert.ok(COLLECTION_LINKS[c.slug], `${c.slug} has no links`);
    assert.ok(coinLinksOf(c.slug)?.website, `${c.slug}: a website or a marketplace page`);
  }
  assert.equal(coinLinksOf("not-a-collection"), null);
  assert.deepEqual(prefillLinks(""), { website: "", twitter: "" });
});

test("the server's COLLECTION_SLUGS is exactly collectionLinks.ts' collections (underscored Solana slugs included)", () => {
  assert.deepEqual([...COLLECTION_SLUGS].sort(), Object.keys(COLLECTION_LINKS).sort());
  assert.ok(COLLECTION_SLUGS.has("mad_lads"));
});

test("picking a collection prefills Website and X: X as @handle, the website or else the first marketplace page", () => {
  const d = withCollection(EMPTY_DRAFT, "azuki");
  assert.equal(d.collection, "azuki");
  assert.equal(d.website, "https://azuki.com");
  assert.equal(d.twitter, "@Azuki");
  assert.equal(normalizeLink("twitter", d.twitter), "https://x.com/Azuki", "the prefill is a valid X input");
  assert.equal(d.telegram, "", "Telegram is never prefilled");
  assert.deepEqual(prefillLinks("bayc"), { website: "https://opensea.io/collection/boredapeyachtclub", twitter: "@BoredApeYC" });
  for (const r of ROBINHOOD_COLLECTIONS) assert.equal(prefillLinks(r.slug).twitter, "", `${r.slug}: no X recorded, none prefilled`);
});

test("switching collections updates an untouched field and keeps an edited or typed one", () => {
  const a = withCollection(EMPTY_DRAFT, "azuki");
  const edited: Draft = { ...a, website: "https://mycoin.example" };
  const b = withCollection(edited, "bitcoin-frogs");
  assert.equal(b.twitter, "@BitcoinFrogs", "untouched: follows the collection");
  assert.equal(b.website, "https://mycoin.example", "edited: kept");
  // a field the creator cleared is empty: the next collection fills it
  assert.equal(withCollection({ ...a, twitter: "" }, "milady").twitter, "@MiladyMaker333");
  // to a collection with no X: an untouched X is cleared, a typed one is kept
  assert.equal(withCollection(a, "chainmancers").twitter, "");
  assert.equal(withCollection({ ...a, twitter: "@mine" }, "chainmancers").twitter, "@mine");
  // the same collection again changes nothing (a cleared field stays clear)
  const cleared = { ...a, website: "" };
  assert.equal(withCollection(cleared, "azuki"), cleared);
});

test("the creator's values reach the metadata as sent; empty stays empty", () => {
  const d = { ...withCollection(EMPTY_DRAFT, "mad_lads"), name: "Lad", symbol: "LAD", twitter: "@mycoin" };
  const req = { name: d.name, symbol: d.symbol, description: "", collection: d.collection,
    website: normalizeLink("website", d.website) ?? "", twitter: normalizeLink("twitter", d.twitter) ?? "", telegram: "" };
  const f = checkFields(req);
  assert.equal(f.error, undefined, "an underscored slug is accepted");
  const m = coinMetadata(f.fields!, "https://img.example/a.png", "image/png");
  assert.deepEqual(m.extensions, { website: "https://www.madlads.com", twitter: "https://x.com/mycoin" });
  const none = coinMetadata(checkFields({ ...req, website: "", twitter: "" }).fields!, "https://img.example/a.png", "image/png");
  assert.deepEqual(none.extensions, {});
  assert.equal("website" in none || "twitter" in none || "external_url" in none, false);
});

test("a collection coin's page shows the collection's links on their own labeled row", () => {
  const html = renderToStaticMarkup(createElement(CollectionLinksRow, { slug: "bitcoin-frogs", name: "Bitcoin Frogs" }));
  assert.match(html, /Collection \(Bitcoin Frogs\)/);
  assert.match(html, /href="https:\/\/bitcoinfrogs\.com"/);
  assert.match(html, /href="https:\/\/x\.com\/BitcoinFrogs"/);
  assert.match(renderToStaticMarkup(createElement(CollectionLinksRow, { slug: "chainmancers", name: "Chainmancers" })), /OpenSea ↗/);
  assert.equal(renderToStaticMarkup(createElement(CollectionLinksRow, { slug: "rook", name: "ROOK" })), "");
});

test("the review step's cost lines: one short explanation each behind \"What are these?\"; the total says SOL and the wX first buy apart", () => {
  const costs = launchCosts(2_000_000_000n);
  const quote = { isSol: false, tok: { name: "Pudgy Penguins", symbol: "wPUDGY", floor: 59_000_000_000n } };
  const shut = renderToStaticMarkup(createElement(LaunchCosts, { costs, firstBuy: 2_000_000_000n, quote }));
  assert.equal((shut.match(/class="lcost__why"/g) ?? []).length, 0, "collapsed by default");
  assert.match(shut, /aria-expanded="false"[^>]*>What are these\?</);
  const html = renderToStaticMarkup(createElement(LaunchCosts, { costs, firstBuy: 2_000_000_000n, quote, explain: true }));
  assert.equal((html.match(/class="lcost__why"/g) ?? []).length, 4, "one per line but the total");
  assert.match(html, /Solana&#x27;s deposit for your coin&#x27;s accounts\. Not refunded\./);
  assert.match(html, /Metaplex&#x27;s charge for the name, ticker and image\./);
  assert.match(html, /Solana&#x27;s transaction fee \(upper estimate\)\./);
  assert.match(html, /Spent from your wallet; not in the SOL total\./);
  // SOL is the figure; the tokens in plain words and the wX label under it; screen readers get the sentence
  assert.match(html, /<span aria-hidden="true">≈ 118 SOL<\/span><span class="vh">≈ 118 SOL \(2 Pudgy Penguins tokens · 2 wPUDGY\) from your wallet<\/span>/);
  assert.match(html, /class="lcost__sub">2 Pudgy Penguins tokens · 2 wPUDGY · from your wallet</);
  assert.match(html, /class="vh">≈ [0-9.]+ SOL, plus your first buy of ≈ 118 SOL \(2 Pudgy Penguins tokens · 2 wPUDGY\)</);
  assert.match(html, /class="lcost__sub">\+ first buy ≈ 118 SOL \(2 wPUDGY\)</);
  const none = renderToStaticMarkup(createElement(LaunchCosts, { costs: launchCosts(0n), firstBuy: 0n, quote, explain: true }));
  assert.doesNotMatch(none, /plus your first buy/);
  assert.match(none, /Optional: buy your own coin/);
});
