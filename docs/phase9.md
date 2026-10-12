# Phase 9: the site made alive, and real Bitcoin assets to pair with

Written before the code. Phase 7 gave the site the pages a launchpad needs; they are correct and inert. Two things are missing,
and the operator named both: the site does not **move**, and it has no **pictures**. A launchpad whose whole promise is "your
coin's fees buy real Bitcoin assets" that shows no Bitcoin asset is asking to be taken on faith.

Nothing in the identity changes. Same carved stone, same `etched` hero (untouched), same Tenor Sans / Hanken Grotesk / Plex Mono,
same single ember accent, same scored rules instead of floating cards. This is a craft pass on life and imagery, not a re-skin.

## 1. The new material: the plate

Every picture on this site sits in a **plate** — a square recess cut into the stone with the same lit lower-right lip and
shadowed upper-left wall as every other cut in the system. The artwork is inset inside the recess, so it reads as an object set
into the surface rather than a card floating above it. One shape, three fills:

| Fill | When | Source |
|---|---|---|
| **Artwork** | an inscription, or a catalogue collection's thumbnail | `ordinals.com/content/<id>`, or the catalogue's committed image |
| **Rune plate** | a rune listing, which has no artwork by design | the rune's own symbol glyph, engraved, over its spaced name |
| **Monogram** | a coin, or any asset whose image is missing, slow, broken, or not an image | drawn from the address: two characters, engraved, over a chisel pattern whose angle and seed come from the address |

The monogram is not a placeholder-grey box and not a coloured identicon blob. It is the same carved letterform the wordmark uses,
so a coin with no logo still looks like it belongs here. Every plate reserves its aspect ratio before it loads, so nothing shifts.

## 2. Five live panels on the front page

Each is a working miniature of the page it opens, each reads the indexer, each degrades to a sentence rather than a zero.

| Panel | Reads | With nothing live |
|---|---|---|
| **Listings strip** | `/v1/listings` for what is listed, the committed catalogue for what could be | drifts through the catalogue and says these are available to list, none listed yet |
| **Receipts feed** | `/v1/acquisitions`, polled; each row opens its Bitcoin transaction | "No vault has bought anything yet. The first purchase appears here with its Bitcoin transaction." |
| **The line** | `/v1/acquisitions` (`settledAt`, `satsPaid`) with 24h / 7d / all | the axis and the sentence, no invented line |
| **Search** | the coins and listings already fetched, plus an address match | still searches the catalogue, which exists before any chain does |
| **What the contracts enforce** | nothing; these are constants of the code | always the same, which is the point |

**The line plots what the vaults spent, not what they took in, and says so on the panel.** The brief asked for fees over time.
The indexer timestamps settlements and launches; it does not timestamp fees. A coin carries `fees24hWei`, `fees7dWei` and a
lifetime total, which are three figures, not a series, and `lastHarvestAt`, which would attribute every coin's whole routed
total to one instant. Drawing any of those as a line over time would invent shape. Every acquisition, by contrast, carries the
second it settled and the sats it paid, so that is the series: real timestamps, real amounts, and cross-chain, which is our
best material. It is a running total, so it steps at each settlement rather than sloping between them. ETH routed to vaults
stays where it is already honest, in the figures above. If the indexer ever timestamps harvests, the same panel takes them.

The fifth panel is deliberately the one that needs no data: 80/20 fixed in bytecode, the public hour, no withdraw function, the
cap and its hour, the fifteen-minute challenge window. It is the only panel that can never be empty, and it says the thing the
product is actually selling.

## 3. The catalogue: real collections and runes to pair with

The Launch wizard's listing step is where a creator decides what their coin's fees will buy. Offering it two fixtures is the
single weakest screen on the site. So the site ships a **catalogue** of real Bitcoin collections and runes.

**It is built once, at build time, and committed.** `web/scripts/catalogue.mjs` fetches, downloads each thumbnail, and writes
`web/public/catalogue/catalogue.json` plus the images beside it. A visitor's browser never calls a marketplace: no key in the
bundle, no rate limit, no page that breaks when someone else's API does, and no dependency on a permission we have asked for and
not yet received. The file records the date it was fetched and the site prints it, so a floor is always "as of", never implied
live.

**Sources, in order.** This section is the record of the fetch that built the file first shipped, and its "top 40 by
lifetime volume" is exactly what **section 11** corrects: the catalogue is now the top fifty of each kind by the volume
traded in the week before the fetch, and a keyless run no longer produces `catalogue.json` at all.

1. **UniSat**, when the operator supplies `UNISAT_API_KEY` in the environment: `POST /v3/market/collection/auction/collection_statistic_list`
   for collections and `POST /v3/market/runes/auction/runes_types` for runes. This is the preferred source and the one the
   operator asked for. **Commercial use of their API requires written permission under §3.1 of their developer terms; we have
   asked and not received it.** The script therefore never runs in CI, never runs from the site, and is rate-limited to well
   under their free tier's 5 calls per second with a delay between calls. Reads only: no endpoint that places, changes or
   cancels an order is ever called. UniSat is credited wherever their data or images appear.
