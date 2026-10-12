/**
 * Meteora DLMM (`lb_clmm` 0.12.0, mainnet `LBUZ…`), the venue of every collection's wX/SOL pool (docs/v9/canister-only-solana.md §17:
 * one pair per collection on preset 262, bin step 50, 0.4 % base fee, fees in SOL, one position owned by the canister's pool desk).
 * The site's first hop (SOL → wX) is DLMM `swap2` on that pair. A port of the canister's `solix2::dlmm` (autobuyer/canister-rs),
 * whose builders and decoders were checked byte for byte against instructions that landed in LiteSVM on the mainnet program bytes
 * (`tests/fixtures/solix2/dlmm.json`); test/twohop.test.ts holds this port against the same bytes.
 *
 * Here: the PDAs, the `swap2` builder, DLMM's own bin-price arithmetic (Q64.64), the `LbPair` and `BinArray` decoders, and a walk
 * over the bins that quotes a swap (DLMM's `swap_exact_in`, with the variable fee). The quote is a preview: every trade is simulated
 * on chain before the wallet is asked, and the minimum out comes from that simulation (src/rook/sol/trade.ts).
 */
import { b58encode } from "./base58";
import { Writer, i32At, i64At, sameBytes, u128At, u16At, u32At, u64At } from "./borsh";
import { NATIVE_MINT, TOKEN_PROGRAM, keyBytes, type Address } from "./keys";
import { pda } from "./pda";
import type { Instruction } from "./tx";

export const DLMM_PROGRAM = "LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo";
export const MEMO_PROGRAM = "MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr";
/** The preset the desk's pairs use: index 262, bin step 50, base fee 0.4 %, fees in Y (SOL) only. */
export const DESK_PRESET = 262;
export const DESK_BIN_STEP = 50;
/** A bin array holds 70 bins. */
export const BINS_PER_ARRAY = 70;
/** DLMM's fee precision (1e9) and cap (10 %). */
export const FEE_PRECISION = 1_000_000_000n;
export const MAX_FEE_RATE = 100_000_000n;
export const MAX_BIN_ID = 443_636;

/** `sha256("global:swap2")[..8]`, `sha256("account:LbPair")[..8]`, `sha256("account:BinArray")[..8]` (checked in the tests). */
export const DLMM_DISC = {
  swap2: Uint8Array.from([65, 75, 63, 76, 235, 91, 91, 136]),
  lbPair: Uint8Array.from([33, 11, 49, 98, 181, 101, 177, 13]),
  binArray: Uint8Array.from([92, 142, 92, 220, 5, 148, 70, 181]),
};
export const LB_PAIR_LEN = 904;
export const BIN_ARRAY_LEN = 10_136;

const m = (pubkey: Address, isWritable = false, isSigner = false) => ({ pubkey, isWritable, isSigner });
const byteCmp = (a: Uint8Array, b: Uint8Array) => { for (let i = 0; i < 32; i++) if (a[i] !== b[i]) return a[i] - b[i]; return 0; };
const le = (v: bigint, size: number) => { const out = new Uint8Array(size); let n = v < 0n ? v + (1n << BigInt(8 * size)) : v; for (let i = 0; i < size; i++) { out[i] = Number(n & 0xffn); n >>= 8n; } return out; };

// ---- addresses ---------------------------------------------------------------------------------------------------------------------

export const dlmmEventAuthority = (): Address => pda(["__event_authority"], DLMM_PROGRAM);
/** `["preset_parameter2", index u16 LE]`. */
export const presetAddress = (index: number): Address => pda(["preset_parameter2", le(BigInt(index), 2)], DLMM_PROGRAM);
/** A pair on a preset: `[preset, min(x, y), max(x, y)]`, one per (preset, mint pair). */
export function pairAddress(preset: Address, x: Address, y: Address): Address {
  const a = keyBytes(x), b = keyBytes(y);
  const [lo, hi] = byteCmp(a, b) < 0 ? [a, b] : [b, a];
  return pda([keyBytes(preset), lo, hi], DLMM_PROGRAM);
}
export const reserveAddress = (pair: Address, mint: Address): Address => pda([keyBytes(pair), keyBytes(mint)], DLMM_PROGRAM);
export const oracleAddress = (pair: Address): Address => pda(["oracle", keyBytes(pair)], DLMM_PROGRAM);
/** `["bin_array", pair, index i64 LE]`; the index of bin `b` is ⌊b / 70⌋ (negative bins included). */
export const binArrayAddress = (pair: Address, index: number): Address => pda(["bin_array", keyBytes(pair), le(BigInt(index), 8)], DLMM_PROGRAM);
export const arrayIndex = (bin: number): number => Math.floor(bin / BINS_PER_ARRAY);
/** `["position", pair, base, lower i32 LE, width i32 LE]` (the desk's position: base = owner = K_desk). */
export const positionAddress = (pair: Address, base: Address, lower: number, width: number): Address =>
  pda(["position", keyBytes(pair), keyBytes(base), le(BigInt(lower), 4), le(BigInt(width), 4)], DLMM_PROGRAM);
