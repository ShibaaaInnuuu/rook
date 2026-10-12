import { POST_POLICY, type PostPolicy } from "../src/floorpost.js";
import { POLICY as DRYRUN_POLICY } from "../src/plan.js";
import type { DrawRule } from "./config.js";

/**
 * docs/v6/p1-services.md §1.4. The production keeper's rules, pure: chain facts and a floor in, a decision and its reason out.
 * The floor itself is planned by src/floorpost.ts's `planPost`, unchanged, under the production policy below.
 */

/** floor-module.md §5: "posts at least every 30 minutes, and on every move of more than 0.5%". The TTL stays the chain's. */
/** R2-S4: the service follows a rise that neither the sales nor five listings back at 2% an hour at most. */
export const PROD_POST_POLICY: PostPolicy = { ...POST_POLICY, refreshBeforeExpiryS: 1_800, repostMovedBps: 50n, liftBpsPerHour: 200n, liftDepth: 5 };

/** SweepVault's H12 constants (src/SweepVault.sol): the cap moves in steps of at most 20%, at most once per 10 minutes, both clocks. */
export const VAULT_RULES = { capTtlS: 3_600, maxStepBps: 2_000n, minPostGapS: 600 } as const;
/** plan.ts's cap policy: re-post under 15 minutes of TTL, or on a move of more than 2%. */
export const CAP_POLICY = { repostIfUnderS: DRYRUN_POLICY.repostIfUnderS, repostIfMovedBps: DRYRUN_POLICY.repostIfMovedBps } as const;

export type CapState = { cap: bigint; capAt: number; capAtBlock: number; fresh: boolean; ceiling: bigint; parentBlockSeconds: number };
export type CapPlan = { type: "skip"; why: string } | { type: "keep"; capWei: string; why: string } | { type: "post"; capWei: string; targetWei: string; why: string };

/**
 * The cap for one sweep vault. `floorWei` is the MODULE'S accepted floor, and only while the module is fresh (null otherwise):
 * a cap never rests on a floor the chain has not accepted. `nowS` / `nowBlock`: the chain's two clocks (the latest block's
 * timestamp, and the EVM's `block.number`, which on 4663 is the parent chain's).
 *
 * C-01 (fixes-B-C-keeper-draws.md §5 item 4): no ceiling means the vault refuses every cap (`NoCeiling`), so nothing is planned;
 * no cap is ever planned above the ceiling; and every post rises at most 20% above the last cap, fresh OR stale (the vault's rule;
 * a stale re-post may fall freely, lower being safer).
 */
export function planCap(floorWei: bigint | null, s: CapState, nowS: number, nowBlock: number, marginBps: bigint): CapPlan {
  if (s.ceiling === 0n) return { type: "skip", why: "the vault has no ceiling (capCeiling() = 0): it refuses every cap (NoCeiling) until the owner sets one" };
  if (floorWei === null || floorWei <= 0n) return { type: "skip", why: "no fresh floor at the module: no cap is posted, and the posted one ages out (sweeps are refused)" };
  let target = floorWei + (floorWei * marginBps) / 10_000n;
  let why = `floor ${floorWei} + ${marginBps} bps`;
  if (target > s.ceiling) { target = s.ceiling; why += `, held at the ceiling ${s.ceiling}`; }
  const left = s.capAt + VAULT_RULES.capTtlS - nowS;
  const up = s.cap + (s.cap * VAULT_RULES.maxStepBps) / 10_000n;
  if (s.fresh && s.cap > 0n) {
    const lo = s.cap - (s.cap * VAULT_RULES.maxStepBps) / 10_000n;
    const stepped = target < lo ? lo : target > up ? up : target;
    if (stepped !== target) why += `; one step toward it inside ±20% of the posted ${s.cap}`;
    const moved = ((stepped > s.cap ? stepped - s.cap : s.cap - stepped) * 10_000n) / s.cap;
    if (moved <= CAP_POLICY.repostIfMovedBps && left > CAP_POLICY.repostIfUnderS) return { type: "keep", capWei: String(s.cap), why: `within ${CAP_POLICY.repostIfMovedBps} bps and ${left} s of its hour left` };
    const gapBlocks = Math.floor(VAULT_RULES.minPostGapS / Math.max(1, s.parentBlockSeconds));
    if (nowS < s.capAt + VAULT_RULES.minPostGapS || nowBlock < s.capAtBlock + gapBlocks)
      return { type: "keep", capWei: String(s.cap), why: `the vault takes one post per ${VAULT_RULES.minPostGapS} s on both clocks; waiting` };
    if (stepped > s.ceiling) return { type: "skip", why: `a step would pass the ceiling ${s.ceiling}` };
    return { type: "post", capWei: String(stepped), targetWei: String(target), why: left <= CAP_POLICY.repostIfUnderS ? `${Math.max(0, left)} s left: refreshed (${why})` : `moved ${moved} bps (${why})` };
  }
  let capWei = target;
  if (s.cap > 0n && target > up) { capWei = up; why += `; one step up: at most 20% above the last cap ${s.cap}, stale or not`; }
  return { type: "post", capWei: String(capWei), targetWei: String(target), why: s.cap === 0n ? `no cap was ever posted (${why})` : `the posted cap is stale (${why})` };
}

