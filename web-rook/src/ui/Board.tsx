import { useMemo, useState } from "react";
import type { Activity, BoardRow } from "../rook/apiTypes";
import { HIGH_FEE_BPS, SORTS, TABS, ageText, capText, changeText, defaultSort, feeNote, king, progressOf, searchRows, sortRows, tabRows, volText, type Sort, type Tab } from "../rook/boardlogic";
import { href, useNow } from "../lib/env";
import { eth } from "../lib/format";
import { CoinFace } from "./Identicon";
import { Pill, ago } from "./bits";
import { useWatchlist } from "../rook/watch";
import { useCatalog } from "../rook/catalogData";
import { STATUS_WORD, wizardOrder } from "../rook/cataloglogic";

/** The board (docs/v6/site-design.md §4, amended): tabs, sorts, search, dense rows; the top coin (the "king of the hoard"); the ticker. */
const PHASE: Record<BoardRow["phase"], string> = { trading: "curve", graduating: "graduating", graduated: "graduated", refunding: "refunding" };

export function Board({ rows, usdPerEth, initialTab = "trending", compact = false }: { rows: BoardRow[]; usdPerEth: number | null; initialTab?: Tab; compact?: boolean }) {
  const now = useNow();
  const [tab, setTab] = useState<Tab>(initialTab);
  const [sort, setSort] = useState<Sort>(defaultSort(initialTab));
  const [q, setQ] = useState("");
  const [take, setTake] = useState(compact ? 12 : 40);
  const [nest, setNest] = useState("");
  const watch = useWatchlist(), cat = useCatalog();
  // docs/v6/collections-catalog.md §4: the collection filter is the catalogue — only a live nest can be picked; the rest are listed greyed with their status
  const nests = useMemo(() => wizardOrder(cat.data?.entries ?? []), [cat.data]);
  const nested = useMemo(() => (nest ? rows.filter((r) => (r.nest.slug ?? "").toLowerCase() === nest.toLowerCase()) : rows), [rows, nest]);
  const shown = useMemo(() => sortRows(searchRows(tabRows(nested, tab, now, watch.list), q), sort), [nested, tab, q, sort, now, watch.list]);
  const pick = (t: Tab) => { setTab(t); setSort(defaultSort(t)); setTake(compact ? 12 : 40); };
  return (
    <div className="board">
      <div className="board__controls">
        <div className="tabs" role="tablist" aria-label="Which coins">{TABS.map(([t, label]) => <button key={t} type="button" role="tab" aria-selected={tab === t} className={tab === t ? "on" : ""} onClick={() => pick(t)}>{label}</button>)}</div>
        <label className="board__search"><span className="vh">Search coins</span><input type="search" placeholder="Search name, ticker, collection or address" value={q} autoComplete="off" spellCheck={false} onChange={(e) => setQ(e.target.value)} /></label>
        <span className="board__sorts"><span className="small muted">Sort</span><span className="seg">{SORTS.map(([s, label]) => <button key={s} type="button" aria-pressed={sort === s} onClick={() => setSort(s)}>{label}</button>)}</span></span>
        {nests.length ? <label className="board__nest small"><span className="muted">Collection</span><select value={nest} aria-label="Only coins on one collection" onChange={(e) => setNest(e.target.value)}>
          <option value="">Every collection</option>
          {nests.map((e) => <option key={e.slug} value={e.rook.kind === "live" ? e.rook.slug : e.slug} disabled={e.rook.kind !== "live"}>{e.name}{e.rook.kind === "live" ? "" : ` — ${STATUS_WORD[e.rook.kind].toLowerCase()}`}</option>)}
        </select></label> : null}
      </div>
      <div className="scroll">
        <table className="board__table">
          <thead><tr><th aria-label="Watch"></th><th>Coin</th><th className="col-nest">Collection</th><th className="r">Market cap</th><th className="r">24h</th><th className="r">Volume 24h</th><th className="r">Holders</th><th>Progress</th><th>Policy</th><th className="r">Fee</th><th className="r">Age</th></tr></thead>
          <tbody>{shown.slice(0, take).map((r) => <Row key={r.address} r={r} usd={usdPerEth} now={now} starred={watch.has(r.address)} star={() => watch.toggle(r.address)} />)}</tbody>
        </table>
      </div>
      {!shown.length ? <p className="muted small" style={{ padding: "1rem 0" }}>{q ? `Nothing matches “${q}”.` : nest ? "No coin on this collection yet." : tab === "new" ? "Nothing launched in the last day." : tab === "watch" ? "Star a coin to watch it here. Stars stay in this browser and go nowhere else." : "No coin here yet."}</p> : null}
      <p className="small muted board__count">{Math.min(take, shown.length)} of {shown.length} coins{shown.length > take ? <> · <button type="button" className="btn btn--quiet btn--small" onClick={() => setTake(take + 40)}>More</button></> : null}{rows.length !== shown.length ? ` · ${rows.length} on the board` : ""}</p>
    </div>
  );
}

