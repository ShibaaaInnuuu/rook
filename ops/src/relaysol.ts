import type { Hex } from "viem";
import { depositMetas, encodeAddr, IX_DEPOSIT_NATIVE, orderIdAny, RELAY_SOL, RELAY_SOLANA, s3Reads, SOL_NATIVE, type Meta, type S3 } from "./fuel/sol.js";
import { BTC_NATIVE, ETH_NATIVE, RELAY_DEPOSITORY_4663, DEPOSIT_NATIVE } from "./relay.js";

/**
 * docs/v9/solana-migration.md §5 "Runner": Relay quotes with a SOLANA side, pre-judged by the canister's own rules before they are
 * handed in, so a quote the canister would refuse is not worth a call. The canister judges again on its own finalized reads and
 * writes its own deposit instruction from the judged amount and order id (relaysol.rs `deposit_instruction`): nothing here is trusted.
 *
 *  - `judgeSolOrigin`: autobuyer/canister-rs/src/relaysol.rs `judge_sol`, rules S1–S12, for SOL → BTC (a frog conversion,
 *    `relay_quote`) and SOL → ETH (an Ethereum NFT buy's advance, `eth_relay_quote`). Audit A5-03: the deposit's sender and depositor
 *    are the canister's dedicated Relay key (the ask's `user`), every Solana refund goes to the canister's MAIN Solana address (the
 *    ask's `refund_to`), never to the Relay key.
 *  - `judgeEthToSol`: relayeth.rs `judge_to_solana` (rules 1–12), for an Ethereum NFT buy's way back (ETH → SOL, `eth_relay_quote`).
 *
 * Rule S10 / 10 (the minimum out against a reference rate) is the canister's alone unless the caller passes a floor: the runner holds
 * no rate it trusts. Rule S12 / 12 (an order accepted once is never accepted again) is the canister's alone: the runner does not know
 * the used orders. S3 (the depository's allocator, upgrade authority and deploy slot) is checked when the runner could read the two
 * accounts (SOL_RPC_URLS); otherwise it is the canister's alone and only the pinned program id is checked here.
 */

type AnyObj = Record<string, any>; // eslint-disable-line @typescript-eslint/no-explicit-any

export { RELAY_SOLANA, SOL_NATIVE, RELAY_SOL };
export const ETHEREUM = 1;
/** Audit C3-03: Robinhood Chain, as the canister's `relayeth::relay_name(4663)` names it ("robinhood"): its NFT side's two Solana legs. */
export const ROBINHOOD = 4663;
/** The NFT chains with a Solana Relay leg, by chain id, as Relay's orders name them (the canister's `relay_name` / `to_solana`). */
export const NFT_CHAINS: Record<number, "ethereum" | "robinhood"> = { [ETHEREUM]: "ethereum", [ROBINHOOD]: "robinhood" };
const DAY = 86_400;
const MAX_QUOTE_BYTES = 64 * 1024;

/** What a Solana-origin quote is judged against (relaysol.rs `SolWant`, less the canister-only reads). */
export type SolOriginWant = {
  /** the canister's main Solana address: every Solana refund */
  ourSol: string;
  /** audit A5-03: the canister's dedicated Relay key: the deposit's sender and depositor (never a refund recipient) */
  depositor: string;
  destChain: "bitcoin" | "ethereum" | "robinhood";
  destRecipient: string;
  destCurrency: string;
  lamports: bigint;
  /** S10, when the caller has a floor it trusts; otherwise only "a minimum above zero" */
  minOutFloor?: bigint;
  /** S3's reads (the depository state's allocator, its ProgramData's slot and authority), or null when not read */
  s3?: S3 | null;
  nowS: number;
};
export type SolJudged = { orderId: Hex; lamports: bigint; minOut: bigint; deadline: number };

const need = (c: unknown, why: string): void => { if (!c) throw new Error(why); };
/** Same address on `chain`, compared as Relay encodes it (relaysol.rs `same`). */
export const sameOn = (a: unknown, b: string, chain: string): boolean => {
  try { return encodeAddr(String(a), chain) === encodeAddr(b, chain); } catch { return false; }
};
const big = (x: unknown, what: string): bigint => { try { return BigInt(String(x)); } catch { throw new Error(`${what} is not a whole number`); } };
const band = (t: number, nowS: number) => Number.isInteger(t) && t <= 0xffff_ffff && t >= nowS + DAY && t <= nowS + 8 * DAY;
const sameMetas = (a: Meta[], b: Meta[]) => a.length === b.length && a.every((x, i) => x.pubkey === b[i].pubkey && x.signer === b[i].signer && x.writable === b[i].writable);

