import type { ReactNode } from "react";
import { href } from "../lib/env";
import { GOVERNING_LAW, LEGAL_CONTACT, LEGAL_ENTITY, LEGAL_UPDATED } from "../config";

/**
 * docs/phase6.md, Part A. Three pages that describe THIS product. They are not legal advice and have not been reviewed by a
 * lawyer; each says so at its top. The operating entity and the governing law are blanks on purpose (src/config.ts).
 */
const Blank = ({ what, value }: { what: string; value: string }) => value ? <>{value}</> : <mark className="blank" title="To be filled in by the owner with counsel">[{what}: not yet decided]</mark>;

function Page({ title, lede, children }: { title: string; lede: string; children: ReactNode }) {
  return (
    <article className="legal">
      <header className="pagehead"><h1>{title}</h1><p>{lede}</p></header>
      <p className="notice notice--plain" role="note"><strong>A draft, and it says so.</strong> Written on {LEGAL_UPDATED} to describe what this product does. It is not legal advice, no lawyer has reviewed it, and it must be reviewed by one before any contract is deployed to Robinhood Chain. Where it names the party behind Rook or a governing law, that is still to be decided, and the text shows a marked blank.</p>
      {children}
      <p className="quiet legal__also">Read with: <a href={href("terms")}>Terms of use</a> · <a href={href("risk")}>Risk disclosure</a> · <a href={href("privacy")}>Privacy</a> · <a href={href("trust")}>What is trusted</a></p>
    </article>
  );
}
const S = ({ n, title, children }: { n: number; title: string; children: ReactNode }) => <section className="legal__s" aria-labelledby={`l-${n}`}><h2 id={`l-${n}`}><span className="num legal__n">{n}</span>{title}</h2>{children}</section>;

