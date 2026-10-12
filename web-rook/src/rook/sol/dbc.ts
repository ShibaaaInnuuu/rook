/** Meteora DBC 0.2.1: the addresses (PDAs) and the instruction builders. The decoders and the curve math are dbcState.ts's, re-exported. */
import { Writer } from "./borsh";
import { ATA_PROGRAM, INSTRUCTIONS_SYSVAR, METADATA_PROGRAM, NATIVE_MINT, SYSTEM_PROGRAM, TOKEN_PROGRAM, keyBytes, type Address } from "./keys";
import { pda } from "./pda";
import { DBC_POOL_AUTHORITY, DBC_PROGRAM, DISC } from "./dbcState";
import type { Instruction } from "./tx";
export * from "./dbcState";

export const dbcEventAuthority = (): Address => pda(["__event_authority"], DBC_PROGRAM);

// ---- addresses ---------------------------------------------------------------------------------------------------------------------

const byteCmp = (a: Uint8Array, b: Uint8Array) => { for (let i = 0; i < 32; i++) if (a[i] !== b[i]) return a[i] - b[i]; return 0; };
/** DBC's pool: ["pool", config, max(base, quote), min(base, quote)] (keys compared as bytes). */
export function poolAddress(config: Address, baseMint: Address, quoteMint: Address = NATIVE_MINT): Address {
  const a = keyBytes(baseMint), b = keyBytes(quoteMint);
  const [hi, lo] = byteCmp(a, b) > 0 ? [a, b] : [b, a];
  return pda(["pool", keyBytes(config), hi, lo], DBC_PROGRAM);
}
export const tokenVault = (mint: Address, pool: Address): Address => pda(["token_vault", keyBytes(mint), keyBytes(pool)], DBC_PROGRAM);
export const metadataAddress = (mint: Address): Address => pda(["metadata", keyBytes(METADATA_PROGRAM), keyBytes(mint)], METADATA_PROGRAM);

// ---- instructions ------------------------------------------------------------------------------------------------------------------

const m = (pubkey: Address, isWritable = false, isSigner = false) => ({ pubkey, isWritable, isSigner });

/** Metaplex's limits on what DBC writes into the coin's metadata (create_metadata_account_v3): name ≤ 32 bytes, symbol ≤ 10, uri ≤ 200. */
export const META_LIMITS = { name: 32, symbol: 10, uri: 200 };
const utf8Len = (s: string) => new TextEncoder().encode(s).length;
export function metadataProblem(p: { name: string; symbol: string; uri: string }): string | null {
  if (!p.name.trim()) return "The coin needs a name.";
  if (utf8Len(p.name) > META_LIMITS.name) return `The name is at most ${META_LIMITS.name} bytes.`;
  if (!p.symbol.trim()) return "The coin needs a symbol.";
  if (utf8Len(p.symbol) > META_LIMITS.symbol) return `The symbol is at most ${META_LIMITS.symbol} bytes.`;
  if (!p.uri) return "The coin needs a metadata URL.";
  if (utf8Len(p.uri) > META_LIMITS.uri) return `The metadata URL is at most ${META_LIMITS.uri} bytes.`;
  return null;
}

/** `initialize_virtual_pool_with_spl_token`: the creator's coin on a config. The base mint is a fresh keypair that signs too. */
export function initializeVirtualPoolWithSplToken(a: { config: Address; creator: Address; payer: Address; baseMint: Address; quoteMint?: Address; name: string; symbol: string; uri: string }): Instruction {
  const quote = a.quoteMint ?? NATIVE_MINT;
  const pool = poolAddress(a.config, a.baseMint, quote);
  return {
    programId: DBC_PROGRAM,
    keys: [
      m(a.config), m(DBC_POOL_AUTHORITY), m(a.creator, false, true), m(a.baseMint, true, true), m(quote), m(pool, true),
      m(tokenVault(a.baseMint, pool), true), m(tokenVault(quote, pool), true), m(metadataAddress(a.baseMint), true), m(METADATA_PROGRAM),
      m(a.payer, true, true), m(TOKEN_PROGRAM), m(TOKEN_PROGRAM), m(SYSTEM_PROGRAM), m(dbcEventAuthority()), m(DBC_PROGRAM),
    ],
    data: new Writer().bytes(DISC.initializeVirtualPoolWithSplToken).string(a.name).string(a.symbol).string(a.uri).done(),
  };
}

