/**
 * The API's answers, from the database alone (no RPC on a request). Shapes: docs/v9/indexer-solana.md §2 (the contract the site
 * builds against). Every number the database does not have is null; nothing is estimated.
 *
 * W13 (canister-only design §3, §5): a coin is quoted in its collection's wX. Every trade and candle is stored in that quote;
 * the SOL figures here go through the wX/SOL rate of the trade's own time (wx.ts: the desk's DLMM pair, else the canister's
 * accepted floor), and the dollar figures through SOL/USD after that. A SOL-quoted config (the pre-W13 shape) is the same code
 * with a rate of one. So `price`, `marketCapSol`, `volume24hSol`, `sol` keep meaning SOL, and a wX-quoted coin with no rate near
 * a trade's time has null there, while `priceWx`, `*Wx` and `*QuoteRaw` are the exact figures in the coin's own quote.
 */
import type { DatabaseSync } from "node:sqlite";
import { candles, type Candle, type Point } from "../candles.js";
import { NATIVE_MINT, priceFromSqrt } from "./dbc.js";
import { cursorOf, ordFromCursor } from "./trades.js";
import { latestRate, rateLookup } from "./usd.js";
import { latestWxRate, wxRateLookup, type WxRate } from "./wx.js";

export type Source = { kind: "live" | "fixture"; note: string };
export type SolPhase = "trading" | "graduating" | "graduated";
/** What a coin is priced in: its collection's wX (the design), or SOL (a config from before W13). */
export type Quote = { kind: "wx" | "sol"; mint: string; decimals: number; symbol: string };
export type SolCoin = {
  kind: "dbc"; address: string; mint: string; pool: string; dammPool: string | null; config: string;
  collection: { slug: string; listingId: number | null; coll: number | null }; nest: { slug: string; listingId: number | null; name: null; symbol: null };
  name: string; symbol: string; logoUrl: string | null; description: string | null; links: { website: string | null; twitter: string | null; telegram: string | null };
  metadataUri: string | null; creator: string; decimals: number; supplyRaw: string | null;
  phase: SolPhase; graduated: boolean; launchedAt: number | null; graduatedAt: number | null; lastTradeAt: number | null;
  curve: { raisedLamports: string; thresholdLamports: string; progress: number } | null;
  raise: { raisedLamports: string; raiseLamports: string } | null;
  quote: Quote; wxSol: number | null;
  priceWx: number | null; price: number | null; priceUsd: number | null; change24h: number | null;
  marketCapWx: number | null; marketCapSol: number | null; marketCapUsd: number | null;
  volume24hQuoteRaw: string; volume24hWx: number | null; volume24hLamports: string; volume24hSol: number; volume24hUsd: number | null;
  trades24h: number; buys24h: number; sells24h: number;
  holders: number | null; holdersAt: number | null;
};
export type SolTrade = { id: string; side: "buy" | "sell"; venue: "curve" | "pool"; quoteRaw: string; quoteAmount: number; wx: number | null; priceWx: number | null; wxSol: number | null;
  solLamports: string | null; sol: number | null; tokenRaw: string; tokens: number; feeRaw: string | null; feeLamports: string | null;
  price: number | null; priceUsd: number | null; trader: string; signature: string; tx: string; slot: number; at: number | null };
export type SolRes = "1m" | "5m" | "15m" | "1h" | "1d";
export const RES_S: Record<SolRes, number> = { "1m": 60, "5m": 300, "15m": 900, "1h": 3_600, "1d": 86_400 };
export const resOf = (s: string | null): SolRes | null => (s === "1m" || s === "5m" || s === "15m" || s === "1h" || s === "1d" ? s : null);
export type ChartQuote = "wx" | "sol" | "usd";
export const chartQuoteOf = (s: string | null): ChartQuote | null | undefined => (s === null ? null : s === "wx" || s === "sol" || s === "usd" ? s : undefined);
const DAY = 86_400;
export const SORTS = ["new", "graduating", "graduated", "trending", "mcap", "active"] as const;
export type Sort = (typeof SORTS)[number];
export type DeskEvent = { signature: string; slot: number; at: number | null; coll: number | null; collection: string | null; kind: string; linkId: number; intent: number | null;
  payer: string; memo: string; feeLamports: number | null; wxMint: string | null; wxDelta: string | null; wxDeltaUnits: number | null; payerLamports: string; module: string | null;
  moduleLamports: string | null; lamports: { address: string; delta: string }[]; tokens: { owner: string | null; mint: string; delta: string }[] };

