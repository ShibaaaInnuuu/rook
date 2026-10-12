# Rook fix round 3: targeted security check (R3-check)

Reviewer: independent AI review, 2026-09-26. Checkout `/Users/jlohayon/Documents/Rook-rev3`, detached at `fixes-2026-09-26`
(head `82a2aff`); diff `2bb3ee7..fixes-2026-09-26` (170 files, +8,682 / −745). `lib/` copied from `~/Documents/Rook/lib`, copied
`.git` renamed `.git.bak-copied`.

**Nothing existing was modified. Nothing was committed, pushed, deployed or signed. No key was used.** Network use was read-only:
public 4663 RPC, drand relays, and a local anvil fork for the dry run. The only additions are untracked PoC files:
- `test/audit3/Audit3{Floor,Curve,Draws}.t.sol` (5 tests);
- `autobuyer/canister-rs/tests/audit3_{scanner,pin}.rs` (4 tests);
- `monitor/test/audit3_monitor.test.mjs` (5 tests);
- `indexer/test/audit3_outbound.test.ts` (5 tests).

The R2 contract PoCs, re-run unmodified, are kept out of the tree in `scratchpad/rook-audit/R3/orig-r2-pocs/` (they now fail, as
they should). Part reports: `R3/part-contracts.md`, `R3/part-canister.md`, `R3/part-services.md`. Logs: `R3/*.txt|log`.

## Verdict

**Not ready to merge to main.** The round-2 Highs and Mediums are all fixed. But round 3 added a new economic surface: a keeper
pause now pays burns at the average backing at once. Two things combine against it:
- **A3-M1:** any pause lets an outsider who minted on a fallen floor burn at the higher average backing, at existing holders'
  expense.
- **M-A / M-B:** outsiders, or ordinary RPC flakiness, can make the monitor's fail-closed deadline pause the keeper repeatably.

Together they turn a "safe" liveness mechanism into a value-transfer lever. There are also two new services Mediums with
straightforward fixes (M-E; M-A/M-B themselves).

**Blocking items**
1. **A3-M1:** price paused burns so that a pre-positioned mint cannot profit (fix below).
2. **M-A and M-B:** stop the monitor's holder index from ratcheting to 1-block pages, and from stalling on one dense block.
   Otherwise the fail-closed pause is a false-pause DoS (and with A3-M1, a profit lever).
3. **M-E:** the indexer's `pinnedHttpsFetch` hangs for ever on `HTTP 101`. This is the R2 N-1 class again, and one hostile draw URI
   freezes a collection's API.

Recommended before merge, not blocking: A3-L1, R3-C1, R3-C2, L-C, L-F.

## Counts

| | Critical | High | Medium | Low | Info |
| --- | ---: | ---: | ---: | ---: | ---: |
| Contracts (this reviewer) | 0 | 0 | 1 | 1 | 2 |
| Canister / timelock | 0 | 0 | 0 | 2 | 5 |
| Services (monitor, keeper, indexer, golive-check) | 0 | 0 | 3 | 5 | 3 |
| **Total new** | **0** | **0** | **4** | **8** | **10** |

## 1. Round-2 Highs and Mediums

