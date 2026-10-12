# v8: redemption shown as a draw of the vault's items and traits — design

Status: **design, before any code**, 2026-09-27, branch `ideas-v8` (from `nft-side` `a37f54b`). Nothing is built or deployed.
Read-only calls only: `tokenURI(1)` of the five collections on Ethereum, drand's and Arbitrum's public documentation.

**Legend.** **[M]** measured 2026-09-27. **[R]** read from this repository. **[A]** assumption. **[D]** public documentation, cited.

**For legal review before anything here is shown to users (§6).** The mechanism already exists (D2, drand-random redemption);
what is new is how it is **presented**. Presenting a random redemption as "a draw of rare traits" is the change most likely to
make a regulator read the product as a lottery or a paid loot box.

---

## 0. Summary

- **No contract change, no canister change.** `EthNftWrapper` already draws uniformly among the items free when the ticket was
  burned, with a drand round pinned 20 rounds (60 s) ahead, no redraw, strictly in ticket order [R, `EthNftWrapper.sol:306-360`].
  Everything here is **the site and the indexer** reading public state and publishing proofs.
- **What the holder sees before burning:** every item in the vault that the next ticket can draw, with its traits and a rarity
  rank; the probability of each item and of each trait value for **the ticket they are about to burn** (exact, including the
  tickets ahead of them in the queue); and the vault's history.
- **What everyone sees after:** each ticket's burn transaction, pinned round, the drand signature and randomness, the free set at
  resolution (read from the contract), the index the formula picks, the item, and a one-command re-computation.
- **Fairness holds** (§3): no one picks, no one can know the round's value in time, no redraw, and deposits or purchases change
  the odds **only for tickets burned after them**. The residual trust is the 4663 sequencer's timestamp latitude and drand's
  threshold (§3.4).
- **Build: ≈ 1 week** (indexer odds engine and proof endpoint, the site's page, a verifier script); review: a half-day read of the
  odds engine against the contract and the legal review of the wording.

---

## 1. Goal and why

- **Goal:** make the random redemption legible and checkable: the holder knows exactly what they might receive and with what
  probability, and anyone can prove after the fact that the item came from the formula and nobody chose it.
- **Why:** D2 chose random over FIFO because a queue lets anyone see the next item and redeem only when a better one is at the
  head (`phase1-nft.md` §2.2). Random is the fairer rule, but a black box invites suspicion ("the operator keeps the rare ones").
  Showing the composition and publishing each draw's proof turns the draw into evidence of honesty.
- **What it is not:** it does not make any item more likely, does not add a prize, and does not change the unit's value (one
  unit is always exactly one item of the collection).

---

## 2. Mechanism

### 2.1 The vault's items, traits and rarity

- **Items:** the wrapper's own books: `itemCount()`, `item(id)` → token id, how it arrived (`Wrapped` = bought, `Deposited` = a
  holder's deposit), and the ticket queue (`ticketCount()`, `ticket(i)`, `freeCount()`/`freeAt(i)`, `pendingCount()`/`pendingAt(i)`)
  [R, `EthNftWrapper.sol:436-446`].
- **Traits:** the collection's metadata by `tokenURI`. Measured [M]: Azuki, Pudgy Penguins and BAYC serve IPFS URIs
  (content-addressed, immutable); **Milady (`https://www.miladymaker.net/milady/json/1`) and MAYC
  (`https://boredapeyachtclub.com/api/mutants/1`) serve from the projects' own web servers**, which can change. So the indexer
  takes **one snapshot of the whole collection's metadata** at deploy, publishes it (R2, with its SHA-256 and the IPFS CID where
  there is one) and uses only the snapshot; a later change of a project's metadata is shown as a diff, never silently applied.
- **Rarity:** OpenRarity, the open standard OpenSea adopted (information content of traits, one-of-ones first) [D: OpenRarity],
  computed by the indexer from the snapshot with the published algorithm, so the rank is reproducible by anyone. The site shows
  the rank and each trait's frequency in the collection and in the vault.

### 2.2 The odds shown before burning (exact)