2. **Without a key**, which is how the committed file was in fact built. Every endpoint below was called once on
   **2026-09-21**, from one machine, sequentially, with a 260 ms delay between calls enforced in one place in the script:
   - **collections**: `GET https://turbo.ordinalswallet.com/collections?offset=&limit=100`, six pages, 600 rows, filtered to
     those with a floor, a supply under 250,000 and usable artwork, then the top 40 by lifetime volume. It gives name, slug,
     description, floor in sats, supply, listed count and the inscription-number range.
   - **the items in a collection**: `GET https://turbo.ordinalswallet.com/collection/<slug>/inscriptions?limit=8`, once per
     kept collection, for the real inscription ids and item names the cards show.
   - **artwork**: `GET https://ordinals.com/content/<inscription id>`, once per kept collection, from the reference ord
     server. 40 of 40 collections got real artwork this way, so every card shows a thing that is actually inscribed rather
     than a marketplace's thumbnail.
   - **runes**: `GET https://ordinals.com/rune/<name>` for a seeded list of well-known runes. The reference ord server
     publishes every rune's id, etching block, divisibility, symbol, supply and whether it is still mintable on a public page
     with no key, which is everything a listing needs and everything the plate draws. 19 of 30 seeds resolved; the other 11
     are not etched under those spellings and were dropped rather than guessed. Runes are seeded rather than enumerated
     because ord lists them in etching order, which is mostly machine-named noise.
   - **what could not be fetched: a rune floor.** A floor needs a marketplace, and every keyless one refuses: UniSat's market
     endpoints answer `code -2004` unauthenticated on every path tried, Hiro's ordinals and runes APIs are retired, Ordiscan
     charges, and ordinals.com's JSON API for runes is disabled. Every rune in the file therefore carries `floorSats: null`
     and the site prints no price for one. With `UNISAT_API_KEY` in the environment the script fills them in.

   UniSat was **not** called on this fetch, keyless or otherwise, beyond the probes that established the above. Both sources
   are credited on every screen their data or pictures appear on.

**What shipped.** 40 collections and 19 runes, one committed picture each, 904 KB in total under `web/public/catalogue/`.

**Membership.** A listing's on-chain membership commitment is a root over inscription ids, so a catalogue entry carries the
collection's inscription-number range and its source, and the registry owner still commits the real root. The catalogue is what
the site offers, never what the contract trusts. Nothing in `#/listings` blurs the two: "listed on chain" and "available to
list" are separate, and today the first is empty on the real chain.

## 4. Life, and what each piece of motion is for

One idea, applied consistently: **things that arrive are cut in.** Nothing bounces; nothing loops except the strip.

| Motion | What it communicates |
|---|---|
| A new receipt row wipes in from its left edge | something happened since you looked |
| Figures count to their value once | the number moved, and by how much |
| Countdowns tick | a window is closing, and you can act before it does |
| The listings strip drifts, pausing on hover | there is more here than fits |
| A plate lifts on hover, its recess deepening | this is a link |
| A filter change cross-fades | the same list, re-ordered, not a new page |

Everything above collapses under `prefers-reduced-motion`: the strip stops, the wipe becomes an appearance, the counters print
their final value, the countdowns still tick because a countdown that does not tick is wrong. Every panel is beautiful still.

## 5. What the parent should add to the indexer later

The site needs one field it cannot get: **a coin's logo.** Pons takes `logo` as a launch parameter (`LaunchParams.logo`, a URL
string) and stores no getter for it, so it exists only in the launch event's data. The indexer already reads that event to
record a launch. It should carry the string through:

```
CoinSummary.logoUrl: string | null   // LaunchParams.logo as given at launch; null when absent or not read
```

Until it does, every coin on this site wears its carved monogram, which is by design good enough that adding the field is an
improvement rather than a repair. The component already prefers a logo when one is present, so the only change needed is in the
indexer. Nothing else on the site is waiting on the indexer.

## 6. How this could mislead

- A catalogue floor is **as of a date**, from one marketplace, for a collection whose price moves. It is shown with its date and
  its source and is never used by any contract.
- A collection in the catalogue is **available to pair with**, not endorsed, not verified by us, and not listed on chain.
- Inscription artwork is fetched from a public gateway at view time. A gateway that is down shows the monogram, not a broken
  image, and never a different picture.
- On staging every coin, price, receipt and delivery is simulated; the ribbon says so and the fixtures are labelled.

## 7. What the three review rounds changed

Screenshots at 1440 and 400, dark and light, looked at full size rather than counted.

**Round one** found the cards were the problem the panels were meant to solve. A two-coin grid stretched two cards across the
whole page, each with a square plate nearly 500 px tall holding two faint letters, and on a phone one coin filled a screen and
a half. The running total was drawn as a diagonal, which turned two purchases into a trend that never happened. Fixed: an
explicit grid, a shallower plate for a mark than for a picture, and a line that steps.

**Round two** found the grid had been fixed with viewport media queries, which are blind to the wizard's narrow column: inside
it the same rule gave eight-five-pixel cards with names broken across three lines. Replaced with `auto-fill` over a minimum
track, which sizes to its container and, unlike `auto-fit`, leaves an empty track rather than stretching three cards across a
row. The engraved monogram was also nearly invisible, because a 1 px highlight disappears under a 120 px letter: the cut is
now set in `em` so it scales with whatever it is cutting, and the chisel marks run under the letters instead of being knocked
out behind them.

