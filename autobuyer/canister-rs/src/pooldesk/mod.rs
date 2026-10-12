//! W7 (docs/v9/canister-only-solana.md §5, §17; spike S4): the pool desk. One Meteora DLMM wX/SOL pair per collection, one position owned
//! by `K_desk[i]`, grown from the collection's fees (`coins::fees`), kept near the floor by the arb loop and re-centred in place.
//!
//! - [`plan`]: the decisions (pure): open, re-centre, arb, the target depth, the treasury's excess.
//! - [`build`]: the transactions (pure).
//! - [`run`]: the tick: one read per collection, one step signed on the pool line; [`step`]: the step type.
//! - [`state`]: what the desk knows per collection, versioned, carried across an upgrade.
//! - [`api`]: the IC glue and the views.
pub mod api;
pub mod build;
pub mod plan;
pub mod run;
pub mod state;
pub mod step;

pub use state::{note_fee_income, wants_fees};
pub use step::PoolStep;
