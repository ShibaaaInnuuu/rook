---
title: Glossary
summary: The words these docs use, in one line each.
group: Start here
---

| Term | Meaning |
|---|---|
| Backing | The wrapped tokens held for a graduated coin, and behind them the NFTs (and SOL waiting to buy NFTs) of the collection. |
| Backing fund | A collection's SOL set aside by the custodian from wrapped-token mints, used to buy NFTs. |
| Backing vault | The wrapped tokens held for one graduated coin. Holders burn coins to take their share. |
| Bonding curve | The automatic price curve a new coin trades on until it graduates. The price rises as people buy. |
| Collection coin | A coin launched on Rook for one NFT collection, priced in that collection's wrapped token. Half of its raise becomes its backing. |
| Coming | A collection with a page but not yet added: the Ordinals beyond Bitcoin Frogs until their floor feeds pass Rook's checks, and the Robinhood Chain collections until the launch ceremony's check there. |
| Custodian | Rook's program on the Internet Computer that mints wrapped tokens, keeps the books, buys, holds and delivers the NFTs. Its keys are threshold keys. |
| Custody address | Where the custodian holds a collection's NFTs, on the NFT's own chain: its EVM address (Ethereum, Robinhood Chain), its taproot address (Bitcoin) or its own wallet (Solana). |
| Dormant | An Ordinals collection added to the custodian but asleep: no floor posted, no wrapped tokens minted, until its floor feed passes the check and it is woken. |
| drand | A public randomness service. Its rounds decide which NFT each redemption receives and who wins each raffle. |
| ERC-6551 (token-bound) | A standard that makes each NFT also an account that can hold tokens. Rook prices and buys such NFTs as NFTs only. |
| Floor price | The cheapest price at which an NFT of a collection is listed for sale, kept by the custodian in SOL. |
| Graduation | The moment a coin's curve reaches its collection's threshold and the coin moves to a Meteora pool. |
| Graduation threshold | The whole wrapped tokens a collection's coins raise before they graduate: at least 3, sized so the raise is worth about 85 SOL or more. ROOK's is 85 SOL. |
| Guardian | A key that can only pause things, never move funds. |
| Intent | A request to the custodian to mint, burn or redeem: it gives you a deposit address of its own, and the custodian acts once your deposit is final, or returns it. |
| Locked liquidity | The coins and wrapped tokens in a graduated coin's pool, which nobody can ever withdraw. |
| Meteora | The Solana exchange that runs Rook's bonding curves, graduated pools and wrapped-token/SOL pools. |
| Open on demand | How a collection starts: it is ready (pinned) at launch, and opens, making its wrapped token and coin settings, the first time someone asks. |
| Operator | Rook's team. Adds collections, sets money parameters with a 24-hour delay and can pause; cannot move funds by hand. |
| Pinned | A collection the custodian has been set up for, ready to open on its first coin. |
| Pool desk | The custodian's wrapped-token/SOL pools on Meteora, one per open collection, grown from fees and kept near the floor. |
| Redemption | Burning one whole wrapped token to receive a real NFT at your address on its chain. |
| Relay | The cross-chain service the custodian uses to turn SOL into ETH (on Ethereum or Robinhood Chain) or BTC. |
| Reserves page | The site's page that matches the custodian's books against Solana and the NFTs' chains. |
| Robinhood Chain | A newer Ethereum-compatible network. Three of Rook's collections live there; the custodian buys them on OpenSea and holds them at its EVM address. |
| ROOK | Rook's genesis coin. Priced in SOL, no backing. Its fees pay Rook's running costs. |
| Threshold key | A key split across many computers, so that no single person or machine can sign alone. |
| Ticket | The record of one redemption, from the draw to delivery or refund. |
| Timelock | The 72-hour public wait before any change to the custodian's code takes effect. |
| Wrapped token | A Solana token for one collection (wMILADY, wMADLAD, wBFROG…). One whole token stands for one NFT. Only the custodian mints it. |
