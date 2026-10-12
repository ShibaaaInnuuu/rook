import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { encodeAbiParameters, keccak256, concat, type Hex } from "viem";
import type { RaffleDraw, Ticket } from "../src/api/types.js";
import { CLOSE_REFUSED, LAUNCHES_PAUSED, REFUNDS_HELD, seedLine, claimRefusal, closeBlocked, deadlineOf, drandSignature, drawSteps, graduatingSteps, launchBlocked, roundTime, ticketLeaf, ticketLine, ticketSteps, verifyProof } from "../src/rook/keyless.js";

/** docs/audit/2026-09-26/fixes-W2-site.md §1: the keyless steps and when the contracts accept them. */

const A = (n: number) => `0x${n.toString(16).padStart(40, "0")}` as `0x${string}`;
const DAY = 86_400;
const ticket = (o: Partial<Ticket>): Ticket => ({ id: 3, owner: A(1), destination: "bc1p", toBurn: false, stage: "pinned", round: 1_000, burnedAt: 10_000, burnTx: "0x01",
  itemId: null, itemRef: null, deliverBy: null, overdue: false, deliveryTx: null, deliveryUrl: null, refundTx: null, ...o });

test("tickets: Resolve once the round is published and it is next in order; Refund 7 days after the pin, or after deliverBy: 72 h once attested undeliverable, else 30 days (R3 §4) — never a redraw", () => {
  const t = ticket({ round: 1_000, burnedAt: 10_000 });
  const before = ticketSteps(t, roundTime(1_000) - 1, 3);
  assert.equal(before.resolve!.ok, false); assert.match(before.resolve!.why!, /not published yet/);
  assert.equal(ticketSteps(t, roundTime(1_000), 3).resolve!.ok, true);
  const notNext = ticketSteps(t, roundTime(1_000), 2).resolve!;
  assert.equal(notNext.ok, false); assert.match(notNext.why!, /ticket 2 is next/);
  assert.equal(ticketSteps(t, 10_000 + 7 * DAY - 1, 3).refund!.ok, false);
  assert.equal(ticketSteps(t, 10_000 + 7 * DAY, 3).refund!.ok, true, "E-03: a pinned ticket is refundable 7 days after its pin");
  const r = ticket({ stage: "resolved", deliverBy: 50_000 });
  assert.equal(ticketSteps(r, 50_001, null).refund!.ok, false, "M-1: not at deliverBy");
  assert.equal(ticketSteps(r, 50_000 + 72 * 3_600, null).refund!.ok, false, "R3 §4: not 72 h after it without the custodian's word");
  assert.match(ticketSteps(r, 50_000 + 72 * 3_600, null).refund!.why!, /once the vault's custodian attests.*72 hours.*30 days after the deadline/);
  assert.equal(ticketSteps(r, 50_000 + 30 * DAY - 1, null).refund!.ok, false);
  assert.equal(ticketSteps(r, 50_000 + 30 * DAY, null).refund!.ok, true, "30 days after it, for a custodian that is gone");
  const att = ticket({ stage: "resolved", deliverBy: 50_000, undeliverable: true });
  assert.equal(ticketSteps(att, 50_000 + 72 * 3_600 - 1, null).refund!.ok, false);
  assert.equal(ticketSteps(att, 50_000 + 72 * 3_600, null).refund!.ok, true, "attested undeliverable: 72 h after it, as fast as before");
  assert.match(ticketLine(att, 60_000), /the vault's custodian attested it cannot be delivered/);
  assert.equal(ticketSteps(r, 60_000, null).resolve, null);
  assert.equal(ticketSteps(ticket({ stage: "refunded" }), 1e10, null).refund, null);
  assert.equal(ticketSteps(ticket({ refundableAt: 123 }), 123, null).refund!.at, 123, "the indexer's time wins when it gives one");
  for (const x of [ticket({}), ticket({ stage: "resolved", deliverBy: 50_000 }), ticket({ stage: "refunded", refundedUnresolved: true })]) assert.doesNotMatch(ticketLine(x, 1e9), /redraw/i);
  assert.match(ticketLine(ticket({ stage: "refunded", refundedUnresolved: true }), 0), /refunded unresolved.*no NFT was drawn/);
  assert.match(ticketLine(ticket({ stage: "resolved", deliverBy: 50_000 }), 60_000), /overdue — refundable from about/);
});

const leafs = [ticketLeaf(A(0xa1), 0n, 5n), ticketLeaf(A(0xa2), 5n, 12n)];
const pair = (a: Hex, b: Hex) => (BigInt(a) < BigInt(b) ? keccak256(concat([a, b])) : keccak256(concat([b, a])));
const ROOT = pair(leafs[0], leafs[1]);
const draw = (o: Partial<RaffleDraw>): RaffleDraw => ({ id: 0, vault: A(0x16), coin: A(0xc0), stage: "open", seedRound: 1, announcedAt: 1_000, snapshotAt: 900, root: ROOT, totalTickets: "12",
  uri: "https://x/l.json", challengeEnds: 2_000, published: { status: "matches", why: null }, round: null, pinnedAt: null, winningTicket: null, resolvedAt: null, winner: null, claim: null, voidableAt: null, tx: "0x01", ...o });

test("draws: pin after the window (not while the keeper is paused), resolve once the round exists, claim with a proof, void when due", () => {
  assert.deepEqual(drawSteps(draw({}), 1_999, false).map((s) => [s.id, s.ok]), [["pin", false]]);
  assert.deepEqual(drawSteps(draw({}), 2_000, false).map((s) => [s.id, s.ok]), [["pin", true]]);
  assert.match(drawSteps(draw({}), 2_000, true)[0].why!, /keeper is paused/, "requestDraw refuses while the keeper is paused");
  const P = roundTime(5_000) - 60;   // pinned a minute before its round
  const pinned = draw({ stage: "pinned", round: 5_000, pinnedAt: P, voidableAt: P + 7 * DAY });
  assert.deepEqual(drawSteps(pinned, roundTime(5_000) - 1, false).map((s) => [s.id, s.ok]), [["resolve", false], ["void", false]]);
  assert.deepEqual(drawSteps(pinned, P + 7 * DAY, false).map((s) => [s.id, s.ok]), [["resolve", true], ["void", true]], "a pinned draw nobody resolves is voidable after 7 days");
  const claim = { holder: A(0xa2), first: "5", end: "12", proof: [leafs[0]] };
  const resolved = draw({ stage: "resolved", winningTicket: "7", claim, voidableAt: 9_999_999 });
  assert.deepEqual(drawSteps(resolved, 5_000, false).map((s) => [s.id, s.ok]), [["claim", true], ["void", false]]);
  assert.match(drawSteps(draw({ stage: "resolved", claim: null, published: { status: "differs", why: "x" } }), 5_000, false)[0].why!, /does not reproduce the vault's root/);
  assert.deepEqual(drawSteps(draw({ stage: "announced", voidableAt: 1_000 + 6 * 3_600 }), 1_000 + 6 * 3_600, false).map((s) => [s.id, s.ok]), [["void", true]]);
  assert.deepEqual(drawSteps(draw({ stage: "claimed" }), 1e10, false), []);
});

test("a claim is checked against the draw as the chain holds it: resolved, the range holds the winner, the proof verifies against the on-chain root", () => {
  const c = { holder: A(0xa2), first: "5", end: "12", proof: [leafs[0]] };
  assert.ok(verifyProof(c.proof, ROOT, ticketLeaf(A(0xa2), 5n, 12n)));
  const chain = { stage: 3, root: ROOT, totalTickets: 12n, winningTicket: 7n };
  assert.equal(claimRefusal(chain, c), null);
  assert.match(claimRefusal({ ...chain, stage: 4 }, c)!, /not resolved on chain/);
  assert.match(claimRefusal({ ...chain, winningTicket: 3n }, c)!, /does not hold the winning ticket/);
  assert.match(claimRefusal({ ...chain, root: ("0x" + "22".repeat(32)) as Hex }, c)!, /does not verify/);
  assert.match(claimRefusal(chain, { ...c, holder: A(0xbad) })!, /does not verify/, "an indexer cannot swap in another holder");
  const manual = keccak256(keccak256(encodeAbiParameters([{ type: "address" }, { type: "uint256" }, { type: "uint256" }], [A(0xa2), 5n, 12n])));
  assert.equal(ticketLeaf(A(0xa2), 5n, 12n), manual, "SweepVault.leaf");
});

test("drand: the signature comes from the relays in turn, only if it is round's 64 bytes", async () => {
  const sig = "ab".repeat(64);
  const asked: string[] = [];
  const f = (async (u: string) => { asked.push(u); if (u.startsWith("https://api.drand.sh")) return new Response("down", { status: 503 });
    if (u.startsWith("https://api2.drand.sh")) return new Response(JSON.stringify({ round: 41, signature: sig })); return new Response(JSON.stringify({ round: 42, signature: sig })); }) as unknown as typeof fetch;
  assert.equal(await drandSignature(42, f), `0x${sig}`);
  assert.equal(asked.length, 3, "a relay answering the wrong round is passed over");
  assert.match(asked[0], /04f1e9062b8a81f848fded9c12306733282b2727ecced50032187751166ec8c3\/public\/42$/);
  const bad = (async () => new Response(JSON.stringify({ round: 42, signature: "abcd" }))) as unknown as typeof fetch;
  await assert.rejects(drandSignature(42, bad), /could not be read/);
  assert.equal(roundTime(1), 1_727_521_075);
});

test("the curve's gates: the closing buy waits for room (sells and smaller buys do not); graduate or open refunds at the hatch; launches pause for item budgets only", () => {
  const coin = { phase: "trading" as const, sweepOnly: false, canClose: false };
  assert.equal(closeBlocked(coin, "buy", true), true);
  assert.equal(closeBlocked(coin, "buy", false), false, "a buy short of the close");
  assert.equal(closeBlocked(coin, "sell", true), false, "holders can still sell back to the curve");
  assert.equal(closeBlocked({ ...coin, canClose: true }, "buy", true), false);
  assert.equal(closeBlocked({ ...coin, sweepOnly: true }, "buy", true), false, "SWEEP_ONLY has no item budget");
  assert.equal(closeBlocked({ ...coin, canClose: undefined }, "buy", true), false, "not read: the contract decides");
  assert.match(CLOSE_REFUSED, /closing buy is unavailable.*still sell/);
  assert.deepEqual(graduatingSteps({ phase: "graduating", canGraduate: true, refundsOpenAt: 5_000 }, 4_999), { graduate: true, refundsAt: 5_000, openRefunds: false, held: false });
  assert.deepEqual(graduatingSteps({ phase: "graduating", canGraduate: false, refundsOpenAt: 5_000 }, 5_000), { graduate: false, refundsAt: 5_000, openRefunds: true, held: false });
  assert.deepEqual(graduatingSteps({ phase: "trading", canGraduate: true, refundsOpenAt: 1 }, 5), { graduate: false, refundsAt: null, openRefunds: false, held: false });
  // R3 N-3: the chain's refundsOpenAt already holds the time; the step only says why
  assert.deepEqual(graduatingSteps({ phase: "graduating", canGraduate: false, refundsOpenAt: 9_000, refundsHeld: true }, 5_000), { graduate: false, refundsAt: 9_000, openRefunds: false, held: true });
  assert.match(REFUNDS_HELD, /keeper is paused.*up to 4 more days.*never longer/);
  assert.match(CLOSE_REFUSED, /or the keeper is paused/);
  assert.equal(launchBlocked(false, false), true); assert.equal(launchBlocked(false, true), false); assert.equal(launchBlocked(null, false), false);
  assert.match(LAUNCHES_PAUSED, /Launches paused — the floor module is full/);
  assert.equal(deadlineOf(1_000.9, 20), 2_200n);
});

test("R3 N-2: a draw that carries a seed says whose; one that ended before its pin says its seed passes on; the snapshot time is never computed", () => {
  assert.match(seedLine({ seedOrigin: 4, seedPassedOn: false, stage: "announced" })!, /reuses draw 4's seed round and snapshot time/);
  assert.match(seedLine({ seedOrigin: null, seedPassedOn: true, stage: "voided" })!, /next draw reuses its seed round and snapshot window/);
  assert.match(seedLine({ seedOrigin: 2, seedPassedOn: true, stage: "cancelled" })!, /draw 2's seed.*next draw reuses/);
  assert.equal(seedLine({ seedOrigin: null, seedPassedOn: false, stage: "open" }), null);
  assert.equal(seedLine({ stage: "resolved" }), null, "a report written before round 3");
  const src = readFileSync(new URL("../src/ui/Raffles.tsx", import.meta.url), "utf8");
  assert.doesNotMatch(src, /announcedAt\s*[-+]/, "the Raffles view computes nothing from announcedAt");
});
