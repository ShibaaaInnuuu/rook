import type { DatabaseSync } from "node:sqlite";
import { getAddress, type Abi, type Hex } from "viem";
import type * as T from "../../types.js";
import { rateOf } from "../usd.js";
import { tryRead, type Rpc } from "../rpc.js";
import type { Views } from "../views.js";
import type { WrappedConfig } from "../wrapped/config.js";
import type { WrappedService } from "../wrapped/service.js";
import { V4_4663 } from "../wrapped/config.js";
import { CURVE_VIEWS, ERC20_VIEWS, FACTORY_VIEWS } from "./abis.js";
import { chartOf, curveRow, graduationItem, launchItem, marketOf, nestItems, newestFirst, ponsRow, tradeItems, DAY } from "./board.js";
import { CurveReader } from "./curve.js";
import type { Activity, Board, BoardRow, RookChart, RookHolderPage, RookRes, RookSource, RookTrade, RookTradePage, WalletCoin, WalletProfile } from "./types.js";

/**
 * docs/v6/site-design.md §6: the board and the coin page's chart, tape and holders for BOTH kinds of coin, in one place.
 * A pons coin is answered from the database the follower fills (views.ts is delegated to where it already answers); a Rook
 * coin is read from its own logs through CurveReader. Nothing is estimated: a number the source does not have is null.
 */
export type RookDeps = { db?: DatabaseSync | null; views?: Views | null; wrapped?: WrappedService | null; wrappedConfig?: WrappedConfig | null; rpc?: Rpc | null;
  now?: () => number; source?: RookSource; ttlS?: number; log?: (o: Record<string, unknown>) => void };
const LIVE: RookSource = { kind: "live", note: "Read from Robinhood Chain by this indexer: the database it follows for coins launched through the Launcher, each Rook coin's own logs for the rest." };
const BOARD_PAGE = 200;

type CurveEntry = { slug: string; col: T.WrappedCollection; coin: T.WrappedCoin; feeBps: number | null; supplyWei: string | null };

export class RookService {
  private readonly reader: CurveReader | null;
  private readonly now: () => number;
  private board_: { at: number; board: Board } | null = null;
  private curveIndex: { at: number; byCoin: Map<string, CurveEntry> } | null = null;
  constructor(private readonly d: RookDeps) {
    this.now = d.now ?? (() => Math.floor(Date.now() / 1000));
    const conf = d.wrappedConfig?.confirmations;
    this.reader = d.rpc ? new CurveReader(d.rpc, { chunk: d.wrappedConfig?.logChunk, confirmations: conf ?? 0, now: this.now }) : null;
  }
  private get source(): RookSource { return this.d.source ?? LIVE; }
  private get ttl(): number { return this.d.ttlS ?? 15; }

