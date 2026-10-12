#!/usr/bin/env node
// K6: watcher v2 and the custody canary, as one always-on service (docs/v6/k6-watcher-canary.md).
//
//   node monitor/src/main.mjs --config monitor.json [--once] [--dry-run] [--port 8080]
//   node monitor/src/main.mjs --test-alert all|<kind> [--dry-run]      one of each kind to the operator's phone (G9)
//
// Environment: ALERT_TELEGRAM_BOT_TOKEN, ALERT_TELEGRAM_CHAT_ID (both required: without them it refuses to start, unless
// --dry-run); MONITOR_CONFIG (a path) or MONITOR_CONFIG_JSON (the JSON) instead of --config; PORT for the status endpoint;
// MONITOR_GUARDIAN_KEY only if the config names a guardian (src/guardian.mjs, off otherwise).
// Every read is public and anonymous. Without a guardian it holds no key and sends no transaction; with one, it sends only
// `pauseKeeper()` and `cancelDraw(id)`, and only on a confirmed trigger (docs/audit/2026-09-26/fixes-W2-monitor.md §1.4).
import { createServer } from "node:http";
import { dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { KINDS, Telegram, deliver, format, heartbeat, heartbeatKey, prune } from "./alerts.mjs";
import { checkBitcoin } from "./canary.mjs";
import { checkChain } from "./chain.mjs";
import { loadConfig, redact } from "./config.mjs";
import { checkCanister } from "./custodyhealth.mjs";
import { canaryProviders, checkEvm } from "./evm.mjs";
import { checkEthNft, sleepingModules } from "./ethnft.mjs";
import { checkFueler } from "./fueler.mjs";
import { guardianFrom } from "./guardian.mjs";
import { indexesFrom } from "./holders.mjs";
import { icClient } from "./ic.mjs";
import { checkLiveness } from "./liveness.mjs";
import { sample } from "./samples.mjs";
import { stackOf } from "./stack.mjs";
import { loadState, saveState } from "./state.mjs";
import { checkWatcher } from "./watcher.mjs";

export const STALE_MS = 30 * 60 * 1000;

export function parseArgs(argv) {
  const o = { once: false, dryRun: false };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i], v = () => { if (i + 1 >= argv.length) throw new Error(`${a} needs a value`); return argv[++i]; };
    if (a === "--config") o.config = v();
    else if (a === "--once") o.once = true;
    else if (a === "--dry-run") o.dryRun = true;
    else if (a === "--port") o.port = Number(v());
    else if (a === "--test-alert") o.testAlert = v();
    else throw new Error(`unknown argument ${a}`);
  }
  if (o.testAlert && o.testAlert !== "all" && !KINDS.includes(o.testAlert)) throw new Error(`--test-alert takes all or one of: ${KINDS.join(", ")}`);
  return o;
}

/** `--test-alert`: one message per kind, marked as a test, each made by the real code from the fixtures. */
export async function testAlerts(which, channel, now = Date.now()) {
  const kinds = which === "all" ? KINDS : [which];
  for (const k of kinds) await channel.send(format(sample(k, now), { test: true }));
  return kinds.length;
}

/**
 * What the EVM checker keeps between polls in memory, not in the state: the holder indexes (each in its own file next to the
 * state, fixes-R3-services.md §1.1) and the canary's two providers (their learned page spans and throttle cool-downs).
 */
export function evmDeps(cfg, http = fetch, dir = cfg.state ? dirname(cfg.state) : null) {
  if (!cfg.evm) return null;
  const evm = { ...cfg.evm, draws: cfg.draws };
  const st = stackOf(evm);
  return { indexes: st.fromBlock !== null && st.collections.length ? indexesFrom(evm, st, { http, dir }) : null, providers: canaryProviders(evm) };
}

/** The checkers this configuration turns on. */
function checkers(cfg, ic, http, guardian = null, deps = null) {
  const readRecords = () => ic.records(cfg.buyer);
  const list = [["watcher", "watch", (prev, now) => checkWatcher({ cfg, prev, ic, now })]];
  if (cfg.bitcoin) list.push(["canary", "btc", (prev, now) => checkBitcoin({ cfg: { ...cfg.bitcoin, acknowledged: cfg.acknowledged },
    prev, now, http, readRecords, timelock: cfg.timelock })]);
  // docs/v7/dormant-collections.md: the no-good-post pause skips the collections the canister keeps DORMANT or WAKING
  const sleeping = cfg.eth_nft && cfg.buyer && ic.ethState ? async () => sleepingModules(await ic.ethState(cfg.buyer), cfg.eth_nft) : null;
  if (cfg.evm) list.push(["evm", "evm", (prev, now) => checkEvm({ cfg: { ...cfg.evm, draws: cfg.draws, acknowledged: cfg.acknowledged }, prev, now, http,
    readRecords, timelock: cfg.timelock, guardian, indexes: deps?.indexes ?? null, providers: deps?.providers ?? null, sleeping })]);
  if (cfg.evm && (cfg.evm.stack || cfg.evm.registry)) list.push(["chain", "chain", (prev, now) => checkChain({ cfg: cfg.evm, prev, now, http })]);
  if (cfg.buyer && ic.ticketLogs) list.push(["canister", "canister", (prev, now) => checkCanister({ cfg, prev, now, ic })]);
  if (cfg.liveness?.length) list.push(["liveness", "liveness", (prev, now) => checkLiveness({ cfg, prev, now, http })]);
  // docs/v7/phase1-nft.md §6: the Ethereum NFT side
  if (cfg.eth_nft) list.push(["ethnft", "ethnft", (prev, now) => checkEthNft({ cfg, prev, now, http, ic: ic.ethState ? ic : null, guardian })]);
  // docs/v7/cycles-from-fees.md §4.2 item 7: the fueler's key, watched from the chain
  if (cfg.fueler) list.push(["fueler", "fueler", (prev, now) => checkFueler({ cfg, prev, now, http, ic })]);
  return list;
}

