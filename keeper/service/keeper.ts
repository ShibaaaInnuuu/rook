import { decodeFunctionResult, encodeFunctionData, type Abi, type Hex } from "viem";
import { planPost, type PostAction, type Posted } from "../src/floorpost.js";
import { balancesOf, type Entry } from "../../tools/snapshot/src/core.js";
import { DRAW_RULES, FACTORY, MODULE, POLICY, REGISTRY, STAGE, VAULT, type DrawView } from "./abis.js";
import { mapLimit, type CoinIndex } from "./coins.js";
import { hasMirror, type ServiceConfig } from "./config.js";
import { blockAt, buildSnapshot, evmNumber, exclusionsFor, firstEmptyBlock, lastBlockAtOrBefore, publish, type Block, type DrawBook } from "./draws.js";
import { provenRound, type DrandRelays } from "./drand.js";
import type { FloorFeed } from "./feed.js";
import type { LogCache } from "./logs.js";
import { announceOpen, openStillAllowed, PROD_POST_POLICY, planCap, slotAt, type CapPlan } from "./plan.js";
import { resolverPass, type ResolverReport } from "./resolver.js";
import { Refusal, type ChainRpc, type Fetch, type Log } from "./rpc.js";
import type { Result, Sender } from "./sender.js";
import type { Stack, StackCollection } from "./stack.js";
import type { Resolver } from "./outbound.js";

/**
 * docs/v6/p1-services.md §1.2–§1.4; docs/audit/2026-09-26/fixes-W2-services.md. One pass of the production keeper: check who it
 * is (and that the registry has not paused it), resolve what it has in flight (re-pricing it when stuck), then EVERY collection's
 * floor and caps first, then the draws (open what was announced, announce at the slot), then the keyless resolver — the last two
 * within the pass's time budget, so the floor is never behind a draw. Every decision the contracts judge is taken on the chain's
 * clock (the latest block), so a restart re-derives the same plan from the same state.
 */
export type DrawEntry = { coin: Hex; drawId?: string; slot?: number; action: "announce" | "open"; plan: string; result: Result | null; uri?: string; root?: string;
  snapshotBlock?: number; snapshotAt?: number; tickets?: string };
export type CollectionReport = {
  slug: string;
  floor: { module: Hex; feed: string; feedWhy: string; plan: PostAction | null; result: Result | null; postedWei: string; fresh: boolean };
  caps: { vault: Hex; policy: "raffle" | "burn"; plan: CapPlan; result: Result | null; capWei: string; fresh: boolean }[];
  draws: DrawEntry[];
};
export type PassReport = { at: string; chainTime: number; block: number; refusals: string[]; alerts: string[]; pending: Result | null; collections: CollectionReport[];
  balanceWei: string | null; resolver: ResolverReport | null; coins: { indexed: number; behind: number } | null };

/** What the keeper remembers between passes in memory only (a restart re-derives it): each slot's due coins, once scanned. */
/** `rebuilt`: a Resolved draw's winning entry rebuilt from the chain, kept for good once it proves (R2-S1); a rebuild that does not
 *  prove, or that fails, is kept as `entry: null` until `retryAt` only (R3-check L-D: the resolver's log provider may have been short
 *  that once), 10 minutes after the first, doubling to 6 hours. */
export type Rebuilt = { entry: Entry | null; retryAt: number; tries: number };
export type KeeperMemory = { slots: Map<string, { due: Hex[]; tickets: Map<string, bigint>; reported: boolean }>; beacons: Map<string, Hex>;
  rebuilt: Map<string, Rebuilt> };
export const newMemory = (): KeeperMemory => ({ slots: new Map(), beacons: new Map(), rebuilt: new Map() });

export type KeeperDeps = { cfg: ServiceConfig; stack: Stack; rpc: ChainRpc; sender: Sender; feed: FloorFeed; from: Hex; log: Log; snapshotDir: string; exclusionsFile: string;
  relays: DrandRelays; logs: LogCache; coinIndex: (c: StackCollection) => CoinIndex; book: (vault: Hex) => DrawBook; fetchImpl: Fetch; nowMs: () => number; mem: KeeperMemory;
  /** how the host of an untrusted URL (a draw's `uri`) is resolved before it is fetched; default: the system's DNS (R2-S6) */
  resolveHost?: Resolver };

