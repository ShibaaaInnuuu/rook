import { decodeEventLog, encodeEventTopics, type Abi, type Hex as VHex } from "viem";
import { ERC721, ETH_NFT_WRAPPER, FLOOR_MODULE, NITRO, NODE_INTERFACE } from "./abis.js";
import { TRANSFER_TOPIC } from "./arrival.js";
import { fetchRound, verifyBeacon } from "./drand.js";
import { UNRESOLVED_ITEM, type VaultEvent } from "./draw.js";
import type { Fetch, Reader } from "./jsonrpc.js";
import type { Address, Hex, Posting, Stray } from "./types.js";

/**
 * The indexer's own reads for the v8 endpoints (not copied to the site: the site and the CLI read through selfcheck.ts). Logs are
 * read in chunks and kept between reports; everything immutable once final (a resolved draw's set, its beacon, its batch) is
 * cached for good. It reads; it has no key and sends nothing.
 */
export type RawLog = { address: string; topics: string[]; data: string; blockNumber: string; logIndex: string; transactionHash: string; blockHash: string; removed?: boolean };

export class LogCache {
  private readonly c = new Map<string, { upTo: number; logs: RawLog[] }>();
  constructor(private readonly chunk: number) {}
  /** Every log of `address` whose first topic is one of `abi`'s events (or exactly `topics`), from `from` to `to`, incrementally. */
  async logs(r: Reader, address: string, abi: Abi | null, from: number, to: number, topics?: (string | null)[]): Promise<RawLog[]> {
    const t0 = abi ? [(abi.filter((x) => x.type === "event") as { name: string }[]).map((e) => encodeEventTopics({ abi, eventName: e.name } as never)[0])] : topics ?? [];
    const key = `${r.host}|${address.toLowerCase()}|${JSON.stringify(t0)}`;
    const e = this.c.get(key) ?? { upTo: from - 1, logs: [] };
    for (let a = e.upTo + 1; a <= to; a += this.chunk) {
      const b = Math.min(to, a + this.chunk - 1);
      const got = await r.raw<RawLog[]>("eth_getLogs", [{ address, topics: t0, fromBlock: `0x${a.toString(16)}`, toBlock: `0x${b.toString(16)}` }]);
      e.logs.push(...got.filter((l) => !l.removed));
      e.upTo = b;
      this.c.set(key, e);
    }
    return e.logs.filter((l) => Number(BigInt(l.blockNumber)) >= from && Number(BigInt(l.blockNumber)) <= to);
  }
}

export type Decoded = { events: VaultEvent[]; recipients: Map<number, Address>; delivered: Map<number, { txHash: Hex; tx: Hex; at: number | null; destination: Address }>; arrivals: Map<number, Hex> };
/** The wrapper's logs, decoded into the replay's events; `timeOf` gives a block's timestamp. */
export async function decodeVault(logs: RawLog[], timeOf: (block: number) => Promise<number | null>): Promise<Decoded> {
  const events: VaultEvent[] = [], recipients = new Map<number, Address>(), delivered: Decoded["delivered"] = new Map(), arrivals = new Map<number, Hex>();
  for (const l of logs) {
    let e: { eventName: string; args: Record<string, unknown> };
    try { e = decodeEventLog({ abi: ETH_NFT_WRAPPER, data: l.data as VHex, topics: l.topics as [VHex, ...VHex[]] }) as unknown as typeof e; } catch { continue; }
    const block = Number(BigInt(l.blockNumber)), logIndex = Number(BigInt(l.logIndex)), tx = l.transactionHash as Hex, at = await timeOf(block), a = e.args;
    const base = { tx, block, logIndex, at };
    switch (e.eventName) {
      case "Wrapped": case "Deposited": {
        const itemId = Number(a.itemId);
        events.push({ name: e.eventName, itemId, tokenId: String(a.tokenId), ...base });
        arrivals.set(itemId, (e.eventName === "Wrapped" ? a.purchaseTx : a.depositTx) as Hex);
        if (e.eventName === "Deposited") recipients.set(itemId, String(a.to) as Address);
        break;
      }
      case "Redeemed": events.push({ name: "Redeemed", ticket: Number(a.ticketId), owner: String(a.owner) as Address, round: Number(a.round), destination: String(a.destination) as Address, ...base }); break;
      case "Resolved": events.push({ name: "Resolved", ticket: Number(a.ticketId), itemId: Number(a.itemId), tokenId: String(a.tokenId), ...base }); break;
      case "Delivered":
        events.push({ name: "Delivered", ticket: Number(a.ticketId), tokenId: String(a.tokenId), txHash: a.txHash as Hex, ...base });
        delivered.set(Number(a.ticketId), { txHash: a.txHash as Hex, tx, at, destination: String(a.destination) as Address });
        break;
      case "Refunded": events.push({ name: "Refunded", ticket: Number(a.ticketId), itemId: String(a.itemId) === UNRESOLVED_ITEM ? UNRESOLVED_ITEM : String(a.itemId), ...base }); break;
    }
  }
  return { events, recipients, delivered, arrivals };
}

