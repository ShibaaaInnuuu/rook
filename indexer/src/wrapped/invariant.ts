import type { CustodyItem, FloorModuleRead, ReserveReport, ReserveStatus } from "../../types.js";

/**
 * docs/v5/ux-and-reserves.md §2. The reserve invariant, recomputed from outside the wrapper's own books:
 *
 *     (items found in custody − tickets in flight) × 1e18 + ETH-backed units  ==  wX totalSupply
 *
 * Pure: every read is an input with its own time, so a test can hand it a stale read, a failed source or a missing item and
 * see the status that follows. Three statuses and one rule above the rest: **nothing that was not read, or not read recently,
 * is ever green**. A proven problem is red even when other sources failed; everything else short of a clean pass is unverified.
 */
export const ONE_ITEM = 10n ** 18n;

export type WrapperRead = { ok: true; totalSupply: bigint; held: number; openTickets: number; block: number; at: number | null } | { ok: false; error: string };
export type InvariantInput = {
  now: number;
  /** a read older than this is stale (seconds) */
  maxAgeS: number;
  wrapper: WrapperRead;
  /** held items folded from the wrapper's events, each with its check */
  items: CustodyItem[];
  /** tickets burned and neither delivered nor refunded, from the same events */
  ticketsInFlight: number;
  /** false when the event log could not be read in full: the ledger is then not a basis for green */
  ledgerComplete: boolean;
  floor: FloorModuleRead;
};
export type InvariantResult = Pick<ReserveReport, "status" | "reasons" | "equation"> & { items: CustodyItem[] };

const plural = (n: number, one: string, many = `${one}s`) => `${n} ${n === 1 ? one : many}`;

