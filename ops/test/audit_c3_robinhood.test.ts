import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { orderIdAny, s3Reads, type S3 } from "../src/fuel/sol.js";
import { ethNftOnce } from "../src/ethnft.js";
import { logger } from "../src/log.js";
import { preJudgeSolAsk, RELAY_SOLANA, SOL_NATIVE, solLegOf, type AnyAsk } from "../src/relaysol.js";
import type { EthBuyV } from "../src/ic.js";

// Audit round C, part C3 (docs/v9/audit-2026-10/c3-robinhood-chain.md), C3-03 (High): the runner's pre-judge of a Solana Relay leg
// (src/relaysol.ts `solLegOf` / `preJudgeSolAsk`) knows three legs, SOL → BTC, SOL → ETHEREUM and ETHEREUM → SOL. A Robinhood Chain
// buy's legs (the canister's `relayeth::relay_name(4663)` = "robinhood": SOL → 4663 for its advance, `ROBINHOOD_TO_SOLANA` for its way
// back) are refused before the quote is read, so the runner never hands a 4663 quote in. The buy stays `Funded` (or `BackFunded`)
// holding the one Relay leg canister-wide (the canister's half: tests/audit_c3_robinhood.rs `poc_c3_03_…`).
// Each test PASSES by exhibiting the bug. A fix must hand the 4663 legs in: `solLegOf` names (RELAY_SOLANA → 4663) and
// (4663 → RELAY_SOLANA), judged with destination / origin "robinhood" (the canister's S1–S12 and judge_to_solana for that leg);
// these tests then fail.
//
// FIXED (2026-10-12): `solLegOf` names `sol-rh` (RELAY_SOLANA → 4663, judged by `judgeSolOrigin` with destination "robinhood") and
// `rh-sol` (4663 → RELAY_SOLANA, judged by `judgeEthToSol` with origin 4663: the deposit on 4663, the payment and the origin refund on
// "robinhood"), from the same table as the canister's `relay_name` / `to_solana`; one runner pass hands a 4663 quote in for its side's
// buy id. The tests below are the inverted PoCs (`fixed_c3_03_…`); the canister also times a quote-less `Funded` buy out (tests/
// audit_c3_robinhood.rs, tests/ethnft_sol.rs `fixed_c3_03_…`).

const FX = new URL("fixtures/relay-sol/", import.meta.url);
const raw = (f: string) => readFileSync(new URL(f, FX), "utf8");
const AT = JSON.parse(raw("manifest.json")).fetched_at as Record<string, number>;
const SOL_ETH = raw("sol-eth-1.quote.json"), ETH_SOL = raw("eth-sol-0.5.quote.json");
const K = "5ecsyZA5SVEuTByKoDCUivha8uzAgeQJmgFnnvokFF21";
const EVM = "0x9db3fb117ecfc8fc14ca263e401a4c57a6913d81";
const ETH_NATIVE = "0x0000000000000000000000000000000000000000";
const CASHCAT = "0xe3b34c4bb0f12c82143745eee6a6cf4e3154b1fa";
const RH = 4663;
const rpcAcc = (f: string) => { const v = JSON.parse(raw(f)).result.value; return { owner: v.owner as string, data: Uint8Array.from(Buffer.from(v.data[0], "base64")) }; };
const S3_READ: S3 = s3Reads(rpcAcc("depository-account.json"), rpcAcc("programdata-head.json"));
type Q = any; // eslint-disable-line @typescript-eslint/no-explicit-any

/** Every "ethereum" refund named "robinhood" and the order id recomputed (the canister's tests' `rename_refunds_and_rehash`). */
function rehash(q: Q): string {
  const o = q.protocol.v2.orderData;
  for (const r of o.inputs[0].refunds) if (r.chainId === "ethereum") r.chainId = "robinhood";
  const id = orderIdAny(o);
  q.protocol.v2.orderId = id;
  return id;
}

/** Relay's real SOL → ETH quote with its output on Robinhood Chain, rebound as Relay would (the canister's `sol_to_rh`). */
function solToRh(): Q {
  const q = JSON.parse(SOL_ETH);
  q.protocol.v2.orderData.output.chainId = "robinhood";
  const id = rehash(q);
  const ix = q.steps[0].items[0].data.instructions[0];
  ix.data = ix.data.slice(0, 32) + id.slice(2);
  return q;
}

/** Relay's real ETH → SOL quote with its origin Robinhood Chain: the deposit on 4663 (the same depository), the order's payment and origin
 * refund on "robinhood", its id recomputed in `depositNative(address, bytes32)` (the canister's `rh_to_sol`). */
function rhToSol(): Q {
  const q = JSON.parse(ETH_SOL);
  const d = q.steps[0].items[0].data;
  d.chainId = RH;
  q.protocol.v2.orderData.inputs[0].payment.chainId = "robinhood";
  q.protocol.v2.paymentDetails.chainId = "robinhood";
  const id = rehash(q);
  d.data = d.data.slice(0, 2 + 8 + 64) + id.slice(2);
  return q;
}

const fwd: AnyAsk = { amountWei: 1_000_000_000n, user: K, recipient: EVM, refundTo: K, originChainId: RELAY_SOLANA, destinationChainId: RH,
  originCurrency: SOL_NATIVE, destinationCurrency: ETH_NATIVE };
