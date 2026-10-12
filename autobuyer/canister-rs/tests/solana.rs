//! The Solana foundation, natively (docs/v9/solana-migration.md §2.4, §4):
//! - `soltx`: the legacy bytes are the Solana CLI 4.3.0's (spike Q3's fixture); the v0 bytes, rebuilt from five real mainnet
//!   transactions (`tests/fixtures/relay-sol/chain`, Q5), verify under their own Ed25519 signatures; v0 compilation with a lookup
//!   table; PDAs against real ones; the 1,232-byte check; Borsh.
//! - `solrpc`: every transform on real devnet answers (Q3, 2026-10-09); agreement.
//! - `solkey`: the signing guard.
//! - `Init.home`: validation (the books home only: decision D-l), upgrades, Candid.

use base64::Engine;
use curve25519_dalek::{edwards::CompressedEdwardsY, scalar::Scalar, EdwardsPoint};
use etch_autobuyer::canister::{restore_home, validate_home, Home, Init};
use etch_autobuyer::solkey;
use etch_autobuyer::solrpc::{account_cap, agree, context, normalise, Kind, Refused};
use etch_autobuyer::soltx::*;
use serde_json::{json, Value};
use sha2::{Digest, Sha512};

fn fixture(path: &str) -> Value {
    let p = format!("{}/tests/fixtures/{path}", env!("CARGO_MANIFEST_DIR"));
    let v: Value = serde_json::from_slice(&std::fs::read(&p).unwrap_or_else(|e| panic!("{p}: {e}"))).unwrap();
    match v.get("result") {
        Some(r) if v.get("jsonrpc").is_some() => r.clone(),
        _ => v,
    }
}

fn pk(s: &str) -> Pubkey {
    pubkey(s).unwrap()
}

// ---- soltx: the spike's CLI fixture ---------------------------------------------------------------------------------------------

const A: &str = "BWbreqGmzv4nZ5nX5DvPH6pihkjsxo7dx1Bi3Dax94ic"; // a throwaway key (Q3), public half only
const N: &str = "2U7CZHN2Qh2Cymgwjk1zyjkfApqieDr6rsWKEcXBCYGh";
const TO: &str = "9xQeWvG816bUx9EPjHmaT23yvVM2ZWbrrpZb9PusVFin";
const BH: &str = "GHtXQBsoZHVnNFa9YevAzFr17DJjgHXk3ycTKD5xD3Zi";
/// `solana transfer --sign-only --blockhash BH --nonce N --nonce-authority A --from A --fee-payer A TO 0.001 --dump-transaction-message`
const CLI_MESSAGE: &str = "AQACBZwqH+bEuLg7XWKDmXUJtMdDZDMckHvQPXJ2Zuefj18pFc1g13G6Y2VbV7Mc+jtZQsYvGVRKBcX3gVUZ2kImjUKFDy1uAqR6+CTQmradxC1wyyjL+iSft+5XudJWwSdi7wAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAABqfVFxksVo7gioRfc9KXiM8DXDFFshqzRNgGLqlAAADjMtr5L6vs6LY/96RABeX9/Zr6FYdWthxalfkEs7jQgQIDAwEEAAQEAAAAAwIAAgwCAAAAQEIPAAAAAAA=";

#[test]
fn base58_round_trips_and_knows_the_system_program() {
    assert_eq!(b58_encode(&SYSTEM_PROGRAM), "11111111111111111111111111111111");
    for s in [A, N, TO, BH, SYSVAR_RENT, SYSVAR_RECENT_BLOCKHASHES, COMPUTE_BUDGET_PROGRAM, TOKEN_PROGRAM, ASSOCIATED_TOKEN_PROGRAM] {
        assert_eq!(b58_encode(&pk(s)), s);
    }
    assert!(pubkey("0OIl").is_err());
    assert!(pubkey("1111").is_err());
    let sig = "5coWyuBNnXjhXkCLhusNKyWDFXjpcmybTG36ZszofdKPKRcra1conGvcCC8wWDXfCvbbZrJW9A9hpUgsgu7r3RvA";
    assert_eq!(b58_encode(&signature(sig).unwrap()), sig);
}

#[test]
fn the_durable_nonce_transfer_is_byte_for_byte_the_clis() {
    let (a, n) = (pk(A), pk(N));
    let msg = compile(&with_nonce(n, a, &[transfer(a, pk(TO), 1_000_000)]), a, pk(BH));
    let cli = base64::engine::general_purpose::STANDARD.decode(CLI_MESSAGE).unwrap();
    assert_eq!(msg.bytes, cli);
    assert_eq!(msg.signers, vec![a]);
    assert_eq!(wire_size(&msg), 1 + 64 + cli.len());
    let tx = assemble(&msg, &[[7u8; 64]]).unwrap();
    assert_eq!((tx.len(), tx[0], &tx[65..]), (wire_size(&msg), 1, &cli[..]));
    assert!(assemble(&msg, &[]).is_err());
    // parse is the inverse, and decompiles to the instructions (flags as the header gives them)
    let p = Parts::parse(&cli).unwrap();
    assert_eq!((p.version, p.serialize()), (None, cli.clone()));
    let ixs = p.decompile(&[], &[]).unwrap();
    assert_eq!(ixs[1], transfer(a, pk(TO), 1_000_000));
    let (sigs, m) = split_tx(&tx).unwrap();
    assert_eq!((sigs, m), (vec![[7u8; 64]], &cli[..]));
}

