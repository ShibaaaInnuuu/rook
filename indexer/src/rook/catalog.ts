import { readFileSync } from "node:fs";
import type { WrappedConfig } from "../wrapped/config.js";
import type { WrappedService } from "../wrapped/service.js";
import { ruleOf, statusOf, weiOfEth } from "./catalogRule.js";
import { coinbaseSatsPerEth, fetchPicture, MagicEden, OpenSea, OrdinalsWallet, spacer, TokenUri, type Fetch, type Picture, type Read } from "./catalogSources.js";
import { guardedFetch } from "../outbound.js";
import type { Catalog, CatalogChain, CatalogEntry, CatalogFloor, CatalogItem, CatalogOne, CatalogSourceName, CatalogSourceRead, CatalogStandard } from "./catalogTypes.js";

/**
 * docs/v6/collections-catalog.md: the directory of collections, whether or not Rook lists them — every one an open nest. The
 * curated seed is the spine; the sources put today's figures on it; the wrapped collections this indexer is configured with
 * mark what is `live`; the listing rule (catalogRule.ts) says where everything else stands.
 *
 * `list()` never waits on the network: it serves the snapshot it has — the seed's dated records, flagged stale, until the first
 * refresh lands — and starts a refresh when the last one is older than the TTL. `refresh()` is the refresh itself, awaited by
 * tests and by the fixture writer. A refresh that fails in part serves what did answer and flags the rest stale; one that fails
 * entirely leaves the last snapshot standing. No figure is ever invented to fill a gap.
 *
 * Pictures (`picture()`) are fetched on demand from the URLs the catalogue itself holds — a collection's image, a sample item's —
 * checked to be pictures, kept in a bounded cache and served from the indexer's own host, so a visitor's browser never talks to
 * a marketplace. A picture that cannot be had is `null`, and the site draws a mark instead.
 */
export type SeedRecord = { at: number; floorNative: string; unit: "sats" | "wei"; items: number | null; owners: number | null; listed: number | null; source: CatalogSourceName; ref: string };
export type SeedEntry = { slug: string; name: string; chain: CatalogChain; chainName: string; chainId: number | null; standard: CatalogStandard; id: string; description: string | null; image: string | null;
  links: CatalogEntry["links"]; sources: { ordinalsWallet?: string; magicEden?: string; opensea?: string }; sample?: CatalogItem[]; recorded: SeedRecord | null };
export type Seed = { note: string; writtenAt: string; /** the sats-per-ETH reference when the seed was written: a stale rate until a fresh one is read */ rate: { satsPerEth: string; at: number; source: "coinbase"; ref: string } | null; entries: SeedEntry[] };
export const loadSeed = (path: URL | string = new URL("./catalog-seed.json", import.meta.url)): Seed => parseSeed(JSON.parse(readFileSync(path, "utf8")));
const SLUG = /^[a-z0-9_-]{1,64}$/i, ADDR = /^0x[0-9a-f]{40}$/;
const CHAINS = new Set(["bitcoin", "ethereum", "base", "robinhood", "other"]), STANDARDS = new Set(["inscriptions", "erc721", "cryptopunks", "erc1155", "other"]);
export function parseSeed(raw: unknown): Seed {
  const s = raw as Seed;
  if (!s || !Array.isArray(s.entries)) throw new Error("a seed has entries[]");
  if (s.rate && (!/^\d+$/.test(s.rate.satsPerEth) || !Number.isInteger(s.rate.at))) throw new Error("seed: the rate is an integer sats per ETH and a time");
  const seen = new Set<string>();
  for (const e of s.entries) {
    if (!SLUG.test(e.slug ?? "")) throw new Error(`seed: bad slug ${e.slug}`);
    if (seen.has(e.slug)) throw new Error(`seed: duplicate slug ${e.slug}`);
    seen.add(e.slug);
    if (!CHAINS.has(e.chain)) throw new Error(`seed ${e.slug}: bad chain`);
    if (typeof e.chainName !== "string" || !e.chainName) throw new Error(`seed ${e.slug}: a chain name`);
    if (!STANDARDS.has(e.standard)) throw new Error(`seed ${e.slug}: bad standard`);
    if ((e.chain === "ethereum" || e.chain === "base" || e.chain === "robinhood") && !ADDR.test(e.id ?? "")) throw new Error(`seed ${e.slug}: an EVM entry's id is its contract, lower-case`);
    if (e.chain === "bitcoin" && e.standard !== "inscriptions") throw new Error(`seed ${e.slug}: a Bitcoin entry is an inscription collection`);
    if (e.recorded && (!/^\d+$/.test(e.recorded.floorNative) || !Number.isInteger(e.recorded.at))) throw new Error(`seed ${e.slug}: a record has an integer floor and a time`);
    if (e.sample && (!Array.isArray(e.sample) || e.sample.length > 12)) throw new Error(`seed ${e.slug}: a sample is at most twelve items`);
  }
  return s;
}
export const CHAIN_NAME: Record<CatalogChain, string> = { bitcoin: "Bitcoin", ethereum: "Ethereum", base: "Base", robinhood: "Robinhood Chain", other: "another chain" };
export const CHAIN_ID: Record<CatalogChain, number | null> = { bitcoin: null, ethereum: 1, base: 8453, robinhood: 4663, other: null };