**Round three** found two real defects and one absence. A mobile rule written for the masthead's search box was unscoped, so
`order: 4` moved the page's own search box to the bottom of its panel on every phone; it is now scoped to the masthead. An
inscription the gateway could not draw fell back to two carved initials taken from the words "Inscription 0000", which mean
nothing; it now carries the inscription's own id in mono. And the chart's height meant nothing without a top, so the peak is
now drawn as one recessive dashed rule with its value beside it.

## 8. Vaults: the word the reader sees

**Amended by §11.3.** The word is unchanged and the rename below stands. What no longer holds is the *section*: everything
in the catalogue now has a vault, so the heading "No vault yet", the fold that says none of these can be chosen, and the
provenance line that opens "None of these has a vault" are all describing a distinction that has stopped existing. §11.3
replaces them with one list carrying each card's own vault status.

A reader looked at the front page and said they could not tell the difference between what was listed and what was merely in
the catalogue. That was the page's fault, not theirs. The catalogue strip sat under the heading **"Paired with"**, which
asserts the opposite of the truth: those forty collections and nineteen runes are paired with nothing. The correction was in
the paragraph below it, in small grey type, which is not where a correction works.

The fix is a noun, and it is the user's own: **vault**. A vault is the thing that exists on chain. One per collection or rune,
holding the ETH that coins paired with it have sent, able to buy that one asset and nothing else. A collection either has a
vault or it does not, and no amount of artwork can blur that.

**Rename the language, never the code.** The registry's `Listing` struct, `listingId`, the indexer's `/v1/listings` and
`indexer/types.ts` are the contracts' and the API's own words. Other agents and the deployed indexer depend on them and they
do not change. What changed is every string a person reads.

| On screen | In the code and on the wire |
|---|---|
| Vaults, a vault, "the vault this coin's fees pay into" | `Listing`, `listingId`, `/v1/listings`, `ListingSummary` |
| "no vault yet" | a catalogue entry, which has no on-chain identity at all |

`#/vaults` is canonical and `#/listings` still answers, then replaces itself with the canonical URL, because the deployed site
and the docs link to the old name and nothing may 404.

**Where the distinction is now made, in order of how hard it is to miss.** The front page shows vaults first, with their
balances and the coins paying into them, and the catalogue immediately after under the heading **"No vault yet"**, so the two
sit side by side and the contrast is the point. The strip no longer mixes them: it carries the catalogue only. Every catalogue
plate is stamped "no vault yet" and every vault plate "vault", so a card lifted out of its section still tells the truth. Each
catalogue card carries one line saying what would have to happen: the registry's owner creates a vault, and until then no coin
can pay into it. In the Launch wizard a creator picks a vault, and the catalogue sits behind a fold that says in its own
heading that nothing in it has one. In the search dropdown every hit wears "vault" or "no vault". The dated, credited
provenance line belongs to the catalogue and sits with it, not between the two.

There is no way to request a vault, so the site says that rather than offering a button that does nothing.

## 9. A coin's logo, now that the indexer has it

Phase 10 added `CoinSummary.logoUrl`, the string the creator gave at launch, byte for byte, naming no gateway because the
gateway is the site's choice. That choice is `lib/plate.ts`: `ipfs://` resolves through a public gateway, `https://` and
`data:image/` are already fetchable, and everything else is refused so the coin keeps its monogram. It is a stranger's string
and only ever becomes an image's `src`. Section 5's request is therefore closed.

## 10. The market: charts, tapes, and one feed for the whole site

The chain has had activity all along and the site showed none of it. It does now.

**A coin's chart** is the page: full width, with the ticket beside it. It draws the indexer's own candles at three
resolutions, and opens on the one the coin's age can fill, because an hourly candle on something twenty minutes old is a
single box. Direction is shown the way candlesticks have always shown it, **by fill rather than by hue** — a hollow body
rose over its bucket, a filled one fell. That keeps the page's single accent free for the three things that are not the
price, and it survives colour blindness and a printer. Under the candles is a rug of the individual trades, buys above the
line and sells below, and under that the volume. Hovering reads the bucket and names the trade nearest it.

**Graduation is drawn**, as a marked line at the second the indexer recorded it, between the last curve trade and the first
pool trade. Nobody else draws it. On staging it appears on the coins that have crossed.

**Price is what the indexer says it is**: the pair asset for one whole coin, taken curve-side *before* Pons's fee and the
creator tax. The footnote under every chart says so, and says why: drawing what each trader actually paid would put a
permanent zig-zag the width of the fee into the line, which would say more about the fee than about the market. Nothing is
interpolated anywhere — an empty bucket is absent, and a coin that never traded is said in words.

**Two sentences were wrong and are now right.** Explore and Analytics both said no price is read anywhere in this system.
A price is read. Both now say where it comes from and what it is not: it is a price in the pair asset, not a valuation, and
there is still no market capitalisation here because no supply read and no worth for a Bitcoin asset are part of this.

### What the indexer should add next

The site-wide feed on the front page is assembled in the browser out of `/v1/coins`, `/v1/acquisitions` and `/v1/draws`.
It works, and it is honest about its hole, but the hole is real:

