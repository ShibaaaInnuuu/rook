import type { DatabaseSync } from "node:sqlite";
import { getAddress } from "viem";
import type * as T from "../types.js";
import { candles, MARKS_MAX, POINTS_MAX, RES, units, windowOf, type Candle, type Range, type Res } from "./candles.js";
import { getMeta, sum } from "./db.js";
import { rateOf } from "./usd.js";
import type { CoinLive, Live, VaultLive } from "./live.js";

/** docs/phase4.md §3. Everything the API answers with. Indexed facts come from SQLite; `live` blocks from live.ts; formulas are here. */
export const MIN_DECLARATION_WEI = 5n * 10n ** 16n;   // the keeper's published policy (keeper/src/plan.ts): 0.05 ETH
export const MIN_ENTRY_WEI = 10n ** 16n;              // AssetVault.MIN_ENTRY
export const VAULT_SHARE_BPS = 8000n;                 // FeeRouter's constant
export const BASE_TAKE_BPS = 70;                      // measured: 0.70% of volume reaches a router before any creator tax (docs/phase1.md)

const A = (a: string): T.Address => getAddress(a) as T.Address;
const H = (h: string): T.Hex => h as T.Hex;
const DAY = 86_400;

/** docs/phase12.md §4. Which of the two the caller asked with: a range names a window and derives its bucket, `res` names a bucket. */
export type ChartSel = { res: Res } | { range: Range };

export type VolumeInputs = { kind: T.ListingKind; paused: boolean; blocked: boolean | null; capWei: bigint | null; capExpiry: number | null; nowS: number;
  spendableWei: bigint | null; coinUnspentWei: bigint | null; coinQueued: boolean | null; harvestableWei: bigint | null; taxBps: number | null };

/** The headline number, as a pure function of its inputs so that anyone can recompute it (and the tests do, by hand). */
export function volumeUntilNextBuy(i: VolumeInputs): T.VolumeUntilNextBuy {
  const s = (v: bigint | null) => (v === null ? null : String(v));
  const capLive = i.capWei !== null && i.capWei > 0n && i.capExpiry !== null && i.capExpiry > i.nowS;
  const takeBps = i.taxBps === null ? null : BASE_TAKE_BPS + i.taxBps;
  const waiting = i.coinQueued === false ? i.coinUnspentWei : i.coinQueued === true ? 0n : null;
  const base: T.VolumeUntilNextBuy["inputs"] = { targetWei: null, capWei: s(i.capWei), capExpiry: i.capExpiry, minDeclarationWei: String(MIN_DECLARATION_WEI),
    minEntryWei: String(MIN_ENTRY_WEI), spendableWei: s(i.spendableWei), coinWaitingWei: s(waiting), coinQueued: i.coinQueued, harvestableWei: s(i.harvestableWei),
    depositNeededWei: null, vaultShareBps: 8000, takeBps };
  const assumptions = [
    "only this coin trades: fees from other coins on the same listing bring the purchase sooner",
    "the cap posted now is still the cap then (a cap lasts one hour and is re-posted from the market)",
    `the share of volume reaching the router stays what was measured: 0.70% plus the coin's creator tax`,
    "fees earned but still inside the curve or the pool are not counted; fees on the router or in Pons's escrow are, at 80%",
    "the keeper declares as its published policy says: one item under the cap and never under 0.05 ETH; any amount over 0.05 ETH for a rune",
  ];
  const none = (reason: NonNullable<T.VolumeUntilNextBuy["reason"]>): T.VolumeUntilNextBuy => ({ volumeNeededWei: null, reason, ready: false, inputs: base, assumptions });
  if (i.paused) return none("listing-paused");
  if (i.blocked) return none("listing-blocked");
  if (i.spendableWei === null || i.coinQueued === null || i.coinUnspentWei === null || i.blocked === null) return none("live-unavailable");
  if (!capLive) return none("no-live-cap");
  if (takeBps === null) return none("tax-unknown");
  const target = i.kind === "inscriptions" ? (i.capWei! > MIN_DECLARATION_WEI ? i.capWei! : MIN_DECLARATION_WEI) : MIN_DECLARATION_WEI;
  const w = waiting ?? 0n;
  // what must still ARRIVE in the vault from this coin: enough to reach the target, and — for a coin not yet in the queue —
  // at least enough to reach MIN_ENTRY, since until then none of its fees are spendable at all
  let need = target - i.spendableWei - w;
  if (!i.coinQueued && MIN_ENTRY_WEI - w > need) need = MIN_ENTRY_WEI - w;
  if (need < 0n) need = 0n;
  // one harvest() away already: 80% of it lands in the vault
  const onHand = ((i.harvestableWei ?? 0n) * VAULT_SHARE_BPS) / 10_000n;
  need = need > onHand ? need - onHand : 0n;
  const inputs = { ...base, targetWei: String(target), depositNeededWei: String(need) };
  if (need === 0n) return { volumeNeededWei: "0", reason: null, ready: true, inputs, assumptions };
  // deposit = volume × take × 0.80  ⇒  volume = deposit × 10000 × 10000 / (takeBps × 8000), rounded up
  const denom = BigInt(takeBps) * VAULT_SHARE_BPS;
  const volume = (need * 10_000n * 10_000n + denom - 1n) / denom;
  return { volumeNeededWei: String(volume), reason: null, ready: false, inputs, assumptions };
}

type TradeRow = { block: number; log_index: number; coin: string; venue: "curve" | "pool"; side: "buy" | "sell"; quote: string; coins: string;
  fee: string | null; tax: string | null; price: number; trader: string; recipient: string | null; ts: number | null; tx: string };
type CoinRow = { coin: string; creator: string; listing_id: number; curve: string; router: string; deferred: string; block: number; ts: number | null; tx: string;
  name: string | null; symbol: string | null; tax_bps: number | null; pool_fee: number | null; tick_spacing: number | null; logo: string | null; supply: string | null;
  listing_name: string | null; vault: string | null };
