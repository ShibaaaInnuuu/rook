import assert from "node:assert/strict";
import { test } from "node:test";
import {
  coinsText, firstBuyWx, floorLamportsOf, friendlyWx, graduateText, holdText, otherUnitText, plainAmount, progressLabel, reviewFirstBuy, reviewTotal, reviewWx,
  solApprox, solToWx, solWithTokens, tokensText, tokOf, wxToSol, wxUnits,
} from "../src/rook/sol/firstBuy";
import { buyProblem, coinProblems, draftBuyUnits, EMPTY_DRAFT, loadDraft, saveDraft, DRAFT_KEY, switchBuyUnit, type Draft } from "../src/rook/sol/launchWizard";
import { firstBuyNote } from "../src/ui/LaunchCosts";

/** The first buy typed in SOL or wX, and the wrapped token in plain words (src/rook/sol/firstBuy.ts). */
const AZUKI = 13_710_000_000n; // collections.ts floorSol 13.71
const az = tokOf("azuki");
const draft: Draft = { ...EMPTY_DRAFT, collection: "azuki", name: "Frog Coin", symbol: "FROG" };
const img = { dataUrl: "data:image/png;base64,iVBORw0KGgo=", type: "image/png", size: 50_000, width: 512, height: 512, name: "a.png" };

test("the recorded floor: collections.ts floorSol in lamports; none for ROOK or an unknown slug", () => {
  assert.equal(floorLamportsOf("azuki"), AZUKI);
  assert.equal(floorLamportsOf(""), null);
  assert.equal(floorLamportsOf("nope"), null);
  assert.deepEqual(az, { name: "Azuki", symbol: "wAZUKI", floor: AZUKI });
  assert.deepEqual(tokOf("rook"), { name: "", symbol: "SOL", floor: null });
});

test("SOL → wX at the floor, rounded down to 9 decimals; wX → SOL at the floor", () => {
  assert.equal(solToWx(27_420_000_000n, AZUKI), 2_000_000_000n, "27.42 SOL is exactly 2 wAZUKI");
  assert.equal(solToWx(27_400_000_000n, AZUKI), 1_998_541_210n, "1.998541210[9…] rounds down");
  assert.equal(solToWx(1n, AZUKI), 0n, "a lamport is below the smallest wX unit");
  assert.equal(wxToSol(2_000_000_000n, AZUKI), 27_420_000_000n);
  assert.equal(wxToSol(500_000_000n, AZUKI), 6_855_000_000n);
  assert.equal(firstBuyWx(27_420_000_000n, "SOL", AZUKI), 2_000_000_000n);
  assert.equal(firstBuyWx(2_000_000_000n, "wX", AZUKI), 2_000_000_000n);
  assert.equal(firstBuyWx(5n, "SOL", null), 5n, "no floor (ROOK's SOL curve): as typed");
  assert.equal(draftBuyUnits({ ...draft, buy: "27.42", buyUnit: "SOL" }), 2_000_000_000n);
  assert.equal(draftBuyUnits({ ...draft, buy: "2", buyUnit: "wX" }), 2_000_000_000n);
  assert.equal(draftBuyUnits({ ...draft, buy: "lots", buyUnit: "SOL" }), null);
});

