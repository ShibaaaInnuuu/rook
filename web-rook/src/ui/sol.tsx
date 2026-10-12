/** Small pieces the Solana pages share. Presentation only; the data comes from src/rook/sol/hooks.ts. */
import type { ReactNode } from "react";
import { href } from "../lib/env";
import { SITE as SOL, useOffChain } from "../rook/sol/hooksLite";
import { short, solscan } from "../rook/sol/format";
import { COLLECTIONS, collectionBySlug, type SolCollection } from "../rook/sol/collections";
import { thresholdOf, wxText } from "../rook/sol/graduation";
import { Picture } from "./Picture";

export const CollectionLogo = ({ c, size = 40 }: { c: Pick<SolCollection, "slug" | "name" | "logo">; size?: number }) => <Picture src={c.logo} seed={c.slug} name={c.name} size={size} />;

/** A coin's picture from its metadata JSON's image, or the initials mark. */
export function CoinFace({ uri, name, seed, size = 32 }: { uri: string | null | undefined; name: string; seed: string; size?: number }) {
  const off = useOffChain(uri);
  return <Picture src={off.data?.image ?? null} seed={seed} name={name || "?"} size={size} round="50%" />;
}

export const Addr = ({ id, kind = "account" }: { id: string; kind?: "account" | "token" | "tx" }) =>
  <a className="mono break" href={solscan(kind, id)} target="_blank" rel="noreferrer noopener" title={id}>{short(id)}</a>;

/** Before launch: what does not exist yet, said once, plainly. */
export function DayOnePending({ what }: { what: string }) {
  return (
    <div className="notice notice--warn">
      <h2 style={{ fontSize: "var(--t1)", marginTop: 0 }}>{what} opens at launch</h2>
      <p>Rook's collection configs (each of the {COLLECTIONS.length} opens the first time someone launches a coin on it) and the genesis ROOK coin are created on Solana by the custodian at launch (Meteora's Dynamic Bonding Curve). Their addresses are only known once that has happened, and this build does not have them yet, so there is no Rook coin to show or trade and nothing here builds a transaction.</p>
      <p style={{ margin: 0 }} className="small muted">Operators: <span className="mono">node ops/solana/site-config.mjs --plan day1-plan.json</span> writes them into <span className="mono">web-rook/src/config/solana.ts</span>.{SOL.problems.length ? ` Config problems: ${SOL.problems.join("; ")}.` : ""}</p>
    </div>
  );
}

/** What happens at graduation: for one collection's coins (its threshold in whole wX), or in general. Used by Launch, Coin and the
 *  collection pages. The copy follows the docs page "How it works". */
export function GraduationExplainer({ slug, children }: { slug?: string | null; children?: ReactNode }) {
  const t = thresholdOf(slug);
  return (
    <div className="panel panel--tight stack">
      <p className="eyebrow" style={{ margin: 0 }}>At graduation</p>
      <ol className="small" style={{ margin: 0, paddingLeft: "1.1rem" }}>
        {t ? <li>The coin trades against {t.symbol} on a bonding curve until the curve holds <strong>{wxText(t.wx)} {t.symbol}</strong>, {t.name}'s threshold (≈ {t.approxSol.toLocaleString("en-US")} SOL at the day-1 floor; the SOL value moves with the floor).</li>
          : <li>The coin trades against its collection's wrapped NFT token (wX) on a bonding curve until the curve holds the collection's threshold: a whole number of wX, <strong>at least 3</strong>, chosen so the raise was worth about 85 SOL at the day-1 floor, more for the priciest collections.</li>}
        <li>It then moves to a Meteora DAMM v2 pool (the custodian handles the move; anyone can trigger it). The pool's liquidity is <strong>permanently locked</strong>: no one, Rook or the creator included, can withdraw it.</li>
        <li><strong>50 % of the raise{t ? ` (${wxText(t.backingWx)} ${t.symbol})` : ""}</strong> is the coin's backing: it is already wX (one whole unit is one NFT in custody) and goes straight into a vault the custodian holds for the coin, at the moment of graduation. Any holder can burn coins for their share.</li>
      </ol>
      {children}
    </div>
  );
}

export function NotOnSolana({ at }: { at: string }) {
  const what: Record<string, string> = { reserves: "The reserves ledger", redeem: "Redemptions", draw: "Raffle draws", rent: "Rentals", bots: "The offer board", wallet: "Wallet profiles" };
  return (
    <div className="pagehead rise"><p className="eyebrow">Not on Solana yet</p><h1>{what[at] ?? "This page"} is not part of the Solana launch</h1>
      <p className="lede">This page belonged to Rook's earlier design on Robinhood Chain, which stopped before launch. On Solana, Rook is coins on Meteora's bonding curve on {COLLECTIONS.length} collections across Ethereum, Solana, Bitcoin and Robinhood Chain, quoted in each collection's wrapped NFT token, with the backing, its vaults and the redemptions kept by Rook's custodian: see <a href={href("docs", "backing")}>Backing and wrapped tokens</a>.</p>
      <p><a href={href("coins")}>The coins</a> · <a href={href("launch")}>Launch a coin</a> · <a href={href("trust")}>What is trusted</a></p></div>
  );
}