#[test]
fn the_seed_address_is_the_clis_and_the_nonce_line_opens_with_one_signer() {
    // `solana create-address-with-seed --from A rook-q3-nonce-0 SYSTEM`
    let n = address_with_seed(&pk(A), "rook-q3-nonce-0", &SYSTEM_PROGRAM).unwrap();
    assert_eq!(b58_encode(&n), "97ncgGo96sQ18Xop3jBtydV2nwnsQJ46atKKsL2Hx9Ab");
    assert!(address_with_seed(&pk(A), &"x".repeat(33), &SYSTEM_PROGRAM).is_err());
    let a = pk(A);
    let msg = compile(&[create_with_seed(a, n, a, "rook-q3-nonce-0", 1_056_640, 80, SYSTEM_PROGRAM), initialize_nonce(n, a)], a, pk(BH));
    assert_eq!(msg.signers, vec![a]);
    assert_eq!(msg.bytes[..3], [1, 0, 3]);
    assert!(wire_size(&msg) < PACKET_DATA_SIZE);
}

#[test]
fn the_1232_byte_check_refuses_before_signing() {
    let a = pk(A);
    let ixs = |k: u8| -> Vec<Instruction> { with_nonce(pk(N), a, &(0..k).map(|i| transfer(a, [i + 1; 32], 1)).collect::<Vec<_>>()) };
    let fits = (1..60u8).take_while(|k| check_size(wire_size(&compile(&ixs(*k), a, pk(BH)))).is_ok()).last().unwrap();
    assert!(fits >= 20, "{fits} transfers fit after the advance (Q3 §3.3 measured 22 with its own keys)");
    let too_big = compile(&ixs(fits + 1), a, pk(BH));
    assert!(check_size(wire_size(&too_big)).unwrap_err().contains("1232"));
    assert!(assemble(&too_big, &[[0; 64]]).is_err());
}

#[test]
fn compact_u16_and_compute_budget() {
    for (n, want) in [(0usize, vec![0u8]), (0x7f, vec![0x7f]), (0x80, vec![0x80, 0x01]), (0x3fff, vec![0xff, 0x7f]), (0x4000, vec![0x80, 0x80, 0x01])] {
        let mut v = vec![];
        compact_u16(n, &mut v);
        assert_eq!(v, want, "{n}");
    }
    // the bytes Relay's solver sends (tx-refund-case-refund.json: `Hb6w7d`, `3Fy5gu4AD7V1`)
    assert_eq!(compute_unit_limit(200_000).data, [vec![2], 200_000u32.to_le_bytes().to_vec()].concat());
    assert_eq!(compute_unit_price(5).data, [vec![3], 5u64.to_le_bytes().to_vec()].concat());
    let refund = fixture("relay-sol/chain/tx-refund-case-refund.json");
    let ixs = &refund["transaction"]["message"]["instructions"];
    for ix in ixs.as_array().unwrap().iter().filter(|i| i["programIdIndex"] == 2) {
        let d = b58_decode(ix["data"].as_str().unwrap()).unwrap();
        let rebuilt = match d[0] {
            2 => compute_unit_limit(u32::from_le_bytes(d[1..5].try_into().unwrap())).data,
            3 => compute_unit_price(u64::from_le_bytes(d[1..9].try_into().unwrap())).data,
            t => panic!("tag {t}"),
        };
        assert_eq!(rebuilt, d);
    }
}

#[test]
fn the_nonce_account_decodes() {
    let st = NonceState { authority: pk(A), durable_nonce: pk(BH), lamports_per_signature: 5000 };
    let mut d = encode_nonce(&st);
    assert_eq!(decode_nonce(&d).unwrap(), st);
    d[4] = 0;
    assert!(decode_nonce(&d).is_err());
    assert!(decode_nonce(&d[..79]).is_err());
}

// ---- v0 against real mainnet transactions ------------------------------------------------------------------------------------------

/// RFC 8032 verification (cofactorless, as Solana's `ed25519-dalek` `verify_strict` minus the small-order checks).
fn ed25519_verify(pubkey: &Pubkey, msg: &[u8], sig: &[u8; 64]) -> bool {
    let Some(a) = CompressedEdwardsY(*pubkey).decompress() else { return false };
    let s: Option<Scalar> = Scalar::from_canonical_bytes(sig[32..].try_into().unwrap()).into();
    let Some(s) = s else { return false };
    let mut h = Sha512::new();
    h.update(&sig[..32]);
    h.update(pubkey);
    h.update(msg);
    let mut w = [0u8; 64];
    w.copy_from_slice(&h.finalize());
    let k = Scalar::from_bytes_mod_order_wide(&w);
    EdwardsPoint::vartime_double_scalar_mul_basepoint(&k, &(-a), &s).compress().to_bytes() == sig[..32]
}

/// A `getTransaction(json)` answer's message as parts.
fn parts_of(tx: &Value) -> Parts {
    let m = &tx["transaction"]["message"];
    let h = &m["header"];
    let u8_of = |v: &Value| v.as_u64().unwrap() as u8;
    Parts {
        version: Some(0),
        header: Header { required_signatures: u8_of(&h["numRequiredSignatures"]), readonly_signed: u8_of(&h["numReadonlySignedAccounts"]),
            readonly_unsigned: u8_of(&h["numReadonlyUnsignedAccounts"]) },
        keys: m["accountKeys"].as_array().unwrap().iter().map(|k| pk(k.as_str().unwrap())).collect(),
        blockhash: pk(m["recentBlockhash"].as_str().unwrap()),
        instructions: m["instructions"].as_array().unwrap().iter().map(|ix| CompiledIx {
            program: u8_of(&ix["programIdIndex"]),
            accounts: ix["accounts"].as_array().unwrap().iter().map(u8_of).collect(),
            data: b58_decode(ix["data"].as_str().unwrap()).unwrap(),
        }).collect(),
        tables: m["addressTableLookups"].as_array().unwrap().iter().map(|t| TableUse {
            table: pk(t["accountKey"].as_str().unwrap()),
            writable: t["writableIndexes"].as_array().unwrap().iter().map(u8_of).collect(),
            readonly: t["readonlyIndexes"].as_array().unwrap().iter().map(u8_of).collect(),
        }).collect(),
    }
}

