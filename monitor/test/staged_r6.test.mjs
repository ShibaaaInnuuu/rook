// Review round 6, part A (A6-L-1, A6-I-2, A6-I-4): the reviewer's PoC (docs/audit/2026-09-27-nft/review6/poc/A/r6_a.test.mjs) turned
// around: a staged step is recognised only when it moves exactly what the step names.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { decodeInit, encodeInitPlain } from "../src/candid.mjs";
import { proposalAlert } from "../src/argdiff.mjs";
import { checkStagedProposal, stagedStep, stepNumbers, stepTags } from "../src/staged.mjs";

// the launch plan these rounds reviewed (J7: five collections, the global the sum, never binding), frozen as it was committed at 9efed1b:
// the staged logic's non-binding branch keeps its regressions. The committed plan (ten collections, a binding global: docs/v7/dormant-
// collections.md §6) is read by test/dormant.test.mjs
const LAUNCH = JSON.parse(readFileSync(new URL("../fixtures/nft-pilot-j7-2026-09-28.json", import.meta.url), "utf8"));
const WRAP = "0x" + "77".repeat(20);
function initAt(step, { extra = [], gItems = null, gValue = null, wrapper = [] } = {}) {
  const s = stepNumbers(LAUNCH, step);
  const collections = Object.entries(s.caps).map(([collection, c]) => ({ collection, max_items: Number(c.items), max_value_wei: String(c.value), max_item_wei: String(c.item) }));
  collections.push(...extra);
  const v = { chain_id: 4663n, vault: "0x00", rpc_urls: ["https://a", "https://b"], btc_api_urls: ["c", "d"], key_name: "key_1", accept_production_key_name: true,
    bridge_allowlist: [], btc_network: "mainnet", operator: [], wrapper, evm_chains: [], holder_deposits: [], wrapper_from_block: [], frog_funding: [],
    custody_caps: [{ collections, global_max_items: gItems ?? collections.reduce((a, c) => a + c.max_items, 0), global_max_value_wei: String(gValue ?? collections.reduce((a, c) => a + BigInt(c.max_value_wei), 0n)) }],
    outflow_limits: [{ ...Object.fromEntries(Object.entries(s.limits).map(([k, v]) => [k, k === "global_value_wei" ? String(v) : Number(v)])) }],
    eth_nft: [{ chain_id: 1n, relay_max_fee_bps: 300, deposits: true, floor: [], gas_pot_target_wei: [], logs_max_blocks: [],
      collections: LAUNCH.collections.map((c) => ({ collection: c.collection, wrapper: "0xaa", module: "0xbb", max_purchase_wei: String(c.maxPurchaseWei), max_premium_bps: 500 })) }] };
  const d = decodeInit(encodeInitPlain(v));
  assert.ok(d.ok, d.error);
  return d;
}
const v = { id: 7, kind: "upgrade", wasm_hash: ["a".repeat(64)], wasm_bytes: [1], arg_hash: ["x"], executable_ns: 2_000_000_000_000_000_000n, proposed_ns: 1_999_740_800_000_000_000n };
const alert = (base, next) => { const staged = stagedStep(LAUNCH, base.init, next.init);
  return proposalAlert(v, { timelock: "tl", builds: [{ commit: "c", buyer: "a".repeat(64) }], baseline: { init: base.init, source: "the install" },
    decoded: next, staged, stagedTags: staged ? stepTags(LAUNCH, staged) : null }); };


const btc = { collection: `btc:${WRAP}`, max_items: 3, max_value_wei: String(10n ** 19n), max_item_wei: String(10n ** 19n) };

