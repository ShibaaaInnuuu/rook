import { href } from "../lib/env";
import { Section } from "../components/ui";

/**
 * docs/v5/ux-and-reserves.md §1, "Rent your item". A description of what is being built, and nothing that can be pressed: the rental
 * contracts are in progress, so there is no form, no deposit and no yield on this page.
 */
export function RentPage({ slug }: { slug?: string }) {
  return (
    <>
      <header className="pagehead">
        <h1>Rent your item</h1>
        <p>Coming later. Nothing on this page does anything yet.</p>
      </header>
      <div className="notice notice--plain" role="status"><p><strong>Not built yet.</strong> The rental contracts are in progress. There is nothing to sign here and nothing to deposit; any page that asks you to deposit an item for rent before this one changes is not ours.</p></div>
      <Section title="What it will be">
        <div className="prose">
          <p>If you own an item of a collection, you will be able to lend it to the collection's custody for a period instead of selling it. While it is lent it backs one token like any other item, and it appears on the <a href={href("reserves", slug ?? "")}>proof of reserves</a> marked as a rental, with the transaction that brought it in.</p>
          <p>In return you would earn a share of what the collection's coins pay, for as long as it is lent. At the end of the period the same item comes back to you — unless it was drawn by a redemption, in which case the terms (still being written) decide what you receive instead.</p>
          <p className="quiet">The exact terms, the risks and the contracts will be published before anything goes live. Until then this is a description, not an offer.</p>
        </div>
      </Section>
      {slug ? <p><a href={href("collection", slug)}>Back to the collection</a></p> : null}
    </>
  );
}
