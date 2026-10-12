import assert from "node:assert/strict";
import type { AddressInfo } from "node:net";
import { test } from "node:test";
import { serve } from "../src/api.js";
import { openDb } from "../src/db.js";
import { Live } from "../src/live.js";
import { Views } from "../src/views.js";
import { parseConfig } from "../src/wrapped/config.js";
import { WrappedService } from "../src/wrapped/service.js";
import { CatalogService, loadSeed, parseSeed, type Seed } from "../src/rook/catalog.js";
import { ALLOWANCE_BPS, maxFloorWei, ruleOf, STAGES, stageFor, statusOf, weiOfEth } from "../src/rook/catalogRule.js";
import { fetchPicture, gatewayed, ipfsToHttp, isPublicHost, metadataOf, owIcon, parseCoinbaseSatsPerEth, pictureOf, sniffRaster, TokenUri, parseMeStat, parseOsCollection, parseOsItems, parseOsStats, parseOwBook, parseOwInfo, parseOwItems, parseOwSales, UA } from "../src/rook/catalogSources.js";
import type { Catalog, CatalogOne } from "../src/rook/catalogTypes.js";
import { rookRoutes } from "../src/rook/routes.js";
import { RookService } from "../src/rook/service.js";
import { buildWorld, NOW, worldConfig } from "./wrappedWorld.js";

/** docs/v6/collections-catalog.md: the rule's numbers, the sources' shapes, the merge, the failure modes, the pictures, the endpoints. */
const E = 10n ** 18n;
const FAKE_KEY = "FAKE-OPENSEA-KEY-0123456789";

// ---- the rule -------------------------------------------------------------------------------------------------------------------------
test("the listing rule carries G7's numbers: stage 0 admits a floor up to 8.333 ETH, stage 1 up to 33.333, stage 2 up to 133.333, stage 3 up to 533.333", () => {
  assert.equal(STAGES[0].perCollectionCapWei, 10n * E); assert.equal(STAGES[0].perItemMaxWei, 10n * E); assert.equal(STAGES[0].perCollectionItems, 3);
  assert.equal(STAGES[3].perCollectionCapWei, 640n * E); assert.equal(STAGES[3].perCollectionItems, 192);
  assert.equal(maxFloorWei(STAGES[0]), (10n * E * 10_000n) / 12_000n);
  assert.equal(maxFloorWei(STAGES[1]), (40n * E * 10_000n) / 12_000n);
  const r = ruleOf();
  assert.equal(r.stage, 0); assert.equal(r.allowanceBps, ALLOWANCE_BPS); assert.equal(r.maxFloorWei, String((10n * E * 10_000n) / 12_000n));
  assert.match(r.text, /8\.333 ETH/); assert.match(r.text, /3 items a collection/); assert.match(r.doc, /launch-gates\.md G7/);
  assert.equal(stageFor(weiOfEth(6.847)), 0); assert.equal(stageFor(weiOfEth(34)), 2); assert.equal(stageFor(weiOfEth(30)), 1); assert.equal(stageFor(weiOfEth(600)), null);
});

test("status: live wins; Base, Solana and non-721s are unsupported with the reason; Robinhood Chain's own collections are held; no floor is `later` with no stage; the floor decides the rest", () => {
  const base = { chain: "ethereum" as const, chainName: "Ethereum", standard: "erc721" as const, floorStale: false, live: null };
  assert.equal(statusOf({ ...base, chain: "base", chainName: "Base", floorWei: weiOfEth(0.01), live: { slug: "x", coins: 0 } }).kind, "live", "listed is listed, whatever the chain");
  const b = statusOf({ ...base, chain: "base", chainName: "Base", floorWei: weiOfEth(0.01) }); assert.equal(b.kind, "unsupported"); assert.match(b.why, /no native bridge route/);
  const sol = statusOf({ ...base, chain: "other", chainName: "Solana", standard: "other", floorWei: null }); assert.equal(sol.kind, "unsupported"); assert.match(sol.why, /^Solana is not a chain/);
  const rh = statusOf({ ...base, chain: "robinhood", chainName: "Robinhood Chain", floorWei: weiOfEth(3.16) }); assert.equal(rh.kind, "listable", "a 4663 collection needs no bridge at all");
  const e = statusOf({ ...base, standard: "erc1155", floorWei: weiOfEth(0.01) }); assert.equal(e.kind, "unsupported"); assert.match(e.why, /ERC-1155/);
  const n = statusOf({ ...base, floorWei: null }); assert.equal(n.kind, "later"); assert.equal((n as { stage: number | null }).stage, null); assert.match(n.why, /No floor is known/);
  const bayc = statusOf({ ...base, floorWei: weiOfEth(6.847) }); assert.equal(bayc.kind, "listable"); assert.equal((bayc as { slotsNow: number }).slotsNow, 1, "10 ETH holds one ape at 6.847 × 1.2");
  const mayc = statusOf({ ...base, floorWei: weiOfEth(0.91) }); assert.equal(mayc.kind, "listable"); assert.equal((mayc as { slotsNow: number }).slotsNow, 3, "capped at the stage's 3 slots");
  const punks = statusOf({ ...base, standard: "cryptopunks", floorWei: weiOfEth(34) }); assert.equal(punks.kind, "later"); assert.equal((punks as { stage: number }).stage, 2); assert.match(punks.why, /fits from stage 2/);
  const glyphs = statusOf({ ...base, floorWei: weiOfEth(600) }); assert.equal(glyphs.kind, "later"); assert.equal((glyphs as { stage: number | null }).stage, null); assert.match(glyphs.why, /above even stage 3/);
  const edge = statusOf({ ...base, floorWei: maxFloorWei(STAGES[0]) }); assert.equal(edge.kind, "listable", "the limit itself is in");
  const over = statusOf({ ...base, floorWei: maxFloorWei(STAGES[0]) + 1n }); assert.equal(over.kind, "later");
  assert.match(statusOf({ ...base, floorWei: weiOfEth(1), floorStale: true }).why, /not from a fresh read/);
});

