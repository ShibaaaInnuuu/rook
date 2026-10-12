// Wave 2: the keeper's and the resolver's liveness from 4663 state (chain.mjs), the other services' heartbeats (liveness.mjs), and
// the canister's own health (custodyhealth.mjs), including the records and types the audit-D fixes added. No network.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { describe } from "../../scripts/timelock-watch.mjs";
import { roundTime } from "../src/abi.mjs";
import { judgeAddress } from "../src/canary.mjs";
import { decodeInit, decodeReply } from "../src/candid.mjs";
import { diffInit } from "../src/argdiff.mjs";
import { checkChain } from "../src/chain.mjs";
import { judgeCanister } from "../src/custodyhealth.mjs";
import { checkLiveness } from "../src/liveness.mjs";
import { EMPTY_RECORDS, recordIndex } from "../src/records.mjs";
import { fakeChain } from "./fakechain.mjs";

const S = JSON.parse(readFileSync(new URL("../fixtures/stack-logs.json", import.meta.url), "utf8"));
const F = JSON.parse(readFileSync(new URL("../fixtures/canister.json", import.meta.url), "utf8"));
const OLD = JSON.parse(readFileSync(new URL("../fixtures/canister-pre-audit-d.json", import.meta.url), "utf8"));
const c = S.stack.collections[0], NOW = 1_790_000_000_000;

/** A 4663 state: `a` is RPC A's answers, `b` overrides for RPC B. */
function state(a, b = {}) {
  const draw = (x) => ({ stage: 0, coin: S.coin, challengeEnds: 0n, challengeEndsBlock: 0n, round: 0n, pinnedAt: 0n, pinnedAtBlock: 0n, resolvedAt: 0n, resolvedAtBlock: 0n,
    root: `0x${"00".repeat(32)}`, totalTickets: 0n, winningTicket: 0n, winner: "0x0000000000000000000000000000000000000000", uri: "", seedRound: 0n, announcedAt: 0n, announcedAtBlock: 0n, snapshotAt: 0n, ...x });
  const ticket = (x) => ({ stage: 0, owner: "0x0000000000000000000000000000000000000001", round: 0n, pinnedAt: 0n, pinnedAtBlock: 0n, deliverBy: 0n, deliverByBlock: 0n,
    acquisitionId: 0n, deliveryTxid: `0x${"00".repeat(32)}`, destination: "", ...x });
  const base = { keeperPaused: false, fresh: true, escaped: false, postedAt: BigInt(NOW / 1000 - 600), lastGoodAt: BigInt(NOW / 1000 - 600), capFresh: true,
    nextToResolve: 0n, ticketCount: 0n, tickets: [], drawCount: 0n, draws: [] };
  const answer = (o) => (to, fn, args) => {
    if (fn === "ticket") return ticket(o.tickets[Number(args[0])]);
    if (fn === "draw") return draw(o.draws[Number(args[0])] ?? {});
    if (fn === "drawCount") return to === c.raffleVault ? o.drawCount : 0n;
    return o[fn];
  };
  const A = { ...base, ...a }, B = { ...A, ...b };
  return fakeChain({ views: (url, to, fn, args) => (url.includes("//a.") ? answer(A) : answer(B))(to, fn, args) });
}
const cfg = { rpcs: ["https://a.example", "https://b.example"], stack: S.stack };
const run = async (w, prev, now = NOW) => checkChain({ cfg, prev, now, http: w.http });

test("a healthy stack pages nothing", async () => {
  const r = await run(state({}));
  assert.deepEqual(r.alerts, []);
});

test("a configured guardian the registry does not name is paged (it could not act)", async () => {
  const g = { ...cfg, guardian: { address: S.guardian } };
  assert.deepEqual((await checkChain({ cfg: g, prev: {}, now: NOW, http: state({ guardian: S.guardian }).http })).alerts, []);
  const r = await checkChain({ cfg: g, prev: {}, now: NOW, http: state({ guardian: "0x0000000000000000000000000000000000000000" }).http });
  assert.deepEqual(r.alerts.map((a) => [a.kind, a.sev]), [["guardian", "SEV2"]]);
  assert.match(r.alerts[0].text, /CANNOT pause the keeper or cancel a draw/);
});

test("the keeper paused: SEV1 at once, persistent, confirmed on the second RPC; a second RPC that disagrees is a disagreement, not a page", async () => {
  const r = await run(state({ keeperPaused: true }));
  assert.deepEqual(r.alerts.map((a) => [a.kind, a.sev, a.persistent]), [["keeper", "SEV1", true]]);
  assert.match(r.alerts[0].text, /THE KEEPER IS PAUSED/);
  const d = await run(state({ keeperPaused: true }, { keeperPaused: false }));
  assert.deepEqual(d.alerts.map((a) => [a.kind, a.sev]), [["provider-disagreement", "SEV2"]]);
});

