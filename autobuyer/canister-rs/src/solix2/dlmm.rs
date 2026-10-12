//! Meteora DLMM (`lb_clmm` 0.12.0, `LBUZ…`), by hand from the IDL, for the wX/SOL pool desk (docs/v9/canister-only-solana.md §17,
//! docs/v9/spikes/s4-wx-pools.md §5.2). Spike S4 chose a DLMM pair, preset 262 (bin step 50, base fee 0.4 %, fees in Y = SOL only),
//! one position owned by `K_desk[i]` (a PDA: no position keypair), about 10 SOL a side in a ±10 % band, re-centred in place by
//! `rebalance_liquidity`.
//!
//! Instruction data is the Anchor discriminator plus Borsh; accounts are in the IDL's order with its flags; an absent optional account
//! is the program id (Anchor's convention); `RemainingAccountsInfo { slices: [] }` (four zero bytes) is enough for classic SPL. The
//! builders were checked byte for byte against instructions that LANDED in LiteSVM on the mainnet program bytes
//! (`tests/fixtures/solix2/dlmm.json`, written by `solana/tests/dlmm_fixtures.rs`, checked by `tests/solix2_dlmm.rs`).
//!
//! Also here: the PDAs, DLMM's own bin-price arithmetic (Q64.64), the decoders of `LbPair`, `PositionV2` and `BinArray` (offsets
//! checked against accounts the program wrote), what a position holds and what it can claim (exact, per bin, against what the chain
//! paid), and a walk over the bins that quotes a swap (the arb loop's closed form: spike S4 §5.1 item 3).
use std::collections::BTreeMap;

use super::{event_authority, id, ix, ro, rw, signer, MEMO_PROGRAM};
use crate::soltx::{find_program_address, Borsh, Instruction, Pubkey, SYSTEM_PROGRAM};

pub const DLMM_PROGRAM: &str = "LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo";
pub const RENT_SYSVAR: &str = "SysvarRent111111111111111111111111111111111";
/// The preset the desk's pairs use: index 262, bin step 50, base fee 0.4 %, `collect_fee_mode` 1 (fees in Y only) [M, spikes S4 §2.1].
pub const DESK_PRESET: u16 = 262;
pub const DESK_BIN_STEP: u16 = 50;
/// A bin array holds 70 bins; a position covers at most 70 bins at creation and grows by at most 91 per `increase_position_length2`.
pub const BINS_PER_ARRAY: i32 = 70;
pub const MAX_RESIZE_LENGTH: i32 = 91;
/// DLMM's fee precision (1e9) and cap (10 %).
pub const FEE_PRECISION: u128 = 1_000_000_000;
pub const MAX_FEE_RATE: u128 = 100_000_000;
/// `bin_id` bounds the program accepts for a pair (`i32` in `StaticParameters`, ±443,636 at bin step 1; the preset's own limits are tighter).
pub const MAX_BIN_ID: i32 = 443_636;

pub fn program() -> Pubkey {
    id(DLMM_PROGRAM)
}

const fn hex8(s: &str) -> [u8; 8] {
    let b = s.as_bytes();
    let mut o = [0u8; 8];
    let mut i = 0;
    while i < 8 {
        o[i] = nib(b[2 * i]) * 16 + nib(b[2 * i + 1]);
        i += 1;
    }
    o
}
const fn nib(c: u8) -> u8 {
    if c >= b'a' {
        c - b'a' + 10
    } else {
        c - b'0'
    }
}

/// The IDL's instruction discriminators, pinned (`sha256("global:<name>")[..8]`; the tests check each one against its name).
pub mod disc {
    use super::hex8;
    pub const INITIALIZE_LB_PAIR2: [u8; 8] = hex8("493b2478ed536cc6");
    pub const INITIALIZE_BIN_ARRAY: [u8; 8] = hex8("235613b94ed44bd3");
    pub const INITIALIZE_POSITION_PDA: [u8; 8] = hex8("2e527d92558de499");
    pub const INCREASE_POSITION_LENGTH2: [u8; 8] = hex8("ffd2cc477389e171");
    pub const ADD_LIQUIDITY_BY_STRATEGY2: [u8; 8] = hex8("03dd95da6f8d76d5");
    pub const REBALANCE_LIQUIDITY: [u8; 8] = hex8("5c04b0c177b95309");
    pub const REMOVE_LIQUIDITY_BY_RANGE2: [u8; 8] = hex8("cc02c391359191cd");
    pub const GO_TO_A_BIN: [u8; 8] = hex8("9248aee028fd54ae");
    pub const SWAP2: [u8; 8] = hex8("414b3f4ceb5b5b88");
    pub const CLAIM_FEE2: [u8; 8] = hex8("70bf65ab1c907fbb");
    pub const CLOSE_POSITION2: [u8; 8] = hex8("ae5a2373ba2893e2");
}

/// A `LiquidityParameterByStrategy` strategy type (the IDL's enum order). The desk uses `SpotImBalanced`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    Spot = 6,
    Curve = 7,
    BidAsk = 8,
}

fn data(d: [u8; 8]) -> Borsh {
    Borsh(d.to_vec())
}
fn none() -> crate::soltx::AccountMeta {
    ro(program())
}
fn ev() -> crate::soltx::AccountMeta {
    ro(event_authority(&program()))
}
/// `RemainingAccountsInfo { slices: [] }`.
fn no_slices(b: Borsh) -> Borsh {
    b.u32(0)
}
fn zeros(b: Borsh, n: usize) -> Borsh {
    b.bytes(&vec![0u8; n])
}
fn i32le(b: Borsh, v: i32) -> Borsh {
    b.bytes(&v.to_le_bytes())
}
fn pda(seeds: &[&[u8]]) -> Pubkey {
    find_program_address(seeds, &program()).expect("a program address exists").0
}
fn sorted(a: &Pubkey, b: &Pubkey) -> (Pubkey, Pubkey) {
    if a < b {
        (*a, *b)
    } else {
        (*b, *a)
    }
}

