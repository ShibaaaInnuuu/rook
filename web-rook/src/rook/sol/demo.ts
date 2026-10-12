/**
 * Demo mode: `?demo` in the URL (https://userook.fun/?demo#/coins). Off unless asked for; every page then carries a DEMO banner.
 *
 * The data layer (src/rook/sol/hooks.ts, hooksLite.ts, and the canister's hooks through src/rook/sol/demoCanister.ts) swaps these
 * fixtures in at the hooks, so the pages render exactly as they will once coins launch. The demo's world (`DEMO_OPEN`): collections
 * open on all four chains (ten Ethereum ones, Bitcoin Frogs, four Solana ones, Chain Mancers on Robinhood Chain), Doodles and the
 * rest of the pinned ones not open yet (the "Open it" path), the 14 other Ordinals among them; a board of 33 coins on the open
 * collections and the genesis ROOK coin, in every state (just launched, mid-curve, graduating, graduated, one graduated coin's
 * backing in its vault), every collection coin quoted in its collection's wX as canister-only Rook quotes them
 * (docs/v9/canister-only-solana.md §3), ROOK in SOL. Nothing here is real or on chain: every address is made up (a hash
 * of a label, never a key anyone holds), the names are invented, and the pictures are the collections' logos or generated mark tiles.
 *
 * Demo never builds, simulates or sends a transaction: the coin page's ticket is a demo ticket, the launch wizard reads `DEMO`
 * and stays dry, and no fixture reaches an RPC. The curve figures are computed from Rook's real curve parameters: one curve shape
 * (the 85 SOL curve of ops/solana/launch-params.mjs, `design_curve.mjs 1e9 20 85 50 6`) scaled in √price and liquidity to each
 * collection's threshold in whole wX (src/rook/sol/graduation.ts), the way `design_curve.mjs` itself scales between thresholds, so
 * the prices, market caps, thresholds and fee schedule read as they will: a Milady coin graduates at 5 wMILADY with a market cap of
 * 12.5 wMILADY. The demo's wX/SOL rate is each collection's day-1 floor (`demoQuoteSol`): no pair is read.
 */
import type { TokenMeta } from "./accounts";
import type { CoinRow, ConfigRef, PoolRefRow } from "./board";
import { COLLECTIONS, collectionBySlug } from "./collections";
import type { DbcConfig, DbcPool } from "./dbcState";
import type { OffChainMeta } from "./offchain";
import { b58encode } from "./base58";
import { ROOK_GRADUATION_SOL, thresholdWxOf } from "./graduation";
import { NATIVE_MINT } from "./keys";
import { SOL, type SolanaConfig } from "./siteConfig";

/** `?demo` (or `?demo=1`); `?demo=0` / `?demo=false` and no parameter are off. */
export const isDemoSearch = (search: string): boolean => {
  const p = new URLSearchParams(search);
  if (!p.has("demo")) return false;
  const v = (p.get("demo") ?? "").toLowerCase();
  return v !== "0" && v !== "false" && v !== "off";
};
export const DEMO: boolean = typeof location !== "undefined" && isDemoSearch(location.search);
export const DEMO_BANNER = "Demo data — nothing here is real or on chain";

// ---- made-up addresses -------------------------------------------------------------------------------------------------------------

/** 32 bytes from a label (xorshift over FNV-1a), base58: a well-formed address nobody holds the key to. Deterministic. */
export function demoAddress(label: string): string {
  let h = 2166136261 >>> 0;
  for (let i = 0; i < label.length; i++) { h ^= label.charCodeAt(i); h = Math.imul(h, 16777619) >>> 0; }
  const out = new Uint8Array(32);
  for (let i = 0; i < 32; i++) { h ^= h << 13; h >>>= 0; h ^= h >>> 17; h ^= h << 5; h >>>= 0; out[i] = h & 0xff; }
  out[0] = (out[0] % 200) + 40;                    // never a leading zero byte: every demo address is 43-44 characters
  return b58encode(out);
}

// ---- the curves: one shape, scaled to each collection's threshold ------------------------------------------------------------------

