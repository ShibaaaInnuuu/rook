// Audit round 4 (R4 check). Part 1: R3's monitor PoCs (review3/poc/audit3_monitor.test.mjs) re-run against the round-4 code, with
// their assertions turned (a pass means the R3 attack now FAILS). Part 2: new PoCs; each asserts CURRENT behaviour (a pass means
// the gap is there). No network.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { encodeAbiParameters, encodeEventTopics } from "viem";
import { MODULE, retryPolicy } from "../src/abi.mjs";
import { verdictN } from "../src/draws.mjs";
import { deadlineCall, evidenceOf, judgeStackLogs } from "../src/evm.mjs";
import { HolderIndex, Provider, TRANSFER, pagedLogs } from "../src/holders.mjs";
import { stackOf } from "../src/stack.mjs";
import { fakeChain, hex } from "./fakechain.mjs";

retryPolicy.baseMs = 0;
const S = JSON.parse(readFileSync(new URL("../fixtures/stack-logs.json", import.meta.url), "utf8"));
const st = stackOf({ stack: S.stack }), c = st.collections[0];
const E18 = 10n ** 18n, NOW = 1_790_000_000_000;
const pad = (a) => `0x${a.slice(2).padStart(64, "0")}`;
const A = (n) => `0x${n.toString(16).padStart(40, "0")}`;

// ================================ Part 1: R3's PoCs, re-run ======================================================================

test("R4 re-run of audit3 M-A: 13 non-range errors no longer ratchet the ceiling; 600 new blocks cost 1 call", async () => {
  let bad = 0;
  const f = fakeChain({ other: (url, method) => {
    if (method !== "eth_getLogs") throw new Error(method);
    if (bad > 0) { bad--; throw new Error("header not found"); }
    return [];
  } });
  const p = new Provider({ url: "https://rpc.example", span: 10_000 });
  let r = await pagedLogs(p, { address: A(1) }, 1, 100_000, { http: f.http });
  assert.equal(r.upTo, 100_000);
  bad = 13;
  // R3's read now ends early on repeated non-range errors (3 tries a page), cursor before the failed page; the next reads go on
  let t = Date.now();
  for (let i = 0; i < 10 && r.upTo < 200_000; i++) { r = await pagedLogs(p, { address: A(1) }, r.upTo + 1, 200_000, { http: f.http, clock: () => t }); t += 3 * 60_000; }
  assert.equal(r.upTo, 200_000);
  assert.equal(p.ceiling, 10_000, "the ceiling is untouched by errors that are not about the size");
  const before = f.calls.length;
  r = await pagedLogs(p, { address: A(1) }, 200_001, 200_600, { http: f.http, clock: () => t });
  assert.equal(f.calls.length - before, 1, "600 new blocks cost one call");
});

/** R3's M-B world, unchanged: its fake answers eth_getBlockReceipts with an error (unknown method) and eth_getBlockByNumber with a
 *  header that has NO `transactions` field (as a lagging backend's `null`, or a provider that omits it, would look to the code). */
function r3DenseWorld({ value = 0n } = {}) {
  const LIMIT = 50;
  const coinA = A(0xa11), coinB = A(0xb22), H = A(0x1001), X = A(0x77), Y = A(0x78);
  const from = 1_000;
  const ev = [];
  ev.push({ address: coinB, blockNumber: from + 5, topics: [TRANSFER, pad(A(0)), pad(H)], data: pad(hex(5n * E18)) });
  ev.push({ address: coinA, blockNumber: from + 5, topics: [TRANSFER, pad(A(0)), pad(X)], data: pad(hex(100n * E18)) });
  // the dense block: LIMIT + 1 transfers of coin A from X to Y (`value` each)
  for (let i = 0; i < LIMIT + 1; i++) ev.push({ address: coinA, blockNumber: from + 100, topics: [TRANSFER, pad(X), pad(Y)], data: pad(hex(value)) });
  const logs = ev.map((l, i) => ({ ...l, blockNumber: hex(l.blockNumber), logIndex: hex(i), transactionHash: pad(hex(i + 1)), removed: false }));
  const head = from + 5_000;
  const f = fakeChain({ other: (url, method, params) => {
    if (method === "eth_blockNumber") return hex(head);
    if (method === "eth_getBlockByNumber") return { timestamp: hex(1_700_000_000 + Number(BigInt(params[0]))) };
    if (method !== "eth_getLogs") throw new Error(method);
    const q = params[0], a = Number(BigInt(q.fromBlock)), b = Number(BigInt(q.toBlock)), addrs = [].concat(q.address).map((x) => x.toLowerCase());
    const hit = logs.filter((l) => addrs.includes(l.address.toLowerCase()) && Number(BigInt(l.blockNumber)) >= a && Number(BigInt(l.blockNumber)) <= b);
    if (hit.length > LIMIT) throw new Error(`query exceeds limit of ${LIMIT}`);
    return hit;
  } });
  return { f, coinA, coinB, X, Y, from, head, LIMIT };
}

