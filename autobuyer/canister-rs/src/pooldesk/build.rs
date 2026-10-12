//! The pool desk's transactions (§5, S4 §5.2): pure builders over `solix2::dlmm`, `solix2::spl` and `soltx`. `K_main` pays every fee and
//! every rent (S4 §5.1 item 4: DLMM's payers are plain payers); `K_desk[i]` owns the pair's position, its token accounts and its SOL;
//! `K_coll[i]` signs where the module mints. A step that changes books ends with the link's Memo.
use crate::solix2::dlmm::{self, Lb, RebalanceAdd, Shape};
use crate::solix2::spl;
use crate::solkey::KeyPath;
use crate::soltx::{associated_token_address, compute_unit_limit, transfer, Instruction, Pubkey};

/// The compute budget of a step that touches bins (S4: a re-centre runs 280–340 k CU; an add or a swap less).
pub const BINS_CU: u32 = 500_000;

/// The addresses of one collection's desk.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Addrs {
    /// `K_main`: fee and rent payer.
    pub main: Pubkey,
    /// `K_desk[i]`.
    pub desk: Pubkey,
    /// `K_coll[i]`: the module address `M[i]` and the wX mint authority.
    pub module: Pubkey,
    /// `K_tre`.
    pub treasury: Pubkey,
    /// The pair: `x` = the wX mint, `y` = wSOL.
    pub lb: Lb,
}

impl Addrs {
    /// The desk's wX account.
    pub fn desk_x(&self) -> Pubkey {
        associated_token_address(&self.desk, &self.lb.x)
    }
    /// The desk's wSOL account.
    pub fn desk_y(&self) -> Pubkey {
        associated_token_address(&self.desk, &self.lb.y)
    }
    /// The desk's position for `[lower, lower + width)`.
    pub fn position(&self, lower: i32, width: i32) -> Pubkey {
        dlmm::position_address(&self.lb.pair, &self.desk, lower, width)
    }
}

/// The pair at `active`, and the desk's two token accounts (created by `K_main`, so they exist before any wX does).
pub fn open_pair_tx(a: &Addrs, active: i32) -> Vec<Instruction> {
    vec![
        spl::create_ata_idempotent(a.main, a.desk, a.lb.x),
        spl::create_ata_idempotent(a.main, a.desk, a.lb.y),
        dlmm::initialize_lb_pair2(a.main, dlmm::preset_address(dlmm::DESK_PRESET), a.lb.x, a.lb.y, active),
    ]
}

pub fn open_arrays_tx(a: &Addrs, indexes: &[i64]) -> Vec<Instruction> {
    indexes.iter().map(|i| dlmm::initialize_bin_array(a.lb.pair, *i, a.main)).collect()
}

pub fn open_position_tx(a: &Addrs, lower: i32, width: i32) -> Vec<Instruction> {
    vec![dlmm::initialize_position_pda(a.main, a.desk, a.lb.pair, a.desk, lower, width)]
}

/// The desk's public mint: `lamports` from `K_desk[i]` into `M[i]`, `units` wX minted to the desk's wX account, the link's memo.
pub fn mint_tx(a: &Addrs, units: u64, lamports: u64, memo: &str) -> Vec<Instruction> {
    vec![spl::create_ata_idempotent(a.main, a.desk, a.lb.x), transfer(a.desk, a.module, lamports), spl::mint_to(a.lb.x, a.desk_x(), a.module, units), spl::memo(memo, &[])]
}

/// `wrap` lamports wrapped into the desk's wSOL account (what `ay` needs beyond the wSOL it holds), then the add of `ax` wX and `ay`
/// lamports over `[lo, hi]` around `active` in a spot shape.
#[allow(clippy::too_many_arguments)]
pub fn add_tx(a: &Addrs, position: Pubkey, ax: u64, ay: u64, wrap: u64, active: i32, lo: i32, hi: i32) -> Vec<Instruction> {
    let mut v = vec![compute_unit_limit(BINS_CU), spl::create_ata_idempotent(a.main, a.desk, a.lb.y)];
    if wrap > 0 {
        v.push(transfer(a.desk, a.desk_y(), wrap));
        v.push(spl::sync_native(a.desk_y()));
    }
    v.push(dlmm::add_liquidity_by_strategy2(&a.lb, position, a.desk, a.desk_x(), a.desk_y(), ax, ay, active, lo, hi, Shape::Spot));
    v
}

/// The up-arb (§5.4, one transaction): the mint leg ([`mint_tx`] without its memo), then the sale of `units` wX for at least `min_out`
/// lamports (X in, Y out: the swap walks the arrays downward from `active`), then the memo.
pub fn arb_up_tx(a: &Addrs, units: u64, lamports: u64, min_out: u64, active: i32, arrays: usize, memo: &str) -> Vec<Instruction> {
    vec![
        compute_unit_limit(BINS_CU),
        spl::create_ata_idempotent(a.main, a.desk, a.lb.x),
        spl::create_ata_idempotent(a.main, a.desk, a.lb.y),
        transfer(a.desk, a.module, lamports),
        spl::mint_to(a.lb.x, a.desk_x(), a.module, units),
        dlmm::swap2(&a.lb, a.desk, a.desk_x(), a.desk_y(), units, min_out, &dlmm::swap_arrays(&a.lb.pair, active, false, arrays)),
        spl::memo(memo, &[]),
    ]
}