export async function read<T>(rpc: ChainRpc, to: Hex, abi: Abi | readonly unknown[], functionName: string, args: unknown[] = [], block = "latest"): Promise<T> {
  const data = encodeFunctionData({ abi: abi as Abi, functionName, args });
  const out = await rpc.call<Hex>("eth_call", [{ to, data }, block]);
  return decodeFunctionResult({ abi: abi as Abi, functionName, data: out }) as T;
}

export const PAUSED = "the registry has paused the keeper (keeperPaused() = true): nothing is signed or sent until the owner unpauses it";

/** §1.2: the identity checks, every pass. Each problem is a refusal: nothing is sent while any stands. */
export async function identityProblems(d: Pick<KeeperDeps, "rpc" | "stack" | "from" | "cfg">): Promise<string[]> {
  const out: string[] = [];
  // fixes-B-C-keeper-draws.md §5 item 5: a paused keeper sends nothing (the registry's keeper() is zero meanwhile anyway)
  if (await read<boolean>(d.rpc, d.stack.registry, REGISTRY, "keeperPaused")) return [PAUSED];
  const me = d.from.toLowerCase();
  const regKeeper = (await read<string>(d.rpc, d.stack.registry, REGISTRY, "keeper")).toLowerCase();
  if (regKeeper !== me) out.push(`the key's address ${d.from} is not registry.keeper() (${regKeeper})`);
  for (const c of d.stack.collections) {
    const mk = (await read<string>(d.rpc, c.floorModule, MODULE, "keeper")).toLowerCase();
    if (c.kind === "nft") {
      // docs/v7/phase1-nft.md §3.2: the canister posts this module's floor (its PosterSource); the keeper only reads it for the caps
      if (mk !== String(c.floorPoster).toLowerCase()) out.push(`${c.slug}: the module's keeper is ${mk}, not the stack's floorPoster (the canister) ${c.floorPoster}`);
      if (mk === me) out.push(`${c.slug}: the keeper's key is this nft module's poster: the canister must post it, never the keeper`);
    } else if (mk !== me) out.push(`${c.slug}: the key's address is not the module's keeper (${mk}; module.keeper() reads the registry's)`);
    for (const [vault, want] of [[c.raffleVault, POLICY.RAFFLE], [c.burnVault, POLICY.BURN]] as const) {
      const [reg, pol] = await Promise.all([read<string>(d.rpc, vault, VAULT, "registry"), read<number>(d.rpc, vault, VAULT, "policy")]);
      if (reg.toLowerCase() !== d.stack.registry.toLowerCase()) out.push(`${c.slug}: vault ${vault} names registry ${reg}, not the stack's`);
      if (Number(pol) !== want) out.push(`${c.slug}: vault ${vault} has policy ${Number(pol)}, not ${want === POLICY.RAFFLE ? "RAFFLE" : "BURN"} (the keeper never posts to a YOYO vault)`);
    }
  }
  if (d.cfg.mode === "send") {
    const code = await d.rpc.call<string>("eth_getCode", [d.from, "latest"]);
    if (code && code !== "0x") out.push(`the keeper's address has code (${code.slice(0, 10)}…): an EIP-7702 delegation would take its gas money (docs/trust-model.md)`);
  }
  return out;
}

type ModuleState = Posted & { fresh: boolean; accepted: bigint; minConfidenceBps: number };
async function posted(rpc: ChainRpc, module: Hex): Promise<ModuleState> {
  const [floor, maxMove, fresh, accepted, minConf, conf] = await Promise.all([read<readonly [bigint, bigint, bigint]>(rpc, module, MODULE, "floor"), read<number>(rpc, module, MODULE, "maxMoveBps"),
    read<boolean>(rpc, module, MODULE, "fresh"), read<bigint>(rpc, module, MODULE, "accepted"), read<bigint>(rpc, module, MODULE, "minConfidenceBps"),
    read<number>(rpc, module, MODULE, "confidenceBps")]);
  let bounds: Posted["bounds"] = null;
  try { const [lo, hi] = await read<readonly [bigint, bigint]>(rpc, module, MODULE, "floorBounds"); bounds = { lo, hi }; } catch { bounds = null; }
  return { priceWei: floor[0], postedAt: Number(floor[1]), ttl: Number(floor[2]), maxMoveBps: Number(maxMove), bounds, fresh, accepted,
    minConfidenceBps: Number(minConf), confidenceBps: Number(conf) };
}

