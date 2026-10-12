//! The buyer's only controller. An upgrade is proposed with its module, waits a public delay during which anyone can read its
//! hash, can be cancelled by the guardian, and is then executed by anyone. See `docs/v2/timelock-controller.md`.
//!
//! [`rules`] decides everything and never touches the network; [`canister`] is the thin shell that calls the management
//! canister once [`rules`] says yes.

pub mod rules;
pub mod canister;
