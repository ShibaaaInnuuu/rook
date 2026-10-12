//! W2: the canister's own SPL / ATA / Memo / Metaplex / Meteora DBC 0.2.1 / DAMM v2 0.2.4 builders and decoders (`src/solix2`),
//! byte for byte against `tests/fixtures/solix2/flow.json`: instructions that LANDED in LiteSVM 0.10 on the mainnet program bytes,
//! built by spl-token / spl-associated-token-account and by the Anchor clients `declare_program!` generates from the official IDLs
//! (written by `solana/tests/solix2_fixtures.rs`, the spike S1/S2 flow with deterministic keys), and the accounts those programs
//! wrote. Also: the guards (what DBC does not check), and the transaction sizes the spikes measured.
use base64::Engine;
use etch_autobuyer::solix2::{self, damm, dbc, decode, guard, mpl, route, spl};
use etch_autobuyer::soltx::{self, Instruction, Pubkey};
use serde_json::Value;

fn flow() -> Value {
    let p = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/solix2/flow.json");
    serde_json::from_str(&std::fs::read_to_string(p).expect("flow.json")).expect("json")
}
fn pk(v: &Value) -> Pubkey {
    soltx::key(v.as_str().unwrap_or_else(|| panic!("a key, got {v}")))
}
fn n(v: &Value) -> u128 {
    match v {
        Value::Number(x) => x.as_u64().expect("u64") as u128,
        Value::String(s) => s.parse().expect("integer string"),
        _ => panic!("a number, got {v}"),
    }
}
fn n64(v: &Value) -> u64 {
    u64::try_from(n(v)).expect("u64")
}
fn st<'a>(v: &'a Value) -> &'a str {
    v.as_str().expect("string")
}
fn fixture_ix(j: &Value) -> Instruction {
    Instruction {
        program: pk(&j["program"]),
        accounts: j["accounts"].as_array().unwrap().iter().map(|a| soltx::meta(pk(&a[0]), a[1].as_bool().unwrap(), a[2].as_bool().unwrap())).collect(),
        data: hex::decode(st(&j["data"])).unwrap(),
    }
}
fn curve_of(v: &Value) -> Vec<(u128, u128)> {
    v.as_array().unwrap().iter().map(|p| (n(&p[0]), n(&p[1]))).collect()
}
fn cfg_params(p: &Value) -> dbc::ConfigParameters {
    dbc::rook_wx_config(n64(&p["t_wx"]), n(&p["sqrt_start_price"]), curve_of(&p["curve"]))
}
fn pool_of(p: &Value) -> damm::Pool {
    damm::Pool::at(pk(&p["pool"]), pk(&p["token_a_mint"]), pk(&p["token_b_mint"]))
}

