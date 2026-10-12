// Integration R3 (docs/audit/2026-09-26/review2/integration-R3.md): what the monitor learned from the three round-3 branches built in
// parallel. The contracts (r3-contracts): SeedCarried and the void of an Announced draw page, a carried snapshot window stays in the
// holder index, exitAtBacking is shown while the keeper is paused, and the Wrapper's custodian-only markUndeliverable is matched
// against the canister's own attestation. The canister (r3-canister, fixes-R3-canister.md §9): its new optional fields, decoded
// from the canister's OWN encoding (fixtures/canister.json, cargo run --example watch_fixtures), and its alert lines. No network.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { encodeAbiParameters, encodeEventTopics } from "viem";
import { IDL } from "@icp-sdk/core/candid";
import { Principal } from "@icp-sdk/core/principal";
import { describe } from "../../scripts/timelock-watch.mjs";
import { VAULT, retryPolicy } from "../src/abi.mjs";
import { decodeReply, plain, timelockIdl } from "../src/candid.mjs";
import { checkChain } from "../src/chain.mjs";
import { ATTEST_LATE_MS, CONFLICT_CEILING_SATS, DEFAULT_MIN_FEE_POOL_SATS, checkCanister, judgeCanister } from "../src/custodyhealth.mjs";
import { checkEvm, judgeLogs, judgeStackLogs, TOPICS } from "../src/evm.mjs";
import { HolderIndex } from "../src/holders.mjs";
import { EMPTY_RECORDS, recordIndex } from "../src/records.mjs";
import { stackOf } from "../src/stack.mjs";
import { validate } from "../src/config.mjs";
import { fakeChain, hex } from "./fakechain.mjs";

retryPolicy.baseMs = 0;
const S = JSON.parse(readFileSync(new URL("../fixtures/stack-logs.json", import.meta.url), "utf8"));
const F = JSON.parse(readFileSync(new URL("../fixtures/canister.json", import.meta.url), "utf8"));
const st = stackOf({ stack: S.stack }), c = st.collections[0];
const NOW = 1_790_000_000_000, ZERO = "0x0000000000000000000000000000000000000000";
const OUT_TXID = "2127bc9003b23d8dd342a60cfe9cb44383288c0581a49aac35e513dc8e0fce57";
const lc = (a) => String(a).toLowerCase();
const log = (address, topics, data = "0x", i = 0) => ({ address, topics, data, blockNumber: hex(71_300_000 + i), transactionHash: `0x${(i + 1).toString(16).padStart(64, "0")}`,
  logIndex: hex(i), blockHash: `0x${(i + 9).toString(16).padStart(64, "0")}`, removed: false });

// ---- the contracts' side --------------------------------------------------------------------------------------------------------

