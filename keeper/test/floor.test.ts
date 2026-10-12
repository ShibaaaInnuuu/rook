import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { Clusters, costToLower, costToRaise, robustFloor, satsToWei, type Contents, type Listing, type Sale } from "../src/floor.js";
import { BookDiff, CoinbaseRate, FixtureOrd, OrdinalsWalletBook, parseOpenSeaListings, parseOpenSeaSales, parseOwListings, parseOwSales,
  withContents, OpenSeaBook } from "../src/floorsources.js";
import { openseaGet } from "../src/opensea.js";
import { logger } from "../src/log.js";

// docs/v5/floor-feed.md §1, §3, §4. Synthetic books for each rule; the recorded 2026-09-23 books for the measurements.
const NOW = 1_790_162_113;
const K = 1_000n;
const insc = (n: number) => `${n.toString(16).padStart(64, "0")}i0`;
const clean = (id: string): Contents => ({ inscriptions: [id], runes: [] });
const addr = (n: number) => `bc1p${String(n).padStart(3, "0")}${"q".repeat(55)}`;

function L(n: number, price: bigint, o: Partial<Listing> = {}): Listing {
  const id = insc(n);
  return { id, chain: "btc", seller: addr(n), price, listedAt: NOW - 7_200, verifiedAt: NOW - 60, fillable: true, contents: clean(id), ...o };
}
function S(n: number, price: bigint, o: Partial<Sale> = {}): Sale {
  return { tx: `${n.toString(16).padStart(64, "a")}`, at: NOW - 3_600 * n, item: insc(500 + n), seller: addr(300 + n), buyer: addr(400 + n), price, itemsInTx: 1, ...o };
}
/** Seven sellers at 100…130k sats, and six unrelated sales at 100k. L = 104k, S = 100k, floor = min(104, 0.7·104 + 0.3·100) = 102.8k. */
const book = (): Listing[] => [L(1, 100n * K), L(2, 102n * K), L(3, 104n * K), L(4, 106n * K), L(5, 108n * K), L(6, 120n * K), L(7, 130n * K)];
const sales = (): Sale[] => [1, 2, 3, 4, 5, 6].map((n) => S(n, 100n * K));
const rules = (f: ReturnType<typeof robustFloor>, id: string) => f.excluded.filter((e) => e.id === id || e.id.endsWith(`:${id}`)).map((e) => e.rule);

test("§1.4 the statistic: lower median of the lowest five listings, one per seller, blended down toward the sales median", () => {
  const f = robustFloor({ listings: book(), sales: sales(), snapshotAt: NOW });
  assert.equal(f.listingFloor, 104n * K);
  assert.equal(f.saleFloor, 100n * K);
  assert.equal(f.price, 102_800n);
  assert.deepEqual(f.used, [insc(1), insc(2), insc(3), insc(4), insc(5)]);
  assert.equal(f.depthItems, 5, "valid listings at or under floor × 1.10 = 113,080");
  assert.equal(f.upCost10, (100n + 102n + 104n + 106n) * K, "buy four from the bottom: the median of 108,120,130 is 120 ≥ 114.4");
  // sales above the listing floor never lift it
  const high = robustFloor({ listings: book(), sales: [1, 2, 3, 4, 5, 6].map((n) => S(n, 300n * K)), snapshotAt: NOW });
  assert.equal(high.price, 104n * K);
  // no sales: the listing floor alone; under three listings: no floor at all
  assert.equal(robustFloor({ listings: book(), sales: [], snapshotAt: NOW }).price, 104n * K);
  const thin = robustFloor({ listings: book().slice(0, 2), sales: sales(), snapshotAt: NOW });
  assert.equal(thin.price, null);
  assert.equal(thin.confidenceBps, 0);
});

test("§1.5 confidence: count × spread × sales × agreement, in bps", () => {
  const f = robustFloor({ listings: book(), sales: sales(), snapshotAt: NOW });
  // count 10000; spread 1 − 8/(2·104) = 9616; sales 10000; agreement 1 − 4/104 = 9616
  assert.equal(f.confidenceBps, 9_246);
  // three listings (count 6000), spread 1 − 4/(2·102) = 9804, no sales (6000), no disagreement
  const three = robustFloor({ listings: book().slice(0, 3), sales: [], snapshotAt: NOW });
  assert.equal(three.confidenceBps, 3_529);
});

