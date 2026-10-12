import { decodeFunctionData, encodeFunctionData, parseAbi, type Hex } from "viem";
import type { CapacityV, CollectionLifeV, EthStateV, Buyer, EthBuyV, EthNftConfigV } from "./ic.js";
import type { Log } from "./log.js";
import { orderIdOf, RELAY_API_DEFAULT, RELAY_DEPOSITORY_4663, DEPOSIT_NATIVE, ETH_NATIVE, relayHeaders, isSolLeg, type SolJudge } from "./relay.js";
import type { S3 } from "./fuel/sol.js";

/**
 * docs/v7/phase1-nft.md §6. The runner's half of the Ethereum NFT side: the canister says what it needs (`eth_nft_state()`), the
 * runner fetches it and hands it in; the canister judges everything again on its own reads and trusts nothing here.
 *   - a buy `Funded` with an ask: Relay's quote for exactly that (4663 → Ethereum, native ETH to the canister's own address),
 *     pre-judged by the canister's rules 1–9 and 11 (`relayeth::judge`), handed in with `eth_relay_quote`; a buy going BACK
 *     (`BackFunded`, audit 2026-09-27-nft round 2, A2-M-1) likewise, the other way (Ethereum → 4663);
 *   - a buy `Arrived`: the collection's cheapest OpenSea listings, their fulfillment data for the canister's address (OpenSea's
 *     key, read at call time and never logged), handed in cheapest first with `eth_listing`;
 *   - every EVIDENCE_EVERY_S per collection: up to 8 listings (encoded from OpenSea's protocol data as fulfillAdvancedOrder), with
 *     `eth_floor_evidence`. NO sales (round 2, A-M-1 / B2-M-1 / D2-M-2): the canister reads the collection's sales itself, so no
 *     runner, honest or not, chooses which sales count, and no stream of wash sales can crowd the real ones out of a hand-in;
 *   - holder deposits (D5, on at launch): every transaction that moved a configured collection's NFT TO the canister's Ethereum
 *     address in the last DEPOSIT_LOOKBACK_BLOCKS (Transfer logs, `to` = us) and that the canister has not judged, with
 *     `eth_deposit`. Discovery only: the canister reads each transaction and its receipt itself, mints a valid deposit past
 *     finality, and sends anything else of a configured collection back to its sender. The runner can only withhold (the
 *     monitor pages an inbound transfer the canister has not judged within two hours).
 */

export const OPENSEA_API = "https://api.opensea.io";
/**
 * 2026-10-12 (Robinhood Chain NFTs): how OpenSea's v2 API names a chain (`fulfillment_data`'s `listing.chain`, `/chain/{chain}/…` paths), by
 * chain id. Ethereum's is OpenSea's documented `ethereum`; Robinhood Chain's could not be read offline (OpenSea indexes the chain; its
 * API's identifier for it is NOT verified here): `robinhood` by default, overridden by the runner's `openseaChains` (env
 * OPENSEA_CHAIN_4663) once read from OpenSea's chain list.
 */
export const OPENSEA_CHAINS: Record<string, string> = { "1": "ethereum", "4663": "robinhood" };

/** OpenSea's name for `chainId` (the override first); a chain it has no name for throws (nothing is fetched on a guess). */
export function openseaChainOf(chainId: number | bigint, override?: Record<string, string>): string {
  const k = String(chainId), n = override?.[k] ?? OPENSEA_CHAINS[k];
  if (!n) throw new Error(`no OpenSea chain name for chain ${k}`);
  return n;
}

/** The chain a configured collection lives on: its own `chain_id` (2026-10-12; the canister's `EthCollection.chain_id`, an `opt`), else the config's. */
export function chainOfCollection(cfg: Pick<EthNftConfigV, "chain_id" | "collections">, collection: string): number {
  const c = cfg.collections.find((x) => lc(x.collection) === lc(collection)) as ({ chain_id?: [] | [bigint] | bigint } | undefined);
  const own = Array.isArray(c?.chain_id) ? c!.chain_id[0] : c?.chain_id;
  return Number(own ?? cfg.chain_id);
}
export const SEAPORT_1_6 = "0x0000000000000068f116a894984e2db1123eb395";
export const TRANSFER_TOPIC = "0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef";
/** How far back the runner looks for NFTs sent to the canister: a day of Ethereum blocks, in chunks an RPC serves. */
export const DEPOSIT_LOOKBACK_BLOCKS = 7_200;
/** The widest eth_getLogs range asked: the operator's Ethereum provider (Dwellir's plan, 2026-09-27) serves at most 500 blocks a
 * request. ETH_LOGS_MAX_BLOCKS sets another. */
