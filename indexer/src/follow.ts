import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import type { DatabaseSync } from "node:sqlite";
import { decodeEventLog, decodeFunctionData, toEventSelector, type Abi, type Hex } from "viem";
import { EVENTS, LAUNCHER_CALLS, READS, type ContractKind } from "./abis.js";
import { cleanLogo } from "./logo.js";
import { getMeta, setMeta } from "./db.js";
import { apply, foldOrder, lotOf, rebuild, type Ev } from "./fold.js";
import { hex, tryRead, RpcError, type RawBlock, type RawLog, type Rpc } from "./rpc.js";
import { refreshRate, USD_POOL } from "./usd.js";

/** docs/phase4.md §2. What the follower needs of a deployment file; the rest of the file is ignored. */
export type Deployment = { chainId: number; registry: string; launcher: string; distributor: string; ponsFactory: string | null; fromBlock: number; demo: boolean };

/** Null when there is no usable file for this chain: the follower then idles and the API says `deployment: false`. */
export function loadDeployment(dir: string, chainId: number): Deployment | null {
  const path = join(dir, `${chainId}.json`);
  if (!existsSync(path)) return null;
  let j: Record<string, unknown>;
  try { j = JSON.parse(readFileSync(path, "utf8")); } catch { return null; }
  const addr = (k: string) => (typeof j[k] === "string" && /^0x[0-9a-fA-F]{40}$/.test(j[k] as string) ? (j[k] as string).toLowerCase() : null);
  const registry = addr("registry"), launcher = addr("launcher"), distributor = addr("distributor");
  // `fromBlock` is the L2 height eth_getLogs understands. `deployedAtBlock` is what block.number said INSIDE a contract, which on an
  // Arbitrum chain is the L1 height: never a place to start reading logs from.
  const fromBlock = Number(j.fromBlock);
  if (!registry || !launcher || !distributor || !Number.isInteger(fromBlock) || fromBlock < 0 || Number(j.chainId) !== chainId) return null;
  return { chainId, registry, launcher, distributor, ponsFactory: addr("ponsFactory"), fromBlock, demo: j.demo === true };
}

/** docs/phase10.md: the two v4 topics the follower asks for by name. Computed, never pasted. */
/** docs/phase12.md §1: how long a coin whose `totalSupply()` would not read waits before the follower asks it again. */
export const SUPPLY_RETRY_S = 3_600;

export const INITIALIZE = toEventSelector("Initialize(bytes32,address,address,uint24,int24,address,uint160,int24)");
export const SWAP = toEventSelector("Swap(bytes32,address,int128,int128,uint160,uint128,int24,uint24)");
/** docs/phase13.md §4: the coins' own ERC-20 topic, asked for by name over a block range that is behind the cursor. */
export const TRANSFER = toEventSelector("Transfer(address,address,uint256)");
/**
 * docs/phase13.md §4. How much history a single pass will fetch transfers for, in ranges of `maxRange`.
 *
 * `Transfer` indexing begins wherever the cursor happens to be, so every coin already on the chain needs its earlier
 * transfers fetched once or its balance is only the tail of its history. Two ranges a pass — 4,000 blocks at the default —
 * walks the staging chain's whole life in three passes and then never runs again. History is fetched ONCE, not every pass.
 */
export const BACKFILL_RANGES = 2;

const isAddress = (v: unknown): v is string => typeof v === "string" && /^0x[0-9a-fA-F]{40}$/.test(v);
export const normalise = (args: Record<string, unknown>): Ev["args"] => Object.fromEntries(Object.entries(args).map(([k, v]) =>
  [k, typeof v === "bigint" ? String(v) : isAddress(v) ? v.toLowerCase() : (v as string | number | boolean)]));

export function decode(kind: ContractKind, log: RawLog): { name: string; args: Ev["args"] } | null {
  try {
    const d = decodeEventLog({ abi: EVENTS[kind] as Abi, data: log.data, topics: log.topics as [Hex, ...Hex[]] });
    return { name: String(d.eventName), args: normalise((d.args ?? {}) as unknown as Record<string, unknown>) };
  } catch { return null; }   // a topic that is not one of ours, from an address that is: ignored
}

