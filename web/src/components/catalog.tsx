import type { ReactNode } from "react";
import { href } from "../lib/env";
import { eth } from "../lib/format";
import { CoinArt, Monogram, RuneFace } from "./Plate";
import { logoSrc } from "../lib/plate";
import { CardSpark } from "./CardSpark";
import type { CoinRow } from "../lib/catalog";
import type * as Api from "../api/types";

/** A row of choices, each one cut into the stone; the chosen one is filled. Used for sorts and filters alike. */
export function Chips<T extends string | number | boolean | null>({ label, value, options, onPick }: {
  label: string; value: T; options: [T, string][]; onPick: (v: T) => void;
}) {
  return (
    <div className="chipset" role="group" aria-label={label}>
      <span className="chipset__label" aria-hidden="true">{label}</span>
      <span className="chips">
        {options.map(([v, text]) => (
          <button key={String(v)} type="button" className={`chip${v === value ? " chip--on" : ""}`} aria-pressed={v === value} onClick={() => onPick(v)}>{text}</button>
        ))}
      </span>
    </div>
  );
}

const ago = (s: number | null): string => {
  if (!s) return "not known";
  const d = Math.floor(Date.now() / 1000) - s;
  if (d < 90) return "just now";
  if (d < 5400) return `${Math.round(d / 60)} min ago`;
  if (d < 172800) return `${Math.round(d / 3600)} h ago`;
  return `${Math.round(d / 86400)} d ago`;
};

/**
 * One coin, cut into the page: its face, what it is paired with, what its fees have done.
 *
 * The face is a monogram cut from the coin's own address until the indexer can hand over the logo Pons takes at launch
 * (docs/phase9.md, section 5). The component already prefers a logo the moment one exists.
 */
export function CoinCard({ c }: { c: CoinRow }) {
  const harvested = c.harvests !== null && c.harvests > 0;
  return (
    <li className="card">
      <a className="card__in" href={href("coin", c.address)}>
        <CoinArt
          address={c.address}
          name={c.name}
          symbol={c.symbol}
          logo={logoSrc(c.logoUrl)}
          tag={harvested ? `harvested ${c.harvests}×` : "no fees routed yet"}
        />
        <span className="card__body">
          <span className="card__head">
            <span className="card__name">{c.name}</span>
            <span className="mono quiet">{c.symbol}</span>
          </span>
          <CardSpark address={c.address} symbol={c.symbol} />
          <span className="card__num num">{c.lifetimeFeesWei === null ? "not known" : eth(c.lifetimeFeesWei, 4)}</span>
          <span className="card__unit">ETH of fees routed</span>
          <span className="card__foot">
            <span>{c.listingName ?? `listing ${c.listingId}`}</span>
            <span>{c.creatorTaxBps === null ? "tax unknown" : `${(c.creatorTaxBps / 100).toFixed(c.creatorTaxBps % 100 ? 2 : 0)}% tax`}</span>
            <span>{ago(c.launchedAt)}</span>
          </span>
        </span>
      </a>
    </li>
  );
}

export const CoinCards = ({ coins }: { coins: CoinRow[] }) => <ul className="cards">{coins.map((c) => <CoinCard key={c.address} c={c} />)}</ul>;

/** Vaults ranked by what has reached them. The order is the ranking, so nothing is numbered. */
export function ListingRail({ listings, limit }: { listings: Api.ListingSummary[]; limit?: number }) {
  const sorted = [...listings].sort((a, b) => (BigInt(b.depositedWei) > BigInt(a.depositedWei) ? 1 : BigInt(b.depositedWei) < BigInt(a.depositedWei) ? -1 : a.id - b.id));
  return (
    <ul className="rail">
      {sorted.slice(0, limit ?? sorted.length).map((l) => (
        <li key={l.id}>
          <a className="rail__in" href={href("listing", l.id)}>
            <span className="rail__art">
              {l.kind === "rune"
                ? <span className="plate plate--rune"><RuneFace symbol={null} name={l.name} /><span className="plate__tag">vault</span></span>
                : <span className="plate"><Monogram id={`listing-${l.id}-${l.ref}`} text={l.name} /><span className="plate__tag">vault</span></span>}
            </span>
            <span className="rail__top">
              <span className="rail__name">{l.name}</span>
              <span className="tag">{l.kind === "rune" ? "rune" : "inscriptions"}</span>
              {l.paused ? <span className="tag tag--warn">paused</span> : null}
            </span>
            <span className="rail__figs">
              <span><span className="num">{eth(BigInt(l.depositedWei), 3)}</span> ETH in the vault</span>
              <span><span className="num">{l.coins}</span> coin{l.coins === 1 ? "" : "s"} paying in</span>
              <span><span className="num">{l.acquisitions}</span> bought</span>
            </span>
          </a>
        </li>
      ))}
    </ul>
  );
}

/** Said once per page that needs the indexer and cannot reach one. Not an error: the chain is being read, it just cannot list. */
export const NeedsIndexer = ({ what }: { what: string }) => (
  <div className="notice" role="status">
    <h2>No indexer to read</h2>
    <p>This build reads the chain directly, which answers questions about one coin or one listing but cannot assemble {what}. That comes from the indexer, and none is configured here.</p>
    <p>Open a <a href="?demo=sample#/">recorded demo</a> to see the page with figures in it, or a coin's own page to read it straight from the chain.</p>
  </div>
);

/** How many rows are shown, and whether that is all of them. Says so rather than implying a complete list. */
export const Count = ({ shown, total, noun }: { shown: number; total: number; noun: string }) => (
  <p className="count quiet">{shown === total ? <>{total} {noun}{total === 1 ? "" : "s"}</> : <>{shown} of {total} {noun}s, sorted within the {shown} this page fetched</>}</p>
);

export const PageHead = ({ title, children }: { title: string; children?: ReactNode }) => (
  <header className="pagehead"><h1>{title}</h1>{children}</header>
);
