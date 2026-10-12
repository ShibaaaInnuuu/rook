# Phase 3: an internal security pass

Written 2026-09-21, before any of its code or findings.

**This is not an audit, and nothing here may be described as one.** It is the work of the people who wrote the contracts trying
to break them. Its purpose is to make a paid audit cheaper and to make the obvious mistakes ours to find first. The contracts
stay "unaudited" in every document and on every page until somebody independent has been paid to attack them.

## What will be done, in order

1. `docs/security/threat-model.md` — every actor (owner, keeper, purchaser, attestor, coin creator, any caller, Pons's protocol
   owner, Pons's sweep operator, a malicious token, a malicious listing, a griefing claimant, drand failing or retiring, a reorg
   on 4663) against every contract: what they can take, block or degrade, and what stops them.
2. An adversarial read of every money path. Each real finding gets a severity, a proof-of-concept test that fails on the code as
   it stood, the fix, and a regression test. Findings are appended below as they are made, with the commit that fixed them.
   Things looked at and found sound are listed too, with the test that shows it.
3. Tools: slither, triaged line by line in `docs/security/slither-triage.md`; invariant campaigns with handlers for conservation
   of ETH, attribution = cost, claim-once, and "ETH leaves a vault only by a matured, bonded, capped release or a bond refund".
   A deep profile (numbers and runtime recorded) and a fast one for CI.
4. Gas at realistic sizes and a worst-case bound for every loop. Unbounded loops are refused by design.
5. What happens if a bug is found after launch: what can be paused, what cannot, how a vault is wound down.
6. `docs/security/audit-brief.md` — what an auditor needs on day one.
7. README, trust model and CI updated to match.

## Severity, as used here

- **critical**: funds can be taken or permanently locked by someone who should not be able to.
- **high**: funds can be locked or misdirected under conditions an attacker can arrange, or a trust assumption the documents
  state is false.
- **medium**: griefing with real cost, or a bound that is wrong in a way that matters at scale.
- **low**: a wrong edge case with no funds at risk. **info**: worth saying.

## Findings

Counts: **2 high, 3 medium, 5 low, 6 informational or accepted. No critical.** Every high, medium and low is fixed, each with a
regression test in `test/security/Regression.t.sol` (or where stated). The proofs of concept were run against the code as it stood
at `4ca871d`; their output is quoted.

### H1 — dust contributions make `release()` impossible: the vault can be locked for a few cents (fixed in `dcc8b80`)

The vault kept a FIFO queue with **one entry per deposit**, and `release()` walked it. Anyone can create a deposit: send 2 wei to
any coin's router and call its permissionless `harvest()`, and 1 wei lands in the queue. Proof of concept, 1,500 such entries ahead
of an honest 1 ETH:

```
gas for one release behind 1,500 dust entries: 14,081,877
gas per dust entry: 9,387
entries that make release impossible at 32M gas: 3,408
```

About 3,400 entries — a few hundred million gas in total, cents on this chain — and no declaration for more than a few thousand
wei can ever be released again. There is no other way out of a vault, so everything in it is stuck, and the attacker can repeat
it faster than the keeper can clear it (each clearing declaration waits an hour). **Fix:** the queue holds *coins*, each at most
once, entering when their unspent fees reach `MIN_ENTRY` (0.01 ETH); below that a coin's fees are `waiting` — attributed to it,
held, not yet spendable. A release is refused above `MAX_PAYERS` (64) coins (`TooManyPayers`; the owner cancels, the keeper declares
smaller; `payersFor(amount)` tells it beforehand). Stuffing the queue now costs 0.01 ETH of real fees per entry, which is then
spent on that coin's holders. Every loop in the vault is bounded by 64 or by `MAX_OPEN` (3). Cost: a coin whose lifetime fees
never reach 0.01 ETH never buys anything; FIFO is by coin, not by deposit, which changes when a coin's money is spent and never
whose holders it is spent for. Tests: `test_H1_*` (three).

### H2 — a purchase that buys less than it declared keeps the change, with an honest attestor (fixed in `dcc8b80`)

