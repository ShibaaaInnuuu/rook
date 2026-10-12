import { href } from "../lib/env";
import { useWrappedCollection, useWrappedList } from "../lib/wrapped";
import { eth, when } from "../lib/format";
import { Failed, Figure, Loading, Section } from "../components/ui";
import { WxGate } from "../components/WxGate";
import { StatusBadge } from "../components/wx";
import type { WrappedCollection } from "../api/types";

/** docs/v5/ux-and-reserves.md §1, "The collection page". `#/collection` lists them; `#/collection/<slug>` is one. */
export function CollectionPage({ slug }: { slug?: string }) {
  return <WxGate title="Collections">{() => (slug ? <One slug={slug} /> : <All />)}</WxGate>;
}

const whole = (units: string | null) => (units === null ? "not known" : (Number(BigInt(units) / 10n ** 14n) / 1e4).toLocaleString("en-US"));

function All() {
  const list = useWrappedList();
  if (list.isLoading) return <Loading what="the collections" />;
  if (list.error) return <Failed what="the collections" error={list.error} />;
  return (
    <>
      <header className="pagehead"><h1>Collections</h1><p>Each collection's items sit in custody; every coin launched on it trades against them. The proof that they are there is on the <a href={href("reserves")}>reserves page</a>.</p></header>
      <ul className="rows">
        {list.data!.collections.map((c) => (
          <li key={c.slug}><a className="row" href={href("collection", c.slug)}>
            <span className="row__main"><span className="row__name">{c.name}</span><StatusBadge status={c.status} /></span>
            <span className="row__meta">{c.itemsHeld ?? "?"} items held · floor {c.floorWei ? `${eth(BigInt(c.floorWei), 3)} ETH` : "no fresh floor"} · {c.coins} coin{c.coins === 1 ? "" : "s"}</span>
          </a></li>
        ))}
      </ul>
    </>
  );
}

function One({ slug }: { slug: string }) {
  const q = useWrappedCollection(slug);
  if (q.isLoading) return <Loading what="the collection" />;
  if (q.error || !q.data) return <div className="notice"><h2>No such collection</h2><p>Nothing is configured under “{slug}”. <a href={href("collection")}>All collections</a>.</p></div>;
  const c: WrappedCollection = q.data;
  return (
    <>
      <header className="pagehead">
        <h1>{c.name}</h1>
        <p>{c.description} <StatusBadge status={c.status} /> <a href={href("reserves", c.slug)}>See the proof of reserves</a>.</p>
      </header>
      <dl className="figures">
        <Figure label="Floor" value={c.floorWei ? eth(BigInt(c.floorWei), 4) : "not known"} unit={c.floorWei ? "ETH" : undefined}
          note={c.floorWei ? "The keeper's fresh price cap: what the collection's buyers may pay for one item." : "No fresh price is posted, so none is shown."} tone={c.floorWei ? undefined : "warn"} />
        <Figure label="Items held" value={c.itemsHeld ?? "not known"} note={c.custody.kind === "bitcoin" ? "Inscriptions at one Bitcoin custody address." : `NFTs at the canister's address on ${c.custody.chainName}.`} />
        <Figure label="Tokens in circulation" value={whole(c.supplyUnits)} unit={c.symbol} note="One whole token is one item, redeemable for a random one." />
        <Figure label="Redemptions in flight" value={c.openTickets.length} note="Burned, waiting for their item." />
      </dl>

      <Section title="Coins" aside="Every coin launched on this collection.">
        {c.coinsList.length === 0 ? <p className="quiet">No coin has launched on it yet.</p> : (
          <ul className="rows">
            {c.coinsList.map((x) => (
              <li key={x.address}><a className="row" href={href("coin", x.address)}>
                <span className="row__main"><span className="row__name">{x.name}</span><span className="mono quiet">{x.symbol}</span>
                  <span className={`tag${x.phase === "graduated" ? " tag--good" : x.phase === "refunding" ? " tag--warn" : ""}`}>{PHASE[x.phase]}</span></span>
                <span className="row__meta">{x.phase === "trading" && x.raisedWei ? `${eth(BigInt(x.raisedWei), 2)} of ${eth(BigInt(x.raiseWei), 2)} ETH raised · ` : ""}{x.policy === "burn" ? "its fees burn items" : x.policy === "yoyo" ? "its fees relist items and burn the coin" : "its fees raffle items"} · {(x.feeBps / 100).toFixed(x.feeBps % 100 ? 1 : 0)}% fee{x.launchedAt ? ` · launched ${when(x.launchedAt)}` : ""}</span>
              </a></li>
            ))}
          </ul>
        )}
      </Section>

      <Section title="Activity" aside="Items arriving and leaving, redemptions, raffles and burns.">
        {c.activity.length === 0 ? <p className="quiet">Nothing yet.</p> : (
          <ul className="wx-activity">
            {c.activity.map((a, i) => <li key={i}><span className={`tag wx-act--${a.kind}`}>{a.kind.replace("-", " ")}</span> {a.text} <span className="quiet small">{a.at ? when(a.at) : `block ${a.block}`}</span></li>)}
          </ul>
        )}
      </Section>

      <Section title="Your items" aside="Optional. Trading never needs any of these.">
        <div className="wx-actions">
          <a className="btn btn--quiet" href={href("redeem", c.slug)}>Redeem an item</a>
          <a className="btn btn--quiet" href={href("rent", c.slug)}>Rent your item</a>
          {c.kind === "inscriptions" ? <a className="btn btn--quiet" href={href("handins", c.slug)}>Hand in a listing</a> : null}
        </div>
      </Section>
    </>
  );
}
const PHASE: Record<string, string> = { trading: "on its curve", graduating: "buying its items", graduated: "graduated", refunding: "refunding" };
