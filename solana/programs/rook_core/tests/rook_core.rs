//! LiteSVM tests of rook_core (they load target/deploy/rook_core.so: run `anchor build` first).

#[path = "suite/common.rs"]
mod common;
#[path = "suite/floor.rs"]
mod floor;

#[path = "suite/convert.rs"]
mod convert;
#[path = "suite/wrap.rs"]
mod wrap;
#[path = "suite/registry.rs"]
mod registry;
#[path = "suite/treasury_wake.rs"]
mod treasury_wake;
#[path = "suite/flow.rs"]
mod flow;
#[path = "suite/props.rs"]
mod props;
#[path = "suite/raffle.rs"]
mod raffle;
#[path = "suite/solnft.rs"]
mod solnft;
