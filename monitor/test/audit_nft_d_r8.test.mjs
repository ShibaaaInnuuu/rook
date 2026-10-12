// Review round 8 of the 2026-09-27 NFT review (docs/audit/2026-09-27-nft/review8/README.md), the monitor's side, turned around. B8-L-1: ONE
// provider of the canister's pair that loses its log retention after go-live is never taken as "too deep" by the canister (B7-L-1), so no
// `lost` / `hole_deep` is set and nothing paged; its only trace is the walk back to the window's start (`SaleScan.down`) stuck above the
// start. The monitor now pages that (`eth-scan-window-unread`, SEV2): `down` unmoved for 12 h while 600 blocks (2 h) or more above the start.
// The state is the canister's own bytes at the end of the review's PoC scenario (autobuyer/canister-rs/tests/audit_nft_r7.rs
// r8_b8_l1_…: one provider 20,000 blocks deep, 216 passes). B8-I-1: a scan of a collection the configuration no longer names pages nothing.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { IDL } from "@icp-sdk/core/candid";
import { EthState, plain } from "../src/candid.mjs";
import { checkEthNft, ethNftAlerts, SALE_FINAL_DEPTH_BLOCKS, SALE_WINDOW_BLOCKS, SCAN_WINDOW_GAP_BLOCKS, SCAN_WINDOW_UNREAD_S } from "../src/ethnft.mjs";

const decodeHex = (h) => plain(IDL.decode([EthState], Uint8Array.from(Buffer.from(h.trim(), "hex")))[0]);
const canister = (f) => readFileSync(new URL(`../../autobuyer/canister-rs/${f}`, import.meta.url), "utf8");
const b8 = () => decodeHex(canister("tests/fixtures/eth-state-round8-b8l1.hex"));
const byKey = (as, k) => as.filter((a) => a.key.startsWith(k));
const NOW = 1_791_000_000, HOUR = 3_600;
/** The PoC's last pin (tests/audit_nft_r7.rs: ETH_PIN + 100 × R8_PASSES), the head the monitor reads beside it. */
const R8_PIN = 26_068_928 + 100 * 216;
const MILADY = "0x5af0d9827e0c53e4799bb226655a1de152a425a5";

/** Polls every `stepS` seconds for `hours`, the state as given (or as `mutate(state, i)` makes it), carrying `prev` like the checker. */
function polls({ state, hours, stepS = HOUR, head = R8_PIN, headStep = 0, mutate = null, collections = null }) {
  let prev = {}, out = [];
  for (let i = 0, t = 0; t <= hours * HOUR; i++, t += stepS) {
    if (mutate) mutate(state, i);
    const r = ethNftAlerts({ buyer: "b", state, nowS: NOW + t, prev, ethHead: BigInt(head + headStep * i), collections });
    prev = r.next;
    out.push({ t, alerts: r.alerts });
  }
  return out;
}

test("B8-L-1: the canister's constants the page relies on are the monitor's (window 50,400, final depth 128)", () => {
  const src = canister("src/ethnft.rs");
  assert.match(src, new RegExp(`pub const SALE_WINDOW_BLOCKS: u64 = ${SALE_WINDOW_BLOCKS.toLocaleString("en").replace(/,/g, "_")};`));
  assert.match(src, new RegExp(`pub const SALE_FINAL_DEPTH_BLOCKS: u64 = ${SALE_FINAL_DEPTH_BLOCKS};`));
});

test("B8-L-1: the PoC's end state decodes with `down` (the IDL carries it now), and nothing the monitor read before pages it", () => {
  const st = b8(), sc = st.sale_scan[0][0];
  assert.equal(Number(sc.down), 26_049_151);
  assert.deepEqual([sc.lost, sc.hole, sc.hole_deep, sc.busy], [[], [], [], []]);
  assert.equal(Number(sc.down) - (R8_PIN - SALE_FINAL_DEPTH_BLOCKS - SALE_WINDOW_BLOCKS), 9_151, "9,151 blocks of the window unread");
  const as = ethNftAlerts({ buyer: "b", state: st, nowS: NOW, ethHead: BigInt(R8_PIN) }).alerts;
  assert.deepEqual(byKey(as, "eth-scan").filter((a) => !a.key.startsWith("eth-scan-window-unread")), [], "round 7's pages stay silent (the review's finding)");
});

test("B8-L-1, turned around: one genuinely shallow provider pages SEV2 once `down` has not moved for 12 h", () => {
  const run = polls({ state: b8(), hours: 26, headStep: 300 });   // the head moves on (300 blocks an hour); `down` does not
  const first = run.find((p) => byKey(p.alerts, "eth-scan-window-unread").length);
  assert.ok(first, "paged");
  assert.equal(first.t, SCAN_WINDOW_UNREAD_S, "at 12 h, not before");
  const a = byKey(first.alerts, "eth-scan-window-unread")[0];
  assert.equal(a.sev, "SEV2");
  assert.ok(a.persistent);
  assert.match(a.text, /stuck at block 26049151 for 12 h/);
  assert.match(a.text, /ONE provider/);
  assert.match(a.text, /depth_probe\.py/);
  // persistent while the unread part is in the window; it leaves the window as the window moves (7,200 blocks a day), and the page clears
  const paged = run.filter((p) => byKey(p.alerts, "eth-scan-window-unread").length).map((p) => p.t / HOUR);
  const gapAt = (h) => 9_151 - 300 * h;
  assert.deepEqual(paged, run.map((p) => p.t / HOUR).filter((h) => h >= 12 && gapAt(h) >= SCAN_WINDOW_GAP_BLOCKS));
  assert.equal(paged.at(-1), 26, "still paged at 26 h (1,351 blocks unread)");
  assert.equal(polls({ state: b8(), hours: 30, headStep: 300 }).filter((p) => byKey(p.alerts, "eth-scan-window-unread").length).at(-1).t / HOUR, 28,
    "cleared once the unread blocks are within 2 h of the window's start (29 h: 451 blocks)");
  // the key does not carry `down`, so one page repeats (persistent) instead of a new one each time the number changes
  assert.equal(a.key, `eth-scan-window-unread:b:${MILADY}`);
});

