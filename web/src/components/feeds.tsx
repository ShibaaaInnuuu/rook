import { useEffect, useRef } from "react";
import { Ext } from "./ui";
import { href } from "../lib/env";
import { eth } from "../lib/format";
import { AssetArt, CoinArt, shortRef } from "./Plate";
import { logoSrc } from "../lib/plate";
import { ago } from "./panels";
import { freshKeys } from "../lib/feed";
import type { Event } from "../lib/activity";

/**
 * A coin's own tape used to be here, as `TradeFeed`. docs/phase13.md §7 moved it into `TradesHolders.tsx`, beside the holder
 * list and under the chart: the page had a trade list in one place and the question a reader asks straight after it — who
 * holds this — in no place at all. Nothing else ever rendered it.
 */

/** One line of what just happened, whatever kind of thing it was. */
function Line({ e }: { e: Event }) {
  if (e.kind === "launch" || e.kind === "harvest") {
    const c = e.coin;
    return (
      <>
        <span className="act__art"><CoinArt address={c.address} name={c.name} symbol={c.symbol} logo={logoSrc(c.logoUrl)} /></span>
        <span className="act__say">
          <span className="act__lead">
            {e.kind === "launch"
              ? <><a href={href("coin", c.address)}>{c.name}</a> launched, paired with {c.listingName ?? `vault ${c.listingId}`}</>
              : <><a href={href("coin", c.address)}>{c.name}</a> routed <span className="num">{eth(c.toVaultWei, 4)}</span> ETH into the {c.listingName ?? `vault ${c.listingId}`} vault</>}
          </span>
          <span className="act__meta quiet">{e.kind === "launch" ? "launch" : `harvest ${c.harvests}`} · {ago(e.at)}</span>
        </span>
      </>
    );
  }
  if (e.kind === "draw") {
    return (
      <>
        <span className="act__art">{e.d.assetRef ? <AssetArt kind={e.d.type === "distribution" ? "rune" : "inscriptions"} assetRef={e.d.assetRef} /> : <span className="plate" />}</span>
        <span className="act__say">
          <span className="act__lead">A {e.d.type === "distribution" ? "distribution" : "draw"} opened on <a href={href("listing", e.d.listingId)}>vault {e.d.listingId}</a></span>
          <span className="act__meta quiet">{e.d.state.replace(/-/g, " ")} · {ago(e.at)}</span>
        </span>
      </>
    );
  }
  const a = e.a;
  const what = a.kind === "rune" ? `${Number(a.amount).toLocaleString("en-US")} of ${a.assetRef}` : shortRef(a.assetRef);
  return (
    <>
      <span className="act__art"><AssetArt kind={a.kind} assetRef={a.assetRef} /></span>
      <span className="act__say">
        <span className="act__lead">
          {e.kind === "purchase"
            ? <><span className="num">{Number(a.satsPaid).toLocaleString("en-US")}</span> sats bought {what}</>
            : <>{what} was delivered on Bitcoin</>}
        </span>
        <span className="act__meta quiet">
          {a.coinSymbol ? <a href={href("coin", a.coin)}>{a.coinSymbol}</a> : "a coin"} fees · {ago(e.at)}
          {e.kind === "delivery" && a.delivery.deliveredTxid ? <> · <Ext href={`https://mempool.space/tx/${a.delivery.deliveredTxid}`}>the transfer</Ext></> : null}
          {e.kind === "purchase" ? <> · <Ext href={`https://mempool.space/tx/${a.btcTxid}`}>Bitcoin</Ext></> : null}
        </span>
      </span>
    </>
  );
}

/**
 * Everything happening across every coin and every vault, in one list.
 *
 * Buys and sells are absent because the indexer serves trades per coin only; see docs/phase9.md, section 10, for what would
 * put them here. The feed says so itself rather than quietly looking complete.
 */
export function ActivityFeed({ events, limit = 12 }: { events: Event[]; limit?: number }) {
  const seen = useRef<Set<string> | null>(null);
  const shown = events.slice(0, limit);
  const keys = shown.map((e) => e.id);
  const fresh = freshKeys(keys, seen.current);
  useEffect(() => { seen.current = new Set(keys); });

  if (!shown.length) {
    return <p className="empty">Nothing has happened yet. Launches, harvests, purchases, deliveries and draws all appear here as they are recorded.</p>;
  }

  return (
    <ul className="act">
      {shown.map((e) => (
        <li className={`act__row act__row--${e.kind}${fresh.has(e.id) ? " act__row--new" : ""}`} key={e.id}>
          <Line e={e} />
        </li>
      ))}
    </ul>
  );
}
