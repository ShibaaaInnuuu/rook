import { useMemo, useState } from "react";
import { href, useNow } from "../lib/env";
import { COLLECTIONS, PINNED_COLLECTIONS } from "../rook/sol/collections";
import { HOW_IT_WORKS } from "../rook/routes";
import { DEMO, SITE as SOL, useRookRow } from "../rook/sol/hooks";
import { GenesisCard } from "../ui/GenesisCard";
import { useActivity, useMarket, type FeedItem, type Market } from "../rook/sol/hooksMarket";
import { changeText, coinFromRow, columnsOf, compact, filterCoins, marketStats, progressOf, topByMcapOf, trendingOf, type MarketCoin } from "../rook/sol/market";
import { quoteOf } from "../rook/sol/graduation";
import { CoinCard, CoinPic, FeedRow, Spark, useTween } from "../ui/market";
import { RookHead } from "../ui/Mark";
import { RafflesSection } from "../ui/RafflePanel";
import { CollectionLogo } from "../ui/sol";

/**
 * The front page is the market: a short hero with the live figures and the trade stream, the trending strip, then the terminal
 * (New / About to graduate / Graduated / Live trades, filterable by collection and search; tabs on a phone). Under it, the raffles
 * and what Rook is. Data: src/rook/sol/hooksMarket.ts (demo fixtures, the indexer, or the RPC board without history).
 */
export function SolHome() {
  const m = useMarket();
  const feed = useActivity(m.coins);
  const now = useNow();
  return (
    <>
      <Hero m={m} feed={feed.items} now={now} />
      <Trending m={m} />
      <Terminal m={m} feed={feed.items} now={now} />
      <RafflesSection />
      <About />
    </>
  );
}

const NEED_INDEXER = "Live trades need Rook's indexer; this build reads the curves straight from Solana.";

function Hero({ m, feed, now }: { m: Market; feed: FeedItem[]; now: number }) {
  const st = useMemo(() => marketStats(m.coins), [m.coins]);
  const live = feed.filter((a) => a.kind === "buy" || a.kind === "sell");
  const vol = useTween(st.volume24hSol), coins = useTween(m.source === "none" ? null : st.coins), backing = useTween(m.source === "none" ? null : st.backingSolAtFloors);
  return (
    <section className="hero3" aria-labelledby="hero-h">
      <div className="hero3__say">
        <p className="hero3__live"><span className="dot" aria-hidden="true" />{DEMO ? "Demo market · invented coins" : !SOL.ready ? "Opens on day 1 · Solana" : "Live on Solana"}</p>
        <h1 className="hero3__h1" id="hero-h"><RookHead size={64} className="hero3__mark" />Memecoins backed by real&nbsp;NFTs.</h1>
        <p className="hero3__lede">Launch a coin on one of {PINNED_COLLECTIONS.length} blue-chip collections on Ethereum, Solana and Bitcoin, more on Robinhood Chain coming. It trades on a Solana bonding curve, priced in that collection's wrapped NFT token, until the curve holds the collection's threshold (a few whole tokens, worth about 85 SOL at the day-1 floor and more for the priciest collections); then it graduates, and half the raise goes straight into the coin's vault: wrapped NFTs held for its holders, more than one NFT's worth from the first graduation.</p>
        <p className="hero3__acts"><a className="btn btn--primary" href={href("launch")}>Launch a coin</a><a className="btn" href="#terminal" onClick={(e) => { e.preventDefault(); document.getElementById("terminal")?.scrollIntoView({ behavior: "smooth", block: "start" }); }}>Explore coins</a><a className="btn btn--quiet" href={href(HOW_IT_WORKS)}>How it works</a></p>
      </div>
      <div className="hero3__board" aria-label="The market now">
        <RookHead size={260} className="hero3__ghost" />
        <dl className="hero3__figs">
          <div><dt>Coins launched</dt><dd className="num">{coins === null ? "—" : Math.round(coins)}</dd></div>
          <div title="Each coin's volume in its own quote (its collection's wrapped token; SOL for ROOK), added up"><dt>Traded, 24 h</dt><dd className="num">{vol === null ? "—" : compact(vol, "wX").replace(" wX", "")}<small> wX</small></dd></div>
          <div title="The graduated coins' vaults: half of each raise, in wrapped NFT tokens, valued at the collections' day-1 floors"><dt>Backing in coin vaults</dt><dd className="num">{backing === null ? "—" : `≈ ${Math.round(backing)}`}<small> SOL</small></dd></div>
          <div><dt>Graduated</dt><dd className="num">{m.source === "none" ? "—" : st.graduated}</dd></div>
        </dl>
        <div className="hero3__next">
          <p className="hero3__k">Next to graduate</p>
          {st.next.length ? <ol>{st.next.map((c) => <li key={c.mint}><a href={href("coin", c.mint)}><CoinPic c={c} size={20} /><strong>{c.symbol}</strong><span className="tcard__bar"><i style={{ width: `${Math.round(progressOf(c) * 100)}%` }} /></span><span className="num">{Math.round(progressOf(c) * 100)}%</span></a></li>)}</ol>
            : <p className="small muted">{m.loading ? "Reading the curves…" : "No coin on the curve yet."}</p>}
        </div>
        <div className="hero3__stream">
          <p className="hero3__k">Live trades</p>
          {live.length ? <ul className="feed feed--hero" aria-live="off">{live.slice(0, 4).map((a) => <FeedRow key={a.key} a={a} now={now} />)}</ul>
            : <p className="small muted">{m.source === "rpc" ? NEED_INDEXER : m.source === "none" ? "Trades stream here once coins launch." : "Waiting for the first trade…"}</p>}
        </div>
      </div>
      <Genesis m={m} />
      <Ticker items={live} now={now} />
    </section>
  );
}

