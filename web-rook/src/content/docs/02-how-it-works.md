---
title: How it works
summary: The whole walk-through, from a coin's launch to an NFT at your address, and what you can check on the way.
group: Start here
---

Rook has three moving parts: **coins** that trade on Meteora's bonding curves on Solana, **wrapped tokens** (one per NFT
collection, one whole token for one NFT) that the coins are priced in, and **the custodian**, a program on the Internet Computer
that mints and burns every wrapped token, keeps every book, buys the NFTs and holds them. Rook deploys no programs of its own on
Solana: what Solana's validators do not enforce, the custodian's code does, and everything it does can be checked.

## A coin's life

1. **Launch.** A creator picks a collection and launches a coin: a name, a ticker, an image, one transaction. Every coin has a
   fixed supply of 1 billion and trades on a Meteora bonding curve **priced in the collection's wrapped token** (wMILADY,
   wMADLAD, wBFROG…): the more people buy, the higher the price. If nobody has launched on the collection yet, the first launch
   opens it (see [Opening a collection](#/docs/how-it-works/opening-a-collection)).
2. **The curve.** Anyone can buy and sell. For the first 10 minutes a falling anti-sniper fee applies, from 50 % down to 1 %;
   after that the fee is about 1 %, paid in the wrapped token. The one exception is the creator's first buy, made in the launch
   transaction itself: it pays only the 1 %.
3. **Graduation.** When the curve holds its collection's threshold, a set number of whole wrapped tokens (below), the coin
   graduates. **Half of the raise goes straight into a vault that belongs to the coin**: that is its backing, from that moment.
   The other half and the remaining coins seed a Meteora pool whose liquidity is **locked for ever**. The few coins left over on
   the curve are burned, which raises every remaining coin's share of the vault.
4. **After graduation.** The coin trades in its locked pool. Any holder can burn coins for their share of the vault's wrapped
   tokens; with one whole wrapped token, they can redeem a real NFT.

A coin that never reaches its threshold keeps trading on its curve. It never gets backing, and there is no refund.

## Buying with SOL

Coins are priced in wrapped tokens, but you pay in SOL. The site builds **one transaction** that swaps your SOL into the
collection's wrapped token on its wrapped-token/SOL pool, then the wrapped token into the coin (on its curve, or in its pool once
graduated), and shows you the all-in price before your wallet signs. Every transaction is simulated first. Trading apps such as
Jupiter may route the same path; the site's own route works with or without them.

You can also get wrapped tokens straight from the custodian, at the floor price, with a **mint intent** (below), and pay with the
wrapped tokens you hold.

## Graduation thresholds

Every collection graduates at its own number of whole wrapped tokens, **at least 3**, chosen so that the raise was worth about
**85 SOL** at the collection's floor on the day the threshold was set (more for the collections whose floor is high). Half of it
is the coin's backing: at least 1.5 whole wrapped tokens, more than one real NFT's worth from the first graduation. After that day
the SOL value of a threshold moves with the floor.

| Collection | Chain | Threshold | Worth at the day's floor |
|---|---|---|---|
| Milady | Ethereum | 5 wMILADY | ≈ 93 SOL |
| Bored Ape Yacht Club | Ethereum | 4 wBOREDAPE | ≈ 547 SOL |
| Pudgy Penguins | Ethereum | 5 wPUDGY | ≈ 295 SOL |
| CloneX | Ethereum | 26 wCLONEX | ≈ 88 SOL |
| Mad Lads | Solana | 11 wMADLAD | ≈ 92 SOL |
| SMB Gen2 | Solana | 7 wSMBGEN2 | ≈ 89 SOL |
| Bitcoin Frogs | Bitcoin | 29 wBFROG | ≈ 87 SOL |
| StonkBrokers | Robinhood Chain | 6 wSTONKBROK | ≈ 201 SOL |
| Chain Mancers | Robinhood Chain | 14 wCHAINMANC | ≈ 88 SOL |

Every collection's threshold is on the [collections page](#/collections) and in
[Backing and wrapped tokens](#/docs/backing/wrapped-tokens). The market cap at graduation is always 2.5 times the threshold.
Everything else is the same in every collection: the anti-sniper fee, the 50 % backing share, the locked pool and its 1 % fee.
ROOK, the genesis coin, trades against SOL, graduates at 85 SOL and has no backing. A collection's threshold is part of its
configuration, which cannot be changed.

## The vault and its backing

At graduation, half the raise is already in wrapped tokens, and it moves into the coin's vault: an account only the custodian can
move, whose balance is public. Nothing has to be triggered and nobody can delay it. Burning coins pays

```
your wrapped tokens = vault balance × coins you burn ÷ coins in existence
```

rounded down. "Coins in existence" includes the coins locked in the pool, so a coin that people sell back to the pool keeps its
share in the vault for the coins still out.

## What backs a wrapped token

Every wrapped token in existence is backed by one of two things: **an NFT the custodian holds**, or **SOL the custodian holds for
that collection**, counted at the floor price. A wrapped token is minted only against SOL at the floor price (a mint intent) or
against an NFT taken into custody. Part of every mint is kept as a reserve and never spent; the rest is the collection's backing
fund. When the fund holds enough for one NFT, the custodian buys one, within a price limit the backing sets.

## How the NFTs are bought, chain by chain

The custodian pays in SOL. For the other chains, a cross-chain service (Relay) turns the SOL into ETH or BTC, from a key that
holds only that one deposit.

| Chain | Collections | Bought on | Paid with | Held at |
|---|---|---|---|---|
| Ethereum | 11, from Milady to Identity MD | OpenSea (Seaport) | SOL → ETH through Relay | the custodian's EVM address on Ethereum |
| Robinhood Chain | StonkBrokers, Chain Mancers, Rekt Tradooors | OpenSea (Seaport 1.6 on Robinhood Chain) | SOL → ETH on Robinhood Chain through Relay | the same EVM address, on Robinhood Chain |
| Bitcoin (Ordinals) | Bitcoin Frogs and 14 more | Ordinals Wallet or UniSat | SOL → BTC through Relay | the custodian's taproot address |
| Solana | 12, from Mad Lads to Cyber Frogs | Tensor and Magic Eden (its buyer-only path, on by default) | SOL | the custodian's own Solana wallet |

Every purchase is checked before it is signed: the custodian rebuilds the transaction it is asked to sign and refuses anything but
the exact item at the price it judged; the marketplace's own program enforces that price. The item counts as held only once it is
seen at the custodian's address, and a purchase that never lands is undone in the books. Each collection's floor comes from its
marketplaces (OpenSea for Ethereum and Robinhood Chain, Ordinals Wallet checked against an independent floor for the Ordinals,
Magic Eden's listings checked on chain for Solana). A floor can move by at most 20 % a day and goes stale after 3 hours without a
fresh reading; a stale floor stops minting.

At launch the custodian holds at most 48 NFTs across all collections, with a cap for each.

## Redemption and delivery

With one whole wrapped token you can ask for a real NFT of its collection:

1. **Open a redemption intent** on the collection's page and name where the NFT goes: an Ethereum address for an Ethereum
   collection, an address on Robinhood Chain for a Robinhood Chain collection, a Bitcoin taproot address (`bc1p…`) for an Ordinals
   collection, or a Solana wallet for a Solana collection. You can also choose a burn address. The custodian checks the
   destination when you open the intent; it cannot be changed later.
2. **Send exactly one whole wrapped token** to the intent's deposit address. Once the deposit is final, the custodian burns it and
   opens a ticket.
3. **The draw.** The ticket is tied to a future round of drand, a public randomness service, that does not exist yet when you
   redeem. The custodian fetches it from two relays that must agree, verifies its signature, and assigns a **random NFT** from the
   collection's custody. You cannot pick which one.
4. **Delivery.** The custodian sends the NFT to your address within **7 days** of the draw: wallet to wallet on Solana, a
   transfer from its EVM address on Ethereum or Robinhood Chain, a Bitcoin transaction for an inscription. If it cannot, your
   wrapped token is minted back to you ([the refund windows](#/docs/backing/redeeming-a-wrapped-token-for-an-nft)).

## Intents: minting, burning and redeeming

Everything you do directly with the custodian goes through an **intent**: you open it (free, no signature), it gives you a deposit
address of its own, you send to it from any wallet or exchange, and the custodian acts once the deposit is final, usually within a
few minutes. Mint (SOL in, at least 0.05 SOL), burn (wrapped tokens in, at least 0.01), redeem an NFT (exactly one), redeem coins
for their vault share. Each costs **0.0015 SOL**. A deposit that cannot be acted on is returned after **6 hours**, less the fee; a
mint over the hour's limit mints what fits and keeps the rest for the next hour. Details:
[Minting, burning and intents](#/docs/backing/minting-burning-and-intents).

## Opening a collection

Rook does not pick which collections go first: the people do. At launch every collection the custodian can already buy, hold and
price is ready (pinned): the 11 Ethereum collections, the 12 Solana collections, Bitcoin Frogs and the 14 other Ordinals
collections, 38 in all. **A collection opens the first time someone asks**: on its page or in the launch wizard, "Not open yet: be
the first" and **Open it**. Opening makes the collection's wrapped token and its coin settings on Solana. It needs no signature,
and Rook pays for it (about 0.023 SOL, from a small float and then from fees). Requests are served oldest first, at most 8 a day:
the page shows a request's place in the queue and, when the custodian is waiting on something (its float, for example), what.

The **3 Robinhood Chain collections** join later: they are added after the launch ceremony, once the code of the marketplace
contract the custodian buys through on Robinhood Chain (Seaport 1.6) has been read back from that chain and pinned in the
custodian. Then each opens on demand like the others.

## The pool desk

Each open collection gets a wrapped-token/SOL pool on Meteora (a DLMM pair: 0.4 % fee, paid in SOL), which is what makes the
one-transaction SOL buy work. The custodian runs it: it keeps the pool's liquidity in a narrow band around the floor and re-centres
it, and when the pool's price drifts it brings it back by minting or burning at the floor (arbitrage), up to its limits. Pools are
**grown from trading fees**, not seeded with outside money, toward about 10 SOL a side, busiest collections first. While a
collection's pool is shallow, a large SOL buy pays more, and the site says so.

## The weekly raffle

Each collection has a raffle. Once a week, one whole wrapped token (one NFT in custody) is drawn among the holders of the
collection's **graduated** coins, weighted by the backing their coins carry. Holding is the entry. The pot is paid by Rook's
treasury; the custodian publishes the snapshot of every holder's weight before it commits to it, and the draw uses a drand round
chosen after that. Details: [The weekly holder raffle](#/docs/backing/the-weekly-holder-raffle).

## The treasury

Rook's share of every curve and pool fee arrives in wrapped tokens. Part builds the collection's pool while it is shallow; the
rest is burned at the floor for SOL, which goes to Rook's treasury. The treasury splits what comes in: **90 % to the collections'
reserves** (by each collection's trading volume over 7 days) and **10 % to operations**. A collection's reserve is capped at about
two floor prices; above the cap, half of its share goes to its raffle pot and the rest to operations. ROOK's fees go to the
custodian's wallet and pay Rook's running costs. Every change to these numbers waits **24 hours** in public. See [Fees](#/docs/fees).

## What you can check, and where

| What | Where |
|---|---|
| Each collection's books: NFTs held with their custody proofs, SOL-backed units, the backing fund, the reserve, tickets in flight | the collection's page, and the [Reserves](#/reserves) page |
| That the wrapped token's supply equals NFTs held plus SOL-backed units, and that the custodian holds the SOL its books owe | the [Reserves](#/reserves) page, read from the custodian and Solana in your browser |
| Every coin vault's balance against its book | the coin's page and the Reserves page |
| Every NFT at the custodian's address on its chain | the Reserves page's custody proofs, checked every 10 minutes by an independent reconciler with two data sources per chain |
| Every wrapped-token mint and burn | on Solana: each carries a label naming its book entry; a mint without one is the loudest alarm Rook has |
| The raffle's snapshot, before the draw | the collection's raffle panel |
| Your intent, step by step | the intent desk, with every transaction linked |
| Who controls what, and the audits | [Security and trust](#/docs/security) |

## The pieces

| Piece | What it does |
|---|---|
| Meteora's bonding curve (DBC) | Runs each coin's curve and graduation. Rook supplies one configuration per collection. |
| Meteora's pools (DAMM v2, DLMM) | Where a graduated coin trades (liquidity locked for ever), and where wrapped tokens trade against SOL. |
| The custodian | A program on the Internet Computer whose keys are threshold keys no single person holds. It mints and burns wrapped tokens, keeps every book, buys and holds the NFTs, and delivers them. |
| Relay | The cross-chain service the custodian uses to turn SOL into ETH (on Ethereum or Robinhood Chain) or BTC. |
| Marketplaces | OpenSea, Ordinals Wallet, UniSat, Tensor and Magic Eden: where the NFTs are bought and floors are read. |
| drand | Public randomness for redemptions and raffles, verified by the custodian. |
