// Alerting: dedup, the 6-hour SEV1 repeat, a failed post retried, the daily heartbeat, `--test-alert all` reaching a (mock)
// Telegram with one message of each kind, the poll's commit rule, silence as an alert, and the status endpoint. A FAKE token
// is used and must never appear in any output, state file or status body.
import assert from "node:assert/strict";
import { execFile } from "node:child_process";
import { mkdtempSync, readFileSync } from "node:fs";
import { createServer } from "node:http";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { KINDS, REPEAT_MS, Telegram, TelegramUnset, deliver, due, format, heartbeatKey } from "../src/alerts.mjs";
import { STALE_MS, parseArgs, pollOnce, statusBody } from "../src/main.mjs";
import { saveState, loadState } from "../src/state.mjs";
import { validate } from "../src/config.mjs";

const MAIN = new URL("../src/main.mjs", import.meta.url).pathname;
const FAKE = "123456:FAKE-TOKEN-never-real";
const T0 = Date.UTC(2026, 8, 25, 13, 0, 0);
const a = (key, over = {}) => ({ kind: "custody-outbound", key, sev: "SEV1", text: `alert ${key}`, ...over });
const recorder = (failAfter = Infinity) => { const got = []; return { got, send: async (t) => { if (got.length >= failAfter) throw new Error("down"); got.push(t); } }; };

async function mockTelegram(fails = 0) {
  const got = [];
  const server = createServer((req, res) => {
    let body = ""; req.on("data", (c) => (body += c));
    req.on("end", () => { got.push({ url: req.url, body: JSON.parse(body) }); res.statusCode = fails-- > 0 ? 500 : 200; res.end("{}"); });
  });
  await new Promise((r) => server.listen(0, "127.0.0.1", r));
  return { got, server, base: `http://127.0.0.1:${server.address().port}` };
}

test("dedup: a key is sent once; a persistent one again after 6 hours; every alert needs a key and a known kind", () => {
  const sent = {};
  assert.equal(due([a("x"), a("x")], sent, T0).length, 1, "the same key twice in one poll is one message");
  sent.x = { at: T0 }; sent.p = { at: T0 };
  assert.equal(due([a("x")], sent, T0 + REPEAT_MS * 10).length, 0, "not persistent: never again");
  assert.equal(due([a("p", { persistent: true })], sent, T0 + REPEAT_MS - 1).length, 0);
  const again = due([a("p", { persistent: true })], sent, T0 + REPEAT_MS);
  assert.equal(again.length, 1);
  assert.equal(again[0].repeat, true);
  assert.throws(() => due([{ kind: "custody-outbound", sev: "SEV1", text: "x" }], {}, T0), /without a key/);
  assert.throws(() => due([a("k", { kind: "nonsense" })], {}, T0), /unknown kind/);
});

test("a failed post is not recorded, so it is sent next time; the ones before it are not sent twice", async () => {
  const state = {};
  const ch = recorder(1);
  const r = await deliver([a("one"), a("two")], state, ch, T0);
  assert.deepEqual(r, { sent: 1, failed: true });
  assert.ok(state.sent.one && !state.sent.two);
  const ch2 = recorder();
  await deliver([a("one"), a("two")], state, ch2, T0 + 1000);
  assert.equal(ch2.got.length, 1);
  assert.match(ch2.got[0], /alert two/);
});

test("the heartbeat: once a day, at or after the configured hour", () => {
  assert.equal(heartbeatKey(Date.UTC(2026, 8, 25, 11, 59), 12, {}), null);
  assert.equal(heartbeatKey(Date.UTC(2026, 8, 25, 12, 0), 12, {}), "heartbeat:2026-09-25");
  assert.equal(heartbeatKey(Date.UTC(2026, 8, 25, 18, 0), 12, { "heartbeat:2026-09-25": { at: 1 } }), null);
  assert.equal(heartbeatKey(Date.UTC(2026, 8, 26, 12, 0), 12, { "heartbeat:2026-09-25": { at: 1 } }), "heartbeat:2026-09-26");
});

test("format: severity and kind first, a test marked as a test, long text cut under Telegram's limit", () => {
  assert.match(format(a("k")), /^🚨 SEV1 · custody-outbound\nalert k$/);
  assert.match(format(a("k"), { test: true }), /^🧪 TEST — no action needed\. 🚨 SEV1/);
  assert.ok(format(a("k", { text: "x".repeat(10_000) })).length < 4096);
});

