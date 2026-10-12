/**
 * The Launch wizard's first buy in two units, and a collection's wrapped token said in plain words. A collection coin's curve is
 * quoted in the collection's wX, and the first buy spends wX the creator already holds; 1 whole wX is 1 NFT, so it is worth about the
 * collection's recorded floor in SOL (collections.ts `floorSol`: the floor the threshold was set from). The creator may type the amount
 * in SOL (the default) or in wX; the transaction always uses wX, converted at the recorded floor and rounded DOWN to the wX's 9
 * decimals. Every SOL figure here is an estimate at that floor, not a live price. Wherever a person decides something, SOL is the main
 * figure, the token is "2 Azuki tokens" and the wX symbol ("2 wAZUKI") is the small precise label (`friendlyWx`). ROOK's SOL-quoted
 * curve (no collection) has no conversion. Pure: test/firstbuy.test.ts is its spec.
 */
import { collectionBySlug } from "./collections";
import { solText, units } from "./format";
import { ROOK_GRADUATION_SOL, thresholdOf } from "./graduation";

export type BuyUnit = "SOL" | "wX";
const ONE = 1_000_000_000n;

/** The collection's recorded floor in lamports per whole wX; null for ROOK and anything unknown (its curve is in SOL). */
export function floorLamportsOf(slug: string | null | undefined): bigint | null {
  const c = collectionBySlug(slug);
  return c && c.floorSol > 0 ? BigInt(Math.round(c.floorSol * 1e9)) : null;
}
/** SOL (lamports) → wX base units at the floor, rounded down. */
export const solToWx = (lamports: bigint, floor: bigint): bigint => (lamports * ONE) / floor;
/** wX base units → SOL (lamports) at the floor, rounded down. */
export const wxToSol = (wx: bigint, floor: bigint): bigint => (wx * floor) / ONE;

/** A SOL estimate, rounded: "296" from 100 SOL up, "27.4" from 1 SOL, "0.1371" below. */
export function solApprox(lamports: bigint): string {
  const n = Number(lamports) / 1e9;
  return n.toLocaleString("en-US", { maximumFractionDigits: n >= 100 ? 0 : n >= 1 ? 1 : 4 });
}
/** "2", "1.9985": a wX amount, up to 4 decimals. */
export const wxAmount = (wx: bigint): string => units(wx, 9, 4);
/** "13.71": the recorded floor as collections.ts holds it. */
export const floorText = (floor: bigint): string => solText(floor, 4);
/** Whole wX (a threshold, a backing share) → base units. */
export const wxUnits = (whole: number): bigint => BigInt(Math.round(whole * 1e9));

/** A collection's wrapped token as the page says it: its collection's name, its wX symbol, its recorded floor. */
export type Tok = { name: string; symbol: string; floor: bigint | null };
export function tokOf(slug: string | null | undefined): Tok {
  const c = collectionBySlug(slug);
  return c ? { name: c.name, symbol: c.wxSymbol, floor: floorLamportsOf(slug) } : { name: "", symbol: "SOL", floor: null };
}
/** An amount of wX said three ways: `plain` "2 Azuki tokens", `label` "2 wAZUKI", `sol` "≈ 27.4 SOL" (null without a floor). */
export type Friendly = { plain: string; label: string; sol: string | null };
export function friendlyWx(t: Tok, wx: bigint): Friendly {
  const n = wxAmount(wx);
  return { plain: t.name ? `${n} ${t.name} token${wx === ONE ? "" : "s"}` : `${n} ${t.symbol}`, label: `${n} ${t.symbol}`, sol: t.floor ? `≈ ${solApprox(wxToSol(wx, t.floor))} SOL` : null };
}
/** "2 Azuki tokens · 2 wAZUKI": the plain words with the precise label. */
export const tokensText = (f: Friendly): string => (f.plain === f.label ? f.label : `${f.plain} · ${f.label}`);

/** The amount the transaction uses, in the curve's quote base units, from what was typed (`typed`, 9 decimals) in `unit`. */
export function firstBuyWx(typed: bigint, unit: BuyUnit, floor: bigint | null): bigint {
  return unit === "SOL" && floor ? solToWx(typed, floor) : typed;
}

/** The live line under the field, in the other unit: "≈ 2 Azuki tokens (2 wAZUKI) at the recorded floor of 13.71 SOL" or "≈ 27.4 SOL at …". */
export function otherUnitText(typed: bigint, unit: BuyUnit, t: Tok): string | null {
  if (!t.floor || typed <= 0n) return null;
  const at = `at the recorded floor of ${floorText(t.floor)} SOL`;
  if (unit === "wX") return `≈ ${solApprox(wxToSol(typed, t.floor))} SOL ${at}`;
  const f = friendlyWx(t, solToWx(typed, t.floor));
  return `≈ ${f.plain} (${f.label}) ${at}`;
}

