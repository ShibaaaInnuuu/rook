// docs/v7/dormant-collections.md: the monitor's side of dormant, wake-on-demand collections, on the canister's own bytes
// (autobuyer/canister-rs/tests/fixtures/eth-state-dormant.hex and init-dormant.hex, written by tests/dormant.rs). A DORMANT or WAKING
// collection's floor, references, pool and poster are not watched (nothing is held; a paused poster would wedge its next wake), the
// no-good-post pause skips it, a slow or timed-out wake pages, and an upgrade argument naming the dormancy decodes strictly and is
// worded. No network.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { IDL } from "@icp-sdk/core/candid";
import { EthState, decodeInit, plain } from "../src/candid.mjs";
import { diffInit } from "../src/argdiff.mjs";
import { CAPACITY_STALE_S, capacityOf, dormancyAlerts, livesOf, sleepingModules, watched, WAKE_QUEUED_S, WAKE_SLOW_S } from "../src/ethnft.mjs";
import { statusBody } from "../src/main.mjs";
import { checkEvm } from "../src/evm.mjs";
import { EMPTY_RECORDS } from "../src/records.mjs";
import { fakeChain, hex } from "./fakechain.mjs";

const FIX = (f) => readFileSync(new URL(`../../autobuyer/canister-rs/tests/fixtures/${f}`, import.meta.url), "utf8").trim();
const state = () => plain(IDL.decode([EthState], Uint8Array.from(Buffer.from(FIX("eth-state-dormant.hex"), "hex")))[0]);
const MILADY = "0x5af0d9827e0c53e4799bb226655a1de152a425a5", DOODLES = "0x8a90cab2b38dba80c64b7734e58ee1db38b8992e", THIRD = `0x${"33".repeat(20)}`;

test("the canister's dormant state decodes with the monitor's IDL: every life, and a pending Fill", () => {
  const st = state();
  const lives = livesOf(st);
  assert.deepEqual([...lives.keys()], [MILADY, DOODLES, THIRD]);
  assert.deepEqual([...lives.values()].map((l) => l.phase), ["Active", "Waking", "Dormant"]);
  assert.equal(lives.get(DOODLES).waking_since, 10);
  assert.ok(Object.keys(st.pending[0].step).includes("Fill"));
  // without dormancy (an older state, or none configured) every collection is watched as before
  assert.equal(livesOf({}), null);
  assert.equal(watched(null, MILADY), true);
});

test("independent review DI-M-1: a pending markServed (Step::Served) decodes, and a life's held desk request is ignored", () => {
  const st = plain(IDL.decode([EthState], Uint8Array.from(Buffer.from(FIX("eth-state-dormant-served.hex"), "hex")))[0]);
  assert.deepEqual(Object.keys(st.pending[0].step), ["Served"]);
  assert.equal(BigInt(st.pending[0].step.Served.id), 4n);
  assert.deepEqual([...livesOf(st).values()].map((l) => l.phase), ["Active", "Waking", "Dormant"]);
});

test("only an ACTIVE collection's floor and poster are watched; the no-good-post pause skips the others", () => {
  const st = state();
  const lives = livesOf(st);
  assert.equal(watched(lives, MILADY), true);
  assert.equal(watched(lives, DOODLES), false, "waking: no post yet, by design");
  assert.equal(watched(lives, THIRD), false, "dormant");
  assert.equal(watched(lives, `0x${"44".repeat(20)}`), true, "a collection the canister has not decided yet is watched");
  const cfg = { collections: [{ collection: MILADY, module: "0xAA" }, { collection: DOODLES, module: "0xBB" }, { collection: THIRD, module: "0xCC" }] };
  assert.deepEqual([...sleepingModules(st, cfg)], ["0xbb", "0xcc"]);
});

test("a wake warming up past 6 h pages SEV2; a timed-out wake pages SEV2 once, for a day", () => {
  const st = state();
  assert.deepEqual(dormancyAlerts({ buyer: "b", state: st, nowS: 10 + WAKE_SLOW_S - 1 }).filter((a) => a.key.startsWith("eth-wake-slow")), []);
  const slow = dormancyAlerts({ buyer: "b", state: st, nowS: 10 + WAKE_SLOW_S });
  assert.ok(slow.some((a) => a.sev === "SEV2" && a.key === `eth-wake-slow:b:${DOODLES}:10` && /WARMING UP/.test(a.text)));
  // the third collection's wake timed out at 13 (its note says so): paged for a day
  assert.ok(slow.some((a) => a.key === `eth-wake-timeout:b:${THIRD}:13` && /TIMED OUT/.test(a.text)));
  assert.ok(!dormancyAlerts({ buyer: "b", state: st, nowS: 13 + 86_400 }).some((a) => a.key.startsWith("eth-wake-timeout")));
});

