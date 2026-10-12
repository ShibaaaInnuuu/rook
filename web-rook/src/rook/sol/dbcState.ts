/**
 * (The account decoders and the curve math: no curve code, so the landing page can decode a pool. The builders are in dbc.ts.)
 * Meteora's Dynamic Bonding Curve (DBC) 0.2.1, mainnet `dbcij3LWUppWqq96dh6gJWwBifmcGfLSB5D4DuSMaqN`, as the site uses it:
 * the account decoders (`PoolConfig`, `VirtualPool`: zero-copy `#[repr(C)]` after the 8-byte discriminator, the offsets
 * `rook_backing`'s own reader pins, solana/programs/rook_backing/src/dbc.rs), the instruction builders
 * (`initialize_virtual_pool_with_spl_token`, `swap`; account lists and discriminators from DBC 0.2.1's IDL,
 * solana/programs/rook_backing/idls/dynamic_bonding_curve.json, and the ceremony tool's ROOK launch,
 * ops/rehearsal/sol-devnet/tool/src/ceremony.rs `rook_launch_ixs`), and the curve's quote (DBC's own swap math over the
 * config's sqrt-price segments, with the base-fee scheduler). The quote is a preview: every trade is SIMULATED on chain before
 * the wallet is asked, and the minimum out comes from that simulation.
 */
import { sameBytes, u128At, u16At, u64At } from "./borsh";
import { type Address } from "./keys";
import { b58encode } from "./base58";

export const DBC_PROGRAM = "dbcij3LWUppWqq96dh6gJWwBifmcGfLSB5D4DuSMaqN";
export const DBC_POOL_AUTHORITY = "FhVo3mqL8PW5pH5U2CN4XE33DokiyZnUwuGpH2hmHLuM";
export const DAMM_V2_PROGRAM = "cpamdpZCGKUy5JxQXB4dcpGPiikHawvSWAd6mEn1sGG";

export const DISC = {
  initializeVirtualPoolWithSplToken: Uint8Array.from([140, 85, 215, 176, 102, 54, 104, 79]),
  swap: Uint8Array.from([248, 198, 158, 145, 225, 117, 135, 200]),
  /** `sha256("global:swap2")[..8]` */
  swap2: Uint8Array.from([65, 75, 63, 76, 235, 91, 91, 136]),
  poolConfig: Uint8Array.from([26, 108, 14, 123, 116, 230, 129, 43]),
  virtualPool: Uint8Array.from([213, 224, 5, 209, 98, 69, 119, 92]),
};
export const POOL_CONFIG_LEN = 8 + 1040;
export const VIRTUAL_POOL_LEN = 8 + 416;
/** DBC's `migration_fee_withdraw_status`: bit 0b100 is the partner's (the claimer's) share taken. Canister-only Rook: the custodian
 *  took the backing share into the coin's vault (docs/v9/canister-only-solana.md §3.3). */
export const PARTNER_MIGRATION_FEE_TAKEN = 0b100;
/** Where the pool's `config` key sits (getProgramAccounts memcmp) and its `activation_point` (the launch time, for sorting). */
export const POOL_CONFIG_OFFSET = 72;
export const POOL_ACTIVATION_OFFSET = 296;
export const FEE_DENOMINATOR = 1_000_000_000n;
export const MAX_FEE_NUMERATOR = 990_000_000n;

const pk = (d: Uint8Array, at: number) => b58encode(d.subarray(at, at + 32));