/// The canister's builder for one recorded instruction, from its recorded parameters.
fn build(name: &str, p: &Value) -> Instruction {
    let curve = || dbc::Curve::new(pk(&p["config"]), pk(&p["base_mint"]), pk(&p["quote_mint"]));
    match name {
        "system.create_account" => spl::create_account(pk(&p["from"]), pk(&p["new"]), n64(&p["lamports"]), n64(&p["space"]), pk(&p["owner"])),
        "system.transfer" => soltx::transfer(pk(&p["from"]), pk(&p["to"]), n64(&p["lamports"])),
        "spl.initialize_mint2" => spl::initialize_mint2(pk(&p["mint"]), pk(&p["authority"]), (!p["freeze"].is_null()).then(|| pk(&p["freeze"])), n64(&p["decimals"]) as u8),
        "spl.set_authority" => spl::set_authority(pk(&p["account"]), spl::AuthorityType::MintTokens, Some(pk(&p["new"])), pk(&p["current"])),
        "spl.mint_to" => spl::mint_to(pk(&p["mint"]), pk(&p["dest"]), pk(&p["authority"]), n64(&p["amount"])),
        "spl.mint_to_checked" => spl::mint_to_checked(pk(&p["mint"]), pk(&p["dest"]), pk(&p["authority"]), n64(&p["amount"]), n64(&p["decimals"]) as u8),
        "spl.burn" => spl::burn(pk(&p["account"]), pk(&p["mint"]), pk(&p["owner"]), n64(&p["amount"])),
        "spl.burn_checked" => spl::burn_checked(pk(&p["account"]), pk(&p["mint"]), pk(&p["owner"]), n64(&p["amount"]), n64(&p["decimals"]) as u8),
        "spl.transfer" => spl::transfer(pk(&p["source"]), pk(&p["dest"]), pk(&p["owner"]), n64(&p["amount"])),
        "spl.transfer_checked" => spl::transfer_checked(pk(&p["source"]), pk(&p["mint"]), pk(&p["dest"]), pk(&p["owner"]), n64(&p["amount"]), n64(&p["decimals"]) as u8),
        "spl.close_account" => spl::close_account(pk(&p["account"]), pk(&p["dest"]), pk(&p["owner"])),
        "spl.sync_native" => spl::sync_native(pk(&p["account"])),
        "ata.create" => spl::create_ata(pk(&p["payer"]), pk(&p["owner"]), pk(&p["mint"])),
        "ata.create_idempotent" => spl::create_ata_idempotent(pk(&p["payer"]), pk(&p["owner"]), pk(&p["mint"])),
        "memo" => spl::memo(st(&p["text"]), &p["signers"].as_array().unwrap().iter().map(pk).collect::<Vec<_>>()),
        "mpl.create_metadata_v3" => mpl::create_metadata_v3(pk(&p["mint"]), pk(&p["mint_authority"]), pk(&p["payer"]), pk(&p["update_authority"]), st(&p["name"]),
            st(&p["symbol"]), st(&p["uri"])),
        "mpl.update_metadata_v2" => mpl::update_metadata_v2(pk(&p["mint"]), pk(&p["update_authority"]), st(&p["name"]), st(&p["symbol"]), st(&p["uri"])),
        "dbc.create_config" => dbc::create_config(pk(&p["config"]), pk(&p["fee_claimer"]), pk(&p["leftover_receiver"]), pk(&p["quote_mint"]), pk(&p["payer"]), &cfg_params(p)),
        "dbc.initialize_virtual_pool_with_spl_token" => dbc::initialize_virtual_pool_with_spl_token(&curve(), pk(&p["creator"]), pk(&p["payer"]), st(&p["name"]),
            st(&p["symbol"]), st(&p["uri"])),
        "dbc.swap" => dbc::swap(&curve(), pk(&p["payer"]), pk(&p["input"]), pk(&p["output"]), n64(&p["amount_in"]), n64(&p["minimum_amount_out"])),
        "dbc.swap2" => dbc::swap2(&curve(), pk(&p["payer"]), pk(&p["input"]), pk(&p["output"]), n64(&p["amount_0"]), n64(&p["amount_1"]), n64(&p["swap_mode"]) as u8),
        "dbc.claim_trading_fee" => dbc::claim_trading_fee(&curve(), pk(&p["fee_claimer"]), pk(&p["token_a_account"]), pk(&p["token_b_account"]), n64(&p["max_amount_a"]),
            n64(&p["max_amount_b"])),
        "dbc.migration_damm_v2" => dbc::migration_damm_v2(&curve(), pk(&p["payer"]), pk(&p["first_nft"]), pk(&p["second_nft"])),
        "dbc.withdraw_migration_fee" => {
            let c = dbc::Curve::new(pk(&p["config"]), base_of(p), pk(&p["quote_mint"]));
            dbc::withdraw_migration_fee(&c, pk(&p["sender"]), pk(&p["token_quote_account"]), n64(&p["flag"]) as u8)
        }
        "dbc.partner_withdraw_surplus" => {
            let c = dbc::Curve::new(pk(&p["config"]), base_of(p), pk(&p["quote_mint"]));
            dbc::partner_withdraw_surplus(&c, pk(&p["fee_claimer"]), pk(&p["token_quote_account"]))
        }
        "dbc.withdraw_leftover" => {
            let c = dbc::Curve::new(pk(&p["config"]), pk(&p["base_mint"]), quote_of(p));
            dbc::withdraw_leftover(&c, pk(&p["leftover_receiver"]))
        }
        "damm.initialize_customizable_pool" => {
            let pool = damm::Pool::customizable(pk(&p["token_a_mint"]), pk(&p["token_b_mint"]));
            damm::initialize_customizable_pool(&pool, pk(&p["creator"]), pk(&p["payer"]), pk(&p["nft_mint"]), pk(&p["payer_token_a"]), pk(&p["payer_token_b"]), &cp_params(p))
        }
        "damm.swap" => damm::swap(&pool_of(p), pk(&p["payer"]), pk(&p["input"]), pk(&p["output"]), n64(&p["amount_in"]), n64(&p["minimum_amount_out"])),
        "damm.swap2" => damm::swap2(&pool_of(p), pk(&p["payer"]), pk(&p["input"]), pk(&p["output"]), n64(&p["amount_0"]), n64(&p["amount_1"]), n64(&p["swap_mode"]) as u8),
        "damm.claim_position_fee" => damm::claim_position_fee(&pool_of(p), pk(&p["nft_mint"]), pk(&p["owner"]), pk(&p["token_a_account"]), pk(&p["token_b_account"])),
        "damm.add_liquidity" => damm::add_liquidity(&pool_of(p), pk(&p["nft_mint"]), pk(&p["owner"]), pk(&p["token_a_account"]), pk(&p["token_b_account"]),
            n(&p["liquidity_delta"]), n64(&p["token_a_amount_threshold"]), n64(&p["token_b_amount_threshold"])),
        "damm.remove_liquidity" => damm::remove_liquidity(&pool_of(p), pk(&p["nft_mint"]), pk(&p["owner"]), pk(&p["token_a_account"]), pk(&p["token_b_account"]),
            n(&p["liquidity_delta"]), n64(&p["token_a_amount_threshold"]), n64(&p["token_b_amount_threshold"])),
        "damm.remove_all_liquidity" => damm::remove_all_liquidity(&pool_of(p), pk(&p["nft_mint"]), pk(&p["owner"]), pk(&p["token_a_account"]), pk(&p["token_b_account"]),
            n64(&p["token_a_amount_threshold"]), n64(&p["token_b_amount_threshold"])),
        "damm.close_position" => damm::close_position(&wx_pool(), pk(&p["nft_mint"]), pk(&p["owner"]), pk(&p["rent_receiver"])),
        "damm.create_position" => damm::create_position(&wx_pool(), pk(&p["owner"]), pk(&p["payer"]), pk(&p["nft_mint"])),
        other => panic!("no builder for {other}"),
    }
}
/// The flow's coin mint and wX mint (the DBC instructions that name only one of them).
fn keys() -> Value {
    flow()["keys"].clone()
}
fn base_of(_: &Value) -> Pubkey {
    pk(&keys()["coin_mint"])
}
fn quote_of(_: &Value) -> Pubkey {
    pk(&keys()["wx_mint"])
}
fn wx_pool() -> damm::Pool {
    damm::Pool::customizable(pk(&keys()["wx_mint"]), solix2::native_mint())
}
fn cp_params(p: &Value) -> damm::CustomizablePool {
    damm::CustomizablePool {
        fee_numerator: n64(&p["fee_numerator"]),
        compounding_fee_bps: 0,
        sqrt_min_price: n(&p["sqrt_min_price"]),
        sqrt_max_price: n(&p["sqrt_max_price"]),
        liquidity: n(&p["liquidity"]),
        sqrt_price: n(&p["sqrt_price"]),
        activation_type: n64(&p["activation_type"]) as u8,
        collect_fee_mode: n64(&p["collect_fee_mode"]) as u8,
        activation_point: None,
    }
}

