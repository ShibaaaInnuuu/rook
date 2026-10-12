import { bn254 } from "@noble/curves/bn254";
import { concat, hexToBytes, keccak256, numberToHex, sha256, toHex } from "viem";
import type { Fetch } from "./jsonrpc.js";
import type { Hex } from "./types.js";

/**
 * docs/v8/redemption-draw.md §2.3: drand evmnet's beacon checked with no key and no chain. The same arithmetic as
 * src/rand/DrandEvmnet.sol and src/rand/BN254HashToCurve.sol, in TypeScript: RFC 9380 hash_to_curve for BN254 G1
 * (expand_message_xmd over Keccak-256, the Shallue–van de Woestijne map), then the pairing check e(sig, G2) = e(H(m), pk) with
 * @noble/curves, and the randomness sha256(signature). Tested against the contract's own vectors (rounds 1000 and 20,823,703) and
 * rounds served by two public relays. Not constant-time: every input is public.
 */
export const EVMNET = {
  chainHash: "04f1e9062b8a81f848fded9c12306733282b2727ecced50032187751166ec8c3",
  genesis: 1727521075,
  period: 3,
  scheme: "bls-bn254-unchained-on-g1",
  /** as the relays' /info serves it: x.c1 ‖ x.c0 ‖ y.c1 ‖ y.c0 of the G2 public key (DrandEvmnet.sol PK_X1, PK_X0, PK_Y1, PK_Y0) */
  publicKey: "07e1d1d335df83fa98462005690372c643340060d205306a9aa8106b6bd0b3820557ec32c2ad488e4d4f6008f89a346f18492092ccc0d594610de2732c8b808f0095685ae3a85ba243747b1b2f426049010f6b73a0cf1d389351d5aaaa1047f6297d3a4f9749b33eb2d904c9d9ebf17224150ddd7abd7567a9bec6c74480ee0b",
  dst: "BLS_SIG_BN254G1_XMD:KECCAK-256_SVDW_RO_NUL_",
} as const;
/** EthNftWrapper.PIN_AHEAD: a burn pins the round 20 rounds (60 s) after the one current at its block's timestamp. */
export const PIN_AHEAD = 20;
/** Public relays that serve evmnet, keyless (measured 2026-09-26/27: api.drand.sh and api2.drand.sh serve historical rounds). */
export const DRAND_RELAYS = ["https://api.drand.sh", "https://api2.drand.sh", "https://drand.cloudflare.com"] as const;

export const roundAt = (ts: number): number => (ts < EVMNET.genesis ? 0 : Math.floor((ts - EVMNET.genesis) / EVMNET.period) + 1);
export const timeOf = (round: number): number => (round === 0 ? 0 : EVMNET.genesis + (round - 1) * EVMNET.period);

const P = 21888242871839275222246405745257275088696311157297823662689037894645226208583n;
const C1 = 4n, C2 = 0x183227397098d014dc2822db40c0ac2ecbc0b548b438e5469e10460b6c3e7ea3n;
const C3 = 0x16789af3a83522eb353c98fc6b36d713d5d8d1cc5dffffffan, C4 = 0x10216f7ba065e00de81ac1e7808072c9dd2b2385cd7b438469602eb24829a9bdn;
const mod = (a: bigint) => ((a % P) + P) % P;
function pow(b: bigint, e: bigint): bigint { let r = 1n; b = mod(b); while (e > 0n) { if (e & 1n) r = (r * b) % P; b = (b * b) % P; e >>= 1n; } return r; }
const g = (x: bigint) => mod(((x * x) % P) * x + 3n);
const isSquare = (a: bigint) => { const l = pow(a, (P - 1n) / 2n); return l === 0n || l === 1n; };
const big = (b: Uint8Array) => (b.length ? BigInt(toHex(b)) : 0n);

