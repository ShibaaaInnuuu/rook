// Audit D (docs/audit/2026-09-27-nft/D-services-deploy.md), reviewer D's PoCs against 57160b0. Each test ASSERTS THE WEAKNESS:
// a pass means the weakness is present; a fix turns it red. No network (the live measurement is scripts/audit_nft_d_window.ts).
import assert from "node:assert/strict";
import { test } from "node:test";
import { ethNftOnce, recentTransferTxs, TRANSFER_TOPIC } from "../src/ethnft.js";
import { logger } from "../src/log.js";
import { parseRunnerConfig, Runner } from "../src/runner.js";

const silent = logger(() => {});
const US = "0x76020b7093bd35d2520d47ded764efc554237efe";
const MIL = "0x5af0d9827e0c53e4799bb226655a1de152a425a5", AZUKI = "0xed5af388653567af2f388e6224dc7c4b3241c544", MAYC = "0x60e4d786628fea6478f785a6d7e704777c86a7c6";
const col = (c: string) => ({ collection: c, wrapper: "0xaa", module: "0xbb", max_purchase_wei: 10n ** 18n, max_premium_bps: 500 });
const NOW = 1_800_000_000;
const rpcOk = (result: unknown) => new Response(JSON.stringify({ result }));

// ---- D-H-1: the honest runner's "sales" are the latest 12 TRANSFER transactions of the last 7,200 blocks ---------------------------
test("D-H-1: recentTransferTxs hands in the newest 12 transfer txs whatever they are, from one un-chunked 7,200-block eth_getLogs, never the sales of the canister's 7-day window", async () => {
  const asked: any[] = []; // eslint-disable-line @typescript-eslint/no-explicit-any
  // 3 real sales two hours back, then 12 plain wallet-to-wallet transfers (anyone's, gas only): the runner hands in the 12 transfers
  const logs = [...[1, 2, 3].map((i) => ({ transactionHash: `0x5a1e${i}`, blockNumber: "0x10" })),
    ...Array.from({ length: 12 }, (_, i) => ({ transactionHash: `0x7a${String(i).padStart(2, "0")}`, blockNumber: "0x20" }))];
  const fetchImpl = (async (_u: string, init?: RequestInit) => {
    const b = JSON.parse(String(init?.body));
    if (b.method === "eth_blockNumber") return rpcOk("0x100000");
    asked.push(b.params[0]);
    return rpcOk(logs);
  }) as unknown as typeof fetch;
  const got = await recentTransferTxs({ ethRpc: "https://eth.example", fetch: fetchImpl, log: silent } as never, MIL, 12);
  assert.equal(got.length, 12);
  assert.ok(got.every((h) => h.startsWith("0x7a")), "every one handed in is a plain transfer: 0 sales, min_sales = 3 fails, no floor is posted");
  assert.ok(!got.some((h) => h.startsWith("0x5a1e")), "the three real sales two hours back are never handed in");
  assert.equal(asked.length, 1, "one eth_getLogs over the whole span (the deposit scan chunks; this one does not)");
  assert.deepEqual(asked[0].topics, [TRANSFER_TOPIC], "every Transfer of the collection, not the marketplaces' sale events");
  assert.equal(Number(BigInt(asked[0].toBlock)) - Number(BigInt(asked[0].fromBlock)), 7_200, "a day of blocks, where the canister accepts 7 days (SALE_WINDOW_BLOCKS 50,400)");
});

// ---- D-L-1: no timeout on any of the runner's outbound calls: one stalled provider stops every tick -------------------------------
test("D-L-1: an OpenSea (or Relay, or ETH_LOGS_RPC) answer that never comes stalls the whole cycle: the canister is never ticked again", async () => {
  let ticks = 0;
  const b = { tick: async () => { ticks++; return { Ok: [] as string[] }; }, paused: async () => false };
  const hang = (() => new Promise<Response>(() => {})) as unknown as typeof fetch;   // a tarpit: headers never sent
  const buyer = {
    eth_nft_config: async () => [{ chain_id: 1n, relay_max_fee_bps: 300, deposits: true, collections: [col(MIL), col(AZUKI), col(MAYC)] }] as never,
    eth_nft_state: async () => ({ buys: [], deliveries: [], readings: [], eth_out: 0n, deposits: [] }),
    evm_address: async () => ({ Ok: US }),
    eth_relay_quote: async () => ({ Err: "x" }), eth_listing: async () => ({ Err: "x" }), eth_floor_evidence: async () => ({ Ok: "held" }), eth_deposit: async () => ({ Ok: "held" }),
  };
  let t = 0;
  const runner = new Runner(b, silent, "p", "c", () => t, () => ethNftOnce({ buyer, fetch: hang, log: silent, slugs: { [MIL]: "milady", [AZUKI]: "azuki", [MAYC]: "mayc" },
    openseaKey: () => "k", ethRpc: "https://eth.example", nowS: () => NOW }));
  const cycle = runner.cycle();
  const raced = await Promise.race([cycle.then(() => "returned"), new Promise((ok) => setTimeout(() => ok("still waiting"), 300))]);
  assert.equal(raced, "still waiting", "the cycle never returns: main()'s loop never reaches its next tick");
  assert.equal(ticks, 1);
  t = 3 * 600_000 + 1;
  assert.equal(runner.healthy(600), false, "only /healthz notices, 30 min later (the monitor's liveness pages 10 min after that)");
});