fn show(i: &Instruction) -> String {
    format!(
        "program {}\n{}\ndata {}",
        soltx::b58_encode(&i.program),
        i.accounts.iter().map(|a| format!("  {} s={} w={}", soltx::b58_encode(&a.key), a.signer, a.writable)).collect::<Vec<_>>().join("\n"),
        hex::encode(&i.data)
    )
}
fn assert_same(name: &str, built: &Instruction, want: &Instruction) {
    assert!(built == want, "{name}: the canister's builder differs from the landed instruction\n-- built --\n{}\n-- landed --\n{}", show(built), show(want));
}

#[test]
fn every_landed_instruction_is_rebuilt_byte_for_byte() {
    let f = flow();
    let ixs = f["instructions"].as_array().unwrap();
    let mut names: Vec<&str> = vec![];
    for j in ixs {
        let name = st(&j["name"]);
        assert_same(name, &build(name, &j["params"]), &fixture_ix(j));
        names.push(name);
    }
    names.sort();
    names.dedup();
    // every builder of W2 is covered by at least one landed instruction
    for want in [
        "system.create_account", "spl.initialize_mint2", "spl.set_authority", "spl.mint_to", "spl.mint_to_checked", "spl.burn", "spl.burn_checked", "spl.transfer",
        "spl.transfer_checked", "spl.close_account", "spl.sync_native", "ata.create", "ata.create_idempotent", "memo", "mpl.create_metadata_v3", "mpl.update_metadata_v2",
        "dbc.create_config", "dbc.initialize_virtual_pool_with_spl_token", "dbc.swap", "dbc.swap2", "dbc.claim_trading_fee", "dbc.migration_damm_v2",
        "dbc.withdraw_migration_fee", "dbc.partner_withdraw_surplus", "dbc.withdraw_leftover", "damm.initialize_customizable_pool", "damm.create_position",
        "damm.add_liquidity", "damm.remove_liquidity", "damm.remove_all_liquidity", "damm.claim_position_fee", "damm.close_position", "damm.swap", "damm.swap2",
    ] {
        assert!(names.contains(&want), "no landed fixture for {want}");
    }
    assert_eq!(ixs.len(), 42);
}

#[test]
fn the_dynamic_config_pool_matches_the_idl_client() {
    // the re-centring path (a new pool: the range is the pool's in DAMM v2); no dynamic config exists in LiteSVM, so this one is
    // checked against the Anchor client generated from the official IDL, not landed
    let f = flow();
    let j = &f["idl_only"][0];
    assert_eq!(st(&j["name"]), "damm.initialize_pool_with_dynamic_config");
    let p = &j["params"];
    let (pool, built) = damm::initialize_pool_with_dynamic_config(pk(&p["config"]), pk(&p["token_a_mint"]), pk(&p["token_b_mint"]), pk(&p["creator"]), pk(&p["payer"]),
        pk(&p["pool_creator_authority"]), pk(&p["nft_mint"]), pk(&p["payer_token_a"]), pk(&p["payer_token_b"]), &cp_params(p));
    assert_eq!(pool.pool, pk(&p["pool"]));
    assert_same("damm.initialize_pool_with_dynamic_config", &built, &fixture_ix(j));
    assert_ne!(pool.pool, pk(&keys()["wx_sol_pool"]), "a second pool of the same pair, at another address");
}

