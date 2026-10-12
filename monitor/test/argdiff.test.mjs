// The upgrade-argument diff, against arguments encoded by the canister's OWN Rust types (fixtures/canister.json, from
// autobuyer/canister-rs/examples/watch_fixtures.rs). Run: node --test monitor/test/
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { IDL } from "@icp-sdk/core/candid";
import { Init, decodeInit, decodeReply } from "../src/candid.mjs";
import { STAGE0_CAP, STAGE0_LIMITS, buildOf, diffInit, effectiveCaps, effectiveLimits, eth, proposalAlert } from "../src/argdiff.mjs";

const F = JSON.parse(readFileSync(new URL("../fixtures/canister.json", import.meta.url), "utf8"));
const arg = (name) => { const d = decodeInit(F.args[name]); assert.ok(d.ok, `${name}: ${d.error}`); return d.init; };
const BASE = { init: arg("install"), source: "the install argument" };
const BUILDS = [{ commit: "c0ffee1", buyer: "a".repeat(64), timelock: "b".repeat(64) }];
const view = (over = {}) => ({ id: "7", by: "p1", kind: "upgrade", wasm_hash: ["a".repeat(64)], wasm_bytes: ["1603201"], arg_hex: [F.args.install],
  arg_hash: ["e".repeat(64)], controllers: [], roles: [], proposed_ns: "1790000000000000000", executable_ns: "1790259200000000000", vetoes: [], ...over });
const alertFor = (name, over = {}) => proposalAlert(view({ arg_hex: [F.args[name]], ...over }),
  { timelock: "tl", builds: BUILDS, baseline: BASE, decoded: decodeInit(F.args[name]), roles: null, controllers: ["tl"] });

test("every Rust-encoded argument decodes, with every field the canister's Init has", () => {
  for (const name of Object.keys(F.args)) arg(name);
  const i = arg("install");
  assert.equal(i.chain_id, "4663");
  assert.equal(i.btc_network, "mainnet");
  assert.deepEqual(i.rpc_urls, ["https://rpc.mainnet.chain.robinhood.com", "https://rpc.ordofi.network"]);
  assert.deepEqual(i.custody_caps, []);
  assert.deepEqual(i.wrapper, ["0x00000000000000000000000000000000000000aa"]);
  assert.equal(i.operator.length, 1);
});

test("the stage-0 numbers mirrored here are the canister's own (its custody_caps() and custody_outflow() replies at stage 0)", () => {
  const [caps] = decodeReply("custody_caps", F.replies.custody_caps_stage0);
  const [lim, outflow] = decodeReply("custody_outflow", F.replies.custody_outflow_stage0);
  const e = effectiveCaps(BASE.init);   // null caps with the wrapper → stage 0 for btc:0x<wrapper>
  assert.equal(caps.collections[0].collection, e.collections[0].collection);
  assert.equal(caps.collections[0].max_items, STAGE0_CAP.max_items);
  assert.equal(BigInt(caps.collections[0].max_value_wei), STAGE0_CAP.max_value_wei);
  assert.equal(caps.global_max_items, STAGE0_CAP.global_max_items);
  assert.deepEqual(effectiveLimits({ outflow_limits: [lim] }), STAGE0_LIMITS);
  assert.deepEqual(outflow.freeze, { Open: null });
});

test("neutral: the same argument, and null written out as the stage-0 numbers it means, are no change at all", () => {
  for (const name of ["neutral_same", "neutral_explicit_stage0"]) {
    const d = diffInit(BASE.init, arg(name));
    assert.deepEqual(d.lines, [], name);
    assert.ok(d.unchanged.includes("custody_caps") && d.unchanged.includes("outflow_limits") && d.unchanged.includes("operator"));
    const a = alertFor(name);
    assert.equal(a.sev, "INFO", name);
    assert.match(a.text, /no configuration change \(a code-only upgrade\)/);
    assert.match(a.text, /the published reproducible build of c0ffee1/);
  }
});

