// Second review (R2), 2026-09-26: PoCs for the monitor's wave-2 fixes. Each test asserts the behaviour FOUND (a gap), so a
// passing test means the gap is present. No network.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { encodeAbiParameters, encodeEventTopics } from "viem";
import { MODULE } from "../src/abi.mjs";
import { recompute } from "../src/draws.mjs";
import { checkEvm, judgeStackLogs } from "../src/evm.mjs";
import { EMPTY_RECORDS } from "../src/records.mjs";
import { stackOf } from "../src/stack.mjs";
import { fakeChain, hex } from "./fakechain.mjs";

const S = JSON.parse(readFileSync(new URL("../fixtures/stack-logs.json", import.meta.url), "utf8"));
const L = S.logs, st = stackOf({ stack: S.stack }), c = st.collections[0];
const E18 = 10n ** 18n, NOW = 1_790_000_000_000;
const evmCfg = { rpcs: ["https://a.example", "https://b.example"], stack: S.stack, from_block: 71_200_000, confirmations: 20 };
const guardianSpy = () => { const done = []; return { done, address: S.guardian, autoPause: true, autoCancel: true,
  pauseKeeper: async () => { done.push("pause"); return { done: true, tx: "0x" + "7".repeat(64) }; },
  cancelDraw: async (v, id) => { done.push(`cancel ${v} ${id}`); return { done: true, tx: "0x" + "8".repeat(64) }; } }; };

function world({ logs, headNum = 71_200_200, getLogsB = null }) {
  return fakeChain({
    views: (url, to, fn) => {
      if (fn === "minConfidenceBps") return 3000n;
      if (fn === "accepted") return 65n * E18 / 100n;
      if (fn === "routerOf") return "0x00000000000000000000000000000000000000c2";
      throw new Error(`no view ${fn}`);
    },
    other: (url, method, params) => {
      if (method === "eth_blockNumber") return hex(headNum);
      if (method === "eth_getLogs") {
        if (getLogsB && url.includes("b.example")) return getLogsB(params);
        const a = Number(BigInt(params[0].fromBlock)), b = Number(BigInt(params[0].toBlock));
        return logs.filter((l) => Number(BigInt(l.blockNumber)) >= a && Number(BigInt(l.blockNumber)) <= b);
      }
      if (method === "eth_getBlockByNumber") return { timestamp: hex(NOW / 1000 - 3600) };
      throw new Error(`unexpected ${method}`);
    },
  });
}

test("R2-M1a: RPC B answering -32005 to eth_getLogs (as rpc.ordofi.network does) fails the WHOLE 4663 checker: a forged draw is never recomputed and the guardian never acts", async () => {
  const { http } = world({ logs: [L.draw_announced, L.snapshot_fixed, L.draw_opened], getLogsB: () => { throw new Error("the network is busy, please try again in a moment"); } });
  const g = guardianSpy();
  let recomputed = 0;
  const bad = async () => { recomputed++; return { root: `0x${"cd".repeat(32)}`, totalTickets: 1000n, block: 71_199_000, holders: 3 }; };
  await assert.rejects(checkEvm({ cfg: evmCfg, prev: {}, now: NOW, http, readRecords: async () => EMPTY_RECORDS, timelock: "tl", guardian: g, recompute: bad }),
    /network is busy/, "one throttled chunk on B throws the poll; there is no retry or failover in abi.mjs rpc()");
  assert.equal(recomputed, 0);
  assert.deepEqual(g.done, [], "no cancel, no pause: the only signal is main.mjs's checker-down after STALE_MS (30 min), past the 15-min challenge window");
});

test("R2-M1b: the real recompute pages 10,000 blocks at a time from the stack's deploy block on every poll; past 5,000 pages (≈58 days of 4663 at ~9.9 blocks/s) it can never finish", async () => {
  let pages = 0;
  const { http } = fakeChain({ other: (url, method, params) => {
    if (method === "eth_getBlockByNumber") { const n = Number(BigInt(params[0])); return { timestamp: hex(1_700_000_000 + Math.floor(n / 10)) }; }
    if (method === "eth_getLogs") { pages++; return []; }
    throw new Error(method);
  } });
  const from = 71_200_000, days = 60, head = from + days * 858_000;
  const snapshotAt = 1_700_000_000 + Math.floor((head - 1000) / 10);
  await assert.rejects(recompute("https://a.example", { coin: S.coin, snapshotAt: BigInt(snapshotAt), fromBlock: from, head, exclusions: [] }, http),
    /more than 5000 log pages/);
  assert.equal(pages, 5000, "5,000 sequential eth_getLogs before giving up, per provider, per draw, per poll");
  // measured 2026-09-26: rpc.ordofi.network takes 7-11 s per 10,000-block eth_getLogs; at day 2 (~172 pages) that is ~20-30 min > the 15-min window
});

