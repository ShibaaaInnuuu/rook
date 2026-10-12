/**
 * The one send path: compile → SIMULATE (the wallet is never asked to sign a transaction that fails in simulation) → the wallet
 * signs → (the page's own key, a new coin's mint, signs the message the wallet returned) → send → poll until confirmed.
 * A compute-unit limit from the simulation (+15 %) and a priority fee from the cluster's recent fees are set on every transaction.
 */
import { b58encode } from "./base58";
import { setComputeUnitLimit, setComputeUnitPrice } from "./dbc";
import type { LocalKeypair } from "./pda";
import { confirm, type Account, type Rpc } from "./rpc";
import { compileMessage, decodeMessage, MAX_TX_BYTES, parseTransaction, serializeMessage, serializeTransaction, txSize, unsignedTransaction, type Instruction } from "./tx";

/** DBC / Anchor / the runtime's reason, in a line, from simulation logs. */
export function failureOf(err: unknown, logs: string[]): string {
  const anchor = logs.map((l) => l.match(/Error Message: (.*?)\.?$/)?.[1]).find(Boolean);
  if (anchor) return anchor;
  const custom = logs.map((l) => l.match(/Program log: (?:Error: )?(.*insufficient.*|.*slippage.*|.*exceed.*)/i)?.[1]).find(Boolean);
  if (custom) return custom;
  const e = JSON.stringify(err);
  if (/InsufficientFundsForRent|insufficient lamports|"Custom":1\b/.test(e + logs.join(" "))) return "Not enough SOL in the wallet for this (amount, rent and fees).";
  if (/AccountNotFound/.test(e)) return "The wallet has no SOL on Solana mainnet yet (its account does not exist).";
  return e.slice(0, 200);
}

export type Simulated = { units: number; accounts: (Account | null)[]; logs: string[] };
export async function simulate(rpc: Rpc, payer: string, ixs: Instruction[], watch: string[] = []): Promise<Simulated> {
  const m = compileMessage(payer, [setComputeUnitLimit(1_400_000), ...ixs], "11111111111111111111111111111111");
  if (txSize(m) > MAX_TX_BYTES) throw new Error(`the transaction is ${txSize(m)} bytes, over Solana's ${MAX_TX_BYTES}: shorten the name, symbol or metadata URL, or leave out the first buy`);
  const r = await rpc.simulate(unsignedTransaction(m), watch);
  if (r.err) throw new Error(`Simulation failed: ${failureOf(r.err, r.logs)}`);
  return { units: r.unitsConsumed ?? 200_000, accounts: r.accounts, logs: r.logs };
}

async function priorityFee(rpc: Rpc): Promise<bigint> {
  try {
    const fees = await rpc.call<{ prioritizationFee: number }[]>("getRecentPrioritizationFees", [[]]);
    const xs = fees.map((f) => f.prioritizationFee).sort((a, b) => a - b);
    const p75 = xs.length ? xs[Math.floor(xs.length * 0.75)] : 0;
    return BigInt(Math.min(2_000_000, Math.max(10_000, p75)));
  } catch { return 10_000n; }
}

export type Signer = { address: string; signTransaction: ((tx: Uint8Array) => Promise<Uint8Array>) | null; signAndSend: ((tx: Uint8Array) => Promise<string>) | null };
export type SendStep = "simulating" | "signing" | "sending" | "confirming";

/** Simulate, then sign and send. `extra`: keys the page holds (a new mint) that must sign after the wallet. Returns the signature. */
export async function simulateAndSend(rpc: Rpc, wallet: Signer, ixs: Instruction[], opts: { extra?: LocalKeypair[]; onStep?: (s: SendStep) => void } = {}): Promise<string> {
  const step = opts.onStep ?? (() => {});
  step("simulating");
  const sim = await simulate(rpc, wallet.address, ixs);
  const units = Math.min(1_400_000, Math.ceil(sim.units * 1.15) + 10_000);
  const [{ blockhash, lastValidBlockHeight }, price] = await Promise.all([rpc.latestBlockhash(), priorityFee(rpc)]);
  const full = [setComputeUnitLimit(units), setComputeUnitPrice(price), ...ixs];
  const m = compileMessage(wallet.address, full, blockhash);
  // the transaction the wallet will see, simulated once more as built (blockhash and budget included)
  const again = await rpc.simulate(unsignedTransaction(m), []);
  if (again.err) throw new Error(`Simulation failed: ${failureOf(again.err, again.logs)}`);
  const unsigned = unsignedTransaction(m);
  const extra = opts.extra ?? [];
  step("signing");
  let sig: string;
  if (!extra.length && wallet.signAndSend) {
    sig = await wallet.signAndSend(unsigned);
  } else {
    if (!wallet.signTransaction) throw new Error("This wallet cannot sign a transaction for the page to send. Try Phantom, Solflare or Backpack.");
    const signed = parseTransaction(await wallet.signTransaction(unsigned));
    // the wallet may have changed the message (a priority fee); every extra key signs the message as returned
    const msg = decodeMessage(signed.message);
    const sigs = signed.signatures.slice();
    for (const k of extra) {
      const at = msg.accountKeys.indexOf(k.address);
      if (at < 0 || at >= msg.header.numRequiredSignatures) throw new Error("The wallet returned a transaction without the coin's mint as a signer; nothing was sent.");
      sigs[at] = k.sign(signed.message);
    }
    if (msg.accountKeys[0] !== wallet.address) throw new Error("The wallet returned a transaction paid by another account; nothing was sent.");
    const wire = serializeTransaction(signed.message, sigs);
    step("sending");
    sig = await rpc.send(wire);
    if (sig !== b58encode(sigs[0])) throw new Error(`the RPC answered signature ${sig}, not the wallet's`);
  }
  step("confirming");
  await confirm(rpc, sig, lastValidBlockHeight);
  return sig;
}

/** For tests: the message the send path would ask the wallet to sign (budget instructions first). */
export const messageFor = (payer: string, ixs: Instruction[], blockhash: string, units: number, price: bigint) =>
  serializeMessage(compileMessage(payer, [setComputeUnitLimit(units), setComputeUnitPrice(price), ...ixs], blockhash));
