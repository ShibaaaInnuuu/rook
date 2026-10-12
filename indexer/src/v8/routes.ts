import { readFileSync } from "node:fs";
import type { RookAnswer, RookRoutes } from "../rook/routes.js";
import { servedStatus } from "./ethReserve.js";
import { parseV8Config, V8Service, type V8Deps } from "./service.js";
import type { CanisterRecords } from "./types.js";

/**
 * The v8 endpoints (docs/v8/proof-of-reserves.md, redemption-draw.md), matched here so main.ts composes one function with the
 * board's (api.ts is not touched):
 *   GET /v1/v8                     the configured collections
 *   GET /v1/v8/reserves/:slug      the Ethereum side's proof of reserves; `served` is its status as of this answer (stale → red)
 *   GET /v1/v8/draws/:slug         the vault's draw state, every draw's proof, the snapshot's hash
 *   GET /v1/v8/snapshots/:slug     the metadata snapshot itself (hash its canonical JSON to check it)
 */
export const V8_ENDPOINTS = ["/v1/v8", "/v1/v8/reserves/:slug", "/v1/v8/draws/:slug", "/v1/v8/snapshots/:slug"];

export function v8Routes(svc: V8Service, now: () => number = () => Math.floor(Date.now() / 1000)): RookRoutes {
  return async (p): Promise<RookAnswer | null> => {
    if (p !== "/v1/v8" && !p.startsWith("/v1/v8/")) return null;
    if (p === "/v1/v8") return { status: 200, body: { collections: svc.slugs(), endpoints: V8_ENDPOINTS }, cache: "public, max-age=60" };
    const m = p.match(/^\/v1\/v8\/(reserves|draws|snapshots)\/([a-z0-9-]{1,40})$/);
    if (!m) return { status: 404, body: { error: "no such endpoint", status: 404 } };
    const [, what, slug] = m;
    if (what === "reserves") {
      const r = await svc.reserve(slug);
      return r ? { status: 200, body: { ...r, served: servedStatus(r, now()) }, cache: "public, max-age=30" } : { status: 404, body: { error: "no such collection", status: 404 } };
    }
    if (what === "draws") { const r = await svc.draws(slug); return r ? { status: 200, body: r, cache: "public, max-age=30" } : { status: 404, body: { error: "no such collection", status: 404 } }; }
    const s = svc.snapshot(slug);
    return s ? { status: 200, body: s.snapshot, cache: "public, max-age=3600" } : { status: 404, body: { error: "no snapshot for this collection", status: 404 } };
  };
}

/**
 * From the environment: V8_CONFIG=<file.json> (parseV8Config); CANISTER_RECORDS_URL, a JSON object of CanisterRecords by slug that
 * the ops reconciler serves (ops/src/ethRecords.ts), read with a deadline and a size cap. Null when V8_CONFIG is unset.
 */
export function v8FromEnv(env: Record<string, string | undefined>, log: (o: Record<string, unknown>) => void): { svc: V8Service; routes: RookRoutes } | null {
  if (!env.V8_CONFIG) return null;
  const config = parseV8Config(JSON.parse(readFileSync(env.V8_CONFIG, "utf8")));
  const url = env.CANISTER_RECORDS_URL;
  const canisterRecords: V8Deps["canisterRecords"] = url ? async (c) => {
    const r = await fetch(url, { signal: AbortSignal.timeout(10_000), headers: { accept: "application/json" } });
    if (!r.ok) return null;
    const text = await r.text();
    if (text.length > 2_000_000) return null;
    return ((JSON.parse(text) as Record<string, CanisterRecords>)[c.slug] ?? null);
  } : null;
  const svc = new V8Service({ config, fetch, canisterRecords, log });
  return { svc, routes: v8Routes(svc) };
}
