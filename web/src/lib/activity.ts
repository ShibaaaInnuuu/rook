/**
 * What is happening across the whole site, merged from the lists the indexer already serves.
 *
 * There is no `/v1/activity` and no site-wide `/v1/trades`, so this is assembled in the browser from launches, harvests,
 * purchases, deliveries and draws. Buys and sells are therefore **missing from it**: trades are served per coin only, and
 * fetching every coin's trades to build one feed would be dozens of requests for one list. What the indexer should add is
 * written down in docs/phase9.md, section 10.
 *
 * Every event carries the second it happened, as the indexer recorded it. An event whose time is unknown is left out
 * rather than floated to the top.
 */
import type * as Api from "../api/types";
import type { CoinRow } from "./coinlist";

export type Event =
  | { kind: "launch"; id: string; at: number; coin: CoinRow }
  | { kind: "harvest"; id: string; at: number; coin: CoinRow }
  | { kind: "purchase"; id: string; at: number; a: Api.Acquisition }
  | { kind: "delivery"; id: string; at: number; a: Api.Acquisition }
  | { kind: "draw"; id: string; at: number; d: Api.DrawSummary };

export const KINDS: { key: Event["kind"] | "all"; label: string }[] = [
  { key: "all", label: "Everything" },
  { key: "launch", label: "Launches" },
  { key: "harvest", label: "Harvests" },
  { key: "purchase", label: "Purchases" },
  { key: "delivery", label: "Deliveries" },
  { key: "draw", label: "Draws" },
];

/** One list, newest first, out of every list the API gives. */
export function mergeActivity(
  coins: CoinRow[],
  acquisitions: Api.Acquisition[],
  draws: Api.DrawSummary[],
  harvestAt: (c: CoinRow) => number | null = () => null,
): Event[] {
  const out: Event[] = [];

  for (const c of coins) {
    if (c.launchedAt) out.push({ kind: "launch", id: `launch-${c.address}`, at: c.launchedAt, coin: c });
    const h = harvestAt(c);
    if (h && c.harvests) out.push({ kind: "harvest", id: `harvest-${c.address}-${h}`, at: h, coin: c });
  }

  for (const a of acquisitions) {
    if (a.settledAt) out.push({ kind: "purchase", id: `buy-${a.listingId}-${a.acquisitionId}`, at: a.settledAt, a });
    // A delivery has no time of its own in the API, so it is placed at its purchase and labelled, never invented later.
    if (a.delivery.deliveredTxid && a.settledAt) {
      out.push({ kind: "delivery", id: `del-${a.listingId}-${a.acquisitionId}`, at: a.settledAt, a });
    }
  }

  for (const d of draws) {
    if (d.openedAt) out.push({ kind: "draw", id: `draw-${d.id}`, at: d.openedAt, d });
  }

  return out.sort((x, y) => y.at - x.at || x.id.localeCompare(y.id));
}

export const ofKind = (events: Event[], kind: Event["kind"] | "all"): Event[] =>
  kind === "all" ? events : events.filter((e) => e.kind === kind);
