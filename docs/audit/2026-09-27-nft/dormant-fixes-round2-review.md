# Confirmation review of the round-2 dormant-collections fixes (`main` 46afc75..07d37c0, 2026-10-08)

Scope: the four commits after independent review 2 was committed, read adversarially against the review's findings and proofs of
concept: `bea711a` (`WakeDesk`), `4145059` (canister), `3a652b3` (site), `07d37c0` (docs). Inputs were
`dormant-review-independent-2.md` and `dormant-fixes-round2.md`. The go-live runbook requires a confirmation review before the
ceremony whenever `buyer.wasm` changes; this is it for the dormant-collections work as merged. Read-only on the chains: nothing was
deployed, signed or sent, and no cycles or ETH were spent. The repository changes made the same day are in §5.

**Verdict: 0 Critical, 0 High, 0 Medium, 0 Low, 3 Info. The fixes hold; the head is fit for the reproducible build and the
ceremony's remaining steps.** `buyer.wasm` of `07d37c0` reproduced (§4): ``9f0e65ab…957f``.

## 1. `bea711a`: `markServed` on the timestamp alone (D2-L-1)

- **The change** is one condition: `p > r.at + window && _elapsed(r, window)` became `p > r.at + window`. The recorded rule is now
  the unrecorded `served`'s rule (`p >= at && p <= at + window`), so the two views of "served" can no longer disagree in the block
  clock's tail.
- **Nothing else moved.** `markServed` still returns silently once recorded, still refuses a closed request, and still reads the
  module's latest post through `_postedAt`'s `try`. `_elapsed` is still what `forfeit` and `reclaim` wait on, so the dual clock
  still protects the requester's grace; only the test of the POST's time dropped the block half, which is right: a post's time is
  its timestamp, and the window is defined on it.
- **The reviewer's PoC** is `test_d2L1_aPostInTheBlockClocksTail_isNotRecordedServed_andTheBondGoesBack`; the fixes note says it
  fails before the fix with "next call did not revert", which is the right failure. The companion test holds the boundary
  (`p == at + window` is still recordable in the tail).
- **Canister side.** `mark_act` now gives the ask up when the module's latest post is past `a.until` (§2), so the canister never
  estimates a `markServed` the desk will refuse on this rule.

## 2. `4145059`: the chain's word, not an estimate's (D2-L-2, D2-L-3, D2-I-2)

- **`mark_act` (D2-L-2).** The flow at head: read `servedAt` (recorded: done); read the module's `postedAt` (not read: wait; before
  `at`: the next post serves it; past `until`: give up, correctly, since a module's latest post never moves back and the desk takes
  only a latest post inside the window); then send. On "would not succeed", `requestAt(id)` is read again: closed → give up; open →
  wait and try next tick; read failed → wait. The only reverts `markServed` can give at that point are `Closed` (now confirmed on
  chain before giving up) and `NotServed` with `!known` (the module's `postedAt` call failing inside the desk), and the latter is
  a wait bounded by the window. A transient provider failure can no longer set `marked`. The PoC counters (`flaky_mark`) are
  reproduced by `d2_l_2_one_transient_estimate_failure_no_longer_abandons_mark_served` and
  `d2_l_2_a_mark_is_given_up_only_when_the_chain_shows_the_request_closed`.
- **`fill_one` (D2-L-2).** A hand-in whose `fill` was not estimated is kept, the collection waits `FILL_RETRY_SECS` (1,800 s), and
  the hand-in is dropped only once the same pin shows the coin filled or its curve no longer `Graduating`. The wait is returned as a
  non-fatal `Err((…, false))`, which `fill_act` already treats as this collection's wait (DI-M-3), so one collection's kept hand-in
  does not hold another's. Only coins in `handed` take this path; a hunted coin that fails its estimate is still passed over as
  before, which is right (the hunt finds it again). Tests: `…_no_longer_drops_the_operators_fill_hand_in`,
  `…_a_kept_hand_in_is_dropped_once_the_curve_is_no_longer_graduating`.
- **Wording (D2-L-3).** Every drop path (`desk_ask` too late, `desk_upkeep` dropped from the queue, the life's note, the
  `DESK_START_MARGIN_SECS` doc) now says the bond goes back "unless the collection is posted inside the window by another wake".
  That is the contract's rule, and the canister keeps recording such a post. Consistent with the site (§3).
- **Queue time (D2-I-2).** `desk_ask` in DORMANT keeps the queued time only when the queued desk wake carries the same request id;
  any other queued desk wake is replaced at the new request's read time. Owed and operator wakes still keep their place (the
  `!matches!(old, Desk)` arm is first). The unit test's expectations were updated accordingly and the fake-world PoC reproduced.

## 3. `3a652b3`: the site (D2-I-1, D2-L-3)

