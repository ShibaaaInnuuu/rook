import { useEffect, useRef, useState } from "react";
import { useCandles } from "../lib/coinchart";
import { cap, capOf, change, hasUsd, pct, price, type Candle } from "../lib/chart";

/**
 * A coin card's own line, so a grid of coins reads as a market rather than a list.
 *
 * docs/phase12.md: the card says the same number the coin's own page says — a market cap, in dollars when the indexer read a
 * rate and in the pair asset when it did not — rather than a price in the range of `3e-9` that means nothing at a glance. It
 * costs nothing extra: the supply and the rate arrive on the same response as the candles.
 *
 * The candles are one request per coin, so a card asks only once it is actually on screen. With sixty coins that is the
 * difference between a handful of requests and sixty. A batch endpoint would be better and is written down for the indexer
 * in docs/phase9.md, section 10.
 */
export function CardSpark({ address, symbol }: { address: string; symbol: string }) {
  const box = useRef<HTMLSpanElement>(null);
  const [near, setNear] = useState(false);

  useEffect(() => {
    const el = box.current;
    if (!el || near) return;
    if (typeof IntersectionObserver !== "function") { setNear(true); return; }
    const io = new IntersectionObserver((entries) => {
      if (entries.some((e) => e.isIntersecting)) { setNear(true); io.disconnect(); }
    }, { rootMargin: "900px" });
    io.observe(el);
    return () => io.disconnect();
  }, [near]);

  // A day, in fifteen-minute buckets: a coin launched twenty minutes ago still gets a line, and a card should be saying what
  // the day did anyway. The indexer derives the bucket from the range, so this can never become one point per trade.
  const q = useCandles(near ? address : undefined, "1d");
  const set = q.data;
  const candles: Candle[] = set?.candles ?? [];
  const last = candles[candles.length - 1];
  const delta = change(candles);
  const unit = hasUsd(set) ? "usd" : "pair";
  const now = set ? capOf(set, last?.c, unit) : null;

  if (!near || q.isLoading) return <span className="spark-card" ref={box} aria-hidden="true" />;
  // an unanswered request is not the same fact as a coin that never traded, and a recorded demo keeps only the most recent
  // coins' charts, so a card that could not ask says so rather than claiming the coin has no trades.
  if (q.error) return <span className="spark-card spark-card--none" ref={box}><span className="spark-card__say quiet">not known</span></span>;
  if (!candles.length) {
    return (
      <span className="spark-card spark-card--none" ref={box}>
        <span className="spark-card__say quiet">{set && set.tradesAllTime > 0 ? "no trade today" : "never traded"}</span>
      </span>
    );
  }

  const w = 120, h = 30;
  let lo = Infinity, hi = -Infinity;
  for (const k of candles) { if (k.l < lo) lo = k.l; if (k.h > hi) hi = k.h; }
  // A single candle, or a price that never moved, is a flat line through the middle rather than a line on the floor.
  const flat = !(hi > lo);
  const range = flat ? 1 : hi - lo;
  const y = (c: number) => (flat ? h / 2 : h - ((c - lo) / range) * h);
  const pts = candles.length > 1 ? candles : [candles[0], candles[0]];
  const step = w / (pts.length - 1);
  const d = pts.map((k, i) => `${i ? "L" : "M"}${(i * step).toFixed(1)} ${y(k.c).toFixed(1)}`).join(" ");

  return (
    <span className="spark-card" ref={box}>
      <svg className="spark-card__svg" viewBox={`0 0 ${w} ${h}`} preserveAspectRatio="none" aria-hidden="true">
        <path className={`spark-card__line ${delta !== null && delta < 0 ? "down" : "up"}`} d={d} />
      </svg>
      <span className="spark-card__read">
        {/* a coin whose supply has not been read yet has no market cap, and says the price it does know rather than nothing */}
        <span className="num">{now === null ? price(last?.c) : cap(now, unit, set?.quote ?? "ETH")}</span>
        {delta !== null ? <span className={`num ${delta >= 0 ? "up" : "down"}`}>{pct(delta)}</span> : null}
      </span>
      <span className="vh">{symbol}{now === null ? "" : " market cap"} over the last day, {candles.length} point{candles.length === 1 ? "" : "s"}</span>
    </span>
  );
}