const REAL_TXS: [&str; 7] = ["tx-deposit-native.json", "tx-refund-case-deposit.json", "tx-refund-case-refund.json", "tx-refund-case2-deposit.json",
    "tx-refund-case2-refund.json", "tx-execute-transfer.json", "tx-solver-fill-memo.json"];

#[test]
fn v0_messages_rebuilt_from_mainnet_transactions_verify_under_their_own_signatures() {
    for name in REAL_TXS {
        let tx = fixture(&format!("relay-sol/chain/{name}"));
        let p = parts_of(&tx);
        let bytes = p.serialize();
        assert_eq!(bytes[0], 0x80, "{name}: v0");
        let sig = signature(tx["transaction"]["signatures"][0].as_str().unwrap()).unwrap();
        assert!(ed25519_verify(&p.keys[0], &bytes, &sig), "{name}: the fee payer's signature verifies over our bytes");
        assert_eq!(Parts::parse(&bytes).unwrap(), p, "{name}: parse is the inverse");
        let mut bad = bytes.clone();
        *bad.last_mut().unwrap() ^= 1;
        assert!(!ed25519_verify(&p.keys[0], &bad, &sig), "{name}: one bit off does not verify");
        assert!(Parts::parse(&[bytes.as_slice(), &[0]].concat()).is_err(), "{name}: trailing bytes refused");
    }
}

/// v0 compilation with a lookup table, on the deposit Relay's own user sent (Q5 §5.1): the instructions decompiled from the real
/// message, compiled again against a table holding the loaded keys at the same indexes, decompile to the same instructions.
#[test]
fn v0_compilation_with_a_lookup_table_keeps_every_instruction() {
    let tx = fixture("relay-sol/chain/tx-deposit-native.json");
    let p = parts_of(&tx);
    let loaded = |k: &str| -> Vec<Pubkey> { tx["meta"]["loadedAddresses"][k].as_array().unwrap().iter().map(|x| pk(x.as_str().unwrap())).collect() };
    let (lw, lr) = (loaded("writable"), loaded("readonly"));
    let original = p.decompile(&lw, &lr).unwrap();
    // the table: the loaded keys at their real indexes, filler elsewhere
    let t = &p.tables[0];
    let mut addresses: Vec<Pubkey> = (0..16u8).map(|i| [0xf0 ^ i; 32]).collect();
    for (i, k) in t.writable.iter().zip(&lw).chain(t.readonly.iter().zip(&lr)) {
        addresses[*i as usize] = *k;
    }
    let table = LookupTable { key: t.table, addresses: addresses.clone() };
    let q = compile_v0_parts(&original, p.keys[0], p.blockhash, &[table.clone()]).unwrap();
    assert_eq!(q.tables.len(), 1);
    let (qw, qr): (Vec<Pubkey>, Vec<Pubkey>) = (q.tables[0].writable.iter().map(|i| addresses[*i as usize]).collect(),
        q.tables[0].readonly.iter().map(|i| addresses[*i as usize]).collect());
    assert_eq!(q.decompile(&qw, &qr).unwrap(), original);
    assert_eq!(q.signers(), p.signers());
    let sorted = |v: &[u8]| { let mut v = v.to_vec(); v.sort(); v };
    assert_eq!((sorted(&q.tables[0].writable), sorted(&q.tables[0].readonly)), (sorted(&t.writable), sorted(&t.readonly)));
    // a signer or an invoked program is never loaded from a table, and a table that supplies nothing is not named
    let payer_in_table = LookupTable { key: [5; 32], addresses: vec![p.keys[0], pk(etch_autobuyer::relaysol::DEPOSITORY_PROGRAM)] };
    let r = compile_v0_parts(&original, p.keys[0], p.blockhash, &[payer_in_table, table]).unwrap();
    assert_eq!(r.tables.len(), 1);
    assert!(r.keys.contains(&p.keys[0]));
    // the same with no table is a legal v0 message with every key static
    let s = compile_v0_parts(&original, p.keys[0], p.blockhash, &[]).unwrap();
    assert!(s.tables.is_empty() && s.decompile(&[], &[]).unwrap() == original);
    assert!(s.serialize().len() > q.serialize().len(), "the table saves bytes");
}

#[test]
fn program_derived_addresses_are_the_real_ones() {
    let program = pk(etch_autobuyer::relaysol::DEPOSITORY_PROGRAM);
    let (state, _) = find_program_address(&[b"relay_depository"], &program).unwrap();
    assert_eq!(b58_encode(&state), etch_autobuyer::relaysol::RELAY_DEPOSITORY);
    let (vault, bump) = find_program_address(&[b"vault"], &program).unwrap();
    assert_eq!((b58_encode(&vault).as_str(), bump), (etch_autobuyer::relaysol::VAULT, 255));
    assert!(!is_on_curve(&vault) && is_on_curve(&pk(etch_autobuyer::relaysol::SOLVER_2026_10)), "PDAs are off the curve, wallets on it");
    assert_eq!(create_program_address(&[b"vault", &[255]], &program), Some(vault));
    assert_eq!(create_program_address(&[&[0u8; 33]], &program), None, "a seed over 32 bytes");
    // associated token accounts, as the solver's USDC fill names them (tx-solver-fill-memo.json postTokenBalances)
    let fill = fixture("relay-sol/chain/tx-solver-fill-memo.json");
    let keys = fill["transaction"]["message"]["accountKeys"].as_array().unwrap();
    for b in fill["meta"]["postTokenBalances"].as_array().unwrap() {
        let acc = keys[b["accountIndex"].as_u64().unwrap() as usize].as_str().unwrap();
        assert_eq!(b58_encode(&associated_token_address(&pk(b["owner"].as_str().unwrap()), &pk(b["mint"].as_str().unwrap()))), acc);
    }
}

