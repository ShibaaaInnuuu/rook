# Fixes for area A (the curve and the graduation path), 2026-09-26

Branch `fix-grad`, from `fixes-2026-09-26`. Written before the code. It answers `A-curve.md` (A-01 to A-10) and C-10 in
`C-treasury-roles-deploy.md` (the same issue as A-05). Nothing here is deployed.

Summary:

| ID | What changes | Where |
| --- | --- | --- |
| A-01 | 1. The lock's widest layer and each coin pool's position become bounded ranges of ±180,000 ticks around the reference, not `[minUsableTick, maxUsableTick]`. 2. The coin seed and the shared add fail soft: a revert defers them, and anyone retries (`seedCoin`, `deploy`). 3. A curve that has not graduated `window + 4 days` after its close can open refunds even when the queue filled it, and a SWEEP_ONLY curve can too | `LiquidityLock.sol`, `FrogCurve.sol` |
| A-02 | The closing buy's gross is clamped to `msg.value` | `FrogCurve.buy` |
| A-03 | Option (c): the factory refuses a non-SWEEP_ONLY launch, and the curve refuses the buy that would close it, while the queue has no room for the item budget. `queue.hasRoomFor(eth)`, `factory.launchesOpen()`, `curve.canGraduate()` are the site's reads | `IItemQueue`, `FloorModule`, `ItemQueue`, `CurveFactory`, `FrogCurve` |
| A-04 | In a stack with a floor module, every shared add is priced against the module's fresh floor (`price()`), within ±200 ticks, instead of against the keeper's sweep cap within ±1,000 | `LiquidityLock.sol` |
| A-05 / C-10 | `SweepVault.sweepViaDesk` is keeper-only | `SweepVault.sol` (one modifier) |
| A-06 | `coinKey` reads the curve's `sweepOnly()` before a coin is seeded | `LiquidityLock.sol` |
| A-07 | `buy(minOut, deadline)` and `sell(amountIn, minEth, deadline)` overloads | `FrogCurve.sol` |
| A-08 | `MintDesk.collect` checks `transfer`'s return | `MintDesk.sol` |
| A-09 | Not changed (noted) | — |
| A-10 | Not changed (reason below) | — |

---

## A-01: tick saturation

### What the attack needs

Uniswap v4 caps the gross liquidity that may reference one initialized tick:

    L_max(s) = (2^128 − 1) / N(s),   N(s) = (maxUsableTick(s) − minUsableTick(s)) / s + 1

This gives:

- `L_max(60) = 1.1506e34` (N = 29,575);
- `L_max(200) = 3.8350e34` (N = 8,873).

To fill the cap at a tick `T` that bounds one of our positions, the cheapest position is one spacing wide, on the far side of `T`
from the price. It is then one-sided. At distance `d = |T − t_price|` ticks, with `√P` the pool's sqrt price in raw units, its
value in currency1 is:

    cost(d) ≈ L_max(s) · (s · 1e-4 / 2) · √P · 1.0001^(−d/2)

This is the same on either side of the price. Below the price the position is all currency1 and costs `L·Δ√P`. Above the price
it is all currency0 and costs `L·Δ(1/√P)`, which valued at `P` is the same expression.

The cost depends only on the distance from the price, not on where the tick is. The old full range put its bounds at
`d ≈ 883,000` (shared pool) and `d ≈ 712,000` (coin pool), where `1.0001^(−d/2)` is about 1e-19 to 1e-16. That is the whole
attack. The formula reproduces the audit's measurements:

| Position | `d` (ticks) | Formula | Measured |
| --- | ---: | --- | --- |
| shared, spacing 60, √P ≈ 1.24 | 883,000 | 2.9e12 wei | 1.9e12 wei |
| coin pool, spacing 200, √P ≈ 1.2e-4 | 712,000 | 1.6e13 wei | 2.1e13 wei |

### The range

The fix bounds the widest layer at `FAR_HALF = 180,000` ticks on each side of a reference tick. For the shared pool the
reference is the **first** center: the far layer is fixed there for good (`farLower`/`farUpper`), like the full range it
replaces — one position shared by every graduation, so A-09's position count does not grow. For a coin pool the reference is the
seed price. 180,000 is a multiple of both spacings (60 and 200). The bounds are then clamped into the usable ticks. Clamping only
moves a bound closer to the price, which makes it dearer.

