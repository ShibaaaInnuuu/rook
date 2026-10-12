import { createServer, type Server } from "node:http";
import type { Hex } from "viem";
import { DEFAULT_CAPS, DEFAULT_SOL_CAPS, LAMPORTS, T, WEI, type FuelCaps } from "./fuel/plan.js";
import { Engine, FileJournal, MemoryJournal, type EngineCfg, type Home } from "./fuel/engine.js";
import { realIo } from "./fuel/io.js";
import { fuelEvmSigner, fuelIcIdentity, fuelSolSigner } from "./fuel/keys.js";
import { FUND_WALLET, isPubkey } from "./fuel/sol.js";
import { realSolIo } from "./fuel/solio.js";
import { ETHEREUM, ROBINHOOD } from "./fuel/pinned.js";
import { IC_HOST } from "./ic.js";
import { envSecrets, logger, urlSecrets } from "./log.js";
import { RELAY_API_DEFAULT } from "./relay.js";

/**
 * docs/v7/cycles-from-fees.md §4.2: the fueler (v1). The platform's ops share of the fees pays the canisters' cycles, with no human:
 * claim the curves' platform fees → `allocate` the treasury (its ops share lands at this fueler's own address) → when the buyer is low,
 * Relay 4663 → Ethereum to itself → the ckETH helper → ICPSwap ckETH → ICP → the CMC's `notify_top_up` for the buyer (and the timelock).
 * Everything above the buffer goes to the cold payee each tick.
 *
 * It is a hot key on money (§2.2): bounded by the caps (src/fuel/plan.ts), by the allowlist of what it may sign (src/fuel/engine.ts
 * `allowed`), and watched from outside by the monitor's `ops-unexpected-tx` (a nonce on either chain the fueler did not journal).
 * FUELER_MODE=dry (the default) reads everything, plans, logs what it would do, and signs nothing.
 *
 *   FUELER_MODE            dry | send (default dry)
 *   FUELER_EVM_KEY         (secret) the ops EOA, the same address on 4663 and Ethereum   FUELER_IC_IDENTITY_PEM (secret) its own IC identity
 *   FUELER_BUYER, FUELER_TIMELOCK   the canisters it tops up (and nothing else)           FUELER_TREASURY  ProtocolTreasury on 4663
 *   FUELER_PAYEE           the operator's COLD address: everything above the buffer goes there
 *   FUELER_RPC_4663, FUELER_RPC_1   comma-separated, at least two each                    FUELER_INDEXER_URL  the curves to claim
 *   RELAY_API_URL, RELAY_API_KEY (secret; /quote/v2 requires one from 2026-10-02)          IC_HOST
 *   caps: FUELER_BUYER_LOW_T 40, FUELER_BUYER_TARGET_T 60, FUELER_TIMELOCK_LOW_T 1.5, FUELER_TIMELOCK_TARGET_T 2, FUELER_RUN_MAX_T 20,
 *         FUELER_WEEK_MAX_T 60, FUELER_DAY_MAX_ETH 0.10, FUELER_WEEK_MAX_ETH 0.25, FUELER_FLOOR_ETH 0.02, FUELER_BUFFER_ETH 0.15,
 *         FUELER_MIN_RUN_ETH 0.005, FUELER_MIN_GAP_S 21600, FUELER_CLAIM_MIN_ETH 0.002, FUELER_ALLOCATE_MIN_ETH 0.002,
 *         FUELER_ETH_MAX_FEE_GWEI 50, FUELER_STUCK_S 7200, FUELER_MINT_STUCK_S 3600
 *   FUELER_EVERY_S 1800, FUELER_STATE_DIR /data, PORT (FUELER_PORT) 8080, FUELER_WATCH_ADDRESS (a dry run with no key: whose balance to plan with)
 *   the operator's exits, read at start: FUELER_UNHALT=yes (clear a halt once its cause is fixed), FUELER_ABORT_RUN=<run id> (close a run
 *   whose money the operator has located); unset them again after the restart
 *
 * FUEL_HOME=books (the default, docs/v9/canister-only-solana.md §9.2: the canister-only Solana home; docs/v9/fueler-solana.md for the
 * Solana leg) | robinhood (the original, above). On the books home the first leg is the Solana hot key's (= the canister's `treasury.ops`):
 * the canister's own cranks pay the ops share to it once 0.1 SOL is owed (no program, no `allocate`: rook_core is retired, decision D-l),
 * the fueler reads the canister's `treasury_status` (FUELER_BUYER is that canister) for the ops key and the income mark, then Relay SOL
 * (Solana) → ETH (Ethereum) to the EVM key's address → the same helper / ckETH / ICPSwap / CMC legs. Everything above the SOL buffer plus
 * a day's cap goes to the cold payee.
 *   FUELER_SOL_KEYPAIR_FILE (secret file, or FUELER_SOL_KEYPAIR the same JSON sealed)   FUELER_RPC_SOL  two https providers at least
 *   FUELER_SOL_PAYEE       the cold payee (default the fund wallet GGgX…6Wuy, decision D-g)
 *   caps (SOL): FUELER_SOL_DAY_MAX 1, FUELER_SOL_WEEK_MAX 2.5, FUELER_SOL_FLOOR 0.01, FUELER_SOL_BUFFER 0.05, FUELER_SOL_MIN_RUN 0.1,
 *         FUELER_SOL_FORWARD_MIN 0.01; Relay: FUELER_SOL_RELAY_MAX_FEE_BPS 200 (S10 against the oracle), FUELER_SOL_SLIPPAGE_BPS 50;
 *         fees: FUELER_SOL_PRIORITY_MICROLAMPORTS 20000, FUELER_SOL_CU_LIMIT 100000
 *   FUELER_SOL_WATCH_ADDRESS (a dry run with no key: whose SOL balance to plan with, and the Relay quote's user)
 *   On the books home FUELER_TREASURY, FUELER_PAYEE, FUELER_INDEXER_URL and FUELER_RPC_4663 are not read; FUELER_EVM_KEY is the Ethereum leg's.
 *
 *   node --import tsx src/fueler.ts --dry-run [--assume-cycles-t N] [--assume-sol N]: one read-only tick against the configured networks with an
 *   in-memory journal, printing the plan (what it would forward, quote and deposit); nothing is signed or sent.
 */
