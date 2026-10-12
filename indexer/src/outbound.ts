import { lookup as dnsLookup, type LookupAddress } from "node:dns";
import { request as httpsRequest } from "node:https";
import { isIP } from "node:net";

/**
 * docs/audit/2026-09-26/E-services-workflow.md E-12, docs/audit/2026-09-26/fixes-W2-site.md §3. Every outbound GET whose URL a
 * collection's creator, a marketplace or a tokenURI can name goes through here. The rules:
 *
 *  - https only; no bare names, `localhost`, `.local`, `.internal`;
 *  - every address the host resolves to must be public (`isPublicIp`), and the production transport checks it AT CONNECT TIME,
 *    inside the socket's own lookup, so the address that was checked is the address connected to (no DNS-rebinding window);
 *  - redirects are never followed by the transport: each `Location` is resolved, checked again, and followed at most `maxRedirects`
 *    times;
 *  - one deadline for the whole request, redirects and body included, that ends it in every state (R3-check M-E: an HTTP 101,
 *    headers that never come, a body that stalls);
 *  - the body is streamed with a byte cap, refused early on a `content-length` over it.
 *
 * It reads; it sends nothing but GETs (and the fixed-host JSON-RPC POSTs a caller passes through, under the same checks).
 */
export type Fetch = typeof fetch;
export type Resolver = (host: string) => Promise<{ address: string; family: number }[]>;

// ---- which addresses are public -------------------------------------------------------------------------------------------------

const v4 = (ip: string): number[] | null => {
  const m = ip.match(/^(\d{1,3})\.(\d{1,3})\.(\d{1,3})\.(\d{1,3})$/);
  if (!m) return null;
  const o = m.slice(1).map(Number);
  return o.every((x) => x <= 255) ? o : null;
};
const publicV4 = ([a, b, c]: number[]): boolean =>
  !(a === 0 || a === 10 || a === 127 || (a === 100 && b >= 64 && b <= 127) || (a === 169 && b === 254) || (a === 172 && b >= 16 && b <= 31)
    || (a === 192 && b === 0 && c === 0) || (a === 192 && b === 0 && c === 2) || (a === 192 && b === 88 && c === 99) || (a === 192 && b === 168)
    || (a === 198 && (b === 18 || b === 19)) || (a === 198 && b === 51 && c === 100) || (a === 203 && b === 0 && c === 113) || a >= 224);

/** An IPv6 address as eight 16-bit groups, or null. Accepts `::` compression and a trailing dotted IPv4. */
const v6 = (ip: string): number[] | null => {
  let s = ip.toLowerCase().replace(/^\[|\]$/g, "");
  const zone = s.indexOf("%"); if (zone >= 0) s = s.slice(0, zone);
  let tail: number[] = [];
  const dotted = s.match(/^(.*:)(\d{1,3}(?:\.\d{1,3}){3})$/);
  if (dotted) { const q = v4(dotted[2]); if (!q) return null; tail = [(q[0] << 8) | q[1], (q[2] << 8) | q[3]]; s = dotted[1].endsWith("::") ? dotted[1] : dotted[1].slice(0, -1); }
  const halves = s.split("::");
  if (halves.length > 2) return null;
  const part = (x: string) => (x === "" ? [] : x.split(":").map((g) => (/^[0-9a-f]{1,4}$/.test(g) ? parseInt(g, 16) : NaN)));
  const head = part(halves[0]), rest = halves.length === 2 ? part(halves[1]) : [];
  if ([...head, ...rest].some((g) => Number.isNaN(g))) return null;
  const n = head.length + rest.length + tail.length;
  if (halves.length === 1 ? n !== 8 : n > 7) return null;
  return [...head, ...new Array(8 - n).fill(0), ...rest, ...tail];
};

