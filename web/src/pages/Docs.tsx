import { href } from "../lib/env";

/** The four things that happen, in the order they happen. It is a real sequence, so it is drawn as one line and not as cards. */
const STATIONS: { verb: string; drop: string; text: string }[] = [
  { verb: "Launch", drop: "0rem", text: "You launch a coin on Pons through the Launcher and pair it with one vault: the one that buys an Ordinals collection, or the one that buys a rune. The pairing is permanent." },
  { verb: "Trade", drop: "2.6rem", text: "Every trade pays Pons's creator fee to a small contract with no owner. It can do one thing: split what arrives, 80 to the vault and 20 to the protocol." },
  { verb: "Buy", drop: "5.2rem", text: "When the vault can afford one item under a posted price cap, a purchase is declared in public, waits an hour, and is released against the purchaser's bond." },
  { verb: "Distribute", drop: "7.8rem", text: "An inscription is drawn for among the coin's holders by a drand round nobody had seen. A rune is shared out pro rata. The winner names a Bitcoin address." },
];

/** Pairs from docs/trust-model.md: what a contract enforces on the left, the word it still rests on to the right. */
const LEDGER: [string, string, string, string][] = [
  ["The split", "80 to the vault, 20 to the treasury, a constant in the router's bytecode. The router has no owner and cannot be upgraded.",
    "Pons's owner", "Pons's protocol owner can redirect any coin's creator fees after its own three-day public timelock. Fees would stop arriving; the vault keeps what it holds."],
  ["No way out but a purchase", "A vault has no withdraw function. ETH leaves only through a declared purchase that waited an hour, fits the cap, and is covered by a bond.",
    "The purchase itself", "It happens on Bitcoin, where no contract here can reach. A purchaser who takes the ETH and buys nothing forfeits the bond, and the coins are repaid from it."],
  ["The cap binds the spend", "A declaration cannot exceed the posted cap times the quantity, at most three are open at once, and one overdue receipt stops every new one.",
    "The cap's number", "The keeper's statement about a market this chain cannot read. It expires in an hour and names its source, but nobody on chain checks it."],
  ["The draw cannot be aimed", "A draw is pinned to a drand round that does not exist yet, and the round's signature is verified on chain. Anyone may request it; anyone may deliver it.",
    "The holder list", "The snapshot behind a draw is the keeper's word. It is public for fifteen minutes first, and anyone can rebuild it byte for byte with the open tool."],
  ["Two keys, not one", "The address that receives released ETH can never be the address that attests the receipt, and a rune payout can never exceed what was acquired.",
    "The receipt", "The attestor's word that the asset arrived. Check it yourself: the vault's Bitcoin address is public, and the transaction is either there or it is not."],
];

const FAQ: [string, string][] = [
  ["Does a coin's creator earn anything?", "No. The creator's fee goes to the router, and the router pays the vault and the treasury. What a creator gets is one decision: what happens to each asset their coin's fees paid for."],
  ["Do I need a collection's permission?", "No. A vault is created by the registry's owner, and any coin may pair with a collection or rune that has one. Purchases are ordinary purchases of existing listings at their asking price."],
  ["What happens after a coin graduates?", "Fees keep arriving at the same router at the same rate: 0.70% of volume plus the creator tax, measured on a fork and against real graduated coins. Pons's own operator sweeps pool fees, on its schedule."],
  ["Who can take the money?", "Nobody with a key can withdraw from a vault. The exposure is ETH released for a purchase: bounded by the cap, limited to three open purchases, and covered by the purchaser's bond."],
  ["Why is my coin's vault not buying?", "A coin joins the queue once it has sent 0.01 ETH to its vault, and the vault buys when it can afford one item under a live cap. The coin's page shows how much more trading that takes."],
  ["Has this been audited?", "No. There has been an internal security pass with sixteen findings fixed, and it is not an audit. Until one is done and published, treat every contract here as unaudited."],
];

/** A tile cut into the panel: a shadowed upper-left wall, a lit lower-right lip, the floor between. */
const Tile = ({ x, y, w, h, win }: { x: number; y: number; w: number; h: number; win?: boolean }) => (
  <g><rect className="fig-lo" x={x - 1} y={y - 1} width={w + 1} height={h + 1} /><rect className="fig-hi" x={x} y={y} width={w + 1} height={h + 1} /><rect className={`fig-cell${win ? " fig-cell--win" : ""}`} x={x} y={y} width={w} height={h} /></g>
);
function Raffle() {
  // ninety-six holders, one drawn
  const cols = 16, rows = 6, size = 15, gap = 4.4, win = 3 * cols + 11;
  return (
    <svg viewBox="0 0 320 150" role="img" aria-label="Ninety-six holders in a grid; one of them is drawn">
      {Array.from({ length: cols * rows }, (_, i) => <Tile key={i} x={6 + (i % cols) * (size + gap)} y={6 + Math.floor(i / cols) * (size + gap)} w={size} h={size} win={i === win} />)}
      <text className="fig-num" x="6" y="143">96 holders</text><text className="fig-num fig-num--win" x="314" y="143" textAnchor="end">1 drawn, by a drand round</text>
    </svg>
  );
}
function ProRata() {
  // one purchase divided in true proportion: each bar's height is that holder's share
  const shares = [26, 18, 13, 10, 8, 7, 6, 5, 4, 3], w = 24, gap = 7.5, floor = 118, scale = 4.1;
  return (
    <svg viewBox="0 0 320 150" role="img" aria-label="One purchase of a rune divided among ten holders in proportion to their balances">
      {shares.map((sh, i) => { const x = 6 + i * (w + gap), h = sh * scale; return <g key={i}><Tile x={x} y={floor - h} w={w} h={h} win={i === 0} /><text className={`fig-num${i === 0 ? " fig-num--win" : ""}`} x={x + w / 2} y={floor + 14} textAnchor="middle">{sh}%</text></g>; })}
      <text className="fig-num" x="6" y="147">each holder's share of the purchase is their share of the coin</text>
    </svg>
  );
}

