# Dormant collections: fixes for independent review 2

Date 2026-09-29. Branch `dormant-collections` (local only; nothing pushed, merged or deployed). The review is
`dormant-review-independent-2.md`, committed as-is in `46afc75`. Every finding (3 Low, 2 Info) is fixed. Each fix has a regression test
that reproduces the reviewer's proof of concept (from the report's appendix) and passes after the fix. Each of those tests was also run
against the code before its fix, and failed there.

## Commits

| Commit | What |
|---|---|
| `46afc75` | The review, as received |
| `bea711a` | `WakeDesk`: `markServed` refuses a post later than `at + window` on the timestamp alone (D2-L-1) |
| `4145059` | Canister: `markServed` and fill hand-ins given up only on the chain's word (D2-L-2); a dropped desk request worded as the desk decides it (D2-L-3); a new desk request queued at its own time (D2-I-2) |
| `3a652b3` | Site: no wake offered on a briefly stale module without lives (D2-I-1); the refund worded as the desk decides it (D2-L-3) |
| (this commit) | This file, and the design doc's "As built" (§3.3, §4.2, §5, §10) |

## Findings

| ID | Fix | Commit | Regression tests |
|---|---|---|---|
| D2-L-1 | `markServed`: `if (!known \|\| p < r.at \|\| p > r.at + window) revert NotServed();`. This is the same bound, on the timestamp alone, that the unrecorded `served` reads. A post after `at + window` is never recorded, whatever the block clock says. A post at or before `at + window` can still be recorded after the window, while it is the module's latest post. The fake world's `mark_ok` follows the same rule. | `bea711a` | forge: `test_d2L1_aPostInTheBlockClocksTail_isNotRecordedServed_andTheBondGoesBack` (the PoC, 20 min past the window at 12.1 s a block; it fails before the fix with "next call did not revert"), `test_d2L1_aPostAtTheWindowsEnd_isStillRecordedServedInTheBlockTail` |
| D2-L-2 | `mark_act`: the module's latest post already read past `a.until` gives the ask up without an estimate. On a "would not succeed", `requestAt(id)` is read again. If it is closed, the ask is given up. If it is open (and `servedAt` was just read 0 and the latest post is inside the window), the chain says the mark can be taken, so the failed estimate is a wait, tried again next tick. A failed read is also a wait. `fill_one`: a hand-in whose `fill` was not estimated is kept, because the same pin has just shown it `Graduating` and unfilled. The collection waits `FILL_RETRY_SECS`, and the hand-in is dropped only when the chain shows it filled or its curve no longer `Graduating`. | `4145059` | cargo: `d2_l_2_one_transient_estimate_failure_no_longer_abandons_mark_served` (PoC 1), `d2_l_2_one_transient_estimate_failure_no_longer_drops_the_operators_fill_hand_in` (PoC 2), `d2_l_2_a_mark_is_given_up_only_when_the_chain_shows_the_request_closed`, `d2_l_2_a_kept_hand_in_is_dropped_once_the_curve_is_no_longer_graduating` |
| D2-L-3 | The wording is aligned with the contract. The canister's drop notes (`desk_ask` too late, `desk_upkeep` dropped from the queue) now say "its bond goes back unless the collection is posted inside the window by another wake", and so do the life's note and the `DESK_START_MARGIN_SECS` docs. The site's `openText` says the same through `SERVED_RULE`, and `RECLAIM_NOTE` adds "Any post inside the window serves the request, whatever woke the collection". The canister keeps recording such a post, as the desk would. | `4145059`, `3a652b3` | cargo: `d2_l_3_a_dropped_request_is_worded_as_the_desk_decides_it_and_a_post_in_its_window_still_serves_it` (the PoC; it fails before the fix on the old "goes back unserved"); site: `D2-L-3: the page words a dropped request's refund as the desk decides it ...`, and the DI-M-2 site test's wording assertion |
| D2-I-1 | Without lives, the page reads `FloorModule.postedAt()` too (`FLOOR_MODULE_FRESH`, `Launch.tsx`). It offers the wake only when the module was never posted, or was last posted at least `NO_LIVES_OFFER_AFTER_S` (2 h) ago. An ACTIVE collection posts well inside the 1 h TTL, with its pass due 25 minutes before it. A module posted under 2 h ago is "probably awake, its next floor post is late", and no wake is offered. Before `postedAt` is read, nothing is offered. Where the wake is offered, `NO_LIVES_NOTE` says plainly that if the collection is in fact awake, its next post serves the request and the bond pays for it (both of the reviewer's suggestions). | `3a652b3` | site: `D2-I-1: with no lives served, an ACTIVE collection's brief staleness offers no wake; ...` (the reviewer's `wakeView(null, false, { dormancy: true, fresh: false })` now gives `canWake: false`); the DI-L-1 test now passes an old `postedAt` |
| D2-I-2 | `Dorm::desk_ask` (DORMANT): a queued desk wake keeps its time only for the same request id. A queued desk wake of an earlier request (closed, or past its window, and not read again yet) is replaced at the new request's own read time. Owed and operator wakes still keep their place. | `4145059` | cargo: `d2_i_2_a_new_desk_request_queues_at_its_own_time_not_a_closed_requests` (the PoC: Doodles' older #1 is now ahead of Milady's #2); unit: `desk_requests_queue_within_their_window_and_never_displace_an_owed_or_operator_wake` (updated: the new request queues at 55, not 10) |

