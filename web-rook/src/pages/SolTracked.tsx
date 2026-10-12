import { useMemo, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { API_BASE, httpSource } from "../api/source";
import { href, useNow } from "../lib/env";
import { catalogFixtures, imageUrlOf } from "../rook/catalogFixtures";
import type { Catalog, CatalogEntry } from "../rook/catalogTypes";
import { STANDARD_WORD, countText, floorNote, floorText, usdText, volText } from "../rook/cataloglogic";
import { TRACKED_SORTS, TRACKED_TABS, countTabs, newestRecord, trackedRows, viewRows, type TrackedRow, type TrackedSort, type TrackedTab } from "../rook/sol/tracked";
import { Fig, Loading, NotYet, Pill } from "../ui/bits";
import { Picture } from "../ui/Picture";
import { COLLECTIONS } from "../rook/sol/collections";

type Answer = { catalog: Catalog; live: boolean };

/** The catalogue: the hosted indexer when this build names one and it answers, else the committed seed records (dated, labelled). */
async function readCatalog(): Promise<Answer> {
  if (API_BASE) {
    try { const c = await httpSource(API_BASE).get<Catalog>("/v1/rook/catalog"); if (Array.isArray(c.entries)) return { catalog: c, live: true }; } catch { /* fall back to the seed */ }
  }
  const src = await catalogFixtures();
  return { catalog: await src.get<Catalog>("/v1/rook/catalog"), live: false };
}
function useTracked() {
  return useQuery({ queryKey: ["sol-tracked"], staleTime: Infinity, refetchInterval: false, retry: 1, queryFn: readCatalog });
}
const day = (s: number) => new Date(s * 1000).toISOString().slice(0, 10);
const inSite = (slug: string) => href("collection", slug);
/** Pictures: the indexer's image route is gone with the Robinhood design; the browser loads the public URL the catalogue recorded, sending no referrer. */
const pic = (e: CatalogEntry) => imageUrlOf(e, null);

/**
 * Every collection Rook tracks (#/tracked): Bitcoin, Ethereum, Base and the rest, each with its floor, supply and holders, and
 * which of them Rook launches coins on (COLLECTIONS). A Rook collection's row opens its page here; any other opens the collection at its marketplace.
 */
export function SolTracked() {
  const q = useTracked(), now = useNow();
  const [tab, setTab] = useState<TrackedTab>("all");
  const [sort, setSort] = useState<TrackedSort>("rook");
  const [text, setText] = useState("");
  const [take, setTake] = useState(60);
  const rows = useMemo(() => (q.data ? trackedRows(q.data.catalog.entries, inSite) : []), [q.data]);
  const shown = useMemo(() => viewRows(rows, tab, text, sort), [rows, tab, text, sort]);
  const counts = useMemo(() => countTabs(rows), [rows]);
  const c = q.data?.catalog, newest = c ? newestRecord(c) : null;
  return (
    <>
      <header className="pagehead rise"><p className="eyebrow">Tracked collections</p><h1>Every collection Rook tracks</h1>
        <p className="lede">Blue-chip collections on Bitcoin, Ethereum, Base and elsewhere, each with its floor and its holders. Rook launches coins on {COLLECTIONS.length} of them, on Solana; <a href={href("collections")}>those {COLLECTIONS.length}</a> are marked.</p>
        {c ? <p className="small muted">{rows.length} collections · {rows.filter((r) => r.rook).length} of Rook's {COLLECTIONS.length} · {q.data!.live ? (c.refreshedAt ? `sources read ${day(c.refreshedAt)}` : "sources not read yet") : `figures are dated records${newest ? `, the newest from ${day(newest)}` : ""}: not the price now`}</p> : null}
      </header>
      {q.isLoading ? <Loading what="the collection list" /> : q.error ? <p className="error small" role="alert">The collection list could not be read: {String((q.error as Error).message).slice(0, 140)}</p> : null}
      {c ? (
        <div className="board rise rise-1">
          <div className="board__controls">
            <div className="tabs" role="tablist" aria-label="Which chain">{TRACKED_TABS.map(([t, label]) => <button key={t} type="button" role="tab" aria-selected={tab === t} className={tab === t ? "on" : ""} onClick={() => { setTab(t); setTake(60); }}>{label} <span className="muted">{counts[t]}</span></button>)}</div>
            <label className="board__search"><span className="vh">Search collections</span><input type="search" placeholder="Search name, chain or contract" value={text} autoComplete="off" spellCheck={false} onChange={(e) => setText(e.target.value)} /></label>
            <span className="board__sorts"><span className="small muted">Sort</span><span className="seg">{TRACKED_SORTS.map(([s, label]) => <button key={s} type="button" aria-pressed={sort === s} onClick={() => setSort(s)}>{label}</button>)}</span></span>
          </div>
          <ul className="cat">{shown.slice(0, take).map((r) => <li key={r.e.slug}><Row r={r} usdPerEth={c.usdPerEth} now={now} /></li>)}</ul>
          {!shown.length ? <p className="muted small" style={{ padding: "1rem 0" }}>{text ? `Nothing matches “${text}”.` : "No collection on this chain."}</p> : null}
          <p className="small muted board__count">{Math.min(take, shown.length)} of {shown.length} collections{shown.length > take ? <> · <button type="button" className="btn btn--quiet btn--small" onClick={() => setTake(take + 60)}>More</button></> : null}</p>
          <p className="small muted">{q.data!.live ? "Floors, counts and volumes come from the marketplaces' public directories, read by Rook's indexer." : "Floors, counts and volumes are the catalogue seed's dated records of what the marketplaces answered (OpenSea, Magic Eden, Ordinals Wallet), each kept with its date; none is a live price."} Pictures load from where the collection publishes them, with no referrer. <a href={href("docs", "collections-catalog")}>How the list is made</a>.</p>
        </div>) : null}
    </>
  );
}

function FloorValue({ e, usd }: { e: CatalogEntry; usd: string | null }) {
  const [main, ...rest] = floorText(e).split(" · ");
  const sub = [...rest, ...(usd ? [usd] : [])].join(" · ");
  return <>{main}{sub ? <small>{sub}</small> : null}</>;
}
function Row({ r, usdPerEth, now }: { r: TrackedRow; usdPerEth: number | null; now: number }) {
  const e = r.e, usd = usdText(e.floor?.eth, usdPerEth);
  const inner = (
    <>
      <Picture src={pic(e)} seed={e.slug} name={e.name} size={48} />
      <span className="cat__name"><strong>{e.name}</strong><small><span className={`chip chip--${e.chain}`}>{e.chainName}</span> {STANDARD_WORD[e.standard]}{e.floor?.stale ? " · dated floor" : ""}</small></span>
      <Fig label="Floor" value={e.floor ? <span title={floorNote(e, now)}><FloorValue e={e} usd={usd} /></span> : <NotYet />} />
      <Fig label={e.chain === "bitcoin" ? "Ordinals" : "NFTs"} value={e.items === null ? <NotYet /> : countText(e.items)} />
      <Fig label="Holders" value={e.owners === null ? <NotYet /> : countText(e.owners)} />
      <Fig label="7d volume" value={e.volume7dEth === null ? <NotYet /> : volText(e.volume7dEth)} />
      <span className="cat__status">{r.rook ? <Pill tone="live">Rook · {r.rook.wxSymbol}</Pill> : <Pill>tracked{r.external && r.link ? " ↗" : ""}</Pill>}</span>
    </>
  );
  if (!r.link) return <div className="cat__row">{inner}</div>;
  return r.external
    ? <a className="cat__row" href={r.link} target="_blank" rel="noreferrer noopener" title={`${e.name} at its marketplace (opens a new tab)`}>{inner}</a>
    : <a className="cat__row" href={r.link}>{inner}</a>;
}

/** `#/collection/<slug>` for a tracked collection that is not one of Rook's collections: what the list knows, and where it trades. */
export function TrackedOne({ slug }: { slug?: string }) {
  const q = useTracked(), now = useNow();
  const row = useMemo(() => (q.data && slug ? trackedRows(q.data.catalog.entries, inSite).find((r) => r.e.slug.toLowerCase() === slug.toLowerCase()) ?? null : null), [q.data, slug]);
  if (q.isLoading) return <Loading what="the collection" />;
  if (!row) return <div className="pagehead rise"><p className="eyebrow">Collection</p><h1>Not a collection Rook knows</h1><p className="lede">No tracked collection has the name <span className="mono break">{slug ?? ""}</span>. <a href={href("tracked")}>Every tracked collection</a> · <a href={href("collections")}>Rook's {COLLECTIONS.length}</a>.</p></div>;
  const e = row.e, l = e.links;
  return (
    <>
      <header className="colhead rise">
        <Picture src={pic(e)} seed={e.slug} name={e.name} size={112} />
        <div className="colhead__id">
          <p className="eyebrow" style={{ margin: 0 }}>Tracked collection · {e.chainName}</p>
          <h1 style={{ margin: ".15rem 0 .35rem" }}>{e.name}</h1>
          <p className="muted" style={{ margin: 0 }}>Rook does not launch coins on this collection. Coins launch on <a href={href("collections")}>{COLLECTIONS.length} collections</a>, on Solana.</p>
        </div>
      </header>
      <div className="strip strip--stats rise rise-1">
        <Fig label="Floor" value={e.floor ? floorText(e) : "—"} />
        <Fig label={e.chain === "bitcoin" ? "Ordinals" : "NFTs"} value={countText(e.items)} />
        <Fig label="Holders" value={countText(e.owners)} />
        <Fig label="7d volume" value={volText(e.volume7dEth)} />
      </div>
      <p className="small muted">{floorNote(e, now)}</p>
      <p className="row small" style={{ gap: ".5rem 1rem" }}>
        {[["Marketplace", l.marketplace], ["Explorer", l.explorer], ["Website", l.website], ["X", l.x]].filter(([, u]) => u && /^https:\/\//.test(u)).map(([k, u]) => <a key={k} href={u!} target="_blank" rel="noreferrer noopener">{k} ↗</a>)}
        <a href={href("tracked")}>Every tracked collection</a>
      </p>
    </>
  );
}
