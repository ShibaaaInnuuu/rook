import type { Fetch } from "./chain.js";
import { openseaGet, type OpenSeaReadOpts } from "./opensea.js";
import type { Contents, Listing, ListingEvent, Sale } from "./floor.js";

/**
 * docs/v5/floor-feed.md §2. Where the floor's facts come from. Every source here READS: GET only (the OpenSea reads included),
 * spaced out, and an answer it does not understand is an error, never a number. Nothing here buys, signs or sends.
 */

export type Book = { listings: Listing[]; sales: Sale[]; snapshotAt: number; ref: string };

const spacer = (everyMs: number, now: () => number, sleep: (ms: number) => Promise<void>) => {
  let last = -Infinity;
  return async () => {
    const wait = last + everyMs - now();
    if (wait > 0) await sleep(wait);
    last = now();
  };
};
const defaultSleep = (ms: number) => new Promise<void>((ok) => setTimeout(ok, ms));
const isTxid = (s: unknown): s is string => typeof s === "string" && /^[0-9a-f]{64}$/.test(s);
const isInscription = (s: unknown): s is string => typeof s === "string" && /^[0-9a-f]{64}i\d+$/.test(s);
const isOutpoint = (s: unknown): s is string => typeof s === "string" && /^[0-9a-f]{64}:\d+$/.test(s);
const btcAddr = (s: unknown): s is string => typeof s === "string" && /^(bc1|tb1|bcrt1|[13mn2])[0-9A-Za-z]{20,90}$/.test(s);
const whole = (v: unknown, what: string): bigint => {
  if ((typeof v !== "number" && typeof v !== "string") || !/^\d+$/.test(String(v))) throw new Error(`invalid ${what} in the source's answer`);
  return BigInt(String(v));
};

// ---- ord: what a listed outpoint carries --------------------------------------------------------------------------------------------

export interface OrdReader { readonly name: string; contents(outpoint: string): Promise<Contents | null>; }

export class FixtureOrd implements OrdReader {
  readonly name = "fixture-ord";
  constructor(private readonly by: Record<string, Contents>) {}
  async contents(o: string): Promise<Contents | null> { return this.by[o] ?? null; }
}

/**
 * An ord server's `/output/<outpoint>` (JSON API). ordinals.com answers it 406 "JSON API disabled" (2026-09-23), so this is for an
 * ord of one's own only; the floor on mainnet uses `VerifiedOrd` (src/ordapi.ts): two hosted providers that must agree.
 */
export class OrdHttp implements OrdReader {
  readonly name = "ord";
  private readonly gate: () => Promise<void>;
  constructor(private readonly base: string, private readonly fetchImpl: Fetch = fetch, everyMs = 250, now: () => number = Date.now,
    sleep: (ms: number) => Promise<void> = defaultSleep) { this.gate = spacer(everyMs, now, sleep); }
  async contents(o: string): Promise<Contents | null> {
    if (!isOutpoint(o)) throw new Error("invalid outpoint");
    await this.gate();
    const r = await this.fetchImpl(`${this.base.replace(/\/$/, "")}/output/${o}`, { headers: { accept: "application/json" } });
    if (!r.ok) return null;
    let j: { inscriptions?: unknown; runes?: unknown; spent?: unknown };
    try { j = (await r.json()) as typeof j; } catch { return null; }
    if (!Array.isArray(j.inscriptions) || !j.inscriptions.every(isInscription)) return null;
    const runes = Array.isArray(j.runes) ? j.runes.map((x) => String(Array.isArray(x) ? x[0] : x)) : j.runes && typeof j.runes === "object" ? Object.keys(j.runes) : [];
    return { inscriptions: j.inscriptions as string[], runes };
  }
}

// ---- Ordinals Wallet: a keyless Bitcoin book --------------------------------------------------------------------------------------

