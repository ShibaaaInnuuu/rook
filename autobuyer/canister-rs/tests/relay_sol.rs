//! Relay from a Solana origin (docs/v9/spikes/q5-relay-solana-legs.md), ported with the spike's tests from
//! `ops/rehearsal/relay-sol/tests/{binding,chain}.rs`: the binding proved on every saved quote (fetched keyless from
//! https://api.relay.link/quote/v2 on 2026-10-09; `tests/fixtures/relay-sol/quotes`), the rules S1–S12 refused one by one, and the
//! depository on mainnet (`tests/fixtures/relay-sol/chain`): its PDAs, state, `ProgramData` (S3), a real deposit, two real refunds,
//! a real `execute_transfer` and a real solver fill.
#![allow(dead_code)]
use std::path::PathBuf;

use etch_autobuyer::relayconv::order_id;
use etch_autobuyer::relaysol::*;
use etch_autobuyer::soltx::{anchor_disc, b58_encode, find_program_address, pubkey};
use serde_json::Value;

// ---- the spike's fixture helpers ----

pub fn dir(sub: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/relay-sol").join(sub)
}

pub fn json(path: PathBuf) -> Value {
    serde_json::from_slice(&std::fs::read(&path).unwrap_or_else(|e| panic!("{path:?}: {e}"))).expect("json")
}

/// Every saved quote: `(name, quote bytes, request, fetched_at)`.
pub fn quotes() -> Vec<(String, Vec<u8>, Value, u64)> {
    let manifest = json(dir("quotes").join("manifest.json"));
    let mut out = Vec::new();
    for (name, at) in manifest["fetched_at"].as_object().expect("fetched_at") {
        let body = std::fs::read(dir("quotes").join(format!("{name}.quote.json"))).expect("quote");
        let req = json(dir("quotes").join(format!("{name}.request.json")));
        out.push((name.clone(), body, req, at.as_u64().expect("u64")));
    }
    assert!(out.len() >= 14, "the manifest lists the saved quotes");
    out
}

/// A `getTransaction` / `getAccountInfo` fixture's `result`, or the bare result when saved bare.
pub fn chain(name: &str) -> Value {
    let v = json(dir("chain").join(name));
    match v.get("result") {
        Some(r) if v.get("jsonrpc").is_some() => r.clone(),
        _ => v,
    }
}

pub fn account_data(name: &str) -> Vec<u8> {
    use base64::Engine;
    let v = chain(name);
    base64::engine::general_purpose::STANDARD.decode(v["value"]["data"][0].as_str().expect("base64")).expect("b64")
}