#[test]
fn derived_addresses_match_the_programs() {
    let k = keys();
    let (wx, coin, config) = (pk(&k["wx_mint"]), pk(&k["coin_mint"]), pk(&k["dbc_config"]));
    assert_eq!(dbc::Curve::new(config, coin, wx).pool, pk(&k["dbc_pool"]));
    assert_eq!(damm::Pool::migrated(coin, wx).pool, pk(&k["migrated_pool"]));
    assert_eq!(damm::Pool::customizable(wx, solix2::native_mint()).pool, pk(&k["wx_sol_pool"]));
    assert_eq!(solix2::metadata_address(&wx), pk(&k["wx_metadata"]));
    assert_eq!(solix2::event_authority(&dbc::program()), pk(&k["dbc_event_authority"]));
    assert_eq!(solix2::event_authority(&damm::program()), pk(&k["damm_event_authority"]));
    assert_eq!(solix2::id(solix2::DBC_POOL_AUTHORITY), pk(&k["dbc_pool_authority"]));
    assert_eq!(solix2::id(solix2::DAMM_POOL_AUTHORITY), pk(&k["damm_pool_authority"]));
    // the canister's keys are on-curve wallets; every program-derived address here is off-curve
    for a in ["k_main", "k_coll", "k_desk", "k_coin"] {
        assert!(soltx::is_on_curve(&pk(&k[a])), "{a} is a wallet");
    }
    for a in ["dbc_pool", "migrated_pool", "wx_sol_pool", "wx_metadata", "dbc_event_authority"] {
        assert!(!soltx::is_on_curve(&pk(&k[a])), "{a} is a PDA");
    }
}

/// The wire size of a legacy transaction (1 + 64 per signature + the message), as the spikes measured it.
fn size(ixs: &[Instruction], payer: Pubkey) -> usize {
    soltx::wire_size(&soltx::compile(ixs, payer, [7; 32]))
}

#[test]
fn the_sites_two_hop_buy_is_the_landed_bundle_and_fits() {
    let f = flow();
    let k = keys();
    let wx = pk(&k["wx_mint"]);
    let wx_pool = wx_pool();
    for b in f["bundles"].as_array().unwrap() {
        let p = &b["params"];
        let curve = dbc::Curve::new(pk(&k["dbc_config"]), pk(&p["coin_mint"]), wx);
        let mig = damm::Pool::migrated(pk(&p["coin_mint"]), wx);
        let pre = st(&b["name"]) == "site.two_hop_pre_graduation";
        let hop = if pre { route::CoinHop::Curve(&curve) } else { route::CoinHop::Graduated(&mig) };
        let built = route::two_hop_buy(pk(&p["user"]), &wx_pool, wx, hop, n64(&p["sol_in"]), n64(&p["wx_min"]), n64(&p["coin_min"]), n64(&p["cu_limit"]) as u32,
            n64(&p["cu_price"]));
        let want: Vec<Instruction> = b["instructions"].as_array().unwrap().iter().map(fixture_ix).collect();
        assert_eq!(built.len(), want.len());
        for (i, (x, y)) in built.iter().zip(&want).enumerate() {
            assert_same(&format!("{} #{i}", st(&b["name"])), x, y);
        }
        let s = size(&built, pk(&p["user"]));
        // spikes S1–S3 §3.5: 1,033 B on the curve, 903 B graduated (one signature, no lookup table)
        assert_eq!(s, if pre { 1_033 } else { 903 }, "{}", st(&b["name"]));
        assert!(s <= soltx::PACKET_DATA_SIZE);
    }
}

#[test]
fn the_pool_opening_fits_a_durable_nonce_only_trimmed() {
    let k = keys();
    let (k_main, k_coll, k_desk, wx) = (pk(&k["k_main"]), pk(&k["k_coll"]), pk(&k["k_desk"]), pk(&k["wx_mint"]));
    let nft = pk(&k["desk_nft"]);
    let nonce = [9u8; 32];
    let s0: u128 = 18_446_744_073_709_551_616 * 43 / 10; // ≈ √18.61 · 2^64 (sizes do not depend on values)
    let (lo, hi) = route::range_around(s0, 11, 10).unwrap();
    let params = damm::CustomizablePool { fee_numerator: 5_000_000, compounding_fee_bps: 0, sqrt_min_price: lo, sqrt_max_price: hi, liquidity: 1 << 70,
        sqrt_price: s0, activation_type: damm::ACTIVATION_TIMESTAMP, collect_fee_mode: damm::COLLECT_FEE_ONLY_B, activation_point: None };
    let (pool, open) = route::open_pool(k_coll, k_desk, wx, nft, 537_345_513, 10_000_000_000, &params);
    assert_eq!(pool.pool, pk(&k["wx_sol_pool"]));
    let cu = [soltx::compute_unit_limit(1_400_000), soltx::compute_unit_price(1)];
    let trimmed: Vec<Instruction> = cu.iter().cloned().chain(open.iter().cloned()).collect();
    let trimmed = soltx::with_nonce(nonce, k_main, &trimmed);
    // the spike's measured fit: pre-made desk ATAs, no memo, nonce in front: 1,195 B
    assert_eq!(size(&trimmed, k_main), 1_195);
    // as first designed (both ATAs + memo in the same transaction, nonce in front): over the limit
    let mut full = route::desk_atas(k_main, k_desk, wx);
    full.extend(open.iter().take(1).cloned());
    full.extend(open.iter().skip(1).cloned());
    full.push(route::open_pool_memo(0, k_coll));
    let full: Vec<Instruction> = cu.iter().cloned().chain(full).collect();
    let full = soltx::with_nonce(nonce, k_main, &full);
    assert!(size(&full, k_main) > soltx::PACKET_DATA_SIZE, "{}", size(&full, k_main));
    // signers: K_main (fee payer, nonce authority), K_coll (mint), K_desk (wrap, pool payer), the position-NFT mint
    let m = soltx::compile(&trimmed, k_main, [7; 32]);
    let signers: Vec<Pubkey> = m.signers.clone();
    assert_eq!(signers.len(), 4);
    for s in [k_main, k_coll, k_desk, nft] {
        assert!(signers.contains(&s));
    }
    assert_eq!(signers[0], k_main, "the fee payer signs first");
    // the ATAs and the memo travel on their own and are small
    assert!(size(&route::desk_atas(k_main, k_desk, wx), k_main) < 600);
    assert!(guard::refuse_forbidden(&trimmed).is_ok());
}

