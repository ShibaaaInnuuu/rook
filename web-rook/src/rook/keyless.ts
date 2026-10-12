import { concat, encodeAbiParameters, keccak256, type Hex } from "viem";
import type { RaffleDraw, Ticket, WrappedCoin } from "../api/types";
import { when } from "../lib/format";

/**
 * docs/audit/2026-09-26/fixes-W2-site.md §1: the keyless steps anyone may take, and when the contracts accept each. Pure, so
 * test/keyless.test.ts is their spec; the pages only draw what these say. Times are the chain's clock: each contract also waits
 * the same span on the parent chain's blocks, so a step may revert `TooEarly` for a few minutes past the time shown ("from about").
 */

// ---- drand evmnet (src/rand/DrandEvmnet.sol) -----------------------------------------------------------------------------------
export const DRAND_CHAIN = "04f1e9062b8a81f848fded9c12306733282b2727ecced50032187751166ec8c3";
/** drand's public relays for evmnet, asked in turn. The beacon verifies the signature, so a bad relay can only make the call revert. */
export const DRAND_RELAYS = ["https://api.drand.sh", "https://api2.drand.sh", "https://api3.drand.sh"] as const;
export const DRAND_GENESIS = 1_727_521_075, DRAND_PERIOD = 3;
/** When `round` is published (`DrandEvmnet.timeOf`). */
export const roundTime = (round: number): number => DRAND_GENESIS + (round - 1) * DRAND_PERIOD;

/** The 64-byte evmnet signature of `round`, from the first relay that gives one of the right shape; throws with every relay's answer. */
export async function drandSignature(round: number, fetchImpl: typeof fetch = fetch): Promise<Hex> {
  const why: string[] = [];
  for (const relay of DRAND_RELAYS) {
    try {
      const r = await fetchImpl(`${relay}/${DRAND_CHAIN}/public/${round}`, { headers: { accept: "application/json" }, signal: AbortSignal.timeout(10_000) });
      if (!r.ok) { why.push(`${new URL(relay).host}: HTTP ${r.status}`); continue; }
      const j = (await r.json()) as { round?: unknown; signature?: unknown };
      const sig = String(j.signature ?? "").replace(/^0x/, "");
      if (Number(j.round) !== round || !/^[0-9a-fA-F]{128}$/.test(sig)) { why.push(`${new URL(relay).host}: not round ${round}'s 64-byte signature`); continue; }
      return `0x${sig.toLowerCase()}` as Hex;
    } catch (e) { why.push(`${new URL(relay).host}: ${String((e as Error)?.name ?? e)}`); }
  }
  throw new Error(`drand round ${round}'s signature could not be read (${why.join("; ")})`);
}

// ---- redemption tickets (Wrapper: E-03, canister M-1, R3 §4) ---------------------------------------------------------------
export const PIN_TIMEOUT_S = 7 * 86_400, REFUND_GRACE_S = 72 * 3_600, LATE_DELIVERY_WINDOW_S = 30 * 86_400;
type Step = { ok: boolean; at: number | null; why: string | null };

/** fixes-R3-contracts §4: a resolved ticket refunds at deliverBy + 72 h once custody attested it undeliverable, else deliverBy + 30 days. */
export function resolvedRefundAt(deliverBy: number | null, undeliverable: boolean | undefined): number | null {
  return deliverBy === null ? null : deliverBy + (undeliverable ? REFUND_GRACE_S : LATE_DELIVERY_WINDOW_S);
}
/** Why a resolved ticket is not refundable yet, in words: the honest non-delivery path and the dead-custodian path. */
export function resolvedRefundWhy(t: Pick<Ticket, "deliverBy" | "undeliverable">): string {
  if (t.undeliverable) return "The vault's custodian attested that this delivery cannot confirm: the ticket is refundable 72 hours after its delivery deadline.";
  const early = t.deliverBy === null ? "" : ` (from about ${when(t.deliverBy + REFUND_GRACE_S)})`;
  return `An undelivered ticket is refundable once the vault's custodian attests that its delivery can no longer confirm, 72 hours after the delivery deadline${early}; without that word, 30 days after the deadline. Until then a late delivery is still recorded, so no refund races it.`;
}

/** Resolve (anyone, strictly in ticket order, once its round exists) and Refund (anyone: pinned + 7 days; resolved + deliverBy + 72 h
 *  once attested undeliverable, else + 30 days). */
