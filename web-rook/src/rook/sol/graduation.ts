/**
 * Graduation thresholds on the site (docs/v9/canister-only-solana.md §3.2). A collection coin's curve is quoted in the collection's
 * wX and graduates when it holds `T_wX = max(3, ⌈tier_SOL / floor_SOL⌉)` whole wX at the day-1 floors of
 * deployments/launch/graduation-tiers.json, computed here exactly as the canister's launch pins compute it
 * (autobuyer/canister-rs/examples/launch_pins.rs: `tier.div_ceil(floor).max(3)` in lamports); the Solana collections' thresholds are
 * solana-collections.json's `threshold_wx`, the same rule on the floors it measured. Half the raise is the coin's backing, straight
 * into its vault at graduation; the market cap at graduation is 2.5 × the threshold (fixed 1B supply, 50 % migration fee). ROOK keeps
 * its SOL-paired curve: 85 SOL, no backing. Once a coin exists, its own on-chain config is the source of truth for its threshold
 * (`thresholdUnitsOf`); the computed threshold is for a collection with no coin yet. Pure: test/sol.test.ts holds every collection
 * against the design's table.
 */
import { COLLECTIONS, collectionBySlug, type Chain, type SolCollection } from "./collections";

const LAMPORTS = 1_000_000_000n;
/** ROOK's graduation (SOL): its own SOL-paired config, unchanged. */
export const ROOK_GRADUATION_SOL = 85;
/** The smallest threshold: three whole wX, so the vault gets at least 1.5 wX (more than one NFT's worth) from the first graduation. */
export const MIN_THRESHOLD_WX = 3;

/** `max(3, ⌈tierSol / floorSol⌉)` in lamport integer arithmetic, as launch_pins.rs computes it. */
export function thresholdWx(tierSol: number, floorSol: number): number {
  const floor = BigInt(Math.round(floorSol * 1e9)), tier = BigInt(tierSol) * LAMPORTS;
  if (floor <= 0n || tier <= 0n) throw new Error(`a threshold needs a floor and a tier: ${floorSol} SOL, ${tierSol} SOL`);
  const t = (tier + floor - 1n) / floor;
  return Number(t < BigInt(MIN_THRESHOLD_WX) ? BigInt(MIN_THRESHOLD_WX) : t);
}

/** One collection's threshold and what follows from it. `approxSol`: the raise at the day-1 floor; `page`: it has a page here. */
export type Threshold = {
  slug: string; name: string; symbol: string; chain: Chain;
  wx: number; tierSol: number; floorSol: number; approxSol: number; backingWx: number; graduationMcapWx: number; page: boolean;
};
const rowOf = (c: Pick<SolCollection, "slug" | "name" | "wxSymbol" | "tierSol" | "floorSol">, chain: Threshold["chain"], wx: number, page: boolean): Threshold =>
  ({ slug: c.slug, name: c.name, symbol: c.wxSymbol, chain, wx, tierSol: c.tierSol, floorSol: c.floorSol, approxSol: Math.round(wx * c.floorSol), backingWx: wx / 2, graduationMcapWx: wx * 2.5, page });
/** Every collection's threshold, each with its page: the 26 of nft-pilot.json (listing order) computed from tier and floor, then the
 *  twelve Solana collections at solana-collections.json's `threshold_wx` (the same rule on the floors it measured). */
export const THRESHOLDS: readonly Threshold[] = COLLECTIONS.map((c) =>
  rowOf(c, c.chain, c.thresholdWx !== undefined ? Math.max(MIN_THRESHOLD_WX, c.thresholdWx) : thresholdWx(c.tierSol, c.floorSol), true));
export const thresholdOf = (slug: string | null | undefined): Threshold | null => THRESHOLDS.find((t) => t.slug === slug) ?? null;
/** A collection's threshold in whole wX; null for ROOK and anything unknown (their curve is in SOL). */
export const thresholdWxOf = (slug: string | null | undefined): number | null => thresholdOf(slug)?.wx ?? null;
/** A collection coin's backing share at graduation (half its threshold, wX); ROOK has none. */
export const backingShareWxOf = (slug: string | null | undefined): number => thresholdOf(slug)?.backingWx ?? 0;

/** What a coin on this collection is quoted in: the collection's wX (9 decimals, as SOL has); ROOK's and anything unknown: SOL. */
export function quoteOf(slug: string | null | undefined): { symbol: string; decimals: number } {
  const c = collectionBySlug(slug);
  return { symbol: c ? c.wxSymbol : "SOL", decimals: 9 };
}

/** "5", "2.5", "174": whole wX, or one decimal for a half (a backing share). */
export const wxText = (n: number): string => n.toLocaleString("en-US", { maximumFractionDigits: 1 });
/** "5 wMILADY (≈ 93 SOL at the day-1 floor)", or without the aside; ROOK: "85 SOL". */
export function thresholdText(slug: string | null | undefined, approx = true): string {
  const t = thresholdOf(slug);
  if (!t) return `${ROOK_GRADUATION_SOL} SOL`;
  return approx ? `${wxText(t.wx)} ${t.symbol} (≈ ${t.approxSol.toLocaleString("en-US")} SOL at the day-1 floor)` : `${wxText(t.wx)} ${t.symbol}`;
}

/** A coin's own threshold in its quote's units: its curve's (read from its config) when known, else its collection's computed one. */
export function thresholdUnitsOf(c: { curve?: { thresholdLamports: string } | null; collection?: { slug: string } | null }): { amount: number; symbol: string } {
  const slug = c.collection?.slug;
  const own = c.curve ? Number(c.curve.thresholdLamports) / 1e9 : NaN;
  return { amount: Number.isFinite(own) && own > 0 ? own : thresholdWxOf(slug) ?? ROOK_GRADUATION_SOL, symbol: quoteOf(slug).symbol };
}
/** "5 wMILADY" / "85 SOL": a coin's threshold as the board prints it. */
export const raiseText = (c: Parameters<typeof thresholdUnitsOf>[0]): string => { const t = thresholdUnitsOf(c); return `${wxText(t.amount)} ${t.symbol}`; };
