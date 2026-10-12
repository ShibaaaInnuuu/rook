import { encodeAbiParameters, keccak256 } from "viem";
import { PIN_AHEAD, roundAt, timeOf, verifyBeacon } from "./drand.js";
import type { Address, Check, DrawProof, DrawVerdict, Hex, TimingFlag, VaultItem, VaultState } from "./types.js";

/**
 * docs/v8/redemption-draw.md §2.2–§2.3 and §3.4. EthNftWrapper's redemption machine replayed from its events, the index its
 * `resolve` computes, a draw proof checked field by field, and the flag for the 4663 sequencer's timestamp latitude. Pure.
 *
 * The contract, restated (src/nft/EthNftWrapper.sol `_admit`, `_redeem`, `resolve`, `refund`):
 *   - an item that arrives (a purchase or a deposit) is queued Pending(itemId, stamp = number of tickets so far);
 *   - a refunded resolved ticket's item is queued again with a NEW stamp; a ticket refunded while pinned drew nothing;
 *   - ticket t resolves strictly in order: every queued item with stamp ≤ t joins `_free` (in queue order), then
 *     i = keccak256(abi.encode(randomness, wrapper, t)) mod |free|, free[i] is drawn, the last element takes its place.
 */
export const UNRESOLVED_ITEM = ((1n << 256n) - 1n).toString();
/** The chain's normal batch-posting delay above which a burn's timing is flagged (measured 138 s on 2026-09-27, block 74,121,585). */
export const NORMAL_BATCH_DELAY_S = 600;

export function drawIndex(randomness: Hex, wrapper: Address, ticket: number | bigint, size: number): number {
  if (size <= 0) throw new Error("nothing to draw from");
  const h = keccak256(encodeAbiParameters([{ type: "bytes32" }, { type: "address" }, { type: "uint256" }], [randomness, wrapper, BigInt(ticket)]));
  return Number(BigInt(h) % BigInt(size));
}

/** The wrapper's events, decoded, as the replay needs them. `itemId` of a pinned refund is UNRESOLVED_ITEM. */
export type VaultEvent =
  | { name: "Wrapped" | "Deposited"; itemId: number; tokenId: string; tx: Hex; block: number; logIndex: number; at: number | null }
  | { name: "Redeemed"; ticket: number; owner: Address; round: number; destination: Address; tx: Hex; block: number; logIndex: number; at: number | null }
  | { name: "Resolved"; ticket: number; itemId: number; tokenId: string; tx: Hex; block: number; logIndex: number; at: number | null }
  | { name: "Delivered"; ticket: number; tokenId: string; txHash: Hex; tx: Hex; block: number; logIndex: number; at: number | null }
  | { name: "Refunded"; ticket: number; itemId: string; tx: Hex; block: number; logIndex: number; at: number | null };

export type Replay = {
  /** the draw set of each resolved ticket, at its resolution, in the contract's order */
  setAt: Map<number, number[]>;
  /** anything the events say that the contract could not have done (an item drawn that was not in the set, a ticket out of order) */
  faults: string[];
  free: number[];
  pending: { itemId: number; stamp: number }[];
  tickets: Map<number, { stage: "pinned" | "resolved" | "delivered" | "refunded"; round: number; at: number | null; itemId: number | null }>;
  items: Map<number, { tokenId: string; how: "purchase" | "deposit"; tx: Hex; at: number | null; held: boolean }>;
  nextToResolve: number;
};

