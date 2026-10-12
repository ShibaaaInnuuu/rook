import assert from "node:assert/strict";
import { test } from "node:test";
import {
  EMPTY_DRAFT, MAX_IMAGE_BYTES, buyUnits, buyProblem, cleanTicker, coinProblems, cropHint, imageFileProblem, launchCosts, launchErrorText,
  launchHash, linkProblem, loadDraft, nameProblem, normalizeLink, parseLaunchRoute, reachableStep, rentFor, saveDraft, sniffImage, stepProblem,
  tickerProblem, DRAFT_KEY, IMAGE_KEY, type Draft, type PickedImage, launchBuyFees, launchBuyNote, launchBuyFeeShort, launchBuyQuote,
} from "../src/rook/sol/launchWizard";
import { demoConfigOf } from "../src/rook/sol/demo";
import { floorView, loadFloors } from "../src/rook/sol/floors";
import { uploadCoin, uploadReady, UploadError } from "../src/rook/sol/upload";
import { COLLECTIONS } from "../src/rook/sol/collections";

/** The Launch wizard's validation and flow logic (src/rook/sol/launchWizard.ts), with no browser. */
const known = (s: string) => COLLECTIONS.some((c) => c.slug === s);
const img: PickedImage = { dataUrl: "data:image/png;base64,iVBORw0KGgo=", type: "image/png", size: 50_000, width: 512, height: 512, name: "a.png" };
const good: Draft = { ...EMPTY_DRAFT, collection: "azuki", name: "Frog Coin", symbol: "FROG" };
const PNG_HEAD = Uint8Array.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 13]);

test("image checks: type by magic bytes, at most 2 MB, empty refused; crop hint for non-square or tiny images", () => {
  assert.equal(sniffImage(PNG_HEAD), "image/png");
  assert.equal(imageFileProblem({ type: "image/png", size: 1000 }, PNG_HEAD), null);
  assert.match(imageFileProblem({ type: "image/png", size: 1000 }, new TextEncoder().encode("<svg xmlns=...>"))!, /PNG, JPG, GIF or WebP/, "a renamed SVG is caught by its bytes");
  assert.match(imageFileProblem({ type: "image/svg+xml", size: 1000 })!, /PNG, JPG/);
  assert.equal(imageFileProblem({ type: "image/webp", size: MAX_IMAGE_BYTES }), null);
  assert.match(imageFileProblem({ type: "image/png", size: MAX_IMAGE_BYTES + 1 }, PNG_HEAD)!, /2 MB/);
  assert.match(imageFileProblem({ type: "image/png", size: 0 })!, /empty/);
  assert.equal(cropHint(512, 512), null);
  assert.match(cropHint(1200, 600)!, /square/);
  assert.match(cropHint(100, 100)!, /400×400/);
  assert.equal(cropHint(null, null), null);
});

test("name and ticker: required, byte limits, ticker cleaned to upper case without $ or spaces", () => {
  assert.match(nameProblem(" ")!, /name/);
  assert.equal(nameProblem("Frog Coin"), null);
  assert.match(nameProblem("x".repeat(33))!, /32/);
  assert.match(nameProblem("🐸".repeat(9))!, /32/, "32 bytes, not characters");
  assert.equal(cleanTicker("$fr og"), "FROG");
  assert.match(tickerProblem("")!, /ticker/);
  assert.match(tickerProblem("ABCDEFGHIJK")!, /10/);
  assert.match(tickerProblem("FR<G")!, /letters/);
  assert.equal(tickerProblem("FROG2"), null);
});

test("links are typed the way people type them and stored as https URLs", () => {
  assert.equal(normalizeLink("twitter", "@frog_coin"), "https://x.com/frog_coin");
  assert.equal(normalizeLink("twitter", "https://twitter.com/frog"), "https://x.com/frog");
  assert.equal(normalizeLink("twitter", "x.com/frog/"), "https://x.com/frog");
  assert.equal(normalizeLink("twitter", "not a handle!"), null);
  assert.equal(normalizeLink("telegram", "t.me/frogcoin"), "https://t.me/frogcoin");
  assert.equal(normalizeLink("telegram", "@frogcoin"), "https://t.me/frogcoin");
  assert.equal(normalizeLink("website", "frog.example"), "https://frog.example");
  assert.equal(normalizeLink("website", "http://frog.example/a"), "https://frog.example/a");
  assert.equal(normalizeLink("website", "javascript:alert(1)"), null);
  assert.equal(normalizeLink("website", "nodot"), null);
  assert.equal(normalizeLink("website", ""), "");
  assert.equal(linkProblem("website", ""), null);
  assert.match(linkProblem("telegram", "??")!, /t\.me/);
});

