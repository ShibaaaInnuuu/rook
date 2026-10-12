import type { Acquisition, Coin, Distribution, Listing, Lot, Purchase, VaultState } from "../lib/data";
import type * as Api from "./types";
import type { Source } from "./source";

/** The indexer's shapes (types.ts, copied from indexer/) turned into the types the pages already render, so a page is the same
 *  page whether it read the chain, the hosted indexer or a recorded demo. `null` from the API stays unknown: it is carried as
 *  `unknown` with its reason and the pages print the reason, never a zero. */
const big = (s: string | null | undefined) => BigInt(s ?? "0");
const A = (s: string) => s as `0x${string}`;
const MODES: Api.Acquisition["delivery"]["mode"][] = ["unset", "hold", "never", "draw-weighted", "draw-per-wallet", "named", "pro-rata"];
const STAGE: Partial<Record<Api.Acquisition["delivery"]["state"], number>> = { "draw-open": 1, "draw-pinned": 2, "draw-resolved": 3, won: 4, delivered: 5 };
const STATUS: Record<Api.Declaration["status"], number> = { declared: 1, cancelled: 2, released: 3, settled: 4, "written-off": 5 };
export const REASONS: Record<NonNullable<Api.VolumeUntilNextBuy["reason"]>, string> = {
  "no-live-cap": "no live price cap is posted for this listing, so there is nothing to measure against",
  "listing-paused": "the listing is paused by the owner",
  "listing-blocked": "a purchase on this listing is past its receipt deadline, and nothing new can be declared until it is settled or written off",
  "tax-unknown": "this coin's creator tax could not be read from Pons at launch, so the rate is unknown",
  "live-unavailable": "the vault's balances are read from the chain at request time, and they were not available",
};

export const listingOf = (l: Api.ListingSummary): Listing => ({ id: l.id, kind: l.kind, paused: l.paused, divisibility: l.divisibility ?? 0, vault: A(l.vault), name: l.name, ref: l.ref, btcVault: l.btcVault ?? "",
  autonomous: l.autonomous ?? false, buyer: l.buyer ?? null, maxInFlightWei: l.maxInFlightWei ?? null });

export function vaultOf(l: Api.ListingDetail, shiftS: number): VaultState {
  const v = l.live, open = l.declarations.filter((d) => d.status === "declared" || d.status === "released").map((d) => d.purchaseId);
  return { balanceWei: big(v?.balanceWei), attributed: big(v?.attributedWei), reserved: big(v?.reservedWei), spendable: big(v?.spendableWei), bond: big(v?.bondWei), outstanding: big(v?.outstandingWei),
    requiredBond: big(v?.requiredBondWei), bondBps: 10_000, writtenOff: big(l.writtenOffWei), capWei: big(l.capWei), capExpiry: l.capExpiry ? l.capExpiry + shiftS : 0, blockedBy: v?.blockedByPurchase ?? null, open,
    purchases: l.declarations.length, acquisitions: l.acquisitions,
    unknown: !v ? REASONS["live-unavailable"] : v.error ? `the chain read failed: ${v.error}` : undefined, bondBpsKnown: false };
}
export const purchasesOf = (l: Api.ListingDetail, shiftS: number): Purchase[] => [...l.declarations].sort((a, b) => b.purchaseId - a.purchaseId).map((d) => ({ id: d.purchaseId, status: STATUS[d.status],
  releasableAt: d.releasableAt + shiftS, deadline: d.deadline ? d.deadline + shiftS : 0, purchaser: d.purchaser ?? "", maxEth: big(d.maxEthWei), quantity: big(d.quantity), settledQuantity: 0n, returned: big(d.returnedWei), capRef: d.capRef }));
export const acquisitionOf = (a: Api.Acquisition): Acquisition => ({ id: a.acquisitionId, purchaseId: a.purchaseId, coin: A(a.coin), amount: big(a.amount), btcTxid: a.btcTxid.startsWith("0x") ? a.btcTxid : `0x${a.btcTxid}`, satsPaid: big(a.satsPaid), assetRef: a.assetRef });

