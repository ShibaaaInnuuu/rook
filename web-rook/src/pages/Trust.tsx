import { useMemo } from "react";
import { marked } from "marked";
import trustModel from "../content/trust-model.md?raw";
import { href } from "../lib/env";
import { Glossary } from "../ui/bits";

/** The Solana statement of what is trusted (docs/v9/canister-only-solana.md §10), then docs/trust-model.md as it is in the
 *  repository, rendered and not rewritten (the canister-only model; the earlier EVM one is docs/v5/trust-model-evm.md). */
export function Trust() {
  const html = useMemo(() => marked.parse(trustModel, { async: false, gfm: true }) as string, []);
  return (
    <>
      <SolanaStatement />
      <header className="pagehead"><p className="eyebrow">Trust · the repository's trust model</p><h1>What is trusted, word for word</h1>
        <p className="lede">The repository's own trust model for canister-only Rook: who can do what, what is left to somebody's word, and what has a test that tries to break it. If a sentence here is more comfortable than the code justifies, that is a bug: say so. Read with the <a href={href("terms")}>terms</a>, the <a href={href("risk")}>risk disclosure</a> and the <a href={href("privacy")}>privacy note</a>.</p>
        <Glossary /></header>
      <article className="prose" dangerouslySetInnerHTML={{ __html: html }} />
    </>
  );
}

function SolanaStatement() {
  return (
    <section className="block rise" aria-labelledby="sol-trust" style={{ marginTop: 0 }}>
      <header className="pagehead"><p className="eyebrow">Trust · Solana</p><h1 id="sol-trust">What you are trusting on Solana</h1>
        <p className="lede">Coins launch and trade on Meteora's Dynamic Bonding Curve under Rook's configs, priced in each collection's wrapped NFT token (wX). Rook deploys no programs of its own: the custodian, a canister on the Internet Computer, is wX's only minter and keeps the books. This is everyone whose word something rests on.</p></header>
      <div className="prose">
        <h2>Meteora</h2>
        <p>The curve (DBC 0.2.1, <span className="mono">dbcij3LW…aqN</span>), the graduation pool (DAMM v2, <span className="mono">cpamdpZC…sGG</span>) and the wX/SOL pools (DLMM) are Meteora's programs. Meteora can upgrade them with one key, with or without Rook. Rook's configs are permanent: DBC has no instruction that changes or closes one, and nothing in them can be changed by Rook either: not the fees, the curve, the graduation threshold (each collection's number of whole wX), the 50 % backing share, nor who may claim it.</p>
        <h2>The custodian</h2>
        <p>Every config's fee claimer, every wX's mint authority and every vault's owner is a key of the custodian: a threshold key split across Internet Computer nodes, which signs only when the nodes jointly run the custodian's code. The custodian writes every mint, burn and payout to its books before it signs, checks the chain against the books first, and undoes the entry if the transaction never lands. Changes to its code wait behind a <strong>72-hour timelock</strong>. It reads Solana through two providers that must agree, and verifies every drand round itself.</p>
        <p>What Solana's validators do not enforce, the custodian's code does: that is the trade. It is made checkable rather than trusted blind: every book is a public query, every wX mint or burn carries a label naming its book entry, and the <a href={href("reserves")}>reserves page</a> matches the books against Solana, Ethereum, Robinhood Chain and Bitcoin every few minutes.</p>
        <h2>The code</h2>
        <p>The custodian's Solana code has been through internal review rounds with proof-of-concept tests and fixes (docs/v9/audit-2026-10 in the repository: no Critical; every High, Medium and Low found is fixed), the newest parts included: buying and delivering Solana NFTs, the Ordinals beyond Bitcoin Frogs, Robinhood Chain, and opening collections on demand. It has also run end to end on Solana's public test network. New code can still hold bugs nobody has found.</p>
        <h2>This site</h2>
        <p>The site holds no key and no funds. It reads Solana through an RPC endpoint and the custodian's public queries, builds each transaction in your browser, simulates it, and asks your wallet to sign; you can read every account it names before you sign. A different site, or the Meteora SDK, can do everything this one does, except open an intent with the custodian, which any client of its public interface can.</p>
      </div>
      <hr style={{ border: 0, borderTop: "1px solid var(--line)", margin: "2.5rem 0" }} />
    </section>
  );
}
