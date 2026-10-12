# Phase 10 — a coin's own artwork

Every picture on the site is real except the one that should be easiest: a coin's own logo. Collections and runes come from a
committed catalogue of real Bitcoin artwork (docs/phase9.md); a launched coin wears a monogram cut from its address. This phase
gives the indexer the logo, so a coin looks like itself.

## Where the logo actually is, read from the chain

`IPonsV2LaunchFactory.TokenParams` carries `string logo` (src/interfaces/IPons.sol), and our `Launcher.LaunchParams` carries it
through. Two facts decide the design, and both were read from mainnet rather than assumed:

1. **No event carries it.** Every log the Pons factory `0x7eD598BcEf8bd9Edd8C97A195C6d13f40801EC7e` emitted in the 3,000 blocks
   to head 69,095,865 was fetched (59 logs, 5 distinct topics). The widest carries **96 bytes** of data — three words. A logo
   string cannot fit in any of them. Our own `Launched(coin, creator, listingId, curve, router)` does not carry it either.
2. **The calldata does.** Transaction `0xd1b10eb45a960131ff95511e04a43b95f52a5a46d8c4341a1b3338332100da26`, a real `launchToken`
   on the factory, holds `ipfs://bafkreicuspukrnaisnzred73svouxh3favyf4frtdbjlha36juefmx2ygq` in its 1,380 bytes of input, beside
   the name `CATDOG`, a description and `https://x.com/catsanddogcoin`.

So the form is **an `ipfs://` URI on mainnet**, not the `https://ipfs.io/...` gateway URL that pons-swarm's own data showed — that
project resolves the URI for display. The staging simulator writes a third form, a `data:image/svg+xml,...` URI of about 310
bytes. All three must be handled.

**A coin launched through our Launcher is reached by one `eth_getTransactionByHash` on the transaction that emitted `Launched`,
and one `decodeFunctionData` against `Launcher.launch`.** Launches are rare, so this is one extra read per coin, once.

*Worth doing later, not now:* adding `logo` to our own `Launched` event would remove the read entirely. It is a contract change,
so it belongs to the next revision of the contracts, before an audit — not to an indexer phase. Recorded here so it is not lost.

## What is stored and served

`coin_meta` gains `logo`; `CoinSummary` gains `logoUrl`. **Kept byte for byte as the chain has it**: an `ipfs://` URI is served as
an `ipfs://` URI. Choosing a gateway is a display decision, and a gateway is a third party that sees every visitor — that choice
belongs to the site, not to the API, and the site records which gateway it picked.

## A logo is a stranger's string

Anyone may launch a coin, so `logo` is arbitrary text from an untrusted author. The indexer therefore **stores only what could be
an image, and serves null otherwise**:

| Accepted | Why |
|---|---|
| `https://…` | an ordinary image URL |
| `ipfs://…` | the mainnet form; the site picks a gateway |
| `data:image/…` | the staging simulator's form; the media type must begin `image/` |

Everything else is dropped to null, including `javascript:`, `data:text/html`, `vbscript:` and any scheme not listed. The stored
value is capped at 1,024 characters — Pons's own cap is believed to be 512 bytes and is **not verified here**, so the indexer
does not rely on it. Control characters and whitespace are rejected rather than trimmed: a logo is a URI or it is nothing.

**The site needs no change.** `CoinArt` in `web/src/components/Plate.tsx` already prefers a logo and already renders it only as
the `src` of an `<img>`, which is the safe form: an SVG loaded through `<img>` executes no script in any current browser, and a
non-image simply fails to `onError` and falls back to the monogram. Two residual facts, stated rather than fixed: an `https://`
logo lets its host see a visitor's IP, and an `ipfs://` logo is resolved through whichever gateway the site names. Neither is a
script-execution risk; both are third-party contact, and the footer already tells a visitor the site loads nothing from third
parties — **that sentence stops being true the day a coin with an `https://` logo appears**, and the site must soften it then.

## Amendment, 2026-09-21: the read had to be allowed, and what staging then showed

Proving this on the staging chain found the one thing the unit tests could not: `rpcOf`'s `READ_ONLY` list, the guard that makes
this package incapable of sending anything, did not include `eth_getTransactionByHash`. Every logo came back null on a real node
while every test passed, because the tests answer from a recording and never pass through that guard. `eth_getTransactionByHash`
is a read, so it joins the list, and `test/guards.test.ts` now asserts both that it is there and that nothing else came with it.