const COIN_SQL = `SELECT c.*, m.name, m.symbol, m.tax_bps, m.pool_fee, m.tick_spacing, m.logo, m.supply, l.name AS listing_name, l.vault FROM coins c
  LEFT JOIN coin_meta m ON m.coin = c.coin LEFT JOIN listings l ON l.id = c.listing_id`;

export class Views {
  constructor(private readonly db: DatabaseSync, private readonly live: Live, private readonly now: () => number = () => Math.floor(Date.now() / 1000)) {}
  private all<R>(sql: string, ...p: (string | number)[]): R[] { return this.db.prepare(sql).all(...p) as R[]; }
  private one<R>(sql: string, ...p: (string | number)[]): R | undefined { return this.db.prepare(sql).get(...p) as R | undefined; }

  private coinSummary(c: CoinRow): T.CoinSummary {
    const t = this.now();
    const hs = this.all<{ to_vault: string; to_treasury: string; ts: number | null }>(`SELECT to_vault, to_treasury, ts FROM harvests WHERE coin = ?`, c.coin);
    const within = (s: number) => sum(hs.filter((h) => h.ts !== null && h.ts >= t - s).flatMap((h) => [h.to_vault, h.to_treasury]));
    const toVault = sum(hs.map((h) => h.to_vault)), toTreasury = sum(hs.map((h) => h.to_treasury));
    return { address: A(c.coin), name: c.name, symbol: c.symbol, creator: A(c.creator), router: A(c.router), curve: A(c.curve), listingId: c.listing_id,
      listingName: c.listing_name, launchedAt: c.ts, launchedBlock: c.block, creatorTaxBps: c.tax_bps, logoUrl: c.logo, supplyWei: c.supply,
      holders: this.holderCount(c),
      // docs/phase14.md §4: volume, not fees. `trades_by_coin_ts` is the index this asks along, and 0 is an answer.
      volume24hWei: sum(this.all<{ quote: string }>(`SELECT quote FROM trades WHERE coin = ? AND ts >= ?`, c.coin, t - DAY).map((x) => x.quote)),
      lifetimeFeesWei: sum([toVault, toTreasury]), toVaultWei: toVault,
      toTreasuryWei: toTreasury, fees24hWei: within(DAY), fees7dWei: within(7 * DAY), harvests: hs.length,
      lastHarvestAt: hs.reduce<number | null>((m, h) => (h.ts !== null && (m === null || h.ts > m) ? h.ts : m), null) };
  }

  coins(limit: number, offset: number, listingId?: number): T.Page<T.CoinSummary> {
    const where = listingId === undefined ? "" : " WHERE c.listing_id = ?", args = listingId === undefined ? [] : [listingId];
    const total = this.one<{ n: number }>(`SELECT count(*) AS n FROM coins c${where}`, ...args)!.n;
    const rows = this.all<CoinRow>(`${COIN_SQL}${where} ORDER BY c.block DESC, c.coin LIMIT ? OFFSET ?`, ...args, limit, offset);
    return { items: rows.map((r) => this.coinSummary(r)), total, limit, offset };
  }

  async coin(address: string): Promise<T.CoinDetail | null> {
    const c = this.one<CoinRow>(`${COIN_SQL} WHERE c.coin = ?`, address.toLowerCase());
    if (!c) return null;
    const l = this.one<{ kind: T.ListingKind; paused: number }>(`SELECT kind, paused FROM listings WHERE id = ?`, c.listing_id);
    const [lc, lv]: [CoinLive | null, VaultLive | null] = c.vault
      ? await Promise.all([this.live.coin({ coin: c.coin, router: c.router, curve: c.curve, vault: c.vault, poolFee: c.pool_fee, tickSpacing: c.tick_spacing }), this.live.vault(c.vault)])
      : [null, null];
    const big = (v: string | null | undefined) => (v === null || v === undefined ? null : BigInt(v));
    const harvestable = lc && (lc.escrowWei !== null || lc.routerBalanceWei !== null) ? (big(lc.escrowWei) ?? 0n) + (big(lc.routerBalanceWei) ?? 0n) : null;
    const volume = volumeUntilNextBuy({ kind: l?.kind ?? "inscriptions", paused: !!l?.paused, blocked: lv?.blocked ?? null, capWei: big(lv?.capWei), capExpiry: lv?.capExpiry ?? null,
      nowS: this.now(), spendableWei: big(lv?.spendableWei), coinUnspentWei: big(lc?.unspentWei), coinQueued: lc?.queued ?? null, harvestableWei: harvestable, taxBps: c.tax_bps });
    const charges = this.all<{ listing_id: number; purchase_id: number; amount: string }>(`SELECT listing_id, purchase_id, amount FROM charges WHERE coin = ?`, c.coin);
    const acquisitions: T.CoinDetail["acquisitions"] = [];
    for (const ch of charges) {
      const total = BigInt(sum(this.all<{ amount: string }>(`SELECT amount FROM charges WHERE listing_id = ? AND purchase_id = ?`, ch.listing_id, ch.purchase_id).map((x) => x.amount)));
      for (const q of this.all<{ acquisition_id: number; asset_ref: string; amount: string }>(`SELECT acquisition_id, asset_ref, amount FROM acquisitions WHERE listing_id = ? AND purchase_id = ?`,
        ch.listing_id, ch.purchase_id)) {
        acquisitions.push({ listingId: ch.listing_id, acquisitionId: q.acquisition_id, purchaseId: ch.purchase_id, chargedWei: ch.amount,
          shareBps: total === 0n ? 0 : Number((BigInt(ch.amount) * 10_000n) / total), assetRef: q.asset_ref, amount: q.amount });
      }
    }
    return { ...this.coinSummary(c),
      harvestsRecent: this.all<{ ts: number | null; block: number; tx: string; claimed: string; to_vault: string; to_treasury: string }>(
        `SELECT ts, block, tx, claimed, to_vault, to_treasury FROM harvests WHERE coin = ? ORDER BY block DESC, log_index DESC LIMIT 50`, c.coin)
        .map((h) => ({ at: h.ts, block: h.block, tx: H(h.tx), claimedWei: h.claimed, toVaultWei: h.to_vault, toTreasuryWei: h.to_treasury })),
      tokensForwarded: this.all<{ ts: number | null; tx: string; token: string; amount: string }>(`SELECT ts, tx, token, amount FROM forwards WHERE coin = ? ORDER BY block DESC LIMIT 50`, c.coin)
        .map((f) => ({ at: f.ts, tx: H(f.tx), token: A(f.token), amount: f.amount })),
      treasuryDeferredWei: c.deferred, acquisitions, volumeUntilNextBuy: volume,
      live: lc === null ? null : { unspentWei: lc.unspentWei, queued: lc.queued, pending: { graduated: lc.graduated, escrowWei: lc.escrowWei, routerBalanceWei: lc.routerBalanceWei,
        pool: lc.pool, curvePending: null, curvePendingReason: "before graduation fees accrue inside Pons's curve, and no getter for that amount has been verified", error: lc.error } } };
  }