// canister-only Rook: a collection's curve is quoted in its wX and holds its threshold in whole wX (docs/v9/canister-only-solana.md
// §3.2), so the first buy is in wX and bounded by the threshold; ROOK's own SOL curve keeps the 85 SOL bound
test("first buy: empty or 0 is none; an amount in the curve's quote; not more than the threshold the curve holds", () => {
  assert.equal(buyUnits(""), 0n);
  assert.equal(buyUnits("0"), 0n);
  assert.equal(buyUnits("0.5"), 500_000_000n);
  assert.equal(buyUnits("0,25"), 250_000_000n);
  assert.equal(buyUnits("abc"), null);
  assert.equal(buyUnits("-1"), null);
  assert.match(buyProblem("abc")!, /amount of SOL/);
  assert.match(buyProblem("86")!, /85 SOL/);
  assert.equal(buyProblem("85"), null);
  // the curve holds its collection's threshold: 4 wBOREDAPE for Bored Apes, 5 wPUDGY for Pudgy Penguins
  assert.match(buyProblem("abc", { amount: 4, symbol: "wBOREDAPE" })!, /amount of wBOREDAPE, like 0.5/);
  assert.equal(buyProblem("3.5", { amount: 4, symbol: "wBOREDAPE" }), null);
  assert.match(buyProblem("4.1", { amount: 4, symbol: "wBOREDAPE" })!, /4 wBOREDAPE the curve holds/);
  assert.match(buyProblem("6", { amount: 5, symbol: "wPUDGY" })!, /5 wPUDGY/);
  assert.equal(coinProblems({ ...good, collection: "bayc", buy: "4", buyUnit: "wX" }, img).buy, undefined);
  assert.match(coinProblems({ ...good, collection: "bayc", buy: "200", buyUnit: "wX" }, img).buy!, /4 wBOREDAPE/);
  assert.match(coinProblems({ ...good, collection: "milady", buy: "5.5", buyUnit: "wX" }, img).buy!, /5 wMILADY/);
  assert.equal(coinProblems({ ...good, collection: "bitcoin-weirdos", buy: "170", buyUnit: "wX" }, img).buy, undefined, "174 wWEIRDO curve");
});

test("Next is gated per step; a URL never lands past a missing field", () => {
  assert.equal(stepProblem(1, EMPTY_DRAFT, null, known), "Pick a collection.");
  assert.equal(stepProblem(1, { ...EMPTY_DRAFT, collection: "nope" }, null, known), "Pick a collection.");
  assert.equal(stepProblem(1, good, null, known), null);
  assert.equal(stepProblem(2, good, null, known), "Add an image for your coin.");
  assert.equal(stepProblem(2, good, img, known), null);
  assert.match(stepProblem(2, { ...good, twitter: "??" }, img, known)!, /X handle/);
  assert.match(stepProblem(2, { ...good, buy: "lots", buyUnit: "wX" }, img, known)!, /amount of wAZUKI/, "the first buy is in the collection's wX");
  assert.deepEqual(Object.keys(coinProblems(EMPTY_DRAFT, null)).sort(), ["image", "name", "symbol"]);
  assert.equal(reachableStep(3, good, img, known), 3);
  assert.equal(reachableStep(3, good, null, known), 2);
  assert.equal(reachableStep(3, EMPTY_DRAFT, null, known), 1);
  assert.equal(reachableStep(2, { ...EMPTY_DRAFT, collection: "azuki" }, null, known), 2);
});

test("the URL reflects the step: #/launch/<slug>/<n>; a collection's Launch link opens step 2", () => {
  assert.deepEqual(parseLaunchRoute(["launch"], known), { slug: null, step: 1 });
  assert.deepEqual(parseLaunchRoute(["launch", "azuki"], known), { slug: "azuki", step: 2 });
  assert.deepEqual(parseLaunchRoute(["launch", "azuki", "3"], known), { slug: "azuki", step: 3 });
  assert.deepEqual(parseLaunchRoute(["launch", "azuki", "1"], known), { slug: "azuki", step: 1 });
  assert.deepEqual(parseLaunchRoute(["launch", "azuki", "9"], known), { slug: "azuki", step: 2 });
  assert.deepEqual(parseLaunchRoute(["launch", "nope", "3"], known), { slug: null, step: 1 });
  assert.equal(launchHash("azuki", 3), "#/launch/azuki/3");
  assert.equal(launchHash("", 1), "#/launch");
});