// ---- addresses -------------------------------------------------------------------------------------------------------------------

/// `["preset_parameter2", index u16 LE]`.
pub fn preset_address(index: u16) -> Pubkey {
    pda(&[b"preset_parameter2", &index.to_le_bytes()])
}
/// A pair on a preset: `[preset, min(x, y), max(x, y)]`, one per (preset, mint pair).
pub fn pair_address(preset: &Pubkey, x: &Pubkey, y: &Pubkey) -> Pubkey {
    let (lo, hi) = sorted(x, y);
    pda(&[preset, &lo, &hi])
}
pub fn reserve_address(pair: &Pubkey, mint: &Pubkey) -> Pubkey {
    pda(&[pair, mint])
}
pub fn oracle_address(pair: &Pubkey) -> Pubkey {
    pda(&[b"oracle", pair])
}
/// `["bin_array", pair, index i64 LE]`; the index of bin `b` is `⌊b / 70⌋` (Euclidean, negative bins included).
pub fn bin_array_address(pair: &Pubkey, index: i64) -> Pubkey {
    pda(&[b"bin_array", pair, &index.to_le_bytes()])
}
pub fn array_index(bin: i32) -> i64 {
    (bin as i64).div_euclid(BINS_PER_ARRAY as i64)
}
/// The bin arrays covering bins `lo..=hi`, in ascending order.
pub fn arrays_for(pair: &Pubkey, lo: i32, hi: i32) -> Vec<Pubkey> {
    (array_index(lo)..=array_index(hi)).map(|i| bin_array_address(pair, i)).collect()
}
/// `["position", pair, base, lower i32 LE, width i32 LE]`: the desk's position, base = owner = `K_desk[i]`.
pub fn position_address(pair: &Pubkey, base: &Pubkey, lower: i32, width: i32) -> Pubkey {
    pda(&[b"position", pair, base, &lower.to_le_bytes(), &width.to_le_bytes()])
}
/// The bin arrays a swap walks, in the swap's direction starting at the active bin's array (DLMM refuses any other order: spikes
/// S4 §2.4): `count` of them, ascending when `up` (a buy: Y in, X out), descending otherwise.
pub fn swap_arrays(pair: &Pubkey, active_id: i32, up: bool, count: usize) -> Vec<Pubkey> {
    let a = array_index(active_id);
    (0..count as i64).map(|k| bin_array_address(pair, if up { a + k } else { a - k })).collect()
}

/// A pair the builders act on: `x` = wX, `y` = wSOL (so the price is SOL per wX and fees arrive in Y = SOL), the bin step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Lb {
    pub pair: Pubkey,
    pub x: Pubkey,
    pub y: Pubkey,
    pub bin_step: u16,
}

impl Lb {
    pub fn new(pair: Pubkey, x: Pubkey, y: Pubkey, bin_step: u16) -> Lb {
        Lb { pair, x, y, bin_step }
    }
    /// The desk's pair on [`DESK_PRESET`] for wX `x` against wSOL.
    pub fn desk(x: Pubkey) -> Lb {
        let y = super::native_mint();
        Lb { pair: pair_address(&preset_address(DESK_PRESET), &x, &y), x, y, bin_step: DESK_BIN_STEP }
    }
    pub fn rx(&self) -> Pubkey {
        reserve_address(&self.pair, &self.x)
    }
    pub fn ry(&self) -> Pubkey {
        reserve_address(&self.pair, &self.y)
    }
    pub fn oracle(&self) -> Pubkey {
        oracle_address(&self.pair)
    }
}

// ---- instructions ------------------------------------------------------------------------------------------------------------------

/// `initialize_lb_pair2` on a preset: anyone can create it, `funder` only pays (0.034647 SOL: pair, two reserves, oracle). Opens at
/// `active_id`.
pub fn initialize_lb_pair2(funder: Pubkey, preset: Pubkey, x: Pubkey, y: Pubkey, active_id: i32) -> Instruction {
    let pair = pair_address(&preset, &x, &y);
    let tp = super::token_program();
    ix(program(), vec![rw(pair), none(), ro(x), ro(y), rw(reserve_address(&pair, &x)), rw(reserve_address(&pair, &y)), rw(oracle_address(&pair)),
        ro(preset), signer(funder, true), none(), none(), ro(tp), ro(tp), ro(SYSTEM_PROGRAM), ev(), ro(program())],
        zeros(i32le(data(disc::INITIALIZE_LB_PAIR2), active_id), 96).done())
}

/// `initialize_bin_array(index)`: 0.071437 SOL, NOT refundable (`close_bin_array` is admin-only); anyone can pay.
pub fn initialize_bin_array(pair: Pubkey, index: i64, funder: Pubkey) -> Instruction {
    ix(program(), vec![ro(pair), rw(bin_array_address(&pair, index)), signer(funder, true), ro(SYSTEM_PROGRAM)],
        data(disc::INITIALIZE_BIN_ARRAY).i64(index).done())
}

/// `initialize_position_pda(lower, width ≤ 70)`: base = owner = `K_desk`, `payer` = `K_main` (0.057406 SOL, refunded on close).
pub fn initialize_position_pda(payer: Pubkey, base: Pubkey, pair: Pubkey, owner: Pubkey, lower: i32, width: i32) -> Instruction {
    ix(program(), vec![signer(payer, true), signer(base, false), rw(position_address(&pair, &base, lower, width)), ro(pair), signer(owner, false),
        ro(SYSTEM_PROGRAM), ro(id(RENT_SYSVAR)), ev(), ro(program())], i32le(i32le(data(disc::INITIALIZE_POSITION_PDA), lower), width).done())
}

