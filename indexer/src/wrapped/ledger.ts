import type { Hex, ItemArrival, Ticket, WrappedActivity, WrappedKind } from "../../types.js";

/**
 * docs/v5/ux-and-reserves.md §2, step 1. The wrapper's own events folded into its ledger: which items entered custody, how and
 * when; which tickets were burned and where each one stands. Pure — the service decodes logs, this only folds them.
 *
 * `Wrapper` (inscriptions) and `NftWrapper` (ERC-721s) emit events of the same names with different arguments; both arrive
 * here as `WrapperEvent`, already decoded against the right ABI.
 */
export type WrapperEvent = { name: string; args: Record<string, unknown>; block: number; logIndex: number; tx: Hex; ts: number | null };
export type LedgerItem = { itemId: string; ref: string; arrival: ItemArrival; held: boolean };
export type Ledger = { items: Map<string, LedgerItem>; tickets: Map<number, Ticket>; activity: WrappedActivity[] };

export type Links = { btcTx: (txid: string) => string; evmTx: ((hash: string) => string) | null };

const s = (x: unknown) => String(x);
const hex32 = (x: unknown) => s(x).replace(/^0x/, "").toLowerCase();

/**
 * `purchaseSinks`: the collection's buy queue and mint desk. `Wrapper` mints every Bitcoin arrival through `wrapDeposit`, the
 * canister's own purchases included (docs/v3/curve.md: it buys, confirms, then `wrapDeposit(…, to = queue)`), so for inscriptions
 * a `Deposited` to one of these is a purchase and any other is a holder's deposit. `NftWrapper` says which with `mint` / `mintDeposit`.
 */
