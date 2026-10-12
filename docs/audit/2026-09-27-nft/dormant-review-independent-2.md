# Independent security review 2: dormant, wake-on-demand collections (branch `dormant-collections`)

Date 2026-09-29. Scope: the whole branch, `git diff 9efed1b..c2663ce`. That covers `WakeDesk.sol`, `dormancy.rs`, the dormancy hooks
in `ethflow.rs` / `ethnet.rs` / `ethnft.rs`, the runner, the monitor, the site (`Launch.tsx`, `dormancy.ts`, `phase.ts`, `abis.ts`,
the Dockerfile), `DeployStack`, golive-check, the dry run and the launch file. The round-1 fixes (`dormant-fixes-round1.md`:
`4ccb4d6`, `4a4a23c`, `6ebf385`) were re-checked against the code and their regression tests, not taken as read.

This is a report only. Nothing on the branch was changed except this file, which is left uncommitted. Every proof of concept ran in
a throwaway copy outside the repository:
- the canister's fake world: a copy of `autobuyer/canister-rs` with PoCs appended to `tests/dormant.rs`, built in its own target
  directory;
- forge: a copy of `src/`, `lib/`, `test/` with `test/wake/PocR2.t.sol` inheriting `WakeDeskTest`.

Nothing was deployed, pushed or signed.

**Threat model:** the same as `dormant-review.md` and independent review 1.
- The attacker is an outsider who can request wakes with the bond, launch coins, send NFTs to the canister, wash trade, self-list,
  call any permissionless function (`markServed`, `forfeit`, `reclaim`), and time any of it.
- The runner may lag or fail. Ethereum and 4663 providers may fail transiently. The `GET /collections` relay is uncertified.
- The canister key, the operator and a timelocked upgrade are trusted.

**Severity scale:** as `README.md`.

## Suites (head `c2663ce`)

| Suite | Result |
|---|---|
| `forge test` | 1,162 passed, 0 failed (133 suites) |
| `cargo test` (autobuyer/canister-rs) | 771 passed, 0 failed |
| monitor `npm test` | 249 / 249 |
| ops `npm test`, `tsc --noEmit` | 74 / 74; tsc clean |
| web-rook tests (`tsx --test`, local binary), `tsc --noEmit` | 97 / 97; tsc clean |
| `DRYRUN_NO_DRAND=1 scripts/golive-dryrun.sh stack` | DRY RUN: ALL PASS (exit 0, 136 `ok` lines) |

## Counts

| Critical | High | Medium | Low | Info |
|---|---|---|---|---|
| 0 | 0 | 0 | 3 | 2 |

No Critical, High or Medium finding. Every round-1 fix closes its proof of concept. The three Low findings are edges of the new
"served" accounting and of the canister's retry rules.

---

## Low

### D2-L-1: `markServed` records a post made AFTER the window, during the block clock's tail, so `forfeit` charges a requester whose wake was not served inside `[at, at + window]`

**Where:**
- `src/floor/WakeDesk.sol:188`: `if (!known || p < r.at || (p > r.at + window && _elapsed(r, window))) revert NotServed();`
- Compare `src/floor/WakeDesk.sol:218`, where the unmarked rule is `p >= r.at && p <= r.at + window`, on the timestamp alone.

**The bug.** `_elapsed` needs both clocks. The parent-block half of the window is `window / parentBlockSeconds` blocks. 4663 deploys
with 12, rounded down from the measured 12.06 to 12.14 s (`Wait.sol`), so the block half ends about 1 h 10 min to 1 h 25 min after
the timestamp half on a 7-day window. It ends later still if the parent chain slows or stalls. During that tail:
- `p > r.at + window` holds: the post is outside the window by the desk's own definition, and the unmarked `_served` says "not
  served";
- `_elapsed(r, window)` is false, so `markServed` accepts the post anyway.

Anyone can then record it (`servedAt = p`). After the grace, `forfeit` sends the bond to the treasury. The round-1 fix promised
that "a late post can never charge the requester" (`dormant-fixes-round1.md`, DI-M-1).

