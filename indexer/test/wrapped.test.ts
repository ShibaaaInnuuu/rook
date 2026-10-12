import assert from "node:assert/strict";
import { test } from "node:test";
import type { AddressInfo } from "node:net";
import { getAddress } from "viem";
import type { CustodyItem, FloorModuleRead, ItemCheck } from "../types.js";
import { serve } from "../src/api.js";
import { openDb } from "../src/db.js";
import { Live } from "../src/live.js";
import { Views } from "../src/views.js";
import { parseConfig } from "../src/wrapped/config.js";
import { computeInvariant, fmt, type InvariantInput } from "../src/wrapped/invariant.js";
import { foldLedger, openState } from "../src/wrapped/ledger.js";
import { WrappedService } from "../src/wrapped/service.js";
import { checkInscription, checkOwner, pinBlock } from "../src/wrapped/verify.js";
import { APES, buildWorld, FROGS, NOW, PROVIDERS, worldConfig } from "./wrappedWorld.js";

/** docs/v5/ux-and-reserves.md §2: the reserve invariant, the ledger, the public-source checks, and the endpoints. */
const E = 10n ** 18n;
const check = (verdict: ItemCheck["verdict"], checkedAt = 1_000, reason: string | null = null): ItemCheck => ({ verdict, reason, checkedAt, atBlock: 1, sources: [], arrivalTx: null });
const item = (ref: string, c: ItemCheck): CustodyItem => ({ itemId: ref, ref, refUrl: null, promisedTo: null, check: c,
  arrival: { how: "purchase", tx: null, txUrl: null, mintTx: "0x00", mintedAt: 1, mintBlock: 1 } });
/** A module read with every term of docs/v5/floor-module.md §13.6: `E` ETH-backed, none of it reserve or float unless said. */
const moduleRead = (over: Partial<Extract<FloorModuleRead, { kind: "read" }>> = {}): FloorModuleRead => ({ kind: "read", address: "0x00", ethBackedUnits: "0", reserveUnits: "0", convertibleUnits: "0",
  floatUnits: "0", escrowWei: "0", backingWei: "0", standingReserveWei: "0", revenueWei: "0", floorWei: "1", floorPostedAt: 1_000, floorFresh: true, priceWei: "1", fillBandBps: 300, fills: [],
  spreadMintBps: 15, spreadBurnBps: 15, betaConvertibleWei: "0", betaReserveWei: "0", mintPriceWei: "1", mintRoomWei: "0", burnPriceWei: "1", burnRoomUnits: "0", loanOutstandingWei: "0",
  treasury: null, volume7dWei: "0", at: 1_050, block: 10, ...over });
const base = (over: Partial<InvariantInput> = {}): InvariantInput => ({
  now: 1_100, maxAgeS: 900, ledgerComplete: true, ticketsInFlight: 1, floor: { kind: "absent", note: "" },
  wrapper: { ok: true, totalSupply: 2n * E, held: 3, openTickets: 1, block: 10, at: 1_050 },
  items: [item("a", check("in-custody")), item("b", check("in-custody")), item("c", check("in-custody"))], ...over });

test("green only when every item is found, every read is fresh, and (held − in flight) × 1 + ETH-backed = supply exactly", () => {
  const r = computeInvariant(base());
  assert.equal(r.status, "green"); assert.deepEqual(r.reasons, []);
  assert.equal(r.equation.backingUnits, String(2n * E)); assert.equal(r.equation.holds, true);
  // the Floor Module's ETH-backed units are a term of the same sum
  const withModule = computeInvariant(base({ wrapper: { ok: true, totalSupply: 2n * E + E / 2n, held: 3, openTickets: 1, block: 10, at: 1_050 },
    floor: moduleRead({ ethBackedUnits: String(E / 2n), convertibleUnits: String(E / 4n), reserveUnits: String(E / 4n), floatUnits: String(E / 8n), escrowWei: "330000000000000000", backingWei: "165000000000000000", standingReserveWei: "165000000000000000" }) }));
  assert.equal(withModule.status, "green"); assert.equal(withModule.equation.ethBackedUnits, String(E / 2n));
  // docs/v5/floor-module.md §13.1: the new terms are carried into the equation, and their own identities are checked
  assert.equal(withModule.equation.reserveUnits, String(E / 4n)); assert.equal(withModule.equation.convertibleUnits, String(E / 4n));
  assert.equal(withModule.equation.floatUnits, String(E / 8n)); assert.equal(withModule.equation.escrowWei, "330000000000000000");
  const tiers = computeInvariant(base({ wrapper: { ok: true, totalSupply: 2n * E + E / 2n, held: 3, openTickets: 1, block: 10, at: 1_050 },
    floor: moduleRead({ ethBackedUnits: String(E / 2n), convertibleUnits: String(E / 2n), reserveUnits: String(E / 4n), escrowWei: "1" }) }));
  assert.equal(tiers.status, "red"); assert.match(tiers.reasons.join(" "), /tiers do not add up/);
  const empty = computeInvariant(base({ wrapper: { ok: true, totalSupply: 2n * E + E / 2n, held: 3, openTickets: 1, block: 10, at: 1_050 },
    floor: moduleRead({ ethBackedUnits: String(E / 2n), convertibleUnits: String(E / 2n), escrowWei: "0" }) }));
  assert.equal(empty.status, "red"); assert.match(empty.reasons.join(" "), /empty escrow/);
});

