//! Spike S4 for canister-only Rook on Solana (docs/v9/canister-only-solana.md §5.4, open item S4): which wX/SOL pool type can
//! the canister keep concentrated around the floor and re-centre?
//!
//! Mainnet program bytes in LiteSVM: Meteora DLMM (`LBUZ…`, dumped 2026-10-10 into tests/fixtures/dlmm.so; IDL 0.12.0 of
//! MeteoraAg/dlmm-sdk), DAMM v2 0.2.4 (the Q1 dump). Six DLMM `PresetParameter2` accounts are cloned from mainnet
//! (tests/fixtures/s4_dlmm_presets.json, read-only `getMultipleAccounts`). DLMM instructions are hand-encoded from the IDL
//! (Anchor discriminator + Borsh), as the canister's `solix2` builders would be. Stand-ins, as in S1:
//!   `env.payer` = K_main (fee payer, nonce authority);  `desk` = K_desk[i] (position owner);  `canister` = K_coll[i]
//!   (wX mint authority). wX = classic SPL, 9 decimals, no freeze authority. Price = SOL per wX = lamports per base unit.
//! Run: `cargo test -p rook_backing --test s4_wx_pools -- --nocapture --test-threads 1`
//! Write-up: docs/v9/spikes/s4-wx-pools.md.

#[path = "harness/mod.rs"]
mod harness;

use anchor_lang::{
    prelude::*,
    solana_program::{instruction::Instruction, system_instruction},
    InstructionData, ToAccountMetas,
};
use anchor_spl::{associated_token::get_associated_token_address, token::spl_token};
use base64::Engine;
use harness::*;
use rook_backing::{constants::*, cp_amm};
use solana_account::Account as SolAccount;
use solana_compute_budget_interface::ComputeBudgetInstruction;
use solana_keypair::Keypair;
use solana_message::Message;
use solana_signer::Signer;

const DLMM_ID: Pubkey = pubkey!("LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo");
const MEMO_ID: Pubkey = pubkey!("MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr");
const ILM_BASE_KEY: Pubkey = pubkey!("MFGQxwAmB91SwuYX36okv2Qmdc9aMuHTwWGUrp4AtB1");
const RENT_SYSVAR: Pubkey = pubkey!("SysvarRent111111111111111111111111111111111");
const TWO64: f64 = 18_446_744_073_709_551_616f64;
const FLOOR: f64 = 18.61; // Milady's floor, SOL per wX, as in S1

// ------------------------------------------------------------------------------------------------- DLMM discriminators (IDL 0.12.0)
const D_INIT_PAIR2: [u8; 8] = hex8("493b2478ed536cc6");
const D_INIT_CUSTOM2: [u8; 8] = hex8("f349817e3313f16b");
const D_INIT_BIN_ARRAY: [u8; 8] = hex8("235613b94ed44bd3");
const D_INIT_POSITION_PDA: [u8; 8] = hex8("2e527d92558de499");
const D_INCREASE_LEN2: [u8; 8] = hex8("ffd2cc477389e171");
const D_ADD_STRATEGY2: [u8; 8] = hex8("03dd95da6f8d76d5");
const D_REBALANCE: [u8; 8] = hex8("5c04b0c177b95309");
const D_SWAP2: [u8; 8] = hex8("414b3f4ceb5b5b88");
const D_CLAIM_FEE2: [u8; 8] = hex8("70bf65ab1c907fbb");
const D_REMOVE_RANGE2: [u8; 8] = hex8("cc02c391359191cd");
const D_CLOSE_POSITION2: [u8; 8] = hex8("ae5a2373ba2893e2");
const D_GO_TO_A_BIN: [u8; 8] = hex8("9248aee028fd54ae");
const D_CLOSE_BIN_ARRAY: [u8; 8] = hex8("44ae5850b5cc13e0");

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

/// Strategy types (IDL enum order).
#[derive(Clone, Copy, Debug)]
enum Shape {
    Spot = 6,   // SpotImBalanced
    Curve = 7,  // CurveImBalanced
    BidAsk = 8, // BidAskImBalanced
}

// ------------------------------------------------------------------------------------------------- Borsh by hand
struct W(Vec<u8>);
impl W {
    fn new(d: [u8; 8]) -> Self {
        W(d.to_vec())
    }
    fn i32(mut self, v: i32) -> Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    fn u16(mut self, v: u16) -> Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    fn u64(mut self, v: u64) -> Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    fn i64(mut self, v: i64) -> Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    fn u8(mut self, v: u8) -> Self {
        self.0.push(v);
        self
    }
    fn zeros(mut self, n: usize) -> Self {
        self.0.extend(std::iter::repeat(0u8).take(n));
        self
    }
    /// RemainingAccountsInfo { slices: [] }
    fn no_slices(mut self) -> Self {
        self.0.extend_from_slice(&0u32.to_le_bytes());
        self
    }
    fn done(self) -> Vec<u8> {
        self.0
    }
}

fn w(k: Pubkey) -> AccountMeta {
    AccountMeta::new(k, false)
}
fn r(k: Pubkey) -> AccountMeta {
    AccountMeta::new_readonly(k, false)
}
fn ws(k: Pubkey) -> AccountMeta {
    AccountMeta::new(k, true)
}
fn rs(k: Pubkey) -> AccountMeta {
    AccountMeta::new_readonly(k, true)
}
fn none() -> AccountMeta {
    AccountMeta::new_readonly(DLMM_ID, false)
}
fn ev() -> AccountMeta {
    r(event_authority(&DLMM_ID))
}

// ------------------------------------------------------------------------------------------------- DLMM PDAs and math
fn ordered(a: &Pubkey, b: &Pubkey) -> (Pubkey, Pubkey) {
    if a < b {
        (*a, *b)
    } else {
        (*b, *a)
    }
}
fn pair_with_preset(preset: &Pubkey, x: &Pubkey, y: &Pubkey) -> Pubkey {
    let (lo, hi) = ordered(x, y);
    pda(&[preset.as_ref(), lo.as_ref(), hi.as_ref()], &DLMM_ID)
}
fn pair_customizable(x: &Pubkey, y: &Pubkey) -> Pubkey {
    let (lo, hi) = ordered(x, y);
    pda(&[ILM_BASE_KEY.as_ref(), lo.as_ref(), hi.as_ref()], &DLMM_ID)
}
fn reserve(pair: &Pubkey, mint: &Pubkey) -> Pubkey {
    pda(&[pair.as_ref(), mint.as_ref()], &DLMM_ID)
}
fn oracle(pair: &Pubkey) -> Pubkey {
    pda(&[b"oracle", pair.as_ref()], &DLMM_ID)
}
fn bin_array(pair: &Pubkey, index: i64) -> Pubkey {
    pda(&[b"bin_array", pair.as_ref(), &index.to_le_bytes()], &DLMM_ID)
}
fn position_pda(pair: &Pubkey, base: &Pubkey, lower: i32, width: i32) -> Pubkey {
    pda(&[b"position", pair.as_ref(), base.as_ref(), &lower.to_le_bytes(), &width.to_le_bytes()], &DLMM_ID)
}
fn array_index(bin: i32) -> i64 {
    (bin as i64).div_euclid(70)
}
fn arrays_for(pair: &Pubkey, lo: i32, hi: i32) -> Vec<Pubkey> {
    (array_index(lo)..=array_index(hi)).map(|i| bin_array(pair, i)).collect()
}
fn bin_price(bin_step: u16, id: i32) -> f64 {
    (1.0 + bin_step as f64 / 10_000.0).powi(id)
}
fn id_for_price(bin_step: u16, p: f64) -> i32 {
    (p.ln() / (1.0 + bin_step as f64 / 10_000.0).ln()).round() as i32
}
/// Bins on each side for a band of ±`pct` % in price.
fn half_width(bin_step: u16, pct: f64) -> i32 {
    ((1.0 + pct / 100.0).ln() / (1.0 + bin_step as f64 / 10_000.0).ln()).round() as i32
}

#[derive(Clone, Copy)]
struct Lb {
    pair: Pubkey,
    x: Pubkey,
    y: Pubkey,
    bin_step: u16,
}
impl Lb {
    fn rx(&self) -> Pubkey {
        reserve(&self.pair, &self.x)
    }
    fn ry(&self) -> Pubkey {
        reserve(&self.pair, &self.y)
    }
}

// LbPair offsets (zero-copy, after the 8-byte discriminator): StaticParameters 32 B, VariableParameters 32 B, bump 1,
// bin_step_seed 2, pair_type 1, active_id i32 @ 76, bin_step u16 @ 80, status u8 @ 82.
fn active_id(env: &Env, pair: &Pubkey) -> i32 {
    let d = env.raw(pair);
    i32::from_le_bytes(d[76..80].try_into().unwrap())
}
fn pair_type(env: &Env, pair: &Pubkey) -> u8 {
    env.raw(pair)[75]
}
fn volatility_acc(env: &Env, pair: &Pubkey) -> u32 {
    let d = env.raw(pair);
    u32::from_le_bytes(d[40..44].try_into().unwrap())
}
// PositionV2: lb_pair 32, owner 32, liquidity_shares 70×16, reward_infos 70×48, fee_infos 70×48 → lower_bin_id @ 7912.
fn position_range(env: &Env, position: &Pubkey) -> (i32, i32) {
    let d = env.raw(position);
    (i32::from_le_bytes(d[7912..7916].try_into().unwrap()), i32::from_le_bytes(d[7916..7920].try_into().unwrap()))
}

