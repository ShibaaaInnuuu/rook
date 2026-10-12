/**
 * Audit E-13 (docs/audit/2026-09-26/fixes-W2-monitor.md §2). The indexer's reserves check (indexer/src/wrapped/verify.ts,
 * unchanged) asks ordinals.com alone where an inscription is. The reconciler hands it this fetch instead: a request for
 * `<ord>/r/inscription/<id>` is answered only when Ordinals Wallet (keyless; the keeper's second ordinals provider,
 * keeper/src/ordapi.ts) names the SAME address and the SAME satpoint. Otherwise the request fails, and the indexer calls the item
 * "unverified" — never green on one provider's word, and never red on one provider's word either ("elsewhere" needs both).
 * Every other request passes through unchanged. GETs only.
 */
export type Fetch = typeof fetch;
export const ORD_BASE = "https://ordinals.com";
export const OW_BASE = "https://turbo.ordinalswallet.com";
const ID = /^[0-9a-f]{64}i\d{1,10}$/;
const host = (u: string) => { try { return new URL(u).host; } catch { return u; } };

export class LocationDisagrees extends Error { constructor(msg: string) { super(msg); this.name = "LocationDisagrees"; } }

export function agreeingOrdFetch(fetchImpl: Fetch, o: { ord?: string; ow?: string; timeoutMs?: number } = {}): Fetch {
  const ord = o.ord ?? ORD_BASE, ow = o.ow ?? OW_BASE, t = o.timeoutMs ?? 10_000;
  return (async (input: string | URL | Request, init?: RequestInit) => {
    const url = typeof input === "string" ? input : input instanceof URL ? input.href : input.url;
    const m = url.startsWith(`${ord}/r/inscription/`) ? url.slice(`${ord}/r/inscription/`.length).split(/[?#]/)[0] : null;
    if (!m || !ID.test(m)) return fetchImpl(input as never, init);
    const [a, b] = await Promise.all([
      fetchImpl(url, { ...init, headers: { accept: "application/json" }, signal: AbortSignal.timeout(t) }),
      fetchImpl(`${ow}/inscription/${m}`, { headers: { accept: "application/json", "user-agent": "rook-reconciler/1 (read-only)" }, signal: AbortSignal.timeout(t) }),
    ]);
    if (!a.ok) throw new Error(`${host(ord)} answered HTTP ${a.status}`);
    if (!b.ok) throw new LocationDisagrees(`the second provider (${host(ow)}) answered HTTP ${b.status}`);
    const [ja, jb] = [await a.json(), await b.json()] as { id?: unknown; address?: unknown; satpoint?: unknown }[];
    if (jb.id !== undefined && jb.id !== m) throw new LocationDisagrees(`${host(ow)} answered about another inscription`);
    if (typeof ja.address !== "string" || typeof ja.satpoint !== "string") throw new Error(`${host(ord)}: no address or satpoint`);
    if (ja.address !== jb.address || ja.satpoint !== jb.satpoint)
      throw new LocationDisagrees(`the ordinals providers disagree: ${host(ord)} says ${ja.address} at ${ja.satpoint}, ${host(ow)} says ${String(jb.address)} at ${String(jb.satpoint)}`);
    return new Response(JSON.stringify(ja), { status: 200, headers: { "content-type": "application/json" } });
  }) as Fetch;
}
