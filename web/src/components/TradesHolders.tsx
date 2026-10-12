import { useEffect, useRef, useState } from "react";
import type { Trade } from "../api/types";
import { CopyAddr, Ext, Failed, Loading } from "./ui";
import { ago } from "./panels";
import { useEnv } from "../lib/env";
import { useSource } from "../lib/catalog";
import { useHolders, useTrades } from "../lib/coinchart";
import { EXCLUDED_IS, curveLeg, holderRows, holdersCaveat, money, recordingRanOut, sharePct, tradeRows } from "../lib/book";
import { EXPLORER_ADDRESS, EXPLORER_TX, explorerFor } from "../lib/explorer";
import { clock, coins, price } from "../lib/chart";
import { eth, short } from "../lib/format";
import { freshKeys } from "../lib/feed";

/**
 * Who traded a coin and who holds it, in one panel under the chart, with two tabs. docs/phase13.md §7.
 *
 * It replaces the page's old "Trades" section rather than sitting beside it, so there is one tape and not two. Three things
 * about it are load-bearing:
 *
 * **Nothing is coloured only.** A side is the word "buy" or "sell" as well as a colour, because a colour is not a fact a
 * reader can rely on. Every colour comes from the custom properties in styles/etch.css, so the panel is the same material as
 * the page in either theme and there is no palette written down here.
 *
 * **Four empty states, four sentences.** "This coin has never traded" is not "this recording holds no more of this coin's
 * tape", which is not "the indexer would not answer", which is not "the holder list is still being built". A page that
 * prints one of those for another is lying about which of them happened.
 *
 * **An address is not a person, and on staging it is not even a trader.** Said above the list, every time, not in a footnote.
 */

type Tab = "trades" | "holders";

export function TradesHolders({ coin, symbol, usdPerEth }: { coin: string; symbol: string; usdPerEth: number | null }) {
  const [tab, setTab] = useState<Tab>("trades");
  const env = useEnv();
  const src = useSource();
  const recorded = src?.kind === "fixtures";
  const staging = env?.net === "staging";
  const base = explorerFor(env && "chain" in env ? env.chain.id : null, recorded);

  const trades = useTrades(coin);
  const holders = useHolders(coin, tab === "holders");
  const tradeTotal = trades.data?.pages[0]?.total ?? null;
  const holderTotal = holders.data?.pages[0]?.holders ?? null;

  return (
    <section className="section book" aria-labelledby="h-book">
      <header className="cut">
        <h2 id="h-book">Trades &amp; holders</h2>
        <div className="cut__aside">Both come from this coin's own logs: the curve's and the pool's trades, and the coin's own transfers.</div>
      </header>
      <div className="book__body">
        <div className="chips book__tabs" role="tablist" aria-label="Trades or holders">
          <button type="button" role="tab" id="tab-trades" aria-controls="book-list" aria-selected={tab === "trades"} className={`chip${tab === "trades" ? " chip--on" : ""}`} onClick={() => setTab("trades")}>
            Trades{tradeTotal === null ? "" : <span className="book__n num"> {tradeTotal.toLocaleString("en-US")}</span>}
          </button>
          <button type="button" role="tab" id="tab-holders" aria-controls="book-list" aria-selected={tab === "holders"} className={`chip${tab === "holders" ? " chip--on" : ""}`} onClick={() => setTab("holders")}>
            Holders{holderTotal === null ? "" : <span className="book__n num"> {holderTotal.toLocaleString("en-US")}</span>}
          </button>
        </div>
        <div id="book-list" role="tabpanel" aria-labelledby={`tab-${tab}`} className="book__list">
          {tab === "trades"
            ? <Trades q={trades} symbol={symbol} usdPerEth={usdPerEth} base={base} recorded={recorded} staging={staging} />
            : <Holders q={holders} symbol={symbol} base={base} recorded={recorded} staging={staging} />}
        </div>
      </div>
    </section>
  );
}

/** A read that failed, told apart: a recording that holds no such page is not an indexer that would not answer. */
function Broken({ what, error, recorded }: { what: string; error: unknown; recorded: boolean }) {
  if (recorded && recordingRanOut(error)) {
    return <p className="empty">This recorded demo holds no more of this coin's {what}. A live indexer has the rest; the recording keeps a few pages of each coin and then stops, and it says so here rather than pretending the list ended.</p>;
  }
  return <Failed what={what} error={error} />;
}

function More({ q, what }: { q: { hasNextPage: boolean; isFetchingNextPage: boolean; fetchNextPage: () => unknown }; what: string }) {
  if (!q.hasNextPage) return null;
  return (
    <button type="button" className="btn btn--quiet" onClick={() => void q.fetchNextPage()} disabled={q.isFetchingNextPage}>
      {q.isFetchingNextPage ? "Reading…" : `Older ${what}`}
    </button>
  );
}

