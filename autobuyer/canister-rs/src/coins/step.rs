//! The coins' steps on the crank line (`solhome::LineStep::Coin`): one transaction each, never queued, signed only on a free line.
use candid::CandidType;
use serde::{Deserialize, Serialize};

use crate::soltx::{b58_encode, Pubkey};

#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum CoinStep {
    /// A collection's wX mint and its Metaplex metadata (§2): one transaction, signed by `K_main` and `K_coll[i]`.
    OpenMint { coll: u32 },
    /// A collection's DBC config (§3.1): signed by `K_main` and the config's own key.
    OpenConfig { coll: u32 },
    /// `migration_damm_v2` (permissionless; the canister runs it when nobody has): a graduated curve becomes a DAMM v2 COIN/wX pool.
    Migrate { coin: Pubkey },
    /// `withdraw_migration_fee`: the partner's half of the raise (wX) straight into the coin's vault.
    MigrationFee { coin: Pubkey },
    /// `partner_withdraw_surplus`: the quote above the threshold the last buy left (wX to `K_fc`, booked as fees).
    Surplus { coin: Pubkey },
    /// `withdraw_leftover`: the curve's unsold coins to `K_fc`'s coin account.
    Leftover { coin: Pubkey },
    /// Burns the leftover coins `K_fc` holds (raises every coin's backing per unit, §3.3).
    BurnLeftover { coin: Pubkey, amount: u64 },
    /// `claim_trading_fee`: the curve's partner fees (wX) to `K_fc`.
    ClaimCurveFee { coin: Pubkey, units: u64 },
    /// `claim_position_fee` on the partner position NFT `K_fc` holds: the migrated pool's LP fees (wX) to `K_fc`.
    ClaimLpFee { coin: Pubkey, units: u64 },
    /// Fee wX of a collection (§9.1, §5.3): `to_desk` moved to `K_desk[i]` as wX, `burn` burned at the module (the public burn: a link,
    /// `chainlink`) for SOL, split between the treasury and the pool desk's share.
    CashFees { coll: u32, burn: u64, to_desk: u64, link: u64 },
    /// A coin redemption: the coins in `K_int[n]`'s account burned, their share of the vault paid in wX (a link).
    Redeem { n: u64, coin: Pubkey, amount: u64, payout: u64, link: u64 },
}

impl CoinStep {
    /// What a wait of this step is filed under (a later scan skips the target while it waits).
    pub fn key(&self) -> String {
        let c = |k: &Pubkey| b58_encode(k);
        match self {
            CoinStep::OpenMint { coll } => format!("open_mint:{coll}"),
            CoinStep::OpenConfig { coll } => format!("open_config:{coll}"),
            CoinStep::Migrate { coin } => format!("migrate:{}", c(coin)),
            CoinStep::MigrationFee { coin } => format!("migration_fee:{}", c(coin)),
            CoinStep::Surplus { coin } => format!("surplus:{}", c(coin)),
            CoinStep::Leftover { coin } => format!("leftover:{}", c(coin)),
            CoinStep::BurnLeftover { coin, .. } => format!("burn_leftover:{}", c(coin)),
            CoinStep::ClaimCurveFee { coin, .. } => format!("curve_fee:{}", c(coin)),
            CoinStep::ClaimLpFee { coin, .. } => format!("lp_fee:{}", c(coin)),
            CoinStep::CashFees { coll, .. } => format!("cash_fees:{coll}"),
            CoinStep::Redeem { n, .. } => format!("redeem:{n}"),
        }
    }

    /// The kind's name (the state's `last` log).
    pub fn kind(&self) -> &'static str {
        match self {
            CoinStep::OpenMint { .. } => "open_mint",
            CoinStep::OpenConfig { .. } => "open_config",
            CoinStep::Migrate { .. } => "migrate",
            CoinStep::MigrationFee { .. } => "migration_fee",
            CoinStep::Surplus { .. } => "surplus",
            CoinStep::Leftover { .. } => "leftover",
            CoinStep::BurnLeftover { .. } => "burn_leftover",
            CoinStep::ClaimCurveFee { .. } => "curve_fee",
            CoinStep::ClaimLpFee { .. } => "lp_fee",
            CoinStep::CashFees { .. } => "cash_fees",
            CoinStep::Redeem { .. } => "redeem",
        }
    }

    /// The link a step carries, if it changes books (`chainlink`).
    pub fn link(&self) -> Option<u64> {
        match self {
            CoinStep::CashFees { link, .. } | CoinStep::Redeem { link, .. } => Some(*link),
            _ => None,
        }
    }

    pub fn label(&self) -> String {
        let c = |k: &Pubkey| b58_encode(k);
        match self {
            CoinStep::OpenMint { coll } => format!("open collection {coll}: wX mint and metadata"),
            CoinStep::OpenConfig { coll } => format!("open collection {coll}: DBC create_config"),
            CoinStep::Migrate { coin } => format!("DBC migration_damm_v2 (coin {})", c(coin)),
            CoinStep::MigrationFee { coin } => format!("DBC withdraw_migration_fee into the vault (coin {})", c(coin)),
            CoinStep::Surplus { coin } => format!("DBC partner_withdraw_surplus (coin {})", c(coin)),
            CoinStep::Leftover { coin } => format!("DBC withdraw_leftover (coin {})", c(coin)),
            CoinStep::BurnLeftover { coin, amount } => format!("burn {amount} leftover coins (coin {})", c(coin)),
            CoinStep::ClaimCurveFee { coin, units } => format!("DBC claim_trading_fee (≥ {units} wX, coin {})", c(coin)),
            CoinStep::ClaimLpFee { coin, units } => format!("DAMM v2 claim_position_fee (≥ {units} wX, coin {})", c(coin)),
            CoinStep::CashFees { coll, burn, to_desk, link } => format!("fee wX of collection {coll}: {burn} burned at the module, {to_desk} to the pool desk (link {link})"),
            CoinStep::Redeem { n, coin, amount, payout, link } => format!("redeem {amount} coins of {} for {payout} wX (intent {n}, link {link})", c(coin)),
        }
    }
}

/// A step ready to sign: its instructions and keys, and the link it carries (a step that changes books).
pub struct Planned {
    pub step: CoinStep,
    pub ixs: Vec<crate::soltx::Instruction>,
    pub keys: Vec<crate::solkey::KeyPath>,
    pub link: Option<u64>,
}
