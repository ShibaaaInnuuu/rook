import { readFileSync } from "node:fs";
import { ETH_NFT_WRAPPER, FLOOR_MODULE } from "./abis.js";
import { DRAND_RELAYS } from "./drand.js";
import { replay, vaultFromReplay } from "./draw.js";
import { computeEthReserve, LIMITS } from "./ethReserve.js";
import { reader, type Fetch, type Reader } from "./jsonrpc.js";
import { snapshotHash } from "./rarity.js";
import { beaconOf, decodeCosts, decodeVault, LogCache, postingOf, setFromViews, strayScan, viewsAt, type RawLog } from "./reads.js";
import { DEFAULT_ETH_RPCS, DEFAULT_HOME_RPCS, readReserves } from "./selfcheck.js";
import type { Address, CanisterRecords, DrawProof, DrawReport, EthReserveReport, Hex, MetadataSnapshot, SnapshotRef, SourceKind, VaultState } from "./types.js";

/**
 * docs/v8/proof-of-reserves.md §3 and redemption-draw.md §2.3: the indexer's v8 reports, one per configured Ethereum collection,
 * recomputed at most every `cadenceS` (P1: 10 min). Configured, never discovered (`V8_CONFIG=<file.json>`, parseV8Config).
 * Its providers default to the site's keyless pair; they must be independent of the canister's and the monitor's (P2).
 */
export type V8Collection = {
  slug: string; name: string; symbol: string;
  wrapper: Address; collection: Address; custodian: Address; module: Address | null;
  /** the first 4663 block to read the wrapper's events from */
  fromBlock: number;
  /** the metadata snapshot (a file written by scripts/snapshot-metadata.ts), where it is published, and where its hash was pinned */
  snapshot?: { file: string; url: string; pinnedIn: string } | null;
};
export type V8Config = { collections: V8Collection[]; homeRpcs?: string[]; ethRpcs?: string[]; relays?: string[]; cadenceS?: number; confirmations?: number; logChunk?: number; strayLookback?: number };
export type V8Deps = {
  config: V8Config;
  fetch: Fetch;
  now?: () => number;
  source?: SourceKind;
  /** P4: the canister's own records for a collection (ops/src/ethRecords.ts reads them); absent: "not read by this indexer" */
  canisterRecords?: ((c: V8Collection) => Promise<CanisterRecords | null>) | null;
  /** a snapshot already loaded (tests, fixtures); else read from `snapshot.file` */
  snapshots?: Map<string, MetadataSnapshot>;
  log?: (o: Record<string, unknown>) => void;
};

const ADDR = /^0x[0-9a-fA-F]{40}$/;
export function parseV8Config(raw: unknown): V8Config {
  const c = raw as V8Config;
  if (!c || !Array.isArray(c.collections)) throw new Error("a v8 config needs collections[]");
  const slugs = new Set<string>();
  for (const x of c.collections) {
    if (!/^[a-z0-9-]{1,40}$/.test(x.slug ?? "") || slugs.has(x.slug)) throw new Error(`bad or duplicate slug ${x.slug}`);
    slugs.add(x.slug);
    for (const k of ["wrapper", "collection", "custodian"] as const) if (!ADDR.test(x[k] ?? "")) throw new Error(`${x.slug}: bad ${k} address`);
    if (x.module !== null && !ADDR.test(x.module ?? "")) throw new Error(`${x.slug}: bad module address (null for none)`);
    if (!Number.isInteger(x.fromBlock) || x.fromBlock < 0) throw new Error(`${x.slug}: fromBlock`);
  }
  return c;
}
const LIVE: SourceKind = { kind: "live", note: "Read from Robinhood Chain, Ethereum and drand's relays by this indexer, from public keyless providers." };
const ETHERSCAN = { tx: (h: string) => `https://etherscan.io/tx/${h}`, nft: (c: string, id: string) => `https://etherscan.io/nft/${c}/${id}`, address: (a: string) => `https://etherscan.io/address/${a}` };
const RH_EXPLORER = "https://explorer.mainnet.chain.robinhood.com";

