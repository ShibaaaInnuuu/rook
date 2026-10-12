import { createServer, type IncomingMessage, type Server } from "node:http";
import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { createHash, timingSafeEqual } from "node:crypto";
import type { PassReport } from "./keeper.js";

/**
 * docs/v6/p1-services.md §1.6, audit E-14. What the host and the public can ask the keeper. GETs only; nothing changes:
 *   /healthz            200 when the last pass ended within three intervals with no refusal in force and no alert in force for
 *                       KEEPER_ALERT_PASSES passes in a row (audit R2-S8; an alert stays in force KEEPER_ALERT_LATCH_S, at least
 *                       alertLatchFloorMs, after the last pass that raised it, R3-check L-G, R4-check L-S2), else 503; its body names the alert KINDS (never texts),
 *                       which the monitor's liveness poll puts in its page
 *   /status             a PUBLIC SUMMARY: mode, chain, the keeper's address, passes, the last pass, how many refusals and alerts
 *   /status/full        the whole state (pending transaction, history, refusal and alert texts, the last pass), only with
 *                       `Authorization: Bearer <KEEPER_STATUS_TOKEN>`; off when no token is configured. No wildcard CORS.
 *   /snapshots/<file>   the published holder snapshots that draws point at (docs/snapshot-v1.md "Version 2"): the URI a draw carries
 *   /                   a one-line pointer to the above
 * The full view hands out timings (a pending nonce, refusals) an attacker could use (E-04, E-10), so it is not public.
 * Everything served is built from the report and the state; no key is ever in either (test/service-keeper.test.ts).
 */
export type Status = {
  mode: string; chainId: number; keeper: string; rpc: { host: string; ok: boolean }[]; startedAt: string; interval: number;
  passes: number; lastPassAt: string | null; lastPassMs: number | null; consecutiveFailures: number; lastError: { at: string; message: string } | null;
  refusals: string[]; alerts: string[]; drawRule: string; gas: { day: string; spentOrReservedWei: string; capWei: string; perTxCapWei: string; maxGasPerTx: string; maxFeePerGasWei: string };
  pending: unknown; history: unknown[]; last: PassReport | null;
  /** consecutive passes that ended with at least one alert, and how many make /healthz 503 (R2-S8) */
  alertPasses: number; alertPassesLimit: number;
};

/** The monitor's liveness rule (monitor DOWN_MS): a /healthz not 200 for this long is paged; and the slack for its poll. */
export const MONITOR_DOWN_S = 600;
export const MONITOR_POLL_SLACK_S = 120;

/**
 * R4-check L-S2: the shortest latch that still pages a one-pass alert at this interval. /healthz turns 503 at the `passes`-th pass
 * in force (`t0 + (passes − 1) × interval`) and stays 503 until the first pass at or after `t0 + latch`, so a latch of
 * `(passes + 1) × interval + DOWN + slack` keeps it 503 for at least `2 × interval + DOWN + slack`: longer than the monitor's
 * 10 minutes plus a poll, at every allowed interval (15–1,800 s).
 */
export const alertLatchFloorMs = (intervalS: number, passes: number): number => ((passes + 1) * intervalS + MONITOR_DOWN_S + MONITOR_POLL_SLACK_S) * 1000;

/**
 * R3-check L-G: an alert raised once (`draw-slot-missed`, `draw-not-opened`) or one that flaps never stayed in force for
 * KEEPER_ALERT_PASSES passes in a row, so it never turned /healthz 503 and was never paged. Every alert raised now stays in force
 * for `latchMs` after the last pass that raised it (default KEEPER_ALERT_LATCH_S = 1,200 s, longer than the monitor's 10-minute
 * DOWN_MS; never shorter than `alertLatchFloorMs` for the service's interval, R4-check L-S2), and the passes are counted on what
 * is in force. `pass(alerts, nowMs)` returns { inForce, alertPasses }.
 */
export class AlertLatch {
  private readonly until = new Map<string, number>();
  private passes = 0;
  constructor(private readonly latchMs: number) {}
  pass(alerts: readonly string[], nowMs: number): { inForce: string[]; alertPasses: number } {
    for (const a of alerts) this.until.set(a, nowMs + this.latchMs);
    for (const [a, t] of this.until) if (t <= nowMs && !alerts.includes(a)) this.until.delete(a);
    const inForce = [...new Set([...alerts, ...this.until.keys()])];
    this.passes = inForce.length ? this.passes + 1 : 0;
    return { inForce, alertPasses: this.passes };
  }
}

