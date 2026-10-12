import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";
import { test } from "node:test";
import type { RaffleDraw } from "../src/api/types.js";
import { DRAW_STAGE, RAFFLE_VAULT } from "../src/rook/abis.js";
import { NO_WINNER, drawSteps, emptyLine } from "../src/rook/keyless.js";

// fixes-R3-carry: a draw over a snapshot with no eligible holder opens with no tickets and closes with no winner (stage `empty`).
const A = (n: number) => `0x${n.toString(16).padStart(40, "0")}` as `0x${string}`;
const draw = (o: Partial<RaffleDraw>): RaffleDraw => ({ id: 3, vault: A(0x16), coin: A(0xc0), stage: "open", seedRound: 1, announcedAt: 1_000, snapshotAt: 900, root: `0x${"11".repeat(32)}`,
  totalTickets: "0", uri: "https://x/l.json", challengeEnds: 2_000, published: { status: "matches", why: null }, round: null, pinnedAt: null, winningTicket: null, resolvedAt: null,
  winner: null, claim: null, voidableAt: null, tx: "0x01", ...o });

test("an empty draw says so: open (it will close with no winner), empty (no winner this round; the wrapped token stays with the coin)", () => {
  assert.equal(emptyLine(draw({ stage: "empty" })), NO_WINNER);
  assert.match(NO_WINNER, /^No eligible holders at the snapshot — no winner this round\. The wrapped token stays with the coin for its next draw\.$/);
  assert.match(emptyLine(draw({}))!, /closes with no winner.*cancels it first if the list is wrong/s);
  assert.equal(emptyLine(draw({ totalTickets: "12" })), null);
  assert.equal(emptyLine(draw({ stage: "claimed", totalTickets: "12" })), null);
});

test("the keyless step on an open empty draw is `close` (the same requestDraw), gated like a pin; an empty draw offers nothing", () => {
  assert.deepEqual(drawSteps(draw({}), 1_999, false).map((s) => [s.id, s.ok]), [["close", false]]);
  assert.deepEqual(drawSteps(draw({}), 2_000, false).map((s) => [s.id, s.ok]), [["close", true]]);
  assert.match(drawSteps(draw({}), 2_000, true)[0].why!, /keeper is paused: no draw is closed/);
  assert.deepEqual(drawSteps(draw({ stage: "empty" }), 1e10, false), []);
});

test("the wire: Empty is stage 8, DrawEmpty is the compiled SweepVault's (skipped without out/)", () => {
  assert.equal(DRAW_STAGE[8], "empty");
  assert.equal(DRAW_STAGE[7], "announced");
  const ours = (RAFFLE_VAULT as readonly { type: string; name?: string }[]).find((x) => x.type === "event" && x.name === "DrawEmpty");
  assert.ok(ours);
  const f = new URL("../../out/SweepVault.sol/SweepVault.json", import.meta.url);
  if (!existsSync(f)) return;
  const compiled = (JSON.parse(readFileSync(f, "utf8")) as { abi: { type: string; name?: string; inputs?: { type: string; indexed?: boolean }[] }[] }).abi;
  const theirs = compiled.find((x) => x.type === "event" && x.name === "DrawEmpty");
  assert.ok(theirs, "SweepVault emits DrawEmpty");
  assert.deepEqual(theirs!.inputs!.map((i) => [i.type, !!i.indexed]), [["uint256", true], ["address", true], ["uint64", false]]);
});
