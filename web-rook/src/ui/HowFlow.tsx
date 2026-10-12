/**
 * The picture at the top of "How it works" (src/content/docs/02-how-it-works.md): three lanes of a few steps each, a coin's life, the
 * NFTs behind the wrapped tokens, and the way back out. Plain HTML and CSS (src/styles/rook.css `.flow`): left to right on a wide
 * screen, top to bottom on a phone, readable without the picture (the page below says the same in words).
 */
const LANES: { name: string; steps: [string, string, boolean?][] }[] = [
  { name: "A coin's life", steps: [
    ["Launch", "A creator launches a coin on a collection; the first launch on a collection opens it."],
    ["Trade", "On a Meteora bonding curve, priced in the collection's wrapped token (wX). Pay in SOL: SOL → wX → coin in one transaction."],
    ["Graduate", "At the collection's threshold in whole wX: half the raise goes into the coin's own vault, the rest into a locked pool.", true],
    ["Exit", "Burn coins for their share of the vault's wX."],
  ] },
  { name: "Behind each wX", steps: [
    ["Mint", "wX is minted only by the custodian, for SOL at the floor price (an intent), or for an NFT taken in."],
    ["Buy", "The SOL buys an NFT near the floor: OpenSea on Ethereum and Robinhood Chain, Ordinals Wallet or UniSat on Bitcoin, Tensor on Solana."],
    ["Hold", "The NFT sits at the custodian's address on its own chain; the Reserves page checks it.", true],
  ] },
  { name: "Out to an NFT", steps: [
    ["Redeem", "Send 1 whole wX to a redemption intent, naming where the NFT goes."],
    ["Draw", "A future drand round picks which NFT in custody, at random."],
    ["Deliver", "The custodian sends it to your address on its chain within 7 days, or mints your wX back.", true],
  ] },
];

export function HowFlow() {
  return (
    <figure className="flow" aria-label="How Rook works, in three lanes">
      {LANES.map((l) => (
        <div key={l.name} className="flow__lane">
          <p className="flow__name">{l.name}</p>
          <ol className="flow__steps">{l.steps.map(([title, text, key]) => <li key={title} className={`flow__step${key ? " flow__step--key" : ""}`}><strong>{title}</strong>{text}</li>)}</ol>
        </div>
      ))}
    </figure>
  );
}
