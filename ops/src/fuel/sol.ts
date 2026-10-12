import { createHash } from "node:crypto";
import { base58 } from "@scure/base";
import { hashStruct, type Hex } from "viem";
import { encodeAddress, ETH_NATIVE, ORDER_TYPES } from "../relay.js";

/**
 * docs/v9/fueler-solana.md (the programs' `allocate` retired with rook_core, decision D-l: on the books home the canister's own cranks pay
 * the ops share to the hot key, docs/v9/canister-only-solana.md §9.2): the fueler's Solana leg, the pure half (no network, no keys, no
 * clock). Everything the Solana hot key may ever sign is built and checked here:
 *  - Relay's `deposit_native(amount, id)` for a SOL → ETH (Ethereum) order judged by the Solana rules S1–S12 of
 *    autobuyer/canister-rs/src/relaysol.rs (docs/v9/spikes/q5-relay-solana-legs.md §3), with the depository program pinned (S3: its
 *    allocator, upgrade authority and deploy slot);
 *  - a plain System transfer to the cold payee (what the wallet holds above its buffer and a day's cap).
 * Each instruction is written here from judged values only (never copied from a quote); `allowedSol` is the only gate before a signature.
 */

// ---- pinned values (relaysol.rs, read from mainnet 2026-10-09) -------------------------------------------------------------------

export const SYSTEM_PROGRAM = "11111111111111111111111111111111";
export const COMPUTE_BUDGET = "ComputeBudget111111111111111111111111111111";
export const SOL_NATIVE = SYSTEM_PROGRAM;
/** Relay's chain id for Solana. */
export const RELAY_SOLANA = 792_703_809;
export const RELAY_SOL = {
  DEPOSITORY_PROGRAM: "99vQwtBwYtrqqD9YSXbdum3KBdxPAVxYTaQ3cfnJSrN2",
  /** `RelayDepository` state, PDA ["relay_depository"]. */
  RELAY_DEPOSITORY: "Dodg2HifwU8rmaVVyMyUZDGTRbqAJTyVYxXPwcbNpBKc",
  /** The SOL vault, PDA ["vault"] (bump 255). */
  VAULT: "7uTT8Xi5RWXzy7h9XL244GRgEycDYDhLjr3ZyNdXi8pZ",
  ALLOCATOR: "HuAjVESMnrR2wuthKHeAsJeDsfN2jKUEi7TCcZC7eW6Z",
  OWNER_AND_UPGRADE_AUTHORITY: "6Bz8AWWipCDf7XCSqNuRXrEtKZM5M4Y56uP9ZRjSaMWD",
  PROGRAMDATA: "6y7C7Lfh1WRRbKohE2FQmFBD2asw3yMi17kStwEcAWWF",
  PROGRAM_DEPLOY_SLOT: 386_211_280n,
  BPF_LOADER_UPGRADEABLE: "BPFLoaderUpgradeab1e11111111111111111111111",
} as const;
/** The operator's fund wallet (docs/v9/solana-migration.md §2.9 D-g): the default cold payee of the Solana home. */
export const FUND_WALLET = "GGgXnNgtY4ySfz8PLAAwkoh8Ztj7p6jrM9towSaa6Wuy";

export const QUOTE_TTL_S = 300;
const DAY = 86_400;
const MAX_QUOTE_BYTES = 64 * 1024;

// ---- keys and addresses -----------------------------------------------------------------------------------------------------------

export const b58 = (b: Uint8Array) => base58.encode(b);
export function key(s: string): Uint8Array {
  let b: Uint8Array;
  try { b = base58.decode(s); } catch { throw new Error(`${s} is not base58`); }
  if (b.length !== 32) throw new Error(`${s} is not a 32-byte Solana address`);
  return b;
}
export const isPubkey = (s: string): boolean => { try { key(s); return true; } catch { return false; } };

const sha256 = (...parts: Uint8Array[]) => { const h = createHash("sha256"); for (const p of parts) h.update(p); return new Uint8Array(h.digest()); };

/** Anchor's 8-byte discriminator, sha256("<ns>:<name>")[..8]. */
export const anchorDisc = (ns: string, name: string) => sha256(Buffer.from(`${ns}:${name}`)).slice(0, 8);

