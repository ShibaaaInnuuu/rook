/** The landing page's hooks: the RPC, SOL/USD, a coin's metadata JSON. No curve code, no PDA search.
 *  Under `?demo` (src/rook/sol/demo.ts) each answers its fixture instead and reads nothing from the chain. */
import { useQuery } from "@tanstack/react-query";
import { solUsd } from "./accounts";
import { DEMO, DEMO_SOL, DEMO_SOL_USD, demoOffChain } from "./demo";
import { fetchOffChain } from "./offchain";
import { Rpc } from "./rpc";
import { SOL, type SolanaConfig } from "./siteConfig";

let rpcOne: Rpc | null = null;
export const rpc = (): Rpc => (rpcOne ??= new Rpc(SOL.rpcUrl));
/** The site config the pages read: src/config/solana.ts's, or under `?demo` the same with made-up day-1 addresses. Never the
 *  config a transaction is built from: the launch and trade code read SOL itself, and demo builds no transaction. */
export const SITE: SolanaConfig = DEMO ? DEMO_SOL : SOL;
export { DEMO };
const k = (key: unknown[]) => (DEMO ? ["demo", ...key] : key);

export function useSolUsd() {
  return useQuery({ queryKey: k(["sol-usd"]), refetchInterval: 60_000, staleTime: 30_000, queryFn: DEMO ? async () => DEMO_SOL_USD : () => solUsd(rpc()) });
}
export function useOffChain(uri: string | null | undefined) {
  return useQuery({ queryKey: k(["sol-offchain", uri]), enabled: !!uri, staleTime: Infinity, refetchInterval: false, retry: 0, queryFn: DEMO ? async () => demoOffChain(uri!) : () => fetchOffChain(uri!) });
}
