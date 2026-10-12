/**
 * The follower. For every Rook DBC config, and for every graduated coin's DAMM v2 pool, one signature stream:
 * getSignaturesForAddress(address) → getTransaction(jsonParsed) → decodeTx → rows. DBC's swap / swap2 take the config as account 1,
 * so a config's history holds every swap, creation and migration of every pool under it. A stream backfills from its newest
 * signature backwards (one page per pass, so live trades never wait for history) and follows forward with `until: head`.
 * Everything is idempotent by (signature, swap index): re-reading a page changes nothing.
 *
 * A third kind of stream (W13, canister-only design §4.4 rule 4): the canister's keys (`K_main`, the fee payer of every canister
 * transaction; any other key the operator names). Only the signatures whose RPC memo carries a Rook memo are fetched, and each
 * becomes a `desk_events` row: the memo's label next to what the transaction's balances say moved.
 *
 * Alongside: pool accounts (curve progress, migration flag) every POOLS_EVERY_S, metadata (Metaplex + the uri's JSON), mint supply,
 * holders, the SOL/USD rate, and (W13) each collection's wX/SOL rate from its desk's DLMM pair, or the canister's accepted floor
 * when there is no pair (wx.ts). Read-only: the RPC wrapper refuses anything that is not a read; the canister is only queried.
 */
import type { DatabaseSync } from "node:sqlite";
import { b64decode } from "./bytes.js";
import type { CanisterReader } from "./canister.js";
import type { RookConfig } from "./configs.js";
import { DBC_PROGRAM, NATIVE_MINT, POOL_CONFIG_OFFSET, TOKEN_2022_PROGRAM, VIRTUAL_POOL_LEN, decodeConfig, decodeDammPool, decodeMetadata, decodeMint, decodePool, type DbcPool } from "./dbc.js";
import { cleanText, decodeTx, type DecodedTx, type ParsedTx } from "./decode.js";
import { getMeta, setMeta } from "./db.js";
import { fetchHolders, label } from "./holders.js";
import { carriesRookMemo, deskEvents } from "./memo.js";
import { fetchOffChain } from "./offchain.js";
import { metadataAddress } from "./pda.js";
import type { Fetch, SolRpc } from "./rpc.js";
import { dammTrade, dbcTrade, ordOf } from "./trades.js";
import { ensureHistory, liveSolUsd, storeRate } from "./usd.js";
import { decodeLbPair, pairAddress, solPerWx, storeWxRate } from "./wx.js";

export type IndexerOpts = {
  db: DatabaseSync; rpc: SolRpc; configs: RookConfig[];
  log?: (o: Record<string, unknown>) => void; now?: () => number; fetch?: Fetch;
  heliusKey?: string | null; cluster?: string;
  backfillMax?: number; pageSize?: number; chunk?: number; txConcurrency?: number; txVersion?: 0 | 1;
  holdersEveryS?: number; poolsEveryS?: number; discoverEveryS?: number; wxEveryS?: number; collectionsEveryS?: number;
  /** W13: the canister's Solana keys whose memo-labelled history is indexed (`K_main` first), and its queries (null: no canister) */
  deskKeys?: string[]; canister?: CanisterReader | null;
  /** false: never fetch metadata JSON / USD over HTTP (tests) */
  http?: boolean;
};
type Sig = { signature: string; slot: number; err: unknown; blockTime?: number | null; transactionIndex?: number; memo?: string | null };
export type StreamKind = "config" | "damm" | "desk";
type Stream = { address: string; kind: StreamKind; mint: string | null; head_sig: string | null; head_slot: number | null; back_before: string | null;
  back_done: number; back_count: number; seen: number; last_new_at: number | null; polled_at: number | null; truncated: number };
type Cfg = { slug: string; listingId: number | null; feeMode: number; decimals: number; quoteMint: string | null; activationType: number;
  quoteDecimals: number; coll: number | null; module: string | null; pair: string | null };
export type IndexerState = { lastPollAt: number | null; lastError: string | null; passes: number; backfilling: boolean };

const IDLE_S = 600;

export class SolIndexer {
  readonly state: IndexerState = { lastPollAt: null, lastError: null, passes: 0, backfilling: false };
  private readonly now: () => number;
  private readonly log: (o: Record<string, unknown>) => void;
  private readonly configSet: Set<string>;
  private readonly cfg = new Map<string, Cfg>();
  private at = { pools: 0, discover: 0, usd: 0, meta: 0, supply: 0, holders: 0, wx: 0, collections: 0 };
  private txVersion: 0 | 1;
  private readonly timeTried = new Set<string>();
  constructor(private readonly o: IndexerOpts) {
    this.now = o.now ?? (() => Math.floor(Date.now() / 1000));
    this.log = o.log ?? (() => {});
    this.configSet = new Set(o.configs.map((c) => c.config));
    this.txVersion = o.txVersion ?? 1;
  }
  private get db() { return this.o.db; }

