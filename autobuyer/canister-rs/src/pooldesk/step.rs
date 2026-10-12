//! The pool desk's steps on the pool line (`solhome::LineStep::Pool`): one transaction each, never queued, signed only on a free line.
use candid::CandidType;
use serde::{Deserialize, Serialize};

#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum PoolStep {
    /// `initialize_lb_pair2` on the desk's preset at the floor's bin, with the desk's wX and wSOL token accounts (§5.1, S4 §2.2).
    OpenPair { coll: u32, active: i32, pair: crate::soltx::Pubkey },
    /// `initialize_bin_array` for the arrays the position's range needs (one or two).
    OpenArrays { coll: u32, indexes: Vec<i64> },
    /// `initialize_position_pda(lower, width)`: the desk's one position, base = owner = `K_desk[i]`.
    OpenPosition { coll: u32, lower: i32, width: i32, position: crate::soltx::Pubkey },
    /// The public mint for the desk's own wX (§5.2: the desk is an ordinary holder): `lamports` into `M[i]`, `units` to `K_desk[i]`
    /// (a `chainlink` link).
    Mint { coll: u32, units: u64, lamports: u64, link: u64 },
    /// `add_liquidity_by_strategy2`: `ax` wX and `ay` lamports (wrapped first) in a spot shape over `[lo, hi]`.
    Add { coll: u32, ax: u64, ay: u64, lo: i32, hi: i32 },
    /// §5.4 up-arb: mint `units` at the module for `lamports` and sell them into the pool for at least `min_out` (one transaction: mint
    /// and swap, so the desk never holds an unsold leg; a link).
    ArbUp { coll: u32, units: u64, lamports: u64, min_out: u64, link: u64, profit: u64 },
    /// §5.4 down-arb: buy wX with `lamports` of the desk's wSOL (at least `min_out`); the burn follows as [`PoolStep::Burn`].
    ArbDown { coll: u32, lamports: u64, min_out: u64, profit: u64 },
    /// The public burn of `units` of the desk's wX inventory: `paid` lamports from `M[i]` to `K_desk[i]` (a link).
    Burn { coll: u32, units: u64, paid: u64, link: u64 },
    /// `rebalance_liquidity` in place: the whole position withdrawn and re-added as a spot shape around `active` (S4 R1).
    Recenter { coll: u32, active: i32, lo: i32, hi: i32 },
    /// `remove_liquidity_by_range2` of the whole position (S4 R2, first transaction: the floor left the band).
    Withdraw { coll: u32, lo: i32, hi: i32 },
    /// `go_to_a_bin`: the empty pool's price to the floor's bin (S4 R2, second transaction); the add follows.
    Move { coll: u32, target: i32 },
    /// `claim_fee2`: the position's SOL fees to the desk.
    Claim { coll: u32, lo: i32, hi: i32 },
    /// The desk's SOL above its target to the treasury (§5.3): `lamports` of wSOL to `K_tre`'s wSOL account.
    Sweep { coll: u32, lamports: u64 },
    /// Audit B3-03: `close_account` of the desk's wSOL account to `K_desk[i]`: its `lamports` of wSOL (and the account's rent) come back
    /// native, for the treasury sweep and the mint legs; the next step that needs the account re-creates it.
    Unwrap { coll: u32, lamports: u64 },
}

impl PoolStep {
    pub fn coll(&self) -> u32 {
        match self {
            PoolStep::OpenPair { coll, .. } | PoolStep::OpenArrays { coll, .. } | PoolStep::OpenPosition { coll, .. } | PoolStep::Mint { coll, .. }
            | PoolStep::Add { coll, .. } | PoolStep::ArbUp { coll, .. } | PoolStep::ArbDown { coll, .. } | PoolStep::Burn { coll, .. }
            | PoolStep::Recenter { coll, .. } | PoolStep::Withdraw { coll, .. } | PoolStep::Move { coll, .. } | PoolStep::Claim { coll, .. }
            | PoolStep::Sweep { coll, .. } | PoolStep::Unwrap { coll, .. } => *coll,
        }
    }

