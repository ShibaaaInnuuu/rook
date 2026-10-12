# Slither triage

Slither 0.11.6, run in Phase 3 over `src/` (libraries, tests and scripts filtered out; informational and optimisation detectors excluded):
`slither . --filter-paths "lib/|test/|script/" --exclude-informational --exclude-optimization`. **59 results: none is a defect that was
not already found by hand; six are rated high by the tool and all six are false positives or the design itself.** Every result is
listed. Slither was run AFTER the fixes of docs/phase3.md; it found neither H1 nor H2, which is the usual lesson about what a static
analyser is for.

## High: `arbitrary-send-eth` — 1

**By design.** `harvest` sends to the coin's vault (fixed at initialisation by the Launcher) and to the registry's treasury (changed only after 7 public days). Neither is caller-supplied.

- FeeRouter.harvest() (src/FeeRouter.sol#89-116) sends eth to arbitrary user

## High: `msg-value-loop` — 2

**False positive.** `returnUnspent` does not spend `msg.value` once per iteration; it splits one `msg.value` into shares that sum to it exactly (the last share is the remainder). `invariant_books` and `invariant_conservation` hold over 200,000 calls.

- AssetVault.returnUnspent(uint256) (src/AssetVault.sol#333-347) use msg.value in a loop: share = msg.value - given (src/AssetVault.sol#342)
- AssetVault.returnUnspent(uint256) (src/AssetVault.sol#333-347) use msg.value in a loop: share = (msg.value * cs[i].weiCharged) / p.maxEth (src/AssetVault.sol#342)

## High: `reentrancy-eth` — 2

**Guarded.** Slither does not recognise the router's own guard (`_entered`), added in this phase (L2); the treasury is paid last, after `treasuryOwed` is cleared. `test_L2_*` re-enters and is refused. The curve and escrow are Pons's contracts.

- Reentrancy in FeeRouter.harvest() (src/FeeRouter.sol#89-116):
- Reentrancy in FeeRouter.harvest() (src/FeeRouter.sol#89-116):

## High: `uninitialized-state` — 1

**False positive.** `_charges` is a mapping to arrays; it is written by `release` (`cs.push`) through a storage reference, which the detector does not follow.

- AssetVault._charges (src/AssetVault.sol#86) is never initialized. It is used in:

## Medium: `incorrect-equality` — 3

**Benign.** `amount == 0` is an early return; `b == take` is whether a coin was fully consumed, exact by construction (`take = min(b, left)`); `_open[i] == id` is an id lookup. None compares a balance an attacker can nudge.

- FeeRouter.forwardToken(IERC20) (src/FeeRouter.sol#79-86) uses a dangerous strict equality:
- AssetVault.release(uint256) (src/AssetVault.sol#295-330) uses a dangerous strict equality:
- AssetVault._close(uint256) (src/AssetVault.sol#431-436) uses a dangerous strict equality:

## Medium: `reentrancy-no-eth` — 1

**Accepted.** `resolve` writes the winning ticket after calling the beacon. The beacon is an immutable address of a contract with no callbacks (our own verifier); a re-entrant `resolve` would find the same stage and the same round and write the same value.

- Reentrancy in Distributor.resolve(uint256,uint256,bytes) (src/Distributor.sol#214-223):

## Medium: `uninitialized-local` — 7

**False positive.** Accumulators (`given`, `payers`, `best`, `bestGap`, `claimed`) rely on Solidity's zero default, deliberately.

- AssetVault.settleRune(uint256,uint256,bytes32,uint64).given (src/AssetVault.sol#387) is a local variable never initialized
- AssetVault.settleInscription(uint256,string,bytes32,uint64,bytes32[]).bestGap (src/AssetVault.sol#366) is a local variable never initialized
- AssetVault.release(uint256).payers (src/AssetVault.sol#305) is a local variable never initialized
- AssetVault.returnUnspent(uint256).given (src/AssetVault.sol#340) is a local variable never initialized
- FeeRouter.harvest().claimed (src/FeeRouter.sol#98) is a local variable never initialized
- AssetVault.settleInscription(uint256,string,bytes32,uint64,bytes32[]).best (src/AssetVault.sol#365) is a local variable never initialized
- AssetVault.writeOff(uint256,bytes32).given (src/AssetVault.sol#241) is a local variable never initialized

## Low: `reentrancy-benign` — 2

**Benign.** `launch` records the coin after Pons returns it (it cannot be recorded sooner: Pons creates it) and is `nonReentrant`; `resolve` as above.

- Reentrancy in Launcher.launch(Launcher.LaunchParams) (src/Launcher.sol#57-78):
- Reentrancy in Distributor.resolve(uint256,uint256,bytes) (src/Distributor.sol#214-223):

## Low: `reentrancy-events` — 2

**Benign.** Events after an external call in `harvest` and `resolve`; ordering of logs only.

- Reentrancy in Distributor.resolve(uint256,uint256,bytes) (src/Distributor.sol#214-223):
- Reentrancy in FeeRouter.harvest() (src/FeeRouter.sol#89-116):

## Low: `shadowing-local` — 13

**Style.** All thirteen are one pattern: a local `Lot storage lot` (or `Distribution`) inside the Distributor shadows the external view function `lot(listingId, acquisitionId)`. A function cannot be confused with a storage pointer by the compiler, and none of these functions calls the view. Renaming is cosmetic and was left, to keep this phase's diff to defects.

- Distributor.voidDraw(uint256,uint256).lot (src/Distributor.sol#230) shadows:
- Distributor.setDestination(uint256,uint256,string).lot (src/Distributor.sol#261) shadows:
- Distributor._pin(uint256,uint256,Distributor.Lot).lot (src/Distributor.sol#206) shadows:
- Distributor.flagOverdue(uint256,uint256).lot (src/Distributor.sol#284) shadows:
- Distributor.claimWin(uint256,uint256,address,uint256,uint256,bytes32[]).lot (src/Distributor.sol#244) shadows:
- Distributor.recordDelivery(uint256,uint256,bytes32).lot (src/Distributor.sol#274) shadows:
- Distributor.resolve(uint256,uint256,bytes).lot (src/Distributor.sol#215) shadows:
- Distributor.choose(uint256,uint256,Distributor.Mode,address).lot (src/Distributor.sol#148) shadows:
- Distributor.openDistribution(uint256,uint256,bytes32,string).lot (src/Distributor.sol#296) shadows:
- Distributor.openDraw(uint256,uint256,bytes32,uint256,string).lot (src/Distributor.sol#166) shadows:
- Distributor.requestDraw(uint256,uint256).lot (src/Distributor.sol#192) shadows:
- Distributor.redraw(uint256,uint256).lot (src/Distributor.sol#200) shadows:
- Distributor.cancelDraw(uint256,uint256).lot (src/Distributor.sol#180) shadows:

## Low: `timestamp` — 25

**By design.** Every delay in the system (the hour before a release, the 15-minute window, 7-day role changes, deadlines) is a comparison against `block.timestamp`. The shortest that matters is 15 minutes; a sequencer's freedom over the timestamp is seconds. The one place where seconds mattered was the draw's pin horizon, fixed in this phase (M2).

- ListingRegistry.acceptRoot(uint256) (src/ListingRegistry.sol#162-169) uses timestamp for comparisons
- AssetVault.isBlocked() (src/AssetVault.sol#441-447) uses timestamp for comparisons
- ListingRegistry.acceptTreasury() (src/ListingRegistry.sol#217-224) uses timestamp for comparisons
- Distributor.voidDraw(uint256,uint256) (src/Distributor.sol#229-240) uses timestamp for comparisons
- AssetVault.payersFor(uint256) (src/AssetVault.sol#168-176) uses timestamp for comparisons
- AssetVault.release(uint256) (src/AssetVault.sol#295-330) uses timestamp for comparisons
- Distributor.setDestination(uint256,uint256,string) (src/Distributor.sol#260-271) uses timestamp for comparisons
- ListingRegistry.acceptBondBps() (src/ListingRegistry.sol#202-208) uses timestamp for comparisons
- ListingRegistry.acceptBtcVault(uint256) (src/ListingRegistry.sol#179-186) uses timestamp for comparisons
- Distributor.requestDraw(uint256,uint256) (src/Distributor.sol#191-196) uses timestamp for comparisons
- Distributor.rollOver(uint256,uint256) (src/Distributor.sol#342-351) uses timestamp for comparisons
- AssetVault.writeOff(uint256,bytes32) (src/AssetVault.sol#226-252) uses timestamp for comparisons
- AssetVault.declarePurchase(uint256,uint256,string) (src/AssetVault.sol#192-210) uses timestamp for comparisons
- ListingRegistry.acceptRoles() (src/ListingRegistry.sol#141-147) uses timestamp for comparisons
- Distributor.cancelDraw(uint256,uint256) (src/Distributor.sol#179-188) uses timestamp for comparisons
- AssetVault._credit(address,uint256) (src/AssetVault.sol#151-164) uses timestamp for comparisons
- Distributor.claimShare(uint256,uint256,address,uint256,bytes32[]) (src/Distributor.sol#322-335) uses timestamp for comparisons
- Distributor.recordDelivery(uint256,uint256,bytes32) (src/Distributor.sol#273-281) uses timestamp for comparisons
- Distributor.redraw(uint256,uint256) (src/Distributor.sol#199-204) uses timestamp for comparisons
- Distributor.cancelDistribution(uint256,uint256) (src/Distributor.sol#311-319) uses timestamp for comparisons
- AssetVault.withdrawBond(uint256) (src/AssetVault.sol#275-289) uses timestamp for comparisons
- AssetVault._close(uint256) (src/AssetVault.sol#431-436) uses timestamp for comparisons
- Distributor.flagPayoutOverdue(uint256) (src/Distributor.sol#380-385) uses timestamp for comparisons
- Distributor.flagOverdue(uint256,uint256) (src/Distributor.sol#283-288) uses timestamp for comparisons
- Distributor.requestPayout(uint256,string) (src/Distributor.sol#362-371) uses timestamp for comparisons

