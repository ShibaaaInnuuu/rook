/** Types for scripts/og-image.mjs, which vite.config.ts and test/og.test.ts import. */
export const WIDTH: number;
export const HEIGHT: number;
export const GROUND: number[];
export const INK: number[];
export const ACCENT: number[];
export const WORDMARK: { unitsPerEm: number; capHeight: number; ascent: number; descent: number; advance: number; glyphs: { ch: string; x: number; d: string }[] };
export function flatten(d: string, tf: (x: number, y: number) => [number, number], minPoints?: number): [number, number][][];
export function circle(cx: number, cy: number, r: number, tf: (x: number, y: number) => [number, number], steps?: number): [number, number][][];
export function stroke(d: string, width: number, tf: (x: number, y: number) => [number, number]): [number, number][][];
export function paint(rgb: Buffer, polys: [number, number][][], color: number[], W?: number, H?: number): void;
export function paintMark(rgb: Buffer, left: number, top: number, size: number, W?: number, H?: number): void;
export function ogPixels(): Buffer;
export function pngOf(rgb: Buffer, w: number, h: number): Buffer;
export function ogPng(): Buffer;
