import { useMemo, useState } from "react";
import { Ext } from "./ui";
import { EntryArt, Monogram, RuneFace } from "./Plate";
import { Chips } from "./catalog";
import { Provenance } from "./panels";
import { href } from "../lib/env";
import { choices, isRune, matchChoices, rankedFigure, satsParts, searchCatalogue, vaultOf, vaultsByRef, vaultTag,
  type Catalogue, type CatalogueEntry, type Choice, type ChoosableVault, type VaultRef } from "../lib/catalogue";

/**
 * The catalogue, browsable: real Bitcoin collections and real runes, with their real artwork and the floor they carried on
 * the day it was fetched.
 *
 * It arrives in the order the fetch ranked it — fifty of each kind, on the two bases the file states separately — so each
 * card prints the figure its own list was ranked by, and `Provenance` states both bases in words underneath. A list whose
 * order is a claim must show the number the claim is made of. docs/phase9.md §11, §11.3.
 *
 * **A vault is a fact about an entry, not a section it lives in.** Every collection and rune in the catalogue is meant to
 * get one, and on staging every one already has; so the card carries its own status, and a reader sees one list rather than
 * a list plus a bucket asserting an absence that has stopped being true. docs/phase9.md §11.3.
 */

export function CatalogueCard({ entry, vault, ranking, note }: {
  entry: CatalogueEntry; vault?: VaultRef; ranking?: Catalogue["ranking"]; note?: string;
}) {
  const rune = isRune(entry);
  const figure = rankedFigure(entry, ranking);
  return (
    <li className="card">
      <div className="card__in card__in--still">
        <EntryArt entry={entry} tag={vaultTag(vault)} />
        <span className="card__body">
          <span className="card__head">
            <span className="card__name">{entry.name}</span>
            <span className="tag">{rune ? "rune" : "inscriptions"}</span>
            {vault?.paused ? <span className="tag tag--warn">paused</span> : null}
          </span>
          <span className="card__num num">{rune ? (entry.symbol ?? "no symbol") : satsParts(entry.floorSats)[0]}</span>
          <span className="card__unit">{rune ? `${entry.divisibility} decimals, block ${entry.etchedBlock.toLocaleString("en-US")}` : `${satsParts(entry.floorSats)[1]} floor, as fetched`}</span>
          <span className="card__foot">
            {rune ? (
              <span>{entry.mintable ? "still mintable" : "minting closed"}</span>
            ) : (
              <>
                <span>{entry.supply ? `${entry.supply.toLocaleString("en-US")} items` : "supply not known"}</span>
                {entry.items[0]?.name ? <span>{entry.items[0].name}</span> : null}
              </>
            )}
            {figure ? <span>{figure}</span> : null}
            <Ext href={entry.sourceUrl}>{entry.source}</Ext>
          </span>
          {vault ? (
            <span className="card__why">
              {vault.paused
                ? "Its vault is paused, so no coin can be paired with it until the registry's owner unpauses it. "
                : "Its vault holds the ETH that coins paired with it have sent, and can buy this and nothing else. "}
              <a href={href("listing", vault.id)}>Vault {vault.id}</a>
            </span>
          ) : (
            <span className="card__why">No vault yet. The registry's owner creates one; until then no coin can pay into it.</span>
          )}
          {note ? <span className="card__unit">{note}</span> : null}
        </span>
      </div>
    </li>
  );
}

type Kind = "all" | "inscriptions" | "rune";

/** A search box, a rune-or-inscription filter, and the grid. The filter cross-fades rather than snapping. */
export function CatalogueBrowser({ catalogue, vaults, initial, limit = 24 }: {
  catalogue: Catalogue | undefined; vaults?: readonly VaultRef[]; initial?: string; limit?: number;
}) {
  const [q, setQ] = useState(initial ?? "");
  const [kind, setKind] = useState<Kind>("all");

  const all = useMemo<CatalogueEntry[]>(
    () => [...(catalogue?.collections ?? []), ...(catalogue?.runes ?? [])],
    [catalogue],
  );
  const by = useMemo(() => vaultsByRef(vaults), [vaults]);
  const pool = kind === "all" ? all : all.filter((e) => e.kind === kind);
  const shown = searchCatalogue(q, pool, limit);
  const without = all.filter((e) => !vaultOf(e, by)).length;

  if (!catalogue) {
    return <p className="empty">The catalogue could not be read, so there is nothing here to browse. It is a file in this site, so this normally means the file is missing from the build.</p>;
  }

  return (
    <div className="catalog">
      <div className="controls">
        <label className="field-inline">
          <span className="vh">Search the catalogue</span>
          <input type="search" value={q} placeholder="Search the catalogue" autoComplete="off" spellCheck={false}
            onChange={(e) => setQ(e.target.value)} />
        </label>
        <Chips label="Kind" value={kind} options={[["all", "All"], ["inscriptions", "Collections"], ["rune", "Runes"]]} onPick={(v) => setKind(v as Kind)} />
      </div>
      {!shown.length ? (
        <p className="empty">Nothing in the catalogue matches that. It holds {catalogue.collections.length} collections and {catalogue.runes.length} runes, fetched once and committed.</p>
      ) : (
        <ul className="cards cards--fade" key={`${kind}-${shown.length}`}>
          {shown.map((e) => <CatalogueCard key={e.id} entry={e} vault={vaultOf(e, by)} ranking={catalogue.ranking} />)}
        </ul>
      )}
      <p className="count quiet">
        {shown.length === pool.length ? `${pool.length} shown` : `${shown.length} of ${pool.length} shown`}
        {vaults && without ? `, ${without} of which ${without === 1 ? "has" : "have"} no vault yet` : ""}
        {catalogue.runes.length === 0 ? ", and no runes: not one ticker the fetch ranked could be resolved to a rune id, and a vault commits to an id" : ""}
      </p>
      <Provenance catalogue={catalogue} />
    </div>
  );
}

