import assert from "node:assert/strict";
import { test } from "node:test";
import { handler } from "../src/api.js";
import { getMeta, openDb, setMeta } from "../src/db.js";
import { apply, foldOrder, rebuild, type Ev } from "../src/fold.js";
import { BACKFILL_RANGES, Follower, TRANSFER } from "../src/follow.js";
import { Live } from "../src/live.js";
import { Views } from "../src/views.js";
import { clockOf, loadTape, replay } from "./helpers.js";
import { COIN, COIN2, CURVE, CURVE2, HOOK, POOLS, ROUTER, story, T0, TX, WHO, WINNER, ZERO } from "./story.js";

/**
 * docs/phase13.md. Holders, folded from the coins' own `Transfer` logs and from nothing else.
 *
 * What these tests hold: that a balance is a sum of signed log deltas (mint and burn included), that the zero address is
 * never a holder, that the plumbing — a coin's curve, its router, Pons's factory, the v4 PoolManager and its hook — is
 * excluded from the count and said out loud rather than hidden, that a percentage of an unread supply is `null` and never
 * `NaN`, and that the history the indexer started too late for is fetched once, in bounded pieces, and admitted to until it
 * is complete.
 */

/** The story, applied and kept as events, so a rebuild has something to rebuild from. */
const filled = () => {
  const db = openDb(":memory:");
  const ins = db.prepare(`INSERT INTO events (block_number, log_index, tx, address, contract, name, args, ts, listing_id, acquisition_id) VALUES (?, ?, ?, ?, ?, ?, ?, ?, NULL, NULL)`);
  for (const e of story()) { apply(db, e); ins.run(e.block, e.logIndex, e.tx, e.address, e.contract, e.name, JSON.stringify(e.args), e.ts); }
  return db;
};
const viewsOf = (db: ReturnType<typeof openDb>) => new Views(db, new Live(null, null), () => T0 + 10_000);
const balances = (db: ReturnType<typeof openDb>, coin: string) =>
  Object.fromEntries((db.prepare(`SELECT holder, amount FROM balances WHERE coin = ? ORDER BY holder`).all(coin) as { holder: string; amount: string }[])
    .map((r) => [r.holder, r.amount]));

test("a balance is the sum of a coin's Transfer deltas: the mint lands, the burn leaves, and the zero address is never a holder", () => {
  const db = filled();
  assert.deepEqual(balances(db, COIN), { [CURVE]: "400000", [WHO]: "500000", [WINNER]: "50000" },
    "minted 1,000,000 to the curve, 600,000 sold on, 100,000 moved wallet to wallet, 50,000 burnt");
  assert.equal(db.prepare(`SELECT count(*) AS n FROM balances WHERE holder = ?`).get(ZERO)!.n, 0, "a mint's source and a burn's destination is not a holder");
  // COIN2: the curve sold all of it and the buyer passed all of it on, so two rows reached zero and were deleted
  assert.deepEqual(balances(db, COIN2), { [WINNER]: "900" }, "a balance that reaches zero loses its row rather than staying on as a holder of nothing");
  const total = (db.prepare(`SELECT sum(CAST(amount AS REAL)) AS s FROM balances WHERE coin = ?`).get(COIN) as { s: number }).s;
  assert.equal(total, 950_000, "what is held adds up to what was minted less what was burnt");
});

test("balances come from the events alone: a rebuild reproduces them, and the order the deltas arrive in cannot change them", () => {
  const db = filled();
  const before = balances(db, COIN);
  rebuild(db);
  assert.deepEqual(balances(db, COIN), before, "a reindex of the same events gives the same balances");
  // the same transfers applied back to front: addition commutes, which is why a backfill may fold history after the present
  const other = openDb(":memory:");
  const evs = story();
  for (const e of evs.filter((x) => x.contract === "launcher")) apply(other, e);
  for (const e of evs.filter((x) => x.contract === "coin").reverse()) apply(other, e);
  assert.deepEqual(balances(other, COIN), before, "a transfer folded out of order still leaves the same balance");
});

