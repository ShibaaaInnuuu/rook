# Fix round 4 (R4): the contracts' answers to the targeted check of round 3

Branch `r4-contracts` (from `fixes-2026-09-26`, head `4a215f4`, the R3 check). Written before the code, as the design; the "As
built" section at the end is filled in when the code lands. Nothing here is deployed, and no key, wallet or live chain is touched.
Network use is read-only (the public drand relays and a local anvil fork of the public 4663 RPC in the dry run).

Inputs: `review3/R3-check.md` (A3-M1, A3-L1, A3-I1, A3-I2 on the contracts), its PoCs `review3/poc/Audit3{Floor,Curve,Draws}.t.sol`,
and the round-3 designs `review2/fixes-R3-contracts.md` (N-1b, N-3) and `review2/fixes-R3-carry.md`. Two other branches fix the
canister (`r4-canister`) and the services (`r4-services`); this one stays in `src/`, `script/`, `test/` and the contract docs. What
those branches must pick up from here is in §6.

## 0. Summary

| ID | Sev | Fix | Where |
| --- | --- | --- | --- |
| **A3-M1** | Medium | While the keeper is paused (or zero) and the stale window has not passed, a burn no longer pays the average backing β. It pays `min(β, F_p) × (1 − base)` per tier, where `F_p` is the **pause floor**: the least any mint could have paid per unit, spread aside, under the floor the keeper left (which cannot move while paused). β is paid only once `escaped()` (a week with no good post), as before round 3. No mint is possible while paused; a mint → pause → burn round trip never profits | `FloorModule._tierPrices`, new `pauseFloor()`, `burnsOpen()`; `exitAtBacking()` now means `escaped()` alone |
| **A3-L1** | Low | The registry stamps the unpause (`keeperResumedAt`, both clocks). For `RESUME_GRACE` (6 h) after it the module still holds unfilled curves' refunds, whether or not the floor is fresh again, so the keeper's first post and the runner's `fill` come before anyone can bind a held graduation to refund. Bounded by the curve's escape hatch as now | `ListingRegistry.unpauseKeeper`, `IKeeperSource`, `FloorModule.holdsRefunds` |
| **A3-I2** | Info | `openDraw` with `totalTickets > 0` refuses an empty sentinel proof: a list with tickets has at least one holder leaf beside the sentinel, so its proof is never empty | `SweepVault.openDraw` |
| **A3-I1** | Info | Not changed in the contracts (the stale keeper that is not paused is the documented ordinary outage). With A3-M1 fixed the monitor may pause on `no-good-post` sooner (services, §6) | — |

## 1. A3-M1: price paused burns so that a pre-positioned mint cannot profit

### 1.1 The gap

