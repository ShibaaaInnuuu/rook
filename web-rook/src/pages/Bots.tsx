import type { HandinReport } from "../api/types";
import { API_BASE } from "../api/source";
import { href, useNow } from "../lib/env";
import { useHandins, useWrappedList, type WSource } from "../lib/wrapped";
import { eth } from "../lib/format";
import { OFFER_BOARD_SUGGESTED } from "../rook/abis";
import { Gate } from "../ui/Gate";
import { HandinRows, HandinTotals } from "../ui/Books";
import { Fig, Loading, Proof } from "../ui/bits";

/**
 * docs/v6/offer-handins.md: how anyone earns — hand a fillable listing in with a bond; the bond comes back with a reward once the
 * purchase is six blocks deep. The board is read through the indexer (`/v1/handins/:slug`, the HandinReport), so a fixture and a
 * live chain draw the same page; nothing here submits — a hand-in is the bot's own two calls, described below.
 */
export function Bots({ slug }: { slug?: string }) {
  return <Gate page={slug ? `bots/${slug}` : "bots"}>{(src) => <Boards src={src} slug={slug} />}</Gate>;
}

function Boards({ src, slug }: { src: WSource; slug?: string }) {
  const list = useWrappedList();
  const nests = (list.data?.collections ?? []).filter((c) => c.kind === "inscriptions");
  const at = slug ?? nests[0]?.slug;
  return (
    <>
      <header className="pagehead rise"><p className="eyebrow">Bots</p><h1>Hand the vault a listing, get paid</h1>
        <p className="lede">The custodian buys from listings anyone hands it. Find a frog listed under the floor, hand it in with a small bond, and when the purchase is six blocks deep the bond comes back with a reward. No key, no permission, no relationship with us.</p></header>
      {list.isLoading ? <Loading what="the collections" /> : null}
      {nests.length > 1 ? <div className="tabs rise" role="tablist" aria-label="Which collection's board" style={{ marginBottom: "1.25rem" }}>{nests.map((c) => <a key={c.slug} role="tab" aria-selected={c.slug === at} className={`tabs__link${c.slug === at ? " on" : ""}`} href={href("bots", c.slug)}>{c.name}</a>)}</div> : null}
      {at ? <BoardOf slug={at} src={src} /> : list.data ? <p className="notice notice--warn small">Only a Bitcoin collection has an offer board, and none is configured.</p> : null}
      <section className="block rise rise-3">
        <h2>How a bot earns the reward</h2>
        <div className="grid">
          <div className="span-7">
            <ol className="seq seq--stack">
              <li><h3>Find a fillable listing</h3><p>An inscription in the collection, priced under the floor the keeper posts: a seller-signed PSBT from a marketplace and the inscription's membership proof from the collection's published tree.</p></li>
              <li><h3>Commit on chain</h3><p>Call <span className="mono">OfferBoard.submit(sha256(psbt), askSats, inscriptionId)</span> with exactly the bond attached. <span className="mono">askSats</span> is what the purchase takes from the canister's float, fee included; lying about it gets the bond slashed. One open submission per listing hash, at most four per address.</p></li>
              <li><h3>Send the bytes</h3><p>Call the custodian's <span className="mono">submit_offer(psbt, to_sign, proof)</span>, an update call open to anyone. Whoever delivers the bytes does not matter: the on-chain record names who is paid.</p></li>
              <li><h3>The cheapest valid one is bought</h3><p>When a coin's vault or the floor module wants an ordinal, the custodian judges the cheapest open hand-in under its cap — the same checks as the operator's own offers — signs the float inputs and broadcasts.</p></li>
              <li><h3>Claim</h3><p>Six blocks deep, with ord showing the vault holding exactly that inscription, the board credits you bond + reward. Pull it with <span className="mono">withdraw()</span>: payments are pull, so a submitter that refuses ETH can never block the canister.</p></li>
            </ol>
          </div>
          <div className="span-5 stack">
            <div><h3>What is slashed, what merely waits</h3><p className="small">The line is: <strong>slashed if the fault is in the bytes you committed to</strong>, whatever happens on the market later. A PSBT that does not parse, a proof that does not verify, a seller output carrying runes, a spend above the ask you stated: slashed after the delay, the bond into the reward pool. A listing sold elsewhere, a cheaper hand-in that won, a cap that moved: not tried, the bond back at expiry. <a href={href("docs", "offer-handins")}>The full table</a>.</p></div>
            <div><h3>What it pays</h3><p className="small">Per confirmed purchase: the reward, plus the bond back. The reward is fixed, not a share of the price, so inflating the ask buys nothing; the canister buys the cheapest valid hand-in, so being cheapest is the whole game. Purchases are one at a time and stage-capped, so the ceiling is a few hundredths of an ETH a day per collection: enough for a listing scraper, not enough to make manipulation pay. A seller can be its own bot and treat the reward as a rebate.</p></div>
          </div>
        </div>
      </section>
    </>
  );
}

