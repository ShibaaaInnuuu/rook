import type { FloorModuleRead } from "../api/types";

type Read = Extract<FloorModuleRead, { kind: "read" }>;

/**
 * What the Books page may say the Floor Module is doing, from one read. R4-check A4-I3 (docs/audit/2026-09-26/review4/
 * fixes-golive-lows.md §8): for up to FLOOR_TTL (3 h; an hour before 2026-10-09) after a pause the last post is still fresh, so "fresh, so the module trades" was shown
 * while mints were refused and burns paid the pause floor. The keeper is down when the registry says so (`keeperPaused`: paused, or
 * no keeper named); failing that read, when `pauseFloor()` is non-zero (the module returns it only while its keeper is down and it
 * has not escaped); failing both (a module built before round 4), the round-4 derivation. Only a fresh floor with its keeper up
 * trades.
 */
export type ModuleState = "trades" | "keeper-down" | "escaped" | "stale";

export function keeperDownOf(m: Pick<Read, "keeperPaused" | "pauseFloorWei" | "burnsOpen" | "floorFresh" | "escaped" | "exitAtBacking">): boolean {
  if (m.keeperPaused === true || m.keeperPaused === false) return m.keeperPaused;
  if (m.pauseFloorWei !== undefined && m.pauseFloorWei !== null) return BigInt(m.pauseFloorWei) > 0n;
  // r4-contracts: exitAtBacking() is escaped() alone; a paused keeper is burnsOpen() on a module that is neither fresh nor escaped
  return m.burnsOpen === true ? !m.floorFresh && !m.escaped : m.exitAtBacking === true && !m.escaped;
}

export function moduleStateOf(m: Pick<Read, "keeperPaused" | "pauseFloorWei" | "burnsOpen" | "floorFresh" | "escaped" | "exitAtBacking">): ModuleState {
  if (m.escaped === true) return "escaped";
  if (keeperDownOf(m)) return "keeper-down";
  return m.floorFresh ? "trades" : "stale";
}