test("docs/v6/relay-conversion.md §2: a unit in transit is a tier of its own, E = C + R + T, and only one at a time", () => {
  const supply = { ok: true as const, totalSupply: 2n * E + 2n * E, held: 3, openTickets: 1, block: 10, at: 1_050 };
  // two ETH-backed units: one convertible, one whose budget was advanced before its item (backing left: the escrow is one unit's)
  const transit = computeInvariant(base({ wrapper: supply,
    floor: moduleRead({ ethBackedUnits: String(2n * E), convertibleUnits: String(E), transitUnits: String(E), advancedWei: "650000000000000000", advancedAt: 1_000, escrowWei: "630000000000000000" }) }));
  assert.equal(transit.status, "green"); assert.deepEqual(transit.reasons, []);
  assert.equal(transit.equation.transitUnits, String(E)); assert.equal(transit.equation.convertibleUnits, String(E));
  // the same books read as if the transit unit did not exist: the tiers do not add up
  const lost = computeInvariant(base({ wrapper: supply, floor: moduleRead({ ethBackedUnits: String(2n * E), convertibleUnits: String(E), escrowWei: "1" }) }));
  assert.equal(lost.status, "red"); assert.match(lost.reasons.join(" "), /tiers do not add up/);
  const two = computeInvariant(base({ wrapper: supply, floor: moduleRead({ ethBackedUnits: String(2n * E), convertibleUnits: "0", transitUnits: String(2n * E), escrowWei: "1" }) }));
  assert.equal(two.status, "red"); assert.match(two.reasons.join(" "), /one advance at a time/);
  // a module built before the advance (null) and a report without the field are read as no transit
  const old = computeInvariant(base({ wrapper: supply, floor: moduleRead({ ethBackedUnits: String(2n * E), convertibleUnits: String(2n * E), transitUnits: null, escrowWei: "1" }) }));
  assert.equal(old.status, "green"); assert.equal(old.equation.transitUnits, undefined);
});

test("a mismatch is red in either direction: under-backed, and more backing than supply (the books are wrong)", () => {
  const under = computeInvariant(base({ wrapper: { ok: true, totalSupply: 3n * E, held: 3, openTickets: 1, block: 10, at: 1_050 } }));
  assert.equal(under.status, "red"); assert.match(under.reasons.join(" "), /under-backed/); assert.equal(under.equation.holds, false);
  const over = computeInvariant(base({ wrapper: { ok: true, totalSupply: E, held: 3, openTickets: 1, block: 10, at: 1_050 } }));
  assert.equal(over.status, "red"); assert.match(over.reasons.join(" "), /more backing than supply/);
  // a single wei of difference is a mismatch: the equation is exact, not approximate
  assert.equal(computeInvariant(base({ wrapper: { ok: true, totalSupply: 2n * E + 1n, held: 3, openTickets: 1, block: 10, at: 1_050 } })).status, "red");
});

test("an item a source places elsewhere is red, even while other sources failed", () => {
  const r = computeInvariant(base({ items: [item("a", check("in-custody")), item("b", check("elsewhere", 1_000, "ord places it at bc1q…")), item("c", check("unverified", 1_000, "ord did not answer"))] }));
  assert.equal(r.status, "red"); assert.match(r.reasons[0], /outside custody: b/); assert.equal(r.equation.itemsElsewhere, 1);
});

