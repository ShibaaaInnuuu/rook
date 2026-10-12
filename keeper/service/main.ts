import { join } from "node:path";
import type { Hex } from "viem";
import { logger, envSecrets } from "../src/log.js";
import { ConfigError, hasMirror, parseServiceConfig, type ServiceConfig } from "./config.js";
import { CoinIndex } from "./coins.js";
import { DrawBook } from "./draws.js";
import { DrandRelays } from "./drand.js";
import { FixtureFeed, OrdinalsWalletFeed, productionRate, type FloorFeed } from "./feed.js";
import { newMemory, read, runPass, PAUSED, type KeeperDeps, type PassReport } from "./keeper.js";
import { LogCache } from "./logs.js";
import { REGISTRY } from "./abis.js";
import { ChainRpc, Refusal, type Log } from "./rpc.js";
import { Sender } from "./sender.js";
import { signerFromEnv, type Signer } from "./signer.js";
import { loadStack, type Stack } from "./stack.js";
import { StateStore, utcDay } from "./state.js";
import { AlertLatch, serveStatus, type Status } from "./status.js";

/**
 * docs/v6/p1-services.md §1. The production keeper: `npm run service -- [--dry-run] [--once]`, configured by its environment
 * (docs/deploy/services-v1.md). Dry run is the default; it sends only with KEEPER_MODE=send and no --dry-run flag.
 */
export type Built = KeeperDeps & { store: StateStore; signer: Signer | null };

/** The secrets a log line or a status body must never carry, besides the key (the signer scrubs that): read at logging time. */
export const serviceSecrets = (env: Readonly<Record<string, string | undefined>>) => (): string[] =>
  [...envSecrets(env), env.KEEPER_STATUS_TOKEN ?? "", env.KEEPER_SNAPSHOT_UPLOAD_AUTH ?? "", env.KEEPER_SNAPSHOT_S3_SECRET_ACCESS_KEY ?? ""];

type BuildOpts = { fetchImpl?: typeof fetch; write?: (line: string) => void; sleep?: (ms: number) => Promise<void>; nowMs?: () => number };

/** The parts both the keeper and the standalone resolver are made of. */
export async function buildCommon(cfg: ServiceConfig, env: Readonly<Record<string, string | undefined>>, o: BuildOpts) {
  // the key is read only in send mode: a dry run never holds it, so it cannot leak it
  const signer = cfg.mode === "send" ? signerFromEnv(env, cfg.role) : null;
  const write = o.write ?? ((l: string) => console.log(l));
  const nowMs = o.nowMs ?? Date.now;
  const log: Log = logger((line) => write(signer ? signer.scrub(line) : line), () => new Date(nowMs()), serviceSecrets(env));
  if (cfg.mode === "send" && !signer) throw new Refusal(`${cfg.role === "resolver" ? "RESOLVER_MODE" : "KEEPER_MODE"}=send, and ${cfg.role === "resolver" ? "RESOLVER_PRIVATE_KEY" : "KEEPER_PRIVATE_KEY"} is not set: nothing can be signed`);
  const stack = loadStack(cfg.stackPath, cfg.chainId);
  const fetchImpl = o.fetchImpl ?? fetch;
  const rpc = new ChainRpc(cfg.rpcUrls, cfg.chainId, fetchImpl, log, 20_000, { sleep: o.sleep });
  const live = await rpc.init();
  const store = new StateStore(cfg.stateDir);
  const relays = new DrandRelays(cfg.drandUrls, fetchImpl, log);
  const logs = new LogCache(join(cfg.stateDir, "logs"), rpc);
  const indexes = new Map<string, CoinIndex>(), books = new Map<string, DrawBook>();
  const coinIndex = (c: Stack["collections"][number]) => {
    const k = c.factory.toLowerCase();
    if (!indexes.has(k)) indexes.set(k, new CoinIndex(join(cfg.stateDir, `coins-${k}.json`), rpc, c.factory, cfg.coinsPerPass));
    return indexes.get(k)!;
  };
  const book = (vault: Hex) => {
    const k = vault.toLowerCase();
    if (!books.has(k)) books.set(k, new DrawBook(join(cfg.stateDir, `draws-${k}.json`), rpc, vault));
    return books.get(k)!;
  };
  return { signer, log, stack, fetchImpl, rpc, live, store, relays, logs, coinIndex, book, nowMs };
}