export function replay(events: VaultEvent[], arrivalTx: (e: Extract<VaultEvent, { name: "Wrapped" | "Deposited" }>) => Hex = (e) => e.tx): Replay {
  const sorted = [...events].sort((a, b) => a.block - b.block || a.logIndex - b.logIndex);
  const free: number[] = [], queue: { itemId: number; stamp: number }[] = [], faults: string[] = [], setAt = new Map<number, number[]>();
  const tickets: Replay["tickets"] = new Map(), items: Replay["items"] = new Map();
  let head = 0, ticketCount = 0, next = 0;
  const skipRefunded = () => { while (next < ticketCount && tickets.get(next)?.stage === "refunded") next++; };
  for (const e of sorted) {
    switch (e.name) {
      case "Wrapped": case "Deposited":
        if (items.has(e.itemId)) faults.push(`item ${e.itemId} minted twice`);
        if (e.itemId !== items.size) faults.push(`item ${e.itemId} minted out of order (expected ${items.size})`);
        items.set(e.itemId, { tokenId: e.tokenId, how: e.name === "Wrapped" ? "purchase" : "deposit", tx: arrivalTx(e), at: e.at, held: true });
        queue.push({ itemId: e.itemId, stamp: ticketCount });
        break;
      case "Redeemed":
        if (e.ticket !== ticketCount) faults.push(`ticket ${e.ticket} burned out of order (expected ${ticketCount})`);
        tickets.set(e.ticket, { stage: "pinned", round: e.round, at: e.at, itemId: null });
        ticketCount = Math.max(ticketCount, e.ticket + 1);
        break;
      case "Resolved": {
        const t = tickets.get(e.ticket);
        if (!t || t.stage !== "pinned") { faults.push(`ticket ${e.ticket} resolved from stage ${t?.stage ?? "none"}`); break; }
        if (e.ticket !== next) faults.push(`ticket ${e.ticket} resolved out of order (next was ${next})`);
        while (head < queue.length && queue[head].stamp <= e.ticket) free.push(queue[head++].itemId);
        setAt.set(e.ticket, free.slice());
        const i = free.indexOf(e.itemId);
        if (i < 0) faults.push(`ticket ${e.ticket} drew item ${e.itemId}, which was not in its set`);
        else { free[i] = free[free.length - 1]; free.pop(); }
        const it = items.get(e.itemId);
        if (it && it.tokenId !== e.tokenId) faults.push(`ticket ${e.ticket}: item ${e.itemId} is token ${it.tokenId}, the event says ${e.tokenId}`);
        t.stage = "resolved"; t.itemId = e.itemId;
        next = e.ticket + 1; skipRefunded();
        break;
      }
      case "Delivered": {
        const t = tickets.get(e.ticket);
        if (!t || t.stage !== "resolved") { faults.push(`ticket ${e.ticket} delivered from stage ${t?.stage ?? "none"}`); break; }
        t.stage = "delivered";
        const it = t.itemId === null ? undefined : items.get(t.itemId);
        if (it) it.held = false;
        break;
      }
      case "Refunded": {
        const t = tickets.get(e.ticket);
        if (!t) { faults.push(`ticket ${e.ticket} refunded but never burned`); break; }
        if (e.itemId !== UNRESOLVED_ITEM) queue.push({ itemId: Number(e.itemId), stamp: ticketCount });
        t.stage = "refunded";
        skipRefunded();
        break;
      }
    }
  }
  return { setAt, faults, free, pending: queue.slice(head), tickets, items, nextToResolve: next };
}

/** The vault as the odds need it, from a replay: the same shape as the wrapper's own views give (`vaultFromViews` in reads.ts). */
export function vaultFromReplay(r: Replay, block: number, at: number | null): VaultState {
  const promised = new Map<number, number>();
  for (const [id, t] of r.tickets) if (t.stage === "resolved" && t.itemId !== null) promised.set(t.itemId, id);
  const pendingOf = new Map(r.pending.map((p) => [p.itemId, p.stamp]));
  const freeSet = new Set(r.free);
  const items: VaultItem[] = [];
  for (const [itemId, it] of r.items) {
    if (!it.held) continue;
    const state = freeSet.has(itemId) ? "free" : pendingOf.has(itemId) ? "pending" : promised.has(itemId) ? "promised" : null;
    if (!state) continue;
    items.push({ itemId, tokenId: it.tokenId, how: it.how, arrivalTx: it.tx, mintedAt: it.at, state, stamp: pendingOf.get(itemId) ?? null, promisedTo: promised.get(itemId) ?? null });
  }
  const pinned = [...r.tickets].filter(([, t]) => t.stage === "pinned").map(([id, t]) => ({ id, round: t.round, burnedAt: t.at })).sort((a, b) => a.id - b.id);
  const open = [...r.tickets.values()].filter((t) => t.stage === "pinned" || t.stage === "resolved").length;
  return { block, at, ticketCount: r.tickets.size, nextToResolve: r.nextToResolve, held: items.length, openTickets: open, free: r.free.slice(), pending: r.pending.slice(), pinned, items };
}

