// fixes-R3-carry (docs/audit/2026-09-26/review2/fixes-R3-carry.md): a draw whose snapshot had no eligible holder is opened as the
// sentinel alone (totalTickets 0) and closed by requestDraw with no winner (Empty). The monitor pages the empty open, recomputes it
// like any list (a false empty is a confirmed bad root: pause, then cancel), and pages the close. No network.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { encodeAbiParameters, encodeEventTopics } from "viem";
import { DRAW_STAGE, VAULT } from "../src/abi.mjs";
import { ticketLeaf, verdictN, weightedTree } from "../src/draws.mjs";
import { drawOutcome, judgeStackLogs } from "../src/evm.mjs";
import { stackOf } from "../src/stack.mjs";
import { hex } from "./fakechain.mjs";

const S = JSON.parse(readFileSync(new URL("../fixtures/stack-logs.json", import.meta.url), "utf8"));
const st = stackOf({ stack: S.stack }), c = st.collections[0];
const NOW = 1_790_000_000_000, ZERO = "0x0000000000000000000000000000000000000000", E18 = 10n ** 18n, T = 1_789_990_000n;
const lc = (a) => String(a).toLowerCase();
const log = (address, topics, data = "0x", i = 0) => ({ address, topics, data, blockNumber: hex(71_300_000 + i), transactionHash: `0x${(i + 1).toString(16).padStart(64, "0")}`,
  logIndex: hex(i), blockHash: `0x${(i + 9).toString(16).padStart(64, "0")}`, removed: false });
const EMPTY_ROOT = ticketLeaf(ZERO, 0n, T);

const opened = (id, root, total, i) => [
  log(c.raffle, encodeEventTopics({ abi: VAULT, eventName: "SnapshotFixed", args: { drawId: id } }), encodeAbiParameters([{ type: "uint64" }], [T]), i),
  log(c.raffle, encodeEventTopics({ abi: VAULT, eventName: "DrawOpened", args: { drawId: id, coin: S.coin } }),
    encodeAbiParameters([{ type: "bytes32" }, { type: "uint256" }, { type: "string" }, { type: "uint64" }], [root, total, "https://keeper.example/s.json", BigInt(NOW / 1000 + 900)]), i + 1)];
const closed = (id, i) => log(c.raffle, encodeEventTopics({ abi: VAULT, eventName: "DrawEmpty", args: { drawId: id, coin: S.coin } }), encodeAbiParameters([{ type: "uint64" }], [T]), i);

test("the stage numbers: Empty is 8, appended after Announced", () => {
  assert.equal(DRAW_STAGE[7], "Announced");
  assert.equal(DRAW_STAGE[8], "Empty");
});

test("an empty open pages SEV2 (the keeper says nobody held a whole coin) and is tracked for the recompute like any list", () => {
  const j = judgeStackLogs({ logs: opened(9n, EMPTY_ROOT, 0n, 1), ctx: { st, now: NOW } });
  const a = j.alerts.find((x) => x.key.startsWith("draw-empty-opened:"));
  assert.ok(a, j.alerts.map((x) => x.key).join(", "));
  assert.equal(a.sev, "SEV2");
  assert.match(a.text, /opened EMPTY: the keeper says NO eligible holder had a whole coin at the snapshot time 1789990000/);
  assert.match(a.text, /false empty.*pause the keeper, then cancel: the cancel carries the same snapshot time/s);
  const d = j.next.draws[`${lc(c.raffle)}:9`];
  assert.equal(d.status, "pending");
  assert.equal(d.totalTickets, "0");
  assert.equal(String(d.snapshotAt), String(T));
  // an ordinary open does not page it
  assert.ok(!judgeStackLogs({ logs: opened(10n, `0x${"ab".repeat(32)}`, 5n, 1), ctx: { st, now: NOW } }).alerts.some((x) => x.key.startsWith("draw-empty-opened:")));
});

test("the recompute of an empty draw: nobody eligible reproduces the sentinel alone (verified); holders at the snapshot make it a confirmed bad root (pause, then cancel)", () => {
  const X = "0x00000000000000000000000000000000000000c1", H = "0x00000000000000000000000000000000000000a1";
  const onChain = { root: EMPTY_ROOT, totalTickets: "0", snapshotAt: String(T) };
  // a true empty: half a coin and an excluded holder
  const t = weightedTree(new Map([[H, E18 / 2n], [X, 9n * E18]]), [X], { sentinel: { snapshotAt: T } });
  assert.equal(t.root, EMPTY_ROOT);
  assert.equal(t.totalTickets, 0n);
  const r = { block: 71_199_000, root: t.root, totalTickets: t.totalTickets, holders: t.holders };
  assert.equal(verdictN(onChain, [r, r], 2).v, "verified");
  // a false empty: H held three whole coins at the snapshot
  const f = weightedTree(new Map([[H, 3n * E18]]), [X], { sentinel: { snapshotAt: T } });
  const rf = { block: 71_199_000, root: f.root, totalTickets: f.totalTickets, holders: f.holders };
  const v = verdictN(onChain, [rf, rf], 2);
  assert.equal(v.v, "bad");
  const d = { vault: lc(c.raffle), id: "9", coin: lc(S.coin), slug: c.slug, root: EMPTY_ROOT, totalTickets: "0", snapshotAt: String(T), confirmed: true };
  const o = drawOutcome(`${lc(c.raffle)}:9`, d, v, [rf, rf], st);
  assert.equal(o.draw.status, "bad");
  assert.deepEqual(o.triggers.map((x) => x.action), ["pause", "cancel"], "pause first (no close while paused), then cancel (which carries the same time)");
});

test("DrawEmpty pages the close: SEV2 when verified, SEV1 when it closed unverified or as a confirmed bad root", () => {
  const id = `${lc(c.raffle)}:9`;
  const ok = judgeStackLogs({ logs: [closed(9n, 3)], ctx: { st, now: NOW }, prev: { draws: { [id]: { status: "verified" } } } });
  const a = ok.alerts.find((x) => x.key.startsWith("draw-empty:"));
  assert.equal(a.sev, "SEV2");
  assert.match(a.text, /CLOSED WITH NO WINNER: no eligible holder had a whole coin at its snapshot time 1789990000 \(the monitor reproduced the empty tree\)/);
  assert.equal(ok.next.draws[id].status, "closed-empty");
  const un = judgeStackLogs({ logs: [closed(9n, 3)], ctx: { st, now: NOW }, prev: { draws: { [id]: { status: "pending" } } } });
  const b = un.alerts.find((x) => x.key.startsWith("draw-empty:"));
  assert.equal(b.sev, "SEV1");
  assert.match(b.text, /NOT VERIFIED/);
  assert.equal(un.next.draws[id].status, "closed-empty-unverified");
  const bad = judgeStackLogs({ logs: [closed(9n, 3)], ctx: { st, now: NOW }, prev: { draws: { [id]: { status: "bad" } } } });
  const x = bad.alerts.find((y) => y.key.startsWith("draw-empty:"));
  assert.equal(x.sev, "SEV1");
  assert.match(x.text, /CONFIRMED BAD ROOT.*"bad draw" incident/s);
  assert.equal(bad.next.draws[id].status, "bad-closed-empty");
});
