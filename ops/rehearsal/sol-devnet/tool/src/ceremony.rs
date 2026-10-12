//! rook-ceremony: day 1 of the Solana launch WITHOUT deploying Rook's programs (docs/v9/solana-migration.md §2.9 D-f, the runbook
//! docs/v9/launch-day1-solana.md). Driven by ops/solana/ceremony-day1.sh, which generates the reserved program keypairs.
//!
//!   rook-ceremony day1 --rook-core <id> --rook-backing <id> --launch <launch.json> --out <dir> [--rpc <url>]
//!                      [--fund-wallet <pubkey>] [--rook-name N --rook-symbol S --rook-uri U] [--sim-payer <pubkey>]
//!                      [--payer <ceremony keypair.json> --send]
//!   rook-ceremony preflight --rook-core <id> --rook-backing <id> --launch <launch.json> --plan <day1-plan.json> --rpc <url> [--rpc2 <url>]
//!   rook-ceremony init      (same + --payer <ceremony keypair.json> [--send]: init_registry(owner = the ceremony key) / init_backing)
//!   rook-ceremony list      --launch --plan --custody <custody.json> --rpc [--rpc2] --canister-sol <pubkey> [--ops <fueler hot key>]
//!                           [--owner <ceremony keypair.json> --send]
//!   rook-ceremony propose-owner --plan --rpc [--rpc2] --owner <ceremony keypair.json> [--send]   (propose_owner(fund wallet))
//!   rook-ceremony accept-owner  --plan --rpc (--print-tx | --ledger usb://ledger[?key=…] [--send])  (the fund wallet's ONE signature)
//!   rook-ceremony verify-owner  --plan --rpc [--rpc2]   (Registry.owner = the fund wallet, nothing pending, upgrade authorities)
//!   rook-ceremony pins [--write]   (ops/solana/day1-pins.json from launch-params.mjs --meteora and nft-pilot.json)
//!
//! Audit A6 (docs/v9/audit-2026-10/a6-launch-ops.md, "Fixes"): the launch file's parameters and listing order must be byte-identical to
//! the committed pins (ceremony_checks.rs), --send needs review.reviewed_by and a permanent ROOK uri, the cluster is checked against
//! the launch file everywhere, and the listing ceremony is ceremony_list.rs.
//!
//! For every listing of the launch file's `listing_order` it derives, from the RESERVED (not deployed) program ids:
//!   listing  = rook_core PDA ["listing", u32 LE id]      claimer = rook_backing PDA ["claimer", listing]
//! and builds Meteora DBC's `create_config` with the launch file's `meteora.config_parameters` and fee_claimer = leftover_receiver
//! = claimer. Then the genesis ROOK coin (decision D-g): its own config (fee claimer = leftover receiver = the fund wallet, no
//! migration fee, the creator's curve-fee share at DBC's maximum) + the pool launch + `transfer_pool_creator` to the fund wallet, in
//! ONE transaction, so the fund wallet (the operator's own key) signs nothing here: it only funds the ceremony payer with one plain
//! transfer. A config keypair per listing is created 0600 in <out>/configs/ (reused on a rerun). With --rpc, every transaction is
//! SIMULATED (sigVerify off, nothing sent) and the config account DBC would write is decoded with rook_backing's own reader and
//! checked against what `register_partner_config` will require later and against decision D-e. The plan (ids, PDAs, unsigned
//! messages, the exact SOL cost) is written to <out>/day1-plan.json. Nothing is sent without --send and --payer; a config that
//! already exists is checked, never recreated. No secret is printed.
use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread::sleep,
    time::{Duration, Instant},
};

use anchor_lang::{
    prelude::Pubkey,
    solana_program::{instruction::Instruction, system_program},
    InstructionData, ToAccountMetas,
};
use base64::Engine;
use rook_backing::{
    constants::{DBC_ID, NATIVE_MINT},
    dbc::{parse_config, POOL_CONFIG_LEN},
    dynamic_bonding_curve::{self as dbc, types as dbct},
};
use serde_json::{json, Value};
use solana_compute_budget_interface::ComputeBudgetInstruction;
use solana_keypair::Keypair;
use solana_message::{Message, VersionedMessage};
use solana_signer::Signer;
use solana_transaction::versioned::VersionedTransaction;

mod ceremony_checks;
mod ceremony_list;
mod ceremony_owner;
use ceremony_checks::*;

const MAINNET_GENESIS: &str = "5eykt4UsFv8P8NJdTREpY1vzqKqZKvdpKuc147dw2N9d";
const DEVNET_GENESIS: &str = "EtWTRABZaYq6iMfeYKouRu166VU2xqa1wcaWoxPkrZBG";
/// The base fee of one signature (lamports). Every create_config carries two: the payer and the config keypair.
const LAMPORTS_PER_SIGNATURE: u64 = 5_000;
/// Compute-unit limit set on every create_config (measured 36,817 CU in Q1; DBC's own logs add a little).
const CU_LIMIT: u32 = 60_000;
/// The fee payer of a SIMULATION when the ceremony payer is not funded yet: a large plain system account on mainnet-beta (an exchange
/// hot wallet, ≈ 1.35 M SOL on 2026-10-09). sigVerify is off: nothing is signed or sent; its balance only lets the simulation run.
/// --sim-payer overrides it (on devnet, any funded account).
const SIM_FEE_PAYER: &str = "5tzFkiKscXHK5ZXCGbXZxdw7gTjjD1mBwuoFbhUvuAi9";
/// The operator's fund wallet (decision D-g; ops/solana/launch-params.mjs FUND_WALLET). --fund-wallet overrides.
pub const FUND_WALLET: &str = "GGgXnNgtY4ySfz8PLAAwkoh8Ztj7p6jrM9towSaa6Wuy";
const MPL_ID: Pubkey = anchor_lang::pubkey!("metaqbxxUerdq28cj1RbAWkYQm3ybzjb6a8bt518x1s");

// ---- args, keys -------------------------------------------------------------------------------------------------------------------

fn args() -> Result<(String, BTreeMap<String, String>), String> {
    let v = std::env::args().skip(1).collect::<Vec<_>>();
    let mut m = BTreeMap::new();
    let mut cmd = None;
    let mut i = 0;
    while i < v.len() {
        if let Some(k) = v[i].strip_prefix("--") {
            let val = if i + 1 < v.len() && !v[i + 1].starts_with("--") {
                i += 1;
                v[i].clone()
            } else {
                "true".into()
            };
            m.insert(k.to_string(), val);
        } else if cmd.is_none() {
            cmd = Some(v[i].clone());
        } else {
            return Err(format!("unexpected argument {}", v[i]));
        }
        i += 1;
    }
    Ok((cmd.ok_or("a command is required: day1 | preflight | init | list | propose-owner | accept-owner | verify-owner | pins")?, m))
}

fn read_kp(p: &Path) -> Result<Keypair, String> {
    let s = fs::read_to_string(p).map_err(|e| format!("{}: {e}", p.display()))?;
    let b: Vec<u8> = serde_json::from_str(&s).map_err(|e| format!("{}: not a keypair file: {e}", p.display()))?;
    Keypair::try_from(&b[..]).map_err(|e| format!("{}: {e}", p.display()))
}

/// A config keypair: created 0600 if absent (create_new: never overwritten), else read back. The secret never leaves the file.
fn config_kp(p: &Path) -> Result<Keypair, String> {
    if !p.exists() {
        use std::os::unix::fs::OpenOptionsExt;
        let k = Keypair::new();
        let mut f = fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(p).map_err(|e| format!("{}: {e}", p.display()))?;
        f.write_all(serde_json::to_string(&k.to_bytes().to_vec()).unwrap().as_bytes()).map_err(|e| e.to_string())?;
    }
    read_kp(p)
}

// ---- PDAs (from the reserved ids, at run time: the compiled declare_id! of this tool is the dev key) ------------------------------

pub fn listing_pda(rook_core: &Pubkey, id: u32) -> Pubkey {
    Pubkey::find_program_address(&[b"listing", &id.to_le_bytes()], rook_core).0
}
pub fn claimer_pda(rook_backing: &Pubkey, listing: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(&[rook_backing::constants::CLAIMER_SEED, listing.as_ref()], rook_backing).0
}
pub fn converter_pda(rook_backing: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(&[rook_backing::constants::CONVERTER_SEED], rook_backing).0
}

// ---- the launch file's meteora section → ConfigParameters -------------------------------------------------------------------------

fn leaf<'a>(v: &'a Value, path: &str) -> Result<&'a Value, String> {
    let mut cur = v;
    for k in path.split('.') {
        cur = cur.get(k).ok_or(format!("meteora.config_parameters.{path}: missing"))?;
    }
    cur.get("value").ok_or(format!("meteora.config_parameters.{path}: no value"))
}
fn int<T: std::str::FromStr + TryFrom<u64>>(v: &Value, path: &str) -> Result<T, String> {
    let x = leaf(v, path)?;
    let s = match x {
        Value::String(s) => s.clone(),
        Value::Number(n) if n.is_u64() => n.to_string(),
        _ => return Err(format!("meteora.config_parameters.{path}: not an unsigned integer: {x}")),
    };
    s.parse::<T>().map_err(|_| format!("meteora.config_parameters.{path}: {s} out of range"))
}
fn big(s: &Value, what: &str) -> Result<u128, String> {
    s.as_str().ok_or(format!("{what}: a decimal string"))?.parse::<u128>().map_err(|_| format!("{what}: not a u128"))
}
fn vesting(v: &Value, p: &str) -> Result<dbct::LiquidityVestingInfoParams, String> {
    Ok(dbct::LiquidityVestingInfoParams {
        vesting_percentage: int(v, &format!("{p}.vesting_percentage"))?,
        bps_per_period: int(v, &format!("{p}.bps_per_period"))?,
        number_of_periods: int(v, &format!("{p}.number_of_periods"))?,
        cliff_duration_from_migration_time: int(v, &format!("{p}.cliff_duration_from_migration_time"))?,
        frequency: int(v, &format!("{p}.frequency"))?,
    })
}

