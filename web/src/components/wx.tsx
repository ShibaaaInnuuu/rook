import type { ReactNode } from "react";
import type { CustodyItem, HandinReport, HandinSubmission, ReserveReport, ReserveStatus, Ticket, WrappedCoin } from "../api/types";
import { pct, type Leg, type Quote } from "../lib/quote";
import { countdown, eth, when } from "../lib/format";
import { coins } from "../lib/chart";

/**
 * docs/v5/ux-and-reserves.md. The presentational pieces of the wrapped-collection pages: no wallet, no fetch, so a test can render
 * them to a string and read what a visitor would. The word "wX" (wFROG, wAPE…) appears only inside Details.
 */

/** Every page that shows fixture data opens with this, with the fixture set's own note. */
export const FixtureBanner = ({ note }: { note: string }) => (
  <p className="banner banner--demo wx-fixture" role="status"><strong>FIXTURE — not deployed.</strong> {note.replace(/^FIXTURE\.\s*/, "")}</p>
);

/** A link that a reader can follow, or, in a fixture, the same address as plain text: a fixture's links point at nothing. */
export function Proof({ href, children, fixture }: { href: string | null; children: ReactNode; fixture: boolean }) {
  if (!href) return <span className="mono">{children}</span>;
  if (fixture) return <span className="mono" title={`${href} (fixture: not a real link)`}>{children}</span>;
  return <a className="mono" href={href} target="_blank" rel="noreferrer noopener">{children}</a>;
}

// ---- the one-click ticket --------------------------------------------------------------------------------------------------

export const PHASE_LINE: Record<string, string> = {
  trading: "On its launch curve. Every buy moves it toward graduation.",
  graduating: "Raise complete — buying its items. Trading reopens on Uniswap when it graduates.",
  graduated: "Graduated — now trading on Uniswap.",
  refunding: "Did not graduate in time. Holders can refund.",
};

const TOKEN = (t: Leg["tokenIn"], symbol: string, wx: string) => (t === "ETH" ? "ETH" : t === "COIN" ? symbol : wx);
const amount = (t: Leg["tokenIn"], v: bigint) => (t === "ETH" ? eth(v, 6) : coins(v.toString(), 4));
const VENUE: Record<Leg["venue"], string> = { curve: "Launch curve", sharedPool: "Collection pool (Uniswap v4)", coinPool: "Coin pool (Uniswap v4)",
  floorMint: "Floor Module mint", floorBurn: "Floor Module burn" };

