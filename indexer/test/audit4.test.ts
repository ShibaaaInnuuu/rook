import assert from "node:assert/strict";
import { test } from "node:test";
import { readFloorModule } from "../src/wrapped/floorModule.js";
import { buildWorld, FROGS, NOW } from "./wrappedWorld.js";

/**
 * R4-check A4-I3 (docs/audit/2026-09-26/review4/fixes-golive-lows.md §8): for up to an hour after a pause the last post is still
 * fresh, and the site derived "keeper down" from `burnsOpen && !fresh && !escaped`, so it said the module trades. The module read
 * now carries `keeperPaused` from the registry the module obeys (paused, or no keeper named), at the report's block.
 */
const REG = "0x1000000000000000000000000000000000000a11";
const KEEPER = "0x1000000000000000000000000000000000000b22";
const ZERO = "0x0000000000000000000000000000000000000000";

async function readWith(paused: boolean | null, keeper: string | null) {
  const w = buildWorld();
  w.view(FROGS.module, "registry", REG);
  if (paused !== null) w.view(REG, "keeperPaused", paused);
  if (keeper !== null) w.view(REG, "keeper", keeper);
  const m = await readFloorModule(w.rpc, FROGS.module as `0x${string}`, FROGS.wrapper as `0x${string}`, w.head, NOW);
  assert.equal(m.kind, "read");
  return m.kind === "read" ? m : null!;
}

test("audit4 A4-I3: the module read says whether the keeper is down at its block: paused, none named, up, or not read", async () => {
  const paused = await readWith(true, ZERO);
  assert.equal(paused.floorFresh, true, "the fixture's last post is still fresh (the reviewer's case: within the hour after a pause)");
  assert.equal(paused.keeperPaused, true, "paused (the registry's keeper() is zero while paused)");
  assert.equal((await readWith(false, ZERO)).keeperPaused, true, "no keeper named");
  assert.equal((await readWith(false, KEEPER)).keeperPaused, false, "a keeper, not paused");
  assert.equal((await readWith(null, null)).keeperPaused, null, "a registry that cannot be read: not read, never a guess");
});