The contract's rule, restated [R]: ticket `t` draws from `_free` at the moment it resolves, which is every item whose pending
stamp is ≤ `t` (free when `t` was burned), less the items drawn by tickets ahead of it. Index = `keccak256(randomness, wrapper,
t) mod size`; the chosen item is swapped with the last and popped.

- **With no unresolved ticket ahead** (the common case: resolution is keyless and takes about a minute), the next ticket's set is
  exactly the current free set plus the pending items with a stamp ≤ its id, and **each item's probability is 1 / N**.
- **With k unresolved tickets ahead**, each of them removes one item uniformly from **its own** set (items stamped before it).
  The indexer computes the exact marginal probability for the new ticket by dynamic programming over those k draws (k is small:
  bounded by the redemption rate; Monte Carlo with 10⁶ samples and a stated error when k × N is large). An item deposited after
  those tickets but before ours cannot be taken by them, so it is **more** likely for us than 1 / N; the page shows it.
- **Per trait:** the probability that the drawn item has trait value v = the sum over items with v. The page shows, e.g.,
  "Background: Orange — 3 of 14 items — 21.4 %".
- **Stated with the odds:** they are for a burn **mined now**; items added later do not enter this ticket's set; an earlier
  ticket refunded later puts its item back with a **new** stamp, so it is drawable only by tickets burned after the refund
  [R, `refund` pushes `Pending(itemId, _tickets.length)`]. So the odds shown can only be wrong in one direction: an item shown may
  be taken first by a ticket ahead that resolves before ours — which the exact computation already accounts for.

### 2.3 Each draw's proof, published

Per ticket, the indexer publishes and the site renders:

| Field | Source |
|---|---|
| ticket id, owner, destination, burn transaction | `Redeemed` event |
| the pinned round | `ticket(id).round` = `roundAt(burn block timestamp) + 20` |
| the drand beacon's signature and randomness | drand **evmnet** (BN254, 3 s, chain hash `04f1e906…6ec8c3` [D]); `beacon.randomnessOf(round)` on 4663 once proven |
| the free set at resolution, in the contract's array order | `freeCount()` / `freeAt(i)` read at the block before the `Resolved` event, cross-checked against a replay of `Wrapped` / `Deposited` / `Resolved` / `Refunded` |
| the index `keccak256(randomness, wrapper, id) mod size` and the item and token it names | recomputed; must equal `Resolved(id, itemId, tokenId)` |
| the delivery | `Delivered` event and the Ethereum transfer (`proof-of-reserves.md`) |

**A verifier** (`scripts/verify-draw.mjs`, keyless, run by anyone): fetch the round from any drand relay, check its BLS signature
against evmnet's public key, recompute the randomness, read the free set at the block before resolution from any 4663 provider,
recompute the index, compare with the event. A mismatch is a SEV1 and is shown on the page.

### 2.4 What the page looks like (sketch)

- **Before burning:** "You will receive one of these 14 Milady Makers, chosen at random. Each is equally likely (1 in 14)." A
  grid of the items (image, token id, rarity rank), sortable, with trait filters showing counts and percentages. Beneath: "The
  item is picked by a public randomness beacon about a minute after your burn. No one, including Rook, can choose or know it in
  advance. How it works · Verify past draws."
- **After:** the ticket's timeline (burned → round → resolved → delivered) with each proof link.
- **Words not used** (§6): win, prize, jackpot, lucky, pull, rare drop, mystery, spin; no animations, no countdowns, no
  leaderboards of "best pulls".

---

## 3. Fairness

### 3.1 No one can pick

- The draw is a pure function of the round's randomness, the wrapper address, the ticket id and the free set; `resolve` is
  keyless and callable by anyone, strictly in ticket order (`OutOfOrder`) [R].
- **No redraw** (audit C-02): a redeemer who has seen the round cannot discard it [R].
- The canister and the operator have no input: the canister delivers the item the contract names (`judge_transfer` reads it
  from the ticket) [R, `phase1-nft.md` §4.4].

### 3.2 No one can predict in time

- The round is fixed at the burn to `roundAt(block.timestamp) + 20`: a round 60 s in the future, not yet produced. drand's value
  is a threshold BLS signature of the League of Entropy; nobody below the threshold can compute it early [D: drand].
