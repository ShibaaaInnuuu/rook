import assert from "node:assert/strict";
import { test } from "node:test";
import { concat, keccak256, numberToHex, pad, sha256, type Hex } from "viem";
import { checkArrival, parseClaim, type ArrivalRead } from "../src/v8/arrival.js";
import { EVMNET, roundAt, timeOf, verifyBeacon } from "../src/v8/drand.js";
import { drawIndex, replay, timingFlag, verifyDraw, NORMAL_BATCH_DELAY_S } from "../src/v8/draw.js";
import { agreed, LIMITS, ownerVerdict, servedStatus } from "../src/v8/ethReserve.js";
import { looksKeyed, reader } from "../src/v8/jsonrpc.js";
import { F, nextTicketOdds, traitOdds } from "../src/v8/odds.js";
import { canonicalJson, diffSnapshots, rarityRanks, snapshotHash, traitCounts } from "../src/v8/rarity.js";
import { checkDraw, checkReserves } from "../src/v8/selfcheck.js";
import { V8Service, parseV8Config } from "../src/v8/service.js";
import type { DrawProof, VaultState } from "../src/v8/types.js";
import { CONTRACT_VECTORS, RELAY_ROUNDS } from "./v8vectors.js";
import { buildV8World, ETH_RPCS, GEN, HOME_RPCS, RELAYS, V8ADDR, v8Fetch, type ChainOpts } from "./v8World.js";

/** docs/v8/redemption-draw.md and docs/v8/proof-of-reserves.md: the verifier, the odds, the status rules, the service. */
const w = buildV8World();
const cfg = { collections: [{ slug: "milady", name: "Milady Maker", symbol: "wMIL", wrapper: V8ADDR.wrapper, collection: V8ADDR.collection, custodian: V8ADDR.custodian, module: V8ADDR.module, fromBlock: 65_000_000 }],
  homeRpcs: HOME_RPCS, ethRpcs: ETH_RPCS, relays: RELAYS, logChunk: 5_000_000 };
const service = (o: ChainOpts = {}, now = GEN + 30) => new V8Service({ config: cfg, fetch: v8Fetch(w, { now, ...o }), now: () => now, canisterRecords: async () => w.canister, snapshots: new Map([["milady", w.snapshot]]) });
const selfCheck = (o: ChainOpts = {}, now = GEN + 30, eth = ETH_RPCS, home = HOME_RPCS) => checkReserves({ slug: "milady", name: "Milady Maker", symbol: "wMIL", wrapper: V8ADDR.wrapper, collection: V8ADDR.collection,
  custodian: V8ADDR.custodian, module: V8ADDR.module, homeRpcs: home, ethRpcs: eth, fetch: v8Fetch(w, { now: GEN + 30, ...o }), now: () => now });
const host = (u: string) => new URL(u).host;
let proofsCache: DrawProof[] | null = null;
const proofs = async () => (proofsCache ??= (await service().draws("milady"))!.proofs);

// ---- drand evmnet, in TypeScript ------------------------------------------------------------------------------------------------

test("drand: the contract's own vectors verify, and the randomness is sha256 of the signature", () => {
  for (const v of CONTRACT_VECTORS) {
    const r = verifyBeacon(v.round, v.signature);
    assert.ok(r.ok, `round ${v.round}`); assert.equal(r.ok && r.randomness, v.randomness);
  }
});
test("drand: every round the two relays served verifies, with the relays' own randomness", () => {
  for (const [round, v] of Object.entries(RELAY_ROUNDS)) {
    const r = verifyBeacon(Number(round), v.signature);
    assert.ok(r.ok, `round ${round}`); assert.equal(r.ok && r.randomness, v.randomness); assert.equal(sha256(v.signature), v.randomness);
  }
});
test("drand: a real signature for another round, a tampered one, a point on the curve that is nobody's, and garbage are all refused", () => {
  const v = CONTRACT_VECTORS[0];
  assert.equal(verifyBeacon(v.round + 1, v.signature).ok, false);
  const flipped = (v.signature.slice(0, -1) + (v.signature.endsWith("6") ? "7" : "6")) as Hex;
  const r = verifyBeacon(v.round, flipped); assert.equal(r.ok, false); assert.match(!r.ok ? r.error : "", /curve/);
  assert.equal(verifyBeacon(v.round, concat([pad("0x01"), pad("0x02")])).ok, false);   // the G1 generator (1, 2)
  assert.equal(verifyBeacon(v.round, "0x1234").ok, false);
  assert.equal(verifyBeacon(0, v.signature).ok, false);
});
test("drand: roundAt and timeOf are DrandEvmnet's (genesis 1727521075, 3 s)", () => {
  assert.equal(roundAt(EVMNET.genesis), 1); assert.equal(roundAt(EVMNET.genesis + 2), 1); assert.equal(roundAt(EVMNET.genesis + 3), 2); assert.equal(roundAt(EVMNET.genesis - 1), 0);
  assert.equal(timeOf(1), EVMNET.genesis); assert.equal(timeOf(roundAt(1_790_000_000)) <= 1_790_000_000, true);
});