async function capState(rpc: ChainRpc, vault: Hex) {
  const [cap, capAt, capAtBlock, fresh, ceiling, pbs] = await Promise.all([read<bigint>(rpc, vault, VAULT, "cap"), read<bigint>(rpc, vault, VAULT, "capAt"),
    read<bigint>(rpc, vault, VAULT, "capAtBlock"), read<boolean>(rpc, vault, VAULT, "capFresh"), read<bigint>(rpc, vault, VAULT, "capCeiling"), read<bigint>(rpc, vault, VAULT, "parentBlockSeconds")]);
  return { cap, capAt: Number(capAt), capAtBlock: Number(capAtBlock), fresh, ceiling, parentBlockSeconds: Number(pbs) };
}

/** A submit that is still waiting ends the pass's sends: one transaction in flight at a time. */
export const stops = (r: Result | null): boolean => r !== null && (r.outcome === "pending" || (r.outcome === "refused" && /in flight/.test(r.detail)));

/** Per pass: the time budget, and a confirmed block (ten minutes behind the head) computed once, when a draw needs it. */
export type PassCtx = { head: Block; nowS: number; nowEvm: number; deadlineMs: number; alerts: string[]; halted: boolean; confirmed: () => Promise<number> };
export const CONFIRM_S = 600;

export function passCtx(d: Pick<KeeperDeps, "rpc" | "stack" | "cfg" | "nowMs">, head: Block, alerts: string[], t0 = d.nowMs()): PassCtx {
  const nowS = Number(BigInt(head.timestamp));
  let confirmedMemo: number | null = null;
  return { head, nowS, nowEvm: evmNumber(head), deadlineMs: t0 + d.cfg.passBudgetS * 1000, alerts, halted: false,
    confirmed: async () => (confirmedMemo ??= await lastBlockAtOrBefore(d.rpc, nowS - CONFIRM_S, d.stack.fromBlock, head).catch(() => -1)) };
}