/** "You pay 0.1 ETH, you get ≈ N ALIEN, total cost 1.2%" — and, only when opened, everything that number is made of. */
export function QuoteSummary({ quote, symbol, wx, slippageBps, deadlineMin, estimate }: { quote: Quote; symbol: string; wx: string; slippageBps: number; deadlineMin: number; estimate: boolean }) {
  const buy = quote.side === "buy";
  const legs = quote.route.legs;
  const viaCurve = quote.route.id === "curve";
  return (
    <div className="oc__quote">
      <div className="tkt__row"><span className="tkt__label">You pay</span><span className="tkt__value num">{buy ? `${eth(quote.amountIn, 6)} ETH` : `${coins(quote.amountIn.toString(), 4)} ${symbol}`}</span></div>
      <div className="tkt__row"><span className="tkt__label">You get</span><span className="tkt__value num">≈ {buy ? `${coins(quote.amountOut.toString(), 4)} ${symbol}` : `${eth(quote.amountOut, 6)} ETH`}</span></div>
      <div className="tkt__row oc__cost"><span className="tkt__label">Total cost</span><span className="tkt__value num">{pct(quote.totalCostBps)}</span></div>
      <div className="tkt__row tkt__row--quiet"><span className="tkt__label">At least, after {pct(slippageBps, slippageBps % 100 ? 1 : 0)} slippage</span>
        <span className="tkt__value num">{buy ? `${coins(quote.minOut.toString(), 4)} ${symbol}` : `${eth(quote.minOut, 6)} ETH`}</span></div>
      {quote.route.change ? <p className="quiet small">This buy completes the raise: only {eth(quote.amountIn, 6)} ETH is used and {eth(quote.route.change, 6)} ETH comes back in the same transaction.</p> : null}
      <details className="oc__details">
        <summary>Details</summary>
        <p className="quiet small">Total cost is everything you give up against the market price before your trade: every fee, every spread, and how far your own trade moves the price.
          {estimate ? " These figures are an estimate from fixture data, not a venue's own quote." : ""}</p>
        <ol className="oc__legs">
          {legs.map((l, i) => (
            <li key={i}><strong>{VENUE[l.venue]}</strong>: {amount(l.tokenIn, l.amountIn)} {TOKEN(l.tokenIn, symbol, wx)} → {amount(l.tokenOut, l.amountOut)} {TOKEN(l.tokenOut, symbol, wx)}
              <span className="quiet"> · {l.contract}</span></li>
          ))}
        </ol>
        <dl className="kv oc__fees">
          {quote.feeLines.map((f, i) => <FeeRow key={i} label={f.label} value={`${pct(f.bps, 3)} · ${eth(f.eth, 6)} ETH`} />)}
          <FeeRow label="All fees and spreads" value={pct(quote.feesBps + quote.spreadBps, 3)} />
          <FeeRow label={quote.impactBps < 0 ? "Price improvement" : "Price impact"} value={pct(Math.abs(quote.impactBps), 3)} />
          <FeeRow label="Total cost" value={pct(quote.totalCostBps, 3)} />
        </dl>
        {quote.alternatives.length ? (
          <p className="quiet small">Also quoted: {quote.alternatives.map((r) => `${r.id === "floorModule" ? "the Floor Module" : "the pool"} (${r.legs.length ? `${buy ? coins(r.legs[r.legs.length - 1].amountOut.toString(), 4) + " " + symbol : eth(r.legs[r.legs.length - 1].amountOut, 6) + " ETH"}` : "no quote"}${r.why ? `: ${r.why}` : ""})`).join("; ")}.</p>
        ) : null}
        <p className="quiet small">{viaCurve
          ? `The launch curve's buy and sell take no deadline, so the ${deadlineMin}-minute setting does not apply to this trade; the minimum above does.`
          : `EtchRouter reverts the whole trade if it is not mined within ${deadlineMin} minutes or would give less than the minimum. ${buy ? `Your ETH becomes ${wx} and then ${symbol} inside one transaction; the ${wx} never reaches your wallet.` : `Your ${symbol} becomes ${wx} and then ETH inside one transaction.`}`}</p>
      </details>
    </div>
  );
}
const FeeRow = ({ label, value }: { label: string; value: string }) => <><dt>{label}</dt><dd className="num">{value}</dd></>;

// ---- proof of reserves --------------------------------------------------------------------------------------------------------

export const STATUS_WORD: Record<ReserveStatus, string> = { green: "Verified", red: "Mismatch", unverified: "Unverified" };
export const StatusBadge = ({ status }: { status: ReserveStatus }) => <span className={`wx-status wx-status--${status}`}>{STATUS_WORD[status]}</span>;

const units = (s: string | null) => (s === null ? "not known" : (() => { const v = BigInt(s), E = 10n ** 18n, f = (v % E).toString().padStart(18, "0").replace(/0+$/, ""); return `${v / E}${f ? "." + f.slice(0, 6) : ""}`; })());

/**
 * docs/v5/floor-module.md §13.1: the ETH-backed units are two tiers, C (convertible: the canister buys items for them) and R (the
 * standing reserve: never converted), each backed by its own pot (B, S); `escrowBalance()` is both pots. The float units are the
 * module's own inventory, part of C. All of it is one term of the equation: (items − in flight) × 1 + (C + R) = supply.
 */
