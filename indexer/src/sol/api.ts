/** The read-only HTTP API (docs/v9/indexer-solana.md §2). GET only, CORS open, no auth, rate-limited per client address. */
import { createServer, type IncomingMessage, type Server, type ServerResponse } from "node:http";
import { RateLimit } from "../api.js";
import { isAddress } from "./bytes.js";
import { KINDS } from "./memo.js";
import { chartQuoteOf, resOf, type SolService } from "./service.js";

export const SOL_VERSION = "sol-2";
export const SOL_ENDPOINTS = ["/v1/health", "/v1/rook/coins?sort=new|graduating|graduated|trending|mcap|active&collection=&q=&limit=&offset=", "/v1/rook/board",
  "/v1/rook/coins/:mint", "/v1/rook/coins/:mint/chart?res=1m|5m|15m|1h|1d&quote=wx|sol|usd", "/v1/rook/coins/:mint/trades?limit=&before=", "/v1/rook/coins/:mint/holders?limit=&offset=",
  "/v1/rook/activity?limit=", "/v1/rook/collections", "/v1/rook/wallets/:address",
  "/v1/rook/desk/events?coll=<index|slug|->&kind=mint|burn|refund|redeem|remint|prize|sweep|coin|close|other&since=&limit=&before=", "/v1/rook/desk/totals?coll="];
export type Answer = { status: number; body: unknown; cache?: string };
export type SolApiDeps = { svc: SolService; health: () => unknown; trustProxy?: boolean; perMinute?: number; burst?: number; log?: (o: Record<string, unknown>) => void };

const intOf = (v: string | null, def: number, min: number, max: number) => { const n = Number.parseInt(v ?? "", 10); return Number.isInteger(n) ? Math.min(max, Math.max(min, n)) : def; };
const B58 = "([1-9A-HJ-NP-Za-km-z]{32,44})";
const nf = (what: string): Answer => ({ status: 404, body: { error: `no such ${what}`, status: 404 } });

/** Pure routing: path + query → answer. The tests call this directly. */
export function route(d: Pick<SolApiDeps, "svc" | "health">, p: string, url: URL): Answer {
  const svc = d.svc, q = url.searchParams;
  const lim = intOf(q.get("limit"), 50, 1, 200), off = intOf(q.get("offset"), 0, 0, 1_000_000);
  if (p === "/v1/health") return { status: 200, body: d.health(), cache: "no-store" };
  if (p === "/" || p === "/v1") return { status: 200, body: { name: "rook-indexer-sol", version: SOL_VERSION, endpoints: SOL_ENDPOINTS } };
  if (p === "/v1/rook/coins") {
    const coll = q.get("collection"), text = q.get("q");
    if (coll !== null && !/^[a-z0-9-]{1,40}$/.test(coll)) return { status: 400, body: { error: "collection must be a slug", status: 400 } };
    return { status: 200, body: svc.list({ sort: q.get("sort"), collection: coll, q: text ? text.slice(0, 64) : null, limit: lim, offset: off }), cache: "public, max-age=5" };
  }
  if (p === "/v1/rook/board") return { status: 200, body: svc.board(), cache: "public, max-age=10" };
  if (p === "/v1/rook/activity") return { status: 200, body: svc.activity(lim), cache: "public, max-age=5" };
  if (p === "/v1/rook/collections") return { status: 200, body: svc.collections(), cache: "public, max-age=15" };
  if (p === "/v1/rook/desk/events" || p === "/v1/rook/desk/totals") {
    // W13: the canister's memo-labelled transactions. `coll` is the canister's collection index, a slug, or `-` for memos naming none
    const c = q.get("coll");
    let coll: number | null = null;
    if (c !== null) {
      if (c === "-") coll = -1;
      else if (!/^([a-z0-9-]{1,40})$/.test(c)) return { status: 400, body: { error: "coll must be a collection index, a slug or -", status: 400 } };
      else { coll = svc.collOf(c); if (coll === null) return { status: 404, body: { error: `no such collection: ${c}`, status: 404 } }; }
    }
    if (p === "/v1/rook/desk/totals") return { status: 200, body: svc.deskTotals(coll), cache: "public, max-age=15" };
    const kind = q.get("kind");
    if (kind !== null && !/^[a-z]{1,16}$/.test(kind)) return { status: 400, body: { error: `kind must be one of ${KINDS.join(", ")}`, status: 400 } };
    const since = q.get("since");
    if (since !== null && !/^\d{1,12}$/.test(since)) return { status: 400, body: { error: "since must be a unix time in seconds", status: 400 } };
    const before = q.get("before");
    if (before !== null && !/^\d{1,12}:\d{1,12}$/.test(before)) return { status: 400, body: { error: "before must be the `next` of a previous page", status: 400 } };
    return { status: 200, body: svc.deskEvents({ coll, kind, since: since === null ? null : Number(since), limit: lim, before }), cache: "public, max-age=5" };
  }
  let m: RegExpMatchArray | null;
  if ((m = p.match(new RegExp(`^/v1/rook/coins/${B58}(/chart|/trades|/holders)?$`)))) {
    const mint = m[1];
    if (!isAddress(mint)) return { status: 400, body: { error: "not a Solana address", status: 400 } };
    if (!m[2]) { const c = svc.coin(mint); return c ? { status: 200, body: c, cache: "public, max-age=5" } : nf("coin"); }
    if (m[2] === "/chart") {
      const res = resOf(q.get("res") ?? "5m");
      if (!res) return { status: 400, body: { error: "res must be one of 1m, 5m, 15m, 1h, 1d", status: 400 } };
      const quote = chartQuoteOf(q.get("quote"));
      if (quote === undefined) return { status: 400, body: { error: "quote must be one of wx, sol, usd", status: 400 } };
      const c = svc.chart(mint, res, quote); return c ? { status: 200, body: c, cache: "public, max-age=5" } : nf("coin");
    }
    if (m[2] === "/trades") {
      const before = q.get("before");
      if (before !== null && !/^\d{1,12}:\d{1,7}$/.test(before)) return { status: 400, body: { error: "before must be the `next` of a previous page", status: 400 } };
      const t = svc.trades(mint, lim, before); return t ? { status: 200, body: t, cache: "public, max-age=5" } : nf("coin");
    }
    const h = svc.holders(mint, intOf(q.get("limit"), 20, 1, 100), off); return h ? { status: 200, body: h, cache: "public, max-age=30" } : nf("coin");
  }
  if ((m = p.match(new RegExp(`^/v1/rook/wallets/${B58}$`)))) {
    if (!isAddress(m[1])) return { status: 400, body: { error: "not a Solana address", status: 400 } };
    return { status: 200, body: svc.wallet(m[1]), cache: "public, max-age=15" };
  }
  return nf("endpoint");
}

