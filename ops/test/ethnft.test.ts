import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { advancedFromProtocolData, calldataFromFulfillment, chainOfCollection, decodeAdvanced, ethAskOf, ethNftOnce, ethQuoteBody, openseaChainOf, parseSlugs, preJudgeEth, SEAPORT_1_6,
  type EthAsk } from "../src/ethnft.js";
import { logger } from "../src/log.js";
import type { EthBuyV } from "../src/ic.js";

// docs/v7/phase1-nft.md §6: the runner's half, on Relay's real 4663 → Ethereum quote (shared with the canister's tests).
const FX = new URL("../../autobuyer/canister-rs/tests/fixtures/", import.meta.url);
const Q = readFileSync(new URL("relay-quote-4663-1-2026-09-27.json", FX), "utf8");
const US = "0x76020b7093bd35d2520d47ded764efc554237efe";
const NOW = 1_791_117_628 - 7 * 86_400;
const ASK: EthAsk = { buy: 3n, amountWei: 500_000_000_000_000_000n, user: US, recipient: US, refundTo: US, originChainId: 4663, destinationChainId: 1 };
const edit = (f: (q: any) => void): any => { const q = JSON.parse(Q); f(q); return q; }; // eslint-disable-line @typescript-eslint/no-explicit-any
const silent = logger(() => {});

test("the real 4663 → Ethereum quote passes the pre-judge, its order id recomputed", () => {
  const j = preJudgeEth(JSON.parse(Q), ASK, NOW);
  assert.equal(j.orderId, "0xbce9b0ca025fca35649d3c604117f642667842017c6527fc2ecf002ebc2e69e2");
  assert.equal(j.minOut, 488_475_083_200_866_055n);
  assert.deepEqual(ethQuoteBody(ASK), { user: US, originChainId: 4663, destinationChainId: 1, originCurrency: "0x0000000000000000000000000000000000000000",
    destinationCurrency: "0x0000000000000000000000000000000000000000", recipient: US, tradeType: "EXACT_INPUT", amount: "500000000000000000", refundTo: US });
});

test("each lie is refused before anything is handed in", () => {
  const r = (q: any, a = ASK) => assert.throws(() => preJudgeEth(q, a, NOW)); // eslint-disable-line @typescript-eslint/no-explicit-any
  r(edit((q) => { q.steps.push(q.steps[0]); }));
  r(edit((q) => { q.steps[0].items[0].data.to = "0x1111111111111111111111111111111111111111"; }));
  r(edit((q) => { q.steps[0].items[0].data.value = "1"; }));
  r(edit((q) => { q.protocol.v2.orderData.output.payments[0].recipient = "0x1111111111111111111111111111111111111111"; }));   // the hash no longer matches
  r(edit((q) => { q.protocol.v2.orderData.output.chainId = "base"; }));
  r(JSON.parse(Q), { ...ASK, user: "0x2222222222222222222222222222222222222222" });
  r(JSON.parse(Q), { ...ASK, amountWei: 1n });
  assert.throws(() => preJudgeEth(JSON.parse(Q), ASK, NOW + 7 * 86_400), /deadline/);
  assert.throws(() => preJudgeEth(JSON.parse(Q), ASK, NOW, 200), /minimum out/, "a 2 % bound refuses Relay's 2.3 % minimum");
});

const PD = {
  parameters: { offerer: "0x1f3bd426d37864c0c97ff40aeadf5b11e7189f9f", zone: "0x0000000000000000000000000000000000000000",
    offer: [{ itemType: 2, token: "0x5af0d9827e0c53e4799bb226655a1de152a425a5", identifierOrCriteria: "9180", startAmount: "1", endAmount: "1" }],
    consideration: [
      { itemType: 0, token: "0x0000000000000000000000000000000000000000", identifierOrCriteria: "0", startAmount: "989900999010000000", endAmount: "989900999010000000", recipient: "0x1f3bd426d37864c0c97ff40aeadf5b11e7189f9f" },
      { itemType: 0, token: "0x0000000000000000000000000000000000000000", identifierOrCriteria: "0", startAmount: "9999000000000000", endAmount: "9999000000000000", recipient: "0x0000a26b00c1f0df003000390027140000faa719" },
    ],
    orderType: 0, startTime: "1790000000", endTime: "1800000000", zoneHash: `0x${"0".repeat(64)}`, salt: "123", conduitKey: "0x0000007b02230091a7ed01230072f7006a004d60a8d4e71d599b8104250f0000",
    totalOriginalConsiderationItems: 2 },
  signature: `0x${"ab".repeat(65)}`,
};

