import { createServer, type IncomingMessage, type Server, type ServerResponse } from "node:http";
import { RANGE_KEYS, rangeOf, resOf } from "./candles.js";
import type { Views } from "./views.js";
import type { FollowerState } from "./follow.js";
import type { WrappedService } from "./wrapped/service.js";
import { ROOK_ENDPOINTS, type RookRoutes } from "./rook/routes.js";

/** docs/phase4.md §4. Read-only, no auth, no cookies, no PII. */
export const VERSION = "1";
export type ApiDeps = { views: Views; /** docs/v5/ux-and-reserves.md: wrapped collections and their proof of reserves; absent = none configured */ wrapped?: WrappedService | null; /** docs/v6/site-design.md: the Rook board (src/rook/); absent = not served */ rook?: RookRoutes | null; follower: () => FollowerState; confirmations: number; trustProxy?: boolean; perMinute?: number; burst?: number; now?: () => number;
  log?: (o: Record<string, unknown>) => void };

/** A token bucket per client address. Memory is bounded: the map is cleared when it grows past a ceiling. */
export class RateLimit {
  private readonly buckets = new Map<string, { tokens: number; at: number }>();
  constructor(private readonly perMinute: number, private readonly burst: number, private readonly now: () => number = Date.now) {}
  /** 0 when allowed, else the seconds to wait. */
  take(key: string): number {
    const t = this.now(), b = this.buckets.get(key) ?? { tokens: this.burst, at: t };
    b.tokens = Math.min(this.burst, b.tokens + ((t - b.at) / 60_000) * this.perMinute); b.at = t;
    if (this.buckets.size > 20_000) this.buckets.clear();
    this.buckets.set(key, b);
    if (b.tokens >= 1) { b.tokens -= 1; return 0; }
    return Math.max(1, Math.ceil(((1 - b.tokens) / this.perMinute) * 60));
  }
}

const intOf = (v: string | null, def: number, min: number, max: number) => { const n = Number.parseInt(v ?? "", 10); return Number.isInteger(n) ? Math.min(max, Math.max(min, n)) : def; };