test("a stale floor pages after 15 min (not before), no good post after 45 min, the stale fallback at once, a stale cap after 15 min", async () => {
  const stale = state({ fresh: false, postedAt: BigInt(NOW / 1000 - 4000), lastGoodAt: BigInt(NOW / 1000 - 4000) });
  const first = await run(stale);
  assert.deepEqual(first.alerts.map((a) => a.key.split(":")[0]), ["no-good-post"], "no good post for 66 min pages; the stale floor starts its 15 minutes");
  const later = await run(stale, first.next, NOW + 16 * 60_000);
  assert.ok(later.alerts.some((a) => a.key.startsWith("floor-stale") && a.sev === "SEV2" && /floor is STALE/.test(a.text)));
  const esc = await run(state({ fresh: false, escaped: true, lastGoodAt: 1n }));
  assert.ok(esc.alerts.some((a) => a.sev === "SEV1" && /FloorModule is ESCAPED/.test(a.text)));
  const cap = state({ capFresh: false });
  const c1 = await run(cap);
  assert.deepEqual(c1.alerts, []);
  const c2 = await run(cap, c1.next, NOW + 16 * 60_000);
  assert.deepEqual(c2.alerts.map((a) => /RAFFLE vault's cap is STALE|BURN vault's cap is STALE/.test(a.text)), [true, true]);
});

test("a ticket Pinned an hour past its round, a draw Announced for an hour, a draw Pinned an hour past its round: each SEV2 (nobody resolves)", async () => {
  const round = BigInt(Math.floor((NOW / 1000 - 2 * 3600 - 1_727_521_075) / 3) + 1);
  assert.ok(roundTime(round) <= NOW / 1000 - 2 * 3600 + 3);
  const w = state({ nextToResolve: 3n, ticketCount: 5n, tickets: { 3: { stage: 2 }, 4: { stage: 1, round } },
    drawCount: 3n, draws: [{ stage: 4 }, { stage: 7, announcedAt: BigInt(NOW / 1000 - 7200) }, { stage: 2, round }] });
  const r = await run(w);
  const texts = r.alerts.map((a) => a.text);
  assert.equal(r.alerts.length, 3, texts.join("\n"));
  assert.ok(texts.some((t) => /redemption ticket 4 is still PINNED 2\.0 h after its drand round/.test(t)));
  assert.ok(texts.some((t) => /RAFFLE draw 1 was ANNOUNCED 2\.0 h ago and is not open/.test(t)));
  assert.ok(texts.some((t) => /RAFFLE draw 2 is still PINNED/.test(t)));
  assert.equal(r.next.drawFrom[c.raffleVault], 1, "the next poll starts at the lowest draw not yet final");
});

test("liveness: a service not answering 200 for 10 min is SEV2 (repeated), its return is said once", async () => {
  let status = 503;
  const http = async () => ({ status });
  const lcfg = { liveness: [{ name: "keeper", url: "https://keeper.example/healthz" }] };
  const a = await checkLiveness({ cfg: lcfg, prev: {}, now: NOW, http });
  assert.deepEqual(a.alerts, []);
  const b = await checkLiveness({ cfg: lcfg, prev: a.next, now: NOW + 11 * 60_000, http });
  assert.deepEqual(b.alerts.map((x) => [x.kind, x.sev, x.persistent]), [["liveness", "SEV2", true]]);
  assert.match(b.alerts[0].text, /keeper is DOWN or refusing: .* for 11 min \(HTTP 503\)/);
  status = 200;
  const up = await checkLiveness({ cfg: lcfg, prev: b.next, now: NOW + 12 * 60_000, http });
  assert.deepEqual(up.alerts.map((x) => x.sev), ["INFO"]);
  const unreachable = await checkLiveness({ cfg: lcfg, prev: { keeper: { downSince: NOW - 20 * 60_000, paged: true } }, now: NOW, http: async () => { throw new Error("ECONNREFUSED"); } });
  assert.match(unreachable.alerts[0].text, /ECONNREFUSED/);
});

// ---- the canister (fixes-D-canister.md §6) ----------------------------------------------------------------------------------

test("the decoder knows the new Init fields: switching holder deposits on is CRITICAL, the scanner's first block a WARN; an argument from before still decodes", () => {
  const on = decodeInit(F.args.deposits_on), base = decodeInit(F.args.install);
  assert.equal(on.ok, true, on.error);
  const { lines, unchanged } = diffInit(base.init, on.init);
  assert.deepEqual(lines.map((l) => [l.sev, l.verb, l.text.split(":")[0]]), [["CRITICAL", "LOOSENS", "holder_deposits"], ["WARN", "CHANGES", "wrapper_from_block"]]);
  assert.ok(unchanged.includes("operator"));
  const old = decodeInit(OLD.args.install);
  assert.equal(old.ok, true, "the audited build's argument (no new fields) decodes");
  assert.deepEqual(diffInit(old.init, base.init).lines, [], "and it is the same configuration: null is null");
});

