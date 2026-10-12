import { useMemo, useState } from "react";
import type { WrappedCoin } from "../api/types";
import { href, useEnv, useNow } from "../lib/env";
import { useWrappedCollection, useWrappedSource } from "../lib/wrapped";
import { eth, when } from "../lib/format";
import { Picture } from "../ui/Picture";
import { useCatalog, useCatalogEntry, type CatalogSrc } from "../rook/catalogData";
import type { CatalogEntry, CatalogItem, CatalogRule, CatalogSourceName } from "../rook/catalogTypes";
import { CATALOG_SORTS, CHAIN_TABS, STANDARD_WORD, STATUS_TONE, STATUS_WORD, countByChain, countText, ethText, floorNote, floorText, launchable, liveEntries, searchEntries, sortEntries, tabEntries, usdText, volText, type CatalogSort, type ChainTab } from "../rook/cataloglogic";
import { poolView } from "../rook/pool";
import { Fig, FixtureBanner, Loading, NothingDeployed, NotYet, Pill, Provenance, StatusPill, Wrapped, ago } from "../ui/bits";
import { nftWord, wrappedLine } from "../rook/words";
import { COLLECTIONS } from "../rook/sol/collections";

const WORD: Record<WrappedCoin["phase"], string> = { trading: "on the curve", graduating: "graduating", graduated: "graduated", refunding: "refunding" };
const SOURCE_WORD: Record<CatalogSourceName, string> = { ordinalswallet: "Ordinals Wallet", magiceden: "Magic Eden", opensea: "OpenSea", coinbase: "Coinbase (the rate)", seed: "the seed's record", nosh: "Nosh (OpenSea's figure)", ord: "the ordinals content gateway", tokenuri: "the contract's tokenURI (public RPC)", rook: "this indexer" };

export { Picture };
function StatusBadge({ e }: { e: CatalogEntry }) {
  return <Pill tone={STATUS_TONE[e.rook.kind]}>{STATUS_WORD[e.rook.kind]}{e.rook.kind === "live" && e.rook.coins ? ` · ${e.rook.coins} coin${e.rook.coins === 1 ? "" : "s"}` : e.rook.kind === "later" && e.rook.stage !== null ? ` · stage ${e.rook.stage}` : ""}</Pill>;
}
const ChainBadge = ({ e }: { e: CatalogEntry }) => <span className={`chip chip--${e.chain}`}>{e.chainName}</span>;
const linkOf = (e: CatalogEntry) => (e.rook.kind === "live" ? href("collection", e.rook.slug) : href("collection", e.slug));

