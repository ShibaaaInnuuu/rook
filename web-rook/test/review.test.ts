import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { capText, feeNote, priceFull, priceText, subZeros, volText } from "../src/rook/boardlogic.js";
import { floorAtZero } from "../src/rook/chartScale.js";
import { vaultTiles } from "../src/rook/cataloglogic.js";
import { GLOSSARY, nftCount, nftWord, wrappedLine } from "../src/rook/words.js";
import { bannedIn } from "../src/rook/v8/present.js";
import type { CatalogEntry } from "../src/rook/catalogTypes.js";

/** The design review of 2026-09-27: the numbers, the one vocabulary, the vault strip, the redemption's steps. */
const here = (p: string) => new URL(p, import.meta.url);

test("a tiny price is written with its zeros counted as a subscript, never as an exponent", () => {
  assert.equal(subZeros(2.778e-8), "0.0₇2778");
  assert.equal(priceText(2.778e-8), "0.0₇2778");
  assert.equal(priceText(2.64e-9), "0.0₈264");
  assert.equal(priceText(1e-12), "0.0₁₁1");
  assert.equal(priceText(9.99999e-5), "0.0₃1", "rounding up to a ten-thousandth still counts right");
  assert.equal(priceText(0.00012345), "0.00012345");
  assert.equal(priceText(0.5), "0.5000");
  assert.equal(priceText(null), "—");
  for (const p of [3e-7, 4.2e-11, 7.77e-15]) assert.doesNotMatch(priceText(p), /e-?\d/, `${p}`);
  assert.equal(priceFull(2.778e-8), "0.00000002778");
  assert.match(capText(2.5e-7, null), /^0\.0₆25 ETH$/); assert.equal(capText(0.0042, null), "0.00420 ETH");
  assert.match(volText(String(3n * 10n ** 10n)), /^0\.0₇3 ETH$/, "3e-8 ETH of volume");
});

test("the chart's axis never goes below zero, and keeps room under the lowest candle when it can", () => {
  const range = (minValue: number, maxValue: number) => () => ({ priceRange: { minValue, maxValue } });
  assert.equal(floorAtZero(range(0.5, 40))!.priceRange!.minValue, 0, "the −5.00 ETH case: clamped at 0");
  const r = floorAtZero(range(100, 110))!.priceRange!;
  assert.ok(r.minValue > 0 && r.minValue < 100 && r.maxValue === 110, "a narrow range still shows its move");
  assert.equal(floorAtZero(() => null), null);
});

test("a fee over 5% says where it goes", () => {
  assert.match(feeNote({ kind: "curve", feeBps: 800 }), /^8% of every trade, fixed at launch.*40% of it buys NFTs from the collection's floor into the vault.*dearer to trade/);
  assert.match(feeNote({ kind: "pons", feeBps: 1070 }), /^10\.70% .*creator's tax/);
  assert.doesNotMatch(feeNote({ kind: "curve", feeBps: 100 }), /dearer/);
});

test("one vocabulary: NFTs, ordinals on Bitcoin, and the wrapped token explained in one sentence", () => {
  assert.equal(nftWord(false), "NFTs"); assert.equal(nftWord(true, 1), "ordinal"); assert.equal(nftCount(1, false), "1 NFT"); assert.equal(nftCount(6, true), "6 ordinals");
  assert.equal(wrappedLine("wFROG", true), "wFROG is this collection's wrapped token: one whole wFROG stands for one ordinal held in the vault, and burning one redeems one.");
  assert.deepEqual(GLOSSARY.map(([, doc]) => doc), ["nest", "hoard, custody", "item", "unit, wrapped unit", "ledger", "sweep", "catalogue"]);
});

test("the copy on screen uses the one vocabulary: no nest, hoard, ledger or catalogue in the nav, the footer and the page titles", () => {
  const app = readFileSync(here("../src/App.tsx"), "utf8"), routes = readFileSync(here("../src/rook/routes.ts"), "utf8");
  assert.match(routes, /\["collections", "Collections"\]/); assert.doesNotMatch(routes, /"Nests"/);
  assert.match(routes, /\["launch", "Launch a coin"\]/);
  assert.doesNotMatch(app, />The hoard<|>Nests<|Count the hoard|>Launch</);
  for (const f of ["Home", "Coins", "Coin", "Collection", "Launch", "Reserves", "ReservesEth", "Redeem", "Draw", "Rent", "Bots", "Lost", "Wallet", "Trust", "Docs"]) {
    const src = readFileSync(here(`../src/pages/${f}.tsx`), "utf8").replace(/\/\*\*[\s\S]*?\*\//g, "").replace(/^\s*\/\/.*$/gm, "");
    const shown = [...src.matchAll(/>([^<>{}]+)</g), ...src.matchAll(/"([A-Z][^"]{3,})"/g)].map((m) => m[1]).filter((t) => !/[;=(){}]/.test(t)).join("\n"); // JSX text and capitalised strings, not code
    assert.doesNotMatch(shown, /\b(hoard|nests?|ledger|catalogue)\b/i, `${f}.tsx still shows an old word`);
  }
});

const entry = (slug: string, pics: number): CatalogEntry => ({ slug, rook: { kind: "live", slug, coins: 1, why: "" }, sample: Array.from({ length: 4 }, (_, i) => ({ id: String(i), name: null, url: null, image: i < pics ? `https://x/${i}.png` : null })) }) as unknown as CatalogEntry;
test("in the vault now: one picture per NFT held, only as many as there are pictures for, the rest counted", () => {
  const v = vaultTiles([{ slug: "frogs", name: "Bitcoin Frogs", itemsHeld: 6 }, { slug: "apes", name: "BAYC", itemsHeld: 1 }, { slug: "gone", name: "Gone", itemsHeld: 2 }], [entry("frogs", 3), entry("apes", 4)]);
  assert.equal(v.held, 9); assert.equal(v.tiles.length, 4); assert.equal(v.more, 5);
  assert.deepEqual(v.tiles.map((t) => `${t.slug}/${t.item}`), ["frogs/0", "frogs/1", "frogs/2", "apes/0"]);
  assert.equal(vaultTiles([{ slug: "apes", name: "BAYC", itemsHeld: 0 }], [entry("apes", 4)]).tiles.length, 0, "an empty vault draws nothing");
});

test("the redemption's four steps keep every fact (7 days, 72 hours, 30 days), link the draw page, and use none of the ruled-out words", () => {
  const src = readFileSync(here("../src/pages/Redeem.tsx"), "utf8");
  const steps = src.slice(src.indexOf('<ol className="timeline">'), src.indexOf("Track a ticket by its number"));
  assert.equal((steps.match(/<li>/g) ?? []).length, 7, "four steps and the three refund cases");
  for (const re of [/within 7 days\./, /within <strong>7 days<\/strong> of your burn/, /<strong>72 hours<\/strong> after the delivery deadline/, /<strong>30 days<\/strong> after the deadline/, /no refund can race it/, /tickets resolve in order/, /the round is final/, /href=\{drawTo\}/]) assert.match(steps, re);
  assert.deepEqual(bannedIn(src), [], "the redemption page's source uses none of the words the draw page rules out");
});
