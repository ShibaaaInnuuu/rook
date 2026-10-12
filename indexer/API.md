# The Etch indexer's API

Read-only `GET` over JSON. No auth, no cookies, no PII (addresses are public chain data). `Access-Control-Allow-Origin: *`.
Lists and details: `Cache-Control: public, max-age=5`. Health and every error: `no-store`. Per client address: 120 requests a
minute with a burst of 40, then `429` with `Retry-After`. Anything but `GET`/`HEAD`/`OPTIONS` is `405`.

Types: [`types.ts`](./types.ts) — one file, no imports, made to be copied. Formulas: `docs/phase4.md` §3.
Static copies of every answer: [`fixtures/demo/`](./fixtures/demo) (recorded off the staging fork, chain 9466301: the 24 most
recent coins, each with all five chart ranges, its whole tape of trades and its holder list; the prices are simulated, the
wallets are the simulator's own keys, and the Bitcoin values are FIXTURES) and
[`fixtures/synthetic/`](./fixtures/synthetic) (invented events that reach every state). Each has an `index.json` mapping a request path to its file.

## Conventions
- ETH amounts are **wei as decimal strings**. Rune amounts are raw units as decimal strings. Times are unix seconds.
- `null` means *not known*, never zero. Where a reason exists it is given (`volumeUntilNextBuy.reason`, `live.error`, `curvePendingReason`).
- Fields under `live` are read from the chain when asked (cached 15 s) and are `null` if the node cannot be reached. Everything else is
  from indexed events, `confirmations` blocks behind the head.
- Addresses are accepted in any case and returned checksummed. A Bitcoin txid is hex as explorers print it, without `0x`
  (the contracts store it as `bytes32` in that byte order — a convention of ours, stated here because nothing on chain enforces it).
- Lists take `?limit=` (1–200, default 50), `?offset=`, and where it makes sense `?listing=<id>`. They return `{ items, total, limit, offset }`.

## Endpoints

| Path | Returns |
|---|---|
| `/v1/health` | `Health` — cursor, head, lag, whether a deployment file exists for the chain, last error class, reorg count |
| `/v1/stats` | `Stats` — protocol totals |
| `/v1/coins` | `Page<CoinSummary>` |
| `/v1/coins/:address` | `CoinDetail` — fees, harvests, what it paid for, **`volumeUntilNextBuy`**, `live.pending` |
| `/v1/coins/:address/trades` | `TradePage` — a coin's trades, newest first, `?limit=` and `?before=block:logIndex` |
| `/v1/coins/:address/holders` | `HolderPage` — the count and a page of the largest, folded from the coin's own `Transfer` logs |
| `/v1/coins/:address/candles?res=trade\|5m\|1h` | `Chart` — candles computed here from those trades, with marks and the graduation |
| `/v1/listings` | `Page<ListingSummary>` |
| `/v1/listings/:id` | `ListingDetail` — vault balances (live), cap and expiry, declarations with countdowns, bond, acquisitions, delivery times |
| `/v1/acquisitions` | `Page<Acquisition>` — each with `delivery` and `verify` |
| `/v1/draws` | `Page<DrawSummary>` — raffles and rune distributions |
| `/v1/draws/:listingId-:acquisitionId` | `DrawDetail` — snapshot root, challenge window, the command that recomputes the root, drand round, winner, delivery, timeline |
| `/v1/wrapped` | `WrappedList` — the wrapped collections (wFROG, wAPE, …) this indexer was configured with (`WRAPPED_CONFIG`); `configured: false` when none |
| `/v1/wrapped/:slug` | `WrappedCollection` — floor, supply, items held, coins with their phase (and `canClose`, `canGraduate`, `refundsOpenAt`), activity, open tickets; `launchesOpen`, the registry's keeper switch (`keeper`), and the raffle vault's `draws` (see below) |
| `/v1/wrapped/:slug/tickets/:id` | `Ticket` — one redemption: pinned, resolved (item, deliver-by), delivered (tx) or refunded; `refundableAt` (pinned + 7 days; resolved: deliverBy + 72 h once the custodian attested it undeliverable, else deliverBy + 30 days — R3 §4), `undeliverable` (the `Undeliverable` event) and `refundedUnresolved` (refunded while pinned: `Refunded` with `acquisitionId = type(uint256).max`, no item drawn) |
| `/v1/reserves` | `{ configured, reports: ReserveReport[] }` |
| `/v1/reserves/:slug` | `ReserveReport` — the proof of reserves: status (green / red / unverified), reasons, the invariant's terms, every item with its verdict and the public links that check it, tickets in flight. docs/v5/ux-and-reserves.md §2 |

### `/v1/health`
```json
{
  "ok": true,
  "deployment": true,
  "chainId": 31337,
  "demo": true,
  "cursor": 68860382,
  "head": 68860382,
  "lagBlocks": 0,
  "confirmations": 0,
  "events": 22,
  "lastPassAt": 1790000516,
  "lastError": null,
  "reorgs": 0,
  "version": "1"
}
```
`deployment: false` means there is no `deployments/<chainid>.json` for the node's chain: the follower idles and every list is empty.
It never invents data.

### `/v1/coins/:address` — the part that matters
```json
{
  "address": "0x5707bF6c8d3D812F3fdfbA81482d346BbB63555C",
  "symbol": "STONE",
  "creatorTaxBps": 200,
  "logoUrl": "ipfs://bafkreicuspukrnaisnzred73svouxh3favyf4frtdbjlha36juefmx2ygq",
  "lifetimeFeesWei": "76491000000000000",
  "toVaultWei": "61192800000000000",
  "toTreasuryWei": "15298200000000000",
  "fees24hWei": "76491000000000000",
  "harvests": 1
}
```
`logoUrl` is the logo its creator gave at launch, exactly as the chain has it. It is **not** in any event — Pons emits none
carrying it — so it is decoded from the transaction that launched the coin (docs/phase10.md). Expect `ipfs://` on Robinhood
Chain, `data:image/…` on staging, and `null` when the coin has none or the string could not be an image. **This field names no
gateway on purpose**: resolving an `ipfs://` URI means choosing a third party that will see your visitors, and that is the
caller's decision. A logo is a stranger's string — render it only as the source of an image, never as markup.

`volumeUntilNextBuy`:
```json
{
  "volumeNeededWei": "1792019224005979862",
  "reason": null,
  "ready": false,
  "inputs": {
    "targetWei": "50000000000000000",
    "capWei": "49900415238529165",
    "capExpiry": 1790003689,
    "minDeclarationWei": "50000000000000000",
    "minEntryWei": "10000000000000000",
    "spendableWei": "11292384761470835",
    "coinWaitingWei": "0",
    "coinQueued": true,
    "harvestableWei": "0",
    "depositNeededWei": "38707615238529165",
    "vaultShareBps": 8000,
    "takeBps": 270
  },
  "assumptions": [
    "only this coin trades: fees from other coins on the same listing bring the purchase sooner",
    "the cap posted now is still the cap then (a cap lasts one hour and is re-posted from the market)",
    "\u2026"
  ]
}
```
`volumeNeededWei` is the trading volume **in this coin alone** that would bring its listing to the keeper's next declaration:
`depositNeeded / (0.80 × (0.70% + creator tax))`. It is `null` with a `reason` (`no-live-cap`, `listing-paused`, `listing-blocked`,
`tax-unknown`, `live-unavailable`) rather than a guess, and it always carries its `inputs` so anyone can recompute it.
Fee windows (`fees24hWei`, `fees7dWei`) sum **harvests**; fees sit in Pons's curve or hook until swept, so a quiet window can mean
"not harvested yet", not "no trading".

**Volume is a different number from fees** (docs/phase14.md §4). `volume24hWei`, on every coin summary, is the pair asset traded
in the coin over the last 24 hours: the sum of the `quote` leg of every trade in the window, at either venue, exactly as the
chain recorded it — on a buy what the buyer sent, on a sell what the seller received, neither adjusted for Pons's fee or the
creator tax. It is the same quantity the chart reports, so `sum(candle.v)` over `range=1d` equals `volume24hWei / 1e18`. It is
never `null`: a coin nobody traded in a day has volume `0`. Rank a market by this, not by `fees24hWei`.

`live.pending`: `escrowWei` and `routerBalanceWei` are one `harvest()` away. After graduation `pool` holds Pons's hook's pending
buckets — the ETH side in wei, the **coin side in the coin's own units, not priced** — and `operatorOnly: true` while any coin-side
fee is pending, because then only Pons's operator may sweep. Before graduation `curvePending` is `null`: no getter for it has been verified.

### `/v1/coins/:address/trades` and `/v1/coins/:address/candles` — the chart

A coin trades on Pons's bonding curve, and after graduation in its Uniswap v4 pool. Both are indexed, into one series.

```json
{ "items": [{ "id": "69014853:2", "side": "sell", "venue": "curve", "quoteWei": "2438000000000000", "coinWei": "826611200000000000000000",
  "feeWei": "25395833333333", "taxWei": "76187500000000", "price": 3.0727e-9, "trader": "0x…", "recipient": "0x…",
  "tx": "0x…", "block": 69014853, "logIndex": 2, "at": 1790022126 }], "next": "69014853:2", "total": 107 }
```

**What the price is.** `price` is **the pair asset per one whole coin** — on Robinhood Chain, ETH per coin, a small number like
`3.07e-9`. It is the *curve-side* price: the quote that actually reached the curve, over the coins that left it. Pons takes its
fee and the creator's tax out of the quote leg on **both** sides (measured: a sell's `fee` is 1.042% of `quoteOut`, which is
1.000% of the gross), so drawing the price each trader paid would put a permanent fee-wide zig-zag in the line. The two gross
legs are on every trade, so a page that wants the price actually paid divides `quoteWei` by `coinWei` itself. Market cap is
`price × totalSupply × dollars per ETH`, and since docs/phase12.md all three of those numbers are served — see
**A market cap in dollars** below — though the product itself is still left to the caller, so nobody has to accept our unit.

