import { COLLECTIONS, availabilityOf, type Chain, type SolCollection } from "../rook/sol/collections";
import { Picture } from "./Picture";

/** The order the marquee deals the chains in. Robinhood Chain's collections come last in each round, marked "soon". */
const MARQUEE_CHAINS: readonly Chain[] = ["Ethereum", "Solana", "Bitcoin", "Robinhood Chain"];

/** Every collection with a page, dealt round-robin from each chain's list in COLLECTIONS order (Ethereum, Solana, Bitcoin, Robinhood,
 *  Ethereum, ...) so neighbours come from different chains, then cut in two: the marquee's top row (the first half) and its bottom row.
 *  The same on every render. */
export function marqueeRows(): [SolCollection[], SolCollection[]] {
  const lists = MARQUEE_CHAINS.map((ch) => COLLECTIONS.filter((c) => c.chain === ch));
  const all: SolCollection[] = [];
  for (let i = 0; lists.some((l) => i < l.length); i++) for (const l of lists) if (i < l.length) all.push(l[i]);
  const half = Math.ceil(all.length / 2);
  return [all.slice(0, half), all.slice(half)];
}

const CHAIN_TAG: Record<Chain, string> = { Ethereum: "ETH", Solana: "SOL", Bitcoin: "ORD", "Robinhood Chain": "HOOD" };

/** One tile: the collection's picture, its name, and what a coin on it needs to graduate (or "soon" for a collection pinned later). */
function Tile({ c, hrefOf, dup }: { c: SolCollection; hrefOf: (slug: string) => string; dup: boolean }) {
  const soon = availabilityOf(c) !== "pinned";
  return (
    <li className={`cmarq__tile${soon ? " cmarq__tile--soon" : ""}`} aria-hidden={dup || undefined}>
      <a href={hrefOf(c.slug)} tabIndex={dup ? -1 : undefined} title={`${c.name} · ${c.chain === "Bitcoin" ? "Ordinals" : c.chain}`}>
        <Picture src={c.logo} seed={c.slug} name={c.name} size={44} round="12px" />
        <span className="cmarq__say">
          <strong>{c.name}</strong>
          <small><span className={`cmarq__chain cmarq__chain--${CHAIN_TAG[c.chain].toLowerCase()}`}>{CHAIN_TAG[c.chain]}</span>{soon ? "soon" : <>≈ {c.tierSol} SOL to graduate</>}</small>
        </span>
      </a>
    </li>
  );
}

/** The front page hero's collections band: two rows of collection tiles drifting in opposite directions under the hero's two halves,
 *  faded at the edges, paused while hovered or while a link in it has focus. Each row is drawn twice for a seamless loop; the second copy
 *  is hidden from assistive technology and kept out of the tab order, so every collection is one labelled link exactly once. With
 *  reduced motion the rows stand still and scroll sideways inside the band (never the page). `hrefOf` is the collection page's link
 *  (the page passes env's `href`, which keeps ?demo). */
export function CollectionMarquee({ hrefOf, allHref, count }: { hrefOf: (slug: string) => string; allHref: string; count: number }) {
  const rows = marqueeRows();
  return (
    <section className="cmarq" aria-labelledby="cmarq-h">
      <p className="cmarq__head"><span className="hero3__k" id="cmarq-h">{count} collections to launch on<span className="cmarq__soon"> · Robinhood Chain soon</span></span><a href={allHref}>All collections →</a></p>
      {rows.map((row, r) => (
        <div key={r} className={`cmarq__rail cmarq__rail--${r ? "rev" : "fwd"}`}>
          <ul className="cmarq__track" style={{ animationDuration: `${row.length * 5}s` }}>
            {row.map((c) => <Tile key={c.slug} c={c} hrefOf={hrefOf} dup={false} />)}
            {row.map((c) => <Tile key={"d" + c.slug} c={c} hrefOf={hrefOf} dup={true} />)}
          </ul>
        </div>
      ))}
    </section>
  );
}