| Missing | Why it matters | What would close it |
|---|---|---|
| **Buys and sells site-wide** | trades are served per coin, so the one feed that is meant to show the whole system is the one place they cannot appear | `GET /v1/trades` across every coin, newest first, same cursor as the per-coin one — or a merged `GET /v1/activity` |
| **A delivery's own time** | `Acquisition.delivery` carries no timestamp, so a delivery is placed at its purchase's second and labelled rather than dated | `delivery.deliveredAt` |
| **Harvests before the last one** | `CoinSummary.lastHarvestAt` gives one sweep per coin, so the feed shows the latest and never the eleven before it | `GET /v1/harvests`, or harvests inside `/v1/activity` |
| **A coin's state in the list** | Explore cannot sort or filter by "on the curve" against "graduated" without a call per coin, so it does not pretend to | `graduated` and `graduatedAt` on `CoinSummary` |
| **Candles in bulk** | each card fetches its own as it nears the viewport; sixty coins is sixty requests | `GET /v1/candles?coins=a,b,c&res=5m`, or `lastPrice` and `change24h` on `CoinSummary` |

Until then: the feed says in its own words that buys and sells live on each coin's page, the cards fetch their line only
once they are near the viewport, and nothing anywhere is guessed to fill a gap.

### The ticket

Every figure under the amount comes from simulating the exact call the button will send, so the quote is the curve's own
answer rather than our arithmetic about it: what you receive, the least after slippage, the price it fills at, the impact
of the order itself, and Pons's fee and the creator tax broken out. What cannot be simulated is not estimated. The bar
below it is how near the curve is to graduating, read from the chain at that moment, which is specific to Pons and which
nobody shows well. A control that cannot be pressed says why on itself: no wallet, the wrong network, more than the wallet
holds, or that this is not staging and a Pons coin is traded on Pons.

## 11. This week's volume, and the fifty the catalogue was always supposed to be

Written before the code. A reader looked at the vault list and said the collections and runes in it were supposed to be
the ones with the top fifty volume, not just any. They were right, and the catalogue was not that, in three separate
ways at once — none of which a screen would have shown, because an order looks like a ranking whatever produced it.

**What it actually did.** `web/scripts/catalogue.mjs` asked UniSat for the seven-day statistics and then threw the
ranking away:

| Where | What it did | Why that is wrong |
|---|---|---|
| the keyed collections call | sent `timeType: "day7"`, then sorted the rows by `totalVolume` | it fetched this week and ranked by all time |
| the keyless collections call | sorted by `volume_all_time`, kept 40 | ranks the whole history of a market; `volume_day` is `null` on every row, so the index cannot rank a week at all |
| `WANT_COLLECTIONS` | defaulted to 40 | the promise was fifty |
| the runes | came from a **seed list** of thirty well-known names held in the script, described by `ordinals.com` | not a ranking by anything; the committed runes carry no volume field of any kind |

Reading UniSat's own OpenAPI document while fixing it turned up two more faults in the same call, both of which mean
the keyed path had **never** returned a seven-day figure even when it was asked for one. `collection_statistic_list`
requires the window inside `filter` (`filter.timeType`) and forbids unknown top-level keys, so a `timeType` sent beside
`start` was not the window being requested; and its `limit` is capped **below 21**, so a request for fifty was refused
outright. Its volume field is `btcValue`, described as the transaction volume, and it publishes **no lifetime figure at
all** — so on that path `volumeAllTimeSats` is now null rather than a copy of the week's, which is the honest shape.

**What the order is now.** The top fifty collections and the top fifty runes by the volume traded in the seven days
before the fetch. The field is renamed so it cannot be misread: `volume7dSats` says its window in its name,
`volumeDaySats` is gone (it was null keyless and held the seven-day figure keyed, so it was misleading either way), and
`volumeAllTimeSats` is kept where a source publishes one and is **never sorted by**. Where each list's week comes from
is **not** the same for both, and section 11.1 is why.

**Runes can be ranked, and they now are.** UniSat has a runes-market sibling of the collection call:
`POST /v3/market/runes/auction/runes_types`, which takes the same `timeType` and answers `tick`, `btcVolume` — the
volume inside that window — `curPrice`, `cap`, `holders` and `transactions`. It answers no rune id, divisibility, symbol
or supply, so `ordinals.com` is still called once per rune and is now called **only** for those facts: a vault commits
to a rune ID, and the reference indexer is the only place an exact one comes from. A tick ord cannot resolve under the
ticker UniSat gave is dropped rather than guessed, and the count dropped is written into the file's `notes`.

If that call ever cannot be read, the seed list comes back as a **fallback and is labelled one**: the file's `notes`
open with `RUNES ARE A CURATED LIST, NOT A RANKING`, `ranking.runes.field` is `null`, and the site prints "*n* runes,
which are a curated list this script holds, in no meaningful order" in place of "the top *n* runes by...". The failure
is visible on the screen rather than in a log nobody reads.

**A keyless run refuses.** Neither list's week can be had without the key, so ranking by this week is only possible
with one. Without one the script now exits non-zero, writes nothing and touches no picture, and prints what to run instead.
`CATALOGUE_ALLOW_UNRANKED=1` writes `catalogue.unranked.json` and `img-unranked/` — a clearly marked lesser file the
site does not read, whose first note is that it is **not** ranked by this week, and which must never be renamed. Both
are gitignored. A keyed run whose UniSat call fails also refuses, instead of falling back to the keyless index: a silent
fallback producing a lifetime ranking is precisely how the committed file came to claim something it was not.

