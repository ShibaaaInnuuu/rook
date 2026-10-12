import { base58, bech32, bech32m } from "@scure/base";
import { hashStruct, type Hex } from "viem";
import type { Buyer, ConvAskV, ConvRecV } from "./ic.js";
import type { Log } from "./log.js";
import type { S3 } from "./fuel/sol.js";

/**
 * docs/v6/relay-conversion.md §1 steps 3–4 and §3.1. The runner's half of a conversion: the canister says what it wants quoted
 * (`frog_conversions()`, a record whose `ask` is set while it is Funded), the runner asks Relay's public API for exactly that,
 * checks the answer with the canister's own rules (1–9 and 11 of `relayconv::judge`; rule 10, the minimum out against a
 * reference rate, is the canister's alone: the runner holds no rate it trusts) and hands the raw JSON in with `relay_quote`.
 * The pre-judge only saves a wasted call: the canister judges again, on its own reads, and never trusts the runner.
 *
 * The order is bound by its hash: the deposit's id is `hashStruct(Order)` (EIP-712, Relay's types), so recomputing it from
 * `protocol.v2.orderData` binds the deposit to the recipient, the minimum, the deadline and the refunds inspected here.
 */

/** Relay's depository on 4663, pinned (docs/v6/relay-conversion.md §0.1: /chains, every quote, and the code on chain). */
export const RELAY_DEPOSITORY_4663 = "0x4cd00e387622c35bddb9b4c962c136462338bc31";
export const DEPOSIT_NATIVE = "0x49290c1c";              // depositNative(address depositor, bytes32 id)
export const ROBINHOOD = 4663;
export const BITCOIN = 8253038;
export const ETH_NATIVE = "0x0000000000000000000000000000000000000000";
export const BTC_NATIVE = "bc1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqmql8k8";
export const RELAY_API_DEFAULT = "https://api.relay.link";
/** Relay's chain id for Solana (relaysol.rs RELAY_SOLANA): an ask with this on either side is a Solana leg. */
export const RELAY_SOLANA_CHAIN = 792_703_809;
export const isSolLeg = (a: { originChainId: number; destinationChainId: number }) => a.originChainId === RELAY_SOLANA_CHAIN || a.destinationChainId === RELAY_SOLANA_CHAIN;
/** The Solana legs' pre-judge (src/relaysol.ts `preJudgeSolAsk`), handed in by the runner: relay.ts and ethnft.ts do not load it themselves
 * (the fueler's image loads them and not the Solana judge). */
export type SolJudge = (q: Record<string, any>, a: { amountWei: bigint; user: string; recipient: string; refundTo: string; originChainId: number; // eslint-disable-line @typescript-eslint/no-explicit-any
  destinationChainId: number; originCurrency: string; destinationCurrency: string }, nowS: number, s3: S3 | null) => { orderId: Hex; minOut: bigint };
const DAY = 86_400;

/** Relay's EIP-712 order types (@relay-protocol/settlement-sdk 0.0.143, `ORDER_EIP712_TYPES`). */
export const ORDER_TYPES = {
  Order: [
    { name: "version", type: "string" }, { name: "solverChainId", type: "string" }, { name: "solver", type: "address" },
    { name: "salt", type: "uint256" }, { name: "inputs", type: "Input[]" }, { name: "output", type: "Output" }, { name: "fees", type: "Fee[]" },
  ],
  Input: [{ name: "payment", type: "InputPayment" }, { name: "refunds", type: "InputRefund[]" }],
  InputPayment: [{ name: "chainId", type: "string" }, { name: "currency", type: "bytes" }, { name: "amount", type: "uint256" }, { name: "weight", type: "uint256" }],
  InputRefund: [
    { name: "chainId", type: "string" }, { name: "recipient", type: "bytes" }, { name: "currency", type: "bytes" },
    { name: "minimumAmount", type: "uint256" }, { name: "deadline", type: "uint32" }, { name: "extraData", type: "bytes" },
  ],
  Output: [
    { name: "chainId", type: "string" }, { name: "payments", type: "OutputPayment[]" }, { name: "deadline", type: "uint32" },
    { name: "calls", type: "bytes[]" }, { name: "extraData", type: "bytes" },
  ],
  OutputPayment: [{ name: "recipient", type: "bytes" }, { name: "currency", type: "bytes" }, { name: "minimumAmount", type: "uint256" }, { name: "expectedAmount", type: "uint256" }],
  Fee: [
    { name: "recipientChainId", type: "string" }, { name: "recipient", type: "bytes" }, { name: "currencyChainId", type: "string" },
    { name: "currency", type: "bytes" }, { name: "amount", type: "uint256" },
  ],
} as const;

