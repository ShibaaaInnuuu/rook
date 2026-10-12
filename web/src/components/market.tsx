import { useState, type ReactNode } from "react";
import { CoinCards } from "./catalog";
import { VaultControl } from "./vaults";
import { useReadings } from "../lib/caps";
import { COIN_SORTS, filterCoins, matchCoins, sharedSupply, sortCoins, wholeCoins, type CoinRow, type CoinSort } from "../lib/catalog";
import type * as Api from "../api/types";

/**
 * The market: every coin, with the apparatus a market page needs.
 *
 * Sorting, filtering and searching all happen here, in the browser, over the one page of rows the indexer's list endpoint
 * returns, because that endpoint takes a limit and an offset and nothing else (docs/phase7.md). The count always says which
 * slice was sorted, so a page of sixty never pretends to be a ranking of all of them.
 *
 * docs/phase14.md: the controls are the search, one row of sort pills, then a vault control that does not grow with the
 * number of vaults, then a count and a provenance line — rather than three stacked, labelled chipsets one of which printed
 * a button per vault. `Market cap` is the one sort whose number is not on the list response; §3 says where it comes from
 * and what it costs, and the reader is told while it is being read.
 *
 * One thing a market usually sorts by is still missing on purpose: how near a coin's vault is to buying. It is not on the
 * list endpoint and it is not guessed here. docs/phase9.md, section 10, says what to add.
 */
export function CoinMarket({ rows, total, vaults, page = 12, sort = "newest", id, note }: {
  rows: CoinRow[]; total: number; vaults: Api.ListingSummary[]; page?: number; sort?: CoinSort;
  /** so a "View all" elsewhere on the page can reach the grid without touching the hash router */
  id?: string;
  /** the provenance a page wants under its own controls: Explore opens here the claims its header used to carry */
  note?: ReactNode;
}) {
  const [by, setBy] = useState<CoinSort>(sort);
  const [vault, setVault] = useState<number | null>(null);
  const [earning, setEarning] = useState(false);
  const [q, setQ] = useState("");
  const [take, setTake] = useState(page);

  const matched = filterCoins(matchCoins(rows, q), vault, earning);
  // docs/phase14.md §3: one request per coin, under the key the cards already use, and only once this sort is picked
  const caps = useReadings(matched, by === "cap");
  const found = sortCoins(matched, by, caps.caps);
  const shown = found.slice(0, take);
  const supply = wholeCoins(sharedSupply(rows));

  return (
    <div className="catalog" id={id}>
      <div className="controls controls--rows">
        <div className="controls__top">
          <label className="field-inline">
            <span className="vh">Search coins by name, ticker or address</span>
            <input type="search" value={q} placeholder="Search coins" autoComplete="off" spellCheck={false}
              onChange={(e) => { setQ(e.target.value); setTake(page); }} />
          </label>
          <span className="chips" role="group" aria-label="Show">
            {([[false, "All"], [true, "Has routed fees"]] as [boolean, string][]).map(([v, text]) => (
              <button key={String(v)} type="button" className={`chip${v === earning ? " chip--on" : ""}`} aria-pressed={v === earning}
                onClick={() => { setEarning(v); setTake(page); }}>{text}</button>
            ))}
          </span>
        </div>

        <span className="chips chips--sorts" role="group" aria-label="Sort by">
          {COIN_SORTS.map(([v, text]) => (
            <button key={v} type="button" className={`chip${v === by ? " chip--on" : ""}`} aria-pressed={v === by}
              onClick={() => { setBy(v); setTake(page); }}>{text}</button>
          ))}
        </span>

        <VaultControl vaults={vaults} value={vault} onPick={(v) => { setVault(v); setTake(page); }} />
      </div>

      <div className="prov">
        <p className="prov__count quiet">
          {shown.length === found.length
            ? <>{found.length} coin{found.length === 1 ? "" : "s"}</>
            : <>{shown.length} of {found.length === rows.length ? total : found.length} coins, sorted within the {rows.length} this page fetched</>}
          {" · "}prices, supplies and the dollar rate are read on chain
        </p>
        {by === "cap" ? (
          <p className="prov__note quiet small">
            {caps.loading ? <>Reading a price for each of these {matched.length} coins, one request each, because the list endpoint carries a supply and not a price. </> : null}
            Market cap is a coin's whole supply times its latest curve-side price.{" "}
            {supply === null
              ? <>These coins do not all share one supply, so market cap, fully diluted value and price order them differently.</>
              : <>Every Pons coin mints its whole supply at launch — all {matched.length} here share a supply of {supply.toLocaleString("en-US")} coins — so this figure is also the fully diluted value, and sorting by market cap, by fully diluted value or by price gives the same list.</>}
            {" "}A coin whose price or supply could not be read is last, not zero.
          </p>
        ) : null}
        {note}
      </div>

      {!shown.length ? (
        <p className="empty">
          {rows.length
            ? <>No coin matches that. {rows.length} {rows.length === 1 ? "is" : "are"} here in all.</>
            : <>No coin has been launched through the Launcher yet. The first one appears here with its market and what its fees have sent.</>}
        </p>
      ) : (
        <>
          <CoinCards coins={shown} />
          {found.length > take ? (
            <p className="more"><button type="button" className="btn btn--quiet" onClick={() => setTake((t) => t + page * 2)}>Show more, {found.length - take} left</button></p>
          ) : null}
        </>
      )}
    </div>
  );
}
