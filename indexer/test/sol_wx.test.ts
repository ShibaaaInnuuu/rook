// W13, wX-quoted prices (canister-only design §3, §5): the desk's DLMM pair → SOL per wX (against the canister's landed pair
// fixture), the canister's accepted floor as the fallback (against its own books_view bytes), the follower storing both, and the
// API answering a wX-quoted coin in wX, SOL and USD.
import { test } from "node:test";
import assert from "node:assert/strict";
import { decodeTx } from "../src/sol/decode.js";
import { openSolDb } from "../src/sol/db.js";
import { SolIndexer } from "../src/sol/indexer.js";
import { SolService, quoteOf, type SolCoin } from "../src/sol/service.js";
import { route } from "../src/sol/api.js";
import { storeRate } from "../src/sol/usd.js";
import { binPrice, decodeLbPair, pairAddress, presetAddress, solPerWx, storeWxRate, latestWxRate, wxRateLookup, DESK_PRESET } from "../src/sol/wx.js";
import { b64decode } from "../src/sol/bytes.js";
import { BOOKS_VIEW, CONFIG, CONFIG2, DLMM, MINT, WX, WX2, configAccount, dlmmAccount, fakeCanister, fakeRpc, fx, mintAccount } from "./solWorld.js";

const near = (a: number, b: number, rel: number, what = "") => assert.ok(Math.abs(a - b) / b <= rel, `${what} ${a} vs ${b}`);

test("the desk's pair address: preset 262, then [preset, min(wX, wSOL), max] — the canister's landed pair", () => {
  assert.equal(DESK_PRESET, 262);
  assert.equal(presetAddress(), DLMM.keys.preset);
  assert.equal(pairAddress(WX), DLMM.keys.pair);
});

test("LbPair decode and the active bin's price: (1 + 50/10⁴)^586 ≈ the 18.61 SOL floor the pair was opened around", () => {
  const a = dlmmAccount("lb_pair_open");
  const p = decodeLbPair(b64decode(a.data[0]));
  assert.equal(p.activeId, a.expect.active_id); assert.equal(p.binStep, 50); assert.equal(p.status, 0); assert.equal(p.pairType, 3);
  assert.equal(p.tokenXMint, WX); assert.equal(p.tokenYMint, "So11111111111111111111111111111111111111112");
  assert.equal(p.reserveX, a.expect.reserve_x); assert.equal(p.reserveY, a.expect.reserve_y);
  const sol = solPerWx(p, WX)!;
  assert.equal(sol, binPrice(50, 586));
  near(sol, DLMM.keys.floor, 0.005, "SOL per wX");
  // after a re-centre the bin moved up: a higher price, by exactly the bins' ratio
  const q = decodeLbPair(b64decode(dlmmAccount("lb_pair_rebalance_inside").data[0]));
  assert.equal(q.activeId, 600);
  near(solPerWx(q, WX)! / sol, 1.005 ** 14, 1e-9, "14 bins up");
  // the other orientation inverts; a pair that is not wX/wSOL, or is disabled, gives no price
  assert.equal(solPerWx({ ...p, tokenXMint: p.tokenYMint, tokenYMint: WX }, WX), 1 / sol);
  assert.equal(solPerWx({ ...p, tokenYMint: MINT }, WX), null);
  assert.equal(solPerWx({ ...p, status: 1 }, WX), null);
  assert.throws(() => decodeLbPair(b64decode(a.data[0]).subarray(1)), /not a DLMM LbPair/);
});

test("the canister's books_view bytes: the accepted floor is lamports per whole wX", () => {
  assert.equal(BOOKS_VIEW.coll, 0); assert.equal(BOOKS_VIEW.accepted, 200_000_000_000n); assert.equal(BOOKS_VIEW.fresh, true);
  assert.equal(BOOKS_VIEW.wxMint, WX2); assert.equal(BOOKS_VIEW.halted, null); assert.equal(BOOKS_VIEW.mintPrice, 200_320_000_000n);
  assert.equal(Number(BOOKS_VIEW.accepted) / 1e9, 200);
});