test("OpenSea's protocol data becomes fulfillAdvancedOrder calldata the canister decodes (evidence), to our address", () => {
  const cd = advancedFromProtocolData(PD, US);
  assert.equal(cd.slice(0, 10), "0xe7acab24");
  const d = decodeAdvanced(cd);
  assert.equal((d.recipient as string).toLowerCase(), US);
  assert.equal(d.advancedOrder.parameters.consideration.length, 2);
  assert.equal(d.advancedOrder.numerator, 1n);
  assert.equal(d.advancedOrder.extraData, "0x");
});

test("fulfillment data: a basic order is re-encoded with the fulfiller's conduit zeroed; anything not to Seaport is refused", () => {
  const p = { considerationToken: "0x0000000000000000000000000000000000000000", considerationIdentifier: "0", considerationAmount: "989900999010000000",
    offerer: PD.parameters.offerer, zone: PD.parameters.zone, offerToken: PD.parameters.offer[0].token, offerIdentifier: "9180", offerAmount: "1", basicOrderType: 0,
    startTime: "1790000000", endTime: "1800000000", zoneHash: PD.parameters.zoneHash, salt: "123", offererConduitKey: PD.parameters.conduitKey,
    fulfillerConduitKey: PD.parameters.conduitKey, totalOriginalAdditionalRecipients: "1",
    additionalRecipients: [{ amount: "9999000000000000", recipient: "0x0000a26b00c1f0df003000390027140000faa719" }], signature: PD.signature };
  const cd = calldataFromFulfillment({ function: "fulfillBasicOrder_efficient_6GL6yc(...)", to: SEAPORT_1_6, input_data: { parameters: p } });
  assert.equal(cd.slice(0, 10), "0xfb0f3ee1");
  assert.ok(!cd.includes(PD.parameters.conduitKey.slice(2)) || cd.split(PD.parameters.conduitKey.slice(2)).length === 2, "only the offerer's conduit key remains");
  assert.throws(() => calldataFromFulfillment({ function: "fulfillBasicOrder", to: "0x1111111111111111111111111111111111111111", input_data: { parameters: p } }), /not Seaport/);
  assert.throws(() => calldataFromFulfillment({ function: "matchAdvancedOrders", to: SEAPORT_1_6, input_data: {} }), /not fulfillBasicOrder/);
  const adv = calldataFromFulfillment({ function: "fulfillAdvancedOrder", to: SEAPORT_1_6, input_data: { advancedOrder: { parameters: PD.parameters,
    numerator: 1, denominator: 1, signature: PD.signature, extraData: "0x00aa" }, recipient: US } });
  assert.equal(decodeAdvanced(adv).advancedOrder.extraData, "0x00aa", "the zone's data is kept");
});

test("slugs parse strictly", () => {
  assert.deepEqual(parseSlugs("0x5Af0D9827E0c53E4799BB226655A1de152A425a5=milady"), { "0x5af0d9827e0c53e4799bb226655a1de152a425a5": "milady" });
  assert.deepEqual(parseSlugs(undefined), {});
  assert.throws(() => parseSlugs("milady=0x5af0"), /not 0x<collection>/);
});

function buy(phase: Record<string, unknown>, ask = false): EthBuyV {
  return { id: 3n, collection: Buffer.from("5af0d9827e0c53e4799bb226655a1de152a425a5", "hex"), module: new Uint8Array(20), wrapper: new Uint8Array(20),
    advanced: 0n, attempts: 0, phase, note: "", ask: ask ? [{ amount_wei: ASK.amountWei, user: US, recipient: US, refund_to: US, origin_chain_id: 4663n,
      destination_chain_id: 1n, origin_currency: "0x0", destination_currency: "0x0" }] : [] };
}

