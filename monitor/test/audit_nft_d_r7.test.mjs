// Fix round 7 of the 2026-09-27 NFT review (docs/audit/2026-09-27-nft/fixes-round7.md): the monitor's side of review round 7's scan
// findings, on the canister's own bytes (eth-state-round7.hex: this round's state, every new field present). B7-L-3: the scan's lag is
// the monitor's own final head less where the scan was last current, so a scan that stopped (no pass rewrites `behind`) or a re-anchored
// one (review7/poc/D test 2: never paged) pages SEV2 at 8 h and SEV1 at a day. B7-L-1: a hole every provider refuses as too deep, or a
// hole unread for a day, pages. B7-L-2: receipts answered as a rate limit, and passes in a row that met one, page. D7-I-2: a home
// transaction with no sent_at is aged from when the monitor first saw it.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { IDL } from "@icp-sdk/core/candid";
import { EthState, plain } from "../src/candid.mjs";
import { ethNftAlerts, HOME_OUT_S, SALE_FINAL_DEPTH_BLOCKS, SCAN_BEHIND_BLOCKS, SCAN_BEHIND_SEV1_BLOCKS, SCAN_HOLE_DAY_S } from "../src/ethnft.mjs";

const decodeHex = (h) => plain(IDL.decode([EthState], Uint8Array.from(Buffer.from(h.trim(), "hex")))[0]);
const fixture = (n) => decodeHex(readFileSync(new URL(`../../autobuyer/canister-rs/tests/fixtures/eth-state-round${n}.hex`, import.meta.url), "utf8"));
const byKey = (as, k) => as.filter((a) => a.key.startsWith(k));
const NOW = 1_791_000_000;

test("round 7: the canister's round-7 state decodes, every new field present; the round-6 state decodes with them absent", () => {
  const sc = fixture(7).sale_scan[0][0];
  assert.deepEqual([Number(sc.current[0]), Number(sc.hole_deep[0][1]), sc.rbusy[0].length, Number(sc.busy[0][1])], [1_000, 4, 1, 9]);
  const old = fixture(6).sale_scan[0][0];
  assert.deepEqual([old.current, old.hole_deep, old.rbusy, old.busy], [[], [], [], []]);
});

test("B7-L-3: the lag is the monitor's final head less `current`: a stopped scan pages SEV2 at 8 h and SEV1 at a day", () => {
  const st = fixture(7);
  const sc = st.sale_scan[0][0];
  sc.behind = [0n];                                    // what a scan that stopped right after a current pass left
  sc.hole = []; sc.hole_deep = []; sc.rbusy = []; sc.busy = []; sc.lost = []; sc.rfail = [];
  const at = (blocksLater) => byKey(ethNftAlerts({ buyer: "b", state: st, nowS: NOW, ethHead: 1_000 + SALE_FINAL_DEPTH_BLOCKS + blocksLater }).alerts, "eth-scan-behind");
  assert.equal(at(300).length, 0, "an hour: nothing");
  assert.equal(at(SCAN_BEHIND_BLOCKS)[0].sev, "SEV2");
  assert.ok(at(SCAN_BEHIND_BLOCKS)[0].persistent);
  assert.equal(at(SCAN_BEHIND_SEV1_BLOCKS)[0].sev, "SEV1");
  assert.equal(at(3 * 7_200)[0].sev, "SEV1", "three days on, still paged (the re-anchor keeps no lag hidden)");
  // no Ethereum head read: the canister's own lag at its last pass
  sc.behind = [BigInt(SCAN_BEHIND_BLOCKS)];
  assert.equal(byKey(ethNftAlerts({ buyer: "b", state: st, nowS: NOW }).alerts, "eth-scan-behind")[0].sev, "SEV2");
});

test("B7-L-1: a hole every provider refuses as too deep pages; a hole unread for a day pages", () => {
  const st = fixture(7);
  const as = ethNftAlerts({ buyer: "b", state: st, nowS: NOW }).alerts;
  const d = byKey(as, "eth-scan-deep")[0];
  assert.equal(d.sev, "SEV2");
  assert.match(d.text, /blocks 10\.\.20/);
  st.sale_scan[0][0].hole_deep = [];
  let prev = {};
  const r1 = ethNftAlerts({ buyer: "b", state: st, nowS: NOW, prev });
  assert.equal(byKey(r1.alerts, "eth-scan-hole-day").length, 0);
  const r2 = ethNftAlerts({ buyer: "b", state: st, nowS: NOW + SCAN_HOLE_DAY_S, prev: r1.next });
  assert.equal(byKey(r2.alerts, "eth-scan-hole-day")[0].sev, "SEV2");
});

test("B7-L-2: receipts answered as a rate limit 6 times in a row, and 9 busy passes in a row, page", () => {
  const as = ethNftAlerts({ buyer: "b", state: fixture(7), nowS: NOW }).alerts;
  assert.equal(byKey(as, "eth-scan-busy-receipts")[0].sev, "SEV2");
  assert.equal(byKey(as, "eth-scan-busy:")[0].sev, "SEV2");
  const st = fixture(7);
  st.sale_scan[0][0].rbusy[0][0][1] = 5;
  st.sale_scan[0][0].busy[0][1] = 8;
  const q = ethNftAlerts({ buyer: "b", state: st, nowS: NOW }).alerts;
  assert.equal(byKey(q, "eth-scan-busy").length, 0);
});

test("D7-I-2: a home transaction with no sent_at (signed by the round-5 build) is aged from when the monitor first saw it", () => {
  const st = fixture(5);
  assert.deepEqual(st.pending[0].sent_at, []);
  const r1 = ethNftAlerts({ buyer: "b", state: st, nowS: NOW });
  assert.equal(byKey(r1.alerts, "eth-home-out").length, 0);
  const r2 = ethNftAlerts({ buyer: "b", state: st, nowS: NOW + HOME_OUT_S, prev: r1.next });
  assert.equal(byKey(r2.alerts, "eth-home-out")[0].sev, "SEV2");
});
