// Audit 2026-09-27-nft, fix round 1 (docs/audit/2026-09-27-nft/fixes-round1.md): reviewer D's monitor PoCs
// (docs/audit/2026-09-27-nft/poc/D/monitor/test/audit_nft_d.test.mjs) as regressions. Each drives the PoC's world and asserts the
// weakness is gone. No network.
import assert from "node:assert/strict";
import { test } from "node:test";
import { encodeAbiParameters, encodeEventTopics, encodeFunctionData, encodeFunctionResult, parseAbi, parseAbiItem } from "viem";
import { checkEthNft } from "../src/ethnft.mjs";
import { judgeLogs, judgeStackLogs, TOPICS } from "../src/evm.mjs";
import { EMPTY_RECORDS, recordIndex } from "../src/records.mjs";
import { stackOf } from "../src/stack.mjs";

const E18 = 10n ** 18n;
const A = (n) => `0x${n.toString(16).padStart(40, "0")}`;
const WRAPPER = A(0xaa), MODULE = A(0xbb), REGISTRY = A(0xd1), CUSTODIAN = A(0xc0), POSTER = A(0xf5), MILADY = "0x5af0d9827e0c53e4799bb226655a1de152a425a5";
const blob = (h) => Uint8Array.from(Buffer.from(h.replace(/^0x/, ""), "hex"));

const NFT_STACK = { chainId: 4663, registry: REGISTRY, treasury: A(0xc7), poolManager: A(0xc8), keeper: A(0xc3), fromBlock: 1,
  collections: [{ slug: "milady", kind: "nft", wrapper: WRAPPER, floorModule: MODULE, raffleVault: A(0xe1), burnVault: A(0xe2), yoyoVault: A(0xe3),
    factory: A(0xf1), lock: A(0xf2), router: A(0xf3), mintDesk: A(0xf4), floorPoster: CUSTODIAN, posterSource: POSTER,
    custody: { kind: "evm", chainId: 1, collection: MILADY, custodian: CUSTODIAN } }] };
const FP = parseAbiItem("event FloorPosted(uint256 target, uint256 accepted, uint256 moveBps, uint16 confidenceBps, uint32 depthItems)");
const floorPosted = (target, accepted, n) => ({ address: MODULE, topics: encodeEventTopics({ abi: [FP], eventName: "FloorPosted" }),
  data: encodeAbiParameters(FP.inputs, [target, accepted, 250n, 10_000, 5]), blockNumber: `0x${(1000 + n).toString(16)}`,
  transactionHash: `0x${String(n).padStart(64, "0")}`, logIndex: "0x0", blockHash: `0x${String(n).padStart(64, "0")}`, removed: false });