  // ---- the Rook coins: which collection lists which coin, and each coin's fixed terms ----------------------------------------
  private async curves(): Promise<Map<string, CurveEntry>> {
    if (this.curveIndex && this.now() - this.curveIndex.at < 60) return this.curveIndex.byCoin;
    const byCoin = new Map<string, CurveEntry>();
    const w = this.d.wrapped, cfg = this.d.wrappedConfig;
    if (w && cfg) for (const c of cfg.collections) {
      const col = await w.collection(c.slug).catch(() => null);
      if (!col) continue;
      for (const coin of col.coinsList) {
        const key = coin.address.toLowerCase(), old = this.curveIndex?.byCoin.get(key);
        let feeBps = old?.feeBps ?? null, supplyWei = old?.supplyWei ?? null;
        if (this.d.rpc && (feeBps === null || supplyWei === null)) {
          const [fee, fee2, supply] = await Promise.all([
            c.factory ? tryRead<bigint>(this.d.rpc, c.factory, FACTORY_VIEWS as Abi, "feeOf", [coin.address]) : null,
            tryRead<bigint>(this.d.rpc, coin.curve, CURVE_VIEWS as Abi, "feeBps"),
            tryRead<bigint>(this.d.rpc, coin.address, ERC20_VIEWS as Abi, "totalSupply")]);
          feeBps = fee !== null ? Number(fee) : fee2 !== null ? Number(fee2) : null;
          supplyWei = supply !== null ? String(supply) : null;
        }
        byCoin.set(key, { slug: c.slug, col, coin, feeBps, supplyWei });
      }
    }
    this.curveIndex = { at: this.now(), byCoin };
    return byCoin;
  }
  private curveCoin(e: CurveEntry) {
    const c = this.d.wrappedConfig!.collections.find((x) => x.slug === e.slug)!;
    return { coin: e.coin.address as Hex, curve: e.coin.curve as Hex, fromBlock: c.fromBlock, lock: (c.lock ?? null) as Hex | null, poolManager: (e.col.contracts.poolManager ?? V4_4663.poolManager) as Hex };
  }
  private holdersOf(balances: Map<string, bigint>, e: CurveEntry, firstIn?: Map<string, number | null>, since?: number): { holders: number; newHolders: number | null; rows: { address: string; balance: bigint }[]; excluded: Set<string> } {
    const c = this.d.wrappedConfig!.collections.find((x) => x.slug === e.slug)!;
    const excluded = new Set([e.coin.curve, c.lock, c.router, c.queue, e.col.contracts.poolManager, e.coin.address].filter((x) => !!x).map((x) => String(x).toLowerCase()));
    const rows = [...balances.entries()].filter(([a, v]) => v > 0n && !excluded.has(a)).map(([address, balance]) => ({ address, balance })).sort((x, y) => (y.balance > x.balance ? 1 : y.balance < x.balance ? -1 : x.address.localeCompare(y.address)));
    const newHolders = firstIn && since !== undefined ? rows.filter((r) => { const t = firstIn.get(r.address); return t !== null && t !== undefined && t >= since; }).length : null;
    return { holders: rows.length, newHolders, rows, excluded };
  }
  /** docs/v6/site-design.md §6.2: a pons coin's holder growth, from its Transfer events — the first inbound transfer per address, kept per coin and read incrementally. */
  private readonly firstIns = new Map<string, { upTo: number; map: Map<string, number | null> }>();
  private ponsNewHolders(coin: string, now: number): number | null {
    const db = this.d.db!;
    const st = this.firstIns.get(coin) ?? { upTo: -1, map: new Map<string, number | null>() };
    const rows = db.prepare(`SELECT block_number, args, ts FROM events WHERE address = ? AND name = 'Transfer' AND block_number > ? ORDER BY block_number, log_index`).all(coin, st.upTo) as { block_number: number; args: string; ts: number | null }[];
    for (const r of rows) { const to = String((JSON.parse(r.args) as { to?: string }).to ?? "").toLowerCase(); if (to && !st.map.has(to)) st.map.set(to, r.ts); st.upTo = r.block_number; }
    this.firstIns.set(coin, st);
    if (this.firstIns.size > 5_000) this.firstIns.clear();
    const holders = db.prepare(`SELECT holder FROM balances WHERE coin = ? AND amt > 0`).all(coin) as { holder: string }[];
    let n = 0;
    for (const h of holders) { const t = st.map.get(h.holder); if (t !== undefined && t !== null && t >= now - DAY) n++; }
    return n;
  }

