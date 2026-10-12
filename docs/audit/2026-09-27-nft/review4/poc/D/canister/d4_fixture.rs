// Review round 4, part D: a live-shaped eth_nft_state() written by the canister's OWN encoder (deliveries in every phase the monitor
// judges, a bought token, deposits, the line, last_mined, a sweep), so the monitor's checks can be run on what plain() makes of the
// real bytes (F3-1 and its siblings). Writes $D4_OUT (hex).
use etch_autobuyer::ethflow::{self as ef, BPhase, DPhase, EthState, XPhase};
use etch_autobuyer::ethnft as en;

const MILADY: [u8; 20] = [0x5a, 0xf0, 0xd9, 0x82, 0x7e, 0x0c, 0x53, 0xe4, 0x79, 0x9b, 0xb2, 0x26, 0x65, 0x5a, 0x1d, 0xe1, 0x52, 0xa4, 0x25, 0xa5];
const WRAPPER: [u8; 20] = [0xaa; 20];
const MODULE: [u8; 20] = [0xbb; 20];
const DEST: [u8; 20] = [0xd0; 20];
const US: [u8; 20] = [0x76, 0x02, 0x0b, 0x70, 0x93, 0xbd, 0x35, 0xd2, 0x52, 0x0d, 0x47, 0xde, 0xd7, 0x64, 0xef, 0xc5, 0x54, 0x23, 0x7e, 0xfe];
const E: u128 = 1_000_000_000_000_000_000;

fn tok(n: u64) -> [u8; 32] { let mut w = [0u8; 32]; w[24..].copy_from_slice(&n.to_be_bytes()); w }

#[test]
fn d4_write_live_shaped_state() {
    let buy = |id: u64, phase: BPhase| ef::EthBuy { id, collection: MILADY, module: MODULE, wrapper: WRAPPER, advanced: E, attempts: 1, phase,
        orders: vec![], deposits: vec![], ask: None, last: None, note: String::new(), sweep: None, arrived_at: None, back_at: None, ended_at: None };
    let del = |ticket: u128, token: u64, phase: DPhase, sent: Option<bool>| ef::Delivery { wrapper: WRAPPER, collection: MILADY, ticket, token: tok(token),
        destination: DEST, deliver_by: 1_800_000_000, phase, note: String::new(), refused_since: None, sent };
    let mut st = EthState::default();
    st.buys = vec![
        buy(1, BPhase::Bought { arrived: E, token: tok(9181), total: E / 2, tx: [0xab; 32], block: 950 }),
        buy(2, BPhase::Arrived { wei: 3 * E / 10, since: 1_799_990_000 }),
        buy(3, BPhase::Minted { arrived: E, token: tok(9182), total: E / 2, tx: [0xac; 32] }),
    ];
    let mut sw = buy(4, BPhase::BackFunded { wei: E / 20, since: 1_799_999_000 });
    sw.sweep = Some(true); sw.advanced = 0;
    st.buys.push(sw);
    st.deliveries = vec![
        del(3, 9180, DPhase::Moved { tx: [0x33; 32], block: 200 }, Some(true)),
        del(4, 9183, DPhase::Sent { tx: [0x44; 32], nonce: 5 }, None),
        del(5, 9184, DPhase::Recorded { tx: [0x55; 32] }, Some(true)),
        del(6, 9185, DPhase::Moved { tx: [0x66; 32], block: 201 }, None),
    ];
    st.deposits = vec![ef::DepositRec { tx: [0x77; 32], collection: MILADY, token: tok(9186), recipient: [0x12; 20], from: [0x13; 20], block: 300,
        phase: XPhase::Minted, refused_since: None },
        ef::DepositRec { tx: [0x78; 32], collection: MILADY, token: tok(9187), recipient: [0x12; 20], from: [0x13; 20], block: 301,
        phase: XPhase::Mint, refused_since: None }];
    st.line = Some(ef::EthTx { kind: ef::EthKind::Buy { buy: 2 }, nonce: 6, hash: [0x99; 32], raw: vec![1], value: E / 4, max_fee: 1_000_000_000,
        priority_fee: 1, signed_at: 1, to: US, data: vec![], gas: 250_000, earlier: vec![], rivals: vec![], first_signed_at: Some(1) });
    st.last_mined = Some((5, 940));
    st.readings = vec![ef::Reading { collection: MILADY, post: en::FloorPost { price: 93 * E / 100, confidence: 10_000, depth: 3, listing: 93 * E / 100,
        sale_median: 93 * E / 100, sale_p25: 9 * E / 10, listings: 3, sales: 3, clamped: false, held: None }, at: 1_799_999_900, pin: 2, due: false, module_posted_at: None }];
    st.leftover = Some(vec![(MILADY, E / 20)]);
    let hex_now = hex::encode(candid::encode_one(st).unwrap());
    std::fs::write(std::env::var("D4_OUT").unwrap(), hex_now).unwrap();
}
