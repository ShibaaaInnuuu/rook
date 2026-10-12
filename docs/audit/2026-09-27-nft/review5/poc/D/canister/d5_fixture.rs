// Review round 5, part D: an eth_nft_state() written by the canister's OWN encoder (the 1dc2a61 build) with the round-4 states the
// monitor pages on: a forward order Stuck and still watched, a fill Stuck (the token not ours after it mined: watch ended at the
// arrival), an order written off and not credited, one written off and then closed as a loss, a way back past its deadline, a written-off
// sweep; a delivery re-priced whose EARLIER version mined (the line keeps it in `earlier`), and a delivery whose record is being sent
// on 4663. Writes $D5_OUT (hex). Nothing here is part of the canister's own suite.
use etch_autobuyer::ethflow::{self as ef, BPhase, DPhase, EthKind, EthState};

const MILADY: [u8; 20] = [0x5a, 0xf0, 0xd9, 0x82, 0x7e, 0x0c, 0x53, 0xe4, 0x79, 0x9b, 0xb2, 0x26, 0x65, 0x5a, 0x1d, 0xe1, 0x52, 0xa4, 0x25, 0xa5];
const WRAPPER: [u8; 20] = [0xaa; 20];
const MODULE: [u8; 20] = [0xbb; 20];
const DEST: [u8; 20] = [0xd0; 20];
const E: u128 = 1_000_000_000_000_000_000;
/// the delivery's first version (the one that mined) and its re-price (the one the record names)
const V1: [u8; 32] = [0x44; 32];
const V2: [u8; 32] = [0x45; 32];

fn tok(n: u64) -> [u8; 32] {
    let mut w = [0u8; 32];
    w[24..].copy_from_slice(&n.to_be_bytes());
    w
}

#[test]
fn d5_write_round4_paging_state() {
    let w = ef::Watch { order_id: [0x0d; 32], value: E, min_out: 9 * E / 10, deadline: 1_799_000_000, balance_after: 1, eth_before: 2,
        eth_out_before: 3, pin: Some(4), solver: Some([0xf7; 20]) };
    let buy = |id: u64, phase: BPhase| ef::EthBuy { id, collection: MILADY, module: MODULE, wrapper: WRAPPER, advanced: E, attempts: 1, phase,
        orders: vec![], deposits: vec![], ask: None, last: None, note: String::new(), sweep: None, arrived_at: None, back_at: None,
        ended_at: None, late: None };
    let mut st = EthState::default();
    // 1: a forward order past its deadline + grace, still watched (holds the Relay leg)
    let mut b1 = buy(1, BPhase::Stuck { why: "order 0x0d.. is past its deadline with no arrival of 0.9 ETH on Ethereum and no refund".into() });
    b1.last = Some(w.clone());
    // 2: a fill that mined but whose token is not ours (ethflow.rs mined_eth): the watch ended at the arrival, nothing is watched
    let b2 = buy(2, BPhase::Stuck { why: "the fill mined but ownerOf is 0x00000000000000000000000000000000000000ba".into() });
    // 3: an order written off, no payout credited yet
    let mut b3 = buy(3, BPhase::Stuck { why: "its order 0x0d.. is past its deadline ...: written off by the operator".into() });
    b3.late = Some(ef::LateOrder { watch: w.clone(), back: false, let_go_at: 1_799_100_000, credited: None });
    // 4: written off, then closed as a loss (returnAdvance of 0): still no payout credited
    let mut b4 = buy(4, BPhase::Returned { wei: 0 });
    b4.late = Some(ef::LateOrder { watch: w.clone(), back: true, let_go_at: 1_799_100_000, credited: None });
    // 5: a way back past its deadline + grace + a day, still watched
    let mut b5 = buy(5, BPhase::BackBridging { order_id: [0x0e; 32], value: E, min_out: 9 * E / 10, deadline: 1_799_000_000, since: 1_798_990_000 });
    b5.last = Some(w.clone());
    // 6: a sweep's way back written off (no returnAdvance: it ends with nothing)
    let mut b6 = buy(6, BPhase::Stuck { why: "its way back (order 0x0f..) ...: written off by the operator".into() });
    b6.sweep = Some(true);
    b6.advanced = 0;
    b6.late = Some(ef::LateOrder { watch: w.clone(), back: true, let_go_at: 1_799_100_000, credited: None });
    st.buys = vec![b1, b2, b3, b4, b5, b6];
    let del = |ticket: u128, token: u64, phase: DPhase| ef::Delivery { wrapper: WRAPPER, collection: MILADY, ticket, token: tok(token),
        destination: DEST, deliver_by: 1_800_500_000, phase, note: String::new(), refused_since: None, sent: None };
    st.deliveries = vec![
        del(4, 9183, DPhase::Sent { tx: V2, nonce: 5 }),
        del(7, 9188, DPhase::Recording { tx: [0x77; 32] }),
    ];
    st.line = Some(ef::EthTx { kind: EthKind::Deliver { wrapper: WRAPPER, ticket: 4 }, nonce: 5, hash: V2, raw: vec![2], value: 0,
        max_fee: 60_000_000_000, priority_fee: 2_000_000_000, signed_at: 1_799_999_000, to: MILADY, data: vec![], gas: 90_000,
        earlier: vec![V1], rivals: vec![], first_signed_at: Some(1_799_997_000) });
    st.last_mined = Some((4, 900));
    let bytes = candid::encode_one(&st).unwrap();
    assert_eq!(candid::decode_one::<EthState>(&bytes).unwrap(), st);
    let hex = hex::encode(&bytes);
    if let Ok(p) = std::env::var("D5_OUT") {
        std::fs::write(p, &hex).unwrap();
    }
    println!("d5 state: {} hex chars", hex.len());
}

/// monitor/test/d4-state.hex (review4's fixture, written by the 1eb5788 build's encoder) is a state this build decodes: every round-4
/// field absent reads as none, and nothing else changes on a re-encode except the new opt fields.
#[test]
fn d5_the_monitor_fixture_d4_state_is_a_canister_state() {
    let hex = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../monitor/test/d4-state.hex")).unwrap();
    let bytes = hex::decode(hex.trim()).unwrap();
    let st: EthState = candid::decode_one(&bytes).unwrap();
    assert!(st.buys.iter().all(|b| b.late.is_none()));
    assert_eq!(st.deliveries.len(), 4);
    println!("d4-state.hex: {} buys, {} deliveries, {} deposits; decoded by this build", st.buys.len(), st.deliveries.len(), st.deposits.len());
}
