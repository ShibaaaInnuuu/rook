---
title: Launching a coin
summary: How to launch a collection coin on userook.fun, opening the collection if you are the first, and what happens after.
group: Using Rook
---

## Before you start

- A Solana wallet, such as Phantom, Solflare or Backpack, holding a little SOL for Solana's account rent and network fees.
  Rook charges no launch fee.
- A name, a ticker and an image for your coin. **Choose carefully: they cannot be changed after launch.**

## Step by step

1. **Connect your wallet** on [userook.fun](https://userook.fun).
2. **Open "Launch a coin" and pick a collection.** The wizard lists every collection Rook can launch on right now: the 38 ready
   at launch (Ethereum, Solana and the Ordinals), and the Robinhood Chain ones once they are added. Your coin's backing will be NFTs from this
   collection. Each card shows the collection's graduation threshold in its wrapped token.
3. **Enter the name, ticker and image.** You can add a description, links, and a first buy paid in the collection's wrapped
   token. The website and X fields start prefilled with the collection's official links (its marketplace page when it has no
   website); you can change or clear them, and your coin carries whatever you leave there.
4. **Review the summary**: the collection, its graduation threshold, the fees and what the launch costs you.
5. **If the collection is not open yet**, the review step says **"Not open yet: be the first"**. Choose **Open it**. It needs no
   signature and Rook pays for it. The page shows the request's place in the queue (Rook opens at most 8 collections a day), what the custodian is waiting
   on if it waits, then the opening, then lets you launch.
6. **Sign the transaction in your wallet.** It creates your coin and its bonding curve on Meteora, using Rook's settings for that
   collection. It is simulated before your wallet is asked. Trading opens straight away.
7. **Share the link.** The coin's page shows its progress towards its threshold.

> [!NOTE]
> Your first buy, made in the launch itself, pays only the normal 1 % fee. Everyone else pays the anti-sniper fee for the first
> 10 minutes: it starts at 50 % and falls steadily to 1 %. A buy you make after the launch, in a separate transaction, pays it too.

## Opening a collection

Rook doesn't choose which collections go first. A collection opens the first time someone asks: from the launch wizard, or with
**Open it** on the collection's page. Anyone can ask, and asking twice changes nothing. Requests are served oldest first.
Opening creates the collection's wrapped token and its coin configuration on Solana (about 0.023 SOL, paid by Rook). While it
opens, the page checks every few seconds. Once open, the collection stays open for everyone: its coins trade, its wrapped token
can be minted and burned, and its backing works. See [Opening a collection](#/docs/how-it-works/opening-a-collection).

## After launch

- **On the curve.** People buy and sell against the collection's wrapped token, paying in SOL through the site if they like.
  There is no deadline and no refund: a coin that never reaches its threshold just keeps trading on its curve. It never gets
  backing.
- **At its threshold.** The coin graduates. Half the raise goes into the coin's own vault as wrapped tokens, from that moment.
  The rest becomes permanently locked liquidity in a Meteora pool. The custodian handles the move. See
  [Graduation thresholds](#/docs/how-it-works/graduation-thresholds).
- **After graduation.** The coin trades in that pool, which is open to the usual Solana trading apps where they route it, and
  its holders can burn coins for their share of the vault. Its holders also enter the collection's weekly raffle.

## What the creator earns

- **On the curve: nothing.** On collection coins the creator's share of curve trading fees is 0 %. Rook's share pays for the
  collections' pools, reserves and operations (see [Fees](#/docs/fees)).
- **After graduation: a share of pool fees.** Half of the locked pool liquidity is a position in the creator's name. It can never
  be withdrawn, but it earns its share of the pool's 1 % trading fee, in wrapped tokens, which the creator can claim.

## Good to know

- Anyone can launch a coin, and anyone can trade it. A coin's name and image are its creator's choice.
- A coin's backing is in wrapped tokens the custodian holds for it. What a wrapped token is worth depends on the collection's
  floor price (see [Risks](#/docs/risks)).
- A first buy in the launch is paid in the collection's wrapped token from your wallet. If you hold none, leave it empty and buy
  with SOL on your coin's page right after, or mint wrapped tokens first with a mint intent.
