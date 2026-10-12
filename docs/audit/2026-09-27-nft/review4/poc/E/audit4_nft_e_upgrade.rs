//! Round 4 of the 2026-09-27 NFT review, part E: upgrade safety of everything new since 55d8d32 (the operator's round a37f54b and
//! fix round 3). Saved states written by the OLD builds themselves (`dump_state_old.rs`, run on `git archive` exports of
//! 4949da0 = 55d8d32's canister tree, and of a37f54b), decoded by this build as `post_upgrade` does, with the C2-L-3 probe
//! (`lost_opts`). Run from `tests/` with the four `fixtures/*.hex` of this folder copied into `tests/fixtures/`. Tests that PASS
//! show what holds.

use etch_autobuyer::canister::{lost_opts, Saved};
use etch_autobuyer::ethflow::{self as ef, BPhase, CachedSales, DPhase, EthBuy, EthState, SaleScan};
use etch_autobuyer::ethnft::{self as en, EthNftConfig};
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

fn eth_of(hex_: &str) -> EthState {
    let bytes = hex::decode(hex_.trim()).unwrap();
    let (s,): (Saved,) = candid::decode_args(&bytes).unwrap();
    s.12.expect("the Ethereum side restores")
}

#[test]
fn e4_old_builds_states_restore_with_nothing_lost_and_the_new_fields_absent() {
    for (name, hex_) in [("e04f49b", include_str!("fixtures/state/saved-e04f49b.hex")),
        ("4949da0-rich", include_str!("fixtures/saved-4949da0-rich.hex")), ("a37f54b-rich", include_str!("fixtures/saved-a37f54b-rich.hex"))] {
        let bytes = hex::decode(hex_.trim()).unwrap();
        let lost = probe(&paged(&bytes)).unwrap();
        println!("AUDIT4-E probe {name}: {} bytes saved, lost {lost:?}", bytes.len());
        assert!(lost.is_empty(), "{name}: an honest upgrade would be refused: {lost:?}");
    }
    for (name, hex_) in [("4949da0", include_str!("fixtures/saved-4949da0-rich.hex")), ("a37f54b", include_str!("fixtures/saved-a37f54b-rich.hex"))] {
        let e = eth_of(hex_);
        assert_eq!(e.buys.len(), 19, "{name}");
        assert!(matches!(e.buys[16].phase, BPhase::BackFunded { .. }) && matches!(e.buys[18].phase, BPhase::BackBridging { .. }));
        assert!(e.buys.iter().all(|b| b.ask.is_some() && b.last.as_ref().is_some_and(|w| w.pin.is_none())), "{name}: watches restore, pin absent");
        assert!(e.buys.iter().all(|b| b.arrived_at.is_none() && b.back_at.is_none() && b.ended_at.is_none()));
        assert_eq!(e.deliveries.len(), 12);
        assert!(e.deliveries.iter().all(|d| d.refused_since.is_some() && d.sent.is_none()), "{name}: sent absent");
        assert!(matches!(e.deliveries[2].phase, DPhase::Moved { .. }));
        assert_eq!(e.deposits.len(), 9);
        let sc = &e.sale_scan.as_ref().unwrap()[0];
        assert_eq!((sc.todo.len(), sc.pend.clone(), sc.rescan, sc.judged.clone()), (2, None, None, None), "{name}: round 2's queue kept for the move into pend");
        assert_eq!(e.sale_cache.as_ref().map(|c| c.len()), Some(2), "{name}: the empty-sales entry (a round-2 judged non-sale) is kept");
        assert!(e.line.is_some() && e.pending.is_some() && e.halted.is_some() && e.last_mined.is_some());
        println!("AUDIT4-E {name}: leftover {:?}, sweep_after {:?}, sweeps {}", e.leftover.as_ref().map(|v| v.len()), e.sweep_after,
            e.buys.iter().filter(|b| b.is_sweep()).count());
    }
    let a = eth_of(include_str!("fixtures/saved-a37f54b-rich.hex"));
    assert_eq!((a.leftover.clone(), a.sweep_after), (Some(vec![([0x5a; 20], 7)]), Some(8)));
    assert!(a.buys[13].is_sweep());
    let o = eth_of(include_str!("fixtures/saved-4949da0-rich.hex"));
    assert_eq!((o.leftover.clone(), o.sweep_after), (None, None));
    let c: EthNftConfig = candid::decode_one(&hex::decode(include_str!("fixtures/cfg-4949da0.hex").trim()).unwrap()).unwrap();
    assert_eq!((c.gas_pot_target_wei, c.logs_max_blocks, c.logs_max(), c.gas_pot_target()), (None, None, en::SALE_SCAN_CHUNK_BLOCKS, en::GAS_POT_TARGET_WEI));
    let c: EthNftConfig = candid::decode_one(&hex::decode(include_str!("fixtures/cfg-a37f54b.hex").trim()).unwrap()).unwrap();
    assert_eq!((c.gas_pot_target_wei, c.logs_max_blocks), (Some(50_000_000_000_000_000), Some(500)));
}

/// The probe's cost at HEAD's bounds: the sale cache 2,048 × 5, the judged non-sales 8,192 × 5 (new), the queue 512 × 5 as `Move`s
/// (new), plus a long history.
#[test]
fn e4_probe_cost_on_the_largest_state_the_code_allows() {
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
        rescan: Some((1, 2)), judged: Some((0..ef::SALE_JUDGED_PER_COLLECTION as u64).map(|k| (k, k)).collect()) }).collect());
    let buy = |id: u64| EthBuy { id, collection: cols[0], module: [0xbb; 20], wrapper: [0xaa; 20], advanced: 1, attempts: 1,
        phase: BPhase::Settled { token: [1; 32], total: 1, tx: [2; 32], cost: 1 }, orders: vec![[3; 32]], deposits: vec![[4; 32]], ask: None, last: None,
        note: "settled at 1 wei".into(), sweep: None, arrived_at: Some(1), back_at: Some(2), ended_at: None };
    e.buys = (0..3_000).map(buy).collect();
    e.used_orders = (0..6_000u32).map(|k| { let mut h = [0u8; 32]; h[..4].copy_from_slice(&k.to_be_bytes()); h }).collect();
    e.outbound = (0..6_000u32).map(|k| (cols[0], [k as u8; 32], k as u64)).collect();
    e.leftover = Some(cols.iter().map(|c| (*c, 1)).collect());
    let saved: Saved = (None, vec![], false, vec!["x".repeat(200); 2_000], None, None, None, None, None, None, None, None, Some(e));
    let bytes = paged(&candid::encode_args((saved,)).unwrap());
    let t = Instant::now();
    let lost = probe(&bytes).unwrap();
    println!("AUDIT4-E probe cost: {} bytes of stable memory; the probe {:?}; lost {:?}", bytes.len(), t.elapsed(), lost);
    assert!(lost.is_empty());
}
