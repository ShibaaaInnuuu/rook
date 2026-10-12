// Fix round 5 of the 2026-09-27 NFT review (docs/audit/2026-09-27-nft/fixes-round5.md): the monitor's PoCs of review round 5, part D
// (review5/poc/D/monitor/test/audit5_nft_d.test.mjs) turned around, on the same fake world and the canister's own bytes (test/d5-state.hex:
// review 5's d5_fixture.rs on the 1dc2a61 build; the canister's eth-state-round5.hex: this round's), and the round-5 pages (F5-M-1's
// unseen home transaction, C5-M-1's hold). Each FAILS on the code before this round.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { IDL } from "@icp-sdk/core/candid";
import { encodeFunctionData, encodeFunctionResult, parseAbi } from "viem";
import { EthState, plain } from "../src/candid.mjs";
import { checkEthNft, ethNftAlerts } from "../src/ethnft.mjs";

const E18 = 10n ** 18n;
const A = (n) => `0x${n.toString(16).padStart(40, "0")}`;
const rep = (b, n = 20) => `0x${b.repeat(n)}`;
const WRAPPER = rep("aa"), MODULE = rep("bb"), DEST = rep("d0"), CUSTODIAN = A(0xc0), POSTER = A(0xf5), MILADY = "0x5af0d9827e0c53e4799bb226655a1de152a425a5";
const BUYER2 = A(0xb0b), THIEF = A(0xbad), OTHER = A(0x0ce);
const NOW = 1_800_000_000_000;
const pad = (x) => `0x${BigInt(x).toString(16).padStart(64, "0")}`;
const padA = (a) => `0x${"0".repeat(24)}${a.slice(2)}`;
const TRANSFER_TOPIC = "0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef";

const WRAP = parseAbi(["function itemCount() view returns (uint256)", "function item(uint256) view returns ((uint256 tokenId, bytes32 sourceTx, bool deposit))",
  "function itemOf(uint256) view returns (uint256)",
  "function ticket(uint256) view returns ((uint8 stage, address owner, uint64 round, uint64 pinnedAt, uint64 pinnedAtBlock, uint64 deliverBy, uint64 deliverByBlock, uint256 itemId, bytes32 deliveryTx, address destination))"]);
const MOD = parseAbi(["function accepted() view returns (uint256)", "function netUnits() view returns (uint256)", "function buffer() view returns (uint256)",
  "function maxItemCost() view returns (uint256)", "function registry() view returns (address)"]);
const ERC = parseAbi(["function ownerOf(uint256) view returns (address)"]);
const sel = (abi, fn, args = []) => encodeFunctionData({ abi, functionName: fn, args }).slice(0, 10);
const err = (message) => new Response(JSON.stringify({ jsonrpc: "2.0", id: 1, error: { code: -32000, message } }));
/**
 * The fixer's fake world (audit_nft_d_r4.test.mjs), with `receipt(url, hash)` per provider (null: not mined there; "error": the
 * provider errors) and `logs(url, from, to)`.
 */
