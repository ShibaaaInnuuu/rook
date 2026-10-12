// D8 per collection (the operator, 2026-10-09; deployments/launch/nft-pilot.json `priceRefs` / `_priceRefs`): Identity MD, which CoinGecko
// does not list, has OpenSea as its ONE price reference. Its poster pause fires at more than 20 % from OpenSea; a missing OpenSea answer
// pages (the floor is not watched) and never pauses; every other collection still needs both references. The monitor's loader accepts
// price_refs ["opensea"] only in that shape and only where the launch file decides it. No network.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { encodeFunctionData, encodeFunctionResult, parseAbi } from "viem";
import { checkEthNft, floorAlerts, openseaAlone } from "../src/ethnft.mjs";
import { validate } from "../src/config.mjs";

const E = 10n ** 18n;
const A = (n) => `0x${n.toString(16).padStart(40, "0")}`;
const IDMD = "0x0000ec93127baa929e58e97dd0095a2bfb38ec1d";
const WRAPPER = A(0xaa), MODULE = A(0xbb), CUSTODIAN = A(0xc0), POSTER = A(0xf5);
const nowS = 1_800_000_000;
const fresh = (price) => ({ price, at: nowS - 60 });
const OS = (v) => [{ name: "OpenSea", value: v }];

test("floorAlerts, OpenSea alone: a fresh reading more than 20 % from OpenSea (same side) pauses, confirmed by its one reference", () => {
  const r = floorAlerts({ slug: "identitymd", module: "m", accepted: 178n * E / 100n, reading: fresh(130n * E / 100n), refs: OS(178n * E / 100n), nowS, alone: true });
  assert.equal(r.pause, true);
  const a = r.alerts.find((x) => x.key === "eth-floor-far:m");
  assert.equal(a.sev, "SEV1");
  assert.match(a.text, /its ONE public reference/);
  assert.match(a.text, /OpenSea alone: price_refs, the operator's decision of 2026-10-09/);
  // above it too (a high manipulated reading)
  assert.equal(floorAlerts({ slug: "identitymd", module: "m", accepted: 178n * E / 100n, reading: fresh(220n * E / 100n), refs: OS(178n * E / 100n), nowS, alone: true }).pause, true);
});

test("floorAlerts, OpenSea alone: within 20 % does not pause; latched does not pause again", () => {
  assert.equal(floorAlerts({ slug: "identitymd", module: "m", accepted: 178n * E / 100n, reading: fresh(150n * E / 100n), refs: OS(178n * E / 100n), nowS, alone: true }).pause, false);
  const l = floorAlerts({ slug: "identitymd", module: "m", accepted: 178n * E / 100n, reading: fresh(100n * E / 100n), refs: OS(178n * E / 100n), nowS, alone: true, latched: true });
  assert.equal(l.pause, false);
  assert.ok(l.alerts.some((x) => x.key === "eth-floor-far-latched:m"));
});

test("floorAlerts, OpenSea alone: a missing OpenSea answer never pauses by itself, and pages that the floor is not watched (as for every collection)", () => {
  const r = floorAlerts({ slug: "identitymd", module: "m", accepted: 178n * E / 100n, reading: fresh(50n * E / 100n), refs: OS(0n), nowS, alone: true });
  assert.equal(r.pause, false);
  const a = r.alerts.find((x) => x.key === "eth-floor-unwatched:m");
  assert.equal(a.sev, "SEV2");
  assert.match(a.text, /NOT WATCHED/);
});

test("floorAlerts, every other collection: one reference read (the other missing, or never configured) still never pauses", () => {
  // a collection that is NOT opted in, with only OpenSea read: refused as before (D8 needs two that agree)
  const one = floorAlerts({ slug: "milady", module: "m", accepted: E, reading: fresh(E / 2n), refs: OS(E), nowS });
  assert.equal(one.pause, false);
  assert.ok(one.alerts.some((x) => x.key === "eth-floor-far-unconfirmed:m"));
  const both = floorAlerts({ slug: "milady", module: "m", accepted: E, reading: fresh(E / 2n), refs: [{ name: "OpenSea", value: E }, { name: "CoinGecko", value: 0n }], nowS });
  assert.equal(both.pause, false);
  // `alone` with two references configured is not "one": both are still needed
  const two = floorAlerts({ slug: "x", module: "m", accepted: E, reading: fresh(E / 2n), refs: [{ name: "OpenSea", value: E }, { name: "CoinGecko", value: 0n }], nowS, alone: true });
  assert.equal(two.pause, false);
});

test("openseaAlone: exactly price_refs [\"opensea\"]", () => {
  assert.equal(openseaAlone({ price_refs: ["opensea"] }), true);
  for (const v of [undefined, [], ["coingecko"], ["opensea", "coingecko"], "opensea"]) assert.equal(openseaAlone({ price_refs: v }), false, JSON.stringify(v));
});

// ---- checkEthNft end to end: the reference fetch and the guardian ------------------------------------------------------------------
const WRAP = parseAbi(["function itemCount() view returns (uint256)"]);
const MOD = parseAbi(["function accepted() view returns (uint256)", "function netUnits() view returns (uint256)", "function buffer() view returns (uint256)",
  "function maxItemCost() view returns (uint256)", "function registry() view returns (address)"]);
const sel = (abi, fn, args = []) => encodeFunctionData({ abi, functionName: fn, args }).slice(0, 10);
function world({ os = 1.78, accepted = 178n * E / 100n }) {
  const calls = [];
  const http = async (url, init) => {
    if (url.includes("opensea.io")) { calls.push("opensea"); return os === null ? new Response("{}", { status: 429 }) : new Response(JSON.stringify({ total: { floor_price: os } })); }
    if (url.includes("coingecko")) { calls.push("coingecko"); return new Response(JSON.stringify({ floor_price: { native_currency: 1.78 } })); }
    const b = JSON.parse(String(init.body));
    const ok = (result) => new Response(JSON.stringify({ jsonrpc: "2.0", id: 1, result }));
    if (b.method === "eth_blockNumber") return ok("0x18cba80");
    if (b.method === "eth_getBalance") return ok("0x2386f26fc10000");
    if (b.method === "eth_getTransactionCount") return ok("0x0");
    if (b.method === "eth_getLogs") return ok([]);
    const s = b.params[0].data.slice(0, 10);
    if (s === sel(WRAP, "itemCount")) return ok(encodeFunctionResult({ abi: WRAP, functionName: "itemCount", result: 0n }));
    if (s === sel(MOD, "accepted")) return ok(encodeFunctionResult({ abi: MOD, functionName: "accepted", result: accepted }));
    if (s === sel(MOD, "registry")) return ok(encodeFunctionResult({ abi: MOD, functionName: "registry", result: POSTER }));
    for (const fn of ["netUnits", "buffer", "maxItemCost"]) if (s === sel(MOD, fn)) return ok(encodeFunctionResult({ abi: MOD, functionName: fn, result: 0n }));
    throw new Error(`unexpected ${url} ${b.method} ${s}`);
  };
  return { http, calls };
}
const cfgOf = (col) => ({ buyer: "rrkah-fqaaa-aaaaa-aaaaq-cai", eth_nft: { home_rpc: ["https://home"], eth_rpc: ["https://a", "https://b"],
  collections: [{ slug: "identitymd", collection: IDMD, wrapper: WRAPPER, module: MODULE, custodian: CUSTODIAN, opensea_slug: "identitymd", ...col }] } });
const guardian = () => { const g = { autoPause: true, calls: [], async pausePoster(p) { g.calls.push(p); return { done: true, tx: "0xpause" }; } }; return g; };
const stateAt = (now, price) => ({ buys: [], deliveries: [], deposits: [], eth_out: 0n, last_mined: [], line: [],
  readings: [{ collection: Uint8Array.from(Buffer.from(IDMD.slice(2), "hex")), post: { price, confidence: 10_000, listing: price, sale_median: price }, at: BigInt(now / 1000 - 60) }] });

test("checkEthNft: Identity MD (price_refs [\"opensea\"]) pauses its poster on OpenSea alone, and never asks CoinGecko", async () => {
  const g = guardian(), w = world({ os: 1.78 }), now = nowS * 1000;
  const st = stateAt(now, 120n * E / 100n);
  const r = await checkEthNft({ cfg: cfgOf({ price_refs: ["opensea"] }), now, http: w.http, guardian: g, ic: { ethState: async () => st } });
  assert.deepEqual(g.calls, [POSTER], "one pause(), on this collection's PosterSource");
  assert.ok(r.alerts.some((a) => a.key === `eth-floor-pause:${MODULE}:${nowS}`));
  assert.ok(!w.calls.includes("coingecko"));
});

test("checkEthNft: Identity MD with OpenSea down pages the floor unwatched and pauses nothing", async () => {
  const g = guardian(), w = world({ os: null }), now = nowS * 1000;
  const r = await checkEthNft({ cfg: cfgOf({ price_refs: ["opensea"] }), now, http: w.http, guardian: g, ic: { ethState: async () => stateAt(now, 50n * E / 100n) } });
  assert.equal(g.calls.length, 0);
  assert.ok(r.alerts.some((a) => a.key === `eth-floor-unwatched:${MODULE}` && a.sev === "SEV2"));
});

test("checkEthNft: the same collection WITHOUT price_refs (OpenSea only configured) is not opted in: no pause on one reference", async () => {
  const g = guardian(), w = world({ os: 1.78 }), now = nowS * 1000;
  const r = await checkEthNft({ cfg: cfgOf({}), now, http: w.http, guardian: g, ic: { ethState: async () => stateAt(now, 120n * E / 100n) } });
  assert.equal(g.calls.length, 0);
  assert.ok(r.alerts.some((a) => a.key === `eth-floor-far-unconfirmed:${MODULE}`));
});

// ---- the loader: price_refs only as ["opensea"], with no coingecko_id, and only where the launch file decides it ---------------------
const LAUNCH = JSON.parse(readFileSync(new URL("../../deployments/launch/nft-pilot.json", import.meta.url), "utf8"));
const RAW = readFileSync(new URL("../../deployments/launch/monitor-nft.json", import.meta.url), "utf8");
const filled = () => {
  const fill = (s) => s.replace(/"<the canister's (custody|float|buyer)_address\(\)>"/g, (_, k) => `"bc1p${{ custody: "q", float: "p", buyer: "z" }[k].repeat(58)}"`)
    .replace(/"<[^"]*canister id>"/g, '"rrkah-fqaaa-aaaaa-aaaaq-cai"').replace(/"<[^"]*(EOA|address|wrapper|floorModule)[^"]*>"/g, `"${A(0xabc)}"`)
    .replace(/"<[^"]*>"/g, '"x"').replace(/https:\/\/<[^"]*>/g, "https://host.example");
  const cfg = JSON.parse(fill(RAW));
  cfg.builds = [{ commit: "c", buyer: "a".repeat(64), timelock: "b".repeat(64) }];
  delete cfg.install_arg_hex;
  cfg.evm.stack = { registry: A(0x1), chainId: 4663 };
  cfg.launch = structuredClone(LAUNCH);
  return cfg;
};
const idx = (cfg, slug) => cfg.eth_nft.collections.findIndex((c) => c.slug === slug);