type CoinDb = { mint: string; pool: string; config: string; creator: string | null; decimals: number; name: string | null; symbol: string | null; uri: string | null; image: string | null;
  description: string | null; website: string | null; twitter: string | null; telegram: string | null; supply: string | null; created_at: number | null; created_slot: number | null;
  quote_reserve: string | null; sqrt_price: string | null; is_migrated: number; completed_at: number | null; damm_pool: string | null; graduated_at: number | null; graduated_slot: number | null;
  graduated_tx: string | null; last_ord: number | null; last_price: number | null; last_trade_at: number | null; trades_n: number; holders: number | null; holders_at: number | null;
  holders_complete: number | null; holders_json: string | null; slug: string | null; listing_id: number | null; threshold: string | null;
  quote_mint: string | null; quote_decimals: number | null; quote_symbol: string | null; coll: number | null; module: string | null; pair: string | null };
type ConfigDb = { config: string; slug: string; listing_id: number | null; quote_mint: string | null; quote_decimals: number | null; quote_symbol: string | null; coll: number | null; module: string | null; pair: string | null };
type TradeDb = { sig: string; n: number; mint: string; ord: number; slot: number; at: number | null; side: "buy" | "sell"; venue: "curve" | "pool"; sol: string; sol_f: number;
  tokens: string; tokens_f: number; fee: string | null; price: number; trader: string };
type EventDb = { rowid: number; sig: string; n: number; slot: number; at: number | null; coll: number | null; kind: string; link_id: number; intent: number | null; payer: string; memo: string;
  fee: number | null; wx_mint: string | null; wx_delta: string | null; payer_lamports: string; module_lamports: string | null; deltas: string };
const CONFIG_COLS = "k.slug, k.listing_id, k.threshold, k.quote_mint, k.quote_decimals, k.quote_symbol, k.coll, k.module, k.pair";

const LIVE: Source = { kind: "live", note: "Read from Solana mainnet by Rook's indexer: Meteora DBC pools on Rook's configs (quoted in each collection's wX), their DAMM v2 pools after graduation, the desks' wX/SOL pairs, and the canister's memo-labelled transactions." };

/** The quote of a config row. A config whose account could not be read is taken as SOL-quoted until it is. */
export const quoteOf = (c: Pick<ConfigDb, "quote_mint" | "quote_decimals" | "quote_symbol">): Quote =>
  c.quote_mint && c.quote_mint !== NATIVE_MINT ? { kind: "wx", mint: c.quote_mint, decimals: c.quote_decimals ?? 9, symbol: c.quote_symbol || "wX" } : { kind: "sol", mint: NATIVE_MINT, decimals: 9, symbol: "SOL" };

export class SolService {
  private cache: { at: number; rows: SolCoin[]; byMint: Map<string, SolCoin> } | null = null;
  private readonly now: () => number;
  constructor(private readonly db: DatabaseSync, private readonly o: { now?: () => number; source?: Source; ttlS?: number; cluster?: string } = {}) {
    this.now = o.now ?? (() => Math.floor(Date.now() / 1000));
  }
  get source(): Source { return this.o.source ?? LIVE; }
  solUsd(): number | null { return latestRate(this.db, this.now())?.usd ?? null; }
  /** The latest wX/SOL rate of a quote (within an hour); one, at once, for SOL itself. */
  wxSolOf(q: Quote): WxRate | null { return q.kind === "sol" ? { sol: 1, at: this.now(), src: "sol" } : latestWxRate(this.db, q.mint, this.now()); }
  /** A per-time wX/SOL lookup over a window for a quote: one for SOL. */
  private wxLookup(q: Quote, from: number, to: number): (ts: number) => number | null { return q.kind === "sol" ? () => 1 : wxRateLookup(this.db, q.mint, from, to); }

