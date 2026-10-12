//! Shared LiteSVM harness for rook_backing and spike Q1: Meteora DBC + DAMM v2 as deployed on mainnet (program bytes
//! dumped 2026-10-09 into tests/fixtures), Metaplex Token Metadata (DBC's pool creation calls it), the DAMM v2
//! "Customizable" migration config A8gMr… cloned from mainnet, rook_backing from target/deploy, and rook_core's `convert`
//! (the mock in tests/mock_rook_core until rook_core builds).
#![allow(dead_code)]

use std::{collections::HashMap, path::PathBuf};

use anchor_lang::{
    prelude::*,
    solana_program::{instruction::Instruction, program_pack::Pack, system_instruction},
    AccountDeserialize, InstructionData, ToAccountMetas,
};
use anchor_spl::{
    associated_token::{get_associated_token_address, spl_associated_token_account::instruction as ata_ix},
    token::spl_token,
};
use base64::Engine;
use litesvm::{
    types::{FailedTransactionMetadata, TransactionMetadata},
    LiteSVM,
};
use rook_backing::{
    constants::*,
    cp_amm,
    dynamic_bonding_curve::{self as dbc, types as dbct},
};
use solana_account::Account as SolAccount;
use solana_compute_budget_interface::ComputeBudgetInstruction;
use solana_keypair::Keypair;
use solana_message::{Message, VersionedMessage};
use solana_signer::Signer;
use solana_transaction::versioned::VersionedTransaction;

pub const MPL_ID: Pubkey = pubkey!("metaqbxxUerdq28cj1RbAWkYQm3ybzjb6a8bt518x1s");
pub const TOKEN_2022_ID: Pubkey = pubkey!("TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb");
pub const DAMM_CONFIG_CUSTOMIZABLE: Pubkey = pubkey!("A8gMrEPJkacWkcb3DGwtJwTe16HktSEfvwtuDh2MCtck");
pub const LAMPORTS: u64 = 1_000_000_000;
/// 2026-10-09T00:00:00Z
pub const T0: i64 = 1_791_504_000;

// ------------------------------------------------------------------------------------------------- the Rook curve
// From tests/q1/design_curve.mjs (a BigInt port of Meteora's designCurve): 1B supply, 6 decimals, 20 % of supply into
// DAMM v2, 85 SOL threshold, 50 % migration fee. Start ≈ 5.3e-8 SOL/coin (mcap ≈ 53 SOL), graduation ≈ 2.125e-7 (≈ 212.5).
pub const TOTAL_SUPPLY: u64 = 1_000_000_000_000_000;
pub const THRESHOLD: u64 = 85 * LAMPORTS;
pub const SQRT_START: u128 = 134452599321738966;
pub const CURVE: [(u128, u128); 2] = [
    (268905193275807567, 215124158914782432327345953095692),
    (79226673521066979257578248091, 3435308751277378593870180),
];
/// Anti-sniper: linear fee scheduler, 50 % at launch, −0.8166666 pt every 10 s for 60 periods → 1.0000040 % after 10 min.
pub const FEE_CLIFF: u64 = 500_000_000; // of 1e9
pub const FEE_PERIODS: u16 = 60;
pub const FEE_PERIOD_SECS: u64 = 10;
pub const FEE_REDUCTION: u64 = 8_166_666;

pub fn rook_config_params(migration_fee_percentage: u8) -> dbct::ConfigParameters {
    dbct::ConfigParameters {
        pool_fees: dbct::PoolFeeParameters {
            base_fee: dbct::BaseFeeParameters {
                cliff_fee_numerator: FEE_CLIFF,
                first_factor: FEE_PERIODS,     // number_of_period
                second_factor: FEE_PERIOD_SECS, // period_frequency
                third_factor: FEE_REDUCTION,    // reduction_factor
                base_fee_mode: 0,               // FeeSchedulerLinear
            },
            dynamic_fee: None,
        },
        collect_fee_mode: 0, // QuoteToken: every fee in SOL
        migration_option: 1, // DAMM v2
        activation_type: 1,  // timestamp
        token_type: 0,       // SPL
        token_decimal: 6,
        partner_liquidity_percentage: 0,
        partner_permanent_locked_liquidity_percentage: 50,
        creator_liquidity_percentage: 0,
        creator_permanent_locked_liquidity_percentage: 50,
        migration_quote_threshold: THRESHOLD,
        sqrt_start_price: SQRT_START,
        locked_vesting: dbct::LockedVestingParams {
            amount_per_period: 0,
            cliff_duration_from_migration_time: 0,
            frequency: 0,
            number_of_period: 0,
            cliff_unlock_amount: 0,
        },
        migration_fee_option: 6, // Customizable → DAMM v2 config A8gMr…
        // The fixed 1B supply only validates for the share the curve was designed for (50 %): the migration base amount
        // depends on the fee (DBC InvalidTokenSupply otherwise). Other shares run with DBC's dynamic supply here.
        token_supply: (migration_fee_percentage == 50).then_some(dbct::TokenSupplyParams {
            pre_migration_token_supply: TOTAL_SUPPLY,
            post_migration_token_supply: TOTAL_SUPPLY,
        }),
        creator_trading_fee_percentage: 0,
        token_update_authority: 1, // Immutable
        migration_fee: dbct::MigrationFee { fee_percentage: migration_fee_percentage, creator_fee_percentage: 0 },
        migrated_pool_fee: dbct::MigratedPoolFee { collect_fee_mode: 0, dynamic_fee: 0, pool_fee_bps: 100 },
        pool_creation_fee: 0,
        partner_liquidity_vesting_info: zero_vesting(),
        creator_liquidity_vesting_info: zero_vesting(),
        migrated_pool_base_fee_mode: 0,
        migrated_pool_market_cap_fee_scheduler_params: dbct::MigratedPoolMarketCapFeeSchedulerParams {
            number_of_period: 0,
            sqrt_price_step_bps: 0,
            scheduler_expiration_duration: 0,
            reduction_factor: 0,
        },
        enable_first_swap_with_min_fee: false,
        compounding_fee_bps: 0,
        padding: [0; 2],
        curve: CURVE
            .iter()
            .map(|(s, l)| dbct::LiquidityDistributionParameters { sqrt_price: *s, liquidity: *l })
            .collect(),
    }
}