export function Terms() {
  return (
    <Page title="Terms of use" lede="What you agree to by using this site and the contracts it talks to.">
      <S n={1} title="Who this is between">
        <p>These terms are between you and <Blank what="the operating entity" value={LEGAL_ENTITY} />. Contact: <Blank what="a contact address" value={LEGAL_CONTACT} />. Rook is not affiliated with Robinhood Markets, Pons, Nosh, UniSat or Magic Eden.</p>
      </S>
      <S n={2} title="What Rook is">
        <p>A set of contracts on Robinhood Chain and this website. A person launches a coin on Pons through the Rook Launcher and pairs it, permanently, with one listing: an Ordinals collection or a rune. The coin's creator fees go to a small contract that has no owner. It sends 80% to that listing's vault and 20% to the protocol's treasury, every time, as constants in its code. When a vault can afford it, a purchase of the listed Bitcoin asset is declared in public, waits an hour, and is released to a purchaser who has posted a bond. Inscriptions go to one holder by a random draw or another mode the coin's creator chooses; runes are shared among holders in proportion to their balances.</p>
        <p>This website is a way to read those contracts and to ask your own wallet to call them. It holds no funds and no keys, and it can be replaced by any other interface. The contracts do what their code says whether or not this site exists.</p>
      </S>
      <S n={3} title="What it is not">
        <ul>
          <li>It is not an investment, a fund, a savings product or a promise of any return. A coin launched here can go to zero, and most coins do.</li>
          <li>It makes no claim about the price of any coin, inscription or rune, and nothing here should be read as one. The vault buys items that are listed for sale, at their asking price, like any other buyer.</li>
          <li>A coin's creator receives nothing from the fees. Nobody is owed a purchase, a draw, a share or a delivery by any date: each happens only if trading produces enough fees, a purchase is possible, and the people in the next section do their parts.</li>
          <li>It is not custody of your assets. You hold your own coins in your own wallet.</li>
        </ul>
      </S>
      <S n={4} title="Who you are trusting">
        <p>The contracts enforce the fee split, the public wait, the spending cap, the bond and the randomness of a draw. They cannot reach Bitcoin. So some things rest on people, and <a href={href("trust")}>the trust page</a> lists all of them. In short:</p>
        <ul>
          <li><strong>The purchaser</strong> receives the released ETH and buys the asset on Bitcoin. If they do not, their bond repays the coins that paid. The bond limits the loss; it does not make the purchase happen.</li>
          <li><strong>The attestor</strong> states that the asset arrived. You can check it against the vault's public Bitcoin address.</li>
          <li><strong>The keeper</strong> posts the price cap and publishes the holder list behind a draw or a distribution. The list is public for fifteen minutes first, and anyone can rebuild it; nothing on chain rejects a wrong one.</li>
          <li><strong>The owner</strong> can pause a listing, change roles after a public delay, cancel a declared purchase and void a draw that cannot settle. The owner cannot withdraw from a vault.</li>
          <li><strong>Pons</strong> decides when pool fees are swept, and its owner can redirect a coin's fee recipient after its own three-day public delay. <strong>A marketplace</strong> can refuse to sell to us at any time. <strong>drand</strong> supplies the randomness.</li>
        </ul>
      </S>
      <S n={5} title="Unaudited code">
        <p>The contracts have not been audited by an independent firm. We reviewed them ourselves and found and fixed serious bugs, which is evidence that others may remain. A bug could lock funds in a vault permanently: a vault has no withdraw function, by design, and nobody can rescue what is stuck in one. Use only what you can lose entirely.</p>
      </S>
      <S n={6} title="Who may use it">
        <p>You must be an adult where you live and legally able to agree to this. You must not be a person or in a place subject to sanctions that apply to you or to us, and you must not use Rook where doing so is unlawful. Whether you may launch a coin, hold one, or take part in a random draw for a prize depends on where you live, and <strong>that is yours to find out</strong>. We do not know your jurisdiction and do not check it.</p>
      </S>
      <S n={7} title="Draws and distributions">
        <p>A draw picks one ticket using a drand round that did not exist when the draw was pinned; nobody, including us, can choose or re-roll it. Winning depends on chance. A winner must post a Bitcoin address within the claim period and may correct it for one hour; an asset sent to a wrong address you gave cannot be recovered. A rune share below the posted minimum is not sent on its own; it waits until your shares add up. Unclaimed shares roll into the coin's next distribution.</p>
      </S>
      <S n={8} title="Your responsibilities">
        <ul>
          <li>Your wallet, your keys, your transactions and their gas are yours. A transaction, once sent, cannot be undone.</li>
          <li>Taxes on anything you receive are yours to work out and pay.</li>
          <li>Do not use Rook to break the law, to launder money, to manipulate a market, or to impersonate a project you do not represent. Do not name a coin or use an image in a way that infringes someone else's rights.</li>
        </ul>
      </S>
      <S n={9} title="No warranty, and limits on liability">
        <p>The site and the contracts are provided as they are, without warranty of any kind, to the extent the law allows. To that same extent, <Blank what="the operating entity" value={LEGAL_ENTITY} /> is not liable for losses from bugs, from the acts or failures of the parties in section 4, from a marketplace, a bridge or a chain failing or changing, from a delivery that is late or never made, or from anything you chose to do with your own wallet. Some places do not allow some of these limits; where they do not, the limit is the smallest the law allows.</p>
      </S>
      <S n={10} title="Changes, and the law that applies">
        <p>The contracts cannot be changed by anyone. This site and these terms can, and the date at the top will say when. These terms are governed by <Blank what="the governing law and the courts or forum" value={GOVERNING_LAW} />.</p>
      </S>
    </Page>
  );
}

