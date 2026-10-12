// docs/v6/relay-conversion.md §6: the Relay conversions as the monitor sees them — the records, the three FloorModule events, the
// conversion states' alerts, the canary's known inbound, and the install argument's new field.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { IDL } from "@icp-sdk/core/candid";
import { diffInit } from "../src/argdiff.mjs";
import { decodeInit, decodeReply, encodeInitPlain, INIT_FIELDS, REPLY_TYPES } from "../src/candid.mjs";
import { ARRIVED_LATE_S, CONVERTING_LATE_S, FUNDED_LATE_S, conversionAlerts } from "../src/conversions.mjs";
import { judgeCanister } from "../src/custodyhealth.mjs";
import { TOPICS, judgeLogs } from "../src/evm.mjs";
import { EMPTY_RECORDS, recordIndex } from "../src/records.mjs";
import { judgeAddress } from "../src/canary.mjs";

const F = JSON.parse(readFileSync(new URL("../fixtures/canister.json", import.meta.url), "utf8"));
const MODULE = "0x" + "a1".repeat(20), OTHER = "0x" + "b2".repeat(20), WRAPPER = "0x" + "c3".repeat(20);
const NOW = 1_790_500_000;
const ADV = 650_000_000_000_000_000n;
const ARRIVAL_TXID = "7".repeat(64);
const b = (h) => Uint8Array.from(Buffer.from(h, "hex"));

/** A conversion record, encoded with the canister's Candid types and decoded as the actor would (plain JSON). */
function conv(frog, phase, extra = {}) {
  const v = { frog: BigInt(frog), module: b(MODULE.slice(2)), advanced: ADV, attempts: extra.attempts ?? 1, phase, orders: [], deposits: [],
    arrival: extra.arrival ? [{ outpoint: { txid: b(extra.arrival), vout: 1 }, sats: 2_000_000n }] : [], ask: [], note: extra.note ?? "", arrived_at: [], abandoned_sats: [], returned_at: [] };
  return v;
}
const reply = (recs) => decodeReply("frog_conversions", Buffer.from(IDL.encode(REPLY_TYPES.frog_conversions, [recs])).toString("hex"));
const word = (x) => BigInt(x).toString(16).padStart(64, "0");
const log = (name, from, words, i = 0, topics = []) => ({ address: from, topics: [TOPICS[name], ...topics], data: `0x${words.map(word).join("")}`,
  blockNumber: "0x10", logIndex: `0x${i.toString(16)}`, transactionHash: `0x${String(i).padStart(64, "e")}`, blockHash: `0x${"f".repeat(64)}`, removed: false });
const judge = (logs, conversions) => judgeLogs({ logs, index: recordIndex({ ...EMPTY_RECORDS, conversions }), cfg: { wrappers: [WRAPPER], floor_module: MODULE } });

test("the records: a conversion decodes from the canister's Candid, its arrival is a known inbound, its advance is indexed by module", () => {
  const recs = reply([conv(3, { Arrived: { order_id: b("ab".repeat(32)), sats: 2_000_000n } }, { arrival: ARRIVAL_TXID }),
    conv(4, { Converting: { order_id: b("cd".repeat(32)), value: ADV, min_sats: 1n, deadline: 2n, since: 3n, balance_after: 4n } })]);
  const idx = recordIndex({ ...EMPTY_RECORDS, conversions: recs });
  assert.equal(idx.inbound.get(ARRIVAL_TXID)?.what, "the Relay conversion for frog purchase #3 arriving at the float (Arrived)");
  assert.deepEqual(idx.advances, [{ frog: "3", module: MODULE, advanced: String(ADV), phase: "Arrived" }, { frog: "4", module: MODULE, advanced: String(ADV), phase: "Converting" }]);
  // the canary: a Relay fill to the float is not "unexplained"
  const tx = { txid: ARRIVAL_TXID, status: { confirmed: true, block_height: 900_001, block_hash: "0".repeat(64) }, vin: [{ prevout: { scriptpubkey_address: "bc1qrelay" } }],
    vout: [{ value: 2_000_000, scriptpubkey_address: "bc1pfloat" }] };
  const views = [{ name: "a", base: "https://a/api", txs: [tx] }, { name: "b", base: "https://b/api", txs: [tx] }];
  const a = judgeAddress({ label: "float", address: "bc1pfloat", views, index: idx, now: NOW * 1000 }).alerts;
  assert.ok(!a.some((x) => /unexplained/.test(x.text)), JSON.stringify(a));
  const bare = judgeAddress({ label: "float", address: "bc1pfloat", views, index: recordIndex(EMPTY_RECORDS), now: NOW * 1000 }).alerts;
  assert.ok(bare.some((x) => /unexplained INBOUND/.test(x.text)));
});

