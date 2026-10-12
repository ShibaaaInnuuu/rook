import type { Health } from "./types";

/**
 * Where API-shaped data comes from. Two sources, one interface:
 *   - http: the hosted indexer at VITE_ETCH_API (indexer/API.md). Unset by default, in which case the site reads the chain or
 *     says that nothing is live.
 *   - fixtures: the indexer's own recorded responses under /fixtures/<set>/, byte-identical copies (see SOURCE.txt), looked up
 *     through that set's index.json, which maps a request path to a file. Always labelled as a demo on screen.
 * A path that a fixture set does not have is an error, never a made-up answer.
 */
export type Source = { kind: "http" | "fixtures"; label: string; note: string; get: <T>(path: string) => Promise<T>;
  /** fixtures carry times from the moment they were recorded; this is added so countdowns in a demo are mid-count, not long over */
  shiftS: number };

export const API_BASE: string | null = ((import.meta as unknown as { env?: Record<string, string | undefined> }).env?.VITE_ETCH_API ?? "").replace(/\/+$/, "") || null;

export function httpSource(base: string, fetchFn: typeof fetch = fetch): Source {
  return { kind: "http", label: base, note: "", shiftS: 0, get: async <T>(path: string) => {
    const r = await fetchFn(`${base}${path}`, { signal: AbortSignal.timeout(8_000), headers: { accept: "application/json" } });
    if (!r.ok) throw new Error(`the indexer answered ${r.status} for ${path}`);
    return (await r.json()) as T;
  } };
}

export type FixtureIndex = { kind: string; note: string; files: Record<string, string> };
export async function fixtureSource(set: "demo" | "synthetic", nowS: number, read: (file: string) => Promise<unknown> = async (f) => (await fetch(`/fixtures/${set}/${f}`)).json()): Promise<Source> {
  const index = (await read("index.json")) as FixtureIndex;
  const lower = new Map(Object.entries(index.files).map(([k, v]) => [k.toLowerCase(), v]));
  const get = async <T>(path: string) => {
    const file = lower.get(path.toLowerCase());
    if (!file) throw new Error(`this demo has no recorded answer for ${path}`);
    return (await read(file)) as T;
  };
  const health = await get<Health>("/v1/health");
  return { kind: "fixtures", label: index.kind, note: index.note, shiftS: health.lastPassAt ? nowS - health.lastPassAt : 0, get };
}
