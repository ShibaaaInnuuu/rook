// Go-live cycles plan (docs/v7/golive-runbook.md §6): the buyer's burn rate and the days it leaves, paged before the level alerts (5 T,
// 2 T) could fire. The review's worst case (≈ 52 T a day for five collections on 13 nodes) drains a 5 T threshold in hours.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { validate } from "../src/config.mjs";
import { statusBody } from "../src/main.mjs";
import { BURN_DEFAULTS, BURN_MIN_SPAN_S, BURN_SAMPLE_S, burnCheck, checkWatcher } from "../src/watcher.mjs";

const T = 1_000_000_000_000n, B = "the buyer ryjl3-tyaaa-aaaaa-aaaba-cai", T0 = 1_791_000_000;
const DAY = 86_400;

/** Polls every minute for `hours`, the balance from `balanceAt(seconds since T0)`; returns every poll's alerts and the last result. */
function run(hours, balanceAt, cfg, { every = 60, prev = {} } = {}) {
  const out = [];
  let r;
  for (let s = 0; s <= hours * 3600; s += every) {
    r = burnCheck(prev, { [B]: { balance: balanceAt(s).toString() } }, T0 + s, cfg);
    prev = r.next;
    out.push({ s, alerts: r.alerts });
  }
  return { out, last: r, prev };
}
const perDay = (tPerDay, start) => (s) => start - (BigInt(Math.round(tPerDay * 1000)) * T * BigInt(s)) / (1000n * BigInt(DAY));
const keys = (out, k) => out.flatMap((p) => p.alerts.map((a) => ({ ...a, s: p.s }))).filter((a) => a.key.includes(`:${k}:`));

test("the defaults are the runbook's: 24 h window, 7 / 2 days, 10 T a day (the dormant model: docs/v7/dormant-collections.md §7; 30 before), 20 T floor", () => {
  assert.deepEqual({ ...BURN_DEFAULTS }, { window_h: 24, min_days: 7, min_days_sev1: 2, burn_warn_t_per_day: 10, min_balance_t: 12 });
});

test("a normal burn (8 T a day from 200 T) pages nothing and is measured", () => {
  const { out, last } = run(48, perDay(8, 200n * T));
  assert.deepEqual(out.flatMap((p) => p.alerts), []);
  const rep = last.report[B];
  assert.ok(Math.abs(Number(BigInt(rep.per_day) / 1_000_000_000n) / 1000 - 8) < 0.05, rep.per_day);
  assert.ok(rep.days_left > 22 && rep.days_left < 24, String(rep.days_left));
  assert.equal(rep.span_h, 24);
  // one sample every 10 minutes, the window's worth
  assert.ok(last.next[B].s.length <= 24 * 3600 / BURN_SAMPLE_S + 1, String(last.next[B].s.length));
});

test("the review's worst case (52 T a day from 200 T): the rate pages once at 6 h, then under 7 days, then SEV1 under 2 days, persistently", () => {
  const { out } = run(3 * 24, perDay(52, 200n * T));
  const rate = keys(out, "rate"), days = keys(out, "days"), sev1 = keys(out, "sev1");
  assert.equal(rate.length, 1, "one page per crossing");
  assert.equal(rate[0].sev, "SEV2");
  assert.equal(rate[0].s, BURN_MIN_SPAN_S, "not before 6 h of samples");
  assert.match(rate[0].text, /burning 52\.\d\dT a day/);
  assert.equal(days[0].s, BURN_MIN_SPAN_S, "200 T at 52 T a day is under 7 days at once");
  assert.equal(days[0].sev, "SEV2");
  // under 2 days: 200 - 52 d < 104 → d > 1.85 days
  assert.ok(sev1.length > 1, "SEV1 repeats (persistent) every poll while it lasts; the channel throttles by key");
  assert.equal(sev1[0].sev, "SEV1");
  assert.ok(sev1[0].persistent);
  assert.ok(Math.abs(sev1[0].s / DAY - 1.85) < 0.02, String(sev1[0].s / DAY));
  assert.ok(new Set(sev1.map((a) => a.key)).size === 1, "one key while it lasts");
  assert.match(sev1[0].text, /UNDER 2 DAYS/);
});

test("a top-up is never counted against the rate, and clears the days pages with an INFO", () => {
  // 52 T a day from 150 T for a day (paged), then a 400 T top-up; the burn goes on
  const bal = (s) => perDay(52, 150n * T)(s) + (s >= DAY ? 400n * T : 0n);
  const { out, last } = run(30, bal);
  const back = out.flatMap((p) => p.alerts).filter((a) => a.sev === "INFO" && /back from "days"/.test(a.text));
  assert.equal(back.length, 1);
  const rate = Number(BigInt(last.report[B].per_day) / 1_000_000_000n) / 1000;
  assert.ok(Math.abs(rate - 52) < 0.5, `the rate is still the burn (${rate}), not burn minus the top-up`);
});

