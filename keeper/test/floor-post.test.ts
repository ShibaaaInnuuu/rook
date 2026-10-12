import assert from "node:assert/strict";
import { test } from "node:test";
import { decodeFunctionData, encodeAbiParameters, keccak256, toHex, toFunctionSelector, type Hex } from "viem";
import type { Floor } from "../src/floor.js";
import { FLOOR_MODULE_ABI, POST_POLICY, band, planPost, postCalldata, readPosted, type Posted } from "../src/floorpost.js";
import { postFloors } from "../staging/floorfeed.js";
import { STAGING_CHAIN_ID } from "../staging/guard.js";

// docs/v5/floor-feed.md §5. The assumed Floor Module interface, the stepping rule, and the staging poster against a fake node.
const E18 = 10n ** 18n;
const T = 1_790_000_000;
const floorOf = (price: bigint | null, confidenceBps = 8_000, depthItems = 4): Floor =>
  ({ price, listingFloor: price, saleFloor: null, confidenceBps, depthItems, upCost10: null, used: [], salesUsed: [], excluded: [] });
const posted = (priceWei: bigint, postedAt = T - 3_600, o: Partial<Posted> = {}): Posted => ({ priceWei, postedAt, ttl: 3_600, maxMoveBps: 200, bounds: null, ...o });

test("the assumed interface: every selector is the keccak of its signature (a rename changes one line)", () => {
  for (const sig of ["floor()", "maxMoveBps()", "floorBounds()", "keeper()", "postFloor(uint256,uint16,uint32)"]) {
    const name = sig.split("(")[0];
    const item = FLOOR_MODULE_ABI.find((x) => x.type === "function" && x.name === name)!;
    assert.equal(toFunctionSelector(item), keccak256(toHex(sig)).slice(0, 10), sig);
  }
  const data = postCalldata(6n * E18, 12_345, 7);
  const d = decodeFunctionData({ abi: FLOOR_MODULE_ABI, data });
  assert.deepEqual(d.args, [6n * E18, 10_000, 7], "confidence is clamped to 10,000 bps");
});

test("§5.2 the band: pro rata to the time since the last post, capped at an hour, and inside the chain's own bounds", () => {
  assert.deepEqual(band(posted(100n * E18, T - 1_800), T), { lo: 99n * E18, hi: 101n * E18 }, "half an hour: half of 2%");
  assert.deepEqual(band(posted(100n * E18, T - 36_000), T), { lo: 98n * E18, hi: 102n * E18 }, "ten hours: still one hour's move");
  assert.deepEqual(band(posted(100n * E18, T - 3_600, { bounds: { lo: 99n * E18, hi: 105n * E18 } }), T), { lo: 99n * E18, hi: 102n * E18 });
  assert.equal(band(posted(0n), T), null, "nothing posted: no band");
  assert.deepEqual(band(posted(100n * E18, T + 600), T), { lo: 100n * E18, hi: 100n * E18 }, "a post stamped ahead of our clock allows no move yet");
});

test("§5.2 skip, keep, post: no floor, low confidence, within 1%, the TTL refresh, the first post", () => {
  assert.equal(planPost(floorOf(null), posted(E18), T).type, "skip");
  const low = planPost(floorOf(E18, 2_999), posted(E18), T);
  assert.equal(low.type, "skip");
  assert.match((low as { why: string }).why, /confidence 2999/);
  assert.equal(planPost(floorOf(E18 + E18 / 200n), posted(E18, T - 600), T).type, "keep", "0.5% move, 50 min of TTL left");
  const refresh = planPost(floorOf(E18), posted(E18, T - 3_000), T);
  assert.equal(refresh.type, "post", "10 min of TTL left: re-posted even unchanged");
  assert.match((refresh as { why: string }).why, /refreshed/);
  const first = planPost(floorOf(7n * E18, 9_000, 3), posted(0n, 0, { ttl: 0 }), T);
  assert.equal(first.type, "post");
  assert.equal((first as { priceWei: string }).priceWei, String(7n * E18));
  assert.deepEqual(decodeFunctionData({ abi: FLOOR_MODULE_ABI, data: (first as { calldata: Hex }).calldata }).args, [7n * E18, 9_000, 3]);
});

test("§5.2 stepping: a 30% move is walked at the chain's pace; every post is inside the pro-rata rule, and inside a rolling-hour chain's floorBounds()", () => {
  const m = 200n, target = 130n * E18;
  // chain A: no floorBounds(); the rule is pro rata to the time since the last post (the reading §5.1 assumes)
  let price = 100n * E18, at = T, now = T;
  for (let i = 0; i < 400 && price !== target; i++) {
    now += 600;
    const a = planPost(floorOf(target), { priceWei: price, postedAt: at, ttl: 3_600, maxMoveBps: 200, bounds: null }, now);
    if (a.type !== "post") continue;
    const next = BigInt(a.priceWei);
    assert.ok(next - price <= (price * m * BigInt(Math.min(now - at, 3_600))) / (10_000n * 3_600n), `pro rata rule at step ${i}`);
    assert.ok(a.stepped || next === target);
    price = next; at = now;
  }
  assert.equal(price, target, "it gets there");
  const hours = (now - T) / 3_600;
  assert.ok(hours >= 13 && hours <= 15, `ln(1.3)/ln(1.02) ≈ 13.2 h, took ${hours}`);
  // chain B: a rolling hour against the price an hour ago, exposed as floorBounds(); the keeper stays inside it
  price = 100n * E18; at = T; now = T;
  const history: [number, bigint][] = [[T - 7_200, price]];
  for (let i = 0; i < 400 && price !== target; i++) {
    now += 600;
    const ref = [...history].reverse().find(([t]) => t <= now - 3_600)?.[1] ?? price;
    const bounds = { lo: ref - (ref * m) / 10_000n, hi: ref + (ref * m) / 10_000n };
    const a = planPost(floorOf(target), { priceWei: price, postedAt: at, ttl: 3_600, maxMoveBps: 200, bounds }, now);
    if (a.type !== "post") continue;
    const next = BigInt(a.priceWei);
    assert.ok(next >= bounds.lo && next <= bounds.hi, `rolling rule at step ${i}`);
    price = next; at = now; history.push([now, price]);
  }
  assert.equal(price, target);
});

