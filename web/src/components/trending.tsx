import { href } from "../lib/env";
import { CoinArt } from "./Plate";
import { logoSrc } from "../lib/plate";
import { cap, price } from "../lib/chart";
import { useReadings } from "../lib/caps";
import { byVolume24h, trendShortlist, volume24hOf, type CoinRow } from "../lib/catalog";

/**
 * docs/phase14.md §4. The coins that actually traded in the last day, in a rail that scrolls in its own box.
 *
 * **What the ranking is, and it is the ranking that is printed.** The strip orders by 24-hour volume and prints 24-hour
 * volume. That number comes from `volume24hWei` on the list response where the indexer serves it — one field, every coin,
 * exact — and otherwise is measured from each coin's own `range=1d` candles, which counts the same trades the other way
 * round. Measuring costs one request per coin, so in that case the sixteen candidates are shortlisted by the best key the
 * list response does carry, and the heading says so in words. Nothing here is labelled 24-hour volume that is not.
 *
 * The market cap beside it is Phase 12's: supply × the latest curve-side close, in dollars where the indexer read a rate and
 * in the pair asset where it did not. A coin whose supply has not been read has no market cap and shows its price instead.
 */
/** A volume in the pair asset. Zero is a day nobody traded, which is a figure; null is a figure nobody has. */
function vol24(v: number | null, loading: boolean): string {
  if (v === null) return loading ? "…" : "not known";
  if (v === 0) return "0 ETH";
  if (v >= 1) return `${v.toFixed(2)} ETH`;
  if (v >= 0.0001) return `${v.toPrecision(3)} ETH`;
  const [m, e] = v.toExponential(2).split("e");
  return `${m}e${e.replace("+", "")} ETH`;
}

export function Trending({ rows, take = 8, onAll }: { rows: CoinRow[]; take?: number; onAll?: () => void }) {
  const { key, rows: shortlist } = trendShortlist(rows);
  // the shortlist is already ordered when the indexer serves volume, so nothing is measured that does not have to be
  const measure = key !== "volume24h";
  const readings = useReadings(shortlist, shortlist.length > 0);
  const ordered = byVolume24h(shortlist, measure ? readings.volumes : undefined).slice(0, take);

  const say = key === "volume24h"
    ? "Ordered by the pair asset traded in the last 24 hours, as the indexer reports it."
    : `Ordered by the pair asset traded in the last 24 hours, measured from each coin's own candles. This indexer reports no per-coin volume, so the ${shortlist.length} candidates were shortlisted by ${key === "fees24h" ? "fees routed in that window" : key === "fees" ? "fees routed over its life" : "how recently it launched"}.`;

  if (!rows.length) return null;

  return (
    <section className="trend" aria-labelledby="trend-h">
      <header className="trend__head">
        <h2 id="trend-h">Trending</h2>
        {onAll ? <button type="button" className="trend__all btn--link" onClick={onAll}>View all {rows.length} coins</button> : null}
      </header>
      <p className="trend__by quiet small">{say}</p>
      <div className="trend__rail" tabIndex={0} role="group" aria-label="Trending coins">
        <ul className="trend__track">
          {ordered.map((c) => {
            const r = readings.by.get(c.address.toLowerCase());
            const vol = volume24hOf(c, measure ? readings.volumes : undefined);
            const usd = r?.capUsd ?? null;
            return (
              <li className="trend__item" key={c.address}>
                <a className="trend__in" href={href("coin", c.address)}>
                  <span className="trend__face">
                    <CoinArt address={c.address} name={c.name} symbol={c.symbol} logo={logoSrc(c.logoUrl)} />
                  </span>
                  <span className="trend__who">
                    <span className="trend__tick mono">{c.symbol}</span>
                    <span className="trend__name">{c.name}</span>
                  </span>
                  <span className="trend__cap num">
                    {usd !== null ? cap(usd, "usd") : r?.capPair !== null && r?.capPair !== undefined ? cap(r.capPair, "pair") : readings.loading ? "…" : price(r?.close)}
                  </span>
                  <span className="trend__vol quiet">
                    <span className="num">{vol24(vol, readings.loading)}</span> 24h volume
                  </span>
                </a>
              </li>
            );
          })}
        </ul>
      </div>
    </section>
  );
}
