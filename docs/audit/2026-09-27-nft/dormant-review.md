# Security review: dormant, wake-on-demand collections (branch `dormant-collections`)

Date 2026-09-29. Scope: `git diff 9efed1b..6bb54d7` (the design `docs/v7/dormant-collections.md`, `WakeDesk.sol`, the canister's
`dormancy.rs` and its hooks in `ethflow.rs` / `ethnet.rs`, the runner, monitor and site changes, and the launch tooling). One
adversarial AI review of the author's own diff, then fixes on the same branch. Nothing was deployed, pushed or signed; the only
chains written to were local anvil forks of the dry run; every network call was public and read-only.

**Threat model.** An outsider who can launch coins for gas, request wakes with the bond, send cheap NFTs to the canister, wash
trade and self-list, and time any of it; a lagging or failed runner (late or missing hand-ins); an uncertified relay of
`GET /collections`. The canister's key, the operator and a timelocked upgrade are trusted as in the earlier rounds. The questions
the task named: **manipulation during WAKING**, **griefing by wake spam** (each wake costs cycles), and **a stale floor at buy
time**.

**Severity scale** as `README.md`.

## Counts

| | Found | Fixed | Mitigated / accepted |
|---|---|---|---|
| High | 3 | 3 | 0 |
| Medium | 4 | 4 | 0 |
| Low | 5 | 4 | 1 (DR-L-2, by DR-H-1's reclaim rule and the watchdog) |

The fixes are commits `ef292b0` (canister, monitor), `134a06c` (site), `1e3ef63` (`WakeDesk`) and `984222c` (launch file, dry
run). Each fix has a test that fails without it.

---

## High

### DR-H-1: a bare or sweep-only launch pins a collection awake for gas alone, and gets the bond back

**Where:** `check_life` (ACTIVE: "a coin still Trading keeps it awake", `launched_at` reset the idle clock), `WakeDesk.reclaim`
(any launch by the requester), `CurveFactory.launch(sweepOnly = true)` (skips `hasRoomFor` and `freshCap`).

**The attack.** `request(wrapper)`, launch a sweep-only coin, `reclaim`: the bond is back, the collection wakes, and its Trading
curve (which never leaves Trading unless bought out) keeps it ACTIVE for good. Three times fills `max_active = 3`: every other
collection is unwakeable, owed wakes included, and the canister burns ≈ 6 T a day. It also happens with no attacker: any honest
coin that never fills pins its collection.

**Fix.**
- Canister: a launch is not a position. ACTIVE stays only for what holds value or waits on the floor: wX supply, items, rentals,
  queued or unreleased graduation, an advance, a float (all read on 4663), or anything open in the canister. The idle clock is
  `idle_days` since a position was last seen, or `NO_POSITION_SECS` = 6 h after ACTIVE when none came since the wake. A graduation
  that reaches a stale module raises `gradEth`, which the DORMANT watchdog reads (6 h) as an owed wake.
- Owed wakes (a deposit, a queued graduation) get one slot above `max_active` (`OWED_RESERVE`), so desk wakes can never lock them out.
- `WakeDesk.reclaim` needs a coin that is not floor-buys-only, whose curve is Graduating or Graduated (it reached its raise, ≥ 5 ETH
  bought), and that has not paid back a bond before.

**Tests:** `dr_h_1_a_wake_nobody_uses_sleeps_six_hours_after_active_whatever_is_launched`,
`the_watchdog_ignores_a_never_posted_module_and_a_bare_launch_and_wakes_on_a_graduation`, the reserve in
`dr_h_2_an_owed_wake_that_times_out_backs_off_and_the_owed_reserve_lets_it_start_past_max_active` (unit), forge
`test_reclaim_paysTheRequesterBack_whenTheyLaunch` (NotGraduated first) and `test_reclaim_refusesAFloorBuysOnlyCoin_andOneCoinPaysBackOneBond`.

**Residual.** A collection with real holders (wX in a pool after a graduation) is correctly pinned ACTIVE: it needs a fresh floor
for burns. Once `max_active` collections have holders, the rest can only be woken through the owed reserve. That is the cycles
budget working as designed: raising `max_active` is a timelocked change, and golive-check fails when the budget does not cover it.

### DR-H-2: an owed wake that cannot confirm loops forever on the only waking slot

**Where:** `dorm_pass` step 3 (an open deposit re-requests an owed wake every tick), `start_wakes` (owed wakes skip the cooldown),
`timeouts`.

**The attack.** Send one cheap NFT of a thin collection (Checks, ≈ 0.16 ETH) to the canister. Its owed wake times out every 12 h
and restarts at once, holding `max_waking = 1` for good: no desk wake ever starts. The PoC ran 10 simulated days.

**Fix.** A timed-out owed wake counts (`owed_fails`) and the next owed wake of that collection waits `cooldown × 2^(fails − 1)`
(at most 7 days, `owed_after`). After `OWED_FAILS_MAX` = 3, a deposit that asked for the wakes is put on the existing return path
(`XPhase::Return`, judged by `judge_return` as any stray) and its queued request is dropped. A wake that becomes ACTIVE clears the
count. Over the three tries the slot is held 36 h of about 4.5 days, and never again.

**Tests:** unit `dr_h_2_an_owed_wake_that_times_out_backs_off_and_the_owed_reserve_lets_it_start_past_max_active`; fake world
`dr_h_2_a_deposit_whose_owed_wakes_keep_timing_out_goes_back_to_its_sender_and_never_holds_the_slot` (a desk wake of another
collection starts afterwards).

### DR-H-3: bonds forfeited for wakes the canister never served

**Where:** `WakeDesk.forfeit` and the canister's queue.

A request can wait behind `max_active`, a cooldown or `wakes_per_day` past the window plus grace. The requester could not launch
an item-buying coin (the module was never fresh), yet the bond went to the treasury while the site said "queued (#n)".

**Fix.** `forfeit` reads the collection's module: if it was posted at or after the request (`postedAt() ≥ at`), the wake was
served and the bond goes to the treasury; otherwise it goes back to the requester (`Refunded`), with a 50,000-gas stipend so a
requester that refuses ETH or burns gas only sends its own bond to the treasury. A module that cannot say counts as served.
`served(id)` is a view for the site. The window is 7 days at launch (`nft-pilot.json`), time for a real coin to fill.

**Tests:** forge `test_forfeit_refundsAWakeTheCanisterNeverServed_andChargesOneItServed`, `test_theDeskHoldsOnlyBonds_eachRequestItsOwn`.

**Residual (accepted).** A requester who asks for wakes the canister cannot serve (a collection too thin to confirm its floor in
12 h) gets the bond back. The cost to the canister is bounded by its own caps: `wakes_per_day` 4 in all and a 24 h cooldown per
collection, so at most 4 backfills a day (≈ 4–6 T), paged by the monitor at each timeout. The operator can raise the bond or drop
the collection from the allowlist, both timelocked.

---

## Medium

### DR-M-1: the first posts after a long dormancy are clamped, and fills and buys ran on them

The module bounds each post (`maxMoveBps` an hour, ±20 % of the day anchor, which is the stale `accepted`). A collection became
ACTIVE on the first clamped post, and `Step::Fill` converted queued graduations at a price that had not caught up (too many or too
few units); new buys were started and then refused by the 5 % MISMATCH check, dropping listings.

**Fix.** `dm::converged(reading, accepted)`: no `Step::Fill` and no new buy (under the dormancy) until the module's accepted floor
is within `BUY_MISMATCH_BPS` (500) of our reading. The next posts walk it there; the hunt then fills.

**Test:** `dr_m_1_after_a_long_dormancy_the_fill_waits_until_the_modules_clamped_floor_has_caught_up` (a fake module that moves
10 % a post: no fill at 1.08 against a 0.93 reading, filled once within 5 %).

### DR-M-2: a flood of free launches hides a queued graduation from the fill hunt

The hunt walked forward from a cursor stuck behind Trading coins, 8 a tick: 10,000 spam coins took longer than the curve's window.

**Fix.** The hunt reads the factory's 8 **newest** coins, plus the operator's hand-ins (`eth_fill(collection, coin)`, the coin of
the module's `GradQueued` event; operator-only, not while paused or mid-tick, at most 8 held). Nothing in a hand-in is trusted but
which coin: the factory's `routerOf`, the module's `isFilled` and the curve's `state` are read before anything is signed. A try is
at most every 30 minutes, and the monitor pages a graduation left queued for an hour (`eth-grad-queued`).

**Test:** `dr_m_2_a_flood_of_newer_coins_hides_a_graduation_from_the_hunt_and_the_operators_hand_in_finds_it`.

### DR-M-3: the slow cadence left little margin under the module's `FLOOR_TTL`

Evidence handed in before a pass was due was dropped; with 20-minute hand-ins and a 30-minute pass floor, one or two late hand-ins
pushed the next post past the one-hour TTL, and a stale module refuses burns for a collection that has holders.

**Fix.** An ACTIVE pass is due whenever the module's last post (`Life.posted_at`: our mined post, or `postedAt()` read at a pass) is
within `TTL_MARGIN_SECS` = 25 min of its TTL, whatever the cadence says. Not-due evidence is kept (until its TTL or a newer hand-in).
The monitor also pages a wake queued over 6 h (`eth-wake-queued`).

**Test:** unit `the_cadence_slows_backs_off_and_speeds_up_when_hot` (the margin), and the fake-world cadence tests.

### DR-M-4: the Launch page failed open

With `VITE_COLLECTIONS_URL` unset or `/collections` down or wrong (an uncertified relay), the page said "Awake" and offered an
item-buying launch on a dormant module. The wake button ignored an open request (a revert), and nothing said how the bond comes back.

**Fix.** With a desk in the stack file, an item-buying launch waits unless the page's own chain read of `FloorModule.fresh()` is
true; the relay's lives only say how to wait. The button reads `WakeDesk.openOf(wrapper)`. The reclaim and refund terms are shown
beside it.

**Test:** web-rook `DR-M-4: under the dormancy the launch fails closed on the chain's own freshness read, whatever GET /collections says`.

---

## Low

- **DR-L-1: a desk request lost while its collection's life was undecided.** The cursor advanced before `request` failed. **Fixed:**
  the cursor stops there and the request is read again next tick.
- **DR-L-2: a request closed by `reclaim` before the canister read it gives no wake.** **Mitigated, not changed:** after DR-H-1 a
  reclaim proves a graduation, which raises `gradEth` and so an owed wake by the watchdog within 6 h; the curve's window is 3 days.
- **DR-L-3: a post that mines after its wake timed out leaves `ever_active` false**, so the watchdog never reads the module that
  now takes launches. **Fixed:** any mined post sets `ever_active` (and `posted_at`).
- **DR-L-4: the drift check passed silently without a recorded floor.** **Fixed:** a fill with no `buy_floor` for the buy is
  refused (the listing dropped; the advance's existing way back applies).
- **DR-L-5: one coin reclaimed several bonds.** **Fixed:** `spent(coin)`; one launch pays back one bond. Forge
  `test_reclaim_refusesTheSameCoinForTwoRequests` (a request in its grace and a newer one, both after the coin).

## Checked and sound (from the review)

- **Legacy mode** (`dormancy = null`): every collection `Always`; no dormancy pass, gate, fresh-read or hunt; behaviour unchanged.
- **Manipulation during WAKING.** No post until the whole 7-day window is read (no hole, lost span, raw range or skipped block, at
  most 8 receipts backed off) and the reading is confirmed by the counted sales (`min_sales`, `held == None`); the first post is
  still bounded by the module; no buy or deposit mint until ACTIVE; and after DR-M-1 no fill or buy until the module agrees with the
  reading. Wash sales inside the window face the same sale judges as before (the canister reads the sales itself).
- **The stale floor at buy time** (`fresh_for_fill`): reading ≤ 900 s old from a pass that ran the scan ≤ 300 final blocks behind,
  confirmed, within 5 % of the module's accepted floor and within 10 % of the advance's floor; fails closed for a collection with
  no life.
- **Re-dormancy safety:** conservative; no path puts holders under a stale module.
- **Invariants:** new reads go through `home_read` at the tick's pin after `home_floor_ok`; no new outcall kind or transform; the
  byte budgets bind WAKING; H3 untouched; the staged caps name a binding global (48 items / 75 ETH at step 1); the dormancy and
  the desk are in the install argument (timelocked); `eth_wake` and `eth_fill` are operator-only.
- **`WakeDesk`:** `nonReentrant`, checks-effects-interactions, `closed` before every transfer; exact bond; both clocks.

## Residual risks

1. **Wake denial at the bond's price.** An attacker can fill `wakes_per_day` with four bonded requests a day on collections nobody
   wants (0.012 ETH a day, forfeited once served). Honest requesters wait up to a day; owed wakes still use the reserve. Raise the
   bond or `wakes_per_day` if it is seen (timelocked).
2. **Slots held by real positions.** Collections with holders stay ACTIVE (correctly). Three of them use the whole budget; see
   DR-H-1's residual.
3. **Unservable wakes are free to the requester** (DR-H-3's residual): bounded by the caps, paged.
4. **The owed reserve can make `max_active + 1` awake for a while.** golive-check plans cycles at `max_active`; a day at + 1 is
   ≈ 2 T more, inside the monitor's 7-day page threshold.

---

## Scaling (2026-09-29): the limits auto-scale from the cycles balance

Scope: the approved change of `docs/v7/dormant-collections.md` §11 (`max_active` and `wakes_per_day` become hard ceilings; under them
the canister's own liquid cycles balance decides how many collections may be awake and whether a wake starts), in the canister
(`dormancy.rs`, `ethflow.rs`, `ethnet.rs`, the `.did`), the monitor, the runner, the site, the launch file, golive-check and the dry run.
One adversarial pass over the author's own diff, then fixes on the same branch. The questions: can a scaling decision be gamed (a
top-up then a drain, a burst, an unread balance), can it overflow or underflow, can scaling down strand anything, and is the launch
number honest.

### Counts

| | Found | Fixed | Accepted |
|---|---|---|---|
| Medium | 2 | 2 | 0 |
| Low | 3 | 3 | 0 |

### Medium

**S-1: owed wakes could wake every collection on a balance that pays for one.** `start_wakes` let an owed wake (a deposit, a queued
graduation) start with `OWED_RESERVE` above the *ceiling* (`max_active` 10 + 1), gated only by the reserve staying whole. Holding one
NFT of each collection (not spending it: a valid deposit mints wX back) and sending them to the canister would have woken all ten at
≈ 25 T a day on a balance that keeps one, draining the runways into the reserve within a day. **Fixed:** an owed wake has one slot
above `effective_max_active`, as it had one above the fixed `max_active` before; it may still use the runways, never the reserve.
**Test:** `dormancy::tests::scaling_positions_pin_collections_awake_above_the_limit_and_no_new_wake_starts` (three awake, the balance
keeps one: the owed wake waits; one awake, the balance keeps one: one owed wake starts, not two).

**S-2: the requested launch balance (18–20 T) was under the canister's own scan floor plus one collection's runway.** The request
sized the start at one collection's 7 days (17.5 T) and a small reserve. Under `EVIDENCE_CYCLES_FLOOR` (8 T, round 4) the canister
stops its own sale scan: no wake can finish its backfill and no buy passes the fresh read (§4.3). At 20 T one awake collection
reaches that floor in about 4 days, not 7. **Fixed:** the reserve is validated at or above the scan floor (`reserve_b ≥ 8,000`), one
collection's exits have their own 3 T, and the launch starts at **34 T** (the first wake needs 32.1 T liquid; 1 T is the freezing
threshold's share), about US$46. golive-check FAILs a start that does not keep one collection awake with its wake; the dry run holds it
to a 20 T file. Lowering the scan floor would be a separate, reviewed change.

### Low

- **S-3: an unread balance put collections to sleep.** With `scaling` set and the balance unknown, `capacity` reports 0, so every idle
  awake collection counted as "over" and slept at its check. Never on the IC (the balance is a system call), but a decision on a number
  that was not read. **Fixed:** scaling down acts only on a balance that was read; no wake starts without one either. **Test:**
  `scaling_review_s_3_a_balance_that_cannot_be_read_puts_nothing_to_sleep_and_wakes_nothing`.
- **S-4: the monitor's balance page at 20 T would have paged within five days of a 34 T start, for no reason of its own.** **Fixed:**
  `min_balance_t` is 12 (the 8 T reserve plus one collection's exit reserve); the capacity page (`eth-capacity-short`) is the one that
  says "top up for the collections with positions".
- **S-5: the Launch page showed no capacity when `VITE_COLLECTIONS_URL` is a same-origin path** (`new URL` refuses a relative URL).
  **Fixed:** a same-origin path maps to its `/capacity` sibling; a protocol-relative URL is refused. **Test:** web-rook `§11: the
  capacity from GET /capacity …`.

### Checked and sound

- **The balance.** `canister_liquid_cycle_balance128`: the freezing threshold's share is excluded, so the scaling never counts cycles
  that keep the canister from freezing. It is read once a tick and passed to every decision of that tick.
- **A top-up then a drain.** No method pays cycles to a caller and none accepts cycles from calls; the IC lets nobody withdraw a
  canister's cycles. The balance rises only by a top-up (which legitimately pays for what it lets wake) and falls only by the canister's
  own spending, bounded as before. A collection woken on a top-up that later holds positions stays awake on the reserve and pages; one
  that holds nothing sleeps at its next check.
- **Double counting.** A backfill under way has not paid its cost yet; `available = balance − waking × wake` counts it unpaid, so a wake
  started this tick or a few ticks ago is never counted twice as headroom (`scaling_a_top_up_raises_the_limit_and_the_wakes_already_under_way_are_counted`).
- **Overflow and underflow.** B-cycle `u64` figures become `u128` cycles; every sum and product saturates, every difference is guarded,
  and the only division is by `slot` (≥ 3.5 T at the lower bounds). Tested at `u128::MAX`, at 0, with more awake than the ceiling, and
  at the largest configuration the bounds allow.
- **Scaling down never strands.** A collection with a position (on 4663 at its check, or open in the canister) is never put to sleep by
  the scaling; it stays awake above the limit and counts against it, and no wake that is not owed starts while the limit is exceeded
  (`scaling_positions_pin_collections_awake_whatever_the_balance_and_no_wake_starts_while_the_limit_is_exceeded`). Only a collection
  with nothing open is put to sleep early (`scaling_down_sleeps_an_idle_collection_at_its_check_but_never_one_with_a_position`).
- **Thresholds.** A wake starts at exactly `wake + need(awake + 1)` and not one cycle below, in the pure rule and in the fake world;
  the headroom agrees with the per-wake rule at every balance and awake count tried.
- **The anti-griefing of DR-H-1 / H-2 / H-3 is unchanged:** the bond, the graduated-coin reclaim, the six-hour sleep of an unused wake,
  the owed back-off and return, the refund of an unserved wake (a wake queued behind the balance past the window is refunded), the
  cooldown and `max_waking`.
- **Governance.** The ceilings and the scaling's figures are in the install argument (timelocked); golive-check holds them to the
  launch file (a canister with fixed caps, a lower burn estimate or another ceiling FAILs in the dry run); the monitor's argument diff
  words a lower burn, runway or reserve as LOOSENS.

### Residual risks

1. **The runway is priced at the configured burn, not a measured one.** A measured rate would read one-time wake costs as a daily burn
   and could be pushed up by a burst, so the canister uses the high end of the estimate (2.5 T a day). The bounded worst case (the
   scan's day budget, ≈ 10.5 T a day per awake collection) would shorten the 7 days to about 2; the monitor's burn-rate page is the guard,
   as before.
2. **Griefing spends headroom, not runways.** A served desk wake nobody uses costs ≈ 2.1 T (the backfill and six hours awake) for a
   forfeited 0.003 ETH bond; the 24 h cooldown bounds it to about one a day per collection, and each needs the balance after its cost to
   keep every awake collection's runway and the reserve. So a griefer can spend the part of a top-up above what the awake collections
   need, at more than the cycles are worth, delaying honest wakes until the next top-up. Raise the bond (timelocked) if it is seen.
3. **At the 34 T start the limit falls to 0 within about a day of the first wake.** From then no new wake starts until a top-up, and an
   awake collection with positions pages `eth-capacity-short`. That is the design (capacity follows funding), but the operator should
   expect to top up soon after the first real use.
4. **A deposit of a collection that cannot wake waits in the queue** (paged at 6 h as `eth-wake-queued`) until a top-up or a slot frees;
   DR-H-2's return path applies to timed-out wakes, not to queued ones.
5. **The exit reserve (3 T per awake collection) is an estimate** (two days of posts without the scan and the binding global's
   deliveries); with the 8 T reserve it covers the wind-down of up to about 11 awake collections. `burn_warn_t_per_day` (10) is static:
   raise it as top-ups raise the limit (runbook §6).

### Tests added

Canister 748 → 759 (7 unit in `dormancy.rs`, 4 in `tests/dormant.rs`); monitor 246 → 248; ops 73 → 74; web-rook 93 → 94; the dry run
adds four refusals (fixed caps, a lower burn estimate, another ceiling, a 20 T start) and holds the launch file's start to
`effective_max_active ≥ 1` with the first wake.