test("DR-M-3: a wake queued past 6 h pages SEV2 (every slot is held, or the day's wakes are spent)", () => {
  const st = state();
  const lives = livesOf(st);
  assert.deepEqual([lives.get(THIRD).queued_at, lives.get(THIRD).queued_by], [14, "Deposit"]);
  assert.ok(!dormancyAlerts({ buyer: "b", state: st, nowS: 14 + WAKE_QUEUED_S - 1 }).some((a) => a.key.startsWith("eth-wake-queued")));
  const q = dormancyAlerts({ buyer: "b", state: st, nowS: 14 + WAKE_QUEUED_S }).find((a) => a.key.startsWith("eth-wake-queued"));
  assert.ok(q && q.sev === "SEV2" && /max_active/.test(q.text));
});

test("an upgrade argument naming the dormancy decodes strictly (every label known) and its diff is worded", () => {
  const d = decodeInit(FIX("init-dormant.hex"));
  assert.ok(d.ok, d.error);
  const e = d.init.eth_nft[0];
  assert.equal(e.dormancy[0].max_active, 10);
  // §11: the scaling, in B cycles
  assert.deepEqual(e.dormancy[0].scaling[0], { active_b_per_day: "2500", idle_b_per_day: "300", wake_b: "1500", runway_days: 7, reserve_b: "8000", exit_b_per_awake: "3000" });
  assert.equal(e.dormancy[0].wake_desk[0], `0x${"de".repeat(20)}`);
  const off = structuredClone(d.init);
  off.eth_nft[0].dormancy = [];
  const on = diffInit(off, d.init);
  assert.ok(on.lines.some((l) => /eth_nft\.dormancy: off → ON/.test(l.text)), JSON.stringify(on.lines));
  const more = structuredClone(d.init);
  more.eth_nft[0].dormancy[0].max_active = 12;
  more.eth_nft[0].dormancy[0].wake_desk = [`0x${"ef".repeat(20)}`];
  const lines = diffInit(d.init, more).lines.map((l) => `${l.sev} ${l.text}`);
  assert.ok(lines.some((l) => /^WARN eth_nft\.dormancy\.max_active: 10 → 12/.test(l)), lines.join("\n"));
  assert.ok(lines.some((l) => /wake_desk/.test(l)), lines.join("\n"));
});

// ---- §11: the capacity on the cycles balance ----------------------------------------------------------------------------------------

test("§11: the canister's capacity decodes; fewer awake-able than holding positions pages SEV2 (not on a stale snapshot)", () => {
  const st = state();
  const cap = capacityOf(st);
  assert.deepEqual({ ...cap, status: undefined }, { at: 16, balance_t: 20, scaling: true, collections: 10, ceiling: 10, effective_max_active: 0, awake: 2,
    pinned: 1, wake_headroom: 0, wakes_left_today: 11, next_wake_at_t: 54.1, status: undefined });
  const page = dormancyAlerts({ buyer: "b", state: st, nowS: 20 }).find((a) => a.key.startsWith("eth-capacity-short"));
  assert.ok(page && page.sev === "SEV2" && page.persistent && /keeps only 0 collection\(s\) awake/.test(page.text) && /1 hold positions/.test(page.text)
    && /TOP IT UP: one more wake starts at 54\.1 T/.test(page.text), page?.text);
  assert.ok(!dormancyAlerts({ buyer: "b", state: st, nowS: 16 + CAPACITY_STALE_S }).some((a) => a.key.startsWith("eth-capacity-short")), "a stale snapshot");
  // enough for those with positions: no page
  const ok = structuredClone(st);
  ok.dorm[0].cap[0].effective_max_active = 1;
  assert.ok(!dormancyAlerts({ buyer: "b", state: ok, nowS: 20 }).some((a) => a.key.startsWith("eth-capacity-short")));
  // fixed caps (no scaling): never
  const fixed = structuredClone(st);
  fixed.dorm[0].cap[0].scaling = false;
  assert.ok(!dormancyAlerts({ buyer: "b", state: fixed, nowS: 20 }).some((a) => a.key.startsWith("eth-capacity-short")));
  assert.equal(capacityOf({ dorm: [{ lives: [], wakes: [], cap: [] }] }), null);
  // /status shows it
  const body = statusBody({}, { ethnft: { capacity: cap } });
  assert.equal(body.eth_capacity.effective_max_active, 0);
  assert.equal(body.eth_capacity.wake_headroom, 0);
});

