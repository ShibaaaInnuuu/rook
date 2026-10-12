// scripts/measure-robinhood-nfts.mjs, offline: a fake fetch answers OpenSea's and Coinbase's reads. Run: node --test scripts/measure-robinhood-nfts.test.mjs
import assert from "node:assert/strict";
import { test } from "node:test";
import { COLLECTIONS, entries, judge, measureAll, SEAPORT_1_6, table, tierOf, twxOf } from "./measure-robinhood-nfts.mjs";

const NOW = 1_791_700_000;
const E = 10n ** 18n;
const listing = (offerer, wei) => ({ order_hash: `0x${offerer}`, protocol_address: SEAPORT_1_6, price: { current: { value: String(wei) } },
  protocol_data: { parameters: { offerer: `0x${offerer.repeat(40).slice(0, 40)}`, offer: [{ itemType: 2 }] } } });
const sale = (q = 1) => ({ event_type: "sale", quantity: q, nft: { identifier: "1" }, event_timestamp: NOW - 86_400 });

test("the twelve collections of Nosh's registry are measured; StonkBrokers is in the list, flagged token-bound", () => {
  assert.equal(COLLECTIONS.length, 12);
  const s = COLLECTIONS.filter((c) => c.tokenBound);
  assert.deepEqual(s.map((c) => c.name), ["StonkBrokers"]);
  assert.match(s[0].note, /ERC-6551.*tokenized stocks/);
});

test("the tier and T_wX: 85 for a floor of 1–10 SOL; a high floor gets its own tier, T_wX stays sensible", () => {
  assert.equal(tierOf(1), 85);
  assert.equal(tierOf(10), 85);
  assert.equal(twxOf(85, 10), 9);
  assert.equal(twxOf(85, 40), 3, "at least 3");
  // StonkBrokers at 12 ETH and 22.7 SOL/ETH: 272 SOL; the smallest multiple of 85 covering 1.33 × it
  const t = tierOf(272.4);
  assert.ok(t > 510 && t % 85 === 0 && t * 0.5 * 0.75 / 1.03 >= 1.33 * 272.4, String(t));
  assert.equal(twxOf(t, 272.4), 4);
});