export const DEPOSIT_CHUNK_BLOCKS = 500;
export const CLAIM_AGAIN_S = 7 * 3_600;
const DAY = 86_400;
/** D-L-1: every outbound call has a deadline and a size cap. */
export const FETCH_TIMEOUT_MS = 15_000;
export const MAX_BODY_BYTES = 4_000_000;
/** D-M-4: a listing handed in for a buy is not handed in again for this long (the canister judged it at the tick that followed). */
export const LISTING_RETRY_S = 2 * 3_600;
export const ORDER_FULFILLED = "0x9d9af8e38d66c62e2c12f0225249fd9d721c54b83f48d9352c97c6cacdcb6f31";
export const SEAPORT_1_5 = "0x00000000000000adc04c56bf30ac9d3c0aaf14dc";
export const BLUR_EXCHANGE_V2 = "0xb2ecfe4e4d61f8790bbb9de2d1259b9e2410cea5";
export const BLUR_721 = ["0x1d5e12b51dee5e4d34434576c3fb99714a85f57b0fd546ada4b0bddd736d12b2", "0x0fcf17fac114131b10f37b183c6a60f905911e52802caeeb3e6ea210398b81ab",
  "0x7dc5c0699ac8dd5250cbe368a2fc3b4a2daadb120ad07f6cccea29f83482686e"];

type AnyObj = Record<string, any>; // eslint-disable-line @typescript-eslint/no-explicit-any

// ---- Relay: the pre-judge of relayeth::judge (4663 → Ethereum) ---------------------------------------------------------------------

export type EthAsk = { buy: bigint; amountWei: bigint; user: string; recipient: string; refundTo: string; originChainId: number; destinationChainId: number;
  /** docs/v9 §2.8: on the Solana home the forward leg is SOL → ETH and the way back ETH → SOL (the ask names its currencies) */
  originCurrency?: string; destinationCurrency?: string };

export function ethAskOf(b: EthBuyV): EthAsk | null {
  const a = b.ask[0];
  if (!a || !("Funded" in b.phase || "BackFunded" in b.phase)) return null;
  return { buy: b.id, amountWei: a.amount_wei, user: a.user, recipient: a.recipient, refundTo: a.refund_to, originChainId: Number(a.origin_chain_id),
    destinationChainId: Number(a.destination_chain_id), originCurrency: a.origin_currency ?? ETH_NATIVE, destinationCurrency: a.destination_currency ?? ETH_NATIVE };
}

export function ethQuoteBody(a: EthAsk): Record<string, unknown> {
  return { user: a.user, originChainId: a.originChainId, destinationChainId: a.destinationChainId, originCurrency: a.originCurrency ?? ETH_NATIVE,
    destinationCurrency: a.destinationCurrency ?? ETH_NATIVE, recipient: a.recipient, tradeType: "EXACT_INPUT", amount: a.amountWei.toString(), refundTo: a.refundTo };
}

const lc = (s: unknown) => String(s ?? "").toLowerCase();

/** The two legs, as `relayeth::TO_ETHEREUM` / `FROM_ETHEREUM` name them (the depository is the same address on both chains). */
const LEGS: Record<string, { origin: string; destination: string }> = { "4663>1": { origin: "robinhood", destination: "ethereum" },
  "1>4663": { origin: "ethereum", destination: "robinhood" } };

/** Rules 1–9 and 11 (and 10 against the install's bound) of `relayeth::judge`, for the leg the ask names. Throws the first refusal. */
export function preJudgeEth(q: AnyObj, a: EthAsk, nowS: number, maxFeeBps = 300, sol: { judge?: SolJudge; s3?: S3 | null } = {}): { orderId: Hex; minOut: bigint } {
  const need = (c: boolean, why: string) => { if (!c) throw new Error(why); };
  // docs/v9 §2.8, the Solana home: SOL → ETH (relaysol.rs judge_sol, S1–S12) or ETH → SOL (relayeth.rs judge_to_solana), by the judge the
  // runner hands in (src/relaysol.ts); rule 10 is the canister's there (its SOL/ETH rate): the runner holds none
  if (isSolLeg(a)) {
    need(!!sol.judge, "a Solana leg, and no Solana judge is configured (ROOK_HOME=robinhood?)");
    return sol.judge!(q, { ...a, originCurrency: a.originCurrency ?? ETH_NATIVE, destinationCurrency: a.destinationCurrency ?? ETH_NATIVE }, nowS, sol.s3 ?? null);
  }
  const leg = LEGS[`${a.originChainId}>${a.destinationChainId}`];
  need(!!leg, `no Relay leg from chain ${a.originChainId} to ${a.destinationChainId}`);
  const steps = q?.steps;
  need(Array.isArray(steps) && steps.length === 1 && steps[0].id === "deposit" && steps[0].kind === "transaction", "not a single deposit transaction");
  need(Array.isArray(steps[0].items) && steps[0].items.length === 1, "exactly one transaction is signed");
  const d = steps[0].items[0]?.data ?? {};
  need(Number(d.chainId) === a.originChainId, `the deposit is not on chain ${a.originChainId}`);
  need(lc(d.to) === RELAY_DEPOSITORY_4663, `the deposit goes to ${d.to}, not Relay's pinned depository`);
  if (d.from !== undefined) need(lc(d.from) === lc(a.user), "the quote was made for another sender");
  const value = BigInt(String(d.value));
  need(value === a.amountWei, `the deposit sends ${value}, not ${a.amountWei}`);
  const data = lc(d.data);
  need(data.length === 2 + 68 * 2 && data.startsWith(DEPOSIT_NATIVE) && `0x${data.slice(34, 74)}` === lc(a.user), "the calldata is not depositNative(us, id)");
  const id = `0x${data.slice(74)}` as Hex;
  const p = q?.protocol?.v2;
  need(!!p?.orderData, "no protocol.v2.orderData");
  const o = p.orderData;
  let computed: Hex;
  try { computed = orderIdOf(o); } catch (e) { throw new Error(`the order cannot be hashed: ${(e as Error).message}`); }
  need(computed === id && lc(p.orderId) === id, `the order hashes to ${computed}, the calldata's id is ${id}, the quote says ${p.orderId}`);
  need(o.inputs?.length === 1 && o.inputs[0].payment.chainId === leg.origin && lc(o.inputs[0].payment.currency) === ETH_NATIVE
    && BigInt(o.inputs[0].payment.amount) === value, `the input is not native ETH on ${leg.origin} for the value`);
  const refunds = o.inputs[0].refunds ?? [];
  need(refunds.length > 0 && refunds.every((r: AnyObj) => ["robinhood", "ethereum"].includes(r.chainId) && lc(r.recipient) === lc(a.refundTo)
    && lc(r.currency) === ETH_NATIVE), "a refund is not ours in native ETH");
  need(refunds.some((r: AnyObj) => r.chainId === leg.origin), `no refund on ${leg.origin}`);
  const out = o.output;
  need(out.chainId === leg.destination, `the output is not on ${leg.destination}`);
  need(out.payments?.length === 1 && lc(out.payments[0].recipient) === lc(a.recipient) && lc(out.payments[0].currency) === ETH_NATIVE,
    "the output does not pay our own address in native ETH");
  need((out.calls ?? []).length === 0 && (o.fees ?? []).length === 0, "the order carries calls or fees");
  const minOut = BigInt(out.payments[0].minimumAmount);
  need(minOut * 10_000n >= value * BigInt(10_000 - maxFeeBps), `the minimum out ${minOut} is under ${10_000 - maxFeeBps} bps of ${value}`);
  const band = (t: number) => t >= nowS + DAY && t <= nowS + 8 * DAY;
  need(band(Number(out.deadline)) && refunds.every((r: AnyObj) => band(Number(r.deadline))), "a deadline is not between a day and eight days away");
  return { orderId: id, minOut };
}

