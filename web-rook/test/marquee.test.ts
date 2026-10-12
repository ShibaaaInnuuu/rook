import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { COLLECTIONS, LAUNCH_COUNT } from "../src/rook/sol/collections";
import { CollectionMarquee, marqueeRows } from "../src/ui/CollectionMarquee";

const render = () => renderToStaticMarkup(createElement(CollectionMarquee, { hrefOf: (slug: string) => `#/collection/${slug}`, allHref: "#/collections", count: LAUNCH_COUNT }));

/** The front page hero's collections band: every collection's own picture, twice per row for the loop, each collection one focusable link. */
test("the band shows every collection once as a link, and its loop copy hidden and out of the tab order", () => {
  const html = render();
  assert.match(html, /^<section class="cmarq" aria-labelledby="cmarq-h">/);
  assert.match(html, /id="cmarq-h">38 collections to launch on/);
  assert.equal((html.match(/class="cmarq__rail /g) ?? []).length, 2, "two rows");
  const tiles = html.match(/<li class="cmarq__tile[^"]*"[^>]*>/g) ?? [];
  assert.equal(tiles.length, COLLECTIONS.length * 2);
  assert.equal(tiles.filter((t) => t.includes('aria-hidden="true"')).length, COLLECTIONS.length, "the loop copy is hidden");
  assert.equal((html.match(/tabindex="-1"/g) ?? []).length, COLLECTIONS.length, "and out of the tab order");
  for (const c of COLLECTIONS) assert.equal((html.match(new RegExp(`href="#/collection/${c.slug}"`, "g")) ?? []).length, 2, `${c.slug} is in the band`);
  const srcs = [...html.matchAll(/<img[^>]* src="([^"]+)"/g)].map((m) => m[1]);
  assert.ok(srcs.length >= COLLECTIONS.length * 2 - 6, "pictures, or the initials mark where a collection has none");
  for (const s of srcs) assert.match(s, /^\/collections\/[a-z0-9_-]+\.(webp|png|jpg|gif)$/, `${s} is served by the site`);
  assert.equal((html.match(/loading="lazy"/g) ?? []).length, srcs.length);
  assert.equal((html.match(/width="44" height="44"/g) ?? []).length, srcs.length, "sized, so nothing shifts as they load");
  assert.match(html, /≈ 85 SOL to graduate/);
  assert.equal((html.match(/cmarq__tile--soon/g) ?? []).length, 3 * 2, "the 3 Robinhood Chain collections are marked soon");
  assert.match(html, /<a href="#\/collections">All collections →<\/a>/);
});

test("the rows hold all 38 launch collections and the 3 Robinhood ones, mixed across chains, in the same order every time", () => {
  const [a, b] = marqueeRows();
  assert.deepEqual([a, b].map((r) => r.map((c) => c.slug)), marqueeRows().map((r) => r.map((c) => c.slug)));
  assert.equal(a.length + b.length, COLLECTIONS.length);
  assert.equal(new Set([...a, ...b].map((c) => c.slug)).size, COLLECTIONS.length, "no collection twice");
  assert.equal([...a, ...b].filter((c) => c.chain !== "Robinhood Chain").length, LAUNCH_COUNT);
  assert.deepEqual(a.slice(0, 4).map((c) => c.chain), ["Ethereum", "Solana", "Bitcoin", "Robinhood Chain"]);
  for (const row of [a, b]) assert.ok(Math.abs(a.length - b.length) <= 1 && new Set(row.map((c) => c.chain)).size >= 2, "each row mixes chains");
});

test("the band pauses on hover and focus, stands still with reduced motion, and never scrolls the page", () => {
  const css = readFileSync(new URL("../src/styles/rook.css", import.meta.url), "utf8");
  assert.match(css, /\.cmarq:hover \.cmarq__track, \.cmarq__track:focus-within \{ animation-play-state: paused; \}/);
  assert.match(css, /\.cmarq__rail \{ overflow: hidden;/);
  assert.match(css, /@media \(prefers-reduced-motion: reduce\) \{\s*\.cmarq__track \{ animation: none; \}\s*\.cmarq__rail \{ overflow-x: auto;[^}]*\}\s*\.cmarq__tile\[aria-hidden="true"\] \{ display: none; \}/);
  assert.match(css, /\.cmarq__rail--rev \.cmarq__track \{ animation-direction: reverse; \}/);
});
