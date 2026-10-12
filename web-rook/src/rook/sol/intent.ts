/**
 * The intent desk's rules as the site states them (docs/v9/canister-only-solana.md §6.1–§6.2; the canister's
 * autobuyer/canister-rs/src/desk/intent.rs holds the numbers): a user opens an intent with the connected wallet as recipient, gets its
 * own deposit address `A = K_int[n]`, sends SOL (a mint) or wX / coins (a burn, a redemption) to it, and the canister acts on the
 * finalized deposit or returns it. Pure: test/intent.test.ts is its spec. The page only draws what these say.
 */
import { createAtaIdempotent, transferLamports } from "./dbc";
import { units } from "./format";
import { TOKEN_PROGRAM, type Address } from "./keys";
import { ata } from "./pda";
import { Writer } from "./borsh";
import type { Instruction } from "./tx";
import { stateOf, type IntentState, type IntentViewV, type OpenKindV, type OpenedV, type QuoteV } from "./canister";

/** One whole wX, base units (9 decimals). */
export const ONE_WX = 1_000_000_000n;
/** The fee per operation: 0.0015 SOL (decision O-8). It pays the intent's extra signature, the Solana fee and a priority fee. */
export const FEE_LAMPORTS = 1_500_000n;
/** The smallest SOL deposit a mint acts on (0.05 SOL); below it the deposit waits, and is swept or returned at expiry. */
export const MIN_MINT_LAMPORTS = 50_000_000n;
/** The smallest wX deposit a burn acts on (0.01 wX). */
export const MIN_BURN_UNITS = ONE_WX / 100n;
/** A redemption takes exactly one whole wX per intent. */
export const MIN_REDEEM_UNITS = ONE_WX;
/** A returned deposit must cover the fee and a new account's rent to be worth a transaction: 0.0025 SOL, or 0.001 wX. */
export const DUST_REFUND_LAMPORTS = 2_500_000n;
export const DUST_REFUND_UNITS = ONE_WX / 1_000n;
/** 24 h of no activity closes an intent. */
export const TTL_SECS = 86_400;
/** A refusal that may pass (a stale floor, a paused poster, an hourly bucket) is waited out this long, then the deposit goes back. */
export const RETRY_WINDOW_SECS = 6 * 3_600;
/** After expiry the address is still watched this long, and a deposit found there is returned. */
export const REFUND_GRACE_SECS = 7 * 86_400;
/** The limits: open intents that saw a deposit, in all; openings never funded; per caller principal per hour; open per recipient. */
export const MAX_OPEN = 300, MAX_UNFUNDED_OPEN = 200, MAX_PER_CALLER_HOUR = 5, MAX_PER_RECIPIENT = 3;
/** The site's bound on a mint's price: at least this share of the quoted wX, or the SOL comes back less the fee (§6.1 `max_price`). */
export const MIN_OUT_BPS = 9_500n;

export type IntentKind = "mint" | "burn" | "redeem_nft" | "redeem_coin";
export const KIND_WORD: Record<IntentKind, string> = { mint: "Mint", burn: "Burn", redeem_nft: "Redeem an NFT", redeem_coin: "Redeem a coin" };

/** What the user sends to the intent's address, and the least that is acted on. */
export function depositOf(kind: IntentKind, wx: string, coinSymbol = "coins"): { asset: string; min: bigint; minText: string; decimals: number } {
  switch (kind) {
    case "mint": return { asset: "SOL", min: MIN_MINT_LAMPORTS, minText: "0.05 SOL", decimals: 9 };
    case "burn": return { asset: wx, min: MIN_BURN_UNITS, minText: `0.01 ${wx}`, decimals: 9 };
    case "redeem_nft": return { asset: wx, min: MIN_REDEEM_UNITS, minText: `exactly 1 ${wx}`, decimals: 9 };
    case "redeem_coin": return { asset: coinSymbol, min: 1n, minText: `any amount of ${coinSymbol}`, decimals: 6 };
  }
}

