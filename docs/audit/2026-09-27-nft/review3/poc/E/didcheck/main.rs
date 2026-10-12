//! Round 3, part E: the .did against the canister's own encoder. Every case of every Ethereum-side enum, encoded by the Rust
//! types, must decode with the .did's `EthState`; and the round-2 fixture too.
use candid::types::Type;
use candid::IDLArgs;
use candid_parser::utils::CandidSource;
use etch_autobuyer::ethflow::*;
use etch_autobuyer::ethnft::FloorPost;

fn main() {
    let did = std::fs::read_to_string("../autobuyer/canister-rs/etch_autobuyer.did").unwrap();
    let (env, _svc) = CandidSource::Text(&did).load().expect("the .did parses");
    env.find_type("EthState").expect("EthState in the .did");
    let t = Type::from(candid::types::TypeInner::Var("EthState".into()));
    println!("the .did parses: {} types", env.0.len());
    let fixture = hex::decode(std::fs::read_to_string("../autobuyer/canister-rs/tests/fixtures/eth-state-round2.hex").unwrap().trim()).unwrap();
    let a = IDLArgs::from_bytes_with_types(&fixture, &env, &[t.clone()]).expect("the round-2 fixture decodes as the .did's EthState");
    let lost = etch_autobuyer::canister::lost_opts(&fixture, &a.to_bytes_with_types(&env, &[t.clone()]).unwrap()).unwrap();
    println!("fixture decodes with the .did: {} bytes; opts the .did loses: {lost:?}", fixture.len());
    let tk = [9u8; 32];
    let phases = vec![BPhase::NeedAdvance, BPhase::Advancing, BPhase::Funded { wei: 5, since: 1 },
        BPhase::Depositing { order_id: [1; 32], value: 5, min_out: 4, deadline: 9 },
        BPhase::Bridging { order_id: [1; 32], value: 5, min_out: 4, deadline: 9, since: 2 }, BPhase::Arrived { wei: 5, since: 3 },
        BPhase::Buying { arrived: 5, token: tk, total: 4, tx: tk }, BPhase::Bought { arrived: 5, token: tk, total: 4, tx: tk, block: 7 },
        BPhase::Final { arrived: 5, token: tk, total: 4, tx: tk }, BPhase::Minting { arrived: 5, token: tk, total: 4, tx: tk },
        BPhase::Minted { arrived: 5, token: tk, total: 4, tx: tk }, BPhase::Settling { token: tk, total: 4, tx: tk, cost: 3 },
        BPhase::Settled { token: tk, total: 4, tx: tk, cost: 3 }, BPhase::Returning { wei: 5 }, BPhase::Returned { wei: 5 },
        BPhase::Stuck { why: "x".into() }, BPhase::BackFunded { wei: 5, since: 6 },
        BPhase::BackSending { wei: 5, order_id: [1; 32], value: 4, min_out: 3, deadline: 9, tx: tk },
        BPhase::BackBridging { order_id: [1; 32], value: 4, min_out: 3, deadline: 9, since: 7 }];
    let w = Watch { order_id: [1; 32], value: 5, min_out: 4, deadline: 9, balance_after: 1, eth_before: 2, eth_out_before: 3 };
    let mut st = EthState::default();
    st.buys = phases.into_iter().enumerate().map(|(i, phase)| EthBuy { id: i as u64, collection: [0x5a; 20], module: [0xbb; 20], wrapper: [0xaa; 20],
        advanced: 7, attempts: 1, phase, orders: vec![], deposits: vec![], ask: None, last: Some(w.clone()), note: String::new() }).collect();
    let dph = vec![DPhase::Waiting, DPhase::Sent { tx: tk, nonce: 1 }, DPhase::Moved { tx: tk, block: 2 }, DPhase::Record { tx: tk },
        DPhase::Recording { tx: tk }, DPhase::Recorded { tx: tk }, DPhase::Cancelling { of: tk, tx: tk, nonce: 1 },
        DPhase::MarkAfter { why: "w".into(), block: 3 }, DPhase::Mark { why: "m".into() }, DPhase::Marking { why: "m".into() },
        DPhase::Marked { why: "m".into() }, DPhase::Gone { why: "g".into() }];
    st.deliveries = dph.into_iter().enumerate().map(|(i, phase)| Delivery { wrapper: [0xaa; 20], collection: [0x5a; 20], ticket: i as u128, token: tk,
        destination: [0x44; 20], deliver_by: 9, phase, note: String::new(), refused_since: Some(1) }).collect();
    let xph = vec![XPhase::Seen, XPhase::Mint, XPhase::Minting, XPhase::Minted, XPhase::Return { why: "r".into() }, XPhase::Returning { tx: tk },
        XPhase::Returned { tx: tk }, XPhase::Held { why: "h".into() }, XPhase::Refused { why: "f".into() }];
    st.deposits = xph.into_iter().map(|phase| DepositRec { tx: tk, collection: [0x5a; 20], token: tk, recipient: [0; 20], from: [0; 20], block: 1, phase,
        refused_since: None }).collect();
    let mut check = |st: &EthState, what: &str| {
        let b = candid::encode_one(st).unwrap();
        match IDLArgs::from_bytes_with_types(&b, &env, &[t.clone()]) {
            // decoded with the .did's types, encoded again with them, and compared as the upgrade probe does: an opt that the .did
            // turned into nothing (Candid's opt rule: an unknown case under an opt decodes as null) is a mismatch too
            Ok(a) => {
                let again = a.to_bytes_with_types(&env, &[t.clone()]).unwrap();
                let lost = etch_autobuyer::canister::lost_opts(&b, &again).unwrap();
                if lost.is_empty() { println!("ok: {what}") } else { println!("MISMATCH: {what}: the .did loses {lost:?}") }
            }
            Err(e) => println!("MISMATCH: {what}: {e}"),
        }
    };
    check(&st, "every BPhase, DPhase and XPhase case");
    for (i, k) in [EthKind::Buy { buy: 1 }, EthKind::Deliver { wrapper: [1; 20], ticket: 2 }, EthKind::Cancel { wrapper: [1; 20], ticket: 2 },
        EthKind::Return { tx: tk, token: tk }, EthKind::CancelFill { buy: 1 }, EthKind::BackDeposit { buy: 1 }].into_iter().enumerate() {
        let mut s = EthState::default();
        s.line = Some(EthTx { kind: k, nonce: 1, hash: tk, raw: vec![], value: 0, max_fee: 1, priority_fee: 1, signed_at: 1, to: [0; 20], data: vec![],
            gas: 1, earlier: vec![], rivals: vec![], first_signed_at: None });
        check(&s, &format!("EthKind case {i}"));
    }
    for (i, step) in [Step::Advance { buy: 1 }, Step::Deposit { buy: 1 }, Step::GiveBack { buy: 1 }, Step::Mint { buy: 1 }, Step::Settle { buy: 1 },
        Step::Record { wrapper: [1; 20], ticket: 1 }, Step::Mark { wrapper: [1; 20], ticket: 1 }, Step::Post { collection: [1; 20] },
        Step::MintDeposit { tx: tk }].into_iter().enumerate() {
        let mut s = EthState::default();
        s.pending = Some(HomeTx { step, raw: vec![], hash: tk, nonce: 1 });
        check(&s, &format!("Step case {i}"));
    }
    let mut s = EthState::default();
    s.readings = vec![Reading { collection: [1; 20], post: FloorPost { price: 1, confidence: 1, depth: 1, listing: 1, sale_median: 1, sale_p25: 1, listings: 1,
        sales: 1, clamped: false, held: Some("h".into()) }, at: 1, pin: 1, due: true, module_posted_at: Some(1) }];
    s.sale_cache = Some(vec![CachedSales { collection: [1; 20], hash: tk, block: 1, sales: vec![(tk, 1, [1; 20], [2; 20])] }]);
    s.sale_scan = Some(vec![SaleScan { collection: [1; 20], down: 1, up: 2, chunk: 3, todo: vec![(tk, 1)] }]);
    s.last_mined = Some((1, 2));
    s.halted = Some("h".into());
    check(&s, "readings (held), sale_cache, sale_scan, last_mined, halted");
    let _ = t;
}
