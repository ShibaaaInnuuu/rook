/**
 * docs/v7/cycles-from-fees.md §3: when the fueler acts, how much, and the price checks. Pure: no I/O, so every cap is tested.
 *
 * Units: ETH and ckETH in wei, ICP in e8s, cycles in whole cycles (1 T = 1e12). The caps are held in two units on purpose: in T
 * (what the canisters need, per run and per 7 days) and in ETH (what may leave the ops wallet, per day and per 7 days).
 */
export const T = 1_000_000_000_000n;
export const WEI = 1_000_000_000_000_000_000n;
export const DAY_S = 86_400;

export type FuelCaps = {
  buyerLowT: number; buyerTargetT: number; timelockLowT: number; timelockTargetT: number;
  runMaxT: number; weekMaxT: number; dayMaxWei: bigint; weekMaxWei: bigint;
  floorWei: bigint; bufferWei: bigint; minRunWei: bigint; minGapS: number;
};

/** The defaults: the dormant model's launch (34 T at the handover, one collection awake ≈ 2.5 T a day), with the small caps agreed
 *  for the first month (20 T a run, 60 T a week); the ETH caps are the design's (§3.2). Every one is an environment override. */
export const DEFAULT_CAPS: FuelCaps = {
  buyerLowT: 40, buyerTargetT: 60, timelockLowT: 1.5, timelockTargetT: 2,
  runMaxT: 20, weekMaxT: 60, dayMaxWei: WEI / 10n, weekMaxWei: WEI / 4n,
  floorWei: WEI / 50n, bufferWei: (WEI * 15n) / 100n, minRunWei: WEI / 200n, minGapS: 6 * 3_600,
};

export const LAMPORTS = 1_000_000_000n;
/** The Solana home's caps (docs/v9/fueler-solana.md §3): the same T caps; the SOL ones at the 2026-10-10 prices (SOL ≈ $110, ETH ≈ $2,495),
 *  sized for the first month: a day's conversion at most 1 SOL (≈ 80 T at the oracle), 2.5 SOL in 7 days; never below 0.01 SOL (rent and
 *  fees); 0.05 SOL kept for fees; no run under 0.1 SOL (Relay's and the Ethereum deposit's fixed costs). What the wallet holds above the
 *  buffer plus a day's cap is forwarded to the cold payee. Every one is an environment override (FUELER_SOL_*). */
export const DEFAULT_SOL_CAPS: FuelCaps = {
  ...DEFAULT_CAPS, dayMaxWei: LAMPORTS, weekMaxWei: (LAMPORTS * 5n) / 2n, floorWei: LAMPORTS / 100n, bufferWei: LAMPORTS / 20n, minRunWei: LAMPORTS / 10n,
};

/** One conversion the fueler made, as the journal keeps it (the caps read these). */
export type Conversion = { at: number; wei: bigint; cycles: bigint };

export type Prices = { ethUsd: number; icpUsd: number; at: number; sources: string[]; solUsd?: number };

/** The asset the run spends: ETH (the Robinhood home, in wei) or SOL (the Solana home, in lamports). Every "wei" below is in this
 *  asset's smallest unit: wei on robinhood, lamports on solana (docs/v9/fueler-solana.md). */
export type Asset = "ETH" | "SOL";
const usdOf = (p: Prices, asset: Asset): number => {
  const v = asset === "SOL" ? p.solUsd : p.ethUsd;
  if (typeof v !== "number" || !(v > 0)) throw new Error(`no ${asset}/USD price`);
  return v;
};
const scaleOf = (asset: Asset) => (asset === "SOL" ? 1e9 : 1e18);

/** §3.3: two sources per pair, agreeing within `tolBps`; the reference is their mean. Missing or disagreeing → null, with why. */
export type Quote3 = { eth?: number; icp?: number; sol?: number };
/** `withSol` (the Solana home): SOL/USD must answer and agree too. */
export function agree(cg: Quote3 | null, cb: Quote3 | null, at: number, tolBps = 200, withSol = false):
  { prices: Prices | null; why: string | null } {
  const ok = (x: unknown): x is number => typeof x === "number" && Number.isFinite(x) && x > 0;
  if (!cg || !cb || !ok(cg.eth) || !ok(cg.icp) || !ok(cb.eth) || !ok(cb.icp)) return { prices: null, why: "a price source did not answer for ETH and ICP" };
  if (withSol && (!ok(cg.sol) || !ok(cb.sol))) return { prices: null, why: "a price source did not answer for SOL" };
  const pairs: [string, number, number][] = [["ETH", cg.eth, cb.eth], ["ICP", cg.icp, cb.icp]];
  if (withSol) pairs.push(["SOL", cg.sol!, cb.sol!]);
  for (const [k, a, b] of pairs) {
    const diff = Math.abs(a - b) / Math.min(a, b);
    if (diff * 10_000 > tolBps) return { prices: null, why: `${k}/USD disagrees: CoinGecko ${a}, Coinbase ${b} (${(diff * 100).toFixed(2)} % > ${tolBps / 100} %)` };
  }
  return { prices: { ethUsd: (cg.eth + cb.eth) / 2, icpUsd: (cg.icp + cb.icp) / 2, at, sources: ["coingecko", "coinbase"],
    ...(withSol ? { solUsd: (cg.sol! + cb.sol!) / 2 } : {}) }, why: null };
}