export type CatalogDeps = { seed?: Seed; /** null: no network, the seed alone (the fixture writer, a test) */ fetch?: Fetch | null; wrapped?: WrappedService | null; wrappedConfig?: WrappedConfig | null;
  now?: () => number; ttlS?: number; /** how long a collection's name, picture, links and item sample are kept before being asked again (default 6 h) */ detailTtlS?: number;
  source?: Catalog["source"]; log?: (o: Record<string, unknown>) => void; apiKey?: () => string | undefined;
  usdPerEth?: () => number | null; sleep?: (ms: number) => Promise<void>; /** spacing per host, ms */ pace?: { ordinalswallet?: number; magiceden?: number; opensea?: number; pictures?: number; rpc?: number };
  /** the picture cache's bounds: entries and total bytes */ pictureCache?: { entries: number; bytes: number } };
const LIVE: Catalog["source"] = { kind: "live", note: "Names and links from the curated seed (Nosh's registry on Robinhood Chain and the ordinals collections the old etched site listed); floors, counts, volumes and item samples from the marketplaces' public APIs at the last refresh; pictures fetched and cached by this indexer; Rook status from its wrapped collections and the listing rule." };
const E = 10n ** 18n;
const SAMPLE = 12;
const norm = (s: string) => s.toLowerCase().replace(/[^a-z0-9]+/g, "");
type Reads = Partial<Record<CatalogSourceName, { asked: number; answered: number; statuses: number[]; notes: Set<string> }>>;
type Detail = { at: number; name?: string | null; description?: string | null; image?: string | null; items?: number | null; links?: Partial<CatalogEntry["links"]>; sample?: CatalogItem[] };

/** A background refresh older than this is presumed hung (each request has its own 30 s deadline; a refresh makes many, paced). */
export const REFRESH_MAX_MS = 15 * 60_000;
export class CatalogService {
  private readonly seed: Seed;
  private readonly now: () => number;
  private readonly ttl: number;
  private readonly detailTtl: number;
  private snap: Catalog | null = null;
  private inflight: Promise<Catalog> | null = null;
  private inflightAt = 0;
  private rate: { satsPerEth: bigint; at: number; fresh: boolean } | null = null;
  /** a collection's slow-moving facts, kept between refreshes so the sources are asked for them a few times a day, not every ten minutes */
  private readonly details = new Map<string, Detail>();
  private readonly pictures = new Map<string, { at: number; p: Picture | null }>();
  private pictureBytes = 0;
  private readonly ow: OrdinalsWallet | null; private readonly me: MagicEden | null; private readonly os: OpenSea | null; private readonly uri: TokenUri | null;
  private readonly pictureGate: () => Promise<void>;
  private readonly fetchImpl: Fetch | null;
  constructor(private readonly d: CatalogDeps) {
    this.seed = d.seed ?? loadSeed();
    this.now = d.now ?? (() => Math.floor(Date.now() / 1000));
    this.ttl = d.ttlS ?? 600;
    this.detailTtl = d.detailTtlS ?? 6 * 3_600;
    if (this.seed.rate) this.rate = { satsPerEth: BigInt(this.seed.rate.satsPerEth), at: this.seed.rate.at, fresh: false };
    // E-12: by default every outbound request of the catalogue is guarded (outbound.ts): resolved and checked at connect time,
    // redirects re-checked by hand, one 30 s deadline, a 2 MB body cap. A test or the fixture writer passes its own transport, or null for none.
    const f = d.fetch === undefined ? guardedFetch({ timeoutMs: 30_000 }) : d.fetch, ms = () => this.now() * 1000, sleep = d.sleep;
    this.fetchImpl = f;
    this.ow = f ? new OrdinalsWallet(f, spacer(d.pace?.ordinalswallet ?? 900, ms, sleep), this.now) : null;
    this.me = f ? new MagicEden(f, spacer(d.pace?.magiceden ?? 600, ms, sleep)) : null;
    this.os = f ? new OpenSea(f, spacer(d.pace?.opensea ?? 300, ms, sleep), d.apiKey) : null;
    this.uri = f ? new TokenUri(f, spacer(d.pace?.rpc ?? 120, ms, sleep)) : null;
    this.pictureGate = spacer(d.pace?.pictures ?? 150, ms, sleep);
  }
  private get source() { return this.d.source ?? LIVE; }

