import { encodeAbiParameters, keccak256, type PublicClient } from "viem";
import type { WrappedCollection, WrappedCoin } from "../api/types";
import { FROG_CURVE, LOCK, STATE_VIEW, V4_QUOTER } from "./abisV3";
import { I_FLOOR_MODULE, mintLeg, burnLeg, type FloorModuleState } from "./floorModule";
import { v4Fees, type Leg, type Phase, type Venues } from "./quote";

/**
 * docs/v5/ux-and-reserves.md §1, "Where the numbers come from", live. The curve's state read and run through FrogCurve's own
 * arithmetic; each pool hop quoted by Uniswap's V4Quoter (what the pool would really give) with the mid and both fees from
 * StateView.getSlot0; the Floor Module through its adapter. Loaded only when there is a chain to read.
 */
type Key = { currency0: `0x${string}`; currency1: `0x${string}`; fee: number; tickSpacing: number; hooks: `0x${string}` };
const ETH = "0x0000000000000000000000000000000000000000";
const ROUTER_MODULE = [{ type: "function", name: "module", stateMutability: "view", inputs: [], outputs: [{ type: "address" }] }] as const;
const poolId = (k: Key) => keccak256(encodeAbiParameters([{ type: "address" }, { type: "address" }, { type: "uint24" }, { type: "int24" }, { type: "address" }],
  [k.currency0, k.currency1, k.fee, k.tickSpacing, k.hooks]));

async function v4Leg(client: PublicClient, c: WrappedCollection["contracts"], key: Key, tokenIn: `0x${string}`, amountIn: bigint,
  venue: "sharedPool" | "coinPool", tin: Leg["tokenIn"], tout: Leg["tokenOut"], what: string): Promise<Leg | null> {
  if (!c.v4Quoter || !c.stateView || amountIn <= 0n) return null;
  const zeroForOne = key.currency0.toLowerCase() === tokenIn.toLowerCase();
  try {
    const [slot0, sim] = await Promise.all([
      client.readContract({ address: c.stateView, abi: STATE_VIEW, functionName: "getSlot0", args: [poolId(key)] }),
      client.simulateContract({ address: c.v4Quoter, abi: V4_QUOTER, functionName: "quoteExactInputSingle", args: [{ poolKey: key, zeroForOne, exactAmount: amountIn, hookData: "0x" }] })]);
    const [sqrtP, , protocolFee, lpFee] = slot0;
    const p1per0 = (Number(sqrtP) / 2 ** 96) ** 2;
    // v4's protocolFee packs both directions: the low 12 bits are 0→1, the high 12 bits 1→0
    const pf = zeroForOne ? Number(protocolFee) & 0xfff : Number(protocolFee) >> 12;
    return { venue, tokenIn: tin, tokenOut: tout, amountIn, amountOut: sim.result[0], mid: zeroForOne ? p1per0 : 1 / p1per0,
      fees: v4Fees(Number(lpFee), pf, what), spreadBps: 0, contract: venue === "sharedPool" ? "the collection's Uniswap v4 pool, through EtchRouter" : "the coin's Uniswap v4 pool, through EtchRouter" };
  } catch { return null; }
}

export async function liveVenues(client: PublicClient, col: WrappedCollection, coin: WrappedCoin): Promise<Venues> {
  const phaseN = await client.readContract({ address: coin.curve, abi: FROG_CURVE, functionName: "state" });
  const phase = (["trading", "graduating", "graduated", "refunding"] as Phase[])[Number(phaseN)] ?? "trading";
  if (phase === "trading") {
    const [v, t, vEnd, tEnd, feeBps, extra] = await Promise.all((["v", "t", "vEnd", "tEnd", "feeBps", "launchExtraBps"] as const).map((fn) => client.readContract({ address: coin.curve, abi: FROG_CURVE, functionName: fn })));
    return { phase, curve: { v: v as bigint, t: t as bigint, vEnd: vEnd as bigint, tEnd: tEnd as bigint, feeBps: Number(feeBps), launchExtraBps: Number(extra) } };
  }
  if (phase !== "graduated" || !col.contracts.lock) return { phase };
  const lock = col.contracts.lock, c = col.contracts, wx = col.wrapper;
  const [shared, coinKey, info] = await Promise.all([client.readContract({ address: lock, abi: LOCK, functionName: "sharedKey" }) as Promise<Key>,
    client.readContract({ address: lock, abi: LOCK, functionName: "coinKey", args: [coin.address] }) as Promise<Key>,
    client.readContract({ address: lock, abi: LOCK, functionName: "coins", args: [coin.address] })]);
  const quoteIsEth = info[1], quote = quoteIsEth ? ETH : wx;
  // the module: its fills price and spreads for the mid and the Details, its capacities for the room; the amounts themselves are
  // the contract's own `quoteMint` / `quoteBurn` (0 = over the room, or stale). A router built without a module cannot route there.
  let floor: Venues["floor"] = null;
  if (c.floorModule) {
    try {
      const mod = c.floorModule, m = (fn: "price" | "spreads" | "mintCapacity" | "burnCapacity") => client.readContract({ address: mod, abi: I_FLOOR_MODULE, functionName: fn }) as Promise<unknown>;
      const [price, spreads, mintCap, burnCap, routerModule] = await Promise.all([m("price") as Promise<bigint>, m("spreads") as Promise<readonly [bigint, bigint]>,
        m("mintCapacity") as Promise<readonly [bigint, bigint]>, m("burnCapacity") as Promise<readonly [bigint, bigint]>,
        c.router ? client.readContract({ address: c.router, abi: ROUTER_MODULE, functionName: "module" }).catch(() => ETH) : Promise.resolve(ETH)]);
      const state: FloorModuleState = { priceWei: price, spreadMintBps: Number(spreads[0]), spreadBurnBps: Number(spreads[1]),
        mintPriceWei: mintCap[0], mintRoomWei: mintCap[1], burnPriceWei: burnCap[0], burnRoomUnits: burnCap[1] };
      const routable = String(routerModule).toLowerCase() === mod.toLowerCase();
      floor = {
        routable, why: routable ? "" : "this collection's EtchRouter was built without the Floor Module; shown for comparison only",
        mint: async (ethIn) => { const leg = mintLeg(state, ethIn); if (!leg) return null;
          const out = await client.readContract({ address: mod, abi: I_FLOOR_MODULE, functionName: "quoteMint", args: [ethIn] }).catch(() => 0n) as bigint;
          return out > 0n ? { ...leg, amountOut: out } : null; },
        burn: async (unitsIn) => { const leg = burnLeg(state, unitsIn); if (!leg) return null;
          const out = await client.readContract({ address: mod, abi: I_FLOOR_MODULE, functionName: "quoteBurn", args: [unitsIn] }).catch(() => 0n) as bigint;
          return out > 0n ? { ...leg, amountOut: out } : null; },
      };
    } catch { floor = null; }
  }
  return {
    phase, sweepOnly: quoteIsEth, floor,
    shared: (x, dir) => dir === "ethToWx" ? v4Leg(client, c, shared, ETH, x, "sharedPool", "ETH", "WX", "Collection pool") : v4Leg(client, c, shared, wx, x, "sharedPool", "WX", "ETH", "Collection pool"),
    coinPool: (x, dir) => dir === "toCoin" ? v4Leg(client, c, coinKey, quote, x, "coinPool", quoteIsEth ? "ETH" : "WX", "COIN", "Coin pool")
      : v4Leg(client, c, coinKey, coin.address, x, "coinPool", "COIN", quoteIsEth ? "ETH" : "WX", "Coin pool"),
  };
}
