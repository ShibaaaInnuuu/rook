# Fixes for areas B and C: the keeper, burns, deliveries and draws

Branch `fix-keeper` (from `fixes-2026-09-26`, the review commit `d478e73`). Written before the code, as the design; the "As built"
section at the end is filled in when the code lands. Nothing here is deployed, and no key, wallet or live chain is touched.

Scope: B-01..B-13 (`B-floor.md`), C-01..C-16 (`C-treasury-roles-deploy.md`), and the on-chain parts of E-02 and E-03
(`E-services-workflow.md`). Other branches fix graduation (LiquidityLock, FrogCurve, the FloorModule graduation/cap path,
`SweepVault.sweepViaDesk`) and the canister; the diffs here stay in the keeper, burn, delivery and draw paths of `FloorModule`,
`SweepVault` and `Wrapper`, plus `ListingRegistry`, `ProtocolTreasury` and the deploy scripts those changes force.

## 0. The root cause, and the one decision everything else follows from

One hot EOA (K3 on Railway) is `registry.keeper()` **and** the immutable `FloorModule.keeper`. A leak is permanent for the module
(E-02, C-11), can only be answered in 7 days for the vaults, and nothing can stop the key in the meantime (B-02, B-09). A loss
locks the module's ETH for good (B-04).

**Decision: the registry is the one keeper indirection, and it gets an instant incident switch.** No new forwarder contract.

