import { mkdirSync, writeFileSync } from "node:fs";
import { balancesOf, serialise, snapshot, type TransferLog } from "./core.js";

/**
 * Writes the fixtures `test/SnapshotTool.t.sol` and `test/SnapshotToolV2.t.sol` read: snapshots built by THIS tool from a made-up transfer history, which the
 * Solidity contracts must accept — root, ticket ranges, share amounts and every proof. Deterministic: run it twice, diff nothing.
 */
const A = (n: number) => `0x${n.toString(16).padStart(40, "0")}`;
const ZERO = A(0), CURVE = A(0xc0de), ONE = 10n ** 18n;
let i = 0;
const t = (from: string, to: string, value: bigint): TransferLog => ({ blockNumber: 100n + BigInt(i), logIndex: i++, from, to, value });
const logs: TransferLog[] = [
  t(ZERO, CURVE, 1_000_000n * ONE),
  t(CURVE, A(0xa11ce), 600n * ONE), t(CURVE, A(0xb0b), 300n * ONE), t(CURVE, A(0xca201), 90n * ONE + ONE / 2n),
  t(CURVE, A(0xd0d0), ONE / 2n),                      // under one whole coin: no ticket
  t(A(0xa11ce), A(0xb0b), 50n * ONE),                // a transfer between holders
  t(CURVE, A(0xe1e), 9n * ONE), t(A(0xe1e), CURVE, 9n * ONE),   // bought and sold back: not a holder
  t(CURVE, A(0xf00d), 10n * ONE),
];
const bal = balancesOf(logs);
const base = { coin: A(0xc01), block: "108", decimals: 18, exclusions: [ZERO, CURVE, A(0xc01)] };
mkdirSync(new URL("../../../test/fixtures/", import.meta.url), { recursive: true });
for (const [name, s] of [
  ["snapshot-weighted", snapshot(bal, { ...base, kind: "weighted" })],
  ["snapshot-per-wallet", snapshot(bal, { ...base, kind: "per-wallet" })],
  ["snapshot-pro-rata", snapshot(bal, { ...base, kind: "pro-rata", total: "100000" })],
  // v2 (docs/snapshot-v1.md "Version 2"): a SweepVault draw's tree, with the sentinel leaf (address(0), totalTickets, snapshotAt)
  ["snapshot-weighted-v2", snapshot(bal, { ...base, kind: "weighted", snapshotAt: "1790000000" })],
] as const) writeFileSync(new URL(`../../../test/fixtures/${name}.json`, import.meta.url), serialise(s));
console.log("fixtures written");
