import { href, useNow } from "../lib/env";
import { when } from "../lib/format";
import { overall, reportSummary, type Equation } from "../rook/sol/booksReserves";
import { b58encode } from "../rook/sol/base58";
import { findingKindOf, hexBytes, type BooksViewV, type CollectionViewV } from "../rook/sol/canister";
import { COLLECTIONS, collectionBySlug } from "../rook/sol/collections";
import { DEMO } from "../rook/sol/demo";
import { age, solscan, units } from "../rook/sol/format";
import { CANISTER_ON, coinsCollectionIndex, useDeskStatus, useReserves, useTreasuryStatus } from "../rook/sol/hooksCanister";
import { Fig, Loading, Pill, Status, StatusPill } from "../ui/bits";
import { Addr, CollectionLogo } from "../ui/sol";

/**
 * The Reserves page on the books home (docs/v9/canister-only-solana.md §10.2): for every collection the canister keeps books for, its
 * equation against Solana at finalized: wX supply = (NFTs in custody − redemptions in flight) + SOL-backed units; SOL in `M[i]` above its
 * rent ≥ what the books owe; each coin vault ≥ its book. Green when every rule holds, red when one fails, unverified when a read is
 * missing. The items' custody (the NFT at the custody address, the inscription at the custody taproot) is checked by the reconciler,
 * two sources each; the page lists every item's custody proof so anyone can. The canister's own last reconciliation is shown too.
 */
export function SolReserves({ slug }: { slug?: string }) {
  if (!CANISTER_ON) return <Header><p className="notice notice--warn">This build names no Rook canister yet (<span className="mono">src/config/canister.ts</span>): there are no books to check until it is installed. When it is, this page reads every collection's books from the canister's public queries and the chain at finalized, and shows where they agree and where they do not.</p></Header>;
  return <>{DEMO ? <p className="notice small" role="note"><strong>Demo.</strong> These books are invented and the chain side is not read: every figure below is the demo canister's, drawn as the live page draws the real canister's books against Solana at finalized.</p> : null}{slug ? <One slug={slug} /> : <All />}</>;
}

function Header({ children, lede }: { children?: React.ReactNode; lede?: React.ReactNode }) {
  return (
    <>
      <header className="pagehead rise"><p className="eyebrow">Reserves</p><h1>Check the reserves</h1>
        <p className="lede">{lede ?? <>For every collection: the NFTs the canister holds in custody, less redemptions in flight, plus the units backed by SOL in its module, must equal the wX in supply; the module must hold what the books say; each coin's vault must hold its book. Read from the canister's public queries and Solana at finalized, in your browser.</>}</p>
        <p className="small muted" style={{ margin: 0 }}>Each collection's wX (wMILADY, wBFROG, …) stands for one NFT in custody or its floor value in SOL: one whole token, one NFT. <a href={href("docs", "backing")}>How the backing works</a>.</p></header>
      {children}
    </>
  );
}

// the canister's collection index is its unlock order, not the site's listing order: the site's collection is found by its wX symbol
const ref = (coll: number, cols: CollectionViewV[]) => {
  const sym = cols.find((x) => x.index === coll)?.symbol;
  const c = (sym ? COLLECTIONS.find((x) => x.wxSymbol === sym) : null) ?? null;
  return { slug: c?.slug ?? `collection-${coll}`, name: c?.name ?? `Collection ${coll}`, wx: c?.wxSymbol ?? sym ?? "wX", chain: c?.chain ?? "Ethereum", c };
};

