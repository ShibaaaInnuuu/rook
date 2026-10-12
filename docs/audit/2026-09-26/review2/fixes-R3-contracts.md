# Fix round 3 (R3): the contracts' answers to the second review

Branch `r3-contracts` (from `fixes-2026-09-26`, head `2bb3ee7`, the second review). Written before the code, as the design; the
"As built" section at the end is filled in when the code lands. Nothing here is deployed, and no key, wallet or live chain is
touched.

Inputs: `review2/R2-contracts.md` (N-1..N-8 and the B-07/B-10 partials), its PoCs `review2/poc/R2{Floor,Curve,Draws}.t.sol`, and
the canister review's N-2 (`review2/R2-canister.md`), whose robust fix is on the EVM side. Two other branches fix the canister
(`r3-canister`) and the services, monitor and site (`r3-services`); this one stays in `src/`, `script/`, `test/` and the contract
docs. What those branches must pick up from here is listed in §8.

## 0. Summary

| ID | Sev | Fix | Where |
| --- | --- | --- | --- |
| **N-1a** | Medium | Burns price the keeper's `target` no lower than the accepted floor could legally reach within the next hour of posts, and never below the day band: a 1-wei post cannot zero a burn while the module stays fresh | `FloorModule._tierPrices` (`_burnTarget`) |
| **N-1b** | Medium | While the keeper is paused (or `registry.keeper()` is zero) burns do not wait for the stale window: they pay the backing per unit less the base spread at once, the stale fallback's price. Mints, fills, floats, sales, rentals and conversions stay refused, so the pause cannot be used to mint cheaply | `FloorModule` (`exitAtBacking()`) |
| **B-10** | Info | The `burn` NatSpec, the contract header and the fixes-B-C doc's "burns never stop" are made true (above) and reworded to say exactly what a burn pays in each state | NatSpec, docs |
| **N-3** | Low | The close gate refuses while the keeper is paused (`hasRoomFor` is false, the curve keeps trading, as for `QueueFull`). A grad queued *before* a pause waits: an unfilled curve's refunds hold while the queue says conversion is paused, for at most the curve's `STUCK_GRACE` | `FloorModule.hasRoomFor`, `IItemQueue.holdsRefunds`, `FrogCurve.openRefunds` / `refundsOpenAt` |
| **N-2** | Low | **The snapshot seed survives a void or a cancel.** A draw voided or cancelled before its pin leaves its seed to the coin: the coin's next announcement reuses that seed round and that snapshot window, so it gets the same snapshot time. Not opening, and cancelling an Open draw, change nothing about the snapshot any more | `SweepVault.announceDraw`, `snapshotTimeOf`, `voidDraw`, `cancelDraw`, `requestDraw` |
| **Canister N-2** | Medium (canister) | **A resolved ticket is refunded only once the delivery can no longer confirm**: the custodian says so (`markUndeliverable`, after its conflict-out confirmed deep or when nothing was ever broadcast), or `LATE_DELIVERY_WINDOW` (30 days) has passed since `deliverBy`. Until then a late delivery is recorded as usual and no refund exists to race it | `Wrapper.refund`, `markUndeliverable` |
| **B-07** | Low | A public mint may not take ETH-backed room a *queued* graduation needs (the queued budgets count against `maxEthBacked` at `gradPrice()`). A public mint front-running a *closing buy* remains possible and is accepted: bounded by the public cap, no loss, measured below | `FloorModule.mint`, `mintCapacity` |
| **N-6** | Info | `NftRegistry._checkBuyer` compares against `namedKeeper()` (the key, paused or not) | `src/nft/NftRegistry.sol` |
| **N-7** | Info | While the keeper is paused the item cap is zero and snapshots do not count: `itemArrived` refuses (`CapStale`, the error the canister already retries on) and `snapshotCap` refuses | `FloorModule.itemCap`, `_capOrSnapshot` |
| **N-8** | Info | `setGuardian` refuses a guardian that is (or is waiting to be) the owner, keeper, purchaser, attestor, treasury or an auto-buyer; `proposeRoles` refuses the guardian as a role | `ListingRegistry` |
| N-4, N-5 | Low/Info | Not changed; listed in §7 with the reason | — |

## 1. N-1: burns through any keeper incident

### 1.1 (a) The fresh freeze: bound the target the burn side reads

