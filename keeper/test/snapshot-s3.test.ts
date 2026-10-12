import assert from "node:assert/strict";
import { test } from "node:test";
import { createServer, type Server } from "node:http";
import { mkdtempSync, readFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { publish } from "../service/draws.js";
import { hasMirror, parseServiceConfig, ConfigError, DEFAULT_S3_PREFIX } from "../service/config.js";
import { mirrorRequired } from "../service/keeper.js";
import { serviceSecrets } from "../service/main.js";
import { urlRefusal } from "../service/outbound.js";
import { EMPTY_SHA256, objectKey, objectUrl, s3Request, sha256Hex, signV4, uriEncode, type S3Target } from "../service/s3.js";
import { S3Stub, expectedSignature, stubFetch, type StubRequest } from "./s3-stub.js";

// The snapshot mirror on an S3-compatible bucket (Cloudflare R2): SigV4 against AWS's published examples, the object key under the
// prefix, a stub bucket that checks every signature, the read-back rules, the configuration, and the secret kept out of every line.

// AWS's published example ("Signature Calculations for the Authorization Header: Transferring Payload in a Single Chunk"):
const AWS = { accessKeyId: "AKIAIOSFODNN7EXAMPLE", secretAccessKey: "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY", region: "us-east-1", service: "s3", amzDate: "20130524T000000Z" };

test("SigV4: AWS's published GET Object example gives its signature (and its canonical request's hash)", () => {
  const s = signV4({ ...AWS, method: "GET", url: "https://examplebucket.s3.amazonaws.com/test.txt", payloadSha256: EMPTY_SHA256,
    headers: { Range: "bytes=0-9", "x-amz-content-sha256": EMPTY_SHA256, "x-amz-date": AWS.amzDate } });
  assert.equal(s.canonicalRequest, ["GET", "/test.txt", "", "host:examplebucket.s3.amazonaws.com\nrange:bytes=0-9\n"
    + `x-amz-content-sha256:${EMPTY_SHA256}\nx-amz-date:20130524T000000Z\n`, "host;range;x-amz-content-sha256;x-amz-date", EMPTY_SHA256].join("\n"));
  assert.equal(sha256Hex(s.canonicalRequest), "7344ae5b7ee6c3e7e6b0fe0640412a37625d1fbfff95c48bbb2dc43964946972");
  assert.equal(s.signature, "f0e8bdb87c964420e857bd35b5d6ed310bd44f0170aba48dd91039c6036bdb41");
  assert.equal(s.authorization, "AWS4-HMAC-SHA256 Credential=AKIAIOSFODNN7EXAMPLE/20130524/us-east-1/s3/aws4_request, "
    + "SignedHeaders=host;range;x-amz-content-sha256;x-amz-date, Signature=f0e8bdb87c964420e857bd35b5d6ed310bd44f0170aba48dd91039c6036bdb41");
});

test("SigV4: AWS's published PUT Object example (a key with `$`, a payload) gives its signature", () => {
  const body = "Welcome to Amazon S3.";
  assert.equal(sha256Hex(body), "44ce7dd67c959e0d3524ffac1771dfbba87d2b6b4b4e99e42034a8b803f8b072");
  const url = `https://examplebucket.s3.amazonaws.com/${uriEncode("test$file.text", true)}`;
  assert.equal(url, "https://examplebucket.s3.amazonaws.com/test%24file.text");
  const s = signV4({ ...AWS, method: "PUT", url, payloadSha256: sha256Hex(body), headers: { Date: "Fri, 24 May 2013 00:00:00 GMT",
    "x-amz-date": AWS.amzDate, "x-amz-storage-class": "REDUCED_REDUNDANCY", "x-amz-content-sha256": sha256Hex(body) } });
  assert.equal(s.stringToSign, "AWS4-HMAC-SHA256\n20130524T000000Z\n20130524/us-east-1/s3/aws4_request\n9e0e90d9c76de8fa5b200d8c849cd5b8dc7a3be3951ddb7f6a76b4158342019d");
  assert.equal(s.signature, "98ad721746da40c64f1a55b78f14c238d841ea1380cd77a1b5971af0ece108bd");
});

test("the stub bucket's own check accepts AWS's published example with the example secret, and refuses it with any other", () => {
  const req: StubRequest = { method: "GET", url: "https://examplebucket.s3.amazonaws.com/test.txt", body: "", headers: { range: "bytes=0-9",
    "x-amz-content-sha256": EMPTY_SHA256, "x-amz-date": AWS.amzDate, authorization: "AWS4-HMAC-SHA256 Credential=AKIAIOSFODNN7EXAMPLE/20130524/us-east-1/s3/aws4_request,"
    + "SignedHeaders=host;range;x-amz-content-sha256;x-amz-date,Signature=f0e8bdb87c964420e857bd35b5d6ed310bd44f0170aba48dd91039c6036bdb41" } };
  const ok = expectedSignature(req, AWS.secretAccessKey, "us-east-1");
  assert.ok(!("error" in ok) && ok.signature === ok.given);
  const bad = expectedSignature(req, "another-secret-another-secret", "us-east-1");
  assert.ok(!("error" in bad) && bad.signature !== bad.given);
});

// ---- the object key, and a real round trip over HTTP to a stub bucket ------------------------------------------------------------

const NAME = `snapshot-0x${"c0".repeat(20)}-123-weighted.json`;
const TEXT = `{"version":2,"coin":"0x${"c0".repeat(20)}","tickets":[]}\n`;
const R2 = { accessKeyId: "0123456789abcdef0123456789abcdef", secretAccessKey: "f".repeat(8) + "0123456789abcdef".repeat(3) + "e".repeat(8), region: "auto" };

test("the object key is the prefix and the file's name; the URL is path-style at the endpoint", () => {
  const t = { endpoint: "https://acct.r2.cloudflarestorage.com", bucket: "rook", prefix: DEFAULT_S3_PREFIX };
  assert.equal(DEFAULT_S3_PREFIX, "rook-snapshots/");
  assert.equal(objectKey(t, NAME), `rook-snapshots/${NAME}`);
  assert.equal(objectUrl(t, NAME), `https://acct.r2.cloudflarestorage.com/rook/rook-snapshots/${NAME}`);
  const put = s3Request({ ...t, ...R2 }, "PUT", NAME, TEXT, Date.UTC(2026, 8, 26, 12, 0, 0));
  assert.equal(put.headers["x-amz-date"], "20260926T120000Z");
  assert.equal(put.headers["x-amz-content-sha256"], sha256Hex(TEXT));
  assert.equal(put.headers["content-type"], "application/json");
  assert.match(put.headers.authorization, /^AWS4-HMAC-SHA256 Credential=0123456789abcdef0123456789abcdef\/20260926\/auto\/s3\/aws4_request, SignedHeaders=content-type;host;x-amz-content-sha256;x-amz-date, Signature=[0-9a-f]{64}$/);
  assert.ok(!JSON.stringify(put).includes(R2.secretAccessKey), "the secret is in no header");
  assert.equal(s3Request({ ...t, ...R2 }, "GET", NAME, null, 0).headers["x-amz-content-sha256"], EMPTY_SHA256);
});

type Side = { server: Server; origin: string };
async function serve(handle: (r: StubRequest) => { status: number; body: string; headers?: Record<string, string> }): Promise<Side> {
  const server = createServer((req, res) => {
    const chunks: Buffer[] = [];
    req.on("data", (c: Buffer) => chunks.push(c));
    req.on("end", () => {
      const headers: Record<string, string> = {};
      for (const [k, v] of Object.entries(req.headers)) if (v !== undefined) headers[k] = Array.isArray(v) ? v.join(",") : v;
      const a = handle({ method: req.method ?? "GET", url: `http://${req.headers.host}${req.url}`, headers, body: Buffer.concat(chunks).toString("utf8") });
      res.writeHead(a.status, a.headers ?? {}); res.end(a.body);
    });
  });
  await new Promise<void>((ok) => server.listen(0, "127.0.0.1", ok));
  const addr = server.address() as { port: number };
  return { server, origin: `http://127.0.0.1:${addr.port}` };
}

test("publish to a stub R2 bucket over HTTP: a signed PUT of rook-snapshots/<name> (application/json, no ACL), read back signed and from the public URL, byte for byte", async () => {
  const stub = new S3Stub({ bucket: "rook", ...R2 });
  const api = await serve((r) => stub.api(r)), pub = await serve((r) => stub.public(r));
  try {
    const s3: S3Target = { endpoint: api.origin, bucket: "rook", prefix: "rook-snapshots/", ...R2 };
    const dir = mkdtempSync(join(tmpdir(), "keeper-s3-"));
    const p = await publish({ dir, baseUrl: `${pub.origin}/rook-snapshots`, uploadUrl: null, uploadAuth: null, uploadReadback: true, s3 }, NAME, TEXT, fetch);
    assert.equal(p.uri, `${pub.origin}/rook-snapshots/${NAME}`, "the draw's URI is the public URL");
    assert.equal(p.mirrored, true);
    assert.deepEqual(p.readBack, [new URL(pub.origin).host, new URL(api.origin).host], "the public URI and the bucket's own object were both read back");
    assert.equal(readFileSync(p.path, "utf8"), TEXT, "and kept on the volume");
    assert.deepEqual([...stub.objects.keys()], [`rook-snapshots/${NAME}`], "the object key");
    assert.equal(stub.objects.get(`rook-snapshots/${NAME}`)!.body, TEXT);
    assert.equal(stub.objects.get(`rook-snapshots/${NAME}`)!.contentType, "application/json");
    const put = stub.log.find((r) => r.method === "PUT")!;
    assert.equal(new URL(put.url).pathname, `/rook/rook-snapshots/${NAME}`);
    assert.equal(put.headers["x-amz-acl"], undefined, "no ACL: the bucket is never made publicly writable");
    assert.ok(stub.log.some((r) => r.method === "GET"), "the object read back through the API, signed");
    assert.ok(!JSON.stringify(stub.log).includes(R2.secretAccessKey), "the secret never left the keeper");
  } finally { api.server.close(); pub.server.close(); }
});

test("a wrong secret is refused by the bucket (SignatureDoesNotMatch) and the error names the key and the code, never the secret", async () => {
  const stub = new S3Stub({ bucket: "rook", ...R2 });
  const api = await serve((r) => stub.api(r)), pub = await serve((r) => stub.public(r));
  try {
    const wrong = "0".repeat(20) + "wrong-secret-wrong-secret";
    const s3: S3Target = { endpoint: api.origin, bucket: "rook", prefix: "rook-snapshots/", ...R2, secretAccessKey: wrong };
    const dir = mkdtempSync(join(tmpdir(), "keeper-s3-"));
    await assert.rejects(publish({ dir, baseUrl: `${pub.origin}/rook-snapshots`, uploadUrl: null, uploadAuth: null, s3 }, NAME, TEXT, fetch), (e: Error) => {
      assert.match(e.message, new RegExp(`refused the upload of rook-snapshots/${NAME}: HTTP 403 SignatureDoesNotMatch`));
      assert.ok(!e.message.includes(wrong) && !e.message.includes(R2.secretAccessKey));
      return true;
    });
    assert.equal(stub.objects.size, 0);
    // a bucket name the credentials do not own
    const other = { ...s3, secretAccessKey: R2.secretAccessKey, bucket: "not-rook" };
    await assert.rejects(publish({ dir, baseUrl: `${pub.origin}/rook-snapshots`, uploadUrl: null, uploadAuth: null, s3: other }, NAME, TEXT, fetch), /HTTP 404 NoSuchBucket/);
  } finally { api.server.close(); pub.server.close(); }
});

test("the read-back: a public URL that serves other bytes, or nothing (a wrong base or prefix), stops the draw", async () => {
  const stub = new S3Stub({ bucket: "rook", ...R2 });
  const api = await serve((r) => stub.api(r)), pub = await serve((r) => stub.public(r));
  try {
    const s3: S3Target = { endpoint: api.origin, bucket: "rook", prefix: "rook-snapshots/", ...R2 };
    const dir = mkdtempSync(join(tmpdir(), "keeper-s3-"));
    stub.publicLie = true;
    await assert.rejects(publish({ dir, baseUrl: `${pub.origin}/rook-snapshots`, uploadUrl: null, uploadAuth: null, s3 }, NAME, TEXT, fetch), /serves other bytes/);
    stub.publicLie = false;
    await assert.rejects(publish({ dir, baseUrl: `${pub.origin}/wrong-prefix`, uploadUrl: null, uploadAuth: null, s3 }, NAME, TEXT, fetch), /not served at 127\.0\.0\.1:\d+ yet/);
    // KEEPER_SNAPSHOT_UPLOAD_READBACK=off: only the public URI is read
    const before = stub.log.filter((r) => r.method === "GET").length;
    await publish({ dir, baseUrl: `${pub.origin}/rook-snapshots`, uploadUrl: null, uploadAuth: null, uploadReadback: false, s3 }, NAME, TEXT, fetch);
    assert.equal(stub.log.filter((r) => r.method === "GET").length, before);
  } finally { api.server.close(); pub.server.close(); }
});

test("the stub fetch (the full pass's double) routes the API and the public side and checks the signature the same way", async () => {
  const stub = new S3Stub({ bucket: "rook", ...R2 });
  const f = stubFetch(stub, "https://acct.r2.cloudflarestorage.com", "https://pub-0.r2.dev", (async () => { throw new Error("elsewhere"); }) as typeof fetch);
  const s3: S3Target = { endpoint: "https://acct.r2.cloudflarestorage.com", bucket: "rook", prefix: "rook-snapshots/", ...R2 };
  const p = await publish({ dir: mkdtempSync(join(tmpdir(), "keeper-s3-")), baseUrl: "https://pub-0.r2.dev/rook-snapshots", uploadUrl: null, uploadAuth: null, s3 }, NAME, TEXT, f);
  assert.deepEqual(p.readBack, ["pub-0.r2.dev", "acct.r2.cloudflarestorage.com"]);
});

// ---- the configuration ----------------------------------------------------------------------------------------------------------

const BASE_ENV = { KEEPER_CHAIN_ID: "4663", KEEPER_STACK: "/x/stack.json", KEEPER_PUBLIC_URL: "https://keeper.up.railway.app" };
const R2_ENV = { KEEPER_SNAPSHOT_S3_ENDPOINT: "https://0123abcd.r2.cloudflarestorage.com", KEEPER_SNAPSHOT_S3_BUCKET: "rook-media",
  KEEPER_SNAPSHOT_S3_ACCESS_KEY_ID: R2.accessKeyId, KEEPER_SNAPSHOT_S3_SECRET_ACCESS_KEY: R2.secretAccessKey,
  KEEPER_SNAPSHOT_BASE_URL: "https://pub-736187aaa62744868846472f72e8d77b.r2.dev/rook-snapshots" };
const problems = (env: Record<string, string>): string => { try { parseServiceConfig([], env); return ""; } catch (e) { return (e as ConfigError).problems.join("\n"); } };

test("config: the operator's R2 bucket parses (prefix rook-snapshots/, region auto); the draw's URI names the r2.dev URL; a mirror is configured", () => {
  const cfg = parseServiceConfig([], { ...BASE_ENV, ...R2_ENV });
  assert.deepEqual(cfg.snapshots.s3, { endpoint: "https://0123abcd.r2.cloudflarestorage.com", bucket: "rook-media", prefix: "rook-snapshots/", accessKeyId: R2.accessKeyId,
    secretAccessKey: R2.secretAccessKey, region: "auto" });
  assert.equal(cfg.snapshots.baseUrl, "https://pub-736187aaa62744868846472f72e8d77b.r2.dev/rook-snapshots");
  assert.equal(cfg.snapshots.uploadUrl, null);
  assert.equal(hasMirror(cfg.snapshots), true);
  assert.equal(mirrorRequired({ mode: "send", chainId: 4663, snapshots: cfg.snapshots }), false, "a draw may be announced on 4663");
  assert.equal(parseServiceConfig([], { ...BASE_ENV, ...R2_ENV, KEEPER_SNAPSHOT_S3_PREFIX: "/snaps/v2" }).snapshots.s3!.prefix, "snaps/v2/");
  assert.equal(parseServiceConfig([], { ...BASE_ENV, ...R2_ENV, KEEPER_SNAPSHOT_S3_PREFIX: "" }).snapshots.s3!.prefix, "");
  // the planned custom domain changes only the base
  assert.equal(parseServiceConfig([], { ...BASE_ENV, ...R2_ENV, KEEPER_SNAPSHOT_BASE_URL: "https://snapshots.rook.fun" }).snapshots.baseUrl, "https://snapshots.rook.fun");
});

test("config: every S3 mistake is refused at start, and the secret is in none of the reasons", () => {
  const { KEEPER_SNAPSHOT_S3_BUCKET: _b, ...noBucket } = R2_ENV;
  assert.match(problems({ ...BASE_ENV, ...noBucket }), /without KEEPER_SNAPSHOT_S3_BUCKET/);
  const { KEEPER_SNAPSHOT_S3_SECRET_ACCESS_KEY: _s, ...noSecret } = R2_ENV;
  assert.match(problems({ ...BASE_ENV, ...noSecret }), /without KEEPER_SNAPSHOT_S3_SECRET_ACCESS_KEY/);
  const { KEEPER_SNAPSHOT_S3_ACCESS_KEY_ID: _k, ...noKey } = R2_ENV;
  assert.match(problems({ ...BASE_ENV, ...noKey }), /without KEEPER_SNAPSHOT_S3_ACCESS_KEY_ID/);
  const { KEEPER_SNAPSHOT_BASE_URL: _u, ...noBase } = R2_ENV;
  assert.match(problems({ ...BASE_ENV, ...noBase }), /KEEPER_SNAPSHOT_S3_ENDPOINT is set on 4663 without KEEPER_SNAPSHOT_BASE_URL/);
  assert.match(problems({ ...BASE_ENV, ...R2_ENV, KEEPER_SNAPSHOT_BASE_URL: "https://keeper.up.railway.app/snapshots" }), /on KEEPER_PUBLIC_URL's host/);
  assert.match(problems({ ...BASE_ENV, ...R2_ENV, KEEPER_SNAPSHOT_BASE_URL: "https://0123abcd.r2.cloudflarestorage.com/rook-media/rook-snapshots" }), /the S3 API's host/);
  assert.match(problems({ ...BASE_ENV, ...R2_ENV, KEEPER_SNAPSHOT_UPLOAD_URL: "https://mirror.example/put" }), /both set: the snapshot has one mirror/);
  assert.match(problems({ ...BASE_ENV, ...R2_ENV, KEEPER_SNAPSHOT_S3_ENDPOINT: "http://0123abcd.r2.cloudflarestorage.com" }), /S3_ENDPOINT must be https/);
  assert.match(problems({ ...BASE_ENV, ...R2_ENV, KEEPER_SNAPSHOT_S3_ENDPOINT: "https://0123abcd.r2.cloudflarestorage.com/rook-media" }), /origin only/);
  assert.match(problems({ ...BASE_ENV, ...R2_ENV, KEEPER_SNAPSHOT_S3_BUCKET: "Rook_Media" }), /not a bucket name/);
  assert.match(problems({ ...BASE_ENV, ...R2_ENV, KEEPER_SNAPSHOT_S3_PREFIX: "../up" }), /not a key prefix/);
  assert.match(problems({ ...BASE_ENV, ...R2_ENV, KEEPER_SNAPSHOT_S3_SECRET_ACCESS_KEY: "short" }), /does not look like a secret access key/);
  assert.match(problems({ ...BASE_ENV, KEEPER_SNAPSHOT_S3_BUCKET: "rook-media", KEEPER_SNAPSHOT_S3_SECRET_ACCESS_KEY: R2.secretAccessKey }),
    /KEEPER_SNAPSHOT_S3_BUCKET is set without KEEPER_SNAPSHOT_S3_ENDPOINT[\s\S]*SECRET_ACCESS_KEY is set without KEEPER_SNAPSHOT_S3_ENDPOINT/);
  for (const env of [{ ...BASE_ENV, ...R2_ENV, KEEPER_SNAPSHOT_S3_BUCKET: "Rook_Media" }, { ...BASE_ENV, ...noBase }])
    assert.ok(!problems(env).includes(R2.secretAccessKey));
  // the plain-PUT mirror is unchanged
  assert.equal(problems({ ...BASE_ENV, KEEPER_SNAPSHOT_UPLOAD_URL: "https://mirror.example/put", KEEPER_SNAPSHOT_BASE_URL: "https://cdn.example" }), "");
});

test("the secret access key is one of the secrets every log line is scrubbed of", () => {
  assert.ok(serviceSecrets({ ...R2_ENV })().includes(R2.secretAccessKey));
});

test("the r2.dev public URL passes the outbound guard (https, a public name, Cloudflare's public addresses)", async () => {
  const cloudflare = async () => [{ address: "104.18.54.45", family: 4 }, { address: "2606:4700:311b::6812:3222", family: 6 }];
  assert.equal(await urlRefusal(`https://pub-736187aaa62744868846472f72e8d77b.r2.dev/rook-snapshots/${NAME}`, cloudflare), null);
  assert.equal(await urlRefusal(`https://snapshots.rook.fun/${NAME}`, cloudflare), null);
});