The canister itself never does this: `mined_home` records `served` only while `now < a.until`, at `ethflow.rs:4362`. A third
party, a keeper bot or a griefer, does. The post can come from any wake: owed, the operator's, or one under way for another
reason.

**PoC** (forge, passes): `test_poc_d2_markServedInTheBlockTail_chargesAPostOutsideTheWindow` in the appendix.
1. Alice requests.
2. Wall time advances `window + 20 min` at 12.1 s per parent block.
3. The canister posts. `postedAt` is later than `at + window`, and `served(id)` is false.
4. A stranger calls `markServed(id)`, which succeeds.
5. After the grace, `forfeit` pays the treasury `BOND`.

**Impact.** A bond of 0.003 ETH is charged against the stated rule. The requester can still reclaim during the grace if a coin of
theirs has graduated. Bounded and narrow.

**Fix.** Use one bound in both places: `if (!known || p < r.at || p > r.at + window) revert NotServed();`. A post later than
`at + window` is never inside the window, whatever the block clock says. Add the PoC as a regression test.

### D2-L-2: one transient estimate failure is read as "would not succeed". The canister gives up `markServed` for good, and drops the operator's fill hand-in

**Where:**
- `ethnet.rs:275`: any `eth_estimateGas` error becomes `"would not succeed now: {e}"`. That includes an outcall timeout, a rate
  limit or a provider disagreement (`canister.rs:1380`, `rpc_max`).
- `ethflow.rs:5703-5706` (`mark_act`): `Err(e) if e.contains("would not succeed") => { z.marked = true; … }`. The ask is never tried
  again.
- `ethflow.rs:5639` then `5644` (`fill_one`): the coin is passed over, and the hand-in is removed from `Dorm.fills` whatever the
  reason.

**The bug.** The round-1 notes say "a revert or a take-back is retried only while the window lasts, and only if the estimate says it
can still succeed". The string match cannot tell a real revert (`Closed`, `NotServed`) from a provider that did not answer.

**Failure.**
- **`markServed`.** One provider hiccup on the estimate sets `marked = true`, days before the window ends. Nothing is recorded on
  the desk. An awake collection keeps posting past the window, so at `forfeit` its latest post lies outside the window and the bond
  goes back. The treasury loses the bond for a wake that was served, and the requester got it free. This is the "mark's margin"
  residual in `dormant-fixes-round1.md`, but reached through a transient error, not through the canister failing for six hours.
- **The fill hand-in.** DR-M-2's only path to an older graduation (one not among the newest eight coins) is the operator's
  `eth_fill`. One transient estimate failure on `fill(coin)` drops the hand-in. The hunt never finds that coin again, so the
  graduation stays queued until:
  - the monitor's `eth-grad-queued` page (1 h) makes the operator hand it in again, or
  - the curve's 3-day window opens refunds.

**PoC** (fake world, both pass; a `flaky_mark` / `flaky_fill` counter injects one `"would not succeed now: eth_estimateGas: HTTP
outcall timed out"`):
- `poc_d2_one_transient_estimate_failure_abandons_mark_served_for_good`. Milady is woken by Alice's request, and the post serves it.
  One estimate failure follows. Over the next three hours no `markServed` is ever sent, `desk_served` stays empty, and the ask is
  `marked`, with more than 5 days of the window left.
- `poc_d2_one_transient_estimate_failure_drops_the_operators_fill_hand_in`. Milady is ACTIVE with a Graduating coin behind 20 newer
  coins. The operator hands it in, and one estimate failure follows. Twelve hours later `Dorm.fills` is empty, the coin is not
  filled, and no `fill` was ever sent. The control, `dr_m_2_…_the_operators_hand_in_finds_it`, fills it within three ticks.

**Fix.**
- Have `home_sign` return a typed error that separates "the node says it reverts" (a JSON-RPC execution error) from a transport or
  consensus failure. Treat only the first as "would not succeed".
- Or, in `mark_act`, set `marked` only after `servedAt` or `requestAt(id).closed` has been read to confirm it. In `fill_one`, drop
  a hand-in only after reading `isFilled`, or a curve state other than Graduating.

