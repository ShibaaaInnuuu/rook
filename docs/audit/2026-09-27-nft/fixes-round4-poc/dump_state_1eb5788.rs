//! Fix round 4 (from review round 4, part E's dumper): a rich saved state written by the 1eb5788 build (round 3's fields present).
//! Round 4, part E: a rich saved state written by an OLD build, run as `examples/dump_state_old.rs` on a `git archive` export of
//! 4949da0 (= 55d8d32's canister tree) as it is here, and on a37f54b with the marked fields filled (`dump_state_old.a37f54b.sed`):
//! every nested `opt` of the Ethereum side present, every buy / delivery / deposit phase of that build, as `pre_upgrade` writes
//! `(saved,)`, then an `eth_nft` config. Printed as hex, one per line.
use etch_autobuyer::canister::Saved;
use etch_autobuyer::ethflow::*;
use etch_autobuyer::ethnft::{EthCollection, EthNftConfig, FloorParams, FloorPost};
use etch_autobuyer::relayconv::ConvAsk;

fn main() {
    let ask = ConvAsk { amount_wei: 5, user: "0xaa".into(), recipient: "0xaa".into(), refund_to: "0xaa".into(), origin_chain_id: 4663,
        destination_chain_id: 1, origin_currency: "0x0".into(), destination_currency: "0x0".into() };
    let w = Watch { order_id: [1; 32], value: 5, min_out: 4, deadline: 9, balance_after: 1, eth_before: 2, eth_out_before: 3, pin: Some(4) };
    let t = [9u8; 32];
    let phases = vec![BPhase::NeedAdvance, BPhase::Advancing, BPhase::Funded { wei: 5, since: 1 },
        BPhase::Depositing { order_id: [1; 32], value: 5, min_out: 4, deadline: 9 },
        BPhase::Bridging { order_id: [1; 32], value: 5, min_out: 4, deadline: 9, since: 2 }, BPhase::Arrived { wei: 5, since: 3 },
        BPhase::Buying { arrived: 5, token: t, total: 4, tx: t }, BPhase::Bought { arrived: 5, token: t, total: 4, tx: t, block: 7 },
        BPhase::Final { arrived: 5, token: t, total: 4, tx: t }, BPhase::Minting { arrived: 5, token: t, total: 4, tx: t },
        BPhase::Minted { arrived: 5, token: t, total: 4, tx: t }, BPhase::Settling { token: t, total: 4, tx: t, cost: 3 },
        BPhase::Settled { token: t, total: 4, tx: t, cost: 3 }, BPhase::Returning { wei: 5 }, BPhase::Returned { wei: 5 },
        BPhase::Stuck { why: "x".into() }, BPhase::BackFunded { wei: 5, since: 4 },
        BPhase::BackSending { wei: 5, order_id: [1; 32], value: 4, min_out: 3, deadline: 9, tx: t },
        BPhase::BackBridging { order_id: [1; 32], value: 4, min_out: 3, deadline: 9, since: 5 }];
    let buys = phases.into_iter().enumerate().map(|(i, phase)| EthBuy { id: i as u64, collection: [0x5a; 20], module: [0xbb; 20],
        wrapper: [0xaa; 20], advanced: 7, attempts: 1, phase, orders: vec![[1; 32]], deposits: vec![[2; 32]], ask: Some(ask.clone()),
        last: Some(w.clone()), note: "n".into() , sweep: Some(i == 13), arrived_at: Some(10), back_at: Some(11), ended_at: Some(12) }).collect();
    let dphases = vec![DPhase::Waiting, DPhase::Sent { tx: t, nonce: 1 }, DPhase::Moved { tx: t, block: 2 }, DPhase::Record { tx: t },
        DPhase::Recording { tx: t }, DPhase::Recorded { tx: t }, DPhase::Cancelling { of: t, tx: t, nonce: 1 },
        DPhase::MarkAfter { why: "w".into(), block: 3 }, DPhase::Mark { why: "m".into() }, DPhase::Marking { why: "m".into() },
        DPhase::Marked { why: "m".into() }, DPhase::Gone { why: "g".into() }];
    let deliveries = dphases.into_iter().enumerate().map(|(i, phase)| Delivery { wrapper: [0xaa; 20], collection: [0x5a; 20], ticket: i as u128,
        token: t, destination: [0x44; 20], deliver_by: 99, phase, note: "d".into(), refused_since: Some(5), sent: Some(true) }).collect();
    let xphases = vec![XPhase::Seen, XPhase::Mint, XPhase::Minting, XPhase::Minted, XPhase::Return { why: "r".into() }, XPhase::Returning { tx: t },
        XPhase::Returned { tx: t }, XPhase::Held { why: "h".into() }, XPhase::Refused { why: "f".into() }];
    let deposits = xphases.into_iter().map(|phase| DepositRec { tx: t, collection: [0x5a; 20], token: t, recipient: [0x44; 20], from: [0x33; 20],
        block: 4, phase, refused_since: Some(6) }).collect();
    let post = FloorPost { price: 1, confidence: 5000, depth: 3, listing: 2, sale_median: 1, sale_p25: 1, listings: 3, sales: 2, clamped: true,
        held: Some("held".into()) };
    let eth = EthState {
        buys, next_buy: 19, deliveries, cursors: vec![([0xaa; 20], 3)], deposits,
        readings: vec![Reading { collection: [0x5a; 20], post, at: 1, pin: 2, due: true, module_posted_at: Some(3) }],
        line: Some(EthTx { kind: EthKind::BackDeposit { buy: 17 }, nonce: 3, hash: t, raw: vec![1, 2], value: 4, max_fee: 1, priority_fee: 1,
            signed_at: 1, to: [0x76; 20], data: vec![7], gas: 21_000, earlier: vec![[3; 32]], rivals: vec![[4; 32]], first_signed_at: Some(1) }),
        eth_out: 11, pending: Some(HomeTx { step: Step::Sent { wrapper: [0xaa; 20], ticket: 3 }, raw: vec![5], hash: [5; 32], nonce: 8 }),
        used_orders: vec![[1; 32]], outbound: vec![([0x5a; 20], t, 3)],
        inbox: Inbox { quotes: vec![(2, vec![1], 3)], listings: vec![(5, "l".into(), 4)], evidence: vec![([0x5a; 20], vec!["x".into()], vec![[7; 32]], 5)],
            deposits: vec![([8; 32], 6)] },
        rr: 3, last_mined: Some((2, 7)), halted: Some("h".into()),
        sale_cache: Some(vec![CachedSales { collection: [0x5a; 20], hash: [6; 32], block: 7, sales: vec![(t, 1, [1; 20], [2; 20])] },
            CachedSales { collection: [0x5a; 20], hash: [7; 32], block: 8, sales: vec![] }]),
        sale_scan: Some(vec![SaleScan { collection: [0x5a; 20], down: 10, up: 20, chunk: 2_000, todo: vec![([3; 32], 15), ([4; 32], 16)],
            pend: Some(vec![etch_autobuyer::ethnft::Move { hash: [5; 32], block: 17, token: t, from: [1; 20] }]), rescan: Some((12, 14)), judged: Some(vec![(9, 13)]) }]),
        leftover: Some(vec![([0x5a; 20], 7)]), sweep_after: Some(8),
    };
    let saved: Saved = (None, vec![], true, vec!["installed".into()], Some(Default::default()), Some(Default::default()), Some(Default::default()),
        Some(Default::default()), Some(Default::default()), Some(Default::default()), Some(Default::default()), Some(Default::default()), Some(eth));
    println!("{}", hex::encode(candid::encode_args((saved,)).unwrap()));
    let cfg = EthNftConfig { chain_id: 1, relay_max_fee_bps: 300, deposits: true, floor: Some(FloorParams::default()), gas_pot_target_wei: Some(50_000_000_000_000_000), logs_max_blocks: Some(500),
        collections: vec![EthCollection { collection: "0x5af0d9827e0c53e4799bb226655a1de152a425a5".into(), wrapper: "0xaa".into(), module: "0xbb".into(),
            max_purchase_wei: 1, max_premium_bps: 500 }] };
    println!("{}", hex::encode(candid::encode_one(cfg).unwrap()));
}
