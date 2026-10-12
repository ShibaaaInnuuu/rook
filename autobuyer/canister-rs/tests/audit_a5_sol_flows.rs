//! Audit A5 (docs/v9/audit-2026-10/a5-canister-sol-flows.md): proofs of concept for the canister's flows on a Solana home. Each test
//! asserts the behaviour the finding says is CORRECT, so it FAILS on the audited baseline (f2c24e0) and passes once the finding is
//! fixed. Run alone: `cargo test --locked --test audit_a5_sol_flows`.
use etch_autobuyer::ethflow::{self as ef, BPhase, EthBuy, EthState, LateOrder, PayoutTx, Watch};
use etch_autobuyer::relayconv::{self as rc, ConvAsk};
use etch_autobuyer::relayeth::{self as re, SolDest, Want, ETHEREUM_TO_SOLANA};
use etch_autobuyer::relaysol;
use etch_autobuyer::soltx;
use serde_json::{json, Value};

const E: u128 = 1_000_000_000_000_000_000;
const ME_SOL: &str = "5ecsyZA5SVEuTByKoDCUivha8uzAgeQJmgFnnvokFF21";
const US: [u8; 20] = [0x9d, 0xb3, 0xfb, 0x11, 0x7e, 0xcf, 0xc8, 0xfc, 0x14, 0xca, 0x26, 0x3e, 0x40, 0x1a, 0x4c, 0x57, 0xa6, 0x91, 0x3d, 0x81];
const ORDER: [u8; 32] = [0x5d; 32];
const REQUEST: [u8; 32] = [0x17; 32];

fn watch(value: u128, min_out: u128) -> Watch {
    Watch { order_id: ORDER, value, min_out, deadline: 1_800_000_000, balance_after: 0, eth_before: 0, eth_out_before: 0, pin: Some(100),
        solver: None, request_id: Some(REQUEST), home_pin: Some(100) }
}

// ---- A5-01: a written-off order's late payout is judged in the wrong currency on the Solana home ------------------------------------

/// A forward order of the Solana home sends LAMPORTS (`home_sol::fund`: `Watch.value = d.lamports`) and is paid out in WEI on Ethereum.
/// Written off (`eth_give_back` on a `Stuck` buy), its late fill on Ethereum is handed in (`eth_credit_payout(buy, 1, tx, 0)`) and
/// judged by `judge_payout`, which compares the wei paid with the lamports sent: a genuine fill of 0.87 ETH for 20 SOL is refused as
/// "more than the order sent", for ever.
#[test]
fn a5_01_a_written_off_forward_orders_late_ethereum_fill_is_credited() {
    // the real sol-eth-20 order: 20 SOL in, at least 0.859 ETH out (tests/ethnft_sol.rs `to_arrival`)
    let o = LateOrder { watch: watch(20_000_000_000, 859_035_589_742_793_043), back: false, let_go_at: 0, credited: None, released_at: None };
    // Relay's solver fills on Ethereum: a plain transfer to our address naming the request id in its input, after the deposit
    let mut input = vec![0u8; 4];
    input.extend(REQUEST);
    let t = PayoutTx { from: [0xf7; 20], to: US, value: 870 * E / 1_000, input, block: 200, success: true };
    let r = ef::judge_payout(&o, &t, &[], &[1; 32], false, &US, 0, &[], true);
    assert_eq!(r, Ok(870 * E / 1_000), "a genuine late fill of the forward order must be creditable: {r:?}");
}

/// The way back of the Solana home sends WEI on Ethereum (`Watch.value = d.value`) and is paid out in LAMPORTS on Solana. Written off
/// (`eth_give_back` on a late `BackBridging` buy), its late fill on Solana is handed in (`eth_credit_payout(buy, 792703809, sig, 0)`)
/// and judged by `judge_sol_payout`, which compares the lamports paid with the wei sent: a genuine fill of 11.07 SOL for 0.5 ETH is
/// refused as "under half the order", for ever.
#[test]
fn a5_01_a_written_off_way_backs_late_solana_fill_is_credited() {
    // the real eth-sol-0.5 order: 0.5 ETH in, at least 11.0685 SOL out
    let o = LateOrder { watch: watch(E / 2, 11_068_534_059), back: true, let_go_at: 0, credited: None, released_at: None };
    let paid: u64 = 11_294_422_509;
    let mut data = 2u32.to_le_bytes().to_vec();
    data.extend(paid.to_le_bytes());
    let solver = relaysol::SOLVER_2026_10;
    let tx = json!({
        "transaction": { "message": { "accountKeys": [solver, ME_SOL, relaysol::SYSTEM_PROGRAM, relaysol::MEMO_V2],
            "instructions": [
                { "programIdIndex": 2, "accounts": [0, 1], "data": soltx::b58_encode(&data) },
                { "programIdIndex": 3, "accounts": [], "data": soltx::b58_encode(format!("0x{}", hex::encode(ORDER)).as_bytes()) },
            ] } },
        "meta": { "err": null, "preBalances": [9_000_000_000_000u64, 1_000_000_000u64, 1, 1],
            "postBalances": [9_000_000_000_000u64 - paid, 1_000_000_000u64 + paid, 1, 1],
            "innerInstructions": [], "loadedAddresses": { "writable": [], "readonly": [] }, "logMessages": [] },
    });
    let r = ef::judge_sol_payout(&o, &tx, &[2; 32], ME_SOL, 0, &[]);
    assert_eq!(r, Ok(paid as u128), "a genuine late fill of the way back must be creditable: {r:?}");
}

