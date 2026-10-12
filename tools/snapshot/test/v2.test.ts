import assert from "node:assert/strict";
import { test } from "node:test";
import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { serialise, sentinelLeaf, snapshot, ticketLeaf, verify } from "../src/core.js";
import { isRateLimited, isTooLarge, lastBlockAtOrBefore, rpc, transferLogsWith, type RawLog } from "../src/rpc.js";
import { readExclusions, stackExclusions } from "../src/stack.js";

// docs/snapshot-v1.md "Version 2": the sentinel leaf, the time → block rule, the log scan's split, and exclusions that fail closed.
const A = (n: number) => `0x${n.toString(16).padStart(40, "0")}`;
const ONE = 10n ** 18n;

test("v2: the sentinel leaf (holder zero, [totalTickets, snapshotAt)) is in the tree with its proof; holders' proofs still verify", () => {
  const bal = new Map([[A(0xa), 2n * ONE], [A(0xb), 3n * ONE + 5n], [A(0xc), ONE - 1n]]);
  const s = snapshot(bal, { coin: A(9), block: "77", kind: "weighted", decimals: 18, exclusions: [], snapshotAt: "1790000123" });
  assert.equal(s.version, 2);
  assert.equal(s.totalTickets, "5");
  assert.equal(s.sentinel!.leaf, ticketLeaf(A(0), 5n, 1_790_000_123n), "the contract's leaf(address(0), totalTickets, snapshotAt)");
  assert.equal(s.sentinel!.leaf, sentinelLeaf(5n, 1_790_000_123n));
  assert.ok(verify(s.sentinel!.proof, s.root, s.sentinel!.leaf));
  assert.ok(!verify(s.sentinel!.proof, s.root, sentinelLeaf(4n, 1_790_000_123n)), "a truncated count is another leaf");
  for (const e of s.entries) assert.ok(verify(e.proof, s.root, e.leaf));
  assert.equal(s.entries.length, 2, "the sentinel is not an entry (claim refuses holder zero)");
  // without a time the output is v1, byte for byte, and the root differs from v2's (the sentinel is committed)
  const v1 = snapshot(bal, { coin: A(9), block: "77", kind: "weighted", decimals: 18, exclusions: [] });
  assert.equal(v1.version, 1);
  assert.equal(v1.sentinel, undefined);
  assert.notEqual(v1.root, s.root);
  assert.ok(!serialise(v1).includes("sentinel"));
  // a holder-less coin still has a root (the sentinel alone); openDraw refuses totalTickets = 0 on chain
  const empty = snapshot(new Map(), { coin: A(9), block: "1", kind: "weighted", decimals: 18, exclusions: [], snapshotAt: "10" });
  assert.equal(empty.totalTickets, "0");
  assert.equal(empty.root, empty.sentinel!.leaf);
  assert.throws(() => snapshot(bal, { coin: A(9), block: "1", kind: "per-wallet", decimals: 18, exclusions: [], snapshotAt: "10" }), /weighted/);
});

test("v2, fixes-R3-carry: no eligible holder gives the sentinel alone (root = sentinelLeaf(0, T), proof []), what openDraw takes for an empty draw", () => {
  const T = "1790000000", E18 = 10n ** 18n;
  const X = "0x00000000000000000000000000000000000000c1";
  // a holder under one whole coin and an excluded holder: no ticket at all
  const bal = new Map([["0x00000000000000000000000000000000000000a1", E18 / 2n], [X, 5n * E18]]);
  const s = snapshot(bal, { coin: "0x00000000000000000000000000000000000000cc", block: "10", kind: "weighted", decimals: 18, exclusions: [X], snapshotAt: T });
  assert.equal(s.version, 2);
  assert.equal(s.totalTickets, "0");
  assert.equal(s.holders, 0);
  assert.deepEqual(s.entries, []);
  assert.equal(s.root, sentinelLeaf(0n, BigInt(T)));
  assert.equal(s.root, ticketLeaf("0x0000000000000000000000000000000000000000", 0n, BigInt(T)));
  assert.deepEqual(s.sentinel!.proof, []);
  assert.ok(verify(s.sentinel!.proof, s.root, s.sentinel!.leaf));
  // v1 (no snapshot time) of nobody still has no root
  assert.throws(() => snapshot(bal, { coin: "0x00000000000000000000000000000000000000cc", block: "10", kind: "weighted", decimals: 18, exclusions: [X] }), /empty snapshot has no root/);
});

test("the snapshot block of a time is the LAST block stamped at or before it, with many blocks per second; refused while no later block exists", async () => {
  const stamps = [100, 100, 101, 101, 101, 101, 102, 105, 105, 106].map(BigInt);     // block n is stamped stamps[n]
  const ts = async (n: bigint) => stamps[Number(n)];
  const head = BigInt(stamps.length - 1);
  assert.equal(await lastBlockAtOrBefore(ts, head, 101n), 5n);
  assert.equal(await lastBlockAtOrBefore(ts, head, 103n), 6n, "a second with no block: the last one before it");
  assert.equal(await lastBlockAtOrBefore(ts, head, 100n), 1n);
  assert.equal(await lastBlockAtOrBefore(ts, head, 105n), 8n);
  await assert.rejects(lastBlockAtOrBefore(ts, head, 106n), /no block is stamped after/);
  await assert.rejects(lastBlockAtOrBefore(ts, head, 99n), /already stamped after/);
});