  // ---- setup -------------------------------------------------------------------------------------------------------------------
  /** Reads each config account (fee mode, decimals, threshold) and opens a stream for it. */
  async init(): Promise<void> {
    const cs = this.o.configs;
    for (let i = 0; i < cs.length; i += 100) {
      const chunk = cs.slice(i, i + 100);
      const r = await this.o.rpc.call<{ value: ({ owner: string; data: [string, string] } | null)[] }>("getMultipleAccounts", [chunk.map((c) => c.config), { encoding: "base64" }]);
      r.value.forEach((a, j) => {
        const c = chunk[j];
        let d: ReturnType<typeof decodeConfig> | null = null;
        if (a && a.owner === DBC_PROGRAM) { try { d = decodeConfig(b64decode(a.data[0])); } catch { d = null; } }
        if (!d) this.log({ level: "warn", event: "config-unreadable", slug: c.slug, config: c.config });
        // a Rook config is quoted in its collection's wX (canister-only design §3); a SOL quote is the pre-W13 shape and still read
        const wx = d && d.quoteMint !== NATIVE_MINT ? d.quoteMint : null;
        const pair = wx ? pairAddress(wx) : null;
        this.db.prepare(`INSERT INTO configs (config, slug, listing_id, quote_mint, fee_mode, decimals, threshold, migration_option, read_at, pair) VALUES (?,?,?,?,?,?,?,?,?,?)
          ON CONFLICT(config) DO UPDATE SET slug=excluded.slug, listing_id=excluded.listing_id, quote_mint=COALESCE(excluded.quote_mint, quote_mint), fee_mode=COALESCE(excluded.fee_mode, fee_mode),
          decimals=COALESCE(excluded.decimals, decimals), threshold=COALESCE(excluded.threshold, threshold), migration_option=COALESCE(excluded.migration_option, migration_option), read_at=excluded.read_at,
          pair=COALESCE(excluded.pair, pair)`)
          .run(c.config, c.slug, c.listingId, d?.quoteMint ?? null, d?.collectFeeMode ?? null, d?.tokenDecimal ?? null, d ? String(d.migrationQuoteThreshold) : null, d?.migrationOption ?? null, this.now(), pair);
        const kept = this.db.prepare(`SELECT quote_decimals, coll, module, pair FROM configs WHERE config = ?`).get(c.config) as { quote_decimals: number | null; coll: number | null; module: string | null; pair: string | null };
        this.cfg.set(c.config, { slug: c.slug, listingId: c.listingId, feeMode: d?.collectFeeMode ?? 0, decimals: d?.tokenDecimal ?? 6, quoteMint: d?.quoteMint ?? null, activationType: d?.activationType ?? 1,
          quoteDecimals: kept.quote_decimals ?? 9, coll: kept.coll, module: kept.module, pair: kept.pair });
        this.db.prepare(`INSERT OR IGNORE INTO streams (address, kind) VALUES (?, 'config')`).run(c.config);
      });
    }
    await this.readQuoteMints();
    for (const k of this.o.deskKeys ?? []) this.db.prepare(`INSERT OR IGNORE INTO streams (address, kind) VALUES (?, 'desk')`).run(k);
    if (this.o.canister) { try { await this.refreshCollections(); } catch (e) { this.log({ level: "warn", event: "collections-error", message: String((e as Error).message).slice(0, 200) }); } }
  }
  /** The wX mints' decimals (9 by the design, read rather than assumed: a price divides by them). */
  private async readQuoteMints(): Promise<void> {
    const need = [...this.cfg.entries()].filter(([, c]) => c.quoteMint && c.quoteMint !== NATIVE_MINT).map(([config, c]) => ({ config, mint: c.quoteMint! }));
    for (let i = 0; i < need.length; i += 100) {
      const chunk = need.slice(i, i + 100);
      const r = await this.o.rpc.call<{ value: ({ data: [string, string] } | null)[] }>("getMultipleAccounts", [chunk.map((x) => x.mint), { encoding: "base64" }]);
      r.value.forEach((a, j) => {
        if (!a) return;
        let m: ReturnType<typeof decodeMint>;
        try { m = decodeMint(b64decode(a.data[0])); } catch { return; }
        this.db.prepare(`UPDATE configs SET quote_decimals = ? WHERE config = ?`).run(m.decimals, chunk[j].config);
        this.cfg.get(chunk[j].config)!.quoteDecimals = m.decimals;
      });
    }
  }
  /** W13: the canister's collection index, module `M[i]` and wX symbol per config (`coins_collections`), matched by wX mint. */
  async refreshCollections(): Promise<void> {
    if (!this.o.canister) return;
    const rows = await this.o.canister.collections();
    for (const [config, c] of this.cfg) {
      const k = rows.find((r) => (c.quoteMint && r.wxMint === c.quoteMint) || r.config === config);
      if (!k) continue;
      c.coll = k.index; c.module = k.module || null;
      this.db.prepare(`UPDATE configs SET coll = ?, module = ?, quote_symbol = ? WHERE config = ?`).run(k.index, k.module || null, k.symbol || null, config);
    }
  }
  /**
   * W13: each wX's price in SOL: the desk's DLMM pair's active bin (wx.ts); the canister's accepted floor when the pair account
   * does not exist. Stored per minute. A pair that exists but is not a wX/wSOL pair, or is disabled, gives nothing (never a guess).
   */
  async refreshWx(): Promise<void> {
    const t = Math.floor(this.now() / 60) * 60;
    const cs = [...this.cfg.entries()].filter(([, c]) => c.pair && c.quoteMint);
    const missing: [string, Cfg][] = [];
    for (let i = 0; i < cs.length; i += 100) {
      const chunk = cs.slice(i, i + 100);
      const r = await this.o.rpc.call<{ value: ({ owner: string; data: [string, string] } | null)[] }>("getMultipleAccounts", [chunk.map(([, c]) => c.pair), { encoding: "base64", commitment: "confirmed" }]);
      r.value.forEach((a, j) => {
        const [, c] = chunk[j];
        if (!a) { missing.push(chunk[j]); return; }
        let sol: number | null = null;
        try { sol = solPerWx(decodeLbPair(b64decode(a.data[0])), c.quoteMint!, c.quoteDecimals); } catch { sol = null; }
        if (sol !== null) storeWxRate(this.db, c.quoteMint!, t, sol, "dlmm");
        else this.log({ level: "warn", event: "pair-unpriced", slug: c.slug, pair: c.pair });
      });
    }
    if (!this.o.canister) return;
    for (const [, c] of missing) {
      if (c.coll === null) continue;
      const f = await this.o.canister.books(c.coll);
      if (f && f.accepted > 0n && f.wxMint === c.quoteMint) storeWxRate(this.db, c.quoteMint!, t, Number(f.accepted) / 1e9, "books");
    }
  }

