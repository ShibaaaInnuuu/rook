//! Native tests: the bytes are held to the Solana CLI 4.3.0's own output, the transforms to real devnet answers (2026-10-09).
use crate::solrpc::{agree, context, normalise, Kind, Refused};
use crate::soltx::*;
use base64::Engine;
use serde_json::json;

const A: &str = "BWbreqGmzv4nZ5nX5DvPH6pihkjsxo7dx1Bi3Dax94ic"; // a throwaway key, generated and deleted (public half only here)
const N: &str = "2U7CZHN2Qh2Cymgwjk1zyjkfApqieDr6rsWKEcXBCYGh";
const TO: &str = "9xQeWvG816bUx9EPjHmaT23yvVM2ZWbrrpZb9PusVFin";
const BH: &str = "GHtXQBsoZHVnNFa9YevAzFr17DJjgHXk3ycTKD5xD3Zi";
/// `solana transfer --sign-only --blockhash BH --nonce N --nonce-authority A --from A --fee-payer A TO 0.001 --dump-transaction-message`
const CLI_MESSAGE: &str = "AQACBZwqH+bEuLg7XWKDmXUJtMdDZDMckHvQPXJ2Zuefj18pFc1g13G6Y2VbV7Mc+jtZQsYvGVRKBcX3gVUZ2kImjUKFDy1uAqR6+CTQmradxC1wyyjL+iSft+5XudJWwSdi7wAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAABqfVFxksVo7gioRfc9KXiM8DXDFFshqzRNgGLqlAAADjMtr5L6vs6LY/96RABeX9/Zr6FYdWthxalfkEs7jQgQIDAwEEAAQEAAAAAwIAAgwCAAAAQEIPAAAAAAA=";

fn pk(s: &str) -> Pubkey {
    pubkey(s).unwrap()
}

#[test]
fn base58_round_trips_and_knows_the_system_program() {
    assert_eq!(b58_encode(&SYSTEM_PROGRAM), "11111111111111111111111111111111");
    for s in [A, N, TO, BH, SYSVAR_RENT, SYSVAR_RECENT_BLOCKHASHES] {
        assert_eq!(b58_encode(&pk(s)), s);
    }
    assert!(pubkey("0OIl").is_err());
    assert!(pubkey("1111").is_err()); // 4 bytes
}

#[test]
fn the_durable_nonce_transfer_is_byte_for_byte_the_clis() {
    let (a, n) = (pk(A), pk(N));
    let msg = compile(&[advance_nonce(n, a), transfer(a, pk(TO), 1_000_000)], a, pk(BH));
    let cli = base64::engine::general_purpose::STANDARD.decode(CLI_MESSAGE).unwrap();
    assert_eq!(msg.bytes, cli);
    assert_eq!(msg.signers, vec![a]);
    assert_eq!(wire_size(&msg), 1 + 64 + cli.len());
    let tx = assemble(&msg, &[[7u8; 64]]).unwrap();
    assert_eq!(tx.len(), wire_size(&msg));
    assert_eq!(tx[0], 1);
    assert_eq!(&tx[65..], &cli[..]);
    assert!(assemble(&msg, &[]).is_err());
}

#[test]
fn the_seed_address_is_the_clis() {
    // `solana create-address-with-seed --from A rook-q3-nonce-0 SYSTEM`
    assert_eq!(b58_encode(&address_with_seed(&pk(A), "rook-q3-nonce-0", &SYSTEM_PROGRAM).unwrap()), "97ncgGo96sQ18Xop3jBtydV2nwnsQJ46atKKsL2Hx9Ab");
    assert!(address_with_seed(&pk(A), &"x".repeat(33), &SYSTEM_PROGRAM).is_err());
}

#[test]
fn create_and_initialise_nonce_has_one_signer() {
    let a = pk(A);
    let n = address_with_seed(&a, "rook-q3-nonce-0", &SYSTEM_PROGRAM).unwrap();
    let msg = compile(&[create_with_seed(a, n, a, "rook-q3-nonce-0", 1_056_640, 80, SYSTEM_PROGRAM), initialize_nonce(n, a)], a, pk(BH));
    assert_eq!(msg.signers, vec![a]);
    assert_eq!(msg.bytes[..3], [1, 0, 3]); // 1 signer; system program + two sysvars readonly
    assert!(wire_size(&msg) < PACKET_DATA_SIZE);
}

#[test]
fn the_1232_byte_check_refuses_before_signing() {
    let a = pk(A);
    // each extra recipient costs 32 (key) + 1 + 1 + 2 + 1 + 12 bytes: 22 recipients pass, 23 do not
    let ixs = |k: u8| -> Vec<Instruction> {
        let mut v = vec![advance_nonce(pk(N), a)];
        v.extend((0..k).map(|i| transfer(a, [i + 1; 32], 1)));
        v
    };
    let fits = (1..60u8).take_while(|k| check_size(wire_size(&compile(&ixs(*k), a, pk(BH)))).is_ok()).last().unwrap();
    let too_big = compile(&ixs(fits + 1), a, pk(BH));
    assert!(wire_size(&too_big) > PACKET_DATA_SIZE);
    assert!(check_size(wire_size(&too_big)).unwrap_err().contains("1232"));
    assert!(assemble(&too_big, &[[0; 64]]).is_err());
    assert!(fits >= 20, "{fits} transfers fit");
}