export class V8Service {
  private readonly logs: LogCache;
  private readonly ethLogs = new LogCache(500);
  private readonly times = new Map<number, number | null>();
  private readonly reserves = new Map<string, { at: number; r: EthReserveReport }>();
  private readonly drawReports = new Map<string, { at: number; r: DrawReport }>();
  private readonly lastGreen = new Map<string, number>();
  /** immutable once final: a resolved draw's proof fields, by wrapper and ticket */
  private readonly finalProofs = new Map<string, Pick<DrawProof, "beacon" | "freeSetRead" | "freeSetNote" | "posting">>();
  private readonly now: () => number;
  private readonly home: Reader[];
  private readonly eth: Reader[];
  constructor(private readonly d: V8Deps) {
    this.now = d.now ?? (() => Math.floor(Date.now() / 1000));
    this.logs = new LogCache(d.config.logChunk ?? 10_000);
    this.home = this.homeRpcs.map((u) => reader(u, "robinhood", d.fetch, null));
    this.eth = this.ethRpcs.map((u) => reader(u, "ethereum", d.fetch, null));
  }
  private get homeRpcs() { return this.d.config.homeRpcs ?? [...DEFAULT_HOME_RPCS]; }
  private get ethRpcs() { return this.d.config.ethRpcs ?? [...DEFAULT_ETH_RPCS]; }
  private get cadence() { return this.d.config.cadenceS ?? LIMITS.cadenceS; }
  private get source() { return this.d.source ?? LIVE; }
  slugs(): string[] { return this.d.config.collections.map((c) => c.slug); }
  private cfg(slug: string) { return this.d.config.collections.find((c) => c.slug === slug); }

  private async timeOf(block: number): Promise<number | null> {
    if (this.times.has(block)) return this.times.get(block)!;
    const b = await this.home[0].raw<{ timestamp: string } | null>("eth_getBlockByNumber", [`0x${block.toString(16)}`, false]).catch(() => null);
    const t = b ? Number(BigInt(b.timestamp)) : null;
    if (t !== null) { if (this.times.size > 50_000) this.times.clear(); this.times.set(block, t); }
    return t;
  }
  private async head(): Promise<number> { return Number(BigInt(await this.home[0].raw<string>("eth_blockNumber", []))) - (this.d.config.confirmations ?? 20); }
  private async vault(c: V8Collection, to: number) {
    const logs = await this.logs.logs(this.home[0], c.wrapper, ETH_NFT_WRAPPER, c.fromBlock, to);
    return { logs, decoded: await decodeVault(logs, (b) => this.timeOf(b)) };
  }

  snapshot(slug: string): { snapshot: MetadataSnapshot; ref: SnapshotRef } | null {
    const c = this.cfg(slug);
    if (!c?.snapshot && !this.d.snapshots?.has(slug)) return null;
    const s = this.d.snapshots?.get(slug) ?? (JSON.parse(readFileSync(c!.snapshot!.file, "utf8")) as MetadataSnapshot);
    return { snapshot: s, ref: { url: c?.snapshot?.url ?? `/v1/v8/snapshots/${slug}`, sha256: snapshotHash(s), pinnedIn: c?.snapshot?.pinnedIn ?? "not pinned", ipfsCid: s.source.ipfsCid,
      takenAt: s.takenAt, tokenCount: s.tokens.length, drift: null } };
  }