/** The bin arrays a swap walks, in the swap's direction from the active bin's array (DLMM refuses any other order): `count` of
 *  them, ascending when `up` (a buy of wX: SOL in, wX out), descending otherwise. */
export function swapArrays(pair: Address, activeId: number, up: boolean, count: number): Address[] {
  const a = arrayIndex(activeId);
  return Array.from({ length: count }, (_, k) => binArrayAddress(pair, up ? a + k : a - k));
}

/** A pair the builders act on: `x` = wX, `y` = wSOL (so the price is SOL per wX and fees arrive in SOL), and its bin step. */
export type Lb = { pair: Address; x: Address; y: Address; binStep: number };
/** The desk's pair on DESK_PRESET for wX `x` against wSOL. */
export const lbDesk = (x: Address): Lb => ({ pair: pairAddress(presetAddress(DESK_PRESET), x, NATIVE_MINT), x, y: NATIVE_MINT, binStep: DESK_BIN_STEP });

// ---- instructions ------------------------------------------------------------------------------------------------------------------

/** `swap2 { amount_in, min_amount_out, remaining_accounts_info: { slices: [] } }`: direction follows `input` / `output` (a buy of wX:
 *  wSOL in → wX out). `arrays` are the bin arrays in the swap's order (swapArrays); the user signs. An absent optional account
 *  (bitmap extension, host fee) is the program id, Anchor's convention. */
export function swap2(lb: Lb, user: Address, input: Address, output: Address, amountIn: bigint, minOut: bigint, arrays: Address[]): Instruction {
  return {
    programId: DLMM_PROGRAM,
    keys: [
      m(lb.pair, true), m(DLMM_PROGRAM), m(reserveAddress(lb.pair, lb.x), true), m(reserveAddress(lb.pair, lb.y), true), m(input, true), m(output, true),
      m(lb.x), m(lb.y), m(oracleAddress(lb.pair), true), m(DLMM_PROGRAM), m(user, false, true), m(TOKEN_PROGRAM), m(TOKEN_PROGRAM), m(MEMO_PROGRAM),
      m(dlmmEventAuthority()), m(DLMM_PROGRAM), ...arrays.map((a) => m(a, true)),
    ],
    data: new Writer().bytes(DLMM_DISC.swap2).u64(amountIn).u64(minOut).u32(0).done(),
  };
}

// ---- DLMM's price arithmetic (Q64.64) -------------------------------------------------------------------------------------------------

const ONE_Q64 = 1n << 64n;
const U128_MAX = (1n << 128n) - 1n;
/** ⌊a · b / 2^64⌋ (null when it does not fit a u128), or its rounding-up form. */
function mulShr64(a: bigint, b: bigint, up: boolean): bigint | null {
  const p = a * b, q = p >> 64n;
  if (q > U128_MAX) return null;
  if (up && (p & ((1n << 64n) - 1n)) !== 0n) return q + 1n > U128_MAX ? null : q + 1n;
  return q;
}
/** a · 2^64 / b for a < 2^64, rounded down or up. */
function shl64Div(a: bigint, b: bigint, up: boolean): bigint | null {
  if (b === 0n || a >> 64n !== 0n) return null;
  const n = a << 64n, q = n / b;
  return up && n % b !== 0n ? q + 1n : q;
}

/** `price(id)` = (1 + bin_step / 10⁴)^id as Q64.64: Y base units per X base unit (for wX/SOL, 9 decimals each: SOL per wX).
 *  Square-and-multiply on the inverted base for a positive id, exactly as DLMM computes it. null outside the range where it fits. */
