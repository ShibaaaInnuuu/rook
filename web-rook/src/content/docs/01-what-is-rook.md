---
title: What is Rook
summary: A memecoin launchpad on Solana where every collection coin builds backing in real NFTs, from four chains.
group: Start here
---

Rook ([userook.fun](https://userook.fun)) is a memecoin launchpad on Solana. Every coin launched on Rook belongs to a well-known
NFT collection: Milady, Pudgy Penguins, Bored Apes, Mad Lads, Bitcoin Frogs and others, on Ethereum, Solana, Bitcoin and
Robinhood Chain. A collection coin trades against that collection's **wrapped token**, a Solana token where one whole unit stands
for one real NFT. When a coin graduates, half of what it raised is set aside in wrapped tokens as the coin's backing. Holders can
later burn their coins for a share of it, and turn a whole wrapped token into a real NFT, delivered to their own address on the
NFT's chain.

## In one minute

- **Launch a coin for a collection.** It trades on a bonding curve run by Meteora, a Solana exchange, priced in the collection's
  wrapped token (wMILADY, wMADLAD, wBFROG…). You can buy with SOL: the site turns SOL into wrapped tokens and coins in one
  transaction.
- **The people choose.** Rook doesn't pick which collections go first. Every collection Rook can already buy, hold and price is
  ready, and each one **opens the first time someone launches a coin on it**.
- **Graduation at the collection's threshold.** Each collection graduates when its curve has raised a set number of whole wrapped
  tokens, sized so that the raise is worth about 85 SOL at the collection's floor (more for the most expensive collections). See
  [Graduation thresholds](#/docs/how-it-works/graduation-thresholds).
- **Half goes to backing, at once.** 50 % of the raise goes straight into a vault that belongs to the coin, as wrapped tokens.
  The other half seeds a Meteora pool whose liquidity is locked forever.
- **Wrapped tokens are backed.** Every wrapped token in existence is backed either by an NFT Rook's custodian holds or by SOL set
  aside for that collection, counted at the floor price. The custodian buys NFTs with that SOL, one at a time, on the
  collection's own chain.
- **Holders can exit into the backing.** Burn coins for your share of your coin's wrapped tokens. With one whole wrapped token,
  redeem a random real NFT of the collection, delivered to your own wallet.
- **A weekly raffle.** Each collection draws one whole wrapped token a week among the holders of its graduated coins.

## Why it is different

On most launchpads nothing stands behind a coin once trading slows down. On Rook, every graduated collection coin can be burned
for its share of wrapped tokens, and those stand for real NFTs, so the backing acts as a floor under the coin. That floor is only
as good as the NFT prices behind it and the systems that hold them: read [Risks](#/docs/risks) before you buy.

## How Rook runs

Rook deploys no programs of its own on Solana. Everything Rook does on Solana is done by **the custodian**: a program on the
Internet Computer whose keys are threshold keys, so that no single person can sign for it. The custodian mints and burns the
wrapped tokens, keeps the books of every collection, buys and holds the NFTs on Ethereum, Robinhood Chain, Bitcoin and Solana,
and moves money only within the rules its code sets. What it does is checkable: every book is a public query, every
wrapped-token mint or burn carries a label on chain, and the [Reserves](#/reserves) page matches the books against the chains
every few minutes. Read [Security and trust](#/docs/security) for what that means and what you are trusting, including what the
audits found.

## The collections

41 collections on four chains. **38 are ready at launch**, and each opens the first time someone launches a coin on it (or asks
for it with **Open it**). Opening makes the collection's wrapped token and its coin settings on Solana. Rook pays for it and opens
at most 8 a day, in the order they are asked for.

- **Ethereum (11, ready):** Bored Ape Yacht Club, Milady, Pudgy Penguins, Azuki, Mutant Ape Yacht Club, Doodles, Lil Pudgys,
  Moonbirds, CloneX, Checks VV and Identity MD.
- **Solana (12, ready):** Mad Lads, SMB Gen2, Claynosaurz, Famous Fox Federation, DeGods, Galactic Geckos, Taiyo Robotics, Okay
  Bears, SMB Gen3, Transdimensional Fox Federation, Pythenians and Cyber Frogs.
- **Bitcoin, Ordinals (15, ready):** Bitcoin Frogs, Ordinal Maxi Biz, NodeMonkes, Bitcoin Puppets, Bitcoin Punks, Natcats,
  Runestone, Bitcoin Weirdos, CENTS, RSIC Metaprotocol, Pizza Ninjas, Quantum Cats, The Wizards of Ord, Taproot Wizards and
  Bitcoin Wizards.
- **Robinhood Chain (3, coming):** StonkBrokers, Chain Mancers and Rekt Tradooors, added after the launch ceremony has checked
  the marketplace contract the custodian buys through on Robinhood Chain.

Each has its own wrapped token, listed in [Backing and wrapped tokens](#/docs/backing/wrapped-tokens).

There is also **ROOK**, the genesis coin. It trades against SOL and has no NFT backing. Its trading fees pay for Rook's running
costs.

## Where to go next

- [How it works](#/docs/how-it-works): the whole walk-through, from launch to an NFT at your address.
- [Launching a coin](#/docs/launching): step by step, including opening a collection.
- [Fees](#/docs/fees): who earns what.
- [Security and trust](#/docs/security): who controls what, and the audits.
