/** Base58, base64 and the little-endian readers the Solana decoders need. Pure; no dependency. (Ported from web-rook/src/rook/sol/base58.ts + borsh.ts.) */
const ALPHABET = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
const INDEX = new Map([...ALPHABET].map((c, i) => [c, i]));

export function b58encode(bytes: Uint8Array): string {
  let zeros = 0;
  while (zeros < bytes.length && bytes[zeros] === 0) zeros++;
  let n = 0n;
  for (const b of bytes) n = (n << 8n) | BigInt(b);
  let out = "";
  while (n > 0n) { out = ALPHABET[Number(n % 58n)] + out; n /= 58n; }
  return "1".repeat(zeros) + out;
}
export function b58decode(s: string): Uint8Array {
  let zeros = 0;
  while (zeros < s.length && s[zeros] === "1") zeros++;
  let n = 0n;
  for (const c of s) {
    const v = INDEX.get(c);
    if (v === undefined) throw new Error(`not base58: "${c}"`);
    n = n * 58n + BigInt(v);
  }
  const body: number[] = [];
  while (n > 0n) { body.unshift(Number(n & 255n)); n >>= 8n; }
  return Uint8Array.from([...new Array(zeros).fill(0), ...body]);
}
export const b64decode = (s: string): Uint8Array => new Uint8Array(Buffer.from(s, "base64"));
export const b64encode = (b: Uint8Array): string => Buffer.from(b).toString("base64");

/** A base58 Solana address (32 bytes). Signatures are 64 bytes (87–88 chars). */
export const ADDRESS_RE = /^[1-9A-HJ-NP-Za-km-z]{32,44}$/;
export function isAddress(s: unknown): s is string {
  if (typeof s !== "string" || !ADDRESS_RE.test(s)) return false;
  try { return b58decode(s).length === 32; } catch { return false; }
}

export const u16At = (d: Uint8Array, at: number) => d[at] | (d[at + 1] << 8);
export const u32At = (d: Uint8Array, at: number) => (d[at] | (d[at + 1] << 8) | (d[at + 2] << 16)) + d[at + 3] * 2 ** 24;
export function uintAt(d: Uint8Array, at: number, size: number): bigint {
  if (at + size > d.length) throw new Error("read past the end");
  let n = 0n;
  for (let i = size - 1; i >= 0; i--) n = (n << 8n) | BigInt(d[at + i]);
  return n;
}
export const u64At = (d: Uint8Array, at: number) => uintAt(d, at, 8);
export const u128At = (d: Uint8Array, at: number) => uintAt(d, at, 16);
export const i64At = (d: Uint8Array, at: number) => { const n = u64At(d, at); return n >= 1n << 63n ? n - (1n << 64n) : n; };
export const pkAt = (d: Uint8Array, at: number) => { if (at + 32 > d.length) throw new Error("read past the end"); return b58encode(d.subarray(at, at + 32)); };
export const sameBytes = (a: ArrayLike<number>, b: ArrayLike<number>) => a.length === b.length && Array.prototype.every.call(a, (x: number, i: number) => x === b[i]);
export const startsWith = (d: Uint8Array, p: ArrayLike<number>) => d.length >= p.length && Array.prototype.every.call(p, (x: number, i: number) => x === d[i]);