test("§11: an upgrade that turns the scaling off or lowers its estimates is worded as loosening", () => {
  const d = decodeInit(FIX("init-dormant.hex"));
  const off = structuredClone(d.init);
  off.eth_nft[0].dormancy[0].scaling = [];
  assert.ok(diffInit(d.init, off).lines.some((l) => l.sev === "WARN" && /scaling: on → off/.test(l.text)));
  assert.ok(diffInit(off, d.init).lines.some((l) => l.sev === "WARN" && /scaling: off → ON .*2\.5 T a day per awake collection, 7 days of runway, 8 T reserve/.test(l.text)));
  const low = structuredClone(d.init);
  low.eth_nft[0].dormancy[0].scaling[0].active_b_per_day = 1500;
  low.eth_nft[0].dormancy[0].scaling[0].reserve_b = 9000;
  const lines = diffInit(d.init, low).lines.map((l) => `${l.sev} ${l.text}`);
  assert.ok(lines.some((l) => /^WARN eth_nft\.dormancy\.scaling\.active_b_per_day: 2500 → 1500/.test(l)), lines.join("\n"));
  assert.ok(lines.some((l) => /^INFO eth_nft\.dormancy\.scaling\.reserve_b: 8000 → 9000/.test(l)), lines.join("\n"));
});

// ---- the no-good-post pause (evm.mjs) -----------------------------------------------------------------------------------------------

const A = (n) => `0x${n.toString(16).padStart(40, "0")}`;
const MODULE = A(0xbb), POSTER = A(0xf5), GUARDIAN = A(0x9a);
const NFT_STACK = { chainId: 4663, registry: A(0xd1), treasury: A(0xc7), poolManager: A(0xc8), keeper: A(0xc3), fromBlock: 1,
  collections: [{ slug: "milady", kind: "nft", wrapper: A(0xaa), floorModule: MODULE, raffleVault: A(0xe1), burnVault: A(0xe2), yoyoVault: A(0xe3),
    factory: A(0xf1), lock: A(0xf2), router: A(0xf3), mintDesk: A(0xf4), floorPoster: A(0xc0), posterSource: POSTER,
    custody: { kind: "evm", chainId: 1, collection: MILADY, custodian: A(0xc0) } }] };
const NOW = 1_790_000_000_000;

async function noGoodPost(sleeping) {
  const lastGood = BigInt(NOW / 1000 - 13 * 3600);
  const w = fakeChain({
    views: (url, to, fn) => { if (fn === "lastGoodAt") return lastGood; if (fn === "drawCount") return 0n; throw new Error(`no view ${fn}`); },
    other: (url, method) => { if (method === "eth_blockNumber") return hex(71_200_200); if (method === "eth_getLogs") return []; throw new Error(method); },
  });
  const done = [];
  const g = { address: GUARDIAN, autoPause: true, autoCancel: true, pauseKeeper: async () => { done.push("pause"); return { done: true, tx: "0x1" }; },
    pausePoster: async (p) => { done.push(`pause-poster ${p}`); return { done: true, tx: "0x2" }; }, cancelDraw: async () => ({ done: true }) };
  const cfg = { rpcs: ["https://a.example", "https://b.example"], stack: NFT_STACK, from_block: 71_200_000, confirmations: 20,
    guardian: { address: GUARDIAN, pause_on_no_good_post_h: 6 } };
  const r = await checkEvm({ cfg, prev: {}, now: NOW, http: w.http, readRecords: async () => EMPTY_RECORDS, timelock: "tl", guardian: g, sleeping });
  return { done, alerts: r.alerts };
}

test("the no-good-post pause never pauses the poster of a collection the canister keeps DORMANT or WAKING", async () => {
  // an ACTIVE collection 13 h without a good post: paged, and its own poster paused (unchanged)
  const active = await noGoodPost(async () => new Set());
  assert.deepEqual(active.done, [`pause-poster ${POSTER}`]);
  // the canister keeps it asleep: no page, no pause (a paused PosterSource needs the Safe, and would wedge the next wake)
  const asleep = await noGoodPost(async () => new Set([MODULE]));
  assert.deepEqual(asleep.done, []);
  assert.ok(!asleep.alerts.some((a) => /NO GOOD/.test(a.text)));
  // the canister's lives cannot be read: paged SEV1 for a human, never paused automatically
  const unsure = await noGoodPost(async () => { throw new Error("the buyer does not answer"); });
  assert.deepEqual(unsure.done, []);
  assert.ok(unsure.alerts.some((a) => a.sev === "SEV1" && /may be DORMANT on purpose/.test(a.text)));
  // without a canister (no eth_nft configured) the old rule stands
  const legacy = await noGoodPost(null);
  assert.deepEqual(legacy.done, [`pause-poster ${POSTER}`]);
});

// ---- the committed launch plan: ten collections, a binding global (docs/v7/dormant-collections.md §6) ----------------------------------

import { encodeInitPlain } from "../src/candid.mjs";
import { checkStagedProposal, stagedStep, stepNumbers } from "../src/staged.mjs";
import { exitProblemsOf } from "../src/argdiff.mjs";

