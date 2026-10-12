import { encodeFunctionData, parseAbi, type Hex } from "viem";
import { withBackoff } from "./backoff.js";
import type { Fetch } from "./chain.js";
import type { Logger } from "./log.js";

/**
 * docs/v4/eth-nft-canister.md §12.2. For a listing the keeper chose, ask OpenSea for its fulfillment data — for a SignedZone
 * listing that is the order plus the zone's `extraData` signature, issued for ONE fulfiller: the canister's address on that
 * chain — and turn it into the calldata the canister is handed today (`nft_preview` / `nft_sign(chain, collection, token id,
 * order_hex)`).
 *
 * Route (OpenSea API v2, https://docs.opensea.io/reference/generate_listing_fulfillment_data_v2):
 *   POST https://api.opensea.io/api/v2/listings/fulfillment_data   header x-api-key
 *   { "listing": { "hash", "chain", "protocol_address" }, "fulfiller": { "address" } }
 *   → { protocol, fulfillment_data: { orders, transaction: { function, to, value, chain, input_data } } }
 *
 * The key is read from process.env.OPENSEA_API_KEY at the moment of the call and goes into exactly one header of exactly one
 * host. It is never logged (log.ts cuts it out of every line), never put in an error, and never reaches the canister: what the
 * canister gets is calldata, which it decodes, checks and re-encodes itself. Nothing here signs or sends anything on a chain.
 */
export const OPENSEA_API = "https://api.opensea.io/api/v2/listings/fulfillment_data";
export const SEAPORT_1_6 = "0x0000000000000068f116a894984e2db1123eb395";
/** OpenSea's chain identifiers (opensea-js `Chain`), for the chains the canister can be configured with. */
export const OPENSEA_CHAIN: Readonly<Record<number, string>> = { 1: "ethereum", 8453: "base", 33139: "ape_chain", 2741: "abstract",
  42161: "arbitrum", 137: "polygon", 81457: "blast" };
const ZERO32 = `0x${"0".repeat(64)}` as Hex;

const SEAPORT = parseAbi([
  "struct OfferItem { uint8 itemType; address token; uint256 identifierOrCriteria; uint256 startAmount; uint256 endAmount; }",
  "struct ConsiderationItem { uint8 itemType; address token; uint256 identifierOrCriteria; uint256 startAmount; uint256 endAmount; address recipient; }",
  "struct OrderParameters { address offerer; address zone; OfferItem[] offer; ConsiderationItem[] consideration; uint8 orderType; uint256 startTime; uint256 endTime; bytes32 zoneHash; uint256 salt; bytes32 conduitKey; uint256 totalOriginalConsiderationItems; }",
  "struct AdvancedOrder { OrderParameters parameters; uint120 numerator; uint120 denominator; bytes signature; bytes extraData; }",
  "struct CriteriaResolver { uint256 orderIndex; uint8 side; uint256 index; uint256 identifier; bytes32[] criteriaProof; }",
  "struct AdditionalRecipient { uint256 amount; address recipient; }",
  "struct BasicOrderParameters { address considerationToken; uint256 considerationIdentifier; uint256 considerationAmount; address offerer; address zone; address offerToken; uint256 offerIdentifier; uint256 offerAmount; uint8 basicOrderType; uint256 startTime; uint256 endTime; bytes32 zoneHash; uint256 salt; bytes32 offererConduitKey; bytes32 fulfillerConduitKey; uint256 totalOriginalAdditionalRecipients; AdditionalRecipient[] additionalRecipients; bytes signature; }",
  "function fulfillAdvancedOrder(AdvancedOrder advancedOrder, CriteriaResolver[] criteriaResolvers, bytes32 fulfillerConduitKey, address recipient) payable returns (bool)",
  "function fulfillBasicOrder(BasicOrderParameters parameters) payable returns (bool)",
]);

/** What the canister is handed: the arguments of `nft_preview` / `nft_sign`, plus what the keeper can say about the timing. */
export type Handoff = { chainId: number; collection: Hex; tokenId: string; orderHex: Hex; fn: "fulfillAdvancedOrder" | "fulfillBasicOrder";
  valueWei: string; zoneExpiration: number | null };

