import { useState } from "react";
import type { WrappedCoin, WrappedCollection } from "../api/types";
import type { BoardRow, RookRes } from "../rook/apiTypes";
import { href, useEnv, useNow } from "../lib/env";
import { useWrappedCoin, useWrappedSource } from "../lib/wrapped";
import { eth, short, when } from "../lib/format";
import { pct } from "../lib/quote";
import { EXPLORER_ADDRESS, EXPLORER_TX, explorerFor } from "../lib/explorer";
import { POLICIES, launchExtraNow } from "../rook/tiers";
import { ageText, capText, changeText, feeNote, priceFull, priceText, progressOf, volText } from "../rook/boardlogic";
import { useBoardAll, useRookChart, useRookHolders, useRookTrades } from "../rook/data";
import { PHASE1 } from "../rook/phase";
import { Ticket } from "../ui/Ticket";
import { Raffles } from "../ui/Raffles";
import { CoinChart } from "../ui/Chart";
import { CoinFace } from "../ui/Identicon";
import { LaunchFeeLine, YoyoPanel } from "../ui/Books";
import { Fig, FixtureBanner, Loading, NothingDeployed, Pill, Wrapped, ago } from "../ui/bits";
import { Share } from "../ui/Share";
import { Star } from "../ui/Board";
import { useWatchlist } from "../rook/watch";

/**
 * The coin page as every launchpad has it (docs/v6/site-design.md §4, amended): the chart beside the ticket, the progress to
 * graduation up top, then Trades / Holders / Info / Activity. The board row is the page's spine; a Rook coin's ticket quotes
 * through the copied lib over the nest's market (the curve with its decaying launch fee, the pools, the Floor Module's own
 * quoteMint / quoteBurn inside its room); a pons coin is traded on Pons and the page says so. The one accent on this screen is
 * the ticket's cluster: the total cost, the Buy button and the launch-fee countdown under it.
 */
const PHASES: { id: BoardRow["phase"]; word: string; line: string }[] = [
  { id: "trading", word: "On the curve", line: "Buys and sells fill against the curve until the raise is met." },
  { id: "graduating", word: "Graduating", line: "The raise is met; its NFT budget is being converted. Trading is paused." },
  { id: "graduated", word: "Graduated", line: "Trading on Uniswap, through the router. Fees keep buying the floor." },
  { id: "refunding", word: "Refunding", line: "The raise was not met in time. Holders take back what is left on the curve." },
];
type Tab = "trades" | "holders" | "info" | "activity";
type Wrapped = { collection: WrappedCollection; coin: WrappedCoin } | null;
/** The collection's activity that belongs on a coin's page: what the vault did with its NFTs. */
const SETTLED = new Set<WrappedCollection["activity"][number]["kind"]>(["raffle-won", "burned", "arrived", "relisted", "sold", "bought-back", "returned"]);

export function CoinPage({ address }: { address?: string }) {
  const env = useEnv(), board = useBoardAll();
  const found = useWrappedCoin(address), wsrc = useWrappedSource();
  if (env === null || board.sourceLoading || board.isLoading) return <Loading what="the coin" />;
  if (!board.source) return <NothingDeployed page={`coin/${address ?? ""}`} />;
  const row = board.data?.rows.find((r) => r.address.toLowerCase() === (address ?? "").toLowerCase()) ?? null;
  if (!row) return <div className="pagehead"><h1>No such coin</h1><p className="lede">Nothing on the board is at <span className="mono break">{address}</span>. <a href={href("coins")}>All coins</a>.</p></div>;
  return <>{board.source.kind === "fixture" ? <FixtureBanner note={board.source.note} /> : null}<Coin row={row} usd={board.data?.usdPerEth ?? null} wrapped={found.data ?? null} wsrc={wsrc.data ?? null} /></>;
}