export function ticketSteps(t: Pick<Ticket, "id" | "stage" | "round" | "burnedAt" | "deliverBy" | "refundableAt" | "undeliverable">, now: number, nextToResolve: number | null): { resolve: Step | null; refund: Step | null } {
  const refundAt = t.refundableAt ?? (t.stage === "pinned" ? (t.burnedAt === null ? null : t.burnedAt + PIN_TIMEOUT_S) : t.stage === "resolved" ? resolvedRefundAt(t.deliverBy, t.undeliverable) : null);
  const refund: Step | null = t.stage === "pinned" || t.stage === "resolved"
    ? { ok: refundAt !== null && now >= refundAt, at: refundAt,
        why: refundAt === null ? "The time it becomes refundable is not known." : now >= refundAt ? null
          : t.stage === "pinned" ? "A ticket nobody resolves is refundable 7 days after its pin." : resolvedRefundWhy(t) }
    : null;
  if (t.stage !== "pinned") return { resolve: null, refund };
  const at = roundTime(t.round);
  const resolve: Step = now < at ? { ok: false, at, why: `drand round ${t.round} is not published yet.` }
    : nextToResolve !== null && nextToResolve !== t.id ? { ok: false, at, why: `Tickets resolve in order: ticket ${nextToResolve} is next.` }
    : { ok: true, at, why: null };
  return { resolve, refund };
}

/** One line for a ticket's stage, and when it is due a refund. There is no redraw: the pinned round is final (C-02). */
export function ticketLine(t: Ticket, now: number): string {
  if (t.stage === "delivered") return `delivered${t.deliveryTx ? ` (${t.deliveryTx.slice(0, 10)}…)` : ""}`;
  if (t.stage === "refunded") return t.refundedUnresolved ? "refunded unresolved: nobody resolved it within 7 days of its pin, no NFT was drawn, one whole wrapped token went back to its owner" : "refunded: one whole wrapped token went back to its owner";
  const { refund } = ticketSteps(t, now, null);
  const due = refund?.ok ? " — refundable by anyone now" : refund?.at ? ` — refundable from about ${when(refund.at)} if still ${t.stage === "pinned" ? "unresolved" : "undelivered"}` : "";
  if (t.stage === "resolved") {
    const late = t.deliverBy !== null && now > t.deliverBy;
    const attested = t.undeliverable ? " — the vault's custodian attested it cannot be delivered" : "";
    return `drew ${t.itemRef ? `${t.itemRef.slice(0, 10)}…` : "an NFT"}, deliver by ${t.deliverBy ? when(t.deliverBy) : "?"}${late ? " — overdue" : ""}${attested}${due}`;
  }
  return `waiting for drand round ${t.round} to be proven (anyone can)${due}`;
}

// ---- raffles (SweepVault: C-02..C-04, E-02) --------------------------------------------------------------------------------------
/** R3 N-2: a draw that carries an earlier draw's seed, or leaves its own to the coin's next draw, said plainly; null otherwise. The
 *  snapshot time shown is only ever the chain's (`SnapshotFixed`), never computed from an announcement. */
export function seedLine(d: Pick<RaffleDraw, "seedOrigin" | "seedPassedOn" | "stage">): string | null {
  const parts: string[] = [];
  if (d.seedOrigin !== undefined && d.seedOrigin !== null)
    parts.push(`It reuses draw ${d.seedOrigin}'s seed round and snapshot time: a draw voided or cancelled before its pin cannot be re-rolled.`);
  if (d.seedPassedOn && (d.stage === "voided" || d.stage === "cancelled"))
    parts.push("It ended before its pin, so the coin's next draw reuses its seed round and snapshot window (the same snapshot time).");
  return parts.length ? parts.join(" ") : null;
}
/** fixes-R3-carry: a draw over a snapshot with no eligible holder, said plainly (open: what will happen; empty: what did); null otherwise. */
export const NO_WINNER = "No eligible holders at the snapshot — no winner this round. The wrapped token stays with the coin for its next draw.";
export function emptyLine(d: Pick<RaffleDraw, "stage" | "totalTickets">): string | null {
  if (d.stage === "empty") return NO_WINNER;
  if (d.stage === "open" && d.totalTickets === "0")
    return "Nobody eligible held a whole coin at the snapshot time, says the keeper's list (anyone can rebuild it). After the 15-minute window it closes with no winner and the wrapped token stays with the coin; the Safe or the guardian cancels it first if the list is wrong.";
  return null;
}
export const OPEN_TTL_S = 6 * 3_600, VOID_AFTER_S = 7 * 86_400;
export type DrawStep = { id: "pin" | "close" | "resolve" | "claim" | "void"; ok: boolean; at: number | null; why: string | null };

/** The keyless steps a draw offers now: pin a round after the 15-minute window (not while the keeper is paused), resolve once the
 *  pinned round exists, claim for the winner once resolved (with a proof from a list that reproduces the root), void when due. */