type Item = { itemType: number | string; token: string; identifierOrCriteria: string | number; startAmount: string | number; endAmount: string | number; recipient?: string };
type Params = { offerer: string; zone: string; offer: Item[]; consideration: Item[]; orderType: number | string; startTime: string | number; endTime: string | number;
  zoneHash: string; salt: string | number; conduitKey: string; totalOriginalConsiderationItems: number | string };

const addr = (a: unknown, what: string): Hex => {
  const s = typeof a === "object" && a !== null && "value" in a ? (a as { value: unknown }).value : a;   // the docs' schema wraps some addresses
  if (typeof s !== "string" || !/^0x[0-9a-fA-F]{40}$/.test(s)) throw new Error(`invalid ${what} in OpenSea's answer`);
  return s.toLowerCase() as Hex;
};
const big = (v: unknown, what: string): bigint => {
  if ((typeof v !== "string" && typeof v !== "number") || !/^(0x[0-9a-fA-F]+|\d+)$/.test(String(v))) throw new Error(`invalid ${what} in OpenSea's answer`);
  return BigInt(v);
};
const bytes = (v: unknown, what: string): Hex => {
  if (typeof v !== "string" || !/^0x([0-9a-fA-F]{2})*$/.test(v)) throw new Error(`invalid ${what} in OpenSea's answer`);
  return v as Hex;
};
const b32 = (v: unknown, what: string): Hex => { const h = bytes(v, what); if (h.length !== 66) throw new Error(`invalid ${what} in OpenSea's answer`); return h; };

function params(p: Params) {
  const item = (i: Item) => ({ itemType: Number(big(i.itemType, "itemType")), token: addr(i.token, "token"), identifierOrCriteria: big(i.identifierOrCriteria, "identifier"),
    startAmount: big(i.startAmount, "startAmount"), endAmount: big(i.endAmount, "endAmount") });
  return {
    offerer: addr(p.offerer, "offerer"), zone: addr(p.zone, "zone"), offer: p.offer.map(item),
    consideration: p.consideration.map((i) => ({ ...item(i), recipient: addr(i.recipient, "recipient") })),
    orderType: Number(big(p.orderType, "orderType")), startTime: big(p.startTime, "startTime"), endTime: big(p.endTime, "endTime"),
    zoneHash: b32(p.zoneHash, "zoneHash"), salt: big(p.salt, "salt"), conduitKey: b32(p.conduitKey, "conduitKey"),
    totalOriginalConsiderationItems: big(p.totalOriginalConsiderationItems, "totalOriginalConsiderationItems"),
  };
}

