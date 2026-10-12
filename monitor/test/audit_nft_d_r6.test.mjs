// Fix round 6 of the 2026-09-27 NFT review (docs/audit/2026-09-27-nft/fixes-round6.md): the monitor's PoCs of review round 6, part D
// (review6/poc/D/audit6_nft_d.test.mjs) turned around, on the canister's own bytes (eth-state-round6.hex: this round's state, every new
// field present), and the round-6 pages: the unseen home transaction worded per step (D6-L-1), a home transaction out too long whatever
// the providers show (D6-L-2), the late hold (D6-L-3, D6-I-2), the sale scan's lag and loss (B6-M-1, D6-I-1). Each FAILS on the code
// before this round.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { IDL } from "@icp-sdk/core/candid";
import { EthState, plain } from "../src/candid.mjs";
import { ethNftAlerts, HOME_OUT_S, HOME_OUT_SEV1_S, LATE_HOLD_SEV1_S } from "../src/ethnft.mjs";

const decodeHex = (h) => plain(IDL.decode([EthState], Uint8Array.from(Buffer.from(h.trim(), "hex")))[0]);
const fixture = (n) => decodeHex(readFileSync(new URL(`../../autobuyer/canister-rs/tests/fixtures/eth-state-round${n}.hex`, import.meta.url), "utf8"));
const alertsAt = (state, nowS) => ethNftAlerts({ buyer: "b", state, nowS }).alerts;
const byKey = (as, k) => as.filter((a) => a.key.startsWith(k));

test("round 6: the canister's round-6 state decodes, every new field present; the round-5 state decodes with the new fields absent", () => {
  const st = fixture(6);
  assert.deepEqual(Object.keys(st.pending[0].step), ["Deposit"]);
  assert.equal(Number(st.pending[0].sent_at[0]), 1_791_000_000);
  const sc = st.sale_scan[0][0];
  assert.deepEqual([Number(sc.behind[0]), sc.hole[0].map(Number), sc.lost[0].map(Number), sc.rfail[0].length], [9_000, [10, 20], [2, 3], 1]);
  const old = fixture(5);
  assert.deepEqual(old.pending[0].sent_at, []);
  assert.deepEqual(old.sale_scan[0][0].hole, []);
});

test("D6-L-1: the unseen-home page names eth_home_clear only for a step it applies to", () => {
  const st = fixture(6);
  st.pending[0].used_since = [1_791_000_000];
  const now = 1_791_000_000 + 2 * 3600;
  const dep = byKey(alertsAt(st, now), "eth-home-unseen")[0];
  assert.match(dep.text, /Deposit of buy #0/);
  assert.match(dep.text, /eth_home_clear\(7, true\)/);
  st.pending[0].step = { Post: { collection: st.buys[0].collection } };
  const post = byKey(alertsAt(st, now), "eth-home-unseen")[0];
  assert.match(post.text, /decides this step by its own effect/);
  assert.doesNotMatch(post.text, /eth_home_clear\(7/);
});

test("D6-L-2: a home transaction out too long pages whatever the providers show (used_since empty)", () => {
  const st = fixture(6);
  st.pending[0].used_since = [];
  const t0 = 1_791_000_000;
  assert.equal(byKey(alertsAt(st, t0 + HOME_OUT_S - 1), "eth-home-out").length, 0);
  const a = byKey(alertsAt(st, t0 + HOME_OUT_S), "eth-home-out")[0];
  assert.equal(a.sev, "SEV2");
  assert.ok(a.persistent);
  assert.equal(byKey(alertsAt(st, t0 + HOME_OUT_SEV1_S), "eth-home-out")[0].sev, "SEV1");
  assert.equal(byKey(alertsAt(st, t0 + 7 * 86_400), "eth-home-out")[0].sev, "SEV1", "a week on, still paged");
});

test("D6-L-3 / D6-I-2: the late hold pages persistently, escalates, and a credited order holds nothing", () => {
  const st = fixture(6);
  const letGo = Number(st.buys[1].late[0].let_go_at);
  const h = byKey(alertsAt(st, letGo + 3600), "eth-late-hold")[0];
  assert.equal(h.sev, "SEV2");
  assert.ok(h.persistent);
  assert.match(h.text, /eth_late_release\(1\)/);
  assert.equal(byKey(alertsAt(st, letGo + LATE_HOLD_SEV1_S), "eth-late-hold")[0].sev, "SEV1");
  st.buys[1].late[0].credited = [[new Uint8Array(32), 5n, true]];
  assert.equal(byKey(alertsAt(st, letGo + 3600), "eth-late-hold").length, 0, "credited: no hold");
  // the Stuck page's "holds the Relay leg" follows the canister's late_hold: not once credited
  st.buys[1].phase = { Stuck: { why: "written off" } };
  const stuck = byKey(alertsAt(st, letGo + 3600), "eth-stuck")[0];
  assert.doesNotMatch(stuck.text, /still holds the Relay leg/);
});

test("B6-M-1 / D6-I-1: the sale scan's lag, what a re-anchor passed over, and what the providers could not serve, page", () => {
  const st = fixture(6);
  const as = alertsAt(st, 1_791_000_000);
  // fix round 7 (B7-L-3): SEV2 from 2,400 final blocks, SEV1 from a day (7,200): the fixture's 9,000 is SEV1
  assert.equal(byKey(as, "eth-scan-behind")[0].sev, "SEV1");
  assert.match(byKey(as, "eth-scan-lost")[0].text, /blocks 2\.\.3/);
  assert.equal(byKey(as, "eth-scan-hole")[0].sev, "SEV3");
  assert.equal(byKey(as, "eth-scan-receipts")[0].sev, "SEV3");
  st.sale_scan[0][0].behind = [100n];
  st.sale_scan[0][0].lost = [];
  assert.equal(byKey(alertsAt(st, 1_791_000_000), "eth-scan-behind").length + byKey(alertsAt(st, 1_791_000_000), "eth-scan-lost").length, 0);
});