export function drawSteps(d: RaffleDraw, now: number, keeperPaused: boolean | null): DrawStep[] {
  const out: DrawStep[] = [];
  if (d.stage === "open" && d.challengeEnds !== null) {
    // fixes-R3-carry: the same requestDraw closes a draw with no tickets (no winner) instead of pinning a round
    const id = d.totalTickets === "0" ? "close" : "pin";
    out.push(now < d.challengeEnds ? { id, ok: false, at: d.challengeEnds, why: "The 15-minute window for cancelling a bad list has not passed." }
      : keeperPaused ? { id, ok: false, at: null, why: `The keeper is paused: no draw is ${id === "close" ? "closed" : "pinned"} until the Safe unpauses it.` }
      : { id, ok: true, at: d.challengeEnds, why: null });
  }
  if (d.stage === "pinned" && d.round !== null) {
    const at = roundTime(d.round);
    out.push(now < at ? { id: "resolve", ok: false, at, why: `drand round ${d.round} is not published yet.` } : { id: "resolve", ok: true, at, why: null });
  }
  if (d.stage === "resolved") {
    out.push(d.claim ? { id: "claim", ok: true, at: null, why: null }
      : { id: "claim", ok: false, at: null, why: d.published?.status === "differs" ? "The published list does not reproduce the vault's root, so no proof is offered." : "No proof for the winning ticket is known here yet." });
  }
  if (d.voidableAt !== null) out.push(now >= d.voidableAt ? { id: "void", ok: true, at: d.voidableAt, why: null } : { id: "void", ok: false, at: d.voidableAt, why: null });
  return out;
}

/** `SweepVault.leaf` and what `MerkleProof.verify` computes, so the page can check a claim against the on-chain root before asking. */
export const ticketLeaf = (holder: string, first: bigint, end: bigint): Hex =>
  keccak256(keccak256(encodeAbiParameters([{ type: "address" }, { type: "uint256" }, { type: "uint256" }], [holder as Hex, first, end])));
const pairHash = (a: Hex, b: Hex): Hex => (BigInt(a) < BigInt(b) ? keccak256(concat([a, b])) : keccak256(concat([b, a])));
export const verifyProof = (proof: readonly Hex[], root: Hex, leaf: Hex): boolean => proof.reduce<Hex>((h, p) => pairHash(h, p), leaf).toLowerCase() === root.toLowerCase();

/** The claim, checked against the draw as the chain holds it (`SweepVault.draw(id)`): resolved, the range holds the winning ticket, the proof verifies. */
export function claimRefusal(onChain: { stage: number; root: Hex; totalTickets: bigint; winningTicket: bigint }, c: NonNullable<RaffleDraw["claim"]>): string | null {
  if (onChain.stage !== 3) return "The draw is not resolved on chain (or was claimed already); nothing was sent.";
  const first = BigInt(c.first), end = BigInt(c.end);
  if (!(first <= onChain.winningTicket && onChain.winningTicket < end && end <= onChain.totalTickets)) return "The claim's range does not hold the winning ticket; nothing was sent.";
  if (!verifyProof(c.proof, onChain.root, ticketLeaf(c.holder, first, end))) return "The claim's proof does not verify against the vault's root; nothing was sent.";
  return null;
}

// ---- the curve's gates (A-03 close gate, A-01 escape hatch) ----------------------------------------------------------------------
/** A-03: a buy whose route closes the curve is refused while the queue has no room (`canClose()` false); sells and smaller buys are not. */
export const CLOSE_REFUSED = "The closing buy is unavailable: the floor module has no room for this coin's NFT budget right now (or the keeper is paused). You can still sell back to the curve at its own price, and the close goes through when room appears.";
export function closeBlocked(coin: Pick<WrappedCoin, "phase" | "sweepOnly" | "canClose">, side: "buy" | "sell", closes: boolean): boolean {
  return coin.phase === "trading" && side === "buy" && closes && !coin.sweepOnly && coin.canClose === false;
}
/** A-01: a Graduating curve can be graduated by anyone when `canGraduate()`; failing that, refunds open at `refundsOpenAt()`. */
/** R3 N-3: while the queue holds refunds (the keeper paused) an unfilled curve's refunds wait up to 4 more days; `refundsOpenAt()`
 *  already reports the held time, so `refundsAt` is the chain's and `held` only says why it moved. */
export function graduatingSteps(coin: Pick<WrappedCoin, "phase" | "canGraduate" | "refundsOpenAt" | "refundsHeld">, now: number): { graduate: boolean; refundsAt: number | null; openRefunds: boolean; held: boolean } {
  if (coin.phase !== "graduating") return { graduate: false, refundsAt: null, openRefunds: false, held: false };
  const at = coin.refundsOpenAt ?? null;
  return { graduate: coin.canGraduate === true, refundsAt: at, openRefunds: at !== null && now >= at, held: coin.refundsHeld === true };
}
export const REFUNDS_HELD = "The keeper is paused, so the floor module cannot fill this graduation: its refunds are held up to 4 more days for the pause to be lifted, never longer.";
/** A-03 launch gate: a launch that buys items is refused while `launchesOpen()` is false; SWEEP_ONLY launches are not. */
export const LAUNCHES_PAUSED = "Launches paused — the floor module is full (stage 0).";
export const launchBlocked = (launchesOpen: boolean | null | undefined, sweepOnly: boolean): boolean => launchesOpen === false && !sweepOnly;
/** The default deadline for a curve or router trade, in minutes (the ticket offers 10, 20 and 60). */
export const DEFAULT_DEADLINE_MIN = 20;
export const deadlineOf = (nowS: number, minutes: number): bigint => BigInt(Math.floor(nowS) + minutes * 60);