test("the loader: the committed launch config (Identity MD on OpenSea alone, as the launch file decides) is valid", () => {
  const cfg = filled();
  assert.deepEqual(cfg.eth_nft.collections[idx(cfg, "identitymd")].price_refs, ["opensea"]);
  assert.doesNotThrow(() => validate(cfg));
});

test("the loader refuses price_refs in another shape, beside a coingecko_id, or where the launch file does not decide it; and a missing coingecko_id elsewhere", () => {
  let c = filled(); c.eth_nft.collections[idx(c, "identitymd")].price_refs = ["opensea", "coingecko"];
  assert.throws(() => validate(c), /identitymd: price_refs is either absent \(both references\) or exactly \["opensea"\]/);
  c = filled(); c.eth_nft.collections[idx(c, "identitymd")].coingecko_id = "identity-md";
  assert.throws(() => validate(c), /identitymd: price_refs \["opensea"\] reads OpenSea alone: remove coingecko_id/);
  c = filled(); const m = c.eth_nft.collections[idx(c, "milady")]; delete m.coingecko_id; m.price_refs = ["opensea"];
  assert.throws(() => validate(c), /milady: price_refs \["opensea"\] where the launch file \(nft-pilot.json\) does not decide priceRefs \["opensea"\] for this collection/);
  c = filled(); delete c.launch.collections.find((x) => x.slug === "identitymd").priceRefs;
  assert.throws(() => validate(c), /identitymd: price_refs \["opensea"\] where the launch file/);
  c = filled(); delete c.eth_nft.collections[idx(c, "mayc")].coingecko_id;
  assert.throws(() => validate(c), /mayc: D8's pause needs both references/);
  c = filled(); delete c.eth_nft.collections[idx(c, "identitymd")].price_refs;
  assert.throws(() => validate(c), /identitymd: D8's pause needs both references/);
});