**The ranking is in the data, not only in this document.** `catalogue.json` now carries `rankedBy`, one sentence, and
`ranking` — the window, the unit, and per kind the basis in words, the field sorted by, the source, how many were asked
for and how many were kept. The site reads that record rather than restating what it hopes is true: `rankingLine()`
turns it into the sentence at the top of the provenance line under every vault and catalogue list, each card prints the
week's figure it was ranked by, and the staging calldata printer prints both the catalogue's basis and which figure the
simulator's weights came from. A file that records no basis — the one committed today, fetched before any of this — says
in those same places that it records none, rather than letting its order be read as a ranking.

**Two things this change could not verify without the key**, and what the script does about each. `btcValue` and
`btcVolume` are assumed to be denominated in sats, as `floorPrice` is: if the largest figure returned is under 10,000
sats the run stops and says the unit is probably BTC, because a BTC figure in a field called `...Sats` would be printed
as sats on every screen. And a window that came back empty would leave an arbitrary order: if not one row carries a
positive `volume7dSats` the run stops and names the field to look at. Neither guard has been exercised against the live
API. The first keyed run is the test, and it will refuse rather than commit something wrong.

### 11.1 What the first keyed run found, and why the two lists get their week differently

The section above was written before the key was available. The operator then ran the fetch with it — the refusal guard
fired first, correctly, and wrote nothing — and the live API contradicted half of what the schema implied.

**The runes are exactly as designed.** `runes_types` with a flat `{ timeType: "day7", start, limit }` answers real rows:
the first carries `tick` UNCOMMON•GOODS, `btcVolume` 97,665,811, `curPrice` 23.8, `holders` 247,348, `cap`
3,653,937,887.6, and echoes `timeType: "day7"` back. So the window is honoured, `btcVolume` is the window's figure, and
it is in sats — the sanity guard's assumption held. It also answers a `runeId` of its own, which is now kept only to be
**checked against ord's**: a ticker the two sources resolve to different runes is dropped and named in the file's
`notes`, because a vault commits to an id and a disputed one must not be offered.

**No UniSat endpoint will state a collection's weekly volume.** Probed live, and then read line by line in their
OpenAPI document:

| Endpoint | Window | Volume | What it does |
|---|---|---|---|
| `collection_statistic_list` | `filter.timeType`, **required**, and nothing else allowed beside it | `btcValue` | answers `{"list":[],"total":0}` for `day7` **and** `day1`. Not a permissions problem: the same key answers runes in the same sequence |
| `collection_statistic` | **none** — takes a `collectionId` and nothing else | `btcValue` | so its "total transaction volume" is a **lifetime** figure, whatever it is used for |
| `collection/auction/list` | none — `filter` takes `nftType` (`brc20`/`collection`/`runes`), `sort` is required and offers only `unitPrice`, `onSaleTime`, `initPrice`, `inscriptionNumber` | **none at all** | the order book: one row per listed item |
| `collection/auction/actions` | none, but every row carries its own `timestamp` | `price`, per fill | listings, cancellations and **sales**, filterable by `event: "Sold"` |

So the collections' week is **summed rather than asked for**: page `collection/auction/actions` backwards with
`filter: { nftType: "collection", event: "Sold" }`, keep the `price` of every fill inside the cutoff, and add them up per
`collectionId`. That is a better provenance than an aggregate would have been, because the file can say exactly what was
summed — `trades7d` is how many fills each figure is made of, and `ranking.collections.from`/`to` are the span actually
covered. The fills also carry `inscriptionId` and `collectionItemName`, so the ids a card draws and a membership root is
built over come from the same rows that ranked the collection, and the extra per-collection order-book read is only made
when the fills named none.

**A span that is not seven days is labelled as the span it is.** The feed is walked until a fill older than the cutoff
proves the whole window was seen. If the page cap (`CATALOGUE_ACTION_PAGES`, sixty pages of ninety-nine) is reached
first, the figure covers fewer days, and `ranking.collections.window` becomes `"5d"` or whatever it truly was, the
`said` sentence names the shortfall, a note records it, and the top-level `rankedBy` splits into two clauses instead of
one. Two lists on two windows is allowed; one window printed as another is not, and that is the whole rule.

**If the fills answer nothing, the run still refuses by default.** No sale in seven days means there is nothing to rank
collections by, and an order over nothing is not a ranking. The operator can then take a deliberate, labelled
degradation — `CATALOGUE_COLLECTIONS_LIFETIME=1` alongside the key — which orders the collections by the keyless index's
lifetime volume while the runes stay on this week, and writes it into the file's first note in those words: *two lists,
two windows, each said out loud*. It is never the default, because an unlabelled version of exactly that is the defect
this section exists to remove.

**One thing was added that was not asked for, because the alternative was a broken file.** The keyed collections
response carries no inscription id, and a collection with no inscription id is skipped by `plannedListings` — so the
first keyed fetch would have produced a catalogue whose every collection quietly dropped out of the vault list. The
script therefore also reads `POST /v3/market/collection/auction/list` per kept collection for its `inscriptionId`s,
which is the keyed equivalent of the Ordinals Wallet call it already made keyless, and which is what the cards draw and
what a membership root is built over. It is a read like every other call here.

**Terms.** Unchanged and still the constraint: **commercial use of UniSat's API requires written permission under §3.1
of their developer terms; we have asked and have not received it.** This change makes the catalogue depend on them
*more* heavily, not less — both lists are now ranked by their figures, the collection floors are theirs, and the
inscription ids behind the artwork are theirs — so the open risk is larger than it was: if permission is refused, the
catalogue loses its ranking as well as its floors, and the honest fallback is the labelled curated list rather than a
lifetime ranking wearing this week's name. Attribution is unchanged: UniSat and Ordinals Wallet are credited on every
screen their data or pictures appear on, and `attribution[].used` now says which of them the ranking came from. The
script still never runs in CI, never runs from the site, keeps its 260 ms gap between calls, and calls no endpoint that
places, changes or cancels an order.