test("tightening: lower caps and limits are said, in words, and are not an emergency", () => {
  const d = diffInit(BASE.init, arg("tightened"));
  assert.ok(d.lines.length >= 6);
  assert.ok(d.lines.every((l) => l.verb === "TIGHTENS" && l.sev === "INFO"), JSON.stringify(d.lines));
  const a = alertFor("tightened");
  assert.equal(a.sev, "INFO");
  assert.match(a.text, /TIGHTENS custody cap btc:0x0{38}aa: items 3 → 2/);
  assert.match(a.text, /TIGHTENS custody cap btc:0x0{38}aa: value 10 ETH → 5 ETH/);
  assert.match(a.text, /TIGHTENS outflow limit: items out a day, per collection 3 → 2/);
});

test("loosening: stage 0 → 1 is CRITICAL, line by line, loosening first, with the guardians' instruction", () => {
  const a = alertFor("loosened");
  assert.equal(a.sev, "SEV1");
  assert.match(a.text, /— CRITICAL/);
  assert.match(a.text, /LOOSENS custody cap btc:0x0{38}aa: items 3 → 12/);
  assert.match(a.text, /LOOSENS custody cap btc:0x0{38}aa: value 10 ETH → 40 ETH/);
  assert.match(a.text, /LOOSENS custody cap, global: items in all 6 → 24/);
  assert.match(a.text, /LOOSENS outflow limit: items out a day, per collection 3 → 12/);
  assert.match(a.text, /LOOSENS outflow limit: tripwire factor .* 3 → 4/);
  assert.match(a.text, /veto unless this exact hash and diff were announced \(H6\)/);
  assert.doesNotMatch(a.text, /refuses it/, "×4 is exactly the allowed step");
  // more than ×4 at once: still CRITICAL, and says the running code would refuse it
  assert.match(alertFor("loosened_x10").text, /items 3 → 30 \(more than ×4 at once: the running code refuses it/);
});

test("the H5 fields: operator, providers and identity changes are CRITICAL even with no bound moving", () => {
  const op = alertFor("operator_swap");
  assert.equal(op.sev, "SEV1");
  assert.match(op.text, /CHANGES operator: \S+ → \S+ \(whoever it names can call every operator path\)/);
  const pv = alertFor("providers_swap");
  assert.equal(pv.sev, "SEV1");
  // round 3 (bayc10-decisions.md §5): a provider is printed as scheme://host only ("/…" when a path was cut: a key may live there)
  assert.match(pv.text, /CHANGES Bitcoin providers; adds https:\/\/btc\.example\.net\/…; removes https:\/\/blockstream\.info\/…/);
  const id = alertFor("identity_change");
  assert.equal(id.sev, "SEV1");
  assert.match(id.text, /CHANGES key_name: "key_1" → "test_key_1"/);
  assert.match(id.text, /CHANGES btc_network: "mainnet" → "testnet4"/);
  // fewer providers is also called loosening
  const fewer = diffInit(BASE.init, { ...BASE.init, rpc_urls: ["https://rpc.mainnet.chain.robinhood.com"] });
  assert.ok(fewer.lines.some((l) => l.verb === "LOOSENS" && /4663 RPC providers: 2 → 1/.test(l.text)));
  // a new EVM chain is loosening
  const chain = diffInit(BASE.init, { ...BASE.init, evm_chains: [[{ chain_id: "1", rpc_urls: [] }]] });
  assert.ok(chain.lines.some((l) => l.verb === "LOOSENS" && /adds chain 1/.test(l.text)));
});

test("the module hash: a published build is named; anything else is UNKNOWN and CRITICAL, even with a neutral argument", () => {
  assert.equal(buildOf(BUILDS, "buyer", "0x" + "A".repeat(64)).commit, "c0ffee1", "case and 0x do not matter");
  assert.equal(buildOf(BUILDS, "buyer", "b".repeat(64)), null, "the timelock's hash is not a buyer build");
  const a = alertFor("neutral_same", { wasm_hash: ["9".repeat(64)] });
  assert.equal(a.sev, "SEV1");
  assert.match(a.text, /module 9{64} \(1603201 bytes\): UNKNOWN — not a published reproducible build/);
});

test("an argument this monitor cannot fully read is CRITICAL: an unknown field, garbage, or no baseline", () => {
  // Init plus one field the canister does not have today: Candid decoding alone would silently drop it
  const Extra = IDL.Record({ ...Init._fields.reduce((o, [k, t]) => ({ ...o, [k]: t }), {}), sweep_everything_to: IDL.Text });
  const base = decodeInit(F.args.install).init;
  const v = { ...base, chain_id: 4663n, operator: [], evm_chains: [], custody_caps: [], outflow_limits: [], sweep_everything_to: "bc1q…" };
  const hex = Buffer.from(IDL.encode([Extra], [v])).toString("hex");
  const d = decodeInit(hex);
  assert.equal(d.ok, false);
  assert.match(d.error, /fields this monitor does not know/);
  const a = proposalAlert(view({ arg_hex: [hex] }), { timelock: "tl", builds: BUILDS, baseline: BASE, decoded: d });
  assert.equal(a.sev, "SEV1");
  assert.match(a.text, /argument: fields this monitor does not know/);

  for (const bad of ["4449444c00", "zz", "4449444c016c00010000", F.args.install + "00"]) {
    const g = decodeInit(bad);
    assert.equal(g.ok, false, bad);
    assert.equal(proposalAlert(view({ arg_hex: [bad] }), { timelock: "tl", builds: BUILDS, baseline: BASE, decoded: g }).sev, "SEV1");
  }
  const none = proposalAlert(view(), { timelock: "tl", builds: BUILDS, baseline: null, decoded: decodeInit(F.args.install) });
  assert.equal(none.sev, "SEV1");
  assert.match(none.text, /no baseline for the running configuration/);
});

test("controllers and roles proposals", () => {
  const c = proposalAlert(view({ kind: "controllers", wasm_hash: [], arg_hex: [], controllers: [["aaaaa-aa"]] }),
    { timelock: "tl", builds: BUILDS, controllers: ["tl"] });
  assert.equal(c.sev, "SEV1");
  assert.match(c.text, /NEW CONTROLLERS for the buyer: aaaaa-aa \(now, certified: tl\)/);
  const now = { proposers: ["p1", "p2"], guardians: ["g1", "g2", "g3"], veto_threshold: 2 };
  const roles = (r) => proposalAlert(view({ kind: "roles", wasm_hash: [], arg_hex: [], roles: [r] }), { timelock: "tl", builds: BUILDS, roles: now });
  const tighter = roles({ proposers: ["p1", "p2"], guardians: ["g1", "g2", "g3", "g4"], veto_threshold: 3 });
  assert.equal(tighter.sev, "SEV2", "a new guardian and a higher k still alert, not as an emergency");
  const looser = roles({ proposers: ["p1", "p2", "pX"], guardians: ["g1", "g2", "gX"], veto_threshold: 2 });
  assert.equal(looser.sev, "SEV1");
  assert.match(looser.text, /LOOSENS removes or replaces guardian\(s\) g3/);
  assert.match(looser.text, /LOOSENS adds proposer\(s\) pX/);
  assert.match(roles({ ...now, veto_threshold: 1 }).text, /LOOSENS the veto threshold 2 → 1/);
});

test("wei reads as ETH", () => {
  assert.equal(eth(10n ** 19n), "10 ETH");
  assert.equal(eth("650000000000000000"), "0.65 ETH");
  assert.equal(eth("40_000_000_000_000_000_000"), "40 ETH", "dfx-style digit separators");
});
