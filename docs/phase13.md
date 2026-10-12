# Phase 13 — who traded and who holds: one panel under the chart

2026-09-21.

Phase 12 put a readable chart on the coin page. Underneath it the page has a "Trades" section that, in the demo the site
actually serves, prints an apology instead of a tape — and nowhere on the site, or in the indexer, is there any answer to the
first question a reader of a launchpad asks after the chart: **who holds this.** This phase indexes holders, records both lists
into the fixtures, and puts the two lists in one panel with two tabs, in the place every site the reader is comparing us
against puts them.

## 1. Three faults, and none of them is cosmetic

**The demo shows no trades, on purpose, and the purpose was wrong.** `indexer/scripts/write-fixtures.ts` records the chart but
not `/v1/coins/:address/trades`. The reasoning was written down: the trades endpoint pages by cursor, and recording only page
one would leave "Show more" reaching for a cursor nobody recorded. That is a real problem with a cheap answer — record the
cursor pages too — and the tradeoff was taken the wrong way round. What it bought was a coin page whose tape says *"This
recorded demo keeps each coin's chart, not its tape"*. What it cost is that the person who opened the site saw a market with no
trades in it.

**"Show more" never worked on a live indexer either.** The API's parameter is `?before=block:logIndex` (`indexer/src/api.ts`).
The site asks for `?cursor=…` (`web/src/lib/coinchart.ts`). An unknown query parameter is ignored, so the second page *is page
one*: `useInfiniteQuery` appends the same twenty-five trades again and offers "Older trades" for ever. Nobody noticed because
the demo never got as far as the button.

**Holders are not tracked at all.** `Transfer` appears in no ABI, no fold and no table. The indexer knows every trade in a coin
and nothing about who ended up with the coins. A holder count and a top-holder list are not derivable from trades: a transfer
is not a trade, and a coin can change hands without either venue seeing it.

## 2. A balance is a sum of signed log deltas, and nothing else

The rule this phase does not bend: **balances are derived from events alone**, so a reindex reproduces them. Every Pons coin is
an ERC-20 and emits `Transfer(address indexed from, address indexed to, uint256 value)` for every movement including the mint.
The fold is one line of arithmetic per log — subtract from `from`, add to `to` — and the two special cases are the rule's own
edges:

- `from` = the zero address is a **mint**: nothing is subtracted, because the zero address is never a holder.
- `to` = the zero address is a **burn**: nothing is added, for the same reason.
- a zero-value transfer (ERC-20 permits it) moves nothing and is folded as nothing.

No `balanceOf` is ever called. A balance read from the chain would be a number the events cannot explain, and the first
reindex would disagree with it.

**One ordering fact forced a change to the fold.** Measured on staging, in a coin's own launch block:

| logIndex | contract | event |
| --- | --- | --- |
| 0 | the coin | `Transfer` 0x0 → the curve, `1e27` — the whole supply, minted |
| 1–6 | the curve, Pons | the curve's own setup |
| 7 | our Launcher | `Launched(coin, creator, listingId, curve, router)` |

The mint arrives **seven logs before the event that tells us the coin is ours.** A fold that applies logs in strict chain order
therefore meets the mint of a coin it has never heard of and must drop it — which is the same rule the curve and the router
already follow, and would leave the curve's balance short by the whole supply for ever. So the fold now applies, within a
block, the two **identity** events first — `Listed` and `Launched` — and then everything else in log order. `rebuild()` orders
the same way, so the invariant that matters is kept: *the same events give the same tables.* The follower's own comment already
said identity had to come first for `lotOf`; this makes it true of the fold as well.

## 3. The plumbing is not a holder

A coin's largest balance is almost always a contract. Measured across the 77 coins on staging on 2026-09-21:

| address | what it is | how many coins | its share of supply |
| --- | --- | --- | --- |
| the coin's own **curve** | Pons's bonding curve, one per coin; it is minted the whole supply and sells it | every coin | 0.00% – **57.30%** |
| `0x7eD5…EC7e` | the **Pons factory** | 64 of 77 | **28.5714%** exactly, in every one |
| `0x8366…0951` | the v4 **PoolManager** | 0 (no coin has graduated on our fork) | — |
| the coin's **router** | ours; it forwards fees, it does not trade | 0 | — |
| the coin **itself** | a coin holding itself | 0 | — |