  // ---- docs/phase13.md: who holds a coin, folded from its own Transfer logs

  /**
   * docs/phase13.md §4. Whether every block's transfers are indexed. Until they are, a holder count would be the tail of a
   * coin's history rather than its history, so every number derived from balances says so instead of standing behind itself.
   */
  private scan(): { complete: boolean; through: number | null; from: number | null } {
    const num = (k: string) => { const v = getMeta(this.db, k); return v === null ? null : Number(v); };
    const through = num("transfers_backfilled_through"), live = num("transfers_live_from");
    // no marks at all: a database nothing has ever followed (the synthetic set, a test). Nothing was missed because nothing
    // was ever skipped — every event it holds has been folded.
    if (through === null || live === null) return { complete: true, through: null, from: null };
    return { complete: through >= live, through, from: through + 1 };
  }

  /**
   * docs/phase13.md §3. The addresses a coin's holder list leaves out, and why each one.
   *
   * Every one of them is ours, Pons's or Uniswap's, and every one is learnt from something already indexed — the coin's own
   * row, its pool's `Initialize` log, the factory in the deployment file — so finding them costs no read. They are not
   * hidden: the response carries each one with its balance, because a reader who can see that the listed shares add up to
   * 14% of the supply is owed the reason.
   */
  private plumbing(c: { coin: string; curve: string; router: string }): { address: string; why: T.HolderExclusion }[] {
    const out: { address: string; why: T.HolderExclusion }[] = [
      { address: c.coin, why: "coin" }, { address: c.curve, why: "curve" }, { address: c.router, why: "router" },
    ];
    const factory = getMeta(this.db, "pons_factory"), manager = getMeta(this.db, "pool_manager");
    if (factory) out.push({ address: factory, why: "pons-factory" });
    if (manager) out.push({ address: manager, why: "pool-manager" });
    for (const p of this.all<{ hooks: string | null }>(`SELECT hooks FROM pools WHERE coin = ?`, c.coin)) if (p.hooks) out.push({ address: p.hooks, why: "hook" });
    const seen = new Set<string>();
    return out.filter((x) => (seen.has(x.address) ? false : (seen.add(x.address), true)));
  }

  /** How many addresses hold a positive balance of this coin, the plumbing excluded. Null while the scan is incomplete. */
  private holderCount(c: { coin: string; curve: string; router: string }): number | null {
    if (!this.scan().complete) return null;
    const ex = this.plumbing(c).map((x) => x.address);
    return this.one<{ n: number }>(`SELECT count(*) AS n FROM balances WHERE coin = ? AND amt > 0 AND holder NOT IN (${ex.map(() => "?").join(", ")})`, c.coin, ...ex)!.n;
  }

  /**
   * A page of a coin's largest holders. `rank` is over the whole list rather than the page, so page two starts at 26, and
   * `share` is a fraction of the supply Phase 12 reads — null, never a NaN and never a guessed denominator, when that read
   * has not succeeded.
   */
  holders(address: string, limit: number, offset: number): T.HolderPage | null {
    const c = this.one<{ coin: string; curve: string; router: string; creator: string; supply: string | null }>(
      `SELECT c.coin, c.curve, c.router, c.creator, m.supply FROM coins c LEFT JOIN coin_meta m ON m.coin = c.coin WHERE c.coin = ?`, address.toLowerCase());
    if (!c) return null;
    const ex = this.plumbing(c), exAddrs = ex.map((x) => x.address);
    const holes = `AND holder NOT IN (${exAddrs.map(() => "?").join(", ")})`;
    const total = this.one<{ n: number }>(`SELECT count(*) AS n FROM balances WHERE coin = ? AND amt > 0 ${holes}`, c.coin, ...exAddrs)!.n;
    const rows = this.all<{ holder: string; amount: string }>(
      `SELECT holder, amount FROM balances WHERE coin = ? AND amt > 0 ${holes} ORDER BY amt DESC, holder LIMIT ? OFFSET ?`, c.coin, ...exAddrs, limit, offset);
    const supply = c.supply === null ? null : BigInt(c.supply);
    // eight decimals of a fraction is a hundredth of a basis point — finer than any screen prints — and the arithmetic to
    // get there is in bigints, so a 27-digit balance is not rounded on its way to a percentage
    const share = (v: bigint) => (supply === null || supply === 0n ? null : Number((v * 100_000_000n) / supply) / 1e8);
    const scan = this.scan();
    return { coin: A(c.coin), holders: scan.complete ? total : null, supplyWei: c.supply, creator: A(c.creator),
      complete: scan.complete, scannedThrough: scan.through, transfersFrom: scan.from,
      items: rows.map((r, i) => ({ rank: offset + i + 1, address: A(r.holder), balanceWei: r.amount, share: share(BigInt(r.amount)), isCreator: r.holder === c.creator })),
      excluded: ex.flatMap((x) => {
        const held = this.one<{ amount: string }>(`SELECT amount FROM balances WHERE coin = ? AND holder = ? AND amt > 0`, c.coin, x.address);
        return held ? [{ address: A(x.address), why: x.why, balanceWei: held.amount, share: share(BigInt(held.amount)) }] : [];
      }),
      total, limit, offset };
  }

