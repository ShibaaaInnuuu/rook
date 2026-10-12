# Internal review v1

**2026-09-22.** Against `84c7d8f` (the head of `main` when this was written). Solidity 0.8.28, via-IR, optimizer 200, EVM
`cancun`, OpenZeppelin 5. Target chain: Robinhood Chain (4663), an Arbitrum Nitro L2.

---

## Read this before you read anything else

**This is a self-review. It was produced by the same party that produced the code, and it is not a substitute for an
independent audit.**

`docs/security/audit-brief.md` says this already, and says it correctly: the brief "was written by the people who wrote the
contracts, after their own security pass (docs/phase3.md), which is not an audit and found two high-severity defects in its own
code — take that as the prior for what is left." This document is the *next* pass by the same party. It found more, including
one defect that permanently destroys a vault for one wei. That should not be read as reassurance. It should be read as
evidence that the prior in the brief was correct and is still correct.

An internal review cannot find the class of bug that comes from an assumption shared with the code. If the author believed
something while writing `release()` and still believes it while reviewing `release()`, the review will not see it — not because
the reviewer is careless but because there is nothing in the process that contradicts the belief. Every finding below was found
by questioning an assumption that happened to be written down somewhere (in a doc, a NatSpec comment, or a competitor's source
code). The assumptions that are *not* written down anywhere are exactly the ones still standing, and this document cannot tell
you what they are. That is what an independent reviewer is for, and there is no internal substitute for one.

Two concrete illustrations of the limit, from this pass:

- **C1 below is a critical, permanent, one-wei loss-of-funds defect that the existing invariant campaigns cannot detect by
  construction.** Every call in `test/security/System.invariant.t.sol`'s handler is wrapped in `try … catch {}`. A panic in
  `writeOff` is swallowed and recorded as "that call wasn't valid in this state"; the books then balance, because nothing
  happened. The campaigns assert *safety* (no wei is created, no wei leaves wrongly) and never once assert *liveness* (a
  transition that must be reachable is reachable). In a system whose central promise is "there is no withdraw function",
  liveness **is** the safety property, and it was never tested. That gap was invisible from the inside for two passes.
- **The single most useful input to this review was somebody else's source code.** The `sweepPool` slippage question, the
  never-expiring declaration (M7), the router's open `receive()` (M8) and the minimum-share rule on attribution (L5) came from
  reading Nosh's verified contracts and the NatSpec comments in which they document the exploits that forced each change.
  Those are four findings that came from outside the building, in a review that was otherwise a careful re-read of code its
  own author already understood.

**Status, 2026-09-22.** C1 is **fixed** (see the closing note under C1). Everything else in this document is open: `src/` is
otherwise unedited. H1, H2 and the bond-specific mediums (M1, M4, M6) live inside machinery the v2 rework is rewriting, so
they are deferred to it deliberately rather than patched twice. The reproductions live in
`test/security/InternalReviewV1.t.sol`: the open ones are named `test_repro_*` and every one of them **asserts the defect**,
not its absence — if a finding is closed, its test must be deleted or inverted, never left passing. C1's three are now named
`test_C1*_fixed_*` and assert the fix, scenario for scenario, plus a fuzz over arbitrary instalment splits.

---

## Summary

**1 critical, 2 high, 7 medium, 9 low, 5 informational.**

