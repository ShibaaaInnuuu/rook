//! W5 (docs/v9/canister-only-solana.md §6, §13.1): the intent desk. Direct mint, burn and redemption with the canister, through a
//! deposit address per intent (`K_int[n]`):
//!
//! - [`intent`]: the intent record (kinds, the six states, versioned), the fee (0.0015 SOL) and the minimums (0.05 SOL / 0.01 wX).
//! - [`quote`], [`build`]: what a deposit would do, and the transactions (sweep deposit → `M[i]` and `MintTo` with the memo; burn and
//!   pay; the redemption's burn; the refunds).
//! - [`run`]: the tick: settle, one batched read of every watched address, execute at most one operation per tick on the desk line,
//!   refund on refusal, failure or expiry, expire, clean up, reconcile.
//! - [`state`]: the books, the pending-ledger and the intents in one thread-local, written to stable memory at upgrade.
//! - [`api`]: `open_intent`, `intent_status`, `quote_mint`, `quote_burn`, the operator's pauses, the logs.
//!
//! Every money-moving step goes through [`crate::chainlink`] (W4): booked and recorded before it is signed, gated by `pre_sign`.
pub mod api;
pub mod build;
pub mod intent;
pub mod quote;
pub mod run;
pub mod state;

pub use api::{is_public, OpenKind};
