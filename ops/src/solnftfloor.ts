import { readFileSync } from "node:fs";
import type { Log } from "./log.js";

/**
 * The Solana NFT collections' floor evidence (docs/v9/solana-nfts-canister.md §5): for every Solana collection of the launch file
 * (deployments/launch/solana-collections.json) that the canister has OPEN (`coins_collections`, matched by its wX symbol), the runner
 * fetches Magic Eden's two keyless answers and hands them in as they are:
 *   - the listings: `GET /v2/collections/<symbol>/listings?listingAggMode=true` (M2, mmm and Tensor aggregated), trimmed to the cheapest
 *     `MAX_CANDIDATES` by Magic Eden's own `price` and to `MAX_LISTINGS_BYTES` (the canister keeps no more);
 *   - the stats: `GET /v2/collections/<symbol>/stats` (its `floorPrice` is the reference that only vetoes), at most `MAX_STATS_BYTES`.
 * Both are hints: the canister reads every listing's escrow and membership on chain at `finalized` and refuses a floor that disagrees
 * with the reference (`solnftfloor.rs`). Bounded: one collection at a time, `GAP_MS` between Magic Eden calls (its public limit is
 * about 120 a minute), each call with a deadline, a size cap and no redirect (`guarded`), each collection at most every `everyS`.
 */

/** The canister's endpoint (the Solana NFT flow, `src/solnftflow/api.rs`): operator-only,
 *  `sol_nft_evidence : (nat16, text, text) -> (Result)`, as in etch_autobuyer.did. */
export const SOL_NFT_EVIDENCE_METHOD = "sol_nft_evidence";
export type SolNftEvidenceSink = { sol_nft_evidence(coll: number, listingsJson: string, statsJson: string): Promise<{ Ok: string } | { Err: string }> };
export type CollectionsSource = { coins_collections(): Promise<{ index: number; symbol: string; stage: string }[]> };

export const ME_API_DEFAULT = "https://api-mainnet.magiceden.dev";
/** The canister's bounds on a hand-in (solnftflow/mod.rs: MAX_CANDIDATES, MAX_LISTINGS_BYTES, MAX_STATS_BYTES). */
export const MAX_CANDIDATES = 40;
export const MAX_LISTINGS_BYTES = 64 * 1024;
export const MAX_STATS_BYTES = 4 * 1024;
/** Between two Magic Eden calls (its public endpoints allow about 120 a minute). */
export const GAP_MS = 600;
export const SOL_NFT_EVERY_DEFAULT_S = 1_800;
const TIMEOUT_MS = 15_000;
const MAX_BODY = 4_000_000;

export type SolNftCollection = { key: string; name: string; symbol: string; meSymbol: string };

/** The launch file's Solana collections: key, wX symbol, Magic Eden symbol (checked: a path segment, nothing else). */
export function solNftCollections(file: { collections?: { key?: unknown; name?: unknown; symbol?: unknown; magic_eden_symbol?: unknown }[] }): SolNftCollection[] {
  return (file.collections ?? []).map((c) => {
    const me = String(c.magic_eden_symbol ?? "");
    if (!/^[a-z0-9_-]{1,64}$/.test(me)) throw new Error(`${String(c.key)}: magic_eden_symbol "${me}" is not a Magic Eden symbol`);
    return { key: String(c.key), name: String(c.name), symbol: String(c.symbol), meSymbol: me };
  });
}
export const loadSolNftCollections = (path: string) => solNftCollections(JSON.parse(readFileSync(path, "utf8")));

/** The listings as the canister keeps them: the cheapest MAX_CANDIDATES by `price`, then fewer until the text fits MAX_LISTINGS_BYTES. */
export function trimListings(text: string): string {
  const v = JSON.parse(text) as unknown;
  if (!Array.isArray(v)) throw new Error("the listings are not an array");
  const price = (x: unknown) => { const p = Number((x as { price?: unknown })?.price); return Number.isFinite(p) ? p : Number.MAX_VALUE; };
  let rows = [...v].sort((a, b) => price(a) - price(b)).slice(0, MAX_CANDIDATES);
  let out = JSON.stringify(rows);
  while (Buffer.byteLength(out) > MAX_LISTINGS_BYTES && rows.length > 1) { rows = rows.slice(0, rows.length - 1); out = JSON.stringify(rows); }
  if (Buffer.byteLength(out) > MAX_LISTINGS_BYTES) throw new Error(`one listing is over ${MAX_LISTINGS_BYTES} bytes`);
  return out;
}

