import { delayFor } from "./backoff.js";
import type { Fetch } from "./chain.js";
import type { Contents } from "./floor.js";
import type { OrdReader } from "./floorsources.js";
import type { Logger } from "./log.js";

/**
 * docs/v6/ordinals-apis.md. The ordinals facts the keeper needs (where an inscription is, what an output carries, what an
 * address holds, how deep a transaction is, what a collection has listed), read from HOSTED providers — Rook runs no ord of
 * its own — and believed only when two of them agree at the same height. One provider's word is "unverified", and unverified
 * is the existing fail-safe everywhere: the floor's L-unverified rule, a delivery that waits, a hand-in judged later.
 *
 * Providers (§2 of the doc): UniSat's Open API (keyed: UNISAT_API_KEY, read here and only here, at call time, into one
 * header of one host; log.ts cuts it out of every line), ord's own recursive endpoints (`/r/…`: ordinals.com today, a
 * self-hosted ord later, keyless) and Ordinals Wallet (keyless; locations, addresses and a book, no per-output view). Hiro's
 * Ordinals API, the plan's second provider, answered 410 Gone on 2026-09-23 (retired 2026-03-09) and is not here.
 *
 * Every read is a GET with a User-Agent; a 429 parks the provider for Retry-After (else a growing wait), and a parked
 * provider is "rate-limited", not "down". Nothing here signs, sends or buys.
 */

export const USER_AGENT = "etch-keeper-ord/1 (read-only)";

export type Inscription = { id: string; outpoint: string; offset: number; value: number | null; address: string | null; contentType: string | null; genesisHeight: number | null };
/** What an output carries. `null` from a provider: not found or spent. Rune names as ord prints them, spacers removed for comparison. */
export type OutputContents = { inscriptions: string[]; runes: string[]; value: number | null };
export type OrdListing = { id: string; price: bigint; seller: string; outpoint: string | null; listedAt: number | null };
export type Read = "height" | "inscription" | "output" | "address" | "tx" | "listings";

export interface OrdProvider {
  readonly name: string;
  readonly reads: ReadonlySet<Read>;
  height(): Promise<number>;
  /** `null`: the provider does not know the inscription. */
  inscription(id: string): Promise<Inscription | null>;
  /** `null`: not found (never seen, or spent). */
  output(outpoint: string): Promise<OutputContents | null>;
  addressInscriptions(address: string): Promise<string[]>;
  /** Confirmations of a transaction; 0 in the mempool; `null`: unknown to the provider. */
  txConfirmations(txid: string): Promise<number | null>;
  listings(collection: string): Promise<OrdListing[]>;
}

export const isInscriptionId = (s: unknown): s is string => typeof s === "string" && /^[0-9a-f]{64}i\d{1,10}$/.test(s);
export const isOutpoint = (s: unknown): s is string => typeof s === "string" && /^[0-9a-f]{64}:\d{1,10}$/.test(s);
const isTxid = (s: unknown): s is string => typeof s === "string" && /^[0-9a-f]{64}$/.test(s);
const isBtcAddress = (s: unknown): s is string => typeof s === "string" && /^(bc1|tb1|bcrt1|[13mn2])[0-9A-Za-z]{20,90}$/.test(s);
const isSlug = (s: unknown): s is string => typeof s === "string" && /^[a-z0-9-]{1,64}$/.test(s);
const whole = (v: unknown): number | null => (typeof v === "number" && Number.isInteger(v) && v >= 0) || (typeof v === "string" && /^\d+$/.test(v)) ? Number(v) : null;
/** A rune's name with ord's spacers (`•`) and any other spacer removed: providers print them differently, the letters are the name. */
export const runeName = (s: string): string => s.replace(/[•.\s]/g, "").toUpperCase();
const sortedUnique = (xs: string[]): string[] => [...new Set(xs)].sort();

