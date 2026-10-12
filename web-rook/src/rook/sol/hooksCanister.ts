/** React Query hooks over the Rook canister's public queries (src/rook/sol/canister.ts) and, for the Reserves page, the chain side at
 *  finalized through the site's RPC. Every hook is off while this build names no canister. Under `?demo` they read the demo's canister
 *  (src/rook/sol/demoCanister.ts: made-up books, nothing sent) and its chain side, never the network. */
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { decodeMint, decodeTokenAccount } from "./accounts";
import { chainAddresses, collectionEquation, moduleOf, type ChainSide, type Equation } from "./booksReserves";
import { canister, HAS_CANISTER, stateOf, type BooksViewV, type CollectionViewV, type IntentViewV, type RookCanister, type VaultViewV } from "./canister";
import { DEMO } from "./demo";
import { demoCanister, demoChainSide } from "./demoCanister";
import { rpc } from "./hooksLite";
import { openingOf, queueStatusOf, waitingToOpen, type Opening } from "./opening";

/** A live canister is read: this build names one and it is not the demo. */
export const CANISTER_LIVE = HAS_CANISTER && !DEMO;
/** The canister's hooks are on: a live canister, or the demo's. */
export const CANISTER_ON = CANISTER_LIVE || DEMO;
/** The canister the hooks read: the demo's under `?demo`, else the site's (src/rook/sol/canister.ts). */
export const ic = (): Promise<RookCanister> => (DEMO ? Promise.resolve(demoCanister()) : canister());

export function useBooksCollections() {
  return useQuery({ queryKey: ["ic-books-collections"], enabled: CANISTER_ON, staleTime: 60_000, refetchInterval: 120_000, queryFn: async () => (await ic()).books_collections() });
}
export function useBooksView(coll: number | null) {
  return useQuery({ queryKey: ["ic-books-view", coll], enabled: CANISTER_ON && coll !== null, refetchInterval: 30_000, staleTime: 10_000,
    queryFn: async () => { const r = await (await ic()).books_view(coll!); return r.length ? r[0] : null; } });
}
export function useDeskStatus() {
  return useQuery({ queryKey: ["ic-desk-status"], enabled: CANISTER_ON, refetchInterval: 30_000, staleTime: 10_000, queryFn: async () => (await ic()).desk_status() });
}
export function useCoinsCollections(fast = false) {
  return useQuery({ queryKey: ["ic-coins-collections"], enabled: CANISTER_ON, refetchInterval: fast ? 8_000 : 60_000, staleTime: fast ? 4_000 : 30_000,
    queryFn: async () => (await ic()).coins_collections() });
}
/** Audit C4-12: the opening queue (`coins_open_queue`): each pending request's place and what the canister waits on for it. Read only
 *  while a collection on the page was asked for; a canister without the query answers nothing (the page then promises nothing). */
export function useOpenQueue(enabled: boolean) {
  return useQuery({ queryKey: ["ic-coins-open-queue"], enabled: CANISTER_ON && enabled, refetchInterval: 8_000, staleTime: 4_000, retry: 0,
    queryFn: async () => (await ic()).coins_open_queue() });
}
/** A collection's opening (src/rook/sol/opening.ts) by its wX symbol, polled every 8 s while it was asked for and is not open yet, with
 *  its `coins_open_queue` entry (`queue`, audit C4-12), and `request()`: `coins_request_open` as an anonymous caller (anyone may ask;
 *  the canister may refuse, e.g. its hourly limits, a full queue, the module off: the Err is the error shown). */
export function useOpening(wxSymbol: string | null | undefined) {
  const first = useCoinsCollections();
  const o = wxSymbol ? openingOf(first.data, wxSymbol) : ({ kind: "unknown" } as Opening);
  const q = useCoinsCollections(waitingToOpen(o));
  const qc = useQueryClient();
  const request = useMutation({
    mutationFn: async () => {
      if (o.kind !== "pinned" && o.kind !== "requested") throw new Error("this collection cannot be asked to open now");
      const r = await (await ic()).coins_request_open(o.index);
      if ("Err" in r) throw new Error(r.Err);
      return r.Ok;
    },
    onSettled: () => qc.invalidateQueries({ queryKey: ["ic-coins-collections"] }),
  });
  const opening = wxSymbol ? openingOf(q.data, wxSymbol) : o;
  const queue = useOpenQueue(waitingToOpen(opening));
  return { opening, queue: queueStatusOf(queue.data, opening), loading: CANISTER_ON && q.isLoading, request };
}
/** The canister's index of a collection, by its wX symbol in `coins_collections` (the canister pins collections in its unlock order,
 *  `coins::unlock::ORDER`, which is NOT the site's listing order: BAYC is 0 there). `null` until the canister has pinned it, or when
 *  this build names no canister. `coinsCollectionIndex` is the pure rule; the hook reads the list. */
