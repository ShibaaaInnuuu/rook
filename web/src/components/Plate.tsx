import { useEffect, useMemo, useRef, useState } from "react";
import type { CatalogueEntry } from "../lib/catalogue";
import { entriesByRef, isRune, useCatalogue } from "../lib/catalogue";
import { chiselOf, initials } from "../lib/plate";

export { initials, seedOf } from "../lib/plate";

/**
 * The plate: a square recess cut into the stone, with artwork set inside it.
 *
 * Everything pictured on this site is a plate, so a coin with no logo, a rune that has no artwork by design, and a real
 * inscription all sit in the same hole in the same surface. The recess is drawn before anything loads, so nothing shifts,
 * and a picture that never arrives leaves a carved monogram rather than a broken image.
 *
 * docs/phase9.md, section 1.
 */

/**
 * The fallback that is not a placeholder. Two engraved characters over chisel marks whose angle and spacing come from the
 * identifier itself, so the same coin is always cut the same way and two coins are never cut alike.
 */
export function Monogram({ id, text, title }: { id: string; text: string; title?: string }) {
  const { angle, gap, shift } = chiselOf(id);
  return (
    <span
      className="plate__mono"
      style={{ "--chisel": `${angle}deg`, "--gap": `${gap}px`, "--shift": `${shift}px` } as React.CSSProperties}
      aria-hidden="true"
      title={title}
    >
      <span className="plate__cut">{initials(text)}</span>
    </span>
  );
}

/** A rune has a symbol and a spaced name and nothing else. That is the artwork: the protocol's own two marks, engraved. */
export function RuneFace({ symbol, name }: { symbol: string | null; name: string }) {
  return (
    <span className="plate__rune" aria-hidden="true">
      <span className="plate__glyph plate__cut">{symbol ?? "ᚱ"}</span>
      <span className="plate__runename">{name}</span>
    </span>
  );
}

type ArtProps = {
  /** where the picture is, when there is one */
  src: string | null;
  /** what it is, for somebody who cannot see it */
  alt: string;
  /** what the monogram is cut from when there is no picture */
  fallbackId: string;
  fallbackText: string;
  /** a short caption engraved along the bottom of the plate */
  tag?: string;
  className?: string;
};

/**
 * A picture in a plate, with every way it can fail already drawn:
 * missing (monogram), slow (the recess, breathing), broken (monogram), or not a picture at all.
 */
export function Art({ src, alt, fallbackId, fallbackText, tag, className }: ArtProps) {
  const [state, setState] = useState<"idle" | "ok" | "failed">(src ? "idle" : "failed");
  const [pixel, setPixel] = useState(false);
  const img = useRef<HTMLImageElement>(null);

  useEffect(() => {
    setState(src ? "idle" : "failed");
    setPixel(false);
  }, [src]);

  // A cached image can finish before React attaches its handler, which would leave the plate breathing forever.
  useEffect(() => {
    const el = img.current;
    if (el?.complete && el.naturalWidth > 0) {
      setPixel(el.naturalWidth <= 128);
      setState("ok");
    }
  }, [src]);

  return (
    <span className={`plate${className ? ` ${className}` : ""}${state === "idle" ? " plate--waiting" : ""}`}>
      {src && state !== "failed" ? (
        <img
          ref={img}
          className={`plate__img${pixel ? " plate__img--pixel" : ""}`}
          src={src}
          alt={alt}
          loading="lazy"
          decoding="async"
          draggable={false}
          onLoad={(e) => {
            setPixel(e.currentTarget.naturalWidth <= 128);
            setState("ok");
          }}
          onError={() => setState("failed")}
        />
      ) : (
        <Monogram id={fallbackId} text={fallbackText} title={alt} />
      )}
      {tag ? <span className="plate__tag">{tag}</span> : null}
    </span>
  );
}

const GATEWAY = "https://ordinals.com/content/";

/**
 * A real inscription, rendered from the reference ord content gateway.
 *
 * Most inscriptions are images and load as one. Some are text or HTML, which an `<img>` cannot show: when the image fails,
 * the content is read once and, if it really is text, set in our own mono face. Anything else falls back to the monogram.
 * Nothing here is ever a different picture standing in for the real one.
 */
