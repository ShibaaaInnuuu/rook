import type { PublicClient } from "viem";
import { DISTRIBUTOR, ERC20, ESCROW, FACTORY, LAUNCHER, REGISTRY, ROUTER, VAULT } from "./abis";
import type { Deployment } from "./chain";
import type { Acquisition, Coin, Distribution, Listing, Lot, Purchase, VaultState } from "./data";

/** Every read of the chain. Imported lazily (it pulls viem's ABI parser in), so the landing page never loads it unless there is a
 *  deployment to read. There is no fallback data: a failed read is shown as a failed read. */
export type Live = { client: PublicClient; d: Deployment };
const A = (a: string) => a as `0x${string}`;
export async function readListings({ client, d }: Live): Promise<Listing[]> {
  const n = Number(await client.readContract({ address: d.registry, abi: REGISTRY, functionName: "count" }));
  return Promise.all(Array.from({ length: n }, async (_, id) => {
    const l = await client.readContract({ address: d.registry, abi: REGISTRY, functionName: "listing", args: [BigInt(id)] });
    // a v2 vault carries its own buyer and ceiling; reading them off the vault costs two calls, and only for a v2 listing
    let buyer: `0x${string}` | null = null, maxInFlightWei: string | null = null;
    if (l.autonomous) {
      const v = <T extends string>(functionName: T) => client.readContract({ address: l.vault, abi: VAULT, functionName: functionName as never }) as Promise<unknown>;
      const [b, m] = await Promise.all([v("buyer").catch(() => null), v("maxInFlight").catch(() => null)]);
      buyer = b === null ? null : A(String(b));
      maxInFlightWei = m === null ? null : String(m);
    }
    return { id, kind: l.kind === 0 ? "inscriptions" as const : "rune" as const, paused: l.paused, divisibility: l.divisibility, vault: l.vault, name: l.name, ref: l.ref, btcVault: l.btcVault,
      autonomous: l.autonomous, buyer, maxInFlightWei };
  }));
}
export async function readVault({ client, d }: Live, vault: `0x${string}`): Promise<VaultState> {
  const r = <T extends string>(functionName: T) => client.readContract({ address: vault, abi: VAULT, functionName: functionName as never }) as Promise<unknown>;
  const [balanceWei, attributed, reserved, bond, outstanding, requiredBond, writtenOff, capWei, capExpiry, blocked, open, purchases, acquisitions, bondBps] = await Promise.all([
    client.getBalance({ address: vault }), r("attributed"), r("reserved"), r("bond"), r("outstanding"), r("requiredBond"), r("totalWrittenOff"), r("capWei"), r("capExpiry"),
    r("isBlocked"), r("openPurchases"), r("purchaseCount"), r("acquisitionCount"), client.readContract({ address: d.registry, abi: REGISTRY, functionName: "bondBps" })]);
  const [isBlocked, by] = blocked as [boolean, bigint];
  return { bondBpsKnown: true, balanceWei, attributed: attributed as bigint, reserved: reserved as bigint, spendable: (attributed as bigint) - (reserved as bigint), bond: bond as bigint,
    outstanding: outstanding as bigint, requiredBond: requiredBond as bigint, bondBps: Number(bondBps), writtenOff: writtenOff as bigint, capWei: capWei as bigint,
    capExpiry: Number(capExpiry), blockedBy: isBlocked ? Number(by) : null, open: (open as bigint[]).map(Number), purchases: Number(purchases), acquisitions: Number(acquisitions) };
}
export async function readPurchases(live: Live, vault: `0x${string}`, count: number): Promise<Purchase[]> {
  const ids = Array.from({ length: Math.min(count, 25) }, (_, i) => count - 1 - i);      // newest first, the last 25
  return Promise.all(ids.map(async (id) => { const p = await live.client.readContract({ address: vault, abi: VAULT, functionName: "purchase", args: [BigInt(id)] });
    return { id, status: p.status, releasableAt: Number(p.releasableAt), deadline: Number(p.deadline), purchaser: p.purchaser, maxEth: p.maxEth, quantity: p.quantity,
      settledQuantity: p.settledQuantity, returned: p.returned, capRef: p.capRef }; }));
}
export async function readAcquisitions(live: Live, vault: `0x${string}`, count: number): Promise<Acquisition[]> {
  const ids = Array.from({ length: Math.min(count, 50) }, (_, i) => count - 1 - i);
  return Promise.all(ids.map(async (id) => { const a = await live.client.readContract({ address: vault, abi: VAULT, functionName: "acquisition", args: [BigInt(id)] });
    return { id, purchaseId: Number(a.purchaseId), coin: a.coin, amount: a.amount, btcTxid: a.btcTxid, satsPaid: a.satsPaid, assetRef: a.assetRef }; }));
}
export async function readCoin({ client, d }: Live, coin: `0x${string}`, vaultOf: (listing: number) => `0x${string}` | undefined): Promise<Coin | null> {
  const router = await client.readContract({ address: d.launcher, abi: LAUNCHER, functionName: "routerOf", args: [coin] });
  if (/^0x0+$/.test(router)) return null;
  const [creator, listing, launched, escrow, name, symbol] = await Promise.all([
    client.readContract({ address: d.launcher, abi: LAUNCHER, functionName: "creatorOf", args: [coin] }),
    client.readContract({ address: d.launcher, abi: LAUNCHER, functionName: "listingOf", args: [coin] }),
    client.readContract({ address: d.ponsFactory, abi: FACTORY, functionName: "getLaunchedToken", args: [coin] }),
    client.readContract({ address: d.ponsFactory, abi: FACTORY, functionName: "feeEscrow" }),
    client.readContract({ address: coin, abi: ERC20, functionName: "name" }).catch(() => "?"),
    client.readContract({ address: coin, abi: ERC20, functionName: "symbol" }).catch(() => "?")]);
  const vault = vaultOf(Number(listing));
  const [routerBalance, inEscrow, treasuryOwed, contributed, charged] = await Promise.all([
    client.getBalance({ address: router }), client.readContract({ address: escrow, abi: ESCROW, functionName: "balanceOf", args: [router] }),
    client.readContract({ address: router, abi: ROUTER, functionName: "treasuryOwed" }),
    vault ? client.readContract({ address: vault, abi: VAULT, functionName: "contributedBy", args: [coin] }) : 0n,
    vault ? client.readContract({ address: vault, abi: VAULT, functionName: "chargedTo", args: [coin] }) : 0n]);
  return { address: coin, name, symbol, creator, router, listingId: Number(listing), curve: launched[1], creatorTaxBps: launched[8], graduated: launched[10] !== 0,
    routerBalance, inEscrow, treasuryOwed, contributed, charged };
}
export async function readCoins(live: Live, vaultOf: (listing: number) => `0x${string}` | undefined): Promise<Coin[]> {
  const n = Number(await live.client.readContract({ address: live.d.launcher, abi: LAUNCHER, functionName: "coinCount" }));
  const ids = Array.from({ length: Math.min(n, 50) }, (_, i) => n - 1 - i);
  const addrs = await Promise.all(ids.map((i) => live.client.readContract({ address: live.d.launcher, abi: LAUNCHER, functionName: "coins", args: [BigInt(i)] })));
  return (await Promise.all(addrs.map((a) => readCoin(live, a, vaultOf)))).filter((c): c is Coin => c !== null);
}
export async function readLots(live: Live, listing: Listing, acquisitions: Acquisition[]): Promise<{ acquisition: Acquisition; lot: Lot; distribution: Distribution }[]> {
  return Promise.all(acquisitions.map(async (a) => {
    const args = [BigInt(listing.id), BigInt(a.id)] as const;
    const [l, dist] = await Promise.all([live.client.readContract({ address: live.d.distributor, abi: DISTRIBUTOR, functionName: "lot", args }),
      live.client.readContract({ address: live.d.distributor, abi: DISTRIBUTOR, functionName: "distribution", args })]);
    const lot: Lot = { listingId: listing.id, acquisitionId: a.id, mode: l.mode, stage: l.stage, overdue: l.overdue, challengeEnds: Number(l.challengeEnds), round: l.round,
      pinnedAt: Number(l.pinnedAt), destinationAt: Number(l.destinationAt), winner: l.winner, root: l.root, totalTickets: l.totalTickets, winningTicket: l.winningTicket,
      deliveryTxid: l.deliveryTxid, uri: l.uri, destination: l.destination };
    const distribution: Distribution = { listingId: listing.id, acquisitionId: a.id, open: dist.open, rolled: dist.rolled, challengeEnds: Number(dist.challengeEnds),
      claimsEnd: Number(dist.claimsEnd), root: dist.root, total: dist.total, claimed: dist.claimed, uri: dist.uri };
    return { acquisition: a, lot, distribution };
  }));
}
export async function readLaunchFee(live: Live): Promise<bigint> { return live.client.readContract({ address: live.d.ponsFactory, abi: FACTORY, functionName: "launchFee" }); }
