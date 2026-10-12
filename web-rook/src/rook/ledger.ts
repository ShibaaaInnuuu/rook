import type { ReserveReport } from "../api/types";

/**
 * docs/v6/site-design.md §6, "The ledger's equation". Pure: a reserve report in, the rows of a ledger out, summing to the
 * supply, with the verdict the report carries.
 *
 *   (items in custody − tickets in flight) × 1  +  to be converted  +  reserve  +  float  =  supply
 *
 * The indexer's report carries the Floor Module's books as built (docs/v5/floor-module.md §13.1): `ethBackedUnits` E, the
 * standing reserve's units R (`reserveUnits`), the convertible units C = E − R (`convertibleUnits`) and the float units the
 * module holds as inventory inside C (`floatUnits`). The ledger shows C − float, R and float as three rows; without a module
 * the ETH-backed row is zero by construction; a module that could not be read is one row that says so.
 */
export type LedgerRow = { key: string; label: string; note?: string; units: bigint | null; kind: "item" | "eth" | "sum" | "supply" };
export type Ledger = {
  rows: LedgerRow[];
  backing: bigint;
  supply: bigint | null;
  /** true / false when decided, null when a term is unknown (the report says so) */
  holds: boolean | null;
  status: ReserveReport["status"];
  reasons: string[];
  /** whether the ETH-backed units are shown split into the three tiers (a module was read) */
  splitKnown: boolean;
};

const ONE = 10n ** 18n;
const big = (x: string | null | undefined) => (x === null || x === undefined ? null : BigInt(x));

export function ledgerOf(r: ReserveReport): Ledger {
  const e = r.equation, m = r.floorModule;
  const itemUnits = BigInt(Math.max(0, e.itemsInCustody - e.ticketsInFlight)) * ONE;
  const ethBacked = BigInt(e.ethBackedUnits);
  const supply = big(e.supplyUnits);
  const rows: LedgerRow[] = [
    { key: "items", label: "NFTs in the vault", note: `${e.itemsInCustody} found${e.itemsUnverified ? `, ${e.itemsUnverified} unverified` : ""}${e.itemsElsewhere ? `, ${e.itemsElsewhere} elsewhere` : ""}`, units: BigInt(e.itemsInCustody) * ONE, kind: "item" },
    { key: "tickets", label: "less redemptions in flight", note: "burnt, not yet delivered: they back no circulating token", units: -BigInt(e.ticketsInFlight) * ONE, kind: "item" },
  ];
  if (m.kind === "absent") {
    rows.push({ key: "eth", label: "Backed by ETH", note: "no floor module for this collection: zero by construction", units: 0n, kind: "eth" });
  } else if (m.kind === "read") {
    const reserve = BigInt(m.reserveUnits), float = BigInt(m.floatUnits), transit = BigInt(m.transitUnits ?? 0);
    const convertible = BigInt(m.convertibleUnits) - float;
    rows.push({ key: "convertible", label: "ETH-backed, to be converted", note: "the canister buys an NFT for each of these", units: convertible < 0n ? 0n : convertible, kind: "eth" });
    // docs/v6/relay-conversion.md §2.4: one unit whose ETH already left to be converted into its frog (E = C + R + T)
    if (transit > 0n) rows.push({ key: "transit", label: "ETH-backed, in transit", note: "its ETH was advanced to be converted into a frog; backed by that ETH until the frog arrives", units: transit, kind: "eth" });
    rows.push({ key: "reserve", label: "ETH-backed, standing reserve", note: "stays ETH so burns are paid at once", units: reserve, kind: "eth" });
    rows.push({ key: "float", label: "ETH-backed, float inventory", note: "the module's own wrapped tokens, bought ahead of demand", units: float, kind: "eth" });
  } else {
    rows.push({ key: "eth", label: "Backed by ETH", note: `the module could not be read: ${m.error}`, units: ethBacked, kind: "eth" });
  }
  const backing = itemUnits + ethBacked;
  rows.push({ key: "sum", label: "Backing", units: backing, kind: "sum" });
  rows.push({ key: "supply", label: "Supply", note: "the wrapper's totalSupply at the same block", units: supply, kind: "supply" });
  return { rows, backing, supply, holds: e.holds, status: r.status, reasons: r.reasons, splitKnown: m.kind === "read" };
}

/** The rows' sum equals the report's own `backingUnits`, and the ETH rows sum to its `ethBackedUnits`: this ledger and the indexer count the same thing. */
export const ledgerAgrees = (l: Ledger, r: ReserveReport): boolean =>
  l.backing === BigInt(r.equation.backingUnits) && l.rows.filter((x) => x.kind === "eth").reduce((a, x) => a + (x.units ?? 0n), 0n) === BigInt(r.equation.ethBackedUnits);

/** "4", "4.25", "0": units as whole items with at most 4 decimals, never scientific. */
export function items(units: bigint | null): string {
  if (units === null) return "?";
  const neg = units < 0n, v = neg ? -units : units;
  const whole = v / ONE, frac = (v % ONE).toString().padStart(18, "0").slice(0, 4).replace(/0+$/, "");
  return `${neg ? "−" : ""}${whole.toLocaleString("en-US")}${frac ? "." + frac : ""}`;
}