test("R2-M1c: without evm.from_block the monitor starts from the stack file's deployedAtBlock (the PARENT chain's block.number), not deploy-stack.sh's L2 fromBlock", () => {
  const s = stackOf({ stack: { ...S.stack, fromBlock: 72_000_000, deployedAtBlock: 26_059_685 } });
  assert.equal(s.fromBlock, 26_059_685, "the keeper (keeper/service/stack.ts) reads fromBlock first; the monitor ignores it");
});

test("R2-M2: two draws that cannot be recomputed hold both DRAWS_PER_POLL slots; a third (forged) draw is never recomputed before its window ends", async () => {
  const { http } = world({ logs: [L.draw_announced, L.snapshot_fixed, L.draw_opened] });
  const g = guardianSpy();
  const stuck = (id) => ({ vault: c.raffle, id: String(id), coin: "0x00000000000000000000000000000000000c0de" + id, slug: "frogs", root: "0x" + "11".repeat(32),
    totalTickets: "5", uri: "https://x/y", challengeEnds: Math.floor(NOW / 1000) + 600, snapshotAt: "1789990000", status: "pending", tries: 3 });
  const prev = { cursor: 71_200_000, draws: { [`${c.raffle}:7`]: stuck(7), [`${c.raffle}:8`]: stuck(8) } };
  const seen = [];
  const rec = async (url, input) => { seen.push(input.coin); if (input.coin !== S.coin) throw new Error("the network is busy"); return { root: `0x${"cd".repeat(32)}`, totalTickets: 1000n, block: 71_199_000, holders: 3 }; };
  const r = await checkEvm({ cfg: evmCfg, prev, now: NOW, http, readRecords: async () => EMPTY_RECORDS, timelock: "tl", guardian: g, recompute: rec });
  assert.ok(!seen.includes(S.coin), "the forged draw 0 was not recomputed this poll");
  assert.equal(r.next.draws[`${c.raffle}:0`].status, "pending");
  assert.deepEqual(g.done, []);
  // 5 minutes before the forged draw's window ends (the fixture's challengeEnds is 1_790_000_900): still not recomputed, and not even
  // the SEV2 "NOT verified" page, which is only raised for the draws in the two slots
  const late = await checkEvm({ cfg: evmCfg, prev: r.next, now: (1_790_000_900 - 300) * 1000, http, readRecords: async () => EMPTY_RECORDS, timelock: "tl", guardian: g, recompute: rec });
  assert.ok(!late.alerts.some((a) => a.key === `draw-unverified:${c.raffle}:0`), "no page for the forged draw");
  assert.ok(late.alerts.some((a) => a.key === `draw-unverified:${c.raffle}:7`), "the stuck draws are the ones paged");
  assert.deepEqual(g.done, []);
});

// FloorPosted(target, accepted, moveBps, confidenceBps, depthItems) as the module emits it
const floorLog = (i, target, accepted, move = 500n, conf = 8000) => ({ address: c.module, topics: encodeEventTopics({ abi: MODULE, eventName: "FloorPosted" }),
  data: encodeAbiParameters([{ type: "uint256" }, { type: "uint256" }, { type: "uint256" }, { type: "uint16" }, { type: "uint32" }], [target, accepted, move, conf, 12]),
  blockNumber: hex(71_300_000 + i * 36_000), transactionHash: `0x${(i + 1).toString(16).padStart(64, "0")}`, logIndex: "0x0", blockHash: `0x${(i + 7).toString(16).padStart(64, "0")}`, removed: false });

test("R2-M3: a stolen key that posts AT the module's floorBounds() edge (target == accepted) walks the floor -5 %/h for a day with no alert and no guardian trigger", () => {
  let f = 65n * E18 / 100n, prev = {}, alerts = 0, triggers = 0;
  const start = f;
  for (let i = 0; i < 24; i++) {
    const next = f * 95n / 100n > start * 80n / 100n ? f * 95n / 100n : start * 80n / 100n;   // what floorBounds() allows: -5 %/h, -20 % a day
    const j = judgeStackLogs({ logs: [floorLog(i, next, next)], prev, ctx: { st, now: NOW + i * 3600_000, minConf: { [c.module]: 3000n }, times: {} } });
    alerts += j.alerts.length; triggers += j.triggers.length; prev = j.next; f = next;
  }
  assert.equal(f, start * 80n / 100n, "the day's full band (-20 %) is walked");
  assert.equal(alerts, 0, "no FloorPosted alert of any severity");
  assert.equal(triggers, 0, "no runaway trigger: the rule needs target ≠ accepted, which a thief reading floorBounds() never posts");
});
