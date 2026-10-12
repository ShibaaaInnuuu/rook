---
title: FAQ
summary: Short answers to the questions people ask first.
group: Trust
---

## Is Rook live?

Yes, for every collection Rook can already buy, hold and price: the eleven Ethereum collections, the twelve Solana collections,
Bitcoin Frogs and the fourteen other Ordinals collections, 38 in all. Rook doesn't choose which go first. A collection opens the
first time someone asks for it (paid by Rook, at most 8 a day, in the order asked), and from then on its coins trade and its
backing works: there is no second stage to wait for. The three Robinhood Chain collections follow after the launch ceremony's
check there. See [What is Rook](#/docs/what-is-rook/the-collections).

## Which chains are the NFTs on?

Four: Ethereum, Solana, Bitcoin (Ordinals) and Robinhood Chain. The coins and the wrapped tokens are all on Solana; only the NFTs
behind them live on their own chains, held by the custodian there. See
[How the NFTs are bought](#/docs/how-it-works/how-the-nfts-are-bought-chain-by-chain).

## How do I open a collection nobody has launched on?

Pick it in the launch wizard, or open its page, and choose **Open it**. It needs no signature, Rook pays for it, and it takes a few
minutes; then you can launch. See [Opening a collection](#/docs/launching/opening-a-collection).

## What do I get for holding a collection coin?

The coin itself, which trades like any memecoin. After graduation, you also get the right to burn coins for your share of that
coin's wrapped tokens, and an entry in the collection's weekly raffle.

## Do I get an NFT for my coins?

Not directly. Burning coins gives you wrapped tokens. Once you hold **one whole** wrapped token, you can redeem it for a random NFT
from that collection's custody, delivered to your address on the NFT's own chain: an Ethereum or Robinhood Chain address, a
Bitcoin taproot address, or a Solana wallet.

## Why are coins priced in a wrapped token and not in SOL?

Because the backing is wrapped tokens: half of every graduation goes straight into the coin's vault, with nothing to convert and
nobody to wait for. You can still pay in SOL: the site swaps SOL into wrapped tokens and coins in one transaction.

## Can I choose which NFT I get?

No. Each redemption is assigned a random NFT from what the custodian holds for that collection, using a public randomness round
that did not exist when you redeemed and whose signature the custodian verifies.

## How long does delivery take?

The custodian has 7 days from the draw to deliver. If it does not, your wrapped token is minted back to you (see
[Backing and wrapped tokens](#/docs/backing)).

## Why do collections graduate at different thresholds?

So that one graduation is always worth about 85 SOL, or more for the most expensive collections, and the coin's vault always
holds more than one NFT's worth. A collection with a high floor needs fewer wrapped tokens; one with a low floor needs more. See
[Graduation thresholds](#/docs/how-it-works/graduation-thresholds).

## What happens if a coin never reaches its threshold?

It keeps trading on its bonding curve. It never graduates and never gets backing. There is no automatic refund.

## Does the creator earn from their coin?

Not from curve trading: on collection coins the creator's share is 0 %. After graduation, the creator's locked pool position earns
its share of the pool's 1 % trading fee.

## What is the holder raffle?

Once a week, each collection draws one whole wrapped token (one NFT in custody) among the holders of its graduated coins, weighted
by the backing their coins carry. The prize is bought from a pot fed by half of the collection's reserve overflow. The snapshot of
every holder's weight is published before the draw, and the randomness comes from a drand round chosen after it. See
[Backing and wrapped tokens](#/docs/backing/the-weekly-holder-raffle).

## Where do Rook's fees go?

To the collections' wrapped-token/SOL pools while they are shallow, then to Rook's treasury, split between the collections'
reserves and operations; above a collection's reserve cap, half goes to its holder raffle. ROOK's fees pay Rook's running costs.
See [Fees](#/docs/fees).

## Who can change Rook?

Changes to the custodian's code wait 72 hours in public. Money parameters set by the operator wait 24 hours. Nobody can move the
custodian's funds by hand. See [Security and trust](#/docs/security).

## Has Rook been audited?

Internally, three times, in October 2026: the first Solana code in six parts (nothing Critical; every Medium fixed), the
custodian-only build (3 High, 9 Medium and 15 Low findings, all fixed, each with a test), and the code written after it (Solana NFT
buying, the Ordinals beyond Bitcoin Frogs, Robinhood Chain, opening on demand: 6 High, 11 Medium and 14 Low, all fixed, each with a
test). See [The audits](#/docs/security/the-audits).