// ---- the sources' shapes, as they answered on 2026-09-23 --------------------------------------------------------------------------------
const OW_INFO = { id: "0536", name: "Bitcoin Frogs", icon: `https://turbo.ordinalswallet.com/inscription/preview/${"d".repeat(64)}i0`, description: "10,000 timeless frog collectibles stored on Bitcoin blockchain.", slug: "bitcoin-frogs", active: true, highest_inscription_num: 412389, lowest_inscription_num: 381224, total_supply: 10000, socials: { discord: "https://discord.gg/x", twitter: "https://twitter.com/BitcoinFrogs", website: "https://www.bitcoinfrogs.com" }, verified: true };
const OW_BOOK = [{ inscription_id: "a".repeat(64) + "i0", outpoint: "b".repeat(64) + ":0", seller_address: "bc1p" + "q".repeat(40), satoshi_price: 500000, created: "2026-09-22 10:00:00+00", bought_at: null, purchase_txid: null },
  { inscription_id: "c".repeat(64) + "i0", outpoint: "d".repeat(64) + ":1", seller_address: "bc1p" + "r".repeat(40), satoshi_price: 433069, created: "2026-09-22 11:00:00+00", bought_at: null, purchase_txid: null },
  { inscription_id: "e".repeat(64) + "i0", outpoint: "f".repeat(64) + ":0", seller_address: "bc1p" + "s".repeat(40), satoshi_price: 100, created: "2026-09-22 12:00:00+00", bought_at: "2026-09-22 13:00:00+00", purchase_txid: "1".repeat(64) }];
const owSales = (now: number) => ({ sales: [{ txid: "1".repeat(64), block_timestamp: now - 3_600, inscription_id: "a".repeat(64) + "i0", price_sats: 450000, inscriptions_in_tx: 1 }, { txid: "2".repeat(64), block_timestamp: now - 2 * 86_400, price_sats: 440000 }, { txid: "3".repeat(64), block_timestamp: now - 8 * 86_400, price_sats: 400000 }] });
const OW_ITEMS = [{ id: "1".repeat(64) + "i0", num: 412199, content_type: "image/webp", meta: { name: "Bitcoin Frog #2866" } }, { id: "2".repeat(64) + "i0", num: 412200, content_type: "text/html", meta: { name: "not a picture" } }, { id: "3".repeat(64) + "i0", num: 412201, content_type: "image/png", meta: {} }, { id: "nope" }];
const ME_STAT = { totalVolume: "151762001352", owners: "5143", supply: "10000", floorPrice: "480000", totalListed: "1010", symbol: "bitcoin-frogs" };
const OS_COL = { collection: "boredapeyachtclub", name: "Bored Ape Yacht Club", description: "The Bored Ape Yacht Club is a collection of 10,000 unique Bored Ape NFTs.", image_url: "https://i2c.seadn.io/collection/boredapeyachtclub/image/x.png", safelist_status: "verified", opensea_url: "https://opensea.io/collection/boredapeyachtclub", project_url: "http://www.boredapeyachtclub.com/", discord_url: "https://discord.gg/3P5K3dzgdB", twitter_username: "BoredApeYC", contracts: [{ address: "0xBC4CA0EdA7647A8aB7C2061c2E118A18a936f13D", chain: "ethereum" }], total_supply: 9997 };
const OS_STATS = { total: { volume: 1582194.59, volume_symbol: "ETH", sales: 57581, num_owners: 5597, floor_price: 6.847, floor_price_symbol: "ETH" }, intervals: [{ interval: "one_day", volume: 39.537, volume_symbol: "ETH", sales: 5 }, { interval: "seven_day", volume: 524.6, volume_symbol: "ETH", sales: 74 }] };
const OS_NFTS = { nfts: [{ identifier: "1", name: "Bored Ape #1", image_url: "https://i.seadn.io/x/1.png", opensea_url: "https://opensea.io/assets/ethereum/0xbc4c/1" }, { identifier: "2", display_image_url: "https://i.seadn.io/x/2.png" }, { identifier: "x" }] };

test("each source's answer is parsed strictly: a floor is the cheapest open listing, a volume only when the list reaches back that far, a foreign-currency floor is no floor, an item without an id is no item", () => {
  const info = parseOwInfo(OW_INFO)!;
  assert.equal(info.name, "Bitcoin Frogs"); assert.equal(info.items, 10000); assert.equal(info.image, `https://ordinals.com/content/${"d".repeat(64)}i0`, "the icon's inscription at the content gateway, which a browser can load"); assert.equal(info.links.x, "https://twitter.com/BitcoinFrogs"); assert.equal(info.first, 381224);
  assert.equal(parseOwInfo({ name: "no slug" }), null);
  const book = parseOwBook(OW_BOOK)!;
  assert.equal(book.floorSats, 433069n, "the bought one does not count, whatever its price"); assert.equal(book.listed, 2);
  assert.equal(parseOwBook([])!.floorSats, null, "nothing listed is no floor, not zero"); assert.equal(parseOwBook({ nope: 1 }), null);
  const now = 1_790_000_000, s = parseOwSales(owSales(now), now)!;
  assert.equal(s.sats24h, 450000n); assert.equal(s.sats7d, 890000n); assert.equal(s.sales, 3);
  const short = parseOwSales({ sales: owSales(now).sales.slice(0, 1) }, now)!;
  assert.equal(short.sats24h, null, "a list that does not reach a day back cannot say what the day traded"); assert.equal(short.sats7d, null);
  const items = parseOwItems(OW_ITEMS)!;
  assert.deepEqual(items.map((i) => [i.id.slice(0, 2), i.name, i.contentType]), [["11", "Bitcoin Frog #2866", "image/webp"], ["22", "not a picture", "text/html"], ["33", null, "image/png"]]);
  const me = parseMeStat(ME_STAT)!;
  assert.equal(me.floorSats, 480000n); assert.equal(me.owners, 5143); assert.equal(me.items, 10000); assert.equal(me.listed, 1010);
  const col = parseOsCollection(OS_COL)!;
  assert.deepEqual(col.contracts, [{ address: "0xbc4ca0eda7647a8ab7c2061c2e118a18a936f13d", chain: "ethereum" }]); assert.equal(col.links.x, "https://x.com/BoredApeYC"); assert.equal(col.verified, true);
  const st = parseOsStats(OS_STATS)!;
  assert.equal(st.floorWei, 6_847_000_000_000_000_000n); assert.equal(st.owners, 5597); assert.equal(st.volume24hEth, 39.537); assert.equal(st.volume7dEth, 524.6); assert.equal(st.sales24h, 5);
  assert.equal(parseOsStats({ total: { floor_price: 100, floor_price_symbol: "USDC" } })!.floorWei, null, "a floor in another unit is not a floor in ETH");
  assert.equal(parseOsStats({ total: { floor_price: 0, floor_price_symbol: "ETH" } })!.floorWei, null, "a zero floor is no floor");
  assert.deepEqual(parseOsItems(OS_NFTS)!.map((i) => [i.id, i.name, i.image]), [["1", "Bored Ape #1", "https://i.seadn.io/x/1.png"], ["2", null, "https://i.seadn.io/x/2.png"]]);
  assert.equal(parseCoinbaseSatsPerEth({ data: { rates: { BTC: "0.031683" } } }), 3_168_300n);
  assert.equal(parseCoinbaseSatsPerEth({ data: { rates: { BTC: "abc" } } }), null);
});

