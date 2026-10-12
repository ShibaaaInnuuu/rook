import { decodeFunctionData, encodeAbiParameters, encodeEventTopics, encodeFunctionData, encodeFunctionResult, keccak256, stringToHex, toHex, concat, numberToHex, type Abi, type Hex } from "viem";
import { ERC721, ETH_NFT_WRAPPER, FLOOR_MODULE, NITRO, NODE_INTERFACE, SEAPORT, SEAPORTS } from "../src/v8/abis.js";
import { EVMNET, PIN_AHEAD, roundAt } from "../src/v8/drand.js";
import { drawIndex, replay, type Replay, type VaultEvent } from "../src/v8/draw.js";
import type { Address, CanisterRecords, MetadataSnapshot, SnapshotToken } from "../src/v8/types.js";
import { FIXTURE_BURNS, RELAY_ROUNDS } from "./v8vectors.js";

/**
 * docs/v8: an in-memory Milady vault for the tests and the fixture set — every EthNftWrapper event from the first purchase to a
 * ticket pinned 40 s before `GEN`, drawn with REAL drand evmnet beacons (test/v8vectors.ts) through the contract's own formula, and
 * a fake Robinhood Chain, a fake Ethereum and drand's relays served from it over JSON-RPC / HTTP, so the REAL v8 service and the
 * REAL self-check run over it unchanged. Addresses other than the collection's, transactions and traits are invented.
 */
export const GEN = 1_790_550_000;
const E18 = 10n ** 18n;
const a = (prefix: string, n: number | bigint) => `0x${prefix}${BigInt(n).toString(16).padStart(40 - prefix.length, "0")}` as Address;
export const V8ADDR = {
  collection: "0x5af0d9827e0c53e4799bb226655a1de152a425a5" as Address,       // Milady Maker on Ethereum (the real contract)
  wrapper: a("30", 1), module: a("30", 0xf1), custodian: a("30", 0xc05), beacon: a("30", 0xbe),
  zone: "0x000056f7000000ece9003ca63978907a00ffd100" as Address,             // an OpenSea SignedZone address shape
  fees: a("0a", 0xfee),
};
export const HOME_RPCS = ["https://rh-a.fixture.invalid", "https://rh-b.fixture.invalid"];
export const ETH_RPCS = ["https://eth-a.fixture.invalid", "https://eth-b.fixture.invalid"];
export const RELAYS = ["https://api.drand.sh", "https://api2.drand.sh"];

// block clocks, exact both ways: 4663 at 10 blocks a second, Ethereum at 12 s (anchored on blocks measured 2026-09-27)
const HB = 74_321_585, HT = 1_790_550_635, EB = 26_071_976, ET = 1_790_550_635;
export const homeBlockAt = (t: number) => HB + (t - HT) * 10;
export const homeTimeOf = (b: number) => HT + Math.floor((b - HB) / 10);
export const ethBlockAt = (t: number) => EB + Math.floor((t - ET) / 12);
export const ethTimeOf = (b: number) => ET + (b - EB) * 12;
const txh = (s: string) => keccak256(stringToHex(s));

function rng(seed: number) { return () => { seed = (seed + 0x6d2b79f5) | 0; let t = seed; t = Math.imul(t ^ (t >>> 15), t | 1); t ^= t + Math.imul(t ^ (t >>> 7), t | 61); return ((t ^ (t >>> 14)) >>> 0) / 4294967296; }; }
const pick = (r: () => number, xs: [string, number][]) => { const total = xs.reduce((s, x) => s + x[1], 0); let v = r() * total; for (const [k, w] of xs) { if ((v -= w) < 0) return k; } return xs[xs.length - 1][0]; };