fn zero_vesting() -> dbct::LiquidityVestingInfoParams {
    dbct::LiquidityVestingInfoParams {
        vesting_percentage: 0,
        bps_per_period: 0,
        number_of_periods: 0,
        cliff_duration_from_migration_time: 0,
        frequency: 0,
    }
}

// ------------------------------------------------------------------------------------------------- PDAs
pub fn pda(seeds: &[&[u8]], program: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(seeds, program).0
}
pub fn event_authority(program: &Pubkey) -> Pubkey {
    pda(&[b"__event_authority"], program)
}
fn max_min(a: &Pubkey, b: &Pubkey) -> (Pubkey, Pubkey) {
    if a > b {
        (*a, *b)
    } else {
        (*b, *a)
    }
}
pub fn dbc_pool_pda(config: &Pubkey, base: &Pubkey) -> Pubkey {
    let (hi, lo) = max_min(base, &NATIVE_MINT);
    pda(&[b"pool", config.as_ref(), hi.as_ref(), lo.as_ref()], &DBC_ID)
}
pub fn dbc_vault(mint: &Pubkey, pool: &Pubkey) -> Pubkey {
    pda(&[b"token_vault", mint.as_ref(), pool.as_ref()], &DBC_ID)
}
pub fn damm_pool_pda(base: &Pubkey) -> Pubkey {
    let (hi, lo) = max_min(base, &NATIVE_MINT);
    pda(&[b"pool", DAMM_CONFIG_CUSTOMIZABLE.as_ref(), hi.as_ref(), lo.as_ref()], &DAMM_V2_ID)
}
pub fn damm_vault(mint: &Pubkey, pool: &Pubkey) -> Pubkey {
    pda(&[b"token_vault", mint.as_ref(), pool.as_ref()], &DAMM_V2_ID)
}
pub fn damm_position(nft_mint: &Pubkey) -> Pubkey {
    pda(&[b"position", nft_mint.as_ref()], &DAMM_V2_ID)
}
pub fn damm_position_nft_account(nft_mint: &Pubkey) -> Pubkey {
    pda(&[b"position_nft_account", nft_mint.as_ref()], &DAMM_V2_ID)
}
pub fn settings_pda() -> Pubkey {
    pda(&[SETTINGS_SEED], &rook_backing::ID)
}
pub fn partner_pda(listing: &Pubkey) -> Pubkey {
    pda(&[PARTNER_SEED, listing.as_ref()], &rook_backing::ID)
}
/// rook_backing's program-wide ["converter"] PDA: the one key rook_core's `Registry.converters` holds for it.
pub fn converter_pda() -> Pubkey {
    pda(&[CONVERTER_SEED], &rook_backing::ID)
}
pub fn claimer_pda(listing: &Pubkey) -> Pubkey {
    pda(&[CLAIMER_SEED, listing.as_ref()], &rook_backing::ID)
}
pub fn claimer_wsol_pda(listing: &Pubkey) -> Pubkey {
    pda(&[CLAIMER_WSOL_SEED, listing.as_ref()], &rook_backing::ID)
}
pub fn vault_pda(coin: &Pubkey) -> Pubkey {
    pda(&[VAULT_SEED, coin.as_ref()], &rook_backing::ID)
}
pub fn vault_wx_pda(coin: &Pubkey) -> Pubkey {
    pda(&[VAULT_WX_SEED, coin.as_ref()], &rook_backing::ID)
}
pub fn treasury_pda() -> Pubkey {
    pda(&[b"treasury"], &ROOK_CORE_ID)
}
/// rook_core's PDAs of a listing (`rook_layouts::core_seeds`).
pub fn core_module_pda(listing: &Pubkey) -> Pubkey {
    pda(&[rook_layouts::core_seeds::MODULE, listing.as_ref()], &ROOK_CORE_ID)
}
pub fn core_module_vault_pda(module: &Pubkey) -> Pubkey {
    pda(&[rook_layouts::core_seeds::MODULE_VAULT, module.as_ref()], &ROOK_CORE_ID)
}
pub fn core_wrapper_pda(listing: &Pubkey) -> Pubkey {
    pda(&[rook_layouts::core_seeds::WRAPPER, listing.as_ref()], &ROOK_CORE_ID)
}
/// The mock's floor: it converts at 2 SOL per unit; the mock `Module` says `accepted` = 2 SOL, `conv_bps` = 0, so `back`'s
/// price bound (audit A3-06) holds with the mock's price.
pub const MOCK_ACCEPTED: u64 = 2 * LAMPORTS;

// ------------------------------------------------------------------------------------------------- the environment
pub struct Env {
    pub svm: LiteSVM,
    pub payer: Keypair,
    /// rook_backing's admin under the mock: the `owner` written into the mock rook_core registry (`setup_collection`).
    pub rook_owner: Keypair,
    /// last CU per label, for the reports
    pub cu: HashMap<String, u64>,
}

pub type TxResult = std::result::Result<TransactionMetadata, FailedTransactionMetadata>;

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures")
}
fn deploy_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/deploy")
}

/// rook_backing as LiteSVM 0.10 can load it. `anchor build` (1.2.1) emits SBPF v3 by default (live on mainnet since epoch
/// 993) but LiteSVM 0.10's runtime refuses v3 ELFs (InvalidAccountData), so the tests run the same source built for SBPF v2:
///   cargo build-sbf --manifest-path programs/rook_backing/Cargo.toml --arch v2 --sbf-out-dir target/deploy-sbpfv2
pub fn rook_backing_so() -> PathBuf {
    let main = deploy_dir().join("rook_backing.so");
    if let Ok(b) = std::fs::read(&main) {
        if b.len() > 52 && b[48] != 3 {
            return main;
        }
    }
    let v2 = deploy_dir().join("../deploy-sbpfv2/rook_backing.so");
    assert!(
        v2.exists(),
        "build the SBPF v2 test binary first: cargo build-sbf --manifest-path programs/rook_backing/Cargo.toml --arch v2 --sbf-out-dir target/deploy-sbpfv2"
    );
    v2
}