export class Unsupported extends Error { constructor(provider: string, read: Read) { super(`${provider} does not answer ${read}`); this.name = "Unsupported"; } }
export class RateLimited extends Error { constructor(provider: string, readonly untilMs: number) { super(`${provider}: rate limited`); this.name = "RateLimited"; } }

export type HttpOpts = { fetchImpl?: Fetch; now?: () => number; sleep?: (ms: number) => Promise<void>; everyMs?: number; log?: Logger };

/**
 * One host's GET gate: requests spaced `everyMs` apart, and a 429 parks the host for Retry-After (capped at a minute) or a
 * wait that doubles with each consecutive 429 (backoff.ts's schedule). While parked, a read is refused at once as RateLimited.
 */
export class Http {
  private last = -Infinity;
  private parkedUntil = 0;
  private strikes = 0;
  readonly fetchImpl: Fetch;
  readonly now: () => number;
  private readonly sleep: (ms: number) => Promise<void>;
  private readonly everyMs: number;
  constructor(readonly host: string, o: HttpOpts = {}) {
    this.fetchImpl = o.fetchImpl ?? fetch; this.now = o.now ?? Date.now; this.everyMs = o.everyMs ?? 250;
    this.sleep = o.sleep ?? ((ms) => new Promise<void>((ok) => setTimeout(ok, ms)));
  }
  async get(url: string, headers: Record<string, string> = {}): Promise<{ status: number; text: string }> {
    if (!url.startsWith(`https://${this.host}/`)) throw new Error(`refused: ${this.host}'s gate does not fetch ${url.split("/")[2] ?? "?"}`);
    if (this.now() < this.parkedUntil) throw new RateLimited(this.host, this.parkedUntil);
    const wait = this.last + this.everyMs - this.now();
    if (wait > 0) await this.sleep(wait);
    this.last = this.now();
    const r = await this.fetchImpl(url, { method: "GET", headers: { accept: "application/json", "user-agent": USER_AGENT, ...headers } });
    if (r.status === 429) {
      const after = Number(r.headers?.get?.("retry-after"));
      const ms = Number.isFinite(after) && after > 0 ? Math.min(after, 60) * 1000 : delayFor(this.strikes, { baseMs: 5_000, factor: 2, maxMs: 60_000 });
      this.strikes++; this.parkedUntil = this.now() + ms;
      throw new RateLimited(this.host, this.parkedUntil);
    }
    this.strikes = 0;
    return { status: r.status, text: await r.text() };
  }
}

const json = (text: string): unknown => { try { return JSON.parse(text); } catch { throw new Error("not JSON"); } };
const reads = (...xs: Read[]): ReadonlySet<Read> => new Set(xs);

// ---- ord's recursive endpoints: ordinals.com today, a self-hosted ord tomorrow, the same code ---------------------------------------

