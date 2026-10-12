//! Review round 5, part E: upgrade safety of round 4's saved state. Dropped into tests/ of a private copy, with the round-2 fixture as
//! each commit wrote it copied to tests/fixtures/r5/eth-state-round2.<commit>.hex (git show <commit>:autobuyer/canister-rs/tests/fixtures/
//! eth-state-round2.hex). Tests that PASS show what holds.

use etch_autobuyer::canister::{lost_opts, Saved};
use etch_autobuyer::ethflow::{self as ef, BPhase, CachedSales, EthBuy, EthState, SaleScan};
use etch_autobuyer::ethnft as en;
use std::time::Instant;

fn paged(b: &[u8]) -> Vec<u8> {
    let mut v = b.to_vec();
    v.resize(b.len().div_ceil(65_536) * 65_536, 0);
    v
}

fn probe(stable: &[u8]) -> Result<Vec<String>, String> {
    let (saved,): (Saved,) = candid::decode_args(stable).or_else(|_| {
        let mut de = candid::de::IDLDeserialize::new(stable).map_err(|e| e.to_string())?;
        let r = de.get_value::<Saved>().map_err(|e| e.to_string())?;
        Ok::<_, String>((r,))
    })?;
    let again = candid::encode_args((saved,)).map_err(|e| e.to_string())?;
    lost_opts(stable, &again)
}