/// The rook_core `Env::new()` loads. Returns (path, is_mock). By default the mock (tests/mock_rook_core: `convert` only, raw
/// accounts). With `ROOK_CORE_REAL=1`, the real program as LiteSVM 0.10 can load it: target/deploy/rook_core.so if that is
/// not SBPF v3, else the v2 build of the same source:
///   cargo build-sbf --manifest-path programs/rook_core/Cargo.toml --arch v2 --sbf-out-dir target/deploy-sbpfv2
/// The real program needs real rook_core state (registry, listing, module, wrapper), which tests/real_rook_core.rs builds
/// with rook_core's own suite; `setup_collection` (mock accounts) therefore refuses to run against it.
pub fn rook_core_so() -> (PathBuf, bool) {
    if std::env::var("ROOK_CORE_REAL").map(|v| v == "1").unwrap_or(false) {
        return (real_rook_core_so(), false);
    }
    (fixtures().join("mock_rook_core.so"), true)
}

/// The real rook_core binary (see `rook_core_so`).
pub fn real_rook_core_so() -> PathBuf {
    let main = deploy_dir().join("rook_core.so");
    if let Ok(b) = std::fs::read(&main) {
        if b.len() > 52 && b[48] != 3 {
            return main;
        }
    }
    let v2 = deploy_dir().join("../deploy-sbpfv2/rook_core.so");
    assert!(
        v2.exists(),
        "build the SBPF v2 test binary first: cargo build-sbf --manifest-path programs/rook_core/Cargo.toml --arch v2 --sbf-out-dir target/deploy-sbpfv2"
    );
    v2
}

/// rook_core's registry PDA ["registry"] (the account rook_backing's admin instructions read).
pub fn registry_pda() -> Pubkey {
    pda(&[rook_layouts::registry::SEED], &ROOK_CORE_ID)
}

impl Env {
    pub fn new() -> Self {
        let mut svm = LiteSVM::new().with_log_bytes_limit(Some(100_000));
        svm.add_program_from_file(ROOK_CORE_ID, rook_core_so().0).unwrap();
        let mut clock: Clock = svm.get_sysvar();
        clock.unix_timestamp = T0;
        clock.slot = 400_000_000;
        svm.set_sysvar(&clock);
        Self::from_svm(svm)
    }

    /// Adds Meteora DBC, DAMM v2, Metaplex, rook_backing and the cloned accounts to an existing SVM (rook_core already in
    /// it: the mock from `new()`, or the real one from rook_core's own test suite).
    pub fn from_svm(mut svm: LiteSVM) -> Self {
        let f = fixtures();
        svm.add_program_from_file(DBC_ID, f.join("dbc.so")).unwrap();
        svm.add_program_from_file(DAMM_V2_ID, f.join("damm_v2.so")).unwrap();
        svm.add_program_from_file(MPL_ID, f.join("mpl_token_metadata.so")).unwrap();
        svm.add_program_from_file(rook_backing::ID, rook_backing_so()).unwrap();

        // the DAMM v2 migration config, cloned from mainnet
        let j: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(f.join("damm_v2_config_customizable.json")).unwrap()).unwrap();
        let data = base64::engine::general_purpose::STANDARD
            .decode(j["account"]["data"][0].as_str().unwrap())
            .unwrap();
        svm.set_account(
            DAMM_CONFIG_CUSTOMIZABLE,
            SolAccount { lamports: j["account"]["lamports"].as_u64().unwrap(), data, owner: DAMM_V2_ID, executable: false, rent_epoch: 0 },
        )
        .unwrap();
        // wSOL mint (LiteSVM does not ship it): 82 bytes, decimals 9, initialized, no authorities
        let mut mint = vec![0u8; 82];
        mint[44] = 9;
        mint[45] = 1;
        svm.set_account(NATIVE_MINT, SolAccount { lamports: 1_000_000_000, data: mint, owner: spl_token::ID, executable: false, rent_epoch: 0 })
            .unwrap();
        // DBC's pool authority pays "flash rent" during migration; on mainnet it holds a float.
        svm.airdrop(&DBC_POOL_AUTHORITY, LAMPORTS).unwrap();

