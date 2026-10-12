# Phase 12 — a chart a person can read: market cap, in dollars

2026-09-21.

The coin page has had a chart since Phase 10. It is not wrong and it is not readable. This phase replaces what it plots, what
it plots it against, and what draws it — and writes down why a competitor's chart shows a token that has not graduated, because
that question is the reason this phase exists.

## 1. What is actually wrong with the chart we have

Three faults, and only the third is cosmetic.

**The unit is unreadable.** The chart plots `pair asset per whole coin`. A live staging coin reports `3.133e-9`. Nobody holds a
mental model of a number in that range, so the axis carries no meaning and the reader cannot tell a good day from a bad one.

**The default bucket is one candle per trade.** `res=trade` gives every trade its own bar. On a bonding curve, consecutive
trades move price by whatever the next buy costs, so the change between two adjacent bars is routinely tens of percent. The
`+82.3%` a reader meets is the change from one trade to the next trade, which is a true number that answers no question anyone
asked. **Every launchpad chart worth copying buckets by time, never by trade**, and this phase removes `trade` as a user-facing
option.

**It is hand-drawn SVG.** `web/src/components/` draws its own axes, its own crosshair and its own hit-testing, and so does
CardPaired's `web/src/components/PriceChart.tsx` — the two are cousins of the same 200-line file. Neither will ever look like
the charts people compare them to, because that look is a library's worth of work in axis stepping, crosshair behaviour and
autoscaling.

## 2. Why a competitor's pre-graduation token has a GeckoTerminal chart

The question that prompted this phase: a coin at a **$4.8k market cap** shows a GeckoTerminal chart, which should be
impossible if GeckoTerminal only indexes DEX pools and the coin has not graduated into one.

It is possible, and the reason is that **GeckoTerminal indexes the bonding curve itself as an exchange.** Queried
2026-09-21, `https://api.geckoterminal.com/api/v2/networks/robinhood/dexes` returns **two separate Pons V2 entries**:

| GeckoTerminal dex id | what it holds | sampled FDV range | sampled liquidity |
| --- | --- | --- | --- |
| `pons-v2` | the **bonding curve**, pre-graduation | $4.6k – $53.6k | $0 – $4.7k |
| `pons-v2-dex` | the **graduated Uniswap v4 pools** | $88k – $3.26M | $29k – $50k |

The pre-graduation sizes cluster exactly where a curve sits and the post-graduation ones exactly where a real pool sits, which
is what makes the split legible. So graduation is not what puts a token on GeckoTerminal. Being on a curve that GeckoTerminal
has integrated is. There is nothing to reverse-engineer and nothing clever happening.

**What that means for us, in two halves.**

On a public chain, an Etch coin launches through the same Pons V2 factory, so it lands on `pons-v2` without us asking, and the
GeckoTerminal embed becomes available the way it is for anyone else. That is a thing to use later, not to build now.

On **staging it can never work**. Our chain is id 9466301, a fork we run; GeckoTerminal has never heard of it and never will.
Every coin a visitor meets today is on that fork. So the chart on the coin page has to be ours, and this phase builds it.

## 3. The three numbers a dollar market cap needs, and where each one comes from

Market cap is `total supply × price × dollars per ETH`. All three are already on chain and none of them needs an external
price API.

**Total supply.** Read `totalSupply()` on the coin. Measured on a live staging coin (`0x19AE…CD8e`): `1e27` at 18 decimals, so
**1,000,000,000 whole tokens**. Pons mints the full supply at launch, so this is a constant per coin, not a series.

**Price.** The candle close the indexer already computes, in pair asset per whole coin.

**Dollars per ETH.** Read `slot0().sqrtPriceX96` from the **WETH/USDG pool at `0x52e65b17fb6e5ba00ed806f37afcd2daa50271ca`**
and square it. This is the same on-chain rate CardPaired's indexer already reads (`indexer/src/sim/trade.ts`,
`usdPerEthFrom`). It works on our fork because the fork carries Robinhood Chain's real state: queried against the staging RPC
on 2026-09-21 it returned **$2,785.14 per ETH**.

Worked through on the coin the reader complained about, `CHAL2SIM` at `3.133e-9`:

```
1e9 tokens  ×  3.133e-9 ETH  ×  $2,785.14  =  $8,725
```

An $8.7k market cap. The number was never wrong. It was being shown in a unit nobody can read.

**Honesty on the rate.** It is a spot read of one pool at one block, it is fetched on our fork where every other price is a
fixture, and it is cached. The coin page must say `rate read from the WETH/USDG pool on chain` rather than implying a market
data feed, and staging must keep saying that its prices are made up.

## 4. What gets built

**A market-cap area chart**, on TradingView's `lightweight-charts` (v5.2.1, ~3 MB unpacked, MIT). It is the library the look
the reader asked for is actually made of, and writing a fourth hand-rolled SVG chart to imitate it is the mistake this phase
exists to stop.

- **Y axis: market cap in USD.** A toggle offers market cap in the pair asset for anyone who wants it. Price-per-token is
  demoted to a figure in the header, not the thing the chart plots.
- **X axis: time, bucketed by the chosen range** — `5M`, `1H`, `6H`, `1D`, `ALL`. The bucket is derived from the range, so a
  reader can never land on one-candle-per-trade.
- **The change figure is over the selected range**, not between two adjacent trades. This is what removes `+82.3%`.
- Crosshair with a tooltip carrying the value and the timestamp, the way the reference charts do.
- A launch marker, and a graduation marker when there is one.

**The same component serves CardPaired.** Its `PriceChart.tsx` has the identical per-trade fault, so the component is written
to take a plain series and a label and nothing repo-specific, and is ported rather than rewritten.

## 5. What this phase does not claim

The graduated half of the chart still has nothing to draw on staging: Phase 11 §8 recorded that our fork's sweep never creates
a Uniswap pool, so a graduated coin's series stops at graduation. That gap is upstream of this phase and is not closed here.

The USD rate is one pool read. If that pool is ever thin or stale on a public chain, the dollar axis inherits it, and the
pair-asset toggle is the honest fallback. No third-party price feed is introduced, and no figure on this chart is presented as
coming from one.
