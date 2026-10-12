# Rook second review (R2): Solidity fixes (areas A, B, C)

Reviewer R2, 2026-09-26. Checkout `/Users/jlohayon/Documents/Rook-rev2-contracts`, detached at `fixes-2026-09-26` (head `46a4f1e`).
Baseline for the diff: `e04f49b`.

What I did:

- `lib/` was copied from `~/Documents/Rook/lib`, and the copied `.git` files were renamed `.git.bak-copied`.
- I added PoC tests under `test/audit2/` only. `src/`, `script/` and `docs/` were not touched.
- No commit, push, deploy, key or chain write.

Inputs:

- A/B/C original reports;
- `fixes-A-graduation.md` and `fixes-B-C-keeper-draws.md`;
- `git diff e04f49b..HEAD -- src script test` (60 files, +3,510/−545);
- a line-by-line read of every changed contract: FloorModule, FrogCurve, CurveFactory, LiquidityLock, SweepVault, Wrapper,
  ListingRegistry, ProtocolTreasury, MintDesk, HomeListing and IKeeperSource;
- the relevant parts of DeployStack and `golive-check.sh`.

Scope note: the "1 Critical, 6 High, 18 Medium, 26 Low" totals cover areas A–E. Areas A–C (Solidity) hold 39 findings: 1 Critical,
10 Medium, 13 Low, 15 Info. **None of the six Highs is in A–C**; they are in D/E (canister, services), which I did not review.

---

## 1. Status of every original A/B/C finding

Legend: **Fixed** means the root cause is removed in code and I checked that the regression fails for the right reason, not an
incidental one. **Partially** means the main vector is closed but a material part remains. **Not fixed** means the code is
unchanged (the reason is noted).

### Area A (curve, graduation, lock)

| ID | Sev | Verdict | Why |
| --- | --- | --- | --- |
| A-01 | Critical | **Fixed** | See the four points below the table. |
| A-02 | Low | **Fixed** | `gross` is clamped to `msg.value` (`FrogCurve.sol:266`), so `fee = gross − net ≥ 0` because `net = room ≤ msg.value − ⌊msg.value·rate⌋`. The audit's fuzz property passes. |
| A-03 | Medium | **Fixed**, with a residual (N-3) | There is a launch gate and a close gate (`hasRoomFor`), and a curve no longer closes into a bound refund while the module is full. Residual: the gate ignores freshness and the new keeper pause, so a close during a pause is still bound to refund (N-3, PoC). |
| A-04 | Low | **Fixed** | Shared adds are referenced to the module's `price()`, which is a TWAP of custodian fills clamped ±3% of `accepted` and not attacker-movable, within ±200 ticks. The ±990 push now defers, and the ±190 push P&L is about 0. |
| A-05 | Low | **Fixed** | `sweepViaDesk` is `onlyKeeper`, and `capFresh()` is false while the keeper is paused. |
| A-06 | Info | **Fixed** | `coinKey` reads `curve.sweepOnly()` before seeding. |
| A-07 | Info | **Fixed** | `buy`/`sell` deadline overloads. |
| A-08 | Info | **Fixed** | `MintDesk.collect` checks `transfer`. |
| A-09 | Info | Not fixed (accepted) | Unbounded protocol-position loop. Growth is at most 3 positions a day, which is fine. |
| A-10 | Info | **Fixed** (on the B branch) | `gradPrice()` counts `target` only up to `max(anchor·1.2, accepted)`. |

A-01 in detail:

1. **The bounds are no longer cheap ticks.** Both the far shared layer and the coin-pool position are ±180,000 ticks around a
   reference (`_farRange`). I re-derived the cost. A one-spacing position at a bound tick `T` costs `L_max·s·1e-4/2 / √P_T` in
   currency0, or `·√P_T` in currency1. That cost does not depend on the current pool price, so pre-initialising the pool at a
   chosen price does not make it cheaper. For each ordering (coin as currency0 or currency1; ETH/wX) the cheaper side is at least
   about 5e24 wei of wX/ETH, or about 3e32 coin-wei against a 1e27 supply. Both are infeasible.