28.5714% is 2/7: Pons reserves it for the pool a graduated coin gets. It is not somebody's position. A reader told that this
coin has 12 holders, two of whom hold 57% and 29%, has been told something false about its distribution — the number that
matters is that it has **10** holders and the largest real one holds 15.2%.

So the count and the top list **exclude** the zero address, the coin, its curve, its router, the Pons factory, the v4
PoolManager, and the hook named in its pool's own `Initialize` log. Each of those is either ours, Pons's, or Uniswap's, and
each is learnt from something already indexed — the `coins` table, the `pools` table, or the factory address in the deployment
file — so no read is added to find them. The excluded addresses are **served, with their balances and the reason each is
excluded**, on the same response: hiding them would make the remaining shares not add up, and a reader who wants to know where
the other 86% of a young coin is deserves the answer.

Percentages are of `totalSupply`, which Phase 12 already reads once per coin into `coin_meta`. A coin whose supply has not
read yet gets `share: null` on every holder, and every screen prints *"share of supply is not known: the supply has not been
read"*. There is no branch in which a percentage becomes `NaN`, and none in which a denominator is guessed.

## 4. The cost, stated in calls

The follower already asks a node for every log of every vault, router and curve it knows, batched 200 addresses to a call.
Coins join that list. Measured on staging:

| | before | after |
| --- | --- | --- |
| addresses in the dynamic `eth_getLogs` | 60 vaults + 77 routers + 77 curves = 214 → **2 calls** | + 77 coins = 291 → **2 calls** |
| `eth_call` per holder, or per pass | 0 | **0** |
| logs a live pass carries | the range's own | + its `Transfer`s (4,684 over the chain's whole 8,850-block life) |

**History is fetched once, not every pass.** `Transfer` indexing starts wherever the follower's cursor happens to be, so every
coin already on the chain would otherwise have a balance made of the tail of its history. A **bounded backfill** closes that:
two block ranges a pass, oldest first, from the deployment's `fromBlock` towards the block the live indexing began at, its
progress in `meta` so a restart resumes rather than starts again. On staging that is 5 ranges of 2,000 blocks — three passes —
and then it is over for ever. `meta.transfers_backfilled_through >= meta.transfers_live_from` is the whole test of whether
balances are complete, and until it holds:

- `/v1/coins/:address/holders` answers with `complete: false` and `scannedThrough`, and the panel says the list is still being
  built rather than showing a number it cannot stand behind;
- `CoinSummary.holders` is **`null`** — not known, in the API's own convention, rather than a count that is merely the recent
  part of the truth.

A backfilled log is applied without its block's timestamp unless the `blocks` table already holds it. Balances need no clock,
and fetching a block per historical transfer would cost more than the balances are worth.

## 5. The endpoint, and the count on the summary

```
GET /v1/coins/:address/holders?limit=&offset=
```

```json
{ "coin": "0x458d…5097", "holders": 10, "supplyWei": "1000000000000000000000000000",
  "complete": true, "scannedThrough": 69023770, "transfersFrom": 69014768,
  "items": [{ "rank": 1, "address": "0x5f0c…5b9f", "balanceWei": "152061129736034564016599375", "share": 0.15206112, "isCreator": false }],
  "excluded": [{ "address": "0x179D…BEcb", "why": "curve", "balanceWei": "572980338266492564151136396" }],
  "total": 10, "limit": 25, "offset": 0 }
```

`rank` is over the whole list, not the page, so page two starts at 26. `share` is a **fraction** of supply (0.15206112 is
15.2%), or `null` when the supply is unread. `holders` is the count after exclusions and is repeated on `CoinSummary`, so a
card can show it without a second request. A coin nobody has ever held — every coin, in the seconds between its launch and its
first buy — is `holders: 0` with an empty list, which is a fact and not an error.

## 6. The fixtures: cursor pages recorded to exhaustion

The writer now records `/v1/coins/:address/trades?limit=25`, then follows `next` and records each cursor page under the exact
path the site asks for it by, until `next` is null. Measured on staging: 77 coins, a median of **67** trades each, 49 coins
above 25, 6 above 100 — so three pages of twenty-five is typical and ten is past the worst of them. The cap is **10 pages**
(250 trades); it is not reached by anything on the chain today, and when a tape does hit it the writer says so on the console
and in `index.json`, and the panel's own "no more recorded" state covers the click.