/** Turn OpenSea's `transaction` into calldata for `fulfiller` (the canister): its recipient, no fulfiller conduit, nothing appended. */
export function toHandoff(answer: unknown, chainId: number, fulfiller: Hex): Handoff {
  const tx = (answer as { fulfillment_data?: { transaction?: Record<string, unknown> } })?.fulfillment_data?.transaction;
  if (!tx || typeof tx.function !== "string") throw new Error("invalid answer: no fulfillment_data.transaction");
  if (addr(tx.to, "transaction.to") !== SEAPORT_1_6) throw new Error(`refused: OpenSea names ${String(tx.to)}, not Seaport 1.6`);
  if (Number(tx.chain) !== chainId && tx.chain !== OPENSEA_CHAIN[chainId]) throw new Error(`refused: OpenSea answered for chain ${String(tx.chain)}, not ${chainId}`);
  const fn = tx.function.split("(")[0];
  const input = tx.input_data as Record<string, unknown>;
  const valueWei = big(tx.value, "value").toString();
  if (fn === "fulfillAdvancedOrder") {
    const ao = input.advancedOrder as { parameters: Params; numerator: unknown; denominator: unknown; signature: unknown; extraData: unknown };
    const p = params(ao.parameters);
    const extraData = bytes(ao.extraData, "extraData");
    const orderHex = encodeFunctionData({ abi: SEAPORT, functionName: "fulfillAdvancedOrder", args: [
      { parameters: p, numerator: big(ao.numerator, "numerator"), denominator: big(ao.denominator, "denominator"), signature: bytes(ao.signature, "signature"), extraData },
      [], ZERO32, fulfiller] });
    // SIP-6 v0 + SIP-7: version, fulfiller (20), expiration (uint64) — so the keeper can say how long it has, and refuse one not for us
    const exp = extraData.length >= 2 + 2 * 29 ? Number(BigInt(`0x${extraData.slice(44, 60)}`)) : null;
    if (extraData.length >= 44 && `0x${extraData.slice(4, 44)}`.toLowerCase() !== fulfiller.toLowerCase()) throw new Error("refused: the zone signed for another fulfiller");
    const [first] = p.offer;
    if (!first) throw new Error("invalid answer: no offer item");
    return { chainId, collection: first.token, tokenId: first.identifierOrCriteria.toString(), orderHex, fn, valueWei, zoneExpiration: exp };
  }
  if (fn === "fulfillBasicOrder") {
    const b = (input.parameters ?? input) as Record<string, unknown>;
    const recips = (b.additionalRecipients as { amount: unknown; recipient: unknown }[]).map((r) => ({ amount: big(r.amount, "amount"), recipient: addr(r.recipient, "recipient") }));
    const orderHex = encodeFunctionData({ abi: SEAPORT, functionName: "fulfillBasicOrder", args: [{
      considerationToken: addr(b.considerationToken, "considerationToken"), considerationIdentifier: big(b.considerationIdentifier, "considerationIdentifier"),
      considerationAmount: big(b.considerationAmount, "considerationAmount"), offerer: addr(b.offerer, "offerer"), zone: addr(b.zone, "zone"),
      offerToken: addr(b.offerToken, "offerToken"), offerIdentifier: big(b.offerIdentifier, "offerIdentifier"), offerAmount: big(b.offerAmount, "offerAmount"),
      basicOrderType: Number(big(b.basicOrderType, "basicOrderType")), startTime: big(b.startTime, "startTime"), endTime: big(b.endTime, "endTime"),
      zoneHash: b32(b.zoneHash, "zoneHash"), salt: big(b.salt, "salt"), offererConduitKey: b32(b.offererConduitKey, "offererConduitKey"), fulfillerConduitKey: ZERO32,
      totalOriginalAdditionalRecipients: big(b.totalOriginalAdditionalRecipients, "totalOriginalAdditionalRecipients"), additionalRecipients: recips,
      signature: bytes(b.signature, "signature") }] });
    return { chainId, collection: addr(b.offerToken, "offerToken"), tokenId: big(b.offerIdentifier, "offerIdentifier").toString(), orderHex, fn, valueWei, zoneExpiration: null };
  }
  throw new Error(`refused: OpenSea proposes ${fn}; the canister accepts fulfillAdvancedOrder and fulfillBasicOrder only`);
}

export type OpenSeaOpts = { fetchImpl?: Fetch; apiKey?: () => string | undefined; sleep?: (ms: number) => Promise<void>; log?: Logger; tries?: number };

/**
 * Ask OpenSea for one listing's fulfillment data, for the canister as fulfiller. Retried through backoff.ts; a 429 first waits
 * what OpenSea's Retry-After says (capped at a minute). A rejected key or a missing listing is not retried.
 */