function All() {
  const q = useReserves(), now = useNow();
  const desk = useDeskStatus();
  const t = useTreasuryStatus();
  const eqs = q.data?.equations ?? [];
  const all = overall(eqs);
  return (
    <Header>
      {q.isLoading ? <Loading what="the books and the chain" /> : q.error ? <p className="error">The books could not be read: {String((q.error as Error).message).slice(0, 200)}</p> : null}
      {q.data ? <p className="rise rise-1"><Status status={all} /> <span className="small muted">{eqs.length} collection{eqs.length === 1 ? "" : "s"} on the books, checked {age(q.data.at, now)} ago in this browser.</span></p> : null}
      <ul className="nests rise rise-1">
        {eqs.map((e) => { const r = ref(e.coll, q.data!.cols); return (
          <li key={e.coll}><a className="nest" href={href("reserves", r.slug)}>
            <span className="nest__name"><span className="row" style={{ gap: ".5rem", alignItems: "center" }}>{r.c ? <CollectionLogo c={r.c} size={28} /> : null}{r.name}</span><small>{r.wx} · {e.reasons.length ? e.reasons[0].slice(0, 90) : "supply, module and vaults agree with the books"}</small></span>
            <Fig label={r.chain === "Bitcoin" ? "Ordinals" : "NFTs"} value={e.held.toString()} />
            <Fig label="In flight" value={e.openTickets.toString()} />
            <Fig label={`${r.wx} supply`} value={e.supply !== null ? units(e.supply, 9, 2) : "—"} />
            <span><StatusPill status={e.status} /></span>
          </a></li>); })}
      </ul>
      {q.data?.errors.length ? <ul className="small warn">{q.data.errors.map((x, i) => <li key={i}>{x}</li>)}</ul> : null}
      <section className="panel rise rise-2" style={{ marginTop: "1.25rem" }}>
        <p className="eyebrow">The canister's own check</p>
        {desk.data ? <CanisterReport desk={desk.data} now={now} /> : desk.isLoading ? <p className="small muted">Reading…</p> : <p className="small error">desk_status could not be read.</p>}
        {t.data ? <p className="small muted" style={{ margin: ".5rem 0 0" }}>Treasury: {units(t.data.treasury.reserves_pot, 9, 3)} SOL in the reserves pot, {units(t.data.treasury.unallocated, 9, 3)} SOL unallocated; {units(t.data.treasury.total_received, 9, 2)} SOL ever received, {units(t.data.treasury.total_to_reserves, 9, 2)} to reserves, {units(t.data.treasury.total_to_raffles, 9, 2)} to raffles, {units(t.data.treasury.total_to_ops, 9, 2)} to operations ({(t.data.treasury.reserves_bps / 100).toFixed(0)} / {(t.data.treasury.raffle_bps / 100).toFixed(0)} / {(t.data.treasury.ops_bps / 100).toFixed(0)} %).{t.data.owed.length ? ` Owed and in transit: ${t.data.owed.map((o) => ("Ops" in o ? `${units(o.Ops.lamports, 9, 3)} SOL to operations` : `${units(o.Surplus.lamports, 9, 3)} SOL surplus to collection ${o.Surplus.coll}`)).join(", ")}.` : ""}{t.data.off ? " The cranks are switched off." : ""}</p> : null}
      </section>
      <p className="small muted" style={{ marginTop: "1rem" }}>The operator's reconciler (<span className="mono">ops/src/booksreserves.ts</span>, open source) runs the same rules every 10 minutes with two Solana providers, and locates every item in custody on Ethereum, Robinhood Chain, Bitcoin and Solana by two sources. Anyone can run it against a public RPC.</p>
    </Header>
  );
}

