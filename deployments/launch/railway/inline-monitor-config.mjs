#!/usr/bin/env node
// The monitor's MONITOR_CONFIG_JSON for Railway: the filled launch configuration with its three files inlined.
//   node deployments/launch/railway/inline-monitor-config.mjs deployments/launch/monitor-nft.json > /tmp/monitor-config.json
// Why: the monitor image carries no deployments/ tree, and with MONITOR_CONFIG_JSON the loader resolves builds_file,
// launch_file and evm.stack_file against the process's cwd, not the file's folder (monitor/src/config.mjs:19-38). It skips a
// *_file whose data is already present, so this writes `builds`, `launch` and `evm.stack` in and drops the *_file keys.
// ${ENV} placeholders (DWELLIR_API_KEY) are left as they are: the monitor expands them from its own environment at load.
// It then checks the result with the monitor's own loader (a stand-in value for each ${ENV}, in this process only) and prints
// the verdict on stderr. Read-only; no network; nothing secret is read or printed.
import { existsSync, readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";

const path = process.argv[2];
if (!path) { console.error("usage: inline-monitor-config.mjs <filled monitor-nft.json>"); process.exit(2); }
const base = dirname(resolve(path));
const cfg = JSON.parse(readFileSync(path, "utf8"));
const read = (rel, what) => {
  const f = resolve(base, rel);
  if (!existsSync(f)) { console.error(`refused: ${what} ${f} does not exist (a deploy output: write it first)`); process.exit(1); }
  return JSON.parse(readFileSync(f, "utf8"));
};
if (cfg.builds_file && !cfg.builds) { cfg.builds = read(cfg.builds_file, "builds_file").builds ?? []; delete cfg.builds_file; }
if (cfg.launch_file && !cfg.launch) { cfg.launch = read(cfg.launch_file, "launch_file"); delete cfg.launch_file; }
if (cfg.evm?.stack_file && !cfg.evm.stack) { cfg.evm.stack = read(cfg.evm.stack_file, "evm.stack_file"); delete cfg.evm.stack_file; }
const out = JSON.stringify(cfg);
// placeholders outside the "//…" comment keys (a comment may quote one)
const bare = JSON.stringify(cfg, (k, v) => (k.startsWith("//") ? undefined : v));
const placeholders = bare.match(/<[^<>"]{2,80}>/g) ?? [];
const { loadConfig } = await import(new URL("../../../monitor/src/config.mjs", import.meta.url));
for (const m of out.matchAll(/\$\{([A-Z0-9_]+)\}/g)) process.env[m[1]] ||= "stand-in-for-the-check";
try {
  loadConfig({ json: out });
  console.error(placeholders.length ? `note: the monitor's loader accepts it, but ${placeholders.length} <placeholder>(s) remain (${[...new Set(placeholders)].slice(0, 5).join(", ")}): golive-check --monitor-config refuses those` : "ok: the monitor's loader accepts it and no <placeholder> remains");
} catch (err) { console.error(`the monitor's loader refuses it:\n${err.message}`); process.exitCode = 1; }
process.stdout.write(out + "\n");