### D2-L-3: a request the canister drops as "its bond goes back unserved" (and the site says so) is then recorded as served by the canister itself when another wake posts inside the window

**Where:**
- `dormancy.rs:620-625` (`desk_upkeep`) and `dormancy.rs:580` (`desk_ask`): the queued desk wake is dropped with the note "its bond
  goes back unserved". The `DeskAsk` stays on the life.
- `ethflow.rs:4362-4363` (`mined_home`, `Step::Post`): any post in `[a.at, a.until)` sets `a.served` whatever happened to the ask.
  `mark_act` then sends `markServed`.
- `web-rook/src/rook/dormancy.ts:64-66` (`openText`): the page tells the requester "That bond goes back to its requester unserved
  (WakeDesk.forfeit)".

**The bug.** The desk's rule is "a post inside the window serves the request", whoever caused the post. The canister's log and the
site both promise a refund once the canister gives up on the desk wake. Within the last `wake_timeout + 6 h` (18 h at launch) of
the window, the collection can still be posted by:
- an owed wake (a deposit, a graduation);
- the operator's `eth_wake`;
- a timed-out wake whose post mines late.

The canister then records the request as served, and the requester is charged against the promise they were shown.

**PoC** (fake world, passes): `poc_d2_a_request_dropped_as_unserved_is_then_marked_served_by_the_canister`.
1. At 20 T, Alice's request queues.
2. At `at + window − 18 h` the canister drops it with "its bond goes back unserved", and Milady is DORMANT.
3. A top-up follows, then the operator's wake.
4. Milady posts inside the window, and the canister sends `markServed(0)`, which the desk records.

**Impact.** 0.003 ETH per request, charged contrary to the canister's own log and the site's text. The requester did get a wake
inside the window, which is why this is Low.

**Fix.** Pick one rule and state it everywhere:
- Either keep the contract's rule and reword the drop note and `openText`: "it is refunded unless the collection is posted
  inside the window by another wake".
- Or mark a dropped ask `served_forfeit = false` (a `dropped` flag on `DeskAsk`) so the canister never marks it. Note that
  `markServed` stays permissionless, so only the wording fix fully aligns the promise with the chain.

---

## Info

- **D2-I-1: the no-lives fallback (DI-L-1) offers the wake whenever `fresh()` reads false, including an ACTIVE collection's brief
  staleness.**
  - Where: `web-rook/src/rook/dormancy.ts:81-85`. With no lives served (no `VITE_COLLECTIONS_URL`, or the relay down),
    `wakeView(null, false, { dormancy: true, fresh: false })` returns `canWake: true` with "it is probably dormant".
  - An ACTIVE collection's module can be stale for minutes, for example while a post is pending past the 1 h TTL or a hand-in was
    late. The desk takes the request, the canister holds it as "served by its next post", and the requester is charged 0.003 ETH
    for a routine post. Checked with `tsx` against `dormancy.ts`: `wakeView(null, false, { dormancy: true, fresh: false })` gives
    `canWake: true`, "Not fresh … it is probably dormant", while the same collection with its life served as `active` gives
    `canWake: false`.
  - With lives served the page says "Awake" and offers nothing.
  - Suggestion: without lives, offer the wake only after `fresh()` has read false for longer than one post cadence (say 2 h since
    `postedAt()`), or say plainly that the bond may pay for a routine post.
- **D2-I-2: a new desk request inherits a closed request's queue position.**
  - Where: `dormancy.rs:585-590` (`desk_ask`, DORMANT): `Some((_, at)) => (by.clone(), at)` keeps the queued time of the desk wake
    already queued.
  - A request #0 reclaimed on chain (closed) and not yet re-read leaves `Desk{#0}` queued. `desk_recheck` reads only the wakes that
    would start this tick, and none starts on a low balance.
  - A request #2 for the same collection, read next, takes #0's queued time. `desk_upkeep` then keeps it, because `l.desk.id` is
    #2's. So #2 sorts ahead of another collection's request that was queued before it.
  - PoC (fake world, passes): `poc_d2_a_new_desk_request_inherits_a_closed_requests_queue_time`, with Milady's #2 at position 1
    ahead of Doodles' older #1.
  - This is only queue fairness, and it needs a graduated coin for the reclaim.
  - Suggestion: queue a new desk request at its own read time (or `ask.at`), not the old queued time.

