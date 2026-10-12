// Wave 2 (docs/audit/2026-09-26/fixes-W2-monitor.md §1.1, §1.3, §1.4): the keeper's events on chain, as foundry's `cast`
// encoded them (fixtures/stack-logs.json), judged; a draw's root recomputed against tools/snapshot's committed fixture and the
// sentinel as `cast` hashes it; the guardian acting only on confirmed triggers. No network.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { decodeStackEvent } from "../src/abi.mjs";
import { balancesOf, merkleRoot, recompute, ticketLeaf, transferLogs, verdict, weightedTree } from "../src/draws.mjs";
import { checkEvm, judgeStackLogs } from "../src/evm.mjs";
import { EMPTY_RECORDS } from "../src/records.mjs";
import { exclusionsFor, stackOf, watchedAddresses } from "../src/stack.mjs";
import { fakeChain, hex } from "./fakechain.mjs";

import { retryPolicy } from "../src/abi.mjs";
retryPolicy.baseMs = 0;                                   // the fakes answer "busy" for a refused range: no real backoff in tests
const S = JSON.parse(readFileSync(new URL("../fixtures/stack-logs.json", import.meta.url), "utf8"));
const SNAP = JSON.parse(readFileSync(new URL("../../test/fixtures/snapshot-weighted.json", import.meta.url), "utf8"));
const L = S.logs, st = stackOf({ stack: S.stack }), c = st.collections[0];
const E18 = 10n ** 18n, NOW = 1_790_000_000_000;
const judge = (logs, prev, ctx = {}) => judgeStackLogs({ logs, prev, ctx: { st, now: NOW, minConf: { [c.module]: 3000n }, accepted: { [c.module]: 65n * E18 / 100n }, ...ctx } });

test("the stack file names what is watched: registry, wrapper, module, and all three vaults", () => {
  assert.deepEqual(watchedAddresses(st).sort(), [S.stack.registry, c.wrapper, c.module, c.raffle, c.burn, c.yoyo].sort());
  const ex = exclusionsFor(st, c, S.coin, "0x00000000000000000000000000000000000000c2", [{ address: "0x8366a39CC670B4001A1121B8F6A443A643e40951" }]);
  for (const a of [S.coin, "0x00000000000000000000000000000000000000c2", c.factory, c.lock, c.router, c.raffle, c.burn, c.yoyo, c.module, c.desk, st.treasury, st.registry, st.poolManager])
    assert.ok(ex.includes(a), `${a} is excluded, as keeper/service/draws.ts exclusionsFor does`);
  assert.equal(new Set(ex).size, ex.length);
});

test("cast-encoded stack events decode", () => {
  const fp = decodeStackEvent(L.floor_clamped_1);
  assert.equal(fp.eventName, "FloorPosted");
  assert.deepEqual([fp.args.target, fp.args.accepted, fp.args.confidenceBps], [40n * E18 / 100n, 6175n * E18 / 10000n, 8000]);
  const d = decodeStackEvent(L.draw_opened);
  assert.deepEqual([d.eventName, d.args.drawId, d.args.totalTickets, d.args.uri], ["DrawOpened", 0n, 1000n, "https://keeper.example/snapshots/s.json"]);
  assert.equal(decodeStackEvent({ topics: ["0x" + "00".repeat(32)], data: "0x" }), null);
});

