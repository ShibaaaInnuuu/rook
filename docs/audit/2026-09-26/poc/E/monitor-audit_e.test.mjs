// AUDIT E (services and workflows) proof of concept; no source is changed. One 4663 RPC that keeps disagreeing about a single
// harmless log freezes the EVM canary's cursor forever: one non-repeating SEV2 is posted, the checker still reports itself
// healthy (lastOk moves, /healthz 200, no watcher-error), and an UNBACKED MINT later on the chain is never alerted.
// Run: cd monitor && node --test test/audit_e.test.mjs
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { decodeReply } from "../src/candid.mjs";
import { pollOnce, statusBody } from "../src/main.mjs";
import { EMPTY_RECORDS } from "../src/records.mjs";

const F = JSON.parse(readFileSync(new URL("../fixtures/canister.json", import.meta.url), "utf8"));
const E = JSON.parse(readFileSync(new URL("../fixtures/evm-logs.json", import.meta.url), "utf8"));
const L = E.logs;
const hex = (n) => "0x" + n.toString(16);
const FROM = 71_200_000;

test("E-06 PoC: a persistent one-log RPC disagreement silences the 4663 canary; a later unbacked mint is never alerted", async () => {
  const harmless = { ...L.deposited_matching, blockNumber: hex(FROM + 10) };          // e.g. a log one node has pruned or lags on
  const unbacked = { ...L.deposited_unknown, blockNumber: hex(FROM + 6_000) };          // both RPCs agree on this one
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
  // the watcher part is not under test: its IC reads fail (and it reports that on its own); only the evm checker matters here
  const ic = { records: async () => records, status: async () => { throw new Error("not under test"); }, history: async () => { throw new Error("not under test"); } };
  const cfg = { timelock: "tl", buyer: "b", network: "ic", every_secs: 60, heartbeat_hour_utc: 25, builds: [],
    evm: { rpcs: ["https://rpc-a.example", "https://rpc-b.example"], wrappers: [E.wrapper], floor_module: E.module, from_block: FROM, confirmations: 20 } };
  const posted = [];
  const channel = { send: async (t) => { posted.push(t); } };
  let state = {}, now = 1_790_000_000_000;
  for (let i = 0; i < 12; i++) {                         // six hours of polls, the chain moving on
    head += 10_000; now += 30 * 60_000;
    state = await pollOnce({ cfg, state, ic, http, channel, now });
  }
  const evmPosts = posted.filter((t) => /evm|4663|UNBACKED|disagree/i.test(t));
  console.log(`E-06: ${evmPosts.length} 4663-related message(s) in 6 h:\n  ${evmPosts.map((t) => t.split("\n").slice(0, 2).join(" | ")).join("\n  ")}`);
  console.log(`E-06: cursor ${state.evm?.cursor} (from ${FROM}); head ${head}; evm lastOk ${state.checkers.evm.lastOk === now ? "= now (healthy)" : state.checkers.evm.lastOk}`);
  assert.equal(state.evm.cursor, FROM, "the cursor never moves past the disputed chunk");
  assert.equal(evmPosts.filter((t) => /disagree/.test(t)).length, 1, "the disagreement is said ONCE (not persistent) and never again");
  assert.equal(posted.filter((t) => /UNBACKED MINT/.test(t)).length, 0, "the unbacked mint at block FROM+6000 is never alerted");
  assert.equal(state.checkers.evm.lastOk, now, "the evm checker counts as having completed");
  assert.equal(posted.filter((t) => /checker-down|evm checker has not completed/.test(t)).length, 0, "no watcher-error for the evm checker");
  const st = statusBody(cfg, { ...state, checkers: { evm: state.checkers.evm } }, now);
  assert.equal(st.checkers.evm.lastError, null);
});