// ------------------------------------------------------------------------------------------------- DLMM instructions
fn ix_init_pair2(funder: &Pubkey, preset: &Pubkey, x: &Pubkey, y: &Pubkey, active: i32) -> Instruction {
    let pair = pair_with_preset(preset, x, y);
    Instruction {
        program_id: DLMM_ID,
        accounts: vec![
            w(pair),
            none(),
            r(*x),
            r(*y),
            w(reserve(&pair, x)),
            w(reserve(&pair, y)),
            w(oracle(&pair)),
            r(*preset),
            ws(*funder),
            none(),
            none(),
            r(spl_token::ID),
            r(spl_token::ID),
            r(anchor_lang::system_program::ID),
            ev(),
            r(DLMM_ID),
        ],
        data: W::new(D_INIT_PAIR2).i32(active).zeros(96).done(),
    }
}

struct Custom {
    active: i32,
    bin_step: u16,
    base_factor: u16,
    collect_fee_mode: u8,
}
fn ix_init_custom2(funder: &Pubkey, x: &Pubkey, y: &Pubkey, ux: &Pubkey, uy: &Pubkey, c: &Custom) -> Instruction {
    let pair = pair_customizable(x, y);
    Instruction {
        program_id: DLMM_ID,
        accounts: vec![
            w(pair),
            none(),
            r(*x),
            r(*y),
            w(reserve(&pair, x)),
            w(reserve(&pair, y)),
            w(oracle(&pair)),
            r(*ux),
            ws(*funder),
            none(),
            none(),
            r(spl_token::ID),
            r(spl_token::ID),
            r(anchor_lang::system_program::ID),
            r(*uy),
            ev(),
            r(DLMM_ID),
        ],
        // active_id, bin_step, base_factor, activation_type (1 = timestamp), has_alpha_vault, activation_point: None,
        // creator_pool_on_off_control, base_fee_power_factor, concrete_function_type, collect_fee_mode, padding[60]
        data: W::new(D_INIT_CUSTOM2)
            .i32(c.active)
            .u16(c.bin_step)
            .u16(c.base_factor)
            .u8(1)
            .u8(0)
            .u8(0)
            .u8(0)
            .u8(0)
            .u8(0)
            .u8(c.collect_fee_mode)
            .zeros(60)
            .done(),
    }
}
fn ix_init_bin_array(pair: &Pubkey, index: i64, funder: &Pubkey) -> Instruction {
    Instruction {
        program_id: DLMM_ID,
        accounts: vec![r(*pair), w(bin_array(pair, index)), ws(*funder), r(anchor_lang::system_program::ID)],
        data: W::new(D_INIT_BIN_ARRAY).i64(index).done(),
    }
}
fn ix_init_position_pda(payer: &Pubkey, base: &Pubkey, pair: &Pubkey, owner: &Pubkey, lower: i32, width: i32) -> Instruction {
    let mut accounts = vec![
        ws(*payer),
        rs(*base),
        w(position_pda(pair, base, lower, width)),
        r(*pair),
        rs(*owner),
        r(anchor_lang::system_program::ID),
        r(RENT_SYSVAR),
        ev(),
        r(DLMM_ID),
    ];
    // same key in two roles: one meta per role is fine for the runtime; Message::new dedups
    if base == owner {
        accounts[4] = rs(*owner);
    }
    Instruction { program_id: DLMM_ID, accounts, data: W::new(D_INIT_POSITION_PDA).i32(lower).i32(width).done() }
}
fn ix_increase_len2(funder: &Pubkey, pair: &Pubkey, position: &Pubkey, owner: &Pubkey, min_upper: i32) -> Instruction {
    Instruction {
        program_id: DLMM_ID,
        accounts: vec![ws(*funder), r(*pair), w(*position), rs(*owner), r(anchor_lang::system_program::ID), ev(), r(DLMM_ID)],
        data: W::new(D_INCREASE_LEN2).i32(min_upper).done(),
    }
}
#[allow(clippy::too_many_arguments)]
fn ix_add_strategy2(lb: &Lb, position: &Pubkey, sender: &Pubkey, ux: &Pubkey, uy: &Pubkey, ax: u64, ay: u64, active: i32, lo: i32, hi: i32, shape: Shape) -> Instruction {
    let mut accounts = vec![
        w(*position),
        w(lb.pair),
        none(),
        w(*ux),
        w(*uy),
        w(lb.rx()),
        w(lb.ry()),
        r(lb.x),
        r(lb.y),
        rs(*sender),
        r(spl_token::ID),
        r(spl_token::ID),
        ev(),
        r(DLMM_ID),
    ];
    accounts.extend(arrays_for(&lb.pair, lo, hi).into_iter().map(w));
    Instruction {
        program_id: DLMM_ID,
        accounts,
        data: W::new(D_ADD_STRATEGY2).u64(ax).u64(ay).i32(active).i32(3).i32(lo).i32(hi).u8(shape as u8).zeros(64).no_slices().done(),
    }
}
fn ix_swap2(lb: &Lb, user: &Pubkey, input: &Pubkey, output: &Pubkey, amount_in: u64, min_out: u64, arrays: &[Pubkey]) -> Instruction {
    let mut accounts = vec![
        w(lb.pair),
        none(),
        w(lb.rx()),
        w(lb.ry()),
        w(*input),
        w(*output),
        r(lb.x),
        r(lb.y),
        w(oracle(&lb.pair)),
        none(),
        rs(*user),
        r(spl_token::ID),
        r(spl_token::ID),
        r(MEMO_ID),
        ev(),
        r(DLMM_ID),
    ];
    accounts.extend(arrays.iter().copied().map(w));
    Instruction { program_id: DLMM_ID, accounts, data: W::new(D_SWAP2).u64(amount_in).u64(min_out).no_slices().done() }
}
fn ix_claim_fee2(lb: &Lb, position: &Pubkey, sender: &Pubkey, ux: &Pubkey, uy: &Pubkey, lo: i32, hi: i32) -> Instruction {
    let mut accounts = vec![
        w(lb.pair),
        w(*position),
        rs(*sender),
        w(lb.rx()),
        w(lb.ry()),
        w(*ux),
        w(*uy),
        r(lb.x),
        r(lb.y),
        r(spl_token::ID),
        r(spl_token::ID),
        r(MEMO_ID),
        ev(),
        r(DLMM_ID),
    ];
    accounts.extend(arrays_for(&lb.pair, lo, hi).into_iter().map(w));
    Instruction { program_id: DLMM_ID, accounts, data: W::new(D_CLAIM_FEE2).i32(lo).i32(hi).no_slices().done() }
}
fn ix_remove_range2(lb: &Lb, position: &Pubkey, sender: &Pubkey, ux: &Pubkey, uy: &Pubkey, lo: i32, hi: i32, bps: u16) -> Instruction {
    let mut accounts = vec![
        w(*position),
        w(lb.pair),
        none(),
        w(*ux),
        w(*uy),
        w(lb.rx()),
        w(lb.ry()),
        r(lb.x),
        r(lb.y),
        rs(*sender),
        r(spl_token::ID),
        r(spl_token::ID),
        r(MEMO_ID),
        ev(),
        r(DLMM_ID),
    ];
    accounts.extend(arrays_for(&lb.pair, lo, hi).into_iter().map(w));
    Instruction { program_id: DLMM_ID, accounts, data: W::new(D_REMOVE_RANGE2).i32(lo).i32(hi).u16(bps).no_slices().done() }
}
fn ix_close_position2(position: &Pubkey, sender: &Pubkey, rent_to: &Pubkey) -> Instruction {
    Instruction { program_id: DLMM_ID, accounts: vec![w(*position), rs(*sender), w(*rent_to), ev(), r(DLMM_ID)], data: D_CLOSE_POSITION2.to_vec() }
}
fn ix_go_to_a_bin(pair: &Pubkey, from_array: Option<Pubkey>, to_array: Option<Pubkey>, bin: i32) -> Instruction {
    Instruction {
        program_id: DLMM_ID,
        accounts: vec![w(*pair), none(), from_array.map(r).unwrap_or_else(none), to_array.map(r).unwrap_or_else(none), ev(), r(DLMM_ID)],
        data: W::new(D_GO_TO_A_BIN).i32(bin).done(),
    }
}
fn ix_close_bin_array(pair: &Pubkey, array: &Pubkey, rent_to: &Pubkey, signer: &Pubkey) -> Instruction {
    let mut accounts = vec![r(*pair), w(*array), ws(*rent_to), rs(*signer)];
    if rent_to == signer {
        accounts[3] = ws(*signer);
    }
    Instruction { program_id: DLMM_ID, accounts, data: D_CLOSE_BIN_ARRAY.to_vec() }
}