type OwListing = { inscription_id?: unknown; outpoint?: unknown; seller_address?: unknown; satoshi_price?: unknown; created?: unknown; bought_at?: unknown; purchase_txid?: unknown };
type OwSale = { txid?: unknown; block_timestamp?: unknown; inscription_id?: unknown; seller_address?: unknown; buyer_address?: unknown; price_sats?: unknown; inscriptions_in_tx?: unknown };

/** Parse Ordinals Wallet's escrows (listings) answer. The outpoint is carried as the listing's `outpoint` for the ord check. */
export function parseOwListings(body: unknown, fetchedAt: number): (Listing & { outpoint: string })[] {
  const rows = Array.isArray(body) ? body : (body as { listings?: unknown })?.listings;
  if (!Array.isArray(rows)) throw new Error("invalid answer: Ordinals Wallet's escrows are not a list");
  const out: (Listing & { outpoint: string })[] = [];
  for (const x of rows as OwListing[]) {
    if (x.bought_at || x.purchase_txid) continue;   // filled: not a listing any more
    if (!isInscription(x.inscription_id) || !isOutpoint(x.outpoint) || !btcAddr(x.seller_address) || typeof x.created !== "string")
      throw new Error("invalid listing in Ordinals Wallet's answer");
    const listedAt = Math.floor(Date.parse(x.created.replace(" ", "T").replace(/\+00$/, "Z")) / 1000);
    if (!Number.isFinite(listedAt)) throw new Error("invalid listing time in Ordinals Wallet's answer");
    out.push({ id: x.inscription_id, chain: "btc", seller: x.seller_address, price: whole(x.satoshi_price, "satoshi_price"), listedAt,
      verifiedAt: fetchedAt, fillable: true, contents: null, outpoint: x.outpoint });
  }
  return out;
}

export function parseOwSales(body: unknown): Sale[] {
  const rows = (body as { sales?: unknown })?.sales;
  if (!Array.isArray(rows)) throw new Error("invalid answer: Ordinals Wallet's sales are not a list");
  return (rows as OwSale[]).map((x) => {
    if (!isTxid(x.txid) || !isInscription(x.inscription_id) || !btcAddr(x.seller_address) || !btcAddr(x.buyer_address) || typeof x.block_timestamp !== "number")
      throw new Error("invalid sale in Ordinals Wallet's answer");
    return { tx: x.txid, at: x.block_timestamp, item: x.inscription_id, seller: x.seller_address, buyer: x.buyer_address,
      price: whole(x.price_sats, "price_sats"), itemsInTx: Number(whole(x.inscriptions_in_tx ?? 1, "inscriptions_in_tx")) };
  });
}

/** Attach ord's contents to each listing (fail closed: an unanswered outpoint stays `contents: null`, which is L-unverified). */
export async function withContents(ls: (Listing & { outpoint: string })[], ord: OrdReader | null, only = 40): Promise<Listing[]> {
  const cheapest = [...ls].sort((a, b) => (a.price < b.price ? -1 : a.price > b.price ? 1 : 0)).slice(0, only);
  const out: Listing[] = [];
  for (const l of cheapest) out.push({ ...l, contents: ord ? await ord.contents(l.outpoint) : null });
  return out;
}

export class OrdinalsWalletBook {
  readonly name = "ordinalswallet";
  private readonly gate: () => Promise<void>;
  constructor(private readonly fetchImpl: Fetch = fetch, private readonly now: () => number = Date.now, everyMs = 3_000,
    sleep: (ms: number) => Promise<void> = defaultSleep) { this.gate = spacer(everyMs, now, sleep); }
  private async get(path: string): Promise<unknown> {
    await this.gate();
    const r = await this.fetchImpl(`https://turbo.ordinalswallet.com/${path}`, { headers: { accept: "application/json", "user-agent": "etch-keeper-floor/1 (read-only)" } });
    if (r.status === 429) throw new Error("ordinalswallet: rate limited; not retried this pass");
    if (!r.ok) throw new Error(`ordinalswallet: HTTP ${r.status}`);
    return r.json();
  }
  async book(slug: string, ord: OrdReader | null): Promise<Book> {
    if (!/^[a-z0-9-]{1,64}$/.test(slug)) throw new Error("invalid collection slug");
    const at = Math.floor(this.now() / 1000);
    const listings = await withContents(parseOwListings(await this.get(`collection/${slug}/escrows`), at), ord);
    const sales = parseOwSales(await this.get(`collection/${slug}/sales`));
    return { listings, sales, snapshotAt: at, ref: `ordinalswallet:${slug}` };
  }
}

