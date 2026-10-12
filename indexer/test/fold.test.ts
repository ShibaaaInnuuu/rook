import assert from "node:assert/strict";
import { test } from "node:test";
import { DERIVED_TABLES, openDb } from "../src/db.js";
import { apply, lotOf, rebuild } from "../src/fold.js";
import { Live } from "../src/live.js";
import { Views } from "../src/views.js";

import { COIN, T0, story } from "./story.js";

import { TX, WHO, WINNER, COIN2 } from "./story.js";
const dump = (db: ReturnType<typeof openDb>) => Object.fromEntries(DERIVED_TABLES.map((t) => [t, db.prepare(`SELECT * FROM ${t} ORDER BY 1, 2, 3`).all()]));
function filled() {
  const db = openDb(":memory:");
  const ins = db.prepare(`INSERT INTO events (block_number, log_index, tx, address, contract, name, args, ts, listing_id, acquisition_id) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)`);
  for (const e of story()) { apply(db, e); const l = lotOf(db, e); ins.run(e.block, e.logIndex, e.tx, e.address, e.contract, e.name, JSON.stringify(e.args), e.ts, l.listingId, l.acquisitionId); }
  return db;
}

test("every event folds: listings, coins, fees, the purchase state machine, bonds, draws, distributions and payouts", async () => {
  const db = filled(), v = new Views(db, new Live(null, null), () => T0 + 10_000);
  const one = (sql: string) => db.prepare(sql).get() as Record<string, unknown>;
  assert.deepEqual({ ...one(`SELECT kind, paused, root, btc_vault, pending_root, pending_btc FROM listings WHERE id = 0`) }, { kind: "inscriptions", paused: 0, root: "0x" + "ab".repeat(32), btc_vault: null, pending_root: null, pending_btc: null });
  // docs/v2/vault-v2.md: a v1 listing stays at the default and a v2 one carries its buyer and its ceiling, from the chain's
  // own AutoBuyerNamed rather than inferred from whether some other column is null.
  assert.deepEqual({ ...one(`SELECT autonomous, buyer, max_in_flight FROM listings WHERE id = 0`) }, { autonomous: 0, buyer: null, max_in_flight: null });
  assert.deepEqual({ ...one(`SELECT autonomous, buyer, max_in_flight FROM listings WHERE id = 2`) }, { autonomous: 1, buyer: "0x" + "b0".repeat(20), max_in_flight: "20000000000000000000" });
  assert.deepEqual({ ...one(`SELECT kind, paused, btc_vault FROM listings WHERE id = 1`) }, { kind: "rune", paused: 0, btc_vault: "bc1pnew" });
  assert.equal(one(`SELECT count(*) AS n FROM roles`).n, 12, "role, treasury, bond and wiring changes (including the NFT registry wiring, buyer reservations, the guardian and the keeper pause) are kept as history");
  const c = (await v.coin(COIN))!;
  assert.deepEqual([c.lifetimeFeesWei, c.toVaultWei, c.toTreasuryWei, c.harvests, c.treasuryDeferredWei], ["150", "120", "30", 2, "0"], "a paid treasury leg clears what was deferred");
  assert.deepEqual([c.fees24hWei, c.fees7dWei], ["150", "150"]);
  assert.equal(c.tokensForwarded[0].amount, "7");
  assert.deepEqual(c.acquisitions.filter((a) => a.acquisitionId === 0).map((a) => [a.purchaseId, a.chargedWei, a.shareBps]), [[0, "60", 10000]]);
  assert.deepEqual([c.volumeUntilNextBuy.reason, c.live], ["live-unavailable", null], "no node, no live numbers, and it says so");
  const l = (await v.listing(0))!;
  assert.deepEqual(l.declarations.map((d) => [d.purchaseId, d.status, d.returnedWei, d.writtenOffWei, d.bondSlashedWei]).reverse(),
    [[0, "settled", "5", null, "0"], [1, "cancelled", "0", null, "0"], [2, "written-off", "0", "5", "15"]]);
  assert.deepEqual([l.depositedWei, l.capWei, l.capExpired, l.openDeclarations, l.writtenOffWei, l.live], ["120", "60", true, 0, "5", null]);
  assert.deepEqual(l.bondEvents.map((b) => [b.type, b.amountWei]).reverse(), [["posted", "100"], ["withdrawn", "40"], ["slashed", "15"]]);
  const s = v.stats();
  assert.deepEqual([s.coins, s.listings, s.claimedWei, s.toVaultsWei, s.toTreasuryWei, s.releasedWei, s.returnedWei, s.spentWei, s.writtenOffWei], [2, 3, "150", "120", "30", "80", "5", "55", "5"]);
  assert.deepEqual(s.purchases, { declared: 1, released: 0, settled: 1, cancelled: 1, writtenOff: 1 });
  assert.deepEqual(s.draws, { open: 0, pinned: 0, resolved: 0, claimed: 1, delivered: 1, voided: 1, cancelled: 2 });
  assert.deepEqual([s.payouts, s.acquisitions], [{ requested: 2, recorded: 1, overdue: 1 }, { inscriptions: 5, runeReceipts: 1 }]);
});

