import assert from "node:assert/strict";
import { test } from "node:test";
import { existsSync, readFileSync, readdirSync } from "node:fs";

// docs/v6/p1-services.md §0: who in ops/ may read the operator's identity, which canister calls each service makes, and what the images carry.
const src = (f: string) => readFileSync(new URL(`../src/${f}`, import.meta.url), "utf8");
const code = (s: string) => s.replace(/\/\*[\s\S]*?\*\//g, "").replace(/(^|[^:])\/\/.*$/gm, "$1");
const files = readdirSync(new URL("../src/", import.meta.url)).filter((f) => f.endsWith(".ts"));

test("OPERATOR_IDENTITY_PEM is read in src/identity.ts and nowhere else; that file prints nothing and exports no key", () => {
  const readers = files.filter((f) => /OPERATOR_IDENTITY_PEM|\bVAR\]/.test(code(src(f)).replace(/"[^"\n]*"|`[^`\n]*`/g, '""')));
  assert.deepEqual(readers, ["identity.ts"]);
  assert.ok(!/console\.|process\.std(out|err)|\blog\(/.test(code(src("identity.ts"))));
  assert.ok(!/export (const|let) .*(pem|key)/i.test(src("identity.ts")));
});

test("the reconciler holds no identity: reconcile.ts and what it imports never touch src/identity.ts, and it calls one query", () => {
  for (const f of ["reconcile.ts", "stage.ts", "ic.ts", "log.ts"]) assert.ok(!/from "\.\/identity\.js"/.test(src(f)), `${f} imports the identity`);
  const r = code(src("reconcile.ts"));
  assert.ok(/custody_caps\(\)/.test(r));
  for (const m of ["tick(", "frog_offer(", "float_address(", "custody_address("]) assert.ok(!r.includes(m), `the reconciler calls ${m}`);
  assert.ok(!/eth_sendRawTransaction|eth_sendTransaction|privateKeyToAccount/.test(r));
});

test("the runner calls tick and paused; the offer tool calls frog_offer only after the dry run's return, and only with --send", () => {
  const run = code(src("runner.ts"));
  const calls = [...run.matchAll(/\bb\.(\w+)\(/g)].map((m) => m[1]);
  // eth_nft_config: a query, read once at the start (audit 2026-09-27-nft D-L-2: the Ethereum step starts fully configured or not at all)
  // eth_nft_state: a query, read after each cycle for the pending 4663 transaction (review round 6, F6-L-1: the runner ticks sooner then)
  // eth_nft_collections: a query, read once at the start (docs/v7/dormant-collections.md: a dormant collection's evidence is never stale)
  // eth_nft_state_of: a query, each pass, the other chains' sides (2026-10-12, Robinhood Chain NFTs: the 4663 side's buys and readings)
  // frog_collections: a query, each pass, which Inscriptions collections are live (Ordinals multi-collection: the buying steps serve those)
  assert.deepEqual([...new Set(calls)].sort(), ["eth_nft_collections", "eth_nft_config", "eth_nft_state", "eth_nft_state_of", "frog_collections", "paused", "tick"]);
  const off = code(src("offer.ts"));
  const refused = off.indexOf("if (!report.ok)"), dry = off.indexOf("if (!send)"), call = off.indexOf("b.frog_offer(");
  assert.ok(refused > 0 && dry > refused && call > dry, "refused → dry run → the call, in that order");
  assert.equal((off.match(/\.frog_offer\(/g) ?? []).length, 1);
});

test("the images arm nothing: no key, identity or token in a Dockerfile or a railway.json; the reconciler's image is built without the identity file's secret", () => {
  for (const f of ["Dockerfile.runner", "Dockerfile.reconciler", "Dockerfile.fueler", "railway.runner.json", "railway.reconciler.json", "railway.fueler.json"]) {
    const p = new URL(`../${f}`, import.meta.url);
    if (!existsSync(p)) continue;
    const t = readFileSync(p, "utf8").split("\n").filter((l) => !l.trimStart().startsWith("#")).join("\n");   // instructions, not comments
    assert.ok(!/OPERATOR_IDENTITY_PEM|ALERT_TELEGRAM_BOT_TOKEN|BEGIN .*PRIVATE KEY|KEEPER_PRIVATE_KEY|FUELER_EVM_KEY|FUELER_IC_IDENTITY_PEM|FUELER_SOL_KEYPAIR|RELAY_API_KEY/.test(t), `${f} carries a secret`);
  }
  // the reconciler's image copies neither the runner nor the identity loader; the runner's copies neither the offer tool nor the reconciler
  const rec = readFileSync(new URL("../Dockerfile.reconciler", import.meta.url), "utf8"), run = readFileSync(new URL("../Dockerfile.runner", import.meta.url), "utf8");
  const copied = (t: string) => t.split("\n").filter((l) => l.startsWith("COPY ops/src")).join(" ");
  assert.ok(!/identity\.ts|runner\.ts|offer\.ts/.test(copied(rec)), copied(rec));
  assert.ok(/runner\.ts/.test(copied(run)) && !/reconcile\.ts|offer\.ts|judge\.ts/.test(copied(run)), copied(run));
  // the fueler's image carries its own keys' loader and nothing of the runner's, the offer tool's or the reconciler's
  const fuel = readFileSync(new URL("../Dockerfile.fueler", import.meta.url), "utf8");
  assert.ok(/fueler\.ts/.test(copied(fuel)) && !/runner\.ts|reconcile\.ts|offer\.ts|judge\.ts/.test(copied(fuel)), copied(fuel));
  // and the build contexts agree: the reconciler's never even sends the identity loader to the builder
  const ignore = (f: string) => readFileSync(new URL(`../${f}`, import.meta.url), "utf8");
  assert.ok(!/identity\.ts|runner\.ts|offer\.ts/.test(ignore("Dockerfile.reconciler.dockerignore")));
  assert.ok(!/reconcile\.ts|offer\.ts|judge\.ts/.test(ignore("Dockerfile.runner.dockerignore")));
});

// go-live prep (2026-09-28): the runner's image copied runner.ts but not relay.ts / ethnft.ts, which it imports (ERR_MODULE_NOT_FOUND at
// start), and the reconciler's missed ethRecords.ts. Every file an image's entry point loads (its value imports, followed through the
// files they load; `import type` is erased by tsx) must be copied by the Dockerfile AND let through its .dockerignore.
test("each image carries every file its entry point loads (value imports, transitively), in the Dockerfile and its .dockerignore", () => {
  const root = new URL("../../", import.meta.url);
  const loads = (entry: string): string[] => {
    const seen = new Set<string>(), todo = [entry];
    while (todo.length) {
      const f = todo.pop()!;
      if (seen.has(f)) continue;
      seen.add(f);
      const text = readFileSync(new URL(f, root), "utf8");
      for (const m of text.matchAll(/^import\s+(?!type\s)[^;]*?from\s+"(\.{1,2}\/[^"]+)"/gms)) {
        const rel = new URL(m[1], new URL(f, root)).pathname.slice(root.pathname.length);
        const ts = rel.replace(/\.js$/, ".ts");
        todo.push(existsSync(new URL(ts, root)) ? ts : rel);
      }
    }
    return [...seen];
  };
  for (const [image, entry] of [["Dockerfile.runner", "ops/src/runner.ts"], ["Dockerfile.reconciler", "ops/src/reconcile.ts"], ["Dockerfile.fueler", "ops/src/fueler.ts"]]) {
    const docker = readFileSync(new URL(`../${image}`, import.meta.url), "utf8");
    const copied = docker.split("\n").filter((l) => /^COPY\s/.test(l) && !l.includes("--from"))
      .flatMap((l) => l.replace(/^COPY\s+(--\S+\s+)*/, "").trim().split(/\s+/).slice(0, -1));
    const allowed = readFileSync(new URL(`../${image}.dockerignore`, import.meta.url), "utf8").split("\n")
      .filter((l) => l.startsWith("!")).map((l) => l.slice(1).trim());
    const under = (f: string, list: string[]) => list.some((c) => f === c || f.startsWith(c.replace(/\/?$/, "/")));
    const files = loads(entry);
    assert.ok(files.length >= 4, `${entry}: ${files.join(", ")}`);
    const missing = files.filter((f) => !under(f, copied)), ignored = files.filter((f) => !under(f, allowed));
    assert.deepEqual(missing, [], `${image} does not COPY what ${entry} loads`);
    assert.deepEqual(ignored, [], `${image}.dockerignore keeps out what ${entry} loads`);
  }
});

test("go-live prep: the runner's keyed ETH_LOGS_RPC is scrubbed from every log line (the whole URL, and its key alone)", async () => {
  const { envSecrets, logger, urlSecrets } = await import("../src/log.js");
  const key = "0123456789abcdef0123456789abcdef";
  const env = { ETH_LOGS_RPC: `https://api-ethereum-mainnet.n.dwellir.com/${key}` };
  assert.ok(urlSecrets(env.ETH_LOGS_RPC).includes(key));
  assert.deepEqual(urlSecrets("https://xrpc.cl/eth"), ["https://xrpc.cl/eth"], "a keyless URL: only itself (no short segment is a secret)");
  const lines: string[] = [];
  const log = logger((l) => lines.push(l), () => envSecrets(env));
  log("warn", "eth-logs", { url: env.ETH_LOGS_RPC, note: `fetch failed for /${key}?x=1` });
  assert.ok(!lines[0].includes(key), lines[0]);
});
