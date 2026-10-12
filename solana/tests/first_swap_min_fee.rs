//! The creator's launch buy at the minimum fee (operator decision 2026-10-11; `dbc::pins::FIRST_SWAP_WITH_MIN_FEE` in the canister),
//! proven on Meteora DBC's mainnet program bytes (tests/fixtures/dbc.so: sha256 4c26a8a5…f23b, DBC 0.2.1, the same bytes as
//! `solana program dump -um dbcij3LWUppWqq96dh6gJWwBifmcGfLSB5D4DuSMaqN` on 2026-10-11) in LiteSVM 0.10.
//!
//! A Rook-shaped config (the canister's `rook_wx_config`: the Q1 anti-sniper schedule 50 % → 1.000004 % over 60 × 10 s, quote = an
//! SPL "wX" with 9 decimals, T_wX = 5, the curve of tests/fixtures/s1_wx_curves.json) with `enable_first_swap_with_min_fee`:
//! - init + swap in ONE transaction, the Instructions sysvar as the swap's remaining account → the minimum fee (10,000,040 / 1e9);
//! - a second swap, in a later transaction at the same second, by another wallet (with or without the sysvar) → 50 %;
//! - init + swap in one transaction WITHOUT the sysvar → 50 %;
//! - init alone, then a first swap in a later transaction with the sysvar (a sniper on a fresh pool) → 50 %;
//! - the same config with the flag OFF (Rook's config before 2026-10-11): init + swap + sysvar in one transaction → 50 %.
//! Every fee is read from the pool: a buy's fee is taken from its input, so fee = amount in − Δ quote_reserve.
//! Run: `cargo test -p rook_backing --test first_swap_min_fee -- --nocapture`

#[path = "harness/mod.rs"]
mod harness;

use anchor_lang::{
    prelude::*,
    solana_program::instruction::Instruction,
    InstructionData, ToAccountMetas,
};
use anchor_spl::{associated_token::get_associated_token_address, token::spl_token};
use harness::*;
use rook_backing::{
    constants::*,
    dbc as dbcread,
    dynamic_bonding_curve::{self as dbc, types as dbct},
};
use solana_keypair::Keypair;
use solana_signer::Signer;

const T_WX: u64 = 5;
const INSTRUCTIONS_SYSVAR: Pubkey = pubkey!("Sysvar1nstructions1111111111111111111111111");
const BUY: u64 = LAMPORTS; // 1 wX
/// The scheduler's minimum (its value after the last period): 500,000,000 − 60 × 8,166,666.
const MIN_FEE_NUM: u64 = FEE_CLIFF - FEE_PERIODS as u64 * FEE_REDUCTION;

fn curve_for(t_wx: u64) -> (u128, Vec<(u128, u128)>) {
    let j: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/s1_wx_curves.json")).unwrap(),
    )
    .unwrap();
    let c = &j[t_wx.to_string()];
    let s = c["sqrtStartPrice"].as_str().unwrap().parse().unwrap();
    let curve = c["curve"].as_array().unwrap().iter().map(|p| (p["sqrtPrice"].as_str().unwrap().parse().unwrap(), p["liquidity"].as_str().unwrap().parse().unwrap())).collect();
    (s, curve)
}
/// The canister's `rook_wx_config(T_wX, …)`, with the first-swap flag as given.
fn wx_params(first_swap_min_fee: bool) -> dbct::ConfigParameters {
    let mut p = rook_config_params(50);
    let (s, curve) = curve_for(T_WX);
    p.migration_quote_threshold = T_WX * LAMPORTS;
    p.sqrt_start_price = s;
    p.curve = curve.iter().map(|(s, l)| dbct::LiquidityDistributionParameters { sqrt_price: *s, liquidity: *l }).collect();
    p.enable_first_swap_with_min_fee = first_swap_min_fee;
    p
}
fn ordered(a: &Pubkey, b: &Pubkey) -> (Pubkey, Pubkey) {
    if a > b {
        (*a, *b)
    } else {
        (*b, *a)
    }
}