  // ---- the loop ----------------------------------------------------------------------------------------------------------------
  /** One pass: every due stream, then whatever periodic refresh is due. Returns the number of new trades. */
  async pass(pollMs = 4_000): Promise<number> {
    const now = this.now();
    let n = 0, err: string | null = null;
    const streams = this.db.prepare(`SELECT * FROM streams`).all() as Stream[];
    this.state.backfilling = streams.some((s) => !s.back_done);
    for (const s of streams) {
      if (s.kind === "config" && !this.configSet.has(s.address)) continue;
      if (s.kind === "desk" && !(this.o.deskKeys ?? []).includes(s.address)) continue;
      // an idle stream (nothing new for 10 minutes) is polled every 30 s, a quiet DAMM pool (a day) every 2 minutes
      const idle = s.last_new_at !== null && now - s.last_new_at > IDLE_S;
      const every = !s.back_done ? 0 : idle ? (s.kind === "damm" && now - (s.last_new_at ?? 0) > 86_400 ? 120 : 30) : pollMs / 1000;
      if (s.polled_at !== null && now - s.polled_at < every) continue;
      try { n += await this.pollStream(s); }
      catch (e) { err = String((e as Error).message).slice(0, 200); this.log({ level: "warn", event: "stream-error", address: s.address, message: err }); }
    }
    for (const [k, every, fn] of [["discover", this.o.discoverEveryS ?? 600, () => this.discover()], ["pools", this.o.poolsEveryS ?? 15, () => this.refreshPools()],
      ["meta", 30, () => this.refreshMeta()], ["supply", 600, () => this.refreshSupply()], ["usd", 60, () => this.refreshUsd()],
      ["holders", 20, () => this.refreshHolders()], ["wx", this.o.wxEveryS ?? 60, () => this.refreshWx()], ["collections", this.o.collectionsEveryS ?? 600, () => this.refreshCollections()]] as const) {
      if (now - this.at[k] < every) continue;
      this.at[k] = now;
      try { await fn(); } catch (e) { err = String((e as Error).message).slice(0, 200); this.log({ level: "warn", event: `${k}-error`, message: err }); }
    }
    this.state.passes++;
    this.state.lastError = err;
    if (!err) { this.state.lastPollAt = this.now(); setMeta(this.db, "last_poll_at", String(this.state.lastPollAt)); }
    return n;
  }

