import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { AreaChart, type Hovered, type Mark, type Point, type Skin } from "./AreaChart";
import { useEnv } from "../lib/env";
import { useCandles } from "../lib/coinchart";
import { fade, usePalette } from "../lib/theme";
import { cap, capOf, change, clock, hasUsd, initialRange, pct, RANGES, type Range, type Unit } from "../lib/chart";

/**
 * A coin's market, as a market cap over time. docs/phase12.md.
 *
 * Three things changed from the chart this replaces, and only the third is cosmetic.
 *
 * **The unit.** It plotted `pair asset per whole coin`, which on a bonding curve is a number like `3.133e-9`. Nobody holds a
 * mental model of that, so the axis carried no meaning and a reader could not tell a good day from a bad one. This plots
 * `supply × price × dollars per ETH`. The dollar rate is a spot read of **one** on-chain pool, not a market data feed, and
 * when it is not there the axis falls back to the pair asset and says which one it is drawing.
 *
 * **The bucket.** `res=trade` gave every trade its own bar, so the change between two bars was whatever the next buy cost —
 * a `+82.3%` that was true and answered no question anyone had asked. Now the reader picks a window, the indexer derives the
 * bucket, and the change figure is over that whole window.
 *
 * **The drawing.** AreaChart.tsx, which is lightweight-charts and nothing site-specific, handed the stone's own colours out
 * of styles/etch.css so the chart is the same material as the page in either theme.
 */

