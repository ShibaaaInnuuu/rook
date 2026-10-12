import type * as Api from "../api/types";

/**
 * docs/phase14.md §8. Everything the vault page prints, as one value read off one response, so that the page itself decides
 * nothing and every figure can be checked without a browser.
 *
 * The rule the whole file keeps: **`null` is "not reported", and it is never a zero.** The committed recording predates
 * four of these fields (§8.7) and a chain-only visitor has none of them at all, so each one can be genuinely absent, and a
 * row that met an absent figure and printed `0.0000 ETH` would be inventing the most reassuring possible lie.
 */

/** wei, or "the source did not report it". */
export type Amount = bigint | null;

/**
 * What a vault has bought, or is holding, in the only form that works for both kinds it can hold.
 *
 * An inscription is one of a kind and a rune is an amount. "Swept 263" is a sentence about a collection; the same sentence
 * about a rune counts receipts, and a receipt is not a quantity — vault 1 of the recording has 26 of them carrying
 * 371,165.08 DEMO•RUNE between them. So the two kinds get two shapes and neither ever stands in for the other.
 */
export type Quantity =
  | { kind: "inscriptions"; items: number }
  | { kind: "rune"; units: bigint; receipts: number }
  | null;

export type Cap = {
  wei: bigint;
  /** a cap lasts an hour from the block that posted it; past that nothing can be declared until a fresh one arrives */
  live: boolean;
  expiry: number;
  /** the other end of the same hour, so a reader can see how fresh the keeper's word is. Null on a source older than §8.4. */
  postedAt: number | null;
};

export type VaultFacts = {
  /** the reason the live reads are missing, when they are. The page prints it and shows no figure in their place. */
  unknown: string | null;
  /** attributed to a coin and not promised to a declaration: the only ETH a declaration can reach */
  budget: Amount;
  balance: Amount;
  attributed: Amount;
  waiting: Amount;
  reserved: Amount;
  bond: Amount;
  requiredBond: Amount;
  outstanding: Amount;
  /**
   * ETH in the contract that no coin is credited with and no bond accounts for.
   *
   * Etch has no donation path — `deposit` reverts for anyone but that coin's own router and `depositFrom` for anyone but
   * the treasury — so the only way a wei gets here is `selfdestruct`, and the contract's own header says what becomes of
   * it: it is never spendable. Measured on the recording, both vaults report exactly zero.
   */
  unattributed: Amount;
  cap: Cap | null;
  /** budget ÷ cap: items at the current cap for a collection, whole units for a rune. Null without both numbers. */
  buysNow: number | null;
  /** every `Deposited`: the coins' own routers, and the treasury's share attributed to the protocol coin */
  fedIn: Amount;
  /** released − returned over settled purchases, the same definition /v1/stats uses */
  spent: Amount;
  released: Amount;
  returned: Amount;
  writtenOff: Amount;
  bought: Quantity;
  held: Quantity;
  /** delivered lots, for either kind */
  delivered: number | null;
  /** raw rune units a recorded claim has paid out. Null on a collection, and on a source that does not report it. */
  paidOut: bigint | null;
  blockedBy: number | null;
};

const big = (s: string | null | undefined): Amount => (s === null || s === undefined ? null : BigInt(s));
const less = (...xs: Amount[]): Amount => (xs.some((x) => x === null) ? null : xs.slice(1).reduce((t: bigint, x) => t - x!, xs[0]!));

/** A ratio of two wei figures as a plain number, four decimals of it, done in bigints so a 19-digit numerator is exact. */
export function ratio(top: Amount, bottom: Amount): number | null {
  if (top === null || bottom === null || bottom <= 0n) return null;
  return Number((top * 10_000n) / bottom) / 10_000;
}

/** How full the groove is: a ratio clamped into it, and 0 when there is nothing to draw. */
export const groove = (r: number | null): number => (r === null || !Number.isFinite(r) || r <= 0 ? 0 : Math.min(1, r));

/**
 * The cap, from whichever of the two shapes the source speaks. `capExpired` is the indexer's own comparison, made when the
 * request was answered; `now` is this browser's clock. Either one saying the hour is over is enough: a cap nobody will
 * accept is not a live cap, and the expired branch is the one that says what it stops.
 */
