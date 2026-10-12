import assert from "node:assert/strict";
import { existsSync } from "node:fs";
import { test } from "node:test";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { SOLANA_SITE_CONFIG } from "../src/config/solana";
import { readRookRow, siteRefsOf } from "../src/rook/sol/board";
import * as Demo from "../src/rook/sol/demo";
import { demoMarket } from "../src/rook/sol/demoMarket";
import { columnsOf, filterCoins, topByMcapOf, trendingOf } from "../src/rook/sol/market";
import { loadSolanaConfig, type RawSiteConfig } from "../src/rook/sol/siteConfig";
import type { Rpc } from "../src/rook/sol/rpc";
import { GenesisCard, ROOK_IMAGE } from "../src/ui/GenesisCard";

/** ROOK, the genesis coin, on the home page: the card before launch, once live, and in the demo; and ROOK on the board. */

const MINT = Demo.DEMO_SOL.rook!.mint;
const render = (p: Parameters<typeof GenesisCard>[0]) => renderToStaticMarkup(createElement(GenesisCard, p));
const rookOf = (coins: ReturnType<typeof demoMarket>) => coins.find((c) => c.mint === MINT)!;

test("the card before launch (no mint): 'Launches soon', ROOK's picture and wording, no coin link, no figures", () => {
  const html = render({ mint: null, coin: null, tradeHref: null, docsHref: "#/docs/what-is-rook" });
  assert.match(html, /Launches soon/);
  assert.match(html, /\$ROOK/);
  assert.ok(html.includes(`src="${ROOK_IMAGE}"`), "the site's own copy of ROOK's picture");
  assert.match(html, /trades against SOL/);
  assert.match(html, /graduates at 85 SOL/);
  assert.match(html, /no NFT backing/);
  assert.ok(!html.includes("#/coin"), "no link to a coin page");
  assert.ok(!/<a[^>]*>Trade \$ROOK<\/a>/.test(html), "no trade link");
  assert.ok(!html.includes("Bonding curve"), "no figures before launch");
  assert.ok(existsSync(new URL(`../public${ROOK_IMAGE}`, import.meta.url)), "the picture is in public/");
});

test("the card once ROOK's mint is set: the Trade $ROOK link to its coin page, its price, market cap and curve progress", () => {
  const coin = rookOf(demoMarket());
  const html = render({ mint: MINT, coin, tradeHref: `#/coin/${MINT}`, docsHref: "#/docs/what-is-rook" });
  assert.ok(html.includes(`<a class="btn btn--primary" href="#/coin/${MINT}">Trade $ROOK</a>`), html);
  assert.ok(!html.includes("Launches soon"));
  assert.match(html, /Live/);
  assert.match(html, /Price/);
  assert.match(html, /Market cap/);
  assert.match(html, new RegExp(`${Math.round(coin.curve!.progress * 100)}% · [0-9.]+ / 85 SOL`));
  // set but not yet read: the link is there, the figures wait
  const bare = render({ mint: MINT, coin: null, tradeHref: `#/coin/${MINT}`, docsHref: "#/docs", loading: true });
  assert.match(bare, /Trade \$ROOK<\/a>/);
  assert.match(bare, /Reading the curve/);
});

test("demo: ROOK is a demo coin (SOL-quoted, on the curve) and the card shows it, marked Demo", () => {
  const coins = demoMarket();
  const rook = rookOf(coins);
  assert.ok(rook, "the demo board has ROOK");
  assert.equal(rook.symbol, "ROOK");
  assert.equal(rook.collection.slug, "rook");
  assert.equal(rook.quote.symbol, "SOL");
  assert.notEqual(rook.phase, "graduated");
  assert.match(Demo.demoOffChain(`demo:${MINT}`)?.description ?? "", /Demo coin/);
  assert.equal(Demo.demoOffChain(`demo:${MINT}`)?.image, ROOK_IMAGE, "the demo's ROOK wears ROOK's real picture");
  const html = render({ mint: MINT, coin: rook, tradeHref: `?demo#/coin/${MINT}`, docsHref: "#/docs", demo: true });
  assert.match(html, /Demo/);
  assert.match(html, /Trade \$ROOK<\/a>/);
});

test("ROOK is on the board like any coin: the columns, the Genesis (ROOK) filter, the search, trending and top market cap", () => {
  const coins = demoMarket();
  const cols = columnsOf(coins);
  assert.equal([...cols.fresh, ...cols.soon, ...cols.done].filter((c) => c.mint === MINT).length, 1);
  assert.deepEqual(filterCoins(coins, { collection: "rook" }).map((c) => c.mint), [MINT]);
  assert.ok(filterCoins(coins, { q: "rook" }).some((c) => c.mint === MINT));
  assert.ok(filterCoins(coins, { q: MINT.toLowerCase() }).some((c) => c.mint === MINT));
  assert.ok(trendingOf(coins, coins.length).some((c) => c.mint === MINT));
  assert.ok(topByMcapOf(coins, coins.length).some((c) => c.mint === MINT));
});

// ---- the site config: ROOK alone is a complete config (canister-only Rook) ---------------------------------------------------------

const A = (n: number) => Demo.demoAddress(`genesis-test:${n}`);
const withRook = (): RawSiteConfig => { const c = structuredClone(SOLANA_SITE_CONFIG) as RawSiteConfig; c.rook = { config: A(1), mint: A(2), pool: A(3) }; return c; };

test("ROOK's three addresses alone (the canister names the collections' configs) make a ready config whose board reads ROOK", () => {
  const c = loadSolanaConfig(withRook());
  assert.deepEqual(c.problems, []);
  assert.equal(c.ready, true);
  assert.deepEqual(c.rook, { config: A(1), mint: A(2), pool: A(3) });
  assert.deepEqual(siteRefsOf(c), [{ config: A(1), slug: "rook", label: "Genesis (ROOK)" }]);
  // the placeholder: nothing
  assert.deepEqual(siteRefsOf(loadSolanaConfig(SOLANA_SITE_CONFIG as RawSiteConfig)), []);
  // half-written: part of ROOK, or ROOK and some of the collections
  const part = withRook(); part.rook.pool = "";
  assert.match(loadSolanaConfig(part).problems.join(" "), /half-written/);
  assert.equal(loadSolanaConfig(part).ready, false);
  const some = withRook(); some.collections[0].config = A(4);
  assert.match(loadSolanaConfig(some).problems.join(" "), /half-written/);
  assert.deepEqual(siteRefsOf(loadSolanaConfig(some)), []);
});

test("readRookRow: nothing unless ROOK's pool and config are DBC accounts", async () => {
  const none = { accounts: async (keys: string[]) => keys.map(() => null) } as unknown as Rpc;
  assert.equal(await readRookRow(none, { config: A(1), mint: A(2), pool: A(3) }), null);
  const other = { accounts: async (keys: string[]) => keys.map(() => ({ owner: A(9), data: new Uint8Array(500), lamports: 1 })) } as unknown as Rpc;
  assert.equal(await readRookRow(other, { config: A(1), mint: A(2), pool: A(3) }), null);
});