Round 3 (N-1b) made a burn pay `β × (1 − base)` per tier as soon as the keeper is paused. β is the tier's average paid-in basis.
When the floor has fallen, β sits above the floor, and a unit minted on the fallen floor just before any pause (a guardian false
positive, the monitor's deadline pause) exits at the higher average: the PoC makes +0.206 ETH (+10 %) on 2.05 ETH, the difference
taken from the existing holders' cushion above the floor.

### 1.2 What a paused burn pays

Nobody can post while the keeper is paused (`postFloor` needs `registry.keeper()`, which is zero), so `accepted` and `target` are
exactly what they were when the pause began: the floor at the pause needs no extra storage, it is the frozen state. From it:

    F_p = min( burnTarget,  max( accepted × (1 − fillBandBps),  min(target, accepted) ) )

- `max(accepted × (1 − fillBandBps), min(target, accepted))` is a lower bound on what any mint paid per unit, spread aside, while
  the keeper's last post stood. A mint pays `max(price(), upTarget) × (1 + sMint)`; `price()` is clamped into
  `accepted × (1 ± fillBandBps)` and `upTarget = min(target, max(dayHigh, accepted)) ≥ min(target, accepted)`. With an ordinary
  keeper (`target == accepted`) `F_p` is simply the accepted floor, the reviewer's `acceptedAtPause`. It is lower only in a fall the
  move limit is holding back (`target < accepted`), where a mint could have cost less than `accepted`: taking `accepted` there
  would leave up to `fillBandBps` (3 %) of round-trip profit, so the paused price follows the mint's own lower bound.
- `burnTarget` (N-1a, unchanged) is `max(target, lo)` with `lo = max(accepted × (1 − maxMoveBps), min(dayLow, accepted))`: the
  paused price is never above what the fresh formula would count for the target, and never below the next hour's legal floor or the
  day band (the reviewer's `dayLow` term). So a 1-wei post before a pause cannot zero a paused burn either (`F_p ≥ min(lo,
  accepted × (1 − fillBandBps)) > 0`).
- Per tier: `pr = min(β_R, F_p) × (1 − base)`, `pc = min(β_C, F_p) × (1 − base)`. Each unit still releases exactly its tier's β
  (M4), and the difference stays in the pots' revenue / surplus as for any burn.

`F_p` can only fall during a pause: `accepted`, `target` and the fill band are frozen, and the only moving part, the day anchor in
`burnTarget`, can only lower `lo` when its window rolls over. Waiting inside a pause never improves the price.

### 1.3 The four states a burn can meet

| State | Burn pays per tier |
| --- | --- |
| Fresh, keeper live | `min(min(price(), burnTarget) × (1 − sBurn), β)` (unchanged) |
| Keeper paused or zero, not escaped | `min(β, F_p) × (1 − base)` (**new**; was `β × (1 − base)`) |
| Escaped (`FLOOR_TTL + STALE_ESCAPE` with no good post, both clocks) | `β × (1 − base)` (unchanged; paused or not) |
| Stale less than a week, keeper live | refused, `Stale` (unchanged: the ordinary outage, ended by the next post or by a pause) |

`exitAtBacking()` keeps its name and signature but now means exactly what it says: burns pay the backing, i.e. `escaped()`. The
new view `burnsOpen()` (`fresh() || keeperDown || escaped()`) says whether a burn is priced at all, and `pauseFloor()` returns
`F_p` while it applies (0 otherwise). `burnPrice`, `burnCapacity` and `quoteBurn` follow.

### 1.4 Why the attack now fails, and what the pause/unpause timing can and cannot do

- **Mint → pause → burn, the same keeper post.** The mint cost at least `F_p × (1 + sMint)` per unit (§1.2) and the paused burn
  pays at most `F_p × (1 − base)`: a loss of both spreads, as for any fresh round trip. Rounding is in the module's favour on both
  legs (units floored on the mint, ETH floored on the burn).
- **A mint under an earlier post.** Between that mint and the pause the keeper moved the floor. The paused price is never above
  the floor the keeper left (`F_p ≤ accepted`), and never above the least a mint under that last post could have paid. What the
  holder gains or loses is the floor's move since the mint, which a fresh burn before the pause would also have realised (less
  its spread); the average backing no longer enters.
- **No mint while paused.** `_priced` (mint, `fundFloat`, `itemSold`), `_tryFill`, `rentIn`, `itemArrived` and `snapshotCap` keep
  refusing while the keeper is down (round 3, unchanged).
- **Unpause timing.** After `unpauseKeeper` the module is stale (a pause always outlasts `FLOOR_TTL` unless it is lifted within
  the hour), so neither mints nor floor-priced burns run until the keeper's first post, which moves `accepted` by at most the hour's
  move limit from the frozen value. A pause lifted within the hour leaves the module fresh: a burn during the pause at `F_p` and a
  mint after it at `≥ F_p × (1 + sMint)` is a loss both ways round. Repeated pauses change nothing: each one prices from the same
  frozen state, never above it.
- **Escape.** A pause that lasts until a week has passed since the last good post reaches the stale fallback, which pays β as it
  always did (a week without a good floor: the floor number is no longer an exit anybody can rely on). This is the pre-round-3
  B-04 behaviour; the pause neither shortens nor lengthens it (it counts from `lastGoodAt`). The runbook lifts or rotates a pause
  within the 7-day role delay; a pause that outlives the escape is a governance failure, and then paying the backing is the
  intended last resort.
- **Always an exit.** Throughout a pause every ETH-backed unit burns at a non-zero price within `burnPerHour`: `F_p > 0` because a
  floor was accepted before any unit could exist (every inflow needs a fresh floor), and β > 0 for a tier with units.

What the holders give up against round 3: during a pause they exit at the frozen floor (at most the backing) instead of the
average backing. A holder whose β is above the floor can wait for the unpause (a fresh floor) or, in a genuine incident, for the
escape. That is the price of making every pause, false or not, worthless to an outsider.

### 1.5 Tests

- `test/audit3/Audit3Floor.t.sol`: the reviewer's PoC as a regression; the same steps must now lose (`got < paid`), and Alice's
  backing per unit must not fall on the attacker's burn. The confidence-0 case (A3-I1) is kept as it is (documented behaviour).
- A fuzz: a random history (a fall or a rise walked at random speeds, a fill at a random price inside the band, a hostile or tiny
  target), an outsider's mint, a random delay with no post, a pause, a random wait inside the stale window, a burn of everything:
  never a profit, and a paused burn never pays above the frozen floor.
- `BurnLiveness.invariant.t.sol`: a new handler action does the round trip (mint at the current state, pause, wait, burn) and
  flags any profit; the liveness invariant uses `burnsOpen()`; a paused burn is checked against `pauseFloor()`.
- `R2Floor.t.sol` (N-1b): the paused burns at +0, +61 min, +3 d and +6 d now pay `min(β, F_p) × (1 − base)`; +9 d (escaped) pays
  `β × (1 − base)`.

## 2. A3-L1: a held graduation gets a grace after the unpause

### 2.1 The gap

N-3 holds an unfilled curve's refunds while the keeper is paused. The hold ends the instant the Safe unpauses, but the floor is
then stale and nobody could post during the pause, so `fill()` reverts `NoRoom` until the keeper's first post. Anyone calling
`openRefunds()` in between (in the limit, back-running the unpause) binds the held graduation to refund.

### 2.2 Fix

- `ListingRegistry.unpauseKeeper` stamps `keeperResumedAt` / `keeperResumedAtBlock` (only on a real transition from paused).
  `IKeeperSource` gains both views.
- `FloorModule.holdsRefunds()` is true while the keeper is down **or** for `RESUME_GRACE = 6 hours` after the last resume, on both
  clocks. The grace does not end early when the floor turns fresh: ending at the first post would move the race to a back-run of
  that post. Six hours cover the keeper's first post and the runner's `fill` with room for a manual one.
- **Bounded.** `FrogCurve.openRefunds` already caps any hold at the curve's escape hatch (`window + STUCK_GRACE`, `_stuck()`); the
  grace adds at most 6 hours after each unpause, and only the Safe unpauses.
- **N-3 not reopened.** `hasRoomFor` stays false only while the keeper is down; a close during the grace is the ordinary "stale
  floor at the close" case (the grad queues and `fill` converts it at the next post), which the grace now also protects.
- `refundsOpenAt()` keeps reporting the hatch while the hold is on (the latest it can be); the site can show the grace's end from
  the registry's `keeperResumedAt + RESUME_GRACE`.

### 2.3 Tests

`test/audit3/Audit3Curve.t.sol`: the reviewer's race now fails (`openRefunds` → `TooEarly` right after the unpause, then the keeper
posts, anyone fills, the curve graduates). Also: the grace ends after 6 hours on both clocks and an unfillable curve then refunds;
the hatch still bounds it; a grace never starts without a pause (an unpause of an unpaused keeper stamps nothing).

## 3. A3-I2: a non-empty list needs a holder leaf

`openDraw` with `totalTickets > 0` reverts `BadProof` on an empty `sentinelProof`. A real list with tickets has at least one holder
leaf beside the sentinel, so its sentinel proof has at least one node; the sentinel-only root with `N > 0` tickets (a draw nobody can
win that still consumes a carried seed) is refused on chain instead of being left to the monitor's recompute. The monitor keeps
treating any mismatch as a bad root. `Audit3Draws.t.sol`'s PoC becomes a regression; its N-2 re-run stays green unchanged.

## 4. A3-I1 (not changed)

A keeper that posts at confidence 0 (or stops) still refuses burns (`Stale`) until someone pauses it or the week passes. Serving
stale burns while the keeper is live would let a live keeper move `accepted` with low-confidence posts under a price nobody can mint
at; the pause is the tool, and with A3-M1 fixed a pause is safe to use early. The monitor should pause on a sustained `no-good-post`
(services, §6).

## 5. Not changed here (listed)

| ID | Why |
| --- | --- |
| L-H (golive-check's Safe check) | `script/golive-check.sh` is the services wave's tool (as in round 3); listed for `r4-services` |
| M-A, M-B, M-E, L-C, L-D, L-F, L-G, I-1..I-3 | Services (`r4-services`) |
| R3-C1..R3-C7 | Canister / timelock (`r4-canister`) |
| The escape's β | Unchanged by design (§1.4): a pause that outlives the week pays the backing, as the stale fallback always did |
| Burns between the unpause and the keeper's first post | Refused (`Stale`), the ordinary outage; bounded by the keeper's first post |

## 6. What the other branches must pick up

- **ABIs** (nothing removed, no struct or existing event changed):
  - `FloorModule`: new views `burnsOpen() → bool`, `pauseFloor() → uint256`, constant `RESUME_GRACE() → uint256` (6 h).
    Changed semantics, same signature: `exitAtBacking()` is now `escaped()` alone (it was also true while paused);
    `burnPrice`/`burnCapacity`/`quoteBurn` while paused price at `min(β, pauseFloor()) × (1 − base)`; `holdsRefunds()` also true for
    `RESUME_GRACE` after an unpause.
  - `ListingRegistry` (and `IKeeperSource`): new views `keeperResumedAt() → uint64`, `keeperResumedAtBlock() → uint64`, set by
    `unpauseKeeper` when it actually resumes. `KeeperPaused` event unchanged.
  - `SweepVault.openDraw`: `totalTickets > 0` with an empty `sentinelProof` → `BadProof`.
- **Monitor / keeper / site (`r4-services`)**:
  - Anything that reads `exitAtBacking()` to decide "burns work" must read `burnsOpen()` (or `burnCapacity()`); the Trust page and
    `docs/v5/launch-gates.md` "Keeper key stolen" row: while paused, holders burn at the frozen floor (at most the backing per unit)
    less the base spread; the backing per unit itself only after a week with no good post.
  - fixes-R3-contracts §1.2's claim that a false pause leaves the holders' backing untouched is corrected here; a false pause now
    costs outsiders nothing to exploit, so the monitor's automatic pause (and a sooner pause on `no-good-post`, A3-I1) is safe
    economically; M-A/M-B/L-F still matter for liveness.
  - The site may show "refunds held until `keeperResumedAt + 6 h`" after an unpause.
  - The keeper never opens a list with tickets and an empty sentinel proof (the snapshot tool never builds one).
- **Canister (`r4-canister`)**: nothing new.

## As built

**As designed**, with these precisions:

- **A3-M1.** `FloorModule._tierPrices()` checks `escaped()` first (β less the base spread), then the keeper being down
  (`min(β_tier, _pauseFloor()) × (1 − base)`), then freshness (unchanged formula), else `Stale`. `_pauseFloor()` is §1.2's `F_p`,
  read from the frozen `accepted` / `target` (no new storage). New views `pauseFloor()` (`F_p` while the keeper is down and not
  escaped, else 0) and `burnsOpen()`; `exitAtBacking()` returns `escaped()`. `burnPrice` and `burnCapacity` gate on `burnsOpen()`.
  The contract header and the `burn` NatSpec say what a burn pays in each state.
- **A3-L1.** `ListingRegistry.keeperResumedAt` / `keeperResumedAtBlock` (declared last), stamped by `unpauseKeeper` only when the
  keeper was paused; both added to `IKeeperSource`. `FloorModule.RESUME_GRACE = 6 hours`; `holdsRefunds()` is true while the
  keeper is down or until `RESUME_GRACE` has passed on both clocks since the stamp. `FrogCurve` is unchanged but for its NatSpec
  (`openRefunds`, `refundsOpenAt`), and so is `IItemQueue` but for its NatSpec.
- **A3-I2.** `SweepVault.openDraw`: `totalTickets > 0` and `sentinelProof.length == 0` → `BadProof`.
- Contract docs: `docs/v5/floor-module.md` §12; a superseded-in-part note at the head of `review2/fixes-R3-contracts.md` §1.2; the
  R3 corrections in `fixes-B-C-keeper-draws.md` §0 and §5 item 16 carry an R4 line.

**Tests.** The three audit3 PoCs are regressions in `test/audit3/` (copied from `review3/poc/`, then turned):

| File | Tests | What it shows |
| --- | --- | --- |
| `Audit3Floor.t.sol` (`Audit3Floor`, reserve 0, and `Audit3FloorReserve`, reserve 25 %) | 7 × 2 | The reviewer's PoC now **loses** (paid 2.0469 → got less; the cushion stays in the pots), and the stale fallback still pays β after the week; the fill band's gap (a held-back fall with fills at the band's edge: a mint below `accepted`, which the reviewer's `acceptedAtPause` would have paid a profit on) is closed; a 1-wei post before a pause cannot zero it; the pause floor never rises over 6.4 days of pause; a short pause and unpause is no lever, and after a long one nothing is priced until the first post; A3-I1 unchanged, the pause ends it at the pause floor; **fuzz** `testFuzz_audit3_aPauseNeverPaysAMintBurnRoundTrip` (a random history of calm, rising, falling, 1-wei, low-confidence and held-back posts and conversions, a mint, a delay with no post, a pause of up to 6 days, a burn of everything: never a profit, never above the pause floor) |
| `Audit3Curve.t.sol` | 5 | A3-L1: the unpause back-run and the first-post back-run both refused (`TooEarly`); the keeper posts, anyone fills, the curve graduates; the grace ends after 6 h on both clocks (the sequencer's clock alone does not end it) and an unfilled curve refunds; the escape hatch still bounds it; no pause, no stamp, no grace; N-3's close gate unchanged around the grace |
| `Audit3Draws.t.sol` | 2 | A3-I2: the sentinel-only root with tickets is refused and the carry untouched; the honest list of the same snapshot opens and pins. The N-2 re-run unchanged |

`BurnLiveness.invariant.t.sol` gains a `roundTrip` handler action (mint at whatever the keeper left, a delay with no post, pause,
wait up to 3 days, burn those units, unpause) and `invariant_aPauseNeverPaysARoundTrip` (no profit, no paused burn above the pause
floor, no paused burn refused); its liveness gate reads `burnsOpen()`; a hand-driven test proves the action is reached.

**Mutation check.** With `_pauseFloor()` forced to `type(uint256).max` (round 3's "pay β" behaviour), the invariant finds a
profitable round trip (`a mint, a pause, a burn: a profit`) and four of the seven `Audit3Floor` cases fail in each variant; restored,
all pass.

**Existing tests changed, and why:**

| File | Change | Why |
| --- | --- | --- |
| `test/audit2/R2Floor.t.sol` | N-1b's paused burns at +0, +61 min, +3 d expect `min(β, pauseFloor()) × (1 − base)`, and β only at +9 d (escaped); `exitAtBacking()` is false while paused, `burnsOpen()` true; the hostile-post case is renamed `…PaysThePauseFloor` and checks the floor is at least the hour's legal move | A3-M1, deliberately |
| `test/audit/B/BurnLiveness.invariant.t.sol` | as above | A3-M1 |
| `test/nft/NftSurface.t.sol` | the registry's exact surface gains `keeperResumedAt()`, `keeperResumedAtBlock()` | A3-L1 |

**Runs** (final tree, this machine):

- `forge test`: **1053 passed, 0 failed** (120 suites; the fork suites return early without `--fork-url`). Before this branch:
  1031 (116 suites).
- `forge test --match-path 'test/audit3/*'`: 21 passed.
- `BurnLiveness` at 500 runs × depth 150, and the A3-M1 fuzz at 5,000 runs: pass.
- `forge test --match-path "test/fork/*" --fork-url robinhood` (public read-only RPC): **19 passed, 0 failed**.
- `script/deploy.test.sh`: **37 ok, exit 0**.
- `STACK_PORT=8697 scripts/golive-dryrun.sh stack` (a local anvil fork of 4663 from the public RPC at block 73005858, under the
  staging chain id; nothing sent to any public chain), with real drand: **DRY RUN: ALL PASS**, exit 0 — golive-check 146 checks ok;
  launch → graduation through the module → module mint → sweep → draw announced → opened with the seed round's real evmnet
  signature (round 20965688) → pinned (round 20965995) → resolved with its real signature → one whole wX claimed. The dry run
  exercises the ordinary path; the pause paths are covered by the forge suites above.

**Still open.**
- A long pause freezes the floor while the market may move: a holder of wrapped items could sell them into the module at the frozen
  floor (a conversion at that floor, within `burnPerHour`; no holder's backing per unit falls). Round 3 paid more (β); round 2 paid
  nothing (refused). The Safe ends a pause, or rotates the keeper, well inside the week.
- The custodian's `itemSold` (floats, OFF in Phase 1) buys a unit at `minSaleProceeds()` = `accepted × (1 − convBps)`, below the
  pause floor; this is the custodian's existing trust (it can already mint against nothing), not new.
- A3-I1 (§4), L-H (services), and the services' and canister's items (§5, §6).
