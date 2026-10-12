/**
 * docs/v5/ux-and-reserves.md §1, "The one number". A trade is a route of hops (legs); each leg knows what went in, what came out,
 * its pre-trade mid price, and every fee and spread it charged. Aggregated, a route gives the visitor one figure:
 *
 *     totalCost = 1 − amountOut / (amountIn × Π midᵢ)
 *
 * which contains every fee, every spread and the price impact, because the mids are fee-free and pre-trade. The fees and spreads
 * are also listed one by one (for Details), and the impact is what is left over. Pure: no chain, no React; the curve's arithmetic is
 * FrogCurve's own, integer for integer, and a pool hop is either Uniswap's own quote (live) or a constant-product estimate (fixture).
 */
export const BPS = 10_000;
export const PIPS = 1_000_000;
const ONE = 10n ** 18n;

export type Token = "ETH" | "WX" | "COIN";
export type Venue = "curve" | "sharedPool" | "coinPool" | "floorMint" | "floorBurn";
export type Fee = { label: string; bps: number };
export type Leg = {
  venue: Venue; tokenIn: Token; tokenOut: Token; amountIn: bigint; amountOut: bigint;
  /** pre-trade, fee-free price: tokenOut per tokenIn, in whole units */
  mid: number;
  fees: Fee[];
  /** a spread charged on top of the mid (the Floor Module's), bps of this leg's input */
  spreadBps: number;
  /** what is called, for Details */
  contract: string;
  /** "estimate" when the amount is our arithmetic on a fixture, not the venue's own answer */
  estimate?: boolean;
};
export type Route = { id: "curve" | "pool" | "floorModule" | "direct"; legs: Leg[]; executable: boolean; why?: string;
  /** a curve buy that reaches the raise uses only part of what was sent: the rest comes back in the same transaction */
  change?: bigint };
export type FeeLine = { label: string; bps: number; eth: bigint };
export type Quote = {
  side: "buy" | "sell"; route: Route; alternatives: Route[];
  amountIn: bigint; amountOut: bigint; minOut: bigint;
  /** the one number, bps of what goes in, valued at the pre-trade mids */
  totalCostBps: number;
  feesBps: number; spreadBps: number; impactBps: number;
  feeLines: FeeLine[];
  /** the ETH value of what goes in (sell: the coins at the mid) — what the percentages are of */
  valueInEth: bigint;
};

// ---- the venues --------------------------------------------------------------------------------------------------------------

/** FrogCurve's state, as `v()`, `t()`, `vEnd()`, `tEnd()`, `feeBps()` and `launchExtraBps()` read it. `feeBps` is the creator's
 *  fee tier (docs/v5/policies-and-fees.md §3); `launchExtraBps` the decaying launch fee a buy pays right now (§2), 0 once decayed. */
export type CurveState = { v: bigint; t: bigint; vEnd: bigint; tEnd: bigint; feeBps: number; launchExtraBps?: number };
const ceilDiv = (a: bigint, b: bigint) => (a + b - 1n) / b;
const tier = (bps: number) => `${(bps / 100).toFixed(bps % 100 ? 1 : 0)}% tier`;

/** FrogCurve.buy, exactly: the fee (the tier plus what is left of the launch fee) on what is sent; the last buy is filled to the
 *  raise at that moment's total rate and the rest returned. */
export function curveBuy(c: CurveState, ethIn: bigint): Leg & { change: bigint } {
  const extra = c.launchExtraBps ?? 0, rate = c.feeBps + extra;
  let gross = ethIn, fee = (gross * BigInt(rate)) / BigInt(BPS), net = gross - fee, out: bigint;
  const room = c.vEnd - c.v;
  if (net >= room) {
    net = room;
    gross = ceilDiv(net * BigInt(BPS), BigInt(BPS - rate));
    out = c.t - c.tEnd;
  } else out = c.t - ceilDiv(c.v * c.t, c.v + net);
  const fees: Fee[] = [{ label: `Curve fee (${tier(c.feeBps)})`, bps: c.feeBps }];
  if (extra > 0) fees.push({ label: "Launch fee, decaying to nothing (all of it to the coin's vault)", bps: extra });
  return { venue: "curve", tokenIn: "ETH", tokenOut: "COIN", amountIn: gross, amountOut: out, mid: Number(c.t) / Number(c.v),
    fees, spreadBps: 0, contract: "FrogCurve.buy(minOut)", change: ethIn - gross };
}
/** FrogCurve.sell, exactly: the fee comes off the ETH out. */
export function curveSell(c: CurveState, coinIn: bigint): Leg {
  const gross = c.v - ceilDiv(c.v * c.t, c.t + coinIn), fee = (gross * BigInt(c.feeBps)) / BigInt(BPS);
  return { venue: "curve", tokenIn: "COIN", tokenOut: "ETH", amountIn: coinIn, amountOut: gross - fee, mid: Number(c.v) / Number(c.t),
    fees: [{ label: `Curve fee (${tier(c.feeBps)})`, bps: c.feeBps }], spreadBps: 0, contract: "FrogCurve.sell(amount, minEth)" };
}

