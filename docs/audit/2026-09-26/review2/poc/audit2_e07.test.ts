import assert from "node:assert/strict";
import { test } from "node:test";
import type { Abi } from "viem";
import { requestRefusal, stackFrom, type StackFile, type StackReader } from "../src/rook/phase.js";
import { ETCH_ROUTER, NFT_WRAPPER_REDEEM } from "../src/lib/abisV3.js";
import { FROG_CURVE_P1 } from "../src/rook/abis.js";

/**
 * Second review (R2) of E-07. PoCs assert the behaviour found (they pass while it is present).
 * The guard binds a trade to "a coin/curve the factory launched", not to the coin the page (its URL, #/coin/<address>) is about.
 */
const A = (n: number) => `0x${n.toString(16).padStart(40, "0")}` as `0x${string}`;
const STACK = { chainId: 4663, label: "stack", stage: 0, registry: A(1), owner: A(2), keeper: A(3), treasury: A(4),
  collections: [{ slug: "frogs", kind: "inscriptions", wrapper: A(0x10), factory: A(0x11), router: A(0x12), floorModule: A(0x13), raffleVault: A(0x16) }] };
const s = stackFrom([STACK]).stack as StackFile;
const PAGE_COIN = A(0xc0), PAGE_CURVE = A(0xcc), EVIL_COIN = A(0xe0), EVIL_CURVE = A(0xee), ME = A(0x3e);
// both coins were launched by the stack's factory (launching is permissionless and nearly free)
const curveOf: Record<string, string> = { [PAGE_COIN]: PAGE_CURVE, [EVIL_COIN]: EVIL_CURVE };
const tokenOf: Record<string, string> = { [PAGE_CURVE]: PAGE_COIN, [EVIL_CURVE]: EVIL_COIN };
const read: StackReader = {
  routerOf: async (f, coin) => (f.toLowerCase() === A(0x11) ? curveOf[coin.toLowerCase()] ?? A(0) : A(0)),
  tokenOf: async (a) => tokenOf[a.toLowerCase()] ?? null,
};
const req = (address: string, abi: readonly unknown[], functionName: string, args: readonly unknown[]) => ({ address, abi: abi as Abi, functionName, args });

test("R2 NEW (Low): a hostile indexer can swap the curve/coin a coin page trades; requestRefusal allows any launched curve or coin", async () => {
  const dl = 2_000_000_000n;
  // the page is #/coin/<PAGE_COIN>; the indexer answers coin.curve = EVIL_CURVE (Ticket.tsx:93 sends to coin.curve)
  assert.equal(await requestRefusal(s, req(EVIL_CURVE, FROG_CURVE_P1, "buy", [1n, dl]), ME, read), null);
  // graduated: coin.address comes from the indexer's coinsList too (Ticket.tsx:94); any launched coin passes router.buy.coin
  assert.equal(await requestRefusal(s, req(A(0x12), ETCH_ROUTER, "buy", [EVIL_COIN, 1n, ME, dl]), ME, read), null);
});

test("R2 Info: an indexer that calls an inscriptions collection 'nft' turns Redeem into redeem(address) and the guard refuses it (DoS only)", async () => {
  const why = await requestRefusal(s, req(A(0x10), NFT_WRAPPER_REDEEM, "redeem", [ME]), ME, read);
  assert.match(String(why), /no rule for/);
});