// ---- a fake network -------------------------------------------------------------------------------------------------------------------
type Hit = { url: string; headers: Record<string, string> };
const PNG = new Uint8Array([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 1, 2, 3, 4]);
type Answer = { status: number; body?: unknown; bytes?: Uint8Array; type?: string } | "throw";
function fakeFetch(answer: (url: string) => Answer, hits: Hit[] = []): typeof fetch & { hits: Hit[] } {
  const f = (async (input: string | URL | Request, init?: RequestInit) => {
    const url = String(input);
    hits.push({ url, headers: Object.fromEntries(Object.entries((init?.headers ?? {}) as Record<string, string>).map(([k, v]) => [k.toLowerCase(), v])) });
    const a = answer(url);
    if (a === "throw") throw new TypeError("fetch failed");
    if (a.bytes) return new Response(Buffer.from(a.bytes), { status: a.status, headers: { "content-type": a.type ?? "image/png", "content-length": String(a.bytes.byteLength) } });
    return new Response(a.body === undefined ? "not json" : JSON.stringify(a.body), { status: a.status, headers: { "content-type": a.type ?? "application/json" } });
  }) as typeof fetch & { hits: Hit[] };
  f.hits = hits;
  return f;
}
const goodNet = (now: number) => fakeFetch((u) => {
  if (u.includes("coinbase")) return { status: 200, body: { data: { rates: { BTC: "0.031683" } } } };
  if (u.includes("ordinalswallet.com/collection/bitcoin-frogs/escrows")) return { status: 200, body: OW_BOOK };
  if (u.includes("ordinalswallet.com/collection/bitcoin-frogs/sales")) return { status: 200, body: owSales(now) };
  if (u.includes("ordinalswallet.com/collection/bitcoin-frogs/inscriptions")) return { status: 200, body: OW_ITEMS };
  if (u.includes("ordinalswallet.com/collection/bitcoin-frogs")) return { status: 200, body: OW_INFO };
  if (u.includes("ordinalswallet.com/collection/")) { const slug = /collection\/([^/?]+)/.exec(u)![1]; return u.includes("/escrows") ? { status: 200, body: [] } : u.includes("/sales") ? { status: 200, body: { sales: [] } } : u.includes("/inscriptions") ? { status: 200, body: [] } : { status: 200, body: { ...OW_INFO, slug, name: `Named ${slug}` } }; }
  if (u.includes("magiceden") && u.includes("bitcoin-frogs")) return { status: 200, body: ME_STAT };
  if (u.includes("magiceden")) return { status: 503 };
  if (u.includes("opensea.io/api/v2/collections/boredapeyachtclub/stats")) return { status: 200, body: OS_STATS };
  if (u.includes("opensea.io/api/v2/collections/boredapeyachtclub")) return { status: 200, body: OS_COL };
  if (u.includes("opensea.io/api/v2/collection/boredapeyachtclub/nfts")) return { status: 200, body: OS_NFTS };
  if (u.includes("opensea.io/api/v2/collection/") && u.includes("/nfts")) return { status: 401, body: { errors: ["Missing an API Key"] } };
  if (u.includes("opensea.io/api/v2/collections/") && u.endsWith("/stats")) return { status: 401, body: { errors: ["Unauthorized"] } };
  if (u.includes("opensea.io/api/v2/collections/")) { const slug = /collections\/([^/]+)/.exec(u)![1]; return { status: 200, body: { ...OS_COL, collection: slug, name: `OS ${slug}`, contracts: [] } }; }
  if (u.startsWith("https://i2c.seadn.io/") || u.startsWith("https://i.seadn.io/x/1.png")) return { status: 200, bytes: PNG, type: "image/png" };
  if (u.startsWith("https://ordinals.com/content/" + "1".repeat(64))) return { status: 200, bytes: PNG, type: "image/webp" };
  if (u.startsWith("https://ordinals.com/content/")) return { status: 200, body: { html: true }, type: "text/html" };
  return { status: 404 };
});
const FEW = ["bitcoin-frogs", "boredapeyachtclub", "cryptopunks", "onchain-gaias", "taproot-wizards", "stonkbrokers-434284142", "mad-lads"];
const fewSeed = (): Seed => parseSeed({ ...loadSeed(), entries: loadSeed().entries.filter((e) => FEW.includes(e.slug)) });
function world() {
  const w = buildWorld(), wrappedConfig = parseConfig(worldConfig());
  const wrapped = new WrappedService({ config: wrappedConfig, rpc: w.rpc, fetch: w.fetch, now: () => NOW });
  return { w, wrapped, wrappedConfig };
}