// ---- A5-02: the way back, waiting for its quote, does not hold the Relay leg against the frog side -----------------------------------

/// `BackFunded` (the way back's ask is out, its deposit signed as soon as a quote is handed in, up to `BACK_QUOTE_SECS` later) is not
/// `in_relay`, so `relay_busy()` (what the frog side reads as `eth_relay_busy`, B-L-6) is false: a SOL → BTC conversion advances and
/// deposits meanwhile, and the way back's deposit (`eth_act` step 3 checks only the Ethereum side's own buys) is then signed while the
/// frog's refund watch reads the same Solana balance: Relay's ETH → SOL payout reads as the frog order's refund (and as the way back's
/// arrival), the same lamports spent twice.
#[test]
fn a5_02_a_buy_waiting_for_its_way_back_quote_holds_the_relay_leg() {
    let now = 1_800_000_000;
    let ask = ConvAsk { amount_wei: E / 2, user: format!("0x{}", hex::encode(US)), recipient: ME_SOL.into(), refund_to: format!("0x{}", hex::encode(US)),
        origin_chain_id: 1, destination_chain_id: relaysol::RELAY_SOLANA, origin_currency: rc::EVM_NATIVE.into(), destination_currency: relaysol::SOL_NATIVE.into() };
    let b = EthBuy { id: 0, collection: [0x5a; 20], module: [1; 20], wrapper: [2; 20], advanced: 20_002_000_000, attempts: 1,
        phase: BPhase::BackFunded { wei: E / 2 + 3_000_000_000_000_000, since: now }, orders: vec![ORDER], deposits: vec![], ask: Some(ask),
        last: None, note: String::new(), sweep: None, arrived_at: Some(now - 48 * 3_600), back_at: None, ended_at: None, late: None };
    let st = EthState { buys: vec![b], ..EthState::default() };
    assert!(st.relay_busy(), "a way back whose Relay deposit may be signed at any moment must hold the one Relay leg (B-L-6)");
}

// ---- A5-04: the Ethereum → Solana judge truncates the order's deadlines (C-07 not carried to relayeth) ----------------------------------

/// `relayconv::judge` and `relaysol::judge_sol` refuse a deadline over a uint32 (conversion review C-07: "refused, never truncated");
/// `relayeth::judge_with` (the way back to Solana, `judge_to_solana`) reads `uint_of(..) as u64`: a deadline of 2^64 + t is judged as
/// t (inside the band) while the order id commits to 2^64 + t.
#[test]
fn a5_04_judge_to_solana_refuses_a_deadline_over_a_uint32() {
    const ETH_SOL: &str = include_str!("fixtures/relay-sol/quotes/eth-sol-0.5.quote.json");
    let now = 1_791_565_570;
    let mut v: Value = serde_json::from_str(ETH_SOL).unwrap();
    let dl = v["protocol"]["v2"]["orderData"]["output"]["deadline"].as_u64().unwrap() as u128;
    v["protocol"]["v2"]["orderData"]["output"]["deadline"] = ((1u128 << 64) + dl).to_string().into();
    let id = rc::order_id(&v["protocol"]["v2"]["orderData"]).unwrap();
    let d = v["steps"][0]["items"][0]["data"]["data"].as_str().unwrap().to_string();
    v["steps"][0]["items"][0]["data"]["data"] = format!("{}{}", &d[..2 + 72], hex::encode(id)).into();
    v["protocol"]["v2"]["orderId"] = format!("0x{}", hex::encode(id)).into();
    let w = Want { leg: ETHEREUM_TO_SOLANA, ours: US, amount_wei: E / 2, max_fee_bps: 300, allocator: rc::RELAY_ALLOCATOR_4663, now,
        handed_in_at: now, used: &[] };
    let r = re::judge_to_solana(&serde_json::to_vec(&v).unwrap(), &w, &SolDest { recipient: ME_SOL, min_out_floor: 11_000_000_000 });
    assert!(r.as_ref().is_err_and(|e| e.contains("rule 11")), "a deadline of 2^64 + t is not t: {r:?}");
}