function world({ items = [], owner = () => CUSTODIAN, head = () => 26_000_000, balance = 10n ** 17n, nonce = 7, tickets = {}, logs = () => [], receipt = () => null }) {
  const calls = [];
  const http = async (url, init) => {
    if (url.includes("opensea.io")) return new Response(JSON.stringify({ total: { floor_price: 0.93 } }));
    if (url.includes("coingecko")) return new Response(JSON.stringify({ floor_price: { native_currency: 0.93 } }));
    const b = JSON.parse(String(init.body));
    calls.push(b.method);
    const ok = (result) => new Response(JSON.stringify({ jsonrpc: "2.0", id: 1, result }));
    if (b.method === "eth_blockNumber") return ok(`0x${head(url).toString(16)}`);
    if (b.method === "eth_getBalance") return ok(`0x${balance.toString(16)}`);
    if (b.method === "eth_getTransactionCount") return ok(`0x${nonce.toString(16)}`);
    if (b.method === "eth_getTransactionReceipt") {
      const r = receipt(url, String(b.params[0]).toLowerCase());
      calls.push(`receipt:${url}:${String(b.params[0]).slice(0, 6)}`);
      return r === "error" ? err("internal error") : ok(r ?? null);
    }
    if (b.method === "eth_getLogs") { const p = b.params[0]; const a = Number(BigInt(p.fromBlock)), z = Number(BigInt(p.toBlock)); calls.push(`logs:${a}-${z}`); return ok(logs(url, a, z)); }
    const { data } = b.params[0], s = data.slice(0, 10);
    if (url === "https://home") {
      if (s === sel(WRAP, "itemCount")) return ok(encodeFunctionResult({ abi: WRAP, functionName: "itemCount", result: BigInt(items.length) }));
      if (s === sel(WRAP, "item", [0n])) { const i = Number(BigInt(`0x${data.slice(10)}`)); return ok(encodeFunctionResult({ abi: WRAP, functionName: "item", result: { tokenId: items[i], sourceTx: `0x${"11".repeat(32)}`, deposit: false } })); }
      if (s === sel(WRAP, "itemOf", [0n])) { const t = BigInt(`0x${data.slice(10)}`); return ok(encodeFunctionResult({ abi: WRAP, functionName: "itemOf", result: BigInt(items.indexOf(t) + 1) })); }
      if (s === sel(WRAP, "ticket", [0n])) {
        const id = String(BigInt(`0x${data.slice(10)}`)), t = tickets[id];
        calls.push(`ticket:${id}`);
        if (!t) return err("execution reverted");
        return ok(encodeFunctionResult({ abi: WRAP, functionName: "ticket", result: { stage: t.stage ?? 2, owner: A(0x1), round: 1n, pinnedAt: 1n, pinnedAtBlock: 1n, deliverBy: 1n,
          deliverByBlock: 1n, itemId: t.itemId, deliveryTx: t.deliveryTx ?? `0x${"00".repeat(32)}`, destination: t.destination ?? DEST } }));
      }
      if (s === sel(MOD, "accepted")) return ok(encodeFunctionResult({ abi: MOD, functionName: "accepted", result: 93n * E18 / 100n }));
      if (s === sel(MOD, "registry")) return ok(encodeFunctionResult({ abi: MOD, functionName: "registry", result: POSTER }));
      for (const fn of ["netUnits", "buffer", "maxItemCost"]) if (s === sel(MOD, fn)) return ok(encodeFunctionResult({ abi: MOD, functionName: fn, result: 0n }));
    }
    if (s === sel(ERC, "ownerOf", [0n])) return ok(encodeFunctionResult({ abi: ERC, functionName: "ownerOf", result: owner(url, BigInt(`0x${data.slice(10)}`)) }));
    throw new Error(`unexpected ${url} ${b.method} ${s}`);
  };
  return { http, calls };
}
const cfgOf = (x = {}) => ({ buyer: "rrkah-fqaaa-aaaaa-aaaaq-cai", eth_nft: { home_rpc: ["https://home"], eth_rpc: ["https://a", "https://b"],
  collections: [{ slug: "milady", collection: MILADY, wrapper: WRAPPER, module: MODULE, custodian: CUSTODIAN, opensea_slug: "milady", coingecko_id: "milady-maker" }], ...x } });
const notHeld = (r) => r.alerts.filter((a) => a.sev === "SEV1" && a.key.startsWith("eth-not-held:"));
const decodeHex = (h) => plain(IDL.decode([EthState], Uint8Array.from(Buffer.from(h.trim(), "hex")))[0]);
const d4 = () => decodeHex(readFileSync(new URL("./d4-state.hex", import.meta.url), "utf8"));
const d5 = () => decodeHex(readFileSync(new URL("./d5-state.hex", import.meta.url), "utf8"));
const rcpt = ({ to = DEST, token, from = CUSTODIAN, address = MILADY, status = "0x1" }) => ({ status, blockNumber: "0x18cba7f",
  logs: [{ address, topics: [TRANSFER_TOPIC, padA(from), padA(to), pad(token)], data: "0x" }] });
const hx = (s) => `0x${String(s).replace(/^0x/, "").toLowerCase()}`;