/// The round-2 fixture as each build wrote it (24fba50: round 2; fbc9174: the operator's round; fb54018 = 2735dac: round 3; HEAD: round
/// 4's re-encoding), decoded as THIS build's EthState: nothing lost, and every version is the same state.
#[test]
fn holds_every_encoding_of_the_round_2_fixture_decodes_into_round_4_with_nothing_lost() {
    let mut states = Vec::new();
    for (name, hex_) in [("24fba50", include_str!("fixtures/r5/eth-state-round2.24fba50.hex")), ("fbc9174", include_str!("fixtures/r5/eth-state-round2.fbc9174.hex")),
        ("fb54018", include_str!("fixtures/r5/eth-state-round2.fb54018.hex")), ("HEAD", include_str!("fixtures/eth-state-round2.hex"))] {
        let bytes = hex::decode(hex_.trim()).unwrap();
        let st: EthState = candid::decode_one(&bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
        let lost = lost_opts(&bytes, &candid::encode_one(&st).unwrap()).unwrap();
        println!("R5-E upgrade: round-2 fixture as {name} wrote it: {} bytes, decodes, lost {lost:?}", bytes.len());
        assert!(lost.is_empty(), "{name}");
        states.push((name, st));
    }
    // round 3's bytes (fb54018 = 2735dac) and round 4's re-encoding are the same state: the re-encoding changed the type table only.
    // (24fba50's and fbc9174's hold another state: the operator's round added a sweep, leftover and sweep_after to round2_state())
    assert_eq!(states[2].1, states[3].1, "fb54018's bytes and HEAD's re-encoded ones are the same state");
    assert!(states[1].1.buys.len() == states[3].1.buys.len() && states[0].1.buys.len() + 1 == states[3].1.buys.len());
    println!("R5-E upgrade: round 3's and round 4's encodings of the fixture decode to the same EthState (only the type table differs)");
}

#[test]
fn holds_the_1eb5788_state_restores_and_its_round_3_content_survives() {
    let bytes = paged(&hex::decode(include_str!("fixtures/state/saved-1eb5788-rich.hex").trim()).unwrap());
    let lost = probe(&bytes).unwrap();
    let mut de = candid::de::IDLDeserialize::new(&bytes).unwrap();
    let s: Saved = de.get_value::<Saved>().unwrap();
    let e = s.12.unwrap();
    println!("R5-E upgrade: 1eb5788's state: lost {lost:?}; {} buys, {} deliveries, {} deposits, pending {:?}", e.buys.len(), e.deliveries.len(),
        e.deposits.len(), e.pending.as_ref().map(|p| &p.step));
    assert!(lost.is_empty());
    assert_eq!((e.buys.len(), e.deliveries.len(), e.deposits.len()), (19, 12, 9));
    assert!(matches!(e.buys[18].phase, BPhase::BackBridging { .. }) && e.buys.iter().all(|b| b.last.as_ref().is_some_and(|w| w.pin == Some(4))));
    // the round-4 semantics of what restores: the new mint key counts the old state's mints that went out
    let n = e.buys.iter().filter(|b| !b.is_sweep() && matches!(b.phase, BPhase::Minting { .. } | BPhase::Minted { .. } | BPhase::Settling { .. }
        | BPhase::Settled { .. })).count() + e.deposits.iter().filter(|d| matches!(d.phase, ef::XPhase::Minting | ef::XPhase::Minted)).count();
    println!("R5-E upgrade: after the upgrade the collection's next mint key is mint:eth:1:0x5a5a…:{n} (no old key has this shape: old keys were \
        mint:eth:<buy id> and mint:deposit:<tx>)");
}

/// The probe at round 4's bounds: review4's largest state plus every new field at its most (SaleScan.bad 8 and skipped a block per 8 of
/// the window, per collection; a written-off order on every buy; 8 payouts; 1,000 credited transactions; a home floor and pin floors).
#[test]
fn holds_probe_cost_on_the_largest_round_4_state() {
    let cols: Vec<[u8; 20]> = (0..5u8).map(|i| [0x50 + i; 20]).collect();
    let mut e = EthState::default();
    let mut cache = Vec::new();
    for c in &cols {
        for k in 0..ef::SALE_CACHE_PER_COLLECTION as u32 {
            let mut h = [0u8; 32];
            h[..4].copy_from_slice(&k.to_be_bytes());
            h[4] = c[0];
            cache.push(CachedSales { collection: *c, hash: h, block: 26_000_000 + k as u64, sales: vec![([k as u8; 32], 1_000_000_000_000_000_000, [1; 20], [2; 20])] });
        }
    }
    e.sale_cache = Some(cache);
    e.sale_scan = Some(cols.iter().map(|c| SaleScan { collection: *c, down: 1, up: 2, chunk: 500, todo: vec![],
        pend: Some((0..en::SALE_TODO_MAX as u32).map(|k| en::Move { hash: [k as u8; 32], block: k as u64, token: [k as u8; 32], from: [k as u8; 20] }).collect()),
        rescan: Some((1, 2)), judged: Some((0..ef::SALE_JUDGED_PER_COLLECTION as u64).map(|k| (k, k)).collect()),
        bad: Some((0..8u64).map(|k| (k, k + 1)).collect()), skipped: Some((0..en::SALE_WINDOW_BLOCKS / 8).map(|k| (k, k, 1u32)).collect()) }).collect());
    let w = ef::Watch { order_id: [1; 32], value: 5, min_out: 4, deadline: 9, balance_after: 1, eth_before: 2, eth_out_before: 3, pin: Some(4), solver: Some([7; 20]) };
    let buy = |id: u64| EthBuy { id, collection: cols[0], module: [0xbb; 20], wrapper: [0xaa; 20], advanced: 1, attempts: 1,
        phase: BPhase::Settled { token: [1; 32], total: 1, tx: [2; 32], cost: 1 }, orders: vec![[3; 32]], deposits: vec![[4; 32]], ask: None, last: Some(w.clone()),
        note: "settled at 1 wei".into(), sweep: None, arrived_at: Some(1), back_at: Some(2), ended_at: Some(3),
        late: Some(ef::LateOrder { watch: w.clone(), back: true, let_go_at: 5, credited: Some(([2; 32], 4, true)) }) };
    e.buys = (0..3_000).map(buy).collect();
    e.used_orders = (0..6_000u32).map(|k| { let mut h = [0u8; 32]; h[..4].copy_from_slice(&k.to_be_bytes()); h }).collect();
    e.outbound = (0..6_000u32).map(|k| (cols[0], [k as u8; 32], k as u64)).collect();
    e.leftover = Some(cols.iter().map(|c| (*c, 1)).collect());
    e.inbox.payouts = Some((0..8u64).map(|b| ef::PayoutClaim { buy: b, home: true, tx: [9; 32], wei: 1, at: 1 }).collect());
    e.credited = Some((0..1_000u32).map(|k| { let mut h = [0u8; 32]; h[..4].copy_from_slice(&k.to_be_bytes()); h }).collect());
    e.home_floor = Some((1, 2));
    e.pin_floors = Some(etch_autobuyer::pin::PinFloors::default());
    let saved: Saved = (None, vec![], false, vec!["x".repeat(200); 2_000], None, None, None, None, None, None, None, None, Some(e));
    let bytes = paged(&candid::encode_args((saved,)).unwrap());
    let t = Instant::now();
    let lost = probe(&bytes).unwrap();
    println!("R5-E upgrade: probe cost at round 4's largest state: {} bytes of stable memory; the probe {:?}; lost {:?}", bytes.len(), t.elapsed(), lost);
    assert!(lost.is_empty());
}