  // ---- docs/phase10.md: a coin's trades, and the chart drawn from them

  /** Whether a coin was ever launched here: a trades page for one that was not is a 404, not an empty page. */
  coinExists(address: string): boolean { return this.one<{ n: number }>(`SELECT count(*) AS n FROM coins WHERE coin = ?`, address.toLowerCase())!.n > 0; }

  private tradeOut(x: TradeRow): T.Trade {
    return { id: `${x.block}:${x.log_index}`, side: x.side, venue: x.venue, quoteWei: x.quote, coinWei: x.coins, feeWei: x.fee, taxWei: x.tax,
      price: x.price, trader: A(x.trader), recipient: x.recipient === null ? null : A(x.recipient), tx: H(x.tx), block: x.block, logIndex: x.log_index, at: x.ts };
  }
  private tradeRows(coin: string, where: string, ...args: (string | number)[]): T.Trade[] {
    return this.all<TradeRow>(`SELECT * FROM trades WHERE coin = ? ${where}`, coin, ...args).map((x) => this.tradeOut(x));
  }

  /**
   * One page of a coin's trades, newest first. `before` is a cursor of the form "block:logIndex" — the place in the chain of
   * the last trade of the page before it — so a page is stable however many trades land while it is being read.
   */
  trades(address: string, limit: number, before?: string | null): T.TradePage {
    const coin = address.toLowerCase();
    const cur = /^(\d{1,12}):(\d{1,6})$/.exec(before ?? "");
    const order = `ORDER BY block DESC, log_index DESC LIMIT ?`;
    const rows = cur
      ? this.tradeRows(coin, `AND (block < ? OR (block = ? AND log_index < ?)) ${order}`, Number(cur[1]), Number(cur[1]), Number(cur[2]), limit + 1)
      : this.tradeRows(coin, order, limit + 1);
    const total = this.one<{ n: number }>(`SELECT count(*) AS n FROM trades WHERE coin = ?`, coin)!.n;
    return { items: rows.slice(0, limit), next: rows.length > limit ? rows[limit - 1].id : null, total };
  }

  /**
   * A coin's chart. The candles are computed here, from the stored trades, so every reader draws the same line. A coin that has
   * never traded gets an empty series and `trades: 0` — never an invented price, and never a flat line at zero.
   *
   * docs/phase12.md §4: a caller picks a RANGE, and the bucket follows from it, so one-candle-per-trade is not reachable. The
   * old `res` is still accepted, unchanged, because the deployed site asks for it and this must not break under it.
   */
  chart(address: string, sel: ChartSel): T.Chart | null {
    const coin = address.toLowerCase();
    const c = this.one<{ ts: number | null; graduated_ts: number | null; graduated_block: number | null; graduated_tx: string | null; symbol: string | null;
      pair_token: string | null; supply: string | null }>(
      `SELECT c.ts, c.graduated_ts, c.graduated_block, c.graduated_tx, m.symbol, m.pair_token, m.supply FROM coins c LEFT JOIN coin_meta m ON m.coin = c.coin WHERE c.coin = ?`, coin);
    if (!c) return null;
    const range: Range | null = "range" in sel ? sel.range : null;
    const r = "range" in sel ? windowOf(sel.range, c.ts, this.now()) : RES[sel.res];
    // the legacy label nearest what was actually drawn, so a client written against the old shape still parses this one
    const res: Res = "range" in sel ? (r.seconds >= RES["1h"].seconds ? "1h" : "5m") : sel.res;
    const since = this.now() - r.span;
    const order = r.seconds ? `ORDER BY block, log_index` : `ORDER BY block DESC, log_index DESC LIMIT ${POINTS_MAX}`;
    const rows = this.all<{ ts: number | null; price: number; quote: string }>(`SELECT ts, price, quote FROM trades WHERE coin = ? AND ts >= ? ${order}`, coin, since);
    const chain = r.seconds ? rows : rows.reverse();
    const cs: Candle[] = candles(chain.filter((x) => x.ts !== null).map((x) => ({ ts: x.ts!, p: x.price, v: units(x.quote) })), r.seconds);
    const marks = this.tradeRows(coin, `AND ts >= ? ORDER BY CAST(quote AS REAL) DESC LIMIT ?`, since, MARKS_MAX)
      .sort((a, b) => a.block - b.block || a.logIndex - b.logIndex);
    // docs/phase12.md §3: the other two numbers a market cap needs, both read on chain and both allowed to be null
    const rate = rateOf(this.db, this.now());
    return { res, range, coin: A(coin), quote: "ETH", priceIs: "pair asset per whole coin", candles: cs, marks,
      supplyWei: c.supply, usdPerEth: rate.usdPerEth, usdPerEthAt: rate.at, usdPool: rate.pool === null ? null : A(rate.pool),
      trades: this.one<{ n: number }>(`SELECT count(*) AS n FROM trades WHERE coin = ? AND ts >= ?`, coin, since)!.n,
      tradesAllTime: this.one<{ n: number }>(`SELECT count(*) AS n FROM trades WHERE coin = ?`, coin)!.n,
      launchedAt: c.ts, span: r.span, seconds: r.seconds,
      graduation: c.graduated_ts === null && c.graduated_block === null ? null
        : { at: c.graduated_ts, block: c.graduated_block, tx: c.graduated_tx === null ? null : H(c.graduated_tx) },
      pool: this.one<{ pool_id: string }>(`SELECT pool_id FROM pools WHERE coin = ?`, coin)?.pool_id ?? null };
  }