        let payer = Keypair::new();
        svm.airdrop(&payer.pubkey(), 10_000 * LAMPORTS).unwrap();
        let rook_owner = Keypair::new();
        svm.airdrop(&rook_owner.pubkey(), 100 * LAMPORTS).unwrap();
        Env { svm, payer, rook_owner, cu: HashMap::new() }
    }

    pub fn now(&self) -> i64 {
        self.svm.get_sysvar::<Clock>().unix_timestamp
    }
    pub fn warp(&mut self, secs: i64) {
        let mut c: Clock = self.svm.get_sysvar();
        c.unix_timestamp += secs;
        c.slot += (secs as u64) * 5 / 2;
        self.svm.set_sysvar(&c);
        self.svm.expire_blockhash();
    }

    pub fn funded(&mut self, sol: u64) -> Keypair {
        let k = Keypair::new();
        self.svm.airdrop(&k.pubkey(), sol * LAMPORTS).unwrap();
        k
    }

    /// Sends `ixs` with a 1.4M CU limit; payer = self.payer plus `signers`.
    pub fn send(&mut self, ixs: &[Instruction], signers: &[&Keypair]) -> TxResult {
        let mut all = vec![ComputeBudgetInstruction::set_compute_unit_limit(1_400_000)];
        all.extend_from_slice(ixs);
        let mut ks: Vec<&Keypair> = vec![&self.payer];
        for s in signers {
            if s.pubkey() != self.payer.pubkey() {
                ks.push(s);
            }
        }
        let msg = Message::new_with_blockhash(&all, Some(&self.payer.pubkey()), &self.svm.latest_blockhash());
        let tx = VersionedTransaction::try_new(VersionedMessage::Legacy(msg), &ks).unwrap();
        let r = self.svm.send_transaction(tx);
        self.svm.expire_blockhash();
        r
    }

    /// Sends and records the CU of the transaction (minus the compute-budget instruction's 150) under `label`.
    pub fn send_ok(&mut self, label: &str, ixs: &[Instruction], signers: &[&Keypair]) -> TransactionMetadata {
        match self.send(ixs, signers) {
            Ok(m) => {
                self.cu.insert(label.to_string(), m.compute_units_consumed.saturating_sub(150));
                m
            }
            Err(e) => panic!("{label} failed: {:?}\n{}", e.err, e.meta.logs.join("\n")),
        }
    }

    pub fn send_err(&mut self, label: &str, ixs: &[Instruction], signers: &[&Keypair]) -> String {
        match self.send(ixs, signers) {
            Ok(_) => panic!("{label}: expected failure, got success"),
            Err(e) => e.meta.logs.join("\n"),
        }
    }

    pub fn lamports(&self, k: &Pubkey) -> u64 {
        self.svm.get_account(k).map(|a| a.lamports).unwrap_or(0)
    }
    pub fn exists(&self, k: &Pubkey) -> bool {
        self.svm.get_account(k).map(|a| a.lamports > 0).unwrap_or(false)
    }
    pub fn token_amount(&self, k: &Pubkey) -> u64 {
        let a = self.svm.get_account(k).expect("token account");
        spl_token::state::Account::unpack(&a.data[..165]).unwrap().amount
    }
    pub fn token_owner(&self, k: &Pubkey) -> Pubkey {
        let a = self.svm.get_account(k).expect("token account");
        Pubkey::new_from_array(a.data[32..64].try_into().unwrap())
    }
    pub fn mint_supply(&self, k: &Pubkey) -> u64 {
        let a = self.svm.get_account(k).expect("mint");
        spl_token::state::Mint::unpack(&a.data[..82]).unwrap().supply
    }
    pub fn anchor<T: AccountDeserialize>(&self, k: &Pubkey) -> T {
        let a = self.svm.get_account(k).expect("account");
        T::try_deserialize(&mut a.data.as_slice()).unwrap()
    }
    pub fn raw(&self, k: &Pubkey) -> Vec<u8> {
        self.svm.get_account(k).expect("account").data
    }

    // ---------------------------------------------------------------------------------------- SPL helpers
    pub fn create_mint(&mut self, authority: &Pubkey, decimals: u8) -> Pubkey {
        let mint = Keypair::new();
        let rent = self.svm.minimum_balance_for_rent_exemption(82);
        let ixs = [
            system_instruction::create_account(&self.payer.pubkey(), &mint.pubkey(), rent, 82, &spl_token::ID),
            spl_token::instruction::initialize_mint2(&spl_token::ID, &mint.pubkey(), authority, None, decimals).unwrap(),
        ];
        self.send_ok("create_mint", &ixs, &[&mint]);
        mint.pubkey()
    }
    pub fn ata(&mut self, owner: &Pubkey, mint: &Pubkey) -> Pubkey {
        let ix = ata_ix::create_associated_token_account_idempotent(&self.payer.pubkey(), owner, mint, &spl_token::ID);
        self.send_ok("ata", &[ix], &[]);
        get_associated_token_address(owner, mint)
    }
    /// A wSOL ATA for `owner` holding `lamports`.
    pub fn wsol(&mut self, owner: &Keypair, lamports: u64) -> Pubkey {
        let ata = get_associated_token_address(&owner.pubkey(), &NATIVE_MINT);
        let ixs = [
            ata_ix::create_associated_token_account_idempotent(&self.payer.pubkey(), &owner.pubkey(), &NATIVE_MINT, &spl_token::ID),
            system_instruction::transfer(&owner.pubkey(), &ata, lamports),
            spl_token::instruction::sync_native(&spl_token::ID, &ata).unwrap(),
        ];
        self.send_ok("wsol", &ixs, &[owner]);
        ata
    }
}

// ------------------------------------------------------------------------------------------------- Meteora DBC
pub struct Coin {
    pub config: Pubkey,
    pub mint: Pubkey,
    pub pool: Pubkey,
    pub base_vault: Pubkey,
    pub quote_vault: Pubkey,
    pub creator: Keypair,
}

pub fn ix_create_config(payer: &Pubkey, config: &Pubkey, fee_claimer: &Pubkey, leftover: &Pubkey, params: dbct::ConfigParameters) -> Instruction {
    Instruction {
        program_id: DBC_ID,
        accounts: dbc::client::accounts::CreateConfig {
            config: *config,
            fee_claimer: *fee_claimer,
            leftover_receiver: *leftover,
            quote_mint: NATIVE_MINT,
            payer: *payer,
            system_program: anchor_lang::system_program::ID,
            event_authority: event_authority(&DBC_ID),
            program: DBC_ID,
        }
        .to_account_metas(None),
        data: dbc::client::args::CreateConfig { config_parameters: params }.data(),
    }
}

impl Env {
    /// DBC create_config signed by a fresh config keypair (the partner's off-chain step).
    pub fn create_config(&mut self, fee_claimer: &Pubkey, migration_fee_percentage: u8) -> Pubkey {
        self.create_config_with(fee_claimer, rook_config_params(migration_fee_percentage))
    }

    pub fn create_config_with(&mut self, fee_claimer: &Pubkey, params: dbct::ConfigParameters) -> Pubkey {
        let config = Keypair::new();
        let ix = ix_create_config(&self.payer.pubkey(), &config.pubkey(), fee_claimer, fee_claimer, params);
        self.send_ok("dbc.create_config", &[ix], &[&config]);
        config.pubkey()
    }

