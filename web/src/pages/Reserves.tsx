import { href, useNow } from "../lib/env";
import { useReserve, useReserves, type WSource } from "../lib/wrapped";
import { API_BASE } from "../api/source";
import { countdown, when } from "../lib/format";
import { Failed, Loading, Section } from "../components/ui";
import { WxGate } from "../components/WxGate";
import { Equation, ItemRows, LiquidityPanel, ModuleBooks, Proof, StatusBadge, TicketRows, TreasuryPanel } from "../components/wx";
import type { ReserveReport } from "../api/types";

/**
 * docs/v5/ux-and-reserves.md §1, "Proof of reserves". The custody is a honeypot, so everything is on this page: every item, where
 * it sits, how it came, what each public source said about it just now, and the equation computed from those answers.
 */
export function ReservesPage({ slug }: { slug?: string }) {
  return <WxGate title="Proof of reserves">{(src) => (slug ? <One slug={slug} src={src} /> : <All src={src} />)}</WxGate>;
}

const STATUS_SAY = { green: "Every item was found where it should be, by every source, and the numbers add up exactly.",
  red: "Something does not add up. The reasons are listed below.", unverified: "Not every source could be read just now. Nothing is shown as verified until it is." } as const;

function All({ src }: { src: WSource }) {
  const q = useReserves();
  if (q.isLoading) return <Loading what="the reserves" />;
  if (q.error) return <Failed what="the reserves" error={q.error} />;
  return (
    <>
      <header className="pagehead">
        <h1>Proof of reserves</h1>
        <p>Every token here is backed by an item in custody or by ETH in its collection's Floor Module — a convertible pot the canister turns into items, and a standing reserve that stays ETH. This page checks that against public sources you can open yourself, not against our word, and shows the treasury and the protocol's own liquidity beside it.</p>
      </header>
      <Method src={src} />
      <ul className="rows">
        {q.data!.reports.map((r) => (
          <li key={r.slug}><a className="row" href={href("reserves", r.slug)}>
            <span className="row__main"><span className="row__name">{r.name}</span><StatusBadge status={r.status} /></span>
            <span className="row__meta">{r.equation.itemsInCustody} of {r.equation.heldInLedger} items found · {r.equation.ticketsInFlight} in flight · checked {when(r.computedAt)}</span>
          </a></li>
        ))}
      </ul>
    </>
  );
}

function One({ slug, src }: { slug: string; src: WSource }) {
  const q = useReserve(slug), now = useNow();
  if (q.isLoading) return <Loading what="the reserve report" />;
  if (q.error || !q.data) return <div className="notice"><h2>No such collection</h2><p>Nothing is configured under “{slug}”. <a href={href("reserves")}>All collections</a>.</p></div>;
  const r: ReserveReport = q.data, fixture = src.kind === "fixture";
  return (
    <>
      <header className="pagehead">
        <h1>{r.name} <span className="mono quiet">{r.symbol}</span></h1>
        <p><a href={href("collection", r.slug)}>The collection page</a> · the same report as JSON: {src.kind === "live" && API_BASE ? <a className="mono" href={`${API_BASE}/v1/reserves/${r.slug}`}>/v1/reserves/{r.slug}</a> : <span className="mono">/v1/reserves/{r.slug}</span>}</p>
      </header>

      <section className={`headline wx-verdict-panel wx-verdict-panel--${r.status}`} aria-labelledby="h-status">
        <h2 id="h-status">Status</h2>
        <p className="headline__num"><StatusBadge status={r.status} /></p>
        <p>{STATUS_SAY[r.status]}</p>
        {r.reasons.length ? <ul className="wx-reasons">{r.reasons.map((x, i) => <li key={i}>{x}</li>)}</ul> : null}
        <p className="quiet">Checked {when(r.computedAt)}. Last verified {r.lastGreenAt ? when(r.lastGreenAt) : "never, since this indexer started"}.</p>
      </section>

      <Section title="The equation" aside="Computed from the sources below, not from the wrapper's own books.">
        <Equation r={r} />
      </Section>

      <Section title="The Floor Module" aside="Its price against the keeper's floor, its spreads, its pots, its last fills.">
        <ModuleBooks r={r} now={now} />
      </Section>

      <Section title="The protocol treasury" aside="Every income stream in; four fixed percentages out; nothing discretionary.">
        <TreasuryPanel r={r} />
      </Section>

      <Section title="Protocol-owned liquidity" aside="Fee slices placed in the collection's pool by the lock, never withdrawable.">
        <LiquidityPanel r={r} />
      </Section>

      <Section title="Where the items are">
        <dl className="kv kv--wide">
          {r.custody.kind === "bitcoin" ? <><dt>Bitcoin custody address</dt><dd><Proof href={r.custody.addressUrl} fixture={fixture}>{r.custody.address}</Proof></dd></>
            : <><dt>Custody</dt><dd>the canister's own address on {r.custody.chainName}: <Proof href={r.custody.addressUrl} fixture={fixture}>{r.custody.custodian}</Proof></dd>
              <dt>Collection contract</dt><dd className="mono">{r.custody.collection}</dd>
              <dt>Checked through</dt><dd className="mono">{r.custody.providers.join(", ")}</dd></>}
          <dt>Wrapper on Robinhood Chain</dt><dd><Proof href={r.wrapperUrl} fixture={fixture}>{r.wrapper}</Proof></dd>
        </dl>
      </Section>

      <Section title="Every item" aside={`${r.items.length} in custody by the wrapper's own events, each checked just now.`}>
        <ItemRows items={r.items} kind={r.kind} fixture={fixture} />
      </Section>

      <Section title="Tickets in flight" aside="Redemptions burned and not yet delivered or refunded.">
        <TicketRows tickets={r.ticketsInFlight} now={now} fixture={fixture} />
        {r.ticketsInFlight.some((t) => t.deliverBy && t.deliverBy > now) ? (
          <p className="quiet small">Next deadline in {countdown(Math.min(...r.ticketsInFlight.filter((t) => t.deliverBy && t.deliverBy > now).map((t) => t.deliverBy!)), now)}. <a href={href("redeem", r.slug)}>Track or refund a ticket</a>.</p>
        ) : null}
      </Section>

      <Method src={src} />
    </>
  );
}

function Method({ src }: { src: WSource }) {
  return (
    <Section title="How to check it yourself">
      <div className="prose">
        <p><strong>Inscriptions:</strong> open each inscription on ordinals.com — it must sit at the custody address — then open the output it sits in on mempool.space and blockstream.info: it must pay the custody address and be unspent.</p>
        <p><strong>NFTs:</strong> call <span className="mono">ownerOf(tokenId)</span> on the collection's contract on its own chain, through any provider or explorer: it must be the custodian.</p>
        <p><strong>Then add up:</strong> items found, minus tickets burned and not yet delivered (their item is still in custody but backs no token), plus the Floor Module's ETH-backed units — its convertible units (<span className="mono">netUnits()</span>) and its reserve units (<span className="mono">reserveUnits()</span>) — must equal the token's <span className="mono">totalSupply()</span> on Robinhood Chain; and the module's <span className="mono">escrowBalance()</span>, both pots, must stand behind those units.</p>
        <p className="quiet">Green only when every source answered, agreed and is recent. A source that failed or disagreed is unverified, never green; an item found anywhere else is red. {src.kind === "fixture" ? "In this fixture the links are shown as text: they point at invented ids." : ""}</p>
      </div>
    </Section>
  );
}
