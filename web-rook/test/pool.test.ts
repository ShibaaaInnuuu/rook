import assert from "node:assert/strict";
import { test } from "node:test";
import { poolView, targetText } from "../src/rook/pool";

// docs/v7/phase1-nft-pooled.md: the collection page's graduation target and pool.
const E = 10n ** 18n, s = (x: bigint) => String(x);

test("a nest's own graduation target, one figure or early-then-normal", () => {
  assert.equal(targetText({ normalRaiseWei: s(9n * E), earlyRaiseWei: s(9n * E), earlyUntilEthWei: s(20n * E), maxRaiseWei: s(30n * E) }), "9 ETH");
  assert.equal(targetText({ normalRaiseWei: s(5n * E), earlyRaiseWei: s(8n * E), earlyUntilEthWei: s(20n * E), maxRaiseWei: s(30n * E) }),
    "8 ETH while the shared pool is under 20 ETH, then 5 ETH");
  assert.equal(targetText(null), null);
});

test("the pool: pooling below one item plus the buffer, due at it, nothing without a module", () => {
  const g = { normalRaiseWei: s(9n * E), earlyRaiseWei: s(9n * E), earlyUntilEthWei: s(20n * E), maxRaiseWei: s(30n * E) };
  // BAYC at 9 ETH after one graduation: 0.82 units of the 1.05 a buy needs
  const one = poolView({ graduation: g, pool: { units: s(82n * E / 100n), dueAtUnits: s(E + E / 20n) } })!;
  assert.deepEqual([one.units, one.dueAt, one.pct, one.due, one.target], [0.82, 1.05, 78, false, "9 ETH"]);
  assert.match(one.text, /0\.82 of the 1\.05 wrapped tokens a buy needs/);
  assert.match(one.text, /another collection's never count toward it/);
  // the second graduation: due
  const two = poolView({ graduation: g, pool: { units: s(164n * E / 100n), dueAtUnits: s(E + E / 20n) } })!;
  assert.deepEqual([two.pct, two.due], [100, true]);
  assert.match(two.text, /next buy for this collection is due/);
  assert.match(poolView({ graduation: null, pool: { units: "0", dueAtUnits: s(E + E / 20n) } })!.text, /Nothing is pooled/);
  assert.equal(poolView({ graduation: g, pool: null }), null);
  assert.equal(poolView({}), null, "a report written before the field");
});
