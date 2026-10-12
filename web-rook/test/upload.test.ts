import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { mkdtempSync, writeFileSync } from "node:fs";
import { createServer, type Server } from "node:http";
import type { AddressInfo } from "node:net";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { after, before, test } from "node:test";
import { checkFields, cloudinarySignature, coinMetadata, createHandler, rateLimiter, sniffImage } from "../server.mjs";

/** server.mjs's POST /api/upload, with Cloudinary mocked: type checks, size, rate limit, missing env, the metadata JSON's shape, secrets. */
const ENV = { CLOUDINARY_CLOUD_NAME: "rookcloud", CLOUDINARY_API_KEY: "key123", CLOUDINARY_API_SECRET: "s3cr3t-never-leaves" };
const PNG = Buffer.concat([Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]), Buffer.alloc(64, 1)]);
const JPG = Buffer.concat([Buffer.from([0xff, 0xd8, 0xff, 0xe0]), Buffer.alloc(64, 2)]);
const GIF = Buffer.concat([Buffer.from("GIF89a", "latin1"), Buffer.alloc(64, 3)]);
const WEBP = Buffer.concat([Buffer.from("RIFF", "latin1"), Buffer.alloc(4), Buffer.from("WEBP", "latin1"), Buffer.alloc(64, 4)]);
const SVG = Buffer.from("<svg xmlns='http://www.w3.org/2000/svg'><script>alert(1)</script></svg>");
const fields = { name: "Frog Coin", symbol: "FROG", description: "hi", website: "https://frog.example", twitter: "https://x.com/frog", telegram: "", collection: "bitcoin-frogs" };

type Call = { url: string; form: FormData };
let calls: Call[] = [];
let failNext = false;
const mockFetch = (async (url: string | URL, init?: RequestInit) => {
  const form = init!.body as FormData;
  calls.push({ url: String(url), form });
  if (failNext) { failNext = false; return new Response(JSON.stringify({ error: { message: "Invalid Signature s3cr3t" } }), { status: 401 }); }
  const raw = String(url).includes("/raw/upload");
  return new Response(JSON.stringify({ secure_url: raw ? `https://res.cloudinary.com/rookcloud/raw/upload/v1/rook-coins/${form.get("public_id")}` : "https://res.cloudinary.com/rookcloud/image/upload/v1/rook-coins/abc.png" }), { status: 200, headers: { "content-type": "application/json" } });
}) as typeof fetch;

const servers: Server[] = [];
async function serve(opts: Parameters<typeof createHandler>[0]): Promise<string> {
  const root = mkdtempSync(join(tmpdir(), "rook-web-"));
  writeFileSync(join(root, "index.html"), "<!doctype html><title>Rook</title>");
  const s = createServer(createHandler({ root, fetch: mockFetch, ...opts }));
  servers.push(s);
  await new Promise<void>((r) => s.listen(0, "127.0.0.1", r));
  return `http://127.0.0.1:${(s.address() as AddressInfo).port}`;
}
const post = (base: string, body: unknown, headers: Record<string, string> = {}) =>
  fetch(`${base}/api/upload`, { method: "POST", headers: { "content-type": "application/json", ...headers }, body: typeof body === "string" ? body : JSON.stringify(body) });
const withImage = (img: Buffer, mime = "image/png", extra: Record<string, unknown> = {}) => ({ ...fields, image: `data:${mime};base64,${img.toString("base64")}`, ...extra });

let base = "";
before(async () => { base = await serve({ env: ENV }); });
after(() => { for (const s of servers) s.close(); });

test("the magic bytes decide the type: PNG, JPEG, GIF, WebP pass; SVG, text and short buffers do not", () => {
  assert.equal(sniffImage(PNG), "image/png");
  assert.equal(sniffImage(JPG), "image/jpeg");
  assert.equal(sniffImage(GIF), "image/gif");
  assert.equal(sniffImage(WEBP), "image/webp");
  assert.equal(sniffImage(SVG), null);
  assert.equal(sniffImage(Buffer.from("hello world, not an image")), null);
  assert.equal(sniffImage(Buffer.from([0x89, 0x50])), null);
});

