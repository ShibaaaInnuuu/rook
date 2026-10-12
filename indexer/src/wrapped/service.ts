import { decodeEventLog, encodeEventTopics, type Abi, type Hex } from "viem";
import type { Address, CustodyItem, GraduationTargets, HandinReport, HandinSubmission, KeeperState, RaffleDraw, ReserveReport, Ticket, WrappedActivity, WrappedCoin, WrappedCollection, WrappedList, WrappedSource, WrappedSummary, YoyoListing } from "../../types.js";
import { guardedFetch, type Fetch as OutFetch } from "../outbound.js";
import { IPFS_GATEWAYS } from "../rook/catalogSources.js";
import { checkPublished, foldDraws, LIST_MAX_BYTES } from "./draws.js";
import { hex, read, type Fetch, type RawLog, type Rpc } from "../rpc.js";
import { CURVE_READS, DRAW_EVENTS, QUEUE_READS, FACTORY_EVENTS, FACTORY_READS, NFT_WRAPPER_EVENTS, OFFER_BOARD_EVENTS, OFFER_BOARD_READS, REGISTRY_READS, REGISTRY_SWITCH_EVENTS, SWEEP_EVENTS, SWEEP_READS, TOKEN_READS, WRAPPER_EVENTS, WRAPPER_READS } from "./abis.js";
import { EVM_CHAINS, HOME_EXPLORER, V4_4663, type CollectionConfig, type WrappedConfig } from "./config.js";
import { readFloorModule, readProtocolLiquidity, readTreasury, registryOf } from "./floorModule.js";
import { computeInvariant, type WrapperRead } from "./invariant.js";
import { foldLedger, openState, type WrapperEvent } from "./ledger.js";
import { BTC_ADDRESS_PAGE, BTC_TX_PAGE, checkInscription, checkOwner, ORD_PAGE, pinBlock } from "./verify.js";

/**
 * docs/v5/ux-and-reserves.md §3. Reads each configured collection from its chains, folds the wrapper's events, checks every held
 * item at its public source, and computes the reserve invariant. One snapshot per collection per `ttlMs`; logs are read
 * incrementally. It reads; it has no key and sends nothing.
 */
const BITCOIN_BURN = "bc1p2zffkaxp5py4fdutfdsrt6t6tcrc5ks09rkfd428hlhf4n5q8tqqwv7ahk";   // Wrapper.BURN_ADDRESS
const EVM_BURN = "0x000000000000000000000000000000000000dEaD";                                 // NftWrapper.DEAD
const PHASES = ["trading", "graduating", "graduated", "refunding"] as const;
const LIVE: WrappedSource = { kind: "live", note: "Read from Robinhood Chain and from each collection's own chain by this indexer." };
const fmtEth = (wei: bigint) => { const E = 10n ** 18n, f = (wei % E).toString().padStart(18, "0").slice(0, 4).replace(/0+$/, ""); return `${wei / E}${f ? "." + f : ""}`; };

type Snapshot = { at: number; report: ReserveReport; collection: WrappedCollection; tickets: Map<number, Ticket>; handins: HandinReport };
type Vaults = { raffle: Address | null; burn: Address | null; yoyo: Address | null };
const ZERO = /^0x0{40}$/i;
const POLICY = ["raffle", "burn", "yoyo"] as const;
const HANDIN_STATUS = ["open", "open", "claimed", "slashed", "refunded"] as const;   // OfferBoard.Status: None, Open, Claimed, Slashed, Refunded
const SETTLED_KEPT = 100;
type LogCache = { upTo: number; logs: RawLog[] };

/** How long a caller waits for a collection's snapshot to be built (the guarded fetch's own deadline is 20 s). */
export const INFLIGHT_MS = 60_000;
const within = <T>(p: Promise<T>, ms: number, slug: string): Promise<T> => new Promise<T>((ok, no) => {
  const t = setTimeout(() => no(new Error(`the ${slug} snapshot took longer than ${Math.round(ms / 1000)} s to build`)), Math.max(0, ms));
  p.then((v) => { clearTimeout(t); ok(v); }, (e) => { clearTimeout(t); no(e); });
});
export type WrappedDeps = { config: WrappedConfig; rpc: Rpc | null; fetch: Fetch; now?: () => number; ttlMs?: number; inflightMs?: number; concurrency?: number;
  log?: (o: Record<string, unknown>) => void;
  /** fixes-W2-site §3: how a draw's published list is fetched (a URL the keeper names). Default: the E-12 guard (outbound.ts), 8 MB,
   *  20 s; a test passes its own; null: never fetched (the check then says "unreadable") */
  outbound?: OutFetch | null };