/** The genesis coin, at the top: ROOK's row from the board when it has one (the indexer's, the RPC board's, or the demo's), else read
 *  straight from ROOK's pool. Before ROOK's addresses are in the site config it says "Launches soon" and reads nothing. */
function Genesis({ m }: { m: Market }) {
  const mint = SOL.ready && SOL.rook ? SOL.rook.mint : null;
  const onBoard = useMemo(() => (mint ? m.coins.find((c) => c.mint === mint) ?? null : null), [m.coins, mint]);
  const direct = useRookRow(!!mint && !onBoard && !DEMO);
  const coin = useMemo(() => onBoard ?? (direct.data ? coinFromRow(direct.data, m.usdPerSol) : null), [onBoard, direct.data, m.usdPerSol]);
  return <GenesisCard mint={mint} coin={coin} tradeHref={mint ? href("coin", mint) : null} docsHref={href("docs", "what-is-rook")} loading={m.loading || direct.isLoading} demo={DEMO} />;
}

/** The scrolling tape under the hero. Its items are a snapshot, refreshed when the loop comes round, so it never jumps. */
function Ticker({ items, now }: { items: FeedItem[]; now: number }) {
  const [snap, setSnap] = useState<FeedItem[] | null>(null);
  const shown = snap && snap.length >= Math.min(8, items.length) ? snap : items.slice(0, 16);
  if (!shown.length) return null;
  const row = (dup: boolean) => shown.map((a) => (
    <li key={(dup ? "d" : "") + a.key} className={`ticker__item ticker__item--${a.kind}`} aria-hidden={dup || undefined}>
      <span className="ticker__kind">{a.kind}</span><a href={href("coin", a.coin.address)} tabIndex={dup ? -1 : undefined}>${a.coin.symbol}</a>
      <span className="num">{a.solLamports ? `${(Number(a.solLamports) / 1e9).toFixed(2)} ${quoteOf(a.coin.collection).symbol}` : ""}</span><span className="muted num">{a.at ? `${Math.max(0, now - a.at)}s` : ""}</span>
    </li>));
  return (
    <div className="ticker hero3__ticker" aria-label="Recent trades">
      <span className="ticker__label"><span className="dot" aria-hidden="true" />Tape</span>
      <div className="ticker__rail"><ul className="ticker__track" onAnimationIteration={() => setSnap(items.slice(0, 16))}>{row(false)}{row(true)}</ul></div>
    </div>
  );
}

