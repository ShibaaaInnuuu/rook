// The follower against a fake JSON-RPC endpoint serving recorded transactions: first page, backfill, forward cursor, a 429 in the
// middle, graduation opening the DAMM v2 stream, and nothing but reads on the wire.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import type { ParsedTx } from "../src/sol/decode.js";
import { openSolDb } from "../src/sol/db.js";
import { SolIndexer } from "../src/sol/indexer.js";
import { solRpc, SOL_READ_ONLY } from "../src/sol/rpc.js";
import { SolService } from "../src/sol/service.js";

const fx = (name: string) => JSON.parse(readFileSync(new URL(`./fixtures/sol/${name}.json`, import.meta.url), "utf8")) as ParsedTx;
const CONFIG = "9q4yEJsVqUQ4SQp24o8moARNrBFFCm1vZDKLo3KPkqTe", MINT = "8gTju7eND1D8pm52664toFE6gHZKMSGA8jnM9PvQoJkp", DAMM = "F9xaKn1zZCaa1NTiC96VKTfmjnd6Vx2e1FrJqbedVm92";

test("first page, backfill, forward, a 429 tolerated, graduation → DAMM stream; idempotent; reads only", async () => {
  const create = fx("dbc-swap-5LAUFFX9zK"), migrate = fx("migrate-damm-v2-FQgKq1qWYL"), damm = fx("damm-swap-23RE4dZAqF");
  const sigOf = (t: ParsedTx) => t.transaction.signatures[0];
  const byAddr: Record<string, ParsedTx[]> = { [CONFIG]: [migrate, create], [DAMM]: [damm] };   // newest first, as the RPC answers
  const txs = new Map([create, migrate, damm].map((t) => [sigOf(t), t]));
  const methods: string[] = [];
  let calls = 0;
  const f = (async (_url: string, init: { body: string }) => {
    const { method, params } = JSON.parse(init.body) as { method: string; params: unknown[] };
    methods.push(method);
    if (++calls === 3) return new Response("", { status: 429 });          // one rate limit mid-run
    let result: unknown = null;
    if (method === "getSignaturesForAddress") {
      const [addr, o] = params as [string, { limit: number; before?: string; until?: string }];
      let xs = byAddr[addr] ?? [];
      if (o.until) xs = xs.slice(0, Math.max(0, xs.findIndex((t) => sigOf(t) === o.until)));
      if (o.before) xs = xs.slice(xs.findIndex((t) => sigOf(t) === o.before) + 1);
      result = xs.slice(0, o.limit).map((t, i) => ({ signature: sigOf(t), slot: t.slot, err: null, blockTime: t.blockTime, transactionIndex: i }));
    } else if (method === "getTransaction") result = txs.get((params as string[])[0]) ?? null;
    else if (method === "getMultipleAccounts") result = { value: (params[0] as string[]).map(() => null) };
    return new Response(JSON.stringify({ jsonrpc: "2.0", id: 1, result }), { status: 200 });
  }) as unknown as typeof fetch;
  const db = openSolDb(":memory:");
  const rpc = solRpc(["https://fake"], { fetch: f, sleep: async () => {} });
  const ix = new SolIndexer({ db, rpc, configs: [{ slug: "milady", listingId: 0, config: CONFIG }], pageSize: 1, http: false, txConcurrency: 1 });
  await ix.init();
  const stream = (a: string) => db.prepare(`SELECT * FROM streams WHERE address = ?`).get(a) as never;

  assert.equal(await ix.pollStream(stream(CONFIG)), 0, "the newest page is the migration: no trade in it");
  assert.equal((db.prepare(`SELECT head_sig FROM streams WHERE address = ?`).get(CONFIG) as { head_sig: string }).head_sig, sigOf(migrate));
  assert.equal(await ix.pollStream(stream(CONFIG)), 1, "the backfill reaches the creation and its buy");
  await ix.pollStream(stream(CONFIG));
  const s = db.prepare(`SELECT back_done, back_count, seen FROM streams WHERE address = ?`).get(CONFIG) as { back_done: number; back_count: number; seen: number };
  assert.equal(s.back_done, 1); assert.equal(s.back_count, 2);
  // a second forward poll with nothing new adds nothing
  assert.equal(await ix.pollStream(stream(CONFIG)), 0);

  db.prepare(`UPDATE coins SET coin_is_a = 1 WHERE mint = ?`).run(MINT);         // what ensureDammSide reads from the pool account
  assert.equal(await ix.pollStream(stream(DAMM)), 1, "the DAMM stream the migration opened carries the pool's swap");
  const svc = new SolService(db, { now: () => damm.blockTime! + 1, ttlS: 0 });
  const c = svc.coin(MINT)!;
  assert.equal(c.phase, "graduated"); assert.equal(c.tradesAllTime, 2); assert.equal(c.name, "Goon Index 6900");
  assert.ok(methods.every((m) => (SOL_READ_ONLY as readonly string[]).includes(m)), "nothing but reads");
  assert.ok(rpc.state()[0].calls > calls - 1);
});
