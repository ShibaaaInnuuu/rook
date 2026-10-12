// Review round 7, part D: the runner's pending cadence (F6-L-1) at a78a542.
import assert from "node:assert/strict";
import { test } from "node:test";
import { parseRunnerConfig, sleepAfter } from "../src/runner.js";
const C = "ryjl3-tyaaa-aaaaa-aaaba-cai";
test("holds: 0, negative, NaN, fractions, above the cadence are refused", () => {
  for (const x of ["0", "-60", "abc", "59", "120.5", "601", "1e9"]) assert.throws(() => parseRunnerConfig({ CANISTER_ID: C, RUNNER_PENDING_EVERY_S: x }), /RUNNER_PENDING_EVERY_S/, x);
});
test("WEAKNESS (D7-I-3, cosmetic): 0x3c / 6e1 are read as 60, and 301..600 is accepted though the runbook says keep it <= 300 (the 4663 state reach)", () => {
  assert.equal(parseRunnerConfig({ CANISTER_ID: C, RUNNER_PENDING_EVERY_S: "0x3c" }).pendingEveryS, 60);
  assert.equal(parseRunnerConfig({ CANISTER_ID: C, RUNNER_PENDING_EVERY_S: "6e1" }).pendingEveryS, 60);
  const c = parseRunnerConfig({ CANISTER_ID: C, RUNNER_PENDING_EVERY_S: "600" });
  assert.equal(sleepAfter(c, true), 600, "no sooner tick at all");
});
