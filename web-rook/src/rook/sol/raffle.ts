/**
 * The holder raffle (docs/v9/raffle-design.md; docs/v9/canister-only-solana.md §9.3): weekly per collection, kept by Rook's canister on
 * its books, no program on chain. `raffle_status(coll)` is a collection's raffle (the pot's lamports earmarked for prizes at `K_tre`, the
 * wX held as the next prize, the round's phase, snapshot, pinned drand round, ticket and winner); `books_view(coll)` prices the next
 * prize (the public mint price now) and names the collection's wX mint, which is the raffle's key (the cranks open the raffle with it)
 * and the `listing` the published snapshot is kept under (`raffle_snapshot(wx_mint, round, offset, limit)`). Pure but for `loadRaffles`
 * and `loadSnapshot`: test/raffle.test.ts is its spec.
 */
import { b58encode } from "./base58";
import { opt, type BooksViewV, type RaffleV, type RookCanister, type SnapshotPageV } from "./canister";
import type { Address } from "./keys";

export const ONE_WX = 1_000_000_000n;
export const RAFFLE_PERIOD = 7 * 86_400;
export const DRAW_TIMEOUT = 7 * 86_400;
export const CLAIM_WINDOW = 30 * 86_400;
/** The treasury's default share of a collection's reserve overflow sent to its raffle pot (`DEFAULT_RAFFLE_BPS`). */
export const DEFAULT_RAFFLE_BPS = 5_000;

export type RafflePhase = "idle" | "committed" | "drawn";
/** The canister's `Raffle` record, its fields in camelCase. `key`: the 32 bytes the draw is keyed by, the collection's wX mint. `winner`:
 *  null while the canister's key is zero (no claim yet). Times in unix seconds, 0 for never. */
export type Raffle = {
  key: Address | null; phase: RafflePhase; pot: bigint; prize: bigint; round: bigint; root: string; totalWeight: bigint; entries: number; snapshotSlot: bigint;
  committedAt: number; drandRound: bigint; ticket: bigint; drawnAt: number; winner: Address | null; claimedAt: number;
  totalIn: bigint; totalSpent: bigint; prizesFunded: bigint; prizesClaimed: bigint; roundsExpired: bigint; roundsCancelled: bigint;
};

const bytes = (b: Uint8Array | number[]): Uint8Array => (b instanceof Uint8Array ? b : Uint8Array.from(b));
const hex = (b: Uint8Array) => Array.from(b, (x) => x.toString(16).padStart(2, "0")).join("");
/** A 32-byte key as an address; the zero key (no winner yet) is null. */
export function keyText(b: Uint8Array | number[]): Address | null {
  const k = bytes(b);
  return k.length === 32 && k.some((x) => x !== 0) ? b58encode(k) : null;
}
export function phaseOf(p: RaffleV["phase"]): RafflePhase {
  if ("Idle" in p) return "idle";
  if ("Committed" in p) return "committed";
  if ("Drawn" in p) return "drawn";
  throw new Error(`unknown raffle phase ${Object.keys(p)[0]}`);
}
/** The canister's record as the pages read it. */
export function raffleOf(v: RaffleV): Raffle {
  return {
    key: keyText(v.key), phase: phaseOf(v.phase), pot: v.pot, prize: v.prize, round: v.round, root: hex(bytes(v.root)), totalWeight: v.total_weight, entries: v.entries,
    snapshotSlot: v.snapshot_slot, committedAt: Number(v.committed_at), drandRound: v.drand_round, ticket: v.ticket, drawnAt: Number(v.drawn_at), winner: keyText(v.winner),
    claimedAt: Number(v.claimed_at), totalIn: v.total_in, totalSpent: v.total_spent, prizesFunded: v.prizes_funded, prizesClaimed: v.prizes_claimed,
    roundsExpired: v.rounds_expired, roundsCancelled: v.rounds_cancelled,
  };
}

/** The pot against the next prize's price: 0..1 (1 = a prize can be funded now). null when the price is unknown. */
export function prizeProgress(pot: bigint, price: bigint | null): number | null {
  if (price === null || price <= 0n) return null;
  return pot >= price ? 1 : Number((pot * 10_000n) / price) / 10_000;
}

export const PHASE_LABEL: Record<RafflePhase, string> = { idle: "Open", committed: "Snapshot committed, drawing", drawn: "Drawn, waiting for the claim" };