// ed25519 point decompression as curve25519-dalek does it (y reduced mod p; the sign bit ignored for validity): a PDA must NOT decode
const P = 2n ** 255n - 19n;
const D = mod(-121665n * inv(121666n));
function mod(a: bigint) { const r = a % P; return r >= 0n ? r : r + P; }
function pow(b: bigint, e: bigint) { let r = 1n; b = mod(b); while (e > 0n) { if (e & 1n) r = (r * b) % P; b = (b * b) % P; e >>= 1n; } return r; }
function inv(a: bigint) { return pow(mod(a), P - 2n); }
export function onCurve(b: Uint8Array): boolean {
  let y = 0n;
  for (let i = 31; i >= 0; i--) y = (y << 8n) | BigInt(b[i]);
  y = mod(y & ((1n << 255n) - 1n));
  const y2 = (y * y) % P, u = mod(y2 - 1n), v = mod(D * y2 + 1n);
  // sqrt_ratio_i(u, v): x = u v³ (u v⁷)^((p−5)/8); valid when v x² = ±u
  const v3 = (v * v % P) * v % P, x = (u * v3 % P) * pow(u * (v3 * v3 % P) % P * v % P, (P - 5n) / 8n) % P;
  const vx2 = v * (x * x % P) % P;
  return vx2 === u || vx2 === mod(-u);
}
export function createProgramAddress(seeds: Uint8Array[], program: string): string | null {
  const h = sha256(...seeds, key(program), Buffer.from("ProgramDerivedAddress"));
  return onCurve(h) ? null : b58(h);
}
export function findProgramAddress(seeds: Uint8Array[], program: string): [string, number] {
  for (let bump = 255; bump >= 0; bump--) {
    const a = createProgramAddress([...seeds, Uint8Array.of(bump)], program);
    if (a) return [a, bump];
  }
  throw new Error("no program address");
}
// ---- account decoders -------------------------------------------------------------------------------------------------------------

const u64le = (b: Uint8Array, at: number) => Buffer.from(b.buffer, b.byteOffset, b.byteLength).readBigUInt64LE(at);
const eq = (a: Uint8Array, b: Uint8Array) => a.length === b.length && a.every((x, i) => x === b[i]);

export const ACC_RELAY_DEPOSITORY = Uint8Array.from([0x80, 0x35, 0x9c, 0xbd, 0xf3, 0x53, 0xf5, 0x6b]);
/** The S3 reads from the two accounts (relaysol.rs s3_reads): the depository state's allocator, the ProgramData's slot and authority. */
export function s3Reads(state: { owner: string; data: Uint8Array }, programdata: { owner: string; data: Uint8Array }): S3 {
  if (state.owner !== RELAY_SOL.DEPOSITORY_PROGRAM) throw new Error("rule S3: the depository state is not owned by the depository program");
  if (programdata.owner !== RELAY_SOL.BPF_LOADER_UPGRADEABLE) throw new Error("rule S3: the ProgramData is not owned by the upgradeable loader");
  const s = state.data, p = programdata.data;
  if (s.length < 73 || !eq(s.slice(0, 8), ACC_RELAY_DEPOSITORY)) throw new Error("rule S3: the depository state does not decode");
  if (p.length < 13 || !eq(p.slice(0, 4), Uint8Array.of(3, 0, 0, 0))) throw new Error("rule S3: the ProgramData header does not decode");
  const auth = p[12] === 1 && p.length >= 45 ? b58(p.slice(13, 45)) : p[12] === 0 ? null : (() => { throw new Error("rule S3: bad authority option"); })();
  return { allocator: b58(s.slice(40, 72)), upgradeAuthority: auth, deploySlot: u64le(p, 4) };
}
export type S3 = { allocator: string; upgradeAuthority: string | null; deploySlot: bigint };

// ---- the order id (relayconv::order_id with "solana": base58 → 32 raw bytes) ----------------------------------------------------

