# Round 5, part A: the exit rule (A4-L-1) and the operator's option (c), go-live checks and the launch configuration

Reviewer: independent AI re-review, 2026-09-28. Worktree `/Users/jlohayon/Documents/Rook-nft`, branch `nft-side` at `1dc2a61`
(fix round 4, `2735dac..1dc2a61`). Scope:

- A4-L-1: `canister.rs` `validate_custody`, `exit_room`, `caps_of`, `check_cap_change`; `outflow.rs` `exit_fits`, the limiter; the
  timelock's delay and queue rules (`autobuyer/timelock-rs/src/rules.rs`, `canister.rs`); `script/golive-check.sh` `EXIT_PY`,
  `exitcheck`, `launchexit`, `canexit`; `scripts/golive-dryrun.sh`; `deployments/launch/nft-pilot.json`.
- The operator's choice, received during the review: **option (c)**, installed at 48 items (13 / 17 / 13 / 3 / 2) at 12 / 24 a
  day, the `global_items` raise to at least 31 queued at launch, and J7's 61 about two weeks later (a dedicated section below).
- A4-L-3 (the keyless rule and the per-chain host count), A3-L-3 (D8 references, placeholders), A4-I-2 (outflow as data).
- The launch configuration against `phase1-nft.md`, `phase1-nft-pooled.md`, `phase1Trust.ts`, `launch-gates.md`, the canister.
- The contracts: `git diff 2735dac..HEAD -- src/ test/` is **empty** (no commit touches them).

**Nothing in the worktree was modified except this report and `review5/poc/A/`; nothing committed, pushed, deployed or signed.**
Every PoC ran in a private copy under `/tmp/rook-r5/A/` (APFS clones of `autobuyer/`, `script/`, `scripts/`, `deployments/`,
`monitor/src` and `monitor/node_modules`). The first copy was deleted mid-review by another reviewer's cleanup; the copy was
re-made, checked with `diff -rq` against the worktree (identical but for the appended PoC modules), and every PoC was re-run on it.
The saved outputs are from that re-run. No anvil was needed. The private copy was removed at the end.

Severity scale: the README's.

## Verdict

**The round-4 check is right as arithmetic, and the two implementations agree.** The canister's `validate_custody` and golive-check's
`EXIT_PY` gave the same verdict on all 392 grid rows that reach the exit rule. The grid covered exactly 48, the global cap binding,
zero caps, a collection at 0, per-collection and global limits of 0, and rounding at the edges. The canister refuses J7's 61 items at
24 a day, at install and at upgrade. golive-check refuses the same pair on the launch file and on the canister's
`custody_caps()` / `custody_outflow()`.

**Option (c) is safe as a sequence of configurations, and the canister enforces its order.** Every upgrade argument is a whole
`Init`, checked at execute time against the new pair and the ×4 rule. A caps-first upgrade is refused; outflow first, then caps,
is accepted. One upgrade that does both at once is also accepted, and it fits the rule. The timelock cannot reorder the two steps:
once any target action executes, even a refused one, every older pending proposal is stale. No path skips the 72 h delay.

**But the rule the check enforces is weaker than the promise it backs.** Two new Lows sit under every A4-L-1 option:

- **A5-L-1.** The rule is checked on the caps an argument names, never on what custody holds. One upgrade that lowers the caps
  and the outflow together passes, while custody still holds more than the new limits can let out.
- **A5-L-2.** "Each collection alone fits, and the total fits" does not ensure the items leave in 2 days under the canister's
  deadline-first delivery order. When other collections' tickets resolve first, Azuki's items are left for the margin day in every
  option:

  | Option | Azukis left for the margin day |
  | --- | --- |
  | (a) at install | 5 |
  | (c) interim | 3 |
  | (c) end state | 9 |
  | (b) | 8 |
  | (d) | 1 |

  A backlog-first delivery order closes it with no configuration change.

**Recommendation: (c), as the operator chose, with two conditions:**

1. Fix A5-L-2 before the caps go back to 61. The fix is backlog-first delivery (preferred, no configuration change). Without it,
   raise `per_collection_items` to at least 22 alongside `global_items` 31, or raise `global_items` to at least 42.
2. Make the outflow-raise proposal an outflow-only diff, and have golive-check and the monitor read the pending proposal.

**Launch configuration: not yet go.** The committed `nft-pilot.json` still says J7's 61. The go/no-go read therefore FAILs on it,
as designed, until the operator commits the 48-item split. The Trust page, `launch-gates.md` G7, `phase1-nft.md` §5 and
`phase1-nft-pooled.md` §6 J7 all still say "the operator chooses".

