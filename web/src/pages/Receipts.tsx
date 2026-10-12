import { useState } from "react";
import { href, useNow } from "../lib/env";
import { countdown, eth, units, when } from "../lib/format";
import { Ext, Failed, Loading } from "../components/ui";
import { Chips, Count, NeedsIndexer } from "../components/catalog";
import { VaultControl } from "../components/vaults";
import { Counter } from "../components/panels";
import { AssetArt, CoinArt, ListingArt, shortRef } from "../components/Plate";
import { logoSrc } from "../lib/plate";
import { newestFirst } from "../lib/feed";
import { sortCoins, useAcquisitionRows, useCoinRows, useListingRows, useNeedsIndexer, useRawStats } from "../lib/catalog";
import type * as Api from "../api/types";

type Tab = "purchases" | "deliveries" | "fees";

const DELIVERY: Record<Api.Acquisition["delivery"]["state"], string> = {
  undecided: "waiting on the creator's choice", held: "held in the vault", "never-delivered": "not to be delivered",
  "draw-open": "snapshot open to challenge", "draw-pinned": "round pinned", "draw-resolved": "winner drawn",
  won: "winner proven, waiting on an address", delivered: "delivered", voided: "voided", cancelled: "cancelled",
  "distribution-open": "open for claims", "distribution-rolled": "rolled into the next one",
};

function Receipt({ a }: { a: Api.Acquisition }) {
  const inscription = a.kind === "inscriptions";
  const d = a.delivery;
  return (
    <li className="receipt">
      <div className="receipt__what">
        <span className="receipt__art"><AssetArt kind={a.kind} assetRef={a.assetRef} /></span>
        <span className="receipt__ref mono" title={a.assetRef}>{shortRef(a.assetRef)}</span>
        <span className="quiet">{inscription ? "one inscription" : <>{units(BigInt(a.amount), 0)} raw units of a rune</>}, bought for <span className="num">{BigInt(a.satsPaid).toLocaleString("en-US")}</span> sats</span>
      </div>
      <div className="receipt__who">
        <span><a href={href("coin", a.coin)}>{a.coinSymbol ?? "the coin"}</a> fees paid for it, from <a href={href("listing", a.listingId)}>vault {a.listingId}</a></span>
        <span className="quiet">{when(a.settledAt ?? 0)}</span>
      </div>
      <div className="receipt__proof">
        <Ext href={`https://mempool.space/tx/${a.btcTxid}`}><span className="mono">{a.btcTxid.slice(0, 12)}…</span></Ext>
        <span className={`tag${d.state === "delivered" ? " tag--good" : d.overdue ? " tag--warn" : ""}`}>{DELIVERY[d.state]}</span>
        {d.deliveredTxid ? <Ext href={`https://mempool.space/tx/${d.deliveredTxid}`}>the delivery</Ext> : null}
      </div>
    </li>
  );
}

/** A fee is a receipt too: it is what filled the vault before anything could be spent from it. */
function FeeRow({ c }: { c: ReturnType<typeof sortCoins>[number] }) {
  return (
    <li className="receipt receipt--fee">
      <div className="receipt__what">
        <span className="receipt__coin">
          <span className="receipt__coinart"><CoinArt address={c.address} name={c.name} symbol={c.symbol} logo={logoSrc(c.logoUrl)} /></span>
          <a href={href("coin", c.address)}>{c.name}</a> <span className="mono quiet">{c.symbol}</span>
        </span>
      </div>
      <div className="receipt__who">
        <span><span className="num">{c.toVaultWei ? eth(c.toVaultWei, 4) : "0.0000"}</span> ETH reached the <a href={href("listing", c.listingId)}>{c.listingName ?? `vault ${c.listingId}`}</a> vault</span>
        <span className="quiet">{c.lifetimeFeesWei === null ? "total not known" : <>of <span className="num">{eth(c.lifetimeFeesWei, 4)}</span> ETH its trading has paid</>}</span>
      </div>
      <div className="receipt__proof">
        <span className="tag">{c.harvests ? `${c.harvests} harvest${c.harvests === 1 ? "" : "s"}` : "not harvested yet"}</span>
      </div>
    </li>
  );
}