test("Telegram: the token only in the request path, scrubbed from errors, absent from JSON; --dry-run prints", async () => {
  const t = await mockTelegram(1);
  try {
    const logs = [];
    const log = { log: (s) => logs.push(s), error: (s) => logs.push(s) };
    const ch = new Telegram({ token: FAKE, chat: "-4242", base: t.base, log });
    await assert.rejects(ch.send("first"), /post failed/);
    await ch.send("second");
    assert.equal(t.got.length, 2);
    assert.equal(t.got[1].url, `/bot${FAKE}/sendMessage`);
    assert.deepEqual([t.got[1].body.chat_id, t.got[1].body.text], ["-4242", "second"]);
    assert.ok(!JSON.stringify(ch).includes("FAKE"), "not in the object's JSON");
    assert.ok(!logs.join("\n").includes("FAKE"), "not in any log line");
    const bad = new Telegram({ token: FAKE, chat: "-1", base: `http://127.0.0.1:1/${FAKE}`, log });
    await assert.rejects(bad.send("x"));
    assert.ok(!logs.join("\n").includes("FAKE"), "not even in a connection error that quotes the URL");
    const printed = [];
    await new Telegram({ token: undefined, chat: undefined, dryRun: true, log: { log: (s) => printed.push(s) } }).send("hello");
    assert.match(printed[0], /dry run .*\nhello/);
  } finally { t.server.close(); }
});

test("--test-alert all posts exactly one alert of each kind, marked TEST, and the token appears in no output", async () => {
  const t = await mockTelegram();
  try {
    const env = { ...process.env, ALERT_TELEGRAM_BOT_TOKEN: FAKE, ALERT_TELEGRAM_CHAT_ID: "-4242", ALERT_TELEGRAM_API_BASE: t.base };
    const out = await new Promise((ok) => execFile("node", [MAIN, "--test-alert", "all"], { env, timeout: 60_000 },
      (err, stdout, stderr) => ok({ code: err ? err.code ?? 1 : 0, text: `${stdout}${stderr}` })));
    assert.equal(out.code, 0, out.text);
    assert.match(out.text, new RegExp(`sent ${KINDS.length} test alert\\(s\\)`));
    assert.equal(KINDS.length, 17, "11 kinds before wave 2, and keeper, draw, guardian, liveness, canister; and the fueler (cycles-from-fees §4.2)");
    assert.equal(t.got.length, KINDS.length);
    const kinds = t.got.map((g) => g.body.text.split("\n")[0].match(/· (\S+)$/)[1]);
    assert.deepEqual(kinds, KINDS, "one of each kind, in order");
    assert.ok(t.got.every((g) => g.body.text.startsWith("🧪 TEST — no action needed.")));
    assert.ok(!out.text.includes("FAKE"));
    // one kind alone
    const one = await new Promise((ok) => execFile("node", [MAIN, "--test-alert", "custody-outbound"], { env, timeout: 60_000 },
      (err, stdout, stderr) => ok({ code: err ? 1 : 0, text: `${stdout}${stderr}` })));
    assert.equal(one.code, 0, one.text);
    assert.match(t.got.at(-1).body.text, /UNEXPECTED OUTBOUND TRANSACTION/);
  } finally { t.server.close(); }
  assert.throws(() => parseArgs(["--test-alert", "everything"]), /all or one of/);
  assert.throws(() => parseArgs(["--token", "x"]), /unknown argument --token/);
});