const TRANSFER = "0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef";
/** A Seaport OrderFulfilled of a Milady and the collection's Transfer (seller → buyer), or a plain wallet move (no market log). */
function saleReceipt(sale: boolean, collection = "0x5af0d9827e0c53e4799bb226655a1de152a425a5", from = "0x11", to = "0x22") {
  const pad = (a: string) => `0x${a.replace(/^0x/, "").padStart(64, "0")}`;
  const logs = [{ address: collection, topics: [TRANSFER, pad(from), pad(to), pad("0x5")], data: "0x" }];
  if (sale) logs.push({ address: SEAPORT_1_6, topics: ["0x9d9af8e38d66c62e2c12f0225249fd9d721c54b83f48d9352c97c6cacdcb6f31", pad(from), pad("0x0")],
    data: `0x${"00".repeat(32)}${pad(to).slice(2)}${pad(collection).slice(2)}` });
  return { status: "0x1", logs };
}

test("one pass: a funded buy's quote, an arrived buy's listing and the floor evidence are handed in; the key is never sent to the canister", async () => {
  const calls: string[] = [];
  const handed: Record<string, unknown[]> = { quote: [], listing: [], evidence: [] };
  const fetchImpl = (async (url: string, init?: RequestInit) => {
    const u = String(url);
    calls.push(`${init?.method ?? "GET"} ${u.replace(/\?.*/, "")}`);
    if (u.endsWith("/quote")) return new Response(Q);
    if (u.includes("/listings/collection/milady/best")) {
      assert.equal((init?.headers as Record<string, string>)["x-api-key"], "sekret");
      return new Response(JSON.stringify({ listings: [{ order_hash: "0xaa", protocol_address: SEAPORT_1_6, protocol_data: PD, price: { current: { value: "999899999000000000" } } }] }));
    }
    if (u.endsWith("/fulfillment_data")) {
      return new Response(JSON.stringify({ fulfillment_data: { transaction: { function: "fulfillAdvancedOrder", to: SEAPORT_1_6, input_data: {
        advancedOrder: { parameters: PD.parameters, numerator: 1, denominator: 1, signature: PD.signature, extraData: "0x" }, recipient: US } } } }));
    }
    if (u === "https://eth.example") {
      const b = JSON.parse(String(init?.body));
      if (b.method === "eth_blockNumber") return new Response(JSON.stringify({ result: "0x100000" }));
      // audit 2026-09-27-nft D-H-1: transfers, then each transaction's receipt: only sales are handed in
      if (b.method === "eth_getTransactionReceipt") return new Response(JSON.stringify({ result: saleReceipt(b.params[0] !== "0x03") }));
      const t = (h: string, n: number) => ({ transactionHash: h, blockNumber: `0x${n.toString(16)}`, logIndex: "0x0", topics: [TRANSFER, "0x1", "0x2", "0x3"] });
      return new Response(JSON.stringify({ result: Number(BigInt(b.params[0].toBlock)) === 0x100000 ? [t("0x01", 0x100000 - 5), t("0x02", 0x100000 - 2), t("0x02", 0x100000 - 2), t("0x03", 0x100000 - 1)] : [] }));
    }
    throw new Error(`unexpected ${u}`);
  }) as unknown as typeof fetch;
  const buyer = {
    eth_nft_config: async () => [{ chain_id: 1n, relay_max_fee_bps: 300, deposits: false, collections: [{ collection: "0x5af0d9827e0c53e4799bb226655a1de152a425a5",
      wrapper: "0xaa", module: "0xbb", max_purchase_wei: 10n ** 18n, max_premium_bps: 500 }] }] as never,
    eth_nft_state: async () => ({ buys: [buy({ Funded: { wei: 1n, since: 1n } }, true), { ...buy({ Arrived: { wei: 11n * 10n ** 17n, since: 1n } }), id: 4n }], deliveries: [], readings: [], eth_out: 0n, deposits: [] }),
    evm_address: async () => ({ Ok: US }),
    eth_relay_quote: async (b: bigint, q: string) => { handed.quote.push([b, q.length]); return { Ok: "held" }; },
    eth_listing: async (b: bigint, o: string) => { handed.listing.push([b, o.slice(0, 10)]); return { Ok: "held" }; },
    eth_floor_evidence: async (c: string, l: string[], s: string[]) => { handed.evidence.push([c, l.length, s]); return { Ok: "held" }; },
    eth_deposit: async () => { throw new Error("deposits are off in this config: no claim is handed in"); },
  };
  const got = await ethNftOnce({ buyer, fetch: fetchImpl, log: silent, slugs: { "0x5af0d9827e0c53e4799bb226655a1de152a425a5": "milady" },
    openseaKey: () => "sekret", ethRpc: "https://eth.example", nowS: () => NOW });
  assert.equal(got, true);
  assert.deepEqual(handed.quote, [[3n, Q.length]]);
  assert.deepEqual(handed.listing, [[4n, "0xe7acab24"]]);
  // round 2 (A-M-1 / B2-M-1 / D2-M-2): listings only; the canister reads the collection's sales itself
  assert.deepEqual(handed.evidence, [["0x5af0d9827e0c53e4799bb226655a1de152a425a5", 1, []]], "no sale hash is handed in");
  assert.ok(!JSON.stringify(handed, (_k, v) => (typeof v === "bigint" ? v.toString() : v)).includes("sekret"));
  assert.ok(ethAskOf(buy({ Arrived: { wei: 1n, since: 1n } }, true)) === null, "only a funded buy (or one going back) is quoted");
});

