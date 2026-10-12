# Independent security review 1: dormant, wake-on-demand collections (branch `dormant-collections`)

Date 2026-09-29. Scope: `git diff 9efed1b..e94d01c`, which covers `WakeDesk.sol`, `dormancy.rs`, the dormancy hooks in `ethflow.rs` / `ethnet.rs`,
the runner, the monitor, the site (`Launch.tsx`, `dormancy.ts`, `phase.ts`), `DeployStack`, golive-check and the dry run. The author's
self-review (`dormant-review.md`: DR-H-1..3, DR-M-1..4, DR-L-1..5, S-1..5) was re-checked against the code rather than taken as read.
This is a report only. Nothing on the branch was changed except this file. Every proof of concept ran in a throwaway copy outside the
repository: the canister's fake world (`tests/dormant.rs` harness) and forge (`test/wake/WakeDesk.t.sol` harness). Nothing was
deployed, pushed or signed.

**Threat model** is the same as `dormant-review.md`. The attacker is an outsider who can request wakes with the bond, launch coins,
send NFTs to the canister, wash trade and self-list, and time any of it. The runner may lag or fail. The `GET /collections` relay is
uncertified. The canister key, the operator and a timelocked upgrade are trusted.

**Severity scale** follows `README.md`.

## Suites

| Suite | Result |
|---|---|
| `forge test` | 1,157 passed, 0 failed (133 suites) |
| `cargo test` (autobuyer/canister-rs) | 759 passed, 0 failed |
| monitor `npm test` | 248 / 248 |
| ops `npm test`, `tsc --noEmit` | 74 / 74; tsc clean |
| web-rook `npm test`, `tsc --noEmit` | 94 / 94; tsc clean |
| `DRYRUN_NO_DRAND=1 scripts/golive-dryrun.sh stack` | DRY RUN: ALL PASS (exit 0) |

## Counts

| Critical | High | Medium | Low | Info |
|---|---|---|---|---|
| 0 | 1 | 3 | 2 | 1 |

---

## High

### DI-H-1: a queued desk (or operator) wake hides the owed wakes of the same collection. Deposits are stranded and graduations run into refunds.

**Where:**
- `ethflow.rs:5310`, `dorm_pass` step 3: `if d.kind(&c) == Kind::Dormant { d.request(&c, WakeBy::Deposit { tx }, now) }`
- `ethflow.rs:5327`, the watchdog's filter: `l.is_dormant() && l.ever_active && l.queued.is_none()`
- `dormancy.rs:497-506`: `Dorm::kind` returns `Kind::Queued`, not `Kind::Dormant`, for a DORMANT life with anything queued

**The bug.** `Dorm::request` is written to upgrade a queued non-owed wake into an owed one (`Some((_, at)) => (by, at)`). No caller
ever reaches that branch:

- A deposit asks for its wake only when `kind == Kind::Dormant`. A collection with a desk or operator wake queued is `Kind::Queued`,
  so the deposit never asks.
- The graduation watchdog skips every collection whose `queued` is `Some`, so `gradEth()` is never read for it.

The queued desk wake stays non-owed. It gets none of the owed path's guarantees:

- No `OWED_RESERVE` slot.
- No cooldown exemption.
- The full `wake_fits` runway test instead of `owed_wake_fits`, which needs only the reserve.

**Exploit or failure.** At the launch numbers (34 T start), `effective_max_active` falls to 0 about a day after the first wake. That
is residual 3 of the scaling review: the expected steady state until a top-up. From then on, a desk wake needs 32.1 T liquid and an
owed wake needs 12.5 T.

1. Mallory pays 0.003 ETH on the WakeDesk for collection X. The canister queues the request, because no desk wake fits.
2. A holder sends an NFT of X to the canister, or a coin on X's once-posted module reaches its raise (`gradEth > 0`). On this
   balance either one should wake X at once through the owed path.
3. Nothing wakes, and three things follow:
   - The deposit sits in `Seen`/`Mint` with the NFT held by the canister.
   - DR-H-2's return path never fires, because `owed_fails` never increments.
   - The graduation is never filled. After the curve's 3-day `window`, anyone may call `FrogCurve.openRefunds()` and the launch fails.
