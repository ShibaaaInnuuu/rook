import type { ReactNode } from "react";
import { eth } from "../lib/format";
import { cap } from "../lib/chart";
import { sats } from "../lib/catalogue";
import { Failed, Loading } from "../components/ui";
import { NeedsIndexer, PageHead } from "../components/catalog";
import { useUsdRate } from "../lib/caps";
import { useAcquisitionRows, useCoinRows, useListingRows, useNeedsIndexer, useRawStats } from "../lib/catalog";

/**
 * Every number the protocol has, with where it came from beside it. We run the indexer, so this page does not pretend
 * otherwise: it says which figures are folded from confirmed events and which are read from the chain at request time.
 *
 * docs/phase14.md §5. It used to be five `<dl className="strip">` under five headings: every number in it, none of them a
 * headline, and nothing saying what any of them means. Now four stat cards carry the figures a reader came for, and each
 * panel says in a sentence what its number is and what cannot happen to the money — which is the part that makes a page of
 * figures a report rather than a dashboard.
 *
 * Two things the phase asked for are not here, and §5 says why in full: there is no graduation count anywhere in the API to
 * put on a card, and there is no Bitcoin price anywhere on this site to say an ETH figure in sats with. The second reading
 * under each figure is therefore dollars, at Phase 12's rate, named for exactly what that rate is; the one real sats figure
 * — what the receipts actually recorded paying — is on the panel where it belongs.
 */

/** A stat card: a label, the figure, and one line saying what the figure is of. */
const Stat = ({ label, value, of }: { label: string; value: ReactNode; of: ReactNode }) => (
  <div className="stat">
    <span className="stat__label">{label}</span>
    <strong className="stat__num num">{value}</strong>
    <span className="stat__of quiet">{of}</span>
  </div>
);

/** One figure inside a panel: a label, a large value, and under it the same amount said another way, where there is one. */
const Fig = ({ label, value, also }: { label: string; value: ReactNode; also?: ReactNode }) => (
  <div className="pfig">
    <span className="pfig__label">{label}</span>
    <span className="pfig__val num">{value}</span>
    <span className="pfig__also quiet">{also ?? " "}</span>
  </div>
);