2. **Fail soft.** Seed and shared add run in self-calls, and a revert inside them defers.
3. **The `_caught` gas guard is sound.** After an inner OOG, the caller keeps at most `g·(1−(63/64)^k)` of its gas. At this path's
   call depth of 4–5 that is under 8%, below the `g/8` threshold, so a gas-starved call reverts instead of deferring.
4. **Escape hatch.** It opens at `window + 4 d` for filled and SWEEP_ONLY curves.

I found no attacker-inducible persistent revert in `_seedCoinPool` or `seed`. A pre-initialised coin pool still only sells to the
lock below its own valuation, and `liq` is always > 0.

### Area B (floor module, wrapper)

| ID | Sev | Verdict | Why |
| --- | --- | --- | --- |
| B-01 | Medium | **Fixed** (the dust-tier vector) | A burn spans the tiers (`_release`), and the paid ≤ released rounding holds in both tiers. The broader burn-liveness property it motivated is broken a different way by N-1 (below). |
| B-02 | Medium | **Fixed** | The reserve basis (`reserve`) and `surplus` are split. Reserve-tier burns are capped at `reserve/reserveUnits`, and the spread goes to `surplus`. `accepted` is bounded to ±20% per day window (verified: 47 h ⇒ ≤ 1.44×). The residual keeper reach (cheap mints on a walked-down floor, ≤ ~20%/day × `maxPublicEthBacked`) is stated in the NatSpec. |
| B-03 | Medium | **Fixed** | `recordDelivery` is custodian-only. |
| B-04 | Medium | **Fixed** for a *lost* keeper | `escaped()` opens 7 d + 1 h after the last good post, burns then pay β less the base spread, and `skim` releases the surplus. Not true for a *compromised or paused* keeper: see N-1 (new, Medium). |
| B-05 | Low | **Fixed** | `wireFloorModule` requires `module.wrapped() == this`. `golive-check` is strict under `--phase1` (script not executed by me). |
| B-06 | Low | Not fixed (deferred in fixes doc §7) | `itemArrived` still checks the absolute wX balance. Custodian-only. |
| B-07 | Low | **Partially** | A public mint can no longer force a refund. It can still hold room: `mint` checks only `publicCap` on convertible units, not `maxEthBacked − queued`. So it can front-run a closing buy into `QueueFull` (a delay, not a loss), or take room from a stale-floor Queued grad, which then refunds. |
| B-08 | Low | Not fixed (deferred) | — |
| B-09 | Info | **Fixed** | `pauseKeeper` (owner or guardian), and the module honours it on every inflow. Its effect on burns is worse than documented (N-1b). |
| B-10 | Info | **Partially** | The `itemArrived` and header NatSpec are corrected. The `burn` NatSpec ("Never paused: an exit does not depend on any key") and fixes-doc §0/§5-16 ("Holders can burn throughout") are false (N-1). |
| B-11 | Info | Not fixed (deferred to stage 1) | — |
| B-12 | Info | Not fixed | `snapshotCap` is still permissionless, so the custodian can still convert at a snapshot for 2 h after a pause (N-7). |
| B-13 | Info | n/a | Confirmation only. |

### Area C (treasury, sweeps and draws, roles, deploy)

