import { useEffect, useMemo, useRef, useState } from "react";
import { CandlestickSeries, ColorType, CrosshairMode, LineSeries, LineStyle, createChart, createSeriesMarkers, type IChartApi, type ISeriesApi, type ISeriesMarkersPluginApi, type SeriesMarker, type Time, type UTCTimestamp } from "lightweight-charts";
import type { RookCandle, RookChart, RookRes } from "../rook/apiTypes";
import { RESES } from "../rook/data";
import { capText, priceText } from "../rook/boardlogic";
import { floorAtZero } from "../rook/chartScale";

/**
 * The coin's chart over TradingView's lightweight-charts: candles or a line, in price (the pair asset per coin) or market cap
 * (supply × price, in dollars when the indexer read a rate), at 1m / 5m / 15m / 1h / 1d. The colours are read from the page's
 * own tokens, again whenever the theme changes, so the chart is the same material as the page.
 */
type Unit = "price" | "cap";
type Kind = "candles" | "line";
const TOKENS = ["--surface", "--muted", "--line", "--ink", "--accent", "--good", "--bad", "--mono"] as const;
function readTokens(): Record<(typeof TOKENS)[number], string> {
  const st = getComputedStyle(document.documentElement);
  const out = {} as Record<(typeof TOKENS)[number], string>;
  for (const t of TOKENS) out[t] = st.getPropertyValue(t).trim() || "#888";
  return out;
}
function useTokens() {
  const [t, setT] = useState(readTokens);
  useEffect(() => {
    const re = () => setT(readTokens());
    const mo = new MutationObserver(re); mo.observe(document.documentElement, { attributes: true, attributeFilter: ["data-theme"] });
    const mq = matchMedia("(prefers-color-scheme: dark)"); mq.addEventListener("change", re);
    return () => { mo.disconnect(); mq.removeEventListener("change", re); };
  }, []);
  return t;
}