function CanisterReport({ desk, now }: { desk: { report: [] | [import("../rook/sol/canister").ReportV]; last_reconcile: bigint; halted: [number, string][]; incidents: import("../rook/sol/canister").IncidentV[]; paused: boolean; open: number; funded: number; executing: number; needing_attention: number }; now: number }) {
  const r = desk.report.length ? desk.report[0] : null;
  const s = reportSummary(r, desk.last_reconcile, now);
  return (
    <>
      <p style={{ margin: 0 }}><StatusPill status={s.tone} /> <span className="small">{s.at ? `${when(s.at)} (${age(s.at, now)} ago${s.stale ? ", stale" : ""}): ` : ""}{s.line}</span></p>
      {r?.findings.length ? <ul className="small" style={{ margin: ".5rem 0 0", paddingLeft: "1.1rem" }}>{r.findings.map((f, i) => <li key={i}><Pill tone={findingKindOf(f) === "HolderBurn" || findingKindOf(f) === "Unbooked" ? "warn" : "bad"}>{findingKindOf(f)}</Pill> collection {f.coll}: books {f.books.toString()}, chain {f.chain.toString()}{f.note ? ` — ${f.note}` : ""}</li>)}</ul> : null}
      {r?.unexplained.length ? <p className="small bad" style={{ margin: ".5rem 0 0" }}>Unexplained memos on chain: {r.unexplained.join(", ")}.</p> : null}
      {desk.halted.length ? <p className="small bad" style={{ margin: ".5rem 0 0" }}>Stopped: {desk.halted.map(([c, why]) => `collection ${c} (${why})`).join("; ")}.</p> : null}
      {desk.incidents.length ? <details className="small" style={{ marginTop: ".5rem" }}><summary>Incidents ({desk.incidents.length})</summary><ul style={{ paddingLeft: "1.1rem" }}>{desk.incidents.map((x, i) => <li key={i}>{when(Number(x.at))}{x.coll.length ? ` · collection ${x.coll[0]}` : ""}: {x.what}</li>)}</ul></details> : null}
      <p className="small muted" style={{ margin: ".5rem 0 0" }}>Desk: {desk.open} intent{desk.open === 1 ? "" : "s"} open, {desk.funded} funded, {desk.executing} executing{desk.needing_attention ? `, ${desk.needing_attention} needing the operator` : ""}{desk.paused ? "; paused" : ""}.</p>
    </>
  );
}

function One({ slug }: { slug: string }) {
  const q = useReserves(), now = useNow();
  const c = collectionBySlug(slug) ?? null;
  const coll = c ? (q.data ? coinsCollectionIndex(q.data.cols, c.wxSymbol) : null) : /^collection-(\d+)$/.test(slug) ? Number(slug.slice(11)) : null;
  if (q.isLoading) return <Header><Loading what="the books and the chain" /></Header>;
  if (q.error) return <Header><p className="error">The books could not be read: {String((q.error as Error).message).slice(0, 200)}</p></Header>;
  const e = q.data?.equations.find((x) => x.coll === coll) ?? null;
  const v = coll !== null ? q.data?.views.get(coll) ?? null : null;
  if (!e || !v) return <Header><div className="notice notice--warn"><p style={{ margin: 0 }}>{c ? `The canister keeps no books for ${c.name} yet.` : `Nothing is on the books under “${slug}”.`} <a href={href("reserves")}>All collections</a>.</p></div></Header>;
  const r = ref(e.coll, q.data!.cols);
  return <Ledger e={e} v={v} r={r} now={now} at={q.data!.at} />;
}