test("R4 re-run of audit3 M-B: the dense block no longer stalls the group or the unrelated coin", async () => {
  const { f, coinA, coinB, from, head } = r3DenseWorld();
  const ix = new HolderIndex({ url: "https://rpc.example", span: 1_000, factories: [], fromBlock: from, confirmations: 20, http: f.http });
  ix.addCoin(coinA); ix.addCoin(coinB);
  for (let i = 0; i < 5; i++) await ix.update({ deadline: Date.now() + 5_000 });
  assert.equal(ix.s.group.cursor, head - 20);
  assert.equal(ix.s.coins[coinB].cursor, head - 20);
  const r = await ix.rootAt({ coin: coinB, snapshotAt: BigInt(1_700_000_000 + from + 3_000), fromBlock: from, head: head - 20, exclusions: [] });
  assert.equal(r.totalTickets, 5n);
});

// ---- L-C, R3's two PoCs -----------------------------------------------------------------------------------------------------------

const floorLog = (i, target, accepted, blk) => ({ address: c.module, topics: encodeEventTopics({ abi: MODULE, eventName: "FloorPosted" }),
  data: encodeAbiParameters([{ type: "uint256" }, { type: "uint256" }, { type: "uint256" }, { type: "uint16" }, { type: "uint32" }], [target, accepted, 500n, 8000, 12]),
  blockNumber: hex(blk), transactionHash: `0x${(i + 1).toString(16).padStart(64, "0")}`, logIndex: "0x0", blockHash: `0x${(i + 7).toString(16).padStart(64, "0")}`, removed: false });
function walk(posts, pauseOnFloorWalk = true) {
  let prev = {};
  const alerts = [], triggers = [];
  posts.forEach(([t, f], i) => {
    const j = judgeStackLogs({ logs: [floorLog(i, f, f, 71_300_000 + i)], prev, ctx: { st, now: NOW + t, minConf: { [c.module]: 3000n }, times: {}, pauseOnFloorWalk } });
    alerts.push(...j.alerts.map((a) => ({ ...a, t }))); triggers.push(...j.triggers); prev = j.next;
  });
  return { alerts, triggers };
}
const H = 3_600_000;

test("R4 re-run of audit3 L-C: the interleaved no-move post no longer evades the edge run; the pause triggers", () => {
  const posts = [];
  let f = 65n * E18 / 100n, t = 0;
  posts.push([t, f]);
  const floor = f * 80n / 100n;
  for (let k = 0; t < 30 * H; k++) {
    const next = f * 95n / 100n < floor ? floor : f * 95n / 100n;
    t += H; f = next; posts.push([t, f]);
    if (k % 2 === 1) { t += 1_000; posts.push([t, f]); }
  }
  const { alerts, triggers } = walk(posts);
  assert.ok(alerts.some((a) => a.key.startsWith("floor-walk:")), "edge-run page");
  assert.ok(triggers.length > 0, "and the pause trigger");
});

test("R4 re-run of audit3 L-C (2): the −0.439 %/h walk now pages SEV1 within the week", () => {
  const posts = [];
  let f = 65n * E18 / 100n;
  for (let h = 0; h <= 24 * 7; h++) { if (h) f = f * 99_561n / 100_000n; posts.push([h * H, f]); }
  const { alerts } = walk(posts, false);
  assert.ok(alerts.some((a) => a.sev === "SEV1"));
});

// ---- L-F, R3's PoC, driven through evidenceOf (the repair) --------------------------------------------------------------------------