/** Everything the loop needs, or a thrown ConfigError / Refusal / SignerError that says why not. Exported for the tests. */
export async function build(argv: readonly string[], env: Readonly<Record<string, string | undefined>>, o: BuildOpts = {}): Promise<Built> {
  const cfg = parseServiceConfig(argv, env);
  const m = await buildCommon(cfg, env, o);
  const { signer, log, stack, rpc, store } = m;
  // who sends: the key's address in send mode; the registry's keeper in a dry run (simulated from that address, never signed)
  const from = (signer ? signer.address : await read<Hex>(rpc, stack.registry, REGISTRY, "keeper")) as Hex;
  // fixes-B-C-keeper-draws.md §5 item 5: the pause is read again immediately before every signature
  const guard = async () => ((await read<boolean>(rpc, stack.registry, REGISTRY, "keeperPaused")) ? PAUSED : null);
  const sender = new Sender({ rpc, chainId: cfg.chainId, from, signer, mode: cfg.mode, caps: cfg.caps, store, log, receiptWaitS: cfg.receiptWaitS, nowMs: o.nowMs, sleep: o.sleep,
    guard, stuckAfterS: cfg.stuckAfterS });
  const feed: FloorFeed = cfg.floorSource === "fixture" ? new FixtureFeed(cfg.floorFixture!)
    : new OrdinalsWalletFeed({ rate: productionRate(cfg.rateMaxDisagreeBps, m.fetchImpl), slugs: cfg.owSlugs, log });
  log("info", "start", { mode: cfg.mode, chainId: cfg.chainId, keeper: from, rpc: m.live, stack: stack.registry, collections: stack.collections.map((c) => c.slug), feed: feed.name,
    resolver: cfg.resolver.enabled, snapshotMirror: cfg.snapshots.s3 ? `s3 ${new URL(cfg.snapshots.s3.endpoint).host}/${cfg.snapshots.s3.bucket}/${cfg.snapshots.s3.prefix}` : cfg.snapshots.uploadUrl !== null ? "put" : null, statusFull: cfg.statusToken !== null,
    note: cfg.mode === "dry-run" ? "dry run: every call is simulated with eth_call; nothing is signed or sent" : "send mode: every call is simulated first, then signed and sent" });
  if (cfg.mode === "send" && cfg.chainId === 4663 && !hasMirror(cfg.snapshots)) log("warn", "no-snapshot-mirror", { note: "neither KEEPER_SNAPSHOT_S3_ENDPOINT nor KEEPER_SNAPSHOT_UPLOAD_URL is set: no draw is announced on 4663 until a mirror is configured" });
  return { cfg, stack, rpc, sender, feed, from, log, signer, store, snapshotDir: join(cfg.stateDir, "snapshots"),
    exclusionsFile: new URL("../../tools/snapshot/exclusions.json", import.meta.url).pathname, relays: m.relays, logs: m.logs, coinIndex: m.coinIndex, book: m.book,
    fetchImpl: m.fetchImpl, nowMs: m.nowMs, mem: newMemory() };
}