/**
 * Uniswap v4's fee on an exact-input swap: the protocol fee is taken first and the LP fee on the rest, which v4 folds into one
 * `swapFee = pf + lp − pf·lp/1e6` (pips). Both are listed separately so Details can say who gets what.
 */
export const v4Fees = (lpFeePips: number, protocolFeePips: number, what: string): Fee[] =>
  [{ label: `${what}: liquidity providers' fee`, bps: (lpFeePips * (1 - protocolFeePips / PIPS)) / 100 }, { label: `${what}: Uniswap protocol fee`, bps: protocolFeePips / 100 }];

/** A pool hop estimated as constant product over reserves (fixture mode only; live mode asks V4Quoter). */
export function poolLeg(p: { reserveIn: bigint; reserveOut: bigint; lpFeePips: number; protocolFeePips: number }, amountIn: bigint,
  venue: "sharedPool" | "coinPool", tokenIn: Token, tokenOut: Token, what: string): Leg {
  const swapFee = BigInt(Math.round(p.protocolFeePips + p.lpFeePips - (p.protocolFeePips * p.lpFeePips) / PIPS));
  const after = (amountIn * (BigInt(PIPS) - swapFee)) / BigInt(PIPS);
  const out = (p.reserveOut * after) / (p.reserveIn + after);
  return { venue, tokenIn, tokenOut, amountIn, amountOut: out, mid: Number(p.reserveOut) / Number(p.reserveIn), fees: v4Fees(p.lpFeePips, p.protocolFeePips, what),
    spreadBps: 0, contract: venue === "sharedPool" ? "the collection's Uniswap v4 pool, through EtchRouter" : "the coin's Uniswap v4 pool, through EtchRouter", estimate: true };
}

// ---- aggregation -------------------------------------------------------------------------------------------------------------

const toN = (x: bigint) => Number(x) / 1e18;
/** A route's end-to-end numbers. `midRef` replaces a leg's mid for the reference price (see `floorModule.ts`): all candidate
 *  routes of one trade are measured against the same market price, so their costs can be compared. */
export function aggregate(side: "buy" | "sell", route: Route, slippageBps: number): Quote {
  const legs = route.legs, amountIn = legs[0].amountIn, amountOut = legs[legs.length - 1].amountOut;
  const mids = legs.map((l) => l.mid), prod = mids.reduce((a, b) => a * b, 1);
  const ideal = toN(amountIn) * prod;
  const total = ideal > 0 ? 1 - toN(amountOut) / ideal : 0;
  const feeFrac = 1 - legs.reduce((a, l) => a * (1 - l.fees.reduce((s, f) => s + f.bps, 0) / BPS), 1);
  const spreadFrac = 1 - legs.reduce((a, l) => a * (1 - l.spreadBps / BPS), 1);
  // ETH per unit of each leg's input token, at the mids: from the ETH end of the route
  const ethPer = legs.map((_, i) => side === "buy" ? 1 / mids.slice(0, i).reduce((a, b) => a * b, 1) : mids.slice(i).reduce((a, b) => a * b, 1));
  const eth = (units: number) => BigInt(Math.max(0, Math.round(units * 1e18)));
  const feeLines: FeeLine[] = [];
  legs.forEach((l, i) => {
    for (const f of l.fees) feeLines.push({ label: f.label, bps: f.bps, eth: eth(toN(l.amountIn) * ethPer[i] * (f.bps / BPS)) });
    if (l.spreadBps) feeLines.push({ label: "Floor Module spread", bps: l.spreadBps, eth: eth(toN(l.amountIn) * ethPer[i] * (l.spreadBps / BPS)) });
  });
  const valueInEth = side === "buy" ? amountIn : eth(toN(amountIn) * prod);
  return { side, route, alternatives: [], amountIn, amountOut, minOut: (amountOut * BigInt(BPS - slippageBps)) / BigInt(BPS),
    totalCostBps: total * BPS, feesBps: feeFrac * BPS, spreadBps: spreadFrac * BPS, impactBps: (total - feeFrac - spreadFrac) * BPS, feeLines, valueInEth };
}

/** The candidate that gives the visitor the most, among those that can actually be executed; ties go to the first listed. */
export function chooseRoute(candidates: Route[]): { chosen: Route | null; others: Route[] } {
  let chosen: Route | null = null;
  for (const r of candidates) if (r.executable && r.legs.length && (!chosen || r.legs[r.legs.length - 1].amountOut > chosen.legs[chosen.legs.length - 1].amountOut)) chosen = r;
  return { chosen, others: candidates.filter((r) => r !== chosen) };
}

