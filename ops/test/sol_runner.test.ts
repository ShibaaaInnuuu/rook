import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";
import { test } from "node:test";
import { orderIdAny, s3Reads, type S3 } from "../src/fuel/sol.js";
import { ethNftOnce, preJudgeEth } from "../src/ethnft.js";
import { logger } from "../src/log.js";
import { liveSatsPerSolHomeUnit } from "../src/owprotected.js";
import { convertOnce } from "../src/relay.js";
import { judgeEthToSol, judgeSolOrigin, preJudgeSolAsk, readS3, RELAY_SOL, RELAY_SOLANA, SOL_NATIVE, solQuoteBodyOf, type AnyAsk } from "../src/relaysol.js";
import { homeOf, parseRunnerConfig, runnerSecrets, RunnerConfigError, sleepAfter } from "../src/runner.js";
import { coingeckoReader, frogFloorOnce, inBand, inscriptionListings, loadInscriptionListings, staleFloors, type FrogColl, type Held, type ReadFloor } from "../src/solfloor.js";
import type { ConvRecV, EthBuyV } from "../src/ic.js";

// docs/v9/solana-migration.md §5 "Runner" on the Solana home: Relay's real Solana-side quotes (spike Q5, fetched keyless 2026-10-09 and
// shared with the canister's tests), the canister's rules S1–S12 / judge_to_solana, the frog floor reading, the config and the cadence.
const FX = new URL("fixtures/relay-sol/", import.meta.url);
const raw = (f: string) => readFileSync(new URL(f, FX), "utf8");
const AT = JSON.parse(raw("manifest.json")).fetched_at as Record<string, number>;
const SOL_BTC = raw("sol-btc-1.quote.json"), SOL_ETH = raw("sol-eth-1.quote.json"), ETH_SOL = raw("eth-sol-0.5.quote.json");
/** the key every fixture quote was made for (sender, depositor and refund: before audit A5-03 split them) */
const K = "5ecsyZA5SVEuTByKoDCUivha8uzAgeQJmgFnnvokFF21";
/** a second key: the canister's MAIN Solana address once the Relay key is its own (A5-03) */
const MAIN = "GGgXnNgtY4ySfz8PLAAwkoh8Ztj7p6jrM9towSaa6Wuy";
const FLOAT = "bc1p49upe77txy03ysp4m3r4m992tf27d2xgce7yk6w53qg5jtc85exqtkpg3t";
const EVM = "0x9db3fb117ecfc8fc14ca263e401a4c57a6913d81";
const BTC_NATIVE = "bc1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqmql8k8", ETH_NATIVE = "0x0000000000000000000000000000000000000000";
const rpcAcc = (f: string) => { const v = JSON.parse(raw(f)).result.value; return { owner: v.owner as string, data: Uint8Array.from(Buffer.from(v.data[0], "base64")) }; };
const S3_READ: S3 = s3Reads(rpcAcc("depository-account.json"), rpcAcc("programdata-head.json"));
const silent = logger(() => {});
type Q = any; // eslint-disable-line @typescript-eslint/no-explicit-any

/** Edit a Solana-origin quote's order, then rebind it as Relay would: the new order id in the instruction's data and in orderId. */
function rebind(text: string, f: (q: Q) => void): Q {
  const q = JSON.parse(text);
  f(q);
  const id = orderIdAny(q.protocol.v2.orderData);
  const ix = q.steps[0].items[0].data.instructions[0];
  ix.data = ix.data.slice(0, 32) + id.slice(2);
  q.protocol.v2.orderId = id;
  return q;
}
const btcWant = (o: Partial<Parameters<typeof judgeSolOrigin>[1]> = {}) => ({ ourSol: K, depositor: K, destChain: "bitcoin" as const, destRecipient: FLOAT,
  destCurrency: BTC_NATIVE, lamports: 1_000_000_000n, s3: S3_READ, nowS: AT["sol-btc-1"], ...o });

