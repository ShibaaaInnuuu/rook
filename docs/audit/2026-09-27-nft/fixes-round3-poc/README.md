# The round-3 reviewers' PoCs, re-run on fix round 3

Written 2026-09-27 on branch `nft-side` (worktree `/Users/jlohayon/Documents/Rook-nft`, fix round 3 uncommitted in the tree). Every
PoC in `../review3/poc/{A,B,C,D,E}` passed on the code the reviewers read (each asserts a weakness). Here each one is run on the new
code with only the compile adaptations the new APIs need. No attack assertion was touched. Each ran in a private copy of its harness
outside the worktree; nothing in the tree was changed but this folder.

"Fails on its attack assertion" means the fix holds. "Passes" means the weakness is still there, or the test asserts something that
still holds (a control, a "what holds" probe). The code run is a snapshot of the tree taken at the start. It was checked again
at the end: the canister `src/`, the tests used as harnesses, the `.did`, `monitor/src/`, `script/golive-check.sh` and `src/` are
identical to the tree, apart from a doc-comment change in `relayeth.rs`. Line numbers are the PoC file's own. `.output.txt` is each run with the canister's
per-tick `  | ` log lines removed.

## How each harness was built

| PoC | Harness run | Compile adaptations |
| --- | --- | --- |
| `C/r3_c.rs`, `E/audit3_nft_e.rs` | The CURRENT `tests/audit_nft_r2.rs` harness (up to its first `#[test]`), with the PoC's own harness additions re-applied (C: `real_outflow`; E: `pin_back`, `fee_of`), then the PoC's tests unchanged | `give_back_now(.., f.now)` (C 5 calls, E 2), `Watch { .., pin: None }` (C 1). The harness hunks in the diff are round 3's plumbing, already in the current harness: `our_logs` (filled in `mine_eth` from our `transferFrom`, answered first by `eth_sale_receipt`, since the canister now judges a delivery at finality by its own receipt), `noted` and `NOTE_SENT` in `apply_home` (both wrappers), `log_ranges`, `FUND_RESERVE`, the config's `gas_pot_target_wei` / `logs_max_blocks`. `r3_c.adaptations.diff`, `audit3_nft_e.adaptations.diff` |
| `A/audit_nft_a3_canister.rs` | The CURRENT `tests/audit_nft_r2.rs`, whole, with the PoC appended (as the reviewer did) | `give_back_now(.., f.now)`. `audit_nft_a3_canister.adaptations.diff` |
| `B/b3_nft_flood.rs.snippet` | The CURRENT `tests/audit_nft_r2.rs` with the snippet appended | none |
| `D/canister/d3_h1_append_to_tests_ethnft.rs` | The CURRENT `tests/ethnft.rs` (which already has `our_logs` / `NOTE_SENT`) with the test appended | none |
| `D/monitor/test/audit3_nft_d.test.mjs` | A copy of the current `monitor/`, the file dropped into `test/` | none. Its fake world does not answer the wrapper's `ticket(uint256)`, which round 3's monitor reads before it exempts a delivery. When that read fails, the monitor pages, by design. It was noted here, not added to the PoC. A separate diagnostic adds it (below) |
| `A/AuditNftA3.t.sol` | A copy of the forge project (`src/ test/ script/ lib/ cache/ out/`, `foundry.toml`, remappings), the file at `test/audit_nft_a3/` | none |
| `A/golive_harness.sh` | Run against a copy of the current `script/golive-check.sh` and `deployments/launch/` | The functions re-extracted at their round-3 lines (`lc`, `same` at 72, 75-78; `launchval` through `monglobal` at 542-651). `launchcheck`'s new 5th argument (the wrapper). The stubbed chain now reads back the operator's J1 values (BAYC's factory at 10 ETH, its module at convBps 1000 / reserveBps 0) and answers the new reads: each wrapper's `name()` / `symbol()` from the committed launch file, and `registry.guardian()` (`$G`) as the filled guardian. `golive_harness.adaptations.diff` |
| `E/audit3_nft_e_upgrade.rs` | The current crate, with `saved-79455e1-rich.hex` copied into `tests/fixtures/` (release, as the reviewer ran it) | `SaleScan { .., pend/rescan/judged: None }`, `EthBuy { .., sweep/arrived_at/back_at/ended_at: None }`. `audit3_nft_e_upgrade.adaptations.diff` |
| `E/didcheck` | With the private crate copy as its path dependency | `Watch.pin`, the four new `EthBuy` fields, `Delivery.sent` and the three new `SaleScan` fields, all `None`. `didcheck.adaptations.diff` |
| `E/dump_state_79455e1.rs` | not re-run | It writes the 79455e1 build's saved state (`saved-79455e1-rich.hex`, which the upgrade test reads). It describes the old build, not this one |

## Results

| PoC test | Finding | On fix round 3 |
| --- | --- | --- |
| **C** `c3_h1_an_advance_waiting_on_the_value_limit_…_trips_the_freeze` | C3-H-1 | **Fails on its attack assertion** (line 828: `assert!(o.frozen().is_some(), "the tripwire froze custody …")`). After 100 ticks H3 holds one ask under a stable key, `advance:eth:1:0x5af0…:0`, and nothing is frozen |
| C `c3_m1_a_way_back_with_no_quote_stops_every_collections_buys_…` | C3-M-1 | **Fails on its attack assertion** (line 636: `… BPhase::BackFunded { .. }), "still waiting for a quote"`). After 7 days Milady is `Arrived` again and Azuki's buy is `Funded` |
| C `c3_m1_a_way_back_past_its_deadline_…_holds_the_relay_leg_for_good` | C3-M-1 | **Fails on its attack assertion** (line 669: `… BPhase::BackBridging { .. })`). Milady is `Stuck` ("no longer watched; … eth_give_back(0) closes it") and Azuki's buy is `Funded` |
| C `c3_m2_the_runners_operator_key_churns_…` | C3-L-2 | **Fails at the attack's own step** (line 703: `ef::give_back_now(&mut st, id, 0, f.now).unwrap()`): "refused: buy 0 arrived 0 s ago; it may be sent back once it has waited 21600 s for a listing (C3-L-2)" |
| C `c3_l1_a_quote_the_judge_accepts_arrives_under_the_floor_the_gate_passed` | C3-L-1 | **Passes: the weakness remains** where the item cap binds, as in this PoC. The operator's round grosses the advance up for the judged 3 % (`advance_bound`), but the gate (`ADVANCE_FEE_EST_BPS`) still assumes 50 bps. `fixes-round3.md` lists C3-L-1 as "Partly". "the judged quote promises 488475083200866055 of 500000000000000000 (230 bps under); the gate assumed 50" |
| C `c3_l2_a_4663_inflow_during_the_way_back_is_returned_as_the_arrival_…` | C3-L-3 | **Fails on its attack assertion** (line 775: `assert!(until(…, BPhase::Returned { .. })))`). An inflow of half the minimum is no longer read as the way back's arrival (`BACK_ARRIVAL_SLACK_BPS`) |
| **E** `e3_m1_a_pin_that_moves_back_reads_a_counted_fill_as_another_collections_arrival` | E3-M-1 | **Fails on its attack assertion** (line 602: `let BPhase::Arrived { wei, .. } = az.phase else { panic!("no phantom: …") }`). Azuki stays `Bridging`: "waits: the Ethereum pin 26068920 is below block 26068928 …: no balance is read there (E3-M-1)" |
| E `e3_m1_a_watch_taken_at_a_pin_that_moved_back_never_sees_the_real_arrival_…` | E3-M-1 | **Fails on its attack assertion** (line 645: `….last.clone().expect("watched")`). No watch is taken at the lower pin |
| E `e3_l1_a_fill_that_reverts_restarts_the_arrived_hold_…` | E3-L-1 | **Fails on its attack assertion** (line 712: `assert_eq!(st.deposits[0].phase, XPhase::Mint, …)`). The deposit is `Minted` after 17 waiting ticks. The log also shows the way back starting three times; the PoC hands in no way-back quote, so each attempt returns to `Arrived` after its 2 h window. Its later assertion "the 48 h way back never started" checks only the final phase |
| E `e3_l2_gas_drift_into_an_earmark_deadlocks_the_next_arrival_with_the_way_back` | E3-L-2 | **Strict run: fails at a SETUP assertion**, not the attack (line 743: `assert!(matches!(az.phase, BPhase::Funded { .. }))` finds `Advancing`). Round 3's `noteSent` takes the home line for one tick, so Azuki's advance comes a tick later. **Tolerant run** (one more tick, `audit3_nft_e.e3_l2-tolerant.diff`): fails at line 770, "Milady waits for the leg", because Milady is `Arrived` again after its quote window (C3-M-1's fix). **Diagnostic** (that one assertion printed instead of asserted): every later assertion holds. Azuki's arrival is refused 720 times in 10 days ("… holds only 495443962449863321 beside the other buys' earmarks: nothing is earmarked on ETH that is not there"). Azuki stays `Bridging` past its deadline, `relay_busy` is true, `buy_holds_mint(AZUKI)` is true, and only a 0.01 ETH top-up frees it. **The deadlock remains**; round 3 did not take up E3-L-2 |
| E `e3_l3_a_post_arrival_stuck_buy_is_open_for_ever_with_no_exit` | E3-L-3 | **Fails on its attack assertion** (line 819: `assert!(give.is_err(), "no operator exit")`). `eth_give_back` now closes it: "closed as a loss: returnAdvance of 0 at the next tick …" |
| E upgrade: `e3_probe_the_audited_e04f49b_state_and_a_rich_79455e1_state_…`, `e3_i_probe_gaps_…`, `e3_probe_cost_on_a_large_state` | E3 (what holds), E3-I | **Pass** (they assert what holds): both saved states restore with nothing lost. The probe takes 345 ms on 3.3 MB (release) |
| E `didcheck` | — | **All ok**: the `.did` parses (106 types), the round-2 fixture decodes, and every old case decodes. A diagnostic (`didcheck.diag.output.txt`) adds round 3's `Step::Sent` and every new optional field present: ok |
| **B** `b3_m1_control_no_flood_the_five_real_sales_confirm_the_floor_…` | control | **Passes** (control): 5 sales counted, confirmed, minted |
| B `b3_m1_a_plain_transfer_flood_permanently_starves_real_sales_…` | B3-M-1 | **Fails on its attack assertion** (line 84: `assert_eq!(cached_reals, 0, "WEAKNESS: the flood starved every real sale …")`: 5). All five real sales are read and counted, the floor is confirmed (`held None`), and the deposit minted |
| B `b3_m1b_at_first_post_accepted_zero_the_flood_blocks_the_floor_post_entirely_…` | B3-M-1 | **Fails on its attack assertion** (line 108: `assert_eq!(cached_reals, 0, "the flood starved the real sales")`: 5). The module was posted 20 times |
| **D** canister `d3_h1_a_redeemer_who_moves_the_nft_before_finality_is_never_recorded_…` | D3-H-1 | **Fails on its attack assertion** (line 27: `assert!(matches!(st.deliveries[0].phase, DPhase::Moved { .. }))` finds `Recorded`). `noteSent` first, then "final, and its receipt shows the token delivered to the destination: recordDelivery is next" |
| D monitor "D3-M-1" (Sent / Moved / Record / Recording, NFT resold) | D3-H-1, the monitor half | **Passes, for a harness reason.** The PoC's world does not answer `ticket(uint256)`, so the exemption's ticket read fails and the monitor pages, by design ("ticket 3 of the wrapper could not be read …: not exempted"). **Diagnostic** with `ticket(3)` answered (`audit3_nft_d.diag.*`): Moved, Record and Recording no longer page. **Sent with the NFT already resold still pages SEV1**: a delivery whose transfer mined, where the canister has not yet ticked it to `Moved` |
| D monitor "D3-M-1 (b)" (Cancelling, owner = destination) | D3-I-7 | **Passes, for the same harness reason.** With `ticket(3)` answered: no page (fixed) |
| D monitor "D3-L-1" | D3-L-1 | **Fails on its attack assertion** (line 90: `assert.ok(!notHeld(r), "… masked by the old record")`) |
| D monitor "D3-L-2" | D3-L-2 | **Fails on its attack assertion** (line 113: `… eth-arrived-gone:${CUSTODIAN}`, "an honest arrival reads as ETH stolen") |
| D monitor "D3-I-1" | D3-I-1 | **Fails on its attack assertion** (line 122: `a.key.endsWith(":0") && /30000000 min/`) |
| D monitor "D3-I-2" | D3-I-2 | **Passes: the weakness remains** (an open Info; round 3 did not take it up). A foreign transaction at a `CancelFill` line's nonce still pages no SEV1 |
| **A** forge `test_A3_L1_repeatedWayBacks_eatTheBAYCPoolUntilTheGateStops_weakness` | A3-L-1 | **Passes, as expected.** The Floor Module's `returnAdvance` is unchanged on chain; round 3's fix is the canister's `RETURN_COOLDOWN_SECS` (24 h), shown by the canister PoC below |
| A forge `test_A3_L1_oneWayBackAtTheJudgesFeeBound_weakness` | A3-L-1 | **Passes, as expected** (same reason) |
| A forge `test_A3_control_aShortReturnIsTakenFromRevenueThenSurplusThenTheUnits` | control | **Passes** (control) |
| A forge `test_A3_I1_pooledFloorRiseTest_theThirdGraduationBranchIsDead` | A3-I-1 | **Passes** (a test-side Info; unchanged) |
| A canister `a3_m1_the_way_back_repeats_without_bound_each_round_trip_returned_short` | A3-L-1 (canister half) | **Strict run: fails at the give-back step** (line 29: `ef::give_back_now(st, id, 0, f.now).unwrap()`, refused "(C3-L-2)"), not at A3-L-1's own mechanism. **Tolerant run** (it waits `ARRIVED_HOLD_SECS` before the give-back, `audit_nft_a3_canister.tolerant.diff`): cycle 0 completes, then **it fails on its attack assertion** in cycle 1 (line 12: `assert!(matches!(phase(st), BPhase::Funded { .. }))` finds `Returned`). No new buy of the collection starts after a return (`RETURN_COOLDOWN_SECS`) |
| A golive control (the committed files) | — | With the PoC's own template fill: **FAIL**, "a <placeholder> is left in the monitor file (timelock, buyer, expect.*, install_arg_hex and 3 more)". Round 3 refuses a placeholder anywhere (A3-L-3), and the PoC fills only the wrapper, module, custodian and guardian. With the template filled as round 3's `scripts/golive-dryrun.sh` fills it (`golive_harness.filled.*`): no FAIL |
| A golive case 1 (names / symbols / slugs rotated) | A3-L-2 | **FAILs** (the fix): 15 FAIL lines, one each for `name()`, `symbol()` and the slug of every collection |
| A golive case 2 (D8 references swapped between Milady and MAYC) | A3-L-3 | **Passes: the weakness remains.** With round 3's fill, no FAIL line; with the PoC's fill, only the placeholder FAIL. The OpenSea slug and CoinGecko id are still not held against anything by address |
| A golive cases 3-5 | A3-L-3 | **FAIL** (the fix): providers "counted by host: 1"; "evm.chain_id is 1, not 4663"; "evm.guardian.address is registry.guardian(): expected 0x1111…, the chain says 0x…dead"; and, with the PoC's fill, the placeholders. The case's `evm.stack_file = "../stack-1.json"` is still not read |
| A golive case 6 (no `pause_on_no_good_post_h`) | control | **FAILs** (the control holds) |

## What stays open, by these runs

Each item agrees with `../fixes-round3.md` (E3-L-2 "Not fixed"; C3-L-1 and A3-L-3 "Partly"; D3-I-2 unchanged). Item 5 follows
from the exemption rule it describes (`Sent` is exempt only at the destination), but it is not listed there as a residual.

1. **E3-L-2 (Low).** The strict run hides it behind a setup-timing failure. In the tolerant run, an arrival still waits without end
   on an earmark cap after gas drift: Azuki is `Bridging` past its deadline, holds the Relay leg and its collection's deposit
   mints, and only a top-up frees it. C3-M-1's fix changed one part: the other buy no longer holds the leg.
2. **C3-L-1 (Low), where the item cap binds.** The advance gate still assumes 50 bps for Relay, while the judge accepts 300.
3. **A3-L-3, partly.**
   - Swapped D8 references still pass `--monitor-config`.
   - `evm.stack_file` is not read.
4. **D3-I-2 (Info).** A foreign transaction at a `CancelFill` nonce is not paged.
5. **Monitor D3-H-1 residual.** A delivery still `Sent` in the canister's records whose NFT is already with someone other than the
   destination pages SEV1. This is the window between our transfer mining and the canister's next tick. Every mined phase is exempt
   once the ticket read succeeds.

## Files

| File | What |
| --- | --- |
| `r3_c.output.txt`, `r3_c.adaptations.diff` | C |
| `audit3_nft_e.output.txt`, `audit3_nft_e.adaptations.diff` | E, strict |
| `audit3_nft_e.e3_l2-tolerant.output.txt`, `audit3_nft_e.e3_l2-tolerant.diff` | E3-L-2 tolerant, then the diagnostic |
| `audit3_nft_e_upgrade.output.txt`, `audit3_nft_e_upgrade.adaptations.diff` | E, upgrade probe |
| `didcheck.output.txt`, `didcheck.adaptations.diff`, `didcheck.diag.output.txt`, `didcheck.diag.diff` | E, the `.did` check and its round-3 diagnostic |
| `b3_nft_flood.output.txt` | B |
| `d3_h1_append_to_tests_ethnft.output.txt` | D, canister |
| `audit3_nft_d.output.txt`, `audit3_nft_d.diag.output.txt`, `audit3_nft_d.diag.test.mjs` | D, monitor, and the diagnostic with `ticket(uint256)` answered |
| `audit_nft_a3_canister.output.txt`, `audit_nft_a3_canister.adaptations.diff`, `audit_nft_a3_canister.tolerant.output.txt`, `audit_nft_a3_canister.tolerant.diff` | A, canister |
| `AuditNftA3.output.txt` | A, forge |
| `golive_harness.output.txt`, `golive_harness.adaptations.diff`, `golive_harness.filled.output.txt`, `golive_harness.filled-variant.diff` | A, go-live checks, with the PoC's fill and with round 3's fill |
