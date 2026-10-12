import { useMemo, useState } from "react";
import { href, useNow } from "../lib/env";
import { COLLECTIONS } from "../rook/sol/collections";
import { DEMO, SITE as SOL } from "../rook/sol/hooks";
import { useMarket } from "../rook/sol/hooksMarket";
import { age } from "../rook/sol/format";
import { changeText, compact, filterCoins, progressOf, raisedSol, type MarketCoin } from "../rook/sol/market";
import { raiseText } from "../rook/sol/graduation";
import { progressLabel } from "../rook/sol/firstBuy";
import { Loading, Pill } from "../ui/bits";
import { CoinPic, CollBadge, Spark } from "../ui/market";
import { DayOnePending } from "../ui/sol";

type Tab = "all" | "trading" | "graduating" | "graduated";
const TABS: [Tab, string][] = [["all", "All"], ["trading", "On the curve"], ["graduating", "Migrating"], ["graduated", "Graduated"]];
type Sort = "new" | "mcap" | "vol" | "change" | "progress";
const SORTS: [Sort, string][] = [["new", "Newest"], ["mcap", "Market cap"], ["vol", "Volume 24h"], ["change", "24h"], ["progress", "Progress"]];

/**
 * Every Rook coin as a table: from Rook's indexer when the build has one (with 24 h change and volume), else straight from Solana
 * (the DBC pools on Rook's configs, newest first), or the demo fixtures. The front page's terminal is the same data in columns.
 */
export function SolCoins({ slug, embedded = false }: { slug?: string; embedded?: boolean }) {
  const [nest, setNest] = useState(slug && (COLLECTIONS.some((c) => c.slug === slug) || slug === "rook") ? slug : "");
  const [tab, setTab] = useState<Tab>("all");
  const [sort, setSort] = useState<Sort>("new");
  const [q, setQ] = useState("");
  const m = useMarket();
  const now = useNow();
  const hist = m.coins.some((c) => c.volume24hSol !== null);
  const shown = useMemo(() => {
    const rows = filterCoins(m.coins, { collection: nest, q }).filter((c) => tab === "all" || c.phase === tab);
    const key: Record<Sort, (c: MarketCoin) => number> = { new: (c) => c.launchedAt ?? 0, mcap: (c) => c.marketCapSol ?? 0, vol: (c) => c.volume24hSol ?? 0, change: (c) => c.change24h ?? -Infinity, progress: (c) => progressOf(c) };
    return rows.slice().sort((a, b) => key[sort](b) - key[sort](a));
  }, [m.coins, nest, q, tab, sort]);
  if (!SOL.ready) return <>{embedded ? null : <header className="pagehead rise"><p className="eyebrow">Coins</p><h1>Every Rook coin</h1></header>}<DayOnePending what="The board" /></>;
  return (
    <>
      {embedded ? <h2 style={{ marginTop: 0 }}>Its coins</h2> : <header className="pagehead rise"><p className="eyebrow">Coins</p><h1>Every Rook coin</h1>
        <p className="lede">{DEMO ? "Demo fixtures: invented coins in every state, laid out as the board will read once coins launch." : m.source === "indexer" ? "Every coin on Meteora's bonding curve under one of Rook's configs, from Rook's indexer." : "Read straight from Solana in your browser: every coin on Meteora's bonding curve under one of Rook's configs, newest first."}</p></header>}
      <div className="board rise rise-1">
        <div className="board__controls">
          <div className="tabs" role="tablist" aria-label="Which coins">{TABS.map(([t, label]) => <button key={t} type="button" role="tab" aria-selected={tab === t} className={tab === t ? "on" : ""} onClick={() => setTab(t)}>{label}</button>)}</div>
          <label className="board__search"><span className="vh">Search coins</span><input type="search" placeholder="Search name, ticker, collection or mint" value={q} autoComplete="off" spellCheck={false} onChange={(e) => setQ(e.target.value)} /></label>
          <div className="board__sorts small"><span className="muted">Sort</span><span className="seg">{SORTS.filter(([s]) => hist || (s !== "vol" && s !== "change")).map(([s, label]) => <button key={s} type="button" aria-pressed={sort === s} onClick={() => setSort(s)}>{label}</button>)}</span>
            {embedded ? null : <label className="board__nest"><span className="vh">Collection</span><select value={nest} aria-label="Only coins on one collection" onChange={(e) => setNest(e.target.value)}>
              <option value="">All collections</option>{COLLECTIONS.map((c) => <option key={c.slug} value={c.slug}>{c.name}</option>)}<option value="rook">Genesis (ROOK)</option></select></label>}
          </div>
        </div>
        {m.loading ? <Loading what="the coins" /> : null}
        {m.error ? <p className="error small">The coins could not be read: {String((m.error as Error).message).slice(0, 160)}{/token|forbidden|403|not available|disabled/i.test(String((m.error as Error).message)) ? " This RPC endpoint does not serve getProgramAccounts to browsers; the board needs one that does (the site's operator sets a domain-restricted key)." : ""}</p> : null}
        <div className="scroll">
          <table className="board__table">
            <thead><tr><th>Coin</th><th className="col-nest">Collection</th><th className="r">Market cap</th>{hist ? <><th className="r">24h</th><th className="r col-vol">Volume 24h</th></> : null}<th>{nest ? progressLabel(nest) : "Progress to graduation"}</th><th className="r">Age</th>{hist ? <th className="col-spark"><span className="vh">Last day</span></th> : null}</tr></thead>
            <tbody>{shown.map((c) => <Row key={c.mint} c={c} usdPer={m.usdPerSol} now={now} hist={hist} />)}</tbody>
          </table>
        </div>
        {!shown.length && !m.loading ? <p className="muted small" style={{ padding: "1rem 0" }}>{q ? `Nothing matches “${q}”.` : m.total === 0 ? "No coin has launched here yet. Be the first." : "No coin in this view."}</p> : null}
        <p className="small muted board__count">{shown.length} shown · {m.coins.length} loaded{m.total !== null ? ` of ${m.total}` : ""}{m.more ? <> · <button type="button" className="btn btn--quiet btn--small" disabled={m.loadingMore} onClick={m.more}>{m.loadingMore ? "Loading…" : "Load 100 more"}</button></> : null}</p>
        <p className="small muted">A collection coin is priced in its collection's wrapped token (wMILADY, wAZUKI…); ROOK in SOL. {DEMO ? "Demo fixtures: nothing here was read from Solana; the dollar figures value each wrapped token at its collection's day-1 floor." : m.source === "indexer" ? "From Rook's indexer; prices are the last trade's, fee excluded." : `Prices are the curve's own (√P from the pool), in the coin's quote; a coin's page prices it in SOL and dollars through its collection's wX/SOL pool. Read through ${new URL(SOL.rpcUrl).host}; 24 h figures need Rook's indexer.`}</p>
      </div>
    </>
  );
}

