import assert from "node:assert/strict";
import { test } from "node:test";
import { aggregate, chooseRoute, curveBuy, curveSell, poolLeg, quoteCoin, type CurveState, type Leg, type Venues } from "../src/lib/quote.js";
import { burnLeg, floorVenue, mintLeg, ROUTER_USES_FLOOR_MODULE, stateFrom } from "../src/lib/floorModule.js";

/**
 * Ported from web/test/quote.test.ts: the one number's composition — every fee, every spread, the impact, nothing counted twice —
 * over the copied lib/quote.ts. docs/v5/ux-and-reserves.md §1.
 */
const E = 10n ** 18n;
const V0 = 2_790_000_000_000_000_000n, T0 = 1_246_000_000n * E;
const curve: CurveState = { v: V0 + 1_200_000_000_000_000_000n, t: (V0 * T0 + V0 + 1_200_000_000_000_000_000n - 1n) / (V0 + 1_200_000_000_000_000_000n), vEnd: 7_790_000_000_000_000_000n, tEnd: 446_400_000n * E, feeBps: 100 };
const near = (a: number, b: number, tol: number, what: string) => assert.ok(Math.abs(a - b) <= tol, `${what}: ${a} vs ${b}`);
const shared = { reserveEth: 33n * E, reserveWrapped: 51n * E, lpFeePips: 3_000, protocolFeePips: 500 };
const coinPool = { reserveWrapped: 5_453_000_000_000_000_000n, reserveCoin: 200_000_000n * E, lpFeePips: 10_000, protocolFeePips: 1_000 };
const venues = (over: Partial<Venues> = {}, s = shared, c = coinPool): Venues => ({
  phase: "graduated",
  shared: async (x, dir) => dir === "ethToWx" ? poolLeg({ reserveIn: s.reserveEth, reserveOut: s.reserveWrapped, ...s }, x, "sharedPool", "ETH", "WX", "wX pool")
    : poolLeg({ reserveIn: s.reserveWrapped, reserveOut: s.reserveEth, ...s }, x, "sharedPool", "WX", "ETH", "wX pool"),
  coinPool: async (x, dir) => dir === "toCoin" ? poolLeg({ reserveIn: c.reserveWrapped, reserveOut: c.reserveCoin, ...c }, x, "coinPool", "WX", "COIN", "coin pool")
    : poolLeg({ reserveIn: c.reserveCoin, reserveOut: c.reserveWrapped, ...c }, x, "coinPool", "COIN", "WX", "coin pool"),
  ...over,
});

test("the curve's buy is FrogCurve.buy to the wei; fees + impact = the one number", () => {
  const leg = curveBuy(curve, E / 10n);
  const fee = (E / 10n) / 100n, net = E / 10n - fee, k = curve.v * curve.t;
  assert.equal(leg.amountOut, curve.t - (k + curve.v + net - 1n) / (curve.v + net));
  const q = aggregate("buy", { id: "curve", legs: [leg], executable: true }, 100);
  assert.equal(q.feeLines.length, 1); assert.equal(q.feeLines[0].bps, 100); assert.equal(q.feeLines[0].eth, fee);
  near(aggregate("buy", { id: "curve", legs: [curveBuy(curve, 10n ** 12n)], executable: true }, 100).totalCostBps, 100, 0.5, "tiny buy costs the fee alone");
  assert.ok(q.totalCostBps > 100 && q.impactBps > 0);
  near(q.feesBps + q.impactBps, q.totalCostBps, 1e-9, "fees + impact = total");
  assert.equal(q.minOut, (q.amountOut * 9_900n) / 10_000n, "1% slippage");
});

test("the buy that reaches the raise uses only what the raise needs; the sell's fee comes off the ETH out", () => {
  const leg = curveBuy(curve, 10n * E);
  assert.equal(leg.amountOut, curve.t - curve.tEnd);
  const net = curve.vEnd - curve.v;
  assert.equal(leg.amountIn, (net * 10_000n + 9_899n) / 9_900n);
  assert.equal(leg.change, 10n * E - leg.amountIn);
  const s = curveSell(curve, 1_000_000n * E);
  const gross = curve.v - (curve.v * curve.t + curve.t + 1_000_000n * E - 1n) / (curve.t + 1_000_000n * E);
  assert.equal(s.amountOut, gross - gross / 100n);
});