  /** The snapshot, at once; a refresh in the background when one is due and there is a network to ask. */
  async list(): Promise<Catalog> {
    if (!this.snap) this.snap = await this.compose(new Map(), this.rate, {}, null, null);
    const due = (this.snap.attemptedAt ?? -Infinity) + this.ttl <= this.now();
    // R3-check M-E: a refresh that never settles no longer blocks every later one: after REFRESH_MAX_MS a new one may start
    if (due && this.fetchImpl !== null && (!this.inflight || Date.now() - this.inflightAt > REFRESH_MAX_MS)) {
      const p: Promise<Catalog> = this.refresh().finally(() => { if (this.inflight === p) this.inflight = null; });
      this.inflight = p; this.inflightAt = Date.now();
      p.catch(() => undefined);
    }
    return this.restatus(this.snap);
  }
  async one(slug: string): Promise<CatalogOne | null> {
    const c = await this.list(), key = slug.toLowerCase();
    const e = c.entries.find((x) => x.slug.toLowerCase() === key || x.id.toLowerCase() === key || (x.rook.kind === "live" && x.rook.slug.toLowerCase() === key)) ?? null;
    return e ? { source: c.source, at: c.at, stale: c.stale, satsPerEth: c.satsPerEth, usdPerEth: c.usdPerEth, rule: c.rule, entry: e } : null;
  }

  /** A collection's picture, or one of its sample items' (`item` is the index in `sample`): fetched once from the URL the catalogue holds, cached, checked. Null when it cannot be had. */
  async picture(slug: string, item: number | null = null): Promise<Picture | null> {
    const one = await this.one(slug);
    if (!one || !this.fetchImpl) return null;
    const u = item === null ? one.entry.image : one.entry.sample[item]?.image ?? null;
    if (!u) return null;
    const key = `${one.entry.slug}${item === null ? "" : `/${item}`}`, now = this.now(), hit = this.pictures.get(key);
    if (hit && now - hit.at < (hit.p ? 24 * 3_600 : 600)) return hit.p;
    await this.pictureGate();
    const r = await fetchPicture(u, this.fetchImpl);
    const p = r.ok ? r.value : null;
    if (hit?.p) this.pictureBytes -= hit.p.body.byteLength;
    this.pictures.set(key, { at: now, p });
    if (p) this.pictureBytes += p.body.byteLength;
    const cap = this.d.pictureCache ?? { entries: 800, bytes: 64 * 1024 * 1024 };
    // oldest first out, until the cache is within its bounds again
    while (this.pictures.size > cap.entries || this.pictureBytes > cap.bytes) {
      const oldest = this.pictures.keys().next().value as string | undefined;
      if (oldest === undefined) break;
      const gone = this.pictures.get(oldest); this.pictures.delete(oldest);
      if (gone?.p) this.pictureBytes -= gone.p.body.byteLength;
    }
    if (!r.ok) this.d.log?.({ level: "debug", event: "catalog-picture-miss", slug: one.entry.slug, item, status: r.status, why: r.error });
    return p;
  }