function Trades({ q, symbol, usdPerEth, base, recorded, staging }: {
  q: ReturnType<typeof useTrades>; symbol: string; usdPerEth: number | null; base: string | null; recorded: boolean; staging: boolean;
}) {
  const rows = tradeRows(q.data?.pages ?? [], usdPerEth);
  const total = q.data?.pages[0]?.total ?? 0;
  const seen = useRef<Set<string> | null>(null);
  const fresh = freshKeys(rows.map((t) => t.id), seen.current);
  useEffect(() => { seen.current = new Set(rows.map((t) => t.id)); });

  if (q.isLoading) return <Loading what="this coin's trades" />;
  if (q.error) return <Broken what="tape" error={q.error} recorded={recorded} />;
  if (!rows.length) {
    return <p className="empty">This coin has never traded. The first buy appears here with the transaction that made it, and until then there is no trade to show and none will be invented.</p>;
  }

/**
 * What the fee and the tax took, under the value. Without it a reader multiplies the price by the size, gets a different
 * number from the value beside it, and concludes the page cannot do arithmetic — which is the right conclusion from what
 * they were shown. A pool trade has no line, because the hook charges outside the `Swap` event and the API does not guess.
 */
function feeLine(t: Trade) {
  const leg = curveLeg(t);
  if (leg === null || leg.feeTaxWei === 0n) return null;
  return (
    <span className="book__fee quiet">
      {t.side === "buy" ? "−" : "+"}{eth(leg.feeTaxWei, 5)} fee
    </span>
  );
}

/** The same sum, spelled out, for whoever hovers to check it. */
function valueTitle(t: Trade): string | undefined {
  const leg = curveLeg(t);
  if (leg === null) return t.venue === "pool"
    ? "A pool's hook charges outside the Swap event, so this trade's fee is not in the chain's record of it and is not guessed here."
    : undefined;
  const q = `${eth(BigInt(t.quoteWei), 5)} ETH`, f = `${eth(leg.feeTaxWei, 5)} ETH`, c = `${eth(leg.curveWei, 5)} ETH`;
  return t.side === "buy"
    ? `The trader paid ${q}. Pons's fee and the creator tax took ${f}, so the curve received ${c} — and that over the coins is the price shown.`
    : `The curve paid ${c}. Pons's fee and the creator tax took ${f}, so the trader received ${q} — and the curve's leg over the coins is the price shown.`;
}

  return (
    <>
      <p className="book__say quiet small">
        Newest first, from the curve's and the pool's own events. The price is the curve-side price of one whole coin: the
        quote that reached the curve over the coins that left it, so a buy and the sell after it are continuous. The value is
        the trader's own leg — what left their wallet on a buy, what arrived in it on a sell — and the two differ by Pons's
        fee and this coin's creator tax, which each row shows.
        {usdPerEth === null ? <> No dollar rate is known here, so the value is in the pair asset alone.</> : <> Dollars are that value at <span className="num">${usdPerEth.toFixed(2)}</span> per ETH, a spot read of one on-chain pool.</>}
        {staging ? <> <strong>On staging every trade is a simulated wallet's</strong>, on a chain we run, for money that is not money.</> : null}
      </p>
      <div className="book__scroll">
        <table className="book__table">
          <thead>
            <tr>
              <th scope="col">Time</th><th scope="col">Side</th>
              <th scope="col" className="book__r">Price</th>
              <th scope="col" className="book__r">{symbol}</th>
              <th scope="col" className="book__r">Value</th>
              <th scope="col" className="book__venue">Venue</th>
              <th scope="col">Trader</th>
              <th scope="col" className="book__r book__tx">Tx</th>
            </tr>
          </thead>
          <tbody>
            {rows.map((t) => (
              <tr key={t.id} className={fresh.has(t.id) ? "book__new" : undefined}>
                <td className="quiet" title={t.at === null ? "this trade's block had no time" : clock(t.at, 60)}>{ago(t.at)}</td>
                <td><span className={`tag tag--${t.side}`}>{t.side}</span></td>
                <td className="num book__r">{price(t.price)}</td>
                <td className="num book__r">{coins(t.coinWei)}</td>
                <td className="num book__r" title={valueTitle(t)}>
                  {eth(BigInt(t.quoteWei), 5)} <span className="quiet">ETH</span>
                  {t.usd === null ? null : <span className="book__usd quiet"> {money(t.usd)}</span>}
                  {feeLine(t)}
                </td>
                <td className="quiet book__venue">{t.venue}</td>
                <td className="mono">
                  {base ? <Ext href={EXPLORER_ADDRESS(base, t.trader)}>{short(t.trader)}</Ext> : <CopyAddr a={t.trader} />}
                  {t.venue === "pool" ? <span className="quiet" title="In a pool the event names the swap router the trade came through, not the person who traded."> ·router</span> : null}
                </td>
                <td className="book__r book__tx">{base ? <Ext href={EXPLORER_TX(base, t.tx)}>tx</Ext> : <span className="quiet" title={t.tx}>{short(t.tx)}</span>}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      <p className="more">
        <More q={q} what="trades" />
        <span className="quiet small"> {rows.length} of {total} shown</span>
      </p>
    </>
  );
}

function Holders({ q, symbol, base, recorded, staging }: {
  q: ReturnType<typeof useHolders>; symbol: string; base: string | null; recorded: boolean; staging: boolean;
}) {
  const page = q.data?.pages[0];
  const rows = holderRows(q.data?.pages ?? []);

  if (q.isLoading || !page) return q.error ? <Broken what="holder list" error={q.error} recorded={recorded} /> : <Loading what="this coin's holders" />;
  if (q.error) return <Broken what="holder list" error={q.error} recorded={recorded} />;

  return (
    <>
      <p className="book__count">
        {page.holders === null
          ? <><span className="num">not known</span><span className="unit"> — how many addresses hold {symbol} cannot be said yet</span></>
          : <><span className="num">{page.holders.toLocaleString("en-US")}</span><span className="unit"> {page.holders === 1 ? "address holds" : "addresses hold"} {symbol}</span></>}
      </p>
      <p className="book__say quiet small">
        {holdersCaveat(staging)}
        {page.supplyWei === null
          ? <> This coin's <span className="mono">totalSupply()</span> has not been read, so there is no percentage to give and none is invented.</>
          : <> Shares are of the whole supply, <span className="num">{(Number(page.supplyWei) / 1e18).toLocaleString("en-US", { maximumFractionDigits: 0 })}</span> {symbol}.</>}
      </p>
      {page.complete ? null : (
        <p className="notice notice--plain" role="status">
          This list is still being built: the indexer is reading the transfers from before it started watching, and has reached
          block <span className="num">{page.scannedThrough?.toLocaleString("en-US")}</span>. The balances below are right for the blocks it
          has read, and the count is left as not known rather than given as a part of the answer.
        </p>
      )}
      {!rows.length ? (
        <p className="empty">Nobody holds this coin yet. Every coin spends its first moments with its whole supply inside its own bonding curve, which is not a holder; the first buy puts the first address here.</p>
      ) : (
        <>
          <div className="book__scroll">
            <table className="book__table">
              <thead>
                <tr>
                  <th scope="col" className="book__r">#</th>
                  <th scope="col">Address</th>
                  <th scope="col" className="book__r">{symbol}</th>
                  <th scope="col" className="book__r">Share of supply</th>
                </tr>
              </thead>
              <tbody>
                {rows.map((h) => (
                  <tr key={h.address}>
                    <td className="num book__r quiet">{h.rank}</td>
                    <td className="mono">
                      {base ? <Ext href={EXPLORER_ADDRESS(base, h.address)}>{short(h.address)}</Ext> : <CopyAddr a={h.address} />}
                      {h.isCreator ? <span className="tag tag--good book__mark">creator</span> : null}
                    </td>
                    <td className="num book__r">{coins(h.balanceWei)}</td>
                    <td className="num book__r">{sharePct(h.share) ?? <span className="quiet">not known</span>}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
          <p className="more">
            <More q={q} what="holders" />
            <span className="quiet small"> {rows.length} of {page.total} shown</span>
          </p>
        </>
      )}
      {page.excluded.length ? (
        <details className="book__ex">
          <summary className="quiet small">
            {page.excluded.length === 1
              ? <>1 address holds this coin and is not counted</>
              : <>{page.excluded.length} addresses hold this coin and are not counted</>}
          </summary>
          <ul className="book__exlist small">
            {page.excluded.map((x) => (
              <li key={x.address}>
                <span className="mono">{short(x.address)}</span> · <span className="num">{coins(x.balanceWei)}</span> {symbol}
                {sharePct(x.share) ? <> · <span className="num">{sharePct(x.share)}</span></> : null}
                <span className="quiet"> — {EXCLUDED_IS[x.why]}</span>
              </li>
            ))}
          </ul>
          <p className="quiet small">
            None of these is somebody's position: each one is this coin's own plumbing, ours, Pons's or Uniswap's. They are
            listed rather than hidden so that the shares above can be seen not to add up to the supply.
          </p>
        </details>
      ) : null}
    </>
  );
}