4. Mallory's bond comes back. `forfeit` refunds it, because the module was never posted.

The cost to Mallory is gas plus eight days of 0.003 ETH locked. It also happens with no attacker: any honest desk or operator request
that queues during a low balance does the same. The monitor pages `eth-wake-queued`, attributed to the desk request. It does not
page the stuck deposit or the graduation, because `eth-grad-queued` fires for awake collections only (`monitor/src/ethnft.mjs:620`).
The page's own advice (a top-up of about 32 T) is the only way out short of an upgrade. This breaks DR-H-1's stated guarantee that
"desk wakes can never lock [owed wakes] out" and S-1's "an owed wake may use the runways".

**PoC** (fake world, all pass):
- `poc_di_1_control_a_deposit_wakes_owed_on_a_balance_that_keeps_no_desk_wake`: at 20 T, a deposit wakes Doodles.
- `poc_di_1_a_queued_desk_request_downgrades_a_later_deposit_and_strands_it`: same balance, with Mallory's desk request queued
  first. After ten days of ticks the life is still `queued: Desk`, the collection is `Queued`, and the deposit is still `Seen`.
- `poc_di_1_a_queued_desk_request_blinds_the_graduation_watchdog`: Milady once active and asleep, a desk request queued, then
  `gradEth = 4 ETH`. Over three days the module is read zero times and no owed wake starts.

**Fix.**
- In step 3, request for any DORMANT life: `d.life(&c).is_some_and(|l| l.is_dormant())`. Here `Dorm::request` already keeps an owed
  wake and upgrades a non-owed one.
- Let the watchdog run on a DORMANT life whose queued wake is not owed:
  `l.queued.as_ref().is_none_or(|(b, _)| !b.owed())`.
- Add a fake-world test for each: a deposit, and a graduation, arriving behind a queued desk wake on a balance that fits only owed
  wakes.

---

## Medium

### DI-M-1: desk requests never expire in the canister. Closed and refunded requests are served later, and a late service charges the requester.

**Where:**
- `ethflow.rs:5249-5282`: each desk request is read once, and `closed` is checked only then.
- `dormancy.rs:537-610`, `start_wakes`: a queued `WakeBy::Desk` is started with no re-read of the desk.
- `WakeDesk.sol:166-173`, `_served`: served means `postedAt() >= r.at`, judged at `forfeit` time, with no upper bound.

**The bug.** Once the canister has queued a desk request, the request stays queued until it starts, however long that takes:

- It is never re-read.
- It never expires with the desk's window.
- It is never dropped when the request is closed on chain by `reclaim` or `forfeit`.

On the chain side, `forfeit` decides "served" from the module's *latest* `postedAt`. Any post after the request counts, including
one after the reclaim deadline.

**Exploit or failure.**
- **Free wakes, and squatting at the head of the queue.** Mallory requests while the balance keeps no desk wake. The request queues.
  After `window + RECLAIM_GRACE` (8 days) with no post, anyone calls `forfeit`: Mallory is refunded and the request is closed. The
  canister still holds the queued wake with its old `at`. At the next top-up that wake starts before every newer honest request,
  because the queue sorts by `at`, and it costs Mallory nothing. With one request per collection, Mallory can hold the front of the
  queue for all ten collections indefinitely for gas alone. That defeats the bond, which is the desk's only anti-griefing lever
  (dormant-review residual 1).
- **Honest requesters lose the bond for a wake they could never use.** Alice's request waits behind a low balance past her reclaim
  deadline. After a top-up the canister serves the stale request and posts. Anyone who then calls `forfeit` sends Alice's bond to the
  treasury, although no reclaim was ever possible. The site's `RECLAIM_NOTE` promises that a wake not served in the window is
  refunded.

**PoC:**
- Fake world: `poc_di_2_a_desk_request_closed_and_refunded_on_chain_is_still_served_later`. The request is queued at 20 T, the
  window and grace pass, the fake desk marks it closed, and it is still `Queued`. After a top-up it starts `Waking`, with
  `by = Desk { id: 0 }`.
