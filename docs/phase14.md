# Phase 14 — the vault wall, Explore and Analytics: fewer things, each one saying what it is

2026-09-21.

Phase 11 gave the site a real inventory and Phase 12 gave a coin a readable figure. Between them they left three screens that
do not match the work behind them: a filter that prints one button per vault and grows without limit, an Explore page whose
header is a methodology note, and an Analytics page that reads as a filing rather than as a report. This phase rebuilds those
three and adds the one sort a market page is expected to have. The reference held up for all of it is Nosh (usenosh.app), for
its restraint — not its colours: every rule here reads the same custom properties in `web/src/styles/etch.css` that the rest
of the page does, so the stone is unchanged.

## 1. The vault filter, measured

`web/src/components/market.tsx` builds one chip per vault, and `web/src/pages/Receipts.tsx` builds the identical list from the
identical line. Measured in a headless Chrome against the committed demo (60 vaults) on 2026-09-21:

| width | what the vault control is | chips |
| --- | --- | --- |
| 1280px | 1152 × **374** px, above the coins | **61** |
| 400px | 360 × **1,298** px | 61 |

At a phone width the filter is longer than four screens of it, and the whole control block around it is 1,564 px. Nothing
caps it: the list is `vaults.map(...)`, so the height is linear in the number of vaults, and the registry's owner adds vaults
for a living. On staging today it is 60; the catalogue Phase 11 drew from holds 59 entries and is not the last one.

**What replaces it.** One compact control, in a new `web/src/components/vaults.tsx`, used by both pages:

- `All vaults`, then **at most eight** vault chips, then one `More…` button. Eleven chips at 60 vaults, eleven at 600.
- The eight are chosen, not taken in order: **vaults with at least one coin paired to them, by coin count, then by the ETH in
  the vault, then by id.** A filter offering vaults that no coin pays into offers empty lists, and on the demo 15 of the 60
  vaults have no coin at all.
- Each chip carries the vault's own face at 18px: the collection image where the committed catalogue has one, the rune's own
  glyph where the vault is a rune, and the engraved initials otherwise. Measured against the demo's 60 vaults, 59 match a
  catalogue entry by slug or rune id: **39 carry an image, 20 are runes and take the glyph, 1 takes initials.** No vault is
  faceless and no vault waits on a network fetch that can fail — `web/public/catalogue/` is a committed file (docs/phase9.md).
- `More…` opens a panel with a text field that filters every vault as you type, each row carrying its face, its kind and how
  many coins pay into it. It is the same list the chips are drawn from, unsliced, so no vault becomes unreachable.
- The chosen vault is **always on screen**: a vault picked out of the panel is added to the row as a chip even when it is not
  one of the eight, so the control never shows a filtered grid with no visible sign of what filtered it.

The selection logic is a pure function, `vaultPicks`, beside the coin list logic in `web/src/lib/coinlist.ts`, and the panel's
search is `matchVaults`. Both are tested without a browser, which is where the repository already keeps this kind of thing.

## 2. Explore

Today the page is a title, a 296px header paragraph that explains `totalSupply()`, curve-side pricing, the dollar rate and what
is not valued, and then the grid. Every one of those claims is true and is kept. What changes is where a reader meets them.

- **The header is one sentence.** The four claims move into a disclosure under the controls, opened by the reader, with their
  wording intact.
- **A Trending strip above the controls**: a row of coins that scrolls in **its own** container, each card carrying the coin's
  face, its ticker, its name, a large market cap and its 24-hour volume beneath it, with `View all` at the right of the
  heading. §4 is about what "trending" is allowed to mean here.
- **One row of sort pills.** The three stacked, labelled chipsets become: the search field, then one unlabelled row of sorts,
  then the vault control with the `Has routed fees` toggle beside it.
- **A right-aligned count and provenance line** closing the filter block: how many coins of how many, and that the prices,
  supplies and rate behind the figures are read on chain.

## 3. The sort by fully diluted value, and why there is one pill and not three

`COIN_SORTS` gains `["cap", "Market cap"]` — one entry.