---

## Round-1 fixes, re-checked

| ID | Holds? | Notes |
|---|---|---|
| DI-H-1 | Yes | Step 3 asks for the owed wake whenever the life is DORMANT and its queued wake is not owed (`ethflow.rs:5369`), and `Dorm::request` upgrades a desk or operator wake, keeping its `at`. The watchdog's filter admits a non-owed queued wake (`ethflow.rs:5387`). The DR-H-2 return clears only a queued `Deposit{tx}` of that deposit, and then `desk_upkeep` re-queues the desk ask only if no wake was tried for it. Regression tests pass. No new stranding found. |
| DI-M-1 | Yes, with D2-L-1 and D2-L-3 | The canister reads `window()` once, holds `until = at + window`, refuses to queue or start a desk wake without `wake_timeout + 6 h` left (`startable`), drops it past that in `desk_upkeep` and again in `start_wakes`, and re-reads `requestAt` for every desk wake about to start (`desk_recheck` loops until the set that would start is fully checked, holding back any whose read failed). A closed request is dropped. The desk counts a post as serving only inside the window, except in the block tail (D2-L-1). The reviewer's PoC (`test_diM1_aPostAfterTheWindow…`) and the fake-world PoC pass as regressions. |
| DI-M-2 | Yes | `request` reverts `Awake()` while `fresh()`. A request read while WAKING is `tried` and served by the first post. One read while ACTIVE (stale module) is served by the next post, or re-queued once if the collection sleeps first. The "one open per wrapper" slot can still be held for a whole window by a request the canister cannot serve (low balance). That was accepted in round 1 ("the one-open slot's tail"), and the requester gets the refund, so it is free only when no wake could have run anyway. |
| DI-M-3 | Yes | `fill_act` collects each collection's wait, pushes its `fill_hunt` by `FILL_RETRY_SECS` and goes on. Only a home-line failure (not "would not succeed") ends the act, and `mark_act` still runs after a wait. The PoC and its control pass. |
| DI-L-1 | Yes | `ARG VITE_COLLECTIONS_URL=` and the `ENV` line are in the Dockerfile. The no-lives path offers the wake from the chain (see D2-I-1 for its edge). |
| DI-L-2 | Yes | `forfeit`'s failed stipend push and `reclaim`'s failed push credit `owed`. `withdraw(to)` zeroes before its call, is `nonReentrant`, and pays the caller's own credit only. The `Reenterer` and `HeavyWallet` tests pass. A requester that bombs `reclaim`'s full-gas push only griefs its own reclaim's caller. |
| DI-I-1 | Yes | `pnpm-lock.yaml` and `pnpm-workspace.yaml` are gone. The Dockerfile and `package.json` use npm only. |

## The new surface, checked and sound

- **`markServed` and window accounting.**
  - `nonReentrant`, and idempotent (`servedAt != 0` returns without reverting).
  - A post before `at` is refused, so an early `markServed` cannot pin a pre-request post.
  - A third party marking first does not make the canister's `Step::Served` revert (the early return), and the canister reads
    `servedAt` before signing.
  - Marking can only charge a requester for a real post by the module's keeper. The one exception is the block tail (D2-L-1).
- **`Step::Served` and its nonce path.**
  - It goes through `send_home`: the pause gate (not allowed while paused, so a pause only costs the treasury), the nonce floor
    (C4-H-1), and saved bytes.
  - `home_effect` reads `servedAt(id) != 0`. It is used only once the nonce is seen used, so a mark by someone else can never raise
    the nonce floor above the chain's nonce.
  - `step_of` attributes it to its collection, so a collection with a mark out is not put to sleep.
