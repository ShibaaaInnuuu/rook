// Audit 2026-09-27-nft, fix round 1 (docs/audit/2026-09-27-nft/fixes-round1.md): reviewer D's ops PoCs
// (docs/audit/2026-09-27-nft/poc/D/ops/test/audit_nft_d.test.ts) as regressions. Each drives the PoC's world and asserts the weakness
// is gone. No network.
import assert from "node:assert/strict";
import { test } from "node:test";
import { backingStatus } from "../src/backing.js";
import { decodeAdvanced, ethNftOnce, ethStepProblems, SEAPORT_1_6, TRANSFER_TOPIC } from "../src/ethnft.js";
import { logger } from "../src/log.js";
import { Runner, staleEvidence } from "../src/runner.js";

const silent = logger(() => {});
const US = "0x76020b7093bd35d2520d47ded764efc554237efe";
const MIL = "0x5af0d9827e0c53e4799bb226655a1de152a425a5", AZUKI = "0xed5af388653567af2f388e6224dc7c4b3241c544", MAYC = "0x60e4d786628fea6478f785a6d7e704777c86a7c6";
const col = (c: string) => ({ collection: c, wrapper: "0xaa", module: "0xbb", max_purchase_wei: 10n ** 18n, max_premium_bps: 500 });
const NOW = 1_800_000_000;
const rpcOk = (result: unknown) => new Response(JSON.stringify({ result }));
const pad = (a: string) => `0x${a.replace(/^0x/, "").padStart(64, "0")}`;
function receipt(sale: boolean, from = "0x11", to = "0x22") {
  const logs = [{ address: MIL, topics: [TRANSFER_TOPIC, pad(from), pad(to), pad("0x5")], data: "0x" }];
  if (sale) logs.push({ address: SEAPORT_1_6, topics: ["0x9d9af8e38d66c62e2c12f0225249fd9d721c54b83f48d9352c97c6cacdcb6f31", pad(from), pad("0x0")],
    data: `0x${"00".repeat(32)}${pad(to).slice(2)}${pad(MIL).slice(2)}` });
  return { status: "0x1", logs };
}

// ---- D-H-1 --------------------------------------------------------------------------------------------------------------------------
test("D-H-1 (fix round 2): the runner hands in no sale at all: the canister reads the collection's sales itself, so no transfer, self-trade or wash sale the runner reads can crowd a real one out", async () => {
  const handed: unknown[][] = [];
  let logsRead = 0;
  const fetchImpl = (async (url: string, init?: RequestInit) => {
    const u = String(url);
    if (u.includes("/listings/collection/milady/best")) return new Response(JSON.stringify({ listings: [] }));
    const b = JSON.parse(String(init?.body ?? "{}"));
    if (b.method === "eth_getLogs" || b.method === "eth_getTransactionReceipt") logsRead++;
    if (b.method === "eth_blockNumber") return rpcOk("0x100000");
    return rpcOk([]);
  }) as unknown as typeof fetch;
  const buyer = {
    eth_nft_config: async () => [{ chain_id: 1n, relay_max_fee_bps: 300, deposits: false, collections: [col(MIL)] }] as never,
    eth_nft_state: async () => ({ buys: [], deliveries: [], readings: [], eth_out: 0n, deposits: [] }),
    evm_address: async () => ({ Ok: US }),
    eth_relay_quote: async () => ({ Err: "x" }), eth_listing: async () => ({ Err: "x" }),
    eth_floor_evidence: async (c: string, l: string[], s: string[]) => { handed.push([c, l.length, s]); return { Ok: "held" }; },
    eth_deposit: async () => ({ Err: "x" }),
  };
  await ethNftOnce({ buyer, fetch: fetchImpl, log: silent, slugs: { [MIL]: "milady" }, openseaKey: () => "k", ethRpc: "https://eth.example", nowS: () => NOW });
  assert.deepEqual(handed, [[MIL, 0, []]], "listings only");
  assert.equal(logsRead, 0, "the runner reads no Transfer log and no receipt for sales");
});

