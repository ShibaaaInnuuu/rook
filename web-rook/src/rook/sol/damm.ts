/**
 * Meteora DAMM v2 (cp_amm 0.2.4, mainnet `cpamd…`): the COIN/wX pool a graduated curve migrates into (LP locked for good), as the
 * site's second hop after graduation. A port of the canister's `solix2::damm` and `solix2::decode::DammPool` (autobuyer/canister-rs),
 * checked byte for byte against instructions that landed in LiteSVM on the mainnet program bytes (`tests/fixtures/solix2/flow.json`);
 * test/twohop.test.ts holds this port against the same bytes. Here: the addresses, `swap` (exact in), the `Pool` decoder, and the
 * pool's own swap math (constant product in √price, the fee on the input for a buy on a fees-in-quote pool) as a preview; every trade
 * is simulated on chain before the wallet is asked.
 */
import { b58encode } from "./base58";
import { Writer, sameBytes, u128At, u16At, u64At } from "./borsh";
import { deltaBase, deltaQuote, DISC as DBC_DISC, FEE_DENOMINATOR } from "./dbcState";
import { TOKEN_PROGRAM, keyBytes, type Address } from "./keys";
import { pda } from "./pda";
import type { Instruction } from "./tx";

export const DAMM_PROGRAM = "cpamdpZCGKUy5JxQXB4dcpGPiikHawvSWAd6mEn1sGG";
export const DAMM_POOL_AUTHORITY = "HLnpSz9h2S4hiLQ43rnSD9XkcUThA7B8hQMKmDaiTLcC";
/** The DAMM v2 config DBC migrates into with `migration_fee_option` 6 (Customizable): the migrated pool is ["pool", config, max, min]. */
export const DAMM_CONFIG_CUSTOMIZABLE = "A8gMrEPJkacWkcb3DGwtJwTe16HktSEfvwtuDh2MCtck";
/** `sha256("account:Pool")[..8]`; `swap`'s discriminator is `sha256("global:swap")[..8]`, DBC's too (Anchor hashes the name alone). */
export const DAMM_DISC = { swap: DBC_DISC.swap, pool: Uint8Array.from([241, 154, 109, 4, 17, 177, 109, 188]) };
export const DAMM_POOL_LEN = 1_112;
/** `collect_fee_mode` 1: fees in token b only (the quote: wX on a migrated COIN/wX pool). */
export const COLLECT_FEE_ONLY_B = 1;

const m = (pubkey: Address, isWritable = false, isSigner = false) => ({ pubkey, isWritable, isSigner });
const byteCmp = (a: Uint8Array, b: Uint8Array) => { for (let i = 0; i < 32; i++) if (a[i] !== b[i]) return a[i] - b[i]; return 0; };
const ordered = (a: Address, b: Address) => { const x = keyBytes(a), y = keyBytes(b); return byteCmp(x, y) > 0 ? [x, y] : [y, x]; };

export const dammEventAuthority = (): Address => pda(["__event_authority"], DAMM_PROGRAM);
/** `["pool", config, max(a, b), min(a, b)]`: a pool on a config. */
export function configPoolAddress(config: Address, a: Address, b: Address): Address { const [hi, lo] = ordered(a, b); return pda(["pool", keyBytes(config), hi, lo], DAMM_PROGRAM); }
/** `["cpool", max, min]`: the one customizable pool of a pair. */
export function customizablePoolAddress(a: Address, b: Address): Address { const [hi, lo] = ordered(a, b); return pda(["cpool", hi, lo], DAMM_PROGRAM); }
export const dammVaultAddress = (mint: Address, pool: Address): Address => pda(["token_vault", keyBytes(mint), keyBytes(pool)], DAMM_PROGRAM);

/** A pool's addresses: token a and b (in the pool's order), their vaults. */
export type DammPoolRef = { pool: Address; a: Address; b: Address; vaultA: Address; vaultB: Address };
export const dammPoolAt = (pool: Address, a: Address, b: Address): DammPoolRef => ({ pool, a, b, vaultA: dammVaultAddress(a, pool), vaultB: dammVaultAddress(b, pool) });
/** The COIN/wX pool DBC migrates a curve into (token a = the coin, token b = the quote). */
export const migratedPool = (coin: Address, quote: Address): DammPoolRef => dammPoolAt(configPoolAddress(DAMM_CONFIG_CUSTOMIZABLE, coin, quote), coin, quote);

/** `swap { amount_in, minimum_amount_out }` (exact in): direction follows `input` / `output`. No referral: the program id. */
export function dammSwap(p: DammPoolRef, payer: Address, input: Address, output: Address, amountIn: bigint, minimumAmountOut: bigint): Instruction {
  return {
    programId: DAMM_PROGRAM,
    keys: [
      m(DAMM_POOL_AUTHORITY), m(p.pool, true), m(input, true), m(output, true), m(p.vaultA, true), m(p.vaultB, true), m(p.a), m(p.b), m(payer, false, true),
      m(TOKEN_PROGRAM), m(TOKEN_PROGRAM), m(DAMM_PROGRAM), m(dammEventAuthority()), m(DAMM_PROGRAM),
    ],
    data: new Writer().bytes(DAMM_DISC.swap).u64(amountIn).u64(minimumAmountOut).done(),
  };
}