/** The argument `open_intent` takes, with the connected wallet as recipient (§6.1: the site always asks for the connected wallet). */
export function openKindFor(kind: IntentKind, a: { coll: number; wallet: Address; minOut?: bigint | null; destination?: string; coin?: string }): OpenKindV {
  const minOut: [] | [bigint] = a.minOut && a.minOut > 0n ? [a.minOut] : [];
  switch (kind) {
    case "mint": return { Mint: { coll: a.coll, recipient: a.wallet, min_out: minOut } };
    case "burn": return { Burn: { coll: a.coll, recipient: a.wallet, refund_to: [], min_out: minOut } };
    case "redeem_nft": {
      const d = (a.destination ?? "").trim();
      if (!d) throw new Error("a redemption needs a destination");
      return { RedeemNft: { coll: a.coll, destination: d, owner: a.wallet } };
    }
    case "redeem_coin": {
      if (!a.coin) throw new Error("a coin redemption needs the coin's mint");
      return { RedeemCoin: { coll: a.coll, coin: a.coin, recipient: a.wallet, refund_to: [] } };
    }
  }
}

/** The least wX a mint accepts for a deposit the quote priced: 95 % of the quote's units (the floor may move between the quote and the
 *  tick); none without a quote. A mint under it is refused, and the SOL goes back less the fee. */
export const mintMinOut = (quote: QuoteV | null, deposit: bigint): bigint | null => {
  if (!quote || !("Mint" in quote) || quote.Mint.deposit <= 0n || quote.Mint.units <= 0n) return null;
  return (quote.Mint.units * deposit * MIN_OUT_BPS) / (quote.Mint.deposit * 10_000n);
};

/** SPL Token `Transfer` (instruction 3): `amount` base units from `source` to `dest`, signed by `owner`. */
export const transferTokens = (source: Address, dest: Address, owner: Address, amount: bigint): Instruction =>
  ({ programId: TOKEN_PROGRAM, keys: [{ pubkey: source, isSigner: false, isWritable: true }, { pubkey: dest, isSigner: false, isWritable: true }, { pubkey: owner, isSigner: true, isWritable: false }],
    data: new Writer().u8(3).u64(amount).done() });

/**
 * The deposit the wallet signs. A mint: SOL to `address` (any wallet can; here the system transfer). A burn, a redemption: the token
 * to `token_account` (`address`'s associated token account of the mint), which the transfer creates idempotently (the rent is the
 * sender's; §6.1 B1-04: the canister closes it to `K_main` after the operation, so the rent is not returned).
 */
export function depositInstructions(kind: IntentKind, a: { wallet: Address; address: Address; tokenAccount: Address | null; mint: Address | null; amount: bigint }): Instruction[] {
  if (a.amount <= 0n) throw new Error("nothing to send");
  if (kind === "mint") return [transferLamports(a.wallet, a.address, a.amount)];
  if (!a.tokenAccount || !a.mint) throw new Error("the intent names no token account to send to");
  if (a.tokenAccount !== ata(a.address, a.mint)) throw new Error("the intent's token account is not the deposit address's associated token account of this mint; nothing is sent");
  return [createAtaIdempotent(a.wallet, a.tokenAccount, a.address, a.mint), transferTokens(ata(a.wallet, a.mint), a.tokenAccount, a.wallet, a.amount)];
}

/** What an amount typed for `kind` must satisfy, in words, or null. */
export function amountProblem(kind: IntentKind, amount: bigint | null, balance: bigint | null, wx: string): string | null {
  const d = depositOf(kind, wx);
  if (amount === null) return `Enter an amount of ${d.asset}.`;
  if (kind === "redeem_nft" && amount !== MIN_REDEEM_UNITS) return `A redemption takes exactly 1 ${wx}: one whole token, one NFT.`;
  if (amount < d.min) return `Below the minimum: the canister acts on ${d.minText} or more; less waits at the address and goes back at expiry${kind === "mint" ? " if it covers the fee" : ""}.`;
  if (balance !== null && (kind === "mint" ? amount + 20_000_000n > balance : amount > balance)) return kind === "mint" ? "More than your SOL (keep ≈ 0.02 SOL for fees)." : `More ${d.asset} than you hold.`;
  return null;
}

// ---- status --------------------------------------------------------------------------------------------------------------------------
export type Tone = "wait" | "work" | "good" | "warn" | "bad";
export type StatusCopy = { state: IntentState; headline: string; detail: string; tone: Tone; final: boolean; step: 1 | 2 | 3 | 4 };
const sol = (l: bigint) => `${units(l, 9, 6)} SOL`;
const wxText = (u: bigint, wx: string) => `${units(u, 9, 6)} ${wx}`;
export const depositText = (v: Pick<IntentViewV, "kind">, amount: bigint, wx: string) => (v.kind === "mint" ? sol(amount) : v.kind === "redeem_coin" ? `${amount.toString()} base units` : wxText(amount, wx));