// ---- D-L-1 --------------------------------------------------------------------------------------------------------------------------
test("D-L-1: an answer that never comes times out: the cycle returns and the canister is ticked again", async () => {
  let ticks = 0;
  const b = { tick: async () => { ticks++; return { Ok: [] as string[] }; }, paused: async () => false };
  const hang = ((_u: string, init?: RequestInit) => new Promise<Response>((_ok, no) => init?.signal?.addEventListener("abort", () => no(new Error("aborted: the deadline"))))) as unknown as typeof fetch;
  const buyer = {
    eth_nft_config: async () => [{ chain_id: 1n, relay_max_fee_bps: 300, deposits: true, collections: [col(MIL), col(AZUKI), col(MAYC)] }] as never,
    eth_nft_state: async () => ({ buys: [], deliveries: [], readings: [], eth_out: 0n, deposits: [] }),
    evm_address: async () => ({ Ok: US }),
    eth_relay_quote: async () => ({ Err: "x" }), eth_listing: async () => ({ Err: "x" }), eth_floor_evidence: async () => ({ Ok: "held" }), eth_deposit: async () => ({ Ok: "held" }),
  };
  const runner = new Runner(b, silent, "p", "c", () => Date.now(), () => ethNftOnce({ buyer, fetch: hang, log: silent, slugs: { [MIL]: "milady", [AZUKI]: "azuki", [MAYC]: "mayc" },
    openseaKey: () => "k", ethRpc: "https://eth.example", nowS: () => NOW, timeoutMs: 50 }));
  const raced = await Promise.race([runner.cycle().then(() => "returned"), new Promise((ok) => setTimeout(() => ok("still waiting"), 3_000))]);
  assert.equal(raced, "returned");
  assert.equal(ticks, 1);
});

test("D-L-1: an eth_deposit call that throws never aborts the pass: the other collections' claims are handed in", async () => {
  const logsFor = (addr: string) => [{ transactionHash: addr === MIL ? "0xa1" : "0xa2", topics: ["t", "a", "b", "c"] }];
  const fetchImpl = (async (_u: string, init?: RequestInit) => {
    const b = JSON.parse(String(init?.body));
    if (b.method === "eth_blockNumber") return rpcOk("0x100000");
    return rpcOk(Number(BigInt(b.params[0].fromBlock)) === 0x100000 - 7_200 ? logsFor(b.params[0].address) : []);
  }) as unknown as typeof fetch;
  const claims: string[] = [];
  const buyer = {
    eth_nft_config: async () => [{ chain_id: 1n, relay_max_fee_bps: 300, deposits: true, collections: [col(MIL), col(AZUKI)] }] as never,
    eth_nft_state: async () => ({ buys: [], deliveries: [], readings: [], eth_out: 0n, deposits: [] }),
    evm_address: async () => ({ Ok: US }),
    eth_relay_quote: async () => ({ Err: "x" }), eth_listing: async () => ({ Err: "x" }), eth_floor_evidence: async () => ({ Err: "x" }),
    eth_deposit: async (tx: string) => { if (tx === "0xa1") throw new Error("Call was rejected: the canister is busy (a tick is running)"); claims.push(tx); return { Ok: "held" }; },
  };
  await ethNftOnce({ buyer, fetch: fetchImpl, log: silent, slugs: {}, openseaKey: () => undefined, ethRpc: "https://eth.example", nowS: () => NOW, lastEvidence: new Map([[MIL, NOW], [AZUKI, NOW]]) });
  assert.deepEqual(claims, ["0xa2"]);
});

// ---- D-L-2 --------------------------------------------------------------------------------------------------------------------------
test("D-L-2: the Ethereum step is refused half-configured, and a collection without accepted evidence makes /healthz unhealthy", () => {
  const cfg = { chain_id: 1n, relay_max_fee_bps: 300, deposits: true, collections: [col(MIL), col(AZUKI)] } as never;
  const p = ethStepProblems(cfg, { ethLogsRpc: null, slugs: { [MIL]: "milady" }, openseaKey: undefined });
  assert.equal(p.length, 3);
  assert.match(p.join("\n"), /ETH_LOGS_RPC/);
  assert.match(p.join("\n"), /OPENSEA_API_KEY/);
  assert.match(p.join("\n"), new RegExp(AZUKI));
  assert.deepEqual(ethStepProblems(cfg, { ethLogsRpc: "https://e", slugs: { [MIL]: "milady", [AZUKI]: "azuki" }, openseaKey: "k" }), []);
  const ok = new Map([[MIL, NOW - 60]]);
  assert.deepEqual(staleEvidence([MIL], ok, NOW, NOW - 7_200), []);
  assert.equal(staleEvidence([MIL, AZUKI], ok, NOW, NOW - 7_200).length, 1, "Azuki: no evidence accepted in two hours");
  const r = new Runner({ tick: async () => ({ Ok: [] }), paused: async () => false }, silent, "p", "c", () => Date.now());
  r.ethProblems = () => ["no floor evidence accepted for x"];
  assert.equal(r.healthy(600), false);
});