**How this could mislead, added to section 6.** A top fifty is a top fifty *of one week*. A week later it is a
different fifty, while a vault, once the registry's owner creates one, is permanent — so the catalogue's membership is a
snapshot of what was trading, not a standing judgement about what deserves a vault, and the fetch date beside the
ranking is what says so.

### 11.2 Magic Eden was asked for the collections' week, and its Bitcoin API is not answering at all

Written before the code. The first keyed run settled §11.1's design and produced a real answer that is unusable: in the
seven days before it, **four** collections traded on UniSat's collection market, across **seventy** fills in total —
Runestone 1,761,247 sats over 17, Bitmap 550,214 over 51, Neo Yauto 14,900 over 1, BTC Survivors 9,700 over 1. That is
not a top fifty and it is not a market; UniSat's *inscription* market is effectively dead. Its **runes** market is
healthy, `runes_types` works exactly as §11.1 describes, and the fifty-rune weekly ranking stays. Ordinals collection
trading moved to Magic Eden, so Magic Eden is where the collections' week was asked for next. The committed catalogue
was restored rather than ship four collections.

**Magic Eden's Bitcoin API did not answer a single live read.** Every route under `api-mainnet.magiceden.dev/v2/ord/btc`
answers `503 "no healthy upstream"`, inscriptions and runes alike, sustained over twenty minutes of patient probing with
growing backoff. The 503 is not this project being rate-limited and it is not a route that has been withdrawn:

| Probed | Answer | What it proves |
|---|---|---|
| `/v2/ord/btc/stat?collectionSymbol=bitcoin-frogs` | **200**, `cf-cache-status: STALE` | the only success ever seen, and it is not a read — see below |
| `/v2/ord/btc/stat?collectionSymbol=nodemonkes` | 503, `cf-cache-status: BYPASS` | the *same route*, so the route is not the problem |
| `/v2/ord/btc/tokens`, `/activities`, `/collections`, `/popular_collections`, `/collection_stats/search` | 503, `BYPASS` | the whole inscription tree |
| `/v2/ord/btc/runes/collection_stats/search`, `/runes/market/{rune}/info` | 503, `BYPASS` | and the whole rune tree with it |
| `/v2/collections` (Solana, same host) | **400** with a real validation body, `x-ratelimit-remaining: 179` of 180 | the gateway is healthy and nothing here is throttled |

**The one 200 is a cache ghost, not a figure.** `bitcoin-frogs` answered `totalVolume` 151,762,001,352, `owners` 5,143,
`supply` 10,000, `floorPrice` 480,000 — every time, byte for byte, under the *same* `x-kong-request-id`
`e947822a…7964d40`, with `cache-control: max-age=5` and `cf-cache-status: STALE`. Cloudflare is replaying one frozen
copy because the origin behind it is unreachable. Appending a harmless `&_=1` to that exact URL, or upper-casing the
symbol, turns the 200 into a 503 `BYPASS` — the cache entry is all there is. **Its age is unknown and unknowable from
here**, which is the point: a figure a marketplace has not actually served is not a figure this catalogue may rank by,
however plausible it looks and whatever HTTP status carries it. That is the §11 defect wearing a 200.

**The unit could not be established, which matters more than it looks.** The whole file is denominated in sats, and
§11's sanity guard exists because a BTC figure written into a field called `...Sats` would be printed as sats on every
screen. The ghost's numbers do *read* as sats — `floorPrice` 480,000 is 0.0048 BTC, a believable Bitcoin Frogs floor,
where a BTC reading would be absurd — but that is one frozen body for one collection, and the field that would actually
be ranked by, a volume over a window, was never seen at all. So the unit is **assumed, not confirmed**, and confirming
it against a floor that can be checked is the first thing an adapter must do rather than the last.

**A seven-day window is documented, and the host that served it is gone.** Their OpenAPI (Ordinals API 2.1.3, last
updated 2025-09-12) does describe exactly what was wanted — `GET /collection_stats/search/bitcoin`, `window` an enum of
`10m/1h/6h/1d/7d/30d`, `sort` including `volume` and `totalVolume`, `direction`, `offset`, `limit` to 1,000 — but it
pins that one path to a **different server**, `stats-mainnet.magiceden.dev`. That hostname **has no DNS record at all**.
The surviving host answers the same path, and refuses the chain: `GET api-mainnet.magiceden.dev/collection_stats/search/bitcoin`
returns `400 {"errors":[{"value":"bitcoin","msg":"Only solana is supported","param":"chain"}]}`. That is a live origin
declining to serve Bitcoin, not an outage — the two facts together are why this is not worth waiting out on the day.

**So neither branch §11 planned for is reachable.** There is no weekly window to rank by, and there is no lifetime
figure either — the only lifetime number obtainable is the one cache ghost above, for one hard-coded collection, of
unknown age. A "two bases, honestly stated" file needs a second basis that was actually read, and there is none. **The
catalogue is therefore not re-ranked from Magic Eden, and nothing about the committed file changes.** Writing a Magic
Eden adapter against field names taken from a schema rather than from a response would commit precisely the kind of
unverified claim §11 exists to remove, so none is written: not one field name below was observed in a live body.