export function Equation({ r }: { r: ReserveReport }) {
  const e = r.equation, sym = r.symbol, m = r.floorModule;
  return (
    <div className="wx-eq" aria-label="The reserve equation">
      <p className="wx-eq__line num">
        (<span title="items found in custody by the public sources">{e.itemsInCustody}</span>
        {" − "}<span title="burned tickets whose item has not left yet">{e.ticketsInFlight}</span>) × 1
        {" + "}(<span title="convertible units: ETH-backed, the canister buys items for them">{units(e.convertibleUnits)}</span>
        {" + "}<span title="reserve units: ETH-backed by the standing reserve, never converted">{units(e.reserveUnits)}</span>)
        {" = "}<strong>{units(e.backingUnits)}</strong>
        <span className="wx-eq__vs"> {e.holds === true ? "=" : e.holds === false ? "≠" : "?"} </span>
        <strong>{units(e.supplyUnits)}</strong> {sym} in supply
      </p>
      <dl className="kv wx-eq__terms">
        <dt>Items found in custody</dt><dd className="num">{e.itemsInCustody} of {e.heldInLedger}{e.itemsUnverified ? `, ${e.itemsUnverified} unverified` : ""}{e.itemsElsewhere ? `, ${e.itemsElsewhere} found elsewhere` : ""}</dd>
        <dt>Tickets in flight</dt><dd className="num">{e.ticketsInFlight} <span className="quiet">— burned, item not yet delivered: still in custody, backing nothing in circulation</span></dd>
        <dt>ETH-backed units</dt><dd className="num">{m.kind === "read" ? <>{units(e.ethBackedUnits)} = {units(e.convertibleUnits)} convertible + {units(e.reserveUnits)} reserve <span className="quiet">— of which {units(e.floatUnits)} float units the module holds as inventory</span></> : m.kind === "absent" ? "0 — no Floor Module is deployed for this collection" : `not read: ${m.error}`}</dd>
        <dt>ETH behind them</dt><dd className="num">{m.kind === "read" ? <>{eth(BigInt(e.escrowWei), 4)} ETH in escrow = {eth(BigInt(m.backingWei), 4)} convertible pot + {eth(BigInt(m.standingReserveWei), 4)} standing reserve <span className="quiet">— both pots, `escrowBalance()`</span></> : m.kind === "absent" ? "0 by construction" : "not read"}</dd>
        <dt>{sym} supply</dt><dd className="num">{units(e.supplyUnits)} <span className="quiet">at block {r.block ?? "—"} on Robinhood Chain</span></dd>
        <dt>The wrapper's own books</dt><dd className="num">held {e.heldOnChain ?? "not read"}, open tickets {e.openTicketsOnChain ?? "not read"} <span className="quiet">— must match its events: {e.heldInLedger} held, {e.ticketsInFlight} open</span></dd>
        {e.itemsUnverified ? <><dt>If every unverified item is there</dt><dd className="num">{units(e.backingUnitsIfUnverifiedHold)}</dd></> : null}
      </dl>
    </div>
  );
}

const bps = (b: number) => `${(b / 100).toFixed(2)}%`;
const E4 = (w: string) => `${eth(BigInt(w), 4)} ETH`;

/** docs/v5/floor-module.md §13.4–§13.6: the module's price beside the keeper's floor, its spreads, its pots and its fills. */
export function ModuleBooks({ r, now }: { r: ReserveReport; now: number }) {
  const m = r.floorModule;
  if (m.kind === "absent") return <p className="quiet">{m.note}</p>;
  if (m.kind === "error") return <p className="quiet">The Floor Module could not be read: {m.error}</p>;
  const price = BigInt(m.priceWei), floor = BigInt(m.floorWei);
  const diff = floor > 0n ? Number(((price - floor) * 10_000n) / floor) : 0;
  return (
    <div className="wx-books">
      <dl className="figures">
        <FigureBox label="Price the module trades at" value={eth(price, 4)} unit="ETH" note={m.fills.length ? `The trailing-24h average of the protocol's own fills, ${diff === 0 ? "at" : `${diff > 0 ? "+" : "−"}${bps(Math.abs(diff))} from`} the keeper's floor, inside ±${bps(m.fillBandBps)} of it.` : "No fill in the last 24 hours: the keeper's accepted floor stands in."} />
        <FigureBox label="The keeper's floor" value={eth(floor, 4)} unit="ETH" note={m.floorFresh ? `Posted ${when(m.floorPostedAt)}; fresh, so the module trades.` : `Posted ${when(m.floorPostedAt)}; stale: nothing trades at the module until the next post.`} tone={m.floorFresh ? undefined : "warn"} />
        <FigureBox label="Spread, mint / burn" value={`${bps(m.spreadMintBps)} / ${bps(m.spreadBurnBps)}`} note="Base, volatility, staleness and confidence on both sides; the stress term on mints only." />
        <FigureBox label="Standing reserve" value={eth(BigInt(m.standingReserveWei), 4)} unit="ETH" note={`Backs ${units(m.reserveUnits)} reserve units (${eth(BigInt(m.betaReserveWei), 4)} ETH each); burns draw it first. Never converted into an item.`} />
      </dl>
      <dl className="kv kv--wide">
        <dt>Convertible pot</dt><dd className="num">{E4(m.backingWei)} behind {units(m.convertibleUnits)} units ({eth(BigInt(m.betaConvertibleWei), 4)} ETH each) <span className="quiet">— what the canister converts into items</span></dd>
        <dt>Float inventory</dt><dd className="num">{units(m.floatUnits)} units <span className="quiet">— bought ahead of demand with the treasury's float allocation; what `itemSold` sells</span></dd>
        <dt>Revenue pot</dt><dd className="num">{E4(m.revenueWei)} <span className="quiet">— realized spread; pays conversions above backing, skimmed to the treasury past its target</span></dd>
        <dt>Room right now</dt><dd className="num">mint up to {E4(m.mintRoomWei)} at {E4(m.mintPriceWei)}; burn up to {units(m.burnRoomUnits)} units at {E4(m.burnPriceWei)}</dd>
        <dt>Bootstrap loan owed</dt><dd className="num">{E4(m.loanOutstandingWei)}{m.treasury ? <span className="quiet"> — to the treasury <span className="mono">{short(m.treasury)}</span>; repaid before any other fee route</span> : null}</dd>
        <dt>7-day volume</dt><dd className="num">{E4(m.volume7dWei)} <span className="quiet">— the treasury's weight for this collection</span></dd>
      </dl>
      {m.fills.length ? (
        <ul className="rows">
          {m.fills.map((f, i) => <li key={i}><div className="row"><span className="row__main"><span className="row__name num">{eth(BigInt(f.priceWei), 4)} ETH</span><span className="tag">fill</span></span><span className="row__meta">{when(f.at)} · {countdown(now, f.at)} ago</span></div></li>)}
        </ul>
      ) : null}
    </div>
  );
}

