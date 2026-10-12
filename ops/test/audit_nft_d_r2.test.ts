// Fix round 2 of the 2026-09-27 NFT review (docs/audit/2026-09-27-nft/fixes-round2.md): the round-2 ops PoCs
// (review2/poc/D/ops/test/audit2_nft_d.test.ts) turned around, and the runner's half of the way back (A2-M-1). No network.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import * as ethnft from "../src/ethnft.js";
import { ethAskOf, ethNftOnce, preJudgeEth, type EthAsk } from "../src/ethnft.js";
import { logger } from "../src/log.js";
import { orderIdOf } from "../src/relay.js";

const silent = logger(() => {});
const US = "0x76020b7093bd35d2520d47ded764efc554237efe";
const MIL = "0x5af0d9827e0c53e4799bb226655a1de152a425a5", AZUKI = "0xed5af388653567af2f388e6224dc7c4b3241c544";
const col = (c: string) => ({ collection: c, wrapper: "0xaa", module: "0xbb", max_purchase_wei: 10n ** 18n, max_premium_bps: 500 });
const NOW = 1_800_000_000;
const rpcOk = (result: unknown) => new Response(JSON.stringify({ result }));
const TRANSFER = "0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef";
const pad = (a: string) => `0x${a.replace(/^0x/, "").padStart(64, "0")}`;

test("D2-M-2 (turned around): with 12 wash sales newer than the real ones on the chain, the runner hands in no sale at all (the canister reads its own sample over the whole window); D2-L-2: no per-transaction sale verdict is kept", async () => {
  const head = 0x1_000_000;
  const W1 = "0xa1", W2 = "0xa2";
  const real = [1, 2, 3].map((i) => ({ tx: `0x5a1e${i}`, block: head - 300, from: `0x1${i}`, to: `0x2${i}`, id: `0x${i}` }));
  const wash = Array.from({ length: 12 }, (_, i) => ({ tx: `0x3a5${String(i).padStart(2, "0")}`, block: head - 20 + i, from: i % 2 ? W2 : W1, to: i % 2 ? W1 : W2, id: "0x77" }));
  const logs = [...real, ...wash].map((x, i) => ({ transactionHash: x.tx, blockNumber: `0x${x.block.toString(16)}`, logIndex: `0x${i.toString(16)}`, topics: [TRANSFER, pad(x.from), pad(x.to), pad(x.id)] }));
  let chainReads = 0;
  const fetchImpl = (async (url: string, init?: RequestInit) => {
    if (String(url).includes("/listings/collection/milady/best")) return new Response(JSON.stringify({ listings: [] }));
    const b = JSON.parse(String(init?.body ?? "{}"));
    chainReads++;
    if (b.method === "eth_blockNumber") return rpcOk(`0x${head.toString(16)}`);
    if (b.method === "eth_getLogs") return rpcOk(logs);
    return rpcOk(null);
  }) as unknown as typeof fetch;
  const handed: unknown[][] = [];
  const buyer = {
    eth_nft_config: async () => [{ chain_id: 1n, relay_max_fee_bps: 300, deposits: false, collections: [col(MIL)] }] as never,
    eth_nft_state: async () => ({ buys: [], deliveries: [], readings: [], eth_out: 0n, deposits: [] }),
    evm_address: async () => ({ Ok: US }),
    eth_relay_quote: async () => ({ Err: "x" }), eth_listing: async () => ({ Err: "x" }), eth_deposit: async () => ({ Err: "x" }),
    eth_floor_evidence: async (c: string, l: string[], s: string[]) => { handed.push([c, l.length, s]); return { Ok: "held" }; },
  };
  await ethNftOnce({ buyer, fetch: fetchImpl, log: silent, slugs: { [MIL]: "milady" }, openseaKey: () => "k", ethRpc: "https://eth.example", nowS: () => NOW });
  assert.deepEqual(handed, [[MIL, 0, []]], "no sale hash: whatever the chain's newest sales are, nothing the runner reads decides which count");
  assert.equal(chainReads, 0, "the runner does not read the collection's sales at all");
  assert.equal((ethnft as Record<string, unknown>).recentSaleTxs, undefined, "and keeps no sale verdict, per transaction or otherwise (D2-L-2)");
});

