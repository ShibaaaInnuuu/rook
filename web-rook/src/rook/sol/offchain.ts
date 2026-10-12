/** A coin's off-chain metadata JSON (its Metaplex uri): image, description, links. Only https / ipfs / ar uris are fetched. */
/** The coin's off-chain metadata JSON (its uri), for the image, description and links. Only https / ipfs / ar uris are fetched. */
export type OffChainMeta = { image: string | null; description: string | null; website: string | null; twitter: string | null; telegram: string | null };
export function gatewayUrl(uri: string): string | null {
  if (/^https:\/\//i.test(uri)) return uri;
  if (/^ipfs:\/\//i.test(uri)) return `https://ipfs.io/ipfs/${uri.replace(/^ipfs:\/\/(ipfs\/)?/i, "")}`;
  if (/^ar:\/\//i.test(uri)) return `https://arweave.net/${uri.slice(5)}`;
  return null;
}
const httpsOrNull = (v: unknown) => (typeof v === "string" && /^https:\/\/|^ipfs:\/\/|^ar:\/\//i.test(v) ? gatewayUrl(v) : null);
const text = (v: unknown, n = 600) => (typeof v === "string" && v.trim() ? v.trim().slice(0, n) : null);
export function parseOffChain(j: unknown): OffChainMeta {
  const o = (j && typeof j === "object" ? j : {}) as Record<string, unknown>;
  const ext = (o.extensions && typeof o.extensions === "object" ? o.extensions : {}) as Record<string, unknown>;
  return {
    image: httpsOrNull(o.image), description: text(o.description),
    website: httpsOrNull(o.website ?? ext.website ?? o.external_url), twitter: text(o.twitter ?? ext.twitter, 200), telegram: text(o.telegram ?? ext.telegram, 200),
  };
}
/** Where to read a uri from: itself, then, for an IPFS path, two other public gateways (ipfs.io rate-limits, and its 429 carries no CORS header). */
export function candidates(uri: string): string[] {
  const u = gatewayUrl(uri);
  if (!u) return [];
  const m = u.match(/\/ipfs\/(.+)$/) ?? (/^https:\/\/(baf[a-z0-9]+|Qm[1-9A-HJ-NP-Za-km-z]{44})\.ipfs\.[^/]+\/?(.*)$/.exec(u) ? [u, u.replace(/^https:\/\/([^.]+)\.ipfs\.[^/]+\/?/, "$1/")] : null);
  const alt = m ? ["https://gateway.pinata.cloud/ipfs/", "https://dweb.link/ipfs/"].map((g) => g + m[1]) : [];
  return [u, ...alt.filter((a) => a !== u)];
}
export async function fetchOffChain(uri: string): Promise<OffChainMeta | null> {
  for (const u of candidates(uri)) {
    try {
      const r = await fetch(u, { referrerPolicy: "no-referrer", credentials: "omit" });
      if (!r.ok) continue;
      if (Number(r.headers.get("content-length") ?? 0) > 256 * 1024) return null;
      return parseOffChain(await r.json());
    } catch { /* the next gateway */ }
  }
  return null;
}
