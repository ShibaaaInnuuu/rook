//! The binding, proved on every saved quote (fetched keyless from https://api.relay.link/quote/v2 on 2026-10-09).
mod common;

use common::*;
use rook_relay_sol_spike::*;
use serde_json::Value;

const TR: &str = "bc1p49upe77txy03ysp4m3r4m992tf27d2xgce7yk6w53qg5jtc85exqtkpg3t";
const TR_ACTIVE: &str = "bc1pl9pvj3t5h6tyajacy2zvx9hn33rtkpn69n9n34876dlngsqurw4s9zdlfw";
const OTHER_SOL: &str = "2c3V2L5wBGKKe4Uetcx8uAr6zW2e5fKM2hrSsHgbbvBM";

fn order(q: &Value) -> &Value {
    &q["protocol"]["v2"]["orderData"]
}
fn quoted_id(q: &Value) -> [u8; 32] {
    hex::decode(q["protocol"]["v2"]["orderId"].as_str().unwrap().trim_start_matches("0x")).unwrap().try_into().unwrap()
}
fn dest(req: &Value) -> (&'static str, &'static str) {
    match req["destinationChainId"].as_u64().unwrap() {
        8_253_038 => ("bitcoin", BTC_NATIVE),
        1 => ("ethereum", EVM_NATIVE),
        c => panic!("destination {c}"),
    }
}
fn is_sol_origin(req: &Value) -> bool {
    req["originChainId"].as_u64() == Some(RELAY_SOLANA)
}

/// The canister's reads, from the saved mainnet accounts.
struct Reads {
    allocator: String,
    authority: Option<String>,
    slot: u64,
}
fn reads() -> Reads {
    let (_, allocator, _, _) = decode_relay_depository(&account_data("depository-account.json")).unwrap();
    let (slot, auth) = decode_programdata_head(&account_data("programdata-head.json")).unwrap();
    Reads { allocator: b58_encode(&allocator), authority: auth.map(|a| b58_encode(&a)), slot }
}

/// `min_out_floor`: a reference rate equal to Relay's own expected amount, less a 300 bps fee bound (the default today).
fn floor(q: &Value) -> u128 {
    let e: u128 = order(q)["output"]["payments"][0]["expectedAmount"].as_str().unwrap().parse().unwrap();
    e * 9_700 / 10_000
}

fn judge_fixture(body: &[u8], req: &Value, at: u64, r: &Reads, used: &[[u8; 32]]) -> Result<SolDeposit, String> {
    let q: Value = serde_json::from_slice(body).unwrap();
    let (dest_chain, dest_currency) = dest(req);
    let w = SolWant { our_sol: req["user"].as_str().unwrap(), dest_chain, dest_recipient: req["recipient"].as_str().unwrap(), dest_currency,
        lamports: req["amount"].as_str().unwrap().parse().unwrap(), min_out_floor: floor(&q), allocator_read: &r.allocator,
        upgrade_authority_read: r.authority.as_deref(), deploy_slot_read: r.slot, now: at, handed_in_at: at, used };
    judge_sol(body, &w)
}

#[test]
fn every_solana_origin_quote_binds_its_instruction_id_to_the_order_hash() {
    let mut n = 0;
    for (name, body, req, _) in quotes().into_iter().filter(|(_, _, r, _)| is_sol_origin(r)) {
        let q: Value = serde_json::from_slice(&body).unwrap();
        let d = &q["steps"][0]["items"][0]["data"];
        let ixs = d["instructions"].as_array().unwrap();
        assert_eq!(ixs.len(), 1, "{name}: one instruction");
        let (program, metas, data) = quote_instruction(&ixs[0], "ix").unwrap();
        assert_eq!(b58_encode(&program), DEPOSITORY_PROGRAM, "{name}");
        assert_eq!(data.len(), 48, "{name}: disc 8 + amount 8 + id 32");
        let (amount, id) = decode_deposit_native(&data).unwrap();
        // the binding: the instruction's id IS hashStruct(Order) of orderData, and Relay's own orderId
        assert_eq!(order_id(order(&q)).unwrap(), id, "{name}: the id is the EIP-712 order hash");
        assert_eq!(quoted_id(&q), id, "{name}: protocol.v2.orderId");
        assert_eq!(encode_deposit_native(amount, &id), data, "{name}: round trip");
        // the amount is the asked lamports and the order's payment
        assert_eq!(amount.to_string(), req["amount"].as_str().unwrap(), "{name}");
        assert_eq!(order(&q)["inputs"][0]["payment"]["amount"].as_str().unwrap(), amount.to_string(), "{name}");
        // the accounts: (relay_depository, sender = us signer, depositor = us, vault, system)
        assert_eq!(metas, deposit_native_metas(&pubkey(req["user"].as_str().unwrap()).unwrap()), "{name}");
        // the one lookup table named is the pinned one (and is not needed)
        assert_eq!(d["addressLookupTableAddresses"], serde_json::json!([QUOTE_LUT]), "{name}");
        n += 1;
    }
    assert_eq!(n, 9, "nine Solana-origin quotes saved");
}

