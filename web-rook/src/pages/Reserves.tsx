import { ticketLine } from "../rook/keyless";
import type { CustodyItem, ReserveReport, Ticket } from "../api/types";
import { API_BASE } from "../api/source";
import { href, useNow } from "../lib/env";
import { useReserve, useReserves, type WSource } from "../lib/wrapped";
import { eth, when } from "../lib/format";
import { items, ledgerAgrees, ledgerOf } from "../rook/ledger";
import { Gate } from "../ui/Gate";
import { LiquidityPanel, ModuleBooks, TreasuryPanel } from "../ui/Books";
import { Fig, Loading, Proof, Status, StatusPill, ago } from "../ui/bits";
import { PHASE1, isOff } from "../rook/phase";
import { useEthReserve } from "../rook/v8/source";
import { EthLedger, EthNests } from "./ReservesEth";

/** docs/v6/site-design.md §6: the ledger (on screen, the reserves). Custody is a honeypot, so all of it is shown. */
export function ReservesPage({ slug }: { slug?: string }) {
  return <Gate page={slug ? `reserves/${slug}` : "reserves"}>{(src) => (slug ? <One slug={slug} src={src} /> : <All src={src} />)}</Gate>;
}

function All({ src }: { src: WSource }) {
  const q = useReserves(), now = useNow();
  return (
    <>
      <header className="pagehead rise"><p className="eyebrow">Reserves</p><h1>Check the reserves</h1>
        <p className="lede">For every collection: the NFTs public sources find in the vault, less redemptions in flight, plus wrapped tokens backed by ETH in the floor module, must equal the wrapped tokens in supply. Green when every NFT is found and the sum is exact; red when it is not; unverified when a source did not answer.</p>
        <p className="small muted" style={{ margin: 0 }}>Each collection's wrapped token (wFROG, wAPE, …) stands for one NFT held in its vault: one whole token, one NFT.</p></header>
      {q.isLoading ? <Loading what="the reserves" /> : null}
      <ul className="nests rise rise-1">
        {(q.data?.reports ?? []).map((r) => (
          <li key={r.slug}><a className="nest" href={href("reserves", r.slug)}>
            <span className="nest__name">{r.name}<small>{r.symbol} · checked {ago(r.computedAt, now)}{r.lastGreenAt ? `, last green ${ago(r.lastGreenAt, now)}` : ", never green"}</small></span>
            <Fig label={r.kind === "inscriptions" ? "Ordinals" : "NFTs"} value={r.equation.itemsInCustody} />
            <Fig label="In flight" value={r.equation.ticketsInFlight} />
            <Fig label={`${r.symbol} supply`} value={items(r.equation.supplyUnits ? BigInt(r.equation.supplyUnits) : null)} />
            <span><StatusPill status={r.status} /></span>
          </a></li>
        ))}
      </ul>
      <EthNests />
      {src.kind === "live" && API_BASE ? <p className="small muted" style={{ marginTop: "1rem" }}>The same as JSON: <a className="mono" href={`${API_BASE}/v1/reserves`}>/v1/reserves</a>. Anyone can recompute it from the sources named on each page.</p> : null}
    </>
  );
}

function One({ slug, src }: { slug: string; src: WSource }) {
  const q = useReserve(slug), now = useNow(), e = useEthReserve(slug);
  if (q.isLoading || e.isLoading || e.sourceLoading) return <Loading what="the reserves" />;
  // docs/v8/proof-of-reserves.md: an Ethereum collection's ledger (src/pages/ReservesEth.tsx), above the v5 ledger when both exist
  if ((q.error || !q.data) && !e.data) return <div className="pagehead"><h1>No such collection</h1><p className="lede">Nothing is configured under “{slug}”. <a href={href("reserves")}>All collections</a>.</p></div>;
  return <>{e.data ? <EthLedger r={e.data} fixture={e.source?.kind === "fixture"} /> : null}{q.data ? <Ledger r={q.data} fixture={src.kind === "fixture"} now={now} /> : null}</>;
}