fn account(name: &str) -> (Pubkey, Vec<u8>, Value) {
    let f = flow();
    let a = f["accounts"].as_array().unwrap().iter().find(|a| a["name"] == name).unwrap_or_else(|| panic!("no account {name}")).clone();
    (pk(&a["owner"]), base64::engine::general_purpose::STANDARD.decode(st(&a["data"])).unwrap(), a["expect"].clone())
}
fn okey(v: &Value) -> Option<Pubkey> {
    (!v.is_null()).then(|| pk(v))
}

#[test]
fn spl_mint_and_token_account_decode_as_spl_token_unpacks_them() {
    for name in ["wx_mint", "frozen_mint"] {
        let (o, d, e) = account(name);
        let m = decode::Mint::decode(&o, &d).unwrap();
        assert_eq!(m.mint_authority, okey(&e["mint_authority"]), "{name}");
        assert_eq!(m.freeze_authority, okey(&e["freeze_authority"]), "{name}");
        assert_eq!(m.supply, n64(&e["supply"]));
        assert_eq!(m.decimals as u64, n64(&e["decimals"]));
        assert_eq!(m.is_initialized, e["is_initialized"].as_bool().unwrap());
    }
    for name in ["desk_wx_ata", "desk_wsol_ata"] {
        let (o, d, e) = account(name);
        let t = decode::TokenAccount::decode(&o, &d).unwrap();
        assert_eq!(t.mint, pk(&e["mint"]));
        assert_eq!(t.owner, pk(&e["owner"]));
        assert_eq!(t.amount, n64(&e["amount"]));
        assert_eq!(t.delegate, okey(&e["delegate"]));
        assert_eq!(t.state as u64, n64(&e["state"]));
        assert_eq!(t.is_native, (!e["is_native"].is_null()).then(|| n64(&e["is_native"])), "{name}");
        assert_eq!(t.delegated_amount, n64(&e["delegated_amount"]));
        assert_eq!(t.close_authority, okey(&e["close_authority"]));
    }
    // wrong owner, short data, a bad COption tag: errors, not panics
    let (o, d, _) = account("wx_mint");
    assert!(decode::Mint::decode(&solix2::id(solix2::TOKEN_2022_PROGRAM), &d).is_err());
    assert!(decode::Mint::decode(&o, &d[..81]).is_err());
    let mut bad = d.clone();
    bad[0] = 7;
    assert!(decode::Mint::decode(&o, &bad).is_err());
    assert!(decode::TokenAccount::decode(&o, &d).is_err());
}