export type FuelerConfig = { engine: EngineCfg; rpcs: Record<number, string[]>; indexer: string; relayApi: string; relayKey?: string; icHost: string;
  everyS: number; stateDir: string; port: number; unhalt: boolean; abortRun: string | null; home: Home; solRpcs: string[] };

const ADDR = /^0x[0-9a-fA-F]{40}$/;
const PRINCIPAL = /^[a-z0-9]{5}(-[a-z0-9]{5})*(-[a-z0-9]{1,5})?$/;

export function parseFuelerConfig(env: Readonly<Record<string, string | undefined>>, o: { dryRun?: boolean; assumeCyclesT?: number; assumeSol?: number } = {}): FuelerConfig {
  const problems: string[] = [];
  const home = (env.FUEL_HOME ?? "books").toLowerCase() as Home;
  if (home !== "robinhood" && home !== "books") problems.push(`FUEL_HOME=${env.FUEL_HOME} is neither books nor robinhood${home === ("solana" as string) ? " (the programs home is retired with rook_core: books)" : ""}`);
  const sol = home === "books";
  const num = (k: string, d: number, lo: number, hi: number) => {
    const v = env[k];
    if (v === undefined || v === "") return d;
    const n = Number(v);
    if (!Number.isFinite(n) || n < lo || n > hi) { problems.push(`${k}=${v} is not a number from ${lo} to ${hi}`); return d; }
    return n;
  };
  const eth = (k: string, d: bigint, hi: number) => BigInt(Math.round(num(k, Number(d) / 1e18, 0, hi) * 1e6)) * (WEI / 1_000_000n);
  const addr = (k: string, required = true): Hex => {
    const v = env[k];
    if (!v) { if (required) problems.push(`${k} is not set`); return "0x" as Hex; }
    if (!ADDR.test(v)) problems.push(`${k}=${v} is not an address`);
    return v.toLowerCase() as Hex;
  };
  const urls = (k: string, needed = true) => {
    const v = (env[k] ?? "").split(",").map((x) => x.trim()).filter(Boolean);
    if (!needed) return v;
    if (v.length < 2) problems.push(`${k}: at least two https providers (reads that move money are taken from both)`);
    for (const u of v) { try { if (new URL(u).protocol !== "https:") problems.push(`${k}: ${new URL(u).host} is not https`); } catch { problems.push(`${k}: a value is not a URL`); } }
    return v;
  };
  const mode = o.dryRun ? "dry" : (env.FUELER_MODE ?? "dry").toLowerCase();
  if (mode !== "dry" && mode !== "send") problems.push(`FUELER_MODE=${env.FUELER_MODE} is neither dry nor send`);
  const buyer = env.FUELER_BUYER ?? "";
  if (!PRINCIPAL.test(buyer)) problems.push("FUELER_BUYER is not a canister id");
  const timelock = env.FUELER_TIMELOCK || null;
  if (timelock !== null && !PRINCIPAL.test(timelock)) problems.push("FUELER_TIMELOCK is not a canister id");
  const sc = (k: string, d: bigint, hi: number) => BigInt(Math.round(num(k, Number(d) / 1e9, 0, hi) * 1e6)) * (LAMPORTS / 1_000_000n);
  const D = sol ? DEFAULT_SOL_CAPS : DEFAULT_CAPS;
  const caps: FuelCaps = {
    buyerLowT: num("FUELER_BUYER_LOW_T", DEFAULT_CAPS.buyerLowT, 8, 10_000), buyerTargetT: num("FUELER_BUYER_TARGET_T", DEFAULT_CAPS.buyerTargetT, 8, 10_000),
    timelockLowT: num("FUELER_TIMELOCK_LOW_T", DEFAULT_CAPS.timelockLowT, 0.5, 100), timelockTargetT: num("FUELER_TIMELOCK_TARGET_T", DEFAULT_CAPS.timelockTargetT, 0.5, 100),
    runMaxT: num("FUELER_RUN_MAX_T", DEFAULT_CAPS.runMaxT, 1, 1_000), weekMaxT: num("FUELER_WEEK_MAX_T", DEFAULT_CAPS.weekMaxT, 1, 5_000),
    ...(sol ? {
      dayMaxWei: sc("FUELER_SOL_DAY_MAX", D.dayMaxWei, 50), weekMaxWei: sc("FUELER_SOL_WEEK_MAX", D.weekMaxWei, 120),
      floorWei: sc("FUELER_SOL_FLOOR", D.floorWei, 10), bufferWei: sc("FUELER_SOL_BUFFER", D.bufferWei, 20), minRunWei: sc("FUELER_SOL_MIN_RUN", D.minRunWei, 20),
    } : {
      dayMaxWei: eth("FUELER_DAY_MAX_ETH", D.dayMaxWei, 2), weekMaxWei: eth("FUELER_WEEK_MAX_ETH", D.weekMaxWei, 5),
      floorWei: eth("FUELER_FLOOR_ETH", D.floorWei, 1), bufferWei: eth("FUELER_BUFFER_ETH", D.bufferWei, 2), minRunWei: eth("FUELER_MIN_RUN_ETH", D.minRunWei, 1),
    }),
    minGapS: num("FUELER_MIN_GAP_S", DEFAULT_CAPS.minGapS, 1_800, 7 * 86_400),
  };
  const N = sol ? { week: "FUELER_SOL_WEEK_MAX", day: "FUELER_SOL_DAY_MAX", buffer: "FUELER_SOL_BUFFER", floor: "FUELER_SOL_FLOOR" }
    : { week: "FUELER_WEEK_MAX_ETH", day: "FUELER_DAY_MAX_ETH", buffer: "FUELER_BUFFER_ETH", floor: "FUELER_FLOOR_ETH" };
  if (caps.buyerTargetT <= caps.buyerLowT) problems.push("FUELER_BUYER_TARGET_T must be above FUELER_BUYER_LOW_T");
  if (caps.timelockTargetT <= caps.timelockLowT) problems.push("FUELER_TIMELOCK_TARGET_T must be above FUELER_TIMELOCK_LOW_T");
  if (caps.weekMaxWei < caps.dayMaxWei) problems.push(`${N.week} must not be under ${N.day}`);
  if (caps.bufferWei <= caps.floorWei) problems.push(`${N.buffer} must be above ${N.floor}`);
  let solCfg: EngineCfg["sol"];
  if (sol) {
    if (env.FUELER_SOL_PROGRAM) problems.push("FUELER_SOL_PROGRAM is set: the books home has no rook_core (the canister pays the hot key itself); unset it");
    const payee = env.FUELER_SOL_PAYEE || FUND_WALLET;
    if (!isPubkey(payee)) problems.push(`FUELER_SOL_PAYEE=${payee} is not a Solana address`);
    const watch = env.FUELER_SOL_WATCH_ADDRESS || undefined;
    if (watch !== undefined && !isPubkey(watch)) problems.push("FUELER_SOL_WATCH_ADDRESS is not a Solana address");
    solCfg = { payee, forwardMinLamports: sc("FUELER_SOL_FORWARD_MIN", LAMPORTS / 100n, 10),
      relayMaxFeeBps: num("FUELER_SOL_RELAY_MAX_FEE_BPS", 200, 10, 1_000), slippageBps: num("FUELER_SOL_SLIPPAGE_BPS", 50, 1, 500),
      priceMicro: BigInt(Math.round(num("FUELER_SOL_PRIORITY_MICROLAMPORTS", 20_000, 0, 5_000_000))), units: num("FUELER_SOL_CU_LIMIT", 100_000, 20_000, 1_400_000),
      ...(watch ? { watchAddress: watch } : {}) };
  }
  const engine: EngineCfg = { mode: mode as EngineCfg["mode"], caps, buyer, timelock, treasury: sol ? ("0x" as Hex) : addr("FUELER_TREASURY"), payee: sol ? ("0x" as Hex) : addr("FUELER_PAYEE"),
    claimMinWei: eth("FUELER_CLAIM_MIN_ETH", WEI / 500n, 1), allocateMinWei: eth("FUELER_ALLOCATE_MIN_ETH", WEI / 500n, 1),
    ethMaxFeeWei: BigInt(Math.round(num("FUELER_ETH_MAX_FEE_GWEI", 50, 1, 1_000) * 1e9)), stuckS: num("FUELER_STUCK_S", 7_200, 600, 86_400),
    mintStuckS: num("FUELER_MINT_STUCK_S", 3_600, 1_200, 86_400), ...(env.FUELER_WATCH_ADDRESS ? { watchAddress: addr("FUELER_WATCH_ADDRESS", false) } : {}),
    home, ...(solCfg ? { sol: solCfg } : {}),
    ...(o.dryRun && o.assumeCyclesT !== undefined ? { assumeCycles: BigInt(Math.round(o.assumeCyclesT * 1e6)) * (T / 1_000_000n) } : {}),
    ...(o.dryRun && sol && o.assumeSol !== undefined ? { assumeBalance: BigInt(Math.round(o.assumeSol * 1e6)) * (LAMPORTS / 1_000_000n) } : {}) };
  if (engine.payee !== "0x" && engine.payee === engine.treasury) problems.push("FUELER_PAYEE must be the operator's cold address, not the treasury");
  const rpcs = { [ROBINHOOD]: urls("FUELER_RPC_4663", !sol), [ETHEREUM]: urls("FUELER_RPC_1") };
  const solRpcs = sol ? urls("FUELER_RPC_SOL") : [];
  const indexer = (env.FUELER_INDEXER_URL ?? "").replace(/\/$/, "");
  if (!sol && !/^https?:\/\//.test(indexer)) problems.push("FUELER_INDEXER_URL is not set (the curves whose platform fees it claims)");
  const relayApi = env.RELAY_API_URL ?? RELAY_API_DEFAULT;
  try { if (new URL(relayApi).protocol !== "https:") problems.push("RELAY_API_URL must be https"); } catch { problems.push("RELAY_API_URL is not a URL"); }
  const unhalt = (env.FUELER_UNHALT ?? "") === "yes";
  if (env.FUELER_UNHALT && !unhalt) problems.push("FUELER_UNHALT is `yes` or unset");
  const abortRun = env.FUELER_ABORT_RUN || null;
  if (abortRun !== null && !/^run-\d+$/.test(abortRun)) problems.push("FUELER_ABORT_RUN names a run id (run-<seconds>, from /status)");
  if (problems.length) throw new Error(problems.join("; "));
  return { engine, rpcs, indexer, relayApi, relayKey: env.RELAY_API_KEY || undefined, icHost: env.IC_HOST ?? IC_HOST,
    everyS: num("FUELER_EVERY_S", 1_800, 60, 86_400), stateDir: env.FUELER_STATE_DIR ?? "/data", port: num("PORT", num("FUELER_PORT", 8080, 1, 65_535), 1, 65_535),
    unhalt, abortRun, home, solRpcs };
}

/** /status: what the monitor reads (the nonces it holds the chain to, the alerts it pages); /healthz: the loop is alive. */
export function statusBody(e: Engine, cfg: FuelerConfig, address: string | null, principal: string, notes: string[]) {
  const s = e.s;
  return { mode: cfg.engine.mode, home: cfg.home, address, solAddress: e.solAddr, principal, notes, nonces: s.nonces,
    // the monitor's Solana watch: every signature this fueler signed with the hot key (journaled before it was sent)
    sol: cfg.home !== "robinhood" ? { pending: s.sol.pending ? { kind: s.sol.pending.kind, sig: s.sol.pending.sig, at: s.sol.pending.at } : null,
      signatures: s.sol.sent.map((x) => x.sig).concat(s.sol.pending ? [s.sol.pending.sig] : []), payee: cfg.engine.sol?.payee } : null, halted: s.halted, lastTickAt: s.lastTickAt, lastDecision: s.lastDecision,
    run: s.run ? { id: s.run.id, phase: s.run.phase, since: s.run.phaseAt, wei: String(s.run.wei), note: s.run.note ?? null } : null,
    pending: Object.fromEntries(Object.entries(s.pending).map(([k, p]) => [k, p ? { kind: p.kind, nonce: p.nonce, hash: p.hash, at: p.at } : null])),
    alerts: Object.entries(s.alerts).map(([kind, a]) => ({ kind, ...a })),
    caps: { ...cfg.engine.caps, dayMaxWei: String(cfg.engine.caps.dayMaxWei), weekMaxWei: String(cfg.engine.caps.weekMaxWei), floorWei: String(cfg.engine.caps.floorWei),
      bufferWei: String(cfg.engine.caps.bufferWei), minRunWei: String(cfg.engine.caps.minRunWei) },
    conversions: s.conversions.map((c) => ({ at: c.at, wei: String(c.wei), cycles: String(c.cycles) })), history: s.history };
}

export function serve(port: number, body: () => unknown, healthy: () => boolean): Server {
  return createServer((req, res) => {
    const path = (req.url ?? "/").split("?")[0];
    const json = (code: number, v: unknown) => { res.writeHead(code, { "content-type": "application/json" }); res.end(JSON.stringify(v, (_k, x) => (typeof x === "bigint" ? String(x) : x))); };
    if (path === "/healthz") return healthy() ? json(200, { ok: true }) : json(503, { ok: false, why: "no tick has completed for three periods" });
    if (path === "/status") return json(200, body());
    json(404, { error: "not found" });
  }).listen(port, "::");
}

async function main() {
  const argv = process.argv.slice(2), dryRun = argv.includes("--dry-run");
  const opt = (k: string) => { const i = argv.indexOf(k); if (i < 0) return undefined; const n = Number(argv[i + 1]); if (!(Number.isFinite(n) && n >= 0)) { console.error(`${k} takes a number`); process.exit(2); } return n; };
  const assumeCyclesT = opt("--assume-cycles-t"), assumeSol = opt("--assume-sol");
  const scrubs: ((l: string) => string)[] = [];
  const secrets = () => [...envSecrets(), ...urlSecrets(process.env.FUELER_RPC_4663), ...urlSecrets(process.env.FUELER_RPC_1), ...urlSecrets(process.env.FUELER_RPC_SOL),
    process.env.RELAY_API_KEY ?? ""].filter(Boolean);
  // every line goes through the keys' own scrubs too (keys.ts), so not even a stray copy of a key could be printed
  const log = logger((l) => console.log(scrubs.reduce((x, f) => f(x), l)), secrets);
  let cfg: FuelerConfig;
  try { cfg = parseFuelerConfig(process.env, { dryRun, assumeCyclesT, assumeSol }); } catch (err) { log("error", "refusing-to-start", { problem: (err as Error).message }); process.exit(2); }
  let signer, identity, solSigner;
  try { signer = fuelEvmSigner(process.env); identity = fuelIcIdentity(process.env); solSigner = cfg.home !== "robinhood" ? fuelSolSigner(process.env) : null; }
  catch (err) { log("error", "refusing-to-start", { problem: (err as Error).message }); process.exit(2); }
  if (signer) scrubs.push(signer.scrub);
  if (solSigner) scrubs.push(solSigner.scrub);
  if (cfg.engine.mode === "send" && (!signer || !identity)) { log("error", "refusing-to-start", { problem: "FUELER_MODE=send needs FUELER_EVM_KEY and FUELER_IC_IDENTITY_PEM" }); process.exit(2); }
  if (cfg.engine.mode === "send" && cfg.home !== "robinhood" && !solSigner) { log("error", "refusing-to-start", { problem: "FUELER_MODE=send on the books home needs the hot key (FUELER_SOL_KEYPAIR_FILE or FUELER_SOL_KEYPAIR)" }); process.exit(2); }
  if (!cfg.relayKey) log("warn", "relay-key-missing", { note: "RELAY_API_KEY is not set: Relay's /quote/v2 requires one from 2026-10-02; quotes may be refused" });
  const io = await realIo({ rpcs: cfg.rpcs, relayApi: cfg.relayApi, relayKey: cfg.relayKey, indexer: cfg.indexer, icHost: cfg.icHost, identity: identity ?? null });
  const sol = cfg.home !== "robinhood" ? { io: realSolIo(cfg.solRpcs), signer: solSigner ?? null } : undefined;
  const engine = new Engine({ io, signer: signer ?? null, journal: dryRun ? new MemoryJournal() : new FileJournal(cfg.stateDir), cfg: cfg.engine, log, sol });
  if (dryRun) {
    // one read-only tick, then the plan; the journal is in memory, nothing is signed (mode dry) and the process exits
    const notes = await engine.preflight();
    await engine.tick();
    const out = { home: cfg.home, mode: cfg.engine.mode, address: engine.solAddr, ethereum: signer?.address ?? cfg.engine.watchAddress ?? null, principal: io.principal(),
      treasury: cfg.engine.sol ? null : cfg.engine.treasury, payee: cfg.engine.sol?.payee ?? cfg.engine.payee, notes, plan: engine.plan,
      decision: engine.s.lastDecision, alerts: engine.s.alerts };
    console.log(scrubs.reduce((x, f) => f(x), JSON.stringify(out, (_k, v) => (typeof v === "bigint" ? String(v) : v), 2)));
    process.exit(0);
  }
  let notes: string[] = engine.operatorExits({ unhalt: cfg.unhalt, abortRun: cfg.abortRun });
  try { notes = [...notes, ...(await engine.preflight())]; } catch (err) { log("error", "refusing-to-start", { problem: (err as Error).message }); process.exit(2); }
  for (const n of notes) log("warn", "preflight", { note: n });
  log("info", "started", { mode: cfg.engine.mode, home: cfg.home, address: signer?.address ?? null, solAddress: engine.solAddr, principal: io.principal(), buyer: cfg.engine.buyer,
    timelock: cfg.engine.timelock, everyS: cfg.everyS });
  serve(cfg.port, () => statusBody(engine, cfg, signer?.address ?? null, io.principal(), notes),
    () => engine.s.lastTickAt === null || Date.now() / 1000 - engine.s.lastTickAt < 3 * cfg.everyS);
  for (const sig of ["SIGINT", "SIGTERM"] as const) process.on(sig, () => process.exit(0));
  let busy = false;
  const tick = async () => { if (busy) return; busy = true; try { await engine.tick(); } finally { busy = false; } };
  await tick();
  // a run in flight is advanced every minute (its legs take seconds to minutes); otherwise every FUELER_EVERY_S
  let last = Date.now();
  setInterval(() => { if (engine.s.run || Object.keys(engine.s.pending).length || engine.s.sol.pending || Date.now() - last >= cfg.everyS * 1000) { last = Date.now(); void tick(); } }, 60_000);
}

if (process.argv[1] && /fueler\.ts$/.test(process.argv[1])) void main();