`feeWei` and `taxWei` are `null` for a `venue: "pool"` trade: there the hook charges both outside the `Swap` event, so the event
does not carry them and this API does not guess. On a pool trade `trader` is the **swap router** the trade came through, not the
person. Pons's anti-snipe tax (9,900 bps decaying over 3 seconds, read from the factory's own `snipeTaxStartBps()` and
`snipeTaxSeconds()`) has no field of its own: when it is charged it is inside these same two numbers, so a buy in a coin's first
seconds really does show an enormous `taxWei`.

`/v1/coins/:address/candles?range=` returns `{res, range, coin, quote, priceIs, candles, marks, supplyWei, usdPerEth,
usdPerEthAt, usdPool, trades, tradesAllTime, launchedAt, span, seconds, graduation, pool}`. A candle is `{t, o, h, l, c, v, n}`:
opening second, open, high, low, close, the pair asset traded in it, and the number of trades in it.

**Ask with `range`.** Five of them — `5m`, `1h`, `6h`, `1d`, `all` — and each names a **window**, from which the bucket follows:
5 seconds, 1 minute, 5 minutes, 15 minutes, and for `all` the finest bucket that keeps the coin's whole life a drawable series.
The response reports both, as `range`, `span` and `seconds`, so you always know what you were given. A `range` that is not one of
those five is a **400**, not a quiet substitution.

