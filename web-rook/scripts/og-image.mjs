// The social card (og:image), 1200 × 630: the worn rook mark and the wordmark on the dark ground, so a shared userook.fun link
// carries a preview. Emitted as og.png by vite.config.ts at build and served at /og.png in dev. Nothing is rendered by a
// browser or a font engine to make it, so the build needs no native dependency:
//   - the mark is src/rook/marks.ts, the same numbers as the components and the favicons, worn as src/rook/brand.ts says;
//   - the wordmark is Bricolage Grotesque's own outlines for "Rook" at the masthead's instance — weight 700, width 100 (the
//     font's width axis stops at 100, which is what a browser draws for the stylesheet's "wdth" 118 too), optical size 96 —
//     taken once from the OFL font with fontkit and kept below as path data in font units (1000 to the em), pen positions
//     and kerning included;
//   - a scanline rasteriser fills the paths under the nonzero rule, four sub-scanlines a pixel with exact horizontal
//     coverage, and node's zlib writes the PNG (filter 0 on every row, so a test can read the pixels back).
// `node --import tsx scripts/og-image.mjs out.png` writes the card to a file for a look.
import { deflateSync } from "node:zlib";
import { writeFileSync } from "node:fs";
import { pathToFileURL } from "node:url";
import { BIRDS } from "../src/rook/marks.ts";
import { BRAND_MARK } from "../src/rook/brand.ts";

export const WIDTH = 1200, HEIGHT = 630;
/** --ground, --ink and --accent of the dark theme (docs/v6/site-design.md §2) */
export const GROUND = [0x0b, 0x0d, 0x10], INK = [0xec, 0xef, 0xf3], ACCENT = [0x2f, 0xd3, 0xc6];

/** "Rook" in Bricolage Grotesque, wght 700 / wdth 100 / opsz 96: each glyph's outline and its pen position, font units. */
export const WORDMARK = {
  unitsPerEm: 1000, capHeight: 660, ascent: 930, descent: -270, advance: 2243,
  glyphs: [
    { ch: "R", x: 0, d: "M43 0L43 660L307 660Q351 660 390.5 652.5Q430 645 462 629.5Q494 614 517.5 591Q541 568 553.5 536.5Q566 505 566 465Q566 435 556 407Q546 379 524 356.5Q502 334 466.5 319.5Q431 305 380 299L380 291Q442 288 476.5 266.5Q511 245 529 211.5Q547 178 557 136L592 0L433 0L408 126Q401 161 387.5 184.5Q374 208 349 220Q324 232 282 232L188 232L188 0ZM188 345L290 345Q352 345 384.5 368.5Q417 392 417 440Q417 492 386.5 517.5Q356 543 294 543L188 543Z" },
    { ch: "o", x: 585.3333333333334, d: "M282 -14Q207 -14 148.5 16.5Q90 47 57 108.5Q24 170 24 262Q24 357 58.5 418.5Q93 480 151.5 509.5Q210 539 283 539Q359 539 417 508.5Q475 478 507.5 416.5Q540 355 540 262Q540 167 506 105.5Q472 44 413.5 15Q355 -14 282 -14ZM285 94Q320 94 344 113Q368 132 381 168Q394 204 394 254Q394 309 380 346.5Q366 384 340.5 404.5Q315 425 279 425Q245 425 221 406Q197 387 184 350.5Q171 314 171 264Q171 184 201 139Q231 94 285 94Z" },
    { ch: "o", x: 1149.8333333333335, d: "M282 -14Q207 -14 148.5 16.5Q90 47 57 108.5Q24 170 24 262Q24 357 58.5 418.5Q93 480 151.5 509.5Q210 539 283 539Q359 539 417 508.5Q475 478 507.5 416.5Q540 355 540 262Q540 167 506 105.5Q472 44 413.5 15Q355 -14 282 -14ZM285 94Q320 94 344 113Q368 132 381 168Q394 204 394 254Q394 309 380 346.5Q366 384 340.5 404.5Q315 425 279 425Q245 425 221 406Q197 387 184 350.5Q171 314 171 264Q171 184 201 139Q231 94 285 94Z" },
    { ch: "k", x: 1714.3333333333335, d: "M39 0L39 695L180 695L180 323Q216 340 244.5 363.5Q273 387 293.5 414.5Q314 442 326.5 470.5Q339 499 346 525L511 525Q500 487 479 450Q458 413 427 380Q396 347 356 321Q316 295 267 278L267 270Q330 279 370 271Q410 263 433.5 242Q457 221 470 190Q483 159 490 122L514 0L358 0L346 86Q341 121 331.5 150Q322 179 298 197Q274 215 226 215L180 215L180 0Z" },
  ],
};

