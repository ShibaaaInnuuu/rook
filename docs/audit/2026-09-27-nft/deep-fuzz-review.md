# Deep-fuzz review, 2026-09-28: confirmation of the DF-M-1 fix (branch `invariant-fix`)

Reviewer: independent AI re-review. Worktree `/Users/jlohayon/Documents/Rook-invariant`, branch `invariant-fix` at `073f606`. Scope:
`git diff 47c6cf5..073f606` against `deep-fuzz-2026-09-28.md`. No contract code was modified. Nothing was pushed, deployed or
signed. Runs were in private copies under `/tmp/rook-dfreview/`. The only chain written to was the stack dry run's local fork anvil,
on port 8671.

## Verdict

**The fix is correct. DF-M-1 is closed. There are no new C, H, M or L findings, only three Infos. The branch is at 0 C / 0 H / 0 M.**

| Check | Result |
| --- | --- |
| The window is gone | `recordReturn` (`Wrapper`, `NftWrapper`) does `ethBacked += ONE_ITEM`, then emits its event, then calls `rentalReturned`. That call runs `backing += escrow` in the same call frame. Between the two writes there is no external call and no module entry point. |
| A legitimate return cannot revert | The wrapper reaches `Ready` only from `Leaving` or `LeavingDrawn`. `Leaving` is set only by `returnRental`, which only `endRental` calls, and `endRental` sets the module's rental to `Leaving`. A module `Leaving` rental closes only through `settleRental`'s taken branch, which needs the wrapper in `Taken`, a terminal state. So wrapper `Ready` implies module `Leaving`, and `WrongStatus` cannot trigger. `rentalReturned` checks no pause, keeper or freshness. The PoC returns with the keeper paused, the poster paused and the floor stale. |
| Upgrades or a missing mapping | Nothing is upgradeable: there is no proxy or delegatecall in `src/`, the module's `wrapper`/`_w` are `immutable`, and `wireFloorModule` runs once and only on a fresh wrapper. There is no public deployment (`deployments/README.md`). No rental can exist from before `_rentalOfItem`. |
| Re-renting an item | Item ids are never reused. The frog side uses `DEPOSIT_FLAG \| depositCount++`, and `NftWrapper` uses `_items.length`. A re-rent of the same token gets a new id, so the mapping is written once per id and never overwritten. The PoC `test_nft_reRentSameToken` shows each return closing its own rental. |
| A never-rented item, or one already closed | `NothingToSettle` / `WrongStatus`, so the escrow is never credited twice. Both are reachable only by the wrapper, and the wrapper calls only from `Ready`. |
| Access control | `msg.sender == address(wrapper)`, the module's immutable wrapper. `Wrapper` also checks `module.wrapped() == this` at wiring (B-05). `NftWrapper` does not, but a module built for another wrapper can never `admitRental` on it (`onlyModule` + `_w`), so it can never have a rental to return. |
| Reentrancy | The wrapper→module call is last (CEI). `rentalReturned` is `nonReentrant` and makes no external call. `recordReturn` is custodian-only, and the module never calls it. The `NftWrapper` custodian must have no code. A custodian contract on the frog side could only make its own `recordReturn` revert, by calling it from inside a module lock (an ETH receive hook). That is a self-DoS with no loss. |
| Taken path | Unchanged: the same branch and the same arithmetic. The removed `else if` is exactly the body moved into `rentalReturned`. |
| Accounting | M3 holds at return (`B += escrow` with the unit). `rentalEscrow` now counts only `Lent`+`Leaving` escrows. `leavingRentals`/`rentalRoom()` free a slot at the return instead of at the settle, which is correct because the item is home. The deep campaign passes `ethIsConserved`, `theModuleIsSolvent`, `noBackingWithoutUnits`, `burnsNeverPayAboveTheBacking` and `theReserveStands` across all rent/return/convert/burn/advance orders. |
| Control | Removing the one `rentalReturned` call from `Wrapper` (`/tmp` copy) fails `invariant_noBackingWithoutUnits` ("a returned rental left open") and the replay (`0 != 633949500000000000`). The tests do detect the bug. |
| Sizes (runtime) | `FloorModule` 30,417 B (> EIP-170, < 4663's 98,304 used by `--code-size-limit`), `Wrapper` 18,752, `NftWrapper` 13,942. `EthNftWrapper` 11,752 is unchanged. |
| Off-chain meaning | The canister, keeper, indexer and ops code call neither `settleRental` nor `recordReturn`. The monitor decodes the wrapper's `RentalReturned` (an OFF-path SEV2 alert), and its topic and emitter are unchanged. Nothing decodes `RentalSettled`. `ops/src/backing.ts` reads `rentalEscrow` as a book, and the conservation sum is unchanged. The one new selector is `rentalReturned(uint256)`. No existing selector or event signature changed. |
| Phase 1 exposure | None. Frog `rentIn` is custodian-only and needs a `rentalRoot` that the keeper never posts, and the canister has no rental path. `EthNftWrapper.admitRental`/`returnRental` revert `NoRentals`, and its `recordReturn` is untouched. |

## Infos

| ID | What | Suggestion |
| --- | --- | --- |
| DF-I-1 | Outside the invariant handler, no unit test covered `recordReturn`/`settleRental`, and `NftWrapper` rentals had never run against a real `FloorModule`. The fix's `NftWrapper` edit was covered only by review. | Adopt `deep-fuzz-review-poc/DfReviewNft.t.sol` (5 tests: round trip, return while paused/stale, re-rent, wrong status, burn after return) into `test/`. |
| DF-I-2 | `recordReturn` now reverts if the paired module lacks `rentalReturned`, for example a wrapper from this commit wired to a module from an older one. Wiring is one-shot, so such a pairing would brick returns (only with rentals ON). | Deploy the wrapper and module from the same commit, as the deploy scripts already do. When rentals are switched on, size the canister's `recordReturn` gas for the extra call (about 30k). |
| DF-I-3 | `returnedUnsettled` only fires when the handler's `settle` samples a returned rental. | Optionally assert `rentalEscrow == Σ escrow(Lent ∪ Leaving)` over the handler's rentals, a direct form. |

## Test results

- `forge test --no-match-path "test/fork/*"` (default profile): **1,117 passed, 0 failed**.
- `FOUNDRY_PROFILE=deep` (1,000 runs × 200 calls): `FloorModule.invariant` 12/12 invariants (200,000 calls, 0 reverts) plus both new
  tests. `Wrapper.invariant` 5/5, `NftWrapper.invariant` 8/8, `EthNft.invariant` 3/3: all pass.
- `FOUNDRY_PROFILE=deep`, every `invariant_*` suite with CI's seed `0xe5f97b93…eb311`: **23/23 suites pass**.
- PoC `DfReviewNft.t.sol` (copied into `test/` of a private copy): 5/5 pass.
- The monitor (`node --test`): 219/219. The indexer: 170/170. Ops: 66/66.
- `STACK_PORT=8671 scripts/golive-dryrun.sh stack`: **DRY RUN: ALL PASS**, run in a private copy with `monitor/`, `ops/` and `indexer/` dependencies installed. A first run in a copy without `monitor/node_modules` failed only its canister install-argument checks (an environment gap); every contract phase passed there too.
