import assert from "node:assert/strict";
import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { DERIVED_TABLES, getMeta, openDb } from "../src/db.js";
import { Follower, loadDeployment } from "../src/follow.js";
import { Live } from "../src/live.js";
import { Views } from "../src/views.js";
import { clockOf, headOf, loadTape, logsOf, replay } from "./helpers.js";

const dump = (db: ReturnType<typeof openDb>) => JSON.stringify(Object.fromEntries([...DERIVED_TABLES, "events"].map((t) => [t, db.prepare(`SELECT * FROM ${t} ORDER BY 1, 2, 3`).all()])));

test("the recorded local run, replayed with no node: a coin, its harvest split 80/20, a purchase settled with a receipt, a draw resolved by a real drand round and delivered", async () => {
  const t = loadTape(), db = openDb(":memory:"), rpc = replay(t);
  const f = new Follower({ db, rpc, deployment: t.deployment, confirmations: 0 });
  const n = await f.pass();
  assert.ok(n >= 20, `${n} events`);
  assert.equal(f.cursor(), headOf(t));
  const v = new Views(db, new Live(rpc, t.deployment.ponsFactory), () => clockOf(t));
  const s = v.stats();
  assert.deepEqual([s.coins, s.listings, s.purchases.settled, s.acquisitions.inscriptions, s.draws.delivered], [1, 2, 1, 1, 1]);
  assert.equal(BigInt(s.toVaultsWei) * 10_000n / (BigInt(s.toVaultsWei) + BigInt(s.toTreasuryWei)), 8000n, "the events say 80/20; the API reports the events");
  const c = (await v.coin(v.coins(10, 0).items[0].address))!;
  assert.deepEqual([c.symbol, c.creatorTaxBps, c.harvests], ["STONE", 200, 1]);
  assert.equal(c.volumeUntilNextBuy.reason, null);
  assert.equal(c.volumeUntilNextBuy.inputs.takeBps, 270, "0.70% measured, plus the coin's 2% tax read from Pons's own record");
  assert.ok(BigInt(c.volumeUntilNextBuy.volumeNeededWei!) > 0n);
  assert.deepEqual([c.live!.queued, c.live!.pending.graduated, c.live!.pending.curvePending], [true, false, null]);
  const l = (await v.listing(0))!;
  assert.deepEqual([l.kind, l.divisibility, l.declarations[0].status, l.acquisitionsRecent[0].delivery.state, l.capExpired], ["inscriptions", 0, "settled", "delivered", false]);
  assert.equal(l.live!.spendableWei, String(BigInt(l.live!.attributedWei!) - BigInt(l.live!.reservedWei!)));
  const d = v.draw(0, 0)!;
  assert.ok(d.draw!.round! > 20_000_000 && /^0x[0-9a-f]{64}$/.test(d.root!));
  assert.deepEqual(d.timeline.map((x) => x.event), ["Receipted", "Chosen", "DrawOpened", "DrawPinned", "DrawResolved", "Won", "DestinationSet", "Delivered"]);
});

test("idempotent and resumable: a second pass adds nothing, and a follower started again on the same file carries on from its cursor", async () => {
  const t = loadTape(), db = openDb(":memory:"), rpc = replay(t);
  const f = new Follower({ db, rpc, deployment: t.deployment, confirmations: 0 });
  await f.pass();
  const before = dump(db);
  assert.equal(await f.pass(), 0);
  assert.equal(await new Follower({ db, rpc, deployment: t.deployment, confirmations: 0 }).pass(), 0);
  assert.equal(dump(db), before);
  assert.ok(!rpc.calls.some((m) => !["eth_blockNumber", "eth_getLogs", "eth_getBlockByNumber", "eth_call", "eth_chainId", "eth_getBalance", "eth_getTransactionByHash"].includes(m)), "reads only");
});

test("confirmations: nothing past head − N is indexed", async () => {
  const t = loadTape(), db = openDb(":memory:"), head = headOf(t);
  // the recording holds ranges up to the head only, so any range it is asked for with a different end is answered from the full one
  const rpc = replay(t, (m, p) => (m === "eth_getLogs" ? (() => {
    const q = p[0] as { address: string[]; fromBlock: string; toBlock: string };
    const full = Object.entries(t.tape).filter(([k]) => k.startsWith("eth_getLogs|")).flatMap(([, v]) => v as { address: string; blockNumber: string }[]);
    return full.filter((l) => q.address.includes(l.address.toLowerCase()) && BigInt(l.blockNumber) >= BigInt(q.fromBlock) && BigInt(l.blockNumber) <= BigInt(q.toBlock));
  })() : m === "eth_getBlockByNumber" && !(`${m}|${JSON.stringify(p)}` in t.tape) ? { number: p[0], hash: "0x" + "0".repeat(63) + "1", parentHash: "0x" + "0".repeat(64), timestamp: "0x1" } : undefined));
  const f = new Follower({ db, rpc, deployment: t.deployment, confirmations: 5 });
  await f.pass();
  assert.equal(f.cursor(), head - 5);
  assert.equal((db.prepare(`SELECT max(block_number) AS b FROM events`).get() as { b: number }).b <= head - 5, true);
});