/** relaysol.rs `judge_sol`, S1–S12 (S10 against `minOutFloor` when given, S12 the canister's). Every refusal names the rule. */
export function judgeSolOrigin(q: AnyObj, w: SolOriginWant): SolJudged {
  need(JSON.stringify(q ?? null).length <= MAX_QUOTE_BYTES, `the quote is over ${MAX_QUOTE_BYTES} bytes`);
  // S1: one deposit step, one item, one instruction; the lookup tables are never used (the canister writes its own message)
  const steps = q?.steps;
  need(Array.isArray(steps) && steps.length === 1, `rule S1: ${Array.isArray(steps) ? steps.length : 0} steps; only a single deposit step is signed`);
  need(steps[0]?.id === "deposit" && steps[0]?.kind === "transaction", "rule S1: the one step is not a deposit transaction");
  need(Array.isArray(steps[0].items) && steps[0].items.length === 1, `rule S1: ${steps[0].items?.length} items; exactly one transaction is signed`);
  const ixs = steps[0].items[0]?.data?.instructions;
  need(Array.isArray(ixs), "rule S1/S2: not a Solana deposit (no instructions)");
  need(ixs.length === 1, `rule S1: ${ixs.length} instructions; only the one deposit_native is signed`);
  const ix = ixs[0] ?? {};
  // S3: the pinned program, and (when read) its allocator and the code reviewed
  need(ix.programId === RELAY_SOL.DEPOSITORY_PROGRAM, `rule S3: the instruction goes to ${ix.programId}, not Relay's pinned depository program`);
  if (w.s3) {
    need(w.s3.allocator === RELAY_SOL.ALLOCATOR, `rule S3: the depository's allocator is ${w.s3.allocator}, not the pinned ${RELAY_SOL.ALLOCATOR}: review before converting`);
    need(w.s3.upgradeAuthority === RELAY_SOL.OWNER_AND_UPGRADE_AUTHORITY && w.s3.deploySlot === RELAY_SOL.PROGRAM_DEPLOY_SLOT,
      "rule S3: the depository program was upgraded (or its authority changed) since it was reviewed");
  }
  // S5: exactly the program's accounts for the Relay key as sender and depositor, flags included
  need(Array.isArray(ix.keys) && ix.keys.every((k: AnyObj) => typeof k?.isSigner === "boolean" && typeof k?.isWritable === "boolean"), "rule S1: the instruction's keys do not decode");
  const metas: Meta[] = ix.keys.map((k: AnyObj) => ({ pubkey: String(k.pubkey), signer: k.isSigner, writable: k.isWritable }));
  need(sameMetas(metas, depositMetas(w.depositor)),
    "rule S5: the accounts are not (relay_depository, our Relay key signer, our Relay key depositor, vault, system program)");
  need(/^[0-9a-fA-F]*$/.test(String(ix.data ?? "")), "rule S1: instructions[0].data is not hex");
  const data = Buffer.from(String(ix.data ?? ""), "hex");
  need(data.length === 48 && data.subarray(0, 8).equals(Buffer.from(IX_DEPOSIT_NATIVE)), "rule S5: the data is not deposit_native(u64, [u8; 32])");
  const amount = data.readBigUInt64LE(8), id = `0x${data.subarray(16).toString("hex")}` as Hex;
  // S4
  need(amount === w.lamports, `rule S4: the deposit sends ${amount} lamports, not the ${w.lamports} asked`);
  // S6: the order is the order inspected
  const v2 = q?.protocol?.v2, o = v2?.orderData;
  need(o, "rule S6: no protocol.v2.orderData: nothing binds the deposit to a recipient");
  let computed: Hex;
  try { computed = orderIdAny(o); } catch (e) { throw new Error(`rule S6: the order cannot be hashed: ${(e as Error).message}`); }
  need(computed === id, `rule S6: the instruction commits to ${id}, but the order hashes to ${computed}`);
  need(String(v2.orderId ?? "").toLowerCase() === id, "rule S6: protocol.v2.orderId is not the instruction's id");
  // S2
  const pd = v2.paymentDetails ?? {};
  need(pd.chainId === "solana" && pd.depository === RELAY_SOL.DEPOSITORY_PROGRAM && pd.currency === SOL_NATIVE && big(pd.amount ?? -1, "paymentDetails.amount") === amount,
    "rule S2/S3/S4: paymentDetails names another chain, program, currency or amount");
  // S7
  need(Array.isArray(o.inputs) && o.inputs.length === 1, `rule S7: ${o.inputs?.length} inputs, not one`);
  const p = o.inputs[0].payment ?? {};
  need(p.chainId === "solana" && p.currency === SOL_NATIVE && big(p.amount ?? -1, "payment.amount") === amount,
    "rule S7: the order's payment is not native SOL on solana for the deposit's amount");
  // S8: every refund ours (Solana: the MAIN address, A5-03), one on solana; S11 on each
  const refunds: AnyObj[] = Array.isArray(o.inputs[0].refunds) ? o.inputs[0].refunds : [];
  for (const [i, r] of refunds.entries()) {
    const ok = r.chainId === "solana" ? sameOn(r.recipient, w.ourSol, "solana") && r.currency === SOL_NATIVE
      : r.chainId === w.destChain ? sameOn(r.recipient, w.destRecipient, w.destChain) && sameOn(r.currency, w.destCurrency, w.destChain) : false;
    need(ok, `rule S8: refunds[${i}] pays ${r.recipient} on ${r.chainId}, not our own address in its native currency`);
    need(band(Number(r.deadline), w.nowS), `rule S11: refunds[${i}]'s deadline ${r.deadline} is outside [now + 1 day, now + 8 days]`);
  }
  need(refunds.some((r) => r.chainId === "solana"), "rule S8: no refund on solana to our address");
  // S9
  const out = o.output ?? {};
  need(out.chainId === w.destChain, `rule S9: the order pays out on ${out.chainId}, not ${w.destChain}`);
  need(Array.isArray(out.payments) && out.payments.length === 1, `rule S9: ${out.payments?.length} payments, not one`);
  const pay = out.payments[0];
  need(sameOn(pay.recipient, w.destRecipient, w.destChain), `rule S9: the order pays ${pay.recipient}, not ${w.destRecipient}`);
  need(sameOn(pay.currency, w.destCurrency, w.destChain), "rule S9: the order does not pay the destination's native currency");
  need(Array.isArray(out.calls) && out.calls.length === 0, "rule S9: the order carries calls");
  need(Array.isArray(o.fees) && o.fees.length === 0, "rule S9: the order carries fees");
  // S10
  const minOut = big(pay.minimumAmount, "minimumAmount");
  need(minOut > 0n, "rule S10: no minimum out");
  if (w.minOutFloor !== undefined) need(minOut >= w.minOutFloor, `rule S10: the order guarantees ${minOut}, under our floor of ${w.minOutFloor}`);
  // S11
  need(band(Number(out.deadline), w.nowS), `rule S11: the order's deadline ${out.deadline} is outside [now + 1 day, now + 8 days]`);
  return { orderId: id, lamports: amount, minOut, deadline: Number(out.deadline) };
}