- Forge: `test_poc_lateServiceChargesARequesterWhoCouldNeverReclaim`. A request, then `window + 1 day + 1`, then the post, then
  `forfeit`. The treasury receives the bond.

**Fix.**
- Canister: before starting a queued `WakeBy::Desk`, re-read `requestAt(id)`. Drop the wake if it is `closed` or if `at + window` has
  passed (this needs the window: add it to the install argument, or read `window()` once). Or re-read queued desk wakes each tick
  with the desk cursor.
- `WakeDesk`: count a post as serving request `id` only if it lands inside the window. Either:
  - have the canister, which is the keeper, call a `served(id)` marker, or
  - record the first post at or after `at`: add `firstPostedAt` to the module, or have `request` read and store `postedAt` so that
    `forfeit` can require `postedAt` inside `[at, at + window]`.

  A later post then no longer turns an unserved request into a forfeit.

### DI-M-2: one open request per wrapper, plus the canister dropping requests for awake collections, lets anyone block the public wake path for a week at little or no cost

**Where:**
- `WakeDesk.sol:106`: `if (last != 0 && _open(last - 1)) revert AlreadyRequested(...)`
- `dormancy.rs:523-524`: a request read while the collection is WAKING or ACTIVE returns `Ok("already …")` and records nothing.

**The bug.** The desk allows one open request per wrapper for the whole `window` (7 days). The canister, meanwhile, discards a
request that it reads while the collection is WAKING or ACTIVE. The request stays open on chain but asks the canister for nothing.
The Launch page reads `openOf(wrapper)`, hides the wake button and says *"A wake is already asked for this collection; it warms up
when the canister reads the request (within a tick)"*. That is false.

**Exploit.**
1. Mallory calls `request(wrapper)` while X is awake. X only needs to be awake for one tick: from its own wake, from anyone's owed
   wake, or in its last hours before the idle sleep.
2. X sleeps (`NO_POSITION_SECS` or the idle rule). For the rest of the 7-day window nobody can ask for X's wake on the desk.

The cost is 0.003 ETH per collection per week, which comes to 0.03 ETH a week for all ten. It is zero when timed well: Mallory
requests during a WAKING that later times out, or after X's last post before its sleep check, so the module is never posted after
`r.at` and `forfeit` refunds Mallory. Deposits and the operator's `eth_wake` still work. The public, bonded path does not.

**PoC** (fake world): `poc_di_3_a_request_read_while_active_is_dropped_and_its_open_window_blocks_every_later_request`. Mallory's
request is read while Milady is ACTIVE ("is already active"). Milady sleeps, and six days later it is still DORMANT with nothing
queued, while the desk would refuse any new request until the window ends (`test_request_recordsWhatTheCanisterReads_andOneOpenPerCollection`).

**Fix.** Any one of these:
- Have the canister keep a desk request read while WAKING or ACTIVE: queue it so that it re-wakes if the collection sleeps within the
  request's window.
- Have `WakeDesk.request` refuse while the collection's module is `fresh()`. The collection is awake, so there is nothing to ask for.
- Drop the one-open-request rule. Several bonded requests are harmless: the canister already de-duplicates by collection.

Also make the site's wording depend on the canister's life (`queued`/`waking`), not only on `openOf`.

### DI-M-3: one collection waiting to converge stops every other collection's graduation fill (`fill_act` returns early)

**Where:** `ethflow.rs:5455-5475`, `fill_act`. Inside the per-collection loop, both waits `return Err(...)`:
- the module's floor is not fresh (`posted_at` older than 59 minutes), and
- DR-M-1's "accepted has not caught up with our reading".

Either one ends the whole hunt for the tick. Collections later in `cols` order are never tried. `fill_act` is the last step of
`home_act` and runs every tick, so the block lasts as long as the first collection's wait.

**Failure.** Collection A (earlier in the configuration) wakes after a long dormancy with a graduation queued. Its floor has moved,
so DR-M-1 holds A's fill until the module's clamped `accepted` walks to the reading. The module moves at most ±5 %/h and ±20 % a day
from the day anchor, so a 40–50 % move takes two to three days. Meanwhile collection B (ACTIVE, fresh, converged) has its own
queued graduation, which is never filled. B's curve `window` is 3 days: B's holders get refunds instead of the graduation they paid
for. An attacker can arrange the trigger cheaply:
1. Launch a coin on a once-posted collection whose market has drifted more than 5 % while it slept.
2. Buy it to its raise. That money is refunded if it is never filled.
3. The owed wake follows.