export function fixtureSnapshot(): MetadataSnapshot {
  const r = rng(42), tokens: SnapshotToken[] = [];
  for (let id = 0; id < 400; id++) {
    const traits: Record<string, string> = {
      Background: pick(r, [["Sky", 30], ["Orange", 18], ["Pink", 14], ["Mint", 12], ["Grey", 10], ["Night", 8], ["Gold", 5], ["Static", 3]]),
      Core: pick(r, [["Pastel", 55], ["Tan", 30], ["Dark", 15]]),
      Eyes: pick(r, [["Smug", 25], ["Tired", 22], ["Sparkle", 18], ["Closed", 15], ["Heart", 12], ["Laser", 8]]),
      Shirt: pick(r, [["Tee", 28], ["Hoodie", 22], ["Stripes", 16], ["Suit", 12], ["Jersey", 10], ["Kimono", 8], ["Armor", 4]]),
    };
    if (r() < 0.6) traits.Hat = pick(r, [["Beanie", 25], ["Cap", 22], ["Bow", 18], ["Halo", 12], ["Horns", 10], ["Crown", 8], ["Cowboy", 5]]);
    if (id === 237) traits.Background = "Void";   // a one-of-one value
    tokens.push({ tokenId: String(id), traits });
  }
  return { version: 1, collection: V8ADDR.collection, chainId: 1, name: "Milady Maker (fixture traits)", takenAt: GEN - 12 * 86_400,
    source: { tokenURI: "fixture://milady/{id}", ipfsCid: null, note: "FIXTURE: 400 invented tokens with invented traits, not Milady's. A real snapshot is written by scripts/snapshot-metadata.ts from the collection's tokenURI, once, and its SHA-256 pinned before the first draw." },
    tokens };
}

export type WorldItem = { itemId: number; tokenId: string; how: "purchase" | "deposit"; mintAt: number; ethTx: Hex; ethAt: number; from: Address; recipient: Address; priceWei: bigint };
export type WorldTicket = { id: number; burnAt: number; round: number; owner: Address; destination: Address; resolveAt: number | null; itemId: number | null; deliveredAt: number | null; deliveryTx: Hex | null; postDelay: number | null };
export type World = {
  snapshot: MetadataSnapshot; items: WorldItem[]; tickets: WorldTicket[]; events: VaultEvent[]; replayed: Replay;
  module: { E: bigint; R: bigint; C: bigint; T: bigint; escrow: bigint; revenue: bigint; advanced: bigint; advancedAt: number; balance: bigint };
  ethBacked: bigint; custodianWei: bigint; stray: { tokenId: string; from: Address; at: number; tx: Hex };
  canister: CanisterRecords;
};