test("R4 re-run of audit3 L-F: the lossy index is repaired on the dissent and then reproduces the honest root", async () => {
  const coin = A(0xc01), H1 = A(0x1001), H2 = A(0x1002), from = 1_000, head = from + 2_000;
  const ev = [
    { address: coin, blockNumber: from + 10, topics: [TRANSFER, pad(A(0)), pad(H1)], data: pad(hex(3n * E18)) },
    { address: coin, blockNumber: from + 20, topics: [TRANSFER, pad(A(0)), pad(H2)], data: pad(hex(2n * E18)) },
  ].map((l, i) => ({ ...l, blockNumber: hex(l.blockNumber), logIndex: hex(i), transactionHash: pad(hex(i + 1)), removed: false }));
  const mk = (drop) => {
    let dropped = false;
    return fakeChain({ other: (url, method, params) => {
      if (method === "eth_blockNumber") return hex(head);
      if (method === "eth_getBlockByNumber") return { timestamp: hex(1_700_000_000 + Number(BigInt(params[0]))) };
      const q = params[0], a = Number(BigInt(q.fromBlock)), b = Number(BigInt(q.toBlock)), addrs = [].concat(q.address).map((x) => x.toLowerCase());
      let hit = ev.filter((l) => addrs.includes(l.address.toLowerCase()) && Number(BigInt(l.blockNumber)) >= a && Number(BigInt(l.blockNumber)) <= b);
      if (drop && !dropped && hit.length > 1) { dropped = true; hit = hit.slice(0, 1); }
      return hit;
    } }).http;
  };
  const good = new HolderIndex({ url: "https://good.example", span: 10_000, fromBlock: from, http: mk(false) });
  const lossy = new HolderIndex({ url: "https://lossy.example", span: 10_000, fromBlock: from, http: mk(true) });
  for (const ix of [good, lossy]) { ix.addCoin(coin); await ix.update({ deadline: Date.now() + 5_000 }); await ix.update({ deadline: Date.now() + 5_000 }); }
  const input = { coin, snapshotAt: BigInt(1_700_000_000 + from + 1_500), fromBlock: from, head: head - 20, exclusions: [] };
  const rg = await good.rootAt(input), rl = await lossy.rootAt(input);
  const onChain = { root: rg.root, totalTickets: String(rg.totalTickets), snapshotAt: String(input.snapshotAt) };
  const d = { coin, root: rg.root, totalTickets: String(rg.totalTickets) };
  const v = verdictN(onChain, [rg, rl], 2);
  assert.equal(v.v, "disagree");
  const e1 = evidenceOf("v:1", d, [rg, rl], [good.url, lossy.url], v, { indexes: [good, lossy] });
  assert.equal(e1.dissent["lossy.example"].repaired, false);
  for (let i = 0; i < 3; i++) await lossy.update({ deadline: Date.now() + 5_000 });
  const rl2 = await lossy.rootAt(input);
  assert.equal(rl2.root, rg.root, "repaired: the rebuilt index reproduces the honest root");
  const e2 = evidenceOf("v:1", { ...d, dissent: e1.dissent }, [rg, rl2], [good.url, lossy.url], verdictN(onChain, [rg, rl2], 2), { indexes: [good, lossy] });
  assert.deepEqual(e2.dissent, {}, "the dissent is dropped");
});

// ================================ Part 2: new PoCs (a pass = the gap is there) ==================================================

test("audit4 N-S1: denseBlock treats a block header with no `transactions` (a lagging backend's null block) as an EMPTY block: the dense coin's logs in that block are skipped silently, its cursor passes the block and nothing is marked stuck", async () => {
  // R3's own M-B world, with value-bearing transfers in the dense block (51 × 1 coin from X to Y)
  const { f, coinA, X, Y, from, head } = r3DenseWorld({ value: 1n * E18 });
  const ix = new HolderIndex({ url: "https://rpc.example", span: 1_000, factories: [], fromBlock: from, confirmations: 20, http: f.http });
  ix.addCoin(coinA);
  for (let i = 0; i < 5; i++) await ix.update({ deadline: Date.now() + 5_000 });
  assert.equal(ix.s.coins[coinA].cursor, head - 20, "coin A's cursor passed its dense block");
  assert.deepEqual(ix.summary().stuck, [], "and nothing says it was not read");
  const bal = ix.balancesAt(coinA, from + 200);
  assert.equal(bal.get(X), 100n * E18, "X still holds 100 (on chain: 49)");
  assert.equal(bal.get(Y), undefined, "Y holds nothing (on chain: 51)");
  assert.ok(f.calls.some((x) => x.method === "eth_getBlockByNumber" && x.params[1] === false), "the header was asked for, and taken as an empty block");
});

