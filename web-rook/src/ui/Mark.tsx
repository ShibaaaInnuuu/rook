import { BRAND_MARK } from "../rook/brand";
import { BIRDS, type BirdId, type MarkSpec } from "../rook/marks";

/**
 * The rook marks (docs/v6/site-design.md §5): four birds on one 32-unit grid, drawn from src/rook/marks.ts. Fills and strokes
 * are `currentColor`; the one accent shape is the accent; the eye is a hole (the ground). On hover the wordmark's CSS swaps
 * every fill and stroke for the iridescent gradient — the only place `--sheen-2` is used — and the eye takes the accent.
 */
type Props = { size?: number; className?: string; title?: string };

export function Bird({ spec, size = 30, className, title }: Props & { spec: MarkSpec }) {
  const grad = `rook-sheen-${spec.id}`;
  return (
    <svg className={`mark mark--${spec.id}${className ? ` ${className}` : ""}`} width={size} height={size} viewBox="0 0 32 32" role={title ? "img" : undefined} aria-hidden={title ? undefined : true} focusable="false">
      {title ? <title>{title}</title> : null}
      <defs><linearGradient id={grad} x1="0" y1="0" x2="1" y2="1"><stop offset="0" stopColor="var(--accent)" /><stop offset="1" stopColor="var(--sheen-2)" /></linearGradient></defs>
      {spec.shapes.map((s, i) => {
        const cls = s.kind === "accent" ? "mark__accent" : s.kind === "eye" ? "mark__eye" : s.kind === "stroke" ? "mark__line" : "mark__fill";
        const paint = s.kind === "stroke" ? { fill: "none", stroke: "currentColor", strokeWidth: spec.strokeWidth, strokeLinecap: "round" as const, strokeLinejoin: "round" as const }
          : s.kind === "eye" ? { fill: "var(--ground)" } : s.kind === "accent" ? { fill: "var(--accent)" } : { fill: "currentColor" };
        return s.circle ? <circle key={i} className={cls} data-sheen={grad} cx={s.circle[0]} cy={s.circle[1]} r={s.circle[2]} {...paint} /> : <path key={i} className={cls} data-sheen={grad} d={s.d} {...paint} />;
      })}
    </svg>
  );
}

const by = (id: BirdId) => BIRDS.find((b) => b.id === id)!;
export const RookPerched = (p: Props) => <Bird spec={by("perched")} {...p} />;
export const RookHead = (p: Props) => <Bird spec={by("head")} {...p} />;
export const RookGlyph = (p: Props) => <Bird spec={by("glyph")} {...p} />;
export const RookFlight = (p: Props) => <Bird spec={by("flight")} {...p} />;

/** The chess piece, built first and kept, not offered: the user chose the bird. */
export function RookPiece({ size = 30, className, title }: Props) {
  return (
    <svg className={`mark${className ? ` ${className}` : ""}`} width={size} height={size} viewBox="0 0 32 32" role={title ? "img" : undefined} aria-hidden={title ? undefined : true} focusable="false">
      {title ? <title>{title}</title> : null}
      <path className="mark__line" fill="none" stroke="currentColor" strokeWidth="2.4" strokeLinecap="round" strokeLinejoin="round" d="M8.5 6.5h4v3.5h2.5V6.5h2v3.5h2.5V6.5h4V13h-2v7.5h2.5v5h-15v-5h2.5V13h-2z" />
      <circle className="mark__accent" cx="16" cy="16.5" r="1.6" fill="var(--accent)" />
    </svg>
  );
}

/** The mark the site wears, from one constant. */
export const Mark = (p: Props) => (BRAND_MARK === "piece" ? <RookPiece {...p} /> : <Bird spec={by(BRAND_MARK)} {...p} />);
