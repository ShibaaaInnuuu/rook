import assert from "node:assert/strict";
import { test } from "node:test";
import { ADVANCE_STALE_S_DEFAULT, backingStatus, type Backing, type BackingRead } from "../src/backing.js";

/** docs/v6/relay-conversion.md §6: the reconciler shows the unit in transit, and an advance open past 48 h is red. */
const E = 10n ** 18n, NOW_S = 1_790_000_000;
const mod = (o: Partial<Backing> = {}): Backing => {
  const books = { backing: 2n * E, reserve: E / 2n, surplus: E / 10n, revenue: 0n, gradEth: 0n, rentalEscrow: 0n, renterPool: 0n, owedTotal: 0n };
  return { slug: "frogs", module: "0xbb", wrapper: "0xaa", block: 100, balance: 3n * E, owed: Object.values(books).reduce((s, x) => s + x, 0n), reserveUnits: E,
    ethBacked: 4n * E, accepted: 65n * E / 100n, fresh: true, escaped: false, postedAt: NOW_S - 600, lastGoodAt: NOW_S - 600, floorTtl: 3600, books, ...o };
};
const read = (m: Backing): BackingRead => ({ kind: "read", at: NOW_S, modules: [m] });

test("an advance within 48 h is green; past it, red, with the amount and the time; two units in transit are red", () => {
  const open = (at: number) => mod({ transitUnits: E, advanced: 65n * E / 100n, advancedAt: at });
  assert.equal(backingStatus(read(open(NOW_S - 3600)), NOW_S).status, "green");
  const late = backingStatus(read(open(NOW_S - ADVANCE_STALE_S_DEFAULT - 1)), NOW_S);
  assert.equal(late.status, "red");
  assert.match(late.reasons[0], /an advance of 0\.65 ETH has been open since .* \(over 48 h\)/);
  assert.equal(backingStatus(read(open(NOW_S - 7200)), NOW_S, { advanceStaleS: 3600 }).status, "red", "the window is configurable");
  assert.match(backingStatus(read(mod({ transitUnits: 2n * E, advancedAt: NOW_S })), NOW_S).reasons[0], /one advance at a time/);
  assert.equal(backingStatus(read(mod()), NOW_S).status, "green", "a module built before the advance reads as none");
});