**A4-L-3 is partly closed (A5-L-3).** A full 32-character key split into 11-character path segments passes as "all keyless". So
does a key in a `wss://` URL. The new host-count FAIL line prints a key subdomain in full. A trailing-dot host counts as a second
provider.

**A3-L-3 is closed** (swapped D8 references are refused); placeholder gaps remain as an Info.

## Counts (this part: new, plus earlier items in scope not fully fixed)

| Critical | High | Medium | Low | Info |
| --- | --- | --- | --- | --- |
| 0 | 0 | 0 | 3 new (A5-L-1..3) + 1 carried (A-L-2) | 5 new (A5-I-1..5) |

## Round-4 items in scope

| ID | Sev (R4) | Verdict | How it was re-attacked |
| --- | --- | --- | --- |
| **A4-L-1** | Low | **The check: fixed and correct.** The rule it enforces has two gaps: A5-L-1 (held custody vs caps) and A5-L-2 (delivery order) | `poc/A/r5a_exit_rule_append_to_canister_rs.rs`: a grid of 720 rows through the real `validate_custody`, run by `exit_agreement.py` through golive-check's `EXIT_PY` (extracted unchanged). 392 exit-rule rows: 392 agree, 0 disagree. The 328 other rows are refused by the canister before the exit rule (a collection over the global cap, per > global, an item limit of 0). golive-check does not re-validate those rows, and the canister refuses them anyway. Also checked: the default install's `caps_of` global (`min(sum, max(room, largest))`, 12 at stage 0), and the canister's error text (`div_ceil(total, days)`: 31) |
| **A4-L-3** | Low | **Partly closed (A5-L-3)** | `poc/A/install_check.sh`: real buyer `Init`s. `INSTALL_PY` and the `installchains` decoder are extracted unchanged and run as `installcheck` runs them (`GOLIVE_PHASE1=1`, argv[3] = the decoded chains), 15 cases |
| **A3-L-3** | Low (carried) | **Closed**; residual A5-I-3 (Info) | The D8 references are held per collection against `openseaSlug` / `coingeckoId` (`golive-check.sh:591-604`); the dry run's swap refusal fires. `poc/A/placeholder_check.py`: `TODO_RUNNER`, `YOUR_RUNNER_HOST`, `PLACEHOLDER`, `{{RUNNER}}` pass |
| **A4-I-2** | Info | **Partly** (A5-I-1) | `outflow` is data in the launch file, and each pair is held to the exit rule. The launch file's outflow is never compared with the canister's `custody_outflow()`: not the item limits, the mints, the 25 ETH or the trip factor. `custodyGlobalItems` is read by no other check |
| **A-L-2** | Low (carried) | **Unchanged** (contracts untouched) | `EthNftWrapper.deployer` is still the deploy EOA (`EthNftWrapper.sol:184`). golive-check now holds `maxMintsPerDay` (`:1017`) and a pending raise (`:984`), not `deployer()` for NFT wrappers. It stays Low |
| src/, test/ | — | **Unchanged** since `2735dac` | `git diff --quiet 2735dac..HEAD -- src/ test/` → no change; no commit in the range touches them |

## The A4-L-1 options (a)–(d), with a recommendation

Common facts (all proven in `poc/A/r5a_exit_rule.output.txt`):