The reason ranges exist is that a window always divides into time. The older `res=trade` gave every trade its own bar, and on a
bonding curve the step from one trade to the next is whatever the next buy cost — routinely tens of percent — so a reader met a
change figure that was true and answered no question they had asked (docs/phase12.md §1).

**`res=` still works, unchanged**, because the deployed site asks for it: `trade` (every trade at its own second, over 6 hours),
`5m` (24 hours), `1h` (7 days), and a coin younger than 6 hours has its whole life inside `trade`. When you ask with `range`,
`range` is your range and `res` is only the nearest old label, kept so an old client still parses the answer; when you ask with
`res`, `range` is `null`. Read `range` and `seconds` for what was actually drawn.

**A market cap in dollars.** Three fields carry the other two numbers, and every one of them may be `null`:

- `supplyWei` — the coin's whole supply in its own base units, read from `totalSupply()` once at launch. Pons mints all of it
  there, so it is a constant per coin. Measured on a live staging coin: `1e27`, which at 18 decimals is a billion whole coins.
- `usdPerEth` and `usdPerEthAt` — dollars per one whole ETH, and the second it was read. It comes from `slot0().sqrtPriceX96`
  on **one Uniswap v3 pool**, `usdPool`, on this same chain, squared and scaled; it is cached and re-read every few minutes, so
  it costs no read per request. It returned **$2,785.14** on Robinhood Chain on 2026-09-21.