/** The signature that landed the operation (a Done or Refunded intent): the last transaction whose result is "landed". */
export function landedSignature(v: Pick<IntentViewV, "txs">): string | null {
  const landed = [...v.txs].reverse().find((t) => /^landed/.test(t.result) && t.signature);
  return landed?.signature ?? null;
}
/** The last transaction that failed or was given up, in words, or null. */
export function lastFailure(v: Pick<IntentViewV, "txs">): string | null {
  const t = [...v.txs].reverse().find((x) => !/^landed/.test(x.result) && x.result);
  return t ? `${t.op}: ${t.result}` : null;
}

/** Copy for every state and refusal the desk can show, from the view the canister serves. */
export function statusCopy(v: IntentViewV, now: number, wx: string): StatusCopy {
  const state = stateOf(v);
  const expiresIn = Number(v.expires_at) - now;
  const note = v.note.trim();
  const refunding = v.refunding.length ? v.refunding[0] : null;
  const what = v.kind === "mint" ? "mint" : v.kind === "burn" ? "burn" : "redemption";
  if (state === "Open") {
    if (v.deposit > 0n) return { state, headline: "Deposit seen, below the minimum", tone: "warn", final: false, step: 1,
      detail: `${depositText(v, v.deposit, wx)} is at the address, under the minimum of ${depositText(v, v.min_deposit, wx)}: the canister waits for more. At expiry ${v.kind === "mint" ? "it is swept to the treasury if it covers the fee, else left where it is" : "it is returned if it covers a transaction's cost"}.` };
    return { state, headline: "Waiting for your deposit", tone: "wait", final: false, step: 1,
      detail: `Send ${v.kind === "mint" ? "SOL" : v.kind === "redeem_coin" ? "the coins" : wx} to the address below from any wallet; the canister reads it once finalized (≈ 13 s), then acts on it at its next tick (30–120 s). ${expiresIn > 0 ? `The address stays open ${Math.max(1, Math.round(expiresIn / 3_600))} h more.` : "It has expired: a deposit found now is returned."}${note && !/^waiting/.test(note) ? ` (${note})` : ""}` };
  }
  if (state === "Funded") {
    const waits = /^waits:/.test(note) || /refused|stale|paused|bucket|cap/i.test(note);
    return { state, headline: waits ? `Deposit in, ${what} waiting` : "Deposit finalized", tone: waits ? "warn" : "work", final: false, step: 2,
      detail: waits ? `${depositText(v, v.deposit, wx)} is finalized but the ${what} cannot go through right now: ${note.replace(/^waits:\s*/, "")}. The canister retries every tick for 6 hours, then returns the deposit less the fee.`
        : `${depositText(v, v.deposit, wx)} is finalized at the address. The next tick signs the ${what}.${note ? ` (${note})` : ""}` };
  }
  if (state === "Executing") {
    const memo = v.in_flight.length ? v.in_flight[0] : null;
    return { state, headline: refunding ? "Returning your deposit" : `${KIND_WORD[v.kind as IntentKind] ?? "Operation"} signed, landing`, tone: "work", final: false, step: 3,
      detail: refunding ? `The deposit is going back to ${v.refund_to}: ${refunding}. The transaction is out; finalization takes ≈ 13 s.`
        : `The transaction is out and waits for finalization (≈ 13 s); ${memo ? `its memo is ${memo}; ` : ""}the canister then books it. ${lastFailure(v) ? `An earlier attempt did not land (${lastFailure(v)}); this is a retry.` : ""}`.trim() };
  }
  if (state === "Done") {
    const sig = landedSignature(v);
    const got = v.kind === "mint" ? (v.minted_units > 0n ? `${wxText(v.minted_units, wx)} minted to ${v.recipient}` : "the mint landed")
      : v.kind === "burn" ? (v.paid_lamports > 0n ? `${sol(v.paid_lamports)} paid to ${v.recipient}` : "the burn landed")
      : v.kind === "redeem_coin" ? (v.paid_lamports > 0n ? `${wxText(v.paid_lamports, wx)} paid to ${v.recipient}` : "the coins were redeemed")
      : v.ticket.length ? `ticket #${v.ticket[0].toString()} opened: the custodian delivers the NFT to the destination` : "the redemption landed";
    return { state, headline: "Done", tone: "good", final: true, step: 4,
      detail: `${got[0].toUpperCase()}${got.slice(1)}.${sig ? "" : " (No landed signature is recorded on the intent yet.)"} A further deposit to the same address before it expires is a new operation.` };
  }
  if (state === "Refunded") {
    return { state, headline: "Deposit returned", tone: "warn", final: true, step: 4,
      detail: `The deposit went back to ${v.refund_to}, less the fee${refunding ? `: ${refunding}` : note ? `: ${note}` : ""}. Nothing was ${what === "mint" ? "minted" : what === "burn" ? "burned" : "redeemed"}; you can open a new intent.` };
  }
  return { state, headline: "Expired", tone: "bad", final: true, step: 4, detail: "Nothing was deposited within 24 hours, so the address closed. A deposit found there within 7 days is still returned; open a new intent to try again." };
}

