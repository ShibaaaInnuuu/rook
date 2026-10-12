/** The raffle hooks: every pinned collection's raffle and books from the canister in one query (src/rook/sol/raffle.ts `loadRaffles`:
 *  `raffle_status` + `books_view` per collection, the canister's index found by wX symbol in `coins_collections`, never the site's
 *  listingId), polled every minute; a round's published snapshot on demand. Under `?demo` the fixtures of src/rook/sol/demoRaffle.ts,
 *  nothing read. */
import { useQuery } from "@tanstack/react-query";
import { canister, type CollectionViewV } from "./canister";
import { COLLECTIONS } from "./collections";
import { DEMO } from "./demo";
import { demoRaffles, demoSnapshot } from "./demoRaffle";
import { CANISTER_ON, coinsCollectionIndex, useCoinsCollections } from "./hooksCanister";
import { loadRaffles, loadSnapshot, type RaffleView, type Snapshot } from "./raffle";

const k = (key: unknown[]) => (DEMO ? ["demo", ...key] : key);
/** false: this build names no Rook canister (and is not the demo). */
export const RAFFLES_READABLE = DEMO || CANISTER_ON;

/** The collections the canister has pinned, each with the canister's index, in the site's order. */
export function pinnedCollections(cols: CollectionViewV[] | undefined): { slug: string; coll: number }[] {
  return COLLECTIONS.flatMap((c) => { const coll = coinsCollectionIndex(cols, c.wxSymbol); return coll === null ? [] : [{ slug: c.slug, coll }]; });
}

export type RafflesRead = { data: RaffleView[] | undefined; loading: boolean; error: unknown };
export function useRaffles(): RafflesRead {
  const cols = useCoinsCollections();
  const pinned = DEMO ? [] : pinnedCollections(cols.data);
  const q = useQuery({ queryKey: k(["ic-raffles", pinned.map((p) => p.coll).join(",")]), enabled: RAFFLES_READABLE && (DEMO || !!cols.data), refetchInterval: 60_000, staleTime: 30_000,
    queryFn: DEMO ? async () => demoRaffles(Math.floor(Date.now() / 1000)) : async () => loadRaffles(await canister(), pinned) });
  return { data: q.data, loading: RAFFLES_READABLE && (q.isLoading || (!DEMO && cols.isLoading)), error: q.error ?? cols.error };
}
export function useRaffle(slug: string | undefined): { view: RaffleView | null; loading: boolean; error: unknown } {
  const q = useRaffles();
  return { view: q.data?.find((v) => v.slug === slug) ?? null, loading: q.loading, error: q.error };
}
/** A round's published snapshot (`raffle_snapshot` under the collection's wX mint), read once asked for. */
export function useRaffleSnapshot(view: RaffleView | null, enabled: boolean) {
  const round = view?.raffle?.round ?? 0n;
  return useQuery<Snapshot | null>({ queryKey: k(["ic-raffle-snapshot", view?.wxMint ?? null, round.toString()]), enabled: enabled && !!view?.raffle && !!view.wxMint && round > 0n,
    staleTime: 300_000, refetchInterval: false, retry: 0,
    queryFn: DEMO ? async () => demoSnapshot(view!.slug, round) : async () => loadSnapshot(await canister(), view!.wxMint!, round) });
}
