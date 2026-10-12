/**
 * Legacy Solana transactions, compiled and serialized by hand (the wire format of solana-sdk's `Message` / `Transaction`):
 * accounts ordered writable signers (fee payer first), read-only signers, writable non-signers, read-only non-signers; the header
 * counts; short-vec lengths. Pure, so the tests hold it against messages the ceremony tool (Rust, solana-message) compiled.
 */
import { b58decode, b58encode } from "./base58";
import { keyBytes, type Address } from "./keys";

export type AccountMeta = { pubkey: Address; isSigner: boolean; isWritable: boolean };
export type Instruction = { programId: Address; keys: AccountMeta[]; data: Uint8Array };
export type Message = {
  header: { numRequiredSignatures: number; numReadonlySigned: number; numReadonlyUnsigned: number };
  accountKeys: Address[];
  recentBlockhash: string;
  instructions: { programIdIndex: number; accounts: number[]; data: Uint8Array }[];
};

export const MAX_TX_BYTES = 1232;

export function shortvec(n: number): number[] {
  const out: number[] = [];
  let v = n;
  for (;;) {
    let b = v & 0x7f;
    v >>= 7;
    if (v) { b |= 0x80; out.push(b); } else { out.push(b); return out; }
  }
}
function readShortvec(b: Uint8Array, at: number): [number, number] {
  let n = 0, shift = 0, i = at;
  for (;;) { const x = b[i++]; n |= (x & 0x7f) << shift; if (!(x & 0x80)) return [n, i]; shift += 7; }
}

export function compileMessage(feePayer: Address, ixs: Instruction[], recentBlockhash: string): Message {
  const metas = new Map<Address, { s: boolean; w: boolean }>();
  const add = (k: Address, s: boolean, w: boolean) => {
    const m = metas.get(k);
    if (m) { m.s ||= s; m.w ||= w; } else metas.set(k, { s, w });
  };
  add(feePayer, true, true);
  for (const ix of ixs) { for (const k of ix.keys) add(k.pubkey, k.isSigner, k.isWritable); add(ix.programId, false, false); }
  // solana-message's CompiledKeys: the fee payer first, then each group in pubkey byte order (it keeps the keys in a BTreeMap)
  const bytes = new Map([...metas.keys()].map((k) => [k, keyBytes(k)]));
  const cmp = (a: Address, b: Address) => { const x = bytes.get(a)!, y = bytes.get(b)!; for (let i = 0; i < 32; i++) if (x[i] !== y[i]) return x[i] - y[i]; return 0; };
  const all = [...metas.entries()].filter(([k]) => k !== feePayer);
  const group = (s: boolean, w: boolean) => all.filter(([, m]) => m.s === s && m.w === w).map(([k]) => k).sort(cmp);
  const ws = [feePayer, ...group(true, true)], rs = group(true, false), wu = group(false, true), ru = group(false, false);
  const accountKeys = [...ws, ...rs, ...wu, ...ru];
  const index = new Map(accountKeys.map((k, i) => [k, i]));
  return {
    header: { numRequiredSignatures: ws.length + rs.length, numReadonlySigned: rs.length, numReadonlyUnsigned: ru.length },
    accountKeys, recentBlockhash,
    instructions: ixs.map((ix) => ({ programIdIndex: index.get(ix.programId)!, accounts: ix.keys.map((k) => index.get(k.pubkey)!), data: ix.data })),
  };
}

export function serializeMessage(m: Message): Uint8Array {
  const out: number[] = [m.header.numRequiredSignatures, m.header.numReadonlySigned, m.header.numReadonlyUnsigned, ...shortvec(m.accountKeys.length)];
  for (const k of m.accountKeys) out.push(...keyBytes(k));
  const bh = b58decode(m.recentBlockhash);
  if (bh.length !== 32) throw new Error("the blockhash is not 32 bytes");
  out.push(...bh);
  out.push(...shortvec(m.instructions.length));
  for (const ix of m.instructions) out.push(ix.programIdIndex, ...shortvec(ix.accounts.length), ...ix.accounts, ...shortvec(ix.data.length), ...ix.data);
  return Uint8Array.from(out);
}

export function decodeMessage(b: Uint8Array): Message {
  if (b[0] & 0x80) throw new Error("a versioned message: only legacy messages are decoded here");
  const header = { numRequiredSignatures: b[0], numReadonlySigned: b[1], numReadonlyUnsigned: b[2] };
  let [n, i] = readShortvec(b, 3);
  const accountKeys: Address[] = [];
  for (let k = 0; k < n; k++, i += 32) accountKeys.push(b58encode(b.subarray(i, i + 32)));
  const recentBlockhash = b58encode(b.subarray(i, i + 32)); i += 32;
  [n, i] = readShortvec(b, i);
  const instructions: Message["instructions"] = [];
  for (let k = 0; k < n; k++) {
    const programIdIndex = b[i++];
    let na: number; [na, i] = readShortvec(b, i);
    const accounts = [...b.subarray(i, i + na)]; i += na;
    let nd: number; [nd, i] = readShortvec(b, i);
    const data = b.slice(i, i + nd); i += nd;
    instructions.push({ programIdIndex, accounts, data });
  }
  return { header, accountKeys, recentBlockhash, instructions };
}

/** The instructions of a decoded message, with their account flags back (for tests and for checking what a wallet returns). */
export function instructionsOf(m: Message): Instruction[] {
  const { numRequiredSignatures: s, numReadonlySigned: rs, numReadonlyUnsigned: ru } = m.header;
  const n = m.accountKeys.length;
  const flags = (i: number) => ({ isSigner: i < s, isWritable: i < s ? i < s - rs : i < n - ru });
  return m.instructions.map((ix) => ({ programId: m.accountKeys[ix.programIdIndex], keys: ix.accounts.map((a) => ({ pubkey: m.accountKeys[a], ...flags(a) })), data: ix.data }));
}

/** A transaction on the wire: the signatures (zeros where a signer has not signed yet), then the message. */
export function serializeTransaction(message: Uint8Array, signatures: Uint8Array[]): Uint8Array {
  const out = new Uint8Array(shortvec(signatures.length).length + 64 * signatures.length + message.length);
  const sv = shortvec(signatures.length);
  out.set(sv, 0);
  signatures.forEach((s, k) => out.set(s, sv.length + 64 * k));
  out.set(message, sv.length + 64 * signatures.length);
  return out;
}
export function parseTransaction(wire: Uint8Array): { signatures: Uint8Array[]; message: Uint8Array } {
  const [n, i] = readShortvec(wire, 0);
  const signatures: Uint8Array[] = [];
  for (let k = 0; k < n; k++) signatures.push(wire.slice(i + 64 * k, i + 64 * (k + 1)));
  return { signatures, message: wire.slice(i + 64 * n) };
}
export const unsignedTransaction = (m: Message): Uint8Array =>
  serializeTransaction(serializeMessage(m), Array.from({ length: m.header.numRequiredSignatures }, () => new Uint8Array(64)));
export const txSize = (m: Message): number => unsignedTransaction(m).length;
