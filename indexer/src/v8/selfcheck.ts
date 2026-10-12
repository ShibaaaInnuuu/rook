import { decodeEventLog, encodeEventTopics, type Hex as VHex } from "viem";
import { ERC721, ETH_NFT_WRAPPER, FLOOR_MODULE, NITRO, NODE_INTERFACE } from "./abis.js";
import { checkArrival, type ArrivalRead, type RpcReceipt, type RpcTx } from "./arrival.js";
import { DRAND_RELAYS, fetchRound } from "./drand.js";
import { verifyDraw } from "./draw.js";
import { computeEthReserve, LIMITS, type EthReserveInput, type LedgerItemIn } from "./ethReserve.js";
import { hostOf, reader, RpcError, type Fetch, type Reader } from "./jsonrpc.js";
import type { Address, ArrivalCheck, CallLog, ChainRead, DrawProof, DrawVerdict, EthReserveReport, Hex, ModuleBooks, OwnerSaid, WrapperBooks } from "./types.js";

/**
 * "Check it yourself" (docs/v8/proof-of-reserves.md §4, redemption-draw.md §2.3): the reads the page's button and the CLI make,
 * from the visitor's own machine to providers the visitor chooses, with no Rook service in between. Everything is decided by the
 * same pure code the indexer runs (ethReserve.ts, draw.ts). Decision P2 (the recommended default): the defaults are keyless
 * providers independent of the canister's pair (eth.drpc.org, xrpc.cl/eth) and of the monitor's (Dwellir, xrpc.cl/eth), measured
 * 2026-09-27 to answer `finalized`, `ownerOf` at the finalized block and CORS from a browser.
 */
export const DEFAULT_ETH_RPCS = ["https://ethereum-rpc.publicnode.com", "https://mainnet.gateway.tenderly.co"] as const;
export const DEFAULT_HOME_RPCS = ["https://rpc.mainnet.chain.robinhood.com", "https://rpc.ordofi.network"] as const;
export const HOME_CHAIN_ID = 4663;

const msg = (e: unknown) => String((e as Error)?.message ?? e).slice(0, 120);
async function pool<T, R>(xs: T[], n: number, f: (x: T, i: number) => Promise<R>): Promise<R[]> {
  const out: R[] = new Array(xs.length); let next = 0;
  await Promise.all(Array.from({ length: Math.min(n, xs.length) }, async () => { while (next < xs.length) { const i = next++; out[i] = await f(xs[i], i); } }));
  return out;
}
async function onChain(r: Reader, want: number): Promise<void> {
  const id = Number(BigInt(await r.raw<string>("eth_chainId", [])));
  if (id !== want) throw new RpcError(`is on chain ${id}, not ${want}`);
}

export type ReservesCheckOpts = {
  slug: string; name: string; symbol: string;
  wrapper: Address; collection: Address; custodian: Address; module: Address | null;
  homeRpcs: string[]; ethRpcs: string[];
  fetch: Fetch; now?: () => number;
  /** blocks behind the lowest 4663 head (default 20, as the indexer) */
  margin?: number;
  arrivals?: boolean;
  concurrency?: number;
};

export type ReservesRead = { input: EthReserveInput; calls: CallLog[]; homeBlock: number | null; ethBlock: number | null };
/**
 * The reads, without the verdict. `ledger`: the indexer passes its fold of the wrapper's events (the self-check reads the item views
 * instead), and the owners and arrivals are read for exactly those items; `recipients`: a deposit's Deposited.to, when known.
 */
