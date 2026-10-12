import type { HandinReport, HandinSubmission, ReserveReport, WrappedCoin } from "../api/types";
import { countdown, eth, short, when } from "../lib/format";
import { items } from "../rook/ledger";
import { moduleStateOf } from "../rook/moduleState";
import { launchExtraNow } from "../rook/tiers";
import { Fig, Pill, Proof, ago } from "./bits";

/**
 * The books the indexer reads at one block, drawn in the site's idiom: the Floor Module's price, spreads, pots and fills
 * (docs/v5/floor-module.md §13.4–§13.6), the protocol treasury (§13.3), the protocol's own liquidity (§13.2), a YOYO coin's
 * loop (docs/v5/policies-and-fees.md §1) and a collection's offer board (docs/v6/offer-handins.md). Presentation only: every
 * number is a field of the report that came in, so a fixture and a live read draw the same.
 */
const E4 = (w: string) => `${eth(BigInt(w), 4)} ETH`;
const bps = (b: number) => `${(b / 100).toFixed(2)}%`;
const units = (w: string) => items(BigInt(w));

export function ModuleBooks({ r, now, fixture }: { r: ReserveReport; now: number; fixture: boolean }) {
  const m = r.floorModule;
  if (m.kind === "absent") return <p className="small muted">{m.note}</p>;
  if (m.kind === "error") return <p className="small warn">The floor module could not be read: {m.error}</p>;
  const price = BigInt(m.priceWei), floor = BigInt(m.floorWei);
  // R4-check A4-I3: "trades" only with a fresh floor AND its keeper up (a pause within the hour leaves the last post fresh)
  const state = moduleStateOf(m);
  const keeperDown = state === "keeper-down";
  const diff = floor > 0n ? Number(((price - floor) * 10_000n) / floor) : 0;
  return (
    <div className="stack">
      <div className="figs">
        <Fig label="Price the module trades at" value={eth(price, 4)} unit="ETH" />
        <Fig label="The keeper's floor" value={eth(floor, 4)} unit="ETH" />
        <Fig label="Spread, mint / burn" value={`${bps(m.spreadMintBps)} / ${bps(m.spreadBurnBps)}`} />
        <Fig label="Standing reserve" value={eth(BigInt(m.standingReserveWei), 4)} unit="ETH" />
      </div>
      <p className="small muted">{m.fills.length ? `The trailing-24h average of the protocol's own fills, ${diff === 0 ? "at" : `${diff > 0 ? "+" : "−"}${bps(Math.abs(diff))} from`} the keeper's floor, held inside ±${bps(m.fillBandBps)} of it.` : "No fill in the last 24 hours: the keeper's accepted floor stands in."} The floor was posted {when(m.floorPostedAt)}{state === "trades" ? "; fresh, so the module trades." : keeperDown ? <>; <span className="warn">the keeper is paused (or none is named): nothing is minted at the module, and anyone can burn, from the moment of the pause, at the pause floor{m.pauseFloorWei && BigInt(m.pauseFloorWei) > 0n ? ` (${eth(BigInt(m.pauseFloorWei), 4)} ETH: the floor the keeper left, at most its fill band's low edge and the cheapest recent mint)` : " (the floor the keeper left, at most its fill band's low edge and the cheapest recent mint)"} or the backing per unit, whichever is lower, less the base spread; at the backing itself only after a week with no good post.{m.refundsHeldUntil ? ` Unfilled curves' refunds are held until ${when(m.refundsHeldUntil)} (6 h after the unpause).` : ""}</span></> : m.refundsHeldUntil && !m.escaped ? <>; <span className="warn">the keeper was unpaused: unfilled curves' refunds are held until {when(m.refundsHeldUntil)} (6 h after the unpause), so the keeper can post and the runner fill first.</span></> : m.escaped ? <>; <span className="warn">stale for over 7 days: the stale fallback is on. Anyone can burn at the backing per unit, less the base spread, with no floor price; nothing else trades at the module until a good post.</span></> : <>; <span className="warn">stale: nothing is minted at the module until the next post. Burns resume at the backing, with no floor, if no good post comes for 7 days.</span></>} The mint side's spread carries the stress term; the burn side's does not.</p>
      <dl className="kv kv--books">
        <dt>Convertible pot</dt><dd>{E4(m.backingWei)} behind {units(m.convertibleUnits)} {r.symbol}, {eth(BigInt(m.betaConvertibleWei), 4)} ETH each<small>what the canister converts into NFTs</small></dd>
        <dt>Standing reserve</dt><dd>{E4(m.standingReserveWei)} behind {units(m.reserveUnits)} {r.symbol}, {eth(BigInt(m.betaReserveWei), 4)} ETH each<small>what the reserve's tokens paid in; a burn is never paid more than that per unit</small></dd>
        {m.surplusWei !== undefined && m.surplusWei !== null ? <><dt>Reserve surplus</dt><dd>{E4(m.surplusWei)}<small>curve fee slices, the treasury's reserve share, item-sale proceeds and the spread burns keep; spent only on conversions, or sent to the treasury after 7 stale days — never paid out through burns</small></dd></> : null}
        <dt>Float inventory</dt><dd>{units(m.floatUnits)} {r.symbol}<small>bought ahead of demand with the treasury's float allocation</small></dd>
        <dt>Revenue pot</dt><dd>{E4(m.revenueWei)}<small>realized spread; skimmed to the treasury past its target</small></dd>
        <dt>Room right now</dt><dd>mint up to {E4(m.mintRoomWei)} at {E4(m.mintPriceWei)}; burn up to {units(m.burnRoomUnits)} {r.symbol} at {E4(m.burnPriceWei)}<small>the leaky bucket and the caps; the ticket quotes only inside it</small></dd>
        <dt>Bootstrap loan owed</dt><dd>{E4(m.loanOutstandingWei)}{m.treasury ? <small>to the treasury <span className="mono">{short(m.treasury)}</span>; repaid before any other fee route</small> : null}</dd>
        <dt>7-day volume</dt><dd>{E4(m.volume7dWei)}<small>the treasury's weight for this collection</small></dd>
      </dl>
      {m.fills.length ? (
        <div className="scroll"><table className="tape">
          <thead><tr><th>Fill</th><th className="r">Implied floor</th><th className="r">When</th></tr></thead>
          <tbody>{m.fills.map((f, i) => <tr key={i}><td className="muted">{i === 0 ? "latest" : `${i + 1} back`}</td><td className="r num">{eth(BigInt(f.priceWei), 4)} ETH</td><td className="r num muted" title={when(f.at)}>{ago(f.at, now)}</td></tr>)}</tbody>
        </table></div>
      ) : null}
      <p className="small muted">Module <Proof href={null} fixture={fixture}>{m.address}</Proof>{r.block ? ` at block ${r.block.toLocaleString("en-US")}` : ""}.</p>
    </div>
  );
}

export function TreasuryPanel({ r }: { r: ReserveReport }) {
  const t = r.treasury;
  if (t.kind === "absent") return <p className="small muted">{t.note}</p>;
  if (t.kind === "error") return <p className="small warn">The treasury could not be read: {t.error}</p>;
  const total = BigInt(t.weightTotalWei);
  return (
    <div className="stack">
      <div className="figs figs--half">
        <Fig label="Held" value={eth(BigInt(t.balanceWei), 4)} unit="ETH" />
        <Fig label={`Reserves pot · ${bps(t.shares.reservesBps)}`} value={eth(BigInt(t.pots.reservesWei), 4)} unit="ETH" />
        <Fig label={`Floats pot · ${bps(t.shares.floatsBps)}`} value={eth(BigInt(t.pots.floatsWei), 4)} unit="ETH" />
        <Fig label={`Bootstrap pot · ${bps(t.shares.bootstrapBps)}`} value={eth(BigInt(t.pots.bootstrapWei), 4)} unit="ETH" />
      </div>
      <dl className="kv kv--books">
        <dt>Unallocated</dt><dd>{E4(t.pots.unallocatedWei)}<small>arrived since the last allocation</small></dd>
        <dt>Operations</dt><dd>{bps(t.shares.opsBps)} of every wei, {E4(t.totals.toOpsWei)} so far<small>{t.ops ? <>paid at once to <span className="mono">{short(t.ops)}</span></> : "nobody set: it joins the reserves pot"}</small></dd>
        <dt>Ever received</dt><dd>{E4(t.totals.receivedWei)}<small>allocated {E4(t.totals.allocatedWei)} · to reserves {E4(t.totals.toReservesWei)} · to floats {E4(t.totals.toFloatsWei)}</small></dd>
        <dt>Loans</dt><dd>lent {E4(t.totals.lentWei)}, repaid {E4(t.totals.repaidWei)}<small>one loan of up to {E4(t.bootstrapLoanWei)} per new collection, repaid first from its fees</small></dd>
        <dt>Registered collections</dt><dd>{t.collections.length ? t.collections.map((c) => <span key={c.module} style={{ display: "block" }}><span className="mono">{short(c.module)}</span> · weight {total > 0n ? bps(Number((BigInt(c.weightWei) * 10_000n) / total)) : "—"} · loan {E4(c.outstandingLoanWei)}</span>) : "none yet"}<small>each module's weight is its 7-day mint + burn volume</small></dd>
      </dl>
      <p className="small muted">The treasury has no function that names a recipient, no withdraw and no pause; the four shares sum to 100% and only the timelock changes them. Treasury <span className="mono break">{t.address}</span>.</p>
    </div>
  );
}

export function LiquidityPanel({ r }: { r: ReserveReport }) {
  const l = r.protocolLiquidity;
  if (l.kind === "absent") return <p className="small muted">{l.note}</p>;
  if (l.kind === "error") return <p className="small warn">The lock could not be read: {l.error}</p>;
  return (
    <div className="stack">
      <dl className="kv kv--books">
        <dt>Protocol positions</dt><dd>{l.positions} in the shared pool<small>the lock has no removing path: they stay for good</small></dd>
        <dt>Waiting to be placed</dt><dd>{E4(l.pendingEthWei)} and {units(l.pendingWrappedUnits)} {r.symbol}<small>deferred by the price guard; anyone may retry with deploy(PROTOCOL)</small></dd>
        <dt>The shared pool, from every graduation</dt><dd>{E4(l.sharedEthInWei)} and {units(l.sharedWrappedInUnits)} {r.symbol}</dd>
        <dt>ETH fees earned by the lock</dt><dd>{E4(l.feesInEthWei)} in, {E4(l.feesPaidEthWei)} paid out<small>{l.treasury ? <>the protocol positions' ETH fees go to the treasury <span className="mono">{short(l.treasury)}</span></> : "no treasury wired: the fees become more protocol liquidity"}</small></dd>
      </dl>
      <p className="small muted">Lock <span className="mono break">{l.lock}</span>.</p>
    </div>
  );
}

/** YOYO only: the listings out on the market, the proceeds waiting for a buyback, and what was bought back and burnt. */
export function YoyoPanel({ coin, wx, now, fixture }: { coin: WrappedCoin; wx: string; now: number; fixture: boolean }) {
  const y = coin.yoyo;
  if (!y) return null;
  const burned = Number(BigInt(y.burnedCoins)) / 1e18;
  return (
    <div className="stack">
      <div className="figs">
        <Fig label="Open listings" value={y.openListings} />
        <Fig label="Proceeds waiting" value={eth(BigInt(y.proceedsWei), 4)} unit="ETH" />
        <Fig label="Bought back" value={eth(BigInt(y.boughtBackWei), 4)} unit="ETH" />
        <Fig label={`${coin.symbol} burnt`} value={burned >= 1e6 ? `${(burned / 1e6).toFixed(2)}M` : burned.toLocaleString("en-US", { maximumFractionDigits: 0 })} />
      </div>
      <p className="small muted">NFTs bought from the floor go back on the market at {(1 + y.markupBps / 10_000).toFixed(2)}× the keeper's cap. What they sell for can only ever buy {coin.symbol} back and send it to a dead address; there is no other exit for the proceeds.</p>
      {y.listings.length ? (
        <div className="scroll"><table className="tape">
          <thead><tr><th>Ticket</th><th>Stage</th><th className="r">Asked</th><th className="r">Sold for</th><th className="r">When</th><th>Tx</th></tr></thead>
          <tbody>{y.listings.map((l) => <tr key={l.ticketId}><td className="num">#{l.ticketId}</td><td><Pill tone={l.stage === "sold" ? "good" : l.stage === "returned" ? "warn" : undefined}>{l.stage}</Pill></td><td className="r num">{eth(BigInt(l.askWei), 4)} ETH</td><td className="r num">{l.proceedsWei ? `${eth(BigInt(l.proceedsWei), 4)} ETH` : "—"}</td><td className="r num muted" title={l.at ? when(l.at) : ""}>{ago(l.at, now)}</td><td className="num"><Proof href={null} fixture={fixture}>{short(l.tx)}</Proof></td></tr>)}</tbody>
        </table></div>
      ) : <p className="small muted">Nothing relisted yet: the vault relists once it holds a whole {wx}.</p>}
    </div>
  );
}

// ---- docs/v6/offer-handins.md: the offer board ------------------------------------------------------------------------------------

const HANDIN_WORD: Record<HandinSubmission["status"], string> = { open: "open", claimed: "bought and rewarded", slashed: "slashed", refunded: "refunded" };

export function HandinTotals({ h }: { h: HandinReport }) {
  const t = h.totals, p = h.params;
  return (
    <div className="figs">
      <Fig label="Reward pool" value={h.rewardPoolWei ? eth(BigInt(h.rewardPoolWei), 3) : "—"} unit="ETH" />
      <Fig label="Paid today" value={h.paidTodayWei ? eth(BigInt(h.paidTodayWei), 3) : "—"} unit={p ? `of ${eth(BigInt(p.rewardDayCapWei), 2)} ETH` : "ETH"} />
      <Fig label="Open now" value={`${t.open}${p ? ` / ${p.maxOpen}` : ""}`} />
      <Fig label="Rewards paid" value={eth(BigInt(t.rewardsPaidWei), 3)} unit="ETH" />
      <Fig label="Bought" value={t.claimed} />
      <Fig label="Slashed" value={t.slashed} unit={`${eth(BigInt(t.bondsSlashedWei), 3)} ETH`} />
    </div>
  );
}

export function HandinRows({ rows, now, fixture, custodian }: { rows: HandinSubmission[]; now: number; fixture: boolean; custodian: string | null }) {
  if (!rows.length) return <p className="small muted">None.</p>;
  return (
    <>
      <div className="scroll"><table className="tape">
        <thead><tr><th className="r">#</th><th>Inscription</th><th className="r">Ask</th><th className="r">Bond</th><th>By</th><th className="r">Submitted</th><th>What became of it</th></tr></thead>
        <tbody>{rows.map((x) => (
          <tr key={x.id}>
            <td className="r num muted">{x.id}</td>
            <td className="num"><span className="mono" title={x.inscriptionId}>{short(x.inscriptionId)}</span></td>
            <td className="r num">{Number(x.askSats).toLocaleString("en-US")} sats</td>
            <td className="r num">{eth(BigInt(x.bondWei), 3)} ETH</td>
            <td className="num"><span className="mono" title={x.submitter}>{short(x.submitter)}</span></td>
            <td className="r num muted" title={x.at ? when(x.at) : ""}>{x.at ? ago(x.at, now) : `block ${x.atBlock}`}</td>
            <td className="small">{x.status === "open" ? <>{x.slashable ? <span className="warn">slashable by the custodian</span> : "waiting"}{x.expired ? " · expired: the bond can be taken back" : ""}</>
              : <>{HANDIN_WORD[x.status]}{x.rewardPaidWei ? <> · reward <span className="num">{eth(BigInt(x.rewardPaidWei), 3)} ETH</span></> : ""}{x.settledTx ? <> · <Proof href={null} fixture={fixture}>{short(x.settledTx)}</Proof></> : ""}{x.settledAt ? ` · ${ago(x.settledAt, now)}` : ""}</>}</td>
          </tr>))}</tbody>
      </table></div>
      {custodian ? <p className="small muted" style={{ marginTop: ".75rem" }}>Only the custodian, <span className="mono">{short(custodian)}</span> (the canister), can claim or slash; the submitter can refund an untried bond after the expiry.</p> : null}
    </>
  );
}

/** One sentence for the launch fee's state, on the coin page's Info tab. */
export function LaunchFeeLine({ coin, now }: { coin: WrappedCoin; now: number }) {
  const lf = coin.launchFee;
  if (!lf) return <>No launch fee: a buy paid the tier alone from the first second.</>;
  const extra = launchExtraNow(lf, now);
  if (extra === 0) return <>Decayed to nothing: a buy pays the {(coin.feeBps / 100).toFixed(coin.feeBps % 100 ? 1 : 0)}% tier alone. It opened at {(lf.bps / 100).toFixed(0)}% and fell to zero over {Math.round(lf.decayS / 60)} minutes.</>;
  return <>A buy right now pays <span className="num">{bps(extra)}</span> on top of the tier, falling to nothing {lf.endsAt !== null ? <>in <span className="num">{countdown(lf.endsAt, now)}</span></> : `over ${Math.round(lf.decayS / 60)} minutes`}; every wei of it to the coin's vault, none to the creator or the protocol.</>;
}