Measured on staging on 2026-09-21: **78 coins, and exactly one distinct `supplyWei` among them** — `1e27` base units, a billion
whole coins at 18 decimals. Pons mints a coin's whole supply at launch and mints the same amount every time. So for every coin
on this chain:

```
market cap = supply × price      FDV = supply × price      supply is the same constant for all of them
```

Fully diluted value, market capitalisation and price therefore produce **the same ordering**, and shipping three sorts that
quietly produce one list would be a lie told three times. There is one pill, and the note under the controls says why:

> Market cap is the coin's whole supply times its latest curve-side price. Every Pons coin mints its whole supply at launch —
> all N on this page share a supply of 1,000,000,000 coins — so this figure is also the fully diluted value, and sorting by
> market cap, by FDV or by price gives the same list.

That sentence is **computed, not asserted**: `sharedSupply(rows)` returns the supply only when every row that has one agrees.
If a coin ever launches with a different supply the note says instead that supplies differ and the orderings come apart. The
claim is therefore checked against the data on every render rather than inherited from this document.

**Where the number comes from, and what that costs.** The indexer's list endpoint carries `supplyWei` but no price: a price
lives on `/v1/coins/:address/candles`, one request per coin (docs/phase9.md §10 still asks for a batch endpoint). So the cap
sort reads each row's latest close from that endpoint, through `useQueries` in a new `web/src/lib/caps.ts`, under **exactly the
query key `CardSpark` already uses** — so the coins whose cards are on screen cost nothing extra, and the cap-only observers
add no polling interval of their own. The requests are made **only when the reader picks that sort**. While they are in flight
the count line says the prices are being read; a coin whose price or supply cannot be read sorts **last** and is marked, in the
convention the rest of the site already keeps: not known is not zero.

Ranking is done in the **pair asset** (`supply × close`), never in dollars. The dollar rate is a property of the chain rather
than of a coin, so mixing units across a list cannot happen — but ranking on the unconverted number means a rate that goes
stale mid-sort cannot reorder the list either. Dollars are for **display**, where Phase 12's rate is known.

## 4. 24-hour volume: what the indexer had, and what it has now

The Trending strip wants volume. **The indexer did not serve any per-coin volume** — `CoinSummary` carried `fees24hWei` and
`fees7dWei`, which sum *harvests*, and a harvest is a sweep that happens when somebody calls `harvest()`. A coin that traded
hard and has not been swept reports nothing, and a coin swept twice in an hour reports a window's worth of an older window's
trading. It is a real number and it is not volume.

The `trades` table has what is needed and is already indexed for the question (`trades_by_coin_ts`), so this phase adds it:

```
volume24hWei   the sum of the pair-asset leg of every trade in this coin in the last 24 hours, in wei
```

One aggregate per coin on the list response. It is the **same quantity** the chart's own volume already reports — the `quote`
leg exactly as the chain recorded it, which on a buy is what the buyer sent and on a sell is what the seller received, both
before any adjustment for Pons's fee or the creator tax (docs/phase10.md keeps the gross legs on each trade for anyone who
wants to divide them differently). `sum(candle.v)` over `range=1d` and `volume24hWei / 1e18` measure the same trades. It is
never null: a coin nobody traded has volume `0`, which is a fact.

**The strip's ordering, and the one inexactness in it, named.** The deployed staging indexer and the committed recording both
predate this field, and I do not deploy the indexer (the operator does), so the site must be honest on an indexer that does
not serve it. Two stages:

1. **A shortlist of 16**, from the list endpoint alone: by `volume24hWei` where it is served, and otherwise by fees routed in
   the last 24 hours, then lifetime fees, then newest.
2. **The order**, and the figure printed: the 24-hour volume itself — from `volume24hWei` when the indexer serves it, and
   otherwise measured from each shortlisted coin's own `range=1d` candles, which is the same quantity read the other way.

So the column and the order always agree, and both are 24-hour volume. The only approximation is the shortlist, and the strip
says which key drew it, in the heading, in words: *"by 24-hour volume"*, or *"by 24-hour volume, of the 16 coins this indexer's
own 24-hour fee figure shortlisted"*. **Nothing on this page is labelled 24-hour volume that is not 24-hour volume.** Once the
indexer serves the field the shortlist becomes exact and the strip needs no change and no extra request.

