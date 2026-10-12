import { decodeFunctionResult, encodeFunctionData, parseAbi, type Hex } from "viem";
import { rpc, type Fetch } from "./chain.js";
import type { Floor } from "./floor.js";

/**
 * docs/v5/floor-feed.md §5. What the keeper WOULD post to a collection's Floor Module, against the interface both sides ASSUME
 * (the contracts are on another branch; reconciling names is a change to FLOOR_MODULE_ABI alone). Pure except `readPosted`,
 * which reads through `chain.ts`'s read-only `rpc`. Nothing here signs or sends: the staging operator (staging/floorfeed.ts) does.
 */
export const FLOOR_MODULE_ABI = parseAbi([
  "function floor() view returns (uint256 priceWei, uint64 postedAt, uint64 ttl)",
  "function maxMoveBps() view returns (uint16)",
  "function floorBounds() view returns (uint256 lo, uint256 hi)",
  "function keeper() view returns (address)",
  "function postFloor(uint256 priceWei, uint16 confidenceBps, uint32 depthItems)",
]);

export const POST_POLICY = {
  minConfidenceBps: 3_000,
  refreshBeforeExpiryS: 900,
  repostMovedBps: 100n,
  hourS: 3_600,
} as const;
/**
 * `liftBpsPerHour` / `liftDepth` (audit R2-S4, the service's policy): a RISE of the floor that the sales do not support (no sale
 * median, or one below the target) and that fewer than `liftDepth` listings back (depthItems) is followed at most `liftBpsPerHour`
 * an hour, pro rata. Buying the one cheap listing of a thin book lifts the listing floor at once (+7.3% on the recorded 2026-09-23
 * book); with this rule the posted floor climbs toward it at 2%/h instead of the module's 5%/h, so a pump has to be held for hours
 * against every other seller. Falls are never slowed here (E-01's side is `maxSalePullBps`).
 */
export type PostPolicy = { minConfidenceBps: number; refreshBeforeExpiryS: number; repostMovedBps: bigint; hourS: number; liftBpsPerHour?: bigint; liftDepth?: number };

/** R2-S4: the highest target an unsupported rise may reach now, or null when the rule does not apply. */
export function liftCeiling(floor: Floor, posted: Posted, target: bigint, nowS: number, pol: PostPolicy): bigint | null {
  if (pol.liftBpsPerHour === undefined || posted.priceWei === 0n || target <= posted.priceWei) return null;
  if (floor.depthItems >= (pol.liftDepth ?? 5)) return null;
  if (floor.saleFloor !== null && floor.saleFloor >= target) return null;
  const elapsed = BigInt(Math.max(0, Math.min(nowS - posted.postedAt, pol.hourS)));
  return posted.priceWei + (posted.priceWei * pol.liftBpsPerHour * elapsed) / (10_000n * BigInt(pol.hourS));
}

/**
 * The module's posted state, as read at one block. `bounds` is null when the module has no `floorBounds()`. `fresh`, the posted
 * `confidenceBps` and the module's `minConfidenceBps` are optional (older modules, the dry run): when given, a module that is not
 * fresh, or whose posted confidence is under its minimum, is REPAIRED by the next post even at the same price (audit E-02a: a
 * confidence-0 post by a stolen key must not be "kept").
 */
export type Posted = { priceWei: bigint; postedAt: number; ttl: number; maxMoveBps: number; bounds: { lo: bigint; hi: bigint } | null;
  fresh?: boolean; confidenceBps?: number; minConfidenceBps?: number };

/** Why the posted floor needs a post whatever the price: it is not fresh, or its confidence is under the module's minimum. */
export function needsRepair(p: Posted): string | null {
  if (p.priceWei === 0n) return null;
  if (p.confidenceBps !== undefined && p.minConfidenceBps !== undefined && p.confidenceBps < p.minConfidenceBps)
    return `the posted confidence ${p.confidenceBps} bps is under the module's minimum ${p.minConfidenceBps}: the module is stale until a good post`;
  if (p.fresh === false) return "the module is not fresh";
  return null;
}

export type PostAction =
  | { type: "skip"; why: string }
  | { type: "keep"; priceWei: string; why: string }
  | { type: "post"; priceWei: string; targetWei: string; confidenceBps: number; depthItems: number; stepped: boolean; calldata: Hex; why: string };

/**
 * The band a post must land in now: the chain's `floorBounds()` when it answers, intersected with the keeper's own conservative
 * reading of "max move per hour" — pro rata to the time since the last post, capped at an hour — so a post is inside the band
 * under either reading of the rule. Null: nothing has been posted yet (any price).
 */
