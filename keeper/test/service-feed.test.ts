import assert from "node:assert/strict";
import { test } from "node:test";
import { readFileSync } from "node:fs";
import { satsToWei } from "../src/floor.js";
import { FixtureRate, OrdinalsWalletBook, type RateSource } from "../src/floorsources.js";
import { OrdFeed, type OrdProvider, type OutputContents, type Read } from "../src/ordapi.js";
import { AgreedRate, KrakenRate, OrdinalsWalletFeed } from "../service/feed.js";
import { planCap, slotAt, slotOpen, PROD_POST_POLICY, type CapState } from "../service/plan.js";

// docs/v6/p1-services.md §1.4: the floor comes from the agreed feed or not at all; the cap and draw rules, pure.
const OW_L = JSON.parse(readFileSync(new URL("../fixtures/floor/ow-bitcoin-frogs-listings-2026-09-23.json", import.meta.url), "utf8"));
const OW_S = JSON.parse(readFileSync(new URL("../fixtures/floor/ow-bitcoin-frogs-sales-2026-09-23.json", import.meta.url), "utf8"));
const byOutpoint: Record<string, string> = Object.fromEntries((OW_L.listings as { outpoint: string; inscription_id: string }[]).map((l) => [l.outpoint, l.inscription_id]));

/** An ordinals provider that answers heights and outputs; `lie` makes it name a different inscription. */
const provider = (name: string, o: { height?: number; lie?: boolean; down?: boolean } = {}): OrdProvider => ({
  name, reads: new Set<Read>(["height", "output"]),
  height: async () => o.height ?? 900_000,
  output: async (op: string): Promise<OutputContents | null> => {
    if (o.down) throw new Error(`${name}: HTTP 503`);
    const id = byOutpoint[op];
    return id ? { inscriptions: [o.lie ? id.replace(/i\d+$/, "i9") : id], runes: [], value: 546 } : null;
  },
  inscription: async () => null, addressInscriptions: async () => [], txConfirmations: async () => null, listings: async () => [],
});
const owBook = () => new OrdinalsWalletBook((async (url: string) => ({ ok: true, status: 200,
  json: async () => (url.endsWith("/escrows") ? OW_L.listings : { sales: OW_S.sales }) })) as unknown as typeof fetch, () => OW_L.fetchedAt * 1000, 0, async () => {});
const feedWith = (ps: OrdProvider[], rate: RateSource = new FixtureRate(3_185_800n)) =>
  new OrdinalsWalletFeed({ book: owBook(), ord: new OrdFeed(ps), rate, slugs: { frogs: "bitcoin-frogs" }, nowS: () => OW_L.fetchedAt });

test("the recorded frog book: with two ordinals providers agreeing on every listed output, the measured floor in wei (docs/v5/floor-feed.md §9)", async () => {
  const r = await feedWith([provider("unisat"), provider("ord")]).floor("frogs");
  assert.ok(r.floor, r.why);
  // the E-01 rules (S-item-repeat, the 5% single-marketplace pull cap): 536,999 sats, not the 503,683 measured before them
  assert.equal(r.floor!.price, satsToWei(536_999n, 3_185_800n));
  assert.equal(r.floor!.confidenceBps, 5_204);
  assert.equal(r.ref, "ordinalswallet:bitcoin-frogs");
});

test("never a floor the feed does not agree on: one provider (no UniSat key), a provider down, providers disagreeing, heights apart", async () => {
  for (const [label, ps] of [["one provider", [provider("ord")]], ["one down", [provider("unisat", { down: true }), provider("ord")]],
    ["disagree", [provider("unisat", { lie: true }), provider("ord")]], ["heights apart", [provider("unisat", { height: 900_002 }), provider("ord")]]] as const) {
    const r = await feedWith([...ps]).floor("frogs");
    assert.equal(r.floor, null, label);
    assert.match(r.why, /fewer than 3 valid listings .* two ordinals providers did not agree/, label);
  }
});

test("the rate: two keyless references must agree (within 1% by default); the higher sats/ETH (the LOWER floor in wei, E-09) is used; one down or apart is no rate", async () => {
  assert.equal(await new AgreedRate([new FixtureRate(3_185_800n), new FixtureRate(3_200_000n)], 100n).satsPerEth(), 3_200_000n);
  assert.equal(await new AgreedRate([new FixtureRate(3_200_000n), new FixtureRate(3_185_800n)], 100n).satsPerEth(), 3_200_000n, "whatever the order");
  // E-09 inverted: at 480,000 sats the picked rate gives the lower of the two wei floors
  const picked = await new AgreedRate([new FixtureRate(3_700_000n), new FixtureRate(3_665_000n)], 100n).satsPerEth();
  assert.ok(satsToWei(480_000n, picked) < satsToWei(480_000n, 3_665_000n), "the conservative floor: mints, caps and maxItemCost rest on the lower wei price");
  await assert.rejects(new AgreedRate([new FixtureRate(3_185_800n), new FixtureRate(3_300_000n)], 100n).satsPerEth(), /differ by more than 100 bps/);
  await assert.rejects(new AgreedRate([new FixtureRate(3_185_800n)], 100n).satsPerEth(), /two references must agree/);
  const r = await feedWith([provider("unisat"), provider("ord")], new AgreedRate([new FixtureRate(3_185_800n), new FixtureRate(3_300_000n)], 100n)).floor("frogs");
  assert.equal(r.floor, null);
  assert.match(r.why, /no rate/);
  // Kraken's ticker, as sats per ETH; an answer it does not understand is an error
  const k = (body: unknown) => new KrakenRate((async () => ({ ok: true, status: 200, json: async () => body })) as unknown as typeof fetch);
  assert.equal(await k({ error: [], result: { XETHXXBT: { c: ["0.031858", "1.0"] } } }).satsPerEth(), 3_185_800n);
  await assert.rejects(k({ error: ["EQuery:Unknown asset pair"] }).satsPerEth(), /kraken/);
  await assert.rejects(k({ error: [], result: { X: { c: ["lots"] } } }).satsPerEth(), /invalid rate/);
});

