import { useState } from "react";
import { initials, seedOf } from "../lib/plate";

/**
 * A collection's or an NFT's picture through the indexer (in the labelled fixture, straight from its public URL, sending no
 * referrer). While it loads, the frame shows the mark (two letters cut from the name on a ground from the seed) rather than an
 * empty grey tile; when there is no picture, or it fails, the mark stays — and with `noImage`, it says so in words. Never a
 * broken image.
 */
export function Picture({ src, seed, name, size = 40, round = "22%", className, noImage = false }: { src: string | null; seed: string; name: string; size?: number; round?: string; className?: string; noImage?: boolean }) {
  const [failed, setFailed] = useState(false);
  const [loaded, setLoaded] = useState(false);
  const hue = seedOf(seed) % 360;
  const ground = `hsl(${hue} 22% 24%)`, ink = `hsl(${hue} 40% 78%)`;
  if (!src || failed) return (
    <span className={`cat__tile${noImage ? " cat__tile--none" : ""}${className ? ` ${className}` : ""}`} role={noImage ? "img" : undefined} aria-label={noImage ? `${name}: no image` : undefined} aria-hidden={noImage ? undefined : true} title={name}
      style={{ width: size, height: size, borderRadius: round, background: ground, color: ink, fontSize: size * 0.36 }}>{initials(name)}{noImage ? <small>no image</small> : null}</span>);
  // the mark sits behind the picture as its background until the picture has loaded
  const mark = `url("data:image/svg+xml,${encodeURIComponent(`<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 100 100'><rect width='100' height='100' fill='${ground}'/><text x='50' y='50' dy='.35em' text-anchor='middle' font-family='ui-monospace,monospace' font-weight='600' font-size='34' fill='${ink}'>${initials(name).replace(/[<&>'"]/g, "")}</text></svg>`)}")`;
  return <img className={`cat__pic${loaded ? "" : " cat__pic--loading"}${className ? ` ${className}` : ""}`} src={src} alt="" width={size} height={size} loading="lazy" decoding="async" referrerPolicy="no-referrer"
    style={{ borderRadius: round, backgroundImage: loaded ? undefined : mark, backgroundSize: "cover" }} title={name} onLoad={() => setLoaded(true)} onError={() => setFailed(true)} />;
}