test("S1–S12: Relay's real SOL → BTC quote passes, its order id recomputed; S3 from the depository's real accounts", () => {
  assert.deepEqual(S3_READ, { allocator: RELAY_SOL.ALLOCATOR, upgradeAuthority: RELAY_SOL.OWNER_AND_UPGRADE_AUTHORITY, deploySlot: RELAY_SOL.PROGRAM_DEPLOY_SLOT });
  const j = judgeSolOrigin(JSON.parse(SOL_BTC), btcWant());
  assert.equal(j.orderId, "0xdbb7dd438a26bafdb187c7bec9c23b5fe663829d5e0f9cef88a23bb4c238b465");
  assert.equal(j.minOut, 128_264n);
  assert.equal(j.lamports, 1_000_000_000n);
  assert.equal(judgeSolOrigin(JSON.parse(SOL_BTC), btcWant({ s3: null })).orderId, j.orderId, "S3 unread: the canister's alone");
});

test("audit A5-03: the Relay key deposits, the MAIN address takes every Solana refund; either one swapped is refused", () => {
  const q = rebind(SOL_BTC, (x) => { x.protocol.v2.orderData.inputs[0].refunds[0].recipient = MAIN; });
  assert.ok(judgeSolOrigin(q, btcWant({ ourSol: MAIN, depositor: K })).orderId.startsWith("0x"));
  assert.throws(() => judgeSolOrigin(q, btcWant({ ourSol: K, depositor: K })), /rule S8/, "a refund to the Relay key is not ours");
  assert.throws(() => judgeSolOrigin(q, btcWant({ ourSol: MAIN, depositor: MAIN })), /rule S5/, "the main key as depositor is not the quote's");
});

test("S1–S11: each lie is refused, naming its rule", () => {
  const r = (q: Q, rule: RegExp, w = btcWant()) => assert.throws(() => judgeSolOrigin(q, w), rule);
  const ed = (f: (q: Q) => void) => { const q = JSON.parse(SOL_BTC); f(q); return q; };
  r(ed((q) => { q.steps.push(q.steps[0]); }), /rule S1/);
  r(ed((q) => { q.steps[0].items[0].data.instructions.push(q.steps[0].items[0].data.instructions[0]); }), /rule S1/);
  r(ed((q) => { q.steps[0].items[0].data.instructions[0].programId = MAIN; }), /rule S3/);
  r(JSON.parse(SOL_BTC), /rule S3/, btcWant({ s3: { ...S3_READ, deploySlot: S3_READ.deploySlot + 1n } }));
  r(JSON.parse(SOL_BTC), /rule S3/, btcWant({ s3: { ...S3_READ, allocator: MAIN } }));
  r(ed((q) => { q.steps[0].items[0].data.instructions[0].keys[3].isWritable = false; }), /rule S5/);
  r(JSON.parse(SOL_BTC), /rule S4/, btcWant({ lamports: 2n }));
  r(ed((q) => { q.protocol.v2.orderData.output.payments[0].minimumAmount = "1"; }), /rule S6/);       // the hash no longer matches
  r(rebind(SOL_BTC, (q) => { q.protocol.v2.orderData.inputs[0].refunds.push({ ...q.protocol.v2.orderData.inputs[0].refunds[0], recipient: MAIN }); }), /rule S8/);
  r(rebind(SOL_BTC, (q) => { q.protocol.v2.orderData.output.payments[0].recipient = "bc1p5cyxnuxmeuwuvkwfem96lqzszd02n6xdcjrs20cac6yqjjwudpxqkedrcr"; }), /rule S9|rule S8/);
  r(rebind(SOL_BTC, (q) => { q.protocol.v2.orderData.fees.push({ recipientChainId: "solana", recipient: MAIN, currencyChainId: "solana", currency: SOL_NATIVE, amount: "1" }); }), /rule S9/);
  r(JSON.parse(SOL_BTC), /rule S10/, btcWant({ minOutFloor: 200_000n }));
  r(JSON.parse(SOL_BTC), /rule S11/, btcWant({ nowS: AT["sol-btc-1"] + 7 * 86_400 }));
});

