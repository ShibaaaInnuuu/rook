import { readFileSync } from "node:fs";
import type { Hex } from "viem";

/**
 * docs/v6/deploy-stack.md §3. The stack file the keeper posts to: written by script/deploy-stack.sh, read here and checked. The
 * keeper posts floors to each collection's module, caps to its RAFFLE and BURN vaults, and draws on its RAFFLE vault. The YOYO
 * vault is read (to be refused, never to be posted to): Phase 1 keeps YOYO off (docs/v6/phase1-scope.md §1).
 */
export type StackCollection = { slug: string; name: string; kind: string; listingId: number; wrapper: Hex; factory: Hex; lock: Hex; router: Hex;
  floorModule: Hex; mintDesk: Hex; raffleVault: Hex; burnVault: Hex; yoyoVault: Hex; custody: { kind: string; address?: string };
  /** docs/v7/phase1-nft.md §3.2: an nft collection's floor is posted by the canister (this address), never by the keeper. */
  floorPoster: Hex | null };
export type Stack = { chainId: number; label: string; fromBlock: number; stage: number; registry: Hex; keeper: Hex; treasury: Hex; poolManager: Hex;
  collections: StackCollection[] };

const ADDR = /^0x[0-9a-fA-F]{40}$/;
export const ZERO = "0x0000000000000000000000000000000000000000";

export function parseStack(text: string, chainId: number): Stack {
  const j = JSON.parse(text) as Record<string, unknown>;
  const problems: string[] = [];
  if (Number(j.chainId) !== chainId) problems.push(`the stack file is for chain ${String(j.chainId)}, and the keeper signs for ${chainId}`);
  if (j.label !== "stack") problems.push(`the stack file's label is ${JSON.stringify(j.label ?? null)}, not "stack"`);
  for (const k of ["registry", "keeper", "treasury", "poolManager"]) if (!ADDR.test(String(j[k]))) problems.push(`the stack file's ${k} is not an address`);
  const cols = Array.isArray(j.collections) ? (j.collections as Record<string, unknown>[]) : [];
  if (cols.length === 0) problems.push("the stack file names no collection");
  for (const c of cols) {
    for (const k of ["wrapper", "factory", "lock", "router", "floorModule", "raffleVault", "burnVault"]) {
      if (!ADDR.test(String(c[k])) || String(c[k]) === ZERO) problems.push(`${String(c.slug)}: ${k} is not an address`);
    }
    if (c.kind === "nft" && (!ADDR.test(String(c.floorPoster)) || String(c.floorPoster) === ZERO)) problems.push(`${String(c.slug)}: an nft collection names no floorPoster`);
  }
  if (problems.length) throw new Error(`refusing the stack file:\n  - ${problems.join("\n  - ")}`);
  const fromBlock = Number(j.fromBlock ?? j.deployedAtBlock ?? 0);
  return { chainId, label: "stack", fromBlock: Number.isFinite(fromBlock) ? fromBlock : 0, stage: Number(j.stage ?? 0), registry: j.registry as Hex, keeper: j.keeper as Hex,
    treasury: j.treasury as Hex, poolManager: j.poolManager as Hex,
    collections: cols.map((c) => ({ slug: String(c.slug), name: String(c.name ?? c.slug), kind: String(c.kind ?? "inscriptions"), listingId: Number(c.listingId ?? 0),
      wrapper: c.wrapper as Hex, factory: c.factory as Hex, lock: c.lock as Hex, router: c.router as Hex, floorModule: c.floorModule as Hex,
      mintDesk: (ADDR.test(String(c.mintDesk)) ? c.mintDesk : ZERO) as Hex, raffleVault: c.raffleVault as Hex, burnVault: c.burnVault as Hex,
      yoyoVault: (ADDR.test(String(c.yoyoVault)) ? c.yoyoVault : ZERO) as Hex, custody: (c.custody ?? { kind: "unknown" }) as StackCollection["custody"],
      floorPoster: (ADDR.test(String(c.floorPoster)) ? c.floorPoster : null) as Hex | null })) };
}

export const loadStack = (path: string, chainId: number): Stack => parseStack(readFileSync(path, "utf8"), chainId);