/// One spot add for the rebalance: `[min_delta, max_delta]` around the active bin, `y0` lamports in each bid bin, and
/// `x0` (value in Y lamports, Q0) for each ask bin, so ask bin `i` gets `x0 / price(i)` of X (SDK `getAmountInBinsAskSide`).
struct RbAdd {
    min_delta: i32,
    max_delta: i32,
    x0: u64,
    y0: u64,
}
#[allow(clippy::too_many_arguments)]
fn ix_rebalance(lb: &Lb, position: &Pubkey, owner: &Pubkey, rent_payer: &Pubkey, ux: &Pubkey, uy: &Pubkey, active: i32, claim_fee: bool, remove: Option<(i32, i32)>, adds: &[RbAdd], arrays: &[Pubkey]) -> Instruction {
    let mut d = W::new(D_REBALANCE)
        .i32(active)
        .u16(3) // max_active_bin_slippage
        .u8(claim_fee as u8)
        .u8(0) // should_claim_reward
        .u64(0) // min_withdraw_x
        .u64(u64::MAX) // max_deposit_x
        .u64(0) // min_withdraw_y
        .u64(u64::MAX) // max_deposit_y
        .u8(0) // shrink_mode: ShrinkBoth
        .zeros(31);
    // removes: Vec<RemoveLiquidityParams { min_bin_id: Option<i32>, max_bin_id: Option<i32>, bps: u16, padding[16] }>
    if let Some((lo, hi)) = remove {
        // None would mean "the active bin" (SDK validateAndSortRebalanceWithdraw), so the range is explicit
        d.0.extend_from_slice(&1u32.to_le_bytes());
        d = d.u8(1).i32(lo).u8(1).i32(hi).u16(10_000).zeros(16);
    } else {
        d.0.extend_from_slice(&0u32.to_le_bytes());
    }
    d.0.extend_from_slice(&(adds.len() as u32).to_le_bytes());
    for a in adds {
        d = d.i32(a.min_delta).i32(a.max_delta).u64(a.x0).u64(a.y0).u64(0).u64(0).u8(0).u8(0).zeros(16);
    }
    let data = d.no_slices().done();
    let mut accounts = vec![
        w(*position),
        w(lb.pair),
        none(),
        w(*ux),
        w(*uy),
        w(lb.rx()),
        w(lb.ry()),
        r(lb.x),
        r(lb.y),
        rs(*owner),
        ws(*rent_payer),
        r(spl_token::ID),
        r(spl_token::ID),
        r(MEMO_ID),
        r(anchor_lang::system_program::ID),
        ev(),
        r(DLMM_ID),
    ];
    if owner == rent_payer {
        accounts[9] = ws(*owner);
    }
    accounts.extend(arrays.iter().copied().map(w));
    Instruction { program_id: DLMM_ID, accounts, data }
}

// ------------------------------------------------------------------------------------------------- environment
fn dlmm_env() -> Env {
    let mut env = Env::new();
    let f = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures");
    env.svm.add_program_from_file(DLMM_ID, f.join("dlmm.so")).unwrap();
    let j: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(f.join("s4_dlmm_presets.json")).unwrap()).unwrap();
    for (_, v) in j.as_object().unwrap() {
        let k: Pubkey = v["pubkey"].as_str().unwrap().parse().unwrap();
        let data = base64::engine::general_purpose::STANDARD.decode(v["data"].as_str().unwrap()).unwrap();
        env.svm
            .set_account(k, SolAccount { lamports: v["lamports"].as_u64().unwrap(), data, owner: DLMM_ID, executable: false, rent_epoch: 0 })
            .unwrap();
    }
    env
}
fn preset(idx: u16) -> Pubkey {
    pda(&[b"preset_parameter2", &idx.to_le_bytes()], &DLMM_ID)
}
/// The preset used for the desk: index 262, bin step 50 (0.5 %), base fee 0.4 %, collect_fee_mode 1 (fees in Y only).
const DESK_PRESET: u16 = 262;

struct WxMint {
    mint: Pubkey,
    canister: Keypair,
}
fn new_wx(env: &mut Env, freeze: bool) -> WxMint {
    let canister = env.funded(1);
    let mint = Keypair::new();
    let rent = env.svm.minimum_balance_for_rent_exemption(82);
    let ck = canister.pubkey();
    let ixs = [
        system_instruction::create_account(&env.payer.pubkey(), &mint.pubkey(), rent, 82, &spl_token::ID),
        spl_token::instruction::initialize_mint2(&spl_token::ID, &mint.pubkey(), &ck, if freeze { Some(&ck) } else { None }, 9)
            .unwrap(),
    ];
    env.send_ok("create wX", &ixs, &[&mint]);
    WxMint { mint: mint.pubkey(), canister }
}
fn mint_wx(env: &mut Env, wx: &WxMint, owner: &Pubkey, amount: u64) -> Pubkey {
    let ata = env.ata(owner, &wx.mint);
    let ix = spl_token::instruction::mint_to_checked(&spl_token::ID, &wx.mint, &ata, &wx.canister.pubkey(), &[], amount, 9).unwrap();
    env.send_ok("mint wX", &[ix], &[&wx.canister]);
    ata
}
/// Wire size of a legacy transaction carrying `ixs` behind a durable-nonce advance + CU limit + CU price (the canister's
/// send line): 1 + 64·signers + message. Returns (bytes, signers, accounts).
fn nonce_tx(payer: &Pubkey, ixs: &[Instruction]) -> (usize, usize, usize) {
    let nonce = Pubkey::new_unique();
    let mut all = vec![
        system_instruction::advance_nonce_account(&nonce, payer),
        ComputeBudgetInstruction::set_compute_unit_limit(1_400_000),
        ComputeBudgetInstruction::set_compute_unit_price(1),
    ];
    all.extend_from_slice(ixs);
    let m = Message::new(&all, Some(payer));
    let n = m.header.num_required_signatures as usize;
    (1 + 64 * n + m.serialize().len(), n, m.account_keys.len())
}
fn first_err(logs: &str) -> String {
    logs.lines()
        .find(|l| l.contains("Error Message") || l.contains("failed:") || l.contains("Error Code"))
        .map(|l| l.trim().rsplit("Error Code: ").next().unwrap_or("").chars().take(200).collect())
        .unwrap_or_else(|| logs.lines().last().unwrap_or("").chars().take(160).collect())
}

// ------------------------------------------------------------------------------------------------- the desk's DLMM pool
struct Desk {
    lb: Lb,
    desk: Keypair,
    wx: WxMint,
    dx: Pubkey,
    dy: Pubkey,
    position: Pubkey,
    lo: i32,
    hi: i32,
}

/// Creates the wX/SOL pair on `preset` at FLOOR, bin arrays for `[lo, hi]`, one PDA position owned by the desk (base = desk:
/// no throwaway key), extended past 70 bins when needed, and deposits `sol_side` SOL + the same value of wX with `shape`.
/// Returns the desk and a size/CU log.
fn open_desk(env: &mut Env, bin_step: u16, preset_idx: Option<u16>, custom_fee_factor: Option<u16>, pct: f64, sol_side: u64, shape: Shape, log: &mut Vec<String>) -> Desk {
    let wx = new_wx(env, false);
    let desk = env.funded(1_000);
    let active = id_for_price(bin_step, FLOOR);
    let p_active = bin_price(bin_step, active);
    let _ = p_active;
    let half = half_width(bin_step, pct);
    let (lo, hi) = (active - half, active + half);
    let wx_amount = (sol_side as f64 / p_active) as u64 + 1_000_000;
    let dx = mint_wx(env, &wx, &desk.pubkey(), wx_amount);
    let dy = env.wsol(&desk, sol_side + LAMPORTS);
    let (pair, init) = match (preset_idx, custom_fee_factor) {
        (Some(i), _) => (pair_with_preset(&preset(i), &wx.mint, &NATIVE_MINT), ix_init_pair2(&desk.pubkey(), &preset(i), &wx.mint, &NATIVE_MINT, active)),
        (None, Some(bf)) => (
            pair_customizable(&wx.mint, &NATIVE_MINT),
            ix_init_custom2(&desk.pubkey(), &wx.mint, &NATIVE_MINT, &dx, &dy, &Custom { active, bin_step, base_factor: bf, collect_fee_mode: 1 }),
        ),
        _ => unreachable!(),
    };
    let lb = Lb { pair, x: wx.mint, y: NATIVE_MINT, bin_step };
    let before = env.lamports(&desk.pubkey());
    let m = env.send_ok("dlmm.init_pair", &[init.clone()], &[&desk]);
    let pair_rent = before - env.lamports(&desk.pubkey());
    let (sz, sg, ac) = nonce_tx(&env.payer.pubkey(), &[init]);
    log.push(format!("init pair: {} CU, {sz} B with nonce, {sg} signers, {ac} accounts; pair+reserves+oracle rent {:.6} SOL (desk pays)", m.compute_units_consumed - 150, pair_rent as f64 / 1e9));
    // bin arrays
    let idxs: Vec<i64> = (array_index(lo)..=array_index(hi)).collect();
    let before = env.lamports(&env.payer.pubkey());
    let ixs: Vec<Instruction> = idxs.iter().map(|i| ix_init_bin_array(&pair, *i, &env.payer.pubkey())).collect();
    let m = env.send_ok("dlmm.init_bin_arrays", &ixs, &[]);
    let ba_rent = before - env.lamports(&env.payer.pubkey()) - 5000;
    let (sz, sg, _) = nonce_tx(&env.payer.pubkey(), &ixs);
    log.push(format!(
        "init {} bin array(s) {:?}: {} CU, {sz} B, {sg} signer(s); rent {:.6} SOL each (anyone can pay; K_main here)",
        idxs.len(),
        idxs,
        m.compute_units_consumed - 150,
        ba_rent as f64 / 1e9 / idxs.len() as f64
    ));
    // position (PDA, base = owner = desk; K_main pays the rent)
    let width = (hi - lo + 1).min(70);
    let position = position_pda(&pair, &desk.pubkey(), lo, width);
    let mut ixs = vec![ix_init_position_pda(&env.payer.pubkey(), &desk.pubkey(), &pair, &desk.pubkey(), lo, width)];
    // increase_position_length2 grows a position by at most MAX_RESIZE_LENGTH (91) bins per call
    let mut upper = lo + width - 1;
    while upper < hi {
        upper = (upper + 91).min(hi);
        ixs.push(ix_increase_len2(&env.payer.pubkey(), &pair, &position, &desk.pubkey(), upper));
    }
    let before = env.lamports(&env.payer.pubkey());
    let m = env.send_ok("dlmm.init_position_pda", &ixs, &[&desk]);
    let pos_rent = before - env.lamports(&env.payer.pubkey()) - 10_000;
    let (sz, sg, _) = nonce_tx(&env.payer.pubkey(), &ixs);
    log.push(format!(
        "init position PDA [{lo}, {hi}] ({} bins{}): {} CU, {sz} B, {sg} signers (K_main, K_desk; no position keypair); rent {:.6} SOL",
        hi - lo + 1,
        if hi - lo + 1 > 70 { ", extended past 70" } else { "" },
        m.compute_units_consumed - 150,
        pos_rent as f64 / 1e9
    ));
    assert_eq!(position_range(env, &position), (lo, hi));
    // deposit
    let (x0, y0) = (env.token_amount(&dx), env.token_amount(&dy));
    let add = ix_add_strategy2(&lb, &position, &desk.pubkey(), &dx, &dy, wx_amount - 1_000_000, sol_side, active, lo, hi, shape);
    let m = env.send_ok("dlmm.add_strategy2", &[add.clone()], &[&desk]);
    let (sz, sg, ac) = nonce_tx(&env.payer.pubkey(), &[add]);
    log.push(format!(
        "add_liquidity_by_strategy2 ({shape:?}): {} CU, {sz} B, {sg} signers, {ac} accounts; deposited {:.4} SOL + {:.6} wX (= {:.4} SOL)",
        m.compute_units_consumed - 150,
        (y0 - env.token_amount(&dy)) as f64 / 1e9,
        (x0 - env.token_amount(&dx)) as f64 / 1e9,
        (x0 - env.token_amount(&dx)) as f64 / 1e9 * p_active
    ));
    Desk { lb, desk, wx, dx, dy, position, lo, hi }
}

