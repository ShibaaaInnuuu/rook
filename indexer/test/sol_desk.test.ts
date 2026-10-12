// W13, desk events from memos (canister-only design §4.4 rule 4, §10): the memo grammar, a memo-carrying transaction decoded into
// its label and its balance deltas, the follower fetching only memo-carrying signatures of the canister's key, and the API.
import { test } from "node:test";
import assert from "node:assert/strict";
import { decodeTx, type ParsedTx } from "../src/sol/decode.js";
import { openSolDb } from "../src/sol/db.js";
import { SolIndexer } from "../src/sol/indexer.js";
import { SolService, type DeskEvent } from "../src/sol/service.js";
import { route } from "../src/sol/api.js";
import { carriesRookMemo, deskEvents, parseMemo, parseRpcMemo } from "../src/sol/memo.js";
import { CONFIG, DLMM, K_MAIN, MODULE, WX, configAccount, fakeCanister, fakeRpc, fx, mintAccount, rpcMemo } from "./solWorld.js";

const MINTER_ATA = "8StSvV6vnNpE8GE2hne5dDSmfJb2QKXWZ75GRFmkLDFr", MINTER = "BUQaUYtAmdm5bSUwqVkSsWCt4R53eQ3cYy2D6nT4r2gA", DEPOSIT = "AoVsGaj8MSJ6xwKxfFxo9iZWH3enC8RRTXKH2fx2F8os";
const know = { wxMintOf: (c: number) => (c === 0 ? WX : null), moduleOf: (c: number) => (c === 0 ? MODULE : null), isWxMint: (m: string) => m === WX };

/** The synthetic mint, turned into the matching burn: the memo's kind, the SOL back out of the module to the burner, the wX burned. */
function burnOf(mint: ParsedTx, sig: string, slot: number): ParsedTx {
  const t = structuredClone(mint);
  t.transaction.signatures = [sig]; t.slot = slot; t.blockTime = mint.blockTime! + 600;
  t.meta!.preBalances = [10_000_000_000, 890_880, 0, 6_000_000_000, 2_039_280, 1_461_600, 1, 934_087_680, 1];
  t.meta!.postBalances = [9_999_995_000, 890_880 + 990_000_000, 0, 5_010_000_000, 2_039_280, 1_461_600, 1, 934_087_680, 1];
  t.meta!.preTokenBalances![0].uiTokenAmount.amount = "50000000"; t.meta!.postTokenBalances![0].uiTokenAmount.amount = "0";
  t.transaction.message.instructions[2].parsed = "rook:v1:0:burn:43";
  return t;
}

test("the memo grammar, as memo.rs writes and reads it; the RPC's [len] decoration stripped", () => {
  assert.deepEqual(parseMemo("rook:v1:3:mint:42:i7"), { coll: 3, kind: "mint", id: 42, intent: 7 });
  assert.deepEqual(parseMemo(" rook:v1:-:refund:9 "), { coll: null, kind: "refund", id: 9, intent: null });
  for (const bad of ["rook:v2:3:mint:1", "rook:v1:x:mint:1", "rook:v1:3:Mint:1", "rook:v1:3:mint:1:7", "rook:v1:3:mint:1:i7:9", "rook:v1:3:mint", "rook:v1:70000:mint:1", "hello", ""]) assert.equal(parseMemo(bad), null, bad);
  assert.equal(parseRpcMemo(`${rpcMemo("rook:v1:3:mint:42:i7")}; [2] hi; ${rpcMemo("rook:v1:-:refund:9")}`).length, 2);
  assert.deepEqual(parseRpcMemo("rook:v1:0:burn:1"), [{ coll: 0, kind: "burn", id: 1, intent: null }], "an undecorated memo still reads");
  assert.deepEqual(parseRpcMemo(null), []);
  assert.equal(carriesRookMemo(null), false); assert.equal(carriesRookMemo("[5] hello"), false); assert.equal(carriesRookMemo(rpcMemo("rook:v1:0:mint:1")), true);
  assert.equal(carriesRookMemo(undefined), true, "an RPC that did not fill the field is not trusted to have looked");
});