export type CurvePoint = { sqrtPrice: bigint; liquidity: bigint };
export type DbcConfig = {
  quoteMint: Address; feeClaimer: Address; leftoverReceiver: Address;
  baseFee: { cliffFeeNumerator: bigint; secondFactor: bigint; thirdFactor: bigint; firstFactor: number; baseFeeMode: number };
  dynamicFee: boolean;
  collectFeeMode: number; migrationOption: number; activationType: number; tokenDecimal: number; version: number; tokenType: number;
  partnerLockedLp: number; creatorLockedLp: number; partnerLp: number; creatorLp: number;
  fixedSupply: boolean; creatorTradingFeePercentage: number; tokenUpdateAuthority: number;
  migrationFeePercentage: number; creatorMigrationFeePercentage: number;
  migrationQuoteThreshold: bigint; migrationBaseThreshold: bigint; migrationSqrtPrice: bigint;
  preMigrationTokenSupply: bigint; postMigrationTokenSupply: bigint;
  migratedPoolFeeBps: number; poolCreationFee: bigint; sqrtStartPrice: bigint; curve: CurvePoint[];
  /** `enable_first_swap_with_min_fee` (byte 365): the pool's first swap, made in the transaction that creates the pool, pays the
   *  scheduler's minimum fee (`minBaseFeeNumerator`), not the anti-sniper fee. On for every collection's config since 2026-10-11. */
  firstSwapMinFee: boolean;
};
export type DbcPool = {
  config: Address; creator: Address; baseMint: Address; baseVault: Address; quoteVault: Address;
  baseReserve: bigint; quoteReserve: bigint; partnerQuoteFee: bigint; creatorQuoteFee: bigint;
  sqrtPrice: bigint; activationPoint: bigint; poolType: number; isMigrated: boolean; migrationProgress: number;
  finishCurveTimestamp: bigint; migrationFeeWithdrawStatus: number;
};

export function decodeConfig(d: Uint8Array): DbcConfig {
  if (d.length !== POOL_CONFIG_LEN || !sameBytes(d.subarray(0, 8), DISC.poolConfig)) throw new Error("not a DBC PoolConfig");
  const curve: CurvePoint[] = [];
  for (let i = 0; i < 20; i++) {
    const at = 408 + 32 * i;
    const p = { sqrtPrice: u128At(d, at), liquidity: u128At(d, at + 16) };
    if (p.sqrtPrice === 0n || p.liquidity === 0n) break;
    curve.push(p);
  }
  return {
    quoteMint: pk(d, 8), feeClaimer: pk(d, 40), leftoverReceiver: pk(d, 72),
    baseFee: { cliffFeeNumerator: u64At(d, 104), secondFactor: u64At(d, 112), thirdFactor: u64At(d, 120), firstFactor: u16At(d, 128), baseFeeMode: d[130] },
    dynamicFee: d[136] === 1,
    collectFeeMode: d[232], migrationOption: d[233], activationType: d[234], tokenDecimal: d[235], version: d[236], tokenType: d[237],
    partnerLockedLp: d[239], partnerLp: d[240], creatorLockedLp: d[241], creatorLp: d[242],
    fixedSupply: d[244] === 1, creatorTradingFeePercentage: d[245], tokenUpdateAuthority: d[246],
    migrationFeePercentage: d[247], creatorMigrationFeePercentage: d[248],
    migrationQuoteThreshold: u64At(d, 264), migrationBaseThreshold: u64At(d, 272), migrationSqrtPrice: u128At(d, 280),
    preMigrationTokenSupply: u64At(d, 344), postMigrationTokenSupply: u64At(d, 352),
    migratedPoolFeeBps: u16At(d, 362), poolCreationFee: u64At(d, 368), sqrtStartPrice: u128At(d, 392), curve,
    firstSwapMinFee: d[365] === 1,
  };
}

export function decodePool(d: Uint8Array): DbcPool {
  if (d.length !== VIRTUAL_POOL_LEN || !sameBytes(d.subarray(0, 8), DISC.virtualPool)) throw new Error("not a DBC VirtualPool");
  return {
    config: pk(d, 72), creator: pk(d, 104), baseMint: pk(d, 136), baseVault: pk(d, 168), quoteVault: pk(d, 200),
    baseReserve: u64At(d, 232), quoteReserve: u64At(d, 240), partnerQuoteFee: u64At(d, 272), creatorQuoteFee: u64At(d, 360),
    sqrtPrice: u128At(d, 280), activationPoint: u64At(d, 296), poolType: d[304], isMigrated: d[305] === 1, migrationProgress: d[308],
    finishCurveTimestamp: u64At(d, 344), migrationFeeWithdrawStatus: d[311],
  };
}

