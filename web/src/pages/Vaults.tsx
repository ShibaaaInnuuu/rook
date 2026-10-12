import { href, useNow } from "../lib/env";
import { countdown, eth, when } from "../lib/format";
import { ago } from "../components/panels";
import { Failed, Loading } from "../components/ui";
import { Count, NeedsIndexer, PageHead } from "../components/catalog";
import { CatalogueBrowser } from "../components/catalogue";
import { ListingArt } from "../components/Plate";
import { useCatalogue } from "../lib/catalogue";
import { useListingRows, useNeedsIndexer } from "../lib/catalog";
import type * as Api from "../api/types";

/**
 * A vault is the thing that exists on chain: one per collection or rune, holding the ETH that coins paired with it have sent,
 * able to buy that one asset and nothing else.
 *
 * The registry calls it a listing and the indexer serves `/v1/listings`; those are their words, kept in the code. What a
 * reader sees is a vault, because a collection either has one or it does not, and that noun is the whole distinction.
 *
 * The catalogue below is no longer "the things with no vault": every entry in it is meant to get one and most already have,
 * so it is one list of collections and runes, and each card says whether its own vault exists. docs/phase9.md §8, §11.3.
 */
function VaultRow({ l, now }: { l: Api.ListingSummary; now: number }) {
  const cap = l.capWei ? BigInt(l.capWei) : 0n;
  const live = cap > 0n && !!l.capExpiry && l.capExpiry > now && !l.capExpired;
  const budget = BigInt(l.depositedWei);
  const done = live && cap > 0n ? Math.min(1, Number((budget * 1000n) / cap) / 1000) : 0;
  return (
    <li className="listing">
      <a className="listing__in" href={href("listing", l.id)}>
        <span className="listing__art"><ListingArt listing={l} /></span>
        <span className="listing__head">
          <span className="listing__name">{l.name}</span>
          <span className="tag">{l.kind === "rune" ? "rune" : "inscriptions"}</span>
          {l.paused ? <span className="tag tag--warn">paused</span> : null}
          {l.openDeclarations ? <span className="tag">{l.openDeclarations} open</span> : null}
        </span>
        <span className="mono quiet listing__ref">{l.ref}</span>
        <span className="groove" role="presentation"><span className="groove__fill" style={{ transform: `scaleX(${done})` }} /></span>
        <span className="listing__figs">
          <span><span className="num">{eth(budget, 3)}</span> ETH in the vault</span>
          {/* docs/phase14.md §8.4: the same hour from both ends. A cap's freshness is a fact about the keeper's word, and
              until this phase the list could only say when it runs out. A source that does not carry it says nothing extra. */}
          <span>{live ? <><span className="num">{eth(cap, 4)}</span> ETH cap, {countdown(l.capExpiry!, now)} left{l.capPostedAt ? <>, posted {ago(l.capPostedAt)}</> : null}</> : <span className="warn-text">{cap > 0n ? `cap expired ${when(l.capExpiry ?? 0)}` : "no cap posted"}</span>}</span>
          <span><span className="num">{l.coins}</span> coin{l.coins === 1 ? "" : "s"} paying in</span>
          <span><span className="num">{l.acquisitions}</span> bought</span>
        </span>
      </a>
    </li>
  );
}

export function Vaults({ focus }: { focus?: string } = {}) {
  const listings = useListingRows();
  const catalogue = useCatalogue();
  const needsIndexer = useNeedsIndexer();
  const now = useNow();

  return (
    <>
      <PageHead title="Vaults">
        <p>A vault is one contract holding ETH that can buy one collection of inscriptions, or one rune, and nothing else. Coins pay into it; it buys; what it buys is receipted on Bitcoin. Only the registry's owner can create one.</p>
      </PageHead>

      <section className="band" aria-labelledby="on-chain-h">
        <h2 className="band__title" id="on-chain-h">Vaults</h2>
        <div className="catalog">
          <p className="offered__say">These exist on chain. Each holds ETH, names the one thing it may buy, and can be paired with a coin today.</p>
          {needsIndexer ? <NeedsIndexer what="the list of vaults" /> : listings.isLoading ? <Loading what="vaults" /> : listings.error ? <Failed what="vaults" error={listings.error} /> : !listings.data?.length ? (
            <p className="empty">No vault exists yet. Only the registry's owner can create one, and a coin can only be paired with a vault that already exists.</p>
          ) : (
            <>
              <Count shown={listings.data.length} total={listings.data.length} noun="vault" />
              <ul className="listings">{listings.data.map((l) => <VaultRow key={l.id} l={l} now={now} />)}</ul>
              <p className="quiet">The bar is how much of one item under the current cap the vault could already pay for. A cap with no time left stops every new purchase until the keeper posts a fresh one, which is the point of the expiry.</p>
            </>
          )}
        </div>
      </section>

      <section className="band" aria-labelledby="offered-h">
        <h2 className="band__title" id="offered-h">Collections and runes</h2>
        <div className="catalog">
          <p className="offered__say">
            The real Bitcoin collections and runes this site offers, with their own artwork and the floor they carried on the
            day the catalogue was fetched. Each card says whether its vault exists yet: a vault is one contract, created by
            the registry's owner, holding the ETH that coins paired with it have sent.
          </p>
          <CatalogueBrowser catalogue={catalogue.data} vaults={listings.data} initial={focus} />
          <p className="quiet small">There is no way to request a vault yet. When there is, it will be on this page.</p>
        </div>
      </section>
    </>
  );
}