test("Advanced: backed by a conversion for that module with that advance; any other is SEV1", () => {
  const recs = reply([conv(3, { Funded: { wei: ADV, since: BigInt(NOW) } })]);
  assert.deepEqual(judge([log("Advanced", MODULE, [ADV, ADV])], recs).alerts, []);
  const wrongAmount = judge([log("Advanced", MODULE, [ADV + 1n, ADV])], recs).alerts;
  assert.equal(wrongAmount.length, 1);
  assert.equal(wrongAmount[0].sev, "SEV1");
  assert.match(wrongAmount[0].text, /AN ADVANCE NO CONVERSION BACKS/);
  assert.equal(judge([log("Advanced", MODULE, [ADV, ADV])], []).alerts.length, 1, "no record at all");
  const otherModule = reply([{ ...conv(3, { Funded: { wei: ADV, since: 1n } }), module: b(OTHER.slice(2)) }]);
  assert.equal(judge([log("Advanced", MODULE, [ADV, ADV])], otherModule).alerts.length, 1, "a record for another module");
  // one record backs one advance, not two
  assert.equal(judge([log("Advanced", MODULE, [ADV, ADV], 1), log("Advanced", MODULE, [ADV, ADV], 2)], recs).alerts.length, 1);
  // an event from an address that is not a watched module is not judged here
  assert.deepEqual(judge([log("Advanced", OTHER, [ADV, ADV])], []).alerts, []);
});

test("AdvanceSettled: only a conversion that arrived (or settled); AdvanceReturned: only one that is returning", () => {
  const arrived = reply([conv(3, { Arrived: { order_id: b("ab".repeat(32)), sats: 1n } })]);
  const settled = log("AdvanceSettled", MODULE, [ADV, 640_000_000_000_000_000n], 3, [`0x${word(5)}`]);
  assert.deepEqual(judge([settled], arrived).alerts, []);
  assert.deepEqual(judge([settled], reply([conv(3, { Settled: { cost: 1n, spare_sats: 2n } })])).alerts, []);
  const early = judge([settled], reply([conv(3, { Converting: { order_id: b("ab".repeat(32)), value: ADV, min_sats: 1n, deadline: 2n, since: 3n, balance_after: 4n } })])).alerts;
  assert.equal(early.length, 1);
  assert.match(early[0].text, /A SETTLEMENT NO ARRIVAL BACKS/);
  const returned = log("AdvanceReturned", MODULE, [ADV - 10n, ADV, 10n], 4);
  assert.deepEqual(judge([returned], reply([conv(3, { Returning: { wei: ADV - 10n } })])).alerts, []);
  assert.deepEqual(judge([returned], reply([conv(3, { Returned: { wei: ADV - 10n } })])).alerts, []);
  const noReturn = judge([returned], arrived).alerts;
  assert.equal(noReturn.length, 1);
  assert.match(noReturn[0].text, /A RETURN NO FAILED CONVERSION BACKS.*loss 0/);
});