  // ---- rows --------------------------------------------------------------------------------------------------------------------
  private coinsDb(where = "", args: (string | number)[] = []): CoinDb[] {
    return this.db.prepare(`SELECT c.*, ${CONFIG_COLS} FROM coins c LEFT JOIN configs k ON k.config = c.config ${where}`).all(...args) as CoinDb[];
  }
  private configsDb(): ConfigDb[] {
    return this.db.prepare(`SELECT config, slug, listing_id, quote_mint, quote_decimals, quote_symbol, coll, module, pair FROM configs ORDER BY listing_id IS NULL, listing_id`).all() as ConfigDb[];
  }
  rows(): { rows: SolCoin[]; byMint: Map<string, SolCoin> } {
    const now = this.now();
    if (this.cache && now - this.cache.at < (this.o.ttlS ?? 5)) return this.cache;
    const usd = this.solUsd(), since = now - DAY;
    // the day's volume per coin: exact in the quote; in SOL through each trade's own rate (a trade with no rate near it counts in the quote only)
    const quotes = new Map(this.configsDb().map((k) => [k.config, quoteOf(k)]));
    const lookups = new Map<string, (ts: number) => number | null>();
    const lookupOf = (config: string) => { let f = lookups.get(config); if (!f) { f = this.wxLookup(quotes.get(config) ?? quoteOf({ quote_mint: null, quote_decimals: null, quote_symbol: null }), since, now); lookups.set(config, f); } return f; };
    const day = new Map<string, { raw: bigint; q: number; sol: number; n: number; b: number }>();
    for (const t of this.db.prepare(`SELECT t.mint, t.at, t.sol, t.sol_f, t.side, c.config FROM trades t JOIN coins c ON c.mint = t.mint WHERE t.at >= ?`).all(since) as (Pick<TradeDb, "mint" | "at" | "sol" | "sol_f" | "side"> & { config: string })[]) {
      const d = day.get(t.mint) ?? { raw: 0n, q: 0, sol: 0, n: 0, b: 0 };
      const r = t.at === null ? null : lookupOf(t.config)(t.at);
      d.raw += BigInt(t.sol); d.q += t.sol_f; d.n++; if (t.side === "buy") d.b++; if (r !== null) d.sol += t.sol_f * r;
      day.set(t.mint, d);
    }
    const before = this.db.prepare(`SELECT price FROM trades WHERE mint = ? AND at < ? ORDER BY at DESC, ord DESC LIMIT 1`);
    const firstIn = this.db.prepare(`SELECT price FROM trades WHERE mint = ? AND at >= ? ORDER BY at, ord LIMIT 1`);
    const rates = new Map<string, WxRate | null>();
    const rows = this.coinsDb().map((c) => {
      const d = day.get(c.mint);
      let change: number | null = null;
      if (c.last_price !== null && c.last_trade_at !== null) {
        const ref = (before.get(c.mint, since) as { price: number } | undefined) ?? (d && d.n > 1 ? (firstIn.get(c.mint, since) as { price: number } | undefined) : undefined);
        if (ref && ref.price > 0) change = (c.last_price - ref.price) / ref.price;
      }
      const q = quoteOf(c);
      if (!rates.has(q.mint)) rates.set(q.mint, this.wxSolOf(q));
      return toCoin(c, { raw: d ? String(d.raw) : "0", quote: d?.q ?? 0, sol: d?.sol ?? 0, n: d?.n ?? 0, buys: d?.b ?? 0 }, change, usd, rates.get(q.mint)?.sol ?? null);
    });
    rows.sort((a, b) => (b.launchedAt ?? 0) - (a.launchedAt ?? 0) || (a.mint < b.mint ? -1 : 1));
    this.cache = { at: now, rows, byMint: new Map(rows.map((r) => [r.mint, r])) };
    return this.cache;
  }

  list(q: { sort?: string | null; collection?: string | null; q?: string | null; limit: number; offset: number }) {
    const sort: Sort = (SORTS as readonly string[]).includes(q.sort ?? "") ? (q.sort as Sort) : "new";
    let xs = this.rows().rows;
    if (q.collection) xs = xs.filter((r) => r.collection.slug === q.collection);
    if (q.q) { const s = q.q.trim().toLowerCase(); xs = xs.filter((r) => r.mint === q.q!.trim() || r.name.toLowerCase().includes(s) || r.symbol.toLowerCase().includes(s)); }
    xs = sortRows(xs, sort);
    return { source: this.source, at: this.now(), solUsd: this.solUsd(), sort, collection: q.collection ?? null, total: xs.length, limit: q.limit, offset: q.offset, items: xs.slice(q.offset, q.offset + q.limit) };
  }
  board() { return { source: this.source, at: this.now(), usdPerSol: this.solUsd(), rows: this.rows().rows.slice(0, 500) }; }

  coin(mint: string) {
    const r = this.rows().byMint.get(mint);
    if (!r) return null;
    const c = this.coinsDb("WHERE c.mint = ?", [mint])[0];
    return { ...r, tradesAllTime: c.trades_n, graduation: graduationOf(c), solUsd: this.solUsd() };
  }

