import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { useAccount } from "wagmi";
import { formatUnits, parseUnits } from "viem";
import type { WrappedCoin, WrappedCollection } from "../api/types";
import { useEnv } from "../lib/env";
import { marketVenues, type WSource } from "../lib/wrapped";
import { quoteCoin, pct, type Quote } from "../lib/quote";
import { ERC20_APPROVE, ETCH_ROUTER } from "../lib/abisV3";
import { eth, when } from "../lib/format";
import { useNow } from "../lib/env";
import { FROG_CURVE_P1 } from "../rook/abis";
import { CLOSE_REFUSED, DEFAULT_DEADLINE_MIN, REFUNDS_HELD, closeBlocked, deadlineOf, graduatingSteps } from "../rook/keyless";
import { PHASE1, pinnedAddress } from "../rook/phase";
import { bundledStack } from "../rook/stack";
import { TxButton } from "./wallet";

/**
 * docs/v5/ux-and-reserves.md §1, drawn as docs/v6/site-design.md §4 says: Buy / Sell, You pay, You get, ONE total-cost number,
 * At least, the button; every hop and fee under Details. The words wX, escrow, mint and burn appear only inside Details. Since the
 * design review of 2026-09-27 the prominent figure is what you get; the total cost sits under it, one number still, secondary.
 */
const E = 10n ** 18n;
const fmtCoin = (x: bigint) => { const n = Number(formatUnits(x, 18)); return n >= 1_000 ? Math.round(n).toLocaleString("en-US") : n.toLocaleString("en-US", { maximumFractionDigits: 4 }); };

