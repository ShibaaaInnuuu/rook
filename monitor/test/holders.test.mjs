// The holder index (src/holders.mjs, fixes-R3-services.md §1.1-§1.3): discovery from Launched, per-provider paging (a provider
// that serves 100-block spans, one that throttles), back-fill of a coin launched later, compaction that keeps the root exact, a
// saved cursor that survives a restart, and the whole poll deciding a draw from two indexes with no injected recompute. No network.
import assert from "node:assert/strict";
import { mkdtempSync, readFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { encodeAbiParameters } from "viem";
import { retryPolicy } from "../src/abi.mjs";
import { balancesOf, weightedTree } from "../src/draws.mjs";
import { checkEvm } from "../src/evm.mjs";
import { HolderIndex, LAUNCHED, TRANSFER, indexesFrom } from "../src/holders.mjs";
import { EMPTY_RECORDS } from "../src/records.mjs";
import { exclusionsFor, stackOf } from "../src/stack.mjs";
import { fakeChain, hex } from "./fakechain.mjs";

retryPolicy.baseMs = 0;
const S = JSON.parse(readFileSync(new URL("../fixtures/stack-logs.json", import.meta.url), "utf8"));
const st = stackOf({ stack: { ...S.stack, fromBlock: 1_000 } }), c = st.collections[0];
const ZERO = "0x0000000000000000000000000000000000000000", E18 = 10n ** 18n, T0 = 1_780_000_000;
const pad = (a) => `0x${a.slice(2).padStart(64, "0")}`;
const A = (n) => `0x${n.toString(16).padStart(40, "0")}`;
const launched = (block, coin, curve, policy = 0) => ({ address: c.factory, blockNumber: block, topics: [LAUNCHED, pad(coin), pad(curve), pad(A(0xcafe))],
  data: encodeAbiParameters([{ type: "uint8" }, { type: "uint256" }, { type: "bool" }, { type: "bool" }, { type: "bool" }], [policy, 0n, false, false, false]) });
const transfer = (block, coin, from, to, value) => ({ address: coin, blockNumber: block, topics: [TRANSFER, pad(from), pad(to)], data: pad(hex(value)) });

/** A chain of one block a second from block 0 at T0; `events` are its logs; each provider has a span limit, and may throttle. */
function chain(events, { head = 20_000, limits = {}, throttle = () => false } = {}) {
  const logs = events.map((l, i) => ({ ...l, blockNumber: hex(l.blockNumber), logIndex: hex(i), transactionHash: pad(hex(i + 1)), removed: false }));
  const state = { head, calls: [] };
  const f = fakeChain({ other: (url, method, params) => {
    if (method === "eth_blockNumber") return hex(state.head);
    if (method === "eth_getBlockByNumber") return { timestamp: hex(T0 + Number(BigInt(params[0]))) };
    if (method === "eth_getLogs") {
      const q = params[0], a = Number(BigInt(q.fromBlock)), b = Number(BigInt(q.toBlock));
      state.calls.push({ url, a, b });
      if (throttle(url)) throw new Error("the network is busy, please try again in a moment");
      const lim = Object.entries(limits).find(([h]) => url.includes(h))?.[1];
      if (lim && b - a + 1 > lim) throw new Error(`ranges over ${lim} blocks are not supported on free plan`);
      const addrs = [].concat(q.address).map((x) => x.toLowerCase()), t0 = [].concat(q.topics?.[0] ?? []).map((x) => x.toLowerCase());
      return logs.filter((l) => addrs.includes(l.address.toLowerCase()) && (!t0.length || t0.includes(l.topics[0])) && Number(BigInt(l.blockNumber)) >= a && Number(BigInt(l.blockNumber)) <= b);
    }
    throw new Error(`unexpected ${method}`);
  } });
  return { ...f, state };
}
const COIN = A(0xc01), CURVE = A(0xc02), COIN2 = A(0xc03), CURVE2 = A(0xc04), H = [A(0x11), A(0x22), A(0x33)];
const EVENTS = [
  launched(1_100, COIN, CURVE), transfer(1_101, COIN, ZERO, CURVE, 1000n * E18),
  transfer(3_000, COIN, CURVE, H[0], 300n * E18), transfer(9_000, COIN, CURVE, H[1], 50n * E18), transfer(9_500, COIN, H[0], H[2], 120n * E18),
  launched(12_000, COIN2, CURVE2), transfer(12_001, COIN2, ZERO, CURVE2, 10n * E18), transfer(12_500, COIN2, CURVE2, H[0], 4n * E18),
  launched(13_000, A(0xb0), A(0xb1), 1),                                                     // a BURN coin: not indexed
  transfer(15_000, COIN, H[2], H[1], 20n * E18),
];
const expected = (coin, block) => balancesOf(EVENTS.filter((l) => l.address === coin && l.blockNumber <= block && l.topics[0] === TRANSFER)
  .map((l, i) => ({ blockNumber: BigInt(l.blockNumber), logIndex: i, from: `0x${l.topics[1].slice(26)}`, to: `0x${l.topics[2].slice(26)}`, value: BigInt(l.data) })));
const idx = (http, o = {}) => new HolderIndex({ url: "https://a.example", span: 10_000, factories: [c.factory], fromBlock: 1_000, confirmations: 0, http, ...o });

test("discovery and balances: RAFFLE coins join from their Launched block, a BURN coin does not, and balances at any block match a full replay", async () => {
  const w = chain(EVENTS);
  const ix = idx(w.http);
  const r = await ix.update({ deadline: Date.now() + 10_000 });
  assert.equal(r.group, 20_000);
  assert.deepEqual(r.behind, []);
  assert.ok(ix.has(COIN) && ix.has(COIN2) && !ix.has(A(0xb0)));
  for (const b of [1_100, 3_000, 9_499, 9_500, 14_999, 15_000, 20_000]) assert.deepEqual(ix.balancesAt(COIN, b), expected(COIN, b), `block ${b}`);
  assert.deepEqual(ix.balancesAt(COIN2, 12_500), expected(COIN2, 12_500), "the coin launched later was back-filled from its launch");
  assert.deepEqual(ix.balancesAt(COIN2, 11_000), new Map(), "before its launch: no holder");
  assert.throws(() => ix.balancesAt(COIN, 20_001), /behind the snapshot block/, "never a guess past what was read");
});

test("a provider that serves 100-block spans (drpc's free plan) is paged at 100 and gets there; its learned span stays", async () => {
  const w = chain(EVENTS, { limits: { "b.example": 100 } });
  const ix = new HolderIndex({ url: "https://b.example", span: 10_000, factories: [c.factory], fromBlock: 1_000, confirmations: 0, http: w.http });
  await ix.update({ deadline: Date.now() + 30_000 });
  assert.equal(ix.summary().group, 20_000);
  assert.ok(ix.summary().ceiling <= 200, `ceiling ${ix.summary().ceiling}: it probes above 100 now and then (R4 M-A: a ceiling recovers), never far`);
  const refused = w.state.calls.filter((x) => x.b - x.a + 1 > 100).length;
  assert.ok(refused <= 20, `${refused} pages refused of ${w.state.calls.length}: the probe backs off where the limit is real`);
  assert.deepEqual(ix.balancesAt(COIN, 15_000), expected(COIN, 15_000));
});

test("a throttling provider (-32005, as rpc.ordofi.network answers every eth_getLogs) cools down: the update returns an error, never throws, and costs a few calls, not thousands", async () => {
  const w = chain(EVENTS, { throttle: (u) => u.includes("o.example") });
  const ix = new HolderIndex({ url: "https://o.example", factories: [c.factory], fromBlock: 1_000, confirmations: 0, http: w.http });
  const r = await ix.update({ deadline: Date.now() + 10_000 });
  assert.match(r.error, /busy|cooling/);
  assert.equal(r.group, 999, "the cursor did not move past what was not read");
  assert.ok(w.state.calls.length <= 3, `${w.state.calls.length} calls: one page, retried by rpc(), then the provider cools down`);
  const again = await ix.update({ deadline: Date.now() + 10_000 });
  assert.match(again.error, /cooling/);
  assert.ok(w.state.calls.length <= 3, "no call at all while it cools down");
});

test("compaction folds old logs into the base and every root inside the retention stays exact; the file survives a restart", async () => {
  const dir = mkdtempSync(join(tmpdir(), "holders-"));
  const w = chain(EVENTS);
  const file = join(dir, "holders-a.json");
  const ix = idx(w.http, { file, retentionBlocks: 5_000 });
  await ix.update({ deadline: Date.now() + 10_000 });
  ix.save();
  const coin = JSON.parse(readFileSync(file, "utf8")).coins[COIN.toLowerCase()];
  assert.equal(coin.baseBlock, 15_000, "folded up to head − retention");
  assert.equal(coin.logs.length, 0, "only logs newer than the base are kept");
  assert.throws(() => ix.balancesAt(COIN, 9_000), /older than the index keeps/);
  assert.deepEqual(ix.balancesAt(COIN, 16_000), expected(COIN, 16_000));
  // a restart: the saved cursor is used, and only the new blocks are read
  w.state.head = 20_600; w.state.calls.length = 0;
  const back = idx(w.http, { file, retentionBlocks: 5_000 });
  await back.update({ deadline: Date.now() + 10_000 });
  assert.equal(w.state.calls.length, 1, "one page");
  assert.equal(w.state.calls[0].a, 20_001);
});

test("the whole poll from two indexes, no injected recompute: an honest root is verified; a forged one is bad on both, and the guardian pauses then cancels", async () => {
  const snapshotAt = T0 + 9_600;                                  // block 9,600
  const excl = exclusionsFor(st, c, COIN, CURVE, []);
  const honest = weightedTree(expected(COIN, 9_600), excl, { sentinel: { snapshotAt } });
  const run = async (root) => {
    const w = chain(EVENTS);
    const views = (url, to, fn, args) => {
      if (fn === "drawCount") return to === c.raffle ? 1n : 0n;
      if (fn === "draw") return { stage: 1, coin: COIN, challengeEnds: BigInt(T0 + 20_000 + 600), challengeEndsBlock: 0n, round: 0n, pinnedAt: 0n, pinnedAtBlock: 0n, resolvedAt: 0n,
        resolvedAtBlock: 0n, root, totalTickets: honest.totalTickets, winningTicket: 0n, winner: ZERO, uri: "https://m.example/x.json", seedRound: 0n, announcedAt: 0n, announcedAtBlock: 0n, snapshotAt: BigInt(snapshotAt) };
      if (fn === "routerOf") return CURVE;
      throw new Error(`no view ${fn}`);
    };
    const f = fakeChain({ views, other: (u, m, p) => w.http(u, { body: JSON.stringify({ method: m, params: p }) }).then((r) => r.json()).then((j) => { if (j.error) throw new Error(j.error.message); return j.result; }) });
    const cfg = { rpcs: ["https://a.example", "https://b.example"], stack: { ...S.stack, fromBlock: 1_000 }, confirmations: 0 };
    const indexes = indexesFrom({ evm: cfg }, st, { http: f.http });
    const done = [];
    const guardian = { address: S.guardian, autoPause: true, autoCancel: true, pauseKeeper: async () => { done.push("pause"); return { done: true, tx: "0x1" }; },
      cancelDraw: async (v, id) => { done.push(`cancel ${id}`); return { done: true, tx: "0x2" }; } };
    const r = await checkEvm({ cfg, prev: {}, now: (T0 + 20_000) * 1000, http: f.http, readRecords: async () => EMPTY_RECORDS, timelock: "tl", guardian, indexes });
    return { r, done };
  };
  const good = await run(honest.root);
  assert.equal(good.r.next.draws[`${c.raffle}:0`].status, "verified", JSON.stringify(good.r.next.draws));
  assert.deepEqual(good.done, []);
  assert.equal(good.r.next.indexes.length, 2);
  const bad = await run(`0x${"cd".repeat(32)}`);
  assert.equal(bad.r.next.draws[`${c.raffle}:0`].status, "bad");
  assert.deepEqual(bad.done, ["pause", "cancel 0"]);
});

// ---- R4 check (docs/audit/2026-09-26/review4/fixes-golive-lows.md §6, §7) ----------------------------------------------------------

/** The whole poll above, with the draw's root and tickets given, and a state provider that may hang (its eth_call never answers
 *  until the read's own timeout aborts it). `order` records the guardian's actions and the reconciliation's first reads. */
async function pollWith({ root, totalTickets, hangState = false, draws = {} }) {
  const snapshotAt = T0 + 9_600;
  const w = chain(EVENTS);
  const order = [];
  const views = (url, to, fn) => {
    if (fn === "drawCount") return to === c.raffle ? 1n : 0n;
    if (fn === "draw") return { stage: 1, coin: COIN, challengeEnds: BigInt(T0 + 20_000 + 600), challengeEndsBlock: 0n, round: 0n, pinnedAt: 0n, pinnedAtBlock: 0n, resolvedAt: 0n,
      resolvedAtBlock: 0n, root, totalTickets, winningTicket: 0n, winner: ZERO, uri: "https://m.example/x.json", seedRound: 0n, announcedAt: 0n, announcedAtBlock: 0n, snapshotAt: BigInt(snapshotAt) };
    if (fn === "routerOf") return CURVE;
    if (fn === "totalSupply" || fn === "balanceOf") { order.push(`state ${fn}`); if (hangState) return new Promise(() => {}); return fn === "totalSupply" ? 1000n * E18 : 0n; }
    throw new Error(`no view ${fn}`);
  };
  const f = fakeChain({ views, other: (u, m, p) => w.http(u, { body: JSON.stringify({ method: m, params: p }) }).then((r) => r.json()).then((j) => { if (j.error) throw new Error(j.error.message); return j.result; }) });
  // a transport that honours the read's timeout (AbortSignal), as fetch does
  const http = (url, init) => Promise.race([f.http(url, init), new Promise((_, rej) => init.signal?.addEventListener("abort", () => rej(new Error(`timed out (${init.signal.reason?.name ?? "abort"})`))))]);
  const cfg = { rpcs: ["https://a.example", "https://b.example"], stack: { ...S.stack, fromBlock: 1_000 }, confirmations: 0, draws };
  const indexes = indexesFrom({ evm: cfg }, st, { http });
  const guardian = { address: S.guardian, autoPause: true, autoCancel: true, pauseKeeper: async () => { order.push("pause"); return { done: true, tx: "0x1" }; },
    cancelDraw: async (v, id) => { order.push(`cancel ${id}`); return { done: true, tx: "0x2" }; } };
  const t0 = Date.now();
  const keep = setInterval(() => {}, 1_000);                       // AbortSignal.timeout's timer does not hold the event loop; a socket would
  try {
    const r = await checkEvm({ cfg, prev: {}, now: (T0 + 20_000) * 1000, http, readRecords: async () => EMPTY_RECORDS, timelock: "tl", guardian, indexes });
    return { r, order, ms: Date.now() - t0, snapshotAt };
  } finally { clearInterval(keep); }
}

test("audit4 A4-I1 (documented: the monitor is the guard): the PoC's root, the sentinel leaf(0, 7, at) beside ONE junk sibling, opens on chain; the indexes reproduce the holders' root instead, so the draw is BAD and the guardian pauses the keeper, then cancels it before the pin", async () => {
  const at = T0 + 9_600;
  const { keccak256, toHex } = await import("viem");
  const junk = keccak256(toHex("junk: no holder can prove this"));
  const { merkleRoot, ticketLeaf } = await import("../src/draws.mjs");
  const root = merkleRoot([junk, ticketLeaf(ZERO, 7n, BigInt(at))]);
  const { r, order } = await pollWith({ root, totalTickets: 7n, draws: { reconcile: false } });
  assert.equal(r.next.draws[`${c.raffle}:0`].status, "bad", JSON.stringify(r.next.draws));
  assert.deepEqual(order.filter((x) => x === "pause" || x.startsWith("cancel")), ["pause", "cancel 0"], "paused, then cancelled: the carry is never consumed by a draw nobody can win");
  assert.ok(r.alerts.some((a) => a.key.startsWith("draw-bad:") && a.sev === "SEV1"));
});

test("audit4 L-S4 (fixed): a state provider that hangs on reconciliation's reads no longer delays the guardian: the bad draw's pause and cancel come first, and reconciliation stops at its own budget", async () => {
  const { r, order, ms } = await pollWith({ root: `0x${"cd".repeat(32)}`, totalTickets: 5n, hangState: true, draws: { reconcile_budget_s: 1 } });
  assert.equal(r.next.draws[`${c.raffle}:0`].status, "bad");
  const pause = order.indexOf("pause"), firstState = order.findIndex((x) => x.startsWith("state "));
  assert.ok(pause >= 0 && firstState > pause, `the guardian acted before any reconciliation read: ${order.join(", ")}`);
  assert.ok(ms < 8_000, `the poll took ${ms} ms with a provider that never answers its state reads (was: up to 17 reads × 3 tries × 30 s a coin)`);
  for (const ix of r.next.indexes) assert.equal(ix.mismatched.length, 0, "a read that ran out of time is not a mismatch");
});