/** What an EVM → Solana quote is judged against (relayeth.rs `Want` with `ETHEREUM_TO_SOLANA` / `ROBINHOOD_TO_SOLANA` and a `SolDest`).
 * `origin`: the NFT chain the way back leaves from (audit C3-03: Ethereum by default, or Robinhood Chain 4663). */
export type EthToSolWant = { ours: string; solRecipient: string; amountWei: bigint; minOutFloor?: bigint; nowS: number; origin?: number };

/** relayeth.rs `judge_to_solana`: rules 1–9 and 11 (10 against `minOutFloor` when given; 12 the canister's), for the leg from `w.origin`
 * (Ethereum unless named; the same depository address on both chains, `relayeth::ROBINHOOD_TO_SOLANA`). */
export function judgeEthToSol(q: AnyObj, w: EthToSolWant): { orderId: Hex; minOut: bigint; deadline: number } {
  const originId = w.origin ?? ETHEREUM;
  const origin = NFT_CHAINS[originId];
  need(!!origin, `rule 2: no Solana Relay leg from chain ${originId}`);
  const lc = (s: unknown) => String(s ?? "").toLowerCase();
  need(JSON.stringify(q ?? null).length <= MAX_QUOTE_BYTES, `the quote is over ${MAX_QUOTE_BYTES} bytes`);
  const steps = q?.steps;
  need(Array.isArray(steps) && steps.length === 1 && steps[0]?.id === "deposit" && steps[0]?.kind === "transaction", "rule 1: not a single deposit transaction");
  need(Array.isArray(steps[0].items) && steps[0].items.length === 1, "rule 1: exactly one transaction is signed");
  const d = steps[0].items[0]?.data ?? {};
  need(Number(d.chainId) === originId, `rule 2: the deposit is for chain ${d.chainId}, not ${originId === ETHEREUM ? "Ethereum" : `${origin} (${originId})`}`);
  need(lc(d.to) === RELAY_DEPOSITORY_4663, `rule 3: the deposit goes to ${d.to}, not Relay's pinned depository`);
  if (d.from !== undefined) need(lc(d.from) === lc(w.ours), "rule 5: the quote was made for another sender");
  const value = big(d.value, "value");
  need(value === w.amountWei, `rule 4: the deposit sends ${value} wei, not the ${w.amountWei} asked`);
  const data = lc(d.data);
  need(data.length === 2 + 68 * 2 && data.startsWith(DEPOSIT_NATIVE), "rule 5: the calldata is not depositNative(address, bytes32)");
  need(/^0{24}$/.test(data.slice(10, 34)) && `0x${data.slice(34, 74)}` === lc(w.ours), "rule 5: the calldata names another depositor");
  const id = `0x${data.slice(74)}` as Hex;
  const v2 = q?.protocol?.v2, o = v2?.orderData;
  need(o, "rule 6: no protocol.v2.orderData");
  let computed: Hex;
  try { computed = orderIdAny(o); } catch (e) { throw new Error(`rule 6: the order cannot be hashed: ${(e as Error).message}`); }
  need(computed === id && lc(v2.orderId) === id, `rule 6: the order hashes to ${computed}, the calldata's id is ${id}, the quote says ${v2.orderId}`);
  need(lc(v2.paymentDetails?.depository) === RELAY_DEPOSITORY_4663 && big(v2.paymentDetails?.amount ?? -1, "paymentDetails.amount") === value,
    "rule 3/4: paymentDetails names another depository or amount");
  need(Array.isArray(o.inputs) && o.inputs.length === 1, `rule 7: ${o.inputs?.length} inputs, not one`);
  const p = o.inputs[0].payment ?? {};
  need(p.chainId === origin && lc(p.currency) === ETH_NATIVE && big(p.amount ?? -1, "payment.amount") === value, `rule 7: the order's payment is not native ETH on ${origin} for the value`);
  const refunds: AnyObj[] = Array.isArray(o.inputs[0].refunds) ? o.inputs[0].refunds : [];
  need(refunds.length > 0, "rule 8: the order has no refund");
  for (const [i, r] of refunds.entries()) {
    const ok = r.chainId === "solana" ? sameOn(r.recipient, w.solRecipient, "solana") && r.currency === SOL_NATIVE
      : r.chainId === origin ? lc(r.recipient) === lc(w.ours) && lc(r.currency) === ETH_NATIVE : false;
    need(ok, `rule 8: refunds[${i}] pays ${r.recipient} on ${r.chainId}, not our own address in its native currency`);
    need(band(Number(r.deadline), w.nowS), `rule 11: refunds[${i}]'s deadline ${r.deadline} is outside [now + 1 day, now + 8 days]`);
  }
  need(refunds.some((r) => r.chainId === origin), `rule 8: no refund on ${origin} to our address`);
  const out = o.output ?? {};
  need(out.chainId === "solana", `rule 9: the order pays out on ${out.chainId}, not solana`);
  need(Array.isArray(out.payments) && out.payments.length === 1, `rule 9: ${out.payments?.length} payments, not one`);
  const pay = out.payments[0];
  need(sameOn(pay.recipient, w.solRecipient, "solana"), `rule 9: the order pays ${pay.recipient}, not our own address`);
  need(pay.currency === SOL_NATIVE, "rule 9: the order does not pay solana's native currency");
  need(Array.isArray(out.calls) && out.calls.length === 0 && Array.isArray(o.fees) && o.fees.length === 0, "rule 9: the order carries calls or fees");
  const minOut = big(pay.minimumAmount, "minimumAmount");
  need(minOut > 0n, "rule 10: no minimum out");
  if (w.minOutFloor !== undefined) need(minOut >= w.minOutFloor, `rule 10: the order promises at least ${minOut} lamports, under our floor of ${w.minOutFloor}`);
  need(band(Number(out.deadline), w.nowS), `rule 11: the output's deadline ${out.deadline} is outside [now + 1 day, now + 8 days]`);
  return { orderId: id, minOut, deadline: Number(out.deadline) };
}