// ---- the lockup, as the masthead sets it (src/styles/rook.css .wordmark): mark, gap, then the word, letter-spacing −0.03em ----
const F = 240;                                   // the font size the card is set at, px
const MARK = (30 / 21.6) * F;                    // the mark is 30px beside a 1.35rem word
const GAP = (0.55 / 1.35) * F;
const TRACK = -0.03 * F;
const WORD_W = (WORDMARK.advance / WORDMARK.unitsPerEm) * F + TRACK * WORDMARK.glyphs.length;
const LEFT = (WIDTH - (MARK + GAP + WORD_W)) / 2;
const MID = HEIGHT / 2;
/** the baseline of a line box of 1.5 line-height whose centre is MID: half-leading, then the ascent */
const BASELINE = MID - 0.75 * F + 0.15 * F + (WORDMARK.ascent / WORDMARK.unitsPerEm) * F;

// ---- paths → polygons ------------------------------------------------------------------------------------------------------
const SEG = 16;
/** Absolute or relative M L H V Q C Z, as fontkit and marks.ts write them, flattened into closed polygons under `tf`. */
export function flatten(d, tf, minPoints = 3) {
  const tok = d.match(/[A-Za-z]|-?\d*\.?\d+(?:e[-+]?\d+)?/g) ?? [];
  const polys = []; let poly = null, cx = 0, cy = 0, sx = 0, sy = 0, cmd = "";
  const put = (x, y) => { if (!poly) { poly = []; polys.push(poly); } poly.push(tf(x, y)); cx = x; cy = y; };
  const quad = (x1, y1, x, y) => { const [ax, ay] = [cx, cy]; for (let i = 1; i <= SEG; i++) { const t = i / SEG, u = 1 - t; put(u * u * ax + 2 * u * t * x1 + t * t * x, u * u * ay + 2 * u * t * y1 + t * t * y); } };
  const cubic = (x1, y1, x2, y2, x, y) => { const [ax, ay] = [cx, cy]; for (let i = 1; i <= SEG; i++) { const t = i / SEG, u = 1 - t; put(u * u * u * ax + 3 * u * u * t * x1 + 3 * u * t * t * x2 + t * t * t * x, u * u * u * ay + 3 * u * u * t * y1 + 3 * u * t * t * y2 + t * t * t * y); } };
  let i = 0;
  const n = () => Number(tok[i++]);
  while (i < tok.length) {
    const t = tok[i];
    if (/[A-Za-z]/.test(t)) { cmd = t; i++; if (cmd === "Z" || cmd === "z") { poly = null; cx = sx; cy = sy; continue; } }
    const rel = cmd === cmd.toLowerCase(), ox = rel ? cx : 0, oy = rel ? cy : 0;
    switch (cmd.toUpperCase()) {
      case "M": { const x = ox + n(), y = oy + n(); poly = null; put(x, y); sx = x; sy = y; cmd = rel ? "l" : "L"; break; }
      case "L": put(ox + n(), oy + n()); break;
      case "H": put(ox + n(), cy); break;
      case "V": put(cx, oy + n()); break;
      case "Q": { const x1 = ox + n(), y1 = oy + n(), x = ox + n(), y = oy + n(); quad(x1, y1, x, y); break; }
      case "C": { const x1 = ox + n(), y1 = oy + n(), x2 = ox + n(), y2 = oy + n(), x = ox + n(), y = oy + n(); cubic(x1, y1, x2, y2, x, y); break; }
      default: throw new Error(`og-image: unsupported path command ${cmd}`);
    }
  }
  return polys.filter((p) => p.length >= minPoints);
}
export const circle = (cx, cy, r, tf, steps = 64) => [Array.from({ length: steps }, (_, i) => { const a = (i / steps) * 2 * Math.PI; return tf(cx + r * Math.cos(a), cy + r * Math.sin(a)); })];
const area = (p) => p.reduce((a, [x0, y0], i) => { const [x1, y1] = p[(i + 1) % p.length]; return a + x0 * y1 - x1 * y0; }, 0) / 2;
const ccw = (p) => (area(p) < 0 ? [...p].reverse() : p);
/** A stroked path as polygons: a quad per segment and a disc per vertex (round caps and joins), all one orientation so the nonzero rule unions them. */
export function stroke(d, width, tf) {
  const out = [], w = width / 2;
  for (const line of flattenOpen(d)) {
    for (let i = 0; i < line.length; i++) {
      const [x, y] = line[i];
      out.push(ccw(circle(x, y, w, tf, 32)[0]));
      if (i + 1 < line.length) {
        const [x1, y1] = line[i + 1], dx = x1 - x, dy = y1 - y, len = Math.hypot(dx, dy) || 1, nx = (-dy / len) * w, ny = (dx / len) * w;
        out.push(ccw([tf(x + nx, y + ny), tf(x1 + nx, y1 + ny), tf(x1 - nx, y1 - ny), tf(x - nx, y - ny)]));
      }
    }
  }
  return out;
}
/** The same parser, but open polylines in user units (a stroke is not closed by Z). */
const flattenOpen = (d) => flatten(d, (x, y) => [x, y], 2);