struct Market {
    env: Env,
    wx: Pubkey,
    wx_auth: Keypair,
}
struct Coin {
    config: Pubkey,
    mint: Keypair,
    pool: Pubkey,
    base_vault: Pubkey,
    quote_vault: Pubkey,
}

impl Market {
    fn new() -> Market {
        let mut env = Env::new();
        let wx_auth = env.funded(1);
        let wx = env.create_mint(&wx_auth.pubkey(), 9);
        Market { env, wx, wx_auth }
    }
    fn config(&mut self, first_swap_min_fee: bool) -> Pubkey {
        let cfg = Keypair::new();
        let claimer = Pubkey::new_unique();
        let ix = Instruction {
            program_id: DBC_ID,
            accounts: dbc::client::accounts::CreateConfig {
                config: cfg.pubkey(),
                fee_claimer: claimer,
                leftover_receiver: claimer,
                quote_mint: self.wx,
                payer: self.env.payer.pubkey(),
                system_program: anchor_lang::system_program::ID,
                event_authority: event_authority(&DBC_ID),
                program: DBC_ID,
            }
            .to_account_metas(None),
            data: dbc::client::args::CreateConfig { config_parameters: wx_params(first_swap_min_fee) }.data(),
        };
        self.env.send_ok("create_config", &[ix], &[&cfg]);
        let d = self.env.raw(&cfg.pubkey());
        assert_eq!(d[365], first_swap_min_fee as u8, "PoolConfig byte 365 is enable_first_swap_with_min_fee (the canister's decode.rs reads it there)");
        cfg.pubkey()
    }
    /// A wallet holding `amount` wX, with its coin account ready.
    fn buyer(&mut self, coin: &Coin, amount: u64) -> Keypair {
        let k = self.env.funded(1);
        let wx_ata = self.env.ata(&k.pubkey(), &self.wx);
        self.env.ata(&k.pubkey(), &coin.mint.pubkey());
        let ix = spl_token::instruction::mint_to(&spl_token::ID, &self.wx, &wx_ata, &self.wx_auth.pubkey(), &[], amount).unwrap();
        let auth = self.wx_auth.insecure_clone();
        self.env.send_ok("mint wX", &[ix], &[&auth]);
        k
    }
    fn quote_reserve(&self, coin: &Coin) -> u64 {
        let a = self.env.svm.get_account(&coin.pool).unwrap();
        dbcread::parse_pool(&a.owner, &a.data).unwrap().quote_reserve
    }
}

fn coin_on(config: Pubkey, wx: &Pubkey) -> Coin {
    let mint = Keypair::new();
    let (hi, lo) = ordered(&mint.pubkey(), wx);
    let pool = pda(&[b"pool", config.as_ref(), hi.as_ref(), lo.as_ref()], &DBC_ID);
    Coin { config, base_vault: dbc_vault(&mint.pubkey(), &pool), quote_vault: dbc_vault(wx, &pool), pool, mint }
}

fn ix_init(c: &Coin, wx: &Pubkey, creator: &Pubkey, payer: &Pubkey) -> Instruction {
    Instruction {
        program_id: DBC_ID,
        accounts: dbc::client::accounts::InitializeVirtualPoolWithSplToken {
            config: c.config,
            pool_authority: DBC_POOL_AUTHORITY,
            creator: *creator,
            base_mint: c.mint.pubkey(),
            quote_mint: *wx,
            pool: c.pool,
            base_vault: c.base_vault,
            quote_vault: c.quote_vault,
            mint_metadata: pda(&[b"metadata", MPL_ID.as_ref(), c.mint.pubkey().as_ref()], &MPL_ID),
            metadata_program: MPL_ID,
            payer: *payer,
            token_quote_program: spl_token::ID,
            token_program: spl_token::ID,
            system_program: anchor_lang::system_program::ID,
            event_authority: event_authority(&DBC_ID),
            program: DBC_ID,
        }
        .to_account_metas(None),
        data: dbc::client::args::InitializeVirtualPoolWithSplToken {
            params: dbct::InitializePoolParameters { name: "Milady Coin".into(), symbol: "MILCOIN".into(), uri: "https://rook.example/c.json".into() },
        }
        .data(),
    }
}