// ---- Seaport calldata from OpenSea's answers -------------------------------------------------------------------------------------

export const SEAPORT_ABI = parseAbi([
  "struct AdditionalRecipient { uint256 amount; address recipient; }",
  "struct BasicOrderParameters { address considerationToken; uint256 considerationIdentifier; uint256 considerationAmount; address offerer; address zone; address offerToken; uint256 offerIdentifier; uint256 offerAmount; uint8 basicOrderType; uint256 startTime; uint256 endTime; bytes32 zoneHash; uint256 salt; bytes32 offererConduitKey; bytes32 fulfillerConduitKey; uint256 totalOriginalAdditionalRecipients; AdditionalRecipient[] additionalRecipients; bytes signature; }",
  "struct OfferItem { uint8 itemType; address token; uint256 identifierOrCriteria; uint256 startAmount; uint256 endAmount; }",
  "struct ConsiderationItem { uint8 itemType; address token; uint256 identifierOrCriteria; uint256 startAmount; uint256 endAmount; address recipient; }",
  "struct OrderParameters { address offerer; address zone; OfferItem[] offer; ConsiderationItem[] consideration; uint8 orderType; uint256 startTime; uint256 endTime; bytes32 zoneHash; uint256 salt; bytes32 conduitKey; uint256 totalOriginalConsiderationItems; }",
  "struct AdvancedOrder { OrderParameters parameters; uint120 numerator; uint120 denominator; bytes signature; bytes extraData; }",
  "struct CriteriaResolver { uint256 orderIndex; uint8 side; uint256 index; uint256 identifier; bytes32[] criteriaProof; }",
  "function fulfillBasicOrder(BasicOrderParameters parameters) payable returns (bool)",
  "function fulfillAdvancedOrder(AdvancedOrder advancedOrder, CriteriaResolver[] criteriaResolvers, bytes32 fulfillerConduitKey, address recipient) payable returns (bool)",
]);

const ZERO32 = `0x${"0".repeat(64)}` as Hex;

