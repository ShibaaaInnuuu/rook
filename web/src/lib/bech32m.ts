/** BIP-350 bech32m, enough to check a mainnet taproot (P2TR) address before a wallet is asked to sign anything.
 *  The contract checks the same things (src/lib/BtcAddress.sol); this is so a typo is caught before gas is spent. */
const CHARSET = "qpzry9x8gf2tvdw0s3jn54khce6mua7l";
const BECH32M = 0x2bc830a3;

function polymod(values: number[]): number {
  const GEN = [0x3b6a57b2, 0x26508e6d, 0x1ea119fa, 0x3d4233dd, 0x2a1462b3];
  let chk = 1;
  for (const v of values) {
    const top = chk >>> 25;
    chk = ((chk & 0x1ffffff) << 5) ^ v;
    for (let i = 0; i < 5; i++) if ((top >>> i) & 1) chk ^= GEN[i];
  }
  return chk >>> 0;
}
const hrpExpand = (hrp: string) => [...hrp].map((c) => c.charCodeAt(0) >>> 5).concat([0], [...hrp].map((c) => c.charCodeAt(0) & 31));

export type TaprootCheck = { ok: true } | { ok: false; why: string };

export function checkTaproot(address: string): TaprootCheck {
  const a = address.trim();
  if (a !== a.toLowerCase() && a !== a.toUpperCase()) return { ok: false, why: "mixed upper and lower case" };
  const s = a.toLowerCase();
  if (!s.startsWith("bc1p")) return { ok: false, why: "a taproot address starts with bc1p" };
  if (s.length !== 62) return { ok: false, why: `a taproot address is 62 characters; this is ${s.length}` };
  const data: number[] = [];
  for (const c of s.slice(3)) {
    const v = CHARSET.indexOf(c);
    if (v < 0) return { ok: false, why: `"${c}" is not a bech32 character` };
    data.push(v);
  }
  if (polymod(hrpExpand("bc").concat(data)) !== BECH32M) return { ok: false, why: "the checksum does not match: one or more characters are wrong" };
  if (data[0] !== 1) return { ok: false, why: "not witness version 1" };
  // 5-bit groups back to bytes: a taproot program is exactly 32 bytes with zero padding
  let acc = 0, bits = 0; const out: number[] = [];
  for (const v of data.slice(1, -6)) { acc = (acc << 5) | v; bits += 5; while (bits >= 8) { bits -= 8; out.push((acc >>> bits) & 0xff); } }
  if (out.length !== 32 || bits >= 5 || ((acc << (8 - bits)) & 0xff) !== 0) return { ok: false, why: "the program is not 32 bytes" };
  return { ok: true };
}
