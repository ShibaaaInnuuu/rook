import { useMemo } from "react";
import { href, useEnv, useNow } from "../lib/env";
import { useAcquisitions, useListingDetail, useListings, usePurchases, useShift, useVault } from "../lib/data";
import { VAULT } from "../lib/abis";
import { BITCOIN_TX, INSCRIPTION } from "../lib/chain";
import { Addr, Ext, Failed, Loading, NoDeployment, Section } from "../components/ui";
import { TxButton } from "../components/wallet";
import { OnlyOneWayOut, PaidInTable, SweepHistory, VaultHead, VaultPanel } from "../components/vaultpage";
import { explorerFor } from "../lib/explorer";
import { isRune, useCatalogue, type CatalogueEntry } from "../lib/catalogue";
import { useSource } from "../lib/catalog";
import { factsFromState, factsOf, paidIn, sweeps } from "../lib/vault";
import { PURCHASE_STATUS, countdown, eth, units, when } from "../lib/format";

/**
 * One vault: what it holds, what it may pay per thing, what it has bought, which coins feed it, and where each thing it
 * bought has gone. docs/phase14.md §8.
 *
 * Two sources reach this page and they do not carry the same fields. With an indexer it reads `/v1/listings/:id`, which
 * carries the totals summed over the vault's whole history; reading the chain directly it has the balances, the cap and
 * the declarations and nothing summed at all. The difference is carried in one value — `VaultFacts`, where a figure this
 * source cannot give is null — so there is one panel and not two, and a row that is missing says so instead of printing a
 * reassuring zero.
 */

/** The catalogue entry a vault is listed under: the registry's `ref` is a collection's slug or a rune's id, and nothing else joins the two. */
function entryFor(ref: string, entries: CatalogueEntry[] | undefined): CatalogueEntry | undefined {
  const want = ref.toLowerCase();
  return entries?.find((e) => (isRune(e) ? e.runeId : e.slug).toLowerCase() === want);
}