function Trending({ m }: { m: Market }) {
  const [mode, setMode] = useState<"trending" | "mcap">("trending");
  const top = useMemo(() => (mode === "mcap" ? topByMcapOf(m.coins, 12) : trendingOf(m.coins, 12)), [m.coins, mode]);
  if (!top.length) return null;
  const hist = m.coins.some((c) => c.volume24hSol !== null);
  return (
    <section className="trend" aria-labelledby="trend-h">
      <div className="trend__head">
        <h2 className="trend__h" id="trend-h">{mode === "mcap" ? "Top market cap" : "Trending"} <span className="muted">· {mode === "mcap" ? "biggest coins on Rook" : hist ? "24 h volume" : "market cap"}</span></h2>
        <div className="tabs trend__tabs" role="tablist" aria-label="Rank by">
          <button type="button" role="tab" aria-selected={mode === "trending"} className={mode === "trending" ? "on" : undefined} onClick={() => setMode("trending")}>Trending</button>
          <button type="button" role="tab" aria-selected={mode === "mcap"} className={mode === "mcap" ? "on" : undefined} onClick={() => setMode("mcap")}>Top market cap</button>
        </div>
      </div>
      <ol className="trend__row">{top.map((c, i) => (
        <li key={c.mint}><a className="trend__card" href={href("coin", c.mint)}>
          <span className="trend__rank num">#{i + 1}</span>
          <CoinPic c={c} size={36} />
          <span className="trend__id"><strong>{c.symbol}</strong><span className="num">{c.marketCapUsd !== null ? compact(c.marketCapUsd, "$") : compact(c.marketCapSol, c.quote.symbol)}</span></span>
          <span className="trend__fig"><span className={`num ${c.change24h === null ? "muted" : c.change24h >= 0 ? "good" : "bad"}`}>{changeText(c.change24h)}</span>{c.volume24hSol !== null ? <span className="num muted">V {compact(c.volume24hSol, c.quote.symbol)}</span> : <span className="num muted">{c.phase === "graduated" ? "graduated" : `${Math.round(progressOf(c) * 100)}%`}</span>}</span>
          <Spark values={c.spark} w={56} h={20} />
        </a></li>))}
      </ol>
    </section>
  );
}

type Col = "fresh" | "soon" | "done" | "feed";
const COLS: [Col, string, string][] = [["fresh", "New", "Just launched, newest first"], ["soon", "About to graduate", "60 % or more of the way to graduation"], ["done", "Graduated", "In their locked Meteora pools"], ["feed", "Live trades", "Every buy and sell, as it lands"]];