- **Timing the burn** (choosing the moment) chooses which future round, never its value. Front-running someone's burn gives the
  front-runner the same kind of ticket over the same set; no advantage.
- **Choosing when to resolve** changes nothing: the randomness and the set are fixed.

### 3.3 Additions and removals change odds only prospectively

- A deposit or a purchase mints an item with a pending stamp = the next ticket id: only tickets burned after it can draw it [R].
- A refunded item returns with a new stamp (§2.2).
- A delivery removes a drawn item that was already no longer free.
- So a depositor cannot place a rare item and steer it to a ticket already burned, and a holder cannot burn "into" an item that
  arrives after them.

### 3.4 The residual trust

- **drand's threshold** (the League of Entropy): the same trust as the existing design; unchanged.
- **The 4663 sequencer's clock.** Robinhood Chain is an Arbitrum Nitro chain [R]; Nitro lets the sequencer set a block's
  timestamp **up to 24 hours earlier** than the current time or 1 hour later [D: Arbitrum, block numbers and time]. A
  malicious sequencer could back-date a burn's block so that `roundAt(timestamp) + 20` names a round **already published**, and so
  include a burn only when that known round favours it. This is not new (the Bitcoin `Wrapper` has the same pin), but a page that
  invites people to chase rare items raises the stakes. Mitigations, in order of cost: (a) the verifier flags any ticket whose
  pinned round was published before the burn block was **posted to Ethereum** (L1 batch time), which a back-date cannot hide; (b) a
  contract change (not proposed here) that pins from the later of the block timestamp and the latest L1 block time the chain
  exposes.
- **Metadata:** rarity is only as honest as the snapshot (§2.1); the snapshot is hashed and published before the first draw.

### 3.5 Honest expectations

- **The vault will mostly hold floor items.** Purchases buy the cheapest verified listings (and, with `bids.md`, collection offers:
  a seller's least wanted token); rational depositors deposit their least valuable items (a unit is worth about the floor). Rare
  traits reach the vault by chance, not design. The page should say so: "the vault's items are mostly bought at the floor".
- **Adverse selection is the reason not to over-sell the draw**: if the vault ever holds a visibly valuable item, it will be
  redeemed quickly by whoever computes the odds best; that is fair (everyone sees the same odds), and it is also exactly what makes
  the mechanic look like a game.

---

## 4. Trust and security model

- **What the canister must judge:** nothing new. It delivers the item the ticket names, as today.
- **What can go wrong:**
  - the odds engine is wrong (shows odds the contract does not implement) → misleading users; mitigated by the verifier on past
    draws and a test suite that replays the contract's own `resolve` on a fork for random histories;
  - a metadata server changes traits (Milady, MAYC) → the snapshot rule;
  - the indexer is down → the page shows "odds unavailable", never stale odds as current;
  - phishing: a fake "draw" page → the site links only from its own domain; nothing here asks for a signature beyond the
    wrapper's own `redeem`.
- **New attack surface:** none on chain. Off chain: the indexer's odds endpoint and the metadata snapshot.

## 5. Interaction with the existing invariants

- 1:1 backing, per-collection isolation, caps and the timelock: untouched (read-only feature).
- The redemption rules (D2, no redraw, the gated refund, `redeemToRelist` refused) are unchanged and restated on the page.
- The proof-of-reserves page (`proof-of-reserves.md`) and this page share the indexer's reads; the draw proofs link to the
  delivery's custody proof.

## 6. Regulatory framing — flag for legal review

The repository's memo (`docs/legal/prize-draw-memo.md`, 2026-09-21) sets out the three elements of a lottery in most systems:
**prize, chance, consideration**. For redemption:

- **Chance:** yes — which item is drawn.
- **Consideration:** yes on the broad tests — the holder burns a unit they paid for.
- **Prize:** the arguable element. The holder always receives one NFT of the collection, the thing the unit is a claim on; the
  chance is only over *which*. With every item at about the floor, the variance is small and the argument that there is no prize
  (only a fungible claim settled in kind) is strongest. **Showing rarity and inviting holders to chase rare traits argues the other
  way**: it creates items worth more than the unit and markets the chance of getting them. That is the structure of a **paid loot
  box**, which Belgium's Gaming Commission classed as illegal gambling in 2018 when paid for with real money [D: Belgian Gaming
  Commission report, 2018], and which other jurisdictions regulate or require odds disclosure for.