/**
 * Saving for the next buy.
 *
 * A vault buys when it holds enough for one item at the price the keeper last posted. That is the only question a holder
 * actually has — when does something happen — so it is answered with a bar per vault, nearest first. The cap is a posted
 * price with an hour on it, not a promise, and a vault whose cap has expired cannot buy at any balance until a fresh one
 * is posted: that is said rather than drawn as a full bar.
 */
function Saving({ listings, now }: { listings: Api.ListingSummary[]; now: number }) {
  const rows = listings
    .map((l) => {
      const cap = l.capWei ? BigInt(l.capWei) : 0n;
      const have = BigInt(l.depositedWei);
      const live = cap > 0n && !!l.capExpiry && l.capExpiry > now && !l.capExpired;
      const share = cap > 0n ? Math.min(1, Number((have * 1000n) / cap) / 1000) : 0;
      // A cap can be a small fraction of an ETH, which four decimals prints as 0.0000. Show enough of it to be a number.
      const dp = cap > 0n && cap < 10n ** 15n ? 6 : 4;
      return { l, cap, have, live, share, ready: cap > 0n && have >= cap, dp };
    })
    .sort((a, b) => b.share - a.share || a.l.id - b.l.id);

  if (!rows.length) return null;

  return (
    <ul className="bars">
      {rows.map(({ l, cap, have, live, share, ready, dp }) => (
        <li key={l.id}>
          <span className="bars__name">
            <span className="bars__art">
              <ListingArt listing={l} tag="" />
            </span>
            <a href={href("listing", l.id)}>{l.name}</a>
          </span>
          <span className="bars__track" role="presentation"><span className="bars__fill" style={{ transform: `scaleX(${share})`, transformOrigin: "left" }} /></span>
          <span className={`bars__val num${ready && live ? " bars__val--ready" : ""}`}>{cap <= 0n ? "no price" : !live ? "expired" : ready ? "enough" : `${Math.round(share * 100)}%`}</span>
          <span className="bars__sub quiet">
            {cap > 0n ? (
              <>
                <span className="num">{eth(have, dp)}</span> ETH in the vault, and one costs <span className="num">{eth(cap, dp)}</span>.{" "}
                {!live ? <span className="warn-text">That price has expired, so nothing can be bought until the keeper posts a fresh one.</span>
                  : ready ? <>It can buy now. The posted price holds for another {countdown(l.capExpiry!, now)}.</>
                  : <>The posted price holds for another {countdown(l.capExpiry!, now)}.</>}
              </>
            ) : (
              <>No price is posted for this vault, so nothing can be bought from it whatever it holds. It has <span className="num">{eth(have, 4)}</span> ETH.</>
            )}
          </span>
        </li>
      ))}
    </ul>
  );
}

/**
 * The page that is the proof.
 *
 * Every purchase here crosses two chains: ETH left a vault on this one and an asset moved on Bitcoin. Both sides carry a
 * transaction anyone can open without asking us. What links the two is the attestor's word, which is why the links are here
 * rather than a tick.
 */
