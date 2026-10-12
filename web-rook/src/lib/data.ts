import { useQuery } from "@tanstack/react-query";
import { acquisitionOf, coinOf, items, listingOf, lotOf, purchasesOf, reader, vaultOf } from "../api/adapt";
import type { Source } from "../api/source";
import { useEnv } from "./env";

/**
 * Every figure in the app comes through one of these hooks, from one of three places and never a fourth:
 *   - the hosted indexer or a recorded demo of it (a `Source`, src/api), when the environment has one;
 *   - the chain itself (src/lib/reads.ts, loaded only then), when there is a deployment and no indexer;
 *   - nowhere: the hooks stay idle and the pages say that nothing is live.
 * A failed read is shown as a failed read, and a value the indexer reports as unknown is carried as unknown with its reason.
 */
/**
 * One listing. `autonomous` is which vault it got — false is v1 (ETH released to a person the registry names, against a bond)
 * and true is v2 (released only to `buyer`, an address fixed when the vault was built). docs/v2/vault-v2.md.
 */
export type Listing = { id: number; kind: "inscriptions" | "rune"; paused: boolean; divisibility: number; vault: `0x${string}`; name: string; ref: string; btcVault: string;
  autonomous: boolean; buyer: `0x${string}` | null; maxInFlightWei: string | null };
export type VaultState = { balanceWei: bigint; attributed: bigint; reserved: bigint; spendable: bigint; bond: bigint; outstanding: bigint; requiredBond: bigint; bondBps: number;
  writtenOff: bigint; capWei: bigint; capExpiry: number; blockedBy: number | null; open: number[]; purchases: number; acquisitions: number;
  /** set when the balances above could not be read: the reason. The page prints it and shows no figure. */
  unknown?: string; bondBpsKnown: boolean };
export type Purchase = { id: number; status: number; releasableAt: number; deadline: number; purchaser: string; maxEth: bigint; quantity: bigint; settledQuantity: bigint; returned: bigint; capRef: string };
export type Acquisition = { id: number; purchaseId: number; coin: `0x${string}`; amount: bigint; btcTxid: string; satsPaid: bigint; assetRef: string };
export type Coin = { address: `0x${string}`; name: string; symbol: string; creator: string; router: `0x${string}`; listingId: number; curve: string; creatorTaxBps: number;
  graduated: boolean; routerBalance: bigint; inEscrow: bigint; treasuryOwed: bigint; contributed: bigint; charged: bigint;
  /** present when the indexer answered: its own computation of the volume until the next buy, with its inputs and reasons */
  indexed?: { volumeWei: bigint | null; why: string; targetWei: bigint | null; spendableWei: bigint | null; takeBps: number | null; capExpiry: number | null; assumptions: string[];
    pendingUnknown: string | null; curveNote: string | null; taxKnown: boolean } };
export type Lot = { listingId: number; acquisitionId: number; mode: number; stage: number; overdue: boolean; challengeEnds: number; round: bigint; pinnedAt: number; destinationAt: number;
  winner: string; root: string; totalTickets: bigint; winningTicket: bigint; deliveryTxid: string; uri: string; destination: string; ended?: "voided" | "cancelled"; command?: string };
export type Distribution = { listingId: number; acquisitionId: number; open: boolean; rolled: boolean; challengeEnds: number; claimsEnd: number; root: string; total: bigint; claimed: bigint; uri: string };

function useWhere(): { src: Source | null; live: import("./reads").Live | null } {
  const env = useEnv();
  if (!env || env.mode === "none") return { src: null, live: null };
  if (env.mode === "sample" || env.mode === "api") return { src: env.source, live: null };
  return { src: ("api" in env ? env.api : undefined) ?? null, live: { client: env.client, d: env.deployment } };
}
const chain = () => import("./reads");

export function useListings() {
  const { src, live } = useWhere();
  return useQuery({ queryKey: ["listings", src?.label, live?.d.registry], enabled: !!src || !!live, queryFn: async (): Promise<Listing[]> =>
    src ? items(await reader(src).listings()).map(listingOf).sort((a, b) => a.id - b.id) : (await chain()).readListings(live!) });
}
/**
 * The listing detail exactly as the indexer serves it, for the vault page, which prints fields the `VaultState` shape above
 * has no room for (docs/phase14.md §8). It is idle without a source: a visitor reading the chain directly has the balances
 * and the declarations and none of the totals, and the page says which rows that costs rather than computing them from a
 * truncated page or filling them in with zeros.
 */