Holders are recorded the same way, by offset, to exhaustion.

This is the honest half of the choice the last change got wrong: the demo's tape genuinely runs out, the way the live one
does, rather than stopping at a cursor nobody recorded. The placeholder sentence in `Coin.tsx` about the recording keeping
charts and not tapes is deleted, because it stops being true.

The **synthetic** set gets both lists too, from `indexer/test/story.ts`, which already has curve trades and now has transfers —
including a mint and a burn, so the states a demo run on staging never reaches (a coin with a burn; a coin whose supply was
never read, and therefore has no percentage to print) are on screen somewhere.

## 7. The panel

One section directly beneath the chart, two tabs, and the existing "Trades" section folded into it rather than left as a
second tape further down the page.

**Trades**, newest first: time, side as a coloured word (never colour alone), the price, the coin amount, the value in the pair
asset, the same value in dollars when Phase 12's rate is known, a shortened trader address, and the transaction. Paged by the
cursor, which now has the parameter name the API actually reads.

**Holders**: the count above the list; then rank, a shortened address, the balance, its share of supply, and a mark on the
creator's own address. Paged by offset.

Both are readable at 400px: the wide grid collapses to four columns and the table lives in its own `overflow-x: auto` box, so
the page itself never scrolls sideways. Every colour comes from the custom properties in `web/src/styles/etch.css` through
`web/src/lib/theme.ts`, the way Phase 12's chart does, so the panel is the same material as the page in either theme.

**Four states, and they are not the same sentence.** "This coin has never traded" is not "this recording has no trade for this
coin", which is not "the indexer could not be reached", which is not "the holder list is still being built". Each is printed as
itself.

**The explorer.** `basescan.org` was hard-coded into the trade row — Base's explorer, for a chain that is not Base. Measured
2026-09-21: `https://explorer.mainnet.chain.robinhood.com` answers `301` to `https://robinhoodchain.blockscout.com`, so
Robinhood Chain has a Blockscout and the first of those is the durable address for it. A link is drawn **only** on Robinhood
Chain. Our staging fork is a chain we run: it has no explorer, so on staging an address is a monospace string that copies
itself, not a link to nowhere.

## 8. Staging honesty

Every coin, trade and holder on staging is made by a simulator we run (docs/phase11.md §7). The panel must not imply
otherwise, and it does not: on staging it says, above the list, that **these are simulated wallets trading with money that is
not money**, and it says it in the panel rather than relying on a banner elsewhere on the page. The 10 addresses holding
`JASP4SIM` are 10 keys the simulator holds; the nearest thing to a person on that chain is whoever pressed the faucet.

## 9. What this phase does not claim

- **A holder is an address, not a person.** One trader with three wallets is three holders; a custodian holding for ten
  thousand people is one. Nothing here clusters addresses, and no field on the response should be read as a count of people.
- **Nothing is excluded for being large.** The exclusion list is exactly seven kinds of address, each a named contract of ours,
  Pons's or Uniswap's, each published on the response. A whale is a holder.
- **A graduated coin's pool liquidity is not attributed to anyone.** On a public chain a coin's pool position sits with the v4
  PoolManager, which this phase excludes and does not decompose into the positions inside it. Our fork has never graduated a
  coin (docs/phase11.md §8), so this is untested against a real pool and is written down as untested.
- **The dollar column inherits Phase 12's rate**, which is a spot read of one on-chain pool and not a market data feed. Where
  the rate is unknown the column is absent, not zero.

## 10. Amendments

**§7, the phone layout.** The panel does not collapse its tape to four columns. At 400px it drops the two columns a reader
can do without — the venue, and the transaction link, which on staging links nowhere anyway — and keeps time, side, price,
the coin amount, the value and the trader. The remaining table is narrow enough to fit most phones outright and, when it is
not, scrolls inside its own `overflow-x: auto` box, which is the part that matters: the page itself never scrolls sideways.
Dropping price or amount to reach four columns would have taken away the two numbers a trader opened the tab for.

**§5, one field the example left out.** The holders response also carries `creator`, the coin's creator address, beside
`isCreator` on each row. The panel marks the creator's own row, and a page that wants to say "the creator holds none of it"
needs the address even when it is in no row.
