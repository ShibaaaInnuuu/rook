/**
 * The site's own two-hop buy (docs/v9/canister-only-solana.md §1 item 2, §5.1, W12): SOL → wX on the collection's wX/SOL pool
 * (a Meteora DLMM pair, §17) → coin on its DBC curve (`swap2`, exact in) or, graduated, on its DAMM v2 COIN/wX pool (`swap`), in ONE
 * legacy transaction the wallet signs. It is required, not a fallback: Jupiter drops a coin still on its curve after its grace period
 * and any wX whose pool fails its liquidity test. The instruction list is the canister's `solix2::route::two_hop_buy` (the reference;
 * its two landed bundles are test/fixtures/sol/two-hop.json), with the first hop on DLMM as §17 decided after those bundles landed:
 * wrap `solIn`, buy at least `wxMin` wX, spend exactly `wxMin` on the coin hop for at least `coinMin`, close the wSOL account. Any wX
 * above `wxMin` stays in the wallet's wX account (swept by a later buy). The send path adds the compute-budget instructions in front.
 *
 * Also here: the symmetric sell (coin → wX, then exactly the wX the quote promised → SOL, the dust of wX staying in the wallet), the
 * one-hop buy for a wallet that already holds wX (the direct mint, §6, or a wX bought before), the quote of each path from the on-chain
 * accounts (both hops, slippage applied per hop), and the reads that feed it. Every trade is simulated on chain before the wallet is
 * asked (src/rook/sol/trade.ts `boundBySimulation`); the quote here is the preview and the first hop's floor.
 */
import { decodeDammPool, dammSwap, migratedPool, quoteDammSwap, type DammPool, type DammPoolRef } from "./damm";
import { closeAccount, createAtaIdempotent, quoteSwap as quoteCurveSwap, swap, swap2, syncNative, transferLamports, type DbcConfig, type DbcPool } from "./dbc";
import { arrayIndex, binArrayAddress, binsOf, decodeBinArray, decodeLbPair, lbDesk, quoteDlmmSwap, solPerWxAt, swap2 as dlmmSwap2, DLMM_PROGRAM, type Bin, type Lb, type LbPair } from "./dlmm";
import { NATIVE_MINT, type Address } from "./keys";
import { ata } from "./pda";
import type { Rpc } from "./rpc";
import { withSlippage, type PoolRef } from "./trade";
import type { Instruction } from "./tx";

/** The coin hop: the curve (quote = wX), or the graduated DAMM v2 COIN/wX pool. */
export type CoinHop = { kind: "curve"; curve: PoolRef } | { kind: "graduated"; pool: DammPoolRef };
const coinOf = (hop: CoinHop) => (hop.kind === "curve" ? hop.curve.baseMint : hop.pool.a);
const wxOf = (hop: CoinHop) => (hop.kind === "curve" ? hop.curve.quoteMint ?? NATIVE_MINT : hop.pool.b);

/** The coin hop alone: exactly `wxIn` wX from the user's wX account for at least `coinMin` coins. */
export function coinHopIx(user: Address, hop: CoinHop, wxIn: bigint, coinMin: bigint): Instruction {
  const uWx = ata(user, wxOf(hop)), uCoin = ata(user, coinOf(hop));
  return hop.kind === "curve"
    ? swap2({ ...hop.curve, quoteMint: wxOf(hop), user, input: uWx, output: uCoin, amount0: wxIn, amount1: coinMin, swapMode: 0 })
    : dammSwap(hop.pool, user, uWx, uCoin, wxIn, coinMin);
}
/** The coin hop the other way: exactly `coinsIn` coins for at least `wxMin` wX. */
export function coinSellIx(user: Address, hop: CoinHop, coinsIn: bigint, wxMin: bigint): Instruction {
  const uWx = ata(user, wxOf(hop)), uCoin = ata(user, coinOf(hop));
  return hop.kind === "curve"
    ? swap({ ...hop.curve, quoteMint: wxOf(hop), user, input: uCoin, output: uWx, amountIn: coinsIn, minimumAmountOut: wxMin })
    : dammSwap(hop.pool, user, uCoin, uWx, coinsIn, wxMin);
}

/**
 * SOL → wX → coin (the canister's `two_hop_buy` without its two compute-budget instructions, which the send path adds): create the
 * wSOL account, wrap `solIn`, create the wX account, DLMM `swap2` for at least `wxMin`, create the coin account, the coin hop spending
 * exactly `wxMin` for at least `coinMin`, close the wSOL account (`unwrap`; left open so a simulation can read it, as trade.ts does).
 * `arrays`: the pair's bin arrays from the active bin's upward (readWxPool gives them).
 */