test("a partial source failure is unverified, never green, and it names what failed", () => {
  const r = computeInvariant(base({ items: [item("a", check("in-custody")), item("b", check("in-custody")), item("c", check("unverified", 1_000, "blockstream.info did not answer"))] }));
  assert.equal(r.status, "unverified"); assert.match(r.reasons.join(" "), /blockstream\.info did not answer/);
  assert.equal(r.equation.holds, null, "not decided: the unread item could be either way");
  assert.equal(r.equation.backingUnitsIfUnverifiedHold, String(2n * E));
  // …unless the unread items could not close the gap even if they were all there: then it is red
  const hopeless = computeInvariant(base({ wrapper: { ok: true, totalSupply: 5n * E, held: 3, openTickets: 1, block: 10, at: 1_050 },
    items: [item("a", check("in-custody")), item("b", check("in-custody")), item("c", check("unverified"))] }));
  assert.equal(hopeless.status, "red"); assert.match(hopeless.reasons.join(" "), /even if every unverified item is in custody/);
});

test("stale reads are unverified: an old item check, an old wrapper read, an old Floor Module read", () => {
  const oldItem = computeInvariant(base({ items: [item("a", check("in-custody", 100)), item("b", check("in-custody")), item("c", check("in-custody"))] }));
  assert.equal(oldItem.status, "unverified"); assert.equal(oldItem.items[0].check.verdict, "unverified"); assert.match(oldItem.items[0].check.reason!, /stale/);
  const noTime = computeInvariant(base({ items: [item("a", check("in-custody")), item("b", { ...check("in-custody"), checkedAt: null }), item("c", check("in-custody"))] }));
  assert.equal(noTime.status, "unverified", "a check with no time is as good as stale");
  assert.equal(computeInvariant(base({ wrapper: { ok: true, totalSupply: 2n * E, held: 3, openTickets: 1, block: 10, at: 10 } })).status, "unverified");
  assert.equal(computeInvariant(base({ floor: moduleRead({ at: 1, block: 1 }) })).status, "unverified");
});

test("failed reads of the wrapper, the Floor Module or the log are unverified, with the reason", () => {
  const w = computeInvariant(base({ wrapper: { ok: false, error: "HTTP 503" } }));
  assert.equal(w.status, "unverified"); assert.match(w.reasons.join(" "), /HTTP 503/); assert.equal(w.equation.supplyUnits, null);
  assert.equal(computeInvariant(base({ floor: { kind: "error", address: "0x00", error: "reverted" } })).status, "unverified");
  const log = computeInvariant(base({ ledgerComplete: false }));
  assert.equal(log.status, "unverified"); assert.match(log.reasons.join(" "), /event log/);
});

test("the wrapper's books disagreeing with its own events is red", () => {
  assert.equal(computeInvariant(base({ wrapper: { ok: true, totalSupply: 2n * E, held: 4, openTickets: 1, block: 10, at: 1_050 } })).status, "red");
  assert.equal(computeInvariant(base({ wrapper: { ok: true, totalSupply: 2n * E, held: 3, openTickets: 2, block: 10, at: 1_050 } })).status, "red");
  assert.equal(fmt(3n * E / 2n), "1.5"); assert.equal(fmt(-E), "−1");
});

