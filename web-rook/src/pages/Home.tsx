import { href, useEnv } from "../lib/env";
import { useWrappedList, useWrappedSource } from "../lib/wrapped";
import { useCatalog, type CatalogSrc } from "../rook/catalogData";
import { liveCount, vaultTiles } from "../rook/cataloglogic";
import { useActivity, useBoard } from "../rook/data";
import { Board, King, Ticker } from "../ui/Board";
import { Fig, FixtureBanner, Loading, NothingDeployed } from "../ui/bits";
import { Picture } from "../ui/Picture";
import type { WrappedSummary } from "../api/types";
import type { Catalog } from "../rook/catalogTypes";

/**
 * The board, not a brochure (docs/v6/site-design.md §4, amended 2026-09-23; the design review of 2026-09-27): the headline with
 * room to breathe and the vault's count under it, what is in the vault now, the live tape, the top coin, then every coin with
 * tabs, sorts and search. The vault's count comes from the same reads the collection pages make; the live-collection count and
 * the list's size from /v1/rook/catalog; the board from /v1/rook/board; the tape from /v1/rook/activity.
 */
export function Home() {
  const env = useEnv();
  const board = useBoard(), activity = useActivity();
  const wsrc = useWrappedSource(), list = useWrappedList(), cat = useCatalog();
  const cols = list.data?.collections ?? [];
  // docs/v6/collections-catalog.md §4: the collections counted are the live ones — what has coins buying its floor — and the list's size is said beside it
  const live = liveCount(cat.data) ?? (list.data ? cols.length : null);
  const fixture = board.source?.kind === "fixture";
  const nothing = env !== null && !board.sourceLoading && !board.source;
  const items = cols.reduce((n, c) => n + (c.itemsHeld ?? 0), 0);
  return (
    <>
      {fixture && board.source ? <FixtureBanner note={board.source.note} /> : null}
      <section className="hero2 rise">
        <div className="hero2__say">
          <h1 className="hero2__h1">Memecoins that buy the floor.</h1>
          <p className="muted hero2__lede">Every trade's fee buys blue-chip NFTs from a collection's floor into a vault; the NFTs are raffled to holders, burnt, or relisted to buy the coin back.</p>
          <p className="hero2__acts"><a className="btn btn--primary" href={href("launch")}>Launch a coin</a><a className="btn btn--quiet" href={href("docs", "site-design")}>How it works</a></p>
        </div>
        <div className="hero2__figs">
          <Fig label="NFTs in the vault" value={list.data ? items : wsrc.data ? "…" : "—"} />
          <Fig label="Live collections" value={live ?? "—"} />
          <Fig label="Collections tracked" value={cat.data ? cat.data.entries.length : "—"} />
          <Fig label="Coins" value={board.data ? board.data.rows.length : "—"} />
          <Fig label="Trading today" value={board.data ? board.data.rows.filter((r) => (r.trades24h ?? 0) > 0).length : "—"} />
        </div>
      </section>
      {list.data && cat.data && cat.source ? <InTheVault cols={cols} cat={cat.data} src={cat.source} fixture={fixture} /> : null}
      {nothing ? <NothingDeployed page="" /> : null}
      {activity.data ? <Ticker items={activity.data.items} fixture={fixture} /> : null}
      {board.data ? <div className="rise rise-1"><King rows={board.data.rows} usdPerEth={board.data.usdPerEth} /></div> : null}
      <section className="rise rise-2" style={{ marginTop: "1.25rem" }}>
        {board.isLoading || board.sourceLoading ? <Loading what="the board" /> : board.error ? <p className="error small">The board could not be read: {String((board.error as Error).message).slice(0, 120)}</p> : board.data ? <Board rows={board.data.rows} usdPerEth={board.data.usdPerEth} /> : null}
      </section>
    </>
  );
}

/** What the vault holds, as pictures: one per NFT held, each linking to that collection's reserves. */
function InTheVault({ cols, cat, src, fixture }: { cols: WrappedSummary[]; cat: Catalog; src: CatalogSrc; fixture: boolean }) {
  const v = vaultTiles(cols, cat.entries);
  if (!v.held) return null;
  const by = cols.filter((c) => (c.itemsHeld ?? 0) > 0);
  return (
    <section className="vaultstrip rise rise-1" aria-labelledby="vault-now">
      <div className="vaultstrip__head">
        <p className="eyebrow" id="vault-now" style={{ margin: 0 }}>In the vault now</p>
        <span className="small muted">{by.map((c) => `${c.itemsHeld} ${c.name}`).join(" · ")}</span>
        <a className="small right" href={href("reserves")}>Check the reserves →</a>
      </div>
      <ul className="vaultstrip__row">
        {v.tiles.map((t, i) => <li key={`${t.slug}-${i}`}><a href={href("reserves", t.slug)} title={`${t.name}: see the reserves`}><Picture src={src.image(t.entry, t.item)} seed={`${t.entry.slug}/${t.item}`} name={t.name} size={72} round="14%" className="vaultstrip__pic" /></a></li>)}
        {v.more ? <li><a className="vaultstrip__more" href={href("reserves")}>+{v.more}</a></li> : null}
      </ul>
      <p className="small muted" style={{ margin: ".4rem 0 0" }}>One picture per NFT held, from each collection's own pictures; the exact NFTs, with their ids and checks, are on the reserves page{fixture ? " (in this fixture the held ids are invented)" : ""}.</p>
    </section>
  );
}
