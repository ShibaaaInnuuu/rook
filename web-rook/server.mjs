// The site's server: static files, /healthz, and one API route. No framework, no dependency, no logging of visitors, no cookies.
// Serves dist/, falls back to index.html, and sets a Content-Security-Policy under which the page runs no script and no style from a
// third party.
//
// POST /api/upload (the Launch page's only server call): the page sends the coin's image (base64 inside a small JSON body) and its
// name, ticker, description and links. The server checks the image (PNG, JPG, GIF or WebP by its magic bytes, at most 2 MB), uploads
// it to Cloudinary with a SIGNED upload into the folder rook-coins/, builds the coin's metadata JSON (Metaplex / DBC fields: name,
// symbol, description, image, links, properties.files) around the image's URL, uploads that as a raw file, and answers the JSON's
// https URL — the uri the launch transaction writes into the coin. The Cloudinary credentials are read from the environment
// (CLOUDINARY_CLOUD_NAME, CLOUDINARY_API_KEY, CLOUDINARY_API_SECRET); they never reach the browser and are never logged. Unset, the
// route answers 503 and the page says "Image upload isn't set up yet". Ten uploads an hour per IP; bodies over 3 MB are refused.
//
// CSP on Solana (2026-10-10):
//   connect-src 'self' https: wss: — 'self' is /api/upload. The page reads Solana over JSON-RPC (src/config/solana.ts's endpoint or
//     VITE_SOLANA_RPC) and fetches coins' metadata JSON from wherever their creators host it (https, IPFS and Arweave gateways),
//     so any https host is allowed; wss: is for RPC providers that offer websockets. None of them is sent anything about the visitor.
//   img-src 'self' data: https: — coin images (creator-hosted; collection logos are 'self'), wallet icons (data: URIs), and the Launch page's
//     preview of a picked image (data: URI). Images load with no referrer.
//   frame-ancestors 'none' keeps the site out of iframes (clickjacking on a signing page).
import { createServer } from "node:http";
import { createHash, randomBytes } from "node:crypto";
import { readFile, stat } from "node:fs/promises";
import { extname, join, normalize } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const DIST = join(fileURLToPath(new URL(".", import.meta.url)), "dist");
const TYPES = { ".html": "text/html; charset=utf-8", ".js": "text/javascript; charset=utf-8", ".css": "text/css; charset=utf-8", ".json": "application/json; charset=utf-8",
  ".woff": "font/woff", ".woff2": "font/woff2", ".svg": "image/svg+xml", ".png": "image/png", ".webp": "image/webp", ".jpg": "image/jpeg", ".jpeg": "image/jpeg", ".gif": "image/gif", ".avif": "image/avif", ".ico": "image/x-icon", ".txt": "text/plain; charset=utf-8" };
export const SECURITY = {
  "content-security-policy": "default-src 'none'; script-src 'self'; style-src 'self' 'unsafe-inline'; font-src 'self'; img-src 'self' data: https:; connect-src 'self' https: wss: http://127.0.0.1:* http://localhost:*; base-uri 'none'; form-action 'none'; frame-ancestors 'none'",
  "x-content-type-options": "nosniff", "referrer-policy": "no-referrer", "x-frame-options": "DENY", "permissions-policy": "geolocation=(), camera=(), microphone=(), interest-cohort=()",
  "strict-transport-security": "max-age=31536000; includeSubDomains",
};

export const MAX_IMAGE_BYTES = 2 * 1024 * 1024;
/** 2 MB of image is 2.8 MB of base64; the rest of the body is a few short strings. */
export const MAX_BODY_BYTES = 3 * 1024 * 1024;
export const UPLOADS_PER_HOUR = 10;
const DRAIN_LIMIT = 8 * 1024 * 1024;
export const FOLDER = "rook-coins";
const NAME_BYTES = 32, SYMBOL_BYTES = 10, DESCRIPTION_CHARS = 600, LINK_CHARS = 200, URI_BYTES = 200;