The strip's market cap comes from the same candle responses, in dollars where Phase 12's rate is known and in the pair asset
where it is not, which is what the coin cards already do.

## 5. Analytics

102 lines of `<dl className="strip">` under five headings, plus a prose block. Every number in it is worth keeping. What it
lacks is a shape: nothing is a headline, nothing says what the figure means, and a reader who wants the one number the page
exists for has to read all of them.

- **Four stat cards**: a label, a large figure, and one line of context. Thin hairline and a 2px radius from the same `--cut`
  idiom the rest of the file uses — this stylesheet's first rule is that nothing floats, so these are recesses in the stone
  and not cards over a background.
- **Three titled panels.** Each: the heading on the left, a small summary figure on the right, **one short paragraph saying
  what the number means and what cannot happen to the money**, then three figures divided by hairlines — a label, a large
  value, and beneath it the same amount in dollars. A footnote states the rate and that everything else is read from the
  contracts.
- The by-vault bars and the `Where these come from` block are kept. They are the substance, and the second is the page's
  honest core: which figures are folded from events, which are read at request time, and which are somebody's word.

**One thing measured after the fact, and amended here rather than left.** The by-vault bars drew one bar per vault, and at
the demo's 60 vaults that is **4,433 px** of them — the identical fault to the vault filter this same phase exists to
remove, on the same screen, found by measuring the rebuilt page rather than by reading the code. They are capped at the
**twelve** vaults the most ETH has reached, with a line saying how many are not drawn and that the vaults page lists every
one with its own balance, its cap and its budget. Nothing about the figures changes; the page simply stops growing without
limit. The write-off line was corrected in the same pass: the recording has one purchase written off for **zero** ETH, which
printed as `0.0000 ETH over 1 purchase`, and now reads as what it is.

Two departures from what was asked for, both because the data is not there:

**"Graduated" is not one of the four cards.** No endpoint counts graduations. `/v1/stats` has no such field, `CoinSummary` has
no such flag, and the only place graduation appears is one coin's own chart and a live read on its own page. Counting it would
mean 78 live reads at page load, or inventing the number. The fourth card is **vaults with a coin paying into them**, which is
derived from the listings response the page already has, and §Where these come from says in a sentence why a graduation count
is not on the page.

**The second reading under each figure is dollars, not sats.** A conversion to sats needs a Bitcoin price, and this site has
no Bitcoin price: the catalogue's floors are dated sats fetched by hand (docs/phase9.md) and are prices of assets, not a rate.
The one rate the chain publishes is Phase 12's dollars per ETH, read from `slot0()` on one Uniswap v3 pool, so that is the
conversion, and the footnote says exactly what it is and is not. The one place a real sats figure exists is a purchase — a
vault's ETH out and the `satsPaid` the receipt recorded — and the spending panel prints that, because it is a recorded number
rather than a conversion.

The rate reaches Analytics the way it reaches everything else: off a candles response, since that is where the indexer
publishes it. `useUsdRate` reads it from the newest coin's `range=1d` response — one request, the same query key the cards
use — and is null when there is no coin, no pool, or a stale rate, in which case no dollar line is drawn at all.

## 6. At 400px

The stylesheet's standing rule is that the page body never scrolls sideways and only a table, a diagram or a chart scrolls
inside its own box. The Trending strip is such a box: `overflow-x: auto` on the rail, the page unaffected. The stat cards go
four-up, then two-up, then one. The vault control wraps and, capped at eleven chips, cannot become a column again. Every
colour is a custom property defined on bare `:root` and redefined under both the media query and `[data-theme]`, so nothing
has its only definition inside a theme block.

## 7. What this phase does not claim

- **Trending is not a prediction.** It is a sorted list of what traded in the last 24 hours, and on staging every one of those
  trades was made by a simulator we run (docs/phase11.md §7). A quiet coin near the top means the day was quiet.
