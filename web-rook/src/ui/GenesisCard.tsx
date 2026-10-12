import { ROOK_GRADUATION_SOL } from "../rook/sol/graduation";
import { compact, priceShort, progressOf, raisedSol, type MarketCoin } from "../rook/sol/market";

// ui/bits' Pill, inline: bits pulls in the EVM env (lib/chain), which this card and its test do not need
const Pill = ({ tone, children }: { tone: "live" | "warn"; children: string }) => <span className={`pill pill--${tone}`}>{children}</span>;

/** ROOK's picture, served by the site itself (the same file as its metadata's image on Cloudinary, `rook-token/rook.png`). */
export const ROOK_IMAGE = "/rook-token/rook.png";

export type GenesisCardProps = {
  /** ROOK's mint once the site config names it (and the config is ready); null before launch: no link, nothing read. */
  mint: string | null;
  /** ROOK's market row (the board's, or read from its pool); null while it is not known. */
  coin: MarketCoin | null;
  /** The coin page's link for `mint`, and the docs' "What is Rook" link (the pages pass `href(…)`). */
  tradeHref: string | null;
  docsHref: string;
  loading?: boolean;
  demo?: boolean;
};

/**
 * The home page's genesis coin card: ROOK's picture, name and ticker, what it is (docs/01-what-is-rook.md, 02-how-it-works.md and
 * 05-fees.md: SOL-paired, graduates at 85 SOL, no NFT backing, its fees pay Rook's running costs), and once launched its price, market
 * cap and curve progress with a "Trade $ROOK" link to its coin page. Before launch (no mint) it says "Launches soon" and links nowhere
 * but the docs. Pure: test/genesis.test.ts renders it.
 */
export function GenesisCard({ mint, coin, tradeHref, docsHref, loading = false, demo = false }: GenesisCardProps) {
  const live = !!mint && !!tradeHref;
  const grad = coin?.phase === "graduated";
  const p = coin ? progressOf(coin) : 0;
  const raised = coin ? raisedSol(coin) : null;
  const mcap = coin ? (coin.marketCapUsd !== null ? compact(coin.marketCapUsd, "$") : compact(coin.marketCapSol, "SOL")) : "—";
  return (
    <section className="genesis" aria-labelledby="genesis-h">
      <img className="genesis__pic" src={ROOK_IMAGE} alt="" width={88} height={88} loading="eager" decoding="async" />
      <div className="genesis__say">
        <p className="genesis__k">Genesis coin {live ? <Pill tone="live">{demo ? "Demo" : "Live"}</Pill> : <Pill tone="warn">Launches soon</Pill>}</p>
        <h2 className="genesis__h" id="genesis-h">Rook <span className="genesis__sym">$ROOK</span></h2>
        <p className="genesis__lede">The genesis coin. It trades against SOL on Meteora's bonding curve and graduates at {ROOK_GRADUATION_SOL} SOL. It has no NFT backing: its trading fees pay for Rook's running costs.</p>
      </div>
      {live ? (
        <dl className="genesis__figs" aria-label="ROOK now">
          <div><dt>Price</dt><dd className="num">{coin ? priceShort(coin.price) : "—"}<small> SOL</small></dd></div>
          <div><dt>Market cap</dt><dd className="num">{mcap}</dd></div>
          <div className="genesis__prog"><dt>{grad ? "Graduated" : "Bonding curve"}</dt>
            <dd>{coin ? (grad ? <span className="small muted">In its locked Meteora pool</span>
              : <><span className="tcard__bar" title={`${Math.round(p * 100)}% of ${ROOK_GRADUATION_SOL} SOL`}><i style={{ width: `${Math.max(2, Math.round(p * 100))}%` }} /></span>
                <span className="num small">{Math.round(p * 100)}% · {raised === null ? "—" : raised.toFixed(raised >= 10 ? 1 : 2)} / {ROOK_GRADUATION_SOL} SOL</span></>)
              : <span className="small muted">{loading ? "Reading the curve…" : "—"}</span>}</dd></div>
        </dl>
      ) : (
        <p className="genesis__soon small muted">ROOK launches with Rook on Solana. Its price, its curve and the link to trade it appear here then.</p>
      )}
      <p className="genesis__acts">
        {live ? <a className="btn btn--primary" href={tradeHref!}>Trade $ROOK</a> : <span className="btn genesis__off" aria-disabled="true">Trading opens at launch</span>}
        <a className="small" href={docsHref}>What is ROOK →</a>
      </p>
    </section>
  );
}