/** True only for an address a request may go to: never loopback, private, CGNAT, link-local (cloud metadata), ULA, multicast or reserved. */
export function isPublicIp(ip: string): boolean {
  const q = v4(ip);
  if (q) return publicV4(q);
  const g = v6(ip);
  if (!g) return false;
  const embedded = (hi: number, lo: number) => publicV4([hi >> 8, hi & 255, lo >> 8, lo & 255]);
  if (g.every((x) => x === 0)) return false;                                           // ::
  if (g.slice(0, 7).every((x) => x === 0) && g[7] === 1) return false;                 // ::1
  if (g.slice(0, 5).every((x) => x === 0) && g[5] === 0xffff) return embedded(g[6], g[7]);   // ::ffff:a.b.c.d
  if (g.slice(0, 6).every((x) => x === 0)) return false;                               // ::a.b.c.d (deprecated compatible form)
  if (g[0] === 0x64 && g[1] === 0xff9b) return embedded(g[6], g[7]);                   // 64:ff9b::/96 NAT64
  if (g[0] === 0x2002) return embedded(g[1], g[2]);                                    // 6to4
  if ((g[0] & 0xfe00) === 0xfc00) return false;                                        // fc00::/7 ULA (Railway's fd12::/16)
  if ((g[0] & 0xffc0) === 0xfe80) return false;                                        // fe80::/10 link-local
  if ((g[0] & 0xffc0) === 0xfec0) return false;                                        // fec0::/10 site-local (deprecated)
  if ((g[0] & 0xff00) === 0xff00) return false;                                        // multicast
  if (g[0] === 0x2001 && g[1] === 0x0db8) return false;                                // documentation
  if (g[0] === 0x2001 && g[1] < 0x0200) return false;                                  // 2001::/23 IETF special purpose (Teredo, ORCHID…)
  if (g[0] === 0x0100 && g.slice(1, 4).every((x) => x === 0)) return false;            // 100::/64 discard
  return (g[0] & 0xe000) === 0x2000;                                                   // only global unicast 2000::/3
}

/** The name rules that hold before any lookup: https, a dotted name that is not ours, or an IP literal that is public. */
export function hostRefusal(url: string): string | null {
  let u: URL;
  try { u = new URL(url); } catch { return "not a URL"; }
  if (u.protocol !== "https:") return "not https";
  if (u.username || u.password) return "credentials in the URL";
  const h = u.hostname.toLowerCase().replace(/^\[|\]$/g, "");
  if (isIP(h)) return isPublicIp(h) ? null : "a private or reserved address";
  if (!h.includes(".") || h === "localhost" || h.endsWith(".localhost") || h.endsWith(".local") || h.endsWith(".internal") || h.endsWith(".arpa")) return "not a public host name";
  return null;
}

const systemResolver: Resolver = (host) => new Promise((ok, no) => dnsLookup(host, { all: true, verbatim: true }, (e, a) => (e ? no(e) : ok(a as LookupAddress[]))));

/** The URL's host resolved and every address checked; null when it may be asked, else why not. */
export async function urlRefusal(url: string, resolve: Resolver = systemResolver): Promise<string | null> {
  const bad = hostRefusal(url);
  if (bad) return bad;
  const h = new URL(url).hostname.replace(/^\[|\]$/g, "");
  if (isIP(h)) return null;
  let addrs: { address: string }[];
  try { addrs = await resolve(h); } catch { return "the name does not resolve"; }
  if (!addrs.length) return "the name does not resolve";
  const priv = addrs.find((a) => !isPublicIp(a.address));
  return priv ? `resolves to a private or reserved address (${priv.address})` : null;
}

// ---- reading a body with a cap --------------------------------------------------------------------------------------------------

export class TooLarge extends Error { constructor(readonly limit: number) { super(`the answer is larger than ${limit} bytes`); this.name = "TooLarge"; } }

/** The whole body, read in chunks and abandoned (the stream cancelled) the moment it passes `max` bytes. */
export async function readCapped(r: Response, max: number): Promise<Uint8Array> {
  const said = Number(r.headers.get("content-length") ?? NaN);
  if (Number.isFinite(said) && said > max) { await r.body?.cancel().catch(() => undefined); throw new TooLarge(max); }
  if (!r.body) return new Uint8Array(0);
  const reader = r.body.getReader();
  const parts: Uint8Array[] = [];
  let n = 0;
  for (;;) {
    const { done, value } = await reader.read();
    if (done) break;
    n += value.byteLength;
    if (n > max) { await reader.cancel().catch(() => undefined); throw new TooLarge(max); }
    parts.push(value);
  }
  const out = new Uint8Array(n);
  let at = 0;
  for (const p of parts) { out.set(p, at); at += p.byteLength; }
  return out;
}
export const JSON_MAX_BYTES = 2 * 1024 * 1024;
/** A JSON body under the cap, or a thrown error (`TooLarge`, or the parser's). */
export async function jsonCapped(r: Response, max = JSON_MAX_BYTES): Promise<unknown> {
  return JSON.parse(new TextDecoder().decode(await readCapped(r, max)));
}