/** OpenSea's `protocol_data` (OrderComponents parameters + signature) as `fulfillAdvancedOrder` calldata: floor evidence only. */
export function advancedFromProtocolData(pd: AnyObj, recipient: string, extra: { numerator?: bigint; denominator?: bigint; extraData?: Hex } = {}): Hex {
  const p = pd.parameters;
  return encodeFunctionData({ abi: SEAPORT_ABI, functionName: "fulfillAdvancedOrder", args: [{
    parameters: {
      offerer: p.offerer, zone: p.zone,
      offer: p.offer.map((i: AnyObj) => ({ itemType: Number(i.itemType), token: i.token, identifierOrCriteria: BigInt(i.identifierOrCriteria),
        startAmount: BigInt(i.startAmount), endAmount: BigInt(i.endAmount) })),
      consideration: p.consideration.map((i: AnyObj) => ({ itemType: Number(i.itemType), token: i.token, identifierOrCriteria: BigInt(i.identifierOrCriteria),
        startAmount: BigInt(i.startAmount), endAmount: BigInt(i.endAmount), recipient: i.recipient })),
      orderType: Number(p.orderType), startTime: BigInt(p.startTime), endTime: BigInt(p.endTime), zoneHash: p.zoneHash, salt: BigInt(p.salt),
      conduitKey: p.conduitKey, totalOriginalConsiderationItems: BigInt(p.totalOriginalConsiderationItems ?? p.consideration.length),
    },
    numerator: extra.numerator ?? 1n, denominator: extra.denominator ?? 1n, signature: (pd.signature ?? "0x") as Hex, extraData: extra.extraData ?? "0x",
  }, [], ZERO32, recipient as Hex] });
}

/** OpenSea's `fulfillment_data.transaction` (for our address) as calldata: a basic order, or an advanced one with the zone's data. */
export function calldataFromFulfillment(tx: AnyObj): Hex {
  const f = String(tx?.function ?? "");
  const input = tx?.input_data ?? {};
  if (lc(tx?.to) !== SEAPORT_1_6) throw new Error(`the fulfillment goes to ${tx?.to}, not Seaport 1.6`);
  if (f.startsWith("fulfillBasicOrder")) {
    const p = input.parameters;
    return encodeFunctionData({ abi: SEAPORT_ABI, functionName: "fulfillBasicOrder", args: [{
      considerationToken: p.considerationToken, considerationIdentifier: BigInt(p.considerationIdentifier), considerationAmount: BigInt(p.considerationAmount),
      offerer: p.offerer, zone: p.zone, offerToken: p.offerToken, offerIdentifier: BigInt(p.offerIdentifier), offerAmount: BigInt(p.offerAmount),
      basicOrderType: Number(p.basicOrderType), startTime: BigInt(p.startTime), endTime: BigInt(p.endTime), zoneHash: p.zoneHash, salt: BigInt(p.salt),
      offererConduitKey: p.offererConduitKey, fulfillerConduitKey: ZERO32, totalOriginalAdditionalRecipients: BigInt(p.totalOriginalAdditionalRecipients),
      additionalRecipients: (p.additionalRecipients ?? []).map((r: AnyObj) => ({ amount: BigInt(r.amount), recipient: r.recipient })),
      signature: p.signature,
    }] });
  }
  if (f.startsWith("fulfillAdvancedOrder")) {
    const ao = input.advancedOrder;
    // the zone's extraData (a restricted order's SignedZone data) is part of the advanced order
    return advancedFromProtocolData({ parameters: ao.parameters, signature: ao.signature }, input.recipient,
      { numerator: BigInt(ao.numerator), denominator: BigInt(ao.denominator), extraData: (ao.extraData ?? "0x") as Hex });
  }
  throw new Error(`the fulfillment is ${f}, not fulfillBasicOrder or fulfillAdvancedOrder`);
}

/** The advanced order's parameters back from calldata (tests and the monitor's read of what was handed in). */
export function decodeAdvanced(cd: Hex): AnyObj {
  const d = decodeFunctionData({ abi: SEAPORT_ABI, data: cd });
  return { advancedOrder: d.args[0], recipient: d.args[3] };
}

// ---- the pass ---------------------------------------------------------------------------------------------------------------------

export type EthNftDeps = {
  buyer: Pick<Buyer, "eth_nft_state" | "eth_nft_config" | "eth_relay_quote" | "eth_listing" | "eth_floor_evidence" | "evm_address" | "eth_deposit">
    & Partial<Pick<Buyer, "eth_nft_collections" | "eth_nft_capacity">>;
  fetch: typeof fetch;
  log: Log;
  /** collection (lower-case) → OpenSea slug */
  slugs: Record<string, string>;
  openseaKey: () => string | undefined;
  /** an Ethereum RPC that serves eth_getLogs (the operator's Dwellir URL, its key in ETH_LOGS_RPC's value only: never logged) */
  ethRpc: string | null;
  /** the widest eth_getLogs range it serves (DEPOSIT_CHUNK_BLOCKS by default) */
  logsMaxBlocks?: number;
  relayApi?: string;
  /** 2026-10-12: OpenSea's chain names by chain id, over `OPENSEA_CHAINS` (Robinhood Chain's from OPENSEA_CHAIN_4663) */
  openseaChains?: Record<string, string>;
  /** 2026-10-12: the other chains' sides of the canister (`eth_nft_state_of(chain)` for each chain the collections live on beside the
   * config's own): their buys are served like the config's own side's (a buy id names its side in the canister) */
  sideStates?: () => Promise<EthStateV[]>;
  /** docs/v9 §5: the quote path (`/quote` by default; `/quote/v2` on the Solana home) and Relay's optional API key, read at call time */
  relayQuotePath?: string;
  relayKey?: () => string | undefined;
  /** the Solana home: its pre-judge (src/relaysol.ts), and rule S3's reads for a SOL → ETH quote, or null when not read */
  solJudge?: SolJudge;
  s3?: () => Promise<S3 | null>;
  nowS?: () => number;
  evidenceEveryS?: number;
  lastEvidence?: Map<string, number>;
  /** claims handed in, and when (seconds): not handed in again for CLAIM_AGAIN_S (the canister drops one it cannot read after 6 h) */
  claimed?: Map<string, number>;
  /** D-M-4: the listings handed in per buy (order hash → when) */
  tried?: Map<string, Map<string, number>>;
  /** D-L-2: per collection, when its evidence was last accepted (the runner's /healthz reads it) */
  evidenceOk?: Map<string, number>;
  /** round 2, D2-L-3: what keeps the step from working, and the collections the canister serves, as of the last pass */
  problems?: string[];
  served?: string[];
  /** docs/v7/dormant-collections.md: each collection's life as of the last pass (the runner's GET /collections serves it) */
  lives?: CollectionLifeV[];
  /** docs/v7/dormant-collections.md §11: the canister's capacity on its cycles balance as of the last pass (the runner's GET /capacity) */
  capacity?: { current: CapacityV | null };
  timeoutMs?: number;
};

