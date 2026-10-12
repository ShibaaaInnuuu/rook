import assert from "node:assert/strict";
import { test } from "node:test";
import { volumeUntilNextBuy, type VolumeInputs } from "../src/views.js";

const E = 10n ** 18n, NOW = 1_800_000_000;
const base: VolumeInputs = { kind: "inscriptions", paused: false, blocked: false, capWei: E / 10n, capExpiry: NOW + 600, nowS: NOW, spendableWei: 0n, coinUnspentWei: 0n,
  coinQueued: true, harvestableWei: 0n, taxBps: 100 };

test("docs/phase4.md §3, by hand: a 0.1 ETH cap, nothing spendable, a 1% tax — 0.1 / (0.80 × 1.70%) = 7.3529… ETH of volume", () => {
  const v = volumeUntilNextBuy(base);
  assert.equal(v.reason, null); assert.equal(v.ready, false);
  assert.equal(v.inputs.targetWei, String(E / 10n)); assert.equal(v.inputs.takeBps, 170); assert.equal(v.inputs.depositNeededWei, String(E / 10n));
  // 0.1e18 × 1e8 / (170 × 8000) rounded up
  assert.equal(v.volumeNeededWei, String((E / 10n * 100_000_000n + 1_360_000n - 1n) / 1_360_000n));
  assert.ok(Math.abs(Number(v.volumeNeededWei) / 1e18 - 7.352941) < 1e-5);
});

test("what is already there counts: spendable balance, and 80% of what one harvest() would move", () => {
  const v = volumeUntilNextBuy({ ...base, spendableWei: E / 25n, harvestableWei: E / 40n });   // 0.04 spendable; 0.025 harvestable → 0.02 to the vault
  assert.equal(v.inputs.depositNeededWei, String(E / 10n - E / 25n - E / 50n));               // 0.1 − 0.04 − 0.02 = 0.04
  const ready = volumeUntilNextBuy({ ...base, spendableWei: E / 10n });
  assert.deepEqual([ready.volumeNeededWei, ready.ready, ready.reason], ["0", true, null]);
});

test("the target is never under the keeper's 0.05 ETH minimum declaration; a rune's target IS that minimum whatever its unit cap", () => {
  assert.equal(volumeUntilNextBuy({ ...base, capWei: E / 100n }).inputs.targetWei, String(E / 20n));
  assert.equal(volumeUntilNextBuy({ ...base, kind: "rune", capWei: 5n * E }).inputs.targetWei, String(E / 20n));
});

test("MIN_ENTRY: a coin not yet in the queue has fees WAITING that are not spendable; it needs at least the rest of 0.01 ETH, and its waiting fees then count", () => {
  // the listing is 0.001 ETH short of its target, but this coin is unqueued with 0.004 waiting: 0.001 more would NOT enter the queue
  const v = volumeUntilNextBuy({ ...base, capWei: E / 20n, spendableWei: E / 20n - E / 1000n + 0n - (E / 250n), coinQueued: false, coinUnspentWei: E / 250n });
  // target 0.05, spendable 0.045, waiting 0.004 → to the target: 0.001; to MIN_ENTRY: 0.006. The larger binds.
  assert.equal(v.inputs.coinWaitingWei, String(E / 250n));
  assert.equal(v.inputs.depositNeededWei, String(6n * E / 1000n));
  const far = volumeUntilNextBuy({ ...base, capWei: E, coinQueued: false, coinUnspentWei: E / 250n });
  assert.equal(far.inputs.depositNeededWei, String(E - E / 250n), "far from the target, the waiting fees simply count towards it");
});

test("every reason it refuses to give a number, and that a refusal still shows its inputs", () => {
  assert.equal(volumeUntilNextBuy({ ...base, paused: true }).reason, "listing-paused");
  assert.equal(volumeUntilNextBuy({ ...base, blocked: true }).reason, "listing-blocked");
  assert.equal(volumeUntilNextBuy({ ...base, capExpiry: NOW }).reason, "no-live-cap");
  assert.equal(volumeUntilNextBuy({ ...base, capWei: 0n }).reason, "no-live-cap");
  assert.equal(volumeUntilNextBuy({ ...base, capWei: null, capExpiry: null }).reason, "no-live-cap");
  assert.equal(volumeUntilNextBuy({ ...base, taxBps: null }).reason, "tax-unknown");
  const dead = volumeUntilNextBuy({ ...base, spendableWei: null });
  assert.deepEqual([dead.reason, dead.volumeNeededWei, dead.inputs.capWei, dead.assumptions.length > 3], ["live-unavailable", null, String(E / 10n), true]);
  assert.equal(volumeUntilNextBuy({ ...base, coinQueued: null }).reason, "live-unavailable");
});

test("a higher creator tax shortens it in proportion: 10% tax needs 1.70/10.70 of what 1% needs", () => {
  const one = BigInt(volumeUntilNextBuy(base).volumeNeededWei!), ten = BigInt(volumeUntilNextBuy({ ...base, taxBps: 1000 }).volumeNeededWei!);
  assert.ok(Math.abs(Number(ten) / Number(one) - 170 / 1070) < 1e-9);
});