test("the production floor policy: refresh with 30 minutes of the hour left, re-post on a move over 0.5% (floor-module.md §5)", () => {
  assert.equal(PROD_POST_POLICY.refreshBeforeExpiryS, 1_800);
  assert.equal(PROD_POST_POLICY.repostMovedBps, 50n);
  assert.equal(PROD_POST_POLICY.minConfidenceBps, 3_000);
});

const E18 = 10n ** 18n;
const cs = (o: Partial<CapState> = {}): CapState => ({ cap: 0n, capAt: 0, capAtBlock: 0, fresh: false, ceiling: 100n * E18, parentBlockSeconds: 12, ...o });
const T = 1_790_000_000, B = T / 12;

test("caps (H12, C-01): floor + margin; stepped inside ±20% of a fresh cap; one post per 10 minutes on both clocks; never over the ceiling; no ceiling or no floor, no cap", () => {
  assert.equal(planCap(null, cs(), T, B, 1_000n).type, "skip");
  const first = planCap(E18, cs(), T, B, 1_000n);
  assert.deepEqual([first.type, (first as { capWei: string }).capWei], ["post", String(11n * E18 / 10n)]);
  // a fresh cap of 1.1: a target of 2.2 is walked in a 20% step
  const step = planCap(2n * E18, cs({ cap: 11n * E18 / 10n, capAt: T - 700, capAtBlock: B - 60, fresh: true }), T, B, 1_000n);
  assert.deepEqual([step.type, (step as { capWei: string }).capWei], ["post", String((11n * E18 / 10n) * 12n / 10n)]);
  // the gap: 9 minutes on the clock, or 9 minutes of parent blocks, is too soon
  assert.equal(planCap(2n * E18, cs({ cap: E18, capAt: T - 540, capAtBlock: B - 60, fresh: true }), T, B, 0n).type, "keep");
  assert.equal(planCap(2n * E18, cs({ cap: E18, capAt: T - 700, capAtBlock: B - 45, fresh: true }), T, B, 0n).type, "keep");
  // within 2% and more than 15 minutes left: keep; under 15 minutes: refresh
  assert.equal(planCap(E18, cs({ cap: E18 + E18 / 100n, capAt: T - 1_000, capAtBlock: B - 100, fresh: true }), T, B, 0n).type, "keep");
  assert.equal(planCap(E18, cs({ cap: E18, capAt: T - 2_800, capAtBlock: B - 240, fresh: true }), T, B, 0n).type, "post");
  // the ceiling holds the target, stale or fresh
  const ceil = planCap(2n * E18, cs({ ceiling: 15n * E18 / 10n }), T, B, 1_000n);
  assert.equal((ceil as { capWei: string }).capWei, String(15n * E18 / 10n));
  // C-01: a stale cap rises at most 20% per post too (the vault's rule since the fixes); it may fall any distance
  const stale = planCap(5n * E18, cs({ cap: E18, capAt: T - 4_000, fresh: false }), T, B, 0n);
  assert.deepEqual([stale.type, (stale as { capWei: string }).capWei], ["post", String(12n * E18 / 10n)]);
  assert.match(stale.why, /at most 20% above the last cap/);
  const down = planCap(E18 / 10n, cs({ cap: E18, capAt: T - 4_000, fresh: false }), T, B, 0n);
  assert.equal((down as { capWei: string }).capWei, String(E18 / 10n), "lower is safer: a stale re-post falls freely");
  // C-01: no ceiling on the vault means it takes no cap at all (NoCeiling): nothing is planned
  const none = planCap(E18, cs({ ceiling: 0n }), T, B, 1_000n);
  assert.equal(none.type, "skip");
  assert.match(none.why, /no ceiling/);
});

test("draw slots: T_k = k × every + offset on the chain's clock; open under the window on both clocks; past it, skipped", () => {
  const r = { everyS: 86_400, offsetS: 54_000, windowS: 3_000 };
  // 1,790,000,000 is 2026-09-21 14:13 UTC: the last slot is the day before's 15:00; 47 minutes later it is that day's
  assert.equal(new Date(slotAt(1_790_000_000, r) * 1000).toISOString(), "2026-09-20T15:00:00.000Z");
  assert.equal(new Date(slotAt(1_790_000_000 + 5_400, r) * 1000).toISOString(), "2026-09-21T15:00:00.000Z");
  assert.deepEqual(slotOpen(T - 100, B - 8, T, B, r, 12).open, true);
  assert.deepEqual(slotOpen(T - 3_000, B - 8, T, B, r, 12).open, false);
  assert.deepEqual(slotOpen(T - 100, B - 250, T, B, r, 12).open, false, "the parent-block clock binds too");
  assert.deepEqual(slotOpen(T + 5, B, T, B, r, 12).open, false, "not yet on chain");
});
