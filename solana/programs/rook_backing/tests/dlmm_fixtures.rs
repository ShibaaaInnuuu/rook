//! Shim: the W7 DLMM fixtures live in solana/tests/dlmm_fixtures.rs (auto-discovered here so Cargo.toml stays untouched).
//! Run: `cargo test -p rook_backing --test dlmm_fixtures -- --nocapture`
#[path = "../../../tests/dlmm_fixtures.rs"]
mod fixtures;
