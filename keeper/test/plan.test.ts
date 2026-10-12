import assert from "node:assert/strict";
import { test } from "node:test";
import type { ListingState } from "../src/chain.js";
import { POLICY, plan, weiFor } from "../src/plan.js";
import { FixtureQuoter } from "../src/sources.js";

const ETH = 10n ** 18n, NOW = 1_800_000_000;
const base: ListingState = { id: 0, kind: "inscriptions", name: "Test", ref: "test-collection", divisibility: 0, vault: "0xv", paused: false,
  spendableWei: 1n * ETH, capWei: 0n, capExpiry: 0, open: 0, blockedBy: null };
const quoter = new FixtureQuoter(3_200_000n, 1_500n, 60n);     // 0.032 BTC an ETH, a fixed 1,500 sats and 0.6% — the shape Phase 0 measured
const price = { sats: 250_000n, ref: "fixture:test-collection" };

test("a paused listing, a vault blocked by an overdue receipt, and a full set of open purchases each get no declaration, and say which", async () => {
  const q = await quoter.quote(ETH);
  assert.deepEqual(plan({ ...base, paused: true }, price, q, NOW, null).map((a) => [a.type, a.rule]), [["skip", "paused"]]);
  assert.deepEqual(plan({ ...base, blockedBy: 4 }, price, q, NOW, null).map((a) => [a.type, a.rule]), [["skip", "blocked"]]);
  const full = plan({ ...base, open: 3 }, price, q, NOW, null);
  assert.deepEqual(full.map((a) => a.type), ["postCap", "skip"]);
  assert.equal(full[1].rule, "max-open");
});

test("the cap is the floor through the bridge plus 5%, so the bridge's cost is inside it; a fresh close cap is kept, a stale or moved one re-posted", async () => {
  const q = await quoter.quote(ETH);
  const item = weiFor(price.sats, q);
  assert.ok(item > (price.sats * ETH) / 3_200_000n, "dearer than the no-cost rate: the route's cost is in the number");
  const first = plan(base, price, q, NOW, null)[0];
  assert.deepEqual([first.type, first.type === "postCap" && first.capWei], ["postCap", String(item + (item * 500n) / 10_000n)]);
  const posted = BigInt((first as { capWei: string }).capWei);
  assert.equal(plan({ ...base, capWei: posted, capExpiry: NOW + 3_000 }, price, q, NOW, null)[0].type, "keepCap");
  assert.equal(plan({ ...base, capWei: posted, capExpiry: NOW + 600 }, price, q, NOW, null)[0].type, "postCap", "under 15 minutes left");
  assert.equal(plan({ ...base, capWei: (posted * 103n) / 100n, capExpiry: NOW + 3_000 }, price, q, NOW, null)[0].type, "postCap", "3% away");
});

test("a declaration never exceeds the balance or cap × quantity, never more than three inscriptions, and nothing under 0.05 ETH", async () => {
  const q = await quoter.quote(ETH);
  const acts = plan(base, price, q, NOW, null);
  const d = acts.find((a) => a.type === "declare") as { maxEth: string; quantity: string; capRef: string };
  const cap = BigInt((acts[0] as { capWei: string }).capWei);
  assert.equal(d.quantity, "3");
  assert.ok(BigInt(d.maxEth) <= base.spendableWei && BigInt(d.maxEth) === cap * 3n);
  assert.equal(d.capRef, "fixture:test-collection", "whose word the cap was");
  const small = plan({ ...base, spendableWei: 4n * 10n ** 16n }, price, q, NOW, null);
  assert.equal(small.at(-1)!.rule, "min-declaration");
  const oneShort = plan({ ...base, spendableWei: 6n * 10n ** 16n }, { ...price, sats: 5_000_000n }, await quoter.quote(6n * 10n ** 16n), NOW, null);
  assert.equal(oneShort.at(-1)!.rule, "under-one-item");
  const buy = acts.at(-1) as { type: string; expectSats: string; bridgeWei: string; why: string };
  assert.deepEqual([buy.type, buy.expectSats], ["wouldBuy", "750000"]);
  assert.match(buy.why, /never attempted/);
});

test("a rune: the cap is per WHOLE unit and the quantity is raw units, exactly as the vault's bound computes it", async () => {
  const rune: ListingState = { ...base, kind: "rune", ref: "840000:1", divisibility: 2, spendableWei: ETH / 10n };
  const q = await quoter.quote(ETH / 10n);
  const acts = plan(rune, { sats: 12n, ref: "fixture:840000:1" }, q, NOW, null);
  const cap = BigInt((acts[0] as { capWei: string }).capWei);
  const d = acts.find((a) => a.type === "declare") as { maxEth: string; quantity: string };
  const quantity = BigInt(d.quantity);
  assert.equal(quantity, (rune.spendableWei * 100n) / cap);
  assert.ok(BigInt(d.maxEth) <= (cap * quantity) / 100n, "within AssetVault.boundFor(quantity) = cap × quantity ÷ 10^divisibility");
  assert.ok(BigInt(d.maxEth) <= rune.spendableWei);
});

test("the policy's numbers are the documented ones", () => {
  assert.deepEqual([POLICY.capMarginBps, POLICY.repostIfUnderS, POLICY.repostIfMovedBps, POLICY.minDeclarationWei, POLICY.maxInscriptions, POLICY.maxOpen],
    [500n, 900, 200n, 5n * 10n ** 16n, 3n, 3]);
});
