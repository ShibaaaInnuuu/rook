import { createServer, type Server } from "node:http";

/** docs/phase4.md §5. What a host's health check asks. It answers GET /healthz and nothing else, and it changes nothing. */
export type Health = { mode: "dry-run"; chainId: number | null; startedAt: string; passes: number; lastPassAt: string | null; lastPassMs: number | null; listingsRead: number;
  lastError: { at: string; kind: string } | null; consecutiveFailures: number };

export const newHealth = (now: () => Date = () => new Date()): Health => ({ mode: "dry-run", chainId: null, startedAt: now().toISOString(), passes: 0, lastPassAt: null, lastPassMs: null,
  listingsRead: 0, lastError: null, consecutiveFailures: 0 });

/** Healthy: it has completed a pass recently enough (three intervals), and is not in a run of failures. */
export const healthy = (h: Health, watchS: number, now: () => Date = () => new Date()): boolean =>
  h.consecutiveFailures < 3 && h.lastPassAt !== null && now().getTime() - Date.parse(h.lastPassAt) <= Math.max(60, watchS * 3) * 1000;

export function serveHealth(h: () => Health, watchS: number, port: number, host = "::"): Server {
  return createServer((req, res) => {
    if (req.method !== "GET" && req.method !== "HEAD") { res.writeHead(405, { allow: "GET, HEAD" }); res.end(); return; }
    if ((req.url ?? "").split("?")[0] !== "/healthz") { res.writeHead(404); res.end(); return; }
    const ok = healthy(h(), watchS);
    res.writeHead(ok ? 200 : 503, { "content-type": "application/json", "cache-control": "no-store" });
    res.end(req.method === "HEAD" ? undefined : JSON.stringify({ ok, ...h() }));
  }).listen(port, host);
}