export function CoinChart({ set, res, onRes, symbol, loading, error }: { set: RookChart | undefined; res: RookRes; onRes: (r: RookRes) => void; symbol: string; loading: boolean; error: unknown }) {
  const host = useRef<HTMLDivElement>(null);
  const chart = useRef<IChartApi | null>(null);
  const series = useRef<ISeriesApi<"Candlestick"> | ISeriesApi<"Line"> | null>(null);
  const pins = useRef<ISeriesMarkersPluginApi<Time> | null>(null);
  const [kind, setKind] = useState<Kind>("candles");
  const [unit, setUnit] = useState<Unit>("cap");
  const [built, setBuilt] = useState(0);
  const tokens = useTokens();
  const usd = set ? set.usdPerEth : null, supply = set?.supplyWei ? Number(BigInt(set.supplyWei)) / 1e18 : null;
  const canCap = supply !== null && supply > 0;
  const drawn: Unit = canCap ? unit : "price";
  const scale = drawn === "cap" ? (supply ?? 1) * (usd ?? 1) : 1;
  const fmt = useMemo(() => (v: number) => (drawn === "cap" ? (usd !== null ? capText(v / usd, usd) : capText(v, null)) : priceText(v)), [drawn, usd]);
  const candles = useMemo<RookCandle[]>(() => (set?.candles ?? []).map((k) => ({ ...k, o: k.o * scale, h: k.h * scale, l: k.l * scale, c: k.c * scale })), [set, scale]);

  useEffect(() => {
    const el = host.current; if (!el) return;
    const c = createChart(el, { autoSize: true, localization: { locale: "en-GB" } });
    chart.current = c; setBuilt((n) => n + 1);
    return () => { c.remove(); chart.current = null; series.current = null; pins.current = null; };
  }, []);
  // the series is rebuilt when the kind changes; colours and data are applied to whichever one is there
  useEffect(() => {
    const c = chart.current; if (!c) return;
    if (series.current) { c.removeSeries(series.current); series.current = null; }
    const s = kind === "candles"
      ? c.addSeries(CandlestickSeries, { upColor: tokens["--good"], downColor: tokens["--bad"], borderVisible: false, wickUpColor: tokens["--good"], wickDownColor: tokens["--bad"], priceLineVisible: false })
      : c.addSeries(LineSeries, { color: tokens["--accent"], lineWidth: 2, priceLineVisible: false, crosshairMarkerBackgroundColor: tokens["--accent"] });
    series.current = s; pins.current = createSeriesMarkers(s, [], { autoScale: false });
    c.applyOptions({
      layout: { background: { type: ColorType.Solid, color: tokens["--surface"] }, textColor: tokens["--muted"], fontFamily: tokens["--mono"], fontSize: 11, attributionLogo: false },
      grid: { vertLines: { color: tokens["--line"], style: LineStyle.Solid }, horzLines: { color: tokens["--line"], style: LineStyle.Solid } },
      rightPriceScale: { borderColor: tokens["--line"], scaleMargins: { top: 0.12, bottom: 0 } },
      timeScale: { borderColor: tokens["--line"], timeVisible: true, secondsVisible: res === "1m" },
      crosshair: { mode: CrosshairMode.Normal, vertLine: { color: tokens["--muted"], style: LineStyle.Dashed, labelBackgroundColor: tokens["--ink"] }, horzLine: { color: tokens["--muted"], style: LineStyle.Dashed, labelBackgroundColor: tokens["--ink"] } },
      localization: { locale: "en-GB", priceFormatter: fmt },
    });
    // a price and a market cap are never negative, so neither is the axis: the air under the lowest candle is added here and stops at 0
    // (the scale's own bottom margin would reach below it, and once showed −5.00 ETH)
    s.applyOptions({ priceFormat: { type: "custom", formatter: fmt, minMove: 1e-12 }, autoscaleInfoProvider: floorAtZero });
  }, [built, kind, tokens, fmt, res]);
  useEffect(() => {
    const c = chart.current, s = series.current; if (!c || !s) return;
    if (kind === "candles") (s as ISeriesApi<"Candlestick">).setData(candles.map((k) => ({ time: k.t as UTCTimestamp, open: k.o, high: k.h, low: k.l, close: k.c })));
    else (s as ISeriesApi<"Line">).setData(candles.map((k) => ({ time: k.t as UTCTimestamp, value: k.c })));
    const marks: SeriesMarker<Time>[] = [];
    const snap = (at: number) => candles.reduce((b, k) => (Math.abs(k.t - at) < Math.abs(b - at) ? k.t : b), candles[0]?.t ?? at);
    if (set?.launchedAt && candles.length && set.launchedAt >= candles[0].t - set.seconds) marks.push({ time: snap(set.launchedAt) as UTCTimestamp, position: "belowBar", shape: "arrowUp", color: tokens["--muted"], text: "launched", size: 1 });
    if (set?.graduation?.at && candles.length) marks.push({ time: snap(set.graduation.at) as UTCTimestamp, position: "aboveBar", shape: "circle", color: tokens["--accent"], text: "graduated", size: 1 });
    pins.current?.setMarkers(marks.sort((a, b) => (a.time as number) - (b.time as number)));
    c.timeScale().fitContent();
  }, [built, kind, candles, set, tokens]);

  const last = candles[candles.length - 1];
  const first = candles[0];
  const delta = first && last && first.o > 0 ? (last.c - first.o) / first.o : null;
  return (
    <div className="chart">
      <div className="chart__head">
        <div className="chart__now">
          <span className="small muted">{symbol} {drawn === "cap" ? `market cap${usd !== null ? "" : ", in ETH"}` : "price, ETH per coin"}</span>
          <span className="chart__figure num">{last ? fmt(last.c) : "—"}</span>
          {delta !== null ? <span className={`small num ${delta >= 0 ? "good" : "bad"}`}>{delta >= 0 ? "+" : ""}{(delta * 100).toFixed(1)}% over the window</span> : <span className="small muted">no move to report</span>}
        </div>
        <div className="chart__picks">
          {canCap ? <span className="seg"><button type="button" aria-pressed={drawn === "cap"} onClick={() => setUnit("cap")}>{usd !== null ? "MCAP $" : "MCAP"}</button><button type="button" aria-pressed={drawn === "price"} onClick={() => setUnit("price")}>Price</button></span> : null}
          <span className="seg"><button type="button" aria-pressed={kind === "candles"} onClick={() => setKind("candles")}>Candles</button><button type="button" aria-pressed={kind === "line"} onClick={() => setKind("line")}>Line</button></span>
          <span className="seg" role="tablist" aria-label="Resolution">{RESES.map(([r, label]) => <button key={r} type="button" role="tab" aria-pressed={res === r} onClick={() => onRes(r)}>{label}</button>)}</span>
        </div>
      </div>
      <div className="chart__frame">
        <div className="chart__plot" ref={host} role="img" aria-label={`${symbol} chart, ${candles.length} candles`} />
        {loading ? <div className="chart__over small muted">Reading the trades…</div>
          : error ? <div className="chart__over small warn">This coin's chart is not known here: {String((error as Error).message).slice(0, 120)}</div>
          : set && !candles.length ? <div className="chart__over small muted">{set.tradesAllTime > 0 ? `Nothing traded in this window; ${set.tradesAllTime} trades over its life. Pick a coarser resolution.` : "This coin has never traded. The first buy draws the first candle."}</div> : null}
      </div>
      {set ? <p className="small muted chart__foot">{set.seconds === ({ "1m": 60, "5m": 300, "15m": 900, "1h": 3600, "1d": 86400 } as Record<string, number>)[res] ? `${res} candles` : `${set.seconds}-second candles (the nearest this source recorded)`} · {set.trades} trades in the window · {set.source.kind === "fixture" ? "fixture" : "live"}{usd !== null ? ` · $${usd.toFixed(0)} per ETH from one on-chain pool, not a feed` : ""}</p> : null}
    </div>
  );
}
