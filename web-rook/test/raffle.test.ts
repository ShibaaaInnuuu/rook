import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { b58decode } from "../src/rook/sol/base58";
import { decodeBooksView, type RaffleV, type RookCanister } from "../src/rook/sol/canister";
import { COLLECTIONS } from "../src/rook/sol/collections";
import { demoRaffles, demoSnapshot, DEMO_TOTAL_WEIGHT, DEMO_WINNER } from "../src/rook/sol/demoRaffle";
import * as R from "../src/rook/sol/raffle";
import { raffleRecord, snapshotPage, WINNER, WX_MINT } from "./fixtures/raffle";

/** W12 (docs/v9/canister-only-solana.md §9.3): the raffle is the canister's, read from `raffle_status` and priced by `books_view`; the
 *  helpers keep raffle-design.md's timing rules over the canister's record. */
const books = decodeBooksView(readFileSync(new URL("fixtures/views/books-view.hex", import.meta.url), "utf8").trim())[0]!;
const now = 1_800_000_000;

test("the canister's Raffle record reads field by field: the key and the winner as addresses (the zero key is none), every phase, times as seconds", () => {
  const r = R.raffleOf(raffleRecord({ phase: { Drawn: null }, winner: b58decode(WINNER), drawn_at: 1_791_050_000n, prize: 1_000_000_000n }));
  assert.equal(r.key, WX_MINT, "the raffle is keyed by the collection's wX mint");
  assert.equal(r.phase, "drawn");
  assert.equal(r.pot, 1_500_000_000n); assert.equal(r.prize, 1_000_000_000n); assert.equal(r.round, 3n); assert.equal(r.root, "ab".repeat(32)); assert.equal(r.totalWeight, 777n);
  assert.equal(r.entries, 42); assert.equal(r.snapshotSlot, 370_000_000n); assert.equal(r.committedAt, 1_791_000_000); assert.equal(r.drandRound, 20_900_000n);
  assert.equal(r.ticket, 555n); assert.equal(r.drawnAt, 1_791_050_000); assert.equal(r.winner, WINNER); assert.equal(r.claimedAt, 1_791_100_000);
  assert.deepEqual([r.totalIn, r.totalSpent, r.prizesFunded, r.prizesClaimed, r.roundsExpired, r.roundsCancelled], [10n, 20n, 30n, 40n, 50n, 60n]);
  assert.equal(R.raffleOf(raffleRecord()).winner, null, "a zero winner is none");
  assert.equal(R.raffleOf(raffleRecord({ key: new Uint8Array(32) })).key, null, "a zero key is none");
  assert.equal(R.raffleOf(raffleRecord({ winner: Array.from(b58decode(WINNER)) })).winner, WINNER, "a blob decoded as a number array reads the same");
  assert.deepEqual([{ Idle: null }, { Committed: null }, { Drawn: null }].map((phase) => R.phaseOf(phase as RaffleV["phase"])), ["idle", "committed", "drawn"]);
  assert.throws(() => R.phaseOf({ Cancelled: null } as unknown as RaffleV["phase"]), /phase/);
});

