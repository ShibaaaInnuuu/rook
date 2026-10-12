/**
 * A collection's opening, as the site shows it (operator decision 2026-10-12, "we don't select, the people do"): the ceremony pins every
 * collection whose pipeline works, and the canister opens one (its wX mint and metadata, then its DBC config, ≈ 0.023 SOL from its float)
 * when someone first asks for it with `coins_request_open`. A coin's launch transaction needs the collection's DBC config, which exists
 * only once the collection is Open, so the launch wizard and the collection page offer "Open it" and wait for `Open`.
 * Audit C4-12: a requested collection's words are the canister's truth (`coins_open_queue`): its place in the queue against the pace of
 * `OPENS_PER_DAY`, and, once the opening pass has waited on it a while, since when and why, not a promise of "a few minutes".
 * Pure (`openingOf`, `configOf`, `requestCopy`, `queueStatusOf`); test/opening.test.ts is its spec.
 */
import type { CollectionViewV, OpenStatusV } from "./canister";
import { COLLECTIONS, PINNED_COLLECTIONS, type SolCollection } from "./collections";

/** The collections a coin can be launched on (opened first if needed): with the canister's `coins_collections` read, every collection
 *  it has pinned (pinned, requested, opening or open; not one refused), so a Robinhood Chain collection appears once it is pinned after
 *  the ceremony's 4663 readback; without it (no canister named, or not read yet), the 38 the ceremony pins. In the site's own order. */
export function launchableOf(cols: CollectionViewV[] | undefined | null): readonly SolCollection[] {
  if (!cols || !cols.length) return PINNED_COLLECTIONS;
  const ok = new Set(cols.filter((x) => !x.stage.startsWith("Refused")).map((x) => x.symbol));
  return COLLECTIONS.filter((c) => ok.has(c.wxSymbol));
}

export type Opening =
  /** this build names no canister (or it is not read yet): the site config's own addresses are all there is */
  | { kind: "unknown" }
  /** not pinned by the canister (the Robinhood Chain ones until the ceremony's 4663 readback) */
  | { kind: "not-pinned" }
  | { kind: "pinned"; index: number }
  | { kind: "requested"; index: number }
  | { kind: "opening"; index: number }
  | { kind: "open"; index: number; config: string; wxMint: string }
  | { kind: "refused"; index: number; why: string };

/** The collection's state in `coins_collections`, found by its wX symbol (the canister's index is its own order). */
export function openingOf(cols: CollectionViewV[] | undefined | null, wxSymbol: string): Opening {
  if (!cols) return { kind: "unknown" };
  const c = cols.find((x) => x.symbol === wxSymbol);
  if (!c) return { kind: "not-pinned" };
  if (c.stage === "Open") return { kind: "open", index: c.index, config: c.config, wxMint: c.wx_mint };
  if (c.stage === "Requested") return { kind: "requested", index: c.index };
  if (c.stage === "Opening") return { kind: "opening", index: c.index };
  if (c.stage.startsWith("Refused")) return { kind: "refused", index: c.index, why: c.stage.replace(/^Refused\("?|"?\)$/g, "") };
  return { kind: "pinned", index: c.index };
}

/** The DBC config a coin launches under: the site config's (written after a ceremony that made it), else the canister's once Open. */
export function configOf(siteConfig: string | null | undefined, o: Opening): string | null {
  if (siteConfig) return siteConfig;
  return o.kind === "open" && o.config ? o.config : null;
}

/** Whether to poll `coins_collections` quickly: the collection was asked for and is not open yet. */
export const waitingToOpen = (o: Opening): boolean => o.kind === "requested" || o.kind === "opening";

/** The canister's pace: at most this many collections open a day (autobuyer/canister-rs/src/coins/state.rs `OPENS_PER_DAY`). */
export const OPENS_PER_DAY = 8;
/** Audit C4-12: a wait shorter than this is an opening in progress; longer, the site says the canister is waiting, and why. */
export const WAIT_SHOWN_AFTER_S = 5 * 60;

/** A requested collection's entry in `coins_open_queue` (null: not in it, or the queue not read). */
export function queueStatusOf(queue: OpenStatusV[] | undefined | null, o: Opening): OpenStatusV | null {
  if (!queue || (o.kind !== "requested" && o.kind !== "opening")) return null;
  return queue.find((q) => q.index === o.index) ?? null;
}

const hhmm = (s: number) => { const d = new Date(s * 1000); return `${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`; };

/** Audit C4-12: the queue's truth for a requested collection: its place ("next", or how many are ahead at the pace of OPENS_PER_DAY a
 *  day), and once the canister has waited on it over WAIT_SHOWN_AFTER_S, since when and the canister's reason. Null: nothing known. */
export function queueLine(st: OpenStatusV | null, nowS: number): string | null {
  if (!st) return null;
  const place = st.position === 0 ? "It opens next." : `${st.position} ${st.position === 1 ? "request is" : "requests are"} ahead of it; Rook opens at most ${OPENS_PER_DAY} a day.`;
  const since = Number(st.waiting_since);
  if (since > 0 && nowS - since >= WAIT_SHOWN_AFTER_S) {
    const when = nowS - since < 20 * 3_600 ? `since ${hhmm(since)}` : ((n) => `for ${n} day${n === 1 ? "" : "s"}`)(Math.max(1, Math.floor((nowS - since) / 86_400)));
    const why = (st.why.trim() || "no reason given").replace(/\.+$/, "");
    return `${place} Waiting ${when}: ${why}.${st.skipped ? " Later requests go first meanwhile; it keeps its place." : ""}`;
  }
  return st.position === 0 ? `${place} It should take a few minutes.` : place;
}

/** The words for a collection that is not open yet, by its state; for a requested one, `queue` (its `coins_open_queue` entry) gives the
 *  truth (audit C4-12), and without it the page promises nothing. */
export function requestCopy(o: Opening, name: string, chain?: string, queue?: OpenStatusV | null, nowS: number = Math.floor(Date.now() / 1000)): { title: string; body: string; action: string | null } {
  const line = queueLine(queue ?? null, nowS);
  switch (o.kind) {
    case "pinned": return { title: "Not open yet: be the first", body: `Nobody has launched a coin on ${name} yet. Opening it makes its wrapped token and its coin settings on Solana; Rook pays for it and opens at most ${OPENS_PER_DAY} a day, in the order asked.`, action: "Open it" };
    case "requested": return { title: "Opening requested", body: `${name} is in the queue to open.${line ? ` ${line}` : ""}`, action: null };
    case "opening": return { title: "Opening now", body: `${name}'s wrapped token and coin settings are being made on Solana.${line && line.includes("Waiting") ? ` ${line}` : " A few minutes more."}`, action: null };
    case "refused": return { title: "Not available", body: `${name} could not be opened (${o.why}). The operator has to look at it.`, action: null };
    case "not-pinned": return { title: "Coming", body: chain === "Robinhood Chain"
      ? `${name} is added once Rook's custodian has checked the marketplace contract it buys through on Robinhood Chain. Then anyone can open it.`
      : `${name} is not pinned by Rook's custodian yet. Once it is, anyone can open it.`, action: null };
    case "unknown": return { title: "Not open yet", body: `${name} opens the first time someone launches a coin on it.`, action: null };
    case "open": return { title: "Open", body: "", action: null };
  }
}