test("the committed seed parses: five chains, Nosh's twelve on Robinhood Chain, the old site's fifty on Bitcoin with item samples, every entry a marketplace link, every EVM entry a lower-case contract, every record dated", () => {
  const s = loadSeed();
  assert.ok(s.entries.length >= 100, `${s.entries.length} entries`);
  const by = (c: string) => s.entries.filter((e) => e.chain === c);
  assert.equal(by("bitcoin").length, 50); assert.equal(by("robinhood").length, 12); assert.ok(by("ethereum").length >= 30); assert.ok(by("base").length >= 4); assert.ok(by("other").length >= 15);
  assert.ok(by("bitcoin").filter((e) => e.sample?.length).length >= 40, "most Bitcoin nests carry the old catalogue's inscription ids");
  assert.ok(by("bitcoin").every((e) => (e.sample ?? []).every((i) => /^[0-9a-f]{64}i\d+$/.test(i.id) && i.image?.startsWith("https://ordinals.com/content/"))), "and every sample item is an inscription with its content URL");
  assert.ok(by("robinhood").every((e) => e.chainId === 4663 && /^0x[0-9a-f]{40}$/.test(e.id) && e.sources.opensea && e.links.explorer?.includes("explorer.mainnet.chain.robinhood.com")));
  assert.ok(by("other").every((e) => e.chainName !== "another chain" && ["Solana", "HyperEVM", "Zcash"].includes(e.chainName)));
  for (const e of s.entries) { assert.ok(e.links.marketplace, e.slug); assert.ok(e.chainName); if (["ethereum", "base", "robinhood"].includes(e.chain)) assert.match(e.id, /^0x[0-9a-f]{40}$/); if (e.recorded) assert.ok(e.recorded.at > 1_780_000_000 && e.recorded.at < 1_900_000_000); }
  for (const slug of ["bitcoin-frogs", "nodemonkes", "boredapeyachtclub", "cryptopunks", "stonkbrokers-434284142", "chain-mancers", "opencatz-by-dizcorvus", "theroyalmechanica", "theapehood", "mad-lads"]) assert.ok(s.entries.some((e) => e.slug === slug), slug);
  assert.ok(s.rate && BigInt(s.rate.satsPerEth) > 1_000_000n);
  // pictures: https URLs (each checked to answer an image when the seed was written) or an on-chain picture; never a gateway that no longer serves
  const pics = s.entries.flatMap((e) => [e.image, ...(e.sample ?? []).map((i) => i.image)]).filter((u): u is string => u !== null);
  assert.ok(pics.every((u) => /^https:\/\/[a-z0-9.-]+\//.test(u) || /^data:image\/svg\+xml;base64,[A-Za-z0-9+/=]+$/.test(u)), "every picture is https or a base64 SVG");
  assert.ok(!pics.some((u) => /^https:\/\/(?:ipfs\.io|dweb\.link|[^/]*nftstorage\.link|cdn\.discordapp\.com|turbo\.ordinalswallet\.com\/inscription\/preview)\b/.test(u)), "no dead host");
  assert.ok(s.entries.filter((e) => e.image).length >= 110, "nearly every collection has its logo");
  for (const c of ["ethereum", "base", "robinhood"]) assert.ok(by(c).every((e) => e.image?.startsWith("https://")), `every ${c} collection has its logo`);
  assert.throws(() => parseSeed({ entries: [{ slug: "x", chain: "ethereum", chainName: "Ethereum", standard: "erc721", id: "0xABC", links: {}, sources: {} }] }), /lower-case/);
  assert.throws(() => parseSeed({ entries: [{ slug: "x", chain: "bitcoin", chainName: "Bitcoin", standard: "erc721", id: "x", links: {}, sources: {} }] }), /inscription collection/);
  assert.throws(() => parseSeed({ entries: [{ slug: "x", chain: "other", standard: "other", id: "x", links: {}, sources: {} }] }), /chain name/);
});

test("before any refresh the catalogue is the seed's dated records, flagged stale, with the seed's rate, and live nests found by contract or by name", async () => {
  const { wrapped, wrappedConfig } = world();
  const svc = new CatalogService({ seed: fewSeed(), fetch: null, wrapped, wrappedConfig, now: () => NOW, usdPerEth: () => 4000 });
  const c = await svc.list();
  assert.equal(c.stale, true); assert.equal(c.refreshedAt, null); assert.equal(c.attemptedAt, null); assert.equal(c.usdPerEth, 4000);
  assert.equal(c.rate?.stale, true); assert.equal(c.satsPerEth, 3_168_300);
  const of = (slug: string) => c.entries.find((e) => e.slug === slug)!;
  const frogs = of("bitcoin-frogs"), apes = of("boredapeyachtclub"), punks = of("cryptopunks");
  assert.equal(frogs.rook.kind, "live"); assert.equal((frogs.rook as { slug: string }).slug, "frogs", "the worked world's nest, matched by name: its config carries no marketplace slug");
  assert.ok(frogs.heldByRook !== null && frogs.heldByRook > 0, "what the worked world's custody holds"); assert.equal(frogs.chainName, "Bitcoin"); assert.ok(frogs.sample.length > 0);
  assert.equal(apes.rook.kind, "live"); assert.equal((apes.rook as { slug: string }).slug, "apes", "matched by name too: the worked world's contract is invented");
  assert.equal(punks.rook.kind, "later"); assert.equal((punks.rook as { stage: number }).stage, 2); assert.equal(punks.heldByRook, null);
  assert.equal(punks.floor?.source, "seed"); assert.equal(punks.floor?.stale, true); assert.equal(punks.floor?.unit, "wei"); assert.equal(punks.floor?.eth, 34);
  assert.equal(frogs.floor?.unit, "sats"); assert.ok(frogs.floor!.eth! > 0.1 && frogs.floor!.eth! < 0.2, "sats through the seed's rate");
  assert.equal(of("taproot-wizards").floor, null, "no record, no floor, nothing invented");
  assert.equal(of("onchain-gaias").rook.kind, "unsupported");
  assert.equal(of("stonkbrokers-434284142").rook.kind, "listable"); assert.equal(of("stonkbrokers-434284142").chain, "robinhood");
  assert.equal(of("mad-lads").rook.kind, "unsupported"); assert.match(of("mad-lads").rook.why, /^Solana/);
  assert.ok(c.entries.every((e) => e.curated));
  assert.equal((await svc.one("apes"))?.entry.slug, "boredapeyachtclub", "a nest's Rook slug finds its catalogue entry");
  assert.equal((await svc.one("0xBC4CA0EdA7647A8aB7C2061c2E118A18a936f13D"))?.entry.slug, "boredapeyachtclub", "so does its contract, whatever the case");
  assert.equal(await svc.one("nope"), null);
  assert.equal(await svc.picture("bitcoin-frogs"), null, "no network, no picture");
});

test("a refresh puts the sources' figures on the seed: fresh floors, counts, volumes, item samples, per-entry source rows with HTTP codes, and the User-Agent on every request", async () => {
  const { wrapped, wrappedConfig } = world();
  const net = goodNet(NOW), log: Record<string, unknown>[] = [];
  const svc = new CatalogService({ seed: fewSeed(), fetch: net, wrapped, wrappedConfig, now: () => NOW, sleep: async () => undefined, apiKey: () => FAKE_KEY, log: (o) => log.push(o) });
  const c = await svc.refresh();
  assert.equal(c.attemptedAt, NOW); assert.equal(c.rate?.stale, false); assert.equal(c.satsPerEth, 3_168_300);
  const of = (slug: string) => c.entries.find((e) => e.slug === slug)!;
  const frogs = of("bitcoin-frogs"), apes = of("boredapeyachtclub"), punks = of("cryptopunks");
  assert.equal(frogs.floor?.native, "433069"); assert.equal(frogs.floor?.source, "ordinalswallet"); assert.equal(frogs.floor?.stale, false); assert.equal(frogs.listed, 2); assert.equal(frogs.owners, 5143, "owners from Magic Eden");
  assert.ok(Math.abs(frogs.volume24hEth! - 450000 / 3_168_300) < 1e-9); assert.ok(Math.abs(frogs.volume7dEth! - 890000 / 3_168_300) < 1e-9);
  assert.deepEqual(frogs.sources.map((s) => [s.name, s.status, s.ok]), [["ordinalswallet", 200, true], ["ordinalswallet", 200, true], ["ordinalswallet", 200, true], ["ordinalswallet", 200, true], ["magiceden", 200, true]]);
  assert.deepEqual(frogs.sample.map((i) => [i.id.slice(0, 2), i.name]), [["11", "Bitcoin Frog #2866"], ["33", "#412201"]], "the sample keeps the pictures and drops the HTML inscription");
  assert.equal(frogs.sample[0].image, `https://ordinals.com/content/${"1".repeat(64)}i0`);
  assert.equal(apes.floor?.native, "6847000000000000000"); assert.equal(apes.floor?.eth, 6.847); assert.equal(apes.owners, 5597); assert.equal(apes.volume24hEth, 39.537); assert.equal(apes.image, OS_COL.image_url); assert.equal(apes.links.x, "https://x.com/BoredApeYC");
  assert.deepEqual(apes.sample.map((i) => i.id), ["1", "2"]); assert.equal(apes.rook.kind, "live");
  assert.equal(punks.floor?.source, "seed", "OpenSea's 401 on its stats leaves the seed's dated record, flagged"); assert.equal(punks.floor?.stale, true);
  assert.deepEqual(punks.sources.map((s) => [s.name, s.status, s.ok]), [["opensea", 401, false], ["opensea", 200, true], ["opensea", 401, false]]);
  assert.equal(c.stale, true, "one failed read makes the whole answer stale"); assert.equal(c.refreshedAt, null);
  const os = c.sources.find((s) => s.name === "opensea")!, me = c.sources.find((s) => s.name === "magiceden")!;
  assert.deepEqual(os.statuses, [200, 401]); assert.match(os.note, /cryptopunks stats: HTTP 401/); assert.deepEqual(me.statuses, [200, 503]);
  assert.ok(net.hits.length > 8 && net.hits.every((h) => h.headers["user-agent"] === UA), "a User-Agent on every request");
  assert.ok(net.hits.filter((h) => h.url.includes("opensea.io")).every((h) => h.headers["x-api-key"] === FAKE_KEY), "the key on every OpenSea request");
  assert.ok(net.hits.filter((h) => !h.url.includes("opensea.io")).every((h) => !("x-api-key" in h.headers)), "and on no other host");
  const text = JSON.stringify(log) + JSON.stringify(c);
  assert.ok(!text.includes(FAKE_KEY), "the key is in no log line and no answer");
  assert.equal(log.at(-1)?.event, "catalog-refresh");
  const hits = net.hits.length;
  const again = await svc.refresh();
  assert.equal(again.entries.find((e) => e.slug === "bitcoin-frogs")!.floor?.native, "433069");
  const asked = net.hits.slice(hits).map((h) => h.url);
  assert.ok(asked.some((u) => u.includes("/escrows")) && !asked.some((u) => u.includes("/inscriptions") || u.includes("/nfts")), "the second refresh asks for floors again but keeps the samples and names it already has");
  assert.deepEqual(again.entries.find((e) => e.slug === "bitcoin-frogs")!.sample.map((i) => i.id.slice(0, 2)), ["11", "33"]);
});

test("pictures: fetched from the URL the catalogue holds, only from public https hosts, only when they are pictures, cached and bounded; a miss is null", async () => {
  const net = goodNet(NOW);
  const svc = new CatalogService({ seed: fewSeed(), fetch: net, now: () => NOW, sleep: async () => undefined, apiKey: () => FAKE_KEY, pictureCache: { entries: 2, bytes: 1_000_000 } });
  await svc.refresh();
  const p = await svc.picture("boredapeyachtclub");
  assert.equal(p?.contentType, "image/png"); assert.deepEqual([...p!.body], [...PNG]);
  const hits = net.hits.length;
  assert.equal((await svc.picture("boredapeyachtclub"))?.contentType, "image/png"); assert.equal(net.hits.length, hits, "the second ask is the cache");
  assert.equal((await svc.picture("boredapeyachtclub", 0))?.contentType, "image/png", "a sample item's picture");
  assert.equal(await svc.picture("boredapeyachtclub", 7), null, "no such item");
  assert.equal((await svc.picture("bitcoin-frogs", 0))?.contentType, "image/webp", "an inscription through the ord content gateway");
  assert.equal(await svc.picture("bitcoin-frogs", 1), null, "an inscription that is HTML is not a picture");
  assert.equal(await svc.picture("nope"), null);
  assert.ok(net.hits.filter((h) => h.url.includes("seadn") || h.url.includes("ordinals.com/content")).every((h) => h.headers["user-agent"] === UA && !("x-api-key" in h.headers)), "a picture request carries the User-Agent and never the key");
  for (const u of ["https://127.0.0.1/x.png", "https://10.0.0.5/x.png", "https://192.168.1.2/x.png", "https://169.254.1.1/x.png", "https://localhost/x.png", "https://indexer/x.png", "https://[::1]/x.png"]) assert.equal((await fetchPicture(u, net)).ok, false, `${u} is refused before any request`);
  assert.equal((await fetchPicture("http://i.seadn.io/x.png", net)).ok, false, "and so is plain http");
  assert.ok(!net.hits.some((h) => /127\.0\.0\.1|10\.0\.0\.5|192\.168|169\.254|localhost|\/\/indexer\//.test(h.url)));
});

test("when every source fails, or none was ever reached, the seed is served flagged stale — and nothing is invented", async () => {
  const dead = fakeFetch(() => "throw");
  const svc = new CatalogService({ seed: fewSeed(), fetch: dead, now: () => NOW, sleep: async () => undefined });
  const c = await svc.refresh();
  assert.equal(c.stale, true); assert.equal(c.attemptedAt, NOW); assert.equal(c.refreshedAt, null);
  for (const s of c.sources) { assert.equal(s.answered, 0); assert.deepEqual(s.statuses, []); }
  const punks = c.entries.find((e) => e.slug === "cryptopunks")!;
  assert.equal(punks.floor?.source, "seed"); assert.equal(punks.floor?.stale, true); assert.equal(punks.floor?.eth, 34);
  assert.ok(punks.sources.every((s) => s.status === null && !s.ok));
  assert.equal(c.entries.find((e) => e.slug === "taproot-wizards")!.floor, null);
  assert.ok(c.entries.find((e) => e.slug === "bitcoin-frogs")!.sample.length > 0, "the seed's sample stands when the source does not answer");
  assert.equal(c.rate?.stale, true, "the seed's rate, stale");
  const notJson = fakeFetch(() => ({ status: 200 }));
  const c2 = await new CatalogService({ seed: fewSeed(), fetch: notJson, now: () => NOW, sleep: async () => undefined }).refresh();
  assert.ok(c2.sources.every((s) => s.answered === 0 && /not JSON|unexpected shape/.test(s.note)), "a 200 that is not the shape is not an answer");
});

test("list() answers from the snapshot at once and refreshes behind it, once per TTL; requests to one host are spaced", async () => {
  let t = NOW; const slept: number[] = [];
  const net = goodNet(NOW);
  const svc = new CatalogService({ seed: fewSeed(), fetch: net, now: () => t, ttlS: 600, sleep: async (ms) => { slept.push(ms); t += Math.ceil(ms / 1000); }, apiKey: () => FAKE_KEY, pace: { ordinalswallet: 5_000, magiceden: 5_000, opensea: 5_000 } });
  const first = await svc.list();
  assert.equal(first.attemptedAt, null, "served before the network answered");
  await new Promise((r) => setTimeout(r, 30));
  for (let i = 0; i < 50 && (await svc.list()).attemptedAt === null; i++) await new Promise((r) => setTimeout(r, 20));
  const c = await svc.list();
  assert.equal(c.attemptedAt, NOW, "the background refresh landed"); assert.equal(c.entries.find((e) => e.slug === "bitcoin-frogs")!.floor?.stale, false);
  const hits = net.hits.length;
  await svc.list(); await new Promise((r) => setTimeout(r, 30));
  assert.equal(net.hits.length, hits, "within the TTL nothing is asked again");
  assert.ok(slept.some((ms) => ms >= 4_000), "the spacer waited between two requests to one host");
});

test("a nest Rook lists that the seed does not name is still in the catalogue, from this indexer's own reads, marked uncurated", async () => {
  const { wrapped, wrappedConfig } = world();
  const seed = parseSeed({ ...loadSeed(), entries: loadSeed().entries.filter((e) => e.slug === "cryptopunks") });
  const c = await new CatalogService({ seed, fetch: null, wrapped, wrappedConfig, now: () => NOW }).list();
  const frogs = c.entries.find((e) => e.rook.kind === "live" && e.rook.slug === "frogs")!;
  assert.ok(frogs); assert.equal(frogs.curated, false); assert.equal(frogs.chain, "bitcoin"); assert.equal(frogs.slug, "frogs"); assert.equal(frogs.sources[0].name, "rook"); assert.ok(frogs.heldByRook !== null && frogs.heldByRook > 0);
  assert.equal(frogs.floor?.source, "rook", "the keeper's cap, as the wrapped reads report it");
  assert.equal((await new CatalogService({ seed, fetch: null, wrapped, wrappedConfig, now: () => NOW }).one("frogs"))?.entry.slug, "frogs");
});

test("the contract's own tokenURI through a public RPC, keyless: from the first of ids 1, 0, 2 that answers, skipping burned ids, else tokenByIndex; data:, ipfs:// and https metadata; a gateway that limits is asked under the next name", async () => {
  const enc = (str: string) => { const h = Buffer.from(str, "utf8").toString("hex"); return `0x${"20".padStart(64, "0")}${str.length.toString(16).padStart(64, "0")}${h.padEnd(Math.ceil(h.length / 64) * 64, "0")}`; };
  const meta = (name: string, image: string) => `data:application/json;base64,${Buffer.from(JSON.stringify({ name, image })).toString("base64")}`;
  const calls: { to: string; data: string }[] = [];
  const rpc = fakeFetch((u) => {
    if (u.includes("4everland.io")) return { status: 200, body: { name: "Kaiju #7", image: "ipfs://QmPic/7.png" } };
    if (u.includes("filebase.io")) return { status: 429, body: {} };
    if (u.endsWith("rhmachines.com/meta/1.json")) return { status: 200, body: { name: "RH #1", image: "https://www.rhmachines.com/img/1.png" } };
    return { status: 404 };
  });
  const net = (async (input: string | URL | Request, init?: RequestInit) => {
    const u = String(input);
    if (!u.includes("drpc.org") && !u.includes("xrpc.cl") && !u.includes("robinhood.com")) return rpc(input, init);
    const body = JSON.parse(String(init?.body)) as { params: [{ to: string; data: string }] };
    calls.push(body.params[0]);
    const { to, data } = body.params[0], id = BigInt(`0x${data.slice(10)}`), fn = data.slice(0, 10);
    const answer = (result: string | null) => new Response(JSON.stringify(result === null ? { jsonrpc: "2.0", id: 1, error: { message: "execution reverted" } } : { jsonrpc: "2.0", id: 1, result }), { status: 200, headers: { "content-type": "application/json" } });
    if (to === "0x1111111111111111111111111111111111111111") return answer(fn === "0xc87b56dd" && id >= 1n && id <= 8n ? enc(meta(`Catz #${id}`, id === 3n ? "not-a-url" : `ipfs://QmCatz/${id}.png`)) : null);   // 1..8, data: URIs
    if (to === "0x2222222222222222222222222222222222222222") return answer(fn === "0xc87b56dd" && id === 0n ? enc("https://www.rhmachines.com/meta/1.json") : fn === "0xc87b56dd" && id >= 2n && id < 8n ? enc("https://www.rhmachines.com/meta/x.json") : null);   // 0..7 (1 reverts), https
    if (to === "0x5555555555555555555555555555555555555555") return answer(fn === "0xc87b56dd" && id >= 2n && id !== 4n ? enc(meta(`Mancer #${id}`, `data:image/svg+xml;utf8,<svg xmlns="http://www.w3.org/2000/svg"><text>${id}</text></svg>`)) : null);   // #1 burned, #0 none, #4 burned: 2, 3, 5…10
    if (to === "0x3333333333333333333333333333333333333333") return answer(fn === "0x4f6ccce7" ? (id === 0n ? `0x${(7).toString(16).padStart(64, "0")}` : null) : fn === "0xc87b56dd" && id === 7n ? enc("ipfs://QmMeta/7") : null);   // tokenByIndex → 7, ipfs
    return answer(null);
  }) as typeof fetch;
  const t = new TokenUri(net, async () => undefined);
  const a = await t.items("robinhood", "0x1111111111111111111111111111111111111111", (id) => `https://x/${id}`);
  assert.ok(a.ok); assert.equal(a.value.length, 8); assert.deepEqual(a.value[0], { id: "1", name: "Catz #1", image: "https://ipfs.filebase.io/ipfs/QmCatz/1.png", url: "https://x/1" }); assert.equal(a.value[2].image, null, "a metadata image that is not a URL is no picture");
  const b = await t.items("ethereum", "0x2222222222222222222222222222222222222222", () => null);
  assert.ok(b.ok); assert.equal(b.value[0].id, "0"); assert.equal(b.value[0].image, "https://www.rhmachines.com/img/1.png"); assert.equal(b.value[1].id, "2", "a token that reverts is skipped"); assert.equal(b.value[1].image, null, "metadata that 404s is no picture");
  const c = await t.items("ethereum", "0x3333333333333333333333333333333333333333", () => null);
  assert.ok(c.ok); assert.deepEqual(c.value.map((i) => [i.id, i.image]), [["7", "https://ipfs.filebase.io/ipfs/QmPic/7.png"]], "found by tokenByIndex; Filebase limited, 4everland answered, the picture is named on the first gateway");
  const m = await t.items("robinhood", "0x5555555555555555555555555555555555555555", () => null);
  assert.ok(m.ok); assert.deepEqual(m.value.map((i) => i.id), ["2", "3", "5", "6", "7", "8", "9", "10"], "a burned #1 and #4 are walked past");
  assert.ok(m.value[0].image!.startsWith("data:image/svg+xml;base64,") && Buffer.from(m.value[0].image!.split(",")[1], "base64").toString("utf8").includes("<text>2</text>"), "a plain-text SVG data: URI is re-encoded as base64");
  const d = await t.items("ethereum", "0x4444444444444444444444444444444444444444", () => null);
  assert.equal(d.ok, false);
  await assert.rejects(t.items("ethereum", "nope", () => null), /invalid contract/);
  assert.equal((await t.items("solana", "0x1111111111111111111111111111111111111111", () => null)).ok, false, "no public RPC for a chain the table does not name");
  assert.equal(ipfsToHttp("ipfs://ipfs/QmA/1.png"), "https://ipfs.filebase.io/ipfs/QmA/1.png"); assert.equal(ipfsToHttp("https://a/b"), "https://a/b");
  assert.equal(await metadataOf("data:application/json;base64,bm90IGpzb24=", net), null);
  const svg = `data:image/svg+xml;base64,${Buffer.from("<svg xmlns='http://www.w3.org/2000/svg'/>").toString("base64")}`;
  const onchain = await fetchPicture(svg, net);
  assert.ok(onchain.ok && onchain.value.contentType === "image/svg+xml" && Buffer.from(onchain.value.body).toString("utf8").startsWith("<svg"), "an on-chain SVG is decoded, and no request is made");
  assert.equal((await fetchPicture("data:image/svg+xml,<svg/>", net)).ok, false, "only base64, only the shape");
  assert.equal(gatewayed("https://ipfs.io/ipfs/QmA/1.png"), "https://ipfs.filebase.io/ipfs/QmA/1.png", "a gateway that no longer serves paths is renamed onto a working one");
  assert.equal(gatewayed(`https://bafkrei${"a".repeat(52)}.ipfs.nftstorage.link/`), `https://ipfs.filebase.io/ipfs/bafkrei${"a".repeat(52)}`);
  assert.equal(gatewayed("https://i2c.seadn.io/x.png"), "https://i2c.seadn.io/x.png");
  assert.equal(pictureOf({ image_data: "<svg xmlns='http://www.w3.org/2000/svg'/>" })?.startsWith("data:image/svg+xml;base64,"), true, "raw SVG under image_data is a picture");
  assert.equal(pictureOf({ image: "data:text/plain,.|.O" }), null, "ASCII art is not a picture");
  const ins = "b".repeat(64) + "i0";
  assert.equal(owIcon("https://cdn.discordapp.com/attachments/1/2/x.png", { id: ins, content_type: "image/png" }), `https://ordinals.com/content/${ins}`, "the icon's inscription when it is an image");
  assert.equal(owIcon("https://cdn.discordapp.com/attachments/1/2/x.png", null), null, "a Discord attachment is no picture");
  assert.equal(owIcon(`https://turbo.ordinalswallet.com/inscription/preview/${ins}`, { id: ins, content_type: "text/html;charset=utf-8" }), `https://ordinals.com/content/${ins}`, "an HTML inscription is not taken as the icon; the preview URL is renamed");
  assert.equal(owIcon("https://media.ordinalswallet.com/x.png", null), "https://media.ordinalswallet.com/x.png");
  assert.equal(sniffRaster(new Uint8Array([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0])), "image/png"); assert.equal(sniffRaster(new TextEncoder().encode("<svg/>")), null);
  const bucket = fakeFetch((u) => (u.endsWith("/kong.png") ? { status: 200, bytes: new Uint8Array([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 1, 2]), type: "binary/octet-stream" } : { status: 200, bytes: new TextEncoder().encode("<html>"), type: "application/octet-stream" }));
  const kong = await fetchPicture("https://cdn.example.com/kong.png", bucket);
  assert.ok(kong.ok && kong.value.contentType === "image/png", "an untyped answer whose bytes are a PNG is a PNG");
  assert.equal((await fetchPicture("https://cdn.example.com/page", bucket)).ok, false, "an untyped answer that is not a raster picture is refused");
  assert.equal(isPublicHost("i.seadn.io"), true); assert.equal(isPublicHost("172.20.0.1"), false); assert.equal(isPublicHost("172.32.0.1"), true);
});

// ---- the endpoints ----------------------------------------------------------------------------------------------------------------------
test("GET /v1/rook/catalog, /v1/rook/catalog/:slug, the picture routes, and the index names them", async () => {
  const { w, wrapped, wrappedConfig } = world();
  const db = openDb(":memory:"), views = new Views(db, new Live(null, null), () => NOW);
  const net = goodNet(NOW);
  const catalog = new CatalogService({ seed: fewSeed(), fetch: net, wrapped, wrappedConfig, now: () => NOW, sleep: async () => undefined, apiKey: () => FAKE_KEY });
  await catalog.refresh();
  const rook = rookRoutes(new RookService({ db, views, wrapped, wrappedConfig, rpc: w.rpc, now: () => NOW, ttlS: 0 }), catalog);
  const server = serve({ views, wrapped, rook, follower: () => ({ chainId: 4663, cursor: 0, head: 0, lastPassAt: NOW, deployment: false } as never), confirmations: 0, perMinute: 10_000, burst: 10_000 }, 0, "127.0.0.1");
  await new Promise((r) => server.once("listening", r));
  const base = `http://127.0.0.1:${(server.address() as AddressInfo).port}`;
  const get = async <X>(p: string) => { const r = await fetch(base + p); return { r, j: (await r.json()) as X }; };
  try {
    const all = await get<Catalog>("/v1/rook/catalog");
    assert.equal(all.r.status, 200); assert.equal(all.r.headers.get("cache-control"), "public, max-age=60"); assert.equal(all.j.entries.length, FEW.length); assert.equal(all.j.rule.stage, 0);
    const one = await get<CatalogOne>("/v1/rook/catalog/boredapeyachtclub");
    assert.equal(one.r.status, 200); assert.equal(one.j.entry.rook.kind, "live");
    assert.equal((await get<CatalogOne>("/v1/rook/catalog/apes")).j.entry.slug, "boredapeyachtclub");
    assert.equal((await get("/v1/rook/catalog/nothing-here")).r.status, 404);
    assert.equal((await get("/v1/rook/catalog/bad slug")).r.status, 404);
    const pic = await fetch(`${base}/v1/rook/catalog/boredapeyachtclub/image`);
    assert.equal(pic.status, 200); assert.equal(pic.headers.get("content-type"), "image/png"); assert.equal(pic.headers.get("cache-control"), "public, max-age=86400"); assert.equal(pic.headers.get("x-content-type-options"), "nosniff");
    assert.deepEqual([...new Uint8Array(await pic.arrayBuffer())], [...PNG]);
    const item = await fetch(`${base}/v1/rook/catalog/bitcoin-frogs/items/0/image`);
    assert.equal(item.status, 200); assert.equal(item.headers.get("content-type"), "image/webp");
    const none = await fetch(`${base}/v1/rook/catalog/bitcoin-frogs/items/1/image`);
    assert.equal(none.status, 404); assert.equal(none.headers.get("content-type"), "application/json; charset=utf-8");
    assert.equal((await fetch(`${base}/v1/rook/catalog/taproot-wizards/image`, { method: "HEAD" })).status, 404, "its picture's host did not answer: the site draws a mark");
    const index = await get<{ endpoints: string[] }>("/v1");
    for (const p of ["/v1/rook/catalog", "/v1/rook/catalog/:slug", "/v1/rook/catalog/:slug/image", "/v1/rook/catalog/:slug/items/:i/image"]) assert.ok(index.j.endpoints.includes(p), p);
    const bare = rookRoutes(new RookService({ db, views, wrapped, wrappedConfig, rpc: w.rpc, now: () => NOW, ttlS: 0 }));
    assert.equal((await bare("/v1/rook/catalog", new URL("http://x/v1/rook/catalog")))?.status, 503, "an indexer without the catalogue says so");
  } finally { server.close(); db.close(); }
});