test("SOL → ETH (an Ethereum buy's advance) and ETH → SOL (its way back): the asks as the canister builds them on the Solana home", () => {
  const fwd: AnyAsk = { amountWei: 1_000_000_000n, user: K, recipient: EVM, refundTo: K, originChainId: RELAY_SOLANA, destinationChainId: 1, originCurrency: SOL_NATIVE, destinationCurrency: ETH_NATIVE };
  const j = preJudgeSolAsk(JSON.parse(SOL_ETH), fwd, AT["sol-eth-1"], S3_READ);
  assert.ok(j.minOut > 0n);
  assert.deepEqual(solQuoteBodyOf(fwd), JSON.parse(raw("sol-eth-1.request.json")), "the request is exactly the spike's");
  assert.throws(() => preJudgeSolAsk(JSON.parse(SOL_ETH), { ...fwd, recipient: "0x1111111111111111111111111111111111111111" }, AT["sol-eth-1"]), /rule S8|rule S9/);
  const back: AnyAsk = { amountWei: 500_000_000_000_000_000n, user: EVM, recipient: K, refundTo: EVM, originChainId: 1, destinationChainId: RELAY_SOLANA,
    originCurrency: ETH_NATIVE, destinationCurrency: SOL_NATIVE };
  const b = preJudgeSolAsk(JSON.parse(ETH_SOL), back, AT["eth-sol-0.5"]);
  assert.equal(b.minOut, 11_068_534_059n);
  assert.deepEqual(solQuoteBodyOf(back), JSON.parse(raw("eth-sol-0.5.request.json")));
  assert.throws(() => judgeEthToSol(JSON.parse(ETH_SOL), { ours: EVM, solRecipient: MAIN, amountWei: back.amountWei, nowS: AT["eth-sol-0.5"] }), /rule 8|rule 9/);
  assert.throws(() => judgeEthToSol(JSON.parse(ETH_SOL), { ours: EVM, solRecipient: K, amountWei: 1n, nowS: AT["eth-sol-0.5"] }), /rule 4/);
  assert.throws(() => judgeEthToSol(JSON.parse(ETH_SOL), { ours: EVM, solRecipient: K, amountWei: back.amountWei, nowS: AT["eth-sol-0.5"], minOutFloor: 12_000_000_000n }), /rule 10/);
  // the Ethereum step's pre-judge routes a Solana leg to the judge the runner hands in, and refuses it without one
  const ea = { buy: 1n, ...fwd };
  assert.equal(preJudgeEth(JSON.parse(SOL_ETH), ea, AT["sol-eth-1"], 300, { judge: preJudgeSolAsk, s3: S3_READ }).orderId, j.orderId);
  assert.throws(() => preJudgeEth(JSON.parse(SOL_ETH), ea, AT["sol-eth-1"]), /no Solana judge/);
});

test("rule S3 read through the runner's Solana reader: the two accounts, or the canister's alone when there is no reader", async () => {
  const got: [string, number | undefined][] = [];
  const s3 = await readS3(async (a, slice) => { got.push([a, slice]); return a === RELAY_SOL.RELAY_DEPOSITORY ? rpcAcc("depository-account.json") : rpcAcc("programdata-head.json"); });
  assert.deepEqual(s3, S3_READ);
  assert.deepEqual(got.sort(), [[RELAY_SOL.PROGRAMDATA, 45], [RELAY_SOL.RELAY_DEPOSITORY, undefined]].sort());
  assert.equal(await readS3(null), null);
  await assert.rejects(readS3(async () => null), /does not exist/);
});

const conv = (ask: AnyAsk): ConvRecV => ({ frog: 7n, module: [], advanced: ask.amountWei, attempts: 0, phase: { Funded: { wei: ask.amountWei, since: 0n } }, orders: [], deposits: [],
  arrival: [], note: "", ask: [{ amount_wei: ask.amountWei, user: ask.user, recipient: ask.recipient, refund_to: ask.refundTo, origin_chain_id: BigInt(ask.originChainId),
    destination_chain_id: BigInt(ask.destinationChainId), origin_currency: ask.originCurrency, destination_currency: ask.destinationCurrency }] });

