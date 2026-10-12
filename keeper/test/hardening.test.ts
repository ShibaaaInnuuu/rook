import assert from "node:assert/strict";
import { appendFileSync, mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import type { AddressInfo } from "node:net";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { delayFor, GaveUp, withBackoff } from "../src/backoff.js";
import { ConfigError, parseConfig } from "../src/config.js";
import { healthy, newHealth, serveHealth } from "../src/health.js";
import { Journal, repair } from "../src/journal.js";
import { logger } from "../src/log.js";

const REG = "0x" + "11".repeat(20), BTC = "bc1p0xlxvlhemja6c4dqv22uapctqupfhlxm9h8z3k2e72q4k9hcz7vqzk5jj0";
const problems = (argv: string[], env: Record<string, string> = {}): string[] => { try { parseConfig(argv, env); return []; } catch (e) { return (e as ConfigError).problems; } };

test("docs/phase4.md §5 — dry-run is the default and the only mode, and the smallest valid configuration is one address", () => {
  assert.deepEqual(parseConfig(["--registry", REG]), { mode: "dry-run", registry: REG, rpc: "http://127.0.0.1:8545", source: "fixture", prices: null, quoter: "fixture", btc: null,
    from: null, watchS: 0, healthPort: null, journalDir: null });
  for (const m of ["live", "send", "production", "local"]) assert.match(problems(["--registry", REG, "--mode", m])[0], /one mode, dry-run/);
  assert.equal(parseConfig(["--registry", REG, "--mode", "dry-run"]).mode, "dry-run");
});

test("it refuses to start on contradictions and unknowns, and says all of them at once", () => {
  assert.deepEqual(problems([]), ["missing --registry"]);
  assert.match(problems(["--registry", "0x12"])[0], /not an address/);
  assert.match(problems(["--registry", REG, "--sign", "yes"])[0], /unknown flag --sign/);
  assert.match(problems(["--registry", REG, "stray"])[0], /is not a flag/);
  assert.match(problems(["--registry", REG, "--watch"])[0], /needs a value/);
  assert.match(problems(["--registry", REG, "--registry", REG])[0], /given twice/);
  assert.match(problems(["--registry", REG, "--watch", "5"])[0], /under 15 s/);
  assert.match(problems(["--registry", REG, "--watch", "1.5"])[0], /whole number/);
  assert.match(problems(["--registry", REG, "--rpc", "ftp://x"])[0], /http\(s\)/);
  assert.match(problems(["--registry", REG, "--quoter", "lifi"]).join("|"), /needs --btc.*needs --from/);
  assert.match(problems(["--registry", REG, "--btc", BTC])[0], /belong to --quoter lifi/);
  assert.match(problems(["--registry", REG, "--quoter", "lifi", "--btc", "1BoatSLRHtKNngkdXEeobR76b53LETtpyT", "--from", REG])[0], /bech32/);
  assert.match(problems(["--registry", REG, "--source", "magiceden", "--prices", "p.json"])[0], /--prices is the fixture source's file/);
  assert.match(problems(["--registry", REG, "--source", "unisat"])[0], /fixture or magiceden/);
  assert.match(problems(["--registry", REG, "--health-port", "8790"])[0], /without --watch/);
  assert.match(problems(["--registry", REG, "--watch", "60", "--health-port", "99999"])[0], /not a port/);
  assert.equal(problems(["--mode", "live", "--watch", "3", "--nope", "1"]).length, 4, "every problem, not just the first");
  assert.equal(parseConfig(["--registry", REG, "--quoter", "lifi", "--btc", BTC, "--from", REG, "--watch", "60", "--health-port", "8790"]).healthPort, 8790);
});

test("KEEPER_MODE=local while pointed at anything that is not this machine is a contradiction, and stops it", () => {
  assert.match(problems(["--registry", REG, "--rpc", "https://rpc.mainnet.chain.robinhood.com"], { KEEPER_MODE: "local" })[0], /local means this machine/);
  assert.match(problems(["--registry", REG], { KEEPER_MODE: "live" })[0], /means nothing/);
  assert.deepEqual(problems(["--registry", REG], { KEEPER_MODE: "local" }), []);
  assert.deepEqual(problems(["--registry", REG, "--rpc", "http://localhost:8545"], { KEEPER_MODE: "local" }), []);
  assert.deepEqual(problems(["--registry", REG, "--rpc", "https://rpc.mainnet.chain.robinhood.com"]), [], "reading a public chain in dry-run is the point");
});

test("the journal: whole lines, flushed; a torn final line from a crash is removed on the next start and nothing good is lost", () => {
  const dir = mkdtempSync(join(tmpdir(), "etch-journal-")), path = join(dir, "2026-09-21.jsonl");
  const j = new Journal(() => path, () => new Date("2026-09-21T10:00:00Z"));
  j.append({ n: 1, wei: 5n * 10n ** 16n }); j.append({ n: 2 });
  assert.deepEqual(readFileSync(path, "utf8").split("\n"), ['{"n":1,"wei":"50000000000000000"}', '{"n":2}', ""]);
  appendFileSync(path, '{"n":3,"actions":[{"type":"decl');                       // the process died mid-write
  assert.equal(repair(path).removedBytes, 31);
  assert.deepEqual(readFileSync(path, "utf8").trimEnd().split("\n").map((l) => JSON.parse(l).n), [1, 2]);
  assert.equal(repair(path).removedBytes, 0, "a clean file is left alone");
  appendFileSync(path, "torn again");
  new Journal(() => path).append({ n: 4 });                                        // a new process repairs before its first write
  assert.deepEqual(readFileSync(path, "utf8").trimEnd().split("\n").map((l) => JSON.parse(l).n), [1, 2, 4]);
  writeFileSync(join(dir, "only-torn.jsonl"), "{half");
  assert.equal(repair(join(dir, "only-torn.jsonl")).removedBytes, 5);
  assert.equal(repair(join(dir, "missing.jsonl")).removedBytes, 0);
});

test("backoff: 1, 2, 4, 8 … capped at 60 s; it gives the pass up after its tries; and what another try cannot fix is thrown at once", async () => {
  assert.deepEqual([0, 1, 2, 3, 6, 10].map((a) => delayFor(a)), [1_000, 2_000, 4_000, 8_000, 60_000, 60_000]);
  const slept: number[] = []; let calls = 0;
  await assert.rejects(withBackoff(async () => { calls++; throw new Error("eth_call: HTTP 503"); }, { sleep: async (ms) => { slept.push(ms); } }),
    (e) => e instanceof GaveUp && e.attempts === 5 && /503/.test(e.last));
  assert.deepEqual([calls, slept], [5, [1_000, 2_000, 4_000, 8_000]]);
  calls = 0;
  assert.equal(await withBackoff(async () => { if (++calls < 3) throw new Error("timeout"); return "ok"; }, { sleep: async () => {} }), "ok");
  calls = 0;
  await assert.rejects(withBackoff(async () => { calls++; throw new Error("refused: eth_sendRawTransaction is not a read"); }, { sleep: async () => {} }), /refused/);
  assert.equal(calls, 1, "a refusal is never retried");
});

test("health: 200 while passes keep completing, 503 after a run of failures or a silence of three intervals; GET /healthz only", async () => {
  let now = new Date("2026-09-21T10:00:00Z");
  const h = newHealth(() => now);
  assert.equal(healthy(h, 60, () => now), false, "no pass yet");
  h.lastPassAt = now.toISOString(); h.passes = 1;
  assert.equal(healthy(h, 60, () => now), true);
  now = new Date(now.getTime() + 181_000); assert.equal(healthy(h, 60, () => now), false, "silent for more than three intervals");
  h.lastPassAt = now.toISOString(); h.consecutiveFailures = 3; assert.equal(healthy(h, 60, () => now), false);
  const live = newHealth(); live.lastPassAt = new Date().toISOString();
  const server = serveHealth(() => live, 60, 0, "127.0.0.1");
  await new Promise((r) => server.once("listening", r));
  const base = `http://127.0.0.1:${(server.address() as AddressInfo).port}`;
  try {
    const ok = await fetch(base + "/healthz");
    assert.deepEqual([ok.status, ((await ok.json()) as { ok: boolean; mode: string }).mode, ok.headers.get("cache-control")], [200, "dry-run", "no-store"]);
    live.consecutiveFailures = 5;
    assert.equal((await fetch(base + "/healthz")).status, 503);
    assert.deepEqual([(await fetch(base + "/")).status, (await fetch(base + "/healthz", { method: "POST" })).status], [404, 405]);
  } finally { server.close(); }
});

test("logs are one JSON object a line, bigints are strings, and a URL is cut down to its host so that a keyed RPC URL never reaches a log", () => {
  const lines: string[] = [];
  logger((l) => lines.push(l), () => new Date("2026-09-21T10:00:00Z"))("info", "start", { rpc: "https://lb.example.io/v1/abcdef0123456789", wei: 7n, n: 1 });
  assert.deepEqual(JSON.parse(lines[0]), { at: "2026-09-21T10:00:00.000Z", level: "info", event: "start", rpc: "lb.example.io", wei: "7", n: 1 });
  assert.ok(!lines[0].includes("abcdef0123456789"));
});