export class WrappedService {
  private readonly snaps = new Map<string, Snapshot>();
  /** a build in progress per collection, shared by the callers that arrive meanwhile; one older than `inflightMs` is not waited on (R3-check M-E) */
  private readonly inflight = new Map<string, { p: Promise<Snapshot>; at: number }>();
  private readonly logs = new Map<string, LogCache>();
  private readonly times = new Map<number, number | null>();
  private readonly lastGreen = new Map<string, number>();
  /** a draw list's verdict, by vault, draw and root: "matches" and "differs" are final, "unreadable" is asked again after 10 minutes */
  private readonly lists = new Map<string, { at: number; v: Pick<RaffleDraw, "published" | "claim"> }>();
  private readonly outbound: OutFetch | null;
  private readonly now: () => number;
  constructor(private readonly d: WrappedDeps) {
    this.now = d.now ?? (() => Math.floor(Date.now() / 1000));
    this.outbound = d.outbound === undefined ? guardedFetch({ maxBytes: LIST_MAX_BYTES, timeoutMs: 20_000 }) : d.outbound;
  }

  get configured(): boolean { return this.d.config.collections.length > 0; }
  private get source(): WrappedSource { return this.d.config.source ?? LIVE; }
  private cfg(slug: string): CollectionConfig | undefined { return this.d.config.collections.find((c) => c.slug === slug); }

  async list(): Promise<WrappedList> {
    const collections: WrappedSummary[] = [];
    for (const c of this.d.config.collections) {
      const s = await this.snapshot(c);
      const { slug, name, symbol, kind, wrapper, supplyUnits, itemsHeld, floorWei, floorFresh, status, coins } = s.collection;
      collections.push({ slug, name, symbol, kind, wrapper, supplyUnits, itemsHeld, floorWei, floorFresh, status, coins });
    }
    return { configured: this.configured, source: this.configured ? this.source : null, collections };
  }
  async collection(slug: string): Promise<WrappedCollection | null> { const c = this.cfg(slug); return c ? (await this.snapshot(c)).collection : null; }
  async reserves(): Promise<ReserveReport[]> { return Promise.all(this.d.config.collections.map(async (c) => (await this.snapshot(c)).report)); }
  async reserve(slug: string): Promise<ReserveReport | null> { const c = this.cfg(slug); return c ? (await this.snapshot(c)).report : null; }
  async ticket(slug: string, id: number): Promise<Ticket | null> { const c = this.cfg(slug); return c ? (await this.snapshot(c)).tickets.get(id) ?? null : null; }
  /** docs/v6/offer-handins.md: the collection's offer board, read; null for a slug that is not configured */
  async handins(slug: string): Promise<HandinReport | null> { const c = this.cfg(slug); return c ? (await this.snapshot(c)).handins : null; }

  private async snapshot(c: CollectionConfig): Promise<Snapshot> {
    const hit = this.snaps.get(c.slug);
    if (hit && (this.now() - hit.at) * 1000 < (this.d.ttlMs ?? 60_000)) return hit;
    // R3-check M-E: a build that never settles (a hostile draw URI once hung the transport) must not hang every later caller of
    // this collection: each waits at most `inflightMs`, and a build older than that is left behind and a new one started
    const limit = this.d.inflightMs ?? INFLIGHT_MS, t = Date.now();
    const running = this.inflight.get(c.slug);
    if (running && t - running.at < limit) return within(running.p, limit - (t - running.at), c.slug);
    const p = this.build(c).then((s) => { this.snaps.set(c.slug, s); return s; })
      .finally(() => { if (this.inflight.get(c.slug)?.p === p) this.inflight.delete(c.slug); });
    p.catch(() => undefined);
    this.inflight.set(c.slug, { p, at: t });
    return within(p, limit, c.slug);
  }