test("the conversion step on the Solana home: /quote/v2 for exactly the ask, the key only when set, pre-judged by S1–S12, handed in with relay_quote", async () => {
  const ask: AnyAsk = { amountWei: 1_000_000_000n, user: K, recipient: FLOAT, refundTo: K, originChainId: RELAY_SOLANA, destinationChainId: 8253038, originCurrency: SOL_NATIVE, destinationCurrency: BTC_NATIVE };
  const calls: { url: string; headers: Record<string, string>; body: unknown }[] = [];
  const fetchImpl = (async (url: string, init: RequestInit) => { calls.push({ url, headers: init.headers as Record<string, string>, body: JSON.parse(String(init.body)) }); return new Response(SOL_BTC); }) as unknown as typeof fetch;
  const handed: [bigint, string][] = [];
  const buyer = { frog_conversions: async () => [conv(ask)], relay_quote: async (f: bigint, t: string) => { handed.push([f, t]); return { Ok: "held" }; } };
  const lines: string[] = [];
  const ok = await convertOnce({ buyer, fetch: fetchImpl, log: logger((l) => lines.push(l)), quotePath: "/quote/v2", apiKey: () => "relay-key-123456", solJudge: preJudgeSolAsk,
    s3: async () => S3_READ, nowS: () => AT["sol-btc-1"] });
  assert.equal(ok, true);
  assert.equal(calls[0].url, "https://api.relay.link/quote/v2");
  assert.equal(calls[0].headers["x-api-key"], "relay-key-123456");
  assert.deepEqual(calls[0].body, JSON.parse(raw("sol-btc-1.request.json")));
  assert.deepEqual(handed, [[7n, SOL_BTC]]);
  assert.ok(lines.some((l) => l.includes("relay-quote-handed-in") && l.includes("sol-btc")));
  // no key: no header; no Solana judge (an EVM-home runner): refused before any hand-in
  handed.length = 0; calls.length = 0;
  assert.equal(await convertOnce({ buyer, fetch: fetchImpl, log: silent, quotePath: "/quote/v2", nowS: () => AT["sol-btc-1"] }), false);
  assert.equal(calls[0].headers["x-api-key"], undefined);
  assert.deepEqual(handed, []);
  // a quote for a stranger's refund is refused before the call
  const bad = rebind(SOL_BTC, (q) => { q.protocol.v2.orderData.inputs[0].refunds[0].recipient = MAIN; });
  const f2 = (async () => new Response(JSON.stringify(bad))) as unknown as typeof fetch;
  assert.equal(await convertOnce({ buyer, fetch: f2, log: silent, solJudge: preJudgeSolAsk, nowS: () => AT["sol-btc-1"] }), false);
  assert.deepEqual(handed, []);
});

test("the Ethereum step on the Solana home: a Funded buy's SOL → ETH quote is pre-judged and handed in with eth_relay_quote", async () => {
  const MIL = "0x5af0d9827e0c53e4799bb226655a1de152a425a5";
  const buy: EthBuyV = { id: 4n, collection: Buffer.from(MIL.slice(2), "hex"), module: [], wrapper: [], advanced: 1_000_000_000n, attempts: 0, note: "",
    phase: { Funded: { wei: 1_000_000_000n, since: 0n } },
    ask: [{ amount_wei: 1_000_000_000n, user: K, recipient: EVM, refund_to: K, origin_chain_id: BigInt(RELAY_SOLANA), destination_chain_id: 1n, origin_currency: SOL_NATIVE, destination_currency: ETH_NATIVE }] };
  const bodies: unknown[] = [];
  const fetchImpl = (async (url: string, init?: RequestInit) => {
    if (String(url).includes("relay.link")) { bodies.push(JSON.parse(String(init?.body))); return new Response(SOL_ETH); }
    return new Response(JSON.stringify({ listings: [] }));
  }) as unknown as typeof fetch;
  const handed: bigint[] = [];
  const buyer = {
    eth_nft_config: async () => [{ chain_id: 1n, relay_max_fee_bps: 300, deposits: false, collections: [{ collection: MIL, wrapper: "w", module: "m", max_purchase_wei: 1n, max_premium_bps: 500 }] }] as never,
    eth_nft_state: async () => ({ buys: [buy], deliveries: [], readings: [], eth_out: 0n, deposits: [] }),
    evm_address: async () => ({ Ok: EVM }),
    eth_relay_quote: async (b: bigint) => { handed.push(b); return { Ok: "held" }; }, eth_listing: async () => ({ Err: "x" }), eth_deposit: async () => ({ Err: "x" }),
    eth_floor_evidence: async () => ({ Ok: "held" }),
  };
  await ethNftOnce({ buyer, fetch: fetchImpl, log: silent, slugs: { [MIL]: "milady" }, openseaKey: () => "k", ethRpc: null, nowS: () => AT["sol-eth-1"],
    relayQuotePath: "/quote/v2", solJudge: preJudgeSolAsk, s3: async () => S3_READ });
  assert.deepEqual(handed, [4n]);
  assert.deepEqual(bodies[0], JSON.parse(raw("sol-eth-1.request.json")), "the currencies are the ask's (SOL in, ETH out), not ETH → ETH");
});

const PILOT = new URL("../../deployments/launch/nft-pilot.json", import.meta.url).pathname;

