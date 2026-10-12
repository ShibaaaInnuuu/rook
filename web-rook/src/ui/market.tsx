/** The terminal's pieces: a coin's picture, its collection badge, a sparkline, the compact coin card, a feed row, a number that
 *  counts up. Presentation only; the data comes from src/rook/sol/hooksMarket.ts. */
import { useEffect, useRef, useState } from "react";
import { href } from "../lib/env";
import { collectionBySlug } from "../rook/sol/collections";
import { age } from "../rook/sol/format";
import { changeText, compact, progressOf, solShort, type MarketCoin } from "../rook/sol/market";
import { quoteOf, raiseText } from "../rook/sol/graduation";
import type { FeedItem } from "../rook/sol/hooksMarket";
import { Picture } from "./Picture";
import { CoinFace } from "./sol";

export function CoinPic({ c, size = 40 }: { c: Pick<MarketCoin, "logoUrl" | "metadataUri" | "name" | "mint">; size?: number }) {
  return c.logoUrl ? <Picture src={c.logoUrl} seed={c.mint} name={c.name} size={size} round="22%" /> : <CoinFace uri={c.metadataUri} name={c.name} seed={c.mint} size={size} />;
}

export function CollBadge({ slug, link = false }: { slug: string; link?: boolean }) {
  const col = collectionBySlug(slug);
  const name = col?.name ?? (slug === "rook" ? "Genesis" : slug);
  const inner = <>{col ? <Picture src={col.logo} seed={col.slug} name={col.name} size={14} round="3px" /> : null}<span>{name}</span></>;
  return link && col ? <a className="cbadge" href={href("collection", slug)} title={`Launched on ${name}`}>{inner}</a> : <span className="cbadge" title={col ? `Launched on ${name}` : "The genesis coin"}>{inner}</span>;
}

/** A sparkline: a few closes as one stroke, green up, red down. */
export function Spark({ values, w = 64, h = 22 }: { values: number[] | null | undefined; w?: number; h?: number }) {
  if (!values || values.length < 2) return <span className="spark spark--none" style={{ width: w, height: h }} aria-hidden="true" />;
  const lo = Math.min(...values), hi = Math.max(...values), span = hi - lo || hi || 1;
  const pts = values.map((v, i) => `${((i / (values.length - 1)) * w).toFixed(1)},${(h - 2 - ((v - lo) / span) * (h - 4)).toFixed(1)}`).join(" ");
  const up = values[values.length - 1] >= values[0];
  return <svg className={`spark ${up ? "spark--up" : "spark--down"}`} width={w} height={h} viewBox={`0 0 ${w} ${h}`} aria-hidden="true"><polyline points={pts} fill="none" strokeWidth="1.5" strokeLinejoin="round" strokeLinecap="round" /></svg>;
}

/** A coin's market cap: in dollars when known, else in the coin's own quote (a wX-quoted coin reads as wX: its SOL and USD value
 *  need the collection's wX/SOL pair, which the board does not read). */
const capOf = (c: MarketCoin, usd: number | null) => (c.marketCapUsd !== null ? compact(c.marketCapUsd, "$") : usd !== null && c.marketCapSol !== null && c.quote.symbol === "SOL" ? compact(c.marketCapSol * usd, "$") : compact(c.marketCapSol, c.quote.symbol));

