// The wrapped tokens' metadata (docs/v9/ceremony-canister-only.md §2): one Metaplex fungible-token JSON per collection of the
// canister's registry, public/wx/<slug>.json, and its picture, public/wx/img/<slug>.png. The launch pins
// (autobuyer/canister-rs/examples/launch_pins.rs) write each wX's on-chain uri as `{uri_base}{slug}.json`, uri_base
// https://userook.fun/wx/ by default, and the site serves public/ at its root, so these files are what every wX points to.
//
// The name and symbol are exactly what the pin puts on chain: autobuyer/canister-rs/src/coins/unlock.rs `ORDER` (slug, symbol,
// name), and for a Robinhood Chain collection deployments/launch/robinhood-collections.json's own (launch_pins takes those). The
// collection's own name (for the description) and its page come from src/rook/sol/collections.ts.
//
// The picture is Rook's, never the collection's art: the worn rook mark (src/rook/marks.ts, as the brand avatar draws it) over the
// wX symbol, on the dark ground, 512 × 512, drawn with scripts/og-image.mjs's rasteriser (no native dependency). The symbol is set
// in IBM Plex Mono Bold, its outlines read from the @fontsource/ibm-plex-mono WOFF the site already ships, the "w" in the accent.
//
//   cd web-rook && node --import tsx scripts/wx-metadata.mjs         (writes all 41; test/wx.test.ts checks them)
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { inflateSync } from "node:zlib";
import { pathToFileURL } from "node:url";
import { ACCENT, GROUND, INK, flatten, paint, paintMark, pngOf } from "./og-image.mjs";
import { COLLECTIONS } from "../src/rook/sol/collections.ts";
import { SITE_URL } from "../site.config.ts";

export const SIZE = 512;
export const WX_BASE = `${SITE_URL}/wx/`;
export const IMG_BASE = `${WX_BASE}img/`;
/** The canister's limits (coins/open.rs `register`): name 1–32 bytes, symbol 1–10 bytes, uri at most 200 bytes. */
export const NAME_BYTES = 32, SYMBOL_BYTES = 10, URI_BYTES = 200;
const here = (p) => new URL(p, import.meta.url);