test("L-unverified and L-extras: ord must say the outpoint carries exactly the one inscription and no rune", () => {
  const ls = book();
  ls[0] = L(1, 50n * K, { contents: null });
  ls[1] = L(2, 51n * K, { contents: { inscriptions: [insc(2), insc(99)], runes: [] } });
  ls[2] = L(3, 52n * K, { contents: { inscriptions: [insc(3)], runes: ["840000:3"] } });
  const f = robustFloor({ listings: [...ls, L(8, 140n * K), L(9, 150n * K)], sales: [], snapshotAt: NOW });
  assert.deepEqual(rules(f, insc(1)), ["L-unverified"]);
  assert.deepEqual(rules(f, insc(2)), ["L-extras"]);
  assert.deepEqual(rules(f, insc(3)), ["L-extras"]);
  assert.equal(f.listingFloor, 120n * K, "106, 108, 120, 130, 140: none of the three cheap bundles moved it");
});

test("L-currency, L-extras, L-unfillable, L-stale on EVM orders", () => {
  const e = (n: number, price: bigint, o: Partial<Listing> = {}): Listing =>
    ({ id: `0xabc:${n}`, chain: "evm", seller: `0x${String(n).padStart(40, "0")}`, price, listedAt: NOW - 7_200, verifiedAt: NOW - 30, fillable: true,
      expiresAt: NOW + 86_400, offerItems: 1, nativeCurrency: true, special: false, ...o });
  const E18 = 10n ** 18n;
  const f = robustFloor({ listings: [e(1, 1n * E18, { nativeCurrency: false }), e(2, 1n * E18, { offerItems: 3 }), e(3, 1n * E18, { special: true }),
    e(4, 1n * E18, { fillable: false }), e(5, 1n * E18, { verifiedAt: NOW - 3_600 }), e(6, 1n * E18, { expiresAt: NOW + 600 }),
    e(7, 6n * E18), e(8, 6n * E18), e(9, 7n * E18)], sales: [], snapshotAt: NOW });
  assert.deepEqual(f.excluded.map((x) => [x.id, x.rule]), [["0xabc:1", "L-currency"], ["0xabc:2", "L-extras"], ["0xabc:3", "L-currency"],
    ["0xabc:4", "L-unfillable"], ["0xabc:5", "L-stale"], ["0xabc:6", "L-stale"]]);
  assert.equal(f.listingFloor, 6n * E18);
});

test("scenario: a fake cheap listing must survive thirty minutes of snipers, and alone moves the median one step", () => {
  const fake = L(90, 50n * K, { listedAt: NOW - 600 });
  const f = robustFloor({ listings: [...book(), fake], sales: [], snapshotAt: NOW });
  assert.deepEqual(rules(f, insc(90)), ["L-unseasoned"]);
  assert.equal(f.price, 104n * K);
  const later = robustFloor({ listings: [...book(), fake].map((l) => ({ ...l, verifiedAt: NOW + 1_800 - 60 })), sales: [], snapshotAt: NOW + 1_800 });
  assert.equal(later.listingFloor, 102n * K, "seasoned and still fillable, it is one vote of five: 50, 100, 102, 104, 106");
  // and the attack's price, per §4.2: 3 − (listings already at or under the target)
  assert.deepEqual(costToLower(book(), new Clusters(), 90n * K), { listings: 3, lossIfSniped: 3n * 14n * K });
  assert.deepEqual(costToLower(book(), new Clusters(), 101n * K), { listings: 2, lossIfSniped: 2n * 3n * K });
});

test("scenario: a delist burst excludes the wallet for six hours after its last event, whatever it lists", () => {
  const w = addr(95);
  const cheap = L(95, 60n * K, { seller: w });
  const events = [1, 2, 3].flatMap((i) => [{ wallet: w, at: NOW - 3_000 + i * 600, kind: "list" as const }, { wallet: w, at: NOW - 2_900 + i * 600, kind: "delist" as const }]);
  const f = robustFloor({ listings: [...book(), cheap], sales: [], events, snapshotAt: NOW });
  assert.deepEqual(rules(f, insc(95)), ["L-burst"]);
  assert.equal(f.price, 104n * K);
  // two delists is not a burst
  assert.equal(robustFloor({ listings: [...book(), cheap], sales: [], events: events.slice(0, 4), snapshotAt: NOW }).listingFloor, 102n * K);
  // six hours after the last event it counts again
  const after = NOW + 21_600;
  const back = robustFloor({ listings: [...book(), { ...cheap, verifiedAt: after }].map((l) => ({ ...l, verifiedAt: after })), sales: [], events, snapshotAt: after });
  assert.equal(back.listingFloor, 102n * K);
  // the keeper's own diff of successive books produces those events
  const d = new BookDiff();
  const snap = (at: number, withIt: boolean) => ({ listings: withIt ? [cheap] : [], sales: [], snapshotAt: at, ref: "t" });
  let ev = d.observe("frogs", snap(NOW - 3_000, false));
  for (let i = 0; i < 3; i++) { ev = d.observe("frogs", snap(NOW - 2_400 + i * 600, true)); ev = d.observe("frogs", snap(NOW - 2_100 + i * 600, false)); }
  assert.equal(ev.filter((e) => e.kind === "delist").length, 3);
  assert.deepEqual(rules(robustFloor({ listings: [...book(), cheap], sales: [], events: ev, snapshotAt: NOW }), insc(95)), ["L-burst"]);
});

