/**
 * What a plate is cut from when there is no picture.
 *
 * Kept apart from the component so it can be tested without a browser, and because the result must be stable: the same coin
 * has to be cut the same way on every render, on every machine, forever. docs/phase9.md, section 1.
 */

/** FNV-1a: small, and identical in a test and in a browser. */
export function seedOf(s: string): number {
  let h = 0x811c9dc5;
  for (let i = 0; i < s.length; i++) {
    h ^= s.charCodeAt(i);
    h = Math.imul(h, 0x01000193) >>> 0;
  }
  return h >>> 0;
}

/** Four tool angles, not three hundred and sixty: a mason works to a bevel, and free angles read as noise. */
export const ANGLES = [21, 66, 114, 159];

/** Two characters, chosen the way a mason would: the initials that identify it, never more than fit. */
export function initials(text: string): string {
  const clean = text.replace(/[^\p{L}\p{N}]+/gu, " ").trim();
  if (!clean) return "··";
  const words = clean.split(/\s+/);
  if (words.length >= 2) return (words[0][0] + words[1][0]).toUpperCase();
  return clean.slice(0, 2).toUpperCase();
}

/** The cut itself: an angle, a spacing and an offset, all derived from the identifier and none of them random. */
export function chiselOf(id: string): { angle: number; gap: number; shift: number } {
  const h = seedOf(id);
  return { angle: ANGLES[h % ANGLES.length], gap: 7 + ((h >> 3) % 7), shift: (h >> 7) % 5 };
}

/**
 * A coin's logo, as the chain has it, turned into something a browser can fetch.
 *
 * The indexer hands the string over untouched and names no gateway, because the gateway is the site's choice, not the
 * chain's (indexer/API.md). This is that choice, in one place: `ipfs://` is resolved through a public gateway, `https://`
 * and `data:image/` are already fetchable, and anything else is refused and the coin keeps its monogram.
 *
 * It is a stranger's string, so it is only ever used as an image's `src`, never as markup.
 */
const IPFS_GATEWAY = "https://ipfs.io/ipfs/";

export function logoSrc(raw: string | null | undefined): string | null {
  if (!raw) return null;
  const s = raw.trim();
  if (s.length > 1024) return null;
  if (/\s/.test(s)) return null;
  if (s.startsWith("ipfs://")) {
    const path = s.slice("ipfs://".length).replace(/^ipfs\//, "");
    return path ? IPFS_GATEWAY + path : null;
  }
  if (s.startsWith("https://")) return s;
  if (s.startsWith("data:image/")) return s;
  return null;
}
