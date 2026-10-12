# Round 6, part A: the launch plan (option (c)) as committed, the staged-upgrade checks, backlog-first delivery, the held-custody exit rule

Reviewer: independent AI re-review, 2026-09-28. Worktree `/Users/jlohayon/Documents/Rook-nft`, branch `nft-side` at `e04fa16`
(delta `8ef82d6..HEAD`: fix round 5). Scope:

- `deployments/launch/nft-pilot.json` (`stage`, `stages`) and `monitor-nft.json` (`launch_file`);
- `script/golive-check.sh`: `--stage`, `stagecheck` (`STAGE_PY`), `launchval`, `launchexit`, `canexit`'s A5-I-1 comparison,
  `--proposal <hex> --stage N` with `--canister` / `--baseline`;
- `monitor/src/staged.mjs` (`stepNumbers`, `checkStagedProposal`, `stagedStep`), `argdiff.mjs` (`exitProblemsOf`, the EXPECTED
  downgrade in `proposalAlert`), `config.mjs` (`launch_file`), `watcher.mjs` (the `staged` context);
- the canister: `ethflow::delivery_order` / `DELIVERY_URGENT_SECS` (A5-L-2), `canister::held_fits` in `post_upgrade` (A5-L-1);
- the docs and site against the code: `phase1-nft.md` §5 / §5.1, `phase1-nft-pooled.md` §6 J7, `phase1Trust.ts`, `launch-gates.md`
  G7 (both copies);
- re-rates: A5-L-3, E5-L-2, A-L-2, A5-I-1..5.

The contracts are unchanged (`git diff 8ef82d6..HEAD -- src/ test/` is empty).

**Nothing in the worktree was modified except this report and `review6/poc/A/`; nothing committed, pushed, deployed or signed.**
The PoCs ran in a private copy under `/tmp/rook-r6-A/` (copies of `monitor/`, `script/` and `deployments/launch/`), removed at the
end. No anvil was needed: `--proposal` reads no chain. The fixer's `monitor/test/staged_r5.test.mjs` passes on the copy (3 of 3).

Severity scale: the README's.

## Verdict

**Option (c) as committed holds. The staged order cannot be bypassed or reordered on the canister**, and no round-5 claim in this
area was found false:

- The canister enforces what matters at execute time, on the argument as executed: caps-first (61 at 24) is refused by
  `validate_custody`; the exit rule on what custody HOLDS is now checked too (`held_fits`, A5-L-1); ×4 per step.
- The timelock's queue rules (unchanged since round 5's review) keep a queued step 2 from surviving any other executed upgrade, and
  one proposer has one pending proposal.
- `golive-check --proposal` refuses both-at-once, caps-first, a step against the wrong running step, and a step 2 moving anything but
  `global_items` (every cap key compared). `stagecheck` holds the file's plan to the rule.