test("L-seller-dup: one wallet, or one cluster, is one vote", () => {
  const me = addr(80);
  const mine = [L(80, 60n * K, { seller: me }), L(81, 61n * K, { seller: me }), L(82, 62n * K, { seller: me })];
  const f = robustFloor({ listings: [...book(), ...mine], sales: [], snapshotAt: NOW });
  assert.deepEqual([rules(f, insc(81)), rules(f, insc(82))], [["L-seller-dup"], ["L-seller-dup"]]);
  assert.equal(f.listingFloor, 102n * K, "60, 100, 102, 104, 106");
  // a second wallet linked to the first by a both-direction trade is the same vote
  const other = addr(83);
  const linked = [S(20, 100n * K, { seller: me, buyer: other }), S(21, 100n * K, { seller: other, buyer: me })];
  const g = robustFloor({ listings: [...book(), mine[0], L(83, 59n * K, { seller: other })], sales: linked, snapshotAt: NOW });
  assert.equal(g.listingFloor, 102n * K, "59 counts once for both wallets");
  assert.ok(g.excluded.some((e) => e.id === insc(80) && e.rule === "L-seller-dup"));
});

test("scenario: wash sales — self-sales, both-direction pairs and sub-quarter transfers are dropped, and cannot lift the floor anyway", () => {
  const a = addr(70), b = addr(71);
  const wash = [S(30, 400n * K, { seller: a, buyer: b }), S(31, 410n * K, { seller: b, buyer: a }), S(32, 420n * K, { seller: a, buyer: a }),
    S(33, 5n * K, { seller: addr(72), buyer: addr(73) })];
  const f = robustFloor({ listings: book(), sales: [...sales(), ...wash], snapshotAt: NOW });
  const by = Object.fromEntries(f.excluded.filter((e) => e.what === "sale").map((e) => [e.id.split(":")[0], e.rule]));
  assert.equal(by[S(30, 0n).tx], "S-wash");
  assert.equal(by[S(31, 0n).tx], "S-wash");
  assert.equal(by[S(32, 0n).tx], "S-self");
  assert.equal(by[S(33, 0n).tx], "S-wash", "a 5k 'sale' at under a quarter of the floor links the two wallets");
  assert.equal(f.saleFloor, 100n * K);
  assert.equal(f.price, 102_800n);
});

test("S-bundle, S-outlier, S-repeat, S-window", () => {
  const s = [S(40, 100n * K, { itemsInTx: 5 }), S(41, 1_000n * K), S(42, 100n * K, { at: NOW - 8 * 86_400 }),
    ...sales(), S(43, 101n * K, { buyer: addr(401) })];
  const f = robustFloor({ listings: book(), sales: s, snapshotAt: NOW });
  const r = (n: number) => f.excluded.find((e) => e.id.startsWith(S(n, 0n).tx))?.rule;
  assert.equal(r(40), "S-bundle");
  assert.equal(r(41), "S-outlier");
  assert.equal(r(42), "S-window");
  assert.equal(r(43), "S-repeat", "buyer 401 bought again later (sale 1): the latest counts, once");
  assert.equal(r(1), undefined);
});

test("scenario: a bought-out bottom — the cost to raise the floor is the sum of what has to be bought", () => {
  const c = new Clusters();
  assert.deepEqual(costToRaise(book(), c, 105n * K), { items: 1, gross: 100n * K });
  assert.deepEqual(costToRaise(book(), c, 108n * K), { items: 2, gross: 202n * K });
  assert.equal(costToRaise(book(), c, 200n * K), null, "the book runs out first");
});