function Ledger({ e, v, r, now, at }: { e: Equation; v: BooksViewV; r: ReturnType<typeof ref>; now: number; at: number }) {
  const item = r.chain === "Bitcoin" ? "inscription" : "NFT";
  const custody = "EthNft" in v.kind ? `0x${Array.from(v.kind.EthNft.custody, (b) => b.toString(16).padStart(2, "0")).join("")}` : "Inscriptions" in v.kind ? v.kind.Inscriptions.custody
    : "SolNft" in v.kind ? b58encode(Uint8Array.from(v.kind.SolNft.custodian)) : null;
  const nftBacked = e.held > e.openTickets ? (e.held - e.openTickets) : 0n;
  return (
    <>
      <header className="pagehead rise"><p className="eyebrow"><a href={href("reserves")}>Reserves</a> · {r.chain}</p><h1>{r.name}</h1>
        <p className="lede"><Status status={e.status} /> <span className="small muted">checked {age(at, now)} ago in this browser; books from the canister, chain at finalized.</span></p></header>
      {e.reasons.length ? <ul className={`notice ${e.status === "red" ? "notice--bad" : "notice--warn"} small`} style={{ paddingLeft: "2rem" }}>{e.reasons.map((x, i) => <li key={i}>{x}</li>)}</ul> : null}
      <section className="panel rise rise-1">
        <p className="eyebrow">The equation</p>
        <div className="figs">
          <Fig label={`${item[0].toUpperCase()}${item.slice(1)}s in custody`} value={e.held.toString()} unit={`${e.itemsListed} listed`} />
          <Fig label="Redemptions in flight" value={e.openTickets.toString()} />
          <Fig label="SOL-backed units" value={units(e.solBacked, 9, 4)} unit={r.wx} />
          <Fig label={`${r.wx} supply on chain`} value={e.supply !== null ? units(e.supply, 9, 4) : "—"} unit={e.expected !== null ? `books: ${units(e.expected, 9, 4)}` : "books inconsistent"} />
        </div>
        <p className="small" style={{ margin: ".75rem 0 0" }}>({e.held.toString()} − {e.openTickets.toString()}) × 1 + {units(e.solBacked, 9, 4)}{e.moduleUnits > 0n ? ` − ${units(e.moduleUnits, 9, 4)} held by the module` : ""} = <strong>{e.expected !== null ? units(e.expected, 9, 4) : "?"} {r.wx}</strong>; the mint's supply is <strong>{e.supply !== null ? units(e.supply, 9, 4) : "unread"}</strong>. {e.supplyHolds === true ? <span className="good">They agree.</span> : e.supplyHolds === false ? <span className="bad">They do not.</span> : <span className="warn">Unverified.</span>}</p>
        <dl className="kv small" style={{ marginTop: ".75rem" }}>
          <dt>Module {e.module ? <Addr id={e.module} /> : "(address not published yet)"}</dt><dd>{e.balance !== null ? `${units(e.balance, 9, 4)} SOL above rent` : "unread"}<small className="muted"> must hold {units(e.owed, 9, 4)} SOL = backing {units(v.backing, 9, 3)} + reserve {units(v.reserve, 9, 3)} + surplus {units(v.surplus, 9, 3)} + revenue {units(v.revenue, 9, 3)}</small></dd>
          <dt>Unbooked at the module</dt><dd>{units(e.unbooked, 9, 4)} SOL<small className="muted"> never counted as books; swept to the surplus at a reconcile</small></dd>
          <dt>Backing per SOL-backed unit</dt><dd>{e.backingPerUnit !== null ? `${units(e.backingPerUnit, 9, 4)} SOL` : "—"}<small className="muted"> the most a burn is paid from the pot, per {r.wx}</small></dd>
          <dt>Accepted floor</dt><dd>{units(v.accepted, 9, 3)} SOL<small className="muted"> posted {when(Number(v.posted_at))}{v.fresh ? ", fresh" : v.escaped ? ", stale over 7 days" : ", stale"}</small></dd>
          <dt>NFT-backed · SOL-backed</dt><dd>{nftBacked.toString()} · {units(e.solBacked, 9, 2)} {r.wx}{v.reserve_units > 0n ? <small className="muted"> ({units(v.reserve_units, 9, 2)} of them in the standing reserve)</small> : null}</dd>
          <dt>wX mint</dt><dd><Addr id={e.wxMint} kind="token" /></dd>
          {custody ? <><dt>Custody{r.chain === "Solana" ? " (the canister's wallet)" : ""}</dt><dd>{r.chain === "Bitcoin" ? <a className="mono break" href={`https://mempool.space/address/${custody}`} target="_blank" rel="noreferrer noopener">{custody}</a>
            : r.chain === "Solana" ? <a className="mono break" href={`https://solscan.io/account/${custody}`} target="_blank" rel="noreferrer noopener">{custody}</a>
            : <a className="mono break" href={`${r.chain === "Robinhood Chain" ? "https://explorer.mainnet.chain.robinhood.com" : "https://etherscan.io"}/address/${custody}`} target="_blank" rel="noreferrer noopener">{custody}</a>}</dd></> : null}
        </dl>
      </section>
      <section className="panel rise rise-2" style={{ marginTop: "1.25rem" }}>
        <p className="eyebrow">Every {item} held ({v.items.length})</p>
        {v.items.length ? <div className="scroll"><table className="tape small">
          <thead><tr><th>#</th><th>{r.chain === "Bitcoin" ? "Inscription" : r.chain === "Solana" ? "Mint" : "Token id"}</th><th>Source</th><th>How it came</th></tr></thead>
          <tbody>{v.items.map((it) => <tr key={it.item_id}><td className="num">{it.item_id}</td><td className="num">{r.chain === "Bitcoin" && it.inscription_id ? <a className="mono" href={`https://ordinals.com/inscription/${it.inscription_id}`} target="_blank" rel="noreferrer noopener">{it.inscription_id.slice(0, 8)}…{it.inscription_id.slice(-4)} ↗</a> : <span className="mono" title={it.key}>{r.chain === "Bitcoin" ? `${it.key.slice(0, 8)}…` : r.chain === "Solana" ? b58encode(hexBytes(it.key)) : BigInt(`0x${it.key}`).toString()}</span>}</td><td className="num"><span className="mono" title={it.source_tx}>{it.source_tx.slice(0, 8)}…{r.chain === "Bitcoin" ? `:${it.vout}` : ""}</span></td><td>{it.deposit ? "deposited by a holder" : "bought by the canister"}</td></tr>)}</tbody>
        </table></div> : <p className="small muted" style={{ margin: 0 }}>None yet: every {r.wx} in supply is backed by SOL in the module.</p>}
        {v.tickets_open.length ? <><p className="eyebrow" style={{ marginTop: "1rem" }}>Redemptions in flight ({v.tickets_open.length})</p><div className="scroll"><table className="tape small"><thead><tr><th>Ticket</th><th>Stage</th><th>Item</th><th>Destination</th><th>Deliver by</th></tr></thead>
          <tbody>{v.tickets_open.map((t) => <tr key={t.id.toString()}><td className="num">#{t.id.toString()}</td><td><Pill tone={t.undeliverable ? "bad" : t.stage === "Resolved" ? "live" : undefined}>{t.stage}{t.undeliverable ? " · undeliverable" : ""}</Pill></td><td className="num">{t.item_id}</td><td className="num"><span className="mono break">{t.destination}</span></td><td className="num">{t.deliver_by > 0n ? when(Number(t.deliver_by)) : "—"}</td></tr>)}</tbody></table></div></> : null}
        <p className="small muted" style={{ margin: ".75rem 0 0" }}>Whether each {item} is at the custody address is the reconciler's check (two sources: {r.chain === "Bitcoin" ? "ord and a second indexer" : r.chain === "Solana" ? "two Solana providers, the custodian's token account for each mint" : r.chain === "Robinhood Chain" ? "two Robinhood Chain providers" : "two Ethereum providers"}); this page shows the books' custody proofs so anyone can look.</p>
      </section>
      <section className="panel rise rise-3" style={{ marginTop: "1.25rem" }}>
        <p className="eyebrow">Coin vaults ({e.vaults.length})</p>
        {e.vaults.length ? <div className="scroll"><table className="tape small"><thead><tr><th>Coin</th><th>Vault</th><th className="r">Book</th><th className="r">On chain</th><th>Stage</th><th></th></tr></thead>
          <tbody>{e.vaults.map((x) => <tr key={x.coin}><td className="num"><a className="mono" href={href("coin", x.coin)}>{x.coin.slice(0, 4)}…{x.coin.slice(-4)}</a></td><td className="num">{x.vault ? <a className="mono" href={solscan("account", x.vault)} target="_blank" rel="noreferrer noopener">{x.vault.slice(0, 4)}…{x.vault.slice(-4)} ↗</a> : "—"}</td><td className="r num">{units(x.book, 9, 4)} {r.wx}</td><td className="r num">{x.onChain !== null ? `${units(x.onChain, 9, 4)} ${r.wx}` : "—"}</td><td>{x.stage}</td><td><StatusPill status={x.status} /></td></tr>)}</tbody></table></div>
          : <p className="small muted" style={{ margin: 0 }}>No graduated coin on this collection yet.</p>}
      </section>
    </>
  );
}