/** docs/v5/floor-module.md §13.3: one treasury, four fixed percentages, no discretionary path. */
export function TreasuryPanel({ r }: { r: ReserveReport }) {
  const t = r.treasury;
  if (t.kind === "absent") return <p className="quiet">{t.note}</p>;
  if (t.kind === "error") return <p className="quiet">The treasury could not be read: {t.error}</p>;
  const total = BigInt(t.weightTotalWei);
  return (
    <div className="wx-books">
      <dl className="figures">
        <FigureBox label="Held" value={eth(BigInt(t.balanceWei), 4)} unit="ETH" note={`${E4(t.pots.unallocatedWei)} arrived since the last allocation.`} />
        <FigureBox label="Collection reserves pot" value={eth(BigInt(t.pots.reservesWei), 4)} unit="ETH" note={`${bps(t.shares.reservesBps)} of every wei, to each module's standing reserve by 7-day volume.`} />
        <FigureBox label="Item floats pot" value={eth(BigInt(t.pots.floatsWei), 4)} unit="ETH" note={`${bps(t.shares.floatsBps)}: mints inventory at the module ahead of demand.`} />
        <FigureBox label="Bootstrap pot" value={eth(BigInt(t.pots.bootstrapWei), 4)} unit="ETH" note={`${bps(t.shares.bootstrapBps)}: one loan of up to ${E4(t.bootstrapLoanWei)} per new collection, repaid first from its fees.`} />
      </dl>
      <dl className="kv kv--wide">
        <dt>Operations</dt><dd className="num">{bps(t.shares.opsBps)}, paid at once to {t.ops ? <span className="mono">{short(t.ops)}</span> : "nobody yet (unset: it joins the reserves pot)"} · {E4(t.totals.toOpsWei)} so far</dd>
        <dt>Ever received</dt><dd className="num">{E4(t.totals.receivedWei)} · allocated {E4(t.totals.allocatedWei)} · to reserves {E4(t.totals.toReservesWei)} · to floats {E4(t.totals.toFloatsWei)}</dd>
        <dt>Loans</dt><dd className="num">lent {E4(t.totals.lentWei)}, repaid {E4(t.totals.repaidWei)}</dd>
        <dt>Registered collections</dt><dd>{t.collections.length ? (
          <ul className="wx-list">{t.collections.map((c) => <li key={c.module} className="num"><span className="mono">{short(c.module)}</span> · weight {total > 0n ? bps(Number((BigInt(c.weightWei) * 10_000n) / total)) : "—"} ({E4(c.weightWei)} of 7-day volume) · loan outstanding {E4(c.outstandingLoanWei)}</li>)}</ul>
        ) : "none yet"}</dd>
      </dl>
      <p className="quiet small">The treasury has no function that names a recipient, no withdraw and no pause; the four shares sum to 100% and only the timelock changes them.</p>
    </div>
  );
}