function BoardOf({ slug, src }: { slug: string; src: WSource }) {
  const q = useHandins(slug), now = useNow();
  const fixture = src.kind === "fixture";
  if (q.isLoading) return <Loading what="the offer board" />;
  if (q.error || !q.data) return <p className="small warn">The offer board is not known here{q.error ? `: ${String((q.error as Error).message).slice(0, 120)}` : ""}.</p>;
  const h: HandinReport = q.data, S = OFFER_BOARD_SUGGESTED, p = h.params;
  return (
    <div className="stack-lg">
      <section className="rise rise-1">
        <div className="row" style={{ marginBottom: ".75rem" }}><div><p className="eyebrow">{h.name} · the board</p><h2 style={{ margin: 0 }}>{h.board ? `${h.totals.open} open hand-in${h.totals.open === 1 ? "" : "s"}` : "No offer board"}</h2></div>
          <span className="right small muted">{h.board ? <>Contract <Proof href={h.boardUrl} fixture={fixture}>{h.board}</Proof>{h.block ? ` · block ${h.block.toLocaleString("en-US")}` : ""}</> : null}</span></div>
        {!h.board ? <p className="notice notice--warn small">{h.error ?? "This collection has no offer board."}</p> : h.error ? <p className="notice notice--warn small">The board could not be read: {h.error}</p> : null}
        {h.board ? <HandinTotals h={h} /> : null}
        <div className="panel panel--tight" style={{ marginTop: "1rem" }}>
          <p className="eyebrow">The board's terms{p ? "" : <span className="warn"> · suggested, no board deployed</span>}</p>
          <div className="figs">
            <Fig label="Bond" value={eth(p ? BigInt(p.bondWei) : S.bondWei, 3)} unit="ETH" />
            <Fig label="Reward" value={eth(p ? BigInt(p.rewardWei) : S.rewardWei, 3)} unit="ETH" />
            <Fig label="Slash delay" value={`${(p ? p.slashDelayS : S.slashDelayS) / 3600} h`} />
            <Fig label="Expiry" value={`${(p ? p.expiryS : S.expiryS) / 3600} h`} />
            <Fig label="Reward a day, at most" value={eth(p ? BigInt(p.rewardDayCapWei) : S.rewardDayCapWei, 2)} unit="ETH" />
            <Fig label="Open slots" value={p ? p.maxOpen : S.maxOpen} unit={`· ${p ? p.maxOpenPerSubmitter : S.maxOpenPerSubmitter} an address`} />
          </div>
          <p className="small muted" style={{ margin: "1rem 0 0" }}>{p ? "All of it immutable in the board's bytecode: the bond is attached to every hand-in and returned with the reward or at expiry; no slash before the delay has passed on both clocks; the submitter may take an untried bond back after the expiry." : "From docs/v6/offer-handins.md §2, the parameters suggested for 4663. Read from the board itself once one is deployed and the indexer names it."}</p>
        </div>
      </section>
      {h.board ? (
        <>
          <section className="rise rise-2"><p className="eyebrow">Open hand-ins</p><p className="small muted">Cheapest first: that is the order the canister tries them in.</p><HandinRows rows={h.open} now={now} fixture={fixture} custodian={h.custodian} /></section>
          <section className="rise rise-2"><p className="eyebrow">Settled</p><p className="small muted">Every hand-in that was bought and rewarded, slashed, or refunded — newest first.</p><HandinRows rows={h.settled} now={now} fixture={fixture} custodian={null} /></section>
        </>
      ) : null}
      <p className="small muted">The same board as JSON: {src.kind === "live" && API_BASE ? <a className="mono" href={`${API_BASE}/v1/handins/${h.slug}`}>/v1/handins/{h.slug}</a> : <span className="mono">/v1/handins/{h.slug}</span>}{fixture ? " (a fixture: nothing here is deployed)" : ""}.</p>
    </div>
  );
}
