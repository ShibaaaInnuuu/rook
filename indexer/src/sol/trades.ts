/**
 * A decoded swap event → one trade row. Pure. The rules (docs/v9/indexer-solana.md §5):
 *  - side: DBC direction 1 (quote→base) is a buy; DAMM v2 direction 0 is A→B, so it is a sell when token A is the coin.
 *  - sol: what the trader paid (buy: the input including fee) or received (sell: the output, after fee), in the QUOTE's base units.
 *    Since W13 (canister-only design §3) a Rook config's quote is its collection's wX, not SOL: the field keeps its name (and the
 *    `trades.sol` column its), but holds wX base units on a wX-quoted config; `price` is then wX per coin. The service turns
 *    both into SOL through the wX/SOL rate of the trade's time (wx.ts), never here.
 *  - price: the curve-side, fee-free price. Where the fee was taken is read from the event itself: an input whose
 *    `included_fee` and `excluded_fee` amounts differ paid the fee on the input; otherwise the fee came out of the output. A v1
 *    `EvtSwap` (no excluded amount) falls back to the config's collect_fee_mode (0 = quote token, 1 = output token).
 */
import type { DammSwapEvent, DbcSwapEvent } from "./dbc.js";

export type TradeRow = { side: "buy" | "sell"; venue: "curve" | "pool"; sol: bigint; tokens: bigint; fee: bigint | null; price: number };

/** Whole quote units per whole coin from quote base units and coin base units. The quote has 9 decimals (SOL, or a wX: §2). */
export const priceOf = (quote: bigint, raw: bigint, decimals: number, quoteDecimals = 9): number => (raw === 0n ? 0 : (Number(quote) / 10 ** quoteDecimals) / (Number(raw) / 10 ** decimals));

/** A DBC swap. `feeMode` is the config's collect_fee_mode (only used for a v1-only event); `qd` the quote mint's decimals. */
export function dbcTrade(ev: DbcSwapEvent, decimals: number, feeMode: number, qd = 9): TradeRow | null {
  const fees = ev.tradingFee + ev.protocolFee + ev.referralFee;
  const buy = ev.direction === 1;
  const inFee = ev.inputExcludedFee !== null ? ev.inputExcludedFee !== ev.inputAmount : buy && feeMode === 0;
  if (buy) {
    const sol = ev.inputAmount, tokens = ev.outputAmount;
    if (tokens === 0n) return null;
    // fee on the input: it is the quote; fee on the output: it is coins
    const price = inFee ? priceOf(ev.inputExcludedFee ?? sol - fees, tokens, decimals, qd) : priceOf(sol, tokens + fees, decimals, qd);
    return { side: "buy", venue: "curve", sol, tokens, fee: inFee ? fees : null, price };
  }
  const tokens = ev.inputAmount, sol = ev.outputAmount;
  if (tokens === 0n) return null;
  // a sell's fee on the input would be in coins (output-token mode never does that, but read it as the event says)
  const price = inFee ? priceOf(sol, ev.inputExcludedFee ?? tokens, decimals, qd) : priceOf(sol + fees, tokens, decimals, qd);
  return { side: "sell", venue: "curve", sol, tokens, fee: inFee ? null : fees, price };
}

/** A DAMM v2 swap; `coinIsA` from the pool account (DBC migrates with the coin as token A, the quote as B). */
export function dammTrade(ev: DammSwapEvent, decimals: number, coinIsA: boolean, qd = 9): TradeRow | null {
  const fees = ev.claimingFee + ev.protocolFee + ev.compoundingFee + ev.referralFee;
  const aToB = ev.direction === 0;
  const buy = coinIsA ? !aToB : aToB;
  const inFee = ev.inputExcludedFee !== ev.inputAmount;
  if (buy) {
    const sol = ev.inputAmount, tokens = ev.outputAmount;
    if (tokens === 0n) return null;
    const price = inFee ? priceOf(ev.inputExcludedFee, tokens, decimals, qd) : priceOf(sol, tokens + fees, decimals, qd);
    return { side: "buy", venue: "pool", sol, tokens, fee: inFee ? fees : null, price };
  }
  const tokens = ev.inputAmount, sol = ev.outputAmount;
  if (tokens === 0n) return null;
  const price = inFee ? priceOf(sol, ev.inputExcludedFee, decimals, qd) : priceOf(sol + fees, tokens, decimals, qd);
  return { side: "sell", venue: "pool", sol, tokens, fee: inFee ? null : fees, price };
}

/** The total order of trades: slot, then the transaction's index in its block, then the swap's index in the transaction. */
export const ordOf = (slot: number, txIndex: number, n: number): number => slot * 1_000_000 + Math.min(9_999, txIndex) * 100 + Math.min(99, n);
export const cursorOf = (ord: number): string => `${Math.floor(ord / 1_000_000)}:${ord % 1_000_000}`;
export function ordFromCursor(c: string): number | null {
  const m = /^(\d{1,12}):(\d{1,7})$/.exec(c);
  if (!m) return null;
  const seq = Number(m[2]);
  return seq < 1_000_000 ? Number(m[1]) * 1_000_000 + seq : null;
}