test("friendly wording: SOL rounded, the token in plain words, wX as the precise label", () => {
  assert.equal(solApprox(27_420_000_000n), "27.4");
  assert.equal(solApprox(95_970_000_000n), "96");
  assert.equal(solApprox(295_000_000_000n), "295");
  assert.equal(solApprox(137_100_000n), "0.1371");
  assert.deepEqual(friendlyWx(az, 2_000_000_000n), { plain: "2 Azuki tokens", label: "2 wAZUKI", sol: "≈ 27.4 SOL" });
  assert.deepEqual(friendlyWx(az, 1_000_000_000n), { plain: "1 Azuki token", label: "1 wAZUKI", sol: "≈ 13.7 SOL" }, "singular");
  assert.equal(friendlyWx(az, 500_000_000n).plain, "0.5 Azuki tokens");
  assert.deepEqual(friendlyWx(tokOf("pudgypenguins"), wxUnits(5)), { plain: "5 Pudgy Penguins tokens", label: "5 wPUDGY", sol: "≈ 295 SOL" });
  assert.deepEqual(friendlyWx(tokOf("rook"), 2_000_000_000n), { plain: "2 SOL", label: "2 SOL", sol: null });
  assert.equal(tokensText(friendlyWx(az, 2_000_000_000n)), "2 Azuki tokens · 2 wAZUKI");
  assert.equal(solWithTokens(az, 2_000_000_000n), "≈ 27.4 SOL (2 Azuki tokens · 2 wAZUKI)");
  assert.deepEqual(graduateText(az, 7), { main: "≈ 96 SOL", label: "7 wAZUKI" });
  assert.equal(progressLabel("pudgypenguins"), "Progress to graduation (5 Pudgy Penguins tokens ≈ 295 SOL)");
  assert.equal(progressLabel("rook"), "Progress to 85 SOL");
});

test("the live line in the other unit says it is at the recorded floor", () => {
  assert.equal(otherUnitText(27_420_000_000n, "SOL", az), "27.4 SOL, paid as 2 wAZUKI from your wallet (at the recorded floor of 13.71 SOL)");
  assert.equal(otherUnitText(27_400_000_000n, "SOL", az), "27.4 SOL, paid as 1.9985 wAZUKI from your wallet (at the recorded floor of 13.71 SOL)");
  assert.equal(otherUnitText(2_000_000_000n, "wX", az), "≈ 27.4 SOL at the recorded floor of 13.71 SOL");
  assert.equal(otherUnitText(10_000_000n, "wX", az), "≈ 0.1371 SOL at the recorded floor of 13.71 SOL", "4 decimals under 1 SOL");
  assert.equal(otherUnitText(0n, "SOL", az), null);
  assert.equal(otherUnitText(1n, "SOL", tokOf("rook")), null);
});

test("switching the unit converts the typed amount, so the buy stays about the same", () => {
  const d = switchBuyUnit({ ...draft, buy: "27.42", buyUnit: "SOL" }, "wX");
  assert.deepEqual([d.buy, d.buyUnit], ["2", "wX"]);
  const back = switchBuyUnit(d, "SOL");
  assert.deepEqual([back.buy, back.buyUnit], ["27.42", "SOL"]);
  assert.deepEqual(switchBuyUnit({ ...draft, buy: "", buyUnit: "SOL" }, "wX"), { ...draft, buy: "", buyUnit: "wX" });
  assert.equal(plainAmount(1_998_541_210n), "1.99854121");
});