A stale module on A (a post that keeps reverting) blocks B the same way.

**PoC** (fake world):
- `poc_di_4_one_collection_catching_up_blocks_every_other_collections_fill`: Milady is catching up (stale 2.0 ETH, walked 1 % a
  post). Over a day, 96 ticks end the fill act on Milady's wait, and Doodles' Graduating coin is never filled.
- `poc_di_4_control_doodles_fills_when_no_other_collection_waits`: Doodles alone is filled.

**Fix.** Handle each collection's wait inside the loop:
- collect it into `waits` and `continue` to the next collection with a due `fill_hunt`, and
- set that collection's `fill_hunt` to `now + FILL_RETRY_SECS` so that it is not re-read every tick.

Return `Err` only when no collection acted.

---

## Low

### DI-L-1: the site as built never offers a wake, because `VITE_COLLECTIONS_URL` is not passed to the Docker build and the page shows no wake button without it

**Where:**
- `web-rook/Dockerfile:15-20`: only `VITE_ETCH_API`, `VITE_ETCH_STAGING_*` and `VITE_ROOK_PHASE` are declared as `ARG`s.
- `web-rook/src/rook/dormancy.ts:43`: `wakeView(null)` returns `canWake: false`.

The documented deploy (`railway up ./web-rook`, a Docker build) cannot pass `VITE_COLLECTIONS_URL`, so the Launch page has no lives.
With no lives, `wakeView` returns "Awake" with `canWake: false`. Under the dormancy the launch still fails closed on `fresh()`, as
DR-M-4 intended, but the only guidance is *"… (wake it if it is dormant)"* with no button. The bonded public wake path is unreachable
from the site, and a user can only call the desk directly. This is safe, but it blocks a feature.

**Fix.**
- Declare `ARG VITE_COLLECTIONS_URL=` and add it to the `ENV` line in the Dockerfile, and name it in the runbook.
- When the stack file names a desk and `fresh()` is false with no lives, offer the wake button, gated on the on-chain `openOf` read.

### DI-L-2: `forfeit`'s refund is a push with a 50,000-gas stipend and no pull fallback

**Where:** `WakeDesk.sol:148-155`. A requester whose receive path needs more than 50k gas loses the bond to the treasury, even though
the request was never served. That includes some smart-contract or 4337 wallets, and proxies with logging. This is by design, and
the documentation says "a requester that refuses ETH … only sends its own bond to the treasury", but an honest contract wallet is
treated as a refuser.

A returndata bomb was measured and is not a problem: 130,770 gas against 124,808 for a plain reverting requester.

**Fix.** On a failed push, credit `owed[requester] += b` and add `withdraw()`, instead of paying the treasury.

---

## Info

- **DI-I-1: stray pnpm files in `web-rook/`.** `pnpm-lock.yaml` (6,035 lines) and `pnpm-workspace.yaml` were added in `134a06c`.
  The workspace file holds pnpm's unfilled placeholder (`allowBuilds: esbuild: set this to true or false`). The build and the Docker
  image use `npm ci` with `package-lock.json`, so neither file has any effect today. Two lockfiles that can drift is a supply-chain
  hazard. Remove them, or pick one package manager.

---

## The author's fixes, re-checked