test("D-L-1: an eth_deposit call that throws (a rejected update) aborts the pass: later collections' claims are not handed in", async () => {
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
  await assert.rejects(ethNftOnce({ buyer, fetch: fetchImpl, log: silent, slugs: {}, openseaKey: () => undefined, ethRpc: "https://eth.example", nowS: () => NOW, lastEvidence: new Map([[MIL, NOW], [AZUKI, NOW]]) }));
  assert.deepEqual(claims, [], "Azuki's inbound NFT is not handed in because Milady's call threw");
});

// ---- D-L-2: the Ethereum step starts half-configured and says so only in log lines ---------------------------------------------------
test("D-L-2: RUNNER_ETH_NFT defaults on and starts without ETH_LOGS_RPC, OPENSEA_API_KEY or ETH_NFT_SLUGS; then no sale is ever handed in and no deposit is discovered", async () => {
  const cfg = parseRunnerConfig({ CANISTER_ID: "rrkah-fqaaa-aaaaa-aaaaq-cai", OPERATOR_IDENTITY_PEM: "x" });
  assert.deepEqual([cfg.eth, cfg.ethLogsRpc, Object.keys(cfg.slugs).length], [true, null, 0], "accepted: nothing required for the eth step");
  const handed: [string, number][] = [];
  let deposits = 0;
  const buyer = {
    eth_nft_config: async () => [{ chain_id: 1n, relay_max_fee_bps: 300, deposits: true, collections: [col(MIL)] }] as never,
    eth_nft_state: async () => ({ buys: [], deliveries: [], readings: [], eth_out: 0n, deposits: [] }),
    evm_address: async () => ({ Ok: US }),
    eth_relay_quote: async () => ({ Err: "x" }), eth_listing: async () => ({ Err: "x" }),
    eth_floor_evidence: async (c: string, l: string[], s: string[]) => { handed.push([c, s.length]); return { Ok: "held" }; },
    eth_deposit: async () => { deposits++; return { Ok: "held" }; },
  };
  const os = (async () => new Response(JSON.stringify({ listings: [] }))) as unknown as typeof fetch;
  await ethNftOnce({ buyer, fetch: os, log: silent, slugs: { [MIL]: "milady" }, openseaKey: () => "k", ethRpc: null, nowS: () => NOW });
  assert.deepEqual(handed, [[MIL, 0]], "evidence with zero sales: the canister can never post (min_sales 3)");
  assert.equal(deposits, 0, "deposits are ON and not one inbound NFT is discovered");
  const noSlug: [string, number][] = [];
  await ethNftOnce({ buyer: { ...buyer, eth_floor_evidence: async (c: string) => { noSlug.push([c, 0]); return { Ok: "held" }; } }, fetch: os, log: silent,
    slugs: {}, openseaKey: () => "k", ethRpc: null, nowS: () => NOW });
  assert.deepEqual(noSlug, [], "a collection the canister serves but ETH_NFT_SLUGS does not name: no evidence at all, one log line");
});

// ---- D-I-3: the E-05 per-unit rule counts the unit in transit, whose escrow already left ------------------------------------------
import { backingStatus } from "../src/backing.js";
test("D-I-3: during an advance the reconciler's 50 %-of-floor rule divides the escrow by E including the transit unit (no escrow behind it): RED at launch numbers once the floor is up 20 % since graduation", () => {
  const E = 10n ** 18n, g = 93n * E / 100n;                              // graduated at 0.93; R 1.1 units, 0.3 convertible left, 1 in transit
  const books = { backing: 3n * g / 10n, reserve: 11n * g / 10n, surplus: 0n, revenue: 0n, gradEth: 0n, rentalEscrow: 0n, renterPool: 0n, owedTotal: 0n };
  const owed = books.backing + books.reserve;
  const m = { slug: "milady", module: "0xbb", wrapper: "0xaa", block: 1, balance: owed, books, owed, reserveUnits: 11n * E / 10n, ethBacked: 24n * E / 10n,
    accepted: g * 12n / 10n, fresh: true, escaped: false, postedAt: NOW, lastGoodAt: NOW, floorTtl: 3_600, transitUnits: E, advanced: g, advancedAt: NOW };
  const r = backingStatus({ kind: "read", at: NOW, modules: [m] }, NOW);
  assert.equal(r.status, "red");
  assert.match(r.reasons.join(" "), /under 50% of the floor/);
  const without = backingStatus({ kind: "read", at: NOW, modules: [{ ...m, ethBacked: m.ethBacked - E }] }, NOW);
  assert.equal(without.status, "green", "the same module, the transit unit left out of the per-unit rule (its ETH is at the canister): green");
});