- **A5-I-1 is fixed** (the canister's whole `custody_outflow()` is held to the step, field by field). **A5-L-1 is fixed** and it
  cannot block a code-only upgrade (below). **A5-L-2 is fixed** for the case it was reported on.

**One new Low**: the step-3 classification is loose. A proposal that is step 3 **plus** other cap loosenings (a ×4 raise of another
collection's cap, the global item and value caps above the sum) is classed "exactly step 3": the monitor pages it EXPECTED (SEV2,
every LOOSENS line downgraded to WARN) and `golive-check --proposal --stage 3 --baseline` passes it (A6-L-1). The canister's own
×4 and exit rules still bound it, so this is the guardians' signal weakened, not a bypass of a limit.

| ID | Sev | Title |
| --- | --- | --- |
| A6-L-1 | Low | Step 3 is recognised with extra cap loosenings: EXPECTED (SEV2) in the monitor, PASS in golive-check |
| A6-I-1 | Info | The plan's timing (after the handover; two clean weeks) is runbook-only; the monitor trusts the launch file's `stages` unchecked |
| A6-I-2 | Info | `golive-check --proposal` without `--baseline` passes a step whose other Init fields changed (a note, not a FAIL) |
| A6-I-3 | Info | Backlog-first holds for simultaneous resolution; the urgent path (deadline-first) returns after a long pause; the Trust page's "whatever order" is broader than the test |
| A6-I-4 | Info | Cosmetic: a staged diff labels a line the running code refuses ("more than ×4 … refused") as "(expected: the staged step)" |

## Counts of OPEN findings (this part)

| | Critical | High | Medium | Low | Info |
| --- | ---: | ---: | ---: | ---: | ---: |
| New | 0 | 0 | 0 | 1 (A6-L-1) | 4 (A6-I-1..4) |
| Carried, re-rated here | 0 | 0 | 0 | 3 (A5-L-3, E5-L-2, A-L-2) | 3 (A5-I-2, A5-I-3, A5-I-5) |

## Round-5 items: verdicts

| ID | Verdict | Basis |
| --- | --- | --- |
| **A5-I-1** (the launch outflow not held against the canister) | **Fixed** | `canexit` (`golive-check.sh:828-841`) compares every `custody_outflow()` field with the current step's `outflow` (or `--stage`'s). A missing field prints empty and fails `same`; an unreadable reply FAILs under `--phase1`. The dry run's trip-factor-12 refusal shows it. The read is an anonymous `--query` (uncertified), as every golive-check canister read |
| **A5-I-4** (docs and stage table for (c)) | **Fixed** | The Trust page, G7 row 0 (both copies identical: diffed), `phase1-nft.md` §5 / §5.1 and `phase1-nft-pooled.md` §6 J7 state 48 at 12 / 24, then `global_items` 31, then J7's 61, and backlog-first. G7 notes rows 1–3 are the Bitcoin wrapper's. Residual wording: A6-I-3 |
| **A5-L-1** (the exit rule on caps, not on held custody) | **Fixed** | `held_fits` (`canister.rs:331-350`) runs in `post_upgrade` after `check_cap_change` (`:527-535`), on the restored book, per collection `max(cap, held)` and in all `max(min(sum, global), held)`, and traps atomically. **It cannot block a code-only or caps-lowering upgrade:** at unchanged item limits it fails only if more is held than any cap that passed the rule at those limits, and `custodycap::reserve` / `admit` refuse any entry over its cap. It refuses only a LOWERING of `per_collection_items` / `global_items` below what is held, as `fixes-round5.md` §9 call 9 admits; value, mint and trip-factor limits can still be lowered in an emergency, and H1 stops every outflow. Not bypassable by the argument (it reads the book, not the argument). (Outside this part: `ethflow::adopt_advance` funds a buy even when its `reserve` is refused, `ethflow.rs:3614-3624`; part F's to judge) |
| **A5-L-2** (deadline-first leaves items for the margin day) | **Fixed for the reported case** | `delivery_order` (`ethflow.rs:2605-2616`): urgent (within 2 days of `deliver_by`) first by deadline, then the largest backlog, deadline within it. `r5_a5_l2_…` drives the real limiter at 10-minute ticks and reproduces review 5's 5 / 3 / 9 before asserting 0. By hand: 16/22/17/4/2 at 12 / 31, largest-first with the per-collection cap leaves 10/7/7/4/2 = 30 for day 2, each ≤ 12. Largest-first with a per-collection cap is the right greedy for a two-day horizon. Residual: A6-I-3 |
| **A5-L-3** (golive-check keyless residuals) | **Stays Low, unchanged** | Not touched (`fixes-round5.md` §7). Operator-mistake only (a key split 11/11/10, `wss://`, a key subdomain printed on the host-count FAIL line, a trailing-dot host) |
| **E5-L-2** (the exit rule ignores the tripwire) | **Stays Low** | Unchanged code. The launch steps keep a margin (step 1: 17 + 3 < 36; step 3: 61 + 6 < 93 in all), so it is reachable only with the stage-0 defaults (deposits on) or a trip factor of 2, both operator configurations golive-check would show |
| **A-L-2** (the wrapper's `deployer` is the deploy key) | **Stays Low, unchanged** | No contract changed |
| **A5-I-2** (golive-check prints option (b), then refuses it) | Unchanged (Info) | Moot under (c) |
| **A5-I-3** (placeholder forms) | Unchanged (Info) | |
| **A5-I-5** (the item tripwire cannot fire on held items) | Unchanged (Info) | By design |

## Option (c): can the staged sequence be bypassed or reordered?

| Attempt | Result |
| --- | --- |
| Caps first (61 at 24) | Refused by the canister (`validate_custody`), by `--proposal --stage 3` (the running pair is not step 2; the exit rule), and paged SEV1 with "executing it fails" |
| Both at once (61 at 31 from step 1) | Accepted by the canister (it fits the rule), but **not** a staged step: `stagedStep` → null, SEV1 (the caps' LOOSENS are CRITICAL); `--proposal --stage 2` and `--stage 3` both FAIL |
| Step 3 queued while step 2 is pending | Its diff from the baseline (step 1) is not step 3: SEV1. If step 2 executes first, the earlier proposal is stale (timelock `target_epoch`) |
| Step 3 at once after step 2 (skipping the two clean weeks) | EXPECTED. The two weeks are the runbook's alone (A6-I-1) |
| A stale or wrong monitor baseline | The baseline is the last executed upgrade's argument, else `install_arg_hex`. `baselineMismatch` pages SEV1 when the canister's `custody_caps()` / `custody_outflow()` differ from it. A step judged against a wrong baseline would also have to pass the canister's own ×4 and exit rules at execute time |
| Editing `"stage"` in the launch file to skip | The monitor ignores `stage` (it diffs against the baseline). golive-check `--launch` holds the canister to the file's step and FAILs when they differ. Fail-closed |
| Editing `stages` (e.g. step 2 at 96 a day) | golive-check's `stagecheck` refuses a step that is not `global_items` alone, not a raise, over ×4, or unfit. **The monitor loads `launch_file` without `stagecheck`**: whatever its `stages` say becomes EXPECTED (A6-I-1). The monitor configuration is the operator's |
| A step plus another Init field (operator, providers, deposits, `eth_nft`) | The monitor: `OTHER` lists every non-cap, non-limit `Init` field (checked against the `.did`), so null → SEV1. golive-check: FAIL with `--baseline`; **only a note without it** (A6-I-2) |
| A step 2 plus another cap change | Refused: `moved()` compares every cap key and the global caps |
| **A step 3 plus other cap loosenings** | **EXPECTED / PASS (A6-L-1)** |
| A known but older build carrying an exact staged step | SEV2 (the module line is INFO for any listed build). This is not new: the same downgrade with no config change is INFO today |

## A6-L-1 (Low): step 3 is recognised with extra cap loosenings

- **Code:**
  - `monitor/src/staged.mjs:51-61` (`offStep`): it checks only the launch collections' own caps, and the global item cap only from
    below (`p.global.items < s.global.items`). It never checks the global value cap.
  - `:78-80` (`kindProblem`, step 3): it asks only that the limits did not move and that some cap moved. Any cap key may move.
  - `:119-128` (`stagedStep`), `:86-113` (`checkStagedProposal`).
  - `monitor/src/argdiff.mjs:291`: once `staged` is set, **every** LOOSENS line of the diff becomes WARN, not only the step's own.
- **What passes as "exactly step 3" from a step-2 baseline:**
  - another collection's cap raised up to ×4, such as a Bitcoin wrapper's inscriptions (3 → 12 items, 10 → 40 ETH) or a new key;
  - the global item cap above the sum (made binding at 62, which J7 says never binds);
  - the global value cap ×4 (85 → 340 ETH).
  - As long as the whole still fits the exit rule, the monitor pages it **SEV2 EXPECTED**, with each extra loosening a WARN marked
    "(expected: the staged step)". `golive-check --proposal --stage 3 --baseline` prints `ALL CHECKS PASS`.
  - Every change is within the canister's ×4 rule and the exit rule, so the running code would execute it.
- **Impact:** the guardians' two tools, the pager and the pre-proposal check, both call a proposal "the planned step" when it
  carries loosenings the plan does not name. The canister's hard limits (×4 a step, the exit rule, `held_fits`) still bound it, and
  the lines are shown. So this weakens detection of a compromised or mistaken proposer, but moves no limit past the canister's
  rules. Low.
- **PoC:** `poc/A/r6_a.test.mjs` (monitor copy, `node --test`: 4 of 4 pass, so the weakness is present), `poc/A/r6_a.output.txt`
  (the alert text). `poc/A/mkhex.mjs` + `poc/A/golive_step3.output.txt` show the real CLI, `script/golive-check.sh --proposal
  evil3.hex --launch deployments/launch/nft-pilot.json --stage 3 --baseline s2.hex`, printing ALL CHECKS PASS. Control: the same
  extras on step 2 are refused.
- **Fix:**
  - In `offStep`, require every cap key that is not a launch collection to be unchanged between the running pair and the proposal.
    Require the global item cap to equal the step's sum (J7: no total binds), and the global value cap the step's value sum.
  - In `proposalAlert`, downgrade only the lines the step itself names (the five item counts and the global item sum for step 3;
    `global_items` for step 2), and keep any other LOOSENS line at its own severity.
  - Test both with this PoC turned around.

## A6-I-1 (Info): timing is runbook-only; the monitor trusts `stages` as written

- The plan says step 2 is queued after step 1 has executed and the handover is done, and step 3 after two clean weeks at step 2.
  Neither is checked: a step-3 proposal right after step 2 executes is EXPECTED. This is acceptable, since the plan's safety
  argument rests on the canister's rules, not on the two weeks. It should be said in §5.1 that the monitor does not enforce it.
- `config.mjs:28-33` loads `launch_file` without the `stagecheck` rules. A mis-edited `stages` in the monitor's copy makes a
  different step EXPECTED.
- **Fix:** run `STAGE_PY`'s checks (or a JS twin) when the monitor loads the file, and refuse to start on a FAIL.

## A6-I-2 (Info): `--proposal` without `--baseline` only notes the other fields

- `staged.mjs:109`: with `--canister` alone, a step-2 argument that also names a new operator and new 4663 providers passes
  (`poc/A/r6_a.test.mjs`, the last test). The monitor pages the same argument SEV1, and §5.1 step 2 says to add `--baseline`.
- **Fix:** make `--baseline` required with `--proposal` under `--phase1`, or FAIL when neither `--baseline` nor the monitor's
  install argument is given.

## A6-I-3 (Info): backlog-first, its limits

- The regression (`r5_a5_l2_…`) resolves every ticket at t0, with deadlines one second apart. The Trust page and G7 say "whatever
  order tickets resolve in". Under the exit rule's own model (a bank run starting when a proposal becomes visible), largest-first
  with the per-collection cap is correct. The wording is broader than the test.
- `ethflow.rs:2610`: deliveries within two days of `deliver_by` go first, deadline first. After a long pause (H1) or a stall, many
  deliveries become urgent together, and the order is deadline-first again, which is A5-L-2's shape. That is the right order for
  deadlines. It is noted because the exit-rule guarantee does not hold in that state (a paused canister also delivers nothing).

## A6-I-4 (Info): an "expected" label on a line the canister refuses

- With `staged` set, `argdiff.mjs:291` appends "(expected: the staged step)" to every LOOSENS line, including one that already
  says "more than ×4 at once: refused by the running code" (seen in the PoC's first run, global value 85 → 460 ETH).
- `stagedStep` requires only the exit rule, not the ×4 rule, so a proposal the canister would refuse can still be EXPECTED.
  Folded into A6-L-1's fix: also require `validate_change`'s ×4 on the whole pair in `stagedStep`.

## The docs and site against the code

| Document | Verdict |
| --- | --- |
| `phase1-nft.md` §5 table, §5.1 runbook | Match the code (48 at 12 / 24; `global_items` 31 alone; J7's 61 caps only, carrying 31; stale-on-any-executed-upgrade; `--stage 2` read after step 2; `eth_home_clear`, `eth_late_release` as coded). §5.1 does not say the monitor cannot tell step 3 with extras from step 3 (A6-L-1) or that timing is not enforced (A6-I-1) |
| `phase1-nft-pooled.md` §6 J7 | DECIDED (c), matches |
| `launch-gates.md` G7, both copies | Identical diffs; match. "whatever order tickets resolve in": A6-I-3 |
| `phase1Trust.ts` | States 48 installed, then 31 a day, then 61, backlog-first, the held-custody rule; matches |
| `nft-pilot.json` | `custodyItems` 13/17/13/3/2 = step 1; `outflow` = step 1's; `stages` pass `stagecheck`. `_outflow`'s text still says "the custodyItems above (61 in all) do NOT fit", now stale (the file's items are 48). Cosmetic |

## PoCs

| File | Finding | Result |
| --- | --- | --- |
| `poc/A/r6_a.test.mjs` (+ `r6_a.output.txt`) | A6-L-1 (a, b), control, A6-I-2 | 4 of 4 pass: step 3 with a ×4 Bitcoin cap raise and the global caps moved is EXPECTED (SEV2) and passes `checkStagedProposal`; with a new unconfigured key too; the same extras on step 2 are refused; `--proposal` without `--baseline` passes a new operator and providers (the monitor: SEV1) |
| `poc/A/mkhex.mjs` + `golive_step3.output.txt` | A6-L-1 through the CLI | `golive-check.sh --proposal … --stage 3 --baseline` prints ALL CHECKS PASS |