test("the ledger: arrivals, purchases told from deposits, deliveries leave, refunds stay, tickets in flight", () => {
  const ev = (name: string, args: Record<string, unknown>, block: number) => ({ name, args, block, logIndex: 0, tx: `0x${block}` as `0x${string}`, ts: block });
  const l = foldLedger("inscriptions", [
    ev("Deposited", { frogId: 1n, inscriptionId: "x1i0", txid: "0x" + "a".repeat(64), vout: 0, to: "0xQueue" }, 1),
    ev("Deposited", { frogId: 2n, inscriptionId: "x2i0", txid: "0x" + "b".repeat(64), vout: 0, to: "0xholder" }, 2),
    ev("Wrapped", { acquisitionId: 7n, inscriptionId: "x3i0", to: "0xdist" }, 3),
    ev("Redeemed", { ticketId: 0n, owner: "0xo", round: 9n, destination: "bc1pdest" }, 4),
    ev("Resolved", { ticketId: 0n, acquisitionId: 1n, deliverBy: 50n }, 5),
    ev("Delivered", { ticketId: 0n, acquisitionId: 1n, txid: "0x" + "c".repeat(64) }, 6),
    ev("Redeemed", { ticketId: 1n, owner: "0xo", round: 9n, destination: "BURN" }, 7),
    ev("Resolved", { ticketId: 1n, acquisitionId: 2n, deliverBy: 50n }, 8),
    ev("Refunded", { ticketId: 1n, acquisitionId: 2n, owner: "0xo" }, 9),
    ev("Redeemed", { ticketId: 2n, owner: "0xo", round: 9n, destination: "bc1pdest" }, 10),
    ev("Resolved", { ticketId: 2n, acquisitionId: 7n, deliverBy: 50n }, 11),
  ], { btcTx: (t) => `btc:${t}`, evmTx: null }, "BURN", 100, ["0xqueue"]);
  assert.equal(l.items.get("1")!.arrival.how, "purchase", "a Deposited to the queue is the canister's purchase");
  assert.equal(l.items.get("2")!.arrival.how, "deposit");
  assert.equal(l.items.get("2")!.arrival.txUrl, `btc:${"b".repeat(64)}`);
  assert.equal(l.items.get("7")!.arrival.tx, null, "a vault receipt names no Bitcoin tx in the event; ord supplies it");
  const s = openState(l);
  assert.deepEqual(s.held.map((i) => i.itemId), ["2", "7"], "the delivered one left; the refunded one is back in the set");
  assert.deepEqual(s.inFlight.map((t) => t.id), [2]); assert.equal(s.promised.get("7"), 2);
  assert.equal(l.tickets.get(1)!.toBurn, true); assert.equal(l.tickets.get(2)!.overdue, true, "past deliverBy with no delivery");
});

test("Bitcoin: ord and both explorers agreeing is in custody; ord elsewhere is elsewhere; an explorer outage or disagreement is unverified", async () => {
  const w = buildWorld(), id = [...w.ord.keys()][0], src = { fetch: w.fetch, now: () => NOW };
  const ok = await checkInscription(id, FROGS.custody, src);
  assert.equal(ok.verdict, "in-custody"); assert.equal(ok.sources.length, 3); assert.ok(ok.sources.every((s) => s.ok && s.url.startsWith("https://")));
  const tx = (w.ord.get(id) as { satpoint: string }).satpoint.split(":")[0];
  assert.equal(ok.arrivalTx, tx, "the output ord names is where it arrived");
  w.ord.set(id, { address: "bc1pthief", satpoint: `${tx}:0:0` });
  assert.equal((await checkInscription(id, FROGS.custody, src)).verdict, "elsewhere");
  w.ord.set(id, { address: FROGS.custody, satpoint: `${tx}:0:0` });
  w.failing.add("http:blockstream.info");
  const down = await checkInscription(id, FROGS.custody, src);
  assert.equal(down.verdict, "unverified"); assert.match(down.reason!, /blockstream\.info did not answer/);
  w.failing.clear(); w.esplora.set(`${tx}:0`, { pays: FROGS.custody, spent: true, height: 1 });
  assert.equal((await checkInscription(id, FROGS.custody, src)).verdict, "unverified", "ord says custody, the explorers say spent: they disagree");
  w.ord.set(id, "404");
  assert.equal((await checkInscription(id, FROGS.custody, src)).verdict, "unverified", "ord not knowing an id we hold is never green");
});

test("EVM: ownerOf on every provider at one pinned block; disagreement and outages are unverified; a revert everywhere is elsewhere", async () => {
  const w = buildWorld(), src = { fetch: w.fetch, chainId: 1, providers: PROVIDERS, now: () => NOW };
  const pin = await pinBlock(src);
  assert.ok(pin.ok && pin.block === w.evmHead - 8);
  const page = () => "https://etherscan.io/nft/x/1";
  assert.equal((await checkOwner("8520", APES.collection, APES.custodian, pin, src, page)).verdict, "in-custody");
  w.owners.get(PROVIDERS[1])!.set("8520", "0x9999999999999999999999999999999999999999");
  assert.equal((await checkOwner("8520", APES.collection, APES.custodian, pin, src, page)).verdict, "unverified", "providers disagree");
  w.owners.get(PROVIDERS[0])!.set("8520", "0x9999999999999999999999999999999999999999");
  assert.equal((await checkOwner("8520", APES.collection, APES.custodian, pin, src, page)).verdict, "elsewhere");
  for (const p of PROVIDERS) w.owners.get(p)!.set("8520", "revert");
  assert.equal((await checkOwner("8520", APES.collection, APES.custodian, pin, src, page)).verdict, "elsewhere");
  w.evmChainId = 8453;
  const wrong = await pinBlock(src);
  assert.equal(wrong.ok, false, "a provider on the wrong chain gives no pin at all");
  assert.equal((await checkOwner("8520", APES.collection, APES.custodian, wrong, src, page)).verdict, "unverified");
});