| ID | Sev | Verdict | Why |
| --- | --- | --- | --- |
| C-01 | Medium | **Fixed** | `postCap` reverts `NoCeiling` without a ceiling. The first ceiling is instant and DeployStack requires it on a real chain. A stale re-post may rise at most 20%. Residual (Info): fresh steps still compound about 3×/h up to the ceiling. The ceiling is a static wei value, so if the floor outgrows it, sweeps stall until a 7-day raise. |
| C-02 | Medium | **Fixed** | `redraw` is removed from SweepVault and Wrapper, and a pinned round is final. Residuals: an unresolved pinned draw is voided after 7 d, and a Wrapper ticket is refunded (a re-roll only if *nobody* resolves, which relies on the off-chain resolver). C-13 lag is now the only on-chain grinding surface. |
| C-03 | Medium | **Partially** | The sentinel leaf commits `totalTickets` and `snapshotAt`, and the keeper no longer supplies `snapshotBlock`. But the list is still the keeper's, and detection is off-chain (monitor recompute). `requestDraw` is anyone's at +15 min, so the effective response window is still 15 min, unless the guardian pauses the keeper, which blocks `requestDraw`. The guardian is optional (`--no-guardian`). |
| C-04 | Medium | **Fixed** against traders; residual N-2 | The snapshot time is drawn uniformly from the 24 h *before* the announcement by a later drand round, so JIT holding captures only in proportion to time held. The keeper, owner or guardian can still discard a snapshot time after seeing it (N-2, PoC). |
| C-05 | Medium | **Fixed** | Same as B-03. |
| C-06 | Low | Not fixed (accepted) | NatSpec now states the Safe can take everything at once. Trust page not verified (web). |
| C-07 | Low | **Fixed** | Constructor defaults are 9000/0/0/1000 with loan 0. `setShares` moves the floats and bootstrap pots into reserves when their share goes to 0. |
| C-08 | Low | Fixed per script diff | `--phase1` turns notes into FAILs and adds the Safe, 7702, pending*, guardian, shares and `module.registry` reads. Not executed by me. |
| C-09 | Low | Out of scope (web) | Not verified. |
| C-10 | Low | **Fixed** | Same as A-05. |
| C-11 | Low | **Partially** | The module reads `registry.keeper()`, which is rotatable in 7 d and pausable instantly. The single hot key still holds every keeper power (floor, caps, draw roots); no separate draw key (fixes doc §8.3). |
| C-12 | Info | **Fixed** | `fundReserve` is called in `try`. Residual: an owner-registered module that burns all gas still reverts `distribute` (63/64), but only the owner can add modules. |
| C-13 | Info | Not fixed (open decision) | Pins still use `roundAt(block.timestamp)+20`. |
| C-14 | Info | Not fixed (open decision) | — |
| C-15 | Info | Not fixed | `CollectionDirectory` unchanged. |
| C-16 | Info | By design | — |

Tally for A–C (39 findings):

| Verdict | Count | Findings |
| --- | ---: | --- |
| Fixed | 23 | Includes A-03, B-04 and C-04, whose residuals are reported as the new findings N-3, N-1 and N-2 |
| Partially | 4 | B-07, B-10, C-03, C-11 |
| Not fixed (deferred or accepted) | 9 | A-09, B-06, B-08, B-11, B-12, C-06, C-13, C-14, C-15 |
| n/a, by design, or not verifiable here | 3 | B-13, C-16, C-09 |

---

## 2. New findings introduced or exposed by the fixes

| ID | Severity | Title | Location | PoC | Result |
| --- | --- | --- | --- | --- | --- |
| N-1 | **Medium** | The module's exit is not live through a keeper incident: (a) a compromised keeper keeps the module *fresh* with a 1-wei `target`, so every burn pays 0 and the stale fallback never opens; (b) the recovery path (pause → 7-day rotation) freezes all burns from +1 h to +7 d 1 h | `FloorModule.sol:485-497` (`_tierPrices` uses raw `target`), `:936-940` (`escaped` needs `!fresh`), `:462` NatSpec | `test/audit2/R2Floor.t.sol` | 2/2 PASS (attack works) |
| N-2 | Low | The C-04 snapshot time can be re-rolled by the privileged roles after it is visible: the keeper by not opening (anyone voids after 6 h), and the guardian or owner by cancelling an Open draw before its pin | `SweepVault.sol:399-470, 521-535` | `test/audit2/R2Draws.t.sol` | 2/2 PASS (29 re-rolls to land the snapshot in a chosen hour; guardian cancel after reveal) |
| N-3 | Low | The A-03 close gate ignores the keeper pause (and freshness), but `_tryFill` requires both: a curve that closes during a pause cannot be filled and is refundable after its 3-day window, the outcome A-03 was meant to prevent. The runbook pause lasts 7 days | `FloorModule.sol:684-689` vs `:662-667`; `FrogCurve.sol:269` | `test/audit2/R2Curve.t.sol` | 1/1 PASS |
| N-4 | Low / Info | The shared-add reference (±200 ticks of `price()`) defers adds in ordinary markets, and anyone can force a deferral at graduation with a same-transaction price push (no loss; `deploy()` retries) | `LiquidityLock.sol:417-455` | analysis | — |
| N-5 | Info | After the escape hatch opens, the first caller chooses refund over graduation even for a healthy curve, so the runner must graduate promptly. Holders of a dud SWEEP_ONLY coin have an incentive to wait 7 days and refund | `FrogCurve.sol:384-401` | analysis | — |
| N-6 | Info | `NftRegistry` role separation reads `r.keeper()`, which is 0 while paused, so a buyer equal to the named keeper passes during a pause (NFT is OFF and not deployed) | `src/nft/NftRegistry.sol:114` | — | — |
| N-7 | Info | "Every flow priced by the keeper's word stops on pause" is not quite true: `itemArrived` still pays up to `itemCap` for the rest of the hour, and up to a permissionless `snapshotCap` for 2 h | `FloorModule.sol:536-542, 596-603` | — | — |
| N-8 | Info | `setGuardian` has no role-separation check (the guardian may be the keeper, purchaser or owner). `golive-check --phase1` catches it | `ListingRegistry.sol:298` | — | — |

