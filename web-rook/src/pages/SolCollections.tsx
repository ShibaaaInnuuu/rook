import { href } from "../lib/env";
import { AVAILABILITY_TEXT, availabilityOf, CHAINS, COLLECTIONS, collectionBySlug, custodyOf, destinationOf, PINNED_COLLECTIONS, type Chain } from "../rook/sol/collections";
import type { CollectionViewV } from "../rook/sol/canister";
import { linksOf, siteLabel, xUrlOf } from "../rook/sol/collectionLinks";
import { THRESHOLDS, thresholdOf, wxText } from "../rook/sol/graduation";
import { SITE as SOL } from "../rook/sol/hooks";
import { useBooksView, useCanisterIndex, useCoinsCollections, useOpening } from "../rook/sol/hooksCanister";
import { configOf, launchableOf, openingOf } from "../rook/sol/opening";
import { OpenCollection } from "../ui/OpenCollection";
import { Pill } from "../ui/bits";
import { CollectionBacking } from "../ui/BackingPanels";
import { IntentDesk } from "../ui/IntentDesk";
import { RafflePanel } from "../ui/RafflePanel";
import { CollectionMosaic } from "../ui/CollectionMosaic";
import { Addr, CollectionLogo, GraduationExplainer } from "../ui/sol";
import { SolCoins } from "./SolCoins";
import type { SolCollection as SolCollectionT } from "../rook/sol/collections";

/** The collection's intent desk (docs/v9/canister-only-solana.md §6.1): mint wX with SOL, burn it for SOL, redeem one for an NFT. The wX
 *  mint it sends to is the books' (`books_view.wx_mint`). */
function CollectionDesk({ c }: { c: SolCollectionT }) {
  // the canister's own index for this collection (its unlock order, not the site's listing order)
  const { coll } = useCanisterIndex(c.wxSymbol);
  const v = useBooksView(coll);
  if (coll === null) return null;
  return <IntentDesk coll={coll} wx={c.wxSymbol} wxMint={v.data?.wx_mint ?? null} kinds={["mint", "burn", "redeem_nft"]} destinationText={destinationOf(c.chain)} />;
}

/** The collection's backing panel under the canister's own index (null until the canister has pinned the collection). */
function CollectionBackingByIndex({ c }: { c: SolCollectionT }) {
  const { coll } = useCanisterIndex(c.wxSymbol);
  return <CollectionBacking coll={coll} wx={c.wxSymbol} slug={c.slug} chain={c.chain} />;
}

/** A collection's standing on the collections page: open (the site config names its DBC config, or the canister says Open), pinned and
 *  waiting for its first coin, asked for, or pinned after the ceremony (from the canister's `coins_collections` once read, the static rule otherwise). */
function StandingPill({ c, cols }: { c: SolCollectionT; cols: CollectionViewV[] | undefined }) {
  const cfg = SOL.collections.find((x) => x.slug === c.slug)?.config ?? null;
  const o = openingOf(cols, c.wxSymbol);
  if (cfg || o.kind === "open") return <Pill tone="good">open</Pill>;
  if (o.kind === "requested" || o.kind === "opening") return <Pill tone="warn">{o.kind === "opening" ? "opening now" : "opening requested"}</Pill>;
  if (o.kind === "pinned") return <Pill tone="warn">{AVAILABILITY_TEXT.pinned}</Pill>;
  if (o.kind === "refused") return <Pill tone="bad">not available</Pill>;
  return <Pill tone={availabilityOf(c) === "pinned" ? "warn" : undefined}>{AVAILABILITY_TEXT[availabilityOf(c)]}</Pill>;
}

/** Rook's collections (COLLECTIONS), grouped by the chain their NFTs live on, each with its graduation threshold in whole wX, its
 *  standing (open, opens on its first coin, after the ceremony), its wX token, and links to its coins and to launch. The thresholds table lists
 *  every collection. */