test("the follower: the pair's active bin when the pair exists, the accepted floor when it does not; nothing when neither", async () => {
  const db = openSolDb(":memory:");
  const rpc = fakeRpc({ accounts: { [CONFIG]: configAccount(WX), [CONFIG2]: configAccount(WX2), [WX]: mintAccount(9), [WX2]: mintAccount(9), [pairAddress(WX)]: dlmmAccount("lb_pair_open") } });
  const canister = fakeCanister([{ index: 0, wxMint: WX, config: CONFIG, module: DLMM.keys.module, symbol: "wMILADY", stage: "Open" }, { index: 1, wxMint: WX2, config: CONFIG2, module: "", symbol: "wAZUKI", stage: "Open" }],
    { 1: { ...BOOKS_VIEW, coll: 1 } });
  const ix = new SolIndexer({ db, rpc, configs: [{ slug: "milady", listingId: 0, config: CONFIG }, { slug: "azuki", listingId: 1, config: CONFIG2 }], http: false, canister, now: () => 1_791_620_000 });
  await ix.init();
  const cfgs = db.prepare(`SELECT config, quote_mint, quote_decimals, quote_symbol, coll, module, pair FROM configs ORDER BY listing_id`).all() as Record<string, unknown>[];
  assert.deepEqual({ ...cfgs[0] }, { config: CONFIG, quote_mint: WX, quote_decimals: 9, quote_symbol: "wMILADY", coll: 0, module: DLMM.keys.module, pair: DLMM.keys.pair });
  assert.equal(cfgs[1].coll, 1); assert.equal(cfgs[1].module, null); assert.equal(cfgs[1].pair, pairAddress(WX2));
  await ix.refreshWx();
  const rates = db.prepare(`SELECT mint, t, sol, src FROM wx_sol ORDER BY mint`).all() as { mint: string; t: number; sol: number; src: string }[];
  assert.equal(rates.length, 2);
  const byMint = new Map(rates.map((r) => [r.mint, r]));
  near(byMint.get(WX)!.sol, DLMM.keys.floor, 0.005); assert.equal(byMint.get(WX)!.src, "dlmm"); assert.equal(byMint.get(WX)!.t, 1_791_620_000 - (1_791_620_000 % 60));
  assert.equal(byMint.get(WX2)!.sol, 200); assert.equal(byMint.get(WX2)!.src, "books");
  assert.deepEqual(canister.asked, [1], "only the collection without a pair is asked for its floor");
  // a floor for another wX than the config's is not taken; no canister: no fallback at all
  const db2 = openSolDb(":memory:");
  const ix2 = new SolIndexer({ db: db2, rpc, configs: [{ slug: "azuki", listingId: 1, config: CONFIG2 }], http: false, canister: fakeCanister([{ index: 1, wxMint: WX2, config: CONFIG2, module: "", symbol: "wAZUKI", stage: "Open" }], { 1: { ...BOOKS_VIEW, coll: 1, wxMint: WX } }), now: () => 1_791_620_000 });
  await ix2.init(); await ix2.refreshWx();
  assert.equal((db2.prepare(`SELECT count(*) AS n FROM wx_sol`).get() as { n: number }).n, 0);
  const db3 = openSolDb(":memory:");
  const ix3 = new SolIndexer({ db: db3, rpc, configs: [{ slug: "azuki", listingId: 1, config: CONFIG2 }], http: false, now: () => 1_791_620_000 });
  await ix3.init(); await ix3.refreshWx();
  assert.equal((db3.prepare(`SELECT count(*) AS n FROM wx_sol`).get() as { n: number }).n, 0);
});

test("the rate store: latest within an hour, nearest within two hours for a time, never beyond", () => {
  const db = openSolDb(":memory:");
  storeWxRate(db, WX, 1000, 20, "dlmm"); storeWxRate(db, WX, 4000, 22, "dlmm"); storeWxRate(db, WX, 4000, NaN, "dlmm"); storeWxRate(db, WX, 5000, 0, "dlmm");
  assert.deepEqual(latestWxRate(db, WX, 4500), { sol: 22, at: 4000, src: "dlmm" });
  assert.equal(latestWxRate(db, WX, 4000 + 3601), null);
  assert.equal(latestWxRate(db, WX2, 4500), null);
  const f = wxRateLookup(db, WX, 0, 10_000);
  assert.equal(f(1100), 20); assert.equal(f(3000), 22); assert.equal(f(2400), 20); assert.equal(f(4000 + 7200), 22); assert.equal(f(4000 + 7201), null);
});