  async reserve(slug: string): Promise<EthReserveReport | null> {
    const c = this.cfg(slug); if (!c) return null;
    const hit = this.reserves.get(slug);
    if (hit && this.now() - hit.at < this.cadence) return hit.r;
    const now = this.now();
    let ledger: Parameters<typeof readReserves>[0]["ledger"], recipients = new Map<number, Address>();
    let costs = new Map<number, { advancedWei: string; costWei: string }>(), block: number | undefined;
    try {
      const B = await this.head(); block = B;
      const { decoded } = await this.vault(c, B);
      const r = replay(decoded.events, (e) => decoded.arrivals.get(e.itemId) ?? e.tx);
      const v = vaultFromReplay(r, B, null);
      ledger = { complete: r.faults.length === 0, items: v.items.map((i) => ({ itemId: i.itemId, tokenId: i.tokenId, how: i.how, arrivalTx: i.arrivalTx, mintedAt: i.mintedAt, promisedTo: i.promisedTo })),
        ticketsInFlight: v.openTickets, note: r.faults.length ? `the events replay with faults: ${r.faults.slice(0, 2).join("; ")}` : `the wrapper's events from block ${c.fromBlock} to ${B}` };
      recipients = decoded.recipients;
      if (c.module) costs = decodeCosts(await this.logs.logs(this.home[0], c.module, FLOOR_MODULE, c.fromBlock, B));
    } catch (e) { ledger = { complete: false, items: [], ticketsInFlight: 0, note: `the wrapper's events could not be read: ${String((e as Error)?.message ?? e).slice(0, 120)}` }; }
    const read = await readReserves({ slug: c.slug, name: c.name, symbol: c.symbol, wrapper: c.wrapper, collection: c.collection, custodian: c.custodian, module: c.module,
      homeRpcs: this.homeRpcs, ethRpcs: this.ethRpcs, fetch: this.d.fetch, now: this.now, margin: this.d.config.confirmations ?? 20, ledger, recipients, block });
    const canister = this.d.canisterRecords ? await this.d.canisterRecords(c).catch(() => null) : null;
    const held = new Set(read.input.ledger.items.map((i) => i.tokenId));
    const strays = read.ethBlock === null ? [] : await strayScan(this.eth[0], this.ethLogs, c.collection, c.custodian, read.ethBlock, this.d.config.strayLookback ?? 7_200, held,
      new Set(canister?.deposits.map((x) => x.tokenId) ?? []), async (b) => { const x = await this.eth[0].raw<{ timestamp: string } | null>("eth_getBlockByNumber", [`0x${b.toString(16)}`, false]).catch(() => null); return x ? Number(BigInt(x.timestamp)) : null; }, now).catch(() => []);
    const open = canister?.buys.find((b) => b.orderId && b.phase !== "Settled");
    const report = computeEthReserve({ ...read.input, now, costs, strays, canister,
      canisterNote: canister ? `The canister's own records (eth_nft_state(), an ICP ${canister.how}: ${canister.how === "query" ? "one replica's answer, not certified" : "a replicated call"}), read ${new Date(canister.readAt * 1000).toISOString()}. They explain; they never turn anything green.`
        : "The canister's own records are not read by this indexer.",
      relay: open?.orderId ? { orderId: open.orderId, status: null, url: `https://relay.link/transaction/${open.orderId}` } : null,
      meta: { ...read.input.meta, source: this.source, lastGreenAt: this.lastGreen.get(slug) ?? null,
        links: { wrapper: `${RH_EXPLORER}/address/${c.wrapper}`, custodian: ETHERSCAN.address(c.custodian), module: c.module ? `${RH_EXPLORER}/address/${c.module}` : null, collection: ETHERSCAN.address(c.collection) } } });
    if (report.status === "green") this.lastGreen.set(slug, now);
    this.reserves.set(slug, { at: now, r: report });
    return report;
  }

  async draws(slug: string): Promise<DrawReport | null> {
    const c = this.cfg(slug); if (!c) return null;
    const hit = this.drawReports.get(slug);
    if (hit && this.now() - hit.at < this.cadence) return hit.r;
    const now = this.now(), snap = this.snapshot(slug);
    const base: DrawReport = { source: this.source, slug, name: c.name, chainId: 1, collection: c.collection, wrapper: c.wrapper, beacon: null, computedAt: now, state: null, stateError: null, proofs: [],
      snapshot: snap?.ref ?? null, links: { wrapper: `${RH_EXPLORER}/address/${c.wrapper}`, tx: `${RH_EXPLORER}/tx/`, ethTx: "https://etherscan.io/tx/", ethNft: `https://etherscan.io/nft/${c.collection}/` } };
    try {
      const h = this.home[0], B = await this.head(), at = await this.timeOf(B);
      base.beacon = await h.call<Address>(c.wrapper, ETH_NFT_WRAPPER, "beacon", [], B).catch(() => null);
      const { logs, decoded } = await this.vault(c, B);
      const r = replay(decoded.events, (e) => decoded.arrivals.get(e.itemId) ?? e.tx);
      const fromReplay = vaultFromReplay(r, B, at);
      let state: VaultState = fromReplay;
      try {
        const v = await viewsAt(h, c.wrapper, B);
        state = { ...fromReplay, free: v.free, pending: v.pending, ticketCount: v.ticketCount, nextToResolve: v.nextToResolve, held: v.held, openTickets: v.openTickets };
        const same = (a: unknown, b: unknown) => JSON.stringify(a) === JSON.stringify(b);
        if (!same(v.free, fromReplay.free) || !same(v.pending, fromReplay.pending) || v.ticketCount !== fromReplay.ticketCount)
          base.stateError = "the wrapper's views and a replay of its events disagree about the draw set: the odds are not shown until they agree";
      } catch (e) { base.stateError = `the wrapper's views could not be read (${String((e as Error)?.message ?? e).slice(0, 100)}); the state is a replay of its events`; }
      if (r.faults.length) base.stateError = `the events replay with faults: ${r.faults.slice(0, 3).join("; ")}`;
      base.state = state;
      base.proofs = await this.proofs(c, r, decoded, logs);
    } catch (e) { base.stateError = `the wrapper could not be read: ${String((e as Error)?.message ?? e).slice(0, 120)}`; }
    this.drawReports.set(slug, { at: now, r: base });
    return base;
  }

