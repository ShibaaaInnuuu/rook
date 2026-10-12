# Review 7, part B PoCs

`tests/r7_b.rs` = `tests/audit_nft_r6.rs` (HEAD a78a542) + three switches in its fake world (`deep_once`, `receipt_busy`,
`diverge_pct`) + four tests at the end (`r7_b_*`). `r7_b.vs_audit_nft_r6.diff` is the whole difference.

Reproduce in a private copy of `autobuyer/canister-rs` (toolchain 1.98.1):

    cp -R autobuyer/canister-rs /tmp/x && cp autobuyer/rust-toolchain.toml /tmp/ && cd /tmp/x
    cp tests/audit_nft_r6.rs tests/r7_b.rs && patch tests/r7_b.rs < r7_b.vs_audit_nft_r6.diff
    cargo test --test r7_b r7_ -- --nocapture --test-threads=1

A passing test shows the weakness (it turns red once fixed), except `r7_b_i1_a_…`, which only measures.
