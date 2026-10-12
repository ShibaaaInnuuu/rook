import { createHash, createHmac } from "node:crypto";

/**
 * A stub S3-compatible bucket (Cloudflare R2's API as the keeper uses it): path-style `/<bucket>/<key>`, every request's AWS
 * Signature V4 checked from the request as received, written here independently of service/s3.ts (its canonical request is
 * rebuilt from the method, the raw path, the query, the signed headers and x-amz-content-sha256; the stub itself is checked against
 * AWS's published example in snapshot-s3.test.ts). Objects are served publicly at `/<key>` on the public side (R2's r2.dev URL).
 */
export type StubRequest = { method: string; url: string; headers: Record<string, string>; body: string };
export type StubAnswer = { status: number; body: string; headers?: Record<string, string> };

const sha = (b: string) => createHash("sha256").update(b, "utf8").digest("hex");
const mac = (k: string | Buffer, d: string) => createHmac("sha256", k).update(d, "utf8").digest();
const enc = (s: string) => encodeURIComponent(s).replace(/[!'()*]/g, (c) => `%${c.charCodeAt(0).toString(16).toUpperCase()}`);
const xmlError = (code: string) => `<?xml version="1.0" encoding="UTF-8"?><Error><Code>${code}</Code><Message>${code}</Message></Error>`;

/** The signature `secret` gives the request, or why the request is malformed. Pure. */
export function expectedSignature(r: StubRequest, secret: string, region: string): { signature: string; given: string; key: string } | { error: string } {
  const h = Object.fromEntries(Object.entries(r.headers).map(([k, v]) => [k.toLowerCase(), v]));
  const m = /^AWS4-HMAC-SHA256 Credential=([^/]+)\/(\d{8})\/([^/]+)\/s3\/aws4_request, ?SignedHeaders=([a-z0-9;-]+), ?Signature=([0-9a-f]{64})$/.exec(h.authorization ?? "");
  if (!m) return { error: "AuthorizationHeaderMalformed" };
  const [, keyId, day, reg, signed, given] = m;
  if (reg !== region) return { error: "AuthorizationHeaderMalformed" };
  const names = signed.split(";");
  if (!names.includes("host") || !names.includes("x-amz-date") || !names.includes("x-amz-content-sha256")) return { error: "AccessDenied" };
  if ((h["x-amz-date"] ?? "").slice(0, 8) !== day) return { error: "AuthorizationHeaderMalformed" };
  const u = new URL(r.url);
  const query = [...u.searchParams].map(([k, v]) => `${enc(k)}=${enc(v)}`).sort().join("&");
  const canon = [r.method, u.pathname, query, names.map((n) => `${n}:${(n === "host" ? u.host : h[n] ?? "").trim().replace(/\s+/g, " ")}\n`).join(""), signed,
    h["x-amz-content-sha256"]].join("\n");
  const sts = ["AWS4-HMAC-SHA256", h["x-amz-date"], `${day}/${reg}/s3/aws4_request`, sha(canon)].join("\n");
  const k = mac(mac(mac(mac(`AWS4${secret}`, day), reg), "s3"), "aws4_request");
  return { signature: createHmac("sha256", k).update(sts, "utf8").digest("hex"), given, key: keyId };
}

export class S3Stub {
  objects = new Map<string, { body: string; contentType: string | null }>();
  log: StubRequest[] = [];
  /** a public GET answers other bytes than were stored (a stale cache, a wrong bucket) */
  publicLie = false;
  constructor(readonly o: { bucket: string; accessKeyId: string; secretAccessKey: string; region?: string }) {}

  /** The S3 API: signed PUT and GET of `/<bucket>/<key>`. */
  api(r: StubRequest): StubAnswer {
    this.log.push(r);
    const h = Object.fromEntries(Object.entries(r.headers).map(([k, v]) => [k.toLowerCase(), v]));
    const sig = expectedSignature(r, this.o.secretAccessKey, this.o.region ?? "auto");
    if ("error" in sig) return { status: 403, body: xmlError(sig.error) };
    if (sig.key !== this.o.accessKeyId) return { status: 403, body: xmlError("InvalidAccessKeyId") };
    if (sig.signature !== sig.given) return { status: 403, body: xmlError("SignatureDoesNotMatch") };
    if (h["x-amz-content-sha256"] !== sha(r.method === "PUT" ? r.body : "")) return { status: 400, body: xmlError("XAmzContentSHA256Mismatch") };
    if (h["x-amz-acl"]) return { status: 403, body: xmlError("AccessDenied") };        // R2: no public-write ACLs
    const path = new URL(r.url).pathname;
    const slash = path.indexOf("/", 1);
    const bucket = decodeURIComponent(path.slice(1, slash < 0 ? undefined : slash)), key = slash < 0 ? "" : decodeURIComponent(path.slice(slash + 1));
    if (bucket !== this.o.bucket) return { status: 404, body: xmlError("NoSuchBucket") };
    if (r.method === "PUT") { this.objects.set(key, { body: r.body, contentType: h["content-type"] ?? null }); return { status: 200, body: "", headers: { etag: `"${sha(r.body).slice(0, 32)}"` } }; }
    if (r.method === "GET") {
      const o = this.objects.get(key);
      return o ? { status: 200, body: o.body, headers: { "content-type": o.contentType ?? "application/octet-stream" } } : { status: 404, body: xmlError("NoSuchKey") };
    }
    return { status: 405, body: xmlError("MethodNotAllowed") };
  }

  /** The bucket's public URL (r2.dev): GET `/<key>`, no credential; nothing else. */
  public(r: StubRequest): StubAnswer {
    if (r.method !== "GET") return { status: 405, body: "" };
    const o = this.objects.get(decodeURIComponent(new URL(r.url).pathname.slice(1)));
    if (!o) return { status: 404, body: "" };
    return { status: 200, body: this.publicLie ? o.body.replace("}", ', "x": 1}') : o.body, headers: { "content-type": o.contentType ?? "application/octet-stream" } };
  }
}

/** A fetch that answers `api` origin from the stub's API and `pub` origin from its public side, and hands anything else to `next`. */
export function stubFetch(stub: S3Stub, api: string, pub: string, next: typeof fetch): typeof fetch {
  return (async (input: string | URL | Request, init: RequestInit = {}) => {
    const url = typeof input === "string" ? input : input instanceof URL ? input.href : input.url;
    const side = url.startsWith(`${api}/`) ? "api" : url.startsWith(`${pub}/`) ? "public" : null;
    if (!side) return next(input as string, init);
    const headers: Record<string, string> = {};
    new Headers(init.headers).forEach((v, k) => { headers[k] = v; });
    const req = { method: (init.method ?? "GET").toUpperCase(), url, headers, body: typeof init.body === "string" ? init.body : "" };
    const a = side === "api" ? stub.api(req) : stub.public(req);
    return new Response(a.body || null, { status: a.status, headers: a.headers });
  }) as typeof fetch;
}