/// `increase_position_length2(minimum_upper_bin_id)`: only for ranges over 70 bins (≤ 91 more bins per call).
pub fn increase_position_length2(funder: Pubkey, pair: Pubkey, position: Pubkey, owner: Pubkey, min_upper: i32) -> Instruction {
    ix(program(), vec![signer(funder, true), ro(pair), rw(position), signer(owner, false), ro(SYSTEM_PROGRAM), ev(), ro(program())],
        i32le(data(disc::INCREASE_POSITION_LENGTH2), min_upper).done())
}

/// `add_liquidity_by_strategy2`: `ax` of X and `ay` of Y at most, over `[lo, hi]` around `active_id` (slippage 3 bins), with a shape.
/// The bin arrays covering the range follow as remaining accounts.
#[allow(clippy::too_many_arguments)]
pub fn add_liquidity_by_strategy2(lb: &Lb, position: Pubkey, sender: Pubkey, user_x: Pubkey, user_y: Pubkey, ax: u64, ay: u64, active_id: i32, lo: i32, hi: i32,
    shape: Shape) -> Instruction {
    let tp = super::token_program();
    let mut accounts = vec![rw(position), rw(lb.pair), none(), rw(user_x), rw(user_y), rw(lb.rx()), rw(lb.ry()), ro(lb.x), ro(lb.y), signer(sender, false),
        ro(tp), ro(tp), ev(), ro(program())];
    accounts.extend(arrays_for(&lb.pair, lo, hi).into_iter().map(rw));
    let d = i32le(i32le(i32le(i32le(data(disc::ADD_LIQUIDITY_BY_STRATEGY2).u64(ax).u64(ay), active_id), 3), lo), hi).u8(shape as u8);
    ix(program(), accounts, no_slices(zeros(d, 64)).done())
}

/// `swap2 { amount_in, min_amount_out }`: direction follows `input` / `output` (a buy: Y in → X out). `arrays` are the bin arrays
/// in the swap's order ([`swap_arrays`]); the user signs.
pub fn swap2(lb: &Lb, user: Pubkey, input: Pubkey, output: Pubkey, amount_in: u64, min_out: u64, arrays: &[Pubkey]) -> Instruction {
    let tp = super::token_program();
    let mut accounts = vec![rw(lb.pair), none(), rw(lb.rx()), rw(lb.ry()), rw(input), rw(output), ro(lb.x), ro(lb.y), rw(lb.oracle()), none(), signer(user, false),
        ro(tp), ro(tp), ro(id(MEMO_PROGRAM)), ev(), ro(program())];
    accounts.extend(arrays.iter().copied().map(rw));
    ix(program(), accounts, no_slices(data(disc::SWAP2).u64(amount_in).u64(min_out)).done())
}

/// `claim_fee2(min_bin, max_bin)`: the position's fees (SOL only on the desk's preset); a stranger is refused.
pub fn claim_fee2(lb: &Lb, position: Pubkey, sender: Pubkey, user_x: Pubkey, user_y: Pubkey, lo: i32, hi: i32) -> Instruction {
    let tp = super::token_program();
    let mut accounts = vec![rw(lb.pair), rw(position), signer(sender, false), rw(lb.rx()), rw(lb.ry()), rw(user_x), rw(user_y), ro(lb.x), ro(lb.y), ro(tp), ro(tp),
        ro(id(MEMO_PROGRAM)), ev(), ro(program())];
    accounts.extend(arrays_for(&lb.pair, lo, hi).into_iter().map(rw));
    ix(program(), accounts, no_slices(i32le(i32le(data(disc::CLAIM_FEE2), lo), hi)).done())
}

/// `remove_liquidity_by_range2(from, to, bps)`: `bps` of the position's liquidity in `[lo, hi]` (10,000 = all). The range is
/// explicit: `None` bounds would mean "the active bin" (spikes S4 §2.4).
#[allow(clippy::too_many_arguments)]
pub fn remove_liquidity_by_range2(lb: &Lb, position: Pubkey, sender: Pubkey, user_x: Pubkey, user_y: Pubkey, lo: i32, hi: i32, bps: u16) -> Instruction {
    let tp = super::token_program();
    let mut accounts = vec![rw(position), rw(lb.pair), none(), rw(user_x), rw(user_y), rw(lb.rx()), rw(lb.ry()), ro(lb.x), ro(lb.y), signer(sender, false),
        ro(tp), ro(tp), ro(id(MEMO_PROGRAM)), ev(), ro(program())];
    accounts.extend(arrays_for(&lb.pair, lo, hi).into_iter().map(rw));
    ix(program(), accounts, no_slices(data(disc::REMOVE_LIQUIDITY_BY_RANGE2).bytes(&lo.to_le_bytes()).bytes(&hi.to_le_bytes()).u16(bps)).done())
}

/// `close_position2`: an EMPTY position; its rent goes to `rent_to`.
pub fn close_position2(position: Pubkey, sender: Pubkey, rent_to: Pubkey) -> Instruction {
    ix(program(), vec![rw(position), signer(sender, false), rw(rent_to), ev(), ro(program())], disc::CLOSE_POSITION2.to_vec())
}

/// `go_to_a_bin(bin)`: moves an EMPTY stretch's active bin (anyone can; refused when liquidity lies between). `from_array` and `to_array`
/// are the bin arrays of the current and target bins when they exist (absent: the program id).
pub fn go_to_a_bin(pair: Pubkey, from_array: Option<Pubkey>, to_array: Option<Pubkey>, bin: i32) -> Instruction {
    ix(program(), vec![rw(pair), none(), from_array.map(ro).unwrap_or_else(none), to_array.map(ro).unwrap_or_else(none), ev(), ro(program())],
        i32le(data(disc::GO_TO_A_BIN), bin).done())
}

/// One spot add of a rebalance: `[min_delta, max_delta]` bins around the active bin; `y0` lamports in each bid bin; `x0` the Y-VALUE of
/// X in each ask bin (ask bin `i` holds `x0 / price(i)` of X, the SDK's `getAmountInBinsAskSide`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RebalanceAdd {
    pub min_delta: i32,
    pub max_delta: i32,
    pub x0: u64,
    pub y0: u64,
}