test("holder deposits (D5): every inbound transfer the canister has not judged is handed in once; ours and judged ones are not", async () => {
  const MIL = "0x5af0d9827e0c53e4799bb226655a1de152a425a5";
  const topicUs = `0x${"0".repeat(24)}${US.slice(2)}`;
  const asked: AnyLog[] = [];
  type AnyLog = { fromBlock: string; toBlock: string; topics: (string | null)[] };
  const fetchImpl = (async (url: string, init?: RequestInit) => {
    const b = JSON.parse(String(init?.body));
    if (b.method === "eth_blockNumber") return new Response(JSON.stringify({ result: `0x${(1_000_000).toString(16)}` }));
    if (b.method === "eth_getLogs") {
      const q = b.params[0];
      asked.push(q);
      assert.equal(q.address, MIL);
      assert.deepEqual(q.topics, ["0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef", null, topicUs], "Transfer(*, us, *)");
      const first = Number(BigInt(q.fromBlock)) === 1_000_000 - 7_200;
      return new Response(JSON.stringify({ result: first ? [
        { transactionHash: "0xAA", topics: ["t", "a", "b", "c"] },         // a deposit or a stray: handed in
        { transactionHash: "0xbb", topics: ["t", "a", "b", "c"] },         // already judged by the canister
        { transactionHash: "0xcc", topics: ["t", "a", "b", "c"] },         // our own fill
        { transactionHash: "0xdd", topics: ["t", "a", "b"] },              // an ERC-20 Transfer: not an NFT
        { transactionHash: "0xaa", topics: ["t", "a", "b", "c"] },         // the same transaction twice
      ] : [] }));
    }
    throw new Error(`unexpected ${url} ${b.method}`);
  }) as unknown as typeof fetch;
  const claims: string[] = [];
  const cfgOn = [{ chain_id: 1n, relay_max_fee_bps: 300, deposits: true, collections: [{ collection: MIL, wrapper: "0xaa", module: "0xbb",
    max_purchase_wei: 10n ** 18n, max_premium_bps: 500 }] }];
  const buyer = {
    eth_nft_config: async () => cfgOn as never,
    eth_nft_state: async () => ({ buys: [buy({ Bought: { arrived: 1n, token: new Uint8Array(32), total: 1n, tx: Buffer.from("cc", "hex"), block: 1n } })],
      deliveries: [], readings: [], eth_out: 0n, deposits: [{ tx: Buffer.from("bb", "hex"), collection: new Uint8Array(20), token: new Uint8Array(32) }] }),
    evm_address: async () => ({ Ok: US }),
    eth_relay_quote: async () => ({ Err: "no" }), eth_listing: async () => ({ Err: "no" }), eth_floor_evidence: async () => ({ Err: "no" }),
    eth_deposit: async (tx: string) => { claims.push(tx); return { Ok: "claim held" }; },
  };
  const claimed = new Map<string, number>();
  const deps = { buyer, fetch: fetchImpl, log: silent, slugs: {}, openseaKey: () => undefined, ethRpc: "https://eth.example", nowS: () => NOW, claimed };
  assert.equal(await ethNftOnce(deps), true);
  assert.deepEqual(claims, ["0xaa"]);
  // the operator's Dwellir plan (2026-09-27): at most 500 blocks a request, so a day of blocks is 15 of them
  assert.equal(asked.length, 15, "a day of blocks in chunks of at most 500");
  assert.ok(asked.every((q) => Number(BigInt(q.toBlock)) - Number(BigInt(q.fromBlock)) + 1 <= 500));
  const before = asked.length;
  await ethNftOnce({ ...deps, logsMaxBlocks: 2_400, claimed: new Map() });
  assert.equal(asked.length - before, 4, "ETH_LOGS_MAX_BLOCKS sets another width");
  asked.length = before;
  claims.length = 1;          // that pass had its own empty claim memory: it handed 0xaa in again
  // the next pass does not hand it in again (it waits in the canister's inbox), and deposits off hands in nothing
  await ethNftOnce(deps);
  assert.deepEqual(claims, ["0xaa"]);
  cfgOn[0].deposits = false;
  claimed.clear();
  await ethNftOnce(deps);
  assert.deepEqual(claims, ["0xaa"]);
});