- **"Market cap" is a curve-side arithmetic, not a valuation.** Supply times the last price the curve traded at. Most of that
  supply is still inside the curve unsold, which is exactly why the figure equals the fully diluted value and why this page
  says so rather than printing a float-based number it cannot compute.
- **The cap sort ranks the page, not the chain.** It sorts the rows the list endpoint returned, as every sort on this page
  already does, and the count line goes on saying so.
- **A vault outside the eight is not unimportant.** The eight are the ones coins actually pay into, most-paired first. The
  panel holds all of them and the search reaches all of them.
- **`volume24hWei` is untested against a graduated coin's pool.** It sums the `quote` leg of both venues, and no coin on our
  fork has ever graduated (docs/phase11.md §8), so the pool half of that sum has been exercised only by the synthetic story.

## 8. Amendment, 2026-09-22 — the vault's own page

Phase 14 rebuilt three screens against Nosh's restraint and left the fourth alone. `#/listing/<id>` is the page a reader lands
on when they have decided a vault is interesting, and it is the thinnest page on the site: a heading, four figures, a
declarations table and a receipts table. Nosh's collection page — read off screenshots of OpenCatz on 2026-09-22 — answers, in
order: what the vault holds, what it may pay per item, when that ceiling was posted, how much of one item it can already
afford, how many it has bought, what it has spent, what it has taken in, what it is holding, which coins fed it and in what
proportion, and every purchase it has made. That is the right list of questions and none of them is asked here today.

This section rebuilds the page to answer all of them. **It is not a translation.** Etch buys inscriptions and runes on
Bitcoin, not NFTs through Seaport, and four of Nosh's rows are either false here, differently shaped here, or absent here.
Every one of those is named below with what Etch says instead and why.

### 8.1 The panel, row by row, against the reference

| Nosh's row | its sub-label | what Etch prints | why it is not the same row |
| --- | --- | --- | --- |
| Budget | ETH the vault holds | **Budget** — `spendable`, with the contract's whole balance beside it | The balance is not the budget. Measured on the committed recording: vault 0 holds **0.2627 ETH**, of which **0.2498** is the purchaser's bond and **0.0129** is spendable. A page that printed "budget 0.2627 ETH" would be out by twenty times. |
| Floor cap | max ETH per NFT, with a **live** badge | **Price cap** — "the keeper's ceiling per inscription", or "per whole rune unit" | Etch's cap is not a floor read off a marketplace; it is the keeper's word about a market this chain cannot read, and it **expires an hour after it is posted** (`AssetVault.CAP_TTL`). Live/expired is kept, and the expired case says what it stops: nothing can be declared without a fresh one. |
| Cap updated | relative · absolute | **Cap posted** — relative · absolute | The same row. It was already in the response (`capHistory[0].at`) and no screen read it. |
| Can sweep now | budget ÷ floor cap | **Buys now** — spendable ÷ cap, and for a rune, whole units, not items | A rune is fungible: "0.62× of one rune" is not a quantity anybody has. The divisor is `capWei` per whole unit, and the figure is said in whole units of that rune. |
| Total swept | 263 NFTs | **Bought** — see §8.3 | Two kinds, one row, and neither is "263 of them". |
| Total spent | | **Spent** — released − returned over settled purchases | New from the indexer (§8.4). |
| Fees received | from coin routers | **Fed in** — every `Deposited`, with the routers named | Nearly the same, with one addition Nosh has no analogue for: `depositFrom` lets the treasury's own share arrive attributed to the protocol coin (`docs/token/protocol-token-v1.md` §2). Both emit the same event and neither is distinguishable in it, so the row says "the coins' own routers, and the treasury's share attributed to the protocol coin" rather than claiming only the first. |
| Donations | | **Unattributed** — balance − attributed − waiting − bond | **Etch has no donation path.** `deposit` reverts for anyone but that coin's own router and `depositFrom` for anyone but the treasury. ETH can still be forced in by `selfdestruct`, and the contract's own header says what happens to it: *"it is never spendable, which is safe and wasteful."* So the row in that position is the one honest thing there is to say about ETH nobody is credited with. Measured on the recording: **exactly zero on both vaults** — every wei is a coin's or the purchaser's. |
| Held | Nothing held. The next sweep lands here. | **Held** — see §8.3 | Same row, both kinds. |

