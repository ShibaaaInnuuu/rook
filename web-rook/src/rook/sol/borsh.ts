/** The few Borsh / little-endian encoders and readers the site's instructions and account decoders need. */
export class Writer {
  private parts: number[] = [];
  bytes(b: ArrayLike<number>) { for (let i = 0; i < b.length; i++) this.parts.push(b[i]); return this; }
  u8(n: number) { this.parts.push(n & 0xff); return this; }
  u16(n: number) { return this.uint(BigInt(n), 2); }
  u32(n: number) { return this.uint(BigInt(n), 4); }
  u64(n: bigint) { if (n < 0n || n >= 1n << 64n) throw new Error("u64 out of range"); return this.uint(n, 8); }
  string(s: string) { const b = new TextEncoder().encode(s); this.u32(b.length); return this.bytes(b); }
  private uint(n: bigint, size: number) { for (let i = 0; i < size; i++) { this.parts.push(Number(n & 0xffn)); n >>= 8n; } return this; }
  done(): Uint8Array { return Uint8Array.from(this.parts); }
}

export const u8At = (d: Uint8Array, at: number) => d[at];
export const u16At = (d: Uint8Array, at: number) => d[at] | (d[at + 1] << 8);
export const u32At = (d: Uint8Array, at: number) => (d[at] | (d[at + 1] << 8) | (d[at + 2] << 16)) + d[at + 3] * 2 ** 24;
export function uintAt(d: Uint8Array, at: number, size: number): bigint {
  let n = 0n;
  for (let i = size - 1; i >= 0; i--) n = (n << 8n) | BigInt(d[at + i]);
  return n;
}
export const u64At = (d: Uint8Array, at: number) => uintAt(d, at, 8);
export const u128At = (d: Uint8Array, at: number) => uintAt(d, at, 16);
export const i64At = (d: Uint8Array, at: number) => { const n = u64At(d, at); return n >= 1n << 63n ? n - (1n << 64n) : n; };
export const i32At = (d: Uint8Array, at: number) => { const n = u32At(d, at); return n >= 2 ** 31 ? n - 2 ** 32 : n; };
export const sameBytes = (a: ArrayLike<number>, b: ArrayLike<number>) => a.length === b.length && Array.prototype.every.call(a, (x: number, i: number) => x === b[i]);
