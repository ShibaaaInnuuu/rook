import { useMemo, useRef, useState } from "react";

/**
 * One line, one axis, one measure.
 *
 * The indexer timestamps settlements, not fees, so the only honest series over time is what the vaults have actually spent
 * on Bitcoin: every acquisition carries the second it settled and the sats it paid. The range chips filter that window and
 * the line is recomputed from the points inside it, so changing the range really does change the line rather than zooming a
 * picture. With nothing settled, the axis and the sentence are drawn and no line is invented. docs/phase9.md, section 2.
 */

import { seriesFor, stepPath as path, type Point, type Range } from "../lib/spark";

export { RANGES, seriesFor, type Point, type Range } from "../lib/spark";

const W = 720;
const H = 190;
const PAD = { top: 14, right: 14, bottom: 22, left: 14 };

export function Spark({ points, range, format, label }: {
  points: Point[]; range: Range; format: (v: number) => string; label: string;
}) {
  const svg = useRef<SVGSVGElement>(null);
  const [at, setAt] = useState<number | null>(null);

  const line = useMemo(() => seriesFor(points, range), [points, range]);

  const plot = useMemo(() => {
    if (line.length < 2) return null;
    const t0 = line[0].t;
    const t1 = line[line.length - 1].t;
    const span = Math.max(1, t1 - t0);
    const peak = Math.max(1, ...line.map((p) => p.v));
    const x = (t: number) => PAD.left + ((t - t0) / span) * (W - PAD.left - PAD.right);
    const y = (v: number) => H - PAD.bottom - (v / peak) * (H - PAD.top - PAD.bottom);
    const pts = line.map((p) => ({ x: x(p.t), y: y(p.v), ...p }));
    return { pts, peak, t0, t1, x, y };
  }, [line]);

  if (!plot) {
    return (
      <div className="spark spark--none">
        <svg className="spark__svg" viewBox={`0 0 ${W} ${H}`} role="img" aria-label={`No ${label} in this window`}>
          <line className="spark__base" x1={PAD.left} y1={H - PAD.bottom} x2={W - PAD.right} y2={H - PAD.bottom} />
        </svg>
        <p className="empty">Nothing settled in this window, so there is no line to draw. Widen the range, or watch the receipts below.</p>
      </div>
    );
  }

  const { pts } = plot;
  const cursor = at === null ? null : pts.reduce((best, p) => (Math.abs(p.x - at) < Math.abs(best.x - at) ? p : best), pts[0]);
  const last = pts[pts.length - 1];

  const move = (e: React.PointerEvent<SVGSVGElement>) => {
    const box = svg.current?.getBoundingClientRect();
    if (!box) return;
    setAt(((e.clientX - box.left) / box.width) * W);
  };

  return (
    <div className="spark">
      <svg
        ref={svg}
        className="spark__svg"
        viewBox={`0 0 ${W} ${H}`}
       
        role="img"
        aria-label={`${label}, ${format(last.v)} over ${range.label.toLowerCase()}`}
        onPointerMove={move}
        onPointerLeave={() => setAt(null)}
      >
        <line className="spark__base" x1={PAD.left} y1={H - PAD.bottom} x2={W - PAD.right} y2={H - PAD.bottom} />
        {/* the height means something only if the top of it is labelled, so it is: one line, one number, recessive */}
        <line className="spark__peak" x1={PAD.left} y1={PAD.top} x2={W - PAD.right} y2={PAD.top} />
        <text className="spark__peaknum" x={PAD.left} y={PAD.top - 4}>{format(plot.peak)}</text>
        <path className="spark__fill" d={`${path(pts)} L${last.x.toFixed(2)} ${H - PAD.bottom} L${pts[0].x.toFixed(2)} ${H - PAD.bottom} Z`} />
        <path className="spark__line" d={path(pts)} />
        {cursor ? (
          <>
            <line className="spark__cross" x1={cursor.x} y1={PAD.top - 6} x2={cursor.x} y2={H - PAD.bottom} />
            <circle className="spark__dot" cx={cursor.x} cy={cursor.y} r={4} />
          </>
        ) : (
          <circle className="spark__dot" cx={last.x} cy={last.y} r={4} />
        )}
      </svg>
      <p className="spark__read num" aria-live="polite">
        {cursor ? (
          <>
            <span className="spark__val">{format(cursor.v)}</span>
            <span className="spark__at"> by {new Date(cursor.t * 1000).toUTCString().slice(5, 22)} UTC</span>
          </>
        ) : (
          <>
            <span className="spark__val">{format(last.v)}</span>
            <span className="spark__at"> over the last {range.label.toLowerCase()}</span>
          </>
        )}
      </p>
    </div>
  );
}
