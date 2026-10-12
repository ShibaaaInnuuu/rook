# Dormant collections: fixes for independent review 1

Date 2026-09-29. Branch `dormant-collections` (local only; nothing pushed, merged or deployed). The review is
`dormant-review-independent-1.md`, committed as-is in `6555554`. Every finding is fixed. Each fix has a regression test that
reproduces the reviewer's proof of concept and passes after the fix.

## Commits

| Commit | What |
|---|---|
| `6555554` | The review, as received |
| `4ccb4d6` | `WakeDesk`: served means posted inside the window (`markServed`), no request while the module is fresh, credited refunds (`owed`, `withdraw`) |
| `4a4a23c` | Canister: owed wakes behind a queued desk wake, desk requests held with their window, the served post recorded (`Step::Served`), per-collection fill waits; the monitor's IDL; the state fixtures re-encoded |
| `6ebf385` | Site: open-request wording from the canister's life, the wake offered from the chain without lives, `VITE_COLLECTIONS_URL` in the Docker build, npm only; the runbook |
| (this commit) | This file, and the design doc's "As built" (§3.1, §3.3, §4.2, §5, §6, §10) |

## Findings

| ID | Fix | Commit | Regression tests |
|---|---|---|---|
| DI-H-1 | Deposit step: asks for the owed wake whenever the life is DORMANT and its queued wake is not already owed (`Dorm::request` upgrades a desk or operator wake, keeping its place). Watchdog: runs on a DORMANT once-active life whose queued wake is not owed. | `4a4a23c` | `di_h_1_control_a_deposit_wakes_owed_on_a_balance_that_keeps_no_desk_wake`, `di_h_1_a_deposit_behind_a_queued_desk_request_wakes_owed_and_is_not_stranded`, `di_h_1_a_queued_desk_request_no_longer_blinds_the_graduation_watchdog` (`tests/dormant.rs`); `desk_requests_queue_within_their_window_and_never_displace_an_owed_or_operator_wake` (`dormancy.rs`) |
| DI-M-1 | Desk: `served` = a post inside `[at, at + window]`, recorded by `markServed(id)` (anyone; checked against the module) or read from the module's latest post. Canister: reads `window()` once; holds each request with its end (`Life.desk`); queues its wake only while `now + wake_timeout + 6 h ≤ at + window`, drops it from the queue past that, never starts it late; re-reads `requestAt(id)` before the wake starts and drops a closed one; records the serving post with `Step::Served` (`send_home`, effect read back from `servedAt`). | `4ccb4d6`, `4a4a23c` | forge: `test_diM1_aPostAfterTheWindow_doesNotChargeARequesterWhoCouldNeverReclaim` (the PoC), `test_diM1_aWakeServedInTheWindow_andRecorded_staysChargedWhateverIsPostedLater`, `test_diM1_theLatestPostInsideTheWindow_countsWithoutARecord_andMayBeRecordedLater`; cargo: `di_m_1_a_desk_request_past_its_window_is_dropped_and_never_served_later` (the PoC), `di_m_1_a_queued_desk_request_closed_on_the_desk_is_read_again_and_dropped_before_its_wake`, `di_m_1_the_post_that_served_a_desk_request_is_recorded_on_the_desk_in_its_window`, `the_dormant_state_with_a_pending_mark_is_what_the_canister_encodes`; monitor: `independent review DI-M-1: a pending markServed ...` |
| DI-M-2 | Desk: `request` reverts `Awake()` while the module is `fresh()`. Canister: a request read while WAKING is served by that wake's first post; one read while ACTIVE (the module stale) is served by the next post, and queues a wake if the collection sleeps first (once). Site: an open request on a dormant collection is worded from its age and the window, no wake is offered while the module is fresh. | `4ccb4d6`, `4a4a23c`, `6ebf385` | forge: `test_diM2_aRequestWhileTheCollectionIsAwake_isRefused_andOneAfterItSleepsIsTaken`; cargo: `di_m_2_a_request_read_while_active_is_kept_and_wakes_the_collection_if_it_sleeps_unserved` (the PoC), `di_m_2_a_request_read_while_waking_is_served_by_the_wakes_first_post`; site: `DI-M-2: an open request on a dormant collection is worded ...` |
| DI-M-3 | `fill_act` tries each due collection (`fill_one`); a collection's wait (not fresh, not converged, a failed read) sets its `fill_hunt` to `now + FILL_RETRY_SECS` and the hunt goes on; only a signature the home line refuses ends it. | `4a4a23c` | `di_m_3_control_doodles_fills_when_no_other_collection_waits`, `di_m_3_one_collection_catching_up_no_longer_blocks_another_collections_fill` (the PoC) |
| DI-L-1 | `ARG VITE_COLLECTIONS_URL=` and the `ENV` line in `web-rook/Dockerfile`; the runbook (`docs/v6/phase1-scope.md`, step 13) names it. With no lives, the dormancy on and `fresh()` read false, the page offers the wake, gated on the `openOf` read. | `6ebf385` | `DI-L-1: with no lives served, a module read not fresh on chain offers the wake, gated on the openOf read` |
| DI-L-2 | `forfeit`: a refund the requester's address does not take within the stipend is credited (`owed`, `Owed`), never paid to the treasury; `reclaim` credits instead of reverting; `withdraw(to)` pays the caller's credit with full gas to any address. | `4ccb4d6` | `test_diL2_aHeavyWalletsUnservedBond_isCredited_andWithdrawn`, `test_aRequesterThatRefusesEth_isCreditedItsReclaim_andWithdrawsElsewhere`, `test_forfeit_refundsAWakeTheCanisterNeverServed_andChargesOneItServed` (its refuser case) |
| DI-I-1 | Checked first: `web-rook/Dockerfile` runs `npm ci` with `package-lock.json`, CI runs `npm ci` / `npm test` / `npm run build` there, and nothing in the repository names pnpm. `pnpm-lock.yaml` and `pnpm-workspace.yaml` are removed. | `6ebf385` | `DI-I-1: one package manager: npm's lockfile, no stray pnpm files` |