const r5 = () => decodeHex(readFileSync(new URL("../../autobuyer/canister-rs/tests/fixtures/eth-state-round5.hex", import.meta.url), "utf8"));

test("D5-L-1 fixed: a delivery whose record lands during the poll (Delivered by THIS delivery) is exempt; another record or a refund still pages", async () => {
  const st = d5();
  st.deliveries = st.deliveries.filter((d) => String(d.ticket) === "7");   // ticket 7, token 9188, Recording (canister bytes)
  const tx = hx(st.deliveries[0].phase.Recording.tx);
  const run = async (t) => {
    const w = world({ items: [9188n], tickets: { 7: { itemId: 0n, ...t } }, owner: (u, x) => (x === 9188n ? BUYER2 : CUSTODIAN) });
    return notHeld(await checkEthNft({ cfg: cfgOf(), now: NOW, http: w.http, ic: { ethState: async () => st } })).length;
  };
  assert.equal(await run({ stage: 3, deliveryTx: tx }), 0, "recorded by this very delivery: exempt");
  assert.equal(await run({ stage: 2 }), 0, "still Resolved: exempt");
  assert.equal(await run({ stage: 3, deliveryTx: rep("99", 32) }), 1, "recorded by another transaction: pages");
  assert.equal(await run({ stage: 4, deliveryTx: tx }), 1, "refunded: pages (D4-I-1)");
});

test("D5-L-2 fixed: a fill-Stuck buy is 'not watched' (pause, then look); a watched one is watched; a written-off sweep closes with nothing", () => {
  const st = d5();
  const a = ethNftAlerts({ buyer: "b", state: st, nowS: 1_800_000_000 }).alerts;
  const stuck = (id) => a.find((x) => x.key === `eth-stuck:b:${id}`);
  assert.match(stuck(1).text, /still WATCHED/, "buy 1: a watched forward order");
  assert.match(stuck(2).text, /NOT watched/);
  assert.match(stuck(2).text, /Pause, then look/);
  assert.doesNotMatch(stuck(2).text, /still WATCHED|not before its deadline/);
  assert.match(stuck(6).text, /closes the sweep with nothing/);
  assert.doesNotMatch(stuck(6).text, /returnAdvance of 0/);
  assert.ok(Array.isArray(st.buys[0].last), "the IDL decodes `last` now");
});

test("F5-M-1: a pending 4663 transaction seen nowhere with its nonce used pages SEV1 after an hour, naming eth_home_clear", () => {
  const st = r5();
  const p = st.pending[0];
  assert.equal(String(p.used_since[0]), "6");
  const at = (nowS) => ethNftAlerts({ buyer: "b", state: st, nowS }).alerts.filter((x) => x.key.startsWith("eth-home-unseen:"));
  assert.equal(at(6 + 3_599).length, 0);
  const a = at(6 + 3_600);
  assert.equal(a.length, 1);
  assert.equal(a[0].sev, "SEV1");
  assert.match(a[0].text, /eth_home_clear\(7, true\)/);
  const none = { ...st, pending: [] };
  assert.equal(ethNftAlerts({ buyer: "b", state: none, nowS: 1e9 }).alerts.filter((x) => x.key.startsWith("eth-home-unseen:")).length, 0);
});

test("C5-M-1: a written-off order's pages say it holds the Relay leg until credited or released (eth_late_release); a released one says so", () => {
  const st = r5();
  const a = ethNftAlerts({ buyer: "b", state: st, nowS: 1_800_000_000 }).alerts;
  const late = (id) => a.find((x) => x.key === `eth-late-order:b:${id}`);
  assert.match(late(1).text, /holds the Relay leg for new orders .*eth_late_release\(1\)/);
  assert.match(late(2).text, /released its hold/);
  assert.match(a.find((x) => x.key === "eth-stuck:b:1").text, /still holds the Relay leg for new orders/);
});

test("the round-5 state (every new field, Inbox.payouts too) decodes through the monitor's IDL", () => {
  const st = r5();
  assert.equal(st.buys.length, 3);
  assert.equal(String(st.buys[2].late[0].released_at[0]), "6");
  assert.equal(st.buys[1].late[0].credited.length, 0);
});