| Area / ID | Sev (R2) | Verdict | How it was re-attacked |
| --- | --- | --- | --- |
| services **R2-M1**: the monitor's recompute cannot finish on the public RPC pair | High | **Fixed** (new residuals M-A, M-B, L-F) | R2's cases (B always `-32005`; 60 days of history) re-run: the draw is found from vault state, the root is replayed from the per-provider incremental index, and a bad root leads to pause, then cancel. The deadline pause works because `requestDraw` refuses while the keeper is paused **or zero** (confirmed in `SweepVault.sol:523`). |
| services **R2-M2**: two stuck draws starve later ones | Medium | **Fixed** | Every deciding draw is recomputed each poll, earliest `challengeEnds` first, each in its own try/catch. No slot limit remains. |
| services **R2-M3**: a floor walk along the band's edge pages nothing | Medium | **Fixed** for R2's PoC (residual **L-C**) | R2's 24 edge posts now page SEV1 at the third. Evasion found: interleaved no-move posts (L-C). |
| services **R2-S1**: a forged file blocks the claim and the void | Medium | **Fixed** (residual L-D) | `entryIn` proves each entry against the **on-chain** root and refuses holder 0. A reverted claim falls back to the chain rebuild. The void is reachable. |
| services **R2-S8**: the keeper's alerts are never paged | Medium | **Fixed** (residual L-G) | `/healthz` answers 503 after 3 passes with alerts in force. An outsider cannot force a permanent 503. |
| services **N-1**: an HTTP status of 600–999 crashes the indexer | Medium | **Fixed** (new sibling **M-E**) | 600–999, 1000, 199, an invalid reason phrase, 100 then 999, an over-cap body and a mid-body reset: the process lives. `HTTP 101` hangs the fetch for ever (M-E). |
| contracts **N-1** (a) 1-wei fresh freeze, (b) the pause freezes burns for a week | Medium | **Fixed** | R2's `R2Floor.t.sol` run unmodified now fails on its attack assertions: `no burn price: 132537322045440000 != 0` and `an hour later: refused`. A new risk comes out of this fix: **A3-M1**. |
| canister **N-1**: the ticket-log scanner stalls on a page over 48 kB | Medium | **Fixed** (residuals R3-C1, R3-C2) | R2's 60-event burst is read in the first tick. A 2.7 MB block is read by ticket. The cursor always advances (`audit3_scanner.rs` control). |
| canister **N-2**: a pinned delivery, then refund, then LostRace | Medium | **Fixed, bounded** (accepted residual) | R2's own "no replacement ever accepted" mempool re-run (`audit3_pin.rs`): 4,500 replacements refused and **0 attestations**. The gated Wrapper refuses the refund at deliverBy + 72 h. A LostRace now needs the pin held 30 days: under TRUC, a market of roughly 82 sat/vB or more and at least 75k sats of child fee at risk. M-1(b) (re-price) and M-1(c) (paused) are fixed: a pause from day 5 to day 20 still conflicts out, and the attestation goes only after the resume. N-12 is fixed. |

Round-2 Lows (contracts N-2, N-3; canister N-3 to N-7): all re-run and fixed. The contract PoCs `R2Draws` (`bounded: 200 >= 200`,
`a new snapshot time`) and `R2Curve` (`the close gate still says yes while paused`) now fail. N-3's hold has the residual A3-L1.

## 2. New findings

### Medium

#### A3-M1: a keeper pause lets an outsider who minted on a fallen floor burn at the higher average backing, at holders' expense

- **Where:** `src/floor/FloorModule.sol:494-501` (`_tierPrices`: `exitAtBacking()` makes burns pay `β × (1 − base)`) and `:985-986`
  (`exitAtBacking = _keeperDown() || escaped()`). Before round 3 this price was reachable only after 7 days stale.
- **Mechanism:**
  - β, the backing per unit, is the average of everything paid in.
  - When the floor falls (honestly, the keeper following the market), `mintPrice ≈ floor × (1 + s)` drops below β.
  - A fresh round trip is unprofitable, because a fresh burn pays `min(price, target)`.
  - But the moment the keeper is paused, a burn pays β less 0.15%. Mints are refused during the pause, so the attacker must be
    positioned **before** it. Holding costs nothing but floor exposure; if no pause comes, the exit is at the spread.
- **PoC:** `test/audit3/Audit3Floor.t.sol::test_audit3_mintLowThenBurnAtBackingDuringAPause`, **PASS (attack works)**, `FloorBase` test
  params (the Phase-1 values except `reserveBps` 0; the reserve tier is capped the same way, at its own average basis). The floor falls 20% within the day band. The attacker mints 4 units, then a guardian pause:

  | Measure | Value |
  | --- | --- |
  | Attacker paid | 2.0469 ETH |
  | Burned during the pause for | 2.2530 ETH |
  | **Profit** | **+0.206 ETH (+10%)** |
  | Existing holder's backing per unit | 0.6339 → 0.5641 (−0.21 ETH on 3 units) |

  The fresh-state control shows no profit.
- **Who can cause the pause:** the Safe, the guardian, and **the monitor automatically**. Its fail-closed deadline pauses on any
  draw not verified before its pin, and outsiders can cause that (services M-A, M-B, L-F). The design note (fixes-R3-contracts
  §1.2) says a false positive leaves "the holders' backing per unit untouched". The PoC shows that the value comes from the existing
  holders' cushion above the floor.