test("an upload goes to Cloudinary signed, into rook-coins/, image then metadata JSON; the answer is the JSON's URL", async () => {
  calls = [];
  const r = await post(base, withImage(PNG));
  assert.equal(r.status, 200);
  const j = await r.json() as { uri: string; image: string };
  assert.match(j.uri, /^https:\/\/res\.cloudinary\.com\/rookcloud\/raw\/upload\/.*\/rook-coins\/[0-9a-f]{24}\.json$/);
  assert.equal(j.image, "https://res.cloudinary.com/rookcloud/image/upload/v1/rook-coins/abc.png");
  assert.equal(calls.length, 2);
  assert.equal(calls[0].url, "https://api.cloudinary.com/v1_1/rookcloud/image/upload");
  assert.equal(calls[1].url, "https://api.cloudinary.com/v1_1/rookcloud/raw/upload");
  for (const c of calls) {
    assert.equal(c.form.get("folder"), "rook-coins");
    assert.equal(c.form.get("api_key"), "key123");
    assert.equal(c.form.get("api_secret"), null, "the secret is never sent, only the signature");
    const signed: Record<string, string> = { folder: "rook-coins", timestamp: String(c.form.get("timestamp")) };
    if (c.form.get("public_id")) signed.public_id = String(c.form.get("public_id"));
    assert.equal(c.form.get("signature"), cloudinarySignature(signed, ENV.CLOUDINARY_API_SECRET));
  }
  // the image went up as bytes, the metadata as JSON
  const img = calls[0].form.get("file") as Blob;
  assert.equal(img.type, "image/png");
  assert.deepEqual(Buffer.from(await img.arrayBuffer()), PNG);
  const meta = JSON.parse(await (calls[1].form.get("file") as Blob).text());
  assert.deepEqual(meta, {
    name: "Frog Coin", symbol: "FROG", description: "hi", image: j.image,
    website: "https://frog.example", twitter: "https://x.com/frog", external_url: "https://frog.example",
    extensions: { website: "https://frog.example", twitter: "https://x.com/frog" },
    properties: { category: "image", files: [{ uri: j.image, type: "image/png" }] },
    createdOn: "https://userook.fun", rook: { collection: "bitcoin-frogs" },
  });
});

test("Cloudinary's signature is SHA-1 over the sorted params followed by the secret", () => {
  assert.equal(cloudinarySignature({ timestamp: "1315060510", public_id: "sample_image", eager: "w_400,h_300,c_pad|w_260,h_200,c_crop" }, "abcd"),
    createHash("sha1").update("eager=w_400,h_300,c_pad|w_260,h_200,c_crop&public_id=sample_image&timestamp=1315060510abcd").digest("hex"));
  assert.equal(cloudinarySignature({ timestamp: "1315060510", public_id: "sample_image", eager: "w_400,h_300,c_pad|w_260,h_200,c_crop" }, "abcd"), "bfd09f95f331f558cbd1320e67aa8d488770583e", "Cloudinary's documented example");
});

test("type checks: JPEG, GIF and WebP pass; SVG or a renamed text file is refused whatever the data URL claims", async () => {
  for (const [img, mime] of [[JPG, "image/jpeg"], [GIF, "image/gif"], [WEBP, "image/webp"]] as const) {
    calls = [];
    const r = await post(`${await serve({ env: ENV })}`, withImage(img, mime));
    assert.equal(r.status, 200, mime);
    assert.equal((calls[0].form.get("file") as Blob).type, mime);
  }
  const svg = await post(base, withImage(SVG, "image/svg+xml"));
  assert.equal(svg.status, 415);
  assert.match((await svg.json() as { error: string }).error, /PNG, JPG, GIF or WebP/);
  const fake = await post(base, withImage(Buffer.from("just text pretending to be an image"), "image/png"));
  assert.equal(fake.status, 415);
});

test("size: an image over 2 MB is 413; a body over the cap is 413 before it is read; garbage and bad fields are 400", async () => {
  const b = await serve({ env: ENV });
  const big = Buffer.concat([PNG, Buffer.alloc(2 * 1024 * 1024)]);
  const r = await post(b, withImage(big));
  assert.equal(r.status, 413);
  assert.match((await r.json() as { error: string }).error, /2 MB/);
  const huge = await post(b, JSON.stringify({ ...fields, image: "A".repeat(3 * 1024 * 1024 + 10) }));
  assert.equal(huge.status, 413);
  assert.equal((await post(b, "{not json")).status, 400);
  assert.equal((await post(b, withImage(PNG, "image/png", { name: "" }))).status, 400);
  assert.equal((await post(b, withImage(PNG, "image/png", { symbol: "WAYTOOLONGTICKER" }))).status, 400);
  assert.equal((await post(b, withImage(PNG, "image/png", { website: "javascript:alert(1)" }))).status, 400);
  assert.equal((await post(b, withImage(PNG, "image/png", { twitter: "https://evil.example/frog" }))).status, 400, "an X link is an x.com one");
  assert.equal((await post(b, withImage(PNG, "image/png", { telegram: "https://evil.example/x" }))).status, 400, "a Telegram link is a t.me one");
  assert.equal((await post(b, withImage(PNG, "image/png", { collection: "not-a-rook-collection" }))).status, 400);
  assert.equal((await post(b, { ...fields })).status, 400, "no image");
  assert.equal((await post(b, withImage(PNG), { "content-type": "text/plain" })).status, 415);
});