struct Trader {
    k: Keypair,
    x: Pubkey,
    y: Pubkey,
}
fn trader(env: &mut Env, mint: &Pubkey) -> Trader {
    let k = env.funded(100);
    let y = env.wsol(&k, 40 * LAMPORTS);
    let x = env.ata(&k.pubkey(), mint);
    Trader { k, x, y }
}
/// Buys wX with `amt` lamports; returns (lamports per base unit paid, active-bin price move) or the refusal.
/// The bin arrays a swap walks, in its direction from the active bin's array (DLMM requires that order).
fn swap_path(env: &Env, lb: &Lb, up: bool) -> Vec<Pubkey> {
    let a = array_index(active_id(env, &lb.pair));
    let idx: Vec<i64> = if up { (a..=a + 3).collect() } else { (a - 3..=a).rev().collect() };
    idx.into_iter().map(|i| bin_array(&lb.pair, i)).filter(|k| env.exists(k)).collect()
}
fn buy(env: &mut Env, d: &Desk, t: &Trader, amt: u64, _arrays: &[Pubkey]) -> std::result::Result<(f64, f64, u64), String> {
    let arrays = &swap_path(env, &d.lb, true);
    let a0 = active_id(env, &d.lb.pair);
    let w0 = env.token_amount(&t.x);
    match env.send(&[ix_swap2(&d.lb, &t.k.pubkey(), &t.y, &t.x, amt, 0, arrays)], &[&t.k]) {
        Ok(m) => {
            let got = env.token_amount(&t.x) - w0;
            let a1 = active_id(env, &d.lb.pair);
            Ok((amt as f64 / got as f64, bin_price(d.lb.bin_step, a1) / bin_price(d.lb.bin_step, a0) - 1.0, m.compute_units_consumed - 150))
        }
        Err(e) => Err(first_err(&e.meta.logs.join("\n"))),
    }
}
fn sell_all(env: &mut Env, d: &Desk, t: &Trader, _arrays: &[Pubkey]) {
    let arrays = &swap_path(env, &d.lb, false);
    let amt = env.token_amount(&t.x);
    if amt > 0 {
        env.send_ok("sell back", &[ix_swap2(&d.lb, &t.k.pubkey(), &t.x, &t.y, amt, 0, arrays)], &[&t.k]);
    }
}
fn wide_arrays(d: &Desk) -> Vec<Pubkey> {
    arrays_for(&d.lb.pair, d.lo, d.hi)
}

// ================================================================================================= S4a: DLMM pair rules
#[test]
fn s4a_dlmm_pair_rules() {
    println!("\n==================== S4a: DLMM pair creation rules (mainnet bytes, mainnet presets) ====================");
    let mut env = dlmm_env();
    let wx = new_wx(&mut env, false);
    let stranger = env.funded(10);
    let active = id_for_price(50, FLOOR);

    // 1. anyone can create a pair on any existing preset (no wX needed)
    let ix = ix_init_pair2(&stranger.pubkey(), &preset(DESK_PRESET), &wx.mint, &NATIVE_MINT, active);
    let before = env.lamports(&stranger.pubkey());
    let m = env.send_ok("pair 262", &[ix], &[&stranger]);
    let pair = pair_with_preset(&preset(DESK_PRESET), &wx.mint, &NATIVE_MINT);
    println!(
        "  1. a stranger with no wX creates wX/SOL on preset 262 (bin step 50, 0.4 %, fees in Y): OK, {} CU, rent {:.6} SOL; pair_type {} active_id {}",
        m.compute_units_consumed - 150,
        (before - env.lamports(&stranger.pubkey())) as f64 / 1e9,
        pair_type(&env, &pair),
        active_id(&env, &pair)
    );
    // 2. one pair per (preset, pair of mints), either order
    let e = env.send_err("dup", &[ix_init_pair2(&env.payer.pubkey(), &preset(DESK_PRESET), &wx.mint, &NATIVE_MINT, active)], &[]);
    println!("  2. second wX/SOL on the same preset: refused ({})", first_err(&e));
    let e = env.send_err("dup rev", &[ix_init_pair2(&env.payer.pubkey(), &preset(DESK_PRESET), &NATIVE_MINT, &wx.mint, -active)], &[]);
    println!("     same preset, X/Y swapped: refused ({})", first_err(&e));
    // 3. another preset = another pair of the same mints
    for idx in [256u16, 286, 72, 83, 68] {
        let r = env.send(&[ix_init_pair2(&env.payer.pubkey(), &preset(idx), &wx.mint, &NATIVE_MINT, id_for_price(bin_step_of(idx), FLOOR))], &[]);
        println!("  3. wX/SOL on preset {idx}: {}", if r.is_ok() { "OK (another pair of the same mints)".to_string() } else { first_err(&r.err().unwrap().meta.logs.join("\n")) });
    }
    // 4. customizable permissionless pair: one per mint pair, creator picks bin step/fee/fee mode; does it need wX?
    let empty_x = env.ata(&stranger.pubkey(), &wx.mint);
    let sy = env.wsol(&stranger, LAMPORTS);
    let c = Custom { active, bin_step: 50, base_factor: 10_000, collect_fee_mode: 1 };
    let r = env.send(&[ix_init_custom2(&stranger.pubkey(), &wx.mint, &NATIVE_MINT, &empty_x, &sy, &c)], &[&stranger]);
    println!(
        "  4. customizable pair (bin step 50, base 0.5 %, fees in Y) by a stranger holding 0 wX: {}",
        match &r {
            Ok(m) => format!("OK, {} CU", m.compute_units_consumed - 150),
            Err(e) => format!("refused ({})", first_err(&e.meta.logs.join("\n"))),
        }
    );
    if r.is_err() {
        let sx = mint_wx(&mut env, &wx, &stranger.pubkey(), 1);
        let r2 = env.send(&[ix_init_custom2(&stranger.pubkey(), &wx.mint, &NATIVE_MINT, &sx, &sy, &c)], &[&stranger]);
        println!(
            "     … holding 1 base unit of wX: {}",
            match &r2 {
                Ok(m) => format!("OK, {} CU", m.compute_units_consumed - 150),
                Err(e) => format!("refused ({})", first_err(&e.meta.logs.join("\n"))),
            }
        );
    }
    let cp = pair_customizable(&wx.mint, &NATIVE_MINT);
    if env.exists(&cp) {
        let r3 = env.send(&[ix_init_custom2(&env.payer.pubkey(), &wx.mint, &NATIVE_MINT, &empty_x, &sy, &Custom { active: active + 5, bin_step: 25, base_factor: 10_000, collect_fee_mode: 1 })], &[]);
        println!("     a second customizable wX/SOL pair (other bin step): {}", if r3.is_ok() { "OK".into() } else { format!("refused ({})", first_err(&r3.err().unwrap().meta.logs.join("\n"))) });
    }
    // 5. a quote mint with a freeze authority
    let frozen = new_wx(&mut env, true);
    let r = env.send(&[ix_init_pair2(&env.payer.pubkey(), &preset(DESK_PRESET), &frozen.mint, &NATIVE_MINT, active)], &[]);
    println!("  5. a mint WITH a freeze authority on a preset pair: {}", if r.is_ok() { "accepted".into() } else { format!("refused ({})", first_err(&r.err().unwrap().meta.logs.join("\n"))) });

    // 6. squatting: the stranger's pair (1.) sits at the right price here; put one at a wrong price and move it back
    let wx2 = new_wx(&mut env, false);
    let wrong = id_for_price(50, FLOOR * 3.0);
    env.send_ok("squat", &[ix_init_pair2(&stranger.pubkey(), &preset(DESK_PRESET), &wx2.mint, &NATIVE_MINT, wrong)], &[&stranger]);
    let sp = pair_with_preset(&preset(DESK_PRESET), &wx2.mint, &NATIVE_MINT);
    let target = id_for_price(50, FLOOR);
    let r = env.send(&[ix_go_to_a_bin(&sp, None, None, target)], &[]);
    println!(
        "  6. a squatted EMPTY pair at 3× the floor (active {wrong}); go_to_a_bin({target}) by anyone, no bin arrays: {} → active {}",
        if r.is_ok() { "OK".to_string() } else { format!("refused ({})", first_err(&r.as_ref().err().unwrap().meta.logs.join("\n"))) },
        active_id(&env, &sp)
    );
    if r.is_err() {
        let ixs = vec![ix_init_bin_array(&sp, array_index(wrong), &env.payer.pubkey()), ix_init_bin_array(&sp, array_index(target), &env.payer.pubkey())];
        env.send_ok("arrays", &ixs, &[]);
        let r = env.send(&[ix_go_to_a_bin(&sp, Some(bin_array(&sp, array_index(wrong))), Some(bin_array(&sp, array_index(target))), target)], &[]);
        println!("     with both bin arrays: {} → active {}", if r.is_ok() { "OK".to_string() } else { format!("refused ({})", first_err(&r.err().unwrap().meta.logs.join("\n"))) }, active_id(&env, &sp));
    }
    // 7. squatted pair at a wrong price WITH a little liquidity: the desk can swap it to the floor (cost ≈ the squatter's
    //    liquidity's worth at the wrong price), then deposit. Shown in s4b via the arb path; not repeated here.
}
fn bin_step_of(idx: u16) -> u16 {
    match idx {
        262 | 256 | 83 => 50,
        286 => 30,
        72 => 25,
        68 => 10,
        _ => 50,
    }
}