/** `GET /r/inscription/<id>`, `/r/utxo/<outpoint>`, `/r/blockheight` (ord ≥ 0.18). The JSON API (`/inscription`, `/output`) is not used: ordinals.com answers it 406. */
export class OrdRecursive implements OrdProvider {
  readonly name: string;
  readonly reads = reads("height", "inscription", "output");
  private readonly http: Http;
  constructor(readonly base = "https://ordinals.com", o: HttpOpts = {}, name = "ord") {
    this.base = base.replace(/\/$/, ""); this.name = name; this.http = new Http(new URL(this.base).host, o);
  }
  async height(): Promise<number> {
    const r = await this.http.get(`${this.base}/r/blockheight`);
    const h = whole(r.text.trim());
    if (r.status !== 200 || h === null) throw new Error(`${this.name}: HTTP ${r.status} for the height`);
    return h;
  }
  async inscription(id: string): Promise<Inscription | null> {
    if (!isInscriptionId(id)) throw new Error("invalid inscription id");
    const r = await this.http.get(`${this.base}/r/inscription/${id}`);
    if (r.status === 404) return null;
    if (r.status !== 200) throw new Error(`${this.name}: HTTP ${r.status}`);
    const j = json(r.text) as { id?: unknown; satpoint?: unknown; output?: unknown; value?: unknown; address?: unknown; content_type?: unknown; height?: unknown };
    if (j.id !== id) throw new Error(`${this.name}: the answer is about a different inscription`);
    if (typeof j.satpoint !== "string") throw new Error(`${this.name}: no satpoint`);
    const i = j.satpoint.lastIndexOf(":"), outpoint = j.satpoint.slice(0, i), offset = whole(j.satpoint.slice(i + 1));
    if (!isOutpoint(outpoint) || offset === null || (j.output !== undefined && j.output !== outpoint)) throw new Error(`${this.name}: an unreadable satpoint`);
    return { id, outpoint, offset, value: whole(j.value), address: isBtcAddress(j.address) ? j.address : null, contentType: typeof j.content_type === "string" ? j.content_type : null,
      genesisHeight: whole(j.height) };
  }
  async output(outpoint: string): Promise<OutputContents | null> {
    if (!isOutpoint(outpoint)) throw new Error("invalid outpoint");
    const r = await this.http.get(`${this.base}/r/utxo/${outpoint}`);
    if (r.status === 404) return null;
    if (r.status !== 200) throw new Error(`${this.name}: HTTP ${r.status}`);
    const j = json(r.text) as { inscriptions?: unknown; runes?: unknown; value?: unknown };
    if (!Array.isArray(j.inscriptions) || !j.inscriptions.every(isInscriptionId)) throw new Error(`${this.name}: no inscriptions field`);
    if (!j.runes || typeof j.runes !== "object" || Array.isArray(j.runes)) throw new Error(`${this.name}: no runes field`);
    return { inscriptions: sortedUnique(j.inscriptions), runes: sortedUnique(Object.keys(j.runes).map(runeName)), value: whole(j.value) };
  }
  async addressInscriptions(): Promise<string[]> { throw new Unsupported(this.name, "address"); }
  async txConfirmations(): Promise<number | null> { throw new Unsupported(this.name, "tx"); }
  async listings(): Promise<OrdListing[]> { throw new Unsupported(this.name, "listings"); }
}

// ---- UniSat's Open API: the keyed provider --------------------------------------------------------------------------------------

export const UNISAT_BASE = "https://open-api.unisat.io";
type Envelope = { code?: unknown; msg?: unknown; data?: unknown };

/**
 * UniSat's `/v1/indexer/…` reads (docs/v6/ordinals-apis.md §2.1; shapes from the service's swagger.yaml, UNVERIFIED against a
 * keyed call: anything unexpected throws). The key goes into `Authorization: Bearer` of open-api.unisat.io and nowhere else.
 */