/** The terminal's coin card. `flash` is the key of the latest live trade on this coin; a new key replays the flash once. */
export function CoinCard({ c, usd, now, flash }: { c: MarketCoin; usd: number | null; now: number; flash?: { key: string; side: string } }) {
  const p = progressOf(c);
  return (
    <li className="tcard">
      <a className="tcard__in" href={href("coin", c.mint)}>
        {flash ? <i key={flash.key} className={`tcard__flash tcard__flash--${flash.side}`} aria-hidden="true" /> : null}
        <CoinPic c={c} size={44} />
        <span className="tcard__id">
          <span className="tcard__top"><strong className="tcard__sym">{c.symbol}</strong><span className="tcard__name">{c.name}</span></span>
          <span className="tcard__meta"><CollBadge slug={c.collection.slug} /><span className="num" title="Age">{age(c.launchedAt ?? 0, now)}</span>{c.holders !== null ? <span className="num tcard__hold" title={`${c.holders} holders`}><svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true"><circle cx="5" cy="3" r="2.1" fill="currentColor" /><path d="M1 9.5c.4-2.3 2-3.4 4-3.4s3.6 1.1 4 3.4z" fill="currentColor" /></svg>{c.holders}</span> : null}</span>
          {c.phase === "graduated" ? null : <span className="tcard__prog"><span className="tcard__bar" title={`${Math.round(p * 100)}% of ${raiseText(c)}`}><i style={{ width: `${Math.max(2, Math.round(p * 100))}%` }} /></span><span className="num">{Math.round(p * 100)}%</span></span>}
        </span>
        <span className="tcard__fig">
          <span className="tcard__cap num" title="Market cap">{capOf(c, usd)}</span>
          {c.phase === "graduated" ? <span className="tcard__tag">graduated</span> : c.phase === "graduating" ? <span className="tcard__tag tcard__tag--live">migrating</span> : null}
          <span className={`num small ${c.change24h === null ? "muted" : c.change24h >= 0 ? "good" : "bad"}`}>{changeText(c.change24h)}</span>
        </span>
        <Spark values={c.spark} />
      </a>
    </li>
  );
}

/** One line of the live feed: who did what on which coin, how long ago. The amount is in the coin's quote (its collection's wX;
 *  SOL for ROOK): `solLamports` carries the quote's base units. */
export function FeedRow({ a, now }: { a: FeedItem; now: number }) {
  const sol = a.solLamports !== null ? Number(a.solLamports) / 1e9 : null;
  const who = a.trader ? `${a.trader.slice(0, 4)}…${a.trader.slice(-2)}` : null;
  return (
    <li className={`feed__row feed__row--${a.kind}${a.live ? " feed__row--live" : ""}`}>
      <a href={href("coin", a.coin.address)}>
        <Picture src={a.coin.logoUrl} seed={a.coin.address} name={a.coin.name} size={22} round="50%" />
        <span className="feed__what">
          {a.kind === "buy" || a.kind === "sell" ? <><span className="mono muted">{who}</span> <span className={`feed__verb feed__verb--${a.kind}`}>{a.kind === "buy" ? "bought" : "sold"}</span> <strong className="num">{sol !== null ? solShort(sol) : "?"}</strong> <span className="muted">{quoteOf(a.coin.collection).symbol} of</span> <strong>${a.coin.symbol}</strong></>
            : a.kind === "launch" ? <><strong>${a.coin.symbol}</strong> <span className="feed__verb feed__verb--launch">launched</span></> : <><strong>${a.coin.symbol}</strong> <span className="feed__verb feed__verb--graduated">graduated</span></>}
        </span>
        <span className="feed__ago num muted">{a.at ? age(a.at, now) : ""}</span>
      </a>
    </li>
  );
}

const REDUCED = () => typeof matchMedia !== "undefined" && matchMedia("(prefers-reduced-motion: reduce)").matches;
/** A number that counts up to its new value (600 ms); immediate under reduced motion. */
export function useTween(v: number | null, ms = 650): number | null {
  const [shown, setShown] = useState(v);
  const from = useRef(v ?? 0);
  useEffect(() => {
    if (v === null) { setShown(null); return; }
    if (REDUCED()) { setShown(v); from.current = v; return; }
    const a = from.current, t0 = performance.now();
    let raf = 0;
    const step = (t: number) => { const k = Math.min(1, (t - t0) / ms), e = 1 - (1 - k) ** 3; setShown(a + (v - a) * e); if (k < 1) raf = requestAnimationFrame(step); else from.current = v; };
    raf = requestAnimationFrame(step);
    return () => { cancelAnimationFrame(raf); from.current = v; };
  }, [v, ms]);
  return shown;
}