test("a memo-carrying transaction: the label, and what the balances say (lamports per account, wX per token account)", () => {
  const tx = fx("desk-mint-rook-memo");
  const d = decodeTx(tx);
  assert.equal(d.ok, true); assert.equal(d.feePayer, K_MAIN); assert.equal(d.fee, 5000);
  assert.deepEqual(d.memos, ["rook:v1:0:mint:42:i7"]);
  assert.deepEqual(d.lamports, [{ address: K_MAIN, delta: -5000n }, { address: DEPOSIT, delta: -1_000_000_000n }, { address: MODULE, delta: 1_000_000_000n }]);
  assert.deepEqual(d.tokens, [{ account: MINTER_ATA, owner: MINTER, mint: WX, delta: 50_000_000n }]);
  const [e, ...rest] = deskEvents(d, know);
  assert.equal(rest.length, 0);
  assert.deepEqual({ ...e, deltas: undefined }, { n: 0, coll: 0, kind: "mint", linkId: 42, intent: 7, memo: "rook:v1:0:mint:42:i7", wxMint: WX, wxDelta: 50_000_000n, payerLamports: -5000n, moduleLamports: 1_000_000_000n, deltas: undefined });
  assert.deepEqual(e.deltas.tokens, [[MINTER, WX, "50000000"]]);
  // the collection unknown here: the one wX the transaction moved still names the mint, the module is unknown
  const e2 = deskEvents(d, { wxMintOf: () => null, moduleOf: () => null, isWxMint: (m) => m === WX })[0];
  assert.equal(e2.wxMint, WX); assert.equal(e2.wxDelta, 50_000_000n); assert.equal(e2.moduleLamports, null);
  // a memo that is not Rook's is no event; a memo on a failed transaction is none either; a Rook memo with nothing behind it is an event with empty deltas
  const other = structuredClone(tx); other.transaction.message.instructions[2].parsed = "gm";
  assert.equal(deskEvents(decodeTx(other), know).length, 0);
  const failed = structuredClone(tx); failed.meta!.err = { InstructionError: [0, "Custom"] };
  assert.equal(deskEvents(decodeTx(failed), know).length, 0);
  const empty = structuredClone(tx); empty.meta!.postBalances = [...empty.meta!.preBalances]; empty.meta!.postTokenBalances = structuredClone(empty.meta!.preTokenBalances);
  const e3 = deskEvents(decodeTx(empty), know)[0];
  assert.equal(e3.kind, "mint"); assert.equal(e3.wxDelta, 0n); assert.equal(e3.payerLamports, 0n); assert.deepEqual(e3.deltas, { lamports: [], tokens: [] });
  // a raw (unparsed) memo instruction reads the same
  const raw = structuredClone(tx); const ix = raw.transaction.message.instructions[2]; delete ix.parsed; ix.data = "2bU9No3gDGDePM9schW9L2KDqFcJ"; ix.accounts = [];
  assert.equal(decodeTx(raw).memos.length, 1);
});

/** The canister's key followed on the fake chain: three signatures in its history, one of them Rook's. */
async function followed() {
  const mint = fx("desk-mint-rook-memo");
  const burn = burnOf(mint, "f839aubTnyB84ebyREeCUVTnE9zMkbc6yvdCxoyu4T947Mn6aZm1AE1eHkDSqxMu565kYug1hSx3R6duzVe3Tky", mint.slot + 500);
  const advance = structuredClone(mint); advance.transaction.signatures = ["4uQurWbfKEz7zhyr6sjhRCzj8c3wJn8u8i8p1LjWimydneuMYX6QAYe763tkLK36NcLQnzYKdfvXAC3cDYrtXeKq"]; advance.slot = mint.slot + 1000;
  advance.transaction.message.instructions = [advance.transaction.message.instructions[0]];
  const rpc = fakeRpc({ accounts: { [CONFIG]: configAccount(WX), [WX]: mintAccount(9) },
    sigs: { [K_MAIN]: [{ tx: advance, memo: null }, { tx: burn, memo: rpcMemo("rook:v1:0:burn:43") }, { tx: mint, memo: `[3] gm; ${rpcMemo("rook:v1:0:mint:42:i7")}` }] } });
  const db = openSolDb(":memory:");
  const canister = fakeCanister([{ index: 0, wxMint: WX, config: CONFIG, module: MODULE, symbol: "wMILADY", stage: "Open" }]);
  const ix = new SolIndexer({ db, rpc, configs: [{ slug: "milady", listingId: 0, config: CONFIG }], http: false, canister, deskKeys: [K_MAIN], pageSize: 10, now: () => mint.blockTime! + 2_000 });
  await ix.init();
  return { db, ix, rpc, mint, burn };
}

