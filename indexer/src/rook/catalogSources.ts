/**
 * docs/v6/collections-catalog.md §2: where the catalogue's facts come from. Every source here READS — GET only, spaced per host,
 * with a User-Agent on every request (Cloudflare fronts answer 403 to a bare client) — and an answer it does not understand is
 * `ok: false` with the status, never a number. Nothing here buys, signs or sends.
 *
 * The one secret, OPENSEA_API_KEY, is read from the environment AT THE MOMENT OF THE CALL (never at load), goes into exactly one
 * header of exactly one host, and appears in no error and no log line: an error carries the HTTP status and nothing of the
 * request. Tests use a fake key and a mocked fetch and hold that. Keyless, OpenSea's `collections/<slug>` and `/stats` answered
 * 200 on 2026-09-23 with intermittent 401s; a key makes the answers dependable, it does not change their shape.
 */
import { decodeFunctionResult, encodeFunctionData, parseAbi, type Hex } from "viem";
import { hostRefusal, isPublicIp, jsonCapped, readCapped, TooLarge } from "../outbound.js";
export type Fetch = typeof fetch;
export type Read<T> = { ok: true; status: number; value: T } | { ok: false; status: number | null; error: string };
export const UA = "rook-indexer-catalog/1 (read-only; collections directory)";

const defaultSleep = (ms: number) => new Promise<void>((ok) => setTimeout(ok, ms));
/** At most one request per `everyMs` to one host, whatever asks. */
export const spacer = (everyMs: number, now: () => number = Date.now, sleep: (ms: number) => Promise<void> = defaultSleep) => {
  let last = -Infinity, chain: Promise<void> = Promise.resolve();
  return () => { chain = chain.then(async () => { const wait = last + everyMs - now(); if (wait > 0) await sleep(wait); last = now(); }); return chain; };
};
export type Gate = () => Promise<void>;

/**
 * One GET, parsed as JSON. A network failure is `status: null`; a non-2xx is its status; a body that is not JSON is `ok: false` too.
 * E-12: the URL must pass the name rules (`hostRefusal`: https, a public name or a public IP literal) before anything is asked, and
 * the body is read under `JSON_MAX_BYTES` (2 MB) and abandoned past it. In production `fetchImpl` is `outbound.guardedFetch`, which
 * also resolves the host at connect time, refuses private addresses and re-checks every redirect hop.
 */
export async function getJson(url: string, fetchImpl: Fetch, headers: Record<string, string> = {}): Promise<Read<unknown>> {
  const refused = hostRefusal(url);
  if (refused) return { ok: false, status: null, error: `refused: ${refused}` };
  let r: Response;
  try { r = await fetchImpl(url, { method: "GET", headers: { accept: "application/json", "user-agent": UA, ...headers }, redirect: "manual", signal: AbortSignal.timeout(15_000) }); }
  catch (err) { return { ok: false, status: null, error: `no answer: ${String((err as Error)?.name ?? "error")}` }; }
  if (!r.ok) { await r.body?.cancel().catch(() => undefined); return { ok: false, status: r.status, error: `HTTP ${r.status}` }; }
  try { return { ok: true, status: r.status, value: await jsonCapped(r) }; }
  catch (err) { return { ok: false, status: r.status, error: err instanceof TooLarge ? "too large" : "not JSON" }; }
}

