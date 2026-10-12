//! Clocks (docs/v9/solana-migration.md §2.4 "Clocks"). The EVM's `Wait` library required BOTH `block.timestamp` and the
//! parent chain's `block.number` to pass every wait, because Arbitrum's sequencer could stamp an hour ahead. Solana has no
//! parent chain; `Clock::unix_timestamp` is the stake-weighted median of validator votes, so every deadline here is unix time
//! alone, and slots are used for nothing but ordering.

use anchor_lang::prelude::*;

pub const MINUTE: i64 = 60;
pub const HOUR: i64 = 3_600;
pub const DAY: i64 = 86_400;

/// Now, unix seconds (never negative).
pub fn now() -> Result<i64> {
    Ok(Clock::get()?.unix_timestamp.max(0))
}

/// Seconds since `at`, 0 if `at` is in the future. Both of the Solidity's `_slowElapsed` and `_fastElapsed` (the lesser and
/// the greater of the two clocks) become this.
pub fn elapsed(now: i64, at: i64) -> i64 {
    if now > at {
        now - at
    } else {
        0
    }
}

/// The drand evmnet chain (src/rand/DrandEvmnet.sol:16-17): genesis and period.
pub const DRAND_GENESIS: i64 = 1_727_521_075;
pub const DRAND_PERIOD: i64 = 3;

/// `DrandEvmnet.roundAt`: the round live at `t` (0 before genesis).
pub fn round_at(t: i64) -> u64 {
    if t < DRAND_GENESIS {
        0
    } else {
        ((t - DRAND_GENESIS) / DRAND_PERIOD) as u64 + 1
    }
}