/** An ask as the canister hands it out (`ConvAsk`), plain. */
export type AnyAsk = { amountWei: bigint; user: string; recipient: string; refundTo: string; originChainId: number; destinationChainId: number;
  originCurrency: string; destinationCurrency: string };

export type SolLeg = "sol-btc" | "sol-eth" | "eth-sol" | "sol-rh" | "rh-sol";

/** Which Solana leg an ask is, or null for an EVM-only one. Audit C3-03: Robinhood Chain's NFT side's two legs (SOL → 4663 for its
 * advance, 4663 → SOL for its way back), as the canister asks them (`ethflow::ask_for`, `relayeth::ROBINHOOD_TO_SOLANA`). */
export function solLegOf(a: Pick<AnyAsk, "originChainId" | "destinationChainId">): SolLeg | null {
  if (a.originChainId === RELAY_SOLANA && a.destinationChainId === 8253038) return "sol-btc";
  if (a.originChainId === RELAY_SOLANA && a.destinationChainId === ETHEREUM) return "sol-eth";
  if (a.originChainId === ETHEREUM && a.destinationChainId === RELAY_SOLANA) return "eth-sol";
  if (a.originChainId === RELAY_SOLANA && a.destinationChainId === ROBINHOOD) return "sol-rh";
  if (a.originChainId === ROBINHOOD && a.destinationChainId === RELAY_SOLANA) return "rh-sol";
  return null;
}