test("the conversion states: late, stuck, unquoted, unbought, returned, refunded — each once, persistent where it waits on a person", () => {
  const plain = (recs) => reply(recs);
  const order = { order_id: b("ab".repeat(32)), value: ADV, min_sats: 500_000n, deadline: BigInt(NOW + 7 * 86_400) };
  const run = (recs, nowS = NOW, prev = {}) => conversionAlerts({ buyer: "buyer-TEST", conversions: plain(recs), nowS, prev });
  // Converting: quiet under 30 min, SEV2 at 30
  assert.deepEqual(run([conv(1, { Converting: { ...order, since: BigInt(NOW - CONVERTING_LATE_S + 60), balance_after: 0n } })]).alerts, []);
  const late = run([conv(1, { Converting: { ...order, since: BigInt(NOW - CONVERTING_LATE_S), balance_after: 0n } })]).alerts;
  assert.deepEqual(late.map((a) => [a.sev, a.persistent]), [["SEV2", true]]);
  assert.match(late[0].text, /LATE: the deposit to Relay mined 30 min ago/);
  // Stuck: SEV1
  const stuck = run([conv(1, { Stuck: { why: "the order's deadline passed with no fill and no refund" } })]).alerts;
  assert.deepEqual(stuck.map((a) => a.sev), ["SEV1"]);
  // Funded for 2 h: the runner is not quoting
  assert.deepEqual(run([conv(1, { Funded: { wei: ADV, since: BigInt(NOW - FUNDED_LATE_S + 1) } })]).alerts, []);
  assert.match(run([conv(1, { Funded: { wei: ADV, since: BigInt(NOW - FUNDED_LATE_S) } })]).alerts[0].text, /waited 120 min for a Relay quote/);
  // Arrived: first seen now, SEV2 a day later
  const a0 = run([conv(1, { Arrived: { order_id: b("ab".repeat(32)), sats: 600_000n } })]);
  assert.deepEqual(a0.alerts, []);
  const a1 = run([conv(1, { Arrived: { order_id: b("ab".repeat(32)), sats: 600_000n } })], NOW + ARRIVED_LATE_S, a0.next);
  assert.match(a1.alerts[0].text, /600000 sats have been in the float for 24 h/);
  // Returned: INFO; a second attempt: INFO (a refund was seen)
  assert.deepEqual(run([conv(1, { Returned: { wei: ADV } }, { attempts: 3 })]).alerts.map((a) => [a.sev, /RETURNED/.test(a.text)]), [["INFO", true]]);
  const refund = run([conv(1, { Funded: { wei: ADV, since: BigInt(NOW) } }, { attempts: 2 })]).alerts;
  assert.deepEqual(refund.map((a) => [a.sev, a.key]), [["INFO", "conv-refund:buyer-TEST:1:2"]]);
  // wired into the canister check, next to its other alerts
  const withConv = { ticketLogs: [], log: [], ordProviders: [["unisat", "https://open-api.unisat.io"]], holderDeposits: [] };
  assert.deepEqual(judgeCanister({ buyer: "b", read: withConv, now: NOW * 1000 }).alerts, []);
});

test("the install argument: frog_funding decodes as a known field; its loosenings and tightenings are said", () => {
  assert.ok(INIT_FIELDS.includes("frog_funding") && INIT_FIELDS.includes("float_first"));
  const base = decodeInit(F.args.install);
  assert.equal(base.ok, true, base.error);
  assert.deepEqual(base.init.frog_funding, [], "an argument from before the field: null (Relay, 300 bps, no float)");
  const withFunding = (max_fee_bps, float_first) => decodeInit(encodeInitPlain({ ...base.init, frog_funding: [{ max_fee_bps, float_first }] }));
  const loose = withFunding(450, true);
  assert.equal(loose.ok, true, loose.error);
  assert.deepEqual(loose.init.frog_funding, [{ max_fee_bps: 450, float_first: true }]);
  const lines = diffInit(base.init, loose.init).lines.map((l) => [l.sev, l.verb, l.text.split(":")[0]]);
  assert.deepEqual(lines, [["WARN", "LOOSENS", "frog_funding.max_fee_bps"], ["WARN", "LOOSENS", "frog_funding.float_first"]]);
  assert.deepEqual(diffInit(base.init, withFunding(900, false).init).lines.map((l) => l.sev), ["CRITICAL"]);
  assert.deepEqual(diffInit(loose.init, base.init).lines.map((l) => [l.sev, l.verb]), [["INFO", "TIGHTENS"], ["INFO", "TIGHTENS"]]);
  assert.deepEqual(diffInit(base.init, withFunding(300, false).init).lines, [], "the explicit default is the default");
  assert.ok(diffInit(base.init, withFunding(300, false).init).unchanged.includes("frog_funding"));
});
