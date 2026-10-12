import type { WrappedMarket } from "../api/types";
import { poolLeg, type Phase, type Venues } from "./quote";
import { floorVenue, stateFrom } from "./floorModule";

/** The pure half of wrapped.ts (no React, no chain): the fixture source, its time shift, and fixture venues for quotes. */
export type WSource = { kind: "live" | "fixture"; note: string; shiftS: number; get: <T>(path: string) => Promise<T> };
type Index = { kind: string; note: string; generatedAt: number; files: Record<string, string> };

/** Fixture times are moved forward by how long ago the set was generated, so a deadline in it is still ahead of you. */
const TIME_KEYS = new Set(["at", "computedAt", "lastGreenAt", "checkedAt", "mintedAt", "burnedAt", "deliverBy", "launchedAt", "endsAt", "settledAt", "floorPostedAt"]);
export function shiftTimes<T>(x: T, by: number): T {
  if (!by || x === null || typeof x !== "object") return x;
  if (Array.isArray(x)) return x.map((v) => shiftTimes(v, by)) as T;
  const out: Record<string, unknown> = {};
  for (const [k, v] of Object.entries(x as Record<string, unknown>)) out[k] = TIME_KEYS.has(k) && typeof v === "number" ? v + by : shiftTimes(v, by);
  return out as T;
}

export async function wrappedFixtures(nowS: number, read: (f: string) => Promise<unknown> = async (f) => (await fetch(`/fixtures/wrapped/${f}`)).json()): Promise<WSource> {
  const index = (await read("index.json")) as Index;
  const shiftS = Math.max(0, nowS - index.generatedAt);
  return { kind: "fixture", note: index.note, shiftS, get: async <T,>(path: string) => {
    const file = index.files[path];
    if (!file) throw new Error(`the fixture has no answer for ${path}`);
    return shiftTimes((await read(file)) as T, shiftS);
  } };
}

/** Fixture mode: the quote's venues from the fixture's market. Each pool hop is a labelled constant-product estimate. */
export function marketVenues(m: WrappedMarket | null, coin: string, phase: Phase, sweepOnly: boolean): Venues {
  const c = m?.coins[coin] ?? m?.coins[coin.toLowerCase()] ?? null, big = BigInt;
  const sh = m?.shared ?? null, p = c?.pool ?? null;
  return {
    phase, sweepOnly,
    curve: c?.curve ? { v: big(c.curve.v), t: big(c.curve.t), vEnd: big(c.curve.vEnd), tEnd: big(c.curve.tEnd), feeBps: c.curve.feeBps, launchExtraBps: c.curve.launchExtraBps ?? 0 } : null,
    shared: sh ? async (x, dir) => dir === "ethToWx"
      ? poolLeg({ reserveIn: big(sh.reserveEth), reserveOut: big(sh.reserveWrapped), ...sh }, x, "sharedPool", "ETH", "WX", "Collection pool")
      : poolLeg({ reserveIn: big(sh.reserveWrapped), reserveOut: big(sh.reserveEth), ...sh }, x, "sharedPool", "WX", "ETH", "Collection pool") : null,
    coinPool: p ? async (x, dir) => dir === "toCoin"
      ? poolLeg({ reserveIn: big(p.reserveWrapped), reserveOut: big(p.reserveCoin), ...p }, x, "coinPool", "WX", "COIN", "Coin pool")
      : poolLeg({ reserveIn: big(p.reserveCoin), reserveOut: big(p.reserveWrapped), ...p }, x, "coinPool", "COIN", "WX", "Coin pool") : null,
    floor: m?.floorModule ? floorVenue(stateFrom({ priceWei: big(m.floorModule.priceWei), spreadMintBps: m.floorModule.spreadMintBps, spreadBurnBps: m.floorModule.spreadBurnBps,
      mintRoomWei: big(m.floorModule.mintRoomWei), burnRoomUnits: big(m.floorModule.burnRoomUnits) })) : null,
  };
}