/** `swap` (exact in, `minimum_amount_out`). No referral: Anchor's "none" for an optional account is the program id itself.
 *  `launch`: the swap rides in the transaction that creates its pool, so it names the Instructions sysvar as its one remaining
 *  account. On a config with `enable_first_swap_with_min_fee` (every collection's since 2026-10-11) DBC 0.2.1 then charges the pool's
 *  first swap the scheduler's minimum fee (≈ 1 %), not the anti-sniper fee: it reads the sysvar for an `initialize_virtual_pool_*` of
 *  this pool earlier in the same transaction, the swap at the top level, no referral (`process_swap.rs`
 *  `validate_contain_initialize_pool_ix_and_no_cpi`). DBC accepts the sysvar on any swap (one extra remaining account, checked to
 *  be the sysvar); elsewhere it changes nothing. */
export function swap(a: { config: Address; pool: Address; baseMint: Address; quoteMint?: Address; baseVault: Address; quoteVault: Address; user: Address; input: Address; output: Address; amountIn: bigint; minimumAmountOut: bigint; launch?: boolean }): Instruction {
  const quote = a.quoteMint ?? NATIVE_MINT;
  return {
    programId: DBC_PROGRAM,
    keys: [
      m(DBC_POOL_AUTHORITY), m(a.config), m(a.pool, true), m(a.input, true), m(a.output, true), m(a.baseVault, true), m(a.quoteVault, true),
      m(a.baseMint), m(quote), m(a.user, false, true), m(TOKEN_PROGRAM), m(TOKEN_PROGRAM), m(DBC_PROGRAM), m(dbcEventAuthority()), m(DBC_PROGRAM),
      ...(a.launch ? [m(INSTRUCTIONS_SYSVAR)] : []),
    ],
    data: new Writer().bytes(DISC.swap).u64(a.amountIn).u64(a.minimumAmountOut).done(),
  };
}

/** `swap2 { amount_0, amount_1, swap_mode }`: mode 0 exact in (`amount_0` in, `amount_1` minimum out), 1 partial fill (stops at the
 *  migration threshold), 2 exact out (`amount_0` out, `amount_1` maximum in). The same accounts as `swap`. The site's two-hop buy
 *  uses mode 0, as the canister's `route::two_hop_buy` does. */
export function swap2(a: { config: Address; pool: Address; baseMint: Address; quoteMint?: Address; baseVault: Address; quoteVault: Address; user: Address; input: Address; output: Address; amount0: bigint; amount1: bigint; swapMode: 0 | 1 | 2 }): Instruction {
  const base = swap({ ...a, amountIn: 0n, minimumAmountOut: 0n });
  return { ...base, data: new Writer().bytes(DISC.swap2).u64(a.amount0).u64(a.amount1).u8(a.swapMode).done() };
}

// the SPL / system / compute-budget instructions around a trade
export const createAtaIdempotent = (payer: Address, ataAddr: Address, owner: Address, mint: Address): Instruction =>
  ({ programId: ATA_PROGRAM, keys: [m(payer, true, true), m(ataAddr, true), m(owner), m(mint), m(SYSTEM_PROGRAM), m(TOKEN_PROGRAM)], data: Uint8Array.of(1) });
export const syncNative = (account: Address): Instruction => ({ programId: TOKEN_PROGRAM, keys: [m(account, true)], data: Uint8Array.of(17) });
export const closeAccount = (account: Address, dest: Address, owner: Address): Instruction =>
  ({ programId: TOKEN_PROGRAM, keys: [m(account, true), m(dest, true), m(owner, false, true)], data: Uint8Array.of(9) });
export const transferLamports = (from: Address, to: Address, lamports: bigint): Instruction =>
  ({ programId: SYSTEM_PROGRAM, keys: [m(from, true, true), m(to, true)], data: new Writer().u32(2).u64(lamports).done() });
export const COMPUTE_BUDGET = "ComputeBudget111111111111111111111111111111";
export const setComputeUnitLimit = (units: number): Instruction => ({ programId: COMPUTE_BUDGET, keys: [], data: new Writer().u8(2).u32(units).done() });
export const setComputeUnitPrice = (microLamports: bigint): Instruction => ({ programId: COMPUTE_BUDGET, keys: [], data: new Writer().u8(3).u64(microLamports).done() });