test("SeedCarried pages SEV2 (the draw reuses its origin's snapshot time); DrawVoided of an Announced draw pages; a void of a resolved one does not", () => {
  const ann = log(c.raffle, encodeEventTopics({ abi: VAULT, eventName: "DrawAnnounced", args: { drawId: 4n, coin: S.coin } }), encodeAbiParameters([{ type: "uint64" }], [123n]), 1);
  const carried = log(c.raffle, encodeEventTopics({ abi: VAULT, eventName: "SeedCarried", args: { drawId: 5n, originId: 4n } }), encodeAbiParameters([{ type: "uint64" }], [123n]), 3);
  const voided = log(c.raffle, encodeEventTopics({ abi: VAULT, eventName: "DrawVoided", args: { drawId: 4n } }), "0x", 2);
  const j = judgeStackLogs({ logs: [ann, voided, carried], ctx: { st, now: NOW } });
  const byKey = Object.fromEntries(j.alerts.map((a) => [a.key.split(":")[0], a]));
  assert.equal(byKey["draw-voided-announced"].sev, "SEV2");
  assert.match(byKey["draw-voided-announced"].text, /VOIDED while still Announced.*carries its seed round and snapshot time/s);
  assert.equal(byKey["seed-carried"].sev, "SEV2");
  assert.match(byKey["seed-carried"].text, /draw 5 CARRIES draw 4's seed round 123 and snapshot time/);
  assert.equal(j.next.draws[`${lc(c.raffle)}:5`].origin, "4");
  // a draw the monitor saw resolved, then voided a week later: no page
  const prev = { draws: { [`${lc(c.raffle)}:4`]: { status: "verified" } } };
  const k = judgeStackLogs({ logs: [voided], ctx: { st, now: NOW }, prev });
  assert.deepEqual(k.alerts, []);
});

test("the draws' state: a carried Announced draw pages from the vault's state alone, and the holder indexes hold its ORIGIN's window", async () => {
  const originAt = BigInt(NOW / 1000 - 2 * 86_400);                           // announced two days ago: its window is past the 36 h retention
  const view = (x) => ({ stage: 7, coin: S.coin, challengeEnds: 0n, challengeEndsBlock: 0n, round: 0n, pinnedAt: 0n, pinnedAtBlock: 0n, resolvedAt: 0n, resolvedAtBlock: 0n,
    root: `0x${"00".repeat(32)}`, totalTickets: 0n, winningTicket: 0n, winner: ZERO, uri: "", seedRound: 77n, announcedAt: 0n, announcedAtBlock: 0n, snapshotAt: 0n, ...x });
  const draws = [view({ stage: 6, announcedAt: originAt }), view({ stage: 7, announcedAt: BigInt(NOW / 1000 - 600) })];
  const w = fakeChain({
    views: (url, to, fn, args) => {
      if (fn === "drawCount") return to === lc(c.raffle) ? BigInt(draws.length) : 0n;
      if (fn === "draw") return draws[Number(args[0])];
      if (fn === "seedOrigin") return Number(args[0]) === 1 ? 0n : BigInt(args[0]);
      if (fn === "carriedSeed") return 0n;
      throw new Error(`no view ${fn}`);
    },
    other: (url, method, params) => {
      if (method === "eth_blockNumber") return hex(71_500_000);
      if (method === "eth_getLogs") return [];
      if (method === "eth_getBlockByNumber") return { timestamp: hex(NOW / 1000 - (71_500_000 - Number(BigInt(params[0])))) };  // a block a second
      throw new Error(`unexpected ${method}`);
    },
  });
  const seen = [];
  const ix = { url: "https://a.example", safe: 71_499_980, addCoin: () => true, setHolds: (h) => seen.push(h), summary: () => ({}), save: () => {},
    update: async () => ({ safe: 71_499_980 }), rootAt: async () => { throw new Error("not open"); } };
  const r = await checkEvm({ cfg: { rpcs: ["https://a.example", "https://b.example"], stack: S.stack, from_block: 71_200_000, confirmations: 20 }, prev: { cursor: 71_499_900 },
    now: NOW, http: w.http, readRecords: async () => EMPTY_RECORDS, timelock: "t", indexes: [ix] });
  const carried = r.alerts.find((a) => a.key === `seed-carried:${lc(c.raffle)}:1`);
  assert.ok(carried, r.alerts.map((a) => a.key).join(", "));
  assert.match(carried.text, /draw 1 CARRIES draw 0's seed round 77/);
  const h = r.next.holds[lc(S.coin)];
  assert.equal(h.originAt, Number(originAt), "the hold counts back from the ORIGIN's announcement, not the carrier's");
  // the block of (originAt − 25 h) on the fake's one-block-a-second clock
  const want = 71_500_000 - (NOW / 1000 - (Number(originAt) - 25 * 3600));
  assert.equal(h.block, want);
  assert.deepEqual(seen.at(-1), { [lc(S.coin)]: want }, "every index is told to keep that window");
});

test("the holder index: a held coin is never folded past its hold, and a coin already folded past a new hold is rebuilt from its launch", () => {
  const ix = new HolderIndex({ url: "https://a.example", fromBlock: 100, retentionBlocks: 1_000 });
  ix.addCoin(S.coin, { launchBlock: 100 });
  const k = lc(S.coin), coin = ix.s.coins[k];
  coin.logs = [[200, 0, ZERO, "0x00000000000000000000000000000000000000a1", "5000000000000000000"], [5_000, 0, "0x00000000000000000000000000000000000000a1", "0x00000000000000000000000000000000000000a2", "2000000000000000000"]];
  coin.cursor = 10_000;
  ix.setHolds({ [k]: 4_000 });
  ix.compact(10_000);                                                          // the retention alone would fold up to block 9,000
  assert.equal(ix.s.coins[k].baseBlock, 3_999, "folded only up to the hold");
  assert.equal(ix.balancesAt(S.coin, 4_500).get("0x00000000000000000000000000000000000000a1"), 5_000_000_000_000_000_000n, "the carried window is still readable");
  ix.setHolds({});
  ix.compact(10_000);
  assert.equal(ix.s.coins[k].baseBlock, 9_000, "no hold: the retention");
  ix.setHolds({ [k]: 4_000 });
  assert.equal(ix.s.coins[k].cursor, 99, "folded past the hold: rebuilt from the launch (re-read next poll)");
  assert.equal(ix.s.coins[k].rebuiltForHold, 4_000);
});

test("the keeper paused: the page says what each module's burn pays (r4-contracts: pauseFloor while paused, the backing only once escaped)", async () => {
  const base = { keeperPaused: true, fresh: false, escaped: false, postedAt: BigInt(NOW / 1000 - 600), lastGoodAt: BigInt(NOW / 1000 - 600), capFresh: true,
    nextToResolve: 0n, ticketCount: 0n, drawCount: 0n, exitAtBacking: false, burnsOpen: true, pauseFloor: 52n * 10n ** 16n };
  const page = async (over) => {
    const w = fakeChain({ views: (url, to, fn) => { const v = { ...base, ...over }; if (fn in v) return v[fn]; throw new Error(`no view ${fn}`); } });
    return (await checkChain({ cfg: { rpcs: ["https://a.example", "https://b.example"], stack: S.stack }, prev: {}, now: NOW, http: w.http })).alerts.find((a) => /THE KEEPER IS PAUSED/.test(a.text)).text;
  };
  const p = await page({});
  assert.match(p, /Burns continue, less the base spread, from the moment of the pause: at the floor the keeper left \(at most the backing per unit/);
  assert.match(p, new RegExp(`${c.slug} burns at the frozen floor 0.52 ETH or its backing per unit, whichever is lower \\(pauseFloor\\)`));
  assert.doesNotMatch(p, /burns at the backing per unit \(a week/, "exitAtBacking is false while paused (escaped() alone): never read as 'at the backing'");
  assert.match(await page({ escaped: true, exitAtBacking: true, pauseFloor: 0n }), new RegExp(`${c.slug} burns at the backing per unit \\(a week with no good post: exitAtBacking\\)`));
  assert.match(await page({ pauseFloor: 0n, burnsOpen: false }), new RegExp(`${c.slug} DOES NOT BURN`));
});

// ---- the Wrapper and the canister: markUndeliverable ------------------------------------------------------------------------------

test("Undeliverable is matched against the canister's attestation: a refused redemption's Sent or Done passes; Due, none, or not refused is SEV1", () => {
  const index = recordIndex({ ...EMPTY_RECORDS, redemptions: decodeReply("redemptions", F.replies.redemptions_attested) });
  assert.deepEqual([...index.attests.keys()], ["11", "12", "13"]);
  assert.deepEqual(index.attests.get("11"), { state: "Sent", hash: "ab".repeat(32), note: null, refused: true });
  const und = (t, i) => log(c.wrapper, [TOPICS.Undeliverable, `0x${t.toString(16).padStart(64, "0")}`], "0x", i);
  const j = judgeLogs({ logs: [und(11, 1), und(12, 2), und(13, 3), und(99, 4)], index, cfg: { wrappers: [c.wrapper] } });
  assert.deepEqual(j.alerts.map((a) => [a.sev, /ticket (\d+)/.exec(a.text)[1]]), [["SEV1", "13"], ["SEV1", "99"]]);
  assert.match(j.alerts[0].text, /AN ATTESTATION NO REFUSAL BACKS.*attestation Due/s);
  assert.match(j.alerts[1].text, /does not exist/);
});

test("a conflict-out re-priced: the canary matches every version (the earlier one, OUT_TXID, is what confirmed)", () => {
  const rs = decodeReply("redemptions", F.replies.redemptions_conflict_repriced);
  const tx = rs[0].phase.Conflicting.conflict.tx[0];
  assert.deepEqual([tx.signed_at, tx.earlier], [["1790007200"], [[OUT_TXID]]]);
  assert.deepEqual(rs[0].attest, [{ Due: { deep: ["99".repeat(32)] } }], "the attestation waits for the conflict to be deep");
  const index = recordIndex({ ...EMPTY_RECORDS, redemptions: rs });
  assert.equal(index.byTxid.get(OUT_TXID).kind, "conflict");
  assert.match(index.byTxid.get(OUT_TXID).what, /an earlier version/);
  assert.equal(index.byTxid.get("99".repeat(32)).kind, "conflict");
  assert.equal(index.attests.get("10").state, "Due");
});

// ---- the canister's health: the new fields and lines ------------------------------------------------------------------------------

test("the canister's new fields decode from its own encoding and page: an unreadable ticket, a block read by ticket, a quarantined fee entry", () => {
  const t = decodeReply("ticket_logs", F.replies.ticket_logs_dense);
  assert.deepEqual([t[0].cap, t[0].dense[0].block, t[0].unreadable], [["400000"], "71300001", [[["71300001", "777"]]]]);
  const pool = decodeReply("custody_fee_pool", F.replies.custody_fee_pool);
  assert.equal(pool.quarantined[0][0].fails[0], 3);
  const ok = [["unisat", "https://open-api.unisat.io"]];
  const r = judgeCanister({ buyer: "b", read: { ticketLogs: t, log: [], ordProviders: ok, holderDeposits: [], feePool: pool, redemptions: [] }, now: NOW });
  assert.deepEqual(r.alerts.map((a) => [a.sev, a.key.split(":")[0]]), [["SEV1", "ticket-logs-unreadable"], ["INFO", "ticket-logs-dense"], ["SEV2", "fee-pool-quarantined"]]);
  // operator 2026-09-26: the minimum is 20,000 sats (min_fee_pool_sats); under it pages SEV2
  const judge = (value, o = {}) => judgeCanister({ buyer: "b", read: { ticketLogs: [], log: [], ordProviders: ok, holderDeposits: [],
    feePool: { entries: [{ ...pool.entries[0], value }], quarantined: [] }, redemptions: [] }, now: NOW, ...o });
  const s = judge("19999");
  assert.deepEqual(s.alerts.map((a) => [a.sev, a.key.split(":")[0]]), [["SEV2", "fee-pool-small"]]);
  assert.ok(s.alerts[0].persistent);
  assert.match(s.alerts[0].text, /no usable entry of 20000 sats or more \(the configured minimum, min_fee_pool_sats; largest 19999\)/);
  // at or above the minimum, under the 100,000-sat ceiling: an INFO note, not a page
  for (const v of ["20000", "60000", "99999"]) {
    const n = judge(v);
    assert.deepEqual(n.alerts.map((a) => [a.sev, a.key.split(":")[0]]), [["INFO", "fee-pool-under-ceiling"]], v);
    assert.ok(!n.alerts[0].persistent);
    assert.match(n.alerts[0].text, /cannot reach its ceiling, so a stuck delivery is cancelled more slowly\. The gated Wrapper \(refunds only after markUndeliverable or 30 days\) keeps that from becoming a loss/);
  }
  assert.deepEqual(judge("100000").alerts, [], "an entry at the ceiling: nothing to say");
  // the minimum is configurable: the old rule is still one setting away
  assert.deepEqual(judge("60000", { minFeePoolSats: 100_000n }).alerts.map((a) => [a.sev, a.key.split(":")[0]]), [["SEV2", "fee-pool-small"]]);
  assert.deepEqual(judge("20000").alerts.map((a) => a.sev), ["INFO"]);
  assert.equal(DEFAULT_MIN_FEE_POOL_SATS, 20_000n);
  assert.equal(CONFLICT_CEILING_SATS, 100_000n, "the canister's ceiling is unchanged");
});

test("the fee pool's minimum is the config's min_fee_pool_sats (default 20000), read by the live check; a bad value is refused", async () => {
  const ok = [["unisat", "https://open-api.unisat.io"]];
  const pool = decodeReply("custody_fee_pool", F.replies.custody_fee_pool);
  const feePool = { entries: [{ ...pool.entries[0], value: "30000" }], quarantined: [] };
  const ic = { ticketLogs: async () => [], log: async () => [], ordProviders: async () => ok, holderDeposits: async () => [], feePool: async () => feePool, redemptions: async () => [] };
  const cfg = (extra = {}) => validate({ timelock: "rrkah-fqaaa-aaaaa-aaaaq-cai", buyer: "ryjl3-tyaaa-aaaaa-aaaba-cai", builds: [{ commit: "x", buyer: "0".repeat(64), timelock: "0".repeat(64) }], ...extra });
  assert.equal(cfg().min_fee_pool_sats, 20_000);
  assert.deepEqual((await checkCanister({ cfg: cfg(), now: NOW, ic })).alerts.map((a) => [a.sev, a.key.split(":")[0]]), [["INFO", "fee-pool-under-ceiling"]]);
  assert.deepEqual((await checkCanister({ cfg: cfg({ min_fee_pool_sats: 50_000 }), now: NOW, ic })).alerts.map((a) => [a.sev, a.key.split(":")[0]]), [["SEV2", "fee-pool-small"]]);
  for (const bad of [0, -1, 1.5, "20000"]) assert.throws(() => cfg({ min_fee_pool_sats: bad }), /min_fee_pool_sats: the custody fee pool's minimum largest entry/, String(bad));
});

test("the canister's alert lines page with what they mean; a markUndeliverable Due or Sent for a day pages, Done never", () => {
  const ok = [["unisat", "https://open-api.unisat.io"]];
  const lines = ["custody: PIN WATCH: output 0 of aa has an unconfirmed child bb; ", "FEE ESTIMATES DISAGREE: [3, 40] sat/vB; paying the upper median, 40",
    "FEE POOL: cc:1 quarantined after 3 failed reads while other outputs read fine (x); the operator removes it", "UNREADABLE ticket events (block, ticket): [(1, 2)]",
    "block 9 is read by ticket (through ticket Some(3))", "ticket 11: markUndeliverable mined (reverted: the ticket moved on)", "nothing to see"];
  const r = judgeCanister({ buyer: "b", read: { ticketLogs: [], log: lines, ordProviders: ok, holderDeposits: [] }, now: NOW });
  assert.deepEqual(r.alerts.map((a) => a.sev), ["SEV2", "SEV2", "SEV2", "SEV1", "INFO", "SEV2"]);
  assert.match(r.alerts[0].text, /PIN WATCH[\s\S]*a pin \(canister review2 N-2\)/);
  const rs = decodeReply("redemptions", F.replies.redemptions_attested);
  const a = judgeCanister({ buyer: "b", read: { ticketLogs: [], log: [], ordProviders: ok, holderDeposits: [], redemptions: rs }, now: NOW });
  assert.deepEqual(a.alerts, [], "not a day yet");
  const b = judgeCanister({ buyer: "b", read: { ticketLogs: [], log: [], ordProviders: ok, holderDeposits: [], redemptions: rs }, prev: a.next, now: NOW + ATTEST_LATE_MS });
  assert.deepEqual(b.alerts.map((x) => x.key), ["attest-late:b:11", "attest-late:b:13"], "Sent and Due; Done (12) never");
});

// ---- the timelock: Started and running ------------------------------------------------------------------------------------------

test("the timelock's Started event and Status.running decode (an unknown variant tag would fail the whole history) and have words", () => {
  const svc = timelockIdl();
  const ret = (name) => svc._fields.find(([n]) => n === name)[1].retTypes;
  const P = Principal.fromText("aaaaa-aa");
  // encoded with the timelock's own .did shape (etch_timelock.did): Started, and a Status with `running`
  const PV = IDL.Record({ id: IDL.Nat64, by: IDL.Principal, kind: IDL.Text, wasm_hash: IDL.Opt(IDL.Text), wasm_bytes: IDL.Opt(IDL.Nat64), arg_hex: IDL.Opt(IDL.Text),
    arg_hash: IDL.Opt(IDL.Text), controllers: IDL.Opt(IDL.Vec(IDL.Principal)), roles: IDL.Opt(IDL.Record({ proposers: IDL.Vec(IDL.Principal), guardians: IDL.Vec(IDL.Principal), veto_threshold: IDL.Nat8 })),
    proposed_ns: IDL.Nat64, executable_ns: IDL.Nat64, vetoes: IDL.Vec(IDL.Principal) });
  const Ev = IDL.Variant({ Started: IDL.Record({ at_ns: IDL.Nat64, by: IDL.Principal, ok: IDL.Bool, why: IDL.Opt(IDL.Text) }) });
  const hist = plain(IDL.decode(ret("history"), IDL.encode([IDL.Vec(Ev)], [[{ Started: { at_ns: 1_790_000_000_000_000_000n, by: P, ok: false, why: ["stopped"] } }]]))[0]);
  assert.equal(hist[0].Started.ok, false);
  assert.match(describe(hist[0], {}), /tried to start the buyer and it FAILED/);
  const Roles = IDL.Record({ proposers: IDL.Vec(IDL.Principal), guardians: IDL.Vec(IDL.Principal), veto_threshold: IDL.Nat8 });
  const St = IDL.Record({ target: IDL.Principal, roles: Roles, delay_secs: IDL.Nat64, min_delay_secs: IDL.Nat64, rehearsal_build: IDL.Bool, pending: IDL.Vec(PV),
    target_epoch: IDL.Nat64, roles_epoch: IDL.Nat64, resume_votes: IDL.Vec(IDL.Principal), now_ns: IDL.Nat64, running: IDL.Vec(IDL.Tuple(PV, IDL.Opt(IDL.Text))) });
  const pv = { id: 3n, by: P, kind: "upgrade", wasm_hash: [], wasm_bytes: [], arg_hex: [], arg_hash: [], controllers: [], roles: [], proposed_ns: 1n, executable_ns: 2n, vetoes: [] };
  const roles = { proposers: [P], guardians: [P], veto_threshold: 1 };
  const status = plain(IDL.decode(ret("status"), IDL.encode([St], [{ target: P, roles, delay_secs: 1n, min_delay_secs: 1n, rehearsal_build: false, pending: [], target_epoch: 0n,
    roles_epoch: 0n, resume_votes: [], now_ns: 0n, running: [[pv, ["vetoed"]]] }]))[0]);
  assert.equal(status.running[0][0][0].id, "3");
  assert.deepEqual(status.running[0][0][1], ["vetoed"]);
});
