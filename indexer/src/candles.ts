/**
 * docs/phase10.md §chart. The two pure pieces of a coin's chart: what a trade's price is, and how trades become candles.
 * Both are here so a test can hold them without a chain, a database or a clock.
 *
 * A price is **the pair asset per one whole coin** — on Robinhood Chain that is ETH per coin, a small number like 3e-9.
 * It is the CURVE-side price: the quote that actually reached the curve, divided by the coins that left it. Pons takes its
 * fee and the creator's tax out of the same leg on both sides, so the price a buyer pays is above the curve and the price a
 * seller receives is below it. Drawing those would put a permanent 7% zig-zag in the line that is fee, not price. The gross
 * legs are kept on the trade itself, so a page that wants the price actually paid can divide them.
 */
export type Res = "trade" | "5m" | "1h";
export const DAY_S = 86_400;
/** `trade` is every trade at its own second over 6 hours; the others bucket. A coin younger than the 6-hour span opens on it. */
export const RES: Record<Res, { seconds: number; span: number }> = {
  trade: { seconds: 0, span: 6 * 3_600 }, "5m": { seconds: 300, span: DAY_S }, "1h": { seconds: 3_600, span: 7 * DAY_S },
};
export const resOf = (s: string | null | undefined): Res => (s === "trade" || s === "1h" ? s : "5m");
export const YOUNG_S = RES.trade.span;
export const POINTS_MAX = 1_000, MARKS_MAX = 150;

/**
 * docs/phase12.md §4. What a reader actually picks, and the thing that replaced `res`.
 *
 * A range names a WINDOW, and the bucket is derived from it rather than chosen. That is the whole point: with `res=trade` a
 * reader could land on one candle per trade, where the step from one bar to the next is whatever the next buy cost — the
 * `+82.3%` that answers no question anyone asked. A window always divides into time, so it cannot happen here.
 *
 * `res` is kept working beside this because the currently-deployed site asks for it and must not break.
 */
export type Range = "5m" | "1h" | "6h" | "1d" | "all";
export const RANGE_KEYS: readonly Range[] = ["5m", "1h", "6h", "1d", "all"];
export const RANGES: Record<Exclude<Range, "all">, { seconds: number; span: number }> = {
  "5m": { seconds: 5, span: 300 }, "1h": { seconds: 60, span: 3_600 }, "6h": { seconds: 300, span: 6 * 3_600 }, "1d": { seconds: 900, span: DAY_S },
};
/** null rather than a silent "5m": an unrecognised range is the caller's mistake and is worth saying so. */
export const rangeOf = (s: string | null | undefined): Range | null => (RANGE_KEYS.includes(s as Range) ? (s as Range) : null);

/** The buckets `all` may choose from: five seconds up to a day. */
export const BUCKETS = [5, 15, 60, 300, 900, 3_600, 6 * 3_600, DAY_S] as const;
/** What `all` aims for. Comfortably under POINTS_MAX, and enough points that a coin's whole life still reads as a line. */
export const POINTS_TARGET = 240;
/** `all` is the one range whose window is not fixed, so its bucket is chosen: the finest that keeps the series a series. */
export function bucketFor(span: number): number {
  let b: number = BUCKETS.find((x) => span / x <= POINTS_TARGET) ?? BUCKETS[BUCKETS.length - 1];
  while (span / b > POINTS_MAX) b *= 2;                 // a coin older than the ladder's reach still gets a drawable series
  return b;
}
/** A coin younger than an hour would otherwise have `all` mean three seconds, which is a window nobody asked for. */
export const ALL_MIN_SPAN = 3_600;
/** `all` with no launch time to measure from: a month, which is longer than anything on this chain has lived. */
export const ALL_FALLBACK_SPAN = 30 * DAY_S;

/** The window and bucket a range means for THIS coin. `all` is the only one that has to look at the coin's age. */
export function windowOf(range: Range, launchedAt: number | null | undefined, nowS: number): { seconds: number; span: number } {
  if (range !== "all") return RANGES[range];
  const age = launchedAt === null || launchedAt === undefined ? ALL_FALLBACK_SPAN : nowS - launchedAt;
  const span = Math.max(ALL_MIN_SPAN, Math.ceil(age));
  return { seconds: bucketFor(span), span };
}

/** Where a chart opens now that there is no per-trade view to open on: the finest range the coin's own age can fill. */
export function initialRange(launchedAt: number | null | undefined, nowS: number): Range {
  if (launchedAt === null || launchedAt === undefined) return "1d";
  const age = nowS - launchedAt;
  if (age < RANGES["1h"].span) return "1h";
  if (age < RANGES["6h"].span) return "6h";
  if (age < RANGES["1d"].span) return "1d";
  return "all";
}

export type Candle = { t: number; o: number; h: number; l: number; c: number; v: number; n: number };
export type Point = { ts: number; p: number; v: number };

/**
 * Candles from points already in chain order. A bucket with no trade in it is LEFT OUT rather than filled: the series says
 * what happened and never invents a price for a minute nobody traded in. A bucket opens at the previous close, so the line
 * is continuous where trading is not.
 */
export function candles(points: readonly Point[], seconds: number): Candle[] {
  const out: Candle[] = [];
  for (const { ts, p, v } of points) {
    const t = seconds ? Math.floor(ts / seconds) * seconds : ts, last = out.at(-1);
    if (seconds && last && last.t === t) { last.h = Math.max(last.h, p); last.l = Math.min(last.l, p); last.c = p; last.v += v; last.n++; }
    else out.push({ t, o: last ? last.c : p, h: Math.max(p, last?.c ?? p), l: Math.min(p, last?.c ?? p), c: p, v, n: 1 });
  }
  return out;
}

const WEI = 1e18;
/** wei to whole units as a JSON number. Exact to about 15 digits, which is more than a chart can draw; the wei stay on the row. */
export const units = (wei: string | bigint): number => Number(BigInt(wei)) / WEI;

/**
 * A curve trade's price. Pons's `CurveBuy(quoteIn, tokensOut, fee, tax)` charges the fee and the tax out of `quoteIn`, so the
 * curve received `quoteIn − fee − tax`; `CurveSell(tokensIn, quoteOut, fee, tax)` pays the seller `quoteOut` AFTER both, so the
 * curve released `quoteOut + fee + tax`. Measured on the staging fork: a sell's fee is 1.042% of `quoteOut`, which is 1.000% of
 * the gross, and the net prices of a buy and the sell after it are continuous while the gross ones jump.
 */
export function curvePrice(buy: boolean, quote: bigint, coins: bigint, fee: bigint, tax: bigint): number {
  if (coins === 0n) return 0;
  const net = buy ? quote - fee - tax : quote + fee + tax;
  return Number(net) / Number(coins);
}

/** A pool trade's price: the two legs of the swap, with no fee adjustment — the hook charges outside the `Swap`. */
export function poolPrice(quote: bigint, coins: bigint): number {
  const c = coins < 0n ? -coins : coins, q = quote < 0n ? -quote : quote;
  return c === 0n ? 0 : Number(q) / Number(c);
}

/** Where a chart opens: trade by trade for a coin younger than the trade span, whose whole life fits in it, else 5-minute buckets. */
export const initialRes = (launchedAt: number | null | undefined, nowS: number): Res =>
  launchedAt !== null && launchedAt !== undefined && nowS - launchedAt < YOUNG_S ? "trade" : "5m";