### N-1 (Medium): burns are not live through a keeper incident

**(a) A fresh freeze.** A reserve-tier or convertible-tier burn pays `min(price(), target) × (1 − s)`, capped by β
(`_tierPrices`). `target` is the keeper's *raw* post. `postFloor` clamps only `accepted`. A compromised keeper posts `priceWei = 1`
with confidence 100% every ≤ 60 min:

- the module stays `fresh()`;
- `lastGoodAt` keeps refreshing, so `escaped()` is never true;
- every burn computes `eth == 0` and reverts `Slippage`, and `burnCapacity()` returns (0, 0), so the router skips the module;
- mints still work at `max(price(), target)`, which is the normal (walked-down) price, so new users keep paying ETH in that they
  cannot burn out.

The freeze lasts until someone pauses the keeper. Without a guardian (which is optional), that is a solo operator at two Safe
devices.

The PoC runs 8 days of such posts: fresh the whole time, never escaped, `quoteBurn = 0`, and a new victim mints 1 unit that it
cannot burn. 2.04 ETH of backing is unreachable.

**(b) The pause itself freezes burns for a week.** After `pauseKeeper`, nobody can post, so one hour later `fresh()` is false.
`_tierPrices` then reverts `Stale` until `lastGoodAt + FLOOR_TTL + STALE_ESCAPE`, which is about 7 d 1 h. The PoC shows burns work
at +0, revert at +61 min and at +3 d, and work again only after escape.

The design's own runbook is: pause → `proposeRoles` → 7 days → accept → unpause. That means about 7 days with no exit on every
incident, including a guardian false positive, unless the Safe unpauses. This contradicts:

- the `burn` NatSpec, "Never paused: an exit does not depend on any key";
- fixes-B-C §0, "Burns are not paused";
- §5 item 16, "Holders can burn throughout (at ≤ backing)".

Existing `test_B09` burns only inside the first hour, and the BurnLiveness handler bounds `target` to `[a/2, 2a]` and never pauses,
so neither catches this.

**Fix.**

- Bound the `target` used for burns (and for mints, for symmetry) into the day band, as `gradPrice` already does:
  `tB = max(target, min(anchor·(1 − MAX_DAY_MOVE_BPS), accepted))`. Alternatively, price burns off `accepted`/`price()` only.
- Treat `registry.keeperPaused()` (and any non-fresh state) as escaped for burns, or at least shorten the wait while paused (for
  example to 1–24 h). The escape price is `β × (1 − base)`, which is safe by construction (M4), so there is no reason to wait 7 days
  once the keeper is known to be paused.
- Add both cases to the liveness invariant: a handler action that posts a tiny target, and one that pauses.

### N-2 (Low): privileged re-roll of the raffle snapshot time

**How it works.**

1. `announceDraw` pins `seedRound`.
2. A minute later the seed is public, and so is `snapshotTimeOf(id, seed)`.
3. The keeper opens only if it likes the time. Otherwise the draw lapses, and anyone voids it after `OPEN_TTL`.
4. The keeper re-announces with a new round and a new time.