test("under 6 h of samples no rate is claimed; the floor pages at once", () => {
  const { out, last } = run(5, () => 11n * T);
  const floor = keys(out, "floor");
  assert.equal(floor.length, 1);
  assert.equal(floor[0].s, 0);
  assert.match(floor[0].text, /under 12 T of cycles \(11\.00T; burning not measured yet\)/);
  assert.equal(last.report[B].per_day, null);
  assert.deepEqual(keys(out, "rate"), []);
});

test("the configuration's thresholds apply; a restart keeps the samples (they are the watcher's state)", () => {
  const cfg = { burn_warn_t_per_day: 100, min_days: 3, min_days_sev1: 1 };
  const { out, prev } = run(12, perDay(52, 400n * T), cfg);
  assert.deepEqual(out.flatMap((p) => p.alerts), [], "52 T a day under a 100 T warning, 400 T is 7.7 days");
  const again = burnCheck(JSON.parse(JSON.stringify(prev)), { [B]: { balance: perDay(52, 400n * T)(12 * 3600 + 60).toString() } }, T0 + 12 * 3600 + 60, cfg);
  assert.equal(again.report[B].span_h > 11, true);
});

test("checkWatcher carries the burn in its state and /status shows it", async () => {
  const TL = "rrkah-fqaaa-aaaaa-aaaaq-cai", BUYER = "ryjl3-tyaaa-aaaaa-aaaba-cai";
  const F = JSON.parse(readFileSync(new URL("../fixtures/canister.json", import.meta.url), "utf8"));
  const { decodeReply } = await import("../src/candid.mjs");
  let bal = 100n * T;
  const ic = {
    status: async () => ({ target: BUYER, roles: { proposers: ["p1", "p2"], guardians: ["g1", "g2", "g3"], veto_threshold: 2 }, delay_secs: "259200",
      min_delay_secs: "259200", rehearsal_build: false, pending: [], target_epoch: "0", roles_epoch: "0", resume_votes: [], now_ns: "0" }),
    history: async () => [],
    certified: async (id) => (id === BUYER ? { controllers: [TL], module_hash: "a".repeat(64) } : { controllers: [], module_hash: "b".repeat(64) }),
    custodyCaps: async () => decodeReply("custody_caps", F.replies.custody_caps_stage0),
    custodyOutflow: async () => decodeReply("custody_outflow", F.replies.custody_outflow_stage0),
    cycles: async (id) => ({ balance: (id === BUYER ? bal : 3n * T).toString(), low_threshold: id === TL ? "500000000000" : "5000000000000" }),
  };
  const cfg = { timelock: TL, buyer: BUYER, network: "ic", builds: [{ commit: "c", buyer: "a".repeat(64), timelock: "b".repeat(64) }],
    install_arg_hex: F.args.install, expect: { proposers: ["p1", "p2"], guardians: ["g1", "g2", "g3"], veto: "2", delay: "259200" }, cycles: { burn_warn_t_per_day: 30 } };
  let prev = {}, all = [];
  for (let s = 0; s <= 8 * 3600; s += 600) {
    bal = perDay(52, 100n * T)(s);
    const r = await checkWatcher({ cfg, prev, ic, now: (T0 + s) * 1000 });
    prev = r.next; all.push(...r.alerts);
  }
  assert.ok(all.some((a) => a.key.startsWith(`cycles-burn:the buyer ${BUYER}:rate:`)), "the buyer's rate paged");
  assert.ok(!all.some((a) => a.key.startsWith(`cycles-burn:the timelock`)), "the timelock burns nothing: no page");
  const body = statusBody(cfg, { watch: prev, checkers: {} }, (T0 + 8 * 3600) * 1000);
  assert.ok(Number(body.cycles[`the buyer ${BUYER}`].days_left) < 2);
});

test("config: the cycles thresholds are validated", () => {
  const base = { timelock: "rrkah-fqaaa-aaaaa-aaaaq-cai", builds: [{ commit: "c", buyer: "a".repeat(64), timelock: "b".repeat(64) }] };
  assert.doesNotThrow(() => validate({ ...base, cycles: { window_h: 24, min_days: 7, min_days_sev1: 2, burn_warn_t_per_day: 30, min_balance_t: 20, "//": "note" } }));
  for (const bad of [{ window_h: 1 }, { min_days: 0 }, { min_days: 2, min_days_sev1: 3 }, { burn_warn_t_per_day: "30" }, { typo: 1 }, []])
    assert.throws(() => validate({ ...base, cycles: bad }), /cycles/, JSON.stringify(bad));
});