// ---- OpenSea: an EVM book, with a key read at call time ------------------------------------------------------------------------------

export type { OpenSeaReadOpts } from "./opensea.js";

const evmAddr = (a: unknown): string | null => {
  const s = typeof a === "object" && a !== null && "address" in a ? (a as { address: unknown }).address : a;
  return typeof s === "string" && /^0x[0-9a-fA-F]{40}$/.test(s) ? s.toLowerCase() : null;
};

/**
 * OpenSea's `listings/collection/<slug>/best` answer, per its v2 docs (UNVERIFIED against the live service: anything unexpected
 * throws). A listing is plain when its Seaport order offers exactly one ERC-721 (item type 2) and is priced in native ETH.
 */
export function parseOpenSeaListings(body: unknown, collection: string, fetchedAt: number): Listing[] {
  const rows = (body as { listings?: unknown })?.listings;
  if (!Array.isArray(rows)) throw new Error("invalid answer: OpenSea's listings are not a list");
  const want = collection.toLowerCase();
  return rows.map((x: any) => {
    const params = x?.protocol_data?.parameters;
    const price = x?.price?.current;
    if (!params || !Array.isArray(params.offer) || !price) throw new Error("invalid listing in OpenSea's answer");
    const offerer = evmAddr(params.offerer);
    if (!offerer) throw new Error("invalid offerer in OpenSea's answer");
    const first = params.offer[0];
    const token = evmAddr(first?.token);
    const nft = params.offer.length === 1 && Number(first?.itemType) === 2 && token === want;
    const native = String(price.currency ?? "").toUpperCase() === "ETH" && Number(price.decimals) === 18;
    return { id: `${want}:${String(first?.identifierOrCriteria ?? "?")}`, chain: "evm" as const, seller: offerer, price: whole(price.value, "price"),
      listedAt: Number(whole(params.startTime, "startTime")), verifiedAt: fetchedAt, fillable: true, expiresAt: Number(whole(params.endTime, "endTime")),
      offerItems: nft ? 1 : Math.max(2, params.offer.length), nativeCurrency: native, special: x?.type !== undefined && x.type !== "basic" };
  });
}

/** OpenSea's `events/collection/<slug>?event_type=sale` answer (UNVERIFIED shape, strict). */
export function parseOpenSeaSales(body: unknown, collection: string): Sale[] {
  const rows = (body as { asset_events?: unknown })?.asset_events;
  if (!Array.isArray(rows)) throw new Error("invalid answer: OpenSea's events are not a list");
  const want = collection.toLowerCase();
  const perTx = new Map<string, number>();
  for (const e of rows as any[]) if (e?.event_type === "sale") perTx.set(String(e.transaction), (perTx.get(String(e.transaction)) ?? 0) + 1);
  return (rows as any[]).filter((e) => e?.event_type === "sale").map((e) => {
    const seller = evmAddr(e.seller), buyer = evmAddr(e.buyer);
    if (!seller || !buyer || typeof e.transaction !== "string" || typeof e.closing_date !== "number") throw new Error("invalid sale in OpenSea's answer");
    const pay = e.payment;
    if (!pay || evmAddr(pay.token_address) !== "0x0000000000000000000000000000000000000000" || Number(pay.decimals) !== 18)
      return null;   // not an ETH sale: not a price in this unit
    const qty = Number(e.quantity ?? 1);
    return { tx: e.transaction, at: e.closing_date, item: `${want}:${String(e.nft?.identifier ?? "?")}`, seller, buyer, price: whole(pay.quantity, "payment"),
      itemsInTx: Math.max(qty, perTx.get(e.transaction) ?? 1) };
  }).filter((s): s is Sale => s !== null);
}