/** "≈ 27.4 SOL": a wX amount's SOL estimate at the floor; null without a floor. */
export const solOf = (wx: bigint, floor: bigint | null): string | null => (floor ? `≈ ${solApprox(wxToSol(wx, floor))} SOL` : null);
/** "≈ 27.4 SOL (2 Azuki tokens · 2 wAZUKI)": SOL first, then the tokens; just the tokens without a floor. */
export function solWithTokens(t: Tok, wx: bigint): string {
  const f = friendlyWx(t, wx);
  return f.sol ? `${f.sol} (${tokensText(f)})` : tokensText(f);
}

/** "≈ 12,345,678 coins (1.23 % of the supply)": what a first buy gets on a fresh curve. */
export function coinsText(amountOut: bigint, decimals: number, supply: bigint | null): string {
  const coins = Number(amountOut) / 10 ** decimals;
  const n = coins.toLocaleString("en-US", { maximumFractionDigits: coins >= 100 ? 0 : 2 });
  if (!supply || supply <= 0n) return `≈ ${n} coins`;
  const pct = (Number(amountOut) / Number(supply)) * 100;
  return `≈ ${n} coins (${pct.toLocaleString("en-US", { maximumFractionDigits: pct >= 10 ? 1 : 2 })} % of the supply)`;
}

/** A review row: the value on one line and its muted sub-line; `text` is the two as one sentence (screen readers, tests). */
export type ReviewValue = { main: string; sub: string | null; text: string };
/** A wX amount for a review row: "≈ 96 SOL" over "7 Azuki tokens · 7 wAZUKI"; without a floor the tokens are the value. */
export function reviewWx(t: Tok, wx: bigint): ReviewValue {
  const f = friendlyWx(t, wx);
  return f.sol ? { main: f.sol, sub: tokensText(f), text: solWithTokens(t, wx) } : { main: f.label, sub: f.plain === f.label ? null : f.plain, text: tokensText(f) };
}
/** The review's First buy: "≈ 27.4 SOL" over "2 Azuki tokens · 2 wAZUKI · from your wallet". */
export function reviewFirstBuy(t: Tok, amount: bigint, isSol = false): ReviewValue {
  if (amount <= 0n) return { main: "none", sub: null, text: "none" };
  if (isSol) return { main: `${solApprox(amount)} SOL`, sub: "in the total", text: `${solApprox(amount)} SOL` };
  const r = reviewWx(t, amount);
  return { main: r.main, sub: `${r.sub ? `${r.sub} · ` : ""}from your wallet`, text: `${r.text} from your wallet` };
}
/** The review's Total: "≈ 0.0276 SOL" over "+ first buy ≈ 27.4 SOL (2 wAZUKI)" (the First buy row above has the plain words). */
export function reviewTotal(t: Tok, totalLamports: bigint, buyWx: bigint): ReviewValue {
  const main = `≈ ${solText(totalLamports, 4)} SOL`;
  if (buyWx <= 0n) return { main, sub: null, text: main };
  const b = solWithTokens(t, buyWx), f = friendlyWx(t, buyWx);
  return { main, sub: `+ first buy ${f.sol ? `${f.sol} (${f.label})` : f.label}`, text: `${main}, plus your first buy of ${b}` };
}
/** "You hold ≈ 6.9 SOL (0.5 Azuki tokens · 0.5 wAZUKI)". */
export const holdText = (t: Tok, wx: bigint): string => `You hold ${solWithTokens(t, wx)}`;
/** "≈ 96 SOL to graduate" for a collection card, the wX threshold beside it; ROOK: "85 SOL". */
export function graduateText(t: Tok, wholeWx: number): { main: string; label: string | null } {
  const f = friendlyWx(t, wxUnits(wholeWx));
  return f.sol ? { main: f.sol, label: f.label } : { main: f.label, label: null };
}

/** A board's progress column for one collection: "Progress to graduation (5 Pudgy Penguins tokens ≈ 295 SOL)"; ROOK: "Progress to 85 SOL". */
export function progressLabel(slug: string | null | undefined): string {
  const t = thresholdOf(slug);
  if (!t) return `Progress to ${ROOK_GRADUATION_SOL} SOL`;
  const f = friendlyWx(tokOf(slug), wxUnits(t.wx));
  return `Progress to graduation (${f.plain}${f.sol ? ` ${f.sol}` : ""})`;
}

/** An amount as a plain decimal string for the input ("1.5", no grouping, up to 9 decimals). */
export function plainAmount(n: bigint): string {
  const whole = n / ONE, frac = (n % ONE).toString().padStart(9, "0").replace(/0+$/, "");
  return frac ? `${whole}.${frac}` : `${whole}`;
}