/** expand_message_xmd (RFC 9380 §5.3.1), H = Keccak-256 (b = 32, s = 136 bytes). */
export function expandMessageXmd(dst: Uint8Array, msg: Uint8Array, len: number): Uint8Array {
  if (dst.length > 255) throw new Error("dst too long");
  const ell = Math.ceil(len / 32), dstPrime = concat([dst, new Uint8Array([dst.length])]);
  const b0 = hexToBytes(keccak256(concat([new Uint8Array(136), msg, new Uint8Array([len >> 8, len & 255, 0]), dstPrime])));
  let bi = hexToBytes(keccak256(concat([b0, new Uint8Array([1]), dstPrime])));
  const out = [bi];
  for (let i = 2; i <= ell; i++) { bi = hexToBytes(keccak256(concat([b0.map((v, j) => v ^ bi[j]), new Uint8Array([i]), dstPrime]))); out.push(bi); }
  return concat(out).slice(0, len);
}
/** The SvdW map (RFC 9380 §F.1) for y² = x³ + 3 with Z = 1, as BN254HashToCurve.mapToPoint. */
export function mapToPoint(u: bigint): { x: bigint; y: bigint } {
  let tv1 = mod(((u * u) % P) * C1);
  const tv2 = mod(1n + tv1);
  tv1 = mod(1n - tv1);
  const tv3 = pow((tv1 * tv2) % P, P - 2n);
  const tv4 = mod(((((u * tv1) % P) * tv3) % P) * C3);
  const x1 = mod(C2 - tv4), x2 = mod(C2 + tv4);
  let x3 = mod(((tv2 * tv2) % P) * tv3);
  x3 = mod(((x3 * x3) % P) * C4 + 1n);
  const x = isSquare(g(x1)) ? x1 : isSquare(g(x2)) ? x2 : x3;
  let y = pow(g(x), (P + 1n) / 4n);
  if ((u & 1n) !== (y & 1n)) y = mod(-y);
  return { x, y };
}
const G1 = bn254.G1.ProjectivePoint, G2 = bn254.G2.ProjectivePoint, Fp2 = bn254.fields.Fp2;
/** The message point of round `round`: hash_to_curve(DST, keccak256(uint64 round, big-endian)). */
export function messagePoint(round: number | bigint): InstanceType<typeof G1> {
  const msg = hexToBytes(keccak256(numberToHex(BigInt(round), { size: 8 })));
  const u = expandMessageXmd(new TextEncoder().encode(EVMNET.dst), msg, 96);
  return G1.fromAffine(mapToPoint(mod(big(u.slice(0, 48))))).add(G1.fromAffine(mapToPoint(mod(big(u.slice(48, 96))))));
}
const pkHex = hexToBytes(`0x${EVMNET.publicKey}`);
const word = (i: number) => big(pkHex.slice(32 * i, 32 * i + 32));
const PK = G2.fromAffine({ x: Fp2.fromBigTuple([word(1), word(0)]), y: Fp2.fromBigTuple([word(3), word(2)]) });

export type BeaconCheck = { ok: true; randomness: Hex } | { ok: false; error: string };
/** Whether `signature` is evmnet's signature of `round`, and if so the round's randomness (sha256 of the signature). */
export function verifyBeacon(round: number | bigint, signature: string): BeaconCheck {
  const sig = signature.startsWith("0x") ? signature : `0x${signature}`;
  if (!/^0x[0-9a-fA-F]{128}$/.test(sig)) return { ok: false, error: "a signature is 64 bytes of hex" };
  const b = hexToBytes(sig as Hex), x = big(b.slice(0, 32)), y = big(b.slice(32));
  if (x >= P || y >= P || (x === 0n && y === 0n)) return { ok: false, error: "the signature is not a point of the curve" };
  if (mod(y * y) !== g(x)) return { ok: false, error: "the signature is not a point of the curve" };
  if (BigInt(round) <= 0n) return { ok: false, error: "round 0 has no beacon" };
  try {
    const S = G1.fromAffine({ x, y });
    const ok = bn254.fields.Fp12.eql(bn254.pairing(S, G2.BASE), bn254.pairing(messagePoint(round), PK));
    return ok ? { ok: true, randomness: sha256(sig as Hex) } : { ok: false, error: `not evmnet's signature of round ${round}` };
  } catch (e) { return { ok: false, error: `the pairing failed: ${String((e as Error)?.message ?? e).slice(0, 80)}` }; }
}

export type RelayAnswer = { relay: string; signature: Hex | null; error: string | null };
/** A round from public relays, each named; the caller checks the bytes itself (BLS signatures are unique per round). */
export async function fetchRound(round: number, relays: readonly string[], fetchImpl: Fetch, timeoutMs = 10_000): Promise<RelayAnswer[]> {
  return Promise.all(relays.map(async (base): Promise<RelayAnswer> => {
    const relay = (() => { try { return new URL(base).host; } catch { return base; } })();
    try {
      const url = `${base.replace(/\/$/, "")}/${EVMNET.chainHash}/public/${round}`;
      const r = await fetchImpl(url, { headers: { accept: "application/json" }, signal: AbortSignal.timeout(timeoutMs) });
      if (!r.ok) return { relay, signature: null, error: `HTTP ${r.status}` };
      const j = (await r.json()) as { round?: unknown; signature?: unknown };
      if (String(j.round) !== String(round) || typeof j.signature !== "string" || !/^[0-9a-f]{128}$/i.test(j.signature)) return { relay, signature: null, error: "an answer without this round's signature" };
      return { relay, signature: `0x${j.signature.toLowerCase()}` as Hex, error: null };
    } catch (e) { return { relay, signature: null, error: String((e as Error)?.message ?? e).slice(0, 80) }; }
  }));
}