export function priceQ64(binStep: number, id: number): bigint | null {
  if (id === 0) return ONE_Q64;
  const base = ONE_Q64 + ((BigInt(binStep) << 64n) / 10_000n);
  let e = Math.abs(id), sq = U128_MAX / base, result = ONE_Q64;
  while (e > 0) {
    if (e & 1) { const r = mulShr64(result, sq, false); if (r === null) return null; result = r; }
    e >>= 1;
    if (e > 0) { const s = mulShr64(sq, sq, false); if (s === null) return null; sq = s; }
  }
  if (result === 0n) return null;
  return id > 0 ? U128_MAX / result : result;
}
/** The price of a floor in Q64.64: lamports_per_whole · 2^64 / 10⁹ (a whole wX is 10⁹ base units). */
export const priceOfFloor = (lamportsPerWhole: bigint): bigint => (lamportsPerWhole << 64n) / 1_000_000_000n;
/** The bin whose price is nearest `p`, within ±MAX_BIN_ID (binary search on the monotone priceQ64). */
export function binForPrice(binStep: number, p: bigint): number | null {
  if (p <= 0n || binStep <= 0) return null;
  let lo = -MAX_BIN_ID, hi = MAX_BIN_ID;
  while (lo < hi) {
    const mid = lo + Math.floor((hi - lo + 1) / 2), v = priceQ64(binStep, mid);
    if (v === null) { if (mid > 0) hi = mid - 1; else lo = mid; } else if (v <= p) lo = mid; else hi = mid - 1;
  }
  const below = priceQ64(binStep, lo), above = priceQ64(binStep, lo + 1);
  if (below === null) return null;
  return above !== null && above - p < (p > below ? p - below : 0n) ? lo + 1 : lo;
}
/** SOL per whole wX at a bin (a float, for display only). */
export const solPerWxAt = (binStep: number, id: number): number | null => { const p = priceQ64(binStep, id); return p === null ? null : Number(p) / 2 ** 64; };

// ---- accounts ------------------------------------------------------------------------------------------------------------------------

const pk = (d: Uint8Array, at: number) => b58encode(d.subarray(at, at + 32));

/** `LbPair` (904 bytes): `StaticParameters` at 8, `VariableParameters` at 40, `pair_type` @75, `active_id` @76, `bin_step` @80,
 *  `status` @82 (0 enabled; Meteora's operator can disable a pair), the mints @88 and @120, the reserves @152 and @184. */
export type LbPair = {
  baseFactor: number; filterPeriod: number; decayPeriod: number; reductionFactor: number; variableFeeControl: number; maxVolatilityAccumulator: number;
  minBinId: number; maxBinId: number; protocolShare: number; baseFeePowerFactor: number;
  volatilityAccumulator: number; volatilityReference: number; indexReference: number; lastUpdateTimestamp: number;
  pairType: number; activeId: number; binStep: number; status: number;
  tokenXMint: Address; tokenYMint: Address; reserveX: Address; reserveY: Address;
};
export function decodeLbPair(d: Uint8Array): LbPair {
  if (d.length !== LB_PAIR_LEN || !sameBytes(d.subarray(0, 8), DLMM_DISC.lbPair)) throw new Error("not a DLMM LbPair");
  return {
    baseFactor: u16At(d, 8), filterPeriod: u16At(d, 10), decayPeriod: u16At(d, 12), reductionFactor: u16At(d, 14), variableFeeControl: u32At(d, 16),
    maxVolatilityAccumulator: u32At(d, 20), minBinId: i32At(d, 24), maxBinId: i32At(d, 28), protocolShare: u16At(d, 32), baseFeePowerFactor: d[34],
    volatilityAccumulator: u32At(d, 40), volatilityReference: u32At(d, 44), indexReference: i32At(d, 48), lastUpdateTimestamp: Number(i64At(d, 56)),
    pairType: d[75], activeId: i32At(d, 76), binStep: u16At(d, 80), status: d[82],
    tokenXMint: pk(d, 88), tokenYMint: pk(d, 120), reserveX: pk(d, 152), reserveY: pk(d, 184),
  };
}
/** The base fee rate in 1e9: base_factor × bin_step × 10 × 10^base_fee_power_factor (preset 262: 8,000 × 50 × 10 = 0.4 %). */
export const baseFeeRate = (p: LbPair): bigint => BigInt(p.baseFactor) * BigInt(p.binStep) * 10n * 10n ** BigInt(p.baseFeePowerFactor);
/** The variable fee rate in 1e9 for a volatility accumulator `v`: ⌈control · (v · bin_step)² / 10¹¹⌉. */
export function variableFeeRate(p: LbPair, v: number): bigint {
  const x = BigInt(v) * BigInt(p.binStep), n = BigInt(p.variableFeeControl) * x * x, den = 100_000_000_000n;
  return (n + den - 1n) / den;
}
/** The total rate for accumulator `v`, capped at 10 %. */
export function feeRate(p: LbPair, v: number): bigint { const r = baseFeeRate(p) + variableFeeRate(p, v); return r > MAX_FEE_RATE ? MAX_FEE_RATE : r; }
/** The volatility references a swap at `now` starts from (DLMM `update_references`): after `filter_period` the index reference is the
 *  active bin; after `decay_period` the volatility reference is 0, else accumulator × reduction_factor / 10⁴. */