test("the service over the worked examples: frogs green, apes green, and one explorer down makes frogs unverified", async () => {
  const w = buildWorld();
  const svc = new WrappedService({ config: parseConfig(worldConfig()), rpc: w.rpc, fetch: w.fetch, now: () => NOW });
  const frogs = (await svc.reserve("frogs"))!;
  assert.equal(frogs.status, "green", frogs.reasons.join("; "));
  assert.equal(frogs.equation.itemsInCustody, 6); assert.equal(frogs.equation.ticketsInFlight, 2); assert.equal(frogs.equation.supplyUnits, "6500000000000000000");
  assert.equal(frogs.items.filter((i) => i.arrival.how === "purchase").length, 5, "six bought, one of them delivered"); assert.equal(frogs.items.filter((i) => i.arrival.how === "deposit").length, 1);
  assert.equal(frogs.items.filter((i) => i.promisedTo !== null).length, 1, "ticket 1 drew an item that is still in custody");
  assert.equal(frogs.lastGreenAt, NOW);
  // docs/v5/floor-module.md §13.6: the module's books, read at the same block: (6 − 2) × 1 + 2.5 ETH-backed = 6.5 in supply
  assert.equal(frogs.floorModule.kind, "read");
  if (frogs.floorModule.kind === "read") {
    const m = frogs.floorModule;
    assert.equal(m.ethBackedUnits, "2500000000000000000"); assert.equal(m.reserveUnits, "625000000000000000"); assert.equal(m.convertibleUnits, "1875000000000000000"); assert.equal(m.floatUnits, "500000000000000000");
    assert.equal(m.escrowWei, "1585000000000000000", "both pots"); assert.equal(m.backingWei, "1188750000000000000"); assert.equal(m.standingReserveWei, "396250000000000000");
    assert.equal(m.floorWei, "633000000000000000", "floor() decodes three words: the price is the first"); assert.equal(m.floorFresh, true); assert.equal(m.floorPostedAt, NOW - 30 - 600);
    assert.equal(m.priceWei, "640000000000000000", "the fills-based price, not the keeper floor"); assert.deepEqual(m.fills.map((f) => f.priceWei), ["645000000000000000", "636000000000000000"]);
    assert.deepEqual([m.spreadMintBps, m.spreadBurnBps], [21, 15]); assert.equal(m.mintRoomWei, String(3n * E)); assert.equal(m.burnRoomUnits, "625000000000000000");
    assert.equal(m.treasury, FROGS.treasury); assert.equal(m.volume7dWei, "3200000000000000000");
  }
  assert.equal(frogs.equation.escrowWei, "1585000000000000000"); assert.equal(frogs.equation.reserveUnits, "625000000000000000");
  // §13.3 the treasury, §13.2 the protocol-owned liquidity
  assert.equal(frogs.treasury.kind, "read");
  if (frogs.treasury.kind === "read") {
    assert.deepEqual(frogs.treasury.shares, { reservesBps: 4_000, floatsBps: 3_000, bootstrapBps: 2_000, opsBps: 1_000 });
    assert.deepEqual(frogs.treasury.pots, { reservesWei: "200000000000000000", floatsWei: "150000000000000000", bootstrapWei: "400000000000000000", unallocatedWei: "50000000000000000" });
    assert.equal(frogs.treasury.balanceWei, "800000000000000000"); assert.equal(frogs.treasury.ops, getAddress(FROGS.ops));
    assert.deepEqual(frogs.treasury.collections, [{ module: FROGS.module, weightWei: "3200000000000000000", outstandingLoanWei: "0" }]);
  }
  assert.equal(frogs.protocolLiquidity.kind, "read");
  if (frogs.protocolLiquidity.kind === "read") { assert.equal(frogs.protocolLiquidity.positions, 2); assert.equal(frogs.protocolLiquidity.pendingEthWei, "10000000000000000"); assert.equal(frogs.protocolLiquidity.treasury, FROGS.treasury); }
  const apes = (await svc.reserve("apes"))!;
  assert.equal(apes.status, "green", apes.reasons.join("; ")); assert.equal(apes.items[0].ref, "8520");
  assert.equal(apes.floorModule.kind, "absent"); assert.equal(apes.treasury.kind, "absent"); assert.equal(apes.equation.escrowWei, "0");
  const col = (await svc.collection("frogs"))!;
  assert.deepEqual(col.coinsList.map((c) => [c.symbol, c.phase, c.policy, c.feeBps]), [["GAMMA", "graduated", "yoyo", 500], ["BETA", "trading", "burn", 300], ["ALIEN", "graduated", "raffle", 100]]);
  // docs/v5/policies-and-fees.md §2: the launch fee, and §1: what the YOYO vault holds for GAMMA
  const gamma = col.coinsList[0], alien = col.coinsList[2];
  assert.deepEqual(gamma.launchFee, { bps: 5_000, decayS: 600, extraBpsNow: 3_333, endsAt: gamma.launchedAt! + 600 });
  assert.equal(alien.launchFee!.extraBpsNow, 0, "an old coin's launch fee has decayed to nothing");
  assert.equal(gamma.yoyo!.openListings, 1); assert.equal(gamma.yoyo!.proceedsWei, "350000000000000000"); assert.equal(gamma.yoyo!.boughtBackWei, "340000000000000000");
  assert.equal(gamma.yoyo!.burnedCoins, String(2_000_000n * E)); assert.equal(gamma.yoyo!.markupBps, 1_000);
  assert.deepEqual(gamma.yoyo!.listings.map((l) => [l.ticketId, l.stage]), [[4, "listed"], [3, "sold"]]);
  assert.equal(alien.yoyo, null);
  assert.equal(col.contracts.treasury, FROGS.treasury); assert.equal(col.contracts.offerBoard, FROGS.board); assert.equal(col.contracts.yoyoVault, FROGS.yoyoVault);
  assert.ok(col.activity.some((a) => a.kind === "bought-back") && col.activity.some((a) => a.kind === "relisted"), "the YOYO vault's events are activity");
  assert.equal(col.floorWei, "646000000000000000"); assert.equal(col.openTickets.length, 2);
  assert.equal((await svc.collection("apes"))!.floorWei, null, "a stale cap is no floor");
  // docs/v7/phase1-nft-pooled.md: the collection's own graduation targets and its pool (1.875 convertible units, due at 1.05)
  assert.deepEqual(col.graduation, { normalRaiseWei: String(9n * E), earlyRaiseWei: String(9n * E), earlyUntilEthWei: String(20n * E), maxRaiseWei: String(30n * E) });
  assert.deepEqual(col.pool, { units: "1875000000000000000", dueAtUnits: "1050000000000000000" });
  const apesCol = (await svc.collection("apes"))!;
  assert.equal(apesCol.pool, null, "no module: no pool");
  assert.equal((await svc.ticket("frogs", 0))!.stage, "delivered");

  const w2 = buildWorld(); w2.failing.add("http:blockstream.info");
  const down = (await new WrappedService({ config: parseConfig(worldConfig()), rpc: w2.rpc, fetch: w2.fetch, now: () => NOW }).reserve("frogs"))!;
  assert.equal(down.status, "unverified"); assert.equal(down.lastGreenAt, null);
  const w3 = buildWorld(); w3.failing.add("rpc:eth_getLogs");
  const noLog = (await new WrappedService({ config: parseConfig(worldConfig()), rpc: w3.rpc, fetch: w3.fetch, now: () => NOW }).reserve("frogs"))!;
  assert.equal(noLog.status, "unverified", "no events, no ledger: never green");
});