// ---- the curve ---------------------------------------------------------------------------------------------------------------------

const Q128 = 1n << 128n;
const divUp = (a: bigint, b: bigint) => (a + b - 1n) / b;
/** Δbase = L·(√P_hi − √P_lo) / (√P_lo·√P_hi) */
export const deltaBase = (lo: bigint, hi: bigint, l: bigint, up: boolean) => { const n = l * (hi - lo), d = lo * hi; return up ? divUp(n, d) : n / d; };
/** Δquote = L·(√P_hi − √P_lo) / 2^128 */
export const deltaQuote = (lo: bigint, hi: bigint, l: bigint, up: boolean) => { const n = l * (hi - lo); return up ? divUp(n, Q128) : n >> 128n; };
const nextFromQuote = (s: bigint, l: bigint, amount: bigint) => s + (amount << 128n) / l;
const nextFromBase = (s: bigint, l: bigint, amount: bigint) => divUp(l * s, l + amount * s);

/** Base out for quote in, along the curve from √P (no fee). null: not enough liquidity on the curve. */
export function quoteToBase(c: Pick<DbcConfig, "curve">, sqrtPrice: bigint, amountIn: bigint): { out: bigint; nextSqrtPrice: bigint } | null {
  let s = sqrtPrice, left = amountIn, out = 0n;
  for (const p of c.curve) {
    if (left === 0n) break;
    if (p.sqrtPrice <= s) continue;
    const max = deltaQuote(s, p.sqrtPrice, p.liquidity, true);
    if (left < max) { const n = nextFromQuote(s, p.liquidity, left); out += deltaBase(s, n, p.liquidity, false); s = n; left = 0n; }
    else { out += deltaBase(s, p.sqrtPrice, p.liquidity, false); s = p.sqrtPrice; left -= max; }
  }
  return left === 0n ? { out, nextSqrtPrice: s } : null;
}
/** Quote out for base in, down the curve from √P to at most the start price (no fee). null: past the start of the curve. */
export function baseToQuote(c: Pick<DbcConfig, "curve" | "sqrtStartPrice">, sqrtPrice: bigint, amountIn: bigint): { out: bigint; nextSqrtPrice: bigint } | null {
  let s = sqrtPrice, left = amountIn, out = 0n;
  for (let i = c.curve.length - 2; i >= 0 && left > 0n; i--) {
    const lo = c.curve[i].sqrtPrice, l = c.curve[i + 1].liquidity;
    if (lo >= s) continue;
    const max = deltaBase(lo, s, l, true);
    if (left < max) { const n = nextFromBase(s, l, left); out += deltaQuote(n, s, l, false); s = n; left = 0n; }
    else { out += deltaQuote(lo, s, l, false); s = lo; left -= max; }
  }
  if (left > 0n) {
    const l = c.curve[0].liquidity, n = nextFromBase(s, l, left);
    if (n < c.sqrtStartPrice) return null;
    out += deltaQuote(n, s, l, false); s = n;
  }
  return { out, nextSqrtPrice: s };
}

