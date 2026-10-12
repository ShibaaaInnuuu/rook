/**
 * Program-derived addresses (PDAs) as the runtime derives them (sha256(seeds ‖ bump ‖ program ‖ "ProgramDerivedAddress"), the first
 * bump from 255 down whose hash is OFF the ed25519 curve); associated token accounts; and a local keypair for the one key the site
 * generates itself (a new coin's mint).
 */
import { ed25519 } from "@noble/curves/ed25519";
import { sha256 } from "@noble/hashes/sha2";
import { b58encode } from "./base58";
import { ATA_PROGRAM, TOKEN_PROGRAM, keyBytes, type Address } from "./keys";

export function isOnCurve(b: Uint8Array): boolean {
  try { ed25519.ExtendedPoint.fromHex(b); return true; } catch { return false; }
}

const PDA_MARKER = new TextEncoder().encode("ProgramDerivedAddress");
const concat = (parts: Uint8Array[]) => {
  const out = new Uint8Array(parts.reduce((n, p) => n + p.length, 0));
  let o = 0;
  for (const p of parts) { out.set(p, o); o += p.length; }
  return out;
};

export function createProgramAddress(seeds: Uint8Array[], program: Address): Address | null {
  for (const s of seeds) if (s.length > 32) throw new Error("a seed is at most 32 bytes");
  const h = sha256(concat([...seeds, keyBytes(program), PDA_MARKER]));
  return isOnCurve(h) ? null : b58encode(h);
}

const cache = new Map<string, [Address, number]>();
export function findProgramAddress(seeds: (Uint8Array | string)[], program: Address): [Address, number] {
  const raw = seeds.map((s) => (typeof s === "string" ? new TextEncoder().encode(s) : s));
  const key = `${program}:${raw.map((s) => b58encode(s)).join(",")}`;
  const hit = cache.get(key);
  if (hit) return hit;
  for (let bump = 255; bump >= 0; bump--) {
    const a = createProgramAddress([...raw, Uint8Array.of(bump)], program);
    if (a) { cache.set(key, [a, bump]); return [a, bump]; }
  }
  throw new Error("no program address found");
}
export const pda = (seeds: (Uint8Array | string)[], program: Address): Address => findProgramAddress(seeds, program)[0];

export const ata = (owner: Address, mint: Address, tokenProgram: Address = TOKEN_PROGRAM): Address =>
  pda([keyBytes(owner), keyBytes(tokenProgram), keyBytes(mint)], ATA_PROGRAM);

/** A keypair the page holds for one transaction (a new coin's mint). The secret never leaves this object. */
export type LocalKeypair = { address: Address; sign: (message: Uint8Array) => Uint8Array };
export function localKeypair(secret: Uint8Array = ed25519.utils.randomPrivateKey()): LocalKeypair {
  const pub = ed25519.getPublicKey(secret);
  return { address: b58encode(pub), sign: (m) => ed25519.sign(m, secret) };
}
