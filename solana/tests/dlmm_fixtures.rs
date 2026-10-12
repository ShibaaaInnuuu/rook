//! W7 fixtures for the canister's Meteora DLMM builders (`autobuyer/canister-rs/src/solix2/dlmm.rs`, docs/v9/canister-only-solana.md
//! §17, docs/v9/spikes/s4-wx-pools.md §5.2). The spike S4 desk lifecycle (`tests/s4_wx_pools.rs`) in LiteSVM against the mainnet bytes
//! of Meteora DLMM (`LBUZ…`, IDL 0.12.0) and its mainnet preset 262, with **deterministic** keys, so every address is reproducible.
//! Every instruction is encoded the spike's way (Anchor discriminator + Borsh, by hand from the IDL; DLMM's IDL is not in the repo, so
//! there is no generated client), **sent, and recorded only once it has landed**. Accounts the canister must decode (`LbPair`,
//! `PositionV2`, `BinArray`) are dumped with the numbers they must decode to (the amounts the chain actually paid out of the position,
//! the fees it actually paid).
//!
//! Output: autobuyer/canister-rs/tests/fixtures/solix2/dlmm.json (checked by autobuyer/canister-rs/tests/solix2_dlmm.rs).
//! Run: `cargo test -p rook_backing --test dlmm_fixtures -- --nocapture`

#[path = "harness/mod.rs"]
mod harness;

use anchor_lang::{
    prelude::*,
    solana_program::{instruction::Instruction, system_instruction},
};
use anchor_spl::token::spl_token;
use base64::Engine;
use harness::*;
use rook_backing::constants::*;
use serde_json::{json, Value};
use solana_account::Account as SolAccount;
use solana_compute_budget_interface::ComputeBudgetInstruction;
use solana_keypair::Keypair;
use solana_message::Message;
use solana_signer::Signer;

const DLMM_ID: Pubkey = pubkey!("LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo");
const MEMO_ID: Pubkey = pubkey!("MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr");
const RENT_SYSVAR: Pubkey = pubkey!("SysvarRent111111111111111111111111111111111");
const FLOOR: f64 = 18.61;
const BIN_STEP: u16 = 50;
const DESK_PRESET: u16 = 262;

const D_INIT_PAIR2: [u8; 8] = hex8("493b2478ed536cc6");
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

/// `SpotImBalanced` (the IDL's strategy enum order).
const SPOT: u8 = 6;

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

