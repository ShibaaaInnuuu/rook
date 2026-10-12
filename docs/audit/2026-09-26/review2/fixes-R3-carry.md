# Round 3, the carried empty snapshot: a draw with no eligible holders closes with no winner

Branch `r3-carry` (from `fixes-2026-09-26` at `20d70f9`, the merged round 3). Written before the code, as the design; "As built"
at the end is filled in when the code lands. Nothing here is deployed; no key, wallet or live chain is touched. Network use is
read-only (the public drand relays and a read-only fork of a public 4663 RPC in the dry run and the keeper fork test).

Input: `integration-R3.md` §2 and §8 ("Still open", first item), and `fixes-R3-contracts.md` §3 (N-2).

## 0. The gap

N-2 (`fixes-R3-contracts.md` §3) made a seed, once pinned for a coin, consumable only by a pin: `voidDraw` of an Announced draw and
`cancelDraw` of an Announced or Open draw store the draw's seed origin as the coin's `carriedSeed`, and the coin's next
`announceDraw` reuses it, so it gets the same snapshot time. `openDraw` refuses `totalTickets == 0`. So if the carried snapshot time
falls on a moment when **no eligible holder had a whole coin**, no carrier can ever be opened, every carrier is voided after
`OPEN_TTL` and carries the seed again, and nothing but a pin (which needs an opened draw) clears it: the coin's units are never drawn.

Integration R3 made the keeper avoid this (a new seed is announced only when the whole 24 h window has tickets; a carried empty
seed is refused and paged `draw-carried-empty`), but it can still happen: a keeper that announced without the check, the exclusion
list changing between the announcement and the open, a log provider that under-reported, a stack's first day. The contracts need
a way out that does not reopen N-2.

## 1. What the remedy must keep

1. **N-2:** nobody can discard a snapshot time after seeing it and get another one. The keeper by not opening, the owner or the
   guardian by cancelling: each is only a delay. The review2 PoCs (`test/audit2/R2Draws.t.sol`) stay green unchanged.
2. **C-03/C-04:** the snapshot time is chosen by a drand round nobody had seen when the origin was announced; the root commits the
   ticket count and the time (the sentinel leaf); the list waits `CHALLENGE` (15 min) for the monitor to recompute it and the owner
   or guardian to cancel it; nothing is pinned while the keeper is paused.
3. **No new keeper power.** Whatever the keeper can do with the remedy must be caught by the same recompute that already catches a
   forged root, and must be no worse than a forged root when it is not caught.
4. **No loss.** The drawn unit is never lost or moved: it returns to its coin for a later draw.

## 2. Options considered

| Option | Verdict |
| --- | --- |
| **A. An empty draw opens and closes with no winner.** `openDraw` accepts `totalTickets == 0` when the root is exactly the sentinel leaf `leaf(address(0), 0, snapshotAt)` (the one-leaf tree). It waits the same 15 min, the owner or guardian can cancel it (the cancel carries the seed as today), and after the window the ordinary `requestDraw` closes it as `Empty`: the carry is consumed, the unit goes back to the coin, no round is pinned. | **Chosen.** The emptiness is committed on chain exactly like any list (the sentinel, which already commits the count and the time), recomputed by the same monitor, and cancellable by the same guardian in the same window. The carry is cleared only by a closed draw of the same snapshot time, never by the passage of time or someone's choice. |
| B. An expiry: after `k` carries (or `T` days) the carry is dropped and the next announcement pins a fresh future round. | Rejected. The contract cannot tell an empty carried snapshot from a full one without a committed list, so the expiry applies to any carry: a keeper that dislikes a snapshot time declines to open `k` times and gets a new one. That is N-2's re-roll with a delay. Limiting it to "empty" needs an emptiness commitment, i.e. option A. |
| C. `voidDraw` of an Announced draw clears the carry when the origin's snapshot time is before the coin's launch (integration-R3 §8 idea 1). | Rejected as the remedy: it covers only the moment before the coin existed, not a young coin's gap, a changed exclusion list or a whole-coin holder selling out. |
| D. The owner (the timelock) drops a carry after `OPEN_TTL` with a public reason (integration-R3 §8 idea 2). | Rejected. It gives the owner a re-roll of a snapshot time already public; the timelock's delay makes it visible, not impossible. |