// ================================================================================================= S4b: DLMM lifecycle
#[test]
fn s4b_dlmm_lifecycle() {
    println!("\n==================== S4b: DLMM desk lifecycle (preset 262, ±10 % spot, 10 SOL a side) ====================");
    let mut env = dlmm_env();
    let mut log = vec![];
    let d = open_desk(&mut env, 50, Some(DESK_PRESET), None, 10.0, 10 * LAMPORTS, Shape::Spot, &mut log);
    for l in &log {
        println!("  {l}");
    }
    let arrays = wide_arrays(&d);
    let t = trader(&mut env, &d.wx.mint);

    // swaps both ways
    let (p, mv, cu) = buy(&mut env, &d, &t, LAMPORTS / 2, &arrays).unwrap();
    let swap = ix_swap2(&d.lb, &t.k.pubkey(), &t.y, &t.x, LAMPORTS / 2, 0, &arrays);
    let (sz, sg, ac) = nonce_tx(&env.payer.pubkey(), &[swap]);
    println!("  swap2 buy 0.5 SOL: {cu} CU, {sz} B with nonce ({sg} signers, {ac} accounts, {} bin arrays); avg {:+.2} % vs active, pool {:+.2} %", arrays.len(), (p / bin_price(50, id_for_price(50, FLOOR)) - 1.0) * 100.0, mv * 100.0);
    sell_all(&mut env, &d, &t, &arrays);
    for _ in 0..5 {
        buy(&mut env, &d, &t, 2 * LAMPORTS, &arrays).unwrap();
        sell_all(&mut env, &d, &t, &arrays);
    }

    // claim fees (desk)
    let (bx, by) = (env.token_amount(&d.dx), env.token_amount(&d.dy));
    let claim = ix_claim_fee2(&d.lb, &d.position, &d.desk.pubkey(), &d.dx, &d.dy, d.lo, d.hi);
    let m = env.send_ok("claim", &[claim.clone()], &[&d.desk]);
    let (sz, sg, _) = nonce_tx(&env.payer.pubkey(), &[claim]);
    println!(
        "  claim_fee2 after 11 swaps (~21 SOL volume): {} CU, {sz} B, {sg} signers; got {} wX base units and {:.6} SOL",
        m.compute_units_consumed - 150,
        env.token_amount(&d.dx) - bx,
        (env.token_amount(&d.dy) - by) as f64 / 1e9
    );
    let e = env.send_err("stranger claim", &[ix_claim_fee2(&d.lb, &d.position, &t.k.pubkey(), &t.x, &t.y, d.lo, d.hi)], &[&t.k]);
    println!("  claim by a stranger: refused ({})", first_err(&e));

    // ---- re-centring. The arb loop moves the pool to the floor; the desk then re-centres its one position around it.
    let base_state = env.svm.clone();
    let half = d.hi - id_for_price(50, FLOOR);
    let mut a_now = 0;
    let (mut nlo, mut nhi) = (0, 0);
    for (case, mv) in [("R1 floor +7 % (inside the band)", 1.07f64), ("R2 floor +15 % (past the band's top)", 1.15)] {
        env.svm = base_state.clone();
        let target = id_for_price(50, FLOOR * mv);
        let arb = trader(&mut env, &d.wx.mint);
        let mut refused = None;
        while active_id(&env, &d.lb.pair) < target {
            if let Err(e) = buy(&mut env, &d, &arb, LAMPORTS / 4, &arrays) {
                refused = Some(e);
                break;
            }
        }
        let mut pre = vec![];
        let past = refused.is_some();
        if let Some(e) = refused {
            println!("  {case}: the arb buys the pool to its top bin {} (a buy past the last liquidity is refused: {})", active_id(&env, &d.lb.pair), e.split('.').next().unwrap_or(""));
            // empty the position first, then nothing lies between the top bin and the new floor: move the active bin there
            let from = bin_array(&d.lb.pair, array_index(active_id(&env, &d.lb.pair)));
            let to = bin_array(&d.lb.pair, array_index(target));
            pre.push(ix_remove_range2(&d.lb, &d.position, &d.desk.pubkey(), &d.dx, &d.dy, d.lo, d.hi, 10_000));
            pre.push(ix_go_to_a_bin(&d.lb.pair, Some(from), Some(to), target));
        }
        let a = if past { target } else { active_id(&env, &d.lb.pair) };
        let (lo2, hi2) = (a - half, a + half);
        for i in array_index(lo2)..=array_index(hi2) {
            if !env.exists(&bin_array(&d.lb.pair, i)) {
                let m = env.send_ok("new bin array", &[ix_init_bin_array(&d.lb.pair, i, &env.payer.pubkey())], &[]);
                println!("     new bin array {i} for the new range (K_main, {} CU, 0.0714 SOL)", m.compute_units_consumed - 150);
            }
        }
        let (bx, by) = (env.token_amount(&d.dx), env.token_amount(&d.dy));
        // the desk is the only LP: its position holds the reserves. Re-deposit (almost) all of it, spot, ±half bins.
        let (rx, ry) = (env.token_amount(&d.lb.rx()), env.token_amount(&d.lb.ry()));
        let p_new = bin_price(50, a);
        // spot: y0 lamports in each of the half+1 bid bins (a-half..a), x0 (Y value) in each of the half ask bins
        let total_value = rx as f64 * p_new + ry as f64;
        let per_bin = total_value * 0.98 / (2 * half + 1) as f64;
        let (x0, y0) = (per_bin as u64, per_bin as u64);
        let all_arrays = arrays_for(&d.lb.pair, d.lo.min(lo2), d.hi.max(hi2));
        // the ask side needs wX the desk no longer holds (the arb bought it out): mint it at the module in the same
        // transaction — SOL from the desk to the module's vault (stand-in key) + MintToChecked signed by K_coll
        let need_x = ((half as f64 * per_bin / p_new) * 1.01) as u64;
        let mint_x = need_x.saturating_sub(rx + env.token_amount(&d.dx));
        let module_vault = Pubkey::new_unique();
        let mut ixs = pre.clone();
        if mint_x > 0 {
            ixs.insert(0, system_instruction::transfer(&d.desk.pubkey(), &module_vault, (mint_x as f64 * p_new) as u64));
            ixs.insert(1, spl_token::instruction::mint_to_checked(&spl_token::ID, &d.wx.mint, &d.dx, &d.wx.canister.pubkey(), &[], mint_x, 9).unwrap());
        }
        let remove = if past { None } else { Some((d.lo, d.hi)) };
        ixs.push(ix_rebalance(&d.lb, &d.position, &d.desk.pubkey(), &env.payer.pubkey(), &d.dx, &d.dy, a, true, remove, &[RbAdd { min_delta: -half, max_delta: half, x0, y0 }], &all_arrays));
        let (sz, sg, ac) = nonce_tx(&env.payer.pubkey(), &ixs);
        match env.send(&ixs, &[&d.desk, &d.wx.canister]) {
            Ok(m) => {
                let (plo, phi) = position_range(&env, &d.position);
                println!(
                    "  {case}: {}{}rebalance_liquidity on the SAME position: {} CU, {sz} B with nonce, {sg} signers (K_main pays growth, K_desk owns{}), {ac} accounts ({} bin arrays); position [{}, {}] → [{plo}, {phi}], active {a}; desk Δ {:+.6} wX, {:+.4} SOL (wallet ↔ position)",
                    if mint_x > 0 { format!("SOL→module + mint {:.4} wX + ", mint_x as f64 / 1e9) } else { String::new() },
                    if pre.is_empty() { "" } else { "remove_liquidity_by_range2 + go_to_a_bin + " },
                    m.compute_units_consumed - 150,
                    if mint_x > 0 { ", K_coll mints" } else { "" },
                    all_arrays.len(),
                    d.lo,
                    d.hi,
                    (env.token_amount(&d.dx) as f64 - bx as f64) / 1e9,
                    (env.token_amount(&d.dy) as f64 - by as f64) / 1e9
                );
                let t2 = trader(&mut env, &d.wx.mint);
                let narr = arrays_for(&d.lb.pair, plo, phi);
                println!(
                    "     reserves after: {:.4} SOL + {:.6} wX (= {:.4} SOL); volatility accumulator {}",
                    env.token_amount(&d.lb.ry()) as f64 / 1e9,
                    env.token_amount(&d.lb.rx()) as f64 / 1e9,
                    env.token_amount(&d.lb.rx()) as f64 / 1e9 * p_new,
                    volatility_acc(&env, &d.lb.pair)
                );
                let (p, mvv, _) = buy(&mut env, &d, &t2, LAMPORTS / 2, &narr).unwrap();
                sell_all(&mut env, &d, &t2, &narr);
                env.warp(3600);
                let (p2, _, _) = buy(&mut env, &d, &t2, LAMPORTS / 2, &narr).unwrap();
                println!(
                    "     then a 0.5 SOL buy: avg {:+.2} % vs the new active bin right after the arb (variable fee still high), {:+.2} % an hour later; pool {:+.2} %",
                    (p / p_new - 1.0) * 100.0,
                    (p2 / p_new - 1.0) * 100.0,
                    mvv * 100.0
                );
            }
            Err(e) => println!("  {case}: rebalance refused: {}\n{}", first_err(&e.meta.logs.join("\n")), e.meta.logs.iter().rev().take(8).cloned().collect::<Vec<_>>().join("\n")),
        }
        a_now = a;
        nlo = lo2;
        nhi = hi2;
        if mv > 1.1 {
            // keep the R2 pre-rebalance state for (b)
            env.svm = base_state.clone();
            let arb = trader(&mut env, &d.wx.mint);
            while buy(&mut env, &d, &arb, LAMPORTS / 4, &arrays).is_ok() && active_id(&env, &d.lb.pair) < target {}
            for i in array_index(lo2)..=array_index(hi2) {
                if !env.exists(&bin_array(&d.lb.pair, i)) {
                    env.send_ok("new bin array", &[ix_init_bin_array(&d.lb.pair, i, &env.payer.pubkey())], &[]);
                }
            }
        }
    }

    // R3: R2's re-centre with plain instructions instead (on R2's state after go_to_a_bin): remove all + claim + close, a new
    // PDA position at the new range, add
    let (bx, by) = (env.token_amount(&d.dx), env.token_amount(&d.dy));
    let rm = ix_remove_range2(&d.lb, &d.position, &d.desk.pubkey(), &d.dx, &d.dy, d.lo, d.hi, 10_000);
    let cl = ix_claim_fee2(&d.lb, &d.position, &d.desk.pubkey(), &d.dx, &d.dy, d.lo, d.hi);
    let close = ix_close_position2(&d.position, &d.desk.pubkey(), &env.payer.pubkey());
    let (sz1, sg1, _) = nonce_tx(&env.payer.pubkey(), &[rm.clone(), cl.clone(), close.clone()]);
    let m1 = env.send_ok("withdraw", &[rm, cl, close], &[&d.desk]);
    let got_x = env.token_amount(&d.dx) - bx;
    let got_y = env.token_amount(&d.dy) - by;
    let width = nhi - nlo + 1;
    let npos = position_pda(&d.lb.pair, &d.desk.pubkey(), nlo, width);
    let p_new = bin_price(50, a_now);
    let mint_x = (got_y as f64 / 2.0 / p_new) as u64;
    let go = ix_go_to_a_bin(&d.lb.pair, Some(bin_array(&d.lb.pair, array_index(active_id(&env, &d.lb.pair)))), Some(bin_array(&d.lb.pair, array_index(a_now))), a_now);
    let pay = system_instruction::transfer(&d.desk.pubkey(), &Pubkey::new_unique(), got_y / 2);
    let mint = spl_token::instruction::mint_to_checked(&spl_token::ID, &d.wx.mint, &d.dx, &d.wx.canister.pubkey(), &[], mint_x, 9).unwrap();
    let init = ix_init_position_pda(&env.payer.pubkey(), &d.desk.pubkey(), &d.lb.pair, &d.desk.pubkey(), nlo, width);
    let add = ix_add_strategy2(&d.lb, &npos, &d.desk.pubkey(), &d.dx, &d.dy, mint_x, got_y / 2, a_now, nlo, nhi, Shape::Spot);
    let tx2 = [go, pay, mint, init, add];
    let (sz2, sg2, ac2) = nonce_tx(&env.payer.pubkey(), &tx2);
    let m2 = env.send_ok("reopen", &tx2, &[&d.desk, &d.wx.canister]);
    println!(
        "  R3 (R2 with plain instructions), two transactions: [remove_liquidity_by_range2 100 % + claim_fee2 + close_position2] {} CU, {sz1} B, {sg1} signers; then [go_to_a_bin + SOL→module + mint wX + initialize_position_pda + add_liquidity_by_strategy2] {} CU, {sz2} B, {sg2} signers, {ac2} accounts. Withdrew {:.6} wX + {:.6} SOL; active now {}",
        m1.compute_units_consumed - 150,
        m2.compute_units_consumed - 150,
        got_x as f64 / 1e9,
        got_y as f64 / 1e9,
        active_id(&env, &d.lb.pair)
    );
    // one transaction?
    let (sz3, sg3, ac3) = nonce_tx(
        &env.payer.pubkey(),
        &[
            ix_remove_range2(&d.lb, &d.position, &d.desk.pubkey(), &d.dx, &d.dy, d.lo, d.hi, 10_000),
            ix_claim_fee2(&d.lb, &d.position, &d.desk.pubkey(), &d.dx, &d.dy, d.lo, d.hi),
            ix_close_position2(&d.position, &d.desk.pubkey(), &env.payer.pubkey()),
            ix_init_position_pda(&env.payer.pubkey(), &d.desk.pubkey(), &d.lb.pair, &d.desk.pubkey(), nlo, width),
            ix_add_strategy2(&d.lb, &npos, &d.desk.pubkey(), &d.dx, &d.dy, mint_x, got_y / 2, a_now, nlo, nhi, Shape::Spot),
        ],
    );
    println!("     R3 as ONE transaction would be {sz3} B ({sg3} signers, {ac3} accounts) — {}", if sz3 <= 1232 { "fits" } else { "over 1,232 B" });

    // close everything: remove all, claim, close position → rent back
    let (lo2, hi2) = position_range(&env, &npos);
    let before = env.lamports(&env.payer.pubkey());
    env.send_ok(
        "final close",
        &[
            ix_remove_range2(&d.lb, &npos, &d.desk.pubkey(), &d.dx, &d.dy, lo2, hi2, 10_000),
            ix_claim_fee2(&d.lb, &npos, &d.desk.pubkey(), &d.dx, &d.dy, lo2, hi2),
            ix_close_position2(&npos, &d.desk.pubkey(), &env.payer.pubkey()),
        ],
        &[&d.desk],
    );
    println!(
        "  close: position rent returned to K_main: {:.6} SOL; reserves left: {} wX base units, {} lamports",
        (env.lamports(&env.payer.pubkey()) + 5000 * 2 - before) as f64 / 1e9,
        env.token_amount(&d.lb.rx()),
        env.token_amount(&d.lb.ry())
    );
    // bin arrays: can the funder close an empty one?
    let ba = bin_array(&d.lb.pair, array_index(d.lo));
    let r = env.send(&[ix_close_bin_array(&d.lb.pair, &ba, &env.payer.pubkey(), &env.payer.pubkey())], &[]);
    println!("  close_bin_array (empty, by its funder K_main): {}", if r.is_ok() { "OK — rent back".to_string() } else { format!("refused ({})", first_err(&r.err().unwrap().meta.logs.join("\n"))) });

    // ---- the arb transaction the canister sends: mint wX (K_coll) to the desk + sell into the pool (K_desk), with nonce
    let mint_ix = spl_token::instruction::mint_to_checked(&spl_token::ID, &d.wx.mint, &d.dx, &d.wx.canister.pubkey(), &[], LAMPORTS / 100, 9).unwrap();
    let sell = ix_swap2(&d.lb, &d.desk.pubkey(), &d.dx, &d.dy, LAMPORTS / 100, 1, &arrays_for(&d.lb.pair, d.lo, nhi));
    let (sz, sg, ac) = nonce_tx(&env.payer.pubkey(), &[mint_ix, sell]);
    println!("  arb up (mint + swap2 sell, one tx): {sz} B with nonce, {sg} signers, {ac} accounts");
}

