import { join } from "node:path";
import type { Hex } from "viem";
import { REGISTRY } from "./abis.js";
import { ConfigError, parseServiceConfig } from "./config.js";
import { blockAt } from "./draws.js";
import { newMemory, passCtx, read } from "./keeper.js";
import { buildCommon } from "./main.js";
import { resolverPass, type ResolverDeps, type ResolverReport } from "./resolver.js";
import { Refusal } from "./rpc.js";
import { Sender } from "./sender.js";
import type { StateStore } from "./state.js";
import { AlertLatch, serveStatus, type Status } from "./status.js";
import type { Signer } from "./signer.js";

/**
 * docs/audit/2026-09-26/fixes-W2-services.md §2. The standalone, keyless resolver: `npm run resolver -- [--dry-run] [--once]`. It
 * sends only calls anyone may make (Wrapper.resolve, SweepVault.requestDraw / resolve / claim / voidDraw), from ANY funded account
 * (RESOLVER_PRIVATE_KEY, read in service/signer.ts only), so redemptions and raffles never depend on the keeper's key or process.
 * It reads the keeper's chain, RPC and stack variables (KEEPER_CHAIN_ID, KEEPER_RPC_URLS, KEEPER_STACK, KEEPER_DRAND_URLS, the gas
 * caps) and its own RESOLVER_MODE (send only when "send"), RESOLVER_STATE_DIR, RESOLVER_INTERVAL_S, RESOLVER_STATUS_PORT.
 * It refuses a key that is the registry's named keeper (the two would race for one nonce: run the keeper's in-process resolver
 * instead), and it keeps running while the keeper is paused — that is when it matters most.
 */
export type BuiltResolver = ResolverDeps & { from: Hex; store: StateStore; signer: Signer | null };

export async function buildResolver(argv: readonly string[], env: Readonly<Record<string, string | undefined>>, o: Parameters<typeof buildCommon>[2] = {}): Promise<BuiltResolver> {
  const cfg = parseServiceConfig(argv, env, "resolver");
  const m = await buildCommon(cfg, env, o);
  const named = (await read<Hex>(m.rpc, m.stack.registry, REGISTRY, "namedKeeper")).toLowerCase();
  // who sends: the resolver's own key; a dry run simulates from the zero-balance address 0x…01 (any caller may make these calls)
  const from = (m.signer ? m.signer.address : "0x0000000000000000000000000000000000000001") as Hex;
  if (m.signer && from.toLowerCase() === named)
    throw new Refusal("RESOLVER_PRIVATE_KEY is the registry's keeper: the standalone resolver runs with its own key (the keeper already resolves in its own process)");
  const sender = new Sender({ rpc: m.rpc, chainId: cfg.chainId, from, signer: m.signer, mode: cfg.mode, caps: cfg.caps, store: m.store, log: m.log, receiptWaitS: cfg.receiptWaitS,
    nowMs: o.nowMs, sleep: o.sleep, stuckAfterS: cfg.stuckAfterS });
  m.log("info", "start", { role: "resolver", mode: cfg.mode, chainId: cfg.chainId, from, rpc: m.live, stack: m.stack.registry, collections: m.stack.collections.map((c) => c.slug) });
  return { cfg, stack: m.stack, rpc: m.rpc, sender, log: m.log, relays: m.relays, logs: m.logs, coinIndex: m.coinIndex, book: m.book, fetchImpl: m.fetchImpl, nowMs: m.nowMs,
    mem: newMemory(), snapshotDir: join(cfg.stateDir, "snapshots"), exclusionsFile: new URL("../../tools/snapshot/exclusions.json", import.meta.url).pathname, from, store: m.store,
    signer: m.signer };
}

