/**
 * A coin's off-chain metadata JSON (its Metaplex uri): image, description, links. The uri is written by whoever launched the coin,
 * so the fetch is fenced: https (or ipfs:// / ar:// through public gateways) only, no IP literals, no localhost / *.internal /
 * *.local hosts, 5 s, 64 KB, JSON only. What it returns is shown as links and an <img> URL by the site, never fetched again here.
 */
import type { Fetch } from "./rpc.js";

export type OffChain = { image: string | null; description: string | null; website: string | null; twitter: string | null; telegram: string | null };

export function gatewayUrl(uri: string): string | null {
  const u = uri.trim();
  if (/^https:\/\//i.test(u)) return u;
  if (/^ipfs:\/\//i.test(u)) return `https://ipfs.io/ipfs/${u.replace(/^ipfs:\/\/(ipfs\/)?/i, "")}`;
  if (/^ar:\/\//i.test(u)) return `https://arweave.net/${u.slice(5)}`;
  return null;
}
/** https, a DNS name, not a private-looking one. */
export function fetchable(url: string): boolean {
  let u: URL;
  try { u = new URL(url); } catch { return false; }
  if (u.protocol !== "https:" || u.username || u.password) return false;
  const h = u.hostname.toLowerCase();
  if (!h.includes(".") || /^[\d.]+$/.test(h) || h.includes(":") || h.startsWith("[")) return false;
  if (h === "localhost" || /\.(internal|local|localhost|lan|home|corp)$/.test(h)) return false;
  return true;
}
const link = (v: unknown) => { if (typeof v !== "string") return null; const g = gatewayUrl(v); return g && g.length <= 500 ? g : null; };
const text = (v: unknown, n: number) => (typeof v === "string" && v.trim() ? v.trim().slice(0, n) : null);
export function parseOffChain(j: unknown): OffChain {
  const o = (j && typeof j === "object" ? j : {}) as Record<string, unknown>;
  const ext = (o.extensions && typeof o.extensions === "object" ? o.extensions : {}) as Record<string, unknown>;
  return { image: link(o.image), description: text(o.description, 600), website: link(o.website ?? ext.website ?? o.external_url),
    twitter: text(o.twitter ?? ext.twitter, 200), telegram: text(o.telegram ?? ext.telegram, 200) };
}

/** The uri itself, then for an IPFS path two other public gateways (ipfs.io rate-limits). */
export function candidates(uri: string): string[] {
  const u = gatewayUrl(uri);
  if (!u) return [];
  const m = /\/ipfs\/(.+)$/.exec(u);
  return m ? [u, ...["https://gateway.pinata.cloud/ipfs/", "https://dweb.link/ipfs/"].map((g) => g + m[1]).filter((x) => x !== u)] : [u];
}
export async function fetchOffChain(uri: string, f: Fetch = fetch, maxBytes = 64 * 1024): Promise<OffChain | null> {
  for (const url of candidates(uri)) {
    if (!fetchable(url)) continue;
    try {
      const r = await f(url, { signal: AbortSignal.timeout(5_000), redirect: "follow", headers: { accept: "application/json" } });
      if (!r.ok || !fetchable(r.url || url)) continue;
      if (Number(r.headers.get("content-length") ?? 0) > maxBytes) return null;
      const buf = new Uint8Array(await r.arrayBuffer());
      if (buf.length > maxBytes) return null;
      return parseOffChain(JSON.parse(new TextDecoder().decode(buf)));
    } catch { /* the next gateway */ }
  }
  return null;
}