/** The recorded DBC trades (test/fixtures/sol) under a config the database says is quoted in wX: the indexer's maths is the same, the quote is read from the config row. */
function wxWorld() {
  const db = openSolDb(":memory:");
  db.prepare(`INSERT INTO configs (config, slug, listing_id, quote_mint, quote_decimals, quote_symbol, coll, module, pair, fee_mode, decimals, threshold) VALUES (?, 'milady', 0, ?, 9, 'wMILADY', 0, ?, ?, 1, 6, '7536904')`).run(CONFIG, WX, DLMM.keys.module, DLMM.keys.pair);
  const ix = new SolIndexer({ db, rpc: fakeRpc({}), configs: [{ slug: "milady", listingId: 0, config: CONFIG }], http: false });
  const create = decodeTx(fx("dbc-swap-5LAUFFX9zK")), migrate = decodeTx(fx("migrate-damm-v2-FQgKq1qWYL")), damm = decodeTx(fx("damm-swap-23RE4dZAqF"));
  ix.write({ address: CONFIG, kind: "config", mint: null }, [{ d: create, txIndex: 5 }, { d: migrate, txIndex: 9 }]);
  db.prepare(`UPDATE coins SET coin_is_a = 1, supply = '1000000000000000' WHERE mint = ?`).run(MINT);
  ix.write({ address: "F9xaKn1zZCaa1NTiC96VKTfmjnd6Vx2e1FrJqbedVm92", kind: "damm", mint: MINT }, [{ d: damm, txIndex: 2 }]);
  // 20 SOL per wX around both trades, 200 USD per SOL
  for (const t of [create.blockTime!, damm.blockTime!]) { storeWxRate(db, WX, Math.floor(t / 60) * 60, 20, "dlmm"); storeRate(db, Math.floor(t / 60) * 60, 200, "pyth"); }
  return { db, create, damm };
}

