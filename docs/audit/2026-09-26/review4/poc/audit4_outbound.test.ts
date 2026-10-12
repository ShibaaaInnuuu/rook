// Audit round 4 (R4 check): R3's M-E PoC (review3/poc/audit3_outbound.test.ts) re-run against the round-4 transport, plus hostile
// server shapes around the new deadline race. Each case runs in a child process against a local TLS server and asserts the fetch
// settles within its deadline, the socket is closed server-side, and the process lives. No network beyond 127.0.0.1.
import { test } from "node:test";
import assert from "node:assert/strict";
import { execFileSync, spawnSync } from "node:child_process";
import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";

const CASES: Record<string, { reply: string; via: "guard" | "raw" }> = {
  "R3 M-E re-run: 101 Switching Protocols, through guardedFetch": { via: "guard", reply: `s.end("HTTP/1.1 101 Switching Protocols\\r\\nconnection: upgrade\\r\\nupgrade: x\\r\\n\\r\\n")` },
  "101 held open (no end), raw transport, no signal": { via: "raw", reply: `s.write("HTTP/1.1 101 Switching Protocols\\r\\nconnection: upgrade\\r\\nupgrade: x\\r\\n\\r\\n")` },
  "headers never sent, through guardedFetch": { via: "guard", reply: `undefined` },
  "headers, then a body that trickles 1 byte a second, through guardedFetch": { via: "guard", reply: `{ s.write("HTTP/1.1 200 OK\\r\\ncontent-length: 100000\\r\\n\\r\\n"); const t = setInterval(() => { try { s.write("a"); } catch { clearInterval(t); } }, 1000); s.on("close", () => clearInterval(t)); }` },
  "a redirect loop to itself that answers slowly, through guardedFetch": { via: "guard", reply: `setTimeout(() => s.end("HTTP/1.1 302 Found\\r\\nlocation: https://x.example/again\\r\\ncontent-length: 0\\r\\n\\r\\n"), 1200)` },
  "102 Processing forever (1xx interim answers, no final one), through guardedFetch": { via: "guard", reply: `{ const t = setInterval(() => { try { s.write("HTTP/1.1 102 Processing\\r\\n\\r\\n"); } catch { clearInterval(t); } }, 300); s.on("close", () => clearInterval(t)); }` },
};

for (const [name, { reply, via }] of Object.entries(CASES)) test(`audit4 M-E: ${name}: settles by the deadline, the socket is closed, the process lives`, () => {
  const dir = mkdtempSync(join(tmpdir(), "audit4-"));
  execFileSync("openssl", ["req", "-x509", "-newkey", "rsa:2048", "-nodes", "-keyout", join(dir, "k.pem"), "-out", join(dir, "c.pem"), "-days", "1", "-subj", "/CN=localhost"], { stdio: "ignore" });
  const outbound = resolve(import.meta.dirname, "../src/outbound.ts");
  const script = `
    import tls from "node:tls"; import { readFileSync } from "node:fs";
    import { pinnedHttpsFetch, guardedFetch } from ${JSON.stringify(outbound)};
    process.on("uncaughtException", (e) => { console.log("UNCAUGHT", e?.name, e?.message); process.exit(3); });
    let open = 0, closedAt = 0;
    const srv = tls.createServer({ key: readFileSync(${JSON.stringify(join(dir, "k.pem"))}), cert: readFileSync(${JSON.stringify(join(dir, "c.pem"))}) }, (s) => {
      open++; s.on("close", () => { open--; closedAt = Date.now(); }); s.on("error", () => {});
      s.once("data", () => ${reply});
    });
    srv.listen(0, "127.0.0.1", async () => {
      const port = srv.address().port, t0 = Date.now();
      const f = ${via === "guard"
        ? `guardedFetch({ resolve: async () => [{ address: "93.184.216.34", family: 4 }], timeoutMs: 3000, maxBytes: 1_000_000, fetch: (u, i) => pinnedHttpsFetch("https://127.0.0.1:" + port + new URL(String(u)).pathname, i) })`
        : `(u) => pinnedHttpsFetch("https://127.0.0.1:" + port + "/")`};
      setTimeout(() => { console.log("STILL PENDING after " + (Date.now() - t0) + " ms"); process.exit(0); }, 12000);
      try { const r = await f("https://x.example/list.json"); const t = await r.text().catch((e) => "BODYERR " + e?.message); console.log("RESOLVED", r.status, String(t).slice(0, 60), "at", Date.now() - t0); }
      catch (e) { console.log("REJECTED at", Date.now() - t0, "ms:", String(e?.message).slice(0, 120)); }
      setTimeout(() => { console.log("OPEN SOCKETS", open, "closed at", closedAt ? closedAt - t0 : -1); console.log("ALIVE"); srv.close(); process.exit(0); }, 800);
    });`;
  writeFileSync(join(dir, "poc.mts"), script);
  const r = spawnSync(process.execPath, ["--import", "tsx", join(dir, "poc.mts")], { env: { ...process.env, NODE_TLS_REJECT_UNAUTHORIZED: "0" }, encoding: "utf8", timeout: 40_000, cwd: resolve(import.meta.dirname, "..") });
  const out = `${r.stdout}\n${r.stderr}`;
  console.log(`[${name}] ${r.stdout.trim().replace(/\n/g, " | ")}`);
  assert.doesNotMatch(out, /STILL PENDING/, out);
  assert.match(out, /RESOLVED|REJECTED/, out);
  const at = Number(/(?:REJECTED at|at) (\d+)/.exec(out)?.[1] ?? "99999");
  assert.ok(at < 4_500, `settled at ${at} ms (deadline 3 s)`);
  assert.match(out, /OPEN SOCKETS 0/, out);
  assert.match(out, /ALIVE/, out);
  assert.doesNotMatch(out, /UNCAUGHT/, out);
});
