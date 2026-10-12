// Round 3 of the 2026-09-27 NFT review, part D (review3/R3-services.md): monitor PoCs against HEAD 4949da0. Each test asserts the
// WEAKNESS: a pass means it is present. The world() fake is the fixer's own (monitor/test/audit_nft_d_r2.test.mjs). No network.
import assert from "node:assert/strict";
import { test } from "node:test";
import { encodeFunctionData, encodeFunctionResult, parseAbi } from "viem";
import { checkEthNft, earmarkOf } from "../src/ethnft.mjs";

const E18 = 10n ** 18n;
const A = (n) => `0x${n.toString(16).padStart(40, "0")}`;
const WRAPPER = A(0xaa), MODULE = A(0xbb), CUSTODIAN = A(0xc0), POSTER = A(0xf5), MILADY = "0x5af0d9827e0c53e4799bb226655a1de152a425a5";
const blob = (h) => Uint8Array.from(Buffer.from(h.replace(/^0x/, ""), "hex"));
const tok = (n) => blob(n.toString(16).padStart(64, "0"));

// ---- the world -------------------------------------------------------------------------------------------------------------------
const WRAP = parseAbi(["function itemCount() view returns (uint256)", "function item(uint256) view returns ((uint256 tokenId, bytes32 sourceTx, bool deposit))",
  "function itemOf(uint256) view returns (uint256)"]);
const MOD = parseAbi(["function accepted() view returns (uint256)", "function netUnits() view returns (uint256)", "function buffer() view returns (uint256)",
  "function maxItemCost() view returns (uint256)", "function registry() view returns (address)"]);
const ERC = parseAbi(["function ownerOf(uint256) view returns (address)"]);
const sel = (abi, fn, args = []) => encodeFunctionData({ abi, functionName: fn, args }).slice(0, 10);
function world({ items = [], accepted = 93n * E18 / 100n, os = 0.93, cg = 0.93, owner = () => CUSTODIAN, ownerErr = {}, head = 26_000_000, logs = [], balance = 10n ** 16n,
  failItemCount = new Set(), nonce = 0 }) {
  const calls = [];
  const http = async (url, init) => {
    if (url.includes("opensea.io")) { calls.push("opensea"); return os === null ? new Response("{}", { status: 429 }) : new Response(JSON.stringify({ total: { floor_price: os } })); }
    if (url.includes("coingecko")) { calls.push("coingecko"); return cg === null ? new Response(JSON.stringify({ status: { error_code: 429 } }), { status: 429 }) : new Response(JSON.stringify({ floor_price: { native_currency: cg } })); }
    const b = JSON.parse(String(init.body));
    const ok = (result) => new Response(JSON.stringify({ jsonrpc: "2.0", id: 1, result }));
    if (b.method === "eth_blockNumber") return ok(`0x${head.toString(16)}`);
    if (b.method === "eth_getBalance") return ok(`0x${balance.toString(16)}`);
    if (b.method === "eth_getTransactionCount") return ok(`0x${nonce.toString(16)}`);
    if (b.method === "eth_getLogs") { const from = Number(BigInt(b.params[0].fromBlock)); return ok(logs.filter((l) => Number(BigInt(l.blockNumber)) >= from)); }
    const { to, data } = b.params[0], s = data.slice(0, 10);
    if (url === "https://home") {
      if (s === sel(WRAP, "itemCount")) { if (failItemCount.has(to)) return new Response(JSON.stringify({ jsonrpc: "2.0", id: 1, error: { code: -32005, message: "the network is busy" } })); return ok(encodeFunctionResult({ abi: WRAP, functionName: "itemCount", result: BigInt(items.length) })); }
      if (s === sel(WRAP, "item", [0n])) { const i = Number(BigInt(`0x${data.slice(10)}`)); return ok(encodeFunctionResult({ abi: WRAP, functionName: "item", result: { tokenId: items[i], sourceTx: `0x${"11".repeat(32)}`, deposit: false } })); }
      if (s === sel(WRAP, "itemOf", [0n])) { const t = BigInt(`0x${data.slice(10)}`); return ok(encodeFunctionResult({ abi: WRAP, functionName: "itemOf", result: BigInt(items.indexOf(t) + 1) })); }
      if (s === sel(MOD, "accepted")) return ok(encodeFunctionResult({ abi: MOD, functionName: "accepted", result: accepted }));
      if (s === sel(MOD, "registry")) return ok(encodeFunctionResult({ abi: MOD, functionName: "registry", result: to === MODULE ? POSTER : A(0xf6) }));
      for (const fn of ["netUnits", "buffer", "maxItemCost"]) if (s === sel(MOD, fn)) return ok(encodeFunctionResult({ abi: MOD, functionName: fn, result: 0n }));
    }
    if (s === sel(ERC, "ownerOf", [0n])) {
      if (ownerErr[url]) return new Response(JSON.stringify({ jsonrpc: "2.0", id: 1, error: { code: -32000, message: ownerErr[url] } }));
      return ok(encodeFunctionResult({ abi: ERC, functionName: "ownerOf", result: owner(url, BigInt(`0x${data.slice(10)}`)) }));
    }
    throw new Error(`unexpected ${url} ${b.method} ${s}`);
  };
  return { http, calls };
}
const cfgOf = (cols = [{ slug: "milady", collection: MILADY, wrapper: WRAPPER, module: MODULE, custodian: CUSTODIAN, opensea_slug: "milady", coingecko_id: "milady-maker" }], extra = {}) =>
  ({ buyer: "rrkah-fqaaa-aaaaa-aaaaq-cai", eth_nft: { home_rpc: ["https://home"], eth_rpc: ["https://a", "https://b"], collections: cols, ...extra } });