/** Cycles per ICP from the CMC's own rate (10,000ths of an XDR per ICP; 1 T cycles = 1 XDR). */
export const cyclesPerIcp = (xdrPermyriadPerIcp: bigint): bigint => (xdrPermyriadPerIcp * T) / 10_000n;

/** At the oracle and the CMC: how many wei of ETH buy `cycles`, before any loss. */
export function weiForCycles(cycles: bigint, p: Prices, xdrPermyriadPerIcp: bigint, asset: Asset = "ETH"): bigint {
  const icp = Number(cycles) / Number(cyclesPerIcp(xdrPermyriadPerIcp));
  return BigInt(Math.ceil(((icp * p.icpUsd) / usdOf(p, asset)) * scaleOf(asset)));
}

/** At the oracle and the CMC: how many cycles `wei` of ETH become, before any loss. */
export function cyclesForWei(wei: bigint, p: Prices, xdrPermyriadPerIcp: bigint, asset: Asset = "ETH"): bigint {
  const icp = ((Number(wei) / scaleOf(asset)) * usdOf(p, asset)) / p.icpUsd;
  return BigInt(Math.floor(icp * Number(cyclesPerIcp(xdrPermyriadPerIcp))));
}

const sum = (xs: bigint[]) => xs.reduce((a, b) => a + b, 0n);
const since = (cs: Conversion[], nowS: number, s: number) => cs.filter((c) => c.at > nowS - s);

export type Room = { weekT: bigint; dayWei: bigint; weekWei: bigint };
/** What the caps still allow, from the conversions already made. */
export function room(cs: Conversion[], nowS: number, caps: FuelCaps): Room {
  const pos = (x: bigint) => (x > 0n ? x : 0n);
  return {
    weekT: pos(BigInt(Math.round(caps.weekMaxT * 1e6)) * (T / 1_000_000n) - sum(since(cs, nowS, 7 * DAY_S).map((c) => c.cycles))),
    dayWei: pos(caps.dayMaxWei - sum(since(cs, nowS, DAY_S).map((c) => c.wei))),
    weekWei: pos(caps.weekMaxWei - sum(since(cs, nowS, 7 * DAY_S).map((c) => c.wei))),
  };
}

export type Target = { canister: string; cycles: bigint };
export type Decision =
  | { kind: "idle"; why: string }
  | { kind: "wait"; why: string }
  | { kind: "run"; targets: Target[]; wei: bigint; expectCycles: bigint }
  | { kind: "blocked"; alert: "cycles-autofund-cap" | "cycles-autofund-short"; why: string };

const tOf = (x: number) => BigInt(Math.round(x * 1e6)) * (T / 1_000_000n);
const fmtT = (c: bigint) => `${(Number(c) / 1e12).toFixed(2)} T`;
const fmtA = (w: bigint, asset: Asset = "ETH") => `${(Number(w) / scaleOf(asset)).toFixed(5)} ${asset}`;

/**
 * §3.1–§3.2: whether a run starts now, for whom, and with how much ETH. `lossBps` is the allowance for the route (Relay, the pool,
 * fees): the ETH taken is what the cycles cost at the oracle, grossed up by it, so the canisters get at least what was planned.
 */