// ---- 2026-10-12: Robinhood Chain NFTs (docs/v9/robinhood-everything.md §3) ----------------------------------------------------------------

test("OpenSea's chain names: Ethereum's, Robinhood Chain's (configurable), and a collection's chain from the canister's config", () => {
  assert.equal(openseaChainOf(1), "ethereum");
  assert.equal(openseaChainOf(4663), "robinhood");
  assert.equal(openseaChainOf(4663n, { "4663": "robinhood_chain" }), "robinhood_chain", "the runner's override (OPENSEA_CHAIN_4663) wins");
  assert.throws(() => openseaChainOf(8453), /no OpenSea chain name/);
  const cfg = { chain_id: 1n, collections: [{ collection: "0x5AF0d9827e0c53e4799bb226655a1de152a425a5" }, { collection: CASHCAT, chain_id: [4663n] },
    { collection: "0x11", chain_id: [] }] } as never;
  assert.equal(chainOfCollection(cfg, "0x5af0d9827e0c53e4799bb226655a1de152a425a5"), 1, "no chain_id: the config's");
  assert.equal(chainOfCollection(cfg, CASHCAT.toUpperCase().replace("0X", "0x")), 4663);
  assert.equal(chainOfCollection(cfg, "0x11"), 1, "an empty opt: the config's");
});

const CASHCAT = "0xe3b34c4bb0f12c82143745eee6a6cf4e3154b1fa";

