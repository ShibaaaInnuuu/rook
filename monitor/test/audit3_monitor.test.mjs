// The targeted check of round 3 (docs/audit/2026-09-26/review3/R3-check.md), turned into regressions by round 4
// (review3/fixes-R4-services.md). The audit3 PoCs asserted the gaps (a pass meant the gap was there); each test here asserts the
// attack now FAILS, followed by the new cases of the fix. No network.
import assert from "node:assert/strict";
import { mkdtempSync, readFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { encodeAbiParameters, encodeEventTopics } from "viem";
import { MODULE, rangeError, retryPolicy } from "../src/abi.mjs";
import { verdictN } from "../src/draws.mjs";
import { checkEvm, deadlineCall, errorClass, evidenceOf, judgeStackLogs } from "../src/evm.mjs";
import { BROKEN_RETRY_MS, HolderIndex, Provider, RECONCILE, SPAN, TRANSFER, pagedLogs } from "../src/holders.mjs";
import { EMPTY_RECORDS } from "../src/records.mjs";
import { stackOf } from "../src/stack.mjs";
import { fakeChain, hex } from "./fakechain.mjs";

retryPolicy.baseMs = 0;
const S = JSON.parse(readFileSync(new URL("../fixtures/stack-logs.json", import.meta.url), "utf8"));
const st = stackOf({ stack: S.stack }), c = st.collections[0];
const E18 = 10n ** 18n, NOW = 1_790_000_000_000, ZERO = "0x0000000000000000000000000000000000000000";
const pad = (a) => `0x${a.slice(2).padStart(64, "0")}`;
const A = (n) => `0x${n.toString(16).padStart(40, "0")}`;

// ---- M-A: a provider's page span recovers ------------------------------------------------------------------------------------------

test("audit3 M-A (fixed): 13 transient non-range errors (header not found) leave the span and its ceiling alone: 600 new blocks cost one call, not 600", async () => {
  let bad = 0;
  const f = fakeChain({ other: (url, method) => {
    if (method !== "eth_getLogs") throw new Error(method);
    if (bad > 0) { bad--; throw new Error("header not found"); }
    return [];
  } });
  const p = new Provider({ url: "https://rpc.example", span: 10_000 });
  let r = await pagedLogs(p, { address: A(1) }, 1, 100_000, { http: f.http });
  assert.equal(r.upTo, 100_000); assert.equal(p.span, 10_000);
  bad = 13;
  for (let i = 0; i < 10 && r.upTo < 200_000; i++) r = await pagedLogs(p, { address: A(1) }, r.upTo + 1, 200_000, { http: f.http, clock: () => NOW + i * 10 * 60_000 });
  assert.equal(r.upTo, 200_000, "the read completes, over a few polls (each page retried at most 3 times, then the provider cools down)");
  assert.equal(p.ceiling, 10_000, "the ceiling is untouched: header not found says nothing about the range");
  const before = f.calls.length;
  r = await pagedLogs(p, { address: A(1) }, 200_001, 200_600, { http: f.http, clock: () => NOW + 3_600_000 });
  assert.equal(f.calls.length - before, 1, "600 new blocks are one eth_getLogs");
  assert.equal(p.span, 10_000);
});

test("M-A: only an answer about the query's size narrows the span; the ceiling grows back after successes, and a fresh window resets it", async () => {
  assert.ok(rangeError(new Error("x: -32000 query exceeds limit of 10000")));
  assert.ok(rangeError(new Error("x: ranges over 10000 blocks are not supported on free plan")));
  assert.ok(rangeError(new Error("x answered HTTP 400: block range too large, max 500 blocks")));
  for (const e of ["header not found", "x answered HTTP 403: <html>Forbidden</html>", "x: -32005 the network is busy", "x: rate limit exceeded", "x answered HTTP 429"]) assert.ok(!rangeError(new Error(e)), e);
  let dense = true, t = NOW;
  const f = fakeChain({ other: (url, method, params) => {
    const q = params[0], a = Number(BigInt(q.fromBlock)), b = Number(BigInt(q.toBlock));
    if (dense && b - a + 1 > 1_000) throw new Error("query exceeds limit of 10000");
    return [];
  } });
  const p = new Provider({ url: "https://rpc.example", span: 10_000 });
  await pagedLogs(p, { address: A(1) }, 1, 50_000, { http: f.http, clock: () => t });
  assert.ok(p.ceiling <= 1_000, `a dense stretch narrows it (ceiling ${p.ceiling})`);
  dense = false;
  const n0 = f.calls.length;
  await pagedLogs(p, { address: A(1) }, 50_001, 1_000_000, { http: f.http, clock: () => t });
  assert.equal(p.ceiling, 10_000, "the dense stretch is over: after successes at its ceiling the provider is back at its full span");
  assert.ok(f.calls.length - n0 < 400, `${f.calls.length - n0} calls for 950,000 blocks (not 950 at 1,000 a page, nor 950,000 at one)`);
  dense = true;
  await pagedLogs(p, { address: A(1) }, 1_000_001, 1_050_000, { http: f.http, clock: () => t });
  assert.ok(p.ceiling < 10_000);
  dense = false; t += SPAN.ceilingTtlMs;
  await pagedLogs(p, { address: A(1) }, 1_050_001, 1_050_001, { http: f.http, clock: () => t });
  assert.equal(p.ceiling, 10_000, "a ceiling ten minutes old is forgotten (a fresh window)");
});

test("M-A: a provider that answers HTTP 403 for good costs at most 3 calls a page, then cools down; the cursor never passes what it did not read", async () => {
  const f = fakeChain({ other: () => { throw new Error("answered HTTP 403: <html>Access denied</html>"); } });
  const p = new Provider({ url: "https://rpc.example", span: 10_000 });
  const r = await pagedLogs(p, { address: A(1) }, 1, 100_000, { http: f.http, clock: () => NOW });
  assert.equal(r.upTo, 0); assert.match(r.error, /403/);
  assert.equal(f.calls.length, SPAN.pageTries, "a page is tried at most 3 times");
  assert.equal(p.ceiling, 10_000, "and the span is untouched");
  for (let i = 1; i < SPAN.failsToCool; i++) await pagedLogs(p, { address: A(1) }, 1, 100_000, { http: f.http, clock: () => NOW });
  assert.ok(p.cooling(NOW), "pages failing that way in a row (three polls) cool the provider down");
  const n = f.calls.length;
  await pagedLogs(p, { address: A(1) }, 1, 100_000, { http: f.http, clock: () => NOW });
  assert.equal(f.calls.length, n, "no call while it cools down");
});

// ---- M-B: a dense block never stalls the index -------------------------------------------------------------------------------------

/** The PoC's chain: coin B's mint at from + 5, then LIMIT + 1 zero-value self-transfers of coin A in block from + 100. */
function denseChain({ receipts = true, txReceipts = true, limit = 50, addressLimit = Infinity } = {}) {
  const coinA = A(0xa11), coinB = A(0xb22), H = A(0x1001), from = 1_000;
  const ev = [{ address: coinB, blockNumber: from + 5, topics: [TRANSFER, pad(A(0)), pad(H)], data: pad(hex(5n * E18)) },
    { address: coinA, blockNumber: from + 6, topics: [TRANSFER, pad(A(0)), pad(A(0x77))], data: pad(hex(2n * E18)) }];
  for (let i = 0; i < limit + 1; i++) ev.push({ address: coinA, blockNumber: from + 100, topics: [TRANSFER, pad(A(0x77)), pad(A(0x77))], data: pad(hex(0n)) });
  ev.push({ address: coinB, blockNumber: from + 200, topics: [TRANSFER, pad(H), pad(A(0x1002))], data: pad(hex(1n * E18)) });
  const logs = ev.map((l, i) => ({ ...l, blockNumber: hex(l.blockNumber), logIndex: hex(i), transactionHash: pad(hex(l.blockNumber * 10 + (i % 3))), removed: false }));
  const head = from + 5_000;
  const f = fakeChain({ other: (url, method, params) => {
    if (method === "eth_blockNumber") return hex(head);
    if (method === "eth_getBlockByNumber") {
      const n = Number(BigInt(params[0]));
      return { timestamp: hex(1_700_000_000 + n), transactions: [...new Set(logs.filter((l) => Number(BigInt(l.blockNumber)) === n).map((l) => l.transactionHash))] };
    }
    if (method === "eth_getBlockReceipts") {
      if (!receipts) throw new Error("the method eth_getBlockReceipts does not exist/is not available");
      const n = Number(BigInt(params[0]));
      return [{ blockNumber: hex(n), blockHash: pad(hex(n)), logs: logs.filter((l) => Number(BigInt(l.blockNumber)) === n) }];
    }
    if (method === "eth_getTransactionReceipt") {
      if (!txReceipts) throw new Error("response size exceeded");
      const mine = logs.filter((l) => l.transactionHash === params[0]);
      return { blockNumber: mine[0]?.blockNumber ?? null, logs: mine };
    }
    if (method !== "eth_getLogs") throw new Error(method);
    const q = params[0], a = Number(BigInt(q.fromBlock)), b = Number(BigInt(q.toBlock)), addrs = [].concat(q.address).map((x) => x.toLowerCase());
    if (addrs.length > addressLimit) throw new Error(`too many addresses in filter (max ${addressLimit})`);
    const hit = logs.filter((l) => addrs.includes(l.address.toLowerCase()) && Number(BigInt(l.blockNumber)) >= a && Number(BigInt(l.blockNumber)) <= b);
    if (hit.length > limit) throw new Error(`query exceeds limit of ${limit}`);
    return hit;
  } });
  return { f, coinA, coinB, H, from, head };
}

test("audit3 M-B (fixed): a block with more Transfer logs than the provider's per-query cap no longer stalls the group: it is read from the block's receipts, and every coin goes on", async () => {
  const { f, coinA, coinB, H, from, head } = denseChain();
  const ix = new HolderIndex({ url: "https://rpc.example", span: 1_000, factories: [], fromBlock: from, confirmations: 20, http: f.http });
  ix.addCoin(coinA); ix.addCoin(coinB);
  for (let i = 0; i < 5; i++) await ix.update({ deadline: Date.now() + 5_000 });
  assert.equal(ix.s.group.cursor, head - 20, "the group reaches the safe head");
  assert.equal(ix.s.coins[coinB].cursor, head - 20, "the unrelated coin B too");
  assert.equal(ix.s.coins[coinA].cursor, head - 20, "and the dense coin A itself (its block read by receipts)");
  assert.deepEqual(ix.summary().stuck, []);
  const r = await ix.rootAt({ coin: coinB, snapshotAt: BigInt(1_700_000_000 + from + 3_000), fromBlock: from, head: head - 20, exclusions: [] });
  assert.equal(r.totalTickets, 5n, "coin B's draw after the dense block is recomputed");
  assert.deepEqual(ix.balancesAt(coinA, from + 101), new Map([[A(0x77), 2n * E18]]), "coin A's balances are exact through its dense block");
  assert.ok(f.calls.some((x) => x.method === "eth_getBlockReceipts"));
  assert.ok(H);
});

test("M-B: receipts refused and every narrower way refused: ONLY the dense coin is left behind at that block (paged), the others advance, and the saved file resumes there", async () => {
  const dir = mkdtempSync(join(tmpdir(), "audit3-"));
  const { f, coinA, coinB, from, head } = denseChain({ receipts: false, txReceipts: false });
  const file = join(dir, "holders-rpc.json");
  const mk = () => new HolderIndex({ url: "https://rpc.example", span: 1_000, factories: [], fromBlock: from, confirmations: 20, http: f.http, file });
  const ix = mk();
  ix.addCoin(coinA); ix.addCoin(coinB);
  for (let i = 0; i < 4; i++) await ix.update({ deadline: Date.now() + 5_000 });
  ix.save();
  assert.equal(ix.s.group.cursor, head - 20, "the group is not held");
  assert.equal(ix.s.coins[coinB].cursor, head - 20, "coin B is not held");
  assert.equal(ix.s.coins[coinA].cursor, from + 99, "coin A stops just before its unreadable block, never past it");
  assert.equal(ix.summary().stuck[0].block, from + 100);
  assert.deepEqual(ix.balancesAt(coinA, from + 99), new Map([[A(0x77), 2n * E18]]), "what is kept of coin A is consistent up to its cursor");
  await assert.rejects(ix.rootAt({ coin: coinA, snapshotAt: BigInt(1_700_000_000 + from + 3_000), fromBlock: from, head: head - 20, exclusions: [] }), /stuck at block 1100/);
  const back = mk();
  assert.equal(back.s.coins[coinA].cursor, from + 99, "the file keeps the consistent cursor");
  assert.equal(back.s.coins[coinA].stuck.block, from + 100);
});

test("M-B: a provider that refuses a filter of more than N addresses: the index asks with fewer, and every coin goes on", async () => {
  const { f, coinA, coinB, from, head } = denseChain({ addressLimit: 1, limit: 1_000 });
  const ix = new HolderIndex({ url: "https://rpc.example", span: 1_000, factories: [], fromBlock: from, confirmations: 20, http: f.http });
  ix.addCoin(coinA); ix.addCoin(coinB);
  for (let i = 0; i < 4; i++) await ix.update({ deadline: Date.now() + 5_000 });
  assert.equal(ix.batch, 1);
  assert.equal(ix.s.group.cursor, head - 20);
  assert.equal(ix.s.coins[coinA].cursor, head - 20); assert.equal(ix.s.coins[coinB].cursor, head - 20);
  assert.equal(ix.balancesAt(coinB, head - 20).size, 2);
});

test("M-B: a refusal for the number of addresses is handed to the caller (it does not narrow the span), and the canary (which may skip nothing) stops before a block it cannot read", async () => {
  const { f, coinA, coinB, from } = denseChain({ addressLimit: 1, limit: 1_000 });
  const p = new Provider({ url: "https://rpc.example", span: 1_000 });
  const r = await pagedLogs(p, { address: [coinA, coinB], topics: [TRANSFER] }, from, from + 10, { http: f.http, allowStuck: true });
  assert.equal(r.tooManyAddresses, true); assert.equal(r.upTo, from - 1);
  assert.equal(p.span, 1_000, "the span is untouched");
  const d = denseChain({ receipts: false, txReceipts: false });
  const q = new Provider({ url: "https://rpc.example", span: 1_000 });
  const c2 = await pagedLogs(q, { address: [d.coinA, d.coinB] }, d.from, d.from + 500, { http: d.f.http });
  assert.equal(c2.upTo, d.from + 99, "the canary's read stops before the unreadable block");
  assert.match(c2.error, /could not be read another way/);
});

// ---- false-pause resistance and L-F -----------------------------------------------------------------------------------------------

test("audit3 L-F (fixed): a provider that once dropped a log disagrees about an honest draw; its coin is rebuilt from its launch for that draw, and the draw is then verified", async () => {
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
  const rg = await good.rootAt(input);
  const onChain = { root: rg.root, totalTickets: String(rg.totalTickets), snapshotAt: String(input.snapshotAt) };
  const d0 = { ...onChain, coin, vault: c.raffle, id: "0" }, id = `${c.raffle}:0`, urls = [good.url, lossy.url];
  let rs = [rg, await lossy.rootAt(input)];
  let v = verdictN(onChain, rs, 2);
  assert.equal(v.v, "disagree", "as in the PoC: an honest draw, one provider computes another root");
  let e = evidenceOf(id, d0, rs, urls, v, { indexes: [good, lossy], now: NOW });
  assert.equal(e.dissent["lossy.example"].repaired, false);
  assert.equal(deadlineCall({ ...d0, ...e, status: "disagree", matches: 1 }).act, "escalate", "at the deadline now: SEV1 by hand, NOT a pause (one provider reproduces the exact root)");
  assert.equal(lossy.s.coins[coin].cursor, from - 1, "the lossy provider's coin is being rebuilt from its launch");
  await lossy.update({ deadline: Date.now() + 5_000, priority: [coin] });
  rs = [rg, await lossy.rootAt(input)];
  v = verdictN(onChain, rs, 2);
  assert.equal(v.v, "verified", "repaired: the draw is verified");
  e = evidenceOf(id, { ...d0, ...e }, rs, urls, v, { indexes: [good, lossy], now: NOW });
  assert.deepEqual(e.dissent, {});
});

test("L-F: reconciliation against the chain's state finds a dropped mint with no draw at all, and rebuilds the coin; a state read that fails changes nothing", async () => {
  const coin = A(0xc07), H1 = A(0x2001), from = 1_000, head = from + 500;
  const ev = [{ address: coin, blockNumber: from + 10, topics: [TRANSFER, pad(A(0)), pad(H1)], data: pad(hex(3n * E18)) },
    { address: coin, blockNumber: from + 20, topics: [TRANSFER, pad(A(0)), pad(H1)], data: pad(hex(2n * E18)) }]
    .map((l, i) => ({ ...l, blockNumber: hex(l.blockNumber), logIndex: hex(i), transactionHash: pad(hex(i + 1)), removed: false }));
  let drop = true, state = true, t = NOW;
  const f = fakeChain({
    views: (url, to, fn) => { if (!state) throw new Error("historical state is not available"); if (fn === "totalSupply") return 5n * E18; if (fn === "balanceOf") return 5n * E18; throw new Error(fn); },
    other: (url, method, params) => {
      if (method === "eth_blockNumber") return hex(head);
      const q = params[0], a = Number(BigInt(q.fromBlock)), b = Number(BigInt(q.toBlock));
      const hit = ev.filter((l) => Number(BigInt(l.blockNumber)) >= a && Number(BigInt(l.blockNumber)) <= b);
      if (drop && hit.length > 1) { drop = false; return hit.slice(0, 1); }
      return hit;
    } });
  const ix = new HolderIndex({ url: "https://lossy.example", span: 10_000, fromBlock: from, confirmations: 0, http: f.http, clock: () => t });
  ix.addCoin(coin);
  await ix.update({ deadline: Date.now() + 5_000 });
  assert.deepEqual(ix.balancesAt(coin, head), new Map([[H1.toLowerCase(), 3n * E18]]), "the dropped mint: silently short (no balance went negative)");
  state = false;
  assert.deepEqual(await ix.reconcile({ head }), [], "no state readable: nothing changes");
  assert.equal(ix.s.coins[coin].reconcile.v, "unreadable");
  state = true; t += RECONCILE.everyMs;
  const found = await ix.reconcile({ head });
  assert.equal(found.length, 1);
  assert.match(found[0].why, /sum to 3000000000000000000, totalSupply\(\) is 5000000000000000000/);
  assert.equal(found[0].rebuilt, true);
  await ix.update({ deadline: Date.now() + 5_000 });
  assert.deepEqual(ix.balancesAt(coin, head), new Map([[H1.toLowerCase(), 5n * E18]]), "rebuilt from the launch: right");
  t += RECONCILE.everyMs;
  assert.deepEqual(await ix.reconcile({ head }), [], "and it reconciles");
  assert.equal(ix.s.coins[coin].reconcile.v, "ok");
});

test("L-F: a coin marked broken (logs missing twice) is rebuilt again after BROKEN_RETRY_MS, not never; an old file's `resets: 1` does not break a coin for good", async () => {
  let t = NOW;
  const f = fakeChain({ other: (url, method) => { if (method === "eth_blockNumber") return hex(5_000); return []; } });
  const ix = new HolderIndex({ url: "https://x.example", span: 10_000, fromBlock: 1_000, confirmations: 0, retentionBlocks: 100, http: f.http, clock: () => t });
  ix.addCoin(A(0xc09));
  const k = A(0xc09);
  ix.s.coins[k] = { ...ix.s.coins[k], cursor: 4_000, logs: [[1_500, 0, A(0x3001), A(0x3002), "1"]], resets: 1 };   // a spend with no mint before it
  ix.compact(4_900);
  assert.ok(!ix.s.coins[k].broken, "a legacy `resets: 1` with no time is an expired reset: rebuilt, not broken");
  ix.s.coins[k] = { ...ix.s.coins[k], cursor: 4_000, logs: [[1_500, 0, A(0x3001), A(0x3002), "1"]] };
  ix.compact(4_900);
  assert.ok(ix.s.coins[k].broken, "missing again within the reset TTL: broken");
  t += BROKEN_RETRY_MS;
  ix.compact(4_900);
  assert.ok(!ix.s.coins[k].broken, "an hour later it is rebuilt again");
  assert.equal(ix.s.coins[k].cursor, 999);
});

test("the deadline pauses on evidence only (fixes-R4-services.md §3.3)", () => {
  const deg = { r: "error", cls: "degraded", why: "a.example: the index of 0xc is at block 5, behind the snapshot block 9" };
  const rows = [
    [{ matches: 1, status: "pending", per: { a: { r: "match" }, b: deg } }, "single"],
    [{ matches: 0, status: "pending", per: { a: deg, b: deg } }, "escalate"],
    [{ matches: 0, status: "pending", per: { a: deg, b: { r: "other" } }, dissent: { b: { root: "0x1", tickets: "3", repaired: false } } }, "pause"],
    [{ matches: 0, status: "pending", per: { a: deg, b: deg }, dissent: { b: { root: "0x1", tickets: "3", repaired: false } } }, "pause"],
    [{ matches: 1, status: "disagree", per: { a: { r: "match" }, b: { r: "other" } }, dissent: { b: { root: "0x1", tickets: "3", repaired: false } } }, "escalate"],
    [{ matches: 1, status: "disagree", per: { a: { r: "match" }, b: { r: "other" } }, dissent: { b: { root: "0x1", tickets: "3", repaired: true } } }, "pause"],
    [{ matches: 0, status: "pending", per: { a: deg, b: { r: "error", cls: "unexplained", why: "b: no block at or before 5" } } }, "pause"],
  ];
  for (const [d, want] of rows) assert.equal(deadlineCall(d).act, want, JSON.stringify(d));
  assert.equal(deadlineCall(rows[1][0], { pauseOnDegraded: true }).act, "pause", "draws.pause_on_degraded restores round 3's rule");
  for (const e of ["a: the index of 0x1 is at block 3, behind the snapshot block 9", "a: the index of 0x1 is stuck at block 7, behind the snapshot block 9", "a.example is cooling down after throttling (busy)",
    "a: -32005 the network is busy", "the poll's time budget ran out", "a answered HTTP 403: denied", "a: the index of 0x1 is broken (logs are missing)"]) assert.equal(errorClass(e), "degraded", e);
  assert.equal(errorClass("a: no block at or before 1700 after the stack's start block 1000"), "unexplained");
});

/** A 4663 with one Open draw in the RAFFLE vault, 5 minutes before its pin (inside the deadline). */
function deadlineWorld() {
  return fakeChain({
    views: (url, to, fn, args) => {
      if (fn === "drawCount") return to === c.raffle ? 1n : 0n;
      if (fn === "draw") return { stage: 1, coin: S.coin, challengeEnds: BigInt(NOW / 1000 + 300), challengeEndsBlock: 0n, round: 0n, pinnedAt: 0n, pinnedAtBlock: 0n,
        resolvedAt: 0n, resolvedAtBlock: 0n, root: `0x${"ab".repeat(32)}`, totalTickets: 1000n, winningTicket: 0n, winner: ZERO, uri: "https://m.example/s.json", seedRound: 0n,
        announcedAt: 0n, announcedAtBlock: 0n, snapshotAt: 1_789_990_000n };
      if (fn === "seedOrigin") return args[0];
      if (fn === "routerOf") return A(0xc2);
      throw new Error(`no view ${fn}`);
    },
    other: (url, method) => { if (method === "eth_blockNumber") return hex(71_200_200); if (method === "eth_getLogs") return []; if (method === "eth_getBlockByNumber") return { timestamp: hex(NOW / 1000 - 3600) }; throw new Error(method); },
  });
}
const spy = () => { const done = []; return { done, address: S.guardian, autoPause: true, autoCancel: true, pauseKeeper: async () => { done.push("pause"); return { done: true, tx: "0x1" }; }, cancelDraw: async () => { done.push("cancel"); return { done: true, tx: "0x2" }; } }; };
const cfgE = { rpcs: ["https://a.example", "https://b.example"], stack: S.stack, from_block: 71_200_000, confirmations: 20 };

test("R3-check M-A/M-B as a false-pause lever (fixed): an honest draw whose providers are only DEGRADED at its deadline is SEV1 and escalated, the checker degraded, and the keeper is NOT paused; with draws.pause_on_degraded it is", async () => {
  const { http } = deadlineWorld();
  const behind = async (url) => { throw new Error(`${new URL(url).host}: the index of ${S.coin} is stuck at block 71199500, behind the snapshot block 71199900`); };
  const g = spy();
  const r = await checkEvm({ cfg: cfgE, prev: {}, now: NOW, http, readRecords: async () => EMPTY_RECORDS, timelock: "tl", guardian: g, recompute: behind });
  assert.deepEqual(g.done, [], "no pause");
  const a = r.alerts.find((x) => x.key === `draw-deadline:${c.raffle}:0`);
  assert.equal(a.sev, "SEV1"); assert.match(a.text, /ESCALATE — ACT BY HAND/);
  assert.ok(r.alerts.some((x) => x.key.startsWith(`draw-deadline-repeat:${c.raffle}:0:`)), "repeated every poll until the pin");
  assert.match(r.degraded, /unverified at its deadline/, "the checker is degraded: the monitor's /healthz answers 503");
  const g2 = spy();
  await checkEvm({ cfg: { ...cfgE, draws: { pause_on_degraded: true } }, prev: {}, now: NOW, http, readRecords: async () => EMPTY_RECORDS, timelock: "tl", guardian: g2, recompute: behind });
  assert.deepEqual(g2.done, ["pause"], "the operator's fail-closed choice");
});

test("the deadline still pauses a forged root when only one provider can read the coin: it computes another root and nobody reproduces the chain's", async () => {
  const { http } = deadlineWorld();
  const rec = async (url) => { if (url.includes("b.example")) throw new Error("b.example: the network is busy"); return { root: `0x${"cd".repeat(32)}`, totalTickets: 1000n, block: 71_199_900, holders: 3 }; };
  const g = spy();
  const r = await checkEvm({ cfg: cfgE, prev: {}, now: NOW, http, readRecords: async () => EMPTY_RECORDS, timelock: "tl", guardian: g, recompute: rec });
  assert.deepEqual(g.done, ["pause"]);
  assert.match(r.alerts.find((x) => x.key === `draw-deadline:${c.raffle}:0`).text, /no provider reproduces the root, and a\.example computes 0xcdcd/);
});

// ---- L-C: the floor-walk rule cannot be evaded by interleaved posts ---------------------------------------------------------------

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

test("audit3 L-C (fixed): a no-move post one second after every second edge post no longer resets the edge run: SEV1 at the third edge post, the 6 h net move too, and pause_on_floor_walk pauses", () => {
  const H = 3_600_000;
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
  const edge = alerts.filter((a) => a.key.startsWith("floor-walk:"));
  assert.ok(edge.length >= 1 && edge[0].sev === "SEV1", "the edge run pages");
  assert.ok(edge[0].t <= 3 * H + 2_000, `at the third edge post (${edge[0].t / H} h)`);
  assert.ok(alerts.some((a) => a.key.startsWith("floor-walk6:") && a.sev === "SEV1"), "and the 6 h net move");
  assert.ok(triggers.some((x) => x.action === "pause"), "pause_on_floor_walk now fires");
  assert.ok(walk(posts, false).triggers.length === 0, "and only when told to");
});

test("audit3 L-C (2) (fixed): a walk just under 10 % a day is paged SEV1 on the three-day rule as soon as three days are kept (−27 %), long before −50 %", () => {
  const H = 3_600_000;
  const posts = [];
  let f = 65n * E18 / 100n;
  for (let h = 0; h <= 24 * 7; h++) { if (h) f = f * 99_561n / 100_000n; posts.push([h * H, f]); }
  const { alerts, triggers } = walk(posts, true);
  const three = alerts.filter((a) => a.key.startsWith("floor-3day-move:"));
  const sev1 = three.find((a) => a.sev === "SEV1");
  assert.ok(sev1, "the three-day rule pages SEV1 (the day rule, at 9.6 %, still pages nothing: the PoC's point)");
  assert.equal(alerts.filter((a) => a.key.startsWith("floor-day-move:")).length, 0);
  const start = 65n * E18 / 100n;
  const at = (t) => { let x = start; for (let h = 1; h <= t / H; h++) x = x * 99_561n / 100_000n; return Number(x * 1000n / start) / 10; };
  assert.ok(at(sev1.t) >= 70, `SEV1 while the floor is still at ${at(sev1.t)} % of its start`);
  assert.ok(triggers.some((x) => x.key.startsWith("floor-3day-move:")), "with the trigger under pause_on_floor_walk");
});

// ---- A3-I1 (contracts, decided here): the guardian may pause on a long stretch with no good floor post ---------------------------

test("A3-I1: with evm.guardian.pause_on_no_good_post_h, a module with no good post for that long pages SEV1 and the guardian pauses (paused burns pay the frozen floor); without it, never", async () => {
  const lastGood = BigInt(NOW / 1000 - 13 * 3600);
  const w = fakeChain({
    views: (url, to, fn) => { if (fn === "lastGoodAt") return lastGood; if (fn === "drawCount") return 0n; throw new Error(`no view ${fn}`); },
    other: (url, method) => { if (method === "eth_blockNumber") return hex(71_200_200); if (method === "eth_getLogs") return []; throw new Error(method); },
  });
  const run = async (guard) => { const g = spy(); const r = await checkEvm({ cfg: { ...cfgE, guardian: guard }, prev: {}, now: NOW, http: w.http, readRecords: async () => EMPTY_RECORDS, timelock: "tl", guardian: g }); return { g, r }; };
  const on = await run({ address: S.guardian, pause_on_no_good_post_h: 12 });
  assert.deepEqual(on.g.done, ["pause"]);
  assert.ok(on.r.alerts.some((a) => a.sev === "SEV1" && /NO GOOD .* FLOOR POST FOR 13\.0 h/.test(a.text)));
  assert.deepEqual((await run({ address: S.guardian, pause_on_no_good_post_h: 24 })).g.done, [], "not yet at 24 h");
  assert.deepEqual((await run({ address: S.guardian })).g.done, [], "off unless configured");
});