It is a **spot read of one pool at one block, not a market data feed**, and a page that shows a dollar figure has to say so.
`null` means not known — a supply not yet read, a chain with no such pool, a pool that would not answer, or a cached rate gone
stale past an hour — and a consumer that meets one falls back to the pair asset and says which unit it is drawing. Nothing here
is defaulted, estimated, or carried over as a guess. Market cap itself is `supplyWei / 1e18 × candle.c × usdPerEth`.

**Nothing is interpolated.** A bucket nobody traded in is left out rather than filled, and each bucket opens at the previous
close, so the line is continuous where the trading is not. A coin that has never traded returns `candles: []` and
`tradesAllTime: 0` — never a price, never a flat line at zero; the page has to say so itself.

`graduation` is `{at, block, tx}` for the moment the bonding curve ended and the pool began, or `null` while a coin is still on
its curve. It sits between the last curve trade and the first pool trade, so a chart can draw the line where one venue became
the other.

### `/v1/coins/:address/holders` — who holds it

```json
{ "coin": "0x458d…5097", "holders": 10, "supplyWei": "1000000000000000000000000000", "creator": "0xe85A…8CF7",
  "complete": true, "scannedThrough": 69023770, "transfersFrom": 69023771,
  "items": [{ "rank": 1, "address": "0x5f0c…5b9f", "balanceWei": "152061129736034564016599375", "share": 0.15206112, "isCreator": false }],
  "excluded": [{ "address": "0x179D…BEcb", "why": "curve", "balanceWei": "572980338266492564151136396", "share": 0.57298033 }],
  "total": 10, "limit": 25, "offset": 0 }
```

**Where a balance comes from.** Every Pons coin is an ERC-20, and this is the running sum of its own
`Transfer(address,address,uint256)` logs — nothing else. `balanceOf` is never called for a holder: a number read from the
chain would be one the events cannot explain, and the first reindex would disagree with it. A mint (`from` = the zero address)
and a burn (`to` = it) are the same arithmetic; **the zero address is never a holder**, and a balance that reaches exactly
zero stops being a row rather than staying on as a holder of nothing. `share` is a **fraction** of `totalSupply` —
`0.15206112` is 15.2% — or `null` when that read has not succeeded, because a percentage of an unknown denominator is not a
number. `rank` is over the whole list rather than the page, so the first row of `offset=25` is 26.

**A holder is an address, not a person.** One trader with three wallets is three rows here; a custodian holding for ten
thousand people is one. Nothing clusters addresses.

**`excluded` is the plumbing, and it is not hidden.** A coin's largest balance is almost always a contract. Measured across
the 77 coins on staging on 2026-09-21: a coin's own **curve** is minted the whole supply and still held up to **57.30%** of
it, and the **Pons factory** held exactly **28.5714%** (2/7, reserved for the pool a graduated coin gets) of 64 of them. So
the count and the list leave out the coin, its curve, its router, Pons's factory, the v4 PoolManager and its hook — each one
ours, Pons's or Uniswap's — and every one of them that holds something is served here with its balance and the reason, since a
reader who can see that the listed shares come to 14% of the supply is owed the rest of the answer. `why` is one of `coin`,
`curve`, `router`, `pons-factory`, `pool-manager`, `hook`.

**`complete`, and why a count can be `null`.** `Transfer` indexing began at whatever block the follower had reached, so the
history before it is fetched once, two ranges of blocks a pass, oldest first (docs/phase13.md §4). While that walk is running
`complete` is `false`, `holders` is `null` both here **and on `CoinSummary`**, and `scannedThrough` says how far it has got: a
count over the tail of a coin's history is not a holder count, and this API does not offer one as though it were. The balances
it does have are still served — they are right for the blocks it has read. `scannedThrough` and `transfersFrom` are both
`null` on a database that has never followed a chain (the synthetic fixtures), where nothing was skipped because nothing was
ever indexed.