  // ---- reads on 4663 ---------------------------------------------------------------------------------------------------------
  private async head(): Promise<number> {
    const h = Number(BigInt(await this.d.rpc!<string>("eth_blockNumber", [])));
    return Math.max(0, h - (this.d.config.confirmations ?? 20));
  }
  private async timeOf(block: number): Promise<number | null> {
    if (this.times.has(block)) return this.times.get(block)!;
    const b = await this.d.rpc!<{ timestamp: Hex } | null>("eth_getBlockByNumber", [hex(block), false]).catch(() => null);
    const t = b ? Number(BigInt(b.timestamp)) : null;
    if (t !== null) this.times.set(block, t);
    if (this.times.size > 50_000) this.times.clear();
    return t;
  }
  /** Every log of `address` with one of `abi`'s event topics, from `from` to `to`, read in chunks and kept between snapshots. */
  private async logsOf(address: string, abi: Abi, from: number, to: number): Promise<RawLog[]> {
    const topics = (abi.filter((x) => x.type === "event") as { name: string }[]).map((e) => encodeEventTopics({ abi, eventName: e.name } as never)[0]);
    const key = `${address.toLowerCase()}|${topics.join(",")}`;
    const cache = this.logs.get(key) ?? { upTo: from - 1, logs: [] };
    const chunk = this.d.config.logChunk ?? 100_000;
    for (let a = cache.upTo + 1; a <= to; a += chunk) {
      const b = Math.min(to, a + chunk - 1);
      const got = await this.d.rpc!<RawLog[]>("eth_getLogs", [{ address, topics: [topics], fromBlock: hex(a), toBlock: hex(b) }]);
      cache.logs.push(...got.filter((l) => !l.removed));
      cache.upTo = b;
      this.logs.set(key, cache);
    }
    return cache.logs.filter((l) => Number(BigInt(l.blockNumber)) <= to);
  }
  private async decoded(address: string, abi: Abi, from: number, to: number): Promise<WrapperEvent[]> {
    const out: WrapperEvent[] = [];
    for (const l of await this.logsOf(address, abi, from, to)) {
      try {
        const e = decodeEventLog({ abi, data: l.data, topics: l.topics as [Hex, ...Hex[]] }) as unknown as { eventName: string; args: Record<string, unknown> };
        const block = Number(BigInt(l.blockNumber));
        out.push({ name: e.eventName, args: e.args, block, logIndex: Number(BigInt(l.logIndex)), tx: l.transactionHash, ts: await this.timeOf(block) });
      } catch { /* a log of another shape at the same address (e.g. an ERC-20 Transfer): not ours */ }
    }
    return out;
  }