test("the Inscriptions listings: Bitcoin Frogs is listing 11 (the canister's wrapper), the others 12+ in file order, 'later' kept as such", () => {
  const ls = loadInscriptionListings(PILOT);
  const pilot = JSON.parse(readFileSync(PILOT, "utf8"));
  assert.equal(ls[0].id, pilot.collections.length);
  assert.equal(ls[0].slug, "bitcoin-frogs");
  assert.equal(ls[0].owSlug, "bitcoin-frogs");
  assert.ok(ls[0].canister && ls.slice(1).every((l) => !l.canister));
  assert.deepEqual(ls.map((l) => l.id), ls.map((_, i) => pilot.collections.length + i));
  assert.equal(ls.length, 1 + pilot.inscriptions.length);
  assert.ok(ls.some((l) => l.wrapping === "later"));
  assert.throws(() => inscriptionListings({ collections: [], bitcoin: { slug: "x", owSlug: "../etc" } }), /not an Ordinals Wallet slug/);
  // Ordinals multi-collection: each listing carries its members root (paired with the canister's), its members file (resolved from the
  // repository root, as the launch file names it) and its CoinGecko id (the band gate's reference)
  assert.equal(ls[0].membersRoot, pilot.bitcoin.membersRoot);
  assert.equal(ls[0].membersFile, new URL("../../deployments/launch/btc/bitcoin-frogs-members.json", import.meta.url).pathname);
  assert.ok(ls.every((l) => existsSync(l.membersFile) && /^0x[0-9a-f]{64}$/.test(l.membersRoot) && l.coingeckoId));
});