test("the bound is the curve's threshold in wX, also said in SOL; typing in SOL converts first", () => {
  const q = { amount: 7, symbol: "wAZUKI" };
  assert.equal(buyProblem("95.97", q, "SOL", AZUKI), null, "95.97 SOL = 7 wAZUKI");
  assert.equal(buyProblem("95.98", q, "SOL", AZUKI), `The first buy can't be more than the 7 wAZUKI the curve holds (≈ 96 SOL at the recorded floor).`);
  assert.match(buyProblem("8", q, "wX", AZUKI)!, /7 wAZUKI the curve holds \(≈ 96 SOL/);
  assert.match(buyProblem("lots", q, "SOL", AZUKI)!, /amount of SOL, like 0.5/);
  assert.match(buyProblem("lots", q, "wX", AZUKI)!, /amount of wAZUKI/);
  assert.match(buyProblem("0.000000001", q, "SOL", AZUKI)!, /less than the smallest amount of wAZUKI/);
  assert.equal(coinProblems({ ...draft, buy: "27.42" }, img).buy, undefined, "SOL is the default unit");
  assert.match(coinProblems({ ...draft, buy: "200" }, img).buy!, /7 wAZUKI the curve holds/);
  assert.match(coinProblems({ ...draft, buy: "8", buyUnit: "wX" }, img).buy!, /7 wAZUKI/);
});

test("the review: SOL is the figure, the tokens under it", () => {
  assert.deepEqual(reviewFirstBuy(az, 2_000_000_000n),
    { main: "≈ 27.4 SOL", sub: "2 Azuki tokens · 2 wAZUKI · from your wallet", text: "≈ 27.4 SOL (2 Azuki tokens · 2 wAZUKI) from your wallet" });
  assert.equal(reviewFirstBuy(az, 0n).text, "none");
  assert.equal(reviewFirstBuy(tokOf("rook"), 500_000_000n, true).text, "0.5 SOL");
  assert.deepEqual(reviewTotal(az, 27_600_000n, 2_000_000_000n),
    { main: "≈ 0.0276 SOL", sub: "+ first buy ≈ 27.4 SOL (2 wAZUKI)", text: "≈ 0.0276 SOL, plus your first buy of ≈ 27.4 SOL (2 Azuki tokens · 2 wAZUKI)" });
  assert.deepEqual(reviewTotal(az, 27_600_000n, 0n), { main: "≈ 0.0276 SOL", sub: null, text: "≈ 0.0276 SOL" });
  assert.deepEqual(reviewWx(az, wxUnits(3.5)), { main: "≈ 48 SOL", sub: "3.5 Azuki tokens · 3.5 wAZUKI", text: "≈ 48 SOL (3.5 Azuki tokens · 3.5 wAZUKI)" });
  assert.equal(holdText(az, 500_000_000n), "You hold ≈ 6.9 SOL (0.5 Azuki tokens · 0.5 wAZUKI)");
  assert.equal(firstBuyNote("the normal 1 % fee (no anti-sniper fee)", 300), "The first buy pays the normal 1 % fee (no anti-sniper fee) and allows up to 3 % slippage.");
});

test("what the buy gets: coins and share of the supply", () => {
  assert.equal(coinsText(55_172_412_000_000n, 6, 1_000_000_000_000_000n), "≈ 55,172,412 coins (5.52 % of the supply)");
  assert.equal(coinsText(199_999_993_000_000n, 6, 1_000_000_000_000_000n), "≈ 199,999,993 coins (20 % of the supply)");
  assert.equal(coinsText(1_500_000n, 6, null), "≈ 1.5 coins");
});

test("drafts keep the unit; a draft saved before it (its amount in wX) loads as wX", () => {
  const m = new Map<string, string>();
  const s = { getItem: (k: string) => m.get(k) ?? null, setItem: (k: string, v: string) => { m.set(k, v); }, removeItem: (k: string) => { m.delete(k); } };
  saveDraft(s, { ...draft, buy: "27.42", buyUnit: "SOL" }, null);
  assert.deepEqual(loadDraft(s).draft, { ...draft, buy: "27.42", buyUnit: "SOL" });
  saveDraft(s, { ...draft, buy: "2", buyUnit: "wX" }, null);
  assert.equal(loadDraft(s).draft.buyUnit, "wX");
  const { buyUnit: _u, ...old } = { ...draft, buy: "2" };
  m.set(DRAFT_KEY, JSON.stringify(old));
  assert.deepEqual(loadDraft(s).draft, { ...draft, buy: "2", buyUnit: "wX" });
  assert.equal(draftBuyUnits(loadDraft(s).draft), 2_000_000_000n, "the old 2 wAZUKI stays 2 wAZUKI");
  m.set(DRAFT_KEY, JSON.stringify({ ...draft, buyUnit: "BTC" }));
  assert.equal(loadDraft(s).draft.buyUnit, "wX");
  m.delete(DRAFT_KEY);
  assert.equal(loadDraft(s).draft.buyUnit, "SOL", "a new draft types in SOL");
});