const fakeGuardian = () => { const g = { autoPause: true, calls: [], registryPauses: 0,
  async pausePoster(p) { g.calls.push(p); return { done: true, tx: "0xpause" }; }, async pauseKeeper() { g.registryPauses++; return { done: true, tx: "0xreg" }; } }; return g; };
const stateWith = (reading) => ({ buys: [], deliveries: [], deposits: [], eth_out: 0n, last_mined: [], line: [],
  readings: reading ? [{ collection: blob(MILADY), post: { price: reading.price, confidence: 10_000, listing: reading.price, sale_median: reading.price }, at: BigInt(reading.at) }] : [] });

import { ethNftAlerts } from "../src/ethnft.mjs";

const DEST = A(0xd00d), BUYER2 = A(0xb0b), THIEF = A(0xbad);
const delivery = (phase, destination = DEST) => ({ wrapper: blob(WRAPPER), collection: blob(MILADY), ticket: 3n, token: tok(9180n), destination: blob(destination),
  deliver_by: 1_800_600_000n, phase, note: "" });
const notHeld = (r) => r.alerts.some((a) => a.sev === "SEV1" && a.key.startsWith("eth-not-held:"));

// ---- D3-M-1: an honest redeemer who moves the NFT on before recordDelivery lands (an arbitrageur selling it at once) pages SEV1 -----
test("D3-M-1: a delivery of ours mined, and the redeemer sold the NFT on before the 4663 record: SEV1 'an NFT left custody ... pause, then look'", async () => {
  for (const ph of [{ Sent: { tx: blob("cd".repeat(32)), nonce: 5n } }, { Moved: { tx: blob("cd".repeat(32)), block: 200n } }, { Record: { tx: blob("cd".repeat(32)) } },
    { Recording: { tx: blob("cd".repeat(32)) } }]) {
    const st = { buys: [], deposits: [], readings: [], eth_out: 0n, last_mined: [[5n, 200n]], line: [], deliveries: [delivery(ph)] };
    // itemOf(9180) is still set on 4663 (recordDelivery not landed); on Ethereum the redeemer DEST already sold it to BUYER2 (a Blur bid)
    const { http } = world({ items: [9180n], owner: () => BUYER2, nonce: 6 });
    const r = await checkEthNft({ cfg: cfgOf(), now: 1_800_000_000_000, http, ic: { ethState: async () => st } });
    assert.ok(notHeld(r), `${Object.keys(ph)[0]}: an honest redemption followed by a resale reads as an NFT stolen from custody`);
  }
});

test("D3-M-1 (b): a delivery pending a day before its deadline is cancelled; the transfer mined first (Cancelling, the canister not yet ticked): SEV1", async () => {
  const st = { buys: [], deposits: [], readings: [], eth_out: 0n, last_mined: [[5n, 200n]], line: [],
    deliveries: [delivery({ Cancelling: { of: blob("cd".repeat(32)), tx: blob("ce".repeat(32)), nonce: 6n } })] };
  const { http } = world({ items: [9180n], owner: () => DEST, nonce: 7 });
  const r = await checkEthNft({ cfg: cfgOf(), now: 1_800_000_000_000, http, ic: { ethState: async () => st } });
  assert.ok(notHeld(r), "the redeemer got the NFT through our own transfer, and the monitor calls it a custody loss");
});

// ---- D3-L-1: a stale Recorded delivery of the same token to the same address masks a real loss ------------------------------------
test("D3-L-1: token 9180 was redeemed to X long ago (Recorded); it came back (a deposit) and is held again; it leaves custody to X with no delivery: no SEV1", async () => {
  const X = DEST;
  const st = { buys: [], deposits: [], readings: [], eth_out: 0n, last_mined: [[9n, 900n]], line: [], deliveries: [delivery({ Recorded: { tx: blob("cd".repeat(32)) } }, X)] };
  const { http } = world({ items: [9180n], owner: () => X, nonce: 10 });
  const r = await checkEthNft({ cfg: cfgOf(), now: 1_800_000_000_000, http, ic: { ethState: async () => st } });
  assert.ok(!notHeld(r), "the wrapper holds item 9180 NOW, no delivery of it is open, and it is at X: masked by the old record");
  // the same for a token the canister BOUGHT again (Bought, not yet in the wrapper)
  const st2 = { ...st, buys: [{ id: 4n, collection: blob(MILADY), module: blob(MODULE), wrapper: blob(WRAPPER), advanced: E18, attempts: 1, note: "",
    phase: { Bought: { arrived: E18, token: tok(9180n), total: E18 / 2n, tx: blob("ab".repeat(32)), block: 950n } } }] };
  const { http: h2 } = world({ items: [], owner: () => X, nonce: 10 });
  const r2 = await checkEthNft({ cfg: cfgOf(), now: 1_800_000_000_000, http: h2, ic: { ethState: async () => st2 } });
  assert.ok(!notHeld(r2), "a bought token at X is masked too, though no delivery can exist for a token not yet in the wrapper");
  // control: any other owner still pages
  const { http: h3 } = world({ items: [9180n], owner: () => THIEF, nonce: 10 });
  assert.ok(notHeld(await checkEthNft({ cfg: cfgOf(), now: 1_800_000_000_000, http: h3, ic: { ethState: async () => st } })));
});