- **Size:** bounded per pause by `mintPerHour` and `publicCap` at positioning time (10 units of public cap at stage 0; ~10% of the
  minted ETH at a 20% fall), and by `burnPerHour` during the pause. It repeats with every pause.
- **Fix (either):**
  - While paused, price burns at `min(β, max(acceptedAtPause, dayLow)) × (1 − base)`. The accepted floor cannot move while paused,
    because nobody can post. This keeps N-1b's liveness without the arbitrage. Keep β only for `escaped()`, after 7 days.
  - Or keep a per-unit cost basis and cap each burn at its own mint price. Heavier.

  Also make the monitor's automatic pause harder to trigger falsely (M-A, M-B, L-F).

#### M-A: the holder index's page span only ever shrinks, so transient errors leave it at 1 block a call for good

- **Where:** `monitor/src/holders.mjs:69,76`, which also serves the canary.
- **PoC:** `monitor/test/audit3_monitor.test.mjs` test 1, **PASS**. After 13 errors that have nothing to do with the range (header
  not found, HTTP 400/403), the ceiling is 1 block. 600 new blocks then cost 600 calls, against a 25 s budget.
- **Impact:** the index never catches up. Draws are verified by one provider only, or, if drpc also lags, the keeper is paused at
  each deadline. With A3-M1 that pause is a profit lever.
- **Fix:**
  - Lower the ceiling only on range errors ("exceeds", "too many", "range", "limit").
  - Back off, without halving, on any other error.
  - Let the ceiling grow back after successes.

#### M-B: one dense block stalls a provider's shared group cursor for every coin, persisted on disk

- **Where:** `monitor/src/holders.mjs:185-193`.
- **PoC:** `audit3_monitor` test 2, **PASS**. The group and an unrelated coin both stop before the dense block, and `rootAt` throws
  "behind the snapshot block".
- **Impact:** every later draw is unverified, and the keeper is paused at every deadline, repeatably, until the index file is
  edited.