  // ---- streams -----------------------------------------------------------------------------------------------------------------
  private sigs(address: string, o: { before?: string; until?: string; limit: number }): Promise<Sig[]> {
    return this.o.rpc.call<Sig[]>("getSignaturesForAddress", [address, { limit: o.limit, ...(o.before ? { before: o.before } : {}), ...(o.until ? { until: o.until } : {}), commitment: "confirmed" }]);
  }
  async pollStream(s: Stream): Promise<number> {
    const page = this.o.pageSize ?? 1000, chunk = Math.min(page, this.o.chunk ?? 100), now = this.now();
    let added = 0;
    if (s.kind === "damm") await this.ensureDammSide(s.mint);
    if (!s.head_sig) {
      // the newest signature becomes the head; everything older is the backfill's, a chunk per pass
      const first = await this.sigs(s.address, { limit: 1 });
      if (!first.length) { this.db.prepare(`UPDATE streams SET back_done = 1, polled_at = ? WHERE address = ?`).run(now, s.address); return 0; }
      added += await this.apply(s, first);
      this.db.prepare(`UPDATE streams SET head_sig = ?, head_slot = ?, back_before = ?, back_done = 0, back_count = 1, seen = seen + 1, last_new_at = ?, polled_at = ? WHERE address = ?`)
        .run(first[0].signature, first[0].slot, first[0].signature, first[0].blockTime ?? now, now, s.address);
      return added;
    }
    // forward: everything newer than the head (newest first from the RPC), applied oldest first, a chunk at a time, the head moving with each
    const fresh: Sig[] = [];
    let before: string | undefined;
    for (let i = 0; i < 20; i++) {
      const p = await this.sigs(s.address, { until: s.head_sig, before, limit: page });
      fresh.push(...p);
      if (p.length < page) break;
      before = p[p.length - 1].signature;
      if (i === 19) { this.db.prepare(`UPDATE streams SET truncated = 1 WHERE address = ?`).run(s.address); this.log({ level: "warn", event: "forward-gap", address: s.address }); }
    }
    for (let end = fresh.length; end > 0; end -= chunk) {
      const part = fresh.slice(Math.max(0, end - chunk), end);
      added += await this.apply(s, part);
      this.db.prepare(`UPDATE streams SET head_sig = ?, head_slot = ?, seen = seen + ?, last_new_at = ? WHERE address = ?`).run(part[0].signature, part[0].slot, part.length, part[0].blockTime ?? now, s.address);
    }
    // backfill: one chunk further back per pass, so live trades never wait for history
    if (!s.back_done && s.back_before) {
      const max = this.o.backfillMax ?? 20_000;
      const want = Math.min(chunk, Math.max(1, max - s.back_count));
      const p = await this.sigs(s.address, { before: s.back_before, limit: want });
      if (p.length) added += await this.apply(s, p);
      const count = s.back_count + p.length;
      const done = p.length < want || count >= max ? 1 : 0;
      this.db.prepare(`UPDATE streams SET back_before = ?, back_count = ?, back_done = ?, truncated = CASE WHEN ? THEN 1 ELSE truncated END, seen = seen + ? WHERE address = ?`)
        .run(p.length ? p[p.length - 1].signature : s.back_before, count, done, count >= max && p.length === want ? 1 : 0, p.length, s.address);
      if (done) this.log({ level: "info", event: "backfill-done", address: s.address, kind: s.kind, transactions: count });
    }
    this.db.prepare(`UPDATE streams SET polled_at = ? WHERE address = ?`).run(now, s.address);
    return added;
  }

  async getTx(sig: string): Promise<ParsedTx | null> {
    try {
      return await this.o.rpc.call<ParsedTx | null>("getTransaction", [sig, { encoding: "jsonParsed", maxSupportedTransactionVersion: this.txVersion, commitment: "confirmed" }]);
    } catch (e) {
      // an RPC that does not know transaction version 1 yet: fall back to 0 (and v1 transactions are then skipped by that RPC's refusal)
      if (this.txVersion === 1 && /maxSupportedTransactionVersion|invalid.*version/i.test(String((e as Error).message)) && !/not supported by the requesting client/i.test(String((e as Error).message))) {
        this.txVersion = 0; return this.getTx(sig);
      }
      throw e;
    }
  }

  /** Fetches and applies a page of signatures (newest first, as the RPC returns them). Throws (nothing written) if any fetch fails. */
  async apply(s: Stream, page: Sig[]): Promise<number> {
    // a desk key's history is mostly nonce advances and reads; only a transaction whose memo is Rook's is worth a getTransaction
    const ok = page.filter((x) => (x.err === null || x.err === undefined) && (s.kind !== "desk" || carriesRookMemo(x.memo)));
    const txs: (ParsedTx | null)[] = new Array(ok.length).fill(null);
    let next = 0;
    const worker = async () => { for (let i = next++; i < ok.length; i = next++) { txs[i] = await this.getTx(ok[i].signature); if (!txs[i]) throw new Error(`transaction ${ok[i].signature} not available yet`); } };
    await Promise.all(Array.from({ length: Math.min(this.o.txConcurrency ?? 6, ok.length) }, worker));
    if (process.env.SOL_DEBUG) this.log({ level: "debug", event: "page", address: s.address, signatures: page.length, fetched: ok.length });
    const decoded = txs.map((t, i) => ({ d: decodeTx(t!), txIndex: ok[i].transactionIndex ?? 0 })).reverse();
    return this.write(s, decoded);
  }

