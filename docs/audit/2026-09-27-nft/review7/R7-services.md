# Round 7, part D: the monitor, the runner and the depth probe after fix round 6

Reviewer: independent AI re-review, 2026-09-28. Worktree `/Users/jlohayon/Documents/Rook-nft`, branch `nft-side` at `a78a542`
(delta `bb713fe..HEAD`: `7b9e8ba` monitor and runner, `b4c3f55` / `2841f90` go-live, `785b384` site).

**Scope:**
- `monitor/src/candid.mjs`: `EthStep`, `HomeTx` (`step`, `sent_at`), `SaleScan`, `EthState.sale_scan`.
- `monitor/src/ethnft.mjs`:
  - `eth-home-unseen`, worded per step (D6-L-1);
  - `eth-home-out` (D6-L-2);
  - `eth-late-hold` (D6-L-3, D6-I-2);
  - `eth-scan-behind` / `-lost` / `-hole` / `-receipts` (B6-M-1, D6-I-1).
- `monitor/src/watcher.mjs` (`stagedTags`).
- `ops/src/runner.ts`: `parseRunnerConfig` (`RUNNER_PENDING_EVERY_S`), `sleepAfter`, the main loop.
- `ops/src/ic.ts`: the IDL reads `pending`.
- `script/golive-check.sh` `depth_probe` (`:896-922`).
- `web-rook/src/rook/phase1Trust.ts`.
- Judgment calls 1 and 4 (the pair's sends; the 240 s cadence).

Each was checked against the canister at HEAD: `etch_autobuyer.did`, `ethflow.rs` `home_clear_applies` `:4525`, `late_hold` `:690`,
`send_home` `:3221`, `sale_scan` `:1233`, `:1578`, `reanchor` `:1671`.

**Nothing in the worktree was modified except this report and `poc/D/`. Nothing was committed, pushed, deployed or signed.**

- **Where the PoCs ran:** private copy `/tmp/rook-r7-D`: `diff -rq` identical before, removed afterwards.
- **Network use:** one read-only `--probe-depth` run on the committed pair, and a local fake RPC on 127.0.0.1.

## Verdict

- **D6-L-1, D6-L-2 and D6-L-3 (monitor side), D6-I-1 and D6-I-2 are closed.**
  - The monitor's `EthStep` is the `.did`'s case for case, and `HomeTx` / `SaleScan` decode the canister's round-6 and round-5 bytes
    (the fixtures).
  - `clearable` (`ethnft.mjs:134`) is exactly `home_clear_applies` (a `Deposit`, or a `GiveBack` of a buy with `sweep == Some(true)`).
  - `sent_at` is in seconds and set once per signing, not on "sent again", so the age page cannot be reset by a resend.
  - `held` is `late_hold`'s `!credited && !released`. SEV1 comes 72 h after `let_go_at`, persistent.
- **F6-L-1, the runner side, is implemented as described:**
  - `RUNNER_PENDING_EVERY_S` refuses 0, negative, NaN, fractions and anything above the cadence;
  - the IDL reads `pending` from the round-4, -5 and -6 bytes;
  - an unreadable state falls back to the normal cadence.
- **The 240 s call is acceptable.** Evidence hand-ins stay at 1,200 s per collection, so the faster tick adds only tick calls. It
  lands inside 4663's ≈ 10 min state reach only while a cycle is short (**D7-I-3**).
- **One new Low, D7-L-1:**
  - `--probe-depth` (and the runbook's IC rehearsal check, which copies it) accepts an EMPTY `eth_getLogs` answer 60,000 blocks back
    as proof of depth.
  - A provider that silently truncates its log history passes.
- **Two of the four scan pages work as intended** (`eth-scan-lost`, `eth-scan-receipts`, with `eth-scan-hole` as SEV3).
  `eth-scan-behind` is effectively unreachable after the re-anchor, and reads a snapshot a stopped scan never updates (**D7-I-1**).
  A stopped scan is still paged indirectly by the keeper's `floor-stale`.
- **The Trust page's new "a shorter sample pages the operator"** is true only for refusals classified `deep` (**D7-I-4**).

## Counts (part D, open)

| Critical | High | Medium | Low | Info |
| ---: | ---: | ---: | ---: | ---: |
| 0 | 0 | 0 | 1 (D7-L-1) | 4 |

## Round-6 items: verdicts

| ID | Verdict | Basis, residuals |
| --- | --- | --- |
| **D6-L-1** (monitor side) | **Closed** | `phaseOf(pend.step)`, where `stepBuy` is the step's buy in `state.buys`. `clearable` (`ethnft.mjs:134`) equals `ethflow::home_clear_applies` (`ethflow.rs:4525-4531`, `is_sweep` = `sweep == Some(true)`). A buy missing from `buys` is not clearable on either side. The unclearable text names no `eth_home_clear`. The candid `EthStep` has the same 10 cases as the `.did` (a new case would fail the whole `eth_nft_state` decode, which is why no enum case may be added to a saved type) |
| **D6-L-2** | **Closed** | `sent_at: Some(io.now())` only in `send_home` (`ethflow.rs:3221`). "Sent again" re-broadcasts the same raw bytes and keeps it. `eth-home-out` (`ethnft.mjs:143`) keys on the hash: SEV2 at 30 min, SEV1 at 2 h, persistent, whatever `used_since` says. Residual **D7-I-2** (a transaction signed by the round-5 build has no `sent_at`) |
| **D6-L-3** / **D6-I-2** | **Closed** | `eth-late-hold` (`ethnft.mjs:117-120`) is emitted for any buy carrying `late` (outside the phase switch, so also once closed as a loss) while neither `credited` nor `released_at`. That is exactly `late_hold` (`ethflow.rs:690-692`). It is SEV1 from `let_go_at + 72 h` (seconds), persistent. `eth-stuck`'s wording uses the same `held` |
| **D6-I-1** / B6-M-1 pages | **Partly** | `eth-scan-lost` (SEV2, persistent) and `eth-scan-receipts` (SEV3 at `fails ≥ 3` = `SALE_RECEIPT_FAST_TRIES`) read the canister's own fields. Absent fields (a round-5 canister) decode as `[]` and page nothing. `eth-scan-behind` is dead: **D7-I-1** |
| **F6-L-1** (runner) | **Closed as implemented**; judgment call acceptable | `parseRunnerConfig` (`runner.ts:50-52`): unset = `min(240, everyS)`, and 60..`everyS` is enforced (`poc/D/r7_d.test.ts`). `sleepAfter` (`:79-81`), and the loop (`:209-214`) reads `eth_nft_state().pending` after each cycle. Residual **D7-I-3** |
| **D6-I-4** (Trust page "the week") | **Partly** | **D7-I-4** |

## D7-L-1 (Low): `--probe-depth` takes an empty log answer as depth; a silently shallow provider passes the gate

**Code:** `script/golive-check.sh:913-915`:
- `logs = rpc("eth_getLogs", [a 500-block Milady Transfer range at head-60,000])`;
- `if not isinstance(logs, list): return …`: any list, **including `[]`**, passes the logs half.

The receipt half reads the first transaction of block head-60,000, which proves the provider's **transaction index** is deep, not its
**log index**. The runbook's IC rehearsal check (`docs/v7/phase1-nft.md:576-580`) copies the same shape: "a 500-block `eth_getLogs`
of a launch collection's `Transfer` 60,000 blocks back, and a receipt there".

**Why it matters:**
- A 500-block Milady range that deep holds 0–5 logs (`fixes-round6.md` §1.1). So an empty answer is also an honest one, and the
  probe cannot tell "no logs here" from "logs not kept here".
- Some providers answer beyond their log retention with `[]` instead of an error: a pruned log index beside a full transaction index,
  or a load balancer mixing archive and full nodes.
- Such a provider passes `--probe-depth`, and a passing probe **admits a provider not on the measured list** (`:1002-1006`). That is
  the path the fixer's judgment call 1 names for replacing either of the pair.
- **In the canister:**
  - Beside an honest provider, the truncating one disagrees on every deep range. The read fails as `Other`, so it is not `Deep`, and
    no `lost` is recorded. The walk back and the hole walk shrink and defer, and those deferred ranges are not paged (D7-I-4).
  - If both providers truncate, the canister counts no sales past their depth and records nothing as lost. The sample is short
    silently, which is the B6-M-1 condition with its new page bypassed.

**PoC:** `poc/D/fake_shallow_rpc.py` is a provider that returns `[]` for logs older than 10,000 blocks and serves blocks and receipts
at any depth. golive-check's `depth_probe`, executed verbatim from `INSTALL_PY`, returns `'ok'` (`poc/D/probe_depth_empty.output.txt`).
The committed pair passes the real probe today (`poc/D/probe_depth_live.output.txt`, 13:21 UTC).

**Why Low:**
- It needs an operator admitting a new provider by probe, or a listed provider changing its retention silently.
- Integrity holds: no false sale is counted, and nothing is signed from it.
- The effect is the price feed's sample, which B6-M-1 rated Medium only when it froze the walk for days.

**Fix:**
- Probe a span known to hold logs: for example the 10,000-block Milady range 60,000 back (44 logs measured). Require at least one log,
  and require the same count on every provider of the pair.
- Also, or instead, read a known old sale's `Transfer` by block and require it.
- The runbook's rehearsal check should say the same.

## Infos

- **D7-I-1 (`eth-scan-behind` is effectively unreachable, and reads a snapshot):**
  - `sc.behind` is written only at the end of a pass (`ethflow.rs:1578`).
  - A pass that starts more than 7,200 blocks behind re-anchors first (`:1233`), leaving `up = top − chunk` (`reanchor` `:1671-1677`,
    chunk ≤ 2,000). So a pass can leave `behind ≥ 7,200` only when the lag was exactly 7,200 at its start and nothing was read.
  - A scan that stops running (the runner down, cycles under the evidence floor, a pause) never updates `behind`, and the page never
    fires, however long it lasts (`poc/D/r7_d.test.mjs` test 2).
  - The stop is still paged indirectly: the keeper's `floor-stale` (`monitor/src/chain.mjs:62`), and the runner's health.
  - `fixes-round6.md` §1.2 and the runbook describe `eth-scan-behind` as "a day behind".
  - **Fix:** page on the monitor's own Ethereum final head minus `up`, or on the reading's age, or drop the page and say so.
- **D7-I-2 (`eth-home-out` is silent for a transaction signed by the round-5 build):**
  - A `HomeTx` saved without `sent_at` (`eth-state-round5.hex`) never pages `eth-home-out`, however old (`poc/D/r7_d.test.mjs` test 1).
  - `eth-home-unseen` still covers it once the nonce is used.
  - It is transitional (one transaction at the upgrade).
  - **Fix (optional):** first-seen time as a fallback, as `seen()` does elsewhere in the same function.
- **D7-I-3 (the 240 s is a sleep after the cycle, not a period):**
  - The next tick starts `cycle length + 240 s` after the signing tick starts (`runner.ts:209-214`). A cycle can be a tick, the Relay
    and Ethereum steps (evidence hand-ins, OpenSea and Relay fetches with timeouts), and a second tick after a quote hand-in.
  - The canister's search reach is `HOME_STATE_REACH_BLOCKS` 4,000 (≈ 400 s). A cycle over ≈ 150 s puts the next tick outside it, and
    that transaction goes to the operator path (paged).
  - `RUNNER_PENDING_EVERY_S` accepts 301–600, and `0x3c` / `6e1`, although the runbook says "keep it ≤ 300" (`poc/D/r7_d.test.ts`).
  - **Fix:** sleep `max(60, pendingEveryS − elapsed since the tick that signed)`, cap the variable at 300, and have the rehearsal
    measure the cycle's length.
- **D7-I-4 (the Trust page over-claims):**
  - `phase1Trust.ts:143`: "its two Ethereum providers must each serve it, and a shorter sample pages the operator".
  - Only a refusal classified `deep` records `lost` and pages.
  - A refusal of another kind, a provider that disagrees on deep ranges, or a silent truncation (D7-L-1) leaves deferred or `raw`
    ranges and a walk back short of the window. The monitor reads none of these (`bad`, `raw`, `down` are not in its IDL).
  - **Fix:** page on `down > bottom` for a day (the window not covered) and on `bad` / `raw` growth, or soften the sentence. Coordinate
    with part B.

## Judgment calls (services side)

| Call | Assessment |
| --- | --- |
| 1: MEV Blocker (private orderflow) + Tenderly (public broadcast) | Acceptable. The canister sends to both, and only the read-back decides (`evmsend.rs`), so neither send reply can mislead it. Privacy is illusory, because Tenderly broadcasts publicly. A Seaport fill can be sniped as before, which costs only the fill's gas and a retry. MEV Blocker's "already known" for a mined transaction is harmless. What the providers' reads can steer is part B's question |
| 4: 240 s runner cadence while a 4663 transaction is out | Acceptable. Evidence hand-ins stay on their 1,200 s clock (`ethnft.ts:325`), so the extra cost is 1.5 more `tick()` calls per 10 min while the line is busy, plus their 4663 reads. It closes F6-L-1 for short cycles. D7-I-3 is the margin |

## PoCs (`poc/D/`)

- `r7_d.test.mjs` / `r7_d.monitor.output.txt`: D7-I-1, D7-I-2, on the canister's own fixture bytes (2/2 pass).
- `r7_d.test.ts` / `r7_d.ops.output.txt`: the runner's clamps hold, and D7-I-3 (2/2 pass).
- `fake_shallow_rpc.py` / `probe_depth_empty.output.txt`: D7-L-1.
- `probe_depth_live.output.txt`: the committed pair passes `--probe-depth` now (read-only).
- The round-6 suites for this area passed in the copy: monitor `audit_nft_d_r6` + `staged_r6` 11/11, ops `audit_nft_r6` 2/2.