/// `rebalance_liquidity`: withdraw `remove` (explicit bins, 100 %), claim fees, and add `adds` around `active_id`, on the SAME position,
/// in one instruction. `rent_payer` (K_main) pays for the position's growth. The bin arrays of the old and the new range follow.
#[allow(clippy::too_many_arguments)]
pub fn rebalance_liquidity(lb: &Lb, position: Pubkey, owner: Pubkey, rent_payer: Pubkey, user_x: Pubkey, user_y: Pubkey, active_id: i32, claim_fee: bool,
    remove: Option<(i32, i32)>, adds: &[RebalanceAdd], arrays: &[Pubkey]) -> Instruction {
    // active_id, max_active_bin_slippage 3, should_claim_fee, should_claim_reward 0, min/max withdraw/deposit x and y, shrink_mode
    // ShrinkBoth (0), 31 bytes of padding
    let mut b = i32le(data(disc::REBALANCE_LIQUIDITY), active_id).u16(3).u8(claim_fee as u8).u8(0).u64(0).u64(u64::MAX).u64(0).u64(u64::MAX).u8(0);
    b = zeros(b, 31);
    match remove {
        Some((lo, hi)) => {
            // RemoveLiquidityParams { min_bin_id: Some, max_bin_id: Some, bps: 10_000, padding[16] }
            b = b.u32(1).u8(1).bytes(&lo.to_le_bytes()).u8(1).bytes(&hi.to_le_bytes()).u16(10_000);
            b = zeros(b, 16);
        }
        None => b = b.u32(0),
    }
    b = b.u32(adds.len() as u32);
    for a in adds {
        b = i32le(i32le(b, a.min_delta), a.max_delta).u64(a.x0).u64(a.y0).u64(0).u64(0).u8(0).u8(0);
        b = zeros(b, 16);
    }
    let tp = super::token_program();
    // owner == rent_payer would be one key in two roles; the compiler merges them, the metas stay one per role
    let mut accounts = vec![rw(position), rw(lb.pair), none(), rw(user_x), rw(user_y), rw(lb.rx()), rw(lb.ry()), ro(lb.x), ro(lb.y), signer(owner, owner == rent_payer),
        signer(rent_payer, true), ro(tp), ro(tp), ro(id(MEMO_PROGRAM)), ro(SYSTEM_PROGRAM), ev(), ro(program())];
    accounts.extend(arrays.iter().copied().map(rw));
    ix(program(), accounts, no_slices(b).done())
}

// ---- DLMM's price arithmetic (Q64.64) -------------------------------------------------------------------------------------------------

const ONE_Q64: u128 = 1u128 << 64;

/// The full 256-bit product of two u128 as (high, low).
fn mul_wide(a: u128, b: u128) -> (u128, u128) {
    let (a0, a1) = (a as u64 as u128, a >> 64);
    let (b0, b1) = (b as u64 as u128, b >> 64);
    let (p00, p01, p10, p11) = (a0 * b0, a0 * b1, a1 * b0, a1 * b1);
    let mid = (p00 >> 64) + (p01 as u64 as u128) + (p10 as u64 as u128);
    let lo = (p00 as u64 as u128) | (mid << 64);
    let hi = p11 + (p01 >> 64) + (p10 >> 64) + (mid >> 64);
    (hi, lo)
}
/// `⌊a · b / 2^64⌋` (`None` when it does not fit a u128) and its rounding-up form.
fn mul_shr64(a: u128, b: u128, up: bool) -> Option<u128> {
    let (hi, lo) = mul_wide(a, b);
    if hi >> 64 != 0 {
        return None;
    }
    let q = (hi << 64) | (lo >> 64);
    if up && lo as u64 != 0 {
        q.checked_add(1)
    } else {
        Some(q)
    }
}
/// `a · 2^64 / b` for `a < 2^64` (every amount here is a u64), rounded down or up.
fn shl64_div(a: u128, b: u128, up: bool) -> Option<u128> {
    if b == 0 || a >> 64 != 0 {
        return None;
    }
    let n = a << 64;
    let q = n / b;
    if up && n % b != 0 {
        q.checked_add(1)
    } else {
        Some(q)
    }
}

/// `price(id)` = `(1 + bin_step / 10⁴)^id` as Q64.64: Y base units per X base unit (for wX/SOL, 9 decimals each, lamports per
/// wX base unit = SOL per wX). Square-and-multiply on the inverted base for a positive id, as DLMM does. `None` outside the range
/// where it fits.
pub fn price_q64(bin_step: u16, id: i32) -> Option<u128> {
    if id == 0 {
        return Some(ONE_Q64);
    }
    let base = ONE_Q64 + ((bin_step as u128) << 64) / 10_000;
    let mut e = id.unsigned_abs();
    // base > 1: work with its inverse (< 1, which cannot overflow), as DLMM does; inverse^|id| is base^-|id|
    let mut sq = u128::MAX / base;
    let mut result = ONE_Q64;
    while e > 0 {
        if e & 1 == 1 {
            result = mul_shr64(result, sq, false)?;
        }
        e >>= 1;
        if e > 0 {
            sq = mul_shr64(sq, sq, false)?;
        }
    }
    if result == 0 {
        return None;
    }
    // a positive id inverts that result again: base^+id
    Some(if id > 0 { u128::MAX / result } else { result })
}

/// The price of a floor in Q64.64: `lamports_per_whole · 2^64 / 10⁹` (a whole wX is 10⁹ base units).
pub fn price_of_floor(lamports_per_whole: u64) -> u128 {
    ((lamports_per_whole as u128) << 64) / 1_000_000_000
}
/// The floor a Q64.64 price means, in lamports per whole wX (rounded down).
pub fn floor_of_price(p: u128) -> Option<u64> {
    u64::try_from(mul_shr64(p, 1_000_000_000, false)?).ok()
}