In the PoC the keeper aimed the snapshot at the hour before the announcement, and got there after 29 discards (about 7 days at
6 h each). A wider target window takes proportionally fewer. The guardian or owner can do the same to an **Open** draw, whose
snapshot time and ticket list are public, because `cancelDraw` now works any time before the pin.

**Impact.** This contradicts "a cancel steers nothing" and "nobody can see an outcome and pick another round". The impact is limited
to parties that are already trusted, but "declining to open" is indistinguishable from downtime, unlike a forged root, which a
recompute catches.

**Fix.**

- Make the snapshot time a function of a round pinned *before* the announcement can be withdrawn. For example, on a void or cancel,
  the next draw for that coin must reuse the same `seedRound`/`snapshotAt`: store `nextSnapshotAt[coin]` and consume it.
- Or limit `cancelDraw` to the 15-minute challenge window for Open draws, and to Announced draws only before the seed round exists.
- Monitor and page on `DrawVoided` for Announced draws.

### N-3 (Low): a close during a keeper pause is bound to refund

`hasRoomFor` deliberately ignores freshness and now also ignores `keeperPaused`. `_tryFill` returns false for either, and
`fill()` reverts `NoRoom` while paused.

The PoC:

1. The guardian pauses.
2. `canClose()` still returns true, and the closing buy goes through.
3. The grad is Queued and cannot be filled.
4. After 3 d 1 h anyone opens refunds.
5. The 8 ETH raise is refunded pro rata; the 0.081 ETH of trade and launch fees is kept.

Launches are already blocked while paused (`capFresh()` is false), but curves already trading are not.

**Fix.** Add `!registry.keeperPaused()` (and optionally `fresh()`) to the close gate (`canClose` / `_buy`), so the curve keeps
trading. Or have `openRefunds` wait while paused, or extend the window by the paused time.

### N-4 (Low/Info): shared-add liveness

- The reference `price()` is a TWAP of custodian fills clamped to ±3% of `accepted`.
- `accepted` lags the market by 5%/h and 20%/day.
- The module's arbitrage band is up to ±10% (spreads), and mints stop at `publicCap`.

So a fair pool can sit more than 2% from `price()` for hours or days, and every graduation's shared add and protocol-liquidity
add defers (reasons 1/2). Anyone can also force a deferral inside `graduate()` by pushing the pool more than 200 ticks and back.
No funds are lost, since `deploy()` is anyone's, but a keeper or runner should retry, and the site should show pending amounts.

---

## 3. Other checks (no finding)

**Reentrancy through the new self-calls.** `seedCoinPoolSelf`/`deploySelf` require `msg.sender == address(this)`. The lock makes
no call with attacker-chosen target or calldata. Outer entry points hold `nonReentrant`. `unlockCallback` checks the PoolManager.
ETH for SWEEP_ONLY deferred seeds (`pendingCoinQuote`) is kept apart from `pendingEth`, and `Overspent` bounds every settle.

**Accounting.**

- `reserve` is never stranded: `_split` gives both parts or neither, and the last unit takes the pot.
- The `backing` pot is fully released when convertible units reach 0.
- `surplus` only grows by fundReserve, itemSold and reserve-tier spread, and only leaves through itemArrived top-ups or an escaped
  `skim`.
- The existing `_books()` solvency assertion includes `surplus` and holds in my PoCs.
- Escape-hatch refunds (FrogCurve) release exactly `g.units` and `gradUnits −= units`; nothing is paid twice.

**Rounding at the new boundaries.**

- `floorBounds` rounds dLo up and dHi down.
- In `_release`, `paid ≤ rel` in both tiers, because `pr ≤ ⌊reserve·1e18/r⌋`.
- The sentinel leaf cannot be claimed: holder 0 is refused, and a double-hashed leaf cannot collide with an internal node.

**Ownership and roles.**