export function referencesAt(p: LbPair, now: number): { vol: number; idx: number } {
  const elapsed = Math.max(0, now - p.lastUpdateTimestamp);
  const idx = elapsed >= p.filterPeriod ? p.activeId : p.indexReference;
  const vol = elapsed >= p.filterPeriod ? (elapsed >= p.decayPeriod ? 0 : Math.floor((p.volatilityAccumulator * p.reductionFactor) / 10_000)) : p.volatilityReference;
  return { vol, idx };
}
/** The all-in rate a swap would pay in the active bin right now (what the ticket shows), 1e9. */
export function currentFeeRate(p: LbPair, now: number): bigint {
  const { vol, idx } = referencesAt(p, now);
  return feeRate(p, Math.min(vol + Math.abs(idx - p.activeId) * 10_000, p.maxVolatilityAccumulator));
}
export const pairEnabled = (p: LbPair) => p.status === 0;

/** One bin of a `BinArray` (144 bytes): `amount_x` @0, `amount_y` @8, `price` @16 (Q64.64), `liquidity_supply` @32. */
export type Bin = { amountX: bigint; amountY: bigint; price: bigint; liquiditySupply: bigint };
export type BinArray = { index: number; lbPair: Address; bins: Bin[] };
export function decodeBinArray(d: Uint8Array): BinArray {
  if (d.length !== BIN_ARRAY_LEN || !sameBytes(d.subarray(0, 8), DLMM_DISC.binArray)) throw new Error("not a DLMM BinArray");
  const bins: Bin[] = [];
  for (let k = 0; k < BINS_PER_ARRAY; k++) { const o = 56 + 144 * k; bins.push({ amountX: u64At(d, o), amountY: u64At(d, o + 8), price: u128At(d, o + 16), liquiditySupply: u128At(d, o + 32) }); }
  return { index: Number(i64At(d, 8)), lbPair: pk(d, 24), bins };
}
/** Every bin of the decoded arrays, by bin id. */
export function binsOf(arrays: BinArray[]): Map<number, Bin> {
  const out = new Map<number, Bin>();
  for (const a of arrays) a.bins.forEach((b, k) => out.set(a.index * BINS_PER_ARRAY + k, b));
  return out;
}

// ---- a quote over the bins ---------------------------------------------------------------------------------------------------------

/** What a swap would do (DLMM's `swap_exact_in` over the bins; `swapForY` = X in, Y out: a SELL of wX; otherwise a BUY of wX). */
export type DlmmQuote = {
  amountInUsed: bigint; amountOut: bigint; fee: bigint;
  /** The active bin after the swap. */
  endId: number;
  /** The whole input was used (false: the liquidity the arrays show ran out first). */
  complete: boolean;
  /** The all-in rate of the last bin touched, 1e9. */
  lastRate: bigint;
};
/** Walks the bins from the active one in the swap's direction. `bins` must hold every bin the swap can reach (the arrays the swap is
 *  given); a bin missing from `bins` ends the walk (`complete` false). The fee of each bin is DLMM's: the base rate plus the variable
 *  rate of the volatility the walk has accumulated since the references. */
export function quoteDlmmSwap(pair: LbPair, bins: Map<number, Bin>, swapForY: boolean, amountIn: bigint, now: number): DlmmQuote {
  const { vol, idx } = referencesAt(pair, now);
  let left = amountIn, out = 0n, feeTotal = 0n, lastRate = baseFeeRate(pair);
  let id = pair.activeId, endId = id, complete = true;
  const step = swapForY ? -1 : 1;
  while (left > 0n) {
    const b = bins.get(id);
    if (!b) { complete = false; break; }
    const has = swapForY ? b.amountY > 0n : b.amountX > 0n;
    if (has) {
      const rate = feeRate(pair, Math.min(vol + Math.abs(idx - id) * 10_000, pair.maxVolatilityAccumulator));
      lastRate = rate;
      const maxOut = swapForY ? b.amountY : b.amountX;
      const maxIn = swapForY ? shl64Div(maxOut, b.price, true) : mulShr64(maxOut, b.price, true);
      if (maxIn === null) throw new Error("bin price overflow");
      // compute_fee: the fee on an amount that EXCLUDES it: ⌈x · rate / (1e9 − rate)⌉
      const maxFee = (maxIn * rate + (FEE_PRECISION - rate) - 1n) / (FEE_PRECISION - rate);
      if (left >= maxIn + maxFee) {
        left -= maxIn + maxFee; out += maxOut; feeTotal += maxFee; endId = id;
      } else {
        const fee = (left * rate + FEE_PRECISION - 1n) / FEE_PRECISION, net = left - fee;
        const g = swapForY ? mulShr64(net, b.price, false) : shl64Div(net, b.price, false);
        if (g === null) throw new Error("bin price overflow");
        out += g > maxOut ? maxOut : g; feeTotal += fee; left = 0n; endId = id;
        break;
      }
    }
    if (left === 0n) break;
    id += step; endId = id;
    if (id < pair.minBinId || id > pair.maxBinId) { complete = false; break; }
  }
  return { amountInUsed: amountIn - left, amountOut: out, fee: feeTotal, endId, complete, lastRate };
}
