// Alerts: the words around them, deduplication, and Telegram (docs/v6/k6-watcher-canary.md §6).
//
// An alert is { kind, key, sev: SEV1 | SEV2 | SEV3 | INFO, text, persistent? }. A key is sent once. A persistent key (an
// unexplained spend, a wrong controller) is sent again every REPEAT_MS while the checker keeps producing it: silence is not
// safety. A post that fails is not recorded, so the next poll sends it again.
//
// The bot token (ALERT_TELEGRAM_BOT_TOKEN) goes in the request path and nowhere else: never logged, printed, returned by the
// status endpoint or written to the state file, and scrubbed from any error text.
//
// Fail closed (audit E-08b): without both the token and the chat id the channel REFUSES to be built, unless the caller asked for
// a dry run by name. A missing variable is a startup error, never an alert "sent" to the log.
import { redact } from "./config.mjs";

export const KINDS = ["proposal", "governance", "controllers", "config", "custody-outbound", "custody-inbound",
  "provider-disagreement", "evm-mint", "cycles", "watcher-error", "heartbeat",
  // wave 2 (docs/audit/2026-09-26/fixes-W2-monitor.md): the keeper on chain, draws, the guardian, the services' heartbeats, the canister
  "keeper", "draw", "guardian", "liveness", "canister",
  // docs/v7/cycles-from-fees.md §4.2: the fueler (ops-unexpected-tx, its own alerts, the ops wallet)
  "fueler"];
export const REPEAT_MS = 6 * 3600 * 1000;
const ICON = { SEV1: "🚨", SEV2: "⚠️", SEV3: "🔎", INFO: "ℹ️" };
const LIMIT = 3900;   // Telegram's message limit is 4096 characters

export function format(a, { test = false } = {}) {
  const head = `${test ? "🧪 TEST — no action needed. " : ""}${ICON[a.sev] ?? ""} ${a.sev} · ${a.kind}`;
  const body = redact(`${head}\n${a.text}`);   // a keyed RPC URL's key never reaches the channel
  return body.length > LIMIT ? `${body.slice(0, LIMIT)}\n… (cut; the monitor's /status has the rest)` : body;
}

/** Which alerts to send now, given what was sent: pure. `sent` maps key → { at, sev }. */
export function due(alerts, sent, now) {
  const out = [], seen = new Set();
  for (const a of alerts) {
    if (!a.key) throw new Error(`an alert without a key: ${a.kind}`);
    if (!KINDS.includes(a.kind)) throw new Error(`an alert of an unknown kind: ${a.kind}`);
    if (seen.has(a.key)) continue;
    seen.add(a.key);
    const s = sent[a.key];
    if (!s || (a.persistent && now - s.at >= REPEAT_MS)) out.push(s ? { ...a, repeat: true } : a);
  }
  return out;
}

/** Keys older than `days` and not persistent are forgotten, so the state stays small. Pure. */
export function prune(sent, now, days = 90) {
  return Object.fromEntries(Object.entries(sent).filter(([, v]) => v.persistent || now - v.at < days * 86400_000));
}

export class TelegramUnset extends Error {
  constructor(missing) { super(`refusing to start: ${missing.join(" and ")} ${missing.length > 1 ? "are" : "is"} not set, so no alert could reach anyone. Set both, or pass --dry-run to print alerts instead.`); this.name = "TelegramUnset"; this.missing = missing; }
}

export class Telegram {
  constructor({ token = process.env.ALERT_TELEGRAM_BOT_TOKEN, chat = process.env.ALERT_TELEGRAM_CHAT_ID,
    base = process.env.ALERT_TELEGRAM_API_BASE ?? "https://api.telegram.org", dryRun = false, log = console } = {}) {
    // kept off `this` as plain fields so that JSON.stringify of the object (a status dump, a debug print) cannot carry it
    const secret = token;
    this.chat = chat;
    this.base = base;
    if (!dryRun && (!token || !chat)) throw new TelegramUnset([!token && "ALERT_TELEGRAM_BOT_TOKEN", !chat && "ALERT_TELEGRAM_CHAT_ID"].filter(Boolean));
    this.dryRun = dryRun;
    this.log = log;
    this.post = async (text) => {
      const r = await fetch(`${this.base}/bot${secret}/sendMessage`, { method: "POST", headers: { "content-type": "application/json" },
        body: JSON.stringify({ chat_id: this.chat, text, disable_web_page_preview: true }), signal: AbortSignal.timeout(20_000) });
      if (!r.ok) throw new Error(`Telegram answered HTTP ${r.status}`);
    };
    this.scrub = (s) => (secret ? String(s).split(secret).join("<token>") : String(s));
  }
  toJSON() { return { chat: this.chat ? "(set)" : "(unset)", dryRun: this.dryRun }; }

  async send(text) {
    if (this.dryRun) { this.log.log(`--- dry run (--dry-run): would post ---\n${text}\n`); return; }
    try {
      await this.post(text);
      this.log.log(`posted: ${text.split("\n")[0]}`);
    } catch (err) {
      this.log.error(`could not post: ${this.scrub(err.message)}`);
      throw new Error("post failed");
    }
  }
}

/**
 * Send what is due and record it in `state.sent`. Returns { sent, failed }. Stops at the first failed post: the rest go on the
 * next poll, in order.
 */
export async function deliver(alerts, state, channel, now = Date.now()) {
  state.sent ??= {};
  state.log ??= [];
  let sent = 0;
  for (const a of due(alerts, state.sent, now)) {
    try {
      await channel.send(format(a));
    } catch {
      return { sent, failed: true };
    }
    state.sent[a.key] = { at: now, sev: a.sev, persistent: !!a.persistent };
    // review round 4 (D4-I-2): the head /status serves is redacted like the message (before it is cut, so a cut secret is caught whole)
    state.log.push({ at: now, kind: a.kind, sev: a.sev, key: a.key, head: redact(a.text.split("\n")[0]).slice(0, 200) });
    sent++;
  }
  state.log = state.log.filter((x) => now - x.at < 7 * 86400_000);
  return { sent, failed: false };
}

/** Once a day, at or after `hourUtc`: the heartbeat's key if it is due, else null. Pure. */
export function heartbeatKey(now, hourUtc, sent) {
  const d = new Date(now);
  if (d.getUTCHours() < hourUtc) return null;
  const key = `heartbeat:${d.toISOString().slice(0, 10)}`;
  return sent?.[key] ? null : key;
}

export function heartbeat(now, { watched, checkers, log }) {
  const day = (log ?? []).filter((x) => now - x.at < 86400_000 && x.kind !== "heartbeat");
  const bySev = ["SEV1", "SEV2", "INFO"].map((s) => `${day.filter((x) => x.sev === s).length} ${s}`).join(", ");
  const ago = (t) => (t ? `${Math.round((now - t) / 60000)} min ago` : "never");
  return {
    kind: "heartbeat", key: `heartbeat:${new Date(now).toISOString().slice(0, 10)}`, sev: "INFO",
    text: [`💓 the monitor is alive (${new Date(now).toISOString().replace(/\.\d+Z$/, "Z")})`, ...watched,
      ...Object.entries(checkers).map(([k, c]) => `${k}: last good poll ${ago(c.lastOk)}${c.lastError ? `; last error: ${c.lastError}` : ""}`),
      `alerts in the last 24 h: ${bySev}`,
      "No heartbeat tomorrow means the monitor is down: treat that as SEV2."].join("\n"),
  };
}