**What is written instead** is `web/scripts/magiceden.mjs`, a read-only prober that asks the one question worth
re-asking — *can Magic Eden rank the collections' week yet?* — and answers `yes`, `no`, or `not today`. It calls only
collection and market reads, never a route that places, changes or cancels an order or a listing. It carries its own
pacing rather than borrowing UniSat's: `MAGICEDEN_GAP_MS` is **2,000 ms**, not the 260 ms of `CATALOGUE_GAP_MS`, and a
503 backs off from twenty seconds and doubles to a five-minute ceiling, because their gateway stays unhappy for minutes
once it is unhappy at all. And it refuses to report a success it did not witness: a 200 whose `cf-cache-status` says
`STALE` or `EXPIRED` is reported as the cache ghost it is, never as a live read. Until it says yes, the collections have
no weekly source and the file says so rather than inventing one.

**Terms, now that there are two marketplaces.** Magic Eden's position is materially lighter than UniSat's and is
published rather than negotiated: their Ordinals API overview says they offer the endpoints "in the spirit of fostering
composability and openness", makes the API "available free of charge", asks that users "adhere to our Terms of Service",
and "kindly request that you attribute Magic Eden on your website or within your app", with a link to the marketplace
where NFTs are shown. It notes the API is rate-limited and that some endpoints are gated by an API key. There is **no
written-permission requirement** of the kind UniSat's §3.1 imposes — so the open question here is narrower than
UniSat's, but it is not closed: nothing has been read of their Terms of Service itself beyond what that page states, so
**whether their ToS constrains automated commercial reads is recorded as open.** UniSat's §3.1 permission is still
asked for and still unanswered, unchanged by any of this. Attribution is unchanged and now names three: UniSat, Ordinals
Wallet and ord are credited where their data or pictures appear, and Magic Eden is credited the moment any figure of
theirs does — which today is nowhere, because none was taken.

**What would reopen this.** `node scripts/magiceden.mjs` answering yes, which needs either the `/v2/ord/btc` upstream to
come back or `collection_stats/search/bitcoin` to accept the chain again on a host that resolves. Either way the next
step is the same as §11.1's: read the real response, take the field names from it, and only then write the adapter.

### 11.3 Fifty of each, on two bases that are printed apart, and a catalogue no longer defined by the absence of a vault

Written before the code. §11 promised the top fifty of each kind. §11.1 delivered it for the runes and designed a way to
sum it for the collections. §11.2 ran that sum and found there is no collection market left on UniSat to sum — **four**
collections over **seventy** fills in a week — and then found Magic Eden's whole Bitcoin API answering nothing live. So
the catalogue that is committed today is still the one §11 was written to replace: forty collections ordered by lifetime
volume, nineteen runes in no order at all, and no `ranking` block, which is why every screen correctly says the file
records no basis.

Two separate things are now wrong with that, and they are wrong in opposite directions.

**The refusal has outlived its reason.** §11 made a keyed run refuse rather than write a lifetime order, and §11.1 kept
that refusal for the case where the fills answer nothing, behind `CATALOGUE_COLLECTIONS_LIFETIME=1`. That was right when
a collection's week looked obtainable and a lifetime order would have been a silent substitution for one. It is now
simply a blocker: there is no weekly figure for a Bitcoin collection anywhere — not from the endpoint that takes a
window, not from the one that takes a collection id, not from a marketplace whose gateway will not answer — and a script
that refuses to write anything until an impossible figure arrives writes nothing for ever. The defect §11 removed was
never *lifetime volume*; it was lifetime volume **printed as this week**. So the lifetime path stops being an opt-in
flag the operator has to remember and becomes what the collections do **when no weekly source answers**, which is today
and every day until one does.

**And the site still describes a distinction that has stopped existing.** §8 gave the reader the word *vault* and put
the catalogue under the heading **"No vault yet"**, because at the time nothing in it had one. On staging there are now
**sixty vaults against fifty-nine catalogue entries**: every collection and every rune in the file has a vault, and the
two demo listings make up the difference. So `VaultChooser`'s fold reads "Collections and runes with no vault" over a
section that says in bold "None of these has a vault, so none can be chosen here" — an empty box asserting something
false, in front of a hundred things that can all be chosen. The intended state on mainnet is exactly the same: the
catalogue **is** the fifty of each that get vaults. A section defined by the absence of a vault has nothing to hold.

**What each list is ranked by, and how the file says it.** Two bases, two sentences, never one:

| List | Basis | Where it comes from | What the file says |
|---|---|---|---|
| **runes** | the volume traded in the seven days before the fetch | `runes_types`, `timeType: "day7"`, `btcVolume`, read not computed | `ranking.runes.field: "volume7dSats"`, `window: "day7"` |
| **collections** | **lifetime** volume | Ordinals Wallet's keyless index, `volume_all_time` | `ranking.collections.field: "volumeAllTimeSats"`, `window: null`, and a `said` that states in plain words that it is the whole history of a market, **not** this week, and that no weekly figure for a collection exists anywhere to rank them by |

`rankedBy` therefore splits into its two clauses, as §11.1 already allows it to, and `rankingLine()` prints both: *the
top fifty collections by lifetime volume … , and the top fifty runes by UniSat's volume over the seven days before this
file was fetched.* A reader who reads only the sentence still cannot mistake one window for the other, which is the
whole rule and the only rule.