test("the follower: a desk stream on the canister's key fetches only the memo-carrying signatures; idempotent", async () => {
  const { db, ix, rpc, mint } = await followed();
  const stream = () => db.prepare(`SELECT * FROM streams WHERE address = ?`).get(K_MAIN) as never;
  assert.deepEqual({ ...db.prepare(`SELECT kind FROM streams WHERE address = ?`).get(K_MAIN) }, { kind: "desk" });
  assert.equal(await ix.pollStream(stream()), 0, "the head is the nonce advance: no memo, not fetched");
  assert.equal(rpc.calls.getTransaction ?? 0, 0);
  assert.equal(await ix.pollStream(stream()), 2, "the backfill: the burn and the mint");
  assert.equal(rpc.calls.getTransaction, 2);
  assert.equal(await ix.pollStream(stream()), 0);
  const rows = db.prepare(`SELECT sig, slot, at, coll, kind, link_id, intent, payer, wx_mint, wx_delta, payer_lamports, module_lamports FROM desk_events ORDER BY slot`).all() as Record<string, unknown>[];
  assert.equal(rows.length, 2);
  assert.deepEqual({ ...rows[0] }, { sig: mint.transaction.signatures[0], slot: mint.slot, at: mint.blockTime, coll: 0, kind: "mint", link_id: 42, intent: 7, payer: K_MAIN, wx_mint: WX, wx_delta: "50000000", payer_lamports: "-5000", module_lamports: "1000000000" });
  assert.deepEqual([rows[1].kind, rows[1].link_id, rows[1].intent, rows[1].wx_delta, rows[1].module_lamports], ["burn", 43, null, "-50000000", "-990000000"]);
  assert.equal((db.prepare(`SELECT count(*) AS n FROM trades`).get() as { n: number }).n, 0, "a desk stream writes no trade");
  // a pass with the key dropped from the configuration leaves its stream alone
  const ix2 = new SolIndexer({ db, rpc, configs: [{ slug: "milady", listingId: 0, config: CONFIG }], http: false, deskKeys: [] });
  const before = rpc.calls.getSignaturesForAddress;
  await ix2.pass();
  assert.equal(rpc.calls.getSignaturesForAddress, before + 1, "only the config's stream was polled");
});