  private async build(c: CollectionConfig): Promise<Snapshot> {
    const now = this.now(), maxAgeS = this.d.config.maxAgeS ?? 900;
    const explorer = HOME_EXPLORER[this.d.config.chainId] ?? null;
    const evm = c.custody.kind === "evm" ? EVM_CHAINS[c.custody.chainId] : undefined;
    const custody: ReserveReport["custody"] = c.custody.kind === "bitcoin"
      ? { kind: "bitcoin", address: c.custody.address, addressUrl: BTC_ADDRESS_PAGE(c.custody.address) }
      : { kind: "evm", chainId: c.custody.chainId, chainName: evm?.name ?? `chain ${c.custody.chainId}`, collection: c.custody.collection, custodian: c.custody.custodian,
        addressUrl: evm ? evm.address(c.custody.custodian) : null, providers: c.custody.providers ?? evm?.providers ?? [] };

    // the wrapper's books and its events, at one block
    let block: number | null = null, wrapper: WrapperRead = { ok: false, error: "no chain to read" }, events: WrapperEvent[] = [], ledgerComplete = false;
    let symbol = c.symbol;
    if (this.d.rpc) {
      try {
        block = await this.head();
        const tag = hex(block), at = await this.timeOf(block);
        const [supply, held, open] = await Promise.all([read<bigint>(this.d.rpc, c.wrapper, WRAPPER_READS, "totalSupply", [], tag),
          read<bigint>(this.d.rpc, c.wrapper, WRAPPER_READS, "held", [], tag), read<bigint>(this.d.rpc, c.wrapper, WRAPPER_READS, "openTickets", [], tag)]);
        wrapper = { ok: true, totalSupply: supply, held: Number(held), openTickets: Number(open), block, at };
        symbol = await read<string>(this.d.rpc, c.wrapper, WRAPPER_READS, "symbol", [], tag).catch(() => c.symbol);
      } catch (e) { wrapper = { ok: false, error: String((e as Error)?.message ?? e).slice(0, 160) }; }
      if (block !== null) {
        try { events = await this.decoded(c.wrapper, c.kind === "nft" ? NFT_WRAPPER_EVENTS : WRAPPER_EVENTS, c.fromBlock, block); ledgerComplete = true; }
        catch (e) { this.d.log?.({ level: "warn", event: "wrapped-logs", slug: c.slug, message: String((e as Error)?.message ?? e).slice(0, 160) }); }
      }
    }
    const ledger = foldLedger(c.kind, events, { btcTx: BTC_TX_PAGE, evmTx: evm ? evm.tx : null }, c.kind === "nft" ? EVM_BURN : BITCOIN_BURN, now,
      [c.queue, c.mintDesk].filter((x): x is Address => !!x));
    const { held, inFlight, promised } = openState(ledger);

    // every held item, checked where it lives
    const checks = await this.checkAll(c, held.map((h) => h.ref));
    const items: CustodyItem[] = held.map((h, i) => ({ itemId: h.itemId, ref: h.ref,
      refUrl: c.kind === "inscriptions" ? ORD_PAGE(h.ref) : evm && c.custody.kind === "evm" ? evm.nft(c.custody.collection, h.ref) : null,
      arrival: h.arrival.tx === null && checks[i].arrivalTx ? { ...h.arrival, tx: checks[i].arrivalTx, txUrl: BTC_TX_PAGE(checks[i].arrivalTx!) } : h.arrival,
      promisedTo: promised.get(h.itemId) ?? null, check: checks[i] }));

    const at = wrapper.ok ? wrapper.at : null;
    const floor = await readFloorModule(this.d.rpc, c.floorModule ?? null, c.wrapper, block ?? 0, at);
    // the treasury the module names (or the config's), and the protocol's own liquidity in the collection's lock
    const treasuryAddr = c.treasury ?? (floor.kind === "read" ? floor.treasury : null);
    const [treasury, protocolLiquidity] = await Promise.all([readTreasury(this.d.rpc, treasuryAddr, block ?? 0, at), readProtocolLiquidity(this.d.rpc, c.lock ?? null, block ?? 0, at)]);
    const inv = computeInvariant({ now, maxAgeS, wrapper, items, ticketsInFlight: inFlight.length, ledgerComplete, floor });
    if (inv.status === "green") this.lastGreen.set(c.slug, now);
    const report: ReserveReport = { source: this.source, slug: c.slug, name: c.name, symbol, kind: c.kind, wrapper: c.wrapper,
      wrapperUrl: explorer ? `${explorer}/address/${c.wrapper}` : null, custody, status: inv.status, reasons: inv.reasons, computedAt: now,
      lastGreenAt: this.lastGreen.get(c.slug) ?? null, block, equation: inv.equation, floorModule: floor, treasury, protocolLiquidity, items: inv.items, ticketsInFlight: inFlight };

    // the collection page: floor, coins, activity
    const vaults = await this.vaults(c, block);
    const [coinsList, sweepActivity, cap, draws, keeper, launchesOpen, graduation] = await Promise.all([this.coins(c, block, vaults), this.sweepActivity(c, block, vaults), this.cap(c, block),
      this.draws(c, block, vaults), this.keeper(c, block, floor.kind === "read" ? floor.address : c.floorModule ?? null), this.launchesOpen(c, block), this.graduation(c, block)]);
    // docs/v7/phase1-nft-pooled.md §3: the module's pool and the level a buy is due at (one item plus the buffer)
    const pool = floor.kind === "read" && floor.bufferUnits != null ? { units: floor.convertibleUnits, dueAtUnits: String(10n ** 18n + BigInt(floor.bufferUnits)) } : null;
    const activity: WrappedActivity[] = [...ledger.activity, ...sweepActivity].sort((a, b) => b.block - a.block).slice(0, 50);
    const collection: WrappedCollection = { source: this.source, slug: c.slug, name: c.name, symbol, kind: c.kind, wrapper: c.wrapper, chainId: this.d.config.chainId,
      description: c.description ?? "", custody, supplyUnits: wrapper.ok ? String(wrapper.totalSupply) : null, itemsHeld: wrapper.ok ? wrapper.held : null,
      floorWei: cap?.fresh ? String(cap.cap) : null, floorFresh: cap?.fresh ?? false, status: inv.status, coins: coinsList.length,
      contracts: { wrapper: c.wrapper, factory: c.factory ?? null, lock: c.lock ?? null, router: c.router ?? null, queue: c.queue ?? null, floorModule: c.floorModule ?? null,
        treasury: treasuryAddr, offerBoard: c.offerBoard ?? null, raffleVault: vaults.raffle, burnVault: vaults.burn, yoyoVault: vaults.yoyo,
        ...(this.d.config.chainId === 4663 ? V4_4663 : { poolManager: null, v4Quoter: null, stateView: null }) } as WrappedCollection["contracts"],
      coinsList, activity, openTickets: inFlight, market: this.d.config.market?.[c.slug] ?? null, launchesOpen, keeper, draws, graduation, pool };
    const handins = await this.handinReport(c, block, at, now, explorer);
    return { at: now, report, collection, tickets: ledger.tickets, handins };
  }