| Pool | Saturation cost at `d = 180,000` | For comparison |
| --- | --- | --- |
| shared ETH/wX, spacing 60, √P ≈ 1.24 | `1.15e34 · 0.003 · 1.24 · 1.0001^(−90,000)` = **5.3e27 wei of wX ≈ 3.4e9 ETH** | one graduation adds ~1 ETH + ~1.5 wX |
| coin/wX, spacing 200, ~3–9 wX against 2e26 coin units (√P ≈ 1.2e-4) | **≈ 5.8e24 wX-wei ≈ 3.7e6 ETH** | the pool holds ~3–9 wX |
| coin/ETH (SWEEP_ONLY), 4.75 ETH against 2e26 coin units | **≈ 7.3e6 ETH** | the pool holds 4.75 ETH |

In the task's terms, "a meaningful fraction of the pool's real liquidity": the cost is 10^6 to 10^9 times the pool's real
liquidity, and more than the ETH supply for the shared pool.

**Our own liquidity cannot saturate a tick.** One ETH of full-width liquidity is `L ≈ 1e18`. Reaching `L_max` would take about
1e16 ETH.

**Later price moves do not make it cheap.**

- A coin pool gets exactly one add, at seeding.
- The shared far layer stays where the first center put it, so a later move of the wX price moves the pool toward one bound
  (dearer) and away from the other (cheaper). At a distance of `180,000 + m` the cost falls by `1.0001^(m/2)`. Even after a
  ×/÷ 4.8e8 move (`m = 200,000`), the far bound still costs about 1.5e5 ETH to saturate, and a saturated bound only defers adds.

**What the range gives up.** The layer covers prices within `1.0001^±180,000 = ×/÷ 6.6e7` of its reference. A memecoin that rises
or falls 66-million-fold from its seed price leaves the range. For such a coin that means only this: past that point the pool
holds one asset. Liquidity per unit of capital is identical to full range to within 1.2e-4, since
`amount = L·√P·(1 − 1.0001^(−90,000))`. So every amount-based test and invariant is unchanged.

**Alternatives rejected.**

- **±100,000**, the audit's example. It is safe (about 1.9e11 ETH for the shared pool) but gives up more range (×/÷ 22,000)
  for no need.
- **±300,000.** This drops the coin-pool cost to about 1e3 ETH, which is still large but within reach of a motivated
  attacker who wants to freeze a big pool.
- 180,000 sits where every pool's cost stays above 1e6 ETH.

### Fail soft

The range fix makes saturation uneconomic. Graduation should still not depend on a Uniswap add succeeding. The lock's
liquidity work is therefore run in self-calls, `this.seedCoinPoolSelf(...)` and `this.deploySelf(coin)`. These are external,
callable only by the lock itself, and wrapped in `try`. A revert rolls back everything the attempt did, the PoolManager's side
included, since the unlock is inside the reverted frame. It is then recorded as deferred:

- **Coin seed.** The coin's reserved tokens and its pool quote (wX or ETH) stay in the lock as `pendingCoinToken[coin]` and
  `pendingCoinQuote[coin]`, and `CoinSeedDeferred(coin)` is emitted. `seedCoin(coin)` (anyone) retries the seed and reverts
  with the reason if it still cannot. On success the quote left over goes where it goes today: wX to the coin's shared
  contribution, ETH back to the curve as pool fees.
- **Shared add.** The amounts stay in `pendingEth/pendingWrapped`, as they already do when the price guard defers, and
  `SharedDeferred(coin, 4)` is emitted. `deploy(coin)` (anyone) retries without the wrapper, so a failed retry reverts and
  says why. The same try-wrapper serves `addProtocolLiquidity`, which covers `fundLiquidity` → `flushProtocolFees` and
  `claimShared(PROTOCOL)`. The audit found by inspection that these revert too once `pendingWrapped[PROTOCOL] ≠ 0`.