test("B8-L-1: a walk back that MOVES (the first days after install) never pages, however far from the start", () => {
  const st = b8(), sc = st.sale_scan[0][0];
  const start = Number(sc.down) + 40_000;
  const run = polls({ state: st, hours: 72, mutate: (s, i) => { s.sale_scan[0][0].down = BigInt(start - 500 * i); } });
  assert.equal(run.flatMap((p) => byKey(p.alerts, "eth-scan-window-unread")).length, 0);
});

test("B8-L-1: a walk back that moves again restarts the 12 h; one that reaches the start clears the page", () => {
  const st = b8(), sc = st.sale_scan[0][0], d0 = BigInt(sc.down);
  // stuck 10 h, one step, stuck again: paged only 12 h after the step
  const run = polls({ state: st, hours: 30, mutate: (s, i) => { s.sale_scan[0][0].down = i < 10 ? d0 : d0 - 100n; } });
  const first = run.find((p) => byKey(p.alerts, "eth-scan-window-unread").length);
  assert.equal(first.t, (10 + 12) * HOUR);
  // at the window's start (the canister's own pass lags the monitor's head by a few hundred blocks): nothing
  const bottom = BigInt(R8_PIN - SALE_FINAL_DEPTH_BLOCKS - SALE_WINDOW_BLOCKS);
  for (const d of [bottom - 300n, bottom, bottom + BigInt(SCAN_WINDOW_GAP_BLOCKS) - 1n]) {
    sc.down = d;
    assert.equal(polls({ state: st, hours: 48 }).flatMap((p) => byKey(p.alerts, "eth-scan-window-unread")).length, 0, `down ${d}`);
  }
});

test("B8-L-1: no Ethereum head read, or a scan never started (up 0): no page on `down` (the lag pages cover those)", () => {
  const st = b8();
  let prev = {};
  for (let t = 0; t <= 30 * HOUR; t += HOUR) prev = ethNftAlerts({ buyer: "b", state: st, nowS: NOW + t, prev }).next;
  assert.equal(byKey(ethNftAlerts({ buyer: "b", state: st, nowS: NOW + 31 * HOUR, prev }).alerts, "eth-scan-window-unread").length, 0);
  st.sale_scan[0][0].up = 0n;
  assert.equal(polls({ state: st, hours: 30 }).flatMap((p) => byKey(p.alerts, "eth-scan-window-unread")).length, 0);
});

test("B8-I-1: a scan of a collection the configuration no longer names pages nothing; a configured one still does", () => {
  // review8/poc/r8_stale.mjs: a frozen entry (a collection removed by an upgrade) paged SEV1 forever
  const st = b8();
  st.sale_scan[0][0].current = [26_000_000n];
  const behind = (collections) => byKey(ethNftAlerts({ buyer: "b", state: st, nowS: NOW, ethHead: BigInt(R8_PIN), collections }).alerts, "eth-scan-behind");
  assert.equal(behind(null)[0].sev, "SEV1", "no list given: as before");
  assert.equal(behind(["0x" + "ed".repeat(20)]).length, 0, "not configured: nothing");
  assert.equal(behind([MILADY.toUpperCase().replace("0X", "0x")])[0].sev, "SEV1", "configured (any case): paged");
});

test("B8-I-1 / B8-L-1 through the checker: checkEthNft passes its collections and its own head", async () => {
  const st = b8();
  const cfg = { buyer: "aaaaa-aa", eth_nft: { eth_rpc: ["https://e1.example", "https://e2.example"], home_rpc: ["https://h.example"], collections: [] } };
  const http = async (url, init) => {
    const body = JSON.parse(init.body);
    const result = body.method === "eth_blockNumber" ? `0x${R8_PIN.toString(16)}` : null;
    return { ok: true, status: 200, json: async () => ({ jsonrpc: "2.0", id: body.id, result }), text: async () => "" };
  };
  const ic = { ethState: async () => st };
  // no collection configured: the frozen scan is filtered; the checker's per-collection reads are not reached
  let prev = {}, got = [];
  for (let t = 0; t <= 25 * HOUR; t += HOUR) {
    const r = await checkEthNft({ cfg, prev, now: (NOW + t) * 1000, http, ic }).catch((e) => ({ error: e }));
    if (r.error) throw r.error;
    prev = r.next; got = r.alerts;
  }
  assert.equal(byKey(got, "eth-scan").length, 0);
});
