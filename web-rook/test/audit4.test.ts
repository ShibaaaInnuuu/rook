import assert from "node:assert/strict";
import { test } from "node:test";
import { keeperDownOf, moduleStateOf } from "../src/rook/moduleState.js";

/**
 * R4-check A4-I3 (docs/audit/2026-09-26/review4/fixes-golive-lows.md §8): the Books page said "fresh, so the module trades" for up to
 * an hour after a pause (the last post still fresh), while mints were refused and burns paid the pause floor.
 */
const base = { floorFresh: true, escaped: false, burnsOpen: true, exitAtBacking: false, pauseFloorWei: "0", keeperPaused: false as boolean | null | undefined };

test("audit4 A4-I3: a pause within the hour (fresh floor, keeper paused) is 'keeper down', never 'trades'", () => {
  const paused = { ...base, keeperPaused: true, pauseFloorWei: "614010000000000000" };
  assert.equal(moduleStateOf(paused), "keeper-down", "the reviewer's case");
  // the round-4 derivation said: burnsOpen && !fresh && !escaped = false, so "fresh, so the module trades"
  assert.equal(paused.burnsOpen && !paused.floorFresh && !paused.escaped, false, "what round 4 computed");
  assert.equal(moduleStateOf(base), "trades", "fresh with its keeper up: the only state that trades");
  assert.equal(moduleStateOf({ ...base, floorFresh: false, burnsOpen: false }), "stale");
  assert.equal(moduleStateOf({ ...base, floorFresh: false, escaped: true, exitAtBacking: true, keeperPaused: true }), "escaped", "escaped wins: burns at the backing");
});

test("audit4 A4-I3: without the registry's word the module's own pauseFloor() says it; without either, round 4's rule", () => {
  assert.equal(keeperDownOf({ ...base, keeperPaused: null, pauseFloorWei: "614010000000000000" }), true, "pauseFloor() > 0 only while the keeper is down");
  assert.equal(keeperDownOf({ ...base, keeperPaused: null, pauseFloorWei: "0" }), false);
  assert.equal(keeperDownOf({ ...base, keeperPaused: undefined, pauseFloorWei: null, floorFresh: false }), true, "a pre-A4 read: burnsOpen, not fresh, not escaped");
  assert.equal(keeperDownOf({ ...base, keeperPaused: undefined, pauseFloorWei: undefined }), false);
  assert.equal(keeperDownOf({ ...base, keeperPaused: false, pauseFloorWei: "5" }), false, "the registry's word first");
});
