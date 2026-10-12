import { href, useNow } from "../lib/env";
import { eth, when } from "../lib/format";
import { CopyAddr, Figure, Section } from "../components/ui";
import { OneClick } from "../components/OneClick";
import { LaunchFeeLine, POLICY_LINE, StatusBadge, YoyoPanel, launchExtraNow } from "../components/wx";
import type { WrappedCollection, WrappedCoin } from "../api/types";
import type { WSource } from "../lib/wrapped";

/**
 * docs/v5/ux-and-reserves.md §1, "The coin page". A coin launched on a wrapped collection: one ticket to buy or sell it with ETH,
 * whatever phase it is in. What it is backed by is named by the collection, not by its token; the token is in Details.
 */
export function WrappedCoinPage({ collection: c, coin, source }: { collection: WrappedCollection; coin: WrappedCoin; source: WSource }) {
  const now = useNow();
  const raised = coin.raisedWei ? BigInt(coin.raisedWei) : null, raise = BigInt(coin.raiseWei);
  const share = raised !== null && raise > 0n ? Math.min(1, Number((raised * 10_000n) / raise) / 10_000) : null;
  const tier = `${(coin.feeBps / 100).toFixed(coin.feeBps % 100 ? 1 : 0)}%`;
  const extra = launchExtraNow(coin.launchFee, now);
  return (
    <>
      <header className="pagehead">
        <h1>{coin.name} <span className="mono quiet">{coin.symbol}</span></h1>
        <p>Backed by <a href={href("collection", c.slug)}>{c.name}</a> <StatusBadge status={c.status} />. {POLICY_LINE[coin.policy]}</p>
      </header>
      <div className="terminal">
        <div className="stack">
          {coin.phase === "trading" && share !== null ? (
            <section className="headline" aria-labelledby="h-raise">
              <h2 id="h-raise">Until it graduates</h2>
              <p className="headline__num"><span className="num">{Math.round(share * 100)}%</span> <span className="unit">of {eth(raise, 2)} ETH raised</span></p>
              <div className="groove" role="progressbar" aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.round(share * 100)} aria-label="How near the curve is to graduating"><div className="groove__fill" style={{ transform: `scaleX(${share})` }} /></div>
              <p className="quiet">When the raise is reached the curve closes, the raise buys real {c.name} items, and the coin moves to Uniswap. The same button keeps working.</p>
            </section>
          ) : (
            <section className="headline" aria-labelledby="h-where">
              <h2 id="h-where">Where it trades</h2>
              <p className="headline__none">{coin.phase === "graduated" ? "On Uniswap, against real items of the collection." : coin.phase === "graduating" ? "Nowhere for now: its raise is buying items." : "Nowhere: it did not graduate, and holders can refund."}</p>
              <p className="quiet">{coin.phase === "graduated" ? `Its raise bought ${c.name} items, which now sit in custody and back its pool. The button beside this takes ETH and hands you ${coin.symbol}, and back, in one transaction.` : coin.phase === "graduating" ? "Trading reopens on Uniswap once its share of items is bought. Nothing is needed from you." : "Refunding returns your share of the ETH left and of any items already bought."}</p>
            </section>
          )}
          <dl className="figures">
            <Figure label="Raise" value={eth(raise, 2)} unit="ETH" note="Fixed at launch." />
            <Figure label="Fee tier" value={tier} note="The creator's choice at launch, fixed for good: every trade pays it, and the coin's pool charges it." />
            <Figure label="Policy" value={coin.policy === "yoyo" ? "YOYO" : coin.policy === "burn" ? "Burn" : "Raffle"} note={coin.policy === "yoyo" ? "Swept items go back on the market at a markup; what they fetch buys the coin back and burns it." : coin.policy === "burn" ? "Swept items are sent where nobody can move them." : "Whole swept items are raffled to holders by a drand round."} />
            <Figure label={extra > 0 ? "Launch fee now" : "Launch fee"} value={extra > 0 ? `+${(extra / 100).toFixed(2)}%` : coin.launchFee ? "over" : "none"} tone={extra > 0 ? "warn" : undefined} note={<LaunchFeeLine coin={coin} now={now} />} />
            <Figure label="Launched" value={coin.launchedAt ? when(coin.launchedAt).slice(0, 10) : "—"} />
          </dl>
        </div>
        <OneClick collection={c} coin={coin} source={source} />
      </div>
      {coin.policy === "yoyo" ? (
        <Section title="Relisted items and buybacks" aside="The YOYO policy: what the vault holds for this coin.">
          <YoyoPanel coin={coin} wx={c.symbol} />
        </Section>
      ) : null}
      <Section title="Addresses">
        <dl className="kv"><dt>Coin</dt><dd><CopyAddr a={coin.address} /></dd><dt>Curve</dt><dd><CopyAddr a={coin.curve} /></dd><dt>Creator</dt><dd><CopyAddr a={coin.creator} /></dd></dl>
      </Section>
    </>
  );
}