/** Why `open_intent` refused, in the site's words (the canister's message is kept). */
export function refusalCopy(err: string): string {
  if (/desk is paused/i.test(err)) return "The desk is paused by the operator: no new intent for now. Open deposits are still returned.";
  if (/paused: no mint/i.test(err)) return "This collection's mint is paused (a guardian or the operator): no mint intent; burns and redemptions stay open.";
  if (/stopped by an incident/i.test(err)) return "This collection is stopped by an incident in the canister (see the Reserves page): no intent until the operator clears it.";
  if (/intents an hour per caller/i.test(err)) return `${MAX_PER_CALLER_HOUR} intents an hour per caller: try again later.`;
  if (/already open for this recipient/i.test(err)) return `${MAX_PER_RECIPIENT} intents are already open for this wallet: fund or let one expire first.`;
  if (/intents are (open|waiting)/i.test(err)) return `The desk is full (${err}). Try again in a few minutes.`;
  if (/coin redemption is not enabled/i.test(err)) return "Coin redemption intents are not enabled on the canister yet.";
  if (/destination/i.test(err)) return `The destination was refused: ${err}`;
  return `The canister refused: ${err}`;
}

/** The desk's steps, for the progress line. */
export const STEPS = ["Open", "Deposit", "Sign", "Landed"] as const;

/** A wallet's intents in this browser (ids only; everything else is read from the canister). */
export const INTENTS_KEY = (wallet: string) => `rook-intents:${wallet}`;
export function rememberedIntents(wallet: string, read: (k: string) => string | null): bigint[] {
  try { const raw = read(INTENTS_KEY(wallet)); return raw ? (JSON.parse(raw) as string[]).map((s) => BigInt(s)) : []; } catch { return []; }
}
export const intentsJson = (ids: bigint[]) => JSON.stringify(ids.slice(-10).map((i) => i.toString()));

/** The opened reply, summarized for the first screen. */
export function openedSummary(o: OpenedV, kind: IntentKind, wx: string): string {
  const q = o.quote.length ? o.quote[0] : null;
  const fee = sol(o.fee);
  if (kind === "mint") return q && "Mint" in q ? `At the minimum deposit of ${sol(q.Mint.min_deposit)} you would get ≈ ${wxText(q.Mint.units, wx)} (${sol(q.Mint.unit_price)} per whole ${wx}); the fee is ${fee}${q.Mint.ata_rent > 0n ? `, plus ${sol(q.Mint.ata_rent)} rent for your ${wx} account the first time` : ""}.` : `No quote right now (the floor is stale or the collection paused): a deposit waits up to 6 h for a good post, then goes back less the ${fee} fee.`;
  if (kind === "burn") return q && "Burn" in q ? `At the minimum of ${wxText(q.Burn.units, wx)} the books pay ${sol(q.Burn.paid)}; ${sol(q.Burn.net)} reaches you after the ${fee} fee.` : `No quote right now: a deposit waits up to 6 h for a good floor post, then goes back less the ${fee} fee.`;
  if (kind === "redeem_nft") return `Send exactly 1 ${wx}. It is burned and a ticket opens; the custodian delivers one NFT to the destination. The fee is ${fee}.`;
  return `Send the coins. At finalization the canister reads the vault and the coin's live supply and pays ⌊vault × coins ÷ supply⌋ ${wx}, rounded down. The fee is ${fee}.`;
}