// ---- a coin, whatever phase it is in ------------------------------------------------------------------------------------------

export type Phase = "trading" | "graduating" | "graduated" | "refunding";
/** Everything a quote can be made from. Each leg function is supplied by the caller: fixture arithmetic or live reads. */
export type Venues = {
  phase: Phase;
  curve?: CurveState | null;
  /** exact input on the collection's shared pool: ETH → wX or wX → ETH */
  shared?: ((amountIn: bigint, dir: "ethToWx" | "wxToEth") => Promise<Leg | null>) | null;
  /** exact input on the coin's own pool: wX → COIN or COIN → wX (or ETH ↔ COIN for a SWEEP_ONLY coin) */
  coinPool?: ((amountIn: bigint, dir: "toCoin" | "fromCoin") => Promise<Leg | null>) | null;
  sweepOnly?: boolean;
  /** the Floor Module's hop, from the adapter (floorModule.ts); null when there is none. Live reads may answer asynchronously. */
  floor?: { mint: (ethIn: bigint) => Promise<Leg | null> | Leg | null; burn: (unitsIn: bigint) => Promise<Leg | null> | Leg | null; routable: boolean; why: string } | null;
};
export type QuoteResult = { ok: true; quote: Quote } | { ok: false; why: string };

const rebase = (l: Leg, mid: number): Leg => ({ ...l, mid });

export async function quoteCoin(side: "buy" | "sell", amountIn: bigint, v: Venues, slippageBps: number): Promise<QuoteResult> {
  if (amountIn <= 0n) return { ok: false, why: "Enter an amount." };
  if (v.phase === "graduating") return { ok: false, why: "Trading is paused while the raise buys its items. It opens on Uniswap when the coin graduates." };
  if (v.phase === "refunding") return { ok: false, why: "This coin did not graduate in time. Holders can refund instead of trading." };
  if (v.phase === "trading") {
    if (!v.curve) return { ok: false, why: "The curve could not be read." };
    const leg = side === "buy" ? curveBuy(v.curve, amountIn) : curveSell(v.curve, amountIn);
    if (leg.amountOut <= 0n) return { ok: false, why: "That amount is too small to trade." };
    const route: Route = { id: "curve", legs: [leg], executable: true, change: "change" in leg ? (leg as { change: bigint }).change : undefined };
    return { ok: true, quote: aggregate(side, route, slippageBps) };
  }
  // graduated: through the pools, EtchRouter
  if (!v.coinPool) return { ok: false, why: "The coin's pool could not be read." };
  if (v.sweepOnly) {
    const leg = await v.coinPool(amountIn, side === "buy" ? "toCoin" : "fromCoin");
    if (!leg || leg.amountOut <= 0n) return { ok: false, why: "The pool cannot fill that amount." };
    return { ok: true, quote: aggregate(side, { id: "direct", legs: [leg], executable: true }, slippageBps) };
  }
  const candidates: Route[] = [];
  let poolMid: number | null = null;
  if (side === "buy") {
    const a = v.shared ? await v.shared(amountIn, "ethToWx") : null;
    if (a) { poolMid = a.mid; const b = await v.coinPool(a.amountOut, "toCoin"); if (b) candidates.push({ id: "pool", legs: [a, b], executable: true }); }
    const m = v.floor ? await v.floor.mint(amountIn) : null;
    if (m) { const b = await v.coinPool(m.amountOut, "toCoin"); if (b) candidates.push({ id: "floorModule", legs: [poolMid ? rebase(m, poolMid) : m, b], executable: v.floor!.routable, why: v.floor!.routable ? undefined : v.floor!.why }); }
  } else {
    const a = await v.coinPool(amountIn, "fromCoin");
    if (a) {
      const b = v.shared ? await v.shared(a.amountOut, "wxToEth") : null;
      if (b) { poolMid = b.mid; candidates.push({ id: "pool", legs: [a, b], executable: true }); }
      const m = v.floor ? await v.floor.burn(a.amountOut) : null;
      if (m) candidates.push({ id: "floorModule", legs: [a, poolMid ? rebase(m, poolMid) : m], executable: v.floor!.routable, why: v.floor!.routable ? undefined : v.floor!.why });
    }
  }
  const { chosen, others } = chooseRoute(candidates);
  if (!chosen || chosen.legs[chosen.legs.length - 1].amountOut <= 0n) return { ok: false, why: "No route can fill that amount right now." };
  const quote = aggregate(side, chosen, slippageBps);
  quote.alternatives = others;
  return { ok: true, quote };
}

/** "1.21%", "0.05%", "−0.30%": what the ticket prints. */
export const pct = (bps: number, digits = 2) => `${bps < 0 ? "−" : ""}${(Math.abs(bps) / 100).toFixed(digits)}%`;
export { ONE };
