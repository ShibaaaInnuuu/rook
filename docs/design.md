# etched — design, version 0

etched was developed under the working names OrdFloor and Etch; earlier commits, findings and documents use those names. Code identifiers keep the `etch` prefix on purpose.

Written 2026-09-21, before any contract. A memecoin launchpad on Robinhood Chain (chain id 4663) over Pons V2 in which a coin's
creator fees do one thing: buy Bitcoin assets — **inscriptions** from one ordinals collection, or units of one **rune** — and
hand them to the coin's holders. Not affiliated with Robinhood Markets, Pons, or Nosh. Unaudited, undeployed.

The mechanism follows a pattern that is live on this chain for EVM NFTs (launcher → per-coin fee router → per-collection vault →
raffle). What is different here is the one fact that decides everything else:

> **The assets are on Bitcoin. No contract on 4663 can buy one, hold one, or see one.**

So the vault cannot enforce "ETH leaves only if an NFT arrives", which is the rule that makes the EVM version trust-light. This
document is mostly about what replaces that rule, and it says plainly where a person's word stands in for a contract.

## The flow

1. **Launch.** A creator launches a coin on Pons V2 through our `Launcher` and picks a **listing**. The Launcher deploys a
   minimal-proxy `FeeRouter` for that coin and names it as the coin's Pons creator-fee recipient. The router has no owner, no
   upgrade path and no function that could hand the role on, so from the creator's side the pairing is permanent.
2. **Trade.** Pons credits the creator's share of trade fees (and any creator tax, and the creator's share of the launch-second
   snipe tax) to the router's balance in Pons's fee escrow — on the curve and, after graduation, from the pool's hook.
3. **Harvest.** Anyone calls `harvest()`. The router claims from escrow and forwards **80%** to the listing's vault, recorded
   against that coin, and **20%** to the treasury. The split is a constant in the bytecode.
4. **Acquire.** The keeper *declares* a purchase (what, how much, at most how much ETH), everyone can watch it for an hour, then
   the ETH is released to the purchaser, who buys on Bitcoin. A **different key**, the attestor, must post the receipt. Until it
   does, inside a deadline, the vault refuses to declare anything else.
5. **Distribute.** Each acquisition is charged to the coins whose fees paid for it, first in first out. The paying coin's creator
   chooses what happens to their share. For an inscription: hold, never deliver, a balance-weighted draw, a one-ticket-per-wallet
   draw, or a named recipient. For rune units: a pro-rata Merkle distribution to holders (the default), a draw for the whole lot,
   or hold.
6. **Deliver.** A winner or claimant posts a Bitcoin address; the keeper sends and records the txid; a late delivery is flagged
   on chain by anyone.

## Two kinds of listing, one vault contract

| | `Inscriptions` | `Rune` |
|---|---|---|
| What it is | a collection: a committed Merkle root over inscription ids | one rune: its id `block:tx`, its name, its divisibility |
| Why a commitment | an ordinals collection is off-chain metadata; "is this inscription in the collection" has no on-chain answer on any chain | not needed: a rune id is the asset |
| Cap the keeper posts (1 h TTL) | floor price in wei **per inscription** | price in wei **per whole rune unit** |
| A declaration is bounded by | `maxEth ≤ cap × count` | `maxEth ≤ cap × amount / 10^divisibility` |
| The receipt | bitcoin txid, inscription id (+ Merkle proof it is in the collection), sats paid | bitcoin txid, rune id, amount received, sats paid |
| Default distribution | a draw: one inscription, one winner | pro-rata to holders, by Merkle claim |

The membership root can be replaced only through a proposal that waits in public (7 days) — a root swapped quietly is how "the
floor of collection X" becomes "whatever was cheap".

## What the contracts enforce

- The 80/20 split, for every wei, for ever. No owner on a router. Nothing reaches the creator.
- A vault has **no withdraw function**. ETH leaves only by `release` of a declaration that (a) was made while a fresh cap was
  posted, (b) is within `cap × quantity`, (c) has waited out the public delay, (d) was not cancelled by the owner, and only to
  the purchaser address of record.
- At most `maxOpen` purchases may be unsettled. **One purchase overdue stops every further declaration** until the attestor
  settles it or the owner writes it off; a write-off is recorded with its amount, for ever.
- The attestor is never the purchaser: the same address cannot hold both roles, and role changes wait in public.
- For inscriptions, a receipt must prove the inscription id is in the committed collection.
- Per-coin accounting: every wei in is attributed to a coin; every acquisition is charged to coins FIFO.
- A draw's randomness is a **drand round that does not exist yet when it is pinned**, verified on chain by pairing check against
  drand's public key (see `docs/phase0.md` for why this is our own verifier). Nobody can pick it, nobody can re-roll it, anyone
  can deliver it, and an undelivered draw can be re-pinned by anyone after 24 h. A draw always settles.