/** An alert's kind: the text before its first colon (`tx-stuck`, `claim-stuck`, `draw-slot-missed` …). Never the text itself. */
export const alertKinds = (alerts: readonly string[]): string[] => [...new Set(alerts.map((a) => (/^([a-z0-9-]{1,40}):/.exec(a)?.[1] ?? "alert")))].sort();

export const healthy = (s: Status, nowMs: number): boolean =>
  s.refusals.length === 0 && s.consecutiveFailures < 3 && s.lastPassAt !== null && nowMs - Date.parse(s.lastPassAt) <= Math.max(90, s.interval * 3) * 1000
  && !(s.alerts.length > 0 && (s.alertPasses ?? 0) >= (s.alertPassesLimit ?? 3));

/** What anyone may see: nothing that times an attack (no pending nonce, no refusal texts, no history). */
export const summaryOf = (s: Status, nowMs: number) => ({ mode: s.mode, chainId: s.chainId, keeper: s.keeper, startedAt: s.startedAt, interval: s.interval,
  passes: s.passes, lastPassAt: s.lastPassAt, healthy: healthy(s, nowMs), refusals: s.refusals.length, alerts: s.alerts.length,
  alertPasses: s.alertPasses ?? 0, pending: s.pending !== null,
  drawRule: s.drawRule, full: "/status/full (Authorization: Bearer …)" });

const SNAPSHOT_FILE = /^snapshot-0x[0-9a-f]{40}-\d+-weighted\.json$/;
const json = (v: unknown) => JSON.stringify(v, (_k, x) => (typeof x === "bigint" ? String(x) : x), 1);

/** Constant-time comparison of the bearer token (both hashed first, so neither length leaks). */
export function authorized(req: Pick<IncomingMessage, "headers">, token: string | null): boolean {
  if (!token) return false;
  const m = /^Bearer (.+)$/.exec(String(req.headers.authorization ?? ""));
  if (!m) return false;
  return timingSafeEqual(createHash("sha256").update(m[1]).digest(), createHash("sha256").update(token).digest());
}

export function serveStatus(status: () => Status, snapshotDir: string, port: number, scrub: (s: string) => string, host = "::", token: string | null = null): Server {
  return createServer((req, res) => {
    if (req.method !== "GET" && req.method !== "HEAD") { res.writeHead(405, { allow: "GET, HEAD" }); res.end(); return; }
    const path = (req.url ?? "/").split("?")[0];
    const send = (code: number, type: string, body: string, cors = true) => {
      res.writeHead(code, { "content-type": type, "cache-control": "no-store", ...(cors ? { "access-control-allow-origin": "*" } : {}) });
      res.end(req.method === "HEAD" ? undefined : body);
    };
    if (path === "/healthz") {
      const s = status(); const ok = healthy(s, Date.now());
      send(ok ? 200 : 503, "application/json", scrub(json({ ok, lastPassAt: s.lastPassAt, refusals: s.refusals.length, alerts: s.alerts.length, alertKinds: alertKinds(s.alerts),
        alertPasses: s.alertPasses ?? 0, alertPassesLimit: s.alertPassesLimit ?? 3 }))); return;
    }
    if (path === "/status") { send(200, "application/json", scrub(json(summaryOf(status(), Date.now())))); return; }
    if (path === "/status/full") {
      if (!token) { send(404, "text/plain", "the full status is off: no KEEPER_STATUS_TOKEN is configured\n", false); return; }
      if (!authorized(req, token)) { res.writeHead(401, { "www-authenticate": "Bearer", "cache-control": "no-store" }); res.end(); return; }
      send(200, "application/json", scrub(json(status())), false); return;
    }
    if (path.startsWith("/snapshots/")) {
      const name = path.slice("/snapshots/".length);
      if (!SNAPSHOT_FILE.test(name) || !existsSync(join(snapshotDir, name))) { send(404, "text/plain", "no such snapshot\n"); return; }
      send(200, "application/json", readFileSync(join(snapshotDir, name), "utf8")); return;
    }
    if (path === "/") { send(200, "text/plain", "rook keeper: GET /status (summary), /status/full (bearer token), /healthz, /snapshots/<file>\n"); return; }
    send(404, "text/plain", "not found\n");
  }).listen(port, host);
}
