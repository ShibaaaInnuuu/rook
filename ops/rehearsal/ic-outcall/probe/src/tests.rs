//! Native tests: the copied transform bodies against the buyer's own functions on the real providers' answers (read-only RPC,
//! 2026-09-28, `tests/fixtures/<provider>-<read>.json`), and the verdict on a good run and on each way a run must fail.
use crate::bodies as b;
use crate::check::{self, Answers};
use etch_autobuyer as buyer;

const PROVIDERS: [&str; 2] = ["mevblocker", "tenderly"];
const URLS: [&str; 2] = check::PAIR;
const PIN: u64 = 26_070_000;

fn raw(p: &str, k: &str) -> Vec<u8> {
    std::fs::read(format!("{}/tests/fixtures/{p}-{k}.json", env!("CARGO_MANIFEST_DIR"))).unwrap()
}

fn head_raw(h: u64) -> Vec<u8> {
    format!(r#"{{"jsonrpc":"2.0","id":1,"result":"0x{h:x}"}}"#).into_bytes()
}

/// What the probe's transforms make of one provider's real answers.
fn answers(i: usize, head: u64) -> Answers {
    let p = PROVIDERS[i];
    Answers {
        url: URLS[i].into(),
        head: Some(Ok(b::head_bucket_body(&head_raw(head), check::BUCKET))),
        // the live pin's block, both providers agreeing (the fixture block stands in for it)
        pin_block: Some(Ok(b::block_body_ts(&raw("mevblocker", "block")))),
        block: Some(Ok(b::block_body_ts(&raw(p, "block")))),
        logs: Some(Ok(b::eth_transfer_logs(200, &raw(p, "logs")))),
        receipt: Some(Ok(b::eth_receipt_logs(200, &raw(p, "receipt"), PIN))),
    }
}

fn good() -> Vec<Answers> {
    vec![answers(0, PIN + 9), answers(1, PIN + 3)]
}

fn failed(cs: &[check::Check]) -> Vec<String> {
    cs.iter().filter(|c| !c.pass).map(|c| format!("{}: {}", c.name, c.detail)).collect()
}

#[test]
fn the_copied_bodies_are_the_buyers_own_byte_for_byte() {
    let weird: [&[u8]; 6] = [b"<html>429</html>", br#"{"jsonrpc":"2.0","id":1,"error":{"code":-32602,"message":"Archive requests require a personal token"}}"#,
        br#"{"jsonrpc":"2.0","id":1,"error":{"code":-32005,"message":"query returned more than 10000 results"}}"#,
        br#"{"jsonrpc":"2.0","id":1,"error":{"code":-32005,"message":"rate limit exceeded"}}"#, br#"{"jsonrpc":"2.0","id":1,"result":null}"#, b""];
    for p in PROVIDERS {
        for k in ["block", "logs", "receipt"] {
            let r = raw(p, k);
            assert_eq!(b::block_body_ts(&r), buyer::chains::block_body_ts(&r), "{p} {k}: block_body_ts");
            assert_eq!(b::transfer_logs_body(&r), buyer::ethnft::transfer_logs_body(&r), "{p} {k}: transfer_logs_body");
            assert_eq!(b::json_rpc_norm_body(&r), buyer::pin::json_rpc_norm_body(&r), "{p} {k}: json_rpc_norm_body");
            let v: serde_json::Value = serde_json::from_slice(&r).unwrap();
            for pin in [0, b'x' as u64, check::PIN_TX_BLOCK - 1, check::PIN_TX_BLOCK, PIN] {
                assert_eq!(b::receipt_projection(&v["result"], pin), buyer::ethnft::receipt_projection(&v["result"], pin), "{p} {k}: receipt_projection");
            }
        }
    }
    for w in weird {
        assert_eq!(b::block_body_ts(w), buyer::chains::block_body_ts(w));
        assert_eq!(b::transfer_logs_body(w), buyer::ethnft::transfer_logs_body(w));
        assert_eq!(b::json_rpc_norm_body(w), buyer::pin::json_rpc_norm_body(w));
        assert_eq!(b::head_bucket_body(w, 8), buyer::pin::head_bucket_body(w, 8));
    }
    for h in [0u64, 7, 8, 26_070_013] {
        assert_eq!(b::head_bucket_body(&head_raw(h), 8), buyer::pin::head_bucket_body(&head_raw(h), 8));
    }
    assert_eq!(b::busy_body(), buyer::pin::busy_body());
    assert_eq!(b::TRANSFER, buyer::ethnft::TRANSFER);
    for hs in [vec![100u64, 108], vec![100, 116], vec![8, 8], vec![]] {
        assert_eq!(b::choose_pin(&hs, 8), buyer::pin::choose_pin(&hs, 8));
    }
}

#[test]
fn the_real_answers_of_the_pair_pass_every_check() {
    let cs = check::judge(&good());
    assert!(failed(&cs).is_empty(), "{:#?}", failed(&cs));
    assert_eq!(cs.len(), 5 * 2 + 6);
    // the raw answers differ (sizes, field order), the transformed ones do not: the transforms are what makes the pair agree
    assert_ne!(raw("mevblocker", "receipt"), raw("tenderly", "receipt"));
    // the raw answers hold the 10 logs, which are 7 transactions (PIN_MOVES), none a mint
    for p in PROVIDERS {
        let v: serde_json::Value = serde_json::from_slice(&raw(p, "logs")).unwrap();
        let ls = v["result"].as_array().unwrap();
        assert_eq!(ls.len(), check::PIN_LOGS, "{p}");
        let txs: std::collections::BTreeSet<&str> = ls.iter().map(|l| l["transactionHash"].as_str().unwrap()).collect();
        assert_eq!(txs.len(), check::PIN_MOVES, "{p}");
    }
}

#[test]
fn a_short_or_empty_log_answer_fails_d7_l_1() {
    for body in [br#"{"jsonrpc":"2.0","id":1,"result":[]}"#.to_vec(), {
        let mut v: serde_json::Value = serde_json::from_slice(&raw("tenderly", "logs")).unwrap();
        v["result"].as_array_mut().unwrap().pop();
        v.to_string().into_bytes()
    }] {
        let mut ps = good();
        ps[1].logs = Some(Ok(b::eth_transfer_logs(200, &body)));
        let f = failed(&check::judge(&ps));
        assert!(f.iter().any(|x| x.starts_with("gateway.tenderly.co: Milady Transfer logs")), "{f:#?}");
        assert!(f.iter().any(|x| x.starts_with("the pair agrees on the logs")), "{f:#?}");
    }
}

#[test]
fn a_deep_refusal_a_null_receipt_or_a_receipt_above_the_pin_fails() {
    let mut ps = good();
    ps[0].logs = Some(Ok(b::eth_transfer_logs(200, br#"{"jsonrpc":"2.0","id":1,"error":{"code":-32602,"message":"Archive requests require a personal token"}}"#)));
    let f = failed(&check::judge(&ps));
    assert!(f.iter().any(|x| x.contains("rpc.mevblocker.io: Milady") && x.contains("\"class\":\"deep\"")), "{f:#?}");
    let mut ps = good();
    ps[1].receipt = Some(Ok(b::eth_receipt_logs(200, br#"{"jsonrpc":"2.0","id":1,"result":null}"#, PIN)));
    assert!(failed(&check::judge(&ps)).iter().any(|x| x.starts_with("gateway.tenderly.co: receipt") && x.contains("null")));
    let mut ps = good();
    ps[1].receipt = Some(Ok(b::eth_receipt_logs(200, &raw("tenderly", "receipt"), check::PIN_TX_BLOCK - 1)));
    assert!(failed(&check::judge(&ps)).iter().any(|x| x.starts_with("gateway.tenderly.co: receipt")));
}

#[test]
fn tenderly_unreachable_fails_by_name_and_a_disagreement_fails() {
    let mut ps = good();
    ps[1].head = Some(Err("CallRejected(SysTransient, \"Connecting to gateway.tenderly.co failed\")".into()));
    let f = failed(&check::judge(&ps));
    assert!(f.iter().any(|x| x.starts_with("gateway.tenderly.co (IPv4 only) is reachable")), "{f:#?}");
    assert!(f.iter().any(|x| x.starts_with("the pair's heads pin")));
    // a block that differs after the transform: nothing is believed
    let mut ps = good();
    let mut v: serde_json::Value = serde_json::from_slice(&raw("tenderly", "block")).unwrap();
    v["result"]["baseFeePerGas"] = "0x1".into();
    ps[1].block = Some(Ok(b::block_body_ts(v.to_string().as_bytes())));
    assert!(failed(&check::judge(&ps)).iter().any(|x| x.starts_with("the pair agrees on the block")));
    // heads more than a bucket apart
    let ps = vec![answers(0, PIN + 40), answers(1, PIN)];
    assert!(failed(&check::judge(&ps)).iter().any(|x| x.starts_with("the pair's heads pin")));
    // the buyer's pin read refused by the IC for its size (measured locally 2026-09-28: recent Ethereum blocks are 11-51 kB with
    // hashes only, median 25.5 kB, over nftnet::pin_on's 16,000-byte cap): a FAIL naming the pin read, and no agreement
    let mut ps = good();
    for p in ps.iter_mut() { p.pin_block = Some(Err("CallRejected(CallRejected { raw_reject_code: 1, reject_message: \"Http body exceeds size limit of 16000 bytes.\" })".into())); }
    let f = failed(&check::judge(&ps));
    assert!(f.iter().filter(|x| x.contains("the buyer's pin read") && x.contains("exceeds size limit")).count() == 2, "{f:#?}");
    assert!(f.iter().any(|x| x.starts_with("the pair agrees on the pinned block")));
    // a rate limit page (HTTP 429, no JSON) is a busy error, as the buyer reads it
    assert_eq!(b::eth_transfer_logs(429, b"error code: 1015"), buyer::pin::busy_body());
}

#[test]
fn the_candid_interface_is_the_committed_one() {
    let did = crate::__export_service();
    let committed = std::fs::read_to_string(format!("{}/probe.did", env!("CARGO_MANIFEST_DIR"))).unwrap_or_default();
    if committed != did {
        std::fs::write(format!("{}/probe.did.new", env!("CARGO_MANIFEST_DIR")), &did).unwrap();
        panic!("probe.did is stale: the interface is in probe.did.new");
    }
}

#[test]
fn the_pin_read_cap_is_the_buyers() {
    assert_eq!(check::PIN_BLOCK_BYTES, buyer::nftnet::ETH_BLOCK_VIEW_BYTES);
    assert_eq!(buyer::nftnet::block_view_bytes(1), check::PIN_BLOCK_BYTES);
}