type AnyObj = Record<string, any>; // eslint-disable-line @typescript-eslint/no-explicit-any
export function encodeAddr(address: string, chain: string): Hex {
  if (chain === "solana") return `0x${Buffer.from(key(address)).toString("hex")}` as Hex;
  return encodeAddress(address, chain);
}
export function orderIdAny(o: AnyObj): Hex {
  const data = {
    version: o.version, solverChainId: o.solverChainId, solver: o.solver, salt: BigInt(o.salt),
    inputs: o.inputs.map((i: AnyObj) => ({
      payment: { chainId: i.payment.chainId, currency: encodeAddr(i.payment.currency, i.payment.chainId), amount: BigInt(i.payment.amount), weight: BigInt(i.payment.weight) },
      refunds: i.refunds.map((r: AnyObj) => ({ chainId: r.chainId, recipient: encodeAddr(r.recipient, r.chainId), currency: encodeAddr(r.currency, r.chainId),
        minimumAmount: BigInt(r.minimumAmount), deadline: Number(r.deadline), extraData: r.extraData as Hex })),
    })),
    output: {
      chainId: o.output.chainId,
      payments: o.output.payments.map((p: AnyObj) => ({ recipient: encodeAddr(p.recipient, o.output.chainId), currency: encodeAddr(p.currency, o.output.chainId),
        minimumAmount: BigInt(p.minimumAmount), expectedAmount: BigInt(p.expectedAmount) })),
      deadline: Number(o.output.deadline), calls: o.output.calls as Hex[], extraData: o.output.extraData as Hex,
    },
    fees: o.fees.map((f: AnyObj) => ({ recipientChainId: f.recipientChainId, recipient: encodeAddr(f.recipient, f.recipientChainId), currencyChainId: f.currencyChainId,
      currency: encodeAddr(f.currency, f.currencyChainId), amount: BigInt(f.amount) })),
  };
  return hashStruct({ types: ORDER_TYPES, primaryType: "Order", data } as never);
}

// ---- instructions -------------------------------------------------------------------------------------------------------------------

export type Meta = { pubkey: string; signer: boolean; writable: boolean };
export type Ix = { program: string; keys: Meta[]; data: Uint8Array };
const m = (pubkey: string, signer: boolean, writable: boolean): Meta => ({ pubkey, signer, writable });

export const IX_DEPOSIT_NATIVE = Uint8Array.from([0x0d, 0x9e, 0x0d, 0xdf, 0x5f, 0xd5, 0x1c, 0x06]);
const u64b = (x: bigint) => { const b = Buffer.alloc(8); b.writeBigUInt64LE(x); return b; };

/** Relay `deposit_native(amount, id)` with the program's five accounts for `hot` as sender and depositor (relaysol.rs deposit_native_metas). */
export const depositMetas = (hot: string): Meta[] => [m(RELAY_SOL.RELAY_DEPOSITORY, false, false), m(hot, true, true), m(hot, false, false),
  m(RELAY_SOL.VAULT, false, true), m(SYSTEM_PROGRAM, false, false)];
export const depositIx = (hot: string, lamports: bigint, id: Hex): Ix => {
  const idb = Buffer.from(id.slice(2), "hex");
  if (idb.length !== 32) throw new Error("an order id is 32 bytes");
  return { program: RELAY_SOL.DEPOSITORY_PROGRAM, keys: depositMetas(hot), data: Uint8Array.from(Buffer.concat([IX_DEPOSIT_NATIVE, u64b(lamports), idb])) };
};
/** System `Transfer { lamports }`. */
export const transferIx = (from: string, to: string, lamports: bigint): Ix =>
  ({ program: SYSTEM_PROGRAM, keys: [m(from, true, true), m(to, false, true)], data: Uint8Array.from(Buffer.concat([Buffer.from([2, 0, 0, 0]), u64b(lamports)])) });
export const cuLimitIx = (units: number): Ix => { const b = Buffer.alloc(5); b[0] = 2; b.writeUInt32LE(units, 1); return { program: COMPUTE_BUDGET, keys: [], data: Uint8Array.from(b) }; };
export const cuPriceIx = (microLamports: bigint): Ix => ({ program: COMPUTE_BUDGET, keys: [], data: Uint8Array.from(Buffer.concat([Buffer.from([3]), u64b(microLamports)])) });

// ---- the legacy message and transaction ----------------------------------------------------------------------------------------------

