import { useEffect, useMemo, useRef, useState } from "react";
import { CandlestickSeries, ColorType, CrosshairMode, HistogramSeries, LineSeries, LineStyle, createChart, createSeriesMarkers, type IChartApi, type ISeriesApi, type ISeriesMarkersPluginApi, type SeriesMarker, type Time, type UTCTimestamp } from "lightweight-charts";
import { RESES, type IxCandle, type IxChart, type IxRes } from "../rook/sol/indexerTypes";
import { supplyOf } from "../rook/sol/indexer";
import { compact, priceShort } from "../rook/sol/market";
import { floorAtZero } from "../rook/chartScale";

/**
 * A coin's chart, ported from the Robinhood site's CoinChart (src/ui/Chart.tsx) to Solana: TradingView's lightweight-charts, candles
 * or a line, in price (the coin's quote per coin: its collection's wX, SOL for ROOK) or market cap (price × supply, in dollars when
 * the source has USD candles or the quote is SOL with a SOL/USD rate, else in the quote), at
 * 1m / 5m / 15m / 1h / 1d, with a volume histogram and the launch and graduation marked. Colours come from the page's tokens,
 * again whenever the theme changes. `missing` replaces the plot with a line saying why there is no chart (no indexer).
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
/** "#35C979" at alpha a → "rgba(53, 201, 121, a)" (the chart library parses no color-mix()). */
export function alpha(c: string, a: number): string {
  const m = /^#?([0-9a-f]{6})$/i.exec(c.trim());
  if (!m) return c;
  const n = parseInt(m[1], 16);
  return `rgba(${n >> 16}, ${(n >> 8) & 255}, ${n & 255}, ${a})`;
}