#[test]
fn dbc_config_and_pool_decode_as_the_program_wrote_them() {
    let (o, d, e) = account("dbc_config");
    let c = decode::DbcConfig::decode(&o, &d).unwrap();
    let k = keys();
    assert_eq!(c.quote_mint, pk(&e["quote_mint"]));
    assert_eq!(c.quote_mint, pk(&k["wx_mint"]));
    assert_eq!(c.fee_claimer, pk(&e["fee_claimer"]));
    assert_eq!(c.leftover_receiver, pk(&e["leftover_receiver"]));
    for (f, v) in [
        ("collect_fee_mode", c.collect_fee_mode), ("migration_option", c.migration_option), ("activation_type", c.activation_type), ("token_type", c.token_type),
        ("partner_permanent_locked_liquidity_percentage", c.partner_permanent_locked_liquidity_percentage), ("partner_liquidity_percentage", c.partner_liquidity_percentage),
        ("creator_permanent_locked_liquidity_percentage", c.creator_permanent_locked_liquidity_percentage), ("creator_liquidity_percentage", c.creator_liquidity_percentage),
        ("migration_fee_option", c.migration_fee_option), ("creator_trading_fee_percentage", c.creator_trading_fee_percentage),
        ("migration_fee_percentage", c.migration_fee_percentage), ("creator_migration_fee_percentage", c.creator_migration_fee_percentage), ("version", c.version),
        ("fixed_token_supply_flag", c.fixed_token_supply_flag), ("migrated_collect_fee_mode", c.migrated_collect_fee_mode),
        ("partner_vesting_initialized", c.partner_vesting_initialized), ("creator_vesting_initialized", c.creator_vesting_initialized), ("token_decimal", c.token_decimal),
        ("token_update_authority", c.token_update_authority),
    ] {
        assert_eq!(v as u64, n64(&e[f]), "{f}");
    }
    assert_eq!(c.migration_quote_threshold, n64(&e["migration_quote_threshold"]));
    assert_eq!(c.pre_migration_token_supply, n64(&e["pre_migration_token_supply"]));
    assert_eq!(c.post_migration_token_supply, n64(&e["post_migration_token_supply"]));
    assert_eq!(c.migrated_compounding_fee_bps as u64, n64(&e["migrated_compounding_fee_bps"]));
    assert_eq!(c.pool_creation_fee, n64(&e["pool_creation_fee"]));
    assert_eq!(c.sqrt_start_price, n(&e["sqrt_start_price"]));
    assert_eq!(c.curve, curve_of(&e["curve"]));
    assert_eq!((c.cliff_fee_numerator, c.fee_first_factor as u64, c.fee_second_factor, c.fee_third_factor),
        (n64(&e["cliff_fee_numerator"]), n64(&e["fee_periods"]), n64(&e["fee_period_secs"]), n64(&e["fee_reduction"])));
    assert_eq!(c.migrated_pool_fee_bps, dbc::pins::MIGRATED_POOL_FEE_BPS);
    // DBC wrote the migration price at the curve's first point, and a raise over it of exactly T_wX (to a base unit)
    assert_eq!(c.migration_sqrt_price, c.curve[0].0);
    assert!(guard::quote_raised(c.sqrt_start_price, &c.curve, c.migration_sqrt_price).abs_diff(c.migration_quote_threshold as u128) <= 1);

    for name in ["dbc_pool_graduated", "dbc_pool_migrated"] {
        let (o, d, e) = account(name);
        let p = decode::DbcPool::decode(&o, &d).unwrap();
        assert_eq!(p.config, pk(&e["config"]));
        assert_eq!(p.creator, pk(&e["creator"]));
        assert_eq!(p.base_mint, pk(&e["base_mint"]));
        assert_eq!(p.base_vault, pk(&e["base_vault"]));
        assert_eq!(p.quote_vault, pk(&e["quote_vault"]));
        assert_eq!(p.quote_reserve, n64(&e["quote_reserve"]));
        assert_eq!(p.activation_point, n64(&e["activation_point"]));
        assert_eq!(p.is_migrated as u64, n64(&e["is_migrated"]), "{name}");
        assert_eq!(p.migration_progress as u64, n64(&e["migration_progress"]));
        assert_eq!(p.migration_fee_withdraw_status as u64, n64(&e["migration_fee_withdraw_status"]));
        // the curve that the builders address is the one the program holds
        let curve = dbc::Curve::new(p.config, p.base_mint, pk(&k["wx_mint"]));
        assert_eq!((curve.base_vault, curve.quote_vault), (p.base_vault, p.quote_vault));
    }
    let (o, d, _) = account("dbc_pool_graduated");
    let p = decode::DbcPool::decode(&o, &d).unwrap();
    assert!(!p.is_migrated && p.quote_reserve >= 5 * dbc::pins::WX_ONE);
    assert!(decode::DbcConfig::decode(&o, &d).is_err(), "a pool is not a config");
    assert!(decode::DbcPool::decode(&damm::program(), &d).is_err(), "owner checked");
}

#[test]
fn damm_pool_and_positions_decode_as_the_program_wrote_them() {
    let (o, d, e) = account("wx_sol_pool");
    let p = decode::DammPool::decode(&o, &d).unwrap();
    assert_eq!((p.token_a_mint, p.token_b_mint, p.token_a_vault, p.token_b_vault),
        (pk(&e["token_a_mint"]), pk(&e["token_b_mint"]), pk(&e["token_a_vault"]), pk(&e["token_b_vault"])));
    assert_eq!((p.liquidity, p.sqrt_min_price, p.sqrt_max_price, p.sqrt_price),
        (n(&e["liquidity"]), n(&e["sqrt_min_price"]), n(&e["sqrt_max_price"]), n(&e["sqrt_price"])));
    assert_eq!((p.activation_type as u64, p.collect_fee_mode as u64), (n64(&e["activation_type"]), n64(&e["collect_fee_mode"])));
    assert_eq!(p.creator, pk(&e["creator"]));
    assert_eq!(u64::from_le_bytes(p.base_fee_info[..8].try_into().unwrap()), 5_000_000, "the fixed 0.5 % fee");
    assert!((p.price_f64() - 18.61).abs() < 1e-6, "{}", p.price_f64());
    assert_eq!(p.dynamic_fee_initialized, 0);
    let pool = wx_pool();
    assert_eq!((pool.vault_a, pool.vault_b), (p.token_a_vault, p.token_b_vault));

    let (o, d, e) = account("desk_position");
    let q = decode::DammPosition::decode(&o, &d).unwrap();
    assert_eq!((q.pool, q.nft_mint), (pk(&e["pool"]), pk(&e["nft_mint"])));
    assert_eq!((q.unlocked_liquidity, q.vested_liquidity, q.permanent_locked_liquidity),
        (n(&e["unlocked_liquidity"]), n(&e["vested_liquidity"]), n(&e["permanent_locked_liquidity"])));
    assert_eq!(q.liquidity(), p.liquidity, "the desk's one position is the pool's whole liquidity");

    let (o, d, e) = account("partner_position");
    let q = decode::DammPosition::decode(&o, &d).unwrap();
    assert_eq!((q.pool, q.nft_mint), (pk(&e["pool"]), pk(&e["nft_mint"])));
    assert!(q.permanent_locked_liquidity > 0 && q.unlocked_liquidity <= 1 && q.vested_liquidity == 0, "the partner LP is permanently locked");
    assert!(q.total_claimed_b_fee > 0 && q.total_claimed_a_fee == 0, "fees claimed in wX (token b) only");
    let (o, d, e) = account("migrated_pool");
    let m = decode::DammPool::decode(&o, &d).unwrap();
    assert_eq!((m.token_a_mint, m.token_b_mint), (pk(&e["token_a_mint"]), pk(&e["token_b_mint"])));
    assert_eq!(m.collect_fee_mode as u64, n64(&e["collect_fee_mode"]));
    assert!(m.permanent_lock_liquidity > 0);
    assert!(decode::DammPosition::decode(&o, &d).is_err(), "a pool is not a position");
}

