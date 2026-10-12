import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { curveLeg, EXCLUDED_IS, holdersCaveat, money, recordingRanOut, sharePct, tradeRows, usdOf } from "../src/lib/book.js";
import { explorerFor, EXPLORER_ADDRESS, EXPLORER_TX, EXPLORERS } from "../src/lib/explorer.js";
import { robinhood, STAGING_ID, local } from "../src/lib/chains.js";
import { fixtureSource } from "../src/api/source.js";
import type { HolderPage, TradePage } from "../src/api/types.js";

/**
 * docs/phase13.md. The panel under the chart: the tape, the holder list, and the four unknowns they have to keep apart.
 */

const here = (p: string) => new URL(p, import.meta.url);
const src = (name: "demo" | "synthetic") => fixtureSource(name, Math.floor(Date.now() / 1000), async (f) => JSON.parse(readFileSync(here(`../public/fixtures/${name}/${f}`), "utf8")));
const panel = readFileSync(here("../src/components/TradesHolders.tsx"), "utf8");
const css = readFileSync(here("../src/styles/etch.css"), "utf8");

test("a share of supply is a percentage, and an unread supply has none rather than a zero", () => {
  assert.equal(sharePct(0.15206112), "15.2%");
  assert.equal(sharePct(0.0285714), "2.86%");
  assert.equal(sharePct(0.000004), "0.00040%");
  assert.equal(sharePct(0), "0%");
  assert.equal(sharePct(null), null, "null in, null out: the page prints 'not known', not '0%'");
  assert.equal(sharePct(undefined), null);
  assert.equal(sharePct(Number.NaN), null, "and a NaN never reaches a screen as 'NaN%'");
});

test("a trade's value in dollars exists only when the rate does", () => {
  assert.equal(usdOf("1000000000000000000", 2_785.14), 2_785.14);
  assert.equal(usdOf("17228400000000000", 2_785.14)!.toFixed(2), "47.98", "the measured staging buy of 0.0172 ETH");
  assert.equal(usdOf("1000000000000000000", null), null, "no rate is not a value of nothing");
  assert.equal(usdOf("1000000000000000000", Number.NaN), null);
  assert.deepEqual([money(47.98), money(1_240.4), money(2_400_000), money(0.004), money(null)], ["$47.98", "$1,240", "$2.40M", "<$0.01", null]);
});

test("the tape's rows keep the indexer's order and carry both units", () => {
  const trade = { id: "1:0", side: "buy", venue: "curve", quoteWei: "1000000000000000000", coinWei: "1000000000000000000000", feeWei: null, taxWei: null,
    price: 1e-9, trader: "0x" + "ab".repeat(20), recipient: null, tx: "0x" + "cd".repeat(32), block: 1, logIndex: 0, at: 1 } as const;
  const rows = tradeRows([{ items: [{ ...trade }, { ...trade, id: "2:0" }] }, { items: [{ ...trade, id: "3:0" }] }] as never, 2_000);
  assert.deepEqual(rows.map((r) => r.id), ["1:0", "2:0", "3:0"], "pages are appended in the order they were read, newest first");
  assert.deepEqual(rows.map((r) => r.usd), [2_000, 2_000, 2_000]);
  assert.equal(tradeRows([{ items: [trade] }] as never, null)[0].usd, null);
});

test("the explorer: Robinhood Chain has one, our own forks do not, and a recording of a fork never gets a link", () => {
  assert.equal(EXPLORERS[robinhood.id], "https://explorer.mainnet.chain.robinhood.com",
    "measured 2026-09-21: this host answers 301 to robinhoodchain.blockscout.com");
  assert.equal(explorerFor(robinhood.id), "https://explorer.mainnet.chain.robinhood.com");
  assert.equal(explorerFor(STAGING_ID), null, "staging is a chain we run: it has no explorer, so no link is drawn");
  assert.equal(explorerFor(local.id), null);
  assert.equal(explorerFor(robinhood.id, true), null, "a recorded demo is a recording of the fork, whatever chain the app thinks it is on");
  assert.equal(explorerFor(null), null);
  assert.equal(EXPLORER_TX("https://x", "0xab"), "https://x/tx/0xab");
  assert.equal(EXPLORER_ADDRESS("https://x", "0xab"), "https://x/address/0xab");
  assert.ok(!/basescan/.test(panel), "the trade row used to link every transaction to Base's explorer");
  assert.ok(!/basescan/.test(readFileSync(here("../src/components/feeds.tsx"), "utf8")));
});