// ---- the recorded measurements (docs/v5/floor-feed.md §3–4) ----------------------------------------------------------------------

const OW_L = JSON.parse(readFileSync(new URL("../fixtures/floor/ow-bitcoin-frogs-listings-2026-09-23.json", import.meta.url), "utf8"));
const OW_S = JSON.parse(readFileSync(new URL("../fixtures/floor/ow-bitcoin-frogs-sales-2026-09-23.json", import.meta.url), "utf8"));
const BAYC = JSON.parse(readFileSync(new URL("../fixtures/floor/eth-bayc-seaport-2026-09-23.json", import.meta.url), "utf8"));

test("§3.1 frogs, recorded: without ord every Bitcoin listing is unverified and nothing is posted; with ord clean, the measured floor", async () => {
  const listings = parseOwListings(OW_L.listings, OW_L.fetchedAt);
  const sold = parseOwSales({ sales: OW_S.sales });
  assert.equal(listings.length, 79);
  const blind = robustFloor({ listings: await withContents(listings, null, 100), sales: sold, snapshotAt: OW_L.fetchedAt });
  assert.equal(blind.price, null, "fail closed: ordinals.com's JSON API was disabled on the day");
  assert.ok(blind.excluded.filter((e) => e.rule === "L-unverified").length === 79);
  // ASSUMPTION for the measurement: ord would have answered "exactly this inscription, no runes" for every listed outpoint
  const ord = new FixtureOrd(Object.fromEntries(listings.map((l) => [l.outpoint, clean(l.id)])));
  const f = robustFloor({ listings: await withContents(listings, ord, 100), sales: sold, snapshotAt: OW_L.fetchedAt });
  assert.equal(f.listingFloor, 565_262n);
  // audit E-01 (docs/audit/2026-09-26/fixes-W2-services.md §3): two frogs sold twice in the week count once (S-item-repeat), and
  // sales from one marketplace pull the floor at most 5% below the listings. Before: S 360,000, floor 503,683 at 4,900 bps.
  assert.equal(f.saleFloor, 382_330n);
  assert.equal(f.excluded.filter((e) => e.rule === "S-item-repeat").length, 2);
  assert.equal(f.price, 536_999n, "max((7 × 565,262 + 3 × 382,330) / 10, 565,262 × 0.95): the single-marketplace pull cap binds");
  assert.equal(f.confidenceBps, 5_204);
  assert.equal(f.depthItems, 3);
  assert.equal(f.upCost10, 998_331n, "two frogs from the bottom");
  // with a second, independent marketplace's sales the full blend applies (the pull cap is for one source)
  const two = robustFloor({ listings: await withContents(listings, ord, 100), sales: sold, snapshotAt: OW_L.fetchedAt, saleSources: 2 });
  assert.equal(two.price, 510_382n, "min(565,262, (7 × 565,262 + 3 × 382,330) / 10)");
  console.log("frogs:", { L: f.listingFloor, S: f.saleFloor, floor: f.price, conf: f.confidenceBps, depth: f.depthItems, upCost10: f.upCost10,
    wei: f.price === null ? null : satsToWei(f.price, 3_185_800n), excludedSales: f.excluded.filter((e) => e.what === "sale").map((e) => e.rule) });
  assert.ok(f.price !== null && f.price <= f.listingFloor!);
  const rulesSeen = new Set(f.excluded.map((e) => e.rule));
  for (const r of ["S-bundle", "S-window"]) assert.ok(rulesSeen.has(r as never), `the real sales exercise ${r}`);
});