export function Inscription({ id, alt, tag }: { id: string; alt: string; tag?: string }) {
  const [text, setText] = useState<string | null>(null);
  const [failed, setFailed] = useState(false);

  const readAsText = async () => {
    try {
      const res = await fetch(GATEWAY + id, { signal: AbortSignal.timeout(8000) });
      const type = res.headers.get("content-type") ?? "";
      if (!res.ok || !/^text\/|json|javascript/.test(type)) {
        setFailed(true);
        return;
      }
      const body = (await res.text()).replace(/<[^>]+>/g, " ").replace(/\s+/g, " ").trim();
      if (body) setText(body.slice(0, 180));
      else setFailed(true);
    } catch {
      setFailed(true);
    }
  };

  if (text !== null) {
    return (
      <span className="plate plate--text">
        <span className="plate__text mono">{text}</span>
        {tag ? <span className="plate__tag">{tag}</span> : null}
      </span>
    );
  }
  if (failed) {
    // Two carved initials mean nothing for an inscription. Its own id does, so that is what the plate carries.
    return (
      <span className="plate plate--id">
        <span className="plate__id mono" aria-hidden="true">{id.slice(0, 6)}<br />…{id.slice(-5)}</span>
        <span className="vh">{alt}, which this gateway could not show</span>
        {tag ? <span className="plate__tag">{tag}</span> : null}
      </span>
    );
  }
  return (
    <span className="plate plate--waiting">
      <img
        className="plate__img"
        src={GATEWAY + id}
        alt={alt}
        loading="lazy"
        decoding="async"
        draggable={false}
        onLoad={(e) => {
          e.currentTarget.classList.toggle("plate__img--pixel", e.currentTarget.naturalWidth <= 128);
          e.currentTarget.parentElement?.classList.remove("plate--waiting");
        }}
        onError={() => void readAsText()}
      />
      {tag ? <span className="plate__tag">{tag}</span> : null}
    </span>
  );
}

/** A catalogue entry's face: a rune's two marks, or the collection's real artwork. */
export function EntryArt({ entry, tag }: { entry: CatalogueEntry; tag?: string }) {
  if (isRune(entry)) {
    return (
      <span className="plate plate--rune">
        <RuneFace symbol={entry.symbol} name={entry.name} />
        <span className="vh">{entry.name}, rune {entry.runeId}</span>
        {tag ? <span className="plate__tag">{tag}</span> : null}
      </span>
    );
  }
  return (
    <Art
      src={entry.image}
      alt={entry.items[0]?.name ? `${entry.items[0].name}, from ${entry.name}` : `Artwork from ${entry.name}`}
      fallbackId={entry.id}
      fallbackText={entry.name}
      tag={tag}
    />
  );
}

/**
 * A coin's face. Pons takes a logo at launch and stores no getter for it, so the indexer cannot hand one over yet
 * (docs/phase9.md, section 5). Until it does, every coin wears the monogram cut from its own address, and this
 * component already prefers a logo the moment one arrives.
 */
export function CoinArt({ address, name, symbol, logo, tag }: {
  address: string; name: string; symbol: string; logo?: string | null; tag?: string;
}) {
  return (
    <Art
      src={logo ?? null}
      alt={`${name} (${symbol})`}
      fallbackId={address}
      fallbackText={symbol && symbol !== "?" ? symbol : name}
      tag={tag}
      // A mark is not a picture and should not pretend to be one: it takes a shallower plate than artwork does.
      className={logo ? undefined : "plate--mark"}
    />
  );
}

/** What a vault actually bought, drawn as the thing itself where a public gateway can show it. */
/**
 * A listing's face wherever a listing is drawn: the collection's real artwork, or a rune's own mark, from the committed
 * catalogue — and the engraved initials it always had when the catalogue does not know this listing.
 *
 * It exists because the vaults page and the receipts page each drew a monogram for every listing while the artwork sat in a
 * file the site already ships. A page whose subject is "what this vault bought" should show the thing.
 */
export function ListingArt({ listing, tag = "vault" }: { listing: { id: number; kind: "inscriptions" | "rune"; name: string; ref: string }; tag?: string }) {
  const catalogue = useCatalogue();
  const entry = useMemo(() => entriesByRef(catalogue.data).get(listing.ref.toLowerCase()), [catalogue.data, listing.ref]);
  if (entry) return <EntryArt entry={entry} tag={tag} />;
  return listing.kind === "rune"
    ? <span className="plate plate--rune"><RuneFace symbol={null} name={listing.name} />{tag ? <span className="plate__tag">{tag}</span> : null}</span>
    : <span className="plate"><Monogram id={`listing-${listing.id}-${listing.ref}`} text={listing.name} />{tag ? <span className="plate__tag">{tag}</span> : null}</span>;
}

export function AssetArt({ kind, assetRef, tag }: { kind: "inscriptions" | "rune"; assetRef: string; tag?: string }) {
  const catalogue = useCatalogue();
  // a rune receipt's assetRef IS the rune id, which is a catalogue key, so its real mark is one lookup away. Without this a
  // rune receipt wore a blank glyph while the symbol sat in a file the site already ships.
  const entry = useMemo(() => entriesByRef(catalogue.data).get(assetRef.toLowerCase()), [catalogue.data, assetRef]);
  if (kind === "rune") {
    const symbol = entry && isRune(entry) ? entry.symbol : null;
    const name = entry?.name ?? assetRef;
    return <span className="plate plate--rune"><RuneFace symbol={symbol} name={name} /><span className="vh">{name}, rune {assetRef}</span>{tag ? <span className="plate__tag">{tag}</span> : null}</span>;
  }
  return <Inscription id={assetRef} alt={`Inscription ${assetRef.slice(0, 10)}`} tag={tag} />;
}

/** An inscription id is 64 hex characters and a suffix. Nobody reads that; they check the ends. */
export const shortRef = (ref: string): string => (ref.length > 24 ? `${ref.slice(0, 10)}…${ref.slice(-7)}` : ref);