const LAMPORTS = 1_000_000_000n;
/** A curve: its √P constants (Q64.64), the single constant-liquidity segment up to graduation, the tail after it, and its threshold
 *  in quote base units (wX or lamports: 9 decimals either way). */
export type DemoCurve = { START: bigint; GRAD: bigint; L1: bigint; TAIL: bigint; THRESHOLD: bigint };
/** The 85 SOL curve, `design_curve.mjs 1000000000 20 85 50 6`, pinned in ops/solana/day1-pins.json: ROOK's, and the shape of every
 *  other. One constant-liquidity segment from the start price to graduation (√P doubles: the market cap is ×4), then the tail. */
const BASE: DemoCurve = { START: 134452599321738966n, GRAD: 268905193275807567n, L1: 215124158914782432327345953095692n, TAIL: 3435308751277378593870180n, THRESHOLD: 85n * LAMPORTS };
const isqrt = (n: bigint): bigint => { if (n < 2n) return n; let x = n, y = (x + 1n) >> 1n; while (y < x) { x = y; y = (x + n / x) >> 1n; } return x; };
/** The base shape scaled to raise `threshold` base units: √P and the liquidity both scale by √(threshold / 85), so the raise
 *  `L·(√P_grad − √P_start) / 2^128` scales by the ratio itself (to a base unit or so of rounding) and the market cap at graduation
 *  stays 2.5 × the threshold. */
export function demoCurveFor(threshold: bigint): DemoCurve {
  const s = isqrt((threshold << 64n) / BASE.THRESHOLD);        // √(threshold / 85 SOL) in Q32
  const k = (v: bigint) => (v * s) >> 32n;
  return { START: k(BASE.START), GRAD: k(BASE.GRAD), L1: k(BASE.L1), TAIL: k(BASE.TAIL), THRESHOLD: threshold };
}
/** A collection's threshold in base units: its whole wX (src/rook/sol/graduation.ts); ROOK's 85 SOL in lamports. */
export const demoThresholdUnits = (slug: string): bigint => BigInt(thresholdWxOf(slug) ?? ROOK_GRADUATION_SOL) * LAMPORTS;
const curves = new Map<string, DemoCurve>();
/** The curve a coin of this collection trades on (ROOK: the 85 SOL curve itself), for src/rook/sol/demoMarket.ts. */
export const demoCurveOf = (slug: string): DemoCurve => { let c = curves.get(slug); if (!c) { c = demoCurveFor(demoThresholdUnits(slug)); curves.set(slug, c); } return c; };
/** ROOK's curve: the base shape, unscaled. */
export const DEMO_CURVE = BASE;
/** The demo's wX mint of a collection (made up); ROOK's quote is wSOL. */
export const demoWxMint = (slug: string): string => (slug === "rook" ? NATIVE_MINT : demoAddress(`wx:${slug}`));
/** The demo's price of one wX in SOL: the collection's day-1 floor (a real pair, docs/v9/canister-only-solana.md §17, is kept
 *  centred on the floor by the custodian); 1 for ROOK, quoted in SOL itself. */
export const demoQuoteSol = (slug: string): number => collectionBySlug(slug)?.floorSol ?? 1;

/** Rook's DBC parameters (ops/solana/launch-params.mjs) with the collection's quote and threshold: `quote_mint` = its wX, the
 *  threshold in whole wX, fees in the quote; ROOK's SOL-paired genesis config at 85 SOL (§3.1). */
