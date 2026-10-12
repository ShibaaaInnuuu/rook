import { logoSrc } from "../lib/plate";
import { seedOf } from "../lib/plate";

/**
 * A coin's face: its logo when the chain has one, else a mark cut from its address — a 5×5 grid, mirrored, in the ink and the
 * accent at two tints — so the same coin is always the same face and two coins are never alike. Nothing random.
 */
export function Identicon({ id, size = 36 }: { id: string; size?: number }) {
  const h = seedOf(id.toLowerCase());
  const cells: { x: number; y: number; k: number }[] = [];
  let bits = h;
  for (let y = 0; y < 5; y++) for (let x = 0; x < 3; x++) {
    const on = (bits & 3) !== 0, k = bits & 3; bits = (bits >>> 2) ^ (h << (x + y)) >>> 0;
    if (on) { cells.push({ x, y, k }); if (x < 2) cells.push({ x: 4 - x, y, k }); }
  }
  const hue = h % 360;
  return (
    <svg className="face" width={size} height={size} viewBox="0 0 5 5" aria-hidden="true" style={{ background: `hsl(${hue} 18% 18%)`, borderRadius: "22%" }}>
      {cells.map((c, i) => <rect key={i} x={c.x} y={c.y} width="1" height="1" fill={c.k === 3 ? "var(--accent)" : c.k === 2 ? `hsl(${hue} 30% 62%)` : `hsl(${hue} 22% 42%)`} />)}
    </svg>
  );
}

export function CoinFace({ address, logoUrl, name, size = 36 }: { address: string; logoUrl: string | null; name: string; size?: number }) {
  const src = logoSrc(logoUrl);
  return src ? <img className="face" src={src} alt="" width={size} height={size} loading="lazy" style={{ borderRadius: "22%", objectFit: "cover" }} title={name} /> : <Identicon id={address} size={size} />;
}
