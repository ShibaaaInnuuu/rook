import type { Holder, HolderExclusion, HolderPage, Trade } from "../api/types";

/**
 * The arithmetic and the words behind the trades-and-holders panel, apart from its drawing, so both can be tested.
 * docs/phase13.md §7.
 *
 * Every number here can be **not known**, and each unknown has its own sentence: an unread supply is not a zero share, a
 * missing dollar rate is not a dollar value of nothing, and a recording that stops is not a coin that never traded.
 */

/** A share of supply as a percentage. Null in, null out: the supply has not been read and there is no percentage to print. */
export function sharePct(share: number | null | undefined): string | null {
  if (share === null || share === undefined || !Number.isFinite(share)) return null;
  const p = share * 100;
  if (p >= 10) return `${p.toFixed(1)}%`;
  if (p >= 0.1) return `${p.toFixed(2)}%`;
  if (p > 0) return `${p.toPrecision(2)}%`;
  return "0%";
}

/** A trade's pair-asset leg in dollars, when Phase 12's rate is known. Null is "no rate", never a dollar value of zero. */
export function usdOf(quoteWei: string, usdPerEth: number | null | undefined): number | null {
  if (usdPerEth === null || usdPerEth === undefined || !Number.isFinite(usdPerEth)) return null;
  const eth = Number(quoteWei) / 1e18;
  return Number.isFinite(eth) ? eth * usdPerEth : null;
}

/**
 * The curve's own leg of a trade, and what the fee and the tax took out of it. Null for a pool trade, where the hook charges
 * outside the `Swap` event and the API does not guess.
 *
 * This exists because a row that shows a price beside a value and nothing else reads as broken arithmetic. `Trade.price` is
 * the CURVE-side price — the quote that reached the curve over the coins that left it — so that a buy and the sell after it
 * are continuous and a chart drawn from them has no sawtooth at every fee. `quoteWei` is the trader's own leg: what left
 * their wallet on a buy, what arrived in it on a sell. The two differ by the fee and the tax, in opposite directions, and a
 * reader multiplying price by size and getting a different number is right to distrust the page until it is told why.
 */
export function curveLeg(t: Trade): { curveWei: bigint; feeTaxWei: bigint } | null {
  if (t.feeWei === null || t.taxWei === null) return null;
  const quote = BigInt(t.quoteWei);
  const feeTax = BigInt(t.feeWei) + BigInt(t.taxWei);
  // a buy pays the fee on the way in, so the curve saw less; a sell pays it on the way out, so the curve paid more
  return { curveWei: t.side === "buy" ? quote - feeTax : quote + feeTax, feeTaxWei: feeTax };
}

/** Dollars, in the sizes a trade comes in: cents up to a thousand, then whole dollars, then abbreviated. */
export function money(usd: number | null): string | null {
  if (usd === null || !Number.isFinite(usd)) return null;
  if (usd >= 1e9) return `$${(usd / 1e9).toFixed(2)}B`;
  if (usd >= 1e6) return `$${(usd / 1e6).toFixed(2)}M`;
  if (usd >= 1_000) return `$${Math.round(usd).toLocaleString("en-US")}`;
  if (usd >= 0.01) return `$${usd.toFixed(2)}`;
  if (usd > 0) return "<$0.01";
  return "$0.00";
}

/** What each excluded address is, in words a reader can check against the coin's own Addresses section. */
export const EXCLUDED_IS: Record<HolderExclusion, string> = {
  coin: "the coin's own address",
  curve: "this coin's bonding curve, which is minted the whole supply and sells it",
  router: "this coin's fee router, ours",
  "pons-factory": "Pons's factory, which holds 2/7 of every supply for the pool a graduated coin gets",
  "pool-manager": "Uniswap v4's PoolManager, where a graduated coin's pool liquidity sits",
  hook: "the Pons hook on this coin's pool",
};

/**
 * Why a holder list is not a list of people, said once, above the list, and never softened.
 * On staging there is the second thing to say, which is that none of these wallets is anybody.
 */
export const holdersCaveat = (staging: boolean): string =>
  staging
    ? "One address, one row: nothing here groups a trader's wallets. On staging every one of them is a SIMULATED wallet — a key the simulator we run holds, trading money that is not money."
    : "One address, one row: nothing here groups a trader's several wallets, and a custodian holding for many people is one row.";

/** The rows of a page of trades, flattened, with the value each one moved in both units. */
export type TradeRow = Trade & { usd: number | null };
export const tradeRows = (pages: { items: Trade[] }[], usdPerEth: number | null | undefined): TradeRow[] =>
  pages.flatMap((p) => p.items).map((t) => ({ ...t, usd: usdOf(t.quoteWei, usdPerEth) }));

/** The rows of a page of holders, flattened. The indexer ranks them; this never re-sorts and never re-ranks. */
export const holderRows = (pages: HolderPage[]): Holder[] => pages.flatMap((p) => p.items);

/**
 * What a failed read of a list means. A recorded demo answers only the paths it holds, and a path it does not hold is an
 * error with those words in it — which is a different thing from an indexer that would not answer, and a different thing
 * again from a coin that has never traded. The panel prints whichever of the three it is.
 */
export const recordingRanOut = (error: unknown): boolean => /no recorded answer/.test(String((error as Error)?.message ?? error));