  // ---- the board ----------------------------------------------------------------------------------------------------------
  async board(): Promise<Board> {
    if (this.board_ && this.now() - this.board_.at < this.ttl) return this.board_.board;
    const now = this.now(), rows: BoardRow[] = [];
    if (this.d.views && this.d.db) {
      for (const c of this.d.views.coins(BOARD_PAGE, 0).items) rows.push(ponsRow(c, this.ponsMarket(c.address, now)));
    }
    for (const e of (await this.curves()).values()) {
      const r = this.reader ? await this.reader.read(this.curveCoin(e)).catch(() => null) : null;
      const hs = r ? this.holdersOf(r.balances, e, r.firstIn, now - DAY) : null;
      rows.push(curveRow(e.col, e.coin, r && hs ? { trades: r.trades, holders: hs.holders, newHolders24h: hs.newHolders, graduatedAt: r.graduation?.at ?? null, feeBps: e.feeBps, supplyWei: e.supplyWei } : { trades: [], holders: null, newHolders24h: null, graduatedAt: null, feeBps: e.feeBps, supplyWei: e.supplyWei }, now));
    }
    const usd = this.d.db ? rateOf(this.d.db, now).usdPerEth : null;
    const board: Board = { source: this.source, at: now, usdPerEth: usd, rows };
    this.board_ = { at: now, board };
    return board;
  }
  private ponsMarket(address: string, now: number): { price: number | null; change24h: number | null; trades24h: number | null; buys24h: number | null; newHolders24h: number | null; graduatedAt: number | null } {
    const db = this.d.db!, coin = address.toLowerCase();
    const last = db.prepare(`SELECT price FROM trades WHERE coin = ? ORDER BY block DESC, log_index DESC LIMIT 1`).get(coin) as { price: number } | undefined;
    const ago = db.prepare(`SELECT price FROM trades WHERE coin = ? AND ts < ? ORDER BY block DESC, log_index DESC LIMIT 1`).get(coin, now - DAY) as { price: number } | undefined;
    const first = db.prepare(`SELECT price FROM trades WHERE coin = ? AND ts >= ? ORDER BY block, log_index LIMIT 1`).get(coin, now - DAY) as { price: number } | undefined;
    const n = (db.prepare(`SELECT count(*) AS n FROM trades WHERE coin = ? AND ts >= ?`).get(coin, now - DAY) as { n: number }).n;
    const buys = (db.prepare(`SELECT count(*) AS n FROM trades WHERE coin = ? AND ts >= ? AND side = 'buy'`).get(coin, now - DAY) as { n: number }).n;
    const g = db.prepare(`SELECT graduated_ts FROM coins WHERE coin = ?`).get(coin) as { graduated_ts: number | null } | undefined;
    const ref = ago ?? (n > 1 ? first : undefined);
    return { price: last?.price ?? null, change24h: last && ref && ref.price > 0 ? (last.price - ref.price) / ref.price : null, trades24h: n, buys24h: buys, newHolders24h: this.ponsNewHolders(coin, now), graduatedAt: g?.graduated_ts ?? null };
  }