- **Levers (for counsel):**
  1. **Composition, not prizes.** Show the vault as an inventory with equal odds per item; show rarity only on the proof-of-reserves
     inventory, not on the redeem flow. Lowest risk, keeps most of the transparency.
  2. **No marketing of outcomes** (memo §4): no "win", no highlight reels of rare redemptions, no social sharing of pulls.
  3. **A non-random path** (a later product change, not proposed): redeem a *chosen* item for the unit plus a premium priced by a
     published rule. It removes chance for anyone who wants certainty; it needs a pricing rule and a contract change.
  4. **Geo-restriction of the front end** (cosmetic, memo §4) if counsel asks.
- **Questions for counsel:** is in-kind redemption of a fungible claim, with the specific item chosen at random, a game of chance
  in the target jurisdictions? Does showing rarity and odds change that? Which wording and which lever is acceptable?

**Recommendation:** build the transparent composition and proof pages (lever 1) now, since they are also the honest answer to
"which items are in the vault"; hold the "rare traits draw" framing until counsel has answered.

## 7. Phasing

1. **Now (site and indexer, no dependency on the canister's fixes):** the metadata snapshot and rarity, the proof endpoint and the
   verifier, the per-ticket timeline. They can run against the dry run's fork and fixtures.
2. **Before launch:** the composition view on the redeem page (lever 1 wording), reviewed by counsel.
3. **After counsel:** any trait-odds framing counsel accepts.

## 8. Open decisions for the operator

- **R1.** The framing: composition with equal odds (recommended) vs trait odds on the redeem flow (after counsel).
- **R2.** Whether to show rarity ranks at all on the redeem page (recommended: only on the inventory).
- **R3.** Whether to fund legal review of this specific mechanic before launch (recommended: yes; it is the one consumer-facing
  chance element at launch).
- **R4.** The sequencer-timestamp flag (§3.4 a): build it into the verifier (recommended), and whether to propose the contract
  change (b) for a later wrapper version.

## 9. Build and review estimate [A]

- Indexer: metadata snapshot job (IPFS and HTTP, hashed), OpenRarity port or library, the odds engine (DP + Monte Carlo), the
  per-ticket proof endpoint: ≈ 3 days.
- Site: the inventory and odds view, the ticket timeline: ≈ 2 days.
- Verifier script and a fork test that replays `resolve` for random histories against the odds engine: ≈ 1–2 days.
- Review: half a day (the odds engine against the contract); legal review separately.

## Sources

- drand: [quicknet / evmnet networks](https://docs.drand.love/developer/), [evmnet on the EVM](https://docs.drand.love/blog/2025/08/26/verifying-bls12-on-ethereum/).
- Arbitrum: [Block numbers and time](https://docs.arbitrum.io/for-devs/concepts/differences-between-arbitrum-ethereum/block-numbers-and-time) (the 24 h / 1 h timestamp bounds).
- Rarity: [OpenRarity](https://www.openrarity.dev/), [OpenSea help: What is OpenRarity](https://support.opensea.io/en/articles/8867099-what-is-openrarity).
- Loot boxes: [Belgium Gaming Commission report, 2018 (summary)](https://www.mygamecounsel.com/2018/05/articles/gambling/belgium-gaming-commission-loot-box-report/),
  [UK Gambling Commission ABSG: international approaches](https://www.gamblingcommission.gov.uk/manual/lootboxes-advice-to-the-gambling-commission-from-absg/international-approaches-lootboxes-advice-to-the-gambling-commission-from).
- Repository: `src/nft/EthNftWrapper.sol`, `src/rand/DrandEvmnet.sol` (`PERIOD` 3, `roundAt`), `docs/legal/prize-draw-memo.md`.
