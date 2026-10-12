import assert from "node:assert/strict";
import { test } from "node:test";
import { inflateSync } from "node:zlib";
import { ACCENT, GROUND, HEIGHT, INK, WIDTH, flatten, ogPixels, ogPng } from "../scripts/og-image.mjs";
import { BIRDS } from "../src/rook/marks.js";
import { BRAND_MARK } from "../src/rook/brand.js";

/** The social card: a 1200 × 630 PNG the build writes, the worn mark and the wordmark in ink on the dark ground, made the same way every time. */
const at = (rgb: Buffer, x: number, y: number) => [...rgb.subarray((y * WIDTH + x) * 3, (y * WIDTH + x) * 3 + 3)];

test("og.png is a well-formed 1200 × 630 RGB PNG whose rows decode back to the card's pixels", () => {
  const png = ogPng();
  assert.deepEqual([...png.subarray(0, 8)], [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]);
  assert.equal(png.subarray(12, 16).toString("latin1"), "IHDR");
  assert.equal(png.readUInt32BE(16), 1200); assert.equal(png.readUInt32BE(20), 630); assert.equal(png[24], 8, "8 bits"); assert.equal(png[25], 2, "RGB");
  const idat = png.indexOf("IDAT", 33, "latin1"), len = png.readUInt32BE(idat - 4);
  const raw = inflateSync(png.subarray(idat + 4, idat + 4 + len)), stride = WIDTH * 3 + 1;
  assert.equal(raw.length, stride * HEIGHT);
  const rgb = ogPixels();
  for (let y = 0; y < HEIGHT; y += 97) { assert.equal(raw[y * stride], 0, "filter 0"); assert.ok(raw.subarray(y * stride + 1, (y + 1) * stride).equals(rgb.subarray(y * WIDTH * 3, (y + 1) * WIDTH * 3)), `row ${y}`); }
  assert.ok(png.subarray(-12).subarray(4, 8).toString("latin1") === "IEND");
  assert.ok(ogPng().equals(png), "made once, the same bytes");
});

test("the ground is the dark theme's, the mark and the word are ink, the mark's eye is a hole, and nothing touches the edges", () => {
  const rgb = ogPixels();
  assert.deepEqual(at(rgb, 0, 0), GROUND); assert.deepEqual(at(rgb, WIDTH - 1, HEIGHT - 1), GROUND); assert.deepEqual(at(rgb, 600, 20), GROUND);
  let ink = 0, accent = 0;
  for (let i = 0; i < WIDTH * HEIGHT; i++) { const o = i * 3; if (rgb[o] === INK[0] && rgb[o + 1] === INK[1] && rgb[o + 2] === INK[2]) ink++; if (rgb[o] === ACCENT[0] && rgb[o + 1] === ACCENT[1] && rgb[o + 2] === ACCENT[2]) accent++; }
  assert.ok(ink > WIDTH * HEIGHT * 0.05 && ink < WIDTH * HEIGHT * 0.5, `the lockup covers a sensible share of the card, not ${ink} pixels`);
  const spec = BIRDS.find((b) => b.id === BRAND_MARK)!;
  assert.equal(accent > 0, spec.shapes.some((s) => s.kind === "accent"), "the accent shows only if the worn mark has an accent shape");
  // the left third holds the mark, the right two thirds the word; a column between them is bare ground
  const col = (x: number) => { let n = 0; for (let y = 0; y < HEIGHT; y++) if (at(rgb, x, y).join() === INK.join()) n++; return n; };
  assert.ok(col(300) > 50, "the mark"); assert.ok(col(800) > 50, "the word"); assert.equal(col(1180), 0); assert.equal(col(20), 0);
  for (let x = 0; x < WIDTH; x++) { assert.deepEqual(at(rgb, x, 0), GROUND); assert.deepEqual(at(rgb, x, HEIGHT - 1), GROUND); }
});

test("the path parser: absolute and relative lines and curves, a Z back to the subpath's start, and an unknown command refuses", () => {
  const sq = flatten("M0 0 H10 V10 H0 Z", (x, y) => [x, y]);
  assert.deepEqual(sq, [[[0, 0], [10, 0], [10, 10], [0, 10]]]);
  const rel = flatten("m5 5 l5 0 l0 5 z m20 20 l1 0 l0 1 z", (x, y) => [x, y]);
  assert.equal(rel.length, 2); assert.deepEqual(rel[1][0], [25, 25]);
  const q = flatten("M0 0 Q5 10 10 0 Z", (x, y) => [x, y])[0];
  assert.equal(q.length, 17); assert.ok(Math.abs(q[8][0] - 5) < 1e-9 && Math.abs(q[8][1] - 5) < 1e-9, "the quadratic's midpoint");
  assert.throws(() => flatten("M0 0 A1 1 0 0 0 1 1", (x, y) => [x, y]), /unsupported/);
});