// ---- binding.rs ----


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
    let w = SolWant { our_sol: req["user"].as_str().unwrap(), depositor: req["user"].as_str().unwrap(), dest_chain, dest_recipient: req["recipient"].as_str().unwrap(), dest_currency,
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
        let w = SolWant { our_sol: req["user"].as_str().unwrap(), depositor: req["user"].as_str().unwrap(), dest_chain: "bitcoin", dest_recipient: TR, dest_currency: BTC_NATIVE,
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

// ---- chain.rs ----


fn program() -> [u8; 32] {
    pubkey(DEPOSITORY_PROGRAM).unwrap()
}

#[test]
fn discriminators_are_anchors() {
    assert_eq!(anchor_disc("global", "deposit_native"), IX_DEPOSIT_NATIVE);
    assert_eq!(anchor_disc("global", "deposit_token"), IX_DEPOSIT_TOKEN);
    assert_eq!(anchor_disc("global", "execute_transfer"), IX_EXECUTE_TRANSFER);
    assert_eq!(anchor_disc("event", "DepositEvent"), EV_DEPOSIT);
    assert_eq!(anchor_disc("event", "TransferExecutedEvent"), EV_TRANSFER_EXECUTED);
    assert_eq!(anchor_disc("account", "RelayDepository"), ACC_RELAY_DEPOSITORY);
}

#[test]
fn base58_round_trips() {
    for s in [DEPOSITORY_PROGRAM, SYSTEM_PROGRAM, RELAY_DEPOSITORY, VAULT, ALLOCATOR] {
        assert_eq!(b58_encode(&pubkey(s).unwrap()), s);
    }
    assert_eq!(pubkey(SYSTEM_PROGRAM).unwrap(), [0u8; 32]);
    assert!(pubkey("abc").is_err());
}

#[test]
fn the_state_and_vault_are_the_programs_pdas() {
    let (state, _) = find_program_address(&[b"relay_depository"], &program()).unwrap();
    assert_eq!(b58_encode(&state), RELAY_DEPOSITORY);
    let (vault, bump) = find_program_address(&[b"vault"], &program()).unwrap();
    assert_eq!(b58_encode(&vault), VAULT);
    let (_, _, vault_bump, _) = decode_relay_depository(&account_data("depository-account.json")).unwrap();
    assert_eq!(bump, vault_bump);
}

#[test]
fn the_state_account_names_the_pinned_owner_allocator_and_domain() {
    let acc = chain("depository-account.json");
    assert_eq!(acc["value"]["owner"], DEPOSITORY_PROGRAM);
    let (owner, allocator, _, domain) = decode_relay_depository(&account_data("depository-account.json")).unwrap();
    assert_eq!(b58_encode(&owner), OWNER_AND_UPGRADE_AUTHORITY);
    assert_eq!(b58_encode(&allocator), ALLOCATOR);
    assert_eq!(hex::encode(domain.unwrap()), DOMAIN_SEPARATOR);
}

#[test]
fn the_program_is_upgradeable_by_the_owner_key() {
    let p = chain("program-account.json");
    assert_eq!(p["value"]["owner"], "BPFLoaderUpgradeab1e11111111111111111111111");
    assert_eq!(p["value"]["executable"], true);
    let d = account_data("program-account.json");
    assert_eq!(d[..4], [2, 0, 0, 0]); // UpgradeableLoaderState::Program { programdata_address }
    assert_eq!(b58_encode(&d[4..36]), PROGRAMDATA);
    let (slot, auth) = decode_programdata_head(&account_data("programdata-head.json")).unwrap();
    assert_eq!(slot, PROGRAM_DEPLOY_SLOT);
    assert_eq!(b58_encode(&auth.unwrap()), OWNER_AND_UPGRADE_AUTHORITY);
}

#[test]
fn a_real_deposit_native_its_event_and_the_vault_credit_agree() {
    let tx = chain("tx-deposit-native.json");
    let (sender, ev) = deposit_in(&tx).unwrap().unwrap();
    assert_eq!(b58_encode(&ev.depositor), sender); // sender == depositor in Relay's own flow
    assert_eq!(ev.token, None);
    assert_eq!(lamport_delta(&tx, VAULT), Some(ev.amount as i128));
    assert_eq!(lamport_delta(&tx, &sender).unwrap() + ev.amount as i128 <= 0, true);
}

/// The two SOL refunds Relay's `/requests/v2?status=refund` listed for a Solana origin on 2026-10-09 (`SLIPPAGE`): the deposit
/// went into the vault with id = order id; the refund is a plain System transfer from Relay's solver wallet to the depositor,
/// with the memo `0x<order id>`. Not an `execute_transfer`: the vault is not touched.
#[test]
fn a_real_refund_is_a_solver_transfer_memoed_with_the_order_id() {
    let reqs = chain("relay-requests-sol-refunds.json");
    for (case, deposit, refund) in [(0, "tx-refund-case-deposit.json", "tx-refund-case-refund.json"),
        (1, "tx-refund-case2-deposit.json", "tx-refund-case2-refund.json")] {
        let r = &reqs[case];
        assert_eq!(r["status"], "refund");
        let (dtx, rtx) = (chain(deposit), chain(refund));
        let (depositor, ev) = deposit_in(&dtx).unwrap().unwrap();
        assert_eq!(depositor, r["user"].as_str().unwrap());
        let got = refund_of(&rtx, &depositor, &ev.id).unwrap().expect("recognised as the refund of that order");
        // what came back: the deposit less a few hundred lamports
        assert!(got <= ev.amount && ev.amount - got < 1_000, "{} of {}", got, ev.amount);
        // the payer is the solver, not the depository; nothing of the depository's runs
        let tr = system_transfers_to(&rtx, &depositor).unwrap();
        assert_eq!(tr.len(), 1);
        assert_eq!(tr[0].0, SOLVER_2026_10);
        assert!(tx_instructions(&rtx).unwrap().iter().all(|i| i.program != DEPOSITORY_PROGRAM));
        assert_eq!(lamport_delta(&rtx, VAULT), None);
        // a memo with another id, or a credit to someone else, is not this refund
        let mut other = ev.id;
        other[0] ^= 1;
        assert_eq!(refund_of(&rtx, &depositor, &other).unwrap(), None);
        assert_eq!(refund_of(&rtx, SOLVER_2026_10, &ev.id).unwrap(), None);
        // the refund comes in the same minute as the deposit
        assert!(rtx["blockTime"].as_i64().unwrap() - dtx["blockTime"].as_i64().unwrap() < 120);
    }
}

/// `execute_transfer` is the allocator-signed release from the vault. The one saved is the solver withdrawing a token deposit
/// to itself (recipient == executor == the solver): Relay's settlement of deposits, not a payout to a user and not tied to any
/// order id (the request carries none).
#[test]
fn a_real_execute_transfer_is_the_solver_settling_and_names_no_order() {
    let tx = chain("tx-execute-transfer.json");
    let ixs = tx_instructions(&tx).unwrap();
    let ix = ixs.iter().find(|i| i.program == DEPOSITORY_PROGRAM).unwrap();
    let req = decode_execute_transfer(&ix.data).unwrap();
    assert_eq!(hex::encode(req.domain), DOMAIN_SEPARATOR);
    assert_eq!(b58_encode(&req.vault_address), VAULT);
    assert_eq!(b58_encode(&req.recipient), SOLVER_2026_10);
    assert_eq!(ix.accounts[1], SOLVER_2026_10); // executor
    assert_eq!(ix.accounts[2], SOLVER_2026_10); // recipient
    assert!(req.token.is_some());
    assert_eq!(encode_transfer_request(&req), ix.data[8..].to_vec());
    // the allocator's signature rides in an Ed25519SigVerify instruction just before it, over sha256(borsh(request))
    let sig = ixs.iter().find(|i| i.program == "Ed25519SigVerify111111111111111111111111111").unwrap();
    let msg_hash: [u8; 32] = <sha2::Sha256 as sha2::Digest>::digest(encode_transfer_request(&req)).into();
    let off = |i: usize| u16::from_le_bytes([sig.data[i], sig.data[i + 1]]) as usize;
    let (pk_at, msg_at, msg_len) = (off(6), off(10), off(12));
    assert_eq!(b58_encode(&sig.data[pk_at..pk_at + 32]), ALLOCATOR);
    assert_eq!(sig.data[msg_at..msg_at + msg_len], msg_hash);
    // the event repeats the request; its id is the used_request account, PDA ["used_request", sha256(borsh(request))]
    let evs: Vec<_> = program_data(&tx, DEPOSITORY_PROGRAM).iter().filter_map(|b| decode_transfer_executed(b)).collect();
    assert_eq!(evs.len(), 1);
    let (ereq, executor, id) = &evs[0];
    assert_eq!(*ereq, req);
    assert_eq!(b58_encode(executor), SOLVER_2026_10);
    assert_eq!(b58_encode(id), ix.accounts[7]);
    let (pda, _) = find_program_address(&[b"used_request", &msg_hash], &program()).unwrap();
    assert_eq!(b58_encode(&pda), ix.accounts[7]);
}

/// A fill on Solana: plain transfers from the solver with two memos, the order id and Relay's request id (0x1791… prefix).
#[test]
fn a_real_solver_fill_carries_order_and_request_memos() {
    let tx = chain("tx-solver-fill-memo.json");
    let m = memos(&tx).unwrap();
    assert_eq!(m.len(), 2);
    assert!(m.iter().all(|x| x.len() == 66 && x.starts_with("0x")));
    assert!(m[1].starts_with("0x1791"));
    // Relay's own record of that request (/requests/v2?id=<the second memo>): a 4663 → Solana fill whose orderId is the first memo
    let r = chain("relay-request-solver-fill.json");
    assert_eq!(r["id"].as_str().unwrap(), m[1]);
    assert_eq!(r["data"]["outTxs"][0]["chainId"], RELAY_SOLANA);
    assert_eq!(r["data"]["inTxs"][0]["chainId"], 4663);
    let order_id = serde_json::to_string(&r).unwrap().split("\"orderId\":\"").nth(1).unwrap()[..66].to_string();
    assert_eq!(order_id, m[0]);
    // the payee is credited with SPL tokens here (a token fill); a SOL fill is the same shape with a System transfer
    assert_eq!(tx_instructions(&tx).unwrap().iter().filter(|i| i.program == "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA").count(), 1);
}

// ---- the canister's side (new with the port) ----

/// The canister writes its own message from the judged deposit (never the quote's table): durable-nonce prefix, compute budget,
/// `deposit_native`: one signer, well under 1,232 bytes, and the same instruction the quote carried.
#[test]
fn the_canister_signs_its_own_deposit_message_from_the_judged_values() {
    use etch_autobuyer::soltx;
    let (_, body, req, at) = quotes().into_iter().find(|(n, ..)| n == "sol-btc-5").unwrap();
    let d = judge_fixture(&body, &req, at, &reads(), &[]).unwrap();
    let ours = pubkey(req["user"].as_str().unwrap()).unwrap();
    let ix = deposit_instruction(&ours, &d);
    assert_eq!((ix.program, ix.accounts.clone(), ix.data.clone()), (d.program, d.metas.clone(), d.data.clone()));
    let all = soltx::with_nonce([0x33; 32], ours, &[soltx::compute_unit_limit(60_000), soltx::compute_unit_price(1_000), ix]);
    let msg = soltx::compile(&all, ours, [1; 32]);
    assert_eq!(msg.signers, vec![ours]);
    assert!(soltx::wire_size(&msg) < 500, "{} bytes (Q5 §2: about 450)", soltx::wire_size(&msg));
}

/// S3 reads straight from the two finalized accounts, owners checked.
#[test]
fn s3_reads_come_from_the_owned_accounts() {
    use etch_autobuyer::solrpc::Account;
    use etch_autobuyer::soltx;
    let state = Account { lamports: 1, owner: soltx::key(DEPOSITORY_PROGRAM), executable: false, data: account_data("depository-account.json") };
    let pd = Account { lamports: 1, owner: soltx::key(BPF_LOADER_UPGRADEABLE), executable: false, data: account_data("programdata-head.json") };
    let (alloc, auth, slot) = s3_reads(&state, &pd).unwrap();
    assert_eq!((alloc.as_str(), auth.as_deref(), slot), (ALLOCATOR, Some(OWNER_AND_UPGRADE_AUTHORITY), PROGRAM_DEPLOY_SLOT));
    let forged = Account { owner: [9; 32], ..state.clone() };
    assert!(s3_reads(&forged, &pd).unwrap_err().contains("rule S3"));
    let forged_pd = Account { owner: [9; 32], ..pd };
    assert!(s3_reads(&state, &forged_pd).unwrap_err().contains("rule S3"));
}

/// `vm_of` knows "solana" in the 4663 judge too: Solana addresses are their 32 raw bytes; anything else on "solana" is refused.
#[test]
fn relayconv_encodes_solana_addresses_as_32_raw_bytes() {
    use etch_autobuyer::relayconv::encode_address;
    assert_eq!(encode_address(SOLVER_2026_10, "solana").unwrap(), pubkey(SOLVER_2026_10).unwrap().to_vec());
    assert_eq!(encode_address(SOL_NATIVE, "solana").unwrap(), vec![0u8; 32]);
    assert!(encode_address("0x0000000000000000000000000000000000000000", "solana").is_err());
    assert!(encode_address("abc", "solana").is_err());
}