pub fn config_parameters(cp: &Value) -> Result<dbct::ConfigParameters, String> {
    if !leaf(cp, "pool_fees.dynamic_fee")?.is_null() {
        return Err("meteora: a dynamic fee is not part of decision D-e (null expected)".into());
    }
    let pre = int::<u64>(cp, "token_supply.pre_migration_token_supply")?;
    let post = int::<u64>(cp, "token_supply.post_migration_token_supply")?;
    let curve = leaf(cp, "curve")?
        .as_array()
        .ok_or("meteora: curve is a list")?
        .iter()
        .enumerate()
        .map(|(i, p)| {
            Ok(dbct::LiquidityDistributionParameters {
                sqrt_price: big(&p["sqrt_price"], &format!("curve[{i}].sqrt_price"))?,
                liquidity: big(&p["liquidity"], &format!("curve[{i}].liquidity"))?,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(dbct::ConfigParameters {
        pool_fees: dbct::PoolFeeParameters {
            base_fee: dbct::BaseFeeParameters {
                cliff_fee_numerator: int(cp, "pool_fees.base_fee.cliff_fee_numerator")?,
                first_factor: int(cp, "pool_fees.base_fee.first_factor")?,
                second_factor: int(cp, "pool_fees.base_fee.second_factor")?,
                third_factor: int(cp, "pool_fees.base_fee.third_factor")?,
                base_fee_mode: int(cp, "pool_fees.base_fee.base_fee_mode")?,
            },
            dynamic_fee: None,
        },
        collect_fee_mode: int(cp, "collect_fee_mode")?,
        migration_option: int(cp, "migration_option")?,
        activation_type: int(cp, "activation_type")?,
        token_type: int(cp, "token_type")?,
        token_decimal: int(cp, "token_decimal")?,
        partner_liquidity_percentage: int(cp, "partner_liquidity_percentage")?,
        partner_permanent_locked_liquidity_percentage: int(cp, "partner_permanent_locked_liquidity_percentage")?,
        creator_liquidity_percentage: int(cp, "creator_liquidity_percentage")?,
        creator_permanent_locked_liquidity_percentage: int(cp, "creator_permanent_locked_liquidity_percentage")?,
        migration_quote_threshold: int(cp, "migration_quote_threshold")?,
        sqrt_start_price: big(leaf(cp, "sqrt_start_price")?, "sqrt_start_price")?,
        locked_vesting: dbct::LockedVestingParams {
            amount_per_period: int(cp, "locked_vesting.amount_per_period")?,
            cliff_duration_from_migration_time: int(cp, "locked_vesting.cliff_duration_from_migration_time")?,
            frequency: int(cp, "locked_vesting.frequency")?,
            number_of_period: int(cp, "locked_vesting.number_of_period")?,
            cliff_unlock_amount: int(cp, "locked_vesting.cliff_unlock_amount")?,
        },
        migration_fee_option: int(cp, "migration_fee_option")?,
        token_supply: Some(dbct::TokenSupplyParams { pre_migration_token_supply: pre, post_migration_token_supply: post }),
        creator_trading_fee_percentage: int(cp, "creator_trading_fee_percentage")?,
        token_update_authority: int(cp, "token_update_authority")?,
        migration_fee: dbct::MigrationFee {
            fee_percentage: int(cp, "migration_fee.fee_percentage")?,
            creator_fee_percentage: int(cp, "migration_fee.creator_fee_percentage")?,
        },
        migrated_pool_fee: dbct::MigratedPoolFee {
            collect_fee_mode: int(cp, "migrated_pool_fee.collect_fee_mode")?,
            dynamic_fee: int(cp, "migrated_pool_fee.dynamic_fee")?,
            pool_fee_bps: int(cp, "migrated_pool_fee.pool_fee_bps")?,
        },
        pool_creation_fee: int(cp, "pool_creation_fee")?,
        partner_liquidity_vesting_info: vesting(cp, "partner_liquidity_vesting_info")?,
        creator_liquidity_vesting_info: vesting(cp, "creator_liquidity_vesting_info")?,
        migrated_pool_base_fee_mode: int(cp, "migrated_pool_base_fee_mode")?,
        migrated_pool_market_cap_fee_scheduler_params: dbct::MigratedPoolMarketCapFeeSchedulerParams {
            number_of_period: int(cp, "migrated_pool_market_cap_fee_scheduler_params.number_of_period")?,
            sqrt_price_step_bps: int(cp, "migrated_pool_market_cap_fee_scheduler_params.sqrt_price_step_bps")?,
            scheduler_expiration_duration: int(cp, "migrated_pool_market_cap_fee_scheduler_params.scheduler_expiration_duration")?,
            reduction_factor: int(cp, "migrated_pool_market_cap_fee_scheduler_params.reduction_factor")?,
        },
        enable_first_swap_with_min_fee: leaf(cp, "enable_first_swap_with_min_fee")?.as_bool().ok_or("enable_first_swap_with_min_fee: a bool")?,
        compounding_fee_bps: int(cp, "compounding_fee_bps")?,
        padding: [0; 2],
        curve,
    })
}

/// Decision D-e and rook_backing's `register_partner_config` checks, on the parameters (before anything is built).
pub fn check_params(p: &dbct::ConfigParameters) -> Result<(), String> {
    let mut bad = vec![];
    let mut want = |ok: bool, what: &str| {
        if !ok {
            bad.push(what.to_string())
        }
    };
    // docs/v9/graduation-tiers.md (operator decision 2026-10-10): one of the graduation tiers (85 / 170 / 255 / 510 SOL); the tier's
    // curve is byte-pinned (check_pinned_tiers), so a threshold with another tier's curve is refused there
    want(
        GRADUATION_TIERS_SOL.iter().any(|t| p.migration_quote_threshold == t * LAMPORTS_PER_SOL_U64),
        "D-e: a migration threshold that is a graduation tier (85 / 170 / 255 / 510 SOL)",
    );
    want(p.migration_fee.fee_percentage == 50, "D-e: a 50 % migration fee to the claimer");
    want(p.migration_fee.creator_fee_percentage == 0, "rook_backing: no creator migration-fee share");
    want(p.creator_trading_fee_percentage == 0, "D-e: creator 0 % of the partner's curve fees (Rook keeps 80 %)");
    want(p.migration_option == 1, "rook_backing: DAMM v2 migration");
    want(p.token_type == 0, "rook_backing: a plain SPL coin");
    want(p.collect_fee_mode == 0, "rook_backing: fees collected in SOL");
    want(p.activation_type == 1, "rook_backing: timestamp activation");
    want(p.token_decimal == 6, "D-e: 6 decimals");
    want(
        p.token_supply.as_ref().map(|s| (s.pre_migration_token_supply, s.post_migration_token_supply)) == Some((1_000_000_000_000_000, 1_000_000_000_000_000)),
        "D-e: a fixed 1B supply (10^15 base units, pre = post)",
    );
    want(
        (p.partner_liquidity_percentage, p.creator_liquidity_percentage, p.partner_permanent_locked_liquidity_percentage, p.creator_permanent_locked_liquidity_percentage)
            == (0, 0, 50, 50),
        "D-e: LP permanently locked (50/50, nothing claimable)",
    );
    want(p.partner_liquidity_vesting_info.vesting_percentage == 0 && p.creator_liquidity_vesting_info.vesting_percentage == 0, "D-e: no LP vesting");
    want(
        (p.pool_fees.base_fee.cliff_fee_numerator, p.pool_fees.base_fee.first_factor, p.pool_fees.base_fee.second_factor, p.pool_fees.base_fee.third_factor, p.pool_fees.base_fee.base_fee_mode)
            == (500_000_000, 60, 10, 8_166_666, 0),
        "D-e: the Q1 anti-sniper schedule (50 % → 1 % over 10 min, linear)",
    );
    want(!p.enable_first_swap_with_min_fee, "D-e: no min-fee first swap");
    if bad.is_empty() {
        Ok(())
    } else {
        Err(format!("refused: the launch file's meteora section breaks: {}", bad.join("; ")))
    }
}

/// rook_backing's `register_partner_config` checks on the config account DBC writes (decoded with rook_backing's own reader), plus D-e.
/// `threshold`: the listing's graduation tier in lamports (`listing_threshold`).
pub fn check_written(owner: &Pubkey, data: &[u8], claimer: &Pubkey, threshold: u64) -> Result<(), String> {
    let c = parse_config(owner, data).map_err(|e| format!("rook_backing would not read this config: {e}"))?;
    let checks = [
        (c.fee_claimer == *claimer, "fee_claimer = the claimer PDA"),
        (c.leftover_receiver == *claimer, "leftover_receiver = the claimer PDA"),
        (c.quote_mint == NATIVE_MINT, "quote = wSOL"),
        (c.migration_option == 1, "DAMM v2"),
        (c.token_type == 0, "SPL coin"),
        (c.collect_fee_mode == 0, "fees in SOL"),
        (c.activation_type == 1, "timestamp activation"),
        (c.migration_fee_percentage == 50, "50 % migration fee"),
        (c.creator_migration_fee_percentage == 0, "no creator migration fee"),
        (c.creator_trading_fee_percentage == 0, "creator 0 % of curve fees"),
        (c.migration_quote_threshold == threshold, "the listing's graduation-tier threshold"),
        ((c.partner_permanent_locked_liquidity_percentage, c.creator_permanent_locked_liquidity_percentage) == (50, 50), "LP locked 50/50"),
        ((c.partner_liquidity_percentage, c.creator_liquidity_percentage) == (0, 0), "no claimable LP"),
    ];
    let bad: Vec<&str> = checks.iter().filter(|(ok, _)| !ok).map(|(_, w)| *w).collect();
    if !bad.is_empty() {
        return Err(format!("the config DBC writes fails: {}", bad.join(", ")));
    }
    // audit A6-05 / A3-02: the rest of the account, byte by byte (migrated collect_fee_mode, creation fee, update authority, vesting…)
    check_config_bytes(owner, data, &WantConfig { claimer: *claimer, threshold, migration_fee_percentage: 50, creator_trading_fee_percentage: 0 })
}

pub fn create_config_ix(payer: &Pubkey, config: &Pubkey, claimer: &Pubkey, params: dbct::ConfigParameters) -> Instruction {
    let ea = Pubkey::find_program_address(&[b"__event_authority"], &DBC_ID).0;
    Instruction {
        program_id: DBC_ID,
        accounts: dbc::client::accounts::CreateConfig {
            config: *config,
            fee_claimer: *claimer,
            leftover_receiver: *claimer,
            quote_mint: NATIVE_MINT,
            payer: *payer,
            system_program: system_program::ID,
            event_authority: ea,
            program: DBC_ID,
        }
        .to_account_metas(None),
        data: dbc::client::args::CreateConfig { config_parameters: params }.data(),
    }
}

// ---- the genesis ROOK coin (decision D-g) ------------------------------------------------------------------------------------------

/// `meteora.config_parameters` with `meteora.genesis_rook.overrides` merged in (leaf by leaf).
pub fn rook_parameters_json(meteora: &Value) -> Result<Value, String> {
    let mut cp = meteora.get("config_parameters").ok_or("meteora.config_parameters missing")?.clone();
    merge_leaves(&mut cp, meteora.pointer("/genesis_rook/overrides").ok_or("refused: the launch file has no meteora.genesis_rook (decision D-g)")?);
    Ok(cp)
}

/// D-g on the ROOK parameters: no backing (migration fee 0), the creator's curve-fee share at DBC's maximum, the rest of D-e's family.
pub fn check_rook_params(p: &dbct::ConfigParameters) -> Result<(), String> {
    let mut q = p.clone();
    // the family checks of D-e, with the two D-g differences put back to D-e's values
    q.migration_fee = dbct::MigrationFee { fee_percentage: 50, creator_fee_percentage: 0 };
    q.creator_trading_fee_percentage = 0;
    check_params(&q).map_err(|e| e.replace("meteora section", "genesis ROOK section"))?;
    if p.migration_quote_threshold != 85 * LAMPORTS_PER_SOL_U64 {
        return Err("refused: D-g — the ROOK config graduates at 85 SOL (the graduation tiers are for collection configs)".into());
    }
    if p.migration_fee.fee_percentage != 0 || p.migration_fee.creator_fee_percentage != 0 {
        return Err("refused: D-g — ROOK has no backing: migration fee 0".into());
    }
    if p.creator_trading_fee_percentage != 100 {
        return Err("refused: D-g — the ROOK creator's curve-fee share is DBC's maximum, 100".into());
    }
    Ok(())
}

pub fn check_rook_written(owner: &Pubkey, data: &[u8], fund: &Pubkey) -> Result<(), String> {
    let c = parse_config(owner, data).map_err(|e| format!("not a DBC config: {e}"))?;
    let ok = c.fee_claimer == *fund
        && c.leftover_receiver == *fund
        && c.quote_mint == NATIVE_MINT
        && c.migration_fee_percentage == 0
        && c.creator_trading_fee_percentage == 100
        && c.migration_quote_threshold == 85_000_000_000
        && c.migration_option == 1
        && (c.partner_permanent_locked_liquidity_percentage, c.creator_permanent_locked_liquidity_percentage) == (50, 50);
    if ok {
        check_config_bytes(owner, data, &WantConfig { claimer: *fund, threshold: 85_000_000_000, migration_fee_percentage: 0, creator_trading_fee_percentage: 100 })
    } else {
        Err("the ROOK config DBC writes is not D-g's (fee claimer / leftover = fund wallet, fee 0, creator share 100, 85 SOL, DAMM v2, LP locked)".into())
    }
}

fn ea(program: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(&[b"__event_authority"], program).0
}
pub fn dbc_pool(config: &Pubkey, mint: &Pubkey) -> Pubkey {
    let (hi, lo) = if mint > &NATIVE_MINT { (*mint, NATIVE_MINT) } else { (NATIVE_MINT, *mint) };
    Pubkey::find_program_address(&[b"pool", config.as_ref(), hi.as_ref(), lo.as_ref()], &DBC_ID).0
}

/// The ROOK launch: `initialize_virtual_pool_with_spl_token` (creator = payer = the ceremony key) and, in the same transaction,
/// `transfer_pool_creator` to the fund wallet (DBC allows it while the curve runs), so the fund wallet never signs.
pub fn rook_launch_ixs(config: &Pubkey, mint: &Pubkey, payer: &Pubkey, fund: &Pubkey, name: &str, symbol: &str, uri: &str) -> Vec<Instruction> {
    let pool = dbc_pool(config, mint);
    let vault = |m: &Pubkey| Pubkey::find_program_address(&[b"token_vault", m.as_ref(), pool.as_ref()], &DBC_ID).0;
    let spl = anchor_spl::token::spl_token::ID;
    vec![
        Instruction {
            program_id: DBC_ID,
            accounts: dbc::client::accounts::InitializeVirtualPoolWithSplToken {
                config: *config,
                pool_authority: rook_backing::constants::DBC_POOL_AUTHORITY,
                creator: *payer,
                base_mint: *mint,
                quote_mint: NATIVE_MINT,
                pool,
                base_vault: vault(mint),
                quote_vault: vault(&NATIVE_MINT),
                mint_metadata: Pubkey::find_program_address(&[b"metadata", MPL_ID.as_ref(), mint.as_ref()], &MPL_ID).0,
                metadata_program: MPL_ID,
                payer: *payer,
                token_quote_program: spl,
                token_program: spl,
                system_program: system_program::ID,
                event_authority: ea(&DBC_ID),
                program: DBC_ID,
            }
            .to_account_metas(None),
            data: dbc::client::args::InitializeVirtualPoolWithSplToken {
                params: dbct::InitializePoolParameters { name: name.into(), symbol: symbol.into(), uri: uri.into() },
            }
            .data(),
        },
        Instruction {
            program_id: DBC_ID,
            accounts: dbc::client::accounts::TransferPoolCreator {
                virtual_pool: pool,
                config: *config,
                creator: *payer,
                new_creator: *fund,
                event_authority: ea(&DBC_ID),
                program: DBC_ID,
            }
            .to_account_metas(None),
            data: dbc::client::args::TransferPoolCreator {}.data(),
        },
    ]
}

// ---- JSON-RPC (curl) --------------------------------------------------------------------------------------------------------------

pub struct Rpc {
    pub url: String,
}

impl Rpc {
    fn call(&self, method: &str, params: Value) -> Result<Value, String> {
        let body = json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params }).to_string();
        for attempt in 0..4u64 {
            let mut c = Command::new("curl")
                .args(["-sS", "-m", "60", "-H", "content-type: application/json", "--data-binary", "@-", &self.url])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .map_err(|e| format!("curl: {e}"))?;
            c.stdin.take().unwrap().write_all(body.as_bytes()).map_err(|e| e.to_string())?;
            let out = c.wait_with_output().map_err(|e| e.to_string())?;
            let text = String::from_utf8_lossy(&out.stdout).to_string();
            match serde_json::from_str::<Value>(&text) {
                Ok(v) if v.get("error").is_some() => {
                    if v["error"]["code"] == json!(429) || text.contains("Too many requests") {
                        sleep(Duration::from_secs(2 + 3 * attempt));
                        continue;
                    }
                    return Err(format!("{method}: {}", v["error"]));
                }
                Ok(v) => return Ok(v["result"].clone()),
                Err(_) => sleep(Duration::from_secs(2 + 3 * attempt)),
            }
        }
        Err(format!("{method}: no answer"))
    }
    fn account(&self, k: &Pubkey) -> Result<Option<(Pubkey, Vec<u8>)>, String> {
        let v = self.call("getAccountInfo", json!([k.to_string(), { "encoding": "base64", "commitment": "confirmed" }]))?;
        if v["value"].is_null() {
            return Ok(None);
        }
        let data = base64::engine::general_purpose::STANDARD.decode(v["value"]["data"][0].as_str().unwrap_or("")).map_err(|e| e.to_string())?;
        Ok(Some((v["value"]["owner"].as_str().unwrap_or("").parse().map_err(|_| "owner")?, data)))
    }
}

fn b64(tx: &VersionedTransaction) -> String {
    base64::engine::general_purpose::STANDARD.encode(bincode_tx(tx))
}
fn bincode_tx(tx: &VersionedTransaction) -> Vec<u8> {
    let mut out = shortvec(tx.signatures.len());
    for s in &tx.signatures {
        out.extend_from_slice(s.as_ref());
    }
    out.extend(tx.message.serialize());
    out
}
fn shortvec(mut n: usize) -> Vec<u8> {
    let mut v = vec![];
    loop {
        let mut b = (n & 0x7f) as u8;
        n >>= 7;
        if n != 0 {
            b |= 0x80;
        }
        v.push(b);
        if n == 0 {
            return v;
        }
    }
}

fn message(ixs: &[Instruction], payer: &Pubkey, blockhash: solana_message::Hash) -> VersionedMessage {
    VersionedMessage::Legacy(Message::new_with_blockhash(ixs, Some(payer), &blockhash))
}

// ---- day 1 ------------------------------------------------------------------------------------------------------------------------

fn sol(l: u64) -> String {
    format!("{}.{:09}", l / 1_000_000_000, l % 1_000_000_000)
}

struct Step {
    name: String,
    ixs: Vec<Instruction>,
    /// the keypairs (besides the payer) that sign: config / mint keypairs Rook's script generated
    extra: Vec<Keypair>,
    /// the config account this step creates, and the check of what DBC writes into it
    config: Pubkey,
    check: Box<dyn Fn(&Pubkey, &[u8]) -> Result<(), String>>,
}

/// Simulate (sigVerify off) with `fee_payer`: (CU, lamports the fee payer spent, rent + fee). The config DBC would write is checked.
fn simulate_step(r: &Rpc, s: &Step, fee_payer: &Pubkey, ixs: &[Instruction]) -> Result<(u64, u64), String> {
    let pre = r.call("getBalance", json!([fee_payer.to_string(), { "commitment": "confirmed" }]))?["value"].as_u64().unwrap_or(0);
    let msg = message(ixs, fee_payer, solana_message::Hash::default());
    let n = msg.header().num_required_signatures as usize;
    let tx = VersionedTransaction { signatures: vec![Default::default(); n], message: msg };
    let sim = r.call("simulateTransaction", json!([b64(&tx), { "encoding": "base64", "sigVerify": false, "replaceRecentBlockhash": true,
        "commitment": "confirmed", "accounts": { "encoding": "base64", "addresses": [s.config.to_string(), fee_payer.to_string()] } }]))?;
    let v = &sim["value"];
    if !v["err"].is_null() {
        return Err(format!("{}: refused in simulation: {} {}", s.name, v["err"], v["logs"]));
    }
    let acc = &v["accounts"][0];
    let data = base64::engine::general_purpose::STANDARD.decode(acc["data"][0].as_str().unwrap_or("")).map_err(|e| e.to_string())?;
    let owner: Pubkey = acc["owner"].as_str().unwrap_or("").parse().map_err(|_| "simulated owner")?;
    (s.check)(&owner, &data)?;
    let post = v["accounts"][1]["lamports"].as_u64().unwrap_or(0);
    Ok((v["unitsConsumed"].as_u64().unwrap_or(0), pre.saturating_sub(post)))
}

fn day1(a: &BTreeMap<String, String>) -> Result<(), String> {
    let need = |k: &str| a.get(k).cloned().ok_or(format!("--{k} is required"));
    let rook_core: Pubkey = need("rook-core")?.parse().map_err(|_| "--rook-core: a base58 pubkey")?;
    let rook_backing: Pubkey = need("rook-backing")?.parse().map_err(|_| "--rook-backing: a base58 pubkey")?;
    let fund: Pubkey = a.get("fund-wallet").map(String::as_str).unwrap_or(FUND_WALLET).parse().map_err(|_| "--fund-wallet: a base58 pubkey")?;
    let launch: Value = serde_json::from_str(&fs::read_to_string(need("launch")?).map_err(|e| e.to_string())?).map_err(|e| format!("launch file: {e}"))?;
    let out = PathBuf::from(need("out")?);
    let cdir = out.join("configs");
    fs::create_dir_all(&cdir).map_err(|e| e.to_string())?;

    let meteora = launch.get("meteora").ok_or("refused: the launch file has no meteora section (node ops/solana/launch-params.mjs)")?;
    let params = config_parameters(meteora.get("config_parameters").ok_or("meteora.config_parameters missing")?)?;
    check_params(&params)?;
    let rook_params = config_parameters(&rook_parameters_json(meteora)?)?;
    check_rook_params(&rook_params)?;
    // audit A6-05: every field of both parameter sets, by byte equality with the committed canonical encoding; and every graduation
    // tier's (docs/v9/graduation-tiers.md)
    let (pin_c, pin_r) = check_pinned_params(meteora, &params, &rook_params)?;
    let tiers = check_pinned_tiers(meteora)?;
    // the final name and symbol are "ROOK" / "ROOK" (operator decision 2026-10-10); the uri has no default
    let md = |k: &str, flag: &str, dflt: &str| {
        a.get(flag)
            .cloned()
            .or(meteora.pointer(&format!("/genesis_rook/metadata/{k}/value")).and_then(|v| v.as_str()).filter(|s| !s.is_empty()).map(String::from))
            .unwrap_or_else(|| dflt.to_string())
    };
    let (rname, rsymbol, ruri) = (md("name", "rook-name", "ROOK"), md("symbol", "rook-symbol", "ROOK"), md("uri", "rook-uri", ROOK_URI));
    if rname.is_empty() || rsymbol.is_empty() || rname.len() > 32 || rsymbol.len() > 10 || ruri.len() > 200 {
        return Err("refused: ROOK metadata: name 1-32 bytes, symbol 1-10, uri ≤ 200 (Metaplex)".into());
    }
    let listings = launch.pointer("/listing_order/listings").and_then(|v| v.as_array()).ok_or("refused: the launch file has no listing_order")?.clone();
    if listings.is_empty() {
        return Err("refused: an empty listing_order".into());
    }
    // audit A6-05 / A6-02: exactly the pinned order, which is nft-pilot.json's (the Ethereum collections, Bitcoin Frogs at 11, then the
    // further Inscriptions listings: docs/v9/ordinals-collections.md)
    check_listing_order(&listings)?;
    let cluster = launch["cluster"].as_str().unwrap_or("").to_string();

    let rpc = a.get("rpc").map(|u| Rpc { url: u.clone() });
    let send = a.contains_key("send");
    let mut reviewed_by = launch.pointer("/review/reviewed_by").cloned().unwrap_or(Value::Null);
    if send {
        // audit A6-05 / A6-06: never sent unreviewed, never with an empty or placeholder ROOK uri (the metadata is immutable)
        reviewed_by = json!(check_reviewed(&launch)?);
        check_rook_metadata_static(&rname, &rsymbol, &ruri)?;
        // the hosted JSON is fetched for mainnet (a localnet / devnet rehearsal has no hosted metadata to fetch)
        if launch["cluster"].as_str() == Some("mainnet-beta") {
            check_rook_metadata_hosted(&rname, &rsymbol, &ruri)?;
        }
    }
    // the ceremony payer: a hot key the script generated (outside the repository), funded by ONE plain transfer from the fund wallet
    let payer_kp = match a.get("payer") {
        Some(p) => Some(read_kp(Path::new(p))?),
        None => None,
    };
    if send && (payer_kp.is_none() || rpc.is_none()) {
        return Err("refused: --send needs --payer <ceremony keypair.json> and --rpc <url>".into());
    }
    if payer_kp.as_ref().map(|k| k.pubkey()) == Some(fund) {
        return Err("refused: the ceremony payer must not be the fund wallet (its key stays with the operator)".into());
    }
    // the ceremony key is the ROOK creator for one instruction, so it is needed even for a dry run (its pubkey; unfunded is fine)
    let payer: Pubkey = payer_kp.as_ref().map(|k| k.pubkey()).ok_or("--payer <ceremony keypair.json> is required (ceremony-day1.sh generates it)")?;

    let genesis = match &rpc {
        Some(r) => {
            let g = r.call("getGenesisHash", json!([]))?.as_str().unwrap_or("").to_string();
            check_cluster(&g, &launch)?;
            if r.account(&DBC_ID)?.is_none() {
                return Err(format!("refused: Meteora DBC ({DBC_ID}) is not on this cluster"));
            }
            Some(g)
        }
        None => None,
    };
    let rent = match &rpc {
        Some(r) => Some(r.call("getMinimumBalanceForRentExemption", json!([POOL_CONFIG_LEN]))?.as_u64().ok_or("rent")?),
        None => None,
    };

    println!("rook_core    (reserved, not deployed) {rook_core}");
    println!("rook_backing (reserved, not deployed) {rook_backing}");
    println!("rook_backing [\"converter\"] PDA        {}   (init_registry's initial converter at deploy)", converter_pda(&rook_backing));
    println!("fund wallet                           {fund}   (ROOK creator + its partner fee claimer; signs nothing here)");
    println!("ceremony payer                        {payer}");
    if let (Some(g), Some(rent)) = (&genesis, rent) {
        println!("cluster {cluster} (genesis {g}); a config account is {} bytes, rent {} SOL", POOL_CONFIG_LEN, sol(rent));
    }

    // one create_config per listing, then the genesis ROOK config + launch (+ creator handover) in one transaction
    let mut steps = vec![];
    let mut rows_meta = vec![];
    for (i, l) in listings.iter().enumerate() {
        let id = l["listing_id"].as_u64().ok_or("listing_id")? as u32;
        if id as usize != i {
            return Err(format!("refused: listing_order is not 0..n in order (entry {i} has id {id})"));
        }
        let slug = l["slug"].as_str().unwrap_or("?").to_string();
        let listing = listing_pda(&rook_core, id);
        let claimer = claimer_pda(&rook_backing, &listing);
        let ckp = config_kp(&cdir.join(format!("{id:02}-{slug}-config.json")))?;
        let cfg = ckp.pubkey();
        // the listing's graduation tier (listing_order graduation_sol, pinned): that tier's pinned parameters
        let threshold = listing_threshold(l)?;
        let tier = threshold / LAMPORTS_PER_SOL_U64;
        let tparams = tiers.get(&tier).ok_or(format!("refused: {slug}: no pinned {tier} SOL tier"))?.0.clone();
        rows_meta.push(json!({ "listing_id": id, "slug": slug, "listing": listing.to_string(), "claimer": claimer.to_string(), "config": cfg.to_string(),
            "graduation_sol": tier }));
        steps.push(Step {
            name: format!("{id:>2} {slug:<16} {tier:>3} SOL  listing {listing}  claimer {claimer}  config {cfg}"),
            ixs: vec![ComputeBudgetInstruction::set_compute_unit_limit(CU_LIMIT), create_config_ix(&payer, &cfg, &claimer, tparams)],
            extra: vec![ckp],
            config: cfg,
            check: Box::new(move |o, d| check_written(o, d, &claimer, threshold)),
        });
    }
    let rkp = config_kp(&cdir.join("genesis-rook-config.json"))?;
    let mkp = config_kp(&cdir.join("genesis-rook-mint.json"))?;
    let (rcfg, rmint) = (rkp.pubkey(), mkp.pubkey());
    let rpool = dbc_pool(&rcfg, &rmint);
    let mut rook_ixs = vec![ComputeBudgetInstruction::set_compute_unit_limit(400_000), create_config_ix(&payer, &rcfg, &fund, rook_params.clone())];
    rook_ixs.extend(rook_launch_ixs(&rcfg, &rmint, &payer, &fund, &rname, &rsymbol, &ruri));
    let combined = message(&rook_ixs, &payer, solana_message::Hash::default());
    let size = shortvec(combined.header().num_required_signatures as usize).len() + 64 * combined.header().num_required_signatures as usize + combined.serialize().len();
    if size > 1232 {
        return Err(format!("the ROOK config + launch transaction is {size} bytes (> 1232): shorten --rook-uri or split it"));
    }
    steps.push(Step {
        name: format!("ROOK genesis    config {rcfg}  mint {rmint}  pool {rpool}  (\"{rname}\" / {rsymbol}; creator handed to the fund wallet in the same tx; {size} bytes)"),
        ixs: rook_ixs,
        extra: vec![rkp, mkp],
        config: rcfg,
        check: Box::new(move |o, d| check_rook_written(o, d, &fund)),
    });

    let sim_payer: Pubkey = a.get("sim-payer").map(String::as_str).unwrap_or(SIM_FEE_PAYER).parse().map_err(|_| "--sim-payer: a base58 pubkey")?;
    let mut rows = vec![];
    let (mut to_send, mut total) = (0u64, 0u64);
    for (k, s) in steps.iter().enumerate() {
        let msg = message(&s.ixs, &payer, solana_message::Hash::default());
        let mut row = if k < rows_meta.len() {
            rows_meta[k].clone()
        } else {
            json!({ "config": rcfg.to_string(), "mint": rmint.to_string(), "pool": rpool.to_string(), "name": rname, "symbol": rsymbol, "uri": ruri,
                "creator_after": fund.to_string(), "fee_claimer": fund.to_string() })
        };
        row["unsigned_message_b64"] = json!(base64::engine::general_purpose::STANDARD.encode(msg.serialize()));
        row["signers"] = json!(std::iter::once(payer.to_string()).chain(s.extra.iter().map(|k| k.pubkey().to_string())).collect::<Vec<_>>());
        let mut line = s.name.clone();
        if let Some(r) = &rpc {
            if let Some((owner, data)) = r.account(&s.config)? {
                (s.check)(&owner, &data)?;
                row["state"] = json!("exists: checked");
                line += "  EXISTS (checked)";
            } else {
                // simulated as the payer will sign it; an unfunded payer is stood in for by the sim payer (rent and fees only)
                let funded = r.call("getBalance", json!([payer.to_string(), { "commitment": "confirmed" }]))?["value"].as_u64().unwrap_or(0) >= 100_000_000;
                let fp = if funded { payer } else { sim_payer };
                let ixs: Vec<Instruction> = s
                    .ixs
                    .iter()
                    .map(|ix| {
                        let mut ix = ix.clone();
                        for m in ix.accounts.iter_mut() {
                            if m.pubkey == payer && m.is_writable && m.is_signer {
                                m.pubkey = fp;
                            }
                        }
                        ix
                    })
                    .collect();
                let (cu, spent) = simulate_step(r, s, &fp, &ixs)?;
                row["state"] = json!("to send");
                row["simulated"] = json!({ "cu": cu, "payer_spent_lamports": spent, "checks": "pass" });
                total += spent;
                to_send += 1;
                line += &format!("  simulated OK ({cu} CU, {} SOL)", sol(spent));
                if send {
                    let pk = payer_kp.as_ref().unwrap();
                    let signers: Vec<&Keypair> = std::iter::once(pk).chain(s.extra.iter()).collect();
                    let sig = send_one(r, &s.ixs, &signers)?;
                    let (owner, data) = r.account(&s.config)?.ok_or("the config does not exist after its transaction")?;
                    (s.check)(&owner, &data)?;
                    row["state"] = json!("sent");
                    row["signature"] = json!(sig);
                    line += &format!("  SENT {sig} (checked on chain)");
                }
            }
        }
        println!("{line}");
        rows.push(row);
    }
    let n = rows_meta.len();
    let plan = json!({
        "_": "Day 1 of the Solana launch without deploying Rook's programs (docs/v9/launch-day1-solana.md). No secret in this file.",
        "generated_at": std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs(),
        "cluster": cluster, "genesis": genesis, "rpc_used": rpc.is_some(),
        "rook_core": rook_core.to_string(), "rook_backing": rook_backing.to_string(),
        "rook_backing_converter": converter_pda(&rook_backing).to_string(),
        "rook_core_registry": Pubkey::find_program_address(&[b"registry"], &rook_core).0.to_string(),
        "rook_core_treasury": Pubkey::find_program_address(&[b"treasury"], &rook_core).0.to_string(),
        "fund_wallet": fund.to_string(), "ceremony_payer": payer.to_string(),
        "config_rent_lamports": rent, "lamports_per_signature": LAMPORTS_PER_SIGNATURE,
        "transactions_to_send": to_send, "total_lamports": total, "total_sol": sol(total),
        "pins": { "file": PINS_PATH, "collection_config_parameters_sha256": pin_c, "genesis_rook_config_parameters_sha256": pin_r,
            "tier_config_parameters_sha256": tiers.iter().map(|(t, (_, h))| (t.to_string(), json!(h))).collect::<serde_json::Map<_, _>>() },
        "reviewed_by": reviewed_by,
        "listings": rows[..n], "genesis_rook": rows[n],
    });
    fs::write(out.join("day1-plan.json"), serde_json::to_string_pretty(&plan).unwrap()).map_err(|e| e.to_string())?;
    if rpc.is_some() {
        println!(
            "day 1 cost (simulated on the cluster, rent + fees): {to_send} transaction(s) = {} SOL{}",
            sol(total),
            if send { " (sent)" } else { " (nothing sent)" }
        );
    }
    println!("plan: {}", out.join("day1-plan.json").display());
    Ok(())
}

/// One row of day1-plan.json, recomputed and checked (`preflight`).
#[derive(Clone)]
pub struct Day1Row {
    pub id: u32,
    pub slug: String,
    pub listing: Pubkey,
    pub claimer: Pubkey,
    pub config: Pubkey,
    /// the listing's graduation tier (lamports): its config's migration_quote_threshold
    pub threshold: u64,
}

fn pk_of(v: &Value, what: &str) -> Result<Pubkey, String> {
    v.as_str().ok_or(format!("day1-plan.json: {what} missing"))?.parse().map_err(|_| format!("day1-plan.json: {what} not base58"))
}

/// Reads an account through the RPC and, when a second RPC is given, requires the same owner and bytes from it (audit A6-09).
pub fn account2(r: &Rpc, r2: Option<&Rpc>, k: &Pubkey) -> Result<Option<(Pubkey, Vec<u8>)>, String> {
    let a = r.account(k)?;
    if let Some(r2) = r2 {
        let b = r2.account(k)?;
        if a != b {
            return Err(format!("refused: the two RPCs disagree about {k}"));
        }
    }
    Ok(a)
}

/// Audit A6-04 (and A6-05 at deploy time), before anything is deployed or initialized:
/// - the RPC's genesis is the launch file's cluster (mainnet-beta ⇒ Bitcoin Mainnet), the launch file is the pinned, reviewed one;
/// - the keys directory is the day-1 one: day1-plan.json names these program ids, this fund wallet (and this ceremony payer);
/// - every listing / claimer PDA is recomputed from the ids and equals the plan's, in the pinned order, and every day-1 config ON
///   CHAIN pays that claimer (fee_claimer = leftover_receiver, the whole account checked); the ROOK config pays the fund wallet.
#[allow(clippy::too_many_arguments)]
pub fn preflight(
    r: &Rpc,
    r2: Option<&Rpc>,
    launch: &Value,
    plan: &Value,
    core: &Pubkey,
    backing: &Pubkey,
    fund: &Pubkey,
    payer: Option<&Pubkey>,
) -> Result<Vec<Day1Row>, String> {
    let g = r.call("getGenesisHash", json!([]))?.as_str().unwrap_or("").to_string();
    if let Some(r2) = r2 {
        if r2.call("getGenesisHash", json!([]))?.as_str() != Some(&g) {
            return Err("refused: the two RPCs serve different clusters".into());
        }
    }
    check_cluster(&g, launch)?;
    let meteora = launch.get("meteora").ok_or("refused: the launch file has no meteora section")?;
    let params = config_parameters(meteora.get("config_parameters").ok_or("meteora.config_parameters missing")?)?;
    let rook = config_parameters(&rook_parameters_json(meteora)?)?;
    check_pinned_params(meteora, &params, &rook)?;
    check_pinned_tiers(meteora)?;
    let order = launch.pointer("/listing_order/listings").and_then(|v| v.as_array()).ok_or("refused: the launch file has no listing_order")?;
    check_listing_order(order)?;
    check_reviewed(launch)?;
    // the keys directory is the day-1 one
    for (k, want) in [("rook_core", core), ("rook_backing", backing), ("fund_wallet", fund)] {
        if pk_of(&plan[k], k)? != *want {
            return Err(format!("refused: day1-plan.json's {k} is {}, not {want}: this keys directory / launch is not day 1's", plan[k]));
        }
    }
    if let Some(p) = payer {
        if pk_of(&plan["ceremony_payer"], "ceremony_payer")? != *p {
            return Err("refused: the ceremony payer is not day 1's (day1-plan.json ceremony_payer)".into());
        }
    }
    if plan["cluster"].as_str() != launch["cluster"].as_str() {
        return Err(format!("refused: day1-plan.json was made for {} but the launch file names {}", plan["cluster"], launch["cluster"]));
    }
    if let Some(pg) = plan["genesis"].as_str() {
        if pg != g {
            return Err(format!("refused: day 1 ran on genesis {pg}; this RPC is {g}"));
        }
    }
    let rows = plan["listings"].as_array().ok_or("day1-plan.json: listings missing")?;
    if rows.len() != order.len() {
        return Err(format!("refused: day1-plan.json has {} listings, the launch file {}", rows.len(), order.len()));
    }
    let mut out = vec![];
    for (i, (row, l)) in rows.iter().zip(order).enumerate() {
        let id = i as u32;
        let slug = l["slug"].as_str().unwrap_or("").to_string();
        if row["slug"].as_str() != Some(&slug) || row["listing_id"].as_u64() != Some(i as u64) {
            return Err(format!("refused: day1-plan.json listing {i} is {} / {}, the launch file's is {slug}", row["listing_id"], row["slug"]));
        }
        let listing = listing_pda(core, id);
        let claimer = claimer_pda(backing, &listing);
        if pk_of(&row["listing"], "listing")? != listing || pk_of(&row["claimer"], "claimer")? != claimer {
            return Err(format!("refused: {slug}: the listing / claimer recomputed from the keys ({listing} / {claimer}) are not day1-plan.json's"));
        }
        let config = pk_of(&row["config"], "config")?;
        let (owner, data) = account2(r, r2, &config)?.ok_or(format!("refused: {slug}: the day-1 config {config} does not exist on this cluster"))?;
        let threshold = listing_threshold(l)?;
        if row["graduation_sol"].as_u64().is_some_and(|t| t * 1_000_000_000 != threshold) {
            return Err(format!("refused: {slug}: day1-plan.json's graduation_sol {} is not the launch file's", row["graduation_sol"]));
        }
        check_written(&owner, &data, &claimer, threshold).map_err(|e| format!("refused: {slug}: the on-chain config {config}: {e}"))?;
        out.push(Day1Row { id, slug, listing, claimer, config, threshold });
    }
    let rc = pk_of(&plan["genesis_rook"]["config"], "genesis_rook.config")?;
    let (owner, data) = account2(r, r2, &rc)?.ok_or(format!("refused: the genesis ROOK config {rc} does not exist on this cluster"))?;
    check_rook_written(&owner, &data, fund).map_err(|e| format!("refused: the on-chain ROOK config {rc}: {e}"))?;
    println!("preflight: cluster {} (genesis {g}), Bitcoin {}; {} day-1 configs on chain pay the claimers recomputed from {core} / {backing}; ROOK config pays {fund}",
        launch["cluster"].as_str().unwrap_or(""), launch["bitcoin_network"].as_str().unwrap_or(""), out.len());
    Ok(out)
}

fn read_json(p: &str) -> Result<Value, String> {
    serde_json::from_str(&fs::read_to_string(p).map_err(|e| format!("{p}: {e}"))?).map_err(|e| format!("{p}: {e}"))
}

/// `rook-ceremony preflight --rook-core <id> --rook-backing <id> --launch <launch.json> --plan <day1-plan.json> --rpc <url>
///   [--rpc2 <url>] [--fund-wallet <pubkey>] [--payer <ceremony keypair.json>]` (deploy-later.sh runs it before building).
fn preflight_cmd(a: &BTreeMap<String, String>) -> Result<(), String> {
    let need = |k: &str| a.get(k).cloned().ok_or(format!("--{k} is required"));
    let core: Pubkey = need("rook-core")?.parse().map_err(|_| "--rook-core")?;
    let backing: Pubkey = need("rook-backing")?.parse().map_err(|_| "--rook-backing")?;
    let fund: Pubkey = a.get("fund-wallet").map(String::as_str).unwrap_or(FUND_WALLET).parse().map_err(|_| "--fund-wallet")?;
    let payer = a.get("payer").map(|p| read_kp(Path::new(p)).map(|k| k.pubkey())).transpose()?;
    let r = Rpc { url: need("rpc")? };
    let r2 = a.get("rpc2").map(|u| Rpc { url: u.clone() });
    preflight(&r, r2.as_ref(), &read_json(&need("launch")?)?, &read_json(&need("plan")?)?, &core, &backing, &fund, payer.as_ref())?;
    Ok(())
}

fn bitcoin_network(launch: &Value) -> Result<rook_core::state::BitcoinNetwork, String> {
    match launch.pointer("/init/init_registry/bitcoin_network").or(launch.get("bitcoin_network")).and_then(|v| v.as_str()) {
        Some("Mainnet") => Ok(rook_core::state::BitcoinNetwork::Mainnet),
        Some("Signet") => Ok(rook_core::state::BitcoinNetwork::Signet),
        Some("Testnet4") => Ok(rook_core::state::BitcoinNetwork::Testnet4),
        n => Err(format!("the launch file's bitcoin_network {n:?}: Mainnet | Signet | Testnet4")),
    }
}

/// The registry as init_registry must have written it: the launch file's network, rook_backing's converter, and an owner that is
/// either the ceremony (setup) key, during the setup, or the fund wallet, after `accept_owner` (`owners`; anything else is refused).
pub fn check_registry(data: &[u8], owners: &[Pubkey], network: rook_core::state::BitcoinNetwork, converter: &Pubkey) -> Result<rook_core::state::Registry, String> {
    use anchor_lang::AccountDeserialize;
    let reg = rook_core::state::Registry::try_deserialize(&mut &data[..]).map_err(|e| format!("not a rook_core Registry: {e}"))?;
    if !owners.contains(&reg.owner) {
        let want = owners.iter().map(|o| o.to_string()).collect::<Vec<_>>().join(" or ");
        return Err(format!("the registry's owner is {}, not {want} (the ceremony key during the setup, then the fund wallet)", reg.owner));
    }
    if reg.pending_owner != Pubkey::default() && reg.pending_owner != owners[owners.len() - 1] {
        return Err(format!("the registry's pending owner is {}, not the fund wallet {}", reg.pending_owner, owners[owners.len() - 1]));
    }
    if reg.bitcoin_network != network {
        return Err(format!("the registry serves Bitcoin {:?}, the launch file names {network:?} (immutable)", reg.bitcoin_network));
    }
    if !reg.is_converter(converter) {
        return Err(format!("rook_backing's converter {converter} is not an allowed converter"));
    }
    Ok(reg)
}

/// Deploy day (ops/solana/deploy-later.sh), after both programs are deployed with the ceremony key as their upgrade authority and
/// BEFORE the authority moves to the fund wallet: `init_registry(owner = the ceremony key, the launch file's Bitcoin network,
/// [rook_backing's "converter" PDA])` and `init_backing`, both signed by the ceremony key (the upgrade authority, which the programs
/// check). The ceremony key stays the registry owner for the setup only (the listing ceremony signs with it, so the fund wallet, the
/// operator's Phantom / Ledger wallet, never exists as a key file); `propose-owner` then names the fund wallet and the fund wallet
/// completes `accept_owner` with one signature (`accept-owner`, `verify-owner`: ceremony_owner.rs). Preflight first (A6-04: cluster, keys = day 1's, the day-1 on-chain configs (one per listing) pay the recomputed claimers); simulated; sent only
/// with --send; existing accounts are decoded and checked (owner, network, converter / pinned PDAs), and read back after sending.
///   rook-ceremony init --rook-core <id> --rook-backing <id> --payer <ceremony keypair.json> --launch <launch.json>
///                      --plan <day1-plan.json> --rpc <url> [--rpc2 <url>] [--fund-wallet <pubkey>] [--send]
fn init(a: &BTreeMap<String, String>) -> Result<(), String> {
    use anchor_lang::{solana_program::bpf_loader_upgradeable, AccountDeserialize};
    let need = |k: &str| a.get(k).cloned().ok_or(format!("--{k} is required"));
    let core: Pubkey = need("rook-core")?.parse().map_err(|_| "--rook-core")?;
    let backing: Pubkey = need("rook-backing")?.parse().map_err(|_| "--rook-backing")?;
    let fund: Pubkey = a.get("fund-wallet").map(String::as_str).unwrap_or(FUND_WALLET).parse().map_err(|_| "--fund-wallet")?;
    let pk = read_kp(Path::new(&need("payer")?))?;
    let r = Rpc { url: need("rpc")? };
    let r2 = a.get("rpc2").map(|u| Rpc { url: u.clone() });
    let launch = read_json(&need("launch").map_err(|_| "--launch <launch.json> is required (audit A6-04: the network comes from the reviewed launch file)")?)?;
    let plan = read_json(&need("plan").map_err(|_| "--plan <keys>/day1/day1-plan.json is required (audit A6-04)")?)?;
    let network = bitcoin_network(&launch)?;
    if let Some(n) = a.get("bitcoin-network") {
        if format!("{network:?}") != *n {
            return Err(format!("refused: --bitcoin-network {n} but the launch file names {network:?}"));
        }
    }
    preflight(&r, r2.as_ref(), &launch, &plan, &core, &backing, &fund, Some(&pk.pubkey()))?;
    let converter = converter_pda(&backing);
    let pd = |p: &Pubkey| Pubkey::find_program_address(&[p.as_ref()], &bpf_loader_upgradeable::ID).0;
    let registry = Pubkey::find_program_address(&[b"registry"], &core).0;
    let treasury = Pubkey::find_program_address(&[b"treasury"], &core).0;
    let settings = Pubkey::find_program_address(&[rook_backing::constants::SETTINGS_SEED], &backing).0;
    let check_settings = |d: &[u8]| -> Result<(), String> {
        let s = rook_backing::state::Settings::try_deserialize(&mut &d[..]).map_err(|e| format!("not rook_backing Settings: {e}"))?;
        if s.registry != registry || s.treasury != treasury || s.converter != converter {
            return Err(format!("rook_backing's settings pin registry {} / treasury {} / converter {}, not {registry} / {treasury} / {converter}: \
                rook_backing was built with another ROOK_CORE_ID", s.registry, s.treasury, s.converter));
        }
        Ok(())
    };
    let check = |name: &str, acct: &Pubkey| -> Result<bool, String> {
        match account2(&r, r2.as_ref(), acct)? {
            None => Ok(false),
            Some((owner, d)) => {
                if name == "init_registry" {
                    if owner != core {
                        return Err(format!("refused: {acct} is not owned by rook_core"));
                    }
                    check_registry(&d, &[pk.pubkey(), fund], network, &converter).map_err(|e| format!("refused: the registry {acct}: {e}"))?;
                } else {
                    if owner != backing {
                        return Err(format!("refused: {acct} is not owned by rook_backing"));
                    }
                    check_settings(&d).map_err(|e| format!("refused: {e}"))?;
                }
                Ok(true)
            }
        }
    };
    let pending: Vec<bool> = [("init_registry", &registry), ("init_backing", &settings)].iter().map(|(n, k)| check(n, k).map(|e| !e)).collect::<Result<_, _>>()?;
    if pending.iter().any(|p| *p) {
        for p in [&core, &backing] {
            let (_, d) = r.account(&pd(p))?.ok_or(format!("{p} is not deployed (no ProgramData)"))?;
            if rook_core::upgrade_authority(&d) != Some(pk.pubkey()) {
                return Err(format!("refused: the upgrade authority of {p} is not the ceremony key (init must run before the handover)"));
            }
        }
    }
    let reg_ix = Instruction {
        program_id: core,
        accounts: rook_core::accounts::InitRegistry { authority: pk.pubkey(), payer: pk.pubkey(), registry, program_data: pd(&core), system_program: system_program::ID }
            .to_account_metas(None),
        data: rook_core::instruction::InitRegistry { owner: pk.pubkey(), bitcoin_network: network, initial_converters: vec![converter] }.data(),
    };
    let back_ix = Instruction {
        program_id: backing,
        accounts: rook_backing::accounts::InitBacking { authority: pk.pubkey(), settings, program: backing, program_data: pd(&backing), system_program: system_program::ID }
            .to_account_metas(None),
        data: rook_backing::instruction::InitBacking {}.data(),
    };
    for ((name, ix, acct), todo) in [("init_registry", reg_ix, registry), ("init_backing", back_ix, settings)].into_iter().zip(pending) {
        if !todo {
            println!("{name}: already done ({acct}; decoded and checked)");
            continue;
        }
        let msg = message(&[ix.clone()], &pk.pubkey(), solana_message::Hash::default());
        let tx = VersionedTransaction { signatures: vec![Default::default(); msg.header().num_required_signatures as usize], message: msg };
        let sim = r.call("simulateTransaction", json!([b64(&tx), { "encoding": "base64", "sigVerify": false, "replaceRecentBlockhash": true, "commitment": "confirmed" }]))?;
        if !sim["value"]["err"].is_null() {
            return Err(format!("{name} refused in simulation: {} {}", sim["value"]["err"], sim["value"]["logs"]));
        }
        print!("{name}: simulated OK ({} CU)", sim["value"]["unitsConsumed"]);
        if a.contains_key("send") {
            let sig = send_one(&r, &[ix], &[&pk])?;
            print!("  SENT {sig}");
            if !check(name, &acct)? {
                return Err(format!("{name}: {acct} does not exist after its transaction"));
            }
            print!("  (read back and checked)");
        }
        println!();
    }
    let Some((_, d)) = account2(&r, r2.as_ref(), &registry)? else {
        println!("(nothing sent: --send; init_registry names the ceremony key {} as the registry owner for the setup)", pk.pubkey());
        return Ok(());
    };
    let reg = rook_core::state::Registry::try_deserialize(&mut &d[..]).map_err(|e| e.to_string())?;
    if reg.owner == fund {
        println!("registry owner = {fund} (the fund wallet: handed over); Bitcoin {network:?}; initial converter = {converter}");
    } else {
        println!("registry owner = {} (the ceremony key, for the setup only: list, then propose-owner, then the fund wallet's accept-owner); \
            Bitcoin {network:?}; initial converter = {converter}", reg.owner);
    }
    Ok(())
}

fn send_one(r: &Rpc, ixs: &[Instruction], signers: &[&Keypair]) -> Result<String, String> {
    let payer = signers[0];
    let v = r.call("getLatestBlockhash", json!([{ "commitment": "finalized" }]))?;
    let bh: solana_message::Hash = v["value"]["blockhash"].as_str().unwrap_or("").parse().map_err(|_| "blockhash")?;
    let last = v["value"]["lastValidBlockHeight"].as_u64().unwrap_or(0);
    let tx = VersionedTransaction::try_new(message(ixs, &payer.pubkey(), bh), signers).map_err(|e| e.to_string())?;
    let sig = tx.signatures[0].to_string();
    let wire = b64(&tx);
    let t0 = Instant::now();
    loop {
        // same bytes until the blockhash expires: at most one lands
        let _ = r.call("sendTransaction", json!([wire, { "encoding": "base64", "skipPreflight": false, "preflightCommitment": "confirmed" }]));
        sleep(Duration::from_secs(3));
        let s = r.call("getSignatureStatuses", json!([[sig], { "searchTransactionHistory": true }]))?;
        let st = &s["value"][0];
        if !st.is_null() {
            if !st["err"].is_null() {
                return Err(format!("{sig} failed: {}", st["err"]));
            }
            if st["confirmationStatus"] == json!("finalized") {
                return Ok(sig);
            }
        }
        let h = r.call("getBlockHeight", json!([{ "commitment": "finalized" }]))?.as_u64().unwrap_or(0);
        if h > last && st.is_null() {
            return Err(format!("{sig} expired unlanded after {:?}: rerun (the config keypair is reused)", t0.elapsed()));
        }
    }
}

fn main() {
    let r = args().and_then(|(cmd, a)| match cmd.as_str() {
        "day1" => day1(&a),
        "init" => init(&a),
        "preflight" => preflight_cmd(&a),
        "list" => ceremony_list::list(&a),
        "propose-owner" => ceremony_owner::propose_owner(&a),
        "accept-owner" => ceremony_owner::accept_owner(&a),
        "verify-owner" => ceremony_owner::verify_owner(&a),
        "pins" => cmd_pins(&a),
        other => Err(format!("unknown command {other}")),
    });
    if let Err(e) = r {
        eprintln!("FAIL {e}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The launch generator's meteora section (ops/solana/launch-params.mjs) parses into exactly Q1's tested config
    /// (solana/tests/harness/mod.rs `rook_config_params(50)`, copied here value for value).
    #[test]
    fn the_launch_files_meteora_section_is_q1s_config() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../..");
        let out = Command::new("node").arg(root.join("ops/solana/launch-params.mjs")).arg("--meteora").output();
        let Ok(out) = out else {
            eprintln!("node not found: skipped");
            return;
        };
        let v: Value = serde_json::from_slice(&out.stdout).expect("launch-params --meteora prints JSON");
        let p = config_parameters(&v["meteora"]["config_parameters"]).unwrap();
        check_params(&p).unwrap();
        let p0 = p.clone();
        assert_eq!(p.sqrt_start_price, 134452599321738966);
        assert_eq!(
            p.curve.iter().map(|c| (c.sqrt_price, c.liquidity)).collect::<Vec<_>>(),
            vec![(268905193275807567, 215124158914782432327345953095692), (79226673521066979257578248091, 3435308751277378593870180)]
        );
        assert_eq!((p.migration_fee_option, p.token_update_authority, p.pool_creation_fee), (6, 1, 0));
        assert_eq!((p.migrated_pool_fee.collect_fee_mode, p.migrated_pool_fee.dynamic_fee, p.migrated_pool_fee.pool_fee_bps), (0, 0, 100));
        assert_eq!((p.migrated_pool_base_fee_mode, p.compounding_fee_bps), (0, 0));
        assert_eq!(p.locked_vesting.amount_per_period + p.locked_vesting.cliff_unlock_amount + p.locked_vesting.number_of_period, 0);
        // the same bytes as the instruction data Q1 sent
        assert_eq!(
            dbc::client::args::CreateConfig { config_parameters: p.clone() }.data().len(),
            dbc::client::args::CreateConfig { config_parameters: p }.data().len()
        );
        let l = v["listing_order"].as_array().unwrap();
        // docs/v9/ordinals-collections.md: 11 Ethereum, Bitcoin Frogs at 11, further Ordinals collections at 12-25
        assert_eq!(l.len(), 26);
        assert_eq!((l[0]["slug"].as_str(), l[11]["slug"].as_str(), l[11]["listing_id"].as_u64()), (Some("milady"), Some("bitcoin-frogs"), Some(11)));
        assert_eq!((l[12]["slug"].as_str(), l[16]["slug"].as_str(), l[16]["listing_id"].as_u64()), (Some("nodemonkes"), Some("natcats"), Some(16)));
        assert_eq!((l[17]["slug"].as_str(), l[25]["slug"].as_str(), l[25]["listing_id"].as_u64()), (Some("runestone"), Some("bitcoin-wizards"), Some(25)));
        assert!(l[11..].iter().all(|e| e["kind"] == "Inscriptions" && e["members_root"].as_str().map(|r| r.len() == 66).unwrap_or(false)));
        assert!(l.len() <= rook_core::state::MAX_COLLECTIONS, "every listing's module must fit the treasury");
        // docs/v9/graduation-tiers.md: every listing names a tier, and every tier's config passes D-e with its own threshold
        assert!(l.iter().all(|e| listing_threshold(e).is_ok()));
        for t in GRADUATION_TIERS_SOL {
            let tp = config_parameters(&tier_parameters_json(&v["meteora"], t).unwrap()).unwrap();
            check_params(&tp).unwrap();
            assert_eq!(tp.migration_quote_threshold, t * 1_000_000_000);
            if t == 85 {
                assert_eq!(ceremony_checks::encode(&tp), ceremony_checks::encode(&p0), "the 85 SOL tier is the collection config");
            } else {
                assert_ne!(tp.sqrt_start_price, p0.sqrt_start_price, "each tier has its own curve");
            }
        }
    }

    /// A config whose threshold is a tier but whose curve is another tier's is refused by the tier pins.
    #[test]
    fn a_tier_with_another_tiers_curve_is_refused() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../..");
        let Ok(out) = Command::new("node").arg(root.join("ops/solana/launch-params.mjs")).arg("--meteora").output() else { return };
        let mut v: Value = serde_json::from_slice(&out.stdout).unwrap();
        check_pinned_tiers(&v["meteora"]).unwrap();
        v["meteora"]["tiers"]["170"]["curve"] = v["meteora"]["tiers"]["85"]["curve"].clone();
        assert!(check_pinned_tiers(&v["meteora"]).unwrap_err().contains("170 SOL tier"));
        let mut v: Value = serde_json::from_slice(&out.stdout).unwrap();
        v["meteora"]["tiers"]["340"] = v["meteora"]["tiers"]["170"].clone();
        assert!(check_pinned_tiers(&v["meteora"]).unwrap_err().contains("pinned set"));
        let mut v: Value = serde_json::from_slice(&out.stdout).unwrap();
        v["meteora"]["tiers"]["255"]["migration_quote_threshold"]["value"] = json!("300000000000");
        assert!(check_pinned_tiers(&v["meteora"]).is_err());
    }

    /// The run-time derivation equals the programs' own (with the dev ids compiled into rook_core / rook_backing).
    #[test]
    fn the_claimer_is_rook_backings_pda_of_rook_cores_listing() {
        for id in [0u32, 1, 11, 12, 16, 17, 25] {
            let listing = listing_pda(&rook_core::ID, id);
            assert_eq!(listing, Pubkey::find_program_address(&[rook_core::seeds::LISTING, &id.to_le_bytes()], &rook_core::ID).0);
            assert_eq!(
                claimer_pda(&rook_backing::ID, &listing),
                Pubkey::find_program_address(&[rook_backing::constants::CLAIMER_SEED, listing.as_ref()], &rook_backing::ID).0
            );
        }
        assert_eq!(converter_pda(&rook_backing::ID), Pubkey::find_program_address(&[b"converter"], &rook_backing::ID).0);
    }

    /// The genesis ROOK config: the collection config with D-g's overrides (fee 0, creator share 100, the fee-0 curve), and its launch
    /// transaction (config + pool + creator handover) fits one legacy transaction.
    #[test]
    fn the_genesis_rook_config_is_d_g_and_fits_one_transaction() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../..");
        let Ok(out) = Command::new("node").arg(root.join("ops/solana/launch-params.mjs")).arg("--meteora").output() else { return };
        let v: Value = serde_json::from_slice(&out.stdout).unwrap();
        let p = config_parameters(&rook_parameters_json(&v["meteora"]).unwrap()).unwrap();
        check_rook_params(&p).unwrap();
        assert_eq!((p.migration_fee.fee_percentage, p.creator_trading_fee_percentage, p.sqrt_start_price), (0, 100, 95072344172433750));
        assert!(check_params(&p).is_err(), "a ROOK config is never a collection config");
        let (payer, fund, cfg, mint) = (Pubkey::new_unique(), FUND_WALLET.parse::<Pubkey>().unwrap(), Pubkey::new_unique(), Pubkey::new_unique());
        let mut ixs = vec![ComputeBudgetInstruction::set_compute_unit_limit(400_000), create_config_ix(&payer, &cfg, &fund, p)];
        ixs.extend(rook_launch_ixs(&cfg, &mint, &payer, &fund, "Rook", "ROOK", &"x".repeat(100)));
        let m = message(&ixs, &payer, Default::default());
        assert_eq!(m.header().num_required_signatures, 3); // payer (= creator), config, mint
        assert!(1 + 3 * 64 + m.serialize().len() <= 1232, "{} bytes", 1 + 3 * 64 + m.serialize().len());
        // the fund wallet never signs
        assert!(m.static_account_keys().iter().position(|k| *k == fund).unwrap() >= 3);
    }

    /// A launch file that drifts from D-e is refused before anything is built.
    #[test]
    fn a_meteora_section_off_d_e_is_refused() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../..");
        let Ok(out) = Command::new("node").arg(root.join("ops/solana/launch-params.mjs")).arg("--meteora").output() else { return };
        let mut v: Value = serde_json::from_slice(&out.stdout).unwrap();
        v["meteora"]["config_parameters"]["migration_fee"]["fee_percentage"]["value"] = json!(30);
        let p = config_parameters(&v["meteora"]["config_parameters"]).unwrap();
        assert!(check_params(&p).unwrap_err().contains("50 % migration fee"));
        v["meteora"]["config_parameters"]["migration_fee"]["fee_percentage"]["value"] = json!(50);
        v["meteora"]["config_parameters"]["creator_trading_fee_percentage"]["value"] = json!(10);
        assert!(check_params(&config_parameters(&v["meteora"]["config_parameters"]).unwrap()).is_err());
    }
}