// ---- the guarded fetch ----------------------------------------------------------------------------------------------------------

export type GuardOptions = {
  /** the transport; default: node:https with the address checked inside the socket's own lookup */
  fetch?: Fetch;
  /** the resolver the pre-flight check uses; default: the system's */
  resolve?: Resolver;
  maxRedirects?: number;
  /** one deadline for the whole request, redirects included */
  timeoutMs?: number;
  /** the body cap */
  maxBytes?: number;
};
const REDIRECT = new Set([301, 302, 303, 307, 308]);

/**
 * A `fetch` that only goes where the rules above allow. Every hop is checked (resolved, every address public), redirects are
 * followed by hand at most `maxRedirects` times and re-checked, the whole request has one deadline, and the answer's body is
 * read here under the cap, so what the caller gets back is a complete, bounded `Response`. Refusals throw `Refused`.
 */
export class Refused extends Error { constructor(why: string) { super(`refused: ${why}`); this.name = "Refused"; } }
export function guardedFetch(o: GuardOptions = {}): Fetch {
  const transport = o.fetch ?? pinnedHttpsFetch;
  const resolve = o.resolve ?? systemResolver;
  const maxRedirects = o.maxRedirects ?? 3, timeoutMs = o.timeoutMs ?? 15_000, maxBytes = o.maxBytes ?? JSON_MAX_BYTES;
  return (async (input: string | URL | Request, init: RequestInit = {}) => {
    let url = typeof input === "string" ? input : input instanceof URL ? input.href : input.url;
    const deadline = AbortSignal.timeout(timeoutMs);
    const signal = init.signal ? AbortSignal.any([init.signal, deadline]) : deadline;
    // R3-check M-E: the deadline ends the fetch in every state, whatever the transport does (a 101 that never settles, headers
    // that never come): the whole request, redirects and body included, races it, and the transport's socket is aborted by it
    let onAbort: (() => void) | null = null;
    const timedOut = new Promise<never>((_ok, no) => {
      onAbort = () => no(new Refused(`timeout: no complete answer within ${timeoutMs} ms from ${safeUrl(url)}`));
      if (signal.aborted) onAbort(); else signal.addEventListener("abort", onAbort, { once: true });
    });
    const run = async (): Promise<Response> => {
      let method = (init.method ?? "GET").toUpperCase(), body = init.body;
      for (let hop = 0; ; hop++) {
        const why = await urlRefusal(url, resolve);
        if (why) throw new Refused(`${why}: ${safeUrl(url)}`);
        const r = await transport(url, { ...init, method, body, redirect: "manual", signal });
        if (REDIRECT.has(r.status)) {
          await r.body?.cancel().catch(() => undefined);
          const loc = r.headers.get("location");
          if (!loc) throw new Refused(`a redirect with no location from ${safeUrl(url)}`);
          if (hop >= maxRedirects) throw new Refused(`more than ${maxRedirects} redirects`);
          url = new URL(loc, url).href;
          if (r.status === 303 || ((r.status === 301 || r.status === 302) && method === "POST")) { method = "GET"; body = undefined; }
          continue;
        }
        const bytes = await readCapped(r, maxBytes);
        return new Response(bytes.byteLength ? (bytes as unknown as BodyInit) : null, { status: r.status, statusText: r.statusText, headers: r.headers });
      }
    };
    const work = run();
    work.catch(() => undefined);                 // abandoned after the deadline: its late failure is not an unhandled rejection
    try { return await Promise.race([work, timedOut]); }
    finally { if (onAbort) signal.removeEventListener("abort", onAbort); }
  }) as Fetch;
}
/** A URL for an error message: its origin and path, never its query (which could carry a key). */
const safeUrl = (u: string) => { try { const x = new URL(u); return `${x.origin}${x.pathname}`.slice(0, 160); } catch { return "an unparsable URL"; } };

