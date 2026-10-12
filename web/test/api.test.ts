import assert from "node:assert/strict";
import { existsSync, readFileSync, readdirSync } from "node:fs";
import { test } from "node:test";
import { acquisitionOf, coinOf, items, listingOf, lotOf, purchasesOf, reader, statsOf, vaultOf, REASONS } from "../src/api/adapt.js";
import { fixtureSource, httpSource } from "../src/api/source.js";
import { destinationProblem } from "../src/lib/destination.js";
import { cap, capOf, hasUsd, RANGES, type CandleSet } from "../src/lib/chart.js";

const here = (p: string) => new URL(p, import.meta.url);
/** what the site asks a list endpoint for (web/src/lib/catalog.ts `PAGE`); catalog.ts itself pulls in Vite-only globs, so it is repeated */
const PAGE = 200;
const set = (name: "demo" | "synthetic", now: number) => fixtureSource(name, now, async (f) => JSON.parse(readFileSync(here(`../public/fixtures/${name}/${f}`), "utf8")));

test("the copies are the indexer's own files, byte for byte (checked whenever the repository copy is beside us)", () => {
  if (!existsSync(here("../../indexer/types.ts"))) return;      // a build context that holds web/ alone
  assert.equal(readFileSync(here("../src/api/types.ts"), "utf8"), readFileSync(here("../../indexer/types.ts"), "utf8"));
  for (const name of ["demo", "synthetic", "wrapped"]) for (const f of readdirSync(here(`../../indexer/fixtures/${name}/`)))
    assert.equal(readFileSync(here(`../public/fixtures/${name}/${f}`), "utf8"), readFileSync(here(`../../indexer/fixtures/${name}/${f}`), "utf8"), `${name}/${f}`);
});

test("the recorded demo: every page's reads resolve, amounts stay exact, and times are moved so a countdown is mid-count", async () => {
  const raw = <T,>(f: string) => JSON.parse(readFileSync(here(`../public/fixtures/demo/${f}`), "utf8")) as T;
  const health = raw<{ lastPassAt: number; chainId: number }>("health.json");
  const NOW = health.lastPassAt + 5_000_000, src = await set("demo", NOW), r = reader(src);
  assert.equal(src.kind, "fixtures"); assert.equal(src.shiftS, 5_000_000); assert.match(src.note, /FIXTURE/);
  assert.equal(health.chainId, 9466301, "recorded from the staging fork, which is what the note says");
  const listings = items(await r.listings()).map(listingOf);
  assert.ok(listings.length >= 24, `a busy chain, not one vault: ${listings.length} listings`);
  assert.deepEqual(listings.slice(0, 2).map((l) => [l.id, l.kind]), [[0, "inscriptions"], [1, "rune"]]);
  const l0 = raw<{ live: { spendableWei: string }; capExpiry: number; declarations: { maxEthWei: string }[] }>("listing-0.json");
  const d0 = await r.listing(0), v = vaultOf(d0, src.shiftS);
  assert.equal(v.unknown, undefined); assert.equal(v.spendable, BigInt(l0.live.spendableWei), "wei survive as wei"); assert.equal(v.capExpiry, d0.capExpiry! + 5_000_000);
  assert.equal(v.bondBpsKnown, false, "the API does not report the bond requirement's rate, so the page does not print one");
  const p = purchasesOf(d0, src.shiftS);
  // seventeen digits: a purchase that went through Number on its way here would come back off by a few wei
  assert.equal(p[0].maxEth, BigInt(l0.declarations[0].maxEthWei));
  assert.ok(p[0].maxEth > 10n ** 16n && String(p[0].maxEth).length >= 17, "the exactness above is tested on a number Number cannot hold");
  const a = d0.acquisitionsRecent.map(acquisitionOf); assert.match(a[0].btcTxid, /^0x[0-9a-f]{64}$/);
  const coins = items(await r.coins());
  assert.equal(coins.length, 24, "the recording keeps the 24 most recent coins (indexer/scripts/write-fixtures.ts COINS_KEPT)");
  const c = coinOf(await r.coin(coins[0].address), src.shiftS);
  // 0.70% of volume reaches a router before any creator tax (docs/phase1.md), plus this coin's own tax. The simulator gives
  // its coins different taxes, so this is checked against the coin rather than pinned to whatever the last recording held.
  assert.equal(c.indexed!.takeBps, 70 + c.creatorTaxBps); assert.ok(c.creatorTaxBps > 0 && c.creatorTaxBps <= 1_000);
  assert.ok(c.indexed!.volumeWei !== undefined);
  const drawn = raw<{ items: { id: string }[] }>("draws.json").items;
  assert.ok(drawn.length > 1, "more than one lot has been drawn for");
  const [listingId, acquisitionId] = drawn[0].id.split("-").map(Number);
  const lot = lotOf(await r.draw(listingId, acquisitionId), src.shiftS);
  assert.ok(lot.lot.stage > 0 || lot.distribution.open || lot.distribution.rolled, "a recorded lot is somewhere in its life, not nowhere");
  const s = statsOf(await r.stats());
  assert.ok(s.coins >= 24 && s.listings >= 24 && s.receipts > 1 && s.delivered >= 1, `the totals of a busy chain: ${JSON.stringify(s, (_k, x) => (typeof x === "bigint" ? String(x) : x))}`);
  assert.equal(s.toVaultsWei, BigInt(raw<{ toVaultsWei: string }>("stats.json").toVaultsWei));
  await assert.rejects(() => r.draw(9_000, 9_000), /no recorded answer/, "a path the demo lacks is an error, not an invention");
});