test("E-11: a log range over the provider's 10,000-log cap is halved until it fits; any other failure throws (never a partial snapshot)", async () => {
  const blocks = Array.from({ length: 64 }, (_, i) => i);
  const logOf = (b: number): RawLog => ({ blockNumber: `0x${b.toString(16)}`, logIndex: "0x0", topics: ["0xddf2", `0x${A(0).slice(2).padStart(64, "0")}`, `0x${A(b + 1).slice(2).padStart(64, "0")}`], data: "0x01" });
  const asked: [bigint, bigint][] = [];
  const get = async (a: bigint, e: bigint) => {
    asked.push([a, e]);
    if (e - a + 1n > 8n) throw new Error("eth_getLogs: logs matched by query exceeds limit of 10000");
    return blocks.filter((b) => BigInt(b) >= a && BigInt(b) <= e).map(logOf);
  };
  const logs = await transferLogsWith(get, 0n, 63n, 64n);
  assert.equal(logs.length, 64, "every block's log, once");
  assert.deepEqual(logs.map((l) => Number(l.blockNumber)), blocks);
  assert.ok(asked.some(([a, e]) => e - a + 1n === 8n) && asked.every(([a, e]) => e >= a));
  await assert.rejects(transferLogsWith(async () => { throw new Error("eth_getLogs: HTTP 502"); }, 0n, 10n), /502/);
  // a single block that is itself over the cap cannot be split: it throws rather than skipping it
  await assert.rejects(transferLogsWith(async () => { throw new Error("exceeds limit of 10000"); }, 5n, 5n), /exceeds limit/);
  assert.ok(isTooLarge("logs matched by query exceeds limit of 10000"));
  assert.ok(isRateLimited(-32005, "the network is busy, please try again in a moment") && isRateLimited(429, "Too Many Requests"));
  assert.ok(!isRateLimited(3, "execution reverted"));
});

test("E-04: a JSON-RPC throttle answered in HTTP 200 (429 / -32005) is retried with backoff, not taken as the answer", async () => {
  let n = 0;
  const f = (async () => {
    n++;
    const body = n === 1 ? { error: { code: 429, message: "Too Many Requests" } } : n === 2 ? { error: { code: -32005, message: "the network is busy" } } : { result: "0x2a" };
    return { ok: true, status: 200, json: async () => ({ jsonrpc: "2.0", id: 1, ...body }) };
  }) as unknown as typeof fetch;
  assert.equal(await rpc<string>("https://rpc.example", "eth_blockNumber", [], f, async () => {}), "0x2a");
  assert.equal(n, 3);
  const revert = (async () => ({ ok: true, status: 200, json: async () => ({ error: { code: 3, message: "execution reverted" } }) })) as unknown as typeof fetch;
  await assert.rejects(rpc("https://rpc.example", "eth_call", [], revert, async () => {}), /execution reverted/);
});

test("E-14: the exclusions file fails closed (unreadable, not a list, a bad address); the stack's exclusions are the keeper's", () => {
  const dir = mkdtempSync(join(tmpdir(), "snap-"));
  assert.throws(() => readExclusions(join(dir, "missing.json")), /could not be read/);
  writeFileSync(join(dir, "bad.json"), "{");
  assert.throws(() => readExclusions(join(dir, "bad.json")), /could not be read/);
  writeFileSync(join(dir, "obj.json"), "{}");
  assert.throws(() => readExclusions(join(dir, "obj.json")), /not a list/);
  writeFileSync(join(dir, "addr.json"), JSON.stringify([{ address: "0x12" }]));
  assert.throws(() => readExclusions(join(dir, "addr.json")), /entry 0/);
  writeFileSync(join(dir, "ok.json"), JSON.stringify([{ address: A(0xABC).toUpperCase().replace("0X", "0x"), why: "x" }]));
  assert.deepEqual(readExclusions(join(dir, "ok.json")), [A(0xabc)]);
  const real = readExclusions(new URL("../exclusions.json", import.meta.url).pathname);
  assert.ok(real.length >= 8, "the committed list reads");
  const stack = { poolManager: A(1), treasury: A(2), registry: A(3), collections: [{ slug: "frogs", factory: A(4), lock: A(5), router: A(6), raffleVault: A(7), burnVault: A(8),
    yoyoVault: A(0), floorModule: A(9), mintDesk: A(10), }] };
  const ex = stackExclusions(stack, "frogs", A(0x20), A(0x21), [A(0xabc)]);
  for (const a of [A(0), "0x000000000000000000000000000000000000dead", A(0x20), A(0x21), A(1), A(2), A(3), A(4), A(5), A(6), A(7), A(8), A(9), A(10), A(0xabc)]) assert.ok(ex.includes(a), a);
  assert.throws(() => stackExclusions(stack, "toads", A(0x20), A(0x21), []), /no collection toads/);
});
