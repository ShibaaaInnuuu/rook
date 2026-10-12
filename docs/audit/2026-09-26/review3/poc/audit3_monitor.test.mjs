// Audit round 3 (R3 check) PoCs against the round-3 monitor. Each test asserts CURRENT behaviour: a pass means the gap is there.
// No network.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { encodeAbiParameters, encodeEventTopics } from "viem";
import { MODULE, retryPolicy } from "../src/abi.mjs";
import { judgeStackLogs } from "../src/evm.mjs";
import { HolderIndex, Provider, TRANSFER, pagedLogs } from "../src/holders.mjs";
import { stackOf } from "../src/stack.mjs";
import { fakeChain, hex } from "./fakechain.mjs";

retryPolicy.baseMs = 0;
const S = JSON.parse(readFileSync(new URL("../fixtures/stack-logs.json", import.meta.url), "utf8"));
const st = stackOf({ stack: S.stack }), c = st.collections[0];
const E18 = 10n ** 18n, NOW = 1_790_000_000_000;
const pad = (a) => `0x${a.slice(2).padStart(64, "0")}`;
const A = (n) => `0x${n.toString(16).padStart(40, "0")}`;

// ---- 1. the provider's span ceiling only ever goes down -------------------------------------------------------------------------

test("audit3 M-A: ONE transient non-range error (HTTP 403 / 400 / a proxy's HTML page) ratchets a provider's span ceiling down for good: after it the index reads one block per call, forever (process lifetime)", async () => {
  let bad = 0;
  const f = fakeChain({ other: (url, method) => {
    if (method !== "eth_getLogs") throw new Error(method);
    if (bad > 0) { bad--; throw new Error("header not found"); }      // -32000, not retryable(): treated as a range refusal
    return [];
  } });
  const p = new Provider({ url: "https://rpc.example", span: 10_000 });
  // healthy
  let r = await pagedLogs(p, { address: A(1) }, 1, 100_000, { http: f.http });
  assert.equal(r.upTo, 100_000); assert.equal(p.span, 10_000);
  // a burst of 13 errors (one per call) of a kind that has nothing to do with the range
  bad = 13;
  r = await pagedLogs(p, { address: A(1) }, 100_001, 200_000, { http: f.http });
  assert.equal(r.upTo, 200_000, "the read completes");
  assert.equal(p.ceiling, 1, "…but the ceiling was set to (failed span − 1) at each halving, down to 1");
  // the provider is healthy again, and stays at one block a call
  const before = f.calls.length;
  r = await pagedLogs(p, { address: A(1) }, 200_001, 200_600, { http: f.http });
  assert.equal(f.calls.length - before, 600, "600 new blocks (one minute of 4663) now cost 600 eth_getLogs, not 1");
  assert.equal(p.span, 1);
});

// ---- 2. one dense block stalls the whole group -----------------------------------------------------------------------------------

test("audit3 M-B: a block whose Transfer logs exceed a provider's per-query log cap (Robinhood: 10,000) stalls that provider's GROUP cursor: every other coin's index stops too, for good", async () => {
  const LIMIT = 50;                                              // stands in for the provider's 10,000-log cap
  const coinA = A(0xa11), coinB = A(0xb22), H = A(0x1001);
  const from = 1_000;
  const ev = [];
  ev.push({ address: coinB, blockNumber: from + 5, topics: [TRANSFER, pad(A(0)), pad(H)], data: pad(hex(5n * E18)) });
  for (let i = 0; i < LIMIT + 1; i++) ev.push({ address: coinA, blockNumber: from + 100, topics: [TRANSFER, pad(A(0x77)), pad(A(0x77))], data: pad(hex(0n)) });
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
  const ix = new HolderIndex({ url: "https://rpc.example", span: 1_000, factories: [], fromBlock: from, confirmations: 20, http: f.http });
  ix.addCoin(coinA); ix.addCoin(coinB);
  for (let i = 0; i < 5; i++) await ix.update({ deadline: Date.now() + 5_000 });
  assert.equal(ix.s.group.cursor, from + 99, "the group cursor stops just before the dense block");
  assert.equal(ix.s.coins[coinB].cursor, from + 99, "coin B, which has nothing to do with it, stops there too");
  // an honest draw of coin B with a snapshot after the dense block can never be recomputed by this provider
  await assert.rejects(ix.rootAt({ coin: coinB, snapshotAt: BigInt(1_700_000_000 + from + 3_000), fromBlock: from, head: head - 20, exclusions: [] }), /behind the snapshot block/);
});

// ---- 3. the floor-walk rule is evaded by a no-move post between edge posts ---------------------------------------------------

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