export class UniSat implements OrdProvider {
  readonly name = "unisat";
  readonly reads = reads("height", "inscription", "output", "address", "tx", "listings");
  private readonly http: Http;
  private readonly apiKey: () => string | undefined;
  constructor(o: HttpOpts & { apiKey?: () => string | undefined } = {}) {
    this.http = new Http("open-api.unisat.io", { everyMs: 250, ...o });   // the free tier: 5 calls a second
    this.apiKey = o.apiKey ?? (() => process.env.UNISAT_API_KEY);
  }
  private async get(path: string): Promise<unknown> {
    const key = this.apiKey();
    if (!key) throw new Error("missing UNISAT_API_KEY: the keeper reads it from its environment at call time");
    const r = await this.http.get(`${UNISAT_BASE}${path}`, { authorization: `Bearer ${key}` });
    if (r.status === 401 || r.status === 403) throw new Error(`refused: UniSat rejected the key (HTTP ${r.status})`);
    if (r.status !== 200) throw new Error(`unisat: HTTP ${r.status}`);
    const j = json(r.text) as Envelope;
    if (j.code !== 0) throw new Error(`unisat: code ${String(j.code)}`);
    return j.data;
  }
  async height(): Promise<number> {
    const d = (await this.get("/v1/indexer/blockchain/info")) as { blocks?: unknown };
    const h = whole(d?.blocks);
    if (h === null) throw new Error("unisat: no height");
    return h;
  }
  async inscription(id: string): Promise<Inscription | null> {
    if (!isInscriptionId(id)) throw new Error("invalid inscription id");
    const d = (await this.get(`/v1/indexer/inscription/info/${id}`)) as { inscriptionId?: unknown; utxo?: { txid?: unknown; vout?: unknown; satoshi?: unknown; address?: unknown } | null;
      offset?: unknown; address?: unknown; contentType?: unknown; height?: unknown } | null;
    if (d === null || d === undefined) return null;
    if (d.inscriptionId !== id) throw new Error("unisat: the answer is about a different inscription");
    const u = d.utxo;
    if (!u || !isTxid(u.txid) || whole(u.vout) === null) throw new Error("unisat: no utxo");
    const offset = whole(d.offset);
    if (offset === null) throw new Error("unisat: no offset");
    return { id, outpoint: `${u.txid}:${whole(u.vout)}`, offset, value: whole(u.satoshi), address: isBtcAddress(d.address) ? d.address : isBtcAddress(u.address) ? u.address : null,
      contentType: typeof d.contentType === "string" ? d.contentType : null, genesisHeight: whole(d.height) };
  }
  async output(outpoint: string): Promise<OutputContents | null> {
    if (!isOutpoint(outpoint)) throw new Error("invalid outpoint");
    const [txid, vout] = outpoint.split(":");
    const d = (await this.get(`/v1/indexer/utxo/${txid}/${vout}`)) as { inscriptions?: unknown; satoshi?: unknown; isSpent?: unknown } | null;
    if (d === null || d === undefined || d.isSpent === true) return null;   // ord answers a spent output 404: the same fact, the same shape
    if (!Array.isArray(d.inscriptions)) throw new Error("unisat: no inscriptions field");
    const ids = d.inscriptions.map((x: { inscriptionId?: unknown }) => x?.inscriptionId);
    if (!ids.every(isInscriptionId)) throw new Error("unisat: an inscription id that is not one");
    const runes = (await this.get(`/v1/indexer/runes/utxo/${txid}/${vout}/balance`)) as { spacedRune?: unknown; rune?: unknown }[] | null;
    if (runes !== null && !Array.isArray(runes)) throw new Error("unisat: no runes list");
    const names = (runes ?? []).map((r) => (typeof r.spacedRune === "string" ? r.spacedRune : typeof r.rune === "string" ? r.rune : null));
    if (names.some((n) => n === null)) throw new Error("unisat: a rune with no name");
    return { inscriptions: sortedUnique(ids as string[]), runes: sortedUnique((names as string[]).map(runeName)), value: whole(d.satoshi) };
  }
  async addressInscriptions(address: string): Promise<string[]> {
    if (!isBtcAddress(address)) throw new Error("invalid address");
    const out: string[] = [];
    for (let cursor = 0; ; cursor += 100) {
      const d = (await this.get(`/v1/indexer/address/${address}/inscription-data?cursor=${cursor}&size=100`)) as { total?: unknown; inscription?: unknown[] } | null;
      const rows = d?.inscription;
      if (!Array.isArray(rows)) throw new Error("unisat: no inscription list");
      for (const r of rows as { inscriptionId?: unknown }[]) { if (!isInscriptionId(r?.inscriptionId)) throw new Error("unisat: an inscription id that is not one"); out.push(r.inscriptionId); }
      const total = whole(d?.total) ?? 0;
      if (rows.length === 0 || out.length >= total || out.length >= 5_000) break;
    }
    return sortedUnique(out);
  }
  async txConfirmations(txid: string): Promise<number | null> {
    if (!isTxid(txid)) throw new Error("invalid txid");
    const d = (await this.get(`/v1/indexer/tx/${txid}`)) as { txid?: unknown; confirmations?: unknown } | null;
    if (d === null || d === undefined) return null;
    if (d.txid !== txid) throw new Error("unisat: the answer is about a different transaction");
    return whole(d.confirmations) ?? 0;
  }
  /** The UniSat marketplace's listings of a collection (`/v3/market/collection/auction/list` is a query-style POST; not GET, so not here yet). */
  async listings(): Promise<OrdListing[]> { throw new Unsupported(this.name, "listings"); }
}