export async function readReserves(o: ReservesCheckOpts & { ledger?: EthReserveInput["ledger"]; recipients?: Map<number, Address>; block?: number }): Promise<ReservesRead> {
  const calls: CallLog[] = [], now = o.now ?? (() => Math.floor(Date.now() / 1000)), n = o.concurrency ?? 4;
  const home = o.homeRpcs.map((u) => reader(u, "robinhood", o.fetch, calls)), eth = o.ethRpcs.map((u) => reader(u, "ethereum", o.fetch, calls));

  // the 4663 block every home read is made at: the lowest head, less a margin
  const heads = await Promise.all(home.map(async (r) => { try { await onChain(r, HOME_CHAIN_ID); return Number(BigInt(await r.raw<string>("eth_blockNumber", []))); } catch (e) { return msg(e); } }));
  const okHeads = heads.filter((h): h is number => typeof h === "number");
  // the indexer pins the block its event fold was made at, so the views and the fold describe the same moment
  const B = !okHeads.length ? null : o.block !== undefined ? Math.min(o.block, ...okHeads) : Math.min(...okHeads) - (o.margin ?? 20);
  let homeAt: number | null = null;
  if (B !== null) {
    const r0 = home[heads.findIndex((h) => typeof h === "number")];
    homeAt = await r0.raw<{ timestamp: string } | null>("eth_getBlockByNumber", [`0x${B.toString(16)}`, false]).then((b) => (b ? Number(BigInt(b.timestamp)) : null)).catch(() => null);
  }
  const wrapper: ChainRead<WrapperBooks>[] = await Promise.all(home.map(async (r, i): Promise<ChainRead<WrapperBooks>> => {
    if (typeof heads[i] === "string" || B === null) return { source: r.host, ok: false, error: typeof heads[i] === "string" ? String(heads[i]) : "no block" };
    try {
      const [s, h, t, e] = await Promise.all(["totalSupply", "held", "openTickets", "ethBacked"].map((f) => r.call<bigint>(o.wrapper, ETH_NFT_WRAPPER, f, [], B)));
      return { source: r.host, ok: true, value: { totalSupply: String(s), held: Number(h), openTickets: Number(t), ethBacked: String(e) }, block: B, at: homeAt };
    } catch (e) { return { source: r.host, ok: false, error: msg(e) }; }
  }));

  // the held items from the wrapper's own views: item(i) for every i, held while itemOf(tokenId) = i + 1
  const r0 = home[wrapper.findIndex((w) => w.ok)] ?? null;
  let ledger: { complete: boolean; items: LedgerItemIn[]; ticketsInFlight: number; note: string } = { complete: false, items: [], ticketsInFlight: 0, note: "no Robinhood Chain provider answered" };
  if (o.ledger) ledger = o.ledger;
  else if (r0 && B !== null) {
    try {
      const count = Number(await r0.call<bigint>(o.wrapper, ETH_NFT_WRAPPER, "itemCount", [], B));
      const all = await pool(Array.from({ length: count }, (_, i) => i), n, async (i): Promise<LedgerItemIn | null> => {
        const it = await r0.call<{ tokenId: bigint; sourceTx: Hex; deposit: boolean }>(o.wrapper, ETH_NFT_WRAPPER, "item", [BigInt(i)], B);
        const of = await r0.call<bigint>(o.wrapper, ETH_NFT_WRAPPER, "itemOf", [it.tokenId], B);
        return of === BigInt(i + 1) ? { itemId: i, tokenId: String(it.tokenId), how: it.deposit ? "deposit" : "purchase", arrivalTx: it.sourceTx, mintedAt: null, promisedTo: null } : null;
      });
      const open = (wrapper.find((w) => w.ok) as Extract<ChainRead<WrapperBooks>, { ok: true }>).value.openTickets;
      ledger = { complete: true, items: all.filter((x): x is LedgerItemIn => x !== null), ticketsInFlight: open, note: `the wrapper's views item(0…${count - 1}) and itemOf on ${r0.host}` };
    } catch (e) { ledger = { complete: false, items: [], ticketsInFlight: 0, note: `the item views on ${r0.host} failed: ${msg(e)}` }; }
  }

  const module: ChainRead<ModuleBooks>[] | null = o.module === null ? null : await Promise.all(home.map(async (r, i): Promise<ChainRead<ModuleBooks>> => {
    if (typeof heads[i] === "string" || B === null) return { source: r.host, ok: false, error: "no block" };
    try {
      const f = (fn: string) => r.call<bigint>(o.module!, FLOOR_MODULE, fn, [], B);
      const [E, R, C, T, esc, rev, adv, advAt] = await Promise.all(["ethBackedUnits", "reserveUnits", "netUnits", "transitUnits", "escrowBalance", "revenue", "advanced", "advancedAt"].map(f));
      const bal = await r.raw<string>("eth_getBalance", [o.module, `0x${B.toString(16)}`]);
      return { source: r.host, ok: true, block: B, at: homeAt, value: { address: o.module!, ethBackedUnits: String(E), reserveUnits: String(R), convertibleUnits: String(C), transitUnits: String(T),
        escrowWei: String(esc), revenueWei: String(rev), advancedWei: String(adv), advancedAt: advAt ? Number(advAt) : null, balanceWei: String(BigInt(bal)) } };
    } catch (e) { return { source: r.host, ok: false, error: msg(e) }; }
  }));

  // Ethereum: the finalized block on every provider, and every read at the lowest
  const finalized: ChainRead<number>[] = await Promise.all(eth.map(async (r): Promise<ChainRead<number>> => {
    try { await onChain(r, 1); const b = await r.raw<{ number: string; timestamp: string } | null>("eth_getBlockByNumber", ["finalized", false]);
      if (!b) throw new RpcError("no finalized block"); return { source: r.host, ok: true, value: Number(BigInt(b.number)), block: Number(BigInt(b.number)), at: Number(BigInt(b.timestamp)) }; }
    catch (e) { return { source: r.host, ok: false, error: msg(e) }; }
  }));
  const fins = finalized.filter((f) => f.ok) as Extract<ChainRead<number>, { ok: true }>[];
  const N = fins.length ? Math.min(...fins.map((f) => f.value)) : null;
  const owners = new Map<number, OwnerSaid[]>(), arrivals = new Map<number, ArrivalCheck>();
  await pool(ledger.items, n, async (it) => {
    owners.set(it.itemId, await Promise.all(eth.map(async (r): Promise<OwnerSaid> => {
      if (N === null) return { source: r.host, owner: null, reverted: false, error: "no finalized block" };
      try { return { source: r.host, owner: (await r.call<string>(o.collection, ERC721, "ownerOf", [BigInt(it.tokenId)], N)).toLowerCase() as Address, reverted: false, error: null }; }
      catch (e) { return { source: r.host, owner: null, reverted: e instanceof RpcError && e.reverted, error: msg(e) }; }
    })));
    if (o.arrivals !== false) {
      const reads: ArrivalRead[] = await Promise.all(eth.map(async (r): Promise<ArrivalRead> => {
        try {
          const receipt = await r.raw<RpcReceipt>("eth_getTransactionReceipt", [it.arrivalTx]);
          const tx = it.how === "deposit" ? await r.raw<RpcTx>("eth_getTransactionByHash", [it.arrivalTx]) : null;
          return { source: r.host, receipt, tx, error: receipt ? null : "no receipt" };
        } catch (e) { return { source: r.host, receipt: null, tx: null, error: msg(e) }; }
      }));
      arrivals.set(it.itemId, checkArrival(reads, { how: it.how, tx: it.arrivalTx, tokenId: it.tokenId, collection: o.collection, custodian: o.custodian, homeChainId: HOME_CHAIN_ID, wrapper: o.wrapper, recipient: o.recipients?.get(it.itemId) ?? null, finalized: N }));
    }
  });
  const custodianWei: ChainRead<string>[] = await Promise.all(eth.map(async (r): Promise<ChainRead<string>> => {
    if (N === null) return { source: r.host, ok: false, error: "no finalized block" };
    try { return { source: r.host, ok: true, value: String(BigInt(await r.raw<string>("eth_getBalance", [o.custodian, `0x${N.toString(16)}`]))), block: N, at: fins.find((f) => f.value === N)?.at ?? null }; }
    catch (e) { return { source: r.host, ok: false, error: msg(e) }; }
  }));

  const input: EthReserveInput = {
    now: now(), limits: LIMITS,
    meta: { source: { kind: "live", note: "Recomputed on this machine from the providers named on each number; no Rook service was asked." }, slug: o.slug, name: o.name, symbol: o.symbol,
      collection: o.collection, custodian: o.custodian, wrapper: o.wrapper, module: o.module, providers: { eth: o.ethRpcs.map(hostOf), home: o.homeRpcs.map(hostOf) }, lastGreenAt: null,
      links: { wrapper: null, custodian: `https://etherscan.io/address/${o.custodian}`, module: null, collection: `https://etherscan.io/address/${o.collection}` },
      nftUrl: (id) => `https://etherscan.io/nft/${o.collection}/${id}`, txUrl: (h) => `https://etherscan.io/tx/${h}` },
    wrapper, ledger, finalized, owners, arrivals, costs: new Map(), module, custodianWei,
    canister: null, canisterNote: "The self-check does not read the canister: its records are the canister's word, and none of this depends on them.", strays: [], relay: null,
  };
  return { input, calls, homeBlock: B, ethBlock: N };
}

