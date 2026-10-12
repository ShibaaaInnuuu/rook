/** The pure half of the board's source: the fixture set and its time shift. No React, no Vite, so a test can hold it. */
export type RookSrc = { kind: "live" | "fixture"; note: string; shiftS: number; get: <T>(path: string) => Promise<T> };
type Index = { kind: string; note: string; generatedAt: number; files: Record<string, string> };
const TIME_KEYS = new Set(["at", "launchedAt", "graduatedAt", "t"]);
export function shiftTimes<T>(x: T, by: number): T {
  if (!by || x === null || typeof x !== "object") return x;
  if (Array.isArray(x)) return x.map((v) => shiftTimes(v, by)) as T;
  const out: Record<string, unknown> = {};
  for (const [k, v] of Object.entries(x as Record<string, unknown>)) out[k] = TIME_KEYS.has(k) && typeof v === "number" ? v + by : shiftTimes(v, by);
  return out as T;
}
/** Fixture times are moved forward by how long ago the set was generated, on the same clock the demo recording is moved by. */
export async function rookFixtures(nowS: number, read: (f: string) => Promise<unknown> = async (f) => (await fetch(`/fixtures/rook/${f}`)).json()): Promise<RookSrc> {
  const index = (await read("index.json")) as Index;
  const shiftS = Math.max(0, nowS - index.generatedAt);
  const lower = new Map(Object.entries(index.files).map(([k, v]) => [k.toLowerCase(), v]));
  return { kind: "fixture", note: index.note, shiftS, get: async <T,>(path: string) => {
    const file = lower.get(path.toLowerCase());
    if (!file) throw new Error(`the fixture has no answer for ${path}`);
    return shiftTimes((await read(file)) as T, shiftS);
  } };
}