/** docs/v7/dormant-collections.md §4: a collection the canister keeps DORMANT (or queued for a wake) gets no evidence (its evidence is
 * refused unread); a WAKING one gets it every pass (its backfill runs one pass per hand-in); an ACTIVE one near a buy (an open buy of
 * it: the fill needs a fresh floor and sale read) every pass too; the rest every `evidenceEveryS`. Without the canister's lives (a build
 * before the dormancy) every collection is served as before. */
export function evidenceCadence(lives: CollectionLifeV[] | null, coll: string, openBuy: boolean): "skip" | "every-pass" | "normal" {
  const st = lives?.find((x) => x.collection.toLowerCase() === coll)?.state ?? "always-on";
  if (st === "dormant" || st === "queued") return "skip";
  if (st === "waking" || openBuy) return "every-pass";
  return "normal";
}

/** Whether the Ethereum NFT side is idle: every collection the canister serves is asleep, and nothing of it is open (a buy, a delivery, a
 * 4663 transaction). The runner then ticks every RUNNER_IDLE_TICK_EVERY_S (a tick still reads the wake desk). */
export function ethIdle(lives: CollectionLifeV[] | null, state: Pick<EthStateV, "buys" | "deliveries" | "pending"> | null): boolean {
  if (!lives || !lives.length || !state) return false;
  if (!lives.every((l) => l.state === "dormant" || l.state === "queued")) return false;
  const open = (p: Record<string, unknown>) => !["Settled", "Returned"].includes(Object.keys(p ?? {})[0] ?? "");
  const done = (p: Record<string, unknown>) => ["Recorded", "Marked", "Gone"].includes(Object.keys(p ?? {})[0] ?? "");
  return !state.buys.some((b) => open(b.phase)) && state.deliveries.every((x) => done(x.phase)) && !(state.pending?.[0]);
}

/** D-L-1: one outbound call with a deadline, a size cap and no redirect (a key in a custom header never follows one: D-I-4). */
export async function guarded(d: Pick<EthNftDeps, "fetch" | "timeoutMs">, url: string, init: RequestInit = {}): Promise<{ ok: boolean; status: number; text: string }> {
  const res = await d.fetch(url, { ...init, redirect: "manual", signal: AbortSignal.timeout(d.timeoutMs ?? FETCH_TIMEOUT_MS) });
  const text = await res.text();
  if (text.length > MAX_BODY_BYTES) throw new Error(`${new URL(url).host}: the answer is over ${MAX_BODY_BYTES} bytes`);
  return { ok: res.ok, status: res.status, text };
}

async function os(d: EthNftDeps, path: string, init?: RequestInit): Promise<AnyObj> {
  const key = d.openseaKey();
  if (!key) throw new Error("OPENSEA_API_KEY is not set: listings and fulfillment data are keyed");
  const res = await guarded(d, `${OPENSEA_API}${path}`, { ...init, headers: { ...(init?.headers ?? {}), "x-api-key": key, accept: "application/json",
    "content-type": "application/json" } });
  if (!res.ok) throw new Error(`OpenSea ${path.split("?")[0]}: HTTP ${res.status}`);
  return JSON.parse(res.text);
}

/** The collection's cheapest listings, with their order hash and protocol data. */
export async function bestListings(d: EthNftDeps, slug: string, limit: number): Promise<AnyObj[]> {
  const j = await os(d, `/api/v2/listings/collection/${encodeURIComponent(slug)}/best?limit=${limit}`);
  return (j.listings ?? []).filter((l: AnyObj) => l?.protocol_data?.parameters && lc(l.protocol_address) === SEAPORT_1_6);
}

/**
 * The floor evidence for one collection: its cheapest listings as Seaport calldata for `us`. 2026-10-12 (found by the devnet rehearsal):
 * OpenSea's best-listings answer carries each order's parameters but an EMPTY signature ("0x"), and the canister refuses an unsigned
 * order ("the signature is not an EOA's or a bulk order's recovering to the offerer"), so no floor would ever post. A listing without
 * a signature is fetched again through `fulfillment_data`, which carries it (and a restricted order's zone data), as the buy step does.
 */