test("A6-L-1 (the reviewer's PoC, turned around): step 3 plus a x4 raise of the Bitcoin wrapper's cap and the global caps above the sum is NOT step 3: SEV1, golive-check --proposal refuses it", () => {
  const s2 = initAt(2, { extra: [btc], wrapper: [WRAP] });
  const btcUp = { ...btc, max_items: 12, max_value_wei: String(4n * 10n ** 19n) };
  const evil = initAt(3, { extra: [btcUp], wrapper: [WRAP], gItems: 62, gValue: 4n * 85n * 10n ** 18n });
  assert.equal(stagedStep(LAUNCH, s2.init, evil.init), null, "not classed as step 3");
  const a = alert(s2, evil);
  assert.equal(a.sev, "SEV1");
  assert.ok(!/EXPECTED/.test(a.text) && !/expected: the staged step/.test(a.text));
  const r = checkStagedProposal({ next: evil, base: s2, launch: LAUNCH, step: 3, running: null });
  assert.ok(!r.ok);
  assert.ok(r.lines.some((l) => /^FAIL .*global item cap 62 is not the caps' sum 73/.test(l)), r.lines.join("\n"));
  assert.ok(r.lines.some((l) => /^FAIL .*other caps move too \(btc:/.test(l)), r.lines.join("\n"));
  // each extra alone is refused too
  for (const e of [initAt(3, { extra: [btcUp], wrapper: [WRAP] }), initAt(3, { extra: [btc], wrapper: [WRAP], gItems: 62 }),
    initAt(3, { extra: [btc], wrapper: [WRAP], gValue: 4n * 85n * 10n ** 18n })]) {
    assert.equal(stagedStep(LAUNCH, s2.init, e.init), null);
    assert.ok(!checkStagedProposal({ next: e, base: s2, launch: LAUNCH, step: 3, running: null }).ok);
  }
});

test("A6-L-1 (b), turned around: step 3 plus a new, unconfigured collection key is NOT step 3", () => {
  const s2 = initAt(2);
  assert.equal(stagedStep(LAUNCH, s2.init, initAt(3, { gItems: 62 }).init), null, "the global item cap above the sum (made binding): not step 3");
  const evil = initAt(3, { extra: [{ collection: "1:" + "0x" + "de".repeat(20), max_items: 1, max_value_wei: String(15n * 10n ** 18n), max_item_wei: String(15n * 10n ** 18n) }] });
  assert.equal(stagedStep(LAUNCH, s2.init, evil.init), null);
  assert.equal(alert(s2, evil).sev, "SEV1");
});

test("the exact step 3 (with the Bitcoin wrapper's cap unchanged) is still EXPECTED, and only its own lines are downgraded", () => {
  // one Bitcoin item beside J7's 61 still fits the exit rule at 31 a day (62 = 31 x 2)
  const one = { ...btc, max_items: 1 };
  const s2 = initAt(2, { extra: [one], wrapper: [WRAP] });
  const s3 = initAt(3, { extra: [one], wrapper: [WRAP] });
  assert.equal(stagedStep(LAUNCH, s2.init, s3.init), 3);
  const a = alert(s2, s3);
  assert.equal(a.sev, "SEV2");
  assert.ok(/EXPECTED: this is step 3/.test(a.text));
  const r = checkStagedProposal({ next: s3, base: s2, launch: LAUNCH, step: 3, running: null });
  assert.ok(r.ok, r.lines.join("\n"));
  assert.deepEqual(stepTags(LAUNCH, 2), ["limit:global_items"]);
  assert.ok(stepTags(LAUNCH, 3).includes("global:global_max_items"));
  // the exact step 2 as well
  assert.equal(stagedStep(LAUNCH, initAt(1).init, initAt(2).init), 2);
  assert.equal(alert(initAt(1), initAt(2)).sev, "SEV2");
});

test("A6-L-1: with ctx.staged set, a LOOSENS line the step does not name keeps its own severity (SEV1)", () => {
  const s2 = initAt(2, { extra: [btc], wrapper: [WRAP] });
  const evil = initAt(3, { extra: [{ ...btc, max_items: 12 }], wrapper: [WRAP] });
  // a caller that claims step 3 anyway (the monitor's stagedStep would not): the Bitcoin line is not the step's
  const a = proposalAlert(v, { timelock: "tl", builds: [{ commit: "c", buyer: "a".repeat(64) }], baseline: { init: s2.init, source: "the install" }, decoded: evil, staged: 3, stagedTags: stepTags(LAUNCH, 3) });
  assert.equal(a.sev, "SEV1");
  assert.ok(/LOOSENS custody cap btc:0x7777[0-9a-f]*: items 3 → 12\n/.test(a.text + "\n"), a.text);
  assert.ok(/items 13 → 16 \(expected: the staged step\)/.test(a.text), a.text);
});

test("A6-I-4: a step past the running code's x4 rule is not EXPECTED", () => {
  // a launch file whose step 3 raises Pudgy from 1 to 5 (more than x4): the running code refuses it
  const L = JSON.parse(JSON.stringify(LAUNCH));
  L.stages[0].custodyItems.pudgypenguins = 1; L.stages[1].custodyItems.pudgypenguins = 1; L.stages[2].custodyItems.pudgypenguins = 5;
  const mk = (step) => { const s = stepNumbers(L, step);
    const collections = Object.entries(s.caps).map(([collection, c]) => ({ collection, max_items: Number(c.items), max_value_wei: String(c.value), max_item_wei: String(c.item) }));
    const x = initAt(step); const i = JSON.parse(JSON.stringify(x.init, (_, y) => (typeof y === "bigint" ? `${y}n` : y)), (_, y) => (typeof y === "string" && /^\d+n$/.test(y) ? BigInt(y.slice(0, -1)) : y));
    i.custody_caps = [{ collections, global_max_items: collections.reduce((a, c) => a + c.max_items, 0), global_max_value_wei: String(collections.reduce((a, c) => a + BigInt(c.max_value_wei), 0n)) }];
    const d = decodeInit(encodeInitPlain(i)); assert.ok(d.ok, d.error); return d; };
  const s2 = mk(2), s3 = mk(3);
  assert.equal(stagedStep(L, s2.init, s3.init), null);
  const r = checkStagedProposal({ next: s3, base: s2, launch: L, step: 3, running: null });
  assert.ok(r.lines.some((l) => /^FAIL .*×4/.test(l)), r.lines.join("\n"));
});

test("A6-I-2, turned around: --proposal under --phase1 without --baseline FAILs (the monitor still pages the extra fields SEV1)", () => {
  const s1 = initAt(1);
  const evil = decodeInit(encodeInitPlain({ ...initAt(2).init, operator: ["aaaaa-aa"], rpc_urls: ["https://evil-a", "https://evil-b"] }));
  assert.ok(evil.ok, evil.error);
  const running = { caps: [s1.init.custody_caps[0]], outflow: [s1.init.outflow_limits[0]] };
  const r = checkStagedProposal({ next: evil, base: null, launch: LAUNCH, step: 2, running, requireBaseline: true });
  assert.ok(!r.ok);
  assert.ok(r.lines.some((l) => /^FAIL no --baseline/.test(l)));
  // with the baseline, the other fields are caught
  const rb = checkStagedProposal({ next: evil, base: s1, launch: LAUNCH, step: 2, running: null, requireBaseline: true });
  assert.ok(!rb.ok && rb.lines.some((l) => /changes more than the step: .*operator/.test(l)));
  assert.equal(alert(s1, evil).sev, "SEV1");
});