/** The only chains an order's addresses may name (rule 6): anything else is refused, never guessed. */
const VM: Record<string, "evm" | "bitcoin"> = { robinhood: "evm", ethereum: "evm", bitcoin: "bitcoin" };   // ethereum: docs/v7/phase1-nft.md

const toHex = (b: Uint8Array): Hex => `0x${Buffer.from(b).toString("hex")}`;

/** An address as Relay's settlement SDK encodes it into an order (`encodeAddress`). */
export function encodeAddress(address: string, chain: string): Hex {
  const vm = VM[chain];
  if (!vm) throw new Error(`an address on chain "${chain}": only robinhood, ethereum and bitcoin are accepted`);
  if (vm === "evm") {
    if (!/^0x[0-9a-fA-F]{40}$/.test(address)) throw new Error(`${address} is not a 20-byte EVM address`);
    return address.toLowerCase() as Hex;
  }
  if (address.startsWith("1") || address.startsWith("3")) {
    const d = base58.decode(address);
    if (d.length < 5) throw new Error(`${address} is not a Base58 address`);
    return toHex(Uint8Array.from([0xff, ...d.slice(0, -4)]));
  }
  const lower = address.toLowerCase();
  if (!lower.startsWith("bc1")) throw new Error(`${address} is not a mainnet Bitcoin address`);
  for (const [coder, v0] of [[bech32, true], [bech32m, false]] as const) {
    try {
      const { prefix, words } = coder.decode(lower as `${string}1${string}`, 90);
      if (prefix !== "bc" || (v0 ? words[0] !== 0 : words[0] < 1)) continue;
      return toHex(Uint8Array.from([words[0], ...bech32.fromWords(words.slice(1))]));
    } catch { /* the other encoding */ }
  }
  throw new Error(`${address} is not a bech32/bech32m Bitcoin address`);
}

type AnyObj = Record<string, any>; // eslint-disable-line @typescript-eslint/no-explicit-any

/** `hashStruct(normalize(orderData))`: the order id a deposit must carry. */
export function orderIdOf(o: AnyObj): Hex {
  const data = {
    version: o.version, solverChainId: o.solverChainId, solver: o.solver, salt: BigInt(o.salt),
    inputs: o.inputs.map((i: AnyObj) => ({
      payment: { chainId: i.payment.chainId, currency: encodeAddress(i.payment.currency, i.payment.chainId), amount: BigInt(i.payment.amount), weight: BigInt(i.payment.weight) },
      refunds: i.refunds.map((r: AnyObj) => ({ chainId: r.chainId, recipient: encodeAddress(r.recipient, r.chainId), currency: encodeAddress(r.currency, r.chainId),
        minimumAmount: BigInt(r.minimumAmount), deadline: Number(r.deadline), extraData: r.extraData as Hex })),
    })),
    output: {
      chainId: o.output.chainId,
      payments: o.output.payments.map((p: AnyObj) => ({ recipient: encodeAddress(p.recipient, o.output.chainId), currency: encodeAddress(p.currency, o.output.chainId),
        minimumAmount: BigInt(p.minimumAmount), expectedAmount: BigInt(p.expectedAmount) })),
      deadline: Number(o.output.deadline), calls: o.output.calls as Hex[], extraData: o.output.extraData as Hex,
    },
    fees: o.fees.map((f: AnyObj) => ({ recipientChainId: f.recipientChainId, recipient: encodeAddress(f.recipient, f.recipientChainId), currencyChainId: f.currencyChainId,
      currency: encodeAddress(f.currency, f.currencyChainId), amount: BigInt(f.amount) })),
  };
  return hashStruct({ types: ORDER_TYPES, primaryType: "Order", data } as never);
}

/** What the canister asked for, as plain strings. */
export type Ask = { frog: bigint; amountWei: bigint; user: string; recipient: string; refundTo: string; originChainId: number; destinationChainId: number;
  originCurrency: string; destinationCurrency: string };

export function askOf(c: ConvRecV): Ask | null {
  const a: ConvAskV | undefined = c.ask[0];
  if (!a) return null;
  return { frog: c.frog, amountWei: a.amount_wei, user: a.user, recipient: a.recipient, refundTo: a.refund_to, originChainId: Number(a.origin_chain_id),
    destinationChainId: Number(a.destination_chain_id), originCurrency: a.origin_currency, destinationCurrency: a.destination_currency };
}