/**
 * The production transport: one https request, no redirects followed, and the socket's lookup refuses any non-public address, so a
 * name that resolved public in the pre-flight check and private at connect time (DNS rebinding) is refused where it matters.
 */
export const pinnedHttpsFetch: Fetch = ((input: string | URL | Request, init: RequestInit = {}) => new Promise<Response>((resolveP, rejectP) => {
  // settles exactly once; every path below that ends the request without an answer rejects (R3-check M-E)
  let settled = false;
  const ok = (r: Response) => { if (!settled) { settled = true; resolveP(r); } };
  const no = (e: Error) => { if (!settled) { settled = true; rejectP(e); } };
  const url = new URL(typeof input === "string" ? input : input instanceof URL ? input.href : input.url);
  if (url.protocol !== "https:") { no(new Refused("not https")); return; }
  const headers: Record<string, string> = {};
  new Headers(init.headers).forEach((v, k) => { headers[k] = v; });
  const req = httpsRequest(url, {
    method: init.method ?? "GET", headers, signal: init.signal ?? undefined,
    lookup: (host, opts, cb) => dnsLookup(host, { ...opts, all: true }, (e, addrs) => {
      if (e) { (cb as (e: Error) => void)(e); return; }
      const list = addrs as unknown as LookupAddress[];
      const bad = list.find((a) => !isPublicIp(a.address));
      if (!list.length || bad) { (cb as (e: Error) => void)(new Refused(`resolves to a private or reserved address (${bad?.address ?? "none"})`)); return; }
      if ((opts as { all?: boolean }).all) (cb as (e: null, a: LookupAddress[]) => void)(null, list);
      else (cb as (e: null, a: string, f: number) => void)(null, list[0].address, list[0].family);
    }),
  }, (res) => {
    // R2 N-1: a status outside what a fetch Response may carry (Node's parser accepts 100-999; undici's Response throws a
    // RangeError outside 200-599) is a refusal of THIS fetch, never an exception thrown out of the socket's listener, which
    // would be uncaught and kill the process. Anything unexpected in building the Response rejects the fetch the same way.
    try {
      const status = res.statusCode ?? 0;
      if (!Number.isInteger(status) || status < 200 || status > 599) { res.destroy(); no(new Refused(`a non-standard HTTP status ${status} from ${url.origin}`)); return; }
      const h = new Headers();
      for (const [k, v] of Object.entries(res.headers)) if (v !== undefined) for (const x of Array.isArray(v) ? v : [v]) { try { h.append(k, String(x)); } catch { /* a header undici refuses is dropped */ } }
      const stream = new ReadableStream<Uint8Array>({
        start(c) {
          res.on("data", (d: Buffer) => { try { c.enqueue(new Uint8Array(d)); } catch { /* cancelled */ } });
          res.on("end", () => { try { c.close(); } catch { /* cancelled */ } });
          res.on("error", (e) => { try { c.error(e); } catch { /* cancelled */ } });
          // the socket closed before the body ended (reset, or aborted by the deadline): the body errors, it never hangs
          res.on("close", () => { if (!res.complete) { try { c.error(new Refused(`the answer from ${url.origin} ended before its body did`)); } catch { /* done */ } } });
        },
        cancel() { res.destroy(); },
      });
      ok(new Response(status === 204 || status === 304 ? null : stream, { status, statusText: res.statusMessage, headers: h }));
    } catch (err) {
      res.destroy();
      no(err instanceof Error ? err : new Error(String(err)));
    }
  });
  // R3-check M-E: HTTP 101 Switching Protocols. With no `upgrade` listener Node closes the socket and emits neither `response`
  // nor `error`, so the fetch never settled. A protocol switch is never what a GET of a document means: refuse it.
  req.on("upgrade", (res, socket) => { socket.destroy(); res.destroy(); no(new Refused(`HTTP 101 (a protocol switch) from ${url.origin}`)); });
  // any other way the request ends without an answer (the socket closed before the headers, whatever the reason)
  req.on("close", () => no(new Refused(`the connection to ${url.origin} closed before an answer`)));
  req.on("error", no);
  if (typeof init.body === "string") req.write(init.body);
  req.end();
})) as Fetch;
