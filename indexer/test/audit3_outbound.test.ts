// The targeted check of round 3 (docs/audit/2026-09-26/review3/R3-check.md M-E), turned into regressions by round 4
// (review3/fixes-R4-services.md §5). Variations on R2 N-1 against pinnedHttpsFetch: each case runs the transport in a child process
// against a local TLS server and asserts the fetch settles (resolves or rejects) and the process LIVES. The audit3 PoC (an HTTP 101
// answer left the fetch pending for ever, past the guard's timeout) is inverted, and a server that never sends headers is added.
import { test } from "node:test";
import assert from "node:assert/strict";
import { execFileSync, spawnSync } from "node:child_process";
import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { WrappedService } from "../src/wrapped/service.js";
import { parseConfig } from "../src/wrapped/config.js";
import { buildWorld, NOW, worldConfig } from "./wrappedWorld.js";

const CASES: Record<string, string> = {
  "100 Continue then 999": `s.end("HTTP/1.1 100 Continue\\r\\n\\r\\nHTTP/1.1 999 X\\r\\ncontent-length: 0\\r\\n\\r\\n")`,
  "200 with a reason phrase undici refuses": `s.end("HTTP/1.1 200 \\u00ff\\u00fe bad\\r\\ncontent-length: 2\\r\\n\\r\\nok")`,
  "200 body far over the reader's cap, streamed fast (cancel mid-flow)": `{ s.write("HTTP/1.1 200 OK\\r\\ncontent-type: application/json\\r\\n\\r\\n"); const b = Buffer.alloc(65536, 97); let n = 0; const go = () => { while (n++ < 2000) { if (!s.write(b)) { s.once("drain", go); return; } } s.end(); }; go(); }`,
  "200 then the socket is reset mid-body": `{ s.write("HTTP/1.1 200 OK\\r\\ncontent-length: 100000\\r\\n\\r\\n" + "a".repeat(1000)); setTimeout(() => s.destroy(), 50); }`,
  "101 Switching Protocols (R3-check M-E)": `s.end("HTTP/1.1 101 Switching Protocols\\r\\nconnection: upgrade\\r\\nupgrade: x\\r\\n\\r\\n")`,
  "101 Switching Protocols, the socket kept open": `s.write("HTTP/1.1 101 Switching Protocols\\r\\nconnection: upgrade\\r\\nupgrade: x\\r\\n\\r\\n")`,
  "headers that never come (the connection held open)": `void 0`,
  "200 whose body stalls after its headers": `s.write("HTTP/1.1 200 OK\\r\\ncontent-length: 100\\r\\n\\r\\nabc")`,
};

