import { resOf } from "./board.js";
import type { CatalogService } from "./catalog.js";
import type { RookService } from "./service.js";

/** The board's endpoints, matched here so api.ts wires one line. Null: not ours. */
/** `raw`: not JSON — a picture the catalogue fetched and checked, served from this host with its own content type. */
export type RookAnswer = { status: number; body: unknown; cache?: string; raw?: { contentType: string; body: Uint8Array } };
export type RookRoutes = (p: string, url: URL) => Promise<RookAnswer | null>;
export const ROOK_ENDPOINTS = ["/v1/rook/board", "/v1/rook/activity", "/v1/rook/coins/:address/chart?res=1m|5m|15m|1h|1d", "/v1/rook/coins/:address/trades", "/v1/rook/coins/:address/holders", "/v1/rook/wallets/:address", "/v1/rook/catalog", "/v1/rook/catalog/:slug", "/v1/rook/catalog/:slug/image", "/v1/rook/catalog/:slug/items/:i/image"];

const intOf = (v: string | null, def: number, min: number, max: number) => { const n = Number.parseInt(v ?? "", 10); return Number.isInteger(n) ? Math.min(max, Math.max(min, n)) : def; };

/** `catalog` (docs/v6/collections-catalog.md): the directory of collections whether or not Rook lists them; null = not served here. */
export function rookRoutes(svc: RookService, catalog: CatalogService | null = null): RookRoutes {
  return async (p, url) => {
    if (!p.startsWith("/v1/rook")) return null;
    const lim = intOf(url.searchParams.get("limit"), 50, 1, 200), off = intOf(url.searchParams.get("offset"), 0, 0, 1_000_000);
    if (p === "/v1/rook/board") return { status: 200, body: await svc.board(), cache: "public, max-age=10" };
    if (p === "/v1/rook/activity") return { status: 200, body: await svc.activity(lim), cache: "public, max-age=5" };
    let m: RegExpMatchArray | null;
    if ((m = p.match(/^\/v1\/rook\/coins\/(0x[0-9a-fA-F]{40})\/chart$/))) {
      const res = resOf(url.searchParams.get("res") ?? "5m");
      if (!res) return { status: 400, body: { error: "res must be one of 1m, 5m, 15m, 1h, 1d", status: 400 } };
      const c = await svc.chart(m[1], res);
      return c ? { status: 200, body: c, cache: "public, max-age=5" } : { status: 404, body: { error: "no such coin", status: 404 } };
    }
    if ((m = p.match(/^\/v1\/rook\/coins\/(0x[0-9a-fA-F]{40})\/trades$/))) {
      const before = url.searchParams.get("before");
      if (before !== null && !/^\d{1,12}:\d{1,6}$/.test(before)) return { status: 400, body: { error: "before must be a cursor of the form block:logIndex", status: 400 } };
      const t = await svc.trades(m[1], lim, before);
      return t ? { status: 200, body: t, cache: "public, max-age=5" } : { status: 404, body: { error: "no such coin", status: 404 } };
    }
    if ((m = p.match(/^\/v1\/rook\/coins\/(0x[0-9a-fA-F]{40})\/holders$/))) {
      const h = await svc.holders(m[1], lim, off);
      return h ? { status: 200, body: h, cache: "public, max-age=15" } : { status: 404, body: { error: "no such coin", status: 404 } };
    }
    if ((m = p.match(/^\/v1\/rook\/wallets\/(0x[0-9a-fA-F]{40})$/))) return { status: 200, body: await svc.wallet(m[1]), cache: "public, max-age=10" };
    // docs/v6/collections-catalog.md: the catalogue answers from its snapshot at once and refreshes behind the answer; a minute's cache in front is plenty
    if (p === "/v1/rook/catalog") return catalog ? { status: 200, body: await catalog.list(), cache: "public, max-age=60" } : { status: 503, body: { error: "the collections catalogue is not served by this indexer", status: 503 } };
    // a collection's picture, or a sample item's, fetched by the indexer from the URL the catalogue holds and cached (docs/v6/collections-catalog.md §2.4); none: 404, and the site draws a mark
    if ((m = p.match(/^\/v1\/rook\/catalog\/([a-zA-Z0-9_-]{1,64})(?:\/image|\/items\/(\d{1,2})\/image)$/))) {
      if (!catalog) return { status: 503, body: { error: "the collections catalogue is not served by this indexer", status: 503 } };
      const pic = await catalog.picture(m[1], m[2] === undefined ? null : Number(m[2]));
      return pic ? { status: 200, body: null, cache: "public, max-age=86400", raw: pic } : { status: 404, body: { error: "no picture for this collection", status: 404 }, cache: "public, max-age=600" };
    }
    if ((m = p.match(/^\/v1\/rook\/catalog\/([a-zA-Z0-9_-]{1,64})$/))) {
      if (!catalog) return { status: 503, body: { error: "the collections catalogue is not served by this indexer", status: 503 } };
      const c = await catalog.one(m[1]);
      return c ? { status: 200, body: c, cache: "public, max-age=60" } : { status: 404, body: { error: "no such collection in the catalogue", status: 404 } };
    }
    return { status: 404, body: { error: "no such endpoint", status: 404 } };
  };
}
