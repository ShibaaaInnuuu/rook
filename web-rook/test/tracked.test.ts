import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import type { Catalog } from "../src/rook/catalogTypes";
import { COLLECTIONS } from "../src/rook/sol/collections";
import { countTabs, newestRecord, rookOf, TRACKED_TABS, trackedRows, viewRows } from "../src/rook/sol/tracked";
import { KNOWN, SOLANA } from "../src/rook/routes";

const here = (p: string) => new URL(p, import.meta.url);
const index = JSON.parse(readFileSync(here("../public/fixtures/catalog/index.json"), "utf8")) as { files: Record<string, string> };
const cat = JSON.parse(readFileSync(here(`../public/fixtures/catalog/${index.files["/v1/rook/catalog"]}`), "utf8")) as Catalog;
const rows = trackedRows(cat.entries, (s) => `#/collection/${s}`);

test("#/tracked is a Solana route (no wagmi chunk), and the bundled catalogue answers it", () => {
  assert.ok(KNOWN.has("tracked") && SOLANA.has("tracked"));
  assert.ok(cat.entries.length > 50);
});

test("the tracked list: Bitcoin, Ethereum, Base, Other; no Robinhood Chain entry and no Robinhood tab", () => {
  assert.deepEqual(TRACKED_TABS.map(([t]) => t), ["all", "bitcoin", "ethereum", "base", "other"]);
  assert.ok(rows.length > 0 && rows.every((r) => r.e.chain !== "robinhood"));
  const n = countTabs(rows);
  assert.equal(n.all, n.bitcoin + n.ethereum + n.base + n.other);
  assert.ok(n.bitcoin > 0 && n.ethereum > 0 && n.base > 0 && n.other > 0);
});

test("Rook's collections link inside the site, every other row to an https marketplace / explorer / website", () => {
  const rook = rows.filter((r) => r.rook);
  assert.ok(rook.length >= 10, `${rook.length} of the twelve are in the catalogue`);
  for (const r of rook) { assert.equal(r.external, false); assert.equal(r.link, `#/collection/${r.rook!.slug}`); }
  for (const r of rows.filter((x) => !x.rook)) { assert.equal(r.external, true); if (r.link) assert.match(r.link, /^https:\/\//); }
  assert.ok(rows.filter((r) => !r.rook && r.link).length / rows.filter((r) => !r.rook).length > 0.9, "nearly every tracked row goes somewhere");
  assert.equal(rookOf(cat.entries.find((e) => e.slug === "boredapeyachtclub")!)?.slug, "bayc");
  assert.equal(rookOf(cat.entries.find((e) => e.slug === "bitcoin-frogs")!)?.slug, "bitcoin-frogs");
  assert.equal(rookOf(cat.entries.find((e) => e.slug === "cryptopunks")!), null);
});

test("view: Rook first in listing order, then the tabs, search and sorts", () => {
  const v = viewRows(rows, "all", "", "rook");
  const head = v.filter((r) => r.rook).map((r) => r.rook!.listingId);
  assert.deepEqual(head, [...head].sort((a, b) => a - b));
  assert.ok(v.slice(0, head.length).every((r) => r.rook));
  assert.ok(viewRows(rows, "bitcoin", "", "floor").every((r) => r.e.chain === "bitcoin"));
  assert.ok(viewRows(rows, "all", "punk", "name").every((r) => /punk/i.test(r.e.name + r.e.slug + r.e.id)));
  const byOwners = viewRows(rows, "all", "", "owners").map((r) => r.e.owners ?? -1);
  for (let i = 1; i < byOwners.length; i++) assert.ok(byOwners[i - 1] >= byOwners[i]);
  assert.ok((newestRecord(cat) ?? 0) > 1_700_000_000);
  // 41 since the Robinhood Chain measurement (2026-10-11): the three robinhood-collections.json collections have pages too
  assert.equal(COLLECTIONS.length, 41, "nft-pilot.json's 26, the 12 Solana collections and the 3 Robinhood Chain ones");
});
