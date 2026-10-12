//! Spike S1 (+ the LiteSVM half of S2) for canister-only Rook on Solana (docs/v9/canister-only-solana.md §2, §3, §5, §15).
//! Mainnet program bytes in LiteSVM (DBC 0.2.1, DAMM v2 0.2.4, Metaplex Token Metadata; tests/fixtures). No Rook program is
//! used: ordinary keypairs stand in for the canister's threshold-Ed25519 keys (all on-curve, all signing like wallets):
//!   `canister`  = K_coll[i] (mint authority + metadata update authority of wX) — and, as the spike asks, also K_fc
//!                 (fee_claimer = leftover_receiver of the DBC config);
//!   `env.payer` = K_main (fee payer of every transaction);
//!   `desk`      = K_desk[i] (the wX/SOL pool's position owner);  `coin_owner` = K_coin[c] (the coin's backing vault ATA).
//! Run: `cargo test -p rook_backing --test s1_canister_only -- --nocapture --test-threads 1`
//! Write-up: docs/v9/spikes/s1-s3-canister-only.md.

#[path = "harness/mod.rs"]
mod harness;

use anchor_lang::{
    prelude::*,
    solana_program::{instruction::Instruction, system_instruction},
    InstructionData, ToAccountMetas,
};
use anchor_spl::{
    associated_token::{get_associated_token_address, spl_associated_token_account::instruction as ata_ix},
    token::spl_token,
};
use harness::*;
use rook_backing::{
    constants::*,
    cp_amm,
    dbc as dbcread,
    dynamic_bonding_curve::{self as dbc, types as dbct},
};
use solana_compute_budget_interface::ComputeBudgetInstruction;
use solana_keypair::Keypair;
use solana_message::Message;
use solana_signer::Signer;

const MEMO_ID: Pubkey = pubkey!("MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr");
const MIN_SQRT_PRICE: u128 = 4_295_048_016;
const MAX_SQRT_PRICE: u128 = 79_226_673_521_066_979_257_578_248_091;
const TWO64: f64 = 18_446_744_073_709_551_616f64;

// ------------------------------------------------------------------------------------------------- curves
/// The curves of tests/fixtures/s1_wx_curves.json (tests/q1/design_curve.mjs 1e9 20 <T_wX> 50 6, quote decimals 9 = wX's).
fn curve_for(t_wx: u64) -> (u128, Vec<(u128, u128)>) {
    let j: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/s1_wx_curves.json")).unwrap(),
    )
    .unwrap();
    let c = &j[t_wx.to_string()];
    assert!(!c.is_null(), "no curve for T_wX = {t_wx}");
    let s = c["sqrtStartPrice"].as_str().unwrap().parse().unwrap();
    let curve = c["curve"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| (p["sqrtPrice"].as_str().unwrap().parse().unwrap(), p["liquidity"].as_str().unwrap().parse().unwrap()))
        .collect();
    (s, curve)
}

/// Rook's config (harness::rook_config_params: Q1 anti-sniper schedule, fixed 1B supply, LP locked 50/50, 50 % migration
/// fee, DAMM v2 Customizable) with the threshold and curve in wX.
fn wx_params(t_wx: u64) -> dbct::ConfigParameters {
    let mut p = rook_config_params(50);
    let (s, curve) = curve_for(t_wx);
    p.migration_quote_threshold = t_wx * LAMPORTS; // wX has 9 decimals like SOL
    p.sqrt_start_price = s;
    p.curve = curve.iter().map(|(s, l)| dbct::LiquidityDistributionParameters { sqrt_price: *s, liquidity: *l }).collect();
    p
}

// ------------------------------------------------------------------------------------------------- PDAs (any quote)
fn ordered(a: &Pubkey, b: &Pubkey) -> (Pubkey, Pubkey) {
    if a > b {
        (*a, *b)
    } else {
        (*b, *a)
    }
}
fn dbc_pool_q(config: &Pubkey, base: &Pubkey, quote: &Pubkey) -> Pubkey {
    let (hi, lo) = ordered(base, quote);
    pda(&[b"pool", config.as_ref(), hi.as_ref(), lo.as_ref()], &DBC_ID)
}
fn damm_mig_pool(base: &Pubkey, quote: &Pubkey) -> Pubkey {
    let (hi, lo) = ordered(base, quote);
    pda(&[b"pool", DAMM_CONFIG_CUSTOMIZABLE.as_ref(), hi.as_ref(), lo.as_ref()], &DAMM_V2_ID)
}
fn cpool(a: &Pubkey, b: &Pubkey) -> Pubkey {
    let (hi, lo) = ordered(a, b);
    pda(&[b"cpool", hi.as_ref(), lo.as_ref()], &DAMM_V2_ID)
}
fn metadata_pda(mint: &Pubkey) -> Pubkey {
    pda(&[b"metadata", MPL_ID.as_ref(), mint.as_ref()], &MPL_ID)
}

// ------------------------------------------------------------------------------------------------- Metaplex (by hand)
fn borsh_str(v: &mut Vec<u8>, s: &str) {
    v.extend_from_slice(&(s.len() as u32).to_le_bytes());
    v.extend_from_slice(s.as_bytes());
}
/// Token Metadata `CreateMetadataAccountV3` (discriminator 33): no creators, 0 royalty, mutable, update authority named
/// (not a signer).
fn ix_create_metadata_v3(mint: &Pubkey, mint_authority: &Pubkey, payer: &Pubkey, update_authority: &Pubkey, name: &str, symbol: &str, uri: &str) -> Instruction {
    let mut d = vec![33u8];
    borsh_str(&mut d, name);
    borsh_str(&mut d, symbol);
    borsh_str(&mut d, uri);
    d.extend_from_slice(&0u16.to_le_bytes()); // seller_fee_basis_points
    d.extend_from_slice(&[0, 0, 0]); // creators, collection, uses: None
    d.push(1); // is_mutable
    d.push(0); // collection_details: None
    Instruction {
        program_id: MPL_ID,
        accounts: vec![
            AccountMeta::new(metadata_pda(mint), false),
            AccountMeta::new_readonly(*mint, false),
            AccountMeta::new_readonly(*mint_authority, true),
            AccountMeta::new(*payer, true),
            AccountMeta::new_readonly(*update_authority, false),
            AccountMeta::new_readonly(anchor_lang::system_program::ID, false),
        ],
        data: d,
    }
}
fn memo(text: &str) -> Instruction {
    Instruction { program_id: MEMO_ID, accounts: vec![], data: text.as_bytes().to_vec() }
}

// ------------------------------------------------------------------------------------------------- wX
struct Wx {
    mint: Pubkey,
    /// K_coll[i]: mint authority, metadata update authority; here also K_fc
    canister: Keypair,
}

impl Wx {
    fn mint_to(&self, env: &mut Env, owner: &Pubkey, amount: u64) -> Pubkey {
        let ata = env.ata(owner, &self.mint);
        let ixs = [
            spl_token::instruction::mint_to_checked(&spl_token::ID, &self.mint, &ata, &self.canister.pubkey(), &[], amount, 9).unwrap(),
            memo("rook:spike:mint"),
        ];
        env.send_ok("spl.mint_to_checked + memo (K_main + K_coll)", &ixs, &[&self.canister]);
        ata
    }
}

/// The ceremony of §2: a hot key creates the mint with itself as authority, the metadata with update authority = canister,
/// then hands mint authority to the canister — one transaction. Returns (wX, metadata lamports, tx size).
fn ceremony_wx(env: &mut Env, canister: Keypair, symbol: &str, freeze: Option<&Pubkey>) -> (Wx, u64, usize) {
    let hot = env.funded(1);
    let mint = Keypair::new();
    let rent = env.svm.minimum_balance_for_rent_exemption(82);
    let ixs = vec![
        system_instruction::create_account(&env.payer.pubkey(), &mint.pubkey(), rent, 82, &spl_token::ID),
        spl_token::instruction::initialize_mint2(&spl_token::ID, &mint.pubkey(), &hot.pubkey(), freeze, 9).unwrap(),
        ix_create_metadata_v3(&mint.pubkey(), &hot.pubkey(), &env.payer.pubkey(), &canister.pubkey(), &format!("Wrapped {symbol}"), symbol, "https://rook.example/wx.json"),
        spl_token::instruction::set_authority(&spl_token::ID, &mint.pubkey(), Some(&canister.pubkey()), spl_token::instruction::AuthorityType::MintTokens, &hot.pubkey(), &[])
            .unwrap(),
    ];
    let size = tx_size(&env.payer.pubkey(), &ixs);
    env.send_ok("ceremony: create wX + metadata + set_authority", &ixs, &[&hot, &mint]);
    let md = env.svm.get_account(&metadata_pda(&mint.pubkey())).unwrap();
    (Wx { mint: mint.pubkey(), canister }, md.lamports, size)
}

