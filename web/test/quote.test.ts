import assert from "node:assert/strict";
import { test } from "node:test";
import { aggregate, chooseRoute, curveBuy, curveSell, poolLeg, quoteCoin, type CurveState, type Leg, type Venues } from "../src/lib/quote.js";
import { burnLeg, floorVenue, mintLeg, ROUTER_USES_FLOOR_MODULE, stateFrom } from "../src/lib/floorModule.js";

/** docs/v5/ux-and-reserves.md §1: the one number, every fee and spread in it, and the route the ticket picks. */
const E = 10n ** 18n;
// the 5 ETH curve of docs/v3/curve.md, 1.2 ETH in: V0 = 2.790 ETH, T0 = 1.246e9, ends at 7.790 ETH / 4.464e8
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

test("the curve's buy is FrogCurve.buy to the wei: 1% of what is sent, rounding against the buyer", () => {
  const leg = curveBuy(curve, E / 10n);
  const fee = (E / 10n) / 100n, net = E / 10n - fee, k = curve.v * curve.t;
  assert.equal(leg.amountOut, curve.t - (k + curve.v + net - 1n) / (curve.v + net));
  assert.equal(leg.amountIn, E / 10n); assert.equal(leg.change, 0n);
  const q = aggregate("buy", { id: "curve", legs: [leg], executable: true }, 100);
  assert.equal(q.feeLines.length, 1); assert.equal(q.feeLines[0].bps, 100); assert.equal(q.feeLines[0].eth, fee, "the fee in ETH is exactly what the curve keeps");
  // a tiny buy costs the fee and almost nothing else; a big one costs the fee plus the price it moved
  near(aggregate("buy", { id: "curve", legs: [curveBuy(curve, 10n ** 12n)], executable: true }, 100).totalCostBps, 100, 0.5, "tiny buy");
  assert.ok(q.totalCostBps > 100 && q.impactBps > 0, "a 0.1 ETH buy moves the curve");
  near(q.feesBps + q.impactBps, q.totalCostBps, 1e-9, "fees + impact = total");
  assert.equal(q.minOut, (q.amountOut * 9_900n) / 10_000n, "1% slippage");
});

test("the buy that reaches the raise uses only what the raise needs, and says what comes back", () => {
  const leg = curveBuy(curve, 10n * E);
  assert.equal(leg.amountOut, curve.t - curve.tEnd, "everything left on the curve");
  const net = curve.vEnd - curve.v;
  assert.equal(leg.amountIn, (net * 10_000n + 9_899n) / 9_900n, "gross = net / 0.99, rounded up, as the contract does");
  assert.equal(leg.change, 10n * E - leg.amountIn);
});

test("the curve's sell: the 1% comes off the ETH out", () => {
  const leg = curveSell(curve, 1_000_000n * E);
  const gross = curve.v - (curve.v * curve.t + curve.t + 1_000_000n * E - 1n) / (curve.t + 1_000_000n * E);
  assert.equal(leg.amountOut, gross - gross / 100n);
});