export function computeInvariant(x: InvariantInput): InvariantResult {
  const red: string[] = [], unverified: string[] = [];
  const stale = (at: number | null) => at === null || x.now - at > x.maxAgeS;

  // an item's check that is older than the limit is no longer a verdict, whatever it said
  const items = x.items.map((it) => it.check.verdict === "in-custody" && stale(it.check.checkedAt)
    ? { ...it, check: { ...it.check, verdict: "unverified" as const, reason: `the last check is stale (older than ${x.maxAgeS} s)` } } : it);
  const inCustody = items.filter((i) => i.check.verdict === "in-custody").length;
  const elsewhere = items.filter((i) => i.check.verdict === "elsewhere");
  const unread = items.filter((i) => i.check.verdict === "unverified");

  if (elsewhere.length) red.push(`${plural(elsewhere.length, "item")} found outside custody: ${elsewhere.slice(0, 5).map((i) => i.ref).join(", ")}${elsewhere.length > 5 ? ", …" : ""}`);
  if (unread.length) unverified.push(`${plural(unread.length, "item")} could not be verified (${unread[0].check.reason ?? "no reason given"}${unread.length > 1 ? ", and others" : ""})`);
  if (!x.ledgerComplete) unverified.push("the wrapper's event log could not be read in full, so the list of items may be incomplete");

  // docs/v5/floor-module.md §13.1: E = C + R (+ T, the unit in transit: docs/v6/relay-conversion.md §2); each ETH-backed unit is backed by the pot of its tier (B for convertible, S for reserve),
  // so the equation's ETH term is E and the pots' sum (`escrowBalance()`) is what stands behind it
  let ethBacked = 0n, reserveUnits = 0n, floatUnits = 0n, convertible = 0n, escrow = 0n, transit = 0n;
  if (x.floor.kind === "read") {
    ethBacked = BigInt(x.floor.ethBackedUnits); reserveUnits = BigInt(x.floor.reserveUnits); floatUnits = BigInt(x.floor.floatUnits);
    convertible = BigInt(x.floor.convertibleUnits); escrow = BigInt(x.floor.escrowWei); transit = BigInt(x.floor.transitUnits ?? 0);
    if (stale(x.floor.at)) unverified.push("the Floor Module read is stale");
    if (convertible + reserveUnits + transit !== ethBacked) red.push(`the Floor Module's tiers do not add up: ${fmt(convertible)} convertible + ${fmt(reserveUnits)} reserve${transit ? ` + ${fmt(transit)} in transit` : ""} units against ${fmt(ethBacked)} ETH-backed`);
    if (transit > ONE_ITEM) red.push(`the Floor Module counts ${fmt(transit)} units in transit: it allows one advance at a time`);
    if (floatUnits > convertible) red.push(`the Floor Module holds ${fmt(floatUnits)} float units, more than its ${fmt(convertible)} convertible units`);
    if (ethBacked > 0n && escrow === 0n) red.push(`${fmt(ethBacked)} ETH-backed units stand against an empty escrow`);
  } else if (x.floor.kind === "error") unverified.push(`the Floor Module could not be read: ${x.floor.error}`);

  let supply: bigint | null = null, heldOnChain: number | null = null, openOnChain: number | null = null;
  if (!x.wrapper.ok) unverified.push(`the wrapper on Robinhood Chain could not be read: ${x.wrapper.error}`);
  else {
    supply = x.wrapper.totalSupply; heldOnChain = x.wrapper.held; openOnChain = x.wrapper.openTickets;
    if (stale(x.wrapper.at)) unverified.push("the wrapper read is stale");
    if (x.ledgerComplete && x.wrapper.held !== items.length)
      red.push(`the wrapper says it holds ${x.wrapper.held} and its own events account for ${items.length}: its books disagree with themselves`);
    if (x.ledgerComplete && x.wrapper.openTickets !== x.ticketsInFlight)
      red.push(`the wrapper counts ${x.wrapper.openTickets} open tickets and its events ${x.ticketsInFlight}`);
  }

  const backing = BigInt(inCustody - x.ticketsInFlight) * ONE_ITEM + ethBacked;
  const best = BigInt(inCustody + unread.length - x.ticketsInFlight) * ONE_ITEM + ethBacked;
  let holds: boolean | null = null;
  // an incomplete ledger is a list of items that may be missing some: no arithmetic on it can prove anything either way
  if (supply !== null && x.ledgerComplete) {
    if (best < supply) red.push(`under-backed: even if every unverified item is in custody, ${fmt(best)} wX of backing stands against ${fmt(supply)} wX in supply`);
    if (unread.length === 0 && elsewhere.length === 0) {
      holds = backing === supply;
      if (!holds && best >= supply) red.push(`mismatch: ${fmt(backing)} wX of backing against ${fmt(supply)} wX in supply (${backing > supply ? "more backing than supply: the books are wrong" : "under-backed"})`);
    } else if (best < supply) holds = false;
  }

  const status: ReserveStatus = red.length ? "red" : unverified.length ? "unverified" : holds === true ? "green" : "unverified";
  if (status === "unverified" && !unverified.length) unverified.push("the equation could not be decided");
  return {
    status, reasons: [...red, ...unverified], items,
    equation: { itemsInCustody: inCustody, itemsUnverified: unread.length, itemsElsewhere: elsewhere.length, ticketsInFlight: x.ticketsInFlight,
      backingUnits: String(backing), backingUnitsIfUnverifiedHold: String(best), ethBackedUnits: String(ethBacked),
      reserveUnits: String(reserveUnits), floatUnits: String(floatUnits), convertibleUnits: String(convertible), ...(transit ? { transitUnits: String(transit) } : {}), escrowWei: String(escrow), supplyUnits: supply === null ? null : String(supply),
      heldOnChain, openTicketsOnChain: openOnChain, heldInLedger: items.length, holds },
  };
}

/** wX base units as whole items, exact to 18 places with trailing zeros cut. */
export function fmt(units: bigint): string {
  const neg = units < 0n, v = neg ? -units : units, frac = (v % ONE_ITEM).toString().padStart(18, "0").replace(/0+$/, "");
  return `${neg ? "−" : ""}${v / ONE_ITEM}${frac ? "." + frac : ""}`;
}