/** `pageCoin`: the coin the page is about, from its URL (#/coin/<address>), not from the indexer: every send is bound to it (R2 N-2). */
export function Ticket({ src, col, coin, feeTierBps, pageCoin }: { src: WSource; col: WrappedCollection; coin: WrappedCoin; feeTierBps: number | null; pageCoin: string }) {
  const env = useEnv();
  const { address } = useAccount();
  const [side, setSide] = useState<"buy" | "sell">("buy");
  const [raw, setRaw] = useState(side === "buy" ? "0.1" : "1000000");
  const [slip, setSlip] = useState(100);
  const [deadline, setDeadline] = useState(DEFAULT_DEADLINE_MIN);
  const now = useNow();
  const fixture = src.kind === "fixture";
  const live = env && env.mode !== "none" && env.mode !== "sample" && env.mode !== "api" ? env : null;
  let wei = 0n; try { wei = parseUnits(raw || "0", 18); } catch { wei = 0n; }
  const wx = col.symbol;

  const q = useQuery({ queryKey: ["quote", src.kind, coin.address, side, String(wei), slip], enabled: wei > 0n, staleTime: 5_000, refetchInterval: fixture ? false : 10_000, queryFn: async () => {
    const venues = fixture || !live ? marketVenues(col.market, coin.address, coin.phase, coin.sweepOnly) : await (await import("../lib/liveQuote")).liveVenues(live.client, col, coin);
    return quoteCoin(side, wei, venues, slip);
  } });
  // E-07 (docs/audit/2026-09-26/fixes-W2-site.md §2): in Phase 1 the router — the approval's spender and the graduated trade's
  // target — is the bundled stack file's, never the indexer's; if the indexer names another, nothing is sent
  const pin = PHASE1 ? pinnedAddress(bundledStack.stack, col.slug, "router", col.contracts.router) : null;
  const router = pin ? pin.address : col.contracts.router;
  const routerWhy = pin?.why ?? null;
  const allowance = useQuery({ queryKey: ["allowance", coin.address, address, router], enabled: !!live && !!address && !!router && side === "sell" && coin.phase === "graduated",
    queryFn: () => live!.client.readContract({ address: coin.address, abi: ERC20_APPROVE, functionName: "allowance", args: [address!, router!] }) });

  const quote: Quote | null = q.data?.ok ? q.data.quote : null;
  const why = q.data && !q.data.ok ? q.data.why : null;
  const needsApproval = side === "sell" && coin.phase === "graduated" && allowance.data !== undefined && allowance.data < wei;
  const unit = side === "buy" ? "ETH" : coin.symbol;
  const pick = (s: "buy" | "sell") => { setSide(s); setRaw(s === "buy" ? "0.1" : "1000000"); };
  const grad = graduatingSteps(coin, now);
  const closes = !!quote?.route.change;

  return (
    <div className="panel ticket">
      <div className="ticket__tabs" role="tablist" aria-label="Buy or sell">
        <button type="button" role="tab" aria-pressed={side === "buy"} onClick={() => pick("buy")}>Buy {coin.symbol}</button>
        <button type="button" role="tab" aria-pressed={side === "sell"} onClick={() => pick("sell")}>Sell</button>
      </div>
      {coin.phase === "graduating" ? <p className="notice notice--warn small">Buying the NFTs now. Trading opens on Uniswap when the coin graduates.</p> : null}
      {coin.phase === "trading" && coin.canClose === false && !coin.sweepOnly ? <p className="notice notice--warn small" role="status">{CLOSE_REFUSED}</p> : null}
      {coin.phase === "refunding" ? <p className="notice notice--bad small">This coin did not graduate in time. Holders can refund what they hold instead of trading.</p> : null}

      <div className="ticket__line"><span className="ticket__label">You pay</span>
        <div className="ticket__in"><input className="num" inputMode="decimal" aria-label={`Amount in ${unit}`} value={raw} onChange={(e) => setRaw(e.target.value.replace(/[^0-9.]/g, ""))} /><span>{unit}</span></div></div>
      <div className="ticket__line ticket__get"><span className="ticket__label">You get</span>
        <span className="ticket__value">{quote ? <>≈ {side === "buy" ? fmtCoin(quote.amountOut) : eth(quote.amountOut, 5)} <small>{side === "buy" ? coin.symbol : "ETH"}</small></> : q.isFetching ? "…" : <span className="muted">—</span>}</span></div>
      <div className="ticket__line ticket__cost"><span className="ticket__label">Total cost <small>fees, spread and price impact, all in</small></span>
        <span className="ticket__value">{quote ? <>{pct(quote.totalCostBps)}{side === "buy" && quote.totalCostBps > 0 ? <small> ≈ {eth((quote.amountIn * BigInt(Math.round(quote.totalCostBps * 100))) / 1_000_000n, 4)} ETH</small> : null}</> : <span className="muted">—</span>}</span></div>
      <div className="ticket__line ticket__least"><span className="ticket__label">At least</span>
        <span className="ticket__value small">{quote ? <>{side === "buy" ? fmtCoin(quote.minOut) : eth(quote.minOut, 5)} <small>{side === "buy" ? coin.symbol : "ETH"} · {pct(slip, 1)} slippage</small></> : <span className="muted">—</span>}</span></div>
      {quote?.route.change ? <p className="small muted">This buy closes the curve: {eth(quote.amountIn, 4)} ETH is used, the rest comes back in the same transaction.</p> : null}
      {why ? <p className="small warn" role="status">{why}</p> : null}
      {q.error ? <p className="small error" role="alert">The quote failed: {String((q.error as Error).message).slice(0, 120)}</p> : null}

      <div className="ticket__go">
        {coin.phase === "refunding" ? (
          <TxButton coin={pageCoin} label={`Refund ${coin.symbol}`} block disabledWhy={wei <= 0n ? "Enter an amount." : null} request={() => ({ address: coin.curve, abi: FROG_CURVE_P1, functionName: "refund", args: [wei] })} />
        ) : coin.phase === "graduating" ? (
          <GraduatingSteps curve={coin.curve} grad={grad} pageCoin={pageCoin} />
        ) : needsApproval ? (
          <TxButton coin={pageCoin} label={`Allow ${coin.symbol} to be sold`} block disabledWhy={routerWhy} request={() => ({ address: coin.address, abi: ERC20_APPROVE, functionName: "approve", args: [router!, wei] })} onDone={() => void allowance.refetch()} />
        ) : (
          <TxButton coin={pageCoin} label={side === "buy" ? `Buy ${coin.symbol}` : `Sell ${coin.symbol}`} block primary={side === "buy"}
            disabledWhy={!quote ? "Waiting for a quote." : closeBlocked(coin, side, closes) ? CLOSE_REFUSED : coin.phase === "graduated" ? routerWhy ?? (!router ? "No router is configured for this collection." : null) : null}
            request={() => {
              // A-07: every trade carries a deadline, the curve's too (the deadline overloads); the default is 20 minutes
              const dl = deadlineOf(Date.now() / 1000, deadline);
              if (coin.phase === "trading") return side === "buy" ? { address: coin.curve, abi: FROG_CURVE_P1, functionName: "buy", args: [quote!.minOut, dl], value: wei } : { address: coin.curve, abi: FROG_CURVE_P1, functionName: "sell", args: [wei, quote!.minOut, dl] };
              return side === "buy" ? { address: router!, abi: ETCH_ROUTER, functionName: "buy", args: [coin.address, quote!.minOut, address!, dl], value: wei } : { address: router!, abi: ETCH_ROUTER, functionName: "sell", args: [coin.address, wei, quote!.minOut, address!, dl] };
            }} />
        )}
      </div>
      <div className="ticket__prefs">
        <span>Slippage <span className="seg">{[50, 100, 300].map((b) => <button key={b} type="button" aria-pressed={slip === b} onClick={() => setSlip(b)}>{pct(b, 1)}</button>)}</span></span>
        <span>Deadline <span className="seg">{[10, 20, 60].map((m) => <button key={m} type="button" aria-pressed={deadline === m} onClick={() => setDeadline(m)}>{m} min</button>)}</span></span>
      </div>

      <details className="details">
        <summary>Details</summary>
        {quote ? <Hops quote={quote} wx={wx} symbol={coin.symbol} fixture={fixture} deadline={deadline} feeTierBps={feeTierBps} /> : <p className="small muted">Enter an amount to see the route.</p>}
      </details>
    </div>
  );
}