  // ---- one coin ----------------------------------------------------------------------------------------------------------------
  /**
   * Candles in the coin's own quote, and in SOL and USD through the rates of each trade's time. `select` picks which series
   * `candles` carries (default: the coin's own quote, as before W13; `quote` says which that is). A series that cannot be
   * computed (no rate near any trade) is null rather than invented.
   */
  chart(mint: string, res: SolRes, select: ChartQuote | null = null) {
    const c = this.coinsDb("WHERE c.mint = ?", [mint])[0];
    if (!c) return null;
    const now = this.now(), seconds = RES_S[res], q = quoteOf(c);
    const span = res === "1d" ? Math.min(365 * DAY, Math.max(DAY, now - (c.created_at ?? now - 30 * DAY))) : seconds * 240;
    const since = now - span;
    const ts = this.db.prepare(`SELECT at, price, sol_f FROM trades WHERE mint = ? AND at >= ? ORDER BY ord`).all(mint, since) as { at: number; price: number; sol_f: number }[];
    const pts: Point[] = ts.map((t) => ({ ts: t.at, p: t.price, v: t.sol_f }));
    const wx = this.wxLookup(q, since, now), usd = rateLookup(this.db, since, now);
    const spts: Point[] = [], upts: Point[] = [];
    for (const p of pts) {
      const r = wx(p.ts);
      if (r === null) continue;
      spts.push({ ts: p.ts, p: p.p * r, v: p.v * r });
      const u = usd(p.ts);
      if (u !== null) upts.push({ ts: p.ts, p: p.p * r * u, v: p.v * r * u });
    }
    const series = { wx: q.kind === "wx" ? (candles(pts, seconds) as Candle[]) : null, sol: q.kind === "sol" ? (candles(pts, seconds) as Candle[]) : spts.length ? (candles(spts, seconds) as Candle[]) : null,
      usd: upts.length ? (candles(upts, seconds) as Candle[]) : null };
    const selected: ChartQuote = select ?? (q.kind === "wx" ? "wx" : "sol");
    return { source: this.source, kind: "dbc" as const, coin: mint, res, seconds, span, quote: q.kind === "wx" ? ("wX" as const) : ("SOL" as const), quoteMint: q.mint, quoteSymbol: q.symbol,
      selected, candles: series[selected] ?? [], ...series, trades: pts.length, tradesAllTime: c.trades_n, launchedAt: c.created_at,
      graduation: graduationOf(c), supplyRaw: c.supply, decimals: c.decimals, wxSol: this.wxSolOf(q)?.sol ?? null, usdPerSol: this.solUsd() };
  }
  trades(mint: string, limit: number, before: string | null) {
    const c = this.coinsDb("WHERE c.mint = ?", [mint])[0];
    if (!c) return null;
    const ord = before ? ordFromCursor(before) : null;
    const xs = (ord !== null
      ? this.db.prepare(`SELECT * FROM trades WHERE mint = ? AND ord < ? ORDER BY ord DESC LIMIT ?`).all(mint, ord, limit + 1)
      : this.db.prepare(`SELECT * FROM trades WHERE mint = ? ORDER BY ord DESC LIMIT ?`).all(mint, limit + 1)) as TradeDb[];
    const page = xs.slice(0, limit), q = quoteOf(c), rate = this.ratesFor(page, q);
    return { source: this.source, kind: "dbc" as const, coin: mint, quote: q, items: page.map((t) => toTrade(t, q, rate)), next: xs.length > limit ? cursorOf(page[page.length - 1].ord) : null, total: c.trades_n };
  }
  private ratesFor(ts: { at: number | null }[], q: Quote): Rates {
    const ats = ts.map((t) => t.at).filter((x): x is number => x !== null);
    if (!ats.length) return { usd: () => null, wx: () => null };
    const from = Math.min(...ats), to = Math.max(...ats);
    return { usd: rateLookup(this.db, from, to), wx: this.wxLookup(q, from, to) };
  }
  holders(mint: string, limit: number, offset: number) {
    const c = this.coinsDb("WHERE c.mint = ?", [mint])[0];
    if (!c) return null;
    const all = c.holders_json ? (JSON.parse(c.holders_json) as { address: string; tokenAccount: string; amount: string; label: "creator" | "curve" | "pool" | null }[]) : [];
    const supply = c.supply ? BigInt(c.supply) : null;
    const share = (v: bigint) => (supply === null || supply === 0n ? null : Number((v * 100_000_000n) / supply) / 1e8);
    return { source: this.source, kind: "dbc" as const, coin: mint, holders: c.holders, supplyRaw: c.supply, decimals: c.decimals, complete: c.holders_complete === 1,
      refreshedAt: c.holders_at, items: all.slice(offset, offset + limit).map((h, i) => ({ rank: offset + i + 1, address: h.address, tokenAccount: h.tokenAccount, amountRaw: h.amount,
        amount: Number(h.amount) / 10 ** c.decimals, share: share(BigInt(h.amount)), label: h.label })), total: all.length, limit, offset };
  }