// ---- Ordinals Wallet: keyless; where an inscription is, what a wallet holds, a collection's book --------------------------------------

export class OrdinalsWallet implements OrdProvider {
  readonly name = "ordinalswallet";
  readonly reads = reads("height", "inscription", "address", "listings");
  private readonly http: Http;
  constructor(o: HttpOpts = {}, readonly base = "https://turbo.ordinalswallet.com") { this.http = new Http(new URL(base).host, { everyMs: 500, ...o }); }
  async height(): Promise<number> {
    const r = await this.http.get(`${this.base}/blockheight`);
    const h = whole(r.text.trim());
    if (r.status !== 200 || h === null) throw new Error(`ordinalswallet: HTTP ${r.status} for the height`);
    return h;
  }
  async inscription(id: string): Promise<Inscription | null> {
    if (!isInscriptionId(id)) throw new Error("invalid inscription id");
    const r = await this.http.get(`${this.base}/inscription/${id}`);
    if (r.status === 404) return null;
    if (r.status !== 200) throw new Error(`ordinalswallet: HTTP ${r.status}`);
    const j = json(r.text) as { id?: unknown; satpoint?: unknown; value?: unknown; address?: unknown; content_type?: unknown; genesis_height?: unknown };
    if (j.id !== id) throw new Error("ordinalswallet: the answer is about a different inscription");
    if (typeof j.satpoint !== "string") throw new Error("ordinalswallet: no satpoint");
    const i = j.satpoint.lastIndexOf(":"), outpoint = j.satpoint.slice(0, i), offset = whole(j.satpoint.slice(i + 1));
    if (!isOutpoint(outpoint) || offset === null) throw new Error("ordinalswallet: an unreadable satpoint");
    return { id, outpoint, offset, value: whole(j.value), address: isBtcAddress(j.address) ? j.address : null, contentType: typeof j.content_type === "string" ? j.content_type : null,
      genesisHeight: whole(j.genesis_height) };
  }
  async output(): Promise<OutputContents | null> { throw new Unsupported(this.name, "output"); }
  async addressInscriptions(address: string): Promise<string[]> {
    if (!isBtcAddress(address)) throw new Error("invalid address");
    const r = await this.http.get(`${this.base}/wallet/${address}`);
    if (r.status !== 200) throw new Error(`ordinalswallet: HTTP ${r.status}`);
    const rows = (json(r.text) as { inscriptions?: unknown })?.inscriptions;
    if (!Array.isArray(rows)) throw new Error("ordinalswallet: no inscriptions list");
    const ids = rows.map((x: { id?: unknown }) => x?.id);
    if (!ids.every(isInscriptionId)) throw new Error("ordinalswallet: an inscription id that is not one");
    return sortedUnique(ids as string[]);
  }
  async txConfirmations(): Promise<number | null> { throw new Unsupported(this.name, "tx"); }
  async listings(collection: string): Promise<OrdListing[]> {
    if (!isSlug(collection)) throw new Error("invalid collection slug");
    const r = await this.http.get(`${this.base}/collection/${collection}/escrows`);
    if (r.status !== 200) throw new Error(`ordinalswallet: HTTP ${r.status}`);
    const body = json(r.text);
    const rows = Array.isArray(body) ? body : (body as { listings?: unknown })?.listings;
    if (!Array.isArray(rows)) throw new Error("ordinalswallet: escrows are not a list");
    const out: OrdListing[] = [];
    for (const x of rows as { inscription_id?: unknown; outpoint?: unknown; seller_address?: unknown; satoshi_price?: unknown; created?: unknown; bought_at?: unknown; purchase_txid?: unknown }[]) {
      if (x.bought_at || x.purchase_txid) continue;
      if (!isInscriptionId(x.inscription_id) || !isBtcAddress(x.seller_address) || whole(x.satoshi_price) === null) throw new Error("ordinalswallet: an unreadable listing");
      const at = typeof x.created === "string" ? Math.floor(Date.parse(x.created.replace(" ", "T").replace(/\+00$/, "Z")) / 1000) : NaN;
      out.push({ id: x.inscription_id, price: BigInt(whole(x.satoshi_price)!), seller: x.seller_address, outpoint: isOutpoint(x.outpoint) ? x.outpoint : null, listedAt: Number.isFinite(at) ? at : null });
    }
    return out;
  }
}