test("the canary vouches for every txid the canister saved: a delivery's earlier (re-priced) version, a conflict-out, a float abandon", () => {
  const ESPL = JSON.parse(readFileSync(new URL("../fixtures/esplora/mempool-address-txs.json", import.meta.url), "utf8"));
  const ADDR = "bc1pfssfalzdt9h8uwqcrs73780l723mh2axrul7keaevke7a8pg4a7szrd0ht";
  const views = [{ name: "mempool.space", base: "https://mempool.space/api", txs: ESPL }, { name: "blockstream.info", base: "https://blockstream.info/api", txs: ESPL }];
  const judgeWith = (r) => judgeAddress({ label: "custody", address: ADDR, views, index: recordIndex({ ...EMPTY_RECORDS, ...r }), now: NOW }).alerts.filter((a) => a.kind === "custody-outbound");
  assert.equal(judgeWith({}).length, 1, "with no record the spend is an unexpected outbound");
  assert.deepEqual(judgeWith({ redemptions: decodeReply("redemptions", F.replies.redemptions_bumped) }), [], "the confirmed EARLIER version of a bumped delivery");
  assert.deepEqual(judgeWith({ redemptions: decodeReply("redemptions", F.replies.redemptions_conflicting) }), [], "a delivery's conflict-out");
  assert.deepEqual(judgeWith({ frog_buys: decodeReply("frog_buys", F.replies.frog_buys_abandoning) }), [], "the float's abandon conflict");
  const idx = recordIndex({ ...EMPTY_RECORDS, redemptions: decodeReply("redemptions", F.replies.redemptions_bumped) });
  assert.deepEqual(idx.deliveries.get("8").txids.length, 2, "a Delivered event may name either version");
});

test("a protected Ordinals Wallet sale handed back (docs/v8/ow-protected.md): the new frog phase decodes and its sale is ours", () => {
  const frogs = decodeReply("frog_buys", F.replies.frog_buys_handed);
  const phase = frogs[0].phase.Handed;
  assert.ok(phase, "Handed decodes");
  assert.equal(phase.at, "1790000000");
  assert.equal(phase.sale_txid, phase.buy.outpoint.txid, "watched by the sale's txid, display order");
  const idx = recordIndex({ ...EMPTY_RECORDS, frog_buys: frogs });
  assert.match(idx.byTxid.get(phase.sale_txid).what, /frog purchase #4 \(Handed\)/);
});

test("canister health: the scanner failing for an hour, TICKET LOGS NOT READ, no UniSat, a parked deposit", () => {
  const tl = decodeReply("ticket_logs", F.replies.ticket_logs_failing);
  const read = { ticketLogs: tl, log: ["custody: TICKET LOGS NOT READ (7 tries): rpc.ordofi.network: -32005", "tick: fine"],
    ordProviders: [["ord", "https://ordinals.com"]], holderDeposits: decodeReply("holder_deposits", F.replies.holder_deposits_parked) };
  const a = judgeCanister({ buyer: "b", read, now: NOW });
  assert.deepEqual(a.alerts.map((x) => [x.kind, x.sev, x.key.split(":")[0]]), [["canister", "SEV2", "ticket-logs-line"], ["canister", "SEV2", "unisat-missing"], ["canister", "INFO", "parked"]]);
  const b = judgeCanister({ buyer: "b", read, prev: a.next, now: NOW + 61 * 60_000 });
  assert.ok(b.alerts.some((x) => x.key.startsWith("ticket-logs-failing") && /FAILED 7 tick\(s\) in a row, for 61 min: rpc\.ordofi\.network: -32005/.test(x.text)));
  const fine = judgeCanister({ buyer: "b", read: { ticketLogs: [], log: [], ordProviders: [["unisat", "https://open-api.unisat.io"]], holderDeposits: [] }, now: NOW });
  assert.deepEqual(fine.alerts, []);
});

test("the limiter's Return flow and acknowledged keys decode; the timelock's NotStopped / NotStarted have words", () => {
  const [, outflow] = decodeReply("custody_outflow", F.replies.custody_outflow_frozen_acked);
  assert.equal(outflow.acked[0].length, 2);
  assert.deepEqual(Object.keys(outflow.acked[0][1].flow), ["Return"]);
  assert.match(describe({ NotStarted: { id: "4", at_ns: "1790000000000000000", why: "stopped" } }, {}), /did NOT START again.*start_target/);
  assert.match(describe({ NotStopped: { id: "4", at_ns: "1790000000000000000", why: "busy" } }, {}), /did NOT run: the buyer did not stop \(busy\)/);
});