export type FollowerState = { lastPassAt: number | null; lastError: { at: number; kind: string } | null; head: number | null; reorgs: number; idle: boolean };
export type FollowerDeps = { db: DatabaseSync; rpc: Rpc; deployment: Deployment | null; confirmations: number; maxRange?: number; log?: (o: Record<string, unknown>) => void;
  now?: () => number;
  /** docs/phase12.md §3: the Uniswap v3 pool the dollar rate is read from. null on a chain that has none, and then there is no dollar rate at all. */
  usdPool?: string | null };

export class Follower {
  readonly state: FollowerState = { lastPassAt: null, lastError: null, head: null, reorgs: 0, idle: false };
  private readonly maxRange: number;
  private readonly now: () => number;
  constructor(private readonly d: FollowerDeps) {
    this.maxRange = d.maxRange ?? 2_000;
    this.now = d.now ?? (() => Math.floor(Date.now() / 1000));
    this.state.idle = d.deployment === null;
    // docs/phase10.md: a database whose derived tables openDb had to drop is refolded from `events` here, before the first pass.
    if (getMeta(d.db, "rebuild_derived") === "1") { rebuild(d.db); setMeta(d.db, "rebuild_derived", "0"); }
    if (d.deployment) {
      setMeta(d.db, "chain_id", String(d.deployment.chainId)); setMeta(d.db, "demo", d.deployment.demo ? "1" : "0");
      // docs/phase13.md §3: the factory, so a view can exclude Pons's own 2/7 of a coin's supply from its holder list
      // without asking the chain who Pons is. It is in the deployment file and in no event.
      if (d.deployment.ponsFactory) setMeta(d.db, "pons_factory", d.deployment.ponsFactory);
      /**
       * docs/phase13.md §4. The two numbers that say which blocks' transfers are indexed, written once.
       *
       * `transfers_live_from` is the block the forward walk will carry transfers from — the cursor as this process found it.
       * `transfers_backfilled_through` starts one block before the deployment and walks up to meet it, a bounded piece a
       * pass. Balances are complete exactly when the second has reached the first; on a fresh database they are equal at
       * birth, and there is nothing to backfill.
       */
      if (getMeta(d.db, "transfers_live_from") === null) {
        const start = d.deployment.fromBlock - 1;
        setMeta(d.db, "transfers_live_from", String(Math.max(start, this.cursor() ?? start)));
        setMeta(d.db, "transfers_backfilled_through", String(start));
      }
    }
  }
  private log(o: Record<string, unknown>) { this.d.log?.(o); }
  cursor(): number | null { const c = getMeta(this.d.db, "cursor"); return c === null ? null : Number(c); }

  private vaults(): Set<string> { return new Set((this.d.db.prepare(`SELECT vault FROM listings`).all() as { vault: string }[]).map((r) => r.vault)); }
  private routers(): Set<string> { return new Set((this.d.db.prepare(`SELECT router FROM coins`).all() as { router: string }[]).map((r) => r.router)); }
  /** docs/phase10.md: Pons's bonding curve, one per coin, learnt from our own `Launched`. Its trades are the chart. */
  private curves(): Set<string> { return new Set((this.d.db.prepare(`SELECT curve FROM coins`).all() as { curve: string }[]).map((r) => r.curve)); }
  private coinList(): string[] { return (this.d.db.prepare(`SELECT coin FROM coins`).all() as { coin: string }[]).map((r) => r.coin); }
  private poolIds(): string[] { return (this.d.db.prepare(`SELECT pool_id FROM pools`).all() as { pool_id: string }[]).map((r) => r.pool_id); }

  private async block(n: number): Promise<RawBlock> {
    const b = await this.d.rpc<RawBlock | null>("eth_getBlockByNumber", [hex(n), false]);
    if (!b) throw new RpcError("rpc", `block ${n} is not there`);
    return b;
  }

  /** The cursor block's hash still matches the node's? If not, fall back to the newest block we hold that it still agrees with. */
  private async checkReorg(): Promise<void> {
    const db = this.d.db, cursor = this.cursor(), stored = getMeta(db, "cursor_hash");
    if (cursor === null || stored === null) return;
    if ((await this.block(cursor)).hash.toLowerCase() === stored) return;
    const held = db.prepare(`SELECT number, hash FROM blocks WHERE number <= ? ORDER BY number DESC LIMIT 512`).all(cursor) as { number: number; hash: string }[];
    let keep = (this.d.deployment?.fromBlock ?? 0) - 1, keepHash: string | null = null;
    for (const h of held) {
      if ((await this.block(h.number)).hash.toLowerCase() === h.hash) { keep = h.number; keepHash = h.hash; break; }
    }
    db.exec("BEGIN");
    try {
      db.prepare(`DELETE FROM events WHERE block_number > ?`).run(keep);
      db.prepare(`DELETE FROM blocks WHERE number > ?`).run(keep);
      setMeta(db, "cursor", String(keep));
      if (keepHash) setMeta(db, "cursor_hash", keepHash); else db.prepare(`DELETE FROM meta WHERE key = 'cursor_hash'`).run();
      const n = rebuild(db);
      db.exec("COMMIT");
      this.state.reorgs++;
      this.log({ level: "warn", event: "reorg", from: cursor, keptThrough: keep, eventsKept: n });
    } catch (err) { db.exec("ROLLBACK"); throw err; }
  }