export function twoHopBuyIxs(user: Address, lb: Lb, hop: CoinHop, solIn: bigint, wxMin: bigint, coinMin: bigint, arrays: Address[], unwrap = true): Instruction[] {
  const wx = wxOf(hop), coin = coinOf(hop);
  const uWsol = ata(user, NATIVE_MINT), uWx = ata(user, wx), uCoin = ata(user, coin);
  return [
    createAtaIdempotent(user, uWsol, user, NATIVE_MINT), transferLamports(user, uWsol, solIn), syncNative(uWsol),
    createAtaIdempotent(user, uWx, user, wx),
    dlmmSwap2(lb, user, uWsol, uWx, solIn, wxMin, arrays),
    createAtaIdempotent(user, uCoin, user, coin),
    coinHopIx(user, hop, wxMin, coinMin),
    ...(unwrap ? [closeAccount(uWsol, user, user)] : []),
  ];
}
/** A wallet that already holds wX: the coin hop alone (the coin account created idempotently). */
export function wxBuyIxs(user: Address, hop: CoinHop, wxIn: bigint, coinMin: bigint): Instruction[] {
  return [createAtaIdempotent(user, ata(user, coinOf(hop)), user, coinOf(hop)), coinHopIx(user, hop, wxIn, coinMin)];
}
/** coin → wX → SOL: the coin hop for at least `wxOut` wX, then exactly `wxOut` wX into the wSOL account for at least `solMin`, unwrapped.
 *  `arrays`: the pair's bin arrays from the active bin's downward. */
export function twoHopSellIxs(user: Address, lb: Lb, hop: CoinHop, coinsIn: bigint, wxOut: bigint, solMin: bigint, arrays: Address[], unwrap = true): Instruction[] {
  const uWsol = ata(user, NATIVE_MINT), uWx = ata(user, wxOf(hop));
  return [
    createAtaIdempotent(user, uWx, user, wxOf(hop)),
    coinSellIx(user, hop, coinsIn, wxOut),
    createAtaIdempotent(user, uWsol, user, NATIVE_MINT),
    dlmmSwap2(lb, user, uWx, uWsol, wxOut, solMin, arrays),
    ...(unwrap ? [closeAccount(uWsol, user, user)] : []),
  ];
}

// ---- reads ---------------------------------------------------------------------------------------------------------------------------

/** How many bin arrays each side of the active one a read fetches (70 bins each: ±2 arrays is ±140 bins, far past a ±10 % band). */
export const ARRAYS_A_SIDE = 2;
/** The collection's wX/SOL pair as read from the chain: the pair, its bins around the active one, and the arrays a swap each way is given
 *  (the ones that exist, contiguous from the active bin's array, in the swap's order). null: no pair for this wX yet. */
export type WxPool = { lb: Lb; pair: LbPair; bins: Map<number, Bin>; arraysUp: Address[]; arraysDown: Address[]; readAt: number };
export async function readWxPool(rpc: Rpc, wx: Address): Promise<WxPool | null> {
  const lb = lbDesk(wx);
  const acc = await rpc.account(lb.pair);
  if (!acc || acc.owner !== DLMM_PROGRAM) return null;
  const pair = decodeLbPair(acc.data);
  const a0 = arrayIndex(pair.activeId);
  const idx = Array.from({ length: 2 * ARRAYS_A_SIDE + 1 }, (_, k) => a0 - ARRAYS_A_SIDE + k);
  const accs = await rpc.accounts(idx.map((i) => binArrayAddress(lb.pair, i)));
  const have = new Map<number, Address>();
  const arrays = [];
  for (let k = 0; k < idx.length; k++) {
    const a = accs[k];
    if (!a || a.owner !== DLMM_PROGRAM) continue;
    try { arrays.push(decodeBinArray(a.data)); have.set(idx[k], binArrayAddress(lb.pair, idx[k])); } catch { /* not a bin array */ }
  }
  const run = (step: 1 | -1) => { const out: Address[] = []; for (let i = a0; have.has(i); i += step) out.push(have.get(i)!); return out; };
  return { lb, pair, bins: binsOf(arrays), arraysUp: run(1), arraysDown: run(-1), readAt: Math.floor(Date.now() / 1000) };
}
/** SOL per whole wX at the pair's active bin (a float, for display), and its USD view. */
export const wxPriceSol = (w: Pick<WxPool, "pair"> | null | undefined): number | null => (w ? solPerWxAt(w.pair.binStep, w.pair.activeId) : null);

/** The graduated pool's account, decoded; null while it does not exist (the curve has not migrated). */
export async function readMigratedPool(rpc: Rpc, coin: Address, wx: Address): Promise<{ ref: DammPoolRef; pool: DammPool } | null> {
  const ref = migratedPool(coin, wx);
  const acc = await rpc.account(ref.pool);
  if (!acc) return null;
  try { return { ref, pool: decodeDammPool(acc.data) }; } catch { return null; }
}

// ---- quotes --------------------------------------------------------------------------------------------------------------------------

/** The coin hop's state for a quote: the curve (its config and pool), or the graduated pool. */
export type HopState = { kind: "curve"; cfg: DbcConfig; pool: DbcPool } | { kind: "graduated"; pool: DammPool };