/**
 * One poll of every checker. A checker's new state is committed only when every alert it produced was delivered (or was
 * already sent): a failed post means the same poll's alerts are produced, and retried, next time.
 */
export async function pollOnce({ cfg, state, ic, http = fetch, channel, now = Date.now(), guardian = null, deps = null }) {
  state.startedAt ??= now;
  state.checkers ??= {};
  for (const [name, part, run] of checkers(cfg, ic, http, guardian, deps)) {
    const c = (state.checkers[name] ??= {});
    try {
      const r = await run(state[part], now);
      const d = await deliver(r.alerts, state, channel, now);
      if (d.failed) { c.lastError = "an alert could not be posted; the poll is retried"; c.lastErrorAt = now; continue; }
      state[part] = r.next;
      c.lastOk = now; c.lastError = null;
      // audit E-06: a checker that completes while blind (a range disputed for an hour, a stalled cursor) is NOT healthy
      c.degraded = r.degraded ?? null;
    } catch (err) {
      c.lastError = redact(String(err.message)).slice(0, 300); c.lastErrorAt = now;
    }
  }
  // silence is not safety: a checker that has not completed for STALE_MS is an alert of its own
  const stale = [];
  for (const [name] of checkers(cfg, ic, http, guardian)) {
    const c = state.checkers[name], since = c.lastOk ?? state.startedAt;
    if (now - since >= STALE_MS) stale.push({ kind: "watcher-error", sev: "SEV2", persistent: true, key: `checker-down:${name}:${since}`,
      text: `the ${name} checker has not completed a poll for ${Math.round((now - since) / 60000)} min${c.lastError ? ` (last error: ${c.lastError})` : ""}. Silence is not safety: check the monitor's /status.` });
  }
  await deliver(stale, state, channel, now);
  if (heartbeatKey(now, cfg.heartbeat_hour_utc, state.sent)) await deliver([heartbeat(now, { watched: watched(cfg), checkers: state.checkers, log: state.log })], state, channel, now);
  state.sent = prune(state.sent ?? {}, now);
  return state;
}

export function watched(cfg) {
  return [`timelock ${cfg.timelock}${cfg.buyer ? `, buyer ${cfg.buyer}` : ""} (${cfg.network})`,
    ...Object.entries(cfg.bitcoin?.addresses ?? {}).map(([l, a]) => `${l} ${a}`),
    ...(cfg.evm ? [cfg.evm.stack ? `4663: the stack of registry ${cfg.evm.stack.registry} (${Object.keys(cfg.evm.stack.collections ?? {}).length} collection(s)): wrappers, modules, vaults${cfg.evm.guardian ? `; guardian ${cfg.evm.guardian.address}` : "; no guardian"}`
      : `4663: wrappers ${(cfg.evm.wrappers ?? []).join(", ")}${cfg.evm.floor_module ? `; FloorModule ${cfg.evm.floor_module}` : ""}`] : []),
    ...(cfg.fueler ? [cfg.fueler.sol_ops ? `the fueler's Solana hot key ${cfg.fueler.sol_ops} and its Ethereum key ${cfg.fueler.ops}` : `the fueler's ops key ${cfg.fueler.ops} on 4663 and Ethereum`] : []),
    ...(cfg.liveness ?? []).map((l) => `service ${l.name}`)];
}

