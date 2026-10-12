import { href, useEnv, useNow } from "../lib/env";
import { useCoin, useListings, useVault } from "../lib/data";
import { ROUTER } from "../lib/abis";
import {CopyAddr, Failed, Figure, Loading, NoDeployment, Section } from "../components/ui";
import { TxButton } from "../components/wallet";
import { eth } from "../lib/format";
import { volumeUntilNextBuy } from "../lib/estimate";
import { Terminal } from "../components/Terminal";
import { MarketChart } from "../components/MarketChart";
import { TradesHolders } from "../components/TradesHolders";
import { useCandles } from "../lib/coinchart";

export function CoinPage({ address }: { address?: string }) {
  const env = useEnv();
  const now = useNow();
  const coin = useCoin(address);
  const listings = useListings();
  const listing = coin.data ? listings.data?.[coin.data.listingId] : undefined;
  const vault = useVault(listing?.vault, listing?.id);
  // the day's set is cheap and gives the ticket a price to measure impact against, and the panel its dollar rate, whatever
  // the chart itself is showing
  const hourly = useCandles(address, "1d");
  if (env === null) return <Loading what="the deployment" />;
  if (env.mode === "none") return <><header className="pagehead"><h1>Coin</h1></header><NoDeployment /></>;
  if (!address || !/^0x[0-9a-fA-F]{40}$/.test(address)) return <p className="error">That is not a coin's address.</p>;
  if (coin.isLoading || listings.isLoading) return <Loading what="the coin" />;
  if (coin.error) return <Failed what="the coin" error={coin.error} />;
  if (!coin.data) return <div className="notice"><h2>Not a coin launched on Rook</h2><p>The Launcher has no record of <span className="mono">{address}</span>. A coin launched on Pons any other way has no router and pays nothing to a vault.</p></div>;
  const c = coin.data;
  const ix = c.indexed;
  const local = vault.data && !vault.data.unknown ? volumeUntilNextBuy({ capWei: vault.data.capWei, capExpiry: vault.data.capExpiry, nowS: now, spendableWei: vault.data.spendable, creatorTaxBps: c.creatorTaxBps }) : null;
  // the indexer's figure when it answered (it also counts the minimum declaration and what is one harvest away); else our own
  const next = ix ? { volumeWei: ix.volumeWei, why: ix.why, shortfallWei: 0n, perEthToVaultWei: ix.takeBps === null ? 0n : (10n ** 18n * BigInt(ix.takeBps) * 8_000n) / 100_000_000n } : local;
  const target = ix ? ix.targetWei : vault.data?.capWei ?? null, have = ix ? ix.spendableWei : vault.data && !vault.data.unknown ? vault.data.spendable : null;
  const waiting = c.routerBalance - c.treasuryOwed + c.inEscrow;
  return (
    <>
      <header className="pagehead">
        <h1>{c.name} <span className="mono quiet">{c.symbol}</span></h1>
        <p>Paired for good with {listing ? <a href={href("listing", listing.id)}>{listing.name}</a> : `listing ${c.listingId}`}, with a creator tax of {ix && !ix.taxKnown ? "an unknown size (it could not be read at launch)" : `${(c.creatorTaxBps / 100).toFixed(c.creatorTaxBps % 100 ? 2 : 0)}%`}. It is {c.graduated ? "trading in a pool, having graduated from the curve" : "still on the bonding curve"}.</p>
      </header>

      <div className="terminal">
        <MarketChart address={c.address} symbol={c.symbol} />
        <Terminal
          coin={c.address}
          curve={c.curve as `0x${string}`}
          symbol={c.symbol}
          creatorTaxBps={c.creatorTaxBps}
          lastPrice={hourly.data?.candles.at(-1)?.c ?? null}
          graduated={c.graduated}
        />
      </div>

      {/* docs/phase13.md §7: one panel directly under the chart, two tabs, where the old Trades section was. The dollar rate
          is the one the chart has already read, so the tape's dollar column costs no second request. */}
      <TradesHolders coin={c.address} symbol={c.symbol} usdPerEth={hourly.data?.usdPerEth ?? null} />

      <section className="headline" aria-labelledby="h-next">
        <h2 id="h-next">Volume until the next buy</h2>
        {!next ? <Loading what="the vault" /> : next.volumeWei === null ? <p className="headline__none">{next.why}</p> : (
          <>
            <p className="headline__num"><span className="num">{next.volumeWei === 0n ? "0" : `≈ ${eth(next.volumeWei, 2)}`}</span> <span className="unit">ETH of trading</span></p>
            {target !== null && have !== null && target > 0n ? (() => {
              const done = Math.min(1, Number((have * 10_000n) / target) / 10_000);
              return (<>
                <div className="groove" role="progressbar" aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.round(done * 100)} aria-label="How much of one item under the cap the vault can already afford"><div className="groove__fill" style={{ transform: `scaleX(${done})` }} /></div>
                <p className="groove__key"><span><span className="num">{eth(have)}</span> ETH the vault can spend</span><span><span className="num">{eth(target)}</span> ETH {ix ? "is the next declaration" : "buys one under the cap"}</span></p>
              </>);
            })() : null}
          </>)}
        <p className="formula">
          <span className="mono">({ix ? "next declaration" : "cap"} − vault's spendable ETH) ÷ (0.80 × (0.70% + {(c.creatorTaxBps / 100).toFixed(2)}% tax))</span>
          {next && target !== null && have !== null && next.volumeWei !== null ? <> = <span className="mono">({eth(target)} − {eth(have)}) ÷ {(Number(next.perEthToVaultWei) / 1e16).toFixed(3)}%</span></> : null}
        </p>
        <p className="quiet">An estimate, not a promise. 0.70% is what was measured reaching a router per unit of volume, on the curve and in the pool. It assumes the cap does not move, that Pons's operator keeps sweeping the pool's fees, and that no other coin on this listing adds to the same vault, which any of them can.</p>
      </section>

      <Section title="Fees">
        <dl className="figures">
          {ix?.pendingUnknown ? <Figure label="Waiting to be harvested" value="not known" tone="warn" note={<>It is read from the chain at request time: {ix.pendingUnknown}.</>} />
            : <Figure label="Waiting to be harvested" value={eth(waiting > 0n ? waiting : 0n, 6)} unit="ETH" note={<>In Pons's escrow and in the router. {ix?.curveNote ? <>Not counted: {ix.curveNote}.</> : "Fees still inside the curve are swept by the harvest itself and are not counted here."}</>} />}
          <Figure label="Sent to the vault, lifetime" value={eth(c.contributed, 6)} unit="ETH" note="80% of everything this coin's router has harvested." />
          <Figure label="Spent on purchases" value={eth(c.charged, 6)} unit="ETH" note="What declarations have charged to this coin, first in, first out." />
        </dl>
        <TxButton label="Harvest" doing="Harvesting…" request={() => ({ address: c.router, abi: ROUTER, functionName: "harvest" })} />
        <p className="quiet">Anyone may press this. It sweeps the curve, claims from escrow, and sends 80% to the vault and 20% to the treasury. You pay the gas; you receive nothing.</p>
      </Section>

      <Section title="Addresses">
        <dl className="kv">
          <dt>Coin</dt><dd><CopyAddr a={c.address} /></dd><dt>Router</dt><dd><CopyAddr a={c.router} /></dd><dt>Curve</dt><dd><CopyAddr a={c.curve} /></dd><dt>Creator</dt><dd><CopyAddr a={c.creator} /></dd>
        </dl>
      </Section>
    </>
  );
}