**The week is still asked for, and it is asked for first.** A keyed run walks UniSat's fills exactly as §11.1 describes
before it does anything else, because the day that market comes back is the day the collections should be on a week
again and nobody should have to notice. The week is taken **only when it can actually fill the fifty**: a window that
names four collections is not a top fifty of anything, and half a list of this week beside half a list of all time
would be the §11 defect with extra steps. When it cannot, the run falls to lifetime, and the reason it fell is written
into the file's notes in the fetch's own words — how many collections traded, over how many fills, in how many days —
so the next reader can see what the market did rather than being told what it could not do.

**What still refuses.** The guards are not relaxed; they are extended to cover the path that now runs. A ranking refuses
and the run exits non-zero when the rows are empty, when not one row carries a positive figure in the field being sorted
by, or when the largest figure is too small to be denominated in sats — the last because a BTC figure written into a
field whose name ends `Sats` would be printed as sats on every screen. Until today those three guards only ever
protected the weekly path; the lifetime path had none, because it was only ever allowed to write the lesser file. Now
that it writes the committed one it gets all three. A keyless run still refuses to touch `catalogue.json` at all, still
writes only `catalogue.unranked.json` and `img-unranked/` under `CATALOGUE_ALLOW_UNRANKED=1`, and that file still says
in its first note that it is not ranked by this week: without a key the runes have no ranking either, and a file with no
ranked list in it is not this catalogue. `CATALOGUE_COLLECTIONS_LIFETIME=1` survives with a narrower meaning — *do not
even ask* — which skips the sixty-page walk when the operator already knows the answer.

**Fifty runes, not fifty tickers.** §11.1 ranks the runes and then asks ord to describe each one, and a ticker ord
cannot resolve, or resolves to a different rune id than UniSat named, is dropped — rightly, because a vault commits to
an id. Dropping from a list of exactly fifty leaves fewer than fifty. So the fetch ranks a **pool** wider than fifty,
walks it in ranked order, and stops at fifty described; the list is then the fifty most-traded runes ord could resolve,
and a note says how many above the fiftieth had to be reached for and why each was passed over. Nothing about the order
changes: it is still descending `btcVolume` over `day7`, and nothing below a rune that was kept is ever promoted above
it.

**What this does to staging.** The plan (`staging/catalogue/listings.ts`) is already pure — a parsed catalogue in,
planned listings out — and the creator (`staging/bootstrap/listings.ts`) is already idempotent, so a hundred-entry file
needs no code to be listed. What it does need is for the tests to stop knowing how many entries there are, which is most
of the way done: `staging/test/catalogue.test.ts` derives its counts from the file already, and the remaining pins are
in `keeper/test/staging-listings.test.ts`, which names `pixel-pepes` and `840000:3` — two entries a fresh fetch may
simply not contain. They become "the first collection that carries an inscription id" and "the first rune", which is
what they were always standing for.

**The chain adds; it never removes.** The creator matches what the registry already holds by kind, ref and name, and
creates only what is missing, so a rebuilt catalogue does not replace the sixty vaults on the staging chain — it adds to
them. An entry that was in the old fifty and is not in the new one keeps its vault, and that vault keeps its ETH and its
coins. That is the correct behaviour for a chain (a vault is permanent: §11.1's own note) and it is the wrong shape for
a *demonstration of the top fifty*, so the operator gets the choice stated rather than made for them: keep the union and
let the manifest's `fromCatalogue` say which vaults the current file still names, or wipe the staging volume and let the
chain be built once from the new file. Nothing in the code decides that.

**The catalogue section, reworked.** One list, not a list plus a fenced-off bucket. Every collection and every rune in
the catalogue is a card, and **the card carries its own vault status**: a card whose vault exists is pickable and says
so, a card whose vault does not yet exist says *that*, on itself, and cannot be picked. When every entry has a vault —
today's case on staging and the intended case on mainnet — the reader sees a single clean list of collections and runes
with no explanatory apology attached to it, because there is nothing left to explain. The headings follow: **"No vault
yet"** on the front page and on `#/vaults` becomes **"Collections and runes"**, the search dropdown stops telling a
reader that *n* collections and runes have no vault yet when none of them is without one, and the provenance line stops
opening with "None of these has a vault."

**What is kept, because it is still true.** The word is still *vault*, and §8's rename stands entirely: the registry's
`Listing`, `listingId`, `/v1/listings` and `ListingSummary` are untouched, and what changed is a heading and a card, not
a contract. A coin's choice is still permanent and the wizard still says so at the moment of choosing. A **paused** vault
still cannot be chosen, and its card says which of the two it is rather than looking like an entry with no vault at all.
Every catalogue card still carries the figure its list was ranked by, and the dated, credited provenance line still sits
under the list. There is still no way to request a vault, and the site still says so rather than offering a button that
does nothing.

**How this could mislead, added to section 6 and to §11.1's note.** The collections are now ordered by a figure that
covers years, which means the list is stable in a way a weekly one would not be, and a reader could take that stability
for a judgement about what deserves a vault. It is not one: it is the only figure any source will still state. The
runes' fifty, by contrast, is a week old the moment it is fetched and will be a different fifty next week. Two lists,
two windows, each said out loud, on every screen that prints either.