  // ---- across coins ------------------------------------------------------------------------------------------------------------
  activity(limit: number) {
    const { byMint } = this.rows();
    const coinOf = (m: string) => { const r = byMint.get(m); return r ? { address: r.mint, symbol: r.symbol, name: r.name, logoUrl: r.logoUrl, collection: r.collection.slug, quote: r.quote } : null; };
    const items: { at: number | null; kind: "buy" | "sell" | "launch" | "graduated"; coin: NonNullable<ReturnType<typeof coinOf>>; text: string; quoteRaw: string | null; solLamports: string | null; trader: string | null; tx: string | null }[] = [];
    const ts = this.db.prepare(`SELECT * FROM trades ORDER BY ord DESC LIMIT ?`).all(limit) as TradeDb[];
    const rates = new Map<string, Rates>();
    for (const t of ts) {
      const c = coinOf(t.mint); if (!c) continue;
      if (!rates.has(c.quote.mint)) rates.set(c.quote.mint, this.ratesFor(ts, c.quote));
      const x = toTrade(t, c.quote, rates.get(c.quote.mint)!);
      items.push({ at: t.at, kind: t.side, coin: c, text: `${t.side === "buy" ? "bought" : "sold"} ${c.symbol} for ${t.sol_f.toFixed(4)} ${c.quote.symbol}`, quoteRaw: t.sol, solLamports: x.solLamports, trader: t.trader, tx: t.sig });
    }
    for (const r of this.db.prepare(`SELECT mint, created_at, created_tx, creator FROM coins WHERE created_at IS NOT NULL ORDER BY created_at DESC LIMIT ?`).all(limit) as { mint: string; created_at: number; created_tx: string | null; creator: string | null }[]) {
      const c = coinOf(r.mint); if (c) items.push({ at: r.created_at, kind: "launch", coin: c, text: `${c.symbol} launched on ${c.collection}`, quoteRaw: null, solLamports: null, trader: r.creator, tx: r.created_tx });
    }
    for (const r of this.db.prepare(`SELECT mint, graduated_at, graduated_tx FROM coins WHERE graduated_at IS NOT NULL ORDER BY graduated_at DESC LIMIT ?`).all(limit) as { mint: string; graduated_at: number; graduated_tx: string | null }[]) {
      const c = coinOf(r.mint); if (c) items.push({ at: r.graduated_at, kind: "graduated", coin: c, text: `${c.symbol} graduated to its pool`, quoteRaw: null, solLamports: null, trader: null, tx: r.graduated_tx });
    }
    items.sort((a, b) => (b.at ?? 0) - (a.at ?? 0));
    return { source: this.source, at: this.now(), items: items.slice(0, limit) };
  }
  collections() {
    const rows = this.rows().rows;
    return { source: this.source, at: this.now(), solUsd: this.solUsd(), items: this.configsDb().map((k) => {
      const xs = rows.filter((r) => r.config === k.config), q = quoteOf(k), rate = this.wxSolOf(q);
      const top = [...xs].sort((a, b) => (b.marketCapSol ?? -1) - (a.marketCapSol ?? -1))[0] ?? null;
      const raw = xs.reduce((a, r) => a + BigInt(r.volume24hQuoteRaw), 0n), sol = xs.reduce((a, r) => a + r.volume24hSol, 0);
      return { slug: k.slug, listingId: k.listing_id, coll: k.coll, config: k.config, quote: q, wxSol: q.kind === "wx" ? rate : null, module: k.module, pair: k.pair,
        coins: xs.length, graduated: xs.filter((r) => r.graduated).length,
        volume24hQuoteRaw: String(raw), volume24hWx: q.kind === "wx" ? Number(raw) / 10 ** q.decimals : null, volume24hLamports: String(Math.round(sol * 1e9)), volume24hSol: sol,
        marketCapSol: xs.reduce((a, r) => a + (r.marketCapSol ?? 0), 0),
        topCoin: top ? { mint: top.mint, symbol: top.symbol, marketCapSol: top.marketCapSol } : null };
    }) };
  }
  wallet(address: string) {
    const { rows, byMint } = this.rows();
    const ts = this.db.prepare(`SELECT * FROM trades WHERE trader = ? ORDER BY ord DESC LIMIT 50`).all(address) as TradeDb[];
    const rates = new Map<string, Rates>();
    return { source: this.source, address, at: this.now(), launches: rows.filter((r) => r.creator === address),
      trades: ts.flatMap((t) => {
        const c = byMint.get(t.mint); if (!c) return [];
        if (!rates.has(c.quote.mint)) rates.set(c.quote.mint, this.ratesFor(ts, c.quote));
        return [{ ...toTrade(t, c.quote, rates.get(c.quote.mint)!), coin: { address: c.mint, symbol: c.symbol, name: c.name, logoUrl: c.logoUrl, quote: c.quote } }];
      }) };
  }