export async function evidenceListings(d: EthNftDeps, slug: string, chain: string, us: string, limit = 8): Promise<Hex[]> {
  const out: Hex[] = [];
  for (const l of await bestListings(d, slug, limit)) {
    const sig = String(l?.protocol_data?.signature ?? "0x");
    if (sig.length > 2) { out.push(advancedFromProtocolData(l.protocol_data, us)); continue; }
    try {
      const f = await os(d, "/api/v2/listings/fulfillment_data", { method: "POST", body: JSON.stringify({ listing: { hash: l.order_hash, chain,
        protocol_address: SEAPORT_1_6 }, fulfiller: { address: us } }) });
      out.push(calldataFromFulfillment(f?.fulfillment_data?.transaction));
    } catch (err) { d.log("warn", "eth-evidence-listing-unsigned", { slug, hash: l.order_hash, error: (err as Error).message.slice(0, 200) }); }
  }
  return out;
}

async function ethCall(d: EthNftDeps, method: string, params: unknown[]): Promise<any> { // eslint-disable-line @typescript-eslint/no-explicit-any
  const r = await guarded(d, d.ethRpc!, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ jsonrpc: "2.0", id: 1, method, params }) });
  const j = JSON.parse(r.text) as AnyObj;
  if (j.error) throw new Error(`${method}: ${JSON.stringify(j.error).slice(0, 200)}`);
  return j.result;
}

/** Every transaction in the last `span` blocks that moved a token of `collection` TO `us` (ERC-721 Transfer, topic 2 = us), oldest first. */
export async function inboundTransferTxs(d: EthNftDeps, collection: string, us: string, span = DEPOSIT_LOOKBACK_BLOCKS): Promise<string[]> {
  if (!d.ethRpc) return [];
  const head = Number(BigInt(await ethCall(d, "eth_blockNumber", [])));
  const to = `0x${"0".repeat(24)}${us.toLowerCase().replace(/^0x/, "")}`;
  const out: string[] = [];
  const chunk = d.logsMaxBlocks ?? DEPOSIT_CHUNK_BLOCKS;
  for (let from = Math.max(0, head - span); from <= head; from += chunk) {
    const end = Math.min(head, from + chunk - 1);
    const logs = await ethCall(d, "eth_getLogs", [{ address: collection, topics: [TRANSFER_TOPIC, null, to], fromBlock: `0x${from.toString(16)}`,
      toBlock: `0x${end.toString(16)}` }]) as AnyObj[];
    // ERC-721 only (four topics: the token id is indexed); an ERC-20 Transfer to us is not an NFT
    for (const l of logs) if ((l.topics ?? []).length === 4 && !out.includes(lc(l.transactionHash))) out.push(lc(l.transactionHash));
  }
  return out;
}

const hex = (b: Uint8Array | number[]) => `0x${Buffer.from(b).toString("hex")}`;

/** The transactions of ours that brought NFTs in: the buys' fills (never a claim). */
function ourFills(state: { buys: EthBuyV[] }): Set<string> {
  const s = new Set<string>();
  for (const b of state.buys) { const body = Object.values(b.phase ?? {})[0] as AnyObj | null; if (body?.tx) s.add(hex(body.tx)); }
  return s;
}

