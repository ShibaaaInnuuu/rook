import { readListings, rpc } from "./chain.js";
import { withBackoff, GaveUp } from "./backoff.js";
import { ConfigError, parseConfig } from "./config.js";
import { newHealth, serveHealth } from "./health.js";
import { Journal } from "./journal.js";
import { logger } from "./log.js";
import { plan } from "./plan.js";
import { FixtureQuoter, FixtureSource, LifiQuoter, MagicEdenSource, type PriceSource, type Quoter } from "./sources.js";

/**
 * docs/keeper-dryrun-v1.md, hardened in docs/phase4.md §5. Usage:
 *   npm run dryrun -- --registry 0x… [--rpc http://127.0.0.1:8545] [--source fixture|magiceden] [--prices fixtures/prices.json]
 *                     [--quoter fixture|lifi --btc bc1p… --from 0x…] [--watch 60 [--health-port 8790]] [--journal-dir DIR]
 * Reads, plans, journals. Dry-run is the default and the only mode: it cannot send (src/chain.ts, test/cannot-send.test.ts).
 * It refuses to start on a contradictory or unknown option, waits out a failing node with a ceiling, and never dies of one.
 */
const log = logger();
let cfg;
try { cfg = parseConfig(process.argv.slice(2), process.env); }
catch (err) { if (err instanceof ConfigError) { for (const p of err.problems) log("error", "config", { problem: p }); process.exit(2); } throw err; }

const source: PriceSource = cfg.source === "magiceden" ? new MagicEdenSource() : new FixtureSource(cfg.prices ?? new URL("../fixtures/prices.json", import.meta.url).pathname);
const quoter: Quoter = cfg.quoter === "lifi" ? new LifiQuoter(cfg.btc!, cfg.from!) : new FixtureQuoter(3_200_000n, 1_500n, 60n);
const journalDir = cfg.journalDir ?? new URL("../journal/", import.meta.url).pathname;
const journal = new Journal((day) => `${journalDir.replace(/\/$/, "")}/${day}.jsonl`);
const health = newHealth();
log("info", "start", { mode: cfg.mode, rpc: cfg.rpc, source: source.name, quoter: quoter.name, watchS: cfg.watchS, note: "reads, plans and journals; it has no signer and sends nothing" });

async function pass(): Promise<void> {
  const began = Date.now(), at = new Date().toISOString(), nowS = Math.floor(began / 1000);
  const retry = { onRetry: (attempt: number, waitMs: number, err: unknown) => log("warn", "rpc-retry", { attempt, waitMs, error: String((err as Error).message).slice(0, 160) }) };
  health.chainId = Number(BigInt(await withBackoff(() => rpc<string>(cfg!.rpc, "eth_chainId", []), retry)));
  const listings = await withBackoff(() => readListings(cfg!.rpc, cfg!.registry), retry);
  for (const l of listings) {
    try {
      const price = l.kind === "inscriptions" ? await source.inscriptionFloor(l.ref) : await source.runeUnitPrice(l.ref, l.name);
      // quoted at the size of what is spendable (at least the minimum), because the bridge's cost depends on size
      const size = l.spendableWei > 5n * 10n ** 16n ? l.spendableWei : 5n * 10n ** 16n;
      const quote = await quoter.quote(size);
      const actions = plan(l, price, quote, nowS, null);
      journal.append({ at, listing: l.id, name: l.name, kind: l.kind, source: source.name, quoter: quoter.name, read: l, price, quote, actions });
      log("info", "planned", { listing: l.id, name: l.name, actions: actions.map((a) => a.type).join(",") });
    } catch (err) {
      journal.append({ at, listing: l.id, name: l.name, error: String((err as Error).message).slice(0, 300) });
      log("warn", "no-plan", { listing: l.id, name: l.name, error: String((err as Error).message).slice(0, 200) });
    }
  }
  health.passes++; health.lastPassAt = new Date().toISOString(); health.lastPassMs = Date.now() - began; health.listingsRead = listings.length;
  health.lastError = null; health.consecutiveFailures = 0;
}

async function attempt(): Promise<void> {
  try { await pass(); } catch (err) {
    health.consecutiveFailures++;
    health.lastError = { at: new Date().toISOString(), kind: err instanceof GaveUp ? "gave-up" : "error" };
    log("error", "pass-failed", { kind: health.lastError.kind, error: String((err as Error).message).slice(0, 200), consecutive: health.consecutiveFailures });
    if (cfg!.watchS === 0) process.exitCode = 1;       // a single run reports its failure; a watch carries on to the next interval
  }
}

if (cfg.healthPort !== null) { serveHealth(() => health, cfg.watchS, cfg.healthPort); log("info", "health", { port: cfg.healthPort, path: "/healthz" }); }
await attempt();
if (cfg.watchS > 0) setInterval(() => void attempt(), cfg.watchS * 1000);
