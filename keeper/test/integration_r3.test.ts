import assert from "node:assert/strict";
import { test } from "node:test";
import { mkdtempSync, readdirSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { encodeAbiParameters, keccak256, type Hex } from "viem";
import { generatePrivateKey, privateKeyToAccount } from "viem/accounts";
import { build, type Built } from "../service/main.js";
import { runPass } from "../service/keeper.js";
import { firstEmptyBlock } from "../service/draws.js";
import { A, COIN, CURVE, FakeChain, RAFFLE, randomnessOf, roundAt, sigOf, stackJson } from "./service-fake.js";

// docs/audit/2026-09-26/review2/integration-R3.md §2: SweepVault's snapshotTimeOf now hashes the draw's seed ORIGIN (review2 N-2,
// SeedCarried): a draw voided or cancelled before its pin leaves its seed round and snapshot window to the coin's next draw. The
// keeper reads the time on chain and never computes it from announcedAt. A new seed is announced only when every moment the snapshot
// time may fall on has a ticket; a carried empty time is opened empty and closes with no winner (fixes-R3-carry).
const E18 = 10n ** 18n;
const CHAIN = 9466301;
const H1 = A(0xa1), H2 = A(0xa2);

type World = { chain: FakeChain; env: Record<string, string>; lines: string[]; now: { ms: number } };
function world(): World {
  const key = generatePrivateKey(), address = privateKeyToAccount(key).address;
  const chain = new FakeChain({ keeper: address, chainId: CHAIN });
  const dir = mkdtempSync(join(tmpdir(), "keeper-r3-"));
  writeFileSync(join(dir, "stack.json"), stackJson(CHAIN));
  writeFileSync(join(dir, "floors.json"), JSON.stringify({ collections: { frogs: { priceWei: "158000000000000000", confidenceBps: 8_000, depthItems: 2 } } }));
  const env: Record<string, string> = { KEEPER_CHAIN_ID: String(CHAIN), KEEPER_STACK: join(dir, "stack.json"), KEEPER_STATE_DIR: join(dir, "state"),
    KEEPER_RPC_URLS: "http://127.0.0.1:1", KEEPER_FLOOR_SOURCE: "fixture", KEEPER_FLOOR_FIXTURE: join(dir, "floors.json"),
    KEEPER_DRAW_EVERY_S: "600", KEEPER_DRAW_OFFSET_S: "150", KEEPER_DRAW_WINDOW_S: "300", KEEPER_PUBLIC_URL: "https://keeper.example",
    KEEPER_MODE: "send", KEEPER_PRIVATE_KEY: key };
  chain.serveDir = join(dir, "state", "snapshots");
  chain.coins = [{ coin: COIN, policy: 0, curve: CURVE }];
  chain.units.set(COIN.toLowerCase(), E18);
  chain.transfers = [{ block: 500, from: A(0), to: H1, value: 50n * E18 }, { block: 600, from: A(0), to: H2, value: 25n * E18 },
    { block: 700, from: A(0), to: CURVE, value: 900n * E18 }];
  return { chain, env, lines: [], now: { ms: 1_790_000_000_000 } };
}
const built = (w: World): Promise<Built> =>
  build([], w.env, { fetchImpl: w.chain.fetch, write: (l) => w.lines.push(l), sleep: async () => { w.chain.advance(1); }, nowMs: () => w.now.ms });
const later = (w: World, s: number) => { w.chain.advance(s); w.now.ms += s * 1000; };
/** What snapshotTimeOf WOULD say if it were computed from this draw's own id and announcement (the pre-R3 rule): never used. */
const naive = (w: World, id: number, rnd: Hex) =>
  w.chain.draws[id].announcedAt - 86_400 + Number(BigInt(keccak256(encodeAbiParameters([{ type: "bytes32" }, { type: "address" }, { type: "uint256" }, { type: "string" }],
    [rnd, RAFFLE, BigInt(id), "snapshot"]))) % 86_400n);

test("firstEmptyBlock: the first block of the window with no ticket, or null when every moment has one", () => {
  const T = (block: number, from: string, to: string, v: bigint) => ({ blockNumber: BigInt(block), logIndex: 0, from, to, value: v });
  const logs = [T(10, A(0), H1, 2n * E18), T(20, H1, H2, E18 / 2n), T(30, H1, CURVE, 2n * E18 - E18 / 2n), T(40, A(0), H2, E18)];
  assert.equal(firstEmptyBlock(logs, 5, []), 5, "nobody at the window's start");
  const X = [CURVE.toLowerCase()];
  assert.equal(firstEmptyBlock(logs, 10, X), 30, "H1 sends its last whole coin to the (excluded) curve at 30; H2 has half until 40");
  assert.equal(firstEmptyBlock(logs, 30, X), 30, "the window starts on the empty block");
  assert.equal(firstEmptyBlock(logs, 40, X), null, "from 40, H2 holds 1.5");
  assert.equal(firstEmptyBlock(logs, 10, []), null, "the curve's 1.5 counts when it is not excluded");
  assert.equal(firstEmptyBlock([T(10, A(0), CURVE, 5n * E18)], 10, [CURVE.toLowerCase()]), 10);
});

test("a draw voided before its pin: the next announcement carries its seed; the keeper opens it at the ORIGIN's snapshot time, read on chain", async () => {
  const w = world();
  const b = await built(w);
  await runPass(b);
  assert.equal(w.chain.draws.length, 1);
  const seed0 = w.chain.draws[0].seedRound;
  // the keeper does not open it (no relay serves the round): the resolver voids it after 6 h, which carries its seed (N-2)
  w.chain.relayDown = new Set(["api.drand.sh", "api2.drand.sh", "drand.cloudflare.com"]);
  later(w, 6 * 3_600 + 200);
  let r = await runPass(b);
  assert.equal(w.chain.draws[0].stage, 6, "Voided");
  assert.equal(w.chain.carried.get(COIN.toLowerCase()), 1, "the coin carries draw 0's seed");
  // relays back: the next slot's announcement carries it, whatever the new announcement time
  w.chain.relayDown = new Set();
  later(w, 600);
  r = await runPass(b);
  assert.equal(w.chain.draws.length, 2, r.collections[0].draws.map((d) => d.plan).join("\n"));
  assert.equal(w.chain.seedOrigin(1), 0);
  assert.equal(w.chain.draws[1].seedRound, seed0, "the same seed round");
  if (w.chain.draws[1].stage === 7) r = await runPass(b);          // opened at once when the round is already public
  assert.equal(w.chain.draws[1].stage, 1, "Open");
  const rnd = randomnessOf(sigOf(seed0));
  const at = w.chain.draws[1].snapshotAt;
  assert.equal(at, w.chain.snapshotTimeOf(0, rnd), "the origin's snapshot time");
  assert.ok(at < w.chain.draws[0].announcedAt, "counted back from the origin's announcement");
  assert.notEqual(at, naive(w, 1, rnd), "not computed from the carrier's own id and announcedAt");
  const open = r.collections[0].draws.find((d) => d.action === "open" && d.drawId === "1")!;
  assert.match(open.plan, /carries draw 0's seed round and snapshot time/);
});

test("a young coin whose snapshot window has an empty moment is not announced (a snapshot there would be a round with no winner)", async () => {
  const w = world();
  // every whole coin was bought in the last hour: most of the 24 h window before now has no ticket
  w.chain.transfers = [{ block: w.chain.head - 3_000, from: A(0), to: H1, value: 50n * E18 }];
  const b = await built(w);
  const r = await runPass(b);
  assert.equal(w.chain.draws.length, 0);
  assert.match(r.collections[0].draws.find((d) => d.action === "announce")!.plan, /nobody held a whole coin at block \d+, inside the 24 h/);
  // a day later the whole window has tickets: announced
  later(w, 86_400);
  await runPass(b);
  assert.equal(w.chain.draws.length, 1);
});

test("a carried snapshot time with no ticket: announced, opened EMPTY (the sentinel alone), closed with no winner; the carry is consumed (fixes-R3-carry)", async () => {
  const w = world();
  // two days of chain; draw 0 was announced (and voided) at a time whose whole window precedes every transfer
  w.chain.head = 200_000;
  w.chain.transfers = w.chain.transfers.map((t) => ({ ...t, block: t.block + 100_000 }));
  const at0 = w.chain.tsOf(99_000);
  w.chain.draws.push({ stage: 6, coin: COIN.toLowerCase() as Hex, challengeEnds: 0, challengeEndsBlock: 0, round: 0n, pinnedAt: 0, pinnedAtBlock: 0, resolvedAt: 0,
    resolvedAtBlock: 0, root: `0x${"0".repeat(64)}`, totalTickets: 0n, winningTicket: 0n, winner: A(0), uri: "", seedRound: roundAt(at0) + 20n, announcedAt: at0,
    announcedAtBlock: w.chain.evm(99_000), snapshotAt: 0 });
  w.chain.carried.set(COIN.toLowerCase(), 1);
  const b = await built(w);
  let r = await runPass(b);
  assert.equal(w.chain.draws.length, 2, "announced: the carried time is fixed, and an empty one now has a way out");
  assert.match(r.collections[0].draws.find((d) => d.action === "announce")!.plan, /carries draw 0's snapshot time \d+, at which nobody held a whole coin: it will open empty and close with no winner/);
  assert.ok(!r.alerts.some((a) => /draw-carried-empty|draw-empty/.test(a)), r.alerts.join("\n"));
  assert.equal(w.chain.seedOrigin(1), 0);
  if (w.chain.draws[1].stage === 7) r = await runPass(b);
  const d1 = w.chain.draws[1];
  assert.equal(d1.stage, 1, "Open");
  assert.equal(d1.totalTickets, 0n, "no tickets");
  assert.equal(d1.snapshotAt, w.chain.snapshotTimeOf(0, randomnessOf(sigOf(w.chain.draws[0].seedRound))), "the origin's snapshot time");
  const open = r.collections[0].draws.find((d) => d.action === "open" && d.drawId === "1")!;
  assert.match(open.plan, /open draw 1 EMPTY: no eligible holder/);
  // the published list is the empty v2 tree: its root is the sentinel leaf (0, snapshotAt) and it has no entries
  const pub = JSON.parse(readFileSync(join(w.chain.serveDir!, readdirSync(w.chain.serveDir!).find((f) => f.endsWith(".json"))!), "utf8"));
  assert.equal(pub.version, 2);
  assert.equal(pub.entries.length, 0);
  assert.equal(pub.totalTickets, "0");
  assert.equal(pub.root.toLowerCase(), d1.root.toLowerCase());
  assert.deepEqual(pub.sentinel.proof, []);
  // after the challenge window the resolver's requestDraw closes it: Empty, the carry consumed, the unit back to the coin
  later(w, 1_000);
  r = await runPass(b);
  assert.equal(w.chain.draws[1].stage, 8, "Empty");
  assert.equal(w.chain.carried.get(COIN.toLowerCase()), undefined, "the carry is consumed");
  // the next slot: a fresh seed (the whole window now has tickets), never draw 0's again
  for (let i = 0; i < 12 && w.chain.draws.length < 3; i++) { later(w, 100); r = await runPass(b); }
  const next = w.chain.draws.at(-1)!;
  assert.equal(w.chain.draws.length, 3, JSON.stringify(r.collections[0].draws.map((d) => d.plan)) + r.alerts.join("\n"));
  assert.equal(w.chain.seedOrigin(2), 2, "its own seed");
  assert.notEqual(next.seedRound, w.chain.draws[0].seedRound);
});

test("a draw with its own seed whose snapshot turns out empty is opened empty, not left to be voided and carried (fixes-R3-carry)", async () => {
  const w = world();
  // announced without the window guard (an older keeper, or holders the logs did not show then): the coin's only transfers come
  // after the announcement, so nobody held a whole coin at any moment its snapshot time can fall on
  w.chain.head = 200_000;
  const at = w.chain.tsOf(w.chain.head - 60);
  w.chain.transfers = [{ block: w.chain.head - 30, from: A(0), to: H1, value: 50n * E18 }];
  w.chain.draws.push({ stage: 7, coin: COIN.toLowerCase() as Hex, challengeEnds: 0, challengeEndsBlock: 0, round: 0n, pinnedAt: 0, pinnedAtBlock: 0, resolvedAt: 0,
    resolvedAtBlock: 0, root: `0x${"0".repeat(64)}`, totalTickets: 0n, winningTicket: 0n, winner: A(0), uri: "", seedRound: roundAt(at) + 20n, announcedAt: at,
    announcedAtBlock: w.chain.evm(w.chain.head - 60), snapshotAt: 0 });
  w.chain.inFlight.set(COIN.toLowerCase(), 1);
  w.chain.units.set(COIN.toLowerCase(), 0n);
  const b = await built(w);
  let r = await runPass(b);
  for (let i = 0; i < 4 && w.chain.draws[0].stage === 7; i++) { later(w, 30); r = await runPass(b); }
  assert.equal(w.chain.draws[0].stage, 1, JSON.stringify(r.collections[0].draws.map((d) => d.plan)));
  assert.equal(w.chain.draws[0].totalTickets, 0n);
  assert.match(r.collections[0].draws.find((d) => d.action === "open")!.plan, /open draw 0 EMPTY/);
  assert.ok(!r.alerts.some((a) => /draw-empty/.test(a)), r.alerts.join("\n"));
  later(w, 1_000);
  await runPass(b);
  assert.equal(w.chain.draws[0].stage, 8, "Empty: closed with no winner");
  assert.equal(w.chain.carried.get(COIN.toLowerCase()), undefined, "nothing carried");
  assert.equal(w.chain.units.get(COIN.toLowerCase()), E18, "the unit is the coin's again");
});

test("the first day of a stack: the window reaches before the stack's first block, so nothing is announced, and the pass goes on", async () => {
  const w = world();
  w.chain.head = 20_000;                                           // 20,000 s of chain: less than one snapshot window
  const b = await built(w);
  const r = await runPass(b);
  assert.equal(w.chain.draws.length, 0);
  assert.match(r.collections[0].draws.find((d) => d.action === "announce")!.plan, /before the stack's first block/);
  assert.ok(!r.alerts.some((a) => /^draws: /.test(a)), r.alerts.join("\n"));
});