export function Receipts() {
  const stats = useRawStats();
  const acq = useAcquisitionRows();
  const coins = useCoinRows();
  const listings = useListingRows();
  const needsIndexer = useNeedsIndexer();
  const now = useNow();
  const [tab, setTab] = useState<Tab>("purchases");
  const [vault, setVault] = useState<number | null>(null);

  const all = acq.data?.rows ?? [];
  const byVault = vault === null ? all : all.filter((a) => a.listingId === vault);
  const shown = tab === "purchases"
    ? newestFirst(byVault, 200)
    : tab === "deliveries"
      ? newestFirst(byVault.filter((a) => a.delivery.deliveredTxid || a.delivery.state === "won" || a.delivery.overdue), 200)
      : [];
  const feeRows = sortCoins((coins.data?.rows ?? []).filter((c) => vault === null || c.listingId === vault), "fees");
  const s = stats.data;

  const empty = tab === "purchases"
    ? "No vault has bought anything yet. Every purchase that happens appears here with the Bitcoin transaction that settled it."
    : tab === "deliveries"
      ? "Nothing has been delivered yet. A delivery appears here once a winner has named a Bitcoin address and the keeper has sent the asset."
      : "No coin has routed a fee yet. When one trades, what reached its vault appears here.";

  return (
    <>
      <header className="pagehead proof">
        <h1>Every purchase has a Bitcoin transaction.</h1>
        <p className="proof__say">
          A vault spends ETH on this chain and an asset moves on Bitcoin. Both sides are on this page: the fee that filled the
          vault, the transaction that emptied it, and the transfer that delivered what it bought. Open any of them yourself.
          Nothing here is a tick from us.
        </p>
      </header>

      {needsIndexer ? <NeedsIndexer what="the receipt log" /> : (
        <>
          <dl className="facts">
            <div className="fact"><dt>assets bought</dt><dd>{s ? <Counter value={s.acquisitions.inscriptions + s.acquisitions.runeReceipts} /> : "…"}</dd></div>
            <div className="fact"><dt>ETH spent from vaults</dt><dd>{s ? <span className="num">{eth(BigInt(s.spentWei), 3)}</span> : "…"}</dd></div>
            <div className="fact"><dt>ETH fees into vaults</dt><dd>{s ? <span className="num">{eth(BigInt(s.toVaultsWei), 3)}</span> : "…"}</dd></div>
            <div className="fact"><dt>delivered on Bitcoin</dt><dd>{s ? <Counter value={s.deliveries.done} /> : "…"}</dd></div>
          </dl>

          <section className="band" aria-labelledby="saving-h">
            <h2 className="band__title" id="saving-h">Saving for the next buy</h2>
            <div className="catalog">
              <p className="offered__say">How near each vault is to affording one item at the price the keeper last posted. Nearest first. A posted price lasts an hour; when it expires the vault cannot buy at any balance until a fresh one arrives.</p>
              {listings.isLoading ? <Loading what="vaults" /> : !listings.data?.length ? (
                <p className="empty">No vault exists yet, so nothing is saving for anything.</p>
              ) : <Saving listings={listings.data} now={now} />}
            </div>
          </section>

          <section className="band" aria-labelledby="log-h">
            <h2 className="band__title" id="log-h">The log</h2>
            <div className="catalog">
              <div className="controls">
                <Chips label="Show" value={tab} options={[["purchases", "Purchases"], ["deliveries", "Deliveries"], ["fees", "Fees"]]} onPick={setTab} />
                {/* docs/phase14.md §1: this page had the identical wall of one chip per vault, and now has the identical control */}
                <VaultControl vaults={listings.data ?? []} value={vault} onPick={setVault} />
              </div>

              {tab === "fees" ? (
                coins.isLoading ? <Loading what="fees" /> : coins.error ? <Failed what="fees" error={coins.error} /> : !feeRows.length ? <p className="empty">{empty}</p> : (
                  <>
                    <Count shown={feeRows.length} total={coins.data?.total ?? feeRows.length} noun="coin" />
                    <ul className="receipts">{feeRows.map((c) => <FeeRow key={c.address} c={c} />)}</ul>
                    <p className="quiet small">A fee is not timestamped by the indexer, so these are totals per coin rather than a list in time order.</p>
                  </>
                )
              ) : acq.isLoading ? <Loading what="receipts" /> : acq.error ? <Failed what="receipts" error={acq.error} /> : !shown.length ? (
                <p className="empty">{empty}</p>
              ) : (
                <>
                  <Count shown={shown.length} total={vault === null && tab === "purchases" ? acq.data?.total ?? shown.length : shown.length} noun={tab === "purchases" ? "receipt" : "delivery"} />
                  <ul className="receipts">{shown.map((a) => <Receipt key={`${a.listingId}-${a.acquisitionId}`} a={a} />)}</ul>
                </>
              )}
            </div>
          </section>

          <p className="quiet proof__foot">
            Every row is read from the vaults' own events on this chain, from the block the indexer started at, and every
            Bitcoin transaction opens on a public explorer. Nothing here has been checked against Bitcoin by us: the
            transaction is either there or it is not, and the link is so that you can tell.
          </p>
        </>
      )}
    </>
  );
}