function Ledger({ r, fixture, now }: { r: ReserveReport; fixture: boolean; now: number }) {
  const l = ledgerOf(r);
  const m = r.floorModule;
  const stale = now - r.computedAt > 3 * 3_600;
  return (
    <>
      <header className="pagehead rise">
        <p className="eyebrow"><a href={href("reserves")}>Reserves</a> · <a href={href("collection", r.slug)}>{r.name}</a></p>
        <h1>{r.name}: the reserves</h1>
        <div className="row"><Status status={r.status} /><span className="small muted">checked {ago(r.computedAt, now)}{r.block ? ` at block ${r.block.toLocaleString("en-US")}` : ""} · {r.lastGreenAt ? `last green ${ago(r.lastGreenAt, now)}` : "never green"}{stale ? " · stale: more than three hours old" : ""}</span></div>
        {r.reasons.length ? <ul className="reasons">{r.reasons.map((x, i) => <li key={i}>{x}</li>)}</ul> : null}
      </header>

      <div className="grid rise rise-1">
        <div className="span-6">
          <p className="eyebrow">The equation</p>
          <dl className="ledger">
            {l.rows.filter((row) => row.kind !== "sum" && row.kind !== "supply").map((row) => (
              <div key={row.key} className={`ledger__row${row.units === 0n && row.key === "eth" ? " ledger__row--muted" : ""}`}><dt>{row.label}{row.note ? <small>{row.note}</small> : null}</dt><dd className="num">{items(row.units)}</dd></div>
            ))}
            <div className="ledger__row ledger__row--sum"><dt>Backing</dt><dd className="num">{items(l.backing)}</dd></div>
            <div className="ledger__row ledger__row--vs"><dt>{r.symbol} supply<small>the wrapper's totalSupply at the same block</small></dt><dd className="num">{l.holds === null ? "?" : l.holds ? "=" : "≠"} {items(l.supply)}</dd></div>
          </dl>
          {!ledgerAgrees(l, r) ? <p className="small bad">This ledger and the indexer count differently: the report says {items(BigInt(r.equation.backingUnits))} of backing over {items(BigInt(r.equation.ethBackedUnits))} ETH-backed units.</p> : null}
          {r.equation.heldOnChain !== null && r.equation.heldOnChain !== r.equation.heldInLedger ? <p className="small bad">The wrapper says it holds {r.equation.heldOnChain}; the event ledger counts {r.equation.heldInLedger}.</p> : null}
          <p className="small muted">{m.kind === "absent" ? m.note : m.kind === "read" ? <>{items(BigInt(m.ethBackedUnits))} ETH-backed {r.symbol} against <span className="num">{eth(BigInt(m.escrowWei), 4)}</span> ETH in both pots, at a fills price of <span className="num">{eth(BigInt(m.priceWei), 4)}</span> ETH with spreads of <span className="num">{(m.spreadMintBps / 100).toFixed(2)}%</span> to mint and <span className="num">{(m.spreadBurnBps / 100).toFixed(2)}%</span> to burn. The module's books are below.</> : `The floor module could not be read: ${m.error}`}</p>
        </div>
        <div className="span-6">
          <p className="eyebrow">Where the numbers come from</p>
          <ul className="list list--tight small">
            <li><strong>{r.kind === "inscriptions" ? "Ordinals" : "NFTs"}</strong> — {r.kind === "inscriptions" ? "each inscription's location from ord, then the output it sits in at two esplora explorers: it exists, pays the custody address, is unspent." : "ownerOf on every configured provider of the collection's chain at one pinned block; all must name the custodian."}</li>
            <li><strong>Redemptions in flight</strong> — the wrapper's open tickets: burnt {r.symbol} whose {r.kind === "inscriptions" ? "ordinal" : "NFT"} is not delivered yet.</li>
            <li><strong>ETH-backed {r.symbol} and supply</strong> — the floor module's `ethBackedUnits`, `reserveUnits`, `netUnits` and `floatUnits`, and the wrapper's totalSupply, on Robinhood Chain at one block.</li>
            <li><strong>The vault</strong> — <Proof href={r.custody.kind === "bitcoin" ? r.custody.addressUrl : r.custody.addressUrl} fixture={fixture}>{r.custody.kind === "bitcoin" ? r.custody.address : r.custody.custodian}</Proof></li>
            <li><strong>Wrapper</strong> — <Proof href={r.wrapperUrl} fixture={fixture}>{r.wrapper}</Proof></li>
            {r.kind === "inscriptions" && !isOff("bots", PHASE1) ? <li><strong>Offer board</strong> — <a href={href("bots", r.slug)}>the hand-ins the custodian buys from</a>.</li> : null}
          </ul>
        </div>
      </div>

      <section className="block rise rise-2">
        <p className="eyebrow">The floor module</p>
        <h2>Its price against the keeper's floor, its spreads, its pots, its last fills</h2>
        <ModuleBooks r={r} now={now} fixture={fixture} />
      </section>

      <div className="grid rise rise-2" style={{ marginTop: "3.5rem" }}>
        <section className="span-6"><p className="eyebrow">The protocol treasury</p><h2>Every income stream in; four fixed percentages out</h2><TreasuryPanel r={r} /></section>
        <section className="span-6"><p className="eyebrow">Protocol-owned liquidity</p><h2>Fee slices placed in the collection's pool by the lock, never withdrawable</h2><LiquidityPanel r={r} /></section>
      </div>

      <section className="block rise rise-3">
        <p className="eyebrow">{r.kind === "inscriptions" ? "Every ordinal" : "Every NFT"} in the vault</p>
        <h2>{r.items.length} on the books{fixture ? " (invented ids; the links are text)" : ""}</h2>
        <div className="scroll"><table>
          <thead><tr><th>{r.kind === "inscriptions" ? "Ordinal" : "NFT"}</th><th>Arrived</th><th>Verdict</th><th>Sources</th></tr></thead>
          <tbody>{r.items.map((it) => <ItemRow key={it.itemId} it={it} kind={r.kind} fixture={fixture} now={now} />)}</tbody>
        </table></div>
      </section>

      <section className="block rise rise-3">
        <p className="eyebrow">Redemptions in flight</p>
        <h2>{r.ticketsInFlight.length ? `${r.ticketsInFlight.length} redemption${r.ticketsInFlight.length === 1 ? "" : "s"} not yet delivered` : "No redemption in flight"}</h2>
        {r.ticketsInFlight.length ? <ul className="list">{r.ticketsInFlight.map((t) => <li key={t.id} className="small"><a href={href("redeem", r.slug, t.id)}><strong>Ticket {t.id}</strong></a> · {ticketStage(t, now)}</li>)}</ul> : null}
      </section>
    </>
  );
}

function ItemRow({ it, kind, fixture, now }: { it: CustodyItem; kind: ReserveReport["kind"]; fixture: boolean; now: number }) {
  return (
    <tr>
      <td><Proof href={it.refUrl} fixture={fixture}>{it.ref.length > 24 ? `${it.ref.slice(0, 10)}…${it.ref.slice(-8)}` : it.ref}</Proof>{it.promisedTo !== null ? <><br /><span className="small muted">drawn by ticket {it.promisedTo}, awaiting delivery</span></> : null}</td>
      <td className="small">{it.arrival.how === "purchase" ? "bought into the vault" : it.arrival.how === "deposit" ? "deposited by a holder" : "lent by its owner"}{it.arrival.mintedAt ? <><br /><span className="muted num">{ago(it.arrival.mintedAt, now)}</span></> : null}{it.arrival.tx ? <><br /><Proof href={it.arrival.txUrl} fixture={fixture}>{kind === "inscriptions" ? "the Bitcoin transaction" : "the transfer"}</Proof></> : null}</td>
      <td><span className={`verdict verdict--${it.check.verdict}`}>{it.check.verdict.replace("-", " ")}</span>{it.check.reason ? <><br /><span className="small muted">{it.check.reason}</span></> : null}{it.check.checkedAt ? <><br /><span className="small muted num">{ago(it.check.checkedAt, now)}</span></> : null}</td>
      <td className="small">{it.check.sources.map((s, i) => <span key={i} style={{ display: "block" }}><Proof href={s.url} fixture={fixture}>{s.name}</Proof>: <span className={s.ok ? "" : "warn"}>{s.said}</span></span>)}</td>
    </tr>
  );
}

/** One line for a ticket's stage, and when it is due a refund (src/rook/keyless.ts: there is no redraw, the pinned round is final). */
export const ticketStage = (t: Ticket, now: number): string => ticketLine(t, now);