export function ListingPage({ id }: { id?: string }) {
  const env = useEnv();
  const now = useNow();
  const shift = useShift();
  const src = useSource();
  const listings = useListings();
  const listing = listings.data?.[Number(id)];
  const vault = useVault(listing?.vault, listing?.id);
  const detail = useListingDetail(listing?.id);
  const purchases = usePurchases(listing?.vault, vault.data?.purchases ?? 0, listing?.id);
  const acquisitions = useAcquisitions(listing?.vault, vault.data?.acquisitions ?? 0, listing?.id);
  const catalogue = useCatalogue();
  const entries = useMemo(() => (catalogue.data ? [...catalogue.data.collections, ...catalogue.data.runes] : undefined), [catalogue.data]);

  if (env === null) return <Loading what="the deployment" />;
  if (env.mode === "none") return <><header className="pagehead"><h1>Vault</h1></header><NoDeployment /></>;
  if (listings.isLoading) return <Loading what="the vault" />;
  if (listings.error) return <Failed what="the vault" error={listings.error} />;
  if (!listing) return <p className="error">There is no vault {id}.</p>;

  const v = vault.data;
  const d = detail.data;
  const facts = d ? factsOf(d, now, shift) : v ? factsFromState(v, listing.kind, now) : null;
  const explorer = explorerFor("chain" in env ? env.chain.id : null, src?.kind === "fixtures");
  const chainName = "chain" in env ? env.chain.name : null;
  const demo = env.mode === "demo" || env.mode === "sample";

  return (
    <>
      <VaultHead listing={listing} entry={entryFor(listing.ref, entries)} explorer={explorer} chain={chainName} />

      {vault.isLoading ? <Loading what="the vault" /> : vault.error ? <Failed what="the vault" error={vault.error} /> : facts ? (
        <>
          {facts.blockedBy !== null ? <p className="banner banner--block" role="alert">Purchase {facts.blockedBy} is past its receipt deadline. The vault declares nothing more until it is settled or written off.</p> : null}

          <VaultPanel f={facts} listing={listing} nowS={now} />

          <OnlyOneWayOut listing={listing} btcVault={listing.btcVault} />

          {d ? <PaidInTable rows={paidIn(d)} listing={listing} total={BigInt(d.depositedWei)} /> : null}

          <Section title="Purchases" aside="declared in public, released an hour later">
            {purchases.isLoading ? <Loading what="purchases" /> : !purchases.data?.length ? <p className="empty">Nothing has been declared yet. When the keeper declares a purchase it appears here with its public hour counting down.</p> : (
              <div className="book__scroll"><table className="book__table">
                <thead><tr><th scope="col">#</th><th scope="col">State</th><th scope="col" className="book__r">Up to</th><th scope="col" className="book__r">Quantity</th><th scope="col">Timing</th><th scope="col">Cap reference</th><th /></tr></thead>
                <tbody>{purchases.data.map((p) => (
                  <tr key={p.id}>
                    <td className="num">{p.id}</td><td><span className={`tag${p.status === 5 ? " tag--warn" : p.status === 4 ? " tag--good" : ""}`}>{PURCHASE_STATUS[p.status]}</span></td>
                    <td className="book__r num">{eth(p.maxEth)} ETH</td><td className="book__r num">{listing.kind === "rune" ? units(p.quantity, listing.divisibility) : String(p.quantity)}</td>
                    <td>{p.status === 1 ? (p.releasableAt > now ? <>releasable in <span className="num">{countdown(p.releasableAt, now)}</span></> : "releasable now")
                      : p.status === 3 ? (p.deadline > now ? <>receipt due in <span className="num">{countdown(p.deadline, now)}</span></> : <strong>receipt overdue since {when(p.deadline)}</strong>) : <span className="quiet">done</span>}</td>
                    <td className="mono small">{p.capRef || "none given"}</td>
                    <td>{p.status === 1 && p.releasableAt <= now ? <TxButton label="Release" request={() => ({ address: listing.vault, abi: VAULT, functionName: "release", args: [BigInt(p.id)] })} /> : null}</td>
                  </tr>))}</tbody>
              </table></div>)}
            <p className="quiet small">Releasing sends the declared ETH to the purchaser named when it was declared. Anyone may do it once the hour has passed; it fails if the bond would not cover it.</p>
          </Section>

          {d ? (
            <SweepHistory rows={sweeps(d.acquisitionsRecent, shift)} listing={listing} total={d.acquisitions} demo={demo} />
          ) : (
            <Section title="What the vault has bought" aside={<a href={href("draws")}>Draws and distributions</a>}>
              {acquisitions.isLoading ? <Loading what="receipts" /> : !acquisitions.data?.length ? <p className="empty">No purchase has been receipted yet. Each one will be listed here with its Bitcoin transaction, so you can check it yourself.</p> : (
                <div className="book__scroll"><table className="book__table">
                  <thead><tr><th scope="col">#</th><th scope="col">{listing.kind === "rune" ? "Amount" : "Inscription"}</th><th scope="col">Paid by</th><th scope="col" className="book__r">Sats paid</th><th scope="col">Bitcoin transaction</th></tr></thead>
                  <tbody>{acquisitions.data.map((a) => (
                    <tr key={a.id}><td className="num">{a.id}</td>
                      <td>{listing.kind === "rune" ? <span className="num">{units(a.amount, listing.divisibility)}</span> : <Ext href={INSCRIPTION(a.assetRef)}><span className="mono">{a.assetRef.slice(0, 10)}…{a.assetRef.slice(-4)}</span></Ext>}</td>
                      <td><a href={href("coin", a.coin)}><Addr a={a.coin} /></a></td><td className="book__r num">{a.satsPaid.toLocaleString("en-US")}</td>
                      <td><Ext href={BITCOIN_TX(a.btcTxid)}><span className="mono">{a.btcTxid.slice(2, 10)}…{a.btcTxid.slice(-6)}</span></Ext></td></tr>))}</tbody>
                </table></div>)}
              <p className="quiet small">
                Read from the chain, which carries no delivery state and no figure for what a receipt's purchase cost: with
                an indexer this table says both. A receipt is the attestor's word — check it against Bitcoin yourself.
              </p>
            </Section>
          )}
        </>
      ) : null}
    </>
  );
}