function Row({ c, usdPer, now, hist }: { c: MarketCoin; usdPer: number | null; now: number; hist: boolean }) {
  const p = progressOf(c), raised = raisedSol(c);
  return (
    <tr className="board__row">
      <td><a className="board__coin" href={href("coin", c.mint)}><CoinPic c={c} size={32} /><span><strong>{c.symbol}</strong><small>{c.name}</small></span></a></td>
      <td className="small col-nest"><CollBadge slug={c.collection.slug} link /></td>
      <td className="r num">{c.marketCapUsd !== null ? compact(c.marketCapUsd, "$") : usdPer !== null && c.marketCapSol !== null && c.quote.symbol === "SOL" ? compact(c.marketCapSol * usdPer, "$") : "—"}<br /><small className="muted">{compact(c.marketCapSol, c.quote.symbol)}</small></td>
      {hist ? <><td className={`r num ${c.change24h === null ? "muted" : c.change24h >= 0 ? "good" : "bad"}`}>{changeText(c.change24h)}</td><td className="r num col-vol">{compact(c.volume24hSol, c.quote.symbol)}</td></> : null}
      <td>{c.phase === "graduated" ? <Pill tone="good">graduated</Pill> : <><span className="progress" title={raised !== null ? `${raised.toFixed(2)} of ${raiseText(c)}` : undefined}><i style={{ width: `${Math.round(p * 100)}%` }} /></span><small className="small muted">{c.phase === "graduating" ? "migrating" : `${(p * 100).toFixed(p < 0.1 ? 1 : 0)}%`}</small></>}</td>
      <td className="r num muted">{age(c.launchedAt ?? 0, now)}</td>
      {hist ? <td className="col-spark"><Spark values={c.spark} /></td> : null}
    </tr>
  );
}
