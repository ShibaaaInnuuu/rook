import { useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { href, useNow } from "../lib/env";
import { collectionBySlug } from "../rook/sol/collections";
import { CollectionLinksRow } from "../ui/CollectionLinksRow";
import { baseFeeNumerator, DBC_PROGRAM, poolStats, quoteSwap, type DbcConfig, type DbcPool } from "../rook/sol/dbc";
import { currentFeeRate } from "../rook/sol/dlmm";
import { quoteTwoHopBuy, quoteTwoHopSell, quoteWxBuy, quoteWxSell, twoHopBuyIxs, twoHopSellIxs, wxBuyIxs, wxPriceSol, wxSellIxs, type CoinHop, type HopState, type WxPool } from "../rook/sol/route";
import { DEMO, SITE, useBalances, useCoin, useCoinMeta, useConfigs, useMigratedPool, useOffChain, useSolUsd, useWxPool, rpc } from "../rook/sol/hooks";
import { demoQuoteSol } from "../rook/sol/demo";
import { isAddress, NATIVE_MINT } from "../rook/sol/keys";
import { ata } from "../rook/sol/pda";
import { age, jupiter, lamportsToSol, parseUnits, pctText, short, solNum, solText, solscan, units, usd } from "../rook/sol/format";
import { HAS_HISTORY, useCoinChart, useCoinHolders, useCoinTrades, useMarketCoin } from "../rook/sol/hooksMarket";
import type { IxRes } from "../rook/sol/indexerTypes";
import { changeText, compact, priceShort, priceText, tokensText, type MarketCoin } from "../rook/sol/market";
import { SolChart } from "../ui/SolChart";
import { CollBadge } from "../ui/market";
import { PARTNER_MIGRATION_FEE_TAKEN } from "../rook/sol/dbc";
import { buyIxs, sellIxs, boundBySimulation, type PoolRef } from "../rook/sol/trade";
import { simulateAndSend, type SendStep } from "../rook/sol/send";
import { SolWalletButton, useSolWallet } from "../rook/sol/wallet";
import { Fig, Loading, Pill } from "../ui/bits";
import { CoinBacking } from "../ui/BackingPanels";
import { IntentDesk } from "../ui/IntentDesk";
import { CANISTER_ON, useCanisterIndex } from "../rook/sol/hooksCanister";
import { Share } from "../ui/Share";
import { RafflePanel } from "../ui/RafflePanel";
import { Addr, CoinFace, DayOnePending, GraduationExplainer } from "../ui/sol";

type BoardCoin = { mint: string; pool: string; p: DbcPool; cfg: DbcConfig; slug: string; label: string };

/**
 * A coin: its metadata, the curve's stats (price and market cap in its quote, the collection's wX; raised of its own threshold, read
 * from its config; the fee now), the ticket (a wX-quoted coin: the site's two-hop buy and sell through the collection's wX/SOL pool,
 * or one hop with wX the wallet holds; ROOK's SOL curve: DBC `swap`; every trade simulated before the wallet is asked; after graduation
 * the DAMM v2 pool, and a Jupiter link on ROOK), the backing panel (the coin's vault from the custodian's books, and what the curve's
 * config says of the backing share), and the redeem-coin desk once graduated. The chart, the trades and the holders (the Robinhood
 * site's coin page, ported to Solana) come from Rook's indexer, or the demo fixtures; without an indexer they say so.
 */
export function SolCoin({ mint }: { mint?: string }) {
  const coin = useCoin(mint), cfgs = useConfigs();
  if (!SITE.ready) return <><header className="pagehead rise"><p className="eyebrow">Coin</p><h1>Coin pages</h1></header><DayOnePending what="Trading" /></>;
  if (!mint || !isAddress(mint)) return <div className="pagehead"><h1>Not a coin address</h1><p className="lede">A coin's page is <span className="mono">#/coin/&lt;mint&gt;</span>. <a href={href("coins")}>All coins</a>.</p></div>;
  if (coin.isLoading || cfgs.isLoading) return <Loading what="the coin from Solana" />;
  if (coin.error) return <p className="error">The coin could not be read: {String((coin.error as Error).message).slice(0, 160)}</p>;
  if (!coin.data) return <div className="pagehead"><h1>Not a Rook coin</h1><p className="lede">No pool on any of Rook's configs has the mint <span className="mono break">{mint}</span>. It may be a coin on another launchpad. <a href={href("coins")}>All Rook coins</a>.</p></div>;
  const cfg = cfgs.data?.get(coin.data.ref.config);
  if (!cfg) return <p className="error">The coin's config could not be read.</p>;
  return <CoinView c={{ mint, pool: coin.data.pool, p: coin.data.poolAcc, cfg, slug: coin.data.ref.slug, label: coin.data.ref.label }} />;
}

type Tab = "trades" | "holders" | "info";
const NEEDS = "needs Rook's indexer: this build reads the curve straight from Solana, which keeps no trade history.";

function CoinView({ c }: { c: BoardCoin }) {
  const md = useCoinMeta(c.mint);
  const off = useOffChain(md?.uri);
  const px = useSolUsd();
  const now = useNow();
  const mk = useMarketCoin(c.mint).data ?? null;
  const [res, setRes] = useState<IxRes>("5m");
  const [tab, setTab] = useState<Tab>("trades");
  const chart = useCoinChart(c.mint, res);
  const s = poolStats(c.cfg, c.p);
  const name = md?.name || "Unnamed coin", sym = md?.symbol || "?";
  const col = collectionBySlug(c.slug);
  const { coll: canisterColl } = useCanisterIndex(col?.wxSymbol); // the canister's index: its unlock order, not the site's listing order
  const feeNow = Number(baseFeeNumerator(c.cfg, c.p.activationPoint, BigInt(now))) / 1e7;
  const launched = c.cfg.activationType === 1 ? Number(c.p.activationPoint) : 0;
  const usdPer = px.data?.price ?? null;
  const phaseWord = s.phase === "graduated" ? "Graduated" : s.phase === "graduating" ? "Migrating" : "On the curve";
  // a canister-only coin is quoted in its collection's wX (docs/v9/canister-only-solana.md §3): its prices are in wX, and the SOL and
  // USD views come through the collection's wX/SOL pair (§5, §17), when one exists
  const wxQuoted = c.cfg.quoteMint !== NATIVE_MINT;
  const wxSym = wxQuoted ? col?.wxSymbol ?? "wX" : "SOL";
  const wxp = useWxPool(wxQuoted ? c.cfg.quoteMint : null);
  // the demo reads no pair: its wX is valued at the collection's day-1 floor
  const wxSol = !wxQuoted ? 1 : DEMO ? demoQuoteSol(c.slug) : wxPriceSol(wxp.data);
  const mcSol = s.marketCapSol === null ? null : wxSol === null ? null : s.marketCapSol * wxSol;
  const priceSol = wxSol === null ? null : s.priceSol * wxSol;
  return (
    <>
      <header className="coinhead2 rise">
        <CoinFace uri={md?.uri} name={name} seed={c.mint} size={72} />
        <div className="coinhead2__id">
          <p className="coinhead2__eyebrow">{col ? <CollBadge slug={c.slug} link /> : <span className="cbadge">{c.slug === "rook" ? "Genesis coin · its fees pay Rook's running costs" : c.label}</span>}<Pill tone={s.phase === "graduated" ? "good" : s.phase === "graduating" ? "warn" : "live"}>{phaseWord}</Pill>{launched ? <span className="muted small num">{age(launched, now)} old</span> : null}</p>
          <h1>{name} <span className="muted">{sym}</span></h1>
          {off.data?.description ? <p className="muted coinhead2__about">{off.data.description}</p> : null}
          <p className="row small coinhead2__links">
            <span>Mint <Addr id={c.mint} kind="token" /></span>
            {off.data?.website ? <a href={off.data.website} target="_blank" rel="noreferrer noopener">Website ↗</a> : null}
            {off.data?.twitter ? <a href={xUrl(off.data.twitter)} target="_blank" rel="noreferrer noopener">X ↗</a> : null}
            {off.data?.telegram ? <a href={tgUrl(off.data.telegram)} target="_blank" rel="noreferrer noopener">Telegram ↗</a> : null}
            <Share address={c.mint} symbol={sym} />
          </p>
          {col ? <CollectionLinksRow slug={c.slug} name={col.name} /> : null}
        </div>
      </header>
      <div className="strip strip--stats coinstats2 rise rise-1">
        <Fig label="Market cap" value={usdPer && mcSol !== null ? usd(mcSol * usdPer) : mcSol !== null ? `${solNum(mcSol, 1)} SOL` : `${solNum(s.marketCapSol, 1)} ${wxSym}`} big />
        <Fig label={wxQuoted ? `In ${wxSym}` : "In SOL"} value={`${solNum(s.marketCapSol, 1)}`} unit={` ${wxSym}`} />
        <Fig label="Price" value={<span title={`${priceText(s.priceSol)} ${wxSym}${wxQuoted ? priceSol !== null ? ` ≈ ${priceText(priceSol)} SOL${usdPer ? ` ≈ $${(priceSol * usdPer).toPrecision(3)}` : ""}` : " (no wX/SOL pool yet: no SOL price)" : ""}`}>{priceShort(s.priceSol)}</span>} unit={` ${wxSym}`} />
        {wxQuoted ? <Fig label="In SOL" value={priceSol !== null ? <span title={`${priceText(priceSol)} SOL · ${wxSym} at ${solNum(wxSol, 3)} SOL on its pool`}>{priceShort(priceSol)}</span> : <span className="muted" title="No wX/SOL pool for this collection yet">—</span>} unit=" SOL" /> : null}
        <Fig label="24h" value={<span className={mk?.change24h == null ? "muted" : mk.change24h >= 0 ? "good" : "bad"}>{changeText(mk?.change24h)}</span>} />
        <Fig label="Volume 24h" value={mk?.volume24hSol != null ? compact(mk.volume24hSol, wxSym) : "—"} />
        <Fig label="Holders" value={mk?.holders ?? "—"} />
        <Fig label={s.phase === "graduated" ? "Pool fee" : "Fee now"} value={s.phase === "graduated" ? "1 %" : `${feeNow.toFixed(feeNow < 2 ? 2 : 1)} %`} />
      </div>
      {s.phase !== "graduated" ? <div className="gradline2 rise rise-1"><span className="progress progress--live"><i style={{ width: `${Math.max(1, Math.round(s.progress * 100))}%` }} /></span><p className="small"><strong className="num">{pctText(s.progress)}</strong> of the way: <span className="num">{solNum(lamportsToSol(s.raisedLamports), 2)}</span> of <span className="num">{solNum(lamportsToSol(s.thresholdLamports), 0)}</span> {wxSym} raised. {s.phase === "graduating" ? "The curve is full; the coin is moving to its locked pool." : `At ${solNum(lamportsToSol(s.thresholdLamports), 0)} ${wxSym} it graduates into a locked Meteora pool${c.slug === "rook" ? "." : ` and half the raise, ${solNum(lamportsToSol(s.thresholdLamports) / 2, 1)} ${wxSym}, goes straight into the coin's vault.`}`}</p></div>
        : <p className="small muted gradline2">Graduated{mk?.graduatedAt ? ` ${age(mk.graduatedAt, now)} ago` : ""}: trading in its Meteora DAMM v2 pool{wxQuoted ? ` (${sym}/${wxSym})` : ""}, liquidity locked for good.</p>}
      <div className="grid coingrid rise rise-2">
        <div className="span-8 stack"><SolChart set={chart.data} res={res} onRes={setRes} symbol={sym} loading={chart.isLoading} error={chart.error} missing={HAS_HISTORY ? null : `The chart ${NEEDS}`} /><div className="coingrid__wide">{c.slug === "rook" ? <RookNote /> : <BackingPanel c={c} sym={sym} />}</div></div>
        <div className="span-4 stack coingrid__side">{DEMO ? <DemoTicket c={c} sym={sym} quote={wxSym} /> : wxQuoted ? <WxTicket c={c} sym={sym} wxSym={wxSym} wxp={wxp.data ?? null} wxpLoading={wxp.isLoading} usdPer={usdPer} /> : <Ticket c={c} sym={sym} />}<div className="coingrid__narrow">{c.slug === "rook" ? <RookNote /> : <BackingPanel c={c} sym={sym} />}</div>
          {col && s.phase === "graduated" && canisterColl !== null ? <IntentDesk coll={canisterColl} wx={col.wxSymbol} wxMint={null} kinds={["redeem_coin"]} coin={{ mint: c.mint, symbol: sym, decimals: c.cfg.tokenDecimal }} compact /> : null}
          {col ? <RafflePanel slug={c.slug} compact /> : null}</div>
      </div>
      <section className="block rise rise-3" aria-label="Coin details">
        <div className="tabs" role="tablist" aria-label="Coin details">{(["trades", "holders", "info"] as Tab[]).map((t) => <button key={t} type="button" role="tab" aria-selected={tab === t} className={tab === t ? "on" : ""} onClick={() => setTab(t)}>{t === "trades" ? `Trades${mk?.tradesAllTime ? ` · ${mk.tradesAllTime}` : ""}` : t === "holders" ? `Holders${mk?.holders ? ` · ${mk.holders}` : ""}` : "Info"}</button>)}</div>
        {tab === "trades" ? <Trades mint={c.mint} sym={sym} quote={wxSym} usdPer={usdPer} now={now} /> : tab === "holders" ? <Holders mint={c.mint} sym={sym} /> : <Info c={c} s={s} mk={mk} off={off.data ?? null} quote={wxSym} />}
      </section>
    </>
  );
}

/** A transaction or account link to Solscan; in demo, the made-up id as text (it leads nowhere). */
const Ln = ({ id, kind, label }: { id: string; kind: "account" | "tx"; label?: string }) => DEMO ? <span className="mono" title={`${id} (demo: invented)`}>{label ?? short(id)}</span> : <a className="mono" href={solscan(kind, id)} target="_blank" rel="noreferrer noopener" title={id}>{label ?? short(id)}</a>;

/** The tape. Amounts and prices are in the coin's quote (`quote`: the collection's wX, SOL for ROOK); the dollar column uses the
 *  trade's own USD price when the source has one, else SOL/USD on a SOL-quoted coin only. */
function Trades({ mint, sym, quote, usdPer, now }: { mint: string; sym: string; quote: string; usdPer: number | null; now: number }) {
  const q = useCoinTrades(mint);
  if (!HAS_HISTORY) return <p className="small muted">The trade tape {NEEDS} Every trade is on Solscan under the coin's mint.</p>;
  const items = q.data?.pages.flatMap((p) => p?.items ?? []) ?? [];
  const total = q.data?.pages[0]?.total ?? items.length;
  if (q.isLoading) return <Loading what="the trades" />;
  if (q.error) return <p className="small warn">The trades could not be read: {String((q.error as Error).message).slice(0, 120)}</p>;
  if (!items.length) return <p className="small muted">This coin has not traded yet. The first buy appears here with its transaction.</p>;
  return (
    <>
      <div className="scroll"><table className="tape">
        <thead><tr><th>Time</th><th>Side</th><th className="r">Price ({quote})</th><th className="r">Size</th><th className="r">{quote}</th><th className="r">USD</th><th>Trader</th><th>Tx</th></tr></thead>
        <tbody>{items.map((t) => (
          <tr key={t.id} className={`tape__row tape__row--${t.side}`}>
            <td className="num muted" title={t.at ? new Date(t.at * 1000).toISOString().replace("T", " ").slice(0, 19) + " UTC" : ""}>{t.at ? age(t.at, now) : "—"}</td>
            <td><span className={t.side === "buy" ? "good" : "bad"}>{t.side}</span>{t.venue === "pool" ? <> <span className="muted small">pool</span></> : null}</td>
            <td className="r num" title={priceText(t.price)}>{priceShort(t.price)}</td>
            <td className="r num">{tokensText(t.tokens)} <span className="muted">{sym}</span></td>
            <td className="r num">{t.sol.toFixed(t.sol < 1 ? 4 : 3)}</td>
            <td className="r num muted">{t.priceUsd !== null && t.price > 0 ? usd((t.sol * t.priceUsd) / t.price) : usdPer !== null && quote === "SOL" ? usd(t.sol * usdPer) : "—"}</td>
            <td><Ln id={t.trader} kind="account" /></td>
            <td><Ln id={t.signature} kind="tx" label={`${t.signature.slice(0, 6)}…`} /></td>
          </tr>))}</tbody>
      </table></div>
      <p className="small muted" style={{ marginTop: ".75rem" }}>{total} trades over the coin's life{DEMO ? " · demo: invented traders and transactions, not linked" : " · links open Solscan"}. {q.hasNextPage ? <button type="button" className="btn btn--quiet btn--small" onClick={() => void q.fetchNextPage()} disabled={q.isFetchingNextPage}>{q.isFetchingNextPage ? "Reading…" : "Older trades"}</button> : null}</p>
    </>
  );
}

function Holders({ mint, sym }: { mint: string; sym: string }) {
  const q = useCoinHolders(mint, true);
  if (!HAS_HISTORY) return <p className="small muted">The holder list {NEEDS}</p>;
  const pages = q.data?.pages.filter((p): p is NonNullable<typeof p> => !!p) ?? [];
  const items = pages.flatMap((p) => p.items);
  if (q.isLoading) return <Loading what="the holders" />;
  if (q.error) return <p className="small warn">The holders could not be read: {String((q.error as Error).message).slice(0, 120)}</p>;
  if (!items.length) return <p className="small muted">Nobody holds this coin yet.</p>;
  const top = Math.max(...items.filter((h) => !h.label || h.label === "creator").map((h) => h.share ?? 0), 0.0001);
  return (
    <>
      <p className="small muted">One address, one row: nothing here groups a trader's several wallets. The curve's vault holds the coins not yet sold; after graduation the pool's vault holds them.</p>
      <div className="scroll"><table className="tape">
        <thead><tr><th className="r">#</th><th>Address</th><th className="r">Share</th><th style={{ width: "28%" }}><span className="vh">Bar</span></th><th className="r">Amount</th></tr></thead>
        <tbody>{items.map((h) => (
          <tr key={h.address + h.rank}>
            <td className="r num muted">{h.rank}</td>
            <td><Ln id={h.address} kind="account" />{h.label ? <> <Pill tone={h.label === "creator" ? "warn" : undefined}>{h.label === "curve" ? "curve (unsold)" : h.label === "pool" ? "pool (locked)" : "creator"}</Pill></> : null}</td>
            <td className="r num">{h.share === null ? "—" : `${(h.share * 100).toFixed(2)}%`}</td>
            <td><span className={`progress${h.label === "curve" || h.label === "pool" ? " progress--vault" : ""}`}><i style={{ width: `${Math.min(100, ((h.share ?? 0) / (h.label === "curve" || h.label === "pool" ? 1 : top)) * 100)}%` }} /></span></td>
            <td className="r num">{tokensText(h.amount)} <span className="muted">{sym}</span></td>
          </tr>))}</tbody>
      </table></div>
      <p className="small muted" style={{ marginTop: ".75rem" }}>{pages[0]?.holders ?? "?"} holders (vaults not counted){pages[0]?.complete === false ? "; the top holders only" : ""}. {q.hasNextPage ? <button type="button" className="btn btn--quiet btn--small" onClick={() => void q.fetchNextPage()} disabled={q.isFetchingNextPage}>More holders</button> : null}</p>
    </>
  );
}

function Info({ c, s, mk, off, quote }: { c: BoardCoin; s: ReturnType<typeof poolStats>; mk: MarketCoin | null; off: { website: string | null; twitter: string | null; telegram: string | null } | null; quote: string }) {
  const col = collectionBySlug(c.slug);
  return (
    <div className="grid">
      <dl className="kv kv--left span-6 small">
        <dt>Collection</dt><dd>{col ? <><a href={href("collection", col.slug)}>{col.name}</a> · priced and backed in {col.wxSymbol}</> : c.slug === "rook" ? "None: ROOK is the genesis coin, priced in SOL; its fees pay Rook's running costs" : c.label}</dd>
        <dt>Config</dt><dd><Addr id={c.p.config} /> · Rook's DBC config for {col?.name ?? c.label}</dd>
        <dt>Fee claimer</dt><dd><Addr id={c.cfg.feeClaimer} /> {c.slug === "rook" ? "(the custodian's wallet)" : "(a key only the custodian signs with)"}</dd>
        <dt>Creator</dt><dd><Addr id={c.p.creator} /></dd>
        <dt>Supply</dt><dd>{s.supply ? `${s.supply.toLocaleString("en-US")} (fixed)` : "—"}</dd>
        <dt>Trading fees</dt><dd>{c.cfg.creatorTradingFeePercentage === 0 ? "80 % partner (Rook) · 20 % Meteora · creator 0 %" : `creator ${c.cfg.creatorTradingFeePercentage} % of the 80 % · Meteora 20 %`}; 50 % at launch falling to 1 % after ten minutes</dd>
        <dt>Migration</dt><dd>DAMM v2 at {solNum(lamportsToSol(s.thresholdLamports), 0)} {quote} · LP {c.cfg.partnerLockedLp} % partner + {c.cfg.creatorLockedLp} % creator, permanently locked · {c.cfg.migrationFeePercentage} % of the raise to the claimer{c.slug === "rook" ? "" : " (the backing share, moved into the coin's vault)"}</dd>
      </dl>
      <dl className="kv kv--left span-6 small">
        <dt>Mint</dt><dd><Addr id={c.mint} kind="token" /></dd>
        <dt>Pool (DBC)</dt><dd><Addr id={c.pool} /></dd>
        {mk?.dammPool ? <><dt>Pool (DAMM v2)</dt><dd><Addr id={mk.dammPool} /></dd></> : null}
        <dt>Base vault</dt><dd><Addr id={c.p.baseVault} /></dd><dt>Quote vault</dt><dd><Addr id={c.p.quoteVault} /></dd>
        <dt>Program</dt><dd><Addr id={DBC_PROGRAM} /> Meteora DBC</dd>
        <dt>Links</dt><dd className="row" style={{ gap: ".25rem .75rem" }}>
          {off?.website ? <a href={off.website} target="_blank" rel="noreferrer noopener">Website ↗</a> : null}
          {off?.twitter ? <a href={xUrl(off.twitter)} target="_blank" rel="noreferrer noopener">X ↗</a> : null}
          {off?.telegram ? <a href={tgUrl(off.telegram)} target="_blank" rel="noreferrer noopener">Telegram ↗</a> : null}
          <a href={solscan("token", c.mint)} target="_blank" rel="noreferrer noopener">Solscan ↗</a>
          {s.phase === "graduated" ? <a href={jupiter(c.mint)} target="_blank" rel="noreferrer noopener">Jupiter ↗</a> : null}
        </dd>
      </dl>
      {c.slug !== "rook" ? <div className="span-12"><GraduationExplainer slug={c.slug} /></div> : null}
    </div>
  );
}

const xUrl = (h: string) => (/^https:\/\//.test(h) ? h : `https://x.com/${h.replace(/^@/, "")}`);
const tgUrl = (h: string) => (/^https:\/\//.test(h) ? h : `https://t.me/${h.replace(/^(t\.me\/|@)/, "")}`);


const SLIPS = [50, 100, 300, 1000];
type Review = { side: "buy" | "sell"; amount: bigint; received: bigint; minOut: bigint; solDelta: bigint };
function Ticket({ c, sym }: { c: BoardCoin; sym: string }) {
  const w = useSolWallet();
  const qc = useQueryClient();
  const now = useNow();
  const bal = useBalances(w.address, c.mint);
  const [side, setSide] = useState<"buy" | "sell">("buy");
  const [amount, setAmount] = useState("");
  const [slip, setSlip] = useState(100);
  const [review, setReview] = useState<Review | null>(null);
  const [step, setStep] = useState<SendStep | "reviewing" | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [done, setDone] = useState<string | null>(null);
  const s = poolStats(c.cfg, c.p);
  if (s.phase === "graduated") return (
    <section className="panel ticket">
      <p className="eyebrow">Trade</p>
      <p>This coin has graduated: it trades in its Meteora DAMM v2 pool (liquidity locked for good), which Jupiter routes.</p>
      <a className="btn btn--primary btn--block" href={jupiter(c.mint)} target="_blank" rel="noreferrer noopener">Trade {sym} on Jupiter ↗</a>
      <p className="small muted" style={{ marginBottom: 0 }}>Jupiter is a third-party site; check the mint (<span className="mono">{c.mint.slice(0, 6)}…</span>) before you sign there.</p>
    </section>
  );
  if (s.phase === "graduating") return (
    <section className="panel ticket"><p className="eyebrow">Trade</p><p>The curve is full ({solNum(lamportsToSol(s.thresholdLamports), 0)} SOL). The coin is moving to its DAMM v2 pool: Meteora's keeper, or anyone, triggers it. The curve takes no more trades; trading resumes in the pool, on <a href={jupiter(c.mint)} target="_blank" rel="noreferrer noopener">Jupiter ↗</a>, once it has moved.</p></section>
  );
  const decimals = side === "buy" ? 9 : c.cfg.tokenDecimal;
  const amt = parseUnits(amount, decimals);
  const est = amt ? quoteSwap(c.cfg, c.p, side, amt, BigInt(now)) : null;
  const ref: PoolRef = { pool: c.pool, config: c.p.config, baseMint: c.mint, baseVault: c.p.baseVault, quoteVault: c.p.quoteVault };
  const outDec = side === "buy" ? c.cfg.tokenDecimal : 9;
  const build = (a: bigint) => (min: bigint, unwrap: boolean) => (side === "buy" ? buyIxs(w.address!, ref, a, min, unwrap) : sellIxs(w.address!, ref, a, min, unwrap));
  const tooMuch = amt && bal.data ? (side === "buy" ? amt + 20_000_000n > bal.data.sol : amt > bal.data.coin) : false;
  const why = !w.address ? "Connect a wallet to trade." : !amt ? `Enter an amount of ${side === "buy" ? "SOL" : sym}.` : tooMuch ? (side === "buy" ? "More than your SOL (keep ≈ 0.02 SOL for fees and accounts)." : `More ${sym} than you hold.`) : !est ? "The curve cannot fill this amount." : null;
  const reset = () => { setReview(null); setErr(null); setDone(null); };
  const doReview = async () => {
    reset(); setStep("reviewing");
    try {
      const out = side === "buy" ? ata(w.address!, c.mint) : ata(w.address!, NATIVE_MINT);
      const r = await boundBySimulation(rpc(), w.address!, out, build(amt!), slip);
      setReview({ side, amount: amt!, received: r.received, minOut: r.minOut, solDelta: r.solDelta });
    } catch (e) { setErr(String((e as Error).message ?? e).slice(0, 240)); }
    setStep(null);
  };
  const doSend = async () => {
    if (!review) return;
    setErr(null);
    try {
      const sig = await simulateAndSend(rpc(), { ...w, address: w.address! }, build(review.amount)(review.minOut, true), { onStep: setStep });
      setDone(sig); setReview(null); setAmount("");
      await qc.invalidateQueries();
    } catch (e) {
      const m = String((e as Error).message ?? e);
      setErr(/reject|denied|cancel/i.test(m) ? "You declined it in your wallet. Nothing was sent." : m.slice(0, 240));
    }
    setStep(null);
  };
  return (
    <section className="panel ticket" aria-label="Trade">
      <div className="ticket__tabs" role="group" aria-label="Buy or sell">{(["buy", "sell"] as const).map((x) => <button key={x} type="button" aria-pressed={side === x} onClick={() => { setSide(x); setAmount(""); reset(); }}>{x === "buy" ? "Buy" : "Sell"}</button>)}</div>
      <label className="ticket__in"><input inputMode="decimal" placeholder="0.0" value={amount} aria-label={side === "buy" ? "SOL to spend" : `${sym} to sell`} onChange={(e) => { setAmount(e.target.value); reset(); }} /><span>{side === "buy" ? "SOL" : sym}</span></label>
      {w.address && bal.data ? <p className="small muted" style={{ margin: ".25rem 0" }}>You have {units(bal.data.sol, 9, 4)} SOL · {units(bal.data.coin, c.cfg.tokenDecimal, 2)} {sym}{side === "sell" && bal.data.coin > 0n ? <> · <button type="button" className="btn btn--quiet btn--small" onClick={() => { setAmount(units(bal.data!.coin, c.cfg.tokenDecimal, c.cfg.tokenDecimal).replace(/,/g, "")); reset(); }}>Max</button></> : null}</p> : null}
      <div className="ticket__line"><span className="ticket__label">You get (estimate)</span><span className="ticket__value">{est ? `${units(est.amountOut, outDec, side === "buy" ? 0 : 6)} ${side === "buy" ? sym : "SOL"}` : "—"}</span></div>
      <div className="ticket__line"><span className="ticket__label">Fee now</span><span className="ticket__value">{est ? `${units(est.fee, 9, 6)} SOL (${(Number(est.feeNumerator) / 1e7).toFixed(2)} %)` : "—"}</span></div>
      <div className="ticket__prefs small"><span className="muted">Slippage</span><span className="seg">{SLIPS.map((b) => <button key={b} type="button" aria-pressed={slip === b} onClick={() => { setSlip(b); reset(); }}>{b / 100} %</button>)}</span></div>
      {review ? (
        <div className="ticket__total">
          <div className="ticket__line"><span className="ticket__label">Simulated on chain: you receive</span><span className="ticket__value">{units(review.received, outDec, 6)} {review.side === "buy" ? sym : "SOL"}</span></div>
          <div className="ticket__line"><span className="ticket__label">At least (slippage {slip / 100} %)</span><span className="ticket__value">{units(review.minOut, outDec, 6)}</span></div>
          <div className="ticket__line"><span className="ticket__label">Your SOL moves by</span><span className="ticket__value">{solText(review.solDelta, 6)} SOL<small> before unwrapping{review.side === "sell" ? " the proceeds" : ""} and the network fee</small></span></div>
        </div>
      ) : null}
      <div className="ticket__go">
        {!w.address ? <SolWalletButton /> : review
          ? <button className="btn btn--primary btn--block" disabled={!!step} onClick={() => void doSend()}>{step === "simulating" ? "Simulating…" : step === "signing" ? "Confirm in your wallet…" : step === "sending" ? "Sending…" : step === "confirming" ? "Confirming…" : `Confirm ${review.side} in wallet`}</button>
          : <button className="btn btn--primary btn--block" disabled={!!why || !!step} onClick={() => void doReview()}>{step === "reviewing" ? "Simulating…" : side === "buy" ? "Review buy" : "Review sell"}</button>}
      </div>
      {why && w.address ? <p className="muted small" style={{ marginTop: ".5rem" }}>{why}</p> : null}
      {err ? <p className="error small" role="alert">{err}</p> : null}
      {done ? <p className="good small" role="status">Confirmed. <a className="mono" href={solscan("tx", done)} target="_blank" rel="noreferrer noopener">{done.slice(0, 10)}…</a></p> : null}
      <p className="small muted" style={{ marginBottom: 0 }}>Exact in, through Meteora DBC <span className="mono">swap</span>. Simulated on chain before your wallet is asked; the minimum out comes from that simulation. SOL is wrapped for the trade and unwrapped after it (any wSOL already in your wallet is unwrapped too).</p>
    </section>
  );
}

type Pay = "sol" | "wx";
type WxReview = { side: "buy" | "sell"; pay: Pay; amount: bigint; wxMin: bigint; received: bigint; minOut: bigint; solDelta: bigint; outDec: number; outSym: string };
/**
 * The ticket of a coin quoted in its collection's wX (canister-only Rook, docs/v9/canister-only-solana.md §3, §5.1): the site's OWN
 * two-hop buy, SOL → wX on the collection's wX/SOL pool (a Meteora DLMM pair the canister's pool desk runs, §17) → coin on its curve
 * (DBC `swap2`, exact in) or, graduated, in its DAMM v2 pool, in one transaction the wallet signs (src/rook/sol/route.ts, the
 * canister's `route::two_hop_buy`); the sell the other way; and the one-hop buy with wX the wallet already holds (the direct mint,
 * §6). Required, not a fallback: Jupiter drops a coin on its curve after its grace period and any wX whose pool is thin. Quoted here
 * from the on-chain accounts, both hops, then simulated on chain before the wallet is asked; the floors come from that simulation.
 * Prices in wX, with the SOL and USD view through the pair.
 */
function WxTicket({ c, sym, wxSym, wxp, wxpLoading, usdPer }: { c: BoardCoin; sym: string; wxSym: string; wxp: WxPool | null; wxpLoading: boolean; usdPer: number | null }) {
  const w = useSolWallet();
  const qc = useQueryClient();
  const now = useNow();
  const wx = c.cfg.quoteMint;
  const bal = useBalances(w.address, c.mint, wx);
  const s = poolStats(c.cfg, c.p);
  const grad = useMigratedPool(c.mint, wx, s.phase === "graduated");
  const [side, setSide] = useState<"buy" | "sell">("buy");
  const [pay, setPay] = useState<Pay>("sol");
  const [amount, setAmount] = useState("");
  const [slip, setSlip] = useState(100);
  const [review, setReview] = useState<WxReview | null>(null);
  const [step, setStep] = useState<SendStep | "reviewing" | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [done, setDone] = useState<string | null>(null);
  const reset = () => { setReview(null); setErr(null); setDone(null); };
  if (s.phase === "graduating") return (
    <section className="panel ticket"><p className="eyebrow">Trade</p><p>The curve is full ({solNum(lamportsToSol(s.thresholdLamports), 0)} {wxSym}). The coin is moving to its DAMM v2 {sym}/{wxSym} pool: Meteora's keeper, or anyone, triggers it. The curve takes no more trades; trading resumes here once it has moved.</p></section>
  );
  const graduated = s.phase === "graduated";
  const hop: CoinHop | null = graduated ? (grad.data ? { kind: "graduated", pool: grad.data.ref } : null) : { kind: "curve", curve: { pool: c.pool, config: c.p.config, baseMint: c.mint, baseVault: c.p.baseVault, quoteVault: c.p.quoteVault, quoteMint: wx } };
  const hopState: HopState | null = graduated ? (grad.data ? { kind: "graduated", pool: grad.data.pool } : null) : { kind: "curve", cfg: c.cfg, pool: c.p };
  const pair = wxp && wxp.pair.status === 0 ? wxp : null;
  const effPay: Pay = pair ? pay : "wx";
  const wxSol = wxPriceSol(pair);
  const inDec = side === "sell" ? c.cfg.tokenDecimal : 9;
  const inSym = side === "sell" ? sym : effPay === "sol" ? "SOL" : wxSym;
  const outDec = side === "buy" ? c.cfg.tokenDecimal : 9;
  const outSym = side === "buy" ? sym : pair ? "SOL" : wxSym;
  const amt = parseUnits(amount, inDec);
  // the quote of the path in use: two hops through the pair, or one hop (no pair: wX in or out of the wallet)
  const est: { out: bigint; wxMin: bigint; feeWx: bigint; feeSol: bigint | null } | null = (() => {
    if (!amt || !hopState) return null;
    if (side === "buy" && effPay === "sol") { const q = pair ? quoteTwoHopBuy(pair, hopState, amt, slip, now) : null; return q && { out: q.coinOut, wxMin: q.wxMin, feeWx: q.feeWx, feeSol: q.feeSol }; }
    if (side === "buy") { const q = quoteWxBuy(hopState, amt, slip, now); return q && { out: q.coinOut, wxMin: 0n, feeWx: q.feeWx, feeSol: null }; }
    if (pair) { const q = quoteTwoHopSell(pair, hopState, amt, slip, now); return q && { out: q.solOut, wxMin: q.wxMin, feeWx: q.feeWx, feeSol: q.feeSol }; }
    const q = quoteWxSell(hopState, amt, slip, now);
    return q && { out: q.wxOut, wxMin: q.wxMin, feeWx: q.feeWx, feeSol: null };
  })();
  const estOut = est ? est.out : null;
  const estWxMin = est ? est.wxMin : 0n;
  const feeWx = est ? est.feeWx : null;
  const feeSol = est ? est.feeSol : null;
  const poolRate = pair ? Number(currentFeeRate(pair.pair, now)) / 1e7 : null;
  const build = (a: bigint, wxMin: bigint) => (min: bigint, unwrap: boolean) => {
    const user = w.address!;
    if (!hop) throw new Error("the pool is not read yet");
    if (side === "buy") return effPay === "sol" && pair ? twoHopBuyIxs(user, pair.lb, hop, a, wxMin, min, pair.arraysUp, unwrap) : wxBuyIxs(user, hop, a, min);
    if (pair) return twoHopSellIxs(user, pair.lb, hop, a, wxMin, min, pair.arraysDown, unwrap);
    return wxSellIxs(user, hop, a, min);
  };
  const tooMuch = amt && bal.data ? (side === "sell" ? amt > bal.data.coin : effPay === "sol" ? amt + 25_000_000n > bal.data.sol : amt > bal.data.wx) : false;
  const why = !w.address ? "Connect a wallet to trade."
    : graduated && !grad.data ? (grad.isLoading ? "Reading the pool…" : "The coin's DAMM v2 pool could not be read.")
    : !amt ? `Enter an amount of ${inSym}.`
    : tooMuch ? (side === "sell" ? `More ${sym} than you hold.` : effPay === "sol" ? "More than your SOL (keep ≈ 0.025 SOL for fees and accounts)." : `More ${wxSym} than you hold.`)
    : !est ? (side === "buy" && effPay === "sol" ? `The ${wxSym}/SOL pool cannot fill this amount: try less, or buy with ${wxSym}.` : side === "sell" && pair ? `The curve or the ${wxSym}/SOL pool cannot fill this amount.` : "The curve cannot fill this amount.")
    : null;
  const doReview = async () => {
    reset(); setStep("reviewing");
    try {
      const user = w.address!;
      const out = side === "buy" ? ata(user, c.mint) : pair ? ata(user, NATIVE_MINT) : ata(user, wx);
      const r = await boundBySimulation(rpc(), user, out, build(amt!, estWxMin), slip);
      setReview({ side, pay: effPay, amount: amt!, wxMin: estWxMin, received: r.received, minOut: r.minOut, solDelta: r.solDelta, outDec, outSym });
    } catch (e) { setErr(String((e as Error).message ?? e).slice(0, 240)); }
    setStep(null);
  };
  const doSend = async () => {
    if (!review) return;
    setErr(null);
    try {
      const sig = await simulateAndSend(rpc(), { ...w, address: w.address! }, build(review.amount, review.wxMin)(review.minOut, true), { onStep: setStep });
      setDone(sig); setReview(null); setAmount("");
      await qc.invalidateQueries();
    } catch (e) {
      const m = String((e as Error).message ?? e);
      setErr(/reject|denied|cancel/i.test(m) ? "You declined it in your wallet. Nothing was sent." : m.slice(0, 240));
    }
    setStep(null);
  };
  const priceWx = s.priceSol, priceSol = wxSol === null ? null : priceWx * wxSol;
  return (
    <section className="panel ticket" aria-label="Trade">
      <div className="ticket__tabs" role="group" aria-label="Buy or sell">{(["buy", "sell"] as const).map((x) => <button key={x} type="button" aria-pressed={side === x} onClick={() => { setSide(x); setAmount(""); reset(); }}>{x === "buy" ? (pair ? "Buy with SOL" : "Buy") : pair ? "Sell for SOL" : "Sell"}</button>)}</div>
      {side === "buy" && pair ? <div className="ticket__prefs small"><span className="muted">Pay with</span><span className="seg">{(["sol", "wx"] as Pay[]).map((p) => <button key={p} type="button" aria-pressed={pay === p} onClick={() => { setPay(p); setAmount(""); reset(); }}>{p === "sol" ? "SOL" : wxSym}</button>)}</span></div> : null}
      <label className="ticket__in"><input inputMode="decimal" placeholder="0.0" value={amount} aria-label={`${inSym} to ${side === "buy" ? "spend" : "sell"}`} onChange={(e) => { setAmount(e.target.value); reset(); }} /><span>{inSym}</span></label>
      {w.address && bal.data ? <p className="small muted" style={{ margin: ".25rem 0" }}>You have {units(bal.data.sol, 9, 4)} SOL · {units(bal.data.wx, 9, 4)} {wxSym} · {units(bal.data.coin, c.cfg.tokenDecimal, 2)} {sym}{side === "sell" && bal.data.coin > 0n ? <> · <button type="button" className="btn btn--quiet btn--small" onClick={() => { setAmount(units(bal.data!.coin, c.cfg.tokenDecimal, c.cfg.tokenDecimal).replace(/,/g, "")); reset(); }}>Max</button></> : null}</p> : null}
      <div className="ticket__line"><span className="ticket__label">Price</span><span className="ticket__value"><span title={`${priceText(priceWx)} ${wxSym}`}>{priceShort(priceWx)}</span> {wxSym}{priceSol !== null ? <small> ≈ {priceShort(priceSol)} SOL{usdPer ? ` ≈ $${(priceSol * usdPer).toPrecision(3)}` : ""}</small> : null}</span></div>
      <div className="ticket__line"><span className="ticket__label">You get (estimate)</span><span className="ticket__value">{estOut !== null ? `${units(estOut, outDec, side === "buy" ? 0 : 6)} ${outSym}` : "—"}</span></div>
      {est && estWxMin > 0n && (side === "sell" || effPay === "sol") ? <div className="ticket__line"><span className="ticket__label">{side === "buy" ? `Through ${wxSym} (at least)` : `${wxSym} the curve pays (at least)`}</span><span className="ticket__value">{units(estWxMin, 9, 6)} {wxSym}{wxSol !== null ? <small> at ≈ {solNum(wxSol, 3)} SOL</small> : null}</span></div> : null}
      <div className="ticket__line"><span className="ticket__label">Fees now</span><span className="ticket__value">{feeWx !== null ? `${units(feeWx, 9, 6)} ${wxSym}` : "—"}{feeSol !== null && feeSol > 0n ? <small> + {units(feeSol, 9, 6)} SOL pool fee{poolRate !== null ? ` (${poolRate.toFixed(2)} %)` : ""}</small> : null}</span></div>
      <div className="ticket__prefs small"><span className="muted">Slippage</span><span className="seg">{SLIPS.map((b) => <button key={b} type="button" aria-pressed={slip === b} onClick={() => { setSlip(b); reset(); }}>{b / 100} %</button>)}</span><small className="muted">per hop</small></div>
      {review ? (
        <div className="ticket__total">
          <div className="ticket__line"><span className="ticket__label">Simulated on chain: you receive</span><span className="ticket__value">{units(review.received, review.outDec, 6)} {review.outSym}</span></div>
          <div className="ticket__line"><span className="ticket__label">At least (slippage {slip / 100} %)</span><span className="ticket__value">{units(review.minOut, review.outDec, 6)}</span></div>
          <div className="ticket__line"><span className="ticket__label">Your SOL moves by</span><span className="ticket__value">{solText(review.solDelta, 6)} SOL<small> before unwrapping and the network fee</small></span></div>
        </div>
      ) : null}
      <div className="ticket__go">
        {!w.address ? <SolWalletButton /> : review
          ? <button className="btn btn--primary btn--block" disabled={!!step} onClick={() => void doSend()}>{step === "simulating" ? "Simulating…" : step === "signing" ? "Confirm in your wallet…" : step === "sending" ? "Sending…" : step === "confirming" ? "Confirming…" : `Confirm ${review.side} in wallet`}</button>
          : <button className="btn btn--primary btn--block" disabled={!!why || !!step} onClick={() => void doReview()}>{step === "reviewing" ? "Simulating…" : side === "buy" ? "Review buy" : "Review sell"}</button>}
      </div>
      {why && w.address ? <p className="muted small" style={{ marginTop: ".5rem" }}>{why}</p> : null}
      {err ? <p className="error small" role="alert">{err}</p> : null}
      {done ? <p className="good small" role="status">Confirmed. <a className="mono" href={solscan("tx", done)} target="_blank" rel="noreferrer noopener">{done.slice(0, 10)}…</a></p> : null}
      {pair ? (
        <p className="small muted" style={{ marginBottom: 0 }}>{side === "buy" && effPay === "sol" ? <>Two hops in one transaction, built by this site: SOL → {wxSym} on the collection's {wxSym}/SOL pool (<Addr id={pair.lb.pair} />, Meteora DLMM, run by Rook's pool desk), then exactly that {wxSym} → {sym} {graduated ? <>in its DAMM v2 pool</> : <>on its curve (DBC <span className="mono">swap2</span>)</>}. </> : side === "buy" ? <>One hop: {wxSym} from your wallet → {sym} {graduated ? "in its DAMM v2 pool" : "on its curve"}. </> : <>Two hops in one transaction: {sym} → {wxSym} {graduated ? "in its pool" : "on its curve"}, then exactly the {wxSym} promised → SOL on the {wxSym}/SOL pool; a sliver of {wxSym} may stay in your wallet. </>}It works with or without Jupiter. Simulated on chain before your wallet is asked; the minimum out comes from that simulation. SOL is wrapped for the trade and unwrapped after it.{graduated ? <> Also on <a href={jupiter(c.mint)} target="_blank" rel="noreferrer noopener">Jupiter ↗</a> where it routes.</> : null}</p>
      ) : (
        <p className="small muted" style={{ marginBottom: 0 }}>{wxpLoading ? `Reading the ${wxSym}/SOL pool…` : <>No {wxSym}/SOL pool yet: a collection's pool opens and grows from its fees (busiest first). Until then: buy with {wxSym} you hold (from the direct mint with Rook), and sells pay {wxSym} into your wallet, redeemable with Rook for SOL. {wxp && wxp.pair.status !== 0 ? "The pool exists but is disabled by Meteora's operator." : null}</>} Simulated on chain before your wallet is asked.</p>
      )}
    </section>
  );
}

/**
 * The ticket under `?demo`: the same estimate from the curve math (pure arithmetic on the fixture pool), and nothing more. It reads
 * no wallet, builds no instruction, simulates nothing and sends nothing; its button is disabled and says so.
 */
function DemoTicket({ c, sym, quote }: { c: BoardCoin; sym: string; quote: string }) {
  const now = useNow();
  const [side, setSide] = useState<"buy" | "sell">("buy");
  const [amount, setAmount] = useState("");
  const s = poolStats(c.cfg, c.p);
  const head = <p className="eyebrow">Trade <Pill tone="warn">demo</Pill></p>;
  if (s.phase === "graduated") return (
    <section className="panel ticket">{head}<p>This demo coin has graduated: a real one trades in its locked Meteora DAMM v2 pool{quote === "SOL" ? ", through Jupiter" : `, ${sym}/${quote}, through this page's SOL route`}.</p>
      <button type="button" className="btn btn--primary btn--block" disabled>Demo: no trade on an invented coin</button></section>
  );
  if (s.phase === "graduating") return <section className="panel ticket">{head}<p>The curve is full ({solNum(lamportsToSol(s.thresholdLamports), 0)} {quote}) and the coin is moving to its DAMM v2 pool. A real coin takes no more curve trades at this point.</p></section>;
  const decimals = side === "buy" ? 9 : c.cfg.tokenDecimal, outDec = side === "buy" ? c.cfg.tokenDecimal : 9;
  const amt = parseUnits(amount, decimals);
  const est = amt ? quoteSwap(c.cfg, c.p, side, amt, BigInt(now)) : null;
  return (
    <section className="panel ticket" aria-label="Trade (demo)">
      {head}
      <div className="ticket__tabs" role="group" aria-label="Buy or sell">{(["buy", "sell"] as const).map((x) => <button key={x} type="button" aria-pressed={side === x} onClick={() => { setSide(x); setAmount(""); }}>{x === "buy" ? "Buy" : "Sell"}</button>)}</div>
      <label className="ticket__in"><input inputMode="decimal" placeholder="0.0" value={amount} aria-label={side === "buy" ? `${quote} to spend` : `${sym} to sell`} onChange={(e) => setAmount(e.target.value)} /><span>{side === "buy" ? quote : sym}</span></label>
      <div className="ticket__line"><span className="ticket__label">You would get (estimate)</span><span className="ticket__value">{est ? `${units(est.amountOut, outDec, side === "buy" ? 0 : 6)} ${side === "buy" ? sym : quote}` : "—"}</span></div>
      <div className="ticket__line"><span className="ticket__label">Fee now</span><span className="ticket__value">{est ? `${units(est.fee, 9, 6)} ${quote} (${(Number(est.feeNumerator) / 1e7).toFixed(2)} %)` : "—"}</span></div>
      <div className="ticket__go"><button type="button" className="btn btn--primary btn--block" disabled>Demo: nothing is simulated or sent</button></div>
      <p className="small muted" style={{ marginBottom: 0 }}>Demo ticket, in the curve's own {quote}. The estimate is the curve's arithmetic on an invented pool; no wallet is asked, no transaction is built, simulated or sent. On a real coin this ticket {quote === "SOL" ? "simulates the trade on chain before your wallet is asked" : `also takes SOL, swapped into ${quote} on the collection's pool in the same transaction, simulated on chain before your wallet is asked`}.</p>
    </section>
  );
}

/** A collection coin's backing (docs page "Backing and wrapped tokens"): the coin's vault from the custodian's books when the build
 *  names the canister, and what the curve's own config says of the backing share: half the raise, in the collection's wX, paid to
 *  the claimer (a key only the custodian signs with) at graduation and moved by the custodian into the coin's vault. */
function BackingPanel({ c, sym }: { c: BoardCoin; sym: string }) {
  const col = collectionBySlug(c.slug);
  const { coll: canisterColl } = useCanisterIndex(col?.wxSymbol); // the canister's index: its unlock order, not the site's listing order
  const wx = col?.wxSymbol ?? "wX";
  const s = poolStats(c.cfg, c.p);
  const share = (c.cfg.migrationQuoteThreshold * BigInt(c.cfg.migrationFeePercentage)) / 100n;
  const taken = (c.p.migrationFeeWithdrawStatus & PARTNER_MIGRATION_FEE_TAKEN) !== 0;
  return (
    <section className="panel">
      <p className="eyebrow">Backing{col ? ` · ${col.wxSymbol}` : ""}</p>
      {col && CANISTER_ON && canisterColl !== null ? <CoinBacking coll={canisterColl} mint={c.mint} symbol={sym} wx={col.wxSymbol} decimals={c.cfg.tokenDecimal} />
        : DEMO ? <p className="small muted">Demo: no books. On the live site this panel shows the coin's vault from the custodian's books: its {wx}, the coins redeemed, and what one coin is worth in {wx}.</p>
        : <p className="small muted">This build names no Rook canister yet: the coin's vault (its {wx}, the coins redeemed, what one coin is worth) is read from the custodian's books once it does.</p>}
      <dl className="kv" style={{ marginTop: ".75rem" }}>
        <dt>Backing share at graduation</dt><dd>{solText(share, 2)} {wx} ({c.cfg.migrationFeePercentage} % of the raise, already {wx}: one whole unit is one NFT in custody)</dd>
        <dt>Status</dt><dd>{s.phase === "curve" ? "not graduated yet: the share goes straight into the coin's vault at graduation" : taken ? "moved into the coin's vault by the custodian" : `${solText(share, 2)} ${wx} waiting in the curve for the custodian to move it into the coin's vault (a key only it signs with can)`}</dd>
        <dt>Rook's curve fees accrued</dt><dd>{solText(c.p.partnerQuoteFee, 4)} {wx} <small className="muted">(Rook's 80 % share, in {wx}: it builds the collection's pool and reserves)</small></dd>
      </dl>
    </section>
  );
}
/** ROOK, the genesis coin (docs page "Fees"): SOL-paired, no backing, its claimer the custodian's wallet (docs/v9/canister-only-solana.md §16). */
function RookNote() {
  return (
    <section className="panel">
      <p className="eyebrow">ROOK, the genesis coin</p>
      <p style={{ marginTop: 0 }}>ROOK has <strong>no NFT backing</strong> and no rights attached. It trades against SOL, and its fee claimer is the custodian's wallet: Rook's 80 % share of every ROOK curve fee pays Rook's running costs (<a href={href("docs", "fees")}>Fees</a>). At 85 SOL the whole raise goes into its locked DAMM v2 pool.</p>
      <p className="small muted" style={{ margin: 0 }}>Buy it only for its own sake. A purchase is not a contribution with any claim, and nothing is promised.</p>
    </section>
  );
}