// ---- the draw ------------------------------------------------------------------------------------------------------------------

test("draw: the index is keccak256(abi.encode(bytes32 randomness, address wrapper, uint256 ticket)) mod size, byte for byte", () => {
  const r = RELAY_ROUNDS[20859662].randomness;
  const packed = keccak256(concat([r, pad(V8ADDR.wrapper), pad("0x07")]));
  assert.equal(drawIndex(r, V8ADDR.wrapper, 7, 13), Number(BigInt(packed) % 13n));
});
test("draw: the world replays with no fault, and each ticket's set is the free set plus the items stamped at or before it", () => {
  assert.deepEqual(w.replayed.faults, []);
  assert.equal(w.replayed.setAt.get(0)!.length, 14, "ticket 0 draws among the 14 first items");
  assert.equal(w.replayed.setAt.get(3)!.length, 13, "ticket 3: 14 − 3 drawn + 2 stamped 3");
  assert.ok(!w.replayed.setAt.get(2)!.includes(14), "an item stamped 3 is not in ticket 2's set");
  assert.ok(w.replayed.setAt.get(3)!.includes(14), "and is in ticket 3's");
});
test("draw: a replay flags events the contract could not have emitted", () => {
  const bad = w.events.map((e) => (e.name === "Resolved" && e.ticket === 2 ? { ...e, itemId: 19 } : e));
  assert.match(replay(bad).faults.join(" "), /not in its set/);
  const swapped = w.events.filter((e) => !(e.name === "Redeemed" && e.ticket === 0));
  assert.match(replay(swapped).faults.join(" "), /out of order|stage none/);
});
test("draw: every published proof verifies — real BLS signature, pinned round, set, index, token — and agrees with the views", async () => {
  const ps = await proofs();
  assert.equal(ps.length, 8);
  for (const p of ps) {
    const v = verifyDraw(p, { wrapper: V8ADDR.wrapper, replayed: w.replayed.setAt.get(p.ticket), tokenOf: (id) => w.items[id].tokenId });
    assert.equal(v.verdict, "verified", `ticket ${p.ticket}: ${JSON.stringify(v.checks.filter((c) => c.ok !== true))}`);
    assert.deepEqual(p.freeSetRead, p.freeSet, `ticket ${p.ticket}: the views at the block before say the same set`);
    assert.deepEqual(p.beacon.relays, RELAYS.map(host));
  }
});
test("draw: a tampered proof is a mismatch, whichever field was changed", async () => {
  const p = (await proofs()).find((x) => x.ticket === 2)!;
  const opts = { wrapper: V8ADDR.wrapper, replayed: w.replayed.setAt.get(2), tokenOf: (id: number) => w.items[id].tokenId };
  const ok = (q: DrawProof) => verifyDraw(q, opts);
  const failed = (q: DrawProof) => ok(q).checks.filter((c) => c.ok === false).map((c) => c.id);
  assert.equal(ok(p).verdict, "verified");
  // another item named
  const other = p.freeSet.find((x) => x !== p.itemId)!;
  assert.equal(ok({ ...p, itemId: other }).verdict, "mismatch"); assert.ok(failed({ ...p, itemId: other }).includes("index"));
  // the set reordered (the same items: only the order moves the index)
  const re = { ...p, freeSet: [...p.freeSet].reverse(), freeSetRead: null };
  assert.equal(ok(re).verdict, "mismatch"); assert.ok(failed(re).includes("replay"));
  // an item slipped into the set, in both the proof and the "views"
  const slipped = { ...p, freeSet: [...p.freeSet, 19], freeSetRead: [...p.freeSet, 19] };
  assert.equal(ok(slipped).verdict, "mismatch");
  // the signature swapped for another round's (valid, but not this round's)
  const sig = { ...p, beacon: { ...p.beacon, signature: RELAY_ROUNDS[20859662].signature } };
  assert.ok(failed(sig).includes("signature"));
  // the randomness replaced while the signature stays
  const rnd = { ...p, beacon: { ...p.beacon, randomness: RELAY_ROUNDS[20859662].randomness } };
  assert.ok(failed(rnd).includes("randomness"));
  // the round moved: the pin rule and the signature both fail
  const rd = { ...p, round: p.round + 1 };
  assert.deepEqual(failed(rd).sort(), ["pin", "signature"].sort());
  // the token the event names changed
  assert.ok(failed({ ...p, tokenId: "1" }).includes("token"));
  // no signature: incomplete, never verified
  assert.equal(ok({ ...p, beacon: { signature: null, randomness: null, relays: [] } }).verdict, "incomplete");
});
test("draw: the 4663 timestamp flag (R4) — clear, within normal batching, flagged, unknown", async () => {
  const levels = Object.fromEntries((await proofs()).map((p) => [p.ticket, verifyDraw(p, { wrapper: V8ADDR.wrapper }).timing.level]));
  assert.equal(levels[4], "flagged", "the round was public 15 min before the burn's batch reached Ethereum");
  assert.equal(levels[5], "unknown", "the batch could not be read");
  for (const t of [0, 1, 2, 3, 6, 7]) assert.equal(levels[t], "within-batch-delay");
  const r = 21_000_000, at = timeOf(r);
  assert.equal(timingFlag(r, at - 5).level, "clear");
  assert.equal(timingFlag(r, at + NORMAL_BATCH_DELAY_S).level, "within-batch-delay");
  assert.equal(timingFlag(r, at + NORMAL_BATCH_DELAY_S + 1).level, "flagged");
  assert.equal(timingFlag(r, null).level, "unknown");
});
test("draw: the independent check re-reads the chain and the relays; a relay serving other bytes is a mismatch", async () => {
  const p = (await proofs()).find((x) => x.ticket === 6)!;
  const run = (o: ChainOpts = {}) => checkDraw({ wrapper: V8ADDR.wrapper, proof: p, homeRpcs: HOME_RPCS, ethRpcs: ETH_RPCS, relays: RELAYS, fetch: v8Fetch(w, o) });
  const good = await run();
  assert.equal(good.verdict.verdict, "verified", JSON.stringify(good.verdict.checks.filter((c) => c.ok !== true)));
  assert.ok(good.verdict.checks.some((c) => c.id === "chain-item" && c.ok), "the Resolved event was read from the chain");
  assert.ok(good.calls.some((c) => c.chain === "drand") && good.calls.some((c) => c.method === "eth_getLogs"));
  const lying = await run({ relayLie: (relay) => (relay === "api2.drand.sh" ? RELAY_ROUNDS[20859662].signature : undefined) });
  assert.equal(lying.verdict.verdict, "mismatch");
  const forged = await checkDraw({ wrapper: V8ADDR.wrapper, proof: { ...p, itemId: p.freeSet.find((x) => x !== p.itemId)! }, homeRpcs: HOME_RPCS, ethRpcs: ETH_RPCS, relays: RELAYS, fetch: v8Fetch(w) });
  assert.equal(forged.verdict.verdict, "mismatch", "a proof naming another item than the chain's Resolved event");
  const noArchive = await run({ noArchive: new Set(HOME_RPCS.map(host)) });
  assert.ok(noArchive.notes.some((n) => /could not serve the wrapper's state/.test(n)));
});

// ---- the odds ----------------------------------------------------------------------------------------------------------------

const vault = (free: number[], pending: { itemId: number; stamp: number }[], pinned: number[], ticketCount: number): VaultState => ({
  block: 1, at: 1, ticketCount, nextToResolve: pinned[0] ?? ticketCount, held: free.length + pending.length + 1, openTickets: pinned.length, free, pending,
  pinned: pinned.map((id) => ({ id, round: 1, burnedAt: 1 })), items: [...free, ...pending.map((p) => p.itemId)].map((itemId) => ({ itemId, tokenId: String(itemId), how: "purchase", arrivalTx: "0x", mintedAt: 1, state: "free", stamp: null, promisedTo: null })) });
const sum = (xs: { p: { n: string; d: string } }[]) => xs.reduce((a, x) => a.add(F.of(x.p)), new F(0n));
/** the contract's process, enumerated: every index each ticket ahead could draw, equally likely */
function brute(state: VaultState): Map<number, F> {
  const out = new Map<number, F>();
  const go = (free: number[], queue: { itemId: number; stamp: number }[], ahead: number[], p: F) => {
    if (!ahead.length) { const set = [...free, ...queue.map((q) => q.itemId)]; for (const x of set) out.set(x, (out.get(x) ?? new F(0n)).add(p.mul(new F(1n, BigInt(set.length))))); return; }
    const [t, ...rest] = ahead, f = [...free], q = [...queue];
    while (q.length && q[0].stamp <= t) f.push(q.shift()!.itemId);
    for (let i = 0; i < f.length; i++) { const g = [...f]; g[i] = g[g.length - 1]; g.pop(); go(g, q, rest, p.mul(new F(1n, BigInt(f.length)))); }
  };
  go(state.free, state.pending, state.pinned.map((x) => x.id), new F(1n));
  return out;
}

test("odds: with no ticket ahead every item is exactly 1/N", () => {
  const o = nextTicketOdds(vault([1, 2, 3, 4, 5, 6, 7], [], [], 3));
  assert.equal(o.method, "uniform"); assert.equal(o.size, 7);
  for (const i of o.items) assert.deepEqual(i.p, { n: "1", d: "7" });
  assert.ok(sum(o.items).eq(new F(1n)));
});
test("odds: the world — one ticket ahead, an item stamped after it: 1/11 for that item, 10/121 for each other, exactly", async () => {
  const d = (await service().draws("milady"))!;
  const o = nextTicketOdds(d.state!);
  assert.equal(o.forTicket, 9); assert.equal(o.ahead, 1); assert.equal(o.method, "exact"); assert.equal(o.size, 11);
  assert.deepEqual(o.items.find((i) => i.itemId === 19)!.p, { n: "1", d: "11" });
  for (const i of o.items.filter((x) => x.itemId !== 19)) assert.deepEqual(i.p, { n: "10", d: "121" });
  assert.ok(sum(o.items).eq(new F(1n)));
});
test("odds: the exact programme equals an enumeration of the contract's own draws, across stamps and several tickets ahead", () => {
  const cases = [vault([10, 11, 12], [{ itemId: 20, stamp: 5 }, { itemId: 21, stamp: 6 }, { itemId: 22, stamp: 6 }, { itemId: 23, stamp: 8 }], [5, 6, 7], 8),
    vault([1, 2], [{ itemId: 3, stamp: 4 }, { itemId: 4, stamp: 5 }], [3, 4], 6), vault([1, 2, 3, 4, 5], [], [0, 1, 2, 3], 4)];
  for (const s of cases) {
    const o = nextTicketOdds(s), b = brute(s);
    for (const i of o.items) assert.ok(F.of(i.p).eq(b.get(i.itemId) ?? new F(0n)), `item ${i.itemId}: ${i.p.n}/${i.p.d} vs ${b.get(i.itemId)?.n}/${b.get(i.itemId)?.d}`);
    assert.ok(sum(o.items).eq(new F(1n)));
  }
});
test("odds: past the state budget the Monte Carlo is used, said, and close to the exact answer", () => {
  const s = vault([1, 2, 3, 4, 5, 6], [{ itemId: 7, stamp: 3 }, { itemId: 8, stamp: 4 }], [2, 3, 4], 5);
  const exact = nextTicketOdds(s), mc = nextTicketOdds(s, undefined, { maxStates: 1, samples: 100_000 });
  assert.equal(mc.method, "monte-carlo"); assert.ok(mc.stderr! > 0 && mc.stderr! < 0.01); assert.equal(mc.samples, 100_000);
  for (const i of exact.items) assert.ok(Math.abs(i.pct - mc.items.find((x) => x.itemId === i.itemId)!.pct) < 1.5, `item ${i.itemId}`);
});
test("odds: nothing drawable (every held item promised) is not redeemable", () => {
  const s = { ...vault([], [], [], 3), held: 2, openTickets: 2 };
  const o = nextTicketOdds(s);
  assert.equal(o.redeemable, false); assert.equal(o.items.length, 0);
});
test("odds: per trait value, the probabilities of the items that carry it, summing to 100 % per trait", async () => {
  const d = (await service().draws("milady"))!, o = nextTicketOdds(d.state!);
  const tokens = new Map(w.snapshot.tokens.map((t) => [t.tokenId, t]));
  const tr = traitOdds(o, tokens, traitCounts(w.snapshot.tokens));
  for (const t of tr) {
    const total = t.values.reduce((a, v) => a.add(F.of(v.p)), new F(0n));
    if (t.trait !== "Hat") assert.ok(total.eq(new F(1n)), `${t.trait} sums to one`);
    else assert.ok(total.n <= total.d, "a trait not every item has sums to at most one");
  }
});

// ---- the snapshot and rarity ------------------------------------------------------------------------------------------------------

test("snapshot: the hash is over canonical JSON (key order does not matter), and any trait change moves it", () => {
  assert.equal(canonicalJson({ b: 1, a: [2, { d: 3, c: 4 }] }), '{"a":[2,{"c":4,"d":3}],"b":1}');
  const s = w.snapshot, h = snapshotHash(s);
  assert.equal(snapshotHash(JSON.parse(JSON.stringify(s))), h);
  const t = structuredClone(s); t.tokens[5].traits.Eyes = t.tokens[5].traits.Eyes === "Laser" ? "Heart" : "Laser";
  assert.notEqual(snapshotHash(t), h);
  const drift = diffSnapshots(s, { tokens: [t.tokens[5], null] }, GEN);
  assert.equal(drift.changed.length, 1); assert.equal(drift.unreadable, 1); assert.equal(drift.changed[0].trait, "Eyes");
});
test("rarity: reproducible ranks; the one-of-one ranks first; ties share a rank", () => {
  const a = rarityRanks(w.snapshot.tokens), b = rarityRanks([...w.snapshot.tokens].reverse());
  assert.deepEqual([...a.entries()].sort(), [...b.entries()].sort(), "input order does not move a rank");
  assert.equal(a.get("237")!.rank, 1); assert.equal(a.get("237")!.unique, true);
  const twins = rarityRanks([{ tokenId: "1", traits: { A: "x" } }, { tokenId: "2", traits: { A: "x" } }, { tokenId: "3", traits: { A: "y" } }, { tokenId: "4", traits: { A: "y" } }]);
  assert.equal(twins.get("1")!.rank, twins.get("2")!.rank);
});

// ---- proof of reserves: the status rules ----------------------------------------------------------------------------------------

test("reserves: the world is green only with every read from two agreeing providers, fresh, and the equation exact", async () => {
  const { report, calls } = await selfCheck();
  assert.equal(report.status, "green", report.reasons.join("; "));
  assert.equal(report.equation.inCustody, 13); assert.equal(report.equation.ticketsInFlight, 2);
  assert.equal(report.equation.supplyUnits, "12850000000000000000"); assert.equal(report.equation.holds, true);
  assert.ok(report.items.every((i) => i.arrival.verdict === "ok" && i.arrival.final));
  assert.ok(report.items.filter((i) => i.how === "purchase").every((i) => i.arrival.pricePaidWei !== null), "every purchase's price is read from its Seaport fill");
  assert.ok(calls.some((c) => c.chain === "ethereum" && c.method === "eth_call") && calls.some((c) => c.chain === "robinhood"));
  assert.ok(report.numbers.every((n) => n.at !== null || n.ok === null || n.key === "ledger"), "every number carries its time");
});
test("reserves: one Ethereum provider down — single source — is never green", async () => {
  const { report } = await selfCheck({ down: new Set([host(ETH_RPCS[1])]) });
  assert.equal(report.status, "unverified"); assert.match(report.reasons.join(" "), /one source is not a verdict|did not answer/);
});
test("reserves: only one provider configured on either chain is never green", async () => {
  assert.equal((await selfCheck({}, GEN + 30, [ETH_RPCS[0]])).report.status, "unverified");
  assert.equal((await selfCheck({}, GEN + 30, ETH_RPCS, [HOME_RPCS[0]])).report.status, "unverified");
});
test("reserves: providers that disagree about an owner make it unverified, not red; both naming another owner is red", async () => {
  const token = w.items.find((i) => w.replayed.items.get(i.itemId)?.held)!.tokenId;
  const thief = "0x00000000000000000000000000000000000bad00";
  const oneLies = await selfCheck({ lie: (h, m, fn, args) => (h === host(ETH_RPCS[1]) && fn === "ownerOf" && String(args[0]) === token ? thief : undefined) });
  assert.equal(oneLies.report.status, "unverified"); assert.match(oneLies.report.reasons.join(" "), /disagree/);
  const both = await selfCheck({ lie: (_h, _m, fn, args) => (fn === "ownerOf" && String(args[0]) === token ? thief : undefined) });
  assert.equal(both.report.status, "red"); assert.match(both.report.reasons.join(" "), new RegExp(`#${token}`));
});
test("reserves: the Robinhood Chain providers disagreeing about the wrapper's books is unverified", async () => {
  const { report } = await selfCheck({ lie: (h, _m, fn) => (h === host(HOME_RPCS[1]) && fn === "totalSupply" ? 99n * 10n ** 18n : undefined) });
  assert.equal(report.status, "unverified"); assert.match(report.reasons.join(" "), /disagree/);
});
test("reserves: supply above backing on both providers is red (under-backed), whatever else holds", async () => {
  const { report } = await selfCheck({ lie: (_h, _m, fn) => (fn === "totalSupply" ? 14n * 10n ** 18n : undefined) });
  assert.equal(report.status, "red"); assert.match(report.reasons.join(" "), /under-backed/);
});
test("reserves: the module's tiers not adding up is red", async () => {
  const { report } = await selfCheck({ lie: (_h, _m, fn) => (fn === "netUnits" ? 1n : undefined) });
  assert.equal(report.status, "red"); assert.match(report.reasons.join(" "), /tiers do not add up/);
});
test("reserves: stale reads are never green; a report past 3 h is red when served; a green with an old number is unverified", async () => {
  // the chain's clock at GEN, the reader's at GEN + 2 h: every read is two hours old
  const old = await selfCheck({}, GEN + 2 * 3_600);
  assert.equal(old.report.status, "unverified"); assert.match(old.report.reasons.join(" "), /stale/);
  const { report } = await selfCheck();
  assert.equal(servedStatus(report, report.computedAt + 60).status, "green");
  assert.equal(servedStatus(report, report.computedAt + LIMITS.maxAgeS + 60).status, "unverified");
  const late = servedStatus(report, report.computedAt + LIMITS.redAfterS + 1);
  assert.equal(late.status, "red"); assert.match(late.reasons[0], /stale/);
});
test("reserves: agreed() needs two distinct sources that agree at one block", () => {
  const ok = (source: string, value: number, block = 1) => ({ source, ok: true as const, value, block, at: 1 });
  assert.equal(agreed([ok("a", 1), ok("b", 1)]).ok, true);
  const single = agreed([ok("a", 1)]); assert.equal(single.ok, false); assert.equal(!single.ok && single.single, true);
  assert.equal(agreed([ok("a", 1), ok("a", 1)]).ok, false, "the same host twice is one source");
  assert.equal(agreed([ok("a", 1), ok("b", 2)]).ok, false);
  assert.equal(agreed([ok("a", 1), ok("b", 1, 2)]).ok, false, "another block is not agreement");
  assert.equal(agreed([ok("a", 1), ok("b", 1), { source: "c", ok: false, error: "down" }]).ok, false, "a failed third provider is said");
  const c = V8ADDR.custodian;
  assert.equal(ownerVerdict([{ source: "a", owner: c, reverted: false, error: null }], c).verdict, "unverified");
  assert.equal(ownerVerdict([{ source: "a", owner: c, reverted: false, error: null }, { source: "b", owner: c, reverted: false, error: null }], c).verdict, "in-custody");
  assert.equal(ownerVerdict([{ source: "a", owner: null, reverted: true, error: "revert" }, { source: "b", owner: null, reverted: true, error: "revert" }], c).verdict, "elsewhere");
});
test("reserves: an arrival that does not check out is red; a receipt one provider lacks is unverified", () => {
  const it = w.items.find((i) => i.how === "deposit")!;
  const tx = { from: it.from, to: V8ADDR.collection, input: "0x" };
  const rc = { status: "0x1", blockNumber: "0x10", transactionHash: it.ethTx, logs: [] };
  const expect = { how: "deposit" as const, tx: it.ethTx, tokenId: it.tokenId, collection: V8ADDR.collection, custodian: V8ADDR.custodian, homeChainId: 4663, wrapper: V8ADDR.wrapper, recipient: it.recipient, finalized: 100 };
  const both = (receipt: ArrivalRead["receipt"]): ArrivalRead[] => [{ source: "a", receipt, tx, error: null }, { source: "b", receipt, tx, error: null }];
  assert.equal(checkArrival(both(rc), expect).verdict, "bad", "no Transfer to the custodian");
  assert.equal(checkArrival(both({ ...rc, status: "0x0" }), expect).verdict, "bad", "reverted");
  assert.equal(checkArrival([{ source: "a", receipt: rc, tx, error: null }, { source: "b", receipt: null, tx: null, error: "down" }], expect).verdict, "unread");
  const claim = parseClaim(concat(["0x45574e31", numberToHex(4663, { size: 4 }), V8ADDR.wrapper, it.recipient]));
  assert.equal(claim?.chainId, 4663); assert.equal(claim?.wrapper, V8ADDR.wrapper); assert.equal(claim?.recipient, it.recipient);
  assert.equal(parseClaim("0x45574e31"), null);
});
test("reserves: the canister's records are labelled and only ever warn (P4)", async () => {
  const svc = new V8Service({ config: cfg, fetch: v8Fetch(w), now: () => GEN + 30,
    canisterRecords: async () => ({ ...w.canister, buys: w.canister.buys.filter((b) => b.phase !== "Settled") }) });
  const r = (await svc.reserve("milady"))!;
  assert.equal(r.status, "green", "the canister's word cannot turn anything red");
  assert.match(r.warnings.join(" "), /no matching record/);
  assert.match(r.canisterNote, /never turn anything green/);
});
test("reserves: the indexer's report — events folded, costs, the advance, the gas pot, the stray", async () => {
  const r = (await service().reserve("milady"))!;
  assert.equal(r.status, "green", r.reasons.join("; "));
  assert.equal(r.advance?.open, true); assert.match(r.advance!.where, /Ethereum, earmarked/);
  assert.equal(r.gas?.potWei, "140000000000000000");
  assert.equal(r.strays.length, 1); assert.equal(r.strays[0].tokenId, "399");
  assert.ok(r.items.filter((i) => i.how === "purchase").every((i) => i.cost !== null), "each purchase has its AdvanceSettled");
  assert.ok(r.items.some((i) => i.promisedTo === 7), "the item drawn by ticket 7 is shown promised");
  assert.ok(r.providers.eth.every((p) => !p.includes("/")), "providers are published as hosts only");
});
test("client code refuses a keyed provider URL", () => {
  assert.equal(looksKeyed("https://ethereum-rpc.publicnode.com"), false);
  assert.equal(looksKeyed("https://xrpc.cl/eth"), false);
  assert.equal(looksKeyed("https://api-ethereum-mainnet.n.dwellir.com/2f9c6a7e-1b2c-4d5e-8f90-1234567890ab"), true);
  assert.equal(looksKeyed("https://mainnet.infura.io/v3/0123456789abcdef0123456789abcdef"), true);
  assert.equal(looksKeyed("https://rpc.example.com/?apikey=x"), true);
  assert.throws(() => reader("https://eth-mainnet.g.alchemy.com/v2/abcDEF0123456789abcdEF", "ethereum", fetch as never, null), /key/);
});
test("the v8 config is parsed strictly", () => {
  assert.doesNotThrow(() => parseV8Config(cfg));
  assert.throws(() => parseV8Config({ collections: [{ ...cfg.collections[0], wrapper: "0x12" }] }), /wrapper/);
  assert.throws(() => parseV8Config({ collections: [cfg.collections[0], cfg.collections[0]] }), /duplicate/);
});

// ---- the snapshot job and the CLI ------------------------------------------------------------------------------------------------

test("snapshot job: tokenURI on Ethereum, IPFS through a gateway, an http(s) server as is; traits only; whole or not taken", async () => {
  const { takeSnapshot, metadataUrls, ipfsCidOf } = await import("../src/v8/snapshotJob.js");
  const { encodeFunctionResult, parseAbi } = await import("viem");
  const abi = parseAbi(["function tokenURI(uint256 id) view returns (string)"]);
  const served = new Map<string, unknown>([["https://ipfs.io/ipfs/QmCID/0", { name: "a", attributes: [{ trait_type: "Hat", value: "Cap" }, { trait_type: "Level", value: 3 }, { bad: 1 }] }],
    ["https://ipfs.io/ipfs/QmCID/1", { attributes: [{ trait_type: "Hat", value: "Bow" }] }]]);
  const fake = async (url: string, init?: { body?: string }) => {
    const res = (o: unknown, status = 200) => ({ ok: status < 400, status, text: async () => JSON.stringify(o), json: async () => o });
    if (url.startsWith("https://eth.fixture.invalid")) {
      const { id, params } = JSON.parse(init!.body!); const tokenId = BigInt(`0x${String(params[0].data).slice(10)}`);
      return res({ jsonrpc: "2.0", id, result: encodeFunctionResult({ abi, functionName: "tokenURI", result: `ipfs://QmCID/${tokenId}` }) });
    }
    return served.has(url) ? res(served.get(url)) : res({}, 404);
  };
  const { snapshot, sha256 } = await takeSnapshot({ collection: V8ADDR.collection, name: "t", tokenIds: ["0", "1"], ethRpc: "https://eth.fixture.invalid", fetch: fake as never, now: 5 });
  assert.deepEqual(snapshot.tokens, [{ tokenId: "0", traits: { Hat: "Cap", Level: "3" } }, { tokenId: "1", traits: { Hat: "Bow" } }]);
  assert.equal(snapshot.source.ipfsCid, "QmCID"); assert.equal(snapshot.source.tokenURI, "ipfs://QmCID/{id}"); assert.equal(sha256, snapshotHash(snapshot));
  await assert.rejects(takeSnapshot({ collection: V8ADDR.collection, name: "t", tokenIds: ["0", "2"], ethRpc: "https://eth.fixture.invalid", fetch: fake as never, now: 5 }), /could not be read/);
  assert.deepEqual(metadataUrls("https://www.miladymaker.net/milady/json/1"), ["https://www.miladymaker.net/milady/json/1"]);
  assert.equal(ipfsCidOf("https://x/1"), null);
});
test("the CLI verifies a published draws report offline, refuses a tampered one, and checks a snapshot's hash", async () => {
  const { spawnSync } = await import("node:child_process");
  const { mkdtempSync, readFileSync, writeFileSync } = await import("node:fs");
  const { tmpdir } = await import("node:os");
  const cli = (args: string[]) => spawnSync(process.execPath, ["--import", "tsx", new URL("../scripts/rook-verify.ts", import.meta.url).pathname, ...args], { encoding: "utf8" });
  const dir = mkdtempSync(`${tmpdir()}/v8cli-`), report = (await service().draws("milady"))!;
  writeFileSync(`${dir}/ok.json`, JSON.stringify(report));
  const ok = cli(["draws", "--report", `${dir}/ok.json`, "--offline"]);
  assert.equal(ok.status, 0, ok.stdout + ok.stderr); assert.match(ok.stdout, /ticket 4: VERIFIED/); assert.match(ok.stdout, /timing: flagged/);
  const bad = structuredClone(report); bad.proofs[3].itemId = bad.proofs[3].freeSet.find((x) => x !== bad.proofs[3].itemId)!;
  writeFileSync(`${dir}/bad.json`, JSON.stringify(bad));
  const no = cli(["draws", "--report", `${dir}/bad.json`, "--offline"]);
  assert.equal(no.status, 1); assert.match(no.stdout, /MISMATCH/);
  writeFileSync(`${dir}/snap.json`, JSON.stringify(w.snapshot));
  assert.equal(cli(["snapshot", "--file", `${dir}/snap.json`, "--sha256", snapshotHash(w.snapshot)]).status, 0);
  assert.equal(cli(["snapshot", "--file", `${dir}/snap.json`, "--sha256", `0x${"0".repeat(64)}`]).status, 1);
  assert.equal(cli(["nonsense"]).status, 64);
  void readFileSync;
});