// ---- D3-L-2: the balance is read at the poll's first block, the earmarks at its end: an arrival in between is "ETH gone" ------------
test("D3-L-2: Relay's arrival lands after the canary's block but before its second records read (canister pin newer): SEV1 'ETH left without a fill of ours'", async () => {
  const pot = 10n ** 16n, wei = 97n * E18 / 100n;
  const base = { deliveries: [], readings: [], eth_out: 0n, deposits: [], last_mined: [[4n, 100n]], line: [] };
  const bridging = { ...base, buys: [{ id: 1n, collection: blob(MILADY), module: blob(MODULE), wrapper: blob(WRAPPER), advanced: E18, attempts: 1, note: "",
    phase: { Bridging: { order_id: blob("11".repeat(32)), value: E18, min_out: wei, deadline: 1_800_100_000n, since: 1_799_999_000n } } }] };
  const arrived = { ...base, buys: [{ ...bridging.buys[0], phase: { Arrived: { wei, since: 1_800_000_000n } } }] };
  let n = 0;
  const ic = { ethState: async () => (n++ === 0 ? bridging : arrived) };     // the canister ticked during the poll and saw the arrival
  const { http } = world({ items: [], owner: () => CUSTODIAN, balance: pot, nonce: 5 });    // at the canary's block the ETH was not there yet
  const r = await checkEthNft({ cfg: cfgOf(), now: 1_800_000_000_000, http, ic });
  assert.ok(r.alerts.some((a) => a.sev === "SEV1" && a.key === `eth-arrived-gone:${CUSTODIAN}`), "an honest arrival reads as ETH stolen");
});

// ---- D3-I-1: eth_give_back on an Arrived buy stores since = 0 until the next tick: an immediate SEV2 "waited 29 million min" -------------
test("D3-I-1: right after eth_give_back (BackFunded since 0), the way-back page fires at once with an absurd wait", () => {
  const st = { buys: [{ id: 7n, collection: blob(MILADY), module: blob(MODULE), wrapper: blob(WRAPPER), advanced: E18, attempts: 1, note: "",
    phase: { BackFunded: { wei: E18, since: 0n } } }], deliveries: [], deposits: [] };
  const r = ethNftAlerts({ buyer: "b", state: st, nowS: 1_800_000_000 });
  const a = r.alerts.find((x) => x.key.startsWith("eth-back-funded:"));
  assert.ok(a && a.key.endsWith(":0") && /30000000 min/.test(a.text), a?.text);
});

// ---- D3-I-2: a foreign transaction at a CancelFill line's nonce: the canister does not halt, and the allowance hides the drain -------------
test("D3-I-2: the line is our cancel of a fill; a transaction not ours takes that nonce and empties the address: no SEV1 from the canary", async () => {
  const arrived = 97n * E18 / 100n, total = 95n * E18 / 100n;
  const st = { buys: [{ id: 1n, collection: blob(MILADY), module: blob(MODULE), wrapper: blob(WRAPPER), advanced: E18, attempts: 1, note: "",
    phase: { Buying: { arrived, token: tok(9180n), total, tx: blob("ab".repeat(32)) } } }],
    deliveries: [], readings: [], eth_out: 0n, deposits: [], last_mined: [[4n, 100n]],
    // the cancel re-priced over 6 h of a stuck fill: 100 gwei max fee, 21k gas (the canary allows 300k at it)
    line: [{ kind: { CancelFill: { buy: 1n } }, nonce: 5n, hash: blob("ce".repeat(32)), value: 0n, max_fee: 100_000_000_000n, gas: 21_000n }] };
  const { http } = world({ items: [], owner: () => CUSTODIAN, balance: 0n, nonce: 6 });
  const r = await checkEthNft({ cfg: cfgOf(), now: 1_800_000_000_000, http, ic: { ethState: async () => st } });
  const sev1 = r.alerts.filter((a) => a.sev === "SEV1").map((a) => a.key);
  assert.deepEqual(sev1, [], "0.97 ETH earmarked, the address holds 0, and nothing pages SEV1 (ethflow.rs superseded(): CancelFill never halts)");
  assert.equal(earmarkOf(st.buys[0], st.line[0], 6n), 0n);
});