/** Runs `reply` on a local TLS server, fetched directly (a 3 s signal) and through guardedFetch (a 3 s deadline), in a child process. */
function run(reply: string) {
  const dir = mkdtempSync(join(tmpdir(), "audit3-"));
  execFileSync("openssl", ["req", "-x509", "-newkey", "rsa:2048", "-nodes", "-keyout", join(dir, "k.pem"), "-out", join(dir, "c.pem"), "-days", "1", "-subj", "/CN=localhost"], { stdio: "ignore" });
  const outbound = resolve(import.meta.dirname, "../src/outbound.ts");
  const script = `
    import tls from "node:tls"; import { readFileSync } from "node:fs";
    import { pinnedHttpsFetch, guardedFetch } from ${JSON.stringify(outbound)};
    process.on("uncaughtException", (e) => { console.log("UNCAUGHT", e?.name, e?.message); process.exit(3); });
    process.on("unhandledRejection", (e) => { console.log("UNHANDLED", e?.name, e?.message); process.exit(4); });
    const srv = tls.createServer({ key: readFileSync(${JSON.stringify(join(dir, "k.pem"))}), cert: readFileSync(${JSON.stringify(join(dir, "c.pem"))}) }, (s) => {
      s.on("error", () => {});
      s.once("data", () => ${reply});
    });
    srv.listen(0, "127.0.0.1", async () => {
      const port = srv.address().port, t0 = Date.now();
      try { const r = await pinnedHttpsFetch("https://127.0.0.1:" + port + "/", { signal: AbortSignal.timeout(3000) }); const t = await r.text().catch((e) => "BODYERR " + e?.message); console.log("DIRECT RESOLVED", r.status, String(t.length)); }
      catch (e) { console.log("DIRECT REJECTED", e?.name, String(e?.message).slice(0, 120)); }
      const t1 = Date.now();
      const f = guardedFetch({ resolve: async () => [{ address: "93.184.216.34", family: 4 }], maxBytes: 1_000_000, timeoutMs: 3000,
        fetch: (u, i) => pinnedHttpsFetch("https://127.0.0.1:" + port + "/", i) });
      try { const r = await f("https://draws.example/list.json"); console.log("GUARD RESOLVED", r.status); }
      catch (e) { console.log("GUARD REJECTED", String(e?.message).slice(0, 120)); }
      console.log("MS", t1 - t0, Date.now() - t1);
      setTimeout(() => { console.log("ALIVE"); process.exit(0); }, 300);
    });`;
  writeFileSync(join(dir, "poc.mts"), script);
  const r = spawnSync(process.execPath, ["--import", "tsx", join(dir, "poc.mts")], { env: { ...process.env, NODE_TLS_REJECT_UNAUTHORIZED: "0" }, encoding: "utf8", timeout: 30_000, cwd: resolve(import.meta.dirname, "..") });
  return { out: `${r.stdout}\n${r.stderr}`, status: r.status, stdout: r.stdout };
}

for (const [name, reply] of Object.entries(CASES)) test(`audit3 N-1/M-E: ${name}: the fetch settles within its deadline, directly and through the guard, and the process lives`, () => {
  const { out, status, stdout } = run(reply);
  console.log(`[${name}] ${stdout.trim().replace(/\n/g, " | ")}`);
  assert.match(out, /DIRECT (RESOLVED|REJECTED)/, out);
  assert.match(out, /GUARD (RESOLVED|REJECTED)/, out);
  const ms = /MS (\d+) (\d+)/.exec(out);
  assert.ok(ms && Number(ms[1]) < 5_000 && Number(ms[2]) < 5_000, `both settle within the 3 s deadline (+ slack): ${ms?.[0]}`);
  assert.match(out, /ALIVE/, out);
  assert.doesNotMatch(out, /UNCAUGHT|UNHANDLED/, out);
  assert.equal(status, 0, out);
  if (/^101/.test(name)) { assert.match(out, /DIRECT REJECTED .*HTTP 101/, "a protocol switch is refused"); assert.match(out, /GUARD REJECTED/); }
  if (/never come|stalls/.test(name)) assert.match(out, /GUARD REJECTED .*no complete answer within 3000 ms/, "the guard's deadline ends it");
});

test("M-E: a collection's snapshot whose build never settles no longer hangs every caller: each waits at most inflightMs, and a later caller starts a new build", async () => {
  const w = buildWorld();
  const svc = new WrappedService({ config: parseConfig(worldConfig()), rpc: w.rpc, fetch: w.fetch, now: () => NOW, inflightMs: 200 });
  let builds = 0;
  (svc as unknown as { build: () => Promise<never> }).build = () => { builds++; return new Promise<never>(() => undefined); };
  const slug = parseConfig(worldConfig()).collections[0].slug;
  const t0 = Date.now();
  await assert.rejects(svc.collection(slug), /took longer than/);
  await assert.rejects(svc.ticket(slug, 1), /took longer than/, "a caller arriving meanwhile shares the build and its bound");
  assert.ok(Date.now() - t0 < 1_000);
  assert.ok(builds >= 1 && builds <= 2);
  const before = builds;
  await assert.rejects(svc.collection(slug), /took longer than/);
  assert.equal(builds, before + 1, "the hung build is left behind and a new one started");
});
