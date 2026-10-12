//! Shim: spike S1 lives in solana/tests/s1_canister_only.rs (auto-discovered here so Cargo.toml stays untouched).
//! Run: `cargo test -p rook_backing --test s1_canister_only -- --nocapture --test-threads 1`
#[path = "../../../tests/s1_canister_only.rs"]
mod s1;