fn ordered(a: &Pubkey, b: &Pubkey) -> (Pubkey, Pubkey) {
    if a < b {
        (*a, *b)
    } else {
        (*b, *a)
    }
}
fn preset(idx: u16) -> Pubkey {
    pda(&[b"preset_parameter2", &idx.to_le_bytes()], &DLMM_ID)
}
fn pair_with_preset(preset: &Pubkey, x: &Pubkey, y: &Pubkey) -> Pubkey {
    let (lo, hi) = ordered(x, y);
    pda(&[preset.as_ref(), lo.as_ref(), hi.as_ref()], &DLMM_ID)
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
fn bin_price(id: i32) -> f64 {
    (1.0 + BIN_STEP as f64 / 10_000.0).powi(id)
}
fn id_for_price(p: f64) -> i32 {
    (p.ln() / (1.0 + BIN_STEP as f64 / 10_000.0).ln()).round() as i32
}
fn half_width(pct: f64) -> i32 {
    ((1.0 + pct / 100.0).ln() / (1.0 + BIN_STEP as f64 / 10_000.0).ln()).round() as i32
}

#[derive(Clone, Copy)]
struct Lb {
    pair: Pubkey,
    x: Pubkey,
    y: Pubkey,
}
impl Lb {
    fn rx(&self) -> Pubkey {
        reserve(&self.pair, &self.x)
    }
    fn ry(&self) -> Pubkey {
        reserve(&self.pair, &self.y)
    }
}
fn active_id(env: &Env, pair: &Pubkey) -> i32 {
    i32::from_le_bytes(env.raw(pair)[76..80].try_into().unwrap())
}
fn position_range(env: &Env, position: &Pubkey) -> (i32, i32) {
    let d = env.raw(position);
    (i32::from_le_bytes(d[7912..7916].try_into().unwrap()), i32::from_le_bytes(d[7916..7920].try_into().unwrap()))
}

// ---- the instructions, as the spike encoded them (every one of these landed in s4_wx_pools.rs) ------------------------------------------
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
fn ix_init_bin_array(pair: &Pubkey, index: i64, funder: &Pubkey) -> Instruction {
    Instruction {
        program_id: DLMM_ID,
        accounts: vec![r(*pair), w(bin_array(pair, index)), ws(*funder), r(anchor_lang::system_program::ID)],
        data: W::new(D_INIT_BIN_ARRAY).i64(index).done(),
    }
}
fn ix_init_position_pda(payer: &Pubkey, base: &Pubkey, pair: &Pubkey, owner: &Pubkey, lower: i32, width: i32) -> Instruction {
    Instruction {
        program_id: DLMM_ID,
        accounts: vec![
            ws(*payer),
            rs(*base),
            w(position_pda(pair, base, lower, width)),
            r(*pair),
            rs(*owner),
            r(anchor_lang::system_program::ID),
            r(RENT_SYSVAR),
            ev(),
            r(DLMM_ID),
        ],
        data: W::new(D_INIT_POSITION_PDA).i32(lower).i32(width).done(),
    }
}
fn ix_increase_len2(funder: &Pubkey, pair: &Pubkey, position: &Pubkey, owner: &Pubkey, min_upper: i32) -> Instruction {
    Instruction {
        program_id: DLMM_ID,
        accounts: vec![ws(*funder), r(*pair), w(*position), rs(*owner), r(anchor_lang::system_program::ID), ev(), r(DLMM_ID)],
        data: W::new(D_INCREASE_LEN2).i32(min_upper).done(),
    }
}
#[allow(clippy::too_many_arguments)]
fn ix_add_strategy2(lb: &Lb, position: &Pubkey, sender: &Pubkey, ux: &Pubkey, uy: &Pubkey, ax: u64, ay: u64, active: i32, lo: i32, hi: i32, shape: u8) -> Instruction {
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
        data: W::new(D_ADD_STRATEGY2).u64(ax).u64(ay).i32(active).i32(3).i32(lo).i32(hi).u8(shape).zeros(64).no_slices().done(),
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
#[allow(clippy::too_many_arguments)]
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
        .u16(3)
        .u8(claim_fee as u8)
        .u8(0)
        .u64(0)
        .u64(u64::MAX)
        .u64(0)
        .u64(u64::MAX)
        .u8(0)
        .zeros(31);
    if let Some((lo, hi)) = remove {
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

// ---- recording -----------------------------------------------------------------------------------------------------------------------
fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}
fn b64(b: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(b)
}
fn s(k: &Pubkey) -> String {
    k.to_string()
}
fn ss(v: &[Pubkey]) -> Vec<String> {
    v.iter().map(s).collect()
}
fn kp(n: u8) -> Keypair {
    Keypair::new_from_array([n; 32])
}
fn jix(name: &str, ix: &Instruction, params: Value) -> Value {
    json!({
        "name": name,
        "program": ix.program_id.to_string(),
        "accounts": ix.accounts.iter().map(|a| json!([a.pubkey.to_string(), a.is_signer, a.is_writable])).collect::<Vec<_>>(),
        "data": hex(&ix.data),
        "params": params,
    })
}
struct Rec {
    ixs: Vec<Value>,
    bundles: Vec<Value>,
    accounts: Vec<Value>,
}
impl Rec {
    fn ix(&mut self, name: &str, ix: &Instruction, params: Value) {
        self.ixs.push(jix(name, ix, params));
    }
    fn account(&mut self, name: &str, env: &Env, key: &Pubkey, expect: Value) {
        let a = env.svm.get_account(key).unwrap();
        self.accounts.push(json!({"name": name, "key": key.to_string(), "owner": a.owner.to_string(), "data": b64(&a.data), "expect": expect}));
    }
    /// A whole transaction that landed: its instructions (after the nonce advance and compute budget), the keys that signed, its
    /// wire size behind a durable-nonce advance + CU limit + CU price (the canister's send line).
    fn bundle(&mut self, name: &str, ixs: &[Instruction], signers: &[Pubkey], payer: &Pubkey) {
        let (size, n, accounts) = nonce_tx(payer, ixs);
        self.bundles.push(json!({
            "name": name,
            "ixs": ixs.iter().map(|i| jix("", i, Value::Null)).collect::<Vec<_>>(),
            "signers": ss(signers),
            "size_with_nonce": size, "signers_with_payer": n, "accounts": accounts,
        }));
    }
}
fn nonce_tx(payer: &Pubkey, ixs: &[Instruction]) -> (usize, usize, usize) {
    let nonce = Pubkey::new_from_array([0x77; 32]);
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

fn dlmm_env() -> Env {
    let mut env = Env::new();
    let f = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures");
    env.svm.add_program_from_file(DLMM_ID, f.join("dlmm.so")).unwrap();
    let j: Value = serde_json::from_str(&std::fs::read_to_string(f.join("s4_dlmm_presets.json")).unwrap()).unwrap();
    for (_, v) in j.as_object().unwrap() {
        let k: Pubkey = v["pubkey"].as_str().unwrap().parse().unwrap();
        let data = base64::engine::general_purpose::STANDARD.decode(v["data"].as_str().unwrap()).unwrap();
        env.svm.set_account(k, SolAccount { lamports: v["lamports"].as_u64().unwrap(), data, owner: DLMM_ID, executable: false, rent_epoch: 0 }).unwrap();
    }
    env.payer = kp(1);
    env.svm.airdrop(&env.payer.pubkey(), 10_000 * LAMPORTS).unwrap();
    env
}

fn new_wx(env: &mut Env, mint_kp: &Keypair, authority: &Keypair) {
    let rent = env.svm.minimum_balance_for_rent_exemption(82);
    let ck = authority.pubkey();
    let ixs = [
        system_instruction::create_account(&env.payer.pubkey(), &mint_kp.pubkey(), rent, 82, &spl_token::ID),
        spl_token::instruction::initialize_mint2(&spl_token::ID, &mint_kp.pubkey(), &ck, None, 9).unwrap(),
    ];
    env.send_ok("create wX", &ixs, &[mint_kp]);
}
fn mint_wx(env: &mut Env, mint: &Pubkey, authority: &Keypair, owner: &Pubkey, amount: u64) -> Pubkey {
    let ata = env.ata(owner, mint);
    let ix = spl_token::instruction::mint_to_checked(&spl_token::ID, mint, &ata, &authority.pubkey(), &[], amount, 9).unwrap();
    env.send_ok("mint wX", &[ix], &[authority]);
    ata
}
/// The bin arrays a swap walks, in its direction from the active bin's array (the spike: a swap given them the other way is refused).
fn swap_path(env: &Env, lb: &Lb, up: bool) -> Vec<Pubkey> {
    let a = array_index(active_id(env, &lb.pair));
    let idx: Vec<i64> = if up { (a..=a + 3).collect() } else { (a - 3..=a).rev().collect() };
    idx.into_iter().map(|i| bin_array(&lb.pair, i)).filter(|k| env.exists(k)).collect()
}

struct Trader {
    k: Keypair,
    x: Pubkey,
    y: Pubkey,
}
fn trader(env: &mut Env, mint: &Pubkey, n: u8) -> Trader {
    let k = kp(n);
    env.svm.airdrop(&k.pubkey(), 100 * LAMPORTS).unwrap();
    let y = env.wsol(&k, 40 * LAMPORTS);
    let x = env.ata(&k.pubkey(), mint);
    Trader { k, x, y }
}

fn lb_expect(env: &Env, lb: &Lb) -> Value {
    let d = env.raw(&lb.pair);
    json!({
        "active_id": i32::from_le_bytes(d[76..80].try_into().unwrap()),
        "bin_step": u16::from_le_bytes(d[80..82].try_into().unwrap()),
        "status": d[82],
        "pair_type": d[75],
        "token_x_mint": s(&lb.x), "token_y_mint": s(&lb.y),
        "volatility_accumulator": u32::from_le_bytes(d[40..44].try_into().unwrap()),
        "reserve_x": s(&lb.rx()), "reserve_y": s(&lb.ry()),
        "len": d.len(),
    })
}

#[test]
fn dlmm_fixtures() {
    let mut env = dlmm_env();
    let payer = env.payer.pubkey();
    let canister = kp(4); // K_coll[i]
    env.svm.airdrop(&canister.pubkey(), LAMPORTS).unwrap();
    let wx_kp = kp(3);
    new_wx(&mut env, &wx_kp, &canister);
    let wx = wx_kp.pubkey();
    let desk = kp(13); // K_desk[i]
    env.svm.airdrop(&desk.pubkey(), 1_000 * LAMPORTS).unwrap();
    let dk = desk.pubkey();
    let mut rec = Rec { ixs: vec![], bundles: vec![], accounts: vec![] };

    // ---- 1. the pair, K_main funds it (any key can)
    let active0 = id_for_price(FLOOR);
    let half = half_width(10.0);
    let (lo, hi) = (active0 - half, active0 + half);
    let init = ix_init_pair2(&payer, &preset(DESK_PRESET), &wx, &NATIVE_MINT, active0);
    env.send_ok("init pair", &[init.clone()], &[]);
    let pair = pair_with_preset(&preset(DESK_PRESET), &wx, &NATIVE_MINT);
    let lb = Lb { pair, x: wx, y: NATIVE_MINT };
    rec.ix("dlmm.initialize_lb_pair2", &init, json!({"funder": s(&payer), "preset": s(&preset(DESK_PRESET)), "x": s(&wx), "y": s(&NATIVE_MINT), "active_id": active0}));
    rec.account("lb_pair_open", &env, &pair, lb_expect(&env, &lb));

    // ---- 2. the bin arrays of the band, one position owned by the desk (PDA, base = owner = desk)
    for i in array_index(lo)..=array_index(hi) {
        let ix = ix_init_bin_array(&pair, i, &payer);
        env.send_ok("bin array", &[ix.clone()], &[]);
        rec.ix("dlmm.initialize_bin_array", &ix, json!({"pair": s(&pair), "index": i, "funder": s(&payer)}));
    }
    let width = hi - lo + 1;
    let position = position_pda(&pair, &dk, lo, width);
    let ip = ix_init_position_pda(&payer, &dk, &pair, &dk, lo, width);
    env.send_ok("position", &[ip.clone()], &[&desk]);
    rec.ix("dlmm.initialize_position_pda", &ip, json!({"payer": s(&payer), "base": s(&dk), "pair": s(&pair), "owner": s(&dk), "lower": lo, "width": width}));
    assert_eq!(position_range(&env, &position), (lo, hi));

    // ---- 3. the deposit: 10 SOL a side, spot
    let sol_side = 10 * LAMPORTS;
    let wx_amount = (sol_side as f64 / bin_price(active0)) as u64 + 1_000_000;
    let dx = mint_wx(&mut env, &wx, &canister, &dk, wx_amount);
    let dy = env.wsol(&desk, sol_side + LAMPORTS);
    let (x0, y0) = (env.token_amount(&dx), env.token_amount(&dy));
    let add = ix_add_strategy2(&lb, &position, &dk, &dx, &dy, wx_amount - 1_000_000, sol_side, active0, lo, hi, SPOT);
    env.send_ok("add", &[add.clone()], &[&desk]);
    rec.ix("dlmm.add_liquidity_by_strategy2", &add, json!({"pair": s(&pair), "x": s(&wx), "y": s(&NATIVE_MINT), "position": s(&position), "sender": s(&dk), "ux": s(&dx), "uy": s(&dy),
        "ax": wx_amount - 1_000_000, "ay": sol_side, "active_id": active0, "lo": lo, "hi": hi, "shape": SPOT,
        "deposited_x": x0 - env.token_amount(&dx), "deposited_y": y0 - env.token_amount(&dy)}));
    rec.bundle("add_liquidity", &[add], &[payer, dk], &payer);

    // accounts for the decoders: the position and the three bin arrays right after the deposit
    let desk_exp = json!({"lb_pair": s(&pair), "owner": s(&dk), "lower": lo, "upper": hi});
    rec.account("position_after_add", &env, &position, desk_exp.clone());
    for i in array_index(lo)..=array_index(hi) {
        rec.account(&format!("bin_array_{i}"), &env, &bin_array(&pair, i), json!({"index": i, "lb_pair": s(&pair)}));
    }
    rec.account("lb_pair_after_add", &env, &pair, lb_expect(&env, &lb));

    // ---- 4. a trader: a 0.5 SOL buy and the sell back, then volume for fees
    let t = trader(&mut env, &wx, 20);
    let up = swap_path(&env, &lb, true);
    let buy = ix_swap2(&lb, &t.k.pubkey(), &t.y, &t.x, LAMPORTS / 2, 0, &up);
    let w0 = env.token_amount(&t.x);
    env.send_ok("buy", &[buy.clone()], &[&t.k]);
    let got = env.token_amount(&t.x) - w0;
    rec.ix("dlmm.swap2", &buy, json!({"pair": s(&pair), "x": s(&wx), "y": s(&NATIVE_MINT), "user": s(&t.k.pubkey()), "input": s(&t.y), "output": s(&t.x),
        "amount_in": LAMPORTS / 2, "min_out": 0, "arrays": ss(&up), "got": got}));
    let down = swap_path(&env, &lb, false);
    let sell = ix_swap2(&lb, &t.k.pubkey(), &t.x, &t.y, got, 0, &down);
    env.send_ok("sell", &[sell.clone()], &[&t.k]);
    rec.ix("dlmm.swap2", &sell, json!({"pair": s(&pair), "x": s(&wx), "y": s(&NATIVE_MINT), "user": s(&t.k.pubkey()), "input": s(&t.x), "output": s(&t.y),
        "amount_in": got, "min_out": 0, "arrays": ss(&down), "got": null}));
    for _ in 0..5 {
        let up = swap_path(&env, &lb, true);
        let w0 = env.token_amount(&t.x);
        env.send_ok("buy", &[ix_swap2(&lb, &t.k.pubkey(), &t.y, &t.x, 2 * LAMPORTS, 0, &up)], &[&t.k]);
        let got = env.token_amount(&t.x) - w0;
        let down = swap_path(&env, &lb, false);
        env.send_ok("sell", &[ix_swap2(&lb, &t.k.pubkey(), &t.x, &t.y, got, 0, &down)], &[&t.k]);
    }

    // ---- 5. fees: dump the accounts first, then claim, and record what the chain paid
    rec.account("position_before_claim", &env, &position, desk_exp.clone());
    for i in array_index(lo)..=array_index(hi) {
        rec.account(&format!("bin_array_before_claim_{i}"), &env, &bin_array(&pair, i), json!({"index": i, "lb_pair": s(&pair)}));
    }
    let (bx, by) = (env.token_amount(&dx), env.token_amount(&dy));
    let claim = ix_claim_fee2(&lb, &position, &dk, &dx, &dy, lo, hi);
    env.send_ok("claim", &[claim.clone()], &[&desk]);
    let (fx, fy) = (env.token_amount(&dx) - bx, env.token_amount(&dy) - by);
    rec.ix("dlmm.claim_fee2", &claim, json!({"pair": s(&pair), "x": s(&wx), "y": s(&NATIVE_MINT), "position": s(&position), "sender": s(&dk), "ux": s(&dx), "uy": s(&dy),
        "lo": lo, "hi": hi, "paid_x": fx, "paid_y": fy}));
    assert_eq!(fx, 0, "fees arrive in SOL only (preset 262)");
    assert!(fy > 0);
    rec.bundle("claim_fee", &[claim], &[payer, dk], &payer);

    // ---- 6. the up-arb: mint wX at the module + sell it into the pool, one transaction (3 signers)
    let base_state = env.svm.clone();
    let module_vault = kp(40).pubkey(); // a stand-in for the module's address M[i]
    {
        let sell_x = LAMPORTS / 100;
        let mint = spl_token::instruction::mint_to_checked(&spl_token::ID, &wx, &dx, &canister.pubkey(), &[], sell_x, 9).unwrap();
        let pay = system_instruction::transfer(&dk, &module_vault, 18_000_000);
        let down = arrays_for(&pair, lo, hi);
        let sell = ix_swap2(&lb, &dk, &dx, &dy, sell_x, 1, &down.iter().rev().copied().collect::<Vec<_>>());
        env.send_ok("arb up", &[pay.clone(), mint.clone(), sell.clone()], &[&desk, &canister]);
        rec.ix("dlmm.swap2", &sell, json!({"pair": s(&pair), "x": s(&wx), "y": s(&NATIVE_MINT), "user": s(&dk), "input": s(&dx), "output": s(&dy),
            "amount_in": sell_x, "min_out": 1, "arrays": ss(&down.iter().rev().copied().collect::<Vec<_>>()), "got": null}));
        rec.bundle("arb_up", &[pay, mint, sell], &[payer, dk, canister.pubkey()], &payer);
        env.svm = base_state.clone();
    }

    // ---- 7. R1: the floor +7 %, inside the band: pay the module + mint wX + rebalance_liquidity on the SAME position (3 signers)
    for (case, mv) in [("rebalance_inside", 1.07f64), ("rebalance_past", 1.15)] {
        env.svm = base_state.clone();
        let target = id_for_price(FLOOR * mv);
        let arb = trader(&mut env, &wx, 21);
        let mut refused = false;
        while active_id(&env, &pair) < target {
            let up = swap_path(&env, &lb, true);
            if env.send(&[ix_swap2(&lb, &arb.k.pubkey(), &arb.y, &arb.x, LAMPORTS / 4, 0, &up)], &[&arb.k]).is_err() {
                refused = true;
                break;
            }
        }
        let past = refused;
        let mut pre = vec![];
        if past {
            let from = bin_array(&pair, array_index(active_id(&env, &pair)));
            let to = bin_array(&pair, array_index(target));
            pre.push(ix_remove_range2(&lb, &position, &dk, &dx, &dy, lo, hi, 10_000));
            pre.push(ix_go_to_a_bin(&pair, Some(from), Some(to), target));
        }
        let a = if past { target } else { active_id(&env, &pair) };
        let (lo2, hi2) = (a - half, a + half);
        for i in array_index(lo2)..=array_index(hi2) {
            if !env.exists(&bin_array(&pair, i)) {
                let ix = ix_init_bin_array(&pair, i, &payer);
                env.send_ok("new bin array", &[ix.clone()], &[]);
                rec.ix("dlmm.initialize_bin_array", &ix, json!({"pair": s(&pair), "index": i, "funder": s(&payer)}));
            }
        }
        let (rx, ry) = (env.token_amount(&lb.rx()), env.token_amount(&lb.ry()));
        let p_new = bin_price(a);
        let total_value = rx as f64 * p_new + ry as f64;
        let per_bin = total_value * 0.98 / (2 * half + 1) as f64;
        let (x0v, y0v) = (per_bin as u64, per_bin as u64);
        let all_arrays = arrays_for(&pair, lo.min(lo2), hi.max(hi2));
        let need_x = ((half as f64 * per_bin / p_new) * 1.01) as u64;
        let mint_x = need_x.saturating_sub(rx + env.token_amount(&dx));
        let mut ixs = pre.clone();
        let pay = system_instruction::transfer(&dk, &module_vault, (mint_x as f64 * p_new) as u64);
        let mint = spl_token::instruction::mint_to_checked(&spl_token::ID, &wx, &dx, &canister.pubkey(), &[], mint_x, 9).unwrap();
        if mint_x > 0 {
            ixs.insert(0, pay.clone());
            ixs.insert(1, mint.clone());
        }
        let remove = if past { None } else { Some((lo, hi)) };
        let adds = [RbAdd { min_delta: -half, max_delta: half, x0: x0v, y0: y0v }];
        let rb = ix_rebalance(&lb, &position, &dk, &payer, &dx, &dy, a, true, remove, &adds, &all_arrays);
        ixs.push(rb.clone());
        env.send_ok(case, &ixs, &[&desk, &canister]);
        let (plo, phi) = position_range(&env, &position);
        rec.ix("dlmm.rebalance_liquidity", &rb, json!({"pair": s(&pair), "x": s(&wx), "y": s(&NATIVE_MINT), "position": s(&position), "owner": s(&dk), "rent_payer": s(&payer),
            "ux": s(&dx), "uy": s(&dy), "active_id": a, "claim_fee": true, "remove": remove.map(|(l, h)| json!([l, h])),
            "adds": [{"min_delta": -half, "max_delta": half, "x0": x0v, "y0": y0v}], "arrays": ss(&all_arrays), "new_lower": plo, "new_upper": phi}));
        if past {
            let from = bin_array(&pair, array_index(active_id(&env, &pair)));
            let _ = from;
            rec.ix("dlmm.remove_liquidity_by_range2", &pre[0], json!({"pair": s(&pair), "x": s(&wx), "y": s(&NATIVE_MINT), "position": s(&position), "sender": s(&dk),
                "ux": s(&dx), "uy": s(&dy), "lo": lo, "hi": hi, "bps": 10_000}));
            let (fa, ta) = (bin_array(&pair, array_index(target)), bin_array(&pair, array_index(target)));
            let _ = (fa, ta);
            rec.ix("dlmm.go_to_a_bin", &pre[1], json!({"pair": s(&pair), "from_array": pre[1].accounts[2].pubkey.to_string(), "to_array": pre[1].accounts[3].pubkey.to_string(), "bin": target}));
        }
        rec.bundle(case, &ixs, &[payer, dk, canister.pubkey()], &payer);
        assert_eq!(phi, a + half, "the position now spans the new band: [{plo}, {phi}] around {a}");
        // the position now: dump for the decoders
        rec.account(&format!("position_{case}"), &env, &position, json!({"lb_pair": s(&pair), "owner": s(&dk), "lower": plo, "upper": phi}));
        rec.account(&format!("lb_pair_{case}"), &env, &pair, lb_expect(&env, &lb));
    }

    // ---- 8. increase_position_length2: a second position over 70 bins (a ±21 % band is 81 bins)
    env.svm = base_state.clone();
    let lo3 = active0 - 40;
    let pos3 = position_pda(&pair, &dk, lo3, 70);
    let ip3 = ix_init_position_pda(&payer, &dk, &pair, &dk, lo3, 70);
    env.send_ok("position 70", &[ip3.clone()], &[&desk]);
    rec.ix("dlmm.initialize_position_pda", &ip3, json!({"payer": s(&payer), "base": s(&dk), "pair": s(&pair), "owner": s(&dk), "lower": lo3, "width": 70}));
    let inc = ix_increase_len2(&payer, &pair, &pos3, &dk, lo3 + 80);
    env.send_ok("extend", &[inc.clone()], &[&desk]);
    rec.ix("dlmm.increase_position_length2", &inc, json!({"funder": s(&payer), "pair": s(&pair), "position": s(&pos3), "owner": s(&dk), "min_upper": lo3 + 80}));
    assert_eq!(position_range(&env, &pos3), (lo3, lo3 + 80));
    rec.account("position_extended", &env, &pos3, json!({"lb_pair": s(&pair), "owner": s(&dk), "lower": lo3, "upper": lo3 + 80}));
    // an empty position closes with its rent going back
    let (cl_lo, cl_hi) = (lo3, lo3 + 80);
    let close3 = ix_close_position2(&pos3, &dk, &payer);
    env.send_ok("close empty", &[close3.clone()], &[&desk]);
    rec.ix("dlmm.close_position2", &close3, json!({"position": s(&pos3), "sender": s(&dk), "rent_to": s(&payer)}));
    let _ = (cl_lo, cl_hi);

    // ---- 9. wind down the desk's main position: dump, remove 100 %, claim, close; record what the chain paid
    env.svm = base_state.clone();
    rec.account("position_before_wind_down", &env, &position, desk_exp.clone());
    for i in array_index(lo)..=array_index(hi) {
        rec.account(&format!("bin_array_before_wind_down_{i}"), &env, &bin_array(&pair, i), json!({"index": i, "lb_pair": s(&pair)}));
    }
    let (bx, by) = (env.token_amount(&dx), env.token_amount(&dy));
    let rm = ix_remove_range2(&lb, &position, &dk, &dx, &dy, lo, hi, 10_000);
    env.send_ok("remove", &[rm.clone()], &[&desk]);
    let (rxp, ryp) = (env.token_amount(&dx) - bx, env.token_amount(&dy) - by);
    rec.ix("dlmm.remove_liquidity_by_range2", &rm, json!({"pair": s(&pair), "x": s(&wx), "y": s(&NATIVE_MINT), "position": s(&position), "sender": s(&dk),
        "ux": s(&dx), "uy": s(&dy), "lo": lo, "hi": hi, "bps": 10_000, "paid_x": rxp, "paid_y": ryp}));
    let cl = ix_claim_fee2(&lb, &position, &dk, &dx, &dy, lo, hi);
    env.send_ok("claim", &[cl.clone()], &[&desk]);
    let close = ix_close_position2(&position, &dk, &payer);
    env.send_ok("close", &[close.clone()], &[&desk]);
    rec.ix("dlmm.close_position2", &close, json!({"position": s(&position), "sender": s(&dk), "rent_to": s(&payer)}));
    rec.bundle("wind_down", &[rm, cl, close], &[payer, dk], &payer);

    // ---- 10. a squatted empty pair, moved back by anyone (no bin arrays needed)
    let wx2_kp = kp(31);
    new_wx(&mut env, &wx2_kp, &canister);
    let wx2 = wx2_kp.pubkey();
    let wrong = id_for_price(FLOOR * 3.0);
    let squat = ix_init_pair2(&payer, &preset(DESK_PRESET), &wx2, &NATIVE_MINT, wrong);
    env.send_ok("squat", &[squat.clone()], &[]);
    let sp = pair_with_preset(&preset(DESK_PRESET), &wx2, &NATIVE_MINT);
    let target = id_for_price(FLOOR);
    let go = ix_go_to_a_bin(&sp, None, None, target);
    env.send_ok("go to a bin", &[go.clone()], &[]);
    rec.ix("dlmm.go_to_a_bin", &go, json!({"pair": s(&sp), "from_array": null, "to_array": null, "bin": target}));
    assert_eq!(active_id(&env, &sp), target);

    let out = json!({
        "keys": {
            "k_main": s(&payer), "k_coll": s(&canister.pubkey()), "k_desk": s(&dk), "wx_mint": s(&wx), "pair": s(&pair), "position": s(&position),
            "preset": s(&preset(DESK_PRESET)), "preset_index": DESK_PRESET, "bin_step": BIN_STEP, "floor": FLOOR, "lo": lo, "hi": hi, "active0": active0,
            "desk_wx": s(&dx), "desk_wsol": s(&dy), "module": s(&module_vault), "dlmm": s(&DLMM_ID),
        },
        "ixs": rec.ixs, "bundles": rec.bundles, "accounts": rec.accounts,
    });
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../autobuyer/canister-rs/tests/fixtures/solix2/dlmm.json");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, serde_json::to_string_pretty(&out).unwrap()).unwrap();
    println!("wrote {} ({} instructions, {} bundles, {} accounts)", path.display(), out["ixs"].as_array().unwrap().len(), out["bundles"].as_array().unwrap().len(), out["accounts"].as_array().unwrap().len());
}