const LAUNCH = JSON.parse(readFileSync(new URL("../../deployments/launch/nft-pilot.json", import.meta.url), "utf8"));
function initAt(step, { gItems = null, gValue = null } = {}) {
  const s = stepNumbers(LAUNCH, step);
  const collections = Object.entries(s.caps).map(([collection, c]) => ({ collection, max_items: Number(c.items), max_value_wei: String(c.value), max_item_wei: String(c.item) }));
  const v = { chain_id: 4663n, vault: "0x00", rpc_urls: ["https://a", "https://b"], btc_api_urls: ["c", "d"], key_name: "key_1", accept_production_key_name: true,
    bridge_allowlist: [], btc_network: "mainnet", operator: [], wrapper: [], evm_chains: [], holder_deposits: [], wrapper_from_block: [], frog_funding: [],
    custody_caps: [{ collections, global_max_items: gItems ?? Number(s.global.items), global_max_value_wei: String(gValue ?? s.global.value) }],
    outflow_limits: [{ ...Object.fromEntries(Object.entries(s.limits).map(([k, x]) => [k, k === "global_value_wei" ? String(x) : Number(x)])) }],
    eth_nft: [{ chain_id: 1n, relay_max_fee_bps: 300, deposits: true, floor: [], gas_pot_target_wei: [], logs_max_blocks: [],
      dormancy: [{ max_active: 3, max_waking: 1, wakes_per_day: 4, wake_timeout_secs: 43_200, cooldown_secs: 86_400, idle_days: 3, wake_desk: [] }],
      collections: LAUNCH.collections.map((c) => ({ collection: c.collection, wrapper: "0xaa", module: "0xbb", max_purchase_wei: String(c.maxPurchaseWei), max_premium_bps: 500 })) }] };
  const d = decodeInit(encodeInitPlain(v));
  assert.ok(d.ok, d.error);
  return d;
}

test("the committed plan: eleven collections (Identity MD since 2026-10-09), the global named and binding (48, 48, 62 items; 75 ETH: Bitcoin Frogs' 3 inside it, nft-pilot.json _bitcoin), each step inside the exit rule", () => {
  assert.equal(LAUNCH.collections.length, 11);
  // Identity MD (dormant): J7's min(24, 15 / 1.78) = 8 at every step, within 2 x per_collection_items
  assert.deepEqual(LAUNCH.stages.map((s) => s.custodyItems.identitymd), [8, 8, 8]);
  assert.deepEqual(LAUNCH.stages.map((s) => [s.step, s.globalItems, s.outflow.global_items]), [[1, 48, 24], [2, 48, 31], [3, 62, 31]]);
  for (const n of [1, 2, 3]) {
    const s = stepNumbers(LAUNCH, n);
    assert.ok(s.binding);
    assert.ok(Object.values(s.caps).reduce((a, c) => a + c.items, 0n) > s.global.items, `step ${n}: the collections' sum is over the global: it binds`);
    assert.deepEqual(exitProblemsOf(s.caps, s.global.items, s.limits), [], `step ${n} fits the exit rule on min(sum, global)`);
  }
  assert.deepEqual(LAUNCH.dormancy, { max_active: 10, max_waking: 1, wakes_per_day: 12, wake_timeout_secs: 43_200, cooldown_secs: 86_400, idle_days: 3,
    scaling: { active_b_per_day: 2500, idle_b_per_day: 300, wake_b: 1500, runway_days: 7, reserve_b: 8000, exit_b_per_awake: 3000 } });
});

test("the staged proposals of the committed plan pass; a step whose global is the sum (J7's rule, not the plan's) does not", () => {
  const s1 = initAt(1), s2 = initAt(2), s3 = initAt(3);
  const r2 = checkStagedProposal({ next: s2, base: s1, launch: LAUNCH, step: 2, running: null });
  assert.ok(r2.ok, r2.lines.join("\n"));
  const r3 = checkStagedProposal({ next: s3, base: s2, launch: LAUNCH, step: 3, running: null });
  assert.ok(r3.ok, r3.lines.join("\n"));
  assert.equal(stagedStep(LAUNCH, s1.init, s2.init), 2);
  assert.equal(stagedStep(LAUNCH, s2.init, s3.init), 3);
  // the global as the collections' sum (189 at step 3, Identity MD's 8 in it): the exit rule and the plan both refuse it
  const sum = initAt(3, { gItems: 189 });
  const bad = checkStagedProposal({ next: sum, base: s2, launch: LAUNCH, step: 3, running: null });
  assert.ok(!bad.ok && bad.lines.some((l) => /^FAIL .*global item cap 189 is not the step's 62/.test(l)) && bad.lines.some((l) => /^FAIL .*exit rule/.test(l)), bad.lines.join("\n"));
  assert.equal(stagedStep(LAUNCH, s2.init, sum.init), null);
});