test("audit4 N-S1 (b): the same with a provider that answers eth_getBlockByNumber with null for the (lagging) block", async () => {
  const w = r3DenseWorld({ value: 1n * E18 });
  const http = async (url, init) => {
    const { method, params } = JSON.parse(init.body);
    if (method === "eth_getBlockByNumber" && Number(BigInt(params[0])) === w.from + 100) return { ok: true, json: async () => ({ jsonrpc: "2.0", id: 1, result: null }) };
    return w.f.http(url, init);
  };
  const ix = new HolderIndex({ url: "https://rpc.example", span: 1_000, factories: [], fromBlock: w.from, confirmations: 20, http });
  ix.addCoin(w.coinA);
  for (let i = 0; i < 5; i++) await ix.update({ deadline: Date.now() + 5_000 });
  assert.equal(ix.s.coins[w.coinA].cursor, w.head - 20);
  assert.equal(ix.balancesAt(w.coinA, w.from + 200).get(w.Y), undefined, "51 coins of transfers skipped");
});

test("audit4 N-S2: a dense block whose receipts call fails TRANSIENTLY (a gateway 504 / a timeout on a multi-MB answer) stalls the whole GROUP again, and cools the provider down (M-B's impact, by the retryable path)", async () => {
  const LIMIT = 50, coinA = A(0xa11), coinB = A(0xb22), from = 1_000, head = from + 5_000;
  const ev = [{ address: coinB, blockNumber: from + 5, topics: [TRANSFER, pad(A(0)), pad(A(0x1001))], data: pad(hex(5n * E18)) }];
  for (let i = 0; i < LIMIT + 1; i++) ev.push({ address: coinA, blockNumber: from + 100, topics: [TRANSFER, pad(A(0x77)), pad(A(0x77))], data: pad(hex(0n)) });
  const logs = ev.map((l, i) => ({ ...l, blockNumber: hex(l.blockNumber), logIndex: hex(i), transactionHash: pad(hex(i + 1)), removed: false }));
  const inner = fakeChain({ other: (url, method, params) => {
    if (method === "eth_blockNumber") return hex(head);
    if (method === "eth_getBlockByNumber") return { timestamp: hex(1_700_000_000 + Number(BigInt(params[0]))), transactions: [] };
    if (method !== "eth_getLogs") throw new Error(method);
    const q = params[0], a = Number(BigInt(q.fromBlock)), b = Number(BigInt(q.toBlock)), addrs = [].concat(q.address).map((x) => x.toLowerCase());
    const hit = logs.filter((l) => addrs.includes(l.address.toLowerCase()) && Number(BigInt(l.blockNumber)) >= a && Number(BigInt(l.blockNumber)) <= b);
    if (hit.length > LIMIT) throw new Error(`query exceeds limit of ${LIMIT}`);
    return hit;
  } });
  // the provider's gateway answers the dense block's receipts (several MB) with a 504, every time
  const http = async (url, init) => {
    const { method } = JSON.parse(init.body);
    if (method === "eth_getBlockReceipts") return { ok: false, status: 504, text: async () => "Gateway Timeout" };
    return inner.http(url, init);
  };
  let t = Date.now();
  const ix = new HolderIndex({ url: "https://rpc.example", span: 1_000, factories: [], fromBlock: from, confirmations: 20, http, clock: () => t });
  ix.addCoin(coinA); ix.addCoin(coinB);
  for (let i = 0; i < 20; i++) { await ix.update({ deadline: t + 5_000 }); t += 31 * 60_000; }   // 20 polls, 10 hours, past every cool-down
  assert.equal(ix.s.group.cursor, from + 99, "the group stops before the dense block, poll after poll");
  assert.equal(ix.s.coins[coinB].cursor, from + 99, "the unrelated coin B too");
  await assert.rejects(ix.rootAt({ coin: coinB, snapshotAt: BigInt(1_700_000_000 + from + 3_000), fromBlock: from, head: head - 20, exclusions: [] }), /behind the snapshot block/);
  assert.match(ix.summary().lastError ?? "", /504/);
});

