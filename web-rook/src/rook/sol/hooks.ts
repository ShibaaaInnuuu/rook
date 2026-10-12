/** React Query hooks over the Solana reads. One Rpc for the page, pointed at the site config's (or VITE_SOLANA_RPC's) endpoint.
 *  Under `?demo` (src/rook/sol/demo.ts) every hook answers its fixture, in the same shape, and nothing is read from the chain. */
import { useMemo } from "react";
import { useInfiniteQuery, useQuery } from "@tanstack/react-query";
import { decodeMetadata, decodeTokenAccount } from "./accounts";
import { boardRefs, findCoin, listPools, loadRows, readConfigs, readRookRow, siteRefsOf, type ConfigRef } from "./board";
import { COLLECTIONS } from "./collections";
import { useCoinsCollections } from "./hooksCanister";
import { metadataAddress } from "./dbc";
import { DEMO, DEMO_REFS, demoConfigs, demoFindCoin, demoMeta, demoPools, demoRows } from "./demo";
import { NATIVE_MINT } from "./keys";
import { ata } from "./pda";
import { readMigratedPool, readWxPool } from "./route";
import { SOL } from "./siteConfig";
import { rpc } from "./hooksLite";
export { DEMO, rpc, SITE, useOffChain, useSolUsd } from "./hooksLite";

const k = (key: unknown[]) => (DEMO ? ["demo", ...key] : key);

/** The site config's configs whose pools are Rook coins (board.ts `siteRefsOf`: the collections the file names, then the genesis ROOK
 *  coin's). Empty before day 1 (outside demo). The board reads `useConfigRefs()`: these plus the collections the canister opened since. */
export const CONFIG_REFS: ConfigRef[] = DEMO ? DEMO_REFS : siteRefsOf(SOL);

/** The genesis ROOK coin's curve, read straight from its two known accounts (its pool and its config: one getMultipleAccounts, which
 *  the keyless RPC serves, unlike the board's getProgramAccounts), for the home page's ROOK card when the board has no row for it.
 *  Off in demo (the demo board has ROOK) and before ROOK's addresses are in the site config. */
export function useRookRow(enabled: boolean) {
  const r = SOL.ready ? SOL.rook : null;
  return useQuery({ queryKey: ["rook-row", r?.pool ?? null], enabled: enabled && !DEMO && !!r, refetchInterval: 15_000, staleTime: 10_000, retry: 1,
    queryFn: () => readRookRow(rpc(), r!) });
}

/** CONFIG_REFS merged with the canister's Open collections (`coins_collections`, their DBC configs; src/rook/sol/board.ts `boardRefs`),
 *  so a coin launched on a collection opened on demand is on the board. Without a canister (or before it answers, or if it fails) the
 *  static list alone; demo reads no canister. */
export function useConfigRefs(): ConfigRef[] {
  const cols = useCoinsCollections();
  const data = DEMO ? undefined : cols.data;
  return useMemo(() => boardRefs(CONFIG_REFS, data, COLLECTIONS), [data]);
}
const configsOf = (refs: ConfigRef[]) => refs.map((r) => r.config);

export function useConfigs() {
  const refs = useConfigRefs();
  return useQuery({ queryKey: k(["sol-configs", configsOf(refs)]), enabled: refs.length > 0, staleTime: Infinity, refetchInterval: false, queryFn: DEMO ? async () => demoConfigs() : () => readConfigs(rpc(), refs) });
}
export function usePoolList(configs: string[]) {
  return useQuery({ queryKey: k(["sol-pools", configs]), enabled: configs.length > 0, refetchInterval: 30_000, staleTime: 15_000, queryFn: DEMO ? async () => demoPools(configs) : () => listPools(rpc(), configs) });
}
export const PAGE = 100;
/** The board's rows, a page of 100 pools at a time, newest first. `enabled` false: nothing is read (the indexer or demo answers). */
export function useRows(enabled: boolean) {
  const refs = useConfigRefs();
  const configs = useMemo(() => (enabled ? configsOf(refs) : []), [enabled, refs]);
  const labels = useMemo(() => new Map(refs.map((r) => [r.config, r])), [refs]);
  const list = usePoolList(configs), cfg = useConfigs();
  const rows = useInfiniteQuery({
    queryKey: k(["sol-rows", configs, list.data?.length ?? 0]), enabled: !!list.data && !!cfg.data, initialPageParam: 0, refetchInterval: 30_000,
    queryFn: ({ pageParam }) => { const page = list.data!.slice(pageParam, pageParam + PAGE); return DEMO ? Promise.resolve(demoRows(page)) : loadRows(rpc(), page, cfg.data!, labels); },
    getNextPageParam: (_last, all) => (all.length * PAGE < (list.data?.length ?? 0) ? all.length * PAGE : undefined),
  });
  return { list, cfg, rows };
}
export function useCoin(mint: string | undefined) {
  const refs = useConfigRefs();
  return useQuery({ queryKey: k(["sol-coin", mint, refs.length]), enabled: !!mint && refs.length > 0, refetchInterval: 10_000, queryFn: DEMO ? async () => demoFindCoin(mint!) : () => findCoin(rpc(), mint!, refs) });
}
/** A coin's Metaplex metadata (name, symbol, URI of its JSON). */
export function useCoinMeta(mint: string) {
  const q = useQuery({ queryKey: k(["sol-meta", mint]), staleTime: Infinity, refetchInterval: false,
    queryFn: DEMO ? async () => demoMeta(mint) : async () => { const a = await rpc().account(metadataAddress(mint)); return a ? decodeMetadata(a.data) : null; } });
  return q.data ?? null;
}
/** The connected wallet's SOL, its balance of one coin, and (a wX-quoted coin) of the collection's wX. Never in demo: a demo page reads no wallet. */
export function useBalances(owner: string | null, mint: string | undefined, wx?: string | null) {
  return useQuery({
    queryKey: ["sol-balances", owner, mint, wx ?? null], enabled: !!owner && !DEMO, refetchInterval: 15_000,
    queryFn: async () => {
      const [w, t, x] = await rpc().accounts([owner!, ata(owner!, mint ?? NATIVE_MINT), ata(owner!, wx ?? NATIVE_MINT)]);
      return { sol: BigInt(w?.lamports ?? 0), coin: mint && t ? decodeTokenAccount(t.data).amount : 0n, wx: wx && x ? decodeTokenAccount(x.data).amount : 0n };
    },
  });
}
/** The collection's wX/SOL pool (a Meteora DLMM pair, docs/v9/canister-only-solana.md §17) with the bins around its active one: the
 *  first hop of the site's own buy (src/rook/sol/route.ts). null while no pair exists for this wX (pools open from fees). Not in demo. */
export function useWxPool(wx: string | null | undefined) {
  return useQuery({ queryKey: ["sol-wxpool", wx], enabled: !!wx && wx !== NATIVE_MINT && !DEMO, refetchInterval: 15_000, staleTime: 5_000, queryFn: () => readWxPool(rpc(), wx!) });
}
/** A graduated coin's DAMM v2 COIN/wX pool (the second hop after graduation); null until the curve has migrated. Not in demo. */
export function useMigratedPool(coin: string | undefined, wx: string | null | undefined, enabled: boolean) {
  return useQuery({ queryKey: ["sol-dammpool", coin, wx], enabled: enabled && !!coin && !!wx && !DEMO, refetchInterval: 10_000, queryFn: () => readMigratedPool(rpc(), coin!, wx!) });
}
