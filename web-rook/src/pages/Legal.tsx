import type { ReactNode } from "react";
import { href } from "../lib/env";
import { GOVERNING_LAW, LEGAL_CONTACT, LEGAL_ENTITY, LEGAL_UPDATED } from "../config";

/** Three pages that describe THIS product. Not legal advice, unreviewed, and they say so. The blanks are the owner's and their counsel's (src/config.ts). */
const Blank = ({ what, value }: { what: string; value: string }) => value ? <>{value}</> : <mark className="blank" title="To be filled in by the owner with counsel">[{what}: not yet decided]</mark>;

function Page({ title, lede, children }: { title: string; lede: string; children: ReactNode }) {
  return (
    <article className="legal">
      <header className="pagehead"><p className="eyebrow">Legal</p><h1>{title}</h1><p className="lede">{lede}</p></header>
      <p className="notice notice--plain small" role="note"><strong>A draft, and it says so.</strong> Written on {LEGAL_UPDATED} to describe what Rook does. It is not legal advice, no lawyer has reviewed it, and it must be reviewed by one before Rook takes real money. Where it names the party behind Rook or a governing law, that is still to be decided, and the text shows a marked blank.</p>
      {children}
      <p className="small muted">Read with: <a href={href("terms")}>Terms</a> · <a href={href("risk")}>Risk</a> · <a href={href("privacy")}>Privacy</a> · <a href={href("trust")}>What is trusted</a></p>
    </article>
  );
}
const S = ({ title, children }: { title: string; children: ReactNode }) => <section><h2 style={{ fontSize: "var(--t1)" }}>{title}</h2>{children}</section>;

export function Terms() {
  return (
    <Page title="Terms of use" lede="What you agree to by using this site and the programs it talks to.">
      <S title="Who this is between"><p>These terms are between you and <Blank what="the operating entity" value={LEGAL_ENTITY} />. Contact: <Blank what="a contact address" value={LEGAL_CONTACT} />. Rook is not affiliated with Meteora, Jupiter, Phantom, Solflare, Backpack, OpenSea, Magic Eden or the collections' creators.</p></S>
      <S title="What Rook is"><p>A launchpad on Solana and this website. A person launches a coin on Meteora's Dynamic Bonding Curve under one of Rook's collection configs, which ties it permanently to one blue-chip NFT collection. The coin is priced in the collection's wrapped NFT token (wX). When the collection's graduation threshold (a set number of whole wX) has gone into the curve the coin graduates into a Meteora pool whose liquidity is permanently locked, and 50 % of the raise, already wX, goes into a vault held for the coin's holders by Rook's custodian, a canister on the Internet Computer; holders may burn coins for their share, and one whole wX may be redeemed for a real NFT.</p><p>This website is a way to read Meteora's programs and the custodian's books, and to ask your own wallet to call them. It holds no funds and no keys, and it can be replaced by any other interface.</p></S>
      <S title="What it is not"><ul><li>Not an investment, a fund, a savings product or a promise of any return. A coin launched here can go to zero, and most coins do.</li><li>No claim about the price of any coin, inscription or NFT. The custodian buys items that are listed for sale, at their asking price, like any other buyer.</li><li>Nobody is owed a purchase, a draw, a delivery or a sale by any date; each happens only if trading produces enough fees and the people in the next section do their parts.</li><li>Not custody of your coins. You hold them in your own wallet.</li></ul></S>
      <S title="Who you are trusting"><p>Meteora's programs enforce the curve, its fees and the locked pool; Rook's custodian keeps the backing and its books, and nothing on Solana enforces those but its code. <a href={href("trust")}>The trust page</a> lists everyone whose word something rests on: Meteora, the custodian canister and the timelock that controls it, the Internet Computer, the marketplaces and the RPC providers.</p></S>
      <S title="New code"><p>Coins trade on Meteora's programs, which Meteora operates and can upgrade. Rook's custodian is new code that has had internal reviews, with every finding fixed; new code can still hold bugs nobody has found. Use only what you can lose entirely.</p></S>
      <S title="Who may use it"><p>You must be an adult where you live and legally able to agree to this. You must not be a person or in a place subject to sanctions that apply to you or to us. Whether you may launch a coin, hold one, or take part in a random draw depends on where you live, and that is yours to find out. We do not know your jurisdiction and do not check it.</p></S>
      <S title="Governing law"><p>These terms are governed by the law of <Blank what="the governing law" value={GOVERNING_LAW} />.</p></S>
    </Page>
  );
}
export function Risk() {
  return (
    <Page title="Risk disclosure" lede="The ways this can lose you money, in plain words.">
      <S title="The coin"><p>A memecoin has no cash flows. Its price is what the next person pays. Most go to zero. A coin that never reaches its graduation threshold never graduates; there is no refund: you can only sell back into the curve, at the curve's price, less the fee.</p></S>
      <S title="The trade"><p>The ticket's estimate is the curve's own math, and every trade is simulated before your wallet is asked; neither is a fill. The price can move before your transaction lands, and the trade fails if it moves past your slippage. Every trade pays a fee in the collection's wX: 50 % in the first seconds after launch, falling in a straight line to 1 % after ten minutes, then 1 %. A buy in the launch transaction itself pays the 50 %.</p></S>
      <S title="The backing"><p>A graduated coin's backing is wX in a vault only the custodian can move. A wX is backed by an NFT the custodian holds on Ethereum, Robinhood Chain, Bitcoin or Solana, or by SOL it holds for the collection at the floor price; the floor can fall. The custodian's books, not Solana's validators, say what backs what; the reserves page lets anyone check them, and you trust the custodian as the trust page describes.</p></S>
      <S title="Meteora"><p>Meteora can upgrade the curve and pool programs with one key. A change there could change how every Rook coin behaves, and Rook cannot stop it.</p></S>
      <S title="Software"><p>A new canister (internally audited), Meteora's programs, and this site. A bug could lock funds permanently.</p></S>
    </Page>
  );
}
export function Privacy() {
  return (
    <Page title="Privacy" lede="What this site records about you: nothing.">
      <S title="No cookies, no analytics"><p>This site sets no cookies and runs no analytics. It remembers your theme choice and the wallet you last connected in your own browser's storage, and nothing else.</p></S>
      <S title="What leaves your browser"><p>Reads of Solana, simulations and the transactions you sign go to the site's RPC provider, which sees your IP address and the accounts the page reads, as any RPC does. Coin images and metadata load from wherever their creators host them, with no referrer; collection logos come from this site. If you upload a coin's image or metadata, it goes to the operator's Cloudinary account and is public. Your wallet signs in its own window; we never see a key.</p></S>
      <S title="Public by nature"><p>Every transaction you send is public for ever on the chain it is sent to, and a Bitcoin address you give as a delivery destination is written on chain in the redemption ticket.</p></S>
    </Page>
  );
}