Against the hosted staging chain (id 9466301), from an empty database: **6 of 8 coins come back with artwork**. The six are the
simulator's, each a `data:image/svg+xml` URI of about 310 bytes, served byte for byte. The other two — the smoke test's `SMOKE`
and the fixture coin `RF` — were launched with `logo: ""`, so they correctly have none and keep their monogram. The simulator
needed no change: all 60 of its generated logos are accepted unaltered, the longest 312 characters against its own 512 cap.

---

# Phase 10, part two — a chart for every coin

A coin has a price the moment somebody buys it, and until now the indexer did not look. This part indexes every trade, on both
venues a Pons coin ever trades on, and serves the two things a chart needs: the trades, and candles computed from them.

## The events, read on the chain rather than taken from a document

Every signature below was matched against live logs on the staging fork of Robinhood Chain (chain 9466301) on 2026-09-21, in the
595 blocks from 69,014,783 to 69,015,378, and against Robinhood Chain itself where staging could not reach.

**The bonding curve**, one contract per coin, its address learnt from our own `Launched`. In that window its logs were:

| Count | Event | |
|---|---|---|
| 212 | `CurveBuy(address indexed buyer, address indexed recipient, uint256 quoteIn, uint256 tokensOut, uint256 fee, uint256 tax)` | a buy |
| 107 | `CurveSell(address indexed seller, address indexed recipient, uint256 tokensIn, uint256 quoteOut, uint256 fee, uint256 tax)` | a sell |
| 3 | `CurveCompleted(address recipient, uint256 quoteOut, uint256 tokenOut)` | **graduation**: the curve's last act |
| 3 | `CurveBuyRefunded(address indexed buyer, uint256 refund)` | |
| 46 | `FeesSwept(uint256,uint256,uint256)` | not a trade |
| 24 | `SnipeTaxExempted(address)` | not a trade |

`PoolGraduated` — which the Pons documentation lists — matched **zero** logs, here and on mainnet; the factory emits
`LaunchSwept(address indexed token, uint256 quoteOut, uint256 tokenOut)` instead, three of them, one per completed curve. The
indexer therefore takes graduation from the curve's own `CurveCompleted`, which is the event it actually sees.

**The pool.** After graduation a coin trades in a Uniswap v4 pool. The PoolManager is not written down per chain: it is read from
the Pons factory itself, `poolManager() → 0x8366a39cc670b4001a1121b8f6a443a643e40951`, beside `memeHook() → 0xe5e7…e044`. Its two
events are `Initialize(bytes32 indexed id, address indexed currency0, address indexed currency1, uint24 fee, int24 tickSpacing,
address hooks, uint160 sqrtPriceX96, int24 tick)` and `Swap(bytes32 indexed id, address indexed sender, int128 amount0,
int128 amount1, uint160 sqrtPriceX96, uint128 liquidity, int24 tick, uint24 fee)`. That contract is shared by every pool on the
chain — 248 distinct pools traded in one 300-block sample — so it is **never read by address alone**: each range asks for the
`Initialize` whose indexed currency is one of our coins, and the `Swap`s of the pools those gave us.

**Proof that the pool half decodes, since staging could not provide it.** No coin on the staging fork has reached a pool: five
have completed their curves, but the fork never runs Pons's own graduation executor, so no `Initialize` and no `Swap` exist
there. The path was proved against Robinhood Chain instead. Pool
`0x9ba8e36b277e5b3e21cf4a088e912fbbec62c7baf0723e3c96ae11295de67e2d`, initialized at block 69,073,823 for coin
`0x33021ce2c4Ea4414E874f3768442cddc33A553B7`, was folded through this indexer's own `decode` and `apply`: the pool row came out
with the coin as currency1, native ETH as the quote, fee 0, tick spacing 200 and the Pons hook, and six of its real swaps became
six trades at about 3.1e-8 ETH a coin, with the buys and the sell on the right sides.

## What a price is here