export function SolCollections() {
  const cols = useCoinsCollections().data;
  const pinned = PINNED_COLLECTIONS.length;
  return (
    <>
      <header className="pagehead colhero rise"><div className="colhero__say"><p className="eyebrow">Collections</p><h1>The {COLLECTIONS.length} collections, on four chains</h1>
        <p className="lede">Launch a coin on any of these. Each coin is priced in its collection's wrapped token, and half of what it raises at graduation goes into a vault of real NFTs.</p>
        <p className="small muted">{pinned} are ready at launch: each opens the first time someone launches on it, so be the first. Robinhood Chain is next. See also <a href={href("tracked")}>every collection Rook tracks</a>.</p></div>
        <CollectionMosaic hrefOf={(slug) => href("collection", slug)} /></header>
      <section className="panel panel--tight rise rise-1 tiers" aria-labelledby="tiers-h">
        <p className="eyebrow" id="tiers-h" style={{ margin: 0 }}>Graduation thresholds</p>
        <p className="small" style={{ margin: ".25rem 0 .5rem" }}>A coin graduates when it raises its collection's threshold, about 85 SOL at the day-1 floor. Half goes into the coin's vault: more than one NFT's worth. ROOK graduates at 85 SOL, with no vault.</p>
        <div className="scroll"><table className="tiers__t small">
          <thead><tr><th>Collection</th><th className="r">Graduates at</th><th className="r">≈ SOL at the day-1 floor</th><th className="r">To the coin's vault</th><th className="r">Market cap at graduation</th></tr></thead>
          <tbody>{THRESHOLDS.map((t) => <tr key={t.slug}>
            <td>{t.page ? <a href={href("collection", t.slug)}>{t.name}</a> : t.name}<span className="muted"> · {t.chain}</span></td>
            <td className="r num"><strong>{wxText(t.wx)} {t.symbol}</strong></td>
            <td className="r num">{t.approxSol.toLocaleString("en-US")} SOL</td>
            <td className="r num">{wxText(t.backingWx)} {t.symbol}</td>
            <td className="r num">{wxText(t.graduationMcapWx)} {t.symbol}</td>
          </tr>)}</tbody>
        </table></div>
      </section>
      {CHAINS.map((chain) => {
        const list = COLLECTIONS.filter((c) => c.chain === chain);
        return (
          <section key={chain} className="block rise rise-2" aria-labelledby={`chain-${chain.replace(/\s/g, "")}`} style={{ marginTop: "1.5rem" }}>
            <h2 id={`chain-${chain.replace(/\s/g, "")}`} style={{ marginBottom: ".25rem" }}>{chain} <span className="muted small">· {list.length}</span></h2>
            <p className="small muted" style={{ margin: "0 0 .5rem" }}>{CHAIN_NOTE[chain]}</p>
            <ul className="cat">{list.map((c) => {
              const t = thresholdOf(c.slug)!;
              const launchable = launchableOf(cols).includes(c);
              return (
                <li key={c.slug}><div className="solcolrow">
                  <a href={href("collection", c.slug)} className="board__coin"><CollectionLogo c={c} size={44} /><span><strong>{c.name}</strong><small>{c.wxSymbol} · graduates at {wxText(t.wx)} {c.wxSymbol}</small></span></a>
                  <span className="small"><StandingPill c={c} cols={cols} /></span>
                  <span className="row small" style={{ gap: ".75rem" }}><a href={href("coins", c.slug)}>Coins</a>{launchable ? <a className="btn btn--small" href={href("launch", c.slug)}>Launch</a> : null}</span>
                </div></li>);
            })}</ul>
          </section>);
      })}
    </>
  );
}

/** One line per chain: how its NFTs are bought and where the custodian holds them. */
const CHAIN_NOTE: Record<Chain, string> = {
  Ethereum: "Bought on OpenSea (Seaport), paid through Relay from SOL; held at the custodian's EVM address on Ethereum.",
  Solana: "Bought on Tensor and Magic Eden (buyer-only, on by default), paid in SOL; held in the custodian's own Solana wallet.",
  Bitcoin: "Bought on Ordinals Wallet or UniSat, paid through Relay from SOL; held at the custodian's taproot address. Bitcoin Frogs is ready; the other Ordinals are added as their floor feeds pass Rook's checks.",
  "Robinhood Chain": "Bought on OpenSea (Seaport 1.6 on Robinhood Chain), paid through Relay from SOL; held at the custodian's EVM address on Robinhood Chain. Added once the custodian has checked Seaport's code there.",
};