    /// DBC initialize_virtual_pool_with_spl_token: a creator launches a coin under `config`.
    pub fn launch(&mut self, config: &Pubkey) -> Coin {
        let creator = self.funded(10);
        let mint = Keypair::new();
        let pool = dbc_pool_pda(config, &mint.pubkey());
        let base_vault = dbc_vault(&mint.pubkey(), &pool);
        let quote_vault = dbc_vault(&NATIVE_MINT, &pool);
        let metadata = pda(&[b"metadata", MPL_ID.as_ref(), mint.pubkey().as_ref()], &MPL_ID);
        let ix = Instruction {
            program_id: DBC_ID,
            accounts: dbc::client::accounts::InitializeVirtualPoolWithSplToken {
                config: *config,
                pool_authority: DBC_POOL_AUTHORITY,
                creator: creator.pubkey(),
                base_mint: mint.pubkey(),
                quote_mint: NATIVE_MINT,
                pool,
                base_vault,
                quote_vault,
                mint_metadata: metadata,
                metadata_program: MPL_ID,
                payer: self.payer.pubkey(),
                token_quote_program: spl_token::ID,
                token_program: spl_token::ID,
                system_program: anchor_lang::system_program::ID,
                event_authority: event_authority(&DBC_ID),
                program: DBC_ID,
            }
            .to_account_metas(None),
            data: dbc::client::args::InitializeVirtualPoolWithSplToken {
                params: dbct::InitializePoolParameters { name: "Rook Test".into(), symbol: "RKT".into(), uri: "https://rook.example/c.json".into() },
            }
            .data(),
        };
        self.send_ok("dbc.initialize_virtual_pool_with_spl_token", &[ix], &[&creator, &mint]);
        Coin { config: *config, mint: mint.pubkey(), pool, base_vault, quote_vault, creator }
    }

    /// DBC swap2 SOL → coin. mode: 0 ExactIn, 1 PartialFill (stops at the migration threshold), 2 ExactOut.
    pub fn buy(&mut self, coin: &Coin, buyer: &Keypair, lamports: u64, mode: u8) -> TxResult {
        let wsol = self.wsol(buyer, lamports);
        let out = get_associated_token_address(&buyer.pubkey(), &coin.mint);
        let ixs = [
            ata_ix::create_associated_token_account_idempotent(&self.payer.pubkey(), &buyer.pubkey(), &coin.mint, &spl_token::ID),
            Instruction {
                program_id: DBC_ID,
                accounts: dbc::client::accounts::Swap2 {
                    pool_authority: DBC_POOL_AUTHORITY,
                    config: coin.config,
                    pool: coin.pool,
                    input_token_account: wsol,
                    output_token_account: out,
                    base_vault: coin.base_vault,
                    quote_vault: coin.quote_vault,
                    base_mint: coin.mint,
                    quote_mint: NATIVE_MINT,
                    payer: buyer.pubkey(),
                    token_base_program: spl_token::ID,
                    token_quote_program: spl_token::ID,
                    referral_token_account: None,
                    event_authority: event_authority(&DBC_ID),
                    program: DBC_ID,
                }
                .to_account_metas(None),
                data: dbc::client::args::Swap2 { params: dbct::SwapParameters2 { amount_0: lamports, amount_1: 0, swap_mode: mode } }.data(),
            },
        ];
        self.send(&ixs, &[buyer])
    }

    /// DBC migration_damm_v2 (permissionless; any payer). Returns (damm pool, partner/creator position NFT mints).
    pub fn migrate(&mut self, coin: &Coin, payer: &Keypair) -> Migrated {
        let damm_pool = damm_pool_pda(&coin.mint);
        let nft1 = Keypair::new();
        let nft2 = Keypair::new();
        let mut metas = dbc::client::accounts::MigrationDammV2 {
            virtual_pool: coin.pool,
            migration_metadata: anchor_lang::system_program::ID, // deprecated, unused by DBC 0.2.1
            config: coin.config,
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
            base_mint: coin.mint,
            quote_mint: NATIVE_MINT,
            token_a_vault: damm_vault(&coin.mint, &damm_pool),
            token_b_vault: damm_vault(&NATIVE_MINT, &damm_pool),
            base_vault: coin.base_vault,
            quote_vault: coin.quote_vault,
            payer: payer.pubkey(),
            token_base_program: spl_token::ID,
            token_quote_program: spl_token::ID,
            token_2022_program: TOKEN_2022_ID,
            damm_event_authority: event_authority(&DAMM_V2_ID),
            system_program: anchor_lang::system_program::ID,
        }
        .to_account_metas(None);
        metas.push(AccountMeta::new_readonly(DAMM_CONFIG_CUSTOMIZABLE, false));
        let ix = Instruction { program_id: DBC_ID, accounts: metas, data: dbc::client::args::MigrationDammV2 {}.data() };
        self.send_ok("dbc.migration_damm_v2", &[ix], &[payer, &nft1, &nft2]);
        Migrated { damm_pool, nft1: nft1.pubkey(), nft2: nft2.pubkey() }
    }

    /// DAMM v2 swap SOL → coin on the graduated pool (to accrue LP fees).
    pub fn damm_buy(&mut self, coin: &Coin, m: &Migrated, buyer: &Keypair, lamports: u64) -> TransactionMetadata {
        let wsol = self.wsol(buyer, lamports);
        let out = get_associated_token_address(&buyer.pubkey(), &coin.mint);
        let ixs = [
            ata_ix::create_associated_token_account_idempotent(&self.payer.pubkey(), &buyer.pubkey(), &coin.mint, &spl_token::ID),
            Instruction {
                program_id: DAMM_V2_ID,
                accounts: cp_amm::client::accounts::Swap {
                    pool_authority: DAMM_V2_POOL_AUTHORITY,
                    pool: m.damm_pool,
                    input_token_account: wsol,
                    output_token_account: out,
                    token_a_vault: damm_vault(&coin.mint, &m.damm_pool),
                    token_b_vault: damm_vault(&NATIVE_MINT, &m.damm_pool),
                    token_a_mint: coin.mint,
                    token_b_mint: NATIVE_MINT,
                    payer: buyer.pubkey(),
                    token_a_program: spl_token::ID,
                    token_b_program: spl_token::ID,
                    referral_token_account: None,
                    event_authority: event_authority(&DAMM_V2_ID),
                    program: DAMM_V2_ID,
                }
                .to_account_metas(None),
                data: cp_amm::client::args::Swap { _params: cp_amm::types::SwapParameters { amount_in: lamports, minimum_amount_out: 0 } }.data(),
            },
        ];
        self.send_ok("damm_v2.swap", &ixs, &[buyer])
    }
}

