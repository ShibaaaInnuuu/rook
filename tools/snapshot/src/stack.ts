import { readFileSync } from "node:fs";

/**
 * docs/snapshot-v1.md "Version 2", "Exclusions", for a coin on our own curve (a stack deployed by script/deploy-stack.sh): the addresses that
 * hold the coin without being holders. The keeper builds its draws with this, and anyone recomputing a draw's root uses the same
 * function (`--stack`), so both sides exclude exactly the same set. Pure except `readExclusions`, which reads a file and FAILS
 * CLOSED: an unreadable or malformed list is an error, never an empty list (audit E-14).
 */
export type StackLike = {
  poolManager: string; treasury: string; registry: string;
  collections: { slug: string; factory: string; lock: string; router: string; raffleVault: string; burnVault: string; yoyoVault: string; floorModule: string; mintDesk: string }[];
};

const ADDR = /^0x[0-9a-fA-F]{40}$/;

/** `tools/snapshot/exclusions.json`: `[{ "address": "0x…", "why": "…" }, …]`. Throws on anything else. */
export function readExclusions(path: string): string[] {
  let j: unknown;
  try { j = JSON.parse(readFileSync(path, "utf8")); } catch (err) {
    throw new Error(`the exclusions file ${path} could not be read (${String((err as Error).message).slice(0, 120)}): no snapshot is built without it`);
  }
  if (!Array.isArray(j)) throw new Error(`the exclusions file ${path} is not a list`);
  return j.map((x, i) => {
    const a = (x as { address?: unknown })?.address;
    if (typeof a !== "string" || !ADDR.test(a)) throw new Error(`the exclusions file ${path}: entry ${i} has no valid address`);
    return a.toLowerCase();
  });
}

/** Zero, dead, the coin, its curve, and every contract of the coin's collection and of the stack, plus the listed addresses. */
export function stackExclusions(stack: StackLike, slug: string, coin: string, curve: string, listed: readonly string[]): string[] {
  const c = stack.collections.find((x) => x.slug === slug);
  if (!c) throw new Error(`the stack names no collection ${slug}`);
  return [...new Set(["0x0000000000000000000000000000000000000000", "0x000000000000000000000000000000000000dead", coin, curve, c.factory, c.lock, c.router,
    stack.poolManager, c.raffleVault, c.burnVault, c.yoyoVault, c.floorModule, c.mintDesk, stack.treasury, stack.registry, ...listed].map((a) => a.toLowerCase()))]
    .filter((a) => /^0x[0-9a-f]{40}$/.test(a));
}
