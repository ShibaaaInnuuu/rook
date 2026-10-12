import type { Buyer, CapPolicyV, EthStateV, HeadroomV } from "./ic.js";

/**
 * docs/v8/proof-of-reserves.md §2 rows 7–9, §2.2, decision P4: the canister's own records for one Ethereum collection, in the shape
 * the reserves page shows them (indexer/src/v8/types.ts `CanisterRecords`, restated here: ops does not import the indexer's v8 code).
 * `eth_nft_state()` is an ICP QUERY — one replica's answer, not certified — so the page labels it "the canister's own records" and
 * never lets it turn anything green; the reconciler serves it at /v1/eth-records for the indexer (CANISTER_RECORDS_URL).
 */
export type CanisterRecords = {
  readAt: number; how: "query" | "update";
  buys: { id: string; collection: string; phase: string; advancedWei: string; orderId: string | null; tokenId: string | null; tx: string | null; note: string }[];
  deliveries: { ticket: number; tokenId: string; phase: string; deliverBy: number; tx: string | null }[];
  deposits: { tx: string; tokenId: string }[];
  leftoverWei: string | null;
  caps: { maxItems: number; maxValueWei: string; itemsHeld: number; itemsInFlight: number; valueWei: string } | null;
};

const hex = (b: Uint8Array | number[]) => `0x${Array.from(b, (x) => x.toString(16).padStart(2, "0")).join("")}`;
const num = (b: Uint8Array | number[]) => (b.length ? BigInt(hex(b)).toString() : "0");
const lc = (s: string) => s.toLowerCase();
type Obj = Record<string, unknown>;

/** A variant `{ Phase: payload }` → its name and payload. */
function variant(v: Record<string, unknown>): [string, Obj] { const k = Object.keys(v)[0] ?? "Unknown"; return [k, (v[k] ?? {}) as Obj]; }
const bytes = (x: unknown) => (Array.isArray(x) || x instanceof Uint8Array ? (x as Uint8Array) : null);

/** One collection's records from the canister's state and caps (pure). `collection` is the Ethereum contract address. */
export function recordsFor(state: EthStateV, caps: [CapPolicyV, HeadroomV[]] | null, collection: string, readAt: number, how: "query" | "update" = "query"): CanisterRecords {
  const mine = (b: Uint8Array | number[]) => lc(hex(b)) === lc(collection);
  const buys = state.buys.filter((b) => mine(b.collection)).map((b) => {
    const [phase, p] = variant(b.phase as Record<string, unknown>);
    const order = bytes(p.order_id), token = bytes(p.token), tx = bytes(p.tx);
    // the ETH a buy holds where it is now: what arrived on Ethereum (`wei`) once there, else the advance
    const wei = typeof p.wei === "bigint" ? p.wei : typeof p.arrived === "bigint" ? p.arrived : b.advanced;
    return { id: b.id.toString(), collection: lc(hex(b.collection)), phase, advancedWei: String(wei), orderId: order ? hex(order) : null, tokenId: token ? num(token) : null, tx: tx ? hex(tx) : null, note: b.note };
  });
  const deliveries = state.deliveries.filter((d) => mine(d.collection)).map((d) => {
    const [phase, p] = variant(d.phase as Record<string, unknown>); const tx = bytes(p.tx);
    return { ticket: Number(d.ticket), tokenId: num(d.token), phase, deliverBy: Number(d.deliver_by), tx: tx ? hex(tx) : null };
  });
  const deposits = state.deposits.filter((d) => mine(d.collection)).map((d) => ({ tx: hex(d.tx), tokenId: num(d.token) }));
  let capsOut: CanisterRecords["caps"] = null;
  if (caps) {
    const pol = caps[0].collections.find((c) => lc(c.collection) === lc(collection)), h = caps[1].find((x) => lc(x.collection) === lc(collection));
    if (pol && h) capsOut = { maxItems: pol.max_items, maxValueWei: String(pol.max_value_wei), itemsHeld: h.items_held, itemsInFlight: h.items_in_flight, valueWei: String(h.value_wei) };
  }
  // the canister's state has no per-collection leftover field (eth_out is canister-wide): not shown rather than guessed
  return { readAt, how, buys, deliveries, deposits, leftoverWei: null, caps: capsOut };
}

/**
 * Every configured EVM collection's records, by slug, from one anonymous query of each method; cached for `ttlS`. 2026-10-12: a
 * collection on another chain than the config's (a Robinhood Chain collection: `chainId` 4663) is read from that chain's side of the
 * canister (`stateOf(chainId)`: `eth_nft_state_of`); without it, or when the canister serves none, it is left out (never another
 * chain's records shown as its own).
 */
export class RecordsCache {
  private last: { at: number; v: Record<string, CanisterRecords> } | null = null;
  constructor(private readonly b: Pick<Buyer, "eth_nft_state" | "custody_caps">, private readonly collections: { slug: string; collection: string; chainId?: number }[], private readonly ttlS = 60,
    private readonly now: () => number = () => Math.floor(Date.now() / 1000), private readonly stateOf?: (chainId: number) => Promise<EthStateV | null>,
    private readonly ownChain = 1) {}
  async get(): Promise<Record<string, CanisterRecords>> {
    const t = this.now();
    if (this.last && t - this.last.at < this.ttlS) return this.last.v;
    const [state, caps] = await Promise.all([this.b.eth_nft_state(), this.b.custody_caps().catch(() => null)]);
    const states = new Map<number, EthStateV | null>([[this.ownChain, state]]);
    for (const c of this.collections) {
      const ch = c.chainId ?? this.ownChain;
      if (!states.has(ch)) states.set(ch, this.stateOf ? await this.stateOf(ch).catch(() => null) : null);
    }
    const v = Object.fromEntries(this.collections.flatMap((c) => {
      const st = states.get(c.chainId ?? this.ownChain);
      return st ? [[c.slug, recordsFor(st, caps, c.collection, t)]] : [];
    }));
    this.last = { at: t, v };
    return v;
  }
}