/** The image's type from its first bytes, or null: only PNG, JPEG, GIF and WebP pass. */
export function sniffImage(b) {
  if (!b || b.length < 12) return null;
  if (b[0] === 0x89 && b[1] === 0x50 && b[2] === 0x4e && b[3] === 0x47 && b[4] === 0x0d && b[5] === 0x0a && b[6] === 0x1a && b[7] === 0x0a) return "image/png";
  if (b[0] === 0xff && b[1] === 0xd8 && b[2] === 0xff) return "image/jpeg";
  if (b[0] === 0x47 && b[1] === 0x49 && b[2] === 0x46 && b[3] === 0x38 && (b[4] === 0x37 || b[4] === 0x39) && b[5] === 0x61) return "image/gif";
  if (b[0] === 0x52 && b[1] === 0x49 && b[2] === 0x46 && b[3] === 0x46 && b[8] === 0x57 && b[9] === 0x45 && b[10] === 0x42 && b[11] === 0x50) return "image/webp";
  return null;
}

/**
 * The slugs of Rook's collections: an upload names one of these or is refused (Solana slugs have underscores, e.g. mad_lads). This
 * file runs alone in the deployed image (no import from src/), so the list is a copy of src/rook/sol/collectionLinks.ts
 * COLLECTION_LINKS' keys, and test/coinlinks.test.ts fails while the two differ. The coin's links are the creator's, as sent.
 */
export const COLLECTION_SLUGS = new Set([
  "milady", "azuki", "mayc", "pudgypenguins", "bayc", "doodles", "moonbirds", "clonex", "lilpudgys", "checks", "identitymd",
  "bitcoin-frogs", "nodemonkes", "omb", "bitcoin-puppets", "bitcoin-punks", "natcats", "runestone", "bitcoin-weirdos", "cents", "rsic",
  "pizza-ninjas", "quantum-cats", "wizards-of-ord", "taproot-wizards", "bitcoin-wizards",
  "mad_lads", "smb_gen2", "claynosaurz", "famous_fox_federation", "degods", "galactic_geckos", "taiyo_robotics", "okay_bears", "smb_gen3",
  "transdimensional_fox_federation", "pythenians", "cyber_frogs",
  "stonkbrokers", "chainmancers", "rekttradooors",
]);

/** The coin's off-chain metadata, as wallets, explorers, Meteora and Jupiter read it (Metaplex fungible standard + pump-style links). */
export function coinMetadata(f, image, mime) {
  const links = {};
  for (const k of ["website", "twitter", "telegram"]) if (f[k]) links[k] = f[k];
  return {
    name: f.name, symbol: f.symbol, description: f.description, image,
    ...links,
    ...(f.website ? { external_url: f.website } : {}),
    extensions: links,
    properties: { category: "image", files: [{ uri: image, type: mime }] },
    createdOn: "https://userook.fun",
    rook: { collection: f.collection },
  };
}

