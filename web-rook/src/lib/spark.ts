/**
 * The line's arithmetic, apart from its drawing, so both can be tested.
 *
 * The indexer timestamps settlements, not fees, so the only honest series over time is what the vaults have spent on
 * Bitcoin. Each range recomputes the series from the points inside its window: changing the range changes the line rather
 * than zooming a picture. docs/phase9.md, section 2.
 */

export type Point = { t: number; v: number };
export type Range = { key: string; label: string; seconds: number | null };

export const RANGES: Range[] = [
  { key: "1d", label: "24 hours", seconds: 86_400 },
  { key: "7d", label: "7 days", seconds: 604_800 },
  { key: "all", label: "All", seconds: null },
];

/**
 * Running totals at each event inside the window, with a zero at the window's start and the final total carried to now, so
 * the line spans the axis instead of floating in the middle of it.
 */
export function seriesFor(points: Point[], range: Range, now = Math.floor(Date.now() / 1000)): Point[] {
  const from = range.seconds === null ? -Infinity : now - range.seconds;
  const inWindow = points.filter((p) => p.t >= from && p.t <= now).sort((a, b) => a.t - b.t);
  if (!inWindow.length) return [];
  const start = range.seconds === null ? inWindow[0].t : now - range.seconds;
  const out: Point[] = [{ t: start, v: 0 }];
  let run = 0;
  for (const p of inWindow) {
    run += p.v;
    out.push({ t: p.t, v: run });
  }
  out.push({ t: now, v: run });
  return out;
}

/**
 * A running total does not slope between events: it is flat until something settles and then it jumps. Drawing it as a
 * diagonal would invent purchases that never happened, which matters most when there have only been one or two.
 */
export const stepPath = (pts: { x: number; y: number }[]): string =>
  pts
    .map((p, i) =>
      i === 0
        ? `M${p.x.toFixed(2)} ${p.y.toFixed(2)}`
        : `L${p.x.toFixed(2)} ${pts[i - 1].y.toFixed(2)} L${p.x.toFixed(2)} ${p.y.toFixed(2)}`,
    )
    .join(" ");