// ================================================================================================= S4c: DLMM depth
#[test]
fn s4c_dlmm_depth() {
    println!("\n==================== S4c: DLMM wX/SOL depth (floor 18.61; each buy sold back before the next) ====================");
    println!("  {:<30} {:>6} {:>8} {:>18} {:>18} {:>18} | {:>14} {:>16}", "pool", "fee", "SOL/side", "0.1 SOL avg/move", "0.5 SOL avg/move", "1 SOL avg/move", "$500 rt loss", "$1k/$500 unit");
    let cases: Vec<(String, u16, Option<u16>, Option<u16>, f64, u64, Shape, &str)> = vec![
        ("preset 262 bs50 ±10 % spot".into(), 50, Some(DESK_PRESET), None, 10.0, 10, Shape::Spot, "0.4 %"),
        ("preset 262 bs50 ±10 % curve".into(), 50, Some(DESK_PRESET), None, 10.0, 10, Shape::Curve, "0.4 %"),
        ("preset 262 bs50 ±10 % bid-ask".into(), 50, Some(DESK_PRESET), None, 10.0, 10, Shape::BidAsk, "0.4 %"),
        ("preset 262 bs50 ±21 % spot".into(), 50, Some(DESK_PRESET), None, 21.0, 10, Shape::Spot, "0.4 %"),
        ("preset 262 bs50 ±10 % spot".into(), 50, Some(DESK_PRESET), None, 10.0, 5, Shape::Spot, "0.4 %"),
        ("preset 262 bs50 ±10 % spot".into(), 50, Some(DESK_PRESET), None, 10.0, 1, Shape::Spot, "0.4 %"),
        ("preset 286 bs30 ±10 % spot".into(), 30, Some(286), None, 10.0, 10, Shape::Spot, "0.3 %"),
        ("custom bs50 ±10 % spot".into(), 50, None, Some(10_000), 10.0, 10, Shape::Spot, "0.5 %"),
        ("custom bs10 ±10 % spot".into(), 10, None, Some(50_000), 10.0, 10, Shape::Spot, "0.5 %"),
        ("custom bs10 ±10 % curve".into(), 10, None, Some(50_000), 10.0, 10, Shape::Curve, "0.5 %"),
    ];
    let mut extra = vec![];
    for (label, bs, pi, cf, pct, sol, shape, fee) in cases {
        let mut env = dlmm_env();
        let mut log = vec![];
        let d = open_desk(&mut env, bs, pi, cf, pct, sol * LAMPORTS, shape, &mut log);
        let arrays = arrays_for(&d.lb.pair, d.lo - 70, d.hi + 70).into_iter().filter(|a| env.exists(a)).collect::<Vec<_>>();
        let p0 = FLOOR;
        let t = trader(&mut env, &d.wx.mint);
        let mut cells = vec![];
        for amt in [LAMPORTS / 10, LAMPORTS / 2, LAMPORTS] {
            // reset the volatility accumulator between buys (as after a quiet period)
            env.warp(3600);
            match buy(&mut env, &d, &t, amt, &arrays) {
                Ok((p, mv, _)) => cells.push(format!("{:+.2}% / {:+.2}%", (p / p0 - 1.0) * 100.0, mv * 100.0)),
                Err(e) => cells.push(format!("refused: {}", e.chars().take(10).collect::<String>())),
            }
            sell_all(&mut env, &d, &t, &arrays);
        }
        // Jupiter: $500 ≈ 4.5 SOL round trip; $1k ≈ 9 SOL vs $500 unit price
        let mut j = vec![];
        for amt in [4_500_000_000u64, 9_000_000_000] {
            env.warp(3600);
            let y0 = env.token_amount(&t.y);
            match buy(&mut env, &d, &t, amt, &arrays) {
                Ok((p, _, _)) => {
                    sell_all(&mut env, &d, &t, &arrays);
                    let back = env.token_amount(&t.y) as i128 - (y0 as i128 - amt as i128);
                    j.push(Ok((p, 1.0 - back as f64 / amt as f64)));
                }
                Err(e) => {
                    j.push(Err(e));
                }
            }
        }
        let t1 = match &j[0] {
            Ok((_, l)) => format!("{:.1} % {}", l * 100.0, if *l < 0.3 { "PASS" } else { "FAIL" }),
            Err(_) => "refused FAIL".into(),
        };
        let t2 = match (&j[0], &j[1]) {
            (Ok((a, _)), Ok((b, _))) => format!("{:+.1} % {}", (b / a - 1.0) * 100.0, if b / a - 1.0 < 0.2 { "PASS" } else { "FAIL" }),
            _ => "refused FAIL".into(),
        };
        if label.contains("bs10 ±10 % spot") || label.contains("bs30") || label.contains("±21") {
            extra.push(format!("{label}:\n      {}", log.join("\n      ")));
        }
        println!("  {:<30} {:>6} {:>8} {:>18} {:>18} {:>18} | {:>14} {:>16}", label, fee, sol, cells[0], cells[1], cells[2], t1, t2);
    }
    println!("  (avg = buyer's average price vs the floor 18.61, all fees included; move = the active bin's price after the buy)");
    for e in extra {
        println!("  {e}");
    }
}