| ID | Holds? | Notes |
|---|---|---|
| DR-H-1 | Partly | The idle rule (`NO_POSITION_SECS`, positions read on 4663) and `WakeDesk.reclaim`'s graduated / not-sweep-only / `spent` rules hold, verified in code and by the existing tests. The owed reserve's guarantee does not hold: see DI-H-1. |
| DR-H-2 | Yes | The back-off doubles, and the third timeout returns the deposit (existing fake-world test). The count never decays until an ACTIVE wake, so a collection with three owed failures bounces every later deposit until a desk or operator wake confirms it (acceptable). |
| DR-H-3 | Partly | An unserved wake is refunded. A wake served after the window is charged, and closed requests are still served: DI-M-1. |
| DR-M-1 | Yes | `converged` gates `Step::Fill` and new buys. Deposit mints are guarded by the older `deposit_floor_ok` band against a stale-high `accepted`. Its early return has a cross-collection side effect: DI-M-3. |
| DR-M-2 | Yes | Hand-ins first, then the 8 newest coins. The factory, `isFilled` and `state` are read before signing. |
| DR-M-3 | Yes | `pass_due` forces a pass within 25 minutes of the TTL, and not-due evidence is kept. |
| DR-M-4 | Yes | Item-buying launches fail closed on `FloorModule.fresh()` read by the page. A relay that lies can only hide the wake button or cause a bond on an awake collection. The deployment gap is DI-L-1. |
| DR-L-1 | Yes | The cursor stops on a life not yet decided. |
| DR-L-3 | Yes | Any mined post sets `ever_active` and `posted_at`. |
| DR-L-4 | Yes | No `buy_floor` means refusal. |
| DR-L-5 | Yes | `spent(coin)`. |
| S-1 | Yes | `waking + active >= eff + OWED_RESERVE` blocks owed wakes beyond one over the limit. |
| S-2 | Yes | `reserve_b >= EVIDENCE_CYCLES_FLOOR / B` is enforced in `validate_scaling`, and golive-check fails a 20 T start. |
| S-3 | Yes | `over` needs a read balance, and `start_wakes` refuses on `None`. |
| S-4, S-5 | Yes | The monitor threshold and `capacityUrl` were checked in code and tests. |

## Checked and sound (this review)

- **WakeDesk bond safety.**
  - Every mutator is `nonReentrant`, and `closed` is set before each transfer, so there is no double reclaim or reclaim-plus-forfeit
    (the existing `Reenterer` test).
  - The exact bond is enforced. `coinsAt` stops a coin launched before the request from counting. A request is closed only once.
  - `_served`'s try/catch covers a module without `postedAt`.
  - The `uint64` casts are safe for any realistic time or block.
  - The treasury is immutable. The desk has no owner and no upgrade path.
- **Floor manipulation during WAKING.**
  - No post until the whole 7-day window is read with at most 8 backed-off receipts, and the reading is confirmed (`min_sales`, not
    held).
  - `floor_from` never posts above the rank-2 verified listing, and a first post (`accepted = 0`) must be confirmed.
  - The first post after a dormancy is clamped by the module, and DR-M-1 holds fills and buys until the module converges.
  - Deposit mints are held to the band over `min(listing, p25)`.
  - Wake-on-demand lets an attacker choose *when* a never-posted collection's first post happens. The same judges apply as before, so
    no new lever was found.
- **Stale floor at buy time.** `fresh_for_fill` (at most 900 s old, scanned at most 300 blocks behind, confirmed, within 5 % of
  `accepted` and 10 % of `buy_floor`) runs immediately before the fill signature, and fails closed for a collection with no life.
- **Scale-down.**
  - A collection with a position (wX supply, items held or rented, graduation ETH or units, transit, float, or anything open in the
    canister) is never put to sleep.
  - `scan_tickets` is skipped only for collections that hold nothing.
  - Deliveries, returns and posts are not gated by the scaling.
  - The only stranding found is DI-H-1's.
- **Arithmetic.** Every figure in `Scaling` saturates, `slot() >= 3.5 T` at the bounds, and `effective_max_active` and
  `wake_headroom` are clamped to the `u8` ceiling. The owed back-off shift is capped (`min(16)`). No overflow was found.
- **Balance read.** `canister_liquid_cycle_balance` is a system call (never `None` on the IC). It is read once a tick and passed to
  every decision. With `None`, no wake starts and nothing is scaled down.
