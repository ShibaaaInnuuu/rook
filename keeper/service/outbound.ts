import { lookup as dnsLookup, type LookupAddress } from "node:dns";
import { isIP } from "node:net";

/**
 * Audit R2-S6. The keeper's and the resolver's GET of a URL someone else chose (a draw's on-chain `uri`, the keeper's word): the
 * indexer's address rules (indexer/src/outbound.ts, E-12), ported so the keeper image does not ship the indexer. Every address the
 * host resolves to must be public (no loopback, private, CGNAT, link-local metadata, ULA…), and the body is streamed with a byte cap.
 * Unlike the indexer's transport the connect is not pinned to the checked address (the window is one DNS TTL); the answer is used
 * only after its proof checks against the on-chain root, so a rebinding can at worst make the GET fail.
 */
export type Resolver = (host: string) => Promise<{ address: string; family: number }[]>;

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

export const systemResolver: Resolver = (host) => new Promise((ok, no) => dnsLookup(host, { all: true, verbatim: true }, (e, a) => (e ? no(e) : ok(a as LookupAddress[]))));

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

/**
 * The body as text, read in chunks and abandoned the moment it passes `max` bytes (a `content-length` over it is refused before
 * reading). Null when too large. A response without a stream (a test double) is read with `text()` and still held to the cap.
 */
export async function textCapped(r: { headers?: Headers; body?: ReadableStream<Uint8Array> | null; text: () => Promise<string> }, max: number): Promise<string | null> {
  const said = Number(r.headers?.get?.("content-length") ?? NaN);
  if (Number.isFinite(said) && said > max) { await r.body?.cancel().catch(() => undefined); return null; }
  if (!r.body || typeof r.body.getReader !== "function") { const t = await r.text(); return Buffer.byteLength(t) > max ? null : t; }
  const reader = r.body.getReader();
  const parts: Uint8Array[] = [];
  let n = 0;
  for (;;) {
    const { done, value } = await reader.read();
    if (done) break;
    n += value.byteLength;
    if (n > max) { await reader.cancel().catch(() => undefined); return null; }
    parts.push(value);
  }
  return Buffer.concat(parts).toString("utf8");
}
