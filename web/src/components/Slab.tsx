import { useEffect, useRef } from "react";

/**
 * The hero: the word "Rook" V-cut into a slab, rendered on a canvas at device resolution.
 *
 * A V-cut is two flat planes meeting at an arris, so its depth at any point is its distance to the nearest edge of the letter. The
 * glyphs are rasterised, a Euclidean distance transform gives that depth, and the gradient of the depth is the plane's slope: a
 * constant direction per plane, which is what makes each plane one flat tone with a sharp line where two meet. Each pixel inside a
 * letter is then lit by one distant light (Lambert), darkened a little with depth, and given a hairline of specular where the
 * gradient collapses, which is the arris. The stone is fine per-pixel grain over a faint large-scale drift.
 *
 * The distance field is computed once per size; moving the light only re-shades the pixels inside the letters, in a frame callback
 * that stops when the light settles or the slab leaves the screen. Reduced motion, touch and no pointer all get the resting frame.
 * Under the canvas sits the same word set in CSS, so there is something cut there before the first frame and without scripts.
 */
export const WORD = "Rook";
const REST_AZ = 222, SWING = 26, ELEV = 34, SLOPE = 1.35;
type Palette = { stone: [number, number, number]; shadow: [number, number, number]; lit: [number, number, number]; grain: number };

const hex = (s: string): [number, number, number] => { const n = parseInt(s.trim().replace("#", ""), 16); return [(n >> 16) & 255, (n >> 8) & 255, n & 255]; };
function palette(el: Element): Palette {
  const c = getComputedStyle(el), v = (k: string, d: string) => c.getPropertyValue(k) || d;
  return { stone: hex(v("--slab", "#20282c")), shadow: hex(v("--slab-shadow", "#050708")), lit: hex(v("--slab-lit", "#b9c4c6")), grain: Number(v("--slab-grain", "7")) };
}

/** Felzenszwalb & Huttenlocher: exact squared Euclidean distance transform, one dimension at a time. */
function edt1d(f: Float32Array, n: number, d: Float32Array, v: Int32Array, z: Float32Array) {
  let k = 0; v[0] = 0; z[0] = -Infinity; z[1] = Infinity;
  for (let q = 1; q < n; q++) {
    let s = 0;
    do { const p = v[k]; s = (f[q] + q * q - (f[p] + p * p)) / (2 * q - 2 * p); } while (s <= z[k] && --k >= 0);
    k++; v[k] = q; z[k] = s; z[k + 1] = Infinity;
  }
  k = 0;
  for (let q = 0; q < n; q++) { while (z[k + 1] < q) k++; const p = v[k]; d[q] = (q - p) * (q - p) + f[p]; }
}
function distanceInside(alpha: Uint8ClampedArray, w: number, h: number): Float32Array {
  const INF = 1e12, grid = new Float32Array(w * h), n = Math.max(w, h);
  // distance to the nearest point OUTSIDE the letter. An edge pixel starts at its own sub-pixel distance, taken from its coverage
  // (the anti-aliased transform used for signed distance fonts): without it the field has a staircase that lights as bands.
  for (let i = 0; i < w * h; i++) { const a = alpha[i * 4 + 3] / 255; grid[i] = a >= 1 ? INF : a <= 0 ? 0 : Math.max(0, a - 0.5) ** 2; }
  const f = new Float32Array(n), d = new Float32Array(n), v = new Int32Array(n), z = new Float32Array(n + 1);
  for (let x = 0; x < w; x++) { for (let y = 0; y < h; y++) f[y] = grid[y * w + x]; edt1d(f, h, d, v, z); for (let y = 0; y < h; y++) grid[y * w + x] = d[y]; }
  for (let y = 0; y < h; y++) { for (let x = 0; x < w; x++) f[x] = grid[y * w + x]; edt1d(f, w, d, v, z); for (let x = 0; x < w; x++) grid[y * w + x] = Math.sqrt(d[x]); }
  return grid;
}