  private listingSummary(l: ListingRow): T.ListingSummary {
    const cap = this.one<{ cap: string; expiry: number; ts: number | null }>(`SELECT cap, expiry, ts FROM caps WHERE listing_id = ? ORDER BY block DESC, log_index DESC LIMIT 1`, l.id);
    const meta = this.one<{ divisibility: number | null }>(`SELECT divisibility FROM listing_meta WHERE id = ?`, l.id);
    return { id: l.id, kind: l.kind, name: l.name, ref: l.ref, divisibility: meta?.divisibility ?? null, vault: A(l.vault), btcVault: l.btc_vault, membershipRoot: l.root ? H(l.root) : null,
      paused: !!l.paused, coins: this.one<{ n: number }>(`SELECT count(*) AS n FROM coins WHERE listing_id = ?`, l.id)!.n,
      depositedWei: sum(this.all<{ amount: string }>(`SELECT amount FROM deposits WHERE listing_id = ?`, l.id).map((d) => d.amount)),
      acquisitions: this.one<{ n: number }>(`SELECT count(*) AS n FROM acquisitions WHERE listing_id = ?`, l.id)!.n,
      // the contract emits nothing when a cap expires: expiry is this comparison, made when the question is asked
      capWei: cap?.cap ?? null, capExpiry: cap?.expiry ?? null, capExpired: cap ? cap.expiry <= this.now() : null,
      // docs/phase14.md §8.4: the same hour told from its other end, so a reader can see how fresh the keeper's word is
      capPostedAt: cap?.ts ?? null,
      openDeclarations: this.one<{ n: number }>(`SELECT count(*) AS n FROM purchases WHERE listing_id = ? AND status IN ('declared', 'released')`, l.id)!.n,
      // docs/v2/vault-v2.md: which vault this listing got, from the chain's own word rather than inferred from what the row
      // does or does not carry. A v1 listing is `false` with both other fields null.
      autonomous: !!l.autonomous, buyer: l.buyer ? A(l.buyer) : null, maxInFlightWei: l.max_in_flight ?? null };
  }

  listings(): T.Page<T.ListingSummary> {
    const rows = this.all<ListingRow>(`SELECT * FROM listings ORDER BY id`);
    return { items: rows.map((l) => this.listingSummary(l)), total: rows.length, limit: rows.length, offset: 0 };
  }

  private declaration(p: PurchaseRow): T.Declaration {
    const t = this.now();
    return { purchaseId: p.purchase_id, status: p.status, maxEthWei: p.max_eth, quantity: p.quantity, capRef: p.cap_ref, declaredAt: p.declared_ts, releasableAt: p.releasable_at,
      releasableIn: p.status === "declared" ? Math.max(0, p.releasable_at - t) : null, releasedAt: p.released_ts, releasedWei: p.released, purchaser: p.purchaser ? A(p.purchaser) : null,
      deadline: p.deadline, overdue: p.status === "released" && p.deadline !== null && p.deadline < t, returnedWei: p.returned, writtenOffWei: p.written_off, bondSlashedWei: p.slashed,
      charges: this.all<{ coin: string; amount: string }>(`SELECT coin, amount FROM charges WHERE listing_id = ? AND purchase_id = ? ORDER BY seq`, p.listing_id, p.purchase_id)
        .map((c) => ({ coin: A(c.coin), amountWei: c.amount })), tx: H(p.tx) };
  }

  /**
   * docs/phase14.md §8.3. A rune listing's units, in the rune's own raw units: what the receipts brought in, what a
   * resolved draw delivered, what a pro-rata claim was paid out, and what is therefore still the vault's.
   *
   * Amounts sum across a rune settle's rows and `satsPaid` does not — one Bitcoin transaction is recorded once per paying
   * coin, each row carrying the same sats — so this adds `amount` and nothing else. It is the figure `Verify.expectAtLeast`
   * hands a runes indexer and the figure the vault page prints as "held", computed once so the two cannot disagree.
   */
  private runeUnits(listingId: number): NonNullable<T.Holdings["runeUnits"]> {
    const received = sum(this.all<{ amount: string }>(`SELECT amount FROM acquisitions WHERE listing_id = ?`, listingId).map((x) => x.amount));
    const paidOut = sum(this.all<{ amount: string }>(`SELECT amount FROM payouts WHERE listing_id = ? AND txid IS NOT NULL`, listingId).map((x) => x.amount));
    const delivered = sum(this.all<{ amount: string }>(`SELECT q.amount FROM acquisitions q JOIN lots l ON l.listing_id = q.listing_id AND l.acquisition_id = q.acquisition_id
      WHERE q.listing_id = ? AND l.state = 'delivered'`, listingId).map((x) => x.amount));
    return { received, delivered, paidOut, held: String(BigInt(received) - BigInt(paidOut) - BigInt(delivered)) };
  }

  /** What this vault has bought and what of it it is still holding. A count for a collection, a quantity for a rune. */
  private holdings(listingId: number, kind: T.ListingKind): T.Holdings {
    const receipts = this.one<{ n: number }>(`SELECT count(*) AS n FROM acquisitions WHERE listing_id = ?`, listingId)!.n;
    const delivered = this.one<{ n: number }>(`SELECT count(*) AS n FROM lots l JOIN acquisitions q ON q.listing_id = l.listing_id AND q.acquisition_id = l.acquisition_id
      WHERE l.listing_id = ? AND l.state = 'delivered'`, listingId)!.n;
    return { receipts, delivered, inscriptionsHeld: kind === "rune" ? null : receipts - delivered, runeUnits: kind === "rune" ? this.runeUnits(listingId) : null };
  }

  /** What a reader, or a program, can check for themselves. This package makes none of these requests (docs/phase4.md §4). */
  private verify(kind: T.ListingKind, a: AcqRow, btcVault: string | null): T.Verify {
    const links = [{ label: "the Bitcoin transaction", url: `https://mempool.space/tx/${a.btc_txid}` }];
    if (btcVault) links.push({ label: "the listing's Bitcoin address", url: `https://mempool.space/address/${btcVault}` });
    if (kind === "inscriptions") {
      links.push({ label: "the inscription", url: `https://ordinals.com/inscription/${a.asset_ref}` });
      return { links, check: { kind: "inscription", txid: a.btc_txid, inscriptionId: a.asset_ref, address: btcVault, expect: "the inscription's current owner is `address`, until it is delivered" },
        note: "A receipt is the attestor's word. These links let anyone check it against Bitcoin; this API has not." };
    }
    links.push({ label: "the rune", url: `https://ordinals.com/rune/${a.asset_ref}` });
    const held = BigInt(this.runeUnits(a.listing_id).held);
    return { links, check: { kind: "rune", txid: a.btc_txid, runeId: a.asset_ref, address: btcVault, expectAtLeast: String(held < 0n ? 0n : held),
      indexerPath: btcVault ? `/v1/indexer/address/${btcVault}/runes/${a.asset_ref}/balance` : "",
      expect: "the address's balance of the rune is at least the undelivered total" },
      note: "`indexerPath` is UniSat's documented Open API path (it needs the reader's own key); any runes indexer answers the same question. A receipt is the attestor's word; this API has not checked it." };
  }

