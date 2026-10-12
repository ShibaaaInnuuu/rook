// Watcher v2 against a fake IC shaped like the real replies (plain values, as `plain()` makes them): the baseline, the
// certified controllers and module hashes, the live caps, the tripwire, cycles, and the first poll.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { decodeReply } from "../src/candid.mjs";
import { baselineOf, certifiedAlerts, checkWatcher, cyclesCheck } from "../src/watcher.mjs";

const F = JSON.parse(readFileSync(new URL("../fixtures/canister.json", import.meta.url), "utf8"));
const TL = "rrkah-fqaaa-aaaaa-aaaaq-cai", BUYER = "ryjl3-tyaaa-aaaaa-aaaba-cai";
const BUILDS = [{ commit: "c0ffee1", buyer: "a".repeat(64), timelock: "b".repeat(64), timelock_rehearsal: "d".repeat(64) }];
const ROLES = { proposers: ["p1", "p2"], guardians: ["g1", "g2", "g3"], veto_threshold: 2 };
const NS = (ms) => String(BigInt(ms) * 1_000_000n);
const view = (id, over = {}) => ({ id: String(id), by: "p1", kind: "upgrade", wasm_hash: ["a".repeat(64)], wasm_bytes: ["1603201"],
  arg_hex: [F.args.install], arg_hash: ["e".repeat(64)], controllers: [], roles: [], proposed_ns: NS(1_000), executable_ns: NS(259_201_000), vetoes: [], ...over });

function fakeIc({ history = [], pending = [], cert, caps = "custody_caps_stage0", outflow = "custody_outflow_stage0", cycles, nowMs = 300_000_000 } = {}) {
  return {
    status: async () => ({ target: BUYER, roles: ROLES, delay_secs: "259200", min_delay_secs: "259200", rehearsal_build: false, pending,
      target_epoch: "0", roles_epoch: "0", resume_votes: [], now_ns: NS(nowMs) }),
    history: async () => history,
    certified: async (id) => (cert?.[id] ?? (id === BUYER ? { controllers: [TL], module_hash: "a".repeat(64) } : { controllers: [], module_hash: "b".repeat(64) })),
    custodyCaps: async () => (typeof caps === "string" ? decodeReply("custody_caps", F.replies[caps]) : caps),
    custodyOutflow: async () => (typeof outflow === "string" ? decodeReply("custody_outflow", F.replies[outflow]) : outflow),
    cycles: async (id) => (cycles?.[id] ?? { balance: "20000000000000", low_threshold: id === TL ? "500000000000" : "5000000000000" }),
  };
}
const CFG = { timelock: TL, buyer: BUYER, network: "ic", builds: BUILDS, install_arg_hex: F.args.install,
  expect: { proposers: ["p1", "p2"], guardians: ["g1", "g2", "g3"], veto: "2", delay: "259200" } };

test("the first poll says what it watches and judges what is pending, and a clean canister raises nothing else", async () => {
  const r = await checkWatcher({ cfg: CFG, prev: {}, ic: fakeIc({ pending: [view(1, { arg_hex: [F.args.loosened] })] }) });
  const kinds = r.alerts.map((a) => `${a.kind}:${a.sev}`);
  assert.deepEqual(kinds, ["governance:INFO", "proposal:SEV1"], JSON.stringify(r.alerts.map((a) => a.text.split("\n")[0])));
  assert.match(r.alerts[0].text, /watching the buyer timelock/);
  assert.equal(r.alerts[1].persistent, true, "a CRITICAL pending proposal repeats every 6 h until it is gone");
  assert.match(r.alerts[1].text, /diff against the install argument recorded at the ceremony/);
  assert.equal(r.next.seen, 0);
  assert.deepEqual(Object.keys(r.next.certified).sort(), [BUYER, TL].sort());
});

test("the baseline is the last EXECUTED upgrade's argument, not a vetoed or pending one; else the install argument", () => {
  const h = [
    { Proposed: { id: "1", at_ns: NS(1), view: view(1, { arg_hex: [F.args.tightened] }) } },
    { Executed: { id: "1", at_ns: NS(2), by: "x" } },
    { Proposed: { id: "2", at_ns: NS(3), view: view(2, { arg_hex: [F.args.loosened] }) } },
    { Vetoed: { id: "2", at_ns: NS(4), by: ["g1", "g2"] } },
    { Proposed: { id: "3", at_ns: NS(5), view: view(3, { kind: "controllers", wasm_hash: [], arg_hex: [], controllers: [[TL]] }) } },
    { Executed: { id: "3", at_ns: NS(6), by: "x" } },
  ];
  const b = baselineOf(h, F.args.install);
  assert.match(b.baseline.source, /upgrade 1/);
  assert.deepEqual(b.baseline.init.custody_caps[0].collections[0].max_items, 2);
  assert.match(baselineOf([], F.args.install).baseline.source, /install argument/);
  assert.equal(baselineOf([], null).baseline, null);
  assert.match(baselineOf([], "4449444c00").error, /does not decode/);
});