  private async logs(addresses: string[], from: number, to: number): Promise<RawLog[]> {
    const out: RawLog[] = [];
    for (let i = 0; i < addresses.length; i += 200) {
      out.push(...await this.d.rpc<RawLog[]>("eth_getLogs", [{ address: addresses.slice(i, i + 200), fromBlock: hex(from), toBlock: hex(to) }]));
    }
    return out.filter((l) => !l.removed);
  }

  /**
   * docs/phase10.md: the v4 PoolManager is a chain-wide contract that thousands of pools share, so it is never read by address
   * alone. Every pool read is filtered by topic — the pools that are ours, or the coins that are — and `topics` positions are
   * the event's indexed arguments in order, with null for "any".
   */
  private async byTopic(address: string | string[], topics: (string | string[] | null)[], from: number, to: number): Promise<RawLog[]> {
    return (await this.d.rpc<RawLog[]>("eth_getLogs", [{ address, topics, fromBlock: hex(from), toBlock: hex(to) }])).filter((l) => !l.removed);
  }

  /** The PoolManager the Pons factory itself names. Read once and kept: it is a property of the chain, not of an event. */
  private async poolManager(): Promise<string | null> {
    const held = getMeta(this.d.db, "pool_manager");
    if (held !== null) return held === "" ? null : held;
    const dep = this.d.deployment!;
    if (!dep.ponsFactory) return null;
    const pm = await tryRead<string>(this.d.rpc, dep.ponsFactory, READS as Abi, "poolManager");
    if (pm === null) return null;                            // not readable this pass: ask again next pass rather than remember a guess
    setMeta(this.d.db, "pool_manager", pm.toLowerCase());
    return pm.toLowerCase();
  }

  /**
   * A range's pool logs: the `Initialize` that says which pool belongs to one of our coins, then the `Swap`s in the pools we
   * already know. Two topic reads and one more, all filtered, so the PoolManager's own traffic is never fetched. A coin's
   * address sits in the second or the third indexed argument depending on which currency sorted first, so both are asked.
   */
  private async poolLogs(from: number, to: number, coins: string[]): Promise<RawLog[]> {
    if (!coins.length) return [];
    const pm = await this.poolManager();
    if (!pm) return [];
    const topic = (a: string) => `0x${"0".repeat(24)}${a.replace(/^0x/, "").toLowerCase()}`;
    const out: RawLog[] = [];
    for (let i = 0; i < coins.length; i += 100) {
      const some = coins.slice(i, i + 100).map(topic);
      out.push(...await this.byTopic(pm, [INITIALIZE, null, some], from, to));
      out.push(...await this.byTopic(pm, [INITIALIZE, null, null, some], from, to));
    }
    const pools = this.poolIds();
    for (let i = 0; i < pools.length; i += 100) out.push(...await this.byTopic(pm, [SWAP, pools.slice(i, i + 100)], from, to));
    return out;
  }

