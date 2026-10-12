import { createHash, createHmac } from "node:crypto";

/**
 * A snapshot mirror on an S3-compatible bucket (Cloudflare R2, or any S3 API): the object is PUT and read back with AWS Signature
 * Version 4 in the Authorization header (docs.aws.amazon.com, "Signature Calculations for the Authorization Header: Transferring
 * Payload in a Single Chunk"), path-style (`<endpoint>/<bucket>/<key>`), the payload's SHA-256 signed. No SDK: the signer is the few
 * lines below, checked against AWS's published examples (keeper/test/snapshot-s3.test.ts).
 *
 * The secret access key is used only as HMAC key material here; it is never put in a header, a URL, an error or a log line. The
 * object is written with no ACL header: the bucket's own public read (R2's public dev URL or custom domain) is how the draw's URI
 * is served, and nothing here can make the bucket publicly writable.
 */
export type S3Target = { endpoint: string; bucket: string; prefix: string; accessKeyId: string; secretAccessKey: string; region: string };

export const EMPTY_SHA256 = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
export const sha256Hex = (b: string | Uint8Array): string => createHash("sha256").update(b).digest("hex");
const hmac = (key: string | Buffer, data: string): Buffer => createHmac("sha256", key).update(data, "utf8").digest();

/** RFC 3986 percent-encoding as SigV4 wants it: every byte but A-Z a-z 0-9 - . _ ~ (and `/` in a path when `keepSlash`). */
export function uriEncode(s: string, keepSlash: boolean): string {
  let out = "";
  for (const b of Buffer.from(s, "utf8")) {
    const c = String.fromCharCode(b);
    if (/[A-Za-z0-9\-._~]/.test(c) || (keepSlash && c === "/")) out += c;
    else out += `%${b.toString(16).toUpperCase().padStart(2, "0")}`;
  }
  return out;
}

/** `YYYYMMDDTHHMMSSZ` */
export const amzDateOf = (ms: number): string => new Date(ms).toISOString().replace(/[-:]/g, "").replace(/\.\d{3}Z$/, "Z");

export type SignInput = {
  method: string;
  /** the request's URL; its path is taken as already percent-encoded where it must be (S3 keys are encoded once, by `objectUrl`) */
  url: string;
  /** headers to sign besides `host` (added from the URL); names any case */
  headers: Record<string, string>;
  payloadSha256: string;
  amzDate: string;
  region: string;
  service: string;
  accessKeyId: string;
  secretAccessKey: string;
};
export type Signed = { authorization: string; signature: string; signedHeaders: string; canonicalRequest: string; stringToSign: string };

/** SigV4 of one request. Pure. */
export function signV4(r: SignInput): Signed {
  const u = new URL(r.url);
  const headers = new Map<string, string>();
  headers.set("host", u.host);
  for (const [k, v] of Object.entries(r.headers)) headers.set(k.toLowerCase(), String(v).trim().replace(/\s+/g, " "));
  const names = [...headers.keys()].sort();
  const signedHeaders = names.join(";");
  const query = [...u.searchParams].map(([k, v]) => [uriEncode(k, false), uriEncode(v, false)])
    .sort((a, b) => (a[0] === b[0] ? (a[1] < b[1] ? -1 : a[1] > b[1] ? 1 : 0) : a[0] < b[0] ? -1 : 1)).map(([k, v]) => `${k}=${v}`).join("&");
  const canonicalRequest = [r.method.toUpperCase(), u.pathname || "/", query, names.map((n) => `${n}:${headers.get(n)}\n`).join(""), signedHeaders, r.payloadSha256].join("\n");
  const day = r.amzDate.slice(0, 8);
  const scope = `${day}/${r.region}/${r.service}/aws4_request`;
  const stringToSign = ["AWS4-HMAC-SHA256", r.amzDate, scope, sha256Hex(canonicalRequest)].join("\n");
  const key = hmac(hmac(hmac(hmac(`AWS4${r.secretAccessKey}`, day), r.region), r.service), "aws4_request");
  const signature = createHmac("sha256", key).update(stringToSign, "utf8").digest("hex");
  return { authorization: `AWS4-HMAC-SHA256 Credential=${r.accessKeyId}/${scope}, SignedHeaders=${signedHeaders}, Signature=${signature}`,
    signature, signedHeaders, canonicalRequest, stringToSign };
}

/** The object's key: the prefix (`rook-snapshots/` by default) and the file's name. */
export const objectKey = (t: Pick<S3Target, "prefix">, name: string): string => `${t.prefix}${name}`;

/** The object's path-style URL at the S3 API: `<endpoint>/<bucket>/<key>`, the key percent-encoded once, its slashes kept. */
export const objectUrl = (t: Pick<S3Target, "endpoint" | "bucket" | "prefix">, name: string): string =>
  `${t.endpoint.replace(/\/$/, "")}/${uriEncode(t.bucket, false)}/${uriEncode(objectKey(t, name), true)}`;

/** The headers of a signed PUT (with `body`) or GET (without) of the object `name`, at `nowMs`. */
export function s3Request(t: S3Target, method: "PUT" | "GET", name: string, body: string | null, nowMs: number): { url: string; headers: Record<string, string> } {
  const url = objectUrl(t, name);
  const payloadSha256 = body === null ? EMPTY_SHA256 : sha256Hex(body);
  const amzDate = amzDateOf(nowMs);
  const headers: Record<string, string> = { "x-amz-content-sha256": payloadSha256, "x-amz-date": amzDate };
  if (body !== null) headers["content-type"] = "application/json";
  const s = signV4({ method, url, headers, payloadSha256, amzDate, region: t.region, service: "s3", accessKeyId: t.accessKeyId, secretAccessKey: t.secretAccessKey });
  return { url, headers: { ...headers, authorization: s.authorization } };
}

/** An S3 error body's `<Code>` (e.g. SignatureDoesNotMatch, NoSuchBucket, AccessDenied), letters only, for an error message. */
export const s3ErrorCode = (xml: string): string | null => /<Code>([A-Za-z]{1,64})<\/Code>/.exec(xml)?.[1] ?? null;