/// The bin whose price is nearest `p` (by difference), within `[-MAX_BIN_ID, MAX_BIN_ID]`. Binary search on the monotone
/// [`price_q64`].
pub fn bin_for_price(bin_step: u16, p: u128) -> Option<i32> {
    if p == 0 || bin_step == 0 {
        return None;
    }
    // the largest id whose price ≤ p
    let (mut lo, mut hi) = (-MAX_BIN_ID, MAX_BIN_ID);
    while lo < hi {
        let mid = lo + (hi - lo + 1) / 2;
        match price_q64(bin_step, mid) {
            Some(v) if v <= p => lo = mid,
            Some(_) => hi = mid - 1,
            // beyond what fits: treated as above p for a positive id, below for a negative one
            None if mid > 0 => hi = mid - 1,
            None => lo = mid,
        }
    }
    let below = price_q64(bin_step, lo)?;
    match price_q64(bin_step, lo + 1) {
        Some(above) if above - p < p.saturating_sub(below) => Some(lo + 1),
        _ => Some(lo),
    }
}

/// Bins on each side of the active bin for a band of `±band_bps` in price: `⌈ln(1 + band) / ln(1 + step)⌉`-nearest, as an integer
/// search on [`price_q64`] (the band's upper price over the centre's).
pub fn half_width(bin_step: u16, band_bps: u32) -> Option<i32> {
    let upper = ONE_Q64 + (((band_bps as u128) << 64) / 10_000);
    bin_for_price(bin_step, upper)
}

// ---- accounts ------------------------------------------------------------------------------------------------------------------------

pub const LB_PAIR_LEN: usize = 904;
pub const POSITION_V2_LEN: usize = 8_120;
pub const BIN_ARRAY_LEN: usize = 10_136;
/// `account:LbPair`, `account:PositionV2`, `account:BinArray` (Anchor: `sha256("account:<name>")[..8]`).
pub fn lb_pair_disc() -> [u8; 8] {
    crate::soltx::anchor_disc("account", "LbPair")
}
pub fn position_disc() -> [u8; 8] {
    crate::soltx::anchor_disc("account", "PositionV2")
}
pub fn bin_array_disc() -> [u8; 8] {
    crate::soltx::anchor_disc("account", "BinArray")
}

fn u16_at(d: &[u8], at: usize) -> u16 {
    u16::from_le_bytes(d[at..at + 2].try_into().expect("2"))
}
fn u32_at(d: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(d[at..at + 4].try_into().expect("4"))
}
fn i32_at(d: &[u8], at: usize) -> i32 {
    i32::from_le_bytes(d[at..at + 4].try_into().expect("4"))
}
fn i64_at(d: &[u8], at: usize) -> i64 {
    i64::from_le_bytes(d[at..at + 8].try_into().expect("8"))
}
fn u64_at(d: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(d[at..at + 8].try_into().expect("8"))
}
fn u128_at(d: &[u8], at: usize) -> u128 {
    u128::from_le_bytes(d[at..at + 16].try_into().expect("16"))
}
fn key_at(d: &[u8], at: usize) -> Pubkey {
    d[at..at + 32].try_into().expect("32")
}
fn check(owner: &Pubkey, d: &[u8], len: usize, disc: [u8; 8], what: &str) -> Result<(), String> {
    if *owner != program() {
        return Err(format!("{what}: owned by {}, not DLMM", crate::soltx::b58_encode(owner)));
    }
    if d.len() != len {
        return Err(format!("{what}: {} bytes, not {len}", d.len()));
    }
    if d[..8] != disc {
        return Err(format!("{what}: wrong discriminator"));
    }
    Ok(())
}

/// `LbPair` (904 bytes): what the desk reads. `StaticParameters` at 8, `VariableParameters` at 40, then `bump`, `bin_step_seed`,
/// `pair_type` @75, `active_id` @76 (4 bytes, the price watch's one read), `bin_step` @80, `status` @82, the mints @88 and @120, the reserves
/// @152 and @184.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LbPair {
    pub base_factor: u16,
    pub filter_period: u16,
    pub decay_period: u16,
    pub reduction_factor: u16,
    pub variable_fee_control: u32,
    pub max_volatility_accumulator: u32,
    pub min_bin_id: i32,
    pub max_bin_id: i32,
    pub protocol_share: u16,
    pub base_fee_power_factor: u8,
    pub volatility_accumulator: u32,
    pub volatility_reference: u32,
    pub index_reference: i32,
    pub last_update_timestamp: i64,
    pub pair_type: u8,
    pub active_id: i32,
    pub bin_step: u16,
    /// 0 enabled, 1 disabled (Meteora's operator can disable a pair: `set_pair_status`).
    pub status: u8,
    pub token_x_mint: Pubkey,
    pub token_y_mint: Pubkey,
    pub reserve_x: Pubkey,
    pub reserve_y: Pubkey,
}

/// The offset of `active_id` in an `LbPair` and the length of a `dataSlice` that reads only it (the price watch's read: 4 bytes a pool).
pub const ACTIVE_ID_OFFSET: usize = 76;
pub fn active_id_of_slice(d: &[u8]) -> Option<i32> {
    (d.len() >= 4).then(|| i32_at(d, 0))
}

impl LbPair {
    pub fn decode(owner: &Pubkey, d: &[u8]) -> Result<LbPair, String> {
        check(owner, d, LB_PAIR_LEN, lb_pair_disc(), "DLMM LbPair")?;
        Ok(LbPair {
            base_factor: u16_at(d, 8),
            filter_period: u16_at(d, 10),
            decay_period: u16_at(d, 12),
            reduction_factor: u16_at(d, 14),
            variable_fee_control: u32_at(d, 16),
            max_volatility_accumulator: u32_at(d, 20),
            min_bin_id: i32_at(d, 24),
            max_bin_id: i32_at(d, 28),
            protocol_share: u16_at(d, 32),
            base_fee_power_factor: d[34],
            volatility_accumulator: u32_at(d, 40),
            volatility_reference: u32_at(d, 44),
            index_reference: i32_at(d, 48),
            last_update_timestamp: i64_at(d, 56),
            pair_type: d[75],
            active_id: i32_at(d, 76),
            bin_step: u16_at(d, 80),
            status: d[82],
            token_x_mint: key_at(d, 88),
            token_y_mint: key_at(d, 120),
            reserve_x: key_at(d, 152),
            reserve_y: key_at(d, 184),
        })
    }

