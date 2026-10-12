// Second review (R2), 2026-09-26, turned into regressions by round 3 (docs/audit/2026-09-26/review2/fixes-R3-services.md). The R2
// PoCs asserted the gaps (a pass meant the gap was there); each test here asserts the attack now FAILS. No network.
import assert from "node:assert/strict";
import { mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { encodeAbiParameters, encodeEventTopics } from "viem";
import { MODULE, retryPolicy } from "../src/abi.mjs";
import { checkChain } from "../src/chain.mjs";
import { loadConfig } from "../src/config.mjs";
import { weightedTree } from "../src/draws.mjs";
import { checkEvm, judgeStackLogs } from "../src/evm.mjs";
import { HolderIndex, LAUNCHED, TRANSFER } from "../src/holders.mjs";
import { checkLiveness } from "../src/liveness.mjs";
import { EMPTY_RECORDS } from "../src/records.mjs";
import { stackOf } from "../src/stack.mjs";
import { fakeChain, hex } from "./fakechain.mjs";

retryPolicy.baseMs = 0;
const S = JSON.parse(readFileSync(new URL("../fixtures/stack-logs.json", import.meta.url), "utf8"));
const st = stackOf({ stack: S.stack }), c = st.collections[0];
const E18 = 10n ** 18n, NOW = 1_790_000_000_000, ZERO = "0x0000000000000000000000000000000000000000";
const evmCfg = { rpcs: ["https://a.example", "https://b.example"], stack: S.stack, from_block: 71_200_000, confirmations: 20 };
const guardianSpy = () => { const done = []; return { done, address: S.guardian, autoPause: true, autoCancel: true,
  pauseKeeper: async () => { done.push("pause"); return { done: true, tx: "0x" + "7".repeat(64) }; },
  cancelDraw: async (v, id) => { done.push(`cancel ${v} ${id}`); return { done: true, tx: "0x" + "8".repeat(64) }; } }; };
const ROOT = `0x${"ab".repeat(32)}`, FORGED = `0x${"cd".repeat(32)}`;
const drawView = (x = {}) => ({ stage: 1, coin: S.coin, challengeEnds: BigInt(NOW / 1000 + 600), challengeEndsBlock: 0n, round: 0n, pinnedAt: 0n, pinnedAtBlock: 0n,
  resolvedAt: 0n, resolvedAtBlock: 0n, root: ROOT, totalTickets: 1000n, winningTicket: 0n, winner: ZERO, uri: "https://mirror.example/s.json", seedRound: 0n,
  announcedAt: 0n, announcedAtBlock: 0n, snapshotAt: 1_789_990_000n, ...x });

/** A 4663 where the RAFFLE vault's state holds `draws` (read with eth_call), and eth_getLogs is answered by `getLogs(url, params)`. */
function world({ draws = [], getLogs = () => [], headNum = 71_200_200 } = {}) {
  return fakeChain({
    views: (url, to, fn, args) => {
      if (fn === "minConfidenceBps") return 3000n;
      if (fn === "accepted") return 65n * E18 / 100n;
      if (fn === "routerOf") return "0x00000000000000000000000000000000000000c2";
      if (fn === "drawCount") return to === c.raffle ? BigInt(draws.length) : 0n;
      if (fn === "draw") return drawView(draws[Number(args[0])]);
      throw new Error(`no view ${fn}`);
    },
    other: (url, method, params) => {
      if (method === "eth_blockNumber") return hex(headNum);
      if (method === "eth_getLogs") return getLogs(url, params);
      if (method === "eth_getBlockByNumber") return { timestamp: hex(NOW / 1000 - 3600) };
      throw new Error(`unexpected ${method}`);
    },
  });
}
const busy = () => { throw new Error("the network is busy, please try again in a moment"); };

test("R2-M1a inverted: RPC B answering -32005 to EVERY eth_getLogs no longer blinds the draw check: the draw is read from the vault's state, recomputed, and the guardian pauses then cancels", async () => {
  const { http } = world({ draws: [{}], getLogs: (url) => (url.includes("b.example") ? busy() : []) });
  const g = guardianSpy();
  const bad = async () => ({ root: FORGED, totalTickets: 1000n, block: 71_199_000, holders: 3 });
  const r = await checkEvm({ cfg: evmCfg, prev: {}, now: NOW, http, readRecords: async () => EMPTY_RECORDS, timelock: "tl", guardian: g, recompute: bad });
  assert.deepEqual(g.done, ["pause", `cancel ${c.raffle} 0`], "found without a single log from B, and acted on");
  assert.equal(r.next.draws[`${c.raffle}:0`].status, "bad");
  assert.ok(r.alerts.some((a) => a.sev === "SEV1" && /A BAD DRAW ROOT/.test(a.text)));
  assert.match(r.next.unread, /busy/, "the canary's own range stays unread (its cursor waits), and the stall is paged after 30 min as before");
});

test("R2-M1b inverted: the holder index reads the history once, then only the new blocks: 60 days in, a poll is one eth_getLogs, and a root needs none", async () => {
  const from = 71_200_000, day = 858_000, factory = c.factory, coin = "0x00000000000000000000000000000000000c0de1", curve = "0x00000000000000000000000000000000000c0de2";
  const H1 = "0x" + "1".repeat(40), H2 = "0x" + "2".repeat(40);
  const pad = (a) => `0x${a.slice(2).padStart(64, "0")}`;
  const events = [
    { address: factory, blockNumber: from + 10, topics: [LAUNCHED, pad(coin), pad(curve), pad(H1)], data: encodeAbiParameters([{ type: "uint8" }, { type: "uint256" }, { type: "bool" }, { type: "bool" }, { type: "bool" }], [0, 0n, false, false, false]) },
    { address: coin, blockNumber: from + 11, topics: [TRANSFER, pad(ZERO), pad(curve)], data: pad(hex(1000n * E18)) },
    { address: coin, blockNumber: from + 30 * day, topics: [TRANSFER, pad(curve), pad(H1)], data: pad(hex(300n * E18)) },
    { address: coin, blockNumber: from + 59 * day, topics: [TRANSFER, pad(curve), pad(H2)], data: pad(hex(50n * E18)) },
  ].map((l, i) => ({ ...l, blockNumber: hex(l.blockNumber), logIndex: hex(i), removed: false }));
  let head = from + 60 * day, getLogs = 0;
  const { http } = fakeChain({ other: (url, method, params) => {
    if (method === "eth_blockNumber") return hex(head);
    if (method === "eth_getBlockByNumber") { const n = Number(BigInt(params[0])); return { timestamp: hex(1_700_000_000 + Math.floor((n - from) / 10)) }; }
    if (method === "eth_getLogs") {
      getLogs++;
      const q = params[0], a = Number(BigInt(q.fromBlock)), b = Number(BigInt(q.toBlock)), addrs = [].concat(q.address).map((x) => x.toLowerCase());
      return events.filter((l) => addrs.includes(l.address.toLowerCase()) && Number(BigInt(l.blockNumber)) >= a && Number(BigInt(l.blockNumber)) <= b);
    }
    throw new Error(method);
  } });
  const ix = new HolderIndex({ url: "https://a.example", span: 2_000_000, factories: [factory], fromBlock: from, confirmations: 20, http });
  const first = await ix.update({ deadline: Date.now() + 60_000 });
  assert.equal(first.group, head - 20, "caught up");
  assert.ok(ix.has(coin), "the RAFFLE coin was discovered from Launched");
  const snapshotAt = 1_700_000_000 + Math.floor((59 * day + 100) / 10);
  getLogs = 0;
  const root = await ix.rootAt({ coin, snapshotAt: BigInt(snapshotAt), fromBlock: from, head: head - 20, exclusions: [ZERO, curve] });
  assert.equal(getLogs, 0, "a root is a local replay");
  const want = weightedTree(new Map([[H1, 300n * E18], [H2, 50n * E18]]), [], { sentinel: { snapshotAt } });
  assert.deepEqual([root.root, root.totalTickets], [want.root, 350n]);
  head += 600;
  await ix.update({ deadline: Date.now() + 60_000 });
  assert.equal(getLogs, 1, "one page for the new 600 blocks (the factory and every caught-up coin together), not 5,000 from the stack's start");
});

test("R2-M1c inverted: the monitor starts from the stack file's L2 fromBlock (as the keeper does), and a configured start later than it is refused", () => {
  const s = stackOf({ stack: { ...S.stack, fromBlock: 72_000_000, deployedAtBlock: 26_059_685 } });
  assert.equal(s.fromBlock, 72_000_000);
  assert.equal(stackOf({ stack: { ...S.stack, fromBlock: 72_000_000 }, from_block: 71_000_000 }).fromBlock, 71_000_000, "an earlier start is only slower");
  const dir = mkdtempSync(join(tmpdir(), "r3-cfg-"));
  const base = JSON.parse(readFileSync(new URL("../config.example.json", import.meta.url), "utf8"));
  const cfg = { ...base, timelock: "rrkah-fqaaa-aaaaa-aaaaq-cai", buyer: "ryjl3-tyaaa-aaaaa-aaaba-cai", builds: [{ commit: "x", buyer: "0".repeat(64), timelock: "0".repeat(64) }], builds_file: undefined,
    install_arg_hex: undefined, bitcoin: undefined, liveness: [], evm: { ...base.evm, stack: { ...S.stack, fromBlock: 72_000_000 }, from_block: 72_100_000, guardian: undefined } };
  writeFileSync(join(dir, "m.json"), JSON.stringify(cfg));
  assert.throws(() => loadConfig({ path: join(dir, "m.json") }), /later than the stack file's fromBlock/);
});

test("R2-L2: the draw exclusions are the published tools/snapshot/exclusions.json; a hand copy that drifted is refused at start", () => {
  const dir = mkdtempSync(join(tmpdir(), "r3-cfg-"));
  const base = JSON.parse(readFileSync(new URL("../config.example.json", import.meta.url), "utf8"));
  const mk = (draws) => ({ ...base, timelock: "rrkah-fqaaa-aaaaa-aaaaq-cai", buyer: "ryjl3-tyaaa-aaaaa-aaaba-cai", builds: [{ commit: "x", buyer: "0".repeat(64), timelock: "0".repeat(64) }],
    builds_file: undefined, install_arg_hex: undefined, bitcoin: undefined, liveness: [], draws, evm: { ...base.evm, stack: S.stack, guardian: undefined } });
  writeFileSync(join(dir, "ok.json"), JSON.stringify(mk({ providers: base.draws.providers })));
  const ok = loadConfig({ path: join(dir, "ok.json") });
  const file = JSON.parse(readFileSync(new URL("../../tools/snapshot/exclusions.json", import.meta.url), "utf8"));
  assert.deepEqual(ok.draws.exclusions, file.map((x) => x.address));
  assert.match(ok.draws.exclusions_sha256, /^[0-9a-f]{64}$/);
  writeFileSync(join(dir, "drift.json"), JSON.stringify(mk({ providers: base.draws.providers, exclusions: file.slice(1).map((x) => x.address) })));
  assert.throws(() => loadConfig({ path: join(dir, "drift.json") }), /draws\.exclusions differs from/);
  writeFileSync(join(dir, "same.json"), JSON.stringify(mk({ providers: base.draws.providers, exclusions: file.map((x) => x.address.toUpperCase().replace("0X", "0x")) })));
  assert.doesNotThrow(() => loadConfig({ path: join(dir, "same.json") }), "the same list, any case, is accepted");
});

test("R2-M2 inverted: two draws that can never be recomputed do not starve a third: the forged one is recomputed the same poll and cancelled; the stuck ones page SEV1 at their deadline", async () => {
  const soon = BigInt(NOW / 1000 + 300);                         // the stuck draws' windows end first (they come first in the queue)
  const other = (n) => `0x00000000000000000000000000000000000c0de${n}`;
  const { http } = world({ draws: [{ coin: other(7), challengeEnds: soon }, { coin: other(8), challengeEnds: soon }, {}] });
  const g = guardianSpy();
  const seen = [];
  const rec = async (url, input) => { seen.push(input.coin); if (input.coin !== S.coin) throw new Error("the network is busy"); return { root: FORGED, totalTickets: 1000n, block: 71_199_000, holders: 3 }; };
  const r = await checkEvm({ cfg: evmCfg, prev: {}, now: NOW, http, readRecords: async () => EMPTY_RECORDS, timelock: "tl", guardian: g, recompute: rec });
  assert.ok(seen.includes(S.coin), "the forged draw 2 was recomputed this poll");
  assert.equal(r.next.draws[`${c.raffle}:2`].status, "bad");
  assert.ok(g.done.includes(`cancel ${c.raffle} 2`));
  for (const id of [0, 1]) assert.ok(r.alerts.some((a) => a.sev === "SEV1" && a.key === `draw-deadline:${c.raffle}:${id}`), `stuck draw ${id} is paged SEV1 before its pin`);
  assert.equal(g.done[0], "pause", "and the keeper is paused first, so none of them can be pinned");
});

// FloorPosted(target, accepted, moveBps, confidenceBps, depthItems) as the module emits it
const floorLog = (i, target, accepted, move = 500n, conf = 8000) => ({ address: c.module, topics: encodeEventTopics({ abi: MODULE, eventName: "FloorPosted" }),
  data: encodeAbiParameters([{ type: "uint256" }, { type: "uint256" }, { type: "uint256" }, { type: "uint16" }, { type: "uint32" }], [target, accepted, move, conf, 12]),
  blockNumber: hex(71_300_000 + i * 36_000), transactionHash: `0x${(i + 1).toString(16).padStart(64, "0")}`, logIndex: "0x0", blockHash: `0x${(i + 7).toString(16).padStart(64, "0")}`, removed: false });

test("R2-M3 inverted: a stolen key posting AT floorBounds() (target == accepted, never clamped) is paged SEV1 within three posts, and the day's move is paged; the guardian pauses only when told to", () => {
  const walk = (pauseOnFloorWalk) => {
    let f = 65n * E18 / 100n, prev = {};
    const start = f, alerts = [], triggers = [];
    for (let i = 0; i < 24; i++) {
      const next = f * 95n / 100n > start * 80n / 100n ? f * 95n / 100n : start * 80n / 100n;
      const j = judgeStackLogs({ logs: [floorLog(i, next, next)], prev, ctx: { st, now: NOW + i * 3600_000, minConf: { [c.module]: 3000n }, times: {}, pauseOnFloorWalk } });
      alerts.push(...j.alerts.map((a) => ({ ...a, i }))); triggers.push(...j.triggers); prev = j.next; f = next;
    }
    return { alerts, triggers };
  };
  const { alerts, triggers } = walk(false);
  const edge = alerts.find((a) => a.key.startsWith("floor-walk:"));
  assert.ok(edge, "an edge run is paged");
  assert.equal(edge.sev, "SEV1");
  assert.equal(edge.i, 3, "on the third edge post in a row (the first post has nothing to compare with)");
  assert.match(edge.text, /BEING WALKED DOWN AT THE BAND'S EDGE/);
  // the monitor first sees the floor after the first step: over the day it sees 0.95 → 0.80 of the start, a 15.8 % move
  assert.ok(alerts.some((a) => a.key.startsWith("floor-day-move:") && a.sev === "SEV2" && /moved 15\.8% DOWN/.test(a.text)), "the day's move is paged too");
  assert.deepEqual(triggers, [], "pause_on_floor_walk off (default): pages only");
  assert.ok(walk(true).triggers.some((t) => t.action === "pause" && /walked down at the band's edge/.test(t.why)));
  // an honest keeper's ordinary moves (±1 % an hour) page nothing
  let f = 65n * E18 / 100n, prev = {}, n = 0;
  for (let i = 0; i < 24; i++) { f = i % 2 ? f * 101n / 100n : f * 99n / 100n; const j = judgeStackLogs({ logs: [floorLog(i, f, f)], prev, ctx: { st, now: NOW + i * 3600_000, minConf: { [c.module]: 3000n }, times: {} } }); n += j.alerts.length; prev = j.next; }
  assert.equal(n, 0);
});

test("R2-S1 (monitor side): a draw Resolved for more than an hour and never claimed is paged", async () => {
  const draw = { stage: 3, resolvedAt: BigInt(NOW / 1000 - 2 * 3600), winningTicket: 7n };
  const views = (url, to, fn, args) => {
    if (fn === "drawCount") return to === c.raffle ? 1n : 0n;
    if (fn === "draw") return drawView(draw);
    return { keeperPaused: false, fresh: true, escaped: false, postedAt: BigInt(NOW / 1000 - 600), lastGoodAt: BigInt(NOW / 1000 - 600), capFresh: true, nextToResolve: 0n, ticketCount: 0n }[fn];
  };
  const r = await checkChain({ cfg: { rpcs: ["https://a.example", "https://b.example"], stack: S.stack }, prev: {}, now: NOW, http: fakeChain({ views }).http });
  const a = r.alerts.find((x) => x.key.startsWith(`draw-resolved-unclaimed:${c.raffle}:0`));
  assert.ok(a, r.alerts.map((x) => x.key).join(","));
  assert.match(a.text, /RESOLVED 2\.0 h ago .* nobody has claimed it/);
  assert.equal(r.next.drawFrom[c.raffle], 0, "a resolved draw is not final: it stays in the scan");
});

test("R2-S8 (monitor side): the keeper's 503 /healthz with alert kinds is paged with the kinds, never the texts", async () => {
  const body = JSON.stringify({ ok: false, refusals: 0, alerts: 2, alertKinds: ["tx-stuck", "claim-stuck"], alertPasses: 3 });
  const http = async () => ({ status: 503, text: async () => body });
  const lcfg = { liveness: [{ name: "keeper", url: "https://keeper.example/healthz" }] };
  const a = await checkLiveness({ cfg: lcfg, prev: {}, now: NOW, http });
  const b = await checkLiveness({ cfg: lcfg, prev: a.next, now: NOW + 11 * 60_000, http });
  assert.match(b.alerts[0].text, /HTTP 503, alerts: tx-stuck, claim-stuck/);
  const odd = await checkLiveness({ cfg: lcfg, prev: a.next, now: NOW + 11 * 60_000, http: async () => ({ status: 503, text: async () => JSON.stringify({ alertKinds: ["x: <b>secret text</b>"] }) }) });
  assert.match(odd.alerts[0].text, /alerts: x\)/, "only the kind before the colon, sanitised");
});