function Hops({ quote, wx, symbol, fixture, deadline, feeTierBps }: { quote: Quote; wx: string; symbol: string; fixture: boolean; deadline: number; feeTierBps: number | null }) {
  const name = (t: string) => (t === "WX" ? wx : t === "COIN" ? symbol : "ETH");
  const routeWord = { curve: "the coin's curve", pool: `the collection's pool and the coin's pool`, floorModule: "the floor module and the coin's pool", direct: "the coin's pool" }[quote.route.id];
  return (
    <div className="stack">
      <p className="small muted">Route: {routeWord}. {fixture ? "Every pool hop is a constant-product estimate from fixture data, labelled as such." : "Pool hops are Uniswap's own quote for this amount."}{feeTierBps !== null ? ` The creator's fee tier is ${pct(feeTierBps, 0)} of every trade, fixed at launch; 40% of it buys NFTs from the floor into the vault.` : ""}</p>
      <ul className="hops">
        {quote.route.legs.map((l, i) => (
          <li key={i} className="hops__hop"><span>{name(l.tokenIn)} → {name(l.tokenOut)}<small className="muted" style={{ display: "block", fontWeight: 400 }}>{l.contract}{l.estimate ? " · estimate" : ""}</small></span><span className="num muted">{l.spreadBps ? `spread ${pct(l.spreadBps)}` : ""}</span><span className="num">{l.fees.reduce((a, f) => a + f.bps, 0) ? pct(l.fees.reduce((a, f) => a + f.bps, 0)) : "—"}</span></li>
        ))}
        {quote.feeLines.map((f, i) => <li key={`f${i}`}><span>{f.label}</span><span className="num">{pct(f.bps)}</span><span className="num">{eth(f.eth, 6)} ETH</span></li>)}
        <li><span>{quote.impactBps < 0 ? "Price improvement" : "Price impact"}<small className="muted" style={{ display: "block" }}>what is left of the total after every fee and spread</small></span><span /><span className={`num ${quote.impactBps < 0 ? "good" : ""}`}>{pct(quote.impactBps)}</span></li>
        <li className="hops__hop"><span>Total cost</span><span /><span className="num accent">{pct(quote.totalCostBps)}</span></li>
      </ul>
      {quote.alternatives.length ? <p className="small muted">Also looked at: {quote.alternatives.map((r) => `${r.id === "floorModule" ? "the floor module" : r.id}${r.executable ? "" : ` (${r.why ?? "not routable"})`}`).join("; ")}.</p> : null}
      <p className="small muted">{quote.route.id === "curve" ? `The curve reverts the whole trade if it is not mined within ${deadline} minutes.` : `EtchRouter reverts the whole trade if it is not mined within ${deadline} minutes.`}</p>
    </div>
  );
}

/** A-01's escape hatch, in the ticket's place while the coin graduates: anyone may graduate it once the queue has filled it (or it is
 *  SWEEP_ONLY); if nobody can, refunds open at `refundsOpenAt()` and anyone may open them. */
function GraduatingSteps({ curve, grad, pageCoin }: { curve: `0x${string}`; grad: ReturnType<typeof graduatingSteps>; pageCoin: string }) {
  return (
    <div className="stack">
      {grad.graduate ? <TxButton coin={pageCoin} label="Graduate" block request={() => ({ address: curve, abi: FROG_CURVE_P1, functionName: "graduate", args: [] })} /> : <p className="small muted" style={{ margin: 0 }}>Waiting for the items' budget to be filled; anyone can press Graduate once it is.</p>}
      {grad.refundsAt !== null ? (grad.openRefunds
        ? <div><p className="small" style={{ marginBottom: ".5rem" }}>It was not graduated in time: anyone may open refunds, and holders then take back what is left.</p><TxButton coin={pageCoin} label="Open refunds" primary={false} block request={() => ({ address: curve, abi: FROG_CURVE_P1, functionName: "openRefunds", args: [] })} /></div>
        : <p className="small muted" style={{ margin: 0 }}>If it is not graduated, refunds open from about {when(grad.refundsAt)}.{grad.held ? ` ${REFUNDS_HELD}` : ""}</p>) : null}
    </div>
  );
}