export function Slab() {
  const host = useRef<HTMLDivElement>(null);
  const canvas = useRef<HTMLCanvasElement>(null);
  useEffect(() => {
    const el = host.current, cv = canvas.current;
    if (!el || !cv) return;
    const ctx = cv.getContext("2d", { willReadFrequently: false });
    if (!ctx) return;
    const still = matchMedia("(prefers-reduced-motion: reduce)").matches;
    let dead = false, frame = 0, seen = true, frames = 0, lastShade = 0, built = false, az = still ? REST_AZ : REST_AZ + 70, target = REST_AZ;
    let shade: ((azimuth: number) => void) | null = null;

    async function build() {
      const t0 = performance.now();
      const face = '400 200px "Tenor Sans"';
      if (!document.fonts.check(face)) { try { await document.fonts.load(face); } catch { /* the fallback face will be cut instead */ } }
      if (dead || !el || !cv || !ctx) return;
      const tFont = performance.now();
      const box = el.getBoundingClientRect(), dpr = Math.min(window.devicePixelRatio || 1, 2);
      const w = Math.max(2, Math.round(box.width * dpr)), h = Math.max(2, Math.round(box.height * dpr));
      cv.width = w; cv.height = h;
      const pal = palette(el);

      // WHERE the word is: read from the CSS word under the canvas, so the two coincide. Its left edge is the text origin; a
      // zero-size inline probe at the end of it sits exactly on its baseline. No arithmetic about line boxes, nothing to drift.
      const word = el.querySelector<HTMLElement>(".slab__word")!, probe = word.querySelector<HTMLElement>(".slab__base")!, cs = getComputedStyle(word);
      const fontPx = parseFloat(cs.fontSize) * dpr, left = (word.getBoundingClientRect().left - box.left) * dpr, baseline = (probe.getBoundingClientRect().top - box.top) * dpr;
      const tracking = (parseFloat(cs.letterSpacing) || 0) * dpr;
      const setType = (c: CanvasRenderingContext2D | OffscreenCanvasRenderingContext2D, scale: number) => {
        c.font = `400 ${fontPx * scale}px "Tenor Sans", Optima, "Gill Sans", sans-serif`; c.textBaseline = "alphabetic"; c.fillStyle = "#000";
        try { (c as CanvasRenderingContext2D & { letterSpacing: string }).letterSpacing = `${tracking * scale}px`; } catch { /* older engines: no tracking */ }
      };
      setType(ctx, 1);
      const m = ctx.measureText(WORD), pad = Math.ceil(4 * dpr);
      const bx = Math.max(0, Math.floor(left - m.actualBoundingBoxLeft) - pad), by = Math.max(0, Math.floor(baseline - m.actualBoundingBoxAscent) - pad);
      const bw = Math.min(w - bx, Math.ceil(m.actualBoundingBoxLeft + m.actualBoundingBoxRight + tracking * WORD.length) + 2 * pad), bh = Math.min(h - by, Math.ceil(m.actualBoundingBoxAscent + m.actualBoundingBoxDescent) + 2 * pad);
      if (bw < 4 || bh < 4) return;

      // THE STONE, drawn by the GPU: a flat tone, a tile of fine grain repeated, and the light's fall-off as one gradient
      ctx.fillStyle = `rgb(${pal.stone.join(",")})`; ctx.fillRect(0, 0, w, h);
      const T = 192, tile = document.createElement("canvas"); tile.width = T; tile.height = T;
      const tctx = tile.getContext("2d")!, timg = tctx.createImageData(T, T);
      let seed = 0x2f6e2b1; const rnd = () => { seed ^= seed << 13; seed ^= seed >>> 17; seed ^= seed << 5; return (seed >>> 0) / 4294967296; };
      const g = pal.grain * (dpr > 1 ? 1.25 : 1);
      for (let k = 0; k < T * T; k++) { const n = (rnd() + rnd() - 1) * g; timg.data[k * 4] = timg.data[k * 4 + 1] = timg.data[k * 4 + 2] = n > 0 ? 255 : 0; timg.data[k * 4 + 3] = Math.min(255, Math.abs(n) * 2.6); }
      tctx.putImageData(timg, 0, 0);
      ctx.fillStyle = ctx.createPattern(tile, "repeat")!; ctx.fillRect(0, 0, w, h);
      const fall = ctx.createLinearGradient(0, 0, w * 0.75, h); fall.addColorStop(0, "rgba(255,255,255,0.045)"); fall.addColorStop(0.45, "rgba(0,0,0,0)"); fall.addColorStop(1, "rgba(0,0,0,0.16)");
      ctx.fillStyle = fall; ctx.fillRect(0, 0, w, h);
      const img = ctx.getImageData(bx, by, bw, bh), px = img.data, base = new Uint8ClampedArray(px);

      // THE CUT. Coverage at full resolution, for a crisp edge; depth from a distance transform at REDUCED resolution, because a
      // distance field is smooth and upsamples exactly (this is what took the first carved frame from ~490 ms to well under 100).
      const full = new OffscreenCanvas(bw, bh).getContext("2d")!; setType(full, 1); full.fillText(WORD, left - bx, baseline - by);
      const cov = full.getImageData(0, 0, bw, bh).data;
      const f = Math.min(1, Math.max(0.25, 380 / fontPx)), lw = Math.ceil(bw * f) + 2, lh = Math.ceil(bh * f) + 2;
      const low = new OffscreenCanvas(lw, lh).getContext("2d")!; setType(low, f); low.fillText(WORD, (left - bx) * f, (baseline - by) * f);
      const dist = distanceInside(low.getImageData(0, 0, lw, lh).data, lw, lh);
      // two passes of a 3x3 mean: the medial axis of a curve wanders between low-resolution pixels, and unsmoothed that lights as ripples
      let sm = dist;
      for (let pass = 0; pass < 2; pass++) { const o = new Float32Array(lw * lh);
        for (let y = 1; y < lh - 1; y++) for (let x = 1; x < lw - 1; x++) { const i = y * lw + x; o[i] = (sm[i - lw - 1] + sm[i - lw] + sm[i - lw + 1] + sm[i - 1] + sm[i] + sm[i + 1] + sm[i + lw - 1] + sm[i + lw] + sm[i + lw + 1]) / 9; }
        sm = o; }
      const GX = new Float32Array(lw * lh), GY = new Float32Array(lw * lh); let maxD = 1;
      for (let y = 2; y < lh - 2; y++) for (let x = 2; x < lw - 2; x++) { const i = y * lw + x; GX[i] = (sm[i + 2] - sm[i - 2]) / 4; GY[i] = (sm[i + 2 * lw] - sm[i - 2 * lw]) / 4; if (sm[i] > maxD) maxD = sm[i]; }

      let count = 0; for (let k = 3; k < cov.length; k += 4) if (cov[k] !== 0) count++;
      const I = new Int32Array(count), nx = new Float32Array(count), ny = new Float32Array(count), nz = new Float32Array(count), ao = new Float32Array(count), ridge = new Float32Array(count), cv8 = new Float32Array(count);
      let n = 0;
      for (let y = 0; y < bh; y++) {
        const fy = Math.min(lh - 2, y * f), y0 = fy | 0, ty = fy - y0;
        for (let x = 0; x < bw; x++) {
          const a = cov[(y * bw + x) * 4 + 3]; if (a === 0) continue;
          const fx = Math.min(lw - 2, x * f), x0 = fx | 0, tx = fx - x0, i00 = y0 * lw + x0, w00 = (1 - tx) * (1 - ty), w10 = tx * (1 - ty), w01 = (1 - tx) * ty, w11 = tx * ty;
          const gx = GX[i00] * w00 + GX[i00 + 1] * w10 + GX[i00 + lw] * w01 + GX[i00 + lw + 1] * w11, gy = GY[i00] * w00 + GY[i00 + 1] * w10 + GY[i00 + lw] * w01 + GY[i00 + lw + 1] * w11;
          const d = sm[i00] * w00 + sm[i00 + 1] * w10 + sm[i00 + lw] * w01 + sm[i00 + lw + 1] * w11, mag = Math.hypot(gx, gy), inv = 1 / Math.hypot(SLOPE * gx, SLOPE * gy, 1);
          I[n] = y * bw + x; nx[n] = SLOPE * gx * inv; ny[n] = SLOPE * gy * inv; nz[n] = inv; ao[n] = 1 - 0.34 * Math.min(1, d / maxD);
          ridge[n] = d > 1.2 ? Math.max(0, 1 - mag / 0.6) : 0; cv8[n] = a / 255; n++;
        }
      }
      const flat = Math.sin((ELEV * Math.PI) / 180), ce = Math.cos((ELEV * Math.PI) / 180);

      // LIGHTING ONLY, per frame: the pixels inside the letters, written back to the letters' own rectangle
      shade = (azimuth: number) => {
        const r = (azimuth * Math.PI) / 180, lx = Math.cos(r) * ce, ly = Math.sin(r) * ce;     // screen space, y down: 222 degrees is up and to the left
        for (let k = 0; k < count; k++) {
          const t = (Math.max(0, nx[k] * lx + ny[k] * ly + nz[k] * flat) / flat) * ao[k];         // 1 on the uncut face
          let r0: number, g0: number, b0: number;
          if (t < 1) { const u = t * t * (3 - 2 * t); r0 = pal.shadow[0] + (pal.stone[0] - pal.shadow[0]) * u; g0 = pal.shadow[1] + (pal.stone[1] - pal.shadow[1]) * u; b0 = pal.shadow[2] + (pal.stone[2] - pal.shadow[2]) * u; }
          else { const u = Math.min(1, (t - 1) / 0.55); r0 = pal.stone[0] + (pal.lit[0] - pal.stone[0]) * u; g0 = pal.stone[1] + (pal.lit[1] - pal.stone[1]) * u; b0 = pal.stone[2] + (pal.lit[2] - pal.stone[2]) * u; }
          const s2 = ridge[k] * 0.32;                                            // a hairline of light along the arris
          r0 += (pal.lit[0] - r0) * s2; g0 += (pal.lit[1] - g0) * s2; b0 += (pal.lit[2] - b0) * s2;
          const p = I[k] * 4, c = cv8[k], grainHere = (base[p] - pal.stone[0]) * 0.5;
          px[p] = base[p] + (r0 + grainHere - base[p]) * c; px[p + 1] = base[p + 1] + (g0 + grainHere - base[p + 1]) * c; px[p + 2] = base[p + 2] + (b0 + grainHere - base[p + 2]) * c;
        }
        ctx.putImageData(img, bx, by);
        frames++;
      };
      const tField = performance.now();
      shade(az);
      cv.dataset.ready = "1";
      const tDone = performance.now();
      (window as unknown as { __etchSlab: unknown }).__etchSlab = { fontMs: Math.round(tFont - t0), fieldMs: Math.round(tField - tFont), firstShadeMs: +(tDone - tField).toFixed(1), readyAt: Math.round(tDone),
        litPixels: count, fieldScale: +f.toFixed(2), frames: () => frames, lastShadeMs: () => lastShade };
      const inner = shade; shade = (a: number) => { const s0 = performance.now(); inner(a); lastShade = +(performance.now() - s0).toFixed(2); };
      wake();
    }

    const step = () => {
      frame = 0;
      if (!shade || !seen) return;
      az += (target - az) * 0.085;
      shade(az);
      if (Math.abs(target - az) > 0.08) frame = requestAnimationFrame(step); else { az = target; shade(az); }
    };
    const wake = () => { if (!frame && seen && shade && !still) frame = requestAnimationFrame(step); };
    const onMove = (e: PointerEvent) => {
      if (e.pointerType === "touch" || still) return;
      const r = el.getBoundingClientRect();
      const nxp = Math.max(-1, Math.min(1, ((e.clientX - r.left) / r.width - 0.5) * 2));
      target = REST_AZ - nxp * SWING;                                          // the light leans toward the pointer's side
      wake();
    };
    const io = new IntersectionObserver(([x]) => { seen = x.isIntersecting; if (seen) wake(); });
    io.observe(el);
    let t = 0;
    // the first build runs at once; later ones (a resize) wait for the dragging to stop
    const ro = new ResizeObserver(() => { clearTimeout(t); if (!built) { built = true; void build(); } else t = window.setTimeout(() => void build(), 140); });
    ro.observe(el);
    const mo = new MutationObserver(() => void build());                        // the theme changed: the stone is a different stone
    mo.observe(document.documentElement, { attributes: true, attributeFilter: ["data-theme"] });
    const scheme = matchMedia("(prefers-color-scheme: dark)"); const onScheme = () => void build(); scheme.addEventListener("change", onScheme);
    addEventListener("pointermove", onMove, { passive: true });
    return () => { dead = true; cancelAnimationFrame(frame); clearTimeout(t); io.disconnect(); ro.disconnect(); mo.disconnect(); scheme.removeEventListener("change", onScheme); removeEventListener("pointermove", onMove); };
  }, []);
  return (
    <div className="slab" ref={host}>
      <div className="wrap-page slab__rule"><span className="slab__word" aria-hidden="true">{WORD}<i className="slab__base" /></span></div>
      <canvas ref={canvas} role="img" aria-label="The word Rook, cut into stone" />
    </div>
  );
}