// ================================================================================================= S4d: DAMM v2 at 10 SOL a side
#[derive(Clone, Copy)]
struct Damm {
    pool: Pubkey,
    a: Pubkey,
    b: Pubkey,
}
fn ix_damm_swap(p: &Damm, trader: &Pubkey, input: Pubkey, output: Pubkey, amount_in: u64, min_out: u64) -> Instruction {
    Instruction {
        program_id: DAMM_V2_ID,
        accounts: cp_amm::client::accounts::Swap {
            pool_authority: DAMM_V2_POOL_AUTHORITY,
            pool: p.pool,
            input_token_account: input,
            output_token_account: output,
            token_a_vault: damm_vault(&p.a, &p.pool),
            token_b_vault: damm_vault(&p.b, &p.pool),
            token_a_mint: p.a,
            token_b_mint: p.b,
            payer: *trader,
            token_a_program: spl_token::ID,
            token_b_program: spl_token::ID,
            referral_token_account: None,
            event_authority: event_authority(&DAMM_V2_ID),
            program: DAMM_V2_ID,
        }
        .to_account_metas(None),
        data: cp_amm::client::args::Swap { _params: cp_amm::types::SwapParameters { amount_in, minimum_amount_out: min_out } }.data(),
    }
}
fn liquidity_for(a: u64, b: u64, s: u128, smin: u128, smax: u128) -> u128 {
    let sp = s as f64 / TWO64;
    let lo = smin as f64 / TWO64;
    let hi = smax as f64 / TWO64;
    let la = a as f64 / (1.0 / sp - 1.0 / hi);
    let lb = b as f64 / (sp - lo);
    (la.min(lb) * 0.999_999 * TWO64) as u128
}