// ---- the Pool account ---------------------------------------------------------------------------------------------------------------

const pk = (d: Uint8Array, at: number) => b58encode(d.subarray(at, at + 32));
/** DAMM v2 0.2.4 `Pool` (1,112 bytes), zero-copy `#[repr(C)]` after the discriminator: the fields the site reads. */
export type DammPool = {
  /** The base fee's cliff numerator (of 1e9); a migrated Rook pool's is 1 % (10,000,000). */
  feeNumerator: bigint; protocolFeePercent: number; compoundingFeeBps: number; dynamicFeeInitialized: number;
  tokenAMint: Address; tokenBMint: Address; tokenAVault: Address; tokenBVault: Address;
  liquidity: bigint; sqrtMinPrice: bigint; sqrtMaxPrice: bigint; sqrtPrice: bigint;
  activationPoint: bigint; activationType: number; poolStatus: number; collectFeeMode: number; poolType: number;
  permanentLockLiquidity: bigint; creator: Address; tokenAAmount: bigint; tokenBAmount: bigint;
};
export function decodeDammPool(d: Uint8Array): DammPool {
  if (d.length !== DAMM_POOL_LEN || !sameBytes(d.subarray(0, 8), DAMM_DISC.pool)) throw new Error("not a DAMM v2 Pool");
  return {
    feeNumerator: u64At(d, 8), protocolFeePercent: d[48], compoundingFeeBps: u16At(d, 54), dynamicFeeInitialized: d[56],
    tokenAMint: pk(d, 168), tokenBMint: pk(d, 200), tokenAVault: pk(d, 232), tokenBVault: pk(d, 264),
    liquidity: u128At(d, 360), sqrtMinPrice: u128At(d, 424), sqrtMaxPrice: u128At(d, 440), sqrtPrice: u128At(d, 456),
    activationPoint: u64At(d, 472), activationType: d[480], poolStatus: d[481], collectFeeMode: d[484], poolType: d[485],
    permanentLockLiquidity: u128At(d, 552), creator: pk(d, 648), tokenAAmount: u64At(d, 680), tokenBAmount: u64At(d, 688),
  };
}
/** The price of token a in token b's base units per a base unit, as a float (display only). */
export const dammPriceRaw = (p: Pick<DammPool, "sqrtPrice">): number => { const r = Number(p.sqrtPrice) / 2 ** 64; return r * r; };

// ---- the pool's swap math ------------------------------------------------------------------------------------------------------------

const divUp = (a: bigint, b: bigint) => (a + b - 1n) / b;
const feeOn = (amount: bigint, num: bigint) => divUp(amount * num, FEE_DENOMINATOR);
export type DammQuote = { amountOut: bigint; fee: bigint; feeNumerator: bigint; nextSqrtPrice: bigint };
/**
 * cp_amm's `get_swap_result` (exact in) with a fixed base fee and no dynamic fee (a migrated Rook pool has neither a scheduler nor one):
 * a buy is b → a (the quote in, the coin out) and moves √P up by amount_in · 2^128 / L; a sell is a → b and moves it down. The fee:
 * on the INPUT when the pool collects fees in b only and the input is b (a buy on a migrated pool); on the OUTPUT otherwise. null when
 * the price would leave the pool's range or the amount is not positive. A preview; the chain's simulation decides.
 */
export function quoteDammSwap(p: DammPool, side: "buy" | "sell", amountIn: bigint): DammQuote | null {
  if (amountIn <= 0n || p.liquidity === 0n) return null;
  const num = p.feeNumerator;
  if (side === "buy") {
    const feeOnInput = p.collectFeeMode === COLLECT_FEE_ONLY_B;
    const fee = feeOnInput ? feeOn(amountIn, num) : 0n;
    const next = p.sqrtPrice + ((amountIn - fee) << 128n) / p.liquidity;
    if (next > p.sqrtMaxPrice) return null;
    const out = deltaBase(p.sqrtPrice, next, p.liquidity, false);
    const outFee = feeOnInput ? 0n : feeOn(out, num);
    return { amountOut: out - outFee, fee: fee + outFee, feeNumerator: num, nextSqrtPrice: next };
  }
  const next = divUp(p.liquidity * p.sqrtPrice, p.liquidity + amountIn * p.sqrtPrice);
  if (next < p.sqrtMinPrice) return null;
  const out = deltaQuote(next, p.sqrtPrice, p.liquidity, false), fee = feeOn(out, num);
  return { amountOut: out - fee, fee, feeNumerator: num, nextSqrtPrice: next };
}