/** The status endpoint: what each checker last saw. Nothing secret is in the state, and the channel is not reachable here. */
export function statusBody(cfg, state, now = Date.now()) {
  // healthy while no checker has been silent for STALE_MS (counting from the start, so a fresh boot is healthy)
  const ok = Object.values(state.checkers ?? {}).every((c) => now - (c.lastOk ?? state.startedAt ?? now) < STALE_MS && !c.degraded);
  return { ok, now: new Date(now).toISOString(), started: state.startedAt ? new Date(state.startedAt).toISOString() : null, watched: watched(cfg),
    checkers: Object.fromEntries(Object.entries(state.checkers ?? {}).map(([k, c]) => [k, {
      lastOk: c.lastOk ? new Date(c.lastOk).toISOString() : null, lastError: c.lastError ?? null,
      lastErrorAt: c.lastErrorAt ? new Date(c.lastErrorAt).toISOString() : null, degraded: c.degraded ?? null }])),
    evm_cursor: state.evm?.cursor ?? null, evm_disputed: state.evm?.disputed ?? [],
    draws: Object.values(state.evm?.draws ?? {}).map((d) => ({ vault: d.vault, id: d.id, status: d.status, challengeEnds: d.challengeEnds ?? null, matches: d.matches ?? null })),
    holder_indexes: state.evm?.indexes ?? null, draw_exclusions: cfg.draws?.exclusions_sha256 ? { file: cfg.draws.exclusions_file, sha256: cfg.draws.exclusions_sha256 } : null,
    timelock_events_seen: state.watch?.seen ?? null,
    // go-live (docs/v7/golive-runbook.md §6): each canister's balance, its measured burn a day and the days that leaves
    cycles: state.watch?.burn_report ?? null,
    // docs/v7/dormant-collections.md §11: how many collections the buyer's cycles balance keeps awake (effective_max_active under the
    // ceiling), how many are, how many hold positions, and the wakes that would start now
    eth_capacity: state.ethnft?.capacity ?? null,
    open_custody_alerts: Object.values(state.btc?.addresses ?? {}).reduce((n, a) => n + Object.keys(a.open ?? {}).length, 0),
    alerts_last_24h: (state.log ?? []).filter((x) => now - x.at < 86400_000).map((x) => ({ at: new Date(x.at).toISOString(), sev: x.sev, kind: x.kind, head: x.head })) };
}

export function serve(cfg, getState, port) {
  const server = createServer((req, res) => {
    const body = statusBody(cfg, getState());
    if (req.url === "/healthz") { res.statusCode = body.ok ? 200 : 503; res.end(body.ok ? "ok\n" : "stale\n"); return; }
    if (req.url === "/status" || req.url === "/") { res.setHeader("content-type", "application/json"); res.end(JSON.stringify(body, null, 2)); return; }
    res.statusCode = 404; res.end();
  });
  server.listen(port);
  return server;
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  let o;
  try { o = parseArgs(process.argv.slice(2)); } catch (err) { console.error(`refused: ${err.message}`); process.exit(2); }
  let channel;
  try { channel = new Telegram({ dryRun: o.dryRun }); } catch (err) { console.error(err.message); process.exit(2); }
  if (o.testAlert) {
    try { const n = await testAlerts(o.testAlert, channel); console.log(`sent ${n} test alert(s)${channel.dryRun ? " (dry run: printed, not posted)" : ""}`); process.exit(0); }
    catch (err) { console.error(`test alerts failed: ${err.message}`); process.exit(1); }
  }
  let cfg;
  try { cfg = loadConfig({ path: o.config ?? process.env.MONITOR_CONFIG }); } catch (err) { console.error(`refused: ${err.message}`); process.exit(2); }
  let guardian;
  try { guardian = guardianFrom(cfg); } catch (err) { console.error(`refused: ${err.message}`); process.exit(2); }
  // not fatal: a monitor must not crash-loop because an RPC is down at start. The chain checker pages a guardian the registry does
  // not name, and every action runs the same preflight first (a failure pages "act by hand").
  if (guardian) await guardian.preflight().catch((err) => console.error(`warning: ${err.message}`));
  const ic = await icClient({ host: cfg.ic_host ?? (cfg.network === "local" ? "http://127.0.0.1:4943" : "https://icp-api.io"), timelock: cfg.timelock, local: cfg.network === "local" });
  let state = loadState(cfg.state);
  const deps = evmDeps(cfg);
  if (cfg.draws?.exclusions_sha256) console.log(`monitor: draw exclusions from ${cfg.draws.exclusions_file} (sha256 ${cfg.draws.exclusions_sha256}, ${cfg.draws.exclusions.length} address(es))`);
  const port = o.port ?? Number(process.env.PORT ?? 8080);
  if (!o.once) serve(cfg, () => state, port);
  console.log(`monitor: watching ${watched(cfg).join("; ")}; every ${cfg.every_secs} s${o.once ? "" : `; status on :${port}/status`}${channel.dryRun ? "; DRY RUN" : ""}${guardian ? `; guardian ${guardian.address} ARMED (pause: ${guardian.autoPause}, cancel: ${guardian.autoCancel})` : "; no guardian"}`);
  for (;;) {
    state = await pollOnce({ cfg, state, ic, channel, guardian, deps });
    saveState(cfg.state, state);
    const errs = Object.entries(state.checkers).filter(([, c]) => c.lastError).map(([k, c]) => `${k}: ${c.lastError}`);
    if (errs.length) console.error(`poll: ${errs.join("; ")}`);
    if (o.once) process.exit(errs.length ? 1 : 0);
    await new Promise((r) => setTimeout(r, cfg.every_secs * 1000));
  }
}
