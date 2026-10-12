import { Bird } from "../ui/Mark";
import { BRAND_MARK } from "../rook/brand";
import { BIRDS } from "../rook/marks";
import { href } from "../lib/env";

/** Four birds, side by side, at the four sizes that matter, on both grounds, with the wordmark, so one can be picked. */
const SIZES: [number, string][] = [[16, "tab"], [24, "nav"], [32, "favicon"], [96, "hero"]];
const GROUNDS = [{ name: "dark", ground: "#0B0D10", ink: "#ECEFF3", accent: "#2FD3C6" }, { name: "light", ground: "#F5F6F8", ink: "#0B0D10", accent: "#0E7C73" }];

export function Brand() {
  return (
    <>
      <header className="pagehead"><p className="eyebrow">Brand</p><h1>Four rooks</h1>
        <p className="lede">One grid, one stroke, one accent shape, and the eye as a hole in the ink. The site wears <strong>{BIRDS.find((b) => b.id === BRAND_MARK)?.name.toLowerCase() ?? BRAND_MARK}</strong> today (<span className="mono">src/rook/brand.ts</span>, one line). Hover a lockup for the sheen. Files: {BIRDS.map((b, i) => <span key={b.id}>{i ? ", " : ""}<a href={`/brand/bird-${b.id}.svg`}>bird-{b.id}.svg</a> / <a href={`/brand/favicon-${b.id}.svg`}>favicon</a></span>)}.</p></header>
      <div className="stack-lg">
        {BIRDS.map((b) => (
          <section key={b.id}>
            <div className="row" style={{ marginBottom: ".5rem" }}><h2 style={{ margin: 0 }}>{b.name}{BRAND_MARK === b.id ? <span className="pill pill--live" style={{ marginLeft: ".75rem", verticalAlign: "middle" }}>in use</span> : null}</h2></div>
            <p className="muted" style={{ maxWidth: "60ch" }}>{b.why}</p>
            <div className="brand__grounds">
              {GROUNDS.map((g) => (
                <div key={g.name} className="brand__ground" style={{ background: g.ground, color: g.ink, ["--accent" as string]: g.accent, ["--ink" as string]: g.ink, ["--ground" as string]: g.ground }}>
                  <p className="eyebrow" style={{ color: g.ink, opacity: .6 }}>{g.name} ground</p>
                  <div className="brand__row">
                    {SIZES.map(([s, label]) => <div key={s} className="brand__cell"><span className="wordmark" style={{ fontSize: s >= 96 ? "3.2rem" : s >= 32 ? "1.5rem" : s >= 24 ? "1.15rem" : "0.9rem" }}><Bird spec={b} size={s} />Rook</span><span className="small" style={{ opacity: .6 }}>{label} · {s}px</span></div>)}
                  </div>
                </div>
              ))}
            </div>
          </section>
        ))}
      </div>
      <p className="small muted" style={{ marginTop: "2rem" }}>The choice and why: <a href={href("docs", "site-design")}>site design, §5</a>.</p>
    </>
  );
}