  // ---- one coin: the chart, the tape, the holders ------------------------------------------------------------------------------
  private async which(address: string): Promise<{ kind: "pons" } | { kind: "curve"; e: CurveEntry } | null> {
    const key = address.toLowerCase();
    const e = (await this.curves()).get(key);
    if (e) return { kind: "curve", e };
    if (this.d.views?.coinExists(key)) return { kind: "pons" };
    return null;
  }
  async chart(address: string, res: RookRes): Promise<RookChart | null> {
    const w = await this.which(address);
    if (!w) return null;
    const now = this.now(), coin = getAddress(address) as Hex;
    if (w.kind === "pons") {
      const db = this.d.db!, key = address.toLowerCase(), c = chartOf(this.ponsTrades(key, now - spanOfRes(res)), res, now);
      const row = db.prepare(`SELECT c.ts, c.graduated_ts, c.graduated_block, c.graduated_tx, m.supply FROM coins c LEFT JOIN coin_meta m ON m.coin = c.coin WHERE c.coin = ?`).get(key) as { ts: number | null; graduated_ts: number | null; graduated_block: number | null; graduated_tx: string | null; supply: string | null };
      const all = (db.prepare(`SELECT count(*) AS n FROM trades WHERE coin = ?`).get(key) as { n: number }).n;
      return { source: this.source, kind: "pons", coin, res, seconds: c.seconds, span: c.span, quote: "ETH", candles: c.candles, trades: c.trades, tradesAllTime: all, launchedAt: row.ts,
        graduation: row.graduated_ts === null && row.graduated_block === null ? null : { at: row.graduated_ts, block: row.graduated_block, tx: (row.graduated_tx as Hex | null) ?? null },
        supplyWei: row.supply, usdPerEth: rateOf(db, now).usdPerEth };
    }
    const r = this.reader ? await this.reader.read(this.curveCoin(w.e)) : null;
    const c = chartOf(r?.trades ?? [], res, now);
    return { source: this.source, kind: "curve", coin, res, seconds: c.seconds, span: c.span, quote: "ETH", candles: c.candles, trades: c.trades, tradesAllTime: r?.trades.length ?? 0,
      launchedAt: w.e.coin.launchedAt, graduation: r?.graduation ?? null, supplyWei: w.e.supplyWei, usdPerEth: this.d.db ? rateOf(this.d.db, now).usdPerEth : null };
  }
  private ponsTrades(coin: string, since: number): RookTrade[] {
    return (this.d.db!.prepare(`SELECT * FROM trades WHERE coin = ? AND ts >= ? ORDER BY block, log_index`).all(coin, since) as { block: number; log_index: number; venue: "curve" | "pool"; side: "buy" | "sell"; quote: string; coins: string; fee: string | null; tax: string | null; price: number; trader: string; ts: number | null; tx: string }[])
      .map((x) => ({ id: `${x.block}:${x.log_index}`, side: x.side, venue: x.venue, quoteWei: x.quote, coinWei: x.coins, feeWei: x.fee, taxWei: x.tax, price: x.price, trader: x.trader as Hex, tx: x.tx as Hex, block: x.block, logIndex: x.log_index, at: x.ts }));
  }
  async trades(address: string, limit: number, before?: string | null): Promise<RookTradePage | null> {
    const w = await this.which(address);
    if (!w) return null;
    if (w.kind === "pons") {
      const p = this.d.views!.trades(address, limit, before);
      return { source: this.source, kind: "pons", items: p.items.map((t) => ({ id: t.id, side: t.side, venue: t.venue, quoteWei: t.quoteWei, coinWei: t.coinWei, feeWei: t.feeWei, taxWei: t.taxWei, price: t.price, trader: t.trader, tx: t.tx, block: t.block, logIndex: t.logIndex, at: t.at })), next: p.next, total: p.total };
    }
    const r = this.reader ? await this.reader.read(this.curveCoin(w.e)) : null;
    const all = [...(r?.trades ?? [])].reverse();
    const cur = /^(\d{1,12}):(\d{1,6})$/.exec(before ?? "");
    const from = cur ? all.filter((t) => t.block < Number(cur[1]) || (t.block === Number(cur[1]) && t.logIndex < Number(cur[2]))) : all;
    const items = from.slice(0, limit);
    return { source: this.source, kind: "curve", items, next: from.length > limit ? items[items.length - 1].id : null, total: all.length };
  }
  async holders(address: string, limit: number, offset: number): Promise<RookHolderPage | null> {
    const w = await this.which(address);
    if (!w) return null;
    if (w.kind === "pons") {
      const h = this.d.views!.holders(address, limit, offset);
      if (!h) return null;
      return { source: this.source, kind: "pons", coin: h.coin, holders: h.holders, supplyWei: h.supplyWei, complete: h.complete,
        items: h.items.map((x) => ({ rank: x.rank, address: x.address, balanceWei: x.balanceWei, share: x.share, label: x.isCreator ? "creator" : null })), total: h.total, limit, offset };
    }
    const r = this.reader ? await this.reader.read(this.curveCoin(w.e)) : null;
    const hs = this.holdersOf(r?.balances ?? new Map(), w.e);
    const supply = w.e.supplyWei === null ? null : BigInt(w.e.supplyWei);
    const share = (v: bigint) => (supply === null || supply === 0n ? null : Number((v * 100_000_000n) / supply) / 1e8);
    const page = hs.rows.slice(offset, offset + limit);
    return { source: this.source, kind: "curve", coin: getAddress(address) as Hex, holders: r ? hs.holders : null, supplyWei: w.e.supplyWei, complete: !!r,
      items: page.map((x, i) => ({ rank: offset + i + 1, address: getAddress(x.address) as Hex, balanceWei: String(x.balance), share: share(x.balance), label: x.address === w.e.coin.creator.toLowerCase() ? "creator" : null })),
      total: hs.rows.length, limit, offset };
  }