const compact = (n: number): number[] => { const out: number[] = []; for (;;) { let b = n & 0x7f; n >>= 7; if (n) { b |= 0x80; out.push(b); } else { out.push(b); return out; } } };

/** A legacy message: the payer first, then signer-writable, signer-readonly, writable, readonly keys (each in first-seen order). */
export function compileLegacy(payer: string, ixs: Ix[], blockhash: string): Uint8Array {
  const flags = new Map<string, { s: boolean; w: boolean }>();
  const add = (k: string, s: boolean, w: boolean) => { const f = flags.get(k); if (f) { f.s ||= s; f.w ||= w; } else flags.set(k, { s, w }); };
  add(payer, true, true);
  for (const ix of ixs) { for (const k of ix.keys) add(k.pubkey, k.signer, k.writable); add(ix.program, false, false); }
  const all = [...flags.entries()];
  const group = (s: boolean, w: boolean) => all.filter(([k, f]) => k !== payer && f.s === s && f.w === w).map(([k]) => k);
  const keys = [payer, ...group(true, true), ...group(true, false), ...group(false, true), ...group(false, false)];
  const nSig = 1 + group(true, true).length + group(true, false).length;
  const out: number[] = [nSig, group(true, false).length, group(false, false).length, ...compact(keys.length)];
  for (const k of keys) out.push(...key(k));
  out.push(...key(blockhash), ...compact(ixs.length));
  for (const ix of ixs) {
    out.push(keys.indexOf(ix.program), ...compact(ix.keys.length), ...ix.keys.map((k) => keys.indexOf(k.pubkey)), ...compact(ix.data.length), ...ix.data);
  }
  return Uint8Array.from(out);
}
export const txBytes = (sigs: Uint8Array[], msg: Uint8Array) => Uint8Array.from([...compact(sigs.length), ...sigs.flatMap((s) => [...s]), ...msg]);

// ---- the allowlist ----------------------------------------------------------------------------------------------------------------------

export type SolKind = "relay" | "forward";
export type SolCtx = { hot: string; payee: string; maxPriceMicro: bigint; maxUnits: number };
const sameMetas = (a: Meta[], b: Meta[]) => a.length === b.length && a.every((x, i) => x.pubkey === b[i].pubkey && x.signer === b[i].signer && x.writable === b[i].writable);

/**
 * The only Solana transactions the fueler may sign (the counterpart of engine.ts `allowed`): at most two ComputeBudget instructions
 * (a unit limit ≤ maxUnits, a price ≤ maxPriceMicro), then exactly one of
 *  - Relay's `deposit_native` with the depository's own five accounts, the hot key sender AND depositor (credited to no one else);
 *  - a System transfer from the hot key to the cold payee, value > 0.
 * There is no generic send. Throws the refusal.
 */
export function allowedSol(ixs: Ix[], c: SolCtx): SolKind {
  const no = (why: string): never => { throw new Error(`refused: no rule lets the fueler sign this on Solana (${why})`); };
  const budget = ixs.filter((i) => i.program === COMPUTE_BUDGET), main = ixs.filter((i) => i.program !== COMPUTE_BUDGET);
  if (budget.length > 2 || main.length !== 1 || ixs[ixs.length - 1] !== main[0]) no("one instruction, after at most two compute-budget ones");
  for (const b of budget) {
    if (b.keys.length) no("a compute-budget instruction with accounts");
    if (b.data[0] === 2 && b.data.length === 5) { if (Buffer.from(b.data).readUInt32LE(1) > c.maxUnits) no("a compute-unit limit over the cap"); }
    else if (b.data[0] === 3 && b.data.length === 9) { if (u64le(b.data, 1) > c.maxPriceMicro) no("a compute-unit price over the cap"); }
    else no("an unknown compute-budget instruction");
  }
  const ix = main[0];
  if (ix.program === RELAY_SOL.DEPOSITORY_PROGRAM && ix.data.length === 48 && eq(ix.data.slice(0, 8), IX_DEPOSIT_NATIVE) && sameMetas(ix.keys, depositMetas(c.hot))) return "relay";
  if (ix.program === SYSTEM_PROGRAM && ix.data.length === 12 && eq(ix.data.slice(0, 4), Uint8Array.of(2, 0, 0, 0)) && u64le(ix.data, 4) > 0n
    && sameMetas(ix.keys, transferIx(c.hot, c.payee, 1n).keys) && c.payee !== c.hot) return "forward";
  return no(`${ix.program} with ${ix.keys.length} accounts and data ${Buffer.from(ix.data.slice(0, 8)).toString("hex")}…`);
}