test("a new proposal after that is diffed against the executed one, so undoing a tightening is called loosening", async () => {
  const history = [{ Proposed: { id: "1", at_ns: NS(1), view: view(1, { arg_hex: [F.args.tightened] }) } }, { Executed: { id: "1", at_ns: NS(2), by: "x" } }];
  const caps = [{ collections: [{ collection: "btc:0x00000000000000000000000000000000000000aa", max_items: 2, max_value_wei: "5000000000000000000", max_item_wei: "5000000000000000000" }], global_max_items: 4, global_max_value_wei: "5000000000000000000" }, []];
  const outflow = [{ per_collection_items: 2, global_items: 4, per_collection_mints: 2, global_mints: 4, global_value_wei: "5000000000000000000", trip_factor: 3 }, { freeze: { Open: null } }];
  const back = [...history, { Proposed: { id: "2", at_ns: NS(3), view: view(2) } }];
  const r = await checkWatcher({ cfg: CFG, prev: { seen: 2 }, ic: fakeIc({ history: back, pending: [view(2)], caps, outflow }) });
  const p = r.alerts.find((a) => a.kind === "proposal");
  assert.equal(p.sev, "SEV1");
  assert.match(p.text, /LOOSENS custody cap btc:0x0{38}aa: items 2 → 3/);
  assert.ok(!r.alerts.some((a) => a.kind === "config"), "the live caps match the executed baseline");
});

