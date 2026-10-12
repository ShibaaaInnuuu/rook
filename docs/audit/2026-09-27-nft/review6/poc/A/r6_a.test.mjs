// Review round 6, part A: the staged-step classification (staged.mjs) is loose for step 3 on everything that is not a launch
// collection's own cap: the global caps above the sum, and any other collection's cap (a new key, a Bitcoin wrapper's inscriptions).
// A "passing" test shows the weakness is present.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { decodeInit, encodeInitPlain } from "../src/candid.mjs";
import { proposalAlert } from "../src/argdiff.mjs";
import { checkStagedProposal, stagedStep, stepNumbers } from "../src/staged.mjs";

const LAUNCH = JSON.parse(readFileSync(new URL("../../deployments/launch/nft-pilot.json", import.meta.url), "utf8"));
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
const alert = (base, next) => proposalAlert(v, { timelock: "tl", builds: [{ commit: "c", buyer: "a".repeat(64) }], baseline: { init: base.init, source: "the install" },
  decoded: next, staged: stagedStep(LAUNCH, base.init, next.init) });

test("A6-L-1: step 3 plus a x4 raise of the Bitcoin wrapper's cap, and the global caps above the sum: EXPECTED (SEV2), golive-check --proposal passes", () => {
  const btc = { collection: `btc:${WRAP}`, max_items: 3, max_value_wei: String(10n ** 19n), max_item_wei: String(10n ** 19n) };
  const s2 = initAt(2, { extra: [btc], wrapper: [WRAP] });
  const btcUp = { ...btc, max_items: 12, max_value_wei: String(4n * 10n ** 19n) };
  const evil = initAt(3, { extra: [btcUp], wrapper: [WRAP], gItems: 62, gValue: 4n * 85n * 10n ** 18n });
  assert.equal(stagedStep(LAUNCH, s2.init, evil.init), 3, "the monitor classes it as exactly step 3");
  const a = alert(s2, evil);
  console.log(a.text);
  assert.equal(a.sev, "SEV2", "paged as EXPECTED, not SEV1");
  const r = checkStagedProposal({ next: evil, base: s2, launch: LAUNCH, step: 3, running: null });
  console.log(r.lines.join("\n"));
  assert.ok(r.ok, "golive-check --proposal --stage 3 --baseline passes it");
});

test("A6-L-1 (b): step 3 plus a new, unconfigured collection key: EXPECTED", () => {
  const s2 = initAt(2);
  const evil = initAt(3, { extra: [{ collection: "1:" + "0x" + "de".repeat(20), max_items: 1, max_value_wei: String(15n * 10n ** 18n), max_item_wei: String(15n * 10n ** 18n) }] });
  assert.equal(stagedStep(LAUNCH, s2.init, evil.init), 3);
  assert.equal(alert(s2, evil).sev, "SEV2");
});

test("control: step 2 with the same extras is refused (kindProblem compares every key)", () => {
  const s1 = initAt(1);
  const evil = initAt(2, { gValue: 300n * 10n ** 18n });
  assert.equal(stagedStep(LAUNCH, s1.init, evil.init), null);
});

test("A6-I-2: golive-check --proposal with --canister only passes a step-2 argument that also names a new operator and new 4663 providers (a note, not a FAIL); the monitor does page it SEV1", () => {
  const s1 = initAt(1);
  const evil = decodeInit(encodeInitPlain({ ...initAt(2).init, operator: ["aaaaa-aa"], rpc_urls: ["https://evil-a", "https://evil-b"] }));
  assert.ok(evil.ok, evil.error);
  const r = checkStagedProposal({ next: evil, base: null, launch: LAUNCH, step: 2, running: { caps: [s1.init.custody_caps[0]], outflow: [s1.init.outflow_limits[0]] } });
  console.log(r.lines.join("\n"));
  assert.ok(r.ok, "passes");
  assert.equal(stagedStep(LAUNCH, s1.init, evil.init), null);
  assert.equal(alert(s1, evil).sev, "SEV1");
});