/// Wire size of a legacy transaction with these instructions plus a CU limit + CU price (1 + 64·sigs + message).
fn tx_size(payer: &Pubkey, ixs: &[Instruction]) -> usize {
    let mut all = vec![ComputeBudgetInstruction::set_compute_unit_limit(1_400_000), ComputeBudgetInstruction::set_compute_unit_price(1)];
    all.extend_from_slice(ixs);
    let m = Message::new(&all, Some(payer));
    let n = m.header.num_required_signatures as usize;
    1 + 64 * n + m.serialize().len()
}
fn sigs_of(payer: &Pubkey, ixs: &[Instruction]) -> usize {
    Message::new(ixs, Some(payer)).header.num_required_signatures as usize
}

// ------------------------------------------------------------------------------------------------- DBC with a wX quote
struct WxCoin {
    config: Pubkey,
    mint: Pubkey,
    pool: Pubkey,
    base_vault: Pubkey,
    quote_vault: Pubkey,
    quote: Pubkey,
    creator: Keypair,
}

fn ix_create_config_q(payer: &Pubkey, config: &Pubkey, fee_claimer: &Pubkey, leftover: &Pubkey, quote: &Pubkey, params: dbct::ConfigParameters) -> Instruction {
    Instruction {
        program_id: DBC_ID,
        accounts: dbc::client::accounts::CreateConfig {
            config: *config,
            fee_claimer: *fee_claimer,
            leftover_receiver: *leftover,
            quote_mint: *quote,
            payer: *payer,
            system_program: anchor_lang::system_program::ID,
            event_authority: event_authority(&DBC_ID),
            program: DBC_ID,
        }
        .to_account_metas(None),
        data: dbc::client::args::CreateConfig { config_parameters: params }.data(),
    }
}

fn try_config(env: &mut Env, claimer: &Pubkey, quote: &Pubkey, params: dbct::ConfigParameters) -> std::result::Result<Pubkey, String> {
    let config = Keypair::new();
    let ix = ix_create_config_q(&env.payer.pubkey(), &config.pubkey(), claimer, claimer, quote, params);
    match env.send(&[ix], &[&config]) {
        Ok(m) => {
            env.cu.insert("dbc.create_config".into(), m.compute_units_consumed - 150);
            Ok(config.pubkey())
        }
        Err(e) => Err(e.meta.logs.iter().filter(|l| l.contains("Error") || l.contains("failed")).cloned().collect::<Vec<_>>().join(" | ")),
    }
}