export function handler(d: SolApiDeps) {
  const limit = new RateLimit(d.perMinute ?? 240, d.burst ?? 60);
  return (req: IncomingMessage, res: ServerResponse): void => {
    const send = (status: number, body: unknown, cache = "public, max-age=5", extra: Record<string, string> = {}) => {
      res.writeHead(status, { "content-type": "application/json; charset=utf-8", "access-control-allow-origin": "*", "access-control-allow-methods": "GET, HEAD, OPTIONS",
        "cache-control": status === 200 ? cache : "no-store", "x-content-type-options": "nosniff", ...extra });
      res.end(req.method === "HEAD" ? undefined : JSON.stringify(body));
    };
    try {
      if (req.method === "OPTIONS") { res.writeHead(204, { "access-control-allow-origin": "*", "access-control-allow-methods": "GET, HEAD, OPTIONS", "access-control-max-age": "86400" }); res.end(); return; }
      if (req.method !== "GET" && req.method !== "HEAD") return send(405, { error: "this API only answers GET", status: 405 }, "no-store", { allow: "GET, HEAD, OPTIONS" });
      const fwd = d.trustProxy ? String(req.headers["x-forwarded-for"] ?? "").split(",")[0].trim() : "";
      const wait = limit.take(fwd || req.socket.remoteAddress || "unknown");
      if (wait) return send(429, { error: "too many requests", status: 429 }, "no-store", { "retry-after": String(wait) });
      const url = new URL(req.url ?? "/", "http://x"), p = url.pathname.replace(/\/+$/, "") || "/";
      const a = route(d, p, url);
      send(a.status, a.body, a.cache);
    } catch (err) {
      d.log?.({ level: "error", event: "api-error", message: String((err as Error).message).slice(0, 200) });
      if (!res.headersSent) send(500, { error: "internal error", status: 500 }, "no-store");
    }
  };
}
export const serveSol = (d: SolApiDeps, port: number, host = "::"): Server => createServer(handler(d)).listen(port, host);
