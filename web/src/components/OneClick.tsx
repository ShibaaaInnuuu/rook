import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { useAccount } from "wagmi";
import { parseEther, type PublicClient } from "viem";
import type { WrappedCollection, WrappedCoin } from "../api/types";
import { useEnv } from "../lib/env";
import { ERC20_APPROVE, ETCH_ROUTER, FROG_CURVE } from "../lib/abisV3";
import { quoteCoin, type Phase } from "../lib/quote";
import { marketVenues, type WSource } from "../lib/wrapped";
import { PHASE_LINE, QuoteSummary } from "./wx";
import { TxButton } from "./wallet";

/**
 * docs/v5/ux-and-reserves.md §1. Buy or sell a coin with ETH in one click. The phase picks the venue (the curve while it trades,
 * EtchRouter after graduation); the quote is one number; wX, the pools and the route are in Details and nowhere else.
 */
const SLIPPAGES = [50, 100, 300] as const, DEADLINES = [10, 20, 60] as const;

export function OneClick({ collection, coin, source }: { collection: WrappedCollection; coin: WrappedCoin; source: WSource }) {
  const env = useEnv();
  const { address } = useAccount();
  const [side, setSide] = useState<"buy" | "sell">("buy");
  const [amount, setAmount] = useState("0.1");
  const [slip, setSlip] = useState<number>(100);
  const [deadline, setDeadline] = useState<number>(20);
  const fixture = source.kind === "fixture";
  const live = env && (env.mode === "live" || env.mode === "demo") ? env : null;
  let wei = 0n;
  try { wei = parseEther(amount.trim() || "0"); } catch { wei = 0n; }

  const quote = useQuery({
    queryKey: ["oneclick", source.kind, coin.address, side, wei.toString(), slip],
    enabled: wei > 0n, retry: 0, staleTime: 5_000, refetchInterval: 15_000,
    queryFn: async () => {
      const venues = fixture
        ? marketVenues(collection.market, coin.address, coin.phase as Phase, coin.sweepOnly)
        : await (await import("../lib/liveQuote")).liveVenues(await client(live?.client), collection, coin);
      return { phase: venues.phase, result: await quoteCoin(side, wei, venues, slip) };
    },
  });
  const phase = (quote.data?.phase ?? coin.phase) as Phase;
  const router = collection.contracts.router;

  const allowance = useQuery({
    queryKey: ["oneclick-allow", coin.address, address, phase, router], enabled: !!live && !!address && side === "sell" && (phase === "trading" || phase === "graduated"),
    queryFn: () => live!.client.readContract({ address: coin.address, abi: ERC20_APPROVE, functionName: "allowance", args: [address!, phase === "trading" ? coin.curve : router!] }),
  });

  const r = quote.data?.result;
  const q = r && r.ok ? r.quote : null;
  const why = wei <= 0n ? "Enter an amount." : r && !r.ok ? r.why : quote.error ? `Could not quote: ${String((quote.error as Error).message).slice(0, 120)}` : !q ? "Quoting…" : null;
  const spender = phase === "trading" ? coin.curve : router;
  const needsApproval = side === "sell" && !!live && allowance.data !== undefined && allowance.data < wei;
  const deadlineAt = () => BigInt(Math.floor(Date.now() / 1000) + deadline * 60);

  return (
    <aside className="tkt oc" aria-label={`Buy or sell ${coin.symbol}`}>
      <p className={`oc__phase oc__phase--${phase}`}>{PHASE_LINE[phase]}</p>
      {phase === "refunding" ? (
        <TxButton label={`Refund ${amount || "0"} ${coin.symbol}`} doing="Refunding…" disabledWhy={wei <= 0n ? "Enter how many to refund." : null}
          request={() => ({ address: coin.curve, abi: FROG_CURVE, functionName: "refund", args: [wei] })} />
      ) : (
        <>
          <div className="tkt__sides" role="tablist" aria-label="Buy or sell">
            {(["buy", "sell"] as const).map((s) => (
              <button key={s} type="button" role="tab" aria-selected={side === s} className={`tkt__side${side === s ? " on" : ""}`} onClick={() => { setSide(s); setAmount(s === "buy" ? "0.1" : ""); }}>
                {s === "buy" ? `Buy ${coin.symbol}` : "Sell"}
              </button>
            ))}
          </div>
          <label className="tkt__amount">
            <span className="tkt__label">{side === "buy" ? "You pay, in ETH" : `You sell, in ${coin.symbol}`}</span>
            <input className="num" inputMode="decimal" autoComplete="off" spellCheck={false} value={amount} onChange={(e) => setAmount(e.target.value)} />
          </label>
          {q ? <QuoteSummary quote={q} symbol={coin.symbol} wx={collection.symbol} slippageBps={slip} deadlineMin={deadline} estimate={fixture} /> : <p className="quiet small">{why}</p>}
          <div className="tkt__slip">
            <span className="tkt__label">Slippage</span>
            <span className="chips">{SLIPPAGES.map((b) => <button key={b} type="button" className={`chip${slip === b ? " chip--on" : ""}`} aria-pressed={slip === b} onClick={() => setSlip(b)}>{(b / 100).toFixed(b % 100 ? 1 : 0)}%</button>)}</span>
          </div>
          <div className="tkt__slip">
            <span className="tkt__label">Deadline</span>
            <span className="chips">{DEADLINES.map((m) => <button key={m} type="button" className={`chip${deadline === m ? " chip--on" : ""}`} aria-pressed={deadline === m} onClick={() => setDeadline(m)}>{m} min</button>)}</span>
          </div>
          <div className="tkt__go">
            {needsApproval && spender ? (
              <TxButton label={`Allow the trade to take your ${coin.symbol}`} doing="Approving…" onDone={() => void allowance.refetch()}
                request={() => ({ address: coin.address, abi: ERC20_APPROVE, functionName: "approve", args: [spender, wei] })} />
            ) : (
              <TxButton label={side === "buy" ? `Buy ${coin.symbol}` : `Sell ${coin.symbol}`} doing={side === "buy" ? "Buying…" : "Selling…"}
                disabledWhy={why ?? (phase === "graduated" && !router ? "No router address is configured for this collection." : null)}
                onDone={() => void quote.refetch()}
                request={() => {
                  const min = q!.minOut;
                  if (phase === "trading") return side === "buy"
                    ? { address: coin.curve, abi: FROG_CURVE, functionName: "buy", args: [min], value: wei }
                    : { address: coin.curve, abi: FROG_CURVE, functionName: "sell", args: [wei, min] };
                  return side === "buy"
                    ? { address: router!, abi: ETCH_ROUTER, functionName: "buy", args: [coin.address, min, address!, deadlineAt()], value: wei }
                    : { address: router!, abi: ETCH_ROUTER, functionName: "sell", args: [coin.address, wei, min, address!, deadlineAt()] };
                }} />
            )}
          </div>
        </>
      )}
    </aside>
  );
}

/** The page's chain client when there is one; otherwise a read-only client for Robinhood Chain (quotes only, it cannot sign). */
async function client(have?: PublicClient): Promise<PublicClient> {
  if (have) return have;
  const { createPublicClient, http } = await import("viem");
  const { robinhood } = await import("../lib/chains");
  return createPublicClient({ chain: robinhood, transport: http() }) as PublicClient;
}