export function statusOf(b: Built, st: { startedAt: string; passes: number; lastPassAt: string | null; lastPassMs: number | null; consecutiveFailures: number;
  lastError: { at: string; message: string } | null; last: PassReport | null; alertPasses?: number; inForce?: string[] }, nowMs = Date.now()): Status {
  const s = b.store.get();
  return { mode: b.cfg.mode, chainId: b.cfg.chainId, keeper: b.from, rpc: b.rpc.hosts(), startedAt: st.startedAt, interval: b.cfg.intervalS, passes: st.passes, lastPassAt: st.lastPassAt,
    lastPassMs: st.lastPassMs, consecutiveFailures: st.consecutiveFailures, lastError: st.lastError, refusals: st.last?.refusals ?? [], alerts: st.inForce ?? st.last?.alerts ?? [],
    drawRule: `announced at slot T = k × ${b.cfg.draw.everyS} + ${b.cfg.draw.offsetS} (chain seconds), within ${b.cfg.draw.windowS} s (a missed slot is an alert); the snapshot is the last block stamped at or before the draw's snapshotAt, which its drand seed round fixes (docs/snapshot-v1.md "Version 2")`,
    gas: { day: utcDay(nowMs), spentOrReservedWei: String(b.store.committed(utcDay(nowMs))), capWei: String(b.cfg.caps.maxWeiPerDay), perTxCapWei: String(b.cfg.caps.maxWeiPerTx),
      maxGasPerTx: String(b.cfg.caps.maxGasPerTx), maxFeePerGasWei: String(b.cfg.caps.maxFeePerGasWei) },
    pending: s.pending ? { key: s.pending.key, what: s.pending.what, hash: s.pending.hash, nonce: s.pending.nonce, sentAt: s.pending.sentAt } : null,
    history: s.history.slice(0, 20), last: st.last, alertPasses: st.alertPasses ?? 0, alertPassesLimit: b.cfg.alertPasses };
}

async function main(): Promise<void> {
  const out = (l: string) => console.log(l);
  let b: Built;
  try { b = await build(process.argv.slice(2), process.env); } catch (err) {
    const problems = err instanceof ConfigError ? err.problems : [String((err as Error).message)];
    for (const p of problems) out(JSON.stringify({ at: new Date().toISOString(), level: "error", event: "refusing-to-start", problem: p }));
    process.exit(2);           // any failure to start is a refusal to start: exit 2 (a pass that is refused exits 1)
  }
  const st = { startedAt: new Date().toISOString(), passes: 0, lastPassAt: null as string | null, lastPassMs: null as number | null, consecutiveFailures: 0,
    lastError: null as { at: string; message: string } | null, last: null as PassReport | null, alertPasses: 0, inForce: [] as string[] };
  const latch = new AlertLatch(b.cfg.alertLatchMs);
  if (b.cfg.port !== null && !b.cfg.once) { serveStatus(() => statusOf(b, st), b.snapshotDir, b.cfg.port, (s) => (b.signer ? b.signer.scrub(s) : s), "::", b.cfg.statusToken); b.log("info", "status", { port: b.cfg.port }); }
  const once = async (): Promise<boolean> => {
    const t0 = Date.now();
    try {
      st.last = await runPass(b);
      st.passes++; st.lastPassAt = new Date().toISOString(); st.lastPassMs = Date.now() - t0; st.consecutiveFailures = 0;
      // R2-S8: alerts in force N passes in a row → /healthz 503; R3-check L-G: a one-pass or flapping alert stays in force a while
      const l = latch.pass(st.last.alerts, Date.now()); st.inForce = l.inForce; st.alertPasses = l.alertPasses;
      b.log(st.last.refusals.length ? "error" : "info", "pass", { passes: st.passes, ms: st.lastPassMs, refusals: st.last.refusals.length, block: st.last.block });
      return st.last.refusals.length === 0;
    } catch (err) {
      st.consecutiveFailures++; st.lastError = { at: new Date().toISOString(), message: String((err as Error).message).slice(0, 300) };
      b.log("error", "pass-failed", { error: st.lastError.message, consecutive: st.consecutiveFailures });
      return false;
    }
  };
  if (b.cfg.once) {
    const ok = await once();
    out(JSON.stringify(st.last, (_k, v) => (typeof v === "bigint" ? String(v) : v), 1).split("\n").map((l) => (b.signer ? b.signer.scrub(l) : l)).join("\n"));
    process.exit(ok ? 0 : 1);
  }
  for (const s of ["SIGINT", "SIGTERM"] as const) process.on(s, () => { b.log("info", "stop", { signal: s }); process.exit(0); });
  for (;;) {
    await once();
    await new Promise((ok) => setTimeout(ok, b.cfg.intervalS * 1000));
  }
}

if (import.meta.url === `file://${process.argv[1]}`) await main();