// ---- the Relay judge (S1–S12) -----------------------------------------------------------------------------------------------------

export type SolWant = {
  /** the hot key: sender, depositor and the Solana refund recipient */
  hot: string;
  /** the fueler's own Ethereum address: the output and the Ethereum refunds */
  recipient: string;
  lamports: bigint;
  /** the least the order must guarantee (S10), from the oracle less the fee bound */
  minOutFloor: bigint;
  s3: S3;
  nowS: number;
  used: string[];
};
export type Judged = { orderId: Hex; lamports: bigint; minOut: bigint; deadline: number };

/** relaysol.rs `judge_sol`, for a SOL → ETH (Ethereum) order whose Solana side is the hot key. Every refusal names the rule. */
export function judgeSolQuote(q: AnyObj, w: SolWant): Judged {
  const need = (c: unknown, why: string) => { if (!c) throw new Error(why); };
  need(JSON.stringify(q ?? null).length <= MAX_QUOTE_BYTES, `the quote is over ${MAX_QUOTE_BYTES} bytes`);
  const lc = (s: unknown) => String(s ?? "").toLowerCase();
  const sameEvm = (a: unknown, b: string) => lc(a) === lc(b);
  // S1: one deposit step, one item, one instruction (the lookup tables are never used: the fueler writes its own message)
  const steps = q?.steps;
  need(Array.isArray(steps) && steps.length === 1, `rule S1: ${steps?.length} steps; only a single deposit step is signed`);
  need(steps[0].id === "deposit" && steps[0].kind === "transaction", "rule S1: the one step is not a deposit transaction");
  need(Array.isArray(steps[0].items) && steps[0].items.length === 1, "rule S1: exactly one transaction is signed");
  const ixs = steps[0].items[0]?.data?.instructions;
  need(Array.isArray(ixs) && ixs.length === 1, "rule S1: only the one deposit_native instruction is signed");
  const ix = ixs[0];
  // S3: the pinned program, its allocator and the code reviewed
  need(ix.programId === RELAY_SOL.DEPOSITORY_PROGRAM, `rule S3: the instruction goes to ${ix.programId}, not Relay's pinned depository program`);
  need(w.s3.allocator === RELAY_SOL.ALLOCATOR, `rule S3: the depository's allocator is ${w.s3.allocator}, not the pinned ${RELAY_SOL.ALLOCATOR}: review before converting`);
  need(w.s3.upgradeAuthority === RELAY_SOL.OWNER_AND_UPGRADE_AUTHORITY && w.s3.deploySlot === RELAY_SOL.PROGRAM_DEPLOY_SLOT,
    "rule S3: the depository program was upgraded (or its authority changed) since it was reviewed");
  // S5: exactly the program's accounts for the hot key as sender and depositor, flags included
  const metas: Meta[] = (ix.keys ?? []).map((k: AnyObj) => m(String(k.pubkey), k.isSigner === true, k.isWritable === true));
  need(sameMetas(metas, depositMetas(w.hot)), "rule S5: the accounts are not (relay_depository, the hot key signer, the hot key depositor, vault, system program)");
  const data = Buffer.from(String(ix.data ?? ""), "hex");
  need(data.length === 48 && eq(data.subarray(0, 8), IX_DEPOSIT_NATIVE), "rule S5: the data is not deposit_native(u64, [u8; 32])");
  const amount = data.readBigUInt64LE(8), id = `0x${data.subarray(16).toString("hex")}` as Hex;
  // S4
  need(amount === w.lamports, `rule S4: the deposit sends ${amount} lamports, not the ${w.lamports} asked`);
  // S6: the order is the order inspected
  const v2 = q?.protocol?.v2, o = v2?.orderData;
  need(o, "rule S6: no protocol.v2.orderData");
  let computed: Hex;
  try { computed = orderIdAny(o); } catch (e) { throw new Error(`rule S6: the order cannot be hashed: ${(e as Error).message}`); }
  need(computed === id, `rule S6: the instruction commits to ${id}, but the order hashes to ${computed}`);
  need(lc(v2.orderId) === id, "rule S6: protocol.v2.orderId is not the instruction's id");
  // S2
  const pd = v2.paymentDetails ?? {};
  need(pd.chainId === "solana" && pd.depository === RELAY_SOL.DEPOSITORY_PROGRAM && pd.currency === SOL_NATIVE && BigInt(pd.amount ?? -1) === amount,
    "rule S2/S3/S4: paymentDetails names another chain, program, currency or amount");
  // S7
  need(o.inputs?.length === 1, `rule S7: ${o.inputs?.length} inputs, not one`);
  const p = o.inputs[0].payment ?? {};
  need(p.chainId === "solana" && p.currency === SOL_NATIVE && BigInt(p.amount ?? -1) === amount, "rule S7: the order's payment is not native SOL on solana for the deposit's amount");
  // S8: every refund ours; one on solana
  const band = (t: number) => Number.isInteger(t) && t <= 0xffff_ffff && t >= w.nowS + DAY && t <= w.nowS + 8 * DAY;
  const refunds: AnyObj[] = o.inputs[0].refunds ?? [];
  for (const [i, r] of refunds.entries()) {
    const ok = r.chainId === "solana" ? r.recipient === w.hot && r.currency === SOL_NATIVE
      : r.chainId === "ethereum" ? sameEvm(r.recipient, w.recipient) && sameEvm(r.currency, ETH_NATIVE) : false;
    need(ok, `rule S8: refunds[${i}] pays ${r.recipient} on ${r.chainId}, not our own address in its native currency`);
    need(band(Number(r.deadline)), `rule S11: refunds[${i}]'s deadline ${r.deadline} is outside [now + 1 day, now + 8 days]`);
  }
  need(refunds.some((r) => r.chainId === "solana"), "rule S8: no refund on solana to the hot key");
  // S9
  const out = o.output ?? {};
  need(out.chainId === "ethereum", "rule S9: the order pays out on another chain");
  need(out.payments?.length === 1, `rule S9: ${out.payments?.length} payments, not one`);
  const pay = out.payments[0];
  need(sameEvm(pay.recipient, w.recipient), `rule S9: the order pays ${pay.recipient}, not ${w.recipient}`);
  need(sameEvm(pay.currency, ETH_NATIVE), "rule S9: the order does not pay native ETH");
  need((out.calls ?? []).length === 0, "rule S9: the order carries calls");
  need((o.fees ?? []).length === 0, "rule S9: the order carries fees");
  // S10
  const minOut = BigInt(pay.minimumAmount);
  need(minOut >= w.minOutFloor, `rule S10: the order guarantees ${minOut} wei, under our floor of ${w.minOutFloor}`);
  // S11
  need(band(Number(out.deadline)), `rule S11: the order's deadline ${out.deadline} is outside [now + 1 day, now + 8 days]`);
  // S12
  need(!w.used.includes(id), `rule S12: order ${id} was accepted before: never twice`);
  return { orderId: id, lamports: amount, minOut, deadline: Number(out.deadline) };
}

/** S10's floor: the oracle's ETH for `lamports` of SOL, less `maxFeeBps`. */
export function minOutFloor(lamports: bigint, solUsd: number, ethUsd: number, maxFeeBps: number): bigint {
  const fair = BigInt(Math.floor((Number(lamports) / 1e9) * (solUsd / ethUsd) * 1e18));
  return (fair * BigInt(10_000 - maxFeeBps)) / 10_000n;
}

/** The Relay quote request for SOL (Solana) → ETH (Ethereum), the hot key paying, our Ethereum address receiving. */
export const solQuoteBody = (hot: string, recipient: string, lamports: bigint, slippageBps: number) => ({
  user: hot, originChainId: RELAY_SOLANA, destinationChainId: 1, originCurrency: SOL_NATIVE, destinationCurrency: ETH_NATIVE,
  recipient, refundTo: hot, tradeType: "EXACT_INPUT", amount: lamports.toString(), slippageTolerance: String(slippageBps),
});
