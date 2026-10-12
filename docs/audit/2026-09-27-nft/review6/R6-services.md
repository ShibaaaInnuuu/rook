# Round 6, part D: the services after fix round 5 (monitor, runner IDL, Trust page)

Reviewer: independent AI re-review, 2026-09-28. Worktree `/Users/jlohayon/Documents/Rook-nft`, branch `nft-side` at `e04fa16`
(delta `8ef82d6..HEAD`; services commit `d2d8c66`, go-live `4fed345`, site `536860c`). Scope: `monitor/src/{argdiff,candid,config,
ethnft,staged,watcher}.mjs` (the `eth-home-unseen` page and its timing, the late-hold texts, D5-L-1's `deliveryTx` exemption, D5-L-2's
`EthBuy.last` and the three Stuck cases, D5-I-3's wording, the EXPECTED staged step), the monitor tests, `monitor/test/d5-state.hex`,
`ops/test/audit_nft_r5.test.ts` and the runner's IDL (`ops/src/ic.ts`); the monitor's and runner's IDLs against the round-5 `.did`;
the Trust page (`web-rook/src/rook/phase1Trust.ts`) against the code; re-rates of D5-L-*, D5-I-*, D-L-3, D-L-6. Canister code was read
where a page depends on it (`ethflow.rs` `resolve_home`, `home_clear`, `late_hold`, `in_relay`).

**Nothing in the worktree was modified except this report and `review6/poc/D/`; nothing committed, pushed, deployed or signed.** PoCs
ran in a private clone under `/tmp/rook-r6-D/` (removed afterwards). The worktree's monitor suite was run unmodified: 203/203.

Severity scale and threat model: `../README.md`.

## Verdict

**No Critical, High or Medium in the services.** The round-5 service fixes hold: D5-L-1 and D5-L-2 are closed. The IDLs are sound. The
new pages have three gaps, all Low. They concern what the operator is told, never what is signed:

- **D6-L-1:** the unseen-home page gives the same instruction for every step. For most steps that call does nothing.
- **D6-L-2:** a home line held by Pending or Unsure never pages.
- **D6-L-3:** the late hold, which stops every new buy, is only a SEV3 note.

## Counts (part D, new)

| Critical | High | Medium | Low | Info |
| ---: | ---: | ---: | ---: | ---: |
| 0 | 0 | 0 | 3 | 5 |

## Round-5 fixes: verdicts

| ID | Verdict | Basis |
| --- | --- | --- |
| **D5-L-1** (false SEV1 when a record lands mid-poll) | **Closed** | `ethnft.mjs:~548`. The exemption needs all of: ticket stage 3, `deliveryTx` equal to this delivery's own mined transaction (`DELIVERY_MINED` phases only), and the same item and destination. Only the custodian (the canister) can call `recordDelivery`, so an attacker cannot write a `deliveryTx`. A different transaction, a refund (stage 4) or a not-yet-mined delivery (`Sent` / `Cancelling`) still pages. The fixer's `audit_nft_d_r5` holds all four cases, and I re-read them. |
| **D5-L-2** (`last` not decoded; Stuck cases confused) | **Closed** | `candid.mjs` decodes `last: opt record { order_id }`, a record subtype of `EthWatch`. The monitor's "watched" is `last` present, exactly the canister's `in_relay` for `Stuck` (`ethflow.rs:254`). A written-off order is told apart by `late`, and a fill-Stuck buy has neither. A sweep closes "with nothing". Residual: D6-I-2 (a *credited* late order is still called held). |
| **D5-I-3** (wording) | **Partly** | The mined-delivery deadline text now says "at finality". The first half (a written-off order's payout landing is not paged) is unchanged, as the fixer states. |
| **E5-I-3** (services side) | **Closed** | Both the monitor and the runner decode `eth-state-round5.hex`, which carries every round-5 field (`HomeTx.pin`, `Watch.request_id` / `home_pin`, `LateOrder.released_at`, `Inbox.home_clear` / `payouts`, `SaleScan.done_big` / `stuck` / `spent`). |
| **F5-M-1** page (`eth-home-unseen`) | **Present; gaps D6-L-1, D6-L-2** | SEV1, persistent, keyed by hash, fires after `used_since` is 3,600 s old (equal to `HOME_NONCE_TAKEN_SECS`). |
| **C5-M-1** texts (the hold) | **Present; gap D6-L-3** | The Stuck and late-order texts name `eth_late_release`, and a released order says the risk. |
| Option (c): argdiff EXPECTED | **Holds for the launch collections** | `stagedStep` (`staged.mjs`) requires all of: every non-cap, non-limit Init field unchanged; the baseline at step n−1; the proposal at step n; the change of step n's kind alone; the proposal inside the exit rule. The baseline is the last *executed* upgrade's argument (`watcher.mjs:15-32`), and a mismatch with the canister's live caps pages SEV1. The module hash is still judged (an unknown build stays CRITICAL). Residual: D6-I-3. |

## IDL and upgrade safety (services)

**Holds.**

- **The monitor's IDL** (`candid.mjs:190-219`) adds only `opt` fields: `LateOrder.released_at`, `EthBuy.last`, and `EthState.pending`
  (`HomeTx` with `hash`, `nonce`, `used_since`). It skips the canister's `step`, `raw` and `pin` by record subtyping. Every record
  it names exists in the `.did` with the same types.
- **PoC `poc/D/audit6_nft_d.test.mjs` test 1** checks decoding in both directions:
  - the round-5 canister state (`eth-state-round5.hex`) decodes with the round-5 monitor IDL, and with the **round-4 monitor IDL**
    (`git show 8ef82d6:monitor/src/candid.mjs`), so an old monitor on the new canister is fine;
  - the round-4 and round-2 states decode with the round-5 IDL. The new `opt` fields come back empty, so the new monitor on an old
    canister is fine.
- **The runner:** its IDL (`ops/src/ic.ts`) is unchanged. `ops/test/audit_nft_r5.test.ts` decodes the round-5 state (re-run in the
  copy: 1/1). The runner calls neither `eth_home_clear` nor `eth_late_release`.
- **`monitor/test/d5-state.hex`** is review 5's canister-encoded fixture from the `1dc2a61` build. It is not the round-5 fixture, which
  differs, and the tests use both.
- **The monitor config:** `launch_file` is new and optional. If missing, the monitor refuses to start (`config.mjs`) rather than
  silently dropping EXPECTED.

## Findings

### D6-L-1 (Low): the unseen-home page sends the operator to `eth_home_clear` for every step, but the canister acts on it only for a Relay deposit or `fundReserve`

- **Code:**
  - `monitor/src/ethnft.mjs:113-117`: the page.
  - `candid.mjs:217`: `HomeTx` without `step`, so the monitor cannot know the step.
  - `ethflow.rs:4183-4195`: `home_clear` accepts any pending step once it has been unseen for 1 h.
  - `ethflow.rs:3565`: the operator's word is read only in the `Ok(None)` arm, the step with nothing to read back.
- **What happens:**
  - The page fires for any pending 4663 step unseen for an hour (a record, an advance, a note, a mint, a post). It always says "if it
    mined, `eth_home_clear(n, true)`; if another transaction took the nonce, `eth_home_clear(n, false)`".
  - For a state-bearing step, the canister's own effect read decides. The operator's word is accepted, and the reply says "is taken
    as MINED at the next tick". It is then never read. It stays in `inbox.home_clear`, which is harmless because the nonce floor never
    comes back to that nonce.
  - A state-bearing step whose effect read keeps failing (the `Err(e)` arm) waits indefinitely. The operator is paged SEV1, makes the
    documented call, gets a success reply, and nothing changes.
- **Impact:** operator confusion during an incident on the one 4663 line. There is no spend: the effect read, not the operator,
  decides these steps, which is the safer design. The page also fires transiently at the same instant the canister drops a
  state-bearing step whose effect is absent (both use 3,600 s). A persistent SEV1 then clears at the next poll.
- **PoC:** `poc/D` test "D6-L-1": the same SEV1 text for any step, naming no step.
- **Fix:**
  - decode `HomeTx.step` in the monitor and say which step it is;
  - name `eth_home_clear` only for `RelayDeposit` / `FundReserve`;
  - for other steps, say "the canister decides by the step's effect; if its read keeps failing, look at the providers";
  - make `home_clear` refuse (not accept) a step it will never read (cross-reference part F).

### D6-L-2 (Low): a 4663 home line held by a provider that shows the transaction as Pending, or by providers that disagree, never pages

- **Code:** `ethflow.rs` `resolve_home`: `EvmView::Pending` and `EvmView::Unsure` set `used_since = None` (F5-M-1's "restart the
  clock"). `ethnft.mjs:113-117` pages only on `used_since` 1 h or older. The monitor's `HomeTx` has no "signed at" time.
- **Example:** one provider keeps a stale pool entry for our transaction, or the two providers alternate on its receipt. The canister
  keeps the transaction pending, rightly never redoing it. `used_since` is reset every tick, so nothing is signed on 4663 again: no
  posts, records, notes or advances.
- **What still pages, indirectly:** the poster pause after 6 h without a good post, delivery deadlines within a day, the pool-idle
  SEV2 after a day. None of them says "the 4663 line is stuck on transaction X".
- **Before round 5:** the round-4 two-sample clock would have paged, then dropped.
- **PoC:** `poc/D` test "D6-L-2": no alert of any kind after an hour, a day or a week with `used_since` empty. A clock reset at 59 min
  never pages.
- **Fix:** keep the time the pending transaction was first signed (`HomeTx.sent_at`, or the monitor's own `firstSeen` keyed by hash).
  Page SEV2 after about 30 min and SEV1 after about 2 h of any one pending home transaction, whatever `used_since` says.

### D6-L-3 (Low): the late hold stops every collection's new Relay orders, and the only page is a non-persistent SEV3 note

- **Code:**
  - `ethflow.rs:655-662`: `relay_busy()` includes `late_hold()`, "whatever its buy's phase".
  - `ethnft.mjs:102-108`: SEV3 `eth-late-order`, not persistent.
  - `ethnft.mjs:66-82`: the SEV1 Stuck page exists only while the buy is `Stuck`.
- **What happens:**
  - The second `eth_give_back` closes the written-off buy as a loss (`Returned`). The SEV1 Stuck page then disappears.
  - The hold stays until a credit or `eth_late_release`. No buy, way back or sweep starts for any collection.
  - The monitor shows only the SEV3 note, plus an INFO `eth-returned`. The pool-idle SEV2 follows a day later for other collections
    whose pools are due, and it names "another collection holding the Relay leg" among three causes.
- **Impact:** liveness only, and operator-induced (the write-off is the operator's own act, and the SEV3 text says the hold). But the
  hold is open-ended, and C4-L-1 was lengthened by it (fixes-round5 §9 call 4). A note that never escalates lets the pipeline sit
  stopped.
- **PoC:** `poc/D` test "D6-L-3" prints `INFO eth-returned`, `SEV3 persistent=false eth-late-order`, and no SEV1 or SEV2.
- **Fix:** while `late_hold()` is set, page a persistent SEV2 "new Relay orders are held by buy N's written-off order", escalating
  after a set time (for example 72 h). Or expose `late_hold` in `eth_nft_state` and page on it directly.

### D6-I-1 (Info): the scan's lag is not paged

- **Background:** fixes-round5 §10 says the canister's reading notes how far the scan is behind the final head. `Reading` in the
  monitor's IDL has no lag field, and nothing pages.
- **What catches a stale floor:** the D8 check (more than 20 % from two references, then pause), and posts stay inside ±5 %/h and
  ±20 %/day.
- **What is lost:** early warning when the sample thins, for example on a shallow provider (see the provider-depth item in part B /
  the summary).
- **Fix:** expose the lag (blocks behind, oldest block read) per collection, and page SEV3 at more than a day and SEV2 when the window
  read is shorter than the 7-day sample.

### D6-I-2 (Info): the Stuck page calls a credited written-off order "still holding the Relay leg"

- **Code:** `ethnft.mjs:75`: `held = late && !released_at`, which ignores `credited`. The canister's `late_hold` (`ethflow.rs:661`)
  needs both absent.
- **Effect:** wrong text only.
- **PoC:** `poc/D` test "D6-I-2".
- **Fix:** `held = late && !credited && !released_at`.

### D6-I-3 (Info): the EXPECTED staged-step read checks only the launch collections' caps and the global item lower bound

- **Code:** `staged.mjs` `offStep` (launch keys only; the global only `>=`); `kindProblem` step 3 (any caps move). The same applies to
  golive-check `--proposal --stage 3` (`checkStagedProposal`).
- **What still counts as EXPECTED (WARN, not SEV1):** a step-3 proposal that also adds a cap for another collection key (up to the
  exit rule's slack: 1 item here, since 62 = 31 × 2), or raises `global_max_value_wei` (75 to 300 ETH).
- **Why this is Info:**
  - an extra key has no flow while `eth_nft` is unchanged, which is required;
  - the per-collection value caps bind before the global value;
  - the argdiff lines still list each LOOSENS, at WARN;
  - a key past the slack fails the exit rule and is not EXPECTED.
- **PoC:** `poc/D` test "D6-I-3".
- **Fix:** in `offStep`, require the proposal's cap keys to be exactly the launch file's, and the global value to equal the sum.

### D6-I-4 (Info): the Trust page's claims, checked against the code

`phase1Trust.ts:20`, `:22`, `:23`, `:143`:

- **Installed numbers and plan:** they match `nft-pilot.json` (48; 12 / 24; then 31; then 61) and the canister's rules.
- **"a late payment is never read as another order's"** (`:22`): true while the hold stands. After the operator's `eth_late_release`,
  a late payment can be read by a new watch. The fixer says so in §2, and the monitor's released-order text says so. The page's
  "never" rests on the operator releasing only on Relay's records. Suggest "unless the operator releases it".
- **"a range too full of transfers … read through the marketplaces' own sale records (Seaport and Blur) … its other sales read
  later"** (`:23`): matches `scan_read` / `defer`. It is part B's to attack.
- **`:143` "three quarters of the week's real sellers"** assumes the canister reads a 7-day sample. If the canister's Ethereum pair
  includes a provider that serves logs only about 10,000 blocks back (publicnode, fixes-round5 §9 call 11), the canister's
  both-provider read covers about 1.4 days. The wash-trading bound is then over a much smaller real-seller set. That claim is only true
  with a provider pair deep enough for the window (see the summary's provider recommendation).

### D6-I-5 (Info): runner and ops unchanged

The runner neither reads the hold nor the pending home transaction. Neither is needed to act. Noted for completeness: the runner hands
in no quote while `relay_busy()` holds, because no buy reaches `Funded`.

## Carried items, re-rated

| ID | Re-rate |
| --- | --- |
| D5-L-1 | **Closed** (above) |
| D5-L-2 | **Closed** (above); residual D6-I-2 |
| D5-I-1 (only the newest version of a re-priced delivery read) | Unchanged, Info |
| D5-I-2 | Unchanged, Info |
| D5-I-3 | Partly (wording fixed; the late payout's landing is still not paged) |
| D-L-3 (runner's 7,200-block window) | Unchanged, Low |
| D-L-6 (the canister never reads `custodian()`) | Unchanged, Low (no services change touches it) |

## Tests

- Worktree `monitor`: **203/203**.
- Copy: `ops/test/audit_nft_r5.test.ts` 1/1.
- `poc/D/audit6_nft_d.test.mjs`: **6/6 pass**. The four WEAKNESS tests pass because the weaknesses are present. The IDL test and
  D6-I-3's control pass as the checks they are.