// ---- the fixes' own bounds ----------------------------------------------------------------------------------------------------------

/// A5-01: a payout on the order's destination chain is bounded by the order's `min_out` in that unit: at least half of it, at most
/// `min_out × 10⁴ / (10⁴ − relayeth::MAX_FEE_BPS)` plus the rate tolerance; a payout on its origin chain (a refund) is bounded by what it
/// sent, as before; and on the EVM home (everything wei) nothing changed.
#[test]
fn a5_01_payouts_are_bounded_in_their_own_unit_and_the_evm_home_is_unchanged() {
    let mut input = vec![0u8; 4];
    input.extend(REQUEST);
    let pay = |wei: u128| PayoutTx { from: [0xf7; 20], to: US, value: wei, input: input.clone(), block: 200, success: true };
    // the forward order's fill on Ethereum (Solana home): min_out 0.859 ETH → cap 0.859 / 0.95 × 1.03 ≈ 0.9314 ETH, floor ≈ 0.4295 ETH
    let fwd = LateOrder { watch: watch(20_000_000_000, 859_035_589_742_793_043), back: false, let_go_at: 0, credited: None, released_at: None };
    let cap = 859_035_589_742_793_043u128 * 10_000 / 9_500 * 10_300 / 10_000;
    assert_eq!(ef::judge_payout(&fwd, &pay(cap), &[], &[1; 32], false, &US, 0, &[], true), Ok(cap));
    assert!(ef::judge_payout(&fwd, &pay(cap + 1), &[], &[1; 32], false, &US, 0, &[], true).unwrap_err().contains("more than the order sent"));
    assert!(ef::judge_payout(&fwd, &pay(429_517_794_871_396_521), &[], &[1; 32], false, &US, 0, &[], true).unwrap_err().contains("under half"));
    assert!(ef::judge_payout(&fwd, &pay(20_000_000_000), &[], &[1; 32], false, &US, 0, &[], true).is_err(), "lamports-sized wei is no fill");
    // the way back's refund on Ethereum (Solana home): wei against the wei it sent, as before
    let back = LateOrder { watch: watch(E / 2, 11_068_534_059), back: true, let_go_at: 0, credited: None, released_at: None };
    assert_eq!(ef::judge_payout(&back, &pay(E / 2), &[], &[1; 32], false, &US, 0, &[], true), Ok(E / 2));
    assert!(ef::judge_payout(&back, &pay(E / 2 + 1), &[], &[1; 32], false, &US, 0, &[], true).unwrap_err()
        .contains("more than the order sent (500000000000000000)"));
    assert!(ef::judge_payout(&back, &pay(E / 4 - 1), &[], &[1; 32], false, &US, 0, &[], true).unwrap_err().contains("under half"));
    // the EVM home: a forward order of 5 ETH (min out 4.85 ETH) is bounded by the 5 ETH it sent, never by its min out
    let evm = LateOrder { watch: watch(5 * E, 485 * E / 100), back: false, let_go_at: 0, credited: None, released_at: None };
    assert_eq!(ef::judge_payout(&evm, &pay(5 * E), &[], &[1; 32], false, &US, 0, &[], false), Ok(5 * E));
    assert!(ef::judge_payout(&evm, &pay(5 * E + 1), &[], &[1; 32], false, &US, 0, &[], false).unwrap_err().contains("more than the order sent"));
    assert!(ef::judge_payout(&evm, &pay(5 * E / 2 - 1), &[], &[1; 32], false, &US, 0, &[], false).unwrap_err().contains("under half"));
}