/** The body of `POST /quote` for exactly the ask. */
export function quoteBody(a: Ask): Record<string, unknown> {
  return { user: a.user, originChainId: a.originChainId, destinationChainId: a.destinationChainId, originCurrency: a.originCurrency,
    destinationCurrency: a.destinationCurrency, recipient: a.recipient, tradeType: "EXACT_INPUT", amount: a.amountWei.toString(), refundTo: a.refundTo };
}

const same = (a: string, chain: string, b: string): boolean => { try { return encodeAddress(a, chain) === encodeAddress(b, chain); } catch { return false; } };
const big = (x: unknown, what: string): bigint => { try { return BigInt(String(x)); } catch { throw new Error(`${what} is not a whole number`); } };

/**
 * Rules 1–9 and 11 of `relayconv::judge` (docs/v6/relay-conversion.md §3.1). Pure. Returns the order id, or throws the first
 * refusal, naming the field. `nowS`: seconds.
 */
export function preJudge(q: AnyObj, a: Ask, nowS: number): { orderId: Hex; minSats: bigint; deadline: number } {
  const need = (c: boolean, why: string) => { if (!c) throw new Error(why); };
  const steps = q?.steps;
  need(Array.isArray(steps) && steps.length === 1, `${Array.isArray(steps) ? steps.length : 0} steps: only a single deposit step is signed`);
  const step = steps[0];
  need(step.id === "deposit" && step.kind === "transaction", "the one step is not a deposit transaction");
  need(Array.isArray(step.items) && step.items.length === 1, "exactly one transaction is signed");
  const d = step.items[0]?.data ?? {};
  need(Number(d.chainId) === ROBINHOOD && a.originChainId === ROBINHOOD, "the deposit is for another chain");
  need(String(d.to ?? "").toLowerCase() === RELAY_DEPOSITORY_4663, `the deposit goes to ${d.to}, not Relay's pinned depository`);
  const p = q?.protocol?.v2;
  need(!!p?.orderData, "the quote carries no protocol.v2.orderData: nothing binds the deposit to a recipient");
  need(String(p.paymentDetails?.depository ?? "").toLowerCase() === RELAY_DEPOSITORY_4663, "paymentDetails names another depository");
  if (d.from !== undefined) need(same(d.from, "robinhood", a.user), "the quote was made for another sender");
  const value = big(d.value, "value");
  need(value === a.amountWei, `the deposit sends ${value} wei, not ${a.amountWei}`);
  need(big(p.paymentDetails?.amount, "paymentDetails.amount") === value, "paymentDetails.amount is not the value");
  const data = String(d.data ?? "").toLowerCase();
  need(data.length === 2 + 68 * 2 && data.startsWith(DEPOSIT_NATIVE), "the calldata is not depositNative(address, bytes32)");
  need(/^0{24}$/.test(data.slice(10, 34)) && `0x${data.slice(34, 74)}` === a.user.toLowerCase(), "the calldata names another depositor");
  const id = `0x${data.slice(74)}` as Hex;
  const o = p.orderData;
  let computed: Hex;
  try { computed = orderIdOf(o); } catch (e) { throw new Error(`the order cannot be hashed: ${(e as Error).message}`); }
  need(computed === id && String(p.orderId).toLowerCase() === id, `the order hashes to ${computed}, the calldata's id is ${id}, the quote says ${p.orderId}`);
  need(Array.isArray(o.inputs) && o.inputs.length === 1, "exactly one input");
  const pay = o.inputs[0].payment;
  need(pay.chainId === "robinhood" && same(pay.currency, "robinhood", ETH_NATIVE) && big(pay.amount, "payment.amount") === value, "the input is not our native ETH on Robinhood Chain for the value");
  const refunds = o.inputs[0].refunds;
  need(Array.isArray(refunds) && refunds.length > 0, "no refund");
  for (const r of refunds) {
    const ours = r.chainId === "robinhood" ? same(r.recipient, "robinhood", a.refundTo) && same(r.currency, "robinhood", ETH_NATIVE)
      : r.chainId === "bitcoin" ? same(r.recipient, "bitcoin", a.recipient) && same(r.currency, "bitcoin", BTC_NATIVE) : false;
    need(ours, `a refund on ${r.chainId} goes to ${r.recipient}, not ours`);
  }
  need(refunds.some((r: AnyObj) => r.chainId === "robinhood"), "no refund on Robinhood Chain");
  const out = o.output;
  need(out.chainId === "bitcoin" && a.destinationChainId === BITCOIN, "the output is not on Bitcoin");
  need(Array.isArray(out.payments) && out.payments.length === 1, "exactly one output payment");
  const op = out.payments[0];
  need(same(op.recipient, "bitcoin", a.recipient), `the output pays ${op.recipient}, not our float`);
  need(same(op.currency, "bitcoin", BTC_NATIVE), "the output is not native BTC");
  need(Array.isArray(out.calls) && out.calls.length === 0, "the order makes calls");
  need(Array.isArray(o.fees) && o.fees.length === 0, "the order carries fees");
  const minSats = big(op.minimumAmount, "minimumAmount");
  need(minSats > 0n, "no minimum out");
  const band = (t: number) => t >= nowS + DAY && t <= nowS + 8 * DAY;
  const deadline = Number(out.deadline);
  need(band(deadline) && refunds.every((r: AnyObj) => band(Number(r.deadline))), `the order's deadline ${deadline} is not between a day and eight days away`);
  return { orderId: id, minSats, deadline };
}

