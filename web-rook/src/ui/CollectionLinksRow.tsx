import { coinLinksOf } from "../rook/sol/collectionLinks";

/**
 * A collection coin's page: its collection's official X and website (collectionLinks.ts `coinLinksOf`), on their own row labeled
 * "Collection", apart from the coin's own links (its metadata's, set by its creator). Nothing for a slug that is not a Rook collection.
 */
export function CollectionLinksRow({ slug, name }: { slug: string; name: string }) {
  const c = coinLinksOf(slug);
  if (!c || !(c.website || c.twitter)) return null;
  return (
    <p className="row small coinhead2__links coinhead2__collinks" aria-label={`${name}'s official links`}>
      <span className="muted">Collection ({name})</span>
      {c.website ? <a href={c.website} target="_blank" rel="noreferrer noopener">{c.websiteFrom ? `${c.websiteFrom} ↗` : "Website ↗"}</a> : null}
      {c.twitter ? <a href={c.twitter} target="_blank" rel="noreferrer noopener">X ↗</a> : null}
    </p>
  );
}