/** When the next draw happens, in words and (when known) as a time. `prizeHeld`: the treasury holds a whole wX as the prize. */
export type NextDraw = { text: string; at: number | null };
export function nextDraw(r: Raffle, now: number, prizeHeld: boolean | null): NextDraw {
  if (r.phase === "committed") {
    const by = r.committedAt + DRAW_TIMEOUT;
    return { text: now >= by ? `drand round ${r.drandRound} was never proven: the round can be cancelled` : `drawing now (drand round ${r.drandRound})`, at: null };
  }
  if (r.phase === "drawn") {
    const until = r.drawnAt + CLAIM_WINDOW;
    return { text: now >= until ? "the claim window is over: the prize rolls to the next round" : `winning ticket ${r.ticket}; claim window until ${dateText(until)}`, at: until };
  }
  const due = r.committedAt === 0 ? now : r.committedAt + RAFFLE_PERIOD;
  if (prizeHeld === false) return { text: r.committedAt === 0 ? "as soon as the pot funds the first prize" : `${due <= now ? "as soon as the pot funds the next prize" : `${dateText(due)}, once the pot funds a prize`}`, at: due > now ? due : null };
  return due <= now ? { text: "due now: the next snapshot is taken at the canister's next pass", at: now } : { text: dateText(due), at: due };
}

export const dateText = (t: number) => new Date(t * 1000).toISOString().slice(0, 16).replace("T", " ") + " UTC";

// ---- the view ---------------------------------------------------------------------------------------------------------------------------

/** One collection's raffle as the pages show it. `coll`: the canister's index. `raffle` null: the canister has not opened it (it does, on
 *  the collection's books, before the first pot arrives). `price`: the next prize's price, the collection's public mint price now (null:
 *  no mint, the floor stale or paused). `prizeHeld`: a whole wX sits at the treasury as the prize. `address`: the raffle's key, the
 *  collection's wX mint. */
export type RaffleView = { slug: string; coll: number; wxMint: Address | null; address: Address | null; raffle: Raffle | null; price: bigint | null; prizeHeld: boolean | null };
export type BooksForRaffle = Pick<BooksViewV, "wx_mint" | "mint_price">;
export function raffleView(slug: string, coll: number, raffle: RaffleV | null, books: BooksForRaffle | null): RaffleView {
  const r = raffle ? raffleOf(raffle) : null;
  const price = books ? opt(books.mint_price) : null;
  return { slug, coll, wxMint: books?.wx_mint ?? null, address: r?.key ?? books?.wx_mint ?? null, raffle: r, price: price !== null && price > 0n ? price : null, prizeHeld: r ? r.prize >= ONE_WX : null };
}

/** Every pinned collection's raffle and books from the canister: `raffle_status` and `books_view` per collection, in parallel. */
export async function loadRaffles(c: RookCanister, cols: { slug: string; coll: number }[]): Promise<RaffleView[]> {
  return Promise.all(cols.map(async ({ slug, coll }) => {
    const [r, b] = await Promise.all([c.raffle_status(coll), c.books_view(coll)]);
    return raffleView(slug, coll, opt(r), opt(b));
  }));
}

/** The leaves the panel shows of a published snapshot (the canister pages them). */
export const SNAPSHOT_LEAVES = 20;
export type Snapshot = SnapshotPageV;
/** The published snapshot of `round`, kept under the collection's wX mint; null when the canister keeps none (it keeps the last rounds). */
export async function loadSnapshot(c: RookCanister, wxMint: Address, round: bigint, limit = SNAPSHOT_LEAVES): Promise<Snapshot | null> {
  return opt(await c.raffle_snapshot(wxMint, round, 0, limit));
}
/** A holder's share of the draw: its weight over the snapshot's total weight. */
export const leafShare = (weight: bigint, total: bigint): number => (total > 0n ? Number((weight * 10_000n) / total) / 10_000 : 0);

/** The front page's list: opened raffles, biggest pot first. */
export const biggestPots = (vs: RaffleView[], n = 6): RaffleView[] => vs.filter((v) => v.raffle).sort((a, b) => (b.raffle!.pot > a.raffle!.pot ? 1 : b.raffle!.pot < a.raffle!.pot ? -1 : 0)).slice(0, n);