The cap bound a declaration (`maxEth ≤ cap × quantity`) and nothing bound the settlement. A purchaser released 10 ETH for 10
inscriptions could buy one; the attestor, truthfully, receipts one and closes the purchase with `settleShort`; `outstanding` falls
by the whole 10 ETH, the bond is free, and 9 ETH of fees has left for good with every role but one honest. Proof of concept:
`the bond is free, and 9 ETH of fees stayed with the purchaser` — passed. Runes had the same hole through `settleRune(amount)`.
**Fix:** a purchase closes only if what was kept is within the *declared* price per unit for what arrived:
`(maxEth − returned) × quantity ≤ maxEth × received`, else `ReturnUnspentFirst(shortfall)`. Until the shortfall is back the purchase
stays open, the bond stays held, and past the deadline it is written off against that bond. `quantity` is therefore a minimum the
purchaser is held to; the keeper's planner says so (`keeper/src/plan.ts`). More than declared is always accepted. Tests: `test_H2_*`.
What it still does not do: check that the attestor's count is true.

### M1 — the bond was one pool, so rotating the purchaser moved it (fixed in `dcc8b80`)

`withdrawBond` paid whoever was the registry's purchaser *now*, and `release` counted the whole pool. After a rotation the new
purchaser could be released ETH against the old one's bond, and could withdraw it; the old one could not. **Fix:** `bondOf` and
`outstandingOf` per purchaser. A release needs the *named* purchaser's own bond; a write-off slashes it; anyone holding a bond
withdraws their own, down to what is in their own hands, rotated out or not. `postBond()` credits the current purchaser.
Test: `test_M1_*`; the system invariant rotates purchasers at random.

### M2 — a draw was pinned 3–6 seconds ahead on a chain whose clock the sequencer sets (fixed in `30b7ea6`)

`PIN_AHEAD` was 2 rounds. `block.timestamp` on an L2 is the sequencer's; measured on 4663 it runs ~1.3 s behind the wall clock, and
nothing in the protocol stops it running further. Six seconds stale, and the pinned round's value is already public when the
transaction lands — and `requestDraw` is permissionless, so whoever is first chooses *when* to be first. **Fix:** 20 rounds, one
minute. A minute costs a draw nothing. The honest answer to "can a caller bias it": not by timing, unless the chain's clock is a
minute wrong, which is an outage; a caller still cannot choose the value, only drand's threshold can. The sequencer ordering two
`requestDraw`s in one block pins the same round for both. Test: `test_M2_*`; the local end-to-end run pinned a real round 57 s ahead.

### M3 — a ticket list with a gap strands the lot for ever (fixed in `30b7ea6`)

Nothing on chain checks that ticket ranges tile `[0, totalTickets)`. If the winning ticket falls in a gap nobody can ever prove a
winner and the lot stays `Resolved`; the asset is never delivered. **Fix:** `voidDraw`, owner only, no sooner than 7 days after
resolution (anyone can claim for a real winner, so a week unclaimed means the list was defective). It returns the lot to the keeper
for a new public snapshot and a new future round; it names nobody. It is also a second draw, which is why it is slow, public and
the owner's. Overlapping ranges are *not* fixed: the first valid claim wins, and the snapshot tool's deterministic output plus the
challenge window are the defence. Tests: `test_M3_*` (two).

### L1 — anyone could initialise a router before the Launcher did, and the implementation itself (fixed in `804bf12`)

Between cloning and `initialize` the Launcher calls Pons. No path was found that hands control to a third party there, but the
guard was only "not yet initialised". **Fix:** `launcher` is an immutable of the implementation (the deployer, i.e. the Launcher),
`initialize` requires it, and the implementation's constructor locks itself. Test: in `test/Launcher.t.sol`.

### L2 — `harvest()` paid the treasury before updating `treasuryOwed` (fixed in `804bf12`)

A re-entering treasury could not take anything (the books balance before it is called; the re-entrant call underflows or splits
zero), but effects now precede the call and a second entry is refused. Test: `test_L2_*`, with a treasury that tries.

### L3 — a winner could change the address after the keeper read it (fixed in `30b7ea6`)

**Fix:** corrections for one hour after the first address; `recordDelivery` only after that hour; the 7-day delivery clock runs
from the first address. Test: `test_L3_*`.

### L4 — `BtcAddress` accepted non-zero padding bits (fixed in `804bf12`)

A 62-character `bc1p…` address carries 260 bits for a 256-bit key; BIP-173 requires the spare four to be zero, and a conforming
wallet refuses otherwise. Only a deliberately built string passes the checksum with them set, so only its author is hurt.
Still true and said in the NatSpec: the library is a typo guard and must never be relied on for safety. It refuses upper-case and
segwit-v0 addresses that Bitcoin accepts. Test: `test_L4_*`.

### L5 — an off-curve signature burned all the gas it was sent with (fixed in `804bf12`)