/** One pass. True when anything was handed in (the runner then ticks again at once). */
export async function ethNftOnce(d: EthNftDeps): Promise<boolean> {
  const now = d.nowS ?? (() => Math.floor(Date.now() / 1000));
  let cfg: EthNftConfigV | null;
  try { cfg = (await d.buyer.eth_nft_config())[0] ?? null; } catch (err) { d.log("warn", "eth-unread", { error: String((err as Error).message).slice(0, 200) }); return false; }
  if (!cfg) { d.problems?.splice(0, d.problems.length, "the canister's eth_nft_config() is null: nothing is served"); return false; }
  // round 2, D2-L-3: what the step needs is checked against what the canister serves on EVERY pass (a restart that met a blip, or an
  // upgrade that added a collection, is caught here, not only at start); /healthz is 503 while any is open. Each part below still
  // skips what it lacks (a collection with no slug gets no evidence and no listing), the rest runs
  const problems = ethStepProblems(cfg, { ethLogsRpc: d.ethRpc ?? null, slugs: d.slugs, openseaKey: d.openseaKey() });
  d.problems?.splice(0, d.problems.length, ...problems);
  // docs/v7/dormant-collections.md: which collections are awake; a dormant one's evidence staleness never makes /healthz unhealthy
  let lives: CollectionLifeV[] | null = null;
  if (d.buyer.eth_nft_collections) {
    try { lives = await d.buyer.eth_nft_collections(); } catch (err) { d.log("warn", "eth-lives-unread", { error: String((err as Error).message).slice(0, 200) }); }
  }
  if (lives) d.lives?.splice(0, d.lives.length, ...lives);
  if (d.capacity && d.buyer.eth_nft_capacity) {
    try { d.capacity.current = (await d.buyer.eth_nft_capacity())[0] ?? null; } catch (err) { d.log("warn", "eth-capacity-unread", { error: String((err as Error).message).slice(0, 200) }); }
  }
  d.served?.splice(0, d.served.length, ...cfg.collections.map((c) => lc(c.collection)).filter((c) => evidenceCadence(lives, c, false) !== "skip"));
  for (const p of problems) d.log("error", "eth-half-configured", { problem: p });
  const own = await d.buyer.eth_nft_state();
  // 2026-10-12: the other chains' sides (Robinhood Chain's collections): their buys and readings are served the same way
  let sides: EthStateV[] = [];
  if (d.sideStates) {
    try { sides = await d.sideStates(); } catch (err) { d.log("warn", "eth-sides-unread", { error: String((err as Error).message).slice(0, 200) }); }
  }
  const state = { ...own, buys: [...own.buys, ...sides.flatMap((s) => s.buys)], readings: [...(own.readings ?? []), ...sides.flatMap((s) => s.readings ?? [])],
    deposits: [...(own.deposits ?? []), ...sides.flatMap((s) => s.deposits ?? [])] };
  let submitted = false;
  const ea = await d.buyer.evm_address();
  if (!("Ok" in ea)) { d.log("warn", "eth-no-address", { error: ea.Err }); return false; }
  const us = ea.Ok;
  for (const b of state.buys) {
    // Relay
    const a = ethAskOf(b);
    if (a) {
      try {
        const res = await guarded(d, `${(d.relayApi ?? RELAY_API_DEFAULT).replace(/\/$/, "")}${d.relayQuotePath ?? "/quote"}`, { method: "POST",
          headers: relayHeaders(d.relayKey?.()), body: JSON.stringify(ethQuoteBody(a)) });
        const text = res.text;
        if (!res.ok) { d.log("warn", "eth-relay-refused", { buy: a.buy, status: res.status, body: text.slice(0, 200) }); continue; }
        const sol = isSolLeg(a);
        const j = preJudgeEth(JSON.parse(text), a, now(), cfg.relay_max_fee_bps, { judge: d.solJudge, s3: sol && d.s3 ? await d.s3() : null });
        const r = await d.buyer.eth_relay_quote(a.buy, text);
        if ("Ok" in r) { submitted = true; d.log("info", "eth-relay-handed-in", { buy: a.buy, orderId: j.orderId, minOut: j.minOut }); }
        else d.log("warn", "eth-relay-not-taken", { buy: a.buy, error: r.Err });
      } catch (err) { d.log("warn", "eth-relay-rejected", { buy: a.buy, why: (err as Error).message.slice(0, 300) }); }
      continue;
    }
    // a listing for an arrived buy: the cheapest not handed in yet (D-M-4: the canister judges a hand-in at the tick that follows; a
    // buy still Arrived after it means the canister refused it, so the next pass tries the next one), and only one the canister's
    // own bound could take (the runner's pre-check, docs §6: what arrived less the gas reserve, and our reading plus the premium)
    if ("Arrived" in b.phase) {
      const coll = `0x${Buffer.from(b.collection).toString("hex")}`;
      const slug = d.slugs[coll];
      if (!slug) { d.log("error", "eth-no-slug", { collection: coll }); continue; }
      const tried = d.tried ?? new Map<string, Map<string, number>>();
      const mine = tried.get(String(b.id)) ?? new Map<string, number>();
      tried.set(String(b.id), mine);
      const arrived = BigInt((b.phase as AnyObj).Arrived.wei);
      const reading = (state.readings ?? []).find((x) => lc(`0x${Buffer.from(x.collection).toString("hex")}`) === coll);
      const premium = BigInt(cfg.collections.find((x) => lc(x.collection) === coll)?.max_premium_bps ?? 0);
      const cap = [arrived - 3_000_000_000_000_000n, reading ? (BigInt(reading.post.price) * (10_000n + premium)) / 10_000n : arrived]
        .reduce((a, c) => (a < c ? a : c));
      try {
        const ls = await bestListings(d, slug, 8);
        for (const l of ls) {
          if ((mine.get(lc(l.order_hash)) ?? -Infinity) + LISTING_RETRY_S > now()) continue;
          const price = (() => { try { return BigInt(l?.price?.current?.value ?? "0"); } catch { return 0n; } })();
          if (price > cap) { d.log("info", "eth-listing-over-bound", { buy: b.id, hash: l.order_hash, price, cap }); continue; }
          // 2026-10-12: the listing's chain is the collection's (Robinhood Chain's own collections are on 4663's Seaport 1.6)
          const chain = openseaChainOf(chainOfCollection(cfg, coll), d.openseaChains);
          const f = await os(d, "/api/v2/listings/fulfillment_data", { method: "POST", body: JSON.stringify({ listing: { hash: l.order_hash, chain,
            protocol_address: SEAPORT_1_6 }, fulfiller: { address: us } }) });
          const cd = calldataFromFulfillment(f?.fulfillment_data?.transaction);
          const r = await d.buyer.eth_listing(b.id, cd);
          mine.set(lc(l.order_hash), now());
          if ("Ok" in r) { submitted = true; d.log("info", "eth-listing-handed-in", { buy: b.id, hash: l.order_hash, price: l?.price?.current?.value }); break; }
          d.log("warn", "eth-listing-not-taken", { buy: b.id, error: r.Err });
        }
      } catch (err) { d.log("warn", "eth-listing-failed", { buy: b.id, error: (err as Error).message.slice(0, 300) }); }
    }
  }
  // floor evidence, per collection, every evidenceEveryS; dormant ones none, waking ones and those near a buy every pass
  const every = d.evidenceEveryS ?? 1_200;
  const last = d.lastEvidence ?? new Map<string, number>();
  const buying = new Set(state.buys.filter((b) => !["Settled", "Returned"].includes(Object.keys(b.phase ?? {})[0] ?? "")).map((b) => lc(`0x${Buffer.from(b.collection).toString("hex")}`)));
  for (const c of cfg.collections) {
    const coll = lc(c.collection);
    const cadence = evidenceCadence(lives, coll, buying.has(coll));
    if (cadence === "skip") continue;
    if (cadence === "normal" && (last.get(coll) ?? 0) + every > now()) continue;
    const slug = d.slugs[coll];
    if (!slug) { d.log("error", "eth-no-slug", { collection: coll }); continue; }
    try {
      const listings = await evidenceListings(d, slug, openseaChainOf(chainOfCollection(cfg, coll), d.openseaChains), us);
      // round 2: no sale hashes; the canister reads the collection's sales itself
      const r = await d.buyer.eth_floor_evidence(coll, listings, []);
      last.set(coll, now());
      if ("Ok" in r) { submitted = true; d.evidenceOk?.set(coll, now()); d.log("info", "eth-evidence-handed-in", { collection: coll, listings: listings.length }); }
      else d.log("warn", "eth-evidence-not-taken", { collection: coll, error: r.Err });
    } catch (err) { d.log("warn", "eth-evidence-failed", { collection: coll, error: (err as Error).message.slice(0, 300) }); }
  }
  // holder deposits (D5): hand in every inbound transfer the canister has not judged
  if (cfg.deposits && d.ethRpc) {
    const judged = new Set((state.deposits ?? []).map((x) => hex(x.tx)));
    const ours = ourFills(state);
    const claimed = d.claimed ?? new Map<string, number>();
    for (const c of cfg.collections) {
      let txs: string[];
      try { txs = await inboundTransferTxs(d, lc(c.collection), us); }
      catch (err) { d.log("warn", "eth-deposit-scan-failed", { collection: lc(c.collection), error: (err as Error).message.slice(0, 300) }); continue; }
      for (const tx of txs) {
        if (judged.has(tx) || ours.has(tx) || (claimed.get(tx) ?? -Infinity) + CLAIM_AGAIN_S > now()) continue;
        // D-L-1: one rejected call (the canister mid-tick) never aborts the other claims, or the other collections'
        try {
          const r = await d.buyer.eth_deposit(tx);
          if ("Ok" in r) { submitted = true; claimed.set(tx, now()); d.log("info", "eth-deposit-handed-in", { collection: lc(c.collection), tx }); }
          else if (/already claimed/.test(r.Err)) claimed.set(tx, now());
          else d.log("warn", "eth-deposit-not-taken", { tx, error: r.Err });
        } catch (err) { d.log("warn", "eth-deposit-rejected", { tx, error: String((err as Error).message).slice(0, 200) }); }
      }
    }
  }
  return submitted;
}