/// DBC `swap` (v1, exact in: what the site's launch builds), wX → coin, optionally with the Instructions sysvar as remaining account.
fn ix_buy(c: &Coin, wx: &Pubkey, buyer: &Pubkey, amount: u64, with_sysvar: bool) -> Instruction {
    let mut accounts = dbc::client::accounts::Swap {
        pool_authority: DBC_POOL_AUTHORITY,
        config: c.config,
        pool: c.pool,
        input_token_account: get_associated_token_address(buyer, wx),
        output_token_account: get_associated_token_address(buyer, &c.mint.pubkey()),
        base_vault: c.base_vault,
        quote_vault: c.quote_vault,
        base_mint: c.mint.pubkey(),
        quote_mint: *wx,
        payer: *buyer,
        token_base_program: spl_token::ID,
        token_quote_program: spl_token::ID,
        referral_token_account: None,
        event_authority: event_authority(&DBC_ID),
        program: DBC_ID,
    }
    .to_account_metas(None);
    if with_sysvar {
        accounts.push(AccountMeta::new_readonly(INSTRUCTIONS_SYSVAR, false));
    }
    Instruction { program_id: DBC_ID, accounts, data: dbc::client::args::Swap { params: dbct::SwapParameters { amount_in: amount, minimum_amount_out: 0 } }.data() }
}

/// The fee DBC charges on an exact-in buy of `amount` at numerator `num` (fee taken from the input, rounded up).
fn fee_at(amount: u64, num: u64) -> u64 {
    ((amount as u128 * num as u128 + 999_999_999) / 1_000_000_000) as u64
}
fn pct(fee: u64, amount: u64) -> f64 {
    fee as f64 * 100.0 / amount as f64
}

/// The creator's launch: init + (wallet funded beforehand) swap in one transaction, signed by the creator, the mint and the payer.
fn launch_with_buy(m: &mut Market, config: Pubkey, with_sysvar: bool) -> (Coin, u64) {
    let coin = coin_on(config, &m.wx);
    // the creator's wX and coin accounts exist before the launch (the coin account's mint does not yet: create it in the tx)
    let creator = m.env.funded(2);
    let wx_ata = m.env.ata(&creator.pubkey(), &m.wx);
    let auth = m.wx_auth.insecure_clone();
    m.env.send_ok("mint wX", &[spl_token::instruction::mint_to(&spl_token::ID, &m.wx, &wx_ata, &auth.pubkey(), &[], BUY).unwrap()], &[&auth]);
    let coin_ata_ix = anchor_spl::associated_token::spl_associated_token_account::instruction::create_associated_token_account_idempotent(
        &creator.pubkey(),
        &creator.pubkey(),
        &coin.mint.pubkey(),
        &spl_token::ID,
    );
    let payer = m.env.payer.pubkey();
    let ixs = [ix_init(&coin, &m.wx, &creator.pubkey(), &payer), coin_ata_ix, ix_buy(&coin, &m.wx, &creator.pubkey(), BUY, with_sysvar)];
    let mint = coin.mint.insecure_clone();
    m.env.send_ok("launch + first buy", &ixs, &[&creator, &mint]);
    let fee = BUY - m.quote_reserve(&coin);
    (coin, fee)
}