  private acquisition(a: AcqRow): T.Acquisition {
    const l = this.one<{ kind: T.ListingKind; btc_vault: string | null }>(`SELECT kind, btc_vault FROM listings WHERE id = ?`, a.listing_id);
    const lot = this.one<LotRow>(`SELECT * FROM lots WHERE listing_id = ? AND acquisition_id = ?`, a.listing_id, a.acquisition_id);
    const sym = this.one<{ symbol: string | null }>(`SELECT symbol FROM coin_meta WHERE coin = ?`, a.coin)?.symbol ?? null;
    // docs/phase14.md §8.4: the vault charges per PURCHASE, so a receipt carries its purchase's figure and the number of
    // receipts sharing it, and divides neither by the other
    const p = this.one<{ released: string | null; returned: string }>(`SELECT released, returned FROM purchases WHERE listing_id = ? AND purchase_id = ?`, a.listing_id, a.purchase_id);
    const siblings = this.one<{ n: number }>(`SELECT count(*) AS n FROM acquisitions WHERE listing_id = ? AND purchase_id = ?`, a.listing_id, a.purchase_id)!.n;
    return { listingId: a.listing_id, acquisitionId: a.acquisition_id, purchaseId: a.purchase_id, kind: l?.kind ?? "inscriptions", coin: A(a.coin), coinSymbol: sym, assetRef: a.asset_ref,
      amount: a.amount, btcTxid: a.btc_txid, satsPaid: a.sats, settledAt: a.ts, tx: H(a.tx),
      purchaseSpentWei: String(BigInt(p?.released ?? "0") - BigInt(p?.returned ?? "0")), purchaseAcquisitions: siblings,
      delivery: { mode: (lot?.mode ?? "unset") as T.Acquisition["delivery"]["mode"], state: (lot?.state ?? "undecided") as T.Acquisition["delivery"]["state"],
        winner: lot?.winner ? A(lot.winner) : null, destination: lot?.destination ?? null, deliveredTxid: lot?.delivered_txid ?? null, overdue: !!lot?.overdue,
        secondsToDeliver: lot?.delivered_ts && lot.destination_ts ? lot.delivered_ts - lot.destination_ts : null },
      verify: this.verify(l?.kind ?? "inscriptions", a, l?.btc_vault ?? null) };
  }

  acquisitions(limit: number, offset: number, listingId?: number): T.Page<T.Acquisition> {
    const where = listingId === undefined ? "" : " WHERE listing_id = ?", args = listingId === undefined ? [] : [listingId];
    const total = this.one<{ n: number }>(`SELECT count(*) AS n FROM acquisitions${where}`, ...args)!.n;
    return { items: this.all<AcqRow>(`SELECT * FROM acquisitions${where} ORDER BY block DESC, acquisition_id DESC LIMIT ? OFFSET ?`, ...args, limit, offset).map((a) => this.acquisition(a)),
      total, limit, offset };
  }

  async listing(id: number): Promise<T.ListingDetail | null> {
    const l = this.one<ListingRow>(`SELECT * FROM listings WHERE id = ?`, id);
    if (!l) return null;
    const lv = await this.live.vault(l.vault);
    const times = this.all<{ s: number }>(`SELECT delivered_ts - destination_ts AS s FROM lots WHERE listing_id = ? AND delivered_ts IS NOT NULL AND destination_ts IS NOT NULL
      UNION ALL SELECT recorded_ts - requested_ts AS s FROM payouts WHERE listing_id = ? AND recorded_ts IS NOT NULL AND requested_ts IS NOT NULL`, id, id).map((r) => r.s).sort((x, y) => x - y);
    const min = this.one<{ units: string; expiry: number }>(`SELECT units, expiry FROM minimums WHERE listing_id = ?`, id);
    const out = lv?.outstandingWei ? BigInt(lv.outstandingWei) : null;
    const ps = this.all<PurchaseRow>(`SELECT * FROM purchases WHERE listing_id = ?`, id), settled = ps.filter((p) => p.status === "settled");
    return { ...this.listingSummary(l),
      live: lv === null ? null : { balanceWei: lv.balanceWei, attributedWei: lv.attributedWei, waitingWei: lv.waitingWei, reservedWei: lv.reservedWei, spendableWei: lv.spendableWei,
        bondWei: lv.bondWei, outstandingWei: lv.outstandingWei, requiredBondWei: lv.requiredBondWei, blockedByPurchase: lv.blockedByPurchase, error: lv.error },
      bondCoverageBps: lv?.bondWei && out !== null && out > 0n ? Number((BigInt(lv.bondWei) * 10_000n) / out) : null,
      bondEvents: this.all<{ ts: number | null; tx: string; type: "posted" | "withdrawn" | "slashed"; who: string | null; amount: string }>(
        `SELECT ts, tx, type, who, amount FROM bond_events WHERE listing_id = ? ORDER BY block DESC, log_index DESC LIMIT 50`, id)
        .map((b) => ({ at: b.ts, tx: H(b.tx), type: b.type, who: b.who ? A(b.who) : null, amountWei: b.amount })),
      capHistory: this.all<{ ts: number | null; cap: string; expiry: number }>(`SELECT ts, cap, expiry FROM caps WHERE listing_id = ? ORDER BY block DESC, log_index DESC LIMIT 48`, id)
        .map((c) => ({ at: c.ts, capWei: c.cap, expiry: c.expiry })),
      declarations: this.all<PurchaseRow>(`SELECT * FROM purchases WHERE listing_id = ? ORDER BY purchase_id DESC LIMIT 100`, id).map((p) => this.declaration(p)),
      acquisitionsRecent: this.all<AcqRow>(`SELECT * FROM acquisitions WHERE listing_id = ? ORDER BY acquisition_id DESC LIMIT 50`, id).map((a) => this.acquisition(a)),
      // docs/phase14.md §8.4: over EVERY purchase, not over the hundred rows of `declarations` above, and `spentWei` is the
      // same definition /v1/stats uses for the whole protocol
      releasedWei: sum(ps.map((p) => p.released)), returnedWei: sum(ps.map((p) => p.returned)),
      spentWei: String(BigInt(sum(settled.map((p) => p.released))) - BigInt(sum(settled.map((p) => p.returned)))),
      holdings: this.holdings(id, l.kind),
      writtenOffWei: sum(ps.map((p) => p.written_off)),
      timeToDeliver: { count: times.length, medianS: times.length ? times[Math.floor((times.length - 1) / 2)] : null, maxS: times.length ? times[times.length - 1] : null },
      minimumPayout: min ? { units: min.units, expiry: min.expiry, expired: min.expiry <= this.now() } : null,
      pending: { root: l.pending_root && l.pending_root_at ? { value: H(l.pending_root), readyAt: l.pending_root_at } : null,
        btcVault: l.pending_btc && l.pending_btc_at ? { value: l.pending_btc, readyAt: l.pending_btc_at } : null },
      coinsList: this.all<{ coin: string; symbol: string | null }>(`SELECT c.coin, m.symbol FROM coins c LEFT JOIN coin_meta m ON m.coin = c.coin WHERE c.listing_id = ? ORDER BY c.block`, id)
        .map((c) => ({ address: A(c.coin), symbol: c.symbol,
          depositedWei: sum(this.all<{ amount: string }>(`SELECT amount FROM deposits WHERE listing_id = ? AND coin = ?`, id, c.coin).map((d) => d.amount)) })) };
  }

