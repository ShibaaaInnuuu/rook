/**
 * Meteora DBC 0.2.1 (`dbcij3LW…`) and DAMM v2 (`cpamdpZC…`) as the indexer reads them: account decoders (offsets as
 * web-rook/src/rook/sol/dbcState.ts and solana/programs/rook_backing/src/dbc.rs pin them), and the Anchor self-CPI events
 * (`emit_cpi!`: an inner instruction to the program itself whose data is EVENT_IX_TAG ‖ event discriminator ‖ Borsh body).
 * Layouts from solana/programs/rook_backing/idls/dynamic_bonding_curve.json (0.2.1) and cp_amm.json (0.2.4).
 */
import { b58encode, pkAt, sameBytes, startsWith, u128At, u16At, u32At, u64At } from "./bytes.js";

export const DBC_PROGRAM = "dbcij3LWUppWqq96dh6gJWwBifmcGfLSB5D4DuSMaqN";
export const DAMM_V2_PROGRAM = "cpamdpZCGKUy5JxQXB4dcpGPiikHawvSWAd6mEn1sGG";
export const TOKEN_PROGRAM = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA";
export const TOKEN_2022_PROGRAM = "TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb";
export const METADATA_PROGRAM = "metaqbxxUerdq28cj1RbAWkYQm3ybzjb6a8bt518x1s";
export const NATIVE_MINT = "So11111111111111111111111111111111111111112";

/** Anchor's event-CPI tag: sha256("anchor:event")[..8], as stored (little-endian u64 0x1d9acb512ea545e4). */
export const EVENT_IX_TAG = Uint8Array.from([0xe4, 0x45, 0xa5, 0x2e, 0x51, 0xcb, 0x9a, 0x1d]);

export const DISC = {
  // DBC instructions
  swap: [248, 198, 158, 145, 225, 117, 135, 200],
  swap2: [65, 75, 63, 76, 235, 91, 91, 136],
  initSpl: [140, 85, 215, 176, 102, 54, 104, 79],
  init2022: [169, 118, 51, 78, 145, 110, 220, 155],
  migrationDammV2: [156, 169, 230, 103, 53, 228, 80, 64],
  migrateMeteoraDamm: [27, 1, 48, 22, 180, 63, 118, 217],
  // DBC events
  evtSwap: [27, 60, 21, 213, 138, 170, 187, 147],
  evtSwap2: [189, 66, 51, 168, 38, 80, 117, 153],
  evtCurveComplete: [229, 231, 86, 84, 156, 134, 75, 24],
  evtInitializePool: [228, 50, 246, 85, 203, 66, 134, 37],
  // accounts
  poolConfig: [26, 108, 14, 123, 116, 230, 129, 43],
  virtualPool: [213, 224, 5, 209, 98, 69, 119, 92],
  dammPool: [241, 154, 109, 4, 17, 177, 109, 188],
} as const;

export const POOL_CONFIG_LEN = 8 + 1040;
export const VIRTUAL_POOL_LEN = 8 + 416;
export const POOL_CONFIG_OFFSET = 72;

// ---- accounts --------------------------------------------------------------------------------------------------------------------

export type DbcConfig = { quoteMint: string; collectFeeMode: number; migrationOption: number; activationType: number; tokenDecimal: number;
  tokenType: number; migrationQuoteThreshold: bigint; preMigrationTokenSupply: bigint; postMigrationTokenSupply: bigint; fixedSupply: boolean };
export function decodeConfig(d: Uint8Array): DbcConfig {
  if (d.length !== POOL_CONFIG_LEN || !startsWith(d, DISC.poolConfig)) throw new Error("not a DBC PoolConfig");
  return { quoteMint: pkAt(d, 8), collectFeeMode: d[232], migrationOption: d[233], activationType: d[234], tokenDecimal: d[235], tokenType: d[237],
    migrationQuoteThreshold: u64At(d, 264), preMigrationTokenSupply: u64At(d, 344), postMigrationTokenSupply: u64At(d, 352), fixedSupply: d[244] === 1 };
}

export type DbcPool = { config: string; creator: string; baseMint: string; baseVault: string; quoteVault: string; baseReserve: bigint; quoteReserve: bigint;
  sqrtPrice: bigint; activationPoint: bigint; poolType: number; isMigrated: boolean; finishCurveTimestamp: bigint };
