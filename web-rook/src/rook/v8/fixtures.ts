import type { DrawReport } from "./core/types";

/** The pure half of source.ts (no React, no Vite): the v8 fixture set and its time shift, so a test can hold it. */
export type V8Src = { kind: "live" | "fixture"; note: string; shiftS: number; get: <T>(path: string) => Promise<T> };
type Index = { kind: string; note: string; generatedAt: number; files: Record<string, string> };

const TIMES = new Set(["computedAt", "lastGreenAt", "homeAt", "ethAt", "at", "mintedAt", "ownerAt", "readAt", "advancedAt", "deliverBy", "burnedAt"]);
export function shiftReserve<T>(x: T, by: number): T {
  if (!by || x === null || typeof x !== "object") return x;
  if (Array.isArray(x)) return x.map((v) => shiftReserve(v, by)) as T;
  const out: Record<string, unknown> = {};
  for (const [k, v] of Object.entries(x as Record<string, unknown>)) out[k] = TIMES.has(k) && typeof v === "number" ? v + by : shiftReserve(v, by);
  return out as T;
}
/** A draws report moved forward: its computation and the vault's state, never the proofs (pinned to real rounds). */
export function shiftDraws(r: DrawReport, by: number): DrawReport {
  return by ? { ...r, computedAt: r.computedAt + by, state: r.state ? shiftReserve(r.state, by) : null } : r;
}

export async function v8Fixtures(nowS: number, read: (f: string) => Promise<unknown> = async (f) => (await fetch(`/fixtures/v8/${f}`)).json()): Promise<V8Src> {
  const index = (await read("index.json")) as Index;
  const shiftS = Math.max(0, nowS - index.generatedAt);
  return { kind: "fixture", note: index.note, shiftS, get: async <T,>(path: string) => {
    const file = index.files[path];
    if (!file) throw new Error(`the fixture has no answer for ${path}`);
    const body = (await read(file)) as T;
    if (path.startsWith("/v1/v8/reserves/") || path.startsWith("selfcheck/")) return shiftReserve(body, shiftS);
    if (path.startsWith("/v1/v8/draws/")) return shiftDraws(body as DrawReport, shiftS) as T;
    return body;
  } };
}

