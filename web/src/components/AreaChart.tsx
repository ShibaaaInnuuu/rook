import { useEffect, useRef, useState } from "react";
import {
  AreaSeries, ColorType, CrosshairMode, LineStyle, createChart, createSeriesMarkers,
  type IChartApi, type ISeriesApi, type ISeriesMarkersPluginApi, type SeriesMarker, type Time, type UTCTimestamp,
} from "lightweight-charts";

/**
 * An area chart over TradingView's lightweight-charts, and nothing else. docs/phase12.md §4.
 *
 * This file knows about a series, a label, a formatter and a set of colours. It knows nothing about coins, market caps,
 * ranges, indexers or which site it is on — deliberately, because CardPaired's own `PriceChart.tsx` has the same per-trade
 * fault this replaces, and the fix should be ported there rather than written a third time. Everything that is about *this*
 * site lives in MarketChart.tsx, which is this component's only caller here.
 *
 * The colours are a parameter for the same reason: a site with two themes cannot have its chart's palette written down in
 * the chart. The caller reads its own CSS and hands the values over, and hands them over again when the theme changes.
 */

export type Point = { time: number; value: number };
/** A moment worth marking on the line — a launch, a graduation. `at` is snapped to a drawn point by this component. */
export type Mark = { at: number; text: string; where: "above" | "below"; color: string };
/** Every colour the chart uses, so none of them is written down in here. */
export type Skin = { background: string; text: string; grid: string; border: string; line: string; fillTop: string; fillBottom: string; crosshair: string; crosshairLabel: string; font: string };
/** What the pointer is over, with the pixel it is at, so a caller can put its own tooltip there. */
export type Hovered = { time: number; value: number; x: number; y: number };

export function AreaChart({ series, label, format, marks, skin, secondsVisible, onHover, className }: {
  series: readonly Point[];
  label: string;
  format: (v: number) => string;
  marks?: readonly Mark[];
  skin: Skin;
  secondsVisible?: boolean;
  onHover?: (h: Hovered | null) => void;
  className?: string;
}) {
  const host = useRef<HTMLDivElement>(null);
  const chart = useRef<IChartApi | null>(null);
  const area = useRef<ISeriesApi<"Area"> | null>(null);
  const pins = useRef<ISeriesMarkersPluginApi<Time> | null>(null);
  const hover = useRef(onHover);
  const shown = useRef(format);
  hover.current = onHover;
  shown.current = format;
  // bumped every time a chart is built, so colours, data and marks are re-applied to a NEW one. React's development
  // double-mount tears the first chart down and builds a second; without this the second would stay empty.
  const [built, setBuilt] = useState(0);

  useEffect(() => {
    const el = host.current;
    if (!el) return;
    const c = createChart(el, { autoSize: true, handleScale: { axisPressedMouseMove: false }, localization: { locale: "en-GB" } });
    const s = c.addSeries(AreaSeries, { lineWidth: 2, priceLineVisible: false, lastValueVisible: true });
    chart.current = c; area.current = s; pins.current = createSeriesMarkers(s, []);
    const move = (p: { time?: Time; point?: { x: number; y: number }; seriesData: Map<unknown, unknown> }) => {
      const v = p.seriesData.get(s) as { value?: number } | undefined;
      hover.current?.(p.time === undefined || !p.point || v?.value === undefined ? null : { time: p.time as number, value: v.value, x: p.point.x, y: p.point.y });
    };
    c.subscribeCrosshairMove(move);
    setBuilt((n) => n + 1);
    return () => { c.unsubscribeCrosshairMove(move); c.remove(); chart.current = null; area.current = null; pins.current = null; };
  }, []);

  useEffect(() => {
    const c = chart.current, s = area.current;
    if (!c || !s) return;
    c.applyOptions({
      layout: { background: { type: ColorType.Solid, color: skin.background }, textColor: skin.text, fontFamily: skin.font, fontSize: 11, attributionLogo: false },
      grid: { vertLines: { color: skin.grid, style: LineStyle.Solid }, horzLines: { color: skin.grid, style: LineStyle.Solid } },
      rightPriceScale: { borderColor: skin.border, scaleMargins: { top: 0.14, bottom: 0.08 } },
      timeScale: { borderColor: skin.border, timeVisible: true, secondsVisible: !!secondsVisible },
      crosshair: {
        mode: CrosshairMode.Magnet,
        vertLine: { color: skin.crosshair, width: 1, style: LineStyle.Dashed, labelBackgroundColor: skin.crosshairLabel },
        horzLine: { color: skin.crosshair, width: 1, style: LineStyle.Dashed, labelBackgroundColor: skin.crosshairLabel },
      },
      localization: { locale: "en-GB", priceFormatter: (v: number) => shown.current(v) },
    });
    s.applyOptions({
      lineColor: skin.line, topColor: skin.fillTop, bottomColor: skin.fillBottom,
      crosshairMarkerBorderColor: skin.background, crosshairMarkerBackgroundColor: skin.line,
      priceFormat: { type: "custom", formatter: (v: number) => shown.current(v), minMove: 0.00000001 },
    });
  }, [built, skin, secondsVisible, format]);

  useEffect(() => {
    const c = chart.current, s = area.current;
    if (!c || !s) return;
    s.setData(series.map((p) => ({ time: p.time as UTCTimestamp, value: p.value })));
    c.timeScale().fitContent();
  }, [built, series]);

  /** A mark is snapped to a point that was actually drawn, so it sits on the line instead of beside it or nowhere at all. */
  useEffect(() => {
    const p = pins.current;
    if (!p) return;
    const out: SeriesMarker<Time>[] = [];
    for (const m of marks ?? []) {
      if (!series.length) continue;
      let best = series[0].time;
      for (const s of series) if (Math.abs(s.time - m.at) < Math.abs(best - m.at)) best = s.time;
      out.push({ time: best as UTCTimestamp, position: m.where === "above" ? "aboveBar" : "belowBar", shape: m.where === "above" ? "circle" : "arrowUp", color: m.color, text: m.text, size: 1 });
    }
    p.setMarkers(out.sort((a, b) => (a.time as number) - (b.time as number)));
  }, [built, marks, series]);

  return <div className={className} ref={host} role="img" aria-label={label} />;
}