export function MarketChart({ address, symbol }: { address: string; symbol: string }) {
  const env = useEnv();
  const palette = usePalette();
  const [range, setRange] = useState<Range>("1d");
  const [chosen, setChosen] = useState(false);
  const [unit, setUnit] = useState<Unit>("usd");
  const [hover, setHover] = useState<Hovered | null>(null);
  const frame = useRef<HTMLDivElement>(null);

  const q = useCandles(address, range);
  const set = q.data;
  const candles = useMemo(() => set?.candles ?? [], [set]);

  /** A coin's age decides what it opens on, and only the answer knows its age. Once the reader picks, this stops guessing. */
  useEffect(() => {
    if (chosen || !set?.launchedAt) return;
    const want = initialRange(set.launchedAt);
    if (want !== range) setRange(want);
  }, [set?.launchedAt, chosen, range]);

  /** No rate, no dollars: the option is not offered, and a reader already on it is moved rather than shown an empty axis. */
  const usd = hasUsd(set);
  useEffect(() => { if (!usd && unit === "usd" && set) setUnit("pair"); }, [usd, unit, set]);
  const drawn: Unit = usd ? unit : "pair";
  const quote = set?.quote ?? "ETH";

  const series = useMemo<Point[]>(() => {
    if (!set) return [];
    const out: Point[] = [];
    for (const k of candles) {
      const value = capOf(set, k.c, drawn);
      if (value === null) continue;                            // a candle we cannot price is left out, never drawn as zero
      if (out.length && out[out.length - 1].time === k.t) out[out.length - 1] = { time: k.t, value };
      else out.push({ time: k.t, value });
    }
    return out;
  }, [candles, set, drawn]);

  const marks = useMemo<Mark[]>(() => {
    if (!set || !series.length) return [];
    const inside = (t: number) => t >= series[0].time - (set.seconds || 1) && t <= series[series.length - 1].time + (set.seconds || 1);
    const out: Mark[] = [];
    if (set.launchedAt && inside(set.launchedAt)) out.push({ at: set.launchedAt, text: "launched", where: "below", color: palette["--dust"] });
    if (set.graduation && inside(set.graduation.at)) out.push({ at: set.graduation.at, text: "graduated", where: "above", color: palette["--ember"] });
    return out;
  }, [set, series, palette]);

  const skin = useMemo<Skin>(() => ({
    background: palette["--stone-deep"], text: palette["--dust"], grid: fade(palette["--seam"], 0.55), border: palette["--seam"],
    line: palette["--ember"], fillTop: fade(palette["--ember"], 0.34), fillBottom: fade(palette["--ember"], 0.02),
    crosshair: palette["--dust"], crosshairLabel: palette["--chalk"], font: palette["--mono"],
  }), [palette]);

  const format = useCallback((v: number) => cap(v, drawn, quote), [drawn, quote]);

  const pick = (r: Range) => { setChosen(true); setRange(r); setHover(null); };
  const last = candles[candles.length - 1];
  const nowCap = set ? capOf(set, last?.c, drawn) : null;
  const shownCap = hover ? hover.value : nowCap;
  const shownAt = hover ? hover.time : last?.t ?? null;
  const delta = change(candles);
  const covers = RANGES.find((r) => r.key === range)?.covers ?? "this range";
  const bucket = set?.seconds ?? 300;
  const staging = env?.net === "staging";
  const width = frame.current?.clientWidth ?? 0;

  return (
    <div className="mcap">
      <div className="mcap__head">
        <div className="mcap__now">
          <span className="mcap__label quiet">
            {symbol} market cap <span className="mcap__unit">{drawn === "usd" ? "in US dollars" : `in ${quote}`}</span>
          </span>
          <span className="mcap__figure num">{cap(shownCap, drawn, quote)}</span>
          <span className="mcap__when quiet num">
            {shownAt === null ? "no trade to stand on yet" : clock(shownAt, bucket)}
            {delta !== null ? <> · <span className={`mcap__delta ${delta >= 0 ? "up" : "down"}`}>{pct(delta)}</span> over {covers}</> : null}
          </span>
        </div>
        <div className="mcap__picks">
          {usd ? (
            <div className="chips chips--unit" role="group" aria-label="Unit">
              <button type="button" className={`chip${drawn === "usd" ? " chip--on" : ""}`} aria-pressed={drawn === "usd"} onClick={() => setUnit("usd")}>USD</button>
              <button type="button" className={`chip${drawn === "pair" ? " chip--on" : ""}`} aria-pressed={drawn === "pair"} onClick={() => setUnit("pair")}>{quote}</button>
            </div>
          ) : null}
          <div className="chips" role="tablist" aria-label="Range">
            {RANGES.map((r) => (
              <button key={r.key} type="button" role="tab" aria-selected={range === r.key} className={`chip${range === r.key ? " chip--on" : ""}`} onClick={() => pick(r.key)}>{r.label}</button>
            ))}
          </div>
        </div>
      </div>

      <div className="mcap__frame" ref={frame}>
        <AreaChart
          className="mcap__plot"
          series={series}
          label={`${symbol} market cap over ${covers}, ${series.length} point${series.length === 1 ? "" : "s"}`}
          format={format}
          marks={marks}
          skin={skin}
          secondsVisible={bucket <= 15}
          onHover={setHover}
        />
        {hover ? (
          // clamped to the frame, so a reading near the right edge never pushes the page sideways
          <div className="mcap__tip num" style={{ left: `${Math.max(64, Math.min(width - 64, hover.x))}px`, top: `${hover.y}px` }} aria-hidden="true">
            <b>{cap(hover.value, drawn, quote)}</b>
            <span className="quiet">{clock(hover.time, bucket)}</span>
          </div>
        ) : null}
        {q.isLoading ? <div className="mcap__over" role="status">Reading this coin's trades…</div>
          : q.error ? (
            // a recorded demo keeps the most recent coins only, so an older one has no chart here. The reason is printed
            // rather than left as an empty frame: an unanswered request reads as "not known", never as a coin that is flat.
            <div className="mcap__over">
              <p className="empty">This coin's market cap is not known here: {(q.error as Error).message}</p>
            </div>
          )
          : !set ? null
          : !series.length ? (
            <div className="mcap__over">
              <p className="empty">
                {set.tradesAllTime > 0
                  ? <>Nothing traded in {covers}. This coin has {set.tradesAllTime} trade{set.tradesAllTime === 1 ? "" : "s"} in all — widen the range.</>
                  : <>This coin has never traded. The first buy draws the first point here, and until then there is no price to show and none will be invented.</>}
              </p>
            </div>
          ) : null}
      </div>

      <p className="mcap__foot quiet small">
        Market cap is this coin's whole supply — <span className="num">{set?.supplyWei ? (Number(set.supplyWei) / 1e18).toLocaleString("en-US", { maximumFractionDigits: 0 }) : "not read"}</span>{" "}
        {symbol}, read from <span className="mono">totalSupply()</span> on chain — times the curve-side price of one coin
        {drawn === "usd" ? <>, times <span className="num">${set?.usdPerEth?.toFixed(2)}</span> per ETH</> : null}.
        {drawn === "usd"
          ? <> That rate is a <strong>spot read of one on-chain pool</strong> — the WETH/USDG pool's <span className="mono">slot0()</span>, read every few minutes — and not a market data feed. </>
          : <> No dollar rate is in this figure: the axis is in {quote}. </>}
        The price is taken curve-side, before Pons's fee and this coin's creator tax, so the line is the market rather than the
        fee. A bucket nobody traded in is left out rather than filled in.
        {staging ? <> <strong>On staging none of this is real money:</strong> the coins, the trades and the prices behind this figure are invented on a chain we run, so the dollars are invented too, however real the rate itself is.</> : null}
      </p>
    </div>
  );
}