  /** Ask every source about every seeded entry, once, spaced per host; then compose. Never throws: a source that fails is a row in `sources`. */
  async refresh(): Promise<Catalog> {
    const t0 = Date.now(), now = this.now(), reads: Reads = {};
    const note = (name: CatalogSourceName, r: Read<unknown>, what: string) => {
      const x = (reads[name] ??= { asked: 0, answered: 0, statuses: [], notes: new Set() });
      x.asked++; if (r.ok) x.answered++; if (r.status !== null) x.statuses.push(r.status);
      if (!r.ok) x.notes.add(`${what}: ${r.error}`);
    };
    const fetched = new Map<string, Fetched>();
    let rate = this.rate ? { ...this.rate, fresh: false } : null;
    if (this.fetchImpl !== null) {
      const rateRead = await coinbaseSatsPerEth(this.fetchImpl);
      note("coinbase", rateRead, "rate");
      if (rateRead.ok) rate = { satsPerEth: rateRead.value, at: now, fresh: true };
    }
    await Promise.all(this.seed.entries.map(async (e) => {
      const f: Fetched = { sources: [] };
      const at = () => this.now();
      const rec = (name: CatalogSourceName, r: Read<unknown>, what: string) => { note(name, r, what); f.sources.push({ name, status: r.status, ok: r.ok, at: at(), note: r.ok ? "answered" : r.error }); };
      const kept = this.details.get(e.slug), askDetail = !kept || now - kept.at >= this.detailTtl;
      const detail: Detail = { at: now };
      if (e.chain === "bitcoin") {
        if (this.ow && e.sources.ordinalsWallet) {
          const s = e.sources.ordinalsWallet;
          const [book, sales, info, items] = await Promise.all([this.ow.book(s), this.ow.sales(s), askDetail ? this.ow.info(s) : null, askDetail ? this.ow.items(s, SAMPLE) : null]);
          rec("ordinalswallet", book, `${s} escrows`); rec("ordinalswallet", sales, `${s} sales`);
          if (info) { rec("ordinalswallet", info, `${s} info`); if (info.ok) { detail.name = info.value.name; detail.description = info.value.description; detail.image = info.value.image; detail.items = info.value.items; detail.links = info.value.links; } }
          if (items) { rec("ordinalswallet", items, `${s} inscriptions`); if (items.ok) detail.sample = items.value.filter((i) => !i.contentType || /^image\//.test(i.contentType)).slice(0, SAMPLE).map((i) => ({ id: i.id, name: i.name ?? (i.num !== null ? `#${i.num}` : null), image: `https://ordinals.com/content/${i.id}`, url: `https://ordinals.com/inscription/${i.id}` })); }
          if (book.ok) { f.listed = book.value.listed; if (book.value.floorSats !== null) f.floor = { native: book.value.floorSats, unit: "sats", source: "ordinalswallet", ref: `ordinalswallet:${s}/escrows`, at: at() }; }
          if (sales.ok) { f.sats24h = sales.value.sats24h; f.sats7d = sales.value.sats7d; }
        }
        if (this.me && e.sources.magicEden) {
          const s = e.sources.magicEden, st = await this.me.stat(s);
          rec("magiceden", st, `${s} stat`);
          if (st.ok) { f.owners = st.value.owners; detail.items ??= st.value.items; f.listed ??= st.value.listed; if (!f.floor && st.value.floorSats !== null) f.floor = { native: st.value.floorSats, unit: "sats", source: "magiceden", ref: `magiceden:stat:${s}`, at: at() }; }
        }
      } else if (this.os && e.sources.opensea) {
        const s = e.sources.opensea;
        const [st, col, items] = await Promise.all([this.os.stats(s), askDetail ? this.os.collection(s) : null, askDetail ? this.os.items(s, SAMPLE) : null]);
        rec("opensea", st, `${s} stats`);
        if (col) { rec("opensea", col, `${s} collection`); if (col.ok) { detail.name = col.value.name; detail.description = col.value.description; detail.image = col.value.image; detail.items = col.value.items; detail.links = col.value.links; } }
        if (items) { rec("opensea", items, `${s} nfts`); if (items.ok && items.value.length) detail.sample = items.value.slice(0, SAMPLE).map((i) => ({ id: i.id, name: i.name ?? `#${i.id}`, image: i.image, url: i.url })); }
        // keyless: the contract's own tokenURI through a public RPC and its metadata's picture, when OpenSea's items needed the key (or answered nothing)
        if (askDetail && !detail.sample && this.uri && e.chain !== "other" && e.standard !== "cryptopunks") {
          const nftUrl = (id: string) => (e.chain === "ethereum" ? `https://etherscan.io/nft/${e.id}/${id}` : e.chain === "base" ? `https://basescan.org/nft/${e.id}/${id}` : e.chain === "robinhood" ? `https://explorer.mainnet.chain.robinhood.com/token/${e.id}/instance/${id}` : null);
          const via = await this.uri.items(e.chain, e.id, nftUrl, 8);
          rec("tokenuri", via, `${e.slug} tokenURI`);
          if (via.ok && via.value.length) detail.sample = via.value.map((i) => ({ id: i.id, name: i.name ?? `#${i.id}`, image: i.image, url: i.url }));
        }
        if (st.ok) { f.owners = st.value.owners; f.volume24hEth = st.value.volume24hEth; f.volume7dEth = st.value.volume7dEth; if (st.value.floorWei !== null) f.floor = { native: st.value.floorWei, unit: "wei", source: "opensea", ref: `opensea:collections/${s}/stats`, at: at() }; }
      }
      // what was asked for and answered this time replaces the kept detail field by field; what was not asked is kept as it was
      const merged: Detail = askDetail ? { at: now, ...(kept ?? {}), ...Object.fromEntries(Object.entries(detail).filter(([, v]) => v !== undefined && v !== null)) } : kept ?? { at: now };
      if (askDetail && (detail.name !== undefined || detail.sample !== undefined)) merged.at = now; else if (kept) merged.at = kept.at;
      this.details.set(e.slug, merged);
      f.detail = merged;
      fetched.set(e.slug, f);
    }));
    const asked = Object.values(reads).reduce((n, x) => n + x.asked, 0), answered = Object.values(reads).reduce((n, x) => n + x.answered, 0);
    this.rate = rate;
    const snap = await this.compose(fetched, rate, reads, now, asked === answered && asked > 0 ? now : (this.snap?.refreshedAt ?? null));
    this.snap = snap;
    this.d.log?.({ level: answered === asked ? "info" : "warn", event: "catalog-refresh", asked, answered, entries: snap.entries.length, stale: snap.stale, ms: Date.now() - t0,
      sources: Object.fromEntries(Object.entries(reads).map(([k, v]) => [k, `${v.answered}/${v.asked}`])) });
    return snap;
  }

  // ---- composing one snapshot from the seed, what was fetched, and the wrapped collections -----------------------------------------------
  private async liveIndex(): Promise<{ byContract: Map<string, Live>; byName: Map<string, Live>; all: Live[] }> {
    const byContract = new Map<string, Live>(), byName = new Map<string, Live>(), all: Live[] = [];
    const cfg = this.d.wrappedConfig;
    if (cfg && this.d.wrapped) {
      const list = await this.d.wrapped.list().catch(() => null);
      for (const c of cfg.collections) {
        const sum = list?.collections.find((x) => x.slug === c.slug) ?? null;
        const chain: CatalogChain = c.custody.kind === "bitcoin" ? "bitcoin" : c.custody.chainId === 8453 ? "base" : c.custody.chainId === 4663 ? "robinhood" : c.custody.chainId === 1 ? "ethereum" : "other";
        const l: Live = { slug: c.slug, name: c.name, coins: sum?.coins ?? 0, chain, chainName: chain === "other" ? `chain ${c.custody.kind === "evm" ? c.custody.chainId : "?"}` : CHAIN_NAME[chain], chainId: c.custody.kind === "evm" ? c.custody.chainId : null,
          id: c.custody.kind === "evm" ? c.custody.collection.toLowerCase() : c.slug, kind: c.kind, floorWei: sum?.floorWei ?? null, floorFresh: sum?.floorFresh ?? false, itemsHeld: sum?.itemsHeld ?? null, description: c.description ?? null };
        all.push(l);
        if (c.custody.kind === "evm") byContract.set(l.id, l);
        byName.set(norm(c.name), l); byName.set(norm(c.slug), l);
      }
    }
    return { byContract, byName, all };
  }
  private liveOf(e: { chain: CatalogChain; id: string; name: string; slug: string }, live: { byContract: Map<string, Live>; byName: Map<string, Live> }): Live | null {
    return (e.chain !== "bitcoin" && e.chain !== "other" ? live.byContract.get(e.id.toLowerCase()) : undefined) ?? live.byName.get(norm(e.name)) ?? live.byName.get(norm(e.slug)) ?? null;
  }
  private async compose(fetched: Map<string, Fetched>, rate: { satsPerEth: bigint; at: number; fresh: boolean } | null, reads: Reads, attemptedAt: number | null, refreshedAt: number | null): Promise<Catalog> {
    const now = this.now(), live = await this.liveIndex(), rule = ruleOf();
    const sats = rate?.satsPerEth ?? null;
    const ethOfSats = (v: bigint): number | null => (sats === null ? null : Number((v * E) / sats) / 1e18);
    const ethOf = (native: bigint, unit: "sats" | "wei") => (unit === "wei" ? Number(native / 10n ** 9n) / 1e9 : ethOfSats(native));
    let anyStale = false;
    const matched = new Set<string>();
    const entries: CatalogEntry[] = this.seed.entries.map((e) => {
      const f: Fetched = fetched.get(e.slug) ?? { sources: [] }, d: Partial<Detail> = f.detail ?? this.details.get(e.slug) ?? {};
      const l = this.liveOf(e, live);
      if (l) matched.add(l.slug);
      // the floor: a fresh read; else the seed's dated record, stale; else the keeper's cap for a live nest; else none
      let floor: CatalogFloor = null;
      if (f.floor) floor = { native: String(f.floor.native), unit: f.floor.unit, eth: ethOf(f.floor.native, f.floor.unit), source: f.floor.source, ref: f.floor.ref, at: f.floor.at, stale: false };
      else if (e.recorded) { anyStale = true; floor = { native: e.recorded.floorNative, unit: e.recorded.unit, eth: ethOf(BigInt(e.recorded.floorNative), e.recorded.unit), source: "seed", ref: `seed:${e.recorded.source}:${e.recorded.ref}`, at: e.recorded.at, stale: true }; }
      else if (l?.floorWei) floor = { native: l.floorWei, unit: "wei", eth: Number(BigInt(l.floorWei) / 10n ** 9n) / 1e9, source: "rook", ref: `rook:wrapped:${l.slug}`, at: now, stale: !l.floorFresh };
      if (!f.sources.length || f.sources.some((s) => !s.ok)) anyStale = true;
      const floorWei = floor ? (floor.unit === "wei" ? BigInt(floor.native) : sats === null ? null : (BigInt(floor.native) * E) / sats) : null;
      return {
        slug: e.slug, name: d.name ?? e.name, chain: e.chain, chainName: e.chainName, chainId: e.chainId, standard: e.standard, id: e.id, description: d.description ?? e.description, image: d.image ?? e.image,
        links: { ...e.links, ...(d.links ? Object.fromEntries(Object.entries(d.links).filter(([, v]) => v !== null)) : {}) },
        floor, items: d.items ?? e.recorded?.items ?? null, owners: f.owners ?? e.recorded?.owners ?? null, listed: f.listed ?? (f.floor ? null : e.recorded?.listed ?? null),
        volume24hEth: f.volume24hEth ?? (f.sats24h !== undefined && f.sats24h !== null ? ethOfSats(f.sats24h) : null),
        volume7dEth: f.volume7dEth ?? (f.sats7d !== undefined && f.sats7d !== null ? ethOfSats(f.sats7d) : null),
        heldByRook: l ? l.itemsHeld : null,
        sample: (d.sample ?? e.sample ?? []).slice(0, SAMPLE),
        sources: f.sources.length ? f.sources : e.recorded ? [{ name: "seed", status: null, ok: true, at: e.recorded.at, note: `the seed's record of ${new Date(e.recorded.at * 1000).toISOString().slice(0, 10)}, from ${e.recorded.source}` }] : [],
        rook: statusOf({ chain: e.chain, chainName: e.chainName, standard: e.standard, floorWei, floorStale: floor?.stale ?? false, live: l ? { slug: l.slug, coins: l.coins } : null }),
        curated: true,
      };
    });
    // a nest Rook lists that the seed does not name is still a collection on the site: composed from this indexer's own reads
    for (const l of live.all) {
      if (matched.has(l.slug)) continue;
      const floor: CatalogFloor = l.floorWei ? { native: l.floorWei, unit: "wei", eth: Number(BigInt(l.floorWei) / 10n ** 9n) / 1e9, source: "rook", ref: `rook:wrapped:${l.slug}`, at: now, stale: !l.floorFresh } : null;
      const standard: CatalogStandard = l.kind === "inscriptions" ? "inscriptions" : "erc721";
      entries.push({ slug: l.slug, name: l.name, chain: l.chain, chainName: l.chainName, chainId: l.chainId, standard, id: l.id, description: l.description, image: null,
        links: { website: null, x: null, discord: null, marketplace: null, explorer: null }, floor, items: l.itemsHeld, owners: null, listed: null, volume24hEth: null, volume7dEth: null, heldByRook: l.itemsHeld, sample: [],
        sources: [{ name: "rook", status: null, ok: true, at: now, note: "this indexer's wrapped-collection reads; the seed does not name it" }],
        rook: statusOf({ chain: l.chain, chainName: l.chainName, standard, floorWei: floor ? BigInt(floor.native) : null, floorStale: floor?.stale ?? false, live: { slug: l.slug, coins: l.coins } }), curated: false });
    }
    const sources = (Object.entries(reads) as [CatalogSourceName, NonNullable<Reads[CatalogSourceName]>][]).map(([name, x]) => ({ name, asked: x.asked, answered: x.answered, statuses: [...new Set(x.statuses)].sort((a, b) => a - b), note: [...x.notes].slice(0, 6).join("; ") || "every read answered" }));
    return { source: this.source, at: now, refreshedAt, attemptedAt, ttlS: this.ttl, stale: anyStale || refreshedAt === null,
      sources, satsPerEth: sats === null ? null : Number(sats), usdPerEth: this.d.usdPerEth?.() ?? null,
      rate: rate ? { source: "coinbase", at: rate.at, stale: !rate.fresh } : null, rule, entries };
  }
  /** Rook status can change between refreshes (a nest listed, a coin launched): recomputed from the wrapped reads on every answer. */
  private async restatus(c: Catalog): Promise<Catalog> {
    const live = await this.liveIndex();
    if (!live.all.length && !c.entries.some((e) => e.rook.kind === "live")) return { ...c, at: this.now(), usdPerEth: this.d.usdPerEth?.() ?? c.usdPerEth };
    const sats = c.satsPerEth === null ? null : BigInt(Math.round(c.satsPerEth));
    const entries = c.entries.map((e) => {
      if (!e.curated) return e;
      const l = this.liveOf(e, live);
      const floorWei = e.floor ? (e.floor.unit === "wei" ? BigInt(e.floor.native) : sats === null ? null : (BigInt(e.floor.native) * E) / sats) : null;
      return { ...e, heldByRook: l ? l.itemsHeld : null, rook: statusOf({ chain: e.chain, chainName: e.chainName, standard: e.standard, floorWei, floorStale: e.floor?.stale ?? false, live: l ? { slug: l.slug, coins: l.coins } : null }) };
    });
    return { ...c, at: this.now(), usdPerEth: this.d.usdPerEth?.() ?? c.usdPerEth, entries };
  }
}
type Live = { slug: string; name: string; coins: number; chain: CatalogChain; chainName: string; chainId: number | null; id: string; kind: "inscriptions" | "nft"; floorWei: string | null; floorFresh: boolean; itemsHeld: number | null; description: string | null };
type Fetched = { owners?: number | null; listed?: number | null; floor?: { native: bigint; unit: "sats" | "wei"; source: CatalogSourceName; ref: string; at: number }; sats24h?: bigint | null; sats7d?: bigint | null;
  volume24hEth?: number | null; volume7dEth?: number | null; detail?: Detail; sources: CatalogSourceRead[] };
export { weiOfEth };