test("the criteria: liquid, buyable from 3 sellers, a floor in [1, 10] SOL; a token-bound collection has its own cap within 25 ETH", () => {
  const base = { slug: "cashcats", sales: Array.from({ length: 31 }, () => sale()), statsFloorEth: null,
    listings: [listing("a", 26n * 10n ** 16n), listing("b", 27n * 10n ** 16n), listing("c", 3n * 10n ** 17n)] };
  const ok = judge(COLLECTIONS[3], base, 22.7);
  assert.equal(ok.verdict, "in");
  assert.equal(ok.tier, 85);
  assert.equal(ok.twx, Math.ceil(85 / (0.26 * 22.7)));
  assert.equal(ok.maxPurchaseWei, 39n * 10n ** 16n, "1.5 × 0.26 ETH");
  assert.match(judge(COLLECTIONS[3], { ...base, sales: base.sales.slice(0, 29).concat([sale(2), sale(3)]) }, 22.7).verdict, /29 single-item sales/);
  assert.match(judge(COLLECTIONS[3], { ...base, listings: [listing("a", E), listing("a", E), listing("c", E)] }, 22.7).verdict, /2 sellers/);
  assert.match(judge(COLLECTIONS[3], { ...base, listings: base.listings.map((l) => ({ ...l, protocol_address: "0x00000000000000adc04c56bf30ac9d3c0aaf14dc" })) }, 22.7).verdict,
    /0 Seaport 1.6 listings/, "a Seaport 1.5 listing is not buyable on 4663");
  assert.match(judge(COLLECTIONS[3], { ...base, listings: [listing("a", 10n ** 15n), listing("b", 10n ** 15n), listing("c", 10n ** 15n)] }, 22.7).verdict, /outside \[1, 10\] SOL/);
  // StonkBrokers at 12 ETH: in, token-bound, its own cap (1.25 × the floor) and tier
  const st = judge(COLLECTIONS[1], { ...base, slug: "stonkbrokers", listings: [listing("a", 12n * E), listing("b", 125n * E / 10n), listing("c", 13n * E)] }, 22.7);
  assert.equal(st.verdict, "in (TOKEN-BOUND: its own cap and tier)");
  assert.equal(st.maxPurchaseWei, 15n * E);
  assert.ok(st.tier > 510 && st.twx === 4, `${st.tier} ${st.twx}`);
  const dear = judge(COLLECTIONS[1], { ...base, listings: [listing("a", 21n * E), listing("b", 22n * E), listing("c", 23n * E)] }, 22.7);
  assert.match(dear.verdict, /over the canister's 25 ETH ceiling/);
  const e = entries({ ...st, slug: "stonkbrokers" }, 22.7, "2026-10-12T00:00Z");
  assert.deepEqual([e.nftPilot.chainId, e.nftPilot.collection, e.nftPilot.tokenBound, e.nftPilot.maxPurchaseWei, e.nftPilot.twx],
    [4663, COLLECTIONS[1].address, true, (15n * E).toString(), 4]);
  assert.equal(e.graduationTiers.stonkbrokers.tier, st.tier);
});

test("a whole run against a fake OpenSea and Coinbase: slugs by contract, stats, listings, 30 days of sale events, a table", async () => {
  const asked = [];
  const f = async (url, init) => {
    const u = new URL(url);
    asked.push(u.pathname);
    assert.equal(init.redirect, "manual");
    if (u.host === "api.coinbase.com") return new Response(JSON.stringify({ data: { rates: { SOL: "22.7" } } }));
    assert.equal(init.headers["x-api-key"], "k");
    const m = u.pathname.match(/^\/api\/v2\/chain\/robinhood\/contract\/(0x[0-9a-f]{40})$/);
    if (m) return m[1] === COLLECTIONS[0].address ? new Response("{}", { status: 404 }) : new Response(JSON.stringify({ collection: `s-${m[1].slice(2, 6)}` }));
    if (u.pathname.endsWith("/stats")) return new Response(JSON.stringify({ total: { floor_price: 0.3 } }));
    if (u.pathname.includes("/listings/collection/")) return new Response(JSON.stringify({ listings: [listing("a", 3n * 10n ** 17n), listing("b", 3n * 10n ** 17n), listing("c", 4n * 10n ** 17n)] }));
    if (u.pathname.includes("/events/collection/")) {
      const page = u.searchParams.get("next") ? 2 : 1;
      return new Response(JSON.stringify({ asset_events: Array.from({ length: 20 }, () => sale()), next: page === 1 ? "p2" : null }));
    }
    throw new Error(`unexpected ${url}`);
  };
  const r = await measureAll(f, { OPENSEA_API_KEY: "k" }, NOW);
  assert.equal(r.solPerEth, 22.7);
  assert.equal(r.rows.length, 12);
  assert.match(r.rows[0].verdict, /unread: .*HTTP 404/);
  // every measured row passes but an excluded one (QUOTRONS: ERC-404), which is out whatever its numbers
  assert.ok(r.rows.slice(1).every((x) => x.sales30d === 40 && (x.pass || /excluded: ERC-404/.test(x.verdict))), r.rows.map((x) => x.verdict).join("\n"));
  assert.equal(r.rows.filter((x) => /excluded/.test(x.verdict)).map((x) => x.name).join(), "QUOTRONS");
  assert.equal(r.rows[1].tokenBound, true);
  const t = table(r);
  assert.match(t, /StonkBrokers \(token-bound\)/);
  assert.match(t, /\| in \|/);
  assert.ok(!t.includes("k\""), "the key is never printed");
  await assert.rejects(measureAll(f, {}, NOW), /OPENSEA_API_KEY is not set/);
});

test("every generated wX symbol fits the canister's 10-byte limit (register refuses longer)", () => {
  for (const c of COLLECTIONS) {
    const m = { ...c, slug: c.slug ?? null, floorEth: 1, floorSol: 25, tier: 85, twx: 4, maxPurchaseWei: E, tokenBound: !!c.tokenBound };
    const sym = entries(m, 25, "t").nftPilot.symbol;
    assert.ok(sym.length <= 10 && sym.startsWith("w"), `${c.name}: ${sym}`);
  }
});

test("QUOTRONS is excluded whatever its numbers: an ERC-404 pair", () => {
  const q = COLLECTIONS.find((c) => c.name === "QUOTRONS");
  assert.match(q.excluded, /ERC-404/);
});