test("the poll commits a checker's new state only when its alerts were delivered; a silent checker becomes an alert", async () => {
  const cfg = validate({ timelock: "rrkah-fqaaa-aaaaa-aaaaq-cai", builds: [{ commit: "c", buyer: "a".repeat(64), timelock: "b".repeat(64) }], state: "/nowhere" });
  let calls = 0;
  const ic = {
    status: async () => { calls++; return { target: "ryjl3-tyaaa-aaaaa-aaaba-cai", roles: { proposers: [], guardians: [], veto_threshold: 2 }, delay_secs: "259200", rehearsal_build: false, pending: [], now_ns: "0" }; },
    history: async () => [], certified: async (id) => ({ controllers: id === cfg.timelock ? [] : [cfg.timelock], module_hash: id === cfg.timelock ? "b".repeat(64) : "a".repeat(64) }),
    custodyCaps: async () => [{ collections: [], global_max_items: 6, global_max_value_wei: "10000000000000000000" }, []],
    custodyOutflow: async () => [{ per_collection_items: 3, global_items: 6, per_collection_mints: 3, global_mints: 6, global_value_wei: "10000000000000000000", trip_factor: 3 }, { freeze: { Open: null } }],
    cycles: async () => ({ balance: "20000000000000", low_threshold: "5000000000000" }),
  };
  const down = recorder(0);
  const s1 = await pollOnce({ cfg, state: {}, ic, channel: down, now: T0 });
  assert.equal(s1.watch, undefined, "nothing was delivered, so nothing is committed");
  assert.match(s1.checkers.watcher.lastError, /could not be posted/);
  const up = recorder();
  const s2 = await pollOnce({ cfg, state: s1, ic, channel: up, now: T0 + 60_000 });
  assert.equal(s2.watch.seen, 0);
  assert.equal(s2.checkers.watcher.lastOk, T0 + 60_000);
  assert.ok(up.got.some((m) => /watching the buyer timelock/.test(m)), "the first poll's words, delivered on the retry");
  assert.ok(up.got.some((m) => /baseline/.test(m)));
  // now the IC stops answering: after 30 minutes of that, the silence itself is posted
  const broken = { ...ic, status: async () => { throw new Error("timelock status: fetch failed"); } };
  const s3 = await pollOnce({ cfg, state: s2, ic: broken, channel: up, now: T0 + 60_000 + STALE_MS - 1 });
  assert.ok(!up.got.some((m) => /has not completed a poll/.test(m)));
  const s4 = await pollOnce({ cfg, state: s3, ic: broken, channel: up, now: T0 + 60_000 + STALE_MS });
  assert.ok(up.got.some((m) => /⚠️ SEV2 · watcher-error\nthe watcher checker has not completed a poll for 30 min \(last error: timelock status: fetch failed\)/.test(m)));
  // the status endpoint's body: stale, the error, and no secret
  const body = statusBody(cfg, s4, T0 + 60_000 + STALE_MS);
  assert.equal(body.ok, false);
  assert.match(body.checkers.watcher.lastError, /fetch failed/);
  assert.ok(body.alerts_last_24h.length >= 2);
  // the state file round-trips and holds no token (there is none in it to hold)
  const f = join(mkdtempSync(join(tmpdir(), "mon-")), "state.json");
  saveState(f, s4);
  assert.deepEqual(loadState(f).watch, s4.watch);
  assert.ok(!readFileSync(f, "utf8").includes("FAKE") && !JSON.stringify(body).includes("FAKE"));
  assert.equal(calls, 2, "the healthy IC was asked twice; the broken one answered nothing");
});

test("the configuration refuses what would make the monitor lie", () => {
  const ok = { timelock: "rrkah-fqaaa-aaaaa-aaaaq-cai", builds: [{ commit: "c", buyer: "a".repeat(64), timelock: "b".repeat(64) }] };
  assert.doesNotThrow(() => validate({ ...ok }));
  assert.throws(() => validate({ ...ok, builds: [] }), /every module is UNKNOWN/);
  assert.throws(() => validate({ ...ok, bitcoin: { providers: ["https://mempool.space/api"], addresses: { custody: "bc1pfssfalzdt9h8uwqcrs73780l723mh2axrul7keaevke7a8pg4a7szrd0ht" } } }),
    /buyer: required[\s\S]*exactly two esplora bases/);
  assert.throws(() => validate({ ...ok, buyer: "ryjl3-tyaaa-aaaaa-aaaba-cai", evm: { rpcs: ["https://a"], wrappers: ["0x12"] } }), /exactly two 4663 RPCs[\s\S]*0x12 is not an address/);
  assert.throws(() => validate({ ...ok, install_arg_hex: "abcd" }), /install_arg_hex/);
});

test("E-08b regression: Telegram FAILS CLOSED — no token or no chat id is a startup refusal, never an alert 'sent' to the log", async () => {
  assert.throws(() => new Telegram({ token: undefined, chat: "123" }), (e) => e instanceof TelegramUnset && /ALERT_TELEGRAM_BOT_TOKEN is not set/.test(e.message));
  assert.throws(() => new Telegram({ token: FAKE, chat: undefined }), /ALERT_TELEGRAM_CHAT_ID is not set/);
  assert.throws(() => new Telegram({ token: undefined, chat: undefined }), /ALERT_TELEGRAM_BOT_TOKEN and ALERT_TELEGRAM_CHAT_ID are not set/);
  assert.equal(new Telegram({ token: undefined, chat: undefined, dryRun: true }).dryRun, true, "only an explicit --dry-run prints instead");
  // the service itself: exit 2 at start, for the test alert and for the watch alike
  const env = { ...process.env, ALERT_TELEGRAM_BOT_TOKEN: "", ALERT_TELEGRAM_CHAT_ID: "", MONITOR_CONFIG: "", MONITOR_CONFIG_JSON: "" };
  const run = (args) => new Promise((ok) => execFile("node", [MAIN, ...args], { env, timeout: 60_000 }, (err, stdout, stderr) => ok({ code: err ? err.code : 0, text: `${stdout}${stderr}` })));
  for (const args of [["--test-alert", "all"], ["--once"]]) {
    const r = await run(args);
    assert.equal(r.code, 2, r.text);
    assert.match(r.text, /refusing to start: ALERT_TELEGRAM_BOT_TOKEN and ALERT_TELEGRAM_CHAT_ID are not set/);
  }
  assert.equal((await run(["--test-alert", "heartbeat", "--dry-run"])).code, 0, "a dry run by name still prints");
});