#[test]
fn every_solana_origin_quote_passes_the_judge() {
    let r = reads();
    for (name, body, req, at) in quotes().into_iter().filter(|(_, _, r, _)| is_sol_origin(r)) {
        let d = judge_fixture(&body, &req, at, &r, &[]).unwrap_or_else(|e| panic!("{name}: {e}"));
        // Relay's deadline is the quote time + 7 days (`at` is when the answer was saved: up to a few seconds later)
        assert!(d.deadline <= at + 7 * 86_400 && d.deadline + 5 >= at + 7 * 86_400, "{name}: deadline {}", d.deadline);
    }
}

#[test]
fn the_evm_and_bitcoin_origin_quotes_hash_the_same_way_with_solana_addresses_in_them() {
    for (name, body, req, _) in quotes().into_iter().filter(|(_, _, r, _)| !is_sol_origin(r)) {
        let q: Value = serde_json::from_slice(&body).unwrap();
        let id = order_id(order(&q)).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(id, quoted_id(&q), "{name}: orderId");
        let data = &q["steps"][0]["items"][0]["data"];
        match req["originChainId"].as_u64().unwrap() {
            1 | 4663 => {
                // depositNative(address depositor, bytes32 id)
                let cd = hex::decode(data["data"].as_str().unwrap().trim_start_matches("0x")).unwrap();
                assert_eq!(cd[..4], [0x49, 0x29, 0x0c, 0x1c], "{name}: depositNative");
                assert_eq!(cd[36..68], id, "{name}: the EVM calldata id is the same hash");
            }
            8_253_038 => {
                // Bitcoin origin: the PSBT's OP_RETURN (or the deposit address flow) carries "0x<order id>" as text
                if let Some(psbt) = data["psbt"].as_str() {
                    let needle = hex::encode(format!("0x{}", hex::encode(id)));
                    assert!(psbt.contains(&needle), "{name}: the PSBT's OP_RETURN names the order");
                }
            }
            c => panic!("{name}: origin {c}"),
        }
    }
}

// ---- refusals: one per rule, each from a real quote with one thing changed -------------------------------------------------------

fn base() -> (Value, Value, u64) {
    let (_, body, req, at) = quotes().into_iter().find(|(n, ..)| n == "sol-btc-5").unwrap();
    (serde_json::from_slice(&body).unwrap(), req, at)
}
fn run(q: &Value, req: &Value, at: u64) -> Result<SolDeposit, String> {
    judge_fixture(&serde_json::to_vec(q).unwrap(), req, at, &reads(), &[])
}
fn ix(q: &mut Value) -> &mut Value {
    &mut q["steps"][0]["items"][0]["data"]["instructions"][0]
}
/// Re-derive the id after editing the order, the way a substituting relayer would, so the later rules are the ones exercised.
fn rehash(q: &mut Value) {
    let id = order_id(order(q)).unwrap();
    q["protocol"]["v2"]["orderId"] = Value::String(format!("0x{}", hex::encode(id)));
    let data = hex::decode(ix(q)["data"].as_str().unwrap()).unwrap();
    let (amount, _) = decode_deposit_native(&data).unwrap();
    ix(q)["data"] = Value::String(hex::encode(encode_deposit_native(amount, &id)));
}
fn refused(r: Result<SolDeposit, String>, rule: &str) {
    let e = r.expect_err("must refuse");
    assert!(e.contains(rule), "wanted {rule}, got: {e}");
}

#[test]
fn a_changed_recipient_breaks_the_hash_s6() {
    let (mut q, req, at) = base();
    q["protocol"]["v2"]["orderData"]["output"]["payments"][0]["recipient"] = TR_ACTIVE.into();
    refused(run(&q, &req, at), "rule S6");
}

