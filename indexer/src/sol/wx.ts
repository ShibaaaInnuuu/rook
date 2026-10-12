/**
 * The wX/SOL rate per collection (W13; canister-only design §5, §4.3, §10.1). A collection's coins are quoted in its wX
 * (Meteora DBC with quote mint = wX; DAMM v2 wX/coin after graduation), so a coin's SOL and dollar prices go through the wX's
 * own price in SOL:
 *   1. The pool desk's DLMM pair for the collection (autobuyer/canister-rs/src/pooldesk, solix2/dlmm.rs): one pair per wX on
 *      Meteora's preset 262 (bin step 50), `[preset, min(wX, wSOL), max(wX, wSOL)]`. Its active bin's price is the market's price
 *      of one wX base unit in Y base units, `(1 + bin_step / 10⁴)^active_id`, which with 9 decimals on both sides is SOL per wX.
 *      Read with the other account refreshes (every minute), stored per minute in `wx_sol` as `sol_usd` is.
 *   2. When the pair does not exist (no desk yet, or the desk withdrew it: §5.4) the canister's accepted floor stands in,
 *      `books_view(coll).accepted` (lamports per whole wX: the price its mint and burn are quoted around, canister.ts).
 * As with SOL/USD, a time with no rate near it (NEAR_S) has no SOL price: nothing is estimated, and the chart's SOL and USD
 * series then leave that trade out. The rate is computed in doubles here (a chart's precision), not DLMM's Q64.64 arithmetic.
 */
import { createHash } from "node:crypto";
import type { DatabaseSync } from "node:sqlite";
import { b58decode, pkAt, startsWith, u16At, u32At } from "./bytes.js";
import { NATIVE_MINT } from "./dbc.js";
import { findProgramAddress } from "./pda.js";
import { NEAR_S } from "./usd.js";

export const DLMM_PROGRAM = "LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo";
/** The desk's preset (dlmm.rs `DESK_PRESET`): bin step 50, base fee 0.4 %. */
export const DESK_PRESET = 262;
export const LB_PAIR_LEN = 904;
/** Anchor: `sha256("account:LbPair")[..8]`. */
export const LB_PAIR_DISC = new Uint8Array(createHash("sha256").update("account:LbPair").digest()).subarray(0, 8);

const cmpBytes = (a: Uint8Array, b: Uint8Array): number => { for (let i = 0; i < 32; i++) if (a[i] !== b[i]) return a[i] - b[i]; return 0; };
/** `["preset_parameter2", index u16 LE]`. */
export const presetAddress = (index = DESK_PRESET): string => findProgramAddress([Buffer.from("preset_parameter2"), Buffer.of(index & 255, index >> 8)], DLMM_PROGRAM);
/** The pair of `wxMint` against wSOL on the desk's preset: `[preset, min(x, y), max(x, y)]` (byte order, as Rust's `Pubkey: Ord`). */
export function pairAddress(wxMint: string, other = NATIVE_MINT, preset = presetAddress()): string {
  const a = b58decode(wxMint), b = b58decode(other);
  const [lo, hi] = cmpBytes(a, b) < 0 ? [a, b] : [b, a];
  return findProgramAddress([b58decode(preset), lo, hi], DLMM_PROGRAM);
}

/** `LbPair` (904 bytes), the fields a price needs: offsets as dlmm.rs `LbPair::decode` pins them. */
export type LbPair = { pairType: number; activeId: number; binStep: number; status: number; tokenXMint: string; tokenYMint: string; reserveX: string; reserveY: string };
export function decodeLbPair(d: Uint8Array): LbPair {
  if (d.length !== LB_PAIR_LEN || !startsWith(d, LB_PAIR_DISC)) throw new Error("not a DLMM LbPair");
  return { pairType: d[75], activeId: u32At(d, 76) | 0, binStep: u16At(d, 80), status: d[82], tokenXMint: pkAt(d, 88), tokenYMint: pkAt(d, 120), reserveX: pkAt(d, 152), reserveY: pkAt(d, 184) };
}
/** DLMM's bin price: Y base units per X base unit at bin `id`, `(1 + bin_step / 10⁴)^id`. */
export const binPrice = (binStep: number, id: number): number => (1 + binStep / 10_000) ** id;
/**
 * SOL per whole wX from a pair's active bin, whichever side the wX is on; null when the pair is not wX/wSOL or is disabled
 * (`status` 1: Meteora's operator can disable a pair, and a disabled pair's bin is not a market).
 */
export function solPerWx(p: LbPair, wxMint: string, wxDecimals = 9): number | null {
  if (p.status !== 0) return null;
  const px = binPrice(p.binStep, p.activeId);
  if (p.tokenXMint === wxMint && p.tokenYMint === NATIVE_MINT) return px * 10 ** (wxDecimals - 9);
  if (p.tokenYMint === wxMint && p.tokenXMint === NATIVE_MINT) return px > 0 ? (1 / px) * 10 ** (wxDecimals - 9) : null;
  return null;
}

export type WxRate = { sol: number; at: number; src: string };
export function storeWxRate(db: DatabaseSync, mint: string, t: number, sol: number, src: string): void {
  if (!(Number.isFinite(sol) && sol > 0)) return;
  db.prepare(`INSERT OR REPLACE INTO wx_sol (mint, t, sol, src) VALUES (?, ?, ?, ?)`).run(mint, t, sol, src);
}
export function latestWxRate(db: DatabaseSync, mint: string, now: number, maxAge = 3_600): WxRate | null {
  const r = db.prepare(`SELECT t, sol, src FROM wx_sol WHERE mint = ? ORDER BY t DESC LIMIT 1`).get(mint) as { t: number; sol: number; src: string } | undefined;
  return r && now - r.t <= maxAge ? { sol: r.sol, at: r.t, src: r.src } : null;
}
/** A lookup over [from, to] for one wX: the rate nearest each time (within NEAR_S), else null. Loads the window once. */
export function wxRateLookup(db: DatabaseSync, mint: string, from: number, to: number): (ts: number) => number | null {
  const rows = db.prepare(`SELECT t, sol FROM wx_sol WHERE mint = ? AND t BETWEEN ? AND ? ORDER BY t`).all(mint, from - NEAR_S, to + NEAR_S) as { t: number; sol: number }[];
  return (ts: number) => {
    if (!rows.length) return null;
    let lo = 0, hi = rows.length - 1;
    while (lo < hi) { const mid = (lo + hi + 1) >> 1; if (rows[mid].t <= ts) lo = mid; else hi = mid - 1; }
    let best: { t: number; sol: number } | null = null;
    for (const c of [rows[lo], rows[lo + 1]]) if (c && Math.abs(c.t - ts) <= NEAR_S && (!best || Math.abs(c.t - ts) < Math.abs(best.t - ts))) best = c;
    return best ? best.sol : null;
  };
}