export function useListingDetail(id?: number) {
  const { src } = useWhere();
  return useQuery({ queryKey: ["listing-detail", src?.label, id], enabled: !!src && id !== undefined, queryFn: () => reader(src!).listing(id!) });
}
/** The seconds a recorded demo's answers are moved by, so that a countdown in one is mid-count rather than long over. */
export function useShift(): number {
  const { src } = useWhere();
  return src?.shiftS ?? 0;
}
export function useVault(vault?: `0x${string}`, listingId?: number) {
  const { src, live } = useWhere(); const detail = useListingDetail(src ? listingId : undefined);
  const q = useQuery({ queryKey: ["vault", vault], enabled: !src && !!live && !!vault, queryFn: async () => (await chain()).readVault(live!, vault!) });
  return src ? { ...detail, data: detail.data ? vaultOf(detail.data, src.shiftS) : undefined } : q;
}
export function usePurchases(vault?: `0x${string}`, count = 0, listingId?: number) {
  const { src, live } = useWhere(); const detail = useListingDetail(src ? listingId : undefined);
  const q = useQuery({ queryKey: ["purchases", vault, count], enabled: !src && !!live && !!vault, queryFn: async () => (await chain()).readPurchases(live!, vault!, count) });
  return src ? { ...detail, data: detail.data ? purchasesOf(detail.data, src.shiftS) : undefined } : q;
}
export function useAcquisitions(vault?: `0x${string}`, count = 0, listingId?: number) {
  const { src, live } = useWhere(); const detail = useListingDetail(src ? listingId : undefined);
  const q = useQuery({ queryKey: ["acquisitions", vault, count], enabled: !src && !!live && !!vault, queryFn: async () => (await chain()).readAcquisitions(live!, vault!, count) });
  return src ? { ...detail, data: detail.data ? detail.data.acquisitionsRecent.map(acquisitionOf).sort((a, b) => b.id - a.id) : undefined } : q;
}
export function useCoins() {
  const { src, live } = useWhere(); const listings = useListings();
  return useQuery({ queryKey: ["coins", src?.label, live?.d.launcher, listings.data?.length], enabled: (!!src || !!live) && !!listings.data, queryFn: async (): Promise<Coin[]> =>
    src ? items(await reader(src).coins()).map((c) => coinOf(c, src.shiftS)) : (await chain()).readCoins(live!, (l) => listings.data![l]?.vault) });
}
export function useCoin(address?: string) {
  const { src, live } = useWhere(); const listings = useListings();
  const ok = !!address && /^0x[0-9a-fA-F]{40}$/.test(address);
  return useQuery({ queryKey: ["coin", src?.label, address, listings.data?.length], enabled: (!!src || !!live) && !!listings.data && ok, queryFn: async (): Promise<Coin | null> => {
    if (!src) return (await chain()).readCoin(live!, address as `0x${string}`, (l) => listings.data![l]?.vault);
    try { return coinOf(await reader(src).coin(address!), src.shiftS); } catch { return null; }
  } });
}
export function useLots(listing?: Listing, acquisitions: Acquisition[] = []) {
  const { src, live } = useWhere();
  return useQuery({ queryKey: ["lots", src?.label, listing?.id, acquisitions.map((a) => a.id).join(",")], enabled: (!!src || !!live) && !!listing, queryFn: async () => {
    if (!src) return (await chain()).readLots(live!, listing!, acquisitions);
    const made = await Promise.all(acquisitions.map(async (a) => { try { return { acquisition: a, ...lotOf(await reader(src).draw(listing!.id, a.id), src.shiftS) }; } catch { return null; } }));
    return made.filter((x): x is NonNullable<typeof x> => x !== null);
  } });
}
export function useLaunchFee() {
  const { live } = useWhere();
  return useQuery({ queryKey: ["launchFee", live?.d.ponsFactory], enabled: !!live, queryFn: async () => (await chain()).readLaunchFee(live!) });
}