export class OpenSeaBook {
  readonly name = "opensea";
  constructor(private readonly o: OpenSeaReadOpts = {}, private readonly now: () => number = Date.now) {}
  async book(slug: string, collection: string): Promise<Book> {
    if (!/^[a-z0-9-]{1,64}$/.test(slug)) throw new Error("invalid collection slug");
    const at = Math.floor(this.now() / 1000);
    const listings = parseOpenSeaListings(await openseaGet(`listings/collection/${slug}/best?limit=50`, this.o), collection, at);
    const sales = parseOpenSeaSales(await openseaGet(`events/collection/${slug}?event_type=sale&limit=50`, this.o), collection);
    this.o.log?.("info", "opensea_book", { slug, listings: listings.length, sales: sales.length });
    return { listings, sales, snapshotAt: at, ref: `opensea:${slug}` };
  }
}

// ---- listing events from successive books ------------------------------------------------------------------------------------------

/**
 * docs/v5/floor-feed.md §1.1: when a source has no activity feed, the keeper diffs its own successive books. A listing that
 * appears is a `list`; one that disappears with no sale of its item is a `delist`. Kept for `keepS`.
 */
export class BookDiff {
  private prev = new Map<string, Map<string, string>>();
  private events = new Map<string, ListingEvent[]>();
  constructor(private readonly keepS = 8 * 3_600) {}
  observe(collection: string, b: Book): ListingEvent[] {
    const now = b.snapshotAt;
    const cur = new Map(b.listings.map((l) => [`${l.id}@${l.price}`, l.seller.toLowerCase()]));
    const before = this.prev.get(collection);
    const ev = (this.events.get(collection) ?? []).filter((e) => now - e.at <= this.keepS);
    if (before) {
      const sold = new Set(b.sales.map((s) => s.item));
      for (const [k, w] of cur) if (!before.has(k)) ev.push({ wallet: w, at: now, kind: "list" });
      for (const [k, w] of before) if (!cur.has(k) && !sold.has(k.split("@")[0])) ev.push({ wallet: w, at: now, kind: "delist" });
    }
    this.prev.set(collection, cur);
    this.events.set(collection, ev);
    return ev;
  }
}

// ---- the rate ---------------------------------------------------------------------------------------------------------------------

export interface RateSource { readonly name: string; satsPerEth(): Promise<bigint>; }
export class FixtureRate implements RateSource {
  readonly name = "fixture-rate";
  constructor(private readonly v: bigint) {}
  async satsPerEth(): Promise<bigint> { return this.v; }
}
/** Coinbase's public exchange-rates endpoint, keyless: BTC per ETH, as sats per ETH. One reference, stated as such (§8.6). */
export class CoinbaseRate implements RateSource {
  readonly name = "coinbase";
  constructor(private readonly fetchImpl: Fetch = fetch) {}
  async satsPerEth(): Promise<bigint> {
    const r = await this.fetchImpl("https://api.coinbase.com/v2/exchange-rates?currency=ETH", { headers: { accept: "application/json" } });
    if (!r.ok) throw new Error(`coinbase: HTTP ${r.status}`);
    const v = String(((await r.json()) as { data?: { rates?: { BTC?: unknown } } })?.data?.rates?.BTC ?? "");
    const m = /^(\d+)\.(\d{1,18})$/.exec(v) ?? /^(\d+)$/.exec(v);
    if (!m) throw new Error("invalid rate in Coinbase's answer");
    const [intPart, frac = ""] = v.split(".");
    const sats = (BigInt(intPart) * 10n ** 18n + BigInt((frac + "0".repeat(18)).slice(0, 18))) / 10n ** 10n;
    if (sats <= 0n) throw new Error("invalid rate in Coinbase's answer");
    return sats;
  }
}