/** docs/v5/floor-module.md §13.2: the protocol's own liquidity in the collection's lock, never withdrawable. */
export function LiquidityPanel({ r }: { r: ReserveReport }) {
  const l = r.protocolLiquidity;
  if (l.kind === "absent") return <p className="quiet">{l.note}</p>;
  if (l.kind === "error") return <p className="quiet">The lock could not be read: {l.error}</p>;
  return (
    <dl className="kv kv--wide">
      <dt>Protocol positions</dt><dd className="num">{l.positions} in the shared pool <span className="quiet">— the lock has no removing path: they stay for good</span></dd>
      <dt>Waiting to be placed</dt><dd className="num">{E4(l.pendingEthWei)} and {units(l.pendingWrappedUnits)} {r.symbol} <span className="quiet">— deferred by the price guard; anyone may retry with `deploy(PROTOCOL)`</span></dd>
      <dt>The shared pool, from every graduation</dt><dd className="num">{E4(l.sharedEthInWei)} and {units(l.sharedWrappedInUnits)} {r.symbol}</dd>
      <dt>ETH fees earned by the lock</dt><dd className="num">{E4(l.feesInEthWei)} in, {E4(l.feesPaidEthWei)} paid out <span className="quiet">— the protocol positions' ETH fees go to {l.treasury ? <>the treasury <span className="mono">{short(l.treasury)}</span></> : "more protocol liquidity (no treasury wired)"}</span></dd>
    </dl>
  );
}

function FigureBox({ label, value, unit, note, tone }: { label: string; value: string; unit?: string; note?: string; tone?: "warn" }) {
  return <div className={`figure${tone ? ` figure--${tone}` : ""}`}><dt>{label}</dt><dd><span className="num">{value}</span>{unit ? <span className="unit"> {unit}</span> : null}</dd>{note ? <p className="figure__note">{note}</p> : null}</div>;
}

// ---- the coin page: policy, tier, launch fee -------------------------------------------------------------------------------------

export const POLICY_LINE: Record<WrappedCoin["policy"], string> = {
  raffle: "Its fees buy items and raffle them to holders.",
  burn: "Its fees buy items and burn them.",
  yoyo: "Its fees buy items, relist them at a markup, and the sale proceeds buy the coin back and burn it.",
};

/** docs/v5/policies-and-fees.md §2: what a buy pays right now, recomputed from the clock so a fixture's countdown runs. */
export function launchExtraNow(lf: WrappedCoin["launchFee"], now: number): number {
  if (!lf || lf.endsAt === null) return lf?.extraBpsNow ?? 0;
  const left = lf.endsAt - now;
  return left <= 0 ? 0 : Math.floor((lf.bps * Math.min(left, lf.decayS)) / lf.decayS);
}

export function LaunchFeeLine({ coin, now }: { coin: WrappedCoin; now: number }) {
  const lf = coin.launchFee;
  if (!lf) return <span>No launch fee: a buy paid the tier alone from the first second.</span>;
  const extra = launchExtraNow(lf, now);
  if (extra === 0) return <span>The launch fee has decayed to nothing: a buy pays the {(coin.feeBps / 100).toFixed(coin.feeBps % 100 ? 1 : 0)}% tier alone.</span>;
  return <span>A buy right now pays <strong className="num">{bps(extra)}</strong> on top of the tier, falling to nothing {lf.endsAt !== null ? <>in <span className="num">{countdown(lf.endsAt, now)}</span></> : `over ${Math.round(lf.decayS / 60)} minutes`} — every wei of it to the coin's vault, none to the creator or the protocol.</span>;
}

