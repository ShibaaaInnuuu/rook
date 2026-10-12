import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";
import { test } from "node:test";
import { type AbiEvent, toEventSelector, type Hex } from "viem";
import { foldLedger, LATE_DELIVERY_WINDOW_S, PIN_TIMEOUT_S, REFUND_GRACE_S, refundableAt } from "../src/wrapped/ledger.js";
import { foldDraws } from "../src/wrapped/draws.js";
import { DRAW_EVENTS, DRAW_SEED_READS, QUEUE_READS, WRAPPER_EVENTS, WRAPPER_READS } from "../src/wrapped/abis.js";
import { I_FLOOR_MODULE, I_KEEPER_SOURCE } from "../src/wrapped/floorModule.js";

/** docs/audit/2026-09-26/review2/integration-R3.md: what the round-3 contracts changed for the indexer. */

const ev = (name: string, args: Record<string, unknown>, block: number, ts: number, logIndex = 0) => ({ name, args, block, logIndex, tx: `0x${block.toString(16)}` as Hex, ts });
const A = (n: number) => `0x${n.toString(16).padStart(40, "0")}` as `0x${string}`;
const VAULT = A(0x16), COIN = A(0xc0);

test("R3 §4: a resolved ticket refunds 30 days after deliverBy, or 72 h once the custodian attests Undeliverable; pinned refunds unchanged", () => {
  const l = foldLedger("inscriptions", [
    ev("Deposited", { frogId: 1n, inscriptionId: "x1i0", txid: "0x" + "a".repeat(64), vout: 0, to: "0xholder" }, 1, 1),
    ev("Deposited", { frogId: 2n, inscriptionId: "x2i0", txid: "0x" + "b".repeat(64), vout: 0, to: "0xholder" }, 1, 1, 1),
    ev("Redeemed", { ticketId: 0n, owner: "0xo", round: 9n, destination: "bc1pdest" }, 2, 1_000),
    ev("Redeemed", { ticketId: 1n, owner: "0xo", round: 10n, destination: "bc1pdest" }, 3, 2_000),
    ev("Redeemed", { ticketId: 2n, owner: "0xo", round: 11n, destination: "bc1pdest" }, 4, 3_000),
    ev("Resolved", { ticketId: 0n, acquisitionId: 1n, deliverBy: 50_000n }, 5, 3_100),
    ev("Resolved", { ticketId: 1n, acquisitionId: 2n, deliverBy: 60_000n }, 6, 3_200),
    ev("Undeliverable", { ticketId: 1n }, 7, 40_000),
  ], { btcTx: (t) => `btc:${t}`, evmTx: null }, "BURN", 10_000);
  const [t0, t1, t2] = [0, 1, 2].map((i) => l.tickets.get(i)!);
  assert.equal(t0.undeliverable, false);
  assert.equal(t0.refundableAt, 50_000 + LATE_DELIVERY_WINDOW_S, "not attested: deliverBy + LATE_DELIVERY_WINDOW");
  assert.equal(t1.undeliverable, true);
  assert.equal(t1.refundableAt, 60_000 + REFUND_GRACE_S, "attested: deliverBy + REFUND_GRACE, as fast as before");
  assert.equal(t2.refundableAt, 3_000 + PIN_TIMEOUT_S, "a pinned ticket: 7 days after its pin, unchanged");
  assert.match(l.activity.map((a) => a.text).join("\n"), /ticket 1's delivery cannot confirm/);
  assert.equal(LATE_DELIVERY_WINDOW_S, 30 * 86_400);
  // docs/v7/phase1-nft.md §3.1: the Phase-1 NFT wrapper (EthNftWrapper) gates refunds as the Bitcoin one does
  assert.equal(refundableAt("nft", { stage: "resolved", burnedAt: 1, deliverBy: 2, undeliverable: true }), 2 + REFUND_GRACE_S, "EthNftWrapper: the same gated refund");
  assert.equal(refundableAt("nft", { stage: "resolved", burnedAt: 1, deliverBy: 2, undeliverable: false }), 2 + LATE_DELIVERY_WINDOW_S);
  // an Undeliverable for a ticket this read never saw is ignored, not guessed
  assert.doesNotThrow(() => foldLedger("inscriptions", [ev("Undeliverable", { ticketId: 9n }, 1, 1)], { btcTx: (t) => t, evmTx: null }, "BURN", 1));
});

test("R3 N-2: SeedCarried names the draw's seed origin; the snapshot time comes only from SnapshotFixed, never from this draw's announcedAt", () => {
  const events = [
    ev("DrawAnnounced", { drawId: 0n, coin: COIN, seedRound: 100n }, 1, 1_000),
    ev("DrawVoided", { drawId: 0n }, 2, 1_000 + 6 * 3_600 + 1),                        // the keeper never opened it: its seed carries
    ev("SeedCarried", { drawId: 1n, originId: 0n, seedRound: 100n }, 3, 90_000, 0),      // emitted just before DrawAnnounced
    ev("DrawAnnounced", { drawId: 1n, coin: COIN, seedRound: 100n }, 3, 90_000, 1),
    ev("SnapshotFixed", { drawId: 1n, snapshotAt: 500n }, 4, 91_000),
    ev("DrawOpened", { drawId: 1n, coin: COIN, root: `0x${"11".repeat(32)}`, totalTickets: 3n, uri: "https://l.example/1.json", challengeEnds: 91_900n }, 4, 91_000, 1),
    ev("DrawCancelled", { drawId: 1n }, 5, 91_500),                                    // cancelled before its pin: carries again
    ev("SeedCarried", { drawId: 2n, originId: 0n, seedRound: 100n }, 6, 200_000, 0),    // the origin stays the first draw, never chained
    ev("DrawAnnounced", { drawId: 2n, coin: COIN, seedRound: 100n }, 6, 200_000, 1),
    ev("DrawAnnounced", { drawId: 3n, coin: A(0xc1), seedRound: 7_000n }, 7, 300_000),
  ];
  const ds = new Map(foldDraws(VAULT, events).map((d) => [d.id, d]));
  assert.equal(ds.get(0)!.stage, "voided"); assert.equal(ds.get(0)!.seedPassedOn, true, "voided while announced: the coin's next draw reuses its seed");
  assert.equal(ds.get(0)!.seedOrigin, null);
  assert.equal(ds.get(1)!.seedOrigin, 0); assert.equal(ds.get(1)!.stage, "cancelled"); assert.equal(ds.get(1)!.seedPassedOn, true);
  assert.equal(ds.get(1)!.snapshotAt, 500, "SnapshotFixed's time, not anything counted back from its own announcement at 90,000");
  assert.equal(ds.get(2)!.seedOrigin, 0); assert.equal(ds.get(2)!.seedRound, 100);
  assert.equal(ds.get(2)!.snapshotAt, null, "announced, not opened: no snapshot time is shown, and none is computed from announcedAt");
  assert.equal(ds.get(2)!.voidableAt, 200_000 + 6 * 3_600, "OPEN_TTL counts from this draw's own announcement");
  assert.equal(ds.get(3)!.seedOrigin, null); assert.equal(ds.get(3)!.seedPassedOn, false);
  // a void of a resolved draw does not carry
  const r = foldDraws(VAULT, [ev("DrawAnnounced", { drawId: 0n, coin: COIN, seedRound: 1n }, 1, 1), ev("DrawOpened", { drawId: 0n, coin: COIN, root: `0x${"22".repeat(32)}`, totalTickets: 1n, uri: "", challengeEnds: 2n }, 2, 2),
    ev("DrawPinned", { drawId: 0n, round: 5n }, 3, 3), ev("DrawResolved", { drawId: 0n, winningTicket: 0n }, 4, 4), ev("DrawVoided", { drawId: 0n }, 5, 5)])[0];
  assert.equal(r.stage, "voided"); assert.equal(r.seedPassedOn, false);
});

test("R3 ABIs are the committed contracts': SeedCarried, Undeliverable, carriedSeed/seedOrigin, holdsRefunds, exitAtBacking, undeliverable (skipped without out/)", () => {
  const out = new URL("../../out/", import.meta.url).pathname;
  if (!existsSync(`${out}SweepVault.sol/SweepVault.json`)) return;
  type I = { type: string; name?: string; inputs?: { type: string }[]; outputs?: { type: string }[] };
  const abiOf = (c: string) => (JSON.parse(readFileSync(`${out}${c}.sol/${c}.json`, "utf8")) as { abi: I[] }).abi;
  const events = (c: string) => new Set(abiOf(c).filter((x) => x.type === "event").map((x) => toEventSelector(x as unknown as AbiEvent)));
  const sig = (x: I) => `${x.name}(${(x.inputs ?? []).map((i) => i.type).join(",")})->(${(x.outputs ?? []).map((o) => o.type).join(",")})`;
  const fns = (c: string) => new Set(abiOf(c).filter((x) => x.type === "function").map(sig));
  const ev1 = (abi: readonly unknown[], name: string) => (abi as AbiEvent[]).find((e) => e.type === "event" && e.name === name)!;
  assert.ok(events("SweepVault").has(toEventSelector(ev1(DRAW_EVENTS, "SeedCarried"))));
  assert.ok(events("Wrapper").has(toEventSelector(ev1(WRAPPER_EVENTS, "Undeliverable"))));
  const check = (c: string, abi: readonly unknown[], names: string[]) => {
    const found = (abi as I[]).filter((x) => x.type === "function" && names.includes(x.name!));
    assert.equal(found.length, names.length, `${c}: ${names.join(", ")} named`);
    for (const x of found) assert.ok(fns(c).has(sig(x)), `${c}.${sig(x)}`);
  };
  check("SweepVault", DRAW_SEED_READS, ["carriedSeed", "seedOrigin"]);
  check("FloorModule", I_FLOOR_MODULE, ["exitAtBacking", "holdsRefunds", "burnsOpen", "pauseFloor", "RESUME_GRACE"]);     // + r4-contracts
  check("ListingRegistry", I_KEEPER_SOURCE, ["keeperResumedAt", "keeperResumedAtBlock", "keeperPaused", "keeper"]);   // + R4-check A4-I3
  check("FloorModule", QUEUE_READS, ["holdsRefunds"]);
  check("ItemQueue", QUEUE_READS, ["holdsRefunds"]);
  check("Wrapper", WRAPPER_READS, ["undeliverable", "LATE_DELIVERY_WINDOW"]);
});
