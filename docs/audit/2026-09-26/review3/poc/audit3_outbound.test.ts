// Audit round 3 (R3 check): variations on R2 N-1 against the fixed pinnedHttpsFetch. Each case runs the transport in a child
// process against a local TLS server and asserts the fetch settles (resolves or rejects) and the process LIVES. The last test is a
// PoC of a remaining gap (a pass means the gap is there): an HTTP 101 answer leaves the fetch pending for ever, past the guard's timeout.
import { test } from "node:test";
import assert from "node:assert/strict";
import { execFileSync, spawnSync } from "node:child_process";
import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";

const CASES: Record<string, string> = {
  "100 Continue then 999": `s.end("HTTP/1.1 100 Continue\\r\\n\\r\\nHTTP/1.1 999 X\\r\\ncontent-length: 0\\r\\n\\r\\n")`,
  "200 with a reason phrase undici refuses": `s.end("HTTP/1.1 200 \\u00ff\\u00fe bad\\r\\ncontent-length: 2\\r\\n\\r\\nok")`,
  "200 body far over the reader's cap, streamed fast (cancel mid-flow)": `{ s.write("HTTP/1.1 200 OK\\r\\ncontent-type: application/json\\r\\n\\r\\n"); const b = Buffer.alloc(65536, 97); let n = 0; const go = () => { while (n++ < 2000) { if (!s.write(b)) { s.once("drain", go); return; } } s.end(); }; go(); }`,
  "200 then the socket is reset mid-body": `{ s.write("HTTP/1.1 200 OK\\r\\ncontent-length: 100000\\r\\n\\r\\n" + "a".repeat(1000)); setTimeout(() => s.destroy(), 50); }`,
};

for (const [name, reply] of Object.entries(CASES)) test(`audit3 N-1 variation: ${name}: the fetch settles and the process lives`, () => {
  const dir = mkdtempSync(join(tmpdir(), "audit3-"));
  execFileSync("openssl", ["req", "-x509", "-newkey", "rsa:2048", "-nodes", "-keyout", join(dir, "k.pem"), "-out", join(dir, "c.pem"), "-days", "1", "-subj", "/CN=localhost"], { stdio: "ignore" });
  const outbound = resolve(import.meta.dirname, "../src/outbound.ts");
  const script = `
    import tls from "node:tls"; import { readFileSync } from "node:fs";
    import { pinnedHttpsFetch, guardedFetch } from ${JSON.stringify(outbound)};
    process.on("uncaughtException", (e) => { console.log("UNCAUGHT", e?.name, e?.message); process.exit(3); });
    const srv = tls.createServer({ key: readFileSync(${JSON.stringify(join(dir, "k.pem"))}), cert: readFileSync(${JSON.stringify(join(dir, "c.pem"))}) }, (s) => {
      s.on("error", () => {});
      s.once("data", () => ${reply});
    });
    srv.listen(0, "127.0.0.1", async () => {
      const port = srv.address().port;
      // through the guard (as the indexer calls it), with the loopback allowed by a fake resolver and the real transport
      const f = guardedFetch({ resolve: async () => [{ address: "93.184.216.34", family: 4 }], maxBytes: 1_000_000, timeoutMs: 8000,
        fetch: (u, i) => pinnedHttpsFetch(String(u).replace("x.example", "127.0.0.1:" + port), i) });
      try { const r = await pinnedHttpsFetch("https://127.0.0.1:" + port + "/", { signal: AbortSignal.timeout(8000) }); const t = await r.text().catch((e) => "BODYERR " + e?.message); console.log("RESOLVED", r.status, String(t.length)); }
      catch (e) { console.log("REJECTED", e?.name, String(e?.message).slice(0, 120)); }
      setTimeout(() => { console.log("ALIVE"); srv.close(); process.exit(0); }, 500);
    });`;
  writeFileSync(join(dir, "poc.mts"), script);
  const r = spawnSync(process.execPath, ["--import", "tsx", join(dir, "poc.mts")], { env: { ...process.env, NODE_TLS_REJECT_UNAUTHORIZED: "0" }, encoding: "utf8", timeout: 30_000, cwd: resolve(import.meta.dirname, "..") });
  const out = `${r.stdout}\n${r.stderr}`;
  console.log(`[${name}] ${r.stdout.trim().replace(/\n/g, " | ")}`);
  assert.match(out, /RESOLVED|REJECTED/, out);
  assert.match(out, /ALIVE/, out);
  assert.doesNotMatch(out, /UNCAUGHT/, out);
  assert.equal(r.status, 0, out);
});

test("audit3 M-E (gap): a TLS server answering HTTP/1.1 101 Switching Protocols leaves pinnedHttpsFetch pending for ever: guardedFetch's 3 s timeout does not end it", () => {
  const dir = mkdtempSync(join(tmpdir(), "audit3-"));
  execFileSync("openssl", ["req", "-x509", "-newkey", "rsa:2048", "-nodes", "-keyout", join(dir, "k.pem"), "-out", join(dir, "c.pem"), "-days", "1", "-subj", "/CN=localhost"], { stdio: "ignore" });
  const outbound = resolve(import.meta.dirname, "../src/outbound.ts");
  const script = `
    import tls from "node:tls"; import { readFileSync } from "node:fs";
    import { pinnedHttpsFetch, guardedFetch } from ${JSON.stringify(outbound)};
    const srv = tls.createServer({ key: readFileSync(${JSON.stringify(join(dir, "k.pem"))}), cert: readFileSync(${JSON.stringify(join(dir, "c.pem"))}) }, (s) => {
      s.on("error", () => {}); s.once("data", () => s.end("HTTP/1.1 101 Switching Protocols\\r\\nconnection: upgrade\\r\\nupgrade: x\\r\\n\\r\\n"));
    });
    srv.listen(0, "127.0.0.1", async () => {
      const port = srv.address().port, t0 = Date.now();
      const f = guardedFetch({ resolve: async () => [{ address: "93.184.216.34", family: 4 }], timeoutMs: 3000, fetch: (u, i) => pinnedHttpsFetch("https://127.0.0.1:" + port + "/", i) });
      setTimeout(() => { console.log("STILL PENDING after " + (Date.now() - t0) + " ms"); process.exit(0); }, 10000);
      try { const r = await f("https://draws.example/list.json"); console.log("RESOLVED", r.status); } catch (e) { console.log("REJECTED", e?.message); }
    });`;
  writeFileSync(join(dir, "poc.mts"), script);
  const r = spawnSync(process.execPath, ["--import", "tsx", join(dir, "poc.mts")], { env: { ...process.env, NODE_TLS_REJECT_UNAUTHORIZED: "0" }, encoding: "utf8", timeout: 30_000, cwd: resolve(import.meta.dirname, "..") });
  const out = `${r.stdout}\n${r.stderr}`;
  console.log(r.stdout.trim());
  assert.match(out, /STILL PENDING after 1\d{4} ms/, out);
  assert.doesNotMatch(out, /RESOLVED|REJECTED/, out);
});
