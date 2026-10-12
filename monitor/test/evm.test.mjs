// The 4663 canary on eth_getLogs-shaped logs whose data was ABI-encoded by foundry's `cast`, not by the monitor
// (fixtures/evm-logs.json), against the canister's own Rust-encoded records. No network.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { decodeReply } from "../src/candid.mjs";
import { TOPICS, checkEvm, compareLogs, decodeDeposited, judgeLogs } from "../src/evm.mjs";
import { EMPTY_RECORDS, recordIndex } from "../src/records.mjs";
import { pollOnce, statusBody } from "../src/main.mjs";

const F = JSON.parse(readFileSync(new URL("../fixtures/canister.json", import.meta.url), "utf8"));
const E = JSON.parse(readFileSync(new URL("../fixtures/evm-logs.json", import.meta.url), "utf8"));
const L = E.logs;
const index = () => recordIndex({ ...EMPTY_RECORDS, redemptions: decodeReply("redemptions", F.replies.redemptions_delivered),
  frog_buys: decodeReply("frog_buys", F.replies.frog_buys) });
const CFG = { wrappers: [E.wrapper], floor_module: E.module };
const judge = (logs, prev) => judgeLogs({ logs, index: index(), cfg: CFG, prev, timelock: "tl" });

test("the topics are the compiled ABI's event signatures (values from `cast keccak`)", () => {
  assert.equal(TOPICS.Deposited, "0x9853eeeea9b6c1f6daab1bb5b6b70c49487154583b78d5fb04d7e9295348f145");
  assert.equal(TOPICS.Delivered, "0xaacc6b338ebefd6dd1df07d0b00f5df4f445624d4f3206876c9976802920187b");
  assert.equal(TOPICS.Wrapped, "0x8891fc9ec66fca4325a8d6ac15db39062e237c7c3ad0d9fa29c573e41b745b5f");
  assert.equal(TOPICS.RentalReturned, "0x57a4e3d8f664789c426e6fe26d9a785a5f0bbdd6059570d36cf8f2f66009762a");
  assert.equal(TOPICS.MintLimitProposed, "0x5f3ca458b0fcc1227c6c6963ea675adad69d7173c1ef1a6bf6fc4ef677d84b76");
  assert.equal(TOPICS.MintLimitSet, "0x28ebdbd10a26a82cec6aa90b2fecb2a9aca61629ad2e80d17de29c6a00185398");
  assert.equal(TOPICS.ItemArrived, "0x8d772fa470bdea41681aa06dce21a856501fd69b06b2c40c728c9ecf359d79fd");
});

test("Deposited decodes: the inscription, the outpoint, the deposit flag, the recipient", () => {
  const d = decodeDeposited(L.deposited_matching);
  assert.deepEqual(d, { frogId: "deposit #0", to: E.module, inscription: "b6ceea8eb7e46c5784c6ff3eb3af6e5eca91f53ed8a06cdda7fe6f6c1b5f738ei0",
    txid: "b6ceea8eb7e46c5784c6ff3eb3af6e5eca91f53ed8a06cdda7fe6f6c1b5f738e", vout: 0 });
});

test("a mint, a delivery record and an arrival that the canister's records back raise nothing", () => {
  const r = judge([L.deposited_matching, L.delivered_matching, L.arrived_first]);
  assert.deepEqual(r.alerts, [], JSON.stringify(r.alerts));
  assert.equal(Object.values(r.next.arrivals)[0], "1", "the arrival is matched to frog purchase #1, and consumes it");
});