A coin that was launched and never held is `holders: 0` with an empty list. A coin nobody launched here is a **404**.

### `/v1/acquisitions` — `verify`
```json
{
  "links": [
    {
      "label": "the Bitcoin transaction",
      "url": "https://mempool.space/tx/42b21327f2d070e71f83239cdd81af02dcd21ba4c8252d13eec2e6a2ed49e930"
    },
    {
      "label": "the listing's Bitcoin address",
      "url": "https://mempool.space/address/bc1p-local-demo-not-a-real-address"
    },
    {
      "label": "the inscription",
      "url": "https://ordinals.com/inscription/0000000000000000000000000000000000000000000000000000000000000000i0"
    }
  ],
  "check": {
    "kind": "inscription",
    "txid": "42b21327f2d070e71f83239cdd81af02dcd21ba4c8252d13eec2e6a2ed49e930",
    "inscriptionId": "0000000000000000000000000000000000000000000000000000000000000000i0",
    "address": "bc1p-local-demo-not-a-real-address",
    "expect": "the inscription's current owner is `address`, until it is delivered"
  },
  "note": "A receipt is the attestor's word. These links let anyone check it against Bitcoin; this API has not."
}
```
`links` are for a person; `check` is the same thing for a program. **This service makes none of those requests.** In particular it
never calls UniSat or Magic Eden: both require written permission for commercial use and we have none. `indexerPath` (runes) is
UniSat's documented Open API path for the reader to call with the reader's own key; any runes indexer answers the same question.
A receipt is the attestor's word — these hints are how anyone checks it against Bitcoin.

### `/v1/draws/:id`
`verifySnapshot.command` is the exact `tools/snapshot` command that recomputes `root`; the block and the other inputs are in the
published file at `uri`. `challengeEndsIn` counts down only while the draw or distribution is open. `draw.drandUrl` is the public
beacon for the pinned round. `timeline` is every event for the lot, in order, with its transaction.

### `/v1/wrapped/:slug` — draws (docs/audit/2026-09-26/fixes-W2-site.md §3)
`draws` are the collection's raffle-vault draws, newest first (30): `DrawAnnounced` → `SnapshotFixed` / `DrawOpened` → `DrawPinned`
→ `DrawResolved` → `Won`, or `DrawCancelled` / `DrawVoided`, with `voidableAt` (announced + 6 h, pinned + 7 days, resolved + 7 days).
For an opened draw the indexer fetches the list at `uri` (https or `ipfs://`, through the E-12 guard, 8 MB cap), rebuilds the tree
with the sentinel leaf `leaf(address(0), totalTickets, snapshotAt)` and reports `published.status` `matches`, `differs` or
`unreadable`. `claim` (holder, range, proof) is given only for a resolved draw, computed from its own tree, and only when that tree
reproduces the on-chain root. The reserve report's floor module gains `surplusWei` and `escaped` (the stale fallback). Round 3: the module's `exitAtBacking` (burns at the backing while the keeper is paused, or escaped), a coin's `refundsHeld` (the queue's `holdsRefunds()`), and a draw's `seedOrigin` (`SeedCarried`) and `seedPassedOn` (voided unopened, or cancelled before its pin: the coin's next draw reuses its seed round and snapshot time; the snapshot time is only ever `SnapshotFixed`'s).
A draw opened over a snapshot with no eligible holder (`totalTickets` 0, the list's tree is the sentinel alone, and an empty
`entries` list `matches`) is closed by `requestDraw` with no winner: `DrawEmpty` → stage `empty` (final, no `voidableAt`, its seed used
rather than passed on; the unit stays with the coin for its next draw). docs/audit/2026-09-26/review2/fixes-R3-carry.md.

## Errors
`{ "error": "no such coin", "status": 404 }` — 404, 405, 429 (`Retry-After` seconds), 500.
