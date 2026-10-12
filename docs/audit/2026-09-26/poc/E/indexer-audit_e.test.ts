import assert from "node:assert/strict";
import { test } from "node:test";
import type { CustodyItem, FloorModuleRead, ItemCheck } from "../types.js";
import { computeInvariant, type InvariantInput } from "../src/wrapped/invariant.js";

/**
 * AUDIT E (services and workflows) proof-of-concept tests; no source is changed. The reserves page (and the K7 reconciler, which
 * runs this same code) is GREEN in states where the ETH-backed half of the supply is not backed, and while the Floor Module is
 * stale. Run: cd indexer && node --experimental-sqlite --no-warnings --import tsx --test test/audit_e.test.ts
 */
const E = 10n ** 18n;
const check = (verdict: ItemCheck["verdict"], checkedAt = 1_000): ItemCheck => ({ verdict, reason: null, checkedAt, atBlock: 1, sources: [], arrivalTx: null });
const item = (ref: string): CustodyItem => ({ itemId: ref, ref, refUrl: null, promisedTo: null, check: check("in-custody"),
  arrival: { how: "purchase", tx: null, txUrl: null, mintTx: "0x00", mintedAt: 1, mintBlock: 1 } });
const moduleRead = (over: Partial<Extract<FloorModuleRead, { kind: "read" }>>): FloorModuleRead => ({ kind: "read", address: "0x00", ethBackedUnits: "0", reserveUnits: "0", convertibleUnits: "0",
  floatUnits: "0", escrowWei: "0", backingWei: "0", standingReserveWei: "0", revenueWei: "0", floorWei: "1", floorPostedAt: 1_000, floorFresh: true, priceWei: "1", fillBandBps: 300, fills: [],
  spreadMintBps: 15, spreadBurnBps: 15, betaConvertibleWei: "0", betaReserveWei: "0", mintPriceWei: "1", mintRoomWei: "0", burnPriceWei: "1", burnRoomUnits: "0", loanOutstandingWei: "0",
  treasury: null, volume7dWei: "0", at: 1_050, block: 10, ...over });
// 3 frogs in custody, no ticket in flight, and 10 ETH-backed units at the module: supply 13 wX
const input = (floor: FloorModuleRead): InvariantInput => ({ now: 1_100, maxAgeS: 900, ledgerComplete: true, ticketsInFlight: 0, floor,
  wrapper: { ok: true, totalSupply: 13n * E, held: 3, openTickets: 0, block: 10, at: 1_050 }, items: [item("a"), item("b"), item("c")] });

test("E-05 PoC: 10 ETH-backed wX standing against ONE wei of escrow is GREEN (only an escrow of exactly 0 is red)", () => {
  const r = computeInvariant(input(moduleRead({ ethBackedUnits: String(10n * E), convertibleUnits: String(10n * E), escrowWei: "1", backingWei: "1" })));
  console.log(`E-05: status=${r.status} reasons=${JSON.stringify(r.reasons)} escrow=${r.equation.escrowWei} wei for ${r.equation.ethBackedUnits} units`);
  assert.equal(r.status, "green", "the page and the reconciler say every wX is backed");
  assert.equal(r.equation.holds, true);
});

test("E-05 PoC: the module's own escrowBalance() is storage accounting; nothing compares it with the module's actual ETH balance", () => {
  // the reserves code never calls eth_getBalance on the module: an accounting bug that pays out more than it books stays green
  const r = computeInvariant(input(moduleRead({ ethBackedUnits: String(10n * E), convertibleUnits: String(10n * E), escrowWei: String(6n * E) })));
  assert.equal(r.status, "green");
});

test("E-05 PoC: a stale keeper floor (mint and burn refused, conversions stopped) is still GREEN", () => {
  const r = computeInvariant(input(moduleRead({ ethBackedUnits: String(10n * E), convertibleUnits: String(10n * E), escrowWei: String(6n * E), floorFresh: false, floorPostedAt: 1 })));
  assert.equal(r.status, "green", "floorFresh=false is not a reason: holders of ETH-backed wX cannot exit at the module, and nothing says so");
});