// ---- the rasteriser ----------------------------------------------------------------------------------------------------------
const SUB = 4;
function coverage(polys, W, H) {
  const cov = new Float32Array(W * H), edges = [];
  let minY = Infinity, maxY = -Infinity;
  for (const poly of polys) for (let i = 0; i < poly.length; i++) {
    const [x0, y0] = poly[i], [x1, y1] = poly[(i + 1) % poly.length];
    if (y0 === y1) continue;
    edges.push(y0 < y1 ? [x0, y0, x1, y1, 1] : [x1, y1, x0, y0, -1]);
    minY = Math.min(minY, y0, y1); maxY = Math.max(maxY, y0, y1);
  }
  const xs = [];
  for (let py = Math.max(0, Math.floor(minY)); py < Math.min(H, Math.ceil(maxY)); py++) {
    for (let s = 0; s < SUB; s++) {
      const y = py + (s + 0.5) / SUB;
      xs.length = 0;
      for (const e of edges) if (y >= e[1] && y < e[3]) xs.push([e[0] + ((y - e[1]) * (e[2] - e[0])) / (e[3] - e[1]), e[4]]);
      xs.sort((a, b) => a[0] - b[0]);
      let w = 0;
      for (let i = 0; i + 1 < xs.length; i++) { w += xs[i][1]; if (w !== 0) span(cov, W, py, xs[i][0], xs[i + 1][0]); }
    }
  }
  return cov;
}
function span(cov, W, py, xa, xb) {
  xa = Math.max(0, xa); xb = Math.min(W, xb);
  if (xb <= xa) return;
  const row = py * W, weight = 1 / SUB, last = Math.ceil(xb) - 1;
  let px = Math.floor(xa);
  if (px === last) { cov[row + px] += (xb - xa) * weight; return; }
  cov[row + px] += (px + 1 - xa) * weight;
  for (px++; px < last; px++) cov[row + px] += weight;
  cov[row + last] += (xb - last) * weight;
}
/** Paints `polys` (nonzero rule) in `color` over an RGB image of `W` × `H` pixels (the card's size by default). */
export function paint(rgb, polys, color, W = WIDTH, H = HEIGHT) {
  const cov = coverage(polys, W, H);
  for (let i = 0; i < cov.length; i++) {
    const c = Math.min(1, cov[i]);
    if (c <= 0) continue;
    const o = i * 3;
    rgb[o] = Math.round(rgb[o] + (color[0] - rgb[o]) * c); rgb[o + 1] = Math.round(rgb[o + 1] + (color[1] - rgb[o + 1]) * c); rgb[o + 2] = Math.round(rgb[o + 2] + (color[2] - rgb[o + 2]) * c);
  }
}

