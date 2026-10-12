//! Round 3, part E: a rich saved state written by the 79455e1 build (= 27c0962's canister tree): every nested `opt` of the
//! Ethereum side present, every buy phase of that build, as `pre_upgrade` writes `(saved,)`. Printed as hex.
use etch_autobuyer::canister::Saved;
use etch_autobuyer::ethflow::*;
use etch_autobuyer::ethnft::FloorPost;
use etch_autobuyer::relayconv::ConvAsk;

fn main() {
    let ask = ConvAsk { amount_wei: 5, user: "0xaa".into(), recipient: "0xaa".into(), refund_to: "0xaa".into(), origin_chain_id: 4663,
        destination_chain_id: 1, origin_currency: "0x0".into(), destination_currency: "0x0".into() };
    let w = Watch { order_id: [1; 32], value: 5, min_out: 4, deadline: 9, balance_after: 1, eth_before: 2, eth_out_before: 3 };
    let t = [9u8; 32];
    let phases = vec![BPhase::NeedAdvance, BPhase::Advancing, BPhase::Funded { wei: 5, since: 1 },
        BPhase::Depositing { order_id: [1; 32], value: 5, min_out: 4, deadline: 9 },
        BPhase::Bridging { order_id: [1; 32], value: 5, min_out: 4, deadline: 9, since: 2 }, BPhase::Arrived { wei: 5, since: 3 },
        BPhase::Buying { arrived: 5, token: t, total: 4, tx: t }, BPhase::Bought { arrived: 5, token: t, total: 4, tx: t, block: 7 },
        BPhase::Final { arrived: 5, token: t, total: 4, tx: t }, BPhase::Minting { arrived: 5, token: t, total: 4, tx: t },
        BPhase::Minted { arrived: 5, token: t, total: 4, tx: t }, BPhase::Settling { token: t, total: 4, tx: t, cost: 3 },
        BPhase::Settled { token: t, total: 4, tx: t, cost: 3 }, BPhase::Returning { wei: 5 }, BPhase::Returned { wei: 5 },
        BPhase::Stuck { why: "x".into() }];
    let buys = phases.into_iter().enumerate().map(|(i, phase)| EthBuy { id: i as u64, collection: [0x5a; 20], module: [0xbb; 20],
        wrapper: [0xaa; 20], advanced: 7, attempts: 1, phase, orders: vec![[1; 32]], deposits: vec![[2; 32]], ask: Some(ask.clone()),
        last: Some(w.clone()), note: "n".into() }).collect();
    let dphases = vec![DPhase::Waiting, DPhase::Sent { tx: t, nonce: 1 }, DPhase::Moved { tx: t, block: 2 }, DPhase::Record { tx: t },
        DPhase::Cancelling { of: t, tx: t, nonce: 1 }, DPhase::MarkAfter { why: "w".into(), block: 3 }, DPhase::Gone { why: "g".into() }];
    let deliveries = dphases.into_iter().enumerate().map(|(i, phase)| Delivery { wrapper: [0xaa; 20], collection: [0x5a; 20], ticket: i as u128,
        token: t, destination: [0x44; 20], deliver_by: 99, phase, note: "d".into(), refused_since: Some(5) }).collect();
    let xphases = vec![XPhase::Seen, XPhase::Mint, XPhase::Return { why: "r".into() }, XPhase::Returning { tx: t }, XPhase::Held { why: "h".into() }];
    let deposits = xphases.into_iter().map(|phase| DepositRec { tx: t, collection: [0x5a; 20], token: t, recipient: [0x44; 20], from: [0x33; 20],
        block: 4, phase, refused_since: Some(6) }).collect();
    let post = FloorPost { price: 1, confidence: 5000, depth: 3, listing: 2, sale_median: 1, sale_p25: 1, listings: 3, sales: 2, clamped: true };
    let eth = EthState {
        buys, next_buy: 16, deliveries, cursors: vec![([0xaa; 20], 3)], deposits,
        readings: vec![Reading { collection: [0x5a; 20], post, at: 1, pin: 2, due: true, module_posted_at: Some(3) }],
        line: Some(EthTx { kind: EthKind::CancelFill { buy: 6 }, nonce: 3, hash: t, raw: vec![1, 2], value: 0, max_fee: 1, priority_fee: 1,
            signed_at: 1, to: [0x76; 20], data: vec![], gas: 21_000, earlier: vec![[3; 32]], rivals: vec![[4; 32]], first_signed_at: Some(1) }),
        eth_out: 11, pending: Some(HomeTx { step: Step::Settle { buy: 10 }, raw: vec![5], hash: [5; 32], nonce: 8 }),
        used_orders: vec![[1; 32]], outbound: vec![([0x5a; 20], t, 3)],
        inbox: Inbox { quotes: vec![(2, vec![1], 3)], listings: vec![(5, "l".into(), 4)], evidence: vec![([0x5a; 20], vec!["x".into()], vec![[7; 32]], 5)],
            deposits: vec![([8; 32], 6)] },
        rr: 3, last_mined: Some((2, 7)), halted: Some("h".into()),
        sale_cache: Some(vec![CachedSales { collection: [0x5a; 20], hash: [6; 32], block: 7, sales: vec![(t, 1, [1; 20], [2; 20])] }]),
    };
    let saved: Saved = (None, vec![], true, vec!["installed".into()], Some(Default::default()), Some(Default::default()), Some(Default::default()),
        Some(Default::default()), Some(Default::default()), Some(Default::default()), Some(Default::default()), Some(Default::default()), Some(eth));
    println!("{}", hex::encode(candid::encode_args((saved,)).unwrap()));
}