/// A5-01: the way back's fill on Solana is bounded by its min out in lamports (the forward order's refund there stays bounded by the
/// lamports it sent: `tests/ethnft_sol.rs::sol_a_late_payout_on_solana_is_credited_by_its_memo_and_its_lamports_never_twice`).
#[test]
fn a5_01_a_way_backs_fill_on_solana_over_its_cap_or_under_half_its_min_out_is_refused() {
    let tx_of = |paid: u64| {
        let mut data = 2u32.to_le_bytes().to_vec();
        data.extend(paid.to_le_bytes());
        json!({
            "transaction": { "message": { "accountKeys": [relaysol::SOLVER_2026_10, ME_SOL, relaysol::SYSTEM_PROGRAM, relaysol::MEMO_V2],
                "instructions": [
                    { "programIdIndex": 2, "accounts": [0, 1], "data": soltx::b58_encode(&data) },
                    { "programIdIndex": 3, "accounts": [], "data": soltx::b58_encode(format!("0x{}", hex::encode(ORDER)).as_bytes()) },
                ] } },
            "meta": { "err": null, "preBalances": [9_000_000_000_000u64, 1_000_000_000u64, 1, 1],
                "postBalances": [9_000_000_000_000u64 - paid, 1_000_000_000u64 + paid, 1, 1],
                "innerInstructions": [], "loadedAddresses": { "writable": [], "readonly": [] }, "logMessages": [] },
        })
    };
    let o = LateOrder { watch: watch(E / 2, 11_068_534_059), back: true, let_go_at: 0, credited: None, released_at: None };
    let cap = 11_068_534_059u128 * 10_000 / 9_500 * 10_300 / 10_000;
    assert_eq!(ef::judge_sol_payout(&o, &tx_of(cap as u64), &[2; 32], ME_SOL, 0, &[]), Ok(cap));
    assert!(ef::judge_sol_payout(&o, &tx_of(cap as u64 + 1), &[2; 32], ME_SOL, 0, &[]).unwrap_err().contains("more than the order sent"));
    assert!(ef::judge_sol_payout(&o, &tx_of(5_534_267_029), &[2; 32], ME_SOL, 0, &[]).unwrap_err().contains("under half"));
}

/// A5-04: the refunds' deadlines too are uint32s, refused over one, never truncated.
#[test]
fn a5_04_a_refund_deadline_over_a_uint32_is_refused() {
    const ETH_SOL: &str = include_str!("fixtures/relay-sol/quotes/eth-sol-0.5.quote.json");
    let now = 1_791_565_570;
    let mut v: Value = serde_json::from_str(ETH_SOL).unwrap();
    let dl = v["protocol"]["v2"]["orderData"]["inputs"][0]["refunds"][0]["deadline"].as_u64().unwrap() as u128;
    v["protocol"]["v2"]["orderData"]["inputs"][0]["refunds"][0]["deadline"] = ((1u128 << 32) + dl).to_string().into();
    let id = rc::order_id(&v["protocol"]["v2"]["orderData"]).unwrap();
    let d = v["steps"][0]["items"][0]["data"]["data"].as_str().unwrap().to_string();
    v["steps"][0]["items"][0]["data"]["data"] = format!("{}{}", &d[..2 + 72], hex::encode(id)).into();
    v["protocol"]["v2"]["orderId"] = format!("0x{}", hex::encode(id)).into();
    let w = Want { leg: ETHEREUM_TO_SOLANA, ours: US, amount_wei: E / 2, max_fee_bps: 300, allocator: rc::RELAY_ALLOCATOR_4663, now,
        handed_in_at: now, used: &[] };
    let r = re::judge_to_solana(&serde_json::to_vec(&v).unwrap(), &w, &SolDest { recipient: ME_SOL, min_out_floor: 11_000_000_000 });
    assert!(r.as_ref().is_err_and(|e| e.contains("rule 11") && e.contains("over a uint32")), "{r:?}");
}

/// A5-I3: the operator's give-back on the Solana home names lamports (its keep is the home's), on the EVM home wei.
#[test]
fn a5_i3_the_give_back_names_the_homes_unit() {
    let b = |phase| EthBuy { id: 0, collection: [0x5a; 20], module: [1; 20], wrapper: [2; 20], advanced: 20_002_000_000, attempts: 1, phase,
        orders: vec![], deposits: vec![], ask: None, last: None, note: String::new(), sweep: None, arrived_at: None, back_at: None,
        ended_at: None, late: None };
    let mut st = EthState { buys: vec![b(BPhase::Funded { wei: 20_000_000_000, since: 1 })], ..EthState::default() };
    let r = ef::give_back_now_keeping(&mut st, 0, 25_000_000_000, 2, etch_autobuyer::solhome::SOL_GAS_KEEP_LAMPORTS).unwrap();
    assert!(r.contains("20000000000 lamports goes back"), "{r}");
    let mut st = EthState { buys: vec![b(BPhase::Funded { wei: E, since: 1 })], ..EthState::default() };
    let r = ef::give_back_now(&mut st, 0, 2 * E, 2).unwrap();
    assert!(r.contains(" wei goes back"), "{r}");
}