A failing pairing precompile consumes everything forwarded: **1,040,430,373** gas in the test runner for one bad `verify`. Only the
caller pays, but `resolve` is what strangers are invited to call. **Fix:** `y² = x³ + 3` is checked first (a few hundred gas).
Test: `test_L5_*`, under 60,000 gas.

### Informational, and accepted

- **I1. `returnUnspent` is open to anyone.** A stranger returning 1 wei makes an exact full refund revert once. The purchaser
  retries with 1 wei less. Accepted: restricting it would stop a purchaser refunding from another wallet.
- **I2. After 24 h undelivered, whoever is awake chooses between the known old round (resolve) and a new one (`redraw`).** It needs
  everyone, the winner included, to have ignored a public signature for a day. No change removes it (expiring the old round gives
  the same choice a day earlier). Accepted; the keeper resolves within minutes and anyone else can.
- **I3. A creator who never calls `choose` leaves their coin's acquisitions undistributed for ever.** A default after N days is a
  product decision, not a fix. Listed for the owner in the audit brief.
- **I4. Forced ETH (selfdestruct) is never spendable.** By design; the invariant carries it as its own term.
- **I5. `carry` for a coin that never buys again on a listing stays unclaimed.** By design.
- **I6. The registry allows keeper = purchaser.** Only the attestor must differ. The deploy script enforces three distinct
  addresses; the registry does not, because the two are one operator either way.

### Attacked and held (each has a test named `test_held_*`, or the invariant named)

80/20 to the wei over 3,000 awkward harvests, never a wei stranded or minted; a refusing treasury never blocks the vault's share
(`invariant_split`); a reverting vault cannot brick `harvest` (the vault's `deposit` reverts only for a non-router or zero value,
and `harvest` sends neither); every illegal state transition; a cap that expires after declaration; force-fed ETH; an inner node
passed as a leaf (leaves are double-hashed, 32-byte preimage against a node's 64); a ticket leaf as a share leaf (96 against 64
bytes); a claim twice, a claim with a stretched range, a claim in the other holder's name; a late fulfilment racing a redraw; a
listing "swapped" under an open purchase (name, reference and vault are immutable; the membership root and the Bitcoin address
change only after 7 public days, and `inscriptionHeld` is keyed by leaf, not by root); a creator using somebody else's listing (any
listing is open to any coin by design, and a coin can only ever attribute to its own router); the router salt (it includes the
creator, so nobody can squat another's); the negated signature, a second encoding of the same point, the point at infinity, a
neighbour's signature, round zero (BN254's G1 has cofactor 1, so there is no small-subgroup point to try).

## Tools

**Slither 0.11.6**, 59 results, triaged line by line in `docs/security/slither-triage.md`: 0 real, 6 "high" all false positives
or the design itself, 11 medium all benign. **Invariants**: `test/security/System.invariant.t.sol` — conservation across
router → vault → treasury → purchaser → bond, vault outflow = releases + bond refunds, the books (attributed + waiting + bond +
forced = balance; per-purchaser bonds sum to the pool; each coin queued once), the 80/20 split, and claim-once. Fast profile
(CI): 64 runs × 64 deep. Deep profile (`FOUNDRY_PROFILE=deep`): 1,000 runs × 200 deep = 200,000 calls per campaign, 10,000 fuzz
runs; the whole non-fork suite takes **45 s** under it. Not vacuous: an invariant asserting that no release ever happens failed
within two calls. **Gas**: `docs/security/gas.md`.

## After launch: what can be stopped, and what cannot

`docs/security/recovery.md`. No sunset path was built, and why is written there.

## Interfaces changed in this phase (for the indexer)

No event was removed or changed. Added — AssetVault: `MIN_ENTRY`, `MAX_PAYERS`, `unspentOf(coin)`, `queued(coin)`, `waiting()`,
`payersFor(amount)`, `queueAt(i)`, `bondOf(purchaser)`, `outstandingOf(purchaser)`, `requiredBondOf(purchaser)`; errors
`TooManyPayers`, `ReturnUnspentFirst(uint256)`. `queueLength()` now counts coins, not deposits. Distributor: `PIN_AHEAD` is 20;
`voidDraw`, `resolvedAt(listing, acquisition)`, event `DrawVoided(listingId, acquisitionId, unclaimedTicket)`, error
`DestinationLocked`, constants `DESTINATION_CORRECTION`, `VOID_AFTER`; `lot.destinationAt` is now the FIRST address's time.
FeeRouter: `launcher()`, errors `NotLauncher`, `Reentered`. Storage: AssetVault's layout moved (`_purchases` is slot 17);
Distributor's did not.