export function band(p: Posted, nowS: number, pol: PostPolicy = POST_POLICY): { lo: bigint; hi: bigint } | null {
  if (p.priceWei === 0n) return p.bounds;
  const elapsed = BigInt(Math.max(0, Math.min(nowS - p.postedAt, pol.hourS)));
  const move = (p.priceWei * BigInt(p.maxMoveBps) * elapsed) / (10_000n * BigInt(pol.hourS));
  let lo = p.priceWei - move, hi = p.priceWei + move;
  if (p.bounds) {
    if (p.bounds.lo > lo) lo = p.bounds.lo;
    if (p.bounds.hi < hi) hi = p.bounds.hi;
  }
  if (lo > hi) return { lo: p.priceWei, hi: p.priceWei };
  return { lo, hi };
}

const clamp = (x: bigint, b: { lo: bigint; hi: bigint } | null): bigint => (b === null ? x : x < b.lo ? b.lo : x > b.hi ? b.hi : x);

export const postCalldata = (priceWei: bigint, confidenceBps: number, depthItems: number): Hex =>
  encodeFunctionData({ abi: FLOOR_MODULE_ABI, functionName: "postFloor", args: [priceWei, Math.max(0, Math.min(10_000, confidenceBps)), Math.max(0, Math.min(0xffffffff, depthItems))] });

/** §5.2: skip, keep, or post (stepped toward the target, never past the band). `floor.price` must already be in wei. */
export function planPost(floor: Floor | null, posted: Posted, nowS: number, pol: PostPolicy = POST_POLICY): PostAction {
  if (!floor || floor.price === null) return { type: "skip", why: "no floor: fewer than the minimum valid listings" };
  if (floor.confidenceBps < pol.minConfidenceBps)
    return { type: "skip", why: `confidence ${floor.confidenceBps} bps is under ${pol.minConfidenceBps}: the posted price is left to age` };
  const target = floor.price;
  if (target <= 0n) return { type: "skip", why: "a floor of zero is not a price" };
  const b = band(posted, nowS, pol);
  const lift = liftCeiling(floor, posted, target, nowS, pol);
  const lifted = lift !== null && lift < target;
  let next = clamp(lifted ? lift : target, b);
  if (lifted && next < posted.priceWei) next = posted.priceWei;
  const left = posted.postedAt + posted.ttl - nowS;
  const repair = needsRepair(posted);
  if (posted.priceWei > 0n && !repair) {
    const diff = next > posted.priceWei ? next - posted.priceWei : posted.priceWei - next;
    const moved = (diff * 10_000n) / posted.priceWei;
    if (moved <= pol.repostMovedBps && left > pol.refreshBeforeExpiryS)
      return { type: "keep", priceWei: String(posted.priceWei), why: `within ${pol.repostMovedBps} bps and ${left} s of its TTL left` };
    if (next === posted.priceWei && left > pol.refreshBeforeExpiryS)
      return { type: "keep", priceWei: String(posted.priceWei), why: "the band allows no move yet" };
  }
  const stepped = next !== target;
  const why = posted.priceWei === 0n ? "nothing is posted yet"
    : lifted && !repair ? `a rise to ${target} that neither the sales nor ${pol.liftDepth ?? 5} listings back: followed at ${pol.liftBpsPerHour} bps an hour (R2-S4), now ${next}`
    : repair ? `repair: ${repair}${stepped ? `; one step toward the target ${target} inside [${b?.lo}, ${b?.hi}]` : ""}`
    : stepped ? `the target ${target} is outside what the chain accepts now [${b?.lo}, ${b?.hi}]: one step toward it`
    : left <= pol.refreshBeforeExpiryS ? `${Math.max(0, left)} s of the TTL left: refreshed` : "moved";
  return { type: "post", priceWei: String(next), targetWei: String(target), confidenceBps: floor.confidenceBps, depthItems: floor.depthItems, stepped,
    calldata: postCalldata(next, floor.confidenceBps, floor.depthItems), why };
}

/** Read the module's posted state through `eth_call` (reads only). A module without `floorBounds()` answers with a revert: null. */
export async function readPosted(url: string, module: Hex, fetchImpl: Fetch = fetch, block: string = "latest"): Promise<Posted> {
  const call = async (functionName: "floor" | "maxMoveBps" | "floorBounds") => {
    const data = encodeFunctionData({ abi: FLOOR_MODULE_ABI, functionName });
    const out = await rpc<Hex>(url, "eth_call", [{ to: module, data }, block], fetchImpl);
    return decodeFunctionResult({ abi: FLOOR_MODULE_ABI, functionName, data: out }) as unknown;
  };
  const [priceWei, postedAt, ttl] = (await call("floor")) as [bigint, bigint, bigint];
  const maxMoveBps = Number(await call("maxMoveBps"));
  let bounds: Posted["bounds"] = null;
  try { const [lo, hi] = (await call("floorBounds")) as [bigint, bigint]; bounds = { lo, hi }; } catch { bounds = null; }
  return { priceWei, postedAt: Number(postedAt), ttl: Number(ttl), maxMoveBps, bounds };
}