    /// The base fee rate in 1e9: `base_factor × bin_step × 10 × 10^base_fee_power_factor` (preset 262: 8,000 × 50 × 10 = 0.4 %).
    pub fn base_fee_rate(&self) -> u128 {
        self.base_factor as u128 * self.bin_step as u128 * 10 * 10u128.pow(self.base_fee_power_factor as u32)
    }
    /// The variable fee rate in 1e9 for a volatility accumulator `v`: `⌈control · (v · bin_step)² / 10¹¹⌉`.
    pub fn variable_fee_rate(&self, v: u32) -> u128 {
        let x = v as u128 * self.bin_step as u128;
        (self.variable_fee_control as u128 * x * x).div_ceil(100_000_000_000)
    }
    /// The total rate for accumulator `v`, capped at 10 %.
    pub fn fee_rate(&self, v: u32) -> u128 {
        (self.base_fee_rate() + self.variable_fee_rate(v)).min(MAX_FEE_RATE)
    }
    /// The volatility references a swap at `now` starts from (DLMM `update_references`): after `filter_period` the index reference is
    /// the active bin; after `decay_period` the volatility reference is 0, else `accumulator × reduction_factor / 10⁴`.
    pub fn references_at(&self, now: i64) -> (u32, i32) {
        let elapsed = now.saturating_sub(self.last_update_timestamp);
        let idx = if elapsed >= self.filter_period as i64 { self.active_id } else { self.index_reference };
        let vol = if elapsed >= self.filter_period as i64 {
            if elapsed >= self.decay_period as i64 {
                0
            } else {
                (self.volatility_accumulator as u64 * self.reduction_factor as u64 / 10_000) as u32
            }
        } else {
            self.volatility_reference
        };
        (vol, idx)
    }
    /// The all-in rate a swap would pay in the active bin right now (what the site's quote shows).
    pub fn current_fee_rate(&self, now: i64) -> u128 {
        let (vol, idx) = self.references_at(now);
        let acc = (vol as u64 + (idx - self.active_id).unsigned_abs() as u64 * 10_000).min(self.max_volatility_accumulator as u64) as u32;
        self.fee_rate(acc)
    }
    pub fn enabled(&self) -> bool {
        self.status == 0
    }
}

/// `PositionV2` (8,120 bytes): `lb_pair` @8, `owner` @40, `liquidity_shares` @72 (70 × u128, scaled by 2^64), reward infos @1192,
/// fee infos @4552 (70 × {fee_x_per_token_complete u128, fee_y_per_token_complete u128, fee_x_pending u64, fee_y_pending u64}),
/// `lower_bin_id` @7912, `upper_bin_id` @7916.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PositionV2 {
    pub lb_pair: Pubkey,
    pub owner: Pubkey,
    pub lower: i32,
    pub upper: i32,
    pub liquidity_shares: Vec<u128>,
    pub fee_infos: Vec<FeeInfo>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FeeInfo {
    pub fee_x_per_token_complete: u128,
    pub fee_y_per_token_complete: u128,
    pub fee_x_pending: u64,
    pub fee_y_pending: u64,
}

impl PositionV2 {
    pub fn decode(owner: &Pubkey, d: &[u8]) -> Result<PositionV2, String> {
        check(owner, d, POSITION_V2_LEN, position_disc(), "DLMM PositionV2")?;
        let (lower, upper) = (i32_at(d, 7912), i32_at(d, 7916));
        if upper < lower {
            return Err(format!("DLMM PositionV2: range [{lower}, {upper}]"));
        }
        let width = ((upper - lower + 1).max(0) as usize).min(BINS_PER_ARRAY as usize);
        let liquidity_shares = (0..width).map(|i| u128_at(d, 72 + 16 * i)).collect();
        let fee_infos = (0..width)
            .map(|i| {
                let o = 4_552 + 48 * i;
                FeeInfo { fee_x_per_token_complete: u128_at(d, o), fee_y_per_token_complete: u128_at(d, o + 16), fee_x_pending: u64_at(d, o + 32), fee_y_pending: u64_at(d, o + 40) }
            })
            .collect();
        Ok(PositionV2 { lb_pair: key_at(d, 8), owner: key_at(d, 40), lower, upper, liquidity_shares, fee_infos })
    }
    pub fn width(&self) -> i32 {
        self.upper - self.lower + 1
    }
    /// Whether it holds no liquidity at all (it can be closed after its fees are claimed).
    pub fn is_empty(&self) -> bool {
        self.liquidity_shares.iter().all(|s| *s == 0)
    }
}

/// One bin of a `BinArray` (144 bytes): `amount_x` @0, `amount_y` @8, `price` @16 (Q64.64), `liquidity_supply` @32, the fee-per-token
/// accumulators @80 and @96.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Bin {
    pub amount_x: u64,
    pub amount_y: u64,
    pub price: u128,
    pub liquidity_supply: u128,
    pub fee_x_per_token_stored: u128,
    pub fee_y_per_token_stored: u128,
}

/// A decoded `BinArray`: its index and its 70 bins (bin `index × 70 + k`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BinArray {
    pub index: i64,
    pub lb_pair: Pubkey,
    pub bins: Vec<Bin>,
}