test("a wX-quoted coin: exact figures in wX; SOL and USD through the rate of each trade's time; the chart in all three quotes", () => {
  const { db, create, damm } = wxWorld();
  const now = damm.blockTime! + 30;
  const svc = new SolService(db, { now: () => now, ttlS: 0 });
  const get = (p: string) => { const u = new URL(p, "http://x"); return route({ svc, health: () => ({}) }, u.pathname, u); };
  assert.deepEqual(quoteOf({ quote_mint: WX, quote_decimals: 9, quote_symbol: "wMILADY" }), { kind: "wx", mint: WX, decimals: 9, symbol: "wMILADY" });

  const r = (get("/v1/rook/coins").body as { items: SolCoin[] }).items[0];
  assert.deepEqual(r.quote, { kind: "wx", mint: WX, decimals: 9, symbol: "wMILADY" }); assert.equal(r.wxSol, 20); assert.equal(r.collection.coll, 0);
  const last = db.prepare(`SELECT price, sol_f FROM trades ORDER BY ord DESC LIMIT 1`).get() as { price: number; sol_f: number };
  assert.equal(r.priceWx, last.price); assert.equal(r.price, last.price * 20); assert.equal(r.priceUsd, last.price * 20 * 200);
  assert.equal(r.marketCapWx, last.price * 1e9); assert.equal(r.marketCapSol, last.price * 1e9 * 20); assert.equal(r.marketCapUsd, last.price * 1e9 * 20 * 200);
  assert.equal(r.volume24hQuoteRaw, String(7536904 + 100000), "the quote's base units, exact");
  near(r.volume24hWx!, (7536904 + 100000) / 1e9, 1e-9); near(r.volume24hSol, ((7536904 + 100000) / 1e9) * 20, 1e-9); assert.equal(r.volume24hLamports, String(Math.round(r.volume24hSol * 1e9)));
  assert.equal(r.volume24hUsd, r.volume24hSol * 200);

  const ch = get(`/v1/rook/coins/${MINT}/chart?res=1h`).body as { quote: string; quoteSymbol: string; selected: string; candles: { c: number; v: number; n: number }[]; wx: { c: number; v: number }[]; sol: { c: number; v: number }[]; usd: { c: number }[]; wxSol: number };
  assert.equal(ch.quote, "wX"); assert.equal(ch.quoteSymbol, "wMILADY"); assert.equal(ch.selected, "wx"); assert.equal(ch.wxSol, 20);
  assert.deepEqual(ch.candles, ch.wx); assert.equal(ch.candles.reduce((a, c) => a + c.n, 0), 2);
  assert.equal(ch.sol.length, ch.wx.length); assert.equal(ch.usd.length, ch.wx.length);
  ch.wx.forEach((c, i) => { near(ch.sol[i].c, c.c * 20, 1e-12); near(ch.sol[i].v, c.v * 20, 1e-12); near(ch.usd[i].c, c.c * 20 * 200, 1e-12); });
  assert.deepEqual((get(`/v1/rook/coins/${MINT}/chart?res=1h&quote=sol`).body as { candles: unknown; sol: unknown; selected: string }).candles, ch.sol);
  assert.equal((get(`/v1/rook/coins/${MINT}/chart?res=1h&quote=usd`).body as { selected: string }).selected, "usd");
  assert.equal(get(`/v1/rook/coins/${MINT}/chart?res=1h&quote=eur`).status, 400);

  const tp = get(`/v1/rook/coins/${MINT}/trades`).body as { quote: { kind: string }; items: { quoteRaw: string; wx: number; priceWx: number; wxSol: number; sol: number; solLamports: string; price: number; priceUsd: number; feeRaw: string | null; feeLamports: string | null }[] };
  assert.equal(tp.quote.kind, "wx");
  const t0 = tp.items[0];
  assert.equal(t0.quoteRaw, "100000"); assert.equal(t0.wx, 1e-4); assert.equal(t0.wxSol, 20); assert.equal(t0.sol, 1e-4 * 20); assert.equal(t0.solLamports, String(Math.round(1e-4 * 20 * 1e9)));
  assert.equal(t0.price, t0.priceWx * 20); assert.equal(t0.priceUsd, t0.priceWx * 20 * 200);
  const act = get("/v1/rook/activity?limit=10").body as { items: { kind: string; text: string; quoteRaw: string | null }[] };
  assert.match(act.items.find((x) => x.kind === "buy")!.text, /wMILADY$/);
  const cols = get("/v1/rook/collections").body as { items: { slug: string; coll: number; quote: { kind: string }; wxSol: { sol: number; src: string }; module: string; pair: string; volume24hQuoteRaw: string; volume24hWx: number }[] };
  assert.equal(cols.items[0].coll, 0); assert.equal(cols.items[0].quote.kind, "wx"); assert.equal(cols.items[0].wxSol.sol, 20); assert.equal(cols.items[0].pair, DLMM.keys.pair);
  assert.equal(cols.items[0].volume24hQuoteRaw, r.volume24hQuoteRaw);

  // no rate near a trade's time: its SOL and USD figures are null, the wX ones stay; the chart's SOL series leaves it out
  // (the two recorded trades are 2 s apart: the curve trade is moved 10,000 s back, out of the rate's two-hour reach)
  db.exec(`DELETE FROM wx_sol`);
  db.prepare(`UPDATE trades SET at = at - 10000 WHERE venue = 'curve'`).run();
  storeWxRate(db, WX, Math.floor(damm.blockTime! / 60) * 60, 25, "books");
  const svc2 = new SolService(db, { now: () => now, ttlS: 0 });
  const tp2 = svc2.trades(MINT, 10, null)!;
  const curve = tp2.items.find((x) => x.venue === "curve")!, pool = tp2.items.find((x) => x.venue === "pool")!;
  assert.equal(curve.at, create.blockTime! - 10000);
  assert.equal(curve.sol, null); assert.equal(curve.price, null); assert.equal(curve.priceUsd, null); assert.equal(curve.solLamports, null); assert.ok(curve.priceWx! > 0);
  assert.equal(pool.wxSol, 25); assert.equal(pool.price, pool.priceWx! * 25);
  const ch2 = svc2.chart(MINT, "1h")!;
  assert.equal(ch2.wx!.reduce((a, c) => a + c.n, 0), 2); assert.equal(ch2.sol!.reduce((a, c) => a + c.n, 0), 1);
  const row = svc2.coin(MINT)!;
  assert.equal(row.wxSol, 25); near(row.volume24hSol, 1e-4 * 25, 1e-9, "only the trade with a rate counts in SOL"); assert.equal(row.volume24hQuoteRaw, String(7536904 + 100000));
  // a stale rate (over an hour old) is no current price: SOL figures null, wX figures stay
  const svc3 = new SolService(db, { now: () => damm.blockTime! + 3_601, ttlS: 0 });
  const row3 = svc3.coin(MINT)!;
  assert.equal(row3.wxSol, null); assert.equal(row3.price, null); assert.equal(row3.marketCapSol, null); assert.ok(row3.priceWx! > 0); assert.ok(row3.marketCapWx! > 0);
});
