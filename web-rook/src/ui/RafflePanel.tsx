/** The holder raffle on the Solana pages: a collection's panel, a coin page's compact line, the front page's list. Data:
 *  src/rook/sol/hooksRaffle.ts (the canister's `raffle_status`, `books_view` and `raffle_snapshot`, or the demo fixtures). */
import { useState, type SyntheticEvent } from "react";
import { href, useNow } from "../lib/env";
import { collectionBySlug } from "../rook/sol/collections";
import { short, solscan, solText, units } from "../rook/sol/format";
import { DEMO } from "../rook/sol/hooksLite";
import { RAFFLES_READABLE, useRaffle, useRaffles, useRaffleSnapshot } from "../rook/sol/hooksRaffle";
import { biggestPots, dateText, leafShare, nextDraw, PHASE_LABEL, prizeProgress, type RaffleView, type Snapshot } from "../rook/sol/raffle";
import { Pill } from "./bits";
import { CollectionLogo } from "./sol";

const pct = (p: number) => `${Math.floor(p * 100)}%`;
const errText = (e: unknown) => String((e as Error).message ?? e).slice(0, 120);

function Progress({ v }: { v: RaffleView }) {
  const p = v.raffle ? prizeProgress(v.raffle.pot, v.price) : null;
  return (
    <>
      <span className="progress progress--live" role="progressbar" aria-valuemin={0} aria-valuemax={100} aria-valuenow={p === null ? 0 : Math.round(p * 100)}><i className={p === 1 ? "done" : undefined} style={{ width: p === null ? "0%" : pct(p) }} /></span>
      <p className="small muted" style={{ margin: ".25rem 0 0" }}>{p === null ? "The next prize's price is the public mint price: known once the collection's floor is posted and minting is open." : p >= 1 ? `The pot can buy the next prize now (${solText(v.price!, 3)} SOL, the public mint price).` : `${pct(p)} of the next prize (${solText(v.price!, 3)} SOL, the public mint price now).`}</p>
    </>
  );
}

const tone = (v: RaffleView) => (v.raffle?.phase === "drawn" ? "good" : v.raffle?.phase === "committed" ? "warn" : "live") as "good" | "warn" | "live";

export function HowRaffleWorks() {
  return (
    <details className="small">
      <summary>How it works</summary>
      <ul style={{ paddingLeft: "1.1rem" }}>
        <li><strong>Weekly, per collection.</strong> Every holder of the collection's graduated coins is in the draw; no ticket to buy.</li>
        <li><strong>Weighted by your share of the backing.</strong> Your weight is the sum, over the collection's graduated coins, of ⌊your balance × the coin's wX backing ÷ its circulating supply⌋. Pool vaults, the canister's own accounts and burned coins are left out.</li>
        <li><strong>The prize is one whole wrapped NFT token</strong> (one wX: one NFT in custody), bought from the pot at the public mint price.</li>
        <li><strong>The pot</strong> is fed by 50 % of the collection's reserve overflow: what its treasury share brings above the collection's reserve cap.</li>
        <li><strong>Auditable.</strong> The snapshot (slot, coins, excluded accounts, every holder's weight, the Merkle root) is published by the canister before it is committed (this panel shows it), so anyone can recompute the root from chain data. The randomness is a drand round pinned after the commit; the canister checks drand's signature before drawing.</li>
        <li><strong>Claim.</strong> The prize goes to the winner's wallet (Rook's canister sends the claim and pays the token account). Unclaimed after 30 days, it rolls to the next round.</li>
      </ul>
    </details>
  );
}

/** The last claim the canister's raffle remembers (`winner`, `claimed_at`): there is no per-round history query. */
function LastWinner({ v, symbol }: { v: RaffleView; symbol: string }) {
  const r = v.raffle!;
  return (
    <div>
      <p className="eyebrow" style={{ margin: ".5rem 0 .25rem" }}>Last winner</p>
      {r.winner ? <p className="small" style={{ margin: 0 }}><a className="mono" href={solscan("account", r.winner)} target="_blank" rel="noreferrer noopener" title={r.winner}>{short(r.winner)}</a>{r.claimedAt > 0 ? ` · claimed ${dateText(r.claimedAt).slice(0, 10)}` : " · claim pending"} · 1 {symbol}{r.prizesClaimed > 1n ? <span className="muted"> · {String(r.prizesClaimed)} prizes claimed so far</span> : null}</p>
        : <p className="small muted" style={{ margin: 0 }}>No prize claimed yet.</p>}
    </div>
  );
}