test("after graduation every hop's fees are in the one number: two LP fees and two Uniswap protocol fees", async () => {
  const r = await quoteCoin("buy", E / 10n, venues(), 100);
  assert.ok(r.ok); const q = r.quote;
  assert.equal(q.route.id, "pool"); assert.deepEqual(q.route.legs.map((l) => l.venue), ["sharedPool", "coinPool"]);
  assert.deepEqual(q.feeLines.map((f) => f.label), ["wX pool: liquidity providers' fee", "wX pool: Uniswap protocol fee", "coin pool: liquidity providers' fee", "coin pool: Uniswap protocol fee"]);
  near(q.feeLines[0].bps + q.feeLines[1].bps, 34.985, 1e-9, "0.3% LP after the 0.05% protocol fee");
  near(q.feeLines[2].bps + q.feeLines[3].bps, 109.9, 1e-9, "1% LP after the 0.1% protocol fee: the 1.099% docs/v3/pools.md measured");
  near(q.feesBps, (1 - (1 - 0.0034985) * (1 - 0.01099)) * 10_000, 1e-6, "composite");
  assert.ok(q.totalCostBps >= q.feesBps, "nothing is cheaper than its own fees");
  near(q.feesBps + q.spreadBps + q.impactBps, q.totalCostBps, 1e-9, "nothing is left out and nothing counted twice");
  const feeEth = q.feeLines.reduce((a, f) => a + f.eth, 0n);
  // each hop's fee on what that hop took in, valued at the mid: the coin pool takes in the wX the first hop gave
  const [l1, l2] = q.route.legs;
  near(Number(feeEth) / 1e18, 0.1 * 0.0034985 + (Number(l2.amountIn) / 1e18 / l1.mid) * 0.01099, 1e-9, "fees in ETH");
  // in very deep pools the price does not move: the total is the fees alone
  const deep = await quoteCoin("buy", E / 10n, venues({}, { ...shared, reserveEth: 33n * 10n ** 30n, reserveWrapped: 51n * 10n ** 30n },
    { ...coinPool, reserveWrapped: 5n * 10n ** 30n, reserveCoin: 2n * 10n ** 38n }), 100);
  assert.ok(deep.ok); near(deep.quote.totalCostBps, deep.quote.feesBps, 0.01, "deep pools: cost = fees"); near(deep.quote.impactBps, 0, 0.01, "no impact");
});

test("a sell after graduation: COIN → wX → ETH, the same fees, valued at the mids", async () => {
  const r = await quoteCoin("sell", 1_000_000n * E, venues(), 50);
  assert.ok(r.ok); const q = r.quote;
  assert.deepEqual(q.route.legs.map((l) => `${l.tokenIn}>${l.tokenOut}`), ["COIN>WX", "WX>ETH"]);
  assert.ok(q.valueInEth > 0n && q.amountOut < q.valueInEth, "you get less ETH than the coins were worth at the mid");
  near(1 - Number(q.amountOut) / Number(q.valueInEth), q.totalCostBps / 10_000, 1e-9, "the one number is exactly that shortfall");
  assert.equal(q.minOut, (q.amountOut * 9_950n) / 10_000n);
});

/** A module state as a fixture carries it: the fills price, each side's spread, and plenty of room unless said. */
const module = (priceWei: bigint, sMint: number, sBurn = sMint, room: Partial<{ mintRoomWei: bigint; burnRoomUnits: bigint }> = {}) =>
  stateFrom({ priceWei, spreadMintBps: sMint, spreadBurnBps: sBurn, mintRoomWei: 100n * E, burnRoomUnits: 100n * E, ...room });

test("the Floor Module: its spread is in the number, the router routes through it, and the cheaper venue wins each way", async () => {
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
  const m = module(646_000_000_000_000_000n, 21, 15, { mintRoomWei: 3n * E, burnRoomUnits: E / 2n });
  assert.equal(m.mintPriceWei, (646_000_000_000_000_000n * 10_021n + 9_999n) / 10_000n, "P_mint = price × (1 + s_mint), rounded up");
  assert.equal(m.burnPriceWei, (646_000_000_000_000_000n * 9_985n) / 10_000n, "P_burn = price × (1 − s_burn), rounded down");
  const mint = mintLeg(m, E)!;
  assert.equal(mint.amountOut, (E * E) / m.mintPriceWei, "units = ethIn × 1e18 / unitPrice");
  near(mint.spreadBps, 21 / 1.0021, 1e-9, "s/(1+s) of the ETH in");
  assert.equal(mintLeg(m, 3n * E + 1n), null, "one wei over mintCapacity: nothing");
  const burn = burnLeg(m, E / 4n)!;
  assert.equal(burn.amountOut, ((E / 4n) * m.burnPriceWei) / E, "eth = units × unitPrice / 1e18"); assert.equal(burn.spreadBps, 15);
  assert.equal(burnLeg(m, E), null, "over burnCapacity: nothing (the router sends the rest to the pool)");
  assert.equal(mintLeg(module(0n, 15), E), null, "no price, no quote");
  assert.equal(mintLeg({ ...m, mintPriceWei: 0n }, E), null, "a stale module (mintCapacity 0, 0) quotes nothing");
});