function configOf(slug: string): DbcConfig {
  const c = demoCurveOf(slug);
  return {
    quoteMint: demoWxMint(slug), feeClaimer: demoAddress("claimer"), leftoverReceiver: demoAddress("claimer"),
    baseFee: { cliffFeeNumerator: 500_000_000n, firstFactor: 60, secondFactor: 10n, thirdFactor: 8_166_666n, baseFeeMode: 0 },
    dynamicFee: false, collectFeeMode: 0, migrationOption: 1, activationType: 1, tokenDecimal: 6, version: 0, tokenType: 0,
    partnerLockedLp: 50, creatorLockedLp: 50, partnerLp: 0, creatorLp: 0, fixedSupply: true, creatorTradingFeePercentage: 0, tokenUpdateAuthority: 1,
    migrationFeePercentage: slug === "rook" ? 0 : 50, creatorMigrationFeePercentage: 0, migrationQuoteThreshold: c.THRESHOLD, migrationBaseThreshold: 0n, migrationSqrtPrice: c.GRAD,
    preMigrationTokenSupply: 1_000_000_000_000_000n, postMigrationTokenSupply: 1_000_000_000_000_000n, migratedPoolFeeBps: 100, poolCreationFee: 0n,
    sqrtStartPrice: c.START, curve: [{ sqrtPrice: c.GRAD, liquidity: c.L1 }, { sqrtPrice: 79226673521066979257578248091n, liquidity: c.TAIL }],
    // the creator's launch buy at the minimum fee: every collection's config (the canister's `rook_wx_config`); ROOK's SOL config is D-e's (off)
    firstSwapMinFee: slug !== "rook",
  };
}
const configs = new Map<string, DbcConfig>();
/** The config of a collection's coins (ROOK: its own). */
export const demoConfigOf = (slug: string): DbcConfig => { let c = configs.get(slug); if (!c) { c = configOf(slug); configs.set(slug, c); } return c; };
/** ROOK's config: the 85 SOL curve, SOL-quoted. */
export const DEMO_CONFIG: DbcConfig = demoConfigOf("rook");
/** A collection's DBC config address in the demo (made up): the one its opening makes. */
export const demoConfigAddress = (slug: string): string => demoAddress(`config:${slug}`);

/** The demo's open collections: on all four chains (operator decision 2026-10-12: a collection opens when someone first launches on
 *  it). Every other collection the ceremony pins is "not open yet" (Doodles among them: its page and the launch wizard show "Open it"),
 *  the 14 other Ordinals and the Robinhood Chain ones beside Chain Mancers are pinned and not open. */
export const DEMO_OPEN: readonly string[] = [
  "milady", "azuki", "mayc", "pudgypenguins", "bayc", "moonbirds", "clonex", "lilpudgys", "checks", "identitymd",
  "bitcoin-frogs", "mad_lads", "smb_gen2", "claynosaurz", "okay_bears", "chainmancers",
];
/** The pinned collection the demo shows as not open yet (and the one its "Open it" story starts from). */
export const DEMO_NOT_OPEN = "doodles";

/** The site config as it will read once the custodian has opened the demo's collections, with made-up addresses: what the demo pages
 *  see in place of src/config/solana.ts. */
export const DEMO_SOL: SolanaConfig = {
  ...SOL, ready: true, problems: [],
  collections: COLLECTIONS.filter((c) => DEMO_OPEN.includes(c.slug)).map((c) => ({ ...c, config: demoConfigAddress(c.slug), listing: demoAddress(`listing:${c.slug}`), claimer: demoAddress(`claimer:${c.slug}`) })),
  rook: { config: demoAddress("config:rook"), mint: demoAddress("mint:ROOK"), pool: demoAddress("pool:ROOK") },
  cloudinary: null,
};
export const DEMO_REFS: ConfigRef[] = [
  ...DEMO_SOL.collections.map((c) => ({ config: c.config!, slug: c.slug, label: c.name })),
  { config: DEMO_SOL.rook!.config, slug: "rook", label: "Genesis (ROOK)" },
];

// ---- the coins ---------------------------------------------------------------------------------------------------------------------