test("D2-L-3 (turned around): what the Ethereum step lacks is checked on every pass, not only at start", async () => {
  let cols = [col(MIL)];
  const buyer = {
    eth_nft_config: async () => [{ chain_id: 1n, relay_max_fee_bps: 300, deposits: false, collections: cols }] as never,
    eth_nft_state: async () => ({ buys: [], deliveries: [], readings: [], eth_out: 0n, deposits: [] }),
    evm_address: async () => ({ Ok: US }),
    eth_relay_quote: async () => ({ Err: "x" }), eth_listing: async () => ({ Err: "x" }), eth_deposit: async () => ({ Err: "x" }),
    eth_floor_evidence: async () => ({ Ok: "held" }),
  };
  const fetchImpl = (async () => new Response(JSON.stringify({ listings: [] }))) as unknown as typeof fetch;
  const problems: string[] = ["the canister's eth_nft_config() could not be read at start: checked again on the next pass"], served: string[] = [];
  const d = { buyer, fetch: fetchImpl, log: silent, slugs: { [MIL]: "milady" }, openseaKey: () => "k", ethRpc: "https://eth.example", nowS: () => NOW, problems, served };
  await ethNftOnce(d);
  assert.deepEqual(problems, [], "read on this pass: nothing lacking");
  assert.deepEqual(served, [MIL]);
  cols = [col(MIL), col(AZUKI)];      // an upgrade adds Azuki; the runner has no slug for it
  await ethNftOnce({ ...d, nowS: () => NOW + 3_600 });
  assert.equal(problems.length, 1);
  assert.match(problems[0], new RegExp(AZUKI));
  assert.deepEqual(served, [MIL, AZUKI]);
});

// ---- A2-M-1: the way back (Ethereum → 4663) --------------------------------------------------------------------------------------
const BACK = readFileSync(new URL("../../autobuyer/canister-rs/tests/fixtures/relay-quote-1-4663-2026-09-27.json", import.meta.url), "utf8");
function backQuote(amount: bigint, now: number): any { // eslint-disable-line @typescript-eslint/no-explicit-any
  const q = JSON.parse(BACK.replaceAll("2427857b22f64a7b9cb22148928f1e410c7aa108", US.slice(2)));
  q.steps[0].items[0].data.value = amount.toString();
  const o = q.protocol.v2.orderData;
  o.inputs[0].payment.amount = amount.toString();
  for (const r of o.inputs[0].refunds) r.deadline = now + 7 * 86_400;
  o.output.deadline = now + 7 * 86_400;
  o.output.payments[0].minimumAmount = (amount / 100n * 98n).toString();
  q.protocol.v2.paymentDetails.amount = amount.toString();
  const id = orderIdOf(o);
  q.steps[0].items[0].data.data = `0x49290c1c${"0".repeat(24)}${US.slice(2)}${id.slice(2)}`;
  q.protocol.v2.orderId = id;
  return q;
}

test("A2-M-1: a buy going back is asked a Relay quote Ethereum → 4663, pre-judged for that leg", () => {
  const amount = 495_443_962_449_863_321n;
  const ask = { amount_wei: amount, user: US, recipient: US, refund_to: US, origin_chain_id: 1n, destination_chain_id: 4663n,
    origin_currency: "0x0000000000000000000000000000000000000000", destination_currency: "0x0000000000000000000000000000000000000000" };
  const b = { id: 7n, collection: new Uint8Array(20), module: new Uint8Array(20), wrapper: new Uint8Array(20), advanced: 1n, attempts: 1, note: "",
    phase: { BackFunded: { wei: amount + 3n * 10n ** 15n, since: 1n } }, ask: [ask] } as never;
  const a = ethAskOf(b) as EthAsk;
  assert.deepEqual([a.originChainId, a.destinationChainId, a.amountWei], [1, 4663, amount]);
  const j = preJudgeEth(backQuote(amount, NOW), a, NOW);
  assert.equal(j.minOut, amount / 100n * 98n);
  // the forward leg's ask never takes a way-back quote, and a way-back quote paying someone else is refused
  assert.throws(() => preJudgeEth(backQuote(amount, NOW), { ...a, originChainId: 4663, destinationChainId: 1 }, NOW));
  const other = backQuote(amount, NOW);
  other.protocol.v2.orderData.output.payments[0].recipient = "0x1111111111111111111111111111111111111111";
  assert.throws(() => preJudgeEth(other, a, NOW));
});

test("round 2: eth_nft_state() with the way back's phases and line decodes with the runner's IDL", async () => {
  const { IDL } = await import("@icp-sdk/core/candid");
  const { buyerIdl } = await import("../src/ic.js");
  const svc = buyerIdl({ IDL }) as unknown as { _fields: [string, { retTypes: Parameters<typeof IDL.decode>[0] }][] };
  const f = svc._fields.find(([n]) => n === "eth_nft_state")![1];
  const hex = readFileSync(new URL("../../autobuyer/canister-rs/tests/fixtures/eth-state-round2.hex", import.meta.url), "utf8").trim();
  const [st] = IDL.decode(f.retTypes, Uint8Array.from(Buffer.from(hex, "hex"))) as [{ buys: { phase: Record<string, unknown> }[] }];
  // A2-L-1 (2026-09-27): the fourth is a gas-pot sweep on its way to fundReserve (the runner quotes its way back like any other)
  assert.deepEqual(st.buys.map((b) => Object.keys(b.phase)[0]), ["BackFunded", "BackSending", "BackBridging", "Returning"]);
});
