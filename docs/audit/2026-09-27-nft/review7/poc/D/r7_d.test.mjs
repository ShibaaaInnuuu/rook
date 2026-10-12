// Review round 7, part D: the round-6 pages at a78a542. "WEAKNESS" assertions pass while the weakness is present.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { IDL } from "@icp-sdk/core/candid";
import { EthState, plain } from "../src/candid.mjs";
import { ethNftAlerts, SCAN_BEHIND_BLOCKS } from "../src/ethnft.mjs";

const fixture = (n) => plain(IDL.decode([EthState], Uint8Array.from(Buffer.from(readFileSync(new URL(`../../autobuyer/canister-rs/tests/fixtures/eth-state-round${n}.hex`, import.meta.url), "utf8").trim(), "hex")))[0]);
const keys = (st, nowS) => ethNftAlerts({ buyer: "b", state: st, nowS }).alerts.map((a) => a.key.split(":")[0]);

test("WEAKNESS (D7-I-2): a home transaction signed by the round-5 build (no sent_at) is never paged by eth-home-out, however old", () => {
  const st = fixture(5);
  assert.equal(st.pending.length, 1);
  assert.deepEqual(st.pending[0].sent_at, []);
  for (const nowS of [1_800_000_000, 1_900_000_000]) assert.ok(!keys(st, nowS).includes("eth-home-out"));
});

test("WEAKNESS (D7-I-1): eth-scan-behind reads only the canister's own snapshot: a scan that stopped running is never paged by it, however long", () => {
  const st = fixture(6);
  const sc = st.sale_scan[0][0];
  // what a pass leaves after the round-6 re-anchor: at most a chunk behind (the re-anchor fires at > 7,200 at a pass's start)
  sc.behind = [2_000n];
  for (const nowS of [1_791_000_000, 1_791_000_000 + 30 * 86_400]) assert.ok(!keys(st, nowS).includes("eth-scan-behind"), "never, whatever the time");
  // it fires only at >= SCAN_BEHIND_BLOCKS, which a pass can leave only when the lag was exactly 7,200 at its start and nothing was read
  sc.behind = [BigInt(SCAN_BEHIND_BLOCKS)];
  assert.ok(keys(st, 1_791_000_000).includes("eth-scan-behind"));
});