test("a draw: its states in order, the FIRST destination time kept when the address is corrected, a txid as explorers print it", () => {
  const db = filled(), v = new Views(db, new Live(null, null), () => T0 + 10_000), d = v.draw(0, 0)!;
  assert.deepEqual([d.type, d.state, d.mode, d.total, d.challengeEndsIn], ["draw", "delivered", "draw-weighted", "1000", null]);
  assert.deepEqual([d.draw!.round, d.draw!.winningTicket, d.draw!.destination, d.draw!.deliveredTxid, d.draw!.overdue], [777, "412", "bc1pfixed", "77".repeat(32), true]);
  assert.ok(d.draw!.destinationSetAt! < d.draw!.deliveredAt!, "the delivery clock ran from the first address");
  assert.match(d.draw!.drandUrl!, /\/public\/777$/);
  assert.match(d.verifySnapshot.command!, /--kind weighted$/);
  assert.deepEqual(d.timeline.map((t) => t.event), ["Receipted", "Chosen", "DrawOpened", "DrawPinned", "DrawResolved", "Won", "DestinationSet", "DestinationSet", "DeliveryOverdue", "Delivered"]);
  assert.deepEqual([v.draw(0, 1)!.state, v.draw(0, 2)!.state, v.draw(0, 2)!.draw!.voided!.unclaimedTicket, v.draw(0, 4)!.state, v.draw(0, 4)!.mode], ["cancelled", "voided", "9", "won", "named"]);
  assert.equal(v.draw(0, 3), null, "a lot that is simply held was never a draw");
  assert.equal(v.draw(7, 7), null);
});

test("a rune distribution: claims add up, the roll-over is kept, and the verification hint asks a runes indexer for at least what is still undelivered", async () => {
  const db = filled(), v = new Views(db, new Live(null, null), () => T0), d = v.draw(1, 0)!;
  assert.deepEqual([d.type, d.state, d.distribution!.claimedUnits, d.distribution!.claims, d.distribution!.rolledOver!.amount], ["distribution", "distribution-rolled", "500", 2, "400"]);
  assert.match(d.verifySnapshot.command!, /--kind pro-rata --total 900$/);
  const a = v.acquisitions(10, 0, 1).items[0];
  assert.equal(a.verify.check.kind, "rune");
  if (a.verify.check.kind === "rune") {
    assert.deepEqual([a.verify.check.expectAtLeast, a.verify.check.indexerPath], ["600", "/v1/indexer/address/bc1pnew/runes/840000:3/balance"], "900 received, 300 paid out and recorded");
  }
  const l = (await v.listing(1))!;
  assert.deepEqual([l.minimumPayout, l.timeToDeliver.count], [{ units: "250", expiry: T0 + 90_000, expired: false }, 1]);
});

test("the fold is a function of the events: rebuilding from the events table gives byte-for-byte the tables that incremental application gave, however often", () => {
  const db = filled(), before = JSON.stringify(dump(db));
  assert.equal(rebuild(db), story().length);
  assert.equal(JSON.stringify(dump(db)), before);
  rebuild(db); rebuild(db);
  assert.equal(JSON.stringify(dump(db)), before);
});

test("a log from an address that was never learnt as a vault or a router changes nothing", () => {
  const db = openDb(":memory:");
  apply(db, { block: 1, logIndex: 0, tx: TX, address: "0x" + "bb".repeat(20), contract: "vault", name: "Deposited", args: { coin: COIN, amount: "1" }, ts: 1 });
  apply(db, { block: 1, logIndex: 1, tx: TX, address: "0x" + "bb".repeat(20), contract: "router", name: "Harvested", args: { claimed: "1", toVault: "1", toTreasury: "0" }, ts: 1 });
  assert.equal(JSON.stringify(dump(db)), JSON.stringify(dump(openDb(":memory:"))));
});

test("the story leaves nothing out: every event the indexer declares is emitted in it at least once", async () => {
  const { EVENTS } = await import("../src/abis.js");
  const all = Object.entries(EVENTS).flatMap(([k, abi]) => (abi as readonly { name: string }[]).map((e) => `${k}.${e.name}`));
  const seen = new Set(story().map((e) => `${e.contract}.${e.name}`));
  assert.equal(all.length, 59);
  assert.deepEqual(all.filter((x) => !seen.has(x)), []);
});