function SnapshotBody({ s, symbol }: { s: Snapshot; symbol: string }) {
  return (
    <div className="stack" style={{ marginTop: ".5rem" }}>
      <dl className="kv kv--left">
        <dt>Root</dt><dd className="mono" style={{ overflowWrap: "anywhere" }}>{s.root}</dd>
        <dt>Slot</dt><dd>{String(s.slot)}{s.slot_lo !== s.slot ? ` (read between ${String(s.slot_lo)} and ${String(s.slot)})` : ""} · taken {dateText(Number(s.taken_at))}</dd>
        <dt>Holders</dt><dd>{s.entries} · total weight {units(s.total_weight, 9, 3)} {symbol}</dd>
        <dt>Coins</dt><dd>{s.coins.length} backed{s.coins.length ? ` (${units(s.coins.reduce((a, c) => a + c.wx, 0n), 9, 3)} ${symbol} in their vaults)` : ""} · {s.excluded.length} account{s.excluded.length === 1 ? "" : "s"} excluded</dd>
      </dl>
      {s.leaves.length ? <>
        <p className="muted" style={{ margin: 0 }}>Leaves {s.offset + 1}–{s.offset + s.leaves.length} of {s.entries}: holder · weight · share of the draw · ticket range starts at</p>
        <ul style={{ margin: 0, paddingLeft: "1.1rem" }}>{s.leaves.map((l) => (
          <li key={l.owner}><a className="mono" href={solscan("account", l.owner)} target="_blank" rel="noreferrer noopener" title={l.owner}>{short(l.owner)}</a> · {units(l.weight, 9, 3)} {symbol} · {(leafShare(l.weight, s.total_weight) * 100).toFixed(2)} % · {String(l.cum_start)}</li>))}</ul>
      </> : <p className="muted" style={{ margin: 0 }}>No leaf in this page.</p>}
    </div>
  );
}

/** The round's published snapshot, read from the canister when opened. */
function SnapshotDetails({ v, symbol }: { v: RaffleView; symbol: string }) {
  const [open, setOpen] = useState(false);
  const q = useRaffleSnapshot(v, open);
  const r = v.raffle!;
  return (
    <details className="small" onToggle={(e: SyntheticEvent<HTMLDetailsElement>) => setOpen(e.currentTarget.open)}>
      <summary>Published snapshot of round {String(r.round)} (slot {String(r.snapshotSlot)})</summary>
      {!open ? null : q.isLoading ? <p className="muted">Reading the snapshot from the canister…</p>
        : q.error ? <p className="error">The snapshot could not be read: {errText(q.error)}</p>
        : !q.data ? <p className="muted">The canister keeps no published snapshot of this round (it keeps the last rounds only).</p>
        : <SnapshotBody s={q.data} symbol={symbol} />}
    </details>
  );
}