export type SolNftFloorDeps = {
  buyer: SolNftEvidenceSink & CollectionsSource;
  fetch: typeof fetch;
  log: Log;
  collections: SolNftCollection[];
  api?: string;
  everyS?: number;
  nowS?: () => number;
  sleep?: (ms: number) => Promise<void>;
  /** per collection key, when it was last handed in (seconds) */
  last?: Map<string, number>;
  /** per collection key, when the canister last took a hand-in (seconds), for /healthz */
  accepted?: Map<string, number>;
};

/** Magic Eden's answer, at most `cap` bytes: audit C4-09, the body is streamed and the read stops (cancelled) once the cap is passed,
 *  after a Content-Length over the cap refuses it unread; never the whole body first. */
export async function guarded(d: Pick<SolNftFloorDeps, "fetch">, url: string, cap = MAX_BODY): Promise<string> {
  const ac = new AbortController(), signal = AbortSignal.any([ac.signal, AbortSignal.timeout(TIMEOUT_MS)]);
  const r = await d.fetch(url, { redirect: "manual", headers: { accept: "application/json" }, signal });
  const path = new URL(url).pathname, over = () => new Error(`Magic Eden ${path}: the answer is over ${cap} bytes`);
  const drop = async () => { ac.abort(); await r.body?.cancel().catch(() => {}); };
  if (!r.ok) { await drop(); throw new Error(`Magic Eden ${path}: HTTP ${r.status}`); }
  const len = r.headers.get("content-length");
  if (len !== null && Number(len) > cap) { await drop(); throw over(); }
  if (!r.body) return "";
  const reader = r.body.getReader(), chunks: Uint8Array[] = [];
  let n = 0;
  for (;;) {
    const { done, value } = await reader.read();
    if (done) break;
    n += value.byteLength;
    if (n > cap) { ac.abort(); await reader.cancel().catch(() => {}); throw over(); }
    chunks.push(value);
  }
  return new TextDecoder().decode(Buffer.concat(chunks));
}

/** One pass. True when the canister took a hand-in (the runner then ticks again at once, so the floor is read now). */
export async function solNftFloorOnce(d: SolNftFloorDeps): Promise<boolean> {
  if (!d.collections.length) return false;
  const now = d.nowS ?? (() => Math.floor(Date.now() / 1000));
  const every = d.everyS ?? SOL_NFT_EVERY_DEFAULT_S, last = d.last ?? new Map<string, number>();
  const sleep = d.sleep ?? ((ms: number) => new Promise<void>((ok) => setTimeout(ok, ms)));
  const api = (d.api ?? ME_API_DEFAULT).replace(/\/$/, "");
  // only what the canister has open: a collection opens on its first request (operator decision 2026-10-12)
  let open: Map<string, number>;
  try { open = new Map((await d.buyer.coins_collections()).filter((c) => c.stage === "Open").map((c) => [c.symbol, c.index])); }
  catch (err) { d.log("warn", "sol-nft-floor-no-collections", { error: String((err as Error).message).slice(0, 200) }); return false; }
  let took = false, calls = 0;
  for (const c of d.collections) {
    const coll = open.get(c.symbol);
    if (coll === undefined) continue;
    if ((last.get(c.key) ?? -Infinity) + every > now()) continue;
    last.set(c.key, now());
    let listings: string, stats: string;
    try {
      if (calls++) await sleep(GAP_MS);
      listings = trimListings(await guarded(d, `${api}/v2/collections/${c.meSymbol}/listings?listingAggMode=true`));
      await sleep(GAP_MS); calls++;
      stats = await guarded(d, `${api}/v2/collections/${c.meSymbol}/stats`);
      if (Buffer.byteLength(stats) > MAX_STATS_BYTES) throw new Error(`the stats are over ${MAX_STATS_BYTES} bytes`);
      JSON.parse(stats);
    } catch (err) { d.log("warn", "sol-nft-floor-unread", { coll, key: c.key, error: String((err as Error).message).slice(0, 200) }); continue; }
    try {
      const r = await d.buyer.sol_nft_evidence(coll, listings, stats);
      if ("Ok" in r) { took = true; d.accepted?.set(c.key, now()); d.log("info", "sol-nft-floor-handed-in", { coll, key: c.key, answer: r.Ok }); }
      else d.log("warn", "sol-nft-floor-not-taken", { coll, key: c.key, error: r.Err });
    } catch (err) { d.log("warn", "sol-nft-floor-call-failed", { coll, key: c.key, error: String((err as Error).message).slice(0, 200) }); }
  }
  return took;
}