#[test]
fn a_substituted_order_rehashed_end_to_end_is_caught_by_the_content_rules() {
    let (mut q, req, at) = base();
    q["protocol"]["v2"]["orderData"]["output"]["payments"][0]["recipient"] = TR_ACTIVE.into();
    rehash(&mut q);
    refused(run(&q, &req, at), "rule S9");

    let (mut q, req, at) = base();
    q["protocol"]["v2"]["orderData"]["inputs"][0]["refunds"][0]["recipient"] = OTHER_SOL.into();
    rehash(&mut q);
    refused(run(&q, &req, at), "rule S8");

    let (mut q, req, at) = base();
    q["protocol"]["v2"]["orderData"]["output"]["payments"][0]["minimumAmount"] = "1".into();
    rehash(&mut q);
    refused(run(&q, &req, at), "rule S10");

    let (mut q, req, at) = base();
    q["protocol"]["v2"]["orderData"]["output"]["deadline"] = (at + 30 * 86_400).into();
    rehash(&mut q);
    refused(run(&q, &req, at), "rule S11");

    let (mut q, req, at) = base();
    q["protocol"]["v2"]["orderData"]["output"]["calls"] = serde_json::json!(["0x00"]);
    rehash(&mut q);
    refused(run(&q, &req, at), "rule S9");
}

#[test]
fn foreign_accounts_program_or_amount_are_refused() {
    let (mut q, req, at) = base();
    ix(&mut q)["keys"][2]["pubkey"] = OTHER_SOL.into(); // the depositor credited
    refused(run(&q, &req, at), "rule S5");

    let (mut q, req, at) = base();
    ix(&mut q)["keys"][3]["pubkey"] = OTHER_SOL.into(); // the vault
    refused(run(&q, &req, at), "rule S5");

    let (mut q, req, at) = base();
    ix(&mut q)["keys"][2]["isSigner"] = true.into();
    refused(run(&q, &req, at), "rule S5");

    let (mut q, req, at) = base();
    ix(&mut q)["programId"] = OTHER_SOL.into();
    refused(run(&q, &req, at), "rule S3");

    let (q, mut req, at) = base();
    req["amount"] = "4000000000".into();
    refused(run(&q, &req, at), "rule S4");

    let (mut q, req, at) = base();
    let extra = ix(&mut q).clone();
    q["steps"][0]["items"][0]["data"]["instructions"].as_array_mut().unwrap().push(extra);
    refused(run(&q, &req, at), "rule S1");

    let (mut q, req, at) = base();
    let mut data = hex::decode(ix(&mut q)["data"].as_str().unwrap()).unwrap();
    data.push(0);
    ix(&mut q)["data"] = hex::encode(data).into();
    refused(run(&q, &req, at), "rule S5");
}

#[test]
fn a_rotated_allocator_an_upgraded_program_a_stale_or_replayed_quote_are_refused() {
    let (q, req, at) = base();
    let body = serde_json::to_vec(&q).unwrap();
    let mut r = reads();
    r.allocator = OTHER_SOL.into();
    refused(judge_fixture(&body, &req, at, &r, &[]), "rule S3");
    let mut r = reads();
    r.slot += 1;
    refused(judge_fixture(&body, &req, at, &r, &[]), "rule S3");
    let id = quoted_id(&q);
    refused(judge_fixture(&body, &req, at, &reads(), &[id]), "rule S12");
    let w_stale = |now: u64| {
        let r = reads();
        let w = SolWant { our_sol: req["user"].as_str().unwrap(), dest_chain: "bitcoin", dest_recipient: TR, dest_currency: BTC_NATIVE,
            lamports: 5_000_000_000, min_out_floor: 0, allocator_read: &r.allocator, upgrade_authority_read: r.authority.as_deref(),
            deploy_slot_read: r.slot, now, handed_in_at: at, used: &[] };
        judge_sol(&body, &w)
    };
    refused(w_stale(at + 301), "rule S11");
    assert!(w_stale(at + 300).is_ok());
}

#[test]
fn the_error_answers_saved() {
    // what each refusal from Relay means (docs §2)
    let e = |n: &str| json(dir("errors").join(format!("{n}.json")));
    assert_eq!(e("btc-sol-0.01-no-utxos")["errorCode"], "INSUFFICIENT_FUNDS");
    assert_eq!(e("btc-sol-0.01-no-utxos")["message"], "No UTXOs available to spend");
    assert_eq!(e("sol-btc-2000")["errorCode"], "INSUFFICIENT_FUNDS");
    assert_eq!(e("sol-btc-2000")["message"], "Insufficient funds");
    assert_eq!(e("sol-btc-swapped-user")["errorCode"], "INVALID_ADDRESS");
    assert_eq!(e("sol-btc-btcorigin-mixup")["errorCode"], "INSUFFICIENT_LIQUIDITY");
}