pub struct Migrated {
    pub damm_pool: Pubkey,
    pub nft1: Pubkey,
    pub nft2: Pubkey,
}

// ------------------------------------------------------------------------------------------------- rook_backing
/// A collection as rook_backing sees it, with the mock rook_core accounts.
pub struct Collection {
    pub listing: Pubkey,
    pub registry: Pubkey,
    pub module: Pubkey,
    pub module_vault: Pubkey,
    pub wrapper: Pubkey,
    pub wx_mint: Pubkey,
    pub partner: Pubkey,
    pub claimer: Pubkey,
    pub admin: Keypair,
}

impl Env {
    /// Gives rook_backing's ProgramData an upgrade authority (LiteSVM deploys with none).
    pub fn set_upgrade_authority(&mut self, authority: &Pubkey) {
        let pd = pda(&[rook_backing::ID.as_ref()], &anchor_lang::solana_program::bpf_loader_upgradeable::ID);
        let mut acc = self.svm.get_account(&pd).unwrap();
        acc.data[12] = 1;
        acc.data[13..45].copy_from_slice(authority.as_ref());
        self.svm.set_account(pd, acc).unwrap();
    }

    pub fn ix_init_backing(&self, authority: &Pubkey) -> Instruction {
        Instruction {
            program_id: rook_backing::ID,
            accounts: rook_backing::accounts::InitBacking {
                authority: *authority,
                settings: settings_pda(),
                program: rook_backing::ID,
                program_data: pda(&[rook_backing::ID.as_ref()], &anchor_lang::solana_program::bpf_loader_upgradeable::ID),
                system_program: anchor_lang::system_program::ID,
            }
            .to_account_metas(None),
            data: rook_backing::instruction::InitBacking {}.data(),
        }
    }

    fn ensure_settings(&mut self) {
        if !self.exists(&settings_pda()) {
            let upgrade = self.funded(1);
            self.set_upgrade_authority(&upgrade.pubkey());
            let ix = self.ix_init_backing(&upgrade.pubkey());
            self.send_ok("init_backing", &[ix], &[&upgrade]);
        }
    }

    pub fn ix_register_partner(&self, admin: &Pubkey, registry: &Pubkey, params: rook_backing::PartnerParams) -> Instruction {
        Instruction {
            program_id: rook_backing::ID,
            accounts: rook_backing::accounts::RegisterPartner {
                admin: *admin,
                settings: settings_pda(),
                registry: *registry,
                partner: partner_pda(&params.listing),
                listing: params.listing,
                module: params.module,
                module_vault: params.module_vault,
                wrapper: params.wrapper,
                system_program: anchor_lang::system_program::ID,
            }
            .to_account_metas(None),
            data: rook_backing::instruction::RegisterPartner { params }.data(),
        }
    }

    /// init_backing + register_partner for real rook_core accounts; `admin` must be the real registry's owner.
    pub fn register_collection(&mut self, admin: &Keypair, listing: Pubkey, registry: Pubkey, module: Pubkey, module_vault: Pubkey, wrapper: Pubkey, wx_mint: Pubkey) -> Collection {
        self.ensure_settings();
        let params = rook_backing::PartnerParams { listing, registry, module, module_vault, wrapper, wx_mint };
        let ix = self.ix_register_partner(&admin.pubkey(), &registry, params);
        self.send_ok("register_partner", &[ix], &[admin]);
        let partner = partner_pda(&listing);
        Collection { listing, registry, module, module_vault, wrapper, wx_mint, partner, claimer: claimer_pda(&listing), admin: admin.insecure_clone() }
    }

    /// The mock rook_core registry at the real PDA ["registry"]: rook_core's `Registry` discriminator and `owner` (the shared
    /// layout, `rook_layouts::registry`, which rook_backing's admin checks read), then, from byte 64 on, the 32-byte keys of
    /// the converters (the mock `convert` scans the data in 32-byte chunks).
    pub fn mock_registry_add_converter(&mut self, converter: &Pubkey) {
        let k = registry_pda();
        let mut data = match self.svm.get_account(&k) {
            Some(a) if a.lamports > 0 => a.data,
            _ => {
                let mut d = vec![0u8; 64];
                d[..8].copy_from_slice(&rook_layouts::registry::DISCRIMINATOR);
                d[8..40].copy_from_slice(self.rook_owner.pubkey().as_ref());
                d
            }
        };
        data.extend_from_slice(converter.as_ref());
        let lamports = self.svm.minimum_balance_for_rent_exemption(data.len());
        self.svm.set_account(k, SolAccount { lamports, data, owner: ROOK_CORE_ID, executable: false, rent_epoch: 0 }).unwrap();
    }

    /// Whether the mock registry lists `key` among its converters (from byte 64 on).
    pub fn mock_registry_has(&self, key: &Pubkey) -> bool {
        match self.svm.get_account(&registry_pda()) {
            Some(a) if a.data.len() > 64 => a.data[64..].chunks(32).any(|c| c == key.as_ref()),
            _ => false,
        }
    }

    /// Rewrites the mock registry's `owner` (what rook_core's accept_owner would do).
    pub fn mock_registry_set_owner(&mut self, owner: &Pubkey) {
        let k = registry_pda();
        let mut a = self.svm.get_account(&k).expect("mock registry");
        a.data[8..40].copy_from_slice(owner.as_ref());
        self.svm.set_account(k, a).unwrap();
    }