export function SolChart({ set, res, onRes, symbol, loading, error, missing }: { set: IxChart | null | undefined; res: IxRes; onRes: (r: IxRes) => void; symbol: string; loading: boolean; error: unknown; missing?: string | null }) {
  const host = useRef<HTMLDivElement>(null);
  const chart = useRef<IChartApi | null>(null);
  const series = useRef<ISeriesApi<"Candlestick"> | ISeriesApi<"Line"> | null>(null);
  const vol = useRef<ISeriesApi<"Histogram"> | null>(null);
  const pins = useRef<ISeriesMarkersPluginApi<Time> | null>(null);
  const [kind, setKind] = useState<Kind>("candles");
  const [unit, setUnit] = useState<Unit>("cap");
  const [built, setBuilt] = useState(0);
  const tokens = useTokens();
  // dollars: the source's own USD candles, or SOL/USD on a SOL-quoted coin; a wX-quoted coin without USD candles stays in wX
  const usd = set && set.usdPerSol !== null && (set.quote === "SOL" || (set.usd && set.usd.length === set.candles.length)) ? set.usdPerSol : null;
  const supply = set ? supplyOf(set.supplyRaw, set.decimals) : null;
  const canCap = supply !== null && supply > 0;
  const drawn: Unit = canCap ? unit : "price";
  // market cap in dollars uses the source's per-bucket USD candles when it has them, else the current rate
  const quote = set?.quote ?? "SOL";
  const fmt = useMemo(() => (v: number) => (drawn === "cap" ? (usd !== null ? compact(v, "$") : compact(v, quote)) : priceShort(v)), [drawn, usd, quote]);
  const candles = useMemo<IxCandle[]>(() => {
    if (!set) return [];
    if (drawn === "price") return set.candles;
    const s = supply ?? 1;
    if (usd !== null && set.usd && set.usd.length === set.candles.length) return set.usd.map((k, i) => ({ ...k, o: k.o * s, h: k.h * s, l: k.l * s, c: k.c * s, v: set.candles[i].v }));
    const m = s * (usd ?? 1);
    return set.candles.map((k) => ({ ...k, o: k.o * m, h: k.h * m, l: k.l * m, c: k.c * m }));
  }, [set, drawn, supply, usd]);

  useEffect(() => {
    const el = host.current; if (!el) return;
    const c = createChart(el, { autoSize: true, localization: { locale: "en-GB" } });
    chart.current = c; setBuilt((n) => n + 1);
    return () => { c.remove(); chart.current = null; series.current = null; vol.current = null; pins.current = null; };
  }, []);
  useEffect(() => {
    const c = chart.current; if (!c) return;
    if (series.current) { c.removeSeries(series.current); series.current = null; }
    if (vol.current) { c.removeSeries(vol.current); vol.current = null; }
    const s = kind === "candles"
      ? c.addSeries(CandlestickSeries, { upColor: tokens["--good"], downColor: tokens["--bad"], borderVisible: false, wickUpColor: tokens["--good"], wickDownColor: tokens["--bad"], priceLineVisible: true, priceLineColor: tokens["--muted"] })
      : c.addSeries(LineSeries, { color: tokens["--accent"], lineWidth: 2, priceLineVisible: true, priceLineColor: tokens["--muted"], crosshairMarkerBackgroundColor: tokens["--accent"] });
    const v = c.addSeries(HistogramSeries, { priceScaleId: "vol", priceFormat: { type: "volume" }, lastValueVisible: false, priceLineVisible: false });
    c.priceScale("vol").applyOptions({ scaleMargins: { top: 0.82, bottom: 0 }, visible: false });
    series.current = s; vol.current = v; pins.current = createSeriesMarkers(s, [], { autoScale: false });
    c.applyOptions({
      layout: { background: { type: ColorType.Solid, color: tokens["--surface"] }, textColor: tokens["--muted"], fontFamily: tokens["--mono"], fontSize: 11, attributionLogo: false },
      grid: { vertLines: { color: tokens["--line"], style: LineStyle.Solid }, horzLines: { color: tokens["--line"], style: LineStyle.Solid } },
      rightPriceScale: { borderColor: tokens["--line"], scaleMargins: { top: 0.1, bottom: 0.2 } },
      timeScale: { borderColor: tokens["--line"], timeVisible: true, secondsVisible: res === "1m", rightOffset: 3 },
      crosshair: { mode: CrosshairMode.Normal, vertLine: { color: tokens["--muted"], style: LineStyle.Dashed, labelBackgroundColor: tokens["--ink"] }, horzLine: { color: tokens["--muted"], style: LineStyle.Dashed, labelBackgroundColor: tokens["--ink"] } },
      localization: { locale: "en-GB", priceFormatter: fmt },
    });
    s.applyOptions({ priceFormat: { type: "custom", formatter: fmt, minMove: 1e-14 }, autoscaleInfoProvider: floorAtZero });
  }, [built, kind, tokens, fmt, res]);
  useEffect(() => {
    const c = chart.current, s = series.current, v = vol.current; if (!c || !s || !v) return;
    if (kind === "candles") (s as ISeriesApi<"Candlestick">).setData(candles.map((k) => ({ time: k.t as UTCTimestamp, open: k.o, high: k.h, low: k.l, close: k.c })));
    else (s as ISeriesApi<"Line">).setData(candles.map((k) => ({ time: k.t as UTCTimestamp, value: k.c })));
    v.setData(candles.map((k) => ({ time: k.t as UTCTimestamp, value: k.v, color: alpha(k.c >= k.o ? tokens["--good"] : tokens["--bad"], 0.35) })));
    const marks: SeriesMarker<Time>[] = [];
    const snap = (at: number) => candles.reduce((b, k) => (Math.abs(k.t - at) < Math.abs(b - at) ? k.t : b), candles[0]?.t ?? at);
    if (set?.launchedAt && candles.length && set.launchedAt >= candles[0].t - set.seconds) marks.push({ time: snap(set.launchedAt) as UTCTimestamp, position: "belowBar", shape: "arrowUp", color: tokens["--muted"], text: snap(set.launchedAt) === candles[0].t ? "" : "launched", size: 1 });
    if (set?.graduation?.at && candles.length && set.graduation.at >= candles[0].t - set.seconds) marks.push({ time: snap(set.graduation.at) as UTCTimestamp, position: "aboveBar", shape: "circle", color: tokens["--accent"], text: "graduated", size: 1 });
    pins.current?.setMarkers(marks.sort((a, b) => (a.time as number) - (b.time as number)));
    c.timeScale().fitContent();
  }, [built, kind, candles, set, tokens]);

  const last = candles[candles.length - 1], first = candles[0];
  const delta = first && last && first.o > 0 ? (last.c - first.o) / first.o : null;
  const want = RESES.find((r) => r[0] === res)?.[2];
  return (
    <div className="chart">
      <div className="chart__head">
        <div className="chart__now">
          <span className="small muted">{symbol} {drawn === "cap" ? `market cap${usd !== null ? "" : `, in ${quote}`}` : `price, ${quote} per coin`}</span>
          <span className="chart__figure num">{last ? fmt(last.c) : "—"}</span>
          {delta !== null ? <span className={`small num ${delta >= 0 ? "good" : "bad"}`}>{delta >= 0 ? "+" : ""}{(delta * 100).toFixed(1)}% over the window</span> : <span className="small muted">no move to report</span>}
        </div>
        <div className="chart__picks">
          {canCap ? <span className="seg"><button type="button" aria-pressed={drawn === "cap"} onClick={() => setUnit("cap")}>{usd !== null ? "MCAP $" : "MCAP"}</button><button type="button" aria-pressed={drawn === "price"} onClick={() => setUnit("price")}>Price</button></span> : null}
          <span className="seg"><button type="button" aria-pressed={kind === "candles"} onClick={() => setKind("candles")}>Candles</button><button type="button" aria-pressed={kind === "line"} onClick={() => setKind("line")}>Line</button></span>
          <span className="seg" role="group" aria-label="Resolution">{RESES.map(([r, label]) => <button key={r} type="button" aria-pressed={res === r} onClick={() => onRes(r)}>{label}</button>)}</span>
        </div>
      </div>
      <div className="chart__frame">
        <div className="chart__plot" ref={host} role="img" aria-label={`${symbol} chart, ${candles.length} candles`} />
        {missing ? <div className="chart__over small muted">{missing}</div>
          : loading ? <div className="chart__over small muted">Reading the trades…</div>
          : error ? <div className="chart__over small warn">This coin's chart could not be read: {String((error as Error).message).slice(0, 120)}</div>
          : set && !candles.length ? <div className="chart__over small muted">{set.tradesAllTime > 0 ? `Nothing traded in this window; ${set.tradesAllTime} trades over its life. Pick a coarser resolution.` : "This coin has never traded. The first buy draws the first candle."}</div> : null}
      </div>
      {set ? <p className="small muted chart__foot">{set.seconds === want ? `${res} candles` : `${set.seconds}-second candles`} · {set.trades} trades in the window · {set.source.kind === "fixture" ? "demo fixture" : "from Rook's indexer"}{usd !== null ? ` · $${usd.toFixed(2)} per SOL` : ""}</p> : null}
    </div>
  );
}
