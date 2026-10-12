# Review round 7, part E: PoCs

Ran in the part-F copy `/tmp/rook-r7-F` (`git archive a78a542 autobuyer/canister-rs`, `diff -rq --exclude=target` identical before
anything was added; removed afterwards). Toolchain 1.98.1.

| File | How | Result |
| --- | --- | --- |
| `r7_e_old_fixtures.rs`, `r7_e_old_fixtures.output.txt` | a new test file in the copy; reads every historical version of `eth-state-round{2,4,5,6}.hex` from git (`git show <commit>:…`, 16 byte strings, 24fba50 .. 2d747ba) | each decodes under HEAD, re-encodes with `lost_opts` empty, the round-6 fields absent (present only in round 6's own); bb713fe's round-2/4/5 bytes hold the SAME values as HEAD's re-encoded fixtures |
| `didcheck7.main.rs`, `didcheck7.output.txt` | review6's `didcheck6` crate (candid 0.10, candid_parser 0.1.4, `etch_autobuyer` by path), with the round-6 fixture added and the old `.did` a parameter | HEAD `.did` `service_compatible` with bb713fe's and e04fa16's (identical); every field of the round-6/5/4/2 fixtures the same through the `.did` and the Rust encoder; the control through the old `.did` names the round-6 fields |
| `r7_e_h3_guard.append.rs`, `r7_e_h3_guard.output.txt` | appended to the copy's `tests/audit_nft_r5.rs` (its private `h3_check` / `src_files`) | the round-6 guard catches a helper-fn key, a macro key, a struct-field key and a closure; it accepts five others (**E7-I-1**): an inner-block decoy `let`, a type-annotated `let` after a decoy, `key_of`'s body made per-call, a `#[path]` module and an `include!` outside `src/`. None is present. The audit_nft_r4/r5/r6 suites pass with the PoCs appended (22 / 34 / 17) |