function Row({ r, usd, now, starred, star }: { r: BoardRow; usd: number | null; now: number; starred: boolean; star: () => void }) {
  const p = progressOf(r);
  return (
    <tr className="board__row">
      <td><Star on={starred} toggle={star} symbol={r.symbol} /></td>
      <td><a className="board__coin" href={href("coin", r.address)}><CoinFace address={r.address} logoUrl={r.logoUrl} name={r.name} size={32} /><span><strong>{r.symbol}</strong><small>{r.name}{r.kind === "pons" ? " · on Pons" : ""}</small></span></a></td>
      <td className="small col-nest">{r.nest.slug ? <a href={href("collection", r.nest.slug)}>{r.nest.name}</a> : <span className="muted">{r.nest.name ?? "—"}</span>}</td>
      <td className="r num">{capText(r.marketCapEth, usd)}</td>
      <td className={`r num ${r.change24h === null ? "muted" : r.change24h >= 0 ? "good" : "bad"}`}>{changeText(r.change24h)}</td>
      <td className="r num">{volText(r.volume24hWei)}</td>
      <td className="r num">{r.holders ?? "—"}</td>
      <td><span className="progress" title={r.phase === "graduated" ? `graduated ${ago(r.graduatedAt, now)}` : r.raise ? `${eth(BigInt(r.raise.raisedWei), 2)} of ${eth(BigInt(r.raise.raiseWei), 1)} ETH` : PHASE[r.phase]}><i style={{ width: `${Math.round((p ?? 0) * 100)}%` }} className={r.phase === "graduated" ? "done" : ""} /></span><small className="small muted">{r.phase === "graduated" ? "graduated" : p !== null ? `${Math.round(p * 100)}%` : PHASE[r.phase]}</small></td>
      <td>{r.policy ? <Pill tone={r.policy === "yoyo" ? "live" : undefined}>{r.policy}</Pill> : <span title="Settles through the collection's vault: each purchase's delivery is chosen by the creator"><Pill>vault</Pill></span>}</td>
      <td className="r num">{r.feeBps === null ? "—" : r.feeBps > HIGH_FEE_BPS
        ? <span className="fee fee--high" title={feeNote(r)} tabIndex={0} aria-label={`Fee ${(r.feeBps / 100).toFixed(2)}%. ${feeNote(r)}`}>{`${(r.feeBps / 100).toFixed(r.feeBps % 100 ? 2 : 0)}%`}<span className="fee__i" aria-hidden="true">i</span></span>
        : <span title={feeNote(r)}>{`${(r.feeBps / 100).toFixed(r.feeBps % 100 ? 2 : 0)}%`}</span>}</td>
      <td className="r num muted">{ageText(r.launchedAt, now)}</td>
    </tr>
  );
}

export function Star({ on, toggle, symbol }: { on: boolean; toggle: () => void; symbol: string }) {
  return <button type="button" className={`star${on ? " star--on" : ""}`} aria-pressed={on} aria-label={on ? `Stop watching ${symbol}` : `Watch ${symbol}`} title={on ? "Watching" : "Watch"} onClick={toggle}>{on ? "★" : "☆"}</button>;
}

export function King({ rows, usdPerEth }: { rows: BoardRow[]; usdPerEth: number | null }) {
  const now = useNow();
  const k = king(rows);
  if (!k) return null;
  return (
    <a className="king panel" href={href("coin", k.address)}>
      <p className="eyebrow" style={{ margin: 0 }}>Top coin today · highest trending score</p>
      <div className="king__body">
        <CoinFace address={k.address} logoUrl={k.logoUrl} name={k.name} size={56} />
        <div className="king__name"><strong>{k.symbol}</strong><span className="muted">{k.name} · {k.nest.name}</span></div>
        <dl className="king__figs">
          <div><dt>Market cap</dt><dd className="num">{capText(k.marketCapEth, usdPerEth)}</dd></div>
          <div><dt>24h</dt><dd className={`num ${k.change24h === null ? "" : k.change24h >= 0 ? "good" : "bad"}`}>{changeText(k.change24h)}</dd></div>
          <div><dt>Volume</dt><dd className="num">{volText(k.volume24hWei)}</dd></div>
          <div><dt>Holders</dt><dd className="num">{k.holders ?? "—"}</dd></div>
          <div><dt>Launched</dt><dd className="num">{ageText(k.launchedAt, now)} ago</dd></div>
        </dl>
      </div>
    </a>
  );
}

const KIND_WORD: Record<Activity["items"][number]["kind"], string> = { buy: "buy", sell: "sell", launch: "launch", graduated: "graduated", "raffle-won": "raffle", burned: "burn", relisted: "relist", arrived: "into the vault" };
export function Ticker({ items, fixture }: { items: Activity["items"]; fixture: boolean }) {
  const now = useNow();
  if (!items.length) return null;
  return (
    <div className="ticker" aria-label="Live activity">
      <span className="ticker__label"><span className="live-dot" aria-hidden="true" />{fixture ? "Fixture tape" : "Live"}</span>
      <div className="ticker__rail"><ul className="ticker__track">{items.slice(0, 40).map((a, i) => (
        <li key={`${a.tx ?? "t"}-${i}`} className={`ticker__item ticker__item--${a.kind}`}>
          <span className="ticker__kind">{KIND_WORD[a.kind]}</span>
          {a.coin ? <a href={href("coin", a.coin.address)}>{a.coin.symbol}</a> : <span>{a.nest}</span>}
          <span className="muted">{a.amountWei ? `${eth(BigInt(a.amountWei), 3)} ETH` : a.kind === "launch" || a.kind === "graduated" ? a.nest ?? "" : a.text.slice(0, 40)}</span>
          <span className="muted small">{ago(a.at, now)}</span>
        </li>))}</ul></div>
    </div>
  );
}
