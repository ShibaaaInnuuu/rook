import { useEffect, useMemo, useState } from "react";
import { href, useEnv, useNow } from "../lib/env";
import { DRAND_RELAYS, fetchRound, timeOf } from "../rook/v8/core/drand";
import { dur, verifyDraw } from "../rook/v8/core/draw";
import { fracWords, pctWords } from "../rook/v8/core/odds";
import type { DrawProof, DrawReport, DrawVerdict, MetadataSnapshot, Odds } from "../rook/v8/core/types";
import { HOW_IT_WORKS, TIMING_WORD, headline, oddsFor, snapshotCheck, verifyAll, type ProofRow } from "../rook/v8/present";
import { checkDraw, DEFAULT_ETH_RPCS, DEFAULT_HOME_RPCS } from "../rook/v8/core/selfcheck";
import { useDraws, useSnapshot, useV8Source, type V8Src } from "../rook/v8/source";
import { FixtureBanner, Loading, NothingDeployed, Pill, ago } from "../ui/bits";
import "../styles/v8.css";

/**
 * docs/v8/redemption-draw.md §2.4: the redemption shown as the vault's composition, with the exact odds for the ticket a burn now
 * would get, and every past draw's proof checked in the visitor's browser. Wording per decision R1 (composition, equal odds) pending
 * the operator's legal review; no rarity rank here (R2); the 4663 timing flag on every draw (R4).
 */
export function DrawPage({ slug, ticket }: { slug?: string; ticket?: string }) {
  const env = useEnv(), s = useV8Source();
  if (env === null || s.isLoading) return <Loading what="where to read from" />;
  if (!s.data) return <NothingDeployed page={slug ? `draw/${slug}` : "draw"} />;
  return <>{s.data.kind === "fixture" ? <FixtureBanner note={s.data.note} /> : null}{slug ? <One slug={slug} ticket={ticket} src={s.data} /> : <Pick src={s.data} />}</>;
}

function Pick({ src }: { src: V8Src }) {
  const [slugs, setSlugs] = useState<string[] | null>(null);
  useEffect(() => { src.get<{ collections: string[] }>("/v1/v8").then((l) => setSlugs(l.collections)).catch(() => setSlugs([])); }, [src]);
  return (
    <header className="pagehead rise"><p className="eyebrow">Redemption</p><h1>What a redemption can give you</h1>
      <p className="lede">Pick a collection to see the NFTs a redemption can receive, the exact odds of each, and every past redemption's proof.</p>
      {slugs === null ? <Loading what="the collections" /> : <ul className="list">{slugs.map((x) => <li key={x}><a href={href("draw", x)}>{x}</a></li>)}</ul>}
    </header>
  );
}