## Choices, and where they differ from the suggested fixes

- **D2-L-2: on-chain confirmation, not a typed error.** The review offered two fixes: a typed error from `home_sign` that separates
  a JSON-RPC execution error from a transport failure, or confirming on chain before giving up. The second was taken, for three
  reasons:
  - It does not depend on how each provider words a revert (`rpc_max` spans several providers and a consensus rule).
  - It leaves the other home steps that share `home_sign` untouched (looper, frogflow, redeem).
  - It is also correct for a real revert that is only temporary. `fill` reverts `NoRoom` while the module is at `maxEthBacked`. The
    hand-in is now kept for that too, until the curve's own window ends it.

  The cost is bounded:
  - A mark whose estimate keeps failing is tried once a tick, only while `now < until`.
  - A kept hand-in is tried once every `FILL_RETRY_SECS`, and at most `FILLS_MAX` are held.
  - A persistent disagreement is visible: it shows as a "waits" line every tick.
- **D2-L-3: the wording, not a `dropped` flag.** The brief asked for whichever option keeps the promise made to the user. Only the
  wording does. With a flag, the canister would not mark a dropped request, but the requester could still be charged, so the old
  promise ("its bond goes back unserved") would stay false in two cases:
  - `markServed` is permissionless: anyone can record the post.
  - The unrecorded `served` also counts the module's latest post if it lies inside the window. A collection woken by a deposit that
    sleeps again before the window ends is charged with no mark at all.

  The contract is what decides, so the canister's log, the life's note and the site now state the contract's rule. The canister keeps
  recording the post, so the treasury's side stays consistent with the rule. The requester did get a wake inside the window.
- **D2-I-1: both suggestions.** The page waits for a stale period (2 h since `postedAt`) and also words the risk where the wake is
  offered. A collection that slept 1 to 2 h ago is not offered a wake without lives. That is a short delay, and with lives served the
  page is unchanged.
- **D2-I-2: the read time (`now`), not `ask.at`.** A new request with nothing queued already queues at its read time, so this is the
  same rule. A request's chain time can only be earlier than its read time, so this never jumps a request ahead of what was already
  queued.

## Invariants (re-checked)

- **Pin-floor order.** No change to the tick's order. The new reads go through `home_read` at the tick's pin, after `home_floor_ok`:
  `requestAt(id)` in `mark_act`, and the fill hunt's existing reads.
- **`max_response_bytes` before the transform.** There is no new HTTPS outcall kind or transform. The estimate and the reads use the
  existing capped paths.
- **Nonce path.** `Step::Served` and `Step::Fill` still go only through `send_home` / `resolve_home`, with one home transaction in
  flight. A failed estimate signs nothing, and nothing is saved as pending. `mined_home` is unchanged apart from a comment.
- **H3, 25 ETH a day.** Untouched: `markServed` and `fill` move no canister value.
- **Staged caps, and the exit rule vs the 72 h timelock.** Untouched. No install-argument, cap or stage change, and golive-check is
  unchanged.
- **Governance only via the timelock.** `WakeDesk` still has no owner, setter, pause or upgrade. `NO_LIVES_OFFER_AFTER_S` is a site
  constant and gates no funds. The canister's rules are constants in the wasm.
- **No code at the canister's addresses.** The canister still only calls `markServed` and `fill`, and deploys nothing.

## Suites

| Suite | Result |
|---|---|
| `forge test` | 1,164 passed, 0 failed (133 suites; `WakeDesk.t.sol` 21, 2 new) |
| `cargo test` (autobuyer/canister-rs) | 777 passed, 0 failed (6 new in `tests/dormant.rs`, 1 unit test updated) |
| monitor `npm test` | 249 / 249 |
| ops `npm test`, `tsc --noEmit` | 74 / 74; tsc clean |
| web-rook tests, `tsc --noEmit` (local binaries) | 99 / 99 (2 new, 2 updated); tsc clean |
| `DRYRUN_NO_DRAND=1 scripts/golive-dryrun.sh stack` | DRY RUN: ALL PASS (exit 0, 136 `ok` lines, as before) |

## Left as it is

- **A mark whose estimate keeps failing for the whole window** while the chain shows it can be taken (a provider that is permanently
  wrong). It is retried every tick and paged by nothing new, but its "waits" line is in every tick's log. If it never lands and the
  collection keeps posting past the window, the bond is refunded: the treasury is the side that loses, as in round 1's "mark's margin".
- **A collection that slept 1 to 2 h ago** is not offered a wake by the page when no lives are served (D2-I-1's gate). With lives
  served, the page is unchanged.
- **`buyer.wasm` b7532c4d** is not installed and was not touched. Nothing was deployed, signed or pushed.