test("one pass with a Robinhood Chain side: its arrived buy's listing is fetched on OpenSea's Robinhood Chain and handed in by its own buy id", async () => {
  const fulfil: unknown[] = [];
  const handed: unknown[] = [];
  const fetchImpl = (async (url: string, init?: RequestInit) => {
    const u = String(url);
    if (u.includes("/listings/collection/cashcats/best")) {
      return new Response(JSON.stringify({ listings: [{ order_hash: "0xcc", protocol_address: SEAPORT_1_6, protocol_data: PD, price: { current: { value: "250000000000000000" } } }] }));
    }
    if (u.endsWith("/fulfillment_data")) {
      fulfil.push(JSON.parse(String(init?.body)));
      return new Response(JSON.stringify({ fulfillment_data: { transaction: { function: "fulfillAdvancedOrder", to: SEAPORT_1_6, input_data: {
        advancedOrder: { parameters: PD.parameters, numerator: 1, denominator: 1, signature: PD.signature, extraData: "0x" }, recipient: US } } } }));
    }
    throw new Error(`unexpected ${u}`);
  }) as unknown as typeof fetch;
  const side = 4663n << 40n;
  const rhBuy = { ...buy({ Arrived: { wei: 5n * 10n ** 17n, since: 1n } }), id: side, collection: Buffer.from(CASHCAT.slice(2), "hex") };
  const buyer = {
    eth_nft_config: async () => [{ chain_id: 1n, relay_max_fee_bps: 300, deposits: false, collections: [{ collection: CASHCAT, wrapper: "books", module: "books",
      max_purchase_wei: 2n * 10n ** 18n, max_premium_bps: 500, chain_id: [4663n], token_bound: [] }] }] as never,
    eth_nft_state: async () => ({ buys: [], deliveries: [], readings: [], eth_out: 0n, deposits: [] }),
    evm_address: async () => ({ Ok: US }),
    eth_relay_quote: async () => ({ Err: "no" }),
    eth_listing: async (b: bigint, o: string) => { handed.push([b, o.slice(0, 10)]); return { Ok: "held" }; },
    eth_floor_evidence: async () => ({ Ok: "held" }),
    eth_deposit: async () => { throw new Error("off"); },
  };
  await ethNftOnce({ buyer, fetch: fetchImpl, log: silent, slugs: { [CASHCAT]: "cashcats" }, openseaKey: () => "sekret", ethRpc: null, nowS: () => NOW,
    sideStates: async () => [{ buys: [rhBuy], deliveries: [], readings: [], eth_out: 0n, deposits: [] } as never] });
  assert.equal(fulfil.length, 1);
  assert.equal((fulfil[0] as { listing: { chain: string } }).listing.chain, "robinhood", "the fulfillment is asked on the collection's chain");
  assert.deepEqual(handed, [[side, "0xe7acab24"]], "handed in by the 4663 side's buy id");
});

test("floor evidence: a best listing with no signature is signed through fulfillment_data (devnet rehearsal 2026-10-12)", async () => {
  const { evidenceListings } = await import("../src/ethnft.ts");
  const us = "0x40f46d1dc19928876c85b4c616996397ce456fb3";
  const params = { offerer: "0x55e73d2df424dba53c23bee13a0f40054622af06", zone: "0x0000000000000000000000000000000000000000",
    offer: [{ itemType: 2, token: "0xbc4ca0eda7647a8ab7c2061c2e118a18a936f13d", identifierOrCriteria: "1", startAmount: "1", endAmount: "1" }],
    consideration: [{ itemType: 0, token: "0x0000000000000000000000000000000000000000", identifierOrCriteria: "0", startAmount: "6230000000000000000",
      endAmount: "6230000000000000000", recipient: "0x55e73d2df424dba53c23bee13a0f40054622af06" }],
    orderType: 0, startTime: "1", endTime: "9999999999", zoneHash: `0x${"00".repeat(32)}`, salt: "7", conduitKey: `0x${"00".repeat(32)}`, totalOriginalConsiderationItems: 1 };
  const sig = `0x${"ab".repeat(65)}`;
  const posted: string[] = [];
  const fetchF = async (url: string, init?: RequestInit) => {
    if (url.includes("/listings/collection/")) return new Response(JSON.stringify({ listings: [
      { order_hash: "0xa1", protocol_address: SEAPORT_1_6, protocol_data: { parameters: params, signature: "0x" } },
      { order_hash: "0xa2", protocol_address: SEAPORT_1_6, protocol_data: { parameters: params, signature: sig } }] }));
    if (url.endsWith("/fulfillment_data")) {
      posted.push(JSON.parse(String(init?.body)).listing.hash);
      return new Response(JSON.stringify({ fulfillment_data: { transaction: { function: "fulfillAdvancedOrder((…))", to: SEAPORT_1_6, input_data: {
        advancedOrder: { parameters: params, numerator: "1", denominator: "1", signature: sig, extraData: "0x" }, recipient: us } } } }));
    }
    throw new Error(url);
  };
  const d = { fetch: fetchF, openseaKey: () => "k", log: () => {} } as never;
  const out = await evidenceListings(d, "boredapeyachtclub", "ethereum", us);
  assert.deepEqual(posted, ["0xa1"], "only the unsigned listing is fetched again");
  assert.equal(out.length, 2);
  for (const cd of out) assert.equal(String(decodeAdvanced(cd).advancedOrder.signature).toLowerCase(), sig, "both carry the signature");
});
