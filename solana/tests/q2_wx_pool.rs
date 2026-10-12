//! Spike Q2 — a per-collection wX/SOL pool on Meteora DAMM v2 (mainnet program bytes 0.2.4, LiteSVM), for
//! docs/v9/wx-pools-design.md. No Rook program is involved: a keypair (`desk`) stands in for the PDA that will own the
//! position, so these numbers are DAMM v2's own CU; the CPI wrapper adds its overhead on top.
//!
//! What it shows:
//!   1. the customizable-pool address `["cpool", max(mint), min(mint)]` is one per pair: anyone who holds a little wX can
//!      create it first, at any price, and Rook's own creation then fails (squatting);
//!   2. a Rook-only dynamic config (`pool_creator_authority` = Rook's PDA; Meteora creates such configs, the DBC migration
//!      uses one) gives a pool address only Rook can create;
//!   3. the pool opens at the floor price, full range, 0.5 % fixed fee, fees in SOL only; the whole position is permanently
//!      locked and can never be withdrawn; later liquidity is added and locked too;
//!   4. swaps both ways; the position's fees are claimable only by the NFT owner, in SOL only;
//!   5. depth: how far a buy/sell of a given size moves the price.
//!
//! Run: `cargo test -p rook_backing --test q2_wx_pool -- --nocapture` (after the SBPF v2 build of rook_backing).

#[path = "harness/mod.rs"]
mod harness;

use anchor_lang::{prelude::*, solana_program::instruction::Instruction, InstructionData, ToAccountMetas};
use anchor_spl::{associated_token::get_associated_token_address, token::spl_token};
use base64::Engine;
use harness::*;
use rook_backing::{constants::*, cp_amm};
use solana_account::Account as SolAccount;
use solana_keypair::Keypair;
use solana_signer::Signer;

/// DAMM v2's price bounds (Q64.64 sqrt prices; cp_amm `constants::{MIN_SQRT_PRICE, MAX_SQRT_PRICE}`).
const MIN_SQRT_PRICE: u128 = 4_295_048_016;
const MAX_SQRT_PRICE: u128 = 79_226_673_521_066_979_257_578_248_091;
/// The pool fee: 0.5 % (numerator of 1e9).
const FEE_NUM: u64 = 5_000_000;
/// A floor of 50 SOL per whole wX. wX and SOL both have 9 decimals, so the pool price (lamports per wX base unit) is 50.
const FLOOR_LAMPORTS: u64 = 50 * LAMPORTS;

fn sqrt_price_of(lamports_per_whole: u64) -> u128 {
    // sqrt(p) · 2^64 with p = lamports per base unit (both 9 decimals)
    let p = lamports_per_whole as f64 / 1e9;
    (p.sqrt() * 18_446_744_073_709_551_616f64) as u128
}
fn price_of_sqrt(s: u128) -> f64 {
    let r = s as f64 / 18_446_744_073_709_551_616f64;
    r * r * 1e9 // lamports per whole unit
}
/// Liquidity (Q64) a full-range position holds with at most `a` wX base units and `b` lamports at sqrt price `s`.
fn liquidity_for(a: u64, b: u64, s: u128) -> u128 {
    let two64 = 18_446_744_073_709_551_616f64;
    let sp = s as f64 / two64;
    let smin = MIN_SQRT_PRICE as f64 / two64;
    let smax = MAX_SQRT_PRICE as f64 / two64;
    let la = a as f64 / (1.0 / sp - 1.0 / smax);
    let lb = b as f64 / (sp - smin);
    (la.min(lb) * 0.999_999 * two64) as u128
}

fn base_fee(num: u64) -> cp_amm::types::BaseFeeParameters {
    // borsh(cliff_fee_numerator u64, number_of_period u16, period_frequency u64, reduction_factor u64, base_fee_mode u8)
    let mut d = [0u8; 27];
    d[..8].copy_from_slice(&num.to_le_bytes());
    cp_amm::types::BaseFeeParameters { data: d }
}
fn pool_params(liquidity: u128, sqrt_price: u128) -> cp_amm::types::InitializeCustomizablePoolParameters {
    cp_amm::types::InitializeCustomizablePoolParameters {
        pool_fees: cp_amm::types::PoolFeeParameters { base_fee: base_fee(FEE_NUM), compounding_fee_bps: 0, padding: 0, dynamic_fee: None },
        sqrt_min_price: MIN_SQRT_PRICE,
        sqrt_max_price: MAX_SQRT_PRICE,
        has_alpha_vault: false,
        liquidity,
        sqrt_price,
        activation_type: 1, // timestamp
        collect_fee_mode: 1, // fees in token b (SOL) only
        activation_point: None,
    }
}