/**
 * audit 2026-09-27-nft D-L-2: what the Ethereum step needs before it runs, against what the canister serves (`eth_nft_config()`): an
 * Ethereum RPC for logs (sales and deposit discovery), the OpenSea key, and a slug for every collection the canister serves. The
 * runner refuses to start without them; nothing is handed in half-configured.
 */
export function ethStepProblems(cfg: EthNftConfigV, env: { ethLogsRpc: string | null; slugs: Record<string, string>; openseaKey: string | undefined }): string[] {
  const p: string[] = [];
  if (!env.ethLogsRpc && cfg.deposits) p.push("ETH_LOGS_RPC is not set: with deposits on, no deposit is ever discovered");
  if (!env.openseaKey) p.push("OPENSEA_API_KEY is not set: no listing and no fulfillment data can be read");
  for (const c of cfg.collections) if (!env.slugs[lc(c.collection)]) p.push(`ETH_NFT_SLUGS names no OpenSea slug for ${lc(c.collection)}, which the canister serves`);
  return p;
}

/** `ETH_NFT_SLUGS=0x5af0…=milady,0xed5a…=azuki` → a map by lower-case collection. */
export function parseSlugs(s: string | undefined): Record<string, string> {
  const out: Record<string, string> = {};
  for (const part of (s ?? "").split(",").map((x) => x.trim()).filter(Boolean)) {
    const [k, v] = part.split("=");
    if (!/^0x[0-9a-fA-F]{40}$/.test(k ?? "") || !/^[a-z0-9-]+$/.test(v ?? "")) throw new Error(`ETH_NFT_SLUGS entry "${part}" is not 0x<collection>=<opensea slug>`);
    out[k.toLowerCase()] = v;
  }
  return out;
}
