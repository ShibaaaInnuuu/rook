---
title: Backing and wrapped tokens
summary: What stands behind a graduated coin, where the NFTs are held, and how to turn a wrapped token into a real NFT.
group: Using Rook
---

## Wrapped tokens

Each collection has a wrapped token on Solana. **One whole wrapped token stands for one NFT** of that collection. Smaller amounts
are fine to hold: a wrapped token has 9 decimals. Only the custodian can mint one, and it only does so against SOL at the floor
price or an NFT taken into custody. A collection's wrapped token exists from the day the collection opens, which is the first
time someone launches a coin on it (see [What is Rook](#/docs/what-is-rook/the-collections)).

| Collection | Chain | Wrapped token | Coins graduate at | Status |
|---|---|---|---|---|
| Milady | Ethereum | wMILADY | 5 | ready |
| Azuki | Ethereum | wAZUKI | 7 | ready |
| Mutant Ape Yacht Club | Ethereum | wMUTANTAPE | 5 | ready |
| Pudgy Penguins | Ethereum | wPUDGY | 5 | ready |
| Bored Ape Yacht Club | Ethereum | wBOREDAPE | 4 | ready |
| Doodles | Ethereum | wDOODLE | 15 | ready |
| Moonbirds | Ethereum | wMOONBIRD | 10 | ready |
| CloneX | Ethereum | wCLONEX | 26 | ready |
| Lil Pudgys | Ethereum | wLILPUDGY | 15 | ready |
| Checks VV | Ethereum | wCHECKSVV | 25 | ready |
| Identity MD | Ethereum | wIDENTITY | 5 | ready |
| Mad Lads | Solana | wMADLAD | 11 | ready |
| SMB Gen2 | Solana | wSMBGEN2 | 7 | ready |
| Claynosaurz | Solana | wCLAYNO | 13 | ready |
| Famous Fox Federation | Solana | wFFF | 27 | ready |
| DeGods | Solana | wDEGOD | 25 | ready |
| Galactic Geckos | Solana | wGECKO | 21 | ready |
| Taiyo Robotics | Solana | wTAIYO | 6 | ready |
| Okay Bears | Solana | wOKAYBEAR | 69 | ready |
| SMB Gen3 | Solana | wSMBGEN3 | 37 | ready |
| Transdimensional Fox Federation | Solana | wTFF | 71 | ready |
| Pythenians | Solana | wPYTHENIAN | 71 | ready |
| Cyber Frogs | Solana | wCYBERFROG | 65 | ready |
| Bitcoin Frogs | Bitcoin | wBFROG | 29 | ready |
| NodeMonkes | Bitcoin | wNODEMONKE | 5 | ready |
| Ordinal Maxi Biz | Bitcoin | wOMB | 7 | ready |
| Bitcoin Puppets | Bitcoin | wPUPPET | 12 | ready |
| Bitcoin Punks | Bitcoin | wBPUNK | 37 | ready |
| Natcats | Bitcoin | wNATCAT | 40 | ready |
| Runestone | Bitcoin | wRUNESTONE | 82 | ready |
| Bitcoin Weirdos | Bitcoin | wWEIRDO | 174 | ready |
| CENTS | Bitcoin | wCENTS | 22 | ready |
| RSIC Metaprotocol | Bitcoin | wRSIC | 152 | ready |
| Pizza Ninjas | Bitcoin | wPIZZA | 13 | ready |
| Quantum Cats | Bitcoin | wQCAT | 11 | ready |
| The Wizards of Ord | Bitcoin | wWOO | 21 | ready |
| Taproot Wizards | Bitcoin | wTAPWIZ | 5 | ready |
| Bitcoin Wizards | Bitcoin | wBWIZ | 9 | ready |
| StonkBrokers | Robinhood Chain | wSTONKBROK | 6 | coming |
| Chain Mancers | Robinhood Chain | wCHAINMANC | 14 | coming |
| Rekt Tradooors | Robinhood Chain | wREKTTRADO | 71 | coming |

"Coins graduate at" is in whole wrapped tokens of the collection. "Ready" collections open on their first coin; "coming" ones (the
Robinhood Chain ones) are added after the launch ceremony's check of the marketplace contract there.

## Where the NFTs are held

Every wrapped token in existence is backed either by an NFT the custodian holds, or by SOL in the collection's backing fund
counted at the floor price. The fund buys an NFT whenever it holds enough for one. The custodian holds each NFT on its own chain,
under keys split across Internet Computer nodes:

| Chain | Held at | Checked by |
|---|---|---|
| Ethereum | the custodian's EVM address | the owner of each token, read on Ethereum |
| Robinhood Chain | the same EVM address, on Robinhood Chain | the owner of each token, read on Robinhood Chain |
| Bitcoin (Ordinals) | the custodian's taproot address | each inscription's location, read from two indexers |
| Solana | the custodian's own Solana wallet | the wallet's token account for each NFT |

The [Reserves](#/reserves) page lists every item held with its custody proof, and an independent reconciler checks each one every
10 minutes with two data sources per chain.

## How a coin gets its backing

When a coin graduates, half its raise is set aside for backing: it is already in wrapped tokens, and it goes straight into a vault
that belongs to the coin, at the moment of graduation. Nothing has to be triggered and nobody can delay it. The vault is an
account only the custodian can move, and its balance is public.

## Burning coins for backing

Any holder can burn coins for their share of the coin's vault:

```
your wrapped tokens = vault balance × coins you burn ÷ coins in existence
```

"Coins in existence" includes the coins locked forever in the pool, so the share per coin is a little smaller than the vault divided
by the coins people hold. The coins you burn are gone. What you receive is wrapped tokens, which you can hold, add to, or redeem.

To burn, you open a **coin redemption intent** on the coin's page and send your coins to the deposit address it gives you. The
custodian pays your wrapped tokens to your wallet once the deposit is final, usually within a few minutes. The page follows every
step. See [Minting, burning and intents](#/docs/backing/minting-burning-and-intents) for how intents work.

## Redeeming a wrapped token for an NFT

With one whole wrapped token you can ask for a real NFT:

1. **Choose where it goes**, on the NFT's own chain:

   | Collection's chain | Destination |
   |---|---|
   | Ethereum | an Ethereum address (`0x…`) |
   | Robinhood Chain | an address on Robinhood Chain (`0x…`) |
   | Bitcoin (Ordinals) | a Bitcoin taproot address (starting `bc1p`) |
   | Solana | a Solana wallet address (a wallet, not a program account) |

   The custodian checks the destination when you open the intent, and it cannot be changed later. You can also choose to send the
   NFT to a burn address.
2. **Redeem.** You open a redemption intent and send exactly one whole wrapped token to its deposit address. The custodian burns it
   and opens a redemption ticket.
3. **The draw.** Your ticket is tied to a future round of drand, a public randomness service: one that does not exist yet when you
   redeem. When that round is published, the custodian checks its signature against two independent relays and assigns a
   **random NFT** from the collection's custody to your ticket. You cannot pick which NFT. Tickets are settled in order.
4. **Delivery.** The custodian sends the NFT to your address. It has **7 days** from the draw.

A redemption needs an NFT in custody: while most of a collection's wrapped tokens are still backed by SOL, redemptions wait for
purchases. Each collection's page shows how many NFTs are held and how many are being delivered.

If something goes wrong, the custodian refunds the ticket by minting your wrapped token back to you:

| Situation | When the refund happens |
|---|---|
| The draw never happens | 7 days after you redeemed, as long as the round's randomness is not yet public |
| The custodian marks the NFT undeliverable | 3 days after the delivery deadline |
| Nothing is sent at all | 30 days after the delivery deadline |

The token always goes back to the ticket's owner.


## The weekly holder raffle

Each collection has a raffle. Once a week, one whole wrapped token of that collection (a real NFT in custody) is drawn among the
holders of the collection's **graduated** coins. There is nothing to buy or sign up for: holding is the entry.

- **Your weight** is your share of the backing your coins carry. For each graduated coin of the collection:
  `your balance × the coin's wrapped tokens ÷ the coin's circulating supply` (rounded down), summed over the coins. The
  circulating supply leaves out the coin's pool vaults, accounts owned by programs (they cannot sign for a prize), burn addresses
  and the custodian's own accounts; those get no weight.
- **The pot** is paid by Rook's treasury: half of what a collection's share brings above its reserve cap (see [Fees](#/docs/fees)).
  When the pot can pay the public mint price of one whole wrapped token, it buys one: that is the next prize.
- **The draw.** The custodian takes a snapshot of every holder's weight at one recent Solana slot, **publishes it** (the coins,
  the excluded accounts, every holder's weight and the Merkle root) and then commits the root at a later slot. The randomness is a
  drand round pinned at the commit, which does not exist yet then, and the custodian verifies the round's signature when it
  arrives. Anyone can recompute the root from chain data and check the winner.
- **The prize** goes to the winner's wallet; the custodian sends it and pays for the token account. A prize not claimed within
  **30 days** stays in the raffle as the next round's prize.

> [!RISK]
> The custodian checks the winner against the snapshot it published itself. Publication before the draw is what keeps it honest:
> a snapshot that does not match chain data would be visible to anyone before the randomness existed.

## Minting, burning and intents

Anyone can mint wrapped tokens with SOL at the posted floor, or burn them back into SOL, with a small spread and hourly limits.
Both go through an **intent**, as do NFT and coin redemptions:

1. **Open an intent** on the collection's page (or the coin's page, for a coin redemption), naming the wallet that should receive
   the result. Opening is free and needs no signature. You get a deposit address that belongs to that intent alone, and a quote.
2. **Send** SOL (to mint, at least 0.05 SOL), wrapped tokens (to burn, at least 0.01; to redeem an NFT, exactly one) or coins
   (to redeem coins) to that address, from any wallet or exchange withdrawal. You can send more than once.
3. **Wait a few minutes.** Once your deposit is final on Solana, the custodian mints, burns or redeems at the price of that moment
   and pays your wallet. The page shows each step, with the transaction on Solana. It usually takes between half a minute and three.

Each operation costs a fee of 0.0015 SOL, taken from the deposit, on top of the spread. The spread runs from 0.15 % in calm
markets up to 10 % when the floor is stale, moving fast or thinly traded. Minting is limited per hour per collection: a deposit
over the hour's limit mints what fits and keeps the rest for the next hour.

If an operation cannot be done (a stale floor, a paused collection, a limit, a price bound you set), or is still waiting after
6 hours, the deposit is returned to the wallet you named, less the fee. An intent expires after 24 hours of no activity; anything
sent to it later is still returned for a week after that. A wallet can have at most 3 intents open at once, and a caller can open
5 an hour.
