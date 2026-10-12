/**
 * The arithmetic behind a coin's chart, apart from its drawing, so both can be tested.
 *
 * The indexer computes the candles and hands them over with the trades that made them. Nothing here interpolates: an empty
 * bucket is simply absent, and a coin that never traded has an empty series that the page must say out loud rather than
 * draw as a flat line. docs/phase9.md, section 10.
 */

/**
 * docs/phase12.md §4. A range is a WINDOW, and the indexer derives the bucket from it. There is deliberately no per-trade
 * option: on a bonding curve the step from one trade to the next is whatever the next buy cost, so a per-trade chart put a
 * `+82.3%` in front of readers that was true and answered no question they had asked.
 */
export type Range = "5m" | "1h" | "6h" | "1d" | "all";

export type Candle = { t: number; o: number; h: number; l: number; c: number; v: number; n: number };

export type Mark = {
  id: string; side: "buy" | "sell"; venue: "curve" | "pool";
  quoteWei: string; coinWei: string; feeWei: string; taxWei: string;
  price: number; trader: string; recipient: string;
  tx: string; block: number; logIndex: number; at: number;
};

export type Graduation = { at: number; block: number; tx: string } | null;

export type CandleSet = {
  res: string; range: Range | null; coin: string; quote: string; priceIs: string;
  candles: Candle[]; marks: Mark[];
  trades: number; tradesAllTime: number;
  launchedAt: number; span: number; seconds: number;
  graduation: Graduation; pool: string | null;
  /** docs/phase12.md §3: the coin's whole supply in base units, and dollars per ETH from one pool. Either may be unknown. */
  supplyWei: string | null; usdPerEth: number | null; usdPerEthAt: number | null; usdPool: string | null;
};

export const RANGES: { key: Range; label: string; covers: string }[] = [
  { key: "5m", label: "5M", covers: "the last five minutes" },
  { key: "1h", label: "1H", covers: "the last hour" },
  { key: "6h", label: "6H", covers: "the last six hours" },
  { key: "1d", label: "1D", covers: "the last day" },
  { key: "all", label: "ALL", covers: "this coin's whole life" },
];

/** The finest range a coin's own age can actually fill: for something launched ten minutes ago, a day is one bar of nothing. */
export function initialRange(launchedAt: number | null, now = Math.floor(Date.now() / 1000)): Range {
  if (!launchedAt) return "1d";
  const age = now - launchedAt;
  if (age < 3_600) return "1h";
  if (age < 21_600) return "6h";
  if (age < 86_400) return "1d";
  return "all";
}

// ---- docs/phase12.md §3: market cap, which is the thing the chart plots --------------------------------------------------

/** Which unit the y axis is in. `usd` is only offered when the indexer actually read a rate. */
export type Unit = "usd" | "pair";

/** A supply in base units as whole coins. Null when the indexer has not read it, because then there is no market cap at all. */
export function wholeSupply(supplyWei: string | null | undefined): number | null {
  if (!supplyWei) return null;
  const n = Number(supplyWei) / 1e18;
  return Number.isFinite(n) && n > 0 ? n : null;
}

/**
 * Market cap: whole supply × price × the rate. Null whenever a number it needs is missing — an unread supply, a chain with
 * no dollar pool, a stale rate — and never a substitute. A caller that gets null falls back to the pair asset and says so.
 */
export function capOf(set: Pick<CandleSet, "supplyWei" | "usdPerEth">, close: number | null | undefined, unit: Unit): number | null {
  const supply = wholeSupply(set.supplyWei);
  if (supply === null || close === null || close === undefined || !Number.isFinite(close) || close <= 0) return null;
  if (unit === "pair") return supply * close;
  // Number.isFinite, not a null check: a recorded demo taken before this phase has no such field at all, and a missing
  // number must read the same as an unknown one rather than multiplying through to NaN.
  return Number.isFinite(set.usdPerEth as number) ? supply * close * (set.usdPerEth as number) : null;
}

/** Whether a dollar axis can be offered at all. When it cannot, the toggle does not appear rather than appearing disabled. */
export const hasUsd = (set: Pick<CandleSet, "supplyWei" | "usdPerEth"> | undefined | null): boolean =>
  !!set && Number.isFinite(set.usdPerEth as number) && wholeSupply(set.supplyWei) !== null;

/** A market cap, in the words a market cap is read in. Big ones are abbreviated; small ones keep their digits. */
export function cap(v: number | null, unit: Unit, quote = "ETH"): string {
  if (v === null || !Number.isFinite(v)) return "not known";
  const abbr = (n: number) => (n >= 1e12 ? `${(n / 1e12).toFixed(2)}T` : n >= 1e9 ? `${(n / 1e9).toFixed(2)}B` : n >= 1e6 ? `${(n / 1e6).toFixed(2)}M` : null);
  if (unit === "usd") {
    const a = abbr(v);
    if (a) return `$${a}`;
    if (v >= 1_000) return `$${Math.round(v).toLocaleString("en-US")}`;
    if (v >= 1) return `$${v.toFixed(2)}`;
    return `$${v.toPrecision(2)}`;
  }
  const a = abbr(v);
  if (a) return `${a} ${quote}`;
  if (v >= 1) return `${v.toFixed(2)} ${quote}`;
  if (v >= 0.0001) return `${v.toPrecision(3)} ${quote}`;
  return `${price(v)} ${quote}`;
}