test("§3.2 BAYC, recorded: the week's Seaport fills as a proxy ladder, the sales rules on real wash and bundle patterns", () => {
  const E18 = 10n ** 18n;
  const t = BAYC.transfers as { tx: string; ts: number; from: string; to: string; tokenId: number; priceWei: string | null; kind: string | null; nftsInTx: number }[];
  const priced = t.filter((x) => x.priceWei !== null);
  assert.equal(t.length, 199);
  assert.equal(priced.length, 50);
  const ladder: Listing[] = priced.filter((x) => x.kind === "listing" && BigInt(x.priceWei!) > 0n).map((x, i) => ({ id: `bayc:${x.tokenId}:${i}`, chain: "evm",
    seller: `0x${(i + 1).toString(16).padStart(40, "0")}`, price: BigInt(x.priceWei!), listedAt: BAYC.fetchedAt - 7_200, verifiedAt: BAYC.fetchedAt, fillable: true,
    expiresAt: null, offerItems: 1, nativeCurrency: true, special: false }));
  const sold: Sale[] = priced.map((x) => ({ tx: x.tx, at: x.ts, item: String(x.tokenId), seller: x.from, buyer: x.to, price: BigInt(x.priceWei!), itemsInTx: x.nftsInTx }));
  const f = robustFloor({ listings: ladder, sales: sold, snapshotAt: BAYC.fetchedAt });
  assert.equal(f.listingFloor, 6_098_400_000_000_000_000n);
  assert.equal(f.price, f.listingFloor, "S is above L: the floor is the listing floor");
  const n = (r: string) => f.excluded.filter((e) => e.what === "sale" && e.rule === r).length;
  console.log("bayc:", { L: f.listingFloor, S: f.saleFloor, conf: f.confidenceBps, depth: f.depthItems, upCost10: f.upCost10,
    bundle: n("S-bundle"), wash: n("S-wash"), outlier: n("S-outlier"), repeat: n("S-repeat"), self: n("S-self") });
  assert.deepEqual([n("S-bundle"), n("S-wash"), n("S-repeat"), n("S-item-repeat")], [8, 5, 15, 6],
    "the sweep; the back-and-forth pair and the 0 ETH 'sale' (linked by rule 2); the bots; apes flipped twice in the week count once (E-01)");
  assert.equal(f.saleFloor, 6_660_000_000_000_000_000n);
  assert.equal(f.confidenceBps, 8_782);
  assert.equal(f.depthItems, 12);
  const c = new Clusters();
  assert.equal(costToRaise(ladder, c, (f.listingFloor! * 105n) / 100n)!.items, 2);
  assert.equal(costToRaise(ladder, c, (f.listingFloor! * 110n) / 100n)!.items, 10);
  assert.equal(costToLower(ladder, c, (f.listingFloor! * 90n) / 100n).listings, 3);
  assert.ok(f.price! / E18 === 6n);
});

// ---- sources: GET only, strict, the key never logged -------------------------------------------------------------------------------

const FAKE_KEY = "fake-opensea-key-floor-0123456789";
function server(replies: Record<string, { status: number; body?: unknown }>) {
  const calls: { url: string; method: string; headers: Record<string, string> }[] = [];
  const f = (async (url: string, init: RequestInit = {}) => {
    calls.push({ url, method: String(init.method ?? "GET"), headers: (init.headers ?? {}) as Record<string, string> });
    const k = Object.keys(replies).find((p) => url.includes(p));
    const r = k ? replies[k] : { status: 404 };
    return { ok: r.status >= 200 && r.status < 300, status: r.status, headers: new Headers(), json: async () => r.body, text: async () => JSON.stringify(r.body ?? "") };
  }) as unknown as typeof fetch;
  return { f, calls };
}