test("identity first: a coin's mint sits seven logs ahead of the Launched that says the coin is ours, and is folded anyway", () => {
  const db = openDb(":memory:");
  const ins = db.prepare(`INSERT INTO events (block_number, log_index, tx, address, contract, name, args, ts, listing_id, acquisition_id) VALUES (?, ?, ?, ?, ?, ?, ?, ?, NULL, NULL)`);
  // the shape measured on staging: the mint at logIndex 0, our Launched at logIndex 7, in one block
  const mint: Ev = { block: 500, logIndex: 0, tx: TX, address: COIN, contract: "coin", name: "Transfer", args: { from: ZERO, to: CURVE, value: "1000000" }, ts: T0 };
  const launched: Ev = { block: 500, logIndex: 7, tx: TX, address: "0x" + "22".repeat(20), contract: "launcher", name: "Launched",
    args: { coin: COIN, creator: WHO, listingId: "0", curve: CURVE, router: ROUTER }, ts: T0 };
  for (const e of [mint, launched]) ins.run(e.block, e.logIndex, e.tx, e.address, e.contract, e.name, JSON.stringify(e.args), e.ts);
  rebuild(db);
  assert.deepEqual(balances(db, COIN), { [CURVE]: "1000000" }, "the whole supply is where it was minted, not lost to log order");
  assert.ok(foldOrder(launched, mint) < 0, "the comparator the follower uses puts the identity event first inside a block");
  assert.ok(foldOrder({ block: 499, logIndex: 9, name: "Transfer" }, launched) < 0, "and an earlier block still comes first");
  // strict log order is what this replaces: it would meet a coin it has never heard of and drop the mint
  const naive = openDb(":memory:");
  for (const e of [mint, launched]) apply(naive, e);
  assert.deepEqual(balances(naive, COIN), {}, "which is exactly what folding in strict log order costs");
});

test("the plumbing is not a holder: the curve, the router, the factory, the PoolManager and the hook are excluded, and said out loud", () => {
  const db = filled();
  setMeta(db, "pons_factory", "0x" + "7e".repeat(20));
  setMeta(db, "pool_manager", POOLS);
  // the factory and the PoolManager holding some of it, the way Pons's 2/7 and a v4 pool's liquidity do on a real chain
  for (const [who, v] of [["0x" + "7e".repeat(20), "10000"], [POOLS, "5000"], [ROUTER, "1"]] as const)
    apply(db, { block: 900, logIndex: 0, tx: TX, address: COIN, contract: "coin", name: "Transfer", args: { from: WHO, to: who, value: v }, ts: T0 });
  const h = viewsOf(db).holders(COIN, 25, 0)!;
  assert.equal(h.holders, 2, "WHO and WINNER hold this coin; the other four addresses are its own plumbing");
  assert.deepEqual(h.items.map((x) => [x.rank, x.address.toLowerCase(), x.balanceWei]), [[1, WHO, "484999"], [2, WINNER, "50000"]]);
  assert.deepEqual(h.excluded.map((x) => x.why).sort(), ["curve", "pons-factory", "pool-manager", "router"].sort(),
    "the four that hold something; the hook holds none of this coin and so is not listed");
  assert.deepEqual(h.excluded.filter((x) => x.why === "curve").map((x) => x.balanceWei), ["400000"], "the curve's own balance is served, not hidden");
  assert.equal(h.excluded.find((x) => x.why === "pons-factory")!.balanceWei, "10000");
  assert.ok(h.excluded.every((x) => BigInt(x.balanceWei) > 0n), "an excluded address holding nothing is not listed");
  assert.equal(h.total, 2);
  assert.equal(viewsOf(db).coins(50, 0).items.find((c) => c.address.toLowerCase() === COIN)!.holders, 2, "the summary carries the same count, for a card");
});

test("a share is a fraction of the supply, and an unread supply is null rather than NaN", () => {
  const db = filled();
  const none = viewsOf(db).holders(COIN, 25, 0)!;
  assert.equal(none.supplyWei, null, "the synthetic set never read totalSupply: there is no denominator");
  assert.deepEqual(none.items.map((x) => x.share), [null, null], "so there is no percentage, and certainly not a NaN");
  db.prepare(`INSERT OR REPLACE INTO coin_meta (coin, supply) VALUES (?, ?)`).run(COIN, "1000000");
  const some = viewsOf(db).holders(COIN, 25, 0)!;
  assert.deepEqual(some.items.map((x) => x.share), [0.5, 0.05], "500,000 of a million is 0.5 — a fraction, which a screen turns into 50%");
  assert.equal(some.excluded.find((x) => x.why === "curve")!.share, 0.4);
  assert.ok(some.items.every((x) => Number.isFinite(x.share!)), "every share a page might print is a finite number");
  db.prepare(`UPDATE coin_meta SET supply = '0' WHERE coin = ?`).run(COIN);
  assert.equal(viewsOf(db).holders(COIN, 25, 0)!.items[0].share, null, "a supply of zero is not a denominator either");
});