**Out-of-gas griefing.** A caller could give `graduate()` just enough gas that the inner attempt runs out, is caught, and the
seed is deferred. To prevent this, after a caught failure the lock requires `gasleft() ≥ gasBefore / 8`. A callee that ran out
of gas leaves the caller about 1/64 of what it had (EIP-150). One whose nested call ran out and bubbled the revert up leaves
about 1/64 per level, and there are a few levels here (lock → PoolManager → callback). Either way that is under 1/8, so such a
failure reverts the whole transaction instead of deferring. (A first version used 1/63, and the gas-sweep test found a nested
case that slipped through.) A real revert (a full tick, a bad state) happens early and leaves far more. A gas estimate, a
binary search for the smallest limit that succeeds, therefore never lands on a deferral.

**What fail soft leaves.** A graduated coin whose seed is deferred has no pool until someone retries. Holders then hold a coin
that trades nowhere, but nothing is lost, and the retry is permissionless. The deferral is visible from `pendingCoinToken(coin) != 0`.

### The escape hatch

This is the backstop for any other reason `graduate()` might not complete (a lock bug, a queue that will not release, and so on).

- `openRefunds()` keeps its normal path: the window has passed, the queue has not filled, and `cancel` returns the ETH (and any
  units, for ItemQueue).
- New: once `gradAt + window + STUCK_GRACE` has passed on both clocks (`STUCK_GRACE = 4 days`, so 7 days for the 3-day
  template), the curve may open refunds even if the queue **filled** it, or if it is SWEEP_ONLY:
  - **Filled.** The curve calls `queue.release(coin)`, which it may do, since it is the coin's curve. The module sends it the
    units minted for the budget (`gradUnits −= units`), plus `dust` ETH for ItemQueue. Holders refund pro rata:
    `reserveEth` (= `poolEth + platformFee`, plus dust) in ETH, and the units in wX. **No double pay.** The item budget stays in
    the module as the backing of exactly those units. The units are ETH-backed, so they are burnable at the module or convertible
    by the canister. They are not returned as ETH as well. This is the exact counterpart of the old refund's
    `refundWrappedPool`, "the units already bought".
  - **SWEEP_ONLY.** The refund pool is `reserveEth`, which is the whole raise. There is no queue.
- Before that time, a filled or SWEEP_ONLY curve still answers `WrongState`, so existing callers see no change.
- `canGraduate()` and `refundsOpenAt()` expose the state to the site.

**Why not "unfill" instead (return the budget as ETH).** `_tryFill` already split the ETH into `reserve` and `backing`, and the
canister may have converted some of those units into items since. Taking ETH back out is not exact: the per-unit backing of every
other holder would move. Releasing the units is exact.

**Race.** Once the hatch is open, `graduate()` and `openRefunds()` are both callable, and the first caller wins. A healthy
curve is graduated within minutes of its close by the runner, the closing buyer or anyone. Seven days of inaction means it is
stuck. After fail soft, "stuck" can only mean something unforeseen.

**Alternatives rejected.** Letting the module `cancel` a Filled grad: same exactness problem as unfilling, and it would change
FloorModule's graduation state machine, which another branch is editing.

## A-02: the closing buy over-books

After re-deriving `gross = ceil(room · BPS / (BPS − rate))`, clamp it: `if (gross > msg.value) gross = msg.value`, then
`fee = gross − net`. This is safe because `net = room ≤ msg.value − ⌊msg.value · rate / BPS⌋ ≤ msg.value`, so the fee stays
≥ 0. The fee may be a wei or two below the nominal rate on that one buy, which favours the buyer by dust. This turns the audit's
fuzz property ("closing buy is solvent") into a passing regression test.

## A-03: the stage-0 capacity

The problem: `maxEthBacked` (40 units) fills after about 5–6 graduations at stage 0, because custody can convert at most 3
units. Every later curve then closes into a Queued grad and refunds after 3 days. The refund is at the average price, and the
fees are not returned.

The options:

- **(a) Queue instead of minting past custody's convertible room at graduation.** This is what already happens at the
  `maxEthBacked` line. Moving the line down to custody's room (3 units) only makes curves refund sooner. **Rejected.**
