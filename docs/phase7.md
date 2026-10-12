# Phase 7: the site gets the pages a launchpad needs

Written 2026-09-21, before its code. Phase 5 built a site that explains etched. It had one long landing page and four app pages.
Somebody arriving to see what has been launched had nowhere to go. This phase adds the pages a person actually navigates, and
moves the explanation out of the front door into its own room.

The visual language does not change: the carved slab, Tenor Sans over Hanken Grotesk with Plex Mono for addresses, one
desaturated orange, scored hairlines instead of cards. This is an information-architecture change, not a re-skin.

## The page inventory

| Route | What it is for | What it reads |
|---|---|---|
| `#/` | The front door. A short hero, the live counters, then what has been launched and what can be paired with. | `/v1/stats`, `/v1/coins`, `/v1/listings` |
| `#/explore` | Every coin, with its listing and what its fees have done. Filters, sort, a count. | `/v1/coins`, `/v1/listings` |
| `#/receipts` | Every purchase and delivery, both chains, each with a Bitcoin transaction anyone can open. | `/v1/stats`, `/v1/acquisitions` |
| `#/claims` | Connect a wallet: what is waiting on you, and nothing else. | `/v1/draws`, then `/v1/draws/:id` for candidates |
| `#/draws` | Open draws with their challenge windows, resolved draws, rune distributions. | as today, plus `/v1/draws` |
| `#/analytics` | The platform's numbers, each labelled with where it came from. | `/v1/stats`, `/v1/listings`, `/v1/coins` |
| `#/launch` | Four steps and a panel that prices the choice as it is made. | `/v1/listings`, Pons's launch fee from the chain |
| `#/listings` | Every listing: one vault each, its cap, its budget, what it has bought. | `/v1/listings` |
| `#/coin/:address`, `#/listing/:id` | One coin, one listing. Unchanged. | as today |
| `#/docs` | How it works. The explanatory sections that used to be the landing page. | nothing |
| `#/trust` | The full trust model, unchanged. | nothing |
| `#/terms`, `#/risk`, `#/privacy` | Unchanged. | nothing |
| `#/staging` | Unchanged. | the staging node's `/info` |

Navigation: Explore, Receipts, Draws, Claims, Analytics, Launch, Docs, then search, the X link when there is one, and the wallet.
The footer becomes three columns: what you can do, what the protocol is (the deployed addresses), and the legal pages.

## The three states, on every page

Unchanged from Phase 5 and worth restating because this phase adds eight more surfaces to get it wrong on.

1. **Live.** An indexer answered, or the chain did. Figures are shown.
2. **Demo.** `?demo=sample` or `?demo=states` replay recorded answers, `?net=staging` reads our fork. A banner says so on every
   screen, every time.
3. **Nothing live.** No deployment, no indexer. The page says that plainly and shows no figure. It never fills a gap with a zero.

A value the API reports as `null` carries its reason and the page prints the reason. That rule does not bend for a grid cell.

## What the API cannot do, and what is done instead

The indexer's list endpoints take `limit`, `offset` and `listing` (`indexer/src/api.ts`). There is no sort parameter and no
filter beyond a listing id. So:

- **Sorting and filtering happen in the browser**, over one page fetched at `limit=200`. Above 200 rows this becomes wrong, and
  the page says which slice it is sorting rather than pretending it sorted everything. If the catalogue outgrows that, the sort
  belongs in the indexer, not in a bigger fetch.
- **There is no market cap and no 24-hour trading volume anywhere in this system.** Nothing reads a price. The competitor's grid
  leads with both; ours cannot, and inventing them is out of the question. The coin grid sorts on what is really there: when it
  launched, fees routed over its life, fees in the last 24 hours, and its creator tax.
- **`graduated` is not in the coins list.** It is read from the chain per coin (`/v1/coins/:address`). A badge on a grid of
  coins would mean one chain read per row, so the grid does not carry one; the coin's own page says it.
- **There is no per-address endpoint.** Claims fetches the draw list, then the detail of the ones whose state can involve a
  winner, and matches the connected address in the browser. That is a handful of requests today and would need an endpoint if
  draws ran into the hundreds. Said on the page when the list is truncated.

## Launch, as four steps

Coin, then listing, then what happens to what it buys, then review. A panel beside it prices the choice as it is made: the split,
the creator tax, and what 1 ETH of trading sends to the vault at the measured rate of 0.70% plus the tax (`docs/phase1.md`). The
panel shows its inputs and calls itself an estimate, because that is what it is.

The third step is new to the form and was always in the contracts: the creator's one power is choosing what happens to each asset
their coin's fees pay for. Making that a step at launch means a creator meets the decision before they meet it in an email.

## What the staging view shows

Built against the staging indexer (`staging-api-production-1b4b`, chain 9466301) and loaded page by page, every route renders
rows rather than an empty state:

| Page | What it read | Rows |
| --- | --- | --- |
| `#/` | stats, coins, listings | 5 counters, 2 coin tiles, 2 listings in the rail |
| `#/explore` | `/v1/coins?limit=200` | 2 of 2 |
| `#/receipts` | `/v1/acquisitions?limit=200`, stats | 1 purchase, 1 delivery |
| `#/listings` | `/v1/listings` | 2 |
| `#/draws` | `/v1/draws?limit=200` plus each draw's detail | 1 |
| `#/claims` | the same draws, matched in the browser | prompts for a wallet until one is connected |
| `#/analytics` | stats | 4 figure groups and 2 bars |
| `#/launch` | `/v1/listings` and the launch fee | 4 steps, 2 listings offered |
| `#/docs`, `#/trust` | nothing | prose |

## Where the code sits

`web/src/lib/coinlist.ts` holds the sorts, the filters and the search, and imports nothing that needs a browser, so the rules can
be tested directly. `web/src/lib/routes.ts` holds the route inventory once: the masthead's order, every page that exists, and
which section a page sits under. A test asserts that a route in one of them is in all of them, including the page dispatch.

## What stays weak

Sorting a slice is a compromise. Claims costs one request per candidate draw. Neither matters at today's sizes and both are
written down here so that the first person to hit the limit knows it was known.

The coin grid looks sparse when a listing has one or two coins, because a grid with one tile in it is mostly grid. It corrects
itself with a dozen coins and was not worth special-casing before there are any.
