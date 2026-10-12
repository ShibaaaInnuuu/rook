import type { ReactNode } from "react";
import { href } from "../lib/env";
import { countdown, eth, short, units, when } from "../lib/format";
import { ago } from "./panels";
import { CopyAddr, Ext, Figure, Section } from "./ui";
import { EntryArt } from "./Plate";
import { Monogram, RuneFace } from "./Plate";
import { BITCOIN_TX, BTC_ADDRESS, INSCRIPTION, RUNE } from "../lib/chain";
import { EXPLORER_ADDRESS } from "../lib/explorer";
import { isRune, runeTicker, type CatalogueEntry } from "../lib/catalogue";
import { DELIVERY_IS, groove, pct, type Amount, type PaidIn, type Quantity, type Sweep, type VaultFacts } from "../lib/vault";
import type { Listing } from "../lib/data";

/**
 * The vault's own page, built to answer the questions Nosh's collection page answers and no others invented for the sake of
 * a row. docs/phase14.md §8, which also records the four of those rows Etch cannot print and what it prints instead.
 *
 * Nothing here is new material. The figures are the page's existing `.figures` grid, the tables are the `.book__table`
 * inside `.book__scroll` that the trades and holders panel already uses, and every colour is a custom property out of
 * `styles/etch.css`. A table scrolls in its own box; the page body never scrolls sideways.
 */

/** A wei figure, or the fact that this source does not report it — which is never shown as a zero. */
function Wei({ label, value, digits = 4, note, tone, absent }: {
  label: string; value: Amount; digits?: number; note?: ReactNode; tone?: "warn" | "good"; absent?: ReactNode;
}) {
  if (value === null) return <Figure label={label} value="not reported" note={absent ?? <>This source does not carry it. It is left unknown rather than shown as zero.</>} />;
  return <Figure label={label} value={eth(value, digits)} unit="ETH" note={note} tone={tone} />;
}

/** "263 inscriptions", or "371,165.08 DEMO•RUNE over 26 receipts". One row, two kinds, neither standing in for the other. */
function quantityWords(q: Quantity, divisibility: number, ticker: string): { value: string; unit?: string; note?: string } {
  if (q === null) return { value: "not reported" };
  if (q.kind === "rune") return { value: units(q.units, divisibility), unit: ticker, note: `over ${q.receipts} receipt${q.receipts === 1 ? "" : "s"}` };
  return { value: q.items.toLocaleString("en-US"), unit: q.items === 1 ? "inscription" : "inscriptions" };
}

export function VaultHead({ listing, entry, explorer, chain }: {
  listing: Listing; entry: CatalogueEntry | undefined; explorer: string | null; chain: string | null;
}) {
  const rune = listing.kind === "rune";
  // the reference's header carries the NFT contract; Etch's asset has no contract, so what stands in its place is where the
  // asset actually lives — the Bitcoin address every receipt for this vault has to pay into
  const real = /^bc1[a-z0-9]{20,}$/.test(listing.btcVault);
  return (
    <header className="vhead">
      <div className="vhead__art">
        {entry ? <EntryArt entry={entry} tag={listing.paused ? "paused" : "vault"} />
          : rune ? <span className="plate plate--rune"><RuneFace symbol={null} name={listing.name} /><span className="plate__tag">vault</span></span>
            : <span className="plate"><Monogram id={`listing-${listing.id}-${listing.ref}`} text={listing.name} /><span className="plate__tag">vault</span></span>}
      </div>
      <div className="vhead__say">
        <h1>{listing.name}</h1>
        <p className="vhead__tags">
          <span className="tag">{rune ? "rune" : "inscriptions"}</span>
          {chain ? <span className="tag">{chain}</span> : null}
          {listing.paused ? <span className="tag tag--warn">paused by the owner</span> : null}
          <span className="mono quiet">{listing.ref}</span>
        </p>
        <p className="vhead__links">
          {rune ? <Ext href={RUNE(listing.name)}>the rune on ordinals.com</Ext> : null}
          {entry ? <Ext href={entry.sourceUrl}>{entry.source}</Ext> : null}
          {explorer ? <Ext href={EXPLORER_ADDRESS(explorer, listing.vault)}>the vault on the explorer</Ext> : null}
        </p>
        <dl className="kv kv--wide">
          <dt>Bitcoin address</dt>
          <dd>{real ? <Ext href={BTC_ADDRESS(listing.btcVault)}><span className="mono">{listing.btcVault}</span></Ext>
            : listing.btcVault ? <><span className="mono">{listing.btcVault}</span> <span className="quiet">(a placeholder, not a Bitcoin address)</span></>
              : <span className="quiet">not set: no receipt can be checked against one yet</span>}</dd>
        </dl>
        <p className="vhead__act">
          <a className="btn" href={href("launch", listing.id)}>Launch a coin</a>
          <span className="quiet small">A coin launched against this vault pays 80% of its fees into it, for good.</span>
        </p>
      </div>
    </header>
  );
}