test("the four unknowns are four different sentences, and staging says its wallets are simulated", () => {
  assert.ok(recordingRanOut(new Error("this demo has no recorded answer for /v1/coins/0x/trades")));
  assert.ok(!recordingRanOut(new Error("the indexer answered 503 for /v1/coins/0x/trades")));
  assert.match(holdersCaveat(true), /SIMULATED wallet/, "on staging the panel says the addresses are the simulator's");
  assert.match(holdersCaveat(false), /One address, one row/);
  assert.ok(!/SIMULATED/.test(holdersCaveat(false)), "off staging it does not claim they are simulated");
  assert.match(panel, /has never traded/);                       // a coin with no trades at all
  assert.match(panel, /holds no more of this coin's/);           // a recording that ran out of pages
  assert.match(panel, /still being built/);                      // a holder scan that has not finished
  assert.match(panel, /Nobody holds this coin yet/);             // launched, never held
  assert.match(panel, /<Failed what=/);                          // an indexer that would not answer
  assert.match(panel, /holdersCaveat\(staging\)/);
  assert.match(panel, /On staging every trade is a simulated wallet's/);
  for (const why of ["coin", "curve", "router", "pons-factory", "pool-manager", "hook"]) assert.ok(EXCLUDED_IS[why as keyof typeof EXCLUDED_IS], why);
  assert.match(EXCLUDED_IS["pons-factory"], /2\/7/, "the measured share Pons holds of every supply");
});

test("the panel is one section under the chart, its side is a word as well as a colour, and a wide table scrolls inside its own box", () => {
  const coin = readFileSync(here("../src/pages/Coin.tsx"), "utf8");
  assert.match(coin, /<TradesHolders coin=/);
  assert.ok(!/TradeFeed/.test(coin), "the old Trades section is folded into the panel, not left beside it");
  assert.ok(!/Section title="Trades"/.test(coin));
  assert.match(panel, /tag tag--\$\{t\.side\}/);
  assert.match(panel, /\{t\.side\}<\/span>/, "the side is printed as a word, so it never depends on colour alone");
  assert.match(css, /\.book__scroll \{ overflow-x: auto/);
  assert.ok(!/\.ledger\b/.test(readFileSync(here("../src/components/TradesHolders.tsx"), "utf8")), "the panel's tables do not borrow the .ledger class the Docs page already owns");
  assert.match(css, /@media \(max-width: 40rem\)/, "and at a phone's width the table fits without scrolling at all");
  assert.ok(!/#[0-9a-f]{3,6}\b/i.test(panel), "no colour is written down in the panel: they are all tokens from etch.css");
  assert.ok(!/\.tape__row/.test(css), "the old tape's styles went with it");
});

test("the synthetic set: a recorded tape and a recorded holder list, under the paths the site asks for", async () => {
  const s = await src("synthetic");
  const coins = await s.get<{ items: { address: string; holders: number | null }[] }>("/v1/coins");
  // the story's first coin (indexer/test/story.ts COIN): the synthetic set reads no name or symbol, because it has no chain
  const traded = coins.items.find((c) => c.address.toLowerCase() === `0x${"c0".repeat(20)}`)!;
  assert.equal(traded.holders, 2, "the count is on the summary too, so a card needs no second request");
  const page = await s.get<TradePage>(`/v1/coins/${traded.address}/trades?limit=25`);
  assert.ok(page.items.length > 0, "the demo's tape is recorded now: the coin page shows trades");
  assert.equal(page.next, null, "and the last page says so, so 'Show more' is not offered past the recording");

  const holders = await s.get<HolderPage>(`/v1/coins/${traded.address}/holders?limit=25`);
  assert.equal(holders.holders, 2);
  assert.equal(holders.total, holders.items.length);
  assert.deepEqual(holders.items.map((h) => h.rank), [1, 2], "ranked by the indexer, largest first");
  assert.ok(holders.items[0].isCreator, "the story's coin was launched by the address that holds most of it");
  assert.equal(holders.supplyWei, null, "the synthetic set never read totalSupply()");
  assert.deepEqual(holders.items.map((h) => sharePct(h.share)), [null, null], "so the panel says the share is not known");
  assert.deepEqual(holders.excluded.map((x) => x.why), ["curve"], "the curve holds 400,000 of it and is not a holder");
  assert.ok(!holders.items.some((h) => holders.excluded.some((x) => x.address === h.address)), "nothing excluded is also counted");
  await assert.rejects(() => s.get(`/v1/coins/${traded.address}/trades?limit=25&before=1%3A0`), /no recorded answer/,
    "a cursor the recording does not hold is an error the panel prints as 'this recording stops here'");
});

test("?demo=sample end to end: every recorded coin's tape pages to its end, and its holders add up", async () => {
  const s = await src("demo");
  const coins = (await s.get<{ items: { address: string; symbol: string; holders: number | null; supplyWei: string | null }[] }>("/v1/coins?limit=200")).items;
  assert.equal(coins.length, 24);

  let traded = 0, held = 0, pagesFollowed = 0, tradesSeen = 0;
  for (const c of coins) {
    // the tape, exactly as the site walks it: page one, then `next` back as `before`, until the recording says there is no more
    let path = `/v1/coins/${c.address}/trades?limit=25`, rows = 0, total = -1;
    for (let guard = 0; ; guard++) {
      const p = await s.get<TradePage>(path);
      if (total < 0) total = p.total;
      rows += p.items.length;
      assert.ok(p.items.every((t) => t.side === "buy" || t.side === "sell"), `${c.symbol}: a side on every trade`);
      assert.ok(p.items.every((t) => BigInt(t.quoteWei) > 0n && BigInt(t.coinWei) > 0n), `${c.symbol}: both legs of every trade`);
      if (!p.next) break;
      assert.ok(guard < 12, `${c.symbol}: a tape that never ends`);
      path = `/v1/coins/${c.address}/trades?limit=25&before=${encodeURIComponent(p.next)}`;
      pagesFollowed++;
    }
    assert.equal(rows, total, `${c.symbol}: the recorded pages hold the whole tape (${rows} of ${total})`);
    tradesSeen += rows;
    if (rows > 0) traded++;

    const h = await s.get<HolderPage>(`/v1/coins/${c.address}/holders?limit=25`);
    assert.equal(h.complete, true, `${c.symbol}: the recording was made by an indexer that had read every transfer`);
    assert.equal(h.holders, h.total);
    if (h.total > 0) held++;
    const ex = new Set(h.excluded.map((x) => x.address));
    assert.ok(!h.items.some((x) => ex.has(x.address)), `${c.symbol}: nothing excluded is also counted`);
    // a young coin's curve holds most of the supply; a curve that has sold out holds exactly nothing and loses its row, so
    // what is asserted is that the plumbing is named and excluded, not which piece of it happens to hold something
    assert.ok(h.excluded.length > 0, `${c.symbol}: some of this coin sits in its own plumbing, and it is listed`);
    assert.ok(h.excluded.every((x) => ["coin", "curve", "router", "pons-factory", "pool-manager", "hook"].includes(x.why)), `${c.symbol}: every exclusion has a reason`);
    const share = [...h.items, ...h.excluded].reduce((a, x) => a + (x.share ?? 0), 0);
    assert.ok(share > 0 && share <= 1.000001, `${c.symbol}: the shares of one supply, not more (${share})`);
    assert.ok(h.items.every((x) => sharePct(x.share) !== null), `${c.symbol}: the recording read the supply, so every row has a percentage`);
  }
  assert.ok(traded >= 20, `the demo shows real trades on nearly every coin it keeps: ${traded} of 24, ${tradesSeen} trades in all`);
  assert.ok(held >= 20, `and real holders: ${held} of 24`);
  assert.ok(pagesFollowed >= 20, `and the cursor pages are recorded, not just page one: ${pagesFollowed} follow-on pages`);
});

// ---- the fee that makes price x size differ from the value ------------------------------------------------------------

const aTrade = { id: "1:0", venue: "curve" as const, coinWei: "0", price: 0, trader: "0x0" as `0x${string}`,
  recipient: null, tx: "0x0" as `0x${string}`, block: 1, logIndex: 0, at: null };

test("a buy pays the fee on the way in, so the curve received less than the trader paid", () => {
  const leg = curveLeg({ ...aTrade, side: "buy", quoteWei: "1000", feeWei: "60", taxWei: "40" });
  assert.deepEqual(leg, { curveWei: 900n, feeTaxWei: 100n });
});

test("a sell pays it on the way out, so the curve paid more than the trader received", () => {
  const leg = curveLeg({ ...aTrade, side: "sell", quoteWei: "1000", feeWei: "60", taxWei: "40" });
  assert.deepEqual(leg, { curveWei: 1100n, feeTaxWei: 100n });
});

test("a pool trade has no fee line: the hook charges outside the Swap event and the API does not guess", () => {
  assert.equal(curveLeg({ ...aTrade, venue: "pool", side: "buy", quoteWei: "1000", feeWei: null, taxWei: null }), null);
});

/**
 * The defect this guards against, on a real recorded row. Every trade in the demo fixture is a buy whose price times size
 * comes out BELOW its value, by the fee and the tax, and a page that shows the first two and not the third reads as broken
 * arithmetic. Here the three are asserted to reconcile to the wei, against the recording rather than against a made-up row.
 */
test("every recorded curve trade reconciles: curve leg + fee = the trader's own leg, and price x size is the curve leg", async () => {
  const src = await fixtureSource("demo", Date.now() / 1000, async (f) =>
    JSON.parse(readFileSync(new URL(`../public/fixtures/demo/${f}`, import.meta.url), "utf8")));
  const page = await src.get<TradePage>("/v1/coins/0x458d4F20B672A39635624dB7D59cfB201a8B5097/trades?limit=25");
  const curve = page.items.filter((t) => t.venue === "curve");
  assert.ok(curve.length >= 10, "the recording has curve trades to check");
  for (const t of curve) {
    const leg = curveLeg(t);
    assert.ok(leg !== null, "a curve trade carries its fee and its tax");
    const signed = t.side === "buy" ? leg.curveWei + leg.feeTaxWei : leg.curveWei - leg.feeTaxWei;
    assert.equal(signed, BigInt(t.quoteWei), "the two legs differ by exactly the fee and the tax");
    // and that curve leg over the coins IS the price the row prints, to floating-point tolerance
    const implied = Number(leg.curveWei) / Number(t.coinWei);
    assert.ok(Math.abs(implied - t.price) / t.price < 1e-9, `price ${t.price} vs implied ${implied}`);
  }
});