export function decodePool(d: Uint8Array): DbcPool {
  if (d.length !== VIRTUAL_POOL_LEN || !startsWith(d, DISC.virtualPool)) throw new Error("not a DBC VirtualPool");
  return { config: pkAt(d, 72), creator: pkAt(d, 104), baseMint: pkAt(d, 136), baseVault: pkAt(d, 168), quoteVault: pkAt(d, 200),
    baseReserve: u64At(d, 232), quoteReserve: u64At(d, 240), sqrtPrice: u128At(d, 280), activationPoint: u64At(d, 296), poolType: d[304],
    isMigrated: d[305] === 1, finishCurveTimestamp: u64At(d, 344) };
}

/** DAMM v2 `Pool` (bytemuck): 8-byte discriminator, PoolFeesStruct (160 bytes), then token A/B mints and vaults. */
export const DAMM_POOL_MINTS_AT = 168;
export type DammPool = { tokenAMint: string; tokenBMint: string; tokenAVault: string; tokenBVault: string };
export function decodeDammPool(d: Uint8Array): DammPool {
  if (!startsWith(d, DISC.dammPool) || d.length < DAMM_POOL_MINTS_AT + 128) throw new Error("not a DAMM v2 Pool");
  const at = DAMM_POOL_MINTS_AT;
  return { tokenAMint: pkAt(d, at), tokenBMint: pkAt(d, at + 32), tokenAVault: pkAt(d, at + 64), tokenBVault: pkAt(d, at + 96) };
}

export type TokenMeta = { mint: string; name: string; symbol: string; uri: string };
/** Metaplex `Metadata` (key 4): key, update authority, mint, then name/symbol/uri as Borsh strings (NUL-padded by older versions). */
export function decodeMetadata(d: Uint8Array): TokenMeta {
  if (d[0] !== 4) throw new Error("not a Metaplex metadata account");
  let at = 65;
  const str = () => {
    const n = u32At(d, at); at += 4;
    if (n > 1024 || at + n > d.length) throw new Error("bad metadata string");
    const s = new TextDecoder().decode(d.subarray(at, at + n)); at += n;
    return s.replace(/\0+$/g, "").trim();
  };
  const mint = pkAt(d, 33), name = str(), symbol = str(), uri = str();
  return { mint, name, symbol, uri };
}
/** SPL / Token-2022 mint: supply @36, decimals @44. */
export function decodeMint(d: Uint8Array): { supply: bigint; decimals: number } {
  if (d.length < 82) throw new Error("not a mint");
  return { supply: u64At(d, 36), decimals: d[44] };
}

// ---- events ----------------------------------------------------------------------------------------------------------------------

/** DBC trade_direction: 0 = BaseToQuote (sell), 1 = QuoteToBase (buy). DAMM v2: 0 = AtoB, 1 = BtoA. */
export type DbcSwapEvent = { kind: "dbc-swap"; version: 1 | 2; pool: string; config: string; direction: number; hasReferral: boolean;
  swapMode: number | null; inputAmount: bigint; inputExcludedFee: bigint | null; outputAmount: bigint; amountLeft: bigint; nextSqrtPrice: bigint;
  tradingFee: bigint; protocolFee: bigint; referralFee: bigint; quoteReserve: bigint | null; migrationThreshold: bigint | null; timestamp: bigint };
export type DammSwapEvent = { kind: "damm-swap"; pool: string; direction: number; collectFeeMode: number; hasReferral: boolean; swapMode: number;
  inputAmount: bigint; inputExcludedFee: bigint; outputAmount: bigint; amountLeft: bigint; nextSqrtPrice: bigint; claimingFee: bigint; protocolFee: bigint; compoundingFee: bigint; referralFee: bigint;
  timestamp: bigint; reserveA: bigint; reserveB: bigint };
export type InitPoolEvent = { kind: "dbc-init"; pool: string; config: string; creator: string; baseMint: string; poolType: number; activationPoint: bigint };
export type CurveCompleteEvent = { kind: "dbc-complete"; pool: string; config: string; baseReserve: bigint; quoteReserve: bigint };
export type ProgramEvent = DbcSwapEvent | DammSwapEvent | InitPoolEvent | CurveCompleteEvent;