export async function runPass(d: KeeperDeps): Promise<PassReport> {
  const t0 = d.nowMs();
  await d.rpc.init();                 // every endpoint asked its chain id again; none left that says ours: this pass is refused
  const head: Block = await blockAt(d.rpc, "latest");
  const nowS = Number(BigInt(head.timestamp)), nowEvm = evmNumber(head);
  const report: PassReport = { at: new Date(t0).toISOString(), chainTime: nowS, block: Number(BigInt(head.number)), refusals: [], alerts: [], pending: null, collections: [],
    balanceWei: null, resolver: null, coins: null };
  try { report.balanceWei = String(BigInt(await d.rpc.call<string>("eth_getBalance", [d.from, "latest"]))); } catch { report.balanceWei = null; }
  report.refusals = await identityProblems(d);
  if (report.refusals.length) {
    for (const r of report.refusals) d.log("error", "refused", { reason: r });
    return report;
  }
  const ctx = passCtx(d, head, report.alerts, t0);
  report.pending = await d.sender.resolvePending();
  if (report.pending?.alert) report.alerts.push(report.pending.alert);
  ctx.halted = stops(report.pending);

  // 1. every collection's floor and caps, before anything else
  for (const c of d.stack.collections) {
    const cr: CollectionReport = { slug: c.slug, floor: { module: c.floorModule, feed: d.feed.name, feedWhy: "", plan: null, result: null, postedWei: "0", fresh: false }, caps: [], draws: [] };
    report.collections.push(cr);
    const p = await posted(d.rpc, c.floorModule);
    const nftPosted = c.kind === "nft";
    const feed = nftPosted ? { floor: null, why: "the canister posts this module's floor (docs/v7/phase1-nft.md §3.2)", ref: null } as unknown as Awaited<ReturnType<typeof d.feed.floor>>
      : await d.feed.floor(c.slug);
    cr.floor.feedWhy = feed.why;
    const policy = { ...PROD_POST_POLICY, minConfidenceBps: Math.max(PROD_POST_POLICY.minConfidenceBps, p.minConfidenceBps) };
    // the module's ±5 %/h and ±20 %/day band (floorBounds) is required: a module that does not answer it is not this version
    const plan: PostAction = nftPosted ? { type: "skip", why: "the canister posts this nft module's floor; the keeper never does" }
      : p.bounds === null ? { type: "skip", why: "the module does not answer floorBounds(): nothing is planned against an unknown band" }
      : planPost(feed.floor, p, nowS, policy);
    cr.floor.plan = plan;
    d.log("info", "floor-plan", { slug: c.slug, module: c.floorModule, action: plan.type, why: plan.why, feed: feed.why, ref: feed.ref });
    if (plan.type === "post" && !ctx.halted) {
      cr.floor.result = await d.sender.submit({ key: `floor:${c.floorModule.toLowerCase()}`, what: `postFloor(${plan.priceWei}, ${plan.confidenceBps}, ${plan.depthItems}) on ${c.slug}`, to: c.floorModule, data: plan.calldata });
      ctx.halted = stops(cr.floor.result);
      if (cr.floor.result.alert) report.alerts.push(cr.floor.result.alert);
    }
    const after = await posted(d.rpc, c.floorModule);
    cr.floor.postedWei = String(after.accepted);
    cr.floor.fresh = after.fresh;

    // the caps, RAFFLE and BURN — never YOYO — from the module's accepted floor while it is fresh
    // a dry run posts nothing, so the caps it simulates rest on the floor it would have posted (clamped by the chain's band)
    const wouldPost = d.cfg.mode === "dry-run" && plan.type === "post" && cr.floor.result?.outcome === "simulated" ? BigInt(plan.priceWei) : null;
    const capFloor = after.fresh && after.accepted > 0n ? after.accepted : wouldPost;
    for (const [vault, pol] of [[c.raffleVault, "raffle"], [c.burnVault, "burn"]] as const) {
      const s = await capState(d.rpc, vault);
      const cp = planCap(capFloor, s, nowS, nowEvm, d.cfg.capMarginBps);
      const entry = { vault, policy: pol, plan: cp, result: null as Result | null, capWei: String(s.cap), fresh: s.fresh };
      cr.caps.push(entry);
      d.log("info", "cap-plan", { slug: c.slug, vault, policy: pol, action: cp.type, why: cp.why });
      if (s.ceiling === 0n) report.alerts.push(`no-ceiling: ${c.slug}'s ${pol} vault has no capCeiling(); it takes no cap until the owner sets one`);
      if (cp.type === "post" && !ctx.halted) {
        entry.result = await d.sender.submit({ key: `cap:${vault.toLowerCase()}`, what: `postCap(${cp.capWei}) on ${c.slug}'s ${pol} vault`, to: vault,
          data: encodeFunctionData({ abi: VAULT, functionName: "postCap", args: [BigInt(cp.capWei)] }) });
        ctx.halted = stops(entry.result);
      }
    }
  }

  // 2. draws on each RAFFLE vault: open what was announced, then announce at the slot (bounded work; E-04)
  for (const [i, c] of d.stack.collections.entries()) {
    const cr = report.collections[i];
    const idx = d.coinIndex(c);
    try { const r = await idx.refresh(); report.coins = { indexed: idx.coins.length, behind: r.behind }; }
    catch (err) { report.alerts.push(`coin-index: ${c.slug}: ${String((err as Error).message).slice(0, 160)}`); }
    try { await draws(d, c, cr, ctx); }
    catch (err) {                      // never silent (E-11): the pass says so, and the next pass tries again
      const why = String((err as Error).message).slice(0, 200);
      report.alerts.push(`draws: ${c.slug}: ${why}`);
      d.log("error", "draws-failed", { slug: c.slug, error: why });
    }
  }

  // 3. the keyless resolver, with the keeper's sender (a standalone resolver with its own key runs the same code)
  if (d.cfg.resolver.enabled) {
    if (d.nowMs() >= ctx.deadlineMs) report.alerts.push("pass-budget: the resolver did not run this pass (the time budget was spent)");
    else {
      try { report.resolver = await resolverPass(d, ctx); }
      catch (err) { report.alerts.push(`resolver: ${String((err as Error).message).slice(0, 200)}`); d.log("error", "resolver-failed", { error: String((err as Error).message).slice(0, 200) }); }
    }
  }
  for (const a of report.alerts) d.log("error", "alert", { alert: a });
  return report;
}

