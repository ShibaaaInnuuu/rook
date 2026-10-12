/**
 * The trades the site builds on a DBC curve, as instruction lists: a buy (SOL → coin: wrap SOL into the wallet's wSOL account, swap,
 * unwrap what is left), a sell (coin → SOL: swap into wSOL, unwrap), and a launch (initialize the pool under a Rook config, with an
 * optional first buy in the same transaction). Exact in, with a minimum out; `boundBySimulation` sets that minimum from what a
 * simulation of the same transaction actually returned, less the visitor's slippage.
 */
import { decodeTokenAccount } from "./accounts";
import { closeAccount, createAtaIdempotent, initializeVirtualPoolWithSplToken, poolAddress, swap, syncNative, tokenVault, transferLamports } from "./dbc";
import { NATIVE_MINT, type Address } from "./keys";
import { ata } from "./pda";
import type { Rpc } from "./rpc";
import { simulate } from "./send";
import type { Instruction } from "./tx";

/** A coin's curve. `quoteMint` is wSOL for the D-e curves and the collection's wX for the canister-only ones (src/rook/sol/route.ts
 *  routes SOL → wX → coin for those); absent, wSOL. */
export type PoolRef = { pool: Address; config: Address; baseMint: Address; baseVault: Address; quoteVault: Address; quoteMint?: Address };

/** `launch`: the buy rides in the pool's creating transaction (`launchIxs`): its swap names the Instructions sysvar (dbc.ts `swap`). */
export function buyIxs(user: Address, p: PoolRef, lamportsIn: bigint, minOut: bigint, unwrap = true, launch = false): Instruction[] {
  const wsol = ata(user, NATIVE_MINT), coin = ata(user, p.baseMint);
  return [
    createAtaIdempotent(user, wsol, user, NATIVE_MINT), transferLamports(user, wsol, lamportsIn), syncNative(wsol),
    createAtaIdempotent(user, coin, user, p.baseMint),
    swap({ ...p, user, input: wsol, output: coin, amountIn: lamportsIn, minimumAmountOut: minOut, launch }),
    ...(unwrap ? [closeAccount(wsol, user, user)] : []),
  ];
}
export function sellIxs(user: Address, p: PoolRef, coinsIn: bigint, minOut: bigint, unwrap = true): Instruction[] {
  const wsol = ata(user, NATIVE_MINT), coin = ata(user, p.baseMint);
  return [
    createAtaIdempotent(user, wsol, user, NATIVE_MINT),
    swap({ ...p, user, input: coin, output: wsol, amountIn: coinsIn, minimumAmountOut: minOut }),
    ...(unwrap ? [closeAccount(wsol, user, user)] : []),
  ];
}

/** A buy paid in the curve's own SPL quote (a collection's wX) from the wallet's wX account: no wrap, nothing to unwrap. */
export function wxBuyIxs(user: Address, p: PoolRef & { quoteMint: Address }, wxIn: bigint, minOut: bigint, launch = false): Instruction[] {
  const coin = ata(user, p.baseMint);
  return [createAtaIdempotent(user, coin, user, p.baseMint), swap({ ...p, user, input: ata(user, p.quoteMint), output: coin, amountIn: wxIn, minimumAmountOut: minOut, launch })];
}

/** The pool a launch creates, and its vaults, before it exists; `quoteMint` is the config's (the collection's wX; wSOL for ROOK's). */
export function newPoolRef(config: Address, mint: Address, quoteMint: Address = NATIVE_MINT): PoolRef {
  const pool = poolAddress(config, mint, quoteMint);
  return { pool, config, baseMint: mint, baseVault: tokenVault(mint, pool), quoteVault: tokenVault(quoteMint, pool), quoteMint };
}
/** The launch: initialize the pool under the config, with an optional first buy of `firstBuy` quote units in the same transaction:
 *  SOL wrapped and unwrapped on a SOL-quoted config, the wallet's wX on a collection's (canister-only Rook: every collection's).
 *  The first buy's swap names the Instructions sysvar, so on a collection's config (`enable_first_swap_with_min_fee`) it pays the
 *  minimum fee, ≈ 1 %, not the anti-sniper fee (the creator's buy, as pump.fun's dev buy; dbc.ts `swap`). */
export function launchIxs(a: { user: Address; config: Address; mint: Address; quoteMint?: Address; name: string; symbol: string; uri: string; firstBuy: bigint; minOut: bigint }, unwrap = true): Instruction[] {
  const quote = a.quoteMint ?? NATIVE_MINT;
  const init = initializeVirtualPoolWithSplToken({ config: a.config, creator: a.user, payer: a.user, baseMint: a.mint, quoteMint: quote, name: a.name, symbol: a.symbol, uri: a.uri });
  if (a.firstBuy <= 0n) return [init];
  const ref = newPoolRef(a.config, a.mint, quote);
  return [init, ...(quote === NATIVE_MINT ? buyIxs(a.user, ref, a.firstBuy, a.minOut, unwrap, true) : wxBuyIxs(a.user, { ...ref, quoteMint: quote }, a.firstBuy, a.minOut, true))];
}

export const withSlippage = (out: bigint, bps: number) => (out * BigInt(10_000 - Math.max(0, Math.min(5_000, Math.round(bps))))) / 10_000n;

/**
 * Simulate the trade with no minimum (and without the final unwrap, so the output account can be read after it), read what the output
 * account received and how the wallet's SOL moved, and return the minimum out at the slippage. The real transaction is simulated
 * again by the send path before the wallet sees it.
 */
export async function boundBySimulation(rpc: Rpc, user: Address, outputAccount: Address, build: (minOut: bigint, unwrap: boolean) => Instruction[], slippageBps: number) {
  const before = await rpc.accounts([user, outputAccount]);
  const preOut = before[1] ? decodeTokenAccount(before[1].data).amount : 0n;
  const preSol = BigInt(before[0]?.lamports ?? 0);
  const sim = await simulate(rpc, user, build(0n, false), [user, outputAccount]);
  const postOut = sim.accounts[1] ? decodeTokenAccount(sim.accounts[1].data).amount : 0n;
  const received = postOut - preOut;
  if (received <= 0n) throw new Error("the simulation returned nothing for this amount");
  return { received, minOut: withSlippage(received, slippageBps), solDelta: BigInt(sim.accounts[0]?.lamports ?? 0) - preSol, units: sim.units };
}
