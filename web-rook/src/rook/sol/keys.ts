/** Solana addresses: 32-byte keys written in base58, and the program ids the site names. No curve code here (src/rook/sol/pda.ts has
 *  it), so the landing page can check an address without loading ed25519. */
import { b58decode } from "./base58";

export type Address = string;

export function keyBytes(a: Address): Uint8Array {
  const b = b58decode(a);
  if (b.length !== 32) throw new Error(`not a 32-byte address: ${a}`);
  return b;
}
export const isAddress = (a: unknown): a is Address => {
  if (typeof a !== "string" || a.length < 32 || a.length > 44) return false;
  try { return b58decode(a).length === 32; } catch { return false; }
};

export const SYSTEM_PROGRAM = "11111111111111111111111111111111";
export const TOKEN_PROGRAM = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA";
export const ATA_PROGRAM = "ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL";
export const COMPUTE_BUDGET_PROGRAM = "ComputeBudget111111111111111111111111111111";
export const METADATA_PROGRAM = "metaqbxxUerdq28cj1RbAWkYQm3ybzjb6a8bt518x1s";
export const NATIVE_MINT = "So11111111111111111111111111111111111111112";
/** The Instructions sysvar: DBC reads it to see a swap's own transaction (trade.ts `launchIxs`). */
export const INSTRUCTIONS_SYSVAR = "Sysvar1nstructions1111111111111111111111111";