test("after graduation every hop's fees are in the one number: two LP fees, two protocol fees, then the impact", async () => {
  const r = await quoteCoin("buy", E / 10n, venues(), 100);
  assert.ok(r.ok); const q = r.quote;
  assert.equal(q.route.id, "pool"); assert.deepEqual(q.route.legs.map((l) => l.venue), ["sharedPool", "coinPool"]);
  near(q.feeLines[0].bps + q.feeLines[1].bps, 34.985, 1e-9, "0.3% LP after the 0.05% protocol fee");
  near(q.feeLines[2].bps + q.feeLines[3].bps, 109.9, 1e-9, "1% LP after the 0.1% protocol fee");
  near(q.feesBps, (1 - (1 - 0.0034985) * (1 - 0.01099)) * 10_000, 1e-6, "composite");
  assert.ok(q.totalCostBps >= q.feesBps, "nothing is cheaper than its own fees");
  near(q.feesBps + q.spreadBps + q.impactBps, q.totalCostBps, 1e-9, "nothing left out, nothing counted twice");
  const [l1, l2] = q.route.legs;
  near(Number(q.feeLines.reduce((a, f) => a + f.eth, 0n)) / 1e18, 0.1 * 0.0034985 + (Number(l2.amountIn) / 1e18 / l1.mid) * 0.01099, 1e-9, "fees in ETH at the mids");
  const deep = await quoteCoin("buy", E / 10n, venues({}, { ...shared, reserveEth: 33n * 10n ** 30n, reserveWrapped: 51n * 10n ** 30n }, { ...coinPool, reserveWrapped: 5n * 10n ** 30n, reserveCoin: 2n * 10n ** 38n }), 100);
  assert.ok(deep.ok); near(deep.quote.totalCostBps, deep.quote.feesBps, 0.01, "deep pools: cost = fees");
});

test("a sell after graduation: the one number is exactly the shortfall against the mids", async () => {
  const r = await quoteCoin("sell", 1_000_000n * E, venues(), 50);
  assert.ok(r.ok); const q = r.quote;
  assert.deepEqual(q.route.legs.map((l) => `${l.tokenIn}>${l.tokenOut}`), ["COIN>WX", "WX>ETH"]);
  near(1 - Number(q.amountOut) / Number(q.valueInEth), q.totalCostBps / 10_000, 1e-9, "the shortfall is the number");
  assert.equal(q.minOut, (q.amountOut * 9_950n) / 10_000n);
});

/** A module state as a fixture carries it: the fills price, each side's spread, and plenty of room unless said. */
const module = (priceWei: bigint, sMint: number, sBurn = sMint, room: Partial<{ mintRoomWei: bigint; burnRoomUnits: bigint }> = {}) =>
  stateFrom({ priceWei, spreadMintBps: sMint, spreadBurnBps: sBurn, mintRoomWei: 100n * E, burnRoomUnits: 100n * E, ...room });