export function handler(d: ApiDeps) {
  const limit = new RateLimit(d.perMinute ?? 120, d.burst ?? 40, d.now);
  return async (req: IncomingMessage, res: ServerResponse): Promise<void> => {
    const send = (status: number, body: unknown, cache = "public, max-age=5", extra: Record<string, string> = {}) => {
      const text = JSON.stringify(body);
      res.writeHead(status, { "content-type": "application/json; charset=utf-8", "access-control-allow-origin": "*", "access-control-allow-methods": "GET, HEAD, OPTIONS",
        "cache-control": status === 200 ? cache : "no-store", "x-content-type-options": "nosniff", ...extra });
      res.end(req.method === "HEAD" ? undefined : text);
    };
    try {
      if (req.method === "OPTIONS") { res.writeHead(204, { "access-control-allow-origin": "*", "access-control-allow-methods": "GET, HEAD, OPTIONS", "access-control-max-age": "86400" }); res.end(); return; }
      if (req.method !== "GET" && req.method !== "HEAD") return send(405, { error: "this API only answers GET", status: 405 }, "no-store", { allow: "GET, HEAD, OPTIONS" });
      const fwd = d.trustProxy ? String(req.headers["x-forwarded-for"] ?? "").split(",")[0].trim() : "";
      const wait = limit.take(fwd || req.socket.remoteAddress || "unknown");
      if (wait) return send(429, { error: "too many requests", status: 429 }, "no-store", { "retry-after": String(wait) });

      const url = new URL(req.url ?? "/", "http://x"), p = url.pathname.replace(/\/+$/, "") || "/";
      const lim = intOf(url.searchParams.get("limit"), 50, 1, 200), off = intOf(url.searchParams.get("offset"), 0, 0, 1_000_000);
      const listing = url.searchParams.get("listing") === null ? undefined : intOf(url.searchParams.get("listing"), -1, -1, 1e9);
      const v = d.views;
      if (p === "/v1/health") return send(200, v.health({ ...d.follower() }, d.confirmations, VERSION), "no-store");
      if (p === "/v1/stats") return send(200, v.stats());
      if (p === "/v1/coins") return send(200, v.coins(lim, off, listing));
      if (p === "/v1/listings") return send(200, v.listings());
      if (p === "/v1/acquisitions") return send(200, v.acquisitions(lim, off, listing));
      if (p === "/v1/draws") return send(200, v.draws(lim, off, listing));
      let m: RegExpMatchArray | null;
      if ((m = p.match(/^\/v1\/coins\/(0x[0-9a-fA-F]{40})$/))) { const c = await v.coin(m[1]); return c ? send(200, c) : send(404, { error: "no such coin", status: 404 }); }
      // docs/phase10.md: a coin's trades, page by page, and the chart computed from them here rather than in the browser
      if ((m = p.match(/^\/v1\/coins\/(0x[0-9a-fA-F]{40})\/trades$/))) {
        const before = url.searchParams.get("before");
        if (before !== null && !/^\d{1,12}:\d{1,6}$/.test(before)) return send(400, { error: "before must be a cursor of the form block:logIndex", status: 400 });
        return v.coinExists(m[1]) ? send(200, v.trades(m[1], lim, before)) : send(404, { error: "no such coin", status: 404 });
      }
      // docs/phase13.md §5: who holds it, folded from the coin's own Transfer logs. Largest first, paged by offset.
      if ((m = p.match(/^\/v1\/coins\/(0x[0-9a-fA-F]{40})\/holders$/))) {
        const h = v.holders(m[1], lim, off);
        return h ? send(200, h) : send(404, { error: "no such coin", status: 404 });
      }
      if ((m = p.match(/^\/v1\/coins\/(0x[0-9a-fA-F]{40})\/candles$/))) {
        // docs/phase12.md §4: `range` names a window and derives its own bucket; `res` is the older parameter and still works,
        // so the currently-deployed site does not break. A range that is not one of ours is said out loud, not quietly swapped
        // for a default — which is exactly what `res` did, and part of how a reader ended up on one candle per trade.
        const asked = url.searchParams.get("range");
        const range = rangeOf(asked);
        if (asked !== null && range === null) return send(400, { error: `range must be one of ${RANGE_KEYS.join(", ")}`, status: 400 });
        const c = v.chart(m[1], range === null ? { res: resOf(url.searchParams.get("res")) } : { range });
        return c ? send(200, c) : send(404, { error: "no such coin", status: 404 });
      }
      if ((m = p.match(/^\/v1\/listings\/(\d{1,9})$/))) { const l = await v.listing(Number(m[1])); return l ? send(200, l) : send(404, { error: "no such listing", status: 404 }); }
      if ((m = p.match(/^\/v1\/draws\/(\d{1,9})-(\d{1,9})$/))) { const x = v.draw(Number(m[1]), Number(m[2])); return x ? send(200, x) : send(404, { error: "no such draw", status: 404 }); }
      // docs/v5/ux-and-reserves.md §3. Wrapped collections and the proof of reserves. Nothing configured: said, never invented.
      const w = d.wrapped ?? null;
      if (p === "/v1/wrapped") return send(200, w ? await w.list() : { configured: false, source: null, collections: [] }, "public, max-age=30");
      if (p === "/v1/reserves") return send(200, w ? { configured: w.configured, reports: await w.reserves() } : { configured: false, reports: [] }, "public, max-age=30");
      if ((m = p.match(/^\/v1\/wrapped\/([a-z0-9-]{1,40})$/))) { const c = w ? await w.collection(m[1]) : null; return c ? send(200, c, "public, max-age=30") : send(404, { error: "no such collection", status: 404 }); }
      if ((m = p.match(/^\/v1\/wrapped\/([a-z0-9-]{1,40})\/tickets\/(\d{1,9})$/))) { const t = w ? await w.ticket(m[1], Number(m[2])) : null; return t ? send(200, t, "public, max-age=15") : send(404, { error: "no such ticket", status: 404 }); }
      if ((m = p.match(/^\/v1\/reserves\/([a-z0-9-]{1,40})$/))) { const r = w ? await w.reserve(m[1]) : null; return r ? send(200, r, "public, max-age=30") : send(404, { error: "no such collection", status: 404 }); }
      // docs/v6/offer-handins.md: the collection's offer board, read; a collection with none says so in `error`
      if ((m = p.match(/^\/v1\/handins\/([a-z0-9-]{1,40})$/))) { const r = w ? await w.handins(m[1]) : null; return r ? send(200, r, "public, max-age=15") : send(404, { error: "no such collection", status: 404 }); }
      if (d.rook) {
        const r = await d.rook(p, url);
        // docs/v6/collections-catalog.md §2.4: a picture the catalogue serves from this host; everything else is JSON as before
        if (r?.raw) { res.writeHead(r.status, { "content-type": r.raw.contentType, "content-length": String(r.raw.body.byteLength), "access-control-allow-origin": "*", "cache-control": r.cache ?? "public, max-age=3600", "x-content-type-options": "nosniff",
          // an SVG (an inscription, an on-chain token) opened on its own runs nothing and loads nothing from this host
          "content-security-policy": "default-src 'none'; img-src data:; style-src 'unsafe-inline'; sandbox" }); res.end(req.method === "HEAD" ? undefined : Buffer.from(r.raw.body)); return; }
        if (r) return send(r.status, r.body, r.cache);
      }
      if (p === "/" || p === "/v1") return send(200, { name: "etch-indexer", version: VERSION, endpoints: ["/v1/health", "/v1/stats", "/v1/coins", "/v1/coins/:address", "/v1/coins/:address/trades",
        "/v1/coins/:address/holders", "/v1/coins/:address/candles?range=5m|1h|6h|1d|all", "/v1/listings", "/v1/listings/:id", "/v1/acquisitions", "/v1/draws", "/v1/draws/:listingId-:acquisitionId",
        "/v1/wrapped", "/v1/wrapped/:slug", "/v1/wrapped/:slug/tickets/:id", "/v1/reserves", "/v1/reserves/:slug", "/v1/handins/:slug", ...(d.rook ? ROOK_ENDPOINTS : [])] });
      return send(404, { error: "no such endpoint", status: 404 });
    } catch (err) {
      d.log?.({ level: "error", event: "api-error", message: String((err as Error).message).slice(0, 200) });
      if (!res.headersSent) send(500, { error: "internal error", status: 500 }, "no-store");
    }
  };
}

/** One handler for the server's life: the rate limiter's memory lives in it. (A handler made per request would forget every client each time — a test holds this.) */
export const serve = (d: ApiDeps, port: number, host = "::"): Server => { const h = handler(d); return createServer((req, res) => void h(req, res)).listen(port, host); };