  private async proofs(c: V8Collection, r: ReturnType<typeof replay>, d: Awaited<ReturnType<typeof decodeVault>>, logs: RawLog[]): Promise<DrawProof[]> {
    const burns = new Map(d.events.filter((e) => e.name === "Redeemed").map((e) => [(e as { ticket: number }).ticket, e as Extract<typeof e, { name: "Redeemed" }>]));
    const resolved = d.events.filter((e): e is Extract<typeof e, { name: "Resolved" }> => e.name === "Resolved").sort((a, b) => b.ticket - a.ticket).slice(0, 50);
    const hashOf = new Map(logs.map((l) => [Number(BigInt(l.blockNumber)), l.blockHash as Hex]));
    const out: DrawProof[] = [];
    for (const e of resolved) {
      const b = burns.get(e.ticket); if (!b) continue;
      const key = `${c.wrapper.toLowerCase()}:${e.ticket}`;
      let fin = this.finalProofs.get(key);
      if (!fin) {
        const beacon = await beaconOf(b.round, this.d.config.relays ?? DRAND_RELAYS.slice(0, 2), this.d.fetch);
        const firstInBlock = !d.events.some((x) => x.name === "Resolved" && x.block === e.block && x.logIndex < e.logIndex);
        let freeSetRead: number[] | null = null, freeSetNote = "the set is a replay of the wrapper's events";
        if (firstInBlock) {
          try { freeSetRead = await setFromViews(this.home[0], c.wrapper, e.block - 1, e.ticket); freeSetNote = `a replay of the wrapper's events, and its views at block ${e.block - 1}`; }
          catch (err) { freeSetNote = `a replay of the wrapper's events (the views at block ${e.block - 1} could not be read: ${String((err as Error)?.message ?? err).slice(0, 80)})`; }
        } else freeSetNote = "a replay of the wrapper's events (another resolution precedes it in its block, so the views at the block before are not its set)";
        const posting = hashOf.get(b.block) ? await postingOf(this.home[0], this.eth[0], b.block, hashOf.get(b.block)!) : null;
        fin = { beacon: { signature: beacon.signature, randomness: beacon.randomness, relays: beacon.relays }, freeSetRead, freeSetNote, posting };
        // cached for good only when complete: a signature not served yet, or a batch not posted yet, is asked again next time
        if (beacon.signature && posting?.postedAt) this.finalProofs.set(key, fin);
      }
      const del = d.delivered.get(e.ticket);
      out.push({ ticket: e.ticket, owner: b.owner, destination: b.destination, toBurn: b.destination.toLowerCase() === "0x000000000000000000000000000000000000dead", burnTx: b.tx, burnBlock: b.block,
        burnAt: b.at ?? 0, round: b.round, resolveTx: e.tx, resolveBlock: e.block, resolvedAt: e.at, beacon: fin.beacon, freeSet: r.setAt.get(e.ticket) ?? [], freeSetRead: fin.freeSetRead,
        freeSetNote: fin.freeSetNote, itemId: e.itemId, tokenId: e.tokenId, delivery: del ? { tx: del.txHash, url: ETHERSCAN.tx(del.txHash), at: del.at } : null, posting: fin.posting });
    }
    return out;
  }
}