// ---- esplora, for the transaction facts a second ordinals provider cannot give ----------------------------------------------------

export class Esplora {
  readonly name: string;
  private readonly http: Http;
  constructor(readonly base = "https://mempool.space/api", o: HttpOpts = {}) { this.base = base.replace(/\/$/, ""); this.name = new URL(this.base).host; this.http = new Http(this.name, o); }
  async height(): Promise<number> {
    const r = await this.http.get(`${this.base}/blocks/tip/height`);
    const h = whole(r.text.trim());
    if (r.status !== 200 || h === null) throw new Error(`${this.name}: HTTP ${r.status} for the height`);
    return h;
  }
  /** Confirmations at the explorer's tip; 0 in the mempool; null: unknown. */
  async txConfirmations(txid: string): Promise<number | null> {
    if (!isTxid(txid)) throw new Error("invalid txid");
    const r = await this.http.get(`${this.base}/tx/${txid}/status`);
    if (r.status === 404) return null;
    if (r.status !== 200) throw new Error(`${this.name}: HTTP ${r.status}`);
    const j = json(r.text) as { confirmed?: unknown; block_height?: unknown };
    if (j.confirmed !== true) return 0;
    const h = whole(j.block_height);
    if (h === null) throw new Error(`${this.name}: confirmed without a height`);
    return (await this.height()) - h + 1;
  }
}

// ---- the feed: two providers, one height, or nothing -----------------------------------------------------------------------------

export type Why = "unsupported" | "one-down" | "rate-limited" | "stale" | "disagree";
export type Verified<T> = { ok: true; value: T; height: number; by: string[] } | { ok: false; why: Why; detail: string };
export const FEED = { maxHeightGap: 1, minProviders: 2, cacheMs: 45_000, muteS: 3_600 } as const;

const canon = (v: unknown): string => JSON.stringify(v, (_k, x) => (typeof x === "bigint" ? String(x) : x));
/** `<provider>: <what went wrong>`, the provider named once. */
const blame = (name: string, err: unknown): string => { const m = String((err as Error)?.message ?? err).slice(0, 200); return m.startsWith(`${name}:`) ? m : `${name}: ${m}`; };

/**
 * Agreement of inscriptions: two providers may know different things (one prints no address, another no value), but where
 * both speak they must say the same. The merged answer carries every fact anyone gave. `null` (unknown) must be unanimous.
 */
export function mergeInscriptions(xs: (Inscription | null)[]): Inscription | null | undefined {
  if (xs.every((x) => x === null)) return null;
  if (xs.some((x) => x === null)) return undefined;
  const ys = xs as Inscription[], m = { ...ys[0] };
  for (const y of ys.slice(1)) {
    if (y.id !== m.id || y.outpoint !== m.outpoint || y.offset !== m.offset) return undefined;
    for (const k of ["value", "address", "contentType", "genesisHeight"] as const) {
      if (m[k] !== null && y[k] !== null && m[k] !== y[k]) return undefined;
      if (m[k] === null) (m as Record<string, unknown>)[k] = y[k];
    }
  }
  return m;
}