  /** The three policy vaults: the config's, else the factory's (`raffleVault`, `burnVault`, `yoyoVault`; zero = none). */
  private async vaults(c: CollectionConfig, block: number | null): Promise<Vaults> {
    const out: Vaults = { raffle: c.raffleVault ?? null, burn: c.burnVault ?? null, yoyo: c.yoyoVault ?? null };
    if (!this.d.rpc || !c.factory || block === null) return out;
    const tag = hex(block), get = (fn: string) => read<string>(this.d.rpc!, c.factory!, FACTORY_READS, fn, [], tag).then((a) => (ZERO.test(a) ? null : (a as Address))).catch(() => null);
    const [r, b, y] = await Promise.all([out.raffle ?? get("raffleVault"), out.burn ?? get("burnVault"), out.yoyo ?? get("yoyoVault")]);
    return { raffle: r, burn: b, yoyo: y };
  }

  // ---- fixes-B-C and fixes-A: the draws, the keeper switch, the launch gate -------------------------------------------------------
  /** The raffle vault's draws, newest first, each opened one with its published list checked against the root. */
  private async draws(c: CollectionConfig, block: number | null, vaults: Vaults): Promise<RaffleDraw[]> {
    if (!this.d.rpc || block === null || !vaults.raffle) return [];
    let draws: RaffleDraw[];
    try { draws = foldDraws(vaults.raffle, await this.decoded(vaults.raffle, DRAW_EVENTS, c.fromBlock, block)); }
    catch (e) { this.d.log?.({ level: "warn", event: "draw-logs", slug: c.slug, message: String((e as Error)?.message ?? e).slice(0, 160) }); return []; }
    const now = this.now();
    for (const d of draws) {
      if (!d.root || d.stage === "announced" || d.stage === "cancelled" || d.stage === "voided") continue;
      const key = `${d.vault.toLowerCase()}:${d.id}:${d.root}:${d.stage}`;
      const hit = this.lists.get(key);
      let v = hit && (hit.v.published?.status !== "unreadable" || now - hit.at < 600) ? hit.v : null;
      if (!v) {
        v = this.outbound ? await checkPublished(d, this.outbound, IPFS_GATEWAYS) : { published: { status: "unreadable", why: "this indexer fetches no lists" }, claim: null };
        this.lists.set(key, { at: now, v });
        if (this.lists.size > 1_000) this.lists.clear();
      }
      d.published = v.published; d.claim = d.stage === "resolved" ? v.claim : null;
    }
    return draws;
  }

  /** The registry's keeper switch at the report's block, and its last change. */
  private async keeper(c: CollectionConfig, block: number | null, module: Address | null): Promise<KeeperState | null> {
    if (!this.d.rpc || block === null) return null;
    const registry = c.registry ?? (module ? await registryOf(this.d.rpc, module, block) : null);
    if (!registry) return null;
    const tag = hex(block), rpc = this.d.rpc;
    const [paused, guardian] = await Promise.all([read<boolean>(rpc, registry, REGISTRY_READS, "keeperPaused", [], tag).catch(() => null),
      read<string>(rpc, registry, REGISTRY_READS, "guardian", [], tag).then((a) => (ZERO.test(a) ? null : (a as Address))).catch(() => null)]);
    const last = (await this.decoded(registry, REGISTRY_SWITCH_EVENTS, c.fromBlock, block).catch(() => [])).at(-1) ?? null;
    return { registry, paused, guardian, lastChange: last ? { paused: Boolean(last.args.paused), by: String(last.args.by) as Address, at: last.ts, tx: last.tx } : null };
  }

  /** docs/v7/phase1-nft-pooled.md §2: the factory's own graduation targets; null when there is no factory or it predates them. */
  private async graduation(c: CollectionConfig, block: number | null): Promise<GraduationTargets | null> {
    if (!this.d.rpc || !c.factory || block === null) return null;
    const tag = hex(block), rpc = this.d.rpc, factory = c.factory;
    try {
      const [n, e, u, m] = await Promise.all(["normalRaise", "earlyRaise", "earlyUntilEth", "maxRaise"].map((fn) => read<bigint>(rpc, factory, FACTORY_READS, fn, [], tag)));
      return { normalRaiseWei: String(n), earlyRaiseWei: String(e), earlyUntilEthWei: String(u), maxRaiseWei: String(m) };
    } catch { return null; }
  }