function Coin({ row, usd, wrapped, wsrc }: { row: BoardRow; usd: number | null; wrapped: Wrapped; wsrc: ReturnType<typeof useWrappedSource>["data"] | null }) {
  const env = useEnv(), now = useNow();
  const [res, setRes] = useState<RookRes>("5m");
  const [tab, setTab] = useState<Tab>("trades");
  const chart = useRookChart(row.kind, row.address, res);
  const phase = PHASES.find((p) => p.id === row.phase)!;
  const policy = POLICIES.find((p) => p.id === row.policy) ?? null;
  const p = progressOf(row);
  const explorer = explorerFor(env && "chain" in env ? env.chain.id : null, env?.mode === "sample");
  const watch = useWatchlist();
  const fixture = wsrc?.kind === "fixture";
  const lf = wrapped?.coin.launchFee ?? null;
  const extra = lf ? launchExtraNow(lf, now) : 0;
  const left = lf && lf.endsAt !== null ? Math.max(0, lf.endsAt - now) : null;
  return (
    <>
      <header className="coinhead rise">
        <div className="coinhead__id">
          <CoinFace address={row.address} logoUrl={row.logoUrl} name={row.name} size={56} />
          <div>
            <p className="eyebrow">{row.nest.slug ? <a href={href("collection", row.nest.slug)}>{row.nest.name}</a> : row.nest.name ?? "no collection"} · {row.kind === "curve" ? "Rook coin" : "launched on Pons"}</p>
            <h1>{row.name} <span className="muted" style={{ fontWeight: 500 }}>{row.symbol}</span></h1>
          </div>
          <div className="coinhead__acts"><Star on={watch.has(row.address)} toggle={() => watch.toggle(row.address)} symbol={row.symbol} /><Share address={row.address} symbol={row.symbol} /></div>
        </div>
        <div className="coinhead__meta">
          <Pill tone={row.phase === "trading" || row.phase === "graduated" ? "good" : row.phase === "refunding" ? "bad" : "warn"}>{phase.word}</Pill>
          {policy ? <span>Policy: <Pill>{policy.title}</Pill></span> : <span>Policy: <Pill>vault</Pill></span>}
          {row.feeBps !== null ? <span title={feeNote(row)}>Fee: <strong className="num">{pct(row.feeBps, row.feeBps % 100 ? 2 : 0)}</strong></span> : null}
          <span>Launched <span className="num">{ageText(row.launchedAt, now)}</span> ago</span>
          <span className="mono break small">{explorer ? <a href={EXPLORER_ADDRESS(explorer, row.address)} target="_blank" rel="noreferrer noopener">{row.address}</a> : row.address}</span>
        </div>
      </header>

      <div className="figs coinstats rise rise-1">
        <Fig label="Market cap" value={capText(row.marketCapEth, usd)} />
        <Fig label="Price" value={<span title={priceFull(row.price) ? `${priceFull(row.price)} ETH` : undefined}>{priceText(row.price)}</span>} unit={row.price !== null ? "ETH" : undefined} />
        <Fig label="24h" value={<span className={row.change24h === null ? "" : row.change24h >= 0 ? "good" : "bad"}>{changeText(row.change24h)}</span>} />
        <Fig label="Volume 24h" value={volText(row.volume24hWei)} />
        {row.holders !== null ? <Fig label="Holders" value={row.holders} /> : null}
        {row.trades24h !== null ? <Fig label="Trades 24h" value={row.trades24h} /> : null}
      </div>
      <div className="gradline rise rise-1">
        {row.phase === "graduated" ? <p className="small"><strong>Graduated</strong> {row.graduatedAt ? ago(row.graduatedAt, now) : ""} — trading on Uniswap{wrapped?.collection.contracts.lock && explorer ? <>, its pool locked in <a href={EXPLORER_ADDRESS(explorer, wrapped.collection.contracts.lock)} target="_blank" rel="noreferrer noopener">the LiquidityLock</a></> : ""}.</p>
          : row.raise ? <><span className="progress progress--big"><i style={{ width: `${Math.round((p ?? 0) * 100)}%` }} /></span><p className="small"><strong className="num">{Math.round((p ?? 0) * 100)}%</strong> of the way: <span className="num">{eth(BigInt(row.raise.raisedWei), 3)}</span> of <span className="num">{eth(BigInt(row.raise.raiseWei), 1)}</span> ETH raised. Graduates at {eth(BigInt(row.raise.raiseWei), 1)} ETH, when the raise buys its first NFTs and its pool opens.</p></>
          : <p className="small muted">{phase.line}</p>}
      </div>

      <div className="grid rise rise-2">
        <div className="span-8"><CoinChart set={chart.data} res={res} onRes={setRes} symbol={row.symbol} loading={chart.isLoading} error={chart.error} /></div>
        <div className="span-4">
          {row.kind === "curve" && wrapped && wsrc ? <Ticket src={wsrc} col={wrapped.collection} coin={wrapped.coin} feeTierBps={row.feeBps} pageCoin={row.address} />
            : row.kind === "curve" ? <div className="panel"><p className="small muted">The ticket needs the collection's market to quote; it is being read.</p></div>
            : PHASE1 ? <div className="panel ticket"><p className="eyebrow">Not a Rook coin</p><p>This coin was launched through the older Launcher, not a Rook curve. In Phase 1 its creator fees go into the {row.nest.name ?? "collection's"} vault and nothing takes them out, so this site does not offer it or send anyone to trade it.</p><p className="small muted" style={{ margin: 0 }}>The chart, tape and holders below are read from the chain as they are. <a href={href("trust")}>What Phase 1 switches on</a>.</p></div>
            : <div className="panel ticket"><p className="eyebrow">Trade</p><p>This coin was launched through the Launcher and trades on Pons's own curve and pool; its creator fees pay into the {row.nest.name ?? "vault"} vault.</p><a className="btn btn--primary btn--block" href="https://pons.fun/" target="_blank" rel="noreferrer noopener">Trade on Pons</a><p className="small muted" style={{ marginTop: ".75rem" }}>Nothing here can send a Pons trade; the chart, tape and holders are read from the same chain.</p></div>}
          {lf && (extra > 0 || (left !== null && left > 0)) ? (
            <div className="panel panel--tight decay" style={{ marginTop: "1rem" }} aria-live="polite">
              <div className="row"><span className="eyebrow" style={{ margin: 0 }}>Launch fee</span><span className="right num decay__now">+{pct(extra, extra % 100 ? 2 : 0)}{left !== null ? ` · ${Math.floor(left / 60)}:${String(left % 60).padStart(2, "0")}` : ""}</span></div>
              <div className="decay__bar"><i style={{ width: `${left !== null && lf.decayS ? Math.min(100, (left / lf.decayS) * 100) : 0}%` }} /></div>
              <p className="small muted" style={{ margin: 0 }}>For the first {Math.round(lf.decayS / 60)} minutes a buy pays an extra fee falling from {pct(lf.bps, 0)} to nothing; all of it goes to the coin's vault. Counted in the ticket's total.</p>
            </div>) : null}
        </div>
      </div>

      {PHASE1 && row.policy === "yoyo" ? (
        <section className="block rise rise-3">
          <p className="notice notice--warn small" role="status"><strong>YOYO is off in Phase 1.</strong> Its creator chose YOYO on chain, but YOYO's floor buys and relistings are not switched on: the share of trading fees meant for buying the floor waits in its vault until they are. Trading the coin, and its graduation, work as for any coin. <a href={href("trust")}>What Phase 1 switches on</a>.</p>
        </section>) : row.policy === "yoyo" && wrapped?.coin.yoyo ? (
        <section className="block rise rise-3">
          <p className="eyebrow">YOYO</p>
          <h2>Relisted, sold, bought back, burnt</h2>
          <YoyoPanel coin={wrapped.coin} wx={wrapped.collection.symbol} now={now} fixture={fixture} />
        </section>) : null}

      {row.policy === "raffle" && wrapped ? (
        <section className="block rise rise-3" aria-labelledby="raffles-head">
          <p className="eyebrow">Raffles</p>
          <h2 id="raffles-head">Each NFT the fees buy goes to one holder</h2>
          <p className="small muted">Each NFT is one whole {wrapped.collection.symbol ? <Wrapped symbol={wrapped.collection.symbol} bitcoin={wrapped.collection.kind === "inscriptions"} /> : "wrapped token"}, and each draw gives one to one holder. The keeper announces a draw and publishes the ticket list; a drand round picks the snapshot time and another picks the winner. Everything after the list is anyone's to do, here or with any wallet: pin a round, resolve it, claim for the winner, void a draw nobody finished.</p>
          <Raffles col={wrapped.collection} coin={row.address} fixture={fixture} />
        </section>) : null}

      <section className="block rise rise-3">
        <div className="tabs" role="tablist" aria-label="Coin details">{(["trades", "holders", "info", "activity"] as Tab[]).map((t) => <button key={t} type="button" role="tab" aria-selected={tab === t} className={tab === t ? "on" : ""} onClick={() => setTab(t)}>{t[0].toUpperCase() + t.slice(1)}</button>)}</div>
        {tab === "trades" ? <Trades row={row} usd={usd} explorer={explorer} /> : tab === "holders" ? <Holders row={row} explorer={explorer} /> : tab === "info" ? <Info row={row} wrapped={wrapped} explorer={explorer} /> : <ActivityTab row={row} wrapped={wrapped} />}
      </section>
    </>
  );
}

const coins = (wei: string) => { const n = Number(BigInt(wei)) / 1e18; return n >= 1e9 ? `${(n / 1e9).toFixed(2)}B` : n >= 1e6 ? `${(n / 1e6).toFixed(2)}M` : n >= 1e3 ? `${(n / 1e3).toFixed(1)}k` : n.toFixed(2); };

function Trades({ row, usd, explorer }: { row: BoardRow; usd: number | null; explorer: string | null }) {
  const q = useRookTrades(row.kind, row.address), now = useNow();
  const items = q.data?.pages.flatMap((p) => p.items) ?? [];
  if (q.isLoading) return <Loading what="the tape" />;
  if (q.error) return <p className="small warn">The tape is not known here: {String((q.error as Error).message).slice(0, 120)}</p>;
  if (!items.length) return <p className="small muted">This coin has never traded. The first buy appears here with the transaction that made it.</p>;
  return (
    <>
      <div className="scroll"><table className="tape">
        <thead><tr><th>Time</th><th>Side</th><th className="r">Price</th><th className="r">Size</th><th className="r">ETH</th><th className="r">USD</th><th>Trader</th><th>Tx</th></tr></thead>
        <tbody>{items.map((t) => {
          const ethV = Number(BigInt(t.quoteWei)) / 1e18;
          return <tr key={t.id}><td className="num muted" title={t.at ? when(t.at) : ""}>{ago(t.at, now)}</td><td><span className={t.side === "buy" ? "good" : "bad"}>{t.side}</span> <span className="muted small">{t.venue}</span></td><td className="r num" title={priceFull(t.price) ? `${priceFull(t.price)} ETH` : undefined}>{priceText(t.price)}</td><td className="r num">{coins(t.coinWei)} {row.symbol}</td><td className="r num">{ethV.toFixed(4)}</td><td className="r num muted">{usd !== null ? `$${(ethV * usd).toFixed(2)}` : "—"}</td><td className="num">{explorer ? <a href={EXPLORER_ADDRESS(explorer, t.trader)} target="_blank" rel="noreferrer noopener">{short(t.trader)}</a> : short(t.trader)}</td><td className="num">{explorer ? <a href={EXPLORER_TX(explorer, t.tx)} target="_blank" rel="noreferrer noopener">{short(t.tx)}</a> : <span className="muted">{short(t.tx)}</span>}</td></tr>;
        })}</tbody>
      </table></div>
      <p className="small muted" style={{ marginTop: ".75rem" }}>{q.data?.pages[0]?.total ?? items.length} trades over the coin's life{explorer ? "" : " · a fixture's transactions lead nowhere, so they are not linked"}. {q.hasNextPage ? <button type="button" className="btn btn--quiet btn--small" onClick={() => void q.fetchNextPage()} disabled={q.isFetchingNextPage}>{q.isFetchingNextPage ? "Reading…" : "Older trades"}</button> : null}</p>
    </>
  );
}

function Holders({ row, explorer }: { row: BoardRow; explorer: string | null }) {
  const q = useRookHolders(row.kind, row.address, true);
  const items = q.data?.pages.flatMap((p) => p.items) ?? [];
  const label = (a: string, l: string | null) => l ?? (a.toLowerCase() === row.creator.toLowerCase() ? "launcher" : null);
  if (q.isLoading) return <Loading what="the holders" />;
  if (q.error) return <p className="small warn">The holders are not known here: {String((q.error as Error).message).slice(0, 120)}</p>;
  if (!items.length) return <p className="small muted">Nobody holds this coin yet.</p>;
  return (
    <>
      <p className="small muted">One address, one row: nothing here groups a trader's several wallets. {q.data?.pages[0]?.complete === false ? "The history before indexing began is still being read; the count is not complete." : ""}</p>
      <div className="scroll"><table className="tape">
        <thead><tr><th className="r">#</th><th>Address</th><th className="r">Share</th><th style={{ width: "30%" }}></th><th className="r">Balance</th></tr></thead>
        <tbody>{items.map((h) => { const l = label(h.address, h.label); return <tr key={h.address}><td className="r num muted">{h.rank}</td><td className="num">{explorer ? <a href={EXPLORER_ADDRESS(explorer, h.address)} target="_blank" rel="noreferrer noopener">{short(h.address)}</a> : short(h.address)}{l ? <> <Pill>{l}</Pill></> : null}</td><td className="r num">{h.share === null ? "—" : `${(h.share * 100).toFixed(2)}%`}</td><td><span className="progress"><i style={{ width: `${Math.min(100, (h.share ?? 0) * 100 * 2)}%` }} /></span></td><td className="r num">{coins(h.balanceWei)}</td></tr>; })}</tbody>
      </table></div>
      <p className="small muted" style={{ marginTop: ".75rem" }}>{q.data?.pages[0]?.holders ?? items.length} holders{row.kind === "curve" ? "; the curve, the lock, the router and the pool are left out" : "; the curve, the router, Pons's factory and the pool are left out"}. {q.hasNextPage ? <button type="button" className="btn btn--quiet btn--small" onClick={() => void q.fetchNextPage()}>More holders</button> : null}</p>
    </>
  );
}

function Info({ row, wrapped, explorer }: { row: BoardRow; wrapped: Wrapped; explorer: string | null }) {
  const now = useNow();
  const policy = POLICIES.find((p) => p.id === row.policy) ?? null;
  const link = (a: string | null | undefined, what: string) => a ? <><dt>{what}</dt><dd className="break">{explorer ? <a href={EXPLORER_ADDRESS(explorer, a)} target="_blank" rel="noreferrer noopener">{a}</a> : a}</dd></> : null;
  const c = wrapped?.collection.contracts;
  return (
    <div className="grid">
      <dl className="kv kv--left span-6">
        <dt>Collection</dt><dd>{row.nest.slug ? <a href={href("collection", row.nest.slug)}>{row.nest.name}</a> : row.nest.name ?? "—"}{row.nest.symbol ? ` (${row.nest.symbol})` : ""}</dd>
        <dt>Policy</dt><dd>{policy ? `${policy.title}: ${policy.line}` : "Its vault buys the listed asset; each acquisition's delivery is chosen by the creator."}</dd>
        <dt>Fee tier</dt><dd>{row.feeBps !== null ? `${pct(row.feeBps, row.feeBps % 100 ? 2 : 0)} of every trade${row.kind === "curve" ? ", fixed at launch; 40% of it buys the floor into the vault" : " (Pons's base plus the creator tax); 80% of what reaches the router goes to the vault"}` : "—"}</dd>
        {wrapped ? <><dt>Launch fee</dt><dd><LaunchFeeLine coin={wrapped.coin} now={now} /></dd></> : null}
        <dt>Launched</dt><dd>{row.launchedAt ? `${when(row.launchedAt)} (${ageText(row.launchedAt, now)} ago)` : "—"}</dd>
        <dt>Supply</dt><dd>{row.supplyWei ? `${coins(row.supplyWei)} ${row.symbol}` : "not read yet"}</dd>
        <dt>Spent buying the floor</dt><dd>{row.sweptWei ? `${eth(BigInt(row.sweptWei), 4)} ETH` : "—"}</dd>
      </dl>
      <dl className="kv kv--left span-6 small">
        {link(row.address, "Coin")}{link(row.curve, "Curve")}{link(row.creator, "Creator")}
        {wrapped && c ? <>{link(wrapped.collection.wrapper, "Wrapped token")}{link(c.factory, "Factory")}{link(c.router, "Router")}{link(c.lock, "Liquidity lock")}{link(c.floorModule, "Floor module")}
          {link(row.policy === "yoyo" ? c.yoyoVault : row.policy === "burn" ? c.burnVault : c.raffleVault, `${policy?.title ?? "Floor-buy"} vault`)}{link(c.offerBoard, "Offer board")}{link(c.treasury, "Treasury")}</> : null}
        <dt>Read</dt><dd><a href={href("docs", row.kind === "curve" ? "policies-and-fees" : "trust-model")}>{row.kind === "curve" ? "The fee and policy design" : "The trust model"}</a></dd>
      </dl>
    </div>
  );
}

function ActivityTab({ row, wrapped }: { row: BoardRow; wrapped: Wrapped }) {
  const now = useNow();
  const settled = wrapped?.collection.activity.filter((a) => SETTLED.has(a.kind)) ?? [];
  if (!wrapped) return <p className="small muted">{row.kind === "pons" ? "A pons coin's purchases, draws and deliveries live on its vault; the activity here covers Rook's collections." : "No vault activity to show."}</p>;
  if (!settled.length) return <p className="small muted">No NFT has been raffled, burnt or relisted from this collection's vault yet.</p>;
  return <ul className="activity">{settled.slice(0, 30).map((a) => <li key={a.tx}><time title={a.at ? when(a.at) : ""}>{ago(a.at, now)}</time><span><span className={`kind kind--${a.kind}`}>{a.kind.replace("-", " ")}</span> · {a.text}</span></li>)}</ul>;
}