test("a reorg: when the node no longer agrees with the cursor block's hash, everything above the newest block it still agrees with is dropped and the tables are rebuilt from what remains", async () => {
  const t = loadTape(), db = openDb(":memory:");
  const f1 = new Follower({ db, rpc: replay(t), deployment: t.deployment, confirmations: 0 });
  await f1.pass();
  const blocks = (db.prepare(`SELECT DISTINCT block_number AS b FROM events ORDER BY b`).all() as { b: number }[]).map((r) => r.b);
  const keep = blocks[Math.floor(blocks.length / 2)], eventsKept = (db.prepare(`SELECT count(*) AS n FROM events WHERE block_number <= ?`).get(keep) as { n: number }).n;
  // the same chain up to `keep`; above it, different hashes and no logs of ours
  const forked = replay(t, (m, p) => {
    if (m === "eth_getBlockByNumber" && BigInt(p[0] as string) > BigInt(keep)) return { number: p[0], hash: "0x" + "f".repeat(64), parentHash: "0x" + "0".repeat(64), timestamp: "0x6ab0a000" };
    if (m === "eth_getLogs") {
      const q = p[0] as Parameters<typeof logsOf>[1];
      if (BigInt(q.fromBlock) > BigInt(keep)) return [];
      // a consistent node: a range that starts on the surviving chain holds only the logs that survived
      return logsOf(t, q).filter((l) => BigInt(l.blockNumber) <= BigInt(keep));
    }
    return undefined;
  });
  const f2 = new Follower({ db, rpc: forked, deployment: t.deployment, confirmations: 0 });
  await f2.pass();
  assert.equal(f2.state.reorgs, 1);
  assert.equal((db.prepare(`SELECT count(*) AS n FROM events`).get() as { n: number }).n, eventsKept);
  assert.equal((db.prepare(`SELECT max(block_number) AS b FROM events`).get() as { b: number }).b, keep);
  assert.equal(getMeta(db, "cursor_hash"), "0x" + "f".repeat(64), "and it has indexed the new branch to its head");
  // what is left is exactly what a fresh index of the surviving chain gives
  const fresh = openDb(":memory:");
  await new Follower({ db: fresh, rpc: forked, deployment: t.deployment, confirmations: 0 }).pass();
  const strip = (s: string) => s;   // both were folded from the same events in the same order
  assert.equal(strip(dump(db)), strip(dump(fresh)));
});

test("no deployment file, a file for another chain, or a broken one: it idles, says so, and every list is empty — nothing is invented", async () => {
  const dir = mkdtempSync(join(tmpdir(), "etch-dep-"));
  assert.equal(loadDeployment(dir, 4663), null);
  writeFileSync(join(dir, "4663.json"), "{ not json");
  assert.equal(loadDeployment(dir, 4663), null);
  writeFileSync(join(dir, "4663.json"), JSON.stringify({ chainId: 1, registry: "0x" + "11".repeat(20), launcher: "0x" + "22".repeat(20), distributor: "0x" + "33".repeat(20), fromBlock: 5 }));
  assert.equal(loadDeployment(dir, 4663), null, "a file that names another chain is not this chain's deployment");
  writeFileSync(join(dir, "4663.json"), JSON.stringify({ chainId: 4663, registry: "0x" + "11".repeat(20), launcher: "0x" + "22".repeat(20), distributor: "0x" + "33".repeat(20), deployedAtBlock: 26_000_000 }));
  assert.equal(loadDeployment(dir, 4663), null, "deployedAtBlock is the L1 height a contract sees, never a place to read logs from: fromBlock is required");
  writeFileSync(join(dir, "4663.json"), JSON.stringify({ chainId: 4663, registry: "0x" + "11".repeat(20), launcher: "0x" + "22".repeat(20), distributor: "0x" + "33".repeat(20), fromBlock: 5 }));
  assert.deepEqual(loadDeployment(dir, 4663), { chainId: 4663, registry: "0x" + "11".repeat(20), launcher: "0x" + "22".repeat(20), distributor: "0x" + "33".repeat(20), ponsFactory: null, fromBlock: 5, demo: false });

  const db = openDb(":memory:"), calls: string[] = [];
  const f = new Follower({ db, rpc: (async (m: string) => { calls.push(m); throw new Error("no"); }) as never, deployment: null, confirmations: 20 });
  assert.equal(await f.pass(), 0);
  assert.deepEqual([calls.length, f.state.idle], [0, true], "an idle follower asks the node nothing");
  const v = new Views(db, new Live(null, null));
  const h = v.health(f.state, 20, "1");
  assert.deepEqual([h.deployment, h.cursor, h.events, v.coins(10, 0).total, v.listings().total, v.acquisitions(10, 0).total, v.draws(10, 0).total, v.stats().claimedWei], [false, null, 0, 0, 0, 0, 0, "0"]);
});

test("a node that fails is an error in /v1/health, not a crash and not a changed table", async () => {
  const t = loadTape(), db = openDb(":memory:");
  const f = new Follower({ db, rpc: replay(t, (m) => { if (m === "eth_getLogs") throw Object.assign(new Error("boom"), {}); return undefined; }), deployment: t.deployment, confirmations: 0 });
  await assert.rejects(f.pass());
  assert.equal(f.state.lastError!.kind, "internal");
  assert.equal((db.prepare(`SELECT count(*) AS n FROM events`).get() as { n: number }).n, 0);
  assert.equal(new Views(db, new Live(null, null)).health(f.state, 0, "1").ok, false);
});