  /** `CurveFactory.launchesOpen()` (A-03): false while the queue has no room for a new coin's item budget. */
  private async launchesOpen(c: CollectionConfig, block: number | null): Promise<boolean | null> {
    if (!this.d.rpc || !c.factory || block === null) return null;
    return read<boolean>(this.d.rpc, c.factory, FACTORY_READS, "launchesOpen", [], hex(block)).catch(() => null);
  }

  // ---- docs/v6/offer-handins.md: the offer board ---------------------------------------------------------------------------------
  private async handinReport(c: CollectionConfig, block: number | null, at: number | null, now: number, explorer: string | null): Promise<HandinReport> {
    const empty: HandinReport = { source: this.source, slug: c.slug, name: c.name, board: c.offerBoard ?? null, boardUrl: c.offerBoard && explorer ? `${explorer}/address/${c.offerBoard}` : null,
      params: null, custodian: null, rewardPoolWei: null, paidTodayWei: null, open: [], settled: [],
      totals: { submitted: 0, open: 0, claimed: 0, slashed: 0, refunded: 0, rewardsPaidWei: "0", bondsSlashedWei: "0", fundedWei: "0" }, computedAt: now, block, error: null };
    if (!c.offerBoard) return { ...empty, error: c.kind === "nft" ? "hand-ins are for Bitcoin collections: an NFT collection has no offer board" : "no offer board is configured for this collection" };
    if (!this.d.rpc || block === null) return { ...empty, error: "no chain to read" };
    const rpc = this.d.rpc, board = c.offerBoard, tag = hex(block);
    const r = <T,>(fn: string, args: unknown[] = []) => read<T>(rpc, board, OFFER_BOARD_READS, fn, args, tag);
    try {
      const [p, perSubmitter, custodian, pool, paidToday, count] = await Promise.all([r<readonly bigint[]>("params"), r<bigint>("maxOpenPerSubmitter"), r<string>("custodian"),
        r<bigint>("rewardPool"), r<bigint>("paidToday"), r<bigint>("submissionCount")]);
      const ids = await r<readonly bigint[]>("openIds", [0n, p[5]]);
      const events = await this.decoded(board, OFFER_BOARD_EVENTS, c.fromBlock, block);
      const settledBy = new Map<number, { tx: Hex; ts: number | null; reward: bigint | null }>();
      let claimed = 0, slashed = 0, refunded = 0, rewards = 0n, slashedBonds = 0n, funded = 0n;
      for (const e of events) {
        if (e.name === "Claimed") { claimed++; rewards += BigInt(e.args.reward as bigint); settledBy.set(Number(e.args.id), { tx: e.tx, ts: e.ts, reward: BigInt(e.args.reward as bigint) }); }
        else if (e.name === "Slashed") { slashed++; slashedBonds += BigInt(e.args.bond as bigint); settledBy.set(Number(e.args.id), { tx: e.tx, ts: e.ts, reward: null }); }
        else if (e.name === "Refunded") { refunded++; settledBy.set(Number(e.args.id), { tx: e.tx, ts: e.ts, reward: null }); }
        else if (e.name === "Funded") funded += BigInt(e.args.amount as bigint);
      }
      const sub = async (id: number): Promise<HandinSubmission> => {
        const s = await r<{ submitter: string; at: bigint; atBlock: bigint; status: number; bond: bigint; askSats: bigint; offerHash: Hex; inscriptionId: string }>("submissionOf", [BigInt(id)]);
        const status = HANDIN_STATUS[Number(s.status)] ?? "open";
        const [slashable, expired] = status === "open" ? await Promise.all([r<boolean>("slashable", [BigInt(id)]), r<boolean>("expired", [BigInt(id)])]) : [false, false];
        const done = settledBy.get(id) ?? null;
        return { id, submitter: s.submitter as Address, status, at: Number(s.at), atBlock: Number(s.atBlock), bondWei: String(s.bond), askSats: String(s.askSats),
          offerHash: s.offerHash, inscriptionId: s.inscriptionId, slashable, expired, rewardPaidWei: done?.reward !== null && done?.reward !== undefined ? String(done.reward) : null,
          settledTx: done?.tx ?? null, settledAt: done?.ts ?? null };
      };
      const open = (await Promise.all(ids.map((i) => sub(Number(i))))).sort((a, b) => Number(BigInt(a.askSats) - BigInt(b.askSats)) || a.id - b.id);
      const settledIds = [...settledBy.keys()].sort((a, b) => b - a).slice(0, SETTLED_KEPT);
      const settled = await Promise.all(settledIds.map(sub));
      return { ...empty, params: { bondWei: String(p[0]), rewardWei: String(p[1]), slashDelayS: Number(p[2]), expiryS: Number(p[3]), rewardDayCapWei: String(p[4]), maxOpen: Number(p[5]), maxOpenPerSubmitter: Number(perSubmitter) },
        custodian: custodian as Address, rewardPoolWei: String(pool), paidTodayWei: String(paidToday), open, settled,
        totals: { submitted: Number(count), open: open.length, claimed, slashed, refunded, rewardsPaidWei: String(rewards), bondsSlashedWei: String(slashedBonds), fundedWei: String(funded) }, computedAt: at ?? now };
    } catch (e) { return { ...empty, error: String((e as Error)?.message ?? e).slice(0, 160) }; }
  }

