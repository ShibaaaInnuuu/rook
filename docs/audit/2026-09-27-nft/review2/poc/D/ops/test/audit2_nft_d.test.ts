// Round-2 re-review (docs/audit/2026-09-27-nft/review2/R2-services.md), area D. Each test ASSERTS THE WEAKNESS: a pass means it is
// there; a fix turns it red. No network.
import assert from "node:assert/strict";
import { test } from "node:test";
import { recentSaleTxs, SEAPORT_1_6, TRANSFER_TOPIC } from "../src/ethnft.js";
import { logger } from "../src/log.js";

const silent = logger(() => {});
const MIL = "0x5af0d9827e0c53e4799bb226655a1de152a425a5", AZUKI = "0xed5af388653567af2f388e6224dc7c4b3241c544";
const rpcOk = (result: unknown) => new Response(JSON.stringify({ result }));
const pad = (a: string) => `0x${a.replace(/^0x/, "").padStart(64, "0")}`;
const ORDER_FULFILLED = "0x9d9af8e38d66c62e2c12f0225249fd9d721c54b83f48d9352c97c6cacdcb6f31";
/** A receipt with a Seaport OrderFulfilled naming `coll`, and `coll`'s Transfer of token `id` from `from` to `to` (a SALE by the runner's
 *  isSale and, with a real price in the event, by the canister's sales_in), plus optional extra logs. */
function saleReceipt(coll: string, from: string, to: string, id: string, extra: object[] = []) {
  return { status: "0x1", logs: [{ address: coll, topics: [TRANSFER_TOPIC, pad(from), pad(to), pad(id)], data: "0x" },
    { address: SEAPORT_1_6, topics: [ORDER_FULFILLED, pad(from), pad("0x0")], data: `0x${"00".repeat(32)}${pad(to).slice(2)}${pad(coll).slice(2)}` }, ...extra] };
}
/** The canister's counted_sales dedupe (ethnft.rs:524-541): no self-trade, one per token, one per seller, one per buyer, newest first
 *  (the ±30 % band is left out here: it can only remove more). */
function countedLikeCanister(sales: { token: string; seller: string; buyer: string }[]): number {
  const t = new Set<string>(), s = new Set<string>(), b = new Set<string>();
  let n = 0;
  for (const x of sales) {
    if (x.seller === x.buyer || t.has(x.token) || s.has(x.seller) || b.has(x.buyer)) continue;
    t.add(x.token); s.add(x.seller); b.add(x.buyer); n++;
  }
  return n;
}

test("D2-M-2: 12 wash sales between two wallets, newer than the real ones, fill every slot the runner hands in: the canister counts at most 2 (< min_sales 3), no post, the module goes stale", async () => {
  const head = 0x1_000_000;
  const W1 = "0xa1", W2 = "0xa2";
  // three real sales (distinct sellers and buyers) an hour back; then 12 Seaport sales of ONE token back and forth between W1 and W2
  const real = [1, 2, 3].map((i) => ({ tx: `0x5a1e${i}`, block: head - 300, from: `0x1${i}`, to: `0x2${i}`, id: `0x${i}` }));
  const wash = Array.from({ length: 12 }, (_, i) => ({ tx: `0x3a5${String(i).padStart(2, "0")}`, block: head - 20 + i, from: i % 2 ? W2 : W1, to: i % 2 ? W1 : W2, id: "0x77" }));
  const all = [...real, ...wash];
  const logs = all.map((x, i) => ({ transactionHash: x.tx, blockNumber: `0x${x.block.toString(16)}`, logIndex: `0x${i.toString(16)}`, topics: [TRANSFER_TOPIC, pad(x.from), pad(x.to), pad(x.id)] }));
  const fetchImpl = (async (_u: string, init?: RequestInit) => {
    const b = JSON.parse(String(init?.body));
    if (b.method === "eth_blockNumber") return rpcOk(`0x${head.toString(16)}`);
    if (b.method === "eth_getTransactionReceipt") { const x = all.find((y) => y.tx === b.params[0])!; return rpcOk(saleReceipt(MIL, x.from, x.to, x.id)); }
    const from = Number(BigInt(b.params[0].fromBlock)), to = Number(BigInt(b.params[0].toBlock));
    return rpcOk(logs.filter((l) => { const n = Number(BigInt(l.blockNumber)); return n >= from && n <= to; }));
  }) as unknown as typeof fetch;
  const got = await recentSaleTxs({ ethRpc: "https://eth.example", fetch: fetchImpl, log: silent } as never, MIL, 12);
  assert.equal(got.length, 12);
  assert.ok(got.every((h) => h.startsWith("0x3a5")), "only the wash sales are handed in; the three real sales never reach the canister");
  const handed = got.map((h) => all.find((x) => x.tx === h)!).map((x) => ({ token: x.id, seller: x.from, buyer: x.to }));
  assert.ok(countedLikeCanister(handed) < 3, `the canister's own rule counts ${countedLikeCanister(handed)} of them: under min_sales = 3, no reading, no post`);
  assert.equal(countedLikeCanister(real.map((x) => ({ token: x.id, seller: x.from, buyer: x.to }))), 3, "the real sales alone would have counted 3");
});

test("D2-L-2: the sale verdict is remembered per transaction, not per collection: a tx that is a sale of Azuki but only a transfer of Milady is judged 'no sale' under Milady first, and never handed in for Azuki", async () => {
  const head = 0x1_000_000;
  const TX = "0xb0b1";
  // one transaction: an Azuki bought on Seaport (a sale of Azuki) and a Milady moved between the buyer's own wallets (no sale of Milady)
  const r = { status: "0x1", logs: [
    { address: AZUKI, topics: [TRANSFER_TOPIC, pad("0x11"), pad("0x22"), pad("0x9")], data: "0x" },
    { address: SEAPORT_1_6, topics: [ORDER_FULFILLED, pad("0x11"), pad("0x0")], data: `0x${"00".repeat(32)}${pad("0x22").slice(2)}${pad(AZUKI).slice(2)}` },
    { address: MIL, topics: [TRANSFER_TOPIC, pad("0x33"), pad("0x44"), pad("0x5")], data: "0x" }] };
  const logsFor = (addr: string) => [{ transactionHash: TX, blockNumber: `0x${(head - 5).toString(16)}`, logIndex: "0x0", topics: [TRANSFER_TOPIC, "a", "b", addr] }];
  const fetchImpl = (async (_u: string, init?: RequestInit) => {
    const b = JSON.parse(String(init?.body));
    if (b.method === "eth_blockNumber") return rpcOk(`0x${head.toString(16)}`);
    if (b.method === "eth_getTransactionReceipt") return rpcOk(r);
    const from = Number(BigInt(b.params[0].fromBlock));
    return rpcOk(from <= head - 5 && Number(BigInt(b.params[0].toBlock)) >= head - 5 ? logsFor(b.params[0].address) : []);
  }) as unknown as typeof fetch;
  const saleVerdicts = new Map<string, boolean>();
  const d = { ethRpc: "https://eth.example", fetch: fetchImpl, log: silent, saleVerdicts } as never;
  assert.deepEqual(await recentSaleTxs(d, MIL, 12), [], "right: not a Milady sale");
  assert.deepEqual(await recentSaleTxs(d, AZUKI, 12), [], "wrong: it IS an Azuki sale, but the Milady verdict is reused");
  assert.deepEqual(await recentSaleTxs({ ethRpc: "https://eth.example", fetch: fetchImpl, log: silent } as never, AZUKI, 12), [TX], "a fresh cache hands it in");
});