export async function fulfillmentData(req: { chainId: number; orderHash: Hex; fulfiller: Hex }, o: OpenSeaOpts = {}): Promise<Handoff> {
  const fetchImpl = o.fetchImpl ?? fetch, sleep = o.sleep ?? ((ms: number) => new Promise<void>((r) => setTimeout(r, ms)));
  const chain = OPENSEA_CHAIN[req.chainId];
  if (!chain) throw new Error(`refused: chain ${req.chainId} has no OpenSea identifier here`);
  if (!/^0x[0-9a-fA-F]{64}$/.test(req.orderHash) || !/^0x[0-9a-fA-F]{40}$/.test(req.fulfiller)) throw new Error("invalid order hash or fulfiller");
  const body = JSON.stringify({ listing: { hash: req.orderHash, chain, protocol_address: SEAPORT_1_6 }, fulfiller: { address: req.fulfiller } });
  const answer = await withBackoff(async () => {
    const key = (o.apiKey ?? (() => process.env.OPENSEA_API_KEY))();
    if (!key) throw new Error("missing OPENSEA_API_KEY: the keeper reads it from its environment at call time");
    const r = await fetchImpl(OPENSEA_API, { method: "POST", headers: { accept: "application/json", "content-type": "application/json", "x-api-key": key }, body });
    if (r.status === 429) {
      const after = Number(r.headers?.get?.("retry-after"));
      const wait = Number.isFinite(after) && after > 0 ? Math.min(after, 60) * 1000 : 0;
      o.log?.("warn", "opensea_rate_limited", { waitMs: wait });
      if (wait) await sleep(wait);
      throw new Error("opensea: rate limited (429)");
    }
    if (r.status === 401 || r.status === 403) throw new Error(`refused: OpenSea rejected the key (HTTP ${r.status})`);
    if (r.status === 404) throw new Error("missing: OpenSea has no such listing, or it is no longer fillable (HTTP 404)");
    if (r.status === 400) throw new Error(`invalid request per OpenSea (HTTP 400): ${String(await r.text?.()).slice(0, 200)}`);
    if (!r.ok) throw new Error(`opensea: HTTP ${r.status}`);
    return r.json();
  }, { tries: o.tries, sleep, onRetry: (attempt, waitMs, err) => o.log?.("warn", "opensea_retry", { attempt, waitMs, error: String((err as Error)?.message ?? err) }) });
  const h = toHandoff(answer, req.chainId, req.fulfiller);
  o.log?.("info", "opensea_fulfillment", { chainId: h.chainId, collection: h.collection, tokenId: h.tokenId, fn: h.fn, valueWei: h.valueWei, zoneExpiration: h.zoneExpiration });
  return h;
}

/** The candid arguments for `nft_sign` (and `nft_preview`): what the operator's dfx call, or an agent, passes to the canister. */
export const candidArgs = (h: Handoff): string => `(${h.chainId} : nat64, "${h.collection}", "${h.tokenId}", "${h.orderHex}")`;

export const OPENSEA_BASE = "https://api.opensea.io/api/v2";
export type OpenSeaReadOpts = { fetchImpl?: Fetch; apiKey?: () => string | undefined; sleep?: (ms: number) => Promise<void>; log?: Logger; tries?: number };

/**
 * docs/v5/floor-feed.md §2. One GET against OpenSea's v2 API, for the floor's listings and sales: the key read exactly as
 * `fulfillmentData` reads it — at the moment of the call, into one header of one host, never logged, never in an error.
 */
export async function openseaGet(path: string, o: OpenSeaReadOpts = {}): Promise<unknown> {
  const fetchImpl = o.fetchImpl ?? fetch, sleep = o.sleep ?? ((ms: number) => new Promise<void>((r) => setTimeout(r, ms)));
  if (!/^[a-z0-9/_?=&.:-]+$/i.test(path)) throw new Error("invalid OpenSea path");
  return withBackoff(async () => {
    const key = (o.apiKey ?? (() => process.env.OPENSEA_API_KEY))();
    if (!key) throw new Error("missing OPENSEA_API_KEY: the keeper reads it from its environment at call time");
    const r = await fetchImpl(`${OPENSEA_BASE}/${path}`, { method: "GET", headers: { accept: "application/json", "x-api-key": key } });
    if (r.status === 429) {
      const after = Number(r.headers?.get?.("retry-after"));
      const wait = Number.isFinite(after) && after > 0 ? Math.min(after, 60) * 1000 : 0;
      o.log?.("warn", "opensea_rate_limited", { waitMs: wait });
      if (wait) await sleep(wait);
      throw new Error("opensea: rate limited (429)");
    }
    if (r.status === 401 || r.status === 403) throw new Error(`refused: OpenSea rejected the key (HTTP ${r.status})`);
    if (r.status === 404) throw new Error("missing: OpenSea has no such collection (HTTP 404)");
    if (!r.ok) throw new Error(`opensea: HTTP ${r.status}`);
    return r.json();
  }, { tries: o.tries, sleep, onRetry: (attempt, waitMs, err) => o.log?.("warn", "opensea_retry", { attempt, waitMs, error: String((err as Error)?.message ?? err) }) });
}