## 3. Design (option A)

### 3.1 `SweepVault`

- **`Stage.Empty`**, appended after `Announced` (value 8): the earlier stages keep their numbers; `Draw` is unchanged.
- **`openDraw(id, root, 0, uri, sentinelProof, seedSignature)`** is accepted iff `sentinelProof` is empty and
  `root == leaf(address(0), 0, at)`, where `at = snapshotTimeOf(id, seed)` as today. That is the whole tree: an empty snapshot with
  its sentinel. Any other root with zero tickets (holder leaves beside a zero sentinel, a wrong time) is `BadProof`. `root == 0` is
  still `BadInput`. Everything else is as for any open: the seed round is proven or read, `SnapshotFixed` and `DrawOpened` (with
  `totalTickets = 0`) are emitted, the challenge window is `CHALLENGE` on both clocks.
- **`requestDraw(id)`** of an Open draw with `totalTickets == 0`, under exactly the gates of a pin (the window over on both clocks;
  the keeper neither paused nor zero), **closes it**: stage `Empty`, `drawInFlight` and `carriedSeed` of the coin cleared (the seed
  is used: the coin's next draw pins a new one), the reserved unit returned to `units[coin]`, event
  `DrawEmpty(drawId, coin, snapshotAt)`. No round is pinned: there is nobody to pick. Anyone may call it, as anyone may pin.
- **`cancelDraw`** of an Open empty draw is unchanged: the owner or the guardian, any time before the close, and it **carries the
  seed** like any cancel. So a false empty (below) is answered by a cancel, the next draw of the coin reuses the same snapshot time,
  and the keeper must open it with the real list.
- `voidDraw`, `resolve`, `claim` refuse an `Empty` draw (`WrongStage`): it is final. No function, error or storage slot is added;
  the exact-surface tests (`test/security/SweepSecurity.t.sol`) keep their function list.

### 3.2 Why this keeps N-2 and adds no keeper power

- **A true empty.** The carried snapshot time is fixed (N-2); at that moment no eligible holder had a whole coin, so there is no
  list the keeper could publish but the empty one, and it is recomputable by anyone from public logs. Closing it with no winner is
  the honest outcome of that snapshot, and only a draw of that same time consumes the carry. The next seed is a new drand round
  pinned at the next announcement, unknown to everyone: C-04 as for any new draw.
- **A false empty** (the keeper opens "empty" on a snapshot that had holders, to throw the time away): the sentinel commits
  `(0, snapshotAt)` and the root is the sentinel alone, so the claim "nobody" is as checkable as any list. The monitor recomputes
  the tree at `snapshotAt` from its holder indexes, finds tickets, and the verdict is `bad` on two providers: the guardian pauses
  the keeper (so nobody can close it: `requestDraw` refuses while the keeper is paused) and cancels it. The cancel carries the seed,
  the coin's next draw gets the **same** snapshot time, and only the honest list opens it. A false empty repeated is a delay, never
  a new time.
- **Uncaught** (the monitor and the guardian both down for the 15 min, the Safe absent), a false empty re-rolls one snapshot time
  and costs nobody their unit (it goes back to the coin). The same failure of the same watchers lets a forged root pay the unit to
  the keeper's own address, which is strictly worse. So the remedy adds nothing to the keeper's power beyond what C-03 already
  accepts (the list is the keeper's word, checked by the monitor, cancellable by the guardian).
- **A false non-empty** (holders invented on an empty snapshot) is the existing bad-root case, unchanged.
- **A guardian that cancels a true empty**, again and again, keeps the coin undrawn. It can already cancel every draw of every coin;
  this is liveness only, and the Safe replaces the guardian. It does not recreate the stuck state on its own: the next carrier opens
  empty again and closes after its window unless cancelled again.
- **The close is gated like a pin**: never before the window (both clocks), never while the keeper is paused or zero. So the
  monitor's pre-deadline pause of an unverified draw (fixes-R3-services §1.3) holds an empty close exactly as it holds a pin.
- **Holders cannot steer with it**: an empty moment has, by definition, nobody who could have won it; the re-announced draw's
  snapshot time comes from a round nobody has seen.

### 3.3 Services

- **keeper** (`keeper/service`): `STAGE.Empty`, `DrawEmpty` in the ABIs; the draw book treats `Empty` as final. Open flow: an
  empty snapshot (the v2 tree of zero holders is the sentinel alone: root = `sentinelLeaf(0, T)`, proof `[]`, which
  `tools/snapshot`'s `snapshot()` already builds) is published and opened with `totalTickets = 0`, planned as "opened with no
  eligible holders: closes with no winner after the window unless cancelled". Announce flow: a carried seed whose snapshot is empty
  is now announced (the carrier opens empty and consumes the carry); `draw-carried-empty` is no longer raised. A new seed keeps the
  whole-window guard (`firstEmptyBlock`): an empty draw is not stuck any more, only a wasted cycle and a round with no winner, which
  holders do not expect. The resolver's `requestDraw` of an Open draw is unchanged (it now closes an empty one).
- **monitor**: `DRAW_STAGE` gains `Empty`; `DrawEmpty` in the ABI and its page (SEV2: which draw, the snapshot time, "the unit
  went back to the coin; the coin's next draw pins a new seed"). An Open draw with zero tickets is recomputed like any other (the
  sentinel-only tree), with a SEV2 page when it opens ("the keeper says NOBODY held a whole coin at T; if the recompute finds
  holders the guardian pauses and cancels"); a `bad` verdict on it is the existing pause-then-cancel. A deciding draw that closes
  `Empty` unverified is SEV1 (the carry was consumed on the keeper's word), as a pin of an unverified draw is today.
- **tools/snapshot**: `STAGES` gains `Empty`; `--vault/--draw-id` on an empty draw rebuilds and compares as usual (exit 3 on a
  mismatch, worded as a false empty when the chain has zero tickets and the rebuild has some); a test holds the empty v2 tree to
  `root = sentinelLeaf(0, T)`, proof `[]`.
- **indexer / site**: `RaffleStage` gains `"empty"` (the three copies of `types.ts` stay identical); `DrawEmpty` folds the draw to
  `empty` (the carry it consumed is not passed on); the site says "No eligible holders at the snapshot — no winner this round. The
  unit stays with the coin for its next draw." ABIs from forge's artifact.
- **docs**: `docs/snapshot-v1.md` "Version 2" (the empty tree); `fixes-R3-contracts.md` §3 and `integration-R3.md` §8 point here;
  the `SweepVault` header and NatSpec.

## 4. Tests

- `test/audit2/R3CarryEmpty.t.sol` (new): the stuck scenario (an empty snapshot voided and carried, re-announced with the same seed)
  now opens empty and closes `Empty`, clears the carry, returns the unit, and the coin's next draw pins a new seed and is drawn
  normally; a fresh (uncarried) empty draw closes the same way; the empty open's root is the sentinel alone (holder leaves beside a
  zero sentinel, a non-empty proof, a wrong time: `BadProof`); the close waits the window on both clocks and refuses while the
  keeper is paused; a false empty is cancellable during and after the window and before the close, the cancel carries the seed,
  and the next draw opens the same snapshot time with the honest list (repeated false empties keep one time); an `Empty` draw
  refuses `resolve`, `claim`, `voidDraw`, `cancelDraw`, `requestDraw`; the books hold (`_assertVaultBooks`).
- `test/audit2/R2Draws.t.sol` and the rest of `forge test` unchanged and green; the sweep invariant handler gains an empty-open
  action so the fuzzer mixes empty draws in.
- keeper, monitor, tools/snapshot, indexer, web-rook: the new stage and event, the empty open and announce plans, the monitor's
  pages and the false-empty `bad` verdict, the tool's empty tree, the site's line.
- `scripts/golive-dryrun.sh stack` (real drand) and `scripts/keeper-fork-test.sh` on a read-only public fork.

## As built

**As designed**, with these precisions:

- **Contract.** `SweepVault`: `Stage.Empty` (8), event `DrawEmpty(drawId, coin, snapshotAt)`. `openDraw` with `totalTickets == 0`
  requires an empty `sentinelProof` and `root == leaf(address(0), 0, snapshotAt)` (else `BadProof`; `root == 0` stays `BadInput`).
  `requestDraw` of such a draw, under the pin's gates, sets `Empty`, clears `drawInFlight` and `carriedSeed`, returns the unit
  (`_unreserve`) and pins no round. No function, error or storage was added; `Draw` is unchanged. `cancelDraw` and `voidDraw` are
  unchanged.
- **Keeper.** `announceable` returns `{ why, note }`: a carried empty seed is announced with the note "it will open empty and close
  with no winner, which consumes the carry"; `draw-carried-empty` and `draw-empty` are gone. The open plan of an empty snapshot says
  `open draw N EMPTY: …`; the file published is the tool's empty v2 tree (`entries: []`, `sentinel.proof: []`). `STAGE.Empty`,
  `DrawEmpty`; `DrawBook` treats `Empty` as final. The resolver needed no change (its `requestDraw` closes the draw).
- **Monitor.** `emptyOpenedAlert` (SEV2, `draw-empty-opened:`) from `DrawOpened` with zero tickets and from the state scan;
  `drawEmptyAlert` (`draw-empty:`) SEV2 after a verified recompute, SEV1 (persistent) when the draw closed while pending/disagree
  or after a confirmed bad root. Statuses `closed-empty`, `closed-empty-unverified`, `bad-closed-empty`. The recompute itself needed
  no change: `weightedTree` of nobody with the sentinel is the sentinel leaf.
- **Snapshot tool.** `snapshot()` already built the empty v2 tree; `STAGES` gains `Empty`, `--vault/--draw-id` says when a draw was
  opened empty and words a mismatch on it as `FALSE EMPTY`.
- **Indexer / site.** `RaffleStage` `"empty"` in the three `types.ts` copies; `DrawEmpty` folds to `empty` (`seedPassedOn` false,
  `voidableAt` null); `judgeList` accepts an empty `entries` list for a zero-ticket draw (it must still rebuild to the sentinel alone).
  The site's draw shows "No eligible holders at the snapshot — no winner this round. The unit stays with the coin for its next
  draw." (and, while Open, what will happen); the keyless step on an open empty draw is "Close (no winner)" (`requestDraw`). The
  Phase 1 trust text names the false empty beside the forged list.
- **Tests added.** `test/audit2/R3CarryEmpty.t.sol` (8), the sweep invariant handler's `openEmptyDraw` (fuzzed) and its reachability
  test; `keeper/test/integration_r3.test.ts` (the carried empty time announced, opened empty, closed, carry consumed, next seed
  fresh; a draw with its own seed opened empty and closed); `monitor/test/carry_empty.test.mjs` (4); `tools/snapshot/test/v2.test.ts`
  (the empty tree); `indexer/test/carry_empty.test.ts` (3); `web-rook/test/carry_empty.test.ts` (3).

**Results** (this machine, 2026-09-26):

| Suite | Result |
| --- | --- |
| `forge test` | 1031 passed, 0 failed (116 suites; `R2Draws.t.sol`'s five N-2 PoCs unchanged and green; `R3CarryEmpty.t.sol` 8) |
| keeper `npm test` / `tsc` | 131 passed / clean |
| monitor `npm test` | 106 passed |
| tools/snapshot `npm test` / `tsc` | 19 passed / clean |
| indexer `npm test` / `tsc` | 123 passed / clean |
| web-rook `npm test` / `tsc` | 65 passed / clean |
| web `npm test` / `tsc` (the shared `types.ts`) | 118 passed / clean |
| `scripts/golive-dryrun.sh stack` (fork of rpc.ordofi.network, real drand rounds for the seed and the pin) | ALL PASS |
| `scripts/keeper-fork-test.sh` (read-only public fork) | ALL PASS (step 9 skipped: no `dfx` here, as before) |

The two scripts exercise the ordinary draw path (announce → open → pin → resolve → claim) against the changed contract; the empty
path is covered by the forge, keeper and monitor tests above, not by a fork run. `cancelDraw`'s NatSpec was reworded after the two
scripts ran (comment only; `forge test` re-run green).

**Still open.** A guardian (or Safe) that cancels a true empty again and again keeps that coin undrawn: liveness only, the same power
it has over every draw. An uncaught false empty re-rolls one snapshot time without loss (§3.2); it is dominated by an uncaught forged
root. The keeper keeps the whole-window guard for new seeds, so an empty draw comes only from a carried seed, an older keeper, or a
list/log change between announcement and open.