  private async checkAll(c: CollectionConfig, refs: string[]) {
    const n = this.d.concurrency ?? 4;
    const out: Awaited<ReturnType<typeof checkInscription>>[] = new Array(refs.length);
    if (c.custody.kind === "bitcoin") {
      const custody = c.custody.address;
      for (let i = 0; i < refs.length; i += n)
        (await Promise.all(refs.slice(i, i + n).map((r) => checkInscription(r, custody, { fetch: this.d.fetch, now: this.now })))).forEach((x, j) => (out[i + j] = x));
      return out;
    }
    const cu = c.custody, evm = EVM_CHAINS[cu.chainId];
    const src = { fetch: this.d.fetch, chainId: cu.chainId, providers: cu.providers ?? evm?.providers ?? [], now: this.now };
    const pin = refs.length ? await pinBlock(src) : { ok: false as const, error: "nothing to check" };
    for (let i = 0; i < refs.length; i += n)
      (await Promise.all(refs.slice(i, i + n).map((r) => checkOwner(r, cu.collection, cu.custodian, pin, src, (id) => evm ? evm.nft(cu.collection, id) : null)))).forEach((x, j) => (out[i + j] = x));
    return out;
  }

  private async cap(c: CollectionConfig, block: number | null): Promise<{ cap: bigint; fresh: boolean } | null> {
    if (!this.d.rpc || !c.factory || block === null) return null;
    try { const [cap, fresh] = await read<readonly [bigint, boolean]>(this.d.rpc, c.factory, FACTORY_READS, "freshCap", [], hex(block)); return { cap, fresh }; } catch { return null; }
  }

