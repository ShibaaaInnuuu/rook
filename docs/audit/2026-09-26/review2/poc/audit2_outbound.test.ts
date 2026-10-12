// Second review (R2) of E-12: indexer/src/outbound.ts. PoCs assert the behaviour found, so they pass while the bug is present.
import { test } from "node:test";
import assert from "node:assert/strict";
import { execFileSync, spawnSync } from "node:child_process";
import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { isPublicIp, hostRefusal, urlRefusal, guardedFetch } from "../src/outbound.js";

test("R2 E-12: address forms are all refused (no gap found)", async () => {
  for (const ip of ["::ffff:7f00:1", "::ffff:127.0.0.1", "::127.0.0.1", "64:ff9b::a00:1", "64:ff9b:1::1", "2002:7f00:1::", "fec0::1", "fd12::1",
    "fe80::1%eth0", "::", "::1", "0.1.2.3", "100.64.0.1", "169.254.169.254", "::ffff:0:7f00:1", "2001::1", "100::"]) assert.equal(isPublicIp(ip), false, ip);
  for (const u of ["https://2130706433/", "https://0x7f.1/", "https://0177.0.0.1/", "https://[::ffff:127.0.0.1]/", "https://127.1/", "http://example.com/"])
    assert.notEqual(hostRefusal(u), null, u);
  // a trailing-dot name slips the NAME rules, but the resolver check still catches its private answer
  assert.equal(hostRefusal("https://metadata.google.internal./"), null);
  assert.match(String(await urlRefusal("https://metadata.google.internal./", async () => [{ address: "169.254.169.254", family: 4 }])), /private/);
});

test("R2 E-12: a redirect to a private host through the guard is refused", async () => {
  const f = guardedFetch({ resolve: async (h) => [{ address: h === "a.example" ? "93.184.216.34" : "10.0.0.5", family: 4 }],
    fetch: (async () => new Response(null, { status: 302, headers: { location: "https://b.example/x" } })) as typeof fetch });
  await assert.rejects(f("https://a.example/"), /private/);
});

test("R2 NEW: pinnedHttpsFetch throws inside the 'response' listener on a status outside 200..599 -> uncaught exception kills the process", () => {
  const dir = mkdtempSync(join(tmpdir(), "audit2-"));
  execFileSync("openssl", ["req", "-x509", "-newkey", "rsa:2048", "-nodes", "-keyout", join(dir, "k.pem"), "-out", join(dir, "c.pem"), "-days", "1", "-subj", "/CN=localhost"], { stdio: "ignore" });
  const outbound = resolve(import.meta.dirname, "../src/outbound.ts");
  const script = `
    import tls from "node:tls"; import { readFileSync } from "node:fs";
    import { pinnedHttpsFetch } from ${JSON.stringify(outbound)};
    const srv = tls.createServer({ key: readFileSync(${JSON.stringify(join(dir, "k.pem"))}), cert: readFileSync(${JSON.stringify(join(dir, "c.pem"))}) }, (s) => {
      s.once("data", () => s.end("HTTP/1.1 999 Nope\\r\\ncontent-length: 0\\r\\n\\r\\n"));
    });
    srv.listen(0, "127.0.0.1", async () => {
      const port = srv.address().port;
      try { const r = await pinnedHttpsFetch("https://127.0.0.1:" + port + "/", { signal: AbortSignal.timeout(5000) }); console.log("RESOLVED", r.status); }
      catch (e) { console.log("REJECTED", e?.name); }
      srv.close(); process.exit(0);
    });`;
  writeFileSync(join(dir, "poc.mts"), script);
  const r = spawnSync(process.execPath, ["--import", "tsx", join(dir, "poc.mts")], { env: { ...process.env, NODE_TLS_REJECT_UNAUTHORIZED: "0" }, encoding: "utf8", timeout: 20_000, cwd: resolve(import.meta.dirname, "..") });
  const out = `${r.stdout}\n${r.stderr}`;
  // the bug: neither RESOLVED nor REJECTED; the process dies with the Response constructor's RangeError
  assert.doesNotMatch(out, /RESOLVED|REJECTED/, out);
  assert.match(out, /RangeError/, out);
  assert.notEqual(r.status, 0, out);
});
