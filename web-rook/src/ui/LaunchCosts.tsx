import { useId, useState, type ReactNode } from "react";
import { solText } from "../rook/sol/format";
import { reviewFirstBuy, reviewTotal, reviewWx, wxUnits, type Tok } from "../rook/sol/firstBuy";
import type { Costs } from "../rook/sol/launchWizard";

/** The one-line note under the costs when there is a first buy: `fee` is launchWizard.ts `launchBuyFeeShort(config)`. */
export const firstBuyNote = (fee: string, slippageBps: number) => `The first buy pays ${fee} and allows up to ${slippageBps / 100} % slippage.`;

/** One row of the review panel: label left, the value on one line right, an optional muted sub-line under it, an optional explanation. */
function Row({ label, value, sub, why, total }: { label: string; value: ReactNode; sub?: string | null; why?: string | null; total?: boolean }) {
  return (
    <div className={`lcost__row${total ? " lcost__row--total" : ""}`}>
      <dt>{label}</dt>
      <dd><span className="lcost__v">{value}</span>{sub ? <small className="lcost__sub">{sub}</small> : null}</dd>
      {why ? <dd className="lcost__why">{why}</dd> : null}
    </div>
  );
}
const Said = ({ v }: { v: { main: string; text: string } }) => <><span aria-hidden="true">{v.main}</span><span className="vh">{v.text}</span></>;

/** The review's Graduation rows: the threshold and the vault's share, each in SOL at the day-1 floor over the tokens (firstBuy.ts). */
export function LaunchGraduation({ tok, wx, backingWx }: { tok: Tok; wx: number; backingWx: number }) {
  const g = reviewWx(tok, wxUnits(wx)), b = reviewWx(tok, wxUnits(backingWx));
  return (
    <dl className="lcost">
      <Row label="Graduates at" value={<Said v={g} />} sub={g.sub} />
      <Row label="To the coin's vault" value={<Said v={b} />} sub={b.sub} />
    </dl>
  );
}

/**
 * The review step's cost lines (launchWizard.ts `launchCosts`). The accounts are the ones `initialize_virtual_pool_with_spl_token`
 * creates (trade.ts `launchIxs`): the curve pool, its two token vaults, the coin's mint and its Metaplex metadata; a first buy also
 * opens the buyer's token account for the coin. A first buy on a collection's curve is in its wX: SOL at the recorded floor is the
 * figure, the tokens under it (firstBuy.ts). One short explanation per row, behind "What are these?".
 */
export function LaunchCosts({ costs, firstBuy, quote, explain = false }: { costs: Costs; firstBuy: bigint; quote: { isSol: boolean; tok: Tok }; explain?: boolean }) {
  const [open, setOpen] = useState(explain);
  const id = useId();
  const buy = reviewFirstBuy(quote.tok, firstBuy, quote.isSol);
  const total = reviewTotal(quote.tok, costs.total, costs.firstBuyWx);
  const w = (s: string) => (open ? s : null);
  return (
    <div>
      <dl className="lcost" id={id}>
        <Row label="Account rent" value={`${solText(costs.rent, 4)} SOL`} why={w("Solana's deposit for your coin's accounts. Not refunded.")} />
        <Row label="Metadata fee" value={`${solText(costs.metaplexFee, 4)} SOL`} why={w("Metaplex's charge for the name, ticker and image.")} />
        <Row label="Network fee" value={`< ${solText(costs.networkFee, 4)} SOL`} why={w("Solana's transaction fee (upper estimate).")} />
        <Row label="First buy" value={<Said v={buy} />} sub={buy.sub}
          why={w(firstBuy <= 0n ? "Optional: buy your own coin at launch." : quote.isSol ? "Your own coin, bought at launch. In the total." : "Spent from your wallet; not in the SOL total. SOL is an estimate at the recorded floor.")} />
        <Row label="Total" value={<Said v={total} />} sub={total.sub} total />
      </dl>
      <button type="button" className="lcost__toggle" aria-expanded={open} aria-controls={id} onClick={() => setOpen((o) => !o)}>{open ? "Hide explanations" : "What are these?"}</button>
    </div>
  );
}
