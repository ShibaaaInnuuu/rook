//! W6 (docs/v9/canister-only-solana.md §2, §3, §7, §9.1, §16, §13.1): the coins. What the canister does around a collection's wX and its
//! DBC coins, all as steps on the crank line (`solhome::LineStep::Coin`), none of it with a Solana program of Rook's own:
//!
//! - [`open`]: a collection's wX mint with its Metaplex metadata, its DBC config with the pinned parameters, the config's read-back
//!   (`solix2::guard::{check_wx_mint, check_config}`: nothing is treated as Rook's until it matches the pin), and its listing in the books;
//!   in the unlock order ([`unlock`]) as the float that the fees fill allows;
//! - [`grad`]: discovery of every Rook coin (`getProgramAccounts` on DBC by config), their graduation (the migration if nobody ran it, the
//!   partner's migration fee into the coin's vault, the leftovers withdrawn and burned) and the vault book;
//! - [`fees`]: the curve's and the pools' fees claimed to `K_fc`, split between the pool desk and the treasury and cashed at the module
//!   (a `chainlink` link: booked first, `pre_sign`, memo);
//! - [`redeem`]: coin redemption, `⌊V · a / S⌋` wX from the vault for `a` coins burned;
//! - [`run`]: the tick, [`link`]: the signing gate, [`state`]: the persisted state, [`build`]: the transactions, [`step`]: the step type.
//!
//! The IC glue is [`api`].
pub mod api;
pub mod build;
pub mod fees;
pub mod grad;
pub mod link;
pub mod open;
pub mod redeem;
pub mod run;
pub mod state;
pub mod step;
pub mod unlock;

pub use step::CoinStep;