  /** What an event alone does not say and a fold cannot recover: read once, kept outside the derived tables. */
  private async fetchMeta(evs: { kind: ContractKind; name: string; args: Ev["args"]; log: RawLog }[]): Promise<void> {
    const dep = this.d.deployment!, db = this.d.db;
    for (const e of evs) {
      if (e.kind === "registry" && e.name === "Listed") {
        const l = await tryRead<{ divisibility: number; membershipRoot: string; btcVault: string }>(this.d.rpc, dep.registry, READS as Abi, "listing", [BigInt(String(e.args.id))]);
        if (l) db.prepare(`INSERT OR REPLACE INTO listing_meta (id, divisibility, btc_vault, root) VALUES (?, ?, ?, ?)`).run(Number(e.args.id), Number(l.divisibility), l.btcVault, l.membershipRoot);
      }
      if (e.kind === "launcher" && e.name === "Launched") await this.coinMeta(String(e.args.coin), e.log.transactionHash.toLowerCase());
    }
  }
  private async coinMeta(coin: string, tx?: string): Promise<void> {
    const dep = this.d.deployment!;
    const launchTx = tx ?? (this.d.db.prepare(`SELECT tx FROM coins WHERE coin = ?`).get(coin) as { tx: string } | undefined)?.tx;
    const [name, symbol, rec, logo, supply] = await Promise.all([tryRead<string>(this.d.rpc, coin, READS as Abi, "name"), tryRead<string>(this.d.rpc, coin, READS as Abi, "symbol"),
      dep.ponsFactory ? tryRead<readonly unknown[]>(this.d.rpc, dep.ponsFactory, READS as Abi, "getLaunchedToken", [coin]) : Promise.resolve(null),
      launchTx ? this.launchLogo(launchTx) : Promise.resolve(null),
      // docs/phase12.md §3: Pons mints the whole supply at launch, so this is one read per coin and not a series
      tryRead<bigint>(this.d.rpc, coin, READS as Abi, "totalSupply")]);
    const ok = rec !== null && rec[14] === true;
    this.d.db.prepare(`INSERT OR REPLACE INTO coin_meta (coin, name, symbol, tax_bps, pool_fee, tick_spacing, pair_token, logo, logo_at, supply, supply_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)`)
      .run(coin, name, symbol, ok ? Number(rec![8]) : null, ok ? Number(rec![6]) : null, ok ? Number(rec![7]) : null, ok ? String(rec![4]).toLowerCase() : null,
        logo, launchTx ? this.now() : null, supply === null ? null : String(supply), this.now());
  }

  /**
   * docs/phase10.md: the logo is in no event, so it is read from the transaction that emitted `Launched` and decoded from
   * `Launcher.launch`. A transaction that will not decode — a coin launched through something other than our Launcher, a node
   * that has forgotten it — is not an error: the coin simply keeps its monogram.
   */
  private async launchLogo(tx: string): Promise<string | null> {
    try {
      const t = await this.d.rpc<{ input: Hex } | null>("eth_getTransactionByHash", [tx]);
      if (!t?.input) return null;
      const { functionName, args } = decodeFunctionData({ abi: LAUNCHER_CALLS as Abi, data: t.input });
      if (functionName !== "launch") return null;
      return cleanLogo((args?.[0] as { logo?: unknown } | undefined)?.logo);
    } catch { return null; }
  }

  /**
   * docs/phase13.md §4. The transfers from before `Transfer` indexing began, a bounded piece at a time.
   *
   * It walks forward from `transfers_backfilled_through` towards `transfers_live_from`, at most `BACKFILL_RANGES` ranges a
   * pass, and stops for ever once they meet — which is the whole test of whether a balance is complete. The logs are asked
   * for BY TOPIC over the coins we know, so no other traffic on those addresses is fetched, and they go into `events` like
   * every other log, because the fold is the only thing allowed to know what a balance is.
   *
   * A backfilled log keeps its block's time only if the `blocks` table already holds it: a balance needs no clock, and one
   * `eth_getBlockByNumber` per historical transfer would cost more than the timestamps are worth.
   */
  private async backfillTransfers(): Promise<number> {
    const db = this.d.db, dep = this.d.deployment!;
    const liveFrom = Number(getMeta(db, "transfers_live_from") ?? dep.fromBlock - 1);
    let through = Number(getMeta(db, "transfers_backfilled_through") ?? dep.fromBlock - 1);
    if (through >= liveFrom) return 0;
    const coins = this.coinList();
    let added = 0;
    for (let i = 0; i < BACKFILL_RANGES && through < liveFrom; i++) {
      const from = through + 1, to = Math.min(through + this.maxRange, liveFrom);
      const logs: RawLog[] = [];
      for (let j = 0; j < coins.length; j += 200) logs.push(...await this.byTopic(coins.slice(j, j + 200), [TRANSFER], from, to));
      logs.sort((x, y) => Number(BigInt(x.blockNumber) - BigInt(y.blockNumber)) || Number(BigInt(x.logIndex) - BigInt(y.logIndex)));
      const ts = new Map((db.prepare(`SELECT number, ts FROM blocks WHERE number BETWEEN ? AND ?`).all(from, to) as { number: number; ts: number }[]).map((b) => [b.number, b.ts]));
      db.exec("BEGIN");
      try {
        const ins = db.prepare(`INSERT OR IGNORE INTO events (block_number, log_index, tx, address, contract, name, args, ts, listing_id, acquisition_id) VALUES (?, ?, ?, ?, ?, ?, ?, ?, NULL, NULL)`);
        for (const l of logs) {
          const d = decode("coin", l);
          if (!d) continue;
          const block = Number(BigInt(l.blockNumber));
          const e: Ev = { block, logIndex: Number(BigInt(l.logIndex)), tx: l.transactionHash.toLowerCase(), address: l.address.toLowerCase(), contract: "coin",
            name: d.name, args: d.args, ts: ts.get(block) ?? null };
          if (Number(ins.run(e.block, e.logIndex, e.tx, e.address, e.contract, e.name, JSON.stringify(e.args), e.ts).changes) !== 1) continue;
          apply(db, e);
          added++;
        }
        setMeta(db, "transfers_backfilled_through", String(to));
        db.exec("COMMIT");
      } catch (err) { db.exec("ROLLBACK"); throw err; }
      through = to;
    }
    this.log({ level: "info", event: "transfers-backfilled", through, liveFrom, added, done: through >= liveFrom });
    return added;
  }

