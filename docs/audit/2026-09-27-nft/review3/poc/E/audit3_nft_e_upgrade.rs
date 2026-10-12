//! Round 3 of the 2026-09-27 NFT review, part E: the round-2 upgrade probe (`canister::lost_opts`, C2-L-3) against real saved
//! shapes, its limits, and its cost on a large state. Tests that PASS show what they name (a probe that holds, or a gap).

use etch_autobuyer::canister::{lost_opts, Saved};
use etch_autobuyer::ethflow::{self as ef, BPhase, CachedSales, EthBuy, EthState, SaleScan};
use std::time::Instant;

/// `stable_bytes()` is the whole stable memory: the saved tuple, then the rest of its last 64 KiB page (zeros).
fn paged(b: &[u8]) -> Vec<u8> {
    let mut v = b.to_vec();
    v.resize(b.len().div_ceil(65_536) * 65_536, 0);
    v
}

/// What `post_upgrade` runs before anything else: restore, encode again, walk both.
fn probe(stable: &[u8]) -> Result<Vec<String>, String> {
    let (saved,): (Saved,) = candid::decode_args(stable).or_else(|_| {
        let mut de = candid::de::IDLDeserialize::new(stable).map_err(|e| e.to_string())?;
        let r = de.get_value::<Saved>().map_err(|e| e.to_string())?;
        Ok::<_, String>((r,))
    })?;
    let again = candid::encode_args((saved,)).map_err(|e| e.to_string())?;
    lost_opts(stable, &again)
}

#[test]
fn e3_probe_the_audited_e04f49b_state_and_a_rich_79455e1_state_restore_with_nothing_lost_and_no_false_refusal() {
    for (name, hex_) in [("e04f49b", include_str!("fixtures/state/saved-e04f49b.hex")), ("79455e1-rich", include_str!("fixtures/saved-79455e1-rich.hex"))] {
        let bytes = hex::decode(hex_.trim()).unwrap();
        let lost = probe(&paged(&bytes)).unwrap();
        println!("AUDIT3-E probe {name}: {} bytes saved, lost {lost:?}", bytes.len());
        assert!(lost.is_empty(), "{name}: an honest upgrade would be refused: {lost:?}");
    }
    // the 79455e1 state: every nested opt of the Ethereum side restores present, and the new fields read as absent
    let bytes = hex::decode(include_str!("fixtures/saved-79455e1-rich.hex").trim()).unwrap();
    let (s,): (Saved,) = candid::decode_args(&bytes).unwrap();
    let e = s.12.unwrap();
    assert_eq!(e.buys.len(), 16);
    assert!(e.buys.iter().all(|b| b.ask.is_some() && b.last.is_some()));
    assert!(e.deliveries.iter().all(|d| d.refused_since.is_some()) && e.deposits.iter().all(|d| d.refused_since.is_some()));
    assert!(e.line.as_ref().is_some_and(|l| l.first_signed_at.is_some()) && e.pending.is_some() && e.last_mined.is_some() && e.halted.is_some());
    assert!(e.sale_cache.as_ref().is_some_and(|c| c.len() == 1) && e.readings[0].module_posted_at.is_some());
    assert_eq!((e.readings[0].post.held.clone(), e.sale_scan.clone()), (None, None));
}

/// The probe compares `opt`s only where BOTH sides still have one: a required field turned into an `opt` of a new shape, or an `opt`
/// renamed, restores as nothing and is not reported (Candid's opt rule and its field-drop rule, by design; the fixer's contract
/// "new fields optional, new cases appended" is what keeps these out, not the probe).
#[test]
fn e3_i_probe_gaps_a_required_field_made_opt_of_a_new_shape_and_a_renamed_opt_are_lost_silently() {
    #[derive(candid::CandidType, serde::Deserialize)]
    struct InnerOld { x: u64 }
    #[derive(candid::CandidType, serde::Deserialize)]
    struct InnerNew { x: u64, y: u64 }
    #[derive(candid::CandidType, serde::Deserialize)]
    struct Old { a: u8, inner: InnerOld, foo: Option<u64> }
    #[derive(candid::CandidType, serde::Deserialize)]
    struct New { a: u8, inner: Option<InnerNew>, #[serde(default)] bar: Option<u64> }
    let saved = candid::encode_args((Some(Old { a: 1, inner: InnerOld { x: 7 }, foo: Some(9) }),)).unwrap();
    let (n,): (Option<New>,) = candid::decode_args(&saved).unwrap();
    let n = n.expect("the record decodes");
    assert!(n.inner.is_none() && n.bar.is_none(), "both values are gone");
    let again = candid::encode_args((Some(n),)).unwrap();
    assert!(lost_opts(&saved, &again).unwrap().is_empty(), "and the probe reports nothing");
}

/// The probe's cost: the state at the sizes the code allows (the sale cache at 2,048 per collection × 5, the scan queues at 512 ×
/// 5) plus a long history. `post_upgrade` decodes the typed state twice, encodes it once, and decodes both as untyped values.
#[test]
fn e3_probe_cost_on_a_large_state() {
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
    e.sale_scan = Some(cols.iter().map(|c| SaleScan { collection: *c, down: 1, up: 2, chunk: 2_000, todo: (0..512u32).map(|k| ([k as u8; 32], k as u64)).collect() }).collect());
    let buy = |id: u64| EthBuy { id, collection: cols[0], module: [0xbb; 20], wrapper: [0xaa; 20], advanced: 1, attempts: 1,
        phase: BPhase::Settled { token: [1; 32], total: 1, tx: [2; 32], cost: 1 }, orders: vec![[3; 32]], deposits: vec![[4; 32]], ask: None, last: None, note: "settled at 1 wei".into() };
    e.buys = (0..3_000).map(buy).collect();
    e.used_orders = (0..6_000u32).map(|k| { let mut h = [0u8; 32]; h[..4].copy_from_slice(&k.to_be_bytes()); h }).collect();
    e.outbound = (0..6_000u32).map(|k| (cols[0], [k as u8; 32], k as u64)).collect();
    let saved: Saved = (None, vec![], false, vec!["x".repeat(200); 2_000], None, None, None, None, None, None, None, None, Some(e));
    let bytes = paged(&candid::encode_args((saved,)).unwrap());
    let t = Instant::now();
    let (a,): (Saved,) = candid::decode_args(&bytes).or_else(|_| { let mut de = candid::de::IDLDeserialize::new(&bytes).unwrap(); Ok::<_, candid::Error>((de.get_value::<Saved>()?,)) }).unwrap();
    let t_decode = t.elapsed();
    let t = Instant::now();
    let lost = probe(&bytes).unwrap();
    let t_probe = t.elapsed();
    drop(a);
    println!("AUDIT3-E probe cost: {} bytes of stable memory; one typed decode {:?}; the probe (typed decode + encode + two untyped decodes + walk) {:?}; lost {:?}",
        bytes.len(), t_decode, t_probe, lost);
    assert!(lost.is_empty());
}
