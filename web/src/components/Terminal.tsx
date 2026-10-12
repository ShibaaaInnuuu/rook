import { useEffect, useMemo, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { useAccount } from "wagmi";
import { formatEther, parseEther } from "viem";
import { useEnv } from "../lib/env";
import { ERC20_TRADE, FACTORY, PONS_CURVE } from "../lib/abis";
import { eth } from "../lib/format";
import { coins, price } from "../lib/chart";
import { TxButton } from "./wallet";

/**
 * The ticket: what a trader is about to sign, with nothing left to guess.
 *
 * Every figure below the amount comes from simulating the exact call the button will send, so the quote is the curve's own
 * answer rather than our arithmetic about it. What cannot be simulated is not estimated: it says so. A control that cannot
 * be pressed says why on itself rather than being greyed out in silence.
 *
 * Trading lives on staging, where the fork's ETH is worth nothing and fees exist so a harvest can be tested. On any other
 * network the same terminal is drawn and disabled with the reason, because a hole in the page teaches nobody anything.
 * docs/phase6.md, docs/phase9.md section 10.
 */

const SLIPPAGES = [50, 100, 300] as const; // basis points
/** Robinhood Chain settles in ETH, and so does the fork. */
const NATIVE = "ETH";
const PONS_FEE_BPS = 100n; // Pons takes one per cent of the quote, curve-side

type Side = "buy" | "sell";

function Row({ label, value, tone }: { label: string; value: React.ReactNode; tone?: "warn" | "quiet" }) {
  return (
    <div className={`tkt__row${tone ? ` tkt__row--${tone}` : ""}`}>
      <span className="tkt__label">{label}</span>
      <span className="tkt__value num">{value}</span>
    </div>
  );
}

export function Terminal({ coin, curve, symbol, creatorTaxBps, lastPrice, graduated }: {
  coin: `0x${string}`; curve: `0x${string}`; symbol: string;
  creatorTaxBps: number; lastPrice: number | null; graduated: boolean;
}) {
  const env = useEnv();
  const { address, chainId } = useAccount();
  const [side, setSide] = useState<Side>("buy");
  const [amount, setAmount] = useState("0.05");
  const [slip, setSlip] = useState<number>(100);

  const live = env && env.mode === "live" ? env : null;
  const staging = env?.net === "staging";
  const wrongChain = !!live && !!chainId && chainId !== live.deployment.chainId;

  /** What the wallet holds, on both sides of the trade. */
  const held = useQuery({
    queryKey: ["tkt-held", coin, address, live?.deployment.chainId],
    enabled: !!live && !!address,
    refetchInterval: 8_000,
    queryFn: async () => {
      const [tokens, allowance, native] = await Promise.all([
        live!.client.readContract({ address: coin, abi: ERC20_TRADE, functionName: "balanceOf", args: [address!] }),
        live!.client.readContract({ address: coin, abi: ERC20_TRADE, functionName: "allowance", args: [address!, curve] }),
        live!.client.getBalance({ address: address! }),
      ]);
      return { tokens, allowance, native };
    },
  });

  /** How near the curve is to graduating: what it holds against the threshold Pons set for it. */
  const curveState = useQuery({
    queryKey: ["tkt-curve", coin, curve, live?.deployment.chainId],
    enabled: !!live && !graduated,
    refetchInterval: 15_000,
    queryFn: async () => {
      const t = await live!.client.readContract({
        address: live!.deployment.ponsFactory, abi: FACTORY, functionName: "getLaunchedToken", args: [coin],
      });
      const threshold = t[5];
      const pair = t[4];
      const inCurve = pair === "0x0000000000000000000000000000000000000000"
        ? await live!.client.getBalance({ address: curve })
        : await live!.client.readContract({ address: pair, abi: ERC20_TRADE, functionName: "balanceOf", args: [curve] });
      return { threshold, inCurve };
    },
  });

  let wei = 0n;
  try { wei = parseEther(amount || "0"); } catch { wei = 0n; }
  const sellingTokens = side === "sell" ? wei : 0n;
  const spending = side === "buy" ? wei : 0n;

  /** The curve's own answer to the exact call the button will send. Nothing here is our arithmetic. */
  const quote = useQuery({
    queryKey: ["tkt-quote", coin, side, wei.toString(), address],
    enabled: !!live && !!address && wei > 0n && !wrongChain,
    retry: 0,
    staleTime: 4_000,
    queryFn: async (): Promise<{ out: bigint } | { error: string }> => {
      try {
        const { result } = side === "buy"
          ? await live!.client.simulateContract({ address: curve, abi: PONS_CURVE, functionName: "buy", args: [spending, 0n, address!], value: spending, account: address! })
          : await live!.client.simulateContract({ address: curve, abi: PONS_CURVE, functionName: "sell", args: [sellingTokens, 0n, address!], account: address! });
        return { out: result as bigint };
      } catch (e) {
        const m = String((e as Error)?.message ?? e);
        return { error: m.split("\n")[0].slice(0, 120) };
      }
    },
  });

  const out = quote.data && "out" in quote.data ? quote.data.out : null;
  const quoteErr = quote.data && "error" in quote.data ? quote.data.error : null;
  const minOut = out !== null ? (out * BigInt(10_000 - slip)) / 10_000n : null;

  const ponsFee = side === "buy" ? (spending * PONS_FEE_BPS) / 10_000n : 0n;
  const creatorTax = side === "buy" ? (spending * BigInt(creatorTaxBps)) / 10_000n : 0n;

  /** Execution against the last trade: how much this order itself moves the price. */
  const impact = useMemo(() => {
    if (out === null || lastPrice === null || lastPrice <= 0) return null;
    const exec = side === "buy"
      ? Number(formatEther(spending)) / (Number(out) / 1e18)
      : (Number(out) / 1e18) / Number(formatEther(sellingTokens));
    if (!Number.isFinite(exec) || exec <= 0) return null;
    return (exec - lastPrice) / lastPrice;
  }, [out, lastPrice, side, spending, sellingTokens]);

  const max = side === "buy" ? held.data?.native ?? 0n : held.data?.tokens ?? 0n;
  const setFraction = (num: bigint, den: bigint) => {
    if (max <= 0n) return;
    // A buy can never spend the whole balance: the transaction itself has to be paid for.
    const room = side === "buy" ? (max * 98n) / 100n : max;
    setAmount(formatEther((room * num) / den));
  };

  useEffect(() => { setAmount(side === "buy" ? "0.05" : "0"); }, [side]);

  const why = !live ? "Nothing is deployed on this network, so there is nothing to trade."
    : !staging ? "This is not staging. A Pons coin is traded on Pons; this ticket exists on the fork so testers can make the fees a harvest needs."
    : !address ? "Connect a wallet to trade."
    : wrongChain ? `Your wallet is on another network. Switch it to chain ${live.deployment.chainId} to sign this.`
    : wei <= 0n ? "Enter an amount."
    : side === "buy" && held.data && spending > held.data.native ? "That is more than the wallet holds."
    : side === "sell" && held.data && sellingTokens > held.data.tokens ? `That is more ${symbol} than the wallet holds.`
    : null;

  const needsApproval = side === "sell" && !!held.data && held.data.allowance < sellingTokens && sellingTokens > 0n;
  const gradShare = curveState.data && curveState.data.threshold > 0n
    ? Math.min(1, Number((curveState.data.inCurve * 10_000n) / curveState.data.threshold) / 10_000)
    : null;

  return (
    <aside className="tkt" aria-label={`Trade ${symbol}`}>
      <div className="tkt__sides" role="tablist" aria-label="Buy or sell">
        {(["buy", "sell"] as Side[]).map((s) => (
          <button key={s} type="button" role="tab" aria-selected={side === s} className={`tkt__side tkt__side--${s}${side === s ? " on" : ""}`} onClick={() => setSide(s)}>{s}</button>
        ))}
      </div>

      <label className="tkt__amount">
        <span className="tkt__label">{side === "buy" ? `Pay, in ${NATIVE}` : `Sell, in ${symbol}`}</span>
        <input className="num" inputMode="decimal" autoComplete="off" spellCheck={false} value={amount} onChange={(e) => setAmount(e.target.value)} />
      </label>

      <div className="tkt__fracs">
        {([[1n, 4n, "25%"], [1n, 2n, "50%"], [1n, 1n, "max"]] as const).map(([n, d, label]) => (
          <button key={label} type="button" className="chip" disabled={max <= 0n} onClick={() => setFraction(n, d)}>{label}</button>
        ))}
        <span className="tkt__have quiet">
          {held.data
            ? side === "buy"
              ? <>{eth(held.data.native, 4)} held</>
              : <>{coins(held.data.tokens.toString())} {symbol} held</>
            : address ? "reading the wallet…" : "no wallet"}
        </span>
      </div>

      <div className="tkt__out">
        <Row
          label={side === "buy" ? "You receive" : "You receive"}
          value={out !== null
            ? side === "buy" ? <>{coins(out.toString())} <span className="tkt__unit">{symbol}</span></> : <>{eth(out, 6)} <span className="tkt__unit">{NATIVE}</span></>
            : quoteErr ? <span className="warn-text">the curve refused this order</span>
            : quote.isFetching ? "quoting…" : "—"}
        />
        <Row label={`Least after ${(slip / 100).toFixed(slip % 100 ? 2 : 0)}% slippage`} value={minOut !== null ? (side === "buy" ? `${coins(minOut.toString())} ${symbol}` : `${eth(minOut, 6)}`) : "—"} />
        <Row label="Price it fills at" value={out !== null && wei > 0n ? price(side === "buy" ? Number(formatEther(spending)) / (Number(out) / 1e18) : (Number(out) / 1e18) / Number(formatEther(sellingTokens))) : "—"} />
        <Row label="Impact on the price" value={impact === null ? "—" : <span className={Math.abs(impact) > 0.05 ? "warn-text" : undefined}>{`${impact > 0 ? "+" : ""}${(impact * 100).toFixed(2)}%`}</span>} />
        <Row label="Pons fee, 1%" value={side === "buy" ? eth(ponsFee, 6) : "taken in coin"} tone="quiet" />
        <Row label={`Creator tax, ${(creatorTaxBps / 100).toFixed(creatorTaxBps % 100 ? 2 : 0)}%`} value={side === "buy" ? eth(creatorTax, 6) : "taken in coin"} tone="quiet" />
      </div>

      <div className="tkt__slip">
        <span className="tkt__label">Slippage</span>
        <span className="chips">
          {SLIPPAGES.map((b) => (
            <button key={b} type="button" className={`chip${slip === b ? " chip--on" : ""}`} aria-pressed={slip === b} onClick={() => setSlip(b)}>{(b / 100).toFixed(b % 100 ? 1 : 0)}%</button>
          ))}
        </span>
      </div>

      <div className="tkt__go">
        {needsApproval ? (
          <TxButton label={`Approve the curve to take ${symbol}`} doing="Approving…" disabledWhy={why} onDone={() => void held.refetch()}
            request={() => ({ address: coin, abi: ERC20_TRADE, functionName: "approve", args: [curve, held.data!.tokens] })} />
        ) : (
          <TxButton
            label={side === "buy" ? `Buy ${symbol}` : `Sell ${symbol}`}
            doing={side === "buy" ? "Buying…" : "Selling…"}
            disabledWhy={why ?? (out === null ? "Waiting for the curve's quote." : null)}
            onDone={() => { void held.refetch(); void quote.refetch(); }}
            request={() => (side === "buy"
              ? { address: curve, abi: PONS_CURVE, functionName: "buy", args: [spending, minOut ?? 0n, address!], value: spending }
              : { address: curve, abi: PONS_CURVE, functionName: "sell", args: [sellingTokens, minOut ?? 0n, address!] })}
          />
        )}
        {why ? <p className="tkt__why quiet">{why}</p> : null}
        {quoteErr && !why ? <p className="tkt__why warn-text">{quoteErr}</p> : null}
        {live ? <p className="tkt__net quiet small">Signs on chain <span className="num">{live.deployment.chainId}</span>{staging ? ", a fork where the ETH is worth nothing" : ""}.</p> : null}
      </div>

      {!graduated && gradShare !== null && curveState.data ? (
        <div className="tkt__grad">
          <div className="tkt__row"><span className="tkt__label">Until it graduates</span><span className="tkt__value num">{Math.round(gradShare * 100)}%</span></div>
          <div className="groove" role="progressbar" aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.round(gradShare * 100)} aria-label="How near the curve is to graduating">
            <div className="groove__fill" style={{ transform: `scaleX(${gradShare})` }} />
          </div>
          <p className="quiet small">
            <span className="num">{eth(curveState.data.inCurve, 4)}</span> of the <span className="num">{eth(curveState.data.threshold, 4)}</span> the curve needs, read from the chain just now.
            {" "}Past it the curve closes and the coin trades in a pool instead.
          </p>
        </div>
      ) : graduated ? (
        <p className="tkt__grad quiet small">This coin has graduated: the curve is closed and it trades in a pool.</p>
      ) : null}
    </aside>
  );
}