/** The base fee now (numerator over 1e9): DBC's fee scheduler, linear (mode 0) or exponential (mode 1), from the pool's activation. */
export function baseFeeNumerator(c: Pick<DbcConfig, "baseFee">, activationPoint: bigint, now: bigint): bigint {
  const f = c.baseFee;
  if (f.baseFeeMode > 1 || f.firstFactor === 0 || f.secondFactor === 0n) return f.cliffFeeNumerator;
  const elapsed = now > activationPoint ? now - activationPoint : 0n;
  let period = elapsed / f.secondFactor;
  if (period > BigInt(f.firstFactor)) period = BigInt(f.firstFactor);
  if (f.baseFeeMode === 0) { const r = f.cliffFeeNumerator - period * f.thirdFactor; return r > 0n ? r : 0n; }
  // exponential: cliff · (1 − reduction/10_000)^period
  let fee = f.cliffFeeNumerator;
  for (let i = 0n; i < period; i++) fee = (fee * (10_000n - f.thirdFactor)) / 10_000n;
  return fee;
}
/** The scheduler's minimum (its value after the last period): what a launch's first buy pays on a `firstSwapMinFee` config (DBC
 *  `FeeScheduler::get_min_base_fee_numerator`). Rook's: 500,000,000 − 60 × 8,166,666 = 10,000,040 (1.000004 %). */
export function minBaseFeeNumerator(c: Pick<DbcConfig, "baseFee">): bigint {
  const f = c.baseFee;
  return baseFeeNumerator(c, 0n, f.secondFactor * BigInt(f.firstFactor));
}
const feeOf = (amount: bigint, num: bigint) => divUp(amount * (num > MAX_FEE_NUMERATOR ? MAX_FEE_NUMERATOR : num), FEE_DENOMINATOR);

export type Quote = { amountOut: bigint; fee: bigint; feeNumerator: bigint; nextSqrtPrice: bigint };
/** A buy (SOL in → coin out) or a sell (coin in → SOL out), fees in SOL (collect_fee_mode 0): taken from the input on a buy, from the output on a sell.
 *  `launchBuy`: the buy rides in the pool's creating transaction (trade.ts `launchIxs`), so on a `firstSwapMinFee` config it pays the minimum fee. */
export function quoteSwap(c: DbcConfig, pool: Pick<DbcPool, "sqrtPrice" | "activationPoint">, side: "buy" | "sell", amountIn: bigint, now: bigint, launchBuy = false): Quote | null {
  if (amountIn <= 0n) return null;
  const num = launchBuy && side === "buy" && c.firstSwapMinFee ? minBaseFeeNumerator(c) : baseFeeNumerator(c, pool.activationPoint, now);
  if (side === "buy") {
    const fee = c.collectFeeMode === 0 ? feeOf(amountIn, num) : 0n;
    const r = quoteToBase(c, pool.sqrtPrice, amountIn - fee);
    return r ? { amountOut: r.out, fee, feeNumerator: num, nextSqrtPrice: r.nextSqrtPrice } : null;
  }
  const r = baseToQuote(c, pool.sqrtPrice, amountIn);
  if (!r) return null;
  const fee = feeOf(r.out, num);
  return { amountOut: r.out - fee, fee, feeNumerator: num, nextSqrtPrice: r.nextSqrtPrice };
}

/** Price of one whole coin in SOL: (√P / 2^64)² lamports per base unit, scaled by the decimals. */
export function priceSol(sqrtPrice: bigint, baseDecimals: number, quoteDecimals = 9): number {
  const s = Number(sqrtPrice) / 2 ** 64;
  return s * s * 10 ** (baseDecimals - quoteDecimals);
}
/** What the coin's page and the board print about a pool on its config. */
export function poolStats(c: DbcConfig, p: DbcPool) {
  const price = priceSol(p.sqrtPrice, c.tokenDecimal);
  const supply = c.fixedSupply ? Number(c.preMigrationTokenSupply) / 10 ** c.tokenDecimal : null;
  const threshold = c.migrationQuoteThreshold;
  const progress = threshold > 0n ? Math.min(1, Number((p.quoteReserve * 10_000n) / threshold) / 10_000) : 0;
  const phase: "curve" | "graduating" | "graduated" = p.isMigrated ? "graduated" : p.quoteReserve >= threshold ? "graduating" : "curve";
  return { priceSol: price, supply, marketCapSol: supply === null ? null : price * supply, progress, phase, raisedLamports: p.quoteReserve, thresholdLamports: threshold };
}