fn cpool_pda(a: &Pubkey, b: &Pubkey) -> Pubkey {
    let (hi, lo) = if a > b { (a, b) } else { (b, a) };
    pda(&[b"cpool", hi.as_ref(), lo.as_ref()], &DAMM_V2_ID)
}
fn config_pool_pda(config: &Pubkey, a: &Pubkey, b: &Pubkey) -> Pubkey {
    let (hi, lo) = if a > b { (a, b) } else { (b, a) };
    pda(&[b"pool", config.as_ref(), hi.as_ref(), lo.as_ref()], &DAMM_V2_ID)
}

struct Pool {
    pool: Pubkey,
    nft: Pubkey,
    wx: Pubkey,
}
impl Pool {
    fn position(&self) -> Pubkey {
        damm_position(&self.nft)
    }
    fn nft_account(&self) -> Pubkey {
        damm_position_nft_account(&self.nft)
    }
    fn vault_a(&self) -> Pubkey {
        damm_vault(&self.wx, &self.pool)
    }
    fn vault_b(&self) -> Pubkey {
        damm_vault(&NATIVE_MINT, &self.pool)
    }
}

/// Offsets of `liquidity` and `sqrt_price` in cp_amm 0.2.4's `Pool`, found by value after the first pool opens (the IDL's
/// bytemuck layout has nested fee structs whose sizes the spike does not hard-code).
static OFFS: std::sync::OnceLock<(usize, usize)> = std::sync::OnceLock::new();
fn find(d: &[u8], v: u128) -> usize {
    let b = v.to_le_bytes();
    let hits: Vec<usize> = (8..d.len() - 16).filter(|&o| d[o..o + 16] == b).collect();
    *hits.last().expect("value in the pool account")
}
fn pool_u128(env: &Env, pool: &Pubkey, field: &str) -> u128 {
    let d = env.raw(pool);
    let (lo, so) = *OFFS.get().expect("offsets learned");
    let o = if field == "liquidity" { lo } else { so };
    u128::from_le_bytes(d[o..o + 16].try_into().unwrap())
}
/// (unlocked, vested, permanently locked) liquidity of a DAMM v2 position (cp_amm 0.2.4 `Position`).
fn position_liquidity(env: &Env, position: &Pubkey) -> (u128, u128, u128) {
    let d = env.raw(position);
    let r = |o: usize| u128::from_le_bytes(d[o..o + 16].try_into().unwrap());
    let o = 8 + 32 + 32 + 32 + 32 + 8 + 8;
    (r(o), r(o + 16), r(o + 32))
}

fn ix_init_customizable(payer: &Pubkey, creator: &Pubkey, nft: &Pubkey, wx: &Pubkey, pa: Pubkey, pb: Pubkey, liq: u128, s: u128) -> Instruction {
    let pool = cpool_pda(wx, &NATIVE_MINT);
    Instruction {
        program_id: DAMM_V2_ID,
        accounts: cp_amm::client::accounts::InitializeCustomizablePool {
            creator: *creator,
            position_nft_mint: *nft,
            position_nft_account: damm_position_nft_account(nft),
            payer: *payer,
            pool_authority: DAMM_V2_POOL_AUTHORITY,
            pool,
            position: damm_position(nft),
            token_a_mint: *wx,
            token_b_mint: NATIVE_MINT,
            token_a_vault: damm_vault(wx, &pool),
            token_b_vault: damm_vault(&NATIVE_MINT, &pool),
            payer_token_a: pa,
            payer_token_b: pb,
            token_a_program: spl_token::ID,
            token_b_program: spl_token::ID,
            token_2022_program: TOKEN_2022_ID,
            system_program: anchor_lang::system_program::ID,
            event_authority: event_authority(&DAMM_V2_ID),
            program: DAMM_V2_ID,
        }
        .to_account_metas(None),
        data: cp_amm::client::args::InitializeCustomizablePool { params: pool_params(liq, s) }.data(),
    }
}

