import { decodeFunctionResult, encodeFunctionData, type Hex } from "viem";
import { stagingSender, type Env, type Fetch, type StagingSender } from "./guard.js";
import { FLOOR_MODULE_ABI, planPost, type PostAction, type Posted } from "../src/floorpost.js";
import type { Floor } from "../src/floor.js";

/**
 * docs/v5/floor-feed.md §5.2. The floor poster on our STAGING fork: for each module, read what is posted, plan (src/floorpost.ts,
 * pure), and send the planned `postFloor` AS the module's keeper address through `stagingSender` — which refuses unless
 * KEEPER_MODE is "staging", the node is on this machine, and the node reports the staging chain id before every send. Nothing
 * here can reach Robinhood Chain or any public chain.
 */
export type FloorJob = { module: Hex; floor: () => Promise<Floor | null> };

async function posted(s: StagingSender, module: Hex): Promise<Posted> {
  const call = async (functionName: "floor" | "maxMoveBps" | "floorBounds") => {
    const out = await s.request<Hex>("eth_call", [{ to: module, data: encodeFunctionData({ abi: FLOOR_MODULE_ABI, functionName }) }, "latest"]);
    return decodeFunctionResult({ abi: FLOOR_MODULE_ABI, functionName, data: out }) as unknown;
  };
  const [priceWei, postedAt, ttl] = (await call("floor")) as [bigint, bigint, bigint];
  const maxMoveBps = Number(await call("maxMoveBps"));
  let bounds: Posted["bounds"] = null;
  try { const [lo, hi] = (await call("floorBounds")) as [bigint, bigint]; bounds = { lo, hi }; } catch { bounds = null; }
  return { priceWei, postedAt: Number(postedAt), ttl: Number(ttl), maxMoveBps, bounds };
}

async function keeperOf(s: StagingSender, module: Hex): Promise<Hex> {
  const out = await s.request<Hex>("eth_call", [{ to: module, data: encodeFunctionData({ abi: FLOOR_MODULE_ABI, functionName: "keeper" }) }, "latest"]);
  return decodeFunctionResult({ abi: FLOOR_MODULE_ABI, functionName: "keeper", data: out }) as Hex;
}

/** One pass over the modules: what was planned for each, and the receipt of each post that was sent. */
export async function postFloors(url: string, jobs: FloorJob[], env: Env = process.env, fetchImpl: Fetch = fetch,
  nowS: () => number = () => Math.floor(Date.now() / 1000)): Promise<{ module: Hex; action: PostAction; tx?: string; error?: string }[]> {
  const s = await stagingSender(url, env, fetchImpl);
  const out: { module: Hex; action: PostAction; tx?: string; error?: string }[] = [];
  for (const j of jobs) {
    let action: PostAction = { type: "skip", why: "not read" };
    try {
      const [floor, p] = await Promise.all([j.floor(), posted(s, j.module)]);
      action = planPost(floor, p, nowS());
      if (action.type !== "post") { out.push({ module: j.module, action }); continue; }
      const receipt = await s.send({ from: await keeperOf(s, j.module), to: j.module, data: action.calldata });
      out.push({ module: j.module, action, tx: receipt.transactionHash });
    } catch (err) {
      out.push({ module: j.module, action, error: String((err as Error).message).slice(0, 200) });
    }
  }
  return out;
}