function Terminal({ m, feed, now }: { m: Market; feed: FeedItem[]; now: number }) {
  const [tab, setTab] = useState<Col>("fresh");
  const [nest, setNest] = useState("");
  const [q, setQ] = useState("");
  const shown = useMemo(() => filterCoins(m.coins, { collection: nest, q }), [m.coins, nest, q]);
  const cols = useMemo(() => columnsOf(shown), [shown]);
  const mints = useMemo(() => new Set(shown.map((c) => c.mint)), [shown]);
  const feedShown = useMemo(() => feed.filter((a) => (nest || q ? mints.has(a.coin.address) : true)), [feed, mints, nest, q]);
  const flash = useMemo(() => { const f = new Map<string, { key: string; side: string }>(); for (const a of feed) if (a.live && !f.has(a.coin.address)) f.set(a.coin.address, { key: a.key, side: a.kind }); return f; }, [feed]);
  const list = (cs: MarketCoin[], empty: string) => cs.length ? <ul className="tcol__list">{cs.map((c) => <CoinCard key={c.mint} c={c} usd={m.usdPerSol} now={now} flash={flash.get(c.mint)} />)}</ul> : <p className="tcol__empty small muted">{m.loading ? "Reading the curves…" : empty}</p>;
  const n = { fresh: cols.fresh.length, soon: cols.soon.length, done: cols.done.length, feed: feedShown.length };
  return (
    <section className="term" id="terminal" aria-labelledby="term-h">
      <div className="term__head">
        <h2 id="term-h" className="term__h"><span className="dot" aria-hidden="true" />Terminal</h2>
        <label className="term__search"><span className="vh">Search coins</span><input type="search" placeholder="Search ticker, name or mint" value={q} autoComplete="off" spellCheck={false} onChange={(e) => setQ(e.target.value)} /></label>
        <label className="term__nest"><span className="vh">Collection</span><select value={nest} onChange={(e) => setNest(e.target.value)} aria-label="Only coins on one collection">
          <option value="">All collections</option>{COLLECTIONS.map((c) => <option key={c.slug} value={c.slug}>{c.name}</option>)}<option value="rook">Genesis (ROOK)</option></select></label>
        <a className="term__all small" href={href("coins")}>Full board →</a>
      </div>
      <div className="term__tabs" role="tablist" aria-label="Column">{COLS.map(([k, label]) => <button key={k} type="button" role="tab" aria-selected={tab === k} className={tab === k ? "on" : ""} onClick={() => setTab(k)}>{label} <span className="num muted">{n[k]}</span></button>)}</div>
      {m.error ? <p className="error small">The coins could not be read: {String((m.error as Error).message).slice(0, 160)}</p> : null}
      {m.source === "none" ? <p className="notice small term__note">Rook's collection configs are created by the day-1 ceremony; coins appear here as soon as they launch. <a href={href("launch")}>How launching works</a>.</p> : null}
      <div className="term__cols">
        {COLS.map(([k, label, sub]) => (
          <div key={k} className={`tcol${tab === k ? " tcol--on" : ""}`} role="tabpanel" aria-label={label}>
            <div className="tcol__head"><h3>{label}</h3><span className="num muted small">{n[k]}</span><p className="small muted">{sub}</p></div>
            {k === "feed" ? (feedShown.length ? <ul className="feed tcol__list">{feedShown.slice(0, 50).map((a) => <FeedRow key={a.key} a={a} now={now} />)}</ul> : <p className="tcol__empty small muted">{m.source === "rpc" ? NEED_INDEXER : m.source === "none" ? "Trades stream here once coins launch." : "Nothing yet."}</p>)
              : list(cols[k], k === "fresh" ? "No new coin right now. Launch one." : k === "soon" ? "No coin is past 60 % yet." : "No coin has graduated yet.")}
          </div>))}
      </div>
      {m.more ? <p className="small"><button type="button" className="btn btn--quiet btn--small" disabled={m.loadingMore} onClick={m.more}>{m.loadingMore ? "Loading…" : "Load older coins"}</button></p> : null}
      <p className="small muted term__foot">{DEMO ? "Demo: invented coins and an invented trade stream, generated in this browser; nothing here is on chain." : m.source === "indexer" ? "From Rook's indexer, which reads Solana and refreshes every few seconds." : m.source === "rpc" ? `Read straight from Solana (${new URL(SOL.rpcUrl).host}); 24 h figures, the tape and charts need the indexer.` : ""}</p>
    </section>
  );
}

function About() {
  return (
    <>
      <section className="block about" aria-labelledby="about-h">
        <p className="eyebrow" id="about-h">What Rook is</p>
        <ol className="about__steps">
          <li><strong>Launch on a collection.</strong> Name, ticker, image: one transaction creates a 1B-supply coin on Meteora's bonding curve under that collection's Rook config, priced in the collection's wrapped NFT token (wX: one unit, one NFT in custody).</li>
          <li><strong>Trade on the curve.</strong> Pay in SOL: the site swaps SOL into wX and wX into the coin in one transaction. Every trade pays a fee in wX: 50 % at launch falling to 1 % after ten minutes (the anti-sniper fee). 80 % is Rook's, 20 % Meteora's.</li>
          <li><strong>Graduate at the collection's threshold.</strong> A set number of whole wX (at least 3), sized so the raise is worth about 85 SOL at the floor, more for the priciest collections. The coin moves to a Meteora pool whose liquidity is locked for good.</li>
          <li><strong>Backed by NFTs, at once.</strong> Half the raise, already wX, goes into the coin's own vault at graduation. Any holder can burn coins for their share; one whole wX redeems a real NFT from custody, delivered to your address on its own chain.</li>
        </ol>
        <p className="small"><a href={href(HOW_IT_WORKS)}>How it works, step by step →</a> the coin's life, how the NFTs are bought on each chain, redemption, the raffle, the treasury, and what you can check.</p>
      </section>
      <section className="block" aria-labelledby="run-h">
        <div className="grid">
          <div className="span-7">
            <p className="eyebrow" id="run-h">How Rook runs</p>
            <h2 style={{ marginTop: 0 }}>No programs to deploy: a custodian that keeps the books</h2>
            <p>Rook deploys nothing of its own on Solana. The custodian, a program on the Internet Computer whose keys no single person holds, mints and burns every wX, keeps every collection's books, buys and holds the NFTs, and signs every move of money. Every wX mint or burn carries a label on chain, every book is a public query, and the <a href={href("reserves")}>reserves page</a> matches the books against Solana, Ethereum, Robinhood Chain and Bitcoin every few minutes.</p>
            <p className="row" style={{ marginTop: "1rem" }}>
              {SOL.ready && SOL.rook ? <a className="btn" href={href("coin", SOL.rook.mint)}>Trade ROOK</a> : <span className="small muted">ROOK, the genesis coin, launches on day 1; its trading link appears here then.</span>}
              <a className="small" href={href("docs", "security")}>Who controls what →</a>
            </p>
          </div>
          <div className="span-5 small">
            <p>Every collection Rook can buy, hold and price is ready at launch, on Ethereum, Solana and Bitcoin: each opens the first time someone launches a coin on it. Rook doesn't pick, the people do. More Ordinals follow as their floor feeds pass Rook's checks, and three Robinhood Chain collections once the custodian has checked the marketplace there. ROOK stays SOL-paired, has no backing, and its fees pay Rook's running costs.</p>
            <details className="small"><summary>What you are trusting</summary>
              <p>The custodian's code (open source, reproducible) and the 72-hour timelock that controls changes to it; the Internet Computer subnets it runs on and that hold its keys; two Solana data providers that must agree; Meteora's programs; Relay for the conversions; the marketplaces it buys from. Internal reviews found and fixed every High, Medium and Low in the code they covered; the newest code (Solana NFT buying, the extra Ordinals, Robinhood Chain, opening on demand) has not been reviewed yet, and no outside firm has audited any of it. It is not trustless: it is checkable. Read the <a href={href("docs", "risks")}>risks</a> before you buy.</p>
            </details>
          </div>
        </div>
      </section>
      <section className="block" aria-labelledby="cols-h">
        <div className="row" style={{ marginBottom: ".75rem" }}><div><p className="eyebrow" id="cols-h">The collections</p><h2 style={{ margin: 0 }}>{COLLECTIONS.length} collections, four chains</h2></div><a className="right small" href={href("collections")}>All {COLLECTIONS.length}, by chain →</a></div>
        <ul className="solcols">{COLLECTIONS.map((c) => <li key={c.slug}><a href={href("collection", c.slug)} title={c.name}><CollectionLogo c={c} size={56} /><span className="small">{c.name}</span></a></li>)}</ul>
      </section>
    </>
  );
}