/** YOYO only: the listings out on the market, the proceeds waiting for a buyback, and what was bought back and burned. */
export function YoyoPanel({ coin, wx }: { coin: WrappedCoin; wx: string }) {
  const y = coin.yoyo;
  if (!y) return null;
  return (
    <div className="wx-books">
      <dl className="figures">
        <FigureBox label="Open listings" value={String(y.openListings)} note={`Swept items relisted at ${(1 + y.markupBps / 10_000).toFixed(2)}× the keeper's cap, not yet sold or returned.`} />
        <FigureBox label="Proceeds waiting" value={eth(BigInt(y.proceedsWei), 4)} unit="ETH" note="Sale proceeds the keeper's next buyback spends. There is no other exit for them." />
        <FigureBox label="Bought back and burned" value={eth(BigInt(y.boughtBackWei), 4)} unit="ETH" note={`${coins(y.burnedCoins, 0)} ${coin.symbol} sent to 0x…dEaD so far.`} />
      </dl>
      {y.listings.length ? (
        <ul className="rows">
          {y.listings.map((l) => <li key={l.ticketId}><div className="row"><span className="row__main"><span className="row__name">Ticket {l.ticketId}</span><span className={`tag${l.stage === "sold" ? " tag--good" : ""}`}>{l.stage}</span></span>
            <span className="row__meta num">asked {eth(BigInt(l.askWei), 4)} ETH{l.proceedsWei ? ` · sold for ${eth(BigInt(l.proceedsWei), 4)} ETH` : ""}{l.at ? ` · ${when(l.at)}` : ""}</span></div></li>)}
        </ul>
      ) : <p className="quiet">Nothing relisted yet: the vault relists once it holds a whole {wx}.</p>}
    </div>
  );
}

// ---- docs/v6/offer-handins.md: the offer board -----------------------------------------------------------------------------------

export const HANDIN_WORD: Record<HandinSubmission["status"], string> = { open: "open", claimed: "bought and rewarded", slashed: "slashed", refunded: "refunded" };

export function HandinRows({ rows, now, fixture, custodian }: { rows: HandinSubmission[]; now: number; fixture: boolean; custodian: string | null }) {
  if (!rows.length) return <p className="quiet">None.</p>;
  return (
    <div className="scroll">
      <table className="wx-table">
        <thead><tr><th>#</th><th>Inscription</th><th>Asks (sats)</th><th>Bond</th><th>By</th><th>Submitted</th><th>What became of it</th></tr></thead>
        <tbody>
          {rows.map((x) => (
            <tr key={x.id} className={`wx-row--${x.status}`}>
              <td className="num">{x.id}</td>
              <td><span className="mono">{short(x.inscriptionId)}</span></td>
              <td className="num">{Number(x.askSats).toLocaleString("en-US")}</td>
              <td className="num">{eth(BigInt(x.bondWei), 3)} ETH</td>
              <td><span className="mono" title={x.submitter}>{short(x.submitter)}</span></td>
              <td className="num">{x.at ? when(x.at) : `block ${x.atBlock}`}</td>
              <td>{x.status === "open" ? <>open{x.slashable ? " · slashable by the custodian" : ""}{x.expired ? " · expired: the bond can be taken back" : ""}</>
                : <>{HANDIN_WORD[x.status]}{x.rewardPaidWei ? ` · reward ${eth(BigInt(x.rewardPaidWei), 3)} ETH` : ""}{x.settledTx ? <> · <Proof href={null} fixture={fixture}>{short(x.settledTx)}</Proof></> : ""}{x.settledAt ? ` · ${countdown(now, x.settledAt)} ago` : ""}</>}</td>
            </tr>
          ))}
        </tbody>
      </table>
      {custodian ? <p className="quiet small">Only the custodian, <span className="mono">{short(custodian)}</span> (the canister), can claim or slash; the submitter can refund after the expiry.</p> : null}
    </div>
  );
}

export function HandinTotals({ h }: { h: HandinReport }) {
  const t = h.totals, p = h.params;
  return (
    <dl className="figures">
      <FigureBox label="Reward pool" value={h.rewardPoolWei ? eth(BigInt(h.rewardPoolWei), 3) : "—"} unit="ETH" note={p ? `Pays ${eth(BigInt(p.rewardWei), 3)} ETH per confirmed purchase, at most ${eth(BigInt(p.rewardDayCapWei), 2)} ETH a day (${h.paidTodayWei ? eth(BigInt(h.paidTodayWei), 3) : "—"} paid today).` : undefined} />
      <FigureBox label="Open now" value={`${t.open}${p ? ` of ${p.maxOpen}` : ""}`} note={p ? `At most ${p.maxOpenPerSubmitter} per address. The canister tries the cheapest valid one first.` : undefined} />
      <FigureBox label="Rewards paid" value={eth(BigInt(t.rewardsPaidWei), 3)} unit="ETH" note={`${t.claimed} hand-in${t.claimed === 1 ? "" : "s"} bought and confirmed six blocks deep.`} />
      <FigureBox label="Bonds slashed" value={eth(BigInt(t.bondsSlashedWei), 3)} unit="ETH" note={`${t.slashed} invalid hand-in${t.slashed === 1 ? "" : "s"}; a slashed bond joins the reward pool. ${t.refunded} refunded.`} />
    </dl>
  );
}