Three rows have no counterpart on the reference and are kept because they are how Etch is safe at all: the **purchaser's bond**
against what is released and unsettled, **written off** where a receipt never came and the bond did not cover it, and the
**blocked** banner naming the purchase past its deadline. They were already on the page and they stay.

### 8.2 "Bought by the vault" is a claim Etch cannot make

Nosh's box says: *"The vault contract buys this collection itself through Seaport. Nothing but an NFT purchase can move its
ETH — there is no withdraw function."* That is an atomic guarantee: the ETH and the NFT move in one transaction, so there is
no interval in which somebody holds the money and owes the asset.

**Etch has no such interval-free path and must not imply one.** The asset is on Bitcoin; this chain cannot see it, cannot
escrow it and cannot condition a transfer on it. What `AssetVault` has instead is a procedure, and the page states it as one:

> The vault does not buy anything itself. It releases ETH to one named purchaser, who buys on Bitcoin and comes back with a
> receipt. Every release is **declared in public an hour before it can happen**, for **no more than the posted cap allows**,
> to the purchaser named at the declaration, from a vault that may have at most **three** declarations open. The purchaser
> has **three days** to produce a Bitcoin transaction paying the listing's public address, and until they do, that ETH is
> covered by a **bond** they posted. Miss the deadline and the vault declares nothing more until the purchase is settled or
> written off; a write-off pays the charged coins back out of the bond, and what the bond does not cover is recorded for
> good. Seven days past the deadline, anyone may do the writing off.

And the weaker half, in the same box rather than in a footnote: a purchaser who takes the ETH and buys nothing costs the coins
whatever the bond does not cover. The contract's own header says so first — *"the purchaser keeping released ETH, or buying
badly… is not bounded to zero"* — and the page does not say better of it than the contract does.

One more difference, because "there is no withdraw function" is **not** true of Etch as flatly as it is of Nosh.
`AssetVault.withdrawBond` sends ETH out of the contract. It sends the **purchaser's own posted bond** back to the address that
posted it, never below the share of what is still in their hands, and it reverts while any of their released purchases is
overdue. It cannot reach a wei that a coin fed in. The page says exactly that, because a reader who greps the source and finds
a function called `withdrawBond` after reading "there is no withdraw function" has been told something false.

### 8.3 "Swept 263 NFTs" needs a form that works for a fungible thing

An inscription is one of a kind and a rune is an amount. A single sentence cannot count both, and rounding a rune into a count
of receipts is the fault: vault 1 in the recording has **26 receipts** that between them carry **371,165.08 DEMO•RUNE**, and
"26" is not what it bought.

So the row is a count for a collection and a quantity for a rune, and the indexer serves both rather than the page guessing:

- **inscriptions** — `N inscriptions bought`, and held is `N − delivered`.
- **rune** — `X.XX RUNE received over N receipts`, and held is `received − paid out − delivered`, in the rune's own units at
  its own divisibility.

The one trap here, which the API now documents where the field is: for a rune settle the contract calls `_record` **once per
paying coin**, and each of those rows carries the **same `btcTxid` and the same `satsPaid`**. Measured in the recording,
purchase 13 of vault 1 is five acquisition rows, one Bitcoin transaction, `satsPaid: 1000` printed five times. Amounts sum;
sats do not. Nothing on the page sums `satsPaid` across rows, and the field's own comment now says why.

### 8.4 What the indexer did not have

Most of the panel was already served. Four things were not, and each is a number the browser cannot honestly compute from
what it is given:

