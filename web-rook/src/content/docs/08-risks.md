---
title: Risks
summary: The plain list. Read it before you buy or launch.
group: Trust
---

> [!RISK]
> Nothing on Rook is financial advice. Memecoins are highly speculative. Only use money you can afford to lose entirely.

## Market

- **Prices can fall, fast and to near zero.** That goes for every coin, ROOK included.
- **NFT floor prices can fall too.** The backing is worth what the NFTs are worth, and a falling floor lowers it.
- **Most coins never graduate.** A coin that never reaches its collection's threshold gets no backing at all.
- **The backing is not a price guarantee.** Burning coins returns a share of wrapped tokens, not a fixed amount of SOL.
- **Shallow pools.** A collection's wrapped-token/SOL pool grows from fees. While it is shallow, buying with SOL costs more, and
  trading apps may not route the coin at all. The direct mint at the floor price is always there, but it takes minutes, not
  seconds.
- **Cheap collections need many wrapped tokens.** A collection with a low floor graduates at many whole wrapped tokens, and each of
  them is backed by SOL until the custodian has bought NFTs for it, a few a day at most.

## Timing

- **Buying NFTs takes time.** NFTs are bought one at a time as each collection's backing fund fills up, within custody limits.
  Until then a wrapped token is backed by SOL at the floor price, not by an NFT, and a redemption waits for a purchase.
- **Redemption takes time.** A random draw, then up to 7 days for delivery, and longer windows before a refund opens.
- **A collection opens on its first coin.** Each collection opens the first time someone asks, which takes a few minutes. The
  Ordinals collections beyond Bitcoin Frogs follow once their floor price feeds pass Rook's checks, and the Robinhood Chain ones
  once the launch ceremony's check there is done; there is no date for either.

## Control

- **The custodian keeps the books.** Rook has no programs on Solana. What backs a wrapped token is what the custodian's books say
  and what the Reserves page can verify, not something Solana's validators enforce. A bug in the custodian's code could mint or
  pay out wrongly; the labels on every transaction and the Reserves page would show it, not prevent it.
- **Pauses.** The operator, through its own guardian keys, can pause a collection, the direct mint, or everything. A pause lifts only 24 hours
  after a resume is asked.
- **A timelock, not a vote.** Changes to the custodian's code wait 72 hours in public. There is no way for holders to block one, and
  no outside guardian either: the keys that could cancel an upgrade are all the operator's. The 72 hours are your warning, and your
  time to burn or redeem.

## Technology

- **New code.** The custodian's Solana code is new. All of it has had internal audits, with fixes, the newest parts (Solana NFTs,
  the Ordinals beyond Bitcoin Frogs, Robinhood Chain NFTs, opening on demand) included. New code can hold bugs that nobody has found
  yet. See [The audits](#/docs/security/the-audits).
- **Dependencies.** Rook relies on Meteora (whose programs can be upgraded by Meteora), the Internet Computer, two Solana data
  providers, drand, and:
  - **Relay**, for every conversion of SOL into ETH (on Ethereum and on Robinhood Chain) and BTC. If Relay stalls or changes its
    terms, purchases on those chains wait.
  - **The marketplaces**: OpenSea (Ethereum and Robinhood Chain), Ordinals Wallet and UniSat (Bitcoin), Tensor and Magic Eden
    (Solana). Any of them can cut access, change fees or go down. On Solana most of the cheapest listings are on Magic Eden, which
    the custodian buys through its buyer-only path (on by default; the operator can switch it off at once). If Magic Eden cuts that
    path off, some Solana collections' purchases may not fill at the price the backing allows.
  - **The chains themselves**: Ethereum, Bitcoin, Solana, and Robinhood Chain.

  A failure in any of them can delay or block purchases, deliveries, mints or backing.
- **Robinhood Chain.** A young network, run by a single sequencer: if it stops, Robinhood Chain NFTs cannot be bought or
  delivered until it is back. The custodian buys there only through a marketplace contract whose code it has pinned: if that code
  changes, purchases stop until it is checked again.
- **Wrong addresses are final.** An NFT sent to an address you do not control cannot be recovered. Check the destination before you
  redeem, and make sure it is an address on the NFT's own chain. The same goes for the wallet you name when you open an intent.