test("the frog floor reading per collection: every LIVE canister collection (paired by root) is read and handed in with its coll, inside the 20 % band; dormant ones only with 'all', held; 'later' dormant ones never read; a refusal is logged", async () => {
  const listings = loadInscriptionListings(PILOT);
  const frogs = listings[0], omb = listings.find((l) => l.slug === "omb")!, runestone = listings.find((l) => l.slug === "runestone")!;
  const read: string[] = [];
  const reader: ReadFloor = async (ow) => { read.push(ow); return ow === "nodemonkes" ? { sats: null, ref: `ordinalswallet:${ow}`, why: "no floor: fewer than 3 valid listings" }
    : { sats: 1_234_567n, confidenceBps: 8_000, depthItems: 9, ref: `ordinalswallet:${ow}`, why: "ok" }; };
  let refSats: bigint | null = 1_100_000n;                       // the reading is 12.2 % above: inside the band
  const refs: string[] = [];
  const ref = async (id: string) => { refs.push(id); return refSats; };
  // the canister: Bitcoin Frogs (coll 4) live, OMB (24) dormant, Runestone (28, wrapping "later") dormant, NodeMonkes (25) dormant
  const colls: FrogColl[] = [{ coll: 4, root: frogs.membersRoot, live: true }, { coll: 24, root: omb.membersRoot.toUpperCase().replace("0X", ""), live: false },
    { coll: 28, root: runestone.membersRoot, live: false }, { coll: 25, root: listings.find((l) => l.slug === "nodemonkes")!.membersRoot, live: false }];
  const posted: [number, bigint, number, number][] = [];
  let answer: { Ok: string } | { Err: string } = { Ok: "posted at the next tick" };
  const buyer = { frog_floor_reading: async (c: number, s: bigint, cb: number, d: number) => { posted.push([c, s, cb, d]); return answer; }, frog_collections: async () => colls };
  let now = 1_000_000;
  const last = new Map<number, number>(), held = new Map<number, Held>(), accepted = new Map<number, number>(), live = new Set<number>();
  assert.equal(await frogFloorOnce({ buyer, log: silent, listings, read: reader, ref, nowS: () => now, last, held, accepted, live }), true);
  assert.deepEqual(read, ["bitcoin-frogs"]);
  assert.deepEqual(refs, ["bitcoin-frogs"], "the band's reference is the listing's coingeckoId");
  assert.deepEqual(posted, [[4, 1_234_567n, 8_000, 9]]);
  assert.deepEqual([accepted.get(4), [...live]], [now, [4]]);
  assert.equal(held.get(frogs.id)?.coll, 4);
  // the cadence: not again before everyS
  assert.equal(await frogFloorOnce({ buyer, log: silent, listings, read: reader, ref, nowS: () => now + 60, last, held }), false);
  assert.equal(posted.length, 1);
  // "all": the dormant listings not marked later are read and held, never posted; a 'later' one is never read while dormant
  read.length = 0; posted.length = 0; now += 4_000;
  await frogFloorOnce({ buyer, log: silent, listings, read: reader, ref, scope: "all", nowS: () => now, last, held });
  assert.deepEqual(read, listings.filter((l) => l.wrapping !== "later").map((l) => l.owSlug));
  assert.ok(!read.includes("runestone"));
  assert.deepEqual(posted.map((p) => p[0]), [4]);
  assert.equal(held.get(omb.id)?.posted, false);
  assert.match(held.get(omb.id)!.note, /collection 24 is asleep \(the operator's frog_sleep; frog_wake\(24\) lifts it\)/);
  assert.match(held.get(listings.find((l) => l.slug === "bitcoin-puppets")!.id)!.note, /no canister collection has this listing's root/);
  assert.match(held.get(listings.find((l) => l.slug === "nodemonkes")!.id)!.note, /no floor/);
  // frog_wake: OMB and Runestone (wrapping "later" in the file: the canister's `live` decides) are now read and posted under their own coll
  colls[1].live = true; colls[2].live = true; posted.length = 0; read.length = 0; now += 4_000;
  assert.equal(await frogFloorOnce({ buyer, log: silent, listings, read: reader, ref, nowS: () => now, last, held, accepted, live }), true);
  assert.deepEqual(posted.map((p) => p[0]), [4, 24, 28]);
  assert.deepEqual([...live].sort((a, b) => a - b), [4, 24, 28]);
  // the 20 % band: out of it, or no reference, the reading is held and nothing is handed in
  for (const [r, why] of [[2_000_000n, /outside the 20 % band/], [null, /no CoinGecko reference/]] as const) {
    refSats = r; posted.length = 0; now += 4_000;
    const lines: string[] = [];
    assert.equal(await frogFloorOnce({ buyer, log: logger((l) => lines.push(l)), listings, read: reader, ref, nowS: () => now, last, held }), false);
    assert.equal(posted.length, 0);
    assert.match(held.get(frogs.id)!.note, why);
    assert.ok(lines.some((l) => l.includes("frog-floor-out-of-band")));
  }
  // the canister refuses (paused, or the poster paused): logged, not counted
  refSats = 1_234_000n; now += 4_000; answer = { Err: "refused: the poster is paused" };
  const lines: string[] = [];
  assert.equal(await frogFloorOnce({ buyer, log: logger((l) => lines.push(l)), listings, read: reader, ref, nowS: () => now, last, held }), false);
  assert.ok(lines.some((l) => l.includes("frog-floor-not-taken")));
  assert.match(held.get(frogs.id)!.note, /refused/);
  // /healthz: a live collection with no accepted reading for 2 h is named
  assert.deepEqual(staleFloors([4, 24], new Map([[4, 10_000]]), 10_000 + 7_300, 0, 7_200).map((t) => t.match(/collection (\d+)/)![1]), ["4", "24"]);
  assert.deepEqual(staleFloors([4], new Map([[4, 10_000]]), 10_000 + 600, 0, 7_200), []);
});

test("the band gate: within 20 % of CoinGecko's floor either way, inclusive; CoinGecko's BTC floor read in sats, anything else no reference", async () => {
  assert.equal(inBand(120n, 100n).ok, true);
  assert.equal(inBand(80n, 100n).ok, true);
  assert.equal(inBand(121n, 100n).ok, false);
  assert.equal(inBand(79n, 100n).ok, false);
  assert.equal(inBand(100n, null).ok, false);
  const urls: string[] = [], heads: unknown[] = [];
  const f = (body: unknown, ok = true) => (async (u: string, init?: RequestInit) => { urls.push(String(u)); heads.push(init?.headers); return { ok, json: async () => body }; }) as unknown as typeof fetch;
  assert.equal(await coingeckoReader(f({ id: "bitcoin-frogs", native_currency_symbol: "btc", floor_price: { native_currency: 0.0123 } }))("bitcoin-frogs"), 1_230_000n);
  assert.equal(urls[0], "https://api.coingecko.com/api/v3/nfts/bitcoin-frogs");
  assert.equal(await coingeckoReader(f({ native_currency_symbol: "eth", floor_price: { native_currency: 1 } }))("x"), null);
  assert.equal(await coingeckoReader(f({}, false))("x"), null);
  await coingeckoReader(f({ floor_price: { native_currency: 1 } }), () => "demo")("x");
  assert.deepEqual(heads.at(-1), { "x-cg-demo-api-key": "demo" });
});

test("the runner's configuration on the Solana home (the default): /quote/v2, the listings, no pending cadence; HOME is read only when it names a home", () => {
  const ID = "ryjl3-tyaaa-aaaaa-aaaba-cai";
  const c = parseRunnerConfig({ CANISTER_ID: ID, SOL_RPC_URLS: "https://a.example, https://b.example/?api-key=k", RUNNER_PENDING_EVERY_S: "900" });
  assert.equal(c.home, "solana");
  assert.equal(c.relayQuotePath, "/quote/v2");
  assert.deepEqual(c.solRpcUrls, ["https://a.example", "https://b.example/?api-key=k"]);
  assert.equal(c.frogFloor, true);
  assert.equal(c.floorScope, "canister");
  assert.equal(c.listings[0].slug, "bitcoin-frogs");
  assert.equal(sleepAfter(c, true, 0), c.everyS, "no pending cadence on the Solana home");
  assert.equal(homeOf({ HOME: "/Users/someone" }), "solana", "the Unix home directory is not a chain");
  assert.equal(homeOf({ HOME: "robinhood" }), "robinhood");
  assert.equal(homeOf({ HOME: "solana", ROOK_HOME: "robinhood" }), "robinhood");
  const p = (env: Record<string, string>) => { try { parseRunnerConfig({ CANISTER_ID: ID, ...env }); return ""; } catch (e) { return (e as RunnerConfigError).problems.join("\n"); } };
  assert.match(p({ ROOK_HOME: "base" }), /ROOK_HOME=base/);
  assert.match(p({ SOL_RPC_URLS: "http://rpc.example" }), /not https/);
  assert.match(p({ RUNNER_FROG_FLOOR_EVERY_S: "7200" }), /RUNNER_FROG_FLOOR_EVERY_S/);
  assert.match(p({ RUNNER_FLOOR_LISTINGS: "some" }), /RUNNER_FLOOR_LISTINGS/);
  assert.match(p({ RUNNER_LAUNCH_FILE: "/nonexistent/nft-pilot.json" }), /RUNNER_LAUNCH_FILE/);
  assert.equal(parseRunnerConfig({ CANISTER_ID: ID, RUNNER_FROG_FLOOR: "off", RUNNER_FROG_PROOFS: "off", RUNNER_LAUNCH_FILE: "/nonexistent" }).frogFloor, false, "both off: the launch file is not needed");
  assert.equal(parseRunnerConfig({ CANISTER_ID: ID, ROOK_HOME: "robinhood" }).relayQuotePath, "/quote");
});

test("a frog's cap on the Solana home is in lamports: the rate is sats per 10^18 lamports, the lower of Coinbase and Kraken, refused when they disagree", async () => {
  const f = (cb: string, kr: string) => (async (url: string) => new Response(JSON.stringify(String(url).includes("coinbase") ? { data: { amount: cb } }
    : { result: { SOLXBT: { c: [kr, "1"] } } }))) as unknown as typeof fetch;
  assert.equal(await liveSatsPerSolHomeUnit(f("0.0020", "0.00201"))(), 200_000n * 1_000_000_000n);
  await assert.rejects(liveSatsPerSolHomeUnit(f("0.0020", "0.0030"))(), /disagrees/);
});

test("the Solana home's secrets are scrubbed from every log line: RELAY_API_KEY, UNISAT_API_KEY, a keyed SOL_RPC_URLS", () => {
  const env = { RELAY_API_KEY: "relay-secret-0001", UNISAT_API_KEY: "unisat-secret-0002", SOL_RPC_URLS: "https://mainnet.helius-rpc.com/?api-key=0123456789abcdef0123,https://solana-rpc.publicnode.com" };
  const lines: string[] = [];
  const log = logger((l) => lines.push(l), () => runnerSecrets(env));
  log("warn", "x", { a: "relay-secret-0001 unisat-secret-0002", url: "https://mainnet.helius-rpc.com/?api-key=0123456789abcdef0123" });
  assert.ok(!/secret-000|0123456789abcdef0123/.test(lines[0]), lines[0]);
});
