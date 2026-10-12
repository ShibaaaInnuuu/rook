import assert from "node:assert/strict";
import { test } from "node:test";
import { existsSync, readFileSync } from "node:fs";
import { toEventSelector, type AbiEvent } from "viem";
import { foldDraws, judgeList, sentinelLeaf, ticketLeaf } from "../src/wrapped/draws.js";
import { DRAW_EVENTS } from "../src/wrapped/abis.js";
import type { WrapperEvent } from "../src/wrapped/ledger.js";
import type { Address } from "../types.js";

// fixes-R3-carry: a draw whose snapshot had no eligible holder is opened as the sentinel alone (totalTickets 0) and closed by
// requestDraw with no winner (DrawEmpty). The indexer folds it to `empty`: final, nothing to void, and its seed is used, not passed on.
const VAULT = "0x00000000000000000000000000000000000000f1" as Address, COIN = "0x00000000000000000000000000000000000000c0" as Address;
const ev = (name: string, args: Record<string, unknown>, block: number, ts: number, logIndex = 0): WrapperEvent =>
  ({ name, args, block, logIndex, ts, tx: `0x${block.toString(16).padStart(64, "0")}` } as unknown as WrapperEvent);

test("DrawEmpty folds a draw to `empty`: no winner, final (nothing to void), its carried seed used up rather than passed on", () => {
  const events = [
    ev("DrawAnnounced", { drawId: 0n, coin: COIN, seedRound: 100n }, 1, 1_000),
    ev("DrawVoided", { drawId: 0n }, 2, 1_000 + 6 * 3_600 + 1),
    ev("SeedCarried", { drawId: 1n, originId: 0n, seedRound: 100n }, 3, 90_000, 0),
    ev("DrawAnnounced", { drawId: 1n, coin: COIN, seedRound: 100n }, 3, 90_000, 1),
    ev("SnapshotFixed", { drawId: 1n, snapshotAt: 500n }, 4, 91_000),
    ev("DrawOpened", { drawId: 1n, coin: COIN, root: `0x${"11".repeat(32)}`, totalTickets: 0n, uri: "https://l.example/1.json", challengeEnds: 91_900n }, 4, 91_000, 1),
    ev("DrawEmpty", { drawId: 1n, coin: COIN, snapshotAt: 500n }, 5, 92_000),
    ev("DrawAnnounced", { drawId: 2n, coin: COIN, seedRound: 9_000n }, 6, 200_000),
  ];
  const ds = new Map(foldDraws(VAULT, events).map((d) => [d.id, d]));
  const d = ds.get(1)!;
  assert.equal(d.stage, "empty");
  assert.equal(d.totalTickets, "0");
  assert.equal(d.snapshotAt, 500);
  assert.equal(d.seedOrigin, 0);
  assert.equal(d.seedPassedOn, false, "a closed empty draw consumes the carry");
  assert.equal(d.voidableAt, null, "final");
  assert.equal(d.winner, null);
  assert.equal(ds.get(2)!.seedOrigin, null, "the coin's next draw has its own seed");
});

test("DrawEmpty is the committed SweepVault's event (skipped without out/)", () => {
  const f = new URL("../../out/SweepVault.sol/SweepVault.json", import.meta.url).pathname;
  if (!existsSync(f)) return;
  const abi = (JSON.parse(readFileSync(f, "utf8")) as { abi: { type: string }[] }).abi;
  const onChain = new Set(abi.filter((x) => x.type === "event").map((x) => toEventSelector(x as unknown as AbiEvent)));
  for (const e of DRAW_EVENTS as readonly AbiEvent[]) assert.ok(onChain.has(toEventSelector(e)), `SweepVault emits ${e.name}`);
});

test("an empty draw's published list (no entries) is judged: it rebuilds to the sentinel alone; a list with holders does not", () => {
  const root = sentinelLeaf(0n, 500n);
  const d = foldDraws(VAULT, [
    ev("DrawAnnounced", { drawId: 0n, coin: COIN, seedRound: 1n }, 1, 1),
    ev("SnapshotFixed", { drawId: 0n, snapshotAt: 500n }, 2, 2),
    ev("DrawOpened", { drawId: 0n, coin: COIN, root, totalTickets: 0n, uri: "https://l.example/0.json", challengeEnds: 900n }, 2, 2, 1),
  ])[0];
  assert.equal(d.stage, "open");
  assert.deepEqual(judgeList(d, { version: 2, totalTickets: "0", entries: [] }).published, { status: "matches", why: null });
  const H = "0x00000000000000000000000000000000000000a1";
  const liar = judgeList(d, { entries: [{ holder: H, first: "0", end: "3" }] });
  assert.equal(liar.published!.status, "differs");
  assert.notEqual(ticketLeaf(H, 0n, 3n), root);
  assert.equal(judgeList(d, {}).published!.status, "unreadable", "a list with no entries array at all is not an empty list");
});
