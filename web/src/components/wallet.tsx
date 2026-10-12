import { useState } from "react";
import { useAccount, useConnect, useDisconnect, useSwitchChain, useWriteContract } from "wagmi";
import { useQueryClient } from "@tanstack/react-query";
import type { Abi } from "viem";
import { useEnv } from "../lib/env";
import { short } from "../lib/format";

export function Wallet() {
  const { address, isConnected } = useAccount();
  const { connect, connectors, isPending } = useConnect();
  const { disconnect } = useDisconnect();
  if (isConnected && address) return <button className="btn btn--quiet" onClick={() => disconnect()} title="Disconnect"><span className="mono">{short(address)}</span></button>;
  const c = connectors[0];
  return <button className="btn btn--quiet" disabled={!c || isPending} onClick={() => c && connect({ connector: c })}>{isPending ? "Connecting…" : "Connect wallet"}</button>;
}

/** One transaction, said in words before it is sent. It is sent by the visitor's own wallet; this site holds nothing. */
export function TxButton({ label, doing = "Sending…", request, disabledWhy, onDone }: { label: string; doing?: string; disabledWhy?: string | null;
  request: () => { address: `0x${string}`; abi: Abi; functionName: string; args?: readonly unknown[]; value?: bigint } | Promise<{ address: `0x${string}`; abi: Abi; functionName: string; args?: readonly unknown[]; value?: bigint }>;
  onDone?: () => void }) {
  const env = useEnv();
  const { isConnected, chainId } = useAccount();
  const { switchChainAsync } = useSwitchChain();
  const { writeContractAsync } = useWriteContract();
  const queries = useQueryClient();
  const [state, setState] = useState<{ busy: boolean; error?: string; hash?: string }>({ busy: false });
  if (!env || env.mode === "none") return null;
  const why = env.mode === "sample" ? "This is a recorded demo. There is nothing to send." : env.mode === "api" ? "This build has no deployment file, so it can show figures and cannot send." : disabledWhy ?? (!isConnected ? "Connect a wallet to send this." : null);
  const go = async () => {
    if (env.mode === "sample" || env.mode === "api") return;
    setState({ busy: true });
    try {
      if (chainId !== env.chain.id) await switchChainAsync({ chainId: env.chain.id });
      const r = await request();
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
      <button className="btn" disabled={state.busy || !!why} onClick={() => void go()}>{state.busy ? doing : label}</button>
      {why ? <p className="quiet tx__why">{why}</p> : null}
      {state.error ? <p className="error" role="alert">{state.error}</p> : null}
      {state.hash ? <p className="good" role="status">Confirmed. <span className="mono">{short(state.hash)}</span></p> : null}
    </div>
  );
}