  private async coins(c: CollectionConfig, block: number | null, vaults: Vaults): Promise<WrappedCoin[]> {
    if (!this.d.rpc || !c.factory || block === null) return [];
    const rpc = this.d.rpc, tag = hex(block), factory = c.factory;
    const blockTs = await this.timeOf(block);
    try {
      const launched = await this.decoded(factory, FACTORY_EVENTS, c.fromBlock, block);
      // fixes-R3-contracts §2 (N-3): the queue's refund hold (the module: keeper paused); a queue built before round 3 answers nothing
      const queue = c.queue ?? c.floorModule ?? null;
      const refundsHeld = queue ? await read<boolean>(rpc, queue, QUEUE_READS, "holdsRefunds", [], tag).catch(() => null) : null;
      const yoyoEvents = vaults.yoyo ? await this.decoded(vaults.yoyo, SWEEP_EVENTS, c.fromBlock, block).catch(() => []) : [];
      return await Promise.all(launched.reverse().map(async (e): Promise<WrappedCoin> => {
        const coin = String(e.args.coin) as Address, curve = String(e.args.curve) as Address;
        const cr = <T,>(fn: string) => read<T>(rpc, curve, CURVE_READS, fn, [], tag).catch(() => null);
        const [name, sym, state, progress, feeOf, feeBps, lfBps, lfDecay, launchedAt, extra] = await Promise.all([read<string>(rpc, coin, TOKEN_READS, "name", [], tag).catch(() => "unnamed"),
          read<string>(rpc, coin, TOKEN_READS, "symbol", [], tag).catch(() => "?"), cr<number>("state"), cr<readonly [bigint, bigint]>("progress"),
          read<bigint>(rpc, factory, FACTORY_READS, "feeOf", [coin], tag).catch(() => null), cr<bigint>("feeBps"), cr<bigint>("launchFeeBps"), cr<bigint>("launchFeeDecay"), cr<bigint>("launchedAt"), cr<bigint>("launchExtraBps")]);
        // fixes-A-graduation A-03 / A-01: the close gate and the escape hatch; a curve built before the fixes answers neither
        const [canClose, canGraduate, opensAt] = await Promise.all([cr<boolean>("canClose"), cr<boolean>("canGraduate"), cr<readonly [bigint, bigint]>("refundsOpenAt")]);
        const policy = POLICY[Number(e.args.policy)] ?? "raffle";
        const launchFee = lfBps && lfBps > 0n && lfDecay ? { bps: Number(lfBps), decayS: Number(lfDecay), extraBpsNow: Number(extra ?? 0n),
          endsAt: launchedAt ? Number(launchedAt) + Number(lfDecay) : e.ts !== null ? e.ts + Number(lfDecay) : null } : null;
        let yoyo: WrappedCoin["yoyo"] = null;
        if (policy === "yoyo" && vaults.yoyo) {
          const vr = <T,>(fn: string, args: unknown[] = []) => read<T>(rpc, vaults.yoyo!, SWEEP_READS, fn, args, tag).catch(() => null);
          const [openN, proceeds, markup] = await Promise.all([vr<bigint>("listedOpen", [coin]), vr<bigint>("proceeds", [coin]), vr<bigint>("markupBps")]);
          const mine = yoyoEvents.filter((y) => String(y.args.coin).toLowerCase() === coin.toLowerCase());
          const listings = new Map<number, YoyoListing>();
          let boughtBack = 0n, burned = 0n;
          for (const y of mine) {
            const tid = Number(y.args.ticketId);
            if (y.name === "Relisted") listings.set(tid, { ticketId: tid, stage: "listed", askWei: String(y.args.ask), proceedsWei: null, at: y.ts, tx: y.tx });
            else if (y.name === "SaleRecorded") { const l = listings.get(tid); if (l) { l.stage = "sold"; l.proceedsWei = String(y.args.proceeds); } }
            else if (y.name === "ListingReturned") { const l = listings.get(tid); if (l) l.stage = "returned"; }
            else if (y.name === "BoughtBack") { boughtBack += BigInt(y.args.spent as bigint); burned += BigInt(y.args.burned as bigint); }
          }
          yoyo = { openListings: Number(openN ?? 0n), proceedsWei: String(proceeds ?? 0n), boughtBackWei: String(boughtBack), burnedCoins: String(burned), markupBps: Number(markup ?? 0n),
            listings: [...listings.values()].sort((a, b) => b.ticketId - a.ticketId).slice(0, 20) };
        }
        void blockTs;
        return { address: coin, curve, name, symbol: sym, creator: String(e.args.creator) as Address, policy,
          sweepOnly: Boolean(e.args.sweepOnly), phase: state === null ? "trading" : PHASES[Number(state)] ?? "trading", raiseWei: String(e.args.raise),
          raisedWei: progress ? String(progress[1]) : null, launchedAt: e.ts, feeBps: Number(feeOf ?? feeBps ?? 0n), launchFee, yoyo,
          canClose, canGraduate, refundsOpenAt: opensAt && opensAt[0] > 0n ? Number(opensAt[0]) : null, refundsHeld };
      }));
    } catch { return []; }
  }

  private async sweepActivity(c: CollectionConfig, block: number | null, vaults: Vaults): Promise<WrappedActivity[]> {
    if (!this.d.rpc || !c.factory || block === null) return [];
    try {
      const out: WrappedActivity[] = [];
      for (const v of [vaults.raffle, vaults.burn, vaults.yoyo]) {
        if (!v) continue;
        for (const e of await this.decoded(v, SWEEP_EVENTS, c.fromBlock, block)) {
          const base = { at: e.ts, block: e.block, tx: e.tx };
          if (e.name === "Won") out.push({ ...base, kind: "raffle-won", text: `raffle ${String(e.args.drawId)} won by ${String(e.args.holder).slice(0, 8)}…` });
          else if (e.name === "Retired") out.push({ ...base, kind: "burned", text: `a swept item retired for good (ticket ${String(e.args.ticketId)})` });
          else if (e.name === "Relisted") out.push({ ...base, kind: "relisted", text: `a swept item put back on the market at ${fmtEth(e.args.ask as bigint)} ETH (ticket ${String(e.args.ticketId)})` });
          else if (e.name === "SaleRecorded") out.push({ ...base, kind: "sold", text: `a relisted item sold for ${fmtEth(e.args.proceeds as bigint)} ETH (ticket ${String(e.args.ticketId)})` });
          else if (e.name === "BoughtBack") out.push({ ...base, kind: "bought-back", text: `${fmtEth(e.args.spent as bigint)} ETH of sale proceeds bought the coin back and burned it` });
          else if (e.name === "ListingReturned") out.push({ ...base, kind: "returned", text: `an unsold item came back to the vault (ticket ${String(e.args.ticketId)})` });
        }
      }
      return out;
    } catch { return []; }
  }
}