impl BinArray {
    pub fn decode(owner: &Pubkey, d: &[u8]) -> Result<BinArray, String> {
        check(owner, d, BIN_ARRAY_LEN, bin_array_disc(), "DLMM BinArray")?;
        let bins = (0..BINS_PER_ARRAY as usize)
            .map(|k| {
                let o = 56 + 144 * k;
                Bin { amount_x: u64_at(d, o), amount_y: u64_at(d, o + 8), price: u128_at(d, o + 16), liquidity_supply: u128_at(d, o + 32),
                    fee_x_per_token_stored: u128_at(d, o + 80), fee_y_per_token_stored: u128_at(d, o + 96) }
            })
            .collect();
        Ok(BinArray { index: i64_at(d, 8), lb_pair: key_at(d, 24), bins })
    }
    /// The bins of this array keyed by bin id.
    pub fn by_id(&self) -> impl Iterator<Item = (i32, Bin)> + '_ {
        let base = (self.index * BINS_PER_ARRAY as i64) as i32;
        self.bins.iter().enumerate().map(move |(k, b)| (base + k as i32, *b))
    }
}

/// Every bin of the decoded arrays, by id.
pub fn bins_of(arrays: &[BinArray]) -> BTreeMap<i32, Bin> {
    arrays.iter().flat_map(|a| a.by_id()).collect()
}

/// What a position holds: `Σ share · amount / liquidity_supply` per bin, rounded down per bin as the program pays it (checked against
/// what `remove_liquidity_by_range2` paid). A bin the arrays do not cover is an error (the amounts would be understated).
pub fn position_amounts(p: &PositionV2, bins: &BTreeMap<i32, Bin>) -> Result<(u64, u64), String> {
    let (mut x, mut y) = (0u128, 0u128);
    for (k, share) in p.liquidity_shares.iter().enumerate() {
        if *share == 0 {
            continue;
        }
        let id = p.lower + k as i32;
        let b = bins.get(&id).ok_or_else(|| format!("bin {id} of the position is in no bin array given"))?;
        if b.liquidity_supply == 0 {
            continue;
        }
        let (hx, lx) = mul_wide(*share, b.amount_x as u128);
        let (hy, ly) = mul_wide(*share, b.amount_y as u128);
        x += div_wide(hx, lx, b.liquidity_supply)?;
        y += div_wide(hy, ly, b.liquidity_supply)?;
    }
    Ok((u64::try_from(x).map_err(|_| "position X over u64")?, u64::try_from(y).map_err(|_| "position Y over u64")?))
}

/// `(hi:lo) / d` when it fits a u128 (shift-subtract long division; `d ≠ 0`).
fn div_wide(hi: u128, lo: u128, d: u128) -> Result<u128, String> {
    if hi == 0 {
        return Ok(lo / d);
    }
    if hi >= d {
        return Err("division overflow".into());
    }
    let (mut r, mut q) = (hi, 0u128);
    for i in (0..128).rev() {
        let carry = r >> 127;
        r = (r << 1) | ((lo >> i) & 1);
        q <<= 1;
        if carry == 1 || r >= d {
            r = r.wrapping_sub(d);
            q |= 1;
        }
    }
    Ok(q)
}

/// What `claim_fee2` would pay now: per bin `pending + ⌊(share / 2^64) · (stored − complete) / 2^64⌋` (checked against what the chain
/// paid). `(X, Y)`.
pub fn claimable_fees(p: &PositionV2, bins: &BTreeMap<i32, Bin>) -> Result<(u64, u64), String> {
    let (mut x, mut y) = (0u128, 0u128);
    for (k, share) in p.liquidity_shares.iter().enumerate() {
        let info = p.fee_infos[k];
        let id = p.lower + k as i32;
        let mut fx = info.fee_x_pending as u128;
        let mut fy = info.fee_y_pending as u128;
        if *share != 0 {
            let b = bins.get(&id).ok_or_else(|| format!("bin {id} of the position is in no bin array given"))?;
            let s = *share >> 64;
            fx += mul_shr64(s, b.fee_x_per_token_stored.saturating_sub(info.fee_x_per_token_complete), false).ok_or("fee overflow")?;
            fy += mul_shr64(s, b.fee_y_per_token_stored.saturating_sub(info.fee_y_per_token_complete), false).ok_or("fee overflow")?;
        }
        x += fx;
        y += fy;
    }
    Ok((u64::try_from(x).map_err(|_| "fee X over u64")?, u64::try_from(y).map_err(|_| "fee Y over u64")?))
}

// ---- a quote over the bins ---------------------------------------------------------------------------------------------------------

/// What a swap would do (DLMM's `swap_exact_in` over the bins; `swap_for_y` = X in, Y out: a SELL of wX; otherwise a BUY of wX).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SwapQuote {
    pub amount_in_used: u64,
    pub amount_out: u64,
    pub fee: u64,
    /// The active bin after the swap.
    pub end_id: i32,
    /// The whole input was used (false: the liquidity the arrays show ran out first).
    pub complete: bool,
    /// The all-in rate of the last bin touched, 1e9.
    pub last_rate: u128,
}

