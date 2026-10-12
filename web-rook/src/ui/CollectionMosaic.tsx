import { COLLECTIONS, type Chain, type SolCollection } from "../rook/sol/collections";
import { Picture } from "./Picture";

/** The chains the mosaic mixes, in the order it deals from them. */
const MOSAIC_CHAINS: readonly Chain[] = ["Ethereum", "Solana", "Bitcoin"];

/** `n` collections for the hero mosaic, dealt round-robin from each chain's list in COLLECTIONS order (Ethereum, Solana, Bitcoin, Ethereum, ...),
 *  so the grid mixes the chains and is the same on every render. A chain that runs out is skipped. */
export function mosaicPicks(n: number): SolCollection[] {
  const lists = MOSAIC_CHAINS.map((ch) => COLLECTIONS.filter((c) => c.chain === ch && c.logo));
  const out: SolCollection[] = [];
  for (let i = 0; out.length < n && lists.some((l) => i < l.length); i++)
    for (const l of lists) if (i < l.length && out.length < n) out.push(l[i]);
  return out;
}

/** The collections page hero's picture: a tilted, staggered 5 × 4 wall of the collections' own images fading out at its edges, each tile a
 *  link to its collection. Decorative (the same links are in the list below it), so it is hidden from assistive technology and kept out of the
 *  tab order. Shown from 900 px wide only (rook.css `.cmosaic`). `hrefOf` is the collection page's link (the page passes env's `href`, which
 *  keeps ?demo). The picture is CollectionLogo's (Picture, its initials mark as the fallback). */
export function CollectionMosaic({ hrefOf, count = 20 }: { hrefOf: (slug: string) => string; count?: number }) {
  const picks = mosaicPicks(count);
  return (
    <div className="cmosaic" aria-hidden="true">
      <div className="cmosaic__grid">
        {picks.map((c) => <a key={c.slug} className="cmosaic__tile" href={hrefOf(c.slug)} tabIndex={-1} aria-label={c.name} title={`${c.name} · ${c.chain === "Bitcoin" ? "Ordinals" : c.chain}`}>
          <Picture src={c.logo} seed={c.slug} name={c.name} size={84} />
        </a>)}
      </div>
    </div>
  );
}