export function decodeCosts(logs: RawLog[]): Map<number, { advancedWei: string; costWei: string }> {
  const out = new Map<number, { advancedWei: string; costWei: string }>();
  for (const l of logs) {
    try {
      const e = decodeEventLog({ abi: FLOOR_MODULE, data: l.data as VHex, topics: l.topics as [VHex, ...VHex[]] }) as unknown as { eventName: string; args: { item: bigint; advanced: bigint; cost: bigint } };
      if (e.eventName === "AdvanceSettled") out.set(Number(e.args.item), { advancedWei: String(e.args.advanced), costWei: String(e.args.cost) });
    } catch { /* another event */ }
  }
  return out;
}

/** The wrapper's redemption books at `block`, from its own views. */
export async function viewsAt(r: Reader, wrapper: Address, block: number): Promise<{ free: number[]; pending: { itemId: number; stamp: number }[]; ticketCount: number; nextToResolve: number; held: number; openTickets: number }> {
  const v = (fn: string, args: unknown[] = []) => r.call<bigint>(wrapper, ETH_NFT_WRAPPER, fn, args, block);
  const [fc, pc, tc, nx, held, open] = (await Promise.all(["freeCount", "pendingCount", "ticketCount", "nextToResolve", "held", "openTickets"].map((f) => v(f)))).map(Number);
  const free = await Promise.all(Array.from({ length: fc }, (_, i) => v("freeAt", [BigInt(i)]).then(Number)));
  const pending = await Promise.all(Array.from({ length: pc }, (_, i) => r.call<readonly [bigint, bigint]>(wrapper, ETH_NFT_WRAPPER, "pendingAt", [BigInt(i)], block).then(([itemId, stamp]) => ({ itemId: Number(itemId), stamp: Number(stamp) }))));
  return { free, pending, ticketCount: tc, nextToResolve: nx, held, openTickets: open };
}

/** A resolved ticket's set as the views give it: `_free` at the block before, then every pending item stamped ≤ the ticket. */
export async function setFromViews(r: Reader, wrapper: Address, beforeBlock: number, ticket: number): Promise<number[]> {
  const v = await viewsAt(r, wrapper, beforeBlock);
  const set = v.free.slice();
  for (const p of v.pending) { if (p.stamp > ticket) break; set.push(p.itemId); }
  return set;
}