/** A collection's raffle. `compact`: the coin page's version (the pot, the progress, the next draw, a link to the collection). */
export function RafflePanel({ slug, compact = false }: { slug: string; compact?: boolean }) {
  const col = collectionBySlug(slug);
  const { view: v, loading, error } = useRaffle(slug);
  const now = useNow();
  if (!col) return null;
  const head = <p className="eyebrow" style={{ margin: 0 }}>Holder raffle{DEMO ? " · demo" : ""}</p>;
  if (!RAFFLES_READABLE) return <section className="panel panel--tight small stack">{head}<p className="muted" style={{ margin: 0 }}>Each collection gets a weekly raffle among the holders of its graduated coins, kept by Rook's canister. This build names no canister yet: the raffle shows here once it is installed.</p>{compact ? null : <HowRaffleWorks />}</section>;
  if (loading) return <section className="panel panel--tight small">{head}<p className="muted">Reading the raffle…</p></section>;
  if (error) return <section className="panel panel--tight small">{head}<p className="error">The raffle could not be read: {errText(error)}</p></section>;
  if (!v) return <section className="panel panel--tight small stack">{head}<p className="muted" style={{ margin: 0 }}>The raffle opens with the collection: the canister has not pinned it yet.</p>{compact ? null : <HowRaffleWorks />}</section>;
  if (!v.raffle) return <section className="panel panel--tight small stack">{head}<p className="muted" style={{ margin: 0 }}>The raffle is not opened yet: the canister opens it on the collection's books before the first pot arrives.</p>{compact ? null : <HowRaffleWorks />}</section>;
  const r = v.raffle, nd = nextDraw(r, now, v.prizeHeld);
  return (
    <section className="panel panel--tight small stack raffle">
      <div className="row">{head}<span className="right"><Pill tone={tone(v)}>{PHASE_LABEL[r.phase]}</Pill></span></div>
      <p className="raffle__pot"><strong className="num">{solText(r.pot, 3)}</strong> <span className="muted">SOL in the pot</span>{v.prizeHeld ? <span className="muted"> · a prize of 1 {col.wxSymbol} is held</span> : null}</p>
      <Progress v={v} />
      <dl className="kv">
        <dt>Next draw</dt><dd>{nd.text}</dd>
        {r.round > 0n ? <><dt>Round</dt><dd>{String(r.round)}{r.phase !== "idle" ? ` · ${r.entries} holders` : ""}</dd></> : null}
        {!compact ? <><dt>Prizes</dt><dd>{String(r.prizesClaimed)} claimed · {String(r.roundsExpired)} rolled over</dd></> : null}
        {!compact && v.address ? <><dt>Raffle key</dt><dd><a className="mono" href={solscan("token", v.address)} target="_blank" rel="noreferrer noopener" title={v.address}>{short(v.address)}</a> <small className="muted">the collection's wX mint</small></dd></> : null}
      </dl>
      {compact ? <p className="small" style={{ margin: 0 }}>Holding this coin enters you in the <a href={href("collection", slug)}>{col.name} raffle</a>, weighted by the backing your coins carry.</p>
        : <>{r.round > 0n ? <SnapshotDetails v={v} symbol={col.wxSymbol} /> : null}<LastWinner v={v} symbol={col.wxSymbol} /><HowRaffleWorks /></>}
    </section>
  );
}

/** The front page's raffles: the biggest pots and their next draws. Nothing before the raffles can be read. */
export function RafflesSection() {
  const q = useRaffles();
  const now = useNow();
  if (!RAFFLES_READABLE) return null;
  const top = biggestPots(q.data ?? []);
  return (
    <section className="block raffles" aria-labelledby="raffles-h">
      <div className="row" style={{ marginBottom: ".75rem" }}><div><p className="eyebrow" id="raffles-h">Raffles{DEMO ? " · demo" : ""}</p><h2 style={{ margin: 0 }}>Weekly prizes for holders</h2></div><a className="right small" href={href("collections")}>Every collection →</a></div>
      <p className="small muted" style={{ marginTop: 0 }}>Each collection draws one whole wrapped NFT a week among the holders of its graduated coins, weighted by the backing their coins carry. The pots fill from each collection's reserve overflow.</p>
      {q.loading ? <p className="small muted">Reading the raffles…</p> : q.error ? <p className="small error">The raffles could not be read: {errText(q.error)}</p>
        : !top.length ? <p className="small muted">No raffle is open yet.</p>
        : <ul className="rafflelist">{top.map((v) => {
          const c = collectionBySlug(v.slug)!, p = prizeProgress(v.raffle!.pot, v.price);
          return (
            <li key={v.slug}><a href={href("collection", v.slug)} className="rafflelist__row">
              <CollectionLogo c={c} size={36} />
              <span className="rafflelist__id"><strong>{c.name}</strong><small className="muted">{nextDraw(v.raffle!, now, v.prizeHeld).text}</small></span>
              <span className="rafflelist__pot"><strong className="num">{solText(v.raffle!.pot, 2)} SOL</strong><span className="progress"><i className={p === 1 ? "done" : undefined} style={{ width: p === null ? "0%" : pct(p) }} /></span></span>
            </a></li>);
        })}</ul>}
    </section>
  );
}
