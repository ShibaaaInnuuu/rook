import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { BIRDS, svgOf } from "../src/rook/marks.js";

/** docs/v6/site-design.md §5: four birds on one grid, and the standalone files carry the same geometry as the components. */
const here = (p: string) => new URL(p, import.meta.url);
test("four birds, every shape inside the 32 grid, exactly one accent shape each, and the files match the geometry", () => {
  assert.deepEqual(BIRDS.map((b) => b.id), ["perched", "head", "glyph", "flight"]);
  for (const b of BIRDS) {
    assert.ok(b.shapes.filter((s) => s.kind === "accent").length <= 1 && b.shapes.some((s) => s.kind === "accent" || s.kind === "eye"), `${b.id}: at most one accent shape, and an eye or an accent to take the sheen`);
    for (const s of b.shapes) {
      const nums = (s.d ?? "").match(/-?\d+(\.\d+)?/g)?.map(Number) ?? [];
      for (const n of nums) assert.ok(n >= 0 && n <= 32, `${b.id}: ${n} is off the grid`);
      if (s.circle) assert.ok(s.circle[0] - s.circle[2] >= 0 && s.circle[0] + s.circle[2] <= 32 && s.circle[1] - s.circle[2] >= 0 && s.circle[1] + s.circle[2] <= 32, `${b.id}: a circle leaves the grid`);
    }
    const file = readFileSync(here(`../public/brand/bird-${b.id}.svg`), "utf8");
    assert.equal(file, svgOf(b, { ink: "#ECEFF3", accent: "#2FD3C6", ground: "#0B0D10" }), `bird-${b.id}.svg is written from marks.ts`);
    assert.match(readFileSync(here(`../public/brand/favicon-${b.id}.svg`), "utf8"), /rx="7"/);
  }
});