  private drawSummary(l: LotRow): T.DrawSummary | null {
    const d = this.one<DistRow>(`SELECT * FROM distributions WHERE listing_id = ? AND acquisition_id = ?`, l.listing_id, l.acquisition_id);
    const isDist = !!d, opened = isDist ? d!.opened_ts : l.opened_ts;
    if (!isDist && l.root === null && !["won", "delivered"].includes(l.state)) return null;   // nothing was ever opened or won for it
    const ends = isDist ? d!.challenge_ends : l.challenge_ends;
    const ref = this.one<{ asset_ref: string }>(`SELECT asset_ref FROM acquisitions WHERE listing_id = ? AND acquisition_id = ?`, l.listing_id, l.acquisition_id)?.asset_ref ?? null;
    return { id: `${l.listing_id}-${l.acquisition_id}`, listingId: l.listing_id, acquisitionId: l.acquisition_id, type: isDist ? "distribution" : "draw",
      state: l.state as T.DrawSummary["state"], mode: l.mode as T.DrawSummary["mode"], assetRef: ref, openedAt: opened, challengeEnds: ends,
      // a window only counts down while the thing is still open; afterwards it is history
      challengeEndsIn: ends === null || !(l.state === "draw-open" || l.state === "distribution-open") ? null : Math.max(0, ends - this.now()), root: (isDist ? d!.root : l.root) ? H((isDist ? d!.root : l.root)!) : null, uri: isDist ? d!.uri : l.uri,
      total: isDist ? d!.total : l.total };
  }

  draws(limit: number, offset: number, listingId?: number): T.Page<T.DrawSummary> {
    const where = listingId === undefined ? "" : " WHERE listing_id = ?", args = listingId === undefined ? [] : [listingId];
    const all = this.all<LotRow>(`SELECT * FROM lots${where} ORDER BY listing_id DESC, acquisition_id DESC`, ...args).map((l) => this.drawSummary(l)).filter((x): x is T.DrawSummary => x !== null);
    return { items: all.slice(offset, offset + limit), total: all.length, limit, offset };
  }

  draw(listingId: number, acquisitionId: number): T.DrawDetail | null {
    const l = this.one<LotRow>(`SELECT * FROM lots WHERE listing_id = ? AND acquisition_id = ?`, listingId, acquisitionId);
    const s = l ? this.drawSummary(l) : null;
    if (!l || !s) return null;
    const d = this.one<DistRow>(`SELECT * FROM distributions WHERE listing_id = ? AND acquisition_id = ?`, listingId, acquisitionId);
    const coin = this.one<{ coin: string }>(`SELECT coin FROM acquisitions WHERE listing_id = ? AND acquisition_id = ?`, listingId, acquisitionId)?.coin ?? null;
    const kind = d ? "pro-rata" : l.mode === "draw-per-wallet" ? "per-wallet" : "weighted";
    const timeline = this.all<{ ts: number | null; block_number: number; tx: string; name: string; args: string }>(
      `SELECT ts, block_number, tx, name, args FROM events WHERE listing_id = ? AND acquisition_id = ? ORDER BY block_number, log_index`, listingId, acquisitionId)
      .map((e) => ({ at: e.ts, block: e.block_number, tx: H(e.tx), event: e.name, detail: JSON.parse(e.args) as Record<string, string | number | boolean | null> }));
    return { ...s,
      verifySnapshot: { tool: "tools/snapshot", command: coin && s.root
        ? `npm run snapshot -- --coin ${A(coin)} --block <block from the published file> --kind ${kind}${kind === "pro-rata" && s.total ? ` --total ${s.total}` : ""}` : null,
        note: `The published file at \`uri\` names the block and the other inputs. Run the command with your own RPC and compare its root with \`root\`${
          s.challengeEndsIn ? ` — ${s.challengeEndsIn} s of the challenge window remain` : ""}. What it cannot prove: that the block was the one promised.` },
      draw: d ? null : { round: l.round, pinnedAt: l.pinned_ts, drandUrl: l.round ? `https://api.drand.sh/04f1e9062b8a81f848fded9c12306733282b2727ecced50032187751166ec8c3/public/${l.round}` : null,
        winningTicket: l.winning_ticket, resolvedAt: l.resolved_ts, winner: l.winner ? A(l.winner) : null, destination: l.destination, destinationSetAt: l.destination_ts,
        deliveredTxid: l.delivered_txid, deliveredAt: l.delivered_ts, overdue: !!l.overdue, voided: l.voided_ticket !== null ? { unclaimedTicket: l.voided_ticket, at: l.voided_ts } : null },
      distribution: d ? { claimedUnits: d.claimed, claims: d.claims, rolledOver: d.rolled_coin ? { coin: A(d.rolled_coin), amount: d.rolled_amount ?? "0", at: d.rolled_ts } : null } : null,
      timeline };
  }