/** progress: share of the coin's own threshold raised (its collection's whole wX; 1 = graduating); "grad": graduated. age: seconds since launch. */
type Seed = { slug: string; name: string; symbol: string; progress: number | "grad"; age: number; logo?: boolean; about: string; /** a graduated coin's price against graduation's, in its pool */ x?: number };
const MIN = 60, H = 3_600, D = 86_400;
const SEEDS: Seed[] = [
  { slug: "milady", name: "Milady Mood", symbol: "MOOD", progress: "grad", x: 2.6, age: 6 * D + 4 * H, logo: true, about: "The first Milady coin to graduate: 2.5 wMILADY in its vault." },
  { slug: "milady", name: "Remilia Rain", symbol: "RAIN", progress: 0.64, age: 19 * H, about: "Weather report: cloudy with a chance of floor." },
  { slug: "azuki", name: "Red Bean Club", symbol: "BEANCLUB", progress: 0.12, age: 2 * D + 3 * H, about: "One bean, then another." },
  { slug: "azuki", name: "Garden Gate", symbol: "GATE", progress: 1, age: 1 * D + 7 * H, logo: true, about: "Through the gate and into the garden." },
  { slug: "mayc", name: "Serum Season", symbol: "SERUM", progress: 0.81, age: 3 * D + 2 * H, about: "Drink responsibly." },
  { slug: "mayc", name: "Mutant Mondays", symbol: "MUTMON", progress: 0.004, age: 3 * MIN + 20, about: "Every week, a little stranger." },
  { slug: "pudgypenguins", name: "Huddle", symbol: "HUDDLE", progress: "grad", x: 1.45, age: 4 * D + 9 * H, logo: true, about: "Warmer together." },
  { slug: "pudgypenguins", name: "Ice Floe", symbol: "FLOE", progress: 0.47, age: 11 * H, about: "Drifting toward 5 wPUDGY." },
  { slug: "bayc", name: "Swamp Club", symbol: "SWAMP", progress: 0.55, age: 2 * D + 20 * H, about: "Meet at the swamp." },
  { slug: "bayc", name: "Banana Peel", symbol: "PEEL", progress: 0.012, age: 7 * MIN, about: "Mind your step." },
  { slug: "moonbirds", name: "Night Owl", symbol: "HOOT", progress: 0.55, age: 1 * D + 2 * H, about: "Up all night watching the curve." },
  { slug: "moonbirds", name: "Nesting", symbol: "NEST", progress: "grad", x: 0.78, age: 5 * D + 1 * H, logo: true, about: "Settled into its locked pool." },
  { slug: "clonex", name: "Drip Forge", symbol: "FORGE", progress: 0.18, age: 9 * H, about: "Forged, not minted." },
  { slug: "clonex", name: "Pixel Clone", symbol: "PXCL", progress: 0.001, age: 45, about: "Launched a moment ago." },
  { slug: "lilpudgys", name: "Snowball", symbol: "SNOWB", progress: 0.72, age: 1 * D + 16 * H, logo: true, about: "Rolling downhill, getting bigger." },
  { slug: "checks", name: "Check Mate", symbol: "CHKMT", progress: 0.39, age: 22 * H, about: "Your move." },
  { slug: "checks", name: "Open Edition", symbol: "OPENED", progress: 0.09, age: 6 * H, about: "Everyone gets one." },
  { slug: "identitymd", name: "Mirror Mirror", symbol: "MIRROR", progress: 0.24, age: 13 * H, logo: true, about: "Who is the fairest of them all?" },
  { slug: "bitcoin-frogs", name: "Frog Pond", symbol: "POND", progress: 1, age: 2 * D + 1 * H, logo: true, about: "The pond is full; moving to its pool." },
  { slug: "bitcoin-frogs", name: "Ribbit Sats", symbol: "RIBBIT", progress: 0.88, age: 3 * D, about: "Inscribed croaks." },
  { slug: "bitcoin-frogs", name: "Lily Pad", symbol: "LILY", progress: 0.008, age: 90, about: "Fresh off the pad." },
  { slug: "identitymd", name: "Second Self", symbol: "SELF", progress: 0.36, age: 10 * H, about: "Page by page, a new face." },
  { slug: "mad_lads", name: "Lad Lounge", symbol: "LOUNGE", progress: 0.33, age: 17 * H, about: "Backpacks off, feet up." },
  { slug: "mad_lads", name: "Lad Legends", symbol: "LEGEND", progress: "grad", x: 1.2, age: 3 * D + 8 * H, about: "Graduated into its locked pool, priced in wMADLAD." },
  { slug: "smb_gen2", name: "Monke Business", symbol: "MONKBIZ", progress: 0.58, age: 1 * D + 9 * H, about: "Strictly business, on Solana." },
  { slug: "smb_gen2", name: "Monke Surf", symbol: "SURF", progress: 0.44, age: 14 * H, about: "Riding the curve to 7 wSMBGEN2." },
  { slug: "claynosaurz", name: "Clay Day", symbol: "CLAYDAY", progress: 0.21, age: 8 * H, about: "Shaped by hand." },
  { slug: "claynosaurz", name: "Dino Egg", symbol: "EGG", progress: 0.027, age: 25 * MIN, about: "Hatched a few minutes ago." },
  { slug: "okay_bears", name: "Okay Okay", symbol: "OKOK", progress: 0.16, age: 5 * H, about: "Everything is okay." },
  { slug: "okay_bears", name: "Bear Market", symbol: "BEARMKT", progress: 0.52, age: 20 * H, about: "Up and down, mostly okay." },
  { slug: "chainmancers", name: "Mancer Guild", symbol: "GUILD", progress: 0.67, age: 2 * D + 6 * H, about: "A Robinhood Chain collection, a Solana coin." },
  { slug: "chainmancers", name: "Chain Spell", symbol: "CHSPELL", progress: 0.05, age: 4 * H, about: "Cast on chain 4663, traded on Solana." },
  { slug: "rook", name: "Rook", symbol: "ROOK", progress: 0.38, age: 1 * D + 5 * H, about: "The genesis coin: its fees pay Rook's running costs. No backing, no rights." },
];
/** The one graduated coin whose backing share the custodian has already moved into its vault (in the demo's story). */
export const DEMO_BACKED_SYMBOL = "MOOD";