test("paging: rank is over the whole list rather than the page, and a coin nobody holds is an empty list, not an error", () => {
  const db = filled();
  const v = viewsOf(db);
  const first = v.holders(COIN, 1, 0)!, second = v.holders(COIN, 1, 1)!, past = v.holders(COIN, 1, 2)!;
  assert.deepEqual([first.items[0].rank, second.items[0].rank], [1, 2], "the first row of page two is 2, not 1");
  assert.deepEqual([first.total, second.total, past.total], [2, 2, 2]);
  assert.deepEqual(past.items, []);
  assert.ok(first.items[0].balanceWei > second.items[0].balanceWei, "largest first");
  assert.equal(v.holders("0x" + "ab".repeat(20), 25, 0), null, "a coin that was never launched here is a 404, not an empty page");
  // a coin launched and never held: 0 is a fact
  apply(db, { block: 950, logIndex: 0, tx: TX, address: "0x" + "22".repeat(20), contract: "launcher", name: "Launched",
    args: { coin: "0x" + "cc".repeat(20), creator: WHO, listingId: "0", curve: "0x" + "ce".repeat(20), router: "0x" + "cf".repeat(20) }, ts: T0 });
  const fresh = v.holders("0x" + "cc".repeat(20), 25, 0)!;
  assert.deepEqual([fresh.holders, fresh.items.length, fresh.complete], [0, 0, true]);
});

test("the creator is marked where they hold, and the creator's own address is served so a page can mark it", () => {
  const db = filled();
  const h = viewsOf(db).holders(COIN, 25, 0)!;
  assert.equal(h.creator.toLowerCase(), WHO, "the story's coin was launched by WHO");
  assert.deepEqual(h.items.map((x) => x.isCreator), [true, false]);
});

test("the API: a holders page, its limit clamped, and a 404 for a coin nobody launched", async () => {
  const db = filled();
  const ask = async (path: string) => {
    const res: { status?: number; body?: string; headers?: Record<string, string> } = {};
    const fake = { writeHead: (s: number, h: Record<string, string>) => { res.status = s; res.headers = h; }, end: (b?: string) => { res.body = b; }, headersSent: false } as never;
    await handler({ views: viewsOf(db), follower: () => ({ lastPassAt: T0, lastError: null, head: null, reorgs: 0, idle: false }), confirmations: 0, perMinute: 1e6, burst: 1e6 })(
      { method: "GET", url: path, headers: {}, socket: { remoteAddress: "1.1.1.1" } } as never, fake);
    return { status: res.status!, json: JSON.parse(res.body!) };
  };
  const ok = await ask(`/v1/coins/${COIN}/holders?limit=1`);
  assert.equal(ok.status, 200);
  assert.deepEqual([ok.json.total, ok.json.items.length, ok.json.limit], [2, 1, 1]);
  assert.equal(ok.json.coin.toLowerCase(), COIN);
  assert.notEqual(ok.json.coin, COIN, "addresses come back checksummed, like everywhere else in this API");
  assert.equal((await ask(`/v1/coins/0x${"ab".repeat(20)}/holders`)).status, 404);
  assert.equal((await ask(`/v1/coins/${COIN}/holders?limit=9999`)).json.limit, 200, "the list limit is the same 200 every list has");
  assert.ok((await ask("/v1")).json.endpoints.includes("/v1/coins/:address/holders"), "and the endpoint says it exists");
});

/**
 * docs/phase13.md §4. The history `Transfer` indexing started too late for.
 *
 * The recorded run is replayed with an override that answers a by-topic `Transfer` query, because the tape was recorded
 * before the follower asked that question. What is being tested is the walk and its bounds, not the node.
 */
