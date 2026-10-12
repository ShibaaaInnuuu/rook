import type { GraduationTargets, WrappedCollection } from "../api/types";

/**
 * docs/v7/phase1-nft-pooled.md: what the collection page says about the nest's own graduation target and its pool. A graduation
 * sends part of its raise to the Floor Module as ETH-backed units; on an expensive collection one graduation is less than an item,
 * so the units wait in the module (every one still backed by its ETH) until the next graduations of the SAME collection complete
 * an item plus the buffer. Pure: every figure is a field the indexer read.
 */
export type PoolView = {
  /** "9 ETH", or "8 ETH early, then 5 ETH" */
  target: string | null;
  /** units in the pool, the level a buy is due at, and the share of the way there (0–100, capped) */
  units: number; dueAt: number; pct: number;
  due: boolean;
  text: string;
};

const u4 = (w: bigint) => Number(w / 10n ** 14n) / 1e4;
const ethWord = (w: string) => `${Number(BigInt(w) / 10n ** 15n) / 1e3} ETH`;

export function targetText(g: GraduationTargets | null | undefined): string | null {
  if (!g) return null;
  return g.earlyRaiseWei === g.normalRaiseWei ? ethWord(g.normalRaiseWei)
    : `${ethWord(g.earlyRaiseWei)} while the shared pool is under ${ethWord(g.earlyUntilEthWei)}, then ${ethWord(g.normalRaiseWei)}`;
}

export function poolView(c: Pick<WrappedCollection, "graduation" | "pool">): PoolView | null {
  if (!c.pool) return null;
  const units = BigInt(c.pool.units), dueAt = BigInt(c.pool.dueAtUnits);
  const due = units >= dueAt;
  const pct = dueAt > 0n ? Math.min(100, Number((units * 100n) / dueAt)) : 0;
  const target = targetText(c.graduation);
  const text = due
    ? `The pool holds ${u4(units)} wrapped tokens: a whole NFT plus the buffer, so the canister's next buy for this collection is due.`
    : units === 0n
      ? "Nothing is pooled: every graduation so far has been turned into an NFT."
      : `The pool holds ${u4(units)} of the ${u4(dueAt)} wrapped tokens a buy needs (one NFT plus the buffer). Graduations that raise less than an NFT pool here, each token backed by its own ETH, until this collection's next graduations complete it; another collection's never count toward it.`;
  return { target, units: u4(units), dueAt: u4(dueAt), pct, due, text };
}