const TILE_HUES = [12, 38, 160, 200, 262, 320, 95, 180, 230, 290];
/** A generated mark tile: the ticker on a two-tone ground. */
export function markTile(symbol: string, i: number): string {
  const h = TILE_HUES[i % TILE_HUES.length], t = symbol.slice(0, 4).replace(/[^A-Z0-9]/gi, "");
  const svg = `<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 120 120'><defs><linearGradient id='g' x1='0' y1='0' x2='1' y2='1'><stop offset='0' stop-color='hsl(${h} 70% 58%)'/><stop offset='1' stop-color='hsl(${(h + 40) % 360} 65% 32%)'/></linearGradient></defs><rect width='120' height='120' fill='url(#g)'/><circle cx='92' cy='26' r='16' fill='hsl(${h} 90% 85%)' opacity='.35'/><text x='60' y='62' dy='.35em' text-anchor='middle' font-family='ui-monospace,monospace' font-weight='700' font-size='${t.length > 3 ? 28 : 34}' fill='white'>${t}</text></svg>`;
  return `data:image/svg+xml,${encodeURIComponent(svg)}`;
}

export type DemoCoin = CoinRow & { meta: TokenMeta; off: OffChainMeta; backed: boolean };

/** The fixture coins, newest first, their launch times measured back from `now` (unix seconds). */
export function demoCoins(now: number = Math.floor(Date.now() / 1000)): DemoCoin[] {
  const coins = SEEDS.map((s, i): DemoCoin => {
    const ref = DEMO_REFS.find((r) => r.slug === s.slug)!;
    const mint = s.slug === "rook" ? DEMO_SOL.rook!.mint : demoAddress(`mint:${s.symbol}`);
    const pool = s.slug === "rook" ? DEMO_SOL.rook!.pool : demoAddress(`pool:${s.symbol}`);
    const { START, GRAD, L1, THRESHOLD } = demoCurveOf(s.slug);
    const graduated = s.progress === "grad";
    const quote = graduated ? THRESHOLD : (THRESHOLD * BigInt(Math.round((s.progress as number) * 10_000))) / 10_000n;
    const launchedAt = now - s.age;
    const backed = s.symbol === DEMO_BACKED_SYMBOL;
    const p: DbcPool = {
      config: ref.config, creator: demoAddress(`creator:${s.symbol}`), baseMint: mint, baseVault: demoAddress(`basevault:${s.symbol}`), quoteVault: demoAddress(`quotevault:${s.symbol}`),
      baseReserve: 0n, quoteReserve: quote, partnerQuoteFee: (quote * 96n) / 10_000n, creatorQuoteFee: 0n,
      sqrtPrice: graduated ? (GRAD * BigInt(Math.round(Math.sqrt(s.x ?? 1) * 1e6))) / 1_000_000n : START + (quote << 128n) / L1, activationPoint: BigInt(launchedAt), poolType: 0, isMigrated: graduated, migrationProgress: graduated ? 3 : 0,
      finishCurveTimestamp: graduated ? BigInt(launchedAt + Math.floor(s.age / 2)) : 0n, migrationFeeWithdrawStatus: backed ? 0b100 : 0,
    };
    const col = collectionBySlug(s.slug);
    const uri = `demo:${mint}`;
    const meta: TokenMeta = { updateAuthority: demoAddress("update-authority"), mint, name: s.name, symbol: s.symbol, uri };
    // ROOK wears its real picture (the site's copy of its metadata's image); the invented coins a generated tile or their collection's logo
    const image = s.slug === "rook" ? "/rook-token/rook.png" : s.logo && col?.logo ? col.logo : markTile(s.symbol, i);
    const prog = graduated ? 1 : (s.progress as number);
    const phase: "curve" | "graduating" | "graduated" = graduated ? "graduated" : quote >= THRESHOLD ? "graduating" : "curve";
    const price = priceOf(p.sqrtPrice);
    return {
      pool, config: ref.config, launchedAt, slug: ref.slug, label: ref.label, mint, creator: p.creator, meta, pool_: p, backed,
      stats: { priceSol: price, supply: 1_000_000_000, marketCapSol: price * 1_000_000_000, progress: Math.min(1, prog), phase, raisedLamports: quote, thresholdLamports: THRESHOLD },
      off: { image, description: `${s.about} (Demo coin: invented, not on chain.)`, website: null, twitter: null, telegram: null },
    };
  });
  return coins.sort((a, b) => b.launchedAt - a.launchedAt || (a.pool < b.pool ? -1 : 1));
}
/** The curve's price in quote per coin (6-decimal coin, 9-decimal quote): wX per coin, or SOL per ROOK. */
const priceOf = (sqrt: bigint) => { const s = Number(sqrt) / 2 ** 64; return s * s * 10 ** (6 - 9); };