test("OpenSea: GETs with the key read at call time; the key is in no log line and no error; plain ETH listings only", async () => {
  const offer = (n: number, token = "0xbc4ca0eda7647a8ab7c2061c2e118a18a936f13d", itemType = 2) => ({ itemType, token, identifierOrCriteria: String(n), startAmount: "1", endAmount: "1" });
  const listing = (n: number, eth: string, o: { currency?: string; offers?: unknown[]; type?: string } = {}) => ({ type: o.type ?? "basic", price: { current: { currency: o.currency ?? "ETH", decimals: 18, value: eth } },
    protocol_data: { parameters: { offerer: `0x${String(n).padStart(40, "1")}`, offer: o.offers ?? [offer(n)], consideration: [], startTime: String(NOW - 7_200), endTime: String(NOW + 86_400) } } });
  const sale = (n: number, wei: string, tx = `0x${String(n).padStart(64, "0")}`) => ({ event_type: "sale", closing_date: NOW - 100 * n, transaction: tx,
    seller: `0x${String(n).padStart(40, "2")}`, buyer: `0x${String(n).padStart(40, "3")}`, quantity: 1, nft: { identifier: String(n) },
    payment: { quantity: wei, token_address: "0x0000000000000000000000000000000000000000", decimals: 18, symbol: "ETH" } });
  const { f, calls } = server({
    "listings/collection/boredapeyachtclub/best": { status: 200, body: { listings: [listing(1, "6000000000000000000"), listing(2, "6100000000000000000"),
      listing(3, "6200000000000000000"), listing(4, "1000000000000000000", { currency: "WETH" }), listing(5, "1000000000000000000", { offers: [offer(5), offer(6)] })] } },
    "events/collection/boredapeyachtclub": { status: 200, body: { asset_events: [sale(1, "6500000000000000000"), sale(2, "6600000000000000000"), sale(3, "6700000000000000000")] } },
  });
  const lines: string[] = [];
  const log = logger((l) => lines.push(l), () => new Date(0), () => [FAKE_KEY]);
  const b = await new OpenSeaBook({ fetchImpl: f, apiKey: () => FAKE_KEY, log, sleep: async () => {} }, () => NOW * 1000).book("boredapeyachtclub", "0xbc4ca0eda7647a8ab7c2061c2e118a18a936f13d");
  assert.ok(calls.every((c) => c.method === "GET" && c.url.startsWith("https://api.opensea.io/api/v2/") && c.headers["x-api-key"] === FAKE_KEY));
  assert.ok(calls.every((c) => !c.url.includes(FAKE_KEY)), "the key is a header, never in a URL");
  const fl = robustFloor({ listings: b.listings, sales: b.sales, snapshotAt: NOW });
  assert.equal(fl.listingFloor, 6_100_000_000_000_000_000n);
  assert.deepEqual(fl.excluded.map((e) => e.rule), ["L-currency", "L-extras"]);
  // a log line that somehow carries the key has it cut out
  log("info", "debug", { note: `oops ${FAKE_KEY}` });
  assert.ok(lines.every((l) => !l.includes(FAKE_KEY)));
  // a missing key is an error that names the variable and nothing else, and no request is made
  const { f: f2, calls: c2 } = server({});
  await assert.rejects(openseaGet("listings/collection/x/best", { fetchImpl: f2, apiKey: () => undefined, sleep: async () => {} }), (e: Error) => /missing OPENSEA_API_KEY/.test(e.message) && !e.message.includes(FAKE_KEY));
  assert.equal(c2.length, 0);
  // a rejected key is not retried and the error does not carry it
  const { f: f3, calls: c3 } = server({ "listings": { status: 401 } });
  await assert.rejects(openseaGet("listings/collection/x/best", { fetchImpl: f3, apiKey: () => FAKE_KEY, sleep: async () => {} }), (e: Error) => /rejected the key/.test(e.message) && !e.message.includes(FAKE_KEY));
  assert.equal(c3.length, 1);
  // a shape it does not know is an error, not a price
  assert.throws(() => parseOpenSeaListings({ nope: 1 }, "0x00", NOW), /invalid answer/);
  assert.throws(() => parseOpenSeaSales({ asset_events: [{ event_type: "sale" }] }, "0x00"), /invalid sale/);
});

test("Ordinals Wallet and ord: GET only, spaced out, strict, and an unanswered outpoint stays unverified", async () => {
  const listed = OW_L.listings.slice(0, 3);
  const { f, calls } = server({ "escrows": { status: 200, body: listed }, "/sales": { status: 200, body: { sales: OW_S.sales.slice(0, 2) } } });
  let t = 10_000_000; const slept: number[] = [];
  const ow = new OrdinalsWalletBook(f, () => t, 3_000, async (ms) => { slept.push(ms); t += ms; });
  const ord = new FixtureOrd({ [listed[0].outpoint]: clean(listed[0].inscription_id) });
  const b = await ow.book("bitcoin-frogs", ord);
  assert.deepEqual(calls.map((c) => [c.method, c.url]), [["GET", "https://turbo.ordinalswallet.com/collection/bitcoin-frogs/escrows"], ["GET", "https://turbo.ordinalswallet.com/collection/bitcoin-frogs/sales"]]);
  assert.deepEqual(slept, [3_000], "one request per three seconds");
  assert.equal(b.listings.filter((l) => l.contents !== null).length, 1);
  assert.equal(b.sales.length, 2);
  await assert.rejects(ow.book("Bad Slug!", ord), /invalid collection slug/);
  assert.throws(() => parseOwListings([{ inscription_id: "x" }], NOW), /invalid listing/);
  // Coinbase's rate, parsed as integer sats per ETH
  const { f: cb } = server({ "exchange-rates": { status: 200, body: { data: { rates: { BTC: "0.031858" } } } } });
  assert.equal(await new CoinbaseRate(cb).satsPerEth(), 3_185_800n);
  assert.equal(satsToWei(507_033n, 3_185_800n), (507_033n * 10n ** 18n) / 3_185_800n);
  assert.throws(() => satsToWei(1n, 0n), /invalid rate/);
});