test("the draft survives a refresh in sessionStorage, and a blocked or full storage never throws", () => {
  const m = new Map<string, string>();
  const s = { getItem: (k: string) => m.get(k) ?? null, setItem: (k: string, v: string) => { m.set(k, v); }, removeItem: (k: string) => { m.delete(k); } };
  saveDraft(s, { ...good, buy: "0.1" }, img);
  assert.deepEqual(loadDraft(s), { draft: { ...good, buy: "0.1" }, image: img });
  saveDraft(s, good, null);
  assert.equal(m.has(IMAGE_KEY), false);
  m.set(DRAFT_KEY, "{garbage"); m.set(IMAGE_KEY, JSON.stringify({ dataUrl: "javascript:x", size: 1 }));
  assert.deepEqual(loadDraft(s), { draft: EMPTY_DRAFT, image: null });
  const throwing = { getItem: () => { throw new Error("SecurityError"); }, setItem: () => { throw new Error("QuotaExceeded"); }, removeItem: () => { throw new Error("SecurityError"); } };
  assert.doesNotThrow(() => saveDraft(throwing, good, img));
  assert.deepEqual(loadDraft(throwing), { draft: EMPTY_DRAFT, image: null });
  assert.deepEqual(loadDraft(null), { draft: EMPTY_DRAFT, image: null });
  // the creator's links are kept, prefilled or typed; a draft saved without them (2026-10-11 builds) loads with them empty
  saveDraft(s, { ...good, website: "https://mine.example", twitter: "@me", telegram: "t.me/frogs" }, null);
  assert.deepEqual(loadDraft(s).draft, { ...good, website: "https://mine.example", twitter: "@me", telegram: "t.me/frogs" });
  const { website: _w, twitter: _t, ...noLinks } = good;
  m.set(DRAFT_KEY, JSON.stringify(noLinks));
  assert.deepEqual(loadDraft(s).draft, good);
});

test("costs: rent of the five new accounts, Metaplex's 0.01 SOL, a network fee bound; a first buy in wX is on top, in SOL it is in the total", () => {
  assert.equal(rentFor(165), 2_039_280n, "a token account's rent-exempt minimum");
  const none = launchCosts(0n), some = launchCosts(500_000_000n);
  assert.equal(none.firstBuy, 0n); assert.equal(none.firstBuyWx, 0n);
  assert.equal(none.total, none.rent + 10_000_000n + none.networkFee);
  assert.equal(some.rent - none.rent, rentFor(165), "a first buy also opens your token account");
  // a collection's curve is quoted in its wX: the first buy comes from the wallet's wX account, not from its SOL
  assert.equal(some.firstBuyWx, 500_000_000n); assert.equal(some.firstBuy, 0n);
  assert.equal(some.total - none.total, rentFor(165));
  // ROOK's SOL-quoted curve: the first buy is SOL and part of the total
  const sol = launchCosts(500_000_000n, "SOL");
  assert.equal(sol.firstBuy, 500_000_000n); assert.equal(sol.firstBuyWx, 0n);
  assert.equal(sol.total - none.total, 500_000_000n + rentFor(165));
  assert.ok(none.total > 20_000_000n && none.total < 40_000_000n, `about 0.025 SOL, not ${none.total}`);
});

