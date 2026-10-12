// Review round 6, part D: PoCs on the round-5 services (monitor). A passing assertion marked WEAKNESS shows the weakness is present.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { IDL } from "@icp-sdk/core/candid";
import { EthState, plain } from "../src/candid.mjs";
import * as R4 from "../src/candid_r4.mjs";
import { ethNftAlerts } from "../src/ethnft.mjs";
import { stagedStep, stepNumbers } from "../src/staged.mjs";
import { decodeInit, encodeInitPlain } from "../src/candid.mjs";

const fx = (n) => Uint8Array.from(Buffer.from(readFileSync(new URL(`../../autobuyer/canister-rs/tests/fixtures/${n}`, import.meta.url), "utf8").trim(), "hex"));
const dec = (T, b) => plain(IDL.decode([T], b)[0]);
const r5 = () => dec(EthState, fx("eth-state-round5.hex"));
const NOW = 1_800_000_000;

test("IDL: round-5 state decodes with the round-5 monitor IDL and with the round-4 (8ef82d6) monitor IDL; round-4/round-2 states with the round-5 IDL", () => {
  const a = r5();
  assert.ok(a.pending.length === 1, "the fixture carries a pending HomeTx");
  console.log("round5 fixture pending:", JSON.stringify({ nonce: a.pending[0].nonce, used_since: a.pending[0].used_since }));
  const old = dec(R4.EthState, fx("eth-state-round5.hex"));
  assert.equal(old.buys.length, a.buys.length);
  for (const f of ["eth-state-round4.hex", "eth-state-round2.hex"]) {
    const s = dec(EthState, fx(f));
    assert.ok(Array.isArray(s.pending), `${f}: pending decodes (opt)`);
    for (const b of s.buys) { assert.ok(Array.isArray(b.last)); if (b.late.length) assert.ok(Array.isArray(b.late[0].released_at)); }
  }
});

const pendingAt = (usedSince) => ({ buys: [], deliveries: [], readings: [], deposits: [], eth_out: "0", line: [], last_mined: [], halted: [],
  pending: [{ hash: "11".repeat(32), nonce: "42", used_since: usedSince === null ? [] : [String(usedSince)] }] });

test("D6-L-1 WEAKNESS: the eth-home-unseen page cannot tell a Relay deposit from a state-bearing step and always names eth_home_clear", () => {
  // the monitor's HomeTx skips `step`: the same SEV1 text for any step, telling the operator to call eth_home_clear, which the canister
  // accepts for ANY pending step but consumes only for a Relay deposit / fundReserve (ethflow.rs resolve_home Ok(None) arm)
  const a = ethNftAlerts({ buyer: "b", state: pendingAt(NOW - 3600), nowS: NOW }).alerts.filter((x) => x.key.startsWith("eth-home-unseen:"));
  assert.equal(a.length, 1);
  assert.equal(a[0].sev, "SEV1");
  assert.match(a[0].text, /eth_home_clear\(42, true\)/);
  assert.doesNotMatch(a[0].text, /Record|Advance|deposit|fundReserve|Post/i, "the page names no step");
});

test("D6-L-2 WEAKNESS: a home line that never clears because a provider keeps showing it Pending / Unsure (used_since reset every tick) pages nothing, whatever its age", () => {
  for (const age of [3600, 86_400, 7 * 86_400]) {
    const a = ethNftAlerts({ buyer: "b", state: pendingAt(null), nowS: NOW + age }).alerts;
    assert.equal(a.length, 0, `after ${age} s: no alert of any kind for the pending home transaction`);
  }
  // and flapping: used_since set 59 min ago, then reset, then set again: never an hour in a row
  assert.equal(ethNftAlerts({ buyer: "b", state: pendingAt(NOW - 3599), nowS: NOW }).alerts.length, 0);
});