/** The moment under the crosshair, said the way a chart says it. Short ranges need the second; a month does not. */
export function clock(t: number, bucketS: number): string {
  const d = new Date(t * 1000);
  const time: Intl.DateTimeFormatOptions = bucketS <= 15
    ? { hour: "2-digit", minute: "2-digit", second: "2-digit" }
    : { hour: "2-digit", minute: "2-digit" };
  const date: Intl.DateTimeFormatOptions = bucketS >= 900 ? { day: "numeric", month: "short" } : {};
  return `${d.toLocaleString("en-GB", { ...date, ...time, timeZone: "UTC" })} UTC`;
}

/** The window the candles occupy, and the prices they reach, with a little air above and below. */
export function bounds(candles: Candle[]): { t0: number; t1: number; lo: number; hi: number } | null {
  if (!candles.length) return null;
  let lo = Infinity, hi = -Infinity;
  for (const k of candles) {
    if (k.l < lo) lo = k.l;
    if (k.h > hi) hi = k.h;
  }
  if (!Number.isFinite(lo) || !Number.isFinite(hi)) return null;
  if (hi === lo) { const pad = hi === 0 ? 1 : Math.abs(hi) * 0.08; lo -= pad; hi += pad; }
  else { const pad = (hi - lo) * 0.08; lo -= pad; hi += pad; }
  return { t0: candles[0].t, t1: candles[candles.length - 1].t, lo: Math.max(0, lo), hi };
}

/**
 * What the price did **across the whole range shown**, as a fraction: the first bucket's open against the last bucket's close.
 *
 * Every range is a window of time, so this is now always a change over a stretch of the clock. It used to be reachable with
 * one candle per trade, where it meant the step from one buy to the next — the `+82.3%` docs/phase12.md §1 removed. A single
 * candle still cannot say anything, and says null rather than pretending.
 */
export function change(candles: Candle[]): number | null {
  if (candles.length < 2) return null;
  const first = candles[0].o;
  const last = candles[candles.length - 1].c;
  if (!first) return null;
  return (last - first) / first;
}

/** The volume in the window, in the pair asset. */
export const volume = (candles: Candle[]): number => candles.reduce((a, k) => a + k.v, 0);

/**
 * A price here is the pair asset for one whole coin, and on a bonding curve that is a very small number. Four significant
 * figures is what a trader reads; below a ten-thousandth the exponent is printed rather than a row of zeros nobody counts.
 */
export function price(p: number | null | undefined): string {
  if (p === null || p === undefined || !Number.isFinite(p) || p <= 0) return "—".replace("—", "not known");
  if (p >= 1000) return p.toLocaleString("en-US", { maximumFractionDigits: 2 });
  if (p >= 0.01) return p.toPrecision(4);
  if (p >= 0.0001) return p.toFixed(8).replace(/0+$/, "");
  const [m, e] = p.toExponential(3).split("e");
  return `${m}e${e.replace("+", "")}`;
}

/** A change, signed, as a percentage, in words a number can be read from. */
export function pct(x: number | null): string {
  if (x === null || !Number.isFinite(x)) return "not known";
  const s = (x * 100).toFixed(x >= 0.1 || x <= -0.1 ? 1 : 2);
  return `${x > 0 ? "+" : ""}${s}%`;
}

/** The trade nearest a moment, for the crosshair. */
export function nearestMark(marks: Mark[], t: number): Mark | null {
  if (!marks.length) return null;
  let best = marks[0];
  for (const m of marks) if (Math.abs(m.at - t) < Math.abs(best.at - t)) best = m;
  return best;
}

/**
 * Where graduation sits on the line: between the last curve trade and the first pool trade.
 *
 * It is a fact the indexer records with its own timestamp, so it is placed by that rather than guessed from the venue
 * change. Null when the coin is still on the curve, which is most of them.
 */
export function graduationAt(set: Pick<CandleSet, "graduation" | "candles">): number | null {
  const g = set.graduation;
  if (!g || !set.candles.length) return null;
  const t0 = set.candles[0].t;
  const t1 = set.candles[set.candles.length - 1].t;
  return g.at >= t0 && g.at <= t1 + 1 ? g.at : null;
}

/** Ten thousand million raw units is not a size anybody reads. Whole coins, abbreviated. */
export function coins(rawWei: string, digits = 2): string {
  const n = Number(rawWei) / 1e18;
  if (!Number.isFinite(n)) return "not known";
  if (n >= 1e9) return `${(n / 1e9).toFixed(digits)}B`;
  if (n >= 1e6) return `${(n / 1e6).toFixed(digits)}M`;
  if (n >= 1e3) return `${(n / 1e3).toFixed(digits)}k`;
  return n.toFixed(digits);
}