#[test]
fn creator_first_buy_in_the_launch_transaction_pays_the_minimum_fee() {
    let mut m = Market::new();
    let on = m.config(true);
    assert_eq!(MIN_FEE_NUM, 10_000_040);

    // 1. init + swap + sysvar in one transaction: the minimum fee
    let (coin, fee) = launch_with_buy(&mut m, on, true);
    println!("launch + buy + sysvar (flag on): fee {fee} of {BUY} = {:.6} %", pct(fee, BUY));
    assert_eq!(fee, fee_at(BUY, MIN_FEE_NUM), "the creator's launch buy pays the scheduler's minimum, 1.000004 %");

    // 2. a second swap by another wallet, in a later transaction at the same second (period 0): the anti-sniper fee, sysvar or not
    for with_sysvar in [true, false] {
        let b = m.buyer(&coin, BUY);
        let before = m.quote_reserve(&coin);
        m.env.send_ok("second buy", &[ix_buy(&coin, &m.wx, &b.pubkey(), BUY, with_sysvar)], &[&b]);
        let fee = BUY - (m.quote_reserve(&coin) - before);
        println!("a later buy at t0 (sysvar {with_sysvar}): fee {fee} = {:.6} %", pct(fee, BUY));
        assert_eq!(fee, fee_at(BUY, FEE_CLIFF), "everyone else pays the anti-sniper fee, 50 % at t0");
    }

    // 3. init + swap in one transaction WITHOUT the sysvar: the anti-sniper fee
    let (_, fee) = launch_with_buy(&mut m, on, false);
    println!("launch + buy, no sysvar (flag on): fee {fee} = {:.6} %", pct(fee, BUY));
    assert_eq!(fee, fee_at(BUY, FEE_CLIFF));

    // 4. init alone, then the pool's first swap in a later transaction, with the sysvar (a sniper on a fresh pool): the anti-sniper fee
    let coin = coin_on(on, &m.wx);
    let creator = m.env.funded(2);
    let payer = m.env.payer.pubkey();
    let mint = coin.mint.insecure_clone();
    m.env.send_ok("launch alone", &[ix_init(&coin, &m.wx, &creator.pubkey(), &payer)], &[&creator, &mint]);
    let sniper = m.buyer(&coin, BUY);
    m.env.send_ok("sniper's first swap", &[ix_buy(&coin, &m.wx, &sniper.pubkey(), BUY, true)], &[&sniper]);
    let fee = BUY - m.quote_reserve(&coin);
    println!("first swap in a later tx + sysvar (flag on): fee {fee} = {:.6} %", pct(fee, BUY));
    assert_eq!(fee, fee_at(BUY, FEE_CLIFF), "only a swap in the pool's own creating transaction qualifies");

    // 5. the flag OFF (Rook's config before 2026-10-11): init + swap + sysvar in one transaction pays the anti-sniper fee
    let off = m.config(false);
    let (_, fee) = launch_with_buy(&mut m, off, true);
    println!("launch + buy + sysvar (flag off): fee {fee} = {:.6} %", pct(fee, BUY));
    assert_eq!(fee, fee_at(BUY, FEE_CLIFF));

    // 6. after the anti-sniper window a swap with the sysvar is an ordinary swap (DBC accepts it, the schedule's 1 % applies)
    let coin = coin_on(on, &m.wx);
    let creator = m.env.funded(2);
    let mint = coin.mint.insecure_clone();
    m.env.send_ok("launch alone", &[ix_init(&coin, &m.wx, &creator.pubkey(), &payer)], &[&creator, &mint]);
    m.env.warp(601);
    let b = m.buyer(&coin, BUY);
    m.env.send_ok("late buy + sysvar", &[ix_buy(&coin, &m.wx, &b.pubkey(), BUY, true)], &[&b]);
    let fee = BUY - m.quote_reserve(&coin);
    println!("a buy at t0 + 601 s + sysvar: fee {fee} = {:.6} %", pct(fee, BUY));
    assert_eq!(fee, fee_at(BUY, MIN_FEE_NUM));
}