/** Decode one inner-instruction's data as an Anchor event of `program`, or null when it is not one we read. */
export function decodeEvent(program: string, data: Uint8Array): ProgramEvent | null {
  if (!startsWith(data, EVENT_IX_TAG) || data.length < 16) return null;
  const disc = data.subarray(8, 16), b = data.subarray(16);
  try {
    if (program === DBC_PROGRAM) {
      if (sameBytes(disc, DISC.evtSwap)) {
        // pool, config, dir u8, has_referral, params{amount_in, min_out}, result{actual_input, output, next_sqrt u128, trading, protocol, referral}, amount_in, ts
        const r = 66 + 16;
        return { kind: "dbc-swap", version: 1, pool: pkAt(b, 0), config: pkAt(b, 32), direction: b[64], hasReferral: b[65] === 1, swapMode: null,
          inputAmount: u64At(b, r), inputExcludedFee: null, outputAmount: u64At(b, r + 8), amountLeft: 0n, nextSqrtPrice: u128At(b, r + 16),
          tradingFee: u64At(b, r + 32), protocolFee: u64At(b, r + 40), referralFee: u64At(b, r + 48), quoteReserve: null, migrationThreshold: null, timestamp: u64At(b, r + 64) };
      }
      if (sameBytes(disc, DISC.evtSwap2)) {
        // pool, config, dir, has_referral, params2{amount_0, amount_1, swap_mode u8}, result2{incl_fee_in, excl_fee_in, left, out, next_sqrt, trading, protocol, referral}, quote_reserve, threshold, ts
        const r = 66 + 17;
        return { kind: "dbc-swap", version: 2, pool: pkAt(b, 0), config: pkAt(b, 32), direction: b[64], hasReferral: b[65] === 1, swapMode: b[66 + 16],
          inputAmount: u64At(b, r), inputExcludedFee: u64At(b, r + 8), outputAmount: u64At(b, r + 24), amountLeft: u64At(b, r + 16), nextSqrtPrice: u128At(b, r + 32),
          tradingFee: u64At(b, r + 48), protocolFee: u64At(b, r + 56), referralFee: u64At(b, r + 64),
          quoteReserve: u64At(b, r + 72), migrationThreshold: u64At(b, r + 80), timestamp: u64At(b, r + 88) };
      }
      if (sameBytes(disc, DISC.evtInitializePool)) return { kind: "dbc-init", pool: pkAt(b, 0), config: pkAt(b, 32), creator: pkAt(b, 64), baseMint: pkAt(b, 96), poolType: b[128], activationPoint: u64At(b, 129) };
      if (sameBytes(disc, DISC.evtCurveComplete)) return { kind: "dbc-complete", pool: pkAt(b, 0), config: pkAt(b, 32), baseReserve: u64At(b, 64), quoteReserve: u64At(b, 72) };
      return null;
    }
    if (program === DAMM_V2_PROGRAM && sameBytes(disc, DISC.evtSwap2)) {
      // pool, dir u8, collect_fee_mode u8, has_referral, params2(17), result2{incl, excl, left, out, next_sqrt u128, claiming, protocol, compounding, referral}, 3 transfer-fee u64s, ts, reserve_a, reserve_b
      const r = 35 + 17;
      return { kind: "damm-swap", pool: pkAt(b, 0), direction: b[32], collectFeeMode: b[33], hasReferral: b[34] === 1, swapMode: b[35 + 16],
        inputAmount: u64At(b, r), inputExcludedFee: u64At(b, r + 8), outputAmount: u64At(b, r + 24), amountLeft: u64At(b, r + 16), nextSqrtPrice: u128At(b, r + 32),
        claimingFee: u64At(b, r + 48), protocolFee: u64At(b, r + 56), compoundingFee: u64At(b, r + 64), referralFee: u64At(b, r + 72),
        timestamp: u64At(b, r + 104), reserveA: u64At(b, r + 112), reserveB: u64At(b, r + 120) };
    }
  } catch { return null; }
  return null;
}

/** Price of one whole coin in SOL from a DBC sqrt price: (√P / 2^64)² lamports per base unit, scaled by the decimals. */
export function priceFromSqrt(sqrtPrice: bigint, baseDecimals: number, quoteDecimals = 9): number {
  const s = Number(sqrtPrice) / 2 ** 64;
  return s * s * 10 ** (baseDecimals - quoteDecimals);
}
export const u16 = u16At;
export const keyOf = (b: Uint8Array) => b58encode(b);