  /** One pass: reorg check, then every full range up to head − confirmations. Returns the number of new events. */
  async pass(): Promise<number> {
    const dep = this.d.deployment, db = this.d.db;
    if (!dep) { this.state.lastPassAt = this.now(); return 0; }
    // docs/phase12.md §3: dollars per ETH, from one pool, at most once every few minutes. It is deliberately here and not in
    // a view: a rate read per request would be one eth_call per chart, and this is a number that changes by the block anyway.
    await refreshRate(this.d.rpc, db, this.d.usdPool === undefined ? USD_POOL : this.d.usdPool, this.now());
    try {
      const head = Number(BigInt(await this.d.rpc<Hex>("eth_blockNumber", [])));
      this.state.head = head;
      await this.checkReorg();
      const target = head - this.d.confirmations;
      let cursor = this.cursor() ?? dep.fromBlock - 1, added = 0;
      while (cursor < target) {
        const from = cursor + 1, to = Math.min(cursor + this.maxRange, target);
        const fixed = [dep.registry, dep.launcher, dep.distributor];
        const kindOf = new Map<string, ContractKind>([[dep.registry, "registry"], [dep.launcher, "launcher"], [dep.distributor, "distributor"]]);
        const first = await this.logs(fixed, from, to);
        const found: { kind: ContractKind; name: string; args: Ev["args"]; log: RawLog }[] = [];
        for (const l of first) { const k = kindOf.get(l.address.toLowerCase())!; const d = decode(k, l); if (d) found.push({ kind: k, ...d, log: l }); }
        const vaults = this.vaults(), routers = this.routers();
        for (const f of found) { if (f.name === "Listed") vaults.add(String(f.args.vault)); if (f.name === "Launched") routers.add(String(f.args.router)); }
        for (const v of vaults) kindOf.set(v, "vault");
        for (const r of routers) kindOf.set(r, "router");
        const curves = this.curves();
        for (const f of found) if (f.name === "Launched") curves.add(String(f.args.curve));
        for (const c of curves) kindOf.set(c, "curve");
        // docs/phase13.md §4: the coins themselves, for their `Transfer` logs. They join a list the follower already asks a
        // node about in batches of 200, so on staging this is 291 addresses where it was 214 — the same two calls.
        const known = new Set(this.coinList());
        for (const f of found) if (f.name === "Launched") known.add(String(f.args.coin));
        for (const c of known) kindOf.set(c, "coin");
        const dynamic = [...vaults, ...routers, ...curves, ...known];
        if (dynamic.length) for (const l of await this.logs(dynamic, from, to)) { const k = kindOf.get(l.address.toLowerCase())!; const d = decode(k, l); if (d) found.push({ kind: k, ...d, log: l }); }
        for (const l of await this.poolLogs(from, to, [...known])) {
          const d = decode("pool", l); if (d) found.push({ kind: "pool", ...d, log: l });
        }
        // docs/phase13.md §2: chain order, with a block's Listed and Launched folded before the logs that need them — a
        // coin's mint sits seven logs ahead of the event that says the coin is ours. `rebuild` sorts the same way.
        found.sort((x, y) => foldOrder(
          { block: Number(BigInt(x.log.blockNumber)), logIndex: Number(BigInt(x.log.logIndex)), name: x.name },
          { block: Number(BigInt(y.log.blockNumber)), logIndex: Number(BigInt(y.log.logIndex)), name: y.name }));

        const blocks = new Map<number, RawBlock>();
        for (const n of new Set(found.map((f) => Number(BigInt(f.log.blockNumber))))) blocks.set(n, await this.block(n));
        // a log whose block is no longer the node's block at that height: the chain moved under this read. Try again next pass.
        if (found.some((f) => blocks.get(Number(BigInt(f.log.blockNumber)))!.hash.toLowerCase() !== f.log.blockHash.toLowerCase())) throw new RpcError("rpc", "the chain moved during the read");
        const tip = blocks.get(to) ?? await this.block(to);
        await this.fetchMeta(found);

        db.exec("BEGIN");
        try {
          const insBlock = db.prepare(`INSERT OR REPLACE INTO blocks (number, hash, ts) VALUES (?, ?, ?)`);
          for (const [n, b] of blocks) insBlock.run(n, b.hash.toLowerCase(), Number(BigInt(b.timestamp)));
          const ins = db.prepare(`INSERT OR IGNORE INTO events (block_number, log_index, tx, address, contract, name, args, ts, listing_id, acquisition_id) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)`);
          for (const f of found) {
            const block = Number(BigInt(f.log.blockNumber));
            const e: Ev = { block, logIndex: Number(BigInt(f.log.logIndex)), tx: f.log.transactionHash.toLowerCase(), address: f.log.address.toLowerCase(), contract: f.kind,
              name: f.name, args: f.args, ts: Number(BigInt(blocks.get(block)!.timestamp)) };
            // Listed and Launched must be applied before lotOf can resolve a vault or router named in the same range
            const fresh = Number(ins.run(e.block, e.logIndex, e.tx, e.address, e.contract, e.name, JSON.stringify(e.args), e.ts, null, null).changes) === 1;
            if (!fresh) continue;                       // seen before (a restart mid-range): applying it twice is what the key prevents
            apply(db, e);
            const lot = lotOf(db, e);
            db.prepare(`UPDATE events SET listing_id = ?, acquisition_id = ? WHERE block_number = ? AND log_index = ?`).run(lot.listingId, lot.acquisitionId, e.block, e.logIndex);
            added++;
          }
          insBlock.run(to, tip.hash.toLowerCase(), Number(BigInt(tip.timestamp)));
          setMeta(db, "cursor", String(to)); setMeta(db, "cursor_hash", tip.hash.toLowerCase());
          db.exec("COMMIT");
        } catch (err) { db.exec("ROLLBACK"); throw err; }
        cursor = to;
      }
      // docs/phase13.md §4: the history `Transfer` indexing started too late for, two ranges a pass, once and never again.
      // It runs after the forward walk so that live data is never behind a backfill, and after the coins it needs exist.
      await this.backfillTransfers();
      // a coin whose Pons record, logo or supply could not be read at launch is asked again, a few at a time. A missing supply
      // is asked for again at most once an hour (docs/phase12.md §1): a node that blinked must not cost the coin its market
      // cap forever, and a coin that will never answer must not cost a read every pass.
      for (const c of db.prepare(`SELECT c.coin, c.tx FROM coins c LEFT JOIN coin_meta m ON m.coin = c.coin
        WHERE m.coin IS NULL OR m.tax_bps IS NULL OR m.logo_at IS NULL OR (m.supply IS NULL AND (m.supply_at IS NULL OR m.supply_at < ?)) LIMIT 5`)
        .all(this.now() - SUPPLY_RETRY_S) as { coin: string; tx: string }[]) await this.coinMeta(c.coin, c.tx);
      // old block rows are only needed for the reorg walk: keep the newest few thousand
      db.prepare(`DELETE FROM blocks WHERE number < ? AND number NOT IN (SELECT DISTINCT block_number FROM events)`).run(cursor - 5_000);
      this.state.lastPassAt = this.now(); this.state.lastError = null;
      return added;
    } catch (err) {
      this.state.lastError = { at: this.now(), kind: err instanceof RpcError ? err.kind : "internal" };
      this.log({ level: "error", event: "pass-failed", kind: this.state.lastError.kind, message: String((err as Error).message).slice(0, 200) });
      throw err;
    }
  }
}