/** The whole self-check: every read from the visitor's machine, then the same verdict the indexer computes. */
export async function checkReserves(o: ReservesCheckOpts): Promise<{ report: EthReserveReport; calls: CallLog[] }> {
  const r = await readReserves(o);
  return { report: computeEthReserve(r.input), calls: r.calls };
}

export type DrawCheckOpts = {
  wrapper: Address;
  /** the published proof: its block numbers say where to look; every field it asserts is read again or recomputed */
  proof: DrawProof;
  homeRpcs: string[]; ethRpcs: string[]; relays?: readonly string[];
  fetch: Fetch;
};
const RESOLVED = encodeEventTopics({ abi: ETH_NFT_WRAPPER, eventName: "Resolved" })[0];
const REDEEMED = encodeEventTopics({ abi: ETH_NFT_WRAPPER, eventName: "Redeemed" })[0];
const topicOf = (n: number) => `0x${n.toString(16).padStart(64, "0")}`;

/** One draw, re-derived from Robinhood Chain, drand's relays and Ethereum, against its published proof. */
export async function checkDraw(o: DrawCheckOpts): Promise<{ verdict: DrawVerdict; calls: CallLog[]; notes: string[] }> {
  const calls: CallLog[] = [], notes: string[] = [], p = o.proof;
  const home = o.homeRpcs.map((u) => reader(u, "robinhood", o.fetch, calls)), eth = o.ethRpcs.map((u) => reader(u, "ethereum", o.fetch, calls));
  const h = home[0];
  let onChainRead: { round: number | null; itemId: number | null; tokenId: string | null; source: string } | null = null;
  const proof: DrawProof = { ...p, beacon: { ...p.beacon }, posting: p.posting ? { ...p.posting } : null };
  try {
    await onChain(h, HOME_CHAIN_ID);
    const t = await h.call<{ stage: number; round: bigint; pinnedAt: bigint; itemId: bigint }>(o.wrapper, ETH_NFT_WRAPPER, "ticket", [BigInt(p.ticket)], "latest");
    onChainRead = { round: Number(t.round), itemId: null, tokenId: null, source: h.host };
    proof.burnAt = Number(t.pinnedAt);
    // the Resolved event, at the block the proof names; and whether another resolution precedes it in that block
    const logs = await h.raw<{ topics: string[]; data: string; logIndex: string; blockHash: string }[]>("eth_getLogs", [{ address: o.wrapper, topics: [RESOLVED], fromBlock: `0x${p.resolveBlock.toString(16)}`, toBlock: `0x${p.resolveBlock.toString(16)}` }]);
    const mine = logs.find((l) => l.topics[1] === topicOf(p.ticket));
    if (!mine) notes.push(`no Resolved event of ticket ${p.ticket} at block ${p.resolveBlock} on ${h.host}`);
    else {
      const e = decodeEventLog({ abi: ETH_NFT_WRAPPER, data: mine.data as VHex, topics: mine.topics as [VHex, ...VHex[]] }) as unknown as { args: { itemId: bigint; tokenId: bigint } };
      onChainRead = { ...onChainRead, itemId: Number(e.args.itemId), tokenId: String(e.args.tokenId) };
      const earlier = logs.filter((l) => Number(BigInt(l.logIndex)) < Number(BigInt(mine.logIndex)));
      if (earlier.length) notes.push(`another resolution precedes this one in block ${p.resolveBlock}: the set read at the block before is not this draw's`);
      else {
        // the set: _free at the block before, then every pending item stamped ≤ the ticket, as resolve moves them in
        const at = p.resolveBlock - 1;
        try {
          const fc = Number(await h.call<bigint>(o.wrapper, ETH_NFT_WRAPPER, "freeCount", [], at));
          const free = await pool(Array.from({ length: fc }, (_, i) => i), 4, async (i) => Number(await h.call<bigint>(o.wrapper, ETH_NFT_WRAPPER, "freeAt", [BigInt(i)], at)));
          const pc = Number(await h.call<bigint>(o.wrapper, ETH_NFT_WRAPPER, "pendingCount", [], at));
          const pend = await pool(Array.from({ length: pc }, (_, i) => i), 4, async (i) => h.call<readonly [bigint, bigint]>(o.wrapper, ETH_NFT_WRAPPER, "pendingAt", [BigInt(i)], at));
          const set = free.slice();
          for (const [itemId, stamp] of pend) { if (Number(stamp) > p.ticket) break; set.push(Number(itemId)); }
          proof.freeSetRead = set;
        } catch (e) { notes.push(`${h.host} could not serve the wrapper's state at block ${at} (${msg(e)}): the published set is compared with its own replay only`); proof.freeSetRead = p.freeSetRead; }
      }
    }
    // the burn: its block's timestamp and hash, and when its batch reached Ethereum (Nitro's NodeInterface, then Ethereum's block time)
    const burnLogs = await h.raw<{ topics: string[]; blockHash: string }[]>("eth_getLogs", [{ address: o.wrapper, topics: [REDEEMED, topicOf(p.ticket)], fromBlock: `0x${p.burnBlock.toString(16)}`, toBlock: `0x${p.burnBlock.toString(16)}` }]);
    if (!burnLogs.length) notes.push(`no Redeemed event of ticket ${p.ticket} at block ${p.burnBlock} on ${h.host}`);
    else {
      try {
        const batch = Number(await h.call<bigint>(NODE_INTERFACE, NITRO, "findBatchContainingBlock", [BigInt(p.burnBlock)], "latest"));
        const conf = Number(await h.call<bigint>(NODE_INTERFACE, NITRO, "getL1Confirmations", [burnLogs[0].blockHash], "latest"));
        const e0 = eth[0];
        const headB = await e0.raw<{ number: string }>("eth_getBlockByNumber", ["latest", false]);
        const parentBlock = Number(BigInt(headB.number)) - conf;
        const pb = await e0.raw<{ timestamp: string } | null>("eth_getBlockByNumber", [`0x${parentBlock.toString(16)}`, false]);
        proof.posting = { batch, parentBlock, postedAt: pb ? Number(BigInt(pb.timestamp)) : null,
          note: `batch ${batch}, ${conf} Ethereum confirmations as ${h.host} counts them, so Ethereum block ≈ ${parentBlock} (± a block: the two nodes' heads differ)` };
      } catch (e) { notes.push(`the batch that posted block ${p.burnBlock} could not be read (${msg(e)})`); proof.posting = null; }
    }
  } catch (e) { notes.push(`${h?.host ?? "no provider"}: ${msg(e)}`); }

  const relays = await fetchRound(p.round, o.relays ?? DRAND_RELAYS.slice(0, 2), o.fetch);
  for (const r of relays) calls.push({ chain: "drand", host: r.relay, method: "GET public/round", params: String(p.round), result: r.signature ? `${r.signature.slice(0, 18)}…` : r.error ?? "", ok: !!r.signature, ms: 0 });
  // the signature to check is the one the relays serve, not the proof's (the proof's is compared with it)
  const served = relays.find((r) => r.signature)?.signature ?? null;
  if (served) proof.beacon = { ...proof.beacon, signature: served, randomness: p.beacon.randomness };
  const verdict = verifyDraw(proof, { wrapper: o.wrapper, relays: relays.map((r) => ({ ...r, signature: r.signature })), onChain: onChainRead });
  if (p.beacon.signature && served && p.beacon.signature.toLowerCase() !== served.toLowerCase()) verdict.checks.push({ id: "published-signature", label: "the published signature is the relays'", ok: false, detail: "the proof carries other bytes than drand's relays serve" });
  if (verdict.checks.some((c) => c.ok === false)) verdict.verdict = "mismatch";
  return { verdict, calls, notes };
}