#[allow(clippy::too_many_arguments)]
fn ix_init_dynamic(payer: &Pubkey, creator: &Pubkey, authority: &Pubkey, config: &Pubkey, nft: &Pubkey, wx: &Pubkey, pa: Pubkey, pb: Pubkey, liq: u128, s: u128) -> Instruction {
    let pool = config_pool_pda(config, wx, &NATIVE_MINT);
    Instruction {
        program_id: DAMM_V2_ID,
        accounts: cp_amm::client::accounts::InitializePoolWithDynamicConfig {
            creator: *creator,
            position_nft_mint: *nft,
            position_nft_account: damm_position_nft_account(nft),
            payer: *payer,
            pool_creator_authority: *authority,
            config: *config,
            pool_authority: DAMM_V2_POOL_AUTHORITY,
            pool,
            position: damm_position(nft),
            token_a_mint: *wx,
            token_b_mint: NATIVE_MINT,
            token_a_vault: damm_vault(wx, &pool),
            token_b_vault: damm_vault(&NATIVE_MINT, &pool),
            payer_token_a: pa,
            payer_token_b: pb,
            token_a_program: spl_token::ID,
            token_b_program: spl_token::ID,
            token_2022_program: TOKEN_2022_ID,
            system_program: anchor_lang::system_program::ID,
            event_authority: event_authority(&DAMM_V2_ID),
            program: DAMM_V2_ID,
        }
        .to_account_metas(None),
        data: cp_amm::client::args::InitializePoolWithDynamicConfig { params: pool_params(liq, s) }.data(),
    }
}

fn ix_lock(p: &Pool, owner: &Pubkey, liq: u128) -> Instruction {
    Instruction {
        program_id: DAMM_V2_ID,
        accounts: cp_amm::client::accounts::PermanentLockPosition {
            pool: p.pool,
            position: p.position(),
            position_nft_account: p.nft_account(),
            signer: *owner,
            event_authority: event_authority(&DAMM_V2_ID),
            program: DAMM_V2_ID,
        }
        .to_account_metas(None),
        data: cp_amm::client::args::PermanentLockPosition { permanent_lock_liquidity: liq }.data(),
    }
}

fn ix_add(p: &Pool, owner: &Pubkey, a: Pubkey, b: Pubkey, liq: u128, max_a: u64, max_b: u64) -> Instruction {
    Instruction {
        program_id: DAMM_V2_ID,
        accounts: cp_amm::client::accounts::AddLiquidity {
            pool: p.pool,
            position: p.position(),
            token_a_account: a,
            token_b_account: b,
            token_a_vault: p.vault_a(),
            token_b_vault: p.vault_b(),
            token_a_mint: p.wx,
            token_b_mint: NATIVE_MINT,
            position_nft_account: p.nft_account(),
            signer: *owner,
            token_a_program: spl_token::ID,
            token_b_program: spl_token::ID,
            event_authority: event_authority(&DAMM_V2_ID),
            program: DAMM_V2_ID,
        }
        .to_account_metas(None),
        data: cp_amm::client::args::AddLiquidity {
            params: cp_amm::types::AddLiquidityParameters { liquidity_delta: liq, token_a_amount_threshold: max_a, token_b_amount_threshold: max_b },
        }
        .data(),
    }
}

fn ix_claim(p: &Pool, owner: &Pubkey, a: Pubkey, b: Pubkey) -> Instruction {
    Instruction {
        program_id: DAMM_V2_ID,
        accounts: cp_amm::client::accounts::ClaimPositionFee {
            pool_authority: DAMM_V2_POOL_AUTHORITY,
            pool: p.pool,
            position: p.position(),
            token_a_account: a,
            token_b_account: b,
            token_a_vault: p.vault_a(),
            token_b_vault: p.vault_b(),
            token_a_mint: p.wx,
            token_b_mint: NATIVE_MINT,
            position_nft_account: p.nft_account(),
            signer: *owner,
            token_a_program: spl_token::ID,
            token_b_program: spl_token::ID,
            event_authority: event_authority(&DAMM_V2_ID),
            program: DAMM_V2_ID,
        }
        .to_account_metas(None),
        data: cp_amm::client::args::ClaimPositionFee {}.data(),
    }
}

fn ix_remove_all(p: &Pool, owner: &Pubkey, a: Pubkey, b: Pubkey) -> Instruction {
    Instruction {
        program_id: DAMM_V2_ID,
        accounts: cp_amm::client::accounts::RemoveAllLiquidity {
            pool_authority: DAMM_V2_POOL_AUTHORITY,
            pool: p.pool,
            position: p.position(),
            token_a_account: a,
            token_b_account: b,
            token_a_vault: p.vault_a(),
            token_b_vault: p.vault_b(),
            token_a_mint: p.wx,
            token_b_mint: NATIVE_MINT,
            position_nft_account: p.nft_account(),
            signer: *owner,
            token_a_program: spl_token::ID,
            token_b_program: spl_token::ID,
            event_authority: event_authority(&DAMM_V2_ID),
            program: DAMM_V2_ID,
        }
        .to_account_metas(None),
        data: cp_amm::client::args::RemoveAllLiquidity { token_a_amount_threshold: 0, token_b_amount_threshold: 0 }.data(),
    }
}

