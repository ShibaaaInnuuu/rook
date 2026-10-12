---
title: Fees
summary: What trading costs, and where every share of every fee goes.
group: Using Rook
---

## On the bonding curve

Every trade on a coin's curve pays a fee in the collection's wrapped token. It starts at **50 %** at launch and falls steadily to
**1 %** over the first 10 minutes (the anti-sniper fee). After that it stays at about 1 %. ROOK's curve has the same schedule, in
SOL. The one exception, on a collection coin: the creator's first buy, made in the launch transaction itself, pays only the 1 %.

| Share of each curve fee | Collection coins | ROOK |
|---|---|---|
| Meteora (protocol) | 20 % | 20 % |
| Rook | 80 % | 80 %, to the custodian's wallet: it pays Rook's running costs |
| Creator | 0 % | 0 % |

## At graduation

A collection coin graduates at its collection's threshold of whole wrapped tokens (see
[Graduation thresholds](#/docs/how-it-works/graduation-thresholds)). ROOK graduates at 85 SOL.

| | Collection coins | ROOK |
|---|---|---|
| Raise at graduation | the threshold, in wrapped tokens | 85 SOL |
| To the coin's backing vault | 50 % | none (ROOK has no backing) |
| Into the locked pool | the other 50 % and the remaining coins | 85 SOL and the remaining coins |

The pool's liquidity is permanently locked, split 50/50 between a position for Rook and a position for the creator. Neither can
ever be withdrawn.

## After graduation

A graduated coin trades in a Meteora pool with a **1 %** fee, paid in wrapped tokens. Meteora keeps 20 % of it. The rest is earned
by the two locked positions.

## Buying with SOL

The site's SOL route swaps SOL into wrapped tokens on the collection's wrapped-token/SOL pool first. That pool charges **0.4 %**,
paid in SOL, which goes to the collection's own pool fund (it is the custodian's position). The site shows the all-in price,
including the pool's price impact, before you sign.

## Where Rook's share goes

Rook's share of fees arrives in wrapped tokens. For each collection, part of it builds that collection's wrapped-token/SOL pool
(the custodian's pool desk, toward about 10 SOL a side) until the pool is deep enough; nobody seeds the pools with outside money.
The rest is burned at the floor price for SOL. That SOL, and the SOL from the pools, goes to Rook's
treasury, which splits every deposit:

| Share | Goes to |
|---|---|
| 90 % | The collections' reserves, divided between collections by their trading volume over the last 7 days. It tops up their backing funds: it pays for NFTs above the backing price and covers shortfalls, and is never paid out to anyone. |
| 10 % | Operations: the custodian's running costs (Internet Computer cycles, data providers). |

The operator can change the split, or the operations wallet, but every change waits **24 hours** in public before it can take
effect, so holders always see it coming.

Each collection's reserve is capped at about two floor prices. Once a collection's reserve reaches its cap, the rest of its share
is split: **50 %** to that collection's [holder raffle](#/docs/backing/the-weekly-holder-raffle) pot, the rest to operations. The
cap and the raffle share can change only after the same 24-hour public wait.

The raffle pot buys one whole wrapped token at the public mint price whenever it can pay for one, and that token is drawn weekly
among the holders of the collection's graduated coins.

## Other costs

- **Launching a coin:** no Rook fee. You pay Solana's account rent and network fees.
- **Opening a collection:** nothing for you. Rook pays the about 0.023 SOL it costs to create the collection's wrapped token and
  coin configuration, from a small float at launch and then from fees.
- **Minting or burning wrapped tokens directly:** 0.0015 SOL per operation plus a spread around the floor price, from **0.15 %**
  in calm markets up to **10 %** when the floor is stale, moving fast or thinly traded. Part of what comes in on a mint is kept
  as a SOL reserve for the collection and never spent on NFTs.
- **Burning coins for your share of the vault, or redeeming an NFT:** 0.0015 SOL per operation, taken from the deposit's account
  rent where there is no SOL in it.