export function SolCollection({ slug }: { slug: string }) {
  const c = collectionBySlug(slug)!;
  const row = SOL.collections.find((x) => x.slug === slug) ?? null;
  const t = thresholdOf(slug)!;
  const links = linksOf(c.slug);
  const item = c.chain === "Bitcoin" ? "inscription" : "NFT";
  const { opening } = useOpening(c.wxSymbol);
  const cols = useCoinsCollections().data;
  const config = configOf(row?.config, opening);
  const launchable = launchableOf(cols).includes(c);
  return (
    <>
      <header className="colhead rise">
        <CollectionLogo c={c} size={112} />
        <div className="colhead__id">
          <p className="eyebrow" style={{ margin: 0 }}>Collection · {c.chain}</p>
          <h1 style={{ margin: ".15rem 0 .35rem" }}>{c.name}</h1>
          <p className="muted" style={{ margin: 0 }}>Wrapped token: <strong>{c.wxSymbol}</strong> ({c.wrappedName}: one whole unit is one {item} in custody; its coins are priced in it). <span className="mono break small">{c.contract}</span></p>
          <p className="small" style={{ margin: ".35rem 0 0" }}>Coins graduate at <strong>{wxText(t.wx)} {c.wxSymbol}</strong> (≈ {t.approxSol.toLocaleString("en-US")} SOL at the day-1 floor of {t.floorSol.toLocaleString("en-US")} SOL); half of it, {wxText(t.backingWx)} {c.wxSymbol}, goes into each coin's vault at graduation: more than one {item}'s worth.</p>
          {links ? <p className="collinks small" aria-label={`${c.name}'s official links`}>
            {links.x ? <a href={xUrlOf(links.x)} target="_blank" rel="noreferrer noopener" title={links.xNote ? `@${links.x}: ${links.xNote}` : `@${links.x} on X`}>X @{links.x} ↗</a> : null}
            {links.website ? <a href={links.website} target="_blank" rel="noreferrer noopener">{siteLabel(links.website)} ↗</a> : null}
            {links.marketplaces.map((m) => <a key={m.name} href={m.url} target="_blank" rel="noreferrer noopener">{m.name} ↗</a>)}
          </p> : null}
        </div>
        <div className="colhead__act">{launchable ? <a className="btn btn--primary" href={href("launch", c.slug)}>Launch a coin</a> : <StandingPill c={c} cols={cols} />}</div>
      </header>
      <div className="grid rise rise-1">
        <div className="span-8 stack"><OpenCollection c={c} /><CollectionDesk c={c} /><SolCoins slug={c.slug} embedded /></div>
        <aside className="span-4 stack">
          <CollectionBackingByIndex c={c} />
          <section className="panel panel--tight small">
            <p className="eyebrow">On Solana</p>
            <dl className="kv">
              <dt>DBC config</dt><dd>{config ? <Addr id={config} /> : <StandingPill c={c} cols={cols} />}</dd>
              <dt>Fee claimer</dt><dd>{row?.claimer ? <Addr id={row.claimer} /> : "—"}</dd>
              <dt>Custody</dt><dd>{custodyOf(c.chain)}</dd>
              <dt>Redeem to</dt><dd>{destinationOf(c.chain)}</dd>
            </dl>
            {c.chain === "Robinhood Chain" ? <p className="muted" style={{ margin: ".5rem 0 0" }}>The {c.name} NFTs behind {c.wxSymbol} are held at the canister's EVM address on Robinhood Chain (the same threshold key as on Ethereum), bought through Seaport 1.6 there; the Reserves check reads each one's owner on Robinhood Chain. Contract <span className="mono break">{c.contract}</span>.</p> : null}
            {c.chain === "Solana" ? <p className="muted" style={{ margin: ".5rem 0 0" }}>The {c.name} NFTs behind {c.wxSymbol} are held by the canister's own Solana wallet (its main key, a threshold key no person holds): each sits in that wallet's token account for its mint, which the Reserves page checks. Collection <span className="mono break">{c.contract}</span>.</p> : null}
            <p className="muted" style={{ margin: ".5rem 0 0" }}>The fee claimer is a key only the custodian signs with: Rook's share of this collection's curve fees and every coin's backing share are paid to it in {c.wxSymbol}, and the custodian moves each backing share into the coin's own vault.</p>
          </section>
          <RafflePanel slug={c.slug} />
          <GraduationExplainer slug={c.slug} />
        </aside>
      </div>
    </>
  );
}