- **Cost:** borderline on Robinhood's 10,000-log cap, about 0.001 ETH a block. It is cheap on any provider with a lower cap
  (drpc's cap was not measured). A filter with more addresses than a provider accepts fails the same way.
- **Fix:**
  - On a 1-block failure, split by address, then by topic.
  - Advance every other coin; mark only the dense coin behind.
  - Page SEV2.

#### M-E: `pinnedHttpsFetch` never settles on `HTTP 101`, and `guardedFetch`'s deadline does not rescue it

- **Where:** `indexer/src/outbound.ts:194-221`; `wrapped/service.ts:74-83` (the shared `inflight`).
- **PoC:** `indexer/test/audit3_outbound.test.ts` test 5, **PASS**: still pending after 10 s with a 3 s timeout.
- **Impact:** one hostile draw URI hangs every API call for its collection for good (tickets, refund dates, draws). A hostile
  tokenURI or marketplace host freezes the catalogue refresh.
- **Fix:**
  - Handle `upgrade` and `close` (reject if not settled).
  - Race the transport against the deadline.
  - Put a timeout on `inflight`.

### Low

| ID | Title | Where | PoC | Fix |
| --- | --- | --- | --- | --- |
| **A3-L1** | The N-3 hold ends the instant the Safe unpauses, but the floor is then stale (a pause always outlasts `FLOOR_TTL`, and nobody can post while paused), so `fill()` reverts `NoRoom`. Anyone calling `openRefunds()` before the keeper's first post plus fill binds a held grad to refund, the outcome N-3 was meant to prevent. In the limit, a back-run of the unpause. | `src/curve/FrogCurve.sol:392-393`; `FloorModule.holdsRefunds` `:719` | `test/audit3/Audit3Curve.t.sol::test_audit3_unpauseRace_refundBeatsFill`: **PASS (attack works)**. Refunding; the grad Cancelled | `holdsRefunds()` also true while `!fresh()` for a short grace after a pause ends (store `unpausedAt`), or extend the window by the paused duration plus `FLOOR_TTL`. Bounded by `_stuck()` as now. |
| **R3-C1** | After a bisected dense range the ticket-log scanner walks the rest of `hot` one block a page: a 500-event block at a page's start delays a ticket 7,990 blocks later by about 6.9 days, and every delivery behind it refunds. Just short of reachable at stage-3 caps (~370 kB against the 400 kB retry). | `autobuyer/canister-rs/src/ticketlog.rs:235-239, 251-254` | `audit3_scanner.rs`: **PASS** (gap present) | Double the span inside `hot` too; clear `hot` once the dense block is read. |
| **R3-C2** | One provider alone can make the scanner give up a chosen ticket's events. A size-looking error substring in its JSON-RPC message leads to the dense path, then `unreadable` after 3 tries, then the cursor passes the block, although the other provider served it. This bypasses "providers must agree". The ticket refunds (loud SEV1); no backing is lost. | `ticketlog.rs:262-271, 323-330`; `evmread.rs:174-177` | `audit3_scanner.rs`: **PASS** | Treat a size error as such only on the IC's own reject; require it from every provider; retry `unreadable` later. |
| **L-C** | The floor-walk edge-run rule is evaded by a no-move post 1 s after every second edge post. `pause_on_floor_walk` never fires, and a walk under ~10% per 23 h pages nothing (−50% in a week). | `monitor/src/evm.mjs:156-181` | `audit3_monitor` tests 3, 4: **PASS** | A sliding window of edge posts, or cumulative one-way movement over 6/24/72 h; a trigger on the day-move SEV1. |
| **L-F** | The holder index never self-repairs. One silently dropped log makes the coin "disagree" for good, and the deadline treats disagree as a pause. `resets`/`broken` are persisted and never cleared. | `holders.mjs:208-229` | `audit3_monitor` test 5: **PASS** | Rebuild the coin on a disagreement before the deadline; cross-check `totalSupply`; expire the flags. |
| **L-D** | The resolver caches one mismatched chain rebuild as "no entry" for the life of the process: no claim, a void after 7 d. The audit2 regression for it is vacuous (0 == 0). | `keeper/service/resolver.ts:186,192-193` | reasoned | Cache only matched rebuilds; retry with backoff. |
| **L-G** | Alerts that last one pass (`draw-slot-missed`, `draw-not-opened`) or flap never reach 3 passes, so they are never paged. | `keeper/service/keeper.ts:286-292`, `main.ts` | reasoned | Latch one-shot alerts for N passes, or count kinds over a window. |
| **L-H** | golive-check's "real Safe" check is defeatable by whoever ran `setup()`: a delegatecall can plant unlinked owners or modules. The fallback handler is not read. `--safe-owners` is optional under `--phase1`. | `script/golive-check.sh:119-147` | reasoned | Require `--safe-owners` under `--phase1`; read the fallback handler; check the Safe's creation tx (`setup` with `to = 0`). |

### Info

- **A3-I1** (contracts): a keeper that posts at confidence 0, or stops, still freezes burns for up to 7 d 1 h until someone pauses
  it. The pause now ends it at once (`test_audit3_confidenceZeroPostsFreezeBurnsUntilAPause`, PASS). This is the documented
  "ordinary outage". It is a reason for the monitor to pause on `no-good-post` sooner, which in turn needs A3-M1 fixed.
- **A3-I2** (contracts): the sentinel-only check applies only to `totalTickets == 0`. A keeper can open a carried draw with
  `totalTickets = N > 0` and `root = leaf(0, N, at)`, with an empty proof. `MerkleProof` accepts a proof of length 0 when
  `leaf == root`. The pin then consumes the carry, nobody can claim, the void comes after 7 d, and the next draw gets a new seed
  (`test_audit3_sentinelOnlyNonEmptyRootConsumesTheCarry`, PASS). It is a bad root like any other, which the monitor recomputes and
  cancels. It is dominated by a forged root paying the keeper, so this is not a new power. Make sure the monitor pages it as bad (it
  does: root mismatch). Optionally require `sentinelProof.length > 0` when `totalTickets > 0`.
- **R3-C3** to **R3-C7** (canister/timelock), in `part-canister.md`:
  - R3-C3: the paused pass skips `assert_chain`.
  - R3-C4: fee-pool quarantine is counted per choice, not per tick.
  - R3-C5: the timelock `running` entry is orphaned if `execute` traps after an await, which blocks `start_target` for good; a veto
    during `install_code` is recorded but not honoured.
  - R3-C6: `markUndeliverable` is never re-priced, and it blocks `recordDelivery`.
  - R3-C7: conflicts still escalate to 100k sats.
- **Services I-1 to I-3**, in `part-services.md`:
  - I-1: the deadline is skipped when `checkEvm` throws early.
  - I-2: with a quorum of 2, one lying provider plus one lagging provider suppresses the pause.
  - I-3: an untracked empty close pages SEV2 instead of SEV1.

### Checked, no finding

**Contracts:**
- `_burnTarget` bounds: a burn never pays more than `min(price, β)`, and a round trip is never profitable while fresh.
- B-07 `_gradRoom`.
- `hasRoomFor` false while paused, `holdsRefunds` bounded by `_stuck()`: no curve can be stuck forever.
- `requestDraw` of an empty draw: `drawInFlight`, the carry and the reserved unit are all restored. `Empty` is final.
- The carry's `seedOrigin` never chains.
- A carried seed round is already public, but the snapshot time was fixed by a round unseen at the origin's announcement.
- Guardian `SameKey` covers owner, pending owner, keeper, pending keeper, purchaser, attestor, treasury, pending treasury and
  auto-buyers, both ways (`setGuardian`, `proposeRoles`, `transferOwnership`, `proposeTreasury`, `_listAuto`, `reserveBuyer`).
  `setGuardian(0)` followed by rotating to the old guardian is allowed by design.
- `markUndeliverable`: custodian and Resolved only. `recordDelivery` stays possible after a mark, so a mark never races a delivery.

**Canister:** the attestation is sent only when no version can confirm (fee input spent 6 deep, or nothing ever sent), never while
paused, and a stranger cannot trigger it. TRUC inputs are confirmed-only. Fee caps are enforced in `build` and H4. The upper-median
rule is correct. Timelock untake, veto and withdraw of running proposals are correct.

**Services:** the resolver's proof checks; the keeper's announce guard and carried-seed handling; the empty-draw verdicts;
`GOLIVE_REHEARSAL` gated by chain id; the site's `requestRefusal` coin binding; `/healthz` 503 not forceable by outsiders.

## 3. Suite results (this machine, on `fixes-2026-09-26` plus the untracked audit3 files)

| Suite | Result |
| --- | --- |
| `forge test` (as shipped) | **1031 passed, 0 failed** (116 suites; the fork suites return early without `--fork-url`) |
| `forge test --match-path 'test/audit3/*'` | 5 passed (the A3-M1 and A3-L1 PoCs show the gap; the N-2 re-run shows it is closed; A3-I1 and A3-I2 as described) |
| R2 contract PoCs, unmodified (`R2Floor`, `R2Curve`, `R2Draws`) | 5/5 now **fail** on their attack assertions (so the attacks are closed) |
| canister `cargo test` | **478 passed, 0 failed**; plus the 4 audit3 tests: 482 passed |
| timelock `cargo test` / `--features rehearsal` | **12 / 12 passed** |
| keeper `npm test` / tsc | 131 pass / clean |
| monitor `npm test` | 106 pass (111 with the 5 audit3 PoCs) |
| ops `npm test` / typecheck | 31 pass / clean |
| indexer `npm test` / typecheck | 123 pass / clean (+5 audit3) |
| web-rook `npm test` / typecheck / both builds | 65 pass / clean / ok |
| tools/snapshot `npm test` / tsc | 19 pass / clean |
| staging `npm test` / typecheck | 29 pass / clean |
| web `npm test` / typecheck | 118 pass / clean |
| `scripts/timelock-watch.test.mjs` | 4 pass, 1 skipped (no local rehearsal timelock) |
| `scripts/audit2_golive_check.test.sh` | 39 ok |
| `STACK_PORT=8697 scripts/golive-dryrun.sh stack` (fork of the public 4663 RPC, real drand) | **DRY RUN: ALL PASS** (exit 0): launch → graduation → module mint → sweep → draw announced → opened with the real evmnet seed signature → pinned → resolved (round 20965091) → one whole wX claimed; the golive-check `--phase1` read passes and its refusals behave; the only "FAIL" in the log is a section title. Log `R3/dryrun-stack.log` |
| `script/deploy.test.sh` | **37 ok**, exit 0 (`R3/deploy-test.log`) |

Not run: the `keeper-fork-test.sh` fork run, a replica or fork run of the scanner against the real providers, the signet TRUC relay,
the canister's reproducible Docker build, clippy/audit, and a fork run driving the keeper and the monitor together. The fix docs list
all of these as open go-live gates.