#[test]
fn s4d_damm_v2_ranges_at_10_sol() {
    println!("\n==================== S4d: DAMM v2 wX/SOL at 10 SOL a side (fee 0.5 %, fees in SOL) ====================");
    println!("  {:<16} {:>8} {:>18} {:>18} {:>18} | {:>14} {:>16}", "range", "SOL/side", "0.1 SOL avg/move", "0.5 SOL avg/move", "1 SOL avg/move", "$500 rt loss", "$1k/$500 unit");
    for (label, k_sqrt, sol) in [("[p/4, 4p]", 2.0f64, 10u64), ("[p/4, 4p]", 2.0, 20), ("[p/1.21, 1.21p]", 1.1, 10), ("[p/1.1, 1.1p]", 1.048_808_848, 10)] {
        let mut env = Env::new();
        let wx = new_wx(&mut env, false);
        let desk = env.funded(100);
        let floor = FLOOR * 1e9;
        let s0 = ((floor / 1e9).sqrt() * TWO64) as u128;
        let (smin, smax) = ((s0 as f64 / k_sqrt) as u128, (s0 as f64 * k_sqrt) as u128);
        let seed = sol * LAMPORTS;
        let seed_wx = (seed as f64 / floor * 1e9) as u64;
        let dx = mint_wx(&mut env, &wx, &desk.pubkey(), seed_wx);
        let dy = env.wsol(&desk, seed);
        let nft = Keypair::new();
        let (hi, lo) = if wx.mint > NATIVE_MINT { (wx.mint, NATIVE_MINT) } else { (NATIVE_MINT, wx.mint) };
        let pool = pda(&[b"cpool", hi.as_ref(), lo.as_ref()], &DAMM_V2_ID);
        let p = Damm { pool, a: wx.mint, b: NATIVE_MINT };
        let mut bf = [0u8; 27];
        bf[..8].copy_from_slice(&5_000_000u64.to_le_bytes());
        let params = cp_amm::types::InitializeCustomizablePoolParameters {
            pool_fees: cp_amm::types::PoolFeeParameters { base_fee: cp_amm::types::BaseFeeParameters { data: bf }, compounding_fee_bps: 0, padding: 0, dynamic_fee: None },
            sqrt_min_price: smin,
            sqrt_max_price: smax,
            has_alpha_vault: false,
            liquidity: liquidity_for(seed_wx, seed, s0, smin, smax),
            sqrt_price: s0,
            activation_type: 1,
            collect_fee_mode: 1,
            activation_point: None,
        };
        let init = Instruction {
            program_id: DAMM_V2_ID,
            accounts: cp_amm::client::accounts::InitializeCustomizablePool {
                creator: desk.pubkey(),
                position_nft_mint: nft.pubkey(),
                position_nft_account: damm_position_nft_account(&nft.pubkey()),
                payer: desk.pubkey(),
                pool_authority: DAMM_V2_POOL_AUTHORITY,
                pool,
                position: damm_position(&nft.pubkey()),
                token_a_mint: wx.mint,
                token_b_mint: NATIVE_MINT,
                token_a_vault: damm_vault(&wx.mint, &pool),
                token_b_vault: damm_vault(&NATIVE_MINT, &pool),
                payer_token_a: dx,
                payer_token_b: dy,
                token_a_program: spl_token::ID,
                token_b_program: spl_token::ID,
                token_2022_program: TOKEN_2022_ID,
                system_program: anchor_lang::system_program::ID,
                event_authority: event_authority(&DAMM_V2_ID),
                program: DAMM_V2_ID,
            }
            .to_account_metas(None),
            data: cp_amm::client::args::InitializeCustomizablePool { params }.data(),
        };
        env.send_ok("init", &[init], &[&desk, &nft]);
        let d = env.raw(&pool);
        let b = s0.to_le_bytes();
        let off = *(8..d.len() - 16).filter(|&o| d[o..o + 16] == b).collect::<Vec<_>>().last().unwrap();
        let sq = |env: &Env| u128::from_le_bytes(env.raw(&pool)[off..off + 16].try_into().unwrap());
        let pr = |s: u128| {
            let r = s as f64 / TWO64;
            r * r
        };
        let t = env.funded(100);
        let ty = env.wsol(&t, 40 * LAMPORTS);
        let tx = env.ata(&t.pubkey(), &wx.mint);
        let mut cells = vec![];
        for amt in [LAMPORTS / 10, LAMPORTS / 2, LAMPORTS] {
            let s_before = sq(&env);
            let w0 = env.token_amount(&tx);
            match env.send(&[ix_damm_swap(&p, &t.pubkey(), ty, tx, amt, 0)], &[&t]) {
                Ok(_) => {
                    let got = env.token_amount(&tx) - w0;
                    cells.push(format!("{:+.2}% / {:+.2}%", (amt as f64 / got as f64 / FLOOR - 1.0) * 100.0, (pr(sq(&env)) / pr(s_before) - 1.0) * 100.0));
                    env.send_ok("sell back", &[ix_damm_swap(&p, &t.pubkey(), tx, ty, got, 0)], &[&t]);
                }
                Err(_) => cells.push("refused".into()),
            }
        }
        let mut j = vec![];
        for amt in [4_500_000_000u64, 9_000_000_000] {
            let y0 = env.token_amount(&ty);
            match env.send(&[ix_damm_swap(&p, &t.pubkey(), ty, tx, amt, 0)], &[&t]) {
                Ok(_) => {
                    let got = env.token_amount(&tx);
                    env.send_ok("sell", &[ix_damm_swap(&p, &t.pubkey(), tx, ty, got, 0)], &[&t]);
                    let back = env.token_amount(&ty) as i128 - (y0 as i128 - amt as i128);
                    j.push(Ok((amt as f64 / got as f64, 1.0 - back as f64 / amt as f64)));
                }
                Err(_) => j.push(Err(())),
            }
        }
        let t1 = match &j[0] {
            Ok((_, l)) => format!("{:.1} % {}", l * 100.0, if *l < 0.3 { "PASS" } else { "FAIL" }),
            Err(_) => "refused FAIL".into(),
        };
        let t2 = match (&j[0], &j[1]) {
            (Ok((a, _)), Ok((b, _))) => format!("{:+.1} % {}", (b / a - 1.0) * 100.0, if b / a - 1.0 < 0.2 { "PASS" } else { "FAIL" }),
            _ => "refused FAIL".into(),
        };
        println!("  {:<16} {:>8} {:>18} {:>18} {:>18} | {:>14} {:>16}", label, sol, cells[0], cells[1], cells[2], t1, t2);
    }
    println!("  (avg = buyer's average price vs the floor, fee included; move = the pool's own price after the buy)");
}

// ================================================================================================= S4e: the site's two-hop with a DLMM hop
/// Size only (legacy, user signs, CU limit + price, no nonce): the S2 bundle of S1 with hop 1 on DAMM v2 (calibration:
/// S2 measured 1,033 B / 903 B) and on DLMM swap2 with 1 or 2 bin arrays. The DBC and COIN/wX accounts are stand-in keys.
#[test]
fn s4e_two_hop_sizes() {
    use anchor_spl::associated_token::spl_associated_token_account::instruction as ata_ix;
    use rook_backing::dynamic_bonding_curve::{self as dbc, types as dbct};
    println!("\n==================== S4e: the site's SOL → wX → coin transaction with a DLMM first hop (sizes) ====================");
    let u = Pubkey::new_unique();
    let wx = Pubkey::new_unique();
    let coin = Pubkey::new_unique();
    let (u_wsol, u_wx, u_coin) = (get_associated_token_address(&u, &NATIVE_MINT), get_associated_token_address(&u, &wx), get_associated_token_address(&u, &coin));
    let dbc_ix = Instruction {
        program_id: DBC_ID,
        accounts: dbc::client::accounts::Swap2 {
            pool_authority: DBC_POOL_AUTHORITY,
            config: Pubkey::new_unique(),
            pool: Pubkey::new_unique(),
            input_token_account: u_wx,
            output_token_account: u_coin,
            base_vault: Pubkey::new_unique(),
            quote_vault: Pubkey::new_unique(),
            base_mint: coin,
            quote_mint: wx,
            payer: u,
            token_base_program: spl_token::ID,
            token_quote_program: spl_token::ID,
            referral_token_account: None,
            event_authority: event_authority(&DBC_ID),
            program: DBC_ID,
        }
        .to_account_metas(None),
        data: dbc::client::args::Swap2 { params: dbct::SwapParameters2 { amount_0: 1, amount_1: 1, swap_mode: 0 } }.data(),
    };
    let coin_pool = Damm { pool: Pubkey::new_unique(), a: coin, b: wx };
    let grad_ix = ix_damm_swap(&coin_pool, &u, u_wx, u_coin, 1, 1);
    let damm_hop = ix_damm_swap(&Damm { pool: Pubkey::new_unique(), a: wx, b: NATIVE_MINT }, &u, u_wsol, u_wx, 1, 1);
    let lb = Lb { pair: Pubkey::new_unique(), x: wx, y: NATIVE_MINT, bin_step: 50 };
    let dlmm1 = ix_swap2(&lb, &u, &u_wsol, &u_wx, 1, 1, &[Pubkey::new_unique()]);
    let dlmm2 = ix_swap2(&lb, &u, &u_wsol, &u_wx, 1, 1, &[Pubkey::new_unique(), Pubkey::new_unique()]);
    let bundle = |hop1: &Instruction, hop2: &Instruction| {
        let ixs = vec![
            ComputeBudgetInstruction::set_compute_unit_limit(400_000),
            ComputeBudgetInstruction::set_compute_unit_price(1),
            ata_ix::create_associated_token_account_idempotent(&u, &u, &NATIVE_MINT, &spl_token::ID),
            system_instruction::transfer(&u, &u_wsol, 1),
            spl_token::instruction::sync_native(&spl_token::ID, &u_wsol).unwrap(),
            ata_ix::create_associated_token_account_idempotent(&u, &u, &wx, &spl_token::ID),
            hop1.clone(),
            ata_ix::create_associated_token_account_idempotent(&u, &u, &coin, &spl_token::ID),
            hop2.clone(),
            spl_token::instruction::close_account(&spl_token::ID, &u_wsol, &u, &u, &[]).unwrap(),
        ];
        let m = Message::new(&ixs, Some(&u));
        (1 + 64 * m.header.num_required_signatures as usize + m.serialize().len(), m.account_keys.len())
    };
    for (label, h1) in [("DAMM v2 hop 1 (S2 calibration)", &damm_hop), ("DLMM swap2, 1 bin array", &dlmm1), ("DLMM swap2, 2 bin arrays", &dlmm2)] {
        let (a, ka) = bundle(h1, &dbc_ix);
        let (b, kb) = bundle(h1, &grad_ix);
        println!("  {label:<32}: on the curve (DBC swap2) {a} B / {ka} accounts; graduated (DAMM v2 COIN/wX) {b} B / {kb} accounts");
    }
}
