# Trust model

Who can do what, and what is left to somebody's word. Read with `docs/design.md`. Unaudited, undeployed.

## Actors and their powers

| Actor | Can | Cannot |
|---|---|---|
| **Owner** (meant to be a multisig) | list a collection or a rune; pause a listing (stops declarations and releases, not receipts); cancel a declaration before it is released; cancel a draw or a distribution during its 15-minute challenge window; write off a purchase whose receipt is overdue (recorded for ever, with the amount); void a resolved draw that nobody has been able to claim for 7 days, which sends it back for a new public snapshot and a new round — a second draw, and the owner's to answer for (docs/phase3.md M3); propose new roles, a new membership root, a new Bitcoin vault address, a new treasury — each waits 7 public days | withdraw from any vault (there is no such function); change the 80/20 split; re-point a coin's router; pick or block a draw once pinned, or re-roll one that anybody has claimed or could claim within a week; change a snapshot root after the window; name a winner's Bitcoin address; shorten any delay |
| **Keeper** | post a price cap (1 h TTL) and a rune delivery minimum (24 h TTL); declare a purchase within cap × quantity; publish a holder snapshot for a draw or a distribution; record a delivery's txid | move ETH (release goes to the *purchaser* address, an hour later, by anyone); settle its own declarations; exceed the cap; declare while a receipt is overdue or while three purchases are open; draw before the window ends; choose the randomness |
| **Purchaser** | receive released ETH and buy on Bitcoin; return unspent ETH; post a bond of their own, and withdraw the part of it nothing in their hands needs — still theirs if they are rotated out | be the attestor; receive more than was declared; receive anything early; receive anything their OWN bond does not cover (at the registry's `bondBps`, 100% by default; a predecessor's bond does not count); have a purchase closed that delivered fewer units than it declared until the ETH declared for the missing ones is back (docs/phase3.md H2); withdraw their bond while a receipt of theirs is overdue |
| **Attestor** | settle a released purchase with a Bitcoin receipt (txid, inscription id + collection proof, or rune amount, and sats paid) | be the purchaser or the keeper; settle an inscription that is not in the committed collection, or one already held; settle a purchase that was written off |
| **A coin's creator** | choose, once, what happens to each acquisition their coin paid for (hold can be revisited) | choose for another coin's acquisitions; receive any fee; change their coin's listing |
| **A winner / claimant** | name the taproot address their asset goes to, and correct it until it is sent | be named by anyone else |
| **Anyone** | `harvest` a router; `sweepPool`; `forwardToken` (a stray token in a router goes to the treasury); add to a vault's bond; write off a purchase whose receipt is 7 days past its deadline; `release` a matured declaration; `requestDraw`; deliver a drand signature; `redraw` a draw whose round is unproven after 24 h; submit a winner's or a holder's Merkle proof; roll unclaimed rune units over; flag a late delivery or payout; prove any drand round | anything above |
| **Pons's protocol owner** | redirect any launch's creator-fee recipient after Pons's own 3-day public timelock | be stopped by us. Fees would stop arriving at that coin's router; what is already in the vault is unaffected |
| **Pons's fee-sweep operator** | sweep a graduated pool's fees. While any coin-denominated fee is pending — which an ordinary buy creates — it is the **only** address that can; our router's `sweepPool` works only in the rare ETH-only state (`docs/phase1.md` F1, verified on a fork) | be replaced by us. Measured 2026-09-21: it swept 903 pools 18,115 times in 5.6 h, the same pool every 84 s at the median, 16 min at p90. If it stopped, post-graduation fees would wait in the hook |
| **The treasury** | receive its 20%, and any stray token `forwardToken` sends it | be made, by any contract, to sell such a token and return the ETH — that is its word |
| **A venue** (UniSat, Magic Eden) | refuse or end our API access at its discretion; change its API | touch anything on this chain. The contracts do not care how the purchaser buys, so a venue closing changes the keeper, not the protocol (`docs/phase1.md` F3) |

## Residual trust, in order of how much money it touches

1. **The purchase on Bitcoin.** Released ETH is in the purchaser's hands. The contracts bound the loss: at most `MAX_OPEN` (3)
   declarations outstanding, each ≤ a fresh cap × quantity, and the first overdue receipt stops everything until a person acts in
   public. They do not make the purchaser buy, buy at the floor, or buy at all. **The bond (Phase 1) changes the worst case:** a
   release is refused unless the vault's bond covers `bondBps` of everything released and unsettled, and a write-off pays the
   charged coins from it. At the default 100% a purchaser who takes `X` forfeits `X` and the coins are made whole; at `b`% the
   worst case is `(1 − b)` of three declarations' worth, once. The requirement rises at once and falls only after 7 public days,
   and after 7 days' silence past a deadline anyone — not only the owner — may write the purchase off. What the bond does not
   touch: a purchase at an inflated price from a friendly seller (the cap and the receipt bound that).
