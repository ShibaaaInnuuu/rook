// Second review (R2), 2026-09-26, kept as a regression in round 3: the monitor's port of the tree (monitor/src/draws.mjs) against this tool's core, on random and
// adversarial holder sets (self-transfers, transfers into and out of excluded addresses, dust, mixed-case addresses, a holder at
// exactly one coin, mint/burn, many holders). They must give the same root, count and holder number, or both refuse.
import assert from "node:assert/strict";
import { test } from "node:test";
import { balancesOf, snapshot, type TransferLog } from "../src/core.js";
// @ts-expect-error the monitor is plain ESM without types
import * as M from "../../../monitor/src/draws.mjs";

const ZERO = "0x0000000000000000000000000000000000000000";
const E18 = 10n ** 18n;
let seed = 0x5eed;
const rnd = (n: number) => { seed = (seed * 1103515245 + 12345) & 0x7fffffff; return seed % n; };
const addr = (i: number) => {
  const h = (BigInt(i) * 0x9e3779b97f4a7c15n % (1n << 160n)).toString(16).padStart(40, "0");
  return "0x" + (rnd(2) ? h.toUpperCase() : h);
};

test("R2-T1: the monitor's tree equals tools/snapshot's on 300 random adversarial histories", () => {
  let ok = 0, maxH = 0;
  for (let round = 0; round < 300; round++) {
    const pool = Array.from({ length: 2 + rnd(40) }, (_, i) => addr(i + 1 + round * 100));
    const excluded = pool.filter(() => rnd(5) === 0);
    const bal = new Map<string, bigint>();
    const logs: TransferLog[] = [];
    let block = 100n;
    for (let k = 0; k < 5 + rnd(120); k++) {
      const to = pool[rnd(pool.length)];
      const kind = rnd(6);
      const holders = [...bal].filter(([, b]) => b > 0n);
      const pick = holders.length ? holders[rnd(holders.length)] : null;
      let from = ZERO, value: bigint;
      if (kind === 0 || !pick) value = BigInt(rnd(5)) * E18 + BigInt(rnd(3)) * (E18 - 1n);                 // mint, incl. 0 and dust-ish
      else { from = pick[0]; value = kind === 1 ? pick[1] : kind === 2 ? pick[1] % E18 : BigInt(rnd(Number(pick[1] > 10n ** 15n ? 10n ** 15n : pick[1]) + 1)) * (pick[1] / (10n ** 15n) + 1n) % (pick[1] + 1n); }
      const dest = kind === 3 && pick ? pick[0] : kind === 4 ? ZERO : to;                                       // self transfer, burn
      if (from !== ZERO) bal.set(from.toLowerCase(), (bal.get(from.toLowerCase()) ?? 0n) - value);
      if (dest !== ZERO) bal.set(dest.toLowerCase(), (bal.get(dest.toLowerCase()) ?? 0n) + value);
      logs.push({ blockNumber: block, logIndex: rnd(3), from, to: dest, value });
      if (rnd(3) === 0) block++;
      // keep (block, logIndex) unique and ordered like the chain
      logs[logs.length - 1].logIndex = logs.filter((l) => l.blockNumber === block).length - 1;
    }
    const snapshotAt = 1_789_990_000n + BigInt(rnd(86_400));
    let tool: { root: string; total: bigint; holders: number } | null = null, mon: { root: string; total: bigint; holders: number } | null = null;
    let toolErr = "", monErr = "";
    try {
      const s = snapshot(balancesOf(logs), { coin: "0x" + "c0".repeat(20), block: String(block), kind: "weighted", decimals: 18, exclusions: excluded, snapshotAt: String(snapshotAt) });
      tool = { root: s.root, total: BigInt(s.totalTickets!), holders: s.holders };
    } catch (e) { toolErr = String((e as Error).message); }
    try {
      const t = M.weightedTree(M.balancesOf(logs), excluded, { sentinel: { snapshotAt } });
      mon = { root: t.root, total: t.totalTickets, holders: t.holders };
    } catch (e) { monErr = String((e as Error).message); }
    if (toolErr || monErr) { assert.ok(toolErr && monErr, `round ${round}: one refused and the other did not: tool "${toolErr}" monitor "${monErr}"`); continue; }
    assert.deepEqual(mon, tool, `round ${round}`); ok++; if (tool!.holders > maxH) maxH = tool!.holders;
  }
  assert.ok(ok > 100, `only ${ok} rounds built a tree`); console.log(`# compared ${ok} trees, up to ${maxH} holders`);
});