const E18 = 10n ** 18n;
export const mirrorRequired = (cfg: Pick<ServiceConfig, "mode" | "chainId" | "snapshots">) => cfg.mode === "send" && cfg.chainId === 4663 && !hasMirror(cfg.snapshots);

/** A coin's curve (its router at the factory): from the coin index, or read. */
export async function curveOf(d: Pick<KeeperDeps, "rpc" | "coinIndex">, c: StackCollection, coin: Hex): Promise<Hex> {
  const known = d.coinIndex(c).coins.find((x) => x.coin.toLowerCase() === coin.toLowerCase());
  return known ? known.curve : read<Hex>(d.rpc, c.factory, FACTORY, "routerOf", [coin]);
}

export async function beaconOf(d: Pick<KeeperDeps, "rpc" | "mem">, holder: Hex, abi: Abi | readonly unknown[] = VAULT): Promise<Hex> {
  const k = holder.toLowerCase();
  if (!d.mem.beacons.has(k)) d.mem.beacons.set(k, await read<Hex>(d.rpc, holder, abi, "beacon"));
  return d.mem.beacons.get(k)!;
}

async function draws(d: KeeperDeps, c: StackCollection, cr: CollectionReport, ctx: PassCtx): Promise<void> {
  const vault = c.raffleVault, { nowS, nowEvm, head } = ctx;
  const book = d.book(vault);
  const inFlight = await book.open();
  const pbs = Number(await read<bigint>(d.rpc, vault, VAULT, "parentBlockSeconds"));
  const out = (e: DrawEntry) => { cr.draws.push(e); d.log("info", "draw-plan", { slug: c.slug, coin: e.coin, drawId: e.drawId, slot: e.slot, action: e.action, plan: e.plan, result: e.result?.outcome }); };

  // 1. open every draw this vault has announced, as soon as its seed round exists
  for (const { id, d: dv } of inFlight) {
    if (Number(dv.stage) !== STAGE.Announced) continue;
    const e: DrawEntry = { coin: dv.coin, drawId: String(id), action: "open", plan: "", result: null };
    if (!openStillAllowed(Number(dv.announcedAt), Number(dv.announcedAtBlock), nowS, nowEvm, pbs, DRAW_RULES.openTtlS)) {
      e.plan = `announced at ${dv.announcedAt}: too late to open (the vault refuses at 6 h); anyone may void it then, and the resolver does`;
      ctx.alerts.push(`draw-not-opened: draw ${id} of ${c.slug} (${dv.coin}) was not opened in time`);
      out(e); continue;
    }
    if (d.nowMs() >= ctx.deadlineMs) { e.plan = "the pass's time budget is spent: opened next pass"; out(e); continue; }
    const beacon = await beaconOf(d, vault);
    let proven;
    try { proven = await provenRound(d.relays, d.rpc, beacon, dv.seedRound); } catch (err) {
      e.plan = `seed round ${dv.seedRound}: ${String((err as Error).message).slice(0, 200)}`;
      ctx.alerts.push(`drand: draw ${id} of ${c.slug}: ${e.plan}`);
      out(e); continue;
    }
    if (!proven) {
      e.plan = `waiting for drand round ${dv.seedRound} (two relays must serve it)`;
      if (nowS > Number(dv.announcedAt) + 900) ctx.alerts.push(`drand-late: draw ${id} of ${c.slug}: seed round ${dv.seedRound} is not served by two relays ${nowS - Number(dv.announcedAt)} s after the announcement`);
      out(e); continue;
    }
    if (proven.liars.length) ctx.alerts.push(`drand-liar: seed round ${dv.seedRound} of draw ${id} of ${c.slug}: ${proven.liars.join(", ")} served bytes the beacon refuses; ${proven.relays.join(" + ")} served the beacon's signature`);
    // the snapshot time is the vault's word, never computed here: a draw that carries the seed of one voided or cancelled before
    // its pin (review2 N-2, SeedCarried) gets its origin's time, counted back from the origin's announcement, not this one's
    const snapshotAt = Number(await read<bigint>(d.rpc, vault, VAULT, "snapshotTimeOf", [id, proven.randomness]));
    e.snapshotAt = snapshotAt;
    const origin = await read<bigint>(d.rpc, vault, VAULT, "seedOrigin", [id]);
    const carried = origin !== id ? `; carries draw ${origin}'s seed round and snapshot time` : "";
    let built;
    try {
      const block = await lastBlockAtOrBefore(d.rpc, snapshotAt, d.stack.fromBlock, head);
      e.snapshotBlock = block;
      const confirmed = await ctx.confirmed();
      built = await buildSnapshot(d.logs, { stack: d.stack, c, coin: dv.coin, curve: await curveOf(d, c, dv.coin), block, snapshotAt, confirmed: Math.min(block, confirmed),
        listedFile: d.exclusionsFile });
    } catch (err) {
      e.plan = `no snapshot yet: ${String((err as Error).message).slice(0, 200)}`;
      ctx.alerts.push(`snapshot: draw ${id} of ${c.slug}: ${e.plan}`);
      out(e); continue;
    }
    e.root = built.snap.root; e.tickets = built.snap.totalTickets;
    // fixes-R3-carry: nobody eligible held a whole coin at the snapshot time. The draw is opened over the sentinel alone (the tree
    // tools/snapshot builds for zero holders: root = sentinel leaf(0, 0, T), no proof); after the challenge window requestDraw
    // closes it with no winner (Empty), its unit back to the coin and a carried seed consumed. The monitor recomputes it like any list.
    const empty = BigInt(built.snap.totalTickets ?? "0") === 0n;
    let pub;
    try { pub = await publish({ dir: d.snapshotDir, ...d.cfg.snapshots }, built.name, built.text, d.fetchImpl); } catch (err) {
      e.plan = `not published: ${String((err as Error).message).slice(0, 200)}`;
      ctx.alerts.push(`publish: draw ${id} of ${c.slug}: ${e.plan}`);
      out(e); continue;
    }
    e.uri = pub.uri;
    e.plan = empty
      ? `open draw ${id} EMPTY: no eligible holder had a whole coin at block ${e.snapshotBlock} (snapshot time ${snapshotAt}${carried}, seed round ${dv.seedRound} from ${proven.relays.join(" + ")}; sha256 ${pub.sha256.slice(0, 16)}…${pub.mirrored ? ", mirrored" : ""}): the sentinel alone; after the challenge window it closes with no winner and the unit stays with the coin`
      : `open draw ${id} over ${built.snap.totalTickets} tickets at block ${e.snapshotBlock} (snapshot time ${snapshotAt}${carried}, seed round ${dv.seedRound} from ${proven.relays.join(" + ")}; sha256 ${pub.sha256.slice(0, 16)}…${pub.mirrored ? ", mirrored" : ""})`;
    if (ctx.halted) { e.plan += "; waiting: a transaction is in flight"; out(e); continue; }
    // openDraw proves the seed round on chain (a BN254 pairing, ~180k gas): it takes the resolver's gas cap, not the floor's
    e.result = await d.sender.submit({ key: `open:${vault.toLowerCase()}:${id}`, what: `openDraw(${id}) for ${dv.coin} on ${c.slug}`, to: vault, maxGas: d.cfg.resolver.maxGasPerTx,
      data: encodeFunctionData({ abi: VAULT, functionName: "openDraw", args: [id, built.snap.root, BigInt(built.snap.totalTickets!), pub.uri, built.snap.sentinel!.proof, proven.signature] }) });
    ctx.halted = stops(e.result);
    if (e.result.alert) ctx.alerts.push(e.result.alert);
    out(e);
  }

  // 2. announce at the slot: each RAFFLE coin with a whole unit, one draw per coin per slot (the vault: one per coin before its pin)
  const slot = slotAt(nowS, d.cfg.draw);
  const win = announceOpen(slot, nowS, d.cfg.draw);
  const key = `${vault.toLowerCase()}:${slot}`;
  let sc = d.mem.slots.get(key);
  if (!win.open) {
    // the window is over: a due coin that was not announced is an alert, once, never a quiet skip (E-11)
    if (sc && !sc.reported) {
      for (const coin of sc.due) if (book.lastAnnouncedAt(coin) < slot) ctx.alerts.push(`draw-slot-missed: ${c.slug} ${coin} had a whole unit at slot ${slot} and no draw was announced in the window`);
      sc.reported = true;
    }
    return;
  }
  if (d.nowMs() >= ctx.deadlineMs) { ctx.alerts.push(`pass-budget: ${c.slug}'s draw announcements wait for the next pass`); return; }
  if (!sc) {
    const raffle = d.coinIndex(c).coins.filter((x) => x.policy === POLICY.RAFFLE);
    const units = await mapLimit(raffle, 8, (x) => read<bigint>(d.rpc, vault, VAULT, "units", [x.coin]));
    sc = { due: raffle.filter((_, i) => units[i] >= E18).map((x) => x.coin), tickets: new Map(), reported: false };
    d.mem.slots.set(key, sc);
  }
  for (const coin of sc.due) {
    const e: DrawEntry = { coin, slot, action: "announce", plan: "", result: null };
    if (book.lastAnnouncedAt(coin) >= slot) { e.plan = `slot ${slot}: already announced`; out(e); continue; }
    const inflight = await read<bigint>(d.rpc, vault, VAULT, "drawInFlight", [coin]);
    if (inflight !== 0n) { e.plan = `slot ${slot}: draw ${inflight - 1n} of this coin is still before its pin (one per coin)`; out(e); continue; }
    if ((await read<bigint>(d.rpc, vault, VAULT, "units", [coin])) < E18) { e.plan = `slot ${slot}: no whole unit any more`; out(e); continue; }
    if (!d.cfg.snapshots.baseUrl) {
      e.plan = `slot ${slot}: no public URL (KEEPER_PUBLIC_URL or KEEPER_SNAPSHOT_BASE_URL): a snapshot could not be published, so no draw is announced`;
      ctx.alerts.push(`draw-not-announced: ${c.slug} ${coin}: ${e.plan}`); out(e); continue;
    }
    if (mirrorRequired(d.cfg)) {
      e.plan = `slot ${slot}: no snapshot mirror (KEEPER_SNAPSHOT_S3_ENDPOINT or KEEPER_SNAPSHOT_UPLOAD_URL): on 4663 a draw's snapshot must live somewhere besides the keeper's volume`;
      ctx.alerts.push(`draw-not-announced: ${c.slug} ${coin}: ${e.plan}`); out(e); continue;
    }
    // a coin nobody holds a whole unit of now would give an empty draw: not announced (it would only be voided)
    if (!sc.tickets.has(coin)) {
      const headNo = Number(BigInt(head.number));
      const logs = await d.logs.upTo(coin, d.stack.fromBlock, headNo, await ctx.confirmed());
      // counted, not built: a coin whose holders all have less than one whole coin has no tree at all (snapshot() throws on it)
      sc.tickets.set(coin, ticketsOf(balancesOf(logs), exclusionsFor(d.stack, c, coin, await curveOf(d, c, coin), d.exclusionsFile)));
    }
    if (sc.tickets.get(coin) === 0n) { e.plan = `slot ${slot}: no holder has a whole coin now: not announced`; out(e); continue; }
    // review2 N-2 (integration R3): a void carries the seed and its snapshot time until a pin, so an empty snapshot would be for good
    const { why, note } = await announceable(d, c, coin, vault, head, nowS, ctx);
    if (why) { e.plan = `slot ${slot}: ${why}`; out(e); continue; }
    if (ctx.halted) { e.plan = `slot ${slot}: waiting: a transaction is in flight`; out(e); continue; }
    e.plan = note ? `slot ${slot}: announce (${win.why}); ${note}` : `slot ${slot}: announce (${win.why}); the seed round it pins will pick the snapshot time`;
    e.result = await d.sender.submit({ key: `announce:${vault.toLowerCase()}:${coin.toLowerCase()}:${slot}`, what: `announceDraw(${coin}) on ${c.slug} at slot ${slot}`, to: vault,
      data: encodeFunctionData({ abi: VAULT, functionName: "announceDraw", args: [coin] }) });
    if (e.result.outcome === "mined") book.noteAnnounced(coin, nowS);
    ctx.halted = stops(e.result);
    if (e.result.alert) ctx.alerts.push(e.result.alert);
    out(e);
  }
}