- `FloorModule` stops storing a keeper. It is built with the registry (`IKeeperSource`) and asks `registry.keeper()` on every
  keeper call, exactly as `SweepVault` already does. `FloorModule.keeper()` stays as a view (it returns the registry's), so the
  keeper service's "every module's keeper" check and `golive-check` keep working unchanged.
- **Rotation** is the registry's existing `proposeRoles` / `acceptRoles`: the Safe, 7 days in public (both clocks). One rotation
  now moves the keeper everywhere: module, vaults, auto-buyer caps.
- **Pause** (new, instant): `ListingRegistry.pauseKeeper()`, callable by the owner (the Safe) or the **guardian**; only the owner
  unpauses. While paused, `registry.keeper()` returns `address(0)`, so every keeper power everywhere reverts at once (floor, rental
  root, caps, draws, YOYO buyback, `AutoBuyerVault.postCap`, `WrapClaim.open`), and every inflow priced by the keeper's word stops:
  `FloorModule.mint`, graduation fills (`fill` / `enqueue`'s immediate fill), `fundFloat`, `itemSold`, `rentIn`; `SweepVault`
  treats its cap as not fresh (no `sweep`, `sweepViaDesk`, `relist`) and refuses `requestDraw`. **Burns are not paused**: they pay
  at most the paid-in backing per unit (below), so they cannot be used against the protocol, and an exit must never depend on a key.
  *(Correction, R3: as first built, a pause made burns revert `Stale` from +1 h until the 7-day stale window, and a fresh 1-wei
  target zeroed them — review2 N-1. Since `review2/fixes-R3-contracts.md` §1 burns pay the backing per unit less the base spread
  from the moment of the pause, and the burn side bounds the target, so this sentence is now true. R4: while paused they pay the
  lower of that backing and the floor the keeper left, `pauseFloor()`, less the base spread; the backing alone once `escaped()` —
  `review3/fixes-R4-contracts.md` §1.)*
- **Guardian** (new, optional): `ListingRegistry.setGuardian(address)`, owner-only and instant (the guardian can only stop things:
  pause the keeper and cancel a draw before it is pinned). Intended holder: an automated key on the monitor's host (not the
  keeper's), so a bad `FloorPosted`/`DrawOpened` can be answered in seconds instead of by a solo operator at two devices.
- Why not a forwarder: it would duplicate the registry's rotation and role-separation checks, and the vaults already read the
  registry. The registry is not deployed yet, so extending it costs nothing.

Pausing and then rotating is the recovery: pause (seconds, guardian or Safe), `proposeRoles` with a new keeper (7 days), accept,
unpause. During the week the module is stale; after 7 days without a good post, burns switch to the stale fallback (§2), so a
keeper incident is a liveness incident, never a loss.

## 1. The fixes, one by one

| Finding | Fix | Where |
| --- | --- | --- |
| **E-02, C-11, B-04 (root), B-09** | Registry-read keeper; instant `pauseKeeper` by Safe or guardian; the module and vaults honour it (above) | `ListingRegistry`, `FloorModule`, `SweepVault` |
| **B-04** stale lockup | **Stale fallback.** If no post with confidence ≥ `minConfidenceBps` has been made for `FLOOR_TTL + STALE_ESCAPE` (1 h + 7 days) on both clocks, anyone may burn at the backing per unit of each tier, less the base spread, with no floor price. Also then `skim` sends the reserve surplus to the platform (the treasury), so no ETH is locked for ever. Measured from the last *good* post, so a thief posting confidence 0 every 50 min (E-02a) cannot keep the module frozen past a week | `FloorModule.burn`, `escaped()`, `skim` |
| **B-02** surplus drain | The standing reserve is split: `reserve` is now the **basis** (the ETH reserve units paid in: every mint's and graduation's reserve share) and `surplus` holds `fundReserve` (curve fee slices, the treasury's 90%) and `itemSold` proceeds. Reserve-tier burns pay at most `reserve / reserveUnits` (the paid-in average) and release exactly that per unit; the spread they keep goes to `surplus`. The surplus is spent only on conversions (`itemArrived` tops up from `revenue`, then `surplus`; custodian-only and capped by the custodian's own checks) and, after the stale window, skimmed to the treasury. **Floor moves are bounded per day**: besides ±5 %/h, the accepted floor stays within ±20 % (`MAX_DAY_MOVE_BPS = 2_000`) of a daily anchor (the accepted floor when the current 24 h window opened, both clocks). Lag is always priced against the trader, so the bound costs liveness in a crash, never safety | `FloorModule` |
| **B-01** dust tier | A burn **spans the tiers**: `min(units, R)` reserve units at the reserve's price, the rest convertible units at theirs, in one call. `burnCapacity()` reports all ETH-backed units (within the hourly cap) and the lower of the two tier prices; `quoteBurn` is exact. A 1-wei reserve tier can no longer stop anything. New invariant: whenever `ethBacked > 0` and the module is fresh or escaped and under the hourly cap, a holder can burn one whole unit (or all it holds) for a non-zero amount | `FloorModule.burn`, `_release`, `burnCapacity`, `quoteBurn` |
| **B-03 / C-05** forged delivery | `Wrapper.recordDelivery` is **custodian-only** (error `NotCustodian`) | `Wrapper` |
| **C-01** ceiling never active | (a) The **first** `proposeCapCeiling` applies at once: with no ceiling the cap is unbounded, so any first ceiling only tightens. Raising still waits 7 days, lowering stays instant. (b) `postCap` refuses while there is no ceiling (`NoCeiling`), so a vault can never be priced unbounded. (c) Any post, fresh or stale, may rise at most `MAX_STEP_BPS` (20 %) above the last cap; a stale re-post may fall freely (lower is safer). DeployStack makes the ceiling part of the deploy: required on a real chain, a default on our own | `SweepVault.postCap`, `proposeCapCeiling`; `DeployStack` |
| **C-02, E-03** redraw grinding | **The pinned round is final.** `redraw` is removed from `SweepVault` and `Wrapper`. drand never skips a round (a halted network signs missed rounds on catch-up), so every pinned round can always be proven by anyone from the public signature. A draw or ticket nobody resolves for `PIN_TIMEOUT` (7 days, both clocks) is not re-rolled: the draw is voided (its unit returns to the coin) and the ticket is refunded (below). Nobody can see an outcome and pick another round | `SweepVault`, `Wrapper` |
| **Canister M-1** late delivery (`fixes-D-canister.md`) | `Wrapper.refund` of a Resolved ticket waits `REFUND_GRACE` (72 h) past `deliverBy`, both clocks: a delivery broadcast before the canister's cut-off that confirms late is recorded first, so nobody keeps both the frog and the token | `Wrapper.refund` |
| **E-03** stuck Pinned ticket | `Wrapper.refund` also takes a **Pinned** ticket once `PIN_TIMEOUT` has passed since its pin, on both clocks: the token is minted back, no frog moves. In-order resolution skips refunded tickets (`nextToResolve` advances over them), so one stuck ticket no longer blocks every later one | `Wrapper.refund`, `resolve` |
| **C-03** unbound draw inputs | The published tree carries a **sentinel leaf** `leaf(address(0), totalTickets, snapshotAt)` (the usual double-hashed leaf; `claim` refuses holder zero, and its range is empty or backwards, so it can never be claimed). `openDraw` requires its proof. So the root commits the ticket count and the snapshot time: publishing the honest leaves with a truncated total gives a different root, which any recomputation catches. `snapshotAt` is no longer the keeper's word (next row) | `SweepVault.openDraw` |
| **C-04** JIT snapshot | **The snapshot time is chosen by a drand round that did not exist when the draw was announced.** `announceDraw(coin)` (keeper) reserves the unit and pins a future `seedRound`; `openDraw(id, root, totalTickets, uri, sentinelProof, seedSignature)` proves (or reads) the seed and fixes `snapshotAt = announcedAt − SNAPSHOT_WINDOW + (seed mod SNAPSHOT_WINDOW)`, `SNAPSHOT_WINDOW = 24 h`: a moment in the day *before* the announcement, unknown until after it. Holding across the whole day is the only sure way to be in it, so capture is proportional to time held. The snapshot block is, by rule, the last block whose timestamp is ≤ `snapshotAt` (the parent-chain `block.number` the keeper passed before could not name an L2 block and bound nothing) | `SweepVault` |
| **E-02** forged draws in bulk | At most **one draw per coin before its pin** (Announced or Open). An announced draw the keeper does not open within `OPEN_TTL` (6 h) can be voided by anyone. The owner **or guardian** may cancel any draw that is not yet pinned (the outcome is unknown then, so a cancel steers nothing); while the keeper is paused `requestDraw` refuses, so a forged draw cannot proceed during an incident | `SweepVault` |
| **Anyone resolves** | Unchanged and kept: `requestDraw`, `resolve`, `claim`, `voidDraw`, `Wrapper.resolve` and `refund` are permissionless. A dead keeper stops new draws only; it cannot hold a user's ticket or prize | — |
| **C-07** Phase-1 treasury | The constructor defaults are Phase 1's: shares 9000/0/0/1000, `bootstrapLoan = 0`. Stage 1 restores 40/30/20/10 and 2 ETH by `setShares` / `setBootstrapLoan` (instant, as before). `setShares` moves an undelivered floats or bootstrap pot into the reserves pot when its share goes to 0, so an OFF path can never be paid from income that arrived before the switch | `ProtocolTreasury` |
| **C-06** Safe drains treasury | **Not changed** (a legitimate, documented Safe power); the NatSpec is corrected and §3 below states it exactly for the Trust page | `ProtocolTreasury` NatSpec |
| **C-12** one bad module blocks `distribute` | `fundReserve` is called in `try`, like `fundFloat`: a refusing module's share stays in the pot and the others are paid | `ProtocolTreasury.distribute` |
| **B-05** unchecked wiring | `Wrapper.wireFloorModule` requires `module.wrapped() == this` | `Wrapper` |
| **B-10** natspec drift | `itemArrived` NatSpec and the contract header's keeper claim (`:38-39`) corrected; floor-module.md §5, §6.3, §12, §13.1 corrected | docs |

## 2. The stale fallback, exactly

`escaped()` is true when the floor is not fresh and `lastGoodAt + FLOOR_TTL + STALE_ESCAPE` has passed on both clocks
(`STALE_ESCAPE = 7 days`), where `lastGoodAt` is the time of the last post with confidence ≥ `minConfidenceBps` (and the module
has ever had one). In that state:

- `burn` pays per unit `β × (BPS − baseSpreadBps) / BPS` of the tier it draws (`β_R = reserve / R`, `β_C = backing / C`), spanning
  the tiers as above, within `burnPerHour`. This keeps M4, M6 and M10 by construction (never above the tier's backing per unit;
  the spread stays in the pots) and needs no key.
- `skim` also sends the whole `surplus` to the platform (the treasury).
- Nothing else runs: no mint, fill, float, sale, rental or `itemArrived` (they need a fresh floor or a fresh cap).
- A new good post ends it.

## 3. Roles and powers after the fixes (for the Trust page wave)

| Role | Powers (Phase-1 contracts) | Delay | Worst case if the key is compromised |
| --- | --- | --- | --- |
| **Safe 2-of-2** (registry owner) | Rotate keeper/purchaser/attestor (`proposeRoles` → `acceptRoles`) | 7 days, both clocks | Install its own keeper after 7 public days, then everything in the keeper row (bounded as there) |
| | `pauseKeeper` / `unpauseKeeper`, `setGuardian` | instant | Unpause a keeper the guardian paused; replace the guardian |
| | `SweepVault.cancelDraw` (any draw not yet pinned), `proposeCapCeiling` (first: instant; lower: instant; raise: 7 days), `setMarkup` | as listed | Cancel every draw before its pin (units return to their coins); a ceiling as high as it likes 7 days after proposing it |
| | `ListingRegistry.setPaused`, listings, `transferOwnership` (2-step, no delay), `setBondBps` raise | instant | Pause sweeps and draws (not redemptions, not burns) |
| | `ProtocolTreasury.setShares`, `setOps`, `setBootstrapLoan`, `addCollection`, `forgive` | instant | **Takes the whole treasury balance at once, allocated pots included** (C-06: add a fake module that names this treasury, set shares/ops/loan, allocate, distribute, bootstrap). Not reduced here; the Trust page must say it |
| | `WrapClaim.initMintLimit` / `wireFloorModule` (once, before any mint), `proposeMintLimit` (lower instant, raise 7 days), `cancel` | as listed | Before step 7 only: wire a fake module (now refused unless it names the wrapper, B-05) |
| | Cannot: move FloorModule ETH, sweep-vault ETH, curve ETH or locked LP; record a delivery; post a floor or cap without first becoming keeper (7 days) | | |
| **Guardian** (optional; an automated key on the monitor host) | `pauseKeeper`; `SweepVault.cancelDraw` (not yet pinned) | instant | Denial of service only: keeper paused (module mint/fill and sweeps stop; burns continue; the Safe unpauses and replaces it), draws cancelled before pin (units return to their coins) |
| **Keeper** (hot EOA = `registry.keeper()`; follows rotation everywhere; `address(0)` while paused) | `FloorModule.postFloor` (±5 %/h and ±20 % per 24 h window), `postRentalRoot` (OFF); `SweepVault.postCap` (≤ the ceiling, which must exist; up by ≤ 20 % per post, 10-min gap while fresh), `announceDraw` / `openDraw` (one per coin before pin; snapshot time from drand; root commits total and time), `buyback` (YOYO, OFF); `AutoBuyerVault.postCap`; `WrapClaim.open` (OFF) | rotation 7 days; pause instant | Until paused: cheap mints on a walked-down floor, at most `maxPublicEthBacked × F × 20 %` per day (≈ 1.3 ETH/day for frogs); sweeps at up to the ceiling; forged draw roots, caught by recomputing the root (which now commits total and time) and cancelled before the pin; a frozen module (confidence 0), which the stale fallback ends after 7 days. **No longer:** drain the reserve surplus through burns, forge deliveries, choose the snapshot block, re-roll a draw, or keep the module's ETH locked |
| **Custodian** (canister EVM address, immutable) | `wrapDeposit`, `itemArrived` (cost ≤ `maxItemCost`, now topped up from the surplus), `itemSold`, `reclaimStray`, `recordDelivery` (**only it**), `recordReturn`, `rentIn` (OFF), YOYO `recordSale`/`reclaimListing` (OFF) | canister timelock 72 h | Custody-level, as before |
| **Anyone** | `mint` (fresh, not paused), `burn` (fresh, or escaped after 7 days), `fill`, `skim`, `requestDraw`, `resolve`, `claim`, `voidDraw` (resolved + 7 days, pinned + 7 days, announced + 6 h), `Wrapper.resolve`, `refund` (resolved past `deliverBy` + 72 h, or pinned + 7 days), `acceptCapCeiling`, `acceptMintLimit`, `allocate`, `distribute`, `bootstrap` (0 in Phase 1) | — | — |

## 4. What changes for deploy

- `FloorModule`'s second constructor argument is now the registry (`IKeeperSource`), not a keeper address.
  `DeployStack` passes the registry; the read-back checks `module.keeper() == registry.keeper()` and `module.registry()`.
- `ETCH_CAP_CEILING` is **required** on a real chain (the vaults refuse caps without a ceiling); on our own chains DeployStack
  uses a rehearsal default. The owner step `proposeCapCeiling` ×3 now **activates** the ceiling at once; there is no
  `acceptCapCeiling` step to forget.
- Optional `ETCH_GUARDIAN`: DeployStack makes (locally) or prints (on a real chain) `registry.setGuardian(guardian)`.
- The treasury needs no `setShares(9000,0,0,1000)` / `setBootstrapLoan(0)` step any more (constructor defaults); stage 1 restores
  the old split by those calls.
- `StackRehearsal` and `golive-dryrun.sh stack` follow the new draw flow: announce, open with the seed round's real drand
  signature and a sentinel leaf, request, resolve, claim.

## 5. What the off-chain wave must change

To be done on the services / site branch; each item names the on-chain change that forces it.

**Keeper service (`keeper/service/`)**

1. Draws: `announceDraw(coin)` at the slot, then wait for `seedRound` (read `draw(id).seedRound`), fetch its signature from two
   drand relays, read `snapshotAt` (or compute it: `announcedAt − 86400 + (uint256(randomness) mod 86400)`), take the snapshot at
   the last L2 block with timestamp ≤ `snapshotAt`, build the tree **with the sentinel leaf** `(address(0), totalTickets,
   snapshotAt)`, publish, then `openDraw(id, root, totalTickets, uri, sentinelProof, seedSignature)` within 6 h. ABI: `openDraw`
   changed, `announceDraw`, `DrawAnnounced`, `SnapshotFixed` new; `Draw` has four new trailing fields.
   Read `seedRound` from the mined announcement (the receipt's `DrawAnnounced` or `draw(id)`), never from a simulation: the
   rehearsal's first run fetched the simulated round's signature and the beacon refused it (`InvalidSignature`).
2. A keyless **resolver**: `requestDraw` after each window, `resolve` every pinned draw and every pinned `Wrapper` ticket in order as
   soon as its round exists (two relays), `claim` for the winner. Without it, draws void and tickets refund after 7 days (E-03).
3. `planPost` must re-post when `!fresh()` or the posted confidence is below the minimum (E-02a), and must plan within the new
   ±20 %/24 h anchor (read `dayAnchor`, `dayAnchorAt`) as well as ±5 %/h.
4. `postCap` needs a ceiling: refuse to plan above `capCeiling`, and treat `capCeiling == 0` as "cannot post" (the vault reverts
   `NoCeiling`). Upward steps are bounded by 20 % even when stale.
5. The signer check "key == `registry.keeper()` and every module's keeper" still holds (`module.keeper()` is a view of the
   registry's); add: refuse to send while `registry.keeperPaused()`.
6. Stop sending `recordDelivery` (never did; the ABI entry, if any, goes). The canister's `may_record` keeper fallback
   (`custodynet.rs`) is now dead code: only the custodian records.

**Monitor / reconciler (`monitor/`, `ops/`)**

7. Watch `FloorPosted` (confidence below minimum; accepted moves not in the keeper's plan), `CapPosted`, `DrawAnnounced`,
   `DrawOpened` (recompute the root with `tools/snapshot` at `snapshotAt` on a second RPC; page on mismatch), `KeeperPaused`,
   `GuardianSet`, `RolesProposed`.
8. Hold the **guardian** key on the monitor's host (not the keeper's project) and call `pauseKeeper()` / `cancelDraw(id)` on a
   root mismatch or an out-of-plan floor move; page the operator.
9. Page on: a Pinned ticket or draw older than 1 h past its round; an Announced draw older than 1 h; `escaped()`; `keeperPaused()`.
10. Reserves (E-05): `escrowBalance()` is still `backing + reserve` (what backs the units); add `surplus()` to the display and
    compare `backing + reserve + surplus + revenue + …` with the module's balance.

**Snapshot tool (`tools/snapshot/`)**

11. Input `--time T` (the draw's `snapshotAt`): resolve the block as the last L2 block with timestamp ≤ T; add the sentinel leaf
    `(address(0), totalTickets, snapshotAt)` to the tree; output it with its proof. Update `docs/snapshot-v1.md` to v2 (drafted in
    §6 below).

**Site (`web-rook/`)**

12. Redeem page: remove Redraw (`redraw` no longer exists); show "Refund" for a Pinned ticket 7 days after its pin, and for a
    Resolved one 72 h past `deliverBy` (`REFUND_GRACE`); add a "Resolve" button (anyone, fetches the drand signature). Raffle: show announced/open/pinned draws, a
    Resolve and a Claim button.
13. Trust page: the §3 table (keeper reach and its bounds, the guardian, the pause, the 7-day rotation, the stale fallback, "the Safe
    can take everything in the treasury at once", raffles rely on the keeper's root plus a pre-pin cancel).
14. ABIs: `Wrapper.redraw` gone, `refund` semantics wider; `SweepVault` draw ABI; `FloorModule.surplus`, `escaped`, `burnCapacity`
    semantics (spans tiers; works when escaped); `ListingRegistry.pauseKeeper/unpauseKeeper/keeperPaused/guardian/setGuardian`.

**Go-live check and runbook (`script/golive-check.sh`, `docs/v6/phase1-scope.md`, `launch-gates.md`)**

15. Fail (not note) on: `capCeiling == 0` on any vault; `keeperPaused()`; treasury shares ≠ 9000/0/0/1000 or `bootstrapLoan ≠ 0`
    at stage 0; guardian unset (or accept in writing); `module.registry() != registry`.
16. Runbook row "Keeper key stolen": guardian or Safe `pauseKeeper()` (instant) → `cancelDraw` for any unpinned draw →
    `proposeRoles(new keeper)` → 7 days → `acceptRoles` → `unpauseKeeper()`. Holders can burn throughout (at ≤ backing), and
    from day 7 without any floor. *(Correction, R3: from the pause, at the backing per unit less the base spread, no floor —
    `review2/fixes-R3-contracts.md` §1. R4: from the pause at the lower of the backing and the floor the keeper left, the backing
    alone from day 7 — `review3/fixes-R4-contracts.md` §1.)*
17. Go-live step 7 loses `setShares`/`setBootstrapLoan` and "accept the ceiling"; gains `setGuardian`.

## 6. Snapshot rules v2 (for the tool; replaces the block rule of `docs/snapshot-v1.md` for SweepVault draws)

- **Time, not block.** A SweepVault draw names `snapshotAt` on chain (`SnapshotFixed`). The snapshot block is the last block of the
  chain (4663's own numbering) whose timestamp is ≤ `snapshotAt`. Balances are applied through that block, as in v1.
- **Sentinel.** The tree is v1's (weighted ranges, sorted leaves, OpenZeppelin pair hash) plus one leaf
  `keccak256(keccak256(abi.encode(address(0), totalTickets, snapshotAt)))`. It commits the count and the time to the root; `claim`
  refuses it (holder zero).
- Everything else (exclusions, ordering, output) is v1's. The v1 Distributor (OFF on 4663) keeps v1.

## 7. Not fixed here (listed, with the reason)

| ID | Why not here | Suggested owner |
| --- | --- | --- |
| B-06 `itemArrived` absolute balance | Custodian-only, needs a wrapper→module arrival counter across both contracts; the canister already tracks arrivals | canister/graduation wave |
| B-07 graduation room griefing | Graduation lane (another branch) | graduation branch |
| B-08 burn race before `itemArrived` | Touches `snapshotCap` (the cap path, another branch) | graduation branch |
| B-11 `volume7d` churn | Only matters with two collections | stage 1 |
| B-12 `snapshotCap` permissionless | Cap path (another branch) | graduation branch |
| B-13 | Informational; confirmed OFF paths | — |
| C-06 | A documented Safe power, kept (§3) | Trust page |
| C-08 golive-check gaps | Go-live script is the later wave (§5 item 15) | off-chain wave |
| C-09 Trust page | Later wave (§5 item 13) | off-chain wave |
| C-10 `sweepViaDesk` griefing | `sweepViaDesk` is another branch's | graduation branch |
| C-13 drand pin vs sequencer lag | Needs a parent-chain-derived round or a larger `PIN_AHEAD`; measured lag ~1 s | open decision |
| C-14 Pons launch shares the stack's listing | A deploy-layout decision (a separate listing id) | open decision |
| C-15 `CollectionDirectory.admin` immutable | Curve area; small | graduation branch |
| C-16 `retire` not pause-gated | By design (redemption is not pausable) | — |
| NftWrapper / home-chain `recordDelivery` by keeper | NFT and home chain are OFF and not deployed | before NFT goes ON |

## 8. Open decisions for the operator

1. `MAX_DAY_MOVE_BPS = 2_000` (20 % per 24 h) and `STALE_ESCAPE = 7 days`, `PIN_TIMEOUT = 7 days`, `OPEN_TTL = 6 h`,
   `SNAPSHOT_WINDOW = 24 h`: constants chosen here; change before deploy if wanted.
2. A guardian key, and where it lives (recommended: the monitor host, a separate Railway project).
3. A separate draw-opening key from the floor/cap key (C-11's last suggestion) was not done: it needs a fourth registry role.
4. The challenge window stays 15 minutes (the guardian can cancel until the pin, not only in the window). Lengthen if no guardian.
5. C-13 and C-14 above.

## As built

Commits on `fix-keeper`: `08ba4aa` (this design), `24a0496` (the fixes and their tests), the merge of `fixes-2026-09-26`
(area A's graduation fixes; no textual conflict), and `1f2a29e` (the area-A test base on the new constructor, and A-10).

**As designed**, with these additions and precisions:

- **A-10** (handed over by area A): `gradPrice()` counts the keeper's raw `target` only up to the top of the day's band
  (`max(anchor × 1.2, accepted)`), so a far-off post cannot shrink a graduation's units by more than the band. Mints and burns keep
  the raw `target` (a far-off one only makes them worse for the trader, who quotes first).
- The stale fallback's `skim` takes **all** revenue beyond unpaid rental debt, not only what exceeds `revenueTarget` (no conversion
  can use it any more), plus the whole surplus: `test_B04_…` ends with the module's balance at zero.
- `floorBounds()` returns the intersection of the hourly and the daily bounds, so the keeper service can plan against one view.
- `SweepVault.capFresh()` is false while the keeper is paused; `sweepViaDesk` (made keeper-only on the area-A branch) inherits it.
- `HomeListing` (home chain, OFF, not deployed) forwards `keeperPaused()` and `guardian()` to the registry, because
  `HomeSweepVault` is a `SweepVault` and now reads them.
- **Canister M-1** (requested after the canister branch merged): `REFUND_GRACE = 72 hours` after `deliverBy` before a resolved
  ticket refunds (`test_M1_aLateDeliveryIsRecordedBeforeAnyRefund`). Tests that refunded at `deliverBy` now wait the grace.
  The `Redeemed`/`Resolved` events the canister reads are unchanged.
- `Wrapper` emits `Refunded(id, type(uint256).max, owner)` for a ticket refunded while pinned (no frog was drawn): the indexer must
  read that sentinel (off-chain wave, §5 item 14).

**Tests.** Regressions of every B and C PoC that this branch fixes, asserting the attack now fails: `test/audit/B/FloorAuditB.t.sol`
(B-01, B-01b, B-02, B-02b, B-03, B-04, B-09, E-02, E-02a, A-10 and the PoC's round-trip fuzz), `test/audit/C/C_SweepVault.t.sol`
(C-01, C-01b, C-02, C-03, C-04 fuzz, E-02, E-03), `test/audit/C/C_ProtocolTreasury.t.sol` (C-07, C-07b, C-12, and C-06 pinned as a
documented power). Invariants: `test/audit/B/BurnLiveness.invariant.t.sol` (a holder can always burn when backing exists; no lockup
after the stale window; burns never above the backing; the books), `test/audit/C/DrawFairness.invariant.t.sol` (a draw's round and
winner, and a ticket's round, are fixed at the pin whoever resolves and whenever, under a beacon with one valid signature per round).

**Existing tests changed, and why** (each to the new semantics, none weakened except where the old behaviour was the finding):

| File | Change | Why |
| --- | --- | --- |
| `test/utils/SweepBase.sol`, `CurveBase.sol`, `PoolBase.sol`, `test/floor/{FloorGraduation,FloorRouter,Treasury}.t.sol`, `test/pools/Collections.t.sol`, `test/YoyoVault.t.sol`, `test/homechain/HomeListing.t.sol`, `test/audit/A/AuditABase.sol` | The owner sets a ceiling on every vault in `setUp`; `_postCap` climbs in legal steps; `_openDraw` helpers | C-01: no cap without a ceiling; stale posts rise one step at most. The draw flow is now announce → open |
| `test/floor/FloorBase.sol`, `Treasury.t.sol`, `FloorRouter.t.sol`, `AuditABase.sol` | `new FloorModule(…, IKeeperSource(registry), …)`; `_books` counts `surplus` | E-02/C-11: no key in the module; B-02: the surplus pot |
| `test/SweepVault.t.sol` | `openDraw` tests → announce/open, the sentinel, `OPEN_TTL`, one draw per coin; the redraw tests → "the pinned round is final, void after a week"; cancel → owner or guardian until the pin | C-02, C-03, C-04, E-02 |
| `test/security/SweepSecurity.t.sol` | The ABI list; the aim fuzz asserts there is no redraw at any age; the stale fuzz checks the announcement's window | as above |
| `test/Sweep.invariant.t.sol` | The handler announces and opens with a sentinel and claims with its one-step proof; no redraw; reservations count Announced draws | as above |
| `test/Wrapper.t.sol`, `WrapClaim.t.sol`, `WrapperDeposit`-style callers | Deliveries recorded by the custodian; the keeper is refused. "A pinned ticket is never refunded" and the three redraw tests → "refunded a week after its pin, resolution moves past it" and "the pinned round is never replaced"; the wiring tests use a stand-in that names the wrapper | B-03/C-05, E-03, C-02, B-05 |
| `test/security/WrapperSecurity.t.sol` | ABI list (no `redraw`, `PIN_TIMEOUT`); S2 → no re-pin at all, and a timestamp jump cannot bring the pinned refund forward | C-02, E-03 |
| `test/Wrapper.invariant.t.sol`, `test/WrapClaim.invariant.t.sol` | The `redraw` action → `refundPinned`; the order invariant allows a ticket refunded while pinned after the turn | E-03 |
| `test/floor/CapStep.t.sol` | The ceiling test → no cap without one, the first at once, a stale post one step up at most, lower at once, raise in 7 days | C-01 |
| `test/floor/FloorReserve.t.sol` | "Burns draw the reserve first" → and span the tiers; "the reserve keeps the spread" → the basis per unit stands and the spread goes to the surplus; `fundReserve` and `itemSold` → surplus; the pushed floor stops at the day's band | B-01, B-02. The old tests asserted the surplus backs the next reserve units, which is exactly the B-02 drain |
| `test/floor/FloorModule.t.sol` | The wiring test expects `NotItsModule` for another wrapper's module; the post-limit fuzz also checks the day band | B-05, B-02 |
| `test/floor/FloorModule.invariant.t.sol` | The burn handler computes the spanned release; solvency counts `surplus`; zero-value burns are skipped (the module rightly refuses them) | B-01, B-02 |
| `test/floor/Treasury.t.sol` | `setUp` sets the stage-1 split (40/30/20/10, 2 ETH) the legitimate way; reserve deltas read `surplus` | C-07 (the defaults are now Phase 1's); B-02 |
| `test/nft/NftSurface.t.sol`, `test/homechain/HomeSurface.t.sol` | The registry's and `HomeListing`'s ABI lists gain the pause and the guardian | the incident switch |

**Runs** (on the final tree, after both merges of `fixes-2026-09-26`):

- `forge test`: **995 passed, 0 failed** (107 suites; the fork suites return early without `--fork-url`).
- `forge test --match-path "test/fork/*" --fork-url robinhood` (public read-only RPC): **19 passed, 0 failed**.
- `script/deploy.test.sh`: **37 ok, exit 0**.
- `STACK_PORT=8677 scripts/golive-dryrun.sh stack` (fork of 4663 from the public archive RPC, all local): **DRY RUN: ALL PASS** —
  golive-check 120 checks ok; launch → graduation → module mint → sweep → draw announced (seed round read back from the chain) →
  opened with the seed round's real drand signature (snapshot time fixed) → pinned → resolved with the pinned round's real
  signature → one whole wX claimed by the holder. Run before the refund grace and the canister merge, neither of which the
  rehearsal's path touches.
