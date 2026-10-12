/**
 * The rook marks' geometry, on one 32-unit grid, as plain path data (docs/v6/site-design.md §5). Kept apart from the
 * components so the standalone files under public/brand/ are written from the same numbers (test/marks.test.ts checks).
 * Fills are `currentColor`; strokes are 2.4 units, round joins; the one accent shape is named `accent`.
 */
export type Shape = { d?: string; circle?: [number, number, number]; kind: "fill" | "stroke" | "accent" | "eye" };
export type MarkSpec = { id: BirdId; name: string; why: string; shapes: Shape[]; strokeWidth: number };
export type BirdId = "perched" | "head" | "glyph" | "flight";

export const BIRDS: MarkSpec[] = [
  {
    id: "perched", name: "The rook, perched", strokeWidth: 2,
    why: "The whole bird in profile on its perch: the heavy body, the long tail, and the pale bare face patch that tells a rook from a crow — the one accent-coloured shape.",
    shapes: [
      { circle: [11.5, 10.5, 5], kind: "fill" },
      { d: "M6.8 8.6 L1.2 11.4 L6.8 12.6 Z", kind: "fill" },
      { d: "M9.6 14.6 C8.2 19.6 10.6 25.2 16.2 26 L24.2 24.6 L31 28 L31.2 25.2 L24.6 20.8 C23.4 16.6 19.8 13.2 15.2 13.4 Z", kind: "fill" },
      { d: "M15.5 26 L14.6 30 M20 25.4 L21 30 M8 30.6 H27", kind: "stroke" },
      { d: "M6.6 8.4 L10.4 9.4 L10.4 12.2 L6.6 12.8 Z", kind: "accent" },
      { circle: [12.8, 9.4, 1.2], kind: "eye" },
    ],
  },
  {
    id: "head", name: "The head", strokeWidth: 2.4,
    why: "Head and beak, bold and geometric: a circle, one decisive wedge, a neck. Unmistakably a corvid at any size, and it sits beside the wide wordmark without competing.",
    shapes: [
      { circle: [18, 15, 9.5], kind: "fill" },
      { d: "M9.4 10.6 L0.8 15.4 L9.4 19.6 Z", kind: "fill" },
      { d: "M11.5 22 L24.5 22 L23 30 L13 30 Z", kind: "fill" },
      { circle: [20.5, 12.2, 1.8], kind: "eye" },
    ],
  },
  {
    id: "glyph", name: "Beak and eye", strokeWidth: 2.6,
    why: "The least that still reads as a rook: the open beak as a chevron, the crown as one arc, the eye. Built for 16 px.",
    shapes: [
      { d: "M13 8.5 L3 16 L13 23.5", kind: "stroke" },
      { d: "M13 8.5 C19 6.5 26 9 29 15.5", kind: "stroke" },
      { circle: [19.5, 13.5, 2.4], kind: "accent" },
    ],
  },
  {
    id: "flight", name: "In flight", strokeWidth: 2.4,
    why: "The bird leaving with something shiny: a lens of a body, the beak forward, the tail fanned, and the wings as two strokes.",
    shapes: [
      { d: "M5 18.5 C9 15.5 18 14.5 25 16.5 C19.5 20.5 10.5 21 5 18.5 Z", kind: "fill" },
      { circle: [7.6, 17.4, 3.2], kind: "fill" },
      { d: "M4.8 15.8 L0.4 17.9 L4.8 19.8 Z", kind: "fill" },
      { d: "M25 16.5 L31 13.6 L31 19.4 Z", kind: "fill" },
      { d: "M16 16.4 C14.4 10.4 10.6 6 5.4 4.6 M17.6 16 C21 9.6 26 6.4 31 6.6", kind: "stroke" },
      { circle: [8.4, 16.6, 1], kind: "eye" },
    ],
  },
];

/** The standalone file for a mark: ink and accent as literal colours (a file has no page to inherit from), eyes as the ground. */
export function svgOf(spec: MarkSpec, o: { ink: string; accent: string; ground: string; size?: number; rounded?: boolean }): string {
  const parts = spec.shapes.map((s) => {
    const paint = s.kind === "accent" ? `fill="${o.accent}"` : s.kind === "eye" ? `fill="${o.ground}"` : s.kind === "stroke" ? `fill="none" stroke="${o.ink}" stroke-width="${spec.strokeWidth}" stroke-linecap="round" stroke-linejoin="round"` : `fill="${o.ink}"`;
    return s.circle ? `<circle cx="${s.circle[0]}" cy="${s.circle[1]}" r="${s.circle[2]}" ${paint}/>` : `<path d="${s.d}" ${paint}/>`;
  });
  const size = o.size ?? 32;
  const bg = o.rounded ? `<rect width="32" height="32" rx="7" fill="${o.ground}"/>` : "";
  const inner = o.rounded ? `<g transform="translate(2 2) scale(0.875)">${parts.join("")}</g>` : parts.join("");
  return `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 32" width="${size}" height="${size}">${bg}${inner}</svg>\n`;
}