const HOW: Record<string, string> = { purchase: "bought", deposit: "deposited by a holder", rental: "lent by its owner" };
const VERDICT: Record<CustodyItem["check"]["verdict"], string> = { "in-custody": "in custody", elsewhere: "NOT in custody", unverified: "unverified" };

export function ItemRows({ items, kind, fixture }: { items: CustodyItem[]; kind: "inscriptions" | "nft"; fixture: boolean }) {
  if (!items.length) return <p className="quiet">No items are in custody.</p>;
  return (
    <div className="scroll">
      <table className="wx-table">
        <thead><tr><th>{kind === "nft" ? "Token" : "Inscription"}</th><th>Arrived</th><th>How</th><th>Check</th><th>Sources</th></tr></thead>
        <tbody>
          {items.map((i) => (
            <tr key={i.itemId} className={`wx-row--${i.check.verdict}`}>
              <td><Proof href={i.refUrl} fixture={fixture}>{kind === "nft" ? `#${i.ref}` : short(i.ref)}</Proof>{i.promisedTo !== null ? <span className="tag tag--warn">drawn by ticket {i.promisedTo}</span> : null}</td>
              <td className="num">{when(i.arrival.mintedAt ?? 0)}{i.arrival.tx ? <> · <Proof href={i.arrival.txUrl} fixture={fixture}>{short(i.arrival.tx)}</Proof></> : null}</td>
              <td>{HOW[i.arrival.how]}</td>
              <td><span className={`wx-verdict wx-verdict--${i.check.verdict}`}>{VERDICT[i.check.verdict]}</span>{i.check.reason ? <span className="quiet small"> — {i.check.reason}</span> : null}
                {i.check.checkedAt ? <span className="quiet small"> · {when(i.check.checkedAt)}</span> : null}</td>
              <td className="small">{i.check.sources.length ? i.check.sources.map((s, k) => <span key={k} className="wx-src"><Proof href={s.url} fixture={fixture}>{s.name}</Proof>: {s.said.replace(/\b(0x)?[0-9a-f]{40,64}\b/gi, (h) => short(h))}</span>) : <span className="quiet">none answered</span>}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

export function TicketRows({ tickets, now, fixture }: { tickets: Ticket[]; now: number; fixture: boolean }) {
  if (!tickets.length) return <p className="quiet">No redemptions are waiting for delivery.</p>;
  return (
    <ul className="rows">
      {tickets.map((t) => (
        <li key={t.id}><div className="row">
          <div className="row__main"><span className="row__name">Ticket {t.id}</span><span className={`tag${t.overdue ? " tag--warn" : ""}`}>{ticketStage(t, now)}</span></div>
          <span className="row__meta">{t.toBurn ? "to be burned for good" : <>to <span className="mono">{short(t.destination)}</span></>}{t.itemRef ? <> · item {short(t.itemRef)}</> : null} · burned <Proof href={null} fixture={fixture}>{short(t.burnTx)}</Proof></span>
        </div></li>
      ))}
    </ul>
  );
}

/** One line for where a ticket stands, the deadline included. */
export function ticketStage(t: Ticket, now: number): string {
  if (t.stage === "pinned") return `waiting for drand round ${t.round}`;
  if (t.stage === "resolved") return t.deliverBy === null ? "drawn, waiting for delivery"
    : t.deliverBy <= now ? `overdue since ${when(t.deliverBy)} — refundable` : `drawn, deliver by ${when(t.deliverBy)}`;
  return t.stage === "delivered" ? "delivered" : "refunded";
}

export const short = (s: string) => (s.length > 18 ? `${s.slice(0, 8)}…${s.slice(-6)}` : s);