#[test]
fn account_discriminators_are_anchors() {
    assert_eq!(decode::DBC_CONFIG_DISC, soltx::anchor_disc("account", "PoolConfig"));
    assert_eq!(decode::DBC_POOL_DISC, soltx::anchor_disc("account", "VirtualPool"));
    assert_eq!(decode::DAMM_POOL_DISC, soltx::anchor_disc("account", "Pool"));
    assert_eq!(decode::DAMM_POSITION_DISC, soltx::anchor_disc("account", "Position"));
}

// ---- the guards: what DBC does not check ------------------------------------------------------------------------------------------

#[test]
fn the_wx_mint_check_refuses_a_freeze_authority_and_a_foreign_mint_authority() {
    let k = keys();
    let k_coll = pk(&k["k_coll"]);
    let (o, d, _) = account("wx_mint");
    let m = decode::Mint::decode(&o, &d).unwrap();
    guard::check_wx_mint(&m, &k_coll).unwrap();
    assert!(guard::check_wx_mint(&m, &pk(&k["k_desk"])).unwrap_err().contains("mint authority"));
    // DBC accepts a quote mint with a freeze authority (S1b); Rook does not
    let (o, d, _) = account("frozen_mint");
    let f = decode::Mint::decode(&o, &d).unwrap();
    assert!(guard::check_wx_mint(&f, &k_coll).unwrap_err().contains("freeze authority"));
    let mut six = m.clone();
    six.decimals = 6;
    assert!(guard::check_wx_mint(&six, &k_coll).is_err());
}

fn pin() -> guard::ConfigPin {
    let f = flow();
    let cc = f["instructions"].as_array().unwrap().iter().find(|i| i["name"] == "dbc.create_config").unwrap()["params"].clone();
    guard::ConfigPin { wx_mint: pk(&cc["quote_mint"]), k_fc: pk(&cc["fee_claimer"]), t_wx: n64(&cc["t_wx"]), sqrt_start_price: n(&cc["sqrt_start_price"]),
        curve: curve_of(&cc["curve"]) }
}

#[test]
fn the_config_check_accepts_rooks_config_and_refuses_what_dbc_lets_through() {
    let (o, d, _) = account("dbc_config");
    let c = decode::DbcConfig::decode(&o, &d).unwrap();
    let pin = pin();
    guard::check_config(&c, &pin).unwrap();
    let refused = |f: &dyn Fn(&mut decode::DbcConfig), why: &str| {
        let mut x = c.clone();
        f(&mut x);
        let e = guard::check_config(&x, &pin).unwrap_err();
        assert!(e.contains(why), "{why}: {e}");
    };
    let other = [3u8; 32];
    refused(&|x| x.quote_mint = other, "quote");
    refused(&|x| x.fee_claimer = other, "fee_claimer");
    refused(&|x| x.leftover_receiver = other, "leftover_receiver");
    // version 1 is DBC's current layout (devnet, 2026-10-12: identical fields); a later version is refused
    refused(&|x| x.version = 2, "version");
    refused(&|x| x.pool_creation_fee = 1, "pool_creation_fee");
    refused(&|x| x.partner_liquidity_percentage = 50, "locked");
    refused(&|x| x.creator_migration_fee_percentage = 50, "creator_migration_fee_percentage");
    refused(&|x| x.activation_type = 0, "activation_type");
    // a threshold that is not the curve's (S1b: DBC accepted 1 wX on the 3-wX curve)
    refused(&|x| x.migration_quote_threshold = 3 * dbc::pins::WX_ONE, "threshold");
    refused(&|x| x.migration_quote_threshold = 3 * dbc::pins::WX_ONE, "raises");
    // the migration price DBC would derive from a lower threshold sits inside the first segment
    refused(&|x| {
        x.migration_sqrt_price = x.sqrt_start_price + (x.curve[0].0 - x.sqrt_start_price) / 5;
    }, "first point");
    refused(&|x| x.curve[1].1 += 1, "curve");
    // the pin for another T_wX does not fit this config
    let mut p2 = pin.clone();
    p2.t_wx = 7;
    assert!(guard::check_config(&c, &p2).is_err());
}