// ---- the collection list (the catalogue): every collection, an open nest each ---------------------------------------------------------------------------------
export function Collections() {
  const env = useEnv(), cat = useCatalog(), now = useNow();
  const [tab, setTab] = useState<ChainTab>("all");
  const [sort, setSort] = useState<CatalogSort>("status");
  const [q, setQ] = useState("");
  const [take, setTake] = useState(60);
  const rows = cat.data?.entries ?? [];
  const shown = useMemo(() => sortEntries(searchEntries(tabEntries(rows, tab), q), sort), [rows, tab, q, sort]);
  const counts = useMemo(() => countByChain(rows), [rows]);
  if (env === null || cat.sourceLoading) return <Loading what="where to read from" />;
  if (!cat.source) return <NothingDeployed page="collections" />;
  const c = cat.data, live = c ? liveEntries(c.entries).length : null;
  return (
    <>
      {cat.source.kind === "fixture" ? <FixtureBanner note={cat.source.note} /> : null}
      <header className="pagehead rise"><p className="eyebrow">Collections</p><h1>Every blue-chip collection, and where it stands</h1>
        <p className="lede">The blue-chip collections Rook tracks on Bitcoin, Ethereum and Base, each with its floor and its holders. Rook launches coins on {COLLECTIONS.length} of them, on Solana: those are on the collections page.</p>
        {c ? <p className="small muted">{c.entries.length} collections · {live} live on Rook · {c.refreshedAt ? `sources read ${ago(c.refreshedAt, now)}` : c.attemptedAt ? `sources last asked ${ago(c.attemptedAt, now)}; some did not answer` : "no source read yet: the seed's dated records"}{c.stale ? " · figures marked stale are not the price now" : ""}</p> : null}
      </header>
      {cat.isLoading ? <Loading what="the collections" /> : cat.error ? <p className="error small">The collection list could not be read: {String((cat.error as Error).message).slice(0, 120)}</p> : null}
      {c ? (
        <div className="board rise rise-1">
          <div className="board__controls">
            <div className="tabs" role="tablist" aria-label="Which chain">{CHAIN_TABS.map(([t, label]) => <button key={t} type="button" role="tab" aria-selected={tab === t} className={tab === t ? "on" : ""} onClick={() => { setTab(t); setTake(60); }}>{label} <span className="muted">{counts[t]}</span></button>)}</div>
            <label className="board__search"><span className="vh">Search collections</span><input type="search" placeholder="Search name, chain, status or contract" value={q} autoComplete="off" spellCheck={false} onChange={(e) => setQ(e.target.value)} /></label>
            <span className="board__sorts"><span className="small muted">Sort</span><span className="seg">{CATALOG_SORTS.map(([s, label]) => <button key={s} type="button" aria-pressed={sort === s} onClick={() => setSort(s)}>{label}</button>)}</span></span>
          </div>
          <ul className="cat">{shown.slice(0, take).map((e) => <li key={e.slug}><CatalogRow e={e} src={cat.source!} usdPerEth={c.usdPerEth} /></li>)}</ul>
          {!shown.length ? <p className="muted small" style={{ padding: "1rem 0" }}>{q ? `Nothing matches “${q}”.` : "No collection on this chain."}</p> : null}
          <p className="small muted board__count">{Math.min(take, shown.length)} of {shown.length} collections{shown.length > take ? <> · <button type="button" className="btn btn--quiet btn--small" onClick={() => setTake(take + 60)}>More</button></> : null}</p>
          <RulePanel rule={c.rule} />
          <Provenance className="prov--block" line={<>Floors, counts and pictures come from the marketplaces' public directories, read by our indexer and never by your browser. <a href={href("docs", "collections-catalog")}>How the collection list is made</a>.</>}>
            Sources on the last refresh: {c.sources.length ? c.sources.map((s) => `${SOURCE_WORD[s.name]} ${s.answered}/${s.asked}${s.statuses.length ? ` (HTTP ${s.statuses.join(", ")})` : " (no answer)"}`).join(" · ") : "none asked yet"}. Names and links come from a curated seed (Nosh's registry on Robinhood Chain and the ordinals collections the old etched site listed); floors, counts, volumes and pictures from the marketplaces' public directories, read by the indexer and never by your browser.
          </Provenance>
        </div>) : null}
    </>
  );
}
/** The floor's main figure on one line; a Bitcoin floor's sats and the USD figure go on the small line beneath, so rows keep one height. */
function FloorValue({ e, usd }: { e: CatalogEntry; usd: string | null }) {
  const [main, ...rest] = floorText(e).split(" · ");
  const sub = [...rest, ...(usd ? [usd] : [])].join(" · ");
  return <>{main}{sub ? <small>{sub}</small> : null}</>;
}
function CatalogRow({ e, src, usdPerEth }: { e: CatalogEntry; src: CatalogSrc; usdPerEth: number | null }) {
  const usd = usdText(e.floor?.eth, usdPerEth);
  return (
    <a className={`cat__row${e.rook.kind === "unsupported" ? " cat__row--dim" : ""}`} href={linkOf(e)}>
      <Picture src={src.image(e)} seed={e.slug} name={e.name} size={48} />
      <span className="cat__name"><strong>{e.name}</strong><small><ChainBadge e={e} /> {STANDARD_WORD[e.standard]}{e.floor?.stale ? " · stale floor" : ""}</small></span>
      <Fig label="Floor" value={e.floor ? <FloorValue e={e} usd={usd} /> : <NotYet />} />
      <Fig label={e.chain === "bitcoin" ? "Ordinals" : "NFTs"} value={e.items === null ? <NotYet /> : countText(e.items)} />
      <Fig label="Holders" value={e.owners === null ? <NotYet /> : countText(e.owners)} />
      <Fig label="7d volume" value={e.volume7dEth === null ? <NotYet /> : volText(e.volume7dEth)} />
      <span className="cat__status"><StatusBadge e={e} /></span>
    </a>
  );
}
function RulePanel({ rule }: { rule: CatalogRule }) {
  const w = (s: string) => ethText(Number(BigInt(s) / 10n ** 12n) / 1e6);
  return (
    <div className="panel panel--tight" style={{ marginTop: "1.25rem" }}>
      <p className="eyebrow">The listing rule · stage {rule.stage} ({rule.stageName})</p>
      <p style={{ margin: 0 }}>{rule.text}</p>
      <dl className="kv kv--left small" style={{ marginTop: ".75rem" }}>
        <dt>Per-NFT maximum</dt><dd>{w(rule.perItemMaxWei)} ETH</dd>
        <dt>Per-collection cap</dt><dd>{w(rule.perCollectionCapWei)} ETH, {rule.perCollectionItems} NFTs</dd>
        <dt>Allowance over the floor</dt><dd>{rule.allowanceBps / 100}%</dd>
        <dt>Highest listable floor</dt><dd>{w(rule.maxFloorWei)} ETH</dd>
      </dl>
      <p className="small muted" style={{ margin: ".5rem 0 0" }}><strong>Live</strong> is listed on Rook; <strong>listable</strong> meets the rule at this stage; <strong>later</strong> is above this stage's caps (the badge says which stage fits) or has no floor to check; <strong>unsupported</strong> is a chain or a standard no vault here can hold. The caps and the stages are <a href={href("docs", "launch-gates")}>{rule.doc.split(";")[0]}</a>.</p>
    </div>
  );
}

// ---- one collection: the header, the stats strip, the items, the coins, the activity, the launch button ----------------------------------
export function CollectionPage({ slug }: { slug?: string }) {
  const env = useEnv(), one = useCatalogEntry(slug), now = useNow();
  if (env === null || one.sourceLoading) return <Loading what="where to read from" />;
  if (!one.source) return <NothingDeployed page={`collection/${slug ?? ""}`} />;
  if (one.isLoading) return <Loading what="the collection" />;
  if (one.error || !one.data) return <div className="pagehead"><h1>No such collection</h1><p className="lede">No collection in the list is called “{slug}”. <a href={href("collections")}>All collections</a>.</p></div>;
  const { entry: e, rule, usdPerEth } = one.data, src = one.source, fixture = src.kind === "fixture";
  const usd = usdText(e.floor?.eth, usdPerEth);
  const links: [string, string | null][] = [["Website", e.links.website], ["X", e.links.x], ["Discord", e.links.discord], ["Marketplace", e.links.marketplace], ["Explorer", e.links.explorer]];
  const can = launchable(e), launchTo = e.rook.kind === "live" ? href("launch", e.rook.slug) : href("launch", e.slug);
  const bitcoin = e.chain === "bitcoin";
  // design review 2026-09-27: a stat no source reported is left out, and named once under the strip, rather than shown as a "—" tile
  const stats: [string, string | null][] = [
    ["Floor", e.floor ? floorText(e) : null], ["In dollars", usd], [bitcoin ? "Ordinals" : "NFTs", e.items === null ? null : countText(e.items)], ["Holders", e.owners === null ? null : countText(e.owners)],
    ["24h volume", e.volume24hEth === null ? null : volText(e.volume24hEth)], ["7d volume", e.volume7dEth === null ? null : volText(e.volume7dEth)],
    ["In Rook's vault", e.heldByRook === null ? (e.rook.kind === "live" ? null : "0") : countText(e.heldByRook)],
  ];
  const missing = stats.filter(([, v]) => v === null).map(([l]) => l.toLowerCase());
  return (
    <>
      {fixture ? <FixtureBanner note={src.note} /> : null}
      <header className="colhead rise">
        <Picture src={src.image(e)} seed={e.slug} name={e.name} size={112} round="18%" className="colhead__pic" />
        <div className="colhead__id">
          <p className="eyebrow" style={{ margin: 0 }}>{e.rook.kind === "live" ? "Live collection" : "Collection"} · {STANDARD_WORD[e.standard]}</p>
          <h1 style={{ margin: ".15rem 0 .35rem" }}>{e.name}</h1>
          <div className="row" style={{ gap: ".5rem .75rem" }}><ChainBadge e={e} /><StatusBadge e={e} />{e.rook.kind === "live" ? <a className="small" href={href("reserves", e.rook.slug)}>Check its reserves</a> : null}</div>
          {e.description ? <p className="muted" style={{ margin: ".6rem 0 0", maxWidth: "60ch" }}>{e.description}</p> : null}
          <p className="row small" style={{ margin: ".6rem 0 0" }}>{links.filter(([, u]) => !!u).map(([label, u]) => <a key={label} href={u!} target="_blank" rel="noreferrer noopener">{label} ↗</a>)}<span className="muted mono break">{e.id}</span></p>
        </div>
        <div className="colhead__act">
          {can ? <a className="btn btn--primary" href={launchTo}>Launch a coin</a> : <button type="button" className="btn btn--primary" disabled title={e.rook.why}>Launch a coin</button>}
          <p className="small muted" style={{ margin: ".5rem 0 0", maxWidth: "26ch" }}>{e.rook.kind === "live" ? "This collection has a curve factory: one transaction launches." : e.rook.kind === "listable" ? "Meets the rule; the launch waits for the listing." : e.rook.kind === "later" ? "Not launchable: above the stage's caps, or no floor." : "Not launchable: no vault can hold it."}</p>
        </div>
      </header>

      <div className="strip strip--stats rise rise-1">
        {stats.filter(([, v]) => v !== null).map(([label, v]) => <Fig key={label} label={label} value={v} big />)}
      </div>
      <Provenance className="prov--stats" line={<>{floorLine(e, now)}{missing.length ? <> Not available yet: {missing.join(", ")}.</> : null}</>}>
        {floorNote(e, now)}{e.floor?.ref ? <> Read as <span className="mono break">{e.floor.ref}</span>.</> : null} {e.listed !== null ? `${countText(e.listed)} listed for sale at the last read. ` : ""}A figure no source reported is left out, never estimated.
      </Provenance>

      <div className="grid rise rise-2">
        <div className="span-8">
          <section className="block">
            <div className="row" style={{ marginBottom: ".75rem" }}><div><p className="eyebrow">{bitcoin ? "Ordinals" : "NFTs"}</p><h2 style={{ margin: 0 }}>{e.sample.length ? `${e.sample.length} of ${countText(e.items)}` : "No sample"}</h2></div></div>
            {e.sample.length ? <ul className="items">{e.sample.map((it, i) => <li key={it.id}><Item it={it} i={i} e={e} src={src} /></li>)}</ul> : <p className="muted small">No source offered a sample of this collection's {nftWord(bitcoin)}.</p>}
            {needsKey(e) ? <p className="notice notice--warn small" style={{ marginTop: ".75rem" }}><strong>NFT pictures need the OpenSea key.</strong> {e.chain === "other" ? "No keyless source here reads this chain's NFTs; OpenSea's item list does, with a key." : "Keyless, OpenSea's item list answers 401, and this contract's own tokenURI gave no picture a public gateway serves; with the key the indexer reads OpenSea's."}</p> : null}
            <Provenance className="prov--block" line={`A sample of the collection's ${nftWord(bitcoin)}, with their public pictures; a mark stands in where there is none.`}>{fixture ? "Pictures are loaded from where the seed recorded them: the ordinals content gateway for inscriptions, the marketplace's CDN or the token's own metadata for NFTs — a mark where there is none." : "A sample of the collection's NFTs: from the marketplace's listing, or the contract's own tokenURI through a public RPC; pictures fetched and checked by the indexer, a mark where there is none."}</Provenance>
          </section>
          {e.rook.kind === "live" ? <LiveNest slug={e.rook.slug} /> : null}
        </div>
        <aside className="span-4 stack">
          <RookPanel e={e} rule={rule} />
          <section className="panel panel--tight">
            <p className="eyebrow">Sources</p>
            {e.sources.length ? <ul className="activity activity--tight">{e.sources.map((s, i) => <li key={i}><time title={when(s.at)}>{ago(s.at, now)}</time><span><span className={`kind${s.ok ? "" : " bad"}`}>{s.ok ? "answered" : "failed"}</span> · {SOURCE_WORD[s.name]}{s.status !== null ? ` · HTTP ${s.status}` : ""}</span></li>)}</ul> : <p className="muted small" style={{ margin: 0 }}>No source was asked about this collection.</p>}
          </section>
        </aside>
      </div>
    </>
  );
}
/** The floor's provenance in a line a person reads first ("Floor from the seed's record, 4 days old"); the record itself is behind the info. */
function floorLine(e: CatalogEntry, now: number): string {
  const f = e.floor;
  if (!f) return "No source has reported a floor yet.";
  const d = Math.max(0, now - f.at), age = d < 3_600 ? `${Math.max(1, Math.floor(d / 60))} min` : d < 86_400 ? `${Math.floor(d / 3_600)} h` : `${Math.floor(d / 86_400)} day${d >= 2 * 86_400 ? "s" : ""}`;
  return f.stale ? `Floor as last recorded, ${age} old: not the price now.` : `Floor read ${age} ago.`;
}
/** An NFT collection whose items no keyless source pictured: only OpenSea's item list (which needs the key) would. Bitcoin's come from the content gateway. */
const needsKey = (e: CatalogEntry) => e.chain !== "bitcoin" && !e.sample.some((i) => i.image);
function Item({ it, i, e, src }: { it: CatalogItem; i: number; e: CatalogEntry; src: CatalogSrc }) {
  const inner = <><Picture src={src.image(e, i)} seed={`${e.slug}/${it.id}`} name={it.name ?? it.id} size={120} round="14%" className="item__pic" noImage /><span className="item__name">{it.name ?? `#${it.id.slice(0, 8)}`}</span></>;
  return it.url ? <a className="item" href={it.url} target="_blank" rel="noreferrer noopener">{inner}</a> : <span className="item">{inner}</span>;
}

/** Where the collection stands with Rook, and — for one that is not listed — what listing it would take. Nothing here is a form: nothing is recorded. */
function RookPanel({ e, rule }: { e: CatalogEntry; rule: CatalogRule }) {
  const max = ethText(Number(BigInt(rule.maxFloorWei) / 10n ** 12n) / 1e6);
  if (e.rook.kind === "live") return (
    <div className="panel"><p className="eyebrow">On Rook</p><strong title={e.rook.why}>Live on Rook{e.rook.coins ? `, with ${e.rook.coins} coin${e.rook.coins === 1 ? "" : "s"} buying its floor` : ""}.</strong><p className="small muted" style={{ margin: ".35rem 0 0" }}>This collection's coins, vault, activity and redemptions are on this page; <a href={href("launch", e.rook.slug)}>launch a coin on it</a>.</p></div>);
  return (
    <div className="panel">
      <p className="eyebrow">Request listing</p>
      <p style={{ margin: 0 }}><StatusBadge e={e} /></p>
      <p className="small" style={{ margin: ".75rem 0 0" }}>{e.rook.why}</p>
      <p className="small muted" style={{ margin: ".75rem 0 0" }}><strong>The rule.</strong> {rule.text}</p>
      {e.rook.kind !== "unsupported" ? (
        <p className="small muted" style={{ margin: ".75rem 0 0" }}><strong>What listing takes, in order.</strong> (1) The collection is added to the vault's custody policy at the stage's caps by a timelocked proposal, announced 72 hours ahead (custody-hardening H2, launch-gates G7): at stage {rule.stage} that is {rule.perCollectionItems} NFTs and a floor of at most {max} ETH. (2) A wrapper is deployed for it and the keeper posts its floor from the floor feed. (3) This indexer's wrapped configuration names it, and it appears here as a live collection. None of that is a button.</p>
      ) : null}
      <p className="small muted" style={{ margin: ".75rem 0 0" }}><strong>This panel records nothing.</strong> There is no form and no submission: nothing you could type here would be sent anywhere, so nothing is asked. When there is a way to request a listing that a person will read, it will be here and say who reads it.</p>
    </div>
  );
}

/** A live collection's (a nest's) own reads: its coins, vault, activity and tickets, as the wrapped-collection pages report them. */
function LiveNest({ slug }: { slug: string }) {
  const now = useNow(), s = useWrappedSource(), q = useWrappedCollection(slug);
  if (s.isLoading || q.isLoading) return <Loading what="the vault" />;
  if (!s.data) return <p className="notice notice--warn small">This build has no reads for this collection's vault, so its coins and vault are not shown here.</p>;
  if (q.error || !q.data) return <p className="error small">The vault could not be read: {String((q.error as Error)?.message ?? "no answer").slice(0, 120)}</p>;
  const c = q.data, fixture = s.data.kind === "fixture", cb = c.kind === "inscriptions";
  const supply = c.supplyUnits ? BigInt(c.supplyUnits) : null;
  const pool = poolView(c);
  return (
    <>
      <section className="block">
        <div className="row" style={{ marginBottom: ".75rem" }}><div><p className="eyebrow">Coins on this collection</p><h2 style={{ margin: 0 }}>{c.coinsList.length ? `${c.coinsList.length} coin${c.coinsList.length === 1 ? " buys" : "s buy"} the floor` : "No coin yet"}</h2></div><a className="right btn btn--small" href={href("launch", c.slug)}>Launch a coin</a></div>
        <ul className="nests">
          {c.coinsList.map((coin) => (
            <li key={coin.address}><a className="nest" href={href("coin", coin.address)}>
              <span className="nest__name">{coin.name} <span className="muted">{coin.symbol}</span><small><Pill>{coin.policy}</Pill> · launched {ago(coin.launchedAt, now)}</small></span>
              <Fig label="Phase" value={<span style={{ fontFamily: "var(--body)", fontSize: "var(--t0)" }}><Pill tone={coin.phase === "trading" || coin.phase === "graduated" ? "live" : coin.phase === "refunding" ? "bad" : "warn"}>{WORD[coin.phase]}</Pill></span>} />
              <Fig label="Raise" value={eth(BigInt(coin.raiseWei), 1)} unit="ETH" />
              <Fig label="Raised" value={coin.raisedWei ? eth(BigInt(coin.raisedWei), 2) : "—"} unit={coin.raisedWei ? "ETH" : undefined} />
              <span />
            </a></li>
          ))}
        </ul>
      </section>
      <section className="block">
        <div className="row" style={{ marginBottom: ".75rem" }}><div><p className="eyebrow">The vault · <Wrapped symbol={c.symbol} bitcoin={cb} /></p><h2 style={{ margin: 0 }}>What the vault holds</h2></div><span className="right"><StatusPill status={c.status} /></span></div>
        <div className="figs">
          <Fig label="Keeper's floor" value={c.floorWei ? eth(BigInt(c.floorWei), 3) : <span className="na">no fresh floor yet</span>} unit={c.floorWei ? "ETH" : undefined} />
          <Fig label={cb ? "Ordinals held" : "NFTs held"} value={c.itemsHeld ?? <NotYet />} />
          <Fig label={`${c.symbol} in supply`} value={supply !== null ? eth(supply, 2) : <NotYet />} />
          <Fig label="Coins" value={c.coins} />
          {pool ? <Fig label={`Pool toward the next ${nftWord(cb, 1)}`} value={`${pool.units} / ${pool.dueAt}`} unit={c.symbol} /> : null}
          {pool?.target ? <Fig label="Graduation target" value={pool.target} /> : null}
        </div>
        {pool ? <p className="small muted" style={{ marginTop: ".75rem" }}><progress max={100} value={pool.pct} aria-label={`The pool toward the next ${nftWord(cb, 1)}`} style={{ width: "10rem", verticalAlign: "middle", marginRight: ".5rem" }} />{pool.text} <a href={href("docs", "floor-module")}>How pooling works</a>.</p> : null}
        <p className="small muted" style={{ marginTop: ".75rem" }}>{wrappedLine(c.symbol, cb)}</p>
        <p className="small muted" style={{ marginTop: ".5rem" }}>The vault's address: <span className="mono break">{c.custody.kind === "bitcoin" ? c.custody.address : c.custody.custodian}</span>{fixture ? " (invented; a fixture)" : c.custody.addressUrl ? <> · <a href={c.custody.addressUrl} target="_blank" rel="noreferrer noopener">on the explorer</a></> : null}. {c.floorFresh ? "The floor is the keeper's fresh cap." : "The keeper's cap is stale: no floor buy or purchase prices against it until a fresh one is posted (burns at the floor module follow the module's own state, shown on the reserves page)."} <a href={href("reserves", c.slug)}>Check its reserves</a> · <a href={href("redeem", c.slug)}>redeem one {c.symbol} for a random {nftWord(cb, 1)}</a> · <a href={href("rent", c.slug)}>lend an {cb ? "ordinal" : "NFT"} in</a>.</p>
      </section>
      <section className="block">
        <p className="eyebrow">Activity</p>
        <h2>What moved in and out</h2>
        {c.activity.length ? (
          <ul className="activity">{c.activity.slice(0, 30).map((a) => <li key={a.tx}><time title={a.at ? when(a.at) : ""}>{ago(a.at, now)}</time><span><span className={`kind kind--${a.kind}`}>{a.kind.replace("-", " ")}</span> · {a.text}</span></li>)}</ul>
        ) : <p className="muted small">Nothing yet.</p>}
        {c.openTickets.length ? <p className="small muted" style={{ marginTop: "1rem" }}>{c.openTickets.length} redemption ticket{c.openTickets.length === 1 ? "" : "s"} in flight: {c.openTickets.map((t) => <a key={t.id} href={href("redeem", c.slug, t.id)} style={{ marginRight: ".5rem" }}>#{t.id}</a>)}</p> : null}
      </section>
    </>
  );
}