const backfilled = async (maxRange: number, passes: number) => {
  const t = loadTape(), db = openDb(":memory:");
  const ranges: [number, number][] = [];
  const rpc = replay(t, (method, params) => {
    if (method !== "eth_getLogs") return undefined;
    const q = params[0] as { address?: string | string[]; topics?: string[]; fromBlock: string; toBlock: string };
    if (q.topics?.[0] !== TRANSFER) return undefined;
    const [from, to] = [Number(BigInt(q.fromBlock)), Number(BigInt(q.toBlock))];
    ranges.push([from, to]);
    const coin = (Array.isArray(q.address) ? q.address[0] : q.address) ?? "";
    // one transfer of 7 in the first block of the range, from the coin's creator to a wallet, so a balance appears where
    // only a backfill could have put it
    return [{ address: coin, topics: [TRANSFER, `0x${"0".repeat(24)}${WHO.slice(2)}`, `0x${"0".repeat(24)}${WINNER.slice(2)}`],
      data: `0x${(7).toString(16).padStart(64, "0")}`, blockNumber: `0x${from.toString(16)}`, logIndex: "0x0",
      transactionHash: TX, blockHash: "0x" + "00".repeat(32), removed: false }];
  });
  // a database that has already followed this chain, and only now learns to index transfers: the live edge is its cursor.
  // The first walk uses the range the recording was made with; only the backfill under test walks in small steps.
  const first = new Follower({ db, rpc, deployment: t.deployment, confirmations: 0 });
  await first.pass();
  const cursor = first.cursor()!;
  db.exec(`DELETE FROM balances`);
  setMeta(db, "transfers_live_from", String(cursor));
  setMeta(db, "transfers_backfilled_through", String(t.deployment.fromBlock - 1));
  const f = new Follower({ db, rpc, deployment: t.deployment, confirmations: 0, maxRange });
  ranges.length = 0;
  for (let i = 0; i < passes; i++) await f.pass();
  return { db, f, views: new Views(db, new Live(null, null), () => clockOf(t)), ranges, cursor, from: t.deployment.fromBlock };
};

test("the backfill is bounded, resumable and finite: two ranges a pass, oldest first, and then it never runs again", async () => {
  const one = await backfilled(5, 1);
  assert.equal(one.ranges.length, BACKFILL_RANGES, "one pass fetches two ranges of history and no more");
  assert.deepEqual(one.ranges, [[one.from, one.from + 4], [one.from + 5, one.from + 9]], "oldest first, in ranges of maxRange");
  assert.equal(Number(getMeta(one.db, "transfers_backfilled_through")), one.from + 9, "and remembers where it got to");

  const done = await backfilled(5, 12);
  assert.equal(Number(getMeta(done.db, "transfers_backfilled_through")), done.cursor, "twelve passes reach the live edge");
  const after = done.ranges.length;
  await done.f.pass();
  assert.equal(done.ranges.length, after, "a pass after that asks for no history at all: it is fetched once, not every pass");
});

test("while the backfill runs, a holder count is null rather than the tail of a coin's history", async () => {
  const part = await backfilled(5, 1);
  const coin = part.views.coins(50, 0).items[0];
  assert.equal(coin.holders, null, "an incomplete scan has no count to give, in the API's own convention");
  const page = part.views.holders(coin.address, 25, 0)!;
  assert.equal(page.complete, false);
  assert.equal(page.holders, null);
  assert.ok(page.scannedThrough! < part.cursor, "and it says how far it has got, so a page can say the list is still being built");
  assert.ok(page.items.length > 0, "the balances it does have are served: they are right for the blocks it has read");

  const done = await backfilled(5, 12);
  const full = done.views.coins(50, 0).items[0];
  assert.equal(typeof full.holders, "number", "once the walk meets the live edge the count stands behind itself");
  assert.equal(done.views.holders(full.address, 25, 0)!.complete, true);
  assert.equal(done.views.holders(full.address, 25, 0)!.holders, full.holders);
});

test("a database that never followed a chain is complete: nothing was skipped because nothing was ever indexed", () => {
  const h = viewsOf(filled()).holders(COIN, 25, 0)!;
  assert.deepEqual([h.complete, h.scannedThrough, h.transfersFrom], [true, null, null]);
  assert.equal(typeof h.holders, "number");
  assert.ok(HOOK.length === 42 && CURVE2.length === 42);      // the story's own addresses, kept honest
});