export function Risk() {
  return (
    <Page title="Risk disclosure" lede="The ways this can go badly, in order of how much money they touch.">
      <S n={1} title="The coin itself">
        <p>A coin launched here is a memecoin on a bonding curve. Most go to zero. Early buyers can sell into later ones. The coin's price has nothing to do with what its vault holds: holders have no claim on the vault, and a purchase for holders does not support the price. Pons charges a fee on every trade, and launch-second buyers pay its anti-snipe tax.</p>
      </S>
      <S n={2} title="Fees may never buy anything">
        <p>About 0.70% of trading volume, plus the creator tax, reaches a coin's router, and 80% of that reaches the vault. At a 1% creator tax it takes roughly 74 ETH of trading to fund one 1-ETH purchase. Most coins never trade that much, and most never graduate from the curve. A vault can sit below the price of one item for ever, and what is in it cannot be withdrawn by anyone. After graduation, fees arrive only when Pons's own operator sweeps the pool, on its schedule.</p>
      </S>
      <S n={3} title="The purchase happens off this chain">
        <p>No contract on Robinhood Chain can buy a Bitcoin asset. Released ETH goes to a purchaser who bridges it and buys. They may fail, be late, or run away; the bridge may fail or cost more than quoted; the marketplace may refuse the order or close its API to us. The purchaser's bond repays the coins that paid if a purchase is written off, which limits the loss to what the bond does not cover. It does not produce the asset.</p>
      </S>
      <S n={4} title="Receipts, lists and caps are somebody's word">
        <p>The receipt for a purchase is the attestor's statement; the holder list behind a draw is the keeper's; the price cap is the keeper's reading of a market this chain cannot see. Each is public and each can be checked by anyone, against Bitcoin or by rebuilding the list with the published tool. Nothing on chain rejects a false one. If nobody checks, nobody will know.</p>
      </S>
      <S n={5} title="Deliveries">
        <p>An inscription or runes are sent to the Bitcoin address the winner or holder posts. A wrong address is final. Deliveries can be late; a late one is flagged on chain and nothing forces it. Sending runes costs Bitcoin fees, so small shares wait under a posted minimum and may wait a long time.</p>
      </S>
      <S n={6} title="Code and keys">
        <p>The contracts are unaudited. Our own review found two high-severity bugs and fixed them; there may be more. There is no upgrade and no rescue function: a bug that locks a vault locks it for good. If the owner's keys were stolen, the thief could pause listings, change roles after a public delay, and cancel or void things; they still could not withdraw from a vault. If the keeper's, purchaser's or attestor's keys were stolen, the damage is bounded by the cap, the bond and the public waits, and it is not zero.</p>
      </S>
      <S n={7} title="Things outside anyone here's control">
        <p>Robinhood Chain, Pons, drand, a bridge and a marketplace can each change, fail, or end. Pons's owner can redirect a coin's fees after three days' notice. The network that supplies randomness could be retired; a draw then waits for the owner to void it. Laws about tokens, prize draws and digital assets differ by country and change; something allowed today may not be tomorrow, and a random draw for a prize is restricted or banned in some places.</p>
      </S>
      <S n={8} title="Staging and test deployments">
        <p>A page with a STAGING ribbon is reading a copy of the chain we run ourselves: its ETH is worthless, and its prices, receipts and deliveries are made up. A page with a TEST DEPLOYMENT ribbon is reading real contracts on Robinhood Chain that hold small amounts of real money and are just as unaudited as everything else.</p>
      </S>
    </Page>
  );
}

export function Privacy() {
  return (
    <Page title="Privacy" lede="What this site learns about you, which is very little, and what a public chain shows about everyone.">
      <S n={1} title="What the site does not do">
        <p>It sets no cookies. It has no accounts, no sign-up, no analytics, no advertising and no tracking pixels. It loads nothing from third parties: the fonts and scripts are served from this site. It remembers one thing in your browser, whether you chose the light or the dark theme, and that never leaves your device.</p>
      </S>
      <S n={2} title="What is public whatever we do">
        <p>A blockchain is a public record. Your wallet address, every transaction it sends, the coins it holds, a draw it wins and the Bitcoin address a winner posts are visible to everyone, permanently, and cannot be deleted by us or by anyone. If you post a Bitcoin address to receive an asset, you are linking it to your Robinhood Chain address in public. Use addresses you are content to have linked.</p>
      </S>
      <S n={3} title="What our indexer stores">
        <p>To load pages quickly, a service of ours reads the contracts' public events and keeps them in a database: launches, fees, purchases, draws, addresses and amounts. It holds nothing that is not already on the chain. It has no accounts and receives nothing from your browser except the page requests themselves.</p>
      </S>
      <S n={4} title="Logs">
        <p>The companies that host this site and that service keep ordinary server logs (the address a request came from, the time, the page) for a limited period, for security and to keep the service running. We use them for nothing else. The hosting provider today is Railway. When you read the chain directly, your wallet or browser talks to an RPC provider, which sees your requests under its own policy; when you verify a delivery you may visit a Bitcoin explorer, which has its own.</p>
      </S>
      <S n={5} title="Your wallet">
        <p>Connecting a wallet tells this page your address so it can show what is yours and prepare a transaction for you to approve. The page never sees your keys, and nothing is sent without your approval in your own wallet.</p>
      </S>
      <S n={6} title="Who is responsible, and your rights">
        <p>The party responsible for the site and the indexer is <Blank what="the operating entity" value={LEGAL_ENTITY} />, reachable at <Blank what="a contact address" value={LEGAL_CONTACT} />. Depending on where you live you may have rights to ask what is held about you and to have it corrected or deleted. For server logs we will do so. For anything recorded on a blockchain nobody can, which is why the second section matters more than this one.</p>
      </S>
    </Page>
  );
}
