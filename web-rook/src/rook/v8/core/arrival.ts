import { decodeEventLog, decodeFunctionData, hexToBytes, toHex, type Hex as VHex } from "viem";
import { ERC721, SEAPORT, SEAPORTS } from "./abis.js";
import type { Address, ArrivalCheck, Hex } from "./types.js";

/**
 * docs/v8/proof-of-reserves.md §2 row 4: an item's arrival transaction on Ethereum, checked from its receipt (and, for a deposit,
 * its input): final (block ≤ N), status 1, the collection's Transfer(x, custodian, tokenId); a purchase also carries a Seaport
 * OrderFulfilled whose offer is that token and whose recipient is the custodian (the price paid is its native consideration); a
 * deposit is the holder's own safeTransferFrom with the 48-byte claim "EWN1" ‖ uint32 4663 ‖ wrapper ‖ recipient
 * (docs/v7/phase1-nft.md §2.4). Pure: the receipts are inputs, one per provider, and they must agree.
 */
export const TRANSFER_TOPIC = "0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef";
export const CLAIM_MAGIC = "0x45574e31";   // "EWN1"
export type RpcLog = { address: string; topics: string[]; data: string; removed?: boolean };
export type RpcReceipt = { status: string; blockNumber: string; transactionHash: string; logs: RpcLog[] } | null;
export type RpcTx = { from: string; to: string | null; input: string } | null;

const lc = (s: string | null | undefined) => (s ?? "").toLowerCase();
const topicAddr = (t: string | undefined) => (t ? `0x${t.slice(-40)}`.toLowerCase() : "");

export type ArrivalExpect = { how: "purchase" | "deposit"; tx: Hex; tokenId: string; collection: Address; custodian: Address; homeChainId: number; wrapper: Address;
  /** the Deposited event's recipient on 4663 */ recipient?: Address | null; finalized: number | null };
export type ArrivalRead = { source: string; receipt: RpcReceipt; tx: RpcTx; error: string | null };

/** A deposit's 48 bytes: magic, chain, wrapper, recipient; null when the data is not that. */
export function parseClaim(data: Hex): { chainId: number; wrapper: Address; recipient: Address } | null {
  const b = hexToBytes(data);
  if (b.length !== 48 || toHex(b.slice(0, 4)) !== CLAIM_MAGIC) return null;
  const chainId = ((b[4] << 24) >>> 0) + (b[5] << 16) + (b[6] << 8) + b[7];
  return { chainId, wrapper: toHex(b.slice(8, 28)) as Address, recipient: toHex(b.slice(28, 48)) as Address };
}

export function checkArrival(reads: ArrivalRead[], x: ArrivalExpect): ArrivalCheck {
  const sources = reads.map((r) => r.source);
  const out = (verdict: ArrivalCheck["verdict"], detail: string, block: number | null = null, price: string | null = null): ArrivalCheck =>
    ({ verdict, block, final: block === null || x.finalized === null ? null : block <= x.finalized, pricePaidWei: price, detail, sources });
  const got = reads.filter((r) => r.receipt);
  if (!got.length) return out("unread", reads.map((r) => `${r.source}: ${r.error ?? "no receipt"}`).join("; ") || "no provider");
  if (new Set(got.map((g) => g.source)).size < 2) return out("unread", `only ${got[0].source} served the receipt: one source is not a verdict`);
  const blocks = new Set(got.map((g) => Number(BigInt(g.receipt!.blockNumber))));
  if (blocks.size > 1 || got.some((g) => g.receipt!.status !== got[0].receipt!.status)) return out("unread", "the providers disagree about the receipt (a reorg in progress?)");
  if (got.length < reads.length) return out("unread", `${reads.filter((r) => !r.receipt).map((r) => r.source).join(", ")} did not serve the receipt`);
  const rc = got[0].receipt!, block = Number(BigInt(rc.blockNumber));
  if (lc(rc.transactionHash) !== lc(x.tx)) return out("bad", `the receipt is of ${rc.transactionHash}, not ${x.tx}`, block);
  if (Number(BigInt(rc.status)) !== 1) return out("bad", "the transaction reverted (status 0)", block);
  const tid = BigInt(x.tokenId);
  const transfers = rc.logs.filter((l) => !l.removed && lc(l.address) === lc(x.collection) && lc(l.topics[0]) === TRANSFER_TOPIC && l.topics.length === 4);
  const inbound = transfers.filter((l) => topicAddr(l.topics[2]) === lc(x.custodian) && BigInt(l.topics[3]) === tid);
  if (!inbound.length) return out("bad", `no Transfer of #${x.tokenId} to the custodian in it`, block);
  if (x.how === "purchase") {
    let price: bigint | null = null;
    for (const l of rc.logs) {
      if (l.removed || !SEAPORTS.includes(lc(l.address) as (typeof SEAPORTS)[number])) continue;
      try {
        const e = decodeEventLog({ abi: SEAPORT, data: l.data as VHex, topics: l.topics as [VHex, ...VHex[]] }) as unknown as { args: { recipient: string; offer: readonly { token: string; identifier: bigint }[]; consideration: readonly { itemType: number; amount: bigint }[] } };
        if (lc(e.args.recipient) !== lc(x.custodian) || !e.args.offer.some((o) => lc(o.token) === lc(x.collection) && o.identifier === tid)) continue;
        price = e.args.consideration.filter((c) => c.itemType === 0).reduce((a, c) => a + c.amount, 0n);
      } catch { /* another event of Seaport */ }
    }
    if (price === null) return out("bad", "a purchase without a Seaport fill of this token to the custodian", block);
    return out("ok", `a Seaport fill of #${x.tokenId} to the custodian for ${price} wei`, block, String(price));
  }
  // a deposit: the holder's own safeTransferFrom, with the claim naming this chain, this wrapper and the recipient
  const txs = reads.map((r) => r.tx).filter(Boolean) as NonNullable<RpcTx>[];
  if (!txs.length) return out("unread", "the transaction's input was not served", block);
  const t = txs[0];
  if (lc(t.to) !== lc(x.collection)) return out("bad", "the deposit transaction does not call the collection", block);
  let data: Hex;
  try {
    const d = decodeFunctionData({ abi: ERC721, data: t.input as VHex });
    if (d.functionName !== "safeTransferFrom") return out("bad", `the deposit calls ${d.functionName}, not safeTransferFrom with a claim`, block);
    const [from, to, id, bytes] = d.args as readonly [string, string, bigint, Hex];
    if (lc(from) !== lc(t.from) || lc(to) !== lc(x.custodian) || id !== tid) return out("bad", "the deposit moves another token, from another sender or to another address", block);
    data = bytes;
  } catch { return out("bad", "the deposit's input is not safeTransferFrom(from, to, id, data)", block); }
  const claim = parseClaim(data);
  if (!claim) return out("bad", "the deposit carries no 48-byte EWN1 claim", block);
  if (claim.chainId !== x.homeChainId || lc(claim.wrapper) !== lc(x.wrapper)) return out("bad", `the claim names chain ${claim.chainId} and wrapper ${claim.wrapper}, not this one`, block);
  if (x.recipient && lc(claim.recipient) !== lc(x.recipient)) return out("bad", `the claim names ${claim.recipient}; the wrapper minted to ${x.recipient}`, block);
  return out("ok", `the holder's own deposit, claiming for ${claim.recipient} on chain ${claim.chainId}`, block);
}