Today a fresh burn pays `min(price(), target) × (1 − s)`, capped by each tier's backing per unit. `target` is the keeper's raw post
and only `accepted` is clamped, so a keeper posting 1 wei with full confidence keeps the module fresh and every burn at 0.

Fix: the burn side reads

    tB = max(target, lo),   lo = max(accepted × (1 − maxMoveBps), min(dayLow, accepted))

where `dayLow = dayAnchor × (1 − MAX_DAY_MOVE_BPS)` (the day band's floor, as `gradPrice` uses its top). `lo` is the lowest the
accepted floor could legally become after one more hour of posts, so the lag is still priced against the trader for as far as the
floor can really move before the next post; beyond that a post says nothing the module would act on. This is tighter than the
review's `min(dayLow, accepted)` (a 1-wei poster could otherwise hold burns 20 % under the floor all day), and it can only move the
burn price up toward the accepted floor, never above `min(price(), β)`.

Mints keep `max(price(), target)` but read the target clamped from above into the day band (`min(target, max(dayHigh, accepted))`,
exactly `gradPrice`'s rule), for symmetry: a far-off high post cannot price every mint out either. Neither change makes a mint
cheaper than `price() × (1 + sMint)`, and a round trip is still never profitable: `min(price(), tB) ≤ price() ≤ max(price(), tM)`.

What a compromised, not yet paused keeper can still do: walk `accepted` down within ±5 %/h and ±20 %/day and post a low target, so
burns pay about `0.95 × accepted × (1 − s)` of a floor that falls 20 % a day. That is the B-02 keeper reach already documented (it
is priced against traders and bounded per day); it is never a freeze, and pausing ends it (b).

### 1.2 (b) The pause: burns at the backing, at once

> **Superseded in part by `review3/fixes-R4-contracts.md` §1 (R3 check A3-M1).** Paying the average backing at once let a unit
> minted on a fallen floor before any pause exit above its cost, at the other holders' expense; the claim below that a false
> positive leaves the holders' backing untouched missed that. A paused burn now pays the lower of the backing per unit and the
> floor the keeper left (`pauseFloor()`) less the base spread; the backing per unit only once `escaped()`.

After `pauseKeeper()` nobody can post, so the module goes stale an hour later and burns reverted `Stale` until
`lastGoodAt + FLOOR_TTL + STALE_ESCAPE` (~7 days) — the whole incident runbook.

Fix: `exitAtBacking()` is true when `escaped()` **or** the keeper is down (`registry.keeperPaused()` or `registry.keeper() == 0`).
Then a burn pays, per tier, `β × (1 − baseSpreadBps)` (the stale fallback's price), whether or not the floor is still fresh.

Why this is safe to do at once:

- **No pot's backing per unit falls.** Each unit still releases exactly its tier's `β` and pays at most that (M4); the base spread
  stays in the pots (reserve tier to `surplus`, convertible tier to revenue).
- **Nobody can mint into it.** Every inflow priced by the keeper's word already refuses while paused (`_priced`: mint, `fundFloat`,
  `itemSold`; `_tryFill`; `rentIn`), and (N-7, §6) `itemArrived` now too. So the pause cannot be used to mint at a stale or
  walked-down floor and burn at `β`. `_priced` also refuses when `registry.keeper()` is zero, for the same reason.
- **Why not wait any shorter window instead:** the escape price is safe by construction, and the pause is a deliberate act by the
  Safe or the guardian. A guardian false positive costs the protocol only revenue it would otherwise have kept on the burns made
  during it (the gap between a floor below `β` and `β`, and between the floor spread and the base spread); the holders' backing
  per unit is untouched, and the Safe unpauses.
- `escaped()` itself is unchanged (it also gates `skim`'s sweep of the surplus and all revenue to the treasury, which should not
  follow a pause that may be a false positive).

The residual, stated: units minted cheaply on a walked-down floor *before* the pause now exit at the tier's average `β` during the
pause instead of after the 7-day window. That is the same cross-subsidy the stale fallback already allowed, a week sooner, and
bounded by the B-02 keeper reach (`maxPublicEthBacked × floor × 20 %` a day).

### 1.3 B-10: the words

- `burn` NatSpec: "Never paused" is replaced by what is true now: fresh → the lower of the (bounded) floor less the spread and the
  backing per unit; keeper paused or stale for a week → the backing per unit less the base spread; stale less than a week with the
  keeper not paused → `Stale` (an ordinary keeper outage, which the next post or the stale window ends).
- The contract header's "burns never stop" gets the same precision.
- `fixes-B-C-keeper-draws.md` §0, §3 and §5 item 16 get a one-line correction pointing here. The runbook row in
  `docs/v5/launch-gates.md` and the Trust page text (`web-rook`) belong to `r3-services` (§8).

## 2. N-3: no close into an unfillable queue during a pause

- `FloorModule.hasRoomFor` returns false while the keeper is down. `canClose()` is then false and the closing buy reverts
  `QueueFull`; the curve keeps trading, and closes normally after the unpause. Launches were already refused (`capFresh()`).
- Freshness stays out of the gate, deliberately (A-03): a stale floor is an hour-scale outage and a queued grad is filled by
  `fill()` at the next post.
- A grad already `Queued` when the pause starts (possible only if the floor was stale at the close) cannot be filled until the
  unpause. New `IItemQueue.holdsRefunds()` (the module: keeper down; `ItemQueue`: false): while it is true, an unfilled curve's
  `openRefunds` waits for `window + STUCK_GRACE` (the escape hatch's time) instead of `window`, so a pause started in the window
  gets up to four more days to be resolved before the curve is bound to refund. It never waits longer: a pause that is never
  lifted cannot hold anyone's refund beyond the hatch. `refundsOpenAt()` reports the held time.

## 3. N-2: the snapshot time cannot be re-rolled

The seed round that picks a draw's snapshot time is pinned at the announcement, but whoever can end a draw before its pin (the
keeper by not opening; the owner or guardian by `cancelDraw`) could discard a snapshot time after seeing it and announce again.

Fix: **a seed, once pinned for a coin, is consumed only by a pin.**

- Each draw has a seed origin: its own id, or the draw whose seed it carries. `snapshotTimeOf(id, r)` hashes the origin's id and
  counts back from the origin's `announcedAt`, so every draw with the same origin gets the same snapshot time from the same round.
- `voidDraw` of an Announced draw and `cancelDraw` of an Announced or Open draw store the draw's origin as the coin's
  `carriedSeed`. The coin's next `announceDraw` reuses it: the same `seedRound` (event `SeedCarried(id, origin)`), the same
  snapshot time. `requestDraw` (the pin) clears it. Voids of pinned or resolved draws do not carry (their outcome is the pinned
  round's, which C-02 already made final).
- So not opening is only a delay, and a cancel after an Open draw's snapshot time is public (the guardian answering a forged root,
  the intended use) makes the keeper open the same snapshot with an honest list. The ticket list at that time is public data; only
  the pinned round, unknown until after `requestDraw`, picks the winner.
- `Draw` is unchanged (the keeper, monitor and snapshot tool decode it). The keeper service already reads the snapshot time with
  `snapshotTimeOf` on chain and the seed round from `draw(id).seedRound`, so a carried draw needs no service change; a carried seed
  round is already public, which only makes opening faster.
- The C-04 property is kept: the carried time was fixed by a round nobody had seen when the *origin* was announced.
- **Follow-up (`fixes-R3-carry.md`):** a carried snapshot time with no eligible holder could never be opened, so the carry was never
  consumed. An empty snapshot is now opened as the sentinel alone (`totalTickets` 0) and `requestDraw` closes it with no winner
  (`Stage.Empty`, `DrawEmpty`), which consumes the carry; a false empty is cancellable and the cancel carries the same time.

## 4. Canister N-2 on the EVM side: a late delivery after a refund

The canister review's N-2: a redeemer pins custody's delivery (a large low-fee child on its own output), so neither the bump nor
the conflict-out can replace it; at `deliverBy + REFUND_GRACE` it refunds, then confirms the pinned delivery and holds both the
frog and the token (LostRace, one unit unbacked).

Options considered:

1. **Escrow the refunded unit, claw it back on a late delivery.** An escrow that ends at a fixed time is only a longer grace (the
   pin must outlast it, same as today); one that claws back from the owner after it ends fails as soon as the owner moves the
   token. The frog would also have to stay out of the random set during the escrow, or a later ticket could draw a frog that is
   about to leave.
2. **Lock the refunded unit in the owner's balance for a period.** Touches every transfer path (`_update`), the sweep vaults'
   `reclaim`, and redemption; still time-bounded.
3. **Refund only once the delivery can no longer confirm (chosen).** The frog stays reserved to the ticket, and no token is
   minted back, until either
   - the custodian calls `markUndeliverable(id)`: it attests that no version of the delivery can confirm (its conflict-out spent
     the delivery's inputs and is deep, or nothing was ever broadcast). The custodian alone signs every spend, so this is no new
     trust: a custodian that lies here could already send the frog anywhere. Then `refund` works at `deliverBy + REFUND_GRACE` as
     today, so an honest refund is no slower; or
   - `LATE_DELIVERY_WINDOW` (30 days, both clocks) has passed since `deliverBy`, for a custodian that is gone.

   Until then, `recordDelivery` is accepted as it is today, so a late delivery is simply recorded and there is never a refund for
   it to race. The pin attack then needs the delivery to stay unmined for 30 days, i.e. the fee market above the delivery's rate
   for a month (the pin blocks replacement, not mining), and the canister to never see its conflict confirm. Sustained spikes have
   lasted days, not a month.

   Pinned-ticket refunds (`PIN_TIMEOUT`, no frog drawn, nothing can be delivered) are unchanged.

   What remains: a delivery that confirms after a refund that was attested or after 30 days is still LostRace on chain (loud in the
   canister, `EarlierConfirmed` after `Refunded`). The canister work in `r3-canister` (TRUC deliveries, re-priced conflicts) is
   what makes the attestation arrive early.

Races, both ways (tests):

- delivery recorded first → `refund` refuses (`WrongStage`), however late;
- pin attack: `refund` at `deliverBy + REFUND_GRACE` refuses (`NotUndeliverable`); the delivery confirms on day 10 and is recorded;
  the unit stays backed; the refund never happens;
- honest non-delivery: the custodian marks it undeliverable → `refund` at `deliverBy + REFUND_GRACE`; a delivery recorded after the
  refund refuses (`WrongStage`);
- dead custodian: `refund` at `deliverBy + LATE_DELIVERY_WINDOW` without any attestation;
- only the custodian marks; only a Resolved ticket.

## 5. B-07: public mints and graduation room

- **Fixed:** a public mint counts the budgets already queued (`gradEth` at `gradPrice()`) against `maxEthBacked`, so it cannot take
  the room a `Queued` graduation (stale floor at the close, or behind the cap) is waiting for, which then refunded. `mintCapacity`
  reports the same room.
- **Accepted:** a public mint front-running a *closing buy* (the curve then reverts `QueueFull` and keeps trading). The measure: a
  public mint's footprint on `ethBacked` is at most `publicCap / (1 − reserveBps)` (13.3 of `maxEthBacked = 40` at the Phase-1
  params, ~8.4 ETH of capital, recoverable through burns at the spread). The closing buy waits until conversions (the canister,
  `maxItemsPerDay`) or burns free the room; nothing is lost and nobody is bound to a refund. A separate "graduation headroom" would
  need a per-curve reservation at launch (the factory does not know when a curve will close), which is not clean; the delay is
  stated on the Trust page's graduation row (`r3-services`).

## 6. Info fixes

- **N-6** `NftRegistry._checkBuyer`: `namedKeeper()` instead of `keeper()` (zero while paused). ListingRegistry's `reserveBuyer`
  already refused the named keeper; now both checks agree. NFT is OFF.
- **N-7** While the keeper is down: `itemCap()` is 0, `_capOrSnapshot` ignores a snapshot, so `itemArrived` refuses `CapStale`
  and `snapshotCap` refuses `CapStale`; `maxItemCost()` is 0, so the canister does not buy. An item bought before the pause waits in
  the module as the canister's until the unpause (then `itemArrived` at the fresh cap) or is taken back with `reclaimStray`.
- **N-8** `setGuardian(g)`: `g` may be zero (none) or any address that is not the owner, the pending owner, the named or pending
  keeper, purchaser or attestor, the treasury or pending treasury, or an auto-buyer (`SameKey`). `proposeRoles` refuses the
  guardian as keeper, purchaser or attestor.

## 7. Not changed (listed)

| ID | Why |
| --- | --- |
| N-4 shared-add deferrals | Liveness only (no loss; `deploy()` is anyone's). The runner and site retry and show pending amounts (`r3-services`) |
| N-5 escape hatch chooses refund | The hatch exists because `graduate()` may revert for good (A-01); telling "healthy" from "stuck" on chain needs trying `graduate()` inside `openRefunds`, which the reentrancy guard and fail-soft paths make fragile. A healthy curve is graduated by the runner within minutes of its fill; the hatch opens 7 days after the close. Noted on the Trust page |
| B-06, B-08, B-11, B-12 (rest), C-06, C-13, C-14, C-15, A-09 | As in the earlier fix docs (deferred or accepted). B-12's pause half is closed by N-7 |
| C-03 partial | The list is still the keeper's; the monitor recompute and the guardian answer it. N-2's carry makes the guardian's cancel steer nothing |
| C-11 partial | One keeper key for floor, caps and roots (a fourth registry role is an open decision) |

## 8. What the other branches must pick up

- **Canister (`r3-canister`)**: call `Wrapper.markUndeliverable(id)` (custodian) once a delivery's conflict-out is deep (≥ the
  canister's finality depth) or when no version of it was ever broadcast; otherwise resolved refunds wait 30 days. `itemArrived`
  and `snapshotCap` refuse `CapStale` while the keeper is paused (already a retry case).
- **Keeper, monitor, site (`r3-services`)**: `announceDraw` may now reuse a carried seed round (`SeedCarried`); read the snapshot
  time from `snapshotTimeOf` (already done) and never compute it off chain from `announcedAt`. Monitor: page on `DrawVoided` of an
  Announced draw (the keeper declined to open) and on `SeedCarried`. Site: the Refund button for a Resolved ticket shows
  `deliverBy + 72 h` if `undeliverable(id)`, else `deliverBy + 30 d`; burn quotes work while paused (`burnCapacity`,
  `exitAtBacking()`); the Trust page's "burns never stop" is now exact (at the backing per unit while paused).
  `docs/v5/launch-gates.md` "Keeper key stolen" row: holders burn at the backing per unit from the pause, not from day 7.
- **ABIs** (every new or changed entry; nothing removed, no struct or existing event changed):
  - `FloorModule`: new view `exitAtBacking() → bool`, new view `holdsRefunds() → bool` (IItemQueue). Changed semantics, same
    signature: `burnPrice`, `burnCapacity`, `quoteBurn` (non-zero while paused; fresh prices use the bounded target), `mintPrice`,
    `gradPrice` (target capped at the day band's top), `mintCapacity` (also bounded by the queued graduations' room), `hasRoomFor`
    (false while paused), `itemCap`/`maxItemCost` (0 while paused), `snapshotCap` (reverts `CapStale` while paused).
  - `ItemQueue`: new view `holdsRefunds() → bool` (always false).
  - `FrogCurve`: `refundsOpenAt()` and `openRefunds()` hold an unfilled curve while `queue.holdsRefunds()`; `canClose()` false
    while paused.
  - `SweepVault`: new views `carriedSeed(address coin) → uint256` (origin id + 1, 0 none), `seedOrigin(uint256 id) → uint256`;
    new event `SeedCarried(uint256 indexed drawId, uint256 indexed originId, uint64 seedRound)` (emitted in `announceDraw` before
    `DrawAnnounced`). `snapshotTimeOf` hashes the origin id. `requestDraw` refuses (`KeeperPaused`) while paused **or** while
    `registry.keeper()` is zero — kept deliberately for the monitor's pre-pin pause (test
    `R2Draws.test_requestDrawRefusesWhileTheKeeperIsPaused`).
  - `Wrapper`: new `markUndeliverable(uint256 id)` (custodian), view `undeliverable(uint256) → bool`, constant
    `LATE_DELIVERY_WINDOW() → uint256` (30 days), event `Undeliverable(uint256 indexed ticketId)`, error `NotUndeliverable()`
    (from `refund`).
  - `ListingRegistry`: `setGuardian`, `proposeRoles`, `transferOwnership`, `proposeTreasury`, `reserveBuyer`/auto listings may
    now revert `SameKey` for the guardian. `NftRegistry`: unchanged ABI.
- **Stack file** (`script/DeployStack.s.sol`, `deployments/stack-<chain>.json`): new top-level `payee`; per collection
  `itemUnits` (the wrapper's `ONE_ITEM()`) and `listingPaused` (`registry.isPaused(listingId)` at deploy). `golive-check.sh` still
  assumes payee = custodian and a unit = 1e18; switching it to read these keys is the services wave's.

## As built

**As designed**, with these precisions:

- **N-1.** `FloorModule._burnTarget()` (the burn side) and `_upTarget()` (mints and `gradPrice`, A-10's rule moved into one
  helper). `exitAtBacking()` is new and public; `burn`, `burnPrice`, `burnCapacity` and `quoteBurn` use it. `_keeperDown()`
  (`keeperPaused() || keeper() == 0`) replaces the bare `keeperPaused()` reads in `_priced`, `_tryFill`, `rentIn`,
  `mintCapacity` and `rentalRoom`. `escaped()` and `skim` are unchanged.
- **N-3.** `IItemQueue.holdsRefunds()` (module: keeper down; `ItemQueue`: `false`). `FrogCurve.openRefunds` holds an unfilled
  curve to `window + STUCK_GRACE` while it is true; `refundsOpenAt()` says so.
- **N-2.** `SweepVault.carriedSeed(coin)` (origin id + 1) and `seedOrigin(id)`; event `SeedCarried(drawId, originId, seedRound)`.
  `Draw` is unchanged. The origin of a carrier is the first draw, not the carrier (`seedOrigin` never chains).
- **Canister N-2.** `Wrapper.markUndeliverable(id)` (custodian, Resolved only), `undeliverable(id)`,
  `LATE_DELIVERY_WINDOW = 30 days`, event `Undeliverable`, error `NotUndeliverable`. `refund` of a Resolved ticket checks
  `REFUND_GRACE` first (`TooEarly`, unchanged), then the attestation or the late window (`NotUndeliverable`). The OFF wrappers
  (`NftWrapper`, `HomeWrapper`) are not changed; port this before either goes ON.
- **B-07.** `_gradRoom()` (units still addable under `maxEthBacked` after `ethBacked` and the queued budgets at `gradPrice()`)
  bounds `mint`, `mintCapacity` and, for the same reason, `fundFloat` (treasury floats, OFF in Phase 1), which replaces its bare
  `maxEthBacked` check.
- **N-6, N-7, N-8** as §6. `ListingRegistry._requireNoRole` now also refuses the guardian, so an auto-buyer, a pending owner and
  a proposed treasury can never be it either; re-setting the same guardian is allowed.
- **Requested by the services wave:** `SweepVault.requestDraw` keeps refusing while the keeper is paused, and now also while
  `registry.keeper()` is zero (the monitor pauses the keeper before the pin of a draw whose root it could not verify; with N-2's
  carry, a later cancel of that draw still steers nothing). `DeployStack` writes `payee`, `itemUnits` and `listingPaused` into the
  stack file.
- Contract docs: the `FloorModule` header and `burn` NatSpec, `SweepVault` header, `Wrapper` header, `docs/v5/floor-module.md`
  (§5 amendment, §12), and a correction note in `fixes-B-C-keeper-draws.md` §0 and §5 item 16.

**Tests.** The three review2 PoCs are regressions in `test/audit2/` asserting the attack now fails:

| File | Tests | What it shows |
| --- | --- | --- |
| `R2Floor.t.sol` | 5 | N-1a: 8 days of 1-wei posts, every day a burn pays at least the next hour's legal floor less the spread; a victim mints and burns back losing < 21 %, never profiting. N-1b: burns at +0, +61 min, +3 d, +9 d of a pause at exactly `β × (1 − base)`; mint refused (`Paused` while fresh, `Stale` after), no mint capacity, `itemCap` 0, `snapshotCap` refused; a pause right after a hostile post pays the backing; after the unpause the floor prices again |
| `R2Curve.t.sol` | 4 | N-3: the closing buy during a pause reverts `QueueFull`, the curve keeps trading and graduates after the unpause; a grad queued before a pause holds refunds past its window and graduates after the unpause; the hold ends at the hatch; no pause, no hold |
| `R2Draws.t.sol` | 5 | N-2: 30 void-and-re-announce cycles, one seed round and one snapshot time; guardian and owner cancels of Open and Announced draws re-open the same snapshot; a pin consumes the carry; carries are per coin; `requestDraw` refuses for a day of pause and pins after the unpause (the monitor's pre-pin pause relies on it) |
| `R3LateDelivery.t.sol` | 6 | canister N-2 both ways: pinned delivery refused a refund at the grace, recorded on day 10, never refunded; attested non-delivery refunds at the grace and refuses a later record; delivery first leaves nothing to refund; dead custodian refunds at 30 days (both clocks); only the custodian marks a Resolved ticket; pinned refunds unchanged |
| `R3GradRoom.t.sol` | 2 | B-07: a public mint cannot take a Queued grad's room and the grad fills; the accepted front-run measured: `QueueFull`, curve keeps trading, room back after the griefer's burns |
| `R3Roles.t.sol` | 2 | N-8 guardian separation (every role, pending roles, auto-buyer, owner transfer, treasury); N-6 the named keeper is refused as an NFT buyer while paused |

`BurnLiveness.invariant.t.sol` gains two handler actions (a 1-wei post at full confidence, pause/unpause); its liveness
invariant now requires a burn to go through whenever the module is fresh **or** `exitAtBacking()`, and a new hand-driven test
proves the handler reaches both states.

**Existing tests changed, and why:**

| File | Change | Why |
| --- | --- | --- |
| `test/Wrapper.t.sol`, `WrapperDeposit.t.sol`, `SweepVault.t.sol`, `YoyoVault.t.sol`, `floor/MintRate.t.sol`, `audit/B/FloorAuditB.t.sol`, `security/WrapperSecurity.t.sol` (W2, S1) | The custodian calls `markUndeliverable` before a Resolved ticket's refund | Canister N-2: that is the honest canister's path; the timing assertions (grace, both clocks) are unchanged |
| `Wrapper.invariant`, `WrapClaim.invariant`, `Yoyo.invariant`, `Sweep.invariant`, `audit/C/DrawFairness.invariant`, `floor/FloorModule.invariant` handlers | Same, inside the refund action | same |
| `security/WrapperSecurity.t.sol`, `security/SweepSecurity.t.sol` | ABI lists gain `markUndeliverable`, `undeliverable`, `LATE_DELIVERY_WINDOW`; `carriedSeed`, `seedOrigin` | the exact-surface checks |
| `test/utils/SweepBase.sol` `_openDraw`, `Sweep.invariant` `openDraw` | Use the seed round's randomness if it is already proven | a carried round has one randomness (the mock beacon, like drand, fixes it at the first proof) |
| `floor/FloorModule.t.sol` `test_theLagIsAlwaysPricedAgainstTheTrader` | A 20 % fall now burns at the lowest legal floor within the hour (`0.95 × accepted`), not the raw post | N-1a, deliberately (§1.1) |

**Runs** (final tree):

- `forge test`: **1021 passed, 0 failed** (114 suites; the fork suites return early without `--fork-url`). Before this branch: 996.
- `forge test --match-path "test/audit2/*"`: 24 passed (the three PoCs now assert the attack fails, plus the new regressions).
- Invariants at 500 runs × depth 150 (`BurnLiveness`, `DrawFairness`, `Sweep`, `Wrapper`): all pass.
- `forge test --match-path "test/fork/*" --fork-url robinhood` (public read-only RPC): **19 passed, 0 failed**.
- `script/deploy.test.sh`: **37 ok, exit 0**.
- `STACK_PORT=8697 scripts/golive-dryrun.sh stack` (a local anvil fork of 4663 from the public RPC; nothing sent to any public
  chain), full, with real drand: **DRY RUN: ALL PASS**, twice (before and after the services wave's two requests) — golive-check
  138 checks ok; launch → graduation through the module → module mint → sweep → draw announced → opened with the seed round's real
  evmnet signature → pinned → resolved with the pinned round's real signature → one whole wX claimed. The stack file carried
  `payee`, `itemUnits` (1e18) and `listingPaused` (false).

**Still open** (by design here, or for another branch): N-4, N-5 (§7); the B-07 front-run (accepted, §5); a compromised keeper
walking the floor down before it is paused (bounded, §1.1); the canister must call `markUndeliverable` (§8), or resolved refunds
wait 30 days; `NftWrapper`/`HomeWrapper` (OFF) lack the late-delivery gate; `golive-check.sh` should read the stack file's new
`payee`/`itemUnits`; a carried snapshot window can be days old if the keeper keeps declining (only a delay, never a choice).