/**
 * §3.4 (a), decision R4: was the pinned round already public when the burn's block reached Ethereum? A sequencer may date a block
 * up to 24 h back; the batch that posts it to Ethereum cannot be back-dated. Normal batching makes the round public a little before
 * the post (the pin is only 60 s ahead): that is shown, and flagged only past `normalDelayS`.
 */
export function timingFlag(round: number, postedAt: number | null, normalDelayS = NORMAL_BATCH_DELAY_S): TimingFlag {
  const roundPublishedAt = timeOf(round);
  if (postedAt === null) return { level: "unknown", roundPublishedAt, postedAt, leadS: null, text: "when this burn's block reached Ethereum could not be read, so a back-dated burn cannot be ruled out for it" };
  const lead = postedAt - roundPublishedAt;
  if (lead <= 0) return { level: "clear", roundPublishedAt, postedAt, leadS: lead, text: `the round was published ${dur(-lead)} after the burn's block reached Ethereum: nobody could know it when the burn was made` };
  if (lead <= normalDelayS) return { level: "within-batch-delay", roundPublishedAt, postedAt, leadS: lead,
    text: `the round was published ${dur(lead)} before the burn's block reached Ethereum, within the chain's normal batching (up to ${dur(normalDelayS)}); a sequencer back-dating by more than that would be flagged` };
  return { level: "flagged", roundPublishedAt, postedAt, leadS: lead,
    text: `FLAG: the round was published ${dur(lead)} before the burn's block reached Ethereum, more than normal batching explains: the sequencer could have dated this burn back to a round it already knew` };
}
export const dur = (s: number): string => { s = Math.round(Math.abs(s)); return s < 90 ? `${s} s` : s < 5400 ? `${Math.round(s / 60)} min` : s < 172_800 ? `${(s / 3600).toFixed(1)} h` : `${(s / 86_400).toFixed(1)} days`; };

export type VerifyOpts = {
  wrapper: Address;
  /** the draw set from the verifier's own replay of the events, when it has them */
  replayed?: number[] | null;
  /** each item's token id, when the verifier has the item list (the wrapper's `item(i)` or a replay) */
  tokenOf?: (itemId: number) => string | null;
  /** the signatures the verifier fetched itself from relays */
  relays?: { relay: string; signature: Hex | null; error: string | null }[];
  /** what the verifier read itself from 4663: the ticket's round and the Resolved event */
  onChain?: { round: number | null; itemId: number | null; tokenId: string | null; source: string } | null;
  normalDelayS?: number;
};

