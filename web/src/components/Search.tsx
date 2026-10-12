import { useId, useRef, useState } from "react";
import { MagnifyingGlass } from "@phosphor-icons/react";
import { href } from "../lib/env";
import { search, useAcquisitionRows, useCoinRows, useListingRows, type Hit } from "../lib/catalog";
import type { Offered } from "../lib/coinlist";
import { useCatalogue } from "../lib/catalogue";

/**
 * A coin's name, a symbol, an address, a Bitcoin transaction id, a rune id, or a collection nobody has listed yet.
 *
 * There is no search endpoint, so this matches over the lists the page already holds and over the committed catalogue,
 * which exists before any chain does. That is why the box still answers on a site with nothing deployed.
 */
export function Search({ wide }: { wide?: boolean } = {}) {
  const id = useId();
  const [q, setQ] = useState("");
  const [open, setOpen] = useState(false);
  const [cursor, setCursor] = useState(-1);
  const box = useRef<HTMLDivElement>(null);
  const coins = useCoinRows();
  const listings = useListingRows();
  const receipts = useAcquisitionRows();
  const catalogue = useCatalogue();

  const offered = catalogue.data
    ? [
        ...catalogue.data.collections.map((c) => ({ id: c.id, slug: c.slug, name: c.name, kind: c.kind })),
        ...catalogue.data.runes.map((r) => ({ id: r.id, slug: r.slug, name: r.name, kind: r.kind, runeId: r.runeId })),
      ]
    : [];
  // What the catalogue holds, and how much of it has no vault yet — which on staging, and by intent on mainnet, is none.
  // Saying "n have no vault yet" when every one of them has one is the distinction that stopped existing: docs/phase9.md §11.3.
  const refs = new Set((listings.data ?? []).map((l) => l.ref.toLowerCase()));
  const hasVault = (o: Offered) => refs.has(o.slug.toLowerCase()) || refs.has((o.runeId ?? "\u0000").toLowerCase());
  const withoutVault = offered.filter((o) => !hasVault(o)).length;

  const ready = !!coins.data || !!listings.data || !!catalogue.data;
  const hits: Hit[] = ready
    ? search(q, coins.data?.rows ?? [], listings.data ?? [], receipts.data?.rows ?? [], offered)
    : [];
  const show = open && q.trim().length >= 2;

  const keys = (e: React.KeyboardEvent<HTMLInputElement>) => {
    if (e.key === "Escape") {
      setOpen(false);
      setCursor(-1);
      (e.target as HTMLInputElement).blur();
      return;
    }
    if (!show || !hits.length) return;
    if (e.key === "ArrowDown") { e.preventDefault(); setCursor((c) => (c + 1) % hits.length); }
    if (e.key === "ArrowUp") { e.preventDefault(); setCursor((c) => (c <= 0 ? hits.length - 1 : c - 1)); }
    if (e.key === "Enter" && cursor >= 0) {
      e.preventDefault();
      location.hash = `#/${hits[cursor].to}`;
      setOpen(false);
      setQ("");
      setCursor(-1);
    }
  };

  return (
    <div
      className={`search${wide ? " search--wide" : ""}`}
      ref={box}
      onBlur={(e) => { if (!box.current?.contains(e.relatedTarget as Node)) setOpen(false); }}
    >
      <label className="search__field" htmlFor={id}>
        <MagnifyingGlass size={wide ? 20 : 16} aria-hidden />
        <span className="vh">Search coins, listings, receipts and collections</span>
        <input
          id={id}
          type="search"
          value={q}
          placeholder={wide ? "Search a coin, a collection, a rune, an address" : "Search"}
          autoComplete="off"
          spellCheck={false}
          role="combobox"
          aria-expanded={show}
          aria-controls={`${id}-out`}
          onChange={(e) => { setQ(e.target.value); setOpen(true); setCursor(-1); }}
          onFocus={() => setOpen(true)}
          onKeyDown={keys}
        />
      </label>
      {show ? (
        <div className="search__out" id={`${id}-out`} role="listbox" aria-label="Results">
          {!ready ? (
            <p className="search__none">Nothing is indexed to search yet.</p>
          ) : !hits.length ? (
            <p className="search__none">Nothing here matches that. The catalogue holds {offered.length} collections and runes{withoutVault ? `, ${withoutVault} of them with no vault yet` : ", each with a vault"}.</p>
          ) : (
            hits.map((h, i) => (
              <a
                key={`${h.kind}-${h.to}`}
                className={`search__hit${i === cursor ? " search__hit--on" : ""}`}
                href={href(...h.to.split("/"))}
                role="option"
                aria-selected={i === cursor}
                onMouseEnter={() => setCursor(i)}
                onClick={() => { setOpen(false); setQ(""); setCursor(-1); }}
              >
                <span className="search__label">{h.label}</span>
                <span className="search__sub quiet">
                  {h.sub}
                  {h.state ? <span className={`tag tag--${h.state === "vault" ? "on" : "off"}`}>{h.state}</span> : null}
                </span>
              </a>
            ))
          )}
        </div>
      ) : null}
    </div>
  );
}