export function Docs() {
  return (
    <>
      <header className="pagehead">
        <h1>How it works</h1>
        <p>What happens when a coin is launched here, what the contracts enforce, and what is still somebody's word. Nothing on this page reads a chain: it is the same whether anything is deployed or not.</p>
      </header>

      <div className="landing">
      <section className="band" aria-labelledby="facts-h">
        <h2 className="band__title" id="facts-h">Cut into the contracts</h2>
        <dl className="facts">
          <div className="fact"><dd>80<small>/</small>20</dd><dt>The split between a listing's vault and the protocol. A constant in bytecode.</dt></div>
          <div className="fact"><dd>1<small>h</small></dd><dt>Every purchase waits in public before any ETH is released.</dt></div>
          <div className="fact"><dd>15<small>min</small></dd><dt>Every holder snapshot is open to challenge before a draw can use it.</dt></div>
          <div className="fact"><dd>0</dd><dt>Withdraw functions, on any vault. ETH leaves only inside a bonded purchase.</dt></div>
        </dl>
      </section>

      <section className="band" aria-labelledby="how-h">
        <h2 className="band__title" id="how-h">How it works</h2>
        <div className="line">
          <svg className="line__svg" viewBox="0 0 1000 192" preserveAspectRatio="none" aria-hidden="true">
            {["line__seam", "line__glint", "line__ember"].map((c) => (
              <path key={c} className={c} pathLength={1} transform={c === "line__glint" ? "translate(0 2.5)" : undefined}
                d="M0 8 H215 C245 8 235 50 265 50 H470 C500 50 490 92 520 92 H725 C755 92 745 134 775 134 H1000" />))}
          </svg>
          <ol className="stations">{STATIONS.map((s) => <li key={s.verb} className="station" style={{ ["--drop" as string]: s.drop }}><h3>{s.verb}</h3><p>{s.text}</p></li>)}</ol>
        </div>
      </section>

      <section className="band" aria-labelledby="holds-h">
        <h2 className="band__title" id="holds-h">What holds, and what is trusted</h2>
        <div className="stack">
          <p className="band__lede">The buying happens on Bitcoin, where no contract on this chain can reach. So here is the whole ledger, both columns.</p>
          <div className="ledger">
            <div className="ledger__head" aria-hidden="true"><span>The contract enforces</span><span>Still somebody's word</span></div>
            {LEDGER.map(([a, at, b, bt]) => <div className="ledger__row" key={a}><p><strong>{a}</strong>{at}</p><p><strong>{b}</strong>{bt}</p></div>)}
          </div>
          <p className="quiet">This is a summary. <a href={href("trust")}>The full trust model</a> lists every actor, what each can and cannot do, and ten residual risks in order of the money they touch.</p>
        </div>
      </section>

      <section className="band" aria-labelledby="kinds-h">
        <h2 className="band__title" id="kinds-h">Two kinds of listing</h2>
        <div className="kinds">
          <article className="kind"><div className="kind__fig"><Raffle /></div><h3>Inscriptions are drawn for</h3>
            <p>An inscription cannot be divided, so it goes to one holder. The coin's creator chooses how: tickets by balance, one ticket a wallet, a named recipient, or holding it in the vault.</p></article>
          <article className="kind"><div className="kind__fig"><ProRata /></div><h3>Runes are shared out</h3>
            <p>A rune is fungible, so a purchase is divided among holders in proportion to their balances. Each claims with a proof. Amounts under the posted minimum wait and roll into the next distribution.</p></article>
        </div>
      </section>

      <section className="band" aria-labelledby="split-h">
        <h2 className="band__title" id="split-h">Where the fees go</h2>
        <div className="split-fig">
          <ul className="flow" aria-label="The path of a fee"><li>a trade on Pons</li><li>0.70% of volume, plus the creator tax</li><li>the coin's router</li><li>split, every time</li></ul>
          <div className="split-bar" role="img" aria-label="80 percent to the vault, 20 percent to the treasury, drawn to scale"><div>80</div><div>20</div></div>
          <div className="split-key"><p><strong>The listing's vault</strong>Can only fund declared, capped, bonded purchases of that one collection or rune.</p><p><strong>The treasury</strong>The protocol's share.</p></div>
          <p className="quiet">None of it goes to the coin's creator. The numbers are constants in the router's bytecode, and the router has no owner.</p>
        </div>
      </section>

      <section className="band" aria-labelledby="faq-h">
        <h2 className="band__title" id="faq-h">Questions</h2>
        <div className="faq">{FAQ.map(([q, a]) => <details key={q}><summary>{q}</summary><p>{a}</p></details>)}</div>
      </section>

      <section className="band" aria-labelledby="next-h">
        <h2 className="band__title" id="next-h">Read on</h2>
        <div className="stack">
          <p className="band__lede">This page is the summary. The trust model is the long answer, and it is the one worth your time.</p>
          <p className="quiet"><a href={href("trust")}>What is trusted</a> lists every actor, what each can and cannot do, and ten residual risks in the order of the money they touch. <a href={href("risk")}>The risk disclosure</a> says what can go wrong for you.</p>
        </div>
      </section>
      </div>
    </>
  );
}
