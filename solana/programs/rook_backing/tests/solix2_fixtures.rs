//! Shim: the W2 fixture dumper lives in solana/tests/solix2_fixtures.rs (auto-discovered here so Cargo.toml stays untouched).
//! Run: `cargo test -p rook_backing --test solix2_fixtures -- --nocapture`
#[path = "../../../tests/solix2_fixtures.rs"]
mod solix2;