Proof that the canister tests reproduce the PoCs: with the DI-H-1 and DI-M-3 changes reverted locally, the three tests
`di_h_1_a_deposit_behind...`, `di_h_1_a_queued_desk_request_no_longer_blinds...` and `di_m_3_one_collection_catching_up...` fail, and
pass with them. The DI-M-1 and DI-M-2 PoCs need the new desk model to compile, so they are held to the PoC's own observables
(`Kind::Queued` then `Kind::Waking` after a top-up in the PoC; `Kind::Dormant` and nothing posted now).

## Choices, and where they differ from the suggested fixes

- **DI-M-1, "served".** The review offered a canister-called marker or a `firstPostedAt`. `markServed` is permissionless and checked
  against the module, so it trusts nobody, and the canister is only its reliable caller. A request is served if a post is recorded
  (or, unrecorded, if the module's latest post is itself inside the window). A late post can never charge the requester, and a
  served wake stays charged whatever is posted later, as long as the mark lands inside the window. The canister starts a desk wake
  only with `wake_timeout + 6 h` of the window left, so the mark has at least six hours after the latest possible first post.
  Considered and rejected: a `firstPostedAt` on `FloorModule`. It would change a reviewed contract for one reader, and it still
  needs a per-request window check.
- **DI-M-1, "expiry the canister honours".** The expiry is the request's own window (`at + window()`, the window read from the desk
  once), with the start deadline above. The review's other suggestion, re-reading every queued desk wake every tick, is narrowed:
  only the desk wakes that would start this tick are read again, which is what matters for closure (a `reclaim`; a `forfeit` can
  only come after the window, by which time the canister has already dropped the request).
- **DI-M-2.** The review said any one of three fixes would do. Two are combined: the desk refuses while the module is fresh, and the
  canister keeps a request it reads while WAKING or ACTIVE (a module can be stale while ACTIVE, for example after missed hand-ins,
  and the desk takes the request then). The one-open-request rule is kept. Dropping it would let one wake charge several bonds, and
  the canister would have to hold several requests per collection.
- **DI-L-1.** Both parts were done: the Dockerfile `ARG`, and the wake offered from the chain when no lives are served.
- **Re-queue once.** A desk request whose wake timed out is not queued again for it. It is refunded as unserved. Otherwise one bond
  could buy a failed wake every cooldown for a week.

## Invariants (re-checked)

- **Pin-floor order.** `home_floor_ok` still runs before `dorm_pass`, and every new read (`window()`, `requestAt`, `servedAt`, the
  module's `floor()`) goes through `home_read` at the tick's pin.
- **`max_response_bytes` before the transform.** There is no new HTTPS outcall kind or transform.
- **Nonces.** `Step::Served` is one home transaction through `send_home` / `resolve_home`: one in flight, the nonce floor, saved
  bytes, and its effect read back (`servedAt(id) != 0`) when no provider shows it. A revert or a take-back is retried only while the
  window lasts, and only if the estimate says it can still succeed.
- **H3 (25 ETH/day)**, the staged caps, and the exit rule vs the 72 h timelock are untouched. The desk holds only bonds and has no
  owner.
- **Governance only via the timelock.** The desk's address is in the install argument (unchanged). `DESK_START_MARGIN_SECS` is a
  constant, and the window is the desk's immutable.
- **No code at the canister's addresses.** The canister calls `markServed` on the desk and deploys nothing.

## Suites

| Suite | Result |
|---|---|
| `forge test` | 1,162 passed, 0 failed (133 suites; `WakeDesk.t.sol` 19, 5 new) |
| `cargo test` (autobuyer/canister-rs) | 771 passed, 0 failed (12 new) |
| monitor `npm test` | 249 / 249 (1 new) |
| ops `npm test`, `tsc --noEmit` | 74 / 74; tsc clean |
| web-rook tests, `tsc --noEmit` (local binaries) | 97 / 97 (3 new); tsc clean; `vite build` clean |
| `DRYRUN_NO_DRAND=1 scripts/golive-dryrun.sh stack` | DRY RUN: ALL PASS (exit 0, 136 `ok` lines) |

## Left as it is

- **The one-open slot's tail.** Once the canister has dropped a request it can no longer serve (the last `wake_timeout + 6 h` of its
  window, about 18 h at launch), the desk still refuses a new request for that collection until the window ends. The site says
  when. A new request in that tail could not be served sooner anyway: the queue is what held the first one.
- **A request made while WAKING** is charged if that wake is served, even though another source started the wake. It asked for a wake
  and got one inside its window. The site offers no wake while a collection is warming up.
- **The mark's margin.** If the canister cannot land `markServed` in the six hours or more between the serving post and the window's
  end, and the collection keeps posting past the window, the bond is refunded instead of charged. The treasury is the side that loses
  here, and only when the canister fails.
- **The monitor's `eth-grad-queued`** still pages awake collections only. With DI-H-1 a graduation behind a queued desk wake now
  wakes owed, and `eth-wake-queued` still pages a queued wake.
- **`wake_timeout_secs` vs the desk's window** is not checked by golive-check. The canister logs, when it first reads `window()`,
  whether the window is above `wake_timeout_secs + 6 h`. At the launch numbers (7 days vs 18 h) it is.
- **`buyer.wasm` b7532c4d** is not installed and was not touched. Nothing was deployed, signed or pushed.