  // ---- one wallet -----------------------------------------------------------------------------------------------------------
  async wallet(address: string): Promise<WalletProfile> {
    const board = await this.board(), now = this.now(), key = address.toLowerCase();
    const coinOf = (r: BoardRow): WalletCoin => ({ address: r.address, symbol: r.symbol, name: r.name, kind: r.kind, logoUrl: r.logoUrl });
    const byAddr = new Map(board.rows.map((r) => [r.address.toLowerCase(), r]));
    const holdings: WalletProfile["holdings"] = [], trades: WalletProfile["trades"] = [];
    const share = (v: bigint, supply: string | null) => (supply && BigInt(supply) > 0n ? Number((v * 100_000_000n) / BigInt(supply)) / 1e8 : null);
    const hold = (r: BoardRow, v: bigint) => holdings.push({ coin: coinOf(r), balanceWei: String(v), share: share(v, r.supplyWei), valueEth: r.price === null ? null : (Number(v) / 1e18) * r.price });
    if (this.d.db) {
      for (const b of this.d.db.prepare(`SELECT coin, amount FROM balances WHERE holder = ? AND amt > 0`).all(key) as { coin: string; amount: string }[]) { const r = byAddr.get(b.coin); if (r) hold(r, BigInt(b.amount)); }
      for (const t of this.d.db.prepare(`SELECT * FROM trades WHERE trader = ? ORDER BY block DESC, log_index DESC LIMIT 50`).all(key) as { block: number; log_index: number; coin: string; venue: "curve" | "pool"; side: "buy" | "sell"; quote: string; coins: string; fee: string | null; tax: string | null; price: number; trader: string; ts: number | null; tx: string }[]) {
        const r = byAddr.get(t.coin); if (!r) continue;
        trades.push({ id: `${t.block}:${t.log_index}`, side: t.side, venue: t.venue, quoteWei: t.quote, coinWei: t.coins, feeWei: t.fee, taxWei: t.tax, price: t.price, trader: t.trader as Hex, tx: t.tx as Hex, block: t.block, logIndex: t.log_index, at: t.ts, coin: coinOf(r) });
      }
    }
    for (const e of (await this.curves()).values()) {
      const r = byAddr.get(e.coin.address.toLowerCase()); if (!r) continue;
      const read = this.reader ? await this.reader.read(this.curveCoin(e)).catch(() => null) : null;
      if (!read) continue;
      const v = read.balances.get(key); if (v && v > 0n) hold(r, v);
      for (const t of read.trades) if (t.trader.toLowerCase() === key) trades.push({ ...t, coin: coinOf(r) });
    }
    const wins: WalletProfile["wins"] = [];
    if (this.d.wrapped && this.d.wrappedConfig) for (const c of this.d.wrappedConfig.collections) {
      const col = await this.d.wrapped.collection(c.slug).catch(() => null); if (!col) continue;
      for (const a of col.activity) if (a.kind === "raffle-won" && a.text.toLowerCase().includes(key.slice(0, 8))) wins.push({ at: a.at, nest: col.name, text: a.text, tx: a.tx });
    }
    holdings.sort((a, b) => (b.share ?? -1) - (a.share ?? -1) || (b.valueEth ?? -1) - (a.valueEth ?? -1));
    trades.sort((a, b) => b.block - a.block || b.logIndex - a.logIndex);
    return { source: this.source, address: getAddress(address) as Hex, at: now, launches: board.rows.filter((r) => r.creator.toLowerCase() === key), holdings, trades: trades.slice(0, 50), wins };
  }

  // ---- the ticker ----------------------------------------------------------------------------------------------------------
  async activity(limit: number): Promise<Activity> {
    const board = await this.board(), now = this.now();
    const items = board.rows.flatMap((r) => [launchItem(r), graduationItem(r)].filter((x): x is NonNullable<typeof x> => x !== null));
    if (this.d.db) for (const r of board.rows.filter((x) => x.kind === "pons")) items.push(...tradeItems(r, this.ponsTrades(r.address.toLowerCase(), now - DAY).slice(-20)));
    for (const e of (await this.curves()).values()) {
      const row = board.rows.find((x) => x.address.toLowerCase() === e.coin.address.toLowerCase());
      const r = this.reader ? await this.reader.read(this.curveCoin(e)).catch(() => null) : null;
      if (row && r) items.push(...tradeItems(row, r.trades.slice(-20)));
    }
    const seen = new Set<string>();
    if (this.d.wrapped && this.d.wrappedConfig) for (const c of this.d.wrappedConfig.collections) {
      if (seen.has(c.slug)) continue; seen.add(c.slug);
      const col = await this.d.wrapped.collection(c.slug).catch(() => null);
      if (col) items.push(...nestItems(col));
    }
    return { source: this.source, at: now, items: newestFirst(items, limit) };
  }
}
const spanOfRes = (res: RookRes) => (res === "1d" ? 30 * DAY : ({ "1m": 60, "5m": 300, "15m": 900, "1h": 3_600 } as Record<string, number>)[res] * 240);
export const market = marketOf;
