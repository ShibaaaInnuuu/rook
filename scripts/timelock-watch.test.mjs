// scripts/timelock-watch.mjs, tested: the words for every event (no network), the configuration check, the cycles alerts, and —
// when the LOCAL replica has the trust-points rehearsal timelock (autobuyer/timelock-rs/rehearse-roles.sh) — a live run: a
// proposal made there, seen, withdrawn, seen again; both canisters' cycles read; and the Telegram path against a local mock
// server with a FAKE token, which must never appear in any output.
// Nothing here touches the IC mainnet or Telegram. Run: node --test scripts/timelock-watch.test.mjs
import assert from "node:assert/strict";
import { execFile, execFileSync } from "node:child_process";
import { mkdtempSync, readFileSync } from "node:fs";
import { createServer } from "node:http";
import { homedir, tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { cyclesAlerts, describe, parseArgs, surprises } from "./timelock-watch.mjs";

const TL = process.env.WATCH_TL ?? "i4pph-ih777-77777-aaakq-cai";   // rehearse-roles.sh's timelock
const WATCH = new URL("./timelock-watch.mjs", import.meta.url).pathname;
const DFX = join(homedir(), "Library/Application Support/org.dfinity.dfx/bin/dfx");
const CANDID = new URL("../autobuyer/timelock-rs/etch_timelock.did", import.meta.url).pathname;
const H = "a".repeat(64);

test("every event is described with what a reader must check", () => {
  const view = { id: "3", by: "prop-a", kind: "upgrade", wasm_hash: [H], wasm_bytes: ["1603201"], arg_hex: ["4449"], arg_hash: ["b".repeat(64)],
    controllers: [], roles: [], proposed_ns: "1790000000000000000", executable_ns: "1790259200000000000", vetoes: [] };
  const p = describe({ Proposed: { id: "3", at_ns: "1790000000000000000", view } }, { timelock: TL });
  assert.match(p, /PROPOSAL 3 \(upgrade\) by prop-a/);
  assert.match(p, new RegExp(`module sha256 ${H} \\(1603201 bytes\\)`));
  assert.match(p, /72\.0 h after it was proposed/);
  assert.match(p, /reproducible-build\.sh/);
  const c = describe({ Proposed: { id: "4", at_ns: "1", view: { ...view, id: "4", kind: "controllers", wasm_hash: [], wasm_bytes: [],
    arg_hash: [], controllers: [["aaaaa-aa"]] } } }, { timelock: TL });
  assert.match(c, /NEW CONTROLLERS for the buyer: aaaaa-aa/);
  assert.match(describe({ Cancelled: { id: "3", at_ns: "1790000000000000000", by: "prop-a" } }, {}), /proposal 3 WITHDRAWN by its proposer prop-a/);
  assert.match(describe({ VetoVote: { id: "3", at_ns: "1", by: "g1", votes: 1, needed: 2 } }, {}), /g1 voted to VETO proposal 3 \(1 of 2\)/);
  assert.match(describe({ Vetoed: { id: "3", at_ns: "1790000000000000000", by: ["g1", "g2"] } }, {}), /proposal 3 VETOED.*g1, g2/);
  assert.match(describe({ ResumeVote: { at_ns: "1", by: "g3", votes: 1, needed: 2 } }, {}), /g3 voted to RESUME the buyer \(1 of 2\)/);
  const roles = { proposers: ["p2", "p3"], guardians: ["g1", "g2", "g3"], veto_threshold: 2 };
  assert.match(describe({ RolesChanged: { id: "4", at_ns: "1", roles } }, {}), /CHANGED THE ROLES: proposers p2, p3; guardians g1, g2, g3; veto 2/);
  const r = describe({ Proposed: { id: "4", at_ns: "1", view: { ...view, id: "4", kind: "roles", wasm_hash: [], wasm_bytes: [], arg_hash: [], roles: [roles] } } }, {});
  assert.match(r, /NEW ROLES: proposers p2, p3; guardians g1, g2, g3; veto 2 of 3/);
  assert.match(describe({ Executed: { id: "3", at_ns: "1790000000000000000", by: "2vxsx-fae" } }, {}), /EXECUTED by 2vxsx-fae.*verify/);
  assert.match(describe({ Failed: { id: "3", at_ns: "1790000000000000000", by: "x", why: "post_upgrade trapped" } }, {}), /FAILED.*post_upgrade trapped.*Nothing changed/);
  assert.match(describe({ Paused: { at_ns: "1790000000000000000", paused: true, ok: true, why: [] } }, {}), /PAUSED the buyer/);
  assert.match(describe({ Paused: { at_ns: "1790000000000000000", paused: false, ok: false, why: ["refused"] } }, {}), /RESUMED.*FAILED: refused/);
  // canister review2 N-6 (integration R3): a failed stop may run again after 10 min; a guardian can start the buyer
  assert.match(describe({ NotStopped: { id: "3", at_ns: "1", why: "busy" } }, {}), /did NOT run.*after 10 minutes/);
  assert.match(describe({ NotStarted: { id: "3", at_ns: "1", why: "stopped" } }, {}), /did NOT START again.*start_target/);
  assert.match(describe({ Started: { at_ns: "1790000000000000000", by: "g1", ok: true, why: [] } }, {}), /guardian g1 STARTED the buyer.*stays paused/);
  assert.match(describe({ Started: { at_ns: "1790000000000000000", by: "g1", ok: false, why: ["no"] } }, {}), /FAILED.*\(no\).*still stopped/);
});

test("a timelock that is not configured as expected is said out loud, and a rehearsal build on the IC above all", () => {
  const s = { target: "t", roles: { proposers: ["p1", "p2"], guardians: ["g1", "g2", "g3"], veto_threshold: 2 }, delay_secs: "259200", rehearsal_build: false };
  assert.deepEqual(surprises(s, { target: "t", proposers: ["p2", "p1"], guardians: ["g1", "g2", "g3"], veto: "2", delay: "259200" }, "ic"), [],
    "the order of a set does not matter");
  assert.equal(surprises(s, { guardians: ["g1", "g2", "gX"] }, "ic").length, 1);
  assert.match(surprises(s, { veto: "3" }, "ic")[0], /veto threshold is 2, expected 3/);
  assert.match(surprises({ ...s, rehearsal_build: true }, {}, "ic")[0], /REHEARSAL build/);
  assert.deepEqual(surprises({ ...s, rehearsal_build: true }, {}, "local"), [], "a rehearsal build is expected locally");
});

test("cycles: one alert when a canister crosses its own threshold, one when it recovers, none while it stays", () => {
  const low = { balance: "4000000000000", low_threshold: "5000000000000" }, fine = { balance: "20000000000000", low_threshold: "5000000000000" };
  let r = cyclesAlerts({}, { buyer: low, timelock: fine });
  assert.equal(r.messages.length, 1);
  assert.match(r.messages[0], /buyer is LOW on cycles: 4\.00T left, below its 5\.00T threshold/);
  r = cyclesAlerts(r.flags, { buyer: low, timelock: fine });
  assert.equal(r.messages.length, 0, "still low: said once");
  r = cyclesAlerts(r.flags, { buyer: fine, timelock: fine });
  assert.match(r.messages[0], /buyer is above its cycles threshold again: 20\.00T/);
  // as dfx's JSON writes them: digit separators
  assert.match(cyclesAlerts({}, { buyer: { balance: "3_500_000_000_000", low_threshold: "5_000_000_000_000" } }).messages[0], /3\.50T left/);
});

test("arguments: a timelock id is required, and nothing unknown is accepted", () => {
  assert.throws(() => parseArgs([]), /--timelock/);
  assert.throws(() => parseArgs(["--timelock", TL, "--token", "x"]), /unknown argument --token/);
  assert.throws(() => parseArgs(["--timelock", TL, "--network", "mainnet"]), /ic or local/);
  assert.equal(parseArgs(["--timelock", TL]).network, "ic", "the IC by default: that is where the real one is");
});

// ---- live, local replica only ----------------------------------------------------------------------------------------------
const replicaHasIt = (() => {
  try {
    execFileSync(DFX, ["canister", "call", "--network", "local", "--identity", "anonymous", "--query", "--candid", CANDID, TL, "status"],
      { stdio: "pipe", timeout: 60_000 });
    return true;
  } catch { return false; }
})();
const run = (args, env = {}) => new Promise((ok) => execFile("node", [WATCH, ...args], { env: { ...process.env, ...env }, timeout: 180_000 },
  (err, stdout, stderr) => ok({ code: err ? err.code ?? 1 : 0, out: `${stdout}${stderr}` })));
const dfx = (...a) => execFileSync(DFX, ["canister", "call", "--network", "local", "--identity", "trust-p2", "--candid", CANDID, TL, ...a],
  { encoding: "utf8", timeout: 120_000, env: { ...process.env, DFX_WARNING: "-mainnet_plaintext_identity" } });

test("live on the local replica: a proposal is seen, its cancellation is seen, and Telegram gets exactly the post", { skip: !replicaHasIt && "the local rehearsal timelock is not on this replica" }, async () => {
  const state = join(mkdtempSync(join(tmpdir(), "tlw-")), "state.json");
  const base = ["--timelock", TL, "--network", "local", "--once", "--state", state];

  const first = await run([...base, "--dry-run"]);
  assert.equal(first.code, 0, first.out);
  assert.match(first.out, /watching the etched buyer timelock/);
  assert.match(first.out, /nothing pending/);
  const seen0 = JSON.parse(readFileSync(state, "utf8"))[TL].seen;

  // a proposal, made here by one of the rehearsal's proposers (the local test identity trust-p2): hand the buyer to the timelock
  // itself, which is who controls it already — harmless, and withdrawn below
  const proposed = dfx("propose_controllers", `(vec { principal "${TL}" })`);
  const id = proposed.match(/id = (\d+)/)?.[1];
  assert.ok(id, proposed);
  const second = await run([...base, "--dry-run"]);
  assert.match(second.out, new RegExp(`PROPOSAL ${id} \\(controllers\\)`));
  assert.match(second.out, /NEW CONTROLLERS for the buyer/);
  assert.equal(JSON.parse(readFileSync(state, "utf8"))[TL].seen, seen0 + 1);

  dfx("withdraw", `(${id} : nat64)`);

  // the Telegram path, against a mock that fails once and then accepts: a failed post leaves the event unseen, so it is retried
  const got = []; let fails = 1;
  const server = createServer((req, res) => {
    let body = ""; req.on("data", (c) => (body += c));
    req.on("end", () => { got.push({ url: req.url, body: JSON.parse(body) }); res.statusCode = fails-- > 0 ? 500 : 200; res.end("{}"); });
  });
  await new Promise((r) => server.listen(0, "127.0.0.1", r));
  const env = { TELEGRAM_BOT_TOKEN: "123456:FAKE-TOKEN-never-real", TELEGRAM_CHAT_ID: "-4242",
    TELEGRAM_API_BASE: `http://127.0.0.1:${server.address().port}` };
  try {
    const failed = await run(base, env);
    assert.notEqual(failed.code, 0, "a failed post fails the poll");
    assert.equal(JSON.parse(readFileSync(state, "utf8"))[TL].seen, seen0 + 1, "and the cancellation is not marked seen");
    const posted = await run(base, env);
    assert.equal(posted.code, 0, posted.out);
    assert.equal(JSON.parse(readFileSync(state, "utf8"))[TL].seen, seen0 + 2);
    assert.equal(got.length, 2, "one failed attempt, one post");
    assert.equal(got[1].url, "/bot123456:FAKE-TOKEN-never-real/sendMessage");
    assert.equal(got[1].body.chat_id, "-4242");
    assert.match(got[1].body.text, new RegExp(`proposal ${id} WITHDRAWN`));
    for (const r of [first, second, failed, posted]) assert.ok(!r.out.includes("FAKE-TOKEN"), "the token never appears in any output");
    assert.ok(!readFileSync(state, "utf8").includes("FAKE-TOKEN"), "nor in the state file");
  } finally {
    server.close();
  }
});
