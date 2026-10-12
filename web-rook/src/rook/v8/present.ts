import { verifyDraw } from "./core/draw";
import { LIMITS } from "./core/ethReserve";
import { nextTicketOdds } from "./core/odds";
import { snapshotHash } from "./core/rarity";
import type { DrawReport, DrawVerdict, MetadataSnapshot, Odds } from "./core/types";

/**
 * docs/v8/redemption-draw.md §2.4 and §6: what the redemption page says, in words, as plain data so test/v8.test.ts holds it.
 * Decision R1 (the recommended default, pending the operator's legal review): the vault's COMPOSITION with its exact odds — never
 * a draw of rare traits. Decision R2: no rarity rank on this page (the reserves inventory shows it). Decision R4: every past draw's
 * timing is checked for the 4663 back-dating flag. The words of §2.4's list are never used.
 */
export const BANNED_WORDS = ["win", "wins", "winner", "prize", "jackpot", "lucky", "luck", "pull", "pulls", "rare drop", "mystery", "spin", "loot", "gamble", "bet"] as const;
export function bannedIn(text: string): string[] {
  const t = text.toLowerCase();
  return BANNED_WORDS.filter((w) => new RegExp(`(^|[^a-z])${w.replace(" ", "\\s+")}([^a-z]|$)`).test(t));
}

/** The odds are shown only from a vault read younger than P1's hour, and only when the views and the event replay agree. */
export function oddsFor(r: DrawReport, now: number): { odds: Odds | null; why: string | null } {
  if (!r.state) return { odds: null, why: r.stateError ?? "the vault could not be read" };
  if (r.stateError) return { odds: null, why: r.stateError };
  const at = r.state.at ?? r.computedAt;
  if (now - at > LIMITS.maxAgeS) return { odds: null, why: `the vault was last read ${Math.round((now - at) / 60)} min ago: odds older than an hour are not shown as current` };
  return { odds: nextTicketOdds(r.state), why: null };
}

export function headline(o: Odds, name: string): { title: string; body: string } {
  const n = o.size, many = n === 1 ? name : name.endsWith("s") ? name : `${name}s`;
  if (!o.redeemable) return { title: "Nothing can be redeemed right now", body: "Every NFT in the vault is already drawn by a redemption waiting for delivery. A redemption burned now would be refused by the wrapper." };
  if (o.ahead === 0) return { title: `One of these ${n} ${many}, chosen at random`, body: `Each is equally likely: 1 in ${n}.` };
  return { title: `One of these ${n} ${many}, chosen at random`,
    body: `${o.ahead === 1 ? "One redemption burned before yours draws" : `${o.ahead} redemptions burned before yours draw`} first, each among the NFTs that were there when it was burned. So an NFT that arrived after ${o.ahead === 1 ? "it" : "them"} is a little more likely for you, and the others a little less. The exact odds of each NFT are below.` };
}
export const HOW_IT_WORKS = [
  "Your burn fixes a drand round about a minute in the future. Nobody, Rook included, can know that round's value before it is published.",
  "Anyone can then resolve your redemption: the wrapper takes keccak256(the round's randomness, the wrapper, your ticket) modulo the number of NFTs you can receive, and that NFT is yours. There is no redraw.",
  "Only NFTs already in the vault when you burn can reach you; an NFT added later is only for redemptions burned after it. The odds above are for a burn mined now.",
  "The vault's NFTs are mostly bought at the floor or deposited by holders; one whole wrapped token is always exactly one NFT of the collection, whichever it is.",
];

export type ProofRow = { ticket: number; verdict: DrawVerdict };
/** Every published proof checked here, in the browser: the BLS signature against evmnet's key, the pin, the set, the index. */
export function verifyAll(r: DrawReport): ProofRow[] {
  const tokenOf = new Map(r.state?.items.map((i) => [i.itemId, i.tokenId]) ?? []);
  return r.proofs.map((p) => ({ ticket: p.ticket, verdict: verifyDraw(p, { wrapper: r.wrapper, tokenOf: (id) => tokenOf.get(id) ?? null }) }));
}

/** The snapshot as served, against the hash the report says was pinned. */
export function snapshotCheck(s: MetadataSnapshot, want: string): { ok: boolean; got: string } {
  const got = snapshotHash(s);
  return { ok: got.toLowerCase() === want.toLowerCase(), got };
}

export const TIMING_WORD = { clear: "clear", "within-batch-delay": "normal", flagged: "flagged", unknown: "not read" } as const;