test("the launch buy's fee, from the config: a collection's (first swap at the minimum fee) charges the creator's launch buy 1 %, everyone else the anti-sniper fee; a config without the flag keeps the old warning", () => {
  const baseFee = { cliffFeeNumerator: 500_000_000n, firstFactor: 60, secondFactor: 10n, thirdFactor: 8_166_666n, baseFeeMode: 0 };
  assert.deepEqual(launchBuyFees({ baseFee, firstSwapMinFee: true }), { firstBuyMinFee: true, firstBuyPct: 1, launchPct: 50, endPct: 1, minutes: 10 });
  assert.deepEqual(launchBuyNote({ baseFee, firstSwapMinFee: true }), {
    headline: "Your first buy, made in the launch itself, pays only the normal 1 % fee.",
    detail: "The anti-sniper fee applies to everyone else for the first 10 minutes: 50 % at launch, falling to 1 %.",
  });
  assert.equal(launchBuyFeeShort({ baseFee, firstSwapMinFee: true }), "the normal 1 % fee (no anti-sniper fee)");
  assert.deepEqual(launchBuyNote({ baseFee, firstSwapMinFee: false }), {
    headline: "Heads-up: a buy in the launch pays the anti-sniper fee at its highest, about 50 %.",
    detail: "The fee falls to 1 % over the first 10 minutes, so buying later costs less.",
  });
  assert.equal(launchBuyFeeShort({ baseFee, firstSwapMinFee: false }), "the launch-time anti-sniper fee");
  // the estimate: the demo's collection config has the flag, ROOK's SOL config does not
  const now = 2_000_000_000n;
  assert.equal(launchBuyQuote(demoConfigOf("rook"), 1_000_000_000n, now)!.feeNumerator, 500_000_000n);
  const col = COLLECTIONS.find((c) => c.slug !== "rook")!.slug;
  assert.equal(launchBuyQuote(demoConfigOf(col), 1_000_000_000n, now)!.feeNumerator, 10_000_040n);
});

test("errors are said in plain words", () => {
  assert.match(launchErrorText(new Error("User rejected the request.")), /cancelled.*Nothing was sent/);
  assert.match(launchErrorText(new Error("Simulation failed: Not enough SOL in the wallet for this (amount, rent and fees).")), /Not enough SOL/);
  assert.match(launchErrorText(new Error("Simulation failed: insufficient lamports"), { total: 30_000_000n, balance: 10_000_000n }), /has 0\.01 SOL; this launch needs about 0\.03 SOL/);
  assert.match(launchErrorText(new Error("Simulation failed: Exceeded slippage tolerance")), /price moved/);
  assert.match(launchErrorText(new Error("Simulation failed: custom program error 0x1771")), /dry run.*0x1771/);
  assert.match(launchErrorText(new Error("block height exceeded")), /in time/);
  assert.match(launchErrorText(new TypeError("Failed to fetch")), /Couldn't reach Solana/);
});

test("the upload client: 503 says upload isn't set up, 429 passes the server's words, success returns the uri", async () => {
  const f = { name: "A", symbol: "A", description: "", website: "", twitter: "", telegram: "", collection: "azuki" };
  const answer = (status: number, body: unknown) => (async () => new Response(JSON.stringify(body), { status, headers: { "content-type": "application/json" } })) as unknown as typeof fetch;
  await assert.rejects(uploadCoin("x", f, answer(503, { error: "Image upload isn't set up yet." })), (e: unknown) => e instanceof UploadError && e.code === "not-configured" && /isn't set up yet/.test(e.message));
  await assert.rejects(uploadCoin("x", f, answer(429, { error: "Too many uploads from your connection. Try again in about 12 minutes." })), /12 minutes/);
  await assert.rejects(uploadCoin("x", f, answer(200, { uri: "http://insecure/a.json", image: "https://i" })), /https/);
  await assert.rejects(uploadCoin("x", f, (async () => { throw new TypeError("Failed to fetch"); }) as unknown as typeof fetch), /connection/);
  assert.deepEqual(await uploadCoin("x", f, answer(200, { uri: "https://res.cloudinary.com/c/raw/upload/a.json", image: "https://res.cloudinary.com/c/image/upload/a.png" })), { uri: "https://res.cloudinary.com/c/raw/upload/a.json", image: "https://res.cloudinary.com/c/image/upload/a.png" });
  assert.equal(await uploadReady(answer(200, { ready: false })), false);
  assert.equal(await uploadReady((async () => new Response("<html>", { status: 200, headers: { "content-type": "text/html" } })) as unknown as typeof fetch), null, "the dev server has no /api");
});

test("floors for the collection cards come from the catalogue's recorded set", async () => {
  assert.deepEqual(floorView({ native: "684799000000000000", unit: "wei", eth: 0.684799, at: 1 }), { text: "0.685 ETH", at: 1 });
  assert.deepEqual(floorView({ native: "433069", unit: "sats", eth: 0.13, at: 2 }), { text: "433,069 sats", at: 2 });
  const { readFile } = await import("node:fs/promises");
  const floors = await loadFloors(COLLECTIONS, async (p) => JSON.parse(await readFile(new URL(`../public${p}`, import.meta.url), "utf8")));
  assert.ok(floors.azuki && floors.mayc && floors.bayc && floors["bitcoin-frogs"], JSON.stringify(Object.keys(floors)));
});