  /** Writes decoded transactions (oldest first). Exposed for tests: no RPC inside. */
  write(s: Pick<Stream, "address" | "kind" | "mint">, decoded: { d: DecodedTx; txIndex: number }[]): number {
    let added = 0;
    const db = this.db;
    db.exec("BEGIN");
    try {
      for (const { d, txIndex } of decoded) {
        if (!d.ok) continue;
        const at = d.blockTime;
        if (s.kind === "desk") {
          // the label and what the balances say, side by side (memo.ts); idempotent by (signature, memo index)
          for (const e of deskEvents(d, this.knowledge())) {
            const r = db.prepare(`INSERT OR IGNORE INTO desk_events (sig, n, slot, at, coll, kind, link_id, intent, payer, memo, fee, wx_mint, wx_delta, payer_lamports, module_lamports, deltas)
              VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)`)
              .run(d.signature, e.n, d.slot, at, e.coll, e.kind, e.linkId, e.intent, d.feePayer, e.memo, d.fee, e.wxMint, e.wxDelta === null ? null : String(e.wxDelta), String(e.payerLamports),
                e.moduleLamports === null ? null : String(e.moduleLamports), JSON.stringify(e.deltas));
            if (Number(r.changes) > 0) added++;
          }
          continue;
        }
        for (const i of d.inits) {
          if (!this.configSet.has(i.ev.config)) continue;
          const c = this.cfg.get(i.ev.config);
          this.upsertCoin({ mint: i.ev.baseMint, pool: i.ev.pool, config: i.ev.config, decimals: c?.decimals ?? 6 });
          db.prepare(`UPDATE coins SET creator = COALESCE(creator, ?), name = COALESCE(name, ?), symbol = COALESCE(symbol, ?), uri = COALESCE(uri, ?), meta_at = CASE WHEN ? IS NOT NULL THEN COALESCE(meta_at, ?) ELSE meta_at END,
            created_slot = ?, created_at = ?, created_tx = ?, activation_point = COALESCE(activation_point, ?) WHERE mint = ?`)
            .run(i.ev.creator, i.name, i.symbol, i.uri, i.name, this.now(), d.slot, at, d.signature, String(i.ev.activationPoint), i.ev.baseMint);
        }
        for (const sw of d.swaps) {
          let mint: string | null = null, row: ReturnType<typeof dbcTrade> = null, qd = 9;
          if (sw.ev.kind === "dbc-swap") {
            if (s.kind !== "config" || !this.configSet.has(sw.ev.config)) continue;
            const c = this.cfg.get(sw.ev.config);
            mint = this.mintOfPool(sw.ev.pool);
            if (!mint && sw.ix) { mint = sw.ix.accounts[7] ?? null; if (mint) this.upsertCoin({ mint, pool: sw.ev.pool, config: sw.ev.config, decimals: c?.decimals ?? 6 }); }
            if (!mint) continue;
            qd = c?.quoteDecimals ?? 9;
            row = dbcTrade(sw.ev, this.decimalsOf(mint), c?.feeMode ?? 0, qd);
          } else {
            if (s.kind !== "damm" || sw.ev.pool !== s.address || !s.mint) continue;
            mint = s.mint;
            const coin = db.prepare(`SELECT coin_is_a, config FROM coins WHERE mint = ?`).get(mint) as { coin_is_a: number | null; config: string } | undefined;
            qd = this.cfg.get(coin?.config ?? "")?.quoteDecimals ?? 9;
            row = dammTrade(sw.ev, this.decimalsOf(mint), coin?.coin_is_a !== 0, qd);
          }
          if (!row) continue;
          const ord = ordOf(d.slot, txIndex, sw.n), when = at ?? Number(sw.ev.timestamp);
          const r = db.prepare(`INSERT OR IGNORE INTO trades (sig, n, mint, ord, slot, at, side, venue, sol, sol_f, tokens, tokens_f, fee, price, trader) VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)`)
            .run(d.signature, sw.n, mint, ord, d.slot, when, row.side, row.venue, String(row.sol), Number(row.sol) / 10 ** qd, String(row.tokens), Number(row.tokens) / 10 ** this.decimalsOf(mint), row.fee === null ? null : String(row.fee), row.price, sw.trader);
          if (Number(r.changes) > 0) {
            added++;
            db.prepare(`UPDATE coins SET trades_n = trades_n + 1, last_ord = CASE WHEN last_ord IS NULL OR ? > last_ord THEN ? ELSE last_ord END,
              last_price = CASE WHEN last_ord IS NULL OR ? > last_ord THEN ? ELSE last_price END, last_trade_at = MAX(COALESCE(last_trade_at, 0), ?) WHERE mint = ?`)
              .run(ord, ord, ord, row.price, when, mint);
          }
        }
        for (const c of d.completes) {
          if (!this.configSet.has(c.config)) continue;
          db.prepare(`UPDATE coins SET completed_at = COALESCE(completed_at, ?) WHERE pool = ?`).run(at, c.pool);
        }
        for (const m of d.migrations) {
          if (!this.configSet.has(m.config) || m.version !== "damm-v2") { if (this.configSet.has(m.config)) db.prepare(`UPDATE coins SET is_migrated = 1, graduated_at = COALESCE(graduated_at, ?) WHERE pool = ?`).run(at, m.virtualPool); continue; }
          const mint = this.mintOfPool(m.virtualPool) ?? m.baseMint;
          if (!this.mintOfPool(m.virtualPool)) { const c = this.cfg.get(m.config); this.upsertCoin({ mint, pool: m.virtualPool, config: m.config, decimals: c?.decimals ?? 6 }); }
          db.prepare(`UPDATE coins SET damm_pool = ?, is_migrated = 1, graduated_at = ?, graduated_slot = ?, graduated_tx = ?, completed_at = COALESCE(completed_at, ?) WHERE mint = ?`)
            .run(m.dammPool, at, d.slot, d.signature, at, mint);
          db.prepare(`INSERT OR IGNORE INTO streams (address, kind, mint) VALUES (?, 'damm', ?)`).run(m.dammPool, mint);
          this.log({ level: "info", event: "graduated", mint, dammPool: m.dammPool });
        }
      }
      db.exec("COMMIT");
    } catch (e) { db.exec("ROLLBACK"); throw e; }
    return added;
  }