export function mergeOutputs(xs: (OutputContents | null)[]): OutputContents | null | undefined {
  if (xs.every((x) => x === null)) return null;
  if (xs.some((x) => x === null)) return undefined;
  const ys = xs as OutputContents[], m = { ...ys[0] };
  for (const y of ys.slice(1)) {
    if (canon(y.inscriptions) !== canon(m.inscriptions) || canon(y.runes) !== canon(m.runes)) return undefined;
    if (m.value !== null && y.value !== null && m.value !== y.value) return undefined;
    if (m.value === null) m.value = y.value;
  }
  return m;
}

export class OrdFeed {
  private readonly log?: Logger;
  private readonly now: () => number;
  private readonly muted = new Map<string, number>();
  private readonly cache = new Map<string, { at: number; value: Verified<unknown> }>();
  constructor(readonly providers: OrdProvider[], o: { log?: Logger; now?: () => number; esplora?: Esplora[] } = {}) {
    this.log = o.log; this.now = o.now ?? Date.now; this.esplora = o.esplora ?? [];
  }
  private readonly esplora: Esplora[];

  /** A failure is logged at most once an hour per kind: the fail-safe is silent by design, not by accident. */
  private fail<T>(read: Read, why: Why, detail: string): Verified<T> {
    const key = `${read}:${why}`, t = this.now();
    if ((this.muted.get(key) ?? -Infinity) + FEED.muteS * 1000 <= t) { this.muted.set(key, t); this.log?.("warn", "ord_unverified", { read, why, detail: detail.slice(0, 300) }); }
    return { ok: false, why, detail };
  }

  /**
   * The read from every provider that answers it, all at heights within `maxHeightGap` of each other, merged by `merge` —
   * which returns `undefined` for a disagreement. Fewer than two providers, one down, one parked, one behind: unverified.
   */
  private async verified<T>(read: Read, ask: (p: OrdProvider) => Promise<T>, merge: (xs: T[]) => T | undefined): Promise<Verified<T>> {
    const ps = this.providers.filter((p) => p.reads.has(read));
    if (ps.length < FEED.minProviders) return this.fail(read, "unsupported", `${ps.length} of ${this.providers.length} providers answer ${read}; ${FEED.minProviders} must`);
    const answers: T[] = [], heights: number[] = [], by: string[] = [];
    for (const p of ps) {
      try {
        const h0 = await p.height(), v = await ask(p), h1 = await p.height();
        if (h1 !== h0) return this.fail(read, "stale", `${p.name} moved from ${h0} to ${h1} during the read`);
        answers.push(v); heights.push(h0); by.push(p.name);
      } catch (err) {
        if (err instanceof RateLimited) return this.fail(read, "rate-limited", `${p.name} is parked until ${new Date(err.untilMs).toISOString()}`);
        return this.fail(read, "one-down", blame(p.name, err));
      }
    }
    const lo = Math.min(...heights), hi = Math.max(...heights);
    if (hi - lo > FEED.maxHeightGap) return this.fail(read, "stale", `heights ${heights.join("/")} from ${by.join("/")} are ${hi - lo} blocks apart`);
    const value = merge(answers);
    if (value === undefined) return this.fail(read, "disagree", `${by.join(" and ")} disagree on ${read}: ${answers.map(canon).join(" vs ")}`.slice(0, 600));
    return { ok: true, value, height: lo, by };
  }

