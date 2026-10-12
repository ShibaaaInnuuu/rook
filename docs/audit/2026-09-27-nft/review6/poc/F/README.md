# Review 6, part F: PoCs

Run in a private copy of `autobuyer/canister-rs` (`/tmp/rook-r6-F`, an APFS clone checked identical to the worktree at `e04fa16` with
`diff -rq`; removed afterwards), toolchain 1.98.1.

| File | What |
| --- | --- |
| `r6_f.harness.diff` | `tests/audit_nft_r5.rs` copied whole to `tests/r6_f.rs`, with three switches (`live_pin`: the 4663 pin advances 10 blocks a second; `state_depth`: `home_nonce_tx` fails when `HomeTx.pin` is further back than the providers keep state; `mint_estimate_refuse`: every `mintDeposit` estimate reverts `ListingPaused`) and the five `r6_*` tests appended |
| `r6_f.output.txt` | `cargo +1.98.1 test --test r6_f r6_`: **5 passed** (each shows its weakness); the whole copied harness: 38 of 38 pass (the fixer's 33 unaffected by the switches) |
| `depth_probe.sh`, `depth_probe.output.txt` | Read-only probe of the canister's two 4663 providers (`scripts/golive-dryrun.sh:115`) and `robinhood.drpc.org`: state depth, block depth, block time |

Rebuild in a private copy: `cp tests/audit_nft_r5.rs tests/r6_f.rs && patch tests/r6_f.rs < r6_f.harness.diff`, then
`cargo +1.98.1 test --test r6_f r6_`.

| Test | Finding | It shows |
| --- | --- | --- |
| `r6_f6_l1_at_the_runners_cadence_…` | F6-L-1 | At a 600 s runner sleep plus a 60 s cycle, the first resolving tick's pin is 6,600 blocks past `HomeTx.pin`; with 6,200 blocks of state, a hidden Relay deposit is never identified (a whole day), always `UNIDENTIFIED` |
| `r6_f6_l1_control_…` | F6-L-1 control | At a 60 s cadence, or on an archive provider, the same deposit is identified at the first tick (MINED, Bridging) |
| `r6_f6_l2_one_provider_that_disagrees_…` | F6-L-2 | A mined deposit whose receipt the providers disagree on: a day with `used_since` never set, `eth_home_clear` refused, no post, mint or record signed, identification never tried |
| `r6_f6_i1_a_two_step_chain_move_…` | F6-I-1 | Dropping `eth_nft`, then re-adding it on another chain, keeps the old chain's Ethereum floors: every pin and nonce of the new chain is refused |
| `r6_f6_i3_a_refused_mint_estimate_…` | F6-I-3 | Four Milady deposits waiting at `Mint` while every mint estimate reverts: all four admitted to the custody book, none minted |
