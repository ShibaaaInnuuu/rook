import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { CollectionMosaic, mosaicPicks } from "../src/ui/CollectionMosaic";

/** The collections page hero's mosaic: the collections' own pictures (served by the site), mixed across chains, hidden on narrow screens. */
test("the mosaic renders 20 tiles, each the collection's local picture, linked to its page, decorative to assistive technology", () => {
  const html = renderToStaticMarkup(createElement(CollectionMosaic, { hrefOf: (slug: string) => `#/collection/${slug}` }));
  assert.match(html, /^<div class="cmosaic" aria-hidden="true">/);
  assert.equal((html.match(/class="cmosaic__tile"/g) ?? []).length, 20);
  const srcs = [...html.matchAll(/<img[^>]* src="([^"]+)"/g)].map((m) => m[1]);
  assert.equal(srcs.length, 20);
  for (const s of srcs) assert.match(s, /^\/collections\/[a-z0-9_-]+\.(webp|png|jpg|gif)$/, `${s} is served by the site`);
  assert.equal((html.match(/loading="lazy"/g) ?? []).length, 20);
  assert.equal((html.match(/width="84" height="84"/g) ?? []).length, 20, "sized, so nothing shifts as they load");
  assert.equal((html.match(/tabindex="-1"/g) ?? []).length, 20, "out of the tab order: the list below has the same links");
  assert.match(html, /<a class="cmosaic__tile" href="#\/collection\/milady" tabindex="-1" aria-label="Milady Maker"/);
});

test("the mosaic's picks mix Ethereum, Solana and the Ordinals, in the same order every time", () => {
  const picks = mosaicPicks(20);
  assert.deepEqual(picks.map((c) => c.slug), mosaicPicks(20).map((c) => c.slug));
  assert.deepEqual(picks.slice(0, 3).map((c) => c.chain), ["Ethereum", "Solana", "Bitcoin"]);
  for (const ch of ["Ethereum", "Solana", "Bitcoin"]) assert.ok(picks.filter((c) => c.chain === ch).length >= 6, `${ch} has at least 6 tiles`);
  assert.equal(new Set(picks.map((c) => c.slug)).size, 20, "no collection twice");
});

test("the mosaic is hidden below 900 px and shown from 900 px", () => {
  const css = readFileSync(new URL("../src/styles/rook.css", import.meta.url), "utf8");
  assert.match(css, /\n\.cmosaic \{ display: none; \}/);
  assert.match(css, /@media \(min-width: 900px\) \{[^@]*\.cmosaic \{ display: block;/);
  assert.match(css, /@media \(prefers-reduced-motion: reduce\) \{ \.cmosaic__grid \{ animation: none;/);
});