test("the live caps must match the baseline, or the diff proves nothing", async () => {
  const caps = [{ collections: [{ collection: "btc:0x00000000000000000000000000000000000000aa", max_items: 12, max_value_wei: "40000000000000000000", max_item_wei: "40000000000000000000" }], global_max_items: 24, global_max_value_wei: "40000000000000000000" }, []];
  const r = await checkWatcher({ cfg: CFG, prev: { seen: 0 }, ic: fakeIc({ caps }) });
  const m = r.alerts.find((a) => a.key.startsWith("baseline-mismatch"));
  assert.equal(m.sev, "SEV1");
  assert.match(m.text, /does NOT match the canister's own answer/);
  const none = await checkWatcher({ cfg: { ...CFG, install_arg_hex: undefined }, prev: { seen: 0 }, ic: fakeIc() });
  assert.ok(none.alerts.some((a) => a.key === "baseline:none" && a.sev === "SEV2"));
});

test("the tripwire freeze and a misconfigured timelock are said", async () => {
  const outflow = [{ per_collection_items: 3, global_items: 6, per_collection_mints: 3, global_mints: 6, global_value_wei: "10000000000000000000", trip_factor: 3 },
    { freeze: { Frozen: { since: NS(5_000), cause: { Tripwire: { collection: "btc:0xaa", flow: { Item: null }, demanded: "9", limit: "3" } } } } }];
  const r = await checkWatcher({ cfg: { ...CFG, expect: { ...CFG.expect, veto: "3" } }, prev: { seen: 0 }, ic: fakeIc({ outflow }) });
  assert.ok(r.alerts.some((a) => a.kind === "config" && a.sev === "SEV2" && /FROZEN by the tripwire/.test(a.text)));
  assert.ok(r.alerts.some((a) => a.kind === "config" && a.sev === "SEV1" && /veto threshold is 2, expected 3/.test(a.text)));
});

test("certified: the buyer must be controlled by the timelock alone, the timelock by nobody, both running published builds", () => {
  const ok = { [BUYER]: { controllers: [TL], module_hash: "a".repeat(64) }, [TL]: { controllers: [], module_hash: "b".repeat(64) } };
  assert.deepEqual(certifiedAlerts({ buyer: BUYER, timelock: TL, now: ok, prev: ok, builds: BUILDS, executed: false }).alerts, []);
  const bad = { [BUYER]: { controllers: [TL, "deployer-principal"], module_hash: "9".repeat(64) }, [TL]: { controllers: ["deployer-principal"], module_hash: "d".repeat(64) } };
  const a = certifiedAlerts({ buyer: BUYER, timelock: TL, now: bad, prev: null, builds: BUILDS, executed: false }).alerts;
  assert.ok(a.every((x) => x.sev === "SEV1" && x.kind === "controllers" && x.persistent));
  assert.ok(a.some((x) => /NOT CONTROLLED BY THE TIMELOCK ALONE/.test(x.text)));
  assert.ok(a.some((x) => /NOT BLACKHOLED/.test(x.text)));
  assert.ok(a.some((x) => /NOT a published reproducible build/.test(x.text)));
  assert.ok(a.some((x) => /REHEARSAL build/.test(x.text)));
});

test("certified: a change with no Executed is SEV1; with Executed it is news; an upgrade running right now gets one poll", () => {
  const was = { [BUYER]: { controllers: [TL], module_hash: "a".repeat(64) }, [TL]: { controllers: [], module_hash: "b".repeat(64) } };
  const now = { ...was, [BUYER]: { controllers: [TL], module_hash: "c".repeat(64) } };
  const builds = [...BUILDS, { commit: "beef", buyer: "c".repeat(64), timelock: "b".repeat(64) }];
  const silent = certifiedAlerts({ buyer: BUYER, timelock: TL, now, prev: was, builds, executed: false, pending: [] });
  assert.equal(silent.alerts.length, 1);
  assert.match(silent.alerts[0].text, /CHANGED WITHOUT AN EXECUTED PROPOSAL/);
  const executed = certifiedAlerts({ buyer: BUYER, timelock: TL, now, prev: was, builds, executed: true, pending: [] });
  assert.equal(executed.alerts[0].sev, "INFO");
  assert.match(executed.alerts[0].text, /the reproducible build of beef/);
  // the upgrade is executing: its module is already installed, its Executed not yet recorded
  const running = certifiedAlerts({ buyer: BUYER, timelock: TL, now, prev: was, builds, executed: false, nowNs: NS(300_000_000),
    pending: [view(4, { wasm_hash: ["c".repeat(64)] })] });
  assert.deepEqual(running.alerts, []);
  assert.deepEqual(running.hold, [BUYER]);
  // …but not one that is still inside its delay
  const early = certifiedAlerts({ buyer: BUYER, timelock: TL, now, prev: was, builds, executed: false, nowNs: NS(2_000),
    pending: [view(4, { wasm_hash: ["c".repeat(64)] })] });
  assert.equal(early.alerts[0].sev, "SEV1");
});

test("the watcher keeps the old certified value for a canister on hold, and reports it next poll if Executed never comes", async () => {
  const was = { [BUYER]: { controllers: [TL], module_hash: "a".repeat(64) }, [TL]: { controllers: [], module_hash: "b".repeat(64) } };
  const cert = { ...was, [BUYER]: { controllers: [TL], module_hash: "c".repeat(64) } };
  const pending = [view(4, { wasm_hash: ["c".repeat(64)] })];
  const builds = [...BUILDS, { commit: "beef", buyer: "c".repeat(64), timelock: "b".repeat(64) }];
  const one = await checkWatcher({ cfg: { ...CFG, builds }, prev: { seen: 0, certified: was }, ic: fakeIc({ cert, pending }) });
  assert.ok(!one.alerts.some((a) => a.key.startsWith("unexecuted")));
  assert.equal(one.next.certified[BUYER].module_hash, "a".repeat(64));
  const two = await checkWatcher({ cfg: { ...CFG, builds }, prev: one.next, ic: fakeIc({ cert, pending: [] }) });
  assert.ok(two.alerts.some((a) => a.key.startsWith("unexecuted") && a.sev === "SEV1"));
});

test("events after the first poll are said once each, in v1's words; a pause is SEV2", async () => {
  const history = [{ Proposed: { id: "1", at_ns: NS(1), view: view(1) } }, { VetoVote: { id: "1", at_ns: NS(2), by: "g1", votes: 1, needed: 2 } },
    { Vetoed: { id: "1", at_ns: NS(3), by: ["g1", "g2"] } }, { Paused: { at_ns: NS(4), paused: true, ok: true, why: [] } }];
  const r = await checkWatcher({ cfg: CFG, prev: { seen: 0 }, ic: fakeIc({ history }) });
  const g = r.alerts.filter((a) => a.kind === "governance" || a.kind === "proposal");
  assert.deepEqual(g.map((a) => a.key), [`proposal:${TL}:1`, `event:${TL}:1`, `event:${TL}:2`, `event:${TL}:3`]);
  assert.match(g[2].text, /VETOED/);
  assert.equal(g[3].sev, "SEV2");
  assert.match(g[3].text, /PAUSED the buyer/);
  assert.equal(r.next.seen, 4);
});

test("cycles: one alert per crossing, each way, at the canister's threshold and at 2 T for the buyer", () => {
  const b = `the buyer ${BUYER}`;
  let r = cyclesCheck({}, { [b]: { balance: "4000000000000", low_threshold: "5000000000000" } });
  assert.equal(r.alerts.length, 1);
  assert.match(r.alerts[0].text, /LOW on cycles: 4\.00T left, below its own 5\.00T threshold/);
  r = cyclesCheck(r.next, { [b]: { balance: "1_900_000_000_000", low_threshold: "5000000000000" } });
  assert.equal(r.alerts.length, 1);
  assert.match(r.alerts[0].text, /below 2\.00T \(G8: page everyone\)/);
  const again = cyclesCheck(r.next, { [b]: { balance: "1900000000000", low_threshold: "5000000000000" } });
  assert.equal(again.alerts.length, 0, "still low: said once");
  const up = cyclesCheck(r.next, { [b]: { balance: "20000000000000", low_threshold: "5000000000000" } });
  assert.equal(up.alerts.length, 2);
  assert.ok(up.alerts.every((a) => /above .* again: 20\.00T/.test(a.text)));
  assert.notEqual(up.alerts[0].key, r.alerts[0].key, "a later crossing has its own key, so dedup does not swallow it");
});
