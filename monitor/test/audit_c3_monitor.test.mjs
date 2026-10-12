// Audit round C, part C3 (docs/v9/audit-2026-10/c3-robinhood-chain.md), C3-05 (Medium): the monitor read `eth_nft_state()` only, so the
// Robinhood Chain (4663) side (its line, a halt, a stuck or quote-less buy holding the Relay leg, a late hold, its sale scan) was never
// paged. Fixed (2026-10-12): `sideAlerts` reads `eth_nft_config()` for the chains its collections live on and `eth_nft_state_of(chain)` for
// each other than the config's own, and runs the same pages on each, keyed by chain; the scan's lag is measured in that chain's blocks.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { IDL } from "@icp-sdk/core/candid";
import { EthState, plain } from "../src/candid.mjs";
import { ethNftAlerts, scanBlocksOf, sideAlerts, sideChainsOf } from "../src/ethnft.mjs";

const decodeHex = (h) => plain(IDL.decode([EthState], Uint8Array.from(Buffer.from(h.trim(), "hex")))[0]);
const fixture = (n) => decodeHex(readFileSync(new URL(`../../autobuyer/canister-rs/tests/fixtures/eth-state-round${n}.hex`, import.meta.url), "utf8"));
const NOW = 1_791_000_000;
const CASHCAT = "0xe3b34c4bb0f12c82143745eee6a6cf4e3154b1fa";
const MILADY = "0x5af0d9827e0c53e4799bb226655a1de152a425a5";
const config = [{ chain_id: 1n, relay_max_fee_bps: 300, deposits: false, collections: [
  { collection: MILADY, wrapper: "books", module: "books", max_purchase_wei: 1n, max_premium_bps: 500, chain_id: [], token_bound: [] },
  { collection: CASHCAT, wrapper: "books", module: "books", max_purchase_wei: 1n, max_premium_bps: 500, chain_id: [4663n], token_bound: [] }] }];

test("fixed C3-05: the sides' chains are the collections' other chains, in listing order", () => {
  assert.deepEqual(sideChainsOf(config), ["4663"]);
  assert.deepEqual(sideChainsOf([]), []);
  assert.deepEqual(sideChainsOf([{ ...config[0], collections: [config[0].collections[0]] }]), []);
});

test("fixed C3-05: the 4663 side is read with eth_nft_state_of(4663) and paged: halted line, quote-less Funded buy, late hold, scan lag", async () => {
  const st = fixture(6);
  st.halted = ["a transaction that is not ours took nonce 4 on 4663"];
  const id = (4663n << 40n);
  st.buys = [{ ...st.buys[0], id, phase: { Funded: { wei: 1_000_000_000n, since: BigInt(NOW - 3 * 3600) } }, late: [], last: [] }];
  const asked = [];
  const ic = { ethConfig: async () => config, ethStateOf: async (_b, chain) => { asked.push(String(chain)); return [st]; } };
  const r = await sideAlerts({ ic, buyer: "b", nowS: NOW });
  assert.deepEqual(asked, ["4663"], "the 4663 side, never the config's own chain again");
  const keys = r.alerts.map((a) => a.key);
  assert.ok(keys.includes("eth-line-halted:b:4663"), keys.join("\n"));
  assert.match(r.alerts.find((a) => a.key === "eth-line-halted:b:4663").text, /eth_line_clear_on\(4663\)/);
  assert.ok(keys.some((k) => k.startsWith(`eth-funded:b@chain4663:${id}:`)), keys.join("\n"));
  // the scan's lag in 4663's blocks: the fixture's 9,000 blocks behind is 15 minutes there, no page; 8 h of 4663's blocks is one
  assert.ok(!keys.some((k) => k.startsWith("eth-scan-behind:b@chain4663")), keys.join("\n"));
  st.sale_scan[0][0].behind = [BigInt(scanBlocksOf(4663).behind)];
  const r2 = await sideAlerts({ ic, buyer: "b", nowS: NOW });
  assert.ok(r2.alerts.some((a) => a.key.startsWith("eth-scan-behind:b@chain4663")), r2.alerts.map((a) => a.key).join("\n"));
  // the same state as the config's own side pages its scan lag in Ethereum's blocks
  st.sale_scan[0][0].behind = [9_000n];
  assert.ok(ethNftAlerts({ buyer: "b", state: st, nowS: NOW }).alerts.some((a) => a.key.startsWith("eth-scan-behind:b:")));
  // a late hold on the 4663 side pages too (it holds the Relay leg for every chain)
  const r3 = await sideAlerts({ ic: { ...ic, ethStateOf: async () => [fixture(6)] }, buyer: "b", nowS: NOW + 80 * 3600 });
  const hold = r3.alerts.find((a) => a.key.startsWith("eth-late-hold:b@chain4663"));
  assert.ok(hold && hold.sev === "SEV1", r3.alerts.map((a) => a.key).join("\n"));
});

test("fixed C3-05: a side the monitor cannot read pages; the config unread pages", async () => {
  const down = await sideAlerts({ ic: { ethConfig: async () => config, ethStateOf: async () => { throw new Error("replica down"); } }, buyer: "b", nowS: NOW });
  assert.deepEqual(down.alerts.map((a) => a.key), ["eth-side-unread:b:4663"]);
  const noCfg = await sideAlerts({ ic: { ethConfig: async () => { throw new Error("x"); }, ethStateOf: async () => [] }, buyer: "b", nowS: NOW });
  assert.deepEqual(noCfg.alerts.map((a) => a.key), ["eth-sides-unread:b"]);
});