/** Pre-judge a quote for a Solana leg's ask, from the ask alone (and S3 when read). Throws the refusal. */
export function preJudgeSolAsk(q: AnyObj, a: AnyAsk, nowS: number, s3: S3 | null = null): { orderId: Hex; minOut: bigint; deadline: number } {
  const leg = solLegOf(a);
  need(leg, `no Solana Relay leg from chain ${a.originChainId} to ${a.destinationChainId}`);
  if (leg === "eth-sol" || leg === "rh-sol") {
    need(a.originCurrency.toLowerCase() === ETH_NATIVE && a.destinationCurrency === SOL_NATIVE, "the ask is not native ETH to native SOL");
    return judgeEthToSol(q, { ours: a.user, solRecipient: a.recipient, amountWei: a.amountWei, nowS, origin: a.originChainId });
  }
  need(a.originCurrency === SOL_NATIVE, "the ask is not native SOL");
  // A5-03: the Relay key deposits, the main address takes the refunds; they are never the same key on a canister after the fix
  const dest = leg === "sol-btc" ? { destChain: "bitcoin" as const, destCurrency: BTC_NATIVE }
    : { destChain: NFT_CHAINS[a.destinationChainId], destCurrency: ETH_NATIVE };
  need(sameOn(a.destinationCurrency, dest.destCurrency, dest.destChain), "the ask's destination currency is not the native one");
  return judgeSolOrigin(q, { ourSol: a.refundTo, depositor: a.user, ...dest, destRecipient: a.recipient, lamports: a.amountWei, s3, nowS });
}

/** The body of Relay's quote request for exactly the ask (the currencies as the canister named them). */
export function solQuoteBodyOf(a: AnyAsk): Record<string, unknown> {
  return { user: a.user, originChainId: a.originChainId, destinationChainId: a.destinationChainId, originCurrency: a.originCurrency,
    destinationCurrency: a.destinationCurrency, recipient: a.recipient, tradeType: "EXACT_INPUT", amount: a.amountWei.toString(), refundTo: a.refundTo };
}

/** S3 from a Solana reader: the depository state and the program's ProgramData head; null when no reader (the canister's alone then). */
export async function readS3(get: ((addr: string, slice?: number) => Promise<{ owner: string; data: Uint8Array } | null>) | null): Promise<S3 | null> {
  if (!get) return null;
  const [state, pd] = await Promise.all([get(RELAY_SOL.RELAY_DEPOSITORY), get(RELAY_SOL.PROGRAMDATA, 45)]);
  if (!state) throw new Error("rule S3: Relay's depository state does not exist");
  if (!pd) throw new Error("rule S3: Relay's ProgramData does not exist");
  return s3Reads(state, pd);
}