/** Every check a draw's proof must pass, each with what it compared. A false check is a mismatch (SEV1); a null one is not checked. */
export function verifyDraw(p: DrawProof, o: VerifyOpts): DrawVerdict {
  const checks: Check[] = [];
  const add = (id: string, label: string, ok: boolean | null, detail: string) => checks.push({ id, label, ok, detail });

  // 1. the beacon: the signature is evmnet's for this round, and the randomness is its sha256
  let randomness: Hex | null = null;
  if (!p.beacon.signature) add("signature", "drand's signature of the round", null, `no signature is published for round ${p.round} yet`);
  else {
    const v = verifyBeacon(p.round, p.beacon.signature);
    if (v.ok) { randomness = v.randomness; add("signature", "drand's signature of the round", true, `a valid BLS signature of round ${p.round} under evmnet's public key, checked here`); }
    else add("signature", "drand's signature of the round", false, v.error);
    if (v.ok && p.beacon.randomness) add("randomness", "the randomness is the signature's sha256", p.beacon.randomness.toLowerCase() === v.randomness, p.beacon.randomness.toLowerCase() === v.randomness ? v.randomness : `published ${p.beacon.randomness}, computed ${v.randomness}`);
  }
  for (const r of o.relays ?? []) {
    if (!r.signature) add(`relay:${r.relay}`, `the round from ${r.relay}`, null, r.error ?? "not served");
    else add(`relay:${r.relay}`, `the round from ${r.relay}`, !p.beacon.signature ? null : r.signature.toLowerCase() === p.beacon.signature.toLowerCase(),
      !p.beacon.signature ? "served; the proof has none to compare" : r.signature.toLowerCase() === p.beacon.signature.toLowerCase() ? "the same bytes as the proof" : "OTHER bytes than the proof");
  }

  // 2. the pin: the round is the one the burn block's timestamp names, 20 rounds ahead; and the chain says the same
  const expected = roundAt(p.burnAt) + PIN_AHEAD;
  add("pin", "the round was pinned at the burn, 20 rounds ahead", p.round === expected, p.round === expected ? `roundAt(${p.burnAt}) + ${PIN_AHEAD} = ${expected}` : `the proof says ${p.round}, the burn's timestamp gives ${expected}`);
  if (o.onChain) {
    if (o.onChain.round !== null) add("chain-round", `ticket(${p.ticket}).round on ${o.onChain.source}`, o.onChain.round === p.round, `${o.onChain.round}`);
    if (o.onChain.itemId !== null) add("chain-item", `the Resolved event on ${o.onChain.source}`, o.onChain.itemId === p.itemId && (o.onChain.tokenId === null || o.onChain.tokenId === p.tokenId),
      `item ${o.onChain.itemId}${o.onChain.tokenId ? `, token #${o.onChain.tokenId}` : ""}`);
  }

  // 3. the set: never empty, and the same from the replay and from the views
  add("set", "the draw set is not empty", p.freeSet.length > 0, `${p.freeSet.length} item${p.freeSet.length === 1 ? "" : "s"}`);
  const same = (a: number[], b: number[]) => a.length === b.length && a.every((x, i) => x === b[i]);
  if (o.replayed) add("replay", "the set equals a replay of the wrapper's events", same(o.replayed, p.freeSet), same(o.replayed, p.freeSet) ? "same items, same order" : `replay [${o.replayed.join(", ")}] vs proof [${p.freeSet.join(", ")}]`);
  if (p.freeSetRead) add("views", "the set equals the wrapper's own views", same(p.freeSetRead, p.freeSet), same(p.freeSetRead, p.freeSet) ? "same items, same order" : `views [${p.freeSetRead.join(", ")}] vs proof [${p.freeSet.join(", ")}]`);

  // 4. the formula: the index names the item the event names
  if (randomness && p.freeSet.length) {
    const i = drawIndex(randomness, o.wrapper, p.ticket, p.freeSet.length);
    add("index", "keccak256(randomness, wrapper, ticket) mod size names the item", p.freeSet[i] === p.itemId, `index ${i} of ${p.freeSet.length} is item ${p.freeSet[i]}; the event says item ${p.itemId}`);
  } else add("index", "keccak256(randomness, wrapper, ticket) mod size names the item", null, "no verified randomness to compute it from");
  const tok = o.tokenOf?.(p.itemId) ?? null;
  if (tok !== null) add("token", "the item is the token the event names", tok === p.tokenId, `item ${p.itemId} is #${tok}; the event says #${p.tokenId}`);

  const timing = timingFlag(p.round, p.posting?.postedAt ?? null, o.normalDelayS);
  const verdict = checks.some((c) => c.ok === false) ? "mismatch" : checks.some((c) => c.ok === null && (c.id === "signature" || c.id === "index")) ? "incomplete" : "verified";
  return { ticket: p.ticket, verdict, checks, timing };
}
