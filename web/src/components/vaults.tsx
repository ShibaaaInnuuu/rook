import { useId, useMemo, useRef, useState } from "react";
import { eth } from "../lib/format";
import { initials } from "../lib/plate";
import { matchVaults, vaultPicks } from "../lib/catalog";
import { isRune, useCatalogue, type Catalogue } from "../lib/catalogue";
import type * as Api from "../api/types";

/**
 * docs/phase14.md §1. The vault filter, as a control that does not grow with the number of vaults.
 *
 * What it replaces printed one button per vault: measured against the committed demo's 60 vaults, 61 chips, 374px tall at
 * 1280px and 1,298px tall at 400px, above the coins, on the home page and on Explore alike. The height was linear in the
 * number of vaults and the registry's owner adds vaults for a living.
 *
 * This shows `All vaults`, at most eight vaults, and one `More…` button — eleven chips at sixty vaults and eleven at six
 * hundred. The eight are the ones coins actually pay into, most-paired first (`vaultPicks`), because a filter offering
 * vaults nothing is paired with offers empty lists. Everything else is behind a panel that filters as you type, and the
 * chosen vault is always on the row even when it is not one of the eight.
 */

type Face = { image: string | null; glyph: string | null; name: string };

/**
 * A face per vault, from the committed catalogue rather than a network call: `web/public/catalogue/` is a file the site
 * ships (docs/phase9.md), so a chip's picture cannot fail on somebody else's outage. A listing's `ref` is a collection slug
 * or a rune id, which is how it is matched. Measured against the demo's 60 vaults: 39 carry an image, 20 are runes and take
 * the protocol's own glyph, 1 matches nothing and takes its engraved initials. No vault is faceless.
 */
function facesOf(catalogue: Catalogue | undefined): Map<string, Face> {
  const out = new Map<string, Face>();
  if (!catalogue) return out;
  for (const e of [...catalogue.collections, ...catalogue.runes]) {
    const face: Face = { image: isRune(e) ? null : e.image, glyph: isRune(e) ? e.symbol ?? "ᚱ" : null, name: e.name };
    out.set(e.slug.toLowerCase(), face);
    if (isRune(e)) out.set(e.runeId.toLowerCase(), face);
  }
  return out;
}

/** 18 pixels of vault: its collection's artwork, a rune's mark, or two engraved letters. Never an empty square. */
function VaultFace({ vault, face }: { vault: Api.ListingSummary; face: Face | undefined }) {
  const [broken, setBroken] = useState(false);
  if (face?.image && !broken)
    return <img className="vface vface--img" src={face.image} alt="" aria-hidden="true" loading="lazy" decoding="async" onError={() => setBroken(true)} />;
  const glyph = face?.glyph ?? (vault.kind === "rune" ? "ᚱ" : null);
  return <span className="vface vface--cut" aria-hidden="true">{glyph ?? initials(vault.name)}</span>;
}

export function VaultControl({ vaults, value, onPick, limit = 8 }: {
  vaults: Api.ListingSummary[]; value: number | null; onPick: (v: number | null) => void; limit?: number;
}) {
  const id = useId();
  const [open, setOpen] = useState(false);
  const [q, setQ] = useState("");
  const box = useRef<HTMLDivElement>(null);
  const catalogue = useCatalogue();
  const faces = useMemo(() => facesOf(catalogue.data), [catalogue.data]);
  const { shown, rest } = useMemo(() => vaultPicks(vaults, value, limit), [vaults, value, limit]);

  // one vault is not a filter: there is nothing to filter to
  if (vaults.length < 2) return null;

  const found = matchVaults(q, vaults);
  const close = () => { setOpen(false); setQ(""); };

  return (
    <div className="vaults" role="group" aria-label="Vault" ref={box}
      onBlur={(e) => { if (!box.current?.contains(e.relatedTarget as Node)) close(); }}>
      <span className="chips">
        <button type="button" className={`chip${value === null ? " chip--on" : ""}`} aria-pressed={value === null}
          onClick={() => { onPick(null); close(); }}>All vaults</button>
        {shown.map((v) => (
          <button key={v.id} type="button" className={`chip chip--face${v.id === value ? " chip--on" : ""}`} aria-pressed={v.id === value}
            onClick={() => { onPick(v.id); close(); }}>
            <VaultFace vault={v} face={faces.get(v.ref.toLowerCase())} />
            <span className="chip__text">{v.name}</span>
          </button>
        ))}
        {rest.length ? (
          <button type="button" className={`chip chip--more${open ? " chip--open" : ""}`} aria-expanded={open} aria-controls={`${id}-panel`}
            onClick={() => (open ? close() : setOpen(true))}>{rest.length} more…</button>
        ) : null}
      </span>

      {open ? (
        <div className="vaults__panel" id={`${id}-panel`}>
          <label className="vaults__find">
            <span className="vh">Find a vault by name, collection slug or rune id</span>
            <input type="search" value={q} placeholder="Find a vault" autoComplete="off" spellCheck={false} autoFocus
              onChange={(e) => setQ(e.target.value)}
              onKeyDown={(e) => { if (e.key === "Escape") { e.stopPropagation(); close(); } }} />
          </label>
          {!found.length ? (
            <p className="vaults__none">No vault matches that. {vaults.length} are here in all.</p>
          ) : (
            <ul className="vaults__list">
              {found.map((v) => (
                <li key={v.id}>
                  <button type="button" className={`vaults__row${v.id === value ? " vaults__row--on" : ""}`} aria-pressed={v.id === value}
                    onClick={() => { onPick(v.id); close(); }}>
                    <VaultFace vault={v} face={faces.get(v.ref.toLowerCase())} />
                    <span className="vaults__name">{v.name}</span>
                    <span className="vaults__meta quiet">
                      {v.coins} coin{v.coins === 1 ? "" : "s"} · <span className="num">{eth(BigInt(v.depositedWei), 3)}</span> ETH
                    </span>
                  </button>
                </li>
              ))}
            </ul>
          )}
          <p className="vaults__say quiet small">Every vault is here, not only the ones on the row. The row shows the vaults coins actually pay into, most first.</p>
        </div>
      ) : null}
    </div>
  );
}