test("the API: /desk/events (filters, cursor), /desk/totals, and a memo is a label: the deltas are the transaction's", async () => {
  const { db, ix, mint, burn } = await followed();
  const s = () => db.prepare(`SELECT * FROM streams WHERE address = ?`).get(K_MAIN) as never;
  await ix.pollStream(s()); await ix.pollStream(s());
  const svc = new SolService(db, { now: () => burn.blockTime! + 10, ttlS: 0 });
  const get = (p: string) => { const u = new URL(p, "http://x"); return route({ svc, health: () => ({}) }, u.pathname, u); };
  const all = get("/v1/rook/desk/events").body as { items: DeskEvent[]; next: string | null; total: number };
  assert.equal(all.total, 2); assert.equal(all.next, null); assert.equal(all.items[0].kind, "burn", "newest first");
  const e = all.items[1];
  for (const k of ["signature", "slot", "at", "coll", "collection", "kind", "linkId", "intent", "payer", "memo", "feeLamports", "wxMint", "wxDelta", "wxDeltaUnits", "payerLamports", "module", "moduleLamports", "lamports", "tokens"]) assert.ok(k in e, `DeskEvent.${k}`);
  assert.equal(e.collection, "milady"); assert.equal(e.linkId, 42); assert.equal(e.intent, 7); assert.equal(e.wxDeltaUnits, 0.05); assert.equal(e.module, MODULE); assert.equal(e.moduleLamports, "1000000000");
  assert.deepEqual(e.lamports, [{ address: K_MAIN, delta: "-5000" }, { address: "AoVsGaj8MSJ6xwKxfFxo9iZWH3enC8RRTXKH2fx2F8os", delta: "-1000000000" }, { address: MODULE, delta: "1000000000" }]);
  assert.deepEqual(e.tokens, [{ owner: "BUQaUYtAmdm5bSUwqVkSsWCt4R53eQ3cYy2D6nT4r2gA", mint: WX, delta: "50000000" }]);
  // filters: by collection index or slug, by kind, since; the cursor
  assert.equal((get("/v1/rook/desk/events?coll=0").body as { total: number }).total, 2);
  assert.equal((get("/v1/rook/desk/events?coll=milady").body as { total: number }).total, 2);
  assert.equal((get("/v1/rook/desk/events?coll=-").body as { total: number }).total, 0);
  assert.equal(get("/v1/rook/desk/events?coll=azuki").status, 404);
  assert.equal(get("/v1/rook/desk/events?coll=../x").status, 400);
  assert.equal((get("/v1/rook/desk/events?kind=burn").body as { total: number }).total, 1);
  assert.equal(get("/v1/rook/desk/events?kind=Mint").status, 400);
  assert.equal((get(`/v1/rook/desk/events?since=${mint.blockTime! + 1}`).body as { total: number; items: DeskEvent[] }).items[0].kind, "burn");
  assert.equal(get("/v1/rook/desk/events?since=now").status, 400);
  const p1 = get("/v1/rook/desk/events?limit=1").body as { items: DeskEvent[]; next: string; total: number };
  assert.equal(p1.items.length, 1); assert.equal(p1.total, 2); assert.match(p1.next, /^\d+:\d+$/);
  const p2 = get(`/v1/rook/desk/events?limit=1&before=${p1.next}`).body as { items: DeskEvent[]; next: string | null };
  assert.equal(p2.items[0].kind, "mint"); assert.equal(p2.next, null);
  assert.equal(get("/v1/rook/desk/events?before=x").status, 400);
  // totals: sums of what the balances said under each label
  const tot = get("/v1/rook/desk/totals").body as { items: Record<string, unknown>[] };
  assert.equal(tot.items.length, 1);
  const t = tot.items[0];
  assert.equal(t.coll, 0); assert.equal(t.collection, "milady"); assert.equal(t.wxMint, WX); assert.equal(t.wxSymbol, "wMILADY"); assert.equal(t.module, MODULE); assert.equal(t.events, 2);
  assert.deepEqual(t.byKind, { mint: 1, burn: 1 });
  assert.equal(t.wxMinted, "50000000"); assert.equal(t.wxBurned, "50000000"); assert.equal(t.wxNet, "0"); assert.equal(t.wxNetUnits, 0);
  assert.equal(t.payerOut, "10000"); assert.equal(t.payerIn, "0"); assert.equal(t.moduleIn, "1000000000"); assert.equal(t.moduleOut, "990000000");
  assert.equal(t.firstAt, mint.blockTime); assert.equal(t.lastAt, burn.blockTime); assert.equal(t.lastSlot, burn.slot);
  assert.equal((get("/v1/rook/desk/totals?coll=milady").body as { items: unknown[] }).items.length, 1);
  assert.equal((get("/v1/rook/desk/totals?coll=-").body as { items: unknown[] }).items.length, 0);
  assert.deepEqual(svc.counts(), { coins: 0, trades: 0, deskEvents: 2 });
  assert.ok((get("/").body as { endpoints: string[] }).endpoints.some((x) => x.startsWith("/v1/rook/desk/events")));
});