export type BuyQuote = {
  /** Hop 1: wX the pair would give for `solIn`, the SOL fee it keeps, and the floor the transaction carries (the coin hop spends exactly it). */
  wxOut: bigint; wxMin: bigint; feeSol: bigint; dlmmRate: bigint;
  /** Hop 2: coins for `wxMin`, the wX fee the curve or pool keeps, the floor. */
  coinOut: bigint; coinMin: bigint; feeWx: bigint; coinFeeNumerator: bigint;
};
/** Both hops from the accounts as read, each hop's floor at `slippageBps` below its quote. null: the pair's visible depth cannot fill
 *  `solIn` (`complete` false), or the coin hop cannot fill `wxMin` (the curve's end, the pool's range). */
export function quoteTwoHopBuy(w: Pick<WxPool, "pair" | "bins">, hop: HopState, solIn: bigint, slippageBps: number, now: number): BuyQuote | null {
  if (solIn <= 0n) return null;
  const q1 = quoteDlmmSwap(w.pair, w.bins, false, solIn, now);
  if (!q1.complete || q1.amountOut <= 0n) return null;
  const wxMin = withSlippage(q1.amountOut, slippageBps);
  if (wxMin <= 0n) return null;
  const q2 = hop.kind === "curve" ? quoteCurveSwap(hop.cfg, hop.pool, "buy", wxMin, BigInt(now)) : quoteDammSwap(hop.pool, "buy", wxMin);
  if (!q2 || q2.amountOut <= 0n) return null;
  return { wxOut: q1.amountOut, wxMin, feeSol: q1.fee, dlmmRate: q1.lastRate, coinOut: q2.amountOut, coinMin: withSlippage(q2.amountOut, slippageBps), feeWx: q2.fee, coinFeeNumerator: q2.feeNumerator };
}

export type SellQuote = {
  /** Hop 1: wX for `coinsIn`, the wX fee, the floor (hop 2 spends exactly it). */
  wxOut: bigint; wxMin: bigint; feeWx: bigint; coinFeeNumerator: bigint;
  /** Hop 2: SOL for `wxMin`, the SOL fee, the floor. */
  solOut: bigint; solMin: bigint; feeSol: bigint; dlmmRate: bigint;
};
export function quoteTwoHopSell(w: Pick<WxPool, "pair" | "bins">, hop: HopState, coinsIn: bigint, slippageBps: number, now: number): SellQuote | null {
  if (coinsIn <= 0n) return null;
  const q1 = hop.kind === "curve" ? quoteCurveSwap(hop.cfg, hop.pool, "sell", coinsIn, BigInt(now)) : quoteDammSwap(hop.pool, "sell", coinsIn);
  if (!q1 || q1.amountOut <= 0n) return null;
  const wxMin = withSlippage(q1.amountOut, slippageBps);
  if (wxMin <= 0n) return null;
  const q2 = quoteDlmmSwap(w.pair, w.bins, true, wxMin, now);
  if (!q2.complete || q2.amountOut <= 0n) return null;
  return { wxOut: q1.amountOut, wxMin, feeWx: q1.fee, coinFeeNumerator: q1.feeNumerator, solOut: q2.amountOut, solMin: withSlippage(q2.amountOut, slippageBps), feeSol: q2.fee, dlmmRate: q2.lastRate };
}
/** The one-hop buy's quote (wX the wallet holds → coin). */
export function quoteWxBuy(hop: HopState, wxIn: bigint, slippageBps: number, now: number): { coinOut: bigint; coinMin: bigint; feeWx: bigint } | null {
  if (wxIn <= 0n) return null;
  const q = hop.kind === "curve" ? quoteCurveSwap(hop.cfg, hop.pool, "buy", wxIn, BigInt(now)) : quoteDammSwap(hop.pool, "buy", wxIn);
  return q && q.amountOut > 0n ? { coinOut: q.amountOut, coinMin: withSlippage(q.amountOut, slippageBps), feeWx: q.fee } : null;
}
/** The one-hop sell's quote (coin → wX, the wX staying in the wallet: no wX/SOL pool yet, §5.3's "no pool" stage). */
export function quoteWxSell(hop: HopState, coinsIn: bigint, slippageBps: number, now: number): { wxOut: bigint; wxMin: bigint; feeWx: bigint } | null {
  if (coinsIn <= 0n) return null;
  const q = hop.kind === "curve" ? quoteCurveSwap(hop.cfg, hop.pool, "sell", coinsIn, BigInt(now)) : quoteDammSwap(hop.pool, "sell", coinsIn);
  return q && q.amountOut > 0n ? { wxOut: q.amountOut, wxMin: withSlippage(q.amountOut, slippageBps), feeWx: q.fee } : null;
}
/** The one-hop sell's instructions: the wX account created idempotently, then the coin hop the other way. */
export function wxSellIxs(user: Address, hop: CoinHop, coinsIn: bigint, wxMin: bigint): Instruction[] {
  return [createAtaIdempotent(user, ata(user, wxOf(hop)), user, wxOf(hop)), coinSellIx(user, hop, coinsIn, wxMin)];
}