const whole = (v: unknown): bigint | null => ((typeof v === "number" && Number.isInteger(v) && v >= 0) || (typeof v === "string" && /^\d+$/.test(v)) ? BigInt(v) : null);
const int = (v: unknown): number | null => { const n = typeof v === "string" && /^\d+$/.test(v) ? Number(v) : v; return typeof n === "number" && Number.isInteger(n) && n >= 0 ? n : null; };
const num = (v: unknown): number | null => { const n = typeof v === "string" ? Number(v) : v; return typeof n === "number" && Number.isFinite(n) && n >= 0 ? n : null; };
const str = (v: unknown): string | null => (typeof v === "string" && v.trim() ? v.trim() : null);
const url = (v: unknown): string | null => { const s = str(v); return s && /^https?:\/\//.test(s) ? s : null; };
const SLUG = /^[a-z0-9_-]{1,64}$/i;

// ---- Ordinals Wallet: a keyless Bitcoin marketplace -----------------------------------------------------------------------------------
export const OW_BASE = "https://turbo.ordinalswallet.com";
export type OwInfo = { name: string | null; description: string | null; image: string | null; items: number | null; verified: boolean; links: { website: string | null; x: string | null; discord: string | null }; first: number | null; last: number | null };
export type OwBook = { floorSats: bigint | null; listed: number };
export type OwSales = { sats24h: bigint | null; sats7d: bigint | null; sales: number; oldestAt: number | null };

/**
 * The collection's icon as a browser can load it (2026-09-23): the icon's inscription at the ordinals content gateway when it is
 * an image (Ordinals Wallet names it as a preview on its own host, which answers 404); an IPFS icon on a working gateway; a Discord
 * attachment is no picture (Discord's attachment links expire, and these answer 404); anything else https as it is.
 */
export function owIcon(icon: unknown, inscription: unknown): string | null {
  const ins = inscription as { id?: unknown; content_type?: unknown } | null;
  if (ins && typeof ins.id === "string" && /^[0-9a-f]{64}i\d+$/.test(ins.id) && typeof ins.content_type === "string" && ins.content_type.startsWith("image/")) return `https://ordinals.com/content/${ins.id}`;
  const u = url(icon);
  if (!u || /^https:\/\/(?:cdn|media)\.discordapp\.(?:com|net)\//.test(u)) return null;
  const preview = u.match(/^https:\/\/(?:turbo|cdn)\.ordinalswallet\.com\/inscription\/preview\/([0-9a-f]{64}i\d+)$/);
  return preview ? `https://ordinals.com/content/${preview[1]}` : gatewayed(u);
}
export function parseOwInfo(j: unknown): OwInfo | null {
  const x = j as Record<string, unknown> | null;
  if (!x || typeof x !== "object" || typeof x.slug !== "string") return null;
  const s = (x.socials ?? {}) as Record<string, unknown>;
  const icon = owIcon(x.icon, x.icon_inscription);
  return { name: str(x.name), description: str(x.description), image: icon, items: int(x.total_supply), verified: x.verified === true,
    links: { website: url(s.website), x: url(s.twitter), discord: url(s.discord) }, first: int(x.lowest_inscription_num), last: int(x.highest_inscription_num) };
}
/** The escrows are the open listings: the floor is the cheapest one not yet bought. Nothing listed is `floorSats: null`, not zero. */
export function parseOwBook(j: unknown): OwBook | null {
  const rows = Array.isArray(j) ? j : (j as { listings?: unknown })?.listings;
  if (!Array.isArray(rows)) return null;
  let floor: bigint | null = null, listed = 0;
  for (const r of rows as Record<string, unknown>[]) {
    if (r.bought_at || r.purchase_txid) continue;
    const p = whole(r.satoshi_price);
    if (p === null || p === 0n) continue;
    listed++;
    if (floor === null || p < floor) floor = p;
  }
  return { floorSats: floor, listed };
}
/** The day's and the week's sales in sats, only when the list reaches back that far: a truncated list gives a lower bound, which is not a volume. */
export function parseOwSales(j: unknown, now: number): OwSales | null {
  const rows = (j as { sales?: unknown })?.sales;
  if (!Array.isArray(rows)) return null;
  let d = 0n, w = 0n, n = 0, oldest: number | null = null;
  for (const r of rows as Record<string, unknown>[]) {
    const at = int(r.block_timestamp), p = whole(r.price_sats);
    if (at === null || p === null) continue;
    n++; oldest = oldest === null ? at : Math.min(oldest, at);
    if (at >= now - 86_400) d += p;
    if (at >= now - 7 * 86_400) w += p;
  }
  const reaches = (span: number) => n > 0 && oldest !== null && oldest <= now - span;
  return { sats24h: reaches(86_400) ? d : null, sats7d: reaches(7 * 86_400) ? w : null, sales: n, oldestAt: oldest };
}
/** `collection/<slug>/inscriptions`: a page of the collection's items, kept to those whose content is an image (a grid shows pictures). */
export type OwItem = { id: string; num: number | null; name: string | null; contentType: string | null };
export function parseOwItems(j: unknown): OwItem[] | null {
  if (!Array.isArray(j)) return null;
  const out: OwItem[] = [];
  for (const r of j as Record<string, unknown>[]) {
    if (typeof r.id !== "string" || !/^[0-9a-f]{64}i\d+$/.test(r.id)) continue;
    out.push({ id: r.id, num: int(r.num), name: str((r.meta as Record<string, unknown> | undefined)?.name), contentType: str(r.content_type) });
  }
  return out;
}
export class OrdinalsWallet {
  readonly name = "ordinalswallet" as const;
  constructor(private readonly fetchImpl: Fetch, private readonly gate: Gate, private readonly now: () => number = () => Math.floor(Date.now() / 1000)) {}
  private async get<T>(path: string, parse: (j: unknown) => T | null): Promise<Read<T>> {
    await this.gate();
    const r = await getJson(`${OW_BASE}/${path}`, this.fetchImpl);
    if (!r.ok) return r;
    const v = parse(r.value);
    return v === null ? { ok: false, status: r.status, error: "unexpected shape" } : { ok: true, status: r.status, value: v };
  }
  info(slug: string) { if (!SLUG.test(slug)) throw new Error("invalid slug"); return this.get(`collection/${slug}`, parseOwInfo); }
  book(slug: string) { if (!SLUG.test(slug)) throw new Error("invalid slug"); return this.get(`collection/${slug}/escrows`, parseOwBook); }
  sales(slug: string) { if (!SLUG.test(slug)) throw new Error("invalid slug"); return this.get(`collection/${slug}/sales`, (j) => parseOwSales(j, this.now())); }
  items(slug: string, limit = 12) { if (!SLUG.test(slug)) throw new Error("invalid slug"); return this.get(`collection/${slug}/inscriptions?offset=0&limit=${limit}`, parseOwItems); }
}

// ---- Magic Eden: keyless collection stats (their `/v2/ord/btc` upstream answered 503 for most symbols on 2026-09-23) ----------------
export const ME_BASE = "https://api-mainnet.magiceden.dev/v2/ord/btc";
export type MeStat = { floorSats: bigint | null; owners: number | null; items: number | null; listed: number | null; volumeAllSats: bigint | null };
export function parseMeStat(j: unknown): MeStat | null {
  const x = j as Record<string, unknown> | null;
  if (!x || typeof x !== "object" || typeof x.symbol !== "string") return null;
  const f = whole(x.floorPrice);
  return { floorSats: f === null || f === 0n ? null : f, owners: int(x.owners), items: int(x.supply), listed: int(x.totalListed), volumeAllSats: whole(x.totalVolume) };
}
export class MagicEden {
  readonly name = "magiceden" as const;
  constructor(private readonly fetchImpl: Fetch, private readonly gate: Gate) {}
  async stat(symbol: string): Promise<Read<MeStat>> {
    if (!SLUG.test(symbol)) throw new Error("invalid symbol");
    await this.gate();
    const r = await getJson(`${ME_BASE}/stat?collectionSymbol=${encodeURIComponent(symbol)}`, this.fetchImpl);
    if (!r.ok) return r;
    const v = parseMeStat(r.value);
    return v === null ? { ok: false, status: r.status, error: "unexpected shape" } : { ok: true, status: r.status, value: v };
  }
}

// ---- OpenSea v2: the EVM directory and stats, with the key read at call time -------------------------------------------------------
export const OPENSEA_BASE = "https://api.opensea.io/api/v2";
export type OsCollection = { name: string | null; description: string | null; image: string | null; items: number | null; contracts: { address: string; chain: string }[];
  links: { website: string | null; x: string | null; discord: string | null; marketplace: string | null }; verified: boolean };
export type OsStats = { floorWei: bigint | null; floorSymbol: string | null; owners: number | null; volume24hEth: number | null; volume7dEth: number | null; sales24h: number | null };
const ethWei = (v: unknown): bigint | null => { const n = num(v); if (n === null || n === 0) return null; return BigInt(Math.round(n * 1e9)) * 10n ** 9n; };
export function parseOsCollection(j: unknown): OsCollection | null {
  const x = j as Record<string, unknown> | null;
  if (!x || typeof x !== "object" || typeof x.collection !== "string") return null;
  const contracts = Array.isArray(x.contracts) ? (x.contracts as Record<string, unknown>[]).flatMap((c) => (typeof c.address === "string" && /^0x[0-9a-fA-F]{40}$/.test(c.address) && typeof c.chain === "string" ? [{ address: c.address.toLowerCase(), chain: c.chain }] : [])) : [];
  const tw = str(x.twitter_username);
  return { name: str(x.name), description: str(x.description), image: url(x.image_url), items: int(x.total_supply), contracts,
    links: { website: url(x.project_url), x: tw ? `https://x.com/${tw.replace(/^@/, "")}` : null, discord: url(x.discord_url), marketplace: url(x.opensea_url) }, verified: x.safelist_status === "verified" };
}
/** `/stats`: the floor and owners under `total`, the windows under `intervals`. A floor in anything but ETH is not a floor in this unit. */
export function parseOsStats(j: unknown): OsStats | null {
  const x = j as { total?: Record<string, unknown>; intervals?: Record<string, unknown>[] } | null;
  if (!x || typeof x !== "object" || !x.total || typeof x.total !== "object") return null;
  const sym = str(x.total.floor_price_symbol);
  const eth = sym === null || sym.toUpperCase() === "ETH";
  const win = (k: string) => (Array.isArray(x.intervals) ? x.intervals.find((i) => i.interval === k) : undefined);
  const vol = (k: string) => { const w = win(k); return w && (str(w.volume_symbol) ?? "ETH").toUpperCase() === "ETH" ? num(w.volume) : null; };
  return { floorWei: eth ? ethWei(x.total.floor_price) : null, floorSymbol: sym, owners: int(x.total.num_owners), volume24hEth: vol("one_day"), volume7dEth: vol("seven_day"), sales24h: int(win("one_day")?.sales) };
}
/** `collection/<slug>/nfts` (a key is required for this one): a page of the collection's items with their pictures. */
export type OsItem = { id: string; name: string | null; image: string | null; url: string | null };
export function parseOsItems(j: unknown): OsItem[] | null {
  const rows = (j as { nfts?: unknown })?.nfts;
  if (!Array.isArray(rows)) return null;
  const out: OsItem[] = [];
  for (const r of rows as Record<string, unknown>[]) {
    const id = str(r.identifier);
    if (!id || !/^\d{1,78}$/.test(id)) continue;
    out.push({ id, name: str(r.name), image: url(r.image_url) ?? url(r.display_image_url), url: url(r.opensea_url) });
  }
  return out;
}
export class OpenSea {
  readonly name = "opensea" as const;
  constructor(private readonly fetchImpl: Fetch, private readonly gate: Gate, private readonly apiKey: () => string | undefined = () => process.env.OPENSEA_API_KEY) {}
  private async get<T>(path: string, parse: (j: unknown) => T | null): Promise<Read<T>> {
    await this.gate();
    const key = this.apiKey();
    // the key is put in the header here and nowhere else; `getJson` never echoes a request into an error
    const r = await getJson(`${OPENSEA_BASE}/${path}`, this.fetchImpl, key ? { "x-api-key": key } : {});
    if (!r.ok) return r;
    const v = parse(r.value);
    return v === null ? { ok: false, status: r.status, error: "unexpected shape" } : { ok: true, status: r.status, value: v };
  }
  collection(slug: string) { if (!SLUG.test(slug)) throw new Error("invalid slug"); return this.get(`collections/${slug}`, parseOsCollection); }
  stats(slug: string) { if (!SLUG.test(slug)) throw new Error("invalid slug"); return this.get(`collections/${slug}/stats`, parseOsStats); }
  items(slug: string, limit = 12) { if (!SLUG.test(slug)) throw new Error("invalid slug"); return this.get(`collection/${slug}/nfts?limit=${limit}`, parseOsItems); }
}

// ---- the contract itself: tokenURI through a public RPC, and the metadata's picture (keyless) ---------------------------------------
/** Keyless providers with deep state (docs/v4/eth-nft-canister.md; wrapped/config.ts EVM_CHAINS), and Robinhood Chain's own RPC. */
export const RPC_OF: Readonly<Record<string, readonly string[]>> = { ethereum: ["https://eth.drpc.org", "https://xrpc.cl/eth"], base: ["https://mainnet.base.org", "https://base.drpc.org"], robinhood: ["https://rpc.mainnet.chain.robinhood.com"] };
/**
 * Public IPFS gateways, tried in order. On 2026-09-23 `ipfs.io`, `dweb.link` and `nftstorage.link` answered 429 "switching to a
 * service worker gateway only" to every path and `gateway.pinata.cloud` 429; Filebase's gateway answered 200 to a burst of a dozen,
 * and 4everland answered (through a redirect to its subdomain form). The first is the one a picture is named on.
 */
export const IPFS_GATEWAYS = ["https://ipfs.filebase.io/ipfs/", "https://4everland.io/ipfs/"];
/** Gateways that no longer serve a path (above): a URL a source names on one of them is renamed onto the first working one. */
const DEAD_GATEWAY = /^https:\/\/(?:(?:ipfs\.io|gateway\.ipfs\.io|dweb\.link|w3s\.link|nftstorage\.link|cloudflare-ipfs\.com|gateway\.pinata\.cloud)\/ipfs\/(.+)|([a-z0-9]{46,})\.ipfs\.(?:nftstorage\.link|dweb\.link|w3s\.link)\/?(.*))$/;
export const gatewayed = (u: string): string => {
  const m = u.match(DEAD_GATEWAY);
  return m ? IPFS_GATEWAYS[0] + (m[1] ?? `${m[2]}${m[3] ? `/${m[3]}` : ""}`) : u;
};
/** 1.5 MB: a blue chip's own picture is often larger than 600 kB (an Azuki is 637 kB, a Moonbird 782 kB, the OMB logo 1.4 MB); the cache's byte bound still holds the total. */
export const IMAGE_MAX_BYTES = 1536 * 1024;
/** A picture the catalogue may name: an https URL, an ipfs:// one (rewritten to a gateway), or the picture itself as a base64 data: URI (an on-chain SVG, as Robinhood Chain's collections carry). */
export const DATA_IMAGE = /^data:image\/[a-z0-9.+-]+;base64,[A-Za-z0-9+/=]+$/;
export const isPictureUrl = (u: string): boolean => /^(https:\/\/|ipfs:\/\/)/.test(u) || (DATA_IMAGE.test(u) && u.length <= IMAGE_MAX_BYTES * 4 / 3 + 64);
export const ipfsToHttp = (u: string, gateway = IPFS_GATEWAYS[0]): string => (u.startsWith("ipfs://") ? gateway + u.slice(7).replace(/^ipfs\//, "") : gatewayed(u));
const ERC721_URI = parseAbi(["function tokenURI(uint256) view returns (string)", "function tokenByIndex(uint256) view returns (uint256)"]);
export type UriItem = { id: string; name: string | null; image: string | null; url: string | null };
/** A token's metadata from its URI: a data: URI is decoded here; an ipfs:// one is asked of the gateways in turn; https is fetched. Null when none answers with JSON. */
export async function metadataOf(uri: string, fetchImpl: Fetch): Promise<Record<string, unknown> | null> {
  try {
    if (uri.startsWith("data:application/json;base64,")) return JSON.parse(Buffer.from(uri.slice(29), "base64").toString("utf8")) as Record<string, unknown>;
    if (uri.startsWith("data:application/json,")) return JSON.parse(decodeURIComponent(uri.slice(22))) as Record<string, unknown>;
  } catch { return null; }
  const urls = uri.startsWith("ipfs://") ? IPFS_GATEWAYS.map((g) => ipfsToHttp(uri, g)) : /^https:\/\//.test(uri) ? [gatewayed(uri)] : [];
  for (const url of urls) {
    const r = await getJson(url, fetchImpl);
    if (r.ok && r.value && typeof r.value === "object") return r.value as Record<string, unknown>;
    if (!r.ok && r.status !== 429 && r.status !== null) return null;   // a gateway that says no is not asked again under another name; a limit or a silence is
  }
  return null;
}
/**
 * A token metadata's picture as a URL the catalogue can hold: `image` (or `image_url`) when it is https, ipfs:// (named on the first
 * gateway) or a base64 data: picture; a plain-text SVG data: URI, or raw SVG under `image_data`, is re-encoded as base64. Else null.
 */
export function pictureOf(m: Record<string, unknown>): string | null {
  const svg64 = (svg: string) => `data:image/svg+xml;base64,${Buffer.from(svg, "utf8").toString("base64")}`;
  const img = typeof m.image === "string" ? m.image.trim() : typeof m.image_url === "string" ? m.image_url.trim() : "";
  if (img) {
    const plain = img.match(/^data:image\/svg\+xml(?:;charset=[a-z0-9-]+|;utf8)?,(.*)$/is);
    if (plain) { let svg: string; try { svg = decodeURIComponent(plain[1]); } catch { svg = plain[1]; } const u = svg64(svg); return isPictureUrl(u) ? u : null; }
    return isPictureUrl(img) ? ipfsToHttp(img) : null;
  }
  if (typeof m.image_data === "string" && /^\s*<svg[\s>]/i.test(m.image_data)) { const u = svg64(m.image_data); return isPictureUrl(u) ? u : null; }
  return null;
}
export class TokenUri {
  readonly name = "tokenuri" as const;
  private lastRpc: Read<Hex> | null = null;
  constructor(private readonly fetchImpl: Fetch, private readonly gate: Gate, private readonly rpcOf: Readonly<Record<string, readonly string[]>> = RPC_OF) {}
  private async call(chain: string, to: string, data: Hex): Promise<Read<Hex>> {
    const urls = this.rpcOf[chain] ?? [];
    if (!urls.length) return { ok: false, status: null, error: `no public RPC for ${chain}` };
    let last: Read<Hex> = { ok: false, status: null, error: "no answer" };
    for (const url of urls) {
      await this.gate();
      let r: Response;
      try { r = await this.fetchImpl(url, { method: "POST", headers: { "content-type": "application/json", accept: "application/json", "user-agent": UA }, body: JSON.stringify({ jsonrpc: "2.0", id: 1, method: "eth_call", params: [{ to, data }, "latest"] }), signal: AbortSignal.timeout(15_000) }); }
      catch (err) { last = { ok: false, status: null, error: `no answer: ${String((err as Error)?.name ?? "error")}` }; continue; }
      if (!r.ok) { last = { ok: false, status: r.status, error: `HTTP ${r.status}` }; continue; }
      let j: { result?: unknown; error?: { message?: string } };
      try { j = (await jsonCapped(r)) as typeof j; } catch (err) { last = { ok: false, status: r.status, error: err instanceof TooLarge ? "too large" : "not JSON" }; continue; }
      if (typeof j.result === "string" && /^0x([0-9a-fA-F]{2})*$/.test(j.result)) return { ok: true, status: r.status, value: j.result as Hex };
      return { ok: false, status: r.status, error: j.error?.message ? "the call reverted" : "unexpected shape" };
    }
    return last;
  }
  private async uri(chain: string, contract: string, id: bigint): Promise<string | null> {
    const r = await this.call(chain, contract, encodeFunctionData({ abi: ERC721_URI, functionName: "tokenURI", args: [id] }));
    this.lastRpc = r;
    if (!r.ok || r.value === "0x") return null;
    try { return decodeFunctionResult({ abi: ERC721_URI, functionName: "tokenURI", data: r.value }); } catch { return null; }
  }
  /**
   * The first `limit` tokens' pictures: from the first of ids 1, 0, 2 that answers a tokenURI (a burned #1 is common: Chain
   * Mancers' reverts, #2 answers), walking up and skipping ids that revert, at most 2·limit asked; else by `tokenByIndex`.
   * A collection whose contract answers none is `ok: false`.
   */
  async items(chain: string, contract: string, nftUrl: (id: string) => string | null, limit = 8): Promise<Read<UriItem[]>> {
    if (!/^0x[0-9a-fA-F]{40}$/.test(contract)) throw new Error("invalid contract");
    const uris = new Map<bigint, string>();
    let start: bigint | null = null;
    for (const id of [1n, 0n, 2n]) { const u = await this.uri(chain, contract, id); if (u !== null) { uris.set(id, u); start = id; break; } }
    if (start !== null) {
      for (let id = start + 1n; uris.size < limit && id < start + BigInt(2 * limit); id++) { const u = await this.uri(chain, contract, id); if (u !== null) uris.set(id, u); }
    } else {
      const ids: bigint[] = [];
      for (let i = 0n; i < BigInt(limit); i++) {
        const r = await this.call(chain, contract, encodeFunctionData({ abi: ERC721_URI, functionName: "tokenByIndex", args: [i] }));
        if (!r.ok || r.value === "0x") break;
        try { ids.push(decodeFunctionResult({ abi: ERC721_URI, functionName: "tokenByIndex", data: r.value })); } catch { break; }
      }
      // the RPC's own failure is the reason when it is one; a contract that simply has no tokenURI is the other
      if (!ids.length) return this.lastRpc && !this.lastRpc.ok && this.lastRpc.error !== "the call reverted" ? this.lastRpc : { ok: false, status: null, error: "the contract answers no tokenURI" };
      for (const id of ids) { const u = await this.uri(chain, contract, id); if (u !== null) uris.set(id, u); }
    }
    const out: UriItem[] = [];
    for (const [id, uri] of uris) {
      const m = await metadataOf(uri, this.fetchImpl);
      out.push({ id: String(id), name: typeof m?.name === "string" && m.name.trim() ? m.name.trim() : null, image: m ? pictureOf(m) : null, url: nftUrl(String(id)) });
    }
    return { ok: true, status: 200, value: out };
  }
}

// ---- pictures: fetched by the indexer, checked, and served from its own host ----------------------------------------------------------
/**
 * A picture's URL is always one the catalogue itself holds — the seed's, a marketplace directory's, a token's metadata's — and
 * never one a visitor named (the route takes a slug and an index). So any https host may be asked, except our own network:
 * loopback, link-local, private, CGNAT and ULA ranges and bare hostnames are refused before any request (E-12: `outbound.ts`, which
 * in production also resolves the name and refuses a private answer at connect time, and re-checks every redirect).
 */
export const isPublicHost = (host: string): boolean => {
  const h = host.toLowerCase();
  const literal = h.match(/^\[([0-9a-f:.]+)\](?::\d+)?$/) ?? h.match(/^(\d+\.\d+\.\d+\.\d+)(?::\d+)?$/);
  if (literal) return isPublicIp(literal[1]);
  return hostRefusal(`https://${h}/`) === null;
};
export type Picture = { contentType: string; body: Uint8Array };
/** PNG, JPEG, GIF or WebP by their first bytes; null for anything else (never SVG or HTML, which a type must name). */
export function sniffRaster(b: Uint8Array): string | null {
  const at = (i: number, ...xs: number[]) => xs.every((x, k) => b[i + k] === x);
  if (at(0, 0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a)) return "image/png";
  if (at(0, 0xff, 0xd8, 0xff)) return "image/jpeg";
  if (at(0, 0x47, 0x49, 0x46, 0x38)) return "image/gif";
  if (at(0, 0x52, 0x49, 0x46, 0x46) && at(8, 0x57, 0x45, 0x42, 0x50)) return "image/webp";
  return null;
}
/** One GET for a picture: only an `image/*` answer (or an untyped one whose bytes are a raster picture) under the size cap is a picture; anything else is `ok: false` with the status. */
export async function fetchPicture(url: string, fetchImpl: Fetch): Promise<Read<Picture>> {
  // a picture that is its own URL (an on-chain SVG) is decoded here, and no request is made
  if (url.startsWith("data:image/")) {
    if (!DATA_IMAGE.test(url)) return { ok: false, status: null, error: "not a picture" };
    const body = new Uint8Array(Buffer.from(url.slice(url.indexOf(",") + 1), "base64"));
    return body.byteLength && body.byteLength <= IMAGE_MAX_BYTES ? { ok: true, status: 200, value: { contentType: url.slice(5, url.indexOf(";")).toLowerCase(), body } } : { ok: false, status: null, error: body.byteLength ? "too large" : "empty" };
  }
  let host: string;
  try { host = new URL(url).host; } catch { return { ok: false, status: null, error: "not a URL" }; }
  if (!/^https:/.test(url) || !isPublicHost(host)) return { ok: false, status: null, error: "not a picture host" };
  let r: Response;
  // 30 s: a public IPFS gateway can take 20 s to find a picture it has not served before (Filebase, 2026-09-23)
  try { r = await fetchImpl(url, { method: "GET", headers: { accept: "image/*", "user-agent": UA }, redirect: "manual", signal: AbortSignal.timeout(30_000) }); }
  catch (err) { return { ok: false, status: null, error: `no answer: ${String((err as Error)?.name ?? "error")}` }; }
  if (!r.ok) { await r.body?.cancel().catch(() => undefined); return { ok: false, status: r.status, error: `HTTP ${r.status}` }; }
  const said = (r.headers.get("content-type") ?? "").split(";")[0].trim().toLowerCase();
  // a bucket that names no type (CyberKongz' answer `binary/octet-stream`) is a picture only when its bytes are one of four raster formats
  const untyped = said === "application/octet-stream" || said === "binary/octet-stream";
  if (!untyped && !/^image\/[a-z0-9.+-]+$/.test(said)) return { ok: false, status: r.status, error: `not a picture: ${said || "no type"}` };
  let body: Uint8Array;
  try { body = await readCapped(r, IMAGE_MAX_BYTES); }
  catch (err) { return { ok: false, status: r.status, error: err instanceof TooLarge ? "too large" : `no answer: ${String((err as Error)?.name ?? "error")}` }; }
  if (body.byteLength === 0) return { ok: false, status: r.status, error: "empty" };
  const type = untyped ? sniffRaster(body) : said;
  if (!type) return { ok: false, status: r.status, error: `not a picture: ${said}` };
  return { ok: true, status: r.status, value: { contentType: type, body } };
}

// ---- the rate: sats per ETH, keyless, one reference stated as such (docs/v5/floor-feed.md §2) -------------------------------------
export function parseCoinbaseSatsPerEth(j: unknown): bigint | null {
  const v = String((j as { data?: { rates?: { BTC?: unknown } } })?.data?.rates?.BTC ?? "");
  if (!/^\d+(\.\d{1,18})?$/.test(v)) return null;
  const [i, f = ""] = v.split(".");
  const sats = (BigInt(i) * 10n ** 18n + BigInt((f + "0".repeat(18)).slice(0, 18))) / 10n ** 10n;
  return sats > 0n ? sats : null;
}
export async function coinbaseSatsPerEth(fetchImpl: Fetch): Promise<Read<bigint>> {
  const r = await getJson("https://api.coinbase.com/v2/exchange-rates?currency=ETH", fetchImpl);
  if (!r.ok) return r;
  const v = parseCoinbaseSatsPerEth(r.value);
  return v === null ? { ok: false, status: r.status, error: "unexpected shape" } : { ok: true, status: r.status, value: v };
}
