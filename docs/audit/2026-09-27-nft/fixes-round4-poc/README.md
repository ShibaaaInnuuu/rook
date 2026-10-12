# The round-4 reviewers' PoCs, re-run on fix round 4

Written 2026-09-28 on branch `nft-side` (fix round 4 at `548f214`, docs after it). Every PoC in `../review4/poc/{A,B,C,D,E}` passed on
`1eb5788` (each asserts a weakness). Here each one is run on the new code, in a private copy of its harness outside the worktree
(`/tmp/rook-r4/`, removed afterwards), with only the compile adaptations the new types need. No attack assertion was touched.

"Fails on its attack assertion" means the fix holds. "Passes" means the weakness is still there, or the test asserts something that
still holds. `canister.*.txt` are the runs with the canister's per-tick `  | ` lines removed; `canister.summary.txt` lists them all.

## How each harness was built

| PoC | Harness | Compile adaptations |
| --- | --- | --- |
| `C/r4_c.rs` | the CURRENT `tests/audit_nft_r3.rs` whole, `r4_c.harness.diff` applied, the PoC appended (as the reviewer did) | none |
| `E/audit4_nft_e.tests.rs` | the same, with `audit4_nft_e.harness.diff` | none |
| `B/b4_h1.rs.snippet` | the CURRENT `tests/audit_nft_r3.rs` up to its first `#[test]`, `b4_poc.harness.diff`, the snippet appended | none |
| `A/a4_noted_reorg_append_to_tests_ethnft.rs` | the CURRENT `tests/ethnft.rs` with the test appended | none |
| `A/a4_exit_fits_global.rs` | dropped into `tests/` | none |
| `E/audit4_nft_e_upgrade.rs` | the crate, with `E/fixtures/*.hex` in `tests/fixtures/` | `SaleScan { .., bad: None, skipped: None }`, `EthBuy { .., late: None }` |
| `A/install_check.sh` | the CURRENT `script/golive-check.sh`'s `INSTALL_PY`, extracted as the script does | none |
| `D/monitor/test/audit4_nft_d.test.mjs` | a copy of the current `monitor/` (and `scripts/`, which `samples.mjs` imports) | none |

`dump_state_1eb5788.rs` is review4's `E/dump_state_old.rs` with round 3's fields filled, run on a `git archive` export of `2735dac`
(= `1eb5788`'s canister): it wrote `tests/fixtures/state/saved-1eb5788-rich.hex`, which `tests/audit_nft_r4.rs` restores.

## Results

| PoC | Finding | On fix round 4 |
| --- | --- | --- |
| C `c4_h1_a_4663_pin_one_bucket_back_…` | C4-H-1 | **Fails on its attack assertion** (line 1283: `l.contains("REFUNDED")`). The lower pin is refused: "the 4663 nonce at this pin is 1, under 2 (past our last mined home transaction) … (C4-H-1)" and nothing of 4663 is read or signed that tick |
| C `c4_h1b_any_home_act_signed_at_a_lower_pin_…` | C4-H-1 | **Fails at the attack's step** (line 1330: `st.pending.clone().expect("the next post is out")`): nothing is signed at the used nonce |
| C `c4_m1_…`, `c4_m1b_…` | C4-M-1 | **Never reach the attack**: their setup loops `while !Stuck`, and a way back is no longer let go by itself (it stays watched until its payout is read or the operator writes it off). Killed after 60 s. The attacks themselves are turned around in `tests/audit_nft_r4.rs` `r4_c4_m1*` |
| C `c4_l1_…` | C4-L-1 | **Passes**: the Relay leg is held for the order's deadline (and now until the operator writes it off: §5 of `../fixes-round4.md`) |
| E `e4_h1_nine_deposits_…_pause_the_canister` | E4-H-1 | **Fails on its attack assertion** (line 1354: `f.paused && o.frozen().is_some()`): 3 minted, the waiting mint one ask, no freeze |
| E `e4_m1_a_home_pin_that_moves_back_…` | E4-M-1 (= C4-H-1) | **Passes in this harness**, for a harness reason: its `home_pin_back` raises the 4663 balance and nothing else (the nonce and the pin are unchanged), so the fake world is indistinguishable from a real refund. On the IC the pin floor lives in `canister::pin` (a pin below one already read is an error) and the world says its pin (`EthIo::home_pin`); `r4_e4_m1_…` in `tests/audit_nft_r4.rs` runs the same path with the pin said, and C's `c4_h1` (which moves the nonce too) fails above |
| E `e4_l1_…`, A `a4_exit_fits_global.rs` | A4-L-1 | **Pass**: they assert `outflow::exit_fits` of the launch numbers, which is unchanged (61 items at 24 a day do not fit). The fix is that the install check now applies it to the whole custody (`canister.rs` `validate_custody`, test `a4_l1_the_whole_custody_…`) and golive-check refuses the pair |
| E controls, E upgrade (2) | — | **Pass** (what holds): old states restore with nothing lost |
| B `b4_h1_a_…`, `b4_h1_b_…` | B4-H-1 | **Both fail on their attack assertions** (line 737: 5 of 5 real sales read, not 3; line 778: the newer sales are read, not 0) |
| A `a4_l2_a_noted_delivery_dropped_by_a_reorg_…` | A4-L-2 | **Fails on its attack assertion** (line 1486: `sent == Some(true)`): nothing is noted before finality |
| A `install_check.sh` | A4-L-3 | Cases 1-5, 7, 9, 10 refused; case 4 prints `https://….example.net/…` (never the key); the control passes. Case 6 (`wss://`) is still unread (harmless: not an outcall). Case 8 (one host twice) is refused in the full golive-check by the per-chain host count (the extracted Python alone has no decoded `Init`, hence its "not decoded" note) |
| D "F3-1 (what holds)" | — | **Passes** |
| D "D4-L-1", "D4-L-2", "D4-I-1", "D4-I-2" | D4-L-1, D4-L-2, D4-I-1, D4-I-2 | **All four fail on their weakness assertions** (`D.audit4_nft_d.output.txt`) |
