// A static file server and nothing else: no framework, no dependency, no logging of visitors, no cookies.
// Serves dist/, answers /healthz, falls back to index.html, and sets a Content-Security-Policy under which the page can load
// nothing from a third party. connect-src is open to https because the page talks to the chain's RPC, to drand, and to wherever
// a keeper published a snapshot file; it sends none of them anything about the visitor.
import { createServer } from "node:http";
import { readFile, stat } from "node:fs/promises";
import { extname, join, normalize } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = join(fileURLToPath(new URL(".", import.meta.url)), "dist");
const PORT = Number(process.env.PORT ?? 8080);
const TYPES = { ".html": "text/html; charset=utf-8", ".js": "text/javascript; charset=utf-8", ".css": "text/css; charset=utf-8", ".json": "application/json; charset=utf-8",
  ".woff": "font/woff", ".woff2": "font/woff2", ".svg": "image/svg+xml", ".png": "image/png", ".webp": "image/webp", ".jpg": "image/jpeg", ".jpeg": "image/jpeg", ".gif": "image/gif", ".avif": "image/avif", ".ico": "image/x-icon", ".txt": "text/plain; charset=utf-8" };
const SECURITY = {
  "content-security-policy": "default-src 'none'; script-src 'self'; style-src 'self' 'unsafe-inline'; font-src 'self'; img-src 'self' data:; connect-src 'self' https: http://127.0.0.1:* http://localhost:*; base-uri 'none'; form-action 'none'; frame-ancestors 'none'",
  "x-content-type-options": "nosniff", "referrer-policy": "no-referrer", "x-frame-options": "DENY", "permissions-policy": "geolocation=(), camera=(), microphone=(), interest-cohort=()",
  "strict-transport-security": "max-age=31536000; includeSubDomains",
};

async function fileFor(path) {
  const safe = normalize(decodeURIComponent(path)).replace(/^(\.\.[/\\])+/, "");
  const full = join(ROOT, safe);
  if (!full.startsWith(ROOT)) return null;
  try { const s = await stat(full); if (s.isFile()) return full; } catch { /* fall through */ }
  return null;
}

createServer(async (req, res) => {
  try {
    const url = new URL(req.url ?? "/", "http://x");
    if (url.pathname === "/healthz") { res.writeHead(200, { "content-type": "text/plain; charset=utf-8", "cache-control": "no-store" }); return res.end("ok\n"); }
    if (req.method !== "GET" && req.method !== "HEAD") { res.writeHead(405, { allow: "GET, HEAD" }); return res.end(); }
    // a missing asset is a 404; any other path is the app (it routes by hash, so this only matters for a mistyped URL)
    let file = await fileFor(url.pathname === "/" ? "/index.html" : url.pathname);
    if (!file && extname(url.pathname)) { res.writeHead(404, { "content-type": "text/plain; charset=utf-8", ...SECURITY }); return res.end("not found\n"); }
    file ??= join(ROOT, "index.html");
    const body = await readFile(file);
    const hashed = /\/assets\//.test(file);
    res.writeHead(200, { "content-type": TYPES[extname(file)] ?? "application/octet-stream", "cache-control": hashed ? "public, max-age=31536000, immutable" : "no-cache", ...SECURITY });
    res.end(req.method === "HEAD" ? undefined : body);
  } catch {
    res.writeHead(500, { "content-type": "text/plain; charset=utf-8" }); res.end("error\n");
  }
}).listen(PORT, "0.0.0.0", () => console.log(`Rook web: listening on :${PORT}`));