- **Invariants.**
  - The 4663 pin check (`home_floor_ok`) runs before `dorm_pass`, and every new read goes through `home_read` at the tick's pin.
  - There is no new HTTPS outcall kind or transform (`max_response_bytes` before the transform is unchanged).
  - `Step::Fill` uses the existing `send_home` nonce path, with `home_effect` reading `isFilled` for a lost transaction.
  - H3 (25 ETH/day) is untouched.
  - The staged caps: the binding global of 48, then 61, at 24 then 31 a day fits the exit rule inside the 72 h timelock less a day.
    `custodycap::admit` enforces the global, and the monitor and golive-check hold it.
  - The dormancy and the desk are in the install argument (timelocked). `eth_wake` and `eth_fill` are operator-only, refused while
    paused or mid-tick.
  - Nothing in this diff gives the canister's Ethereum or 4663 addresses code.

## Appendix: PoC sources (run in a throwaway copy)

Canister, appended to a copy of `autobuyer/canister-rs/tests/dormant.rs` (its `Fake`, `tick`, `next`, `wake_milady`, `launch`):

```rust
const MALLORY: [u8; 20] = [0x66; 20];
fn deposit_of(c: [u8; 20]) -> ef::DepositRec {
    ef::DepositRec { tx: [7; 32], collection: c, token: [1; 32], recipient: ALICE, from: [9; 20], block: 1, phase: XPhase::Seen, refused_since: None }
}
#[test] fn poc_di_1_a_queued_desk_request_downgrades_a_later_deposit_and_strands_it() {
    let mut f = Fake::new(Some(launch(true))); f.deposits_on = true; f.cycles = Some(20 * T);
    let mut st = EthState::default(); tick(&mut f, &mut st);
    f.desk.push((f.cols[1].wrapper, MALLORY, f.now, 3_000_000_000_000_000, false)); next(&mut f, &mut st);
    assert_eq!(kind(&f, &st, DOODLES), Kind::Queued);
    st.deposits.push(deposit_of(DOODLES));
    for _ in 0..(10 * 144) { next(&mut f, &mut st); }
    assert!(matches!(st.dorm.as_ref().unwrap().life(&DOODLES).unwrap().queued, Some((dm::WakeBy::Desk { .. }, _))));
    assert_eq!(kind(&f, &st, DOODLES), Kind::Queued);
    assert!(matches!(st.deposits[0].phase, XPhase::Seen | XPhase::Mint));
}
// (the control without the desk request reaches Kind::Waking in one tick at the same 20 T)
#[test] fn poc_di_2_a_desk_request_closed_and_refunded_on_chain_is_still_served_later() {
    let mut f = Fake::new(Some(launch(true))); f.cycles = Some(20 * T);
    let mut st = EthState::default(); tick(&mut f, &mut st);
    f.desk.push((f.cols[0].wrapper, MALLORY, f.now, 3_000_000_000_000_000, false)); next(&mut f, &mut st);
    for _ in 0..(8 * 144 + 1) { next(&mut f, &mut st); }
    f.desk[0].4 = true; next(&mut f, &mut st);
    assert_eq!(kind(&f, &st, MILADY), Kind::Queued);
    f.cycles = Some(100 * T); next(&mut f, &mut st);
    assert_eq!(kind(&f, &st, MILADY), Kind::Waking);
}
```

The watchdog variant (`poc_di_1_…_blinds_the_graduation_watchdog`), DI-M-2 (`poc_di_3_…`) and DI-M-3 (`poc_di_4_…` and its control)
follow the same pattern, as described under each finding.

Forge, `test/wake/PocIndependent.t.sol` inheriting `WakeDeskTest`:

```solidity
function test_poc_lateServiceChargesARequesterWhoCouldNeverReclaim() public {
    vm.prank(alice);
    uint256 id = desk.request{value: BOND}(address(milady.w));
    _skip(WINDOW + 1 days + 1);          // a reclaim is now TooLate
    _post(milady, 0.9 ether);            // the canister serves its stale queued request
    uint256 t0 = protocol.balance;
    desk.forfeit(id);
    assertEq(protocol.balance - t0, BOND);
}
```

## The dry run

`DRYRUN_NO_DRAND=1 scripts/golive-dryrun.sh stack` ended with **DRY RUN: ALL PASS** (exit 0, 136 `ok` lines). The `FAIL` lines in its
output are refusals the script provokes on purpose, for example a keyed provider URL in the install argument. With `DRYRUN_NO_DRAND=1`
the draw is rehearsed only as far as its announcement.