export function buildV8World(): World {
  const snapshot = fixtureSnapshot(), r = rng(7);
  const ids = new Set<string>(["237"]);
  while (ids.size < 20) ids.add(String(Math.floor(r() * 399)));
  const tokenIds = [...ids].filter((x) => x !== "237"); tokenIds.splice(16, 0, "237");
  const D = 86_400, B = FIXTURE_BURNS;
  const mintTimes = [...Array.from({ length: 14 }, (_, k) => GEN - 10 * D + Math.round(k * 0.33 * D)), 1_790_210_000, 1_790_230_000, 1_790_380_000, GEN - 60_000, GEN - 3_600, GEN - 20];
  const deposits = new Set([3, 8, 15, 19]);
  const items: WorldItem[] = mintTimes.map((t, k) => ({ itemId: k, tokenId: tokenIds[k], how: deposits.has(k) ? "deposit" : "purchase", mintAt: t, ethTx: txh(`eth-arrival:${k}`), ethAt: t - 1_200,
    from: deposits.has(k) ? a("40", k + 1) : a("5e", k + 1), recipient: deposits.has(k) ? a("41", k + 1) : V8ADDR.wrapper, priceWei: 940_000_000_000_000_000n + BigInt(k) * 1_000_000_000_000_000n }));
  const delays = [130, 150, 170, 140, 960, null, 160, 145];
  const tickets: WorldTicket[] = [...B, GEN - 40].map((burnAt, id) => ({ id, burnAt, round: roundAt(burnAt) + PIN_AHEAD, owner: a("71", id + 1), destination: id === 3 ? "0x000000000000000000000000000000000000dEaD" : a("70", id + 1),
    resolveAt: id < 8 ? burnAt + 75 : null, itemId: null, deliveredAt: id < 7 ? burnAt + 75 + 1_500 : null, deliveryTx: id < 7 ? txh(`eth-delivery:${id}`) : null, postDelay: id < 8 ? delays[id] : 150 }));

  // the contract's machine, run forward in time: arrivals, burns, resolutions with the real beacons, deliveries
  type Ev = { t: number; order: number; run: () => VaultEvent };
  const evs: Ev[] = [], free: number[] = [], queue: { itemId: number; stamp: number }[] = [];
  let head = 0, ticketCount = 0;
  const base = (t: number, name: string, n: number) => ({ tx: txh(`home:${name}:${n}`), block: homeBlockAt(t), logIndex: 0, at: t });
  for (const it of items) evs.push({ t: it.mintAt, order: 0, run: () => { queue.push({ itemId: it.itemId, stamp: ticketCount });
    return { name: it.how === "purchase" ? "Wrapped" : "Deposited", itemId: it.itemId, tokenId: it.tokenId, ...base(it.mintAt, "mint", it.itemId) }; } });
  for (const tk of tickets) {
    evs.push({ t: tk.burnAt, order: 1, run: () => { ticketCount++; return { name: "Redeemed", ticket: tk.id, owner: tk.owner, round: tk.round, destination: tk.destination, ...base(tk.burnAt, "burn", tk.id) }; } });
    if (tk.resolveAt !== null) evs.push({ t: tk.resolveAt, order: 2, run: () => {
      while (head < queue.length && queue[head].stamp <= tk.id) free.push(queue[head++].itemId);
      const rr = RELAY_ROUNDS[tk.round];
      if (!rr) throw new Error(`no vector for round ${tk.round}`);
      const i = drawIndex(rr.randomness, V8ADDR.wrapper, tk.id, free.length), itemId = free[i];
      free[i] = free[free.length - 1]; free.pop();
      tk.itemId = itemId;
      return { name: "Resolved", ticket: tk.id, itemId, tokenId: items[itemId].tokenId, ...base(tk.resolveAt!, "resolve", tk.id) };
    } });
    if (tk.deliveredAt !== null) evs.push({ t: tk.deliveredAt, order: 3, run: () => ({ name: "Delivered", ticket: tk.id, tokenId: items[tk.itemId!].tokenId, txHash: tk.deliveryTx!, ...base(tk.deliveredAt!, "deliver", tk.id) }) });
  }
  const events = evs.sort((x, y) => x.t - y.t || x.order - y.order).map((e) => e.run());
  const replayed = replay(events, (e) => items[e.itemId].ethTx);
  const module = { E: 1_850_000_000_000_000_000n, R: 500_000_000_000_000_000n, C: 350_000_000_000_000_000n, T: E18, escrow: 799_000_000_000_000_000n, revenue: 12_000_000_000_000_000n,
    advanced: 970_000_000_000_000_000n, advancedAt: GEN - 7_200, balance: 811_000_000_000_000_000n };
  const held = items.filter((it) => replayed.items.get(it.itemId)?.held);
  const canister: CanisterRecords = { readAt: GEN - 120, how: "query",
    buys: [...items.filter((i) => i.how === "purchase").map((i) => ({ id: String(i.itemId), collection: V8ADDR.collection, phase: "Settled", advancedWei: String(module.advanced), orderId: txh(`relay:${i.itemId}`), tokenId: i.tokenId, tx: i.ethTx, note: "bought the cheapest verified listing" })),
      { id: "20", collection: V8ADDR.collection, phase: "Arrived", advancedWei: "960100000000000000", orderId: txh("relay:20"), tokenId: null, tx: null, note: "arrived at the canister's Ethereum address; waiting for a listing under the bound" }],
    deliveries: tickets.filter((t) => t.itemId !== null && t.deliveredAt === null).map((t) => ({ ticket: t.id, tokenId: items[t.itemId!].tokenId, phase: "Pinned", deliverBy: t.resolveAt! + 7 * D, tx: null })),
    deposits: items.filter((i) => i.how === "deposit").map((i) => ({ tx: i.ethTx, tokenId: i.tokenId })), leftoverWei: "21000000000000000",
    caps: { maxItems: 16, maxValueWei: String(15n * E18), itemsHeld: held.length, itemsInFlight: 1, valueWei: String(BigInt(held.length) * 940_000_000_000_000_000n) } };
  return { snapshot, items, tickets, events, replayed, module, ethBacked: module.E, custodianWei: 1_121_100_000_000_000_000n,
    stray: { tokenId: "399", from: a("66", 1), at: GEN - 1_800, tx: txh("eth-stray:399") }, canister };
}