  private upsertCoin(c: { mint: string; pool: string; config: string; decimals: number }): void {
    this.db.prepare(`INSERT OR IGNORE INTO coins (mint, pool, config, decimals) VALUES (?, ?, ?, ?)`).run(c.mint, c.pool, c.config, c.decimals);
  }
  private mintOfPool(pool: string): string | null { return ((this.db.prepare(`SELECT mint FROM coins WHERE pool = ?`).get(pool) as { mint: string } | undefined)?.mint) ?? null; }
  /** What a memo's collection index means here: its wX mint and module, from the canister's `coins_collections` (null until read). */
  private knowledge() {
    const byColl = new Map([...this.cfg.values()].filter((c) => c.coll !== null).map((c) => [c.coll!, c]));
    const wx = new Set([...this.cfg.values()].map((c) => c.quoteMint).filter((m): m is string => !!m && m !== NATIVE_MINT));
    return { wxMintOf: (coll: number) => byColl.get(coll)?.quoteMint ?? null, moduleOf: (coll: number) => byColl.get(coll)?.module ?? null, isWxMint: (m: string) => wx.has(m) };
  }
  private decimalsOf(mint: string): number { return ((this.db.prepare(`SELECT decimals FROM coins WHERE mint = ?`).get(mint) as { decimals: number } | undefined)?.decimals) ?? 6; }

  /** A DAMM pool's token order, read once before its swaps are written (DBC migrates the coin as token A; this checks). */
  private async ensureDammSide(mint: string | null): Promise<void> {
    if (!mint) return;
    const c = this.db.prepare(`SELECT damm_pool, coin_is_a FROM coins WHERE mint = ?`).get(mint) as { damm_pool: string | null; coin_is_a: number | null } | undefined;
    if (!c?.damm_pool || c.coin_is_a !== null) return;
    const a = await this.o.rpc.call<{ value: { data: [string, string] } | null }>("getAccountInfo", [c.damm_pool, { encoding: "base64" }]);
    if (!a.value) throw new Error(`DAMM pool ${c.damm_pool} not readable yet`);
    const p = decodeDammPool(b64decode(a.value.data[0]));
    this.db.prepare(`UPDATE coins SET coin_is_a = ?, damm_vault_a = ?, damm_vault_b = ? WHERE mint = ?`).run(p.tokenAMint === mint ? 1 : 0, p.tokenAVault, p.tokenBVault, mint);
  }