test("audit4 N-S3 (Info): a walk just under every SEV1 rule loses ~49 % in a week (SEV2 only), and just under every SEV2 rule ~31 % with no page at all", () => {
  const run = (keepPerHour) => {
    const posts = [];
    let f = 65n * E18 / 100n;
    const start = f;
    for (let h = 0; h <= 24 * 7; h++) { if (h) f = f * keepPerHour / 1_000_000n; posts.push([h * H, f]); }
    const { alerts, triggers } = walk(posts, true);
    return { left: Number(f * 1000n / start) / 10, alerts, triggers };
  };
  const a = run(996_010n);                                   // −0.399 %/h: 72 h ≈ −24.99 %, a day ≈ −9.2 %, 6 h ≈ −2.4 %
  assert.ok(a.left < 52, `left ${a.left} %`);
  assert.equal(a.alerts.filter((x) => x.sev === "SEV1").length, 0, "no SEV1");
  assert.deepEqual(a.triggers, [], "no pause trigger with pause_on_floor_walk on");
  assert.ok(a.alerts.some((x) => x.sev === "SEV2"), "SEV2 (the three-day rule) pages");
  const b = run(997_760n);                                   // −0.224 %/h: 72 h ≈ −14.9 %
  assert.ok(b.left < 69, `left ${b.left} %`);
  assert.equal(b.alerts.length, 0, b.alerts.map((x) => x.key).join(","));
});

test("audit4 (Info) the deadline pauses on a STALE dissent: a provider that dissented once is being rebuilt (degraded), the other is degraded this poll: pause, although the rebuilt index has not said anything yet", () => {
  const deg = { r: "error", cls: "degraded", why: "x: behind the snapshot block" };
  const d = { matches: 0, status: "pending", per: { a: deg, b: deg }, dissent: { b: { root: "0x1", tickets: "3", repaired: false } } };
  assert.equal(deadlineCall(d).act, "pause");
});

test("audit4 N-S4: reconcile() ignores the poll's deadline inside a coin: 1 totalSupply + 16 balanceOf reads run one after the other, each with the full 30 s timeout (× 3 tries on a timeout), and checkEvm's deadline pause runs only after it", async () => {
  const coin = A(0xc01), from = 1_000, head = from + 2_000;
  const holders = Array.from({ length: 20 }, (_, i) => A(0x2000 + i));
  const ev = holders.map((h, i) => ({ address: coin, blockNumber: hex(from + 10 + i), topics: [TRANSFER, pad(A(0)), pad(h)], data: pad(hex(E18)), logIndex: hex(i), transactionHash: pad(hex(i + 1)), removed: false }));
  const DELAY = 150;
  let calls = 0;
  const f = fakeChain({
    views: async (url, to, fn) => { calls++; await new Promise((r) => setTimeout(r, DELAY)); if (fn === "totalSupply") return 20n * E18; if (fn === "balanceOf") return E18; throw new Error(fn); },
    other: (url, method, params) => {
      if (method === "eth_blockNumber") return hex(head);
      if (method === "eth_getLogs") { const q = params[0], a = Number(BigInt(q.fromBlock)), b = Number(BigInt(q.toBlock)); return ev.filter((l) => Number(BigInt(l.blockNumber)) >= a && Number(BigInt(l.blockNumber)) <= b); }
      throw new Error(method);
    } });
  const ix = new HolderIndex({ url: "https://rpc.example", span: 10_000, fromBlock: from, http: f.http });
  ix.addCoin(coin); ix.addCoin(A(0xc02));
  await ix.update({ deadline: Date.now() + 5_000 });
  const t0 = Date.now();
  await ix.reconcile({ head, deadline: t0 + DELAY });            // a budget of one call
  const took = Date.now() - t0;
  assert.ok(calls >= 17, `${calls} sequential state reads`);
  assert.ok(took >= 16 * DELAY, `took ${took} ms against a ${DELAY} ms budget`);
});
