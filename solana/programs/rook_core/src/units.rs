//! Unit conversions of the ported contracts (docs/v9/solana-migration.md §3 "Amounts"). Only true conversions live here:
//!
//! - **Units** (one wrapped item): the EVM's `1e18` becomes [`ONE`] = 10^9, the 9 decimals of the `wX` SPL mint. An amount of
//!   units is a `u64` (max ≈ 1.8·10^10 whole items: never reached).
//! - **Money**: wei (1e18 per ETH) becomes lamports (1e9 per SOL), and the denomination changes from ETH to SOL (§2.3).
//!
//! No exchange rate is here (decision D-c, 2026-10-09, solana-migration.md §2.9): every ETH-denominated value of the EVM
//! (`revenueTarget`, the wake bond and its bounds) is an explicit parameter of the instruction that fixes it (`init_module`'s
//! [`crate::params::Params::revenue_target`], `init_desk`'s `DeskParams`), and the launch file carries it already converted at
//! the live SOL/ETH rate (`ops/solana/launch-params.mjs`, which records each value's ETH origin and the rate used).
//!
//! | EVM (source) | value | here | value |
//! |---|---|---|---|
//! | `ONE`, `ONE_ITEM` (FloorModule:111, EthNftWrapper:58) | 1e18 | [`ONE`] | 10^9 |
//! | `defaultParams.mintPerHour` / `burnPerHour` (StackChecks:180-181) | 5·1e18 | `mint_per_hour` / `burn_per_hour` | 5·10^9 |
//! | `defaultParams.maxPublicEthBacked` (StackChecks:182) | 10·1e18 | `max_public_eth_backed` | 10·10^9 |
//! | `defaultParams.maxEthBacked` (StackChecks:183) | 40·1e18 | `max_eth_backed` | 40·10^9 |
//! | `defaultParams.revenueTarget` (StackChecks:184) | 1 ether | `revenue_target` | lamports, from the launch file |
//! | `WakeDesk` bond, `MIN_BOND`, `MAX_BOND` | 0.002 / 0.0005 / 1 ether | `DeskParams` | lamports, from the launch file |
//! | `FloorModule.ACC` (rentals) | 1e18 | — | dropped with rentals |
//! | floor price (wei per whole item) | wei | `accepted`, `target` | lamports per whole unit (10^9) |
//!
//! Everything in basis points (spreads, `convBps`, `reserveBps`, the ±20 %/day clamp...) is unit-free and unchanged.

/// One whole wrapped item, in `wX` base units (9 decimals). EVM `1e18`.
pub const ONE: u64 = 1_000_000_000;
/// The SPL decimals of every `wX` mint.
pub const WX_DECIMALS: u8 = 9;
pub const LAMPORTS_PER_SOL: u64 = 1_000_000_000;
pub const BPS: u64 = 10_000;