  // ---- the desk (W13: the canister's memo-labelled transactions) ----------------------------------------------------------------
  /** A collection by its canister index or its slug: null when unknown here. */
  collOf(s: string): number | null {
    const ks = this.configsDb();
    const k = /^\d{1,5}$/.test(s) ? ks.find((x) => x.coll === Number(s)) : ks.find((x) => x.slug === s);
    return k?.coll ?? (/^\d{1,5}$/.test(s) ? Number(s) : null);
  }
  /** Newest first, cursor-paged by (slot, rowid). `since` is a unix time; `coll` the canister's index (or `-1` for memos naming no collection). */
  deskEvents(q: { coll: number | null; kind: string | null; since: number | null; limit: number; before: string | null }) {
    const where: string[] = ["1"], args: (number | string)[] = [];
    if (q.coll !== null) { if (q.coll < 0) where.push(`coll IS NULL`); else { where.push(`coll = ?`); args.push(q.coll); } }
    if (q.kind) { where.push(`kind = ?`); args.push(q.kind); }
    if (q.since !== null) { where.push(`at >= ?`); args.push(q.since); }
    const total = (this.db.prepare(`SELECT count(*) AS n FROM desk_events WHERE ${where.join(" AND ")}`).get(...args) as { n: number }).n;
    const cur = q.before ? /^(\d{1,12}):(\d{1,12})$/.exec(q.before) : null;
    if (cur) { where.push(`(slot < ? OR (slot = ? AND rowid < ?))`); args.push(Number(cur[1]), Number(cur[1]), Number(cur[2])); }
    const xs = this.db.prepare(`SELECT rowid, * FROM desk_events WHERE ${where.join(" AND ")} ORDER BY slot DESC, rowid DESC LIMIT ?`).all(...args, q.limit + 1) as EventDb[];
    const page = xs.slice(0, q.limit), ks = this.configsDb();
    return { source: this.source, at: this.now(), coll: q.coll, kind: q.kind, since: q.since, items: page.map((e) => toEvent(e, ks)), next: xs.length > q.limit ? `${page[page.length - 1].slot}:${page[page.length - 1].rowid}` : null, total };
  }
  /**
   * Per collection: how many events of each kind, and the sums of what the balances said: wX minted and burned (net supply change,
   * from token balances), lamports out of and into the fee payer, out of and into the module `M[i]` when it is known. Sums of
   * labels are not books; they are what Solana history shows under the canister's labels, for the reconciler to compare.
   */
  deskTotals(coll: number | null) {
    const ks = this.configsDb();
    const rows = (coll === null ? this.db.prepare(`SELECT * FROM desk_events`).all() : coll < 0 ? this.db.prepare(`SELECT * FROM desk_events WHERE coll IS NULL`).all() : this.db.prepare(`SELECT * FROM desk_events WHERE coll = ?`).all(coll)) as EventDb[];
    type Tot = { coll: number | null; collection: string | null; wxMint: string | null; wxSymbol: string | null; wxDecimals: number; module: string | null; events: number; byKind: Record<string, number>;
      wxMinted: bigint; wxBurned: bigint; payerOut: bigint; payerIn: bigint; moduleOut: bigint; moduleIn: bigint; firstAt: number | null; lastAt: number | null; lastSlot: number | null };
    const by = new Map<number | null, Tot>();
    for (const e of rows) {
      let t = by.get(e.coll);
      if (!t) {
        const k = e.coll === null ? undefined : ks.find((x) => x.coll === e.coll), q = k ? quoteOf(k) : null;
        t = { coll: e.coll, collection: k?.slug ?? null, wxMint: q?.kind === "wx" ? q.mint : null, wxSymbol: q?.kind === "wx" ? q.symbol : null, wxDecimals: q?.decimals ?? 9, module: k?.module ?? null,
          events: 0, byKind: {}, wxMinted: 0n, wxBurned: 0n, payerOut: 0n, payerIn: 0n, moduleOut: 0n, moduleIn: 0n, firstAt: null, lastAt: null, lastSlot: null };
        by.set(e.coll, t);
      }
      t.events++; t.byKind[e.kind] = (t.byKind[e.kind] ?? 0) + 1;
      const w = e.wx_delta === null ? 0n : BigInt(e.wx_delta), p = BigInt(e.payer_lamports), m = e.module_lamports === null ? 0n : BigInt(e.module_lamports);
      if (w > 0n) t.wxMinted += w; else t.wxBurned -= w;
      if (p < 0n) t.payerOut -= p; else t.payerIn += p;
      if (m < 0n) t.moduleOut -= m; else t.moduleIn += m;
      if (e.at !== null) { t.firstAt = t.firstAt === null ? e.at : Math.min(t.firstAt, e.at); t.lastAt = t.lastAt === null ? e.at : Math.max(t.lastAt, e.at); }
      t.lastSlot = Math.max(t.lastSlot ?? 0, e.slot);
    }
    const items = [...by.values()].sort((a, b) => (a.coll ?? 1e9) - (b.coll ?? 1e9)).map((t) => ({ ...t, wxMinted: String(t.wxMinted), wxBurned: String(t.wxBurned), wxNet: String(t.wxMinted - t.wxBurned),
      wxNetUnits: Number(t.wxMinted - t.wxBurned) / 10 ** t.wxDecimals, payerOut: String(t.payerOut), payerIn: String(t.payerIn), moduleOut: String(t.moduleOut), moduleIn: String(t.moduleIn) }));
    return { source: this.source, at: this.now(), coll, items };
  }
  counts() {
    return { coins: (this.db.prepare(`SELECT count(*) AS n FROM coins`).get() as { n: number }).n, trades: (this.db.prepare(`SELECT count(*) AS n FROM trades`).get() as { n: number }).n,
      deskEvents: (this.db.prepare(`SELECT count(*) AS n FROM desk_events`).get() as { n: number }).n };
  }
}