test("?demo=sample end to end: the coin list, every range tab and a card's market cap all come out of the recording", async () => {
  const src = await set("demo", Math.floor(Date.now() / 1000));
  // the paths the site itself asks for: catalog.ts PAGE, then coinchart.ts once a card is on screen
  const page = await src.get<{ items: { address: string; symbol: string }[]; total: number }>(`/v1/coins?limit=${PAGE}`);
  assert.ok(page.items.length >= 12, `a market, not one coin: ${page.items.length} recorded of ${page.total} on the chain`);

  let traded = 0, priced = 0;
  for (const c of page.items) {
    await src.get(`/v1/coins/${c.address}`);                      // the coin's own page
    for (const r of RANGES) {                                     // every tab a reader can click
      const chart = await src.get<CandleSet>(`/v1/coins/${c.address}/candles?range=${r.key}`);
      assert.equal(chart.coin.toLowerCase(), c.address.toLowerCase());
      // MarketChart's series: a candle the page cannot price is dropped, so a chart is only drawable if capOf answers
      const series = chart.candles.map((k) => capOf(chart, k.c, hasUsd(chart) ? "usd" : "pair")).filter((v) => v !== null);
      assert.equal(series.length, chart.candles.length, `${c.symbol} ${r.key}: every candle prices`);
      for (const v of series) assert.ok(Number.isFinite(v!) && v! > 0, `${c.symbol} ${r.key}: a market cap, not NaN`);
    }
    // the card: a day, the last close, in dollars
    const day = await src.get<CandleSet>(`/v1/coins/${c.address}/candles?range=1d`);
    const last = day.candles[day.candles.length - 1];
    if (!last) continue;
    traded++;
    assert.ok(hasUsd(day), `${c.symbol}: the recording carries a supply and a rate, so the card can say dollars`);
    const usd = capOf(day, last.c, "usd")!;
    assert.ok(usd > 1_000 && usd < 1e12, `${c.symbol}: a market cap in dollars, not ${usd}`);
    assert.match(cap(usd, "usd"), /^\$[\d,.]+[KMBT]?$/);
    priced++;
  }
  assert.ok(traded >= 12, `most recorded coins have traded in the last day: ${traded}`);
  assert.equal(priced, traded);

  // a coin the recording does not keep: the reads that back the chart and the card fail with a reason, which is what the
  // page prints. Nothing is invented and nothing throws past the query.
  await assert.rejects(() => src.get(`/v1/coins/0x${"ab".repeat(20)}/candles?range=1d`), /no recorded answer/);
});

test("the synthetic set: unknown stays unknown, and the states the demo run never reaches are all reachable", async () => {
  const src = await set("synthetic", 1_900_000_000), r = reader(src);
  const rune = await r.listing(1), v = vaultOf(rune, src.shiftS);
  assert.equal(v.unknown, REASONS["live-unavailable"], "no live block: the page gets a reason, and prints no balances");
  const open = purchasesOf(rune, src.shiftS).find((x) => x.status === 1)!;
  assert.ok(open.releasableAt > 1_900_000_000 && open.releasableAt <= 1_900_000_000 + 3_600, "an open declaration, inside its public hour");
  const c = coinOf(await r.coin("0xC1C1c1c1C1C1C1c1c1C1C1C1c1C1C1C1c1C1c1c1"), src.shiftS);
  assert.equal(c.indexed!.volumeWei, null); assert.equal(c.indexed!.why, REASONS["live-unavailable"]); assert.ok(c.indexed!.pendingUnknown);
  const states = new Map<string, ReturnType<typeof lotOf>>();
  for (const id of [[0, 0], [0, 1], [0, 2], [0, 4], [1, 0], [1, 9]] as const) states.set(id.join("-"), lotOf(await r.draw(id[0], id[1]), src.shiftS));
  assert.equal(states.get("0-2")!.lot.ended, "voided"); assert.equal(states.get("0-1")!.lot.ended, "cancelled"); assert.equal(states.get("0-4")!.lot.stage, 4);
  assert.deepEqual([states.get("1-0")!.distribution.open, states.get("1-0")!.distribution.rolled], [true, true]);
  assert.equal(states.get("1-9")!.distribution.open, false, "a cancelled distribution is not shown as open");
});

test("the hosted indexer: a bad status is an error with the path in it, and the base URL is used as given", async () => {
  const seen: string[] = [];
  const ok = httpSource("https://api.example", (async (u: string) => { seen.push(u); return { ok: true, status: 200, json: async () => ({ coins: 3 }) }; }) as unknown as typeof fetch);
  assert.deepEqual(await ok.get("/v1/stats"), { coins: 3 }); assert.deepEqual(seen, ["https://api.example/v1/stats"]);
  const bad = httpSource("https://api.example", (async () => ({ ok: false, status: 503, json: async () => ({}) })) as unknown as typeof fetch);
  await assert.rejects(() => bad.get("/v1/coins"), /503 for \/v1\/coins/);
});

test("the Bitcoin address form: empty, wrong checksum, wrong kind and a good taproot address", () => {
  const good = "bc1p0xlxvlhemja6c4dqv22uapctqupfhlxm9h8z3k2e72q4k9hcz7vqzk5jj0";
  assert.equal(destinationProblem(""), "Enter the address."); assert.equal(destinationProblem(good), null); assert.equal(destinationProblem(`  ${good} `), null);
  assert.match(destinationProblem(good.replace("hemja", "hemjq"))!, /will not be accepted/);
  assert.match(destinationProblem("bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4")!, /will not be accepted/);
  assert.match(destinationProblem("0x" + "a".repeat(40))!, /will not be accepted/);
});
