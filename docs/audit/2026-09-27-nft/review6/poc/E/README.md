# Review round 6, part E: PoCs

All ran in a private APFS clone of `autobuyer/canister-rs` at `/tmp/rook-r6-E/canister-rs` (checked with `diff -rq --exclude=target`
against the worktree at `e04fa16`; it differed only by the appended tests and fixtures below) and removed afterwards. Toolchain 1.98.1.

| File | How | Result |
| --- | --- | --- |
| `r6_e_h3_guard.append.rs` | appended to the copy's `tests/audit_nft_r5.rs` (uses its private `h3_check` / `src_files`) | **E6-I-1**: 5 mutations the E5-I-2 guard accepts. `r6_e_h3_guard.output.txt` |
| `r6_e_chain_moved.append.rs` | same | **E6-I-2**: a two-step chain move keeps chain 1's floors. `r6_e_chain_moved.output.txt` |
| `dump_saved_1dc2a61.append.rs` | appended to `tests/audit_nft_r4.rs` of a `git archive` export of `1dc2a61` (its sources touched so cargo rebuilt them); writes `saved-1dc2a61-rich.hex` | a whole `(Saved,)` written by the 1dc2a61 build, every round-4 Ethereum field present, `Inbox.payouts` too. The same run passed 1dc2a61's own `r4_the_round_4_state_fixture_is_what_the_canister_encodes`, so `tests/fixtures/state/eth-state-1dc2a61.hex` is byte for byte what 1dc2a61 encodes |
| `r6_e_restore.append.rs` | appended to the copy's `tests/audit_nft_r5.rs`, with `saved-1dc2a61-rich.hex` in `tests/fixtures/state/` | that state restores in HEAD, `lost_elements` and `lost_opts` empty, every round-5 field absent; the largest round-5 probe (519 kB) decodes in 23 ms (debug). `r6_e_restore.output.txt` |
| `r6_e_old_fixtures.append.rs` | same, with 8ef82d6's `eth-state-round2.hex` / `eth-state-round4.hex` as `old-round{2,4}.hex` | the pre-round-5 bytes decode in HEAD with nothing lost and equal the re-encoded fixtures' values. `r6_e_old_fixtures.output.txt` |
| `didcheck6.main.rs` | a crate next to the copy (`candid` 0.10.36, `candid_parser` 0.1.4, `etch_autobuyer` by path) | HEAD `.did` service-compatible with 8ef82d6's; every field of the round-5 / round-4 / round-2 fixtures the same in the Rust encoder and through the `.did`, both directions; control against 8ef82d6's `.did` names the round-5 fields. `didcheck6.output.txt` |
| `audit_nft_r5_with_pocs.output.txt` | the whole `audit_nft_r5` test file with the PoCs appended | 38 passed (the fixer's 33 and these 5) |
