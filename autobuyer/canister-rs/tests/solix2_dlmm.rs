//! W7: the canister's Meteora DLMM builders and decoders (`src/solix2/dlmm.rs`), byte for byte against
//! `tests/fixtures/solix2/dlmm.json`: instructions that LANDED in LiteSVM on the mainnet bytes of DLMM (`LBUZ…`, IDL 0.12.0) and its
//! mainnet preset 262, in spike S4's desk lifecycle (written by `solana/tests/dlmm_fixtures.rs`); and the accounts that program wrote
//! (`LbPair`, `PositionV2`, `BinArray`), decoded to the numbers the chain actually paid.
use base64::Engine;
use etch_autobuyer::solix2::{self, dlmm, guard};
use etch_autobuyer::soltx::{self, Instruction, Pubkey};
use serde_json::Value;
use sha2::{Digest, Sha256};

fn flow() -> Value {
    let p = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/solix2/dlmm.json");
    serde_json::from_str(&std::fs::read_to_string(p).expect("dlmm.json")).expect("json")
}
fn pk(v: &Value) -> Pubkey {
    soltx::key(v.as_str().unwrap_or_else(|| panic!("a key, got {v}")))
}
fn n(v: &Value) -> i64 {
    match v {
        Value::Number(x) => x.as_i64().or_else(|| x.as_u64().map(|u| u as i64)).expect("integer"),
        Value::String(s) => s.parse().expect("integer string"),
        _ => panic!("a number, got {v}"),
    }
}
fn nu(v: &Value) -> u64 {
    n(v) as u64
}
fn st(v: &Value) -> &str {
    v.as_str().expect("string")
}
fn keys_of(v: &Value) -> Vec<Pubkey> {
    v.as_array().unwrap().iter().map(pk).collect()
}
fn fixture_ix(j: &Value) -> Instruction {
    Instruction {
        program: pk(&j["program"]),
        accounts: j["accounts"].as_array().unwrap().iter().map(|a| soltx::meta(pk(&a[0]), a[1].as_bool().unwrap(), a[2].as_bool().unwrap())).collect(),
        data: hex::decode(st(&j["data"])).unwrap(),
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
fn lb_of(p: &Value) -> dlmm::Lb {
    dlmm::Lb::new(pk(&p["pair"]), pk(&p["x"]), pk(&p["y"]), dlmm::DESK_BIN_STEP)
}

/// The canister's builder for one recorded instruction, from its recorded parameters.
fn build(name: &str, p: &Value) -> Instruction {
    match name {
        "dlmm.initialize_lb_pair2" => dlmm::initialize_lb_pair2(pk(&p["funder"]), pk(&p["preset"]), pk(&p["x"]), pk(&p["y"]), n(&p["active_id"]) as i32),
        "dlmm.initialize_bin_array" => dlmm::initialize_bin_array(pk(&p["pair"]), n(&p["index"]), pk(&p["funder"])),
        "dlmm.initialize_position_pda" => dlmm::initialize_position_pda(pk(&p["payer"]), pk(&p["base"]), pk(&p["pair"]), pk(&p["owner"]), n(&p["lower"]) as i32, n(&p["width"]) as i32),
        "dlmm.increase_position_length2" => dlmm::increase_position_length2(pk(&p["funder"]), pk(&p["pair"]), pk(&p["position"]), pk(&p["owner"]), n(&p["min_upper"]) as i32),
        "dlmm.add_liquidity_by_strategy2" => {
            assert_eq!(n(&p["shape"]), dlmm::Shape::Spot as i64);
            dlmm::add_liquidity_by_strategy2(&lb_of(p), pk(&p["position"]), pk(&p["sender"]), pk(&p["ux"]), pk(&p["uy"]), nu(&p["ax"]), nu(&p["ay"]), n(&p["active_id"]) as i32,
                n(&p["lo"]) as i32, n(&p["hi"]) as i32, dlmm::Shape::Spot)
        }
        "dlmm.swap2" => dlmm::swap2(&lb_of(p), pk(&p["user"]), pk(&p["input"]), pk(&p["output"]), nu(&p["amount_in"]), nu(&p["min_out"]), &keys_of(&p["arrays"])),
        "dlmm.claim_fee2" => dlmm::claim_fee2(&lb_of(p), pk(&p["position"]), pk(&p["sender"]), pk(&p["ux"]), pk(&p["uy"]), n(&p["lo"]) as i32, n(&p["hi"]) as i32),
        "dlmm.remove_liquidity_by_range2" => dlmm::remove_liquidity_by_range2(&lb_of(p), pk(&p["position"]), pk(&p["sender"]), pk(&p["ux"]), pk(&p["uy"]), n(&p["lo"]) as i32,
            n(&p["hi"]) as i32, n(&p["bps"]) as u16),
        "dlmm.close_position2" => dlmm::close_position2(pk(&p["position"]), pk(&p["sender"]), pk(&p["rent_to"])),
        "dlmm.go_to_a_bin" => dlmm::go_to_a_bin(pk(&p["pair"]), (!p["from_array"].is_null()).then(|| pk(&p["from_array"])), (!p["to_array"].is_null()).then(|| pk(&p["to_array"])),
            n(&p["bin"]) as i32),
        "dlmm.rebalance_liquidity" => {
            let remove = (!p["remove"].is_null()).then(|| (n(&p["remove"][0]) as i32, n(&p["remove"][1]) as i32));
            let adds: Vec<dlmm::RebalanceAdd> = p["adds"].as_array().unwrap().iter()
                .map(|a| dlmm::RebalanceAdd { min_delta: n(&a["min_delta"]) as i32, max_delta: n(&a["max_delta"]) as i32, x0: nu(&a["x0"]), y0: nu(&a["y0"]) })
                .collect();
            dlmm::rebalance_liquidity(&lb_of(p), pk(&p["position"]), pk(&p["owner"]), pk(&p["rent_payer"]), pk(&p["ux"]), pk(&p["uy"]), n(&p["active_id"]) as i32,
                p["claim_fee"].as_bool().unwrap(), remove, &adds, &keys_of(&p["arrays"]))
        }
        other => panic!("no builder for {other}"),
    }
}

#[test]
fn every_landed_dlmm_instruction_is_rebuilt_byte_for_byte() {
    let f = flow();
    let ixs = f["ixs"].as_array().unwrap();
    let mut names: Vec<&str> = vec![];
    for j in ixs {
        let name = st(&j["name"]);
        assert_same(name, &build(name, &j["params"]), &fixture_ix(j));
        names.push(name);
    }
    names.sort();
    names.dedup();
    // every builder of W7 is covered by at least one landed instruction
    for want in [
        "dlmm.initialize_lb_pair2", "dlmm.initialize_bin_array", "dlmm.initialize_position_pda", "dlmm.increase_position_length2", "dlmm.add_liquidity_by_strategy2",
        "dlmm.swap2", "dlmm.claim_fee2", "dlmm.remove_liquidity_by_range2", "dlmm.close_position2", "dlmm.go_to_a_bin", "dlmm.rebalance_liquidity",
    ] {
        assert!(names.contains(&want), "no landed fixture for {want}");
    }
    assert_eq!(ixs.len(), 19);
    // both rebalance forms landed: inside the band (remove + add) and past it (add only, after remove + go_to_a_bin)
    let rb: Vec<&Value> = ixs.iter().filter(|j| j["name"] == "dlmm.rebalance_liquidity").collect();
    assert_eq!(rb.len(), 2);
    assert!(!rb[0]["params"]["remove"].is_null() && rb[1]["params"]["remove"].is_null());
    // a go_to_a_bin with both arrays and one with neither (a squatted empty pair)
    let go: Vec<&Value> = ixs.iter().filter(|j| j["name"] == "dlmm.go_to_a_bin").collect();
    assert!(go.iter().any(|j| j["params"]["from_array"].is_null()) && go.iter().any(|j| !j["params"]["from_array"].is_null()));
}

#[test]
fn the_discriminators_are_the_idls() {
    let d = |name: &str| -> [u8; 8] { Sha256::digest(format!("global:{name}").as_bytes())[..8].try_into().unwrap() };
    assert_eq!(dlmm::disc::INITIALIZE_LB_PAIR2, d("initialize_lb_pair2"));
    assert_eq!(dlmm::disc::INITIALIZE_BIN_ARRAY, d("initialize_bin_array"));
    assert_eq!(dlmm::disc::INITIALIZE_POSITION_PDA, d("initialize_position_pda"));
    assert_eq!(dlmm::disc::INCREASE_POSITION_LENGTH2, d("increase_position_length2"));
    assert_eq!(dlmm::disc::ADD_LIQUIDITY_BY_STRATEGY2, d("add_liquidity_by_strategy2"));
    assert_eq!(dlmm::disc::REBALANCE_LIQUIDITY, d("rebalance_liquidity"));
    assert_eq!(dlmm::disc::REMOVE_LIQUIDITY_BY_RANGE2, d("remove_liquidity_by_range2"));
    assert_eq!(dlmm::disc::GO_TO_A_BIN, d("go_to_a_bin"));
    assert_eq!(dlmm::disc::SWAP2, d("swap2"));
    assert_eq!(dlmm::disc::CLAIM_FEE2, d("claim_fee2"));
    assert_eq!(dlmm::disc::CLOSE_POSITION2, d("close_position2"));
}

#[test]
fn derived_addresses_match_the_program() {
    let f = flow();
    let k = &f["keys"];
    let (wx, k_desk) = (pk(&k["wx_mint"]), pk(&k["k_desk"]));
    assert_eq!(dlmm::preset_address(dlmm::DESK_PRESET), pk(&k["preset"]));
    assert_eq!(dlmm::Lb::desk(wx).pair, pk(&k["pair"]));
    let (lo, hi) = (n(&k["lo"]) as i32, n(&k["hi"]) as i32);
    assert_eq!(dlmm::position_address(&pk(&k["pair"]), &k_desk, lo, hi - lo + 1), pk(&k["position"]));
    // a negative bin lies in a negative array (Euclidean division)
    assert_eq!((dlmm::array_index(-1), dlmm::array_index(0), dlmm::array_index(69), dlmm::array_index(70), dlmm::array_index(-70), dlmm::array_index(-71)), (-1, 0, 0, 1, -1, -2));
    // the pair, the arrays and the position are PDAs; the desk's keys are wallets
    assert!(!soltx::is_on_curve(&pk(&k["pair"])) && !soltx::is_on_curve(&pk(&k["position"])));
    assert!(soltx::is_on_curve(&k_desk));
    // swap arrays: ascending for a buy, descending for a sell, from the active bin's array
    let pair = pk(&k["pair"]);
    assert_eq!(dlmm::swap_arrays(&pair, 586, true, 2), vec![dlmm::bin_array_address(&pair, 8), dlmm::bin_array_address(&pair, 9)]);
    assert_eq!(dlmm::swap_arrays(&pair, 586, false, 2), vec![dlmm::bin_array_address(&pair, 8), dlmm::bin_array_address(&pair, 7)]);
}

/// The wire size of a legacy transaction behind a durable-nonce advance + CU limit + CU price (the canister's send line).
fn nonce_size(ixs: &[Instruction], payer: Pubkey) -> (usize, usize, usize) {
    let mut all = vec![soltx::compute_unit_limit(1_400_000), soltx::compute_unit_price(1)];
    all.extend(ixs.iter().cloned());
    let all = soltx::with_nonce([0x77; 32], payer, &all);
    let parts = soltx::compile_parts(&all, payer, [7; 32]).unwrap();
    let m = parts.message();
    (soltx::wire_size(&m), m.signers.len(), parts.keys.len())
}

#[test]
fn the_landed_transactions_have_the_sizes_the_spike_measured_and_fit() {
    let f = flow();
    let payer = pk(&f["keys"]["k_main"]);
    let mut sizes = vec![];
    for b in f["bundles"].as_array().unwrap() {
        let ixs: Vec<Instruction> = b["ixs"].as_array().unwrap().iter().map(fixture_ix).collect();
        let (size, signers, accounts) = nonce_size(&ixs, payer);
        assert_eq!(size as i64, n(&b["size_with_nonce"]), "{}", st(&b["name"]));
        assert_eq!(signers as i64, n(&b["signers_with_payer"]));
        assert_eq!(accounts as i64, n(&b["accounts"]));
        assert!(size <= soltx::PACKET_DATA_SIZE, "{} is {size} B", st(&b["name"]));
        // the signers the canister must hold: K_main plus the named keys, nothing else
        let named = keys_of(&b["signers"]);
        let mut all = vec![soltx::compute_unit_limit(1_400_000), soltx::compute_unit_price(1)];
        all.extend(ixs.iter().cloned());
        let m = soltx::compile(&soltx::with_nonce([0x77; 32], payer, &all), payer, [7; 32]);
        assert_eq!(m.signers[0], payer);
        for s in &m.signers {
            assert!(named.contains(s), "{}: an unexpected signer", st(&b["name"]));
        }
        assert!(guard::refuse_forbidden(&ixs).is_ok());
        sizes.push((st(&b["name"]).to_string(), size));
    }
    let size_of = |name: &str| sizes.iter().find(|(n, _)| n == name).unwrap().1;
    // spike S4 §2.2 and §2.4: the add 899 B, the claim 842 B (their own rows), the arb up 997 B plus its own accounts, R1 1,165 B, R2 exactly 1,232 B
    assert_eq!(size_of("add_liquidity"), 899);
    assert_eq!(size_of("claim_fee"), 842);
    assert_eq!(size_of("rebalance_inside"), 1_165);
    assert_eq!(size_of("rebalance_past"), 1_232, "R2 has no margin: the mint leg moves to an earlier transaction");
    assert_eq!(size_of("wind_down"), 899);
}

// ---- accounts ----------------------------------------------------------------------------------------------------------------------

fn account(name: &str) -> (Pubkey, Vec<u8>, Value) {
    let f = flow();
    let a = f["accounts"].as_array().unwrap().iter().find(|a| a["name"] == name).unwrap_or_else(|| panic!("no account {name}")).clone();
    (pk(&a["owner"]), base64::engine::general_purpose::STANDARD.decode(st(&a["data"])).unwrap(), a["expect"].clone())
}

#[test]
fn lb_pair_decodes_as_the_program_wrote_it() {
    for name in ["lb_pair_open", "lb_pair_after_add", "lb_pair_rebalance_inside", "lb_pair_rebalance_past"] {
        let (o, d, e) = account(name);
        let p = dlmm::LbPair::decode(&o, &d).unwrap();
        assert_eq!(p.active_id as i64, n(&e["active_id"]), "{name}");
        assert_eq!(p.bin_step as i64, n(&e["bin_step"]));
        assert_eq!(p.status as i64, n(&e["status"]));
        assert_eq!(p.pair_type as i64, n(&e["pair_type"]));
        assert_eq!(p.token_x_mint, pk(&e["token_x_mint"]));
        assert_eq!(p.token_y_mint, pk(&e["token_y_mint"]));
        assert_eq!(p.reserve_x, pk(&e["reserve_x"]));
        assert_eq!(p.reserve_y, pk(&e["reserve_y"]));
        assert_eq!(p.volatility_accumulator as i64, n(&e["volatility_accumulator"]));
        assert!(p.enabled());
        // preset 262: base factor 8,000 at bin step 50 = 0.4 %, fees in Y
        assert_eq!((p.base_factor, p.filter_period, p.decay_period, p.reduction_factor), (8_000, 120, 1_200, 5_000));
        assert_eq!((p.variable_fee_control, p.max_volatility_accumulator), (10_000, 250_000));
        assert_eq!(p.base_fee_rate(), 4_000_000);
        // the watch's 4-byte slice
        assert_eq!(dlmm::active_id_of_slice(&d[dlmm::ACTIVE_ID_OFFSET..dlmm::ACTIVE_ID_OFFSET + 4]), Some(p.active_id));
    }
    // a foreign owner, a short account, a wrong discriminator: errors, not panics
    let (o, d, _) = account("lb_pair_open");
    assert!(dlmm::LbPair::decode(&solix2::id(solix2::DAMM_PROGRAM), &d).is_err());
    assert!(dlmm::LbPair::decode(&o, &d[..903]).is_err());
    let mut bad = d.clone();
    bad[0] ^= 1;
    assert!(dlmm::LbPair::decode(&o, &bad).is_err());
    assert_eq!(dlmm::active_id_of_slice(&[1, 2]), None);
}

#[test]
fn position_and_bin_arrays_decode_and_the_amounts_are_what_the_chain_paid() {
    let f = flow();
    let k = &f["keys"];
    let (po, pd, pe) = account("position_before_wind_down");
    let pos = dlmm::PositionV2::decode(&po, &pd).unwrap();
    assert_eq!((pos.lower as i64, pos.upper as i64), (n(&pe["lower"]), n(&pe["upper"])));
    assert_eq!(pos.lb_pair, pk(&pe["lb_pair"]));
    assert_eq!(pos.owner, pk(&k["k_desk"]));
    assert_eq!(pos.width(), 39);
    assert!(!pos.is_empty());
    let (bo, bd, _) = account("bin_array_before_wind_down_8");
    let arr = dlmm::BinArray::decode(&bo, &bd).unwrap();
    assert_eq!((arr.index, arr.bins.len()), (8, 70));
    assert_eq!(arr.lb_pair, pk(&k["pair"]));
    let bins = dlmm::bins_of(&[arr.clone()]);
    // DLMM's own bin prices are the canister's `price_q64` (Q64.64, to within the last few units of a 2^-64 fraction)
    for (id, b) in &bins {
        let mine = dlmm::price_q64(dlmm::DESK_BIN_STEP, *id).unwrap();
        let diff = mine.abs_diff(b.price);
        assert!(diff <= b.price / 1_000_000_000_000, "bin {id}: mine {mine}, DLMM's {}", b.price);
    }
    // what the position holds = what remove_liquidity_by_range2 100 % paid (exact: per bin, rounded down as the program pays)
    let rm = f["ixs"].as_array().unwrap().iter().filter(|j| j["name"] == "dlmm.remove_liquidity_by_range2").last().unwrap();
    let (x, y) = dlmm::position_amounts(&pos, &bins).unwrap();
    assert_eq!((x, y), (nu(&rm["params"]["paid_x"]), nu(&rm["params"]["paid_y"])));
    assert!(x > 0 && y > 0);
    // a bin missing from the arrays given is an error, never an understated inventory
    assert!(dlmm::position_amounts(&pos, &Default::default()).unwrap_err().contains("no bin array"));

    // what the position can claim = what claim_fee2 paid (SOL only on this preset)
    let (_, pd2, _) = account("position_before_claim");
    let pos2 = dlmm::PositionV2::decode(&po, &pd2).unwrap();
    let (_, bd2, _) = account("bin_array_before_claim_8");
    let bins2 = dlmm::bins_of(&[dlmm::BinArray::decode(&bo, &bd2).unwrap()]);
    let claim = f["ixs"].as_array().unwrap().iter().find(|j| j["name"] == "dlmm.claim_fee2").unwrap();
    let (fx, fy) = dlmm::claimable_fees(&pos2, &bins2).unwrap();
    assert_eq!((fx, fy), (nu(&claim["params"]["paid_x"]), nu(&claim["params"]["paid_y"])));
    assert_eq!(fx, 0);
    assert!(fy > 0);
    // a position extended past 70 bins is a longer account with another layout: refused with its length, never misread (the desk's
    // band is at most 70 bins: `pooldesk` enforces it)
    let (_, pd3, e3) = account("position_extended");
    assert_eq!((n(&e3["upper"]) - n(&e3["lower"]) + 1) as usize, 81);
    assert!(dlmm::PositionV2::decode(&po, &pd3).unwrap_err().contains("bytes, not"));
}

#[test]
fn the_bin_walk_quotes_the_first_buy_the_chain_filled() {
    let f = flow();
    let (o, d, _) = account("lb_pair_after_add");
    let pair = dlmm::LbPair::decode(&o, &d).unwrap();
    let (bo, bd, _) = account("bin_array_8");
    let bins = dlmm::bins_of(&[dlmm::BinArray::decode(&bo, &bd).unwrap()]);
    let buy = f["ixs"].as_array().unwrap().iter().find(|j| j["name"] == "dlmm.swap2").unwrap();
    let amount_in = nu(&buy["params"]["amount_in"]);
    let got = nu(&buy["params"]["got"]);
    // the pool as it stood before the swap: volatility accumulator 0, last update at 0
    let q = dlmm::quote_swap(&pair, &bins, false, amount_in, 1_000_000).unwrap();
    assert!(q.complete);
    // the quote is within 0.05 % of the chain's fill (the variable fee's per-bin accounting is the one approximated piece)
    let diff = q.amount_out.abs_diff(got);
    assert!(diff * 2_000 <= got, "quoted {}, the chain filled {got}", q.amount_out);
    assert_eq!(q.amount_in_used, amount_in);
    assert!(q.end_id > pair.active_id, "a buy walks up from the active bin");
    // a sell walks down and has nothing to take from where only bids lie... it takes the bid bins
    let sell = dlmm::quote_swap(&pair, &bins, true, got, 1_000_000).unwrap();
    assert!(sell.amount_out > 0 && sell.end_id <= pair.active_id);
    // more than the arrays show: partial, flagged
    let big = dlmm::quote_swap(&pair, &bins, false, 100 * 1_000_000_000, 1_000_000).unwrap();
    assert!(!big.complete && big.amount_in_used < 100 * 1_000_000_000);
    // the fee on a 0.5 SOL buy is about 0.4 %
    assert!(q.fee > amount_in * 35 / 10_000 && q.fee < amount_in * 50 / 10_000, "{}", q.fee);
}