/**
 * The step where a creator decides which vault their coin's fees will pay into, for good.
 *
 * ONE list of the collections and runes, each card carrying its own vault status. A card whose vault exists is pickable; a
 * card whose vault does not yet exist says so on itself and cannot be picked; a paused vault says which of the two it is,
 * because "paused" and "not created" are different facts and a reader must not have to guess. When everything has a vault —
 * the normal case now and the intended case on mainnet — this is simply a list of collections and runes, with no apology
 * attached. docs/phase9.md §11.3.
 */
export function VaultChooser({ vaults, value, onPick, catalogue }: {
  vaults: ChoosableVault[]; value: string; onPick: (id: string) => void; catalogue: Catalogue | undefined;
}) {
  const [q, setQ] = useState("");
  const rows = useMemo(() => choices(catalogue, vaults), [catalogue, vaults]);
  const shown = matchChoices(q, rows);
  const open = rows.filter((r) => r.vault && !r.vault.paused).length;
  const without = rows.filter((r) => !r.vault).length;

  return (
    <div className="chooser">
      <div className="controls">
        <label className="field-inline">
          <span className="vh">Search collections and runes</span>
          <input type="search" value={q} placeholder="Search collections and runes" autoComplete="off" spellCheck={false}
            onChange={(e) => setQ(e.target.value)} />
        </label>
        <span className="quiet small">{open} of {rows.length} can be chosen</span>
      </div>

      <p className="offered__say">
        A coin pays into one vault and can never be pointed at another, so this choice is permanent. Everything with a vault
        can be chosen here.{without ? ` ${without} of these ${without === 1 ? "has" : "have"} no vault yet: the registry's owner creates one, and until then no coin can pay into it.` : ""}
      </p>

      {!rows.length ? (
        <p className="empty">No vault exists and the catalogue could not be read, so there is nothing a coin's fees could pay into yet.</p>
      ) : !open ? (
        <p className="empty">No vault exists, so there is nothing a coin's fees could pay into yet. A vault has to exist before a coin can be paired with it.</p>
      ) : !shown.length ? (
        <p className="empty">Nothing matches that. {open} of {rows.length} collections and runes can be chosen.</p>
      ) : (
        <ul className="cards" role="radiogroup" aria-label="The vault this coin's fees pay into">
          {shown.map((r) => <ChoiceCard key={r.key} row={r} on={!!r.vault && String(r.vault.id) === value} onPick={onPick} ranking={catalogue?.ranking} />)}
        </ul>
      )}
    </div>
  );
}

/** A card that is a button when it can be chosen, and plain when it cannot, so nothing unpickable looks pickable. */
function ChoiceCard({ row, on, onPick, ranking }: {
  row: Choice; on: boolean; onPick: (id: string) => void; ranking?: Catalogue["ranking"];
}) {
  const pickable = !!row.vault && !row.vault.paused;
  const tag = vaultTag(row.vault);
  const art = row.entry
    ? <EntryArt entry={row.entry} tag={tag} />
    : row.kind === "rune"
      ? <span className="plate plate--rune"><RuneFace symbol={null} name={row.name} /><span className="plate__tag">{tag}</span></span>
      : <span className="plate"><Monogram id={`listing-${row.vault?.id ?? row.ref}-${row.ref}`} text={row.name} /><span className="plate__tag">{tag}</span></span>;
  const figure = row.entry ? rankedFigure(row.entry, ranking) : null;
  const body = (
    <span className="card__body">
      <span className="card__head">
        <span className="card__name">{row.name}</span>
        <span className="tag">{row.kind === "rune" ? "rune" : "inscriptions"}</span>
        {row.vault?.paused ? <span className="tag tag--warn">paused</span> : null}
      </span>
      <span className="card__unit mono">{row.ref}</span>
      {figure ? <span className="card__unit">{figure}</span> : null}
      <span className="card__unit">
        {!row.vault
          ? "No vault yet, so it cannot be chosen. The registry's owner creates one."
          : row.vault.paused
            ? "Its vault is paused, so no coin can be paired with it."
            : on ? "chosen. Permanent once launched." : "choose this"}
      </span>
    </span>
  );
  return (
    <li className="card">
      {pickable ? (
        <button type="button" role="radio" aria-checked={on} className={`card__in card__pick${on ? " card__pick--on" : ""}`} onClick={() => onPick(String(row.vault!.id))}>
          {art}
          {body}
        </button>
      ) : (
        <div className="card__in card__in--still">{art}{body}</div>
      )}
    </li>
  );
}