- **(b) Size the cap to the expected graduations.** `maxEthBacked` is a per-deployment immutable (`StackChecks.defaultParams`).
  Raising it grows the ETH-backed exposure that §0 of `phase1-scope.md` states and the operator accepted. It is a policy knob,
  not a fix: any finite cap fills at stage 0. **Not changed here.** The operator can still raise it per stage at deploy.
- **(c) Stop selling into curves that cannot graduate. Chosen**, because it is the least surprising option for users: nobody
  buys into a curve bound to refund. It has two gates:
  1. **Launch gate.** `CurveFactory` refuses a non-SWEEP_ONLY launch (`QueueFull`) unless `queue.hasRoomFor(itemBudget)` holds
     for the budget that launch would get. `launchesOpen()` answers the same question for the site. SWEEP_ONLY launches are
     unaffected, since they have no item budget.
  2. **Close gate.** `FrogCurve.buy` refuses the buy that would close the curve (`QueueFull`) while
     `queue.hasRoomFor(itemBudget)` is false. The curve stays in **Trading**: holders can still sell back to the curve at its
     own price (exact, no loss) instead of waiting three days for an average-price refund, and the close goes through as soon as
     room appears (burns at the module, conversions, a stage increase).

     This also handles concurrency, where several curves are open and one fills the room, and the A-03/B-07 front-run: a public
     mint in front of a closing buy now makes that buy revert instead of forcing the curve into refunds.

`FloorModule.hasRoomFor(eth)` is a view with the same arithmetic as `_tryFill`, counting the budgets already queued:

    units = (gradEth + eth) · 1e18 / gradPrice();   room ⇔ gradPrice() ≠ 0 ∧ ethBacked + units ≤ maxEthBacked

It deliberately ignores freshness. A stale floor is transient and handled by the existing Queued → `fill` path. `ItemQueue`
(not deployed in Phase 1) answers `true`: it has no cap, and it buys over time.

**What the operator should know.**

- At stage 0 the frog collection's non-SWEEP_ONLY launches **close after about 5–6 graduations**. They reopen when ETH-backed
  units are burned or converted.
- This is the honest steady state of stage-0 custody. The site should show it: "launches paused: the floor module is at
  capacity (stage 0)" from `factory.launchesOpen()`, and per curve "the last buy waits for room" from
  `module.hasRoomFor(curve.itemBudget())`.
- Raising `maxEthBacked` at deploy (b) is the lever if the operator wants more.

**Residual.** A curve can still close while the floor is stale and then find the room taken by the time the floor is fresh. It
then waits and refunds as before. This needs a stale floor at the exact close, followed by other closes. It is narrow and
unchanged in kind.

## A-04: the graduate() sandwich

**The cause.** A shared add is priced at the pool's spot, accepted within `maxDeviation = 1,000` ticks of the keeper's **sweep
cap**. In production the keeper posts `cap = floor × (1 + 10%)` (`p1-services.md` §1, `KEEPER_CAP_MARGIN_BPS = 1000`), so a
pool at fair value already sits about 953 ticks from the cap. The guard's window therefore lets an attacker push the price across
almost 2,000 ticks (about 21%) before an add, not the 10% the audit measured with `cap == floor`. The profit per sandwich grows
roughly as δ² times the added liquidity, less fees that grow as δ.

The options:

- **Reduce `maxDeviation` against the cap.** Impossible: the cap is 10% off fair by design, so a window under ~960 ticks would
  defer every add in production.
- **Restrict who may call `graduate()` early.** This leaves `deploy(coin)` and the protocol adds (`fundLiquidity`,
  `claimShared(PROTOCOL)`) sandwichable, and it makes liveness depend on an operator service that does not call `graduate`
  today. **Rejected.**
- **A reference close to the pool's fair price. Chosen.** In a stack whose factory's queue is a FloorModule, which is every
  Phase-1 stack, the lock prices every shared add, the alignment of an empty pool, the first center and `moveCenter` against the
  module's `price()` while `fresh()`. The router arbitrages the pool against the module's mint and burn prices, so a fair pool
  sits within the spread plus the pool fee of `price()`. The window is then `REF_DEVIATION = 200` ticks (about 2%).
  - The module is found once, at `wire`: `factory.queue()` is probed for `gradPrice()` inside `try`. There is no deploy-script
    change.
  - Stacks without a module (ItemQueue) keep the cap and `maxDeviation`, unchanged.
  - A stale module defers adds (reason 1) just as a stale cap does.