| added | to | why not in the browser |
| --- | --- | --- |
| `capPostedAt` | `ListingSummary` | The list response carried the cap and its expiry but never when it was posted, so the vaults list could say "expires in 41 min" and not "posted 19 min ago". It is one column on a query that already runs. |
| `releasedWei`, `returnedWei`, `spentWei` | `ListingDetail` | `declarations` is capped at 100 rows. Summing a truncated list is how a total becomes quietly wrong on the first busy vault. `spentWei` uses the **same definition `/v1/stats` already uses** — released − returned over *settled* purchases — so the vault's figure and the protocol's are the same arithmetic. |
| `holdings` | `ListingDetail` | `acquisitionsRecent` is capped at 50 and carries no payouts at all, so neither the rune quantity nor the held remainder is reachable from it. The rune half of this was already computed inside `verify` for the `expectAtLeast` hint; it is now one method serving both, which also means the page and the verification hint can never disagree. |
| `purchaseSpentWei`, `purchaseAcquisitions` | `Acquisition` | A receipt row wants to print what the vault paid for it, and the vault charges **per purchase, not per item**: one purchase produced 3 inscriptions in vault 0 and 5 rune rows in vault 1. Both numbers are served and **neither is divided by the other here**, because the contract's own division is by *declared* quantity and `settleShort` makes that a different number from what an item cost. The page prints the purchase's figure and says how many receipts came out of it. |

Deliberately **not** added, because they are ratios of two numbers already in the same response and adding them would be a
second place for the same fact to be wrong: a coin's **share** of what was fed in (`coinsList[].depositedWei` over
`depositedWei`), and **unattributed** (`live.balanceWei − attributedWei − waitingWei − bondWei`).

### 8.5 Paired coins, and the sentence that is different

Nosh's table is COIN / FED TO VAULT / SHARE, under: *"Share weights raffle tickets: holders of a coin that fed more of the
vault get more tickets."* The table is the same here — `coinsList` has every column. **The sentence is false on Etch.**

A draw on Etch is not over the vault. Every receipt names the one coin whose fees paid for it (`Receipted(…, coin, …)`), only
that coin's creator may choose its fate (`Distributor.choose` reverts for anybody else), and the snapshot the draw runs over is
that coin's own holders — the command that recomputes the root takes `--coin <address>` and nothing else. A rune distribution's
unclaimed remainder rolls over to that same coin's next distribution (`carry[listingId][coin]`), not to the vault's.

So the line under the table is:

> A share here does not weight a draw. Every receipt names the one coin whose fees paid for it, and what happens to it is
> chosen by that coin's creator and drawn among that coin's holders alone. What a bigger share buys is more receipts, not
> better odds on somebody else's.

### 8.6 The rest of the page

- **A header that says where you are**: the collection's own artwork from the committed catalogue, the name, the kind, the
  chain, the registry's `ref` linking out to the marketplace the catalogue fetched it from, the vault contract with a copy
  control and an explorer link where the chain has an explorer (`explorerFor`: Robinhood Chain has one, our forks do not), and
  **Launch a coin** — which, unlike the reference's, arrives at the launch form with this vault already chosen (`#/launch/<id>`).
- **Purchases** keeps its table. It is the stage Nosh has no analogue for — a declaration is public for an hour before the
  money moves — and it is the one part of this page a reader can act on, with `Release` on a matured declaration.
- **What the vault has bought** becomes the reference's sweep history: what was bought, what the purchase cost, which coin
  paid, how long ago, the Bitcoin transaction, and **the delivery state**, which is the column Nosh does not have — held, a
  draw open, won, delivered, rolled over, or written off.

### 8.7 At 400px, and what is not claimed

The panel is the page's existing `.figures` grid, which already goes from four columns to one, and both tables are
`.book__table` inside `.book__scroll` — the same material as the trades and holders panel, so a table scrolls in its own box
and the page body never does. No new colour is introduced: every rule reads the custom properties in `web/src/styles/etch.css`.

- **"Buys now" is not a promise that it will.** It is spendable ÷ cap at this instant, and the cap expires within the hour.
- **The recording predates every field in §8.4.** It was taken off the staging indexer on 2026-09-21 and this repository does
  not deploy that indexer. Each new field is read as *absent* rather than as zero, exactly as `volume24hWei` is in §4, and the
  row says it is not reported by this source. The synthetic set is regenerated here and does carry them.
- **Nothing on this page says an inscription is owned by the vault.** It says a receipt was recorded and gives the links to
  check it against Bitcoin, which is the strongest claim anything in this repository is entitled to make.