// ---- the fake chains ------------------------------------------------------------------------------------------------------------

type Log = { address: string; topics: Hex[]; data: Hex; blockNumber: Hex; logIndex: Hex; transactionHash: Hex; blockHash: Hex };
const hx = (n: number | bigint) => numberToHex(BigInt(n));
const bh = (chain: string, n: number) => txh(`${chain}-block:${n}`);
function logOf(abi: Abi, eventName: string, args: Record<string, unknown>, address: string, block: number, logIndex: number, tx: Hex, chain: string): Log {
  const ev = abi.find((x) => x.type === "event" && x.name === eventName) as unknown as { inputs: { name: string; type: string; indexed?: boolean; components?: unknown }[] };
  const topics = encodeEventTopics({ abi, eventName, args } as never) as Hex[];
  const nonIdx = ev.inputs.filter((i) => !i.indexed);
  const data = encodeAbiParameters(nonIdx as never, nonIdx.map((i) => args[i.name]) as never);
  return { address, topics, data, blockNumber: hx(block), logIndex: hx(logIndex), transactionHash: tx, blockHash: bh(chain, block) };
}

export type ChainOpts = {
  /** the world's clock: heads are read at this time */
  now?: number;
  /** providers that do not answer (by host) */
  down?: Set<string>;
  /** providers that refuse state older than their head (no archive) */
  noArchive?: Set<string>;
  /** a provider's answer replaced: return a value to use it, undefined to answer honestly */
  lie?: (host: string, method: string, fn: string | null, args: readonly unknown[]) => unknown;
  /** the drand relays' answers replaced (a round's signature) */
  relayLie?: (relay: string, round: number) => string | undefined;
};