const back: AnyAsk = { amountWei: 500_000_000_000_000_000n, user: EVM, recipient: K, refundTo: EVM, originChainId: RH, destinationChainId: RELAY_SOLANA,
  originCurrency: ETH_NATIVE, destinationCurrency: SOL_NATIVE };

test("fixed C3-03: the pre-judge judges both Relay legs of a Robinhood Chain buy, each against its own chain", () => {
  assert.equal(solLegOf(fwd), "sol-rh");
  assert.equal(solLegOf(back), "rh-sol");
  const j = preJudgeSolAsk(solToRh(), fwd, AT["sol-eth-1"], S3_READ);
  assert.ok(j.orderId.startsWith("0x") && j.minOut > 0n);
  assert.ok(preJudgeSolAsk(rhToSol(), back, AT["eth-sol-0.5"]).orderId.startsWith("0x"));
  // each leg is its own chain's: Ethereum's quotes are refused on the 4663 legs (S9 / rule 2), and the 4663 quotes on Ethereum's
  assert.throws(() => preJudgeSolAsk(JSON.parse(SOL_ETH), fwd, AT["sol-eth-1"], S3_READ), /rule S8|rule S9/);
  assert.throws(() => preJudgeSolAsk(JSON.parse(ETH_SOL), back, AT["eth-sol-0.5"]), /rule 2: the deposit is for chain 1, not robinhood \(4663\)/);
  assert.throws(() => preJudgeSolAsk(solToRh(), { ...fwd, destinationChainId: 1 }, AT["sol-eth-1"], S3_READ), /rule S8|rule S9/);
  assert.throws(() => preJudgeSolAsk(rhToSol(), { ...back, originChainId: 1 }, AT["eth-sol-0.5"]), /rule 2: the deposit is for chain 4663, not Ethereum/);
  // the Ethereum legs are judged as before; an unknown chain still has no leg
  assert.ok(preJudgeSolAsk(JSON.parse(SOL_ETH), { ...fwd, destinationChainId: 1 }, AT["sol-eth-1"], S3_READ).orderId.startsWith("0x"));
  assert.throws(() => preJudgeSolAsk(solToRh(), { ...fwd, destinationChainId: 8453 }, AT["sol-eth-1"], S3_READ), /no Solana Relay leg from chain 792703809 to 8453/);
});

test("fixed C3-03: one runner pass with a Funded Robinhood Chain buy hands Relay's 4663 quote in for its side's buy id", async () => {
  const side = 4663n << 40n;
  const buy: EthBuyV = { id: side, collection: Buffer.from(CASHCAT.slice(2), "hex"), module: [], wrapper: [], advanced: 1_000_000_000n, attempts: 0, note: "",
    phase: { Funded: { wei: 1_000_000_000n, since: 0n } },
    ask: [{ amount_wei: 1_000_000_000n, user: K, recipient: EVM, refund_to: K, origin_chain_id: BigInt(RELAY_SOLANA), destination_chain_id: BigInt(RH),
      origin_currency: SOL_NATIVE, destination_currency: ETH_NATIVE }] } as never;
  let asked = 0;
  const fetchImpl = (async (url: string) => {
    if (String(url).includes("relay.link")) { asked += 1; return new Response(JSON.stringify(solToRh())); }
    return new Response(JSON.stringify({ listings: [] }));
  }) as unknown as typeof fetch;
  const handed: bigint[] = [];
  const lines: string[] = [];
  const buyer = {
    eth_nft_config: async () => [{ chain_id: 1n, relay_max_fee_bps: 300, deposits: false, collections: [{ collection: CASHCAT, wrapper: "books", module: "books",
      max_purchase_wei: 2n * 10n ** 18n, max_premium_bps: 500, chain_id: [BigInt(RH)], token_bound: [] }] }] as never,
    eth_nft_state: async () => ({ buys: [], deliveries: [], readings: [], eth_out: 0n, deposits: [] }),
    evm_address: async () => ({ Ok: EVM }),
    eth_relay_quote: async (b: bigint) => { handed.push(b); return { Ok: "held" }; }, eth_listing: async () => ({ Err: "x" }), eth_deposit: async () => ({ Err: "x" }),
    eth_floor_evidence: async () => ({ Ok: "held" }),
  };
  await ethNftOnce({ buyer, fetch: fetchImpl, log: logger((l) => lines.push(JSON.stringify(l))), slugs: { [CASHCAT]: "cashcats" }, openseaKey: () => "k",
    ethRpc: null, nowS: () => AT["sol-eth-1"], relayQuotePath: "/quote/v2", solJudge: preJudgeSolAsk, s3: async () => S3_READ,
    sideStates: async () => [{ buys: [buy], deliveries: [], readings: [], eth_out: 0n, deposits: [] } as never] });
  assert.equal(asked, 1, "Relay was asked for the 4663 leg");
  assert.deepEqual(handed, [side], "the quote is handed in for the 4663 side's buy");
  assert.ok(lines.some((l) => l.includes("eth-relay-handed-in")), lines.join("\n"));
  assert.ok(!lines.some((l) => l.includes("eth-relay-rejected")), lines.join("\n"));
});