test("the endpoints: configured collections answer; none configured says so; unknown slugs are 404", async () => {
  const w = buildWorld(), db = openDb(":memory:");
  const views = new Views(db, new Live(null, null));
  const follower = () => ({ cursor: null, head: null, lastPassAt: null, lastError: null, reorgs: 0 }) as never;
  const start = (wrapped: WrappedService | null) => new Promise<{ url: string; close: () => void }>((ok) => {
    const s = serve({ views, follower, confirmations: 0, wrapped }, 0, "127.0.0.1");
    s.on("listening", () => ok({ url: `http://127.0.0.1:${(s.address() as AddressInfo).port}`, close: () => s.close() }));
  });
  const a = await start(new WrappedService({ config: parseConfig(worldConfig()), rpc: w.rpc, fetch: w.fetch, now: () => NOW }));
  try {
    const list = await (await fetch(`${a.url}/v1/wrapped`)).json() as { configured: boolean; collections: { slug: string; status: string }[] };
    assert.equal(list.configured, true); assert.deepEqual(list.collections.map((c) => c.slug), ["frogs", "apes"]);
    const r = await (await fetch(`${a.url}/v1/reserves/frogs`)).json() as { status: string; items: unknown[] };
    assert.equal(r.status, "green"); assert.equal(r.items.length, 6);
    assert.equal((await fetch(`${a.url}/v1/reserves/nope`)).status, 404);
    assert.equal((await fetch(`${a.url}/v1/wrapped/frogs/tickets/1`)).status, 200);
    assert.equal((await fetch(`${a.url}/v1/wrapped/frogs/tickets/99`)).status, 404);
    const hh = await (await fetch(`${a.url}/v1/handins/frogs`)).json() as { open: unknown[]; totals: { claimed: number } };
    assert.equal(hh.open.length, 1); assert.equal(hh.totals.claimed, 1);
    assert.equal((await fetch(`${a.url}/v1/handins/nope`)).status, 404);
  } finally { a.close(); }
  const b = await start(null);
  try {
    assert.deepEqual(await (await fetch(`${b.url}/v1/reserves`)).json(), { configured: false, reports: [] });
    assert.equal((await fetch(`${b.url}/v1/wrapped/frogs`)).status, 404);
    assert.equal((await fetch(`${b.url}/v1/handins/frogs`)).status, 404);
  } finally { b.close(); }
});