  stats(): T.Stats {
    const hs = this.all<{ claimed: string; to_vault: string; to_treasury: string }>(`SELECT claimed, to_vault, to_treasury FROM harvests`);
    const ps = this.all<PurchaseRow>(`SELECT * FROM purchases`);
    const count = (sql: string, ...p: (string | number)[]) => this.one<{ n: number }>(sql, ...p)!.n;
    const lotsBy = (state: string) => count(`SELECT count(*) AS n FROM lots WHERE state = ?`, state);
    const settled = ps.filter((p) => p.status === "settled");
    const released = sum(ps.map((p) => p.released)), returned = sum(ps.map((p) => p.returned));
    return { coins: count(`SELECT count(*) AS n FROM coins`), listings: count(`SELECT count(*) AS n FROM listings`), claimedWei: sum(hs.map((h) => h.claimed)),
      toVaultsWei: sum(hs.map((h) => h.to_vault)), toTreasuryWei: sum(hs.map((h) => h.to_treasury)), releasedWei: released, returnedWei: returned,
      spentWei: String(BigInt(sum(settled.map((p) => p.released))) - BigInt(sum(settled.map((p) => p.returned)))),
      writtenOffWei: sum(ps.map((p) => p.written_off)),
      acquisitions: { inscriptions: count(`SELECT count(*) AS n FROM acquisitions q JOIN listings l ON l.id = q.listing_id WHERE l.kind = 'inscriptions'`),
        runeReceipts: count(`SELECT count(*) AS n FROM acquisitions q JOIN listings l ON l.id = q.listing_id WHERE l.kind = 'rune'`) },
      purchases: { declared: ps.filter((p) => p.status === "declared").length, released: ps.filter((p) => p.status === "released").length, settled: settled.length,
        cancelled: ps.filter((p) => p.status === "cancelled").length, writtenOff: ps.filter((p) => p.status === "written-off").length },
      draws: { open: lotsBy("draw-open"), pinned: lotsBy("draw-pinned"), resolved: lotsBy("draw-resolved"), claimed: lotsBy("won"), delivered: lotsBy("delivered"), voided: lotsBy("voided"),
        cancelled: lotsBy("cancelled") },
      distributions: { open: count(`SELECT count(*) AS n FROM distributions WHERE state = 'open'`), rolled: count(`SELECT count(*) AS n FROM distributions WHERE state = 'rolled'`) },
      deliveries: { done: lotsBy("delivered"), overdue: count(`SELECT count(*) AS n FROM lots WHERE overdue = 1 AND state != 'delivered'`),
        waiting: count(`SELECT count(*) AS n FROM lots WHERE state = 'won' AND destination IS NOT NULL`) },
      payouts: { requested: count(`SELECT count(*) AS n FROM payouts`), recorded: count(`SELECT count(*) AS n FROM payouts WHERE txid IS NOT NULL`),
        overdue: count(`SELECT count(*) AS n FROM payouts WHERE overdue = 1 AND txid IS NULL`) } };
  }

  health(f: { lastPassAt: number | null; lastError: { at: number; kind: string } | null; head: number | null; reorgs: number; idle: boolean }, confirmations: number, version: string): T.Health {
    const cursor = getMeta(this.db, "cursor"), chain = getMeta(this.db, "chain_id");
    const c = cursor === null ? null : Number(cursor);
    return { ok: f.lastError === null, deployment: !f.idle, chainId: chain === null ? null : Number(chain), demo: getMeta(this.db, "demo") === "1", cursor: c, head: f.head,
      lagBlocks: c !== null && f.head !== null ? Math.max(0, f.head - c) : null, confirmations, events: this.one<{ n: number }>(`SELECT count(*) AS n FROM events`)!.n,
      lastPassAt: f.lastPassAt, lastError: f.lastError, reorgs: f.reorgs, version };
  }
}

type ListingRow = { id: number; kind: T.ListingKind; vault: string; name: string; ref: string; paused: number; btc_vault: string | null; root: string | null;
  autonomous: number; buyer: string | null; max_in_flight: string | null;
  pending_root: string | null; pending_root_at: number | null; pending_btc: string | null; pending_btc_at: number | null };
type PurchaseRow = { listing_id: number; purchase_id: number; status: T.Declaration["status"]; max_eth: string; quantity: string; releasable_at: number; cap_ref: string;
  declared_ts: number | null; tx: string; released_ts: number | null; released: string | null; purchaser: string | null; deadline: number | null; returned: string;
  written_off: string | null; slashed: string };
type AcqRow = { listing_id: number; acquisition_id: number; purchase_id: number; coin: string; asset_ref: string; amount: string; btc_txid: string; sats: string; ts: number | null; tx: string };
type LotRow = { listing_id: number; acquisition_id: number; mode: string; state: string; root: string | null; total: string | null; uri: string | null; challenge_ends: number | null;
  opened_ts: number | null; round: number | null; pinned_ts: number | null; winning_ticket: string | null; resolved_ts: number | null; winner: string | null; destination: string | null;
  destination_ts: number | null; delivered_txid: string | null; delivered_ts: number | null; overdue: number; voided_ticket: string | null; voided_ts: number | null };
type DistRow = { state: string; root: string | null; total: string | null; uri: string | null; challenge_ends: number | null; opened_ts: number | null; claimed: string; claims: number;
  rolled_coin: string | null; rolled_amount: string | null; rolled_ts: number | null };