/// The down-arb's buy: `lamports` of the desk's wSOL for at least `min_out` wX (Y in, X out: the arrays upward from `active`).
pub fn arb_down_tx(a: &Addrs, lamports: u64, min_out: u64, active: i32, arrays: usize) -> Vec<Instruction> {
    vec![
        compute_unit_limit(BINS_CU),
        spl::create_ata_idempotent(a.main, a.desk, a.lb.x),
        dlmm::swap2(&a.lb, a.desk, a.desk_y(), a.desk_x(), lamports, min_out, &dlmm::swap_arrays(&a.lb.pair, active, true, arrays)),
    ]
}

/// The public burn of `units` of the desk's wX: burned from its account, `paid` lamports from `M[i]` to `K_desk[i]`, the memo.
pub fn burn_tx(a: &Addrs, units: u64, paid: u64, memo: &str) -> Vec<Instruction> {
    vec![spl::burn(a.desk_x(), a.lb.x, a.desk, units), transfer(a.module, a.desk, paid), spl::memo(memo, &[])]
}

/// S4 R1: the whole of `remove` withdrawn (fees claimed) and `adds` put back around `active` over `[new_lo, new_hi]`, in one
/// `rebalance_liquidity` (the position grows or shrinks to the new range; `K_main` pays the growth).
#[allow(clippy::too_many_arguments)]
pub fn recenter_tx(a: &Addrs, position: Pubkey, active: i32, remove: Option<(i32, i32)>, adds: &[RebalanceAdd], new_lo: i32, new_hi: i32) -> Vec<Instruction> {
    let (lo, hi) = remove.unwrap_or((new_lo, new_hi));
    let arrays = dlmm::arrays_for(&a.lb.pair, lo.min(new_lo), hi.max(new_hi));
    vec![
        compute_unit_limit(BINS_CU),
        dlmm::rebalance_liquidity(&a.lb, position, a.desk, a.main, a.desk_x(), a.desk_y(), active, true, remove, adds, &arrays),
    ]
}

/// S4 R2, first transaction: the whole position withdrawn (into the desk's wSOL account, re-created first: audit B3-03).
pub fn withdraw_tx(a: &Addrs, position: Pubkey, lo: i32, hi: i32) -> Vec<Instruction> {
    vec![
        compute_unit_limit(BINS_CU),
        spl::create_ata_idempotent(a.main, a.desk, a.lb.y),
        dlmm::remove_liquidity_by_range2(&a.lb, position, a.desk, a.desk_x(), a.desk_y(), lo, hi, 10_000),
    ]
}

/// S4 R2, second transaction: the empty pool's active bin moved to `target` (`from` and `to` are the bin arrays that exist).
pub fn move_tx(a: &Addrs, from: Option<Pubkey>, to: Option<Pubkey>, target: i32) -> Vec<Instruction> {
    vec![dlmm::go_to_a_bin(a.lb.pair, from, to, target)]
}

/// The fees to the desk's wSOL account (re-created first: an [`unwrap_tx`] may have closed it, audit B3-03).
pub fn claim_tx(a: &Addrs, position: Pubkey, lo: i32, hi: i32) -> Vec<Instruction> {
    vec![spl::create_ata_idempotent(a.main, a.desk, a.lb.y), dlmm::claim_fee2(&a.lb, position, a.desk, a.desk_x(), a.desk_y(), lo, hi)]
}

/// `lamports` of the desk's native SOL to `K_tre` (review F3: the treasury pays native; its income arrives native).
pub fn sweep_tx(a: &Addrs, lamports: u64) -> Vec<Instruction> {
    vec![transfer(a.desk, a.treasury, lamports)]
}

/// Audit B3-03: everything the pool pays the desk arrives as wSOL (fees, an up-arb's proceeds, a withdrawal's SOL side) and everything
/// the desk pays is native (the mint legs, the review-F3 sweep). `CloseAccount` of a native account pays ALL its lamports (the wrapped
/// SOL and the rent) to `K_desk[i]`, natively; the desk signs as the owner. Every builder that pays into or spends the wSOL account
/// re-creates it (`create_ata_idempotent`, `K_main` paying) or holds none to spend.
pub fn unwrap_tx(a: &Addrs) -> Vec<Instruction> {
    vec![spl::close_account(a.desk_y(), a.desk, a.desk)]
}

/// The keys beside `K_main`: the desk's for anything it owns, the collection's where the module mints or pays.
pub fn desk_keys(coll: u32) -> Vec<KeyPath> {
    vec![KeyPath::Desk(coll)]
}
pub fn module_keys(coll: u32) -> Vec<KeyPath> {
    vec![KeyPath::Desk(coll), KeyPath::Coll(coll)]
}
/// A step `K_main` alone pays for (the pair, the arrays, the position: the desk is a PDA base, not a signer, there).
pub fn main_only() -> Vec<KeyPath> {
    vec![]
}