export function capState(l: Pick<Api.ListingSummary, "capWei" | "capExpiry" | "capExpired"> & { capPostedAt?: number | null }, nowS: number, shiftS = 0): Cap | null {
  const wei = big(l.capWei);
  if (wei === null || wei <= 0n || !l.capExpiry) return null;
  const expiry = l.capExpiry + shiftS;
  return { wei, expiry, live: expiry > nowS && l.capExpired !== true, postedAt: l.capPostedAt === null || l.capPostedAt === undefined ? null : l.capPostedAt + shiftS };
}

const quantityOf = (h: Api.Holdings | undefined, kind: Api.ListingKind, which: "bought" | "held"): Quantity => {
  if (!h) return null;
  if (kind === "rune") {
    const u = h.runeUnits;
    if (!u) return null;
    return { kind, units: BigInt(which === "bought" ? u.received : u.held), receipts: h.receipts };
  }
  return { kind: "inscriptions", items: which === "bought" ? h.receipts : h.inscriptionsHeld ?? h.receipts - h.delivered };
};

/**
 * One reading of a vault, from the listing detail the indexer serves. `unknown` carries the reason the `live` block is
 * missing; every figure that depended on it is then null and the page prints the reason once instead of nine zeros.
 */
export function factsOf(l: Api.ListingDetail, nowS: number, shiftS = 0, liveError?: string | null): VaultFacts {
  const v = l.live;
  const d = l as Partial<Api.ListingDetail>;      // a recording older than docs/phase14.md §8.4 has none of the four
  const attributed = big(v?.attributedWei), waiting = big(v?.waitingWei), bond = big(v?.bondWei), balance = big(v?.balanceWei);
  const cap = capState(l, nowS, shiftS);
  const budget = big(v?.spendableWei);
  return {
    unknown: !v ? (liveError ?? "the vault's balances are read from the chain at request time, and they were not available") : v.error ? `the chain read failed: ${v.error}` : null,
    budget, balance, attributed, waiting, reserved: big(v?.reservedWei), bond, requiredBond: big(v?.requiredBondWei), outstanding: big(v?.outstandingWei),
    unattributed: less(balance, attributed, waiting, bond),
    cap, buysNow: cap ? ratio(budget, cap.wei) : null,
    fedIn: big(l.depositedWei),
    spent: big(d.spentWei), released: big(d.releasedWei), returned: big(d.returnedWei), writtenOff: big(l.writtenOffWei),
    bought: quantityOf(d.holdings, l.kind, "bought"), held: quantityOf(d.holdings, l.kind, "held"),
    delivered: d.holdings?.delivered ?? null,
    paidOut: d.holdings?.runeUnits ? BigInt(d.holdings.runeUnits.paidOut) : null,
    blockedBy: v?.blockedByPurchase ?? null,
  };
}

/** One coin paying into the vault, and its share of everything that has been paid in. */
export type PaidIn = { address: string; symbol: string | null; wei: bigint; share: number | null };

/**
 * The coins paying into this vault, most first, each with its share of the total.
 *
 * The share is a ratio of two numbers in the same response, which is why the indexer does not serve it: a second place for
 * the same fact is a second place for it to be wrong. A vault nothing has been paid into has no shares rather than shares
 * of zero.
 */
export function paidIn(l: Pick<Api.ListingDetail, "coinsList" | "depositedWei">): PaidIn[] {
  const total = BigInt(l.depositedWei);
  return l.coinsList
    .map((c) => ({ address: c.address as string, symbol: c.symbol, wei: BigInt(c.depositedWei), share: total > 0n ? Number((BigInt(c.depositedWei) * 1_000_000n) / total) / 1e6 : null }))
    .sort((a, b) => (b.wei > a.wei ? 1 : b.wei < a.wei ? -1 : 0) || a.address.localeCompare(b.address));
}

/** A share as a percentage, or null when there is no denominator. Never "NaN%", never a percentage of nothing. */
export function pct(share: number | null): string | null {
  if (share === null || !Number.isFinite(share)) return null;
  if (share === 0) return "0%";
  return `${(share * 100).toFixed(share >= 0.1 ? 1 : 2)}%`;
}

