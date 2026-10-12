import assert from "node:assert/strict";
import { test } from "node:test";
import { readFileSync } from "node:fs";
import { balancesOf, merkle, serialise, shareLeaf, snapshot, ticketLeaf, verify, type TransferLog } from "../src/core.js";

const A = (n: number) => `0x${n.toString(16).padStart(40, "0")}`;
const ONE = 10n ** 18n;
const log = (i: number, from: string, to: string, value: bigint): TransferLog => ({ blockNumber: 10n, logIndex: i, from, to, value });

test("balances: logs applied in (block, log index) order whatever order they arrive in; a missing log is an error, not a guess", () => {
  const logs = [log(2, A(1), A(2), 40n), log(0, A(0), A(1), 100n), log(1, A(1), A(3), 10n)];
  const b = balancesOf(logs);
  assert.deepEqual([b.get(A(1)), b.get(A(2)), b.get(A(3))], [50n, 40n, 10n]);
  assert.throws(() => balancesOf([log(0, A(1), A(2), 1n)]), /logs are missing/);
});

test("weighted: one ticket a whole coin, contiguous ranges in address order, exclusions out, dust holders dropped", () => {
  const bal = new Map([[A(0xb), 3n * ONE + 5n], [A(0xa), 2n * ONE], [A(0xc), ONE - 1n], [A(0xdead), 999n * ONE]]);
  const s = snapshot(bal, { coin: A(9), block: "1", kind: "weighted", decimals: 18, exclusions: [A(0xDEAD).toUpperCase().replace("0X", "0x")] });
  assert.deepEqual(s.entries.map((e) => [e.holder, e.first, e.end]), [[A(0xa), "0", "2"], [A(0xb), "2", "5"]]);
  assert.equal(s.totalTickets, "5");
  for (const e of s.entries) {
    assert.equal(e.leaf, ticketLeaf(e.holder, BigInt(e.first!), BigInt(e.end!)));
    assert.ok(verify(e.proof, s.root, e.leaf));
  }
});

test("per-wallet: one ticket each at or above the minimum; pro-rata: floors, zero shares dropped, the remainder named", () => {
  const bal = new Map([[A(1), 5n * ONE], [A(2), ONE], [A(3), ONE / 2n]]);
  const w = snapshot(bal, { coin: A(9), block: "1", kind: "per-wallet", decimals: 18, exclusions: [] });
  assert.deepEqual(w.entries.map((e) => [e.first, e.end]), [["0", "1"], ["1", "2"]]);
  const p = snapshot(bal, { coin: A(9), block: "1", kind: "pro-rata", decimals: 18, total: "10", exclusions: [] });
  assert.deepEqual(p.entries.map((e) => e.units), ["7", "1"]);          // 50/65, 10/65, 5/65 of 10 → 7, 1, 0
  assert.deepEqual([p.allocated, p.remainder], ["8", "2"]);
  assert.equal(p.entries[0].leaf, shareLeaf(A(1), 7n));
  assert.throws(() => snapshot(bal, { coin: A(9), block: "1", kind: "pro-rata", decimals: 18, exclusions: [] }), /total/);
});

test("the tree: every proof verifies, a wrong leaf does not, one leaf is its own root, none is refused, odd layers hold", () => {
  for (const n of [1, 2, 3, 5, 8, 13, 100]) {
    const leaves = Array.from({ length: n }, (_, i) => shareLeaf(A(i + 1), BigInt(i + 1)));
    const t = merkle(leaves);
    for (const l of leaves) assert.ok(verify(t.proofOf(l), t.root, l), `n=${n}`);
    assert.ok(!verify(t.proofOf(leaves[0]), t.root, shareLeaf(A(1), 2n)));
    if (n === 1) assert.equal(t.root, leaves[0]);
  }
  assert.throws(() => merkle([]), /no root/);
});

test("deterministic, byte for byte: holder order in, run count and key order do not change the file", () => {
  const entries: [string, bigint][] = [[A(5), 9n * ONE], [A(1), 2n * ONE], [A(3), 4n * ONE]];
  const inputs = { coin: A(9), block: "7", kind: "weighted" as const, decimals: 18, exclusions: [A(2), A(1).toUpperCase().replace("0X", "0x")] };
  const a = serialise(snapshot(new Map(entries), inputs)), b = serialise(snapshot(new Map([...entries].reverse()), { ...inputs, exclusions: [...inputs.exclusions].reverse() }));
  assert.equal(a, b);
  assert.ok(!/\d{4}-\d{2}-\d{2}T/.test(a), "no timestamps");
});

test("the committed fixtures are what this tool writes today", async () => {
  const before = ["weighted", "per-wallet", "pro-rata", "weighted-v2"].map((k) => readFileSync(new URL(`../../../test/fixtures/snapshot-${k}.json`, import.meta.url), "utf8"));
  await import("../src/fixtures.js");
  const after = ["weighted", "per-wallet", "pro-rata", "weighted-v2"].map((k) => readFileSync(new URL(`../../../test/fixtures/snapshot-${k}.json`, import.meta.url), "utf8"));
  assert.deepEqual(after, before);
});

test("structurally read-only: nothing in this package can sign or send", () => {
  for (const f of ["core.ts", "rpc.ts", "cli.ts", "fixtures.ts", "stack.ts", "giveaway.ts"]) {
    const src = readFileSync(new URL(`../src/${f}`, import.meta.url), "utf8");
    assert.ok(!/privateKey|createWalletClient|sendTransaction|eth_sendRawTransaction|signTransaction|mnemonic|viem\/accounts/i.test(src), f);
  }
});