test("progress to the next prize and the next draw in every phase (raffle-design.md: a week per round, a week to prove, 30 days to claim)", () => {
  assert.equal(R.prizeProgress(500n, 2_000n), 0.25);
  assert.equal(R.prizeProgress(3_000n, 2_000n), 1);
  assert.equal(R.prizeProgress(1n, null), null);
  assert.equal(R.prizeProgress(1n, 0n), null);
  const base = R.raffleOf(raffleRecord({ committed_at: 0n }));
  assert.match(R.nextDraw(base, now, false).text, /first prize/);
  assert.match(R.nextDraw(base, now, true).text, /due now/);
  assert.equal(R.nextDraw(base, now, null).at, now, "unknown prize: due at the next pass");
  const week = { ...base, committedAt: now - 86_400 };
  const nd = R.nextDraw(week, now, true);
  assert.equal(nd.at, now - 86_400 + R.RAFFLE_PERIOD);
  assert.match(nd.text, /UTC$/);
  assert.match(R.nextDraw(week, now, false).text, /once the pot funds a prize/);
  assert.match(R.nextDraw({ ...week, committedAt: now - 8 * 86_400 }, now, false).text, /as soon as the pot funds the next prize/);
  assert.match(R.nextDraw({ ...week, phase: "committed" }, now, true).text, /drand round 20900000/);
  assert.match(R.nextDraw({ ...week, phase: "committed", committedAt: now - R.DRAW_TIMEOUT }, now, true).text, /cancelled/);
  const drawn = { ...week, phase: "drawn" as const, drawnAt: now - 3_600 };
  assert.equal(R.nextDraw(drawn, now, true).at, now - 3_600 + R.CLAIM_WINDOW);
  assert.match(R.nextDraw(drawn, now, true).text, /winning ticket 555/);
  assert.match(R.nextDraw({ ...drawn, drawnAt: now - R.CLAIM_WINDOW }, now, true).text, /rolls to the next round/);
  for (const p of ["idle", "committed", "drawn"] as const) assert.ok(R.PHASE_LABEL[p].length > 0);
  assert.equal(R.ONE_WX, 1_000_000_000n);
  assert.equal(R.DEFAULT_RAFFLE_BPS, 5_000);
});

test("the view: the next prize costs the books' public mint price, the key is the wX mint, a whole wX at the treasury is the prize held", () => {
  assert.equal(books.wx_mint, WX_MINT);
  const price = books.mint_price[0]!;
  assert.ok(price > 0n);
  const v = R.raffleView("milady", 0, raffleRecord({ prize: R.ONE_WX }), books);
  assert.deepEqual([v.slug, v.coll, v.wxMint, v.address, v.price, v.prizeHeld], ["milady", 0, WX_MINT, WX_MINT, price, true]);
  assert.equal(v.raffle!.round, 3n);
  assert.equal(R.raffleView("milady", 0, raffleRecord({ prize: R.ONE_WX - 1n }), books).prizeHeld, false, "999,999,999 units is not a prize");
  assert.equal(R.raffleView("milady", 0, raffleRecord({ prize: 2n * R.ONE_WX }), books).prizeHeld, true);
  const none = R.raffleView("milady", 0, null, books);
  assert.deepEqual([none.raffle, none.prizeHeld, none.address, none.price], [null, null, WX_MINT, price], "not opened: the books still name the key and the price");
  assert.equal(R.raffleView("milady", 0, raffleRecord(), { ...books, mint_price: [] }).price, null, "no mint (floor stale or paused): no price");
  assert.equal(R.raffleView("milady", 0, raffleRecord(), { ...books, mint_price: [0n] }).price, null);
  const noBooks = R.raffleView("milady", 0, raffleRecord(), null);
  assert.deepEqual([noBooks.price, noBooks.wxMint, noBooks.address], [null, null, WX_MINT], "no books: the raffle's own key");
  assert.equal(R.prizeProgress(v.raffle!.pot, v.price), Number((1_500_000_000n * 10_000n) / price) / 10_000);
});