/**
 * What has become of a lot, in words. Every state the Distributor can be in has one, because "draw-pinned" is a value in a
 * database and not a thing to put in front of a reader.
 */
export const DELIVERY_IS: Record<Api.Acquisition["delivery"]["state"], string> = {
  undecided: "waiting on its coin's creator",
  held: "held in the vault",
  "never-delivered": "never to be delivered",
  "draw-open": "draw open",
  "draw-pinned": "round pinned",
  "draw-resolved": "draw resolved",
  won: "winner proven",
  delivered: "delivered",
  voided: "draw voided, still held",
  cancelled: "draw cancelled",
  "distribution-open": "claims open",
  "distribution-rolled": "rolled over",
};

/** One line of the purchase history: what was bought, what its purchase cost, and where the thing is now. */
export type Sweep = {
  id: string;
  acquisitionId: number;
  purchaseId: number;
  kind: Api.ListingKind;
  assetRef: string;
  amount: bigint;
  coin: string;
  coinSymbol: string | null;
  btcTxid: string;
  satsPaid: bigint;
  at: number | null;
  /** what the whole purchase cost the vault, and how many receipts came out of it. Null on a source older than §8.4. */
  purchaseSpent: bigint | null;
  purchaseItems: number | null;
  state: Api.Acquisition["delivery"]["state"];
};

export function sweeps(rows: readonly Api.Acquisition[], shiftS = 0): Sweep[] {
  return rows.map((a) => {
    const x = a as Partial<Api.Acquisition>;
    return {
      id: `${a.listingId}-${a.acquisitionId}`, acquisitionId: a.acquisitionId, purchaseId: a.purchaseId, kind: a.kind, assetRef: a.assetRef, amount: BigInt(a.amount),
      coin: a.coin, coinSymbol: a.coinSymbol, btcTxid: a.btcTxid, satsPaid: BigInt(a.satsPaid), at: a.settledAt === null ? null : a.settledAt + shiftS,
      purchaseSpent: x.purchaseSpentWei === undefined ? null : BigInt(x.purchaseSpentWei), purchaseItems: x.purchaseAcquisitions ?? null,
      state: a.delivery.state,
    };
  }).sort((p, q) => q.acquisitionId - p.acquisitionId);
}

/**
 * The same reading, for a visitor with a chain and no indexer.
 *
 * `readVault` asks the contract for its balances and its cap and for nothing that has to be summed over history, so the
 * totals — fed in, spent, released, the holdings — are genuinely **not reported** here rather than zero. The one count it
 * does have is `acquisitionCount()`, which is a number of inscriptions for a collection and a number of receipts for a
 * rune, and a number of receipts is not a quantity of a rune, so that half stays null too (§8.3).
 */
export function factsFromState(v: {
  balanceWei: bigint; attributed: bigint; reserved: bigint; spendable: bigint; bond: bigint; outstanding: bigint; requiredBond: bigint;
  writtenOff: bigint; capWei: bigint; capExpiry: number; blockedBy: number | null; acquisitions: number; unknown?: string;
}, kind: Api.ListingKind, nowS: number): VaultFacts {
  const known = <T,>(x: T): T | null => (v.unknown ? null : x);
  const cap = v.unknown || v.capWei <= 0n || !v.capExpiry ? null : { wei: v.capWei, expiry: v.capExpiry, live: v.capExpiry > nowS, postedAt: null };
  return {
    unknown: v.unknown ?? null,
    budget: known(v.spendable), balance: known(v.balanceWei), attributed: known(v.attributed), waiting: null, reserved: known(v.reserved),
    bond: known(v.bond), requiredBond: known(v.requiredBond), outstanding: known(v.outstanding),
    // `waiting` is not among the reads, and a subtraction missing one of its terms is not a number
    unattributed: null,
    cap, buysNow: cap ? ratio(known(v.spendable), cap.wei) : null,
    fedIn: null, spent: null, released: null, returned: null, writtenOff: v.writtenOff,
    bought: kind === "rune" ? null : { kind: "inscriptions", items: v.acquisitions }, held: null, delivered: null, paidOut: null,
    blockedBy: v.blockedBy,
  };
}