  inscription(id: string): Promise<Verified<Inscription | null>> { return this.verified("inscription", (p) => p.inscription(id), mergeInscriptions); }
  output(outpoint: string): Promise<Verified<OutputContents | null>> { return this.verified("output", (p) => p.output(outpoint), mergeOutputs); }
  addressInscriptions(address: string): Promise<Verified<string[]>> {
    return this.verified("address", (p) => p.addressInscriptions(address), (xs) => (xs.every((x) => canon(x) === canon(xs[0])) ? xs[0] : undefined));
  }
  /** Confirmations: every ordinals provider that answers plus every esplora; two answers at least, all equal (an explorer's tip may differ by one: that is "stale"). */
  async txConfirmations(txid: string): Promise<Verified<number | null>> {
    const sources: { name: string; ask: () => Promise<number | null> }[] = [
      ...this.providers.filter((p) => p.reads.has("tx")).map((p) => ({ name: p.name, ask: () => p.txConfirmations(txid) })),
      ...this.esplora.map((e) => ({ name: e.name, ask: () => e.txConfirmations(txid) })),
    ];
    if (sources.length < FEED.minProviders) return this.fail("tx", "unsupported", `${sources.length} sources answer tx; ${FEED.minProviders} must`);
    const got: (number | null)[] = [], by: string[] = [];
    for (const s of sources) {
      try { got.push(await s.ask()); by.push(s.name); } catch (err) {
        if (err instanceof RateLimited) return this.fail("tx", "rate-limited", `${s.name} is parked until ${new Date(err.untilMs).toISOString()}`);
        return this.fail("tx", "one-down", blame(s.name, err));
      }
    }
    const nums = got.filter((x): x is number => x !== null);
    if (nums.length !== got.length && nums.length !== 0) return this.fail("tx", "disagree", `${by.join("/")}: ${got.join("/")}`);
    if (nums.length && Math.max(...nums) - Math.min(...nums) > FEED.maxHeightGap) return this.fail("tx", "stale", `confirmations ${got.join("/")} from ${by.join("/")}`);
    return { ok: true, value: nums.length ? Math.min(...nums) : null, height: 0, by };
  }
  /** A collection's book: one provider's word (a listing is not a fact that moves money; the outpoint check above is), cached `cacheMs`. */
  async listings(collection: string): Promise<Verified<OrdListing[]>> {
    const key = `listings:${collection}`, hit = this.cache.get(key);
    if (hit && this.now() - hit.at < FEED.cacheMs) return hit.value as Verified<OrdListing[]>;
    const ps = this.providers.filter((p) => p.reads.has("listings"));
    let last: Verified<OrdListing[]> = { ok: false, why: "unsupported", detail: "no provider serves listings" };
    for (const p of ps) {
      try { last = { ok: true, value: await p.listings(collection), height: 0, by: [p.name] }; break; } catch (err) {
        last = this.fail("listings", err instanceof RateLimited ? "rate-limited" : "one-down", blame(p.name, err));
      }
    }
    this.cache.set(key, { at: this.now(), value: last });
    return last;
  }
}

/** The floor feed's ord (docs/v5/floor-feed.md §2) over the verified read: an unverified outpoint stays `null`, which is L-unverified. */
export class VerifiedOrd implements OrdReader {
  readonly name = "ord-verified";
  constructor(private readonly feed: OrdFeed) {}
  async contents(outpoint: string): Promise<Contents | null> {
    const v = await this.feed.output(outpoint);
    return v.ok && v.value ? { inscriptions: v.value.inscriptions, runes: v.value.runes } : null;
  }
}

/** The default feed for Bitcoin mainnet: UniSat (keyed), ord's recursive endpoints at ordinals.com, Ordinals Wallet; mempool.space and blockstream.info for tx facts. */
export const mainnetFeed = (o: HttpOpts & { apiKey?: () => string | undefined } = {}): OrdFeed =>
  new OrdFeed([new UniSat(o), new OrdRecursive("https://ordinals.com", o), new OrdinalsWallet(o)], { log: o.log, now: o.now,
    esplora: [new Esplora("https://mempool.space/api", o), new Esplora("https://blockstream.info/api", o)] });
