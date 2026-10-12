/**
 * The Launch wizard's logic, with no React: what each step needs before Next, the URL of each step, the image checks, links typed
 * the way people type them, the draft kept in sessionStorage, the costs, and wallet / RPC errors said in plain words.
 * test/launch.test.ts is its spec.
 */
import { META_LIMITS, minBaseFeeNumerator, quoteSwap, type DbcConfig, type Quote as DbcQuote } from "./dbc";
import { coinLinksOf } from "./collectionLinks";
import { parseUnits } from "./format";
import { ROOK_GRADUATION_SOL, thresholdUnitsOf, wxText } from "./graduation";
import { firstBuyWx, floorLamportsOf, plainAmount, solOf, solToWx, wxToSol, type BuyUnit } from "./firstBuy";

export type Step = 1 | 2 | 3;
export const STEPS: readonly [Step, string][] = [[1, "Collection"], [2, "Your coin"], [3, "Review & launch"]];

/** `buy` is the first buy as typed, in `buyUnit`: SOL (the default, converted to wX at the recorded floor) or the collection's wX. */
export type Draft = { collection: string; name: string; symbol: string; description: string; website: string; twitter: string; telegram: string; buy: string; buyUnit: BuyUnit };
export const EMPTY_DRAFT: Draft = { collection: "", name: "", symbol: "", description: "", website: "", twitter: "", telegram: "", buy: "", buyUnit: "SOL" };
/** The picked image, as the page holds it: a data: URL (preview and upload), its type, size and pixel size. */
export type PickedImage = { dataUrl: string; type: string; size: number; width: number | null; height: number | null; name: string };

const utf8Len = (s: string) => new TextEncoder().encode(s).length;

// ── the image ──────────────────────────────────────────────────────────────────────────────────────────────────────────────────────
export const MAX_IMAGE_BYTES = 2 * 1024 * 1024;
export const IMAGE_TYPES = ["image/png", "image/jpeg", "image/gif", "image/webp"] as const;
export const IMAGE_ACCEPT = ".png,.jpg,.jpeg,.gif,.webp,image/png,image/jpeg,image/gif,image/webp";

/** The type from the file's first bytes (the server checks the same way), or null. */
export function sniffImage(b: Uint8Array): (typeof IMAGE_TYPES)[number] | null {
  if (b.length < 12) return null;
  if (b[0] === 0x89 && b[1] === 0x50 && b[2] === 0x4e && b[3] === 0x47 && b[4] === 0x0d && b[5] === 0x0a && b[6] === 0x1a && b[7] === 0x0a) return "image/png";
  if (b[0] === 0xff && b[1] === 0xd8 && b[2] === 0xff) return "image/jpeg";
  if (b[0] === 0x47 && b[1] === 0x49 && b[2] === 0x46 && b[3] === 0x38 && (b[4] === 0x37 || b[4] === 0x39) && b[5] === 0x61) return "image/gif";
  if (b[0] === 0x52 && b[1] === 0x49 && b[2] === 0x46 && b[3] === 0x46 && b[8] === 0x57 && b[9] === 0x45 && b[10] === 0x42 && b[11] === 0x50) return "image/webp";
  return null;
}

/** What is wrong with a picked file, or null. `head`: its first bytes, when read. */
export function imageFileProblem(f: { type: string; size: number }, head?: Uint8Array): string | null {
  if (!f.size) return "That file is empty.";
  const sniffed = head ? sniffImage(head) : null;
  if (head ? !sniffed : !(IMAGE_TYPES as readonly string[]).includes(f.type)) return "Use a PNG, JPG, GIF or WebP image.";
  if (f.size > MAX_IMAGE_BYTES) return `That image is ${(f.size / 1024 / 1024).toFixed(1)} MB. The limit is 2 MB.`;
  return null;
}
/** A gentle note about a non-square image (it is shown in a circle), or null. */
export function cropHint(w: number | null, h: number | null): string | null {
  if (!w || !h) return null;
  if (Math.max(w, h) / Math.min(w, h) > 1.15) return `This image is ${w}×${h}. It will be cropped to a circle; a square image looks best.`;
  if (Math.min(w, h) < 200) return `This image is ${w}×${h}. At least 400×400 looks sharper.`;
  return null;
}