// ---- D-H-2 ------------------------------------------------------------------------------------------------------------------------
test("D-H-2: the canister posts inside floorBounds(), so its honest moves page nothing; an out-of-plan post stops THIS collection's poster, never the registry", () => {
  const st = stackOf({ stack: NFT_STACK });
  const T0 = 1_800_000_000_000;
  // the canister's reading moved to 1.00; it posts what the module allows (target = accepted), 30 minutes apart
  const p1 = floorPosted(95_325n * E18 / 100_000n, 95_325n * E18 / 100_000n, 1);
  const p2 = floorPosted(977_081_250_000_000_000n, 977_081_250_000_000_000n, 2);
  const ctx = (t) => ({ st, now: t, minConf: { [MODULE]: 3000n }, times: { [`${p1.transactionHash}:0`]: T0, [`${p2.transactionHash}:0`]: T0 + 1_800_000 } });
  const one = judgeStackLogs({ logs: [p1], prev: {}, ctx: ctx(T0) });
  const two = judgeStackLogs({ logs: [p2], prev: one.next, ctx: ctx(T0 + 1_800_000) });
  assert.deepEqual([...one.alerts, ...two.alerts].filter((a) => a.sev === "SEV1"), [], "no 'stolen key' page for an honest move");
  assert.deepEqual(two.triggers, [], "and no pause");
  // a post outside the bounds (the canister's key used by something else): SEV1, and the runaway stops the collection's poster only
  const q1 = floorPosted(100n * E18 / 100n, 95_325n * E18 / 100_000n, 3), q2 = floorPosted(100n * E18 / 100n, 977_081_250_000_000_000n, 4);
  const ctx2 = (t) => ({ st, now: t, minConf: { [MODULE]: 3000n }, times: { [`${q1.transactionHash}:0`]: T0, [`${q2.transactionHash}:0`]: T0 + 1_800_000 } });
  const a = judgeStackLogs({ logs: [q1], prev: {}, ctx: ctx2(T0) });
  assert.match(a.alerts[0].text, /THE CANISTER \(THIS COLLECTION'S POSTER\) POSTED/);
  const b = judgeStackLogs({ logs: [q2], prev: a.next, ctx: ctx2(T0 + 1_800_000) });
  assert.equal(b.triggers.length, 1);
  assert.deepEqual([b.triggers[0].action, b.triggers[0].poster], ["pause-poster", POSTER], "never pauseKeeper() on the registry");
});

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

// ---- D-M-1 ------------------------------------------------------------------------------------------------------------------------
test("D-M-1: a real 21 % drop the module must lag pauses nothing (the canister's own reading follows the market), and pages one stable key", async () => {
  const g = fakeGuardian();
  const { http } = world({ accepted: 93n * E18 / 100n, os: 0.73, cg: 0.73 });
  const keys = new Set();
  let prev = {};
  for (let k = 0; k < 5; k++) {
    const now = 1_800_000_000_000 + k * 60_000;
    const st = stateWith({ price: 73n * E18 / 100n, at: now / 1000 - 60 });
    const r = await checkEthNft({ cfg: cfgOf(), prev, now, http, guardian: g, ic: { ethState: async () => st } });
    prev = r.next;
    for (const a of r.alerts) keys.add(a.key);
  }
  assert.equal(g.calls.length + g.registryPauses, 0, "no pause of anything: the module lags a real move by design");
  assert.ok([...keys].every((k) => !/:\d{10}$/.test(k)), "no key carries the poll's time");
});

test("D-M-1: a manipulated reading 20 % from BOTH agreeing references pauses THAT collection's poster once, then is latched until the operator acknowledges", async () => {
  const g = fakeGuardian();
  const { http } = world({ accepted: 93n * E18 / 100n, os: 0.93, cg: 0.93 });
  let prev = {};
  const at = (k) => 1_800_000_000_000 + k * 60_000;
  for (let k = 0; k < 5; k++) {
    const st = stateWith({ price: 60n * E18 / 100n, at: at(k) / 1000 - 60 });
    const r = await checkEthNft({ cfg: cfgOf(), prev, now: at(k), http, guardian: g, ic: { ethState: async () => st } });
    prev = r.next;
  }
  assert.deepEqual(g.calls, [POSTER], "one pause(), on this collection's PosterSource");
  assert.equal(g.registryPauses, 0, "never the registry");
  // the operator acknowledges (after the Safe resumed the poster): the guard is armed again
  const latchAt = prev.latch[MODULE].at;
  const st = stateWith({ price: 60n * E18 / 100n, at: at(6) / 1000 - 60 });
  await checkEthNft({ cfg: cfgOf(undefined, { floor_pause_ack: { [MODULE]: latchAt } }), prev, now: at(6), http, guardian: g, ic: { ethState: async () => st } });
  assert.equal(g.calls.length, 2);
});

test("D-M-1: one reference (CoinGecko 429) never pauses; both references down pages that the floor is unwatched", async () => {
  const g = fakeGuardian();
  const now = 1_800_000_000_000, st = stateWith({ price: 60n * E18 / 100n, at: now / 1000 - 60 });
  const one = world({ os: 0.93, cg: null });
  const r1 = await checkEthNft({ cfg: cfgOf(), now, http: one.http, guardian: g, ic: { ethState: async () => st } });
  assert.equal(g.calls.length, 0, "one keyless answer is not both references");
  assert.ok(r1.alerts.some((a) => a.key === `eth-floor-far-unconfirmed:${MODULE}`), "but it pages");
  const none = world({ os: null, cg: null });
  const r2 = await checkEthNft({ cfg: cfgOf(), now: now + 600_000, http: none.http, guardian: g, ic: { ethState: async () => st } });
  assert.ok(r2.alerts.some((a) => a.key === `eth-floor-unwatched:${MODULE}`));
});

// ---- D-M-2 ------------------------------------------------------------------------------------------------------------------------
const ETH_EV = {
  Undeliverable: parseAbiItem("event Undeliverable(uint256 indexed ticketId)"),
  Delivered: parseAbiItem("event Delivered(uint256 indexed ticketId, uint256 indexed tokenId, address destination, bytes32 txHash)"),
  Wrapped: parseAbiItem("event Wrapped(uint256 indexed itemId, uint256 indexed tokenId, bytes32 purchaseTx, address indexed to)"),
  Deposited: parseAbiItem("event Deposited(uint256 indexed itemId, uint256 indexed tokenId, bytes32 depositTx, address indexed to)"),
};
const ev = (name, args, n, data = "0x") => ({ address: WRAPPER, topics: encodeEventTopics({ abi: [ETH_EV[name]], eventName: name, args }), data, blockNumber: `0x${(2000 + n).toString(16)}`,
  transactionHash: `0x${String(n).padStart(64, "a")}`, logIndex: "0x0", removed: false });
const nftCfg = { wrappers: [WRAPPER], modules: [MODULE], nft_wrappers: [WRAPPER], nft_collection_of: { [WRAPPER]: MILADY } };

test("D-M-2: an honest eth markUndeliverable is backed by the canister's Ethereum delivery record: no false SEV1", () => {
  const eth = { buys: [], deposits: [], deliveries: [{ wrapper: blob(WRAPPER), collection: blob(MILADY), ticket: 7n, token: blob("00".repeat(31) + "01"), destination: blob("00".repeat(20)),
    deliver_by: 1n, phase: { Marked: { why: "a relist ticket is never delivered" } }, note: "" }] };
  const r = judgeLogs({ logs: [ev("Undeliverable", { ticketId: 7n }, 1)], index: recordIndex({ ...EMPTY_RECORDS, eth }), cfg: nftCfg });
  assert.deepEqual(r.alerts, []);
  // an Undeliverable no record backs is still SEV1
  const r2 = judgeLogs({ logs: [ev("Undeliverable", { ticketId: 8n }, 2)], index: recordIndex({ ...EMPTY_RECORDS, eth }), cfg: nftCfg });
  assert.equal(r2.alerts[0].sev, "SEV1");
});

test("D-M-2: EthNftWrapper's Wrapped / Deposited / Delivered are read by the canary: unbacked ones are SEV1, backed ones are quiet", () => {
  assert.ok(["NftWrapped", "NftDeposited", "NftDelivered"].every((k) => TOPICS[k]));
  const unbacked = [
    ev("Wrapped", { itemId: 1n, tokenId: 9180n, to: A(0xbad) }, 2, encodeAbiParameters([{ type: "bytes32" }], [`0x${"de".repeat(32)}`])),
    ev("Deposited", { itemId: 2n, tokenId: 9181n, to: A(0xbad) }, 3, encodeAbiParameters([{ type: "bytes32" }], [`0x${"ad".repeat(32)}`])),
    ev("Delivered", { ticketId: 0n, tokenId: 9180n }, 4, encodeAbiParameters([{ type: "address" }, { type: "bytes32" }], [A(0xbad), `0x${"00".repeat(31)}01`])),
  ];
  const none = recordIndex({ ...EMPTY_RECORDS, eth: { buys: [], deliveries: [], deposits: [] } });
  const r = judgeLogs({ logs: unbacked, index: none, cfg: nftCfg });
  assert.equal(r.alerts.filter((a) => a.sev === "SEV1").length, 3);
  // backed: the buy that minted 9180 to the module, the deposit of 9181 to its recipient, the delivery recorded with that hash
  const tok = (n) => blob(n.toString(16).padStart(64, "0"));
  const eth = {
    buys: [{ id: 1n, collection: blob(MILADY), module: blob(A(0xbad)), wrapper: blob(WRAPPER), phase: { Minted: { arrived: 1n, token: tok(9180n), total: 1n, tx: blob("de".repeat(32)) } } }],
    deposits: [{ tx: blob("ad".repeat(32)), collection: blob(MILADY), token: tok(9181n), recipient: blob(A(0xbad)), phase: { Minted: null } }],
    deliveries: [{ wrapper: blob(WRAPPER), ticket: 0n, phase: { Recorded: { tx: blob("00".repeat(31) + "01") } } }] };
  const r2 = judgeLogs({ logs: unbacked, index: recordIndex({ ...EMPTY_RECORDS, eth }), cfg: nftCfg });
  assert.deepEqual(r2.alerts, []);
});

// ---- D-M-3 ------------------------------------------------------------------------------------------------------------------------
test("D-M-3: the Ethereum outbound canary: a bought-not-minted NFT that left, an arrived buy's ETH that left, and a nonce not on the line each page SEV1", async () => {
  const state = { buys: [
    { id: 1n, collection: blob(MILADY), module: blob(MODULE), advanced: E18, attempts: 1, note: "", phase: { Final: { arrived: 97n * E18 / 100n, token: blob((9180).toString(16).padStart(64, "0")), total: 93n * E18 / 100n, tx: blob("ab".repeat(32)) } } },
    { id: 2n, collection: blob(MILADY), module: blob(MODULE), advanced: E18, attempts: 1, note: "", phase: { Arrived: { wei: 97n * E18 / 100n, since: 1_799_999_000n } } },
  ], deliveries: [], readings: [], eth_out: 0n, deposits: [], last_mined: [[4n, 1n]], line: [] };
  const { http } = world({ items: [], owner: () => A(0xbad), balance: 0n, nonce: 7 });
  const r = await checkEthNft({ cfg: cfgOf(), now: 1_800_000_000_000, http, ic: { ethState: async () => state } });
  const keys = r.alerts.filter((a) => a.sev === "SEV1").map((a) => a.key);
  assert.ok(keys.some((k) => k.startsWith("eth-not-held:")), "the bought token left custody");
  assert.ok(keys.some((k) => k.startsWith("eth-arrived-gone:")), "the arrived ETH left");
  assert.ok(keys.some((k) => k.startsWith("eth-outbound-unknown:")), "nonce 7 where the records account for 5");
});

// ---- D-L-3 ------------------------------------------------------------------------------------------------------------------------
test("D-L-3: an inbound NFT not judged is paged until the canister judges it, past the scan's window", async () => {
  const head = 26_000_000;
  const log = (n, tx) => ({ address: MILADY, topics: ["0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef", `0x${"00".repeat(12)}${"11".repeat(20)}`, `0x${"00".repeat(12)}${CUSTODIAN.slice(2)}`, `0x${"00".repeat(31)}05`],
    blockNumber: `0x${n.toString(16)}`, transactionHash: tx });
  const state = { buys: [], deliveries: [], readings: [], eth_out: 0n, deposits: [] };
  const young = world({ head, logs: [log(head - 700, `0x${"a1".repeat(32)}`)] });
  const r1 = await checkEthNft({ cfg: cfgOf(), now: 1_800_000_000_000, http: young.http, ic: { ethState: async () => state } });
  assert.equal(r1.alerts.filter((a) => a.key.startsWith("eth-unclaimed:")).length, 1);
  const later = world({ head: head + 7_300, logs: [] });   // a day on: the log is out of the scan's window
  const r2 = await checkEthNft({ cfg: cfgOf(), prev: r1.next, now: 1_800_086_400_000, http: later.http, ic: { ethState: async () => state } });
  assert.equal(r2.alerts.filter((a) => a.key.startsWith("eth-unclaimed:")).length, 1, "still paged: remembered until judged");
  const judged = { ...state, deposits: [{ tx: blob("a1".repeat(32)), collection: blob(MILADY), token: blob("00".repeat(32)), phase: { Seen: null } }] };
  const r3 = await checkEthNft({ cfg: cfgOf(), prev: r2.next, now: 1_800_090_000_000, http: later.http, ic: { ethState: async () => judged } });
  assert.equal(r3.alerts.filter((a) => a.key.startsWith("eth-unclaimed:")).length, 0);
});

// ---- D-L-4 ------------------------------------------------------------------------------------------------------------------------
test("D-L-4: a provider ERROR is unread, never an owner: no page per poll, one stable SEV2 after three polls, never a false SEV1", async () => {
  let prev = {};
  const keys = new Set();
  for (const head of [26_000_000, 26_000_005, 26_000_010]) {
    const { http } = world({ items: [9180n], head, ownerErr: { "https://b": "rate limited" } });
    const r = await checkEthNft({ cfg: cfgOf(), prev, now: 1_800_000_000_000, http });
    prev = r.next;
    for (const a of r.alerts.filter((x) => /owner/.test(x.key))) keys.add(a.key);
  }
  assert.deepEqual([...keys], ["eth-owner-unread:0x5af0d9827e0c53e4799bb226655a1de152a425a5:9180"]);
  const both = world({ items: [9180n], ownerErr: { "https://a": "header not found", "https://b": "header not found" } });
  const r = await checkEthNft({ cfg: cfgOf(), now: 1_800_000_000_000, http: both.http });
  assert.equal(r.alerts.find((a) => a.key.startsWith("eth-not-held:")), undefined, "two failing providers are not an owner");
});

// ---- D-L-5 ------------------------------------------------------------------------------------------------------------------------
test("D-L-5: one collection's failed read never skips the next collection's floor or its pause", async () => {
  const W2 = A(0xab), M2 = A(0xbc);
  const cols = [{ slug: "milady", collection: MILADY, wrapper: WRAPPER, module: MODULE, custodian: CUSTODIAN, opensea_slug: "milady", coingecko_id: "milady-maker" },
    { slug: "azuki", collection: "0xed5af388653567af2f388e6224dc7c4b3241c544", wrapper: W2, module: M2, custodian: CUSTODIAN, opensea_slug: "azuki", coingecko_id: "azuki" }];
  const g = fakeGuardian();
  const now = 1_800_000_000_000;
  const st = { ...stateWith(null), readings: [{ collection: blob("0xed5af388653567af2f388e6224dc7c4b3241c544"), post: { price: 40n * E18 / 100n }, at: BigInt(now / 1000 - 60) }] };
  const { http } = world({ os: 0.93, cg: 0.93, failItemCount: new Set([WRAPPER]) });
  const r = await checkEthNft({ cfg: cfgOf(cols), now, http, guardian: g, ic: { ethState: async () => st } });
  assert.ok(r.alerts.some((a) => a.key.startsWith(`eth-collection-unread:${MILADY}`)), "Milady's failure is paged");
  assert.deepEqual(g.calls, [A(0xf6)], "and Azuki's pause still ran (its own poster)");
});