/** The worn mark (src/rook/brand.ts) in its 32-unit grid, scaled to `size` px with its top-left corner at (`left`, `top`), on an RGB image of `W` × `H`. */
export function paintMark(rgb, left, top, size, W = WIDTH, H = HEIGHT) {
  const spec = BIRDS.find((b) => b.id === BRAND_MARK) ?? BIRDS[1];
  const k = size / 32;
  const tf = (x, y) => [left + x * k, top + y * k];
  for (const s of spec.shapes) {
    const color = s.kind === "accent" ? ACCENT : s.kind === "eye" ? GROUND : INK;
    if (s.circle) paint(rgb, circle(s.circle[0], s.circle[1], s.circle[2], tf), color, W, H);
    else if (s.kind === "stroke") paint(rgb, stroke(s.d, spec.strokeWidth, tf), color, W, H);
    else paint(rgb, flatten(s.d, tf), color, W, H);
  }
}

// ---- the card -----------------------------------------------------------------------------------------------------------------
/** The card's pixels, RGB, row-major. */
export function ogPixels() {
  const rgb = Buffer.alloc(WIDTH * HEIGHT * 3);
  for (let i = 0; i < WIDTH * HEIGHT; i++) rgb.set(GROUND, i * 3);
  // the mark, in its 32-unit grid scaled to MARK px and centred on the card's middle line
  paintMark(rgb, LEFT, MID - MARK / 2, MARK);
  // the word: each glyph at its pen position plus the tracking, the font's y up
  const g = F / WORDMARK.unitsPerEm, x0 = LEFT + MARK + GAP;
  WORDMARK.glyphs.forEach((glyph, i) => paint(rgb, flatten(glyph.d, (x, y) => [x0 + (glyph.x + x) * g + TRACK * i, BASELINE - y * g]), INK));
  return rgb;
}

// ---- PNG ----------------------------------------------------------------------------------------------------------------------
const CRC = new Uint32Array(256).map((_, n) => { let c = n; for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1; return c >>> 0; });
const crc32 = (buf) => { let c = 0xffffffff; for (const b of buf) c = CRC[(c ^ b) & 0xff] ^ (c >>> 8); return (c ^ 0xffffffff) >>> 0; };
function chunk(type, data) {
  const len = Buffer.alloc(4); len.writeUInt32BE(data.length);
  const body = Buffer.concat([Buffer.from(type, "latin1"), data]);
  const crc = Buffer.alloc(4); crc.writeUInt32BE(crc32(body));
  return Buffer.concat([len, body, crc]);
}
export function pngOf(rgb, w, h) {
  const stride = w * 3, raw = Buffer.alloc((stride + 1) * h);
  for (let y = 0; y < h; y++) { raw[y * (stride + 1)] = 0; rgb.copy(raw, y * (stride + 1) + 1, y * stride, (y + 1) * stride); }
  const ihdr = Buffer.alloc(13); ihdr.writeUInt32BE(w, 0); ihdr.writeUInt32BE(h, 4); ihdr[8] = 8; ihdr[9] = 2;
  return Buffer.concat([Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]), chunk("IHDR", ihdr), chunk("IDAT", deflateSync(raw, { level: 9 })), chunk("IEND", Buffer.alloc(0))]);
}
let cached = null;
/** The card as a PNG file's bytes, made once per process. */
export const ogPng = () => (cached ??= pngOf(ogPixels(), WIDTH, HEIGHT));

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const out = process.argv[2] ?? "og.png";
  writeFileSync(out, ogPng());
  console.log(`wrote ${out} (${WIDTH}×${HEIGHT}, ${ogPng().length} bytes)`);
}