- **Request expiry and queue drops.**
  - A request read after `at + window` asks for nothing. A closed request is skipped at read and at `desk_recheck`.
  - A timed-out desk wake is not re-queued (`tried`). A request can hold at most one open slot per collection per window. Wakes
    are bounded by `wakes_per_day`, the cooldown and the scaled limit.
- **`Awake()` and `fresh()` races.**
  - The refusal is on chain, so the page's own `fresh` read only decides what it offers.
  - A collection asleep with a still-fresh module (up to 1 h after its sleep) refuses requests, and launches are open then. That is
    the DR-H-1 design, and its graduation wakes it owed.
  - `fresh()` also requires confidence at or above `minConfidenceBps`, and the canister's held posts are 5,000 against a floor of
    3,000, so a canister post always makes the module fresh.
- **Owed / withdraw pull payment.**
  - `owed` is keyed by the requester, and credits are only ever added by the desk itself.
  - `withdraw` pays `msg.sender`'s credit to any `to`, with state cleared first.
  - The treasury is immutable. A treasury that refuses ETH blocks served forfeits (unchanged from round 1; the treasury is the
    stack's `ProtocolTreasury`).
- **The fill-retry loop.** Bounded per collection by `FILL_RETRY_SECS`, eight coins and at most `FILLS_MAX` hand-ins. Its only
  weakness is D2-L-2's transient-error drop.
- **The upgrade path from a queued desk wake to owed.**
  - Deposit, graduation and operator requests keep an owed wake.
  - An owed wake upgrades a desk or operator wake, keeping its time.
  - The desk ask stays on the life and is `tried` when the owed wake starts, so it is decided by that wake's post.
  - A desk ask read behind a queued owed or operator wake leaves that wake in place.
- **Floor manipulation during WAKING and thin volume.**
  - No change from round 1: no post until the whole window is read and the reading is confirmed. The first post is clamped by the
    module, and DR-M-1 holds fills and buys until `converged`.
  - A held reading blocks the backfill, so a wash campaign can only make a wake time out, which is paged and whose desk bond is
    refunded.
- **Stale floor at buy time.** `fresh_for_fill` (900 s, scanned at most 300 blocks behind, confirmed, within 5 % of `accepted` and
  10 % of the advance's floor) runs right before the fill signature, under the dormancy. The buy start is gated on `live()` and
  `converged`.
- **Griefing and cycles drain.**
  - A desk request costs the bond, charged when served. Unserved wakes (refunded) are bounded by one per collection per window,
    `wakes_per_day` and the cooldown.
  - Pinning collections awake needs a real position (wX, items, rentals, graduation ETH, float), which is the designed behaviour.
    It is paged as `eth-capacity-short`, and while the limit is exceeded no wake that is not owed starts.
- **Stranded positions on scale-down.** `check_life` puts a collection to sleep only with nothing open on 4663 (the seven reads)
  and nothing in the canister (`canister_open`: a buy, the leg, a delivery, a deposit waiting, a pending home step of it, the
  Ethereum line, a leftover).
- **Balance-read failures.** With scaling set, a `None` balance starts no wake and puts nothing to sleep (`over` needs a read
  balance).
- **Overflow.** Every `Scaling` figure saturates, the `u64` and `u128` casts in the desk are safe for real times and blocks, and
  `until` saturates.
- **Canister/chain races.**
  - A post mined before the request (by chain time) is caught by `mark_act`'s `postedAt < a.at` re-read.
  - A sequencer skew of up to 1 h is inside the 6 h margin.
  - `dorm_pass` does not run while paused. Requests read after the pause past their window ask for nothing and are refunded.
- **Launch-page trust in unverified data.**
  - The launch fails closed on the page's own `fresh()` read.
  - The wake is gated on the on-chain `openOf`. `requestRefusal` holds the wake call to one of the stack's wrappers and exactly the
    stack file's bond.
  - A lying relay can only hide the offer or show wrong wording (DR-M-4).

## Invariants

- **Pin-floor monotonic reads.** `home_floor_ok` runs before `dorm_pass` at every tick. Every new read (`window()`, `requestAt`,
  `servedAt`, `gradEth`, the positions, the fill hunt's reads) goes through `home_read` at the tick's pin.
- **`max_response_bytes` before the transform.** There is no new HTTPS outcall kind or transform in the diff. `eth_estimateGas` for
  `markServed` and `fill` uses the existing `rpc_max`.
- **Nonce handling.** `Step::Fill` and `Step::Served` use `send_home` and `resolve_home`, with their effects read back
  (`isFilled`, `servedAt`), so there is one home transaction in flight.
- **H3, 25 ETH a day.**
  - Untouched: `global_value_wei` is 25 ETH at every stage.
  - `fill` and `markServed` move no canister value.
  - A buy's advance still passes `outflow_check(Flow::Value)`.
- **Staged caps.**
  - The binding global of 48 items at steps 1-2 and 61 at step 3 (75 ETH) is held by `staged.mjs` (`binding`) and golive-check
    (`same … globalItems`).
  - The exit rule is on `min(sum, global)`: 48 at 24 a day and 61 at 31 a day both leave in 2 days.
  - Per collection, the largest is 24 at 12 a day: 2 days.
- **Exit rule vs the 72 h timelock.** Two usable days (72 h less a day) cover each step, as above. `held_fits` at `post_upgrade` is
  unchanged.
- **Governance only via the timelock.**
  - The dormancy, its scaling and the desk address are in the install argument. `argdiff.mjs` flags every loosening.
  - `WakeDesk` has no owner, setter, pause or upgrade.
  - `eth_wake` and `eth_fill` are operator-only and refused while paused or mid-tick (the 12-update test).
- **No code at the canister's addresses.** The canister only calls `markServed` and `fill`, and deploys nothing. `DeployStack`
  deploys the desk from the deployer.

## Appendix: PoC sources (run in a throwaway copy)

Forge, `test/wake/PocR2.t.sol`:

```solidity
contract PocR2 is WakeDeskTest {
    function test_poc_d2_markServedInTheBlockTail_chargesAPostOutsideTheWindow() public {
        vm.prank(alice);
        uint256 id = desk.request{value: BOND}(address(milady.w));
        uint64 at = uint64(block.timestamp);
        uint256 t = WINDOW + 20 minutes;              // wall time, at 12.1 s a parent block
        vm.warp(block.timestamp + t);
        vm.roll(block.number + (t * 10) / 121);
        _post(milady, 0.9 ether);                     // outside [at, at + window]
        assertGt(milady.m.postedAt(), at + WINDOW);
        assertFalse(desk.served(id));
        vm.prank(stranger);
        desk.markServed(id);                          // accepted: the block clock's window is still open
        assertGt(desk.servedAt(id), at + WINDOW);
        _skip(2 days);
        uint256 t0 = protocol.balance;
        desk.forfeit(id);
        assertEq(protocol.balance - t0, BOND);        // charged for a post outside the window
    }
}
```

Canister. These are appended to a copy of `tests/dormant.rs`. The copy's `Fake` gains two fields, `flaky_mark: u32` and
`flaky_fill: u32`, and `home_sign` returns `Err("would not succeed now: eth_estimateGas: HTTP outcall timed out")` once for each,
for a call to `DESK` or with the `FILL` selector, before the harness's real revert checks.

```rust
#[test]
fn poc_d2_a_request_dropped_as_unserved_is_then_marked_served_by_the_canister() {
    let mut f = Fake::new(Some(launch(true)));
    f.cycles = Some(20 * T);
    let mut st = EthState::default();
    tick(&mut f, &mut st);
    let at = f.now;
    f.desk.push((f.cols[0].wrapper, ALICE, at, BOND, false));
    next(&mut f, &mut st);
    assert_eq!(kind(&f, &st, MILADY), Kind::Queued);
    let deadline = at + f.desk_window - launch(true).wake_timeout_secs - dm::DESK_START_MARGIN_SECS;
    while f.now + TICK <= deadline { next(&mut f, &mut st); }
    let l = next(&mut f, &mut st);
    assert!(l.iter().any(|x| x.contains("its bond goes back unserved")));
    f.cycles = Some(100 * T);
    ef::put_wake(&mut st, &f.cfg(), MILADY, f.now).unwrap();
    for _ in 0..16 {
        if f.desk_served.contains_key(&0) { break; }
        evidence(&f, &mut st).ok();
        next(&mut f, &mut st);
    }
    assert!(f.now < at + f.desk_window);
    assert!(f.desk_served.contains_key(&0), "the dropped request is recorded served: the bond is charged");
}

#[test]
fn poc_d2_one_transient_estimate_failure_abandons_mark_served_for_good() {
    let mut f = Fake::new(Some(launch(true)));
    let mut st = EthState::default();
    tick(&mut f, &mut st);
    let at = f.now;
    f.flaky_mark = 1;
    wake_milady(&mut f, &mut st);
    for _ in 0..18 { evidence(&f, &mut st).ok(); next(&mut f, &mut st); }
    assert!(f.now + 5 * 86_400 < at + f.desk_window);
    assert_eq!(f.flaky_mark, 0);
    assert!(!f.home_calls.iter().any(|(a, _)| *a == DESK), "markServed never sent again");
    assert!(f.desk_served.is_empty());
    assert!(life(&st, MILADY).desk.as_ref().is_none_or(|a| a.marked));
}

#[test]
fn poc_d2_one_transient_estimate_failure_drops_the_operators_fill_hand_in() {
    let mut f = Fake::new(Some(launch(true)));
    let mut st = EthState::default();
    tick(&mut f, &mut st);
    wake_milady(&mut f, &mut st);
    let (coin, curve) = ([0x71; 20], [0x72; 20]);
    f.c(MILADY).coins.push((coin, curve, 1, false));
    for k in 0..20u8 { f.c(MILADY).coins.push(([0x10 + k; 20], [0x50 + k; 20], 0, false)); }
    f.c(MILADY).grad_eth = 4 * E;
    for _ in 0..3 { evidence(&f, &mut st).ok(); next(&mut f, &mut st); }
    f.flaky_fill = 1;
    ef::put_fill(&mut st, &f.cfg(), MILADY, coin, f.now).unwrap();
    for _ in 0..72 { evidence(&f, &mut st).ok(); next(&mut f, &mut st); }
    assert_eq!(f.flaky_fill, 0);
    assert!(st.dorm.as_ref().unwrap().fills.is_empty(), "the hand-in is gone");
    assert!(!f.cols[0].coins[0].3, "twelve hours later, not filled");
    assert!(!f.home_calls.iter().any(|(_, d)| sel(d) == dm::FILL));
}

#[test]
fn poc_d2_a_new_desk_request_inherits_a_closed_requests_queue_time() {
    let mut f = Fake::new(Some(launch(true)));
    f.cycles = Some(20 * T);
    let mut st = EthState::default();
    tick(&mut f, &mut st);
    let t0 = f.now;
    f.desk.push((f.cols[0].wrapper, MALLORY, t0, BOND, false));
    next(&mut f, &mut st);
    f.desk.push((f.cols[1].wrapper, [0xb0; 20], f.now, BOND, false));
    next(&mut f, &mut st);
    for _ in 0..6 { next(&mut f, &mut st); }
    f.desk[0].4 = true;                                   // #0 reclaimed on chain
    f.desk.push((f.cols[0].wrapper, ALICE, f.now, BOND, false));
    next(&mut f, &mut st);
    let q = life(&st, MILADY).queued.clone().unwrap();
    assert!(matches!(q.0, dm::WakeBy::Desk { id: 2, .. }));
    assert_eq!(q.1, t0 + TICK, "request #2 carries the time request #0 was queued at");
    let d = st.dorm.as_ref().unwrap();
    assert!(d.position(&MILADY).unwrap() < d.position(&DOODLES).unwrap());
}
```

## The dry run

`DRYRUN_NO_DRAND=1 scripts/golive-dryrun.sh stack` ended with **DRY RUN: ALL PASS** (exit 0, 136 `ok` lines), the same count as round 1.
Its `FAIL` lines are refusals the script provokes on purpose. With `DRYRUN_NO_DRAND=1` the draw is rehearsed only as far as its
announcement.