export function coinsCollectionIndex(cols: CollectionViewV[] | undefined, wxSymbol: string): number | null {
  const row = cols?.find((x) => x.symbol === wxSymbol);
  return row ? row.index : null;
}
export function useCanisterIndex(wxSymbol: string | null | undefined): { coll: number | null; loading: boolean } {
  const q = useCoinsCollections();
  return { coll: wxSymbol ? coinsCollectionIndex(q.data, wxSymbol) : null, loading: CANISTER_ON && q.isLoading };
}
export function useCoinsVaults(coll: number | null) {
  return useQuery({ queryKey: ["ic-coins-vaults", coll], enabled: CANISTER_ON && coll !== null, refetchInterval: 60_000, staleTime: 30_000, queryFn: async () => (await ic()).coins_vaults(coll!) });
}
export function useTreasuryStatus() {
  return useQuery({ queryKey: ["ic-treasury"], enabled: CANISTER_ON, refetchInterval: 60_000, staleTime: 30_000, queryFn: async () => (await ic()).treasury_status() });
}
export function usePoolDesks() {
  return useQuery({ queryKey: ["ic-pool-desks"], enabled: CANISTER_ON, refetchInterval: 60_000, staleTime: 30_000, queryFn: async () => (await ic()).pool_desks() });
}
/** A mint quote for `lamports` of collection `coll` (null: no quote, with the canister's reason in `error`). */
export function useQuoteMint(coll: number | null, lamports: bigint | null) {
  return useQuery({ queryKey: ["ic-quote-mint", coll, lamports?.toString() ?? null], enabled: CANISTER_ON && coll !== null && !!lamports && lamports > 0n, staleTime: 10_000, retry: 0,
    queryFn: async () => { const r = await (await ic()).quote_mint(coll!, lamports!); if ("Err" in r) throw new Error(r.Err); return r.Ok; } });
}
export function useQuoteBurn(coll: number | null, units: bigint | null) {
  return useQuery({ queryKey: ["ic-quote-burn", coll, units?.toString() ?? null], enabled: CANISTER_ON && coll !== null && !!units && units > 0n, staleTime: 10_000, retry: 0,
    queryFn: async () => { const r = await (await ic()).quote_burn(coll!, units!); if ("Err" in r) throw new Error(r.Err); return r.Ok; } });
}
const FINAL = new Set(["Done", "Refunded", "Expired"]);
/** An intent, polled every 5 s until it is final (then every minute: a later deposit reopens it). */
export function useIntent(id: bigint | null) {
  return useQuery({ queryKey: ["ic-intent", id?.toString() ?? null], enabled: CANISTER_ON && id !== null, staleTime: 2_000,
    refetchInterval: (q) => { const v = q.state.data as IntentViewV | null | undefined; return v && FINAL.has(stateOf(v)) ? 60_000 : 5_000; },
    queryFn: async () => { const r = await (await ic()).intent_status(id!); return r.length ? r[0] : null; } });
}

/** The chain side of one collection's equation, at finalized: the wX mint, the module, each coin vault; null where a read fails. */
export async function readChainSide(v: BooksViewV, module: string | null, vaults: VaultViewV[]): Promise<ChainSide> {
  const r = rpc();
  const addrs = chainAddresses(v, module, vaults);
  const [rent, accounts] = await Promise.all([r.rentExempt(0), r.accounts(addrs, "finalized").catch(() => addrs.map(() => null))]);
  const at = (a: string) => accounts[addrs.indexOf(a)] ?? null;
  const mint = at(v.wx_mint);
  let supply: bigint | null = null;
  try { supply = mint ? decodeMint(mint.data).supply : null; } catch { supply = null; }
  const m = module ? at(module) : null;
  const vaultsRead = new Map<string, bigint | null>();
  for (const x of vaults) {
    if (!x.vault.length) continue;
    const a = at(x.vault[0]);
    try { vaultsRead.set(x.vault[0], a ? decodeTokenAccount(a.data).amount : null); } catch { vaultsRead.set(x.vault[0], null); }
  }
  return { supply, moduleLamports: module ? (m ? BigInt(m.lamports) : null) : null, rent, vaults: vaultsRead };
}

export type ReservesRead = { equations: Equation[]; views: Map<number, BooksViewV>; cols: CollectionViewV[]; at: number; errors: string[] };
/** Every collection's equation: the books from the canister, the chain at finalized. Collections the canister cannot serve are reported, not dropped. */
export async function readReserves(nowS: number): Promise<ReservesRead> {
  const c = await ic();
  const [ids, cols] = await Promise.all([c.books_collections(), c.coins_collections()]);
  const equations: Equation[] = [], views = new Map<number, BooksViewV>(), errors: string[] = [];
  for (const [coll] of ids) {
    try {
      const [vr, vaults] = await Promise.all([c.books_view(coll), c.coins_vaults(coll).catch(() => [] as VaultViewV[])]);
      if (!vr.length) { errors.push(`the canister lists collection ${coll} but serves no books for it`); continue; }
      const v = vr[0]; views.set(coll, v);
      const module = moduleOf(cols, coll);
      equations.push(collectionEquation(v, module, vaults, DEMO ? demoChainSide(v, vaults) : await readChainSide(v, module, vaults)));
    } catch (e) { errors.push(`collection ${coll}: ${String((e as Error).message ?? e).slice(0, 160)}`); }
  }
  return { equations, views, cols, at: nowS, errors };
}
export function useReserves() {
  return useQuery({ queryKey: ["ic-reserves"], enabled: CANISTER_ON, refetchInterval: 120_000, staleTime: 60_000, queryFn: () => readReserves(Math.floor(Date.now() / 1000)) });
}
