import assert from "node:assert/strict";
import { test } from "node:test";
import { mkdtempSync, readFileSync, writeFileSync, existsSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { decodeFunctionData, encodeFunctionData, parseTransaction, type Hex } from "viem";
import { generatePrivateKey, privateKeyToAccount } from "viem/accounts";
import { build, statusOf, type Built } from "../service/main.js";
import { runPass, type PassReport } from "../service/keeper.js";
import { MODULE, VAULT } from "../service/abis.js";
import { Refusal } from "../service/rpc.js";
import { A, BURN, FakeChain, MOD, RAFFLE, REG, YOYO, stackJson } from "./service-fake.js";

// docs/v6/p1-services.md §1: the production keeper against a fake node — what it posts, what it refuses, what it remembers.
const E18 = 10n ** 18n;
const FLOOR = 158_000_000_000_000_000n;          // 0.158 ETH, the measured frog floor (docs/v5/floor-feed.md §9)

type World = { chain: FakeChain; dir: string; env: Record<string, string>; lines: string[]; key: Hex; address: Hex; now: { ms: number } };
function world(o: { send?: boolean; chainId?: number; confidence?: number; extra?: Record<string, string> } = {}): World {
  const key = generatePrivateKey(), address = privateKeyToAccount(key).address;
  const chain = new FakeChain({ keeper: address, chainId: o.chainId ?? 4663 });
  const dir = mkdtempSync(join(tmpdir(), "keeper-svc-"));
  writeFileSync(join(dir, "stack.json"), stackJson(o.chainId ?? 4663));
  writeFileSync(join(dir, "floors.json"), JSON.stringify({ collections: { frogs: { priceWei: String(FLOOR), confidenceBps: o.confidence ?? 8_000, depthItems: 2 } } }));
  const env: Record<string, string> = { KEEPER_CHAIN_ID: String(o.chainId ?? 9466301), KEEPER_STACK: join(dir, "stack.json"), KEEPER_STATE_DIR: join(dir, "state"),
    KEEPER_RPC_URLS: "http://127.0.0.1:1", KEEPER_FLOOR_SOURCE: "fixture", KEEPER_FLOOR_FIXTURE: join(dir, "floors.json"), ...(o.extra ?? {}) };
  if (o.send) { env.KEEPER_MODE = "send"; env.KEEPER_PRIVATE_KEY = key; }
  if ((o.chainId ?? 4663) !== 9466301 && !o.extra?.KEEPER_CHAIN_ID) env.KEEPER_CHAIN_ID = String(o.chainId ?? 4663);
  // on 4663 a fixture may not be sent: those tests run on the fork's id; everything else is chain-agnostic
  return { chain, dir, env, lines: [], key, address, now: { ms: 1_790_000_000_000 } };
}
const chainAt = (w: World, id: number) => { w.chain.chainId = id; w.env.KEEPER_CHAIN_ID = String(id); writeFileSync(join(w.dir, "stack.json"), stackJson(id)); };
async function built(w: World, argv: string[] = []): Promise<Built> {
  return build(argv, w.env, { fetchImpl: w.chain.fetch, write: (l) => w.lines.push(l), sleep: async () => { w.chain.advance(1); }, nowMs: () => w.now.ms });
}
const sends = (w: World) => w.chain.sent.map((raw) => { const t = parseTransaction(raw); return { to: t.to!.toLowerCase(), fn: (() => { try { return decodeFunctionData({ abi: [...MODULE, ...VAULT] as never, data: t.data! }).functionName; } catch { return "?"; } })() }; });

test("a dry run simulates the floor and both caps from the agreed floor, signs nothing, and never calls eth_sendRawTransaction", async () => {
  const w = world(); chainAt(w, 9466301);
  const b = await built(w, ["--dry-run"]);
  assert.equal(b.signer, null, "a dry run never reads the key");
  const r = await runPass(b);
  assert.deepEqual(r.refusals, []);
  const c = r.collections[0];
  assert.equal(c.floor.plan?.type, "post");
  assert.equal(c.floor.result?.outcome, "simulated");
  assert.deepEqual(c.caps.map((x) => [x.policy, x.plan.type, x.result?.outcome]), [["raffle", "post", "simulated"], ["burn", "post", "simulated"]]);
  assert.equal((c.caps[0].plan as { capWei: string }).capWei, String(FLOOR + FLOOR / 10n), "cap = floor + 10%");
  assert.ok(!w.chain.asked.includes("eth_sendRawTransaction"));
  assert.equal(w.chain.sent.length, 0);
  assert.equal(w.chain.accepted, 0n, "nothing moved");
});

test("send: postFloor, then postCap on RAFFLE and BURN — never YOYO; a second pass and a restart keep, and send nothing", async () => {
  const w = world({ send: true }); chainAt(w, 9466301);
  const b = await built(w);
  const r1 = await runPass(b);
  assert.deepEqual(sends(w).map((s) => [s.to, s.fn]), [[MOD.toLowerCase(), "postFloor"], [RAFFLE.toLowerCase(), "postCap"], [BURN.toLowerCase(), "postCap"]]);
  assert.equal(w.chain.accepted, FLOOR);
  assert.ok(w.chain.fresh());
  assert.equal(w.chain.vaults[RAFFLE.toLowerCase()].cap, FLOOR + FLOOR / 10n);
  assert.equal(w.chain.vaults[BURN.toLowerCase()].cap, FLOOR + FLOOR / 10n);
  assert.equal(w.chain.vaults[YOYO.toLowerCase()].cap, 0n, "the YOYO vault is never posted to");
  assert.ok(r1.collections[0].caps.every((c) => c.result?.outcome === "mined"));
  const r2 = await runPass(b);
  assert.equal(r2.collections[0].floor.plan?.type, "keep");
  assert.ok(r2.collections[0].caps.every((c) => c.plan.type === "keep"));
  assert.equal(w.chain.sent.length, 3, "the second pass sent nothing");
  // a restart: a new process over the same state directory and the same chain
  const again = await built(w);
  await runPass(again);
  assert.equal(w.chain.sent.length, 3, "a restart re-derives the same plan and sends nothing");
  // 31 minutes on: the floor is refreshed (every 30 minutes, floor-module.md §5); the caps still have more than 15 minutes
  w.chain.advance(31 * 60);
  const r3 = await runPass(again);
  assert.equal(r3.collections[0].floor.plan?.type, "post");
  assert.match(r3.collections[0].floor.plan!.why, /refreshed/);
  assert.deepEqual(sends(w).slice(3).map((s) => s.fn), ["postFloor"]);
  // 47 minutes after the caps: under 15 minutes left, both are refreshed
  w.chain.advance(16 * 60);
  await runPass(again);
  assert.deepEqual(sends(w).slice(4).map((s) => s.fn).sort(), ["postCap", "postCap"]);
});

test("the floor is posted only from an agreed feed: no fixture row, or confidence under the module's minimum, posts nothing — and then no cap either", async () => {
  const w = world({ send: true, confidence: 2_999 }); chainAt(w, 9466301);
  const r = await runPass(await built(w));
  assert.equal(r.collections[0].floor.plan?.type, "skip");
  assert.match(r.collections[0].floor.plan!.why, /confidence 2999/);
  assert.ok(r.collections[0].caps.every((c) => c.plan.type === "skip" && /no fresh floor/.test(c.plan.why)));
  assert.equal(w.chain.sent.length, 0);
  // the module's own minimum binds when it is higher than the keeper's
  const v = world({ send: true, confidence: 4_000 }); chainAt(v, 9466301);
  v.chain.minConfidenceBps = 5_000;
  const r2 = await runPass(await built(v));
  assert.equal(r2.collections[0].floor.plan?.type, "skip");
  assert.equal(v.chain.sent.length, 0);
});

test("refusals every pass: a key that is not registry.keeper(), or not the module's keeper, or that has code (EIP-7702) — nothing is sent", async () => {
  const w = world({ send: true }); chainAt(w, 9466301);
  const b = await built(w);
  w.chain.keeper = A(0x777);
  let r = await runPass(b);
  assert.match(r.refusals.join("\n"), /not registry\.keeper\(\)/);
  w.chain.keeper = w.address; w.chain.moduleKeeper = A(0x778);
  r = await runPass(b);
  assert.match(r.refusals.join("\n"), /not the module's keeper/);
  w.chain.moduleKeeper = null; w.chain.keeperCode = "0xef0100" + "11".repeat(20);
  r = await runPass(b);
  assert.match(r.refusals.join("\n"), /has code/);
  w.chain.vaults[BURN.toLowerCase()].policy = 2;   // a stack that names the YOYO vault as its BURN vault
  w.chain.keeperCode = "0x";
  r = await runPass(b);
  assert.match(r.refusals.join("\n"), /never posts to a YOYO vault/);
  assert.equal(w.chain.sent.length, 0);
  assert.equal(statusOf(b, { startedAt: "", passes: 1, lastPassAt: new Date().toISOString(), lastPassMs: 1, consecutiveFailures: 0, lastError: null, last: r }).refusals.length, 1);
});

test("the chain id: an RPC that reports another is not used (none left: refused at start); one that changes before a send has that send refused", async () => {
  const w = world({ send: true }); chainAt(w, 9466301);
  w.chain.chainId = 4663;
  await assert.rejects(built(w), (e: unknown) => e instanceof Refusal && /no RPC reports chain id 9466301/.test((e as Error).message));
  w.chain.chainId = 9466301;
  const b = await built(w);
  w.chain.chainId = 1;                              // the node is repointed between passes: the pass is refused before any read
  await assert.rejects(runPass(b), /no RPC reports chain id 9466301/);
  // repointed in the middle of a pass, after the simulation and the signature: the bytes are refused before they leave
  w.chain.chainId = 9466301;
  await b.rpc.init();
  const realSend = w.chain.handle.bind(w.chain);
  w.chain.handle = async (m, p) => { if (m === "eth_getTransactionCount") w.chain.chainId = 1; return realSend(m, p); };
  const r = await b.sender.submit({ key: "floor", what: "postFloor", to: MOD, data: encodeFunctionData({ abi: MODULE, functionName: "postFloor", args: [FLOOR, 8_000, 2] }) });
  assert.equal(r.outcome, "refused");
  assert.match(r.detail, /now reports chain id 1, not 9466301/);
  assert.equal(w.chain.sent.length, 0);
  assert.equal(b.store.get().pending, null, "nothing is in flight: the bytes never left");
});

test("gas caps: per-transaction gas, the fee (refused, never lowered), the per-transaction worst case, and the day's budget (which resets at the UTC day)", async () => {
  const w = world({ send: true }); chainAt(w, 9466301);
  let b = await built(w);
  w.chain.estimate = 400_000n;                      // + 25% = 500,000 > 400,000
  let r = await runPass(b);
  assert.equal(r.collections[0].floor.result?.outcome, "refused");
  assert.match(r.collections[0].floor.result!.detail, /over the per-transaction cap of 400000/);
  w.chain.estimate = 100_000n; w.chain.baseFee = 600_000_000n;       // 2 × 0.6 gwei > 1 gwei
  r = await runPass(b);
  assert.match(r.collections[0].floor.result!.detail, /max fee .* over the cap/);
  w.chain.baseFee = 10_000_000n;
  w.env.KEEPER_MAX_WEI_PER_TX = "1000000000"; w.env.KEEPER_MAX_WEI_PER_DAY = "1000000000";
  b = await built(w);
  r = await runPass(b);
  assert.match(r.collections[0].floor.result!.detail, /per-transaction cap of 1000000000/);
  // the day: each transaction reserves its worst case (125,000 gas × 0.02 gwei = 2.5e12) and costs 6e11 once mined, so a
  // 3.5e12 day takes two and refuses the third (1.2e12 spent + 2.5e12 reserved)
  w.env.KEEPER_MAX_WEI_PER_TX = "3000000000000"; w.env.KEEPER_MAX_WEI_PER_DAY = "3500000000000";
  b = await built(w);
  r = await runPass(b);
  assert.deepEqual([r.collections[0].floor.result?.outcome, ...r.collections[0].caps.map((c) => c.result?.outcome)], ["mined", "mined", "refused"]);
  assert.match(r.collections[0].caps[1].result!.detail, /over the day's cap/);
  w.now.ms += 86_400_000;                           // the next UTC day
  r = await runPass(b);
  assert.equal(r.collections[0].caps[1].result?.outcome, "mined");
});

test("one transaction in flight: no receipt yet → the pass stops; the next pass re-broadcasts the SAME bytes; mined → recorded; a used nonce with no receipt → dropped", async () => {
  const w = world({ send: true, extra: { KEEPER_RECEIPT_WAIT_S: "5" } }); chainAt(w, 9466301);
  w.chain.mine = false;
  const b = await built(w);
  let r = await runPass(b);
  assert.equal(r.collections[0].floor.result?.outcome, "pending");
  assert.equal(r.collections[0].caps[0].result, null, "nothing else is sent while one is in flight");
  const first = w.chain.sent[0];
  assert.equal(w.chain.sent.length, 1);
  const pending = JSON.parse(readFileSync(join(w.dir, "state", "keeper-state.json"), "utf8")).pending;
  assert.equal(pending.raw, first, "the bytes were written down before they were sent");
  // a restart while it is pending: the same bytes, never a new signature for that nonce
  const b2 = await built(w);
  r = await runPass(b2);
  assert.equal(r.pending?.outcome, "pending");
  assert.equal(w.chain.sent.length, 1, "re-broadcast of the same bytes is 'already known', not a second transaction");
  assert.ok(w.chain.asked.filter((m) => m === "eth_sendRawTransaction").length >= 2);
  // it is mined: the next pass records it and carries on
  w.chain.mineTx([...w.chain.txs.values()][0]);
  w.chain.mine = true;
  r = await runPass(b2);
  assert.equal(r.pending?.outcome, "mined");
  assert.equal(JSON.parse(readFileSync(join(w.dir, "state", "keeper-state.json"), "utf8")).history[0].outcome, "mined");
  // dropped: a transaction in flight whose nonce the chain has used without it
  const v = world({ send: true, extra: { KEEPER_RECEIPT_WAIT_S: "5" } }); chainAt(v, 9466301);
  v.chain.mine = false;
  const c = await built(v);
  await runPass(c);
  v.chain.txs.clear(); v.chain.nonce = 1;           // something else took nonce 0
  r = await runPass(c);
  assert.equal(r.pending?.outcome, "rejected");
  assert.equal(JSON.parse(readFileSync(join(v.dir, "state", "keeper-state.json"), "utf8")).history[0].outcome, "dropped");
});

test("a node that refuses the bytes outright (insufficient funds) leaves nothing in flight; a failed simulation sends nothing and says why by name", async () => {
  const w = world({ send: true }); chainAt(w, 9466301);
  w.chain.rejectSend = "insufficient funds for gas * price + value";
  const b = await built(w);
  const r = await runPass(b);
  assert.equal(r.collections[0].floor.result?.outcome, "rejected");
  assert.equal(b.store.get().pending, null);
  w.chain.rejectSend = null;
  const bad = await b.sender.submit({ key: "x", what: "postCap from a vault the keeper does not own", to: RAFFLE,
    data: encodeFunctionData({ abi: VAULT, functionName: "postCap", args: [0n] }) });
  assert.equal(bad.outcome, "simulation-reverted");
  assert.equal(bad.detail, "BadInput()");
});

test("the key is never printed: not in a log line, a status body, a pass report or an error", async () => {
  const w = world({ send: true }); chainAt(w, 9466301);
  const b = await built(w);
  const r: PassReport = await runPass(b);
  w.chain.rejectSend = `nonce too low: key ${w.key}`;      // a node that (absurdly) echoes it: still cut out of the log
  w.chain.advance(31 * 60);
  const r2 = await runPass(b);
  assert.equal(r2.collections[0].floor.result?.outcome, "rejected");
  // what /status and --once serve passes through the signer's scrubber (service/status.ts, service/main.ts)
  const status = b.signer!.scrub(JSON.stringify(statusOf(b, { startedAt: "", passes: 2, lastPassAt: null, lastPassMs: null, consecutiveFailures: 0, lastError: null, last: r2 })));
  const everything = [...w.lines, status, JSON.stringify(r)].join("\n").toLowerCase();
  const bare = w.key.slice(2).toLowerCase();
  assert.ok(w.lines.length > 5);
  assert.ok(!everything.includes(bare), "the key's hex appears nowhere");
  assert.ok(w.lines.some((l) => l.includes("[redacted]")), "the echoed key was cut out of the log line");
});

test("the registry is read for the dry run's sender; a dry run on 4663 with a fixture is allowed (it signs nothing)", async () => {
  const w = world({ chainId: 4663 });
  const b = await built(w);
  assert.equal(b.cfg.mode, "dry-run");
  assert.equal(b.from.toLowerCase(), w.address.toLowerCase(), "simulated from registry.keeper()");
  await runPass(b);
  assert.equal(w.chain.sent.length, 0);
  assert.ok(REG);
});