#[test]
fn quote_raised_matches_dbc_on_every_pinned_curve_shape() {
    let pin = pin();
    let end = pin.curve[0].0;
    let r = guard::quote_raised(pin.sqrt_start_price, &pin.curve, end);
    assert!(r.abs_diff(5 * dbc::pins::WX_ONE as u128) <= 1, "{r}");
    // half way in √price raises less; past the first point uses the second segment's liquidity
    assert!(guard::quote_raised(pin.sqrt_start_price, &pin.curve, (pin.sqrt_start_price + end) / 2) < r);
    assert!(guard::quote_raised(pin.sqrt_start_price, &pin.curve, end * 2) > r);
    assert_eq!(guard::quote_raised(pin.sqrt_start_price, &pin.curve, pin.sqrt_start_price), 0);
    // every curve of §3.2 (design_curve.mjs 1e9 20 <T_wX> 50 6; the 20 that S1b saw DBC accept): its first segment raises T_wX
    let p = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/solix2/wx_curves.json");
    let all: Value = serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap();
    let mut seen = 0;
    for (t, c) in all.as_object().unwrap() {
        let t: u64 = t.parse().unwrap();
        let curve: Vec<(u128, u128)> = c["curve"].as_array().unwrap().iter().map(|p| (n(&p["sqrtPrice"]), n(&p["liquidity"]))).collect();
        let start = n(&c["sqrtStartPrice"]);
        let raised = guard::quote_raised(start, &curve, curve[0].0);
        let want = (t * dbc::pins::WX_ONE) as u128;
        assert!(raised.abs_diff(want) <= want / 1_000_000, "T_wX {t}: {raised} vs {want}");
        // and the config the canister would encode for it carries that threshold and curve
        let cfg = dbc::rook_wx_config(t, start, curve.clone());
        assert_eq!(cfg.migration_quote_threshold, t * dbc::pins::WX_ONE);
        seen += 1;
    }
    assert_eq!(seen, 20);
}

#[test]
fn the_canister_never_signs_set_authority_approve_or_an_authority_moving_metadata_update() {
    let k = keys();
    let (wx, k_coll, k_desk) = (pk(&k["wx_mint"]), pk(&k["k_coll"]), pk(&k["k_desk"]));
    let ata = soltx::associated_token_address(&k_desk, &wx);
    let ok = vec![spl::mint_to_checked(wx, ata, k_coll, 1, 9), spl::memo("rook:v1:0:mint:1", &[k_coll]), mpl::update_metadata_v2(wx, k_coll, "Wrapped X", "wX", "u"),
        mpl::create_metadata_v3(wx, k_coll, k_coll, k_coll, "n", "s", "u")];
    guard::refuse_forbidden(&ok).unwrap();
    for (what, bad) in [
        ("MintTokens", spl::set_authority(wx, spl::AuthorityType::MintTokens, Some(k_desk), k_coll)),
        ("renounce", spl::set_authority(wx, spl::AuthorityType::MintTokens, None, k_coll)),
        ("account owner", spl::set_authority(ata, spl::AuthorityType::AccountOwner, Some(k_coll), k_desk)),
        ("approve", soltx::Instruction { program: solix2::token_program(), accounts: vec![], data: vec![spl::tag::APPROVE, 1, 0, 0, 0, 0, 0, 0, 0] }),
    ] {
        let mut v = ok.clone();
        v.push(bad);
        assert!(guard::refuse_forbidden(&v).is_err(), "{what}");
    }
    // an update that moves the update authority (Some(new) in the trailing options) is refused
    let mut moving = mpl::update_metadata_v2(wx, k_coll, "n", "s", "u");
    let l = moving.data.len();
    moving.data.truncate(l - 3);
    moving.data.push(1);
    moving.data.extend(k_desk);
    moving.data.extend([0, 0]);
    assert!(guard::refuse_forbidden(&[moving]).is_err());
    // the same SetAuthority under Token-2022 is refused too
    let mut t22 = spl::set_authority(wx, spl::AuthorityType::MintTokens, Some(k_desk), k_coll);
    t22.program = solix2::id(solix2::TOKEN_2022_PROGRAM);
    assert!(guard::refuse_forbidden(&[t22]).is_err());
}

#[test]
fn the_narrow_range_is_ten_percent_in_sqrt_price() {
    let s: u128 = 79_582_404_548_441_276_416; // ≈ √18.61 · 2^64
    let (lo, hi) = route::range_around(s, 11, 10).unwrap();
    assert_eq!(lo, s * 10 / 11);
    assert_eq!(hi, s * 11 / 10);
    let (pl, ph) = ((lo as f64 / s as f64).powi(2), (hi as f64 / s as f64).powi(2));
    assert!((1.0 / pl - 1.21).abs() < 1e-9 && (ph - 1.21).abs() < 1e-9);
    assert!(route::range_around(s, 10, 10).is_none());
    assert!(route::range_around(u128::MAX / 2, 11, 10).is_none());
    assert!(route::range_around(damm::MIN_SQRT_PRICE, 11, 10).is_none());
}