export function decide(a: { nowS: number; lastRunAt: number | null; buyerCycles: bigint; timelockCycles: bigint | null; opsWei: bigint;
  gasReserveWei: bigint; conversions: Conversion[]; prices: Prices; xdrPermyriadPerIcp: bigint; caps: FuelCaps; buyer: string; timelock: string | null;
  lossBps?: number; asset?: Asset }): Decision {
  const c = a.caps, lossBps = BigInt(a.lossBps ?? 500), asset = a.asset ?? "ETH";
  const fmtE = (w: bigint) => fmtA(w, asset);
  const want: Target[] = [];
  if (a.buyerCycles < tOf(c.buyerLowT)) want.push({ canister: a.buyer, cycles: tOf(c.buyerTargetT) - a.buyerCycles });
  if (a.timelock && a.timelockCycles !== null && a.timelockCycles < tOf(c.timelockLowT)) want.push({ canister: a.timelock, cycles: tOf(c.timelockTargetT) - a.timelockCycles });
  if (!want.length) return { kind: "idle", why: `the buyer holds ${fmtT(a.buyerCycles)} (acts under ${c.buyerLowT} T)` };
  if (a.lastRunAt !== null && a.nowS - a.lastRunAt < c.minGapS) return { kind: "wait", why: `the last run was ${Math.round((a.nowS - a.lastRunAt) / 60)} min ago (at most one in ${c.minGapS / 3600} h)` };

  // the T caps: per run, then the 7 days; the timelock first (it is small and it guards the buyer's upgrades)
  want.sort((x, y) => (x.canister === a.timelock ? -1 : y.canister === a.timelock ? 1 : 0));
  const r = room(a.conversions, a.nowS, c);
  let budget = tOf(c.runMaxT) < r.weekT ? tOf(c.runMaxT) : r.weekT;
  if (budget <= 0n) return { kind: "blocked", alert: "cycles-autofund-cap", why: `the 7-day cap of ${c.weekMaxT} T is spent: a human decides whether the burn is real` };
  const targets: Target[] = [];
  for (const w of want) { const take = w.cycles < budget ? w.cycles : budget; if (take > 0n) { targets.push({ canister: w.canister, cycles: take }); budget -= take; } }

  // the ETH it takes, then the ETH caps and the wallet's floor
  let cycles = sum(targets.map((t) => t.cycles));
  let wei = (weiForCycles(cycles, a.prices, a.xdrPermyriadPerIcp, asset) * 10_000n) / (10_000n - lossBps);
  const spendable = a.opsWei - c.floorWei - a.gasReserveWei;
  const ethCap = [r.dayWei, r.weekWei].reduce((m, x) => (x < m ? x : m));
  const limit = ethCap < spendable ? ethCap : spendable;
  if (wei > limit) {
    if (limit < c.minRunWei) {
      return ethCap < spendable
        ? { kind: "blocked", alert: "cycles-autofund-cap", why: `the ${asset} caps leave ${fmtE(ethCap)} (day ${fmtE(r.dayWei)}, 7 days ${fmtE(r.weekWei)}), under a run's minimum of ${fmtE(c.minRunWei)}` }
        : { kind: "blocked", alert: "cycles-autofund-short", why: `the ops wallet holds ${fmtE(a.opsWei)}: ${fmtE(spendable > 0n ? spendable : 0n)} above its floor and gas, under a run's minimum of ${fmtE(c.minRunWei)}` };
    }
    // scale every target down by the same factor
    const f = (limit * 1_000_000n) / wei;
    for (const t of targets) t.cycles = (t.cycles * f) / 1_000_000n;
    cycles = sum(targets.map((t) => t.cycles));
    wei = limit;
  }
  if (wei < c.minRunWei) wei = c.minRunWei <= limit ? c.minRunWei : wei;
  return { kind: "run", targets, wei, expectCycles: cycles };
}

/** §3.3, the pool leg: the quote must be within `tolBps` of the oracle's ICP for that ckETH, and `amountOutMinimum` is the quote less
 *  `slipBps`. Throws the refusal. */
export function judgeSwap(ckWei: bigint, quoteE8s: bigint, p: Prices, tolBps = 150, slipBps = 50): { minOut: bigint; oracleE8s: bigint } {
  const oracleE8s = BigInt(Math.floor(((Number(ckWei) / 1e18) * p.ethUsd / p.icpUsd) * 1e8));
  if (quoteE8s * 10_000n < oracleE8s * BigInt(10_000 - tolBps))
    throw new Error(`the pool quotes ${quoteE8s} e8s for ${ckWei} wei of ckETH, more than ${tolBps / 100} % under the oracle's ${oracleE8s}: refused (thin pool or a moved price)`);
  return { minOut: (quoteE8s * BigInt(10_000 - slipBps)) / 10_000n, oracleE8s };
}

/** §3.3, end to end: the cycles the run will mint must be at least `minBps` of what its ETH is worth at the oracle and the CMC. */
export function judgeEndToEnd(wei: bigint, icpE8s: bigint, p: Prices, xdrPermyriadPerIcp: bigint, minBps = 9_500, asset: Asset = "ETH"): bigint {
  const got = (icpE8s * cyclesPerIcp(xdrPermyriadPerIcp)) / 100_000_000n;
  const fair = cyclesForWei(wei, p, xdrPermyriadPerIcp, asset);
  if (got * 10_000n < fair * BigInt(minBps))
    throw new Error(`the run would mint ${fmtT(got)} for ${fmtA(wei, asset)}, under ${minBps / 100} % of the ${fmtT(fair)} it is worth: refused`);
  return got;
}

/** Split `icpE8s` across the targets in proportion to their cycles, each net of the ledger fee; the last takes the remainder. */
export function split(icpE8s: bigint, targets: Target[], feeE8s: bigint): { canister: string; e8s: bigint }[] {
  const total = sum(targets.map((t) => t.cycles));
  if (total === 0n || icpE8s <= feeE8s * BigInt(targets.length)) return [];
  const net = icpE8s - feeE8s * BigInt(targets.length);
  let left = net;
  return targets.map((t, i) => {
    const e8s = i === targets.length - 1 ? left : (net * t.cycles) / total;
    left -= e8s;
    return { canister: t.canister, e8s };
  }).filter((x) => x.e8s > 0n);
}