#[test]
fn compact_u16_matches_the_short_vec_encoding() {
    for (n, want) in [(0usize, vec![0u8]), (0x7f, vec![0x7f]), (0x80, vec![0x80, 0x01]), (0x3fff, vec![0xff, 0x7f]), (0x4000, vec![0x80, 0x80, 0x01])] {
        let mut v = vec![];
        compact_u16(n, &mut v);
        assert_eq!(v, want, "{n}");
    }
}

#[test]
fn the_nonce_account_decodes() {
    let mut d = vec![1, 0, 0, 0, 1, 0, 0, 0];
    d.extend(pk(A));
    d.extend(pk(BH));
    d.extend(5000u64.to_le_bytes());
    let s = decode_nonce(&d).unwrap();
    assert_eq!((s.authority, s.durable_nonce, s.lamports_per_signature), (pk(A), pk(BH), 5000));
    d[4] = 0;
    assert!(decode_nonce(&d).is_err());
    assert!(decode_nonce(&d[..79]).is_err());
}

// ---- transforms on real devnet answers ----------------------------------------------------------------------------------------

fn tf(kind: Kind, bucket: u64, body: &str) -> serde_json::Value {
    normalise(200, body.as_bytes(), &context(kind, bucket))
}

#[test]
fn slots_round_down_and_moving_fields_are_dropped() {
    assert_eq!(tf(Kind::SlotBucket, 20, r#"{"jsonrpc":"2.0","result":509249712,"id":1}"#), json!({"n": 509249700}));
    assert_eq!(tf(Kind::SlotBucket, 20, r#"{"jsonrpc":"2.0","result":509249719,"id":7}"#), json!({"n": 509249700}));
    assert_eq!(tf(Kind::SlotBucket, 1, r#"{"jsonrpc":"2.0","result":509249719,"id":7}"#), json!({"n": 509249719}));
    let a = tf(Kind::Number, 1, r#"{"jsonrpc":"2.0","result":{"context":{"apiVersion":"4.4.0-beta.0","slot":509249719},"value":1009200},"id":1}"#);
    let b = tf(Kind::Number, 1, r#"{"jsonrpc":"2.0","result":{"context":{"apiVersion":"3.0.1","slot":509249755},"value":1009200},"id":1}"#);
    assert_eq!((a.clone(), a), (b, json!({"n": 1009200})));
    assert_eq!(tf(Kind::Number, 1, r#"{"jsonrpc":"2.0","result":1056640,"id":1}"#), json!({"n": 1056640}));
}

#[test]
fn blocks_keep_hash_and_height_and_skips_are_a_class() {
    let v = tf(Kind::Block, 1, r#"{"jsonrpc":"2.0","result":{"blockHeight":496482075,"blockTime":1791565840,"blockhash":"E1Qcvhux7mnf7oB5UhNJrFkBsfhFyzdjbc6B6nPTM2qe","parentSlot":509249699,"previousBlockhash":"Cw35kz15LWn5oM7Xt9MfsVuwG9R5SnceJANj65nnB5as"},"id":1}"#);
    assert_eq!(v, json!({"blockhash": "E1Qcvhux7mnf7oB5UhNJrFkBsfhFyzdjbc6B6nPTM2qe", "blockHeight": 496482075u64, "parentSlot": 509249699u64}));
    let s = tf(Kind::Block, 1, r#"{"jsonrpc":"2.0","error":{"code":-32009,"message":"Slot 1 was skipped, or missing in long-term storage"},"id":1}"#);
    assert_eq!(s, json!({"skipped": -32009}));
    assert_eq!(tf(Kind::Block, 1, r#"{"jsonrpc":"2.0","error":{"code":-32007,"message":"Slot 9 was skipped"},"id":1}"#), json!({"skipped": -32007}));
}

#[test]
fn accounts_and_statuses_drop_the_context() {
    let v = tf(Kind::Account, 1, r#"{"jsonrpc":"2.0","result":{"context":{"apiVersion":"4.4.0-beta.0","slot":509249715},"value":{"data":["2BMAAAAAAAAAAAAAAADwPzI=","base64"],"executable":false,"lamports":1009200,"owner":"Sysvar1111111111111111111111111111111111111","rentEpoch":18446744073709551615,"space":17}},"id":1}"#);
    assert_eq!(v, json!({"account": {"lamports": 1009200, "owner": "Sysvar1111111111111111111111111111111111111", "data": "2BMAAAAAAAAAAAAAAADwPzI="}}));
    assert_eq!(tf(Kind::Account, 1, r#"{"jsonrpc":"2.0","result":{"context":{"slot":5},"value":null},"id":1}"#), json!({"account": null}));
    assert_eq!(tf(Kind::SigStatus, 1, r#"{"jsonrpc":"2.0","result":{"context":{"apiVersion":"4.4.0-beta.0","slot":509249718},"value":[null]},"id":1}"#), json!({"status": null}));
    let a = tf(Kind::SigStatus, 1, r#"{"jsonrpc":"2.0","result":{"context":{"slot":82},"value":[{"slot":72,"confirmations":10,"err":null,"status":{"Ok":null},"confirmationStatus":"confirmed"}]},"id":1}"#);
    let b = tf(Kind::SigStatus, 1, r#"{"jsonrpc":"2.0","result":{"context":{"slot":85},"value":[{"slot":72,"confirmations":13,"err":null,"status":{"Ok":null},"confirmationStatus":"confirmed"}]},"id":1}"#);
    assert_eq!(a, b);
    assert_eq!(a, json!({"status": "confirmed", "err": null}));
}

#[test]
fn send_answers_are_classes_not_prose() {
    assert_eq!(tf(Kind::Send, 1, r#"{"jsonrpc":"2.0","result":"57LgFZH6Xi2roFRWjbzA7ciFM6sNuSg2CpSZ4kT9a7wSRhrM4pHP12psXi6ZcXUsQk3Jv21rrfH1bSTCq2dwPhuP","id":1}"#), json!({"class": "accepted"}));
    let ap = tf(Kind::Send, 1, r#"{"jsonrpc":"2.0","error":{"code":-32002,"message":"Transaction simulation failed: This transaction has already been processed","data":{"accounts":null,"err":"AlreadyProcessed","logs":[],"unitsConsumed":0}},"id":1}"#);
    assert_eq!(ap["class"], "already_processed");
    let bh = tf(Kind::Send, 1, r#"{"jsonrpc":"2.0","error":{"code":-32002,"message":"Transaction simulation failed: Blockhash not found","data":{"accounts":null,"err":"BlockhashNotFound","logs":[],"unitsConsumed":0}},"id":1}"#);
    let bh2 = tf(Kind::Send, 1, r#"{"jsonrpc":"2.0","error":{"code":-32002,"message":"Transaction simulation failed: Blockhash not found","data":{"accounts":null,"err":"BlockhashNotFound","logs":["x"],"unitsConsumed":150}},"id":1}"#);
    assert_eq!((bh["class"].clone(), &bh), (json!("blockhash_not_found"), &bh2));
    let de = tf(Kind::Send, 1, r#"{"jsonrpc":"2.0","error":{"code":-32602,"message":"failed to deserialize solana_transaction::versioned::VersionedTransaction: Attempting to read 64 bytes"},"id":1}"#);
    assert_eq!(de, json!({"class": "error", "code": -32602, "err": null}));
    let anf = tf(Kind::Send, 1, r#"{"jsonrpc":"2.0","error":{"code":-32002,"message":"Transaction simulation failed: Attempt to debit an account but found no record of a prior credit.","data":{"accounts":null,"err":"AccountNotFound","logs":[],"unitsConsumed":0}},"id":1}"#);
    assert_eq!(anf["class"], "account_not_found");
    let sf = tf(Kind::Send, 1, r#"{"jsonrpc":"2.0","error":{"code":-32003,"message":"Transaction signature verification failure"},"id":1}"#);
    assert_eq!(sf["class"], "signature_failure");
    assert_eq!(normalise(429, b"Too many requests", &context(Kind::Send, 1)), json!({"http": 429}));
}

#[test]
fn disagreement_is_refused_never_guessed() {
    assert_eq!(agree(vec![Ok(json!({"n": 1})), Ok(json!({"n": 1}))]).unwrap(), json!({"n": 1}));
    assert!(matches!(agree(vec![Ok(json!({"n": 1})), Ok(json!({"n": 2}))]), Err(Refused::Disagree(_))));
    assert!(matches!(agree(vec![Ok(json!({"n": 1})), Err("timeout".into())]), Err(Refused::Failed(_))));
    assert!(matches!(agree(vec![]), Err(Refused::Failed(_))));
}

/// `SOL_SIGN_WRITE_DID=1 cargo test did` regenerates `sol_sign.did`; otherwise the file is held to the code.
#[test]
fn did_is_current() {
    let did = crate::__export_service();
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/sol_sign.did");
    if std::env::var("SOL_SIGN_WRITE_DID").is_ok() {
        std::fs::write(path, &did).unwrap();
    }
    assert_eq!(std::fs::read_to_string(path).unwrap_or_default(), did, "run SOL_SIGN_WRITE_DID=1 cargo test did");
}