#[test]
fn borsh_reads_are_bounded_and_strict() {
    let d = Borsh::default().u32(3).bytes(b"abc").bool(true).u8(2).done();
    let mut r = Reader::new(&d);
    assert_eq!((r.string(3).unwrap(), r.bool().unwrap()), ("abc".to_string(), true));
    assert!(r.bool().is_err(), "a bool of 2");
    assert!(Reader::new(&d).string(2).is_err(), "over its max_len");
    assert!(Reader::new(&d[..5]).string(3).is_err(), "short");
    let big = Borsh::default().u32(u32::MAX).done();
    assert!(Reader::new(&big).string(usize::MAX).is_err(), "a length past the data, never a panic");
    assert!(Reader::account(&[0; 4], "Registry").is_err());
    assert_eq!(anchor_disc("global", "deposit_native"), etch_autobuyer::relaysol::IX_DEPOSIT_NATIVE);
}

// ---- solrpc: transforms on real devnet answers (Q3, 2026-10-09) -------------------------------------------------------------------

fn tf(kind: Kind, bucket: u64, body: &str) -> Value {
    normalise(200, body.as_bytes(), &context(kind, bucket))
}

#[test]
fn slots_round_down_and_moving_fields_are_dropped() {
    assert_eq!(tf(Kind::SlotBucket, 20, r#"{"jsonrpc":"2.0","result":509249712,"id":1}"#), json!({"n": 509249700}));
    assert_eq!(tf(Kind::SlotBucket, 20, r#"{"jsonrpc":"2.0","result":509249719,"id":7}"#), json!({"n": 509249700}));
    assert_eq!(tf(Kind::HeightBucket, 20, r#"{"jsonrpc":"2.0","result":496489286,"id":7}"#), json!({"n": 496489280}));
    let a = tf(Kind::Number, 1, r#"{"jsonrpc":"2.0","result":{"context":{"apiVersion":"4.4.0-beta.0","slot":509249719},"value":1009200},"id":1}"#);
    let b = tf(Kind::Number, 1, r#"{"jsonrpc":"2.0","result":{"context":{"apiVersion":"3.0.1","slot":509249755},"value":1009200},"id":1}"#);
    assert_eq!((a.clone(), a), (b, json!({"n": 1009200})));
}

#[test]
fn blocks_keep_hash_and_height_and_skips_are_a_class() {
    let v = tf(Kind::Block, 1, r#"{"jsonrpc":"2.0","result":{"blockHeight":496482075,"blockTime":1791565840,"blockhash":"E1Qcvhux7mnf7oB5UhNJrFkBsfhFyzdjbc6B6nPTM2qe","parentSlot":509249699,"previousBlockhash":"Cw35kz15LWn5oM7Xt9MfsVuwG9R5SnceJANj65nnB5as"},"id":1}"#);
    assert_eq!(v, json!({"blockhash": "E1Qcvhux7mnf7oB5UhNJrFkBsfhFyzdjbc6B6nPTM2qe", "blockHeight": 496482075u64, "parentSlot": 509249699u64}));
    assert_eq!(tf(Kind::Block, 1, r#"{"jsonrpc":"2.0","error":{"code":-32009,"message":"Slot 1 was skipped, or missing in long-term storage"},"id":1}"#), json!({"skipped": -32009}));
    assert_eq!(tf(Kind::Block, 1, r#"{"jsonrpc":"2.0","error":{"code":-32007,"message":"Slot 9 was skipped"},"id":1}"#), json!({"skipped": -32007}));
    assert_eq!(tf(Kind::Number, 1, r#"{"jsonrpc":"2.0","error":{"code":-32007,"message":"x"},"id":1}"#), json!({"error": -32007}));
}

#[test]
fn accounts_and_statuses_drop_the_context() {
    let v = tf(Kind::Account, 1, r#"{"jsonrpc":"2.0","result":{"context":{"apiVersion":"4.4.0-beta.0","slot":509249715},"value":{"data":["2BMAAAAAAAAAAAAAAADwPzI=","base64"],"executable":false,"lamports":1009200,"owner":"Sysvar1111111111111111111111111111111111111","rentEpoch":18446744073709551615,"space":17}},"id":1}"#);
    assert_eq!(v, json!({"account": {"lamports": 1009200, "owner": "Sysvar1111111111111111111111111111111111111", "executable": false, "data": "2BMAAAAAAAAAAAAAAADwPzI="}}));
    assert_eq!(tf(Kind::Account, 1, r#"{"jsonrpc":"2.0","result":{"context":{"slot":5},"value":null},"id":1}"#), json!({"account": null}));
    // an encoding not asked for is kept whole (and so cannot pass for base64)
    let j = tf(Kind::Account, 1, r#"{"jsonrpc":"2.0","result":{"context":{"slot":5},"value":{"data":{"parsed":1},"executable":false,"lamports":1,"owner":"11111111111111111111111111111111"}},"id":1}"#);
    assert!(j["account"]["data"].get("unexpected").is_some());
    let m = tf(Kind::Accounts, 1, r#"{"jsonrpc":"2.0","result":{"context":{"slot":9},"value":[null,{"data":["AQ==","base64"],"executable":false,"lamports":2,"owner":"11111111111111111111111111111111","rentEpoch":0,"space":1}]},"id":1}"#);
    assert_eq!(m, json!({"accounts": [null, {"lamports": 2, "owner": "11111111111111111111111111111111", "executable": false, "data": "AQ=="}]}));
    assert_eq!(tf(Kind::SigStatus, 1, r#"{"jsonrpc":"2.0","result":{"context":{"apiVersion":"4.4.0-beta.0","slot":509249718},"value":[null]},"id":1}"#), json!({"statuses": [null]}));
    let a = tf(Kind::SigStatus, 1, r#"{"jsonrpc":"2.0","result":{"context":{"slot":82},"value":[{"slot":72,"confirmations":10,"err":null,"status":{"Ok":null},"confirmationStatus":"confirmed"}]},"id":1}"#);
    let b = tf(Kind::SigStatus, 1, r#"{"jsonrpc":"2.0","result":{"context":{"slot":85},"value":[{"slot":72,"confirmations":13,"err":null,"status":{"Ok":null},"confirmationStatus":"confirmed"}]},"id":1}"#);
    assert_eq!(a, b);
    assert_eq!(a, json!({"statuses": [{"status": "confirmed", "err": null}]}));
}

#[test]
fn send_answers_are_classes_not_prose() {
    assert_eq!(tf(Kind::Send, 1, r#"{"jsonrpc":"2.0","result":"57LgFZH6Xi2roFRWjbzA7ciFM6sNuSg2CpSZ4kT9a7wSRhrM4pHP12psXi6ZcXUsQk3Jv21rrfH1bSTCq2dwPhuP","id":1}"#), json!({"class": "accepted"}));
    let ap = tf(Kind::Send, 1, r#"{"jsonrpc":"2.0","error":{"code":-32002,"message":"Transaction simulation failed: This transaction has already been processed","data":{"accounts":null,"err":"AlreadyProcessed","logs":[],"unitsConsumed":0}},"id":1}"#);
    assert_eq!(ap["class"], "already_processed");
    let bh = tf(Kind::Send, 1, r#"{"jsonrpc":"2.0","error":{"code":-32002,"message":"Transaction simulation failed: Blockhash not found","data":{"accounts":null,"err":"BlockhashNotFound","logs":[],"unitsConsumed":0}},"id":1}"#);
    let bh2 = tf(Kind::Send, 1, r#"{"jsonrpc":"2.0","error":{"code":-32002,"message":"Transaction simulation failed: Blockhash not found","data":{"accounts":null,"err":"BlockhashNotFound","logs":["x"],"unitsConsumed":150}},"id":1}"#);
    assert_eq!((bh["class"].clone(), &bh), (json!("blockhash_not_found"), &bh2));
    let anf = tf(Kind::Send, 1, r#"{"jsonrpc":"2.0","error":{"code":-32002,"message":"Transaction simulation failed: Attempt to debit an account but found no record of a prior credit.","data":{"accounts":null,"err":"AccountNotFound","logs":[],"unitsConsumed":0}},"id":1}"#);
    assert_eq!(anf["class"], "account_not_found");
    let sf = tf(Kind::Send, 1, r#"{"jsonrpc":"2.0","error":{"code":-32003,"message":"Transaction signature verification failure"},"id":1}"#);
    assert_eq!(sf["class"], "signature_failure");
    assert_eq!(normalise(429, b"Too many requests", &context(Kind::Send, 1)), json!({"http": 429}));
    assert_eq!(normalise(200, b"<html>", &context(Kind::Account, 1)), json!({"unparsable": true}));
    assert_eq!(normalise(200, b"{}", &[99]), json!({"bad_context": true}));
}

#[test]
fn disagreement_is_refused_never_guessed_and_caps_are_bounded() {
    assert_eq!(agree(vec![Ok(json!({"n": 1})), Ok(json!({"n": 1}))]).unwrap(), json!({"n": 1}));
    assert!(matches!(agree(vec![Ok(json!({"n": 1})), Ok(json!({"n": 2}))]), Err(Refused::Disagree(_))));
    assert!(matches!(agree(vec![Ok(json!({"n": 1})), Err("timeout".into())]), Err(Refused::Failed(_))));
    assert!(matches!(agree(vec![]), Err(Refused::Failed(_))));
    // a cap fits the largest account read (the Module, 957 bytes) and stays small
    assert!(account_cap(957) >= 2_000 + 957 * 4 / 3 && account_cap(957) < 4_000, "a 957-byte account (a rook_core Module, once)");
    assert!(account_cap(80) < 2_400);
}

// ---- solkey ---------------------------------------------------------------------------------------------------------------------

#[test]
fn the_key_signs_only_a_message_it_alone_signs_and_that_fits() {
    let me = [0x11; 32];
    assert!(solkey::pubkey_of(&[1; 33]).is_err());
    assert_eq!(solkey::pubkey_of(&[1; 32]).unwrap(), [1; 32]);
    let one = compile(&[transfer(me, [2; 32], 1)], me, [3; 32]);
    assert!(solkey::signable(&one, &me).is_ok());
    assert!(solkey::signable(&one, &[9; 32]).unwrap_err().contains("signs alone"));
    let two = compile(&[transfer(me, [2; 32], 1), transfer([4; 32], [2; 32], 1)], me, [3; 32]);
    assert!(solkey::signable(&two, &me).is_err());
    let big = compile(&(0..30u8).map(|i| transfer(me, [i + 10; 32], 1)).collect::<Vec<_>>(), me, [3; 32]);
    assert!(solkey::signable(&big, &me).unwrap_err().contains("1232"));
    assert_eq!(solkey::DERIVATION, b"sol");
}

// ---- Init.home -----------------------------------------------------------------------------------------------------------------

fn sol_home() -> Home {
    Home::Solana { cluster: "devnet".into(), rpc_urls: vec!["https://api.devnet.solana.com".into(), "https://solana-devnet.api.onfinality.io/public".into()],
        rook_core: "books".into(), nonce_seed: "rook-home-0".into(), blockhash_fallback: None, rook_backing: None, drand_urls: None }
}

/// Decision D-l (docs/v9/canister-only-solana.md §0): Rook deploys no Solana programs. The home names `rook_core = "books"` and nothing
/// else; a program id, for rook_core or rook_backing, is refused with the decision named.
#[test]
fn a_solana_home_needs_two_distinct_providers_a_known_cluster_and_the_books() {
    assert!(validate_home(&sol_home()).is_ok());
    let with = |f: &dyn Fn(&mut Home)| { let mut h = sol_home(); f(&mut h); validate_home(&h) };
    let Home::Solana { rpc_urls, .. } = sol_home();
    assert!(with(&|h| { let Home::Solana { rpc_urls, .. } = h; rpc_urls.truncate(1); }).unwrap_err().contains("two distinct"));
    let same = rpc_urls[0].clone();
    assert!(with(&|h| { let Home::Solana { rpc_urls, .. } = h; rpc_urls[1] = same.clone(); }).unwrap_err().contains("two distinct"));
    assert!(with(&|h| { let Home::Solana { cluster, .. } = h; *cluster = "mainnet".into(); }).unwrap_err().contains("cluster"));
    let e = with(&|h| { let Home::Solana { rook_core, .. } = h; *rook_core = "GepazuDZfS6zjwQ3U2zHCgk93xpRkp34AkmEFZKrAd7T".into(); }).unwrap_err();
    assert!(e.contains("rook_core") && e.contains("D-l") && e.contains("books"), "a program id, the earlier design: {e}");
    assert!(with(&|h| { let Home::Solana { rook_core, .. } = h; *rook_core = "0x01".into(); }).unwrap_err().contains("D-l"));
    assert!(with(&|h| { let Home::Solana { nonce_seed, .. } = h; *nonce_seed = "x".repeat(33); }).is_err());
    assert!(with(&|h| { let Home::Solana { nonce_seed, .. } = h; *nonce_seed = String::new(); }).is_err());
    // rook_backing is retired with the programs: any value is refused; mainnet-beta needs none (the ceremony's `rook_backing: null`)
    let e = with(&|h| { let Home::Solana { rook_backing, .. } = h; *rook_backing = Some("11111111111111111111111111111112".into()); }).unwrap_err();
    assert!(e.contains("rook_backing") && e.contains("D-l"), "{e}");
    assert!(with(&|h| { let Home::Solana { cluster, .. } = h; *cluster = "mainnet-beta".into(); }).is_ok());
    // the configuration the flows read: the lifetime and the seed; a program id never reaches it
    let cfg = etch_autobuyer::solhome::SolHomeCfg::of(&sol_home()).unwrap();
    assert_eq!((cfg.lifetime, cfg.nonce_seed.as_str()), (etch_autobuyer::solsend::LifetimeKind::DurableNonce, "rook-home-0"));
    let mut h = sol_home();
    let Home::Solana { rook_core, .. } = &mut h;
    *rook_core = "11111111111111111111111111111111".into();
    assert!(etch_autobuyer::solhome::SolHomeCfg::of(&h).unwrap_err().contains("D-l"));
}

#[test]
fn an_upgrade_keeps_the_homes_identity_and_takes_its_providers() {
    let saved = sol_home();
    let mut arg = sol_home();
    {
        let Home::Solana { cluster, rpc_urls, rook_core, nonce_seed, blockhash_fallback, rook_backing, drand_urls } = &mut arg;
        *rook_backing = Some("11111111111111111111111111111112".into());
        *drand_urls = Some(vec!["https://api.drand.sh".into(), "https://drand.cloudflare.com".into()]);
        *cluster = "mainnet-beta".into();
        *rook_core = "not-books".into();
        *nonce_seed = "other".into();
        *rpc_urls = vec!["https://a".into(), "https://b".into()];
        *blockhash_fallback = Some(true);
    }
    let Home::Solana { cluster, rpc_urls, rook_core, nonce_seed, blockhash_fallback, rook_backing, drand_urls } = restore_home(Some(saved.clone()), Some(&arg)).unwrap();
    assert_eq!(drand_urls.map(|v| v.len()), Some(2), "D-i: the drand relays are operational, from the argument");
    assert_eq!(rook_backing.as_deref(), Some("11111111111111111111111111111112"), "a home saved without rook_backing takes the argument's once");
    let mut saved2 = saved.clone();
    let Home::Solana { rook_backing: b, .. } = &mut saved2;
    *b = Some("11111111111111111111111111111113".into());
    let Some(Home::Solana { rook_backing, .. }) = restore_home(Some(saved2), Some(&arg)) else { panic!() };
    assert_eq!(rook_backing.as_deref(), Some("11111111111111111111111111111113"), "then it is identity");
    assert_eq!((cluster.as_str(), rook_core.as_str(), nonce_seed.as_str()), ("devnet", "books", "rook-home-0"), "identity as saved");
    assert_eq!((rpc_urls, blockhash_fallback), (vec!["https://a".to_string(), "https://b".to_string()], Some(true)), "operational from the argument");
    assert_eq!(restore_home(Some(saved.clone()), None), Some(saved.clone()), "an argument cannot remove a home");
    assert_eq!(restore_home(None, Some(&arg)), Some(arg), "set once, from nothing");
    assert_eq!(restore_home(None, None), None);
}

#[test]
fn an_init_with_a_solana_home_round_trips_through_candid() {
    let i = Init { chain_id: 4663, vault: "0x00000000000000000000000000000000000000cc".into(),
        rpc_urls: vec!["https://rpc.mainnet.chain.robinhood.com".into(), "https://rpc.ordofi.network".into()],
        btc_api_urls: vec!["https://mempool.space/api".into(), "https://blockstream.info/api".into()], key_name: "key_1".into(),
        accept_production_key_name: true, bridge_allowlist: vec![], btc_network: "mainnet".into(), operator: None, wrapper: None, evm_chains: None,
        custody_caps: None, outflow_limits: None, holder_deposits: None, wrapper_from_block: None, frog_funding: None, eth_nft: None,
        home: Some(sol_home()) };
    let back: Init = candid::decode_one(&candid::encode_one(&i).unwrap()).unwrap();
    assert_eq!(back.home, Some(sol_home()));
    // the .did names the field and the type the code encodes
    // (the .did is generated from the code, records spread over lines: the `Solana` variant's fields are checked one by one)
    let did = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/etch_autobuyer.did")).unwrap();
    let flat: String = did.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(flat.contains("home : opt Home;"));
    let sol = flat.split("Solana : record {").nth(1).expect("the Solana home variant").split('}').next().unwrap();
    for f in ["cluster : text;", "rpc_urls : vec text;", "rook_core : text;", "nonce_seed : text;", "blockhash_fallback : opt bool;",
        "rook_backing : opt text;", "drand_urls : opt vec text;"] {
        assert!(sol.contains(f), "{f} in the Solana home: {sol}");
    }
    assert!(flat.contains("sol_address : () -> (Result"));
}

// ---- phase B2: the line moves canister-wide; the cluster check; the simulation's transform; the frog wrapper on the home ---------

#[test]
fn b1s_line_moves_from_the_ethereum_sides_state_to_the_canister_wide_slot_on_upgrade() {
    use etch_autobuyer::canister::{lost_opts, Saved};
    use etch_autobuyer::ethflow::{EthState, Step};
    use etch_autobuyer::solhome::{self, LineStep, SolLine};
    // B1's own bytes (tests/fixtures/eth-state-sol-incident.hex, written by B1's ethnft_sol test): an EthState whose `sol` holds the
    // line with an incident under a pending advance. They decode into this build's EthState (the new `sol_step` absent: none)
    let bytes = hex::decode(include_str!("fixtures/eth-state-sol-incident.hex").trim()).unwrap();
    let eth: EthState = candid::decode_one(&bytes).unwrap();
    let old = eth.sol.clone().expect("B1 kept its line here");
    assert!(old.line.incident.is_some() && old.pending.is_some() && eth.sol_step.is_none());
    // the upgrade tuple B1 wrote had thirteen elements: it decodes into this build's fourteen with the line absent, and nothing that
    // was saved decodes as nothing (`post_upgrade`'s own check)
    type SavedB1 = (Option<Init>, Vec<etch_autobuyer::state::Purchase>, bool, Vec<String>, Option<etch_autobuyer::redeem::Custody>,
        Option<etch_autobuyer::chains::NftState>, Option<etch_autobuyer::frogflow::FrogState>, Option<etch_autobuyer::floortrigger::FloorState>,
        Option<(etch_autobuyer::custodycap::Book, etch_autobuyer::outflow::Outflow)>, Option<etch_autobuyer::handin::HandinState>,
        Option<etch_autobuyer::sale::SaleState>, Option<etch_autobuyer::custody::FeePool>, Option<EthState>);
    let b1: SavedB1 = (None, vec![], false, vec![], None, None, None, None, None, None, None, None, Some(eth.clone()));
    let stable = candid::encode_args((b1,)).unwrap();
    let (saved,): (Saved,) = candid::decode_args(&stable).unwrap();
    assert!(saved.13.is_none(), "no canister-wide line in a state B1 saved");
    assert!(lost_opts(&stable, &candid::encode_args((saved.clone(),)).unwrap()).unwrap().is_empty(), "nothing saved restores as nothing");
    // post_upgrade's move (`solhome::migrate`): the line, its intent counter, its deposits, its pending step; the Ethereum side keeps a
    // copy of its step; EthState.sol is empty from then on
    let mut eth = saved.12.unwrap();
    let line = solhome::migrate(saved.13, &mut eth).unwrap().expect("moved");
    assert_eq!(line.line, old.line, "the same nonce account, saved bytes, incident and history");
    assert_eq!((line.next_intent, line.opening, line.deposits.clone()), (old.next_intent, old.opening, old.deposits.clone()));
    let p = line.pending.clone().unwrap();
    assert!(matches!(p.step, LineStep::Eth(Step::Advance { .. })), "{:?}", p.step);
    assert_eq!((p.intent, p.at), (old.pending.as_ref().unwrap().intent, old.pending.as_ref().unwrap().at));
    assert_eq!(eth.sol_step, Some(p));
    assert!(eth.sol.is_none());
    // once: a state saved by this build carries no B1 line, and the move leaves the slot as it was
    assert_eq!(solhome::migrate(Some(line.clone()), &mut eth).unwrap(), Some(line.clone()));
    // a state carrying both is refused (nothing guesses which line is real), and an empty B1 line moves nothing
    let mut both: EthState = candid::decode_one(&bytes).unwrap();
    assert!(solhome::migrate(Some(line.clone()), &mut both).unwrap_err().contains("refused"));
    assert!(both.sol.is_some(), "kept for the operator to look at");
    let mut empty = EthState { sol: Some(Default::default()), ..Default::default() };
    assert_eq!(solhome::migrate(None, &mut empty).unwrap(), None);
    assert_eq!(solhome::migrate(Some(SolLine::default()), &mut EthState::default()).unwrap(), Some(SolLine::default()));
}

#[test]
fn the_solana_home_checks_its_cluster_by_genesis_hash_and_the_simulation_keeps_only_its_error() {
    use etch_autobuyer::solrpc::genesis_of;
    assert_eq!(genesis_of("mainnet-beta"), Some("5eykt4UsFv8P8NJdTREpY1vzqKqZKvdpKuc147dw2N9d"));
    assert_eq!(genesis_of("devnet"), Some("EtWTRABZaYq6iMfeYKouRu166VU2xqa1wcaWoxPkrZBG"));
    assert_eq!(genesis_of("testnet"), Some("4uhcVJyU9pJkvQyS88uRDiswHXSDkY9xV8AYqv2UwvNH"));
    for c in etch_autobuyer::canister::SOLANA_CLUSTERS {
        assert!(genesis_of(c).is_some(), "every cluster an install may name has its genesis pinned: {c}");
    }
    // getGenesisHash: the string, agreed
    assert_eq!(tf(Kind::Text, 1, r#"{"jsonrpc":"2.0","result":"EtWTRABZaYq6iMfeYKouRu166VU2xqa1wcaWoxPkrZBG","id":1}"#),
        json!({ "text": "EtWTRABZaYq6iMfeYKouRu166VU2xqa1wcaWoxPkrZBG" }));
    // simulateTransaction: the error only; logs, units and the context's slot move between providers and are dropped
    let a = tf(Kind::Simulate, 1, r#"{"jsonrpc":"2.0","result":{"context":{"apiVersion":"4.4.0","slot":9},"value":{"err":{"InstructionError":[1,{"Custom":6012}]},"logs":["Program log: a"],"unitsConsumed":1234,"accounts":null,"returnData":null}},"id":1}"#);
    let b = tf(Kind::Simulate, 1, r#"{"jsonrpc":"2.0","result":{"context":{"apiVersion":"3.0.1","slot":45},"value":{"err":{"InstructionError":[1,{"Custom":6012}]},"logs":["Program log: b"],"unitsConsumed":1999}},"id":1}"#);
    assert_eq!(a, b);
    assert_eq!(a, json!({ "sim_err": { "InstructionError": [1, { "Custom": 6012 }] } }));
    assert_eq!(tf(Kind::Simulate, 1, r#"{"jsonrpc":"2.0","result":{"context":{"slot":9},"value":{"err":null,"logs":[]}},"id":1}"#),
        json!({ "sim_err": null }));
    assert_eq!(etch_autobuyer::solsend::simulation_class("\"InsufficientFundsForFee\""), "insufficient_funds");
    assert_eq!(etch_autobuyer::solsend::simulation_class("\"AccountNotFound\""), "insufficient_funds");
    assert_eq!(etch_autobuyer::solsend::simulation_class("{\"InstructionError\":[1,{\"Custom\":6012}]}"), "error");
    // getProgramAccounts with a dataSlice: (address, owner, the slice), sorted, whatever order a provider answers in
    let one = |k: &str, d: &str| json!({ "pubkey": k, "account": { "data": [d, "base64"], "owner": TO, "lamports": 1, "executable": false, "rentEpoch": 0, "space": 259 } });
    let body = |v: Vec<Value>| json!({ "jsonrpc": "2.0", "result": v, "id": 1 }).to_string();
    let x = tf(Kind::ProgramSlices, 1, &body(vec![one(A, "AQ=="), one(TO, "Ag==")]));
    let y = tf(Kind::ProgramSlices, 1, &body(vec![one(TO, "Ag=="), one(A, "AQ==")]));
    assert_eq!(x, y, "sorted by address");
}

#[test]
fn the_frog_wrapper_names_the_homes_chain_and_holder_deposits_stay_off_on_solana() {
    use etch_autobuyer::canister::{validate_frog_home, wrapper_key};
    let (pda, _) = find_program_address(&[b"wrapper", &[7; 32]], &pk(etch_autobuyer::relaysol::DEPOSITORY_PROGRAM)).unwrap();
    let b58 = b58_encode(&pda);
    let (k, sol) = wrapper_key(&b58).unwrap();
    assert_eq!((k.as_slice(), sol), (&pda[..20], Some(pda)), "the record key is the PDA's first 20 bytes");
    assert_eq!(wrapper_key("0x00000000000000000000000000000000000000cc").unwrap().1, None);
    assert!(validate_frog_home(Some(&b58), false, true).is_ok());
    assert!(validate_frog_home(Some("0x00000000000000000000000000000000000000cc"), false, false).is_ok());
    assert!(validate_frog_home(Some(&b58), false, false).unwrap_err().contains("Solana address"), "a Solana wrapper on the EVM home");
    assert!(validate_frog_home(Some("0x00000000000000000000000000000000000000cc"), false, true).unwrap_err().contains("EVM address"));
    assert!(validate_frog_home(None, true, true).unwrap_err().contains("OFF on the Solana home"), "EWD2 deposits are off at launch");
    assert!(validate_frog_home(None, true, false).is_ok(), "the EVM home's deposits, unchanged");
    // no EVM chain is the home on Solana: Ethereum may be listed whatever the vault loop's chain id says
    assert_eq!(etch_autobuyer::chains::evm_home(1, true), 0);
    assert_eq!(etch_autobuyer::chains::evm_home(4663, false), 4663);
}

