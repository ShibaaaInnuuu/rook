// The other services' heartbeats (audit E-08; docs/audit/2026-09-26/fixes-W2-monitor.md §1.5). Railway reads a service's
// /healthz only when it deploys it; after that nothing would notice a keeper, runner, reconciler or indexer that is down or
// refusing. The monitor asks each configured URL every poll: not 200 for DOWN_MS is a page, repeated every 6 h, and its return
// is said once.
import { host } from "./abi.mjs";
import { redactUrl } from "./config.mjs";

export const DOWN_MS = 10 * 60 * 1000;

/**
 * R2-S8: the keeper's (and the resolver's) /healthz turns 503 when its pass alerts have been in force for a few passes, with the
 * alert kinds in a JSON body. The page names them: ", alerts: tx-stuck, claim-stuck". Only kinds (short tokens) are repeated,
 * never free text; a body that is not such JSON adds nothing.
 */
async function alertKinds(r) {
  try {
    const t = typeof r.text === "function" ? (await r.text()).slice(0, 4096) : "";
    const j = JSON.parse(t);
    const kinds = [j.alertKinds, j.kinds, Array.isArray(j.alerts) ? j.alerts : null].find(Array.isArray) ?? [];
    const clean = kinds.map((k) => String(k).split(":")[0].replace(/[^a-z0-9_-]/gi, "").slice(0, 40)).filter(Boolean).slice(0, 12);
    const refusals = Number.isFinite(j.refusals) && j.refusals > 0 ? `, ${j.refusals} refusal(s)` : "";
    return clean.length ? `, alerts: ${[...new Set(clean)].join(", ")}${refusals}` : refusals;
  } catch { return ""; }
}

/** cfg.liveness: [{ name, url }]. prev: { [name]: { downSince, error, lastOk } }. Returns { alerts, next }. */
export async function checkLiveness({ cfg, prev = {}, now = Date.now(), http = fetch }) {
  const alerts = [], next = {};
  for (const s of cfg.liveness ?? []) {
    const p = { ...(prev[s.name] ?? {}) };
    let error = null;
    try {
      const r = await http(s.url, { signal: AbortSignal.timeout(15_000) });
      if (r.status !== 200) error = `HTTP ${r.status}${await alertKinds(r)}`;
    } catch (err) { error = String(err.message).slice(0, 160); }
    if (!error) {
      if (p.downSince && p.paged) alerts.push({ kind: "liveness", sev: "INFO", key: `up:${s.name}:${now}`, text: `✅ ${s.name} answers again (${host(s.url)}), after ${Math.round((now - p.downSince) / 60000)} min down.` });
      next[s.name] = { lastOk: now, downSince: null, error: null, paged: false };
      continue;
    }
    const downSince = p.downSince ?? now, paged = p.paged || now - downSince >= DOWN_MS;
    if (now - downSince >= DOWN_MS) alerts.push({ kind: "liveness", sev: "SEV2", persistent: true, key: `down:${s.name}:${downSince}`,
      text: `${s.name} is DOWN or refusing: ${redactUrl(s.url)} has not answered 200 for ${Math.round((now - downSince) / 60000)} min (${error}). Railway does not restart a running service whose /healthz fails; check its logs and /status.` });
    next[s.name] = { lastOk: p.lastOk ?? null, downSince, error, paged };
  }
  return { alerts, next };
}
