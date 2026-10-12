import { b58decode } from "../../src/rook/sol/base58";
import type { RaffleV, SnapshotPageV } from "../../src/rook/sol/canister";

/** The wX mint of test/fixtures/views/books-view.hex: the raffle's key and the published snapshot's `listing`. */
export const WX_MINT = "DyRkUpQxYG2VnP2SkMdQs5BTsVPeKCpSHLxzByc2Sxvj";
export const WINNER = "GGgXnNgtY4ySfz8PLAAwkoh8Ztj7p6jrM9towSaa6Wuy";

/** A raffle as the canister's `raffle_status` would reply it (the canister's tests write no raffle fixture: test/canister.test.ts
 *  builds the bytes from this with the IDL), with overrides. Idle after round 3, no winner yet. */
export const raffleRecord = (over: Partial<RaffleV> = {}): RaffleV => ({
  key: b58decode(WX_MINT), phase: { Idle: null }, pot: 1_500_000_000n, prize: 0n, round: 3n, root: new Uint8Array(32).fill(0xab), total_weight: 777n, entries: 42,
  snapshot_slot: 370_000_000n, committed_at: 1_791_000_000n, drand_round: 20_900_000n, ticket: 555n, drawn_at: 0n, winner: new Uint8Array(32), claimed_at: 1_791_100_000n,
  total_in: 10n, total_spent: 20n, prizes_funded: 30n, prizes_claimed: 40n, rounds_expired: 50n, rounds_cancelled: 60n, ...over,
});

/** A page of a published snapshot as `raffle_snapshot(WX_MINT, 3, 0, limit)` would reply it: three leaves, one coin, one excluded account. */
export const snapshotPage = (over: Partial<SnapshotPageV> = {}): SnapshotPageV => ({
  listing: WX_MINT, raffle: WX_MINT, round: 3n, slot: 370_000_000n, slot_lo: 369_999_990n, taken_at: 1_790_990_000n, root: "ab".repeat(32), total_weight: 777n, entries: 3, offset: 0,
  leaves: [{ owner: WINNER, cum_start: 0n, weight: 500n }, { owner: "7UVimffxr9ow1uXYxsr4LHAcV58mLzhmwaeKvJ1pjLiE", cum_start: 500n, weight: 200n }, { owner: "EwhvTBi8sDnoJjTHneujyBBjxmQFQHw23HvENB5A6oFs", cum_start: 700n, weight: 77n }],
  coins: [{ mint: "7UVimffxr9ow1uXYxsr4LHAcV58mLzhmwaeKvJ1pjLiE", vault_wx: "EwhvTBi8sDnoJjTHneujyBBjxmQFQHw23HvENB5A6oFs", wx: 2_500_000_000n, supply: 1_000_000_000_000_000n, circ: 990_000_000_000_000n, holders: 3 }],
  excluded: [{ owner: WINNER, coin: "7UVimffxr9ow1uXYxsr4LHAcV58mLzhmwaeKvJ1pjLiE", account: "EwhvTBi8sDnoJjTHneujyBBjxmQFQHw23HvENB5A6oFs", amount: 10_000_000_000_000n, reason: "pool vault" }],
  ...over,
});