let cache: { at: number; coins: DemoCoin[] } | null = null;
/** The fixture coins, made once per page load (so ages tick on from the moment the page opened). */
export const DEMO_COINS = (): DemoCoin[] => (cache ??= { at: Date.now(), coins: demoCoins() }).coins;

// ---- what the hooks answer in demo mode --------------------------------------------------------------------------------------------

/** Every collection's config (an opening in the demo makes one at its made-up address) and ROOK's. */
export const demoConfigs = (): Map<string, DbcConfig> => new Map([...COLLECTIONS.map((c): [string, DbcConfig] => [demoConfigAddress(c.slug), demoConfigOf(c.slug)]),
  [DEMO_SOL.rook!.config, demoConfigOf("rook")]]);
export const demoPools = (configs: string[]): PoolRefRow[] => DEMO_COINS().filter((c) => configs.includes(c.config)).map((c) => ({ pool: c.pool, config: c.config, launchedAt: c.launchedAt }));
export const demoRows = (refs: PoolRefRow[]): CoinRow[] => refs.map((r) => DEMO_COINS().find((c) => c.pool === r.pool)).filter((c): c is DemoCoin => !!c);
export function demoFindCoin(mint: string): { ref: ConfigRef; pool: string; poolAcc: DbcPool } | null {
  const c = DEMO_COINS().find((x) => x.mint === mint);
  return c ? { ref: DEMO_REFS.find((r) => r.config === c.config)!, pool: c.pool, poolAcc: c.pool_ } : null;
}
export const demoMeta = (mint: string): TokenMeta | null => DEMO_COINS().find((c) => c.mint === mint)?.meta ?? null;
export const demoOffChain = (uri: string): OffChainMeta | null => DEMO_COINS().find((c) => c.meta.uri === uri)?.off ?? null;
export const DEMO_SOL_USD = { price: 171.84, publishTime: 0 };
/** How many of each collection's coins the fixtures hold (every collection, open or not; ROOK), for a test and the demo canister. */
export const demoCountBySlug = (): Record<string, number> => Object.fromEntries([...COLLECTIONS.map((c) => c.slug), "rook"].map((s) => [s, DEMO_COINS().filter((c) => c.slug === s).length]));