- No draw or claim can start during the 15-minute challenge window after a snapshot root is published.
- A winner or claimant is the only one who can name the Bitcoin address their asset goes to.
- A rune claim under the posted minimum is not payable on its own: it accrues to the holder until it crosses the minimum.

## What is somebody's word

Stated here so that nobody finds it later.

1. **The purchase itself.** Once ETH is released the purchaser holds it. The contracts bound *how much* and *how often* and stop
   the machine if a receipt does not come; they cannot make the purchaser buy, or buy well. The worst case is bounded by
   `maxOpen × the largest declaration`, not by the vault's balance.
2. **The receipt.** The attestor's word that a txid delivered that inscription or that many rune units to the vault's Bitcoin
   address. Anyone can check it against the public Bitcoin address with any ordinals/runes indexer; nothing on 4663 can.
3. **The holder snapshot.** The Merkle root of tickets or of rune amounts is the keeper's word. The challenge window makes a bad
   root visible — the full list is published and anyone can recompute it — but no contract rejects one. The owner can cancel a
   draw or distribution inside the window; that is a person acting on a report, not a proof.
4. **Collection membership.** The root is the registry owner's word about which inscriptions are the collection.
5. **Custody on Bitcoin.** The vault's Bitcoin address is controlled by keys, not by a contract. Phase 0 recommends a 2-of-3
   taproot multisig and says what that does and does not change.
6. **The ETH → BTC leg.** Bridging cost and failure are real and outside the contracts.
7. **Pons's owner.** Pons V2 lets its protocol owner redirect any launch's creator-fee recipient after a 3-day public timelock.
   Our router cannot prevent that. It is visible on chain three days ahead.
8. **Delivery.** The keeper sends the asset on Bitcoin. A late delivery is flagged on chain; it is not forced.

## Roles

`owner` (a multisig): lists, pauses, cancels a pending declaration, cancels a draw or distribution inside its challenge window,
writes off an overdue purchase, proposes role and root changes that wait in public. **Cannot withdraw.**
`keeper`: posts caps and minimums, declares purchases, opens draws and distributions with a snapshot, records deliveries.
`purchaser`: receives released ETH. `attestor`: settles purchases with receipts; never the purchaser.
`anyone`: harvest, release a matured declaration, request a draw, deliver randomness, re-pin a stale draw, claim for a winner,
flag an overdue purchase or delivery.

## Not in this version

A web app, a keeper that buys, bridge code, anything deployed. BRC-20. Any claim that this is trustless.

## Amendment 1 (2026-09-21): what Phase 0 changed

- **Harvest sweeps first.** Pons keeps a curve's fees inside the curve until an entitled caller sweeps them; the router, as the
  creator-fee recipient, is entitled, so `harvest()` sweeps its own curve before it claims (`docs/phase0.md` §1a). After
  graduation the ETH-side pool fees can be swept by the router (`sweepPool`), the token-side half only by Pons's sweep operator.
  That dependency is added to "what is somebody's word" as item 9: **Pons's sweep operator, for half of post-graduation fees.**
- **The randomness is drand `evmnet`, verified by our own contract** with the ordinary BN254 pairing precompile. The open-looking
  oracle on this chain is allowlisted and owned; it is not used.
- **One Distributor, not two contracts**: draws and rune distributions share a contract so that an acquisition can never be both
  raffled and shared out.
- **Inscriptions bought in one purchase by several coins** go one each to the coin with the most paid-for value not yet matched;
  **rune units** are split between the paying coins in proportion, the last taking the rounding.

## Amendment 2 (2026-09-21): what Phase 1 changed

- **After graduation the economics hold, on somebody else's clock.** The router earns 0.7% + the creator tax of pool volume, in
  ETH, exactly as on the curve — measured on a fork and on eight real graduated coins with contract recipients (`docs/phase1.md`
  F1). But an ordinary buy's fee is charged in the coin, and while any coin-denominated fee is pending only Pons's sweep operator
  may sweep the pool. Item 9 of "what is somebody's word" is therefore not half of post-graduation fees but, in practice, **all of
  them, on the operator's schedule** — today a busy automated sweeper (median 84 s between sweeps of a pool).
- **`FeeRouter.forwardToken`.** The router is never handed the coin by a sweep; Pons's owner-only rescue is the one path that
  does. A stray token goes to the treasury, in the open. No swap on an ownerless contract: its caller would choose the minimum.
- **The purchaser's bond.** A release needs the vault's bond to cover `bondBps` (default 100%) of everything released and
  unsettled; a write-off pays the charged coins from it; anyone may write off 7 days past a deadline.
- **A snapshot anyone can recompute** (`docs/snapshot-v1.md`, `tools/snapshot/`), and **a keeper that cannot send**
  (`docs/keeper-dryrun-v1.md`, `keeper/`).
- **Venues:** UniSat first (both kinds buyable by PSBT API), Magic Eden for runes only (no documented endpoint buys an
  inscription), OKX none. Written permission from each before any key buys. The contracts are indifferent to how the purchaser
  buys, which is what makes a venue replaceable.