test("a mint no record names is SEV1 (an unbacked mint), and stays open until acknowledged", () => {
  const r = judge([L.deposited_unknown]);
  assert.equal(r.alerts.length, 1);
  assert.deepEqual([r.alerts[0].kind, r.alerts[0].sev, r.alerts[0].persistent], ["evm-mint", "SEV1", true]);
  assert.match(r.alerts[0].text, /AN UNBACKED MINT\? The wrapper 0x0{38}aa minted frog deposit #1 \(one wX\)/);
  assert.match(r.alerts[0].text, /no canister record names that outpoint/);
  assert.equal(judge([], r.next).alerts[0].key, r.alerts[0].key, "the next range still repeats it");
  assert.deepEqual(judgeLogs({ logs: [], index: index(), cfg: { ...CFG, acknowledged: [L.deposited_unknown.transactionHash] }, prev: r.next }).alerts, []);
});

test("a delivery record with no delivery behind it (a keeper-made record, T4) is SEV1", () => {
  const r = judge([L.delivered_no_delivery]);
  assert.equal(r.alerts[0].sev, "SEV1");
  assert.match(r.alerts[0].text, /recorded ticket 7 as delivered in Bitcoin tx 7{64}, and the canister's redemption for that ticket does not exist/);
});

test("each frog backs one arrival: the second arrival at the same cost is SEV1", () => {
  const r = judge([L.arrived_first, L.arrived_second]);
  assert.equal(r.alerts.length, 1);
  assert.match(r.alerts[0].text, /AN ARRIVAL NO FROG BACKS: .* cost 0\.65 ETH/);
  // the consumption survives across polls
  const again = judge([L.arrived_second], judge([L.arrived_first]).next);
  assert.equal(again.alerts.length, 1);
});

test("paths that are OFF in Phase 1, and a mint-limit raise, alert as SEV2; a log from another contract is ignored", () => {
  const r = judge([L.wrapped, L.mint_limit_proposed]);
  assert.deepEqual(r.alerts.map((a) => [a.kind, a.sev]), [["evm-mint", "SEV2"], ["evm-mint", "SEV2"]]);
  assert.match(r.alerts[0].text, /emitted Wrapped: a path that is OFF in Phase 1/);
  assert.match(r.alerts[1].text, /mint limit \(H11\) to 40 a day/);
  assert.deepEqual(judge([{ ...L.deposited_unknown, address: "0x" + "cc".repeat(20) }]).alerts, []);
});

test("two RPCs: the same logs agree; a missing or altered log is a disagreement, kept aside and re-read while the cursor goes on (E-06)", async () => {
  assert.equal(compareLogs([L.deposited_matching], [L.deposited_matching]).same, true);
  const altered = { ...L.deposited_matching, data: L.deposited_unknown.data };
  assert.equal(compareLogs([L.deposited_matching], [altered]).same, false);
  assert.equal(compareLogs([L.deposited_matching], []).onlyA.length, 1);

  const rpc = (logsFor) => async (url, init) => {
    const { method } = JSON.parse(init.body);
    const result = method === "eth_blockNumber" ? "0x" + (71_200_100).toString(16) : logsFor(url);
    return { ok: true, json: async () => ({ jsonrpc: "2.0", id: 1, result }) };
  };
  const order = [];
  const readRecords = async () => { order.push("records"); return { ...EMPTY_RECORDS, redemptions: decodeReply("redemptions", F.replies.redemptions_delivered), frog_buys: decodeReply("frog_buys", F.replies.frog_buys) }; };
  const cfg = { rpcs: ["https://a.example/rpc", "https://b.example/rpc"], ...CFG, from_block: 71_200_000, confirmations: 20 };
  const agree = await checkEvm({ cfg, prev: {}, http: rpc(() => { order.push("logs"); return [L.deposited_matching, L.delivered_matching]; }), readRecords, timelock: "tl" });
  assert.deepEqual(agree.alerts, []);
  assert.equal(agree.next.cursor, 71_200_081, "up to head − 20, inclusive");
  assert.equal(order.at(-1), "records", "records are read after the logs");
  const split = await checkEvm({ cfg, prev: {}, http: rpc((u) => (u.includes("a.example") ? [L.deposited_matching] : [])), readRecords, timelock: "tl" });
  assert.deepEqual(split.alerts.map((a) => [a.kind, a.sev, a.persistent]), [["provider-disagreement", "SEV2", true]]);
  assert.match(split.alerts[0].text, /a\.example has 1 the other lacks/);
  assert.equal(split.next.cursor, 71_200_081, "the cursor goes on past the disputed range");
  assert.deepEqual(split.next.disputed.map((d) => [d.from, d.to]), [[71_200_000, 71_200_080]], "which is kept aside and re-read");
  assert.equal(split.degraded, null, "not degraded yet: the dispute is new");
});

test("E-06 regression: a persistent one-log RPC disagreement no longer silences the canary; the later unbacked mint is alerted, the dispute repeats, and the checker turns unhealthy", async () => {
  const FROM = 71_200_000, hex = (n) => "0x" + n.toString(16);
  const harmless = { ...L.deposited_matching, blockNumber: hex(FROM + 10) };
  const unbacked = { ...L.deposited_unknown, blockNumber: hex(FROM + 6_000) };
  let head = FROM + 100;
  const http = async (url, init) => {
    const { method, params } = JSON.parse(init.body);
    let result;
    if (method === "eth_blockNumber") result = hex(head);
    else if (method === "eth_getLogs") {
      const a = Number(BigInt(params[0].fromBlock)), b = Number(BigInt(params[0].toBlock));
      const all = url.includes("rpc-a") ? [harmless, unbacked] : [unbacked];
      result = all.filter((l) => Number(BigInt(l.blockNumber)) >= a && Number(BigInt(l.blockNumber)) <= b);
    } else throw new Error(`unexpected ${method}`);
    return { ok: true, json: async () => ({ jsonrpc: "2.0", id: 1, result }) };
  };
  const records = { ...EMPTY_RECORDS, redemptions: decodeReply("redemptions", F.replies.redemptions_delivered), frog_buys: decodeReply("frog_buys", F.replies.frog_buys) };
  const ic = { records: async () => records, status: async () => { throw new Error("not under test"); }, history: async () => { throw new Error("not under test"); } };
  const cfg = { timelock: "tl", buyer: "b", network: "ic", every_secs: 60, heartbeat_hour_utc: 25, builds: [],
    evm: { rpcs: ["https://rpc-a.example", "https://rpc-b.example"], wrappers: [E.wrapper], floor_module: E.module, from_block: FROM, confirmations: 20 } };
  const posted = [];
  const channel = { send: async (t) => { posted.push(t); } };
  let state = {}, now = 1_790_000_000_000;
  for (let i = 0; i < 14; i++) { head += 10_000; now += 30 * 60_000; state = await pollOnce({ cfg, state, ic, http, channel, now }); }
  assert.ok(state.evm.cursor > FROM + 6_000, "the cursor moved past the disputed chunk");
  assert.equal(posted.filter((t) => /UNBACKED MINT/.test(t)).length >= 1, true, "the unbacked mint both RPCs show is alerted");
  const disputes = posted.filter((t) => /RPCs disagree about the stack's logs in blocks 71200000/.test(t));
  assert.ok(disputes.length >= 2, `the dispute is said again (every 6 h, and once more after 1 h): ${disputes.length}`);
  assert.ok(posted.some((t) => /STILL, for/.test(t)), "a 'still disagreeing' message after an hour");
  assert.match(state.checkers.evm.degraded, /have disagreed about blocks 71200000–71204999 for/);
  const st = statusBody(cfg, { ...state, checkers: { evm: state.checkers.evm } }, now);
  assert.equal(st.ok, false, "/healthz is 503 while blind to a range for an hour");
});

test("a cursor stuck behind while the heads move is a watcher-error and degrades the checker (one RPC not keeping up)", async () => {
  let headA = 71_200_100, headB = 71_200_100;
  const http = async (url, init) => {
    const { method } = JSON.parse(init.body);
    const result = method === "eth_blockNumber" ? "0x" + (url.includes("a.example") ? headA : headB).toString(16) : [];
    return { ok: true, json: async () => ({ jsonrpc: "2.0", id: 1, result }) };
  };
  const cfg = { rpcs: ["https://a.example/rpc", "https://b.example/rpc"], ...CFG, from_block: 71_200_000, confirmations: 20 };
  const readRecords = async () => EMPTY_RECORDS;
  let r = await checkEvm({ cfg, prev: {}, now: 0, http, readRecords, timelock: "tl" });
  for (let i = 1; i <= 7; i++) { headA += 600; r = await checkEvm({ cfg, prev: r.next, now: i * 5 * 60_000, http, readRecords, timelock: "tl" }); }
  assert.match(r.degraded, /cursor has not moved for 0\.5 h/);
  assert.ok(r.alerts.some((a) => a.kind === "watcher-error" && /STALLED/.test(a.text)));
});