// ---- the registry, as the launch pins read it -----------------------------------------------------------------------------------
/** unlock.rs `ORDER`: [{ slug, symbol, name, source }] in registry order. */
export function parseOrder(rust) {
  const start = rust.indexOf("pub const ORDER");
  if (start < 0) throw new Error("unlock.rs: no ORDER");
  const body = rust.slice(start, rust.indexOf("];", start));
  return [...body.matchAll(/e\("([^"]+)", "([^"]+)", "([^"]+)", (\w+)\)/g)].map((m) => ({ slug: m[1], symbol: m[2], name: m[3], source: m[4] }));
}
/** Every collection launch_pins can pin (the 38 of the default run and the 3 of --include-robinhood): slug, the on-chain name and
 *  symbol, the collection's own name and its page on the site. Read from the repository (unlock.rs, robinhood-collections.json). */
export function registry(root = here("../../")) {
  const order = parseOrder(readFileSync(new URL("autobuyer/canister-rs/src/coins/unlock.rs", root), "utf8"));
  const rh = JSON.parse(readFileSync(new URL("deployments/launch/robinhood-collections.json", root), "utf8")).collections;
  return order.map((e) => {
    // launch_pins: a Robinhood Chain collection's name and symbol are its launch file's
    const r = e.source === "Robinhood" ? rh.find((c) => c.slug === e.slug) : null;
    if (e.source === "Robinhood" && !r) throw new Error(`${e.slug}: not in robinhood-collections.json`);
    const site = COLLECTIONS.find((c) => c.slug === e.slug);
    if (!site) throw new Error(`${e.slug}: not in src/rook/sol/collections.ts`);
    return { slug: e.slug, name: r ? r.name : e.name, symbol: r ? r.symbol : e.symbol, collection: site.name, page: `${SITE_URL}/#/collection/${e.slug}` };
  });
}

// ---- the metadata -----------------------------------------------------------------------------------------------------------------
/** The Metaplex fungible-token JSON of one wX. The description says what docs 02-how-it-works.md ("What backs a wrapped token") says. */
export function metadataOf(e) {
  const image = `${IMG_BASE}${e.slug}.png`;
  return {
    name: e.name,
    symbol: e.symbol,
    description: `Rook's wrapped ${e.collection} token: each whole token is backed either by a ${e.collection} NFT held by Rook's custodian or by SOL the custodian holds for the collection, counted at the floor price.`,
    image,
    external_url: e.page ?? SITE_URL,
    properties: { files: [{ uri: image, type: "image/png" }], category: "image" },
  };
}
export const metadataText = (e) => `${JSON.stringify(metadataOf(e), null, 2)}\n`;

// ---- IBM Plex Mono Bold's outlines, from the WOFF ---------------------------------------------------------------------------------
const FONT = here("../node_modules/@fontsource/ibm-plex-mono/files/ibm-plex-mono-latin-700-normal.woff");
/** A WOFF 1.0 file's TrueType tables (zlib per table). */
function woffTables(b) {
  if (b.toString("latin1", 0, 4) !== "wOFF") throw new Error("not a WOFF file");
  const n = b.readUInt16BE(12), t = {};
  for (let i = 0; i < n; i++) {
    const o = 44 + i * 20, tag = b.toString("latin1", o, o + 4), off = b.readUInt32BE(o + 4), comp = b.readUInt32BE(o + 8), orig = b.readUInt32BE(o + 12);
    const raw = b.subarray(off, off + comp);
    t[tag] = comp < orig ? inflateSync(raw) : raw;
  }
  return t;
}
let font = null;
/** The font: units per em, the (monospaced) advance, and a glyph's outline as path data in font units (y up). */
export function plexMono() {
  if (font) return font;
  const t = woffTables(readFileSync(FONT));
  const unitsPerEm = t.head.readUInt16BE(18), longLoca = t.head.readInt16BE(50) === 1;
  const loca = (g) => (longLoca ? t.loca.readUInt32BE(g * 4) : t.loca.readUInt16BE(g * 2) * 2);
  // cmap: the Windows Unicode BMP subtable, format 4
  const cm = t.cmap, nSub = cm.readUInt16BE(2);
  let sub = -1;
  for (let i = 0; i < nSub; i++) { const p = cm.readUInt16BE(4 + i * 8), e = cm.readUInt16BE(6 + i * 8); if (p === 3 && e === 1) sub = cm.readUInt32BE(8 + i * 8); }
  if (sub < 0 || cm.readUInt16BE(sub) !== 4) throw new Error("cmap: no format-4 (3,1) subtable");
  const seg = cm.readUInt16BE(sub + 6) / 2, ends = sub + 14, starts = ends + seg * 2 + 2, deltas = starts + seg * 2, ranges = deltas + seg * 2;
  const glyphOf = (ch) => {
    const c = ch.codePointAt(0);
    for (let i = 0; i < seg; i++) {
      if (cm.readUInt16BE(ends + i * 2) < c) continue;
      const s = cm.readUInt16BE(starts + i * 2);
      if (s > c) break;
      const delta = cm.readUInt16BE(deltas + i * 2), ro = cm.readUInt16BE(ranges + i * 2);
      if (!ro) return (c + delta) & 0xffff;
      const g = cm.readUInt16BE(ranges + i * 2 + ro + 2 * (c - s));
      return g ? (g + delta) & 0xffff : 0;
    }
    return 0;
  };
  /** A glyph's contours: [[{ x, y, on }]], composites resolved (offsets and a uniform scale). */
  const contours = (g) => {
    const start = loca(g), end = loca(g + 1);
    if (end <= start) return [];
    const d = t.glyf.subarray(start, end), nc = d.readInt16BE(0);
    if (nc < 0) {
      const out = [];
      let p = 10, flags;
      do {
        flags = d.readUInt16BE(p); const gi = d.readUInt16BE(p + 2); p += 4;
        let dx, dy;
        if (flags & 1) { dx = d.readInt16BE(p); dy = d.readInt16BE(p + 2); p += 4; } else { dx = d.readInt8(p); dy = d.readInt8(p + 1); p += 2; }
        if (!(flags & 2)) throw new Error("composite glyph with point-matched components");
        let sx = 1, sy = 1;
        if (flags & 8) { sx = sy = d.readInt16BE(p) / 16384; p += 2; } else if (flags & 0x40) { sx = d.readInt16BE(p) / 16384; sy = d.readInt16BE(p + 2) / 16384; p += 4; } else if (flags & 0x80) throw new Error("composite glyph with a 2 × 2 transform");
        for (const c of contours(gi)) out.push(c.map((q) => ({ x: q.x * sx + dx, y: q.y * sy + dy, on: q.on })));
      } while (flags & 0x20);
      return out;
    }
    const endPts = Array.from({ length: nc }, (_, i) => d.readUInt16BE(10 + i * 2)), n = endPts[nc - 1] + 1;
    let p = 10 + nc * 2; p += 2 + d.readUInt16BE(p);
    const fl = [];
    while (fl.length < n) { const f = d[p++]; fl.push(f); if (f & 8) { let r = d[p++]; while (r--) fl.push(f); } }
    const coords = (short, same) => { let v = 0; return fl.map((f) => { if (f & short) { const b = d[p++]; v += f & same ? b : -b; } else if (!(f & same)) { v += d.readInt16BE(p); p += 2; } return v; }); };
    const xs = coords(2, 16), ys = coords(4, 32);
    const pts = fl.map((f, i) => ({ x: xs[i], y: ys[i], on: !!(f & 1) }));
    return endPts.map((e, i) => pts.slice(i ? endPts[i - 1] + 1 : 0, e + 1));
  };
  /** Quadratic contours as M/L/Q/Z path data: two off-curve points in a row imply the on-curve point between them. */
  const pathOf = (ch) => contours(glyphOf(ch)).map((c) => {
    const mid = (a, b) => ({ x: (a.x + b.x) / 2, y: (a.y + b.y) / 2, on: true });
    const i0 = c.findIndex((q) => q.on), first = i0 >= 0 ? c[i0] : mid(c[0], c[1]);
    const ring = i0 >= 0 ? [...c.slice(i0 + 1), ...c.slice(0, i0)] : [...c.slice(1), c[0]];
    let s = `M${first.x} ${first.y}`, ctrl = null;
    for (const q of [...ring, first]) {
      if (q.on) { s += ctrl ? `Q${ctrl.x} ${ctrl.y} ${q.x} ${q.y}` : `L${q.x} ${q.y}`; ctrl = null; }
      else { if (ctrl) { const m = mid(ctrl, q); s += `Q${ctrl.x} ${ctrl.y} ${m.x} ${m.y}`; } ctrl = q; }
    }
    return `${s}Z`;
  }).join("");
  const advance = t.hmtx.readUInt16BE(0);
  font = { unitsPerEm, advance, glyphOf, pathOf };
  return font;
}

// ---- the picture ------------------------------------------------------------------------------------------------------------------
/** The mark (232 px, as the avatar sets it) above the symbol (60 px, centred, "w" in the accent); everything inside the inscribed
 *  circle, so a wallet that crops the picture round loses nothing. RGB, row-major. */
export const MARK_PX = 232, MARK_TOP = 78, TEXT_PX = 60, BASELINE = 400;
export function wxPixels(symbol) {
  const rgb = Buffer.alloc(SIZE * SIZE * 3);
  for (let i = 0; i < SIZE * SIZE; i++) rgb.set(GROUND, i * 3);
  paintMark(rgb, (SIZE - MARK_PX) / 2, MARK_TOP, MARK_PX, SIZE, SIZE);
  const f = plexMono(), k = TEXT_PX / f.unitsPerEm, adv = f.advance * k, left = (SIZE - adv * symbol.length) / 2;
  [...symbol].forEach((ch, i) => {
    const d = f.pathOf(ch);
    if (!d) { if (ch.trim()) throw new Error(`no glyph for ${JSON.stringify(ch)}`); return; }
    paint(rgb, flatten(d, (x, y) => [left + i * adv + x * k, BASELINE - y * k]), i === 0 && ch === "w" ? ACCENT : INK, SIZE, SIZE);
  });
  return rgb;
}
export const wxPng = (symbol) => pngOf(wxPixels(symbol), SIZE, SIZE);

// ---- write ------------------------------------------------------------------------------------------------------------------------
if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const out = here("../public/wx/");
  mkdirSync(new URL("img/", out), { recursive: true });
  const all = registry();
  for (const e of all) {
    if (Buffer.byteLength(e.name) > NAME_BYTES || Buffer.byteLength(e.symbol) > SYMBOL_BYTES) throw new Error(`${e.slug}: over the canister's limits`);
    writeFileSync(new URL(`${e.slug}.json`, out), metadataText(e));
    writeFileSync(new URL(`img/${e.slug}.png`, out), wxPng(e.symbol));
  }
  console.log(`wrote ${all.length} metadata files and pictures to public/wx/`);
}