export function v8Fetch(w: World, o: ChainOpts = {}) {
  const now = o.now ?? GEN + 30;
  const homeLogs: Log[] = w.events.map((e) => {
    const x = e as VaultEvent & Record<string, unknown>;
    const args: Record<string, unknown> =
      e.name === "Wrapped" ? { itemId: BigInt(e.itemId), tokenId: BigInt(e.tokenId), purchaseTx: w.items[e.itemId].ethTx, to: V8ADDR.wrapper } :
      e.name === "Deposited" ? { itemId: BigInt(e.itemId), tokenId: BigInt(e.tokenId), depositTx: w.items[e.itemId].ethTx, to: w.items[e.itemId].recipient } :
      e.name === "Redeemed" ? { ticketId: BigInt(e.ticket), owner: e.owner, round: BigInt(e.round), destination: e.destination } :
      e.name === "Resolved" ? { ticketId: BigInt(e.ticket), itemId: BigInt(e.itemId), tokenId: BigInt(e.tokenId), deliverBy: BigInt((e.at ?? 0) + 7 * 86_400) } :
      e.name === "Delivered" ? { ticketId: BigInt(e.ticket), tokenId: BigInt(e.tokenId), destination: w.tickets[e.ticket].destination, txHash: e.txHash } :
      { ticketId: BigInt(Number(x.ticket)), itemId: BigInt(String(x.itemId)), owner: w.tickets[Number(x.ticket)].owner };
    return logOf(ETH_NFT_WRAPPER, e.name, args, V8ADDR.wrapper, e.block, e.logIndex, e.tx, "home");
  });
  for (const it of w.items) if (it.how === "purchase") {
    const cost = (w.module.advanced * it.priceWei + 960_100_000_000_000_000n - 1n) / 960_100_000_000_000_000n;
    homeLogs.push(logOf(FLOOR_MODULE, "AdvanceSettled", { item: BigInt(it.itemId), advanced: w.module.advanced, cost }, V8ADDR.module, homeBlockAt(it.mintAt), 1, txh(`home:mint:${it.itemId}`), "home"));
  }
  // Ethereum: each arrival's receipt (and a deposit's input), the Transfer logs to the custodian, the stray
  const receipts = new Map<string, { status: Hex; blockNumber: Hex; transactionHash: Hex; logs: Log[] }>(), txs = new Map<string, { from: string; to: string; input: Hex }>();
  const ethLogs: Log[] = [];
  for (const it of w.items) {
    const blk = ethBlockAt(it.ethAt), logs: Log[] = [];
    logs.push(logOf(ERC721, "Transfer", { from: it.from, to: V8ADDR.custodian, tokenId: BigInt(it.tokenId) }, V8ADDR.collection, blk, 0, it.ethTx, "eth"));
    if (it.how === "purchase") {
      const fee = it.priceWei / 100n;
      logs.push(logOf(SEAPORT, "OrderFulfilled", { orderHash: txh(`order:${it.itemId}`), offerer: it.from, zone: V8ADDR.zone, recipient: V8ADDR.custodian,
        offer: [{ itemType: 2, token: V8ADDR.collection, identifier: BigInt(it.tokenId), amount: 1n }],
        consideration: [{ itemType: 0, token: "0x0000000000000000000000000000000000000000", identifier: 0n, amount: it.priceWei - fee, recipient: it.from },
          { itemType: 0, token: "0x0000000000000000000000000000000000000000", identifier: 0n, amount: fee, recipient: V8ADDR.fees }] }, SEAPORTS[0], blk, 1, it.ethTx, "eth"));
      txs.set(it.ethTx, { from: V8ADDR.custodian, to: SEAPORTS[0], input: "0x" });
    } else {
      const claim = concat(["0x45574e31", numberToHex(4663, { size: 4 }), V8ADDR.wrapper, it.recipient]);
      txs.set(it.ethTx, { from: it.from, to: V8ADDR.collection, input: encodeFunctionData({ abi: ERC721, functionName: "safeTransferFrom", args: [it.from, V8ADDR.custodian, BigInt(it.tokenId), claim] }) });
    }
    receipts.set(it.ethTx, { status: "0x1", blockNumber: hx(blk), transactionHash: it.ethTx, logs });
    ethLogs.push(logs[0]);
  }
  ethLogs.push(logOf(ERC721, "Transfer", { from: w.stray.from, to: V8ADDR.custodian, tokenId: BigInt(w.stray.tokenId) }, V8ADDR.collection, ethBlockAt(w.stray.at), 0, w.stray.tx, "eth"));

  const states = new Map<number, Replay>();
  const stateAt = (b: number) => { let s = states.get(b); if (!s) { s = replay(w.events.filter((e) => e.block <= b), (e) => w.items[e.itemId].ethTx); states.set(b, s); } return s; };
  const homeHead = homeBlockAt(now), ethHead = ethBlockAt(now), finalized = ethBlockAt(now - 780);
  const ticketStage = { pinned: 1, resolved: 2, delivered: 3, refunded: 4 } as const;
  const blockOf = (tag: unknown, head: number) => (tag === "latest" || tag === "finalized" || tag === undefined ? head : Number(BigInt(String(tag))));

  function home(host: string, method: string, params: unknown[]): unknown {
    if (method === "eth_chainId") return hx(4663);
    if (method === "eth_blockNumber") return hx(homeHead);
    if (method === "eth_getBlockByNumber") { const b = blockOf(params[0], homeHead); return { number: hx(b), hash: bh("home", b), timestamp: hx(homeTimeOf(b)), l1BlockNumber: hx(ethBlockAt(homeTimeOf(b))) }; }
    if (method === "eth_getBalance") return hx(String(params[0]).toLowerCase() === V8ADDR.module ? w.module.balance : 0n);
    if (method === "eth_getLogs") {
      const f = params[0] as { address: string; topics?: (string | string[] | null)[]; fromBlock: string; toBlock: string };
      const t0 = f.topics?.[0], t1 = f.topics?.[1];
      return homeLogs.filter((l) => l.address.toLowerCase() === f.address.toLowerCase() && Number(BigInt(l.blockNumber)) >= Number(BigInt(f.fromBlock)) && Number(BigInt(l.blockNumber)) <= Number(BigInt(f.toBlock))
        && (!t0 || (Array.isArray(t0) ? t0.includes(l.topics[0]) : t0 === l.topics[0])) && (!t1 || t1 === l.topics[1]));
    }
    if (method === "eth_call") {
      const [{ to, data }, tag] = params as [{ to: string; data: Hex }, unknown];
      const b = blockOf(tag, homeHead);
      if (o.noArchive?.has(host) && b < homeHead - 100) throw new Error("missing trie node");
      const T = to.toLowerCase();
      if (T === NODE_INTERFACE.toLowerCase()) {
        const d = decodeFunctionData({ abi: NITRO, data });
        const lied = o.lie?.(host, method, d.functionName, d.args ?? []); if (lied !== undefined) return encodeFunctionResult({ abi: NITRO, functionName: d.functionName, result: lied } as never);
        if (d.functionName === "findBatchContainingBlock") return encodeFunctionResult({ abi: NITRO, functionName: "findBatchContainingBlock", result: 283_951n + BigInt(Math.floor((Number(d.args[0]) - 74_121_585) / 2_000)) });
        const hash = String(d.args[0]);
        const tk = w.tickets.find((t) => bh("home", homeBlockAt(t.burnAt)) === hash);
        if (!tk || tk.postDelay === null) throw new Error("execution reverted: block not found in any batch");
        return encodeFunctionResult({ abi: NITRO, functionName: "getL1Confirmations", result: BigInt(Math.max(0, ethHead - ethBlockAt(tk.burnAt + tk.postDelay))) });
      }
      if (T === V8ADDR.module) {
        const d = decodeFunctionData({ abi: FLOOR_MODULE, data }), m = w.module;
        const val: Record<string, bigint> = { ethBackedUnits: m.E, reserveUnits: m.R, netUnits: m.C, transitUnits: m.T, escrowBalance: m.escrow, revenue: m.revenue, advanced: m.advanced, advancedAt: BigInt(m.advancedAt) };
        const lied = o.lie?.(host, method, d.functionName, d.args ?? []);
        return encodeFunctionResult({ abi: FLOOR_MODULE, functionName: d.functionName, result: lied ?? val[d.functionName] } as never);
      }
      if (T !== V8ADDR.wrapper) return "0x";
      const d = decodeFunctionData({ abi: ETH_NFT_WRAPPER, data }), s = stateAt(b), args = (d.args ?? []) as readonly bigint[];
      const heldN = [...s.items.values()].filter((i) => i.held).length, open = [...s.tickets.values()].filter((t) => t.stage === "pinned" || t.stage === "resolved").length;
      const result = ((): unknown => {
        switch (d.functionName) {
          case "totalSupply": return BigInt(heldN - open) * E18 + w.ethBacked;
          case "held": return BigInt(heldN);
          case "openTickets": return BigInt(open);
          case "ethBacked": return w.ethBacked;
          case "itemCount": return BigInt(s.items.size);
          case "item": { const it = w.items[Number(args[0])]; return { tokenId: BigInt(it.tokenId), sourceTx: it.ethTx, deposit: it.how === "deposit" }; }
          case "itemOf": { const e = [...s.items].find(([, i]) => i.held && i.tokenId === String(args[0])); return e ? BigInt(e[0] + 1) : 0n; }
          case "ticketCount": return BigInt(s.tickets.size);
          case "nextToResolve": return BigInt(s.nextToResolve);
          case "freeCount": return BigInt(s.free.length);
          case "freeAt": return BigInt(s.free[Number(args[0])]);
          case "pendingCount": return BigInt(s.pending.length);
          case "pendingAt": { const p = s.pending[Number(args[0])]; return [BigInt(p.itemId), BigInt(p.stamp)]; }
          case "beacon": return V8ADDR.beacon;
          case "ticket": {
            const id = Number(args[0]), t = s.tickets.get(id), wt = w.tickets[id];
            return { stage: t ? ticketStage[t.stage] : 0, owner: wt.owner, round: BigInt(wt.round), pinnedAt: BigInt(wt.burnAt), pinnedAtBlock: BigInt(ethBlockAt(wt.burnAt)),
              deliverBy: wt.resolveAt ? BigInt(wt.resolveAt + 7 * 86_400) : 0n, deliverByBlock: 0n, itemId: BigInt(t?.itemId ?? 0), deliveryTx: t?.stage === "delivered" ? wt.deliveryTx! : toHex(0, { size: 32 }), destination: wt.destination };
          }
        }
        throw new Error(`execution reverted: ${(d as { functionName: string }).functionName}`);
      })();
      const lied = o.lie?.(host, method, d.functionName, args);
      return encodeFunctionResult({ abi: ETH_NFT_WRAPPER, functionName: d.functionName, result: lied ?? result } as never);
    }
    throw new Error(`method ${method} not served`);
  }
  function eth(host: string, method: string, params: unknown[]): unknown {
    if (method === "eth_chainId") return "0x1";
    if (method === "eth_getBlockByNumber") { const b = params[0] === "finalized" ? finalized : blockOf(params[0], ethHead); return { number: hx(b), hash: bh("eth", b), timestamp: hx(ethTimeOf(b)) }; }
    if (method === "eth_getBalance") return hx(String(params[0]).toLowerCase() === V8ADDR.custodian ? w.custodianWei : 0n);
    if (method === "eth_getTransactionReceipt") return receipts.get(String(params[0])) ?? null;
    if (method === "eth_getTransactionByHash") return txs.get(String(params[0])) ?? null;
    if (method === "eth_getLogs") {
      const f = params[0] as { address: string; topics?: (string | null)[]; fromBlock: string; toBlock: string };
      return ethLogs.filter((l) => l.address.toLowerCase() === f.address.toLowerCase() && Number(BigInt(l.blockNumber)) >= Number(BigInt(f.fromBlock)) && Number(BigInt(l.blockNumber)) <= Number(BigInt(f.toBlock))
        && (f.topics ?? []).every((t, i) => !t || t.toLowerCase() === l.topics[i]?.toLowerCase()));
    }
    if (method === "eth_call") {
      const [{ to, data }] = params as [{ to: string; data: Hex }];
      if (to.toLowerCase() !== V8ADDR.collection) return "0x";
      const d = decodeFunctionData({ abi: ERC721, data });
      const tokenId = String(d.args![0]);
      const lied = o.lie?.(host, method, d.functionName, d.args ?? []);
      if (lied !== undefined) return encodeFunctionResult({ abi: ERC721, functionName: "ownerOf", result: lied as Address });
      const it = w.items.find((i) => i.tokenId === tokenId);
      const heldNow = it && w.replayed.items.get(it.itemId)?.held;
      const tk = it ? w.tickets.find((t) => t.itemId === it.itemId && t.deliveredAt !== null) : undefined;
      const owner = heldNow || tokenId === w.stray.tokenId ? V8ADDR.custodian : tk ? tk.destination : a("60", Number(tokenId) + 1);
      return encodeFunctionResult({ abi: ERC721, functionName: "ownerOf", result: owner });
    }
    throw new Error(`method ${method} not served`);
  }
  const res = (status: number, body: string) => ({ ok: status < 400, status, text: async () => body, json: async () => JSON.parse(body) });
  return async (url: string, init?: { method?: string; body?: string }) => {
    const u = new URL(url);
    if (o.down?.has(u.host)) throw new Error("connect ECONNREFUSED");
    if (RELAYS.some((x) => new URL(x).host === u.host)) {
      const m = u.pathname.match(/\/([0-9a-f]{64})\/public\/(\d+)$/);
      if (!m || m[1] !== EVMNET.chainHash) return res(404, "no such beacon");
      const round = Number(m[2]), v = RELAY_ROUNDS[round], lied = o.relayLie?.(u.host, round);
      if (!v && !lied) return res(425, "not yet");
      return res(200, JSON.stringify({ round, signature: (lied ?? v.signature).replace(/^0x/, ""), randomness: v?.randomness.replace(/^0x/, "") ?? "" }));
    }
    const { method, params, id } = JSON.parse(init?.body ?? "{}") as { method: string; params: unknown[]; id: number };
    try {
      const result = HOME_RPCS.some((x) => new URL(x).host === u.host) ? home(u.host, method, params) : ETH_RPCS.some((x) => new URL(x).host === u.host) ? eth(u.host, method, params) : (() => { throw new Error("unknown host"); })();
      return res(200, JSON.stringify({ jsonrpc: "2.0", id, result }));
    } catch (e) { return res(200, JSON.stringify({ jsonrpc: "2.0", id, error: { code: /revert/.test(String(e)) ? 3 : -32000, message: String((e as Error).message) } })); }
  };
}
