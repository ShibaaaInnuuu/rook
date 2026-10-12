import { Failed, Loading } from "../components/ui";
import { NeedsIndexer, PageHead } from "../components/catalog";
import { CoinMarket } from "../components/market";
import { Trending } from "../components/trending";
import { useCoinRows, useListingRows, useNeedsIndexer } from "../lib/catalog";

/**
 * Every coin launched through the Launcher, with the vault it pays into and what its fees have done. Sorting, filtering
 * and search happen in the browser over one page of rows: the indexer's list endpoint has none of them (docs/phase7.md).
 *
 * docs/phase14.md §2: the header is one sentence. It used to carry four claims about how the figures on the cards are made
 * — the supply read from `totalSupply()`, the curve-side price, the dollar rate's provenance, and what is not valued at all
 * — in a 296px paragraph a reader met before meeting a single coin. Every one of those claims is kept, under the controls,
 * in a disclosure the reader opens, because that is where somebody who has started reading figures wants them.
 */
export function Explore() {
  const coins = useCoinRows();
  const listings = useListingRows();
  const needsIndexer = useNeedsIndexer();
  const rows = coins.data?.rows ?? [];

  const provenance = (
    <details className="prov__open">
      <summary>What the figures on these cards are</summary>
      <p>
        A market capitalisation is the coin's whole supply, read from <span className="mono">totalSupply()</span> on chain,
        times the curve-side price the indexer computes from each trade — the quote that reached the curve over the coins that
        left it, before Pons's fee and the creator tax.
      </p>
      <p>
        Where a dollar figure is shown, the rate behind it is a spot read of one on-chain Uniswap pool at one block rather
        than a market data feed, and where that rate cannot be read the figure stays in the pair asset instead of being
        guessed. Volume is the pair asset traded in the last 24 hours, at either venue, as the chain recorded it.
      </p>
      <p>What is still not valued is anything a vault holds.</p>
    </details>
  );

  return (
    <>
      <PageHead title="Explore">
        <p>Every coin launched through the Launcher, and the vault its fees pay into.</p>
      </PageHead>

      {needsIndexer ? <NeedsIndexer what="a list of every coin" /> : coins.isLoading ? <Loading what="coins" /> : coins.error ? <Failed what="coins" error={coins.error} /> : (
        <>
          <Trending rows={rows} onAll={() => document.getElementById("market")?.scrollIntoView({ behavior: "smooth", block: "start" })} />
          <CoinMarket id="market" rows={rows} total={coins.data?.total ?? 0} vaults={listings.data ?? []} page={24} note={provenance} />
        </>
      )}
    </>
  );
}