/**
 * Why `coin` must not be announced now (null: it may). With a carried seed (a draw of this coin voided or cancelled before its pin),
 * the next draw's snapshot time is already fixed and nothing the keeper does changes it: it is announced (once the origin's seed
 * round is served, so the time can be read on chain); if nobody held a whole coin then, it is opened empty and closes with no winner,
 * which consumes the carry (fixes-R3-carry). With a new seed, every moment of the window its snapshot time may fall in must have
 * tickets (`firstEmptyBlock`): an empty draw is a round with no winner. Never computed from `announcedAt`.
 */
async function announceable(d: KeeperDeps, c: StackCollection, coin: Hex, vault: Hex, head: Block, nowS: number, ctx: PassCtx): Promise<{ why: string | null; note?: string }> {
  const curve = await curveOf(d, c, coin), exclusions = exclusionsFor(d.stack, c, coin, curve, d.exclusionsFile);
  const confirmed = await ctx.confirmed(), headNo = Number(BigInt(head.number));
  const carried = await read<bigint>(d.rpc, vault, VAULT, "carriedSeed", [coin]);
  if (carried !== 0n) {
    const origin = carried - 1n;
    const od = await read<DrawView>(d.rpc, vault, VAULT, "draw", [origin]);
    const proven = await provenRound(d.relays, d.rpc, await beaconOf(d, vault), od.seedRound).catch(() => null);
    if (!proven) return { why: `carries draw ${origin}'s seed round ${od.seedRound}, not served by two relays yet: announced when it is` };
    const at = Number(await read<bigint>(d.rpc, vault, VAULT, "snapshotTimeOf", [origin, proven.randomness]));
    const block = await lastBlockAtOrBefore(d.rpc, at, d.stack.fromBlock, head);
    // fixes-R3-carry: an empty carried time is announced all the same; the draw opens over the sentinel alone and closes with no
    // winner, which consumes the carry (before, it was refused and paged draw-carried-empty: every void would have carried it again)
    if (ticketsOf(balancesOf(await d.logs.upTo(coin, d.stack.fromBlock, block, Math.min(block, confirmed))), exclusions) === 0n) {
      return { why: null, note: `carries draw ${origin}'s snapshot time ${at}, at which nobody held a whole coin: it will open empty and close with no winner, which consumes the carry (fixes-R3-carry); the unit stays with the coin` };
    }
    return { why: null, note: `carries draw ${origin}'s seed round and snapshot time ${at} (review2 N-2)` };
  }
  // the stack's first day: the window reaches before the stack's first block, so no coin can have had a ticket through all of it
  const start = await lastBlockAtOrBefore(d.rpc, nowS - DRAW_RULES.snapshotWindowS, d.stack.fromBlock, head).catch(() => null);
  if (start === null) return { why: `the ${DRAW_RULES.snapshotWindowS / 3_600} h its snapshot time may fall in reaches before the stack's first block (${d.stack.fromBlock}): announced once the stack is a window old` };
  const empty = firstEmptyBlock(await d.logs.upTo(coin, d.stack.fromBlock, headNo, confirmed), start, exclusions);
  if (empty !== null) return { why: `nobody held a whole coin at block ${empty}, inside the ${DRAW_RULES.snapshotWindowS / 3_600} h its snapshot time may fall in; a snapshot there would be a round with no winner (fixes-R3-carry): announced once the coin has had a ticket for a whole window` };
  return { why: null };
}
/** The weighted tickets of `balances` (one per whole coin), exclusions left out: tools/snapshot's count, without a tree. */
export function ticketsOf(balances: ReadonlyMap<string, bigint>, exclusions: readonly string[]): bigint {
  const x = new Set(exclusions.map((a) => a.toLowerCase()));
  let n = 0n;
  for (const [a, b] of balances) if (!x.has(a.toLowerCase())) n += b / E18;
  return n;
}

export { Refusal };
