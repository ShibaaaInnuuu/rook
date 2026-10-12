import assert from "node:assert/strict";
import { test } from "node:test";
import { mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { membersForListing, proofForListing, leafOf, memberId, STAGING_MEMBERS } from "../staging/members.js";
import { plannedListings, processProof, type Catalogue } from "../../staging/catalogue/listings.js";
import { MANIFEST_NOTE, type Manifest } from "../../staging/bootstrap/listings.js";
import { FixtureSource } from "../src/sources.js";

const CATALOGUE = JSON.parse(readFileSync(new URL("../../web/public/catalogue/catalogue.json", import.meta.url), "utf8")) as Catalogue;

/**
 * The manifest the listing creator writes, for two catalogue listings, as it lands on the staging volume.
 *
 * Which two is DERIVED from the committed catalogue rather than named: it is the fifty most-traded collections and the
 * fifty most-traded runes of the fetch that built it (docs/phase9.md §11, §11.3), so the slugs move with every fetch and
 * a test that pinned two of them would fail on a fresh catalogue instead of on a defect.
 */
function manifestDir(): { dir: string; manifest: Manifest } {
  const plan = plannedListings(CATALOGUE, { chainId: 9466301, fixtureMembers: STAGING_MEMBERS });
  const pepes = plan.listings.find((l) => l.kind === "inscriptions")!;   // any collection the catalogue gave ids for
  const dog = plan.listings.find((l) => l.kind === "rune")!;             // and any rune, which is listed under its id
  const manifest = {
    source: plan.source, fetchedAt: plan.fetchedAt, chainId: 9466301, placeholderRoots: true, note: MANIFEST_NOTE,
    progress: { created: 2, total: 2, done: true, startedAt: "now", finishedAt: "now", ms: 1 },
    skipped: plan.skipped,
    listings: [
      { id: 2, kind: "inscriptions" as const, name: pepes.name, ref: pepes.ref, vault: "0x2", btcVault: pepes.btcVault, divisibility: 0,
        membershipRoot: pepes.membershipRoot, floorSats: pepes.floorSats, weight: pepes.weight, fromCatalogue: true, rootCovers: pepes.rootCovers, sourceUrl: null },
      { id: 3, kind: "rune" as const, name: dog.name, ref: dog.ref, vault: "0x3", btcVault: dog.btcVault, divisibility: dog.divisibility,
        membershipRoot: null, floorSats: null, weight: dog.weight, fromCatalogue: true, rootCovers: null, sourceUrl: null },
    ],
  };
  const dir = mkdtempSync(join(tmpdir(), "etch-keeper-listings-"));
  writeFileSync(`${dir}/listings.json`, JSON.stringify(manifest));
  return { dir, manifest };
}

test("a catalogue listing's members: the real inscription ids first, the fixture ids after, and proofs against the root the registry holds", () => {
  const { dir, manifest } = manifestDir();
  const m = membersForListing(dir, 2)!;
  assert.ok(m, "the manifest names listing 2");
  assert.equal(m.root, manifest.listings[0].membershipRoot, "the root the keeper proves against is the one the registry was given");
  assert.equal(m.real, manifest.listings[0].rootCovers!.realIds.length);
  assert.deepEqual(m.ids.slice(0, m.real), manifest.listings[0].rootCovers!.realIds, "the catalogue's own ids come first, so a real one is delivered first");
  assert.equal(m.ids[m.real], memberId(0), "then the fixture members, so a fork that is never reset never runs out");
  assert.equal(m.ids.length, m.real + STAGING_MEMBERS);

  // every member can prove itself, exactly as AssetVault.settleInscription checks it
  for (const i of [0, m.real - 1, m.real, m.real + 1, m.ids.length - 1]) {
    assert.equal(processProof(proofForListing(m, i), leafOf(m.ids[i])), m.root, `member ${i} (${m.ids[i]})`);
  }
  // and an id that is not in this collection cannot
  assert.notEqual(processProof(proofForListing(m, 0), leafOf("f".repeat(64) + "i0")), m.root);
});

test("a listing the manifest does not name keeps the fixture collection the deploy gave it", () => {
  const { dir } = manifestDir();
  assert.equal(membersForListing(dir, 0), null, "the demo listing is not in the manifest");
  assert.equal(membersForListing(dir, 3), null, "and a rune has no membership at all");
  assert.equal(membersForListing(mkdtempSync(join(tmpdir(), "etch-empty-")), 2), null, "no manifest, no catalogue members");
});

test("a rune has no floor to post a cap from, and the source says so in the words the operator listens for", async () => {
  const dir = mkdtempSync(join(tmpdir(), "etch-prices-"));
  writeFileSync(`${dir}/prices.json`, JSON.stringify({ inscriptions: { "pixel-pepes": "411099" }, runes: {} }));
  const source = new FixtureSource(`${dir}/prices.json`);
  assert.deepEqual(await source.inscriptionFloor("pixel-pepes"), { sats: 411099n, ref: "fixture:pixel-pepes" });

  // the operator turns exactly this refusal into "no cap is posted", and anything else into a failure it reports
  const NO_PRICE = /has no (floor|price)/;
  await assert.rejects(source.runeUnitPrice("840000:3"), (e: Error) => NO_PRICE.test(e.message), "a rune with no floor");
  await assert.rejects(source.inscriptionFloor("not-a-collection"), (e: Error) => NO_PRICE.test(e.message), "a collection with no floor");
});