    fn set_core_account(&mut self, key: Pubkey, data: Vec<u8>, extra: u64) {
        let lamports = self.svm.minimum_balance_for_rent_exemption(data.len()) + extra;
        self.svm.set_account(key, SolAccount { lamports, data, owner: ROOK_CORE_ID, executable: false, rent_epoch: 0 }).unwrap();
    }

    /// rook_core-shaped accounts of one collection at rook_core's PDAs, as rook_backing reads them (`rook_layouts`): a
    /// `Listing` (id), its `Module` (listing, wrapper, vault, `conv_bps` 0, `accepted` = the mock's price), `ModuleVault` and
    /// `Wrapper` (listing, wx_mint). Returns (listing, module, module_vault, wrapper).
    pub fn mock_core_accounts(&mut self, wx_mint_of: impl FnOnce(&mut Self, &Pubkey) -> Pubkey) -> (Pubkey, Pubkey, Pubkey, Pubkey, Pubkey) {
        let id: u32 = rand_u32();
        let listing = pda(&[rook_layouts::core_seeds::LISTING, &id.to_le_bytes()], &ROOK_CORE_ID);
        let module = core_module_pda(&listing);
        let module_vault = core_module_vault_pda(&module);
        let wrapper = core_wrapper_pda(&listing);
        let wx_mint = wx_mint_of(self, &wrapper);
        let mut l = vec![0u8; 284];
        l[..8].copy_from_slice(&rook_layouts::listing::DISCRIMINATOR);
        l[8..12].copy_from_slice(&id.to_le_bytes());
        self.set_core_account(listing, l, 0);
        let mut m = vec![0u8; rook_layouts::module::LEN];
        m[..8].copy_from_slice(&rook_layouts::module::DISCRIMINATOR);
        m[8..40].copy_from_slice(listing.as_ref());
        m[40..72].copy_from_slice(wrapper.as_ref());
        m[72..104].copy_from_slice(module_vault.as_ref());
        let a = rook_layouts::module::ACCEPTED;
        m[a..a + 8].copy_from_slice(&MOCK_ACCEPTED.to_le_bytes());
        self.set_core_account(module, m, 0);
        self.set_core_account(module_vault, rook_layouts::module_vault::DISCRIMINATOR.to_vec(), LAMPORTS);
        let mut w = vec![0u8; 662];
        w[..8].copy_from_slice(&rook_layouts::wrapper::DISCRIMINATOR);
        w[8..40].copy_from_slice(listing.as_ref());
        w[40..72].copy_from_slice(wx_mint.as_ref());
        self.set_core_account(wrapper, w, 0);
        (listing, module, module_vault, wrapper, wx_mint)
    }

    /// Rewrites the mock module's accepted floor (rook_backing's `back` bounds its price by it, audit A3-06).
    pub fn mock_set_accepted(&mut self, module: &Pubkey, accepted: u64) {
        let mut a = self.svm.get_account(module).expect("mock module");
        let o = rook_layouts::module::ACCEPTED;
        a.data[o..o + 8].copy_from_slice(&accepted.to_le_bytes());
        self.svm.set_account(*module, a).unwrap();
    }

    /// init_backing + register_partner, with mock rook_core accounts (the registry lists rook_backing's ["converter"] PDA;
    /// its owner, `self.rook_owner`, is the admin).
    pub fn setup_collection(&mut self) -> Collection {
        assert!(
            rook_core_so().1,
            "setup_collection builds mock rook_core accounts; with ROOK_CORE_REAL=1 use tests/real_rook_core.rs (real-rook-core feature)"
        );
        self.ensure_settings();
        let admin = self.rook_owner.insecure_clone();
        let (listing, module, module_vault, wrapper, wx_mint) = self.mock_core_accounts(|e, w| e.create_mint(w, 9));
        let claimer = claimer_pda(&listing);
        if !self.mock_registry_has(&converter_pda()) {
            self.mock_registry_add_converter(&converter_pda());
        }
        let registry = registry_pda();
        // rook_core treasury exists (rent-exempt) so lamport credits land on an existing account
        if !self.exists(&treasury_pda()) {
            self.svm.airdrop(&treasury_pda(), LAMPORTS).unwrap();
        }
        let params = rook_backing::PartnerParams { listing, registry, module, module_vault, wrapper, wx_mint };
        let ix = self.ix_register_partner(&admin.pubkey(), &registry, params);
        self.send_ok("register_partner", &[ix], &[&admin]);
        Collection { listing, registry, module, module_vault, wrapper, wx_mint, partner: partner_pda(&listing), claimer, admin }
    }

    pub fn ix_register_config(&self, col: &Collection, admin: &Pubkey, config: &Pubkey) -> Instruction {
        Instruction {
            program_id: rook_backing::ID,
            accounts: rook_backing::accounts::RegisterPartnerConfig {
                admin: *admin,
                settings: settings_pda(),
                registry: col.registry,
                partner: col.partner,
                dbc_config: *config,
            }
            .to_account_metas(None),
            data: rook_backing::instruction::RegisterPartnerConfig {}.data(),
        }
    }

    pub fn ix_open_vault(&self, col: &Collection, pool: &Pubkey, coin_mint: &Pubkey) -> Instruction {
        Instruction {
            program_id: rook_backing::ID,
            accounts: rook_backing::accounts::OpenVault {
                payer: self.payer.pubkey(),
                partner: col.partner,
                dbc_pool: *pool,
                coin_mint: *coin_mint,
                wx_mint: col.wx_mint,
                vault: vault_pda(coin_mint),
                vault_wx: vault_wx_pda(coin_mint),
                wx_token_program: spl_token::ID,
                system_program: anchor_lang::system_program::ID,
            }
            .to_account_metas(None),
            data: rook_backing::instruction::OpenVault {}.data(),
        }
    }