export function coinOf(c: Api.CoinSummary | Api.CoinDetail, shiftS: number): Coin {
  const d = "volumeUntilNextBuy" in c ? c : null, p = d?.live?.pending, n = d?.volumeUntilNextBuy;
  return { address: A(c.address), name: c.name ?? "unnamed", symbol: c.symbol ?? "?", creator: c.creator, router: A(c.router), listingId: c.listingId, curve: c.curve, creatorTaxBps: c.creatorTaxBps ?? 0,
    graduated: p?.graduated ?? false, routerBalance: big(p?.routerBalanceWei), inEscrow: big(p?.escrowWei), treasuryOwed: big(d?.treasuryDeferredWei), contributed: big(c.toVaultWei),
    charged: d ? d.acquisitions.reduce((x, a) => x + big(a.chargedWei), 0n) : 0n,
    indexed: d && n ? { volumeWei: n.volumeNeededWei === null ? null : big(n.volumeNeededWei), why: n.reason ? REASONS[n.reason] : n.ready ? "the vault can already declare its next purchase" : "",
      targetWei: n.inputs.targetWei === null ? null : big(n.inputs.targetWei), spendableWei: n.inputs.spendableWei === null ? null : big(n.inputs.spendableWei), takeBps: n.inputs.takeBps,
      capExpiry: n.inputs.capExpiry ? n.inputs.capExpiry + shiftS : null, assumptions: n.assumptions,
      pendingUnknown: !d.live ? REASONS["live-unavailable"] : p?.error ? `the chain read failed: ${p.error}` : null,
      curveNote: p && p.graduated === false && p.curvePending === null ? p.curvePendingReason : null, taxKnown: c.creatorTaxBps !== null } : undefined };
}

export function lotOf(d: Api.DrawDetail, shiftS: number): { lot: Lot; distribution: Distribution } {
  const t = (s: number | null | undefined) => (s ? s + shiftS : 0), isDist = d.type === "distribution", ended = d.state === "voided" ? "voided" : d.state === "cancelled" ? "cancelled" : undefined;
  const lot: Lot = { listingId: d.listingId, acquisitionId: d.acquisitionId, mode: Math.max(0, MODES.indexOf(d.mode)), stage: isDist ? 0 : STAGE[d.state] ?? 0, overdue: d.draw?.overdue ?? false,
    challengeEnds: isDist ? 0 : t(d.challengeEnds), round: BigInt(d.draw?.round ?? 0), pinnedAt: t(d.draw?.pinnedAt), destinationAt: t(d.draw?.destinationSetAt), winner: d.draw?.winner ?? "",
    root: isDist ? "" : d.root ?? "", totalTickets: isDist ? 0n : big(d.total), winningTicket: big(d.draw?.winningTicket), deliveryTxid: d.draw?.deliveredTxid ? `0x${d.draw.deliveredTxid.replace(/^0x/, "")}` : "",
    uri: isDist ? "" : d.uri ?? "", destination: d.draw?.destination ?? "", ended, command: d.verifySnapshot.command ?? undefined };
  const open = isDist && (d.state === "distribution-open" || d.state === "distribution-rolled");
  return { lot, distribution: { listingId: d.listingId, acquisitionId: d.acquisitionId, open, rolled: d.state === "distribution-rolled", challengeEnds: isDist ? t(d.challengeEnds) : 0,
    claimsEnd: 0, root: isDist ? d.root ?? "" : "", total: isDist ? big(d.total) : 0n, claimed: big(d.distribution?.claimedUnits), uri: isDist ? d.uri ?? "" : "" } };
}

export type StripStats = { coins: number; listings: number; toVaultsWei: bigint; receipts: number; delivered: number };
export const statsOf = (s: Api.Stats): StripStats => ({ coins: s.coins, listings: s.listings, toVaultsWei: big(s.toVaultsWei), receipts: s.acquisitions.inscriptions + s.acquisitions.runeReceipts, delivered: s.deliveries.done });

/** Every read a page makes, against one source. */
export const reader = (src: Source) => ({
  listings: async () => (await src.get<Api.Page<Api.ListingSummary> | Api.ListingSummary[]>("/v1/listings")),
  listing: (id: number) => src.get<Api.ListingDetail>(`/v1/listings/${id}`),
  coins: () => src.get<Api.Page<Api.CoinSummary> | Api.CoinSummary[]>("/v1/coins"),
  coin: (address: string) => src.get<Api.CoinDetail>(`/v1/coins/${address}`),
  draw: (listingId: number, acquisitionId: number) => src.get<Api.DrawDetail>(`/v1/draws/${listingId}-${acquisitionId}`),
  stats: () => src.get<Api.Stats>("/v1/stats"),
});
export const items = <T,>(x: Api.Page<T> | T[]): T[] => (Array.isArray(x) ? x : x.items);