test("audit3 L-C: a stolen key inserts a no-move post one second after every second edge post: the edge-run rule never fires and pause_on_floor_walk never triggers; only the day-move rule (SEV1 at 18 %) catches the full 20 %/day walk", () => {
  const H = 3_600_000;
  const posts = [];
  let f = 65n * E18 / 100n, t = 0;
  posts.push([t, f]);
  const floor = f * 80n / 100n;                                  // the module's day band: −20 % of the day's anchor
  for (let k = 0; t < 30 * H; k++) {
    const next = f * 95n / 100n < floor ? floor : f * 95n / 100n;
    t += H; f = next; posts.push([t, f]);                        // edge (5 % in an hour) until the band's bottom, then flat
    if (k % 2 === 1) { t += 1_000; posts.push([t, f]); }        // same floor, 1 s later: resets the streak and the module's clock
  }
  const { alerts, triggers } = walk(posts);
  assert.equal(alerts.filter((a) => a.key.startsWith("floor-walk:")).length, 0, "no edge-run page");
  assert.deepEqual(triggers, [], "and so no automatic pause even with pause_on_floor_walk = true");
  const day = alerts.filter((a) => a.key.startsWith("floor-day-move:"));
  assert.ok(day.some((a) => a.sev === "SEV1"), "the day-move rule still pages SEV1 (no trigger) once 23 h of history exist");
});

test("audit3 L-C (2): a walk just under 10 % a day (the module allows 20 %) pages nothing at all, day after day", () => {
  const H = 3_600_000;
  const posts = [];
  let f = 65n * E18 / 100n;
  for (let h = 0; h <= 24 * 7; h++) { if (h) f = f * 99_561n / 100_000n; posts.push([h * H, f]); }   // −0.439 %/h ≈ −10 %/day... just under
  const { alerts } = walk(posts, false);
  const start = 65n * E18 / 100n;
  assert.ok(f * 100n / start <= 50n, `the floor fell to ${Number(f * 1000n / start) / 10} % of its start in a week`);
  assert.equal(alerts.length, 0, alerts.map((a) => a.key).join(","));
});

// ---- 4. one silently dropped log is permanent, and "disagree" pauses the keeper at every later draw's deadline -----------------

test("audit3 L-F: a provider that once returns a page short by one log (a lagging load-balanced backend) keeps a wrong index for good: every later draw of that coin is 'disagree', which the deadline turns into a keeper pause", async () => {
  const { verdictN } = await import("../src/draws.mjs");
  const coin = A(0xc01), H1 = A(0x1001), H2 = A(0x1002), from = 1_000, head = from + 2_000;
  const ev = [
    { address: coin, blockNumber: from + 10, topics: [TRANSFER, pad(A(0)), pad(H1)], data: pad(hex(3n * E18)) },
    { address: coin, blockNumber: from + 20, topics: [TRANSFER, pad(A(0)), pad(H2)], data: pad(hex(2n * E18)) },   // H2 never spends
  ].map((l, i) => ({ ...l, blockNumber: hex(l.blockNumber), logIndex: hex(i), transactionHash: pad(hex(i + 1)), removed: false }));
  const mk = (drop) => {
    let dropped = false;
    return fakeChain({ other: (url, method, params) => {
      if (method === "eth_blockNumber") return hex(head);
      if (method === "eth_getBlockByNumber") return { timestamp: hex(1_700_000_000 + Number(BigInt(params[0]))) };
      const q = params[0], a = Number(BigInt(q.fromBlock)), b = Number(BigInt(q.toBlock)), addrs = [].concat(q.address).map((x) => x.toLowerCase());
      let hit = ev.filter((l) => addrs.includes(l.address.toLowerCase()) && Number(BigInt(l.blockNumber)) >= a && Number(BigInt(l.blockNumber)) <= b);
      if (drop && !dropped && hit.length > 1) { dropped = true; hit = hit.slice(0, 1); }             // once
      return hit;
    } }).http;
  };
  const good = new HolderIndex({ url: "https://good.example", span: 10_000, fromBlock: from, http: mk(false) });
  const lossy = new HolderIndex({ url: "https://lossy.example", span: 10_000, fromBlock: from, http: mk(true) });
  for (const ix of [good, lossy]) { ix.addCoin(coin); await ix.update({ deadline: Date.now() + 5_000 }); await ix.update({ deadline: Date.now() + 5_000 }); }
  const input = { coin, snapshotAt: BigInt(1_700_000_000 + from + 1_500), fromBlock: from, head: head - 20, exclusions: [] };
  const rg = await good.rootAt(input), rl = await lossy.rootAt(input);
  assert.notEqual(rl.root, rg.root, "the lossy index is wrong, silently (no balance went negative, so compaction never resets it)");
  const v = verdictN({ root: rg.root, totalTickets: String(rg.totalTickets), snapshotAt: String(input.snapshotAt) }, [rg, rl], 2);
  assert.equal(v.v, "disagree", "an HONEST draw: one provider reproduces it, the other computes another root");
  // evm.mjs's deadline: `(d.matches ?? 0) >= 1 && d.status !== "disagree"` is the only no-pause path, so disagree → pauseKeeper()
  for (let i = 0; i < 3; i++) await lossy.update({ deadline: Date.now() + 5_000 });
  assert.notEqual((await lossy.rootAt(input)).root, rg.root, "never repaired: the cursor has moved past the dropped log");
});
