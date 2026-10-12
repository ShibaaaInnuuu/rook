import { useMemo, useState } from "react";
import { href, useEnv } from "../lib/env";
import { Slab } from "../components/Slab";
import { eth } from "../lib/format";
import { Failed, Loading } from "../components/ui";
import { Search } from "../components/Search";
import { Count, ListingRail, NeedsIndexer } from "../components/catalog";
import { CoinMarket } from "../components/market";
import { ActivityFeed } from "../components/feeds";
import { CatalogueStrip, Counter, Provenance, ReceiptFeed, Rules } from "../components/panels";
import { RANGES, Spark, type Point } from "../components/Spark";
import { KINDS, mergeActivity, ofKind, type Event } from "../lib/activity";
import { sats, useCatalogue } from "../lib/catalogue";
import { useAcquisitionRows, useCoinRows, useDrawRows, useListingRows, useNeedsIndexer, useStats } from "../lib/catalog";

/**
 * The front door: what Rook is in one sentence, then the scale in five numbers, then the market.
 *
 * Launched leads because it is the product's inventory, and a visitor who has read the hero should meet coins rather than
 * more prose. The hero is unchanged from Phase 5. docs/phase9.md, sections 2 and 10.
 */
export function Home() {
  const env = useEnv();
  const stats = useStats();
  const coins = useCoinRows();
  const listings = useListingRows();
  const receipts = useAcquisitionRows();
  const draws = useDrawRows();
  const catalogue = useCatalogue();
  const needsIndexer = useNeedsIndexer();
  const [range, setRange] = useState(RANGES[1]);
  const [kind, setKind] = useState<Event["kind"] | "all">("all");

  const spent: Point[] = (receipts.data?.rows ?? [])
    .filter((a) => a.settledAt)
    .map((a) => ({ t: a.settledAt as number, v: Number(a.satsPaid || 0) }));

  const events = useMemo(
    () => mergeActivity(coins.data?.rows ?? [], receipts.data?.rows ?? [], draws.data?.rows ?? [], (c) => c.lastHarvestAt ?? null),
    [coins.data, receipts.data, draws.data],
  );

  return (
    <>
      <section className="hero" aria-labelledby="hero-h">
        <Slab />
        <div className="hero__say">
          <h1 id="hero-h"><span>Fees that buy Bitcoin assets</span><span>for a coin's holders.</span></h1>
          <div className="hero__sub">
            <p>Launch on Pons. Every trade pays a contract that can do one thing: fund purchases of inscriptions or runes.</p>
            <div className="actions"><a className="btn" href={href("launch")}>Launch a coin</a><a href={href("docs")}>How it works</a></div>
          </div>
        </div>
      </section>

      <section className="band" aria-labelledby="live-h">
        <h2 className="band__title" id="live-h">Right now</h2>
        {stats.isLoading ? <Loading what="the totals" /> : stats.data ? (
          <dl className="strip">
            <div><dt>coins launched</dt><dd><Counter value={stats.data.coins} /></dd></div>
            <div><dt>vaults</dt><dd><Counter value={stats.data.listings} /></dd></div>
            <div><dt>ETH sent to vaults</dt><dd><span className="num">{eth(stats.data.toVaultsWei, 2)}</span></dd></div>
            <div><dt>purchases receipted</dt><dd><Counter value={stats.data.receipts} /></dd></div>
            <div><dt>delivered on Bitcoin</dt><dd><Counter value={stats.data.delivered} /></dd></div>
          </dl>
        ) : <p className="strip__none">No contracts are live yet. There are no figures to show, and this page will not invent any.</p>}
      </section>

      <section className="band" aria-labelledby="coins-h">
        <h2 className="band__title" id="coins-h">Launched</h2>
        {needsIndexer ? <NeedsIndexer what="a list of every coin" /> : coins.isLoading ? <Loading what="coins" /> : coins.error ? <Failed what="coins" error={coins.error} /> : (
          <CoinMarket rows={coins.data?.rows ?? []} total={coins.data?.total ?? 0} vaults={listings.data ?? []} />
        )}
      </section>

      <section className="band" aria-labelledby="act-h">
        <h2 className="band__title" id="act-h">Happening</h2>
        <div className="catalog">
          <div className="controls">
            <div className="chips" role="group" aria-label="Kind">
              {KINDS.map((k) => (
                <button key={k.key} type="button" className={`chip${kind === k.key ? " chip--on" : ""}`} aria-pressed={kind === k.key} onClick={() => setKind(k.key)}>{k.label}</button>
              ))}
            </div>
          </div>
          {needsIndexer ? null : coins.isLoading || receipts.isLoading ? <Loading what="what is happening" /> : (
            <ActivityFeed events={ofKind(events, kind)} />
          )}
          <p className="quiet small">Launches, harvests, purchases, deliveries and draws, newest first. Buys and sells are not here yet: the indexer serves trades one coin at a time, so they live on each coin's own page.</p>
        </div>
      </section>

      <section className="band" aria-labelledby="flow-h">
        <h2 className="band__title" id="flow-h">What the vaults have done</h2>
        <div className="panels">
          <div className="panel">
            <div className="panel__head">
              <h3>Spent on Bitcoin</h3>
              <span className="panel__open"><a href={href("analytics")}>Analytics</a></span>
            </div>
            {needsIndexer ? <NeedsIndexer what="a history of purchases" /> : receipts.isLoading ? <Loading what="settlements" /> : (
              <>
                <div className="chips" role="group" aria-label="Range">
                  {RANGES.map((r) => (
                    <button key={r.key} type="button" className={`chip${r.key === range.key ? " chip--on" : ""}`}
                      aria-pressed={r.key === range.key} onClick={() => setRange(r)}>{r.label}</button>
                  ))}
                </div>
                <Spark points={spent} range={range} format={(v) => sats(v)} label="spent on Bitcoin assets" />
                <p className="panel__note">Every point is a settled purchase, at the second the indexer recorded it. Fees are not timestamped, so this line is what the vaults spent, not what they took in.</p>
              </>
            )}
          </div>

          <div className="panel">
            <div className="panel__head">
              <h3>Receipts</h3>
              <span className="panel__open"><a href={href("receipts")}>Every receipt</a></span>
            </div>
            {needsIndexer ? null : receipts.isLoading ? <Loading what="receipts" /> : receipts.error ? <Failed what="receipts" error={receipts.error} /> : (
              <ReceiptFeed rows={receipts.data?.rows ?? []} />
            )}
          </div>
        </div>
      </section>

      <section className="band" aria-labelledby="vaults-h">
        <h2 className="band__title" id="vaults-h">Vaults</h2>
        <div className="catalog">
          <Search wide />
          <p className="offered__say">A vault holds the ETH that coins paired with it have sent, and can buy one collection or one rune with it. These exist on chain now.</p>
          {needsIndexer ? null : listings.isLoading ? <Loading what="vaults" /> : listings.error ? <Failed what="vaults" error={listings.error} /> : !listings.data?.length ? (
            <p className="empty">No vault exists yet. When the registry's owner creates one, it appears here with its balance and the coins paying into it.</p>
          ) : (
            <>
              <ListingRail listings={listings.data} limit={5} />
              <Count shown={Math.min(5, listings.data.length)} total={listings.data.length} noun="vault" />
              <p className="more"><a href={href("vaults")}>Every vault, with its cap and its budget</a></p>
            </>
          )}
        </div>
      </section>

      <section className="band" aria-labelledby="catalogue-h">
        <h2 className="band__title" id="catalogue-h">Collections and runes</h2>
        <div className="catalog">
          <p className="offered__say">The real Bitcoin collections and runes a vault can hold: the fifty most-traded of each kind on the day the catalogue was fetched. Each plate says whether that one has a vault yet.</p>
          <CatalogueStrip catalogue={catalogue.data} vaults={listings.data} />
          <Provenance catalogue={catalogue.data} />
          <p className="more"><a href={href("vaults")}>All {(catalogue.data?.collections.length ?? 0) + (catalogue.data?.runes.length ?? 0)} of them, searchable</a></p>
        </div>
      </section>

      <section className="band" aria-labelledby="rules-h">
        <h2 className="band__title" id="rules-h">What the contracts enforce</h2>
        <div className="catalog">
          <Rules />
          <p className="more"><a href={href("trust")}>And what is trusted instead</a>, actor by actor.</p>
        </div>
      </section>

      {env?.mode === "none" ? (
        <section className="band" aria-labelledby="what-h">
          <h2 className="band__title" id="what-h">What this is</h2>
          <div className="stack">
            <p className="band__lede">A coin's trading fees buy real Bitcoin assets, and its holders get them. No contract here can reach Bitcoin, so the whole ledger of what is enforced and what is trusted is published.</p>
            <p className="quiet"><a href={href("docs")}>How it works</a>, in full, and <a href={href("trust")}>what is trusted</a>, actor by actor.</p>
          </div>
        </section>
      ) : null}
    </>
  );
}