// ---- reads, and the staging poster ------------------------------------------------------------------------------------------

type Rpc = { method: string; params: unknown[] };
function node(state: { price: bigint; postedAt: number; ttl: number; move: number; bounds?: [bigint, bigint]; keeper: Hex; chainId?: number }) {
  const calls: Rpc[] = [];
  const sel = (sig: string) => keccak256(toHex(sig)).slice(0, 10);
  const f = (async (_url: string, init: RequestInit) => {
    const { method, params } = JSON.parse(String(init.body)) as Rpc;
    calls.push({ method, params });
    const ok = (result: unknown) => ({ json: async () => ({ jsonrpc: "2.0", id: 1, result }) });
    const err = (message: string) => ({ json: async () => ({ jsonrpc: "2.0", id: 1, error: { message } }) });
    if (method === "eth_chainId") return ok(`0x${(state.chainId ?? STAGING_CHAIN_ID).toString(16)}`);
    if (method === "eth_call") {
      const data = String((params[0] as { data: string }).data).slice(0, 10);
      if (data === sel("floor()")) return ok(encodeAbiParameters([{ type: "uint256" }, { type: "uint64" }, { type: "uint64" }], [state.price, BigInt(state.postedAt), BigInt(state.ttl)]));
      if (data === sel("maxMoveBps()")) return ok(encodeAbiParameters([{ type: "uint16" }], [state.move]));
      if (data === sel("floorBounds()")) return state.bounds ? ok(encodeAbiParameters([{ type: "uint256" }, { type: "uint256" }], state.bounds)) : err("execution reverted");
      if (data === sel("keeper()")) return ok(encodeAbiParameters([{ type: "address" }], [state.keeper]));
      return err("unknown call");
    }
    if (method === "eth_sendTransaction") return ok(`0x${"ab".repeat(32)}`);
    if (method === "eth_getTransactionReceipt") return ok({ status: "0x1", transactionHash: `0x${"ab".repeat(32)}`, blockNumber: "0x1", gasUsed: "0x1", logs: [] });
    return err(`unexpected ${method}`);
  }) as unknown as typeof fetch;
  return { f, calls };
}
const MODULE = "0x00000000000000000000000000000000000f100e" as Hex;
const KEEPER = "0x000000000000000000000000000000000000cee9" as Hex;

test("readPosted: eth_call only; a module without floorBounds() reads as no bounds", async () => {
  const { f, calls } = node({ price: 5n * E18, postedAt: T, ttl: 3_600, move: 200, keeper: KEEPER });
  assert.deepEqual(await readPosted("http://x", MODULE, f), { priceWei: 5n * E18, postedAt: T, ttl: 3_600, maxMoveBps: 200, bounds: null });
  assert.ok(calls.every((c) => c.method === "eth_call"));
  const { f: g } = node({ price: 5n * E18, postedAt: T, ttl: 3_600, move: 200, bounds: [4n * E18, 6n * E18], keeper: KEEPER });
  assert.deepEqual((await readPosted("http://x", MODULE, g)).bounds, { lo: 4n * E18, hi: 6n * E18 });
});

test("the staging poster: sends the stepped postFloor as the module's keeper, and refuses anything but the staging chain on this machine", async () => {
  const { f, calls } = node({ price: 100n * E18, postedAt: T - 3_600, ttl: 3_600, move: 200, bounds: [98n * E18, 102n * E18], keeper: KEEPER });
  const env = { KEEPER_MODE: "staging" };
  const r = await postFloors("http://127.0.0.1:8546", [{ module: MODULE, floor: async () => floorOf(110n * E18) }], env, f, () => T);
  assert.equal(r[0].action.type, "post");
  assert.equal((r[0].action as { priceWei: string }).priceWei, String(102n * E18), "stepped to the band's edge, not the target");
  const sent = calls.find((c) => c.method === "eth_sendTransaction")!.params[0] as { from: string; to: string; data: Hex };
  assert.equal(sent.from.toLowerCase(), KEEPER);
  assert.equal(sent.to.toLowerCase(), MODULE);
  assert.deepEqual(decodeFunctionData({ abi: FLOOR_MODULE_ABI, data: sent.data }).args, [102n * E18, 8_000, 4]);
  // a keep sends nothing
  const { f: g, calls: gc } = node({ price: 100n * E18, postedAt: T - 60, ttl: 3_600, move: 200, keeper: KEEPER });
  const k = await postFloors("http://127.0.0.1:8546", [{ module: MODULE, floor: async () => floorOf(100n * E18) }], env, g, () => T);
  assert.equal(k[0].action.type, "keep");
  assert.ok(!gc.some((c) => c.method === "eth_sendTransaction"));
  // refusals: not staging mode, not this machine, a node that says Robinhood Chain
  await assert.rejects(postFloors("http://127.0.0.1:8546", [], {}, f), /KEEPER_MODE/);
  await assert.rejects(postFloors("https://rpc.example.org", [], env, f), /this machine/);
  const { f: h } = node({ price: 0n, postedAt: 0, ttl: 0, move: 200, keeper: KEEPER, chainId: 4663 });
  await assert.rejects(postFloors("http://127.0.0.1:8546", [], env, h), /4663/);
  void POST_POLICY;
});
