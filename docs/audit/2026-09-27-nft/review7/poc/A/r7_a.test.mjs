// Review round 7, part A: attempts on the exact staged step (A6-L-1) and golive-check --proposal's baseline (A6-I-2), at a78a542.
// A passing assertion marked "WEAKNESS" shows a weakness present; every other assertion is the fix holding.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { decodeInit, encodeInitPlain } from "../src/candid.mjs";
import { proposalAlert } from "../src/argdiff.mjs";
import { checkStagedProposal, stagedStep, stepNumbers, stepTags } from "../src/staged.mjs";

const LAUNCH = JSON.parse(readFileSync(new URL("../../deployments/launch/nft-pilot.json", import.meta.url), "utf8"));
function initAt(step, { caps = null, gItems = null, gValue = null, limits = {}, patch = {} } = {}) {
  const s = stepNumbers(LAUNCH, step);
  const collections = Object.entries(s.caps).map(([collection, c]) => ({ collection, max_items: Number(c.items), max_value_wei: String(c.value), max_item_wei: String(c.item) }));
  if (caps) caps(collections);
  const v = { chain_id: 4663n, vault: "0x00", rpc_urls: ["https://a", "https://b"], btc_api_urls: ["c", "d"], key_name: "key_1", accept_production_key_name: true,
    bridge_allowlist: [], btc_network: "mainnet", operator: [], wrapper: [], evm_chains: [], holder_deposits: [], wrapper_from_block: [], frog_funding: [],
    custody_caps: [{ collections, global_max_items: gItems ?? collections.reduce((a, c) => a + c.max_items, 0), global_max_value_wei: String(gValue ?? collections.reduce((a, c) => a + BigInt(c.max_value_wei), 0n)) }],
    outflow_limits: [{ ...Object.fromEntries(Object.entries(s.limits).map(([k, v]) => [k, k === "global_value_wei" ? String(v) : Number(v)])), ...limits }],
    eth_nft: [{ chain_id: 1n, relay_max_fee_bps: 300, deposits: true, floor: [], gas_pot_target_wei: [], logs_max_blocks: [],
      collections: LAUNCH.collections.map((c) => ({ collection: c.collection, wrapper: "0xaa", module: "0xbb", max_purchase_wei: String(c.maxPurchaseWei), max_premium_bps: 500 })) }],
    ...patch };
  const d = decodeInit(encodeInitPlain(v));
  assert.ok(d.ok, d.error);
  return d;
}
const v = { id: 7, kind: "upgrade", wasm_hash: ["a".repeat(64)], wasm_bytes: [1], arg_hash: ["x"], executable_ns: 2_000_000_000_000_000_000n, proposed_ns: 1_999_740_800_000_000_000n };
const alert = (base, next) => { const staged = stagedStep(LAUNCH, base.init, next.init);
  return proposalAlert(v, { timelock: "tl", builds: [{ commit: "c", buyer: "a".repeat(64) }], baseline: { init: base.init, source: "the install" },
    decoded: next, staged, stagedTags: staged ? stepTags(LAUNCH, staged) : null }); };
// the canister's custody_caps() / custody_outflow() as golive-check hands them in (dfx JSON)
const running = (d) => ({ caps: d.init.custody_caps, outflow: d.init.outflow_limits });

test("control: the exact steps are EXPECTED and pass --proposal", () => {
  const s1 = initAt(1), s2 = initAt(2), s3 = initAt(3);
  assert.equal(stagedStep(LAUNCH, s1.init, s2.init), 2);
  assert.equal(stagedStep(LAUNCH, s2.init, s3.init), 3);
  assert.equal(alert(s1, s2).sev, "SEV2");
  assert.equal(alert(s2, s3).sev, "SEV2");
  assert.ok(checkStagedProposal({ next: s2, base: s1, launch: LAUNCH, step: 2, running: null }).ok);
  assert.ok(checkStagedProposal({ next: s3, base: s2, launch: LAUNCH, step: 3, running: null }).ok);
});

test("holds: near-steps are not EXPECTED (other Init field, a lowering, a trip factor, a removed collection, a value/item cap, global value)", () => {
  const s1 = initAt(1), s2 = initAt(2);
  const cases = {
    "step 2 + operator": [s1, initAt(2, { patch: { operator: ["aaaaa-aa"] } })],
    "step 2 + deposits off->on unchanged but relay fee up": [s1, initAt(2, { patch: { eth_nft: [{ ...s2.init.eth_nft[0], relay_max_fee_bps: 500 }] } })],
    "step 2 + a mint limit lowered": [s1, initAt(2, { limits: { global_mints: 5 } })],
    "step 2 + trip factor 3->12": [s1, initAt(2, { limits: { trip_factor: 12 } })],
    "step 3 + a launch collection removed": [s2, initAt(3, { caps: (c) => c.splice(4, 1) })],
    "step 3 + milady's one-item value lowered": [s2, initAt(3, { caps: (c) => { c[0].max_item_wei = String(10n ** 19n); } })],
    "step 3 + global value 75 -> 76 ETH": [s2, initAt(3, { gValue: 76n * 10n ** 18n })],
    "step 3 + rpc_urls replaced": [s2, initAt(3, { patch: { rpc_urls: ["https://evil-a", "https://evil-b"] } })],
  };
  for (const [name, [b, n]] of Object.entries(cases)) {
    assert.equal(stagedStep(LAUNCH, b.init, n.init), null, name);
    assert.equal(alert(b, n).sev, "SEV1", name);
  }
});

test("WEAKNESS (A7-I-1): the runbook's --proposal command (no --phase1) passes a step 2 that also replaces the operator: only a note", () => {
  const s1 = initAt(1);
  const evil = initAt(2, { patch: { operator: ["aaaaa-aa"], rpc_urls: ["https://evil-a", "https://evil-b"] } });
  // docs/v7/phase1-nft.md §5.1 step 2: `golive-check --proposal <hex> --launch … --stage 2 --canister <buyer>` (the baseline in brackets)
  const r = checkStagedProposal({ next: evil, base: null, launch: LAUNCH, step: 2, running: running(s1) });
  assert.ok(r.ok, "WEAKNESS: passes");
  assert.ok(r.lines.some((l) => /^note no --baseline/.test(l)));
  // with --phase1 (A6-I-2) it fails; the monitor pages it SEV1 regardless
  assert.ok(!checkStagedProposal({ next: evil, base: null, launch: LAUNCH, step: 2, running: running(s1), requireBaseline: true }).ok);
  assert.equal(alert(s1, evil).sev, "SEV1");
});
