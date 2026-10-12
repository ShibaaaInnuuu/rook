// Fix round 5 of the 2026-09-27 NFT review (fixes-round5.md §6: the operator's option (c)): the staged upgrades of the committed launch
// file, as golive-check --proposal and the monitor's argument diff read them. No network.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { decodeInit, encodeInitPlain } from "../src/candid.mjs";
import { proposalAlert } from "../src/argdiff.mjs";
import { checkStagedProposal, stagedStep, stepNumbers } from "../src/staged.mjs";

// the launch plan these rounds reviewed (J7: five collections, the global the sum, never binding), frozen as it was committed at 9efed1b:
// the staged logic's non-binding branch keeps its regressions. The committed plan (ten collections, a binding global: docs/v7/dormant-
// collections.md §6) is read by test/dormant.test.mjs
const LAUNCH = JSON.parse(readFileSync(new URL("../fixtures/nft-pilot-j7-2026-09-28.json", import.meta.url), "utf8"));
const E = 10n ** 18n;
/** The buyer's Init at a step of the launch file (its caps and outflow), with `patch` over it. */
function initAt(step, patch = {}) {
  const s = stepNumbers(LAUNCH, step);
  const collections = Object.entries(s.caps).map(([collection, c]) => ({ collection, max_items: Number(c.items), max_value_wei: String(c.value), max_item_wei: String(c.item) }));
  const base = { chain_id: 4663n, vault: "0x00", rpc_urls: ["https://a", "https://b"], btc_api_urls: ["c", "d"], key_name: "key_1", accept_production_key_name: true,
    bridge_allowlist: [], btc_network: "mainnet", operator: [], wrapper: [], evm_chains: [], holder_deposits: [], wrapper_from_block: [],
    custody_caps: [{ collections, global_max_items: Number(s.global.items), global_max_value_wei: String(s.global.value) }],
    outflow_limits: [{ ...Object.fromEntries(Object.entries(s.limits).map(([k, v]) => [k, k === "global_value_wei" ? String(v) : Number(v)])) }],
    eth_nft: [{ chain_id: 1n, relay_max_fee_bps: 300, deposits: true, floor: [], gas_pot_target_wei: [], logs_max_blocks: [],
      collections: LAUNCH.collections.map((c) => ({ collection: c.collection, wrapper: "0xaa", module: "0xbb", max_purchase_wei: String(c.maxPurchaseWei), max_premium_bps: 500 })) }] };
  const v = { ...base, ...patch };
  const d = decodeInit(encodeInitPlain(v));
  assert.ok(d.ok, d.error);
  return d;
}
const running = (d) => ({ caps: [d.init.custody_caps[0], []], outflow: [d.init.outflow_limits[0], {}] });

test("the committed launch file: 48 items at 12 / 24, then 31 a day, then J7's 61", () => {
  assert.equal(LAUNCH.stage, 1);
  assert.deepEqual(LAUNCH.stages.map((s) => [s.step, Object.values(s.custodyItems).reduce((a, b) => a + b, 0), s.outflow.global_items]), [[1, 48, 24], [2, 48, 31], [3, 61, 31]]);
  assert.deepEqual(LAUNCH.collections.map((c) => c.custodyItems), [13, 17, 13, 3, 2]);
});

test("golive-check --proposal: step 2 is the outflow raise alone; step 3 the caps alone; anything else is refused", () => {
  const s1 = initAt(1), s2 = initAt(2), s3 = initAt(3);
  const ok2 = checkStagedProposal({ next: s2, base: s1, launch: LAUNCH, step: 2, running: running(s1) });
  assert.ok(ok2.ok, ok2.lines.join("\n"));
  const ok3 = checkStagedProposal({ next: s3, base: s2, launch: LAUNCH, step: 3, running: null });
  assert.ok(ok3.ok, ok3.lines.join("\n"));
  // the reviewers' reordering risk: an "outflow raise" built from J7's caps raises both at once (61 at 31 fits the rule): refused
  const both = checkStagedProposal({ next: s3, base: s1, launch: LAUNCH, step: 2, running: running(s1) });
  assert.ok(!both.ok && both.lines.some((l) => /^FAIL .*custody caps move too|^FAIL the proposal's custody caps/.test(l)), both.lines.join("\n"));
  // caps first (61 at 24): not step 3 from step 2's running pair, and does not fit the exit rule
  const capsFirst = initAt(3, { outflow_limits: s1.init.outflow_limits });
  const cf = checkStagedProposal({ next: capsFirst, base: s1, launch: LAUNCH, step: 3, running: running(s1) });
  assert.ok(!cf.ok && cf.lines.some((l) => /exit rule/.test(l) && l.startsWith("FAIL")), cf.lines.join("\n"));
  // step 2 with another field moved (a provider, deposits off): refused with the baseline
  const extra = initAt(2, { rpc_urls: ["https://a", "https://c"] });
  const ex = checkStagedProposal({ next: extra, base: s1, launch: LAUNCH, step: 2, running: null });
  assert.ok(!ex.ok && ex.lines.some((l) => /changes more than the step: rpc_urls/.test(l)), ex.lines.join("\n"));
  // a raise of more than global_items (per_collection_items too): refused
  const per = initAt(2, { outflow_limits: [{ ...s2.init.outflow_limits[0], per_collection_items: 22 }] });
  assert.ok(!checkStagedProposal({ next: per, base: s1, launch: LAUNCH, step: 2, running: null }).ok);
  // queued against the wrong running step: refused
  assert.ok(!checkStagedProposal({ next: s2, base: null, launch: LAUNCH, step: 2, running: running(s3) }).ok);
});

test("the monitor: a proposal that is exactly a staged step is EXPECTED (WARN); the same numbers with anything else stay SEV1", () => {
  const s1 = initAt(1), s2 = initAt(2), s3 = initAt(3);
  const v = { id: 4, kind: "upgrade", wasm_hash: ["a".repeat(64)], wasm_bytes: [1], arg_hash: ["x"], executable_ns: 2_000_000_000_000_000_000n, proposed_ns: 1_999_740_800_000_000_000n };
  const ctx = (base, next) => ({ timelock: "tl", builds: [{ commit: "c", buyer: "a".repeat(64), timelock: "b".repeat(64) }], baseline: { init: base.init, source: "the install" },
    decoded: next, staged: stagedStep(LAUNCH, base.init, next.init) });
  assert.equal(stagedStep(LAUNCH, s1.init, s2.init), 2);
  assert.equal(stagedStep(LAUNCH, s2.init, s3.init), 3);
  assert.equal(stagedStep(LAUNCH, s1.init, s3.init), null, "both at once is no step");
  const a = proposalAlert(v, ctx(s1, s2));
  assert.equal(a.sev, "SEV2", a.text);
  assert.match(a.text, /EXPECTED: this is step 2 of the launch plan/);
  const b = proposalAlert(v, ctx(s1, s3));
  assert.equal(b.sev, "SEV1", "the caps and the outflow together: not a staged step");
  // a caps-first proposal (61 at 24) is annotated: the running code refuses it
  const c = proposalAlert(v, ctx(s2, initAt(3, { outflow_limits: s1.init.outflow_limits })));
  assert.equal(c.sev, "SEV1");
  assert.match(c.text, /does not fit the exit rule .* executing it fails/);
});