export function VaultPanel({ f, listing, nowS }: { f: VaultFacts; listing: Listing; nowS: number }) {
  const rune = listing.kind === "rune";
  const ticker = rune ? runeTicker(listing.name) : "";
  const bought = quantityWords(f.bought, listing.divisibility, ticker);
  const held = quantityWords(f.held, listing.divisibility, ticker);
  const nothingHeld = f.held !== null && (f.held.kind === "inscriptions" ? f.held.items === 0 : f.held.units === 0n);
  // a figure that needs the chain and a figure the source never carried are missing for different reasons, and a row that
  // gave one reason for the other would be guessing at its own provenance
  const liveGone = f.unknown ? <>It is read from the chain when this page is answered, and that read did not come back — the line above says why.</> : undefined;

  return (
    <Section title="The vault" aside={<CopyAddr a={listing.vault} />}>
      {f.unknown ? <p className="banner banner--block" role="status">The vault's balances are not known right now: {f.unknown}. No figure is shown in their place.</p> : null}
      <dl className="figures">
        <Wei label="Budget" value={f.budget} absent={liveGone} note={<>ETH attributed to a coin and not already promised to a declaration — the only ETH a purchase can reach.
          {f.balance !== null && f.bond !== null ? <> The contract holds <span className="num">{eth(f.balance)}</span> ETH in all, of which <span className="num">{eth(f.bond)}</span> is the purchaser's bond.</> : null}</>} />

        {f.cap === null ? (
          <Figure label={rune ? "Price cap, per whole unit" : "Price cap, per inscription"} value="none" tone="warn"
            note="No cap has been posted. Nothing can be declared without one." />
        ) : (
          <Figure label={rune ? "Price cap, per whole unit" : "Price cap, per inscription"} value={eth(f.cap.wei, 6)} unit="ETH" tone={f.cap.live ? undefined : "warn"}
            note={f.cap.live
              ? <><span className="tag tag--good">live</span> The keeper's ceiling on what may be declared, about a market this chain cannot read. It expires in {countdown(f.cap.expiry, nowS)}.</>
              : <><span className="tag tag--warn">expired</span> It lapsed {when(f.cap.expiry)}. Nothing can be declared until the keeper posts a fresh one, which is what the hour is for.</>} />
        )}

        <Figure label="Cap posted" value={f.cap?.postedAt ? ago(f.cap.postedAt) : "not reported"}
          note={f.cap?.postedAt ? <>{when(f.cap.postedAt)}. A cap lasts one hour from the block that posted it.</>
            : f.cap ? <>This source does not say when the cap was posted; it says only when it expires, {when(f.cap.expiry)}.</> : "No cap to date."} />

        <Figure label="Buys now" value={f.buysNow === null ? "not reported" : rune ? f.buysNow.toFixed(2) : `${f.buysNow.toFixed(2)}×`}
          unit={f.buysNow === null ? undefined : rune ? `whole ${ticker}` : undefined}
          note={f.buysNow === null ? "It needs both the budget and a posted cap." : <>The budget divided by the cap: what it could pay for at the ceiling standing now. The keeper declares nothing under 0.05 ETH, and the cap expires within the hour.</>} />

        <Figure label="Bought" value={bought.value} unit={bought.unit} note={bought.note ?? (f.bought === null ? "This source does not carry the total." : "Each one receipted on Bitcoin, below.")} />
        <Wei label="Spent" value={f.spent} note={<>Released less what came back, over purchases that were settled.
          {f.released !== null && f.returned !== null ? <> <span className="num">{eth(f.released)}</span> ETH has been released in all and <span className="num">{eth(f.returned)}</span> came back.</> : null}</>} />
        <Wei label="Fed in" value={f.fedIn} note="Every deposit the vault has taken: the coins' own routers, and the treasury's share attributed to the protocol coin. Nothing else may pay in." />

        <Wei label="Unattributed" value={f.unattributed} tone={f.unattributed !== null && f.unattributed > 0n ? "warn" : undefined}
          absent={<>ETH in the contract that no coin is credited with. Etch has no way to donate: a deposit is refused from anything but that coin's own router or the treasury. {liveGone ?? "This source does not carry every term of the subtraction, so there is no figure."}</>}
          note={<>ETH in the contract that no coin is credited with and no bond accounts for. Etch has no way to donate: a deposit is refused from anything but that coin's own router or the treasury, so anything here was forced in, and it is never spendable.</>} />

        <Figure label="Held" value={nothingHeld ? "nothing" : held.value} unit={nothingHeld ? undefined : held.unit}
          note={nothingHeld ? "Nothing held. The next receipt lands here."
            : f.held === null ? "This source does not carry it."
              : <>Receipted and not yet handed on.{f.delivered ? <> {f.delivered} lot{f.delivered === 1 ? " has" : "s have"} been delivered.</> : null}
                {f.paidOut !== null && f.paidOut > 0n ? <> <span className="num">{units(f.paidOut, listing.divisibility)}</span> has been paid out on claims.</> : null}</>} />

        <Wei label="Purchaser's bond" value={f.bond} absent={liveGone} tone={f.bond !== null && f.requiredBond !== null && f.bond < f.requiredBond ? "warn" : undefined}
          note={<>Posted against what is released and unsettled.
            {f.requiredBond !== null && f.outstanding !== null ? <> <span className="num">{eth(f.requiredBond)}</span> ETH is required now, against <span className="num">{eth(f.outstanding)}</span> outstanding.</> : null} A write-off repays the charged coins out of it.</>} />

        {f.writtenOff !== null && f.writtenOff > 0n
          ? <Figure label="Written off, uncovered" value={eth(f.writtenOff)} unit="ETH" tone="warn" note="Released, never receipted, and beyond what the bond repaid. Recorded for good." />
          : null}
      </dl>

      {f.cap && f.buysNow !== null ? (
        <>
          <div className="groove" role="progressbar" aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.round(groove(f.buysNow) * 100)}
            aria-label={rune ? "How much of one whole unit the budget could pay for at the cap" : "How much of one inscription the budget could pay for at the cap"}>
            <div className="groove__fill" style={{ transform: `scaleX(${groove(f.buysNow)})` }} />
          </div>
          <p className="groove__key">
            <span><span className="num">{f.budget === null ? "?" : eth(f.budget)}</span> ETH the vault can spend</span>
            <span><span className="num">{eth(f.cap.wei, 6)}</span> ETH buys {rune ? "one whole unit" : "one inscription"} at the cap</span>
          </p>
        </>
      ) : null}
    </Section>
  );
}

/**
 * The honest counterpart of the reference's "Bought by the vault". It cannot be the same claim: Etch's asset is on Bitcoin
 * and this chain can neither see it nor escrow it, so there is no transaction in which the money and the thing move
 * together. docs/phase14.md §8.2.
 */
export function OnlyOneWayOut({ listing, btcVault }: { listing: Listing; btcVault: string }) {
  const real = /^bc1[a-z0-9]{20,}$/.test(btcVault);
  const receipt = (
    <li key="receipt"><strong>Receipted within three days</strong>, with a Bitcoin transaction paying the listing's public address — {real ? <Ext href={BTC_ADDRESS(btcVault)}><span className="mono">{btcVault}</span></Ext> : <><span className="mono">{btcVault}</span> <span className="quiet">(a placeholder, not a Bitcoin address)</span></>}.</li>
  );
  return (
    <Section title="How the ETH gets out" aside="one way, and it is not a purchase this chain can make">
      <div className="claim">
        <p>
          <strong>The vault does not buy anything itself.</strong> It releases ETH to {listing.autonomous
            ? <>one address fixed when the vault was built, which buys on Bitcoin and comes back with a receipt</>
            : <>one named purchaser, who buys on Bitcoin and comes back with a receipt</>}. This chain cannot see Bitcoin,
          so it cannot hold the asset, and there is no single transaction in which the money and the thing change hands
          together. What there is instead is a procedure nobody can shorten:
        </p>
        {listing.autonomous ? (
          <>
            <ul className="claim__steps">
              <li><strong>Announced in public.</strong> The buyer announces a purchase for no more than the posted cap allows, and the coins that will pay for it are charged right then, so nothing about it can change while it waits. It cannot be pulled for an hour. The owner may cancel it in that hour; so may anyone at all, seven days later, if the buyer has stopped.</li>
              <li><strong>Pulled to the buyer, and nowhere else.</strong> Only the buyer may pull, only after the hour, and the destination is not an argument: it is {listing.buyer ? <span className="mono">{listing.buyer}</span> : <>the address fixed at construction</>}, which no key can change. Changing it means a new vault.</li>
              {receipt}
              <li><strong>Or the vault stops.</strong> One purchase past its deadline and nothing more is announced until it is settled, returned or written off. There is no bond to repay the coins from: what they paid and did not get back stays on the record as exactly that. Seven days past the deadline, anyone may do the writing off.</li>
            </ul>
            <p>
              <strong>What that does not protect against.</strong> A bug in the buyer, a bridge that takes the ETH and never
              pays out, or a venue that lies about what it delivered. Each is bounded by the ceiling on what may be out at
              once{listing.maxInFlightWei === null ? null : <> — <span className="num">{eth(BigInt(listing.maxInFlightWei), 4)}</span> ETH</>} and by at
              most three purchases open together, and none is bounded to zero. And the receipt is the buyer's own word:
              v1 had a second key whose job was to look at Bitcoin and say what it saw, and this does not. The contract still
              refuses an inscription outside the committed collection, refuses one already held, and refuses to close a short
              delivery until the money for what never arrived is back — but that the Bitcoin transaction is real is checkable
              by you against Bitcoin, and by nothing here.
            </p>
            <p className="quiet">
              The contract has no function that takes a wei a coin paid in, and no function that names where money goes. The
              buyer's key is held by a subnet of the Internet Computer rather than by a person, which is what removes the
              bond — a key that cannot leave cannot be made to post collateral against leaving. The cost of that is stated
              rather than hidden: a buyer that is lost strands whatever this vault holds, for ever, because the address is
              immutable and there is no withdraw.
            </p>
          </>
        ) : (
          <>
            <ul className="claim__steps">
              <li><strong>Declared in public.</strong> The keeper declares a purchase for no more than the posted cap allows, and it cannot be released for an hour. The owner may cancel it in that hour; nobody else may do anything.</li>
              <li><strong>Released to a named purchaser.</strong> Anyone may release it once the hour is up. It goes to the purchaser the registry named at the declaration and nowhere else, and it fails if the bond would not cover it.</li>
              {receipt}
              <li><strong>Or the vault stops.</strong> One purchase past its deadline and nothing more is declared until it is settled or written off. A write-off repays the charged coins out of the purchaser's bond; what the bond does not cover is recorded for good. Seven days past the deadline, anyone may do the writing off.</li>
            </ul>
            <p>
              <strong>What that does not protect against.</strong> A purchaser who takes released ETH and buys nothing costs the
              coins whatever their bond does not cover. The loss is bounded — at most three declarations open at once, each
              within cap × quantity — but it is not bounded to zero, and a receipt is the attestor's word until somebody checks
              it against Bitcoin.
            </p>
            <p className="quiet">
              The contract has no function that takes a wei a coin paid in. It does have <span className="mono">withdrawBond</span>,
              which sends the <em>purchaser's own posted bond</em> back to the address that posted it — never below the share of
              what is still in their hands, and never while one of their purchases is overdue. That money was never the coins'.
            </p>
          </>
        )}
      </div>
    </Section>
  );
}

/**
 * The coins paying into this vault and their shares — and the sentence under it, which is where Etch parts company with the
 * reference most sharply. A share does not weight a draw here. docs/phase14.md §8.5.
 */
export function PaidInTable({ rows, listing, total }: { rows: PaidIn[]; listing: Listing; total: bigint }) {
  return (
    <Section title="Paired coins" aside={<>{rows.length} coin{rows.length === 1 ? "" : "s"} pay into this vault</>}>
      {!rows.length ? (
        <p className="empty">No coin has been paired with this vault yet. Until one is, nothing pays in and nothing can be declared.</p>
      ) : (
        <>
          <div className="book__scroll">
            <table className="book__table">
              <thead><tr><th scope="col">Coin</th><th scope="col">Address</th><th scope="col" className="book__r">Fed in</th><th scope="col" className="book__r">Share</th></tr></thead>
              <tbody>
                {rows.map((c) => (
                  <tr key={c.address}>
                    <td><a href={href("coin", c.address)}>{c.symbol ?? "unnamed"}</a></td>
                    <td className="mono quiet">{short(c.address)}</td>
                    <td className="num book__r">{eth(c.wei, 4)} <span className="quiet">ETH</span></td>
                    <td className="num book__r">{pct(c.share) ?? <span className="quiet">—</span>}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
          <p className="quiet small">
            <span className="num">{eth(total)}</span> ETH in all. A share here <strong>does not weight a draw</strong>: every
            receipt names the one coin whose fees paid for it, what happens to it is chosen by that coin's creator, and the
            draw for it is over that coin's own holders. What a bigger share buys is more receipts, not better odds on
            somebody else's.
          </p>
        </>
      )}
    </Section>
  );
}

/** What the vault has bought, newest first: what it is, what its purchase cost, who paid, and where the thing is now. */
export function SweepHistory({ rows, listing, total, demo }: { rows: Sweep[]; listing: Listing; total: number; demo: boolean }) {
  const rune = listing.kind === "rune";
  // a column of "not reported" forty-one times over is not information: where no row has the figure the column goes and the
  // absence is said once, under the table
  const cost = rows.some((s) => s.purchaseSpent !== null);
  return (
    <Section title="What the vault has bought" aside={<><span className="num">{total}</span> receipt{total === 1 ? "" : "s"} · <a href={href("draws")}>draws and distributions</a></>}>
      {!rows.length ? (
        <p className="empty">No purchase has been receipted yet. Each one will be listed here with its Bitcoin transaction, so you can check it yourself.</p>
      ) : (
        <>
          <div className="book__scroll">
            <table className="book__table">
              <thead>
                <tr>
                  <th scope="col">When</th>
                  <th scope="col">{rune ? "Amount" : "Inscription"}</th>
                  {cost ? <th scope="col" className="book__r">Its purchase cost</th> : null}
                  <th scope="col">Paid by</th>
                  <th scope="col">Now</th>
                  <th scope="col" className="book__r book__tx">Bitcoin</th>
                </tr>
              </thead>
              <tbody>
                {rows.map((s) => (
                  <tr key={s.id}>
                    <td className="quiet" title={s.at === null ? "this receipt's block had no time" : when(s.at)}>{ago(s.at)}</td>
                    <td>{rune
                      ? <span className="num">{units(s.amount, listing.divisibility)}</span>
                      : <Ext href={INSCRIPTION(s.assetRef)}><span className="mono">{s.assetRef.slice(0, 8)}…{s.assetRef.slice(-4)}</span></Ext>}</td>
                    {cost ? (
                      <td className="num book__r">
                        {s.purchaseSpent === null ? <span className="quiet">—</span> : <>{eth(s.purchaseSpent, 4)} <span className="quiet">ETH</span></>}
                        {s.purchaseItems !== null && s.purchaseItems > 1
                          ? <span className="book__usd quiet" title="The vault charges coins per purchase, not per item, so this is the purchase's figure and not this row's share of it.">purchase {s.purchaseId}, {s.purchaseItems} receipts</span>
                          : <span className="book__usd quiet">purchase {s.purchaseId}</span>}
                      </td>
                    ) : null}
                    <td><a href={href("coin", s.coin)}>{s.coinSymbol ?? short(s.coin)}</a></td>
                    <td><span className={`tag${s.state === "delivered" ? " tag--good" : s.state === "voided" || s.state === "cancelled" ? " tag--warn" : ""}`}>{DELIVERY_IS[s.state]}</span></td>
                    <td className="book__r book__tx"><Ext href={BITCOIN_TX(s.btcTxid)}><span className="mono">{s.btcTxid.replace(/^0x/, "").slice(0, 8)}…</span></Ext></td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
          <p className="quiet small">
            A receipt is the attestor's word. Check it yourself: the transaction should exist on Bitcoin and pay into the
            address above. If the link finds nothing, the receipt is false{demo ? " (in this demo every Bitcoin value is a fixture, so it will find nothing)" : ""}.
            {!cost ? <> This source does not report what each receipt's purchase cost the vault, so there is no column for it;
              the Purchases table above carries what each declaration was for.</> : null}
            The sats a receipt records belong to its <em>purchase</em>: a rune settle writes one row per paying coin out of
            one Bitcoin transaction, all carrying the same figure, so nothing here adds them up.
          </p>
        </>
      )}
    </Section>
  );
}

export { isRune };