// ── the coin ───────────────────────────────────────────────────────────────────────────────────────────────────────────────────────
export function nameProblem(name: string): string | null {
  if (!name.trim()) return "Give your coin a name.";
  if (utf8Len(name.trim()) > META_LIMITS.name) return `The name is too long (at most ${META_LIMITS.name} characters).`;
  return null;
}
/** Tickers are kept as typed but upper-cased, spaces and $ removed. */
export const cleanTicker = (s: string) => s.replace(/[\s$]/g, "").toUpperCase();
export function tickerProblem(symbol: string): string | null {
  const s = symbol.trim();
  if (!s) return "Give your coin a ticker.";
  if (utf8Len(s) > META_LIMITS.symbol) return `The ticker is too long (at most ${META_LIMITS.symbol} characters).`;
  if (/[^\p{L}\p{N}_.-]/u.test(s)) return "Use letters and numbers in the ticker.";
  return null;
}
export const DESCRIPTION_MAX = 600;

/** A link as people type it → the https URL the metadata holds; "" for empty; null when it cannot be one. */
export function normalizeLink(kind: "website" | "twitter" | "telegram", raw: string): string | null {
  const v = raw.trim();
  if (!v) return "";
  if (kind === "twitter") {
    const m = v.match(/^@?([A-Za-z0-9_]{1,15})$/) ?? v.match(/^(?:https?:\/\/)?(?:www\.)?(?:x|twitter)\.com\/([A-Za-z0-9_]{1,15})\/?(?:[?#].*)?$/i);
    return m ? `https://x.com/${m[1]}` : null;
  }
  if (kind === "telegram") {
    const m = v.match(/^@?([A-Za-z0-9_]{4,32})$/) ?? v.match(/^(?:https?:\/\/)?(?:www\.)?(?:t\.me|telegram\.me)\/(\+?[A-Za-z0-9_-]{4,64})\/?$/i);
    return m ? `https://t.me/${m[1]}` : null;
  }
  const u = /^https?:\/\//i.test(v) ? v.replace(/^http:/i, "https:") : `https://${v}`;
  try { const p = new URL(u); if (!p.hostname.includes(".") || /\s/.test(v)) return null; return u.length <= 200 ? u : null; } catch { return null; }
}
/**
 * The Website and X a collection prefills on step 2 (collectionLinks.ts `coinLinksOf`): X as "@handle", the website as its URL
 * (its first marketplace page when it has none); "" where the collection has none.
 */
export function prefillLinks(slug: string): { website: string; twitter: string } {
  const c = coinLinksOf(slug);
  return { website: c?.website ?? "", twitter: c?.twitter ? `@${c.twitter.replace(/^https:\/\/x\.com\//, "")}` : "" };
}
/**
 * The draft with its collection set to `slug`. Website and X are prefilled from the collection's official links, but only where the
 * field is empty or still holds the previous collection's prefill: switching collections updates untouched fields and never
 * overwrites what the creator typed. The same collection again: the draft as is (a field the creator cleared stays clear).
 */
export function withCollection(d: Draft, slug: string): Draft {
  if (d.collection === slug) return d;
  const prev = prefillLinks(d.collection), next = prefillLinks(slug);
  const out: Draft = { ...d, collection: slug };
  for (const k of ["website", "twitter"] as const) if (!d[k].trim() || d[k] === prev[k]) out[k] = next[k];
  return out;
}
export function linkProblem(kind: "website" | "twitter" | "telegram", raw: string): string | null {
  if (normalizeLink(kind, raw) !== null) return null;
  return kind === "twitter" ? "Use an X handle like @name or an x.com link." : kind === "telegram" ? "Use a Telegram link like t.me/name." : "Use a website address like example.com.";
}

/** The curve a first buy goes into: its threshold in its quote's units (the collection's wX; SOL for ROOK's own curve). */
export type Quote = { amount: number; symbol: string };
const ROOK_QUOTE: Quote = { amount: ROOK_GRADUATION_SOL, symbol: "SOL" };
/** The first buy in the quote's base units (9 decimals: wX and SOL alike): 0 for empty; null when it is not an amount. */
export function buyUnits(buy: string): bigint | null {
  const t = buy.trim().replace(",", ".");
  if (!t || /^0*\.?0*$/.test(t)) return 0n;
  const v = parseUnits(t, 9);
  return v === null || v < 0n ? null : v;
}
/** The most a first buy can be: the curve's whole raise, its collection's threshold (docs/v9/canister-only-solana.md §3.2). */
export const maxFirstBuy = (q: Quote): bigint => BigInt(Math.round(q.amount * 1e9));
/**
 * What is wrong with a typed first buy, or null. `unit` "SOL" on a wX curve with a recorded `floor` (firstBuy.ts) converts at the floor;
 * the bound is the curve's threshold in wX, also stated in SOL at the floor.
 */
export function buyProblem(buy: string, q: Quote = ROOK_QUOTE, unit: BuyUnit = "wX", floor: bigint | null = null): string | null {
  const wxCurve = q.symbol !== "SOL" && floor !== null;
  const inSol = unit === "SOL" && wxCurve;
  const l = buyUnits(buy);
  if (l === null) return `Enter the first buy as an amount of ${inSol ? "SOL" : q.symbol}, like 0.5.`;
  const wx = firstBuyWx(l, inSol ? "SOL" : "wX", floor);
  if (l > 0n && wx === 0n) return `That's less than the smallest amount of ${q.symbol} (0.000000001).`;
  const max = maxFirstBuy(q);
  if (wx > max) return `The first buy can't be more than the ${wxText(q.amount)} ${q.symbol} the curve holds${wxCurve ? ` (${solOf(max, floor)} at the recorded floor)` : ""}.`;
  return null;
}
/** The first buy the transaction uses, in the curve's quote base units (wX on a collection's curve); null when it is not an amount. */
export function draftBuyUnits(d: Pick<Draft, "collection" | "buy" | "buyUnit">): bigint | null {
  const l = buyUnits(d.buy);
  return l === null ? null : firstBuyWx(l, d.buyUnit, floorLamportsOf(d.collection));
}
/** The draft with the first buy's unit switched, the typed amount converted at the recorded floor (so the buy stays about the same). */
export function switchBuyUnit(d: Draft, unit: BuyUnit): Draft {
  if (d.buyUnit === unit) return d;
  const floor = floorLamportsOf(d.collection), l = buyUnits(d.buy);
  if (!floor || !l) return { ...d, buyUnit: unit };
  return { ...d, buyUnit: unit, buy: plainAmount(unit === "wX" ? solToWx(l, floor) : wxToSol(l, floor)) };
}

/** The fields of step 2 with their problems, for inline messages. */
export function coinProblems(d: Draft, image: PickedImage | null): Partial<Record<"name" | "symbol" | "image" | "description" | "website" | "twitter" | "telegram" | "buy", string>> {
  const p: ReturnType<typeof coinProblems> = {};
  const n = nameProblem(d.name); if (n) p.name = n;
  const s = tickerProblem(d.symbol); if (s) p.symbol = s;
  if (!image) p.image = "Add an image for your coin.";
  else { const i = imageFileProblem(image); if (i) p.image = i; }
  if (d.description.length > DESCRIPTION_MAX) p.description = `At most ${DESCRIPTION_MAX} characters.`;
  for (const k of ["website", "twitter", "telegram"] as const) { const l = linkProblem(k, d[k]); if (l) p[k] = l; }
  const b = buyProblem(d.buy, thresholdUnitsOf({ collection: { slug: d.collection } }), d.buyUnit, floorLamportsOf(d.collection)); if (b) p.buy = b;
  return p;
}

/** Why Next is not possible on a step, or null. */
export function stepProblem(step: Step, d: Draft, image: PickedImage | null, known: (slug: string) => boolean): string | null {
  if (!d.collection || !known(d.collection)) return "Pick a collection.";
  if (step === 1) return null;
  const p = coinProblems(d, image);
  const first = p.name ?? p.symbol ?? p.image ?? p.description ?? p.website ?? p.twitter ?? p.telegram ?? p.buy;
  return first ?? null;
}
/** The furthest step the draft allows, up to the one asked for (a refreshed or shared URL never lands past a missing field). */
export function reachableStep(asked: Step, d: Draft, image: PickedImage | null, known: (slug: string) => boolean): Step {
  if (asked >= 2 && stepProblem(1, d, image, known)) return 1;
  if (asked >= 3 && stepProblem(2, d, image, known)) return 2;
  return asked;
}

// ── the URL ────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────
/**
 * ["launch"] → step 1; ["launch", slug] → step 2 on that collection (the collections' "Launch" buttons); ["launch", slug, "1"|"2"|"3"]
 * → that step. An unknown slug is step 1.
 */
export function parseLaunchRoute(route: string[], known: (slug: string) => boolean): { slug: string | null; step: Step } {
  const slug = route[1] && known(route[1]) ? route[1] : null;
  if (!slug) return { slug: null, step: 1 };
  const n = Number(route[2]);
  return { slug, step: n === 1 || n === 2 || n === 3 ? (n as Step) : 2 };
}
export const launchHash = (slug: string, step: Step) => (step === 1 && !slug ? "#/launch" : `#/launch/${slug}/${step}`);

// ── the draft in sessionStorage ────────────────────────────────────────────────────────────────────────────────────────────────────
export const DRAFT_KEY = "rook.launch.draft.v1", IMAGE_KEY = "rook.launch.image.v1";
type Store = Pick<Storage, "getItem" | "setItem" | "removeItem">;
export function loadDraft(s: Store | null): { draft: Draft; image: PickedImage | null } {
  let draft = EMPTY_DRAFT, image: PickedImage | null = null;
  try {
    const j = JSON.parse(s?.getItem(DRAFT_KEY) ?? "null") as Partial<Draft> | null;
    if (j && typeof j === "object") {
      for (const k of Object.keys(EMPTY_DRAFT) as (keyof Draft)[]) if (k !== "buyUnit" && typeof j[k] === "string") draft = { ...draft, [k]: (j[k] as string).slice(0, 2_000) };
      // drafts saved before the SOL unit (2026-10-11 builds) hold the first buy in wX
      draft = { ...draft, buyUnit: j.buyUnit === "SOL" || j.buyUnit === "wX" ? j.buyUnit : "wX" };
    }
  } catch { /* storage blocked or garbage: start empty */ }
  try {
    const i = JSON.parse(s?.getItem(IMAGE_KEY) ?? "null") as PickedImage | null;
    if (i && typeof i.dataUrl === "string" && /^data:image\/(png|jpeg|gif|webp);base64,/.test(i.dataUrl) && typeof i.size === "number") image = i;
  } catch { /* no image */ }
  return { draft, image };
}
export function saveDraft(s: Store | null, draft: Draft, image: PickedImage | null): void {
  try { s?.setItem(DRAFT_KEY, JSON.stringify(draft)); } catch { /* storage blocked: the draft lives in memory only */ }
  try { if (image) s?.setItem(IMAGE_KEY, JSON.stringify(image)); else s?.removeItem(IMAGE_KEY); } catch { try { s?.removeItem(IMAGE_KEY); } catch { /* full or blocked */ } }
}
export function clearDraft(s: Store | null): void {
  try { s?.removeItem(DRAFT_KEY); s?.removeItem(IMAGE_KEY); } catch { /* blocked */ }
}

// ── costs ──────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────
/** Rent-exempt minimum for an account of n bytes (Solana's 3,480 lamports per byte-year × 2 years, with the 128-byte header). */
export const rentFor = (bytes: number) => BigInt((bytes + 128) * 6_960);
/** The accounts a launch creates and pays rent for: the DBC pool (424 B), its two token vaults (165 B each), the mint (82 B), the Metaplex metadata (679 B). */
export const LAUNCH_ACCOUNTS = [424, 165, 165, 82, 679];
/** A first buy also opens your token account for the coin (165 B). */
export const BUYER_ACCOUNT = 165;
/** Base fee (5,000 lamports a signature, two signers) plus a generous priority fee. An upper estimate. */
export const NETWORK_FEE_ESTIMATE = 10_000n + 600_000n;
/** Metaplex's fee for creating a token's metadata (0.01 SOL). */
export const METAPLEX_CREATE_FEE = 10_000_000n;
/** `total` is the SOL the launch takes from the wallet. A first buy is in the curve's quote: on a collection's curve that is its wX,
 *  paid from the wallet's wX account on top (`firstBuyWx`); on a SOL-quoted curve (ROOK's) it is SOL and part of `total`. */
export type Costs = { rent: bigint; metaplexFee: bigint; networkFee: bigint; firstBuy: bigint; firstBuyWx: bigint; total: bigint };
export function launchCosts(firstBuy: bigint, quote: "SOL" | "wX" = "wX"): Costs {
  const rent = LAUNCH_ACCOUNTS.reduce((a, b) => a + rentFor(b), 0n) + (firstBuy > 0n ? rentFor(BUYER_ACCOUNT) : 0n);
  const inSol = quote === "SOL" ? firstBuy : 0n;
  const total = rent + METAPLEX_CREATE_FEE + NETWORK_FEE_ESTIMATE + inSol;
  return { rent, metaplexFee: METAPLEX_CREATE_FEE, networkFee: NETWORK_FEE_ESTIMATE, firstBuy: inSol, firstBuyWx: quote === "wX" ? firstBuy : 0n, total };
}

// ── the launch buy's fee ───────────────────────────────────────────────────────────────────────────────────────────────────────────
/** A fee numerator (over 1e9) as a whole percent, the way the page prints fees (10,000,040 → 1; 500,000,000 → 50). */
const feePct = (num: bigint) => Math.round(Number(num) / 1e7);
/** The fees around a launch, from the config: `firstBuyPct`, what the creator's first buy pays in the launch transaction (the
 *  scheduler's minimum on a `firstSwapMinFee` config: every collection's since 2026-10-11, the creator's buy as pump.fun's dev buy;
 *  else the anti-sniper fee at its highest); `launchPct`, the anti-sniper fee everyone else pays at launch; `endPct`, where it ends;
 *  `minutes`, how long it falls. */
export function launchBuyFees(c: Pick<DbcConfig, "baseFee" | "firstSwapMinFee">) {
  const f = c.baseFee;
  const launchPct = feePct(f.cliffFeeNumerator), endPct = feePct(minBaseFeeNumerator(c));
  return { firstBuyMinFee: c.firstSwapMinFee, firstBuyPct: c.firstSwapMinFee ? endPct : launchPct, launchPct, endPct,
    minutes: Math.round(Number(f.secondFactor * BigInt(f.firstFactor)) / 60) };
}
/** The review step's estimate of the launch's first buy (`amount` in the curve's quote), at the fee it will pay (`launchBuyFees`). */
export const launchBuyQuote = (c: DbcConfig, amount: bigint, now: bigint): DbcQuote | null =>
  quoteSwap(c, { sqrtPrice: c.sqrtStartPrice, activationPoint: now }, "buy", amount, now, true);
/** The review step's note on the first buy's fee: a headline (bold) and the sentence after it. */
export function launchBuyNote(c: Pick<DbcConfig, "baseFee" | "firstSwapMinFee">): { headline: string; detail: string } {
  const x = launchBuyFees(c);
  return x.firstBuyMinFee
    ? { headline: `Your first buy, made in the launch itself, pays only the normal ${x.firstBuyPct} % fee.`,
        detail: `The anti-sniper fee applies to everyone else for the first ${x.minutes} minutes: ${x.launchPct} % at launch, falling to ${x.endPct} %.` }
    : { headline: `Heads-up: a buy in the launch pays the anti-sniper fee at its highest, about ${x.launchPct} %.`,
        detail: `The fee falls to ${x.endPct} % over the first ${x.minutes} minutes, so buying later costs less.` };
}
/** The costs list's line under a first buy: the fee it pays. */
export const launchBuyFeeShort = (c: Pick<DbcConfig, "baseFee" | "firstSwapMinFee">): string =>
  launchBuyFees(c).firstBuyMinFee ? `the normal ${launchBuyFees(c).firstBuyPct} % fee (no anti-sniper fee)` : "the launch-time anti-sniper fee";

// ── errors in plain words──────────────────────────────────────────────────────────────────────────────────────────────────────────
export function launchErrorText(e: unknown, need?: { total: bigint; balance: bigint | null }): string {
  const m = String((e as Error)?.message ?? e ?? "");
  if (/reject|denied|declin|cancel|User rejected|4001/i.test(m)) return "You cancelled in your wallet. Nothing was sent and nothing was spent.";
  if (/not enough sol|insufficient|InsufficientFunds|no SOL on Solana/i.test(m)) {
    const extra = need?.balance !== null && need?.balance !== undefined ? ` Your wallet has ${solShort(need.balance)} SOL; this launch needs about ${solShort(need.total)} SOL.` : "";
    return `Not enough SOL in your wallet.${extra} Add SOL or lower the first buy, then try again.`;
  }
  if (/slippage/i.test(m)) return "The price moved before your first buy went through. Nothing was sent. Try again, or lower the first buy.";
  if (/over Solana's \d+|too large|transaction is \d+ bytes/i.test(m)) return "The transaction came out too large. Shorten the name or ticker, or leave out the first buy.";
  if (/block ?height exceeded|expired|blockhash/i.test(m)) return "Solana didn't confirm the transaction in time, so it was dropped. Nothing was spent. Try again.";
  if (/Simulation failed/i.test(m)) return `Solana rejected the launch in a dry run, so your wallet wasn't asked to sign. Nothing was sent. (${m.replace(/^.*Simulation failed:\s*/i, "").slice(0, 140)})`;
  if (/Failed to fetch|NetworkError|network|timeout|429|503|502/i.test(m)) return "Couldn't reach Solana right now. Check your connection and try again.";
  if (/cannot sign|without the coin's mint|paid by another account/i.test(m)) return m;
  return `Something went wrong: ${m.slice(0, 160) || "unknown error"}. Nothing was sent.`;
}
const solShort = (l: bigint) => (Number(l) / 1e9).toLocaleString("en-US", { maximumFractionDigits: 4 });