test("the offer board (docs/v6/offer-handins.md): what is open, cheapest first; what each hand-in became; what was paid", async () => {
  const w = buildWorld();
  const svc = new WrappedService({ config: parseConfig(worldConfig()), rpc: w.rpc, fetch: w.fetch, now: () => NOW });
  const h = (await svc.handins("frogs"))!;
  assert.equal(h.error, null); assert.equal(h.board, FROGS.board); assert.equal(h.custodian, getAddress(FROGS.custodian));
  assert.deepEqual(h.params, { bondWei: "10000000000000000", rewardWei: "10000000000000000", slashDelayS: 3_600, expiryS: 86_400, rewardDayCapWei: "200000000000000000", maxOpen: 64, maxOpenPerSubmitter: 4 });
  assert.equal(h.rewardPoolWei, "60000000000000000"); assert.equal(h.paidTodayWei, "10000000000000000");
  assert.deepEqual(h.open.map((x) => [x.id, x.status, x.askSats, x.slashable, x.expired]), [[2, "open", "1005000", false, false]]);
  assert.deepEqual(h.settled.map((x) => [x.id, x.status, x.rewardPaidWei]), [[1, "slashed", null], [0, "claimed", "10000000000000000"]]);
  assert.ok(h.settled[1].settledTx && h.settled[1].settledAt, "a claim names the transaction that paid it");
  assert.deepEqual(h.totals, { submitted: 3, open: 1, claimed: 1, slashed: 1, refunded: 0, rewardsPaidWei: "10000000000000000", bondsSlashedWei: "10000000000000000", fundedWei: "60000000000000000" });
  const apes = (await svc.handins("apes"))!;
  assert.equal(apes.board, null); assert.match(apes.error!, /Bitcoin collections/); assert.deepEqual(apes.open, []);
  assert.equal(await svc.handins("nope"), null);
});

test("a config must say where every item is checked", () => {
  assert.throws(() => parseConfig({ chainId: 4663, collections: [{ ...worldConfig().collections[0], custody: { kind: "evm" } }] }), /Bitcoin custody/);
  assert.throws(() => parseConfig({ chainId: 4663, collections: [worldConfig().collections[0], worldConfig().collections[0]] }), /duplicate/);
  assert.throws(() => parseConfig({ chainId: 4663, collections: [{ ...worldConfig().collections[1], custody: { kind: "evm", chainId: 1, collection: "0x1" } }] }), /contract and custodian/);
});

test("an incomplete event log is unverified even when the arithmetic on what was read looks under-backed", () => {
  const r = computeInvariant(base({ ledgerComplete: false, items: [] }));
  assert.equal(r.status, "unverified"); assert.equal(r.equation.holds, null);
});