fn ix_swap(p: &Pool, trader: &Pubkey, input: Pubkey, output: Pubkey, amount_in: u64) -> Instruction {
    Instruction {
        program_id: DAMM_V2_ID,
        accounts: cp_amm::client::accounts::Swap {
            pool_authority: DAMM_V2_POOL_AUTHORITY,
            pool: p.pool,
            input_token_account: input,
            output_token_account: output,
            token_a_vault: p.vault_a(),
            token_b_vault: p.vault_b(),
            token_a_mint: p.wx,
            token_b_mint: NATIVE_MINT,
            payer: *trader,
            token_a_program: spl_token::ID,
            token_b_program: spl_token::ID,
            referral_token_account: None,
            event_authority: event_authority(&DAMM_V2_ID),
            program: DAMM_V2_ID,
        }
        .to_account_metas(None),
        data: cp_amm::client::args::Swap { _params: cp_amm::types::SwapParameters { amount_in, minimum_amount_out: 0 } }.data(),
    }
}

/// A Rook-only dynamic config: the mainnet "Customizable" DBC migration config's bytes with `pool_creator_authority` set to
/// `authority` (what Meteora's `create_dynamic_config` would write for Rook).
fn rook_dynamic_config(env: &mut Env, authority: &Pubkey) -> Pubkey {
    let j: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/damm_v2_config_customizable.json"))
            .unwrap(),
    )
    .unwrap();
    let mut data = base64::engine::general_purpose::STANDARD.decode(j["account"]["data"][0].as_str().unwrap()).unwrap();
    data[40..72].copy_from_slice(authority.as_ref());
    let key = Keypair::new().pubkey();
    env.svm
        .set_account(key, SolAccount { lamports: j["account"]["lamports"].as_u64().unwrap(), data, owner: DAMM_V2_ID, executable: false, rent_epoch: 0 })
        .unwrap();
    key
}

fn mint_to(env: &mut Env, mint: &Pubkey, authority: &Keypair, dest: &Pubkey, amount: u64) {
    let ix = spl_token::instruction::mint_to(&spl_token::ID, mint, dest, &authority.pubkey(), &[], amount).unwrap();
    env.send_ok("mint_to", &[ix], &[authority]);
}