/** A round's signature: served identically by the relays that have it and valid under evmnet's key, checked here before it is published. */
export async function beaconOf(round: number, relays: readonly string[], fetchImpl: Fetch): Promise<{ signature: Hex | null; randomness: Hex | null; relays: string[]; note: string }> {
  const got = await fetchRound(round, relays, fetchImpl);
  const valid = new Map<string, string[]>();
  for (const g of got) if (g.signature && verifyBeacon(round, g.signature).ok) valid.set(g.signature, [...(valid.get(g.signature) ?? []), g.relay]);
  const liars = got.filter((g) => g.signature && !valid.has(g.signature)).map((g) => g.relay);
  const best = [...valid].sort((a, b) => b[1].length - a[1].length)[0];
  if (!best) return { signature: null, randomness: null, relays: [], note: `no relay served a valid signature of round ${round} (${got.map((g) => `${g.relay}: ${g.error ?? "invalid"}`).join("; ")})` };
  return { signature: best[0] as Hex, randomness: (verifyBeacon(round, best[0]) as { randomness: Hex }).randomness, relays: best[1], note: liars.length ? `${liars.join(", ")} served bytes that are not evmnet's signature` : "" };
}

/** When block `block` of 4663 reached Ethereum: Nitro's batch for it, its depth there, and that Ethereum block's time. */
export async function postingOf(home: Reader, eth: Reader, block: number, blockHash: Hex): Promise<Posting> {
  try {
    const batch = Number(await home.call<bigint>(NODE_INTERFACE, NITRO, "findBatchContainingBlock", [BigInt(block)], "latest"));
    const conf = Number(await home.call<bigint>(NODE_INTERFACE, NITRO, "getL1Confirmations", [blockHash], "latest"));
    if (conf === 0) return { batch, parentBlock: null, postedAt: null, note: `batch ${batch} is not on Ethereum yet` };
    const head = await eth.raw<{ number: string }>("eth_getBlockByNumber", ["latest", false]);
    const parentBlock = Number(BigInt(head.number)) - conf;
    const b = await eth.raw<{ timestamp: string } | null>("eth_getBlockByNumber", [`0x${parentBlock.toString(16)}`, false]);
    return { batch, parentBlock, postedAt: b ? Number(BigInt(b.timestamp)) : null, note: `batch ${batch}, ${conf} Ethereum confirmations as ${home.host} counts them (± a block: two nodes' heads)` };
  } catch (e) { return { batch: null, parentBlock: null, postedAt: null, note: `the batch could not be read: ${String((e as Error)?.message ?? e).slice(0, 100)}` }; }
}

/**
 * proof-of-reserves.md §2 row 11: tokens of the collection that reached the custodian in the last `lookback` Ethereum blocks, are
 * still owned by it at N, and are not held by the wrapper: a deposit awaiting finality or its mint, a stray being returned or held.
 */
export async function strayScan(eth: Reader, cache: LogCache, collection: Address, custodian: Address, N: number, lookback: number, heldTokens: Set<string>,
  pendingDeposits: Set<string>, timeOf: (b: number) => Promise<number | null>, now: number): Promise<Stray[]> {
  const to = `0x${custodian.slice(2).toLowerCase().padStart(64, "0")}`;
  const logs = await cache.logs(eth, collection, null, Math.max(0, N - lookback), N, [TRANSFER_TOPIC, null, to]);
  const seen = new Map<string, RawLog>();
  for (const l of logs) if (l.topics.length === 4) seen.set(BigInt(l.topics[3]).toString(), l);
  const out: Stray[] = [];
  for (const [tokenId, l] of seen) {
    if (heldTokens.has(tokenId)) continue;
    const owner = await eth.call<string>(collection, ERC721, "ownerOf", [BigInt(tokenId)], N).catch(() => null);
    if (!owner || owner.toLowerCase() !== custodian.toLowerCase()) continue;     // already gone back, or delivered
    const block = Number(BigInt(l.blockNumber)), at = await timeOf(block);
    const deposit = pendingDeposits.has(tokenId);
    out.push({ tokenId, from: `0x${l.topics[1].slice(-40)}` as Address, tx: l.transactionHash as Hex, block, at,
      state: deposit ? "a holder's deposit, judged, awaiting its mint" : "not a valid claim of this wrapper: being returned to its sender, or held", overdue: at !== null && now - at > 2 * 3_600 });
  }
  return out.sort((a, b) => b.block - a.block);
}