/// Walks the bins from the active one in the swap's direction (spike S4 §5.1 item 3: "a walk over the bins"). `bins` must hold every bin
/// the swap can reach (the arrays a swap would be given); a bin missing from `bins` ends the walk (`complete` false).
pub fn quote_swap(pair: &LbPair, bins: &BTreeMap<i32, Bin>, swap_for_y: bool, amount_in: u64, now: i64) -> Result<SwapQuote, String> {
    let (vol_ref, idx_ref) = pair.references_at(now);
    let mut left = amount_in as u128;
    let (mut out, mut fee_total, mut last_rate) = (0u128, 0u128, pair.base_fee_rate());
    let mut id = pair.active_id;
    let mut end_id = id;
    let step: i32 = if swap_for_y { -1 } else { 1 };
    let mut complete = true;
    while left > 0 {
        let Some(b) = bins.get(&id) else {
            complete = false;
            break;
        };
        let has = if swap_for_y { b.amount_y > 0 } else { b.amount_x > 0 };
        if has {
            let acc = (vol_ref as u64 + (idx_ref - id).unsigned_abs() as u64 * 10_000).min(pair.max_volatility_accumulator as u64) as u32;
            let rate = pair.fee_rate(acc);
            last_rate = rate;
            let price = b.price;
            let max_out = if swap_for_y { b.amount_y } else { b.amount_x } as u128;
            let max_in = if swap_for_y { shl64_div(max_out, price, true) } else { mul_shr64(max_out, price, true) }.ok_or("bin price overflow")?;
            // compute_fee: the fee on an amount that EXCLUDES it: ⌈x · rate / (1e9 − rate)⌉
            let max_fee = (max_in * rate).div_ceil(FEE_PRECISION - rate);
            if left >= max_in + max_fee {
                left -= max_in + max_fee;
                out += max_out;
                fee_total += max_fee;
                end_id = id;
            } else {
                let fee = (left * rate).div_ceil(FEE_PRECISION);
                let net = left - fee;
                let got = if swap_for_y { mul_shr64(net, price, false) } else { shl64_div(net, price, false) }.ok_or("bin price overflow")?.min(max_out);
                out += got;
                fee_total += fee;
                left = 0;
                end_id = id;
                break;
            }
        }
        // the bin is spent for this direction: the next one
        let next = id.checked_add(step).ok_or("bin id overflow")?;
        if left == 0 {
            break;
        }
        id = next;
        end_id = id;
        if id < pair.min_bin_id || id > pair.max_bin_id {
            complete = false;
            break;
        }
    }
    Ok(SwapQuote {
        amount_in_used: (amount_in as u128 - left) as u64,
        amount_out: u64::try_from(out).map_err(|_| "out over u64")?,
        fee: u64::try_from(fee_total).map_err(|_| "fee over u64")?,
        end_id,
        complete,
        last_rate,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_price_of_bin_zero_is_one_and_it_grows_by_the_bin_step() {
        assert_eq!(price_q64(50, 0), Some(ONE_Q64));
        let p1 = price_q64(50, 1).unwrap() as f64 / ONE_Q64 as f64;
        assert!((p1 - 1.005).abs() < 1e-12, "{p1}");
        let m1 = price_q64(50, -1).unwrap() as f64 / ONE_Q64 as f64;
        assert!((m1 - 1.0 / 1.005).abs() < 1e-12, "{m1}");
        let p586 = price_q64(50, 586).unwrap() as f64 / ONE_Q64 as f64;
        assert!((p586 / 1.005f64.powi(586) - 1.0).abs() < 1e-9, "{p586}");
    }

    #[test]
    fn bin_for_price_inverts_price_q64() {
        for id in [-5000, -1, 0, 1, 586, 1200] {
            let p = price_q64(50, id).unwrap();
            assert_eq!(bin_for_price(50, p), Some(id));
            // a hair above and below stay in the same bin
            assert_eq!(bin_for_price(50, p + p / 100_000), Some(id));
            assert_eq!(bin_for_price(50, p - p / 100_000), Some(id));
        }
        // Milady's floor 18.61 SOL is bin 586 at step 50
        assert_eq!(bin_for_price(50, price_of_floor(18_610_000_000)), Some(586));
        assert_eq!(bin_for_price(50, 0), None);
    }

    #[test]
    fn a_ten_percent_band_is_nineteen_or_twenty_bins_a_side() {
        let h = half_width(50, 1_000).unwrap();
        assert_eq!(h, 19, "ln(1.1) / ln(1.005) = 19.11");
        assert_eq!(half_width(30, 1_000), Some(32));
    }

    #[test]
    fn the_fee_rates_are_dlmms() {
        let p = LbPair {
            base_factor: 8000, filter_period: 120, decay_period: 1200, reduction_factor: 5000, variable_fee_control: 10_000, max_volatility_accumulator: 250_000,
            min_bin_id: -7032, max_bin_id: 7032, protocol_share: 1000, base_fee_power_factor: 0, volatility_accumulator: 0, volatility_reference: 0,
            index_reference: 586, last_update_timestamp: 0, pair_type: 3, active_id: 586, bin_step: 50, status: 0, token_x_mint: [0; 32], token_y_mint: [1; 32],
            reserve_x: [2; 32], reserve_y: [3; 32],
        };
        assert_eq!(p.base_fee_rate(), 4_000_000, "0.4 %");
        assert_eq!(p.variable_fee_rate(0), 0);
        // one bin of volatility: 10,000 · (10,000 · 50)² / 1e11 = 25,000 (0.0025 %)
        assert_eq!(p.variable_fee_rate(10_000), 25_000);
        // a 25-bin move: the cap of the accumulator, 250,000: 1.5625 %
        assert_eq!(p.variable_fee_rate(250_000), 15_625_000);
        assert_eq!(p.fee_rate(250_000), 19_625_000);
        // the references decay: filter 120 s, decay 1,200 s
        let mut q = p.clone();
        q.volatility_accumulator = 100_000;
        q.volatility_reference = 40_000;
        q.last_update_timestamp = 1_000;
        q.active_id = 600;
        assert_eq!(q.references_at(1_060), (40_000, 586), "inside the filter period nothing moves");
        assert_eq!(q.references_at(1_200), (50_000, 600), "past the filter period: 100,000 × 5,000 / 10,000");
        assert_eq!(q.references_at(2_300), (0, 600), "past the decay period: 0");
    }

    #[test]
    fn a_wide_product_divides_exactly() {
        let (a, b) = (u128::MAX / 3, 1u128 << 70);
        let (hi, lo) = mul_wide(a, b);
        assert!(hi != 0, "a real 256-bit product");
        assert_eq!(div_wide(hi, lo, b).unwrap(), a);
        assert!(div_wide(5, 0, 3).is_err());
        assert_eq!(div_wide(0, 100, 7).unwrap(), 14);
    }
}
