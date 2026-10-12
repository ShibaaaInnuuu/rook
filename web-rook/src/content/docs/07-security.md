---
title: Security and trust
summary: Who controls what, what you are trusting, and what the audits found.
group: Trust
---

## Who controls what

| Part | Controlled by | What that means for you |
|---|---|---|
| The custodian | A program on the Internet Computer, whose keys are threshold keys | No single person holds the keys that mint wrapped tokens, hold the NFTs on Ethereum, Robinhood Chain, Bitcoin and Solana, or sign its transactions. It acts only as its code says. Changes to that code wait behind a **72-hour timelock**, in public. |
| The operator | Rook | Adds collections (each checked by the custodian as it is pinned), sets money parameters (the treasury split, the reserve cap, the raffle share), each change waiting **24 hours** in public. Can pause a collection, the direct mint, or the whole custodian. Has no way to move money by hand, and cannot change a collection once it is pinned. |
| The guardian | Rook, on separate keys | Can only stop things (a collection, a pool, every mint) or cancel an upgrade, never move funds. A pause lifts only 24 hours after a resume is asked. There are no outside guardians: the guardian keys are the operator's own, kept on separate devices. |
| Anyone | You | Can ask for a pinned collection to open, open an intent, burn their own wrapped tokens, and read every book. |
| Meteora's programs | Meteora | Meteora can upgrade its bonding-curve and pool programs with one key. |
| Relay | Relay | Converts the custodian's SOL into ETH (on Ethereum or Robinhood Chain) or BTC to buy NFTs. |
| Marketplaces | OpenSea, Ordinals Wallet, UniSat, Tensor, Magic Eden | List the NFTs the custodian buys, and can cut access. They cannot make it pay more than the ask it checked. |

Rook deploys **no programs of its own** on Solana. The wrapped tokens are ordinary Solana tokens whose only minter is the
custodian, and the books that say what backs them are kept by the custodian, not enforced by Solana's validators. That is the
main thing to understand about Rook's trust model: it is not trustless. It is made checkable instead.

## How the custodian is protected

- **Threshold keys.** The custodian's Solana, Ethereum (the same key on Robinhood Chain) and Bitcoin keys are split across
  Internet Computer nodes. A transaction is signed only when the nodes jointly run the custodian's code.
- **Books before signatures.** Every mint, burn and payout is written to the books before it is signed, checked against what
  Solana shows at that moment, and undone if the transaction never lands. The custodian refuses to sign anything its own books
  do not cover.
- **Two providers must agree.** The custodian reads Solana through two independent data providers. If their answers differ, it
  does nothing and tries again later. It never guesses.
- **Checked purchases.** Before it signs a purchase on any chain, the custodian rebuilds the transaction itself and refuses
  anything but the exact item at the price it judged. On Robinhood Chain it buys only through the marketplace contract whose code
  it has pinned.
- **Bounded floor prices.** A floor can move by at most 20 % a day and goes stale after 3 hours.
- **Custody limits.** At launch the custodian holds at most 48 NFTs across all collections, with a cap for each collection.
- **Verified randomness.** Every drand round used for a redemption or a raffle is fetched from two relays that must agree, and
  its signature is verified by the custodian itself.
- **A separate key for Relay.** Each deposit into Relay is made from a dedicated key that holds only that deposit, so Relay's
  program can never reach the rest of the custodian's SOL.

## How you can check it

- **Public books.** Every collection's books (backing, reserve, floor, the NFTs held with their custody proofs, open tickets),
  every coin's vault, the treasury and every raffle snapshot are public queries of the custodian.
- **Labelled transactions.** Every wrapped-token mint or burn the custodian signs carries a label naming the book entry it is for.
  A mint of a wrapped token without one is the loudest alarm Rook has.
- **The Reserves page.** Every few minutes an independent checker reads the books and matches them against Solana (supply and
  balances) and against each NFT's chain (every NFT at the custody address on Ethereum, Robinhood Chain, Bitcoin or Solana). The
  [Reserves](#/reserves) page shows the result per collection, green or red.
- **Open source, reproducible.** The custodian's code is public, and the build that runs can be reproduced and compared with what
  is deployed.

## The audits

**October 2026, six parts.** All the code first written for Rook on Solana went through an audit in six parts (the money logic,
the registry and redemptions, coin backing and the Meteora integration, the custodian's Solana signing and data layer, its buying
and delivery flows, the launch tools). Every High or Critical finding needed a failing proof-of-concept test, and every finding
was checked by an independent verifier. It found nothing Critical; the three findings first rated High were lowered to Medium on
verification; all 14 Mediums were fixed.

**The custodian-only build, October 2026.** When Rook moved from Solana programs to the custodian keeping the books, the new code
was reviewed the same way, in a same-day review of the author's modules and three further parts (the intent desk and the books'
transaction links; the Solana primitives; the pool desk, the raffle snapshot and the operator surface):

| Severity | Findings | Status |
|---|---|---|
| Critical | 0 | — |
| High | 3 | All fixed. |
| Medium | 9 | All fixed. |
| Low | 15 | All fixed. |
| Informational | 16 | Recorded. |

Every fix has a test that fails without it.

**The newest code, October 2026.** The four parts written after that review were reviewed the same way: buying, holding and
delivering **Solana NFTs**; the **Ordinals collections beyond Bitcoin Frogs** (several collections sharing one custody); **Robinhood
Chain NFTs**; and **opening collections on demand**.

| Severity | Findings | Status |
|---|---|---|
| Critical | 0 | — |
| High | 6 | All fixed. |
| Medium | 11 | All fixed. |
| Low | 14 | All fixed. |
| Informational | 26 | Most fixed; the rest recorded. |

Two are limited rather than closed: the Solana NFT floor still comes from data Rook's own service hands in (a buy is capped at the
lowest floor of the last 7 days plus a premium), and the check of a new Ordinals floor against CoinGecko only catches a broken
feed, since CoinGecko reads the same marketplaces.


## What you are trusting

- **The custodian's code**, and the 72-hour timelock that controls changes to it.
- **The Internet Computer**: the subnet the custodian runs on, and the one that holds its keys.
- **Two data providers** not to agree on a wrong answer.
- **Meteora** not to break or change its programs in a way that strands backing. The custodian reads Meteora's current version
  and must be retested whenever Meteora releases a new one.
- **Relay** to complete conversions. If Relay stalls, NFT purchases wait.
- **NFT marketplaces and floor prices.** OpenSea, Ordinals Wallet, UniSat, Tensor and Magic Eden: the custodian buys near the floor
  it reads, and a marketplace that cuts access delays purchases.
- **The chains themselves**: Solana, Ethereum, Bitcoin and Robinhood Chain (a newer network run by one sequencer).
