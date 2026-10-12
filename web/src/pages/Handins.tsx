import { href, useNow } from "../lib/env";
import { useHandins, useWrappedCollection, type WSource } from "../lib/wrapped";
import { API_BASE } from "../api/source";
import { eth, when } from "../lib/format";
import { Failed, Loading, Section } from "../components/ui";
import { WxGate } from "../components/WxGate";
import { HandinRows, HandinTotals, Proof } from "../components/wx";
import type { HandinReport } from "../api/types";

/**
 * docs/v6/offer-handins.md. The collection's offer board, read: what is on it, what each hand-in became, what was paid, and how a
 * bot earns the reward. Nothing here submits anything: a hand-in is two calls a bot makes with its own tools (`submit` on the board
 * with the bond, `submit_offer` to the canister with the bytes), and this page says exactly how, rather than pretending to.
 */
export function HandinsPage({ slug }: { slug?: string }) {
  return <WxGate title="Offer hand-ins">{(src) => slug ? <One slug={slug} src={src} /> : <p>Pick a collection on the <a href={href("collection")}>collections page</a>: hand-ins are per collection, and only Bitcoin collections have a board.</p>}</WxGate>;
}

function One({ slug, src }: { slug: string; src: WSource }) {
  const q = useHandins(slug), col = useWrappedCollection(slug), now = useNow();
  if (q.isLoading || col.isLoading) return <Loading what="the offer board" />;
  if (q.error) return <Failed what="the offer board" error={q.error} />;
  if (!q.data) return <div className="notice"><h2>No such collection</h2><p>Nothing is configured under “{slug}”. <a href={href("collection")}>All collections</a>.</p></div>;
  const h: HandinReport = q.data, fixture = src.kind === "fixture";
  return (
    <>
      <header className="pagehead">
        <h1>Offer hand-ins <span className="quiet">for {h.name}</span></h1>
        <p><a href={href("collection", h.slug)}>The collection page</a> · the same board as JSON: {src.kind === "live" && API_BASE ? <a className="mono" href={`${API_BASE}/v1/handins/${h.slug}`}>/v1/handins/{h.slug}</a> : <span className="mono">/v1/handins/{h.slug}</span>}</p>
      </header>

      {!h.board ? (
        <div className="notice" role="status"><h2>No offer board</h2><p>{h.error ?? "This collection has no offer board."}</p></div>
      ) : (
        <>
          {h.error ? <div className="notice" role="status"><h2>The board could not be read</h2><p>{h.error}</p></div> : null}
          <Section title="The board" aside={<>Contract <Proof href={h.boardUrl} fixture={fixture}>{h.board}</Proof>{h.block ? ` · block ${h.block}` : ""} · read {when(h.computedAt)}</>}>
            <HandinTotals h={h} />
            {h.params ? <p className="quiet small">Bond {eth(BigInt(h.params.bondWei), 3)} ETH, attached to every hand-in and returned with the reward or at expiry; no slash before {Math.round(h.params.slashDelayS / 3600)} h has passed on both clocks; the submitter may take an untried bond back after {Math.round(h.params.expiryS / 3600)} h. All of it immutable in the board's bytecode.</p> : null}
          </Section>

          <Section title="Open hand-ins" aside="Cheapest first: that is the order the canister tries them in.">
            <HandinRows rows={h.open} now={now} fixture={fixture} custodian={h.custodian} />
          </Section>

          <Section title="Settled" aside="Every hand-in that was bought and rewarded, slashed, or refunded — newest first.">
            <HandinRows rows={h.settled} now={now} fixture={fixture} custodian={null} />
          </Section>
        </>
      )}

      <Section title="How a bot earns the reward">
        <div className="prose">
          <p>The collection's custodian — a canister that holds the items — buys an inscription whenever one of its sinks wants one: a graduating coin, a mint request, the Floor Module. It used to get its marketplace offers from one operator. Now anyone may hand one in, and the canister checks it exactly as it checked the operator's.</p>
          <ol>
            <li><strong>Find a fillable listing</strong> of an inscription in this collection, priced under the floor the keeper posts: a seller-signed PSBT from a marketplace, and the inscription's membership proof from the collection's published tree.</li>
            <li><strong>Commit on chain.</strong> Call <span className="mono">submit(sha256(psbt), askSats, inscriptionId)</span> on the board with exactly the bond attached. <span className="mono">askSats</span> is what the purchase will take from the canister's float, fee included; lying about it gets the bond slashed.</li>
            <li><strong>Deliver the bytes.</strong> Call the canister's <span className="mono">submit_offer(psbt_hex, to_sign, proof_hex)</span> from any principal. The hash must match an open submission; the canister keeps at most one set of bytes per hash.</li>
            <li><strong>Wait.</strong> When a sink wants an item, the canister takes the cheapest open hand-in that has bytes and judges it: the proof, ord's location, the explorers' view of the seller's output, the PSBT's own sat flow and fee. Valid: it signs, broadcasts, and once the purchase is six blocks deep it calls <span className="mono">claim</span> — the bond plus the reward are credited to the address that submitted. Invalid bytes: <span className="mono">slash</span>, after the delay. A listing that vanished, or a dearer hand-in than another: nothing happens, and the bond is refundable at expiry.</li>
            <li><strong>Collect.</strong> Call <span className="mono">withdraw()</span> on the board: payments are pull, so a submitter that refuses ETH can never block the canister.</li>
          </ol>
          <p className="quiet">The reward is fixed, not a share of the price, so inflating the ask buys nothing; the canister buys the cheapest valid hand-in, so being cheapest is the whole game. Purchases are one at a time and stage-capped, so the ceiling is a few hundredths of an ETH a day per collection: enough for a listing scraper, not enough to make manipulation pay. This page reads the board; it does not submit — a hand-in is your own transaction and your own call to the canister, with the parameters above.</p>
        </div>
      </Section>
    </>
  );
}