- **`wakeView` without lives.** With `fresh() == false` and no open request, the wake is offered only when `postedAt` has been read
  AND (the module was never posted, or its last post is at least `NO_LIVES_OFFER_AFTER_S` = 2 h old). Before `postedAt` is read,
  nothing is offered ("Reading when it was last posted"). An ACTIVE collection's brief staleness (a post pending past the 1 h TTL)
  is "probably awake, its next post is late" and no bond is asked. Where the wake IS offered, `NO_LIVES_NOTE` says the bond may pay
  for a routine post if the collection is in fact awake: both of the reviewer's suggestions.
- **ABI.** `postedAt()` is added to `FLOOR_MODULE_FRESH` as `uint64`, matching `FloorModule.postedAt`'s type; `Launch.tsx` reads it
  on the same 60 s cadence as `fresh`.
- **Wording.** `openText` and `RECLAIM_NOTE` carry `SERVED_RULE` ("any post inside the window serves the request"), the same rule as
  the canister's notes and the contract.

## 4. Suites and the build, at `07d37c0`

| Suite | Result |
| --- | --- |
| forge `test/wake/*` (WakeDesk, incl. the two D2-L-1 regressions) | 21 passed |
| forge `test/floor/*` (after §5's harness fix) | 132 passed, 12 suites |
| cargo `--test dormant` (the fake world; the five D2 regressions among them) | 42 passed |
| cargo, whole canister | 777 passed, 0 failed (748 at the dormant as-built, + round 1 and round 2 regressions) |
| IC probe `cargo test` (`ops/rehearsal/ic-outcall/probe`) | 7 passed |
| web-rook `npm test` | 99 passed, 0 failed (99 tests) |
| `python3 script/depth_probe.py` (read-only, 2026-10-09T03:16Z) | PASS, both providers, 139,191 blocks deep (re-pin past 400,000 ≈ 2026-11-14) |

**Reproducible build** (`docker build --no-cache --platform linux/amd64 -f autobuyer/Dockerfile.build` on `git archive 07d37c0`):

```
9f0e65ab023ed16cc21c8881ebb3095714563e4fadc4f9d84c07479c3883957f  buyer.wasm  (7,456,262 bytes; gzip -9 -n 1,415,612)
b23a190365d4c5b2bfc41bd9e9caf147bf2eba50ba3b3d477e2be372840edc41  timelock.wasm            (unchanged since review 8)
2145099ea00214e8b521d456f6fc48145c7139f1ecd0c5643393c09c688e62f4  timelock-rehearsal.wasm  (unchanged since review 8)
```

Review 8's `b7532c4d…28ec` and the go-live prep's `d38b808d…effc6` are both superseded: install only the `buyer.wasm` above, once a
second person reproduces it from a fresh clone of `07d37c0` (`docs/deploy/canister-mainnet.md`, step 0).

## 5. The repository's state found on the way (fixed the same day, `golive-2026-10-08`)

- **The nightly deep campaign was red.** `contracts-deep` of 2026-10-07 found a counterexample in `testFuzz_wXStaysFullyBacked`
  (`Slippage()`): a burn of 2 units whose reserve part and convertible part each floor to 0 wei. The module refuses the dust burn and
  nothing moves, so the invariant holds; the harness's guard (`u × min(pr, pc) ≥ 1e18`) did not match `_release`'s per-tier
  flooring. The guard now requires each tier part that exists to pay a wei, and `u ≠ 0` (a 0-unit burn reverts `BadInput`, which a
  second counterexample found). Both counterexamples replay as PASS; 2 × 10,000 runs pass.
- **The nightly was also being cancelled** at the job's 60-minute timeout (10-06, 10-08; 28 min on 09-30, 55 on 10-07): 120 minutes
  now. The run on this branch: `.github` → Actions → `ci` on `golive-2026-10-08`.

## 6. Info

| ID | Note |
| --- | --- |
| R2-I-1 | A kept fill hand-in whose `fill` reverts for a durable reason other than "filled" or "not Graduating" (none is known today) would be re-estimated every 1,800 s until the curve's 3-day refund window closes it: ≈ 144 estimates a day, bounded, and visible in the life's note. A `dropped` reason on the hand-in would let the operator withdraw it sooner; not needed for launch. |
| R2-I-2 | `mark_act` re-reads `requestAt` only after a failed estimate, one extra read a tick per open ask at worst. Bounded by `DESK_READS_PER_TICK`'s spirit but not counted against it; cycles-wise negligible. |
| R2-I-3 | The site's 2-hour rule reads `postedAt` from the module alone. A collection that slept with a post under 2 hours old (the six-hour idle rule makes this impossible after a wake with no position, but a collection put to sleep by `idle_days` posts until it sleeps) shows "probably awake" for up to 2 hours before the wake is offered, with no lives served. Harmless: a request is only delayed, never mis-charged. |