const utf8 = (s) => Buffer.byteLength(s, "utf8");
const str = (v) => (typeof v === "string" ? v.trim() : "");
/** The request's fields, checked; or { error } in words the page can show as is. */
export function checkFields(j) {
  if (!j || typeof j !== "object") return { error: "The request is not a JSON object." };
  const f = { name: str(j.name), symbol: str(j.symbol), description: str(j.description), website: str(j.website), twitter: str(j.twitter), telegram: str(j.telegram), collection: str(j.collection) };
  if (!f.name || utf8(f.name) > NAME_BYTES) return { error: `The name must be 1 to ${NAME_BYTES} characters.` };
  if (!f.symbol || utf8(f.symbol) > SYMBOL_BYTES) return { error: `The ticker must be 1 to ${SYMBOL_BYTES} characters.` };
  if (f.description.length > DESCRIPTION_CHARS) return { error: `The description is at most ${DESCRIPTION_CHARS} characters.` };
  for (const k of ["website", "twitter", "telegram"]) {
    if (f[k] && (!/^https:\/\/[^\s"<>]+$/i.test(f[k]) || f[k].length > LINK_CHARS)) return { error: `The ${k === "twitter" ? "X" : k} link must be an https:// address of at most ${LINK_CHARS} characters.` };
  }
  if (f.twitter && !/^https:\/\/(www\.)?(x|twitter)\.com\/[^\s"<>]+$/i.test(f.twitter)) return { error: "The X link must be an https://x.com/ address." };
  if (f.telegram && !/^https:\/\/(t\.me|telegram\.me)\/[^\s"<>]+$/i.test(f.telegram)) return { error: "The Telegram link must be a https://t.me/ address." };
  if (!COLLECTION_SLUGS.has(f.collection)) return { error: "Choose the collection first." };
  return { fields: f };
}

/** "data:image/png;base64,…" or bare base64 → bytes, or null when it is not base64. */
export function decodeImage(v) {
  if (typeof v !== "string" || !v) return null;
  const m = v.match(/^data:[^;,]*;base64,(.*)$/s);
  const b64 = (m ? m[1] : v).replace(/\s+/g, "");
  if (!/^[A-Za-z0-9+/]*={0,2}$/.test(b64)) return null;
  return Buffer.from(b64, "base64");
}

/** Cloudinary's signature: SHA-1 of the signed params, sorted, joined with &, followed by the secret. */
export function cloudinarySignature(params, secret) {
  const s = Object.keys(params).filter((k) => params[k] !== undefined && params[k] !== "").sort().map((k) => `${k}=${params[k]}`).join("&");
  return createHash("sha1").update(s + secret).digest("hex");
}

export function cloudinaryEnv(env) {
  const cloudName = (env.CLOUDINARY_CLOUD_NAME ?? "").trim(), apiKey = (env.CLOUDINARY_API_KEY ?? "").trim(), apiSecret = (env.CLOUDINARY_API_SECRET ?? "").trim();
  return cloudName && apiKey && apiSecret && /^[a-z0-9_-]+$/i.test(cloudName) ? { cloudName, apiKey, apiSecret } : null;
}

async function cloudinaryUpload(cfg, fetchImpl, kind, blob, filename, extra, nowSec) {
  const params = { folder: FOLDER, timestamp: String(nowSec), ...extra };
  const body = new FormData();
  for (const [k, v] of Object.entries(params)) body.append(k, v);
  body.append("api_key", cfg.apiKey);
  body.append("signature", cloudinarySignature(params, cfg.apiSecret));
  body.append("file", blob, filename);
  const r = await fetchImpl(`https://api.cloudinary.com/v1_1/${encodeURIComponent(cfg.cloudName)}/${kind}/upload`, { method: "POST", body, signal: AbortSignal.timeout(25_000) });
  const j = await r.json().catch(() => ({}));
  if (!r.ok || typeof j.secure_url !== "string" || !j.secure_url.startsWith("https://")) {
    // the status only: Cloudinary's message can echo request details, and nothing about the visitor is logged
    console.error(`upload: Cloudinary ${kind} upload answered HTTP ${r.status}`);
    throw new Error("upstream");
  }
  return j.secure_url;
}

/** A sliding-window limit per key (the visitor's IP), in memory. Old keys are dropped as the map is used. */
export function rateLimiter(limit = UPLOADS_PER_HOUR, windowMs = 3_600_000) {
  const hits = new Map();
  return {
    /** true when this request may go ahead (and is counted); false when the key has used its allowance. */
    take(key, now) {
      if (hits.size > 10_000) for (const [k, ts] of hits) if (!ts.length || ts[ts.length - 1] <= now - windowMs) hits.delete(k);
      const ts = (hits.get(key) ?? []).filter((t) => t > now - windowMs);
      if (ts.length >= limit) { hits.set(key, ts); return { ok: false, retryAfter: Math.ceil((ts[0] + windowMs - now) / 1000) }; }
      ts.push(now); hits.set(key, ts);
      return { ok: true, retryAfter: 0 };
    },
  };
}

/** Railway's edge sets X-Real-IP; failing that, the right-most X-Forwarded-For hop (the one the edge appended), then the socket. */
export function clientIp(req) {
  const real = req.headers["x-real-ip"];
  if (typeof real === "string" && real.trim()) return real.trim();
  const xff = req.headers["x-forwarded-for"];
  if (typeof xff === "string" && xff.trim()) { const hops = xff.split(",").map((s) => s.trim()).filter(Boolean); if (hops.length) return hops[hops.length - 1]; }
  return req.socket?.remoteAddress ?? "unknown";
}

function readBody(req, cap) {
  return new Promise((resolve, reject) => {
    const declared = Number(req.headers["content-length"] ?? 0);
    // over the cap: answer 413 at once. A modest overshoot is drained so the client reads the answer; a huge one closes the connection.
    if (declared > cap) { reject(Object.assign(new Error("too big"), { status: 413, close: declared > DRAIN_LIMIT })); req.resume(); return; }
    const chunks = []; let n = 0, done = false;
    req.on("data", (c) => {
      if (done) return;
      n += c.length;
      if (n > cap) { done = true; reject(Object.assign(new Error("too big"), { status: 413, close: true })); req.resume(); return; }
      chunks.push(c);
    });
    req.on("end", () => { if (!done) { done = true; resolve(Buffer.concat(chunks)); } });
    req.on("error", (e) => { if (!done) { done = true; reject(e); } });
  });
}

function sendJson(res, status, obj, extra = {}) {
  res.writeHead(status, { "content-type": "application/json; charset=utf-8", "cache-control": "no-store", ...SECURITY, ...extra });
  res.end(JSON.stringify(obj));
}

/** POST /api/upload. Answers { uri, image } or { error } (a sentence for the page). */
async function upload(req, res, o) {
  // GET: whether uploads are configured (a boolean; nothing else about the setup), so the page can say so before the visitor fills it in
  if (req.method === "GET" || req.method === "HEAD") return sendJson(res, 200, { ready: !!cloudinaryEnv(o.env) });
  if (req.method !== "POST") return sendJson(res, 405, { error: "POST only." }, { allow: "GET, POST" });
  const origin = req.headers.origin;
  if (typeof origin === "string" && origin && origin !== "null") {
    let host = "";
    try { host = new URL(origin).host; } catch { /* a malformed origin is refused below */ }
    if (host !== req.headers.host) return sendJson(res, 403, { error: "Uploads are accepted from this site only." });
  }
  const cfg = cloudinaryEnv(o.env);
  if (!cfg) return sendJson(res, 503, { error: "Image upload isn't set up yet.", code: "not-configured" });
  if (!/^application\/json\b/i.test(String(req.headers["content-type"] ?? ""))) return sendJson(res, 415, { error: "Send the upload as JSON." });
  const gate = o.limiter.take(clientIp(req), o.now());
  if (!gate.ok) return sendJson(res, 429, { error: `Too many uploads from your connection. Try again in about ${Math.max(1, Math.ceil(gate.retryAfter / 60))} minutes.`, code: "rate-limited" }, { "retry-after": String(gate.retryAfter) });
  let raw;
  try { raw = await readBody(req, MAX_BODY_BYTES); } catch (e) {
    return sendJson(res, e.status === 413 ? 413 : 400, { error: e.status === 413 ? "The image is too large: at most 2 MB." : "The upload was interrupted." }, e.close ? { connection: "close" } : {});
  }
  let j;
  try { j = JSON.parse(raw.toString("utf8")); } catch { return sendJson(res, 400, { error: "The request is not valid JSON." }); }
  const checked = checkFields(j);
  if (checked.error) return sendJson(res, 400, { error: checked.error });
  const img = decodeImage(j?.image);
  if (!img || !img.length) return sendJson(res, 400, { error: "The coin needs an image." });
  if (img.length > MAX_IMAGE_BYTES) return sendJson(res, 413, { error: "The image is too large: at most 2 MB." });
  const mime = sniffImage(img);
  if (!mime) return sendJson(res, 415, { error: "The image must be a PNG, JPG, GIF or WebP file." });
  const f = checked.fields;
  const nowSec = Math.floor(o.now() / 1000);
  try {
    const ext = mime.split("/")[1].replace("jpeg", "jpg");
    const image = await cloudinaryUpload(cfg, o.fetch, "image", new Blob([img], { type: mime }), `coin.${ext}`, {}, nowSec);
    const meta = JSON.stringify(coinMetadata(f, image, mime));
    const publicId = `${randomBytes(12).toString("hex")}.json`;
    const uri = await cloudinaryUpload(cfg, o.fetch, "raw", new Blob([meta], { type: "application/json" }), publicId, { public_id: publicId }, nowSec);
    if (utf8(uri) > URI_BYTES) return sendJson(res, 502, { error: "The image host returned an address too long to write on chain. Try again." });
    return sendJson(res, 200, { uri, image });
  } catch {
    return sendJson(res, 502, { error: "The image host didn't accept the upload. Try again in a minute." });
  }
}

async function fileFor(root, path) {
  const safe = normalize(decodeURIComponent(path)).replace(/^(\.\.[/\\])+/, "");
  const full = join(root, safe);
  if (!full.startsWith(root)) return null;
  try { const s = await stat(full); if (s.isFile()) return full; } catch { /* fall through */ }
  return null;
}

/** The request handler. Options exist for tests: another dist/, another environment, a mocked fetch, a clock. */
export function createHandler(opts = {}) {
  const o = { root: opts.root ?? DIST, env: opts.env ?? process.env, fetch: opts.fetch ?? fetch, now: opts.now ?? Date.now, limiter: opts.limiter ?? rateLimiter() };
  return async (req, res) => {
    try {
      const url = new URL(req.url ?? "/", "http://x");
      if (url.pathname === "/healthz") { res.writeHead(200, { "content-type": "text/plain; charset=utf-8", "cache-control": "no-store" }); return res.end("ok\n"); }
      if (url.pathname === "/api/upload") return await upload(req, res, o);
      if (url.pathname.startsWith("/api/")) return sendJson(res, 404, { error: "Not found." });
      if (req.method !== "GET" && req.method !== "HEAD") { res.writeHead(405, { allow: "GET, HEAD" }); return res.end(); }
      // a missing asset is a 404; any other path is the app (it routes by hash, so this only matters for a mistyped URL)
      let file = await fileFor(o.root, url.pathname === "/" ? "/index.html" : url.pathname);
      if (!file && extname(url.pathname)) { res.writeHead(404, { "content-type": "text/plain; charset=utf-8", ...SECURITY }); return res.end("not found\n"); }
      file ??= join(o.root, "index.html");
      const body = await readFile(file);
      const hashed = /\/assets\//.test(file);
      // /wx/: the wrapped tokens' metadata and pictures (scripts/wx-metadata.mjs), the uri every wX carries on chain; explorers and
      // wallets that read it from a browser need it readable from any origin
      const open = url.pathname.startsWith("/wx/") ? { "access-control-allow-origin": "*" } : {};
      res.writeHead(200, { "content-type": TYPES[extname(file)] ?? "application/octet-stream", "cache-control": hashed ? "public, max-age=31536000, immutable" : "no-cache", ...SECURITY, ...open });
      res.end(req.method === "HEAD" ? undefined : body);
    } catch {
      if (!res.headersSent) res.writeHead(500, { "content-type": "text/plain; charset=utf-8" });
      res.end("error\n");
    }
  };
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const PORT = Number(process.env.PORT ?? 8080);
  createServer(createHandler()).listen(PORT, "0.0.0.0", () => console.log(`rook web: listening on :${PORT}${cloudinaryEnv(process.env) ? "" : " (image upload not configured: CLOUDINARY_* unset)"}`));
}