    /// The kind's name (the waits' key, the state's `last` log).
    pub fn kind(&self) -> &'static str {
        match self {
            PoolStep::OpenPair { .. } => "open_pair",
            PoolStep::OpenArrays { .. } => "open_arrays",
            PoolStep::OpenPosition { .. } => "open_position",
            PoolStep::Mint { .. } => "mint",
            PoolStep::Add { .. } => "add",
            PoolStep::ArbUp { .. } => "arb_up",
            PoolStep::ArbDown { .. } => "arb_down",
            PoolStep::Burn { .. } => "burn",
            PoolStep::Recenter { .. } => "recenter",
            PoolStep::Withdraw { .. } => "withdraw",
            PoolStep::Move { .. } => "move",
            PoolStep::Claim { .. } => "claim",
            PoolStep::Sweep { .. } => "sweep",
            PoolStep::Unwrap { .. } => "unwrap",
        }
    }

    /// What a wait of this step is filed under: one per kind and collection.
    pub fn key(&self) -> String {
        format!("{}:{}", self.kind(), self.coll())
    }

    /// The link a step carries, if it changes books (`chainlink`).
    pub fn link(&self) -> Option<u64> {
        match self {
            PoolStep::Mint { link, .. } | PoolStep::ArbUp { link, .. } | PoolStep::Burn { link, .. } => Some(*link),
            _ => None,
        }
    }

    pub fn label(&self) -> String {
        match self {
            PoolStep::OpenPair { coll, active, .. } => format!("pool {coll}: open the pair at bin {active}"),
            PoolStep::OpenArrays { coll, indexes } => format!("pool {coll}: bin arrays {indexes:?}"),
            PoolStep::OpenPosition { coll, lower, width, .. } => format!("pool {coll}: the position [{lower}, {}]", lower + width - 1),
            PoolStep::Mint { coll, units, lamports, link } => format!("pool {coll}: mint {units} wX for {lamports} lamports (link {link})"),
            PoolStep::Add { coll, ax, ay, lo, hi } => format!("pool {coll}: add {ax} wX and {ay} lamports over [{lo}, {hi}]"),
            PoolStep::ArbUp { coll, units, lamports, min_out, link, .. } => {
                format!("pool {coll}: up-arb, mint {units} wX for {lamports} and sell for ≥ {min_out} (link {link})")
            }
            PoolStep::ArbDown { coll, lamports, min_out, .. } => format!("pool {coll}: down-arb, buy ≥ {min_out} wX for {lamports} lamports"),
            PoolStep::Burn { coll, units, paid, link } => format!("pool {coll}: burn {units} wX at the module for {paid} lamports (link {link})"),
            PoolStep::Recenter { coll, active, lo, hi } => format!("pool {coll}: re-centre [{lo}, {hi}] around bin {active}"),
            PoolStep::Withdraw { coll, lo, hi } => format!("pool {coll}: withdraw [{lo}, {hi}]"),
            PoolStep::Move { coll, target } => format!("pool {coll}: move the empty pool to bin {target}"),
            PoolStep::Claim { coll, lo, hi } => format!("pool {coll}: claim the fees of [{lo}, {hi}]"),
            PoolStep::Sweep { coll, lamports } => format!("pool {coll}: {lamports} lamports to the treasury"),
            PoolStep::Unwrap { coll, lamports } => format!("pool {coll}: unwrap {lamports} lamports of wSOL"),
        }
    }
}

/// A step ready to sign: its instructions, the keys beside `K_main`, and the link it carries.
pub struct Planned {
    pub step: PoolStep,
    pub ixs: Vec<crate::soltx::Instruction>,
    pub keys: Vec<crate::solkey::KeyPath>,
    pub link: Option<u64>,
}