// ---- pure pieces -------------------------------------------------------------------------------------------------------------------
type Rates = { usd: (ts: number) => number | null; wx: (ts: number) => number | null };
export function phaseOf(c: Pick<CoinDb, "damm_pool" | "is_migrated" | "completed_at" | "quote_reserve" | "threshold">): SolPhase {
  if (c.damm_pool || c.is_migrated) return "graduated";
  if (c.completed_at !== null || (c.quote_reserve && c.threshold && BigInt(c.threshold) > 0n && BigInt(c.quote_reserve) >= BigInt(c.threshold))) return "graduating";
  return "trading";
}
export function progressOf(raised: string | null, threshold: string | null): number {
  if (!raised || !threshold || BigInt(threshold) === 0n) return 0;
  return Math.min(1, Number((BigInt(raised) * 10_000n) / BigInt(threshold)) / 10_000);
}
const graduationOf = (c: CoinDb) => (c.graduated_slot !== null || c.graduated_at !== null ? { at: c.graduated_at, slot: c.graduated_slot ?? 0, tx: c.graduated_tx ?? "", pool: c.damm_pool } : null);
const mul = (a: number | null, b: number | null): number | null => (a !== null && b !== null ? a * b : null);

/** `d` is the day's volume: exact base units of the quote, whole quote units, and SOL through each trade's rate. `wxSol` is the latest rate (1 for a SOL quote). */
export function toCoin(c: CoinDb, d: { raw: string; quote: number; sol: number; n: number; buys: number }, change24h: number | null, usd: number | null, wxSol: number | null): SolCoin {
  const phase = phaseOf(c), graduated = phase === "graduated", q = quoteOf(c);
  const priceQ = c.last_price ?? (c.sqrt_price && c.sqrt_price !== "0" && !graduated ? priceFromSqrt(BigInt(c.sqrt_price), c.decimals, q.decimals) : null);
  const supply = c.supply ? Number(c.supply) / 10 ** c.decimals : null;
  const capQ = priceQ !== null && supply !== null ? priceQ * supply : null;
  const price = mul(priceQ, wxSol), cap = mul(capQ, wxSol);
  const raised = c.quote_reserve ?? "0", threshold = c.threshold ?? "0";
  const slug = c.slug ?? "unknown";
  return {
    kind: "dbc", address: c.mint, mint: c.mint, pool: c.pool, dammPool: c.damm_pool, config: c.config,
    collection: { slug, listingId: c.listing_id, coll: c.coll }, nest: { slug, listingId: c.listing_id, name: null, symbol: null },
    name: c.name || "unnamed", symbol: c.symbol || "?", logoUrl: c.image, description: c.description, links: { website: c.website, twitter: c.twitter, telegram: c.telegram },
    metadataUri: c.uri, creator: c.creator ?? "", decimals: c.decimals, supplyRaw: c.supply,
    phase, graduated, launchedAt: c.created_at, graduatedAt: c.graduated_at, lastTradeAt: c.last_trade_at,
    curve: graduated ? null : { raisedLamports: raised, thresholdLamports: threshold, progress: progressOf(raised, threshold) },
    raise: graduated ? null : { raisedLamports: raised, raiseLamports: threshold },
    quote: q, wxSol: q.kind === "wx" ? wxSol : null,
    priceWx: q.kind === "wx" ? priceQ : null, price, priceUsd: mul(price, usd), change24h,
    marketCapWx: q.kind === "wx" ? capQ : null, marketCapSol: cap, marketCapUsd: mul(cap, usd),
    volume24hQuoteRaw: d.raw, volume24hWx: q.kind === "wx" ? d.quote : null, volume24hLamports: q.kind === "sol" ? d.raw : String(Math.round(d.sol * 1e9)), volume24hSol: d.sol, volume24hUsd: mul(d.sol, usd),
    trades24h: d.n, buys24h: d.buys, sells24h: d.n - d.buys,
    holders: c.holders, holdersAt: c.holders_at,
  };
}
/** A trade in its quote (exact), and in SOL and USD through the rates of its time; a wX trade with no rate near it has null SOL figures. */
export function toTrade(t: TradeDb, q: Quote, rate: Rates): SolTrade {
  const r = t.at !== null ? rate.wx(t.at) : null, u = t.at !== null ? rate.usd(t.at) : null;
  const sol = mul(t.sol_f, r), price = mul(t.price, r);
  return { id: `${t.sig}:${t.n}`, side: t.side, venue: t.venue, quoteRaw: t.sol, quoteAmount: t.sol_f, wx: q.kind === "wx" ? t.sol_f : null, priceWx: q.kind === "wx" ? t.price : null, wxSol: q.kind === "wx" ? r : null,
    solLamports: q.kind === "sol" ? t.sol : sol === null ? null : String(Math.round(sol * 1e9)), sol, tokenRaw: t.tokens, tokens: t.tokens_f,
    feeRaw: t.fee, feeLamports: q.kind === "sol" ? t.fee : t.fee !== null && r !== null ? String(Math.round((Number(t.fee) / 10 ** q.decimals) * r * 1e9)) : null,
    price, priceUsd: mul(price, u), trader: t.trader, signature: t.sig, tx: t.sig, slot: t.slot, at: t.at };
}
export function toEvent(e: EventDb, ks: ConfigDb[]): DeskEvent {
  const k = e.coll === null ? undefined : ks.find((x) => x.coll === e.coll);
  const d = JSON.parse(e.deltas) as { lamports: [string, string][]; tokens: [string | null, string, string][] };
  return { signature: e.sig, slot: e.slot, at: e.at, coll: e.coll, collection: k?.slug ?? null, kind: e.kind, linkId: e.link_id, intent: e.intent, payer: e.payer, memo: e.memo, feeLamports: e.fee,
    wxMint: e.wx_mint, wxDelta: e.wx_delta, wxDeltaUnits: e.wx_delta === null ? null : Number(e.wx_delta) / 10 ** (k?.quote_decimals ?? 9), payerLamports: e.payer_lamports, module: k?.module ?? null,
    moduleLamports: e.module_lamports, lamports: d.lamports.map(([address, delta]) => ({ address, delta })), tokens: d.tokens.map(([owner, mint, delta]) => ({ owner, mint, delta })) };
}
export function sortRows(xs: SolCoin[], sort: Sort): SolCoin[] {
  const by = (f: (r: SolCoin) => number) => [...xs].sort((a, b) => f(b) - f(a) || (b.launchedAt ?? 0) - (a.launchedAt ?? 0));
  switch (sort) {
    case "graduating": return by((r) => r.curve?.progress ?? -1).filter((r) => !r.graduated);
    case "graduated": return by((r) => r.graduatedAt ?? 0).filter((r) => r.graduated);
    case "trending": return [...xs].sort((a, b) => b.volume24hSol - a.volume24hSol || b.trades24h - a.trades24h || (b.launchedAt ?? 0) - (a.launchedAt ?? 0));
    case "mcap": return by((r) => r.marketCapSol ?? -1);
    case "active": return by((r) => r.lastTradeAt ?? 0);
    default: return by((r) => r.launchedAt ?? 0);
  }
}
