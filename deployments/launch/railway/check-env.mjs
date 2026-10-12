#!/usr/bin/env node
// Checks each <service>.env.example here against the variables the service's code reads, so a template cannot silently miss one.
//   node deployments/launch/railway/check-env.mjs        (from the repo root or anywhere; read-only, no network)
// A variable counts as read when the code has env.X, env["X"], process.env.X, or passes "X" (an all-caps name with the service's
// prefix) to its env helpers (keeper's num()/big()/httpsUrl(), the reconciler's n()); a monitor config's ${X} counts for the monitor.
// A template lists a variable as `X=` or `# X=`. Exit 1 when the code reads one the template does not name.
import { readFileSync, readdirSync, statSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const root = resolve(here, "../../..");
const files = (p) => statSync(join(root, p)).isDirectory()
  ? readdirSync(join(root, p), { recursive: true }).map(String).filter((f) => /\.(ts|mjs|js)$/.test(f) && !f.includes("node_modules")).map((f) => join(p, f))
  : [p];
const SERVICES = {
  runner: { src: ["ops/src/runner.ts", "ops/src/ic.ts", "ops/src/identity.ts", "ops/src/log.ts", "ops/src/relay.ts", "ops/src/ethnft.ts", "ops/src/inscriptions.ts", "ops/src/solfloor.ts"], prefix: /^(RUNNER|RELAY|ETH)_/ },
  keeper: { src: ["keeper/service", "keeper/src", "tools/snapshot/src"], prefix: /^KEEPER_/, drop: /^RESOLVER_/ },
  resolver: { src: ["keeper/service/config.ts", "keeper/service/resolver-main.ts", "keeper/service/signer.ts"], prefix: /^(RESOLVER)_/, only: /^(KEEPER_(CHAIN_ID|STACK|RPC_URLS|DRAND_URLS)|RESOLVER_.*)$/ },
  monitor: { src: ["monitor/src"], prefix: /^MONITOR_/, config: "deployments/launch/monitor-nft.json" },
  reconciler: { src: ["ops/src/reconcile.ts", "ops/src/ic.ts", "ops/src/log.ts", "monitor/src/alerts.mjs"], prefix: /^RECONCILE_/ },
  indexer: { src: ["indexer/src"], prefix: /^$/, drop: /^$/ },
  fueler: { src: ["ops/src/fueler.ts", "ops/src/fuel", "ops/src/ic.ts", "ops/src/log.ts", "ops/src/relay.ts"], prefix: /^(FUELER|RELAY)_/ },
};
const IGNORE = new Set(["NODE_ENV", "HOME", "PATH"]);
let bad = 0;
for (const [svc, s] of Object.entries(SERVICES)) {
  const read = new Set();
  for (const f of s.src.flatMap(files)) {
    if (/\/test\/|write-.*fixtures/.test(f)) continue;
    const t = readFileSync(join(root, f), "utf8");
    for (const m of t.matchAll(/env(?:\.|\[\s*["'])([A-Z][A-Z0-9_]+)/g)) read.add(m[1]);
    for (const m of t.matchAll(/["']([A-Z][A-Z0-9_]{3,})["']/g)) if (s.prefix.test(m[1]) || /_(PRIVATE_KEY|IDENTITY_PEM)$/.test(m[1])) read.add(m[1]);
  }
  if (s.config) for (const m of readFileSync(join(root, s.config), "utf8").matchAll(/\$\{([A-Z0-9_]+)\}/g)) read.add(m[1]);
  for (const v of [...read]) if (IGNORE.has(v) || (s.drop && s.drop.source !== "^$" && s.drop.test(v)) || (s.only && !s.only.test(v) && !/^KEEPER_(MAX_|RESOLVER_MAX|RECEIPT|STUCK|PASS|COINS|ALERT|STATUS_TOKEN)/.test(v))) read.delete(v);
  if (svc === "resolver") { read.delete("KEEPER_PRIVATE_KEY"); read.delete("KEEPER_MODE"); }
  const listed = new Set([...readFileSync(join(here, `${svc}.env.example`), "utf8").matchAll(/^#?\s*([A-Z][A-Z0-9_]+)=/gm)].map((m) => m[1]));
  const missing = [...read].filter((v) => !listed.has(v)).sort();
  console.log(`${missing.length ? "FAIL" : "ok  "} ${svc}.env.example: ${read.size} variable(s) read by the code${missing.length ? `; not in the template: ${missing.join(", ")}` : ", all listed"}`);
  if (missing.length) bad++;
}
process.exit(bad ? 1 : 0);