export function Analytics() {
  const stats = useRawStats();
  const listings = useListingRows();
  const coins = useCoinRows();
  const acquisitions = useAcquisitionRows();
  const needsIndexer = useNeedsIndexer();
  const s = stats.data;
  const rate = useUsdRate(coins.data?.rows ?? []);

  /** the same wei in dollars, at the rate the indexer read, or nothing at all when it read none */
  const usd = (wei: string): ReactNode => rate.usdPerEth === null ? undefined : <>{cap((Number(wei) / 1e18) * rate.usdPerEth, "usd")} at the rate below</>;

  const vaults = listings.data ?? [];
  const paired = vaults.filter((l) => l.coins > 0).length;
  const bought = s ? s.acquisitions.inscriptions + s.acquisitions.runeReceipts : 0;
  const satsPaid = (acquisitions.data?.rows ?? []).reduce((t, a) => t + Number(a.satsPaid || 0), 0);

  const bars = vaults.map((l) => ({ name: l.name, wei: BigInt(l.depositedWei), bought: l.acquisitions, coins: l.coins }))
    .sort((a, b) => (b.wei > a.wei ? 1 : b.wei < a.wei ? -1 : 0));
  const top = bars.reduce((m, b) => (b.wei > m ? b.wei : m), 0n);
  // docs/phase14.md §5: measured at 60 vaults, every bar drawn was 4,433px of them — the same wall the vault filter was,
  // in the same phase that removed it. Twelve, then a line saying how many are not drawn and where every one of them is.
  const BARS = 12;
  const drawn = bars.slice(0, BARS);

  return (
    <>
      <PageHead title="Analytics">
        <p>What the protocol has done, folded from confirmed contract events by the indexer this site reads, which we run. Every figure below names its own source.</p>
      </PageHead>

      {needsIndexer ? <NeedsIndexer what="platform totals" /> : stats.isLoading ? <Loading what="the totals" /> : stats.error ? <Failed what="the totals" error={stats.error} /> : !s ? (
        <p className="empty">No contracts are live yet, so there is nothing to total.</p>
      ) : (
        <>
          <div className="stats">
            <Stat label="coins launched" value={s.coins} of="through the Launcher, each paired with one vault at launch" />
            <Stat label="vaults with a coin" value={paired} of={<>of {vaults.length} vault{vaults.length === 1 ? "" : "s"} on chain; the rest have nothing paying in yet</>} />
            <Stat label="bought on Bitcoin" value={bought} of={<>{s.acquisitions.inscriptions} inscription{s.acquisitions.inscriptions === 1 ? "" : "s"} and {s.acquisitions.runeReceipts} rune receipt{s.acquisitions.runeReceipts === 1 ? "" : "s"}, each with a transaction</>} />
            <Stat label="delivered to a holder" value={s.deliveries.done} of={<>of {bought} bought; {s.deliveries.waiting} waiting on an address{s.deliveries.overdue ? `, ${s.deliveries.overdue} overdue` : ""}</>} />
          </div>

          <section className="rep">
            <header className="rep__head">
              <h2>Fees</h2>
              <span className="rep__sum"><span className="num">{eth(BigInt(s.claimedWei), 4)}</span> ETH claimed</span>
            </header>
            <p className="rep__say">
              A harvest claims what a coin's trades have earned in Pons's escrow and splits it where the FeeRouter's own
              bytecode says: 80% to the vault the creator named at launch, 20% to the treasury. The split is not a policy and
              cannot be changed after launch, and <strong>nothing can take ETH out of a vault except a declared purchase</strong> —
              not the creator, not the keeper, not us.
            </p>
            <div className="pfigs">
              <Fig label="claimed from Pons" value={<>{eth(BigInt(s.claimedWei), 4)} <span className="pfig__unit">ETH</span></>} also={usd(s.claimedWei)} />
              <Fig label="to vaults, 80%" value={<>{eth(BigInt(s.toVaultsWei), 4)} <span className="pfig__unit">ETH</span></>} also={usd(s.toVaultsWei)} />
              <Fig label="to the treasury, 20%" value={<>{eth(BigInt(s.toTreasuryWei), 4)} <span className="pfig__unit">ETH</span></>} also={usd(s.toTreasuryWei)} />
            </div>
          </section>

          <section className="rep">
            <header className="rep__head">
              <h2>Spending</h2>
              <span className="rep__sum"><span className="num">{eth(BigInt(s.spentWei), 4)}</span> ETH spent</span>
            </header>
            <p className="rep__say">
              ETH leaves a vault only for a purchase that was declared in advance, to a purchaser who has posted a bond, and
              only after a delay anybody can watch. What is not spent comes back. What leaves and never becomes an asset is
              written off and <strong>repaid to the charged coins out of that bond</strong>, and the write-off stays on chain
              forever rather than being netted away.
            </p>
            <div className="pfigs">
              <Fig label="released to purchase" value={<>{eth(BigInt(s.releasedWei), 4)} <span className="pfig__unit">ETH</span></>} also={usd(s.releasedWei)} />
              <Fig label="spent on assets" value={<>{eth(BigInt(s.spentWei), 4)} <span className="pfig__unit">ETH</span></>}
                also={satsPaid > 0 ? <>and the receipts recorded {sats(satsPaid)} paid</> : usd(s.spentWei)} />
              <Fig label="returned unspent" value={<>{eth(BigInt(s.returnedWei), 4)} <span className="pfig__unit">ETH</span></>} also={usd(s.returnedWei)} />
            </div>
            <p className="rep__foot">
              {!s.purchases.writtenOff
                ? <>Nothing has been written off.</>
                : BigInt(s.writtenOffWei) === 0n
                  ? <>Written off: {s.purchases.writtenOff} purchase{s.purchases.writtenOff === 1 ? "" : "s"} and <span className="num">no</span> ETH — everything released came back.</>
                  : <>Written off: <span className="num over">{eth(BigInt(s.writtenOffWei), 4)}</span> ETH over {s.purchases.writtenOff} purchase{s.purchases.writtenOff === 1 ? "" : "s"}.</>}
              {" "}The sats figure beside what was spent is not a conversion: it is what the receipts themselves recorded
              paying on Bitcoin.
            </p>
          </section>

          <section className="rep">
            <header className="rep__head">
              <h2>Purchases, draws and deliveries</h2>
              <span className="rep__sum"><span className="num">{s.purchases.settled}</span> settled</span>
            </header>
            <p className="rep__say">
              Every purchase is declared, released, and then either receipted with a Bitcoin transaction or written off. What
              was bought is drawn for, or shared out pro rata, against a snapshot whose root is published before the draw and
              can be recomputed by anyone during a challenge window — so <strong>a winner cannot be chosen after the fact</strong>.
            </p>
            <div className="pfigs">
              <Fig label="declared" value={s.purchases.declared} also={<>{s.purchases.released} released, {s.purchases.cancelled} cancelled</>} />
              <Fig label="settled with a receipt" value={s.purchases.settled} also={<>{s.purchases.writtenOff} written off</>} />
              <Fig label="delivered on Bitcoin" value={s.deliveries.done} also={<>{s.draws.open} draw{s.draws.open === 1 ? "" : "s"} open, {s.draws.resolved} resolved, {s.draws.voided} voided</>} />
            </div>
            <p className="rep__foot">
              {rate.usdPerEth === null
                ? <>No dollar figure is shown anywhere above: the indexer could not read a rate, and a rate is not guessed here.</>
                : <>The dollar figures above use <span className="num">${rate.usdPerEth.toLocaleString("en-US", { maximumFractionDigits: 2 })}</span> per ETH,
                   read from <span className="mono">slot0()</span> on <strong>one</strong> Uniswap v3 pool on this same chain
                   {rate.at ? <> at {new Date(rate.at * 1000).toLocaleString("en-GB", { timeZone: "UTC" })} UTC</> : null}. It is a spot read of one pool
                   at one block, not a market data feed.</>}
              {" "}Everything else on this page is read from the contracts' own events.
            </p>
          </section>

          {bars.length ? (
            <section className="section">
              <header className="cut"><h2>By vault</h2><div className="cut__aside">ETH that reached each vault, drawn to scale.</div></header>
              <ul className="bars">
                {drawn.map((b) => (
                  <li key={b.name}>
                    <span className="bars__name">{b.name}</span>
                    <span className="bars__track"><span className="bars__fill" style={{ width: top > 0n ? `${Number((b.wei * 1000n) / top) / 10}%` : "0%" }} /></span>
                    <span className="bars__val num">{eth(b.wei, 3)} ETH</span>
                    <span className="bars__sub quiet">{b.coins} coin{b.coins === 1 ? "" : "s"}, {b.bought} bought</span>
                  </li>
                ))}
              </ul>
              {bars.length > drawn.length ? (
                <p className="quiet small">The {drawn.length} vaults the most ETH has reached, of {bars.length}. <a href="#/vaults">Every vault</a> is listed with its own balance, its cap and its budget.</p>
              ) : null}
            </section>
          ) : null}

          <section className="section">
            <header className="cut"><h2>Where these come from</h2></header>
            <dl className="kv kv--wide">
              <dt>Folded from events</dt><dd>Every figure above. The indexer replays the contracts' own logs from the deployment block, waits for confirmations, and rolls back on a reorg. Run it yourself against any RPC and the totals are the same.</dd>
              <dt>Read at request time</dt><dd>A vault's current balance and a coin's pending fees, on the pages that show them. They come from a node when the page is opened and are marked unknown when it cannot be reached.</dd>
              <dt>Read, with a caveat</dt><dd>A coin's price, its supply, and the dollar rate. The price the indexer computes from each trade, curve-side, before Pons's fee and the creator tax; the supply it reads once from the coin's own <span className="mono">totalSupply()</span>; and dollars per ETH from one pool, as the footnote above says. What is still not read is the value of a Bitcoin asset a vault holds, so no worth is put on anything delivered.</dd>
              <dt>Not counted at all</dt><dd>How many coins have graduated from their curve into a Uniswap pool. No endpoint counts it: <span className="mono">/v1/stats</span> has no such field and a coin summary has no such flag, so the only honest place to read it is one coin's own page, which reads it from the chain when you open it. It is left off this page rather than estimated.</dd>
              <dt>Somebody's word</dt><dd>That a Bitcoin transaction carries the asset it claims to. Every receipt links to the transaction so you can check it. <a href="#/trust">The trust model</a> is the full list.</dd>
            </dl>
            <p className="quiet">{coins.data ? <>{coins.data.total} coin{coins.data.total === 1 ? "" : "s"} and {vaults.length} vault{vaults.length === 1 ? "" : "s"} are in these totals.</> : null}</p>
          </section>
        </>
      )}
    </>
  );
}