test("D6-L-3 WEAKNESS: a written-off order whose buy was closed as a loss holds every new Relay order (late_hold) with only a non-persistent SEV3 note", () => {
  const st = r5();
  const b = st.buys.find((x) => x.late.length && x.late[0].credited.length === 0 && x.late[0].released_at.length === 0);
  assert.ok(b, "the fixture has a held late order");
  b.phase = { Returned: { wei: "0" } };   // closed as a loss by the second eth_give_back: finished, but late_hold() still holds (ethflow.rs:660)
  const a = ethNftAlerts({ buyer: "b", state: { ...st, buys: [b], deliveries: [], pending: [] }, nowS: NOW }).alerts;
  console.log(a.map((x) => `${x.sev} persistent=${!!x.persistent} ${x.key}`).join("\n"));
  assert.ok(!a.some((x) => x.sev === "SEV1" || x.sev === "SEV2"), "no SEV1/SEV2 says new buys of every collection are held");
  assert.ok(a.some((x) => x.sev === "SEV3" && x.key.startsWith("eth-late-order:") && !x.persistent));
});

test("D6-I-2 WEAKNESS: the Stuck page says a CREDITED written-off order still holds the Relay leg (held ignores credited)", () => {
  const st = r5();
  const b = structuredClone(st.buys.find((x) => x.late.length));
  b.phase = { Stuck: { why: "x", since: "1" } }; b.last = [];
  b.late[0].credited = [["aa".repeat(32), "1", true]]; b.late[0].released_at = [];
  const a = ethNftAlerts({ buyer: "b", state: { ...st, buys: [b], deliveries: [], pending: [] }, nowS: NOW }).alerts;
  const s = a.find((x) => x.key.startsWith("eth-stuck:"));
  assert.match(s.text, /still holds the Relay leg for new orders/);
});

test("D6-I-3: a step-3 proposal that also adds a cap for another collection, or raises the global value cap, still pages EXPECTED (the step-3 read checks only the launch keys)", () => {
  const LAUNCH = JSON.parse(readFileSync(new URL("../../deployments/launch/nft-pilot.json", import.meta.url), "utf8"));
  const initAt = (step, extra = [], gv = null) => {
    const s = stepNumbers(LAUNCH, step);
    const collections = [...Object.entries(s.caps).map(([collection, c]) => ({ collection, max_items: Number(c.items), max_value_wei: String(c.value), max_item_wei: String(c.item) })), ...extra];
    const v = { chain_id: 4663n, vault: "0x00", rpc_urls: ["https://a", "https://b"], btc_api_urls: ["c", "d"], key_name: "key_1", accept_production_key_name: true,
      bridge_allowlist: [], btc_network: "mainnet", operator: [], wrapper: [], evm_chains: [], holder_deposits: [], wrapper_from_block: [],
      custody_caps: [{ collections, global_max_items: Number(s.global.items), global_max_value_wei: gv ?? String(s.global.value) }],
      outflow_limits: [{ ...Object.fromEntries(Object.entries(s.limits).map(([k, v]) => [k, k === "global_value_wei" ? String(v) : Number(v)])) }], eth_nft: [] };
    const d = decodeInit(encodeInitPlain(v)); assert.ok(d.ok, d.error); return d.init;
  };
  assert.equal(stagedStep(LAUNCH, initAt(2).valueOf(), initAt(3)), 3, "control: step 3 is EXPECTED");
  assert.equal(stagedStep(LAUNCH, initAt(2), initAt(3, [], String(300n * 10n ** 18n))), 3, "global value 75 -> 300 ETH still EXPECTED");
  const x = { collection: `1:0x${"ab".repeat(20)}`, max_items: 1, max_value_wei: String(15n * 10n ** 18n), max_item_wei: String(15n * 10n ** 18n) };
  const i3 = initAt(3, [x]); i3.custody_caps[0].global_max_items = 62;
  assert.equal(stagedStep(LAUNCH, initAt(2), i3), 3, "WEAKNESS: a new collection's cap (1 item) beside step 3 is still EXPECTED");
  const i3b = initAt(3, [{ ...x, max_items: 3 }]); i3b.custody_caps[0].global_max_items = 64;
  assert.equal(stagedStep(LAUNCH, initAt(2), i3b), null, "3 items: past the exit rule, not EXPECTED");
});