// ---- D-I-3 --------------------------------------------------------------------------------------------------------------------------
test("D-I-3: during an advance the 50 %-of-floor rule leaves the transit unit out (its escrow left with the advance): green at launch numbers", () => {
  const E = 10n ** 18n, g = 93n * E / 100n;
  const books = { backing: 3n * g / 10n, reserve: 11n * g / 10n, surplus: 0n, revenue: 0n, gradEth: 0n, rentalEscrow: 0n, renterPool: 0n, owedTotal: 0n };
  const owed = books.backing + books.reserve;
  const m = { slug: "milady", module: "0xbb", wrapper: "0xaa", block: 1, balance: owed, books, owed, reserveUnits: 11n * E / 10n, ethBacked: 24n * E / 10n,
    accepted: g * 12n / 10n, fresh: true, escaped: false, postedAt: NOW, lastGoodAt: NOW, floorTtl: 3_600, transitUnits: E, advanced: g, advancedAt: NOW };
  assert.equal(backingStatus({ kind: "read", at: NOW, modules: [m] }, NOW).status, "green");
});

// ---- D-M-4 --------------------------------------------------------------------------------------------------------------------------
test("D-M-4: a listing the canister refused is not handed in again; the next pass tries the next one", async () => {
  const PD = (id: number) => ({ parameters: { offerer: "0x1f3bd426d37864c0c97ff40aeadf5b11e7189f9f", zone: "0x0000000000000000000000000000000000000000",
    offer: [{ itemType: 2, token: MIL, identifierOrCriteria: String(id), startAmount: "1", endAmount: "1" }],
    consideration: [{ itemType: 0, token: "0x0000000000000000000000000000000000000000", identifierOrCriteria: "0", startAmount: "1", endAmount: "1", recipient: "0x1f3bd426d37864c0c97ff40aeadf5b11e7189f9f" }],
    orderType: 0, startTime: "1", endTime: "2000000000", zoneHash: `0x${"0".repeat(64)}`, salt: "1", conduitKey: `0x${"0".repeat(64)}`, totalOriginalConsiderationItems: 1 }, signature: `0x${"ab".repeat(65)}` });
  const fetchImpl = (async (u: string, init?: RequestInit) => {
    if (u.includes("/best")) return new Response(JSON.stringify({ listings: [1, 2, 3].map((i) => ({ order_hash: `0x0${i}`, protocol_address: SEAPORT_1_6, protocol_data: PD(i), price: { current: { value: "900000000000000000" } } })) }));
    if (u.endsWith("/fulfillment_data")) { const h = JSON.parse(String(init?.body)).listing.hash; const i = Number(h.slice(3));
      return new Response(JSON.stringify({ fulfillment_data: { transaction: { function: "fulfillAdvancedOrder", to: SEAPORT_1_6, input_data: {
        advancedOrder: { parameters: PD(i).parameters, numerator: 1, denominator: 1, signature: PD(i).signature, extraData: "0x" }, recipient: US } } } })); }
    return new Response(JSON.stringify({ result: [] }));
  }) as unknown as typeof fetch;
  const handed: string[] = [];
  const buyer = {
    eth_nft_config: async () => [{ chain_id: 1n, relay_max_fee_bps: 300, deposits: false, collections: [col(MIL)] }] as never,
    eth_nft_state: async () => ({ buys: [{ id: 4n, collection: Buffer.from(MIL.slice(2), "hex"), module: new Uint8Array(20), wrapper: new Uint8Array(20), advanced: 1n, attempts: 1,
      phase: { Arrived: { wei: 10n ** 18n, since: 1n } }, ask: [] as [], note: "eth buy 4: listing refused" }], deliveries: [], readings: [], eth_out: 0n, deposits: [] }),
    evm_address: async () => ({ Ok: US }),
    eth_relay_quote: async () => ({ Err: "x" }), eth_floor_evidence: async () => ({ Err: "x" }), eth_deposit: async () => ({ Err: "x" }),
    eth_listing: async (_b: bigint, o: string) => { handed.push(String(decodeAdvanced(o as `0x${string}`).advancedOrder.parameters.offer[0].identifierOrCriteria)); return { Ok: "listing held for buy 4; judged at the next tick" }; },
  };
  const tried = new Map<string, Map<string, number>>();
  for (let pass = 0; pass < 3; pass++)
    await ethNftOnce({ buyer, fetch: fetchImpl, log: silent, slugs: { [MIL]: "milady" }, openseaKey: () => "k", ethRpc: null, nowS: () => NOW, lastEvidence: new Map([[MIL, NOW]]), tried });
  assert.deepEqual(handed, ["1", "2", "3"], "each listing once, cheapest first");
});