/** One resolver pass: what is in flight first (re-priced when stuck), then tickets and draws. */
export async function runResolverPass(b: BuiltResolver): Promise<ResolverReport & { alerts: string[]; pendingDetail: string | null }> {
  await b.rpc.init();
  const alerts: string[] = [];
  const ctx = passCtx(b, await blockAt(b.rpc, "latest"), alerts);
  const pending = await b.sender.resolvePending();
  if (pending?.alert) alerts.push(pending.alert);
  ctx.halted = pending !== null && pending.outcome === "pending";
  for (const c of b.stack.collections) await b.coinIndex(c).refresh().catch((err) => alerts.push(`coin-index: ${String((err as Error).message).slice(0, 160)}`));
  const r = await resolverPass(b, ctx);
  for (const a of alerts) b.log("error", "alert", { alert: a });
  return { ...r, alerts, pendingDetail: pending?.detail ?? null };
}

async function main(): Promise<void> {
  const out = (l: string) => console.log(l);
  let b: BuiltResolver;
  try { b = await buildResolver(process.argv.slice(2), process.env); } catch (err) {
    const problems = err instanceof ConfigError ? err.problems : [String((err as Error).message)];
    for (const p of problems) out(JSON.stringify({ at: new Date().toISOString(), level: "error", event: "refusing-to-start", problem: p }));
    process.exit(2);
  }
  const st = { startedAt: new Date().toISOString(), passes: 0, lastPassAt: null as string | null, failures: 0, last: null as Awaited<ReturnType<typeof runResolverPass>> | null, alertPasses: 0, inForce: [] as string[] };
  const latch = new AlertLatch(b.cfg.alertLatchMs);
  const status = (): Status => ({ mode: b.cfg.mode, chainId: b.cfg.chainId, keeper: b.from, rpc: b.rpc.hosts(), startedAt: st.startedAt, interval: b.cfg.intervalS, passes: st.passes,
    lastPassAt: st.lastPassAt, lastPassMs: null, consecutiveFailures: st.failures, lastError: null, refusals: [], alerts: st.inForce, drawRule: "resolver: permissionless calls only",
    gas: { day: "", spentOrReservedWei: "", capWei: String(b.cfg.caps.maxWeiPerDay), perTxCapWei: String(b.cfg.caps.maxWeiPerTx), maxGasPerTx: String(b.cfg.resolver.maxGasPerTx),
      maxFeePerGasWei: String(b.cfg.caps.maxFeePerGasWei) }, pending: b.store.get().pending ? { nonce: b.store.get().pending!.nonce } : null, history: [], last: null,
    alertPasses: st.alertPasses, alertPassesLimit: b.cfg.alertPasses });
  if (b.cfg.port !== null && !b.cfg.once) serveStatus(status, b.snapshotDir, b.cfg.port, (s) => (b.signer ? b.signer.scrub(s) : s), "::", b.cfg.statusToken);
  const once = async () => {
    try { st.last = await runResolverPass(b); st.passes++; st.lastPassAt = new Date().toISOString(); st.failures = 0; { const l = latch.pass(st.last.alerts, Date.now()); st.inForce = l.inForce; st.alertPasses = l.alertPasses; } b.log("info", "pass", { sent: st.last.sent, alerts: st.last.alerts.length }); return true; }
    catch (err) { st.failures++; b.log("error", "pass-failed", { error: String((err as Error).message).slice(0, 300) }); return false; }
  };
  if (b.cfg.once) {
    const ok = await once();
    out(JSON.stringify(st.last, (_k, v) => (typeof v === "bigint" ? String(v) : v), 1).split("\n").map((l) => (b.signer ? b.signer.scrub(l) : l)).join("\n"));
    process.exit(ok ? 0 : 1);
  }
  for (const s of ["SIGINT", "SIGTERM"] as const) process.on(s, () => { b.log("info", "stop", { signal: s }); process.exit(0); });
  for (;;) { await once(); await new Promise((ok) => setTimeout(ok, b.cfg.intervalS * 1000)); }
}

if (import.meta.url === `file://${process.argv[1]}`) await main();