  // ---- discovery and accounts --------------------------------------------------------------------------------------------------
  /** Every pool on each Rook config (getProgramAccounts). An RPC that refuses it leaves discovery to the streams. */
  async discover(): Promise<number> {
    let n = 0;
    for (const c of this.o.configs) {
      let rows: { pubkey: string; account: { data: [string, string] } }[];
      try {
        rows = await this.o.rpc.call("getProgramAccounts", [DBC_PROGRAM, { encoding: "base64", filters: [{ dataSize: VIRTUAL_POOL_LEN }, { memcmp: { offset: POOL_CONFIG_OFFSET, bytes: c.config } }], commitment: "confirmed" }]);
      } catch (e) { this.log({ level: "info", event: "discover-skipped", config: c.config, message: String((e as Error).message).slice(0, 120) }); return n; }
      for (const r of rows) {
        let p: DbcPool;
        try { p = decodePool(b64decode(r.account.data[0])); } catch { continue; }
        if (!this.mintOfPool(r.pubkey)) { this.upsertCoin({ mint: p.baseMint, pool: r.pubkey, config: c.config, decimals: this.cfg.get(c.config)?.decimals ?? 6 }); n++; }
        this.savePool(r.pubkey, p);
      }
    }
    if (n) this.log({ level: "info", event: "discovered", pools: n });
    return n;
  }
  private savePool(pool: string, p: DbcPool): void {
    const c = this.cfg.get(p.config);
    // a timestamp-activated config's activation point is the launch time; the creation transaction's time wins when it is known
    const launched = c?.activationType === 1 ? Number(p.activationPoint) : null;
    this.db.prepare(`UPDATE coins SET creator = COALESCE(creator, ?), base_vault = ?, quote_vault = ?, quote_reserve = ?, sqrt_price = ?, is_migrated = MAX(is_migrated, ?),
      activation_point = COALESCE(activation_point, ?), created_at = COALESCE(created_at, ?), completed_at = CASE WHEN completed_at IS NULL AND ? > 0 THEN ? ELSE completed_at END, pool_at = ? WHERE pool = ?`)
      .run(p.creator, p.baseVault, p.quoteVault, String(p.quoteReserve), String(p.sqrtPrice), p.isMigrated ? 1 : 0, String(p.activationPoint), launched,
        Number(p.finishCurveTimestamp), Number(p.finishCurveTimestamp), this.now(), pool);
  }
  /** The curve state of every coin still on (or just off) its curve: getMultipleAccounts, 100 at a time. */
  async refreshPools(): Promise<void> {
    const pools = (this.db.prepare(`SELECT pool FROM coins WHERE is_migrated = 0 OR pool_at IS NULL`).all() as { pool: string }[]).map((r) => r.pool);
    for (let i = 0; i < pools.length; i += 100) {
      const chunk = pools.slice(i, i + 100);
      const r = await this.o.rpc.call<{ value: ({ owner: string; data: [string, string] } | null)[] }>("getMultipleAccounts", [chunk, { encoding: "base64", commitment: "confirmed" }]);
      r.value.forEach((a, j) => { if (a && a.owner === DBC_PROGRAM) { try { this.savePool(chunk[j], decodePool(b64decode(a.data[0]))); } catch { /* not a pool */ } } });
    }
    // a slot-activated config's coin whose creation was not seen: its launch time is its activation slot's block time (a few per pass)
    const noTime = (this.db.prepare(`SELECT mint, config, activation_point FROM coins WHERE created_at IS NULL AND activation_point IS NOT NULL`).all() as { mint: string; config: string; activation_point: string }[])
      .filter((c) => this.cfg.get(c.config)?.activationType === 0 && !this.timeTried.has(c.mint)).slice(0, 10);
    for (const c of noTime) {
      this.timeTried.add(c.mint);
      const t = await this.o.rpc.call<number | null>("getBlockTime", [Number(c.activation_point)]).catch(() => null);
      if (t) this.db.prepare(`UPDATE coins SET created_at = COALESCE(created_at, ?) WHERE mint = ?`).run(t, c.mint);
    }
  }
  /** Name/symbol/uri for coins whose creation was not seen (Metaplex metadata), then each uri's JSON (image, links). */
  async refreshMeta(): Promise<void> {
    const need = this.db.prepare(`SELECT mint FROM coins WHERE meta_at IS NULL LIMIT 100`).all() as { mint: string }[];
    if (need.length) {
      const addrs = need.map((r) => metadataAddress(r.mint));
      const r = await this.o.rpc.call<{ value: ({ data: [string, string] } | null)[] }>("getMultipleAccounts", [addrs, { encoding: "base64" }]);
      r.value.forEach((a, j) => {
        let m: ReturnType<typeof decodeMetadata> | null = null;
        if (a) { try { m = decodeMetadata(b64decode(a.data[0])); } catch { m = null; } }
        this.db.prepare(`UPDATE coins SET name = COALESCE(name, ?), symbol = COALESCE(symbol, ?), uri = COALESCE(uri, ?), meta_at = ? WHERE mint = ?`).run(cleanText(m?.name ?? null), cleanText(m?.symbol ?? null, 16), m?.uri ? m.uri.slice(0, 300) : null, this.now(), need[j].mint);
      });
    }
    if (this.o.http === false) return;
    // the uri's JSON: new coins first, a failed fetch retried after an hour, at most 8 per pass
    const off = this.db.prepare(`SELECT mint, uri FROM coins WHERE uri IS NOT NULL AND uri != '' AND (offchain_at IS NULL OR (image IS NULL AND offchain_at < ?)) ORDER BY created_at DESC LIMIT 8`).all(this.now() - 3_600) as { mint: string; uri: string }[];
    await Promise.all(off.map(async (c) => {
      const j = await fetchOffChain(c.uri, this.o.fetch);
      this.db.prepare(`UPDATE coins SET image = COALESCE(?, image), description = COALESCE(?, description), website = COALESCE(?, website), twitter = COALESCE(?, twitter), telegram = COALESCE(?, telegram), offchain_at = ? WHERE mint = ?`)
        .run(j?.image ?? null, j?.description ?? null, j?.website ?? null, j?.twitter ?? null, j?.telegram ?? null, this.now(), c.mint);
    }));
  }
  /** Mint supply and decimals (jsonParsed); a Token-2022 coin's name/symbol/uri from its tokenMetadata extension (it has no Metaplex account). */
  async refreshSupply(): Promise<void> {
    const mints = (this.db.prepare(`SELECT mint FROM coins WHERE supply_at IS NULL OR supply_at < ? ORDER BY supply_at IS NOT NULL, supply_at LIMIT 500`).all(this.now() - 3_600) as { mint: string }[]).map((r) => r.mint);
    type Parsed = { owner: string; data: { parsed?: { info?: { supply?: string; decimals?: number; extensions?: { extension: string; state?: { name?: string; symbol?: string; uri?: string } }[] } } } };
    for (let i = 0; i < mints.length; i += 100) {
      const chunk = mints.slice(i, i + 100);
      const r = await this.o.rpc.call<{ value: (Parsed | null)[] }>("getMultipleAccounts", [chunk, { encoding: "jsonParsed" }]);
      r.value.forEach((a, j) => {
        const info = a?.data?.parsed?.info;
        if (!a || !info || info.supply === undefined || typeof info.decimals !== "number") return;
        this.db.prepare(`UPDATE coins SET supply = ?, decimals = ?, supply_at = ? WHERE mint = ?`).run(String(info.supply), info.decimals, this.now(), chunk[j]);
        if (a.owner === TOKEN_2022_PROGRAM) {
          setMeta(this.db, `t22:${chunk[j]}`, "1");
          const md = info.extensions?.find((e) => e.extension === "tokenMetadata")?.state;
          if (md) this.db.prepare(`UPDATE coins SET name = COALESCE(name, ?), symbol = COALESCE(symbol, ?), uri = COALESCE(uri, ?) WHERE mint = ?`)
            .run(cleanText(md.name ?? null), cleanText(md.symbol ?? null, 16), md.uri ? md.uri.slice(0, 300) : null, chunk[j]);
        }
      });
    }
  }
  async refreshUsd(): Promise<void> {
    if (this.o.http === false) return;
    const r = await liveSolUsd(this.o.fetch);
    if (r) storeRate(this.db, Math.floor(this.now() / 60) * 60, r.usd, r.src);
    // history for trades older than the live series (the backfill), once per range
    const first = this.db.prepare(`SELECT min(at) AS a FROM trades`).get() as { a: number | null };
    const live = this.db.prepare(`SELECT min(t) AS t FROM sol_usd WHERE src != 'pyth-history'`).get() as { t: number | null };
    const done = Number(getMeta(this.db, "usd_history_from") ?? "0");
    if (first.a !== null && (done === 0 || first.a < done)) {
      const to = live.t ?? this.now();
      if (to > first.a) await ensureHistory(this.db, first.a, to, this.o.fetch);
      setMeta(this.db, "usd_history_from", String(first.a));
    }
  }
  /** A few coins per pass: active ones (a trade in the last day) every holdersEveryS, the rest every 30 minutes. */
  async refreshHolders(limit = 4, only?: string): Promise<void> {
    const now = this.now(), every = this.o.holdersEveryS ?? 180;
    const due = only ? this.db.prepare(`SELECT mint, creator, base_vault, damm_vault_a, damm_vault_b FROM coins WHERE mint = ?`).all(only) as never[] : this.db.prepare(`SELECT mint, creator, base_vault, damm_vault_a, damm_vault_b FROM coins
      WHERE holders_at IS NULL OR (COALESCE(last_trade_at, 0) >= ? AND holders_at < ?) OR holders_at < ? ORDER BY holders_at IS NOT NULL, last_trade_at DESC LIMIT ?`)
      .all(now - 86_400, now - every, now - 1_800, limit) as { mint: string; creator: string | null; base_vault: string | null; damm_vault_a: string | null; damm_vault_b: string | null }[];
    for (const c of due) {
      const f = await fetchHolders(this.o.rpc, c.mint, { tokenProgram: getMeta(this.db, `t22:${c.mint}`) ? TOKEN_2022_PROGRAM : undefined, heliusKey: this.o.heliusKey, fetch: this.o.fetch, cluster: this.o.cluster,
        preferGpa: now - Number(getMeta(this.db, "gpa_refused_at") ?? "0") > 3_600 });
      // an RPC that refuses getProgramAccounts (PublicNode does) is asked again only hourly; the top 20 stand in meanwhile
      if (f.gpaRefused) setMeta(this.db, "gpa_refused_at", String(now));
      const snap = label(f.rows, { creator: c.creator, curveVaults: [c.base_vault ?? ""], poolVaults: [c.damm_vault_a ?? "", c.damm_vault_b ?? ""] }, f.complete);
      this.db.prepare(`UPDATE coins SET holders = ?, holders_at = ?, holders_complete = ?, holders_json = ? WHERE mint = ?`)
        .run(snap.count, now, snap.complete ? 1 : 0, JSON.stringify(snap.items.map((x) => ({ ...x, amount: String(x.amount) }))), c.mint);
    }
  }
}