test("rate limit: ten uploads an hour per IP, then 429 with Retry-After; another IP is not affected; the window slides", async () => {
  let now = 1_800_000_000_000;
  const b = await serve({ env: ENV, now: () => now });
  for (let i = 0; i < 10; i++) assert.equal((await post(b, withImage(PNG), { "x-real-ip": "203.0.113.7" })).status, 200, `upload ${i + 1}`);
  const r = await post(b, withImage(PNG), { "x-real-ip": "203.0.113.7" });
  assert.equal(r.status, 429);
  assert.ok(Number(r.headers.get("retry-after")) > 0);
  assert.match((await r.json() as { error: string }).error, /Too many uploads/);
  assert.equal((await post(b, withImage(PNG), { "x-real-ip": "198.51.100.1" })).status, 200);
  now += 3_600_001;
  assert.equal((await post(b, withImage(PNG), { "x-real-ip": "203.0.113.7" })).status, 200);
  const lim = rateLimiter(2, 1000);
  assert.equal(lim.take("a", 0).ok, true); assert.equal(lim.take("a", 10).ok, true); assert.equal(lim.take("a", 20).ok, false); assert.equal(lim.take("a", 1001).ok, true);
});

test("missing env: 503 with the sentence the page shows; GET says ready false; partial env counts as missing", async () => {
  for (const env of [{}, { CLOUDINARY_CLOUD_NAME: "x", CLOUDINARY_API_KEY: "y" }]) {
    calls = [];
    const b = await serve({ env });
    const r = await post(b, withImage(PNG));
    assert.equal(r.status, 503);
    assert.deepEqual(await r.json(), { error: "Image upload isn't set up yet.", code: "not-configured" });
    assert.equal(calls.length, 0);
    assert.deepEqual(await (await fetch(`${b}/api/upload`)).json(), { ready: false });
  }
  assert.deepEqual(await (await fetch(`${base}/api/upload`)).json(), { ready: true });
});

test("the secret is never in a response or a log, even when Cloudinary fails; cross-origin posts are refused", async () => {
  const b = await serve({ env: ENV });
  const logged: string[] = [];
  const orig = console.error, origLog = console.log;
  console.error = (...a: unknown[]) => { logged.push(a.join(" ")); };
  console.log = (...a: unknown[]) => { logged.push(a.join(" ")); };
  try {
    failNext = true;
    const r = await post(b, withImage(PNG));
    const text = await r.text();
    assert.equal(r.status, 502);
    assert.doesNotMatch(text, /s3cr3t|key123/);
    assert.match(JSON.parse(text).error, /didn't accept/);
  } finally { console.error = orig; console.log = origLog; }
  assert.ok(logged.length > 0 && logged.every((l) => !/s3cr3t|key123|203\.0\.113/.test(l)), logged.join("\n"));
  const x = await post(b, withImage(PNG), { origin: "https://evil.example" });
  assert.equal(x.status, 403);
});

test("the page and /healthz still work; the CSP allows 'self' for /api; other /api paths are 404", async () => {
  const h = await fetch(`${base}/healthz`);
  assert.equal(h.status, 200);
  assert.equal(await h.text(), "ok\n");
  const page = await fetch(`${base}/launch`);
  assert.equal(page.status, 200);
  assert.match(page.headers.get("content-security-policy")!, /connect-src 'self'/);
  assert.equal((await fetch(`${base}/api/nope`)).status, 404);
  assert.equal((await fetch(`${base}/`, { method: "POST" })).status, 405);
});

test("field checks and the metadata builder, pure", () => {
  assert.equal(checkFields({ ...fields, collection: "../x" }).error, "Choose the collection first.");
  assert.equal(checkFields({ ...fields, name: "x".repeat(33) }).error?.includes("32"), true);
  assert.deepEqual(checkFields({ ...fields, name: "  Frog Coin  " }).fields?.name, "Frog Coin");
  assert.equal(checkFields({ ...fields, collection: "mad_lads" }).error, undefined, "a Solana collection's slug has underscores");
  assert.equal(checkFields({ ...fields, collection: "transdimensional_fox_federation" }).error, undefined);
  assert.equal(checkFields({ ...fields, collection: "rook" }).error, "Choose the collection first.", "not a Rook collection");
  assert.equal(checkFields({ ...fields, twitter: "https://twitter.com/frog", telegram: "https://t.me/frogs" }).error, undefined);
  // the creator's links are kept as sent, and empty stays empty: the collection's are never forced in
  const sent = checkFields({ ...fields, website: "https://mine.example", twitter: "https://x.com/me" }).fields!;
  assert.deepEqual([sent.website, sent.twitter], ["https://mine.example", "https://x.com/me"]);
  const empty = checkFields({ ...fields, website: "", twitter: "", telegram: "" }).fields!;
  const m = coinMetadata(empty, "https://img.example/a.webp", "image/webp");
  assert.equal("external_url" in m, false);
  assert.equal("website" in m || "twitter" in m, false, "no website or X from the collection (bitcoinfrogs.com) when the creator left them empty");
  assert.deepEqual(m.extensions, {});
  assert.deepEqual((m.properties as { files: unknown[] }).files, [{ uri: "https://img.example/a.webp", type: "image/webp" }]);
});