- The canister's install check refuses J7 at 12 / 24 and accepts 48 at 12 / 24.
- An install may not start `global_items` above 24 (×4 of stage 0's 6), so 31 is refused at install.
- At upgrade, `global_items` may go 24 → 96 in one step, but not to 97. `per_collection_items` may go 12 → 48.
- A pair that does not fit (61 at 24) is refused whether it arrives by install or by upgrade. Lowering the outflow under 61 held
  caps is refused.
- The global item tripwire (3 × `global_items`) exceeds anything custody can hold under every option: 72 or 93 against 48 or 61.
  So the tripwire never fires from deliveries of held items (A5-I-5).

| Option | Canister | golive-check | Safety | Verdict |
| --- | --- | --- | --- | --- |
| **(a)** item caps summing to 48 at 24 a day (13 / 17 / 13 / 3 / 2) | Accepts | Launch-file and canister reads pass (the dry run shows it) | Fits the rule. Under the deadline-first order the worst redemption order leaves 5 Azukis for the margin day (A5-L-2). Every collection keeps 15 ETH of value cap; fewer items fit in it (Milady 12.1, Azuki 11.4, MAYC 11.4, Pudgy 10.1 ETH at the 2026-09-27 floors). BAYC's 2 are unchanged. Deposits (D5) are bounded by the smaller caps. Nothing in H3, J1 or the 25 ETH depends on the item counts | **Safe as a launch state.** Capacity drops by 13 items |
| **(b)** global cap 48 over J7's per-collection caps | Accepts (`install (b) … ACCEPTED`) | **Refused by golive-check's own J7 read**: "a total that binds before the collections' own caps (J7: no total)" (`:410-417`; `poc/A/option_b_golive.output.txt`). golive-check prints (b) as an option but refuses it (A5-I-2) | Fits the rule. A binding total lets one collection crowd the others out (Azuki 22 + Milady 16 + MAYC 10 leaves Pudgy and BAYC no room), which J7 rejected. Worst order: 8 Azukis for the margin day | **Not recommended.** It changes J7 and fails the go/no-go read as written |
| **(c)** install at (a), raise `global_items` to ≥ 31, then caps back to 61 | Enforces the order: caps-first refused; outflow-first then caps accepted; both in one upgrade accepted; lowering the outflow under 61 refused | Each step's state passes on the canister read. The launch file must be edited per step, and the pending proposal is read by nothing (A5-I-4) | Every intermediate configuration fits the rule, and no upgrade can take the configuration out of it. Raising the limit to 31 a day: day-1 exposure of a compromised signing path goes from about 43 to 49 ETH of items at the 2026-09-27 floors (BAYC and Pudgy leave on day 1 under every option). "Nothing legitimate empties custody in a day" still holds (31 < 48, 31 < 61). The per-collection 12 a day is unchanged. The end state leaves 9 Azukis for the margin day under the worst order (A5-L-2) | **Recommended**, with A5-L-2 fixed before step 2, or step 1 raising `per_collection_items` to ≥ 22 (or `global_items` to ≥ 42) |
| **(d)** a longer delay (96 h) | Needs a code change (`ASSUMED_DELAY_SECS`), which golive-check's hard-coded `days = 2` (`:667`) must follow. The timelock's `delay_secs` can already be set above the floor at install without a code change. But the buyer cannot read its controller's delay, so its assumption must equal a floor that every timelock build enforces: `MIN_DELAY_SECS` raised too, a new timelock module hash and a timelock migration. The rehearsal build's floor is 10 s | — | J7 at 24 a day with 3 usable days leaves 1 Azuki for the margin day under the worst order (A5-L-2). Every upgrade, security fixes included, waits 96 h | **Not recommended.** It changes two canisters and the timelock for one item of margin; (c) gets there by configuration |

**Recommendation: (c).** Two conditions:

1. **A5-L-2 fixed** (backlog-first delivery) before the caps go back to 61. As a configuration-only alternative, step 1 raises
   `per_collection_items` to 22 with `global_items` 31. With the per-collection limit at the largest collection, any
   work-conserving order is optimal (checked: nothing left under any order tried). The other alternative is `global_items` 42.
2. **The pending proposal read before the guardians let it run** (A5-I-4). The outflow raise must diff as `outflow_limits` only,
   with custody caps unchanged. The monitor's `argdiff` already prints that list.

## Option (c) as the operator chose it (2026-09-28)

The plan: install at 48 items (Milady 13, Azuki 17, MAYC 13, Pudgy 3, BAYC 2) at 12 / 24 a day. Queue the timelocked raise of
`global_items` (≥ 31) at launch, so it lands after 72 h. About two weeks later, if clean, raise the caps to J7's 61.

### 1. Is the install at 48 safe, and is it enforced?

- **Safe as configuration:** 48 items at 24 a day fit the canister's rule exactly (`install (a) 48 @ 24/day: ACCEPTED`). Each
  collection fits alone (Azuki 17 at 12 a day).
- **Under the worst redemption order** (Azuki's tickets resolved after the others'), 43 of 48 leave in the two usable days. The
  remaining 5 Azukis leave at the 48 h mark, inside the margin day and before a 72 h upgrade lands (A5-L-2).
- **The canister enforces it at install:**
  - `validate_custody` panics in `init` on 61 at 24;
  - it refuses `global_items` above 24 (the ×4 rule from stage 0);
  - it refuses a collection above 24 items (`FIRST_EVM_MAX_ITEMS`).
- **golive-check and the dry run:**
  - A launch file with 48 passes on both reads. The dry run's generated `launch.fits.json` is exactly 13 / 17 / 13 / 3 / 2, and its
    go/no-go read passes.
  - A file with 61 is refused on both, the launch file (`launchexit`) and the canister's pair (`canexit`). The dry run shows both
    refusals.
  - **But the committed `deployments/launch/nft-pilot.json` still names J7's 61.** Until the operator commits the 48 split, the
    real go/no-go read FAILs. That is the intended fail-closed, and a launch step to do.
  - `capcheck` then holds each collection's canister cap to the file's `custodyItems`. The J7 total read requires the canister's
    global cap to be at least 48, which the sum is.
- **Not enforced:** that the launch file's outflow is the canister's (A5-I-1). A file saying 12 / 31 beside a canister at 12 / 24
  passes both reads, each pair on its own. So does a canister installed with `trip_factor` 12 or `global_value_wei` 100 ETH (both
  within ×4 of stage 0): the go/no-go read never compares them with the launch decision.

### 2. Does the upgrade check enforce outflow first, caps second?

`post_upgrade` → `validate(&arg)` → `check_cap_change(saved, kept)` → `validate_custody(kept)` (`canister.rs:302-324, 502, 555-563`).
It runs at execute time, on the argument as executed, against the saved `Init` at that moment. A refusal traps, and the IC's install
is atomic.

| Case | Result (`poc/A/r5a_exit_rule.output.txt`) |
| --- | --- |
| A single upgrade raising both (48 @ 24 → 61 @ 31) | **Accepted.** The check reads the new pair only, and 61 at 31 fits. Safe by the rule; it skips the operator's two-week gate, which only the proposal's diff shows |
| A caps-first upgrade (48 @ 24 → 61 @ 24) | **Refused** ("custody holds up to 61 items in all and lets 24 out a day …") |
| Outflow first (→ 48 @ 31), then caps (→ 61 @ 31) | **Both accepted** |
| Lowering the outflow later while the caps say 61 (61 @ 31 → 61 @ 24) | **Refused** |
| Checked against the new outflow or the old? | **The new** (`limits_of(kept)`). The ×4 rule compares the new with the saved |
| ×4 bound: 24 → 96 in one step, 31 reachable? | 24 → 96 accepted, 97 refused; 31 is one step. `per_collection_items` 12 → 22 is also one step |
| Custody already held above a new, lower cap | **Not handled (A5-L-1).** Lowering is always allowed, and the exit check never reads the book. 61 @ 31 → caps 12 @ 6 a day is accepted while custody holds 61; then 12 of 61 can leave in the next proposal's usable days |

**Could the caps ever exceed what the outflow can move?** As configuration, no: no install or upgrade can leave a named pair outside
the rule. As state, yes, in two ways:

- custody can hold more than a lowered cap (A5-L-1);
- the rule's two-part test does not cover the canister's delivery order (A5-L-2).

### 3. The queue plan against the timelock (`poc/A/r5a_timelock_queue.output.txt`)

- **Queuing the raise right after the install is allowed.** Any proposer with no pending proposal of its own can propose; it runs
  72 h later (`MIN_DELAY_SECS`, the production floor), by anyone's `execute`. It is safe: the raise's own argument is checked when
  it executes.
- **The launch install must execute first.** If the launch install is itself a timelocked upgrade, a raise queued beside it goes
  stale when the install executes (`target_epoch`), and must be queued again after it. If the install is the pre-handover
  reinstall (the operator's own `install_code`, not a timelock action), a raise queued after the handover is live.
- **Args computed at queue time against stale state:** the argument is fixed when proposed, but it cannot land on a state other
  than the one it was built for. Any target action executed after it was proposed makes it stale: an upgrade, a controllers
  change, or a refused upgrade, since `take_ready` bumps the epoch before the install and a failure keeps it moved
  (`timelock canister.rs:255-262`). So its `saved` at execute time is the state at queue time, bar the canister's own runtime state
  (the book, the limiter), which the check does not read (A5-L-1).
- **Reordering, case by case:**
  - A caps upgrade queued later but ripe first cannot run before the raise's 72 h. Whichever executes first makes the other stale
    (shown both ways).
  - A caps-first proposal that runs first is refused by the canister, and it still makes the raise stale: the raise waits another
    72 h. This costs liveness only, and a proposer can already do it with any executed upgrade.
  - The same proposer cannot queue the caps raise while the outflow raise is pending ("already has proposal 1 pending").
  - **A full `Init` carrying caps is the real reordering risk.** Every upgrade argument names the caps. An outflow-raise argument
    built from the committed `nft-pilot.json` (still 61) would raise both at once. The canister accepts that, since it fits, and
    the two-week gate would be skipped. The guardians' only signal is the monitor's `argdiff`: "custody cap … max_items 13 → 16
    LOOSENS" CRITICAL lines beside the outflow line.
  - No emergency path skips the delay. `pause_target` (one guardian), `start_target` and `resume_target` (k guardians) install
    nothing; the only actions are `Upgrade`, `Controllers` and `Roles`, all delayed.
  - A proposal withdrawn and re-queued waits the full delay again (withdrawn at 10 h, runnable at 82 h).
- **The rehearsal timelock** has a 10 s floor. The buyer's exit rule assumes the production 72 h, and golive-check does not read
  the controller's delay (the monitor's timelock watch is the check).

### 4. What is promised, and what holds, in the 72 h window and the two weeks after

- **Trust page** (`phase1Trust.ts:20`):
  - It still says "16 Miladys, 22 Azukis …", "24 a day in all", and "the 61 items above do not fit yet … this page will state the
    numbers installed". At launch it must say 13 / 17 / 13 / 3 / 2, 12 / 24.
  - During the raise's 72 h it must say that a raise to 31 is queued. After it lands, 31. After the caps raise, 61.
  - "Every item must be able to leave within the timelock's delay less a day" holds as configuration at every step, but not under
    the worst redemption order (A5-L-2) and not after a lowering (A5-L-1).
  - "a tripwire at three times any of those" never fires for deliveries of held items at any of these configurations (A5-I-5).
- **`launch-gates.md` G7 row 0** (`:322`) and the site's copy say "the operator chooses". Row 1 (`:323`) caps an Ethereum
  collection at 12 items and outflow at 6 / 12, **below** the launch's J7 stage 0. "Stage 1" by that table is a lowering. `ops/src/stage.ts` instead scales J7's 24
  items ×4 a stage for Ethereum collections, so the table and the reconciler disagree (A5-I-4).
- **The monitor:**
  - `argdiff` pages the planned raise as **CRITICAL / SEV1 "LOOSENS"** ("outflow limit: items out a day, in all 24 → 31"). The
    runbook must tell the guardians it is expected and what an outflow-only diff looks like.
  - `argdiff` does not evaluate the exit rule on a proposal: a caps-first proposal the canister will refuse is shown as an ordinary
    loosening. It does annotate refusals for the ×4 rule.
  - After execution, `watcher.mjs` compares the running `custody_outflow()` limits with the baseline argument, so the new 31 is
    held once the executed argument is the baseline.

### 5. Runbook, golive-check and dry-run changes needed for (c)

1. **Commit the 48 split** to `nft-pilot.json` (`custodyItems` 13 / 17 / 13 / 3 / 2, `outflow` 12 / 24). Update the Trust page,
   G7, `phase1-nft.md` §5 and `phase1-nft-pooled.md` §6 J7. The dry run should read the committed file, not a generated one.
2. **golive-check `--proposal <timelock id>`** (or a `--pending` mode):
   - decode the pending upgrade's argument with the monitor's IDL (as `installchains` does);
   - diff it against the running `Init`, and require that only `outflow_limits` moved for step 1 and only `custody_caps` for
     step 2;
   - run the exit rule on its pair, the robust form of A5-L-2 included;
   - hold its outflow against the launch file's next stage.
3. **Stage the launch file:** an `outflowNext` / `custodyItemsNext`, or one launch file per step. Also:
   - hold the canister's whole `custody_outflow()` against the file (A5-I-1);
   - keep the J7 total read, so the caps raise must name a global of at least 61.
4. **Monitor `argdiff`:** annotate a proposal whose pair fails the exit rule ("the running code refuses it, so executing it
   fails"), as it does for ×4. For a lowering of caps or outflow, compare with `custody_caps()` headroom (items held) and page if
   held custody would not fit (A5-L-1).
5. **Runbook:**
   - queue the raise only after the launch install has executed and the handover is done;
   - no other target proposal in the 72 h, since any executed upgrade makes the raise stale;
   - if a security upgrade must go first, re-queue the raise after it;
   - the caps raise is a separate proposal after two clean weeks, and its argument must carry `global_items` ≥ 31, since it is a
     whole `Init`.

## The launch configuration

| Decision | Flag |
| --- | --- |
| Five collections, BAYC 10 ETH / conv 1,000 / reserve 0 (J1) | Unchanged since round 4 (contracts untouched; `nft-pilot.json` BAYC values unchanged). OK |
| J7 per-collection caps 16 / 22 / 17 / 4 / 2, 15 ETH each | Refused by the canister at 12 / 24, as it should be. The operator's (c) installs 13 / 17 / 13 / 3 / 2 first. **The committed file is still J7's** (go/no-go FAIL until changed) |
| Deposits on (D5) | Bounded by the smaller caps under (a) / (c). New mints during a bank run are limited to 6 a day and are served after the earlier tickets (deadline order), so they do not take earlier redeemers' slots |
| H3: 25 ETH a day, trip 3, 12 / 24 items, mints 3 / 6 | Consistent with (c). The launch file's values are data but not held against the canister (A5-I-1). The item tripwire cannot fire from held items (A5-I-5) |
| Timelock 72 h | The canister's `ASSUMED_DELAY_SECS` equals the production floor. OK |
| D1–D9 | D8 references held per collection (A3-L-3 closed). D6 / keyless canister: A5-L-3 residuals. D1 / D4 stale text marked overtaken in round 4 |
| Docs and site | The Trust page, G7 row 0, `phase1-nft.md` §5 and `phase1-nft-pooled.md` §6 J7 say "the operator chooses"; G7 row 1 sits below J7's stage 0 (A5-I-4). No statement contradicts the canister's defaults: `caps_of` with no caps gives 6 items per Ethereum collection and a global of 12 at stage 0's 6 a day, as `fixes-round4.md` §9 item 8 says |

## Findings

### A5-L-1 (Low): the exit rule is checked on the caps an argument names, never on what custody holds

- **Code:**
  - `autobuyer/canister-rs/src/canister.rs:302-324` (`validate_custody` reads `caps_of(i)` and `limits_of(i)` only);
  - `:555-563` (`check_cap_change`; lowering is always allowed, `custodycap.rs:132-140` `raise_ok`);
  - `:502` versus `:507` (the upgrade check runs before, and without, the restored `CUSTODY_BOOK`);
  - `monitor/src/argdiff.mjs:196-218` (a lowering of caps or outflow is an INFO "TIGHTENS").
- **Impact:**
  - With 61 items held at (c)'s end state (61 @ 12 / 31), one upgrade to caps 3 / 3 / 3 / 2 / 1 and outflow 3 / 6 a day is
    accepted, and the monitor labels it a tightening.
  - After it lands, custody still holds 61 items and lets 6 a day out. When the next, malicious, proposal appears, **12 of 61** can
    leave within its usable days (the real limiter, `poc/A` "held 61, limits 3/6 a day: 12 leave within 48 h, 49 cannot").
  - The first proposal looks protective ("back to stage 0"), so holders have no reason to leave during its delay.
  - The same holds for any lowering of the caps below the items held, with a lower outflow.
- **Who can trigger:** a proposer, and k guardians who let two proposals pass, the first of them benign-looking. This is the
  timelock's stated limit, as A4-L-1. The loss is bounded by the items that cannot leave. Low, as A4-L-1. Compared with A4-L-1 it
  needs one more proposal, but it reaches more of custody.
- **PoC:** `poc/A/r5a_exit_rule_append_to_canister_rs.rs`
  `weakness_lowering_caps_and_outflow_together_passes_while_custody_holds_more` (passes: the weakness is present).
- **Fix:**
  - In `post_upgrade`, run the exit rule on `max(cap, held + in flight)` per collection and in all, from the restored book, at the
    new limits. Refuse a lowering of the outflow that held custody would not fit. Lowering caps alone stays allowed.
  - In `argdiff`, compare a lowering with `custody_caps()` headroom and page when held custody would not fit.

### A5-L-2 (Low): "each collection fits, and the total fits" is not enough under deadline-first delivery; every option leaves items for the margin day

- **Code:**
  - `outflow.rs:147-156` (`exit_fits`);
  - `canister.rs:307-324` (the per-collection and global tests, each alone);
  - `ethflow.rs:2179-2180` (`waiting.sort_by_key(deliver_by)`: deliveries in the order their tickets resolved; one a tick, the
    first the limiter allows) and `:2230`;
  - `script/golive-check.sh:666-690` (`EXIT_PY`, the same two tests).
- **Impact:**
  - When Milady's and MAYC's tickets resolve first, they take day 1's global room. Azuki then gets only its per-collection 12 on
    day 2. The two tests are exactly the feasibility condition of an optimal schedule; the deadline-first order is not optimal.
    Measured with the real H3 limiter, 10-minute ticks, one delivery a tick, deadline order:

    | Configuration | Worst order: left after the usable days | Order that suits Azuki |
    | --- | --- | --- |
    | (a) 13 / 17 / 13 / 3 / 2 @ 12 / 24 | 5 Azukis | 0 |
    | (c) interim 48 @ 12 / 31 | 3 | 0 |
    | (c) end 61 @ 12 / 31 | **9** | 0 |
    | (b) 16 / 22 / 10 held @ 12 / 24 | 8 | 0 |
    | (d) 61 @ 12 / 24, 96 h | 1 | 0 |

  - The items left go out at the 48 h mark, inside the day the rule keeps in reserve for finality and the record. That is still
    before a 72 h upgrade lands, the same effect as A4-L-1's third batch.
  - The order is set by who redeems first. Anyone holding Milady or MAYC units can take the early slots, and so can the proposer of
    a malicious upgrade who redeems before proposing. Honest ordering does it by chance.
- **Who can trigger:** as A4-L-1 (it matters only against a pending malicious upgrade). Low.
- **PoC:**
  - `poc/A/r5a_exit_rule_append_to_canister_rs.rs` `weakness_the_check_passes_but_the_delivery_order_leaves_items_past_the_usable_days`
    (passes: the weakness is present). The same test shows J7 at 22 / 31 and at 12 / 42 leave nothing under every order tried.
  - `poc/A/backlog_first.py`: the backlog-first order leaves nothing in every configuration above, (a) at install included.
- **Fix (preferred, no configuration change):**
  - When several waiting deliveries are allowed, sign the one from the collection with the most deliveries still waiting.
  - Deadline order still wins for a delivery within two days of its deadline.
  - One delivery a tick makes this the unit-by-unit rule, which keeps the rule's two tests sufficient.
- **Or tighten the check** (for 2 usable days):
  - require `per_collection_items` ≥ the largest item cap and total ≤ 2 × `global_items`;
  - or Σ min(cap, per) ≤ `global_items` (day 1 never global-bound) and Σ max(0, cap − per) ≤ `global_items`.
  - In (c), the second means step 1 raises `per_collection_items` to 22 (with `global_items` 31) or `global_items` to 42.

### A5-L-3 (Low): A4-L-3's keyless rule and host count, residual gaps

- **Code:** `script/golive-check.sh:778-793` (`keyed`), `:764` (`texts` reads `https?://` only), `:811` (the registrable domain),
  `:820-826` (the per-chain host count).
- **Impact** (`poc/A/install_check.output.txt`, real `Init`s, the checker extracted unchanged, `GOLIVE_PHASE1=1`):
  - **A full 32-hex key as 11 / 11 / 10 path segments passes as "all keyless"** (case 2), and so does an 11-character key (case 1).
    The 12+ rule applies per part, and the separator rule looks inside one segment only.
  - **A key in a `wss://` URL is never read** (case 11). The URL is published with the argument, and the chain still counts two
    https hosts, so it passes "all keyless" and "two hosts".
  - **The host-count FAIL line prints the full hostname** (case 14): two URLs on one key subdomain print
    `f3a9c2e17b4d4e0a9c1b2d3e4f5a6b7c.eth.rpc.example.net`. The keyed line above it redacts the key.
  - **One provider counts as two hosts:**
    - `xrpc.cl` and `xrpc.cl.` (a trailing-dot FQDN, case 12; the `ok` line then prints `cl.`);
    - two subdomains of one provider (`ethereum-rpc.publicnode.com`, `eth.publicnode.com`, case 13);
    - IPv4 hosts print as their last two octets (`113.7`), which is cosmetic.
  - **Refused, as the fixer says:** userinfo, a query under any name, a fragment, percent-encoding (upper case too), a split key
    inside one segment, a hex key in a subdomain, and the dry run's three new refusals.
  - A letters-only key in a hyphenated subdomain (case 4) passes. This is rare.
  - The cost of a missed key is a paid plan's quota, not funds. One provider counted twice makes the canister's two-provider
    agreement one provider's word. Both need an operator mistake.
- **Fix:**
  - Treat any path of 16+ key characters in all, across segments and separators, as keyed.
  - Read every `://` text, `wss` included, and refuse any non-`https` URL in `rpc_urls`.
  - Redact hostnames in every line (the host-count FAIL included).
  - Count hosts by registrable domain, trailing dot stripped.
  - Better, the round-4 alternative: allow-list the canister's providers under `--phase1`.

### Infos

- **A5-I-1: the launch outflow is data but not held against the canister (A4-I-2, partly).**
  - `launchexit` and `canexit` (`golive-check.sh:699-731`) each hold their own pair to the exit rule.
  - Nothing compares `custody_outflow()` with `nft-pilot.json` `outflow`: not the item limits, the mints, `global_value_wei` (the
    operator's 25 ETH) or `trip_factor`.
  - Fix: `same` each field of `custody_outflow()` against the launch file (its current stage under (c)).
- **A5-I-2: golive-check prints option (b) and then refuses it.**
  - The J7 total read (`:410-417`) FAILs any global cap under the collections' sum (`poc/A/option_b_golive.output.txt`), and the
    canister accepts (b).
  - `launchexit` reads a `custodyGlobalItems` key that no other check reads, and that nothing compares with the canister's global.
  - Moot under (c). Drop (b) from the printed options or make the J7 read accept a declared binding total.
- **A5-I-3: placeholders and D8 references (A3-L-3 residual).**
  - `TODO_RUNNER`, `RUNNER_TODO`, `YOUR_RUNNER_HOST`, `PLACEHOLDER`, `FILL_ME_IN`, `{{RUNNER}}` and `[runner]` pass the
    placeholder rule (`:640`; `\b` does not break at `_`) (`poc/A/placeholder_check.output.txt`).
  - The D8 references are only as right as the launch file: nothing checks that `openseaSlug` names this contract. OpenSea's
    keyless `collections/{slug}` returns the contract, and the design already reads it.
- **A5-I-4: the documents and the stage table for (c).**
  - The committed `nft-pilot.json`, the Trust page (`phase1Trust.ts:20`), G7 row 0 (`launch-gates.md:322`, the site's copy),
    `phase1-nft.md` §5 (`:532`) and `phase1-nft-pooled.md` §6 J7 (`:416-420`) all say "the operator chooses". None of them states
    (c)'s steps.
  - G7 row 1 (`launch-gates.md:323`: 12 items / 24 global, 6 / 12 a day) is below J7's stage 0 for Ethereum collections and below the launch outflow. `ops/src/stage.ts` scales J7's 24 items ×4 a stage
    for them instead, so the table and the reconciler disagree.
  - The monitor's `argdiff` pages the planned raise as CRITICAL and does not evaluate the exit rule on proposals. golive-check reads
    no pending proposal. See the (c) section, item 5.
- **A5-I-5: H3's item tripwire cannot fire on held items, and the item limit bounds count, not value.**
  - The exit rule forces `global_items` ≥ custody / 2, so 3 × `global_items` > custody at every configuration it accepts (72 or 93
    against 48 or 61). The per-collection trip (36) is likewise above every cap. So the freeze fires only on asks for items custody
    does not hold.
  - A compromised signing path takes BAYC's 2 and Pudgy's 3–4 (≈ 24–27 ETH) on day 1 under every option. Raising to 31 moves the
    day-1 total from about 43 to 49 ETH at the 2026-09-27 floors.
  - The exit promise also assumes the operator's runner keeps ticking (`tick` is operator-only; known from `D-services-deploy.md`,
    the monitor's liveness pages it).
  - The Trust page's "a tripwire at three times any of those" should say what it covers.

## What holds (checked, not findings)

- **The arithmetic.**
  - `exit_fits` is `ceil(n / per) ≤ 2`, the same as golive-check's `n > per × 2`. A `per_day` of 0 is refused by both (the
    canister's `outflow::validate` refuses an item limit of 0 first).
  - The canister's total is `min(Σ caps, global)`, and golive-check's is `min(total, gcap) if gcap`. The only difference is a
    global of 0, which the canister refuses as malformed.
  - Exactly 48 at 24 passes on both. 49 at 24 fails on both. [24, 24] and [24, 25] behave at the per-collection edge as expected.
- **The canister reads the canister's own truth.** `canexit` reads `custody_caps()` (every collection, a Bitcoin wrapper's
  included) and `custody_outflow()`, the same pair `validate_custody` sees. The launch file cannot make it pass: a canister pair that
  fails, fails.
- **The order of (c)** is enforced by the canister, and a queue race cannot reorder it: the timelock's staleness and "one pending
  per proposer" (`r5a_timelock_queue`).
- **No path shortens the delay:** `Upgrade`, `Controllers` and `Roles` all wait `delay_secs` ≥ 72 h, and pause, start and resume
  install nothing.
- **The contracts are unchanged** since `2735dac`: round 4's A-part conclusions on `EthNftWrapper`, `noteSent` and the refund
  guard stand.
- **A3-L-3's swap** is refused per collection, against the launch file, and fails closed when the launch file lacks the collection or
  a reference.

## PoCs (`poc/A/`)

| File | Finding | Result |
| --- | --- | --- |
| `r5a_exit_rule_append_to_canister_rs.rs` (+ `r5a_exit_rule.output.txt`, `r5a_grid.txt`) | A4-L-1 check; (c) sequence; A5-L-1; A5-L-2 | 4 tests pass. The (c) sequence is as tabled; the `weakness_*` tests pass (the weaknesses are present) |
| `exit_agreement.py` (+ `exit_agreement.output.txt`) | A4-L-1 (the canister and golive-check agree) | 392 exit-rule rows agree, 0 disagree |
| `backlog_first.py` (+ `.output.txt`) | A5-L-2's fix | Backlog-first leaves nothing in every configuration; deadline-first's worst order leaves 3–9 Azukis |
| `r5a_timelock_queue_append_to_rules_rs.rs` (+ `.output.txt`) | (c)'s queue plan | 1 test passes: one pending per proposer; stale after any executed or refused target action; a re-queue restarts the delay |
| `install_check.sh`, `mkinit.mjs` (+ `install_check.output.txt`) | A5-L-3 | Cases 1, 2, 4, 11, 12, 13 pass as keyless / two hosts; case 14 prints the key subdomain; cases 3, 5–8 refused |
| `placeholder_check.py` (+ `.output.txt`) | A5-I-3 | 10 of 16 placeholder forms pass |
| `option_b_golive.sh` (+ `.output.txt`) | A5-I-2 | Option (b) FAILs golive-check's J7 total read |