test("loadRaffles asks raffle_status and books_view for each pinned collection; loadSnapshot reads a round under the wX mint", async () => {
  const calls: string[] = [];
  const fake = {
    raffle_status: async (coll: number) => { calls.push(`raffle_status ${coll}`); return coll === 1 ? [raffleRecord({ prize: R.ONE_WX })] : []; },
    books_view: async (coll: number) => { calls.push(`books_view ${coll}`); return coll === 1 ? [books] : []; },
    raffle_snapshot: async (listing: string, round: bigint, offset: number, limit: number) => { calls.push(`raffle_snapshot ${listing} ${round} ${offset} ${limit}`); return listing === WX_MINT && round === 3n ? [snapshotPage()] : []; },
  } as unknown as RookCanister;
  const vs = await R.loadRaffles(fake, [{ slug: "bayc", coll: 0 }, { slug: "milady", coll: 1 }]);
  assert.deepEqual(vs.map((v) => [v.slug, v.coll, !!v.raffle, v.prizeHeld, v.price !== null]), [["bayc", 0, false, null, false], ["milady", 1, true, true, true]]);
  assert.deepEqual(calls.sort(), ["books_view 0", "books_view 1", "raffle_status 0", "raffle_status 1"]);
  assert.equal(await R.loadSnapshot(fake, WX_MINT, 2n), null, "a round the canister no longer keeps");
  const s = (await R.loadSnapshot(fake, WX_MINT, 3n))!;
  assert.equal(s.leaves.length, 3);
  assert.equal(s.leaves.reduce((a, l) => a + l.weight, 0n), s.total_weight);
  assert.equal(calls.at(-1), `raffle_snapshot ${WX_MINT} 3 0 ${R.SNAPSHOT_LEAVES}`);
  assert.equal(R.leafShare(500n, 777n), 0.6435);
  assert.equal(R.leafShare(1n, 0n), 0);
  assert.deepEqual(R.biggestPots([]), []);
});

test("demo raffles: every collection, a pot partway to its prize, one past winner, every phase, a published snapshot, biggest pots first", () => {
  const vs = demoRaffles(now);
  assert.equal(vs.length, COLLECTIONS.length);
  assert.ok(vs.every((v, i) => v.slug === COLLECTIONS[i].slug && v.coll === i && v.wxMint !== null));
  const milady = vs.find((v) => v.slug === "milady")!;
  const p = R.prizeProgress(milady.raffle!.pot, milady.price)!;
  assert.ok(p > 0 && p < 1, `partway: ${p}`);
  assert.equal(milady.raffle!.winner, DEMO_WINNER);
  assert.equal(milady.address, milady.wxMint, "keyed by its wX mint");
  assert.ok(milady.raffle!.claimedAt > now - 7 * 86_400 && milady.raffle!.claimedAt < now);
  assert.equal(vs.find((v) => v.slug === "azuki")!.raffle!.winner, null);
  assert.deepEqual(new Set(vs.filter((v) => v.raffle).map((v) => v.raffle!.phase)), new Set(["idle", "committed", "drawn"]));
  assert.ok(vs.some((v) => !v.raffle), "some collection's raffle is not opened");
  assert.ok(vs.some((v) => v.raffle && v.raffle.round === 0n && v.prizeHeld === false), "one is opened with no round yet");
  for (const v of vs.filter((x) => x.raffle?.phase === "drawn")) assert.ok(v.raffle!.ticket < v.raffle!.totalWeight);
  const top = R.biggestPots(vs);
  assert.ok(top.length > 0 && top.every((v, i) => i === 0 || v.raffle!.pot <= top[i - 1].raffle!.pot));
  const s = demoSnapshot("milady", 3n)!;
  assert.equal(s.listing, milady.wxMint);
  assert.equal(s.round, 3n);
  assert.equal(s.total_weight, DEMO_TOTAL_WEIGHT);
  assert.equal(s.leaves.reduce((a, l) => a + l.weight, 0n), DEMO_TOTAL_WEIGHT, "the page's leaves sum to the total");
  assert.ok(s.leaves.every((l, i) => l.cum_start === s.leaves.slice(0, i).reduce((a, x) => a + x.weight, 0n)), "ticket ranges chain");
  assert.ok(Math.abs(s.leaves.reduce((a, l) => a + R.leafShare(l.weight, s.total_weight), 0) - 1) < 0.001);
  assert.equal(demoSnapshot("milady", 2n), null, "an older round is not kept");
  assert.equal(demoSnapshot("bitcoin-frogs", 0n), null, "no round yet");
  assert.equal(demoSnapshot("doodles", 1n), null, "not opened");
});
