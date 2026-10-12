/**
 * docs/phase4.md §5. The keeper's configuration, validated as a whole: it REFUSES TO START on anything contradictory or unknown,
 * listing every problem at once. Dry-run is the default and, outside a local node, the only mode: there is no flag or variable
 * that makes src/ send, because src/ has no code that could.
 */
export type Config = { mode: "dry-run"; registry: string; rpc: string; source: "fixture" | "magiceden"; prices: string | null; quoter: "fixture" | "lifi";
  btc: string | null; from: string | null; watchS: number; healthPort: number | null; journalDir: string | null };

export class ConfigError extends Error {
  constructor(readonly problems: string[]) { super(`refusing to start:\n  - ${problems.join("\n  - ")}`); this.name = "ConfigError"; }
}

const FLAGS = ["registry", "rpc", "source", "prices", "quoter", "btc", "from", "watch", "health-port", "journal-dir", "mode"] as const;
const isLocal = (url: string): boolean => { try { return ["127.0.0.1", "localhost", "[::1]", "::1"].includes(new URL(url).hostname); } catch { return false; } };

export function parseConfig(argv: readonly string[], env: Readonly<Record<string, string | undefined>> = {}): Config {
  const problems: string[] = [], got = new Map<string, string>();
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (!a.startsWith("--")) { problems.push(`"${a}" is not a flag`); continue; }
    const name = a.slice(2);
    // an unknown flag's value is part of the same mistake, not a second one
    if (!(FLAGS as readonly string[]).includes(name)) { problems.push(`unknown flag --${name}`); if (argv[i + 1] !== undefined && !argv[i + 1].startsWith("--")) i++; continue; }
    const v = argv[i + 1];
    if (v === undefined || v.startsWith("--")) { problems.push(`--${name} needs a value`); continue; }
    if (got.has(name)) problems.push(`--${name} is given twice`);
    got.set(name, v); i++;
  }
  const mode = got.get("mode") ?? "dry-run";
  if (mode !== "dry-run") problems.push(`--mode ${mode}: this package has one mode, dry-run. It reads, plans and journals; it has no signer and no code that sends`);
  const registry = got.get("registry") ?? "";
  if (!/^0x[0-9a-fA-F]{40}$/.test(registry)) problems.push(got.has("registry") ? `--registry ${registry} is not an address` : "missing --registry");
  const rpc = got.get("rpc") ?? "http://127.0.0.1:8545";
  try { const u = new URL(rpc); if (!/^https?:$/.test(u.protocol)) problems.push(`--rpc must be http(s), not ${u.protocol}`); } catch { problems.push(`--rpc ${rpc} is not a URL`); }
  // KEEPER_MODE=local is what the end-to-end runner needs before it will send to a node that says 31337. Set while THIS program is
  // pointed at anything that is not this machine, it is a contradiction worth stopping for: somebody believes the run is on
  // their own machine, and it is not.
  if (env.KEEPER_MODE !== undefined && env.KEEPER_MODE !== "" && env.KEEPER_MODE !== "local") problems.push(`KEEPER_MODE=${env.KEEPER_MODE} means nothing: it is "local" or unset`);
  if (env.KEEPER_MODE === "local" && !isLocal(rpc)) problems.push(`KEEPER_MODE=local with --rpc ${(() => { try { return new URL(rpc).host; } catch { return "?"; } })()}: local means this machine`);
  const source = got.get("source") ?? "fixture", quoter = got.get("quoter") ?? "fixture";
  if (source !== "fixture" && source !== "magiceden") problems.push(`--source ${source}: fixture or magiceden`);
  if (quoter !== "fixture" && quoter !== "lifi") problems.push(`--quoter ${quoter}: fixture or lifi`);
  if (source === "magiceden" && got.has("prices")) problems.push("--prices is the fixture source's file, and --source is magiceden");
  if (quoter === "lifi" && !got.has("btc")) problems.push("--quoter lifi needs --btc <the Bitcoin address a quote would deliver to>");
  if (quoter === "lifi" && !got.has("from")) problems.push("--quoter lifi needs --from <the address a quote would spend from>");
  if (quoter === "fixture" && (got.has("btc") || got.has("from"))) problems.push("--btc and --from belong to --quoter lifi, and the quoter is fixture");
  if (got.has("from") && !/^0x[0-9a-fA-F]{40}$/.test(got.get("from")!)) problems.push(`--from ${got.get("from")} is not an address`);
  if (got.has("btc") && !/^(bc1|tb1|bcrt1)[0-9a-z]{20,90}$/.test(got.get("btc")!)) problems.push("--btc is not a bech32 Bitcoin address");
  const watchS = got.has("watch") ? Number(got.get("watch")) : 0;
  if (!Number.isInteger(watchS) || watchS < 0) problems.push(`--watch ${got.get("watch")} is not a whole number of seconds`);
  else if (watchS !== 0 && watchS < 15) problems.push(`--watch ${watchS}: under 15 s is a hammer, not a watch (0 runs once)`);
  const healthPort = got.has("health-port") ? Number(got.get("health-port")) : null;
  if (healthPort !== null && (!Number.isInteger(healthPort) || healthPort < 1 || healthPort > 65_535)) problems.push(`--health-port ${got.get("health-port")} is not a port`);
  if (healthPort !== null && watchS === 0) problems.push("--health-port without --watch: a run that ends has no health to report");
  if (problems.length) throw new ConfigError(problems);
  return { mode: "dry-run", registry, rpc, source: source as Config["source"], prices: got.get("prices") ?? null, quoter: quoter as Config["quoter"], btc: got.get("btc") ?? null,
    from: got.get("from") ?? null, watchS, healthPort, journalDir: got.get("journal-dir") ?? null };
}
