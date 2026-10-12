import assert from "node:assert/strict";
import { test } from "node:test";
import { mkdtempSync, readFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import type { ReserveReport } from "../../indexer/types.js";
import { Telegram } from "../../monitor/src/alerts.mjs";
import { alertsFor, combine, page, runOnce, served, Store, type AlertState, type Channel, type ReconcileConfig } from "../src/reconcile.js";
import { compareCaps, capsStatus, stageOf, STAGES } from "../src/stage.js";
import { logger } from "../src/log.js";
import type { CapPolicyV } from "../src/ic.js";

// docs/v6/p1-services.md §3 (K7): the stage from custody_caps(), red when stale, the alert policy, and no token anywhere.
const E = 10n ** 18n;
const FROGS = JSON.parse(readFileSync(new URL("../../indexer/fixtures/wrapped/reserves-frogs.json", import.meta.url), "utf8")) as ReserveReport;
const APES = JSON.parse(readFileSync(new URL("../../indexer/fixtures/wrapped/reserves-apes.json", import.meta.url), "utf8")) as ReserveReport;
const policy = (items: number, value: bigint, gItems: number, gValue: bigint, n = 1): CapPolicyV =>
  ({ collections: Array.from({ length: n }, (_, i) => ({ collection: `btc:0x${String(i).padStart(40, "0")}`, max_items: items, max_value_wei: value, max_item_wei: 10n * E })), global_max_items: gItems, global_max_value_wei: gValue });

test("J7 (2026-09-27): the Ethereum launch caps (15 ETH each, 16/22/17/4/2 items, the total their sum) are stage 0, not above it", () => {
  const evm = (a: string, n: number) => ({ collection: `1:0x${a.repeat(40)}`, max_items: n, max_value_wei: 15n * E, max_item_wei: 15n * E });
  const j7: CapPolicyV = { collections: [evm("1", 16), evm("2", 22), evm("3", 17), evm("4", 4), evm("5", 2)], global_max_items: 61, global_max_value_wei: 75n * E };
  assert.deepEqual(stageOf(j7), { stage: 0, exact: true, label: "stage 0" });
  assert.equal(capsStatus(compareCaps(j7, [], 0, 1)).status, "green");
  assert.equal(capsStatus(compareCaps({ ...j7, global_max_value_wei: 76n * E }, [], 0, 1)).status, "red", "a total over the sum is above stage 0");
  const over = { ...j7, collections: [...j7.collections.slice(1), evm("1", 25)], global_max_items: 70 };
  assert.equal(stageOf(over).stage, 1, "25 items is above J7's stage 0");
});

test("the stage custody_caps() implies (G7): exact, 'at most', above 3; against the stack's stage: above is red, below a note, unread unverified", () => {
  for (const s of STAGES) assert.deepEqual(stageOf(policy(s.items, s.value, s.globalItems, s.globalValue, 2)), { stage: s.stage, exact: true, label: `stage ${s.stage}` });
  assert.deepEqual(stageOf(policy(2, 5n * E, 6, 10n * E)), { stage: 0, exact: false, label: "custom, at most stage 0" });
  assert.deepEqual(stageOf(policy(12, 40n * E, 6, 10n * E)), { stage: 1, exact: false, label: "custom, at most stage 1" });
  assert.equal(stageOf(policy(193, 640n * E, 384, 640n * E)).label, "above stage 3");
  const stage0 = policy(3, 10n * E, 6, 10n * E);
  assert.deepEqual(capsStatus(compareCaps(stage0, [], 0, 1)), { status: "green", reasons: [] });
  const above = capsStatus(compareCaps(policy(12, 40n * E, 24, 40n * E), [], 0, 1));
  assert.equal(above.status, "red");
  assert.match(above.reasons[0], /stage 1, above the stack's stage 0/);
  assert.match(capsStatus(compareCaps(policy(2, 5n * E, 6, 10n * E), [], 0, 1)).reasons[0], /^note: .* under the stack's stage 0/);
  assert.equal(capsStatus({ kind: "error", error: "IC down", stackStage: 0, at: 1 }).status, "unverified");
  assert.equal(capsStatus({ kind: "absent", note: "CANISTER_ID is not set", stackStage: 0, at: 1 }).status, "unverified");
});

test("combined: the worst of the collections and the caps; served after RECONCILE_STALE_S (3 h) it is RED, whatever it said", () => {
  const caps = compareCaps(policy(3, 10n * E, 6, 10n * E), [], 0, 1);
  const g = combine([FROGS], caps, 1_790_000_000, 4663);
  assert.equal(g.status, "green");
  assert.equal(combine([FROGS, APES], caps, 1_790_000_000, 4663).status, "unverified");
  const r = combine([FROGS], compareCaps(policy(12, 40n * E, 24, 40n * E), [], 0, 1), 1_790_000_000, 4663);
  assert.equal(r.status, "red");
  assert.equal(served(g, 1_790_000_000 + 10_800, 10_800).servedStatus, "green", "exactly 3 h: still green");
  const stale = served(g, 1_790_000_000 + 10_801, 10_800);
  assert.equal(stale.servedStatus, "red");
  assert.equal(stale.stale, true);
  assert.match(stale.servedReasons[0], /^stale: last computed 2026-09-21T14:13:20.000Z, 180 min ago \(over 3 h\)/);
  assert.equal(served(null, 1, 10_800).servedStatus, "unverified");
  // the page says it in words and in colour
  assert.match(page(stale), /RED — computed 2026-09-21T14:13:20.000Z .* STALE/);
  assert.match(page(served(g, 1_790_000_100, 10_800)), /custody_caps\(\): <b>stage 0<\/b>/);
});

test("alerts: on turning red, on a change of reasons, every 6 h while red, once on recovery; unverified alone is not alerted", () => {
  const caps = compareCaps(policy(3, 10n * E, 6, 10n * E), [], 0, 1);
  const green = served(combine([FROGS], caps, 1_000, 4663), 1_000, 10_800);
  const red1 = served(combine([{ ...FROGS, status: "red", reasons: ["1 item elsewhere"] }], caps, 1_000, 4663), 1_000, 10_800);
  const red2 = served(combine([{ ...FROGS, status: "red", reasons: ["2 items elsewhere"] }], caps, 1_000, 4663), 1_000, 10_800);
  const unv = served(combine([APES], caps, 1_000, 4663), 1_000, 10_800);
  const H = 3_600_000, start: AlertState = { lastStatus: null, redKey: null, redAt: 0 };
  let s = start, a;
  ({ alerts: a, next: s } = alertsFor(s, green, 0, 6 * H, "x")); assert.equal(a.length, 0);
  ({ alerts: a, next: s } = alertsFor(s, unv, 1, 6 * H, "x")); assert.equal(a.length, 0, "unverified is shown, not alerted");
  ({ alerts: a, next: s } = alertsFor(s, red1, 2, 6 * H, "x")); assert.equal(a.length, 1); assert.equal(a[0].sev, "SEV1", "an item not in custody is SEV1");
  ({ alerts: a, next: s } = alertsFor(s, red1, 3 * H, 6 * H, "x")); assert.equal(a.length, 0, "the same red within 6 h: quiet");
  ({ alerts: a, next: s } = alertsFor(s, red2, 4 * H, 6 * H, "x")); assert.equal(a.length, 1, "new reasons: said again");
  ({ alerts: a, next: s } = alertsFor(s, red2, 10 * H + 1, 6 * H, "x")); assert.equal(a.length, 1); assert.match(a[0].text, /still red/);
  ({ alerts: a, next: s } = alertsFor(s, green, 11 * H, 6 * H, "x")); assert.equal(a.length, 1); assert.equal(a[0].sev, "INFO"); assert.match(a[0].text, /back to green/);
  ({ alerts: a, next: s } = alertsFor(s, green, 12 * H, 6 * H, "x")); assert.equal(a.length, 0);
  const staleRed = served(combine([FROGS], caps, 1_000, 4663), 1_000 + 10_801, 10_800);
  ({ alerts: a } = alertsFor(start, staleRed, 0, 6 * H, "x")); assert.equal(a[0].sev, "SEV2", "stale is SEV2");
  // the staleness minutes do not make every watchdog pass a "new" red
  const later = served(combine([FROGS], caps, 1_000, 4663), 1_000 + 12_000, 10_800);
  const k1 = alertsFor(start, staleRed, 0, 6 * H, "x").next.redKey, k2 = alertsFor(start, later, 0, 6 * H, "x").next.redKey;
  assert.equal(k1, k2);
});

const cfg = (dir: string): ReconcileConfig => ({ stackPath: "", chainId: 4663, rpcUrls: [], canisterId: null, icHost: "", everyS: 3_600, staleS: 10_800, realertS: 21_600, stateDir: dir, port: null, publicUrl: null });

test("a run: read, combine, persist, alert; a restart serves the real age (red after 3 h), and the watchdog alerts it", async () => {
  const dir = mkdtempSync(join(tmpdir(), "reconcile-"));
  const sent: string[] = [], lines: string[] = [];
  const channel: Channel = { send: async (t) => { sent.push(t); } };
  let now = 1_790_000_000_000;
  const caps = async () => compareCaps(policy(3, 10n * E, 6, 10n * E), [], 0, now / 1000);
  const d = { cfg: cfg(dir), service: { reserves: async () => [FROGS] }, caps, channel, store: new Store(dir), log: logger((l) => lines.push(l)), nowMs: () => now, stackStage: 0 };
  const s = await runOnce(d);
  assert.equal(s.servedStatus, "green");
  assert.equal(sent.length, 0);
  // the job hangs; a new process over the same state directory, 3 h and a second later: red, and alerted once
  now += 10_801_000;
  const { alert } = await import("../src/reconcile.js");
  const again = await alert({ cfg: cfg(dir), channel, store: new Store(dir), log: d.log }, now);
  assert.equal(again.servedStatus, "red");
  assert.equal(sent.length, 1);
  assert.match(sent[0], /SEV2 · reserves\nreserves RED/);
  await alert({ cfg: cfg(dir), channel, store: new Store(dir), log: d.log }, now + 300_000);
  assert.equal(sent.length, 1, "the next watchdog pass stays quiet");
  // a failed post is not recorded: it is tried again
  const flaky: Channel = { send: async () => { throw new Error("post failed"); } };
  const dir2 = mkdtempSync(join(tmpdir(), "reconcile-"));
  const d2 = { ...d, cfg: cfg(dir2), store: new Store(dir2), channel: flaky, service: { reserves: async () => [{ ...FROGS, status: "red" as const, reasons: ["1 item elsewhere"] }] } };
  await runOnce(d2);
  assert.equal(new Store(dir2).load().alerts.redKey, null, "not recorded as sent");
  // nothing read at all is unverified, never green
  const d3 = { ...d, cfg: cfg(mkdtempSync(join(tmpdir(), "reconcile-"))), service: { reserves: async () => { throw new Error("rpc down"); } } };
  d3.store = new Store(d3.cfg.stateDir);
  assert.equal((await runOnce(d3)).servedStatus, "unverified");
});

test("the Telegram token (monitor/src/alerts.mjs's channel): in one URL only; never in a log line, even when the failing fetch's error carries the URL", async () => {
  const TOKEN = "123456789:AAE-super-secret-token-value";
  const lines: string[] = [];
  const log = logger((l) => lines.push(l), () => [TOKEN]);
  const urls: string[] = [];
  const real = globalThis.fetch;
  globalThis.fetch = (async (url: string) => { urls.push(url); throw new Error(`request to ${url} failed, reason: getaddrinfo ENOTFOUND`); }) as unknown as typeof fetch;
  try {
    const tg = new Telegram({ token: TOKEN, chat: "42", base: "https://api.telegram.org", log: { log: (m: string) => log("info", "telegram", { m }), error: (m: string) => log("error", "telegram", { m }) } as unknown as Console });
    await assert.rejects(tg.send("reserves RED"), /post failed/);
    assert.equal(urls.length, 1);
    assert.ok(urls[0].startsWith("https://api.telegram.org/bot") && urls[0].includes(TOKEN), "the token goes in the one request path");
    assert.ok(!JSON.stringify(tg).includes(TOKEN));
    log("error", "alert-failed", { error: `fetch ${urls[0]}` });   // even a line built carelessly from the URL is cut
    assert.ok(lines.length >= 2);
    assert.ok(lines.every((l) => !l.includes(TOKEN)), lines.join("\n"));
  } finally { globalThis.fetch = real; }
});
