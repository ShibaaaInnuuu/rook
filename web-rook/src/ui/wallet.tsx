import { useState } from "react";
import { useAccount, useConnect, useDisconnect, useSwitchChain, useWriteContract } from "wagmi";
import { useQueryClient } from "@tanstack/react-query";
import type { Abi } from "viem";
import { useEnv } from "../lib/env";
import { short } from "../lib/format";
import { PHASE1, requestRefusal, sendRefusal, type StackReader } from "../rook/phase";
import { bundledStack } from "../rook/stack";

/** docs/v6/phase1-scope.md §8.3 and E-07 (docs/audit/2026-09-26/fixes-W2-site.md §2): in a Phase-1 build the target, the function
 *  and every address among the arguments are checked (the target on chain) before the wallet is asked. */
const ROUTER_OF = [{ type: "function", name: "routerOf", stateMutability: "view", inputs: [{ name: "coin", type: "address" }], outputs: [{ type: "address" }] }] as const;
const TOKEN = [{ type: "function", name: "token", stateMutability: "view", inputs: [], outputs: [{ type: "address" }] }] as const;
const readerOf = (client: { readContract: (a: never) => Promise<unknown> }): StackReader => ({
  routerOf: async (factory, coin) => String(await client.readContract({ address: factory, abi: ROUTER_OF, functionName: "routerOf", args: [coin] } as never)),
  tokenOf: async (curve) => { try { return String(await client.readContract({ address: curve, abi: TOKEN, functionName: "token" } as never)); } catch { return null; } },
});

/** The one connector web/ offers: whatever wallet the browser has. No relay, no third-party request. */
export function Wallet() {
  const { address, isConnected } = useAccount();
  const { connect, connectors, isPending } = useConnect();
  const { disconnect } = useDisconnect();
  if (isConnected && address) return <button className="btn btn--quiet btn--small" onClick={() => disconnect()} title="Disconnect"><span className="mono">{short(address)}</span></button>;
  const c = connectors[0];
  return <button className="btn btn--quiet btn--small" disabled={!c || isPending} onClick={() => c && connect({ connector: c })}>{isPending ? "Connecting…" : "Connect wallet"}</button>;
}

type Req = { address: `0x${string}`; abi: Abi; functionName: string; args?: readonly unknown[]; value?: bigint };
/** One transaction, said in words before it is sent. The visitor's own wallet sends it; this site holds nothing. */
/** `coin`: the coin the page is about (its URL). A request to a coin or a curve, or a router trade, must be for it (R2 N-2). */
export function TxButton({ label, doing = "Sending…", request, disabledWhy, primary = true, block, onDone, coin = null }: { label: string; doing?: string; disabledWhy?: string | null; primary?: boolean; block?: boolean;
  request: () => Req | Promise<Req>; onDone?: () => void; coin?: string | null }) {
  const env = useEnv();
  const { isConnected, chainId, address: account } = useAccount();
  const { switchChainAsync } = useSwitchChain();
  const { writeContractAsync } = useWriteContract();
  const queries = useQueryClient();
  const [state, setState] = useState<{ busy: boolean; error?: string; hash?: string }>({ busy: false });
  if (!env || env.mode === "none") return null;
  // Phase 1: nothing is sent without the bundled 4663 stack file, and never from a page reading another chain
  const phase1 = PHASE1 && env.mode !== "sample" ? sendRefusal(bundledStack, env.mode === "api" ? null : env.chain.id) : null;
  const why = env.mode === "sample" ? "This is a fixture. There is nothing to send." : phase1 ?? (env.mode === "api" ? "This build has no deployment file, so it can show figures and cannot send." : disabledWhy ?? (!isConnected ? "Connect a wallet to send this." : null));
  const go = async () => {
    if (env.mode === "sample" || env.mode === "api" || phase1) return;
    setState({ busy: true });
    try {
      const r = await request();
      if (PHASE1) {
        const bad = await requestRefusal(bundledStack.stack!, r, account ?? null, readerOf(env.client as never), coin);
        if (bad) { setState({ busy: false, error: bad }); return; }
      }
      if (chainId !== env.chain.id) await switchChainAsync({ chainId: env.chain.id });
      const hash = await writeContractAsync({ ...r, chainId: env.chain.id } as never);
      await env.client.waitForTransactionReceipt({ hash });
      setState({ busy: false, hash });
      await queries.invalidateQueries();
      onDone?.();
    } catch (e) {
      const m = String((e as { shortMessage?: string; message?: string }).shortMessage ?? (e as Error).message ?? e);
      setState({ busy: false, error: /rejected|denied/i.test(m) ? "You declined it in your wallet. Nothing was sent." : m.slice(0, 220) });
    }
  };
  return (
    <div className="tx">
      <button className={`btn${primary ? " btn--primary" : ""}${block ? " btn--block" : ""}`} disabled={state.busy || !!why} onClick={() => void go()}>{state.busy ? doing : label}</button>
      {why ? <p className="muted small" style={{ marginTop: ".5rem" }}>{why}</p> : null}
      {state.error ? <p className="error small" role="alert">{state.error}</p> : null}
      {state.hash ? <p className="good small" role="status">Confirmed. <span className="mono">{short(state.hash)}</span></p> : null}
    </div>
  );
}