export function foldLedger(kind: WrappedKind, events: WrapperEvent[], links: Links, burnDestination: string, now: number, purchaseSinks: string[] = []): Ledger {
  const sinks = new Set(purchaseSinks.map((x) => x.toLowerCase()));
  const items = new Map<string, LedgerItem>(), tickets = new Map<number, Ticket>(), activity: WrappedActivity[] = [];
  const byToken = new Map<string, string>();      // NFT token id -> item id, for Delivered(tokenId)
  const sorted = [...events].sort((a, b) => a.block - b.block || a.logIndex - b.logIndex);
  const refOf = (itemId: string) => items.get(itemId)?.ref ?? null;
  for (const e of sorted) {
    const a = e.args, at = { at: e.ts, block: e.block, tx: e.tx };
    switch (e.name) {
      case "Wrapped": case "Deposited": {
        const deposit = e.name === "Deposited" && !(kind === "inscriptions" && sinks.has(s(a.to).toLowerCase()));
        const itemId = s(kind === "inscriptions" ? (e.name === "Deposited" ? a.frogId : a.acquisitionId) : a.itemId);
        const ref = kind === "inscriptions" ? s(a.inscriptionId) : s(a.tokenId);
        // the transaction that brought it in: an NFT's purchase or deposit tx on its chain; an inscription's deposit txid.
        // A vault purchase (Wrapper.wrap) carries no Bitcoin txid in the event; ord's satpoint supplies it at check time.
        const raw = kind === "nft" ? hex32(e.name === "Deposited" ? a.depositTx : a.purchaseTx) : e.name === "Deposited" ? hex32(a.txid) : null;
        const tx = raw === null ? null : kind === "nft" ? `0x${raw}` : raw;
        const txUrl = tx === null ? null : kind === "nft" ? (links.evmTx ? links.evmTx(tx) : null) : links.btcTx(tx);
        items.set(itemId, { itemId, ref, held: true, arrival: { how: deposit ? "deposit" : "purchase", tx, txUrl, mintTx: e.tx, mintedAt: e.ts, mintBlock: e.block } });
        if (kind === "nft") byToken.set(ref, itemId);
        activity.push({ ...at, kind: "arrived", text: `${kind === "nft" ? `#${ref}` : short(ref)} entered custody (${deposit ? "a holder's deposit" : "bought"})` });
        break;
      }
      case "Redeemed": {
        const id = Number(a.ticketId), destination = s(a.destination);
        const toBurn = destination.toLowerCase() === burnDestination.toLowerCase();
        tickets.set(id, { id, owner: s(a.owner) as Hex, destination, toBurn, stage: "pinned", round: Number(a.round), burnedAt: e.ts, burnTx: e.tx,
          itemId: null, itemRef: null, deliverBy: null, overdue: false, deliveryTx: null, deliveryUrl: null, refundTx: null });
        activity.push({ ...at, kind: toBurn ? "burned" : "redeemed", text: toBurn ? `one whole token burned for an item sent where nobody can move it (ticket ${id})` : `one whole token redeemed for a random item (ticket ${id})` });
        break;
      }
      case "Repinned": { const t = tickets.get(Number(a.ticketId)); if (t) t.round = Number(a.round); break; }
      case "Resolved": {
        const t = tickets.get(Number(a.ticketId)); if (!t) break;
        t.stage = "resolved"; t.itemId = s(kind === "inscriptions" ? a.acquisitionId : a.itemId); t.itemRef = refOf(t.itemId); t.deliverBy = Number(a.deliverBy);
        activity.push({ ...at, kind: "resolved", text: `ticket ${t.id} drew ${t.itemRef ? (kind === "nft" ? `#${t.itemRef}` : short(t.itemRef)) : `item ${t.itemId}`}` });
        break;
      }
      case "Delivered": {
        const t = tickets.get(Number(a.ticketId)); if (!t) break;
        const itemId = kind === "nft" ? byToken.get(s(a.tokenId)) ?? t.itemId : s(a.acquisitionId);
        const raw = hex32(kind === "nft" ? a.txHash : a.txid), tx = kind === "nft" ? `0x${raw}` : raw;
        t.stage = "delivered"; t.deliveryTx = tx; t.deliveryUrl = kind === "nft" ? (links.evmTx ? links.evmTx(tx) : null) : links.btcTx(tx);
        const item = itemId ? items.get(itemId) : undefined; if (item) item.held = false;
        activity.push({ ...at, kind: "delivered", text: `ticket ${t.id} delivered` });
        break;
      }
      case "Undeliverable": {
        // fixes-R3-contracts §4 (canister review2 N-2): the custodian attests that no version of the delivery can confirm (its
        // conflict-out is deep, or nothing was ever broadcast); the ticket then refunds at deliverBy + REFUND_GRACE, not + 30 days
        const t = tickets.get(Number(a.ticketId)); if (!t) break;
        t.undeliverable = true;
        activity.push({ ...at, kind: "resolved", text: `custody attested that ticket ${t.id}'s delivery cannot confirm: it is refundable 72 hours after its delivery deadline` });
        break;
      }
      case "Refunded": {
        const t = tickets.get(Number(a.ticketId)); if (!t) break;
        // fixes-B-C E-03: a ticket nobody resolved within PIN_TIMEOUT of its pin is refunded with acquisitionId = type(uint256).max:
        // no item was drawn, so none goes back to the set (Wrapper.sol refund). The NFT wrapper (OFF) has no such path.
        // EthNftWrapper (docs/v7/phase1-nft.md §3.1) refunds an unresolved ticket the same way, with itemId = type(uint256).max
        const unresolved = s(kind === "inscriptions" ? a.acquisitionId : a.itemId) === UNRESOLVED;
        t.stage = "refunded"; t.refundTx = e.tx; t.refundedUnresolved = unresolved;
        activity.push({ ...at, kind: "refunded", text: unresolved
          ? `ticket ${t.id} was not resolved within 7 days of its pin and was refunded: one whole token back to its owner, no item drawn`
          : `ticket ${t.id} was not delivered in time and was refunded: one whole token back to its owner, the item back in the set` });
        break;
      }
    }
  }
  for (const t of tickets.values()) {
    t.overdue = t.stage === "resolved" && t.deliverBy !== null && now > t.deliverBy;
    t.refundableAt = refundableAt(kind, t);
    if (t.refundedUnresolved === undefined) t.refundedUnresolved = false;
    if (t.undeliverable === undefined) t.undeliverable = false;
  }
  return { items, tickets, activity: activity.reverse() };
}

/** Items still in custody, and tickets still in flight (burned, neither delivered nor refunded), with who each held item is promised to. */
export function openState(l: Ledger): { held: LedgerItem[]; inFlight: Ticket[]; promised: Map<string, number> } {
  const inFlight = [...l.tickets.values()].filter((t) => t.stage === "pinned" || t.stage === "resolved");
  const promised = new Map<string, number>();
  for (const t of inFlight) if (t.stage === "resolved" && t.itemId) promised.set(t.itemId, t.id);
  return { held: [...l.items.values()].filter((i) => i.held), inFlight, promised };
}

/** `Wrapper.refund`'s sentinel for a ticket refunded while pinned: `type(uint256).max`. */
export const UNRESOLVED = ((1n << 256n) - 1n).toString();
/** Wrapper.PIN_TIMEOUT, Wrapper.REFUND_GRACE (fixes-B-C E-03; fixes-D-canister M-1) and Wrapper.LATE_DELIVERY_WINDOW (fixes-R3-contracts §4). */
export const PIN_TIMEOUT_S = 7 * 86_400, REFUND_GRACE_S = 72 * 3_600, LATE_DELIVERY_WINDOW_S = 30 * 86_400;
/**
 * When anyone may refund a ticket, by the chain's time (the contract also waits the same span on the parent chain's blocks, so the
 * transaction may revert `TooEarly` / `NotUndeliverable` for a few minutes past this): pinned, `PIN_TIMEOUT` after its pin (the burn
 * pins it); resolved, `REFUND_GRACE` after `deliverBy` once the custodian has attested it undeliverable (`Undeliverable`), else
 * `LATE_DELIVERY_WINDOW` (30 days) after `deliverBy` (fixes-R3-contracts §4: until then a late delivery is recorded, and no refund
 * exists for it to race). The NFT wrapper (OFF, not deployed) keeps its own rules and gets null.
 */
/** Both wrappers in service gate refunds the same way (Wrapper, and EthNftWrapper: docs/v7/phase1-nft.md §3.1). */
export function refundableAt(kind: WrappedKind, t: Pick<Ticket, "stage" | "burnedAt" | "deliverBy" | "undeliverable">): number | null {
  void kind;
  if (t.stage === "pinned") return t.burnedAt === null ? null : t.burnedAt + PIN_TIMEOUT_S;
  if (t.stage === "resolved") return t.deliverBy === null ? null : t.deliverBy + (t.undeliverable ? REFUND_GRACE_S : LATE_DELIVERY_WINDOW_S);
  return null;
}

const short = (id: string) => (id.length > 16 ? `${id.slice(0, 8)}…${id.slice(-4)}` : id);
