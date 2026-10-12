import assert from "node:assert/strict";
import { test } from "node:test";
import { feeNowBps, launchExtraBps, volumePerItem } from "../src/rook/tiers.js";

/** docs/v5/policies-and-fees.md §2 and §3: the launch wizard's preview and the coin page's countdown. */
const E = 10n ** 18n;
const ethOf = (wei: bigint) => Number(wei) / 1e18;

test("ETH volume per item is the document's table: frogs at 0.158 ETH, 40% vault share — 39.5 / 13.2 / 7.9 / 4.9 ETH at 1 / 3 / 5 / 8%", () => {
  const frog = 158_000_000_000_000_000n;
  assert.ok(Math.abs(ethOf(volumePerItem(frog, 100, 4_000)) - 39.5) < 0.01);
  assert.ok(Math.abs(ethOf(volumePerItem(frog, 300, 4_000)) - 13.17) < 0.01);
  assert.ok(Math.abs(ethOf(volumePerItem(frog, 500, 4_000)) - 7.9) < 0.01);
  assert.ok(Math.abs(ethOf(volumePerItem(frog, 800, 4_000)) - 4.94) < 0.01);
  // apes at 6.85 ETH, 50% share: 1,370 ETH at 1%
  assert.ok(Math.abs(ethOf(volumePerItem(6_850_000_000_000_000_000n, 100, 5_000)) - 1_370) < 0.01);
  assert.equal(volumePerItem(E, 10_000, 10_000), E, "a 100% fee all to the vault: one ETH of volume per ETH of item");
  assert.throws(() => volumePerItem(E, 0));
});

test("the launch fee decays linearly on the slower clock: full at t = 0, half at half, nothing at and after the decay", () => {
  assert.equal(launchExtraBps(5_000, 600, 0), 5_000);
  assert.equal(launchExtraBps(5_000, 600, 300), 2_500);
  assert.equal(launchExtraBps(5_000, 600, 600), 0);
  assert.equal(launchExtraBps(5_000, 600, 6_000), 0);
  assert.equal(launchExtraBps(5_000, 600, -5), 5_000, "before launch is at launch");
  assert.equal(launchExtraBps(0, 600, 0), 0, "disabled");
  assert.equal(feeNowBps(100, 5_000, 600, 60), 100 + 4_500, "a 1% coin one minute in pays 46%");
});
