// Writes public/brand/bird-<id>.svg and favicon-<id>.svg from src/rook/marks.ts, so the files and the components share one geometry.
// Run: node --import tsx scripts/write-marks.mjs (tsx is a devDependency).
import { writeFileSync, mkdirSync } from "node:fs";
const { BIRDS, svgOf } = await import("../src/rook/marks.ts");
mkdirSync(new URL("../public/brand/", import.meta.url), { recursive: true });
for (const b of BIRDS) {
  writeFileSync(new URL(`../public/brand/bird-${b.id}.svg`, import.meta.url), svgOf(b, { ink: "#ECEFF3", accent: "#2FD3C6", ground: "#0B0D10" }));
  writeFileSync(new URL(`../public/brand/favicon-${b.id}.svg`, import.meta.url), svgOf(b, { ink: "#ECEFF3", accent: "#2FD3C6", ground: "#0B0D10", rounded: true }));
}
console.log(`wrote ${BIRDS.length} marks`);