/** The draw slot at or before `nowS`: `T_k = k × everyS + offsetS`, on the chain's clock (§1.4). */
export function slotAt(nowS: number, r: DrawRule): number {
  const k = Math.floor((nowS - r.offsetS) / r.everyS);
  return k * r.everyS + r.offsetS;
}

/**
 * Whether slot `T`'s draws may be ANNOUNCED now: under `windowS` after the slot on the chain's clock. The announcement no longer
 * picks the snapshot (a drand round that does not exist yet does: SweepVault.snapshotTimeOf), so one clock is enough here; past
 * the window the slot is missed, which is an alert, never a quiet skip.
 */
export function announceOpen(slot: number, nowS: number, r: DrawRule): { open: boolean; why: string } {
  if (nowS < slot) return { open: false, why: "the slot has not come yet" };
  const age = nowS - slot;
  if (age >= r.windowS) return { open: false, why: `the slot was ${age} s ago: past the ${r.windowS} s window` };
  return { open: true, why: `the slot was ${age} s ago` };
}

/** Kept for the rehearsal's arithmetic: a moment `snapshotTs` under `windowS` old on both clocks. */
export function slotOpen(snapshotTs: number, snapshotEvmBlock: number, nowS: number, nowEvmBlock: number, r: DrawRule, parentBlockSeconds: number): { open: boolean; why: string } {
  if (snapshotTs > nowS || snapshotEvmBlock > nowEvmBlock) return { open: false, why: "the slot's block is not yet on chain" };
  const age = nowS - snapshotTs, blocks = nowEvmBlock - snapshotEvmBlock;
  const maxBlocks = Math.floor(r.windowS / Math.max(1, parentBlockSeconds));
  if (age >= r.windowS || blocks >= maxBlocks) return { open: false, why: `the slot's block is ${age} s / ${blocks} parent blocks old: past the ${r.windowS} s window, skipped (no other block is chosen)` };
  return { open: true, why: `the slot's block is ${age} s old` };
}

/**
 * Whether an announced draw may still be OPENED by the keeper: under `OPEN_TTL − margin` since the announcement on both clocks
 * (the vault refuses at `OPEN_TTL`, and anyone may void it then). The margin leaves a stuck transaction time to land.
 */
export const OPEN_MARGIN_S = 900;
export function openStillAllowed(announcedAt: number, announcedAtBlock: number, nowS: number, nowEvm: number, parentBlockSeconds: number, openTtlS: number): boolean {
  const limit = openTtlS - OPEN_MARGIN_S;
  return nowS < announcedAt + limit && nowEvm < announcedAtBlock + Math.floor(limit / Math.max(1, parentBlockSeconds));
}

/** Both clocks past `at` / `atBlock + seconds of parent blocks` (lib/Wait.sol's `elapsed`, as the contracts judge it). */
export const elapsed = (at: number, atBlock: number, seconds: number, nowS: number, nowEvm: number, parentBlockSeconds: number): boolean =>
  nowS >= at + seconds && nowEvm >= atBlock + Math.floor(seconds / Math.max(1, parentBlockSeconds));

/** The published name of a draw's snapshot: coin, block and kind, as tools/snapshot's CLI names its output. */
export const snapshotName = (coin: string, block: number): string => `snapshot-${coin.toLowerCase()}-${block}-weighted.json`;