| # | Sev | Where | One line |
|---|---|---|---|
| **C1** | critical | `AssetVault.returnUnspent` / `writeOff` | One wei from any stranger makes `writeOff` panic, and a vault with no withdraw function is then bricked for ever — **FIXED 2026-09-22** |
| **H1** | high | `AssetVault.release` | Any caller picks the order of two matured releases; one order makes the other permanently unreleasable |
| **H2** | high | `ListingRegistry` | `setPaused` + `renounceOwnership`, both instant, are a one-way door that freezes every vault for ever |
| **M1** | medium | `ListingRegistry.proposeRoles` | A rotation may make the keeper the purchaser; only the deploy script checks, and only for the first set |
| **M2** | medium | `Distributor.postMinimum` | The keeper can block every rune payout for ever with an unbounded minimum; no override exists |
| **M3** | medium | `AssetVault._credit` | Refunded change below `MIN_ENTRY` is attributed to a coin and can never be spent |
| **M4** | medium | `AssetVault.postBond` | After a rotation the named purchaser's bond cannot be topped up; their declaration is stranded |
| **M5** | medium | `Distributor` | A lot pinned to a beacon that stops producing can never leave `Pinned`; `voidDraw` starts from `Resolved` |
| **M6** | medium | `AssetVault.requiredBondOf` | Lowering `bondBps` retroactively un-bonds a release already in the purchaser's hands |
| **M7** | medium | `AssetVault.release` | A declaration never expires: the keeper can pre-arm three and fire them a year later, sterilising the vault meanwhile |
| **M8** | medium | `FeeRouter.receive` | A queue slot costs a direct donation plus a launch fee, not "0.01 ETH of real fees", and the ETH comes back as a `Named` lot |
| **M9** | medium | `Distributor.redraw` | Permissionless, and gated on "unproven" rather than "unavailable": a free, repeatable selective abort on the randomness |
| **L1** | low | `AssetVault.isBlocked` | An overdue receipt stops new declarations but not a matured release |
| **L2** | low | `FeeRouter.forwardToken` | Sends ERC-20s, permissionlessly, to a `Treasury` that has no function able to move them |
| **L3** | low | `Launcher.launch` | `CoinInfo.exists` is written and never read; a duplicate coin address would silently rewrite `creatorOf` |
| **L4** | low | `FeeRouter.sweepPool` | Hardcoded `0, 0` slippage is safe only under the current Pons hook, and the router cannot be fixed |
| **L5** | low | `AssetVault.settleInscription` | Greedy largest-gap attribution has no minimum-share rule and no carry between purchases |
| **L6** | low | `AssetVault.DECLARE_DELAY` | The public hour rests on the same sequencer clock that M2 of phase 3 declared untrustworthy |
| **L7** | low | `BtcAddress` / `setDestination` | Upper-case addresses Bitcoin accepts are refused, and a wrong destination locks after one hour with no recovery |
| **L8** | low | `AssetVault.postBond` | Before roles are set, a bond credits `bondOf[address(0)]` and is unrecoverable |
| **L9** | low | `Distributor.claimShare` | A root that over-promises makes honest holders race, and the losers have no recourse after 15 minutes |
| I1–I5 | info | various | See [Informational](#informational) |

---

## Critical

### C1 — one wei from any stranger bricks a vault for ever

**Severity: critical.** Funds are permanently and irrecoverably locked by someone with no role, at a cost of one wei.

**Where.** `AssetVault.returnUnspent` (lines 356–362) and `AssetVault.writeOff` (lines 257–263), both against
`chargedTo[coin] -= share`.

**The defect.** `release` splits `maxEth` across the charge list **exactly** — each payer's `weiCharged` is what it actually
paid, and they sum to `maxEth`. Both places that later *decrement* `chargedTo` split with a different rule:

```solidity
uint256 share = i == cs.length - 1 ? msg.value - given : (msg.value * cs[i].weiCharged) / p.maxEth;
```

The last payer takes the remainder. Every truncated division in the loop above it is added to the last payer's share. So the
last payer's cumulative decrement exceeds its `weiCharged` as soon as `returned + slash` reaches `maxEth` across more than one
call — and it is unsigned subtraction, so it does not misreport, it **panics**.

At the default `bondBps` of 10 000, `slash = min(bondOf, maxEth − returned) = maxEth − returned`, so `returned + slash` equals
`maxEth` **exactly, on every write-off**. The default setting — the one the trust model calls the safe one — is the setting
that guarantees the panic.

**What an attacker does.** Waits for any release with two or more payers, which is the ordinary case on any listing with more
than one coin, and calls `returnUnspent{value: 1}(id)`. That is the whole attack. `returnUnspent` is deliberately open to
anyone (accepted as I1 in phase 3: "a stranger returning 1 wei makes an exact full refund revert once. The purchaser retries
with 1 wei less"). That triage is wrong: it looked at the `p.returned + msg.value > p.maxEth` check and missed the `chargedTo`
subtraction underneath it.

With two payers at 0.6 / 0.4 ETH of a 1 ETH purchase, the 1-wei return gives the last payer **100%** of it (the other payer's
proportional share, `1 × 6e17 / 1e18`, truncates to zero). `chargedTo[coinB]` is now `0.4 ether − 1`. Three days later the
write-off computes coinB's share of the slash as exactly `0.4 ether`, and `0.4 ether − 1 -= 0.4 ether` panics.

**What it costs them.** One wei and one transaction's gas. No role, no capital, no timing.

**What it gains them.** Nothing — this is pure destruction, which is the point. `writeOff` is the only exit from `Released`
when nothing was delivered. Once it panics:

- `isBlocked()` returns true for ever, so `declarePurchase` reverts for ever;
- `cancel` is for `Declared` and refuses;
- `settleShort` needs `settledQuantity != 0` and refuses;
- `harvest()` is permissionless and keeps depositing fees into the vault, which can never spend them;
- `AssetVault` has no withdraw function, by design (`docs/security/recovery.md`), so no key can recover any of it.

`test_repro_C1_oneWeiFromAStrangerMakesWriteOffPanicAndBricksTheVaultForEver` ends with 50 ETH attributed to a live coin and
permanently unspendable.

**It also fires with nobody attacking.** `test_repro_C1b` shows a purchaser returning change in two instalments: the second
reverts, because the first truncated. The precise condition is that *some* instalment's split truncates — a split into amounts
that happen to divide the charges exactly is fine, which is why round numbers in a test hide it. Real refunds are not round
numbers. The honest path the design most wants (buy under the cap, return the difference) is the path that triggers it.

**The only way out, and what it costs.** `settleInscription` never touches `chargedTo`, so the purchase can still be closed by
receipting the full *declared* quantity. When the purchaser has bought nothing, that means the attestor signing a false
receipt against real member inscription ids, which are then `inscriptionHeld` for ever and can never be bought again — and the
purchaser walks away with the whole release *and* the whole bond, with `totalWrittenOff` reading zero, because on chain nothing
went wrong. `test_repro_C1c` is that sequence. Incident response for a vault bricked by one wei is "have the attestor lie",
which is the exact act the separation of keys exists to make detectable.

**Smallest change that closes it.** Drop the subtraction. `chargedTo` is a bookkeeping view — nothing in the money path reads
it — so the minimal fix is to stop decrementing it and expose the net figure as `chargedTo[coin] − creditedBack[coin]`, or to
clamp: `uint256 c = chargedTo[coin]; chargedTo[coin] = c > share ? c - share : 0;`. The clamp is one line and cannot panic.

The deeper fix, which is the one worth making, is to stop giving the remainder to the last payer in a running total. Give the
remainder to the payer whose `weiCharged` is *largest* (it can always absorb it), or track the dust in a single vault-level
counter. But the clamp closes the critical path today.

**And the general rule this violates.** Nosh writes it down in their own source and audits against it: because the vault has no
withdraw, *any* call that must succeed and can be made to revert is a permanent fund freeze, not a griefing bug. Etch has the
same no-withdraw design and no such rule. Every required transition — `writeOff`, `release`, `settleShort` — should have a test
that asserts it is reachable from every state that can precede it, and the invariant handlers' blanket `try … catch {}` should
be replaced by a catch that records which revert reason occurred and asserts that a required transition never reverts with a
panic.

**Closed, 2026-09-22.** Fixed by the deeper change, not the clamp. Both split sites now go through one private `_giveBack`,
and `Charge` carries a fourth field, `givenBackWei`, recording what that coin has already had back. The proportional pass gives
a coin no more than `weiCharged − givenBackWei`, and a second pass hands whatever truncation left over to the coins that still
have room. The two passes always place the whole amount, because `release` charges until nothing is left (so the charges sum to
`maxEth` exactly) and neither caller ever gives back more than `maxEth` in total — so no coin can be given back more than it
paid, and the subtraction cannot underflow from any sequence of instalments. `settleRune`'s split is untouched: it divides rune
*units*, touches no `chargedTo`, and settles the purchase in one call, so nothing accumulates.

The three reproductions are inverted in place and now assert the fixed behaviour — the stranger's wei is absorbed and the
write-off succeeds, the truncating two-instalment refund completes with both coins exactly whole, and the overdue write-off is
available without an attestor's false receipt (no member id is burned, and the bond, not the coins, pays). A fuzz test,
`testFuzz_C1_fixed_anySplitOfARefundCompletes`, asserts the general statement over arbitrary three-instalment splits: 512 runs,
green. Full suite 174/174.

What is **not** fixed is the general rule in the paragraph above — liveness is still untested everywhere else, and the
invariant handlers still swallow panics in a blanket `try … catch {}`. That belongs with the v2 rework.

---

## High

### H1 — the order of two matured releases is anybody's to pick, and one order permanently bricks the other

**Severity: high.** Under the phase-3 scale: funds are locked under conditions an attacker can arrange, and a documented
assumption is false. No theft; recovery costs one owner transaction, per occurrence, indefinitely.

**Where.** `AssetVault.release` (line 325, `TooManyPayers`), against `declarePurchase`'s lack of any payer reservation.

**The defect.** `payersFor(amount)` is correct when the keeper calls it and is **not reserved**. `reserved` reserves wei; it
reserves nothing about *which* coins will pay them or *how many*. `release` is permissionless and takes an id, so any caller
chooses which of up to `MAX_OPEN` matured declarations eats the head of the queue first. One order leaves a large coin at the
head; the other drains it and pushes the next declaration onto a run of dust coins.

`TooManyPayers` is not a temporary condition. The queue only ever grows at the tail and `queueHead` only moves forward, so a
declaration that needs 65 payers needs 65 payers for ever.

**What an attacker does.** Nothing but choose the order, once two declarations are mature. In `test_repro_H1` a declaration
that `payersFor` reported as **one payer** at declaration time becomes permanently unreleasable because a stranger released the
other one first.

**What it costs them.** In the steady state, one transaction's gas. The test funds 70 coins at `MIN_ENTRY` to build the
condition, but a live listing accumulates coins near `MIN_ENTRY` on its own — that is what `MIN_ENTRY` is for. The attacker
does not need to create the queue shape; they need to be watching when it exists.

**What it gains them.** Liveness denial. Each occurrence costs the operator an owner (multisig) `cancel`, a fresh cap, a fresh
declaration and another public hour. An attacker who repeats it every cycle turns every purchase into a multisig ceremony.

This is a sharper answer than the brief gives itself. Question 2 of the brief asks about "64 coins parked at the head with
0.01 ETH each to force small declarations" and answers "it costs 0.64 ETH of real fees that are then spent; we think that is
acceptable". The cost analysis is wrong on two counts (see M8) and, more importantly, the question is the wrong one: the
attacker does not need to park anything. They need to pick an ordering.

**Smallest change that closes it.** Record the payer count at declaration and reserve the queue prefix, or — far simpler —
make the declaration's charge list *immutable at declaration*: compute the charges in `declarePurchase` and store them, so
`release` only pays out what was already decided. That also removes the last external dependency between two open declarations.
If that is too large a change, the one-liner is to let `release` fall back to charging *fewer, larger* payers by skipping over
exhausted queue entries — but that changes which coins pay, which is a policy decision, not a fix.

### H2 — the owner cannot take a vault's ETH, but can destroy it instantly and irreversibly

**Severity: high.** Total, permanent, unrecoverable loss of every vault's balance, from a single key, in two transactions,
with no delay on either.

**Where.** `ListingRegistry.setPaused` (line 121) and the inherited, un-overridden `Ownable.renounceOwnership`.

**The defect.** `setPaused` is instant by design, and correctly so — a protection should apply at once. `renounceOwnership` is
inherited from OpenZeppelin's `Ownable`; `Ownable2Step` overrides `transferOwnership` and `_transferOwnership` and does **not**
disable it, so `renounceOwnership()` sets the owner to `address(0)` with no delay and no second step. Together they are a
one-way door:

- `release` and `declarePurchase` both check `registry.isPaused(listingId)` and refuse;
- `cancel` and `writeOff`-before-7-days are `onlyOwner` and nobody is `address(0)`;
- nobody can unpause, because nobody is the owner;
- `AssetVault` has no withdraw function.

`test_repro_H2` leaves 5 ETH in a vault that nothing can ever move.

**What an attacker does.** Holds the owner key. That key is meant to be a 2-of-3 or better multisig and `DeployChecks` refuses
a non-contract owner on a public chain, so this is not an unprivileged attack. It is the blast radius of the key the documents
say cannot cause loss.

**What it costs them.** Two transactions.

**What it gains them.** Nothing, which is why this matters: it is available to a compromised key, a coerced signer set, or a
mistake. `renounceOwnership` in particular is a function nobody in this design ever wants to call, left enabled by default.

**Why this is the right question to raise against `recovery.md`.** The brief's first question is whether "no sunset path" is
the right side of the trade, and `recovery.md` argues it is, because "a sunset path with a 30-day delay is a 30-day-delayed rug
for whoever holds the owner key". That argument weighs *theft* against *stranding*. It never weighs *destruction*, and
destruction is available today with **zero** delay while every loosening in the registry waits seven public days. The trade as
actually implemented is not "the owner cannot take the money"; it is "the owner cannot take the money but can burn all of it
instantly, and a withdraw path behind 30 public days was rejected as too dangerous." Those are not obviously the right way
round. An auditor should be asked to weigh them, and the operator should decide with the second sentence in front of them, not
the first.

**Smallest change that closes it.** One line in `ListingRegistry`:

```solidity
function renounceOwnership() public pure override { revert(); }
```

That removes the irreversibility without touching the pause. A pause held by a live owner is a hostage situation, which is
bad; a pause held by nobody is a cremation, which is worse and is the part that costs one line.

---

## Medium

### M1 — a role rotation may collapse the keeper and the purchaser into one key

**Where.** `ListingRegistry.proposeRoles` (line 131).

**The defect.** `proposeRoles` rejects `purchaser == attestor` and `keeper == attestor`, and says nothing about
`keeper == purchaser`. `script/DeployChecks.sol` (line 105) rejects all three — but the deploy script runs once, and the
*first* role set is the one it guards. Every rotation afterwards goes through `proposeRoles` alone, unchecked.

Phase 3 records this as I6 and accepts it: "Only the attestor must differ. The deploy script enforces three distinct
addresses; the registry does not, because the two are one operator either way." Two things are wrong with that. The deploy
script does not enforce it at rotation, which is when it matters — rotation is what happens under the time pressure of a
suspected key compromise. And "one operator either way" is an operational claim, not a cryptographic one: with
`keeper != purchaser`, a compromised keeper key alone can only send ETH to the *purchaser's* address, and the attacker gets
nothing. With `keeper == purchaser` it is one key that posts its own cap, declares against it and receives the ETH.

**What an attacker does.** Nothing directly; this is a configuration hole that turns a single-key compromise from a nuisance
into a drain. `test_repro_M1` rotates to one key and takes the whole vault.

**What it costs them / gains them.** At `bondBps = 10 000` the key must lock the released amount as bond first, and getting it
back needs the attestor's settle, so the theft is not free. At any lower rate the profit is `(1 − bondBps/10000) × maxEth` per
declaration, up to `MAX_OPEN` at a time, with a one-hour public window for the owner to cancel.

**Smallest change.** Add `|| keeper_ == purchaser_` to the existing `SameKey` check. One clause.

### M2 — the keeper can block every rune payout for ever, with no bound and no override

**Where.** `Distributor.postMinimum` (line 355) and `requestPayout` (line 363).

**The defect.** `postMinimum` takes any `units` with no upper bound, no delay and no relation to the listing's divisibility or
to any distribution's size. `requestPayout` refuses both when `accrued < minimumUnits` **and** when the last minimum has
expired. Both levers are keeper-only and there is no owner override anywhere in the contract.

**What an attacker does.** Compromises or corrupts the keeper key and calls `postMinimum(listingId, type(uint256).max)` every
24 hours — or simply stops posting, which has the same effect once the TTL lapses. Every holder's claimed units are frozen
indefinitely. `test_repro_M2` shows the owner cannot post one either.

**What it costs them.** One transaction a day.

**What it gains them.** Censorship of the entire pro-rata delivery path, with the units still recorded as claimed and the
Bitcoin-side rune still in the project's custody. The trust model lists the keeper as able to "post … a rune delivery minimum
(24 h TTL)" and does not list this as something it cannot do.

**Smallest change.** Bound it — reject `units` greater than some fraction of the listing's largest distribution total so far —
or let the owner post one too, or both. The dust floor the function exists for (546 sats plus a fee share) is a small number;
nothing legitimate needs an unbounded one.

### M3 — change returned below `MIN_ENTRY` is attributed to a coin and can never be spent

**Where.** `AssetVault._credit` (lines 168–181), reached from `returnUnspent` and `writeOff`.

**The defect.** A coin drained to zero by a release is dequeued, and re-enters the queue only when its balance reaches
`MIN_ENTRY` again. Any refund share below 0.01 ETH therefore lands in `waiting` — attributed to the coin, counted in the
balance, not spendable — until that coin earns another 0.01 ETH of fees. For a coin that has stopped trading, which is the
ordinary end of a memecoin, that is never.

This follows from H1's fix in phase 3 and is not mentioned there. `recovery.md` acknowledges "every coin's `waiting` dust" as
part of what stays in a vault for ever, but frames it as a coin that never earned enough — not as money that was earned,
spent, and refunded.

**What it costs.** Nobody attacks this; it is a drag. With 64 payers and a 10% refund, *every* payer's share is under
`MIN_ENTRY` and the whole refund is stranded. The same happens to every write-off's bond slash: a coin that paid 0.01 ETH into
a purchase written off at a 20% bond gets 0.002 ETH back, which is stranded on arrival.

**Smallest change.** Let a re-entering coin keep its place: credit refunds back into a coin's *queued* balance if it was
charged by the purchase being refunded, regardless of `MIN_ENTRY`. The anti-dust rule exists to stop *new* entries being
created for nothing; a coin being paid back what it already put in is not a new entry.

### M4 — after a rotation the named purchaser's bond cannot be topped up, and topping up funds their successor

**Where.** `AssetVault.postBond` (line 285) credits `registry.purchaser()`; `release` (line 341) requires
`requiredBondOf(p.purchaser)`, the purchaser named at declaration.

**The defect.** Phase 3's M1 fix correctly made `bondOf` and `outstandingOf` per purchaser so that a rotated-in purchaser
cannot be released ETH against a predecessor's bond. It left `postBond` crediting whoever holds the role *now*. So a
declaration made before a rotation names a purchaser whose bond can no longer be increased by anybody, including themselves —
and the old purchaser's attempt to fund their own obligation is a donation to their successor, who can withdraw it immediately
because nothing is in their hands. `test_repro_M4` is exactly that, with the successor walking off with 5 ETH of the
predecessor's money.

**What it costs / gains.** No theft by a stranger; a footgun that loses the old purchaser's own capital and strands an
otherwise valid declaration until the owner cancels it.

**Smallest change.** `postBond(address purchaser_)`, requiring `purchaser_` to be either the current purchaser or an address
that already holds a bond. Keep the no-argument version as a wrapper for the common case.

### M5 — a lot pinned to a beacon that stops producing can never leave `Pinned`

**Where.** `Distributor.voidDraw` (line 230), which requires `Stage.Resolved`.

**The defect.** Phase 3's M3 built `voidDraw` for a ticket list with a gap — a lot that *resolved* and that nobody can claim.
There is no equivalent for a lot that can never resolve. If drand's evmnet is retired (question 6 in the brief asks exactly
this), every pinned lot is stuck: `resolve` cannot prove a round nobody signed, `redraw` only pins another round nobody will
sign, `voidDraw` refuses the stage, `openDraw` needs `None`, `choose` is final for anything but `Hold`, and `beacon` is an
immutable of the Distributor with no migration path. `test_repro_M5` walks all five doors.

**Note the asymmetry.** Voiding from `Pinned` is *strictly less dangerous* than voiding from `Resolved`, which is the one they
built: in `Pinned` no winning ticket is known yet, so voiding cannot possibly be a second draw — the objection that made
`voidDraw` owner-only and 7-day-delayed does not apply at all. The safe version is the one missing.

**Smallest change.** Allow `voidDraw` from `Stage.Pinned` as well, gated on `block.timestamp >= lot.pinnedAt + VOID_AFTER` and
on `beacon.randomnessOf(lot.round) == 0`. It returns the lot to the keeper for a fresh snapshot, as it already does.

### M6 — lowering `bondBps` retroactively un-bonds a release already in the purchaser's hands

**Where.** `AssetVault.requiredBondOf` (line 279) reads `registry.bondBps()` live.

**The defect.** The rule is stated as "raised at once; lowered only after `CHANGE_DELAY` in public", and the seven public days
are presented as the safeguard. They are not one. `SETTLE_DEADLINE` is only three days, so a purchase released *inside* the
pending window is still live — not yet overdue, so `withdrawBond`'s overdue check does not block — when the lower rate lands.
The purchaser withdraws the difference and then simply never settles.

`test_repro_M6`: the owner announces 100% → 10% on day 0; a 5 ETH purchase is declared and released, fully bonded, on day 5;
the reduction lands on day 7 while its deadline is day 8; the purchaser pulls 4.5 ETH back out and walks. `totalWrittenOff`
ends at 4.5 ETH against a release that was 100% bonded when it happened.

**What it costs / gains.** It needs the owner to lower the rate, which is a legitimate and publicly announced act. It does not
need the owner to be malicious — an honest reduction hands the difference to any purchaser holding a live release.

**Smallest change.** Snapshot the rate: store `bondBps` in the `Purchase` at `release`, and compute the required bond as the
sum over a purchaser's open purchases of `(maxEth − returned) × p.bondBps / 10000`. Then a reduction applies to future releases
only, which is what the seven days were supposed to guarantee.

### M7 — a declaration never expires, so it can be fired against a cap that died a year ago

**Where.** `AssetVault.release` (line 315), which checks `block.timestamp >= p.releasableAt` and nothing else.

**The defect.** `CAP_TTL` bounds when a purchase may be **declared**. Nothing bounds when it may be **fired**. This is
deliberate and tested (`test_held_aCapThatExpiresAfterDeclarationDoesNotStopTheRelease`, "bounded by the cap of its own hour"),
and the reasoning is sound for a release that happens an hour later. It is not sound for one that happens a year later.

The keeper can declare `MAX_OPEN` purchases at a moment when the cap is honest, sit on them, and release them into a collapsed
market. Meanwhile `reserved` keeps that ETH out of every other declaration — `test_repro_M7` shows a vault with 5 ETH of fresh
fees unable to declare anything for a year because three stale declarations hold every open slot and every reserved wei. Only
the owner can clear it, and only by watching.

**This one came from the differential.** Nosh's `ExternalSweepVault` pairs its `WITHDRAWAL_DELAY = 1h` announce-then-execute
with a `WITHDRAWAL_WINDOW = 6h` **expiry**, and their comment says why: an announcement that never expires lets a keeper
pre-arm withdrawals and fire them months later against funds that had not arrived when they were announced. Etch's public hour
is the same construction with the second half missing.

**Smallest change.** Add `releasableUntil = releasableAt + RELEASE_WINDOW` (a few hours) and refuse a release past it. An
expired declaration then needs re-declaring under a fresh cap, which is the intent. Anyone can already be the one to release,
so the window does not create a liveness dependency on the keeper.

### M8 — a queue slot costs a donation, not "real fees", and the ETH comes back to the attacker

**Where.** `FeeRouter.receive()` (line 65) and `harvest()` (line 101), against the brief's cost model for the `MIN_ENTRY`
queue.

**The defect.** The brief's question 2 answers the dust-queue concern with "it costs 0.64 ETH of real fees that are then
spent; we think that is acceptable". Both halves are wrong.

*Not real fees.* `FeeRouter.receive()` accepts ETH from anybody, and `harvest()` splits `address(this).balance` regardless of
where it came from — deliberately ("nor does it care where ETH came from"). So an attacker does not need to trade their coin
on Pons at all. They launch a coin (one Pons launch fee), send 0.0125 ETH straight to its router, and call the permissionless
`harvest()`. 0.01 ETH lands in the vault attributed to their coin and the coin joins the queue. No market risk, no spread, no
Pons trading fees.

*Not spent away from them.* An acquisition belongs to the coin whose fees paid for it, and that coin's creator — the attacker —
chooses its fate. `Distributor.choose(listingId, acquisitionId, Mode.Named, attacker)` hands it straight back. So the parked
ETH is not a sunk cost; it is a purchase the attacker directs to themselves, net of the 20% treasury cut.

The real cost of parking N coins in the queue is therefore `N × launchFee + 0.20 × (N × 0.0125 ETH)` plus gas — not
`N × 0.01 ETH`. For N = 64 the ETH component is about 0.16 ETH, not 0.64.

**Nosh is stricter here, explicitly.** Their `FeeRouter.receive()` rejects everything except the Pons escrow. Etch's accepts
all comers and documents it as a property rather than a risk.

**Smallest change.** Reject unattributed ETH in `FeeRouter.receive()` unless the sender is the escrow or the curve. That does
not close H1 (which needs no parking at all) but it restores the cost model the brief reasons from, and it removes a
donation path that is currently a free route into a listing's queue.

### M9 — `redraw` is a free, repeatable selective abort on the randomness

**Where.** `Distributor.redraw` (line 200).

**The defect.** `redraw` is permissionless and its only condition is that nobody has **proven** the pinned round on chain —
not that drand failed to produce it. drand's value for a round is public seconds after the round is due. So at
`pinnedAt + 24 h`, any caller who has computed the outcome off chain and does not like it can replace the round instead of
resolving it, and repeat. `test_repro_L2` shows a stranger discarding a round whose value is already determined.

Phase 3 records this as I2 and accepts it: "It needs everyone, the winner included, to have ignored a public signature for a
day." That framing assumes nobody is actively contesting. Once somebody is, the honest party has to be awake at exactly
`pinnedAt + 24 h` — a moment the griefer knows precisely and the honest party has no reason to be watching — and has to win a
race against them on a first-come-first-served sequencer.

**The internal inconsistency that gives it away.** `voidDraw` is owner-only and waits seven days *because it is a second draw*.
`redraw` is also a second draw and is open to anyone after one day. The two were reasoned about separately and ended up at
opposite ends.

**What it costs.** One transaction per attempt, plus 24 hours. **What it gains.** With enough patience, the attacker's
preferred outcome, on a draw whose value is the asset.

**Smallest change.** Make `redraw` owner-only, matching `voidDraw`. The liveness case it exists for — drand skipped a round —
is rare and does not need to be permissionless. A stricter alternative that keeps it open: require evidence that drand moved
on without the pinned round, i.e. that `randomnessOf(r) != 0` for some proven `r > lot.round`.

---

## Low

### L1 — an overdue receipt stops new declarations but not a matured release
`isBlocked` gates `declarePurchase` and `withdrawBond`; it does not gate `release`. The documents say "the machine stops at the
first overdue receipt" (`AssetVault` NatSpec, `trust-model.md` §1); up to `MAX_OPEN − 1` already-declared purchases still pay
out after it. `test_repro_L1`. The bond check at release is the real gate, and at the default 100% it is a strong one. Either
add the `isBlocked` check to `release` or correct the sentence.

### L2 — `forwardToken` sends ERC-20s to a treasury that cannot move them
`FeeRouter.forwardToken` is permissionless and sends the router's whole balance of any ERC-20 to `registry.treasury()`. The
NatSpec says selling it is "the treasury's word". `src/Treasury.sol` — the contract the deployment puts there — has **no**
ERC-20 function: no transfer, no rescue, no owner hook, no `IERC20` import. It is not a word that treasury can keep, and
`test_repro_L3` shows six ways of asking and six refusals. Either add a guarded token-forward to `Treasury`, or gate
`forwardToken` so it cannot be called against a treasury that is the `Treasury` contract.

### L3 — `CoinInfo.exists` is written and never read
`Launcher._coins[coin] = CoinInfo(msg.sender, router, p.listingId, true)` sets a flag that `grep -rn "exists" src/` shows is
read nowhere. If Pons's factory ever returned an address already in `_coins`, the Launcher would silently rewrite
`creatorOf(coin)` and `routerOf(coin)` — changing who controls an existing coin's acquisitions and breaking the old router's
`deposit`. Pons's own CREATE2 should make that impossible, so this is a defence-in-depth gap rather than a live bug; the
presence of the unused field suggests the check was intended and dropped. Add `if (_coins[coin].exists) revert`.

Nosh hit the adjacent problem in production and moved `routerOfCoin` / `collectionOfCoin` onto their Registry, because
rotating the Launcher silently rewrote every existing coin's disposition. Etch is safe from *that* one — `ListingRegistry.wire`
is one-shot — which is worth saying, because it is a place Etch is stricter.

### L4 — `sweepPool`'s hardcoded `0, 0` is safe only under the current Pons hook
`FeeRouter.sweepPool` calls `hook.sweepPoolFees(poolId, 0, 0)` — zero minimum conversion out, zero minimum buyback out —
permissionlessly, with an arbitrary `poolId`. That is safe *only* because Pons's hook currently refuses to perform the internal
swap for a non-operator caller (`docs/phase1.md` F1, verified on a fork). This is the answer to question 7 of the brief: **yes,
something in the router is unsafe if Pons's contracts are upgraded.** If the hook is ever changed to let a non-operator sweep
with conversion, `sweepPool` becomes a permissionless zero-slippage swap that anyone can sandwich, and 80% of the loss lands on
the vault. The router has no owner and no upgrade path, so it cannot be fixed afterwards — every coin already launched keeps
the function. Pass a caller-supplied minimum instead of a constant zero, or drop `sweepPool` and let callers hit the hook
directly.

### L5 — greedy attribution has no minimum-share rule and no carry
`settleInscription` charges each arriving inscription to whichever payer has the largest unmatched value, in increments of
`p.maxEth / p.quantity`. There is no minimum share: a coin that funded a small fraction of a purchase can end up owning an
entire inscription's disposition, and `assignedWei` is per-purchase and discarded, so a coin that wins nothing gets no credit
carried into the next purchase either. (Runes have `carry`; inscriptions do not.) Also, `bestGap` starts at 0 and the
comparison is strict, so if every gap is zero the item silently goes to `cs[0]`.

Nosh solved the same problem with `MAX_PAYERS_PER_NFT = 4` and a rule that the top payer must cover **at least half** the price
before it gets policy rights over the item, and their NatSpec documents the exploit that forced it: with four payers the
"winner" owned the policy on an eighth of the price. Etch has no such floor.

### L6 — the public hour rests on the clock phase 3 already declared untrustworthy
Phase 3's M2 widened `PIN_AHEAD` from 2 rounds to 20 precisely because "`block.timestamp` on an L2 is the sequencer's". The
same clock enforces `DECLARE_DELAY`. The exposure is the mirror image — a *stale* clock helps a draw attacker, a *fast*
one helps a release attacker — and an hour is a far larger margin than the 1.3 s drift measured, so this is not currently a
practical concern. It is worth naming in `trust-model.md`, which credits the sequencer for the draw and not for the delay.

### L7 — the destination path has one hour and no recovery, and refuses addresses Bitcoin accepts
`BtcAddress.isTaproot` rejects upper-case bech32m, which BIP-173 permits, so a winner pasting an upper-case address from a
conforming wallet gets `BadAddress`. Separately, `setDestination` locks one hour after the first address and `recordDelivery`
then sends to whatever is stored: a winner who mis-types and notices on hour two has no recovery, and the keeper has no
on-chain way to refuse. Both are documented as accepted; the second deserves a second look, because the asset is the prize.

**Verified: nothing depends on `BtcAddress` being more than a typo guard.** It is called from exactly two places
(`setDestination`, `requestPayout`), neither moves ETH, and a wrong verdict in either direction costs a delivery, not a
balance. The brief's characterisation is correct.

### L8 — a bond posted before roles are set is unrecoverable
`postBond()` credits `bondOf[registry.purchaser()]`, which is `address(0)` between deployment and the first `proposeRoles`.
`withdrawBond` requires `bondOf[msg.sender] != 0` and nobody is `address(0)`. ETH posted in that window is locked for ever. The
window is short and the amount would be a mistake rather than an attack; a `if (p == address(0)) revert ZeroAddress()` closes
it. The rest of the pre-role window was checked and is benign (see below).

### L9 — an over-promising distribution root makes honest holders race
`claimShare` reverts `OverTotal` once `d.claimed + units > d.total`. If the keeper publishes a root promising more than was
bought, the first claimants are paid in full and the last get nothing, with `rollOver` giving them nothing either (the
distribution is exhausted) and `cancelDistribution` unavailable after 15 minutes. The contract correctly refuses to over-pay —
that is invariant 6 and it holds — but the failure mode it produces is a silent race among honest holders with no recourse.
Worth a sentence in `trust-model.md` §4, and worth considering a pro-rata scale-down instead of first-come-first-served.

---

## Informational

- **I1. `expandMessageXmd` omits the RFC 9380 `ell > 255` abort.** Unreachable: the only call site passes a constant 96, giving
  `ell = 3`. Worth adding for a library that reads as general-purpose.
- **I2. `boundFor` and `_requireUnitPrice` can overflow on absurd `quantity`.** `capWei * quantity` and `p.maxEth * received`
  are unchecked-free 0.8 arithmetic, so an absurd quantity reverts rather than wrapping. The consequence is a settle that can
  never succeed and a forced write-off, not a bypass. Bounding `quantity` would be cheap.
- **I3. `_sqrt` does not verify its own result.** `a^((P+1)/4)` returns a wrong root for a non-residue. If the SvdW branch
  selection were ever wrong, the failure would surface as `EcAddFailed` from the ecAdd precompile rather than as a point off
  the curve — a revert, not a forgery. The selection was independently verified correct (below), so this is defence in depth.
- **I4. `hashToPoint` can in principle produce the point at infinity** if `u0` and `u1` map to opposite points, which the
  pairing then rejects. Negligible probability; a revert, not a wrong answer.
- **I5. The invariant campaigns cannot detect a liveness failure.** Every handler call is `try … catch {}`, and
  `System.invariant.t.sol` runs at `bondBps = 5_000` — the one setting that left headroom for C1's underflow and hid it,
  while the *default* of 10 000 guaranteed it. (C1 itself is now fixed; this limit of the campaigns is not, and it is the
  reason C1 survived two passes. Raising the campaign's `bondBps` to the default would not fix it either — it would only
  re-cover the one defect already found.) `FOUNDRY_PROFILE=deep` (10,000 fuzz runs, 1,000 × 200 invariant calls, 46 s)
  was run against this commit: **174 passed, 0 failed.** 200,000 handler calls per campaign do not surface a defect that one
  wei triggers deterministically, because the handler catches the panic and the books then balance. This is the most important
  informational item in this document. See the note at the top.

---

## Checked and found sound

A list of only defects tells you nothing about coverage. These were attacked and held.

**The randomness (the strongest part of the codebase).**
- **The SvdW constants are correct.** Independently recomputed against RFC 9380 §F.1 in Python: `C1 = g(Z) = 4`,
  `C2 = −Z/2`, `C3² = −g(Z)(3Z² + 4A)` with `sgn0(C3) = 0` as the RFC requires, `C4 = −4g(Z)/(3Z² + 4A)`. All four match the
  contract's hardcoded values exactly.
- **`Z = 1` is a valid SvdW parameter for BN254.** All three RFC conditions verified: `g(Z) ≠ 0`; `−(3Z² + 4A)/(4g(Z))` is a
  non-zero square; and `g(Z)` is a square.
- **The map always lands on the curve.** 20 000 random field elements plus the degenerate cases (`u = 0, 1, ±1/2`, where
  `tv1·tv2 = 0` and `inv0` returns 0) reimplemented in Python against the contract's exact branch order: zero off-curve
  results.
- **`expand_message_xmd` matches the RFC** line for line for Keccak-256: `s_in_bytes = 136` (the Keccak rate), `DST_prime`,
  the `l_i_b_str` two-byte length, the `b_0` / `b_1` / `b_i` chain, `ell = 3`, `L = 48`, and the `hi·2²⁵⁶ + lo` reduction of
  each 48-byte block.
- **The randomness derivation matches drand's own output.** `sha256(signature) == randomness` verified in Python for all 20
  real evmnet beacons in `test/fixtures/drand-evmnet-20.json`, independently of the contract.
- **The G1 checks are right and the missing subgroup check is correctly missing.** BN254's G1 has cofactor 1 and prime order,
  so every on-curve point is in the subgroup; there is nothing to check. The `sx, sy < p` bounds give a signature exactly one
  encoding. `sy = 0` cannot occur (it would be a 2-torsion point in an odd-order group), so `p − sy` is never `p`. The pairing
  input order `(x.c1, x.c0, y.c1, y.c0)` matches EIP-197.
- **The L5 gas fix works.** An off-curve point is a cheap revert rather than a precompile that burns all forwarded gas.

**`BtcAddress`.** The hrp expansion `[3,3,0,2,3]`, the 62-character length, the `bc1p` prefix, the `BECH32M` constant
`0x2bc830a3`, the five polymod generator constants, and the padding check on `a[55] & 0x0f` (52 program characters carry 260
bits for a 256-bit key; the low four bits of the last are the padding) are all correct. And, as above: nothing depends on it.

**`AssetVault` reentrancy.** Every external call was traced. `release` makes its call to `p.purchaser` **after** every state
change, under `nonReentrant`, which also covers `withdrawBond`, `writeOff` and `returnUnspent`. The functions left unguarded
are either role-gated (`declarePurchase`, `postCap`, `cancel`, `settle*`) or harmless to re-enter (`deposit`, `depositFrom`
and `postBond` only add money, and `release`'s bond check has already passed). `FeeRouter.harvest`'s own `_entered` guard
covers the escrow claim and the treasury call, and effects precede the call (the L2 fix). No reentrancy path was found.

**The queue's internal consistency.** A queued coin can never have a zero balance (`_credit` only ever adds a non-zero amount,
and `release` dequeues exactly at zero), so neither the `release` loop nor `payersFor` can spin; and both terminate even if it
could. `waiting`'s `-=` cannot underflow, because the only branch that subtracts is the re-queue branch, and it subtracts
exactly what was previously in `waiting`. `attributed ≥ reserved` holds, so a declaration always has coins behind it.

**Double settlement, and a settle after a write-off.** All three settle paths require `Status.Released`. `Settled` and
`WrittenOff` both refuse. `settledQuantity` cannot overshoot `quantity` because the status changes at equality.
`inscriptionHeld` is keyed by leaf, never cleared, and survives a membership-root change — the safe side of the stale-state
problem Nosh hit from the other direction.

**The settlement rule (`_requireUnitPrice`, phase 3's H2).** `(maxEth − returned) × quantity ≤ maxEth × received` was checked
for rounding — the floor division favours the vault — and for the `quantity`-gaming the brief asks about. The rule holds the
purchaser to the *declared* per-unit price for what actually arrived, which is what it claims; it does not and cannot hold them
to a *good* price, which the documents say. A partial delivery followed by a write-off over-slashes only a purchaser who chose
not to return what they were still holding, which is correct.

**The 80/20 split and the treasury-deferral path.** Rounding favours the treasury by at most 1 wei per harvest;
`treasuryOwed` can never exceed the balance, so `address(this).balance - treasuryOwed` cannot underflow; a refusing treasury
never blocks the vault's share.

**Registry role separation, as far as it goes.** `purchaser == attestor` and `keeper == attestor` are correctly refused
(M1 is that the third pair is not). `wire` is one-shot. `leafOf`'s double hash correctly prevents an inner node being passed as
a leaf, and the same applies to `ticketLeaf` and `shareLeaf`, whose different ABI lengths keep them from being confused.

**Claim conservation.** `claimShare`'s `d.claimed + units > d.total` check plus `carry`'s restore-on-cancel means the sum paid
out can never exceed what was acquired plus what rolled over. `cancelDistribution` restores exactly the carry it consumed, and
stale `shareClaimed` flags cannot exist because claims cannot happen inside the challenge window.

**The pre-role window.** Between deployment and the first `proposeRoles`, every role modifier refuses (nobody is
`address(0)`); before `wire`, `deposit` and `harvest` revert on the zero launcher and fees simply accumulate in the router
until wiring; `Treasury.setToken` refuses. The only exposure found is L8, and it costs whoever does it their own wei.

**`Launcher` salting.** `keccak256(abi.encode(msg.sender, p.salt))` binds a router address to its creator, so nobody can squat
another's, and a reused salt only collides with yourself.

---

## The differential: Nosh

Nosh (usenosh.app) runs the same architecture on the same chain — a registry, per-collection vaults holding fees, a keeper
posting floor caps, a fee router per coin splitting 80/20, purchases through a marketplace, raffle distribution weighted by
which coin fed the vault. It is live: 47 collections, a sample vault showing `totalSpent` of 0.3265 ETH and a floor cap of
0.0871 ETH. Their `Launcher` at `0x1a66D8ECc2274Fc8887ab386e35D7Da981BA2F37` is verified on Sourcify, and because it is a
flat-source verification the bundle carries their whole contract set: `SweepVault.sol`, `CollectionRegistry.sol`,
`FeeRouter.sol`, `ExternalSweepVault.sol`, `Disposition.sol`. Their raffle
(`0x4F2C9A8c1f181D65aCab66430E5075a7998f18d3`) and their randomness beacon (`0xA096500E4414199F65ba738F1728AABBA438e786`) are
**not** verified anywhere; what is said about those below is from bytecode and live `eth_call` and is labelled as such.

### The one structural difference, and it is the whole design

**Nosh's vault executes the purchase itself, on chain, against an allow-listed marketplace. Etch hands ETH to a person.**

`SweepVault.sweep()` takes `(target, value, data, candidateTokenIds, minBought)`, checks `registry.sweepTargets(target)`,
snapshots its own ETH and NFT balances, makes the opaque `target.call{value: value}(data)` into Seaport, and then asserts on
the deltas: `bought >= minBought`, `spent <= cap * bought`, every arriving token id in the candidate list, no duplicates, none
already owned. It never decodes the order. `SweepVault` has **no withdraw function at all** — the only ETH exit is that call
into an allow-listed target, and the only NFT exit is `release()` gated on the distributor.

Etch's `AssetVault` has the same "no withdraw" promise and a completely different meaning for it: the ETH leaves to an EOA
named at declaration. Every piece of machinery in `AssetVault` that is not in `SweepVault` — the bond, the attestor, the
receipt, `_requireUnitPrice`, `writeOff`, `PUBLIC_WRITEOFF_AFTER`, the whole `Status` enum past `Released` — exists to
compensate for that one decision. And C1, M4, M6, L1 and half of M1 are defects *inside that compensating machinery*. Nosh
does not have those bugs because it does not have that machinery.

That is not a criticism that can be acted on: Etch buys Bitcoin ordinals and runes, which no EVM contract can execute. Bitcoin
is the reason the purchaser exists. But it is the honest framing of the risk, and it should be stated in `trust-model.md` in
those terms: **the single largest source of complexity and of defects in this codebase is the fact that the asset lives on
another chain**, and the comparable protocol that does not have that constraint has roughly a third of the state machine.

### Where Nosh is stricter, and why it matters

| | Nosh | Etch |
|---|---|---|
| **Who holds the money at the moment of purchase** | The vault. ETH never reaches a human. | The purchaser, an EOA, for up to 3 days. |
| **What bounds the purchase** | Enforced atomically: `spent ≤ cap × bought`, asserted on balance deltas in the same transaction. | Enforced at declaration (`maxEth ≤ cap × quantity`) and again days later by the attestor's word. |
| **Spend targets** | Allow-listed: `registry.sweepTargets(target)`. | N/A — but the analogous surface, `sweepPool`'s arbitrary `poolId` with `0,0` slippage, is not (L4). |
| **Router `receive()`** | Rejects everything except the Pons escrow. | Accepts anyone's ETH (M8). |
| **Announce-then-execute** | `WITHDRAWAL_DELAY = 1h` **and `WITHDRAWAL_WINDOW = 6h` expiry.** | `DECLARE_DELAY = 1h` and **no expiry** (M7). |
| **Attribution floor** | `MAX_PAYERS_PER_NFT = 4`, top payer must cover **≥ half** before it gets policy rights. | Greedy largest gap, no floor, no carry (L5). |
| **Unclaimed prize** | `DELIVERY_TIMEOUT = 7 days` to reclaim from a winner who never posts an address. | `flagOverdue` sets a boolean. A winner who never calls `setDestination` strands the lot for ever. |
| **Design rule against DoS** | Written down: because there is no withdraw, an unbounded loop that stops fitting in a block **freezes the balance permanently**. Audited against. | Not written down. C1 is an instance of exactly this and was missed twice. |

**The single most useful thing they wrote.** They made `sweep` keeper-only after it had been permissionless, and left the
reason in a NatSpec comment:

> "This was permissionless, on the theory that the on-chain checks made the caller irrelevant. They do stop theft — ETH only
> leaves against an NFT arriving — but they do not stop a bad purchase. `floorCap` is public, so no seller ever offered the
> vault less than the cap and it paid the ceiling on every buy instead of the floor. Worse, the cap bounds ETH per NFT
> *delivered*, not per unit of value, so anyone holding cheap or freshly mintable supply could sell the vault a pile of it at
> cap and empty the vault in one call."

Both halves apply to Etch, and neither is in the threat model.

*A public per-unit cap is a price floor for sellers.* `capWei` is on chain, `capRef` is published, and the keeper's stated
policy is "the floor through the bridge's quote plus 5%". Anyone watching a vault knows what it will pay. Etch's purchaser buys
off chain, so this is weaker than Nosh's case — but `_requireUnitPrice` holds the purchaser to the *declared* price per unit
and creates no obligation whatsoever to beat it. A purchaser who buys at 60% of the cap and keeps the rest satisfies every
on-chain rule, provided they return nothing and the attestor receipts `quantity` items. There is no on-chain pressure toward
the floor, only toward the cap.

*A per-unit cap is not a per-value cap.* `boundFor(quantity) = capWei × quantity` and `settleInscription` accepts **any**
inscription in the committed membership root. A real ordinals collection spans two orders of magnitude in value between its
floor and its rare traits. So a cap posted against a blended or rare price, with a `quantity` chosen to match, can be satisfied
entirely with floor items, truthfully receipted by an honest attestor, with the difference kept. This is the same shape as
phase 3's H2 — which bounded *price per declared unit* — one level up: it bounds price per unit but not value per unit, and
the membership root is too coarse to distinguish them. Nosh's answer was to stop letting an unprivileged caller choose what
gets bought. Etch's keeper already chooses, so Etch's version of the fix would be a per-listing floor reference in `capRef`
that the attestor is required to check, which is a process change rather than a contract change — but it should be a named
process, not an assumption.

### Where Etch is stricter

Say this too, because it is true and it is load-bearing.

- **The bond.** Nosh has no stake, no collateral and no slashing anywhere in their codebase. Their answer to "the keeper
  mis-buys" is "the keeper can't steal, only mis-buy, and the owner can rotate it." Etch's bond at 100% genuinely changes the
  worst case for the one actor who holds real funds. It is the best idea in this codebase and it has no analogue in theirs.
- **The randomness beacon.** Etch's `DrandEvmnet` is ownerless, has no admin function, no fee and no allowlist, is in this
  repository, and is verified against 20 real beacons plus (now) an independent recomputation of its constants. Nosh's beacon
  is **unverified bytecode owned by a single EOA** — the same deployer key that deployed their registry — while their protocol
  itself sits behind a 2-of-3 Safe. They present it as neutral infrastructure ("OpenVRF"); it is their own key. Etch's is
  strictly better and it is not close.
- **Key separation.** Nosh's keeper both posts the cap and executes the spend, with no attestor and no second key. Etch
  separates cap/declaration from execution from attestation — three keys where they have one — which is what makes M1 (the
  registry permitting two of them to collapse) worth fixing rather than shrugging at.
- **Immutable coin→listing binding.** `ListingRegistry.wire` is one-shot, so Etch cannot suffer the bug that forced Nosh to
  move `routerOfCoin` off their Launcher.
- **Delays.** Every loosening in `ListingRegistry` waits seven public days. Nosh's `setFloorCap`, `setKeeper`,
  `setCollectionStatus` and `setSweepTarget` are all immediate owner or keeper calls with no delay at all.

### Where they are the same, and both wrong

**Nothing on chain bounds the cap, in either protocol.** Nosh's `setFloorCap` is `floorCap = cap; floorCapUpdatedAt =
block.timestamp;` with no sanity band, no maximum delta against the previous value, no timelock and no multisig on the cap
itself. Etch's `postCap` is `capWei = capWei_; capExpiry = ...` with the same absence. Two independent teams reached the same
answer, which is weak evidence that it is the right one and strong evidence that it is the default one. A max-delta-per-post
bound (say, no more than 2× the previous cap without a delay) is cheap in both and would turn a compromised keeper key from a
drain into a slow drain.

**Both depend on the same drand chain.** The beacon Nosh reads carries chain hash
`04f1e9062b8a81f848fded9c12306733282b2727ecced50032187751166ec8c3` — the same one `DrandEvmnet` hardcodes, with the same
`GENESIS = 1727521075` and `PERIOD = 3`. So brief question 6 ("what happens if evmnet is retired") is a shared systemic
dependency for both protocols on this chain, and M5 says Etch has no way back from it.

---

## What I could not reach a conclusion on

Stated as open questions, not as clean bills of health.

1. **Whether the SvdW *intermediate* values match a reference implementation.** I verified the constants, the RFC conditions
   on `Z`, and that 20 000 random inputs land on the curve, using a Python reimplementation of the contract's own branch
   order. That catches a wrong constant and a wrong branch. It does **not** catch a case where my Python and the Solidity share
   a misreading of the RFC — which is the same shared-assumption problem as the rest of this document, one level down. The
   end-to-end check against 20 real beacons is much stronger evidence than anything I added, and it remains the real argument.
   A comparison against drand's own Go implementation's intermediates, which the brief already asks for, is still not done.
2. **Whether `p.maxEth / p.quantity == 0` is reachable in `settleInscription`.** It needs `maxEth < quantity`, i.e. more
   inscriptions than wei, which is absurd — but `quantity` is a keeper-supplied `uint256` with no upper bound and I did not
   prove that every path to it is bounded. If it were reachable, every gap stays at its charge and every item is attributed to
   `cs[0]`.
3. **Whether `MAX_PAYERS = 64` is ever too small for a real purchase.** The brief asks. `settleRune` at 64 payers costs 7.99M
   gas, which is what set the ceiling, and a 64-payer release covers at least 0.64 ETH. Whether a listing that wants to buy a
   5 ETH inscription can assemble 5 ETH from ≤ 64 coins is a question about the live fee distribution, and there is no live
   fee distribution yet. Until there is, H1's fix matters more than the constant's value.
4. **Whether Nosh's beacon has an owner-privileged write path.** It is unverified. `fulfill(uint256,bytes)` simulated from a
   junk EOA reverts with a domain error rather than `OwnableUnauthorizedAccount`, which is evidence the relay is genuinely
   permissionless, but an owner-only path elsewhere in the bytecode was not ruled out. This affects the comparison, not Etch.
5. **Whether the agent's reading of Nosh's beacon as BLS12-381 is right.** The reported chain hash is evmnet's (BN254), and
   BLS12-381 field constants were also reported in the same bytecode. Those two do not obviously reconcile and I did not
   resolve it. It does not change any finding above.
6. **Everything in `keeper/`, `tools/snapshot/`, `autobuyer/` and `web/`.** Out of scope by the brief; not looked at. The
   keeper is where the cap policy and the snapshot actually live, and `trust-model.md` correctly says both are its word.
7. **Pons V2 itself.** Out of scope. L4 depends entirely on an assumption about a contract nobody here controls.

---

## Reproducing

```
forge test --no-match-path "test/fork/*"                       # 174 tests, all pass, 15 of them test_repro_*
forge test --match-path "test/security/InternalReviewV1.t.sol" # the 15 reproductions alone
FOUNDRY_PROFILE=deep forge test --no-match-path "test/fork/*"  # 10,000 fuzz runs, 1,000 x 200 invariant calls, 46 s
```

Both profiles were run at `84c7d8f` with the reproductions in place: 174 passed, 0 failed, in each. Note what that means for
the deep profile — see I5.

Every reproduction asserts the defect. When a finding is closed, delete or invert its test — do not leave it passing.
