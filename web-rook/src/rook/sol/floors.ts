/**
 * Collection floors for the Launch wizard's cards, from the collections catalogue's recorded set (public/fixtures/catalog, the same
 * files the "tracked collections" page reads). They are dated records, not live prices: the card says "floor ~X" and its title the date.
 */
import type { SolCollection } from "./collections";

/** Where a Solana collection's slug differs from the catalogue's. */
const CATALOG_SLUG: Record<string, string> = { mayc: "mutant-ape-yacht-club", bayc: "boredapeyachtclub", doodles: "doodles-official" };
type Floor = { native: string; unit: "wei" | "sats"; eth: number | null; at: number };
export type FloorView = { text: string; at: number };

export function floorView(f: Floor | null | undefined): FloorView | null {
  if (!f) return null;
  if (f.unit === "wei") {
    const eth = f.eth ?? Number(BigInt(f.native) / 10n ** 12n) / 1e6;
    return { text: `${eth >= 1 ? eth.toLocaleString("en-US", { maximumFractionDigits: 2 }) : eth.toLocaleString("en-US", { maximumFractionDigits: 3 })} ETH`, at: f.at };
  }
  const sats = Number(f.native);
  return { text: sats >= 1e7 ? `${(sats / 1e8).toLocaleString("en-US", { maximumFractionDigits: 3 })} BTC` : `${sats.toLocaleString("en-US")} sats`, at: f.at };
}

/** slug → floor, for the given collections; a collection the catalogue lacks is simply absent. */
export async function loadFloors(cols: readonly SolCollection[], read: (path: string) => Promise<unknown> = async (p) => { const r = await fetch(p); if (!r.ok) throw new Error(String(r.status)); return r.json(); }): Promise<Record<string, FloorView>> {
  const index = (await read("/fixtures/catalog/index.json")) as { files: Record<string, string> };
  const out: Record<string, FloorView> = {};
  await Promise.all(cols.map(async (c) => {
    const file = index.files[`/v1/rook/catalog/${CATALOG_SLUG[c.slug] ?? c.slug}`];
    if (!file) return;
    try {
      const j = (await read(`/fixtures/catalog/${file}`)) as { entry?: { floor?: Floor | null } };
      const v = floorView(j.entry?.floor);
      if (v) out[c.slug] = v;
    } catch { /* no floor for this one */ }
  }));
  return out;
}