- `proposeRoles` and `_requireNoRole` use `_keeper`, not `keeper()`, so a pause cannot be abused into an instant role set.
- Rotation during a pause keeps it paused until the owner unpauses.
- With the guardian unset, only the owner can pause or cancel.
- With the keeper paused, `keeper()` is 0; no `msg.sender` is 0.
- Wrapper pinned-refund: `openTickets` and the supply stay in step (W1 holds). `_skipRefunded` advances the order correctly for
  refunded head and non-head tickets.
- `drawInFlight` is cleared on every exit from Announced/Open.

**Clocks.** Every new wait (day window, escape, OPEN_TTL, PIN_TIMEOUT, REFUND_GRACE, STUCK_GRACE) uses `Wait.elapsed` on both clocks.
A lagging parent block count only lengthens waits.

---

## 4. Test results

- **`forge test` (full suite, default profile): 996 passed, 0 failed, 0 skipped (108 suites).** Log:
  `scratchpad/rook-audit/R2/forge-test-full.txt`. Fork suites return early without `--fork-url` (not run).
- **`forge test --match-path 'test/audit2/*'`: 5 passed** (every PoC reproduces its issue):

  | Test | Result |
  | --- | --- |
  | `R2Floor::test_N1a_freshTinyTargetFreezesEveryBurn_noEscapeEver` | PASS (accepted 0.1327 ETH after 8 d; 2.036 ETH of backing not burnable) |
  | `R2Floor::test_N1b_aKeeperPauseFreezesBurnsForAWeek` | PASS |
  | `R2Draws::test_N2a_theKeeperRerollsTheSnapshotTimeByNotOpening` | PASS (29 discarded announcements) |
  | `R2Draws::test_N2b_theGuardianCancelsAnOpenDrawAfterSeeingItsSnapshot` | PASS |
  | `R2Curve::test_N3_aCurveThatClosesDuringAKeeperPauseIsForcedIntoRefunds` | PASS (8 ETH refund pool, 0.081 ETH fees kept) |

- **The original A/B/C PoCs, now regression tests** (`test/audit/{A,B,C}`), are included in the 996 and all pass. For each Critical
  and Medium I checked that the regression asserts the mechanism, not an incidental revert:
  - A-01: saturating the old bound is shown harmless, the new bound is measured at 3.4e9 / 7.3e6 ETH, and deferral with retry is
    shown;
  - B-02: the payout is ≤ basis and the surplus is untouched;
  - C-01: `NoCeiling`, `CapStep`, `OverCeiling` with the exact selector arguments.

## 5. Slither (0.11.6, `slither . --filter-paths "lib|test|script"`)

486 results, against 457 at `e04f49b`. Outputs: `scratchpad/rook-audit/R2/slither.{json,txt}`. Compared without line numbers,
there are 45 new-only entries. All are benign or false positives:

- `incorrect-equality`, `timestamp` and `uninitialized-local` on the new code paths;
- `reentrancy-benign`/`-events` in the lock's seed and deploy, where the PoolManager is trusted and the entry points are
  `nonReentrant`;
- `reentrancy-no-eth` in `SweepVault.openDraw`, a call to the immutable beacon;
- `missing-zero-check` in `setGuardian`, where zero means none, by design;
- `unused-return` on `release().spent` in `openRefunds` and on the `gradPrice()` probe in `wire`, both intended;
- `var-read-using-this` in `launchesOpen`, which is intended (try/catch);
- `missing-inheritance` notes.

No detector flags N-1 to N-3. They are economic and design issues.

## 6. Not covered

- **Areas D and E**: the canister (Rust), timelock, keeper, runner, monitor, indexer and site. That includes the six original
  Highs and the off-chain half of several fixes: the resolver, the monitor recompute of draw roots, guardian automation, and the
  keeper's `planPost` within the day band.
- `golive-check.sh`, `deploy.test.sh` and `golive-dryrun.sh` were read (diff) but not executed. Fork tests (`--fork-url`) and the
  `deep` fuzz profile were not run.
- Home-chain, NFT, v1 `AssetVault`/`Distributor`: only where the diff touched them (HomeListing forwarding) plus the N-6 grep.
- Uniswap protocol-fee effects and 4663 gas measurements.
- The economic sizing of N-4 on real pool depth.