test("the Floor Module: its own spreads are in the number, the router routes through it, and the cheaper venue wins each way", async () => {
  assert.equal(ROUTER_USES_FLOOR_MODULE, true, "EtchRouter as built takes the module and fills the pool up to its price, then the module");
  const floor = module(600_000_000_000_000_000n, 100);     // cheaper than the pool's 0.647
  const r = await quoteCoin("buy", E, venues({ floor: floorVenue(floor) }), 100);
  assert.ok(r.ok); assert.equal(r.quote.route.id, "floorModule", "the module, because it gives more and can be sent");
  assert.ok(r.quote.spreadBps > 0 && r.quote.feeLines.some((f) => f.label === "Floor Module spread"), "the spread is listed");
  near(r.quote.feesBps + r.quote.spreadBps + r.quote.impactBps, r.quote.totalCostBps, 1e-9, "and in the total");
  const pool = r.quote.alternatives.find((x) => x.id === "pool")!;
  assert.ok(pool.executable && pool.legs[1].amountOut < r.quote.amountOut, "the pool is quoted beside it and gives less");
  // a router built without the module: the module's quote is shown, never chosen
  const shown = await quoteCoin("buy", E, venues({ floor: floorVenue(floor, false) }), 100);
  assert.ok(shown.ok); assert.equal(shown.quote.route.id, "pool");
  const alt = shown.quote.alternatives.find((x) => x.id === "floorModule")!;
  assert.equal(alt.executable, false); assert.match(alt.why!, /built without the Floor Module/);
  // a module dearer than the pool loses even when routable; one with no room for the amount is not quoted at all
  const dear = await quoteCoin("buy", E, venues({ floor: floorVenue(module(800_000_000_000_000_000n, 100)) }), 100);
  assert.ok(dear.ok); assert.equal(dear.quote.route.id, "pool");
  const full = await quoteCoin("buy", E, venues({ floor: floorVenue(module(600_000_000_000_000_000n, 100, 100, { mintRoomWei: E / 2n })) }), 100);
  assert.ok(full.ok); assert.equal(full.quote.route.id, "pool"); assert.ok(!full.quote.alternatives.some((x) => x.id === "floorModule"), "over the room: no module leg");
  // selling: burn at price − burn spread against the pool's bid
  const sell = await quoteCoin("sell", 10_000_000n * E, venues({ floor: floorVenue(module(700_000_000_000_000_000n, 65, 50)) }), 100);
  assert.ok(sell.ok); assert.equal(sell.quote.route.id, "floorModule"); assert.equal(sell.quote.route.legs[1].venue, "floorBurn");
  assert.equal(sell.quote.route.legs[1].spreadBps, 50, "the burn side's own spread, without the stress term");
});

test("the module's legs are FloorModule.quoteMint / quoteBurn: the unit price rounded against the trader, the room a hard edge", () => {
  const m = module(646_000_000_000_000_000n, 150, 120, { mintRoomWei: 2n * E, burnRoomUnits: E / 2n });
  assert.equal(m.mintPriceWei, (m.priceWei * 10_150n + 9_999n) / 10_000n, "rounded up"); assert.equal(m.burnPriceWei, (m.priceWei * 9_880n) / 10_000n, "rounded down");
  assert.equal(mintLeg(m, E)!.amountOut, (E * E) / m.mintPriceWei);
  assert.equal(burnLeg(m, E / 4n)!.amountOut, ((E / 4n) * m.burnPriceWei) / E);
  assert.equal(mintLeg(m, 2n * E + 1n), null, "one wei over the mint room"); assert.equal(burnLeg(m, E), null, "over the burn room");
  assert.equal(mintLeg(module(0n, 1), E), null, "no price, no quote");
  near(mintLeg(m, E)!.spreadBps, (150 / 10_150) * 10_000, 1e-9, "the mint spread as a share of what goes in");
});

test("route choice and phases: most out among executable routes; curve while trading; nothing while graduating; refunds when failed", async () => {
  const leg = (out: bigint): Leg => ({ venue: "sharedPool", tokenIn: "ETH", tokenOut: "WX", amountIn: E, amountOut: out, mid: 1, fees: [], spreadBps: 0, contract: "" });
  const a = { id: "pool" as const, legs: [leg(5n)], executable: true }, b = { id: "floorModule" as const, legs: [leg(5n)], executable: true };
  assert.equal(chooseRoute([a, b]).chosen, a);
  assert.equal(chooseRoute([a, { ...b, legs: [leg(6n)] }]).chosen!.id, "floorModule");
  assert.equal(chooseRoute([{ ...a, executable: false }]).chosen, null);
  const t = await quoteCoin("buy", E / 10n, { phase: "trading", curve }, 100);
  assert.ok(t.ok && t.quote.route.id === "curve");
  assert.ok(!(await quoteCoin("buy", E / 10n, { phase: "graduating", curve }, 100)).ok);
  const f = await quoteCoin("sell", E, { phase: "refunding" }, 100);
  assert.ok(!f.ok); assert.match(f.why, /refund/);
});
