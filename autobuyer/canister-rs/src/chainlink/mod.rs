//! W4 (docs/v9/canister-only-solana.md §4.4, §13.1): the links between the books and the chain, so they never diverge.
//!
//! - [`ledger`]: the pending-ledger. Every operation is booked and recorded (memo, chain effects, undo journal) BEFORE it is signed;
//!   it is final when its transaction finalizes and undone when it provably can never land (see the module header for the decision).
//! - [`gate`]: `pre_sign`, the single gate every signature passes (supply, payout, balance, collection invariants) against the chain
//!   at finalized.
//! - [`memo`]: the memo ledger, `rook:v1:<coll>:<kind>:<id>`.
//! - [`recon`]: books against chain per collection, strikes, and the stop.
//! - [`driver`]: the async side over a [`DeskIo`]: reading the chain, `pre_sign` then signing on the desk line, settling the line's
//!   outcomes, finding what a lost outcome became (the transaction by its signature, the memo scan), the reconciliation pass.
pub mod driver;
pub mod gate;
pub mod ledger;
pub mod memo;
pub mod recon;

pub use driver::DeskIo;
pub use ledger::{Effects, Incident, Ledger, Link, LinkError, OpKind, Phase, Scope, Spec, Undo};
