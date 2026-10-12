import assert from "node:assert/strict";
import { test } from "node:test";
import { openDb } from "../src/db.js";
import { apply, lotOf } from "../src/fold.js";
import { Live } from "../src/live.js";
import { Views } from "../src/views.js";
import { T0, story } from "./story.js";

/**
 * docs/phase14.md §8. What a vault's own page asks the indexer for, and what the browser must not be left to compute.
 *
 * The story in test/story.ts already reaches every one of these states: listing 0 releases twice, has one of those
 * purchases settled and the other written off with the bond part-slashed, and receipts five inscriptions out of the ONE
 * purchase — which is exactly the case a per-item price would get wrong. Listing 1 is a rune that was receipted without a
 * release at all, had a claim paid out and another requested and left overdue.
 */
function views() {
  const db = openDb(":memory:");
  const ins = db.prepare(`INSERT INTO events (block_number, log_index, tx, address, contract, name, args, ts, listing_id, acquisition_id) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)`);
  for (const e of story()) { apply(db, e); const l = lotOf(db, e); ins.run(e.block, e.logIndex, e.tx, e.address, e.contract, e.name, JSON.stringify(e.args), e.ts, l.listingId, l.acquisitionId); }
  return new Views(db, new Live(null, null), () => T0 + 10_000);
}

test("a vault's money out is summed over every purchase, not over the page of declarations", async () => {
  const l = (await views().listing(0))!;
  // released 60 then 20; 5 of the first came back; only the first was settled
  assert.deepEqual([l.releasedWei, l.returnedWei, l.spentWei], ["80", "5", "55"]);
  assert.equal(l.writtenOffWei, "5", "and what the slashed bond did not cover is its own figure, not part of what was spent");
  assert.ok(BigInt(l.spentWei) + BigInt(l.writtenOffWei) < BigInt(l.releasedWei), "a release still in the purchaser's hands is neither spent nor lost");
  const one = (await views().listing(1))!;
  assert.deepEqual([one.releasedWei, one.returnedWei, one.spentWei], ["0", "0", "0"], "a receipt without a release spends nothing: the figure is zero, not the receipt's sats");
});

test("what a vault has bought: a count for a collection, a quantity for a rune, and never one standing in for the other", async () => {
  const coll = (await views().listing(0))!.holdings;
  assert.deepEqual([coll.receipts, coll.delivered, coll.inscriptionsHeld], [5, 1, 4]);
  assert.equal(coll.runeUnits, null, "an inscriptions listing has no quantity, and gets null rather than a zero");

  const rune = (await views().listing(1))!.holdings;
  assert.deepEqual([rune.receipts, rune.delivered, rune.inscriptionsHeld], [1, 0, null]);
  // 900 units in; 300 paid out on a recorded claim; the second payout has no txid and is not out yet
  assert.deepEqual(rune.runeUnits, { received: "900", delivered: "0", paidOut: "300", held: "600" });
});

test("the held figure the page prints is the one the verification hint hands a runes indexer", async () => {
  const l = (await views().listing(1))!;
  const check = l.acquisitionsRecent[0].verify.check;
  assert.equal(check.kind, "rune");
  assert.equal(check.kind === "rune" ? check.expectAtLeast : null, l.holdings.runeUnits!.held,
    "one method serves both, so what a reader is told to check and what the page shows cannot drift apart");
});

test("a receipt carries its purchase's cost and how many receipts share it, undivided", async () => {
  const l = (await views().listing(0))!;
  const a = l.acquisitionsRecent.find((x) => x.acquisitionId === 0)!;
  assert.equal(a.purchaseSpentWei, "55", "released 60 less the 5 that came back");
  assert.equal(a.purchaseAcquisitions, 5, "one purchase, five inscriptions: 55/5 is the contract's own division and not what an item cost");
  for (const x of l.acquisitionsRecent) assert.deepEqual([x.purchaseSpentWei, x.purchaseAcquisitions], ["55", 5], "every row of the purchase says the same thing about it");
});

test("sats are per purchase and amounts are per row, which is why nothing sums the first", async () => {
  const l = (await views().listing(0))!;
  const sats = l.acquisitionsRecent.map((a) => a.satsPaid);
  assert.equal(new Set(l.acquisitionsRecent.map((a) => a.btcTxid)).size, 1, "five receipts, one Bitcoin transaction");
  assert.notEqual(sats.reduce((t, s) => t + BigInt(s), 0n), 150_001n, "adding them up would be adding one transaction's sats five times");
  assert.equal(l.acquisitionsRecent.reduce((t, a) => t + BigInt(a.amount), 0n), 5n, "amounts, by contrast, do sum: five inscriptions");
});

test("a cap is served from both ends of its hour, and a vault that never had one has neither", async () => {
  const v = views();
  const l = (await v.listing(0))!;
  assert.equal(l.capPostedAt, l.capHistory[0].at, "the same row the cap and its expiry come from");
  // the story invents its own expiry rather than adding the contract's CAP_TTL to a block time, so what is asserted is the
  // order of the two, which is the property a page reads: a cap is posted, and expires after it
  assert.ok(l.capPostedAt! < l.capExpiry!, `posted at one end of its window and expiring at the other: ${l.capPostedAt} < ${l.capExpiry}`);
  const rune = (await v.listing(1))!;
  assert.deepEqual([rune.capWei, rune.capExpiry, rune.capExpired, rune.capPostedAt], [null, null, null, null],
    "no cap was ever posted for it, and four nulls say so rather than four zeros");
  assert.equal(v.listings().items.find((x) => x.id === 0)!.capPostedAt, l.capPostedAt, "the list carries it too: the vaults page needs it as much as the vault's own page");
});