test("the launch fee (docs/v5/policies-and-fees.md §2) and the tier (§3) are both in the one number, and named in Details", () => {
  const early: CurveState = { ...curve, feeBps: 300, launchExtraBps: 2_500 };
  const leg = curveBuy(early, E / 10n);
  const fee = ((E / 10n) * 2_800n) / 10_000n;
  assert.equal(leg.amountIn - (leg.amountOut > 0n ? 0n : 0n), E / 10n);
  assert.deepEqual(leg.fees.map((f) => [f.label, f.bps]), [["Curve fee (3% tier)", 300], ["Launch fee, decaying to nothing (all of it to the coin's vault)", 2_500]]);
  const q = aggregate("buy", { id: "curve", legs: [leg], executable: true }, 100);
  assert.equal(q.feeLines.reduce((a, f) => a + f.eth, 0n), fee, "the two fees in ETH are exactly what the curve keeps: 28% of what is sent");
  near(q.feesBps, 2_800, 1e-9, "the tier and the launch fee, composed");
  // the last buy is grossed up at the total rate of the moment
  const close = curveBuy(early, 10n * E);
  assert.equal(close.amountIn, ((early.vEnd - early.v) * 10_000n + 7_199n) / 7_200n);
  // decayed: the tier alone, one fee line
  const later = curveBuy({ ...curve, feeBps: 300, launchExtraBps: 0 }, E / 10n);
  assert.deepEqual(later.fees.map((f) => f.label), ["Curve fee (3% tier)"]);
  assert.deepEqual(curveSell({ ...curve, feeBps: 300, launchExtraBps: 2_500 }, E).fees.map((f) => f.bps), [300], "sells never pay the launch fee");
});

test("route choice: the most out among executable routes; ties to the first listed; nothing executable is nothing", () => {
  const leg = (out: bigint): Leg => ({ venue: "sharedPool", tokenIn: "ETH", tokenOut: "WX", amountIn: E, amountOut: out, mid: 1, fees: [], spreadBps: 0, contract: "" });
  const a = { id: "pool" as const, legs: [leg(5n)], executable: true }, b = { id: "floorModule" as const, legs: [leg(5n)], executable: true };
  assert.equal(chooseRoute([a, b]).chosen, a);
  assert.equal(chooseRoute([a, { ...b, legs: [leg(6n)] }]).chosen!.id, "floorModule");
  assert.equal(chooseRoute([{ ...a, executable: false }]).chosen, null);
});

test("the phase decides the venue: the curve while trading, nothing while graduating, refunds when it failed, a direct pool for SWEEP_ONLY", async () => {
  const t = await quoteCoin("buy", E / 10n, { phase: "trading", curve }, 100);
  assert.ok(t.ok && t.quote.route.id === "curve");
  const g = await quoteCoin("buy", E / 10n, { phase: "graduating", curve }, 100);
  assert.ok(!g.ok); assert.match(g.why, /opens on Uniswap when the coin graduates/);
  const f = await quoteCoin("sell", E, { phase: "refunding" }, 100);
  assert.ok(!f.ok); assert.match(f.why, /refund/);
  const direct = await quoteCoin("buy", E / 10n, venues({ sweepOnly: true, coinPool: async (x) => poolLeg({ reserveIn: 10n * E, reserveOut: 10n ** 27n, lpFeePips: 10_000, protocolFeePips: 1_000 }, x, "coinPool", "ETH", "COIN", "coin pool") }), 100);
  assert.ok(direct.ok); assert.equal(direct.quote.route.id, "direct"); assert.equal(direct.quote.route.legs.length, 1);
  assert.ok(!(await quoteCoin("buy", 0n, { phase: "trading", curve }, 100)).ok);
});