export type RelayDeps = { buyer: Pick<Buyer, "frog_conversions" | "relay_quote">; fetch: typeof fetch; log: Log; api?: string; nowS?: () => number;
  /** docs/v9 §5: the quote path (`/quote` on the EVM home; `/quote/v2` on the Solana home, as the spike Q5 measured) and Relay's API key
   * (RELAY_API_KEY, optional while /quote/v2 is keyless; read at call time, sent only to the Relay API, scrubbed from logs) */
  quotePath?: string; apiKey?: () => string | undefined;
  /** the Solana home: its pre-judge (relaysol.ts), and rule S3's reads (the depository's allocator, upgrade authority, deploy slot), or null when not read */
  solJudge?: SolJudge;
  s3?: () => Promise<S3 | null>;
};

/** The headers of a Relay quote request (the key only when set). */
export const relayHeaders = (key: string | undefined): Record<string, string> => ({ "content-type": "application/json", ...(key ? { "x-api-key": key } : {}) });

/**
 * One pass: every conversion that asks for a quote gets one, pre-judged, handed in. True when at least one was accepted for judging.
 * The ask names its leg: 4663 → BTC is judged by `preJudge` (relayconv rules), SOL → BTC on the Solana home by relaysol.ts (S1–S12).
 */
export async function convertOnce(d: RelayDeps): Promise<boolean> {
  const api = (d.api ?? RELAY_API_DEFAULT).replace(/\/$/, "");
  const now = d.nowS ?? (() => Math.floor(Date.now() / 1000));
  let submitted = false;
  let convs: ConvRecV[];
  try { convs = await d.buyer.frog_conversions([]); } catch (err) { d.log("warn", "relay-unread", { error: String((err as Error).message).slice(0, 200) }); return false; }
  for (const c of convs) {
    const a = askOf(c);
    if (!a) continue;
    const sol = isSolLeg(a);
    let text: string;
    try {
      const res = await d.fetch(`${api}${d.quotePath ?? "/quote"}`, { method: "POST", headers: relayHeaders(d.apiKey?.()), body: JSON.stringify(quoteBody(a)),
        redirect: "manual", signal: AbortSignal.timeout(20_000) });
      text = await res.text();
      if (!res.ok) { d.log("warn", "relay-quote-refused", { frog: a.frog, status: res.status, body: text.slice(0, 300) }); continue; }
    } catch (err) { d.log("warn", "relay-quote-failed", { frog: a.frog, error: String((err as Error).message).slice(0, 200) }); continue; }
    let judged: { orderId: Hex; minOut: bigint };
    try {
      if (sol) {
        if (!d.solJudge) throw new Error("a Solana leg, and this runner has no Solana judge (ROOK_HOME=robinhood?)");
        judged = d.solJudge(JSON.parse(text), a, now(), d.s3 ? await d.s3() : null);
      }
      else { const j = preJudge(JSON.parse(text), a, now()); judged = { orderId: j.orderId, minOut: j.minSats }; }
    } catch (err) { d.log("warn", "relay-quote-rejected", { frog: a.frog, why: (err as Error).message }); continue; }
    try {
      const r = await d.buyer.relay_quote(a.frog, text);
      if ("Ok" in r) { submitted = true; d.log("info", "relay-quote-handed-in", { frog: a.frog, orderId: judged.orderId, minOut: judged.minOut, leg: sol ? "sol-btc" : "evm-btc", answer: r.Ok }); }
      else d.log("warn", "relay-quote-not-taken", { frog: a.frog, error: r.Err });
    } catch (err) { d.log("warn", "relay-quote-call-failed", { frog: a.frog, error: String((err as Error).message).slice(0, 200) }); }
  }
  return submitted;
}