function One({ slug, ticket, src }: { slug: string; ticket?: string; src: V8Src }) {
  const q = useDraws(slug), snap = useSnapshot(slug), now = useNow();
  const r = q.data;
  const [rows, setRows] = useState<ProofRow[] | null>(null);
  // the proofs are checked after the first paint: a BLS pairing each, some tens of milliseconds
  useEffect(() => { setRows(null); if (!r) return; const t = setTimeout(() => setRows(verifyAll(r)), 30); return () => clearTimeout(t); }, [r]);
  if (q.isLoading) return <Loading what="the vault" />;
  if (q.error || !r) return <div className="pagehead"><h1>No such collection</h1><p className="lede">Nothing is configured under “{slug}”. <a href={href("draw")}>All collections</a>.</p></div>;
  const { odds, why } = oddsFor(r, now);
  const name = r.name;
  const h = odds ? headline(odds, name) : { title: `${name}: what a redemption can give you`, body: "" };
  const focus = ticket !== undefined ? r.proofs.find((p) => String(p.ticket) === ticket) ?? null : null;
  return (
    <>
      <header className="pagehead rise">
        <p className="eyebrow"><a href={href("draw")}>Redemption</a> · <a href={href("reserves", slug)}>{name}: the reserves</a></p>
        <h1>{h.title}</h1>
        {h.body ? <p className="lede">{h.body}</p> : null}
        {odds ? <p className="small muted">For ticket {odds.forTicket}, the one a burn mined now would get · the vault read at Robinhood Chain block {r.state!.block.toLocaleString("en-US")}, {ago(r.state!.at, now)} · computed {odds.method === "monte-carlo" ? `by simulation (${odds.samples?.toLocaleString("en-US")} runs, standard error ${(odds.stderr! * 100).toFixed(2)} points)` : "exactly"} in your browser from the wrapper's own books</p> : null}
        {why ? <p className="notice notice--warn" role="status"><strong>Odds unavailable.</strong> {why}</p> : null}
      </header>

      {odds ? <Composition r={r} odds={odds} snap={snap.data ?? null} now={now} fixture={src.kind === "fixture"} /> : null}

      <section className="block rise rise-2">
        <p className="eyebrow">How it works</p>
        <h2>No one picks the item, and no one can know it in advance</h2>
        <ol className="v8-how">{HOW_IT_WORKS.map((x, i) => <li key={i}>{x}</li>)}</ol>
      </section>

      <section className="block rise rise-3" id="past">
        <p className="eyebrow">Every past redemption, checked here</p>
        <h2>{rows ? `${rows.filter((x) => x.verdict.verdict === "verified").length} of ${rows.length} proofs verified in your browser` : `${r.proofs.length} proofs, being checked in your browser…`}</h2>
        <p className="small muted v8-measure">For each: drand's BLS signature of the pinned round, checked against the evmnet network's public key; the round against the burn's own timestamp; the set of NFTs the wrapper drew from; the index the formula gives; and when the burn's block reached Ethereum (a flag if the round was public well before it: the chain's sequencer may date a block up to 24 hours back).</p>
        <ProofTable r={r} rows={rows} now={now} slug={slug} />
        {focus ? <ProofDetail r={r} p={focus} row={rows?.find((x) => x.ticket === focus.ticket) ?? null} src={src} now={now} /> : <p className="small muted">Open a ticket to see its proof, field by field, and check it against drand's relays yourself.</p>}
      </section>
    </>
  );
}

function Composition({ r, odds, snap, now, fixture }: { r: DrawReport; odds: Odds; snap: MetadataSnapshot | null; now: number; fixture: boolean }) {
  const tokens = useMemo(() => new Map(snap?.tokens.map((t) => [t.tokenId, t]) ?? []), [snap]);
  const check = useMemo(() => (snap && r.snapshot ? snapshotCheck(snap, r.snapshot.sha256) : null), [snap, r.snapshot]);
  const items = new Map(r.state!.items.map((i) => [i.itemId, i]));
  const promised = r.state!.items.filter((i) => i.state === "promised");
  const ahead = r.state!.pinned.filter((t) => t.id < odds.forTicket);
  const max = Math.max(...odds.items.map((i) => i.pct));
  return (
    <>
      <section className="block rise rise-1" style={{ marginTop: "1.5rem" }}>
        <div className="row"><p className="eyebrow" style={{ margin: 0 }}>The vault's composition</p>
          <span className="right small muted">{check ? (check.ok ? <>traits from the metadata snapshot of {new Date(r.snapshot!.takenAt * 1000).toISOString().slice(0, 10)}, <span className="good">hash checked here</span></> : <span className="bad">the snapshot does not match its published hash</span>) : "traits: the snapshot is not available"}</span></div>
        <ul className="v8-items">
          {odds.items.map((o) => {
            const it = items.get(o.itemId), t = tokens.get(o.tokenId);
            return (
              <li key={o.itemId} className="v8-item panel panel--tight">
                <div className="v8-item__head"><strong className="mono">#{o.tokenId}</strong><span className="small muted">{it?.how === "deposit" ? "deposited by a holder" : "bought"}{it?.state === "pending" ? " · arrived after the last draw" : ""}</span></div>
                <div className="v8-odds"><span className="num">{fracWords(o.p)}</span><span className="small muted num">{pctWords(o.pct)}</span></div>
                <div className="v8-bar" aria-hidden="true"><span style={{ width: `${Math.max(4, (o.pct / max) * 100)}%` }} /></div>
                {t ? <dl className="v8-traits">{Object.entries(t.traits).sort(([a], [b]) => a.localeCompare(b)).map(([k, v]) => <div key={k}><dt>{k}</dt><dd>{v}</dd></div>)}</dl> : <p className="small muted">not in the snapshot</p>}
              </li>
            );
          })}
        </ul>
        <div className="grid" style={{ marginTop: "1.25rem" }}>
          <div className="span-6 small">
            {ahead.length ? <p className="notice notice--plain" style={{ margin: 0 }}>{ahead.map((t) => <span key={t.id} style={{ display: "block" }}><strong>Ticket {t.id}</strong> was burned {ago(t.burnedAt, now)} and draws first. {timeOf(t.round) > now ? `Its round, ${t.round.toLocaleString("en-US")}, is published in ${dur(timeOf(t.round) - now)}` : `Its round, ${t.round.toLocaleString("en-US")}, is published`}; anyone may then resolve it.</span>)}</p>
              : <p className="muted" style={{ margin: 0 }}>No redemption is waiting ahead of yours.</p>}
          </div>
          <div className="span-6 small">
            {promised.length ? <p className="muted" style={{ margin: 0 }}>Already drawn and waiting for delivery, so not in your set: {promised.map((i) => `#${i.tokenId} (ticket ${i.promisedTo})`).join(", ")}.</p> : null}
            <p className="muted" style={{ margin: promised.length ? "0.5rem 0 0" : 0 }}>These odds are for a burn mined now. An NFT added, or a redemption burned, before yours is mined changes them; nothing added after your burn can reach you.</p>
          </div>
        </div>
        {r.snapshot ? <p className="small muted" style={{ marginTop: "1rem" }}>Snapshot sha256 <span className="mono break">{r.snapshot.sha256}</span>, {r.snapshot.tokenCount.toLocaleString("en-US")} tokens, pinned in {r.snapshot.pinnedIn}.{r.snapshot.drift && r.snapshot.drift.changed.length ? ` The project's metadata has changed for ${r.snapshot.drift.changed.length} traits since; the snapshot is used, and the changes are listed on the reserves page.` : ""}{fixture ? " The traits in this fixture are invented." : ""}</p> : null}
      </section>
    </>
  );
}

const verdictTone = (v: DrawVerdict["verdict"]) => (v === "verified" ? "good" : v === "mismatch" ? "bad" : "warn");
const timingTone = (l: DrawVerdict["timing"]["level"]) => (l === "flagged" ? "bad" : l === "unknown" ? "warn" : "good");

function ProofTable({ r, rows, now, slug }: { r: DrawReport; rows: ProofRow[] | null; now: number; slug: string }) {
  const by = new Map(rows?.map((x) => [x.ticket, x.verdict]) ?? []);
  return (
    <div className="scroll"><table className="v8-proofs">
      <thead><tr><th>Ticket</th><th>Burned</th><th>Round</th><th>Received</th><th>Proof</th><th>Timing</th><th>Delivered</th></tr></thead>
      <tbody>{r.proofs.map((p) => { const v = by.get(p.ticket); return (
        <tr key={p.ticket}>
          <td><a href={href("draw", slug, p.ticket)}><strong>{p.ticket}</strong></a>{p.toBurn ? <span className="small muted"> · burned for good</span> : null}</td>
          <td className="num small">{ago(p.burnAt, now)}</td>
          <td className="num small">{p.round.toLocaleString("en-US")}</td>
          <td className="mono">#{p.tokenId}</td>
          <td>{v ? <Pill tone={verdictTone(v.verdict)}>{v.verdict}</Pill> : <span className="small muted">checking…</span>}</td>
          <td>{v ? <Pill tone={timingTone(v.timing.level)}>{TIMING_WORD[v.timing.level]}</Pill> : null}</td>
          <td className="small">{p.delivery ? ago(p.delivery.at, now) : "awaiting"}</td>
        </tr>); })}</tbody>
    </table></div>
  );
}

function ProofDetail({ r, p, row, src, now }: { r: DrawReport; p: DrawProof; row: ProofRow | null; src: V8Src; now: number }) {
  const [relay, setRelay] = useState<{ state: "idle" | "asking" | "done"; verdict: DrawVerdict | null }>({ state: "idle", verdict: null });
  useEffect(() => setRelay({ state: "idle", verdict: null }), [p.ticket]);
  const [notes, setNotes] = useState<string[]>([]);
  // live: the relays AND Robinhood Chain (the Resolved event, the round, the set at the block before, the batch's posting);
  // fixture: the relays only (the fixture's chain is invented; its rounds are real)
  const ask = async () => {
    setRelay({ state: "asking", verdict: null }); setNotes([]);
    if (src.kind === "live") {
      const c = await checkDraw({ wrapper: r.wrapper, proof: p, homeRpcs: [...DEFAULT_HOME_RPCS], ethRpcs: [...DEFAULT_ETH_RPCS], fetch });
      setNotes(c.notes); setRelay({ state: "done", verdict: c.verdict });
    } else {
      const relays = await fetchRound(p.round, DRAND_RELAYS.slice(0, 2), fetch);
      setRelay({ state: "done", verdict: verifyDraw(p, { wrapper: r.wrapper, relays }) });
    }
  };
  const v = relay.verdict ?? row?.verdict ?? null;
  const tx = (h: string) => (src.kind === "fixture" || !r.links.tx ? <span className="mono break">{h.slice(0, 18)}…</span> : <a className="mono break" href={`${r.links.tx}${h}`} target="_blank" rel="noreferrer noopener">{h.slice(0, 18)}…</a>);
  return (
    <div className="panel v8-detail" style={{ marginTop: "1.25rem" }}>
      <div className="row"><h3 style={{ margin: 0 }}>Ticket {p.ticket}: #{p.tokenId}</h3>{v ? <Pill tone={verdictTone(v.verdict)}>{v.verdict}</Pill> : null}
        <span className="right"><button type="button" className="btn btn--small" onClick={ask} disabled={relay.state === "asking"}>{relay.state === "asking" ? "Asking…" : src.kind === "live" ? "Check against drand's relays and the chain" : "Check against drand's relays"}</button></span></div>
      <ol className="v8-timeline small">
        <li><strong>Burned</strong> {new Date(p.burnAt * 1000).toISOString().replace("T", " ").slice(0, 19)} UTC at block {p.burnBlock.toLocaleString("en-US")} · {tx(p.burnTx)}</li>
        <li><strong>Round {p.round.toLocaleString("en-US")}</strong> published {new Date(timeOf(p.round) * 1000).toISOString().replace("T", " ").slice(0, 19)} UTC{p.beacon.relays.length ? ` · served by ${p.beacon.relays.join(" and ")}` : ""}</li>
        <li><strong>Resolved</strong> {p.resolvedAt ? `${ago(p.resolvedAt, now)}` : ""} at block {p.resolveBlock.toLocaleString("en-US")} · {tx(p.resolveTx)} · NFT {p.itemId} of a set of {p.freeSet.length}</li>
        <li><strong>{p.delivery ? "Delivered" : "Awaiting delivery"}</strong> {p.delivery ? `${ago(p.delivery.at, now)} · ` : ""}{p.delivery ? (src.kind === "fixture" || !p.delivery.url ? <span className="mono">{p.delivery.tx.slice(0, 18)}…</span> : <a className="mono" href={p.delivery.url} target="_blank" rel="noreferrer noopener">{p.delivery.tx.slice(0, 18)}…</a>) : `to ${p.toBurn ? "the burn address" : p.destination}`}</li>
      </ol>
      {v ? <ul className="v8-checks">{v.checks.map((c) => <li key={c.id} className={c.ok === true ? "ok" : c.ok === false ? "bad" : "na"}><span className="v8-mark">{c.ok === true ? "✓" : c.ok === false ? "✗" : "–"}</span><span><strong>{c.label}</strong><br /><span className="small muted break">{c.detail}</span></span></li>)}</ul> : <Loading what="the checks" />}
      {v ? <p className={`notice ${v.timing.level === "flagged" ? "notice--bad" : v.timing.level === "unknown" ? "notice--warn" : "notice--plain"} small`}><strong>Timing ({TIMING_WORD[v.timing.level]}).</strong> {v.timing.text}.{p.posting?.note ? ` ${p.posting.note}.` : ""}</p> : null}
      {notes.length ? <ul className="small muted">{notes.map((n, i) => <li key={i}>{n}</li>)}</ul> : null}
      <details className="small"><summary>The set it drew from, in the wrapper's order ({p.freeSet.length} NFTs)</summary><p className="mono break">[{p.freeSet.join(", ")}]</p><p className="muted">{p.freeSetNote}.</p></details>
      <p className="small muted" style={{ marginBottom: 0 }}>From a terminal, with no Rook service: <code className="break">npm run -s verify -- draw --proof &lt;this proof&gt; --wrapper {r.wrapper}</code> in the repository's indexer/ (it also re-reads the Resolved event and the set from Robinhood Chain).{src.kind === "fixture" ? " In this fixture the chain is invented, so only drand's relays can be asked; the rounds are real." : ""}</p>
    </div>
  );
}
