/** Demo raffles (`?demo`, src/rook/sol/demo.ts): made-up pots, one past winner and a published snapshot, in the canister's shapes
 *  (src/rook/sol/canister.ts `RaffleV`, `SnapshotPageV`) through src/rook/sol/raffle.ts's `raffleView`, as the live pages read them. */
import { b58decode } from "./base58";
import type { RaffleV, SnapshotPageV } from "./canister";
import { COLLECTIONS } from "./collections";
import { demoAddress, demoWxMint } from "./demo";
import { raffleView, type RafflePhase, type RaffleView } from "./raffle";

const D = 86_400;
type Seed = { slug: string; pot: number; price: number; phase: RafflePhase; round: number; committedAgo: number | null; held: boolean; winner?: boolean };
/** Pot and the next prize's price (the public mint price) in SOL. milady: a pot partway to its next prize, round 3 claimed last week (the
 *  past winner). bitcoin-frogs and chainmancers (Robinhood Chain): opened, no round yet. mad_lads (Solana): one round drawn and claimed
 *  last week. The other collections: no raffle yet. */
const SEEDS: Seed[] = [
  { slug: "milady", pot: 0.62, price: 1.55, phase: "idle", round: 3, committedAgo: 4 * D, held: false, winner: true },
  { slug: "pudgypenguins", pot: 3.4, price: 3.1, phase: "idle", round: 1, committedAgo: 9 * D, held: true },
  { slug: "bayc", pot: 1.9, price: 4.6, phase: "committed", round: 2, committedAgo: 40, held: true },
  { slug: "azuki", pot: 0.25, price: 0.9, phase: "drawn", round: 5, committedAgo: 2 * D, held: true },
  { slug: "bitcoin-frogs", pot: 0.08, price: 0.52, phase: "idle", round: 0, committedAgo: null, held: false },
  { slug: "mad_lads", pot: 0.9, price: 1.45, phase: "idle", round: 1, committedAgo: 6 * D, held: false },
  { slug: "chainmancers", pot: 0.3, price: 1.1, phase: "idle", round: 0, committedAgo: null, held: false },
];
const PHASE: Record<RafflePhase, RaffleV["phase"]> = { idle: { Idle: null }, committed: { Committed: null }, drawn: { Drawn: null } };
const ZERO = new Uint8Array(32);
const lamports = (sol: number) => BigInt(Math.round(sol * 1e9));

export const DEMO_WINNER = demoAddress("raffle-winner:milady");
/** The demo snapshots' total weight: the six leaves of `demoSnapshot` sum to it. */
export const DEMO_TOTAL_WEIGHT = 3_870_000_000n;

/** The canister's `Raffle` record for a seed, as `raffle_status` would reply it (the key is the collection's wX mint). */
function record(s: Seed, now: number): RaffleV {
  const committedAt = s.committedAgo === null ? 0 : now - s.committedAgo;
  return {
    key: b58decode(demoWxMint(s.slug)), phase: PHASE[s.phase], pot: lamports(s.pot), prize: s.held ? 1_000_000_000n : 0n, round: BigInt(s.round), root: new Uint8Array(32).fill(0xab),
    total_weight: DEMO_TOTAL_WEIGHT, entries: 412, snapshot_slot: 371_000_000n, committed_at: BigInt(committedAt), drand_round: 20_900_000n, ticket: s.phase === "drawn" ? 1_234_567_890n : 0n,
    drawn_at: BigInt(s.phase === "drawn" ? now - 2 * D + 120 : 0), winner: s.winner ? b58decode(DEMO_WINNER) : ZERO, claimed_at: BigInt(s.winner ? now - 4 * D + 900 : 0),
    total_in: lamports(s.pot + s.round * s.price), total_spent: lamports(s.round * s.price), prizes_funded: BigInt(s.round + (s.held ? 1 : 0)),
    prizes_claimed: BigInt(s.winner ? s.round : Math.max(0, s.round - 1)), rounds_expired: 0n, rounds_cancelled: 0n,
  };
}

/** Every collection, in the site's order (the demo's canister index is that order): the seeded five with a raffle, the rest not opened. */
export function demoRaffles(now: number): RaffleView[] {
  return COLLECTIONS.map((c, i) => {
    const s = SEEDS.find((x) => x.slug === c.slug);
    const books = { wx_mint: demoWxMint(c.slug), mint_price: (s ? [lamports(s.price)] : []) as [] | [bigint] };
    return raffleView(c.slug, i, s ? record(s, now) : null, books);
  });
}

/** The published snapshot of a demo raffle's round: six holders (the first page of 412), two backed coins, one excluded account. */
export function demoSnapshot(slug: string, round: bigint): SnapshotPageV | null {
  const s = SEEDS.find((x) => x.slug === slug);
  if (!s || s.round === 0 || round !== BigInt(s.round)) return null;
  const weights = [1_200_000_000n, 900_000_000n, 640_000_000n, 500_000_000n, 380_000_000n, 250_000_000n];
  let cum = 0n;
  const leaves = weights.map((w, i) => { const l = { owner: demoAddress(`holder:${slug}:${i}`), cum_start: cum, weight: w }; cum += w; return l; });
  const coin = (n: number) => demoAddress(`coin:${slug}:${n}`);
  return {
    listing: demoWxMint(slug), raffle: demoWxMint(slug), round, slot: 371_000_000n, slot_lo: 370_999_880n, taken_at: 1_791_000_000n, root: "ab".repeat(32),
    total_weight: DEMO_TOTAL_WEIGHT, entries: 412, offset: 0, leaves,
    coins: [
      { mint: coin(0), vault_wx: demoAddress(`vault:${slug}:0`), wx: 2_500_000_000n, supply: 1_000_000_000_000_000n, circ: 980_000_000_000_000n, holders: 300 },
      { mint: coin(1), vault_wx: demoAddress(`vault:${slug}:1`), wx: 1_370_000_000n, supply: 1_000_000_000_000_000n, circ: 995_000_000_000_000n, holders: 112 },
    ],
    excluded: [{ owner: demoAddress("desk"), coin: coin(0), account: demoAddress(`desk-ata:${slug}`), amount: 12_000_000_000_000n, reason: "canister key K_desk" }],
  };
}