    pub fn ix_back(&self, col: &Collection, coin: &Coin, cranker: &Pubkey, min_units: u64) -> Instruction {
        Instruction {
            program_id: rook_backing::ID,
            accounts: rook_backing::accounts::Back {
                cranker: *cranker,
                settings: settings_pda(),
                partner: col.partner,
                vault: vault_pda(&coin.mint),
                dbc_pool: coin.pool,
                dbc_config: coin.config,
                dbc_quote_vault: coin.quote_vault,
                dbc_pool_authority: DBC_POOL_AUTHORITY,
                quote_mint: NATIVE_MINT,
                claimer: col.claimer,
                claimer_wsol: claimer_wsol_pda(&col.listing),
                dbc_event_authority: event_authority(&DBC_ID),
                dbc_program: DBC_ID,
                rook_core: ROOK_CORE_ID,
                converter: converter_pda(),
                registry: col.registry,
                listing: col.listing,
                module: col.module,
                module_vault: col.module_vault,
                wrapper: col.wrapper,
                wx_mint: col.wx_mint,
                vault_wx: vault_wx_pda(&coin.mint),
                coin_mint: coin.mint,
                token_program: spl_token::ID,
                wx_token_program: spl_token::ID,
                system_program: anchor_lang::system_program::ID,
            }
            .to_account_metas(None),
            data: rook_backing::instruction::Back { min_units }.data(),
        }
    }

    pub fn ix_redeem(&self, coin_mint: &Pubkey, wx_mint: &Pubkey, holder: &Pubkey, holder_coin: &Pubkey, holder_wx: &Pubkey, amount: u64) -> Instruction {
        Instruction {
            program_id: rook_backing::ID,
            accounts: rook_backing::accounts::RedeemBacking {
                holder: *holder,
                vault: vault_pda(coin_mint),
                coin_mint: *coin_mint,
                holder_coin: *holder_coin,
                vault_wx: vault_wx_pda(coin_mint),
                wx_mint: *wx_mint,
                holder_wx: *holder_wx,
                coin_token_program: spl_token::ID,
                wx_token_program: spl_token::ID,
            }
            .to_account_metas(None),
            data: rook_backing::instruction::RedeemBacking { amount }.data(),
        }
    }

    pub fn ix_sweep_partner_fees(&self, col: &Collection, coin: &Coin, cranker: &Pubkey, treasury: &Pubkey) -> Instruction {
        Instruction {
            program_id: rook_backing::ID,
            accounts: rook_backing::accounts::SweepPartnerFees {
                cranker: *cranker,
                settings: settings_pda(),
                partner: col.partner,
                vault: vault_pda(&coin.mint),
                treasury: *treasury,
                dbc_pool: coin.pool,
                dbc_config: coin.config,
                dbc_base_vault: coin.base_vault,
                dbc_quote_vault: coin.quote_vault,
                dbc_pool_authority: DBC_POOL_AUTHORITY,
                coin_mint: coin.mint,
                quote_mint: NATIVE_MINT,
                claimer: col.claimer,
                claimer_wsol: claimer_wsol_pda(&col.listing),
                claimer_coin: get_associated_token_address(&col.claimer, &coin.mint),
                dbc_event_authority: event_authority(&DBC_ID),
                dbc_program: DBC_ID,
                coin_token_program: spl_token::ID,
                token_program: spl_token::ID,
                system_program: anchor_lang::system_program::ID,
            }
            .to_account_metas(None),
            data: rook_backing::instruction::SweepPartnerFees {}.data(),
        }
    }

    pub fn ix_sweep_lp_fees(&self, col: &Collection, coin: &Coin, m: &Migrated, nft: &Pubkey, cranker: &Pubkey, treasury: &Pubkey) -> Instruction {
        Instruction {
            program_id: rook_backing::ID,
            accounts: rook_backing::accounts::SweepLpFees {
                cranker: *cranker,
                settings: settings_pda(),
                partner: col.partner,
                vault: vault_pda(&coin.mint),
                treasury: *treasury,
                damm_pool: m.damm_pool,
                damm_position: damm_position(nft),
                position_nft_account: damm_position_nft_account(nft),
                damm_token_a_vault: damm_vault(&coin.mint, &m.damm_pool),
                damm_token_b_vault: damm_vault(&NATIVE_MINT, &m.damm_pool),
                damm_pool_authority: DAMM_V2_POOL_AUTHORITY,
                coin_mint: coin.mint,
                quote_mint: NATIVE_MINT,
                claimer: col.claimer,
                claimer_wsol: claimer_wsol_pda(&col.listing),
                claimer_coin: get_associated_token_address(&col.claimer, &coin.mint),
                damm_event_authority: event_authority(&DAMM_V2_ID),
                damm_program: DAMM_V2_ID,
                coin_token_program: spl_token::ID,
                token_program: spl_token::ID,
                system_program: anchor_lang::system_program::ID,
            }
            .to_account_metas(None),
            data: rook_backing::instruction::SweepLpFees {}.data(),
        }
    }

    /// One graduated, Rook-backed coin: config registered, coin launched, vault opened, bought to the threshold
    /// (after the anti-sniper window), migrated. `back` is left to the caller.
    pub fn graduated_coin(&mut self, col: &Collection, migration_fee_percentage: u8) -> (Coin, Migrated) {
        let config = self.create_config(&col.claimer, migration_fee_percentage);
        let ix = self.ix_register_config(col, &col.admin.pubkey(), &config);
        self.send_ok("register_partner_config", &[ix], &[&col.admin]);
        let coin = self.launch(&config);
        let ix = self.ix_open_vault(col, &coin.pool, &coin.mint);
        self.send_ok("open_vault", &[ix], &[]);
        self.warp(700);
        let whale = self.funded(200);
        self.buy(&coin, &whale, 150 * LAMPORTS, 1).map_err(|e| e.meta.logs.join("\n")).expect("buy to threshold");
        let anyone = self.funded(5);
        let m = self.migrate(&coin, &anyone);
        (coin, m)
    }
}

/// A pseudo-random u32 (a fresh key's first bytes): distinct mock listing ids within a test.
pub fn rand_u32() -> u32 {
    let k = Keypair::new().pubkey().to_bytes();
    u32::from_le_bytes([k[0], k[1], k[2], k[3]])
}
