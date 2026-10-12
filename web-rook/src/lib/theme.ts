import { useEffect, useState } from "react";

/**
 * The stone's own colours, as numbers a canvas can use.
 *
 * Everything on this site is drawn from the custom properties in styles/etch.css, and there are two full palettes: limestone
 * on `:root`, basalt under `prefers-color-scheme: dark` and under `[data-theme="dark"]`. A chart library cannot read a CSS
 * variable, so the values are read out of the computed style and handed over — and read again when the theme changes, whether
 * that was the reader picking one or the operating system changing its mind. Hard-coding a palette here would give one of the
 * two themes a chart it cannot see.
 */
export const TOKENS = ["--stone", "--stone-deep", "--stone-raised", "--seam", "--glint", "--chalk", "--dust", "--ember", "--ember-ink", "--good", "--bad", "--mono"] as const;
export type Token = (typeof TOKENS)[number];
export type Palette = Record<Token, string>;

/** Values that are never empty: a computed style read before the stylesheet has applied must not paint a chart in nothing. */
const FALLBACK: Palette = {
  "--stone": "#e7ebe9", "--stone-deep": "#d8dedb", "--stone-raised": "#eef1f0", "--seam": "#b3bdb9", "--glint": "#fbfcfc",
  "--chalk": "#1a2023", "--dust": "#56626a", "--ember": "#b4570a", "--ember-ink": "#fff8f0", "--good": "#2f6b4f", "--bad": "#a12a1f",
  "--mono": "ui-monospace, monospace",
};

export function readPalette(): Palette {
  if (typeof document === "undefined") return FALLBACK;
  const style = getComputedStyle(document.documentElement);
  const out = { ...FALLBACK };
  for (const t of TOKENS) {
    const v = style.getPropertyValue(t).trim();
    if (v) out[t] = v;
  }
  return out;
}

/** The palette now, and again whenever the theme changes under it. */
export function usePalette(): Palette {
  const [palette, setPalette] = useState<Palette>(readPalette);
  useEffect(() => {
    const reread = () => setPalette(readPalette());
    reread();                                                      // the stylesheet may have landed after the first render
    const mo = new MutationObserver(reread);
    mo.observe(document.documentElement, { attributes: true, attributeFilter: ["data-theme", "class", "style"] });
    const scheme = matchMedia("(prefers-color-scheme: dark)");
    scheme.addEventListener("change", reread);
    return () => { mo.disconnect(); scheme.removeEventListener("change", reread); };
  }, []);
  return palette;
}

/** A colour at an opacity, for the fill under an area line. The tokens are hex, so this stays in one place. */
export function fade(color: string, alpha: number): string {
  const hex = color.trim();
  const m = /^#([0-9a-f]{3}|[0-9a-f]{6})$/i.exec(hex);
  if (!m) return `color-mix(in srgb, ${hex} ${Math.round(alpha * 100)}%, transparent)`;
  const full = m[1].length === 3 ? m[1].split("").map((c) => c + c).join("") : m[1];
  const [r, g, b] = [0, 2, 4].map((i) => Number.parseInt(full.slice(i, i + 2), 16));
  return `rgba(${r}, ${g}, ${b}, ${alpha})`;
}