fn launch_q(env: &mut Env, config: &Pubkey, quote: &Pubkey) -> WxCoin {
    let creator = env.funded(10);
    let mint = Keypair::new();
    let pool = dbc_pool_q(config, &mint.pubkey(), quote);
    let base_vault = dbc_vault(&mint.pubkey(), &pool);
    let quote_vault = dbc_vault(quote, &pool);
    let ix = Instruction {
        program_id: DBC_ID,
        accounts: dbc::client::accounts::InitializeVirtualPoolWithSplToken {
            config: *config,
            pool_authority: DBC_POOL_AUTHORITY,
            creator: creator.pubkey(),
            base_mint: mint.pubkey(),
            quote_mint: *quote,
            pool,
            base_vault,
            quote_vault,
            mint_metadata: metadata_pda(&mint.pubkey()),
            metadata_program: MPL_ID,
            payer: env.payer.pubkey(),
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
    };
    env.send_ok("dbc.initialize_virtual_pool_with_spl_token (wX quote)", &[ix], &[&creator, &mint]);
    WxCoin { config: *config, mint: mint.pubkey(), pool, base_vault, quote_vault, quote: *quote, creator }
}

/// DBC swap2, quote (wX) → coin. mode 0 ExactIn, 1 PartialFill.
fn ix_dbc_buy(c: &WxCoin, buyer: &Pubkey, input: Pubkey, output: Pubkey, amount: u64, min_out: u64, mode: u8) -> Instruction {
    Instruction {
        program_id: DBC_ID,
        accounts: dbc::client::accounts::Swap2 {
            pool_authority: DBC_POOL_AUTHORITY,
            config: c.config,
            pool: c.pool,
            input_token_account: input,
            output_token_account: output,
            base_vault: c.base_vault,
            quote_vault: c.quote_vault,
            base_mint: c.mint,
            quote_mint: c.quote,
            payer: *buyer,
            token_base_program: spl_token::ID,
            token_quote_program: spl_token::ID,
            referral_token_account: None,
            event_authority: event_authority(&DBC_ID),
            program: DBC_ID,
        }
        .to_account_metas(None),
        data: dbc::client::args::Swap2 { params: dbct::SwapParameters2 { amount_0: amount, amount_1: min_out, swap_mode: mode } }.data(),
    }
}

struct Mig {
    pool: Pubkey,
    nft1: Pubkey,
    nft2: Pubkey,
}

fn migrate_q(env: &mut Env, c: &WxCoin) -> std::result::Result<Mig, String> {
    let damm_pool = damm_mig_pool(&c.mint, &c.quote);
    let nft1 = Keypair::new();
    let nft2 = Keypair::new();
    let anyone = env.funded(2);
    let mut metas = dbc::client::accounts::MigrationDammV2 {
        virtual_pool: c.pool,
        migration_metadata: anchor_lang::system_program::ID,
        config: c.config,
        pool_authority: DBC_POOL_AUTHORITY,
        pool: damm_pool,
        first_position_nft_mint: nft1.pubkey(),
        first_position_nft_account: damm_position_nft_account(&nft1.pubkey()),
        first_position: damm_position(&nft1.pubkey()),
        second_position_nft_mint: Some(nft2.pubkey()),
        second_position_nft_account: Some(damm_position_nft_account(&nft2.pubkey())),
        second_position: Some(damm_position(&nft2.pubkey())),
        damm_pool_authority: DAMM_V2_POOL_AUTHORITY,
        amm_program: DAMM_V2_ID,
        base_mint: c.mint,
        quote_mint: c.quote,
        token_a_vault: damm_vault(&c.mint, &damm_pool),
        token_b_vault: damm_vault(&c.quote, &damm_pool),
        base_vault: c.base_vault,
        quote_vault: c.quote_vault,
        payer: anyone.pubkey(),
        token_base_program: spl_token::ID,
        token_quote_program: spl_token::ID,
        token_2022_program: TOKEN_2022_ID,
        damm_event_authority: event_authority(&DAMM_V2_ID),
        system_program: anchor_lang::system_program::ID,
    }
    .to_account_metas(None);
    metas.push(AccountMeta::new_readonly(DAMM_CONFIG_CUSTOMIZABLE, false));
    let ix = Instruction { program_id: DBC_ID, accounts: metas, data: dbc::client::args::MigrationDammV2 {}.data() };
    match env.send(&[ix], &[&anyone, &nft1, &nft2]) {
        Ok(m) => {
            env.cu.insert("dbc.migration_damm_v2 (wX quote)".into(), m.compute_units_consumed - 150);
            Ok(Mig { pool: damm_pool, nft1: nft1.pubkey(), nft2: nft2.pubkey() })
        }
        Err(e) => Err(e.meta.logs.join("\n")),
    }
}

fn ix_withdraw_migration_fee(c: &WxCoin, sender: &Pubkey, dest: Pubkey, flag: u8) -> Instruction {
    Instruction {
        program_id: DBC_ID,
        accounts: dbc::client::accounts::WithdrawMigrationFee {
            pool_authority: DBC_POOL_AUTHORITY,
            config: c.config,
            virtual_pool: c.pool,
            token_quote_account: dest,
            quote_vault: c.quote_vault,
            quote_mint: c.quote,
            sender: *sender,
            token_quote_program: spl_token::ID,
            event_authority: event_authority(&DBC_ID),
            program: DBC_ID,
        }
        .to_account_metas(None),
        data: dbc::client::args::WithdrawMigrationFee { flag }.data(),
    }
}
fn ix_claim_trading_fee(c: &WxCoin, claimer: &Pubkey, base_dest: Pubkey, quote_dest: Pubkey) -> Instruction {
    Instruction {
        program_id: DBC_ID,
        accounts: dbc::client::accounts::ClaimTradingFee {
            pool_authority: DBC_POOL_AUTHORITY,
            config: c.config,
            pool: c.pool,
            token_a_account: base_dest,
            token_b_account: quote_dest,
            base_vault: c.base_vault,
            quote_vault: c.quote_vault,
            base_mint: c.mint,
            quote_mint: c.quote,
            fee_claimer: *claimer,
            token_base_program: spl_token::ID,
            token_quote_program: spl_token::ID,
            event_authority: event_authority(&DBC_ID),
            program: DBC_ID,
        }
        .to_account_metas(None),
        data: dbc::client::args::ClaimTradingFee { max_amount_a: u64::MAX, max_amount_b: u64::MAX }.data(),
    }
}
fn ix_withdraw_leftover(c: &WxCoin, receiver: &Pubkey, dest: Pubkey) -> Instruction {
    Instruction {
        program_id: DBC_ID,
        accounts: dbc::client::accounts::WithdrawLeftover {
            pool_authority: DBC_POOL_AUTHORITY,
            config: c.config,
            virtual_pool: c.pool,
            token_base_account: dest,
            base_vault: c.base_vault,
            base_mint: c.mint,
            leftover_receiver: *receiver,
            token_base_program: spl_token::ID,
            event_authority: event_authority(&DBC_ID),
            program: DBC_ID,
        }
        .to_account_metas(None),
        data: dbc::client::args::WithdrawLeftover {}.data(),
    }
}
fn ix_partner_surplus(c: &WxCoin, claimer: &Pubkey, dest: Pubkey) -> Instruction {
    Instruction {
        program_id: DBC_ID,
        accounts: dbc::client::accounts::PartnerWithdrawSurplus {
            pool_authority: DBC_POOL_AUTHORITY,
            config: c.config,
            virtual_pool: c.pool,
            token_quote_account: dest,
            quote_vault: c.quote_vault,
            quote_mint: c.quote,
            fee_claimer: *claimer,
            token_quote_program: spl_token::ID,
            event_authority: event_authority(&DBC_ID),
            program: DBC_ID,
        }
        .to_account_metas(None),
        data: dbc::client::args::PartnerWithdrawSurplus {}.data(),
    }
}

// ------------------------------------------------------------------------------------------------- DAMM v2 (any pair)
#[derive(Clone, Copy)]
struct Damm {
    pool: Pubkey,
    a: Pubkey,
    b: Pubkey,
}
impl Damm {
    fn va(&self) -> Pubkey {
        damm_vault(&self.a, &self.pool)
    }
    fn vb(&self) -> Pubkey {
        damm_vault(&self.b, &self.pool)
    }
}
fn ix_damm_swap(p: &Damm, trader: &Pubkey, input: Pubkey, output: Pubkey, amount_in: u64, min_out: u64) -> Instruction {
    Instruction {
        program_id: DAMM_V2_ID,
        accounts: cp_amm::client::accounts::Swap {
            pool_authority: DAMM_V2_POOL_AUTHORITY,
            pool: p.pool,
            input_token_account: input,
            output_token_account: output,
            token_a_vault: p.va(),
            token_b_vault: p.vb(),
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
fn ix_damm_claim(p: &Damm, nft: &Pubkey, owner: &Pubkey, a_acc: Pubkey, b_acc: Pubkey) -> Instruction {
    Instruction {
        program_id: DAMM_V2_ID,
        accounts: cp_amm::client::accounts::ClaimPositionFee {
            pool_authority: DAMM_V2_POOL_AUTHORITY,
            pool: p.pool,
            position: damm_position(nft),
            token_a_account: a_acc,
            token_b_account: b_acc,
            token_a_vault: p.va(),
            token_b_vault: p.vb(),
            token_a_mint: p.a,
            token_b_mint: p.b,
            position_nft_account: damm_position_nft_account(nft),
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
fn ix_damm_add(p: &Damm, nft: &Pubkey, owner: &Pubkey, a: Pubkey, b: Pubkey, liq: u128, max_a: u64, max_b: u64) -> Instruction {
    Instruction {
        program_id: DAMM_V2_ID,
        accounts: cp_amm::client::accounts::AddLiquidity {
            pool: p.pool,
            position: damm_position(nft),
            token_a_account: a,
            token_b_account: b,
            token_a_vault: p.va(),
            token_b_vault: p.vb(),
            token_a_mint: p.a,
            token_b_mint: p.b,
            position_nft_account: damm_position_nft_account(nft),
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
fn ix_damm_remove(p: &Damm, nft: &Pubkey, owner: &Pubkey, a: Pubkey, b: Pubkey, liq: Option<u128>) -> Instruction {
    let accounts = cp_amm::client::accounts::RemoveLiquidity {
        pool_authority: DAMM_V2_POOL_AUTHORITY,
        pool: p.pool,
        position: damm_position(nft),
        token_a_account: a,
        token_b_account: b,
        token_a_vault: p.va(),
        token_b_vault: p.vb(),
        token_a_mint: p.a,
        token_b_mint: p.b,
        position_nft_account: damm_position_nft_account(nft),
        signer: *owner,
        token_a_program: spl_token::ID,
        token_b_program: spl_token::ID,
        event_authority: event_authority(&DAMM_V2_ID),
        program: DAMM_V2_ID,
    }
    .to_account_metas(None);
    let data = match liq {
        Some(l) => cp_amm::client::args::RemoveLiquidity {
            params: cp_amm::types::RemoveLiquidityParameters { liquidity_delta: l, token_a_amount_threshold: 0, token_b_amount_threshold: 0 },
        }
        .data(),
        None => cp_amm::client::args::RemoveAllLiquidity { token_a_amount_threshold: 0, token_b_amount_threshold: 0 }.data(),
    };
    Instruction { program_id: DAMM_V2_ID, accounts, data }
}
fn ix_close_position(p: &Damm, nft: &Pubkey, owner: &Pubkey, rent_to: &Pubkey) -> Instruction {
    Instruction {
        program_id: DAMM_V2_ID,
        accounts: cp_amm::client::accounts::ClosePosition {
            position_nft_mint: *nft,
            position_nft_account: damm_position_nft_account(nft),
            pool: p.pool,
            position: damm_position(nft),
            pool_authority: DAMM_V2_POOL_AUTHORITY,
            rent_receiver: *rent_to,
            owner: *owner,
            token_program: TOKEN_2022_ID,
            event_authority: event_authority(&DAMM_V2_ID),
            program: DAMM_V2_ID,
        }
        .to_account_metas(None),
        data: cp_amm::client::args::ClosePosition {}.data(),
    }
}

fn base_fee(num: u64) -> cp_amm::types::BaseFeeParameters {
    let mut d = [0u8; 27];
    d[..8].copy_from_slice(&num.to_le_bytes());
    cp_amm::types::BaseFeeParameters { data: d }
}
/// Liquidity of a concentrated position holding at most `a` (token a) and `b` (token b) at sqrt price `s` in [smin, smax].
fn liquidity_for(a: u64, b: u64, s: u128, smin: u128, smax: u128) -> u128 {
    let sp = s as f64 / TWO64;
    let lo = smin as f64 / TWO64;
    let hi = smax as f64 / TWO64;
    let la = a as f64 / (1.0 / sp - 1.0 / hi);
    let lb = b as f64 / (sp - lo);
    (la.min(lb) * 0.999_999 * TWO64) as u128
}
fn sqrt_of(lamports_per_whole: f64) -> u128 {
    ((lamports_per_whole / 1e9).sqrt() * TWO64) as u128
}
fn price_of(s: u128) -> f64 {
    let r = s as f64 / TWO64;
    r * r // lamports per base unit = SOL per whole wX
}
/// sqrt_price in a cp_amm 0.2.4 `Pool`: the offset is learned by value at pool creation.
static SQRT_OFF: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
fn pool_sqrt(env: &Env, pool: &Pubkey) -> u128 {
    let d = env.raw(pool);
    let o = *SQRT_OFF.get().unwrap();
    u128::from_le_bytes(d[o..o + 16].try_into().unwrap())
}
fn position_liq(env: &Env, nft: &Pubkey) -> (u128, u128, u128) {
    let d = env.raw(&damm_position(nft));
    let r = |o: usize| u128::from_le_bytes(d[o..o + 16].try_into().unwrap());
    (r(152), r(168), r(184))
}

// ================================================================================================= S1a: wX + DBC
#[test]
fn s1a_dbc_with_wx_quote() {
    for t_wx in [5u64, 174] {
        dbc_flow(t_wx);
    }
}

fn dbc_flow(t_wx: u64) {
    println!("\n==================== S1a: DBC with a wX quote, T_wX = {t_wx} ====================");
    let mut env = Env::new();
    let canister = Keypair::new();
    env.svm.airdrop(&canister.pubkey(), LAMPORTS).unwrap(); // K_fc holds no SOL in the design; only for ATA-less paths
    assert!(canister.pubkey().is_on_curve());
    let (wx, md_lamports, size) = ceremony_wx(&mut env, canister, "wMILADY", None);
    let m = spl_token::state::Mint::unpack_from_slice(&env.raw(&wx.mint)[..82]).unwrap();
    let md = env.raw(&metadata_pda(&wx.mint));
    let md_update_auth = Pubkey::new_from_array(md[1..33].try_into().unwrap());
    println!(
        "ceremony (1 tx, {size} B): mint authority {} (= canister {}), freeze {:?}, supply {}, decimals {}, metadata update authority = canister: {}, metadata account {} lamports",
        m.mint_authority.unwrap(),
        m.mint_authority.unwrap() == wx.canister.pubkey(),
        m.freeze_authority,
        m.supply,
        m.decimals,
        md_update_auth == wx.canister.pubkey(),
        md_lamports
    );
    assert_eq!(m.mint_authority, solana_program_option(wx.canister.pubkey()));
    assert!(m.freeze_authority.is_none());
    assert_eq!(md_update_auth, wx.canister.pubkey());

    // config: quote = wX (live mint authority), fee_claimer = leftover_receiver = the canister key (on curve)
    let k_fc = wx.canister.pubkey();
    let config = try_config(&mut env, &k_fc, &wx.mint, wx_params(t_wx)).expect("create_config with quote = wX");
    let a = env.svm.get_account(&config).unwrap();
    let cfg = dbcread::parse_config(&a.owner, &a.data).unwrap();
    println!(
        "create_config OK: quote {} = wX, fee_claimer = leftover_receiver = K_fc: {}, threshold {} wX, collect_fee_mode {}, migrated collect_fee_mode {}, fixed supply {}, quote_token_flag byte {}",
        cfg.quote_mint,
        cfg.fee_claimer == k_fc && cfg.leftover_receiver == k_fc,
        cfg.migration_quote_threshold as f64 / 1e9,
        cfg.collect_fee_mode,
        cfg.migrated_collect_fee_mode,
        cfg.fixed_token_supply_flag,
        quote_token_flag(&env, &config)
    );
    assert_eq!(cfg.quote_mint, wx.mint);

    // launch, anti-sniper window, buy to graduation with wX
    let coin = launch_q(&mut env, &config, &wx.mint);
    env.warp(700);
    let whale = env.funded(5);
    let whale_wx = wx.mint_to(&mut env, &whale.pubkey(), 2 * t_wx * LAMPORTS);
    let whale_coin = env.ata(&whale.pubkey(), &coin.mint);
    // a small first buy (fees accrue), then a partial fill to the threshold
    let ix = ix_dbc_buy(&coin, &whale.pubkey(), whale_wx, whale_coin, LAMPORTS / 10, 0, 0);
    env.send_ok("dbc.swap2 wX → coin (exact in)", &[ix], &[&whale]);
    let ix = ix_dbc_buy(&coin, &whale.pubkey(), whale_wx, whale_coin, 2 * t_wx * LAMPORTS, 0, 1);
    env.send_ok("dbc.swap2 wX → coin (partial fill to threshold)", &[ix], &[&whale]);
    let spent = 2 * t_wx * LAMPORTS - env.token_amount(&whale_wx);
    let p = pool_of(&env, &coin.pool);
    println!(
        "bought to graduation: spent {:.6} wX, quote_reserve {:.6} wX (threshold {}), migration_progress {}, coins {}",
        spent as f64 / 1e9,
        p.quote_reserve as f64 / 1e9,
        t_wx,
        p.migration_progress,
        env.token_amount(&whale_coin)
    );
    assert!(p.quote_reserve >= t_wx * LAMPORTS);

    // curve trading fees (partner share) → K_fc, in wX; claimed before migration
    let fc_wx = env.ata(&k_fc, &wx.mint);
    let fc_coin = env.ata(&k_fc, &coin.mint);
    let ix = ix_claim_trading_fee(&coin, &k_fc, fc_coin, fc_wx);
    env.send_ok("dbc.claim_trading_fee (K_main + K_fc)", &[ix], &[&wx.canister]);
    let fee_wx = env.token_amount(&fc_wx);
    let fee_coin = env.token_amount(&fc_coin);
    println!("claim_trading_fee as K_fc: {:.9} wX, {} coin base units (fees in quote = wX)", fee_wx as f64 / 1e9, fee_coin);
    assert!(fee_wx > 0 && fee_coin == 0);

    // a stranger cannot claim
    let stranger = env.funded(1);
    let st_wx = env.ata(&stranger.pubkey(), &wx.mint);
    let st_coin = env.ata(&stranger.pubkey(), &coin.mint);
    let logs = env.send_err("claim_trading_fee by a stranger", &[ix_claim_trading_fee(&coin, &stranger.pubkey(), st_coin, st_wx)], &[&stranger]);
    println!("  stranger's claim_trading_fee refused: {}", first_err(&logs));

    // migrate to DAMM v2 COIN/wX (permissionless)
    let mig = migrate_q(&mut env, &coin).unwrap_or_else(|l| panic!("migration failed:\n{l}"));
    let p = pool_of(&env, &coin.pool);
    assert_eq!(p.is_migrated, 1);
    let damm = Damm { pool: mig.pool, a: coin.mint, b: wx.mint };
    let o1 = env.token_owner(&damm_position_nft_account(&mig.nft1));
    let o2 = env.token_owner(&damm_position_nft_account(&mig.nft2));
    let partner_nft = if o1 == k_fc { mig.nft1 } else { mig.nft2 };
    assert!(o1 == k_fc || o2 == k_fc, "a position NFT goes to the fee_claimer");
    let (u, v, l) = position_liq(&env, &partner_nft);
    println!(
        "migrated → DAMM v2 COIN/wX {}: vaults {} coin / {:.6} wX; partner position NFT owner = K_fc: {}; partner LP unlocked {u} vested {v} locked {l}",
        mig.pool,
        env.token_amount(&damm.va()),
        env.token_amount(&damm.vb()) as f64 / 1e9,
        true
    );
    assert!(u <= 1 && v == 0 && l > 0, "partner LP permanently locked");

    // withdraw_migration_fee (partner, flag 0) into a THIRD-PARTY ATA: K_coin[c]'s wX ATA
    let coin_owner = Keypair::new();
    let vault_wx = env.ata(&coin_owner.pubkey(), &wx.mint);
    let logs = env.send_err("withdraw_migration_fee by a stranger", &[ix_withdraw_migration_fee(&coin, &stranger.pubkey(), st_wx, 0)], &[&stranger]);
    println!("  stranger's withdraw_migration_fee refused: {}", first_err(&logs));
    let ixs = [ix_withdraw_migration_fee(&coin, &k_fc, vault_wx, 0), memo("rook:0:back:coin")];
    let sz = tx_size(&env.payer.pubkey(), &[ata_ix::create_associated_token_account_idempotent(&env.payer.pubkey(), &coin_owner.pubkey(), &wx.mint, &spl_token::ID), ixs[0].clone(), ixs[1].clone()]);
    env.send_ok("dbc.withdraw_migration_fee → K_coin ATA + memo (K_main + K_fc)", &ixs, &[&wx.canister]);
    let mf = env.token_amount(&vault_wx);
    println!(
        "withdraw_migration_fee(flag 0) signed by K_fc into K_coin's ATA: {:.6} wX (= 50 % of {} wX: {}), tx with idempotent ATA + memo {} B",
        mf as f64 / 1e9,
        t_wx,
        mf == t_wx * LAMPORTS / 2,
        sz
    );
    assert_eq!(mf, t_wx * LAMPORTS / 2);
    let logs = env.send_err("withdraw_migration_fee twice", &[ix_withdraw_migration_fee(&coin, &k_fc, vault_wx, 0)], &[&wx.canister]);
    println!("  second withdraw_migration_fee refused: {}", first_err(&logs));

    // partner surplus (quote above the threshold left by the partial fill)
    let before = env.token_amount(&fc_wx);
    match env.send(&[ix_partner_surplus(&coin, &k_fc, fc_wx)], &[&wx.canister]) {
        Ok(_) => println!("partner_withdraw_surplus: {:.9} wX", (env.token_amount(&fc_wx) - before) as f64 / 1e9),
        Err(e) => println!("partner_withdraw_surplus refused: {}", first_err(&e.meta.logs.join("\n"))),
    }
    // leftover base (unsold curve coins) → leftover_receiver (K_fc), then burned
    let ix = ix_withdraw_leftover(&coin, &k_fc, fc_coin);
    match env.send(&[ix], &[]) {
        Ok(_) => {
            let left = env.token_amount(&fc_coin);
            println!("withdraw_leftover (permissionless, no K_fc signature): {} coin base units ({:.3} whole coins) → K_fc", left, left as f64 / 1e6);
            if left > 0 {
                let ix = spl_token::instruction::burn_checked(&spl_token::ID, &fc_coin, &coin.mint, &k_fc, &[], left, 6).unwrap();
                env.send_ok("spl.burn_checked leftover coins (K_main + K_fc)", &[ix], &[&wx.canister]);
                println!("  burned by K_fc; coin supply now {}", env.mint_supply(&coin.mint));
            }
        }
        Err(e) => println!("withdraw_leftover refused: {}", first_err(&e.meta.logs.join("\n"))),
    }

    // trade on the graduated COIN/wX pool both ways, then the partner position's fees
    let trader = env.funded(2);
    let t_wx_acc = wx.mint_to(&mut env, &trader.pubkey(), LAMPORTS);
    let t_coin = env.ata(&trader.pubkey(), &coin.mint);
    env.send_ok("damm_v2.swap wX → coin (graduated)", &[ix_damm_swap(&damm, &trader.pubkey(), t_wx_acc, t_coin, LAMPORTS / 2, 0)], &[&trader]);
    let got = env.token_amount(&t_coin);
    env.send_ok("damm_v2.swap coin → wX (graduated)", &[ix_damm_swap(&damm, &trader.pubkey(), t_coin, t_wx_acc, got / 2, 0)], &[&trader]);
    let (bw, bc) = (env.token_amount(&fc_wx), env.token_amount(&fc_coin));
    env.send_ok("damm_v2.claim_position_fee partner (K_main + K_fc)", &[ix_damm_claim(&damm, &partner_nft, &k_fc, fc_coin, fc_wx)], &[&wx.canister]);
    let (lw, lc) = (env.token_amount(&fc_wx) - bw, env.token_amount(&fc_coin) - bc);
    println!("claim_position_fee on the partner NFT (0.5 wX buy + a sell): {:.9} wX, {} coin base units", lw as f64 / 1e9, lc);
    assert!(lw > 0);
    let logs = env.send_err("claim_position_fee by a stranger", &[ix_damm_claim(&damm, &partner_nft, &stranger.pubkey(), st_coin, st_wx)], &[&stranger]);
    println!("  stranger's claim_position_fee refused: {}", first_err(&logs));

    print_cu(&env, "S1a");
}

// ================================================================================================= S1b: refusals / bounds
#[test]
fn s1b_dbc_bounds() {
    println!("\n==================== S1b: what DBC accepts and refuses with a wX-like quote ====================");
    let mut env = Env::new();
    let canister = Keypair::new();
    let k = canister.pubkey();
    let (wx, _, _) = ceremony_wx(&mut env, canister, "wMILADY", None);

    // every T_wX of the design's table (§3.2)
    let mut ok = vec![];
    for t in [3u64, 4, 5, 7, 9, 10, 11, 12, 13, 15, 21, 22, 25, 26, 29, 37, 40, 82, 152, 174] {
        match try_config(&mut env, &k, &wx.mint, wx_params(t)) {
            Ok(_) => ok.push(t),
            Err(e) => println!("  T_wX {t}: REFUSED {e}"),
        }
    }
    println!("create_config accepted for T_wX = {ok:?}");

    // quote mint with a freeze authority
    let c2 = Keypair::new();
    let fa = c2.pubkey();
    let (wx_f, _, _) = ceremony_wx(&mut env, c2, "wFRZ", Some(&fa));
    println!("quote with a FREEZE authority: {:?}", try_config(&mut env, &k, &wx_f.mint, wx_params(5)).map(|_| "accepted"));
    // quote mint with 6 decimals (USDC-like): curve as designed for 9 decimals but scaled is a different curve; just check acceptance
    let m6 = env.create_mint(&k, 6);
    println!("quote with 6 decimals (same raw curve): {:?}", try_config(&mut env, &k, &m6, wx_params(5)).map(|_| "accepted"));
    // fee modes
    for (mode, name) in [(0u8, "QuoteToken"), (1, "OutputToken")] {
        let mut p = wx_params(5);
        p.collect_fee_mode = mode;
        println!("collect_fee_mode {mode} ({name}) with wX quote: {:?}", try_config(&mut env, &k, &wx.mint, p).map(|_| "accepted"));
    }
    for mode in [0u8, 1] {
        let mut p = wx_params(5);
        p.migrated_pool_fee.collect_fee_mode = mode;
        println!("migrated_pool_fee.collect_fee_mode {mode} with wX quote: {:?}", try_config(&mut env, &k, &wx.mint, p).map(|_| "accepted"));
    }
    // fee claimer = a PDA-like off-curve key is what Q1 used; here on-curve worked above. A config with a different
    // leftover_receiver than the claimer:
    let other = Keypair::new().pubkey();
    let config = Keypair::new();
    let ix = ix_create_config_q(&env.payer.pubkey(), &config.pubkey(), &k, &other, &wx.mint, wx_params(5));
    println!("leftover_receiver ≠ fee_claimer: {}", if env.send(&[ix], &[&config]).is_ok() { "accepted" } else { "refused" });
    // the mint authority of the quote changes after configs exist (canister rotation): DBC never reads it again
    let cfg = try_config(&mut env, &k, &wx.mint, wx_params(5)).unwrap();
    let ix = spl_token::instruction::set_authority(&spl_token::ID, &wx.mint, Some(&other), spl_token::instruction::AuthorityType::MintTokens, &k, &[]).unwrap();
    env.send_ok("set_authority", &[ix], &[&wx.canister]);
    let coin = launch_q(&mut env, &cfg, &wx.mint);
    println!("launch after the quote's mint authority changed: OK (pool {})", coin.pool);
    // threshold below DBC's minimum? try 1 wX with the T=3 curve scaled is not designed; try the T=3 curve with threshold 1
    let mut p = wx_params(3);
    p.migration_quote_threshold = LAMPORTS;
    println!("threshold 1 wX on the 3-wX curve: {:?}", try_config(&mut env, &k, &wx.mint, p).map(|_| "accepted"));
}

// ================================================================================================= S1c: wX/SOL pool
#[test]
fn s1c_wx_sol_pool_managed() {
    println!("\n==================== S1c: wX/SOL DAMM v2 pool, range [p/4, 4p], managed (not locked) by the desk ====================");
    let mut env = Env::new();
    let canister = Keypair::new();
    let (wx, _, _) = ceremony_wx(&mut env, canister, "wMILADY", None);
    let desk = env.funded(10); // K_desk[i]: the treasury's desk money
    let floor = 18.61e9; // Milady floor, lamports per whole wX
    let s0 = sqrt_of(floor);
    let (smin, smax) = (s0 / 2, s0 * 2); // [p/4, 4p]
    let seed_sol = LAMPORTS / 2; // O-1 b: 0.5 SOL a side
    let seed_wx = (seed_sol as f64 / floor * 1e9) as u64;
    let liq = liquidity_for(seed_wx, seed_sol, s0, smin, smax);

    // ONE transaction (§5.1): the canister's first wX mint goes to the desk, the desk wraps SOL and creates the pool.
    let desk_wx = get_associated_token_address(&desk.pubkey(), &wx.mint);
    let desk_wsol = get_associated_token_address(&desk.pubkey(), &NATIVE_MINT);
    let nft = Keypair::new(); // the position NFT mint must sign: in the canister, one more derived key (or a throwaway)
    let pool = cpool(&wx.mint, &NATIVE_MINT);
    let p = Damm { pool, a: wx.mint, b: NATIVE_MINT };
    let params = cp_amm::types::InitializeCustomizablePoolParameters {
        pool_fees: cp_amm::types::PoolFeeParameters { base_fee: base_fee(5_000_000), compounding_fee_bps: 0, padding: 0, dynamic_fee: None },
        sqrt_min_price: smin,
        sqrt_max_price: smax,
        has_alpha_vault: false,
        liquidity: liq,
        sqrt_price: s0,
        activation_type: 1,
        collect_fee_mode: 1, // fees in token b = SOL
        activation_point: None,
    };
    let init = Instruction {
        program_id: DAMM_V2_ID,
        accounts: cp_amm::client::accounts::InitializeCustomizablePool {
            creator: desk.pubkey(),
            position_nft_mint: nft.pubkey(),
            position_nft_account: damm_position_nft_account(&nft.pubkey()),
            payer: desk.pubkey(), // DAMM v2: `payer` is also the authority of payer_token_a/b, so the token owner pays the pool rent
            pool_authority: DAMM_V2_POOL_AUTHORITY,
            pool,
            position: damm_position(&nft.pubkey()),
            token_a_mint: wx.mint,
            token_b_mint: NATIVE_MINT,
            token_a_vault: p.va(),
            token_b_vault: p.vb(),
            payer_token_a: desk_wx,
            payer_token_b: desk_wsol,
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
    let payer = env.payer.pubkey();
    let ixs = vec![
        ata_ix::create_associated_token_account_idempotent(&payer, &desk.pubkey(), &wx.mint, &spl_token::ID),
        spl_token::instruction::mint_to_checked(&spl_token::ID, &wx.mint, &desk_wx, &wx.canister.pubkey(), &[], seed_wx, 9).unwrap(),
        ata_ix::create_associated_token_account_idempotent(&payer, &desk.pubkey(), &NATIVE_MINT, &spl_token::ID),
        system_instruction::transfer(&desk.pubkey(), &desk_wsol, seed_sol),
        spl_token::instruction::sync_native(&spl_token::ID, &desk_wsol).unwrap(),
        init,
        memo("rook:0:pool:open"),
    ];
    let size = tx_size(&payer, &ixs);
    let nsig = sigs_of(&payer, &ixs);
    {
        // what the canister's line would add: a durable-nonce advance first (nonce account + RecentBlockhashes sysvar)
        let nonce = Keypair::new().pubkey();
        let adv = system_instruction::advance_nonce_account(&nonce, &payer);
        let mut with_nonce = vec![adv.clone()];
        with_nonce.extend_from_slice(&ixs);
        let mut lean = vec![adv];
        lean.extend(ixs.iter().filter(|i| i.program_id != MEMO_ID).cloned());
        lean.remove(1 + 2); // the desk's wSOL ATA exists already
        let ata_prog = anchor_spl::associated_token::ID;
        let lean2: Vec<Instruction> = with_nonce.iter().filter(|i| i.program_id != ata_prog).cloned().collect();
        println!(
            "  sizes: as sent {} B; + durable-nonce advance {} B; + nonce, no memo, wSOL ATA pre-made {} B; + nonce + memo, both desk ATAs pre-made {} B; + nonce, no memo, no ATAs {} B (limit 1232)",
            size,
            tx_size(&payer, &with_nonce),
            tx_size(&payer, &lean),
            tx_size(&payer, &lean2),
            tx_size(&payer, &lean2.iter().filter(|i| i.program_id != MEMO_ID).cloned().collect::<Vec<_>>())
        );
    }
    let r = env.send_ok("mint seed + wrap + damm_v2.initialize_customizable_pool + memo (one tx)", &ixs, &[&wx.canister, &desk, &nft]);
    {
        let d = env.raw(&pool);
        let b = s0.to_le_bytes();
        let hits: Vec<usize> = (8..d.len() - 16).filter(|&o| d[o..o + 16] == b).collect();
        let _ = SQRT_OFF.set(*hits.last().unwrap());
    }
    println!(
        "one-tx pool opening: {} signatures (K_main, K_coll, K_desk, position-NFT mint), {} B legacy (limit 1232), {} CU; seed {} wX base units ({:.6} wX) + {} lamports; used {} / {}",
        nsig,
        size,
        r.compute_units_consumed - 150,
        seed_wx,
        seed_wx as f64 / 1e9,
        seed_sol,
        seed_wx - env.token_amount(&desk_wx),
        seed_sol - env.token_amount(&desk_wsol)
    );
    assert_eq!(env.token_owner(&damm_position_nft_account(&nft.pubkey())), desk.pubkey());
    let (u, v, l) = position_liq(&env, &nft.pubkey());
    println!("position: unlocked {u}, vested {v}, locked {l} (managed, not locked)");
    assert!(u > 0 && l == 0);

    // swaps: buy wX with SOL, then sell
    let trader = env.funded(5);
    let t_wsol = env.wsol(&trader, 2 * LAMPORTS);
    let t_wx = env.ata(&trader.pubkey(), &wx.mint);
    for amt in [LAMPORTS / 100, LAMPORTS / 20, LAMPORTS / 10] {
        let s_before = pool_sqrt(&env, &pool);
        let w0 = env.token_amount(&t_wx);
        env.send_ok("damm_v2.swap SOL → wX", &[ix_damm_swap(&p, &trader.pubkey(), t_wsol, t_wx, amt, 0)], &[&trader]);
        let got = env.token_amount(&t_wx) - w0;
        let eff = amt as f64 / got as f64; // lamports per base unit = SOL per wX
        let moved = price_of(pool_sqrt(&env, &pool)) / price_of(s_before) - 1.0;
        println!(
            "  buy {:.3} SOL → {:.6} wX: avg {:.4} SOL/wX ({:+.2} % vs floor 18.61), pool price moves {:+.2} %",
            amt as f64 / 1e9,
            got as f64 / 1e9,
            eff,
            (eff / 18.61 - 1.0) * 100.0,
            moved * 100.0
        );
        // sell back to restore
        let s1 = pool_sqrt(&env, &pool);
        env.send_ok("damm_v2.swap wX → SOL", &[ix_damm_swap(&p, &trader.pubkey(), t_wx, t_wsol, got, 0)], &[&trader]);
        let _ = s1;
    }
    println!("  after round trips the price is {:.4} SOL/wX", price_of(pool_sqrt(&env, &pool)));

    // desk claims fees (SOL only), adds, removes part, removes all, closes the position
    let (a0, b0) = (env.token_amount(&desk_wx), env.token_amount(&desk_wsol));
    env.send_ok("damm_v2.claim_position_fee (K_main + K_desk)", &[ix_damm_claim(&p, &nft.pubkey(), &desk.pubkey(), desk_wx, desk_wsol)], &[&desk]);
    println!("claim_position_fee: {} wX base units, {} lamports (fees in SOL only)", env.token_amount(&desk_wx) - a0, env.token_amount(&desk_wsol) - b0);

    let s_now = pool_sqrt(&env, &pool);
    let add_wx = seed_wx / 2;
    let add_sol = seed_sol / 2;
    let ixs = [
        spl_token::instruction::mint_to_checked(&spl_token::ID, &wx.mint, &desk_wx, &wx.canister.pubkey(), &[], add_wx, 9).unwrap(),
        system_instruction::transfer(&desk.pubkey(), &desk_wsol, add_sol),
        spl_token::instruction::sync_native(&spl_token::ID, &desk_wsol).unwrap(),
    ];
    env.send_ok("mint + wrap", &ixs, &[&wx.canister, &desk]);
    let dl = liquidity_for(add_wx, add_sol, s_now, smin, smax);
    env.send_ok("damm_v2.add_liquidity (K_desk)", &[ix_damm_add(&p, &nft.pubkey(), &desk.pubkey(), desk_wx, desk_wsol, dl, u64::MAX, u64::MAX)], &[&desk]);
    let (u2, _, _) = position_liq(&env, &nft.pubkey());
    println!("add_liquidity: +{dl} → {u2}");
    let (a1, b1) = (env.token_amount(&desk_wx), env.token_amount(&desk_wsol));
    env.send_ok("damm_v2.remove_liquidity (K_desk)", &[ix_damm_remove(&p, &nft.pubkey(), &desk.pubkey(), desk_wx, desk_wsol, Some(u2 / 4))], &[&desk]);
    println!("remove_liquidity 25 %: got {} wX base units, {} lamports", env.token_amount(&desk_wx) - a1, env.token_amount(&desk_wsol) - b1);
    let (a2, b2) = (env.token_amount(&desk_wx), env.token_amount(&desk_wsol));
    env.send_ok("damm_v2.remove_all_liquidity (K_desk)", &[ix_damm_remove(&p, &nft.pubkey(), &desk.pubkey(), desk_wx, desk_wsol, None)], &[&desk]);
    println!(
        "remove_all_liquidity: got {} wX base units, {} lamports; pool vaults now {} / {}",
        env.token_amount(&desk_wx) - a2,
        env.token_amount(&desk_wsol) - b2,
        env.token_amount(&p.va()),
        env.token_amount(&p.vb())
    );
    // burn the recovered wX (what the desk does at the module), close the position
    let left = env.token_amount(&desk_wx);
    let ix = spl_token::instruction::burn_checked(&spl_token::ID, &desk_wx, &wx.mint, &desk.pubkey(), &[], left, 9).unwrap();
    env.send_ok("spl.burn_checked recovered wX (K_desk)", &[ix], &[&desk]);
    match env.send(&[ix_close_position(&p, &nft.pubkey(), &desk.pubkey(), &desk.pubkey())], &[&desk]) {
        Ok(m) => println!("close_position: OK ({} CU); wX supply now {}", m.compute_units_consumed - 150, env.mint_supply(&wx.mint)),
        Err(e) => println!("close_position refused: {}", first_err(&e.meta.logs.join("\n"))),
    }
    // the pool is now empty: re-opening at another price needs a new position (create_position + add_liquidity)
    print_cu(&env, "S1c");
}

// ================================================================================================= S1d: depth vs range/seed
#[test]
fn s1d_pool_depth() {
    println!("\n==================== S1d: wX/SOL pool depth by range and seed (Milady floor 18.61, fee 0.5 %) ====================");
    println!("  {:>14} {:>10} {:>9} {:>14} {:>14}", "range", "SOL/side", "buy SOL", "avg vs floor", "price moves");
    for (label, k_sqrt) in [("[p/4, 4p]", 2.0f64), ("[p/1.44,1.44p]", 1.2), ("[p/1.21,1.21p]", 1.1), ("[p/1.1, 1.1p]", 1.048_808_848)] {
        for seed in [LAMPORTS / 2, 5 * LAMPORTS] {
            let mut env = Env::new();
            let (wx, _, _) = ceremony_wx(&mut env, Keypair::new(), "wMILADY", None);
            let desk = env.funded(20);
            let floor = 18.61e9;
            let s0 = sqrt_of(floor);
            let (smin, smax) = ((s0 as f64 / k_sqrt) as u128, (s0 as f64 * k_sqrt) as u128);
            let seed_wx = (seed as f64 / floor * 1e9) as u64;
            let desk_wx = wx.mint_to(&mut env, &desk.pubkey(), seed_wx);
            let desk_wsol = env.wsol(&desk, seed);
            let nft = Keypair::new();
            let pool = cpool(&wx.mint, &NATIVE_MINT);
            let p = Damm { pool, a: wx.mint, b: NATIVE_MINT };
            let params = cp_amm::types::InitializeCustomizablePoolParameters {
                pool_fees: cp_amm::types::PoolFeeParameters { base_fee: base_fee(5_000_000), compounding_fee_bps: 0, padding: 0, dynamic_fee: None },
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
                    token_a_vault: p.va(),
                    token_b_vault: p.vb(),
                    payer_token_a: desk_wx,
                    payer_token_b: desk_wsol,
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
            let trader = env.funded(5);
            let t_wsol = env.wsol(&trader, 3 * LAMPORTS);
            let t_wx = env.ata(&trader.pubkey(), &wx.mint);
            for amt in [LAMPORTS / 100, LAMPORTS / 10, LAMPORTS / 2] {
                let s_before = sq(&env);
                let w0 = env.token_amount(&t_wx);
                match env.send(&[ix_damm_swap(&p, &trader.pubkey(), t_wsol, t_wx, amt, 0)], &[&trader]) {
                    Ok(_) => {
                        let got = env.token_amount(&t_wx) - w0;
                        let eff = amt as f64 / got as f64;
                        let moved = price_of(sq(&env)) / price_of(s_before) - 1.0;
                        println!("  {:>14} {:>10.2} {:>9.2} {:>13.2}% {:>13.2}%", label, seed as f64 / 1e9, amt as f64 / 1e9, (eff / 18.61 - 1.0) * 100.0, moved * 100.0);
                        env.send_ok("sell back", &[ix_damm_swap(&p, &trader.pubkey(), t_wx, t_wsol, got, 0)], &[&trader]);
                    }
                    Err(e) => println!("  {:>14} {:>10.2} {:>9.2}   refused: {}", label, seed as f64 / 1e9, amt as f64 / 1e9, first_err(&e.meta.logs.join("\n"))),
                }
            }
        }
    }
}

// ================================================================================================= S1e: Jupiter's liquidity test
/// Jupiter's post-grace market test (developers.jup.ag/docs/swap/routing/market-listing): a $500 round trip must lose < 30 %.
/// At ≈ $110/SOL, $500 ≈ 4.5 SOL. Buy 4.5 SOL of wX, sell it all back, report the loss.
#[test]
fn s1e_jupiter_round_trip() {
    println!("\n==================== S1e: $500 (4.5 SOL) round trip on the wX/SOL pool ====================");
    for (label, k_sqrt) in [("[p/4, 4p]", 2.0f64), ("[p/1.21,1.21p]", 1.1), ("full range", 0.0)] {
        for seed in [LAMPORTS / 2, 5 * LAMPORTS, 10 * LAMPORTS, 20 * LAMPORTS] {
            let mut env = Env::new();
            let (wx, _, _) = ceremony_wx(&mut env, Keypair::new(), "wMILADY", None);
            let desk = env.funded(50);
            let floor = 18.61e9;
            let s0 = sqrt_of(floor);
            let (smin, smax) = if k_sqrt == 0.0 { (MIN_SQRT_PRICE, MAX_SQRT_PRICE) } else { ((s0 as f64 / k_sqrt) as u128, (s0 as f64 * k_sqrt) as u128) };
            let seed_wx = (seed as f64 / floor * 1e9) as u64;
            let desk_wx = wx.mint_to(&mut env, &desk.pubkey(), seed_wx);
            let desk_wsol = env.wsol(&desk, seed);
            let nft = Keypair::new();
            let pool = cpool(&wx.mint, &NATIVE_MINT);
            let p = Damm { pool, a: wx.mint, b: NATIVE_MINT };
            let params = cp_amm::types::InitializeCustomizablePoolParameters {
                pool_fees: cp_amm::types::PoolFeeParameters { base_fee: base_fee(5_000_000), compounding_fee_bps: 0, padding: 0, dynamic_fee: None },
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
                    token_a_vault: p.va(),
                    token_b_vault: p.vb(),
                    payer_token_a: desk_wx,
                    payer_token_b: desk_wsol,
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
            let trader = env.funded(30);
            let t_wsol = env.wsol(&trader, 20 * LAMPORTS);
            let t_wx = env.ata(&trader.pubkey(), &wx.mint);
            // test 1: $500 round trip; test 2: per-unit price of a $1,000 buy within 20 % of a $500 buy
            let mut res = vec![];
            for amt in [4_500_000_000u64, 9_000_000_000] {
                let s_before = env.token_amount(&t_wsol);
                match env.send(&[ix_damm_swap(&p, &trader.pubkey(), t_wsol, t_wx, amt, 0)], &[&trader]) {
                    Err(e) => res.push(Err(first_err(&e.meta.logs.join("\n")).split("Error Message: ").nth(1).unwrap_or("refused").split('.').next().unwrap().to_string())),
                    Ok(_) => {
                        let got = env.token_amount(&t_wx);
                        env.send_ok("sell", &[ix_damm_swap(&p, &trader.pubkey(), t_wx, t_wsol, got, 0)], &[&trader]);
                        let back = env.token_amount(&t_wsol) as i128 - (s_before as i128 - amt as i128);
                        res.push(Ok((amt as f64 / got as f64, 1.0 - back as f64 / amt as f64)));
                    }
                }
            }
            let t1 = match &res[0] { Ok((_, loss)) => format!("loss {:.1} % {}", loss * 100.0, if *loss < 0.3 { "PASS" } else { "FAIL" }), Err(e) => format!("buy refused ({e}) FAIL") };
            let t2 = match (&res[0], &res[1]) {
                (Ok((p5, _)), Ok((p10, _))) => format!("$1k/$500 unit price {:+.1} % {}", (p10 / p5 - 1.0) * 100.0, if p10 / p5 - 1.0 < 0.2 { "PASS" } else { "FAIL" }),
                (_, Err(e)) => format!("$1k buy refused ({e}) FAIL"),
                _ => "n/a".into(),
            };
            println!("  {:>14} {:>6.1} SOL/side: test 1 {t1}; test 2 {t2}", label, seed as f64 / 1e9);
        }
    }
}

// ================================================================================================= S2: two-hop bundle
#[test]
fn s2_two_hop_bundle() {
    println!("\n==================== S2 (LiteSVM): the site's own SOL → wX → coin transaction ====================");
    let mut env = Env::new();
    let canister = Keypair::new();
    let k = canister.pubkey();
    let (wx, _, _) = ceremony_wx(&mut env, canister, "wMILADY", None);
    // the wX/SOL pool at 18.61, 0.5 SOL a side, [p/4, 4p]
    let desk = env.funded(30);
    let floor = 18.61e9;
    let s0 = sqrt_of(floor);
    let (smin, smax) = (s0 / 2, s0 * 2);
    let seed_sol = 10 * LAMPORTS; // deep enough that a 0.1 SOL buy stays inside 3 % slippage (size and CU do not depend on depth)
    let seed_wx = (seed_sol as f64 / floor * 1e9) as u64;
    let desk_wx = wx.mint_to(&mut env, &desk.pubkey(), seed_wx);
    let desk_wsol = env.wsol(&desk, seed_sol);
    let nft = Keypair::new();
    let pool = cpool(&wx.mint, &NATIVE_MINT);
    let wp = Damm { pool, a: wx.mint, b: NATIVE_MINT };
    let params = cp_amm::types::InitializeCustomizablePoolParameters {
        pool_fees: cp_amm::types::PoolFeeParameters { base_fee: base_fee(5_000_000), compounding_fee_bps: 0, padding: 0, dynamic_fee: None },
        sqrt_min_price: smin,
        sqrt_max_price: smax,
        has_alpha_vault: false,
        liquidity: liquidity_for(seed_wx, seed_sol, s0, smin, smax),
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
            token_a_vault: wp.va(),
            token_b_vault: wp.vb(),
            payer_token_a: desk_wx,
            payer_token_b: desk_wsol,
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
    env.send_ok("init pool", &[init], &[&desk, &nft]);

    // a coin on the curve (T_wX = 5), past the anti-sniper window
    let config = try_config(&mut env, &k, &wx.mint, wx_params(5)).unwrap();
    let coin = launch_q(&mut env, &config, &wx.mint);
    env.warp(700);

    // the bundle a fresh wallet signs: no wSOL, wX or coin ATA yet
    let user = env.funded(3);
    let u = user.pubkey();
    let u_wsol = get_associated_token_address(&u, &NATIVE_MINT);
    let u_wx = get_associated_token_address(&u, &wx.mint);
    let u_coin = get_associated_token_address(&u, &coin.mint);
    let sol_in = LAMPORTS / 10;
    let wx_min = (sol_in as f64 / 18.61 * 0.97) as u64; // quoted out less 3 % slippage
    let bundle = |coin_hop: Instruction| -> Vec<Instruction> {
        vec![
            ComputeBudgetInstruction::set_compute_unit_limit(400_000),
            ComputeBudgetInstruction::set_compute_unit_price(10_000),
            ata_ix::create_associated_token_account_idempotent(&u, &u, &NATIVE_MINT, &spl_token::ID),
            system_instruction::transfer(&u, &u_wsol, sol_in),
            spl_token::instruction::sync_native(&spl_token::ID, &u_wsol).unwrap(),
            ata_ix::create_associated_token_account_idempotent(&u, &u, &wx.mint, &spl_token::ID),
            ix_damm_swap(&wp, &u, u_wsol, u_wx, sol_in, wx_min),
            ata_ix::create_associated_token_account_idempotent(&u, &u, &coin.mint, &spl_token::ID),
            coin_hop,
            spl_token::instruction::close_account(&spl_token::ID, &u_wsol, &u, &u, &[]).unwrap(),
        ]
    };
    let ixs = bundle(ix_dbc_buy(&coin, &u, u_wx, u_coin, wx_min, 1, 0));
    let r = send_as(&mut env, &user, &ixs);
    let m = Message::new(&ixs, Some(&u));
    let size = 1 + 64 * m.header.num_required_signatures as usize + m.serialize().len();
    println!(
        "pre-graduation bundle (CU limit+price, wrap, 3 idempotent ATAs, DAMM v2 swap SOL→wX, DBC swap2 wX→coin, close wSOL): {} B legacy of 1232, {} accounts, {} CU; user got {} coins, {} wX dust left",
        size,
        m.account_keys.len(),
        r.compute_units_consumed,
        env.token_amount(&u_coin),
        env.token_amount(&u_wx)
    );
    assert!(size <= 1232);

    // graduate the coin, then the post-graduation bundle (DAMM v2 wX/SOL + DAMM v2 COIN/wX)
    let whale = env.funded(2);
    let whale_wx = wx.mint_to(&mut env, &whale.pubkey(), 20 * LAMPORTS);
    let whale_coin = env.ata(&whale.pubkey(), &coin.mint);
    env.send_ok("fill", &[ix_dbc_buy(&coin, &whale.pubkey(), whale_wx, whale_coin, 20 * LAMPORTS, 0, 1)], &[&whale]);
    let mig = migrate_q(&mut env, &coin).unwrap();
    let cp = Damm { pool: mig.pool, a: coin.mint, b: wx.mint };
    let ixs = bundle(ix_damm_swap(&cp, &u, u_wx, u_coin, wx_min, 1));
    let before = env.token_amount(&u_coin);
    let r = send_as(&mut env, &user, &ixs);
    let m = Message::new(&ixs, Some(&u));
    let size = 1 + 64 * m.header.num_required_signatures as usize + m.serialize().len();
    println!(
        "post-graduation bundle (DAMM v2 SOL→wX + DAMM v2 wX→coin): {} B legacy, {} accounts, {} CU; user got {} more coins",
        size,
        m.account_keys.len(),
        r.compute_units_consumed,
        env.token_amount(&u_coin) - before
    );
    assert!(size <= 1232);
}

fn send_as(env: &mut Env, user: &Keypair, ixs: &[Instruction]) -> litesvm::types::TransactionMetadata {
    let msg = Message::new_with_blockhash(ixs, Some(&user.pubkey()), &env.svm.latest_blockhash());
    let tx = solana_transaction::versioned::VersionedTransaction::try_new(solana_message::VersionedMessage::Legacy(msg), &[user]).unwrap();
    let r = env.svm.send_transaction(tx);
    env.svm.expire_blockhash();
    r.unwrap_or_else(|e| panic!("bundle failed: {:?}\n{}", e.err, e.meta.logs.join("\n")))
}

// ------------------------------------------------------------------------------------------------- utils
fn pool_of(env: &Env, key: &Pubkey) -> dbcread::DbcPool {
    let a = env.svm.get_account(key).unwrap();
    dbcread::parse_pool(&a.owner, &a.data).unwrap()
}
/// PoolConfig.quote_token_flag (byte after token_type): 0 = SOL? DBC sets it at create_config from the quote mint.
fn quote_token_flag(env: &Env, config: &Pubkey) -> u8 {
    // discriminator 8 + quote_mint 32 + fee_claimer 32 + leftover_receiver 32 + pool_fees … ; found via the IDL order:
    // the u8 run collect_fee_mode, migration_option, activation_type, token_decimal, version, token_type, quote_token_flag.
    // Locate it relative to collect_fee_mode by matching the known values (0, 1, 1, 6, 0, 0).
    let d = env.raw(config);
    for o in 104..d.len() - 7 {
        if d[o..o + 6] == [0, 1, 1, 6, 0, 0] {
            return d[o + 6];
        }
    }
    255
}
fn first_err(logs: &str) -> String {
    logs.lines().filter(|l| l.contains("Error") || l.contains("failed")).take(2).collect::<Vec<_>>().join(" | ")
}
fn solana_program_option(k: Pubkey) -> anchor_lang::solana_program::program_option::COption<Pubkey> {
    anchor_lang::solana_program::program_option::COption::Some(k)
}
fn print_cu(env: &Env, tag: &str) {
    println!("\n{tag} CU (excluding the compute-budget instruction):");
    let mut v: Vec<_> = env.cu.iter().filter(|(k, _)| !matches!(k.as_str(), "ata" | "wsol" | "create_mint")).collect();
    v.sort();
    for (k, c) in v {
        println!("  {k:80} {c:>9}");
    }
}

use anchor_lang::solana_program::program_pack::Pack;
