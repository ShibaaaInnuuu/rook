// Round-2 re-review (docs/audit/2026-09-27-nft/review2/R2-services.md), area D. Each test ASSERTS THE WEAKNESS: a pass means it is
// there; a fix turns it red. No network. The world is copied from the fixer's monitor/test/audit_nft_d.test.mjs.
import assert from "node:assert/strict";
import { test } from "node:test";
import { encodeFunctionData, encodeFunctionResult, parseAbi } from "viem";
import { checkEthNft } from "../src/ethnft.mjs";

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


// ---- D2-M-1 (a): every honest fill pages "ETH left without a fill of ours" until the canister's next tick ------------------------------
test("D2-M-1a: a fill of ours mined on Ethereum while the canister still says Buying (it reads the receipt at its next tick, up to RUNNER_TICK_EVERY_S = 600 s later): SEV1 'pause, then look'", async () => {
  const arrived = 97n * E18 / 100n, total = 93n * E18 / 100n, gas = 2n * 10n ** 15n, pot = 10n ** 16n;
  const state = { buys: [{ id: 1n, collection: blob(MILADY), module: blob(MODULE), wrapper: blob(WRAPPER), advanced: E18, attempts: 1, note: "",
    phase: { Buying: { arrived, token: tok(9180n), total, tx: blob("ab".repeat(32)) } } }],
    deliveries: [], readings: [], eth_out: 0n, deposits: [], last_mined: [[4n, 100n]], line: [{ kind: { Buy: { buy: 1n } }, nonce: 5n, hash: blob("ab".repeat(32)) }] };
  // the chain after the fill mined: the address paid `total` and the gas out of the arrived ETH; its gas pot is what is left
  const { http } = world({ items: [], owner: () => CUSTODIAN, balance: pot + arrived - total - gas, nonce: 6 });
  const r = await checkEthNft({ cfg: cfgOf(), now: 1_800_000_000_000, http, ic: { ethState: async () => state } });
  const sev1 = r.alerts.filter((a) => a.sev === "SEV1").map((a) => a.key);
  assert.ok(sev1.some((k) => k.startsWith("eth-arrived-gone:")), "an honest purchase reads as ETH stolen from the address");
});

// ---- D2-M-1 (b): every honest delivery pages "left custody without a recorded delivery" until recordDelivery lands on 4663 ------------
test("D2-M-1b: a delivery of ours mined on Ethereum (Moved: waiting for finality, then the 4663 record): the wrapper still holds the item, so SEV1 'an NFT left custody' on every redemption", async () => {
  const DEST = A(0xd00d);
  const state = { buys: [], deposits: [], readings: [], eth_out: 0n, last_mined: [[5n, 200n]], line: [],
    deliveries: [{ wrapper: blob(WRAPPER), collection: blob(MILADY), ticket: 3n, token: tok(9180n), destination: blob(DEST), deliver_by: 1_800_600_000n,
      phase: { Moved: { tx: blob("cd".repeat(32)), block: 200n } }, note: "" }] };
  // itemOf(9180) is still 1 on 4663 (EthNftWrapper deletes it only in recordDelivery); ownerOf on Ethereum is the redeemer
  const { http } = world({ items: [9180n], owner: () => DEST, nonce: 6 });
  const r = await checkEthNft({ cfg: cfgOf(), now: 1_800_000_000_000, http, ic: { ethState: async () => state } });
  assert.ok(r.alerts.some((a) => a.sev === "SEV1" && a.key.startsWith("eth-not-held:")), "an honest delivery reads as an NFT stolen from custody");
});

// ---- D2-L-4: one failed read of a collection resets its timers -------------------------------------------------------------------
test("D2-L-4: one failed read of a collection in a day drops its pool-idle timer: a pool due a buy with no buy for 25 h is never paged", async () => {
  const MODX = parseAbi(["function netUnits() view returns (uint256)", "function buffer() view returns (uint256)"]);
  const state = { buys: [], deliveries: [], readings: [], eth_out: 0n, deposits: [], last_mined: [], line: [] };
  const due = (fail) => { const w = world({ failItemCount: fail ? new Set([WRAPPER]) : new Set() });
    return async (url, init) => {
      const b = init?.body ? JSON.parse(String(init.body)) : null;
      const s = b?.params?.[0]?.data?.slice(0, 10);
      if (url === "https://home" && s === encodeFunctionData({ abi: MODX, functionName: "netUnits" }).slice(0, 10))
        return new Response(JSON.stringify({ jsonrpc: "2.0", id: 1, result: encodeFunctionResult({ abi: MODX, functionName: "netUnits", result: 2n * E18 }) }));
      return w.http(url, init);
    }; };
  const T = 1_800_000_000_000, H = 3_600_000;
  let prev = {}, paged = false;
  for (const [t, fail] of [[0, false], [12 * H, true], [25 * H, false], [30 * H, false]]) {
    const r = await checkEthNft({ cfg: cfgOf(), prev, now: T + t, http: due(fail), ic: { ethState: async () => state } });
    prev = r.next;
    if (r.alerts.some((a) => a.key.startsWith("eth-pool-idle:"))) paged = true;
  }
  assert.equal(paged, false, "due since hour 0, never paged by hour 30: the 12 h blip restarted the 24 h clock");
});