**The pair asset per one whole coin** — ETH per coin on this chain, a number like 3.1e-8. It is the *curve-side* price: the quote
that reached the curve, over the coins that left it. Pons takes its fee and the creator's tax out of the quote leg on **both**
sides — measured on staging, a sell's `fee` is 1.042% of `quoteOut`, which is exactly 1.000% of the gross — so the price a buyer
pays sits above the curve and the price a seller receives sits below it. Drawing those would put a permanent 7% zig-zag through
the line that is fee, not price; the net prices of a buy and the sell after it are continuous, and those are what the chart draws.
Both gross legs stay on the trade in wei, so a page that wants the price actually paid divides them itself.

This is not pons-swarm's convention, which multiplies by the supply and calls the result a market cap in ETH. Market cap is
`price × supply` and any reader can do it; a price that is not multiplied by anything is the one that needs no explanation.
Nothing here converts to dollars: this chain's ETH has no price the indexer can read.

> **Amended 2026-09-21 (Phase 12).** That last sentence is wrong and was wrong when it was written. This chain's ETH *does*
> have a price the indexer can read: `slot0().sqrtPriceX96` on the WETH/USDG Uniswap v3 pool at
> `0x52e65b17fb6e5ba00ed806f37afcd2daa50271ca`, squared, is dollars per ETH — it returned **$2,785.14** against the staging RPC
> on 2026-09-21, because our fork carries Robinhood Chain's real state. docs/phase12.md §3 records the arithmetic and the
> caveats. The indexer now reads it every few minutes, caches it, and serves it beside the candles as `usdPerEth`, so the
> chart plots a market cap in dollars. What has **not** changed: it is a spot read of one pool at one block, not a market data
> feed; it degrades to `null` rather than to a guess; and it still puts no value on what a vault holds. The paragraph above
> stands in every other respect — `price` itself is still the pair asset per whole coin and is still multiplied by nothing.

**The anti-snipe tax has no field of its own.** The factory says `snipeTaxStartBps() = 9900` decaying over
`snipeTaxSeconds() = 3`. Whichever of `fee` and `tax` it lands in, it is inside them, and no trade was observed paying one (the
creator's own first buy is exempt), so the indexer does not split out a number it has not seen. A buy in a coin's first seconds
will show an enormous `taxWei`. That is real.

## What is served

`GET /v1/coins/:address/trades?limit=&before=block:logIndex` — newest first, cursored by the trade's own place in the chain.
`GET /v1/coins/:address/candles?res=trade|5m|1h` — `{res, coin, quote, priceIs, candles, marks, trades, tradesAllTime,
launchedAt, span, seconds, graduation, pool}`, with candles `{t,o,h,l,c,v,n}` computed **in the indexer**, so every reader draws
the same line. Three resolutions, as CardPaired's chart has: every trade over 6 hours, 5-minute buckets over 24 hours, hourly
over 7 days. A coin younger than 6 hours has its whole life in the first.

**Nothing is interpolated.** A bucket nobody traded in is left out, not filled; a bucket opens at the previous close, so the line
is continuous where the trading is not. A coin that has never traded returns an empty array and `tradesAllTime: 0` — never a
price, never a flat line at zero. `graduation` is `{at, block, tx}` or null, and sits between the last curve trade and the first
pool trade, so the chart can mark where one venue became the other.

## Measured on staging, 2026-09-21

13 coins, 11 with artwork, **434 trades** and **505 candles** across the three resolutions; five coins have graduated and carry
the mark. The two coins with no artwork and the one with no trades are the smoke test's and a fixture's, not the simulator's.

## What the site must change, and it is not one line this time

Two sentences on the site stop being true the moment this is drawn, and they are load-bearing sentences about honesty:

- `web/src/pages/Explore.tsx`, the paragraph beginning "Every coin launched through the Launcher": *"No price is read anywhere
  in this system, so there is no market cap here and no trading volume."*
- `web/src/pages/Analytics.tsx`, the "Not read at all" row: *"Prices. Nothing in this system knows what a coin or an asset is
  worth, so there is no market cap here, no trading volume, and no value of anything held."*

Both must now say what is actually true: the indexer reads every trade on the coin's curve and pool and knows a coin's price in
ETH; it still reads no dollar price, still puts no value on what a vault holds, and a coin's price still has nothing to do with
what its vault bought. That is the design agent's sentence to write, not the indexer's to change.
