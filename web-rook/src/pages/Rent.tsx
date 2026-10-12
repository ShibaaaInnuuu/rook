import { href } from "../lib/env";
import { useWrappedCollection } from "../lib/wrapped";
import { Gate } from "../ui/Gate";

/** docs/v5/floor-module.md §9: renting is designed and not built. A description, and nothing that can be pressed. */
export function RentPage({ slug }: { slug?: string }) {
  return <Gate page={`rent/${slug ?? ""}`}>{() => <Rent slug={slug} />}</Gate>;
}
function Rent({ slug }: { slug?: string }) {
  const c = useWrappedCollection(slug);
  const name = c.data?.name ?? "the collection";
  return (
    <>
      <header className="pagehead"><p className="eyebrow">Rent · {name}</p><h1>Lend one NFT to the vault for 30 days</h1>
        <p className="lede"><strong>Not built yet.</strong> The contracts for this are designed and in progress. Nothing on this page can be pressed, and nothing here holds a deposit or pays a yield.</p></header>
      <div className="grid">
        <div className="span-7 stack">
          <p>When it lands, a holder of a floor NFT in {name} lends it to the collection's floor module for thirty days and gets <strong>the same NFT back</strong> at the end. While it is lent it backs one wrapped token that would otherwise be backed by ETH, so the vault has one fewer NFT to buy.</p>
          <p>The renter earns half of the module's realised spread revenue for the term, shared equally among active rentals. If a random redemption draws the lent NFT and it is delivered, the renter receives the floor plus ten percent in ETH instead of the NFT.</p>
          <p>Only floor NFTs are eligible: the keeper posts an eligibility list and rare ones are not on it. The intake goes through the same deposit path as any wrap, so the same checks apply.</p>
          <p className="small muted">The design: <a href={href("docs", "floor-module")}>the floor module, §9</a>. <a href={href("collection", slug ?? "")}>Back to the collection</a>.</p>
        </div>
      </div>
    </>
  );
}