#[test]
fn q2_wx_pool() {
    let mut env = Env::new();
    // wX: SPL Token, 9 decimals (as rook_core's `["wx_mint", wrapper]`); a keypair stands in for the wrapper PDA
    let wrapper = env.funded(1);
    let wx = env.create_mint(&wrapper.pubkey(), 9);
    // the desk: the PDA that will own the position (system-owned, signs DAMM v2 through invoke_signed)
    let desk = env.funded(100);
    let desk_wx = env.ata(&desk.pubkey(), &wx);

    // ---- seed: the pool slice of a first graduation on the 170 SOL tier (85 SOL migration fee × 25 % = 21.25 SOL of value):
    //      10.625 SOL + 10.625 SOL of wX at 50 (docs/v9/wx-pools-design.md §3, option A)
    let seed_sol = 10_625_000_000u64;
    let seed_wx = (seed_sol as u128 * 1_000_000_000 / FLOOR_LAMPORTS as u128) as u64; // 0.2125 wX
    mint_to(&mut env, &wx, &wrapper, &desk_wx, seed_wx);
    let desk_wsol = env.wsol(&desk, seed_sol);
    let s0 = sqrt_price_of(FLOOR_LAMPORTS);
    let liq = liquidity_for(seed_wx, seed_sol, s0);

    // ---- 1. squatting the customizable address: a stranger with a little wX creates ["cpool", wX, SOL] first
    let squatter = env.funded(10);
    let sq_wx = env.ata(&squatter.pubkey(), &wx);
    mint_to(&mut env, &wx, &wrapper, &sq_wx, 1_000); // a millionth of a unit
    let sq_wsol = env.wsol(&squatter, 1_000_000);
    let bad_s = sqrt_price_of(FLOOR_LAMPORTS * 10); // ten times the floor
    let sq_nft = Keypair::new();
    let ix = ix_init_customizable(&squatter.pubkey(), &squatter.pubkey(), &sq_nft.pubkey(), &wx, sq_wx, sq_wsol, liquidity_for(1_000, 1_000_000, bad_s), bad_s);
    env.send_ok("damm_v2.initialize_customizable_pool (squatter)", &[ix], &[&squatter, &sq_nft]);
    let nft = Keypair::new();
    let ix = ix_init_customizable(&desk.pubkey(), &desk.pubkey(), &nft.pubkey(), &wx, desk_wx, desk_wsol, liq, s0);
    let logs = env.send_err("rook's customizable pool after the squat", &[ix], &[&desk, &nft]);
    assert!(logs.contains("already in use"), "{logs}");
    println!("1. the customizable pool address is one per pair and anyone holding a little wX can take it first: squatted at 10× the floor; Rook's creation refused");

    // ---- 2. a Rook-only dynamic config: only its pool_creator_authority can create pools on it
    let config = rook_dynamic_config(&mut env, &desk.pubkey());
    let nft_s = Keypair::new();
    let ix = ix_init_dynamic(&squatter.pubkey(), &squatter.pubkey(), &squatter.pubkey(), &config, &nft_s.pubkey(), &wx, sq_wx, sq_wsol, liquidity_for(1_000, 1_000_000, bad_s), bad_s);
    let logs = env.send_err("stranger on Rook's dynamic config", &[ix], &[&squatter, &nft_s]);
    assert!(logs.contains("InvalidPoolCreatorAuthority") || logs.contains("ConstraintHasOne") || logs.contains("Error"), "{logs}");
    let ix = ix_init_dynamic(&desk.pubkey(), &desk.pubkey(), &desk.pubkey(), &config, &nft.pubkey(), &wx, desk_wx, desk_wsol, liq, s0);
    env.send_ok("damm_v2.initialize_pool_with_dynamic_config (Rook)", &[ix], &[&desk, &nft]);
    let p = Pool { pool: config_pool_pda(&config, &wx, &NATIVE_MINT), nft: nft.pubkey(), wx };
    {
        let d = env.raw(&p.pool);
        let _ = OFFS.set((find(&d, liq), find(&d, s0)));
    }
    let used_wx = seed_wx - env.token_amount(&desk_wx);
    let used_sol = seed_sol - env.token_amount(&desk_wsol);
    let px = price_of_sqrt(pool_u128(&env, &p.pool, "sqrt_price"));
    println!(
        "2. Rook's pool on its own dynamic config: {} wX base units + {} lamports, price {:.6} SOL/wX (floor 50), leftover {} wX / {} lamports",
        used_wx,
        used_sol,
        px / 1e9,
        seed_wx - used_wx,
        seed_sol - used_sol
    );
    assert!((px - FLOOR_LAMPORTS as f64).abs() / (FLOOR_LAMPORTS as f64) < 1e-9);
    assert!(seed_wx - used_wx <= seed_wx / 10_000 && seed_sol - used_sol <= seed_sol / 10_000);
    assert_eq!(env.token_owner(&p.nft_account()), desk.pubkey(), "the position NFT is held by the desk");

    // ---- 3. permanent lock of the whole position
    let (u, _, _) = position_liquidity(&env, &p.position());
    env.send_ok("damm_v2.permanent_lock_position", &[ix_lock(&p, &desk.pubkey(), u)], &[&desk]);
    let (u2, v2, l2) = position_liquidity(&env, &p.position());
    assert_eq!((u2, v2, l2), (0, 0, u));
    let logs = env.send_err("remove_all_liquidity after the lock", &[ix_remove_all(&p, &desk.pubkey(), desk_wx, desk_wsol)], &[&desk]);
    println!("3. permanently locked {} liquidity; remove_all_liquidity refused: {}", l2, logs.lines().filter(|l| l.contains("Error")).last().unwrap_or(""));

    // ---- 4. swaps both ways, then fees: SOL only, to the NFT owner only
    let trader = env.funded(50);
    let t_wsol = env.wsol(&trader, 20 * LAMPORTS);
    let t_wx = env.ata(&trader.pubkey(), &wx);
    let before = pool_u128(&env, &p.pool, "sqrt_price");
    env.send_ok("damm_v2.swap (buy wX with 0.1 SOL)", &[ix_swap(&p, &trader.pubkey(), t_wsol, t_wx, LAMPORTS / 10)], &[&trader]);
    let bought = env.token_amount(&t_wx);
    let after = pool_u128(&env, &p.pool, "sqrt_price");
    let eff = (LAMPORTS / 10) as f64 * 1e9 / bought as f64;
    println!(
        "4a. 0.1 SOL buys {} wX base units: average {:.4} SOL/wX ({:+.3} % vs the floor); the pool price moves {:+.3} %",
        bought,
        eff / 1e9,
        (eff / FLOOR_LAMPORTS as f64 - 1.0) * 100.0,
        (price_of_sqrt(after) / price_of_sqrt(before) - 1.0) * 100.0
    );
    let sell = bought / 2;
    env.send_ok("damm_v2.swap (sell wX)", &[ix_swap(&p, &trader.pubkey(), t_wx, t_wsol, sell)], &[&trader]);

    let stranger = env.funded(1);
    let st_wx = env.ata(&stranger.pubkey(), &wx);
    let st_wsol = env.wsol(&stranger, 1_000_000);
    let logs = env.send_err("claim_position_fee by a stranger", &[ix_claim(&p, &stranger.pubkey(), st_wx, st_wsol)], &[&stranger]);
    assert!(logs.contains("Error"), "{logs}");
    let (a0, b0) = (env.token_amount(&desk_wx), env.token_amount(&desk_wsol));
    env.send_ok("damm_v2.claim_position_fee", &[ix_claim(&p, &desk.pubkey(), desk_wx, desk_wsol)], &[&desk]);
    let (fa, fb) = (env.token_amount(&desk_wx) - a0, env.token_amount(&desk_wsol) - b0);
    println!("4b. fees claimed by the NFT owner: {} wX base units, {} lamports (a stranger is refused)", fa, fb);
    assert_eq!(fa, 0, "fees accrue in SOL only (collect_fee_mode 1)");
    assert!(fb > 0);

    // ---- 5. a later graduation deepens the pool at the pool's current price; the new liquidity is locked too
    let s_now = pool_u128(&env, &p.pool, "sqrt_price");
    let add_sol = 10_625_000_000u64;
    let add_wx = (add_sol as f64 * 1e9 / price_of_sqrt(s_now)) as u64;
    mint_to(&mut env, &wx, &wrapper, &desk_wx, add_wx);
    let ixs = [
        anchor_lang::solana_program::system_instruction::transfer(&desk.pubkey(), &desk_wsol, add_sol),
        spl_token::instruction::sync_native(&spl_token::ID, &desk_wsol).unwrap(),
    ];
    env.send_ok("wrap", &ixs, &[&desk]);
    let dl = liquidity_for(add_wx, add_sol, s_now);
    env.send_ok("damm_v2.add_liquidity", &[ix_add(&p, &desk.pubkey(), desk_wx, desk_wsol, dl, add_wx, add_sol)], &[&desk]);
    env.send_ok("damm_v2.permanent_lock_position (the added liquidity)", &[ix_lock(&p, &desk.pubkey(), dl)], &[&desk]);
    let (u3, v3, l3) = position_liquidity(&env, &p.position());
    assert_eq!((u3, v3), (0, 0));
    assert_eq!(l3, l2 + dl);
    assert_eq!(pool_u128(&env, &p.pool, "liquidity"), l3, "the pool's whole liquidity is the locked position");
    println!("5. a second slice added {} liquidity at the pool's price and locked it: {} locked in all", dl, l3);

    // ---- depth of the full-range pool after two slices (≈ 21.25 SOL a side)
    let s_before = pool_u128(&env, &p.pool, "sqrt_price");
    env.send_ok("damm_v2.swap (buy wX with 1 SOL)", &[ix_swap(&p, &trader.pubkey(), t_wsol, t_wx, LAMPORTS)], &[&trader]);
    let moved = price_of_sqrt(pool_u128(&env, &p.pool, "sqrt_price")) / price_of_sqrt(s_before) - 1.0;
    println!("6. with ≈ 21.25 SOL a side, a 1 SOL buy moves the price {:+.2} % (full range: ±1 % ≈ 0.5 % of a side)", moved * 100.0);

    println!("\nCU (DAMM v2 alone, excluding the compute-budget instruction):");
    let mut v: Vec<_> = env.cu.iter().filter(|(k, _)| k.starts_with("damm_v2")).collect();
    v.sort();
    for (k, c) in v {
        println!("  {k:60} {c:>9}");
    }
    let _ = get_associated_token_address(&desk.pubkey(), &NATIVE_MINT);
}