test("FloorPosted: a good post is quiet; low confidence is SEV2; a post the module clamped is SEV1; two clamped the same way within 6 h trigger the guardian", () => {
  assert.deepEqual(judge([L.floor_ok]).alerts, []);
  const low = judge([L.floor_lowconf]);
  assert.deepEqual(low.alerts.map((a) => [a.kind, a.sev]), [["keeper", "SEV2"]]);
  assert.match(low.alerts[0].text, /confidence 0 bps, below the module's minimum 3000/);
  const one = judge([L.floor_clamped_1], {}, { times: { [`${L.floor_clamped_1.transactionHash}:0`]: NOW - 3 * 3600_000 } });
  assert.deepEqual(one.alerts.map((a) => [a.kind, a.sev, a.persistent]), [["keeper", "SEV1", true]]);
  assert.match(one.alerts[0].text, /OUTSIDE THE MODULE'S BOUNDS: target 0\.4 ETH, accepted 0\.6175 ETH/);
  assert.deepEqual(one.triggers, [], "one clamped post pages; it does not pause");
  const two = judge([L.floor_clamped_2], one.next);
  assert.equal(two.triggers.length, 1);
  assert.deepEqual([two.triggers[0].action, /runaway frogs floor: 2 posts .* all down/.test(two.triggers[0].why)], ["pause", true]);
  // the same two posts a day apart are not a runaway
  const apart = judge([L.floor_clamped_2], judge([L.floor_clamped_1], {}, { times: { [`${L.floor_clamped_1.transactionHash}:0`]: NOW - 24 * 3600_000 } }).next);
  assert.deepEqual(apart.triggers, []);
});

test("CapPosted: 110% of the floor is quiet; above 125% is SEV2; ANY cap on the YOYO vault is SEV1", () => {
  assert.deepEqual(judge([L.cap_raffle]).alerts, []);
  assert.deepEqual(judge([L.cap_high]).alerts.map((a) => [a.sev, /BURN cap of 1 ETH, 153\.8% of the accepted floor/.test(a.text)]), [["SEV2", true]]);
  assert.deepEqual(judge([L.cap_yoyo]).alerts.map((a) => [a.sev, /YOYO VAULT/.test(a.text)]), [["SEV1", true]]);
});

test("draws, the pause, the guardian and role rotations", () => {
  const ann = judge([L.draw_announced]);
  assert.deepEqual(ann.alerts.map((a) => [a.kind, a.sev]), [["draw", "INFO"]]);
  const opened = judge([L.snapshot_fixed, L.draw_opened], ann.next);
  const d = opened.next.draws[`${c.raffle}:0`];
  assert.deepEqual([d.status, d.root, d.totalTickets, d.snapshotAt, d.challengeEnds], ["pending", `0x${"ab".repeat(32)}`, "1000", "1789990000", 1_790_000_900]);
  const p = judge([L.keeper_paused], {}, { guardianAddress: S.guardian });
  assert.deepEqual(p.alerts.map((a) => [a.kind, a.sev]), [["keeper", "SEV1"]]);
  assert.match(p.alerts[0].text, /THE KEEPER IS PAUSED, by 0x0+a9 \(this monitor's guardian\)/);
  assert.deepEqual(judge([L.guardian_set]).alerts.map((a) => a.sev), ["SEV2"]);
  const r = judge([L.roles_proposed]).alerts;
  assert.deepEqual(r.map((a) => [a.sev, a.persistent]), [["SEV1", true]]);
  assert.match(r[0].text, /A ROLE ROTATION WAS PROPOSED: keeper 0x0+c4/);
});

// ---- the root ---------------------------------------------------------------------------------------------------------------

test("the tree is tools/snapshot's: its committed weighted fixture is reproduced leaf for leaf and root for root", () => {
  const bal = new Map(SNAP.entries.map((e) => [e.holder, BigInt(e.balance)]));
  const t = weightedTree(bal, SNAP.inputs.exclusions);
  assert.equal(t.root, SNAP.root);
  assert.equal(String(t.totalTickets), SNAP.totalTickets);
  assert.deepEqual([...t.leaves].sort(), SNAP.entries.map((e) => e.leaf).sort());
  assert.equal(ticketLeaf(SNAP.entries[0].holder, BigInt(SNAP.entries[0].first), BigInt(SNAP.entries[0].end)), SNAP.entries[0].leaf);
});

test("v2: the root is tools/snapshot v2's committed fixture (the keeper's builder), sentinel and all, byte for byte", () => {
  const V2 = JSON.parse(readFileSync(new URL("../../test/fixtures/snapshot-weighted-v2.json", import.meta.url), "utf8"));
  const bal = new Map(V2.entries.map((e) => [e.holder, BigInt(e.balance)]));
  const t = weightedTree(bal, V2.inputs.exclusions, { sentinel: { snapshotAt: BigInt(V2.inputs.snapshotAt) } });
  assert.equal(t.root, V2.root);
  assert.equal(String(t.totalTickets), V2.totalTickets);
  assert.ok(t.leaves.includes(V2.sentinel.leaf), "the sentinel leaf is the tool's");
});

test("v2's sentinel leaf is SweepVault.leaf(address(0), totalTickets, snapshotAt), as cast hashes it, and it moves the root", () => {
  assert.equal(ticketLeaf("0x0000000000000000000000000000000000000000", 1000n, 1_789_990_000n), S.sentinel_1000_1789990000);
  const bal = new Map(SNAP.entries.map((e) => [e.holder, BigInt(e.balance)]));
  const v1 = weightedTree(bal, SNAP.inputs.exclusions), v2 = weightedTree(bal, SNAP.inputs.exclusions, { sentinel: { snapshotAt: 1_789_990_000n } });
  assert.notEqual(v2.root, v1.root);
  assert.equal(v2.root, merkleRoot([...v1.leaves, S.sentinel_1000_1789990000]));
  assert.notEqual(weightedTree(bal, SNAP.inputs.exclusions, { sentinel: { snapshotAt: 1_789_990_001n } }).root, v2.root, "the time is committed");
  assert.throws(() => balancesOf([{ blockNumber: 1n, logIndex: 0, from: "0x" + "1".repeat(40), to: "0x" + "2".repeat(40), value: 1n }]), /logs are missing/);
});

/** A chain with one block a second from `base`, a coin whose Transfer logs are `transfers`, and ordofi's 10,000-block limit. */
function coinChain({ base = 71_200_000, t0 = 1_789_900_000, transfers, maxSpan = 10_000 }) {
  const TRANSFER = "0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef";
  const pad = (a) => `0x${a.slice(2).padStart(64, "0")}`;
  return fakeChain({ other: (url, method, params) => {
    if (method === "eth_getBlockByNumber") return { number: params[0], timestamp: hex(t0 + Number(BigInt(params[0])) - base) };
    if (method === "eth_getLogs") {
      const a = Number(BigInt(params[0].fromBlock)), b = Number(BigInt(params[0].toBlock));
      if (b - a + 1 > maxSpan) throw new Error("the network is busy, please try again in a moment");
      return transfers.filter((x) => x.block >= a && x.block <= b).map((x, i) => ({ blockNumber: hex(x.block), logIndex: hex(i), topics: [TRANSFER, pad(x.from), pad(x.to)], data: pad(hex(x.value)), removed: false }));
    }
    throw new Error(`unexpected ${method}`);
  } });
}

test("recompute: the snapshot block by time, the Transfer logs paged under a 10,000-block limit, the weighted tree with its sentinel", async () => {
  const Z = "0x0000000000000000000000000000000000000000", H1 = "0x" + "1".repeat(40), H2 = "0x" + "2".repeat(40), CURVE = "0x" + "3".repeat(40);
  const transfers = [{ block: 71_200_010, from: Z, to: CURVE, value: 1000n * E18 }, { block: 71_230_000, from: CURVE, to: H1, value: 300n * E18 },
    { block: 71_260_000, from: CURVE, to: H2, value: 50n * E18 }, { block: 71_290_001, from: H1, to: H2, value: 100n * E18 }];   // the last one is after the snapshot
  const { http, calls } = coinChain({ transfers });
  const snapshotAt = 1_789_900_000 + 90_000;                       // block 71,290,000
  const r = await recompute("https://ordofi.example", { coin: S.coin, snapshotAt: BigInt(snapshotAt), fromBlock: 71_200_000, head: 71_300_000, exclusions: [Z, CURVE] }, http, { span: 50_000 });
  assert.equal(r.block, 71_290_000);
  const want = weightedTree(new Map([[H1, 300n * E18], [H2, 50n * E18]]), [], { sentinel: { snapshotAt } });
  assert.deepEqual([r.root, r.totalTickets], [want.root, 350n]);
  assert.ok(calls.filter((x) => x.method === "eth_getLogs").every((x) => Number(BigInt(x.params[0].toBlock)) <= 71_290_000), "no log past the snapshot block");
  const got = await transferLogs("https://x", S.coin, 71_200_000, 71_290_000, http, { span: 40_000 });
  assert.equal(got.length, 3, "a refused page is asked again in halves, and nothing is lost");
});

test("verdict: both reproduce the chain → verified; both the same other root → bad (confirmed); they differ → disagree; one failed → pending", () => {
  const onChain = { root: "0xaa", totalTickets: "10", snapshotAt: "1" };
  const ok = { root: "0xaa", totalTickets: 10n, block: 5 }, other = { root: "0xbb", totalTickets: 10n, block: 5 };
  assert.equal(verdict(onChain, ok, ok).v, "verified");
  assert.equal(verdict(onChain, other, other).v, "bad");
  assert.equal(verdict(onChain, { ...ok, totalTickets: 9n }, { ...ok, totalTickets: 9n }).v, "bad", "a truncated count is a bad root too");
  assert.equal(verdict(onChain, ok, other).v, "disagree");
  assert.equal(verdict(onChain, { error: "x" }, ok).v, "pending");
});

// ---- the whole poll, with the guardian -------------------------------------------------------------------------------------

function pollWorld({ logs, accepted = 65n * E18 / 100n, headNum = 71_200_200 }) {
  return fakeChain({
    views: (url, to, fn) => {
      if (fn === "minConfidenceBps") return 3000n;
      if (fn === "accepted") return accepted;
      if (fn === "routerOf") return "0x00000000000000000000000000000000000000c2";
      throw new Error(`no view ${fn}`);
    },
    other: (url, method, params) => {
      if (method === "eth_blockNumber") return hex(headNum);
      if (method === "eth_getLogs") { const a = Number(BigInt(params[0].fromBlock)), b = Number(BigInt(params[0].toBlock)); return logs.filter((l) => Number(BigInt(l.blockNumber)) >= a && Number(BigInt(l.blockNumber)) <= b); }
      if (method === "eth_getBlockByNumber") return { timestamp: hex(NOW / 1000 - 3600) };
      throw new Error(`unexpected ${method}`);
    },
  });
}
const evmCfg = { rpcs: ["https://a.example", "https://b.example"], stack: S.stack, from_block: 71_200_000, confirmations: 20 };
const fakeGuardian = () => { const done = []; return { done, address: S.guardian, autoPause: true, autoCancel: true,
  pauseKeeper: async () => { done.push("pause"); return { done: true, tx: "0x" + "7".repeat(64) }; },
  cancelDraw: async (v, id) => { done.push(`cancel ${v} ${id}`); return { done: true, tx: "0x" + "8".repeat(64) }; } }; };

test("a confirmed bad root: SEV1, the guardian pauses the keeper FIRST and then cancels the draw, once (R2-L3)", async () => {
  const { http } = pollWorld({ logs: [L.draw_announced, L.snapshot_fixed, L.draw_opened] });
  const g = fakeGuardian(), seen = [];
  const bad = async (url, input) => { seen.push([url, input.snapshotAt, input.exclusions.length]); return { root: `0x${"cd".repeat(32)}`, totalTickets: 1000n, block: 71_199_000, holders: 3 }; };
  const r = await checkEvm({ cfg: evmCfg, prev: {}, now: NOW, http, readRecords: async () => EMPTY_RECORDS, timelock: "tl", guardian: g, recompute: bad });
  assert.deepEqual(seen.map((x) => x[0]), ["https://a.example", "https://b.example"], "recomputed on BOTH RPCs");
  assert.equal(seen[0][1], 1_789_990_000n, "at the snapshot time the chain fixed");
  assert.deepEqual(g.done, ["pause", `cancel ${c.raffle} 0`], "pause first: a paused keeper's draw cannot be pinned");
  assert.ok(r.alerts.some((a) => a.kind === "draw" && a.sev === "SEV1" && /A BAD DRAW ROOT/.test(a.text)));
  assert.equal(r.alerts.filter((a) => a.kind === "guardian").length, 2);
  assert.match(r.alerts.filter((a) => a.kind === "guardian")[0].text, /THE GUARDIAN PAUSED THE KEEPER/);
  assert.match(r.alerts.filter((a) => a.kind === "guardian")[1].text, /THE GUARDIAN CANCELLED DRAW 0/);
  // next poll: the draw is judged, the triggers are spent
  const again = await checkEvm({ cfg: evmCfg, prev: r.next, now: NOW + 60_000, http, readRecords: async () => EMPTY_RECORDS, timelock: "tl", guardian: g, recompute: bad });
  assert.deepEqual(g.done.length, 2, "never twice");
  assert.equal(again.next.draws[`${c.raffle}:0`].status, "bad");
});

test("a root both RPCs reproduce is verified and nothing is done; recomputations that differ page a disagreement and do nothing", async () => {
  const { http } = pollWorld({ logs: [L.draw_announced, L.snapshot_fixed, L.draw_opened] });
  const g = fakeGuardian();
  const good = async () => ({ root: `0x${"ab".repeat(32)}`, totalTickets: 1000n, block: 71_199_000, holders: 3 });
  const r = await checkEvm({ cfg: evmCfg, prev: {}, now: NOW, http, readRecords: async () => EMPTY_RECORDS, timelock: "tl", guardian: g, recompute: good });
  assert.ok(r.alerts.some((a) => /the root is reproduced on both RPCs/.test(a.text)));
  assert.deepEqual(g.done, []);
  let n = 0;
  const split = async () => ({ root: `0x${(n++ ? "cd" : "ab").repeat(32)}`, totalTickets: 1000n, block: 71_199_000, holders: 3 });
  const d = await checkEvm({ cfg: evmCfg, prev: {}, now: NOW, http, readRecords: async () => EMPTY_RECORDS, timelock: "tl", guardian: g, recompute: split });
  assert.ok(d.alerts.some((a) => a.kind === "provider-disagreement" && /recomputations differ/.test(a.text)));
  assert.deepEqual(g.done, [], "no automatic action on a disagreement");
});

test("no guardian (the operator's choice): a confirmed trigger pages ACT BY HAND with the Safe's exact call; a runaway floor pauses when there is one", async () => {
  const { http } = pollWorld({ logs: [L.floor_clamped_1, L.floor_clamped_2] });
  const r = await checkEvm({ cfg: evmCfg, prev: {}, now: NOW, http, readRecords: async () => EMPTY_RECORDS, timelock: "tl", guardian: null });
  const hand = r.alerts.find((a) => a.kind === "guardian");
  assert.equal(hand.sev, "SEV1");
  assert.match(hand.text, /ACT BY HAND: a runaway frogs floor.*No guardian is configured.*cast send 0x0+d1 "pauseKeeper\(\)"/s);
  const g = fakeGuardian();
  await checkEvm({ cfg: evmCfg, prev: {}, now: NOW, http, readRecords: async () => EMPTY_RECORDS, timelock: "tl", guardian: g });
  assert.deepEqual(g.done, ["pause"]);
  const off = { ...fakeGuardian(), autoPause: false };
  const r2 = await checkEvm({ cfg: evmCfg, prev: {}, now: NOW, http, readRecords: async () => EMPTY_RECORDS, timelock: "tl", guardian: off });
  assert.deepEqual(off.done, [], "auto_pause false: the key is kept for manual use");
  assert.match(r2.alerts.find((a) => a.kind === "guardian").text, /configured not to act on this/);
});

test("a draw not verified 10 minutes before its window ends is said (SEV2), 6 minutes before it is SEV1 and the guardian pauses, and a failing guardian says ACT BY HAND NOW", async () => {
  const { http } = pollWorld({ logs: [L.draw_announced, L.snapshot_fixed, L.draw_opened] });
  const failing = async () => { throw new Error("rpc down"); };
  const r = await checkEvm({ cfg: evmCfg, prev: {}, now: (1_790_000_900 - 500) * 1000, http, readRecords: async () => EMPTY_RECORDS, timelock: "tl", recompute: failing });
  assert.ok(r.alerts.some((a) => a.kind === "draw" && a.sev === "SEV2" && /NOT verified and its challenge window ends/.test(a.text)));
  const g = fakeGuardian();
  const late = await checkEvm({ cfg: evmCfg, prev: r.next, now: (1_790_000_900 - 300) * 1000, http, readRecords: async () => EMPTY_RECORDS, timelock: "tl", recompute: failing, guardian: g });
  assert.ok(late.alerts.some((a) => a.sev === "SEV1" && /DRAW 0 \(frogs, coin .*\) NOT VERIFIED BEFORE ITS PIN AT 2026-/.test(a.text)));
  assert.deepEqual(g.done, ["pause"], "fail closed: the keeper is paused, so the draw cannot be pinned");
  const broken = { ...fakeGuardian(), cancelDraw: async () => { throw new Error("the guardian will not act: the registry's guardian is 0x0"); } };
  const bad = async () => ({ root: `0x${"cd".repeat(32)}`, totalTickets: 1000n, block: 1, holders: 1 });
  const b = await checkEvm({ cfg: evmCfg, prev: {}, now: NOW, http, readRecords: async () => EMPTY_RECORDS, timelock: "tl", guardian: broken, recompute: bad });
  assert.match(b.alerts.find((a) => a.key.startsWith("guardian-failed")).text, /THE GUARDIAN FAILED to cancel: the guardian will not act.*ACT BY HAND NOW: .*cancelDraw\(uint256\)" 0/s);
});