**Effect.** The largest push that still gets an add falls from about 1,950 ticks to about 400 (±200 around the reference, the
pool starting anywhere inside). Since the profit is proportional to δ² and the fees to δ, a push of 2% or less against a
0.3% pool earns about `½δ² − 0.003δ` of the in-range add's value. That is at most about 1.4e-4 of it (1.4e-4 ETH for a 1-ETH
near-band add), against about 1e-2 before.

Measured in the regression test, after one graduation, as in the audit:

| Push | Before the fix | After the fix |
| --- | --- | --- |
| ±990 ticks (the audit's) | +0.0107 / +0.0130 ETH | the add is deferred whole; P&L −0.0035 / −0.0037 ETH (fees only) |
| ±190 ticks | — | −0.000056 / +0.000023 ETH |

**What changes for honest adds.** They wait (and anyone retries) while the pool is more than 2% from the module's price, for
example in a volatile hour with a wide spread. Nothing is lost by waiting, as the design already says. The shared layers center
on the floor rather than on floor + 10%, which is where the design meant the depth to be (`pools.md` §3: "deep at the floor").

## A-05 / C-10: MintDesk OFF is policy

`SweepVault.sweepViaDesk` gets `onlyKeeper`. The Phase-1 keeper service has a closed call list without it (`p1-services.md` §1),
so the desk route cannot be reached on chain. `collectDesk` stays open to anyone, since it only brings escrow home.

Rejected: deploying the vaults with `desk = 0`. It is also correct, but it changes `DeployStack`, the stack file, `golive-check`'s
read-backs and the rehearsal, which another branch owns. It would also make a later desk impossible without new vaults, which
are immutable.

## Informational

- **A-06.** `coinKey` for an unseeded coin reads `IPoolFeeSink(curve).sweepOnly()`, so a SWEEP_ONLY coin names its COIN/ETH pool
  from launch.
- **A-07.** `buy(uint256 minOut, uint256 deadline)` and `sell(uint256, uint256 minEth, uint256 deadline)` revert `Expired` past
  `deadline`, by `block.timestamp`. That is a user-side guard, so one clock is enough. The old signatures stay.
- **A-08.** `MintDesk.collect` reverts `TransferFailed` if `transfer` returns false.
- **A-09.** Not changed. Protocol positions grow by at most three a day, while the center moves. `claimShared(PROTOCOL)` stays
  within gas for decades (about 20k gas per position poke).
- **A-10.** Not changed. `gradPrice = max(price(), target) × (1 + convBps)` is deliberate (`floor-module.md` §6.4). After a real
  upward move, `accepted` lags by up to an hour at 5%/h. Pricing at `accepted` would then mint units the canister cannot convert
  at their backing, and the shortfall would come out of `revenue`. The keeper's power here is to shrink a graduation's units by
  posting a high target. The ETH is not lost: it stays as backing and benefits burners. This is keeper trust, already stated in
  the trust model. Flagged for the area-B owners if they want `min(target, accepted × (1 + maxMoveBps))`.

## Tests

The audit's PoCs become `test/audit/A/` regression tests that assert the attack now fails, with their controls kept:

- `TickSaturation`: the audit's saturation no longer blocks graduation. Saturating the new bounds is measured at 3.43e9 ETH
  (the shared far layer) and 7.3e6 ETH (a SWEEP_ONLY coin pool). Even when paid, it only defers: the shared add
  (`SharedDeferred(coin, 4)`, then `deploy`) and the coin seed (`pendingCoin*`, then `seedCoin`) go through once the position
  is removed. A sweep of gas limits shows that no limit defers anything: `graduate()` either reverts whole or completes. The
  escape hatch covers filled curves (a lock mocked to revert) and SWEEP_ONLY curves, with exact books.
- `LastBuyAccounting`: the fuzz property passes, and the curve stays solvent in the band.
- `GraduationSandwich`: the ±990 push defers. The ±190 push profit is bounded.
- `DeskOff`: a stranger's `sweepViaDesk` reverts `NotKeeper`.
- Capacity: after the room fills, a launch reverts `QueueFull`, a closing buy reverts `QueueFull`, holders can still sell, and
  the close goes through once room returns.
- Fuzz: "`graduate()` never reverts because of the liquidity adds", over random pre-saturation. "Curve ETH out ≤ ETH in" over
  buy/sell/close/refund sequences.

## Results (after the code)

- `forge test`: **969 passed, 0 failed** (102 suites). That is the 940 existing tests plus 28 new ones in `test/audit/A/` and 1
  in `test/MintDesk.t.sol`.
- `forge test --match-path "test/fork/*" --fork-url` a local anvil fork of the public read-only RPC: **19 passed**. This includes
  `Pools.fork`'s graduation into the real 4663 PoolManager.
- `script/deploy.test.sh`: 37 ok, no FAIL.
- `DRYRUN_NO_DRAND=1 scripts/golive-dryrun.sh stack`: **ALL PASS**, forked from the public archive RPC `rpc.ordofi.network`.
  This rehearsal posts cap = floor × 1.1, the production shape. Launch, graduation through the module, a module mint, a
  router sweep and a draw opened all pass. The drand resolve was skipped by the flag.
- `test/audit/A/GraduationSandwich.t.sol::ProductionCapMarginTest` checks the production shape directly. With the cap 10% above
  the floor, the layers center on the floor's tick, not the cap's, and the add is made.

Existing tests changed, and why:

- `MintDesk.t.sol`: the four vault-route tests call `sweepViaDesk` as the keeper (A-05). A new test shows a stranger is refused.
- `FloorGraduation.t.sol`: `test_theHardCapMakesALaterGraduationWait_untilConversionsMakeRoom` became
  `test_theHardCapHoldsALaterCloseInTrading_untilConversionsMakeRoom` (A-03). The second curve no longer closes into a Queued
  graduation. Its closing buy is refused while there is no room, a buy short of the close still goes through, and after a
  conversion it closes and fills at once.
- `LiquidityLock.t.sol`:
  - The coin pool's position is read at `coinRange(coin)`, ±FAR_HALF, not at the usable-tick extremes (A-01).
  - The surface list gains the new views, the retry and the two self-calls.
  - The "coin left unpaired" tolerance goes from `reserved / 1e12` to `reserved / 1e5`. A bounded range on the 200-tick grid
    puts the seed price up to one spacing off its middle, so the two sides pair to within about 1e-6. The measured leftover is
    2.2e-6 of the reserve (445 coins of 200M), which stays in the lock, burned in effect, as the dust always did.
- `Pools.invariant.t.sol`, `Pools.fork.t.sol`: the coin position is read at `coinRange(coin)`.

## Left open

- **The site** (`web-rook/`, not touched here) should:
  - read `factory.launchesOpen()` to show "launches paused (module at capacity)";
  - read `curve.canClose()` to explain a refused last buy, and `curve.canGraduate()` / `refundsOpenAt()`;
  - use the `buy(minOut, deadline)` / `sell(…, deadline)` overloads (A-07). The old selectors still work.
- **The runner or keeper** could call `lock.seedCoin(coin)` / `lock.deploy(coin)` when it sees `CoinSeedDeferred` or
  `SharedDeferred(coin, 4)`. Anyone can, so this is only a convenience.
- **`phase1-scope.md`** still says `sweepViaDesk` is anyone's (the Sweeps row) and describes the Queued → refund path for a full
  module (§1). It is left to whoever merges the fix branches, to avoid conflicts.
- **`docs/v3/pools.md`** says "20% full range". That layer is now ±180,000 ticks around the first center.
- **Before the keeper's first floor post** the module has no `gradPrice()`, so `hasRoomFor` is false: a non-SWEEP_ONLY launch
  is refused (`QueueFull`) until the keeper has posted once. Before this change such a launch went through and its close would
  have waited.
- **A-09 and A-10**, as above. B-07 (public mints holding the room) is area B's. Here it can only delay a close, not force a
  refund.