// ---- D-M-4: the runner hands in only the cheapest listing, every pass, whatever the canister judged -------------------------------
import { SEAPORT_1_6, decodeAdvanced } from "../src/ethnft.js";
test("D-M-4: eth_listing always answers Ok (held for the tick), so the runner stops at the first listing: a cheapest listing the canister refuses (an optional creator fee, a zone, a stale order) is handed in again every pass and the others never", async () => {
  const PD = (id: number) => ({ parameters: { offerer: "0x1f3bd426d37864c0c97ff40aeadf5b11e7189f9f", zone: "0x0000000000000000000000000000000000000000",
    offer: [{ itemType: 2, token: MIL, identifierOrCriteria: String(id), startAmount: "1", endAmount: "1" }],
    consideration: [{ itemType: 0, token: "0x0000000000000000000000000000000000000000", identifierOrCriteria: "0", startAmount: "1", endAmount: "1", recipient: "0x1f3bd426d37864c0c97ff40aeadf5b11e7189f9f" }],
    orderType: 0, startTime: "1", endTime: "2000000000", zoneHash: `0x${"0".repeat(64)}`, salt: "1", conduitKey: `0x${"0".repeat(64)}`, totalOriginalConsiderationItems: 1 }, signature: `0x${"ab".repeat(65)}` });
  const fetchImpl = (async (u: string, init?: RequestInit) => {
    if (u.includes("/best")) return new Response(JSON.stringify({ listings: [1, 2, 3].map((i) => ({ order_hash: `0x0${i}`, protocol_address: SEAPORT_1_6, protocol_data: PD(i) })) }));
    if (u.endsWith("/fulfillment_data")) { const h = JSON.parse(String(init?.body)).listing.hash; const i = Number(h.slice(3));
      return new Response(JSON.stringify({ fulfillment_data: { transaction: { function: "fulfillAdvancedOrder", to: SEAPORT_1_6, input_data: {
        advancedOrder: { parameters: PD(i).parameters, numerator: 1, denominator: 1, signature: PD(i).signature, extraData: "0x" }, recipient: US } } } })); }
    return new Response(JSON.stringify({ result: [] }));
  }) as unknown as typeof fetch;
  const handed: string[] = [];
  const buyer = {
    eth_nft_config: async () => [{ chain_id: 1n, relay_max_fee_bps: 300, deposits: false, collections: [col(MIL)] }] as never,
    eth_nft_state: async () => ({ buys: [{ id: 4n, collection: Buffer.from(MIL.slice(2), "hex"), module: new Uint8Array(20), wrapper: new Uint8Array(20), advanced: 1n, attempts: 1,
      phase: { Arrived: { wei: 1n, since: 1n } }, ask: [] as [], note: "eth buy 4: listing refused: a fee to 0x…, not a chain fee recipient" }], deliveries: [], readings: [], eth_out: 0n, deposits: [] }),
    evm_address: async () => ({ Ok: US }),
    eth_relay_quote: async () => ({ Err: "x" }), eth_floor_evidence: async () => ({ Err: "x" }), eth_deposit: async () => ({ Err: "x" }),
    // the canister's put_listing: Ok("listing held for buy 4; judged at the next tick") for any well-formed order
    eth_listing: async (_b: bigint, o: string) => { handed.push(String(decodeAdvanced(o as `0x${string}`).advancedOrder.parameters.offer[0].identifierOrCriteria)); return { Ok: "listing held for buy 4; judged at the next tick" }; },
  };
  for (let pass = 0; pass < 3; pass++)
    await ethNftOnce({ buyer, fetch: fetchImpl, log: silent, slugs: { [MIL]: "milady" }, openseaKey: () => "k", ethRpc: null, nowS: () => NOW, lastEvidence: new Map([[MIL, NOW]]) });
  assert.equal(handed.length, 3, "one listing a pass");
  assert.deepEqual(handed, ["1", "1", "1"], "the same (cheapest) listing every pass, though the buy's own note says it was refused: listings 2 and 3 are never tried");
});