2. **The cap.** The keeper's word about a market this chain cannot read. A cap posted at ten times the floor permits a purchase at
   ten times the floor. It expires in an hour, it is an event, and every declaration carries a `capRef` saying where it came from.
   Nobody on chain checks it. The keeper dry-run (`keeper/`) shows the policy a cap should follow — the floor through the
   bridge's quote plus 5% — so a posted cap can be compared with what the policy would have posted from the same `capRef`.
3. **The receipt.** The attestor's word. For inscriptions the contract checks the id is in the committed collection and not already
   held; whether that inscription actually sits at the listing's Bitcoin address is checkable by anyone with an indexer and by no
   contract. **For runes, the amount received is wholly the attestor's word** — verifiable against the vault's public Bitcoin address
   with `ord --index-runes` or any runes indexer, and only there. A colluding purchaser and attestor can report a purchase that
   never happened; the separation of keys and the public Bitcoin address are what make that detectable, not impossible.
4. **The holder snapshot.** The keeper's word, for draws and for rune distributions alike. The list is published, the root is on
   chain, nothing can use it for 15 minutes, and the owner can cancel it in that window. No contract verifies balances: the coins
   trade on a curve and then in a v4 pool, and a snapshot at a block is an off-chain computation. A wrong root that nobody
   challenges in 15 minutes pays the wrong people. **Anyone can now recompute it** (`tools/snapshot/`, rules in
   `docs/snapshot-v1.md`): take `coin`, `block` and `kind` from the published file, run the tool against your own RPC, compare
   the root — same inputs, same file, byte for byte. The first real run showed why the window matters: Pons's liquidity locker
   was the largest "holder" until it was added to the exclusions. **What the tool cannot prove:** that the keeper used the block
   it promised beforehand, which is why the block must be named before it exists. For rune distributions the contract does enforce one thing: the sum paid out
   can never exceed what was actually acquired plus what rolled over.
5. **Collection membership.** The owner's word about which inscription ids are the collection; replaced only after 7 public days.
   UniSat's collection indexer answers "is this inscription in this collection" for anyone — an outside opinion to check the
   committed root against, and still one indexer's opinion.
6. **Custody on Bitcoin.** Keys, not contracts. 2-of-3 is recommended; it changes who can steal, not whether theft is possible.
7. **The ETH → BTC leg.** A bridge (LayerSwap via LI.FI was the route quoted). A failed or slow bridge is indistinguishable, on
   chain, from an overdue receipt, and is handled the same way.
8. **Delivery.** The keeper sends. Lateness is flaggable by anyone and visible for ever; it is not enforced.
9. **The venues.** Every usable venue reserves the right to cut us off, and both require written permission for commercial use
   of their APIs (`docs/phase1.md` F3). No key is used to buy before that answer exists, and the design depends on no single venue.
10. **drand.** A threshold of the League of Entropy could bias a round. The verifier cannot see that; nothing can.

## What is NOT trusted

The split, the pairing of a coin to a listing, the absence of a withdraw path, the per-declaration bound, the public hour, the
limit on open purchases, the stop on an overdue receipt, the separation of purchaser and attestor, the randomness once a round is
pinned, who may name a Bitcoin address, that a rune distribution cannot over-pay, that a release is covered by the bond and a write-off pays from it, and every delay. Each has a test that tries to
break it (`test/`), and the ETH-out rule and the split have invariant tests.

## Operational rules the contracts cannot enforce

- **A role address must be fresh and empty of code.** On Robinhood Chain the well-known development addresses carry EIP-7702
  delegations to a sweeper: ETH sent to them is taken in the same transaction (docs/phase2.md F2). The purchaser receives ETH, so a
  purchaser address anybody else has ever had the key to is a loss waiting for its first release. Check every role for code
  before naming it; the registry does not.

## What an internal security pass changed (Phase 3)

`docs/phase3.md` lists sixteen findings from our own attempt to break these contracts — two high, three medium, five low — with
their fixes; `docs/security/` holds the threat model per actor, the slither triage, the gas bounds, what can and cannot be done if
a bug is found after launch (there is no sunset function, and why), and a brief for an auditor. **It is not an audit.** Two things
it adds to the residual trust above: a coin whose lifetime fees never reach 0.01 ETH never buys anything (the price of a queue
nobody can stuff with dust), and ETH stranded in a vault by a bug stays stranded, because every exit we could build is a withdraw
function under another name.
